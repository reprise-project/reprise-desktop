//! Refusals before a logon, logged so a storm of connections stays under journald's rate limit.

use std::collections::BTreeMap;
use std::time::Duration;

pub const REPORT_EVERY: Duration = Duration::from_secs(60);

/// The first refusal after each report is logged in full; the ones after it are counted by kind
/// until the next report.
#[derive(Default)]
pub struct Refusals {
    held_back: bool,
    counts: BTreeMap<&'static str, u32>,
}

impl Refusals {
    /// Whether a refusal of `kind` is logged in full; if not, it is counted for the report.
    pub fn log_in_full(&mut self, kind: &'static str) -> bool {
        if !self.held_back {
            self.held_back = true;
            return true;
        }
        *self.counts.entry(kind).or_default() += 1;
        false
    }

    /// The count of refusals held back since the last report, taken every `REPORT_EVERY`.
    pub fn report(&mut self) -> Option<String> {
        self.held_back = false;
        if self.counts.is_empty() {
            return None;
        }
        let kinds: Vec<String> = (self.counts.iter())
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect();
        self.counts.clear();
        Some(format!("refused more connections: {}", kinds.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_log_the_first_of_each_minute_in_full_and_count_the_rest() {
        let mut refusals = Refusals::default();
        assert!(refusals.log_in_full("backed off"), "the first is in full");
        for kind in ["backed off", "TLS", "backed off", "full"] {
            assert!(!refusals.log_in_full(kind), "{kind} is counted");
        }
        assert_eq!(
            refusals.report().as_deref(),
            Some("refused more connections: 1 TLS, 2 backed off, 1 full")
        );
        assert!(
            refusals.log_in_full("TLS"),
            "the first after each report is in full"
        );
        assert!(!refusals.log_in_full("TLS"), "the rest are counted");
        assert_eq!(
            refusals.report().as_deref(),
            Some("refused more connections: 1 TLS")
        );
        assert_eq!(refusals.report(), None, "a quiet minute has no line");
        assert!(
            refusals.log_in_full("TLS"),
            "the first after a quiet minute is in full"
        );
    }
}
