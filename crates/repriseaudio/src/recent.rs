//! The levels a program wrote lately, standing in for the ones `pw-dump`
//! reports until those catch up.

use std::time::{Duration, Instant};

/// How long a write stands in for the level the snapshot has: `pw-dump`
/// reports a write back a moment later, and a key held down or a wheel
/// burst counts on from the write, not the stale level. Short enough
/// that a write nobody honoured (or a level changed elsewhere) is
/// forgotten before the next one.
pub const SETTLE: Duration = Duration::from_secs(1);

/// The last value written per key, and when; each stands in for
/// [`SETTLE`]. A program keeps one for every path that writes the same
/// levels (the tray wheel and the volume popup), so each counts on from
/// the other's write.
#[derive(Clone, Debug)]
pub struct Recent<K, V>(Vec<(K, V, Instant)>);

impl<K, V> Default for Recent<K, V> {
    fn default() -> Self {
        Recent(Vec::new())
    }
}

impl<K: PartialEq, V: Copy> Recent<K, V> {
    /// `value` was written to `key` at `at`: it replaces the key's last
    /// write, and writes that have settled are dropped.
    pub fn note(&mut self, key: K, value: V, at: Instant) {
        self.0
            .retain(|(k, _, t)| *k != key && at.saturating_duration_since(*t) < SETTLE);
        self.0.push((key, value, at));
    }

    /// The value last written to `key`, while it has not settled.
    pub fn get(&self, key: &K) -> Option<V> {
        self.0
            .iter()
            .find(|(k, _, t)| k == key && t.elapsed() < SETTLE)
            .map(|(_, v, _)| *v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_stands_in_until_it_settles() {
        let mut r = Recent::default();
        r.note(41, 0.7, Instant::now());
        assert_eq!(r.get(&41), Some(0.7));
        assert_eq!(r.get(&40), None, "another key has no write");
        let mut old = Recent::default();
        old.note(41, 0.7, Instant::now() - SETTLE);
        assert_eq!(old.get(&41), None, "a settled write is forgotten");
    }

    #[test]
    fn a_write_replaces_its_keys_last_and_drops_settled_ones() {
        let mut r = Recent::default();
        let now = Instant::now();
        r.note(40, 0.2, now - SETTLE);
        r.note(41, 0.5, now);
        r.note(41, 0.55, now);
        assert_eq!(r.get(&41), Some(0.55));
        assert_eq!(r.0.len(), 1, "one entry per key, the settled one gone");
    }
}
