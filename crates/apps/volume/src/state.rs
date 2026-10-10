//! Volume Control state in `$XDG_STATE_HOME/reprise/volume.toml`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub hidden: BTreeSet<String>,
    pub advanced: bool,
}

impl State {
    pub fn path() -> PathBuf {
        repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
            .join("reprise")
            .join("volume.toml")
    }

    pub fn load(path: &Path) -> State {
        let Ok(text) = std::fs::read_to_string(path) else {
            return State::default();
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("volume state: ignoring {}: {e}", path.display());
            State::default()
        })
    }

    pub fn save(&self, path: &Path) {
        match toml::to_string(self) {
            Ok(text) => {
                if let Err(e) = repriseconf::write_atomic(path, &text) {
                    log::warn!("volume state: could not write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("volume state: could not serialize: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_round_trips_and_a_partial_file_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("volume.toml");
        assert_eq!(State::load(&path), State::default(), "no file yet");
        let s = State {
            hidden: BTreeSet::from(["app:firefox".into(), "source:mic".into()]),
            advanced: true,
        };
        s.save(&path);
        assert_eq!(State::load(&path), s);
        std::fs::write(&path, "advanced = true\n").unwrap();
        assert_eq!(
            State::load(&path),
            State {
                advanced: true,
                ..State::default()
            }
        );
        std::fs::write(&path, "advanced = 3\n").unwrap();
        assert_eq!(
            State::load(&path),
            State::default(),
            "a bad file is ignored"
        );
    }
}
