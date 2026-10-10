//! Programs filming with a camera device they opened themselves (`/dev/video*`) rather than
//! through PipeWire, as Chromium and Firefox do by default; `privacy` shows the camera for
//! them as for a PipeWire consumer.
//!
//! A thread keeps inotify watches on every `/dev/video*` node (and on `/dev`, for a camera
//! plugged in later) and, after `SETTLE`, scans `/proc/*/fd` for holders and reports each with
//! its parent chain, so the compositor can name it by a window. PipeWire and WirePlumber are
//! left out; the PipeWire monitor names their consumers.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use inotify::{EventMask, Inotify, WatchMask};
use smithay::reexports::calloop::channel::Sender;

/// How long after an open or a close the thread waits before it looks:
/// a program probing a device opens and closes it a few times in a row.
const SETTLE: Duration = Duration::from_millis(250);
const CHAIN: usize = 8;
const SKIPPED: [&str; 2] = ["pipewire", "wireplumber"];

/// A process holding a camera device open: its pid and its parents'
/// (nearest first), and its program's file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub chain: Vec<i32>,
    pub exe: String,
}

/// Start the thread; it sends the holders each time they change, and
/// once at the start. It runs as long as the compositor does.
pub fn start(tx: Sender<Vec<Holder>>) {
    let spawned = std::thread::Builder::new()
        .name("camera-devices".into())
        .spawn(move || {
            if let Err(e) = watch(&tx) {
                log::info!("privacy: no camera device watch ({e})");
            }
        });
    if let Err(e) = spawned {
        log::warn!("privacy: camera device thread: {e}");
    }
}

fn is_camera_node(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|n| n.strip_prefix("video"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

const DEVICE_MASK: WatchMask = WatchMask::OPEN
    .union(WatchMask::CLOSE_WRITE)
    .union(WatchMask::CLOSE_NOWRITE);

fn watch(tx: &Sender<Vec<Holder>>) -> std::io::Result<()> {
    let mut ino = Inotify::init()?;
    let dev = ino.watches().add("/dev", WatchMask::CREATE)?;
    for entry in std::fs::read_dir("/dev")?.flatten() {
        if is_camera_node(&entry.file_name()) {
            let _ = ino.watches().add(entry.path(), DEVICE_MASK);
        }
    }
    let mut last = scan(Path::new("/proc"));
    let _ = tx.send(last.clone());
    let mut buf = [0u8; 4096];
    loop {
        let mut add = Vec::new();
        for ev in ino.read_events_blocking(&mut buf)? {
            if ev.wd == dev
                && ev.mask.contains(EventMask::CREATE)
                && let Some(name) = ev.name.filter(|n| is_camera_node(n))
            {
                add.push(Path::new("/dev").join(name));
            }
        }
        for path in add {
            let _ = ino.watches().add(path, DEVICE_MASK);
        }
        std::thread::sleep(SETTLE);
        while ino
            .read_events(&mut buf)
            .is_ok_and(|mut e| e.next().is_some())
        {}
        let now = scan(Path::new("/proc"));
        if now != last {
            last = now;
            if tx.send(last.clone()).is_err() {
                return Ok(());
            }
        }
    }
}

/// The processes under `proc` (a procfs root) holding a `/dev/video*`
/// node open, by pid, PipeWire's own left out. Processes of other users,
/// whose fds cannot be read, are skipped.
pub fn scan(proc: &Path) -> Vec<Holder> {
    let Ok(dir) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    let mut pids: Vec<i32> = dir
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| holds_camera(&proc.join(pid.to_string())))
        .collect();
    pids.sort_unstable();
    pids.into_iter()
        .filter_map(|pid| {
            let dir = proc.join(pid.to_string());
            let comm = read_trim(&dir.join("comm"));
            if SKIPPED.contains(&comm.as_str()) {
                return None;
            }
            let exe = std::fs::read_link(dir.join("exe"))
                .ok()
                .and_then(|p| Some(p.file_name()?.to_str()?.to_string()))
                .unwrap_or(comm);
            Some(Holder {
                chain: chain(proc, pid),
                exe,
            })
        })
        .collect()
}

fn holds_camera(pid_dir: &Path) -> bool {
    let Ok(fds) = std::fs::read_dir(pid_dir.join("fd")) else {
        return false;
    };
    fds.flatten().any(|fd| {
        std::fs::read_link(fd.path()).is_ok_and(|t| {
            t.parent() == Some(Path::new("/dev")) && t.file_name().is_some_and(is_camera_node)
        })
    })
}

fn read_trim(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim_end().to_string())
        .unwrap_or_default()
}

/// `pid` and its parents, nearest first, up to `CHAIN` long, stopping
/// before init.
fn chain(proc: &Path, pid: i32) -> Vec<i32> {
    let mut out = vec![pid];
    while out.len() < CHAIN {
        let last = out[out.len() - 1];
        match parent(&read_trim(&proc.join(last.to_string()).join("stat"))) {
            Some(p) if p > 1 => out.push(p),
            _ => break,
        }
    }
    out
}

/// The parent pid from a `/proc/<pid>/stat` line: the fourth field,
/// counted after the parenthesised command, which may hold spaces.
fn parent(stat: &str) -> Option<i32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake procfs process: its comm, exe, parent and fd targets.
    fn process(root: &Path, pid: i32, comm: &str, ppid: i32, fds: &[&str]) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(dir.join("fd")).unwrap();
        std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            dir.join("stat"),
            format!("{pid} ({comm} x) S {ppid} 1 1 0 -1\n"),
        )
        .unwrap();
        symlink(format!("/usr/lib/{comm}/{comm}"), dir.join("exe")).unwrap();
        for (i, t) in fds.iter().enumerate() {
            symlink(t, dir.join("fd").join(i.to_string())).unwrap();
        }
    }

    #[test]
    fn holders_of_a_video_node_are_found_with_their_parents() {
        let root = std::env::temp_dir().join(format!("reprise-camdev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = root.as_path();
        process(r, 100, "firefox", 1, &["/dev/null"]);
        process(r, 120, "Isolated", 100, &["/dev/video0", "/dev/null"]);
        process(r, 200, "pipewire", 1, &["/dev/video0"]);
        process(r, 300, "foot", 1, &["/dev/pts/0", "/dev/videos"]);
        std::fs::create_dir_all(r.join("self")).unwrap();
        let found = scan(r);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            found,
            vec![Holder {
                chain: vec![120, 100],
                exe: "Isolated".into()
            }]
        );
    }

    #[test]
    fn the_parent_is_read_after_the_command() {
        assert_eq!(parent("42 (Web Content) S 7 42 42 0"), Some(7));
        assert_eq!(parent("42 (a) b) R 9 1"), Some(9));
        assert_eq!(parent(""), None);
    }
}
