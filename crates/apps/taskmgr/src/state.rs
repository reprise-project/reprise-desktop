//! Task Manager's state in `$XDG_STATE_HOME/reprise/taskmgr.toml`: the Options and View menus'
//! picks and the Processes columns.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Speed;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub on_top: bool,
    pub speed: Speed,
    pub per_core: bool,
    /// The Processes columns shown, by [`crate::columns::Spec::key`]; None shows the defaults.
    pub columns: Option<Vec<String>>,
    /// Column widths a divider drag set, in 100% px, by key.
    pub widths: BTreeMap<String, i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_round_trips_and_a_partial_file_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("taskmgr.toml");
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State::default(),
            "no file yet"
        );
        let s = State {
            on_top: true,
            speed: Speed::Low,
            per_core: true,
            columns: Some(vec!["name".into(), "command_line".into()]),
            widths: BTreeMap::from([("name".into(), 150)]),
        };
        repriseconf::statefile::save(&path, &s);
        assert_eq!(repriseconf::statefile::load::<State>(&path), s);
        std::fs::write(&path, "speed = \"paused\"\n").unwrap();
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State {
                speed: Speed::Paused,
                ..State::default()
            }
        );
        std::fs::write(&path, "speed = 3\n").unwrap();
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State::default(),
            "a bad file is ignored"
        );
    }
}
