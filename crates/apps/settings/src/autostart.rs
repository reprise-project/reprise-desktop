//! The settings app's side of XDG autostart: the user files that check and uncheck a row. Parsing,
//! text edits and the directory scan are [`reprisefs::autostart`]'s.

use reprisefs::apps;
use reprisefs::autostart::{self, Entry};
use std::path::{Path, PathBuf};

pub fn location(e: &Entry) -> &'static str {
    if e.user { "User" } else { "System" }
}

pub fn user_dir(config_home: &Path) -> PathBuf {
    config_home.join("autostart")
}

/// Check or uncheck `e`.
///
/// A system entry gains a user file marked `X-Reprise-Override=true`; such a file is deleted when
/// checked again, so the system entry comes back untouched. Any other user file just has `Hidden` rewritten.
pub fn set_enabled(dir: &Path, e: &Entry, on: bool) -> std::io::Result<()> {
    let path = dir.join(format!("{}.desktop", e.id));
    if e.user {
        let text = std::fs::read_to_string(&path)?;
        if on && autostart::is_override(&text) {
            return std::fs::remove_file(&path);
        }
        return repriseconf::write_atomic(&path, &autostart::set_hidden(&text, !on));
    }
    let shadow = Entry {
        hidden: !on,
        ..e.clone()
    };
    std::fs::create_dir_all(dir)?;
    repriseconf::write_atomic(&path, &autostart::override_text(&shadow))
}

/// Delete a user entry's file. A system entry has none and can only be unchecked, so that is an error, not a no-op.
pub fn remove(dir: &Path, e: &Entry) -> std::io::Result<()> {
    if !e.user {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "a system entry has no user file to remove",
        ));
    }
    std::fs::remove_file(dir.join(format!("{}.desktop", e.id)))
}

/// Add `path` to the user autostart directory: a `.desktop` file is copied in as it stands, anything
/// else is wrapped in a launcher that runs it. Returns the id the new entry will be listed under.
///
/// Refuses rather than overwriting an existing entry (a hand-written file, or a shadow of a system
/// entry), and a name that is not UTF-8, since the id and `Exec=` are text.
pub fn add(dir: &Path, path: &Path) -> std::io::Result<String> {
    let invalid = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidInput, why);
    let not_utf8 = || invalid("its name is not valid UTF-8");
    let stem = path
        .file_stem()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("no file name"))?
        .to_str()
        .ok_or_else(not_utf8)?
        .to_string();
    let text = if path.extension().is_some_and(|e| e == "desktop") {
        std::fs::read_to_string(path)?
    } else {
        let exec = apps::exec_arg(path.to_str().ok_or_else(not_utf8)?);
        apps::launcher_text(&stem, None, apps::Launch::Exec(&exec)).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the file name holds a control character",
            )
        })?
    };
    let target = dir.join(format!("{stem}.desktop"));
    if target.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} is already an autostart entry", target.display()),
        ));
    }
    std::fs::create_dir_all(dir)?;
    repriseconf::write_atomic(&target, &text)?;
    Ok(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "reprise-settings-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("temp dir");
        p
    }

    fn put(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join(name), text).expect("write");
    }

    fn entry(id: &str, text: &str, user: bool) -> Entry {
        autostart::parse(id, text, user).expect("parses")
    }

    #[test]
    fn scan_merges_system_and_user_with_xdg_precedence() {
        let root = tmp("scan");
        let (etc, other, home) = (root.join("etc"), root.join("other"), root.join("home"));
        put(
            &etc.join("autostart"),
            "a.desktop",
            "[Desktop Entry]\nName=Alpha\nExec=/etc/a\n",
        );
        put(
            &other.join("autostart"),
            "a.desktop",
            "[Desktop Entry]\nName=Alpha\nExec=/other/a\n",
        );
        put(
            &other.join("autostart"),
            "b.desktop",
            "[Desktop Entry]\nName=Bravo\nExec=/other/b\n",
        );
        put(
            &user_dir(&home),
            "b.desktop",
            "[Desktop Entry]\nName=Bravo\nExec=/home/b\nHidden=true\n",
        );

        let got = autostart::scan(&[etc, other], &home);
        let by: Vec<(&str, &str, bool, bool)> = got
            .iter()
            .map(|e| (e.id.as_str(), e.exec.as_str(), e.user, e.hidden))
            .collect();
        assert_eq!(
            by,
            vec![("a", "/etc/a", false, false), ("b", "/home/b", true, true)],
            "the first config dir wins, and the user file beats both"
        );
        assert_eq!(location(&got[0]), "System");
        assert_eq!(location(&got[1]), "User");
    }

    #[test]
    fn unchecking_a_system_entry_writes_a_marked_override() {
        let dir = tmp("shadow").join("autostart");
        let e = entry(
            "reprise-sound",
            "[Desktop Entry]\nName=Volume\nExec=reprise-sound --tray\n",
            false,
        );
        set_enabled(&dir, &e, false).expect("write");

        let text = std::fs::read_to_string(dir.join("reprise-sound.desktop")).expect("read");
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=Volume\nExec=reprise-sound --tray\nHidden=true\nX-Reprise-Override=true\n"
        );

        // Re-checking deletes the shadow instead of writing Hidden=false, so the system entry stands alone again.
        let shadow = entry("reprise-sound", &text, true);
        set_enabled(&dir, &shadow, true).expect("remove");
        assert!(!dir.join("reprise-sound.desktop").exists());
    }

    #[test]
    fn unchecking_a_user_entry_sets_hidden_in_place() {
        let dir = tmp("hide").join("autostart");
        put(
            &dir,
            "mine.desktop",
            "# mine\n[Desktop Entry]\nName=Mine\nExec=mine\nComment=keep me\n",
        );
        let e = entry(
            "mine",
            &std::fs::read_to_string(dir.join("mine.desktop")).unwrap(),
            true,
        );

        set_enabled(&dir, &e, false).expect("hide");
        let text = std::fs::read_to_string(dir.join("mine.desktop")).unwrap();
        assert_eq!(
            text,
            "# mine\n[Desktop Entry]\nHidden=false\nName=Mine\nExec=mine\nComment=keep me\n"
                .replace("Hidden=false", "Hidden=true")
        );

        let e = entry("mine", &text, true);
        set_enabled(&dir, &e, true).expect("show");
        let text = std::fs::read_to_string(dir.join("mine.desktop")).unwrap();
        assert!(text.contains("Hidden=false"), "the file stays: {text}");
        assert!(text.contains("Comment=keep me"));
    }

    /// A file name with a line break would write a second key into the launcher; Add refuses it before touching the directory.
    #[test]
    fn add_refuses_a_name_with_a_line_break() {
        let root = tmp("add-newline");
        let dir = root.join("autostart");
        let bin = root.join("my\napp");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let err = add(&dir, &bin).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.exists(), "nothing was written");
        let bin = root.join("my\rapp");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert!(add(&dir, &bin).is_err());
    }

    #[test]
    fn add_refuses_a_path_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let root = tmp("add-not-utf8");
        let dir = root.join("autostart");
        let odd = root.join(std::ffi::OsStr::from_bytes(b"bin\xff"));
        std::fs::create_dir_all(&odd).unwrap();
        let bin = odd.join("app");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let err = add(&dir, &bin).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "its name is not valid UTF-8");
        assert!(!dir.exists(), "nothing was written");
    }

    #[test]
    fn add_copies_a_desktop_file_and_wraps_an_executable() {
        let root = tmp("add");
        let dir = root.join("autostart");
        let src = root.join("thing.desktop");
        std::fs::write(&src, "[Desktop Entry]\nName=Thing\nExec=thing %U\n").unwrap();
        assert_eq!(add(&dir, &src).unwrap(), "thing");
        assert_eq!(
            std::fs::read_to_string(dir.join("thing.desktop")).unwrap(),
            "[Desktop Entry]\nName=Thing\nExec=thing %U\n",
            "copied as it stands"
        );

        assert!(
            add(&dir, &src).is_err(),
            "adding the same entry twice does not overwrite it"
        );

        let bin = root.join("myapp");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(add(&dir, &bin).unwrap(), "myapp");
        let text = std::fs::read_to_string(dir.join("myapp.desktop")).unwrap();
        let e = entry("myapp", &text, true);
        assert_eq!(apps::exec_argv(&e.exec_line), [bin.to_string_lossy()]);
        assert!(!autostart::is_override(&text), "not a shadow of anything");
        assert!(e.runs_in("reprise"));
    }

    /// Add's launcher runs the path it was given, quoting and `%` included.
    #[test]
    fn an_added_executable_parses_back_to_its_path() {
        let root = tmp("add-quoted");
        let dir = root.join("autostart");
        for name in ["run me", "a$b`c\\d", "50% off"] {
            let bin = root.join(name);
            std::fs::write(&bin, "#!/bin/sh\n").unwrap();
            let id = add(&dir, &bin).unwrap();
            let text = std::fs::read_to_string(dir.join(format!("{id}.desktop"))).unwrap();
            let e = autostart::parse(&id, &text, true).expect("parses");
            assert_eq!(
                apps::exec_argv(&e.exec_line),
                [bin.to_string_lossy()],
                "{text}"
            );
        }
    }

    #[test]
    fn remove_drops_a_user_file_and_refuses_a_system_one() {
        let dir = tmp("remove").join("autostart");
        put(&dir, "mine.desktop", "[Desktop Entry]\nExec=mine\n");
        let user = entry("mine", "[Desktop Entry]\nExec=mine\n", true);
        remove(&dir, &user).expect("removed");
        assert!(!dir.join("mine.desktop").exists());

        let system = entry("theirs", "[Desktop Entry]\nExec=theirs\n", false);
        assert!(remove(&dir, &system).is_err());
    }
}
