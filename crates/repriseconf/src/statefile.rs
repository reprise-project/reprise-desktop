//! A program's own state between runs: a TOML file in `$XDG_STATE_HOME/reprise`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `$XDG_STATE_HOME/reprise/<name>.toml`.
pub fn path(name: &str) -> PathBuf {
    reprisexdg::state_home()
        .unwrap_or_default()
        .join("reprise")
        .join(format!("{name}.toml"))
}

/// The state in `path`: the default when the file can't be read, and, with a warning, when it
/// doesn't parse.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    toml::from_str(&text).unwrap_or_else(|e| {
        log::warn!("{}: ignoring the state: {e}", path.display());
        T::default()
    })
}

/// Writes `state` to `path` with [`crate::write_atomic`], warning when it can't.
pub fn save<T: Serialize>(path: &Path, state: &T) {
    let written = toml::to_string(state)
        .map_err(std::io::Error::other)
        .and_then(|text| crate::write_atomic(path, &text));
    if let Err(e) = written {
        log::warn!("{}: could not write the state: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Kept {
        on: bool,
        n: u32,
    }

    #[test]
    fn state_round_trips_and_a_missing_or_broken_file_reads_as_the_default() {
        let dir = std::env::temp_dir().join(format!("repriseconf-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("reprise").join("kept.toml");
        assert_eq!(load::<Kept>(&path), Kept::default(), "no file yet");
        let kept = Kept { on: true, n: 3 };
        save(&path, &kept);
        assert_eq!(load::<Kept>(&path), kept);
        std::fs::write(&path, "n = \"three\"\n").unwrap();
        assert_eq!(
            load::<Kept>(&path),
            Kept::default(),
            "a bad file is ignored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_path_is_named_after_the_program() {
        assert!(path("volume").ends_with("reprise/volume.toml"));
    }
}
