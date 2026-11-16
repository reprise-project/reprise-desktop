//! `reprise-explorer --file-manager`: owns `org.freedesktop.FileManager1` for the session and
//! starts an Explorer process for each window or sheet a call asks for.

use reprisebus::filemanager1::{Request, Show};
use reprisebus::instance::Handoff;
use reprisefs::dropdata::file_uri_path;
use reprisefs::netshare::Address;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const FLAG: &str = "--file-manager";
const PROGRAM: &str = "reprise-explorer";
/// The desktop entry whose app scope each window starts in, as a Start menu launch's does.
const APP: &str = "reprise-explorer";

/// Serves until the session bus goes; the process's exit status.
pub(crate) fn serve() -> i32 {
    let (conn, requests) = match reprisebus::filemanager1::serve() {
        Ok(served) => served,
        Err(e) => {
            log::warn!(
                "file manager: cannot own {}: {e}",
                reprisebus::filemanager1::NAME
            );
            return 1;
        }
    };
    loop {
        match requests.recv_timeout(Duration::from_secs(30)) {
            Ok(r) => start(&r),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !conn.is_closed() => {}
            Err(_) => return 0,
        }
    }
}

/// Starts `r`'s processes; the first takes the call's activation token.
fn start(r: &Request) {
    let mut handoff = r.handoff.clone();
    for argv in argvs(r.show, &r.uris) {
        if let Err(e) = repriseapp::spawn_handed(&argv, None, Some(APP), &handoff) {
            log::warn!("file manager: spawn {argv:?}: {e}");
        }
        handoff = Handoff::default();
    }
}

/// The command line of each process `show` of `uris` starts: a window per folder, one per folder
/// of the items to select, a sheet per item. A URI that is neither a local file nor, for a folder,
/// a server address is dropped.
fn argvs(show: Show, uris: &[String]) -> Vec<Vec<OsString>> {
    let argv = |args: Vec<OsString>| [vec![OsString::from(PROGRAM)], args].concat();
    let paths = || {
        uris.iter().filter_map(|u| {
            let path = file_uri_path(u);
            if path.is_none() {
                log::info!("file manager: {u} is not a local file");
            }
            path
        })
    };
    match show {
        Show::Folders => uris
            .iter()
            .filter_map(|u| match file_uri_path(u) {
                Some(p) => Some(p.into_os_string()),
                None => Address::parse(u).map(|a| a.uri().into()),
            })
            .map(|a| argv(vec![a]))
            .collect(),
        Show::Items => {
            let mut groups: Vec<(PathBuf, Vec<OsString>)> = Vec::new();
            for p in paths() {
                let Some(dir) = p.parent().map(Path::to_path_buf) else {
                    groups.push((p.clone(), vec![p.into_os_string()]));
                    continue;
                };
                match groups.iter_mut().find(|(d, _)| *d == dir) {
                    Some((_, items)) => items.push(p.into_os_string()),
                    None => groups.push((dir, vec!["--select".into(), p.into_os_string()])),
                }
            }
            groups.into_iter().map(|(_, args)| argv(args)).collect()
        }
        Show::ItemProperties => paths()
            .map(|p| argv(vec!["--properties".into(), p.into_os_string()]))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(show: Show, uris: &[&str]) -> Vec<String> {
        let uris: Vec<String> = uris.iter().map(|u| u.to_string()).collect();
        argvs(show, &uris)
            .iter()
            .map(|a| {
                a.iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn items_open_one_window_per_folder_with_its_items_selected() {
        assert_eq!(
            lines(
                Show::Items,
                &[
                    "file:///home/u/a%20b.txt",
                    "file:///srv/x",
                    "file://localhost/home/u/c",
                    "https://example.org/d",
                    "file:///",
                ]
            ),
            [
                "reprise-explorer --select /home/u/a b.txt /home/u/c",
                "reprise-explorer --select /srv/x",
                "reprise-explorer /",
            ]
        );
    }

    #[test]
    fn folders_open_a_window_each_and_a_server_address_connects() {
        assert_eq!(
            lines(
                Show::Folders,
                &[
                    "file:///home/u",
                    "smb://nas/public",
                    "--select",
                    "trash:///"
                ]
            ),
            [
                "reprise-explorer /home/u",
                "reprise-explorer smb://nas/public"
            ]
        );
    }

    #[test]
    fn properties_open_a_sheet_per_item() {
        assert_eq!(
            lines(
                Show::ItemProperties,
                &["file:///home/u/a", "file://otherhost/b", "file:///home/u/c"]
            ),
            [
                "reprise-explorer --properties /home/u/a",
                "reprise-explorer --properties /home/u/c",
            ]
        );
    }
}
