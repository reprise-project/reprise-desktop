//! The first-login flag, `welcome_pending` in the state file: the compositor sets it at a new
//! user's first start, and Welcome's autostart run shows only while it is true, then clears it.

use std::path::Path;
use toml_edit::{DocumentMut, Item, value};

const KEY: &str = "welcome_pending";

/// Set the flag when neither `config` nor any state exists yet, logging which rule decided. Runs
/// at the compositor's start before it writes state, so an upgrading user is never flagged.
pub fn mark_first_login(config: &Path, state: &Path) -> bool {
    if config.exists() {
        log::info!("not a first login, {} exists", config.display());
        return false;
    }
    let entries = match std::fs::read_to_string(state) {
        Ok(text) => text.parse::<DocumentMut>().map_or(1, |d| d.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => 1,
    };
    if entries > 0 {
        log::info!("not a first login, {} already has entries", state.display());
        return false;
    }
    match set(state, true) {
        Ok(()) => {
            log::info!("first login (no config.toml, no state), Welcome runs from autostart");
            true
        }
        Err(e) => {
            log::warn!("could not write {}: {e}", state.display());
            false
        }
    }
}

/// Whether Welcome still has to show at this login.
pub fn pending(state: &Path) -> bool {
    std::fs::read_to_string(state)
        .ok()
        .and_then(|t| t.parse::<DocumentMut>().ok())
        .and_then(|d| d.get(KEY).and_then(Item::as_bool))
        .unwrap_or(false)
}

/// Write the flag into `state`, leaving every other key as it is.
pub fn set(state: &Path, on: bool) -> Result<(), String> {
    let written = crate::edit_or_reset(state, |text| {
        let mut doc = text.parse::<DocumentMut>().unwrap_or_default();
        doc[KEY] = value(on);
        Ok::<_, String>(doc.to_string())
    });
    match written {
        Ok(r) => r.map(|_| ()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("repriseconf-welcome-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_user_is_flagged_and_welcome_clears_it() {
        let d = dir("new");
        let (config, state) = (d.join("config.toml"), d.join("reprise/state.toml"));
        assert!(mark_first_login(&config, &state));
        assert!(pending(&state));
        set(&state, false).unwrap();
        assert!(!pending(&state));
        assert!(
            !mark_first_login(&config, &state),
            "the cleared flag is an entry, so the next login is not a first one"
        );
    }

    #[test]
    fn an_existing_config_or_state_is_not_a_first_login() {
        let d = dir("old");
        let (config, state) = (d.join("config.toml"), d.join("state.toml"));
        std::fs::write(&config, "").unwrap();
        assert!(!mark_first_login(&config, &state));
        assert!(!pending(&state));
        assert!(!state.exists(), "nothing is written for an existing user");

        std::fs::remove_file(&config).unwrap();
        std::fs::write(&state, "auto_arrange = true\n").unwrap();
        assert!(!mark_first_login(&config, &state));
        assert!(!pending(&state));
    }

    #[test]
    fn setting_the_flag_keeps_the_other_keys() {
        let d = dir("keep");
        let state = d.join("state.toml");
        std::fs::write(&state, "run_history = [\"foot\"]\n").unwrap();
        set(&state, true).unwrap();
        let text = std::fs::read_to_string(&state).unwrap();
        assert!(text.contains("run_history = [\"foot\"]"), "{text}");
        assert!(pending(&state));
    }
}
