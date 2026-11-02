//! The running programs: the user manager's `app-reprise-<id>-<suffix>.scope` cgroups
//! ([`reprisefs::scope`]), each with the processes in it, read from the cgroup tree.

use std::path::{Path, PathBuf};

/// One app scope and its processes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub unit: String,
    /// The desktop entry id it was started from, without `.desktop`.
    pub app: String,
    pub pids: Vec<i32>,
}

/// The app id inside scope unit `unit`, unescaped; None for any other unit. The id is
/// unit-escaped and the suffix is hex fields joined by `x`, so the id ends at the last dash.
pub fn app_of(unit: &str) -> Option<String> {
    let rest = unit.strip_prefix("app-reprise-")?.strip_suffix(".scope")?;
    let (id, suffix) = rest.rsplit_once('-')?;
    if id.is_empty() || suffix.is_empty() {
        return None;
    }
    Some(unescape(id))
}

/// `systemd-escape`'s `\xNN` back to bytes.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 2..i + 4)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (
            b[i],
            b.get(i + 1),
            hex.and_then(|h| u8::from_str_radix(h, 16).ok()),
        ) {
            (b'\\', Some(b'x'), Some(v)) => {
                out.push(v);
                i += 4;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The cgroup directory of the user manager's `app.slice`, from a `/proc/<pid>/cgroup` text of
/// a process under that manager (`0::/user.slice/.../user@1000.service/...`).
pub fn app_slice(cgroup: &str, root: &Path) -> Option<PathBuf> {
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        parts.push(part);
        if part.starts_with("user@") && part.ends_with(".service") {
            return Some(root.join(parts.join("/")).join("app.slice"));
        }
    }
    None
}

/// The app scopes under `slice` that still hold a process, sorted by unit.
pub fn read(slice: &Path) -> Vec<Scope> {
    let Ok(dir) = std::fs::read_dir(slice) else {
        return Vec::new();
    };
    let mut out: Vec<Scope> = dir
        .filter_map(|e| {
            let e = e.ok()?;
            let unit = e.file_name().into_string().ok()?;
            let app = app_of(&unit)?;
            let procs = std::fs::read_to_string(e.path().join("cgroup.procs")).ok()?;
            let pids: Vec<i32> = procs
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .collect();
            (!pids.is_empty()).then_some(Scope { unit, app, pids })
        })
        .collect();
    out.sort_by(|a, b| a.unit.cmp(&b.unit));
    out
}

/// This process's user manager's `app.slice` under the cgroup2 mount.
pub fn own_slice() -> Option<PathBuf> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    app_slice(&cgroup, Path::new("/sys/fs/cgroup"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_unit_names_its_app() {
        assert_eq!(
            app_of("app-reprise-org.kde.ark-1ax2x3.scope").as_deref(),
            Some("org.kde.ark")
        );
        assert_eq!(
            app_of("app-reprise-reprise\\x2dnotepad-1ax2x3.scope").as_deref(),
            Some("reprise-notepad")
        );
        assert_eq!(app_of("app-reprise-firefox-.scope"), None);
        assert_eq!(app_of("app-gnome-firefox-12.scope"), None);
        assert_eq!(app_of("app-reprise-firefox-1a.service"), None);
    }

    #[test]
    fn the_app_slice_sits_under_the_user_manager() {
        let cg =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-x-1.scope\n";
        assert_eq!(
            app_slice(cg, Path::new("/sys/fs/cgroup")),
            Some(PathBuf::from(
                "/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice"
            ))
        );
        assert_eq!(
            app_slice("0::/system.slice/sshd.service\n", Path::new("/")),
            None
        );
    }

    #[test]
    fn scopes_group_their_processes_and_empty_ones_are_left_out() {
        let slice = tempfile::tempdir().unwrap();
        let mk = |unit: &str, procs: &str| {
            let d = slice.path().join(unit);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("cgroup.procs"), procs).unwrap();
        };
        mk("app-reprise-org.kde.ark-1ax2x3.scope", "4242\n4250\n");
        mk("app-reprise-reprise\\x2dnotepad-2ax2x0.scope", "77\n");
        mk("app-reprise-gone-3ax1x0.scope", "");
        mk("dbus.service", "12\n");
        let got = read(slice.path());
        assert_eq!(
            got,
            [
                Scope {
                    unit: "app-reprise-org.kde.ark-1ax2x3.scope".into(),
                    app: "org.kde.ark".into(),
                    pids: vec![4242, 4250],
                },
                Scope {
                    unit: "app-reprise-reprise\\x2dnotepad-2ax2x0.scope".into(),
                    app: "reprise-notepad".into(),
                    pids: vec![77],
                },
            ]
        );
    }
}
