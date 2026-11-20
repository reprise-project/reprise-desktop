//! The tokens [`Reply::handoffs`](crate::Reply::handoffs) asks for, given to the program as
//! `Event::Handoff` in the order asked: a tokenless one waits behind those still pending, and one
//! pending for [`LIFE`] goes tokenless, so a start the program holds for it never waits forever.

use reprisebus::instance::Handoff;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a handoff waits for its token: past the compositor's 30 s token life.
pub(crate) const LIFE: Duration = Duration::from_secs(35);

/// Handoffs in the order asked.
#[derive(Default)]
pub(crate) struct Hands {
    next: u64,
    queue: VecDeque<Hand>,
}

struct Hand {
    id: u64,
    asked: Instant,
    by_key: bool,
    /// `None` while the token is pending; `Some(None)` for a tokenless handoff.
    token: Option<Option<String>>,
}

impl Hands {
    /// Queues a handoff whose token is asked for at `now`, under the id [`Hands::answer`] takes.
    pub(crate) fn ask(&mut self, by_key: bool, now: Instant) -> u64 {
        let id = self.next;
        self.next += 1;
        self.queue.push_back(Hand {
            id,
            asked: now,
            by_key,
            token: None,
        });
        id
    }

    /// Queues a handoff with no token, behind those pending.
    pub(crate) fn tokenless(&mut self, by_key: bool, now: Instant) {
        let id = self.ask(by_key, now);
        self.answer_with(id, None);
    }

    /// Fills handoff `id` with `token`; nothing once it went tokenless.
    pub(crate) fn answer(&mut self, id: u64, token: String) {
        self.answer_with(id, Some(token));
    }

    fn answer_with(&mut self, id: u64, token: Option<String>) {
        if let Some(h) = self
            .queue
            .iter_mut()
            .find(|h| h.id == id && h.token.is_none())
        {
            h.token = Some(token);
        }
    }

    /// Makes every handoff pending since [`LIFE`] before `now` tokenless.
    pub(crate) fn expire(&mut self, now: Instant) {
        for h in &mut self.queue {
            if h.token.is_none() && now.saturating_duration_since(h.asked) >= LIFE {
                h.token = Some(None);
            }
        }
    }

    /// The oldest handoff, once it is no longer pending.
    pub(crate) fn pop(&mut self) -> Option<Handoff> {
        self.queue.front()?.token.as_ref()?;
        let h = self.queue.pop_front()?;
        Some(Handoff {
            token: h.token.flatten(),
            by_key: h.by_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn given(hands: &mut Hands) -> Vec<Option<String>> {
        std::iter::from_fn(|| hands.pop())
            .map(|h| h.token)
            .collect()
    }

    #[test]
    fn a_tokenless_handoff_waits_behind_a_pending_one() {
        let now = Instant::now();
        let mut hands = Hands::default();
        let a = hands.ask(false, now);
        hands.tokenless(false, now);
        assert_eq!(given(&mut hands), Vec::<Option<String>>::new());
        hands.answer(a, "a".into());
        assert_eq!(given(&mut hands), [Some("a".into()), None]);
    }

    #[test]
    fn handoffs_keep_the_order_asked_whatever_order_tokens_come_in() {
        let now = Instant::now();
        let mut hands = Hands::default();
        let a = hands.ask(true, now);
        let b = hands.ask(false, now);
        hands.answer(b, "b".into());
        assert!(hands.pop().is_none(), "b waits for a");
        hands.answer(a, "a".into());
        assert_eq!(
            hands.pop(),
            Some(Handoff {
                token: Some("a".into()),
                by_key: true
            })
        );
        assert_eq!(given(&mut hands), [Some("b".into())]);
    }

    #[test]
    fn a_handoff_pending_for_its_life_goes_tokenless_and_its_late_token_is_dropped() {
        let t0 = Instant::now();
        let mut hands = Hands::default();
        let a = hands.ask(false, t0);
        let b = hands.ask(false, t0 + Duration::from_secs(10));
        hands.expire(t0 + LIFE - Duration::from_millis(1));
        assert!(hands.pop().is_none(), "a still waits");
        hands.expire(t0 + LIFE);
        assert_eq!(given(&mut hands), [None]);
        hands.answer(a, "late".into());
        assert!(hands.pop().is_none(), "a's late token is not b's");
        hands.answer(b, "b".into());
        assert_eq!(given(&mut hands), [Some("b".into())]);
    }
}
