//! A name for the set of screens connected now, so what is kept per layout (a docked and an
//! undocked one) is told apart.

use smithay::output::Output;

use crate::compositor::State;

/// The set's name: its monitor ids (`displays::monitor_ids`) sorted, so the order they came in
/// does not count.
pub fn fingerprint(mut ids: Vec<String>) -> String {
    ids.sort();
    ids.join(" + ")
}

impl State {
    /// The [`fingerprint`] of the outputs in the layout now.
    pub fn output_set(&self) -> String {
        let outputs: Vec<&Output> = (self.scene.outputs().iter())
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .collect();
        fingerprint(crate::outputlayouts::monitor_ids(&outputs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_does_not_depend_on_the_order_its_screens_came_in() {
        let a = fingerprint(vec!["eDP-1".into(), "Dell|U2720Q|".into()]);
        let b = fingerprint(vec!["Dell|U2720Q|".into(), "eDP-1".into()]);
        assert_eq!(a, b);
        assert_eq!(a, "Dell|U2720Q| + eDP-1");
        assert_ne!(
            fingerprint(vec!["eDP-1".into()]),
            a,
            "undocked is its own set"
        );
    }
}
