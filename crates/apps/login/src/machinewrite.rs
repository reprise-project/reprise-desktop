//! Writes the machine settings for `reprise-login-settings`.

use std::io;
use std::path::Path;

use repriselogin::machine::MachineConfig;

use crate::atomic;

/// The longest settings text the helper reads, in bytes.
pub const MAX_INPUT: usize = 16 * 1024;

/// Why [`apply`] failed.
#[derive(Debug)]
pub enum Error {
    /// Every warning the input raised.
    Invalid(Vec<String>),
    Io(io::Error),
}

/// Validates `input` and atomically replaces `path` with its settings. Invalid input leaves `path`
/// untouched.
pub fn apply(input: &str, path: &Path) -> Result<(), Error> {
    let (config, warnings) = MachineConfig::parse(input);
    if !warnings.is_empty() {
        return Err(Error::Invalid(warnings));
    }
    atomic::replace(path, config.to_toml().as_bytes(), 0o755, 0o644).map_err(Error::Io)
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

#[cfg(test)]
mod tests {
    use std::fs::Permissions;
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
    fn a_planted_tmp_symlink_is_replaced_and_its_target_kept() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("victim");
        std::fs::write(&victim, "victim").unwrap();
        std::fs::set_permissions(&victim, Permissions::from_mode(0o600)).unwrap();
        let path = dir.path().join("login.toml");
        std::os::unix::fs::symlink(&victim, dir.path().join("login.toml.tmp")).unwrap();
        apply("[login]\nshow_users = false\n", &path).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        let mode = std::fs::metadata(&victim).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o600);
        assert!(
            !MachineConfig::parse(&std::fs::read_to_string(&path).unwrap())
                .0
                .login
                .show_users
        );
        assert!(std::fs::symlink_metadata(dir.path().join("login.toml.tmp")).is_err());
    }

    const CHILD_DIR: &str = "REPRISE_MACHINEWRITE_DIR";

    #[test]
    #[ignore = "run by apply_sets_the_modes_whatever_the_umask"]
    fn apply_in_a_child() {
        let Some(dir) = std::env::var_os(CHILD_DIR) else {
            return;
        };
        apply(
            "[login]\nshow_users = false\n",
            &Path::new(&dir).join("login.toml"),
        )
        .unwrap();
    }

    #[test]
    fn apply_sets_the_modes_whatever_the_umask() {
        use std::os::unix::process::CommandExt as _;
        for umask in [0, 0o077] {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("reprise");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "machinewrite::tests::apply_in_a_child",
                    "--ignored",
                ])
                .env(CHILD_DIR, &dir);
            // SAFETY: umask is async-signal-safe and touches only the child.
            unsafe {
                child.pre_exec(move || {
                    libc::umask(umask);
                    Ok(())
                });
            }
            let out = child.output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode(&dir), 0o755, "directory under umask {umask:o}");
            assert_eq!(
                mode(&dir.join("login.toml")),
                0o644,
                "file under umask {umask:o}"
            );
        }
    }

    #[test]
    fn a_failed_rename_is_an_io_error_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "").unwrap();
        let err = apply("[login]\nshow_users = false\n", &path).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "{err:?}");
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["login.toml"]);
    }

    #[test]
    fn apply_refuses_invalid_settings_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let before = "[remote_desktop]\nport = 3390\n";
        std::fs::write(&path, before).unwrap();
        let Err(Error::Invalid(warnings)) = apply("[remote_desktop]\nport = 0\n", &path) else {
            panic!("port 0 is refused as invalid");
        };
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
