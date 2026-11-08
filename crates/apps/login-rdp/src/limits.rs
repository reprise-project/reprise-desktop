//! What one connection may cost before it authenticates, and the addresses told to wait.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv6Addr};
use std::rc::Rc;
use std::time::Duration;

use repriselogin::throttle::{Attempt, PAUSE, Pause, Throttle};
use tokio::time::Instant;

/// Connections that have not authenticated yet, from all addresses.
pub const MAX_PREAUTH: usize = 32;
/// Connections one address (an IPv6 /64) may have logging in; xfreerdp3 opens two about
/// 5 ms apart.
pub const PREAUTH_PER_IP: usize = 2;
/// Connections one IPv6 /48 may have logging in, whatever its /64s.
pub const PREAUTH_PER_SITE: usize = 4;
/// From the TCP accept to the TLS ClientHello, the X.224 request included.
pub const HELLO: Duration = Duration::from_secs(10);
/// From the TLS ClientHello to the credentials, HYBRID_EX's verified password included: long
/// enough for a client's user to answer its certificate or password prompt.
pub const HANDSHAKE: Duration = Duration::from_secs(60);
/// From the `Authenticate` send to reprise-login's verdict: its 3 s refusal hold plus PAM.
pub const VERDICT_WAIT: Duration = Duration::from_secs(12);
/// Bytes read from a client before it authenticates.
pub const MAX_PREAUTH_BYTES: usize = 256 * 1024;
/// Bytes of a user name, as the daemon takes them.
pub const MAX_USER: usize = 32;
/// Bytes of the recorded exchange; the hand-over frame carries it as hex, two bytes each.
pub const MAX_RECORDING: usize = 12 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Full,
    LoggingIn,
    /// The address's IPv6 /48 has `PREAUTH_PER_SITE` connections logging in.
    SiteLoggingIn,
    /// Refused for the time left.
    BackedOff(Duration),
    /// Every Network Level Authentication logon waits out a pause after many failed passwords.
    Paused,
}

impl Refusal {
    /// What the refusal counts as in the refusals report.
    pub fn kind(&self) -> &'static str {
        match self {
            Refusal::Full => "full",
            Refusal::LoggingIn | Refusal::SiteLoggingIn => "logging in",
            Refusal::BackedOff(_) => "backed off",
            Refusal::Paused => "NLA paused",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Full => write!(f, "{MAX_PREAUTH} connections are logging in"),
            Refusal::LoggingIn => {
                write!(f, "{PREAUTH_PER_IP} connections from here are logging in")
            }
            Refusal::SiteLoggingIn => {
                write!(
                    f,
                    "{PREAUTH_PER_SITE} connections from this /48 are logging in"
                )
            }
            Refusal::BackedOff(left) => {
                write!(
                    f,
                    "a failed logon from here waits {} s more",
                    left.as_secs_f32().ceil()
                )
            }
            Refusal::Paused => f.write_str(
                "Network Level Authentication is paused after repeated failed passwords",
            ),
        }
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    by_ip: HashMap<IpAddr, usize>,
    by_site: HashMap<IpAddr, usize>,
}

impl Counts {
    fn release(map: &mut HashMap<IpAddr, usize>, key: &IpAddr) {
        if let Some(n) = map.get_mut(key) {
            *n -= 1;
            if *n == 0 {
                map.remove(key);
            }
        }
    }
}

pub struct Limits {
    total: usize,
    per_ip: usize,
    counts: Rc<RefCell<Counts>>,
    backed_off: HashMap<IpAddr, Instant>,
}

/// One admitted connection's place; dropping it frees the place.
pub struct Permit {
    counts: Rc<RefCell<Counts>>,
    ip: IpAddr,
    site: Option<IpAddr>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let counts = &mut *self.counts.borrow_mut();
        counts.total -= 1;
        Counts::release(&mut counts.by_ip, &self.ip);
        if let Some(site) = &self.site {
            Counts::release(&mut counts.by_site, site);
        }
    }
}

impl Limits {
    pub fn new(total: usize, per_ip: usize) -> Limits {
        Limits {
            total,
            per_ip,
            counts: Rc::default(),
            backed_off: HashMap::new(),
        }
    }

    pub fn admit(&mut self, ip: IpAddr, now: Instant) -> Result<Permit, Refusal> {
        let ip = place(ip);
        self.backed_off.retain(|_, until| *until > now);
        if let Some(until) = self.backed_off.get(&ip) {
            return Err(Refusal::BackedOff(*until - now));
        }
        let mut counts = self.counts.borrow_mut();
        if counts.by_ip.get(&ip).copied().unwrap_or(0) >= self.per_ip {
            return Err(Refusal::LoggingIn);
        }
        let site = site(ip);
        if site.is_some_and(|s| counts.by_site.get(&s).copied().unwrap_or(0) >= PREAUTH_PER_SITE) {
            return Err(Refusal::SiteLoggingIn);
        }
        if counts.total >= self.total {
            return Err(Refusal::Full);
        }
        counts.total += 1;
        *counts.by_ip.entry(ip).or_default() += 1;
        if let Some(site) = site {
            *counts.by_site.entry(site).or_default() += 1;
        }
        Ok(Permit {
            counts: self.counts.clone(),
            ip,
            site,
        })
    }

    /// Refuses `ip` until `for_` from `now`, or longer when it is already backed off longer.
    pub fn back_off(&mut self, ip: IpAddr, for_: Duration, now: Instant) {
        let until = now + for_;
        let entry = self.backed_off.entry(place(ip)).or_insert(until);
        *entry = (*entry).max(until);
    }
}

/// The Remote Desktop passwords the front checks itself, which reprise-login's throttle never
/// sees: that throttle's back-off and pause, on tokio's clock and per place.
#[derive(Default)]
pub struct NlaThrottle(Throttle);

impl NlaThrottle {
    /// Refuses a password check from `ip` while `ip` backs off or every check is paused.
    pub fn check(&mut self, ip: IpAddr) -> Result<(), Refusal> {
        let left = self.0.delay(place(ip), now());
        if !left.is_zero() {
            return Err(Refusal::BackedOff(left));
        }
        if self.paused() {
            return Err(Refusal::Paused);
        }
        Ok(())
    }

    /// Counts a password check from `ip` as failed until it [`succeeded`](Self::succeeded).
    pub fn failed(&mut self, ip: IpAddr) -> Attempt {
        self.0.failed(place(ip), now())
    }

    pub fn succeeded(&mut self, ip: IpAddr, attempt: Attempt) {
        self.0.succeeded(place(ip), attempt, now());
    }

    /// Whether every check is paused. Logs the start of each pause once.
    pub fn paused(&mut self) -> bool {
        match self.0.paused(now()) {
            Pause::Off => false,
            Pause::Started => {
                log::info!(
                    "Network Level Authentication is paused for {} minutes after repeated failed passwords",
                    PAUSE.as_secs() / 60
                );
                true
            }
            Pause::Running => true,
        }
    }
}

fn now() -> std::time::Instant {
    Instant::now().into_std()
}

/// What the limits count `ip` as: itself for IPv4, its /64 for IPv6.
fn place(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
        v4 => v4,
    }
}

/// The /48 an IPv6 place belongs to; none for IPv4.
fn site(place: IpAddr) -> Option<IpAddr> {
    match place {
        IpAddr::V6(v6) => Some(IpAddr::V6(Ipv6Addr::from_bits(
            v6.to_bits() & (u128::MAX << 80),
        ))),
        IpAddr::V4(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_third_connection_from_one_address_while_two_log_in_is_refused() {
        let mut limits = Limits::new(MAX_PREAUTH, PREAUTH_PER_IP);
        let now = Instant::now();
        let first = limits
            .admit(ip("192.0.2.1"), now)
            .expect("the first is admitted");
        let _second = limits
            .admit(ip("192.0.2.1"), now)
            .expect("the second is admitted");
        assert_eq!(
            limits.admit(ip("::ffff:192.0.2.1"), now).err(),
            Some(Refusal::LoggingIn),
            "an IPv4-mapped address is the same address"
        );
        limits
            .admit(ip("192.0.2.2"), now)
            .expect("another address is admitted");
        drop(first);
        limits
            .admit(ip("192.0.2.1"), now)
            .expect("the third is admitted once the first ends");
    }

    #[test]
    fn connections_past_max_preauth_are_refused() {
        let mut limits = Limits::new(MAX_PREAUTH, PREAUTH_PER_IP);
        let now = Instant::now();
        let permits: Vec<Permit> = (0..MAX_PREAUTH)
            .map(|i| {
                limits
                    .admit(IpAddr::from([10, 0, 0, i as u8]), now)
                    .expect("admitted below the cap")
            })
            .collect();
        assert_eq!(
            limits.admit(ip("10.0.1.0"), now).err(),
            Some(Refusal::Full),
            "the 33rd is refused"
        );
        drop(permits);
        limits.admit(ip("10.0.1.0"), now).expect("admitted again");
    }

    #[test]
    fn a_backed_off_address_is_refused_until_its_back_off_ends() {
        let mut limits = Limits::new(MAX_PREAUTH, PREAUTH_PER_IP);
        let now = Instant::now();
        limits.back_off(ip("192.0.2.1"), Duration::from_secs(4), now);
        assert_eq!(
            limits
                .admit(ip("192.0.2.1"), now + Duration::from_secs(1))
                .err(),
            Some(Refusal::BackedOff(Duration::from_secs(3)))
        );
        limits
            .admit(ip("192.0.2.2"), now)
            .expect("another address is admitted");
        limits.back_off(ip("192.0.2.1"), Duration::from_secs(1), now);
        assert!(
            limits
                .admit(ip("192.0.2.1"), now + Duration::from_secs(2))
                .is_err(),
            "a shorter back-off keeps the longer one"
        );
        limits
            .admit(ip("192.0.2.1"), now + Duration::from_secs(4))
            .expect("admitted once the back-off ends");
    }

    #[test]
    fn addresses_in_one_ipv6_64_count_as_one() {
        let mut limits = Limits::new(MAX_PREAUTH, PREAUTH_PER_IP);
        let now = Instant::now();
        let _first = limits.admit(ip("2001:db8::1"), now).expect("admitted");
        let _second = limits.admit(ip("2001:db8::2"), now).expect("admitted");
        assert_eq!(
            limits.admit(ip("2001:db8::ffff:3"), now).err(),
            Some(Refusal::LoggingIn),
            "a third address in the /64 is the same place"
        );
        limits
            .admit(ip("2001:db8:0:1::1"), now)
            .expect("the next /64 is admitted");
        limits.back_off(ip("2001:db8:0:2::1"), Duration::from_secs(4), now);
        assert_eq!(
            limits.admit(ip("2001:db8:0:2::9"), now).err(),
            Some(Refusal::BackedOff(Duration::from_secs(4))),
            "a back-off holds the whole /64"
        );
    }

    #[test]
    fn one_ipv6_48_holds_at_most_preauth_per_site_places() {
        let mut limits = Limits::new(MAX_PREAUTH, PREAUTH_PER_IP);
        let now = Instant::now();
        let held: Vec<Permit> = (0..PREAUTH_PER_SITE)
            .map(|i| {
                let v6 = format!("2001:db8:0:{:x}::1", i / PREAUTH_PER_IP);
                limits
                    .admit(ip(&v6), now)
                    .expect("admitted below the /48's cap")
            })
            .collect();
        assert_eq!(
            limits.admit(ip("2001:db8:0:ff::1"), now).err(),
            Some(Refusal::SiteLoggingIn),
            "another /64 of the same /48 is refused"
        );
        limits
            .admit(ip("2001:db8:1::1"), now)
            .expect("the next /48 is admitted");
        limits
            .admit(ip("192.0.2.1"), now)
            .expect("IPv4 has no /48 cap");
        let mapped: Vec<Permit> = (0..=PREAUTH_PER_SITE)
            .map(|i| {
                limits
                    .admit(ip(&format!("::ffff:192.0.2.{}", 10 + i)), now)
                    .expect("IPv4-mapped addresses have no /48 cap")
            })
            .collect();
        drop((held, mapped));
        limits
            .admit(ip("2001:db8:0:ff::1"), now)
            .expect("admitted once the /48's connections end");
    }
}
