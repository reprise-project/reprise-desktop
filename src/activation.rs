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

    pub fn spawn_activated(&mut self, cmd: &str) -> bool {
        let token = self.new_activation_token();
        crate::spawn::spawn(cmd, &self.env, Some(&token))
    }

    pub fn spawn_argv_activated<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
        &mut self,
        argv: &[S],
    ) {
        let token = self.new_activation_token();
        crate::spawn::spawn_argv(argv, &self.env, Some(&token))
    }

    /// `spawn_argv_activated` for a command built from the desktop entry `app`.
    pub fn spawn_app_activated<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
        &mut self,
        argv: &[S],
        app: &str,
    ) {
        let token = self.new_activation_token();
        crate::spawn::spawn_argv_in(argv, Some(app), None, &self.env, Some(&token))
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
