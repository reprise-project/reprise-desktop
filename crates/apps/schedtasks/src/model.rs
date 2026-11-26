//! A user timer as the window shows it, and the text of each column.

use reprisekit::tr;

/// systemd's `USEC_INFINITY`: no time set.
pub const NEVER: u64 = u64::MAX;

/// One timer unit of the user manager and the unit it starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    /// The timer unit, `backup.timer`.
    pub timer: String,
    /// The unit the timer starts, `backup.service`.
    pub service: String,
    /// The started unit's `Description`, which systemd sets to its name when the file has none.
    pub description: String,
    /// The timer's `ActiveState` and `UnitFileState`.
    pub timer_state: String,
    pub file_state: String,
    /// The started unit's `ActiveState`; empty when it is not loaded.
    pub service_state: String,
    /// `TimersCalendar`: each setting's name (`OnCalendar`) and its normalized spec.
    pub calendar: Vec<(String, String)>,
    /// `TimersMonotonic`: each setting's name (`OnBootUSec`) and its span in µs.
    pub monotonic: Vec<(String, u64)>,
    /// Wall-clock µs since the epoch of the next and the last run.
    pub next: Option<u64>,
    pub last: Option<u64>,
    /// The started service's `Result`, `ExecMainCode` (a `CLD_*` code, 0 before a run) and
    /// `ExecMainStatus`.
    pub result: String,
    pub exit_code: i32,
    pub exit_status: i32,
    /// The service's `ExecStart` command lines, one per line.
    pub command: String,
    /// The timer's unit file.
    pub file: String,
    /// New Task wrote the timer and its service, so Delete may remove them ([`crate::units::ours`]).
    pub ours: bool,
}

impl Task {
    /// The service's description, else the timer's name without `.timer`.
    pub fn name(&self) -> &str {
        match self.description.as_str() {
            "" => self.stem(),
            d if d == self.service => self.stem(),
            d => d,
        }
    }

    pub fn stem(&self) -> &str {
        self.timer.strip_suffix(".timer").unwrap_or(&self.timer)
    }

    /// The timer is started, so it runs its unit when it elapses.
    pub fn on(&self) -> bool {
        matches!(
            self.timer_state.as_str(),
            "active" | "activating" | "reloading"
        )
    }

    pub fn running(&self) -> bool {
        matches!(
            self.service_state.as_str(),
            "active" | "activating" | "deactivating" | "reloading"
        )
    }

    /// Enable and Disable can act: a masked timer starts for nobody.
    pub fn can_toggle(&self) -> bool {
        !self.file_state.starts_with("masked")
    }

    /// Run Now can start the unit: it is loaded and not running.
    pub fn can_run(&self) -> bool {
        !self.service_state.is_empty() && !self.running()
    }

    pub fn status(&self) -> String {
        if !self.on() {
            tr!("status-disabled")
        } else if self.running() {
            tr!("status-running")
        } else if self.service_state == "failed" && self.result == "signal" {
            tr!("status-stopped")
        } else if self.service_state == "failed" {
            tr!("status-failed")
        } else {
            tr!("status-ready")
        }
    }

    /// The schedule as a sentence per setting, joined by "; ".
    pub fn schedule(&self, is_12h: bool) -> String {
        let calendar = self.calendar.iter().map(|(_, spec)| calendar(spec, is_12h));
        let monotonic = self.monotonic.iter().map(|(base, us)| after(base, *us));
        calendar.chain(monotonic).collect::<Vec<_>>().join("; ")
    }

    /// The settings as the unit file spells them, one per line: `OnCalendar=*-*-* 03:00:00`.
    pub fn settings(&self) -> Vec<String> {
        let calendar = (self.calendar.iter()).map(|(base, spec)| format!("{base}={spec}"));
        let monotonic = self.monotonic.iter().map(|(base, us)| {
            let key = base.strip_suffix("USec").unwrap_or(base);
            format!("{key}={}", seconds(*us))
        });
        calendar.chain(monotonic).collect()
    }

    pub fn next_text(&self, is_12h: bool) -> String {
        self.next
            .filter(|_| self.on())
            .map_or_else(String::new, |us| when(us, is_12h))
    }

    pub fn last_text(&self, is_12h: bool) -> String {
        self.last
            .map_or_else(|| tr!("when-never"), |us| when(us, is_12h))
    }

    /// Takes the run `was` (this timer as read before) shows when the manager no longer reports
    /// one as late: it forgets a service's start and result once it unloads the service.
    pub fn keep_run(&mut self, was: &Task) {
        if was.last > self.last {
            self.last = was.last;
            self.result.clone_from(&was.result);
            self.exit_code = was.exit_code;
            self.exit_status = was.exit_status;
        }
    }

    /// How the last run ended.
    pub fn result_text(&self) -> String {
        match self.result.as_str() {
            _ if self.last.is_none() && self.exit_code == 0 => tr!("result-not-run"),
            "success" => tr!("result-success"),
            "exit-code" if self.exit_code == libc::CLD_EXITED => {
                tr!("result-exit", status = self.exit_status)
            }
            "signal" | "core-dump" => tr!("result-signal"),
            "timeout" => tr!("result-timeout"),
            other => other.to_string(),
        }
    }
}

/// A wall-clock time in µs as the Date Modified column writes one.
pub fn when(us: u64, is_12h: bool) -> String {
    reprisefs::listing::date_text((us / 1_000_000) as i64, is_12h)
}

/// `us` as a `.timer` file writes a span: `90min` reads `1h 30min`.
fn seconds(us: u64) -> String {
    let s = us / 1_000_000;
    let parts = [(86_400, "d"), (3_600, "h"), (60, "min"), (1, "s")];
    let mut left = s;
    let mut out = Vec::new();
    for (unit, name) in parts {
        if left >= unit {
            out.push(format!("{}{name}", left / unit));
            left %= unit;
        }
    }
    if out.is_empty() {
        "0".into()
    } else {
        out.join(" ")
    }
}

/// A span in words: "1 day 2 h", "15 min", "30 s".
fn span(us: u64) -> String {
    let s = us / 1_000_000;
    let (d, h, m, sec) = (s / 86_400, s % 86_400 / 3_600, s % 3_600 / 60, s % 60);
    let mut out = Vec::new();
    if d > 0 {
        out.push(tr!("span-days", n = d as i64));
    }
    if h > 0 {
        out.push(tr!("span-hours", n = h as i64));
    }
    if m > 0 {
        out.push(tr!("span-minutes", n = m as i64));
    }
    if sec > 0 || out.is_empty() {
        out.push(tr!("span-seconds", n = sec as i64));
    }
    out.join(" ")
}

/// A monotonic setting in words.
fn after(base: &str, us: u64) -> String {
    let span = span(us);
    match base {
        "OnBootUSec" => tr!("after-boot", span = span),
        "OnStartupUSec" => tr!("after-startup", span = span),
        "OnActiveUSec" => tr!("after-start", span = span),
        "OnUnitActiveUSec" => tr!("every", span = span),
        "OnUnitInactiveUSec" => tr!("after-finish", span = span),
        _ => format!("{base}={span}"),
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The weekday systemd abbreviates as `abbr`, named in the LC_TIME locale.
pub fn day_name(abbr: &str) -> Option<String> {
    let wday = DAYS.iter().position(|d| *d == abbr)?;
    // SAFETY: `libc::tm` is plain data; strftime's `%A` reads only `tm_wday`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_wday = wday as i32;
    Some(reprisefs::locale::strftime(&tm, c"%A"))
}

/// The weekdays of a normalized spec's `Mon..Fri` or `Mon,Wed` part in words.
fn days(spec: &str) -> Option<String> {
    let parts: Option<Vec<String>> = spec
        .split(',')
        .map(|part| match part.split_once("..") {
            Some((a, b)) => Some(tr!("days-range", first = day_name(a)?, last = day_name(b)?)),
            None => day_name(part),
        })
        .collect();
    Some(parts?.join(", "))
}

/// `H:M:S` with a set hour and minute and second 0, as the clock writes it.
fn clock(time: &str, is_12h: bool) -> Option<String> {
    let mut it = time.split(':');
    let h: i32 = it.next()?.parse().ok()?;
    let m: i32 = it.next()?.parse().ok()?;
    if it.next()? != "00" || it.next().is_some() || !(0..24).contains(&h) {
        return None;
    }
    Some(if is_12h {
        let (h12, ampm) = match h {
            0 => (12, "AM"),
            1..=11 => (h, "AM"),
            12 => (12, "PM"),
            _ => (h - 12, "PM"),
        };
        format!("{h12}:{m:02} {ampm}")
    } else {
        format!("{h}:{m:02}")
    })
}

/// A normalized `OnCalendar` spec in words where it has a common shape (every day, given
/// weekdays or a day of the month at a time; every hour or minute), else the spec itself.
pub fn calendar(spec: &str, is_12h: bool) -> String {
    calendar_words(spec, is_12h).unwrap_or_else(|| spec.to_string())
}

fn calendar_words(spec: &str, is_12h: bool) -> Option<String> {
    let parts: Vec<&str> = spec.split_whitespace().collect();
    let (dow, date, time) = match parts.as_slice() {
        [date, time] => (None, *date, *time),
        [dow, date, time] => (Some(*dow), *date, *time),
        _ => return None,
    };
    let at = clock(time, is_12h);
    match (dow, date, at) {
        (None, "*-*-*", Some(time)) => Some(tr!("schedule-daily", time = time)),
        (Some(d), "*-*-*", Some(time)) => Some(tr!("schedule-days", days = days(d)?, time = time)),
        (None, date, Some(time)) => {
            let day = date.strip_prefix("*-*-")?;
            let n: i64 = day.parse().ok()?;
            Some(tr!("schedule-monthly", day = n, time = time))
        }
        (None, "*-*-*", None) => match time.split(':').collect::<Vec<_>>().as_slice() {
            ["*", "*", "00"] => Some(tr!("schedule-minutely")),
            ["*", "00", "00"] => Some(tr!("schedule-hourly")),
            ["*", m, "00"] if m.parse::<u8>().is_ok_and(|m| m < 60) => {
                Some(tr!("schedule-hourly-at", minute = m.to_string()))
            }
            _ => None,
        },
        _ => None,
    }
}

/// The latest of the timer's `LastTriggerUSec` and its service's `ExecMainStartTimestamp` and
/// `InactiveExitTimestamp`, wall-clock µs: Run Now starts the service without touching the timer,
/// and a service the manager unloaded and loaded again has lost its own times.
pub fn last_run(stamps: [u64; 3]) -> Option<u64> {
    stamps.into_iter().filter(|&t| t != 0 && t != NEVER).max()
}

/// The wall-clock µs a timer elapses next, from its `NextElapseUSecRealtime` and
/// `NextElapseUSecMonotonic`, the sooner of the two as `systemctl list-timers` takes it. `now`
/// is the wall clock and `CLOCK_MONOTONIC` in µs.
pub fn next_elapse(realtime: u64, monotonic: u64, now: (u64, u64)) -> Option<u64> {
    let set = |t: u64| t != 0 && t != NEVER;
    let mono = set(monotonic).then(|| (now.0 + monotonic).saturating_sub(now.1));
    let real = set(realtime).then_some(realtime);
    match (real, mono) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000;

    fn task() -> Task {
        Task {
            timer: "backup.timer".into(),
            service: "backup.service".into(),
            description: "backup.service".into(),
            timer_state: "active".into(),
            file_state: "enabled".into(),
            service_state: "inactive".into(),
            result: "success".into(),
            ..Task::default()
        }
    }

    #[test]
    fn the_name_is_the_services_description_or_the_timers_stem() {
        assert_eq!(task().name(), "backup", "a description systemd made up");
        let described = Task {
            description: "Copy the photos".into(),
            ..task()
        };
        assert_eq!(described.name(), "Copy the photos");
        let bare = Task {
            description: String::new(),
            ..task()
        };
        assert_eq!(bare.name(), "backup");
    }

    #[test]
    fn common_calendar_shapes_read_as_words() {
        assert_eq!(calendar("*-*-* 03:00:00", false), "Every day at 3:00");
        assert_eq!(calendar("*-*-* 15:30:00", true), "Every day at 3:30 PM");
        assert_eq!(calendar("*-*-* 00:00:00", true), "Every day at 12:00 AM");
        let day = |d| day_name(d).unwrap();
        assert_eq!(
            calendar("Mon *-*-* 00:00:00", false),
            format!("Every {} at 0:00", day("Mon"))
        );
        assert_eq!(
            calendar("Mon..Fri *-*-* 08:15:00", false),
            format!("Every {} to {} at 8:15", day("Mon"), day("Fri"))
        );
        assert_eq!(
            calendar("Sat,Sun *-*-* 10:00:00", false),
            format!("Every {}, {} at 10:00", day("Sat"), day("Sun"))
        );
        assert_eq!(
            calendar("*-*-01 00:00:00", false),
            "Day 1 of every month at 0:00"
        );
        assert_eq!(calendar("*-*-* *:00:00", false), "Every hour");
        assert_eq!(calendar("*-*-* *:15:00", false), "Every hour at :15");
        assert_eq!(calendar("*-*-* *:*:00", false), "Every minute");
    }

    #[test]
    fn other_calendar_specs_show_as_systemd_writes_them() {
        for spec in [
            "*-*-* 03:00:30",
            "*-01,04,07,10-01 00:00:00",
            "*-*-* *:0/15:00",
            "*-*-* 03:00:00 Europe/Budapest",
            "Mon *-*-01 00:00:00",
            "Xyz *-*-* 00:00:00",
        ] {
            assert_eq!(calendar(spec, false), spec);
        }
    }

    #[test]
    fn monotonic_settings_read_as_spans_from_their_base() {
        let t = Task {
            monotonic: vec![
                ("OnBootUSec".into(), 15 * 60 * S),
                ("OnUnitActiveUSec".into(), 86_400 * S + 2 * 3_600 * S),
            ],
            ..task()
        };
        assert_eq!(t.schedule(false), "15 min after startup; Every 1 day 2 h");
        assert_eq!(t.settings(), ["OnBoot=15min", "OnUnitActive=1d 2h"]);
        assert_eq!(
            after("OnUnitInactiveUSec", 30 * S),
            "30 s after each run finishes"
        );
        assert_eq!(
            after("OnStartupUSec", 2 * 86_400 * S),
            "2 days after logging on"
        );
    }

    #[test]
    fn calendar_and_monotonic_settings_join_in_one_schedule() {
        let t = Task {
            calendar: vec![("OnCalendar".into(), "*-*-* 03:00:00".into())],
            monotonic: vec![("OnActiveUSec".into(), 3_600 * S)],
            ..task()
        };
        assert_eq!(
            t.schedule(false),
            "Every day at 3:00; 1 h after the timer starts"
        );
        assert_eq!(t.settings(), ["OnCalendar=*-*-* 03:00:00", "OnActive=1h"]);
    }

    #[test]
    fn the_next_run_is_the_sooner_clock_turned_into_wall_time() {
        let now = (1_000_000 * S, 500 * S);
        assert_eq!(next_elapse(0, 0, now), None);
        assert_eq!(next_elapse(NEVER, NEVER, now), None);
        assert_eq!(next_elapse(2_000_000 * S, 0, now), Some(2_000_000 * S));
        assert_eq!(
            next_elapse(0, 800 * S, now),
            Some(1_000_300 * S),
            "300 s of monotonic clock ahead"
        );
        assert_eq!(
            next_elapse(1_000_100 * S, 800 * S, now),
            Some(1_000_100 * S)
        );
    }

    #[test]
    fn the_last_run_is_the_later_of_the_timers_and_the_services_start() {
        assert_eq!(last_run([0, 0, 0]), None);
        assert_eq!(last_run([NEVER, 0, NEVER]), None);
        assert_eq!(last_run([100 * S, 0, 0]), Some(100 * S), "the timer fired");
        assert_eq!(
            last_run([100 * S, 300 * S, 300 * S]),
            Some(300 * S),
            "Run Now after the timer fired"
        );
        assert_eq!(
            last_run([0, 0, 200 * S]),
            Some(200 * S),
            "a service with no main process start"
        );
        assert_eq!(last_run([500 * S, 300 * S, 300 * S]), Some(500 * S));
    }

    #[test]
    fn the_status_follows_the_timer_then_the_service() {
        assert_eq!(task().status(), "Ready");
        let off = Task {
            timer_state: "inactive".into(),
            ..task()
        };
        assert_eq!(off.status(), "Disabled");
        assert_eq!(off.next_text(false), "", "a stopped timer has no next run");
        let running = Task {
            service_state: "activating".into(),
            ..task()
        };
        assert_eq!(running.status(), "Running");
        assert!(!running.can_run());
        let failed = Task {
            service_state: "failed".into(),
            ..task()
        };
        assert_eq!(failed.status(), "Failed");
        assert!(failed.can_run());
        let stopped = Task {
            result: "signal".into(),
            ..failed.clone()
        };
        assert_eq!(stopped.status(), "Stopped", "a run a signal ended");
        let dumped = Task {
            result: "core-dump".into(),
            ..failed.clone()
        };
        assert_eq!(dumped.status(), "Failed", "a crash");
        let masked = Task {
            file_state: "masked".into(),
            ..task()
        };
        assert!(!masked.can_toggle());
        let unloaded = Task {
            service_state: String::new(),
            ..task()
        };
        assert!(!unloaded.can_run());
    }

    #[test]
    fn the_last_result_reads_how_the_run_ended() {
        assert_eq!(task().last_text(false), "Never");
        assert_eq!(task().result_text(), "Not run yet");
        let ran = Task {
            last: Some(1_000 * S),
            ..task()
        };
        assert_eq!(ran.result_text(), "Succeeded");
        let exited = Task {
            result: "exit-code".into(),
            exit_code: libc::CLD_EXITED,
            exit_status: 3,
            ..ran.clone()
        };
        assert_eq!(exited.result_text(), "Exited with status 3");
        let killed = Task {
            result: "signal".into(),
            exit_code: libc::CLD_KILLED,
            ..ran.clone()
        };
        assert_eq!(killed.result_text(), "Ended by a signal");
    }
}
