//! Terminal: `reprise-terminal [-e prog args...]`, a tabbed terminal window in the inherited cwd.
//! With the tabs in the caption each tab is a window of its own; the program ends with its last
//! window.

mod chrome;
mod cwd;
mod find;
mod keys;
mod newtab;
mod options;
mod pointer;
mod schemes;
mod settings;
mod snapshot;
mod sshhosts;
mod tab;
mod tabs;
mod urls;

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use alacritty_terminal::grid::Scroll;
use repriseapp::keys::{form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, Event, KeyInput, MAIN, Reply, WinId};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::termgrid::{self, CursorShape, Metrics, Snapshot};
use reprisekit::controls::{finddlg, form, propsheet, tabstrip, toolbar};
use reprisekit::interface::Interface;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::TextSpot;
use reprisekit::theme::{Color, FontConfig, Theme};

use crate::chrome::Chrome;
use crate::keys::Shortcut;
use crate::newtab::{Choice, Launch};
use crate::options::{Options, Outcome};
use crate::schemes::Scheme;
use crate::settings::{Settings, State, TabPosition};
use crate::tab::Tab;
use crate::tabs::Tabs;

/// The monospace faces tried after `[font]`, in order.
pub const FONTS: [&str; 3] = ["Lucida Console", "Liberation Mono", "DejaVu Sans Mono"];
/// The grid a new window opens at, in cells.
const OPEN_CELLS: (i32, i32) = (80, 24);
/// The text sizes zoom stays between, px.
const ZOOM: (u32, u32) = (6, 72);
const CONFIRM: WinId = "confirm";
const OPTIONS: WinId = "options";
const PROFILE: WinId = "profile";
const FIND: WinId = "find";
const NOTFOUND: WinId = "notfound";

/// What a confirm box is asking to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Closing {
    /// The tab whose program has this pid.
    Tab(u32),
    Window(WinId),
}

/// One window's own state: its tabs, its chrome and what it last drew.
struct Pane {
    tabs: Tabs,
    scale: i32,
    /// The grid (cols, rows) and cell (device px) last handed to the tabs.
    fitted: Option<((usize, usize), (i32, i32))>,
    snap: Snapshot,
    /// The canvas size and scale last drawn into; another one comes cleared.
    drawn: (u32, u32, i32),
    /// The cursor as last drawn.
    shown: (usize, usize, CursorShape),
    focused: bool,
    chrome: Chrome,
    mouse: pointer::Mouse,
    /// The next frame draws everything and damages the whole canvas.
    full: bool,
    /// Its tabs are windows of one caption tab group, one tab each, rather than a strip under
    /// the toolbar. Fixed when the window opens.
    caption: bool,
    /// The window whose caption tab group this one joins as it opens.
    leader: Option<WinId>,
    /// The client size last drawn at, logical px: what a window opened from this one takes.
    size: (i32, i32),
}

impl Pane {
    fn new(caption: bool, leader: Option<WinId>) -> Pane {
        let black = Color::rgb(0, 0, 0);
        Pane {
            tabs: Tabs::new(),
            scale: 1,
            fitted: None,
            snap: Snapshot::new(0, 0, black, black),
            drawn: (0, 0, 0),
            shown: (0, 0, CursorShape::Hidden),
            focused: false,
            chrome: Chrome::new(),
            mouse: pointer::Mouse::default(),
            full: true,
            caption,
            leader,
            size: (0, 0),
        }
    }
}

struct Terminal {
    settings: Settings,
    cmd: Option<Vec<OsString>>,
    waker: Option<repriseapp::Waker>,
    font: TextRenderer,
    /// The text size, zoomed, px.
    px: u32,
    /// Cell metrics and the scale and size they were taken at.
    metrics: ((i32, u32), Metrics),
    /// The window the event or draw at hand is for (`enter`); the others wait in `parked`.
    w: Pane,
    cur: WinId,
    parked: Vec<(WinId, Pane)>,
    /// The open windows, in tab order as they opened.
    order: Vec<WinId>,
    /// The compositor joins windows into caption tab groups (`repriseapp::tab_groups`).
    tab_groups: bool,
    confirm: Option<(Closing, MsgBox)>,
    /// The last event's text and form theme, which the confirm box is sized with.
    forms: Option<(Rc<TextRenderer>, Theme)>,
    options: Option<Options>,
    options_on: WinId,
    find: Option<form::Form>,
    /// The window Find searches and sits over.
    find_on: WinId,
    last_find: find::Query,
    notfound: Option<MsgBox>,
    config: PathBuf,
    /// The settings file's mtime when last loaded or written.
    loaded: Option<std::time::SystemTime>,
    state_path: PathBuf,
    /// The client size of the window as the user last sized it.
    window: Option<(i32, i32)>,
}

/// The arguments after `-e`, the program to run instead of the login shell.
fn command(args: impl Iterator<Item = OsString>) -> Option<Vec<OsString>> {
    let mut args = args.skip_while(|a| a != "-e");
    args.next()?;
    Some(args.collect()).filter(|c: &Vec<OsString>| !c.is_empty())
}

fn load_font(s: &Settings) -> TextRenderer {
    let family: Vec<String> = std::iter::once(s.font.as_str())
        .filter(|f| !f.is_empty())
        .chain(FONTS)
        .map(str::to_owned)
        .collect();
    let cfg = |antialias| FontConfig {
        family: family.clone(),
        size_px: s.size_px,
        antialias,
        italic: false,
        full_hinting: false,
        shell_dlg: None,
    };
    // Lucida Console draws hinted monochrome; any other face blotches that way at 13 px, so it is
    // antialiased.
    if s.font.is_empty() {
        let mono = TextRenderer::load(&cfg(false));
        if mono.has_face(FONTS[0]) {
            return mono;
        }
    }
    TextRenderer::load(&cfg(true))
}

/// `s` with its text size inside the zoom range.
fn in_range(s: Settings) -> Settings {
    Settings {
        size_px: s.size_px.clamp(ZOOM.0, ZOOM.1),
        ..s
    }
}

fn interface(theme: &Theme) -> Interface {
    match theme.look.style() {
        None => Interface::Classic,
        Some(s) if s.glass.is_some() => Interface::Glass,
        Some(_) => Interface::Blue,
    }
}

impl Terminal {
    fn new(settings: Settings, cmd: Option<Vec<OsString>>) -> Terminal {
        Terminal::with_state(settings, cmd, settings::state_path())
    }

    fn with_state(settings: Settings, cmd: Option<Vec<OsString>>, state_path: PathBuf) -> Terminal {
        let settings = in_range(settings);
        let font = load_font(&settings);
        let px = settings.size_px;
        let metrics = termgrid::metrics(&font, px, 1);
        Terminal {
            settings,
            cmd,
            waker: None,
            font,
            px,
            metrics: ((1, px), metrics),
            w: Pane::new(false, None),
            cur: MAIN,
            parked: Vec::new(),
            order: vec![MAIN],
            tab_groups: false,
            confirm: None,
            forms: None,
            options: None,
            options_on: MAIN,
            find: None,
            find_on: MAIN,
            last_find: find::Query::default(),
            notfound: None,
            config: settings::path(),
            loaded: None,
            window: State::load(&state_path).window,
            state_path,
        }
    }

    /// Leave the program, keeping the window's size for the next run.
    fn quit(&self) -> Reply {
        if self.window.is_some() {
            State {
                window: self.window,
            }
            .save(&self.state_path);
        }
        Reply {
            quit: true,
            ..Default::default()
        }
    }

    /// The opacity of the grid's background: see-through only where the frame is glass.
    fn background_alpha(&self, theme: &Theme) -> u8 {
        let glass = reprisekit::look::client_glass(theme, true).is_some();
        if glass && self.settings.see_through {
            (u32::from(self.settings.opacity.min(100)) * 255 / 100) as u8
        } else {
            255
        }
    }

    /// Open the Options sheet on the settings in force.
    fn open_options(&mut self) -> Reply {
        self.options
            .get_or_insert_with(|| Options::new(&self.settings));
        self.options_on = self.cur;
        Reply::redraw_all()
    }

    /// Take `s` for every open tab and the window, and keep it in the settings file.
    fn apply(&mut self, s: Settings) {
        if let Err(e) = settings::try_load(&self.config) {
            log::warn!(
                "{}: replacing a file that did not load ({e})",
                self.config.display()
            );
        }
        if let Err(e) = settings::save(&self.config, &s) {
            log::warn!("{}: {e}", self.config.display());
        }
        self.loaded = mtime(&self.config);
        self.take(s);
    }

    /// Another process may have saved the settings: take the file's when its mtime moved and it
    /// parses. True when settings were taken.
    fn reload(&mut self) -> bool {
        let now = mtime(&self.config);
        if now.is_none() || now == self.loaded {
            return false;
        }
        self.loaded = now;
        match settings::try_load(&self.config) {
            Ok(s) => {
                self.take(s);
                true
            }
            Err(e) => {
                log::warn!(
                    "{}: {e}; keeping the settings in force",
                    self.config.display()
                );
                false
            }
        }
    }

    fn take(&mut self, s: Settings) {
        let s = in_range(s);
        let font =
            (s.font.as_str(), s.size_px) != (self.settings.font.as_str(), self.settings.size_px);
        self.settings = s;
        if font {
            self.font = load_font(&self.settings);
            self.px = self.settings.size_px;
            self.metrics.0 = (0, 0);
        }
        for p in panes_mut(&mut self.w, &mut self.parked) {
            for t in &p.tabs.list {
                t.configure(&self.settings);
            }
            p.full = true;
        }
    }

    /// Whether a window opened now keeps its tabs in the caption: the setting asks for it and
    /// the compositor can; the strip under the toolbar otherwise.
    fn caption_mode(&self) -> bool {
        self.settings.tab_position == TabPosition::Caption && self.tab_groups
    }

    fn scheme(&self, theme: &Theme) -> Scheme {
        schemes::scheme(&self.settings.scheme, interface(theme))
    }

    fn metrics(&mut self, scale: i32) -> Metrics {
        if self.metrics.0 != (scale, self.px) {
            self.metrics = (
                (scale, self.px),
                termgrid::metrics(&self.font, self.px, scale),
            );
        }
        self.metrics.1
    }

    /// Hand every tab the grid the client holds under the chrome, when it changed.
    fn fit(&mut self, ctx: &Ctx) {
        let m = self.metrics(self.w.scale);
        let (cols, rows) =
            termgrid::grid_size(&m, chrome::grid_rect(ctx, self.w.caption), self.w.scale);
        let want = ((cols.max(2), rows.max(1)), (m.cell_w, m.cell_h));
        if self.w.fitted != Some(want) {
            self.w.fitted = Some(want);
            for t in &self.w.tabs.list {
                t.resize(want.0, want.1);
            }
        }
    }

    fn spawn_tab(&mut self, launch: &Launch, cwd: Option<PathBuf>) {
        let Some(waker) = self.waker.clone() else {
            return;
        };
        let m = self.metrics.1;
        let open = (OPEN_CELLS.0 as usize, OPEN_CELLS.1 as usize);
        let (size, cell) = self.w.fitted.unwrap_or((open, (m.cell_w, m.cell_h)));
        match Tab::spawn(launch, cwd, size, cell, waker) {
            Ok(tab) => {
                tab.configure(&self.settings);
                let label = tab.title().to_owned();
                self.w.tabs.open(tab, label);
            }
            Err(e) => log::error!("cannot start {:?}: {e}", launch.argv),
        }
    }

    /// A new tab running `choice`, in the open tab's directory.
    fn new_tab(&mut self, choice: &Choice) -> Reply {
        let launch = newtab::launch(choice, &self.settings);
        let cwd = self.w.tabs.current().and_then(Tab::cwd);
        if self.w.caption {
            return self.new_pane(&launch, cwd);
        }
        let before = self.w.tabs.list.len();
        self.spawn_tab(&launch, cwd);
        match self.w.tabs.list.len() == before {
            true => Reply::none(),
            false => self.switched(),
        }
    }

    /// A window of its own running `launch`, joining this window's caption tab group right
    /// after it.
    fn new_pane(&mut self, launch: &Launch, cwd: Option<PathBuf>) -> Reply {
        let leader = self.cur;
        let id = (1..)
            .map(repriseapp::window_id)
            .find(|id| !self.order.contains(id))
            .expect("a free window id");
        let mut pane = Pane::new(true, Some(leader));
        (pane.size, pane.fitted) = (self.w.size, self.w.fitted);
        self.parked.push((id, pane));
        let at = self
            .order
            .iter()
            .position(|&w| w == leader)
            .map_or(0, |i| i + 1);
        self.order.insert(at, id);
        self.enter(id);
        self.spawn_tab(launch, cwd);
        if self.w.tabs.list.is_empty() {
            self.drop_pane();
            self.enter(leader);
            return Reply::none();
        }
        Reply::redraw_all()
    }

    /// Make `win`'s pane the one at hand; false when `win` is no terminal window.
    fn enter(&mut self, win: WinId) -> bool {
        if win == self.cur {
            return self.order.contains(&win);
        }
        let Some(i) = self.parked.iter().position(|(id, _)| *id == win) else {
            return false;
        };
        let (id, pane) = self.parked.swap_remove(i);
        let was = std::mem::replace(&mut self.w, pane);
        if self.order.contains(&self.cur) {
            self.parked.push((self.cur, was));
        }
        self.cur = id;
        true
    }

    fn pane(&self, win: WinId) -> Option<&Pane> {
        match win == self.cur {
            true => self.order.contains(&win).then_some(&self.w),
            false => self
                .parked
                .iter()
                .find(|(id, _)| *id == win)
                .map(|(_, p)| p),
        }
    }

    /// The window whose tabs include the one running `pid`.
    fn pane_of_pid(&self, pid: u32) -> Option<WinId> {
        self.order.iter().copied().find(|&id| {
            self.pane(id)
                .is_some_and(|p| p.tabs.list.iter().any(|t| t.pid() == pid))
        })
    }

    /// Close the window at hand: another one comes to hand and takes over the dialogs it had.
    /// True when it was the last.
    fn drop_pane(&mut self) -> bool {
        let gone = self.cur;
        self.order.retain(|&w| w != gone);
        let Some(&next) = self.order.first() else {
            return true;
        };
        let (_, pane) = self
            .parked
            .iter()
            .position(|(id, _)| *id == next)
            .map(|i| self.parked.swap_remove(i))
            .expect("every open window but the one at hand is parked");
        self.w = pane;
        self.cur = next;
        if self.options_on == gone {
            self.options_on = next;
        }
        if self.find_on == gone {
            (self.find, self.notfound, self.find_on) = (None, None, next);
        }
        let asked = match self.confirm {
            Some((Closing::Window(w), _)) => w != gone,
            Some((Closing::Tab(pid), _)) => self.pane_of_pid(pid).is_some(),
            None => true,
        };
        if !asked {
            self.confirm = None;
        }
        false
    }

    /// Close the window at hand, the program with its last.
    fn close_pane(&mut self) -> Reply {
        match self.drop_pane() {
            true => self.quit(),
            false => Reply::redraw_all(),
        }
    }

    /// Another tab is open (or the open one moved): its cells and the strip are drawn again.
    fn switched(&mut self) -> Reply {
        for t in &self.w.tabs.list {
            t.end_find();
        }
        self.w.snap.dirty.fill(true);
        self.chrome_changed()
    }

    /// The command line of another window in the open tab's directory.
    fn new_window(&self) -> (Vec<OsString>, Option<PathBuf>) {
        let cwd = self.w.tabs.current().and_then(Tab::cwd);
        (vec!["reprise-terminal".into()], cwd)
    }

    fn copy(&self) -> Reply {
        Reply {
            copy: self.w.tabs.current().and_then(Tab::selection),
            ..Default::default()
        }
    }

    fn paste(&self) -> Reply {
        Reply {
            paste: true,
            ..Default::default()
        }
    }

    fn zoom(&mut self, px: u32, ctx: &Ctx) -> Reply {
        self.px = px.clamp(ZOOM.0, ZOOM.1);
        self.fit(ctx);
        for p in panes_mut(&mut self.w, &mut self.parked) {
            p.full = true;
        }
        Reply::redraw_all()
    }

    /// Close tab `i`, asking first while a program other than its shell runs there.
    fn ask_close(&mut self, i: usize, ctx: &mut Ctx) -> Reply {
        let Some(t) = self.w.tabs.list.get(i) else {
            return Reply::none();
        };
        if t.busy() {
            let text = "A program is still running in this tab.\nClose the tab anyway?";
            self.ask(Closing::Tab(t.pid()), text, ctx);
            return Reply::redraw();
        }
        self.close_tab(i)
    }

    fn close_tab(&mut self, i: usize) -> Reply {
        self.drop_press();
        if self.w.tabs.close(i) {
            return self.close_pane();
        }
        self.switched()
    }

    fn close_window(&mut self, ctx: &mut Ctx) -> Reply {
        if self.w.tabs.list.iter().any(Tab::busy) {
            let text = "Programs are still running in this window.\nClose the window anyway?";
            self.ask(Closing::Window(self.cur), text, ctx);
            return Reply::redraw();
        }
        self.close_pane()
    }

    /// A press on the strip names tabs by index, which a closed tab shifts; a toolbar button's
    /// index stays.
    fn drop_press(&mut self) {
        if !matches!(self.w.chrome.press, Some(chrome::Press::Button(_))) {
            self.w.chrome.press = None;
        }
        self.w.tabs.strip.held = false;
        self.w.tabs.strip.hot_adder = None;
    }

    /// Ask about `what`, unless a box is already up: the first question stands.
    fn ask(&mut self, what: Closing, text: &str, ctx: &mut Ctx) {
        if self.confirm.is_some() {
            return;
        }
        let fields = Fields {
            text: text.into(),
            icon: ctx.icons.get("dialog-warning", 32),
            buttons: vec!["&Close".into(), "Cancel".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
        };
        self.confirm = Some((what, MsgBox::new("Terminal", fields)));
    }

    fn confirm_event(&mut self, ev: Event) -> Reply {
        let (Some((what, b)), Some((text, forms))) = (&mut self.confirm, &self.forms) else {
            return Reply::none();
        };
        match b.event(&ev, text, forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                let what = *what;
                self.confirm = None;
                let reply = match (button, what) {
                    (Some(0), Closing::Window(w)) if self.enter(w) => self.close_pane(),
                    (Some(0), Closing::Tab(pid)) => {
                        let w = self.pane_of_pid(pid).filter(|&w| self.enter(w));
                        match w.and_then(|_| self.w.tabs.list.iter().position(|t| t.pid() == pid)) {
                            Some(i) => self.close_tab(i),
                            None => Reply::none(),
                        }
                    }
                    _ => Reply::none(),
                };
                reply.and(Reply::redraw_all())
            }
        }
    }

    /// Carry out what each window's tabs queued; a window whose last tab closed closes. Only the
    /// windows whose shown tab, strip or chrome changed draw again.
    fn wake(&mut self, ctx: &Ctx) -> Reply {
        let scheme = self.scheme(&ctx.theme);
        let mut reply = Reply::none();
        for id in self.order.clone() {
            if !self.enter(id) {
                continue;
            }
            let open = self.w.tabs.strip.open;
            let mut shown = false;
            for (i, t) in self.w.tabs.list.iter_mut().enumerate() {
                let Some(copied) = t.pump(&scheme) else {
                    continue;
                };
                shown |= i == open;
                reply.copy = copied.clipboard.or(reply.copy);
                reply.copy_primary = copied.primary.or(reply.copy_primary);
            }
            let reaped = self.reap();
            for (program, code) in &reaped {
                if *code != 0 {
                    log::warn!("{program} exited with status {code}");
                }
            }
            if self.w.tabs.list.is_empty() {
                if self.drop_pane() {
                    return self.quit();
                }
                reply.redraw_all = true;
                continue;
            }
            for (label, t) in self.w.tabs.strip.labels.iter_mut().zip(&self.w.tabs.list) {
                if label != t.title() {
                    *label = t.title().to_owned();
                    self.w.chrome.dirty = true;
                }
            }
            if shown || !reaped.is_empty() || self.w.chrome.dirty {
                reply.redraw_wins.push(id);
            }
        }
        reply
    }

    /// Close the tabs whose program exited; their programs and exit codes.
    fn reap(&mut self) -> Vec<(String, i32)> {
        let mut out = Vec::new();
        let mut i = self.w.tabs.list.len();
        while i > 0 {
            i -= 1;
            if let Some(code) = self.w.tabs.list[i].exited() {
                out.push((self.w.tabs.list[i].program().to_owned(), code));
                self.w.tabs.close(i);
                self.drop_press();
                self.w.snap.dirty.fill(true);
                self.w.chrome.dirty = true;
            }
        }
        if let Some((Closing::Tab(pid), _)) = self.confirm
            && self.pane_of_pid(pid).is_none()
        {
            self.confirm = None;
        }
        out
    }

    fn key(&mut self, k: &KeyInput, ctx: &mut Ctx) -> Reply {
        if self.w.chrome.menu.is_some() {
            return self.menu_key(k, ctx);
        }
        let Some(tab) = self.w.tabs.current() else {
            return Reply::none();
        };
        let mode = tab.mode();
        if let Some(s) = keys::shortcut(k, mode) {
            return self.shortcut(s, ctx);
        }
        match keys::encode(k, mode) {
            Some(bytes) => {
                tab.scroll(Scroll::Bottom);
                tab.write(bytes.into());
                Reply::redraw()
            }
            None => Reply::none(),
        }
    }

    fn shortcut(&mut self, s: Shortcut, ctx: &mut Ctx) -> Reply {
        match s {
            Shortcut::NewTab => self.new_tab(&Choice::Shell),
            Shortcut::CloseTab => self.ask_close(self.w.tabs.strip.open, ctx),
            Shortcut::NextTab | Shortcut::PrevTab if self.w.caption => Reply {
                raise: self.sibling(s == Shortcut::PrevTab),
                ..Default::default()
            },
            Shortcut::NextTab | Shortcut::PrevTab => {
                match self.w.tabs.strip.cycle(s == Shortcut::PrevTab) {
                    true => self.switched(),
                    false => Reply::none(),
                }
            }
            Shortcut::NewWindow => {
                let (argv, cwd) = self.new_window();
                if let Err(e) = repriseapp::spawn_in(&argv, cwd.as_deref(), None) {
                    log::warn!("cannot open another window: {e}");
                }
                Reply::none()
            }
            Shortcut::Copy => self.copy(),
            Shortcut::Paste => self.paste(),
            Shortcut::Find => self.open_find(),
            Shortcut::ZoomIn => self.zoom(self.px + 1, ctx),
            Shortcut::ZoomOut => self.zoom(self.px.saturating_sub(1), ctx),
            Shortcut::ZoomReset => self.zoom(self.settings.size_px, ctx),
            Shortcut::PageUp | Shortcut::PageDown => {
                if let Some(t) = self.w.tabs.current() {
                    t.scroll(match s {
                        Shortcut::PageUp => Scroll::PageUp,
                        _ => Scroll::PageDown,
                    });
                }
                Reply::redraw()
            }
        }
    }

    /// The caption-mode window after this one in opening order, or before it, going round.
    fn sibling(&self, backwards: bool) -> Option<WinId> {
        let ring: Vec<WinId> = self
            .order
            .iter()
            .copied()
            .filter(|&w| self.pane(w).is_some_and(|p| p.caption))
            .collect();
        let i = ring.iter().position(|&w| w == self.cur)?;
        let n = ring.len();
        let j = match backwards {
            true => (i + n - 1) % n,
            false => (i + 1) % n,
        };
        Some(ring[j]).filter(|&w| w != self.cur)
    }

    /// The cursor to draw: hollow while the window is not focused, gone in the blink's off half
    /// and while an input method composes.
    fn cursor(&self, ctx: &Ctx, preedit: bool) -> (usize, usize, CursorShape) {
        let (r, c, shape) = self.w.snap.cursor;
        let shape = match shape {
            CursorShape::Hidden => CursorShape::Hidden,
            _ if preedit => CursorShape::Hidden,
            _ if !self.w.focused => CursorShape::HollowBlock,
            _ if self.settings.blink && !ctx.caret_on => CursorShape::Hidden,
            s => s,
        };
        (r, c, shape)
    }

    fn draw_main(&mut self, c: &mut Canvas, ctx: &Ctx) {
        let scale = c.scale();
        self.w.scale = scale;
        self.w.size = ctx.size;
        self.fit(ctx);
        let m = self.metrics(scale);
        let lay = self.chrome_layout(ctx);
        let canvas = (c.width, c.height, scale);
        if self.w.drawn != canvas {
            self.w.drawn = canvas;
            self.w.full = true;
        }
        let theme = &ctx.theme;
        if !self
            .w
            .chrome
            .drawn
            .as_ref()
            .is_some_and(|(t, a)| t == theme && *a == theme.active)
        {
            self.w.chrome.drawn = Some((theme.clone(), theme.active));
            self.w.full = true;
        }
        // An open menu lies over cells that may change under it, so it is drawn whole each time.
        let full = std::mem::take(&mut self.w.full) || self.w.chrome.menu.is_some();
        // The deeper report wins: the chrome's band, or the whole client when see-through.
        if chrome::on_glass(ctx) {
            c.report_glass_top(lay.band);
        }
        if self.background_alpha(theme) < 255 {
            c.report_glass_top(c.height as i32);
        }
        let chrome = std::mem::take(&mut self.w.chrome.dirty) || full;
        if chrome {
            self.draw_chrome(c, ctx, &lay);
        }
        let scheme = self.scheme(theme);
        if let Some(tab) = self.w.tabs.current() {
            tab.snapshot(&scheme, &mut self.w.snap);
        }
        if full {
            self.w.snap.dirty.fill(true);
        }
        let alpha = self.background_alpha(theme);
        if alpha != self.w.snap.alpha {
            self.w.snap.alpha = alpha;
            self.w.snap.dirty.fill(true);
        }
        let preedit = c.preedit().map(|p| p.text.clone());
        if let Some(text) = &preedit {
            snapshot::overlay_preedit(&mut self.w.snap, text);
        }
        let cursor = self.cursor(ctx, preedit.is_some());
        snapshot::move_cursor(&mut self.w.snap, self.w.shown, cursor);
        self.w.shown = cursor;
        let rects = termgrid::draw(c, &self.font, &m, lay.grid, &self.w.snap, self.px);
        self.w.snap.dirty.fill(false);
        self.draw_menu(c, ctx);
        // No report damages the whole canvas, so an unchanged frame reports an empty rect.
        if !full {
            if chrome {
                c.report_damage(Rect::new(0, 0, lay.toolbar.width, lay.band));
            }
            if rects.is_empty() && !chrome {
                c.report_damage(Rect::default());
            }
            for r in rects {
                c.report_damage(r);
            }
        }
        let caret = Rect::new(
            lay.grid.x + cursor.1 as i32 * m.cell_w / scale,
            lay.grid.y + cursor.0 as i32 * m.cell_h / scale,
            (m.cell_w / scale).max(1),
            (m.cell_h / scale).max(1),
        );
        c.report_spot(TextSpot {
            field: lay.grid,
            caret,
            surrounding: None,
            secret: false,
            multiline: true,
            can_undo: false,
            selected: false,
        });
    }
}

impl App for Terminal {
    fn title(&self) -> String {
        pane_title(self.pane(MAIN))
    }

    fn hidden(&self) -> bool {
        !self.order.contains(&MAIN)
    }

    fn app_id(&self) -> String {
        "reprise.terminal".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.window.unwrap_or_else(|| {
            let m = self.metrics.1;
            let band = chrome::band_height(&repriseapp::start_theme(), self.w.caption);
            (OPEN_CELLS.0 * m.cell_w, OPEN_CELLS.1 * m.cell_h + band)
        })
    }

    fn children(&self) -> Vec<Child> {
        let Some((text, forms)) = &self.forms else {
            return Vec::new();
        };
        let u = &forms.text;
        let mut out: Vec<Child> = self
            .order
            .iter()
            .filter(|&&id| id != MAIN)
            .filter_map(|&id| self.pane(id).map(|p| (id, p)))
            .map(|(id, p)| Child {
                id,
                title: pane_title(Some(p)),
                size: match p.size {
                    (0, 0) => self.initial_size(),
                    size => size,
                },
                fixed: false,
                modal: false,
            })
            .collect();
        if let Some(o) = &self.options {
            out.push(Child {
                id: OPTIONS,
                title: "Terminal Options".into(),
                size: u.size(options::CLIENT),
                fixed: true,
                modal: false,
            });
            if o.editing.is_some() {
                out.push(Child {
                    id: PROFILE,
                    title: "Profile".into(),
                    size: u.size(options::PROFILE_CLIENT),
                    fixed: true,
                    modal: true,
                });
            }
        }
        if self.find.is_some() {
            out.push(Child {
                id: FIND,
                title: "Find".into(),
                size: u.size(finddlg::FIND_CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(b) = &self.notfound {
            out.push(b.child(NOTFOUND, text, forms));
        }
        if let Some((_, b)) = &self.confirm {
            out.push(b.child(CONFIRM, text, forms));
        }
        out
    }

    fn parent(&self, win: WinId) -> Option<WinId> {
        match win {
            FIND | NOTFOUND => Some(self.find_on),
            OPTIONS | PROFILE => Some(self.options_on),
            CONFIRM => match self.confirm {
                Some((Closing::Window(w), _)) => Some(w),
                Some((Closing::Tab(pid), _)) => self.pane_of_pid(pid).or(Some(self.cur)),
                None => Some(self.cur),
            },
            _ if self.order.contains(&win) => None,
            _ => Some(MAIN),
        }
    }

    fn tab_with(&self, win: WinId) -> Option<WinId> {
        self.pane(win).and_then(|p| p.leader)
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        self.pane(win).map(|_| repriseapp::MIN_SIZE)
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        self.waker = Some(waker);
        self.tab_groups = repriseapp::tab_groups();
        self.w.caption = self.caption_mode();
        let shell = Some(&self.settings.shell)
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.into()]);
        let launch = Launch {
            argv: self.cmd.take().or(shell),
            name: None,
        };
        self.spawn_tab(&launch, None);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let reloaded = matches!(ev, Event::Focus(true)) && self.reload();
        let reply = self.route(win, ev, ctx);
        match reloaded {
            true => reply.and(Reply::redraw_all()),
            false => reply,
        }
    }

    fn follow_scale(&mut self, win: WinId, scale: i32, ctx: &mut Ctx) {
        if !self.enter(win) {
            return;
        }
        if scale != self.w.scale || self.w.chrome.bar.items.is_empty() {
            self.w.chrome.bar.items = chrome::toolbar_items(ctx, self.w.caption);
            self.w.chrome.dirty = true;
        }
        self.w.scale = scale;
        self.fit(ctx);
    }

    fn keeps_canvas(&self, win: WinId) -> bool {
        self.order.contains(&win)
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        if win == OPTIONS || win == PROFILE {
            self.draw_options(win, c, ctx);
            return;
        }
        if win == NOTFOUND {
            if let Some(b) = &self.notfound {
                b.draw(c, &ctx.forms(), &ctx.text);
            }
            return;
        }
        if win == FIND {
            self.draw_find(c, ctx);
            return;
        }
        if win == CONFIRM {
            if let Some((_, b)) = &self.confirm {
                b.draw(c, &ctx.forms(), &ctx.text);
            }
            return;
        }
        if self.enter(win) {
            self.draw_main(c, ctx);
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == OPTIONS || win == PROFILE {
            self.access_options(win, tree, ctx);
            return;
        }
        if win == NOTFOUND {
            if let Some(b) = &self.notfound {
                b.access(tree, &ctx.text, &ctx.forms());
            }
            return;
        }
        if win == FIND {
            self.access_find(tree, ctx);
            return;
        }
        if win == CONFIRM {
            if let Some((_, b)) = &self.confirm {
                b.access(tree, &ctx.text, &ctx.forms());
            }
            return;
        }
        if !self.enter(win) {
            return;
        }
        let lay = self.chrome_layout(ctx);
        let bar = &self.w.chrome.bar;
        toolbar::access(bar, lay.toolbar, &lay.buttons, tree, ROOT, "toolbar");
        if !self.w.caption {
            tabstrip::doc_access(&self.w.tabs.strip, &lay.strip, tree, ROOT, "tabs");
        }
        termgrid::access(&self.w.snap, lay.grid, tree, ROOT, "grid");
    }
}

impl Terminal {
    /// The event for window `win`, after any settings reload.
    fn route(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if !matches!(ev, Event::Wake) || self.forms.is_none() {
            self.forms = Some((ctx.text.clone(), ctx.forms()));
        }
        if win == CONFIRM {
            return self.confirm_event(ev);
        }
        if win == OPTIONS || win == PROFILE {
            return self.options_event(win, ev, ctx);
        }
        if win == NOTFOUND {
            return self.notfound_event(ev);
        }
        if win == FIND {
            self.enter(self.find_on);
            return self.find_event(ev, ctx);
        }
        if ev == Event::Wake {
            return self.wake(ctx);
        }
        if !self.enter(win) {
            return Reply::none();
        }
        if self.w.tabs.list.is_empty() {
            return self.close_pane();
        }
        match ev {
            Event::Key(k) => self.key(&k, ctx),
            Event::Pointer(p) => self.pointer(p, ctx),
            Event::Pasted(text) | Event::PastedPrimary(text) => {
                if let Some(tab) = self.w.tabs.current() {
                    tab.scroll(Scroll::Bottom);
                    tab.write(keys::bracket_paste(&text, tab.mode()).into());
                }
                Reply::redraw()
            }
            Event::Focus(f) => {
                self.w.focused = f;
                self.chrome_changed()
            }
            Event::Blink if self.w.focused && self.settings.blink => Reply::redraw(),
            Event::CloseRequest => self.close_window(ctx),
            Event::Resized => {
                if !ctx.maximized && !ctx.at_pick {
                    self.window = Some(ctx.size);
                }
                self.fit(ctx);
                Reply::redraw()
            }
            _ => Reply::none(),
        }
    }

    fn options_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(o) = &mut self.options else {
            return Reply::none();
        };
        let edit = match win {
            PROFILE => o.editing.as_ref().map(|p| &p.form),
            _ => Some(o.sheet.open()),
        };
        let clip = form_clipboard(edit.and_then(|f| f.focused_edit()), &ev);
        let reply = if win == PROFILE {
            match &ev {
                Event::CloseRequest => {
                    o.profile_event(form::Event::Cancel);
                    Reply::redraw_all()
                }
                _ => match o
                    .editing
                    .as_mut()
                    .and_then(|p| form_input(&mut p.form, &ev, ctx))
                {
                    Some(fev) => {
                        o.profile_event(fev);
                        Reply::redraw_all()
                    }
                    None => Reply::none(),
                },
            }
        } else if matches!(ev, Event::CloseRequest) {
            self.options = None;
            Reply::redraw_all()
        } else {
            match sheet_input(&mut o.sheet, &ev, ctx).map(|e| o.event(e)) {
                None | Some(Outcome::None) => Reply::none(),
                Some(Outcome::Redraw) => Reply::redraw_all(),
                Some(Outcome::Apply(s)) => {
                    self.apply(s);
                    Reply::redraw_all()
                }
                Some(Outcome::ApplyAndClose(s)) => {
                    self.apply(s);
                    self.options = None;
                    Reply::redraw_all()
                }
                Some(Outcome::Close) => {
                    self.options = None;
                    Reply::redraw_all()
                }
            }
        };
        Reply {
            copy: clip.copy,
            paste: clip.paste,
            ..reply
        }
    }

    fn access_options(&self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let Some(o) = &self.options else {
            return;
        };
        let px = ctx.forms().fonts.size_px;
        match win {
            OPTIONS => {
                oneshot::access_sheet(&o.sheet, tree, &ctx.text, px);
            }
            _ => {
                if let Some(p) = &o.editing {
                    oneshot::access_form(&p.form, tree, &ctx.text, px);
                }
            }
        }
    }

    fn draw_options(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let Some(o) = &self.options else {
            return;
        };
        let forms = ctx.forms();
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        let px = forms.fonts.size_px;
        c.fill_rect(c.bounds(), forms.colors.button_face);
        match win {
            OPTIONS => propsheet::draw(c, &ctx.text, &paint, &o.sheet, true, px),
            _ => {
                if let Some(p) = &o.editing {
                    form::draw(c, &ctx.text, &paint, &p.form, true, px);
                }
            }
        }
    }
}

/// Each window's pane: the one at hand and the parked ones.
fn panes_mut<'a>(
    w: &'a mut Pane,
    parked: &'a mut [(WinId, Pane)],
) -> impl Iterator<Item = &'a mut Pane> {
    std::iter::once(w).chain(parked.iter_mut().map(|(_, p)| p))
}

/// A window's title: its open tab's.
fn pane_title(p: Option<&Pane>) -> String {
    p.and_then(|p| p.tabs.current())
        .map_or_else(|| "Terminal".into(), |t| t.title().to_owned())
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn main() {
    reprisebus::journal::init_logger("error");
    let settings = settings::load(&settings::path());
    let cmd = command(std::env::args_os().skip(1));
    repriseapp::run(Box::new(Terminal::new(settings, cmd)));
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
