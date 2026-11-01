//! What each app's Open and Save As dialogs open with, in `$XDG_STATE_HOME/reprise/filedialog.toml`:
//! a table per app id (`""` for dialogs whose app is unknown) holding the last folder a dialog
//! was confirmed in, the view and sort (`reprisekit::controls::filedlg::Remembered`'s keys) and the
//! [`Geometry`] (written by the compositor), and `used`, when it was last written.

use std::path::{Path, PathBuf};

use toml::{Table, Value};

pub fn path() -> PathBuf {
    crate::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("filedialog.toml")
}

/// The most apps the file keeps; writing another drops the one written longest ago.
pub const MOST_APPS: usize = 32;

/// The stored text; a missing or unreadable file reads as empty.
pub fn load_from(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// `text`'s app tables; text that is not TOML, and keys outside a table, read as none.
fn tables(text: &str) -> Table {
    let mut t: Table = toml::from_str(text).unwrap_or_default();
    t.retain(|_, v| v.is_table());
    t
}

/// `app`'s table in `text` as `key = value` lines; empty when it has none.
pub fn section(text: &str, app: &str) -> String {
    match tables(text).get(app) {
        Some(Value::Table(t)) => t.to_string(),
        _ => String::new(),
    }
}

/// Every app's table in `text`, as [`section`] gives it.
pub fn sections(text: &str) -> Vec<(String, String)> {
    tables(text)
        .into_iter()
        .filter_map(|(app, v)| match v {
            Value::Table(t) => Some((app, t.to_string())),
            _ => None,
        })
        .collect()
}

/// The `folder` line for `dir`; None for a path that is not UTF-8, which TOML cannot hold.
pub fn folder_text(dir: &Path) -> Option<String> {
    let mut t = Table::new();
    t.insert("folder".into(), Value::String(dir.to_str()?.into()));
    Some(t.to_string())
}

/// The `folder` in [`section`]'s lines.
pub fn folder(section: &str) -> Option<PathBuf> {
    let t: Table = toml::from_str(section).ok()?;
    t.get("folder")?.as_str().map(PathBuf::from)
}

/// Writes `lines`' keys over `app`'s table, keeping its other keys and every other app's, under
/// [`crate::edit`]'s lock so that a program and the compositor writing at once both keep theirs.
/// A file that is not UTF-8 is replaced.
pub fn save_to(path: &Path, app: &str, lines: &str) -> std::io::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    crate::edit_or_reset(path, |old| {
        Ok::<_, std::convert::Infallible>(merged(old, app, lines, now))
    })
    .map(|_| ())
}

fn merged(old: &str, app: &str, lines: &str, now: u64) -> String {
    let mut all = tables(old);
    let new: Table = toml::from_str(lines).unwrap_or_default();
    let entry = all.entry(app).or_insert_with(|| Value::Table(Table::new()));
    if let Value::Table(t) = entry {
        t.extend(new);
        t.insert("used".into(), Value::Integer(now as i64));
    }
    let used = |v: &Value| v.get("used").and_then(Value::as_integer).unwrap_or(0);
    while all.len() > MOST_APPS {
        let Some(oldest) = (all.iter())
            .filter(|(k, _)| k.as_str() != app)
            .min_by_key(|(_, v)| used(v))
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        all.remove(&oldest);
    }
    all.to_string()
}

/// Where an app's last file dialog was left: its frame's top-left in layout pixels and its client
/// size at 100% text size, the restored size when it was maximized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Geometry {
    /// A dialog at `at` with client `size` at `text_size` percent.
    pub fn kept(at: (i32, i32), size: (i32, i32), text_size: u32) -> Geometry {
        let ts = i64::from(text_size.max(1));
        let unscale = |v: i32| ((i64::from(v) * 100 + ts / 2) / ts) as i32;
        Geometry {
            x: at.0,
            y: at.1,
            width: unscale(size.0),
            height: unscale(size.1),
        }
    }

    /// The client size at `text_size` percent.
    pub fn size_at(&self, text_size: u32) -> (i32, i32) {
        let ts = i64::from(text_size);
        let scale = |v: i32| ((i64::from(v) * ts + 50) / 100) as i32;
        (scale(self.width), scale(self.height))
    }

    /// `x`, `y`, `width` and `height` as TOML lines.
    pub fn text(&self) -> String {
        format!(
            "x = {}\ny = {}\nwidth = {}\nheight = {}\n",
            self.x, self.y, self.width, self.height
        )
    }

    /// Reads [`Geometry::text`]'s lines; None unless all four are there and the size is positive.
    pub fn parse(text: &str) -> Option<Geometry> {
        let value = |name: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == name)
                .and_then(|(_, v)| v.trim().parse::<i32>().ok())
        };
        let g = Geometry {
            x: value("x")?,
            y: value("y")?,
            width: value("width")?,
            height: value("height")?,
        };
        (g.width > 0 && g.height > 0).then_some(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("repriseconf-fd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_save_over_a_file_that_is_not_utf8_replaces_it() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("filedialog.toml");
        std::fs::write(&path, b"view = \"list\"\n\xff\xfe\n").unwrap();
        save_to(&path, "a", "sort = \"size\"\n").unwrap();
        assert_eq!(
            section(&load_from(&path), "a").lines().next(),
            Some("sort = \"size\"")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn each_app_has_its_own_table_and_a_save_keeps_its_other_keys() {
        let text = merged("", "reprise.notepad", "view = \"details\"\n", 10);
        let text = merged(&text, "org.example.Editor", "x = 5\ny = 6\n", 11);
        let text = merged(&text, "reprise.notepad", "x = 1\ny = 2\n", 12);
        assert_eq!(
            text,
            "[\"org.example.Editor\"]\nused = 11\nx = 5\ny = 6\n\n\
             [\"reprise.notepad\"]\nused = 12\nview = \"details\"\nx = 1\ny = 2\n"
        );
        assert_eq!(
            section(&text, "reprise.notepad"),
            "used = 12\nview = \"details\"\nx = 1\ny = 2\n"
        );
        assert_eq!(section(&text, "org.example.Other"), "", "no table yet");
        assert_eq!(sections(&text).len(), 2);
    }

    #[test]
    fn the_shared_keys_of_an_older_file_are_dropped() {
        let old = "view = \"list\"\nx = 3\ny = 4\nwidth = 600\nheight = 400\n";
        assert_eq!(section(old, ""), "");
        assert_eq!(
            merged(old, "", "view = \"icons\"\n", 7),
            "[\"\"]\nused = 7\nview = \"icons\"\n"
        );
    }

    #[test]
    fn past_the_most_apps_the_one_written_longest_ago_goes() {
        let mut text = String::new();
        for i in 0..MOST_APPS as u64 {
            text = merged(&text, &format!("app{i}"), "view = \"list\"\n", 100 - i);
        }
        let text = merged(&text, "new", "view = \"list\"\n", 200);
        let apps: Vec<String> = sections(&text).into_iter().map(|(a, _)| a).collect();
        assert_eq!(apps.len(), MOST_APPS);
        assert!(apps.contains(&"new".to_string()));
        let last = format!("app{}", MOST_APPS - 1);
        assert!(!apps.contains(&last), "{last} was written longest ago");
    }

    #[test]
    fn a_folder_round_trips_through_its_line() {
        let dir = Path::new("/home/u/My \"Docs\"");
        let text = merged("", "a", &folder_text(dir).unwrap(), 1);
        assert_eq!(folder(&section(&text, "a")), Some(dir.to_path_buf()));
        assert_eq!(folder("view = \"list\"\n"), None);
        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/\xff"));
        assert_eq!(folder_text(raw), None);
    }

    #[test]
    fn geometry_text_round_trips_and_a_partial_or_bad_entry_reads_as_none() {
        let g = Geometry {
            x: -40,
            y: 120,
            width: 700,
            height: 520,
        };
        assert_eq!(Geometry::parse(&g.text()), Some(g));
        let view = "view = \"details\"\nsort = \"size\"\nascending = false\n";
        assert_eq!(Geometry::parse(&format!("{view}{}", g.text())), Some(g));
        assert_eq!(Geometry::parse(view), None, "no geometry yet");
        assert_eq!(Geometry::parse("x = 1\ny = 2\nwidth = 3\n"), None);
        assert_eq!(
            Geometry::parse("x = 1\ny = 2\nwidth = big\nheight = 4\n"),
            None
        );
        assert_eq!(
            Geometry::parse("x = 1\ny = 2\nwidth = 0\nheight = 4\n"),
            None
        );
    }

    #[test]
    fn geometry_keeps_the_size_in_100_percent_units() {
        let g = Geometry::kept((10, 20), (800, 600), 150);
        assert_eq!((g.width, g.height), (533, 400));
        assert_eq!(g.size_at(150), (800, 600));
        assert_eq!(g.size_at(100), (533, 400));
        assert_eq!(
            Geometry::kept((0, 0), (563, 419), 100).size_at(100),
            (563, 419)
        );
    }

    #[test]
    fn two_writers_at_once_both_keep_their_keys() {
        let dir = temp("race");
        let file = dir.join("filedialog.toml");
        let rounds = 200;
        let go = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            s.spawn(|| {
                go.wait();
                for i in 0..rounds {
                    save_to(&file, "a", &format!("view = \"v{i}\"\n")).unwrap();
                }
            });
            s.spawn(|| {
                go.wait();
                for i in 0..rounds {
                    save_to(&file, "a", &format!("width = {i}\n")).unwrap();
                }
            });
        });
        let text = section(&load_from(&file), "a");
        let last = rounds - 1;
        assert!(text.contains(&format!("view = \"v{last}\"\n")), "{text}");
        assert!(text.contains(&format!("width = {last}\n")), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
