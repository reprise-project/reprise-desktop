//! xdg-activation-v1. A trusted token focuses the window; any other only flags
//! the taskbar button. Trust is stamped at mint.

use std::time::Duration;

use smithay::{
    input::Seat,
    reexports::wayland_server::{Resource, backend::ClientId, protocol::wl_surface::WlSurface},
    wayland::{
        compositor::get_parent,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
    },
};

use crate::compositor::State;

const TOKEN_LIFETIME: Duration = Duration::from_secs(30);

struct Trusted;

/// A client's new token earns trust only with an input serial on our seat and a
/// surface of the client holding keyboard focus; a pointer enter alone yields a
/// valid serial.
fn client_token_trusted(
    serial_ok: bool,
    token_client: Option<ClientId>,
    focus_client: Option<ClientId>,
) -> bool {
    serial_ok && token_client.is_some() && token_client == focus_client
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        self.activation
            .retain_tokens(|_, d| d.timestamp.elapsed() < TOKEN_LIFETIME);
        let keyboard = self.seat.get_keyboard();
        let serial_ok = data.serial.as_ref().is_some_and(|(serial, seat)| {
            Seat::<State>::from_resource(seat).as_ref() == Some(&self.seat)
                && keyboard
                    .as_ref()
                    .and_then(|k| k.last_enter())
                    .is_some_and(|enter| serial.is_no_older_than(&enter))
        });
        let token_client = data
            .surface
            .as_ref()
            .and_then(|s| s.client())
            .map(|c| c.id());
        let focus_client = keyboard
            .and_then(|k| k.current_focus())
            .and_then(|s| s.client())
            .map(|c| c.id());
        if client_token_trusted(serial_ok, token_client, focus_client) {
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
        let trusted =
            data.timestamp.elapsed() < TOKEN_LIFETIME && data.user_data.get::<Trusted>().is_some();
        self.activation.remove_token(&token);
        let mut root = surface;
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let Some(w) = self.window_for_surface(&root) else {
            return;
        };
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !win.mapped {
            win.activate_on_map |= trusted;
            return;
        }
        let minimized = win.minimized;
        log::info!(
            "activation of {:?}: {}",
            win.title(),
            if trusted { "focused" } else { "attention" }
        );
        let focused = self.focused == Some(w);
        if trusted && !self.is_locked() {
            if minimized {
                self.set_minimized(w, false);
            } else {
                self.focus_window(w);
            }
        } else if !focused {
            if let Some(win) = self.win_mut(w) {
                win.attention = true;
            }
            self.redraw_taskbars();
            self.redraw_all();
        }
    }
}

impl State {
    pub fn new_activation_token(&mut self) -> String {
        self.activation
            .retain_tokens(|_, d| d.timestamp.elapsed() < TOKEN_LIFETIME);
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
        crate::spawn::spawn_in(cmd, app.as_deref(), &self.env, Some(&launch))
    }

    /// The Run dialog's line: one that names a folder opens it in Explorer, anything else runs
    /// as [`State::spawn_command_activated`]. The folder is checked on a file job, so one on a
    /// mount that does not answer never stalls the loop.
    pub fn run_line_activated(&mut self, cmd: &str) {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let Some(dir) = crate::spawn::command_folder(cmd, home.as_deref()) else {
            self.spawn_command_activated(cmd);
            return;
        };
        let cmd = cmd.to_string();
        self.spawn_fs_job(move || {
            let is_dir = dir.is_dir();
            Box::new(move |state: &mut State| {
                if is_dir {
                    state.spawn_argv_activated(&reprisefs::apps::open_folder_argv(&dir));
                } else {
                    state.spawn_command_activated(&cmd);
                }
            })
        });
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
            Some(app) => self.spawn_app_activated(&open.argv, app),
            None => self.spawn_argv_activated(&open.argv),
        }
    }

    /// `spawn_argv_activated` for a command built from the desktop entry `app`.
    pub fn spawn_app_activated<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
        &mut self,
        argv: &[S],
        app: &str,
    ) {
        let launch = self.new_launch();
        crate::spawn::spawn_argv_in(argv, Some(app), None, &self.env, Some(&launch))
    }

    /// `spawn_app_activated` for launcher `app`. One that runs `xdg-open <file>` (a Quick Launch
    /// document shortcut) on a regular file opens it in its default program and that program's
    /// scope, as the file's desktop icon does. The file is checked on a file job.
    pub fn spawn_launcher_activated(&mut self, argv: &[std::ffi::OsString], app: &str) {
        if crate::spawn::xdg_open_file(argv).is_none() {
            return self.spawn_app_activated(argv, app);
        }
        let (argv, app) = (argv.to_vec(), app.to_string());
        self.spawn_fs_job(move || {
            let doc = crate::spawn::xdg_open_document(&argv);
            Box::new(move |state: &mut State| {
                if let Some(open) = doc.and_then(|file| state.document_opening(&file)) {
                    return state.spawn_opening_activated(&open);
                }
                state.spawn_app_activated(&argv, &app)
            })
        });
    }

    fn document_opening(&self, file: &std::path::Path) -> Option<reprisefs::props::Opening> {
        let name = file.file_name()?.to_string_lossy();
        let mime = self.with_mime(|db| db.mime_of_name(&name))?;
        let default = reprisefs::props::default_app_entry(&mime, &self.app_index)?;
        Some(reprisefs::props::open_file(
            file,
            Some(default),
            &self.config.terminal,
        ))
    }
}

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
    fn token_without_a_serial_is_not_trusted() {
        let (focused, _) = two_clients();
        assert!(
            !client_token_trusted(false, Some(focused.clone()), Some(focused)),
            "expected untrusted"
        );
    }
}
