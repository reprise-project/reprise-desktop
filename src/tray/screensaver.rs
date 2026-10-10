//! The `org.freedesktop.ScreenSaver` inhibit service on the compositor's bus connection: while any cookie is held the
//! idle timer is paused. A cookie goes with `UnInhibit` from its peer, or when that peer leaves the bus.

use super::TrayEvent;
use std::os::fd::RawFd;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use zbus::message::Header;

pub const SCREENSAVER_NAME: &str = "org.freedesktop.ScreenSaver";
pub const SCREENSAVER_PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
/// An application name or reason is cut to this before it is kept or logged.
const TEXT_CHARS: usize = 63;

/// Cookies held at most, in total and by one peer: each costs a little
/// memory and a log line, and one peer needs only a few.
const MAX_HELD: usize = 64;
const MAX_HELD_PER_SENDER: usize = 8;

/// One cookie: the unique bus name that took it and the application name
/// it gave, for the log.
struct Held {
    cookie: u32,
    sender: String,
    app: String,
}

#[derive(Default)]
pub struct Inhibits {
    held: Vec<Held>,
    next: u32,
}

#[derive(Debug, PartialEq)]
pub enum Refusal {
    TotalFull,
    SenderFull,
}

impl Inhibits {
    /// Take a cookie for `sender`. A cookie is never 0 and never one that
    /// is still held.
    pub fn add(&mut self, sender: &str, app: &str) -> Result<u32, Refusal> {
        if self.held.len() >= MAX_HELD {
            return Err(Refusal::TotalFull);
        }
        if self.held.iter().filter(|h| h.sender == sender).count() >= MAX_HELD_PER_SENDER {
            return Err(Refusal::SenderFull);
        }
        let cookie = loop {
            self.next = self.next.wrapping_add(1);
            if self.next != 0 && !self.held.iter().any(|h| h.cookie == self.next) {
                break self.next;
            }
        };
        self.held.push(Held {
            cookie,
            sender: sender.to_string(),
            app: app.to_string(),
        });
        Ok(cookie)
    }

    /// Release `cookie` if `sender` holds it; the application name it was
    /// taken under, or None when `sender` holds no such cookie.
    pub fn remove(&mut self, sender: &str, cookie: u32) -> Option<String> {
        let i = self
            .held
            .iter()
            .position(|h| h.cookie == cookie && h.sender == sender)?;
        Some(self.held.remove(i).app)
    }

    /// Drop every cookie `sender` holds, because it left the bus. Returns
    /// how many went.
    pub fn forget(&mut self, sender: &str) -> usize {
        let before = self.held.len();
        self.held.retain(|h| h.sender != sender);
        before - self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

/// The cookies, shared by the interface (one instance per path) and the
/// tray loop, which forgets a peer that leaves the bus.
#[derive(Clone)]
pub struct Shared {
    inhibits: Arc<Mutex<Inhibits>>,
    events: Sender<TrayEvent>,
    eventfd: RawFd,
}

impl Shared {
    pub fn new(events: Sender<TrayEvent>, eventfd: RawFd) -> Shared {
        Shared {
            inhibits: Arc::new(Mutex::new(Inhibits::default())),
            events,
            eventfd,
        }
    }

    /// Run `f` on the cookies, and tell the main loop when that took the
    /// set from empty to held or back. The event goes out under the lock,
    /// so two calls on different executor threads cannot report out of
    /// order.
    pub fn update<R>(&self, f: impl FnOnce(&mut Inhibits) -> R) -> R {
        let mut held = self.inhibits.lock().unwrap_or_else(|e| e.into_inner());
        let was = !held.is_empty();
        let r = f(&mut held);
        let now = !held.is_empty();
        if was != now {
            super::dbus::emit(
                &self.events,
                self.eventfd,
                TrayEvent::ScreenSaverInhibited(now),
            );
        }
        r
    }
}

/// Whether the bus's answer to GetNameOwner says the peer has left. Only
/// the bus's own NameHasNoOwner does: a timeout or a broken connection
/// says nothing about the peer, and dropping a live peer's cookie would
/// let the screen go dark under it.
fn has_left(answer: &zbus::fdo::Result<()>) -> bool {
    matches!(answer, Err(zbus::fdo::Error::NameHasNoOwner(_)))
}

pub struct ScreenSaver(pub Shared);

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    async fn inhibit(
        &self,
        application_name: String,
        reason_for_inhibit: String,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<u32> {
        let Some(sender) = hdr.sender().map(|s| s.to_string()) else {
            return Err(zbus::fdo::Error::AccessDenied("no sender".into()));
        };
        let app = super::clip(application_name, TEXT_CHARS);
        let cookie = self.0.update(|h| h.add(&sender, &app)).map_err(|r| {
            log::warn!("screensaver: refusing {app:?} from {sender}: {r:?}");
            zbus::fdo::Error::LimitsExceeded(format!("{r:?}"))
        })?;
        log::info!(
            "screensaver: {app:?} ({sender}) inhibits: {:?}, cookie {cookie}",
            super::clip(reason_for_inhibit, TEXT_CHARS)
        );
        // The peer may have left the bus after sending this call, and the
        // loop may already have handled its NameOwnerChanged. Then nothing
        // would ever drop the cookie.
        let answer = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(sender.as_str(),),
            )
            .await
            .map(drop)
            .map_err(zbus::fdo::Error::from);
        if has_left(&answer) {
            self.0.update(|h| h.forget(&sender));
            log::info!("screensaver: {sender} already left; cookie {cookie} dropped");
        } else if let Err(e) = answer {
            log::warn!("screensaver: could not check on {sender} ({e}); keeping cookie {cookie}");
        }
        Ok(cookie)
    }

    #[zbus(name = "UnInhibit")]
    async fn un_inhibit(&self, cookie: u32, #[zbus(header)] hdr: Header<'_>) {
        let Some(sender) = hdr.sender().map(|s| s.to_string()) else {
            return;
        };
        match self.0.update(|h| h.remove(&sender, cookie)) {
            Some(app) => log::info!("screensaver: {app:?} ({sender}) released cookie {cookie}"),
            None => log::info!("screensaver: {sender} holds no cookie {cookie}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_are_never_zero_and_never_repeat_while_held() {
        let mut h = Inhibits::default();
        let a = h.add(":1.5", "deskflow").unwrap();
        let b = h.add(":1.5", "deskflow").unwrap();
        let c = h.add(":1.6", "mpv").unwrap();
        assert!(a != 0 && b != 0 && c != 0, "cookie 0 is never handed out");
        assert!(
            a != b && b != c && a != c,
            "three held cookies are distinct"
        );
    }

    #[test]
    fn the_cookie_counter_wraps_past_zero_and_past_held_cookies() {
        let mut h = Inhibits::default();
        assert_eq!(h.add(":1.5", "deskflow"), Ok(1));
        h.next = u32::MAX;
        assert_eq!(
            h.add(":1.5", "deskflow"),
            Ok(2),
            "0 and the held 1 are skipped"
        );
    }

    #[test]
    fn a_cookie_is_released_only_by_its_holder() {
        let mut h = Inhibits::default();
        let a = h.add(":1.5", "deskflow").unwrap();
        assert_eq!(h.remove(":1.6", a), None, "another peer cannot release it");
        assert!(!h.is_empty());
        assert_eq!(h.remove(":1.5", a), Some("deskflow".to_string()));
        assert!(h.is_empty());
        assert_eq!(h.remove(":1.5", a), None, "a released cookie is gone");
    }

    #[test]
    fn a_peer_leaving_the_bus_drops_every_cookie_it_holds() {
        let mut h = Inhibits::default();
        h.add(":1.5", "deskflow").unwrap();
        h.add(":1.5", "deskflow").unwrap();
        h.add(":1.6", "mpv").unwrap();
        assert_eq!(h.forget(":1.5"), 2);
        assert!(!h.is_empty(), "the other peer's cookie stays");
        assert_eq!(h.forget(":1.6"), 1);
        assert!(h.is_empty());
    }

    #[test]
    fn inhibits_are_capped_per_sender_and_in_total() {
        let mut h = Inhibits::default();
        for _ in 0..MAX_HELD_PER_SENDER {
            h.add(":1.5", "deskflow").unwrap();
        }
        assert_eq!(h.add(":1.5", "deskflow"), Err(Refusal::SenderFull));
        let mut peer = 100;
        while h.held.len() < MAX_HELD {
            h.add(&format!(":1.{peer}"), "x").unwrap();
            peer += 1;
        }
        assert_eq!(h.add(":1.999", "x"), Err(Refusal::TotalFull));
    }

    #[test]
    fn only_the_bus_saying_no_owner_counts_as_the_peer_gone() {
        use zbus::fdo::Error;
        assert!(has_left(&Err(Error::NameHasNoOwner(":1.5".into()))));
        assert!(!has_left(&Ok(())), "an owner answer is a live peer");
        assert!(
            !has_left(&Err(Error::Timeout("5 s".into()))),
            "a timeout says nothing about the peer"
        );
        assert!(
            !has_left(&Err(Error::from(zbus::Error::InputOutput(
                std::io::Error::other("broken pipe").into()
            )))),
            "a broken connection says nothing about the peer"
        );
    }

    #[test]
    fn the_main_loop_hears_only_when_the_set_fills_or_empties() {
        let (tx, rx) = std::sync::mpsc::channel();
        let s = Shared::new(tx, -1);
        let a = s.update(|h| h.add(":1.5", "deskflow")).unwrap();
        s.update(|h| h.add(":1.5", "deskflow")).unwrap();
        s.update(|h| h.remove(":1.5", a));
        s.update(|h| h.forget(":1.5"));
        let got: Vec<bool> = rx
            .try_iter()
            .map(|e| match e {
                TrayEvent::ScreenSaverInhibited(on) => on,
                _ => panic!("only inhibit events are sent"),
            })
            .collect();
        assert_eq!(
            got,
            vec![true, false],
            "one event as the first cookie is taken, one as the last goes"
        );
    }
}
