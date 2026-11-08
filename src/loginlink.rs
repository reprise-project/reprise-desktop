//! The session's link to reprise-login over `REPRISE_LOGIN_SOCKET`: Hello with the interface,
//! Switch User. While the daemon is away it calls every 2 s, until `MAX_REFUSALS` in a row.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use repriselogin::frame::{self, FrameReader, write_msg};
use repriselogin::session::{SessionReply, SessionRequest};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken};
use smithay::reexports::rustix::net::sockopt;

use crate::compositor::State;
use crate::console::Side;

/// The uid the daemon runs as.
const ROOT: u32 = 0;
const RETRY: Duration = Duration::from_secs(2);
/// Links in a row the daemon refuses, as not one of its sessions, before the link stops calling.
const MAX_REFUSALS: u32 = 5;

pub struct LoginLink {
    /// None for a link made from a stream, which never calls again.
    path: Option<PathBuf>,
    /// The uid the server must run as (`SO_PEERCRED`).
    server_uid: u32,
    stream: Option<UnixStream>,
    watch: Option<RegistrationToken>,
    retry: Option<RegistrationToken>,
    /// A refused server was logged at warn since the last link; later refusals log at debug.
    refusal_warned: bool,
    /// The daemon sent `Refused` on the current stream.
    refused: bool,
    /// Streams in a row the daemon refused.
    refusals: u32,
    /// The worker started the session in the background (`repriselogin::BACKGROUND_ENV`).
    background: bool,
}

impl LoginLink {
    fn at(path: PathBuf, server_uid: u32) -> LoginLink {
        LoginLink {
            path: Some(path),
            server_uid,
            stream: None,
            watch: None,
            retry: None,
            refusal_warned: false,
            refused: false,
            refusals: 0,
            background: false,
        }
    }

    /// Counts the stream that just closed; whether to call again.
    fn closed(&mut self) -> bool {
        if std::mem::take(&mut self.refused) {
            self.refusals += 1;
        } else {
            self.refusals = 0;
        }
        self.refusals < MAX_REFUSALS
    }

    /// A link over `stream`, with no uid check.
    #[cfg(test)]
    pub fn from_stream(stream: UnixStream) -> LoginLink {
        LoginLink {
            stream: Some(stream),
            path: None,
            ..LoginLink::at(PathBuf::new(), ROOT)
        }
    }
}

/// Link to the daemon when the session runs under reprise-login.
pub fn setup(state: &mut State) {
    if let Some(path) = std::env::var_os(repriselogin::SOCKET_ENV) {
        setup_at(state, PathBuf::from(path));
    }
    if let Some(link) = state.login_link.as_mut() {
        link.background = std::env::var_os(repriselogin::BACKGROUND_ENV).is_some();
    }
}

pub fn setup_at(state: &mut State, path: PathBuf) {
    install(state, LoginLink::at(path, ROOT));
}

pub(crate) fn install(state: &mut State, link: LoginLink) {
    let linked = link.stream.is_some();
    state.login_link = Some(link);
    if linked {
        on_linked(state);
    } else {
        call(state);
    }
}

pub fn connected(state: &State) -> bool {
    (state.login_link.as_ref()).is_some_and(|l| l.stream.is_some())
}

/// Whether `side`'s lock offers Switch User: only the computer's own, never a remote viewer's.
pub fn offered(state: &State, side: Side) -> bool {
    side == Side::Console && connected(state)
}

/// Whether the desktop offers Switch User (Log Off, Start's options): not to a lock-mode remote.
pub fn desk_offers(state: &State) -> bool {
    connected(state) && !crate::console::remote_side(state)
}

/// Whether a logind Unlock is refused: off the seat it comes from the daemon resuming the
/// session, and without a link no daemon is resuming it.
pub fn refuses_unlock(state: &State) -> bool {
    state.login_link.is_some() && !connected(state) && state.lock.seat_away
}

/// A session reprise-login started on a VT nobody shows comes up locked: Ctrl+Alt+Fn at the
/// computer must not show its desktop. `active` is the seat's state at start.
pub fn born(state: &mut State, active: bool) {
    let Some(link) = state.login_link.as_ref() else {
        return;
    };
    if active && !link.background {
        return;
    }
    if !active {
        crate::lockview::seat_away(state, true);
    }
    crate::lockview::lock(state);
}

/// Lock the console, then ask the daemon for the login screen. No-op without a link.
pub fn switch_user(state: &mut State) {
    if !connected(state) {
        return;
    }
    crate::lockview::lock(state);
    send(state, &SessionRequest::SwitchUser);
    log::info!("reprise-login: Switch User");
}

/// logind reports the session `closing`: under reprise-login, its worker is gone and nothing
/// will end the session's scope, so the compositor ends itself.
pub fn session_closing(state: &mut State) {
    if state.login_link.is_none() || crate::crash::is_ending() {
        return;
    }
    log::info!("reprise-login: the logind session is closing; ending the session");
    crate::sessionend::quit(state);
}

/// The daemon's socket at `path`, refused unless the server runs as `server_uid`.
fn open(path: &Path, server_uid: u32) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    let uid = sockopt::socket_peercred(&stream)?.uid.as_raw();
    if uid != server_uid {
        let e = format!("the server runs as uid {uid}, not {server_uid}");
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, e));
    }
    Ok(stream)
}

/// Call the daemon, or try again in `RETRY`.
fn call(state: &mut State) {
    let Some(link) = state.login_link.as_mut() else {
        return;
    };
    let Some(path) = link.path.clone() else {
        return;
    };
    match open(&path, link.server_uid) {
        Ok(stream) => {
            link.stream = Some(stream);
            on_linked(state);
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && !link.refusal_warned => {
            link.refusal_warned = true;
            log::warn!("reprise-login: {}: {e}", path.display());
            retry(state);
        }
        Err(e) => {
            log::debug!("reprise-login: {}: {e}", path.display());
            retry(state);
        }
    }
}

fn retry(state: &mut State) {
    let Some(link) = state.login_link.as_mut() else {
        return;
    };
    if link.retry.is_some() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(RETRY), |_, _, state| {
            if let Some(link) = state.login_link.as_mut() {
                link.retry = None;
            }
            call(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => link.retry = Some(t),
        Err(e) => log::warn!("reprise-login: cannot time the next call: {}", e.error),
    }
}

/// Watch the new stream for the daemon going, say hello, and offer Switch User.
fn on_linked(state: &mut State) {
    let Some(link) = state.login_link.as_mut() else {
        return;
    };
    link.refusal_warned = false;
    let Some(stream) = link.stream.as_ref() else {
        return;
    };
    let watched = stream
        .set_nonblocking(true)
        .and_then(|()| stream.try_clone());
    let watched = match watched {
        Ok(s) => s,
        Err(e) => {
            log::warn!("reprise-login: cannot watch the link: {e}");
            lost(state);
            return;
        }
    };
    let mut frames = FrameReader::new();
    let token = state.handle.insert_source(
        Generic::new(watched, Interest::READ, Mode::Level),
        move |_, stream, state| {
            let mut buf = [0; 256];
            match (&**stream).read(&mut buf) {
                Ok(n) if n > 0 => {
                    heard(state, &mut frames, &buf[..n]);
                    return Ok(PostAction::Continue);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(PostAction::Continue);
                }
                _ => {}
            }
            if let Some(link) = state.login_link.as_mut() {
                link.watch = None;
            }
            log::info!("reprise-login: the link closed");
            lost(state);
            Ok(PostAction::Remove)
        },
    );
    match token {
        Ok(t) => {
            if let Some(link) = state.login_link.as_mut() {
                link.watch = Some(t);
            }
        }
        Err(e) => {
            log::warn!("reprise-login: cannot watch the link: {}", e.error);
            lost(state);
            return;
        }
    }
    let interface = crate::greeter::wire_interface(state.config.interface);
    if send(state, &SessionRequest::Hello { interface }) {
        log::info!("reprise-login: linked");
        // An earlier daemon may have unlocked the session for a resume and died. Like `born`,
        // this locks the console only: a connected client keeps its desktop.
        if state.lock.seat_away {
            crate::lockview::lock(state);
        }
        crate::lockview::redraw(state);
    }
}

/// `[appearance] interface` changed: the daemon's login screen follows the session's.
pub fn interface_changed(state: &mut State) {
    if connected(state) {
        let interface = crate::greeter::wire_interface(state.config.interface);
        send(state, &SessionRequest::Hello { interface });
    }
}

/// Notes a `Refused` among `bytes`; anything else the daemon sends is skipped.
fn heard(state: &mut State, frames: &mut FrameReader, bytes: &[u8]) {
    if frames.push(bytes).is_err() {
        return;
    }
    while let Some(body) = frames.next_frame() {
        if let Ok(SessionReply::Refused) = frame::decode(&body)
            && let Some(link) = state.login_link.as_mut()
        {
            link.refused = true;
        }
    }
}

/// Send `msg`; a failure drops the link. Whether it went.
fn send(state: &mut State, msg: &SessionRequest) -> bool {
    let Some(stream) = state.login_link.as_mut().and_then(|l| l.stream.as_mut()) else {
        return false;
    };
    match write_msg(stream, msg) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("reprise-login: the link failed: {e}");
            drain(state);
            lost(state);
            false
        }
    }
}

/// Reads what the daemon sent before the stream broke, so a `Refused` the watch has not read
/// yet still counts.
fn drain(state: &mut State) {
    let Some(stream) = state.login_link.as_mut().and_then(|l| l.stream.as_mut()) else {
        return;
    };
    let mut bytes = Vec::new();
    if stream.set_nonblocking(true).is_ok() {
        let _ = stream.read_to_end(&mut bytes);
    }
    heard(state, &mut FrameReader::new(), &bytes);
}

/// The daemon went or dropped the session: forget the stream, take Switch User away, lock the
/// console off the seat, ask logind whether the session is closing and call again.
fn lost(state: &mut State) {
    let Some(link) = state.login_link.as_mut() else {
        return;
    };
    link.stream = None;
    let again = link.closed();
    if let Some(t) = link.watch.take() {
        state.handle.remove(t);
    }
    // A daemon that died between unlocking this session for a resume and switching to it left
    // it unlocked on a VT anyone can reach. Like `born`, a connected client keeps its desktop.
    if state.lock.seat_away {
        crate::lockview::lock(state);
    }
    crate::lockview::redraw(state);
    crate::power::check_closing(state);
    if again {
        retry(state);
    } else {
        log::warn!(
            "reprise-login: refused {MAX_REFUSALS} times in a row as not its session; no more calls"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    use repriselogin::Interface;
    use repriselogin::frame::{FrameError, read_msg};
    use repriselogin::session::SessionRequest;

    use super::*;
    use crate::config::{Action, Config};
    use crate::harness::Harness;

    fn own_uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn socket_path(name: &str) -> PathBuf {
        let dir = crate::harness::scratch().join(format!("loginlink-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("session.sock")
    }

    fn heard(theirs: &mut UnixStream) -> SessionRequest {
        theirs
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_msg(theirs).expect("a request")
    }

    fn hello(interface: Interface) -> SessionRequest {
        SessionRequest::Hello { interface }
    }

    /// A harness with one output and a link over a socket pair, its Hello read.
    fn linked() -> (Harness, UnixStream) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        install(&mut h.state, LoginLink::from_stream(ours));
        assert_eq!(heard(&mut theirs), hello(Interface::Classic));
        (h, theirs)
    }

    fn log_off_offers(h: &mut Harness) -> bool {
        offers_on(h, |s| s.run_action(&Action::LogOff, 0))
    }

    /// Whether the Log Off panel `open` opens offers Switch User.
    fn offers_on(h: &mut Harness, open: fn(&mut State)) -> bool {
        open(&mut h.state);
        let d = &h
            .state
            .dialogs
            .views
            .last()
            .expect("Log Off is open")
            .dialog;
        let offered = (d.buttons.iter())
            .any(|b| b.enabled && b.choice == Some(reprisekit::powerpanel::Choice::SwitchUser));
        h.state.apply_dialog(crate::dialog::Outcome::Close);
        offered
    }

    #[test]
    fn the_link_refuses_a_server_that_is_not_root() {
        if own_uid() == ROOT {
            return;
        }
        let path = socket_path("not-root");
        let listener = UnixListener::bind(&path).unwrap();
        let mut h = Harness::new();
        setup_at(&mut h.state, path);
        assert!(
            h.state.login_link.is_some(),
            "the session runs under reprise-login"
        );
        assert!(!connected(&h.state));
        let (mut theirs, _) = listener.accept().unwrap();
        theirs
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(
            read_msg::<SessionRequest>(&mut theirs),
            Err(FrameError::Closed),
            "the link hangs up without a Hello"
        );
    }

    #[test]
    fn the_link_says_hello_with_the_interface() {
        let config = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        let mut h = Harness::with_config(config);
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        install(&mut h.state, LoginLink::from_stream(ours));
        assert!(connected(&h.state));
        assert_eq!(heard(&mut theirs), hello(Interface::Glass));
    }

    #[test]
    fn switch_user_locks_then_asks_the_daemon() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        switch_user(&mut h.state);
        assert!(!h.state.console_locked(), "no link, no Switch User");

        let (mut h, mut theirs) = linked();
        switch_user(&mut h.state);
        assert!(h.state.console_locked());
        assert_eq!(heard(&mut theirs), SessionRequest::SwitchUser);
    }

    #[test]
    fn log_off_offers_switch_user_only_under_reprise_login() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        assert!(
            !log_off_offers(&mut h),
            "Switch User is greyed without a link"
        );

        let (mut h, mut theirs) = linked();
        assert!(log_off_offers(&mut h));
        h.state.run_action(&Action::LogOff, 0);
        let outcome = (h.state.dialogs.views.last_mut().unwrap().dialog).press(0);
        assert_eq!(outcome, crate::dialog::Outcome::SwitchUser);
        h.state.apply_dialog(outcome);
        assert!(h.state.dialogs.views.is_empty(), "the panel closes");
        assert!(h.state.console_locked());
        assert_eq!(heard(&mut theirs), SessionRequest::SwitchUser);
    }

    #[test]
    fn a_refusal_sent_before_the_hello_still_counts() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        frame::write_msg(&mut theirs, &SessionReply::Refused).unwrap();
        drop(theirs);
        install(&mut h.state, LoginLink::from_stream(ours));
        assert!(!connected(&h.state), "the Hello found the stream closed");
        assert_eq!(h.state.login_link.as_ref().unwrap().refusals, 1);
    }

    #[test]
    fn a_reloaded_interface_says_hello_again() {
        let (mut h, mut theirs) = linked();
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        crate::configwatch::reload(&mut h.state, glass);
        assert_eq!(heard(&mut theirs), hello(Interface::Glass));
    }

    #[test]
    fn a_remote_viewer_is_never_offered_switch_user() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        install(&mut h.state, LoginLink::from_stream(ours));
        assert_eq!(heard(&mut theirs), hello(Interface::Classic));
        assert!(
            !log_off_offers(&mut h),
            "the remote's Log Off offers Disconnect instead"
        );
        crate::lockview::lock_remote(&mut h.state);
        let switch_on = |side| {
            let l = h.state.lock.on(side).expect("locked");
            l.outputs
                .iter()
                .map(|lo| lo.buttons.switch_user.is_some())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            switch_on(Side::Console),
            [true],
            "the computer's own lock offers it"
        );
        assert_eq!(switch_on(Side::Remote), [false]);
    }

    #[test]
    fn the_unlock_dialogs_switch_user_asks_the_daemon() {
        let (mut h, mut theirs) = linked();
        crate::lockview::lock(&mut h.state);
        let l = h.state.lock.console.as_ref().unwrap();
        let r = l.outputs[0].buttons.switch_user.expect("Switch User shows");
        let at = smithay::utils::Point::from((f64::from(r.x + 2), f64::from(r.y + 2)));
        crate::lockview::button(&mut h.state, Side::Console, crate::input::BTN_LEFT, at);
        assert!(h.state.console_locked(), "the lock stays up");
        assert_eq!(heard(&mut theirs), SessionRequest::SwitchUser);
    }

    #[test]
    fn a_session_born_in_the_background_under_reprise_login_starts_locked() {
        let (mut h, _theirs) = linked();
        born(&mut h.state, false);
        assert!(h.state.console_locked());
        assert!(
            h.state.lock.seat_away,
            "the lock starts no reader conversation"
        );

        let (mut h, _theirs) = linked();
        born(&mut h.state, true);
        assert!(
            !h.state.console_locked(),
            "a session born on the monitor opens"
        );
    }

    #[test]
    fn a_background_start_locks_even_when_its_vt_is_on_the_monitor_at_start() {
        let (mut h, _theirs) = linked();
        h.state.login_link.as_mut().unwrap().background = true;
        born(&mut h.state, true);
        assert!(
            h.state.console_locked(),
            "a VT switched to before the compositor started shows the lock"
        );
        assert!(
            !h.state.lock.seat_away,
            "on the monitor, the lock starts its reader conversation"
        );
    }

    #[test]
    fn a_session_born_locked_tells_logind_once_power_is_set_up() {
        let (mut h, _theirs) = linked();
        born(&mut h.state, false);
        let calls = crate::power::capture_calls(&mut h.state);
        assert_eq!(
            calls.try_recv(),
            Ok(crate::power::Call::Hint(crate::power::Hint::Locked(true)))
        );
        assert_eq!(
            calls.try_recv(),
            Ok(crate::power::Call::Hint(crate::power::Hint::Idle(false)))
        );
        crate::lockview::lock(&mut h.state);
        assert!(calls.try_recv().is_err(), "still locked: nothing new");
    }

    #[test]
    fn a_session_born_in_the_background_after_a_locked_log_off_is_saved_locked() {
        let (mut h, _theirs) = linked();
        h.state.session.was_locked = true;
        crate::crash::on_start(&mut h.state, false, false, false);
        assert!(h.state.console_locked());
        assert_eq!(crate::state::SCREEN_LOCKED.get(), Some(true));
    }

    #[test]
    fn without_reprise_login_an_inactive_start_locks_nothing() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        born(&mut h.state, false);
        assert!(!h.state.console_locked());
    }

    #[test]
    fn a_seat_pause_under_reprise_login_locks() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::backend::pause_session(&mut h.state);
        assert!(!h.state.console_locked(), "a plain session stays open");

        let (mut h, theirs) = linked();
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        crate::backend::pause_session(&mut h.state);
        assert!(
            h.state.console_locked(),
            "under reprise-login the session locks"
        );
    }

    #[test]
    fn the_lock_leaves_the_reader_alone_while_the_session_is_off_the_seat() {
        let (mut h, _theirs) = linked();
        h.state.lock.helpers.finger = |_| {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg("echo 'I ready'; exec sleep 60");
            Some(c)
        };
        let finger =
            |h: &Harness| (h.state.lock.console.as_ref()).is_some_and(|l| l.finger.is_some());
        crate::backend::pause_session(&mut h.state);
        assert!(h.state.console_locked());
        assert!(!finger(&h), "a paused session starts no conversation");
        crate::backend::activate_session(&mut h.state);
        assert!(finger(&h), "the session back on the seat starts one");
        crate::backend::pause_session(&mut h.state);
        assert!(!finger(&h), "a pause ends the running one");
        crate::backend::activate_session(&mut h.state);
        assert!(finger(&h));
        crate::lockview::unlock(&mut h.state);
    }

    #[test]
    fn the_blue_log_off_offers_switch_user_only_under_reprise_login() {
        let offers = |h: &mut Harness| offers_on(h, State::blue_log_off);
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        assert!(!offers(&mut h), "Switch User is greyed without a link");
        let (mut h, _theirs) = linked();
        assert!(offers(&mut h));
    }

    #[test]
    fn losing_the_link_asks_logind_whether_the_session_is_closing() {
        let (mut h, theirs) = linked();
        let calls = crate::power::capture_calls(&mut h.state);
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state));
        assert_eq!(
            crate::power::calls_but_hints(&calls),
            [crate::power::Call::State]
        );
    }

    #[test]
    fn a_closing_logind_session_ends_the_compositor_only_under_reprise_login() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        session_closing(&mut h.state);
        assert!(!crate::crash::is_ending(), "a plain session stays");

        let (mut h, _theirs) = linked();
        session_closing(&mut h.state);
        assert!(crate::crash::is_ending(), "the session ends");
    }

    #[test]
    fn a_refused_link_counts_toward_the_refusals_that_stop_its_calls() {
        let (mut h, mut theirs) = linked();
        write_msg(&mut theirs, &SessionReply::Refused).unwrap();
        drop(theirs);
        h.dispatch();
        h.dispatch();
        assert!(!connected(&h.state));
        assert_eq!(h.state.login_link.as_ref().unwrap().refusals, 1);
    }

    #[test]
    fn the_link_stops_calling_after_refusals_in_a_row_and_a_plain_close_resets_them() {
        let mut link = LoginLink::at(PathBuf::new(), ROOT);
        let refuse = |link: &mut LoginLink| {
            link.refused = true;
            link.closed()
        };
        for _ in 1..MAX_REFUSALS {
            assert!(refuse(&mut link));
        }
        assert!(link.closed(), "a close without a refusal");
        assert_eq!(link.refusals, 0);
        for _ in 1..MAX_REFUSALS {
            assert!(refuse(&mut link));
        }
        assert!(!refuse(&mut link), "no more calls");
    }

    #[test]
    fn relinking_away_from_the_seat_locks_the_console_side_only() {
        let path = socket_path("relink-away");
        let listener = UnixListener::bind(&path).unwrap();
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        install(&mut h.state, LoginLink::at(path.clone(), own_uid()));
        let (mut first, _) = listener.accept().unwrap();
        assert_eq!(heard(&mut first), hello(Interface::Classic));
        crate::backend::pause_session(&mut h.state);
        drop((first, listener));
        std::fs::remove_file(&path).unwrap();
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        crate::lockview::unlock(&mut h.state);
        assert!(!h.state.console_locked(), "unlocked while unlinked");
        let _listener = UnixListener::bind(&path).unwrap();
        h.run_for(2500);
        assert!(connected(&h.state));
        assert!(h.state.console_locked(), "relinked away from the seat");

        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        crate::lockview::seat_away(&mut h.state, true);
        let (ours, _theirs) = UnixStream::pair().unwrap();
        install(&mut h.state, LoginLink::from_stream(ours));
        assert!(h.state.console_locked());
        assert!(
            !h.state.desktop_locked(),
            "the connected client keeps its desktop"
        );
    }

    #[test]
    fn a_link_lost_off_the_seat_locks_the_console() {
        let (mut h, theirs) = linked();
        crate::lockview::seat_away(&mut h.state, true);
        assert!(!h.state.console_locked(), "unlocked for a resume");
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        assert!(h.state.console_locked());
    }

    #[test]
    fn a_link_lost_in_lock_mode_leaves_the_remote_side_unlocked() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        let (ours, theirs) = UnixStream::pair().unwrap();
        install(&mut h.state, LoginLink::from_stream(ours));
        crate::lockview::seat_away(&mut h.state, true);
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        assert!(h.state.console_locked());
        assert!(
            !h.state.desktop_locked(),
            "the connected client keeps its desktop"
        );
    }

    #[test]
    fn a_link_lost_on_the_seat_locks_nothing() {
        let (mut h, theirs) = linked();
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        assert!(!h.state.console_locked());
    }

    #[test]
    fn a_logind_unlock_off_the_seat_waits_for_the_link() {
        let (mut h, theirs) = linked();
        crate::backend::pause_session(&mut h.state);
        drop(theirs);
        h.dispatch();
        assert!(!connected(&h.state), "the daemon went");
        crate::power::report(&mut h.state, crate::power::Report::Unlock);
        assert!(
            h.state.console_locked(),
            "no daemon is resuming the session"
        );

        let (mut h, _theirs) = linked();
        crate::backend::pause_session(&mut h.state);
        crate::power::report(&mut h.state, crate::power::Report::Unlock);
        assert!(!h.state.console_locked(), "the daemon resumes the session");
    }

    #[test]
    fn the_link_reconnects_after_the_daemon_goes() {
        let path = socket_path("reconnect");
        let listener = UnixListener::bind(&path).unwrap();
        let mut h = Harness::new();
        install(&mut h.state, LoginLink::at(path.clone(), own_uid()));
        assert!(connected(&h.state));
        let (mut first, _) = listener.accept().unwrap();
        assert_eq!(heard(&mut first), hello(Interface::Classic));

        drop((first, listener));
        std::fs::remove_file(&path).unwrap();
        h.dispatch();
        assert!(!connected(&h.state), "the link sees the daemon go");

        let listener = UnixListener::bind(&path).unwrap();
        h.run_for(1000);
        assert!(!connected(&h.state), "no call before 2 s");
        h.run_for(1500);
        assert!(connected(&h.state));
        let (mut second, _) = listener.accept().unwrap();
        assert_eq!(heard(&mut second), hello(Interface::Classic));
        let mut rest = [0; 1];
        second.set_nonblocking(true).unwrap();
        assert!(second.read(&mut rest).is_err(), "one Hello per call");
    }
}
