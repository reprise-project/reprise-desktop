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

/// How long the client a drag was dropped on may raise the window it took it with: the drop
/// is the user's intent, though the keyboard is still on the drag's source.
const DROP_GRACE: Duration = Duration::from_secs(3);

fn drop_target_trusted<C: PartialEq>(
    dropped_on: Option<&(C, Instant)>,
    token_client: Option<&C>,
    now: Instant,
) -> bool {
    dropped_on.is_some_and(|(c, at)| {
        Some(c) == token_client && now.saturating_duration_since(*at) < DROP_GRACE
    })
}

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
        let serial_ok = data.serial.as_ref().is_some_and(|(serial, seat)| {
            Seat::<State>::from_resource(seat).as_ref() == Some(&self.seat)
                && since.is_some_and(|since| serial.is_no_older_than(&since))
        });
        let token_client = data
            .surface
            .as_ref()
            .and_then(|s| s.client())
            .map(|c| c.id());
        let dropped = drop_target_trusted(
            self.dnd.dropped_on.as_ref(),
            token_client.as_ref(),
            Instant::now(),
        );
        if dropped {
            self.dnd.dropped_on = None;
        }
        if dropped || client_token_trusted(serial_ok, token_client, focus_client) {
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
    fn a_drop_target_may_raise_its_window_for_a_moment() {
        let at = Instant::now();
        let dropped = Some(&(1, at));
        assert!(drop_target_trusted(
            dropped,
            Some(&1),
            at + Duration::from_secs(1)
        ));
        assert!(
            !drop_target_trusted(dropped, Some(&2), at),
            "another client"
        );
        assert!(!drop_target_trusted(dropped, None, at), "no surface");
        assert!(
            !drop_target_trusted(dropped, Some(&1), at + DROP_GRACE),
            "the moment has passed"
        );
        assert!(!drop_target_trusted::<i32>(None, Some(&1), at));
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
