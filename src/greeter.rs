//! `reprise --greeter`: the compositor with the login screen and nothing of a session, run by
//! reprise-login as `reprise-greeter`. It exits 0 after a login or SIGTERM and 1 on a lost link.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use repriselogin::greeter::Greeting;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};

use crate::compositor::{Host, State};
use crate::config::Config;
use crate::greeterlink::GreeterLink;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn main() -> i32 {
    let path = Path::new(repriselogin::GREETER_SOCKET);
    let Ok(mut link) = GreeterLink::connect(path) else {
        log::error!("greeter: no login manager at {}", path.display());
        return 1;
    };
    let greeting = match link.hello(HELLO_TIMEOUT) {
        Ok(g) => g,
        Err(e) => {
            log::error!("greeter: no greeting from the login manager: {e}");
            return 1;
        }
    };
    match run(link, greeting) {
        Ok(lost) => i32::from(lost),
        Err(e) => {
            log::error!("greeter: {e}");
            1
        }
    }
}

/// Whether the link was lost.
fn run(link: GreeterLink, greeting: Greeting) -> Result<bool, Box<dyn Error>> {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    // No config file: the greeting carries what the login screen follows.
    let mut state = State::new(
        config_from(&greeting),
        PathBuf::new(),
        event_loop.handle(),
        event_loop.get_signal(),
        Host::Greeter,
    )?;
    state.keyboard_setup();
    crate::backend::init(&mut state)?;
    crate::idle::setup(&mut state);
    crate::greeterview::setup(&mut state, greeting);
    state.greeter_link = Some(link.tx()?);
    link.register(&event_loop.handle())?;
    event_loop.handle().insert_source(
        Generic::new(crate::spawn::sigchld_fd()?, Interest::READ, Mode::Level),
        |_, fd, _| {
            crate::spawn::drain_signalfd(fd);
            crate::spawn::reap_children();
            Ok(PostAction::Continue)
        },
    )?;
    event_loop.handle().insert_source(
        Generic::new(crate::spawn::term_fd()?, Interest::READ, Mode::Level),
        |_, fd, state| {
            if let Some(sig) = crate::spawn::read_signal(fd) {
                log::info!("greeter: signal {sig}: shutting down");
                state.signal.stop();
            }
            Ok(PostAction::Continue)
        },
    )?;
    log::info!(
        "greeter: running on WAYLAND_DISPLAY={}",
        state.socket().to_string_lossy()
    );
    event_loop.run(None, &mut state, State::turn_end)?;
    Ok(state.greeter.lost)
}

/// The defaults, in the greeting's interface and keyboard layout.
pub fn config_from(g: &Greeting) -> Config {
    let interface = interface(g.interface);
    let base = repriseconf::config::Config {
        interface,
        theme: reprisekit::interfaces::theme_for(interface),
        keyboard: crate::config::KeyboardConfig {
            layout: g.keyboard.layout.clone(),
            variant: g.keyboard.variant.clone(),
            options: g.keyboard.options.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    Config {
        base,
        ..Config::default()
    }
}

pub fn interface(i: repriselogin::Interface) -> reprisekit::interface::Interface {
    use reprisekit::interface::Interface as Kit;
    match i {
        repriselogin::Interface::Classic => Kit::Classic,
        repriselogin::Interface::Blue => Kit::Blue,
        repriselogin::Interface::Glass => Kit::Glass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriselogin::greeter::Keyboard;

    #[test]
    fn the_greeting_sets_interface_and_keyboard() {
        let g = Greeting {
            interface: repriselogin::Interface::Glass,
            show_users: true,
            users: Vec::new(),
            last_user: None,
            keyboard: Keyboard {
                layout: "de".into(),
                variant: "nodeadkeys".into(),
                options: "caps:escape".into(),
            },
            notice: None,
        };
        let config = config_from(&g);
        assert_eq!(config.interface, reprisekit::interface::Interface::Glass);
        assert_eq!(
            config.theme,
            reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass)
        );
        assert_eq!(config.keyboard.layout, "de");
        assert_eq!(config.keyboard.variant, "nodeadkeys");
        assert_eq!(config.keyboard.options, "caps:escape");
    }
}
