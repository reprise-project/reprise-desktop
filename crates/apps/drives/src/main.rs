//! reprise-drives: removable drives for the session. Mounts plugged-in
//! drives through UDisks2 (balloon, passphrase box for locked ones) and
//! lists them in the Safely Remove Hardware tray menu. No main window; the
//! process is autostarted for the session.

mod model;
mod tray;
mod watch;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use repriseapp::keys::form_clipboard;
use repriseapp::unlock;
use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, KeyInput, PointerInput, Reply, WinId};
use reprisefs::udisks::{ErrorKind, Snapshot};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::menu::{
    MenuAction, MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu, draw_shadow, draws_own_shadow,
    place_popup,
};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::taskbar::Edge;
use reprisekit::textedit::keysym;

use model::{Arrival, Entry};

const FLYOUT: WinId = "flyout";
const UNLOCK: WinId = "unlock";

struct Flyout {
    at: (i32, i32),
    edge: Edge,
    entries: Vec<Entry>,
    tree: MenuTree,
    menu: Option<PopupMenu>,
}

/// Opens the menu with a corner on click point `at`; `edge` is the bar's
/// edge, and `place_popup` flips it where it would leave the output.
fn open_menu(tree: &MenuTree, at: (i32, i32), edge: Edge, cx: &MenuCtx) -> PopupMenu {
    let size = PopupMenu::open(tree, (0, 0), cx, false).rect();
    let corner = match edge {
        Edge::Bottom => (at.0, at.1 - size.height),
        Edge::Top | Edge::Left => at,
        Edge::Right => (at.0 - size.width, at.1),
    };
    let origin = place_popup(corner, (size.width, size.height), cx.within);
    PopupMenu::open(tree, origin, cx, false)
}

/// A pending safe removal's names, read before the drive goes away.
struct Removing {
    disk: String,
    volumes: String,
}

#[derive(Default)]
struct DrivesApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    bus: Option<Receiver<watch::Msg>>,
    calls: Option<watch::Calls>,
    snapshot: Option<Snapshot>,
    entries: Vec<Entry>,
    tray_state: Option<reprisetray::Shown>,
    flyout: Option<Flyout>,
    unlock: Option<unlock::Unlock>,
    unlock_queue: VecDeque<String>,
    removing: HashMap<String, Removing>,
    balloons: HashMap<u32, PathBuf>,
    next_tag: u32,
}

impl DrivesApp {
    fn snap(&self) -> &Snapshot {
        static EMPTY: Snapshot = Snapshot {
            disks: Vec::new(),
            volumes: Vec::new(),
        };
        self.snapshot.as_ref().unwrap_or(&EMPTY)
    }

    fn notify(&self, (summary, body): (String, String)) {
        if let Some(t) = &self.tray {
            t.notify(&summary, &body);
        }
    }

    fn push_state(&mut self) {
        self.entries = model::entries(self.snap());
        let next = tray::shown(&self.entries);
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(next.clone());
        }
        self.tray_state = Some(next);
    }

    fn snapshot_in(&mut self, s: Snapshot) {
        let arrivals = model::arrivals(self.snapshot.as_ref(), &s);
        self.snapshot = Some(s);
        if let Some(c) = &self.calls {
            for a in arrivals {
                c.arrived(a);
            }
        }
        let snap = self.snapshot.as_ref().expect("just set");
        let gone = |v: &str| snap.volume(v).is_none_or(|v| v.cleartext.is_some());
        if self.unlock.as_ref().is_some_and(|u| gone(&u.volume)) {
            self.unlock = None;
        }
        self.unlock_queue.retain(|v| !gone(v));
        self.push_state();
        if self
            .flyout
            .as_ref()
            .is_some_and(|f| f.entries != self.entries)
        {
            self.flyout = None;
        }
    }

    fn arrived(&mut self, a: Arrival) {
        match a {
            Arrival::Mount(v) => {
                if let Some(c) = &self.calls {
                    c.mount(v);
                }
            }
            Arrival::Unlock(v) => self.unlock_queue.push_back(v),
        }
    }

    fn open_unlock(&mut self, ctx: &mut Ctx) {
        if self.unlock.is_some() {
            return;
        }
        let Some(volume) = self.unlock_queue.pop_front() else {
            return;
        };
        let name = model::container_name(self.snap(), &volume);
        self.unlock = Some(unlock::Unlock::open(volume, &name, ctx));
    }

    fn mounted(&mut self, volume: &str, result: Result<PathBuf, reprisefs::udisks::Error>) {
        let name = self.snap().volume(volume).map_or_else(
            || "Removable Disk".into(),
            |v| model::volume_name(self.snap(), v),
        );
        match result {
            Ok(at) => {
                let (summary, body) = model::mounted_balloon(&name, &at);
                let tag = self.next_tag;
                self.next_tag = self.next_tag.wrapping_add(1);
                self.balloons.insert(tag, at);
                if let Some(t) = &self.tray {
                    t.notify_clickable(&summary, &body, tag);
                }
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {}
            Err(e) => {
                log::warn!("mounting {volume}: {e}");
                self.notify((format!("Could not mount {name}"), e.message));
            }
        }
    }

    /// The cleartext volume may show up in a snapshot first and close the box.
    fn unlocked(&mut self, volume: &str, result: Result<String, reprisefs::udisks::Error>) {
        let open = self.unlock.as_ref().is_some_and(|u| u.volume == volume);
        match result {
            Ok(clear) => {
                if open {
                    self.unlock = None;
                }
                if let Some(c) = &self.calls {
                    c.mount(clear);
                }
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {
                if open {
                    self.unlock = None;
                }
            }
            Err(e) => {
                let Some(u) = self.unlock.as_mut().filter(|_| open) else {
                    return;
                };
                log::info!("unlocking {volume}: {e}");
                u.fail(unlock::failure(&e.message));
            }
        }
    }

    fn removed(&mut self, disk: &str, result: Result<(), reprisefs::udisks::Error>) {
        let Some(r) = self.removing.remove(disk) else {
            return;
        };
        match result {
            Ok(()) => self.notify(model::safe_balloon(&r.disk)),
            Err(e) if e.kind == ErrorKind::Cancelled => {}
            Err(e) if e.kind == ErrorKind::Busy => {
                self.notify(model::busy_balloon(&r.disk, &r.volumes))
            }
            Err(e) => {
                log::warn!("removing {disk}: {e}");
                self.notify((format!("Problem Ejecting {}", r.disk), e.message));
            }
        }
    }

    fn remove(&mut self, entry: &Entry) {
        if self.removing.contains_key(&entry.disk) {
            return;
        }
        let snap = self.snap();
        let Some(d) = snap.disk(&entry.disk) else {
            return;
        };
        let volumes: Vec<String> = snap
            .volumes_on(&d.path)
            .into_iter()
            .filter(|v| !v.mounts.is_empty())
            .map(|v| v.name(Some(d)))
            .collect();
        let r = Removing {
            disk: d.name(),
            volumes: volumes.join(", "),
        };
        self.removing.insert(entry.disk.clone(), r);
        if let Some(c) = &self.calls {
            c.remove(entry.disk.clone());
        }
    }

    fn open_folder(&mut self, tag: u32) {
        let Some(at) = self.balloons.remove(&tag) else {
            return;
        };
        let argv = reprisefs::apps::open_argv_for(&at);
        let token = self.tray.as_ref().and_then(|t| t.take_activation_token());
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        if let Some(t) = token {
            cmd.env("XDG_ACTIVATION_TOKEN", t);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => log::warn!("could not open {}: {e}", at.display()),
        }
    }

    fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        while let Some(m) = self.bus.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                watch::Msg::Snapshot(s) => self.snapshot_in(s),
                watch::Msg::Locked(v) => log::info!("{v} arrived under the lock; left alone"),
                watch::Msg::Arrived(a) => self.arrived(a),
                watch::Msg::Mounted { volume, result } => self.mounted(&volume, result),
                watch::Msg::Unlocked { volume, result } => self.unlocked(&volume, result),
                watch::Msg::Removed { disk, result } => self.removed(&disk, result),
            }
        }
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match tray::translate(raw) {
                Some(tray::TrayMsg::Open(x, y)) if !self.entries.is_empty() => {
                    let entries = self.entries.clone();
                    let tree = MenuTree::flat(
                        entries
                            .iter()
                            .map(|e| MenuItem::entry(e.label.clone()))
                            .collect(),
                    );
                    self.flyout = Some(Flyout {
                        at: (x, y),
                        edge: Edge::from_name(&repriseconf::taskbar_position()).unwrap_or_default(),
                        entries,
                        tree,
                        menu: None,
                    });
                }
                Some(tray::TrayMsg::Remove(i)) => {
                    if let Some(e) = self.entries.get(i).cloned() {
                        self.remove(&e);
                    }
                }
                Some(tray::TrayMsg::Balloon(tag)) => self.open_folder(tag),
                _ => {}
            }
        }
        self.open_unlock(ctx);
        Reply::redraw_all()
    }

    fn menu<'a>(f: &'a mut Flyout, ctx: &Ctx) -> &'a mut PopupMenu {
        let cx = MenuCtx {
            text: &ctx.text,
            theme: &ctx.theme,
            px: ctx.theme.fonts.size_px,
            within: Rect::new(0, 0, ctx.size.0, ctx.size.1),
        };
        let at = (f.at.0 - ctx.origin.0, f.at.1 - ctx.origin.1);
        f.menu
            .get_or_insert_with(|| open_menu(&f.tree, at, f.edge, &cx))
    }

    fn flyout_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(f) = self.flyout.as_mut() else {
            return Reply::none();
        };
        let cx = MenuCtx {
            text: &ctx.text,
            theme: &ctx.theme,
            px: ctx.theme.fonts.size_px,
            within: Rect::new(0, 0, ctx.size.0, ctx.size.1),
        };
        Self::menu(f, ctx);
        let Some(menu) = f.menu.as_mut() else {
            return Reply::none();
        };
        let action = match ev {
            Event::Pointer(PointerInput::Motion { x, y }) => {
                if menu.motion(&f.tree, x, y, &cx) {
                    MenuAction::Redraw
                } else {
                    MenuAction::None
                }
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => menu.click(&f.tree, x, y, &cx),
            Event::Pointer(PointerInput::Press { x, y, .. }) if !menu.contains(x, y) => {
                MenuAction::Close
            }
            Event::Key(k) => match menu_key(&k) {
                Some(key) => menu.key(&f.tree, key, &cx),
                None => MenuAction::None,
            },
            Event::CloseRequest | Event::Focus(false) => MenuAction::Close,
            _ => MenuAction::None,
        };
        match action {
            MenuAction::None => Reply::none(),
            MenuAction::Redraw => Reply::redraw(),
            MenuAction::Close => {
                self.flyout = None;
                Reply::redraw()
            }
            MenuAction::Chosen(path) => {
                let entry = path.first().and_then(|&i| f.entries.get(i)).cloned();
                self.flyout = None;
                if let Some(e) = entry {
                    self.remove(&e);
                }
                Reply::redraw()
            }
        }
    }

    fn unlock_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(u) = self.unlock.as_mut() else {
            return Reply::none();
        };
        match u.input(ev, ctx) {
            unlock::Outcome::None => Reply::none(),
            unlock::Outcome::Redraw => Reply::redraw(),
            unlock::Outcome::Cancel => {
                self.unlock = None;
                Reply::redraw_all()
            }
            unlock::Outcome::Submit(pass) => {
                if let Some(c) = &self.calls {
                    c.unlock(u.volume.clone(), pass);
                }
                Reply::redraw()
            }
        }
    }
}

fn menu_key(k: &KeyInput) -> Option<MenuKey> {
    Some(match k.sym {
        keysym::ESCAPE => MenuKey::Escape,
        keysym::UP | keysym::KP_UP => MenuKey::Up,
        keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
        keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
        _ => match k.utf8.as_deref().and_then(|s| s.chars().next()) {
            Some(c) if c.is_alphanumeric() => MenuKey::Char(c),
            _ => return None,
        },
    })
}

impl App for DrivesApp {
    fn title(&self) -> String {
        tray::TIP.into()
    }

    fn app_id(&self) -> String {
        "reprise.drives".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        unlock::CLIENT
    }

    fn hidden(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        self.unlock
            .iter()
            .map(|_| Child {
                id: UNLOCK,
                title: unlock::TITLE.into(),
                size: unlock::CLIENT,
                fixed: true,
                modal: false,
            })
            .collect()
    }

    fn flyout(&self) -> Option<repriseapp::Flyout> {
        self.flyout.as_ref().map(|f| repriseapp::Flyout {
            id: FLYOUT,
            at: f.at,
        })
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        // The tray first: it owns the bus name, so a second instance exits here.
        match tray::start(waker.clone()) {
            reprisetray::Started::Running(t, msgs) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
            }
            reprisetray::Started::AlreadyRunning => std::process::exit(0),
            reprisetray::Started::Unavailable => {
                log::error!("the tray item could not start (no session bus?); exiting");
                std::process::exit(1);
            }
        }
        let (rx, calls) = watch::watch(waker);
        self.bus = Some(rx);
        self.calls = Some(calls);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match (win, ev) {
            (_, Event::Wake) => self.wake(ctx),
            (FLYOUT, ev) => self.flyout_event(ev, ctx),
            (UNLOCK, ev) => {
                let field = self.unlock.as_ref().and_then(|u| u.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.unlock_event(ev, ctx).with_clipboard(clip)
            }
            _ => Reply::none(),
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        match win {
            FLYOUT => {
                if let Some(f) = self.flyout.as_mut() {
                    let focus = Self::menu(f, ctx).access(tree, ROOT, "menu");
                    tree.set_focus(focus);
                }
            }
            UNLOCK => {
                if let Some(u) = &self.unlock {
                    u.access(tree, ctx);
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        match win {
            FLYOUT => {
                let Some(f) = self.flyout.as_mut() else {
                    return;
                };
                let menu = Self::menu(f, ctx);
                let r = menu.rect();
                let s = ctx.theme.scale.max(1);
                if !draws_own_shadow(&ctx.theme) {
                    let shadow =
                        draw_shadow(r.width * s, r.height * s, ctx.theme.metrics.menu_shadow);
                    c.blit_over(r.x * s, r.y * s, &shadow);
                }
                menu.draw(c, &ctx.theme, &ctx.text, false);
            }
            UNLOCK => {
                if let Some(u) = &self.unlock {
                    u.draw(c, ctx);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(DrivesApp::default()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    #[test]
    fn the_menu_opens_up_and_left_of_the_click() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = MenuCtx {
            text: &text,
            theme: &theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 1920, 1080),
        };
        let tree = MenuTree::flat(vec![
            MenuItem::entry("Safely remove Kingston DataTraveler - KINGSTON"),
            MenuItem::entry("Safely remove USB Mass Storage Device - VAULT"),
        ]);
        let r = open_menu(&tree, (1849, 1050), Edge::Bottom, &cx).rect();
        assert_eq!((r.right(), r.bottom()), (1849, 1050));
        assert_eq!(r.height, 2 * 17 + 6, "two 17 px rows inside a 3 px border");
    }

    #[test]
    fn a_one_row_menu_opens_up_from_a_bottom_bar() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = MenuCtx {
            text: &text,
            theme: &theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 1920, 1080),
        };
        let tree = MenuTree::flat(vec![MenuItem::entry("Safely remove TEST")]);
        let r = open_menu(&tree, (1849, 1050), Edge::Bottom, &cx).rect();
        assert_eq!((r.right(), r.bottom()), (1849, 1050));
    }

    #[test]
    fn under_a_top_bar_the_menu_opens_down_and_left_of_the_click() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = MenuCtx {
            text: &text,
            theme: &theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 1920, 1080),
        };
        let tree = MenuTree::flat(vec![MenuItem::entry(
            "Safely remove Kingston DataTraveler - KINGSTON",
        )]);
        let r = open_menu(&tree, (1849, 30), Edge::Top, &cx).rect();
        assert_eq!((r.right(), r.y), (1849, 30));
    }

    #[test]
    fn beside_a_side_bar_the_menu_opens_away_from_it() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cx = MenuCtx {
            text: &text,
            theme: &theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 1920, 1080),
        };
        let tree = MenuTree::flat(vec![MenuItem::entry("Safely remove TEST")]);
        let r = open_menu(&tree, (60, 1060), Edge::Left, &cx).rect();
        assert_eq!((r.x, r.bottom()), (60, 1060));
        let r = open_menu(&tree, (1860, 1060), Edge::Right, &cx).rect();
        assert_eq!((r.right(), r.bottom()), (1860, 1060));
    }
}
