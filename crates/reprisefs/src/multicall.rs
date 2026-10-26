//! The programs share one installed binary, [`BINARY`]; each program's name in the install
//! directory is a symlink to it, so a process's `/proc/<pid>/exe` names [`BINARY`] and its
//! command line names the program.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub const BINARY: &str = "reprise-apps";

pub const PROGRAMS: [&str; 16] = [
    "reprise-about",
    "reprise-accounts",
    "reprise-bluetooth",
    "reprise-datetime",
    "reprise-drives",
    "reprise-explorer",
    "reprise-network",
    "reprise-notepad",
    "reprise-polkit",
    "reprise-power",
    "reprise-printers",
    "reprise-settings",
    "reprise-sound",
    "reprise-terminal",
    "reprise-updates",
    "reprise-volume",
];

fn program(name: &OsStr) -> Option<&'static str> {
    PROGRAMS.into_iter().find(|p| name == *p)
}

/// The program a command line of [`BINARY`] runs and the arguments it gets: argv[0]'s file name
/// when that is a program, else the first argument.
pub fn split(args: &[OsString]) -> Option<(&'static str, &[OsString])> {
    let first = Path::new(args.first()?).file_name().and_then(program);
    match first {
        Some(p) => Some((p, &args[1..])),
        None => Some((program(args.get(1)?)?, &args[2..])),
    }
}

/// The program `pid` runs when its executable is [`BINARY`], from its command line.
pub fn program_of(pid: i32) -> Option<&'static str> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let exe = exe.as_os_str().as_bytes();
    let exe = exe.strip_suffix(b" (deleted)").unwrap_or(exe);
    if Path::new(OsStr::from_bytes(exe)).file_name()? != BINARY {
        return None;
    }
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    split(&cmdline_args(&cmdline)).map(|(p, _)| p)
}

/// The arguments of a `/proc/<pid>/cmdline`, each ended by a NUL.
fn cmdline_args(cmdline: &[u8]) -> Vec<OsString> {
    let cmdline = cmdline.strip_suffix(b"\0").unwrap_or(cmdline);
    cmdline
        .split(|&b| b == 0)
        .map(|a| OsStr::from_bytes(a).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_link_name_picks_the_program() {
        let a = args(&["/usr/bin/reprise-notepad", "notes.txt"]);
        assert_eq!(split(&a), Some(("reprise-notepad", &a[1..])));
        let a = args(&["reprise-sound", "--tray"]);
        assert_eq!(split(&a), Some(("reprise-sound", &a[1..])));
    }

    #[test]
    fn the_binary_by_its_own_name_takes_the_program_from_the_first_argument() {
        let a = args(&[
            "/usr/bin/reprise-apps",
            "reprise-settings",
            "--page",
            "taskbar",
        ]);
        assert_eq!(split(&a), Some(("reprise-settings", &a[2..])));
        let a = args(&["/proc/self/exe", "reprise-explorer"]);
        assert_eq!(split(&a), Some(("reprise-explorer", &a[2..])));
    }

    #[test]
    fn an_unknown_name_runs_nothing() {
        assert_eq!(split(&args(&["reprise-apps"])), None);
        assert_eq!(split(&args(&["reprise-apps", "notepad"])), None);
        assert_eq!(split(&[]), None);
    }

    #[test]
    fn a_cmdline_splits_on_its_nuls() {
        assert_eq!(
            cmdline_args(b"/usr/bin/reprise-apps\0reprise-polkit\0"),
            args(&["/usr/bin/reprise-apps", "reprise-polkit"])
        );
        assert_eq!(
            split(&cmdline_args(b"reprise-drives\0--tray\0")).map(|(p, _)| p),
            Some("reprise-drives")
        );
    }

    #[test]
    fn a_process_of_another_binary_runs_no_program() {
        assert_eq!(program_of(std::process::id() as i32), None);
    }
}
