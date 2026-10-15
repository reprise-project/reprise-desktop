//! The compositor's displays and their levels: one entry per output, read
//! through the media worker's lanes and stepped or set from the cached level.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use reprisekit::art::icons;
use reprisekit::controls::osd::{self, Level};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::media::{
    DisplayJob, DisplayReport, Job, Outcome, Reading, Value, is_internal, step_percent,
};

/// An output as the display list sees it: its connector and EDID names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub connector: String,
    pub make: String,
    pub model: String,
}

/// Each head's label: "Built-in display" for a laptop panel, else the model, the make or the connector; a label two heads share gets each one's connector.
pub fn labels(heads: &[Head]) -> Vec<String> {
    let known = |s: &str| !s.is_empty() && s != "Unknown";
    let base: Vec<String> = heads
        .iter()
        .map(|h| {
            if is_internal(&h.connector) {
                "Built-in display".to_owned()
            } else if known(&h.model) {
                h.model.clone()
            } else if known(&h.make) {
                h.make.clone()
            } else {
                h.connector.clone()
            }
        })
        .collect();
    base.iter()
        .zip(heads)
        .map(|(l, h)| {
            if base.iter().filter(|b| *b == l).count() > 1 {
                format!("{l} ({})", h.connector)
            } else {
                l.clone()
            }
        })
        .collect()
}

pub struct Display {
    pub connector: String,
    pub label: String,
    pub internal: bool,
    slots: BTreeMap<Value, Slot>,
    retry: Option<Retry>,
}

/// When a display that is not adjustable was last read, and how long the timer waits before the next read.
#[derive(Clone, Copy)]
struct Retry {
    last: Instant,
    gap: Duration,
}

/// The least time between two reads of a display that is not adjustable, and the timer's first wait.
pub const RETRY: Duration = Duration::from_secs(10);
/// The timer's wait doubles after each read that leaves the display not adjustable, up to this.
pub const RETRY_MAX: Duration = Duration::from_secs(300);

/// One value's cached level; `last_set` is the seq of the last set sent, which decides which replies still apply.
struct Slot {
    level: Level,
    floor: u8,
    last_set: u64,
}

impl Display {
    pub fn level(&self, v: Value) -> Level {
        self.slots.get(&v).map_or(Level::Unread, |s| s.level)
    }

    fn fixed(&self) -> bool {
        self.slots.values().any(|s| s.level == Level::Fixed)
    }
}

#[derive(Default)]
pub struct Displays {
    pub list: Vec<Display>,
    seq: u64,
    /// The displays cell's wheel carry for [`wheel_steps`]; the panel keeps its own.
    pub wheel: (i32, f64),
    timer: Option<RegistrationToken>,
}

impl Displays {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn read(&mut self, connector: &str) -> Vec<DisplayJob> {
        Value::ALL
            .iter()
            .map(|&value| DisplayJob::Read {
                display: connector.to_owned(),
                value,
                seq: self.next_seq(),
            })
            .collect()
    }

    /// Follow `heads` (in layout order): known displays keep their levels, gone ones drop, new ones start unread with a read each.
    pub fn sync(&mut self, heads: &[Head]) -> Vec<DisplayJob> {
        let mut old = std::mem::take(&mut self.list);
        let mut jobs = Vec::new();
        for (h, label) in heads.iter().zip(labels(heads)) {
            let display = match old.iter().position(|d| d.connector == h.connector) {
                Some(i) => Display {
                    label,
                    ..old.swap_remove(i)
                },
                None => {
                    jobs.extend(self.read(&h.connector));
                    Display {
                        connector: h.connector.clone(),
                        label,
                        internal: is_internal(&h.connector),
                        slots: BTreeMap::new(),
                        retry: None,
                    }
                }
            };
            self.list.push(display);
        }
        jobs
    }

    /// A read of every value on every display.
    pub fn reread(&mut self) -> Vec<DisplayJob> {
        let connectors: Vec<String> = self.list.iter().map(|d| d.connector.clone()).collect();
        connectors.iter().flat_map(|c| self.read(c)).collect()
    }

    /// Reads of the displays that are not adjustable and due one at `now`: `eager` (a key or the wheel) reads one last read at least [`RETRY`] ago, else (the timer) one whose wait is over. A display that just became not adjustable starts its clock without a read; one that is adjustable again drops it.
    pub fn retry(&mut self, now: Instant, eager: bool) -> Vec<DisplayJob> {
        let mut due = Vec::new();
        for d in &mut self.list {
            if !d.fixed() {
                d.retry = None;
                continue;
            }
            let Some(r) = d.retry else {
                d.retry = Some(Retry {
                    last: now,
                    gap: RETRY,
                });
                continue;
            };
            let wait = if eager { RETRY } else { r.gap };
            if now >= r.last + wait {
                d.retry = Some(Retry {
                    last: now,
                    gap: (r.gap * 2).min(RETRY_MAX),
                });
                due.push(d.connector.clone());
            }
        }
        due.iter().flat_map(|c| self.read(c)).collect()
    }

    /// When the timer next has a display to read again.
    pub fn next_retry(&self) -> Option<Instant> {
        self.list
            .iter()
            .filter_map(|d| d.retry.map(|r| r.last + r.gap))
            .min()
    }

    /// Move `value` on `connector`'s display (every display for None) by `steps` from its own level; unread and fixed displays stay.
    pub fn step(&mut self, connector: Option<&str>, value: Value, steps: i32) -> Vec<DisplayJob> {
        let mut jobs = Vec::new();
        for i in 0..self.list.len() {
            if connector.is_some_and(|c| c != self.list[i].connector) {
                continue;
            }
            let Some(slot) = self.list[i].slots.get(&value) else {
                continue;
            };
            let Level::At(cur) = slot.level else {
                continue;
            };
            let level = step_percent(cur, steps, slot.floor);
            jobs.extend(self.move_to(i, value, level));
        }
        jobs
    }

    /// Put `value` on `connector`'s display at `level` (kept within its floor and 100); None when it is not adjustable or already there.
    pub fn set(&mut self, connector: &str, value: Value, level: u8) -> Option<DisplayJob> {
        let i = self.list.iter().position(|d| d.connector == connector)?;
        let slot = self.list[i].slots.get(&value)?;
        let Level::At(_) = slot.level else {
            return None;
        };
        let level = level.clamp(slot.floor, 100);
        self.move_to(i, value, level)
    }

    fn move_to(&mut self, i: usize, value: Value, level: u8) -> Option<DisplayJob> {
        let seq = self.seq + 1;
        let d = &mut self.list[i];
        let slot = d.slots.get_mut(&value)?;
        if slot.level == Level::At(level) {
            return None;
        }
        slot.level = Level::At(level);
        slot.last_set = seq;
        self.seq = seq;
        Some(DisplayJob::Set {
            display: d.connector.clone(),
            value,
            level,
            seq,
        })
    }

    /// Apply a worker reply: a read newer than the last set, or the failure of the last set; true when the shown level changed.
    pub fn report(&mut self, r: &DisplayReport) -> bool {
        let Some(d) = self.list.iter_mut().find(|d| d.connector == r.display) else {
            return false;
        };
        let slot = d.slots.entry(r.value).or_insert(Slot {
            level: Level::Unread,
            floor: 0,
            last_set: 0,
        });
        let reading = match &r.outcome {
            Outcome::Read(x) if r.seq > slot.last_set => x,
            Outcome::Failed(x) if r.seq == slot.last_set => x,
            _ => return false,
        };
        let before = slot.level;
        match reading {
            Some(Reading { level, floor }) => {
                slot.level = Level::At(*level);
                slot.floor = *floor;
            }
            None => slot.level = Level::Fixed,
        }
        slot.level != before
    }

    /// Whether any display's `value` can be changed.
    pub fn adjustable(&self, value: Value) -> bool {
        self.list
            .iter()
            .any(|d| matches!(d.level(value), Level::At(_)))
    }

    pub fn lines(&self, value: Value) -> osd::Lines {
        osd::Lines {
            title: value.title().to_owned(),
            lines: self
                .list
                .iter()
                .map(|d| osd::Line {
                    icon: if d.internal {
                        &icons::LAPTOP
                    } else {
                        &icons::MONITOR
                    },
                    label: d.label.clone(),
                    level: d.level(value),
                })
                .collect(),
        }
    }

    pub fn tip_lines(&self, value: Value) -> Vec<String> {
        self.list
            .iter()
            .map(|d| match d.level(value) {
                Level::At(l) => format!("{}: {l}%", d.label),
                Level::Fixed => format!("{}: not adjustable", d.label),
                Level::Unread => d.label.clone(),
            })
            .collect()
    }
}

/// Scroll travel in surface units that makes one step when the wheel sends no notches (a touchpad).
pub const TOUCH_STEP: f64 = 15.0;

/// Whole steps an axis event makes on the displays cell, positive to brighten; `carried` holds the 120ths of a notch and the touchpad travel left over.
pub fn wheel_steps(discrete: i32, continuous: f64, carried: &mut (i32, f64)) -> i32 {
    if discrete != 0 {
        let (steps, residual) = crate::tray::wheel_steps(discrete, continuous, carried.0);
        *carried = (residual, 0.0);
        return steps;
    }
    carried.1 += continuous;
    let steps = (carried.1 / TOUCH_STEP).trunc();
    carried.1 -= steps * TOUCH_STEP;
    -(steps as i32)
}

/// Whether the displays cell shows: a display can be dimmed, or night light is on in the config.
pub fn cell_shown(state: &State) -> bool {
    state.displays.adjustable(Value::Brightness) || crate::nightlight::enabled(&state.night)
}

/// The displays cell's tooltip: each display's brightness, then night light's line while it is enabled.
pub fn tip(state: &State) -> String {
    let night =
        crate::nightlight::enabled(&state.night).then(|| crate::nightlight::tip(&state.night));
    tip_text(state.displays.tip_lines(Value::Brightness), night)
}

fn tip_text(lines: Vec<String>, night: Option<String>) -> String {
    lines
        .into_iter()
        .chain(night)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The outputs changed: follow them in layout order (left to right, then top to bottom) and read the new ones.
pub fn outputs_changed(state: &mut State) {
    let before = state.displays.adjustable(Value::Brightness);
    let mut placed: Vec<((i32, i32), Head)> = state
        .scene
        .outputs()
        .iter()
        .map(|o| {
            let r = state.layout_box(Some(o));
            let p = o.physical_properties();
            let head = Head {
                connector: o.name(),
                make: p.make,
                model: p.model,
            };
            ((r.x, r.y), head)
        })
        .collect();
    placed.sort_by_key(|(at, _)| *at);
    let heads: Vec<Head> = placed.into_iter().map(|(_, h)| h).collect();
    let jobs = state.displays.sync(&heads);
    send(state, jobs);
    changed(state, before);
}

pub fn reread(state: &mut State) {
    let jobs = state.displays.reread();
    send(state, jobs);
}

/// Step `value` (see [`Displays::step`]) and read again the displays that are not adjustable, so a key or the wheel brings back one that failed a while ago.
pub fn step(state: &mut State, connector: Option<&str>, value: Value, steps: i32) {
    let before = state.displays.adjustable(Value::Brightness);
    let mut jobs = state.displays.step(connector, value, steps);
    jobs.extend(state.displays.retry(Instant::now(), true));
    send(state, jobs);
    schedule_retry(state);
    changed(state, before);
}

pub fn set(state: &mut State, connector: &str, value: Value, level: u8) {
    let before = state.displays.adjustable(Value::Brightness);
    let jobs = state
        .displays
        .set(connector, value, level)
        .into_iter()
        .collect();
    send(state, jobs);
    changed(state, before);
}

pub fn report(state: &mut State, r: DisplayReport) {
    let before = state.displays.adjustable(Value::Brightness);
    if state.displays.report(&r) {
        changed(state, before);
    }
    let jobs = state.displays.retry(Instant::now(), false);
    send(state, jobs);
    schedule_retry(state);
}

/// Arm one timer for the next display due a read again; with none, no timer runs.
fn schedule_retry(state: &mut State) {
    if let Some(t) = state.displays.timer.take() {
        state.handle.remove(t);
    }
    let Some(at) = state.displays.next_retry() else {
        return;
    };
    let added = state
        .handle
        .insert_source(Timer::from_deadline(at), |_, _, state| {
            state.displays.timer = None;
            let jobs = state.displays.retry(Instant::now(), false);
            send(state, jobs);
            schedule_retry(state);
            TimeoutAction::Drop
        });
    match added {
        Ok(t) => state.displays.timer = Some(t),
        Err(e) => log::warn!("displays: no retry timer: {}", e.error),
    }
}

fn send(state: &mut State, jobs: Vec<DisplayJob>) {
    for j in jobs {
        state.send_job(Job::Display(j));
    }
}

/// A level changed: redraw the OSD if it shows and the open panel, and the taskbars when the displays cell may come or go.
fn changed(state: &mut State, adjustable_before: bool) {
    state.update_display_osd();
    crate::displaysview::refresh(state);
    if state.displays.adjustable(Value::Brightness) != adjustable_before {
        state.redraw_taskbars();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Value = Value::Brightness;

    fn head(c: &str, make: &str, model: &str) -> Head {
        Head {
            connector: c.into(),
            make: make.into(),
            model: model.into(),
        }
    }

    /// Answer a fresh read of `connector` with `level` (None: no answer).
    fn read(d: &mut Displays, connector: &str, level: Option<u8>, floor: u8) {
        let seq = d
            .reread()
            .into_iter()
            .find(|j| j.display() == connector)
            .expect("the display is listed")
            .seq();
        d.report(&DisplayReport {
            display: connector.into(),
            value: B,
            seq,
            outcome: Outcome::Read(level.map(|level| Reading { level, floor })),
        });
    }

    fn reply(connector: &str, seq: u64, outcome: Outcome) -> DisplayReport {
        DisplayReport {
            display: connector.into(),
            value: B,
            seq,
            outcome,
        }
    }

    fn set_job(display: &str, level: u8, seq: u64) -> DisplayJob {
        DisplayJob::Set {
            display: display.into(),
            value: B,
            level,
            seq,
        }
    }

    fn levels(d: &Displays) -> Vec<Level> {
        d.list.iter().map(|x| x.level(B)).collect()
    }

    fn connectors(d: &Displays) -> Vec<&str> {
        d.list.iter().map(|x| x.connector.as_str()).collect()
    }

    /// Displays named by `connectors`, each at `At(level)` with floor 1.
    fn at(list: &[(&str, u8)]) -> Displays {
        let mut d = Displays::default();
        let heads: Vec<Head> = list.iter().map(|(c, _)| head(c, "", "")).collect();
        d.sync(&heads);
        for (c, l) in list {
            read(&mut d, c, Some(*l), 1);
        }
        d
    }

    fn seq_of(jobs: &[DisplayJob]) -> u64 {
        assert_eq!(jobs.len(), 1, "one job: {jobs:?}");
        jobs[0].seq()
    }

    #[test]
    fn a_wheel_notch_steps_once_and_down_dims() {
        let mut c = (0, 0.0);
        assert_eq!(wheel_steps(-120, -15.0, &mut c), 1);
        assert_eq!(wheel_steps(120, 15.0, &mut c), -1);
        assert_eq!(wheel_steps(60, 7.5, &mut c), 0);
        assert_eq!(
            wheel_steps(60, 7.5, &mut c),
            -1,
            "two half notches make one"
        );
    }

    #[test]
    fn a_touchpad_swipe_steps_once_per_notch_of_travel() {
        let mut c = (0, 0.0);
        assert_eq!(wheel_steps(0, 7.0, &mut c), 0);
        assert_eq!(wheel_steps(0, 9.0, &mut c), -1);
        assert_eq!(wheel_steps(0, -40.0, &mut c), 2);
        assert_eq!(wheel_steps(0, 0.0, &mut c), 0);
    }

    #[test]
    fn the_cell_tip_lists_each_display_then_night_light() {
        assert_eq!(
            tip_text(
                vec!["Built-in display: 60%".into(), "DELL U2720Q: 40%".into()],
                Some("Night light: 3400 K".into())
            ),
            "Built-in display: 60%\nDELL U2720Q: 40%\nNight light: 3400 K"
        );
        assert_eq!(tip_text(vec!["DP-1: 50%".into()], None), "DP-1: 50%");
    }

    #[test]
    fn labels_name_the_built_in_panel_then_the_model_then_the_make_then_the_connector() {
        assert_eq!(
            labels(&[
                head("eDP-1", "BOE", "0x095F"),
                head("DP-2", "Dell Inc.", "DELL U2720Q"),
                head("HDMI-A-1", "GSM", "Unknown"),
                head("DP-3", "Unknown", "")
            ]),
            ["Built-in display", "DELL U2720Q", "GSM", "DP-3"]
        );
    }

    #[test]
    fn two_monitors_with_one_label_get_their_connectors() {
        assert_eq!(
            labels(&[
                head("DP-1", "Dell Inc.", "DELL U2720Q"),
                head("DP-2", "Dell Inc.", "DELL U2720Q"),
                head("eDP-1", "", "")
            ]),
            [
                "DELL U2720Q (DP-1)",
                "DELL U2720Q (DP-2)",
                "Built-in display"
            ]
        );
    }

    #[test]
    fn sync_keeps_known_displays_and_reads_new_ones() {
        let mut d = Displays::default();
        let edp = head("eDP-1", "BOE", "0x095F");
        let dp = head("DP-2", "Dell Inc.", "DELL U2720Q");
        let jobs = d.sync(std::slice::from_ref(&edp));
        assert!(
            matches!(&jobs[..], [DisplayJob::Read { display, .. }] if display == "eDP-1"),
            "one read for eDP-1: {jobs:?}"
        );
        read(&mut d, "eDP-1", Some(60), 1);

        let jobs = d.sync(&[edp.clone(), dp.clone()]);
        assert!(
            matches!(&jobs[..], [DisplayJob::Read { display, .. }] if display == "DP-2"),
            "one read for DP-2: {jobs:?}"
        );
        assert_eq!(levels(&d), [Level::At(60), Level::Unread]);
        assert_eq!(d.list[0].label, "Built-in display");
        assert!(d.list[0].internal && !d.list[1].internal);

        assert_eq!(d.sync(std::slice::from_ref(&dp)), []);
        assert_eq!(connectors(&d), ["DP-2"]);

        let jobs = d.sync(&[dp, edp]);
        assert!(
            matches!(&jobs[..], [DisplayJob::Read { display, .. }] if display == "eDP-1"),
            "a read for eDP-1 again: {jobs:?}"
        );
        assert_eq!(connectors(&d), ["DP-2", "eDP-1"]);
        assert_eq!(d.list[1].level(B), Level::Unread);
    }

    #[test]
    fn a_step_moves_each_display_from_its_own_level() {
        let mut d = Displays::default();
        d.sync(&[
            head("A", "", ""),
            head("B", "", ""),
            head("C", "", ""),
            head("D", "", ""),
        ]);
        read(&mut d, "A", Some(60), 1);
        read(&mut d, "B", Some(40), 0);
        read(&mut d, "D", None, 0);

        let jobs = d.step(None, B, 1);
        let seqs: Vec<u64> = jobs.iter().map(|j| j.seq()).collect();
        assert_eq!(jobs, [set_job("A", 65, seqs[0]), set_job("B", 45, seqs[1])]);
        assert!(seqs[0] < seqs[1]);
        assert_eq!(
            levels(&d),
            [Level::At(65), Level::At(45), Level::Unread, Level::Fixed]
        );

        let jobs = d.step(Some("B"), B, -2);
        assert_eq!(jobs, [set_job("B", 35, jobs[0].seq())]);
        assert!(jobs[0].seq() > seqs[1]);
        assert_eq!(d.list[0].level(B), Level::At(65));
    }

    #[test]
    fn a_set_moves_only_an_adjustable_display_and_only_when_the_level_changes() {
        let mut d = at(&[("A", 60)]);
        d.sync(&[head("A", "", ""), head("C", "", ""), head("D", "", "")]);
        read(&mut d, "D", None, 0);

        let job = d.set("A", B, 30).expect("a set for A");
        assert_eq!(job, set_job("A", 30, job.seq()));
        assert_eq!(d.list[0].level(B), Level::At(30));
        assert_eq!(d.set("A", B, 30), None, "already at 30");
        assert_eq!(
            d.set("A", B, 0)
                .map(|j| d.list[0].level(B) == Level::At(1) && j.seq() > job.seq()),
            Some(true),
            "a set below the floor lands on it, with a newer seq"
        );
        assert_eq!(d.set("C", B, 50), None, "unread");
        assert_eq!(d.set("D", B, 50), None, "not adjustable");
        assert_eq!(d.set("E", B, 50), None, "not listed");

        let s = d.set("A", B, 80).unwrap().seq();
        assert!(!d.report(&reply(
            "A",
            s - 1,
            Outcome::Read(Some(Reading {
                level: 30,
                floor: 1
            }))
        )));
        assert_eq!(
            d.list[0].level(B),
            Level::At(80),
            "a read sent before the set is stale"
        );
    }

    #[test]
    fn a_step_down_stops_at_the_floor_a_read_gave() {
        let mut d = at(&[("A", 3)]);
        let jobs = d.step(None, B, -1);
        assert_eq!(jobs, [set_job("A", 1, seq_of(&jobs))]);
        assert_eq!(d.step(None, B, -1), []);
        assert_eq!(d.list[0].level(B), Level::At(1));
    }

    #[test]
    fn a_read_older_than_the_last_set_is_ignored() {
        let mut d = at(&[("A", 60)]);
        let r = seq_of(&d.reread());
        let s = seq_of(&d.step(None, B, 1));
        assert!(s > r);
        let sixty = Some(Reading {
            level: 60,
            floor: 1,
        });
        assert!(!d.report(&reply("A", r, Outcome::Read(sixty))));
        assert_eq!(d.list[0].level(B), Level::At(65));

        let r2 = seq_of(&d.reread());
        assert!(r2 > s);
        let sixty_four = Some(Reading {
            level: 64,
            floor: 1,
        });
        assert!(d.report(&reply("A", r2, Outcome::Read(sixty_four))));
        assert_eq!(d.list[0].level(B), Level::At(64));
    }

    #[test]
    fn a_failed_write_falls_back_to_what_the_display_reads() {
        let mut d = at(&[("A", 60)]);
        let s = seq_of(&d.step(None, B, 1));
        let sixty = Some(Reading {
            level: 60,
            floor: 1,
        });
        assert!(d.report(&reply("A", s, Outcome::Failed(sixty))));
        assert_eq!(d.list[0].level(B), Level::At(60));

        let s1 = seq_of(&d.step(None, B, 1));
        let s2 = seq_of(&d.step(None, B, 1));
        assert!(
            !d.report(&reply("A", s1, Outcome::Failed(None))),
            "a later set superseded it"
        );
        assert!(!d.report(&reply("A", s2, Outcome::Wrote)));
        assert_eq!(d.list[0].level(B), Level::At(70));
    }

    #[test]
    fn a_write_with_no_answer_marks_the_display_not_adjustable() {
        let mut d = at(&[("A", 60)]);
        assert!(d.adjustable(B));
        let s = seq_of(&d.step(None, B, 1));
        assert!(d.report(&reply("A", s, Outcome::Failed(None))));
        assert_eq!(d.list[0].level(B), Level::Fixed);
        assert!(!d.adjustable(B));
        assert_eq!(d.step(None, B, 1), []);
    }

    /// `A` at 60 whose set got no answer, with the retry clock started at the returned instant.
    fn failed() -> (Displays, Instant) {
        let mut d = at(&[("A", 60)]);
        let s = seq_of(&d.step(None, B, 1));
        d.report(&reply("A", s, Outcome::Failed(None)));
        let t0 = Instant::now();
        assert_eq!(d.retry(t0, false), [], "the failed set was the read");
        (d, t0)
    }

    #[test]
    fn a_key_reads_a_display_that_is_not_adjustable_at_most_every_ten_seconds() {
        let (mut d, t0) = failed();
        assert_eq!(d.retry(t0 + Duration::from_secs(9), true), []);
        let jobs = d.retry(t0 + RETRY, true);
        assert!(
            matches!(&jobs[..], [DisplayJob::Read { display, .. }] if display == "A"),
            "one read for A: {jobs:?}"
        );
        assert_eq!(d.retry(t0 + RETRY + Duration::from_secs(9), true), []);
        assert_eq!(seq_of(&d.retry(t0 + RETRY * 2, true)), jobs[0].seq() + 1);
    }

    #[test]
    fn the_timer_reads_a_display_that_is_not_adjustable_with_a_doubling_wait() {
        let (mut d, t0) = failed();
        let mut t = t0;
        let mut waits = Vec::new();
        for _ in 0..7 {
            let next = d.next_retry().expect("a read is scheduled");
            assert_eq!(d.retry(next - Duration::from_millis(1), false), []);
            assert_eq!(d.retry(next, false).len(), 1);
            waits.push((next - t).as_secs());
            t = next;
        }
        assert_eq!(waits, [10, 20, 40, 80, 160, 300, 300]);
    }

    #[test]
    fn a_display_that_reads_again_is_adjustable_and_leaves_the_timer() {
        let (mut d, t0) = failed();
        assert!(!d.adjustable(B));
        let r = seq_of(&d.retry(t0 + RETRY, true));
        let fifty = Some(Reading {
            level: 50,
            floor: 1,
        });
        assert!(d.report(&reply("A", r, Outcome::Read(fifty))));
        assert_eq!(d.retry(t0 + RETRY, false), []);
        assert_eq!(d.next_retry(), None);
        assert!(d.adjustable(B));
        assert_eq!(seq_of(&d.step(None, B, 1)), r + 1);
        assert_eq!(d.list[0].level(B), Level::At(55));
    }

    #[test]
    fn the_tip_and_the_osd_lines_say_each_displays_level() {
        let mut d = Displays::default();
        d.sync(&[
            head("eDP-1", "BOE", "0x095F"),
            head("DP-2", "Dell Inc.", "DELL U2720Q"),
            head("DP-3", "Unknown", "Unknown"),
        ]);
        read(&mut d, "eDP-1", Some(60), 1);
        read(&mut d, "DP-2", None, 0);
        assert_eq!(
            d.tip_lines(B),
            [
                "Built-in display: 60%",
                "DELL U2720Q: not adjustable",
                "DP-3"
            ]
        );
        let lines = d.lines(B);
        assert_eq!(lines.title, "Brightness");
        assert!(std::ptr::eq(lines.lines[0].icon, &icons::LAPTOP));
        assert!(std::ptr::eq(lines.lines[1].icon, &icons::MONITOR));
        assert!(std::ptr::eq(lines.lines[2].icon, &icons::MONITOR));
        let got: Vec<Level> = lines.lines.iter().map(|l| l.level).collect();
        assert_eq!(got, [Level::At(60), Level::Fixed, Level::Unread]);
        assert_eq!(lines.lines[1].label, "DELL U2720Q");
    }
}
