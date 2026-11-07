use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use repriselogin::Interface;
use serde::{Deserialize, Serialize};

pub const STATE_PATH: &str = "/var/lib/reprise-login/state.toml";

/// What the login screen offers first: the last user who logged in and the interface they used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Remembered {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_interface: Option<Interface>,
}

impl Remembered {
    /// Reads the file at `path`; a missing or unreadable one gives nothing remembered.
    pub fn load(path: &Path) -> Remembered {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Remembered::default(),
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                return Remembered::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("{}: {e}", path.display());
            Remembered::default()
        })
    }

    /// Writes a 0600 temp file next to `path` and renames it over `path`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string(self).map_err(io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn remembered_round_trips_with_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        assert_eq!(Remembered::load(&path), Remembered::default());
        let state = Remembered {
            last_user: Some("anna".into()),
            last_interface: Some(Interface::Blue),
        };
        state.save(&path).unwrap();
        assert_eq!(Remembered::load(&path), state);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "the temp file is renamed over the state");
    }
}
