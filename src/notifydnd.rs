//! Do Not Disturb for a while: an hour, or until tomorrow morning, from the bell's menu and the
//! history's check box. A timed one outlives a config reload, never the session.

use reprisekit::controls::menu::{MenuItem, place_popup};
use smithay::output::Output;

use crate::compositor::State;
use crate::menuview::{MenuFrom, MenuKind};
use crate::nightlight::Local;
use crate::nightlight::schedule::LocalClock;
use crate::raster::Rect;
use crate::startmenu::{MenuLayout, MenuStyle};

/// Where "until tomorrow" ends without quiet hours running past midnight, minutes after midnight.
const MORNING: u32 = 8 * 60;
const DAY: i64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Hour,
    /// Until the end of quiet hours that run past midnight, else 08:00, whichever comes next.
    Tomorrow,
}

/// When `span` taken at `now` ends; `quiet` is `[notifications] quiet_hours`.
pub fn end(span: Span, now: i64, quiet: Option<(u32, u32)>, clock: &dyn LocalClock) -> i64 {
    match span {
        Span::Hour => now + 3600,
        Span::Tomorrow => {
            let minute = match quiet {
                Some((from, to)) if to < from => to,
                _ => MORNING,
            };
            next_at(now, minute, clock)
        }
    }
}

/// The first `minute` after local midnight later than `now`.
fn next_at(now: i64, minute: u32, clock: &dyn LocalClock) -> i64 {
    let date = clock.date(now);
    let t = clock.at(date, minute);
    if t > now {
        return t;
    }
    // Noon plus a day is the next date even across a DST change.
    clock.at(clock.date(clock.at(date, 12 * 60) + DAY), minute)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    On,
    For(Span),
    Off,
}

/// The rows: while on, a greyed row saying until when and Turn off; else Do not disturb; then the
/// two timed ones.
fn menu(on: bool, until: Option<i64>) -> (Vec<MenuItem>, Vec<Option<Row>>) {
    let (mut items, mut rows) = if on {
        let status = match until {
            Some(t) => format!("Do not disturb until {}", crate::taskbar::time_text(t)),
            None => "Do not disturb is on".to_string(),
        };
        (
            vec![
                MenuItem::entry(status).enabled(false),
                MenuItem::item("&Turn off").bold(true),
            ],
            vec![None, Some(Row::Off)],
        )
    } else {
        (
            vec![MenuItem::item("&Do not disturb").bold(true)],
            vec![Some(Row::On)],
        )
    };
    items.extend([
        MenuItem::Separator,
        MenuItem::item("Do not disturb for 1 &hour"),
        MenuItem::item("Do not disturb until &tomorrow"),
    ]);
    rows.extend([
        None,
        Some(Row::For(Span::Hour)),
        Some(Row::For(Span::Tomorrow)),
    ]);
    (items, rows)
}

fn rows_now(state: &State) -> (Vec<MenuItem>, Vec<Option<Row>>) {
    let log = &state.notify.log;
    menu(log.dnd, log.dnd_until)
}

/// The bell's menu, opened from `cell` (layout coordinates) on `output` on the screen side of the
/// bar.
pub fn open_from_bell(state: &mut State, output: &Output, cell: Rect) {
    if state.layout_box(Some(output)).width <= 0 {
        return;
    }
    let (items, rows) = rows_now(state);
    let usable = state.usable_area(Some(output));
    let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
        &state.config.theme,
        &state.text,
        &items,
        usable,
    );
    let (x, y) = crate::barplace::bar_popup(
        &state.bar_placement(output),
        cell,
        (layout.width, layout.height),
        usable,
    );
    let kind = MenuKind::Dnd(rows);
    state.push_menu(
        0,
        (x, y),
        layout,
        items,
        output.clone(),
        kind,
        MenuFrom::Bar,
    );
    state.redraw_menu(0);
}

/// The same menu at layout point `at`: the history's check box.
pub fn open_at(state: &mut State, at: (i32, i32)) {
    state.close_menu();
    let Some(output) = state.output_at(f64::from(at.0), f64::from(at.1)) else {
        return;
    };
    let (items, rows) = rows_now(state);
    let usable = state.usable_area(Some(&output));
    let style = MenuStyle::context_for(&state.config.theme, &items).fit(
        &state.text,
        &items,
        state.config.theme.fonts.size_px,
        usable.width,
    );
    let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let pos = place_popup(at, (layout.width, layout.height), usable);
    let kind = MenuKind::Dnd(rows);
    state.push_menu(0, pos, layout, items, output, kind, MenuFrom::Elsewhere);
    state.redraw_menu(0);
}

pub fn pick(state: &mut State, row: Row) {
    match row {
        Row::On => crate::notifyview::set_dnd(state, true),
        Row::Off => crate::notifyview::set_dnd(state, false),
        Row::For(span) => {
            let now = crate::nightlight::now();
            let quiet = state.config.notifications.quiet_hours;
            let until = end(span, now, quiet, &Local);
            crate::notifyview::set_dnd_until(state, true, Some(until));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local time is UTC.
    struct Utc;

    impl LocalClock for Utc {
        fn date(&self, t: i64) -> (i32, i32, i32) {
            (0, 0, t.div_euclid(DAY) as i32)
        }
        fn at(&self, date: (i32, i32, i32), minute: u32) -> i64 {
            i64::from(date.2) * DAY + i64::from(minute) * 60
        }
    }

    const DAY0: i64 = 20_000 * DAY;

    fn at(h: i64, m: i64) -> i64 {
        DAY0 + h * 3600 + m * 60
    }

    #[test]
    fn an_hour_ends_an_hour_on_and_tomorrow_ends_at_the_next_morning() {
        assert_eq!(end(Span::Hour, at(22, 10), None, &Utc), at(23, 10));
        assert_eq!(end(Span::Tomorrow, at(22, 10), None, &Utc), at(32, 0));
        assert_eq!(
            end(Span::Tomorrow, at(2, 0), None, &Utc),
            at(8, 0),
            "after midnight, this morning"
        );
        assert_eq!(end(Span::Tomorrow, at(8, 0), None, &Utc), at(32, 0));
    }

    #[test]
    fn tomorrow_ends_with_quiet_hours_that_run_past_midnight() {
        let night = Some((22 * 60, 6 * 60 + 30));
        assert_eq!(end(Span::Tomorrow, at(15, 0), night, &Utc), at(30, 30));
        assert_eq!(end(Span::Tomorrow, at(23, 0), night, &Utc), at(30, 30));
        let day = Some((9 * 60, 17 * 60));
        assert_eq!(
            end(Span::Tomorrow, at(15, 0), day, &Utc),
            at(32, 0),
            "daytime quiet hours leave it at 08:00"
        );
    }

    fn labels(items: &[MenuItem]) -> Vec<(String, bool)> {
        (items.iter())
            .map(|i| match i {
                MenuItem::Entry { label, enabled, .. } => (label.clone(), *enabled),
                _ => ("-".into(), false),
            })
            .collect()
    }

    #[test]
    fn the_menu_turns_on_for_good_or_for_a_while_and_says_until_when() {
        let (items, rows) = menu(false, None);
        assert_eq!(
            labels(&items),
            [
                ("Do not disturb".into(), true),
                ("-".into(), false),
                ("Do not disturb for 1 hour".into(), true),
                ("Do not disturb until tomorrow".into(), true),
            ]
        );
        assert_eq!(
            rows,
            [
                Some(Row::On),
                None,
                Some(Row::For(Span::Hour)),
                Some(Row::For(Span::Tomorrow))
            ]
        );
        let (items, rows) = menu(true, Some(0));
        let shown = labels(&items);
        assert_eq!(
            shown[0],
            (
                format!("Do not disturb until {}", crate::taskbar::time_text(0)),
                false
            ),
            "the end time is a greyed row"
        );
        assert_eq!(shown[1], ("Turn off".into(), true));
        assert_eq!(rows[..2], [None, Some(Row::Off)]);
        assert_eq!(labels(&menu(true, None).0)[0].0, "Do not disturb is on");
    }

    #[test]
    fn a_timed_do_not_disturb_outlives_a_reload_and_ends_by_itself() {
        let mut h = crate::harness::Harness::new();
        pick(&mut h.state, Row::For(Span::Hour));
        let until = h.state.notify.log.dnd_until.expect("timed");
        assert!(h.state.notify.log.dnd);
        assert!((until - crate::nightlight::now() - 3600).abs() <= 2);
        let cfg = h.state.config.clone();
        crate::configwatch::reload(&mut h.state, cfg);
        assert_eq!(
            h.state.notify.log.dnd_until,
            Some(until),
            "a reload keeps it"
        );
        pick(&mut h.state, Row::Off);
        assert!(!h.state.notify.log.dnd);
        assert_eq!(h.state.notify.log.dnd_until, None, "turned off early");
        let past = crate::nightlight::now() - 1;
        crate::notifyview::set_dnd_until(&mut h.state, true, Some(past));
        h.run_for(50);
        assert!(!h.state.notify.log.dnd, "it ends once its time is up");
        assert_eq!(h.state.notify.log.dnd_until, None);
        pick(&mut h.state, Row::For(Span::Tomorrow));
        crate::notifyview::set_dnd(&mut h.state, false);
        assert_eq!(
            h.state.notify.log.dnd_until, None,
            "the keybind's plain toggle ends it"
        );
    }
}
