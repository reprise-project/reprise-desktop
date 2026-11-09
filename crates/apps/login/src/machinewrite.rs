//! Writes the machine settings for `reprise-login-settings`.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd as _;
use std::path::Path;

use repriselogin::machine::{MachineConfig, file_tag, parse_save_request};

use crate::atomic;

/// The longest settings text the helper reads, in bytes.
pub const MAX_INPUT: usize = 16 * 1024;

/// Why [`apply`] failed.
#[derive(Debug)]
pub enum Error {
    /// Every warning the input raised.
    Invalid(Vec<String>),
    /// The file is no longer the one the settings were read from.
    Changed,
    Io(io::Error),
}

/// Validates `input`, a `save_request`, and atomically replaces `path` with its settings while
/// `path` still has the tag the request expects. Invalid input or a changed file leaves `path`
/// untouched. A lock on the directory runs one save at a time.
pub fn apply(input: &str, path: &Path) -> Result<(), Error> {
    let Some((expect, toml)) = parse_save_request(input) else {
        return Err(Error::Invalid(vec![
            "the settings do not say which file they replace".into(),
        ]));
    };
    let (config, warnings) = MachineConfig::parse(toml);
    if !warnings.is_empty() {
        return Err(Error::Invalid(warnings));
    }
    let _lock = lock_dir(path).map_err(Error::Io)?;
    let now = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::Io(e)),
    };
    if file_tag(now.as_deref()) != expect {
        return Err(Error::Changed);
    }
    atomic::replace(path, config.to_toml().as_bytes(), 0o755, 0o644).map_err(Error::Io)
}

/// An exclusive `flock` on `path`'s directory, made when missing, held until the file drops.
fn lock_dir(path: &Path) -> io::Result<File> {
    let dir = path.parent().unwrap_or(Path::new("."));
    atomic::make_dir(dir, 0o755)?;
    let d = File::open(dir)?;
    loop {
        // SAFETY: flock on a descriptor this function owns.
        if unsafe { libc::flock(d.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(d);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    use repriselogin::machine::MachineRemote;

    use super::*;
    use repriselogin::machine::save_request;

    /// `toml` as a save request expecting `path` as it is now.
    fn req(path: &Path, toml: &str) -> String {
        save_request(&file_tag(std::fs::read(path).ok().as_deref()), toml)
    }

    #[test]
    fn apply_writes_the_parsed_settings_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(&path, "[login]\nshow_users = true\n").unwrap();
        let input = "[login]\nshow_users = false\n[remote_desktop]\nenabled = true\nport = 3390\n";
        apply(&req(&path, input), &path).unwrap();
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
        apply(&req(&path, "[login]\nshow_users = false\n"), &path).unwrap();
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
        let path = Path::new(&dir).join("login.toml");
        apply(&req(&path, "[login]\nshow_users = false\n"), &path).unwrap();
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
        let err = apply(
            &save_request("none", "[login]\nshow_users = false\n"),
            &path,
        )
        .unwrap_err();
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
        let Err(Error::Invalid(warnings)) =
            apply(&req(&path, "[remote_desktop]\nport = 0\n"), &path)
        else {
            panic!("port 0 is refused as invalid");
        };
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn unknown_tables_are_dropped_and_known_keys_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        apply(
            &req(
                &path,
                "[other]\nkey = 1\n[remote_desktop]\ngroup = \"rdp\"\n",
            ),
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

    #[test]
    fn a_file_changed_since_it_was_read_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(&path, "[remote_desktop]\nenabled = true\n").unwrap();
        let stale = req(&path, "[remote_desktop]\nenabled = true\nport = 3390\n");
        let turned_off = "[remote_desktop]\nenabled = false\n";
        std::fs::write(&path, turned_off).unwrap();
        assert!(matches!(apply(&stale, &path), Err(Error::Changed)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), turned_off);
        std::fs::remove_file(&path).unwrap();
        assert!(
            matches!(apply(&stale, &path), Err(Error::Changed)),
            "a removed file is a change too"
        );
        assert!(!path.exists());
    }

    #[test]
    fn a_request_without_its_tag_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let Err(Error::Invalid(w)) = apply("[login]\nshow_users = false\n", &path) else {
            panic!("the bare TOML of an older Settings is refused");
        };
        assert_eq!(w, ["the settings do not say which file they replace"]);
        assert!(!path.exists());
    }

    #[test]
    fn a_save_waits_for_the_one_holding_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let held = lock_dir(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(apply(&req(&p, "[login]\nshow_users = false\n"), &p).is_ok());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the save waits"
        );
        drop(held);
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true));
    }
}
