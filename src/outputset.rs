//! A name for the set of screens connected now, so what is kept per layout (a docked and an
//! undocked one) is told apart.

use smithay::output::Output;

use crate::compositor::State;

/// What a backend reports for a field EDID does not give.
const UNKNOWN: &str = "Unknown";

/// One screen's name in the set: its make, model and serial, or its connector name when EDID gives
/// neither make nor model.
pub fn head(connector: &str, make: &str, model: &str, serial: &str) -> String {
    let known = |s: &str| !s.is_empty() && s != UNKNOWN;
    let id: Vec<&str> = [make, model, serial]
        .into_iter()
        .filter(|s| known(s))
        .collect();
    if known(make) || known(model) {
        id.join(" ")
    } else {
        connector.to_string()
    }
}

/// The set's name: its screens' [`head`]s sorted, so the order they came in does not count.
pub fn fingerprint(mut heads: Vec<String>) -> String {
    heads.sort();
    heads.join(" + ")
}

impl State {
    /// The [`fingerprint`] of the outputs in the layout now.
    pub fn output_set(&self) -> String {
        let heads = (self.scene.outputs().iter())
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .map(output_head)
            .collect();
        fingerprint(heads)
    }
}

fn output_head(o: &Output) -> String {
    let p = o.physical_properties();
    head(&o.name(), &p.make, &p.model, &p.serial_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_is_named_by_its_make_model_and_serial_else_its_connector() {
        assert_eq!(
            head("DP-1", "Dell", "U2720Q", "ABC123"),
            "Dell U2720Q ABC123"
        );
        assert_eq!(head("DP-1", "Dell", "U2720Q", "Unknown"), "Dell U2720Q");
        assert_eq!(head("eDP-1", "BOE", "", ""), "BOE");
        assert_eq!(
            head("Virtual-1", "Unknown", "Unknown", "Unknown"),
            "Virtual-1"
        );
        assert_eq!(head("HDMI-A-1", "", "", "42"), "HDMI-A-1");
    }

    #[test]
    fn the_set_does_not_depend_on_the_order_its_screens_came_in() {
        let a = fingerprint(vec!["eDP-1".into(), "Dell U2720Q".into()]);
        let b = fingerprint(vec!["Dell U2720Q".into(), "eDP-1".into()]);
        assert_eq!(a, b);
        assert_eq!(a, "Dell U2720Q + eDP-1");
        assert_ne!(
            fingerprint(vec!["eDP-1".into()]),
            a,
            "undocked is its own set"
        );
    }
}
