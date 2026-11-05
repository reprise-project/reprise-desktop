//! What Paint keeps between runs, in `$XDG_STATE_HOME/reprise/paint.toml`: the Color dialog's
//! custom colours as `custom_colors = ["#rrggbb", ...]`.

use reprisekit::controls::colordlg::{self, CUSTOM};
use reprisekit::theme::Color;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, value};

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

pub fn path() -> PathBuf {
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("paint.toml")
}

/// The custom colours kept at `path`: white where it keeps none or one does not read.
pub fn custom_colors(path: &Path) -> [Color; CUSTOM] {
    let mut out = [WHITE; CUSTOM];
    let doc = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<DocumentMut>().ok());
    let kept = doc
        .as_ref()
        .and_then(|d| d.get("custom_colors")?.as_array().cloned());
    for (slot, v) in out.iter_mut().zip(kept.iter().flatten()) {
        if let Some(c) = v.as_str().and_then(Color::parse) {
            *slot = c;
        }
    }
    out
}

/// Keep `colors` at `path`, leaving its other keys as they are.
pub fn save_custom_colors(path: &Path, colors: &[Color; CUSTOM]) {
    let mut doc = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<DocumentMut>().ok())
        .unwrap_or_default();
    let mut a = Array::new();
    for &c in colors {
        a.push(colordlg::hex(c));
    }
    doc["custom_colors"] = value(a);
    if let Err(e) = repriseconf::write_atomic(path, &doc.to_string()) {
        log::warn!("paint state: could not write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_colours_round_trip_and_a_bad_entry_stays_white() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("paint.toml");
        assert_eq!(custom_colors(&path), [WHITE; CUSTOM], "no file yet");
        let mut colors = [WHITE; CUSTOM];
        colors[0] = Color::rgb(0x12, 0x34, 0x56);
        colors[15] = Color::rgb(0, 0, 0);
        save_custom_colors(&path, &colors);
        assert_eq!(custom_colors(&path), colors);
        std::fs::write(
            &path,
            "other = 1\ncustom_colors = [\"#ff0000\", \"red\", \"#00ff00\"]\n",
        )
        .unwrap();
        let read = custom_colors(&path);
        assert_eq!(
            &read[..4],
            &[Color::rgb(0xFF, 0, 0), WHITE, Color::rgb(0, 0xFF, 0), WHITE]
        );
        save_custom_colors(&path, &colors);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("other = 1"), "other keys are kept: {text}");
    }
}
