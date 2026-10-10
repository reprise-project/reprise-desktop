//! The Send To submenu's rows, shared by the desktop's menu and Explorer's.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{names, ops, places};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendTo {
    Zip,
    DesktopShortcut,
    Mail,
    Folder(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendToRow {
    pub label: String,
    pub icon: &'static str,
    pub dest: SendTo,
    pub enabled: bool,
}

pub fn rows(
    desktop: Option<&Path>,
    documents: Option<PathBuf>,
    drives: &[PathBuf],
) -> Vec<SendToRow> {
    let row = |label: &str, icon: &'static str, dest: SendTo| SendToRow {
        label: label.to_string(),
        icon,
        dest,
        enabled: true,
    };
    let mut rows = vec![
        row("Compressed (zipped) Folder", "application-zip", SendTo::Zip),
        SendToRow {
            enabled: desktop.is_some_and(Path::is_dir),
            ..row(
                "Desktop (create shortcut)",
                "user-desktop",
                SendTo::DesktopShortcut,
            )
        },
        row("Mail Recipient", "internet-mail", SendTo::Mail),
    ];
    if let Some(docs) = documents {
        rows.push(row("Documents", "folder-documents", SendTo::Folder(docs)));
    }
    for d in drives {
        let label = d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| d.display().to_string());
        rows.push(row(
            &label,
            "drive-removable-media",
            SendTo::Folder(d.clone()),
        ));
    }
    rows
}

pub fn session_rows(drives: &[PathBuf]) -> Vec<SendToRow> {
    rows(
        places::user_dir("DESKTOP", "Desktop").as_deref(),
        places::user_dir("DOCUMENTS", "Documents"),
        drives,
    )
}

pub fn mail_argv(paths: &[PathBuf]) -> Vec<OsString> {
    let mut argv = vec![OsString::from("xdg-email")];
    for p in paths {
        argv.push("--attach".into());
        argv.push(p.into());
    }
    argv
}

/// The zip row's command for `paths`, items of one folder whose other names are `existing`: one
/// archive named after the first item and numbered past an existing name. None for an empty
/// selection.
pub fn zip_argv(paths: &[PathBuf], existing: &[String]) -> Option<Vec<OsString>> {
    let zip = names::zip_path(paths.first()?, existing);
    Some(ops::zip_argv(paths, &zip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_come_in_order_and_grey_a_missing_desktop() {
        let desktop = tempfile::tempdir().unwrap();
        let rows = rows(
            Some(desktop.path()),
            Some(PathBuf::from("/home/u/Documents")),
            &[PathBuf::from("/run/media/u/USB")],
        );
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Compressed (zipped) Folder",
                "Desktop (create shortcut)",
                "Mail Recipient",
                "Documents",
                "USB",
            ]
        );
        assert!(rows.iter().all(|r| r.enabled), "every row is live");
        assert_eq!(rows[1].dest, SendTo::DesktopShortcut);
        assert_eq!(rows[4].dest, SendTo::Folder("/run/media/u/USB".into()));
        assert_eq!(rows[4].icon, "drive-removable-media");

        let bare = super::rows(Some(Path::new("/nonexistent/Desktop")), None, &[]);
        assert_eq!(bare.len(), 3);
        assert!(!bare[1].enabled);
        assert!(!super::rows(None, None, &[])[1].enabled);
    }

    #[test]
    fn zip_takes_the_whole_set_into_one_archive_named_after_the_first() {
        let paths: Vec<PathBuf> = vec!["/d/b2.txt".into(), "/d/a1.txt".into(), "/d/c3".into()];
        let argv = zip_argv(&paths, &["b2.zip".into()]).unwrap();
        assert_eq!(
            argv,
            [
                "bsdtar",
                "-a",
                "-cf",
                "/d/b2 (2).zip",
                "-C",
                "/d",
                "--",
                "b2.txt",
                "a1.txt",
                "c3"
            ]
        );
        assert_eq!(zip_argv(&[], &[]), None);
    }

    #[test]
    fn mail_attaches_every_path() {
        let argv = mail_argv(&["/a b".into(), "/c".into()]);
        assert_eq!(argv, ["xdg-email", "--attach", "/a b", "--attach", "/c"]);
    }
}
