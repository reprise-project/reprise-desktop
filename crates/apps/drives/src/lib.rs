//! reprise-drives: removable drives for the session. Does what `[drives]
//! on_insert` says with plugged-in drives (asks in a balloon, mounts through
//! UDisks2, opens a window, raises the passphrase box for locked ones) and
//! lists them, and the phones and cameras gvfs mounted, in the Safely Remove
//! Hardware tray menu, warns when the disk of
//! `$HOME` or `/` runs low on space (`space`), and asks in a balloon before a
//! Thunderbolt device is authorized (`thunderbolt`). No main window; the process
//! is autostarted for the session.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("drives");

mod model;
mod space;
mod thunderbolt;
mod tray;
mod watch;

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use repriseapp::keys::form_clipboard;
use repriseapp::unlock;
use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisefs::udisks::{ErrorKind, Snapshot};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::menu::{
    MenuAction, MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu, draw_shadow, draws_own_shadow,
    place_popup,
};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::taskbar::Edge;
use reprisekit::tr;

use model::{Act, Arrival, Entry, Target};

/// How long a balloon asking what to do with a new drive or device stays up, longer than an
/// ordinary balloon so a slower user still answers it there and not from the history.
const ASK_STAYS: std::time::Duration = std::time::Duration::from_secs(30);

const FLYOUT: WinId = WinId::named("flyout");
const UNLOCK: WinId = WinId::named("unlock");
const UNLOCK_NEXT: WinId = WinId::named("unlock-next");

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

/// The call a landed unlock leads to.
#[derive(Debug, PartialEq, Eq)]
enum Unlocked {
    /// The cleartext volume.
    Mount(String),
    /// The container, whose box was cancelled while it unlocked.
    Lock(String),
}

/// A pending safe removal's names, read before the drive goes away.
struct Removing {
    disk: String,
    /// What a refusal no volume answers for names: a drive's one mounted volume, or else the
    /// drive or share.
    named: String,
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
    /// The unlock box's client size at the text size it last heard.
    unlock_size: (i32, i32),
    unlock_queue: VecDeque<String>,
    /// Which of [`UNLOCK`] and [`UNLOCK_NEXT`] the box is in; it changes as a box closes, so
    /// the next maps a window of its own.
    unlock_turn: bool,
    /// Containers with an unlock under way, and whether it is still wanted: one cancelled while
    /// it ran is locked again when it lands.
    unlocking: HashMap<String, bool>,
    /// gvfs's mounts, whose phones and cameras get a Safely Remove row, named after
    /// `devices` ([`reprisefs::portable::dress`]).
    shares: Vec<reprisefs::netshare::Share>,
    devices: Vec<reprisefs::portable::Device>,
    removing: HashMap<Target, Removing>,
    balloons: HashMap<u32, PathBuf>,
    /// Ask balloons by tag, kept after the balloon goes: the history still runs its buttons.
    asks: HashMap<u32, Arrival>,
    /// Volumes that get a window once mounted; a container's moves to its
    /// cleartext volume when it unlocks.
    open_after: HashSet<String>,
    next_tag: u32,
    space: Option<Receiver<Vec<space::Reading>>>,
    warned: space::Warned,
    /// Low disk space balloons by tag, kept for the history's buttons.
    space_asks: HashMap<u32, space::Reading>,
    bolt: Option<(
        Receiver<thunderbolt::Msg>,
        repriseapp::Poster<thunderbolt::Msg>,
    )>,
    /// What boltd knows now, which a pressed button checks its device against.
    bolt_devices: Vec<reprisebus::bolt::Device>,
    bolt_asked: thunderbolt::Asked,
    /// Thunderbolt balloons by tag, kept for the history's buttons while their device waits.
    bolt_asks: HashMap<u32, reprisebus::bolt::Device>,
    /// The uids with a change under way, so a second button for one starts no second prompt.
    bolt_authorizing: HashSet<String>,
}

impl DrivesApp {
    fn snap(&self) -> &Snapshot {
        static EMPTY: Snapshot = Snapshot {
            disks: Vec::new(),
            volumes: Vec::new(),
            filesystems: Vec::new(),
        };
        self.snapshot.as_ref().unwrap_or(&EMPTY)
    }

    fn notify(&self, (summary, body): (String, String)) {
        if let Some(t) = &self.tray {
            t.notify(&summary, &body);
        }
    }

    fn push_state(&mut self) {
        self.entries = model::entries(self.snap(), &self.shares);
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
        let box_gone = self.unlock.as_ref().is_some_and(|u| gone(&u.volume));
        self.unlock_queue.retain(|v| !gone(v));
        self.open_after.retain(|v| snap.volume(v).is_some());
        if box_gone {
            self.close_unlock();
        }
        self.entries_changed();
    }

    /// The tray takes the new rows; an open menu showing others closes.
    fn entries_changed(&mut self) {
        self.push_state();
        if self
            .flyout
            .as_ref()
            .is_some_and(|f| f.entries != self.entries)
        {
            self.flyout = None;
        }
    }

    fn tag(&mut self) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        tag
    }

    /// `[drives] on_insert` is read at each arrival, so a change applies to the next drive.
    fn arrived(&mut self, a: Arrival) {
        let mode = model::OnInsert::parse(&repriseconf::current().drives_on_insert);
        let volume = a.volume().to_string();
        match model::act(a, mode) {
            Act::Ask(a) => self.ask(a),
            Act::Go { arrival, open } => self.go(arrival, open),
            Act::Nothing => log::info!("{volume} arrived; left alone"),
        }
    }

    /// The "New drive" balloon asking what to do with `a`.
    fn ask(&mut self, a: Arrival) {
        let name = model::arrival_name(self.snap(), a.volume());
        let b = model::ask_balloon(&name, matches!(a, Arrival::Unlock(_)));
        let tag = self.tag();
        self.asks.insert(tag, a);
        if let Some(t) = &self.tray {
            t.notify_ask(
                &b.summary,
                &b.body,
                tag,
                &b.buttons.each_ref().map(String::as_str),
                ASK_STAYS,
            );
        }
    }

    fn go(&mut self, a: Arrival, open: bool) {
        if open {
            self.open_after.insert(a.volume().to_string());
        }
        match a {
            Arrival::Mount(v) => {
                if let Some(c) = &self.calls {
                    c.mount(v);
                }
            }
            Arrival::Unlock(v) => self.unlock_queue.push_back(v),
        }
    }

    /// Button `i` of ask balloon `tag`: 0 mounts and opens, 1 mounts. A
    /// balloon pressed from the history after its drive went or was mounted
    /// elsewhere does only what still makes sense.
    fn ask_answered(&mut self, tag: u32, i: usize) {
        let Some(a) = self.asks.remove(&tag) else {
            return;
        };
        let open = i == 0;
        let Some(v) = self.snap().volume(a.volume()) else {
            return;
        };
        match &a {
            Arrival::Mount(_) if !v.mounts.is_empty() => {
                if open {
                    let at = v.mounts[0].clone();
                    self.open_path(&at);
                }
            }
            Arrival::Unlock(_) if !v.locked() => {}
            _ => self.go(a, open),
        }
    }

    /// `[storage] low_space_percent` is read at each round, so a change applies to the next.
    fn space_read(&mut self, round: &[space::Reading]) {
        let low = self
            .warned
            .feed(round, repriseconf::current().low_space_percent);
        for r in low {
            let (summary, body) = space::balloon(r);
            log::info!("{body}");
            let tag = self.tag();
            self.space_asks.insert(tag, r.clone());
            if let Some(t) = &self.tray {
                let buttons = space::buttons(r);
                let buttons: Vec<&str> = buttons.iter().map(String::as_str).collect();
                t.notify_buttons(&summary, &body, tag, &buttons);
            }
        }
    }

    /// Button `i` of low disk space balloon `tag`: Open opens the disk, Clean Up... opens Disk
    /// Usage on it with Clean Up over it, Empty Recycle Bin asks to empty the bin.
    fn space_answered(&mut self, tag: u32, i: usize) {
        let Some(r) = self.space_asks.get(&tag) else {
            return;
        };
        let mount = r.mount.clone();
        match i {
            space::OPEN => self.open_path(&mount),
            space::CLEAN_UP => {
                let args = [OsStr::new("--cleanup"), mount.as_os_str()];
                self.spawn_with_token("reprise-diskusage", args)
            }
            space::EMPTY_BIN if r.bin => {
                std::thread::spawn(|| {
                    if let Err(e) = reprisebus::tray::empty_recycle_bin() {
                        log::warn!("could not ask to empty the Recycle Bin: {e}");
                    }
                });
            }
            _ => {}
        }
    }

    /// What boltd knows now. While it is off the bus (a restart) the devices asked about stay
    /// asked, so its return brings no second balloon.
    fn bolt_found(&mut self, found: Option<reprisebus::bolt::Found>) {
        let Some(found) = found else {
            self.bolt_devices.clear();
            return;
        };
        self.bolt_devices = found.devices;
        let devices = &self.bolt_devices;
        self.bolt_asks
            .retain(|_, a| devices.iter().any(|d| d.uid == a.uid && d.waiting()));
        for d in self.bolt_asked.feed(&self.bolt_devices) {
            log::info!(
                "Thunderbolt device {} ({}) waits for authorization",
                d.uid,
                d.label_or("a Thunderbolt device")
            );
            let (summary, body) = thunderbolt::balloon(&d);
            let tag = self.tag();
            self.bolt_asks.insert(tag, d);
            if let Some(t) = &self.tray {
                t.notify_ask(
                    &summary,
                    &body,
                    tag,
                    &thunderbolt::buttons().each_ref().map(String::as_str),
                    ASK_STAYS,
                );
            }
        }
    }

    /// Button `i` of Thunderbolt balloon `tag`. A button pressed from the history acts only while
    /// the device still waits and no change for it is under way. True when a change started.
    fn bolt_answered(&mut self, tag: u32, i: usize) -> bool {
        let Some(asked) = self.bolt_asks.remove(&tag) else {
            return false;
        };
        let Some(change) = thunderbolt::change_for(i) else {
            return false;
        };
        let Some(d) = self
            .bolt_devices
            .iter()
            .find(|d| d.uid == asked.uid && d.waiting())
        else {
            log::info!("Thunderbolt device {} no longer waits", asked.uid);
            return false;
        };
        if !self.bolt_authorizing.insert(d.uid.clone()) {
            log::info!("Thunderbolt device {} is already being authorized", d.uid);
            return false;
        }
        if let Some((_, poster)) = &self.bolt {
            thunderbolt::carry_out(poster, d.clone(), change);
        }
        true
    }

    /// A change on device `uid` finished; a failure is told only while the device is still here.
    fn bolt_done(&mut self, uid: &str, label: &str, result: Result<(), Option<String>>) {
        self.bolt_authorizing.remove(uid);
        match result {
            Ok(()) => log::info!("Thunderbolt device {label} authorized"),
            Err(None) => {}
            Err(Some(why)) if self.bolt_devices.iter().any(|d| d.uid == uid) => {
                self.notify(thunderbolt::failed_balloon(label, &why));
            }
            Err(Some(why)) => log::info!("Thunderbolt device {label} left: {why}"),
        }
    }

    fn unlock_win(&self) -> WinId {
        if self.unlock_turn {
            UNLOCK_NEXT
        } else {
            UNLOCK
        }
    }

    fn close_unlock(&mut self) {
        self.unlock = None;
        self.unlock_turn = !self.unlock_turn;
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
        self.unlock_size = unlock::Unlock::client(ctx);
    }

    fn mounted(&mut self, volume: &str, result: Result<PathBuf, reprisefs::udisks::Error>) {
        let name = self.snap().volume(volume).map_or_else(
            || tr!("removable-disk"),
            |v| model::volume_name(self.snap(), v),
        );
        match result {
            Ok(at) if self.open_after.remove(volume) => self.open_path(&at),
            Ok(at) => {
                let (summary, body) = model::mounted_balloon(&name, &at);
                let tag = self.tag();
                self.balloons.insert(tag, at);
                if let Some(t) = &self.tray {
                    t.notify_clickable(&summary, &body, tag);
                }
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {
                self.open_after.remove(volume);
            }
            Err(e) => {
                self.open_after.remove(volume);
                log::warn!("mounting {volume}: {e}");
                self.notify((tr!("mount-failed", name = name.as_str()), e.message));
            }
        }
    }

    /// The cleartext volume may show up in a snapshot first and close the box.
    fn unlocked(
        &mut self,
        volume: &str,
        result: Result<String, reprisefs::udisks::Error>,
    ) -> Option<Unlocked> {
        let open = self.unlock.as_ref().is_some_and(|u| u.volume == volume);
        let wanted = self.unlocking.remove(volume) != Some(false);
        match result {
            Ok(_) if !wanted => Some(Unlocked::Lock(volume.to_string())),
            Ok(clear) => {
                if open {
                    self.close_unlock();
                }
                if self.open_after.remove(volume) {
                    self.open_after.insert(clear.clone());
                }
                Some(Unlocked::Mount(clear))
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {
                if open {
                    self.close_unlock();
                }
                self.open_after.remove(volume);
                None
            }
            Err(e) => {
                let u = self.unlock.as_mut().filter(|_| open)?;
                log::info!("unlocking {volume}: {e}");
                u.fail(unlock::failure(&e.message));
                None
            }
        }
    }

    fn removed(&mut self, disk: &str, result: Result<(), reprisefs::udisks::Error>) {
        let Some(r) = self.removing.remove(&Target::Disk(disk.to_string())) else {
            return;
        };
        match result {
            Ok(()) => self.notify(model::safe_balloon(&r.disk)),
            Err(e) => {
                log::info!("removing {disk}: {e}");
                if let Some(text) = reprisefs::udisks::eject_failed_text(&r.named, &e) {
                    self.notify((tr!("eject-failed", disk = r.disk.as_str()), text));
                }
            }
        }
    }

    fn unmounted(&mut self, at: PathBuf, result: Result<(), reprisefs::netshare::Unmount>) {
        use reprisefs::netshare::Unmount;
        let Some(r) = self.removing.remove(&Target::Mount(at)) else {
            return;
        };
        match result {
            Ok(()) => self.notify(model::safe_balloon(&r.disk)),
            Err(Unmount::Busy) => self.notify(model::busy_balloon(&r.disk, &r.named)),
            Err(Unmount::Error(e)) => {
                log::warn!("unmounting {}: {e}", r.disk);
                self.notify((tr!("eject-failed", disk = r.disk.as_str()), e));
            }
        }
    }

    fn remove(&mut self, entry: &Entry) {
        if self.removing.contains_key(&entry.target) {
            return;
        }
        let disk = match &entry.target {
            Target::Disk(disk) => disk,
            Target::Mount(at) => {
                let Some(s) = self.shares.iter().find(|s| s.path == *at) else {
                    return;
                };
                let r = Removing {
                    disk: s.name.clone(),
                    named: s.name.clone(),
                };
                self.removing.insert(entry.target.clone(), r);
                if let Some(c) = &self.calls {
                    c.unmount(at.clone());
                }
                return;
            }
        };
        let snap = self.snap();
        let Some(d) = snap.disk(disk) else {
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
            named: match volumes.as_slice() {
                [one] => one.clone(),
                _ => d.name(),
            },
        };
        self.removing.insert(entry.target.clone(), r);
        if let Some(c) = &self.calls {
            c.remove(disk.clone());
        }
    }

    fn open_folder(&mut self, tag: u32) {
        if let Some(at) = self.balloons.remove(&tag) {
            self.open_path(&at);
        }
    }

    /// A window on `at`, presented with the token a balloon click or button press left.
    fn open_path(&self, at: &Path) {
        let argv = reprisefs::apps::open_argv_for(at);
        self.spawn_with_token(&argv[0], &argv[1..]);
    }

    /// Starts `program` with `args` and the tray's activation token, so its window comes up.
    fn spawn_with_token(
        &self,
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) {
        let token = self.tray.as_ref().and_then(|t| t.take_activation_token());
        let mut cmd = std::process::Command::new(program.as_ref());
        cmd.args(args);
        reprisebus::instance::Handoff {
            token,
            by_key: false,
        }
        .apply(&mut cmd);
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => log::warn!("could not start {:?}: {e}", program.as_ref()),
        }
    }

    fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        while let Some(m) = self.bus.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                watch::Msg::Snapshot(s) => self.snapshot_in(s),
                watch::Msg::Locked(v) => log::info!("{v} arrived under the lock; left alone"),
                watch::Msg::Arrived(a) => self.arrived(a),
                watch::Msg::Mounted { volume, result } => self.mounted(&volume, result),
                watch::Msg::Unlocked { volume, result } => {
                    let next = self.unlocked(&volume, result);
                    if let Some(c) = &self.calls {
                        match next {
                            Some(Unlocked::Mount(clear)) => c.mount(clear),
                            Some(Unlocked::Lock(volume)) => c.lock(volume),
                            None => {}
                        }
                    }
                }
                watch::Msg::Removed { disk, result } => self.removed(&disk, result),
                watch::Msg::Shares(shares) => {
                    self.shares = shares;
                    reprisefs::portable::dress(&mut self.shares, &self.devices);
                    self.entries_changed();
                }
                watch::Msg::Devices(devices) => {
                    self.devices = devices;
                    reprisefs::portable::dress(&mut self.shares, &self.devices);
                    self.entries_changed();
                }
                watch::Msg::Unmounted { at, result } => self.unmounted(at, result),
            }
        }
        while let Some(round) = self.space.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.space_read(&round);
        }
        while let Some(m) = self.bolt.as_ref().and_then(|(rx, _)| rx.try_recv().ok()) {
            match m {
                thunderbolt::Msg::Found(f) => self.bolt_found(f),
                thunderbolt::Msg::Done { uid, label, result } => {
                    self.bolt_done(&uid, &label, result)
                }
            }
        }
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match tray::translate(raw, &self.entries) {
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
                        edge: repriseconf::current().taskbar.position,
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
                Some(tray::TrayMsg::Button(tag, i)) if self.space_asks.contains_key(&tag) => {
                    self.space_answered(tag, i)
                }
                Some(tray::TrayMsg::Button(tag, i)) if self.bolt_asks.contains_key(&tag) => {
                    self.bolt_answered(tag, i);
                }
                Some(tray::TrayMsg::Button(tag, i)) => self.ask_answered(tag, i),
                _ => {}
            }
        }
        if let Some(u) = self.unlock.as_mut() {
            u.land_icon(ctx);
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
            Event::Key(k) => match MenuKey::from_keysym(k.sym, k.utf8.as_deref()) {
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

    fn unlock_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(u) = self.unlock.as_mut() else {
            return Reply::none();
        };
        match u.input(ev, ctx) {
            unlock::Outcome::None => Reply::none(),
            unlock::Outcome::Redraw => Reply::redraw(),
            unlock::Outcome::Cancel => {
                self.open_after.remove(&u.volume);
                if let Some(wanted) = self.unlocking.get_mut(&u.volume) {
                    *wanted = false;
                }
                self.close_unlock();
                self.open_unlock(ctx);
                Reply::redraw_all()
            }
            unlock::Outcome::Submit(pass) => {
                self.unlocking.insert(u.volume.clone(), true);
                if let Some(c) = &self.calls {
                    c.unlock(u.volume.clone(), pass);
                }
                Reply::redraw()
            }
        }
    }
}

impl App for DrivesApp {
    fn title(&self) -> String {
        tray::tip().into()
    }

    fn app_id(&self) -> String {
        "reprise.drives".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        unlock::CLIENT
    }

    fn children(&self) -> Vec<Child> {
        self.unlock
            .iter()
            .map(|_| Child {
                id: self.unlock_win(),
                title: unlock::TITLE.into(),
                size: self.unlock_size,
                fixed: true,
                modal: false,
            })
            .collect()
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        // The tray first: it owns the bus name, so a second instance exits here.
        match tray::start(waker.clone()).or_exit(true) {
            Ok(Some((t, msgs))) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        let (poster, rx) = repriseapp::Poster::channel(waker.clone());
        space::watch(poster);
        self.space = Some(rx);
        self.bolt = Some(thunderbolt::watch(waker.clone()));
        let (rx, calls) = watch::watch(waker);
        self.bus = Some(rx);
        self.calls = Some(calls);
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match (win, ev) {
            (_, Event::Wake) => self.wake(ctx),
            (FLYOUT, ev) => self.flyout_event(ev, ctx),
            (w, ev) if w == self.unlock_win() => {
                if ev == Event::Resized {
                    self.unlock_size = unlock::Unlock::client(ctx);
                }
                let field = self.unlock.as_ref().and_then(|u| u.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.unlock_event(ev, ctx).with_clipboard(clip)
            }
            _ => Reply::none(),
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
            w if w == self.unlock_win() => {
                if let Some(u) = &self.unlock {
                    u.draw(c, ctx);
                }
            }
            _ => {}
        }
    }
}

impl repriseapp::Extended for DrivesApp {
    fn hidden(&self) -> bool {
        true
    }

    fn flyout(&self) -> Option<repriseapp::Flyout> {
        self.flyout.as_ref().map(|f| repriseapp::Flyout {
            id: FLYOUT,
            at: f.at,
            title: tray::tip().to_string(),
        })
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        if win != self.unlock_win() {
            return None;
        }
        let u = self.unlock.as_mut()?;
        let form = repriseapp::DropForm {
            form: &mut u.form,
            px: None,
        };
        Some(repriseapp::ReaderForm::Form(form))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        match win {
            FLYOUT => {
                if let Some(f) = self.flyout.as_mut() {
                    let focus = Self::menu(f, ctx).access(tree, ROOT, "menu");
                    tree.set_focus(focus);
                }
            }
            w if w == self.unlock_win() => {
                if let Some(u) = &self.unlock {
                    u.access(tree, ctx);
                }
            }
            _ => {}
        }
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, |_| DrivesApp::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    fn test_ctx(text_size: u32) -> Ctx {
        let theme = Theme::default();
        Ctx {
            theme: theme.with_text_size(text_size),
            text: TextRenderer::load(&theme.fonts).into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: unlock::CLIENT,
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            output_size: None,
            at_pick: false,
            text_size,
        }
    }

    #[test]
    fn a_device_being_authorized_takes_no_second_change_until_the_first_ends() {
        use reprisebus::bolt::{Device, Found, Status};
        let dock = Device {
            path: "/org/freedesktop/bolt/devices/dock".into(),
            uid: "dock".into(),
            name: "Dock".into(),
            vendor: "Acme".into(),
            status: Status::Connected,
            stored: false,
            auto: false,
        };
        let mut app = DrivesApp::default();
        let found = |devices: Vec<Device>| {
            Some(Found {
                domains: 1,
                devices,
            })
        };
        app.bolt_found(found(vec![dock.clone()]));
        app.bolt_found(None);
        app.bolt_found(found(vec![dock.clone()]));
        assert_eq!(
            app.bolt_asks.len(),
            1,
            "a boltd restart asks no second time"
        );
        app.bolt_asks.insert(7, dock.clone());
        let first = *app.bolt_asks.keys().find(|&&t| t != 7).unwrap();
        assert!(app.bolt_answered(first, thunderbolt::AUTHORIZE_ONCE));
        assert!(
            !app.bolt_answered(7, thunderbolt::AUTHORIZE_ONCE),
            "the history entry while the first change runs"
        );
        app.bolt_done("dock", "Acme Dock", Err(None));
        app.bolt_asks.insert(8, dock.clone());
        assert!(app.bolt_answered(8, thunderbolt::ALWAYS));
        let authorized = Device {
            status: Status::Authorized,
            ..dock
        };
        app.bolt_asks.insert(9, authorized.clone());
        app.bolt_found(found(vec![authorized]));
        assert!(
            app.bolt_asks.is_empty(),
            "a device that stopped waiting drops its balloons"
        );
    }

    #[test]
    fn a_new_drive_balloon_asks_to_stay_up_for_30_seconds() {
        let (tray, sent) = reprisetray::TrayHandle::recording();
        let mut app = DrivesApp {
            tray: Some(tray),
            ..DrivesApp::default()
        };
        let sdc = "/org/freedesktop/UDisks2/block_devices/sdc";
        app.ask(Arrival::Unlock(sdc.into()));
        app.ask(Arrival::Mount(sdc.into()));
        let expires: Vec<_> = sent.balloons().into_iter().map(|(_, e)| e).collect();
        assert_eq!(expires, [Some(std::time::Duration::from_secs(30)); 2]);
    }

    #[test]
    fn a_thunderbolt_balloon_asks_to_stay_up_for_30_seconds() {
        use reprisebus::bolt::{Device, Found, Status};
        let (tray, sent) = reprisetray::TrayHandle::recording();
        let mut app = DrivesApp {
            tray: Some(tray),
            ..DrivesApp::default()
        };
        app.bolt_found(Some(Found {
            domains: 1,
            devices: vec![Device {
                path: "/org/freedesktop/bolt/devices/dock".into(),
                uid: "dock".into(),
                name: "Dock".into(),
                vendor: "Acme".into(),
                status: Status::Connected,
                stored: false,
                auto: false,
            }],
        }));
        let expires: Vec<_> = sent.balloons().into_iter().map(|(_, e)| e).collect();
        assert_eq!(expires, [Some(std::time::Duration::from_secs(30))]);
    }

    #[test]
    fn the_unlock_box_resizes_when_the_text_size_changes() {
        let mut app = DrivesApp::default();
        let mut ctx = test_ctx(100);
        app.unlock_queue
            .push_back("/org/freedesktop/UDisks2/block_devices/sdc".into());
        app.open_unlock(&mut ctx);
        assert_eq!(app.children()[0].size, unlock::CLIENT);
        let mut ctx = test_ctx(200);
        let _ = app.event(UNLOCK, Event::Resized, &mut ctx);
        assert_eq!(
            app.children()[0].size,
            (2 * unlock::CLIENT.0, 2 * unlock::CLIENT.1)
        );
        let fresh = unlock::Unlock::open("sdc".into(), "Removable Disk", &mut ctx);
        let u = app.unlock.as_ref().expect("the box stays up");
        assert_eq!(
            u.form.item("message").map(|i| &i.control),
            fresh.form.item("message").map(|i| &i.control),
            "the message wraps as a box opened at 200% wraps it"
        );
    }

    fn ctrl_v() -> Event {
        let sym = u32::from(b'v');
        Event::Key(repriseapp::KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl: true,
                ..Default::default()
            },
            edit: reprisekit::textedit::key_for(sym, true, false, None),
        })
    }

    fn enter() -> Event {
        Event::Key(repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: Default::default(),
            edit: None,
        })
    }

    #[test]
    fn a_cancel_while_the_unlock_runs_leaves_the_drive_locked_and_unopened() {
        let sdc = "/org/freedesktop/UDisks2/block_devices/sdc";
        let clear = "/org/freedesktop/UDisks2/block_devices/dm_2d0";
        let mut app = DrivesApp::default();
        let mut ctx = test_ctx(100);
        app.go(Arrival::Unlock(sdc.into()), true);
        app.open_unlock(&mut ctx);
        let win = app.unlock_win();
        let _ = app.event(win, ctrl_v(), &mut ctx);
        let _ = app.event(win, Event::Pasted("secret".into()), &mut ctx);
        let _ = app.event(win, enter(), &mut ctx);
        let _ = app.event(win, Event::CloseRequest, &mut ctx);
        assert!(app.unlock.is_none());
        assert_eq!(
            app.unlocked(sdc, Ok(clear.into())),
            Some(Unlocked::Lock(sdc.into())),
            "the container is locked again, not mounted"
        );
        assert!(app.open_after.is_empty(), "no window waits for a mount");
    }

    #[test]
    fn an_unlock_that_lands_with_its_box_up_mounts_and_opens() {
        let sdc = "/org/freedesktop/UDisks2/block_devices/sdc";
        let clear = "/org/freedesktop/UDisks2/block_devices/dm_2d0";
        let mut app = DrivesApp::default();
        let mut ctx = test_ctx(100);
        app.go(Arrival::Unlock(sdc.into()), true);
        app.open_unlock(&mut ctx);
        let win = app.unlock_win();
        let _ = app.event(win, ctrl_v(), &mut ctx);
        let _ = app.event(win, Event::Pasted("secret".into()), &mut ctx);
        let _ = app.event(win, enter(), &mut ctx);
        assert_eq!(
            app.unlocked(sdc, Ok(clear.into())),
            Some(Unlocked::Mount(clear.into()))
        );
        assert!(app.open_after.contains(clear));
    }

    #[test]
    fn the_next_unlock_box_comes_up_in_a_window_of_its_own_when_one_is_cancelled() {
        let sdc = "/org/freedesktop/UDisks2/block_devices/sdc";
        let sdd = "/org/freedesktop/UDisks2/block_devices/sdd";
        let mut app = DrivesApp::default();
        let mut ctx = test_ctx(100);
        app.unlock_queue.extend([sdc.to_string(), sdd.to_string()]);
        app.open_unlock(&mut ctx);
        let first = app.unlock_win();
        let _ = app.event(first, Event::CloseRequest, &mut ctx);
        assert_eq!(app.unlock.as_ref().map(|u| u.volume.as_str()), Some(sdd));
        let next = app.children();
        assert_eq!(next.len(), 1);
        assert_ne!(next[0].id, first);
        let _ = app.event(first, Event::CloseRequest, &mut ctx);
        assert!(
            app.unlock.is_some(),
            "a late event on the first window misses the second box"
        );
    }

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
