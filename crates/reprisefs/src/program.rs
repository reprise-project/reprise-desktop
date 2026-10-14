//! Files that run when opened instead of opening in their type's program.

use std::io;
use std::path::Path;

/// Whether `head`, a file's first bytes, starts a script (`#!`) or an ELF binary.
pub fn is_program_header(head: &[u8]) -> bool {
    head.starts_with(b"#!") || head.starts_with(b"\x7fELF")
}

/// Whether opening `path` runs it: a regular file with the owner's execute bit that holds a script
/// or an ELF binary. A `.desktop` file never is; it opens as a launcher or a plain file. `meta`
/// follows a symlink. Reads the file's first bytes, so a host asks it off its event loop.
pub fn is_program(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file()
        && meta.permissions().mode() & 0o100 != 0
        && path.extension().is_none_or(|e| e != "desktop")
        && head(path).is_ok_and(|h| is_program_header(&h))
}

/// Up to the first four bytes of the regular file at `path`; the open does not block on a FIFO.
fn head(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(4);
    file.take(4).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The argument vector that runs the program at `path`, which must be absolute: a bare name
/// would be looked up on `PATH`.
pub fn run_argv(path: &Path) -> Vec<std::ffi::OsString> {
    vec![path.into()]
}

/// The error box text for the program `name` when starting it failed with `e`.
pub fn run_failed_text(name: &str, e: &io::Error) -> String {
    let why = match e.raw_os_error() {
        // A missing interpreter on the `#!` line, or a binary's missing loader.
        Some(libc::ENOENT) => "The program that runs it was not found.".to_string(),
        // A mount that does not run programs, or no execute permission.
        Some(libc::EACCES) => "Access is denied.".to_string(),
        Some(libc::ENOEXEC) => "It is not a program this computer can run.".to_string(),
        _ => e.to_string(),
    };
    format!("Cannot run {name}: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("reprise-program-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn file(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    fn runs(path: &Path) -> bool {
        is_program(path, &std::fs::metadata(path).unwrap())
    }

    #[test]
    fn only_an_executable_script_or_elf_file_runs() {
        let d = dir();
        let script = file(&d, "hi.sh", b"#!/bin/sh\necho hi\n", 0o755);
        let elf = file(&d, "tool", b"\x7fELF\x02\x01\x01\0", 0o700);
        let photo = file(&d, "photo.png", b"\x89PNG\r\n\x1a\n", 0o777);
        let plain = file(&d, "plain.sh", b"#!/bin/sh\n", 0o644);
        let group_only = file(&d, "group.sh", b"#!/bin/sh\n", 0o654);
        let launcher = file(&d, "app.desktop", b"#!/usr/bin/env xdg-open\n", 0o755);
        let empty = file(&d, "empty", b"", 0o755);
        std::fs::create_dir(d.join("folder")).unwrap();
        assert!(runs(&script), "a +x script with #! runs");
        assert!(runs(&elf), "a +x ELF file runs");
        assert!(!runs(&photo), "a +x picture opens in its program");
        assert!(
            !runs(&plain),
            "a script without the execute bit opens in its program"
        );
        assert!(!runs(&group_only), "only the owner's execute bit counts");
        assert!(!runs(&launcher), "a .desktop file keeps its launcher rules");
        assert!(!runs(&empty), "an empty +x file opens in its program");
        assert!(!runs(&d.join("folder")), "a folder opens in Explorer");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_start_says_why() {
        let e = |n| io::Error::from_raw_os_error(n);
        assert_eq!(
            run_failed_text("hi.sh", &e(libc::ENOENT)),
            "Cannot run hi.sh: The program that runs it was not found."
        );
        assert_eq!(
            run_failed_text("hi.sh", &e(libc::EACCES)),
            "Cannot run hi.sh: Access is denied."
        );
        assert_eq!(
            run_failed_text("tool", &e(libc::ENOEXEC)),
            "Cannot run tool: It is not a program this computer can run."
        );
    }
}
