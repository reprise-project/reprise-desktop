//! One reading of the machine: every process with its share of the CPU since the last reading,
//! the CPU's and each core's load, memory, and the app scopes. [`Sampler`] keeps what the next
//! reading is measured against.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::procfs::{self, CpuTimes, MemInfo};
use crate::scopes::{self, Scope};

/// A process row. `cpu` is its share of the whole machine since the last reading, in percent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcRow {
    pub pid: i32,
    /// When it started, in clock ticks after boot: with the pid, it names the process.
    pub start: u64,
    pub name: String,
    pub uid: u32,
    pub user: String,
    pub cpu: f64,
    /// Resident memory, in KiB.
    pub mem_kb: u64,
    pub threads: u32,
    pub cmdline: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub procs: Vec<ProcRow>,
    /// The CPU's load across every core, 0 to 1; None on the first reading.
    pub cpu: Option<f64>,
    pub cores: Vec<f64>,
    pub mem: MemInfo,
    pub scopes: Vec<Scope>,
}

impl Snapshot {
    pub fn threads(&self) -> u64 {
        self.procs.iter().map(|p| u64::from(p.threads)).sum()
    }
}

/// The share of `now - prev` not spent idle, 0 to 1.
pub fn busy(prev: CpuTimes, now: CpuTimes) -> f64 {
    let total = now.total.saturating_sub(prev.total);
    if total == 0 {
        return 0.0;
    }
    let idle = now.idle.saturating_sub(prev.idle).min(total);
    1.0 - idle as f64 / total as f64
}

/// A process's share of the machine in percent: its ticks over every core's ticks in the same
/// stretch.
pub fn share(prev_ticks: u64, now_ticks: u64, machine_ticks: u64) -> f64 {
    if machine_ticks == 0 {
        return 0.0;
    }
    let used = now_ticks.saturating_sub(prev_ticks) as f64;
    (used / machine_ticks as f64 * 100.0).min(100.0)
}

/// A process the pid alone does not name once the pid is reused: pid and start time.
type Key = (i32, u64);

pub struct Sampler {
    root: PathBuf,
    slice: Option<PathBuf>,
    cpu: Option<(CpuTimes, Vec<CpuTimes>)>,
    ticks: HashMap<Key, u64>,
    cmdlines: HashMap<Key, String>,
    users: HashMap<u32, String>,
    page_kb: u64,
}

impl Sampler {
    /// Reads processes under `root` (`/proc`) and app scopes under `slice`.
    pub fn new(root: &Path, slice: Option<PathBuf>, users: HashMap<u32, String>) -> Sampler {
        // SAFETY: sysconf only reads a configuration value.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Sampler {
            root: root.to_path_buf(),
            slice,
            cpu: None,
            ticks: HashMap::new(),
            cmdlines: HashMap::new(),
            users,
            page_kb: (page.max(4096) as u64) / 1024,
        }
    }

    /// The machine's processes, users and scopes as they are.
    pub fn system() -> Sampler {
        let users = std::fs::read_to_string("/etc/passwd")
            .map(|s| procfs::parse_passwd(&s))
            .unwrap_or_default();
        let slice = scopes::own_slice();
        if slice.is_none() {
            log::info!("no user manager cgroup: the Applications page stays empty");
        }
        Sampler::new(Path::new("/proc"), slice, users)
    }

    pub fn sample(&mut self) -> Snapshot {
        let cpu = procfs::read_cpu(&self.root);
        let (load, cores, machine) = match (&self.cpu, &cpu) {
            (Some((p, pc)), Some((n, nc))) => (
                Some(busy(*p, *n)),
                pc.iter().zip(nc).map(|(a, b)| busy(*a, *b)).collect(),
                n.total.saturating_sub(p.total),
            ),
            _ => (None, Vec::new(), 0),
        };
        let mut ticks = HashMap::new();
        let mut procs = Vec::new();
        for pid in procfs::pids(&self.root) {
            let Some(p) = procfs::read_process(&self.root, pid) else {
                continue;
            };
            let key = (pid, p.stat.start);
            let prev = self.ticks.get(&key).copied();
            ticks.insert(key, p.stat.ticks);
            let root = &self.root;
            let cmdline = self
                .cmdlines
                .entry(key)
                .or_insert_with(|| procfs::read_cmdline(root, pid))
                .clone();
            procs.push(ProcRow {
                pid,
                start: p.stat.start,
                name: p.stat.comm,
                uid: p.uid,
                user: self
                    .users
                    .get(&p.uid)
                    .cloned()
                    .unwrap_or_else(|| p.uid.to_string()),
                cpu: prev.map_or(0.0, |t| share(t, p.stat.ticks, machine)),
                mem_kb: p.stat.rss_pages * self.page_kb,
                threads: p.stat.threads,
                cmdline,
            });
        }
        self.cmdlines.retain(|k, _| ticks.contains_key(k));
        self.ticks = ticks;
        if cpu.is_some() {
            self.cpu = cpu;
        }
        Snapshot {
            procs,
            cpu: load,
            cores,
            mem: procfs::read_meminfo(&self.root),
            scopes: self.slice.as_deref().map(scopes::read).unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    Pid,
    User,
    Cpu,
    Memory,
}

pub const COLUMNS: [Column; 5] = [
    Column::Name,
    Column::Pid,
    Column::User,
    Column::Cpu,
    Column::Memory,
];

/// `rows` by `col`, ascending or not; equal rows by pid.
pub fn sort(rows: &mut [ProcRow], col: Column, ascending: bool) {
    rows.sort_by(|a, b| {
        let o = match col {
            Column::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Column::Pid => a.pid.cmp(&b.pid),
            Column::User => a.user.cmp(&b.user),
            Column::Cpu => a.cpu.total_cmp(&b.cpu),
            Column::Memory => a.mem_kb.cmp(&b.mem_kb),
        };
        let o = if ascending { o } else { o.reverse() };
        o.then(a.pid.cmp(&b.pid))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(total: u64, idle: u64) -> CpuTimes {
        CpuTimes { total, idle }
    }

    #[test]
    fn load_is_the_share_not_spent_idle_between_two_readings() {
        assert_eq!(busy(t(1000, 800), t(1400, 900)), 0.75);
        assert_eq!(busy(t(1000, 800), t(1000, 800)), 0.0, "no time passed");
        assert_eq!(
            busy(t(1000, 800), t(1100, 950)),
            0.0,
            "idle past total is clamped"
        );
    }

    #[test]
    fn a_process_share_is_of_every_core_together() {
        assert_eq!(share(100, 150, 400), 12.5);
        assert_eq!(share(100, 100, 400), 0.0);
        assert_eq!(
            share(100, 90, 400),
            0.0,
            "a counter going back reads as idle"
        );
        assert_eq!(share(0, 10, 0), 0.0);
    }

    fn stat(pid: i32, comm: &str, ticks: u64, start: u64) -> String {
        format!("{pid} ({comm}) S 1 1 1 0 -1 0 0 0 0 0 {ticks} 0 0 0 20 0 3 0 {start} 1000 250 0")
    }

    fn write(root: &Path, cpu: &str, procs: &[(i32, String)]) {
        std::fs::write(root.join("stat"), cpu).unwrap();
        std::fs::write(
            root.join("meminfo"),
            "MemTotal: 1000 kB\nMemAvailable: 250 kB\n",
        )
        .unwrap();
        for (pid, s) in procs {
            let d = root.join(pid.to_string());
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("stat"), s).unwrap();
            std::fs::write(d.join("cmdline"), format!("prog{pid}\0-x\0")).unwrap();
        }
    }

    #[test]
    fn cpu_shares_come_from_the_ticks_between_two_samples() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        write(
            r,
            "cpu 100 0 0 900 0 0 0 0\ncpu0 50 0 0 450 0 0 0 0\ncpu1 50 0 0 450 0 0 0 0\n",
            &[(10, stat(10, "busy", 100, 5)), (11, stat(11, "idle", 7, 6))],
        );
        let owner = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(r).unwrap());
        let mut s = Sampler::new(r, None, HashMap::from([(owner, "u".into())]));
        let first = s.sample();
        assert_eq!(first.cpu, None, "nothing to measure against yet");
        assert!(first.procs.iter().all(|p| p.cpu == 0.0));
        write(
            r,
            "cpu 300 0 0 1100 0 0 0 0\ncpu0 250 0 0 450 0 0 0 0\ncpu1 50 0 0 650 0 0 0 0\n",
            &[(10, stat(10, "busy", 300, 5)), (11, stat(11, "idle", 7, 6))],
        );
        let next = s.sample();
        assert_eq!(next.cpu, Some(0.5));
        assert_eq!(next.cores, [1.0, 0.0]);
        let busy = next.procs.iter().find(|p| p.pid == 10).unwrap();
        assert_eq!(busy.cpu, 50.0, "200 of the machine's 400 ticks");
        assert_eq!(busy.cmdline, "prog10 -x");
        assert_eq!(busy.mem_kb, 250 * s.page_kb);
        assert_eq!(busy.user, "u");
        assert_eq!(next.mem.used(), 750);
    }

    #[test]
    fn a_reused_pid_starts_over() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let cpu = |n: u64| format!("cpu {n} 0 0 1000 0 0 0 0\n");
        write(r, &cpu(0), &[(10, stat(10, "old", 500, 5))]);
        let mut s = Sampler::new(r, None, HashMap::new());
        s.sample();
        write(r, &cpu(100), &[(10, stat(10, "new", 20, 99))]);
        let p = s.sample().procs.remove(0);
        assert_eq!(p.name, "new");
        assert_eq!(p.cpu, 0.0, "not measured against the old process");
        assert_eq!(
            p.user,
            p.uid.to_string(),
            "an unknown uid reads as its number"
        );
    }

    fn row(pid: i32, name: &str, cpu: f64, mem_kb: u64) -> ProcRow {
        ProcRow {
            pid,
            name: name.into(),
            cpu,
            mem_kb,
            ..Default::default()
        }
    }

    #[test]
    fn rows_sort_by_any_column_and_ties_by_pid() {
        let mut rows = vec![
            row(3, "bash", 2.0, 10),
            row(1, "Xwayland", 2.0, 500),
            row(2, "agent", 9.0, 30),
        ];
        let pids = |r: &[ProcRow]| r.iter().map(|p| p.pid).collect::<Vec<_>>();
        sort(&mut rows, Column::Name, true);
        assert_eq!(pids(&rows), [2, 3, 1], "names compare case-insensitively");
        sort(&mut rows, Column::Cpu, false);
        assert_eq!(pids(&rows), [2, 1, 3], "equal shares fall back to the pid");
        sort(&mut rows, Column::Memory, true);
        assert_eq!(pids(&rows), [3, 2, 1]);
        sort(&mut rows, Column::Pid, false);
        assert_eq!(pids(&rows), [3, 2, 1]);
    }
}
