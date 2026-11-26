//! xdg-activation-v1. A trusted token focuses the window; any other only flags
//! the taskbar button. Trust is stamped at mint, or granted by [`Raises`].

use std::time::{Duration, Instant};

use smithay::{
    input::Seat,
    reexports::wayland_server::{Resource, backend::ClientId, protocol::wl_surface::WlSurface},
    utils::Serial,
    wayland::{
        compositor::get_parent,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
    },
};

use crate::compositor::State;
use reprisefs::apps::{AppId, DesktopId};

const TOKEN_LIFETIME: Duration = Duration::from_secs(30);

/// The most live tokens one client holds; Smithay drops a token `token_created` refuses.
const TOKENS_PER_CLIENT: usize = 32;

struct Trusted;

/// A client's new token earns trust only with an input serial on our seat from the client's
/// [`KeyboardTurn`] and a surface of the client holding keyboard focus; a pointer enter alone
/// yields a valid serial.
fn client_token_trusted(
    serial_ok: bool,
    token_client: Option<ClientId>,
    focus_client: Option<ClientId>,
) -> bool {
    serial_ok && token_client.is_some() && token_client == focus_client
}

/// The client holding the keyboard and the enter that gave it the keyboard. The keyboard moving
/// between that client's surfaces (to its menu and back, from a closed dialog to its window) keeps
/// the enter, so an input on the menu or the dialog still counts; a move to another client or to
/// no surface starts a new turn.
#[derive(Default)]
pub struct KeyboardTurn(Option<(ClientId, Serial)>);

impl KeyboardTurn {
    /// The keyboard entered a surface of `client` on `serial`.
    pub fn entered(&mut self, client: Option<ClientId>, serial: Option<Serial>) {
        match (client, serial) {
            (Some(c), _) if self.0.as_ref().is_some_and(|(held, _)| *held == c) => {}
            (Some(c), Some(s)) => self.0 = Some((c, s)),
            _ => self.0 = None,
        }
    }

    /// The keyboard left every surface.
    pub fn left(&mut self) {
        self.0 = None;
    }

    /// The enter that started `client`'s turn, when the turn is its.
    fn since(&self, client: Option<&ClientId>) -> Option<Serial> {
        (self.0.as_ref())
            .filter(|(held, _)| Some(held) == client)
            .map(|(_, s)| *s)
    }
}

/// How long the client a drag was dropped on, or the last finger landed on, may raise a window:
/// the drop or the tap is the user's intent, though the keyboard is elsewhere.
const PICK_GRACE: Duration = Duration::from_secs(3);

fn picked_trusted<C: PartialEq>(
    picked: Option<&(C, Instant)>,
    token_client: Option<&C>,
    now: Instant,
) -> bool {
    picked.is_some_and(|(c, at)| {
        Some(c) == token_client && now.saturating_duration_since(*at) < PICK_GRACE
    })
}

/// Trusted tokens one tap may mint; a launcher may ask twice for one start.
const TOKENS_PER_TAP: u8 = 2;

/// Programs Reprise just asked to act, by desktop id, for those the token never reaches: a search
/// provider call carries none, and some programs drop a notification action's. The next
/// activation a window of one asks for within [`TOKEN_LIFETIME`] is trusted, once.
#[derive(Default)]
pub struct Raises(Vec<(DesktopId, Instant)>);

impl Raises {
    pub fn grant(&mut self, app: DesktopId, now: Instant) {
        self.0
            .retain(|(a, at)| *a != app && now.saturating_duration_since(*at) < TOKEN_LIFETIME);
        self.0.push((app, now));
    }

    /// Uses up the live grant for `app`; false when there is none.
    fn take(&mut self, app: Option<&DesktopId>, now: Instant) -> bool {
        self.0
            .retain(|(_, at)| now.saturating_duration_since(*at) < TOKEN_LIFETIME);
        let Some(i) = app.and_then(|app| self.0.iter().position(|(a, _)| a == app)) else {
            return false;
        };
        self.0.remove(i);
        true
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        prune(&mut self.activation);
        let held = (self.activation.tokens())
            .filter(|(_, d)| d.client_id.is_some() && d.client_id == data.client_id)
            .count();
        if held >= TOKENS_PER_CLIENT {
            return false;
        }
        let keyboard = self.seat.get_keyboard();
        let focus_client = (keyboard.as_ref())
            .and_then(|k| k.current_focus())
            .and_then(|s| s.client())
            .map(|c| c.id());
        let since = (self.keyboard_turn.since(focus_client.as_ref()))
            .or_else(|| keyboard.as_ref().and_then(|k| k.last_enter()));
        let serial = (data.serial.as_ref())
            .filter(|(_, seat)| Seat::<State>::from_resource(seat).as_ref() == Some(&self.seat))
            .map(|(serial, _)| *serial);
        let serial_ok = (serial.zip(since)).is_some_and(|(s, since)| s.is_no_older_than(&since));
        let now = Instant::now();
        let tapped = self.tap_trusted(serial, data.client_id.as_ref(), now);
        let token_client = data
            .surface
            .as_ref()
            .and_then(|s| s.client())
            .map(|c| c.id());
        let dropped = picked_trusted(self.dnd.dropped_on.as_ref(), token_client.as_ref(), now);
        if dropped {
            self.dnd.dropped_on = None;
        }
        if dropped || tapped || client_token_trusted(serial_ok, token_client, focus_client) {
            data.user_data.insert_if_missing(|| Trusted);
        }
        true
    }

    /// Focus the window for a trusted token, else flag it. A window not yet
    /// mapped keeps the verdict for `map_window`.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let live = data.timestamp.elapsed() < TOKEN_LIFETIME;
        let minted_trusted = live && data.user_data.get::<Trusted>().is_some();
        self.activation.remove_token(&token);
        let mut root = surface;
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let Some(w) = self.window_for_surface(&root) else {
            return;
        };
        let app = (self.win(w)).and_then(|win| {
            reprisefs::apps::AppId::new(&win.app_id())
                .and_then(|a| reprisefs::apps::entry_for_app_id(&self.app_index, &a, |_| true))
                .map(|e| e.id.clone())
                .filter(|id| crate::sandbox::may_take_raise(win, id))
        });
        let trusted = minted_trusted || live && self.raises.take(app.as_ref(), Instant::now());
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !win.mapped {
            win.activate_on_map |= trusted;
            return;
        }
        let minimized = win.minimized;
        log::info!(
            "activation of {w:?} ({:?}): {}",
            win.app_id(),
            if trusted { "focused" } else { "attention" }
        );
        let focused = self.focused == Some(w);
        if trusted && !self.desktop_locked() {
            if minimized {
                self.set_minimized(w, false);
            } else {
                self.focus_window(w);
            }
        } else if !focused {
            self.ask_attention(w);
            self.redraw_taskbars();
        }
    }
}

impl State {
    /// Whether the newest tap trusts `client`'s token on `serial`: it was sent that down within
    /// [`PICK_GRACE`] and the tap minted fewer than [`TOKENS_PER_TAP`]. The client is checked, not
    /// the token's surface: a launcher panel without the keyboard names none.
    fn tap_trusted(
        &mut self,
        serial: Option<Serial>,
        client: Option<&ClientId>,
        now: Instant,
    ) -> bool {
        if self.console_locked() || self.desktop_locked() {
            return false;
        }
        let touch = &mut self.devices.touch;
        let Some(tap) = touch.tapped.as_mut() else {
            return false;
        };
        let picked = (tap.client.clone(), tap.at);
        if serial != Some(tap.serial) || !picked_trusted(Some(&picked), client, now) {
            return false;
        }
        tap.spent += 1;
        if tap.spent >= TOKENS_PER_TAP {
            touch.tapped = None;
        }
        true
    }
}

/// Drop expired tokens. Runs as each token is made, so what stays is at most the tokens made in
/// the [`TOKEN_LIFETIME`] before the newest, and no timer wakes an idle loop.
fn prune(activation: &mut XdgActivationState) {
    activation.retain_tokens(|_, d| d.timestamp.elapsed() < TOKEN_LIFETIME);
}

impl State {
    pub fn new_activation_token(&mut self) -> String {
        prune(&mut self.activation);
        let (token, data) = self.activation.create_external_token(None);
        data.user_data.insert_if_missing(|| Trusted);
        token.to_string()
    }

    /// A token for a program started now, marked by key when the last input was a keystroke.
    pub fn new_launch(&mut self) -> crate::spawn::Launch {
        crate::spawn::Launch {
            token: self.new_activation_token(),
            by_key: self.key_input.last_key_input,
        }
    }

    pub fn spawn_activated(&mut self, cmd: &str) -> bool {
        let launch = self.new_launch();
        crate::spawn::spawn(cmd, &self.env, Some(&launch))
    }

    /// `spawn_activated` for a command typed or bound by hand, in its [`crate::spawn::command_app`]
    /// scope.
    pub fn spawn_command_activated(&mut self, cmd: &str) -> bool {
        let app = crate::spawn::command_app(cmd, &self.app_index);
        let launch = self.new_launch();
        crate::spawn::spawn_in(cmd, app.as_ref(), &self.env, Some(&launch))
    }

    /// The configured terminal on its own in `dir`, in its [`crate::spawn::command_app`] scope.
    pub fn spawn_terminal_in(&mut self, dir: &std::path::Path) {
        let terminal = self.config.terminal.clone();
        let app = crate::spawn::command_app(&terminal, &self.app_index);
        let launch = self.new_launch();
        let argv = reprisefs::apps::terminal_argv(&terminal);
        crate::spawn::spawn_argv_in(&argv, app.as_ref(), Some(dir), &self.env, Some(&launch));
    }

    /// The line typed in Run box `from`: one that names a folder opens it in Explorer, a document
    /// opens as its icon does ([`State::open_document`]), anything else runs as
    /// [`State::spawn_command_activated`]. Each closes the box. A line whose program is not there
    /// keeps the box open under a notice. The paths are checked off the loop on a
    /// [`State::spawn_job`], so a mount that does not answer stalls neither the loop nor the launch.
    /// The box takes no second OK while its line is checked, and a check that lands after the box
    /// closed runs nothing.
    pub fn run_line_activated(&mut self, cmd: &str, from: u64) {
        if self.dialogs.checking.contains(&from) {
            return;
        }
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let folder = crate::spawn::command_folder(cmd, home.as_deref());
        let program = crate::spawn::command_program(cmd, home.as_deref());
        let path = std::env::var_os("PATH");
        let mime = (folder.as_ref().and_then(|d| d.file_name()))
            .and_then(|n| self.with_mime(|db| db.mime_of_name(&n.to_string_lossy())));
        let cmd = cmd.to_string();
        self.dialogs.checking.push(from);
        let started = self.spawn_job(move || {
            let line = match (folder, program) {
                (Some(dir), _) => match std::fs::metadata(&dir) {
                    Ok(m) if m.is_dir() => RunLine::Folder(dir),
                    Ok(m) => {
                        let fixed_modes = reprisefs::props::Mount::of(&dir).fixed_modes;
                        match names_a_document(&m, mime.as_deref(), fixed_modes) {
                            true => RunLine::Document(dir),
                            false => RunLine::Command,
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        RunLine::Missing(cmd.trim().to_string())
                    }
                    Err(_) => RunLine::Command,
                },
                (None, Some((word, p))) if p.missing(path.as_deref()) => RunLine::Missing(word),
                (None, _) => RunLine::Command,
            };
            Box::new(move |state: &mut State| state.run_line_checked(&cmd, from, line))
        });
        if !started {
            self.dialogs.checking.retain(|&id| id != from);
        }
    }

    fn run_line_checked(&mut self, cmd: &str, from: u64, line: RunLine) {
        self.dialogs.checking.retain(|&id| id != from);
        let Some(i) = self.dialog_index(from) else {
            return;
        };
        if let RunLine::Missing(word) = &line {
            let d = crate::dialog::Dialog::run_not_found(word);
            return self.open_dialog_over_id(d, Some(from));
        }
        self.close_dialog_at(i);
        crate::dialog::remember(&mut self.dialogs.run_history, cmd);
        crate::state::save_dialogs(&self.dialogs.run_history);
        match line {
            RunLine::Folder(dir) => {
                self.spawn_argv_activated(&reprisefs::apps::open_folder_argv(&dir))
            }
            RunLine::Document(path) => {
                let how = reprisefs::program::Confirm::on_open(self.config.confirm_scripts);
                self.open_document(&path, None, how, true);
            }
            RunLine::Command | RunLine::Missing(_) => {
                self.spawn_command_activated(cmd);
            }
        }
    }

    pub fn spawn_argv_activated<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
        &mut self,
        argv: &[S],
    ) {
        let launch = self.new_launch();
        crate::spawn::spawn_argv(argv, &self.env, Some(&launch))
    }

    /// Open a path as `open` says, in its program's app scope when it has one.
    pub fn spawn_opening_activated(&mut self, open: &reprisefs::props::Opening) {
        match &open.app {
            Some(app) => self.spawn_app_activated(&open.argv, &app.app_id(), open.dir.as_deref()),
            None => self.spawn_argv_activated(&open.argv),
        }
    }

    /// `spawn_argv_activated` for a command built from the desktop entry `app`, in its `Path=`
    /// folder `dir`.
    pub fn spawn_app_activated<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
        &mut self,
        argv: &[S],
        app: &AppId,
        dir: Option<&std::path::Path>,
    ) {
        let launch = self.new_launch();
        crate::spawn::spawn_argv_in(argv, Some(app), dir, &self.env, Some(&launch))
    }

    /// Start the desktop entry `app` with no file, in its app scope.
    pub fn launch_entry(&mut self, app: &reprisefs::apps::AppEntry) {
        match reprisefs::apps::launch_argv(app, &self.config.terminal) {
            Ok(argv) => self.spawn_app_activated(&argv, &app.id.app_id(), app.path.as_deref()),
            Err(refused) => self.exec_refused(&refused),
        }
    }

    /// Run `app`'s desktop action `action` in the app's scope, as [`State::launch_entry`] starts
    /// the app.
    pub fn launch_action(
        &mut self,
        app: &reprisefs::apps::AppEntry,
        action: &reprisefs::apps::DesktopAction,
    ) {
        match reprisefs::apps::action_argv(app, action, &self.config.terminal) {
            Ok(argv) => self.spawn_app_activated(&argv, &app.id.app_id(), app.path.as_deref()),
            Err(refused) => self.exec_refused(&refused),
        }
    }

    /// The error box for a desktop entry Reprise refuses to run; nothing runs.
    pub fn exec_refused(&mut self, refused: &reprisefs::apps::ExecRefused) {
        let d = crate::dialog::Dialog::error(&refused.name, refused.to_string());
        self.open_dialog(d);
    }

    /// `spawn_app_activated` for launcher `app`. One that runs `xdg-open <file>` (a Quick Launch
    /// document shortcut) on a regular file opens it as the file's desktop icon does
    /// ([`State::open_document`]). The file is checked off the loop on a [`State::spawn_job`].
    pub fn spawn_launcher_activated(
        &mut self,
        argv: &[std::ffi::OsString],
        app: &AppId,
        dir: Option<&std::path::Path>,
    ) {
        if crate::spawn::xdg_open_file(argv).is_none() {
            return self.spawn_app_activated(argv, app, dir);
        }
        let (argv, app, dir) = (
            argv.to_vec(),
            app.clone(),
            dir.map(std::path::Path::to_path_buf),
        );
        self.spawn_job(move || {
            let doc = crate::spawn::xdg_open_document(&argv);
            Box::new(move |state: &mut State| match doc {
                Some(file) => {
                    let how = reprisefs::program::Confirm::on_open(state.config.confirm_scripts);
                    state.open_document(&file, None, how, false)
                }
                None => state.spawn_app_activated(&argv, &app, dir.as_deref()),
            })
        });
    }
}

/// Whether a Run line's path is a document to record as opened: a regular file with no execute
/// bit, since an executable it names runs instead, or, on a mount whose modes are fixed (FAT,
/// exFAT and NTFS can mark every file executable), one whose name's type `mime` is no program's.
/// What a Run line was found to name, off the loop.
enum RunLine {
    Folder(std::path::PathBuf),
    Document(std::path::PathBuf),
    /// The program, as typed, is not there.
    Missing(String),
    /// Anything else, for `sh`.
    Command,
}

fn names_a_document(meta: &std::fs::Metadata, mime: Option<&str>, fixed_modes: bool) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file()
        && (meta.permissions().mode() & 0o111 == 0
            || fixed_modes && mime.is_some_and(|m| !PROGRAM_TYPES.contains(&m)))
}

const PROGRAM_TYPES: [&str; 5] = [
    "application/x-executable",
    "application/x-pie-executable",
    "application/x-sharedlib",
    "application/x-shellscript",
    "application/x-desktop",
];

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::wayland_server::Display;

    fn two_clients() -> (ClientId, ClientId) {
        let display: Display<()> = Display::new().expect("display");
        let mut dh = display.handle();
        let mut id = || {
            let (a, _b) = std::os::unix::net::UnixStream::pair().expect("pair");
            dh.insert_client(a, std::sync::Arc::new(NoData))
                .expect("client")
                .id()
        };
        (id(), id())
    }

    struct NoData;
    impl smithay::reexports::wayland_server::backend::ClientData for NoData {}

    #[test]
    fn token_from_the_keyboard_focused_client_is_trusted() {
        let (focused, _) = two_clients();
        assert!(
            client_token_trusted(true, Some(focused.clone()), Some(focused)),
            "expected trusted"
        );
    }

    #[test]
    fn token_from_a_client_without_keyboard_focus_is_not_trusted() {
        let (focused, hovered) = two_clients();
        assert!(
            !client_token_trusted(true, Some(hovered), Some(focused)),
            "expected untrusted"
        );
    }

    #[test]
    fn token_without_a_surface_is_not_trusted_even_when_nothing_has_focus() {
        assert!(
            !client_token_trusted(true, None, None),
            "expected untrusted"
        );
    }

    #[test]
    fn a_client_minting_a_hundred_tokens_holds_only_the_cap() {
        let mut h = crate::harness::Harness::new();
        let mut c = h.connect();
        for _ in 0..100 {
            c.mint_token();
        }
        h.roundtrip(&mut c);
        assert_eq!(h.state.activation.tokens().count(), TOKENS_PER_CLIENT);
    }

    #[test]
    fn a_drop_target_or_a_tapped_client_may_raise_a_window_for_a_moment() {
        let at = Instant::now();
        let picked = Some(&(1, at));
        assert!(picked_trusted(
            picked,
            Some(&1),
            at + Duration::from_secs(1)
        ));
        assert!(!picked_trusted(picked, Some(&2), at), "another client");
        assert!(!picked_trusted(picked, None, at), "no client");
        assert!(
            !picked_trusted(picked, Some(&1), at + PICK_GRACE),
            "the moment has passed"
        );
        assert!(!picked_trusted::<i32>(None, Some(&1), at));
    }

    #[test]
    fn a_raise_grant_lasts_a_tokens_lifetime() {
        let at = Instant::now();
        let mut r = Raises::default();
        let id = |n: &str| DesktopId::from_file_name(n).unwrap();
        r.grant(id("calc.desktop"), at);
        assert!(!r.take(Some(&id("chars.desktop")), at), "another program");
        assert!(!r.take(None, at), "a window of no known program");
        assert!(
            !r.take(Some(&id("calc.desktop")), at + TOKEN_LIFETIME),
            "expired"
        );
        r.grant(id("calc.desktop"), at);
        r.grant(id("calc.desktop"), at);
        assert!(r.take(Some(&id("calc.desktop")), at + Duration::from_secs(1)));
        assert!(
            !r.take(Some(&id("calc.desktop")), at + Duration::from_secs(1)),
            "a second pick of the same program renews its grant, it doesn't add one"
        );
    }

    use crate::harness::{Harness, TestClient, Toplevel};

    /// On a touchscreen: a program's `target` window behind `busy`, another program's, which holds
    /// the keyboard, and a launcher whose 200x30 panel in the top-left corner takes no keyboard.
    struct Launcher {
        h: Harness,
        launcher: TestClient,
        app: TestClient,
        target: Toplevel,
        /// Its client stays connected while the test runs.
        busy: (TestClient, Toplevel),
    }

    fn launcher() -> Launcher {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.add_touchscreen();
        let mut app = h.connect();
        let target = h.map_toplevel(&mut app, "test.app", 300, 200);
        let mut other = h.connect();
        let busy = h.map_toplevel(&mut other, "test.busy", 300, 200);
        let mut launcher = h.connect();
        h.map_panel(&mut launcher);
        launcher.bind_touch();
        h.roundtrip(&mut launcher);
        h.state.focus_window(busy.id);
        h.roundtrip(&mut other);
        Launcher {
            h,
            launcher,
            app,
            target,
            busy: (other, busy),
        }
    }

    impl Launcher {
        /// A tap on the panel; the down's serial the launcher heard.
        fn tap_panel(&mut self) -> u32 {
            self.h.tap(100.0, 15.0);
            self.h.roundtrip(&mut self.launcher);
            self.launcher.side.touch_down_serial
        }

        /// A token the launcher asks on `serial`, naming no surface as GTK does without the
        /// keyboard.
        fn mint(&mut self, serial: u32) -> String {
            self.launcher.token_from(serial, None);
            self.h.roundtrip(&mut self.launcher);
            self.launcher.side.tokens.last().expect("a token").clone()
        }

        /// `token` presented for `target`.
        fn present(&mut self, token: &str) {
            self.app.activate(token, self.target);
            self.h.roundtrip(&mut self.app);
        }

        /// [`Launcher::mint`] on `serial`, presented for `target`.
        fn raise_on(&mut self, serial: u32) {
            let token = self.mint(serial);
            self.present(&token);
        }

        /// A point inside `busy`'s window, clear of the panel.
        fn busy_point(&self) -> (f64, f64) {
            let w = self.h.state.win(self.busy.1.id).expect("busy");
            let (x, y) = (w.x + w.geom.client.x + 20, w.y + w.geom.client.y + 40);
            (f64::from(x), f64::from(y))
        }

        fn keyboard_on_busy(&self) -> bool {
            self.h.state.focused == Some(self.busy.1.id)
        }
    }

    #[test]
    fn a_tap_on_a_launcher_without_the_keyboard_lets_it_raise_a_window() {
        let mut l = launcher();
        let down = l.tap_panel();
        assert_ne!(down, 0, "the panel hears the finger");
        assert_eq!(
            l.h.state.focused,
            Some(l.busy.1.id),
            "the panel takes no keyboard"
        );
        l.raise_on(down);
        assert_eq!(
            l.h.state.focused,
            Some(l.target.id),
            "the window takes the keyboard"
        );
    }

    #[test]
    fn a_tap_on_a_launcher_raises_nothing_on_an_older_tap_or_after_a_tap_elsewhere() {
        let mut l = launcher();
        let first = l.tap_panel();
        let second = l.tap_panel();
        l.raise_on(first);
        assert_eq!(l.h.state.focused, Some(l.busy.1.id), "on the older tap");

        let w = l.h.state.win(l.busy.1.id).expect("busy");
        let (x, y) = (w.x + w.geom.client.x + 20, w.y + w.geom.client.y + 20);
        l.h.tap(f64::from(x), f64::from(y));
        l.raise_on(second);
        assert_eq!(
            l.h.state.focused,
            Some(l.busy.1.id),
            "after a finger landed elsewhere"
        );
        assert!(
            l.h.state
                .win(l.target.id)
                .is_some_and(|w| w.attention.is_some())
        );
    }

    #[test]
    fn a_program_that_was_not_tapped_raises_nothing_on_the_taps_serial() {
        let mut l = launcher();
        let down = l.tap_panel();
        l.app.token_on(down, l.target);
        l.h.roundtrip(&mut l.app);
        let token = l.app.side.tokens.last().expect("a token").clone();
        l.app.activate(&token, l.target);
        l.h.roundtrip(&mut l.app);
        assert_eq!(
            l.h.state.focused,
            Some(l.busy.1.id),
            "busy keeps the keyboard"
        );
    }

    #[test]
    fn a_finger_a_drag_keeps_from_the_launcher_lets_it_raise_nothing() {
        let mut l = launcher();
        l.busy.0.bind_touch();
        l.h.roundtrip(&mut l.busy.0);
        let (x, y) = l.busy_point();
        l.h.finger_down(0, x, y);
        l.h.roundtrip(&mut l.busy.0);
        let serial = l.busy.0.side.touch_down_serial;
        let source = l.busy.0.data_source(&["text/plain"], b"x");
        let origin = l.busy.0.side.shells[l.busy.1.shell].surface.clone();
        (l.busy.0.data_device()).start_drag(Some(&source), &origin, None, serial);
        l.h.roundtrip(&mut l.busy.0);
        assert!(l.h.state.dnd.running(), "busy drags under the first finger");

        l.h.finger_down(1, 100.0, 15.0);
        l.h.roundtrip(&mut l.launcher);
        assert_eq!(
            l.launcher.side.touch_down_serial, 0,
            "the panel hears no down"
        );
        let down = (l.h.state.seat.get_touch())
            .and_then(|t| t.last_down())
            .expect("the second finger's down");
        l.raise_on(down.into());
        assert!(l.keyboard_on_busy(), "busy keeps the keyboard");
    }

    #[test]
    fn a_second_finger_on_the_launcher_lets_it_raise_a_window() {
        let mut l = launcher();
        l.busy.0.bind_touch();
        l.h.roundtrip(&mut l.busy.0);
        let (x, y) = l.busy_point();
        l.h.finger_down(0, x, y);
        l.h.finger_down(1, 100.0, 15.0);
        l.h.roundtrip(&mut l.launcher);
        let down = l.launcher.side.touch_down_serial;
        assert_ne!(down, 0, "the panel hears the second finger");
        l.raise_on(down);
        assert_eq!(l.h.state.focused, Some(l.target.id));
    }

    #[test]
    fn a_tap_on_a_launcher_raises_nothing_on_a_serial_it_was_not_sent() {
        let mut l = launcher();
        let down = l.tap_panel();
        l.raise_on(down.wrapping_add(1000));
        assert!(l.keyboard_on_busy(), "on a later serial");
    }

    #[test]
    fn a_tap_on_a_launcher_lets_it_raise_twice() {
        let mut l = launcher();
        let down = l.tap_panel();
        let tokens: Vec<String> = (0..3).map(|_| l.mint(down)).collect();
        let mut raised = Vec::new();
        for token in &tokens {
            l.h.state.focus_window(l.busy.1.id);
            l.present(token);
            raised.push(l.h.state.focused == Some(l.target.id));
        }
        assert_eq!(raised, [true, true, false]);
    }

    #[test]
    fn a_tap_on_a_launcher_raises_nothing_after_a_click_elsewhere() {
        let mut l = launcher();
        let down = l.tap_panel();
        let (x, y) = l.busy_point();
        l.h.click(x, y);
        l.raise_on(down);
        assert!(l.keyboard_on_busy());
    }

    #[test]
    fn a_tap_on_a_launcher_raises_nothing_after_a_key_another_program_hears() {
        let mut l = launcher();
        let down = l.tap_panel();
        l.h.key(30, true);
        l.h.key(30, false);
        l.raise_on(down);
        assert!(l.keyboard_on_busy());
    }

    #[test]
    fn a_tap_on_a_launcher_raises_nothing_once_the_session_locked() {
        let mut l = launcher();
        let down = l.tap_panel();
        crate::lockview::lock(&mut l.h.state);
        l.h.dispatch();
        let token = l.mint(down);
        crate::lockview::unlock(&mut l.h.state);
        l.h.dispatch();
        l.h.state.focus_window(l.busy.1.id);
        l.present(&token);
        assert!(l.keyboard_on_busy());
    }

    /// A click in the middle of window `t`, given the keyboard first; the serial `c` heard for it.
    fn click_on(h: &mut Harness, c: &mut TestClient, t: Toplevel) -> u32 {
        h.state.focus_window(t.id);
        h.roundtrip(c);
        let w = h.state.win(t.id).expect("the window");
        let (client, x, y) = (w.geom.client, w.x, w.y);
        let (x, y) = (
            x + client.x + client.width / 2,
            y + client.y + client.height / 2,
        );
        h.click(f64::from(x), f64::from(y));
        h.roundtrip(c);
        c.side.button_serial
    }

    /// A token `c` asks on `serial` from `from`, presented for `raise`.
    fn raise_on(h: &mut Harness, c: &mut TestClient, serial: u32, from: Toplevel, raise: Toplevel) {
        c.token_on(serial, from);
        h.roundtrip(c);
        let token = c.side.tokens.last().expect("a token").clone();
        c.activate(&token, raise);
        h.roundtrip(c);
    }

    #[test]
    fn a_pick_from_a_programs_menu_raises_another_of_its_windows_after_the_menu_closes() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let sheet = h.map_toplevel(&mut c, "test.sheet", 300, 200);
        let folder = h.map_toplevel(&mut c, "test.folder", 300, 200);
        click_on(&mut h, &mut c, folder);
        let menu = h.map_popup(&mut c, folder, 100, 80);
        assert_eq!(c.side.keyboard_focus.as_ref(), Some(&menu.surface));
        let w = h.state.win(folder.id).expect("the folder");
        let (x, y) = (w.x + w.geom.client.x + 10, w.y + w.geom.client.y + 10);
        h.click(f64::from(x), f64::from(y));
        h.roundtrip(&mut c);
        let pick = c.side.button_serial;
        let enters = c.side.enters;
        c.hide_popup(&menu);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.enters,
            enters + 1,
            "the keyboard comes back to the folder"
        );
        raise_on(&mut h, &mut c, pick, folder, sheet);
        assert_eq!(
            h.state.focused,
            Some(sheet.id),
            "the sheet takes the keyboard"
        );
    }

    #[test]
    fn a_click_in_a_programs_dialog_raises_another_of_its_windows_after_the_dialog_closes() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let sheet = h.map_toplevel(&mut c, "test.sheet", 300, 200);
        let folder = h.map_toplevel(&mut c, "test.folder", 300, 200);
        let dialog = h.map_dialog(&mut c, "test.folder", Some(folder), "", (200, 100));
        let ok = click_on(&mut h, &mut c, dialog);
        c.destroy(dialog);
        h.roundtrip(&mut c);
        assert_eq!(
            h.state.focused,
            Some(folder.id),
            "the keyboard is back on the folder"
        );
        raise_on(&mut h, &mut c, ok, folder, sheet);
        assert_eq!(
            h.state.focused,
            Some(sheet.id),
            "the sheet takes the keyboard"
        );
    }

    #[test]
    fn a_token_minted_before_the_lock_raises_nothing_under_it() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let sheet = h.map_toplevel(&mut c, "test.sheet", 300, 200);
        let folder = h.map_toplevel(&mut c, "test.folder", 300, 200);
        let pick = click_on(&mut h, &mut c, folder);
        c.token_on(pick, folder);
        h.roundtrip(&mut c);
        let token = c.side.tokens.last().expect("a token").clone();
        crate::lockview::lock(&mut h.state);
        h.dispatch();
        c.activate(&token, sheet);
        h.roundtrip(&mut c);
        assert_eq!(h.state.focused, Some(folder.id), "the folder stays active");
        assert!(
            h.state.win(sheet.id).is_some_and(|w| w.attention.is_some()),
            "the sheet asks for attention"
        );
    }

    #[test]
    fn a_program_without_the_keyboard_raises_nothing_on_a_serial_as_new_as_the_focused_ones() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut other = h.connect();
        let behind = h.map_toplevel(&mut other, "test.other", 300, 200);
        let second = h.map_toplevel(&mut other, "test.other", 300, 200);
        let mut c = h.connect();
        let folder = h.map_toplevel(&mut c, "test.folder", 300, 200);
        let click = click_on(&mut h, &mut c, folder);
        raise_on(&mut h, &mut other, click, behind, second);
        assert_eq!(
            h.state.focused,
            Some(folder.id),
            "the folder keeps the keyboard"
        );
        assert!(
            h.state
                .win(second.id)
                .is_some_and(|w| w.attention.is_some())
        );
    }

    #[test]
    fn a_serial_from_before_the_keyboard_went_elsewhere_raises_nothing() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut other = h.connect();
        let elsewhere = h.map_toplevel(&mut other, "test.other", 300, 200);
        let mut c = h.connect();
        let sheet = h.map_toplevel(&mut c, "test.sheet", 300, 200);
        let folder = h.map_toplevel(&mut c, "test.folder", 300, 200);
        let before = click_on(&mut h, &mut c, folder);
        click_on(&mut h, &mut other, elsewhere);
        h.state.focus_window(folder.id);
        h.roundtrip(&mut c);
        raise_on(&mut h, &mut c, before, folder, sheet);
        assert_eq!(
            h.state.focused,
            Some(folder.id),
            "after another program had it"
        );

        let before = click_on(&mut h, &mut c, folder);
        h.state.focus_desktop();
        h.state.focus_window(folder.id);
        h.roundtrip(&mut c);
        raise_on(&mut h, &mut c, before, folder, sheet);
        assert_eq!(h.state.focused, Some(folder.id), "after the desktop had it");
    }

    #[test]
    fn token_without_a_serial_is_not_trusted() {
        let (focused, _) = two_clients();
        assert!(
            !client_token_trusted(false, Some(focused.clone()), Some(focused)),
            "expected untrusted"
        );
    }

    #[test]
    fn a_run_line_opens_a_document_and_names_a_missing_program_over_the_open_box() {
        use std::os::unix::fs::PermissionsExt;
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let run = |h: &mut crate::harness::Harness| {
            h.state.open_dialog(crate::dialog::Dialog::run(&[]));
            h.state.dialogs.views.last().expect("Run").id
        };
        crate::spawn::record();
        let from = run(&mut h);
        h.state
            .run_line_activated("reprise-no-such-program --flag", from);
        h.until("the notice", |s| s.dialogs.views.len() == 2);
        assert!(
            h.state.dialog_index(from).is_some(),
            "the Run box stays open"
        );
        assert!(
            (h.state.dialogs.views[1].dialog.text)
                .starts_with("Reprise cannot find 'reprise-no-such-program'."),
            "the notice names the program"
        );
        assert_eq!(
            crate::spawn::recorded(),
            Vec::<String>::new(),
            "nothing runs"
        );
        h.state.close_all_dialogs();

        let dir = crate::harness::scratch().join("run-line");
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("notes.txt");
        std::fs::write(&doc, "").unwrap();
        std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o644)).unwrap();
        let from = run(&mut h);
        h.state.run_line_activated(&doc.display().to_string(), from);
        h.until("the document opens", |s| {
            s.dialog_index(from).is_none()
                && (!s.dialogs.views.is_empty() || !crate::spawn::recorded().is_empty())
        });
        assert!(
            !crate::spawn::recorded().contains(&"notes.txt".to_string()),
            "the document is not handed to the shell"
        );
    }

    /// Run with `line` in its field, its line checked on a thread, and starts recorded.
    fn run_box(line: &str) -> crate::harness::Harness {
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.output("o", 800, 600);
        h.state
            .open_dialog(crate::dialog::Dialog::run(&[line.to_string()]));
        crate::spawn::record();
        h
    }

    /// Key `code` down and up without turning the loop, so no check lands in between.
    fn tap(h: &mut crate::harness::Harness, code: u32) {
        use smithay::backend::input::InputTime;
        for pressed in [true, false] {
            h.state
                .with_source(crate::inputproto::InputSource::Physical, |s| {
                    s.handle_key((code + 8).into(), pressed, InputTime::now())
                });
        }
    }

    const ENTER: u32 = 28;

    #[test]
    fn enter_twice_in_run_before_its_line_is_checked_runs_it_once() {
        let mut h = run_box("true");
        tap(&mut h, ENTER);
        tap(&mut h, ENTER);
        h.until("the line is checked", |s| s.dialogs.checking.is_empty());
        assert_eq!(crate::spawn::recorded().len(), 1, "the line runs once");
        assert!(h.state.dialogs.views.is_empty(), "Run closes");
    }

    #[test]
    fn a_line_checked_after_run_closed_runs_nothing() {
        const ESCAPE: u32 = 1;
        let mut h = run_box("true");
        tap(&mut h, ENTER);
        tap(&mut h, ESCAPE);
        assert!(h.state.dialogs.views.is_empty(), "Escape closes Run");
        h.until("the line is checked", |s| s.dialogs.checking.is_empty());
        assert_eq!(
            crate::spawn::recorded(),
            Vec::<String>::new(),
            "nothing runs"
        );
    }

    #[test]
    fn a_run_line_records_a_document_but_not_a_program() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-run-line-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (doc, prog) = (dir.join("notes.txt"), dir.join("backup.sh"));
        std::fs::write(&doc, "").unwrap();
        std::fs::write(&prog, "").unwrap();
        std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
        let meta = |p: &std::path::Path| std::fs::metadata(p).unwrap();
        assert!(
            names_a_document(&meta(&doc), None, false),
            "a 0644 file is recorded"
        );
        assert!(
            !names_a_document(&meta(&prog), None, false),
            "a 0755 file is not"
        );
        let py = dir.join("x.py");
        std::fs::write(&py, "").unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fixed = reprisefs::props::Mount::of(&py).fixed_modes;
        assert!(
            !names_a_document(&meta(&py), Some("text/x-python"), fixed),
            "nor a 0755 x.py where modes are kept"
        );
        assert!(
            !names_a_document(&meta(&prog), Some("application/x-shellscript"), true),
            "nor a 0755 script where modes are fixed"
        );
        std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            names_a_document(&meta(&doc), Some("text/plain"), true),
            "a 0777 document where modes are fixed, as on vfat, is recorded"
        );
        assert!(
            !names_a_document(&meta(&dir), Some("text/plain"), true),
            "a folder is not"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
