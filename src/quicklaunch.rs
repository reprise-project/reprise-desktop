//! The Quick Launch folder (`reprisefs::quicklaunch`): one `.desktop` file per button and the order
//! file. The config's `[[quicklaunch]]` entries only seed it the first time; after that the folder is
//! the truth: dropping a shortcut on the bar copies a file into it and reprise-settings edits it while
//! `crate::quickwatch` looks on. Pure file work; the compositor holds the loaded list.

use crate::config::{Action, QuickLaunchEntry};
use reprisefs::apps;
use reprisefs::quicklaunch::{Button, slug, unique_name};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickItem {
    pub file: String,
    pub entry: QuickLaunchEntry,
    /// The application entry the file holds; None for a compositor action. A pinned slot matches windows and opens recent files by it.
    pub app: Option<apps::AppEntry>,
}

/// Create `dir` from `entries` if it does not exist yet. Returns the file names written, in `entries`
/// order; an existing folder returns None and is left alone.
pub fn seed(dir: &Path, entries: &[QuickLaunchEntry]) -> Option<Vec<String>> {
    if dir.exists() {
        return None;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("quicklaunch: could not create {}: {e}", dir.display());
        return None;
    }
    let mut files = Vec::new();
    for e in entries {
        let file = unique_name(dir, &format!("{}.desktop", slug(&e.label)));
        let text = e
            .action
            .launch()
            .and_then(|l| apps::launcher_text(&e.label, e.icon.as_deref(), l));
        let Some(text) = text else {
            log::warn!(
                "quicklaunch: {:?} has a control character in its command",
                e.label
            );
            continue;
        };
        match std::fs::write(dir.join(&file), text) {
            Ok(()) => files.push(file),
            Err(e) => log::warn!("quicklaunch: could not write {file}: {e}"),
        }
    }
    Some(files)
}

/// The buttons in `dir`, sorted by file name; `terminal` wraps entries that want one, as the Programs menu does.
pub fn scan(dir: &Path, terminal: &str) -> Vec<QuickItem> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<QuickItem> = rd
        .flatten()
        .filter_map(|d| {
            let path = d.path();
            let file = path.file_name()?.to_str()?.to_string();
            let id = apps::DesktopId::from_file_name(&file)?;
            let text = apps::read_desktop_file(&path).ok()?;
            let entry = parse(&text, &id, terminal)?;
            let app = apps::parse_desktop(&text, &id);
            Some(QuickItem { file, entry, app })
        })
        .collect();
    items.sort_by(|a, b| a.file.cmp(&b.file));
    items
}

/// The buttons after `dir` changed: its files in the order file's order. A move of the compositor's
/// own not yet written (`moved`) keeps the order `shown` has instead. Nothing is written: the order
/// file is the truth, and one another program just wrote stands.
pub fn reread(dir: &Path, terminal: &str, shown: &[QuickItem], moved: bool) -> Vec<QuickItem> {
    let order = if moved {
        shown.iter().map(|q| q.file.clone()).collect()
    } else {
        reprisefs::quicklaunch::read_order(dir).unwrap_or_default()
    };
    let mut items = scan(dir, terminal);
    reprisefs::quicklaunch::apply_order(&mut items, &order, |q| &q.file);
    items
}

/// Put a shortcut for `src` in `dir`: a [runnable](runs_as_is) `.desktop` file that [`parse`]
/// accepts is copied as is, anything else gets a launcher built on `apps::open_command_for`, which
/// asks before it runs a launcher. Returns the new file's name.
pub fn add(dir: &Path, src: &Path) -> Option<String> {
    add_from(dir, src, &apps::default_dirs())
}

/// [`add`], with `app_dirs` the application folders.
fn add_from(dir: &Path, src: &Path, app_dirs: &[PathBuf]) -> Option<String> {
    let name = src.file_name()?.to_string_lossy().into_owned();
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("quicklaunch: could not create {}: {e}", dir.display());
        return None;
    }
    let as_is = apps::DesktopId::from_file_name(&name).filter(|_| runs_as_is(src, dir, app_dirs));
    let (file, text) = if let Some(id) = as_is {
        let text = apps::read_desktop_file(src).ok()?;
        if parse(&text, &id, "").is_none() {
            log::warn!("quicklaunch: {name} is not an application or action");
            return None;
        }
        (unique_name(dir, &name), text)
    } else {
        let icon = if src.is_dir() {
            "folder"
        } else {
            reprisefs::icons::icon_for_file(src)
        };
        let Some(cmd) = apps::open_command_for(src) else {
            log::warn!("quicklaunch: {} is not a UTF-8 path", src.display());
            return None;
        };
        let Some(text) = apps::launcher_text(&name, Some(icon), apps::Launch::Exec(&cmd)) else {
            log::warn!("quicklaunch: {name:?} has a control character in its name");
            return None;
        };
        (unique_name(dir, &format!("{}.desktop", slug(&name))), text)
    };
    match std::fs::write(dir.join(&file), text) {
        Ok(()) => Some(file),
        Err(e) => {
            log::warn!("quicklaunch: could not write {file}: {e}");
            None
        }
    }
}

/// Whether the `.desktop` file `src` may become a button that runs its `Exec`: it sits in one of
/// `app_dirs` (or a folder below one) or in `dir` (Quick Launch's own), or the user allowed it to
/// run.
fn runs_as_is(src: &Path, dir: &Path, app_dirs: &[PathBuf]) -> bool {
    let real = |d: &Path| std::fs::canonicalize(d).ok();
    let Some(parent) = src.parent().and_then(real) else {
        return false;
    };
    real(dir).is_some_and(|d| d == parent)
        || apps::under_apps_dirs(app_dirs, &parent)
        || apps::is_trusted(src)
}

pub fn remove(dir: &Path, file: &str) {
    if let Err(e) = std::fs::remove_file(dir.join(file)) {
        log::warn!("quicklaunch: could not remove {file}: {e}");
    }
}

/// One `.desktop` file's button. A compositor action travels as `X-Reprise-Action=`; an application
/// entry becomes its argument vector, as the Programs menu builds it.
pub fn parse(text: &str, id: &apps::DesktopId, terminal: &str) -> Option<QuickLaunchEntry> {
    let is_action = |a: &str| Action::from_parts(a, None).is_ok();
    Some(match reprisefs::quicklaunch::button(text, id, is_action)? {
        Button::App(app) => QuickLaunchEntry {
            action: Action::Exec {
                argv: apps::launch_argv(&app, terminal),
                app: app.id.clone(),
                dir: app.path.clone(),
            },
            label: app.name,
            icon: app.icon,
        },
        Button::Action { name, icon, action } => QuickLaunchEntry {
            label: name,
            icon,
            action: Action::from_parts(&action, None).ok()?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("reprise-quick-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn exec(argv: &[&str], app: &str) -> Action {
        Action::Exec {
            argv: Ok(argv.iter().map(Into::into).collect()),
            app: app.parse().unwrap(),
            dir: None,
        }
    }

    fn entry(label: &str, action: Action) -> QuickLaunchEntry {
        QuickLaunchEntry {
            label: label.into(),
            icon: Some("x".into()),
            action,
        }
    }

    /// A file dropped in lands after the listed ones and leaves the order file alone, so an order
    /// written after it (Settings moving the new file up) is what a re-read shows. A move of our own not yet written keeps its order.
    #[test]
    fn a_reread_takes_the_order_file_and_never_writes_it() {
        let dir = fixture("reread");
        let _c = Cleanup(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["a", "b", "x"] {
            let text = apps::launcher_text(f, None, apps::Launch::Exec(f)).unwrap();
            std::fs::write(dir.join(format!("{f}.desktop")), text).unwrap();
        }
        let order = |v: &[&str]| v.iter().map(|f| format!("{f}.desktop")).collect::<Vec<_>>();
        let files = |items: &[QuickItem]| items.iter().map(|q| q.file.clone()).collect::<Vec<_>>();
        reprisefs::quicklaunch::write_order(&dir, &order(&["a", "b"])).unwrap();
        let shown = reread(&dir, "foot", &[], false);
        assert_eq!(files(&shown), order(&["a", "b", "x"]));
        reprisefs::quicklaunch::write_order(&dir, &order(&["a", "x", "b"])).unwrap();
        let shown = reread(&dir, "foot", &shown, false);
        assert_eq!(files(&shown), order(&["a", "x", "b"]));
        assert_eq!(
            reprisefs::quicklaunch::read_order(&dir),
            Some(order(&["a", "x", "b"])),
            "the order Settings wrote stands"
        );
        let mut moved = shown.clone();
        moved.swap(0, 2);
        let kept = reread(&dir, "foot", &moved, true);
        assert_eq!(files(&kept), order(&["b", "x", "a"]));
    }

    #[test]
    fn actions_and_commands_round_trip_through_desktop_files() {
        let t = apps::launcher_text(
            "Show Desktop",
            Some("user-desktop"),
            Action::ShowDesktop.launch().unwrap(),
        )
        .unwrap();
        assert!(t.contains("X-Reprise-Action=showdesktop\n"));
        let p = parse(&t, &"a.desktop".parse().unwrap(), "foot").unwrap();
        assert_eq!(
            p,
            entry("Show Desktop", Action::ShowDesktop).with_icon("user-desktop")
        );
        let t = apps::launcher_text("Files", None, apps::Launch::Exec("nautilus 100%")).unwrap();
        let p = parse(&t, &"b.desktop".parse().unwrap(), "foot").unwrap();
        assert_eq!(p.action, exec(&["nautilus", "100%"], "b.desktop"));
        assert_eq!(p.icon, None);
        let text = "[Desktop Entry]\nType=Application\nName=Top\nExec=htop\nTerminal=true\n";
        let p = parse(text, &"htop.desktop".parse().unwrap(), "foot").unwrap();
        assert_eq!(p.action, exec(&["foot", "-e", "htop"], "htop.desktop"));
        assert!(
            parse(
                "[Desktop Entry]\nName=Nothing\n",
                &"n.desktop".parse().unwrap(),
                "foot"
            )
            .is_none()
        );
    }

    impl QuickLaunchEntry {
        fn with_icon(mut self, icon: &str) -> Self {
            self.icon = Some(icon.into());
            self
        }
    }

    #[test]
    fn seed_writes_once_then_scan_add_and_remove_manage_the_folder() {
        let dir = fixture("seed");
        let _c = Cleanup(dir.clone());
        let entries = vec![
            entry("Show Desktop", Action::ShowDesktop),
            entry("Terminal", Action::Terminal),
        ];
        let files = seed(&dir, &entries).unwrap();
        assert_eq!(files, ["show-desktop.desktop", "terminal.desktop"]);
        assert_eq!(seed(&dir, &entries), None);
        let items = scan(&dir, "foot");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].file, "show-desktop.desktop");
        assert_eq!(items[0].entry.action, Action::ShowDesktop);
        let doc = dir.join("Notes.txt");
        std::fs::write(&doc, "hi").unwrap();
        let added = add(&dir, &doc).unwrap();
        assert_eq!(added, "notes-txt.desktop");
        let again = add(&dir, &doc).unwrap();
        assert_eq!(again, "notes-txt (2).desktop");
        let items = scan(&dir, "foot");
        let notes = items.iter().find(|i| i.file == added).unwrap();
        assert_eq!(notes.entry.label, "Notes.txt");
        assert_eq!(notes.entry.icon.as_deref(), Some("text-x-generic"));
        assert_eq!(
            notes.entry.action,
            exec(&["xdg-open", &doc.to_string_lossy()], &added)
        );
        let sub = dir.join("Reports");
        std::fs::create_dir(&sub).unwrap();
        let added_dir = add(&dir, &sub).unwrap();
        assert_eq!(added_dir, "reports.desktop");
        let items = scan(&dir, "foot");
        let reports = items.iter().find(|i| i.file == added_dir).unwrap();
        assert_eq!(reports.entry.icon.as_deref(), Some("folder"));
        assert_eq!(
            reports.entry.action,
            exec(&["reprise-explorer", &sub.to_string_lossy()], &added_dir)
        );
        remove(&dir, &added_dir);
        std::fs::remove_dir(&sub).unwrap();
        let copied = add(&dir, &dir.join("terminal.desktop")).unwrap();
        assert_eq!(copied, "terminal (2).desktop");
        remove(&dir, &copied);
        remove(&dir, &again);
        assert_eq!(scan(&dir, "foot").len(), 3);
    }

    /// Pinning copies the app's `.desktop` file from an application folder into Quick Launch's,
    /// keeping its name and class so the app's windows find it; unpinning deletes the copy and nothing
    /// else.
    #[test]
    fn pin_copies_the_desktop_file_and_unpin_removes_it() {
        let src = fixture("pin-src");
        let _c = Cleanup(src.clone());
        std::fs::create_dir_all(&src).unwrap();
        let app = src.join("vscode.desktop");
        std::fs::write(
            &app,
            "[Desktop Entry]\nType=Application\nName=Code\nExec=code %F\nStartupWMClass=Code\n",
        )
        .unwrap();
        let dir = fixture("pin-dst");
        let _c2 = Cleanup(dir.clone());
        let file = add_from(&dir, &app, std::slice::from_ref(&src)).unwrap();
        assert_eq!(file, "vscode.desktop");
        let items = scan(&dir, "foot");
        assert_eq!(items.len(), 1);
        let pinned = items[0].app.as_ref().expect("an application entry");
        assert_eq!(pinned.startup_wm_class.as_deref(), Some("Code"));
        assert_eq!(crate::taskbar::pin_for(&items, "code", None), Some(0));
        remove(&dir, &file);
        assert!(scan(&dir, "foot").is_empty());
        assert!(app.exists(), "the source stays");
    }

    #[test]
    fn a_launcher_from_elsewhere_opens_it_until_launching_it_is_allowed() {
        let src = fixture("loose-src");
        let _c = Cleanup(src.clone());
        std::fs::create_dir_all(&src).unwrap();
        let invoice = src.join("Invoice.desktop");
        std::fs::write(
            &invoice,
            "[Desktop Entry]\nType=Application\nName=Invoice\nExec=sh -c true\n",
        )
        .unwrap();
        let dir = fixture("loose-dst");
        let _c2 = Cleanup(dir.clone());
        let apps_dir = fixture("loose-apps");
        let file = add_from(&dir, &invoice, std::slice::from_ref(&apps_dir)).unwrap();
        assert_eq!(file, "invoice-desktop.desktop");
        let items = scan(&dir, "foot");
        assert_eq!(
            items[0].entry.action,
            exec(&["xdg-open", &invoice.to_string_lossy()], &file),
            "opening it asks before it runs"
        );
        assert_eq!(items[0].entry.label, "Invoice.desktop");
        apps::allow_launching(&invoice).unwrap();
        let file = add_from(&dir, &invoice, &[apps_dir]).unwrap();
        assert_eq!(file, "Invoice.desktop", "an allowed launcher is copied");
        let copied = scan(&dir, "foot");
        let copied = copied.iter().find(|i| i.file == file).unwrap();
        assert_eq!(copied.entry.action, exec(&["sh", "-c", "true"], &file));
    }

    #[test]
    fn a_launcher_in_a_folder_below_an_application_folder_is_copied() {
        let apps_dir = fixture("nested-apps");
        let _c = Cleanup(apps_dir.clone());
        let folder = apps_dir.join("wine/Programs/Test");
        std::fs::create_dir_all(&folder).unwrap();
        let app = folder.join("Test.desktop");
        std::fs::write(
            &app,
            "[Desktop Entry]\nType=Application\nName=Test\nExec=true\n",
        )
        .unwrap();
        let dir = fixture("nested-dst");
        let _c2 = Cleanup(dir.clone());
        assert_eq!(
            add_from(&dir, &app, &[apps_dir]).as_deref(),
            Some("Test.desktop")
        );
    }

    #[test]
    fn add_refuses_a_desktop_file_that_is_not_a_button() {
        let src = fixture("link-src");
        let _c = Cleanup(src.clone());
        std::fs::create_dir_all(&src).unwrap();
        let link = src.join("site.desktop");
        std::fs::write(
            &link,
            "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org\n",
        )
        .unwrap();
        let dir = fixture("link-dst");
        let _c2 = Cleanup(dir.clone());
        assert_eq!(add_from(&dir, &link, std::slice::from_ref(&src)), None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_fifo_named_desktop_is_skipped_by_scan_and_refused_by_add() {
        let dir = fixture("fifo");
        let _c = Cleanup(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("pipe.desktop");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert_eq!(scan(&dir, "foot"), []);
        let other = fixture("fifo-dst");
        let _c2 = Cleanup(other.clone());
        assert_eq!(add_from(&other, &fifo, std::slice::from_ref(&dir)), None);
        assert_eq!(scan(&other, "foot"), []);
    }
}
