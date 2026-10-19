//! What every Open and Save As dialog opens with, in `$XDG_STATE_HOME/reprise/filedialog.toml`:
//! the view and sort (`reprisekit::controls::filedlg::Remembered`'s keys, written by whichever
//! dialog changed them) and the [`Geometry`] (written by the compositor).

use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    crate::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("filedialog.toml")
}

/// The stored text; a missing or unreadable file reads as empty.
pub fn load_from(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Writes `text`'s `key = value` lines over the file's lines of the same keys, keeping the rest,
/// so the view and the geometry writers leave each other's keys alone, under [`crate::edit`]'s
/// lock so that a program and the compositor writing at once both keep theirs.
pub fn save_to(path: &Path, text: &str) -> std::io::Result<()> {
    crate::edit(path, |old| {
        Ok::<_, std::convert::Infallible>(merged(old, text))
    })
    .map(|_| ())
}

fn key(line: &str) -> Option<&str> {
    line.split_once('=').map(|(k, _)| k.trim())
}

fn merged(old: &str, new: &str) -> String {
    let mut out = String::new();
    let mut written = Vec::new();
    for line in old.lines() {
        let line = match key(line) {
            Some(k) => match new.lines().find(|n| key(n) == Some(k)) {
                Some(n) => {
                    written.push(k);
                    n
                }
                None => line,
            },
            None => line,
        };
        out.push_str(line);
        out.push('\n');
    }
    for line in new.lines() {
        if key(line).is_some_and(|k| !written.contains(&k)) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Where the last file dialog was left: its frame's top-left in layout pixels and its client
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
    fn a_save_replaces_its_own_keys_and_keeps_the_others() {
        let dir = std::env::temp_dir().join(format!("repriseconf-filedlg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("state").join("filedialog.toml");
        save_to(
            &file,
            "view = \"list\"\nsort = \"name\"\nascending = true\n",
        )
        .unwrap();
        save_to(&file, "x = 1\ny = 2\nwidth = 600\nheight = 400\n").unwrap();
        save_to(
            &file,
            "view = \"details\"\nsort = \"size\"\nascending = false\n",
        )
        .unwrap();
        save_to(&file, "x = 5\ny = 6\nwidth = 700\nheight = 500\n").unwrap();
        assert_eq!(
            load_from(&file),
            "view = \"details\"\nsort = \"size\"\nascending = false\n\
             x = 5\ny = 6\nwidth = 700\nheight = 500\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_writers_at_once_both_keep_their_keys() {
        let dir =
            std::env::temp_dir().join(format!("repriseconf-filedlg-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("filedialog.toml");
        let rounds = 200;
        let go = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            s.spawn(|| {
                go.wait();
                for i in 0..rounds {
                    save_to(&file, &format!("view = \"v{i}\"\n")).unwrap();
                }
            });
            s.spawn(|| {
                go.wait();
                for i in 0..rounds {
                    save_to(&file, &format!("width = {i}\n")).unwrap();
                }
            });
        });
        let text = load_from(&file);
        let last = rounds - 1;
        assert!(text.contains(&format!("view = \"v{last}\"\n")), "{text}");
        assert!(text.contains(&format!("width = {last}\n")), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
