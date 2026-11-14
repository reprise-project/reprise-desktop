//! Thunderbolt and USB4 devices waiting for authorization: boltd is followed on a thread of its own
//! (never started from here), each device that connects unauthorized gets one balloon, and its
//! buttons authorize it once, enroll it, or leave it.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;

use reprisebus::bolt::{self, Change, Device, Found};

pub enum Msg {
    /// What boltd knows now; None while it is not on the bus.
    Found(Option<Found>),
    /// A button's change finished: the device's uid and label, and what to tell when it failed.
    Done {
        uid: String,
        label: String,
        result: Result<(), Option<String>>,
    },
}

type Poster = repriseapp::Poster<Msg>;

pub fn watch(waker: repriseapp::Waker) -> (Receiver<Msg>, Poster) {
    let (poster, rx) = Poster::channel(waker);
    let p = poster.clone();
    let started = std::thread::Builder::new()
        .name("reprise-drives-bolt".into())
        .spawn(move || match reprisebus::system() {
            Ok(conn) => reprisebus::wait(bolt::follow(&conn, |f| p.post(Msg::Found(f)))),
            Err(e) => log::info!("no system bus ({e}); no Thunderbolt devices"),
        });
    if let Err(e) = started {
        log::warn!("could not follow Thunderbolt devices: {e}");
    }
    (rx, poster)
}

/// Carries out `change` on `d` off the loop; polkit may ask for a password meanwhile.
pub fn carry_out(poster: &Poster, d: Device, change: Change) {
    let p = poster.clone();
    let started = std::thread::Builder::new()
        .name("reprise-drives-authorize".into())
        .spawn(move || {
            let result = bolt::carry_out(&d, change);
            p.post(Msg::Done {
                uid: d.uid.clone(),
                label: d.label(),
                result,
            });
        });
    if let Err(e) = started {
        log::warn!("could not authorize a Thunderbolt device: {e}");
    }
}

/// The waiting devices asked about, by uid, until each stops waiting: a device unplugged and
/// plugged in again is asked about again, one left waiting is not.
#[derive(Default)]
pub struct Asked(HashSet<String>);

impl Asked {
    /// The devices in `now` to ask about.
    pub fn feed(&mut self, now: &[Device]) -> Vec<Device> {
        self.0
            .retain(|uid| now.iter().any(|d| d.uid == *uid && d.waiting()));
        now.iter()
            .filter(|d| d.waiting() && self.0.insert(d.uid.clone()))
            .cloned()
            .collect()
    }
}

pub const AUTHORIZE_ONCE: usize = 0;
pub const ALWAYS: usize = 1;
pub const BUTTONS: [&str; 3] = ["Authorize Once", "Always Authorize", "Ignore"];

/// What button `i` of a device's balloon asks for; Ignore asks nothing.
pub fn change_for(i: usize) -> Option<Change> {
    match i {
        AUTHORIZE_ONCE => Some(Change::Authorize),
        ALWAYS => Some(Change::Always),
        _ => None,
    }
}

/// The balloon asking about `d`.
pub fn balloon(d: &Device) -> (String, String) {
    (
        format!("Authorize {}?", d.label()),
        "This Thunderbolt device does nothing until you authorize it, and can then reach this \
         computer's memory. Authorize it only if you trust it."
            .into(),
    )
}

/// The balloon telling why authorizing the device `label` failed.
pub fn failed_balloon(label: &str, why: &str) -> (String, String) {
    (format!("Could not authorize {label}"), why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::bolt::Status;

    fn dev(uid: &str, status: Status) -> Device {
        Device {
            path: format!("/org/freedesktop/bolt/devices/{uid}"),
            uid: uid.into(),
            name: "Dock".into(),
            vendor: "Acme".into(),
            status,
            stored: false,
            auto: false,
        }
    }

    fn uids(v: Vec<Device>) -> Vec<String> {
        v.into_iter().map(|d| d.uid).collect()
    }

    #[test]
    fn a_waiting_device_is_asked_about_once_per_connection() {
        let mut asked = Asked::default();
        let dock = dev("dock", Status::Connected);
        let drive = dev("drive", Status::Authorized);
        assert_eq!(uids(asked.feed(&[dock.clone(), drive.clone()])), ["dock"]);
        assert!(
            asked.feed(&[dock.clone(), drive.clone()]).is_empty(),
            "left waiting after Ignore"
        );
        let away = dev("dock", Status::Disconnected);
        assert!(asked.feed(&[away]).is_empty());
        assert_eq!(
            uids(asked.feed(std::slice::from_ref(&dock))),
            ["dock"],
            "plugged in again"
        );
        assert!(asked.feed(&[dev("dock", Status::Authorized)]).is_empty());
        assert!(asked.feed(&[]).is_empty(), "boltd gone");
        assert_eq!(uids(asked.feed(&[dock])), ["dock"]);
    }

    #[test]
    fn a_device_boltd_authorizes_itself_is_not_asked_about() {
        let mut asked = Asked::default();
        let enrolled = Device {
            stored: true,
            auto: true,
            ..dev("dock", Status::Connected)
        };
        assert!(asked.feed(&[enrolled]).is_empty());
        let manual = Device {
            stored: true,
            ..dev("dock", Status::Connected)
        };
        assert_eq!(uids(asked.feed(&[manual])), ["dock"]);
    }

    #[test]
    fn the_buttons_authorize_once_enroll_or_do_nothing() {
        assert_eq!(BUTTONS.len(), 3);
        assert_eq!(change_for(0), Some(Change::Authorize));
        assert_eq!(change_for(1), Some(Change::Always));
        assert_eq!(change_for(2), None);
    }

    #[test]
    fn the_balloon_names_the_device() {
        let (summary, body) = balloon(&dev("dock", Status::Connected));
        assert_eq!(summary, "Authorize Acme Dock?");
        assert!(body.contains("only if you trust it"));
    }
}
