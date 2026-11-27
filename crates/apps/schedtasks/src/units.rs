//! The unit files New Task writes, Edit rewrites and Delete removes: a
//! `reprise-task-<slug>.timer` and `.service` pair in the user manager's own folder
//! ([`config_dir`]), each opening with [`MARKER`]. Edit and Delete take only a pair that is ours
//! by [`ours`]; every other timer is never written or removed.

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
    /// On the hour.
    Hourly,
    /// Every `every` minutes from the hour; `every` divides 60.
    Minutes {
        every: u32,
    },
    AtLogOn,
    /// As [`Schedule::AtLogOn`], skipped when the task already ran since the machine started
    /// ([`stamp_run`]).
    AtStartup,
}

/// systemd's weekday names, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The minutes [`Schedule::Minutes`] repeats at.
pub const EVERY: [u32; 4] = [5, 10, 15, 30];

/// How long after the user manager starts an at-log-on task runs, so the desktop has handed its
/// environment to the manager first.
pub const LOGON_DELAY_S: u64 = 30;

impl Schedule {
    /// The `OnCalendar` spec in the form systemd normalizes it to, `*-*-* 09:30:00`; None for at
    /// log on.
    pub fn calendar(&self) -> Option<String> {
        let time = |(h, m): (u32, u32)| format!("{h:02}:{m:02}:00");
        Some(match *self {
            Schedule::Daily { at } => format!("*-*-* {}", time(at)),
            Schedule::Weekly { day, at } => {
                let day = WEEKDAYS.get(day).copied().unwrap_or("Mon");
                format!("{day} *-*-* {}", time(at))
            }
            Schedule::Monthly { day, at } => format!("*-*-{day:02} {}", time(at)),
            Schedule::Once {
                date: (y, mo, d),
                at,
            } => format!("{y:04}-{mo:02}-{d:02} {}", time(at)),
            Schedule::Hourly => "*-*-* *:00:00".into(),
            Schedule::Minutes { every } => format!("*-*-* *:00/{every}:00"),
            Schedule::AtLogOn | Schedule::AtStartup => return None,
        })
    }

    /// The `[Timer]` line: `OnCalendar=*-*-* 09:30:00`, or `OnStartupSec=` [`LOGON_DELAY_S`]
    /// for both log-on schedules.
    pub fn setting(&self) -> String {
        match self.calendar() {
            Some(spec) => format!("OnCalendar={spec}"),
            None => format!("OnStartupSec={LOGON_DELAY_S}s"),
        }
    }

    /// The schedule whose [`Schedule::setting`] is `setting`; None for any other line. The two
    /// log-on schedules share a line, which reads as [`Schedule::AtLogOn`].
    pub fn parse(setting: &str) -> Option<Schedule> {
        let spec = setting.strip_prefix("OnCalendar=");
        let Some(spec) = spec else {
            return (setting == Schedule::AtLogOn.setting()).then_some(Schedule::AtLogOn);
        };
        let num = |s: &str| s.parse::<u32>().ok();
        let at = |t: &str| {
            let mut it = t.split(':');
            Some((num(it.next()?)?, num(it.next()?)?))
        };
        let s = match spec.split(' ').collect::<Vec<_>>().as_slice() {
            ["*-*-*", "*:00:00"] => Schedule::Hourly,
            ["*-*-*", t] if t.starts_with("*:00/") => Schedule::Minutes {
                every: num(t.strip_prefix("*:00/")?.strip_suffix(":00")?)?,
            },
            [day, "*-*-*", t] => Schedule::Weekly {
                day: WEEKDAYS.iter().position(|d| d == day)?,
                at: at(t)?,
            },
            ["*-*-*", t] => Schedule::Daily { at: at(t)? },
            [date, t] if date.starts_with("*-*-") => Schedule::Monthly {
                day: num(date.strip_prefix("*-*-")?)?,
                at: at(t)?,
            },
            [date, t] => {
                let mut it = date.split('-');
                let (y, mo, d) = (it.next()?, it.next()?, it.next()?);
                Schedule::Once {
                    date: (y.parse().ok()?, num(mo)?, num(d)?),
                    at: at(t)?,
                }
            }
            _ => return None,
        };
        let fits = match s {
            Schedule::Minutes { every } => EVERY.contains(&every),
            Schedule::Daily { at }
            | Schedule::Weekly { at, .. }
            | Schedule::Monthly { at, .. }
            | Schedule::Once { at, .. } => at.0 < 24 && at.1 < 60,
            _ => true,
        };
        (fits && s.calendar().as_deref() == Some(spec)).then_some(s)
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
    /// An at-startup task, with no `reprise-schedtasks` it can run to stamp each run.
    Helper,
}

/// The user's own unit folder in the manager's search path `search` (its `UnitPath`): the first
/// `<config>/systemd/user` listed with its `user.control` beside it, which only
/// `$XDG_CONFIG_HOME` and `$XDG_RUNTIME_DIR` have, the first before the second. A folder under
/// `/run/` is refused: a task written there would be gone at the next boot.
pub fn config_dir(search: &[String]) -> Option<PathBuf> {
    search
        .iter()
        .find(|p| {
            p.starts_with('/')
                && !p.starts_with("/run/")
                && p.ends_with("/systemd/user")
                && search.iter().any(|c| *c == format!("{p}.control"))
        })
        .map(PathBuf::from)
}

/// The user manager's unit folders, from its search path (`UnitPath`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folders {
    /// [`config_dir`]: New Task writes here and Delete removes from here.
    pub own: PathBuf,
    /// Every folder searched. A drop-in folder in any of them applies to a unit of its name.
    pub search: Vec<PathBuf>,
}

impl Folders {
    /// The folders of `search`; None when it has no [`config_dir`].
    pub fn of(search: &[String]) -> Option<Folders> {
        Some(Folders {
            own: config_dir(search)?,
            search: search
                .iter()
                .filter(|p| p.starts_with('/'))
                .map(PathBuf::from)
                .collect(),
        })
    }
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

/// `reprise-task-<slug>`, with `-2`, `-3`... added while any folder of `folders` holds a unit
/// file or drop-in folder of that name, the own folder its enable link, or `taken` (the timers
/// the manager knows) the timer.
pub fn free_stem(folders: &Folders, slug: &str, taken: &[String]) -> String {
    let there = |dir: &Path, name: &str| dir.join(name).symlink_metadata().is_ok();
    let used = |stem: &str| {
        there(&folders.own, &format!("timers.target.wants/{stem}.timer"))
            || std::iter::once(&folders.own)
                .chain(&folders.search)
                .any(|dir| {
                    ["timer", "service", "timer.d", "service.d"]
                        .into_iter()
                        .any(|ext| there(dir, &format!("{stem}.{ext}")))
                })
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
        Some(rest) => home()?.join(rest.trim_start_matches('/')),
        None if typed.contains('/') => PathBuf::from(typed),
        None => std::env::split_paths(path?)
            .filter_map(|dir| normal(&dir.join(typed)))
            .find(|p| runs(p))?,
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

/// `name` as the unit files keep it: trimmed, with no line-continuing `\` at its end.
pub fn kept_name(name: &str) -> &str {
    name.trim()
        .trim_end_matches(|c: char| c == '\\' || c.is_whitespace())
}

/// `name` as a `Description=` value: [`kept_name`] with `%` doubled.
fn description(name: &str) -> Result<String, Problem> {
    if name.chars().any(char::is_control) {
        return Err(Problem::Control);
    }
    Ok(kept_name(name).replace('%', "%%"))
}

/// The argument that has `reprise-schedtasks` tell of a failed run ([`crate::notice`]).
pub const FAILED: &str = "--failed";

/// The argument that has `reprise-schedtasks` stamp an at-startup task's run ([`stamp_run`]).
pub const ONCE_PER_BOOT: &str = "--once-per-boot";

/// The folder of an at-startup task's stamps, in the user manager's state folder (`%S`).
const STAMPS: &str = "%S/reprise/schedtasks";

/// `reprise-schedtasks` as found on `PATH`, which a task's service runs after each run and, at
/// startup, before it.
pub fn notifier() -> Option<PathBuf> {
    find_program("reprise-schedtasks", std::env::var_os("PATH").as_deref())
}

/// What keeps `task` from being written with `notifier` ([`texts`]).
pub fn check(task: &NewTask, notifier: Option<&Path>) -> Result<(), Problem> {
    texts(task, notifier).map(drop)
}

/// The `[Unit]` and `[Service]` lines that run `schedule` once per boot; none for the other
/// schedules. The manager skips the start while the stamp of this boot (`%b`) exists, so only
/// that skips a run, and `notifier` writes the stamp before the run; its failure fails the run.
fn once_per_boot(schedule: Schedule, notifier: Option<&Path>) -> Result<[String; 2], Problem> {
    if schedule != Schedule::AtStartup {
        return Ok(Default::default());
    }
    let stamp = notifier.and_then(|n| exec_start(n, &[ONCE_PER_BOOT.to_string()]).ok());
    let stamp = stamp.ok_or(Problem::Helper)?;
    Ok([
        format!("ConditionPathExists=!{STAMPS}/%N.%b\n"),
        format!("ExecStartPre={stamp} \"{STAMPS}\" \"%N\" \"%b\"\n"),
    ])
}

/// The `.service` and `.timer` files' text for `task`. With a `notifier` the service runs it
/// after each run with [`FAILED`] and the task's name; its failure is ignored. An at-startup
/// task needs one ([`once_per_boot`]).
pub fn texts(task: &NewTask, notifier: Option<&Path>) -> Result<(String, String), Problem> {
    let desc = description(&task.name)?;
    let exec = exec_start(&task.program, &task.args)?;
    let [condition, stamp] = once_per_boot(task.schedule, notifier)?;
    let tell = [FAILED.to_string(), kept_name(&task.name).to_string()];
    let after = notifier
        .and_then(|n| exec_start(n, &tell).ok())
        .map_or_else(String::new, |e| format!("ExecStopPost=-{e}\n"));
    let service = format!(
        "{MARKER}\n[Unit]\nDescription={desc}\n{condition}\n[Service]\nType=oneshot\n{stamp}ExecStart={exec}\n{after}"
    );
    let timer = format!(
        "{MARKER}\n[Unit]\nDescription={desc}\n\n[Timer]\n{}\n\n[Install]\nWantedBy=timers.target\n",
        task.schedule.setting()
    );
    Ok((service, timer))
}

/// Writes `task` into the own folder of `folders` as `<stem>.service` and `<stem>.timer` (made
/// with [`free_stem`]), the service first, so the timer never points at a missing unit. Each file
/// is written beside its place, synced and linked in, which fails rather than replace a file
/// that appeared since. Returns the timer's unit name.
pub fn write(
    folders: &Folders,
    task: &NewTask,
    taken: &[String],
    notifier: Option<&Path>,
) -> io::Result<String> {
    let (service, timer) = texts(task, notifier).map_err(|p| io::Error::other(format!("{p:?}")))?;
    let dir = &folders.own;
    std::fs::create_dir_all(dir)?;
    let stem = free_stem(folders, &slug(&task.name), taken);
    put_new(&dir.join(format!("{stem}.service")), &service)?;
    if let Err(e) = put_new(&dir.join(format!("{stem}.timer")), &timer) {
        let _ = std::fs::remove_file(dir.join(format!("{stem}.service")));
        return Err(e);
    }
    Ok(format!("{stem}.timer"))
}

/// The task the pair [`texts`] wrote for it holds, with or without a notifier: `service` and
/// `timer` are the files' text. None when a line it writes is missing or reads otherwise, as
/// after an edit by hand.
pub fn parse(service: &str, timer: &str) -> Option<NewTask> {
    let value = |text: &str, key: &str| {
        let mut found = text.lines().filter_map(|l| l.strip_prefix(key));
        let v = found.next()?;
        found.next().is_none().then_some(v.to_string())
    };
    let name = value(service, "Description=")?.replace("%%", "%");
    let mut words = unquote(value(service, "ExecStart=")?.strip_prefix(':')?)?.into_iter();
    let program = PathBuf::from(words.next()?);
    let schedule = match Schedule::parse(timer.lines().find(|l| l.starts_with("On"))?)? {
        Schedule::AtLogOn if value(service, "ExecStartPre=").is_some() => Schedule::AtStartup,
        s => s,
    };
    let task = NewTask {
        name,
        program,
        args: words.collect(),
        schedule,
    };
    let notifier = match value(service, "ExecStopPost=-:") {
        Some(line) => Some(PathBuf::from(unquote(&line)?.into_iter().next()?)),
        None => None,
    };
    let written = texts(&task, notifier.as_deref()).ok()?;
    (written == (service.to_string(), timer.to_string())).then_some(task)
}

/// The words of an `ExecStart=` value [`exec_start`] wrote, past its `:`.
fn unquote(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while chars.peek().is_some() {
        if !out.is_empty() && chars.next()? != ' ' {
            return None;
        }
        if chars.next()? != '"' {
            return None;
        }
        let mut w = String::new();
        loop {
            match chars.next()? {
                '"' => break,
                '\\' => w.push(chars.next()?),
                '%' => match chars.next()? {
                    '%' => w.push('%'),
                    _ => return None,
                },
                c => w.push(c),
            }
        }
        out.push(w);
    }
    Some(out)
}

/// The task timer `timer` and its service in `dir` hold ([`parse`]).
pub fn read(dir: &Path, timer: &str) -> Option<NewTask> {
    let stem = timer.strip_suffix(".timer").filter(|s| is_our_stem(s))?;
    let service = std::fs::read_to_string(dir.join(format!("{stem}.service"))).ok()?;
    let timer = std::fs::read_to_string(dir.join(timer)).ok()?;
    parse(&service, &timer)
}

/// Whether `s` is a boot id as systemd's `%b` writes it: 32 hex digits.
fn is_boot(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The `ExecStartPre=` of an at-startup task: stamps boot `boot` (`%b`) for task `stem` in `dir`
/// and drops its stamps of other boots. Refuses arguments the unit [`texts`] wrote never passes.
/// A stamp of another boot that cannot go is logged and does not fail the run, which this boot's
/// stamp already counts.
pub fn stamp_run(dir: &Path, stem: &str, boot: &str) -> io::Result<()> {
    if !dir.is_absolute() || !is_our_stem(stem) || !is_boot(boot) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a stamp: {} {stem} {boot}", dir.display()),
        ));
    }
    std::fs::create_dir_all(dir)?;
    put_over(&dir.join(format!("{stem}.{boot}")), "")?;
    if let Err(e) = forget_runs(dir, stem, Some(boot)) {
        log::warn!("{stem}: a stamp of an earlier boot stays: {e}");
    }
    Ok(())
}

/// Removes task `stem`'s stamps from `dir`, but the one of boot `keep`, so the task runs at its
/// next log on. A folder already gone is no error; one stamp that cannot go does not keep the
/// rest, and the error is the first.
pub fn forget_runs(dir: &Path, stem: &str, keep: Option<&str>) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let mut first = None;
    for entry in entries {
        let removed = entry.and_then(|entry| {
            let name = entry.file_name();
            let boot = name
                .to_str()
                .and_then(|n| n.strip_prefix(stem)?.strip_prefix('.'));
            match boot.is_some_and(|b| is_boot(b) && Some(b) != keep) {
                true => gone(std::fs::remove_file(entry.path())),
                false => Ok(()),
            }
        });
        if let Err(e) = removed {
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// The folder of task `stem`'s stamps, from `condition`: the path its unit's
/// `ConditionPathExists=` resolved to in the manager (its `Conditions`), so the window and the
/// stamping run agree on `%S`.
pub fn stamps_dir<'a>(condition: &'a str, stem: &str) -> Option<&'a Path> {
    let path = Path::new(condition);
    let name = path.file_name()?.to_str()?;
    let boot = name.strip_prefix(stem)?.strip_prefix('.')?;
    let ours = path.is_absolute() && is_our_stem(stem) && is_boot(boot);
    path.parent().filter(|_| ours)
}

/// `args` as the Arguments field shows them, so [`split_args`] reads them back: a word holding a
/// space, a quote or a `\`, or none at all, in single quotes, or double quotes when it holds `'`.
pub fn join_args(args: &[String]) -> String {
    let plain =
        |a: &str| !a.is_empty() && !a.contains(|c: char| c.is_whitespace() || "'\"\\".contains(c));
    args.iter()
        .map(|a| match a {
            a if plain(a) => a.clone(),
            a if !a.contains('\'') => format!("'{a}'"),
            a => format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rewrites timer `timer` and its service in `dir` with `task`'s text, the service first; each
/// file is written beside its place, synced and renamed over it. When the timer cannot be
/// written the service's old text goes back, so the pair never mixes two tasks. The caller has
/// checked the pair is [`ours`].
pub fn rewrite(dir: &Path, timer: &str, task: &NewTask, notifier: Option<&Path>) -> io::Result<()> {
    let (service, timer_text) =
        texts(task, notifier).map_err(|p| io::Error::other(format!("{p:?}")))?;
    let stem = timer.strip_suffix(".timer").filter(|s| is_our_stem(s));
    let stem = stem.ok_or_else(|| io::Error::other("not a task New Task makes"))?;
    let service_path = dir.join(format!("{stem}.service"));
    let was = std::fs::read_to_string(&service_path)?;
    put_over(&service_path, &service)?;
    if let Err(e) = put_over(&dir.join(timer), &timer_text) {
        let _ = put_over(&service_path, &was);
        return Err(e);
    }
    Ok(())
}

fn put_over(path: &Path, text: &str) -> io::Result<()> {
    let (mut f, tmp) = reprisefs::ops::create_temp_beside(path, None)?;
    let done = (|| {
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
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
