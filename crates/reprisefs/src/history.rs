//! Linear navigation history with a Back/Forward branch, like a
//! browser's: visiting a new place after going back drops everything
//! that was ahead of the cursor.

/// How many places a [`History`] keeps; a visit past it forgets the
/// oldest.
const MAX_ENTRIES: usize = 300;

/// A cursor over a list of visited `T`s.
pub struct History<T> {
    entries: Vec<T>,
    at: usize,
}

impl<T> History<T> {
    pub fn new(first: T) -> Self {
        Self {
            entries: vec![first],
            at: 0,
        }
    }

    pub fn current(&self) -> &T {
        &self.entries[self.at]
    }

    /// Visit `t`: drops anything ahead of the cursor, then appends and
    /// moves onto `t`. Past [`MAX_ENTRIES`] the oldest place goes.
    pub fn visit(&mut self, t: T) {
        self.entries.truncate(self.at + 1);
        self.entries.push(t);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.at = self.entries.len() - 1;
    }

    pub fn back(&mut self) -> Option<&T> {
        self.go(1)
    }

    pub fn forward(&mut self) -> Option<&T> {
        if self.at + 1 >= self.entries.len() {
            return None;
        }
        self.at += 1;
        Some(&self.entries[self.at])
    }

    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }

    /// The places behind the cursor, nearest first, for the Back
    /// button's drop-down.
    pub fn back_list(&self) -> Vec<&T> {
        self.entries[..self.at].iter().rev().collect()
    }

    /// The places ahead of the cursor, nearest first, for the Forward
    /// button's drop-down.
    pub fn forward_list(&self) -> Vec<&T> {
        self.entries[(self.at + 1).min(self.entries.len())..]
            .iter()
            .collect()
    }

    /// Jump `steps_back` places back (0 is a no-op, staying on the
    /// current place). `None` and unchanged if that overshoots the
    /// start.
    pub fn go(&mut self, steps_back: usize) -> Option<&T> {
        if steps_back > self.at {
            return None;
        }
        self.at -= steps_back;
        Some(&self.entries[self.at])
    }

    /// [`History::go`]'s mirror: jump `steps_forward` places forward (0
    /// stays put). `None` and unchanged past the end.
    pub fn go_forward(&mut self, steps_forward: usize) -> Option<&T> {
        let to = self.at.checked_add(steps_forward)?;
        if to >= self.entries.len() {
            return None;
        }
        self.at = to;
        Some(&self.entries[self.at])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visit_back_forward_walk_the_list() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        assert_eq!(*h.current(), "c");
        assert_eq!(h.back(), Some(&"b"));
        assert_eq!(h.back(), Some(&"a"));
        assert_eq!(h.back(), None);
        assert_eq!(*h.current(), "a");
        assert_eq!(h.forward(), Some(&"b"));
        assert_eq!(h.forward(), Some(&"c"));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn visit_after_back_truncates_the_forward_branch() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        h.back();
        h.back();
        assert!(!h.can_back());

        h.visit("d");

        assert_eq!(*h.current(), "d");
        assert!(!h.can_forward());
        assert_eq!(h.back_list(), vec![&"a"]);
    }

    #[test]
    fn visit_past_the_cap_forgets_the_oldest_place() {
        let mut h = History::new(0);
        for i in 1..=MAX_ENTRIES {
            h.visit(i);
        }
        assert_eq!(*h.current(), MAX_ENTRIES);
        assert_eq!(h.back_list().len(), MAX_ENTRIES - 1);
        assert_eq!(h.back_list().last(), Some(&&1), "place 0 is forgotten");
        assert_eq!(h.go(MAX_ENTRIES - 1), Some(&1));
    }

    #[test]
    fn can_back_and_forward_reflect_position() {
        let mut h = History::new(1);
        assert!(!h.can_back());
        assert!(!h.can_forward());
        h.visit(2);
        assert!(h.can_back());
        assert!(!h.can_forward());
        h.back();
        assert!(!h.can_back());
        assert!(h.can_forward());
    }

    #[test]
    fn back_list_is_nearest_first() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        assert_eq!(h.back_list(), vec![&"b", &"a"]);
    }

    /// The Forward drop reads the other way: nearest first is the place
    /// one step ahead, so row 0 is what one Forward would reach.
    #[test]
    fn forward_list_is_nearest_first_and_empty_at_the_end() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        assert!(h.forward_list().is_empty());
        h.back();
        h.back();
        assert_eq!(h.forward_list(), vec![&"b", &"c"]);
        h.forward();
        assert_eq!(h.forward_list(), vec![&"c"]);
    }

    #[test]
    fn go_forward_jumps_directly_and_rejects_overshoot() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        h.go(2);
        assert_eq!(h.go_forward(2), Some(&"c"));
        assert_eq!(h.go_forward(1), None);
        assert_eq!(*h.current(), "c");
        assert_eq!(h.go_forward(0), Some(&"c"));
    }

    #[test]
    fn go_jumps_directly_and_rejects_overshoot() {
        let mut h = History::new("a");
        h.visit("b");
        h.visit("c");
        assert_eq!(h.go(2), Some(&"a"));
        assert_eq!(*h.current(), "a");
        assert_eq!(h.go(1), None);
        assert_eq!(*h.current(), "a");
        assert_eq!(h.go(0), Some(&"a"));
    }
}
