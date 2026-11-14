//! The taskbar clock's month popup on the scene graph: one
//! `reprisekit::controls::calendar` popup panel on the menu layer, its
//! edge against the taskbar's on the screen side of it: its right edge on
//! the clock's along the bottom or top, its foot level with the clock's
//! beside a side bar. A left
//! click on the clock opens it; a second click there, Escape, or a press
//! anywhere else closes it. Its arrows step the month. With
//! evolution-data-server installed and `[taskbar] calendar_events` on, it
//! lists a day's events under the month, today's until a click on a day
//! picks another; read off the loop, a late answer dropped, and read again
//! every [`REFRESH`] while the popup stays open. A click on an
//! event opens the calendar program. Otherwise the days are read-only. A
//! double click on the clock closes it and opens Date and Time Properties
//! instead. Under a look that frames its flyouts as
//! windows the arrow under the pointer lights, and the border has the
//! blurred backdrop under it. A look with an analog clock beside the month
//! ticks it each second while the popup is open, and redraws the time line
//! under it when the minute turns.
//!
//! The lock closes it as it engages; an output going away closes the one
//! it opened.

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use reprisekit::access::accesskit::Role;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::calendar::{self, Agenda, Calendar, DayEvent, PopupHit};
use reprisekit::metrics::ControlMetrics;
use reprisekit::pointer::DoubleClick;
use reprisekit::text::TextRenderer;
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Buffer as BufferCoords, Rectangle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROPERTIES: &str = "reprise-datetime";
/// The most characters of an event's summary kept.
const SUMMARY_MAX: usize = 256;
/// The program a click on an event opens when no calendar type has a default.
const GNOME_CALENDAR: &str = "org.gnome.Calendar.desktop";
/// How often the open popup reads its day's events again.
const REFRESH: Duration = Duration::from_secs(60);

/// `State::calendar`: the open popup and the last clock click.
#[derive(Default)]
pub struct CalendarState {
    view: Option<CalendarView>,
    /// Left presses on a bar's clock, paired into its double click.
    clicks: DoubleClick<TreeId>,
    /// Whether evolution-data-server is installed, as last read; None before.
    eds: Option<bool>,
    /// Counts the reads of a day's events; an answer to an older one is dropped.
    asked: u64,
    /// The last read whose answer came back: one is still out while it trails `asked`.
    answered: u64,
}

struct CalendarView {
    model: Calendar,
    /// The bar whose clock opened it, to stand against when it grows.
    bar: TreeId,
    /// What a click on an event opens, read with the first day's events.
    program: Option<reprisefs::apps::AppEntry>,
    /// In `layers.menu`, tagged `SceneOwner::Calendar`: the glass tree,
    /// then the buffer.
    tree: TreeId,
    /// Holds the border's blurred glass, under the buffer.
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// The arrow or event row under the pointer, or `Panel`.
    hot: PopupHit,
    /// Top-left in layout coordinates.
    at: (i32, i32),
    /// The output whose clock opened it, for its scale.
    output: Output,
    /// Moves the analog clock on each second; none in a look without one.
    tick: Option<RegistrationToken>,
    /// Reads the day's events again every [`REFRESH`]; none without the list.
    refresh: Option<RegistrationToken>,
    /// What the buffer shows, and the popup without the clock beneath it, at `scale`: a tick
    /// redraws only the clock's square from them.
    drawn: Option<Drawn>,
}

struct Drawn {
    shown: Canvas,
    base: Canvas,
    scale: i32,
}

fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn today() -> (i32, u32, u32) {
    let tm = reprisefs::locale::localtime(since_epoch().as_secs() as i64);
    (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
}

/// The local time of day: hour 0..23, minute, second.
fn now() -> (u32, u32, u32) {
    let tm = reprisefs::locale::localtime(since_epoch().as_secs() as i64);
    let field = |v: i32| v.max(0) as u32;
    (
        field(tm.tm_hour),
        field(tm.tm_min),
        field(tm.tm_sec).min(59),
    )
}

/// The time line under the analog clock: hours and minutes as the bar's clock words them.
fn time_line() -> String {
    crate::taskbar::time_text(since_epoch().as_secs() as i64)
}

/// Until just past the next whole second, so a tick reads the second it was armed for.
fn to_next_second() -> Duration {
    let into = since_epoch().subsec_nanos();
    Duration::from_nanos(u64::from(1_000_000_000 - into) + 5_000_000)
}

pub fn is_open(state: &State) -> bool {
    state.calendar.view.is_some()
}

pub fn open_on(state: &State) -> Option<Output> {
    state.calendar.view.as_ref().map(|v| v.output.clone())
}

/// A left press on bar `bar`'s clock at layout `at`, `time` (ms): the second of a
/// double click opens Date and Time Properties; otherwise the popup
/// opens, or stays closed when it was open before the press (`was_open`,
/// read before the press closed anything).
pub fn clock_press(state: &mut State, bar: TreeId, at: (i32, i32), time: u32, was_open: bool) {
    let click = state.config.theme.metrics.click(at.0, at.1, time);
    if state.calendar.clicks.press(bar, click) {
        close(state);
        state.spawn_activated(PROPERTIES);
        return;
    }
    if was_open {
        close(state);
    } else {
        open(state, bar);
    }
}

/// The rect the calendar, `size` big, opens from off the clock at
/// `clock` (layout coordinates) on bar `p`: right-aligned to the clock
/// along the bottom or top; beside a side bar, where the clock ends the
/// column, its foot level with the clock's.
fn calendar_anchor(
    p: &crate::barplace::BarPlacement,
    clock: crate::raster::Rect,
    (w, h): (i32, i32),
) -> crate::raster::Rect {
    if p.edge.is_side() {
        crate::raster::Rect::new(clock.x, clock.bottom() - h, clock.width, h)
    } else {
        crate::raster::Rect::new(clock.right() - w, clock.y, w, clock.height)
    }
}

/// Where a popup `size` big stands off bar `bar`'s clock, in layout coordinates.
fn place(state: &State, bar: TreeId, size: (i32, i32)) -> Option<(Output, (i32, i32))> {
    let (output, clock) = state.bar(bar).map(|b| (b.output.clone(), b.layout.clock))?;
    let b = state.layout_box(Some(&output));
    let p = state.bar_placement(&output);
    let clock = crate::raster::Rect::new(
        p.shown.x + clock.x,
        p.shown.y + clock.y,
        clock.width,
        clock.height,
    );
    let anchor = calendar_anchor(&p, clock, size);
    let at = crate::barplace::bar_popup(&p, anchor, size, b);
    Some((output, at))
}

fn open(state: &mut State, bar: TreeId) {
    close(state);
    if state.desktop_locked() {
        return;
    }
    let (y, m, d) = today();
    let mut model = Calendar::new(y, m, None, reprisefs::locale::first_weekday());
    model.today = Some((y, m, d));
    let ticks = state.config.theme.controls().clock_face > 0;
    model.now = ticks.then(now);
    model.time = ticks.then(time_line);
    let events = state.config.taskbar.calendar_events;
    model.agenda =
        (events && state.calendar.eds == Some(true)).then(|| Agenda::new((y, m, d), false));
    let size = calendar::popup_size(&state.config.theme.controls(), model.agenda.is_some());
    let Some((output, (x, top))) = place(state, bar, size) else {
        return;
    };
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    crate::glass::leave_out(scene, tree);
    crate::output::keep_on(scene, tree, &output);
    scene.set_data(tree, Some(SceneOwner::Calendar));
    // Before the buffer, so it lies beneath it.
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    scene.set_position(tree, x, top);
    state.calendar.view = Some(CalendarView {
        model,
        bar,
        program: None,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        hot: PopupHit::Panel,
        at: (x, top),
        output,
        tick: None,
        refresh: None,
        drawn: None,
    });
    if ticks {
        arm_tick(state);
    }
    redraw(state);
    if events {
        read_day(state, (y, m, d));
        arm_refresh(state);
    }
}

fn arm_refresh(state: &mut State) {
    let token = state
        .handle
        .insert_source(Timer::from_duration(REFRESH), |_, _, state| refresh(state));
    match token {
        Ok(t) => {
            if let Some(v) = state.calendar.view.as_mut() {
                v.refresh = Some(t);
            }
        }
        Err(e) => log::warn!("calendar events: no timer: {}", e.error),
    }
}

/// The refresh timer's turn: the open popup's day is read again unless a read is still out. A
/// popup without a list, or none, stops it.
fn refresh(state: &mut State) -> TimeoutAction {
    let Some(v) = state.calendar.view.as_mut() else {
        return TimeoutAction::Drop;
    };
    let Some(day) = v.model.agenda.as_ref().map(|a| a.day) else {
        v.refresh = None;
        return TimeoutAction::Drop;
    };
    if state.calendar.answered == state.calendar.asked {
        read_day(state, day);
    }
    TimeoutAction::ToDuration(REFRESH)
}

/// Checks off the loop whether evolution-data-server is installed, so the first popup opens at its
/// size; starts nothing.
pub fn setup(state: &mut State) {
    if !state.config.taskbar.calendar_events {
        return;
    }
    state.spawn_job(|| {
        let installed =
            reprisebus::session().is_ok_and(|c| reprisebus::wait(reprisebus::eds::installed(&c)));
        Box::new(move |state: &mut State| {
            state.calendar.eds.get_or_insert(installed);
        })
    });
}

/// Reads day `day`'s events off the loop for the open popup, and on the first read what a click on
/// one opens.
fn read_day(state: &mut State, day: (i32, u32, u32)) {
    state.calendar.asked += 1;
    let asked = state.calendar.asked;
    let apps = (state.calendar.view.as_ref())
        .is_some_and(|v| v.program.is_none())
        .then(|| state.app_index.clone());
    state.spawn_job(move || {
        let read = read_events(day);
        let program = apps.and_then(|a| calendar_program(&a).cloned());
        Box::new(move |state: &mut State| landed(state, asked, day, read, program))
    });
}

#[cfg(not(test))]
fn read_events(day: (i32, u32, u32)) -> Result<Option<reprisebus::eds::Day>, String> {
    let conn = reprisebus::session().map_err(|e| e.to_string())?;
    reprisebus::wait(reprisebus::eds::day(&conn, day))
}

/// Tests read no session bus: every day is empty.
#[cfg(test)]
fn read_events(_day: (i32, u32, u32)) -> Result<Option<reprisebus::eds::Day>, String> {
    Ok(Some(reprisebus::eds::Day {
        start: 0,
        end: 0,
        events: Vec::new(),
    }))
}

/// The program a click on an event opens: the default for calendar files, else for webcal links,
/// else GNOME Calendar.
fn calendar_program(apps: &[reprisefs::apps::AppEntry]) -> Option<&reprisefs::apps::AppEntry> {
    reprisefs::props::own_default_app("text/calendar", apps)
        .or_else(|| reprisefs::props::default_app("x-scheme-handler/webcal", apps))
        .map(|i| &apps[i])
        .or_else(|| apps.iter().find(|a| a.id == GNOME_CALENDAR))
}

/// Read `asked` came back: the popup lists `read`'s events, or drops its list when
/// evolution-data-server is not installed. A failed read leaves the rows blank.
fn landed(
    state: &mut State,
    asked: u64,
    day: (i32, u32, u32),
    read: Result<Option<reprisebus::eds::Day>, String>,
    program: Option<reprisefs::apps::AppEntry>,
) {
    if asked != state.calendar.asked {
        return;
    }
    state.calendar.answered = asked;
    match &read {
        Ok(found) => state.calendar.eds = Some(found.is_some()),
        Err(e) => log::warn!("calendar events: {e}"),
    }
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    if program.is_some() {
        v.program = program;
    }
    let opens = v.program.is_some();
    match read {
        Ok(None) => v.model.agenda = None,
        Ok(Some(d)) => {
            let a = v
                .model
                .agenda
                .get_or_insert_with(|| Agenda::new(day, opens));
            a.opens = opens;
            let bounds = (d.start, d.end);
            a.set_events(
                (d.events.iter())
                    .map(|o| day_event(o, bounds, crate::taskbar::time_text))
                    .collect(),
            );
        }
        Err(_) => {}
    }
    redraw(state);
}

/// Occurrence `o` as a row of the day `from..to` (seconds): all day when it covers the day, else at
/// its start, or until its end when it began the day before. `time` words a time of day.
fn day_event(
    o: &reprisebus::eds::Occurrence,
    (from, to): (i64, i64),
    time: impl Fn(i64) -> String,
) -> DayEvent {
    let all_day = o.all_day || (o.start <= from && o.end >= to);
    let when = if all_day {
        "All day".to_string()
    } else if o.start >= from {
        time(o.start)
    } else {
        format!("Until {}", time(o.end))
    };
    DayEvent {
        all_day,
        start: o.start.max(from),
        when,
        summary: match reprisefs::names::shown_line(&o.summary, SUMMARY_MAX).trim() {
            "" => "Untitled event".into(),
            s => s.to_string(),
        },
    }
}

fn arm_tick(state: &mut State) {
    let token =
        state
            .handle
            .insert_source(Timer::from_duration(to_next_second()), |_, _, state| {
                let Some(v) = state.calendar.view.as_mut() else {
                    return TimeoutAction::Drop;
                };
                v.model.now = Some(now());
                let time = Some(time_line());
                if v.model.time != time {
                    v.model.time = time;
                    redraw(state);
                } else {
                    tick(state);
                }
                TimeoutAction::ToDuration(to_next_second())
            });
    match token {
        Ok(t) => {
            if let Some(v) = state.calendar.view.as_mut() {
                v.tick = Some(t);
            }
        }
        Err(e) => log::warn!("calendar clock: no timer: {}", e.error),
    }
}

pub fn close(state: &mut State) {
    let Some(v) = state.calendar.view.take() else {
        return;
    };
    for t in [v.tick, v.refresh].into_iter().flatten() {
        state.handle.remove(t);
    }
    state.destroy_redrawn(v.tree);
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// A press on the popup: the arrows step the month, a day lists its events, an event opens the
/// calendar program and closes the popup.
pub fn clicked(state: &mut State, hit: PopupHit) {
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    match hit {
        PopupHit::Prev => v.model.step_month(-1),
        PopupHit::Next => v.model.step_month(1),
        PopupHit::Day(d) => {
            let day = (v.model.year, v.model.month, d);
            let Some(a) = v.model.agenda.as_mut().filter(|a| a.day != day) else {
                return;
            };
            a.day = day;
            a.events = None;
            redraw(state);
            return read_day(state, day);
        }
        PopupHit::Event(_) => {
            let program = v.program.clone();
            close(state);
            if let Some(app) = program {
                state.launch_entry(&app);
            }
            return;
        }
        PopupHit::Panel => return,
    }
    redraw(state);
}

/// Where layout point (`lx`, `ly`) lands on the open popup.
pub fn hit(state: &State, lx: f64, ly: f64) -> PopupHit {
    match state.calendar.view.as_ref() {
        Some(v) => calendar::popup_hit(
            &state.config.theme.controls(),
            &v.model,
            lx.floor() as i32 - v.at.0,
            ly.floor() as i32 - v.at.1,
        ),
        None => PopupHit::Panel,
    }
}

/// The pointer moved over the open popup onto `hit`, or off it (None):
/// it redraws when the lit arrow or event row changes.
pub fn hover(state: &mut State, hit: Option<PopupHit>) {
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    let hot = match hit {
        Some(h @ (PopupHit::Prev | PopupHit::Next | PopupHit::Event(_))) => h,
        _ => PopupHit::Panel,
    };
    if std::mem::replace(&mut v.hot, hot) != hot {
        redraw(state);
    }
}

fn redraw(state: &mut State) {
    let Some(v) = state.calendar.view.as_ref() else {
        return;
    };
    let size = calendar::popup_size(&state.config.theme.controls(), v.model.agenda.is_some());
    // The list comes or goes with the first read: the popup grows or shrinks against the bar.
    if let Some((_, at)) = place(state, v.bar, size)
        && at != v.at
        && let Some(v) = state.calendar.view.as_mut()
    {
        v.at = at;
        state.scene.set_position(v.tree, at.0, at.1);
    }
    let Some(v) = state.calendar.view.as_ref() else {
        return;
    };
    let tree = v.tree;
    let was = state.scene.outputs_of(tree);
    let scale = state.draw_scale_at(Some(&v.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let base =
        calendar::draw_popup_base(&theme, state.text.for_output(theme.scale), &v.model, v.hot);
    let mut shown = base.clone();
    if let Some(now) = v.model.now {
        calendar::draw_popup_clock(&theme, &mut shown, &base, now);
    }
    let pane = glass::flyout_glass(&state.config, size);
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, v.buffer, &shown, scale);
    v.drawn = Some(Drawn { shown, base, scale });
    glass::place_flyout(scene, v.buffer, &state.config.theme, size);
    v.glass.set(scene, &state.glass, v.glass_tree, pane);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

/// The clock moved on a second: only its square is drawn, compared and uploaded.
fn tick(state: &mut State) {
    let Some(v) = state.calendar.view.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&v.output)).max(1);
    if v.drawn.as_ref().is_none_or(|d| d.scale != scale) {
        return redraw(state);
    }
    let theme = state.config.theme.scaled(scale);
    let (Some(v), Some(clock)) = (state.calendar.view.as_mut(), calendar::popup_clock(&theme))
    else {
        return;
    };
    let (Some(d), Some(now)) = (v.drawn.as_mut(), v.model.now) else {
        return;
    };
    calendar::draw_popup_clock(&theme, &mut d.shown, &d.base, now);
    let (w, h) = (d.shown.width as i32, d.shown.height as i32);
    state
        .scene
        .buffer_set_canvas_within(v.buffer, &d.shown.pixels, w, h, buffer_box(clock));
    let output = v.output.clone();
    crate::backend::schedule(state, &output);
}

/// `r` (canvas px) as a scene buffer box.
fn buffer_box(r: Rect) -> Rectangle<i32, BufferCoords> {
    Rectangle::new((r.x, r.y).into(), (r.width, r.height).into())
}

/// The popup's tree for a screen reader: the month buttons and the grid,
/// which reads today's date and has the focus.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    let v = state.calendar.view.as_ref()?;
    let px = state.config.theme.fonts.size_px;
    let cm = state.config.theme.controls();
    Some(popup_tree(&v.model, &cm, v.at, &state.text, px))
}

/// The tree of a popup showing `model` with its top-left at `at`.
fn popup_tree(
    model: &Calendar,
    cm: &ControlMetrics,
    at: (i32, i32),
    text: &TextRenderer,
    px: u32,
) -> Tree {
    let (w, h) = calendar::popup_size(cm, model.agenda.is_some());
    let rect = crate::raster::Rect::new(at.0, at.1, w, h);
    let mut tree = crate::access::root(Role::Window, "Calendar", rect);
    let focus = calendar::popup_access(model, cm, text, px, &mut tree, ROOT);
    tree.set_focus(focus);
    tree
}

/// The layout changed: a popup whose output went away goes with it.
pub fn output_gone(state: &mut State) {
    let gone = state
        .calendar
        .view
        .as_ref()
        .is_some_and(|v| !state.scene.outputs().contains(&v.output));
    if gone {
        close(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::access::accesskit::Affine;

    #[test]
    fn the_calendar_opens_beside_a_side_bar_level_with_the_clocks_foot() {
        use reprisekit::taskbar::Edge;
        let screen = crate::raster::Rect::new(0, 0, 1920, 1080);
        let size = calendar::popup_size(&ControlMetrics::CLASSIC, false);
        let left = crate::barplace::place(screen, Edge::Left, 60);
        let clock = crate::raster::Rect::new(2, 1030, 54, 40);
        let anchor = calendar_anchor(&left, clock, size);
        assert_eq!(
            crate::barplace::bar_popup(&left, anchor, size, screen),
            (60, 1070 - size.1),
            "right of the bar, its foot level with the clock's"
        );
        let bottom = crate::barplace::place(screen, Edge::Bottom, 30);
        let clock = crate::raster::Rect::new(1840, 1054, 70, 22);
        let anchor = calendar_anchor(&bottom, clock, size);
        assert_eq!(
            anchor.right(),
            1910,
            "right-aligned to the clock, as before"
        );
    }

    #[test]
    fn an_occurrence_reads_at_its_start_all_day_or_until_its_end() {
        let day = (1000, 1000 + 86_400);
        let o = |start: i64, end: i64, all_day: bool| reprisebus::eds::Occurrence {
            summary: "Standup".into(),
            all_day,
            start,
            end,
        };
        let time = |s: i64| format!("t{s}");
        let row = |o| day_event(&o, day, time);
        assert_eq!(row(o(4600, 5500, false)).when, "t4600");
        let carried = row(o(-2000, 3000, false));
        assert_eq!(
            (carried.when.as_str(), carried.start),
            ("Until t3000", 1000)
        );
        let all = row(o(-2000, 90_000, false));
        assert_eq!((all.when.as_str(), all.all_day), ("All day", true));
        assert_eq!(row(o(1000, 87_400, true)).when, "All day");
        let mut blank = o(4600, 4600, false);
        blank.summary = " \u{200b}".into();
        assert_eq!(row(blank).summary, "Untitled event");
        let mut spoof = o(4600, 4600, false);
        spoof.summary = "Pay\u{202e}gpj.exe".into();
        assert_eq!(row(spoof).summary, "Paygpj.exe", "no bidi controls");
    }

    /// The harness has no job threads: each read lands as it is asked.
    #[test]
    fn the_open_popup_reads_its_day_again_and_a_closed_one_reads_nothing() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.calendar.eds = Some(true);
        let bar = h.state.bars[0].tree;
        open(&mut h.state, bar);
        assert_eq!(h.state.calendar.answered, 1, "the first read");
        let rows = |s: &State| {
            let v = s.calendar.view.as_ref()?;
            v.model.agenda.as_ref()?.events.clone()
        };
        assert_eq!(rows(&h.state), Some(Vec::new()));
        assert!(matches!(
            refresh(&mut h.state),
            TimeoutAction::ToDuration(REFRESH)
        ));
        assert_eq!(h.state.calendar.answered, 2, "read again");
        h.state.calendar.asked += 1;
        refresh(&mut h.state);
        assert_eq!(h.state.calendar.asked, 3, "none while a read is out");
        h.state.calendar.answered = 3;
        close(&mut h.state);
        assert!(matches!(refresh(&mut h.state), TimeoutAction::Drop));
        assert_eq!(h.state.calendar.asked, 3, "none while closed");
    }

    #[test]
    fn the_popup_tree_sits_at_the_popup_and_focuses_today() {
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut model = Calendar::new(2026, 10, None, 0);
        model.today = Some((2026, 10, 3));
        let t = popup_tree(&model, &ControlMetrics::CLASSIC, (1100, 560), &text, 11);
        let root = t.get(ROOT).expect("root");
        assert_eq!(root.label(), Some("Calendar"));
        assert_eq!(
            root.transform().map(|a| a.as_coeffs()),
            Some(Affine::translate((1100.0, 560.0)).as_coeffs())
        );
        let run = t.get(t.focus()).unwrap().children()[0];
        assert_eq!(t.get(run).unwrap().value(), Some("October 3, 2026"));
    }
}
