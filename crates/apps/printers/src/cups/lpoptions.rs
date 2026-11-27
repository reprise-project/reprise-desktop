use super::*;

use repriseapp::print::cups::{SYSTEM_LPOPTIONS, lpoptions_path};
pub(super) use repriseapp::print::cups::{
    lpoptions_text, own_default, own_default_in, system_lpoptions_text,
};

/// The folders whose lpoptions name a default: the user's, then the system's.
pub fn lpoptions_folders() -> Vec<std::path::PathBuf> {
    let user = lpoptions_path().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    user.into_iter()
        .chain([std::path::PathBuf::from(SYSTEM_LPOPTIONS)])
        .collect()
}

/// `text` with its `Default` line naming `name`. Other lines, other
/// destinations' options included, stay as they are.
pub fn with_user_default(text: &str, name: &str) -> String {
    let mut out = String::new();
    for l in text.lines() {
        match l.trim().strip_prefix("Default ") {
            // The options after the destination are its own; they move
            // to a Dest line so they stay with it.
            Some(rest) => {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let dest = parts.next().unwrap_or_default();
                if let Some(opts) = parts.next().map(str::trim).filter(|o| !o.is_empty()) {
                    out.push_str(&format!("Dest {dest} {opts}\n"));
                }
            }
            None => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    format!("Default {name}\n{out}")
}

pub(super) fn write_lpoptions(text: &str) -> Result<()> {
    let Some(path) = lpoptions_path() else {
        return Err(Error::Refused(tr!("cups-no-home")));
    };
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("reprise-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| Error::Refused(tr!("cups-default-not-saved", why = e.to_string())))
}
