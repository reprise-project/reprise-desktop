//! The hidden name a copy is built under, `.reprise-<pid>-<start>.part`, and the sweep of parts
//! whose program is gone.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const PREFIX: &str = ".reprise-";
const SUFFIX: &str = ".part";

/// The start time of `pid` in clock ticks since boot (`/proc/<pid>/stat` field 22), which tells a
/// reused pid from the one that made a part. `Err` when the process is gone or cannot be read.
fn start_of(pid: &str) -> std::io::Result<Option<u64>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // The command name may hold spaces and parentheses: fields count from its last `)`.
    let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    Ok(after
        .split_whitespace()
        .nth(19)
        .and_then(|s| s.parse().ok()))
}

/// This process's start time, None when `/proc` cannot tell it.
fn own_start() -> Option<u64> {
    static START: OnceLock<Option<u64>> = OnceLock::new();
    *START.get_or_init(|| start_of("self").ok().flatten())
}

/// The part name for this process. Without a start time it names no owner, and no sweep runs.
pub(super) fn own() -> OsString {
    match own_start() {
        Some(start) => format!("{PREFIX}{}-{start}{SUFFIX}", std::process::id()).into(),
        None => ".reprise.part".into(),
    }
}

/// The pid and start time a part name (numbered too: `.reprise-1-2 (3).part`) holds.
fn owner(name: &OsStr) -> Option<(u32, u64)> {
    let stem = name.to_str()?.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let stem = stem.split_once(" (").map_or(stem, |(s, _)| s);
    let (pid, start) = stem.split_once('-')?;
    Some((pid.parse().ok()?, start.parse().ok()?))
}

/// Whether the program that made a part may still be building it. A process `/proc` will not
/// show for another reason than being gone counts as alive.
fn alive(pid: u32, start: u64) -> bool {
    match start_of(&pid.to_string()) {
        Ok(now) => now.is_none_or(|now| now == start),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Remove the parts in `dir` that a program which is gone left (killed, or the power went), once
/// per folder per process. Only this user's parts with a known owner go.
pub(super) fn sweep(dir: &Path) {
    use std::os::unix::fs::MetadataExt;
    static SWEPT: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    if own_start().is_none() {
        return;
    }
    let first = SWEPT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_default()
        .insert(dir.to_path_buf());
    if !first {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    for entry in entries.flatten() {
        let Some((pid, start)) = owner(&entry.file_name()) else {
            continue;
        };
        let path = entry.path();
        if alive(pid, start) || std::fs::symlink_metadata(&path).map_or(true, |m| m.uid() != uid) {
            continue;
        }
        log::info!(
            "reprisefs: removing {}, left by a copy that never finished",
            path.display()
        );
        if let Err(e) = super::remove_tree(&path) {
            log::warn!("reprisefs: cannot remove {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_name_tells_its_owner_numbered_or_not() {
        assert_eq!(owner(OsStr::new(".reprise-12-345.part")), Some((12, 345)));
        assert_eq!(
            owner(OsStr::new(".reprise-12-345 (2).part")),
            Some((12, 345))
        );
        assert_eq!(owner(OsStr::new(".reprise.part")), None);
        assert_eq!(owner(OsStr::new(".reprise-12.part")), None);
    }

    #[test]
    fn a_folder_copy_removes_parts_left_by_a_gone_program_and_keeps_live_ones() {
        use std::fs;
        use std::process::Command;
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("Photos");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.jpg"), "a").unwrap();
        let usb = t.path().join("usb");
        fs::create_dir(&usb).unwrap();
        let mut done = Command::new("true").spawn().unwrap();
        let gone = done.id();
        done.wait().unwrap();
        let mut live = Command::new("sleep").arg("60").spawn().unwrap();
        let live_start = start_of(&live.id().to_string()).unwrap().unwrap();
        let part = |pid: u32, start: u64| usb.join(format!("{PREFIX}{pid}-{start}{SUFFIX}"));
        let dead = part(gone, 1);
        let reused = part(std::process::id(), own_start().unwrap() + 1);
        let running = part(live.id(), live_start);
        fs::create_dir(&dead).unwrap();
        fs::write(dead.join("a.jpg"), "half").unwrap();
        fs::write(&reused, "half").unwrap();
        fs::create_dir(&running).unwrap();
        fs::write(usb.join(".reprise.part"), "no owner").unwrap();

        let landed = crate::ops::put_in(
            &src,
            "Photos",
            false,
            &usb,
            crate::ops::DropOp::Copy,
            &[],
            &|_| None,
        );
        live.kill().unwrap();
        live.wait().unwrap();
        assert_eq!(landed.unwrap(), "Photos");
        assert!(!dead.exists(), "a dead pid's part is removed");
        assert!(!reused.exists(), "a reused pid's part is removed");
        assert!(running.exists(), "a live program's part is kept");
        assert!(
            usb.join(".reprise.part").exists(),
            "a part with no owner is kept"
        );
        assert_eq!(fs::read_to_string(usb.join("Photos/a.jpg")).unwrap(), "a");
    }
}
