//! A `dbus://` subscription makes cupsd raise `org.cups.cupsd.Notifier` signals on the system
//! bus, so the worker waits on events instead of a timer and a socket-activated cupsd can exit.

use std::time::{Duration, SystemTime};

use crate::cups::{Client, Error, Result};
use crate::ipp::{Request, SUBSCRIPTION, Value};

const CREATE_PRINTER_SUBSCRIPTIONS: u16 = 0x0016;
const RENEW_SUBSCRIPTION: u16 = 0x001A;
const GET_SUBSCRIPTIONS: u16 = 0x0019;

const RECIPIENT: &str = "dbus://";
/// Marks the subscription as this program's, for the next run to find: cupsd reports
/// `notify-events` back expanded, so the events cannot tell it apart.
const USER_DATA: &[u8] = b"reprise-printers";
/// Every printer event and job state or config change. No `job-progress`: a running job is
/// read on a timer anyway.
const EVENTS: [&str; 3] = ["printer-changed", "job-state-changed", "job-config-changed"];
/// Seconds. Long, since renewing wakes cupsd; a crash leaves the subscription this long at most.
const LEASE: i32 = 24 * 60 * 60;

const PATH: &str = "/org/cups/cupsd/Notifier";
const IFACE: &str = "org.cups.cupsd.Notifier";

/// A subscription and when cupsd last granted it. `granted` is `None` for one that never
/// expires (cupsd answers a lease of 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub id: i32,
    pub granted: Option<Duration>,
    pub since: SystemTime,
}

impl Lease {
    fn new(id: i32, granted: i32, since: SystemTime) -> Lease {
        Lease {
            id,
            granted: (granted > 0).then(|| Duration::from_secs(granted as u64)),
            since,
        }
    }

    /// Renew once half the lease is gone. The wall clock, as cupsd counts it: a suspend
    /// stops `Instant` but not the lease. A clock set back renews too.
    pub fn due(&self, now: SystemTime) -> bool {
        let Some(granted) = self.granted else {
            return false;
        };
        match now.duration_since(self.since) {
            Ok(gone) => gone >= granted / 2,
            Err(_) => true,
        }
    }
}

fn server_uri() -> Value {
    Value::uri("ipp://localhost/")
}

pub fn create_request() -> Request {
    Request::new(CREATE_PRINTER_SUBSCRIPTIONS)
        .op_attr("printer-uri", server_uri())
        .add(
            SUBSCRIPTION,
            "notify-recipient-uri",
            vec![Value::uri(RECIPIENT)],
        )
        .add(
            SUBSCRIPTION,
            "notify-events",
            EVENTS.iter().map(|e| Value::keyword(e)).collect(),
        )
        .add(
            SUBSCRIPTION,
            "notify-lease-duration",
            vec![Value::Int(LEASE)],
        )
        .add(
            SUBSCRIPTION,
            "notify-user-data",
            vec![Value::Octets(USER_DATA.to_vec())],
        )
}

pub fn renew_request(id: i32) -> Request {
    Request::new(RENEW_SUBSCRIPTION)
        .op_attr("printer-uri", server_uri())
        .op_attr("notify-subscription-id", Value::Int(id))
        .op_attr("notify-lease-duration", Value::Int(LEASE))
}

/// The account's own subscriptions on the server, not a printer's.
fn mine_request() -> Request {
    Request::new(GET_SUBSCRIPTIONS)
        .op_attr("printer-uri", server_uri())
        .op_attr("my-subscriptions", Value::Bool(true))
}

/// The id of a subscription of ours a previous run left behind, so a restart renews it
/// instead of adding another.
pub fn ours(groups: &[crate::ipp::Group]) -> Option<i32> {
    groups
        .iter()
        .filter(|g| g.tag == SUBSCRIPTION)
        .find(|g| {
            g.str("notify-recipient-uri") == Some(RECIPIENT)
                && g.get("notify-printer-uri").is_none()
                && g.get("notify-user-data") == Some(&[Value::Octets(USER_DATA.to_vec())])
        })
        .and_then(|g| g.int("notify-subscription-id"))
}

/// Renew the subscription a previous run left, else create one.
pub fn subscribe(c: &Client, now: SystemTime) -> Result<Lease> {
    let left = c
        .call("/", mine_request())
        .ok()
        .and_then(|r| ours(&r.groups));
    if let Some(id) = left
        && let Ok(l) = renew(c, id, now)
    {
        return Ok(l);
    }
    let resp = c.call("/", create_request())?;
    let g = resp
        .first(SUBSCRIPTION)
        .ok_or_else(|| Error::Refused("CUPS made no subscription.".into()))?;
    if let Some(status) = g.int("notify-status-code").filter(|s| *s != 0) {
        return Err(Error::Refused(format!(
            "CUPS refused the subscription (status 0x{status:04x})."
        )));
    }
    let id = g
        .int("notify-subscription-id")
        .ok_or_else(|| Error::Refused("CUPS made no subscription.".into()))?;
    Ok(Lease::new(
        id,
        g.int("notify-lease-duration").unwrap_or(LEASE),
        now,
    ))
}

pub fn renew(c: &Client, id: i32, now: SystemTime) -> Result<Lease> {
    let resp = c.call("/", renew_request(id))?;
    let granted = resp
        .first(SUBSCRIPTION)
        .and_then(|g| g.int("notify-lease-duration"))
        .unwrap_or(LEASE);
    Ok(Lease::new(id, granted, now))
}

/// Call `wake` for each printer or job signal on the system bus, from a thread of its own.
/// False when there is no system bus to listen on. Server signals are skipped: reading
/// cupsd on `ServerStopped` would start it again.
pub fn listen(wake: impl Fn() + Send + 'static) -> bool {
    let conn = match zbus::blocking::Connection::system() {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus for printer events: {e}");
            return false;
        }
    };
    let rule = match zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .path(PATH)
        .and_then(|b| b.interface(IFACE))
    {
        Ok(b) => b.build(),
        Err(e) => {
            log::warn!("no match rule for printer events: {e}");
            return false;
        }
    };
    let signals = match zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None) {
        Ok(s) => s,
        Err(e) => {
            log::info!("cannot listen for printer events: {e}");
            return false;
        }
    };
    std::thread::Builder::new()
        .name("cups-events".into())
        .spawn(move || {
            for m in signals.flatten() {
                let header = m.header();
                let member = header.member().map(|m| m.as_str()).unwrap_or_default();
                if wakes(member) {
                    wake();
                }
            }
            log::warn!("the system bus closed; printer events stop");
        })
        .is_ok()
}

fn wakes(member: &str) -> bool {
    member.starts_with("Printer") || member.starts_with("Job")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::{self, Attr, Group, OPERATION};

    fn attr(name: &str, values: Vec<Value>) -> Attr {
        Attr {
            name: name.into(),
            values,
        }
    }

    #[test]
    fn the_subscription_asks_for_dbus_events_on_the_whole_server() {
        let req = ipp::parse(&create_request().encode()).expect("parses");
        assert_eq!(req.status, CREATE_PRINTER_SUBSCRIPTIONS);
        let op = req.groups.iter().find(|g| g.tag == OPERATION).unwrap();
        assert_eq!(op.str("printer-uri"), Some("ipp://localhost/"));
        let sub = req.groups.iter().find(|g| g.tag == SUBSCRIPTION).unwrap();
        assert_eq!(sub.str("notify-recipient-uri"), Some("dbus://"));
        assert_eq!(
            sub.strs("notify-events"),
            ["printer-changed", "job-state-changed", "job-config-changed"]
        );
        assert_eq!(sub.int("notify-lease-duration"), Some(86400));
        assert_eq!(
            sub.get("notify-user-data"),
            Some(&[Value::Octets(b"reprise-printers".to_vec())][..])
        );

        let req = ipp::parse(&renew_request(42).encode()).expect("parses");
        assert_eq!(req.status, RENEW_SUBSCRIPTION);
        let op = req.groups.iter().find(|g| g.tag == OPERATION).unwrap();
        assert_eq!(op.int("notify-subscription-id"), Some(42));
        assert_eq!(op.int("notify-lease-duration"), Some(86400));
    }

    #[test]
    fn a_lease_renews_at_half_time_by_the_wall_clock() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let l = Lease::new(7, 86400, t0);
        assert!(!l.due(t0));
        assert!(!l.due(t0 + Duration::from_secs(43199)));
        assert!(l.due(t0 + Duration::from_secs(43200)));
        assert!(
            l.due(t0 - Duration::from_secs(1)),
            "a clock set back renews"
        );
        let forever = Lease::new(7, 0, t0);
        assert!(!forever.due(t0 + Duration::from_secs(10 * 86400)));
    }

    #[test]
    fn a_restart_finds_its_own_server_subscription() {
        let sub = |id: i32, recipient: &str, data: &[u8], printer: bool| {
            let mut attrs = vec![
                attr("notify-subscription-id", vec![Value::Int(id)]),
                attr("notify-recipient-uri", vec![Value::uri(recipient)]),
                attr("notify-events", vec![Value::keyword("job-created")]),
                attr("notify-user-data", vec![Value::Octets(data.to_vec())]),
            ];
            if printer {
                attrs.push(attr(
                    "notify-printer-uri",
                    vec![Value::uri("ipp://localhost/printers/Laser")],
                ));
            }
            Group {
                tag: SUBSCRIPTION,
                attrs,
            }
        };
        let groups = vec![
            sub(1, "rss://", USER_DATA, false),
            sub(2, "dbus://", b"", false),
            sub(3, "dbus://", USER_DATA, true),
            sub(4, "dbus://", USER_DATA, false),
        ];
        assert_eq!(ours(&groups), Some(4));
        assert_eq!(ours(&groups[..3]), None);
    }

    #[test]
    fn server_signals_do_not_wake_the_worker() {
        assert!(wakes("PrinterStateChanged"));
        assert!(wakes("JobState"));
        assert!(!wakes("ServerStopped"));
        assert!(!wakes("ServerStarted"));
    }
}
