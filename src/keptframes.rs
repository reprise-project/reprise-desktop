//! Each app's last normal frame per set of screens (`outputset`), kept in
//! `$XDG_STATE_HOME/reprise/windows.toml`: a table per set, a table per app id inside it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::windowrules::Remembered;

/// The most apps kept per set of screens; the least recently stored go first.
pub const APPS_MAX: usize = 200;
/// The most sets of screens kept; the one stored to least recently goes first.
pub const SETS_MAX: usize = 16;

/// A kept frame and when it was stored, counted up across the file.
type Stamped = (Remembered, u64);

/// Read at start and after Settings imported the file (`State::reload_remembered`): the
/// compositor is its only other writer.
pub struct KeptFrames {
    pub path: PathBuf,
    sets: BTreeMap<String, BTreeMap<String, Stamped>>,
    clock: u64,
}

pub fn path() -> PathBuf {
    crate::state::path().with_file_name("windows.toml")
}

impl KeptFrames {
    pub fn empty(path: PathBuf) -> KeptFrames {
        KeptFrames {
            path,
            sets: BTreeMap::new(),
            clock: 0,
        }
    }

    pub fn load() -> KeptFrames {
        let path = path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        KeptFrames::parse(path, &text)
    }

    /// What `text` keeps; an entry that does not read is dropped, the rest stay.
    pub fn parse(path: PathBuf, text: &str) -> KeptFrames {
        let mut k = KeptFrames::empty(path);
        let table: toml::Table = match toml::from_str(text) {
            Ok(t) => t,
            Err(e) => {
                if !text.is_empty() {
                    log::warn!("window frames {}: {e}", k.path.display());
                }
                return k;
            }
        };
        for (set, apps) in table {
            let toml::Value::Table(apps) = apps else {
                continue;
            };
            let apps: BTreeMap<String, Stamped> = (apps.into_iter())
                .filter_map(|(app, v)| {
                    let toml::Value::Table(mut t) = v else {
                        return None;
                    };
                    let used = t.remove("used").and_then(|u| u.as_integer()).unwrap_or(0);
                    let frame: Remembered = t.try_into().ok()?;
                    Some((app, (frame, u64::try_from(used).unwrap_or(0))))
                })
                .collect();
            let newest = apps.values().map(|(_, u)| *u).max().unwrap_or(0);
            k.clock = k.clock.max(newest);
            if !apps.is_empty() {
                k.sets.insert(set, apps);
            }
        }
        k
    }

    pub fn get(&self, set: &str, app: &str) -> Option<&Remembered> {
        self.sets.get(set)?.get(app).map(|(f, _)| f)
    }

    /// Keeps `frame` for `app` under `set`, as the most recent; whether the file needs writing.
    pub fn store(&mut self, set: &str, app: &str, frame: Remembered) -> bool {
        let newest = self.newest_in(set);
        if let Some((f, u)) = self.sets.get(set).and_then(|a| a.get(app))
            && *f == frame
            && Some(*u) == newest
        {
            return false;
        }
        self.clock += 1;
        let apps = self.sets.entry(set.to_string()).or_default();
        apps.insert(app.to_string(), (frame, self.clock));
        if apps.len() > APPS_MAX
            && let Some(oldest) = oldest(apps.iter().map(|(k, (_, u))| (k, *u)))
        {
            apps.remove(&oldest);
        }
        if self.sets.len() > SETS_MAX {
            let newest = |apps: &BTreeMap<String, Stamped>| {
                apps.values().map(|(_, u)| *u).max().unwrap_or(0)
            };
            if let Some(oldest) = oldest(self.sets.iter().map(|(k, a)| (k, newest(a)))) {
                self.sets.remove(&oldest);
            }
        }
        true
    }

    fn newest_in(&self, set: &str) -> Option<u64> {
        self.sets.get(set)?.values().map(|(_, u)| *u).max()
    }

    /// Drops every kept frame.
    pub fn clear(&mut self) {
        self.sets.clear();
    }

    pub fn text(&self) -> String {
        let mut table = toml::Table::new();
        for (set, apps) in &self.sets {
            let mut inner = toml::Table::new();
            for (app, (frame, used)) in apps {
                let Ok(mut t) = toml::Table::try_from(frame) else {
                    continue;
                };
                t.insert(
                    "used".into(),
                    i64::try_from(*used).unwrap_or(i64::MAX).into(),
                );
                inner.insert(app.clone(), t.into());
            }
            table.insert(set.clone(), inner.into());
        }
        toml::to_string(&table).unwrap_or_default()
    }

    /// Writes the file on the state writer thread.
    pub fn save(&self) {
        let (path, text) = (self.path.clone(), self.text());
        crate::state::run_in_order(move || {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = repriseconf::write_atomic(&path, &text) {
                log::warn!("window frames: could not write {}: {e}", path.display());
            }
        });
    }
}

fn oldest<'a>(stamps: impl Iterator<Item = (&'a String, u64)>) -> Option<String> {
    stamps.min_by_key(|(_, u)| *u).map(|(k, _)| k.clone())
}

#[cfg(test)]
mod tests;
