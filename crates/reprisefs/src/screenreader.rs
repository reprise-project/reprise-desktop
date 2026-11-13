//! The screen reader: whether Orca is installed, which of this user's processes run it, and asking
//! them to quit. The compositor's `screen-reader` action and Settings both go through here.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub const PROGRAM: &str = "orca";

pub fn installed() -> bool {
    crate::autostart::find_program(PROGRAM).is_some()
}

/// The pids of this user's live screen reader processes.
pub fn running() -> Vec<i32> {
    // SAFETY: getuid has no failure mode.
    let uid = unsafe { libc::getuid() };
    running_in(Path::new("/proc"), uid)
}

/// The processes under `proc` owned by `uid` whose name is [`PROGRAM`], zombies left out.
fn running_in(proc: &Path, uid: u32) -> Vec<i32> {
    let Ok(rd) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    let mut pids: Vec<i32> = rd
        .flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            let dir = e.path();
            if std::fs::metadata(&dir).ok()?.uid() != uid {
                return None;
            }
            let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
            live_reader(&stat).then_some(pid)
        })
        .collect();
    pids.sort_unstable();
    pids
}

/// Whether a `/proc/<pid>/stat` line is a live [`PROGRAM`]: its name sits between the first `(`
/// and the last `)`, the state letter right after.
fn live_reader(stat: &str) -> bool {
    let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
        return false;
    };
    let state = stat[close + 1..].split_whitespace().next();
    &stat[open + 1..close] == PROGRAM && !matches!(state, Some("Z" | "X") | None)
}

/// Ask each of `pids` to quit; the reader says it is off as it goes.
pub fn stop(pids: &[i32]) {
    for &pid in pids {
        // SAFETY: kill only sends a signal; a pid that has exited since `running` fails with ESRCH.
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            log::warn!("screen reader {pid}: {}", std::io::Error::last_os_error());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc_with(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (name, stat) in entries {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("stat"), stat).unwrap();
        }
        root
    }

    #[test]
    fn running_lists_the_live_readers_of_the_user_alone() {
        let root = proc_with(&[
            ("40", "40 (orca) S 1 40 40 0 -1"),
            ("7", "7 (orca) R 1 7 7 0 -1"),
            ("41", "41 (orca) Z 1 41 41 0 -1"),
            ("42", "42 (orca-helper) S 1 42 42 0 -1"),
            ("43", "43 (a (orca)) S 1 43 43 0 -1"),
            ("self", "44 (orca) S 1 44 44 0 -1"),
        ]);
        // SAFETY: getuid has no failure mode.
        let me = unsafe { libc::getuid() };
        assert_eq!(running_in(root.path(), me), [7, 40]);
        assert_eq!(running_in(root.path(), me + 1), Vec::<i32>::new());
    }
}
