//! Where each held key's press went, so its release goes the same way
//! (`keyboard`'s route): one owner per xkb keycode.

use std::collections::HashMap;

use crate::keyroute::GlobalHit;

/// Where a key's press went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The seat: the focused client or a seat grab. Never stored, so a
    /// key the client holds has no entry.
    Client,
    /// Reprise's own route: no client heard the press.
    Taken,
    /// Held while the pointer took the keys from the client (a drag, a
    /// click that closed the switcher): the release reaches no client.
    Drag,
    /// The input method, serving Reprise's own field.
    Ime,
    /// The global shortcuts the press fired; the release sends each one
    /// deactivated.
    Global(Vec<GlobalHit>),
}

/// The held keys that a client does not own, by xkb keycode.
#[derive(Debug, Default)]
pub struct HeldKeys(HashMap<u32, Owner>);

impl HeldKeys {
    /// `code`'s press went to `owner`, replacing what an earlier press of
    /// the same key left. A key holding global shortcuts keeps them until
    /// its release, adding those a further press fires.
    pub fn press(&mut self, code: u32, owner: Owner) {
        if let Some(Owner::Global(held)) = self.0.get_mut(&code) {
            if let Owner::Global(hits) = owner {
                held.extend(hits);
            }
            return;
        }
        match owner {
            Owner::Client => {
                self.0.remove(&code);
            }
            owner => {
                self.0.insert(code, owner);
            }
        }
    }

    /// Where `code`'s press went, as its release forgets it; `None` for
    /// the client's keys.
    pub fn release(&mut self, code: u32) -> Option<Owner> {
        self.0.remove(&code)
    }

    /// `code`, still down, was taken from the client by the pointer: its
    /// release reaches no client. The input method and global shortcuts
    /// keep theirs.
    pub fn hold_back(&mut self, code: u32) {
        if !matches!(self.0.get(&code), Some(Owner::Ime | Owner::Global(_))) {
            self.0.insert(code, Owner::Drag);
        }
    }

    /// `code` is down and its press went to Reprise's own route.
    pub fn is_taken(&self, code: u32) -> bool {
        self.0.get(&code) == Some(&Owner::Taken)
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str) -> GlobalHit {
        GlobalHit {
            app: "talk".into(),
            id: id.into(),
        }
    }

    #[test]
    fn a_clients_key_leaves_no_entry_and_clears_an_earlier_owner() {
        let mut h = HeldKeys::default();
        h.press(30, Owner::Client);
        assert!(h.is_empty());
        assert_eq!(h.release(30), None);
        h.press(30, Owner::Taken);
        h.press(30, Owner::Client);
        assert_eq!(h.release(30), None, "the second press went to the client");
    }

    #[test]
    fn a_taken_key_is_released_once() {
        let mut h = HeldKeys::default();
        h.press(105, Owner::Taken);
        h.press(30, Owner::Client);
        assert_eq!(h.release(105), Some(Owner::Taken));
        assert_eq!(h.release(105), None);
        assert!(h.is_empty());
    }

    #[test]
    fn a_held_back_key_drops_its_release_until_pressed_again() {
        let mut h = HeldKeys::default();
        h.hold_back(30);
        h.press(105, Owner::Taken);
        h.hold_back(105);
        assert_eq!(h.release(30), Some(Owner::Drag));
        assert_eq!(h.release(105), Some(Owner::Drag));
        h.hold_back(30);
        h.press(30, Owner::Client);
        assert_eq!(h.release(30), None, "a new press ends the hold");
    }

    #[test]
    fn holding_back_leaves_the_input_method_and_global_shortcuts_their_keys() {
        let mut h = HeldKeys::default();
        h.press(30, Owner::Ime);
        h.press(40, Owner::Global(vec![hit("mute")]));
        h.hold_back(30);
        h.hold_back(40);
        assert_eq!(h.release(30), Some(Owner::Ime));
        assert_eq!(h.release(40), Some(Owner::Global(vec![hit("mute")])));
    }

    #[test]
    fn an_input_method_key_pressed_twice_is_released_once() {
        let mut h = HeldKeys::default();
        h.press(30, Owner::Ime);
        h.press(30, Owner::Ime);
        assert_eq!(h.release(30), Some(Owner::Ime));
        assert_eq!(h.release(30), None, "the next release is the client's");
    }

    #[test]
    fn a_release_takes_only_its_own_global_shortcuts() {
        let mut h = HeldKeys::default();
        h.press(40, Owner::Global(vec![hit("mute")]));
        h.press(41, Owner::Global(vec![hit("deafen")]));
        h.press(40, Owner::Taken);
        h.press(40, Owner::Global(vec![hit("ptt")]));
        h.press(40, Owner::Client);
        assert_eq!(
            h.release(40),
            Some(Owner::Global(vec![hit("mute"), hit("ptt")]))
        );
        assert_eq!(h.release(40), None);
        assert_eq!(h.release(41), Some(Owner::Global(vec![hit("deafen")])));
    }
}
