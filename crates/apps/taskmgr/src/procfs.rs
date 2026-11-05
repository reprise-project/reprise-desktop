//! What `/proc` says of the processes, the CPU time counters and memory. The parsers take the
//! files' text; [`read_process`] and friends read them from a root, `/proc` outside tests.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// One process's `stat` line, the fields a task manager shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub pid: i32,
    pub comm: String,
    pub state: char,
    /// User and system time, in clock ticks.
    pub ticks: u64,
    pub threads: u32,
    /// When it started, in clock ticks after boot: with the pid, it names one process.
    pub start: u64,
    pub rss_pages: u64,
}

/// `/proc/<pid>/stat`. The name sits in parentheses and may hold spaces and parentheses
/// itself, so the fields count from its last `)`.
pub fn parse_stat(s: &str) -> Option<Stat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let pid = s[..open].trim().parse().ok()?;
    let comm = s.get(open + 1..close)?.to_string();
    let f: Vec<&str> = s[close + 1..].split_whitespace().collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        pid,
        comm,
        state: f.first()?.chars().next()?,
        ticks: num(11)? + num(12)?,
        threads: num(17)? as u32,
        start: num(19)?,
        rss_pages: num(21)?,
    })
}

/// One `cpu` line of `/proc/stat`: every tick counted, and those spent idle or waiting on I/O.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
}

/// `/proc/stat`'s `cpu` line, and its `cpuN` lines in order.
pub fn parse_cpu(s: &str) -> Option<(CpuTimes, Vec<CpuTimes>)> {
    let mut all = None;
    let mut cores = Vec::new();
    for line in s.lines() {
        let mut f = line.split_whitespace();
        let Some(name) = f.next().filter(|n| n.starts_with("cpu")) else {
            continue;
        };
        // guest and guest_nice are already inside user and nice.
        let v: Vec<u64> = f.take(8).filter_map(|x| x.parse().ok()).collect();
        if v.len() < 5 {
            continue;
        }
        let t = CpuTimes {
            total: v.iter().sum(),
            idle: v[3] + v[4],
        };
        if name == "cpu" {
            all = Some(t);
        } else {
            cores.push(t);
        }
    }
    Some((all?, cores))
}

/// `/proc/meminfo`'s figures a task manager shows, in KiB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl MemInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

pub fn parse_meminfo(s: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in s.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let v = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        match key {
            "MemTotal" => m.total = v,
            "MemAvailable" => m.available = v,
            "Cached" => m.cached = v,
            "SwapTotal" => m.swap_total = v,
            "SwapFree" => m.swap_free = v,
            _ => {}
        }
    }
    m
}

/// `/proc/<pid>/cmdline`: the arguments joined by spaces.
pub fn parse_cmdline(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    let args: Vec<&str> = s.split('\0').filter(|a| !a.is_empty()).collect();
    args.join(" ")
}

/// `/etc/passwd`'s user names by uid.
pub fn parse_passwd(s: &str) -> HashMap<u32, String> {
    s.lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let uid = f.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

/// A process as read: its stat line and its owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Process {
    pub stat: Stat,
    pub uid: u32,
}

/// The numbered directories of `root`.
pub fn pids(root: &Path) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

/// Process `pid` under `root`; None once it is gone.
pub fn read_process(root: &Path, pid: i32) -> Option<Process> {
    let dir = root.join(pid.to_string());
    let uid = std::fs::metadata(&dir).ok()?.uid();
    let stat = parse_stat(&std::fs::read_to_string(dir.join("stat")).ok()?)?;
    Some(Process { stat, uid })
}

pub fn read_cmdline(root: &Path, pid: i32) -> String {
    std::fs::read(root.join(pid.to_string()).join("cmdline"))
        .map(|b| parse_cmdline(&b))
        .unwrap_or_default()
}

/// `/proc/<pid>/io`: the bytes it read from and wrote to storage, together. None when a field is
/// missing.
pub fn parse_io(s: &str) -> Option<u64> {
    let field = |name: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    Some(field("read_bytes")? + field("write_bytes")?)
}

/// [`parse_io`] of process `pid`; None where the file is unreadable, as another user's is.
pub fn read_io(root: &Path, pid: i32) -> Option<u64> {
    parse_io(&std::fs::read_to_string(root.join(pid.to_string()).join("io")).ok()?)
}

pub fn read_cpu(root: &Path) -> Option<(CpuTimes, Vec<CpuTimes>)> {
    parse_cpu(&std::fs::read_to_string(root.join("stat")).ok()?)
}

pub fn read_meminfo(root: &Path) -> MemInfo {
    std::fs::read_to_string(root.join("meminfo"))
        .map(|s| parse_meminfo(&s))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const STAT: &str = "4242 (Web Content (x)) S 1 4242 4242 0 -1 4194560 \
        100 0 0 0 250 50 0 0 20 0 7 0 98765 123456789 3000 18446744073709551615 1 1 0 0 0 0 0 \
        4096 0 0 0 0 17 3 0 0 0 0 0";

    #[test]
    fn a_stat_line_reads_past_a_name_with_spaces_and_parentheses() {
        let s = parse_stat(STAT).unwrap();
        assert_eq!(s.pid, 4242);
        assert_eq!(s.comm, "Web Content (x)");
        assert_eq!(s.state, 'S');
        assert_eq!(s.ticks, 300, "utime 250 + stime 50");
        assert_eq!(s.threads, 7);
        assert_eq!(s.start, 98765);
        assert_eq!(s.rss_pages, 3000);
        assert_eq!(parse_stat("12 (cut"), None);
    }

    #[test]
    fn the_cpu_lines_count_iowait_as_idle_and_leave_out_guest_time() {
        let text = "cpu  100 5 50 800 20 3 2 0 40 0\n\
                    cpu0 60 5 25 400 10 2 1 0 40 0\n\
                    cpu1 40 0 25 400 10 1 1 0 0 0\n\
                    intr 12345\n";
        let (all, cores) = parse_cpu(text).unwrap();
        assert_eq!(
            all,
            CpuTimes {
                total: 980,
                idle: 820
            }
        );
        assert_eq!(cores.len(), 2);
        assert_eq!(cores[1].idle, 410);
        assert_eq!(parse_cpu("intr 1\n"), None);
    }

    #[test]
    fn meminfo_reads_memory_and_swap() {
        let text = "MemTotal:       16000000 kB\nMemFree:  1000 kB\n\
                    MemAvailable:   12000000 kB\nCached:  3000000 kB\n\
                    SwapTotal:       8000000 kB\nSwapFree:        6000000 kB\n";
        let m = parse_meminfo(text);
        assert_eq!(m.used(), 4_000_000);
        assert_eq!(m.cached, 3_000_000);
        assert_eq!(m.swap_used(), 2_000_000);
    }

    #[test]
    fn a_command_line_joins_its_arguments() {
        assert_eq!(
            parse_cmdline(b"/usr/bin/foo\0--bar\0x y\0"),
            "/usr/bin/foo --bar x y"
        );
        assert_eq!(parse_cmdline(b""), "");
    }

    #[test]
    fn io_adds_the_bytes_read_and_written_to_storage() {
        let text = "rchar: 900\nwchar: 800\nsyscr: 3\nsyscw: 2\nread_bytes: 4096\n\
                    write_bytes: 8192\ncancelled_write_bytes: 0\n";
        assert_eq!(parse_io(text), Some(12288));
        assert_eq!(parse_io("rchar: 900\n"), None);
    }

    #[test]
    fn passwd_names_users_by_uid() {
        let m = parse_passwd("root:x:0:0::/root:/bin/bash\nu:x:1000:1000:U:/home/u:/bin/fish\n");
        assert_eq!(m[&1000], "u");
        assert_eq!(m[&0], "root");
    }

    #[test]
    fn a_process_is_read_from_its_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("4242");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("stat"), STAT).unwrap();
        std::fs::write(dir.join("cmdline"), b"firefox\0-contentproc\0").unwrap();
        std::fs::create_dir(root.path().join("self")).unwrap();
        assert_eq!(pids(root.path()), [4242]);
        let p = read_process(root.path(), 4242).unwrap();
        assert_eq!(p.stat.comm, "Web Content (x)");
        assert_eq!(read_cmdline(root.path(), 4242), "firefox -contentproc");
        assert_eq!(read_process(root.path(), 1), None);
    }
}
