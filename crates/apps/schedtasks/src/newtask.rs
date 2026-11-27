//! New Task: the box behind New Task... and Edit.... Create (OK for Edit) stays greyed until the
//! fields make a task; a line says what is missing, and another the schedule in words.

use crate::model;
use crate::units::{self, NewTask, Problem, Schedule, WEEKDAYS};
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::combo::COMBO_H;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::timefield::TimeField;
use reprisekit::raster::Rect;
use reprisekit::tr;
use std::path::{Path, PathBuf};

pub const CLIENT: (i32, i32) = (400, 322);

const X: i32 = 12;
const FIELD_X: i32 = 100;
const FIELD_W: i32 = CLIENT.0 - X - FIELD_X;
const FIELD_H: i32 = 21;
const PITCH: i32 = 28;

pub const NAME: &str = "name";
pub const PROGRAM: &str = "program";
pub const BROWSE: &str = "browse";
pub const ARGS: &str = "args";
pub const KIND: &str = "kind";
pub const DAY: &str = "day";
pub const DATE: &str = "date";
pub const TIME: &str = "time";
pub const SETTING: &str = "setting";
pub const HINT: &str = "hint";
pub const CREATE: &str = "create";
pub const CANCEL: &str = "cancel";

/// The Run combo's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Minutes(u32),
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Once,
    AtLogOn,
}

const KINDS: [Kind; 10] = [
    Kind::Minutes(5),
    Kind::Minutes(10),
    Kind::Minutes(15),
    Kind::Minutes(30),
    Kind::Hourly,
    Kind::Daily,
    Kind::Weekly,
    Kind::Monthly,
    Kind::Once,
    Kind::AtLogOn,
];

fn kind_row(kind: Kind) -> usize {
    KINDS.iter().position(|k| *k == kind).unwrap_or(0)
}

impl Kind {
    fn label(self) -> String {
        match self {
            Kind::Minutes(n) => tr!("kind-minutes", n = n as i64),
            Kind::Hourly => tr!("kind-hourly"),
            Kind::Daily => tr!("kind-daily"),
            Kind::Weekly => tr!("kind-weekly"),
            Kind::Monthly => tr!("kind-monthly"),
            Kind::Once => tr!("kind-once"),
            Kind::AtLogOn => tr!("kind-logon"),
        }
    }

    fn timed(self) -> bool {
        matches!(
            self,
            Kind::Daily | Kind::Weekly | Kind::Monthly | Kind::Once
        )
    }
}

/// A local date and time: (year, month, day, hour, minute).
pub type Local = (i32, u32, u32, u32, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Browse,
    Create(NewTask),
    Cancel,
}

pub struct NewTaskBox {
    pub form: Form,
    /// The time now ([`now`]), which a once task must come after.
    clock: fn() -> Local,
    twelve: bool,
    /// The timer Edit opened the box for and the task its files held then; None for New Task.
    pub editing: Option<(String, NewTask)>,
    /// The Run row the Day combo holds rows for, and the day picked for weekly and monthly.
    shown: Kind,
    weekday: usize,
    month_day: usize,
}

fn field(id: &'static str, y: i32, w: i32) -> Item {
    Item::edit(id, Rect::new(FIELD_X, y, w, FIELD_H), "", true)
}

fn label(id: &'static str, y: i32, text: &str) -> Item {
    Item::label_at(id, X, y + 4, FIELD_X - X - 4, text)
}

fn weekdays() -> Vec<String> {
    WEEKDAYS
        .iter()
        .map(|d| model::day_name(d).unwrap_or_else(|| d.to_string()))
        .collect()
}

fn month_days() -> Vec<String> {
    (1..=31).map(|d| d.to_string()).collect()
}

/// The day after `now`, as a once task's first guess.
fn next_day((y, mo, d, ..): Local) -> (i32, u32, u32) {
    match (d < days_in(y, mo), mo) {
        (true, _) => (y, mo, d + 1),
        (false, 12) => (y + 1, 1, 1),
        (false, _) => (y, mo + 1, 1),
    }
}

fn days_in(y: i32, mo: u32) -> u32 {
    match mo {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY-MM-DD` as (year, month, day), when it is a real date.
fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.trim().split('-');
    let (y, mo, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || mo.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, mo, d): (i32, u32, u32) = (y.parse().ok()?, mo.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&mo) && (1..=days_in(y, mo)).contains(&d)).then_some((y, mo, d))
}

impl NewTaskBox {
    /// The box reading the time from `clock`, its time field in 12-hour form when `twelve`.
    pub fn new(clock: fn() -> Local, twelve: bool) -> NewTaskBox {
        let kinds = KINDS.map(Kind::label);
        let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
        let (y, mo, d) = next_day(clock());
        let row = |i: i32| X + i * PITCH;
        let sy = row(3) + 10;
        let by = CLIENT.1 - X - BUTTON_H;
        let items = vec![
            label("name_label", row(0), &tr!("new-name")),
            field(NAME, row(0), FIELD_W),
            label("program_label", row(1), &tr!("new-program")),
            field(PROGRAM, row(1), FIELD_W - BUTTON_W - 6),
            Item::button_at(
                BROWSE,
                CLIENT.0 - X - BUTTON_W,
                row(1),
                BUTTON_W,
                &tr!("new-browse"),
                true,
            ),
            label("args_label", row(2), &tr!("new-args")),
            field(ARGS, row(2), FIELD_W),
            Item::new(
                "sep",
                Rect::new(X, sy - 6, CLIENT.0 - 2 * X, 2),
                Control::Separator,
            ),
            label("kind_label", sy, &tr!("new-kind")),
            Item::combo(KIND, FIELD_X, sy, 140, &kinds, KINDS.len()),
            label("day_label", sy + PITCH, &tr!("new-day")),
            Item::combo(DAY, FIELD_X, sy + PITCH, 140, &[], 8),
            label("date_label", sy + 2 * PITCH, &tr!("new-date")),
            field(DATE, sy + 2 * PITCH, 100),
            label("time_label", sy + 3 * PITCH, &tr!("new-time")),
            Item::new(
                TIME,
                Rect::new(FIELD_X, sy + 3 * PITCH, 100, FIELD_H),
                Control::Time(TimeField::hm(9, 0, twelve)),
            ),
            label("setting_label", sy + 4 * PITCH, &tr!("new-setting")),
            Item::label_at(SETTING, FIELD_X, sy + 4 * PITCH + 4, FIELD_W, ""),
            Item::label(HINT, Rect::new(X, by - 22, CLIENT.0 - 2 * X, 13), ""),
            Item::button(
                CREATE,
                Rect::new(CLIENT.0 - X - 2 * BUTTON_W - 6, by, BUTTON_W, BUTTON_H),
                &tr!("new-create"),
                false,
                true,
            ),
            Item::button_at(
                CANCEL,
                CLIENT.0 - X - BUTTON_W,
                by,
                BUTTON_W,
                &tr!("new-cancel"),
                true,
            ),
        ];
        debug_assert!(sy + 4 * PITCH + 17 < by - 22 && COMBO_H <= PITCH);
        let mut b = NewTaskBox {
            form: Form::new(items),
            clock,
            twelve,
            editing: None,
            shown: Kind::AtLogOn,
            weekday: 0,
            month_day: 0,
        };
        b.form.set_combo(KIND, kind_row(Kind::Daily));
        b.form
            .show_edit_text(DATE, &format!("{y:04}-{mo:02}-{d:02}"));
        b.form.focus_id(NAME);
        b.sync();
        b
    }

    /// The box for Edit on timer `timer`, holding `task` as [`units::read`] read it.
    pub fn edit(clock: fn() -> Local, twelve: bool, timer: &str, task: &NewTask) -> NewTaskBox {
        let mut b = NewTaskBox::new(clock, twelve);
        b.editing = Some((timer.to_string(), task.clone()));
        if let Some(Control::Button { label, .. }) = b.form.item_mut(CREATE).map(|i| &mut i.control)
        {
            *label = tr!("edit-ok");
        }
        b.form.show_edit_text(NAME, &task.name);
        b.form
            .show_edit_text(PROGRAM, &task.program.to_string_lossy());
        b.form.show_edit_text(ARGS, &units::join_args(&task.args));
        let (kind, at) = match task.schedule {
            Schedule::Minutes { every } => (Kind::Minutes(every), None),
            Schedule::Hourly => (Kind::Hourly, None),
            Schedule::Daily { at } => (Kind::Daily, Some(at)),
            Schedule::Weekly { day, at } => {
                b.weekday = day;
                (Kind::Weekly, Some(at))
            }
            Schedule::Monthly { day, at } => {
                b.month_day = day.saturating_sub(1) as usize;
                (Kind::Monthly, Some(at))
            }
            Schedule::Once {
                date: (y, mo, d),
                at,
            } => {
                b.form
                    .show_edit_text(DATE, &format!("{y:04}-{mo:02}-{d:02}"));
                (Kind::Once, Some(at))
            }
            Schedule::AtLogOn => (Kind::AtLogOn, None),
        };
        if let (Some((h, m)), Some(Control::Time(t))) =
            (at, b.form.item_mut(TIME).map(|i| &mut i.control))
        {
            (t.h, t.m) = (h, m);
        }
        b.form.set_combo(KIND, kind_row(kind));
        b.sync();
        b
    }

    fn kind(&self) -> Kind {
        let i = self.form.combo_selected(KIND);
        i.and_then(|i| KINDS.get(i)).copied().unwrap_or(Kind::Daily)
    }

    fn at(&self) -> (u32, u32) {
        match self.form.item(TIME).map(|i| &i.control) {
            Some(Control::Time(t)) => (t.h, t.m),
            _ => (9, 0),
        }
    }

    /// The schedule the fields give, or why they give none.
    fn schedule(&self) -> Result<Schedule, String> {
        let at = self.at();
        let day = self.form.combo_selected(DAY).unwrap_or(0);
        Ok(match self.kind() {
            Kind::Daily => Schedule::Daily { at },
            Kind::Weekly => Schedule::Weekly { day, at },
            Kind::Monthly => Schedule::Monthly {
                day: day as u32 + 1,
                at,
            },
            Kind::Once => {
                let date =
                    parse_date(self.form.edit_text(DATE)).ok_or_else(|| tr!("new-bad-date"))?;
                let (y, mo, d, h, mi) = (self.clock)();
                if (date, at) <= ((y, mo, d), (h, mi)) {
                    return Err(tr!("new-past"));
                }
                Schedule::Once { date, at }
            }
            Kind::Hourly => Schedule::Hourly,
            Kind::Minutes(every) => Schedule::Minutes { every },
            Kind::AtLogOn => Schedule::AtLogOn,
        })
    }

    /// The task the fields make, or what they lack.
    pub fn task(&self) -> Result<NewTask, String> {
        let name = self.form.edit_text(NAME).trim();
        if name.is_empty() {
            return Err(tr!("new-no-name"));
        }
        let typed = self.form.edit_text(PROGRAM).trim();
        if typed.is_empty() {
            return Err(tr!("new-no-program"));
        }
        let path = std::env::var_os("PATH");
        let program = units::find_program(typed, path.as_deref())
            .ok_or_else(|| tr!("new-no-such-program", program = typed))?;
        let args = units::split_args(self.form.edit_text(ARGS)).map_err(problem)?;
        let task = NewTask {
            name: name.to_string(),
            program,
            args,
            schedule: self.schedule()?,
        };
        units::texts(&task, None).map_err(problem)?;
        Ok(task)
    }

    /// Greys what the Run row leaves unused, fills the Day combo for it, and shows the setting
    /// and what is missing.
    fn sync(&mut self) {
        let kind = self.kind();
        if kind != self.shown {
            match self.shown {
                Kind::Weekly => self.weekday = self.form.combo_selected(DAY).unwrap_or(0),
                Kind::Monthly => self.month_day = self.form.combo_selected(DAY).unwrap_or(0),
                _ => {}
            }
            match kind {
                Kind::Weekly => self.form.set_combo_rows(DAY, &weekdays(), self.weekday, 7),
                Kind::Monthly => self
                    .form
                    .set_combo_rows(DAY, &month_days(), self.month_day, 8),
                _ => self.form.set_combo_rows(DAY, &[], 0, 8),
            }
            self.shown = kind;
        }
        let f = &mut self.form;
        f.set_enabled(DAY, matches!(kind, Kind::Weekly | Kind::Monthly));
        f.set_enabled("day_label", matches!(kind, Kind::Weekly | Kind::Monthly));
        f.set_enabled(DATE, kind == Kind::Once);
        f.set_enabled("date_label", kind == Kind::Once);
        f.set_enabled(TIME, kind.timed());
        f.set_enabled("time_label", kind.timed());
        let setting = self.schedule().map(|s| words(&s, self.twelve));
        let setting = setting.unwrap_or_default();
        let task = self.task();
        let hint = match &task {
            Err(why) => why.clone(),
            Ok(_) if kind == Kind::AtLogOn && self.editing.is_none() => tr!("new-logon-now"),
            Ok(_) => String::new(),
        };
        let f = &mut self.form;
        f.set_text(SETTING, &setting);
        f.set_text(HINT, &hint);
        f.set_enabled(CREATE, task.is_ok());
    }

    /// The program field holds `path`, picked with Browse.
    pub fn browsed(&mut self, path: &Path) {
        self.form.show_edit_text(PROGRAM, &path.to_string_lossy());
        self.sync();
    }

    /// Where Browse opens: the folder of the program typed, else home.
    pub fn browse_dir(&self) -> PathBuf {
        let typed = Path::new(self.form.edit_text(PROGRAM).trim());
        typed
            .parent()
            .filter(|d| d.is_absolute() && d.is_dir())
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        let out = match ev {
            form::Event::Pressed(CANCEL) | form::Event::Cancel => return Outcome::Cancel,
            form::Event::Pressed(BROWSE) => return Outcome::Browse,
            form::Event::Pressed(CREATE) | form::Event::Default => match self.task() {
                Ok(t) => return Outcome::Create(t),
                Err(_) => Outcome::Redraw,
            },
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        };
        self.sync();
        out
    }
}

/// `s` as the list's Schedule column words it.
fn words(s: &Schedule, twelve: bool) -> String {
    match s.calendar() {
        Some(spec) => model::calendar(&spec, twelve),
        None => model::after("OnStartupUSec", units::LOGON_DELAY_S * 1_000_000),
    }
}

fn problem(p: Problem) -> String {
    match p {
        Problem::Control => tr!("new-control"),
        Problem::Program => tr!("new-no-program"),
        Problem::Quote => tr!("new-quote"),
        Problem::Special => tr!("new-special"),
    }
}

/// The local date and time now.
pub fn now() -> Local {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    local(t)
}

/// The local date and time `t` seconds after the epoch.
fn local(t: i64) -> Local {
    let t = t as libc::time_t;
    // SAFETY: `tm` is plain data that `localtime_r` fills; `t` outlives the call.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    (
        tm.tm_year + 1900,
        tm.tm_mon as u32 + 1,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Local = (2026, 11, 26, 10, 30);

    fn typed(b: &mut NewTaskBox, id: &'static str, text: &str) {
        b.form.show_edit_text(id, text);
        b.sync();
    }

    fn pick(b: &mut NewTaskBox, kind: Kind) {
        b.form.set_combo(KIND, kind_row(kind));
        b.sync();
    }

    fn label(b: &NewTaskBox, id: &str) -> String {
        match b.form.item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            _ => panic!("no {id}"),
        }
    }

    fn enabled(b: &NewTaskBox, id: &str) -> bool {
        match b.form.item(id).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) | Some(Control::Edit { enabled, .. }) => *enabled,
            Some(Control::Combo(c)) => c.enabled,
            Some(Control::Time(t)) => t.enabled,
            _ => panic!("no {id}"),
        }
    }

    fn filled() -> NewTaskBox {
        let mut b = NewTaskBox::new(|| NOW, false);
        typed(&mut b, NAME, "Back up");
        typed(&mut b, PROGRAM, "/bin/sh");
        typed(&mut b, ARGS, r#"-c "echo hi""#);
        b
    }

    #[test]
    fn the_box_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("new task".to_string(), NewTaskBox::new(|| NOW, true).form)]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn create_greys_until_the_fields_make_a_task() {
        let mut b = NewTaskBox::new(|| NOW, false);
        assert!(!enabled(&b, CREATE));
        assert_eq!(label(&b, HINT), "Type a name for the task.");
        typed(&mut b, NAME, "Back up");
        assert_eq!(
            label(&b, HINT),
            "Type the program to run, or Browse for it."
        );
        typed(&mut b, PROGRAM, "/no/such/program");
        assert_eq!(label(&b, HINT), "Cannot find the program /no/such/program.");
        typed(&mut b, PROGRAM, "sh");
        typed(&mut b, ARGS, "\"open");
        assert_eq!(
            label(&b, HINT),
            "An argument opens a quote it never closes, or ends in \\."
        );
        typed(&mut b, ARGS, "-c true");
        assert_eq!(label(&b, HINT), "");
        assert!(enabled(&b, CREATE));
        let t = b.task().unwrap();
        assert!(t.program.is_absolute(), "a bare name found on PATH");
        assert_eq!(t.args, ["-c", "true"]);
    }

    #[test]
    fn each_run_row_shows_its_schedule_and_greys_the_rest() {
        let mut b = filled();
        let day = |d| model::day_name(d).unwrap();
        assert_eq!(label(&b, SETTING), "Every day at 9:00", "Daily at first");
        assert!(!enabled(&b, DAY) && !enabled(&b, DATE) && enabled(&b, TIME));
        pick(&mut b, Kind::Weekly);
        assert!(enabled(&b, DAY));
        b.form.set_combo(DAY, 2);
        b.sync();
        assert_eq!(label(&b, SETTING), format!("Every {} at 9:00", day("Wed")));
        pick(&mut b, Kind::Monthly);
        assert_eq!(label(&b, SETTING), "Day 1 of every month at 9:00");
        b.form.set_combo(DAY, 14);
        b.sync();
        assert_eq!(label(&b, SETTING), "Day 15 of every month at 9:00");
        pick(&mut b, Kind::Weekly);
        assert_eq!(
            label(&b, SETTING),
            format!("Every {} at 9:00", day("Wed")),
            "the weekday picked comes back"
        );
        pick(&mut b, Kind::Once);
        assert!(enabled(&b, DATE) && !enabled(&b, DAY));
        assert_eq!(
            b.schedule().unwrap().setting(),
            "OnCalendar=2026-11-27 09:00:00",
            "tomorrow"
        );
        pick(&mut b, Kind::Hourly);
        assert!(!enabled(&b, TIME) && !enabled(&b, DAY) && !enabled(&b, DATE));
        assert_eq!(label(&b, SETTING), "Every hour");
        pick(&mut b, Kind::Minutes(15));
        assert!(!enabled(&b, TIME));
        assert_eq!(label(&b, SETTING), "Every 15 minutes");
        assert_eq!(b.schedule(), Ok(Schedule::Minutes { every: 15 }));
        pick(&mut b, Kind::AtLogOn);
        assert!(!enabled(&b, TIME) && !enabled(&b, DATE));
        assert_eq!(label(&b, SETTING), "30 s after logging on");
        assert_eq!(
            label(&b, HINT),
            "The task also runs once now, when it is made."
        );
        assert_eq!(
            b.event(form::Event::Pressed(CREATE)),
            Outcome::Create(NewTask {
                name: "Back up".into(),
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "echo hi".into()],
                schedule: Schedule::AtLogOn,
            })
        );
    }

    #[test]
    fn a_once_task_takes_a_real_date_still_to_come() {
        let mut b = filled();
        pick(&mut b, Kind::Once);
        for (date, hint) in [
            ("2026-02-29", "Type the date as 2026-12-31."),
            ("27.11.2026", "Type the date as 2026-12-31."),
            ("2026-11-26", "That time has passed."),
            ("2025-12-01", "That time has passed."),
            ("2028-02-29", ""),
        ] {
            typed(&mut b, DATE, date);
            assert_eq!(label(&b, HINT), hint, "{date}");
        }
        b.clock = || (2026, 11, 26, 8, 0);
        typed(&mut b, DATE, "2026-11-26");
        assert_eq!(label(&b, HINT), "", "an hour away");
        b.clock = || (2026, 11, 26, 9, 30);
        assert_eq!(
            b.event(form::Event::Pressed(CREATE)),
            Outcome::Redraw,
            "Create reads the clock again"
        );
        assert_eq!(label(&b, HINT), "That time has passed.");
        assert_eq!(next_day((2026, 12, 31, 0, 0)), (2027, 1, 1));
        assert_eq!(next_day((2028, 2, 28, 0, 0)), (2028, 2, 29));
    }

    #[test]
    fn a_program_path_systemd_refuses_is_told_before_create() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let mut b = filled();
        for name in ["Bob's backup.sh", "sync [nas].sh"] {
            let path = tmp.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            b.browsed(&path);
            assert_eq!(
                label(&b, HINT),
                "The program's path cannot hold \\ \" ' * ? or [.",
                "{name}"
            );
            assert!(!enabled(&b, CREATE));
        }
    }

    #[test]
    fn edit_opens_on_the_task_and_gives_it_back_unchanged() {
        for schedule in [
            Schedule::Minutes { every: 10 },
            Schedule::Hourly,
            Schedule::Daily { at: (7, 45) },
            Schedule::Weekly { day: 4, at: (9, 5) },
            Schedule::Monthly {
                day: 31,
                at: (23, 0),
            },
            Schedule::Once {
                date: (2027, 3, 1),
                at: (6, 0),
            },
            Schedule::AtLogOn,
        ] {
            let task = NewTask {
                name: "Back up".into(),
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "echo 'hi there'".into(), String::new()],
                schedule,
            };
            let mut b = NewTaskBox::edit(|| NOW, false, "reprise-task-back-up.timer", &task);
            assert_eq!(label(&b, HINT), "", "{schedule:?}");
            assert_eq!(
                b.event(form::Event::Pressed(CREATE)),
                Outcome::Create(task),
                "{schedule:?}"
            );
        }
    }

    #[test]
    fn browse_fills_the_program() {
        let mut b = filled();
        assert_eq!(b.event(form::Event::Pressed(BROWSE)), Outcome::Browse);
        assert_eq!(b.browse_dir(), PathBuf::from("/bin"));
        b.browsed(Path::new("/usr/bin/true"));
        assert_eq!(b.form.edit_text(PROGRAM), "/usr/bin/true");
        assert_eq!(b.event(form::Event::Cancel), Outcome::Cancel);
    }
}
