//! The New menus' templates: the visible files in `XDG_TEMPLATES_DIR`, each made as a copy under
//! its own name.

use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub path: PathBuf,
    /// The file name without its extension, as the menu row reads.
    pub label: String,
}

/// `XDG_TEMPLATES_DIR`; None without a home, or when it is the home folder, which is how
/// xdg-user-dirs turns a folder off.
pub fn dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    unless_home(crate::places::user_dir("TEMPLATES", "Templates")?, &home)
}

fn unless_home(dir: PathBuf, home: &Path) -> Option<PathBuf> {
    (dir != home).then_some(dir)
}

/// The templates in `dir` by label: its files and links to files, without hidden names. Folders
/// are left out. A missing or unreadable `dir` has none.
pub fn list(dir: &Path) -> Vec<Template> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Template> = read
        .flatten()
        .filter(|e| !e.file_name().as_encoded_bytes().starts_with(b"."))
        .filter(|e| e.path().is_file())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Template {
                path: e.path(),
                label: label(&name).to_string(),
            }
        })
        .collect();
    found.sort_by_cached_key(|t| (t.label.to_lowercase(), t.path.clone()));
    found
}

/// `name` without its extension, split as [`crate::names::numbered`] splits it.
fn label(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !name[dot..].contains(' ') => &name[..dot],
        _ => name,
    }
}

/// A copy of `template` in `dir` named after it, numbered past a taken name ("Report (2).odt").
/// Only a regular file is copied, opened without blocking on a FIFO. The copy keeps the mode a new
/// file gets, writable by its owner, plus the template's execute bits where it is readable. A copy
/// that fails part way is removed.
pub fn make(template: &Path, dir: &Path) -> io::Result<PathBuf> {
    let mut from = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(template)?;
    let meta = from.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the template is not a file",
        ));
    }
    let name = template.file_name().unwrap_or_default().to_string_lossy();
    let (mut to, path) = crate::ops::create_new_numbered(dir, &name)?;
    let copied = io::copy(&mut from, &mut to).and_then(|_| {
        let mode = (to.metadata()?.mode() & 0o777) | 0o600;
        let exec = meta.mode() & ((mode & 0o444) >> 2);
        to.set_permissions(std::fs::Permissions::from_mode(mode | exec))
    });
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn templates_list_visible_files_by_label_without_extensions() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("Report.odt"), "r").unwrap();
        fs::write(t.path().join("blank"), "").unwrap();
        fs::write(t.path().join("Mr. Smith"), "").unwrap();
        fs::write(t.path().join(".hidden.txt"), "").unwrap();
        fs::create_dir(t.path().join("Folder")).unwrap();
        let labels: Vec<String> = list(t.path()).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["blank", "Mr. Smith", "Report"]);
        assert!(list(&t.path().join("missing")).is_empty());
    }

    #[test]
    fn a_template_is_copied_under_its_name_and_numbered_past_a_taken_one() {
        let t = tempfile::tempdir().unwrap();
        let template = t.path().join("Report.odt");
        fs::write(&template, "body").unwrap();
        let dest = t.path().join("dest");
        fs::create_dir(&dest).unwrap();
        assert_eq!(make(&template, &dest).unwrap(), dest.join("Report.odt"));
        assert_eq!(make(&template, &dest).unwrap(), dest.join("Report (2).odt"));
        assert_eq!(
            fs::read_to_string(dest.join("Report (2).odt")).unwrap(),
            "body"
        );
    }

    #[test]
    fn a_copy_is_writable_by_its_owner_and_keeps_only_the_execute_bits() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        let (read_only, program) = (t.path().join("Form.txt"), t.path().join("run.sh"));
        fs::write(&read_only, "").unwrap();
        fs::write(&program, "").unwrap();
        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o4755)).unwrap();
        let new_file = mode(&crate::ops::create_new_numbered(&dest, "plain").unwrap().1);

        let form = make(&read_only, &dest).unwrap();
        assert_eq!(mode(&form), new_file | 0o600, "a new file's mode, writable");
        let script = make(&program, &dest).unwrap();
        assert_eq!(mode(&script) & 0o7000, 0, "no setuid, setgid or sticky bit");
        assert_eq!(mode(&script) & 0o100, 0o100, "the owner may run it");
    }

    #[test]
    fn a_template_that_is_not_a_file_is_refused_without_waiting() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let fifo = t.path().join("pipe");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        let folder = t.path().join("linked");
        std::os::unix::fs::symlink(&dest, &folder).unwrap();

        for template in [fifo, folder] {
            let err = make(&template, &dest).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{template:?}");
        }
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0, "no copy left");
    }

    #[test]
    fn a_templates_folder_set_to_home_turns_templates_off() {
        let home = Path::new("/home/u");
        for line in [
            "XDG_TEMPLATES_DIR=\"$HOME\"",
            "XDG_TEMPLATES_DIR=\"$HOME/\"",
        ] {
            let dir = crate::places::parse_user_dir(line, home, "TEMPLATES").unwrap();
            assert_eq!(unless_home(dir, home), None, "{line}");
        }
        let dir = crate::places::parse_user_dir(
            "XDG_TEMPLATES_DIR=\"$HOME/Templates\"",
            home,
            "TEMPLATES",
        )
        .unwrap();
        assert_eq!(unless_home(dir, home), Some(home.join("Templates")));
    }
}
