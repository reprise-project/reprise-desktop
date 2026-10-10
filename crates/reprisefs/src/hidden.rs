//! A folder's `.hidden` file: the names it hides, one per line, beside the leading dot that hides a
//! name on its own. File managers read the same file.
//!
//! Writes go through [`edit`]: the file is replaced atomically, untouched lines keep their text and
//! order, and a file Reprise created (marked with the `user.reprise.created` xattr) is deleted once it lists nothing.

use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub const FILE: &str = ".hidden";
const FILE_C: &CStr = c".hidden";

/// Largest `.hidden` read or rewritten: a list of every name in a big folder fits.
pub const MAX: u64 = 1 << 20;

const CREATED_XATTR: &CStr = c"user.reprise.created";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed(HashSet<Vec<u8>>);

impl Listed {
    /// What `dir`'s `.hidden` lists; nothing when it has none, cannot be read, is no regular file or
    /// is over [`MAX`] bytes ([`crate::ops::read_regular`]).
    pub fn read(dir: &Path) -> Listed {
        match crate::ops::read_regular(&dir.join(FILE), MAX) {
            Ok(text) => Listed::parse(&text),
            Err(_) => Listed::default(),
        }
    }

    fn parse(text: &[u8]) -> Listed {
        Listed(
            lines(text)
                .into_iter()
                .filter(|l| !l.is_empty())
                .map(<[u8]>::to_vec)
                .collect(),
        )
    }

    pub fn contains(&self, name: &OsStr) -> bool {
        self.0.contains(name.as_bytes())
    }

    pub fn hides(&self, name: &OsStr) -> bool {
        name.as_bytes().starts_with(b".") || self.contains(name)
    }
}

pub fn is_listed(path: &Path) -> bool {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => Listed::read(dir).contains(name),
        _ => false,
    }
}

pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.as_bytes().starts_with(b"."))
        || is_listed(path)
}

/// Whether a name written for an item takes a leading dot with Hidden at `hidden`: the dot stands for
/// Hidden, except on an item its folder's list hides without one (`listed` and not `dotted`), which keeps its name.
pub fn wants_dot(hidden: bool, dotted: bool, listed: bool) -> bool {
    hidden && (dotted || !listed)
}

pub fn unlist(path: &Path) -> std::io::Result<()> {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) if is_listed(path) => edit(dir, &[], &[name]),
        _ => Ok(()),
    }
}

/// Rename or move `from` to `to` with [`crate::trash::move_to_free`], then [`carry`] its line in
/// `.hidden`. Every user rename and move goes through here, so a listed item stays hidden under its
/// new name. A list that cannot be written is logged: the item moved.
pub fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    crate::trash::move_to_free(from, to)?;
    if let Err(e) = carry(from, to) {
        log::warn!(
            "reprisefs: moved {} to {}, but not its .hidden line: {e}",
            from.display(),
            to.display()
        );
    }
    Ok(())
}

/// After `from` was renamed or moved to `to`: when `from`'s folder lists its name, the line goes.
/// Within one folder the new name takes its place; into another folder nothing is added, so a moved
/// item shows unless its new folder's list names it. True when the list named `from`, so an undo can [`list`] it again.
pub fn carry(from: &Path, to: &Path) -> std::io::Result<bool> {
    let (Some(dir), Some(old)) = (from.parent(), from.file_name()) else {
        return Ok(false);
    };
    if !Listed::read(dir).contains(old) {
        return Ok(false);
    }
    let add: Vec<&OsStr> = match (to.parent(), to.file_name()) {
        (Some(d), Some(new)) if d == dir => vec![new],
        _ => Vec::new(),
    };
    edit(dir, &add, &[old]).map(|()| true)
}

pub fn list(path: &Path) -> std::io::Result<()> {
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => edit(dir, &[name], &[]),
        _ => Ok(()),
    }
}

pub fn edit(dir: &Path, add: &[&OsStr], remove: &[&OsStr]) -> std::io::Result<()> {
    let dir = std::fs::File::open(dir)?;
    let add: Vec<&[u8]> = add.iter().map(|n| n.as_bytes()).collect();
    let remove: Vec<&[u8]> = remove.iter().map(|n| n.as_bytes()).collect();
    edit_at(&dir, &add, &remove)
}

/// Add the names `add` that the `.hidden` in the open folder `dir` does not list yet, at its end, and
/// drop every line in `remove`. Other lines keep their text and order. The new text goes to a temporary
/// file renamed over the old one; a list left with no name is deleted when Reprise created it and
/// written empty otherwise. No change writes nothing, and no list is created with nothing to add. A
/// `.hidden` that is a symlink is refused, so its target is never replaced. A name holding a newline is skipped.
pub(crate) fn edit_at(dir: &std::fs::File, add: &[&[u8]], remove: &[&[u8]]) -> std::io::Result<()> {
    let old = read_at(dir)?;
    let text = old.as_ref().map(|o| o.text.as_slice()).unwrap_or_default();
    let mut kept: Vec<&[u8]> = lines(text)
        .into_iter()
        .filter(|l| !remove.contains(l))
        .collect();
    let dropped = kept.len() != lines(text).len();
    let mut added = false;
    for name in add {
        if !name.is_empty() && !name.contains(&b'\n') && !kept.contains(name) {
            kept.push(name);
            added = true;
        }
    }
    if !dropped && !added {
        return Ok(());
    }
    let created = old.as_ref().is_none_or(|o| o.created);
    if kept.iter().all(|l| l.is_empty()) && created {
        return remove_at(dir);
    }
    let mut new = Vec::with_capacity(text.len() + 64);
    for l in &kept {
        new.extend_from_slice(l);
        new.push(b'\n');
    }
    let mode = old.as_ref().map_or(0o644, |o| o.mode);
    replace_at(dir, &new, mode, created)
}

fn lines(text: &[u8]) -> Vec<&[u8]> {
    let mut v: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    if v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v
}

/// An existing `.hidden`: its text, permission bits and whether Reprise created it.
struct Old {
    text: Vec<u8>,
    mode: u32,
    created: bool,
}

/// The `.hidden` in `dir`, or None when there is none. A link, non-regular file or one over [`MAX`]
/// bytes is an error, opened without blocking and never read.
fn read_at(dir: &std::fs::File) -> std::io::Result<Option<Old>> {
    use std::os::unix::fs::PermissionsExt;
    let flags =
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
    // SAFETY: `dir` is an open fd and the name is NUL-terminated.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), FILE_C.as_ptr(), flags) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            Some(libc::ELOOP) => Err(std::io::Error::other("the folder's .hidden is a link")),
            _ => Err(e),
        };
    }
    // SAFETY: `openat` returned a fresh fd that nothing else owns.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let text = crate::ops::read_opened(&file, MAX)?;
    let mode = file.metadata()?.permissions().mode() & 0o7777;
    // SAFETY: the fd is open, the name NUL-terminated; a zero-length
    // query writes nothing.
    let created =
        unsafe { libc::fgetxattr(fd, CREATED_XATTR.as_ptr(), std::ptr::null_mut(), 0) } >= 0;
    Ok(Some(Old {
        text,
        mode,
        created,
    }))
}

fn remove_at(dir: &std::fs::File) -> std::io::Result<()> {
    // SAFETY: `dir` is an open fd and the name is NUL-terminated.
    if unsafe { libc::unlinkat(dir.as_raw_fd(), FILE_C.as_ptr(), 0) } != 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ENOENT) {
            return Err(e);
        }
    }
    Ok(())
}

/// Put `text` in `dir`'s `.hidden` through a temporary file with `mode`, marked Reprise's when `created`, renamed over the old one.
fn replace_at(dir: &std::fs::File, text: &[u8], mode: u32, created: bool) -> std::io::Result<()> {
    let dfd = dir.as_raw_fd();
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut n = 0u32;
    let (tmp, fd) = loop {
        let tmp = CString::new(format!(".hidden.reprise-{}-{n}", std::process::id()))
            .expect("no NUL in the name");
        // SAFETY: `dfd` is an open fd and `tmp` NUL-terminated.
        let fd = unsafe { libc::openat(dfd, tmp.as_ptr(), flags, 0o600) };
        if fd >= 0 {
            break (tmp, fd);
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EEXIST) || n >= 100 {
            return Err(e);
        }
        n += 1;
    };
    // SAFETY: `openat` returned a fresh fd that nothing else owns.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let written = (|| {
        file.write_all(text)?;
        // SAFETY: the fd is open for the call.
        if unsafe { libc::fchmod(fd, mode as libc::mode_t) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if created {
            // A file system without `user.` xattrs keeps the list once emptied: nothing marks it as Reprise's.
            // SAFETY: the fd is open, the name NUL-terminated and the
            // value readable for its length.
            unsafe {
                libc::fsetxattr(fd, CREATED_XATTR.as_ptr(), b"1".as_ptr().cast(), 1, 0);
            }
        }
        file.sync_all()?;
        // SAFETY: `dfd` is open and both names are NUL-terminated.
        if unsafe { libc::renameat(dfd, tmp.as_ptr(), dfd, FILE_C.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })();
    if written.is_err() {
        // SAFETY: as above.
        unsafe { libc::unlinkat(dfd, tmp.as_ptr(), 0) };
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    fn read(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(FILE)).unwrap()
    }

    #[test]
    fn a_listed_name_or_a_leading_dot_hides() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(FILE), "a b\n\nc\nd").unwrap();
        let l = Listed::read(t.path());
        for n in ["a b", "c", "d", ".x"] {
            assert!(l.hides(os(n)), "{n} is hidden");
        }
        assert!(
            !l.hides(os("a")) && !l.hides(os("")),
            "a blank line lists nothing"
        );
        assert!(is_listed(&t.path().join("c")));
        assert!(is_hidden(&t.path().join(".y")) && !is_hidden(&t.path().join("y")));
        assert_eq!(Listed::read(&t.path().join("none")), Listed::default());
    }

    #[test]
    fn a_dot_stands_for_hidden_unless_the_list_hides_the_item() {
        assert!(wants_dot(true, false, false), "hiding puts a dot on");
        assert!(wants_dot(true, true, true), "a dotted name keeps its dot");
        assert!(
            !wants_dot(true, false, true),
            "a listed name keeps its name"
        );
        assert!(!wants_dot(false, true, false), "showing takes the dot off");
    }

    /// An edit keeps untouched lines in order, adds only names not listed yet and writes nothing when nothing moves.
    #[test]
    fn an_edit_keeps_other_lines_in_order() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join(FILE);
        std::fs::write(&file, "zeta\n\nalpha\nmid\n").unwrap();
        edit(t.path(), &[os("new"), os("alpha"), os("new")], &[os("mid")]).unwrap();
        assert_eq!(read(t.path()), "zeta\n\nalpha\nnew\n");
        let before = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        edit(t.path(), &[os("zeta")], &[os("gone")]).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            before
        );
        edit(t.path(), &[os("a\nb")], &[]).unwrap();
        assert_eq!(
            read(t.path()),
            "zeta\n\nalpha\nnew\n",
            "a name with a newline"
        );
        let names: Vec<_> = std::fs::read_dir(t.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file is left");
    }

    /// A list the user made stays, emptied; one Reprise made goes once empty, and none is made for nothing.
    #[test]
    fn only_a_list_reprise_created_is_deleted_when_emptied() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join(FILE);
        edit(t.path(), &[], &[os("a")]).unwrap();
        assert!(!file.exists(), "nothing to add makes no list");
        edit(t.path(), &[os("a"), os("b")], &[]).unwrap();
        assert_eq!(read(t.path()), "a\nb\n");
        edit(t.path(), &[], &[os("a")]).unwrap();
        assert_eq!(read(t.path()), "b\n", "a list with a name left stays");
        edit(t.path(), &[], &[os("b")]).unwrap();
        assert!(!file.exists(), "Reprise's emptied list is deleted");

        std::fs::write(&file, "x\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        edit(t.path(), &[os("y")], &[]).unwrap();
        edit(t.path(), &[], &[os("x"), os("y")]).unwrap();
        assert_eq!(read(t.path()), "", "the user's list stays, emptied");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the list keeps its mode");
    }

    /// A `.hidden` that is a FIFO or a link to a device lists nothing and an edit of it fails; neither blocks nor streams.
    #[test]
    fn a_list_that_is_no_regular_file_lists_nothing_and_is_not_edited() {
        let t = tempfile::tempdir().unwrap();
        let [fifo, zero] = ["fifo", "zero"].map(|n| t.path().join(n));
        for d in [&fifo, &zero] {
            std::fs::create_dir(d).unwrap();
        }
        crate::testfs::mkfifo(&fifo.join(FILE));
        std::os::unix::fs::symlink("/dev/zero", zero.join(FILE)).unwrap();
        for d in [fifo.clone(), zero] {
            assert_eq!(
                crate::testfs::within(move || Listed::read(&d)),
                Listed::default()
            );
        }
        let edited = crate::testfs::within(move || edit(&fifo, &[os("a")], &[]).is_err());
        assert!(edited, "a FIFO list is refused");
    }

    #[test]
    fn a_linked_list_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("real"), "a\n").unwrap();
        std::os::unix::fs::symlink("real", t.path().join(FILE)).unwrap();
        assert!(edit(t.path(), &[os("b")], &[]).is_err());
        assert_eq!(
            std::fs::read_to_string(t.path().join("real")).unwrap(),
            "a\n"
        );
        assert!(is_listed(&t.path().join("a")), "reading follows the link");
    }

    /// A rename in the folder moves the line to the new name; a move to another folder drops it and lists nothing there.
    #[test]
    fn a_listed_item_stays_hidden_under_its_new_name() {
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(t.path().join(n), n).unwrap();
        }
        std::fs::write(t.path().join(FILE), "a\nb\n").unwrap();
        rename(&t.path().join("a"), &t.path().join("A 2")).unwrap();
        assert_eq!(read(t.path()), "b\nA 2\n");
        assert!(is_hidden(&t.path().join("A 2")));
        rename(&t.path().join("c"), &t.path().join("d")).unwrap();
        assert_eq!(
            read(t.path()),
            "b\nA 2\n",
            "an unlisted rename lists nothing"
        );
        rename(&t.path().join("b"), &sub.join("b")).unwrap();
        assert_eq!(read(t.path()), "A 2\n", "a move leaves the old list");
        assert!(!sub.join(FILE).exists(), "and adds to no new one");
        assert!(rename(&t.path().join("gone"), &t.path().join("x")).is_err());
        assert_eq!(read(t.path()), "A 2\n", "a failed rename leaves the list");
    }

    #[test]
    fn unlist_takes_one_name_off() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(FILE), "a\nb\n").unwrap();
        unlist(&t.path().join("a")).unwrap();
        unlist(&t.path().join("c")).unwrap();
        assert_eq!(read(t.path()), "b\n");
    }
}
