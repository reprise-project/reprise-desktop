//! The XDG base directories. An empty or relative `*_HOME` counts as unset, and a `*_DIRS` list
//! keeps its absolute entries, falling back to the spec's default when none is left.

use std::ffi::OsString;
use std::path::PathBuf;

/// `$XDG_DATA_HOME`, else `~/.local/share`; `None` without an absolute `$HOME`.
pub fn data_home() -> Option<PathBuf> {
    home("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_CONFIG_HOME`, else `~/.config`; `None` without an absolute `$HOME`.
pub fn config_home() -> Option<PathBuf> {
    home("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_CACHE_HOME`, else `~/.cache`; `None` without an absolute `$HOME`.
pub fn cache_home() -> Option<PathBuf> {
    home("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_STATE_HOME`, else `~/.local/state`; `None` without an absolute `$HOME`.
pub fn state_home() -> Option<PathBuf> {
    home("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_DATA_DIRS`, most important first, else `/usr/local/share` and `/usr/share`.
pub fn data_dirs() -> Vec<PathBuf> {
    dirs_from(
        std::env::var_os("XDG_DATA_DIRS"),
        &["/usr/local/share", "/usr/share"],
    )
}

/// `$XDG_CONFIG_DIRS`, most important first, else `/etc/xdg`.
pub fn config_dirs() -> Vec<PathBuf> {
    dirs_from(std::env::var_os("XDG_CONFIG_DIRS"), &["/etc/xdg"])
}

fn home(var: &str, fallback: &str) -> Option<PathBuf> {
    home_from(std::env::var_os(var), std::env::var_os("HOME"), fallback)
}

fn home_from(value: Option<OsString>, home: Option<OsString>, fallback: &str) -> Option<PathBuf> {
    let absolute = |v: OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    value
        .and_then(absolute)
        .or_else(|| home.and_then(absolute).map(|h| h.join(fallback)))
}

fn dirs_from(value: Option<OsString>, default: &[&str]) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = value
        .iter()
        .flat_map(std::env::split_paths)
        .filter(|p| p.is_absolute())
        .collect();
    if dirs.is_empty() {
        default.iter().map(PathBuf::from).collect()
    } else {
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_relative_value_counts_as_unset() {
        let home = |value: Option<&str>, home: Option<&str>| {
            home_from(value.map(Into::into), home.map(Into::into), ".local/share")
        };
        let at = |p: &str| Some(PathBuf::from(p));
        let homes = [
            (Some("/x/data"), Some("/home/u"), at("/x/data")),
            (None, Some("/home/u"), at("/home/u/.local/share")),
            (Some(""), Some("/home/u"), at("/home/u/.local/share")),
            (
                Some("rel/data"),
                Some("/home/u"),
                at("/home/u/.local/share"),
            ),
            (Some("/x/data"), None, at("/x/data")),
            (Some(""), None, None),
            (None, Some(""), None),
            (None, Some("rel"), None),
        ];
        for (value, h, want) in homes {
            assert_eq!(home(value, h), want, "{value:?} with HOME {h:?}");
        }

        let dirs = |value: Option<&str>| dirs_from(value.map(Into::into), &["/d1", "/d2"]);
        let list = |ps: &[&str]| ps.iter().map(PathBuf::from).collect::<Vec<_>>();
        let lists = [
            (None, list(&["/d1", "/d2"])),
            (Some(""), list(&["/d1", "/d2"])),
            (Some(":"), list(&["/d1", "/d2"])),
            (Some("rel"), list(&["/d1", "/d2"])),
            (Some("/a::/b"), list(&["/a", "/b"])),
            (Some("/a:rel:/b:"), list(&["/a", "/b"])),
        ];
        for (value, want) in lists {
            assert_eq!(dirs(value), want, "{value:?}");
        }
    }
}
