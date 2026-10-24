use super::*;

/// The `Default` line of an lpoptions file: the user's own default,
/// which programs take over the server's.
pub fn user_default(lpoptions: &str) -> Option<String> {
    lpoptions.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Default ")?;
        let dest = rest.split_whitespace().next()?;
        Some(dest.split('/').next()?.to_string())
    })
}

/// The folder of the system's lpoptions, which the user's own overrides.
const SYSTEM_LPOPTIONS: &str = "/etc/cups";

fn lpoptions_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".cups/lpoptions"))
}

/// The folders whose lpoptions name a default: the user's, then the system's.
pub fn lpoptions_folders() -> Vec<std::path::PathBuf> {
    let user = lpoptions_path().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    user.into_iter()
        .chain([std::path::PathBuf::from(SYSTEM_LPOPTIONS)])
        .collect()
}

pub(super) fn own_default() -> Option<String> {
    own_default_in(&lpoptions_text(), &system_lpoptions_text())
}

pub(super) fn system_lpoptions_text() -> String {
    std::fs::read_to_string(std::path::Path::new(SYSTEM_LPOPTIONS).join("lpoptions"))
        .unwrap_or_default()
}

/// The default an lpoptions file names, the user's before the system's.
pub(super) fn own_default_in(user: &str, system: &str) -> Option<String> {
    user_default(user).or_else(|| user_default(system))
}

pub(super) fn lpoptions_text() -> String {
    lpoptions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
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
        return Err(Error::Refused(
            "There is no home folder to keep the setting in.".into(),
        ));
    };
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("reprise-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| Error::Refused(format!("Could not save the default printer: {e}.")))
}
