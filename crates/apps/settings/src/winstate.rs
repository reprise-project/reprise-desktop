//! The settings window's client size, kept in `$XDG_STATE_HOME/reprise/settings.toml`.

use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, value};

/// The client size the sheet opens at before it was ever resized.
pub const DEFAULT: (i32, i32) = (760, 680);

pub fn path() -> PathBuf {
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("settings.toml")
}

/// The window size the file at `path` keeps, no smaller than `min`; [`DEFAULT`] when there is none.
pub fn load(path: &Path, min: (i32, i32)) -> (i32, i32) {
    let size = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .and_then(|doc| {
            let a = doc.get("window")?.as_array()?;
            let side = |i: usize| i32::try_from(a.get(i)?.as_integer()?).ok();
            Some((side(0)?, side(1)?))
        })
        .unwrap_or(DEFAULT);
    (size.0.max(min.0), size.1.max(min.1))
}

/// Write `size` as the window size into the file at `path`, keeping whatever else it holds; a
/// failure is logged, not fatal.
pub fn save(path: &Path, size: (i32, i32)) {
    let mut doc = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .unwrap_or_default();
    let mut a = Array::new();
    a.push(i64::from(size.0));
    a.push(i64::from(size.1));
    doc["window"] = value(a);
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
        assert_eq!(load(&path, min), DEFAULT, "no file yet");
        save(&path, (900, 700));
        assert_eq!(load(&path, min), (900, 700));
        save(&path, (100, 100));
        assert_eq!(load(&path, min), min, "never under the smallest");
        std::fs::write(&path, "window = \"big\"\n").unwrap();
        assert_eq!(load(&path, min), DEFAULT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
