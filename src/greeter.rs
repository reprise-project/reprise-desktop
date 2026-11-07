//! `reprise --greeter`: the compositor with the login screen and nothing of a session, run by
//! reprise-login as `reprise-greeter`. It exits 0 after a login or SIGTERM and 1 on a lost link.

use std::error::Error;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use repriselogin::greeter::Greeting;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};

use crate::compositor::{Host, State};
use crate::config::Config;
use crate::greeterlink::GreeterLink;

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn main(socket: bool) -> i32 {
    let path = Path::new(repriselogin::GREETER_SOCKET);
    let mut link = match GreeterLink::connect(path) {
        Ok(link) => link,
        Err(e) => {
            log::error!("greeter: no login manager at {}: {e}", path.display());
            return 1;
        }
    };
    let greeting = match link.hello(HELLO_TIMEOUT) {
        Ok(g) => g,
        Err(e) => {
            log::error!("greeter: no greeting from the login manager: {e}");
            return 1;
        }
    };
    match run(link, greeting, socket) {
        Ok(lost) => i32::from(lost),
        Err(e) => {
            log::error!("greeter: {e}");
            1
        }
    }
}

/// Whether the link was lost.
fn run(link: GreeterLink, greeting: Greeting, socket: bool) -> Result<bool, Box<dyn Error>> {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let mut state = display(config_from(&greeting), &event_loop, socket)?;
    state.keyboard_setup();
    crate::backend::init(&mut state)?;
    crate::idle::setup(&mut state);
    hold_power_keys();
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
    match socket {
        true => log::info!(
            "greeter: running on WAYLAND_DISPLAY={}",
            state.socket().to_string_lossy()
        ),
        false => log::info!("greeter: running with no Wayland socket"),
    }
    event_loop.run(None, &mut state, State::turn_end)?;
    Ok(state.greeter.lost)
}

/// The login screen's state, with no config file: the greeting carries what it follows. Only
/// with `socket` does it listen, since any process of its user could connect.
fn display(
    config: Config,
    event_loop: &EventLoop<'static, State>,
    socket: bool,
) -> Result<State, Box<dyn Error>> {
    let (handle, signal) = (event_loop.handle(), event_loop.get_signal());
    match socket {
        true => State::new(config, PathBuf::new(), handle, signal, Host::Greeter),
        false => State::on_display(
            config,
            PathBuf::new(),
            handle,
            signal,
            String::new(),
            Host::Greeter,
        ),
    }
}

/// Takes logind's power, suspend and hibernate keys for the greeter's life: the power key opens
/// the screen's Turn Off, and no key sleeps the machine unasked.
fn hold_power_keys() {
    static HELD: OnceLock<OwnedFd> = OnceLock::new();
    let spawned = std::thread::Builder::new()
        .name("greeter-keys".into())
        .spawn(|| {
            let taken = zbus::block_on(async {
                let conn = zbus::Connection::system().await?;
                let reply = conn
                    .call_method(
                        Some("org.freedesktop.login1"),
                        "/org/freedesktop/login1",
                        Some("org.freedesktop.login1.Manager"),
                        "Inhibit",
                        &(
                            "handle-power-key:handle-suspend-key:handle-hibernate-key",
                            "reprise",
                            "the login screen asks before turning off",
                            "block",
                        ),
                    )
                    .await?;
                reply.body().deserialize::<zbus::OwnedFd>()
            });
            match taken {
                Ok(fd) => {
                    let _ = HELD.set(fd.into());
                }
                Err(e) => log::info!("greeter: logind keeps the power keys: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("greeter: no thread for the power keys: {e}");
    }
}

/// The defaults, in the greeting's interface and keyboard layout, with no idle lock: the login
/// screen is what a lock would cover. Glass's screen lies over its default wallpaper.
pub fn config_from(g: &Greeting) -> Config {
    let interface = interface(g.interface);
    let glass = reprisekit::glass::Settings::default();
    let wallpaper = (interface == reprisekit::interface::Interface::Glass)
        .then(|| reprisekit::interfaces::default_wallpaper(interface))
        .flatten()
        .map(|b| PathBuf::from(reprisekit::interfaces::resolve_wallpaper(b, &glass).uri()));
    let base = repriseconf::config::Config {
        interface,
        theme: reprisekit::interfaces::theme_for(interface),
        wallpaper: crate::config::WallpaperConfig {
            image: wallpaper,
            ..Default::default()
        },
        keyboard: crate::config::KeyboardConfig {
            layout: g.keyboard.layout.clone(),
            variant: g.keyboard.variant.clone(),
            options: g.keyboard.options.clone(),
            ..Default::default()
        },
        idle: crate::config::IdleConfig {
            lock_after: 0,
            lock_fade: 0,
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

pub fn wire_interface(i: reprisekit::interface::Interface) -> repriselogin::Interface {
    use reprisekit::interface::Interface as Kit;
    match i {
        Kit::Classic => repriselogin::Interface::Classic,
        Kit::Blue => repriselogin::Interface::Blue,
        Kit::Glass => repriselogin::Interface::Glass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;
    use repriselogin::greeter::Keyboard;

    fn greeting() -> Greeting {
        Greeting {
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
        }
    }

    #[test]
    fn the_login_screen_listens_on_no_socket_by_default() {
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().unwrap();
        let state = display(config_from(&greeting()), &event_loop, false).unwrap();
        assert!(state.socket().is_empty(), "{:?}", state.socket());
    }

    #[test]
    fn greeter_socket_is_a_greeter_flag() {
        use clap::Parser as _;
        let cli = crate::Cli::try_parse_from(["reprise", "--greeter"]).unwrap();
        assert!(cli.greeter && !cli.greeter_socket);
        let cli = crate::Cli::try_parse_from(["reprise", "--greeter", "--greeter-socket"]).unwrap();
        assert!(cli.greeter_socket);
        assert!(crate::Cli::try_parse_from(["reprise", "--greeter-socket"]).is_err());
    }

    #[test]
    fn the_greeting_sets_interface_and_keyboard() {
        let config = config_from(&greeting());
        assert_eq!(config.interface, reprisekit::interface::Interface::Glass);
        assert_eq!(
            config.theme,
            reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass)
        );
        assert_eq!(config.keyboard.layout, "de");
        assert_eq!(config.keyboard.variant, "nodeadkeys");
        assert_eq!(config.keyboard.options, "caps:escape");
    }

    #[test]
    fn glass_lies_over_its_default_wallpaper_and_the_others_over_none() {
        use reprisekit::interface::Interface as Kit;
        let glass = config_from(&greeting());
        let want = reprisekit::interfaces::default_wallpaper(Kit::Glass)
            .map(|b| reprisekit::interfaces::resolve_wallpaper(b, &Default::default()).uri());
        assert_eq!(
            glass.wallpaper.image.as_deref().and_then(|p| p.to_str()),
            want.as_deref()
        );
        for i in [
            repriselogin::Interface::Classic,
            repriselogin::Interface::Blue,
        ] {
            let g = Greeting {
                interface: i,
                ..greeting()
            };
            assert_eq!(config_from(&g).wallpaper.image, None, "{i:?}");
        }
    }

    #[test]
    fn the_greeter_never_locks_itself() {
        let idle = config_from(&greeting()).idle_for(false);
        assert_eq!((idle.lock_after, idle.lock_fade), (0, 0), "no idle lock");
        assert_eq!(idle.screen_off_after, 900, "the screen still powers off");

        let mut h = Harness::with_host(Default::default(), Host::Greeter);
        assert_eq!(h.state.config.idle.lock_after, 600);
        h.output("TEST-1", 1024, 768);
        let queued = crate::state::QUEUED.with(|n| n.get());
        crate::idle::idle_for(&mut h.state, Duration::from_secs(601));
        crate::lockview::lock(&mut h.state);
        crate::lockview::engage(&mut h.state);
        assert!(
            !h.state.console_locked(),
            "no lock cover, so no locker or fingerprint conversation"
        );
        assert_eq!(
            crate::state::QUEUED.with(|n| n.get()),
            queued,
            "no saved-state write"
        );
    }
}
