//! The tray arrow in looks whose shape has one
//! (`TaskbarShape::tray_arrow`): a left click (on the release over the
//! arrow) slides the hidden icons out between the arrow and the shown
//! ones, and a second click slides them back, as does the pointer leaving the bar's notification area (the
//! arrow, the tray and its well) for `IDLE_MS`, each bar on its own
//! timer. A run opened with the
//! pointer elsewhere (from the keyboard) stays out until the pointer has
//! passed over the area and left it, or under auto-hide until the
//! keyboard leaves the bar (`keys_gone`). Each bar keeps its own run
//! (`Bar::tray_open`, `Bar::tray_reveal`); a timer ticks every `TICK_MS`
//! while any run moves. Looks without the arrow keep the ^ flyout
//! (`trayflyout`).

use std::time::{Duration, Instant};

use reprisekit::{
    anim::{Ease, Slide},
    taskbar::{TaskbarHit, TaskbarShape},
};
use reprisescene::TreeId;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{compositor::State, input::Interaction, taskbarview::Bar};

const TICK_MS: u64 = 16;
/// How long a run takes to open or shut on a Bottom or Top bar. A side
/// bar's run opens and shuts at once.
const SLIDE_MS: u32 = 150;
/// How long an open run stays out once the pointer has left its bar's
/// notification area.
const IDLE_MS: u64 = 1500;
/// How often the idle timer looks again while a popup holds it.
const HELD_POLL_MS: u64 = 100;

/// Whether the look's ^ cell is the inline arrow rather than the flyout's.
pub fn arrow_look(state: &State) -> bool {
    state.config.theme.taskbar_shape().tray_arrow.is_some()
}

/// How many items wait behind the arrow; 0 while the bar shows none.
pub fn hidden(state: &State) -> usize {
    if crate::tray::chevron_shown(state) {
        state.tray.as_ref().map_or(0, |t| t.hidden.len())
    } else {
        0
    }
}

/// How many hidden cells a bar draws with its run opened `reveal` px, of
/// `hidden`: whole cells inside the run along a Bottom or Top bar, every
/// one of them on a side bar once the run is open at all. The tray's
/// pictures (`tray::bar_icons`) take the same count as the layout.
pub fn cells(side: bool, shape: &TaskbarShape, hidden: usize, reveal: i32) -> usize {
    if side {
        if reveal > 0 { hidden } else { 0 }
    } else {
        reprisekit::taskbar::revealed(shape, hidden, reveal)
    }
}

/// Keep a run in step with the hidden items: an open run shuts at once
/// when `full` is 0 (nothing hidden, no room, overflow off, a look without
/// the arrow), drops at once to a `full` it is past, since the layout
/// draws no more than `full`, and otherwise slides on to `full`. Returns
/// whether the run moves on from `now`.
fn follow(open: &mut bool, slide: &mut Slide, full: i32, now: Instant) -> bool {
    if !*open {
        return false;
    }
    if full == 0 {
        *open = false;
        *slide = Slide::at(0);
        return false;
    }
    let target = slide.x(now) - slide.offset(now);
    if slide.x(now) >= full {
        if target != full || !slide.settled(now) {
            *slide = Slide::at(full);
        }
    } else if target != full {
        slide.retarget(full, now);
    }
    !slide.settled(now)
}

/// A left press on bar `tree`'s arrow: it draws held, and the release
/// over it ([`release`]) opens or shuts the run.
pub fn press(state: &mut State, tree: TreeId) {
    state.interaction = Interaction::ArrowPress { bar: tree };
    set_held(state, tree, true);
}

/// The pointer moved with bar `tree`'s arrow held: it draws held, and
/// lit, only while the pointer is on it, so dragging off shows it
/// released in every look.
pub fn press_moved(state: &mut State, tree: TreeId) {
    let on = on_arrow(state, tree);
    set_look(state, tree, on, on);
}

/// The press on bar `tree`'s arrow came up: over the arrow it opens or
/// shuts the run; elsewhere it does nothing.
pub fn release(state: &mut State, tree: TreeId) {
    set_held(state, tree, false);
    if on_arrow(state, tree) {
        toggle(state, tree);
    }
}

fn on_arrow(state: &State, tree: TreeId) -> bool {
    let p = state.pointer_location();
    state
        .bar(tree)
        .is_some_and(|b| state.bar_hit(b, p.x, p.y) == TaskbarHit::Chevron)
}

/// Light bar `tree`'s arrow only while the pointer is on it where it is
/// drawn now: a click that opens or shuts the run moves the arrow, and
/// it goes to rest once it has slid out from under a still pointer.
/// While the arrow is held, `press_moved` owns its look.
fn relight(state: &mut State, tree: TreeId) {
    let Some(hot) = state.bar(tree).map(|b| b.chevron_hot) else {
        return;
    };
    let held = matches!(state.interaction, Interaction::ArrowPress { bar } if bar == tree);
    let lit = lit_after_slide(held, hot, on_arrow(state, tree));
    if lit != hot {
        let held = state.bar(tree).is_some_and(|b| b.chevron_held);
        set_look(state, tree, lit, held);
    }
}

/// Whether an arrow lit `hot` stays lit after its run moved, the pointer
/// now `on` it or not: a held arrow keeps its look, any other is lit
/// exactly while the pointer is on it.
fn lit_after_slide(held: bool, hot: bool, on: bool) -> bool {
    if held { hot } else { on }
}

pub fn set_held(state: &mut State, tree: TreeId, held: bool) {
    let hot = state.bar(tree).is_some_and(|b| b.chevron_hot);
    set_look(state, tree, hot, held);
}

fn set_look(state: &mut State, tree: TreeId, hot: bool, held: bool) {
    if let Some(b) =
        (state.bar_mut(tree)).filter(|b| (b.chevron_hot, b.chevron_held) != (hot, held))
    {
        (b.chevron_hot, b.chevron_held) = (hot, held);
        state.redraw_taskbar(tree);
    }
}

/// Open bar `tree`'s run, or shut it: a click on its arrow, or Enter on
/// it from the bar keys, which acts on a bar still sliding in too.
pub fn toggle(state: &mut State, tree: TreeId) {
    let Some(open) = state.bar(tree).map(|b| !b.tray_open) else {
        return;
    };
    set(state, tree, open);
}

fn set(state: &mut State, tree: TreeId, open: bool) {
    let Some((full, side)) = state.bar(tree).map(|b| {
        let side = state.bar_placement(&b.output).edge.is_side();
        (b.layout.run_width, side)
    }) else {
        return;
    };
    let ms = if side { 0 } else { SLIDE_MS };
    let now = Instant::now();
    let Some(bar) = state.bar_mut(tree) else {
        return;
    };
    let open = open && full > 0;
    bar.tray_open = open;
    let from = bar.tray_reveal.x(now);
    bar.tray_reveal = Slide::new(from, if open { full } else { 0 }, now, ms, Ease::Out);
    state.redraw_taskbar(tree);
    arm_tick(state);
}

/// Shut every open run at once: the bar moved, or the session locked.
pub fn collapse_all(state: &mut State) {
    let trees: Vec<TreeId> = (state.bars.iter())
        .filter(|b| b.tray_open || b.tray_reveal.x(Instant::now()) != 0)
        .map(|b| b.tree)
        .collect();
    for tree in trees {
        if let Some(bar) = state.bar_mut(tree) {
            bar.tray_open = false;
            bar.tray_reveal = Slide::at(0);
        }
        state.redraw_taskbar(tree);
    }
    for (_, t) in std::mem::take(&mut state.bar_shared.reveal_idle) {
        state.handle.remove(t);
    }
}

/// Keep `bar`'s run heading for the width its new layout opens fully
/// (`TaskbarLayout::run_width`, see [`follow`]), so a run capped by the
/// bar's room stops sliding where its cells stop. `bar` is out of
/// `state.bars` while it draws.
pub fn follow_layout(state: &mut State, bar: &mut Bar, now: Instant) {
    let full = bar.layout.run_width;
    if follow(&mut bar.tray_open, &mut bar.tray_reveal, full, now) {
        arm_tick(state);
    }
}

/// What the idle timers do as the pointer goes from the notification
/// area of bar `was` to that of bar `now` (None: off every area): stop
/// `now`'s timer if it is `pending`, and arm `was`'s if its run is
/// `open`. Each bar's timer is its own, so going from one bar's area
/// straight to another's neither restarts nor stops the first one's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IdleStep<T> {
    cancel: Option<T>,
    arm: Option<T>,
}

fn idle_step<T: Copy + PartialEq>(
    was: Option<T>,
    now: Option<T>,
    pending: impl Fn(T) -> bool,
    open: impl Fn(T) -> bool,
) -> IdleStep<T> {
    if was == now {
        return IdleStep {
            cancel: None,
            arm: None,
        };
    }
    IdleStep {
        cancel: now.filter(|&t| pending(t)),
        arm: was.filter(|&t| open(t)),
    }
}

/// The pointer moved onto bar `area`'s notification area (the arrow, the
/// tray and its well), or off every one (None). Leaving an open run's
/// area arms its bar's idle timer, which shuts that run; coming back
/// onto it stops the timer. An access click's stand-in press moves
/// nothing.
pub fn hover(state: &mut State, area: Option<TreeId>) {
    if state.access.click_at.is_some() {
        return;
    }
    let was = std::mem::replace(&mut state.bar_shared.tray_pointer, area);
    let step = {
        let idle = &state.bar_shared.reveal_idle;
        let pending = |t: TreeId| idle.iter().any(|&(b, _)| b == t);
        let open = |t: TreeId| state.bar(t).is_some_and(|b| b.tray_open);
        idle_step(was, area, pending, open)
    };
    if let Some(tree) = step.cancel {
        disarm_idle(state, tree);
    }
    if let Some(tree) = step.arm {
        arm_idle(state, tree);
    }
}

fn disarm_idle(state: &mut State, tree: TreeId) {
    let idle = &mut state.bar_shared.reveal_idle;
    if let Some(i) = idle.iter().position(|&(b, _)| b == tree) {
        let (_, t) = idle.swap_remove(i);
        state.handle.remove(t);
    }
}

/// What a bar's idle timer does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fire {
    /// The run is shut already, or the pointer is back on its area: stop.
    Done,
    /// A menu or tray popup on the run's bar is open: look again in
    /// `HELD_POLL_MS`.
    Wait,
    /// The popup that held it closed: the whole `IDLE_MS` again, from
    /// now.
    Rearm,
    /// Shut the run.
    Shut,
}

/// [`Fire`] for a timer whose run is `open`, with the pointer `on` its
/// area or not, that was `was_held` by a popup at its last look and is
/// `held` now.
fn idle_fire(open: bool, on: bool, was_held: bool, held: bool) -> Fire {
    match (was_held, held) {
        _ if !open || on => Fire::Done,
        (_, true) => Fire::Wait,
        (true, false) => Fire::Rearm,
        (false, false) => Fire::Shut,
    }
}

/// Arm bar `tree`'s idle timer, from now. A menu or client popup on the
/// bar's output (`autohideview::popup_on`), such as a revealed icon's,
/// holds it; its time starts again once that closes.
fn arm_idle(state: &mut State, tree: TreeId) {
    disarm_idle(state, tree);
    let mut held = false;
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(IDLE_MS)),
        move |_, _, state| {
            let bar = state.bar(tree);
            let open = bar.is_some_and(|b| b.tray_open);
            let on = state.bar_shared.tray_pointer == Some(tree);
            let now_held = bar.is_some_and(|b| crate::autohideview::popup_on(state, &b.output));
            match idle_fire(open, on, held, now_held) {
                Fire::Wait => {
                    held = true;
                    TimeoutAction::ToDuration(Duration::from_millis(HELD_POLL_MS))
                }
                Fire::Rearm => {
                    held = false;
                    TimeoutAction::ToDuration(Duration::from_millis(IDLE_MS))
                }
                fire @ (Fire::Done | Fire::Shut) => {
                    (state.bar_shared.reveal_idle).retain(|&(b, _)| b != tree);
                    if fire == Fire::Shut {
                        set(state, tree, false);
                    }
                    TimeoutAction::Drop
                }
            }
        },
    );
    match token {
        Ok(t) => state.bar_shared.reveal_idle.push((tree, t)),
        Err(e) => log::warn!("tray reveal idle timer: {}", e.error),
    }
}

/// Whether the bar keys going off a bar arm its idle timer: under
/// auto-hide, a run that is `open` with the pointer not `on` its area
/// would otherwise hold the bar shown until the pointer passes over the
/// area. With auto-hide off a run opened from the keyboard stays out.
fn keys_arm(auto_hide: bool, open: bool, on: bool) -> bool {
    auto_hide && open && !on
}

/// The bar keys left bar `tree` (Escape, a click elsewhere, an act that
/// hands the keyboard on): arm its idle timer if [`keys_arm`] says so.
pub fn keys_gone(state: &mut State, tree: TreeId) {
    let open = state.bar(tree).is_some_and(|b| b.tray_open);
    let on = state.bar_shared.tray_pointer == Some(tree);
    if keys_arm(state.config.taskbar.auto_hide, open, on) {
        arm_idle(state, tree);
    }
}

/// Whether `bar` must stay shown under auto-hide: its run is open or
/// still moving.
pub fn holds(bar: &Bar) -> bool {
    bar.tray_open || !bar.tray_reveal.settled(Instant::now())
}

fn arm_tick(state: &mut State) {
    if state.bar_shared.reveal_tick.is_some() {
        return;
    }
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(TICK_MS)),
        |_, _, state| tick(state),
    );
    match token {
        Ok(t) => state.bar_shared.reveal_tick = Some(t),
        Err(e) => log::warn!("tray reveal timer: {}", e.error),
    }
}

/// Whether the tick redraws a bar whose run is `slide` and was last drawn
/// open `drawn` px: while it moves, and once more at its end.
fn tick_redraws(slide: &Slide, drawn: i32, now: Instant) -> bool {
    !slide.settled(now) || drawn != slide.x(now)
}

/// One frame of every moving run: redraw its bar, and tick again while
/// any still moves. A run that settled since its bar's last draw gets one
/// more, at its end.
fn tick(state: &mut State) -> TimeoutAction {
    let now = Instant::now();
    let due: Vec<TreeId> = (state.bars.iter())
        .filter(|b| tick_redraws(&b.tray_reveal, b.tray_reveal_drawn, now))
        .map(|b| b.tree)
        .collect();
    for t in due {
        state.redraw_taskbar(t);
    }
    // The arrow moves with its run, away from a pointer that has not.
    let trees: Vec<TreeId> = state.bars.iter().map(|b| b.tree).collect();
    for t in trees {
        relight(state, t);
    }
    if state.bars.iter().any(|b| !b.tray_reveal.settled(now)) {
        TimeoutAction::ToDuration(Duration::from_millis(TICK_MS))
    } else {
        state.bar_shared.reveal_tick = None;
        // The last frame may have drawn the end already; auto-hide still
        // has to hear that the run settled.
        crate::autohideview::poke(state);
        TimeoutAction::Drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::sync::Arc;

    fn shape() -> TaskbarShape {
        TaskbarShape::CLASSIC
    }

    /// A run that opened to `x` over `SLIDE_MS` and has settled there.
    fn opened(x: i32, now: Instant) -> Slide {
        let then = now - Duration::from_millis(u64::from(SLIDE_MS));
        Slide::new(0, x, then, SLIDE_MS, Ease::Out)
    }

    #[test]
    fn a_bottom_bar_draws_the_whole_cells_inside_the_run() {
        let s = shape();
        let one = reprisekit::taskbar::reveal_width(&s, 1);
        assert_eq!(cells(false, &s, 3, 0), 0);
        assert_eq!(cells(false, &s, 3, one - 1), 0);
        assert_eq!(cells(false, &s, 3, one), 1);
        assert_eq!(cells(false, &s, 3, 10 * one), 3);
    }

    #[test]
    fn a_side_bar_draws_every_hidden_cell_once_the_run_opens() {
        let s = shape();
        assert_eq!(cells(true, &s, 3, 0), 0);
        assert_eq!(cells(true, &s, 3, 1), 3);
    }

    #[test]
    fn an_open_run_shuts_at_once_when_nothing_is_hidden() {
        let now = Instant::now();
        let mut open = true;
        let mut slide = Slide::new(0, 60, now, SLIDE_MS, Ease::Out);
        assert!(!follow(&mut open, &mut slide, 0, now));
        assert!(!open);
        assert_eq!(slide.x(now), 0);
        assert!(slide.settled(now));
    }

    #[test]
    fn an_open_run_slides_on_to_a_wider_run_as_items_come() {
        let now = Instant::now();
        let mut open = true;
        let mut slide = opened(40, now);
        assert!(follow(&mut open, &mut slide, 60, now));
        assert!(open);
        assert_eq!(slide.x(now), 40, "the run moves on from where it is drawn");
        assert_eq!(slide.offset(now), -20, "the run heads for 60 px");
        let later = now + Duration::from_millis(u64::from(SLIDE_MS));
        assert_eq!(slide.x(later), 60);
    }

    #[test]
    fn a_run_past_its_width_settles_there_at_once() {
        let now = Instant::now();
        let mut open = true;
        let mut slide = opened(60, now);
        assert!(!follow(&mut open, &mut slide, 40, now), "items went");
        assert!(open);
        assert_eq!(slide.x(now), 40);
        assert!(slide.settled(now));
        let mut slide = Slide::new(0, 600, now - Duration::from_millis(50), SLIDE_MS, Ease::Out);
        assert!(slide.x(now) > 100);
        assert!(
            !follow(&mut open, &mut slide, 100, now),
            "a run opening past the bar's room stops at it"
        );
        assert_eq!(slide.x(now), 100);
    }

    #[test]
    fn a_settled_run_at_its_width_stays_put() {
        let now = Instant::now();
        let mut open = true;
        let mut slide = opened(60, now);
        assert!(!follow(&mut open, &mut slide, 60, now));
        assert_eq!(slide.x(now), 60);
    }

    /// `idle_step` with the timers in `pending` armed and the runs in
    /// `open` open.
    fn step(was: Option<u8>, now: Option<u8>, pending: &[u8], open: &[u8]) -> IdleStep<u8> {
        idle_step(was, now, |t| pending.contains(&t), |t| open.contains(&t))
    }

    const KEEP: IdleStep<u8> = IdleStep {
        cancel: None,
        arm: None,
    };

    fn arm(t: u8) -> IdleStep<u8> {
        IdleStep {
            cancel: None,
            arm: Some(t),
        }
    }

    fn cancel(t: u8) -> IdleStep<u8> {
        IdleStep {
            cancel: Some(t),
            arm: None,
        }
    }

    #[test]
    fn leaving_an_open_runs_area_arms_its_idle_timer() {
        assert_eq!(step(Some(1), None, &[], &[1]), arm(1));
        assert_eq!(step(Some(1), Some(2), &[], &[1]), arm(1));
    }

    #[test]
    fn leaving_a_shut_runs_area_arms_nothing() {
        assert_eq!(step(Some(1), None, &[], &[]), KEEP);
    }

    #[test]
    fn coming_back_onto_the_area_stops_its_idle_timer() {
        assert_eq!(step(None, Some(1), &[1], &[1]), cancel(1));
    }

    #[test]
    fn another_bars_area_keeps_the_idle_timer() {
        assert_eq!(step(None, Some(2), &[1], &[1]), KEEP);
    }

    #[test]
    fn going_straight_between_two_open_runs_areas_leaves_the_first_ones_timer_be() {
        // On 1, then onto 2: 1's timer starts.
        assert_eq!(step(Some(1), Some(2), &[], &[1, 2]), arm(1));
        // Off 2: only 2's starts; 1's runs on from when the pointer left 1.
        assert_eq!(step(Some(2), None, &[1], &[1, 2]), arm(2));
        // Straight back onto 1: its timer stops and 2's starts.
        assert_eq!(
            step(Some(2), Some(1), &[1], &[1, 2]),
            IdleStep {
                cancel: Some(1),
                arm: Some(2),
            }
        );
    }

    #[test]
    fn moving_inside_the_area_or_off_every_area_changes_nothing() {
        assert_eq!(step(Some(1), Some(1), &[], &[1]), KEEP);
        assert_eq!(step(None, None, &[1], &[1]), KEEP);
    }

    #[test]
    fn the_tick_redraws_a_moving_run_and_a_settled_one_once_more() {
        let now = Instant::now();
        let moving = Slide::new(0, 60, now, SLIDE_MS, Ease::Out);
        assert!(tick_redraws(&moving, 0, now), "a moving run");
        let settled = opened(60, now);
        assert!(
            tick_redraws(&settled, 40, now),
            "settled since its last draw"
        );
        assert!(!tick_redraws(&settled, 60, now), "drawn at its end");
    }

    #[test]
    fn an_arrow_that_slid_from_under_the_pointer_goes_to_rest() {
        assert!(!lit_after_slide(false, true, false), "slid away: at rest");
        assert!(lit_after_slide(false, false, true), "slid under: lit");
        assert!(lit_after_slide(false, true, true), "still under: lit");
    }

    #[test]
    fn a_held_arrow_keeps_its_look_while_its_run_moves() {
        assert!(lit_after_slide(true, true, false));
        assert!(!lit_after_slide(true, false, true));
    }

    #[test]
    fn keys_leaving_an_open_run_under_auto_hide_arm_the_idle_timer() {
        assert!(keys_arm(true, true, false));
    }

    #[test]
    fn keys_leaving_arm_nothing_without_auto_hide_or_with_the_pointer_on_the_area() {
        assert!(
            !keys_arm(false, true, false),
            "auto-hide off: the run stays out"
        );
        assert!(
            !keys_arm(true, true, true),
            "the pointer on the area holds it"
        );
        assert!(
            !keys_arm(true, false, false),
            "a shut run has nothing to shut"
        );
    }

    #[test]
    fn a_popup_holds_the_idle_timer_and_its_closing_restarts_it() {
        assert_eq!(idle_fire(true, false, false, false), Fire::Shut);
        assert_eq!(idle_fire(true, false, false, true), Fire::Wait);
        assert_eq!(idle_fire(true, false, true, true), Fire::Wait);
        assert_eq!(idle_fire(true, false, true, false), Fire::Rearm);
    }

    #[test]
    fn an_idle_timer_stops_on_a_shut_run_or_with_the_pointer_back_on_it() {
        for held in [false, true] {
            assert_eq!(idle_fire(false, false, held, held), Fire::Done);
            assert_eq!(idle_fire(true, true, held, held), Fire::Done);
        }
    }

    #[test]
    fn a_shut_run_ignores_the_hidden_items() {
        let now = Instant::now();
        let mut open = false;
        let mut slide = Slide::at(0);
        assert!(!follow(&mut open, &mut slide, 60, now));
        assert!(!open);
        assert_eq!(slide.x(now), 0);
    }

    use crate::harness::{Harness, LEFT};
    use crate::tray::{TrayEvent, TrayItem};

    const KEY_ENTER: u32 = 28;

    fn passive(id: &str) -> TrayItem {
        TrayItem {
            id: id.into(),
            key: id.into(),
            service: id.into(),
            path: "/StatusNotifierItem".into(),
            title: id.into(),
            tooltip: String::new(),
            icon_name: None,
            icon_theme_path: None,
            pixmaps: Arc::from([]),
            named: Arc::from([]),
            menu_path: None,
            passive: true,
            pid: None,
        }
    }

    /// A Classic bar on one output whose tray hides two passive items behind the arrow, and its
    /// tree.
    fn hidden_two() -> (Harness, TreeId) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let (tx, wake) = crate::tray::headless(&mut h.state);
        for id in ["a", "b"] {
            tx.send(TrayEvent::Added(passive(id))).expect("send");
        }
        // SAFETY: `wake` is an open eventfd and the buffer a live 8-byte local.
        let n = unsafe { libc::write(wake.as_raw_fd(), (&1u64 as *const u64).cast(), 8) };
        assert_eq!(n, 8);
        h.dispatch();
        assert_eq!(hidden(&h.state), 2, "both items wait behind the arrow");
        let tree = h.state.bars[0].tree;
        (h, tree)
    }

    /// The middle of bar `tree`'s arrow where it is drawn now.
    fn arrow(h: &Harness, tree: TreeId) -> (f64, f64) {
        let bar = h.state.bar(tree).expect("bar");
        let y = 768.0 - 15.0;
        let xs: Vec<f64> = (0..1024)
            .map(f64::from)
            .filter(|&x| h.state.bar_hit(bar, x, y) == TaskbarHit::Chevron)
            .collect();
        let (first, last) = (
            xs.first().expect("the arrow"),
            xs.last().expect("the arrow"),
        );
        ((first + last) / 2.0, y)
    }

    fn open(h: &Harness, tree: TreeId) -> bool {
        h.state.bar(tree).is_some_and(|b| b.tray_open)
    }

    fn look(h: &Harness, tree: TreeId) -> (bool, bool) {
        let b = h.state.bar(tree).expect("bar");
        (b.chevron_hot, b.chevron_held)
    }

    #[test]
    fn a_click_on_the_arrow_opens_the_run_and_a_second_shuts_it() {
        let (mut h, tree) = hidden_two();
        let (x, y) = arrow(&h, tree);
        h.click(x, y);
        assert!(open(&h, tree));
        h.run_for(u64::from(SLIDE_MS) + 50);
        let (x2, y) = arrow(&h, tree);
        assert!(
            x2 < x,
            "the run opened between the arrow and the shown icons"
        );
        h.click(x2, y);
        assert!(!open(&h, tree));
    }

    #[test]
    fn a_held_arrow_looks_held_only_while_the_pointer_is_on_it() {
        let (mut h, tree) = hidden_two();
        let (x, y) = arrow(&h, tree);
        h.pointer_to(x, y);
        h.button(LEFT, true);
        assert_eq!(look(&h, tree), (true, true), "held under the pointer");
        h.pointer_to(x, y - 200.0);
        assert_eq!(look(&h, tree), (false, false), "dragged off: at rest");
        h.pointer_to(x, y);
        assert_eq!(look(&h, tree), (true, true), "back on: held again");
        h.pointer_to(x, y - 200.0);
        h.button(LEFT, false);
        assert!(!open(&h, tree), "a release off the arrow opens nothing");
    }

    #[test]
    fn a_second_button_ends_the_arrow_press_without_opening() {
        let (mut h, tree) = hidden_two();
        let (x, y) = arrow(&h, tree);
        h.pointer_to(x, y);
        h.button(LEFT, true);
        h.button(crate::input::BTN_RIGHT, true);
        assert!(!look(&h, tree).1, "no longer held");
        h.button(crate::input::BTN_RIGHT, false);
        h.button(LEFT, false);
        assert!(!open(&h, tree));
    }

    #[test]
    fn enter_on_the_arrow_of_an_auto_hidden_bar_toggles_its_run_and_keeps_the_keys() {
        let (mut h, tree) = hidden_two();
        let mut c = h.state.config.clone();
        c.taskbar.auto_hide = true;
        crate::configwatch::reload(&mut h.state, c);
        h.pointer_to(512.0, 300.0);
        h.run_for(crate::autohide::HIDE_DELAY.as_millis() as u64 + 400);
        let output = h.state.bars[0].output.clone();
        assert!(h.state.bar_hide.drawn(&output) > 0.0, "the bar hid");
        h.state.focus_bar_item(tree, TaskbarHit::Chevron);
        h.key(KEY_ENTER, true);
        h.key(KEY_ENTER, false);
        assert!(open(&h, tree));
        let keys = h.state.bar_shared.keys.expect("the bar keeps the keyboard");
        assert_eq!(keys.at, TaskbarHit::Chevron);
        h.key(KEY_ENTER, true);
        h.key(KEY_ENTER, false);
        assert!(!open(&h, tree));
    }

    #[test]
    fn leaving_the_open_runs_area_shuts_it_after_the_idle_time_and_coming_back_holds_it() {
        let (mut h, tree) = hidden_two();
        let (x, y) = arrow(&h, tree);
        h.click(x, y);
        assert!(open(&h, tree));
        let armed = |h: &Harness| {
            h.state
                .bar_shared
                .reveal_idle
                .iter()
                .any(|&(b, _)| b == tree)
        };
        h.pointer_to(x, y - 200.0);
        assert!(armed(&h), "off the area: the idle timer runs");
        h.pointer_to(x, y);
        assert!(!armed(&h), "back on the area: it stops");
        h.run_for(IDLE_MS + 100);
        assert!(open(&h, tree), "the pointer on the area holds the run");
        h.pointer_to(x, y - 200.0);
        h.run_for(IDLE_MS + 100);
        assert!(!open(&h, tree), "the idle time shut it");
        assert!(!armed(&h));
    }
}
