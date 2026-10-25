//! The last choice per app (`$XDG_STATE_HOME/reprise-portal/lastpick.toml`), so the picker starts on it; a preselection is
//! never an answer. Unsandboxed clients send an empty app id and share the "" slot.

use crate::wayland::Target;
use reprisebus::mutex::lock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("reprise-portal").join("lastpick.toml")
}

fn read(p: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn encode(t: &Target) -> String {
    match t {
        Target::Output(n) => format!("output:{n}"),
        Target::Toplevel(i) => format!("toplevel:{i}"),
    }
}

pub fn decode(s: &str) -> Option<Target> {
    if let Some(n) = s.strip_prefix("output:") {
        return Some(Target::Output(n.to_string()));
    }
    s.strip_prefix("toplevel:")
        .map(|i| Target::Toplevel(i.to_string()))
}

pub fn load(app_id: &str) -> Option<Target> {
    read(&path()).get(app_id).and_then(|s| decode(s))
}

pub fn save(app_id: &str, t: &Target) {
    save_to(&path(), app_id, t);
}

/// Serializes the read-modify-write, so two Starts finishing together each
/// keep their pick.
static SAVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn save_to(p: &Path, app_id: &str, t: &Target) {
    let _held = lock(&SAVE);
    let mut map = read(p);
    map.insert(app_id.to_string(), encode(t));
    let text = match toml::to_string(&map) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("lastpick: {e}");
            return;
        }
    };
    if let Err(e) = reprisefs::ops::write_atomic(p, text.as_bytes()) {
        log::warn!("lastpick: {}: {e}", p.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_round_trip() {
        for t in [
            Target::Output("Virtual-1".into()),
            Target::Toplevel("abc:1".into()),
        ] {
            assert_eq!(decode(&encode(&t)), Some(t));
        }
        assert_eq!(decode("junk"), None);
    }

    #[test]
    fn concurrent_saves_keep_every_app() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("reprise-portal").join("lastpick.toml");
        std::thread::scope(|s| {
            for i in 0..16 {
                let p = &p;
                s.spawn(move || save_to(p, &format!("app{i}"), &Target::Output(format!("O-{i}"))));
            }
        });
        let map = read(&p);
        for i in 0..16 {
            assert_eq!(
                map.get(&format!("app{i}")),
                Some(&format!("output:O-{i}")),
                "app{i} is kept"
            );
        }
    }
}
