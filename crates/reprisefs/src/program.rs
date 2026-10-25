//! Files that run when opened instead of opening in their type's program.

use std::ffi::OsString;
use std::io;
use std::path::Path;

/// What a program file holds ([`program`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// Starts with `#!`.
    Script,
    Binary,
}

/// Whether opening `path` runs it: a regular file with the owner's execute bit that holds a script
/// or an ELF binary. A `.desktop` file never is; it opens as a launcher or a plain file. `meta`
/// follows a symlink. Reads the file's first bytes, so a host asks it off its event loop.
pub fn is_program(path: &Path, meta: &std::fs::Metadata) -> bool {
    program(path, meta).is_some()
}

/// [`is_program`], telling a script from a binary.
pub fn program(path: &Path, meta: &std::fs::Metadata) -> Option<Program> {
    use std::os::unix::fs::PermissionsExt;
    if !meta.is_file()
        || meta.permissions().mode() & 0o100 == 0
        || path.extension().is_some_and(|e| e == "desktop")
    {
        return None;
    }
    let head = head(path, 4).ok()?;
    if head.starts_with(b"#!") {
        Some(Program::Script)
    } else {
        head.starts_with(b"\x7fELF").then_some(Program::Binary)
    }
}

/// Up to the first `n` bytes of the regular file at `path`; the open does not block on a FIFO.
fn head(path: &Path, n: u64) -> io::Result<Vec<u8>> {
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
    let mut bytes = Vec::new();
    file.take(n).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// What a script's terminal does when the script ends: the `script_terminal` key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptEnd {
    /// Waits for Enter after the script, so its output can be read.
    #[default]
    Stay,
    /// Closes when the script succeeds; a failure still waits for Enter.
    Close,
}

impl ScriptEnd {
    pub fn parse(s: &str) -> Option<ScriptEnd> {
        match s {
            "stay" => Some(ScriptEnd::Stay),
            "close" => Some(ScriptEnd::Close),
            _ => None,
        }
    }
}

/// Run by `sh -c` with the script's path as `$0`, so no path is ever part of the shell text.
const STAY: &str = r#""$0"; s=$?; echo; [ "$s" -eq 0 ] || echo "The script ended with status $s."; printf 'Press Enter to close.'; read -r _"#;
/// [`STAY`] that exits when the script succeeds, so a failure is never closed unread. The wrapper
/// leads the terminal's session and its exit hangs up its process group (`$$`), so it first waits
/// up to 2 s for the script's background children to leave the group, as `setsid cmd &` does.
/// The scan reads `/proc` with builtins only: a forked check would itself be in the group.
const CLOSE: &str = r#""$0"; s=$?
if [ "$s" -eq 0 ]; then
    read -r up _ < /proc/uptime
    end=$((${up%.*} * 100 + 1${up#*.} - 100 + 200))
    while :; do
        n=0
        for f in /proc/[0-9]*/stat; do
            { read -r l < "$f"; } 2>/dev/null || continue
            set -- ${l##*") "}
            [ "$3" = "$$" ] && [ "$f" != "/proc/$$/stat" ] && [ "$1" != Z ] && n=1 && break
        done
        read -r up _ < /proc/uptime
        [ "$n" = 0 ] || [ $((${up%.*} * 100 + 1${up#*.} - 100)) -ge "$end" ] && exit 0
        sleep 0.05
    done
fi
echo; echo "The script ended with status $s."; printf 'Press Enter to close.'; read -r _"#;

/// How opening a script runs it: the `script_open` key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptOpen {
    /// As a program of its own, with no window.
    #[default]
    Run,
    /// In `terminal`, which [`ScriptEnd`] keeps open after it.
    Terminal,
}

impl ScriptOpen {
    pub fn parse(s: &str) -> Option<ScriptOpen> {
        match s {
            "run" => Some(ScriptOpen::Run),
            "terminal" => Some(ScriptOpen::Terminal),
            _ => None,
        }
    }

    /// Its row in a script's context menu, which stands in for Open.
    pub fn label(self) -> &'static str {
        match self {
            ScriptOpen::Run => "R&un",
            ScriptOpen::Terminal => "Run in termina&l",
        }
    }
}

/// The rows a program file's context menu shows in place of Open, each with whether it is the
/// default (bold) one: Run and Run in terminal for a script, with `open` the default; `None`
/// (plain Open) for a binary, which always runs bare.
pub fn run_rows(program: Program, open: ScriptOpen) -> Option<[(ScriptOpen, bool); 2]> {
    let both = [ScriptOpen::Run, ScriptOpen::Terminal];
    (program == Program::Script).then(|| both.map(|o| (o, o == open)))
}

/// The argument vector that runs the program at `path`, which must be absolute: a bare name
/// would be looked up on `PATH`. A binary, and a script `open` runs bare, is its own path. A
/// script `open` puts in `terminal` (the `terminal` key) runs there, which `end` keeps open
/// after it; one that cannot start (no execute permission, a noexec mount, a missing
/// interpreter) is an error here, since the terminal would start anyway. Reads the file's first
/// line.
pub fn run_argv(
    path: &Path,
    open: ScriptOpen,
    terminal: &str,
    end: ScriptEnd,
) -> io::Result<Vec<OsString>> {
    let line = head(path, 128)?;
    let Some(rest) = line
        .strip_prefix(b"#!")
        .filter(|_| open == ScriptOpen::Terminal)
    else {
        return Ok(vec![path.into()]);
    };
    executable(path)?;
    let interpreter = rest
        .split(|b| matches!(b, b' ' | b'\t' | b'\n'))
        .find(|t| !t.is_empty());
    if let Some(interpreter) = interpreter {
        use std::os::unix::ffi::OsStrExt;
        executable(Path::new(std::ffi::OsStr::from_bytes(interpreter)))?;
    }
    let wrapper = match end {
        ScriptEnd::Close => CLOSE,
        ScriptEnd::Stay => STAY,
    };
    let argv = vec!["sh".into(), "-c".into(), wrapper.into(), path.into()];
    Ok(crate::apps::in_terminal(terminal, argv))
}

fn executable(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::ENOENT))?;
    // SAFETY: `c` is a NUL-terminated string that outlives the call.
    if unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The error box text for the program `name` when starting it failed with `e`.
pub fn run_failed_text(name: &str, e: &io::Error) -> String {
    let why = match e.raw_os_error() {
        // A missing terminal, interpreter on the `#!` line or binary loader.
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

    fn dir(test: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reprise-program-{test}-{}", std::process::id()));
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

    fn runs(path: &Path) -> Option<Program> {
        program(path, &std::fs::metadata(path).unwrap())
    }

    #[test]
    fn only_an_executable_script_or_elf_file_runs() {
        let d = dir("runs");
        let script = file(&d, "hi.sh", b"#!/bin/sh\necho hi\n", 0o755);
        let elf = file(&d, "tool", b"\x7fELF\x02\x01\x01\0", 0o700);
        let photo = file(&d, "photo.png", b"\x89PNG\r\n\x1a\n", 0o777);
        let plain = file(&d, "plain.sh", b"#!/bin/sh\n", 0o644);
        let group_only = file(&d, "group.sh", b"#!/bin/sh\n", 0o654);
        let launcher = file(&d, "app.desktop", b"#!/usr/bin/env xdg-open\n", 0o755);
        let empty = file(&d, "empty", b"", 0o755);
        std::fs::create_dir(d.join("folder")).unwrap();
        assert_eq!(
            runs(&script),
            Some(Program::Script),
            "a +x script with #! runs"
        );
        assert_eq!(runs(&elf), Some(Program::Binary), "a +x ELF file runs");
        assert!(runs(&photo).is_none(), "a +x picture opens in its program");
        assert!(
            runs(&plain).is_none(),
            "a script without the execute bit opens in its program"
        );
        assert!(
            runs(&group_only).is_none(),
            "only the owner's execute bit counts"
        );
        assert!(
            runs(&launcher).is_none(),
            "a .desktop file keeps its launcher rules"
        );
        assert!(
            runs(&empty).is_none(),
            "an empty +x file opens in its program"
        );
        assert!(
            runs(&d.join("folder")).is_none(),
            "a folder opens in Explorer"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_script_runs_in_the_terminal_and_a_binary_runs_bare() {
        let d = dir("argv").join(r#"it's a "$HOME" dir"#);
        std::fs::create_dir_all(&d).unwrap();
        let script = file(&d, "hi $1 `x`.sh", b"#!/bin/sh\necho hi\n", 0o755);
        let elf = file(&d, "tool", b"\x7fELF\x02\x01\x01\0", 0o755);
        let os = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };
        let p = script.to_str().unwrap();
        let argv =
            |path: &Path, end| run_argv(path, ScriptOpen::Terminal, "foot --title x", end).unwrap();
        assert_eq!(
            argv(&script, ScriptEnd::Close),
            os(&["foot", "--title", "x", "-e", "sh", "-c", CLOSE, p])
        );
        assert_eq!(
            argv(&script, ScriptEnd::Stay),
            os(&["foot", "--title", "x", "-e", "sh", "-c", STAY, p]),
            "the path is an argument, never shell text"
        );
        assert_eq!(argv(&elf, ScriptEnd::Stay), os(&[elf.to_str().unwrap()]));
        assert_eq!(
            run_argv(&script, ScriptOpen::Run, "foot", ScriptEnd::Stay).unwrap(),
            os(&[p]),
            "Run starts the script bare"
        );
        let lost = file(&d, "lost.sh", b"#! /no/such/shell -e\n", 0o755);
        let e = run_argv(&lost, ScriptOpen::Terminal, "foot", ScriptEnd::Stay).unwrap_err();
        assert_eq!(
            e.raw_os_error(),
            Some(libc::ENOENT),
            "a missing interpreter"
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn the_stay_wrapper_runs_the_script_and_waits_for_enter() {
        use std::io::Write;
        let d = dir("stay").join("a b $x 'q\"");
        std::fs::create_dir_all(&d).unwrap();
        let run = |body: &[u8]| {
            let script = file(&d, "s $(id).sh", body, 0o755);
            let mut child = std::process::Command::new("sh")
                .args(["-c", STAY])
                .arg(&script)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"\n").unwrap();
            String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
        };
        assert_eq!(
            run(b"#!/bin/sh\necho hello; exit 3\n"),
            "hello\n\nThe script ended with status 3.\nPress Enter to close."
        );
        assert_eq!(
            run(b"#!/bin/sh\necho \"$0\"\n"),
            format!(
                "{}\n\nPress Enter to close.",
                d.join("s $(id).sh").display()
            ),
            "a clean exit shows no status, and the script gets its own path"
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn the_close_wrapper_waits_only_when_the_script_fails() {
        use std::io::Write;
        let d = dir("close");
        let run = |name: &str, body: &[u8]| {
            let script = file(&d, name, body, 0o755);
            // Enter is queued before the start: a clean exit never reads it and may be gone
            // before a later write, which would fail with EPIPE.
            let (enter, mut key) = std::io::pipe().unwrap();
            key.write_all(b"\n").unwrap();
            drop(key);
            let out = std::process::Command::new("sh")
                .args(["-c", CLOSE])
                .arg(&script)
                .stdin(enter)
                .stderr(std::process::Stdio::null())
                .output()
                .unwrap();
            (String::from_utf8(out.stdout).unwrap(), out.status.code())
        };
        assert_eq!(
            run("ok.sh", b"#!/bin/sh\necho done\n"),
            ("done\n".into(), Some(0)),
            "a clean exit closes without a prompt"
        );
        assert_eq!(
            run("bad.sh", b"#!/bin/sh\nif then\n").0,
            "\nThe script ended with status 2.\nPress Enter to close.",
            "a syntax error"
        );
        assert_eq!(
            run("env.sh", b"#!/usr/bin/env no-such-interpreter-x\n").0,
            "\nThe script ended with status 127.\nPress Enter to close.",
            "a missing `env` interpreter"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_close_wrapper_outlasts_a_setsid_child_until_it_leaves_the_group() {
        use std::os::unix::process::CommandExt;
        let d = dir("close-group");
        let pid_file = d.join("pid");
        let body = format!(
            "#!/bin/sh\nsleep 0.3 &\nsetsid sleep 30 >/dev/null 2>&1 &\necho $! > '{}'\n",
            pid_file.display()
        );
        let script = file(&d, "bg.sh", body.as_bytes(), 0o755);
        let start = std::time::Instant::now();
        let mut child = std::process::Command::new("sh")
            .args(["-c", CLOSE])
            .arg(&script)
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let group = child.id() as libc::pid_t;
        assert!(child.wait().unwrap().success());
        let took = start.elapsed();
        // What a terminal's session leader exiting does to its process group.
        // SAFETY: plain syscalls on pids this test started.
        unsafe { libc::killpg(group, libc::SIGHUP) };
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        // SAFETY: as above.
        let alive = unsafe { libc::kill(pid, 0) == 0 && libc::kill(pid, libc::SIGKILL) == 0 };
        assert!(alive, "the setsid child survives the hangup");
        assert!(
            took >= std::time::Duration::from_millis(300),
            "the wrapper waits for the group's sleep 0.3, took {took:?}"
        );
        assert!(
            took < std::time::Duration::from_secs(2),
            "the wrapper ends once the group is empty, took {took:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_script_menu_offers_run_and_run_in_terminal_with_the_default_bold() {
        use ScriptOpen::{Run, Terminal};
        assert_eq!(
            run_rows(Program::Script, Run),
            Some([(Run, true), (Terminal, false)])
        );
        assert_eq!(
            run_rows(Program::Script, Terminal),
            Some([(Run, false), (Terminal, true)])
        );
        assert_eq!(run_rows(Program::Binary, Run), None, "a binary keeps Open");
        assert_eq!(ScriptOpen::parse("run"), Some(Run));
        assert_eq!(ScriptOpen::parse("terminal"), Some(Terminal));
        assert_eq!(ScriptOpen::parse("Run"), None);
        assert_eq!(ScriptOpen::default(), Run);
    }

    #[test]
    fn script_end_reads_stay_and_close() {
        assert_eq!(ScriptEnd::parse("stay"), Some(ScriptEnd::Stay));
        assert_eq!(ScriptEnd::parse("close"), Some(ScriptEnd::Close));
        assert_eq!(ScriptEnd::parse("Close"), None);
        assert_eq!(ScriptEnd::default(), ScriptEnd::Stay);
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
