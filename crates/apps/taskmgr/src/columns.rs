//! The Processes page's columns. [`SPECS`] is the table View > Select Columns... picks from: a
//! column is one row there, an arm in [`text`] and one in [`sort`].

use std::cmp::Ordering;

use crate::sample::ProcRow;
use crate::{grouped, kb_text};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    Name,
    Pid,
    User,
    Cpu,
    Memory,
    Threads,
    Disk,
    Gpu,
    CommandLine,
}

pub struct Spec {
    pub col: Column,
    /// Its name in the state file.
    pub key: &'static str,
    pub title: &'static str,
    /// Its check box in Select Columns.
    pub label: &'static str,
    /// 100% px.
    pub width: i32,
    pub right: bool,
    pub shown: bool,
    /// A first click on its title sorts it ascending.
    pub ascending: bool,
}

/// Every column in the order they show. Image Name always shows; Command Line comes last, where
/// the list's leftover width goes.
pub const SPECS: [Spec; 9] = [
    Spec {
        col: Column::Name,
        key: "name",
        title: "Image Name",
        label: "&Image Name",
        width: 112,
        right: false,
        shown: true,
        ascending: true,
    },
    Spec {
        col: Column::Pid,
        key: "pid",
        title: "PID",
        label: "&PID (Process Identifier)",
        width: 48,
        right: false,
        shown: true,
        ascending: true,
    },
    Spec {
        col: Column::User,
        key: "user",
        title: "User Name",
        label: "&User Name",
        width: 72,
        right: false,
        shown: true,
        ascending: true,
    },
    Spec {
        col: Column::Cpu,
        key: "cpu",
        title: "CPU",
        label: "&CPU Usage",
        width: 56,
        right: true,
        shown: true,
        ascending: false,
    },
    Spec {
        col: Column::Memory,
        key: "memory",
        title: "Mem Usage",
        label: "&Memory Usage",
        width: 72,
        right: true,
        shown: true,
        ascending: false,
    },
    Spec {
        col: Column::Threads,
        key: "threads",
        title: "Threads",
        label: "&Thread Count",
        width: 56,
        right: true,
        shown: false,
        ascending: false,
    },
    Spec {
        col: Column::Disk,
        key: "disk",
        title: "Disk",
        label: "&Disk Read and Write",
        width: 72,
        right: true,
        shown: false,
        ascending: false,
    },
    Spec {
        col: Column::Gpu,
        key: "gpu",
        title: "GPU",
        label: "&GPU Usage",
        width: 48,
        right: true,
        shown: false,
        ascending: false,
    },
    Spec {
        col: Column::CommandLine,
        key: "command_line",
        title: "Command Line",
        label: "Command &Line",
        width: 200,
        right: false,
        shown: false,
        ascending: true,
    },
];

pub fn spec(col: Column) -> &'static Spec {
    SPECS
        .iter()
        .find(|s| s.col == col)
        .expect("every column has a spec")
}

pub fn by_key(key: &str) -> Option<Column> {
    SPECS.iter().find(|s| s.key == key).map(|s| s.col)
}

pub fn defaults() -> Vec<Column> {
    SPECS.iter().filter(|s| s.shown).map(|s| s.col).collect()
}

/// `picked` in table order, Image Name included whether picked or not.
pub fn shown(picked: impl IntoIterator<Item = Column>) -> Vec<Column> {
    let picked: Vec<Column> = picked.into_iter().collect();
    (SPECS.iter())
        .map(|s| s.col)
        .filter(|c| *c == Column::Name || picked.contains(c))
        .collect()
}

/// What a process with no arguments (a kernel thread, a zombie) shows for its command line: its
/// name in brackets.
pub fn command_line(p: &ProcRow) -> String {
    if p.cmdline.is_empty() {
        format!("[{}]", p.name)
    } else {
        p.cmdline.clone()
    }
}

/// A rate in bytes per second as the Disk column writes it.
pub fn rate_text(bytes: u64) -> String {
    format!("{} KB/s", grouped(bytes / 1024))
}

pub fn text(col: Column, p: &ProcRow) -> String {
    match col {
        Column::Name => p.name.clone(),
        Column::Pid => p.pid.to_string(),
        Column::User => p.user.clone(),
        Column::Cpu => format!("{:02}", p.cpu.round() as u64),
        Column::Memory => kb_text(p.mem_kb),
        Column::Threads => p.threads.to_string(),
        Column::Disk => p.disk.map(rate_text).unwrap_or_default(),
        Column::Gpu => (p.gpu)
            .map(|g| format!("{:02}", (g * 100.0).round() as u64))
            .unwrap_or_default(),
        Column::CommandLine => command_line(p),
    }
}

fn order(col: Column, a: &ProcRow, b: &ProcRow) -> Ordering {
    match col {
        Column::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        Column::Pid => a.pid.cmp(&b.pid),
        Column::User => a.user.cmp(&b.user),
        Column::Cpu => a.cpu.total_cmp(&b.cpu),
        Column::Memory => a.mem_kb.cmp(&b.mem_kb),
        Column::Threads => a.threads.cmp(&b.threads),
        Column::Disk => a.disk.cmp(&b.disk),
        Column::Gpu => (a.gpu.unwrap_or(-1.0)).total_cmp(&b.gpu.unwrap_or(-1.0)),
        Column::CommandLine => command_line(a).cmp(&command_line(b)),
    }
}

/// `rows` by `col`, ascending or not; equal rows by pid.
pub fn sort(rows: &mut [ProcRow], col: Column, ascending: bool) {
    rows.sort_by(|a, b| {
        let o = order(col, a, b);
        let o = if ascending { o } else { o.reverse() };
        o.then(a.pid.cmp(&b.pid))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        rows[0].disk = Some(4096);
        rows[2].disk = Some(100);
        sort(&mut rows, Column::Disk, false);
        assert_eq!(
            pids(&rows),
            [3, 1, 2],
            "an unreadable rate sorts below any rate"
        );
    }

    #[test]
    fn the_gpu_share_is_a_percentage_blank_and_last_where_unreadable() {
        let mut rows = vec![
            row(1, "a", 0.0, 0),
            row(2, "b", 0.0, 0),
            row(3, "c", 0.0, 0),
        ];
        rows[0].gpu = Some(0.054);
        rows[2].gpu = Some(0.0);
        assert_eq!(text(Column::Gpu, &rows[0]), "05");
        assert_eq!(text(Column::Gpu, &rows[1]), "");
        sort(&mut rows, Column::Gpu, false);
        let pids: Vec<i32> = rows.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [1, 3, 2]);
    }

    #[test]
    fn a_process_without_arguments_shows_its_name_in_brackets() {
        let mut p = row(7, "kworker/0:1", 0.0, 0);
        assert_eq!(text(Column::CommandLine, &p), "[kworker/0:1]");
        p.cmdline = "/usr/bin/foot --server".into();
        assert_eq!(text(Column::CommandLine, &p), "/usr/bin/foot --server");
    }

    #[test]
    fn the_disk_rate_is_blank_where_unreadable() {
        let mut p = row(7, "x", 0.0, 0);
        assert_eq!(text(Column::Disk, &p), "");
        p.disk = Some(5 * 1024 * 1024 + 512);
        assert_eq!(text(Column::Disk, &p), "5,120 KB/s");
    }

    #[test]
    fn image_name_always_shows_and_the_rest_keep_the_table_order() {
        assert_eq!(
            shown([Column::CommandLine, Column::Pid]),
            [Column::Name, Column::Pid, Column::CommandLine]
        );
        assert_eq!(
            defaults(),
            [
                Column::Name,
                Column::Pid,
                Column::User,
                Column::Cpu,
                Column::Memory
            ]
        );
        assert!(SPECS.iter().all(|s| by_key(s.key) == Some(s.col)));
    }
}
