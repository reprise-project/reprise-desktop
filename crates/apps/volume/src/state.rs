//! Volume Control state in `$XDG_STATE_HOME/reprise/volume.toml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub hidden: BTreeSet<String>,
    pub advanced: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_round_trips_and_a_partial_file_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("volume.toml");
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State::default(),
            "no file yet"
        );
        let s = State {
            hidden: BTreeSet::from(["app:firefox".into(), "source:mic".into()]),
            advanced: true,
        };
        repriseconf::statefile::save(&path, &s);
        assert_eq!(repriseconf::statefile::load::<State>(&path), s);
        std::fs::write(&path, "advanced = true\n").unwrap();
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State {
                advanced: true,
                ..State::default()
            }
        );
        std::fs::write(&path, "advanced = 3\n").unwrap();
        assert_eq!(
            repriseconf::statefile::load::<State>(&path),
            State::default(),
            "a bad file is ignored"
        );
    }
}
