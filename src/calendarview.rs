//! The taskbar clock's month popup on the scene graph: one
//! `reprisekit::controls::calendar` popup panel on the menu layer, its
//! edge against the taskbar's on the screen side of it: its right edge on
//! the clock's along the bottom or top, its foot level with the clock's
//! beside a side bar. A left
//! click on the clock opens it; a second click there, Escape, or a press
//! anywhere else closes it. Its arrows step the month; the days are
//! read-only. A double click on the clock closes it and opens Date and
//! Time Properties instead. Under a look that frames its flyouts as
//! windows the arrow under the pointer lights, and the border has the
//! blurred backdrop under it. A look with an analog clock beside the month
//! ticks it each second while the popup is open.
//!
//! The lock closes it as it engages; an output going away closes the one
//! it opened.

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use reprisekit::access::accesskit::Role;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::calendar::{self, Calendar, PopupHit};
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

/// `State::calendar`: the open popup and the last clock click.
#[derive(Default)]
pub struct CalendarState {
    view: Option<CalendarView>,
    /// Left presses on a bar's clock, paired into its double click.
    clicks: DoubleClick<TreeId>,
}

struct CalendarView {
    model: Calendar,
    /// In `layers.menu`, tagged `SceneOwner::Calendar`: the glass tree,
    /// then the buffer.
    tree: TreeId,
    /// Holds the border's blurred glass, under the buffer.
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// The arrow under the pointer, or `Panel`.
    hot: PopupHit,
    /// Top-left in layout coordinates.
    at: (i32, i32),
    /// The output whose clock opened it, for its scale.
    output: Output,
    /// Moves the analog clock on each second; none in a look without one.
    tick: Option<RegistrationToken>,
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

fn open(state: &mut State, bar: TreeId) {
    close(state);
    if state.is_locked() {
        return;
    }
    let Some((output, clock)) = state.bar(bar).map(|b| (b.output.clone(), b.layout.clock)) else {
        return;
    };
    let (y, m, d) = today();
    let mut model = Calendar::new(y, m, None, reprisefs::locale::first_weekday());
    model.today = Some((y, m, d));
    let ticks = state.config.theme.controls().clock_face > 0;
    model.now = ticks.then(now);
    let b = state.layout_box(Some(&output));
    let p = state.bar_placement(&output);
    let (w, h) = calendar::popup_size(&state.config.theme.controls());
    let clock = crate::raster::Rect::new(
        p.shown.x + clock.x,
        p.shown.y + clock.y,
        clock.width,
        clock.height,
    );
    let anchor = calendar_anchor(&p, clock, (w, h));
    let (x, top) = crate::barplace::bar_popup(&p, anchor, (w, h), b);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    crate::glass::leave_out(scene, tree);
    scene.set_data(tree, Some(SceneOwner::Calendar));
    // Before the buffer, so it lies beneath it.
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    scene.set_position(tree, x, top);
    state.calendar.view = Some(CalendarView {
        model,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        hot: PopupHit::Panel,
        at: (x, top),
        output,
        tick: None,
        drawn: None,
    });
    if ticks {
        arm_tick(state);
    }
    redraw(state);
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
                tick(state);
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
    if let Some(t) = v.tick {
        state.handle.remove(t);
    }
    state.scene.destroy(v.tree);
    state.redraw_all();
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// A press on the popup: the arrows step the month.
pub fn clicked(state: &mut State, hit: PopupHit) {
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    match hit {
        PopupHit::Prev => v.model.step_month(-1),
        PopupHit::Next => v.model.step_month(1),
        PopupHit::Panel => return,
    }
    redraw(state);
}

/// Where layout point (`lx`, `ly`) lands on the open popup.
pub fn hit(state: &State, lx: f64, ly: f64) -> PopupHit {
    match state.calendar.view.as_ref() {
        Some(v) => calendar::popup_hit(
            &state.config.theme.controls(),
            lx.floor() as i32 - v.at.0,
            ly.floor() as i32 - v.at.1,
        ),
        None => PopupHit::Panel,
    }
}

/// The pointer moved over the open popup onto `hit`, or off it (None):
/// a look that lights the arrows redraws when the lit one changes.
pub fn hover(state: &mut State, hit: Option<PopupHit>) {
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    let hot = hit.unwrap_or(PopupHit::Panel);
    if std::mem::replace(&mut v.hot, hot) != hot {
        redraw(state);
    }
}

fn redraw(state: &mut State) {
    let Some(v) = state.calendar.view.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&v.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let base = calendar::draw_popup_base(&theme, &state.text, &v.model, v.hot);
    let mut shown = base.clone();
    if let Some(now) = v.model.now {
        calendar::draw_popup_clock(&theme, &mut shown, &base, now);
    }
    let size = calendar::popup_size(&state.config.theme.controls());
    let pane = glass::flyout_glass(&state.config, size);
    let Some(v) = state.calendar.view.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, v.buffer, &shown, scale);
    v.drawn = Some(Drawn { shown, base, scale });
    glass::place_flyout(scene, v.buffer, &state.config.theme, size);
    v.glass.set(scene, &state.glass, v.glass_tree, pane);
    state.redraw_all();
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
    let (w, h) = calendar::popup_size(cm);
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
        let size = calendar::popup_size(&ControlMetrics::CLASSIC);
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
