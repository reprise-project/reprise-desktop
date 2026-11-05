//! The settings app's side of XDG autostart: the user files that check and uncheck a row. Parsing,
//! text edits and the directory scan are [`reprisefs::autostart`]'s.

use reprisefs::apps;
use reprisefs::autostart::{self, Entry};
use std::path::{Path, PathBuf};

/// The id of the session's polkit agent, the only one it starts.
pub const RUN_AS: &str = "reprise-polkit";

pub fn location(e: &Entry) -> &'static str {
    if e.user { "User" } else { "System" }
}

pub fn user_dir(config_home: &Path) -> PathBuf {
    config_home.join("autostart")
}

/// Unchecking sets `Hidden=true` (a marked override for a system entry). Checking deletes a stand-in for a system
/// entry that runs on `desktops`, else leaves a user file whose fence admits them ([`Entry::admitted`]).
pub fn set_enabled(
    dir: &Path,
    e: &Entry,
    on: bool,
    desktops: &[String],
    config_dirs: &[PathBuf],
) -> std::io::Result<()> {
    let path = dir.join(format!("{}.desktop", e.id));
    if !on {
        if e.user {
            let text = std::fs::read_to_string(&path)?;
            return repriseconf::write_atomic(&path, &autostart::set_hidden(&text, true));
        }
        std::fs::create_dir_all(dir)?;
        let shadow = Entry {
            hidden: true,
            ..e.clone()
        };
        return repriseconf::write_atomic(&path, &autostart::override_text(&shadow));
    }
    let admitted = Entry {
        hidden: false,
        ..e.admitted(desktops)
    };
    if e.user {
        let text = std::fs::read_to_string(&path)?;
        // A marked override, or a bare `Hidden=true` file with no `Exec`, only stands in for the system entry.
        let stand_in = autostart::is_override(&text)
            || autostart::parse(&e.id, &text, true).is_none_or(|f| f.exec.is_empty());
        if !stand_in {
            let fenced =
                admitted.only_show_in != e.only_show_in || admitted.not_show_in != e.not_show_in;
            let text = if fenced {
                autostart::set_show_in(&text, &admitted)
            } else {
                text
            };
            return repriseconf::write_atomic(&path, &autostart::set_hidden(&text, false));
        }
        if autostart::system_entry(config_dirs, &e.id).is_some_and(|s| s.runs_on(desktops)) {
            return std::fs::remove_file(&path);
        }
    }
    std::fs::create_dir_all(dir)?;
    repriseconf::write_atomic(&path, &autostart::copy_text(&admitted))
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
        let root = tmp("shadow");
        let (etc, dir) = (vec![root.join("etc")], root.join("autostart"));
        let sys = "[Desktop Entry]\nName=Volume\nExec=reprise-sound --tray\n";
        put(&etc[0].join("autostart"), "reprise-sound.desktop", sys);
        let e = entry("reprise-sound", sys, false);
        let here = vec!["reprise".to_string()];
        set_enabled(&dir, &e, false, &here, &etc).expect("write");

        let text = std::fs::read_to_string(dir.join("reprise-sound.desktop")).expect("read");
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=Volume\nExec=reprise-sound --tray\nHidden=true\nX-Reprise-Override=true\n"
        );

        // Re-checking deletes the shadow instead of writing Hidden=false, so the system entry stands alone again.
        let shadow = entry("reprise-sound", &text, true);
        set_enabled(&dir, &shadow, true, &here, &etc).expect("remove");
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

        let here = vec!["reprise".to_string()];
        set_enabled(&dir, &e, false, &here, &[]).expect("hide");
        let text = std::fs::read_to_string(dir.join("mine.desktop")).unwrap();
        assert_eq!(
            text,
            "# mine\n[Desktop Entry]\nHidden=false\nName=Mine\nExec=mine\nComment=keep me\n"
                .replace("Hidden=false", "Hidden=true")
        );

        let e = entry("mine", &text, true);
        set_enabled(&dir, &e, true, &here, &[]).expect("show");
        let text = std::fs::read_to_string(dir.join("mine.desktop")).unwrap();
        assert!(text.contains("Hidden=false"), "the file stays: {text}");
        assert!(text.contains("Comment=keep me"));
    }

    #[test]
    fn checking_an_entry_fenced_to_another_desktop_sticks_through_a_rescan() {
        let dir = tmp("fenced").join("autostart");
        let here = vec!["reprise".to_string()];
        let e = entry(
            "kwallet",
            "[Desktop Entry]\nName=KWallet\nExec=kwalletd6\nOnlyShowIn=KDE;\n",
            false,
        );
        set_enabled(&dir, &e, true, &here, &[]).expect("write");
        let text = std::fs::read_to_string(dir.join("kwallet.desktop")).unwrap();
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=KWallet\nExec=kwalletd6\nHidden=false\nOnlyShowIn=KDE;reprise;\n"
        );
        let copy = entry("kwallet", &text, true);
        assert!(copy.runs_on(&here), "checked after Apply");

        set_enabled(&dir, &copy, false, &here, &[]).expect("uncheck");
        let off = entry(
            "kwallet",
            &std::fs::read_to_string(dir.join("kwallet.desktop")).unwrap(),
            true,
        );
        assert!(!off.runs_on(&here));
        set_enabled(&dir, &off, true, &here, &[]).expect("check again");
        let again = entry(
            "kwallet",
            &std::fs::read_to_string(dir.join("kwallet.desktop")).unwrap(),
            true,
        );
        assert!(again.runs_on(&here), "a second check sticks too");
    }

    #[test]
    fn checking_a_marked_shadow_still_fenced_off_here_rewrites_it() {
        let dir = tmp("old-shadow").join("autostart");
        let here = vec!["reprise".to_string()];
        let sys = entry(
            "kwallet",
            "[Desktop Entry]\nName=KWallet\nExec=kwalletd6\nOnlyShowIn=KDE;\n",
            false,
        );
        put(&dir, "kwallet.desktop", &autostart::override_text(&sys));
        let shadow = entry(
            "kwallet",
            &std::fs::read_to_string(dir.join("kwallet.desktop")).unwrap(),
            true,
        );
        set_enabled(&dir, &shadow, true, &here, &[]).expect("write");
        let text = std::fs::read_to_string(dir.join("kwallet.desktop")).expect("the file stays");
        assert!(!autostart::is_override(&text));
        assert!(entry("kwallet", &text, true).runs_on(&here));
    }

    #[test]
    fn checking_a_marked_shadow_of_a_hidden_system_entry_keeps_it_running() {
        let root = tmp("hidden-sys");
        let (etc, home) = (vec![root.join("etc")], root.join("home"));
        let dir = user_dir(&home);
        let here = vec!["reprise".to_string()];
        let sys = "[Desktop Entry]\nName=Tray\nExec=tray\nHidden=true\n";
        put(&etc[0].join("autostart"), "tray.desktop", sys);
        put(
            &dir,
            "tray.desktop",
            &autostart::override_text(&entry("tray", sys, false)),
        );
        let row = |id: &str| {
            autostart::scan(&etc, &home)
                .into_iter()
                .find(|e| e.id == id)
                .expect("listed")
        };
        assert!(!row("tray").runs_on(&here));

        set_enabled(&dir, &row("tray"), true, &here, &etc).expect("check");
        assert!(row("tray").runs_on(&here), "checked after a rescan");
    }

    #[test]
    fn checking_a_users_own_fenced_file_edits_it_in_place() {
        let dir = tmp("own-fenced").join("autostart");
        let here = vec!["reprise".to_string()];
        let text = "[Desktop Entry]\nName=Mine\nExec=mine\nComment=keep me\nOnlyShowIn=GNOME;\nX-GNOME-Autostart-Delay=5\n";
        put(&dir, "mine.desktop", text);
        set_enabled(&dir, &entry("mine", text, true), true, &here, &[]).expect("check");
        assert_eq!(
            std::fs::read_to_string(dir.join("mine.desktop")).unwrap(),
            "[Desktop Entry]\nHidden=false\nName=Mine\nExec=mine\nComment=keep me\nOnlyShowIn=GNOME;reprise;\nX-GNOME-Autostart-Delay=5\n"
        );
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
