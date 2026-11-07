//! Failed logins: a per-address back-off after repeated failures, and a pause of every login
//! after many. The caller passes the time, so the throttle reads no clock.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub const FREE_FAILURES: u32 = 3;
pub const MAX_WAIT: Duration = Duration::from_secs(60);
/// How long after its last failure an address is forgotten.
pub const FORGET: Duration = Duration::from_secs(1200);
/// `PAUSE_AFTER` failures from any addresses within `PAUSE_WINDOW` pause every login for `PAUSE`.
pub const PAUSE_AFTER: usize = 10;
pub const PAUSE_WINDOW: Duration = Duration::from_secs(600);
pub const PAUSE: Duration = Duration::from_secs(600);

/// After `FREE_FAILURES` an address waits a back-off doubling up to `MAX_WAIT`. A login counts as
/// failed from the start of its check, so a check cut short still counts.
#[derive(Default)]
pub struct Throttle {
    peers: HashMap<IpAddr, Failures>,
    /// Failures from every address, oldest first, dropped once older than `PAUSE_WINDOW + PAUSE`.
    recent: VecDeque<Instant>,
    /// The end of the pause last reported as started.
    announced: Option<Instant>,
}

/// One counted login, handed back to [`Throttle::succeeded`].
pub struct Attempt(Instant);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    Off,
    /// The first call to see this pause; the caller logs it.
    Started,
    Running,
}

struct Failures {
    count: u32,
    until: Instant,
}

impl Throttle {
    /// How long `peer` is still refused before its next login is checked.
    pub fn delay(&self, peer: IpAddr, now: Instant) -> Duration {
        self.peers
            .get(&peer)
            .map_or(Duration::ZERO, |f| f.until.saturating_duration_since(now))
    }

    pub fn failed(&mut self, peer: IpAddr, now: Instant) -> Attempt {
        self.forget(now);
        let f = self.peers.entry(peer).or_insert(Failures {
            count: 0,
            until: now,
        });
        f.count += 1;
        f.until = now + wait_after(f.count);
        self.recent.push_back(now);
        Attempt(now)
    }

    /// Takes back `attempt`'s failure and forgets `peer`'s; the failures of other logins stay.
    pub fn succeeded(&mut self, peer: IpAddr, attempt: Attempt, now: Instant) {
        self.forget(now);
        self.peers.remove(&peer);
        if let Some(i) = self.recent.iter().rposition(|&t| t == attempt.0) {
            self.recent.remove(i);
        }
    }

    /// Whether every login is paused; `Started` comes once per pause.
    pub fn paused(&mut self, now: Instant) -> Pause {
        let Some(until) = self.pause_end() else {
            return Pause::Off;
        };
        if now >= until {
            return Pause::Off;
        }
        if self.announced == Some(until) {
            return Pause::Running;
        }
        self.announced = Some(until);
        Pause::Started
    }

    pub fn failures(&self, peer: IpAddr) -> u32 {
        self.peers.get(&peer).map_or(0, |f| f.count)
    }

    fn forget(&mut self, now: Instant) {
        self.peers
            .retain(|_, f| now.duration_since(f.until - wait_after(f.count)) < FORGET);
        while self
            .recent
            .front()
            .is_some_and(|&t| now.duration_since(t) >= PAUSE_WINDOW + PAUSE)
        {
            self.recent.pop_front();
        }
    }

    /// The end of the pause started by the latest failures. No login is checked during a pause,
    /// so only the latest `PAUSE_AFTER` can have started the one still running.
    fn pause_end(&self) -> Option<Instant> {
        let n = self.recent.len();
        let first = *self.recent.get(n.checked_sub(PAUSE_AFTER)?)?;
        let last = *self.recent.back()?;
        (last.duration_since(first) < PAUSE_WINDOW).then_some(last + PAUSE)
    }
}

fn wait_after(failures: u32) -> Duration {
    match failures.checked_sub(FREE_FAILURES) {
        None | Some(0) => Duration::ZERO,
        Some(n) => Duration::from_secs(1u64 << n.min(6)).min(MAX_WAIT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
    const B: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// `n` failures one second apart, each from its own address, the first at `t0`.
    fn spread_failures(t: &mut Throttle, t0: Instant, n: usize) {
        for i in 0..n {
            t.failed(
                IpAddr::V4(Ipv4Addr::new(198, 51, 100, i as u8)),
                t0 + secs(i as u64),
            );
        }
    }

    #[test]
    fn the_throttle_waits_from_the_fourth_failure() {
        let mut t = Throttle::default();
        let now = Instant::now();
        let mut waits = Vec::new();
        for _ in 0..10 {
            t.failed(A, now);
            waits.push(t.delay(A, now).as_secs());
        }
        assert_eq!(waits, [0, 0, 0, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn success_resets_the_address() {
        let mut t = Throttle::default();
        let now = Instant::now();
        for _ in 0..4 {
            t.failed(A, now);
        }
        let attempt = t.failed(A, now);
        t.succeeded(A, attempt, now);
        assert_eq!(t.delay(A, now), Duration::ZERO);
        for _ in 0..3 {
            t.failed(A, now);
        }
        assert_eq!(
            t.delay(A, now),
            Duration::ZERO,
            "the count starts again after a success"
        );
    }

    #[test]
    fn addresses_are_throttled_apart() {
        let mut t = Throttle::default();
        let now = Instant::now();
        for _ in 0..5 {
            t.failed(A, now);
        }
        t.failed(B, now);
        assert_eq!(t.delay(A, now), secs(4));
        assert_eq!(t.delay(B, now), Duration::ZERO);
    }

    #[test]
    fn an_address_is_forgotten_20_minutes_after_its_last_failure() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            t.failed(A, t0);
        }
        t.failed(B, t0 + FORGET - secs(1));
        assert_eq!(t.failures(A), 5, "a failure 1 s short of FORGET is kept");
        t.failed(B, t0 + FORGET);
        assert_eq!(t.failures(A), 0);
        assert_eq!(t.failures(B), 2);
    }

    #[test]
    fn the_wait_runs_down() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        for _ in 0..4 {
            t.failed(A, t0);
        }
        assert_eq!(
            t.delay(A, t0 + Duration::from_millis(1500)),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn ten_failures_in_10_minutes_start_one_pause_of_10_minutes() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        spread_failures(&mut t, t0, PAUSE_AFTER - 1);
        assert_eq!(t.paused(t0 + secs(9)), Pause::Off);
        spread_failures(&mut t, t0 + secs(9), 1);
        let last = t0 + secs(9);
        assert_eq!(t.paused(last), Pause::Started);
        assert_eq!(t.paused(last), Pause::Running, "a pause starts once");
        assert_eq!(t.paused(last + PAUSE - secs(1)), Pause::Running);
        assert_eq!(t.paused(last + PAUSE), Pause::Off);
    }

    #[test]
    fn failures_spread_over_more_than_10_minutes_start_no_pause() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        spread_failures(&mut t, t0, PAUSE_AFTER - 1);
        t.failed(A, t0 + PAUSE_WINDOW);
        assert_eq!(t.paused(t0 + PAUSE_WINDOW), Pause::Off);
    }

    #[test]
    fn a_success_takes_back_its_own_failure_only() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        spread_failures(&mut t, t0, PAUSE_AFTER - 1);
        let attempt = t.failed(A, t0 + secs(20));
        t.succeeded(A, attempt, t0 + secs(21));
        assert_eq!(
            t.paused(t0 + secs(21)),
            Pause::Off,
            "a success is no failure"
        );
        t.failed(B, t0 + secs(22));
        assert_eq!(t.paused(t0 + secs(22)), Pause::Started);
    }
}
