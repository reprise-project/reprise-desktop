//! Writes the machine settings for `reprise-login-settings`.

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use repriselogin::machine::MachineConfig;

/// The longest settings text the helper reads, in bytes.
pub const MAX_INPUT: usize = 16 * 1024;

/// Validates `input` and replaces `path` with its settings, or returns every warning and leaves
/// `path` untouched.
pub fn apply(input: &str, path: &Path) -> Result<(), Vec<String>> {
    let (config, warnings) = MachineConfig::parse(input);
    if !warnings.is_empty() {
        return Err(warnings);
    }
    write(&config.to_toml(), path).map_err(|e| vec![format!("{}: {e}", path.display())])
}

/// `warning` on one line: a TOML parse error's heading and message, without its source snippet.
pub fn one_line(warning: &str) -> String {
    let snippet = |l: &str| {
        let l = l
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start();
        l.is_empty() || l.starts_with('|')
    };
    let lines: Vec<_> = warning.lines().filter(|l| !snippet(l)).collect();
    lines.join(": ")
}

fn write(text: &str, path: &Path) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&tmp)?;
    // The mode above is masked by the umask, which pkexec passes on from the caller.
    file.set_permissions(Permissions::from_mode(0o644))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use repriselogin::machine::MachineRemote;

    use super::*;

    #[test]
    fn apply_writes_the_parsed_settings_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(&path, "[login]\nshow_users = true\n").unwrap();
        let input = "[login]\nshow_users = false\n[remote_desktop]\nenabled = true\nport = 3390\n";
        apply(input, &path).unwrap();
        let (config, warnings) = MachineConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config, MachineConfig::parse(input).0);
        assert!(!config.login.show_users);
        assert_eq!((config.remote.enabled, config.remote.port), (true, 3390));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o644);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["login.toml"]);
    }

    #[test]
    fn apply_refuses_invalid_settings_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let before = "[remote_desktop]\nport = 3390\n";
        std::fs::write(&path, before).unwrap();
        let warnings = apply("[remote_desktop]\nport = 0\n", &path).unwrap_err();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_parse_error_is_one_line() {
        let (_, warnings) = MachineConfig::parse("[login]\nshow_users = \n");
        assert!(warnings[0].contains('\n'), "{warnings:?}");
        let line = one_line(&warnings[0]);
        assert!(!line.contains('\n'), "{line}");
        assert!(line.starts_with("TOML parse error at line 2"), "{line}");
        assert!(!line.contains('|'), "{line}");
        assert_eq!(
            one_line("port: expected a number"),
            "port: expected a number"
        );
    }

    #[test]
    fn unknown_tables_are_dropped_and_known_keys_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        apply(
            "[other]\nkey = 1\n[remote_desktop]\ngroup = \"rdp\"\n",
            &path,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        let mut keys: Vec<_> = table.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["login", "remote_desktop"]);
        let (config, _) = MachineConfig::parse(&text);
        assert_eq!(
            config.remote,
            MachineRemote {
                group: "rdp".into(),
                ..MachineRemote::default()
            }
        );
    }
}
