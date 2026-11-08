//! The file operations' tests: the pure planning bits, and the window's operations over a real temporary folder.

use super::*;
use crate::commands::Command;
use crate::dialogs::Dialog;
use crate::place::Place;
use crate::tests::test_ctx;
use reprisebus::mutex::lock;
use reprisefs::apps::AppEntry;
use reprisefs::sendto::SendToRow;
use std::ffi::OsString;

#[path = "fileops_tests/closing.rs"]
mod closing;
#[path = "fileops_tests/creating.rs"]
mod creating;
#[path = "fileops_tests/delete.rs"]
mod delete;
#[path = "fileops_tests/open_with.rs"]
mod open_with;
#[path = "fileops_tests/paste.rs"]
mod paste;
#[path = "fileops_tests/recycle_bin.rs"]
mod recycle_bin;
#[path = "fileops_tests/replace.rs"]
mod replace;
#[path = "fileops_tests/send_to.rs"]
mod send_to;

/// Alt+`c` as the seat delivers it: the access key `chooser_event` reads off `KeyInput`.
fn key_event_alt(c: char) -> repriseapp::Event {
    let utf8 = c.to_string();
    let edit = reprisekit::textedit::key_for(c as u32, false, false, Some(&utf8));
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: c as u32,
        utf8: Some(utf8),
        mods: repriseapp::Mods {
            ctrl: false,
            shift: false,
            alt: true,
        },
        edit,
    })
}

fn app(name: &str, id: &str, mimes: &[&str]) -> AppEntry {
    AppEntry {
        name: name.into(),
        exec_line: format!("/usr/bin/{id} %f"),
        terminal: false,
        id: format!("{id}.desktop"),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: mimes.iter().map(|m| (*m).to_string()).collect(),
        shown: true,
    }
}

/// `$XDG_DATA_HOME` is process-wide, so tests that point the trash at a temporary home take turns,
/// and so do the tests that delete into the scratch one ([`crate::tests::scratch_bin`]), whose
/// bin would otherwise be the other test's.
static TRASH_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Points `$XDG_DATA_HOME` at `dir` and `$XDG_CONFIG_HOME` at `dir/config` (the test's Recycle Bin
/// settings), with no drives (only the home trash is listed), and puts back the scratch home on drop
/// so a test cannot leave the process pointing at a deleted tempdir. Held under [`TRASH_ENV`].
struct DataHome(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl DataHome {
    fn set(dir: &Path) -> Self {
        crate::tests::test_env();
        let vars = [
            ("XDG_DATA_HOME", dir.to_path_buf()),
            ("XDG_CONFIG_HOME", dir.join("config")),
        ];
        let was = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in vars {
            // SAFETY: std serialises its own env access and only `TRASH_ENV` holders read these variables.
            unsafe { std::env::set_var(name, value) };
        }
        reprisefs::bincap::set_config_path(Some(dir.join("config/reprise/config.toml")));
        Self(was)
    }

    fn recycle_bin(&self, keys: &str) {
        let path = reprisefs::bincap::config_path();
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(path, format!("[recycle_bin]\n{keys}")).expect("write");
    }
}

impl Drop for DataHome {
    fn drop(&mut self) {
        crate::tests::scratch_bin();
        for (name, old) in self.0.drain(..) {
            // SAFETY: as in `DataHome::set`; still under `TRASH_ENV`.
            unsafe {
                match old {
                    Some(old) => std::env::set_var(name, old),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// Puts `dir` back to `0o700` on drop, so a failed assertion in the chmod-0500 test cannot leave an undeletable tempdir.
struct Mode0700(PathBuf);

impl Drop for Mode0700 {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

fn key_event_plain(sym: u32, edit: Option<reprisekit::textedit::EditKey>) -> repriseapp::Event {
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit,
    })
}

fn right() -> repriseapp::Event {
    key_event_plain(
        reprisekit::textedit::keysym::RIGHT,
        Some(reprisekit::textedit::EditKey::Right {
            select: false,
            word: false,
        }),
    )
}

fn enter() -> repriseapp::Event {
    key_event_plain(reprisekit::textedit::keysym::RETURN, None)
}

fn asked(ex: &Explorer) -> Option<(&'static str, String)> {
    match &ex.dialog {
        Some(Dialog::Replace(r)) => Some((r.fields.title(), r.fields.name.clone())),
        _ => None,
    }
}
