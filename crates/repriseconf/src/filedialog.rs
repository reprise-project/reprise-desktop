//! The view and sort every Open and Save As dialog opens in, in
//! `$XDG_STATE_HOME/reprise/filedialog.toml`. The text is
//! `reprisekit::controls::filedlg::Remembered`'s; the last dialog to write wins.

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

pub fn save_to(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::write_atomic(path, text)
}
