/// Unclean greeter ends in a row before the text login takes over.
pub const MAX_FAILURES: u8 = 3;

/// Counts greeter runs that ended uncleanly in a row.
#[derive(Debug, Default)]
pub struct GreeterRuns {
    failures: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterGreeter {
    Again,
    TextLogin,
}

impl GreeterRuns {
    /// Records a greeter's end; the third unclean end in a row gives `TextLogin`.
    pub fn ended(&mut self, clean: bool) -> AfterGreeter {
        if clean {
            self.failures = 0;
            return AfterGreeter::Again;
        }
        self.failures = self.failures.saturating_add(1);
        if self.failures >= MAX_FAILURES {
            AfterGreeter::TextLogin
        } else {
            AfterGreeter::Again
        }
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_failures_in_a_row_fall_back_to_the_text_login() {
        let mut runs = GreeterRuns::default();
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::TextLogin);
        runs.reset();
        assert_eq!(runs.ended(false), AfterGreeter::Again);
    }

    #[test]
    fn a_clean_end_resets_the_count() {
        let mut runs = GreeterRuns::default();
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(true), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::Again);
        assert_eq!(runs.ended(false), AfterGreeter::TextLogin);
    }
}
