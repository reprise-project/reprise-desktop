//! Programs filming with a camera device they opened themselves (`/dev/video*`) rather than
//! through PipeWire, as Chromium and Firefox do by default; `privacy` shows the camera for
//! them as for a PipeWire consumer.
//!
//! A thread keeps inotify watches on every `/dev/video*` node (and on `/dev`, for a camera
//! plugged in later) and, after `SETTLE`, scans `/proc/*/fd` for holders and reports each with
//! its parent chain, so the compositor can name it by a window. PipeWire and WirePlumber are
//! left out; the PipeWire monitor names their consumers. A v4l2loopback node (a virtual camera)
//! counts only once a second process holds it: its feeder alone films nothing.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
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
    let mut last = scan(Path::new("/proc"), Path::new("/sys"));
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
        let now = scan(Path::new("/proc"), Path::new("/sys"));
        if now != last {
            last = now;
            if tx.send(last.clone()).is_err() {
                return Ok(());
            }
        }
    }
}

/// The processes under `proc` (a procfs root) holding a `/dev/video*`
/// node open, by pid, PipeWire's own left out; a loopback node (per `sys`, a
/// sysfs root) only while two or more processes hold it. Processes of other
/// users, whose fds cannot be read, are skipped.
pub fn scan(proc: &Path, sys: &Path) -> Vec<Holder> {
    let Ok(dir) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    let mut held: Vec<(i32, Vec<OsString>)> = dir
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .map(|pid| (pid, camera_nodes(&proc.join(pid.to_string()))))
        .filter(|(_, nodes)| !nodes.is_empty())
        .collect();
    held.sort_unstable();
    let mut holders: HashMap<&OsStr, usize> = HashMap::new();
    for node in held.iter().flat_map(|(_, n)| n) {
        *holders.entry(node).or_default() += 1;
    }
    let counts = |node: &OsStr| holders[node] > 1 || !is_loopback(sys, node);
    let pids: Vec<i32> = held
        .iter()
        .filter(|(_, nodes)| nodes.iter().any(|n| counts(n)))
        .map(|(pid, _)| *pid)
        .collect();
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

/// The `/dev/video*` nodes a process holds, each once.
fn camera_nodes(pid_dir: &Path) -> Vec<OsString> {
    let Ok(fds) = std::fs::read_dir(pid_dir.join("fd")) else {
        return Vec::new();
    };
    let mut nodes: Vec<OsString> = fds
        .flatten()
        .filter_map(|fd| {
            let t = std::fs::read_link(fd.path()).ok()?;
            let name = t.file_name()?;
            (t.parent() == Some(Path::new("/dev")) && is_camera_node(name))
                .then(|| name.to_os_string())
        })
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

/// Whether `/dev/<node>` is a virtual device (v4l2loopback): its sysfs entry
/// sits under `devices/virtual`, where a real camera's sits on its bus.
fn is_loopback(sys: &Path, node: &OsStr) -> bool {
    std::fs::read_link(sys.join("class/video4linux").join(node))
        .is_ok_and(|t| t.components().any(|c| c.as_os_str() == "virtual"))
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

    fn root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("reprise-camdev-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// A fake sysfs with video0 on USB and video9 a loopback node.
    fn sysfs(root: &Path) -> std::path::PathBuf {
        let class = root.join("sys/class/video4linux");
        std::fs::create_dir_all(&class).unwrap();
        let usb = "../../devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/video4linux/video0";
        symlink(usb, class.join("video0")).unwrap();
        symlink(
            "../../devices/virtual/video4linux/video9",
            class.join("video9"),
        )
        .unwrap();
        root.join("sys")
    }

    #[test]
    fn holders_of_a_video_node_are_found_with_their_parents() {
        let root = root("holders");
        let r = root.as_path();
        process(r, 100, "firefox", 1, &["/dev/null"]);
        process(r, 120, "Isolated", 100, &["/dev/video0", "/dev/null"]);
        process(r, 200, "pipewire", 1, &["/dev/video0"]);
        process(r, 300, "foot", 1, &["/dev/pts/0", "/dev/videos"]);
        std::fs::create_dir_all(r.join("self")).unwrap();
        let found = scan(r, &sysfs(r));
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
    fn a_loopback_node_held_by_its_feeder_alone_is_no_camera_in_use() {
        let root = root("loopback");
        let r = root.as_path();
        let sys = sysfs(r);
        process(r, 400, "obs", 1, &["/dev/video9", "/dev/video9"]);
        assert_eq!(scan(r, &sys), [], "the feeder alone");
        process(r, 500, "ffplay", 1, &["/dev/video9"]);
        let exes: Vec<String> = scan(r, &sys).into_iter().map(|h| h.exe).collect();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(exes, ["obs", "ffplay"], "a reader joined the feeder");
    }

    #[test]
    fn the_parent_is_read_after_the_command() {
        assert_eq!(parent("42 (Web Content) S 7 42 42 0"), Some(7));
        assert_eq!(parent("42 (a) b) R 9 1"), Some(9));
        assert_eq!(parent(""), None);
    }
}
