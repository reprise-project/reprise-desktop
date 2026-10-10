//! The location answers a user asked Reprise to remember: one entry per
//! program, by desktop id, true for Allow and false for Deny. The
//! compositor's location prompt writes them and reads them each time a
//! program asks; reprise-settings lists them and forgets one on request.
//!
//! They live in `$XDG_STATE_HOME/reprise/location.toml`, a table of their
//! own:
//!
//! ```toml
//! [apps]
//! "org.gnome.Maps" = true
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

pub fn path() -> PathBuf {
    crate::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("location.toml")
}

/// The remembered answers, by desktop id. A missing or broken file, or an
/// entry that is not a boolean, reads as nothing remembered.
pub fn load() -> BTreeMap<String, bool> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> BTreeMap<String, bool> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse(&text)
}

fn parse(text: &str) -> BTreeMap<String, bool> {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return BTreeMap::new();
    };
    doc.get("apps")
        .and_then(Item::as_table_like)
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| Some((k.to_string(), v.as_bool()?)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn remember(desktop_id: &str, allow: bool) -> std::io::Result<()> {
    remember_in(&path(), desktop_id, allow)
}

pub fn remember_in(path: &Path, desktop_id: &str, allow: bool) -> std::io::Result<()> {
    change(path, |apps| {
        apps.insert(desktop_id, value(allow));
    })
}

pub fn forget(desktop_id: &str) -> std::io::Result<()> {
    forget_in(&path(), desktop_id)
}

pub fn forget_in(path: &Path, desktop_id: &str) -> std::io::Result<()> {
    change(path, |apps| {
        apps.remove(desktop_id);
    })
}

/// Edit the `[apps]` table under the file's lock (`crate::edit`, which
/// makes the directory); a file that does not parse starts over empty.
fn change(path: &Path, f: impl FnOnce(&mut Table)) -> std::io::Result<()> {
    let edited = crate::edit(path, |text| {
        let mut doc = text.parse::<DocumentMut>().unwrap_or_default();
        if !doc.get("apps").is_some_and(Item::is_table) {
            doc.insert("apps", Item::Table(Table::new()));
        }
        if let Some(apps) = doc.get_mut("apps").and_then(Item::as_table_mut) {
            f(apps);
        }
        Ok::<_, std::convert::Infallible>(doc.to_string())
    })?;
    let Ok(_) = edited;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_remembered_replaced_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("repriseconf-location-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("reprise").join("location.toml");
        assert!(load_from(&p).is_empty(), "no file remembers nothing");
        remember_in(&p, "org.gnome.Maps", true).unwrap();
        remember_in(&p, "org.example.Weather", false).unwrap();
        assert_eq!(
            load_from(&p),
            BTreeMap::from([
                ("org.example.Weather".to_string(), false),
                ("org.gnome.Maps".to_string(), true),
            ])
        );
        remember_in(&p, "org.gnome.Maps", false).unwrap();
        forget_in(&p, "org.example.Weather").unwrap();
        let left = load_from(&p);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            left,
            BTreeMap::from([("org.gnome.Maps".to_string(), false)])
        );
    }

    #[test]
    fn a_broken_file_or_entry_reads_as_nothing_remembered() {
        assert!(parse("not toml [").is_empty());
        assert_eq!(
            parse("[apps]\n\"a\" = true\n\"b\" = \"yes\"\n"),
            BTreeMap::from([("a".to_string(), true)])
        );
    }
}
