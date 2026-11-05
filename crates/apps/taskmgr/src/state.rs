//! Task Manager's state in `$XDG_STATE_HOME/reprise/taskmgr.toml`: the View menu's picks and the
//! Processes columns.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Speed;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub speed: Speed,
    pub per_core: bool,
    /// The Processes columns shown, by [`crate::columns::Spec::key`]; None shows the defaults.
    pub columns: Option<Vec<String>>,
    /// Column widths a divider drag set, in 100% px, by key.
    pub widths: BTreeMap<String, i32>,
}

impl State {
    pub fn path() -> PathBuf {
        repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
            .join("reprise")
            .join("taskmgr.toml")
    }

    pub fn load(path: &Path) -> State {
        let Ok(text) = std::fs::read_to_string(path) else {
            return State::default();
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("task manager state: ignoring {}: {e}", path.display());
            State::default()
        })
    }

    pub fn save(&self, path: &Path) {
        match toml::to_string(self) {
            Ok(text) => {
                if let Err(e) = repriseconf::write_atomic(path, &text) {
                    log::warn!(
                        "task manager state: could not write {}: {e}",
                        path.display()
                    );
                }
            }
            Err(e) => log::warn!("task manager state: could not serialize: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_round_trips_and_a_partial_file_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("taskmgr.toml");
        assert_eq!(State::load(&path), State::default(), "no file yet");
        let s = State {
            speed: Speed::Low,
            per_core: true,
            columns: Some(vec!["name".into(), "command_line".into()]),
            widths: BTreeMap::from([("name".into(), 150)]),
        };
        s.save(&path);
        assert_eq!(State::load(&path), s);
        std::fs::write(&path, "speed = \"paused\"\n").unwrap();
        assert_eq!(
            State::load(&path),
            State {
                speed: Speed::Paused,
                ..State::default()
            }
        );
        std::fs::write(&path, "speed = 3\n").unwrap();
        assert_eq!(
            State::load(&path),
            State::default(),
            "a bad file is ignored"
        );
    }
}
