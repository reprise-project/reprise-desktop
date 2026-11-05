//! What the settings window keeps between runs in `$XDG_STATE_HOME/reprise/settings.toml`: its
//! client size with the text size it was left at, the slideshow folder last picked and the Color
//! dialog's custom colours.

use std::path::{Path, PathBuf};

use reprisekit::controls::colordlg::{self, CUSTOM};
use reprisekit::theme::Color;
use toml_edit::{Array, DocumentMut, value};

/// The client size the sheet opens at before it was ever resized.
pub const DEFAULT: (i32, i32) = (760, 680);

pub fn path() -> PathBuf {
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("settings.toml")
}

/// A window size the state file keeps and the text size it was left at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept {
    pub size: (i32, i32),
    pub text_size: Option<u32>,
}

impl Kept {
    /// The size the window opens at `text_size`, no smaller than `min`. A size left at a larger
    /// text size shrinks by the ratio of the two, down to no less than [`DEFAULT`].
    pub fn at(self, text_size: u32, min: (i32, i32)) -> (i32, i32) {
        let size = match self.text_size.filter(|&pct| pct > text_size) {
            Some(pct) => {
                let shrink = |v: i32, natural: i32| {
                    let k = (i64::from(v) * i64::from(text_size) / i64::from(pct)) as i32;
                    v.min(k.max(natural))
                };
                (
                    shrink(self.size.0, DEFAULT.0),
                    shrink(self.size.1, DEFAULT.1),
                )
            }
            None => self.size,
        };
        (size.0.max(min.0), size.1.max(min.1))
    }
}

/// The window size the file at `path` keeps; [`DEFAULT`] when there is none.
pub fn kept(path: &Path) -> Kept {
    let doc = read(path);
    let size = doc
        .as_ref()
        .and_then(|doc| {
            let a = doc.get("window")?.as_array()?;
            let side = |i: usize| i32::try_from(a.get(i)?.as_integer()?).ok();
            Some((side(0)?, side(1)?))
        })
        .unwrap_or(DEFAULT);
    let text_size = doc
        .as_ref()
        .and_then(|doc| u32::try_from(doc.get("text_size")?.as_integer()?).ok());
    Kept { size, text_size }
}

/// The window size the file at `path` keeps, opened at `text_size` ([`Kept::at`]).
#[cfg(test)]
pub fn load(path: &Path, min: (i32, i32), text_size: u32) -> (i32, i32) {
    kept(path).at(text_size, min)
}

/// Write `size`, left at `text_size` percent, as the window size into the file at `path`.
pub fn save(path: &Path, size: (i32, i32), text_size: u32) {
    let mut a = Array::new();
    a.push(i64::from(size.0));
    a.push(i64::from(size.1));
    set(path, "window", value(a));
    set(path, "text_size", value(i64::from(text_size)));
}

/// The slideshow folder kept at `path`, as the config spells it; empty for none.
pub fn slideshow(path: &Path) -> String {
    read(path)
        .and_then(|doc| Some(doc.get("slideshow")?.as_str()?.to_string()))
        .unwrap_or_default()
}

pub fn save_slideshow(path: &Path, folder: &str) {
    set(path, "slideshow", value(folder));
}

fn read(path: &Path) -> Option<DocumentMut> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// The custom colours kept at `path` as `custom_colors = ["#rrggbb", ...]`: white where it keeps
/// none or one does not read.
pub fn custom_colors(path: &Path) -> [Color; CUSTOM] {
    let mut out = [Color::rgb(0xFF, 0xFF, 0xFF); CUSTOM];
    let kept = read(path).and_then(|d| d.get("custom_colors")?.as_array().cloned());
    for (slot, v) in out.iter_mut().zip(kept.iter().flatten()) {
        if let Some(c) = v.as_str().and_then(Color::parse) {
            *slot = c;
        }
    }
    out
}

pub fn save_custom_colors(path: &Path, colors: &[Color; CUSTOM]) {
    let a: Array = colors.iter().map(|&c| colordlg::hex(c)).collect();
    set(path, "custom_colors", value(a));
}

/// Write `key` into the file at `path`, keeping whatever else it holds; a failure is logged, not
/// fatal.
fn set(path: &Path, key: &str, item: toml_edit::Item) {
    let mut doc = read(path).unwrap_or_default();
    doc[key] = item;
    if let Err(e) = repriseconf::write_atomic(path, &doc.to_string()) {
        log::warn!("settings state: could not write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_size_round_trips_and_a_bad_file_gives_the_default() {
        let dir = std::env::temp_dir().join(format!("reprise-winstate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("reprise").join("settings.toml");
        let min = (544, 528);
        assert_eq!(load(&path, min, 100), DEFAULT, "no file yet");
        save(&path, (900, 700), 100);
        assert_eq!(load(&path, min, 100), (900, 700));
        save(&path, (100, 100), 100);
        assert_eq!(load(&path, min, 100), min, "never under the smallest");
        std::fs::write(&path, "window = \"big\"\n").unwrap();
        assert_eq!(load(&path, min, 100), DEFAULT);
        let mut colors = [Color::rgb(0xFF, 0xFF, 0xFF); CUSTOM];
        colors[2] = Color::rgb(0x12, 0x34, 0x56);
        save_custom_colors(&path, &colors);
        save(&path, (900, 700), 100);
        assert_eq!(custom_colors(&path), colors, "kept beside the window size");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_slideshow_folder_round_trips_beside_the_window_size() {
        let dir =
            std::env::temp_dir().join(format!("reprise-winstate-slides-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.toml");
        assert_eq!(slideshow(&path), "", "no file yet");
        save(&path, (900, 700), 100);
        save_slideshow(&path, "~/Pictures/Trips");
        assert_eq!(slideshow(&path), "~/Pictures/Trips");
        assert_eq!(load(&path, (544, 528), 100), (900, 700), "the size stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_size_left_at_a_larger_text_size_shrinks_back() {
        let dir =
            std::env::temp_dir().join(format!("reprise-winstate-text-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.toml");
        let min = (544, 528);
        save(&path, (1088, 1056), 200);
        assert_eq!(
            load(&path, min, 100),
            DEFAULT,
            "200% to 100%: the opening size"
        );
        assert_eq!(
            load(&path, (816, 792), 150),
            (816, 792),
            "150%: its smallest"
        );
        assert_eq!(
            load(&path, (1088, 1056), 200),
            (1088, 1056),
            "the same size"
        );
        save(&path, (1800, 1000), 200);
        assert_eq!(
            load(&path, min, 100),
            (900, 680),
            "halved, no less than the opening size"
        );
        save(&path, (900, 700), 100);
        assert_eq!(
            load(&path, (1088, 1056), 200),
            (1088, 1056),
            "a smaller one grows to the smallest"
        );
        std::fs::write(&path, "window = [1000, 900]\n").unwrap();
        assert_eq!(
            load(&path, min, 100),
            (1000, 900),
            "no text size kept: as it is"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
