use std::path::Path;

use repriselogin::machine::MachineConfig;

/// Reads the machine settings at `path`, logging each warning. A missing file gives the defaults.
pub fn load(path: &Path) -> MachineConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MachineConfig::default(),
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            return MachineConfig::default();
        }
    };
    let (config, warnings) = MachineConfig::parse(&text);
    for w in warnings {
        log::warn!("{}: {w}", path.display());
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("login.toml"));
        assert_eq!(config, MachineConfig::default());
        assert!(config.login.show_users);
        assert!(
            !config.login.greeter_socket,
            "the login screen has no socket"
        );
        assert!(!config.remote.enabled, "remote logins ship off");
    }
}
