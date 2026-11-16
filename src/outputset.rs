//! A name for the set of screens connected now, so what is kept per layout (a docked and an
//! undocked one) is told apart.

use serde::{Deserialize, Serialize};
use smithay::output::Output;

use crate::compositor::State;

/// A monitor's id (`displays::monitor_ids`) among the outputs in the layout: the same monitor
/// keeps it on another connector.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorId(pub String);

impl From<&str> for MonitorId {
    fn from(id: &str) -> MonitorId {
        MonitorId(id.to_string())
    }
}

/// The set's name: its monitor ids (`displays::monitor_ids`) sorted, so the order they came in
/// does not count.
pub fn fingerprint(mut ids: Vec<String>) -> String {
    ids.sort();
    ids.join(" + ")
}

impl State {
    /// The [`fingerprint`] of the outputs in the layout now.
    pub fn output_set(&self) -> String {
        fingerprint(self.monitors().into_iter().map(|(_, id)| id.0).collect())
    }

    /// The outputs in the layout with their monitor ids, the one list every id is taken from.
    pub fn monitors(&self) -> Vec<(Output, MonitorId)> {
        let outputs: Vec<&Output> = (self.scene.outputs().iter())
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .collect();
        let ids = crate::outputlayouts::monitor_ids(&outputs);
        (outputs.into_iter().cloned())
            .zip(ids.into_iter().map(MonitorId))
            .collect()
    }

    /// `out`'s monitor id; None when it is not in the layout.
    pub fn monitor_id(&self, out: &Output) -> Option<MonitorId> {
        (self.monitors().into_iter()).find_map(|(o, id)| (o == *out).then_some(id))
    }

    /// The output in the layout with monitor id `id`.
    pub fn output_of_monitor(&self, id: &MonitorId) -> Option<Output> {
        (self.monitors().into_iter()).find_map(|(o, m)| (m == *id).then_some(o))
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
