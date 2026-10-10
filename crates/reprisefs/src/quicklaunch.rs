//! The Quick Launch folder, `$XDG_CONFIG_HOME/reprise/quicklaunch/`: one `.desktop` file per taskbar button, and [`ORDER`], their order. A file the order does not list comes after the listed ones, by name.

use std::path::{Path, PathBuf};

/// The order file's name; hidden, and no `.desktop`, so no scan takes it for a button.
pub const ORDER: &str = ".order";

pub const ACTION_KEY: &str = "X-Reprise-Action";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Button {
    App(crate::apps::AppEntry),
    Action {
        name: String,
        icon: Option<String>,
        action: String,
    },
}

/// Whether the `.desktop` file `text` (named `id`) is a button, and which: a shown program's entry, else a `Name=` with an [`ACTION_KEY`] naming an action `is_action` knows. The compositor and reprise-settings both go by this.
pub fn button(text: &str, id: &str, is_action: impl Fn(&str) -> bool) -> Option<Button> {
    if let Some(app) = crate::apps::parse_desktop(text, id) {
        return Some(Button::App(app));
    }
    let kv = crate::apps::desktop_entry_kv(text);
    let action = kv.get(ACTION_KEY).filter(|a| is_action(a))?;
    Some(Button::Action {
        name: kv.get("Name")?.to_string(),
        icon: kv
            .get("Icon")
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string()),
        action: action.to_string(),
    })
}

pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("reprise").join("quicklaunch"))
}

pub fn files(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter_map(|d| d.file_name().to_str().map(str::to_string))
        .filter(|f| f.ends_with(".desktop"))
        .collect();
    out.sort();
    out
}

pub fn read_order(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(ORDER)).ok()?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// Write the order file atomically, so a reader never sees half of it.
pub fn write_order(dir: &Path, files: &[String]) -> std::io::Result<()> {
    let mut text = files.join("\n");
    text.push('\n');
    crate::ops::write_atomic(&dir.join(ORDER), text.as_bytes())
}

/// Put `items` in the order `files` lists them: listed ones first, then the rest as they were. Names no item has are skipped.
pub fn apply_order<T>(items: &mut Vec<T>, files: &[String], file: impl Fn(&T) -> &str) {
    let mut rest = std::mem::take(items);
    for f in files {
        if let Some(i) = rest.iter().position(|e| file(e) == f) {
            items.push(rest.remove(i));
        }
    }
    items.append(&mut rest);
}

pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for ch in label.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "shortcut".into()
    } else {
        out
    }
}

pub fn unique_name(dir: &Path, name: &str) -> String {
    crate::names::first_free(name.as_ref(), |c| dir.join(c).exists())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_is_a_program_or_a_known_action() {
        let known = |a: &str| a == "showdesktop";
        let action = |a: &str| {
            format!("[Desktop Entry]\nType=Application\nName=Go\nIcon=\nX-Reprise-Action={a}\n")
        };
        assert_eq!(
            button(&action("showdesktop"), "a.desktop", known),
            Some(Button::Action {
                name: "Go".into(),
                icon: None,
                action: "showdesktop".into()
            })
        );
        assert_eq!(button(&action("bogus"), "a.desktop", known), None);
        let app = "[Desktop Entry]\nType=Application\nName=Ed\nExec=ed\n";
        assert!(matches!(button(app, "ed.desktop", known), Some(Button::App(a)) if a.name == "Ed"));
        assert_eq!(button("[Desktop Entry]\n", "x.desktop", known), None);
    }

    #[test]
    fn slug_is_file_safe() {
        assert_eq!(slug("Launch Web Browser"), "launch-web-browser");
        assert_eq!(slug("  A/B (c) "), "a-b-c");
        assert_eq!(slug("///"), "shortcut");
    }

    #[test]
    fn listed_files_come_first_in_their_order_and_the_rest_keep_theirs() {
        let mut v = vec!["a", "b", "c", "d"];
        let order: Vec<String> = ["c", "gone", "a"].iter().map(|s| s.to_string()).collect();
        apply_order(&mut v, &order, |s| s);
        assert_eq!(v, ["c", "a", "b", "d"]);
    }

    #[test]
    fn the_order_file_round_trips_and_is_no_button() {
        let dir = std::env::temp_dir().join(format!("reprisefs-quick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_order(&dir), None);
        let order = vec!["b.desktop".to_string(), "a.desktop".to_string()];
        write_order(&dir, &order).unwrap();
        assert_eq!(read_order(&dir), Some(order));
        std::fs::write(dir.join("a.desktop"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        assert_eq!(files(&dir), ["a.desktop"]);
        assert_eq!(unique_name(&dir, "a.desktop"), "a (2).desktop");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
