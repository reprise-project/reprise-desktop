//! The unit files New Task writes and Delete removes: a `reprise-task-<slug>.timer` and
//! `.service` pair in the user manager's own folder ([`config_dir`]), each opening with
//! [`MARKER`]. Delete takes only a pair that is ours by [`is_ours`]; every other timer is
//! never written or removed.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "reprise-task-";
/// The first line of every file written here.
pub const MARKER: &str = "# Written by Reprise Scheduled Tasks, whose Delete removes this file.";
/// How long a slug runs, so a unit name stays well under systemd's 255 bytes.
const SLUG_MAX: usize = 40;
/// What systemd refuses in an `ExecStart=` executable path ("contains special characters"):
/// quotes and `\`, and in newer releases the glob characters.
const SPECIAL: [char; 6] = ['\\', '"', '\'', '*', '?', '['];

/// When a new task runs. Times are (hour, minute) in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    Daily {
        at: (u32, u32),
    },
    /// `day` counts from Monday, 0.
    Weekly {
        day: usize,
        at: (u32, u32),
    },
    /// Day `day` of the month, 1 to 31; a month without it is skipped.
    Monthly {
        day: u32,
        at: (u32, u32),
    },
    /// (year, month, day).
    Once {
        date: (i32, u32, u32),
        at: (u32, u32),
    },
    AtLogOn,
}

/// systemd's weekday names, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl Schedule {
    /// The `[Timer]` line: `OnCalendar=*-*-* 09:30:00`. At log on waits 30 s, so the desktop has
    /// handed its environment to the user manager before the program starts.
    pub fn setting(&self) -> String {
        let time = |(h, m): (u32, u32)| format!("{h:02}:{m:02}:00");
        match *self {
            Schedule::Daily { at } => format!("OnCalendar=*-*-* {}", time(at)),
            Schedule::Weekly { day, at } => {
                let day = WEEKDAYS.get(day).copied().unwrap_or("Mon");
                format!("OnCalendar={day} *-*-* {}", time(at))
            }
            Schedule::Monthly { day, at } => format!("OnCalendar=*-*-{day:02} {}", time(at)),
            Schedule::Once {
                date: (y, mo, d),
                at,
            } => {
                format!("OnCalendar={y:04}-{mo:02}-{d:02} {}", time(at))
            }
            Schedule::AtLogOn => "OnStartupSec=30s".into(),
        }
    }
}

/// A task as New Task makes it. `program` is absolute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTask {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub schedule: Schedule,
}

/// Why a task cannot be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A control character in the name, the program or an argument.
    Control,
    /// The program's path is not UTF-8 or not absolute.
    Program,
    /// The arguments open a quote they never close, or end in a lone `\`.
    Quote,
    /// The program's path holds a character systemd refuses in an executable ([`SPECIAL`]).
    Special,
}

/// The user's own unit folder in the manager's search path `search` (its `UnitPath`): the first
/// `<config>/systemd/user` listed with its `user.control` beside it, which only
/// `$XDG_CONFIG_HOME` and `$XDG_RUNTIME_DIR` have, the first before the second.
pub fn config_dir(search: &[String]) -> Option<PathBuf> {
    search
        .iter()
        .find(|p| {
            p.starts_with('/')
                && p.ends_with("/systemd/user")
                && search.iter().any(|c| *c == format!("{p}.control"))
        })
        .map(PathBuf::from)
}

/// A unit name part made of `name`: ASCII letters and digits in lower case, every other run of
/// characters one `-`, at most [`SLUG_MAX`] long; `task` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(SLUG_MAX);
    let out = out.trim_end_matches('-');
    match out {
        "" => "task".into(),
        s => s.into(),
    }
}

/// Whether `stem` (a unit name without `.timer`) is one [`free_stem`] could have made.
pub fn is_our_stem(stem: &str) -> bool {
    stem.strip_prefix(PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && !rest.starts_with('-')
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// `reprise-task-<slug>`, with `-2`, `-3`... added while `dir` holds a unit file or drop-in
/// folder of that name or `taken` (the timers the manager knows) the timer.
pub fn free_stem(dir: &Path, slug: &str, taken: &[String]) -> String {
    let used = |stem: &str| {
        let file = |ext: &str| dir.join(format!("{stem}.{ext}")).symlink_metadata().is_ok();
        ["timer", "service", "timer.d", "service.d"]
            .into_iter()
            .any(file)
            || taken.iter().any(|t| *t == format!("{stem}.timer"))
    };
    let base = format!("{PREFIX}{slug}");
    (1..)
        .map(|n| match n {
            1 => base.clone(),
            n => format!("{base}-{n}"),
        })
        .find(|s| !used(s))
        .unwrap_or(base)
}

/// `typed` as the program a task runs: a path (`~/` for home) to an executable file, or a bare
/// name found in the folders of `path` (a `PATH` value). A link is kept as it is, so the task
/// follows an update behind it. The path comes back [`normal`].
pub fn find_program(typed: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let runs = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.is_file() && m.permissions().mode() & 0o111 != 0
        })
    };
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let found = match typed.strip_prefix("~/") {
        Some(rest) => home()?.join(rest),
        None if typed.contains('/') => PathBuf::from(typed),
        None => std::env::split_paths(path?)
            .map(|dir| dir.join(typed))
            .find(|p| p.is_absolute() && runs(p))?,
    };
    let found = normal(&found)?;
    runs(&found).then_some(found)
}

/// `path` as systemd takes an executable: absolute, with no `//` or `.` part; None when it holds
/// a `..`, which would have to follow links to resolve.
pub fn normal(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut parts = path.components();
    let mut out = PathBuf::from("/");
    if parts.next() != Some(Component::RootDir) {
        return None;
    }
    for part in parts {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => return None,
        }
    }
    Some(out)
}

/// `text` split into arguments as a shell would: spaces, tabs and newlines part them, `'...'`
/// keeps its text as it is, `"..."` takes `\"` and `\\`, and a backslash outside quotes takes the
/// next character.
pub fn split_args(text: &str) -> Result<Vec<String>, Problem> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => out.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next().ok_or(Problem::Quote)? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next().ok_or(Problem::Quote)? {
                        '"' => break,
                        '\\' => match chars.next().ok_or(Problem::Quote)? {
                            c @ ('"' | '\\') => w.push(c),
                            c => {
                                w.push('\\');
                                w.push(c);
                            }
                        },
                        c => w.push(c),
                    }
                }
            }
            '\\' => {
                let next = chars.next().ok_or(Problem::Quote)?;
                word.get_or_insert_with(String::new).push(next);
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    out.extend(word);
    Ok(out)
}

/// `arg` as one `ExecStart=` word that systemd reads back unchanged: in double quotes, with `\`
/// and `"` escaped and `%` (a specifier) doubled. `$` stays, as [`exec_start`]'s `:` turns
/// variables off.
fn quote(arg: &str) -> Result<String, Problem> {
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            c if c.is_control() => return Err(Problem::Control),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// The `ExecStart=` value running `program` (absolute and [`normal`]) with `args`, each word as
/// typed: the `:` prefix keeps systemd from expanding `$` variables.
pub fn exec_start(program: &Path, args: &[String]) -> Result<String, Problem> {
    let normal = normal(program).filter(|n| n.as_os_str() == program.as_os_str());
    let program = normal.as_deref().and_then(Path::to_str);
    let program = program.ok_or(Problem::Program)?;
    if program.contains(SPECIAL) {
        return Err(Problem::Special);
    }
    let words = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote);
    Ok(format!(
        ":{}",
        words.collect::<Result<Vec<_>, _>>()?.join(" ")
    ))
}

/// `name` as a `Description=` value: trimmed, `%` doubled, no line-continuing `\` at its end.
fn description(name: &str) -> Result<String, Problem> {
    if name.chars().any(char::is_control) {
        return Err(Problem::Control);
    }
    Ok(name.trim().trim_end_matches('\\').replace('%', "%%"))
}

/// The `.service` and `.timer` files' text for `task`.
pub fn texts(task: &NewTask) -> Result<(String, String), Problem> {
    let desc = description(&task.name)?;
    let exec = exec_start(&task.program, &task.args)?;
    let service = format!(
        "{MARKER}\n[Unit]\nDescription={desc}\n\n[Service]\nType=oneshot\nExecStart={exec}\n"
    );
    let timer = format!(
        "{MARKER}\n[Unit]\nDescription={desc}\n\n[Timer]\n{}\n\n[Install]\nWantedBy=timers.target\n",
        task.schedule.setting()
    );
    Ok((service, timer))
}

/// Writes `task` into `dir` as `<stem>.service` and `<stem>.timer` (made with [`free_stem`]),
/// the service first, so the timer never points at a missing unit. Each file is written beside
/// its place, synced and linked in, which fails rather than replace a file that appeared since.
/// Returns the timer's unit name.
pub fn write(dir: &Path, task: &NewTask, taken: &[String]) -> io::Result<String> {
    let (service, timer) = texts(task).map_err(|p| io::Error::other(format!("{p:?}")))?;
    std::fs::create_dir_all(dir)?;
    let stem = free_stem(dir, &slug(&task.name), taken);
    put_new(&dir.join(format!("{stem}.service")), &service)?;
    if let Err(e) = put_new(&dir.join(format!("{stem}.timer")), &timer) {
        let _ = std::fs::remove_file(dir.join(format!("{stem}.service")));
        return Err(e);
    }
    Ok(format!("{stem}.timer"))
}

/// Writes `text` to the new file `path` whole or not at all.
fn put_new(path: &Path, text: &str) -> io::Result<()> {
    let (mut f, tmp) = reprisefs::ops::create_temp_beside(path, None)?;
    let done = (|| {
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::hard_link(&tmp, path)
    })();
    let _ = std::fs::remove_file(&tmp);
    done
}

/// Removes what [`write`] and enabling made for `timer` in `dir`: its `timers.target.wants`
/// link, the service and the timer. A file already gone is no error.
pub fn remove(dir: &Path, timer: &str) -> io::Result<()> {
    let stem = timer.strip_suffix(".timer").filter(|s| is_our_stem(s));
    let stem = stem.ok_or_else(|| io::Error::other("not a task New Task makes"))?;
    let link = dir.join("timers.target.wants").join(timer);
    if link
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        gone(std::fs::remove_file(link))?;
    }
    gone(std::fs::remove_file(dir.join(format!("{stem}.service"))))?;
    gone(std::fs::remove_file(dir.join(timer)))
}

fn gone(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Whether timer unit `timer` and its service in `dir` are ours: the name is one [`free_stem`]
/// makes, the manager loaded the timer from `fragment` there, and both are plain files opening
/// with [`MARKER`]. A service already gone passes, as [`remove`] takes it first.
pub fn ours(dir: &Path, timer: &str, fragment: &str) -> bool {
    let Some(stem) = timer.strip_suffix(".timer").filter(|s| is_our_stem(s)) else {
        return false;
    };
    let [service, timer] = [
        dir.join(format!("{stem}.service")),
        dir.join(format!("{stem}.timer")),
    ];
    let marked = |p: &Path| {
        p.symlink_metadata().is_ok_and(|m| m.file_type().is_file())
            && std::fs::read_to_string(p).is_ok_and(|t| t.lines().next() == Some(MARKER))
    };
    let service_ok = marked(&service)
        || service
            .symlink_metadata()
            .is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
    Path::new(fragment) == timer && marked(&timer) && service_ok
}

#[cfg(test)]
mod tests;
