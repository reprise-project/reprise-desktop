//! A consent form ([`Form::set_consent`]): a prompt that grants something, so typing meant for
//! another window must not answer it. The host reports focus, keys and presses with the time.

use std::time::{Duration, Instant};

use super::Form;

/// How long a consent form ignores keys and presses after it gets the keyboard.
pub const ARM: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Gate {
    /// When the form last got the keyboard; `None` until it first does.
    since: Option<Instant>,
    /// The host's ids of keys that went down while the gate was shut: their repeats count no
    /// more than their press did, until they come up.
    held: Vec<u32>,
}

impl Gate {
    fn open(&self, now: Instant) -> bool {
        self.since
            .is_some_and(|t| now.saturating_duration_since(t) >= ARM)
    }
}

impl Form {
    /// Make this a consent form: an access key needs Alt (a bare letter presses nothing), Enter
    /// presses the default (refusing) button wherever the focus is, Space presses no other
    /// button and arrows don't move between buttons, so only a click or an Alt access key grants.
    /// Keys and presses count only [`ARM`] after the form gets the keyboard
    /// ([`Form::consent_focused`]). Hosts ask [`Form::consent_key`] and
    /// [`Form::consent_press`] before handing either on.
    pub fn set_consent(&mut self) {
        self.consent = Some(Gate::default());
    }

    pub fn is_consent(&self) -> bool {
        self.consent.is_some()
    }

    /// The window got the keyboard at `now`: a consent form starts its [`ARM`] wait again.
    pub fn consent_focused(&mut self, now: Instant) {
        if let Some(g) = &mut self.consent {
            g.since = Some(now);
        }
    }

    /// Whether key `id` (a host's keysym or keycode), down at `now`, may reach [`Form::key`].
    /// Always true on any other form. A key first down while the gate was shut stays refused,
    /// repeats included, until [`Form::consent_key_up`].
    pub fn consent_key(&mut self, id: u32, now: Instant) -> bool {
        let Some(g) = &mut self.consent else {
            return true;
        };
        if g.held.contains(&id) {
            return false;
        }
        if g.open(now) {
            return true;
        }
        g.held.push(id);
        false
    }

    pub fn consent_key_up(&mut self, id: u32) {
        if let Some(g) = &mut self.consent {
            g.held.retain(|&k| k != id);
        }
    }

    /// Whether a pointer press at `now` may reach [`Form::click`]. Always true on any other form.
    pub fn consent_press(&self, now: Instant) -> bool {
        self.consent.as_ref().is_none_or(|g| g.open(now))
    }
}
