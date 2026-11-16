//! Where `config.toml` lies. The compositor hands the file it reads to every program it starts as
//! [`ENV`], so `reprise --config` reaches them too.

use reprisebus::mutex::lock;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;

/// The config file the compositor reads, set in every program it starts.
pub const ENV: &str = "REPRISE_CONFIG";

static SET: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Points [`path`] at `path` for the whole process: the compositor's `--config`, or a test's own
/// file, so a program's tests never read the developer's.
pub fn set_path(path: Option<PathBuf>) {
    *lock(&SET) = path;
}

/// The config file: the one [`set_path`] set, else [`ENV`]'s when absolute, else
/// `$XDG_CONFIG_HOME/reprise/config.toml`.
pub fn path() -> PathBuf {
    if let Some(path) = lock(&SET).clone() {
        return path;
    }
    path_from(std::env::var_os(ENV), crate::autostart::config_home())
}

fn path_from(env: Option<OsString>, config_home: PathBuf) -> PathBuf {
    env.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| config_home.join("reprise").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compositors_file_wins_over_the_default_when_absolute() {
        let home = || PathBuf::from("/home/u/.config");
        let default = PathBuf::from("/home/u/.config/reprise/config.toml");
        assert_eq!(path_from(None, home()), default);
        assert_eq!(
            path_from(Some("/tmp/test.toml".into()), home()),
            PathBuf::from("/tmp/test.toml")
        );
        assert_eq!(path_from(Some("".into()), home()), default, "empty");
        assert_eq!(
            path_from(Some("x.toml".into()), home()),
            default,
            "relative"
        );
    }
}
