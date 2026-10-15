//! The bars' auto-hide on the scene (`[taskbar] auto_hide`): one
//! `autohide::AutoHide` per output's bar, fed what the compositor sees around
//! it (`signals`). `poke` steps every bar once the loop is idle (`refresh`); a
//! timer steps them when they change by themselves, and each output's frames
//! move its bar along a slide (`frame`). The Start key, Alt+Tab and a press on
//! a reveal line step a bar at once, so a menu opened by the same key docks
//! on the bar it brings back. A client layer surface mapping soon after a
//! press on the tray or the hidden icons (`tray_popup_holds`) is that item's
//! popup and holds the bar until it unmaps.
//!
//! `State::bar_drawn` is where a bar is drawn, partway along its slide;
//! `State::bar_placement` is where it is headed, which popups, the usable area
//! and window placement read. Hiding and showing never touch the usable area.
//! The lock screen leaves every bar where it is; unlock pokes them.

use std::time::{Duration, Instant};

use reprisescene::TreeId;
use smithay::output::Output;
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::autohide::{AutoHide, Rules, Signals};
use crate::compositor::State;
use crate::config::Config;
use crate::input::Interaction;
use crate::menuview::{Menu, MenuFrom};
use crate::raster::Rect;
use crate::taskbarview::Bar;

/// One output's bar: its machine, how hidden it was last put on the scene,
/// and whether it was headed out as of the last step.
struct Entry {
    output: Output,
    hide: AutoHide,
    drawn: f32,
    was_out: bool,
}

#[derive(Default)]
pub struct BarHide {
    entries: Vec<Entry>,
    timer: Option<(RegistrationToken, Instant)>,
    queued: bool,
    tray_press: Option<(Output, Instant)>,
    tray_popups: Vec<(Output, WlSurface)>,
}

impl BarHide {
    /// How hidden `output`'s bar is drawn: 0.0 shown (and with auto-hide
    /// off), 1.0 hidden.
    pub fn drawn(&self, output: &Output) -> f32 {
        self.entry(output).map_or(0.0, |e| e.drawn)
    }

    /// Whether `output`'s bar is hiding or hidden.
    pub fn headed_out(&self, output: &Output) -> bool {
        self.entry(output).is_some_and(|e| e.hide.headed_out())
    }

    fn entry(&self, output: &Output) -> Option<&Entry> {
        self.entries.iter().find(|e| e.output == *output)
    }

    fn entry_mut(&mut self, output: &Output) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.output == *output)
    }

    /// Whether a client layer surface that mapped on `output` at
    /// `mapped_at` with exclusive zone `zone` is the popup of the last tray
    /// press on that bar, mapped within `window` of it (`tray_popup_holds`).
    /// A press opens one popup: the surface it catches uses it up. A
    /// press whose window passed is dropped, on any bar, whether or not
    /// it opened anything.
    fn catch_tray_popup(
        &mut self,
        output: &Output,
        mapped_at: Instant,
        window: Duration,
        zone: i32,
    ) -> bool {
        let press = (self.tray_press.as_ref())
            .filter(|(o, _)| o == output)
            .map(|&(_, t)| t);
        let caught = tray_popup_holds(press, mapped_at, window, zone);
        let lapsed = (self.tray_press.as_ref())
            .is_some_and(|&(_, t)| mapped_at.saturating_duration_since(t) > window);
        if caught || lapsed {
            self.tray_press = None;
        }
        caught
    }
}

/// The triggers and the slide time `c` sets.
fn rules(c: &Config) -> Rules {
    Rules {
        edge: c.taskbar.reveal_on_edge,
        keys: c.taskbar.reveal_on_keys,
        attention: c.taskbar.reveal_on_attention,
        slide_ms: c.effects.taskbar_hide_ms,
    }
}

/// Whether the root of `menus` was opened from the bar on `out`, holding it
/// shown while open.
fn menu_holds(menus: &[Menu], out: &Output) -> bool {
    menus
        .first()
        .is_some_and(|m| m.output == *out && m.from == MenuFrom::Bar)
}

fn attention_on(state: &State, out: &Output) -> bool {
    if crate::notifyview::balloon_up(state) && state.first_output().as_ref() == Some(out) {
        return true;
    }
    state.windows.iter().any(|w| {
        let r = w.frame_rect();
        let centre = (f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        w.attention
            && w.mapped
            && state.on_current_desktop(w.id)
            && state.output_at(centre.0, centre.1).as_ref() == Some(out)
    })
}

pub fn popup_on(state: &State, out: &Output) -> bool {
    let menu = menu_holds(&state.menus, out);
    let tray_menu =
        (state.tray.as_ref()).is_some_and(|t| t.pending_menu.as_ref().is_some_and(|p| p.1 == *out));
    let tray_popup = (state.bar_hide.tray_popups.iter()).any(|(o, _)| o == out);
    menu || tray_menu || tray_popup
}

fn owned_open(state: &State, bar: &Bar) -> bool {
    let out = &bar.output;
    popup_on(state, out)
        || state.start_panel_bar() == Some(bar.tree)
        || crate::calendarview::open_on(state).as_ref() == Some(out)
        || crate::displaysview::open_on(state).as_ref() == Some(out)
        || crate::trayflyout::open_on(state).as_ref() == Some(out)
        || crate::trayreveal::holds(bar)
        || crate::notifycenter::open_on(state).as_ref() == Some(out)
        || state.task_preview.shown_on() == Some(out)
        || state.bar_shared.keys.is_some_and(|k| k.bar == bar.tree)
}

fn signals(state: &State, bar: &Bar) -> Signals {
    let out = &bar.output;
    let pos = state.pointer_location();
    let (x, y) = (pos.x.floor() as i32, pos.y.floor() as i32);
    let area = state.layout_box(Some(out));
    let drawn = state.bar_drawn(out);
    let s = drawn.shown;
    // A shown bar's orb reaches past it, and the pointer on it is on the
    // bar.
    let orb = (state.bar_hide.drawn(out) == 0.0)
        .then_some(bar.layout.start_above)
        .flatten()
        .map(|r| Rect::new(s.x + r.x, s.y + r.y, r.width, r.height));
    let dragging = matches!(
        &state.interaction,
        Interaction::TaskPress { bar: t, .. }
            | Interaction::QuickPress { bar: t, .. }
            | Interaction::ArrowPress { bar: t }
            | Interaction::Band { bar: t, .. }
            | Interaction::BarMove { bar: t, .. }
            | Interaction::BarWidth { bar: t, .. } if *t == bar.tree
    );
    Signals {
        on_line: drawn.reserved.contains(x, y),
        over_bar: s.intersect(&area).contains(x, y) || orb.is_some_and(|r| r.contains(x, y)),
        owned_open: owned_open(state, bar),
        dragging,
        switcher: (state.switcher.panel.as_ref())
            .is_some_and(|p| area.contains(p.area.x, p.area.y)),
        attention: attention_on(state, out),
        fullscreen: state.under_fullscreen(bar),
    }
}

/// Something a bar's auto-hide reads may have changed: step the bars once the
/// loop is idle.
pub fn poke(state: &mut State) {
    if !state.config.taskbar.auto_hide || state.bar_hide.queued {
        return;
    }
    state.bar_hide.queued = true;
    state.handle.insert_idle(|state| {
        state.bar_hide.queued = false;
        refresh(state);
    });
}

/// Step every bar's machine to now with what it sees, move the bars whose
/// slide changed and arm the timer for the next change. Nothing moves while
/// the session is locked.
pub fn refresh(state: &mut State) {
    if !state.config.taskbar.auto_hide || state.is_locked() {
        return;
    }
    let now = Instant::now();
    let r = rules(&state.config);
    let bars: Vec<(TreeId, Output)> = (state.bars.iter())
        .map(|b| (b.tree, b.output.clone()))
        .collect();
    (state.bar_hide.entries).retain(|e| bars.iter().any(|(_, o)| *o == e.output));
    let mut moved = Vec::new();
    let mut frames = Vec::new();
    let mut turned = false;
    for (tree, output) in bars {
        let Some(s) = state.bar(tree).map(|b| signals(state, b)) else {
            continue;
        };
        let bh = &mut state.bar_hide;
        if bh.entry(&output).is_none() {
            bh.entries.push(Entry {
                output: output.clone(),
                hide: AutoHide::new(now),
                drawn: 0.0,
                was_out: false,
            });
        }
        let Some(e) = bh.entry_mut(&output) else {
            continue;
        };
        e.hide.update(s, r, now);
        let drawn = e.hide.hidden(now);
        if drawn != e.drawn {
            moved.push(tree);
        }
        e.drawn = drawn;
        turned |= e.hide.headed_out() != e.was_out;
        e.was_out = e.hide.headed_out();
        if e.hide.moving(now) {
            frames.push(output);
        }
    }
    for t in moved {
        state.with_bar(t, |s, b| s.place_bar(b));
    }
    if turned {
        crate::notifyview::reposition(state);
        crate::access::touch(state);
    }
    for o in &frames {
        crate::backend::schedule(state, o);
    }
    arm(state, now);
}

fn arm(state: &mut State, now: Instant) {
    let r = rules(&state.config);
    let due = (state.bar_hide.entries.iter())
        .filter_map(|e| e.hide.wake(r, now))
        .min();
    if state.bar_hide.timer.as_ref().map(|&(_, at)| at) == due {
        return;
    }
    if let Some((t, _)) = state.bar_hide.timer.take() {
        state.handle.remove(t);
    }
    let Some(at) = due else {
        return;
    };
    let token = state
        .handle
        .insert_source(Timer::from_deadline(at), |_, _, state| {
            state.bar_hide.timer = None;
            refresh(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => state.bar_hide.timer = Some((t, at)),
        Err(e) => log::warn!("auto-hide timer: {}", e.error),
    }
}

/// `output` is about to render a frame: put its bar where its slide is now
/// and ask for the next frame while it still slides.
pub fn frame(state: &mut State, output: &Output) {
    if state.is_locked() {
        return;
    }
    let Some(e) = state.bar_hide.entry_mut(output) else {
        return;
    };
    let now = Instant::now();
    let drawn = e.hide.hidden(now);
    let more = e.hide.moving(now);
    let moved = drawn != e.drawn;
    e.drawn = drawn;
    if moved
        && let Some(t) = (state.bars.iter())
            .find(|b| b.output == *output)
            .map(|b| b.tree)
    {
        state.with_bar(t, |s, b| s.place_bar(b));
    }
    if more {
        crate::backend::schedule(state, output);
    }
}

/// The Start key, Ctrl+Esc or Alt+Tab acts on `output`: with
/// `reveal_on_keys` its bar heads out first, so the menu docks on the shown
/// bar. Over a fullscreen window only the Start key does.
pub fn keys(state: &mut State, output: &Output, start: bool) {
    if !state.config.taskbar.auto_hide || state.is_locked() {
        return;
    }
    let Some(fullscreen) = (state.bars.iter())
        .find(|b| b.output == *output)
        .map(|b| state.under_fullscreen(b))
    else {
        return;
    };
    let r = rules(&state.config);
    if let Some(e) = state.bar_hide.entry_mut(output) {
        e.hide.keyed(r, start, fullscreen, Instant::now());
    }
    refresh(state);
}

/// A pointer button went down on bar `tree`. A bar not all the way out takes
/// it as a press on its reveal line (`reveal_on_edge`). Returns whether the
/// bar took the press.
pub fn line_pressed(state: &mut State, tree: TreeId) -> bool {
    if state.is_locked() {
        return false;
    }
    let Some(bar) = state.bar(tree) else {
        return false;
    };
    if state.bar_hide.drawn(&bar.output) == 0.0 {
        return false;
    }
    let (output, fullscreen) = (bar.output.clone(), state.under_fullscreen(bar));
    let r = rules(&state.config);
    if let Some(e) = state.bar_hide.entry_mut(&output) {
        e.hide.pressed(r, fullscreen, Instant::now());
    }
    refresh(state);
    true
}

/// How soon after a tray press a client layer surface must map to count as
/// that item's popup: `c`'s double click interval and 250 ms.
fn tray_popup_window(c: &Config) -> Duration {
    Duration::from_millis(c.theme.metrics.double_click_ms.max(0) as u64 + 250)
}

/// Whether a layer surface that mapped at `mapped_at` with exclusive zone
/// `zone` is the popup of the last tray press (`last_tray_press`): it mapped
/// within `window` of it and reserves no space. A surface that reserves space
/// (a panel, an on-screen keyboard) would hold the bar shown for good.
pub fn tray_popup_holds(
    last_tray_press: Option<Instant>,
    mapped_at: Instant,
    window: Duration,
    zone: i32,
) -> bool {
    zone <= 0
        && last_tray_press
            .and_then(|t| mapped_at.checked_duration_since(t))
            .is_some_and(|d| d <= window)
}

pub fn tray_pressed(state: &mut State, output: &Output) {
    state.bar_hide.tray_press = Some((output.clone(), Instant::now()));
}

/// Layer surface `surface` mapped on `output`: right after a tray press and
/// reserving no space, it is the item's popup and holds the bar and its open
/// tray run until it unmaps or goes (`popup_gone`). A press holds one surface
/// at most.
pub fn popup_mapped(state: &mut State, output: &Output, surface: &WlSurface, zone: i32) {
    let window = tray_popup_window(&state.config);
    if state
        .bar_hide
        .catch_tray_popup(output, Instant::now(), window, zone)
    {
        (state.bar_hide.tray_popups).push((output.clone(), surface.clone()));
        poke(state);
    }
}

/// Layer surface `surface` unmapped or went: it holds no bar now. A destroyed
/// surface redraws no bar, so this steps the bars itself.
pub fn popup_gone(state: &mut State, surface: &WlSurface) {
    let popups = &mut state.bar_hide.tray_popups;
    let before = popups.len();
    popups.retain(|(_, s)| s != surface);
    if popups.len() != before {
        poke(state);
    }
}

/// `[taskbar] auto_hide` was turned on or off; runs on reload. Every machine,
/// the timer and the tray holds go, and every bar is placed whole at once so
/// none is left hidden or taking presses only as a reveal line.
pub fn config_changed(state: &mut State) {
    if let Some((t, _)) = state.bar_hide.timer.take() {
        state.handle.remove(t);
    }
    state.bar_hide.entries.clear();
    state.bar_hide.tray_press = None;
    state.bar_hide.tray_popups.clear();
    let trees: Vec<TreeId> = state.bars.iter().map(|b| b.tree).collect();
    for t in trees {
        state.with_bar(t, |s, b| s.place_bar(b));
    }
    poke(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menuview::MenuKind;

    fn test_output(name: &str) -> Output {
        Output::new(
            name.into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Reprise".into(),
                model: "Test".into(),
                serial_number: "0".into(),
            },
        )
    }

    #[test]
    fn a_menu_holds_the_bar_by_where_it_opened_not_by_its_rows() {
        let mut scene = crate::compositor::DeskScene::new();
        let (a, b) = (test_output("A-1"), test_output("B-1"));
        let program = || MenuKind::StartPanel { rows: Vec::new() };
        let start = Menu::for_test(&mut scene, a.clone(), program(), MenuFrom::Bar);
        let launcher = Menu::for_test(&mut scene, a.clone(), program(), MenuFrom::Elsewhere);
        assert!(menu_holds(&[start], &a), "a Start panel row's menu");
        assert!(
            !menu_holds(&[launcher], &a),
            "the same menu from the launcher"
        );
        let start = Menu::for_test(&mut scene, a.clone(), program(), MenuFrom::Bar);
        assert!(!menu_holds(&[start], &b), "another output's bar");
        assert!(!menu_holds(&[], &a), "no menu");
    }

    #[test]
    fn the_rules_are_the_configs_keys() {
        let mut c = Config::default();
        c.taskbar.reveal_on_keys = false;
        c.effects.taskbar_hide_ms = 0;
        assert_eq!(
            rules(&c),
            Rules {
                edge: true,
                keys: false,
                attention: true,
                slide_ms: 0,
            }
        );
    }

    #[test]
    fn a_layer_surface_mapping_soon_after_a_tray_press_is_its_popup() {
        let mut c = Config::default();
        c.theme.metrics.double_click_ms = 700;
        let window = tray_popup_window(&c);
        assert_eq!(
            window,
            Duration::from_millis(950),
            "the double click and 250 ms"
        );
        let pressed = Instant::now();
        let after = |ms| pressed + Duration::from_millis(ms);
        assert!(
            tray_popup_holds(Some(pressed), pressed, window, 0),
            "at the press"
        );
        assert!(
            tray_popup_holds(Some(pressed), after(950), window, 0),
            "950 ms after it"
        );
        assert!(
            !tray_popup_holds(Some(pressed), after(951), window, 0),
            "later, it is not the press's popup"
        );
        assert!(
            !tray_popup_holds(None, after(0), window, 0),
            "no press, no popup"
        );
        assert!(
            !tray_popup_holds(Some(after(100)), pressed, window, 0),
            "mapped before the press"
        );
        let default = tray_popup_window(&Config::default());
        assert!(
            tray_popup_holds(Some(pressed), after(515), default, 0),
            "a popup that waits out the default double click"
        );
        assert!(
            !tray_popup_holds(Some(pressed), after(100), window, 30),
            "a surface that reserves space is no popup"
        );
        assert!(
            tray_popup_holds(Some(pressed), after(100), window, -1),
            "one that only asks not to be moved is"
        );
    }

    #[test]
    fn one_tray_press_holds_one_popup() {
        let (a, b) = (test_output("A-1"), test_output("B-1"));
        let pressed = Instant::now();
        let after = pressed + Duration::from_millis(100);
        let mut hide = BarHide {
            tray_press: Some((a.clone(), pressed)),
            ..BarHide::default()
        };
        let window = Duration::from_millis(750);
        assert!(
            !hide.catch_tray_popup(&b, after, window, 0),
            "another bar's surface is not the press's popup"
        );
        assert!(
            !hide.catch_tray_popup(&a, after, window, 30),
            "a surface that reserves space is not the press's popup"
        );
        assert!(
            hide.catch_tray_popup(&a, after, window, 0),
            "the press's popup"
        );
        assert!(
            !hide.catch_tray_popup(&a, after, window, 0),
            "a second surface after the popup is not held"
        );
    }

    #[test]
    fn a_tray_press_that_opened_nothing_is_dropped_once_its_window_passes() {
        let out = test_output("A-1");
        let pressed = Instant::now();
        let window = Duration::from_millis(750);
        let mut hide = BarHide {
            tray_press: Some((out.clone(), pressed)),
            ..BarHide::default()
        };
        assert!(!hide.catch_tray_popup(&out, pressed + Duration::from_millis(100), window, 30));
        assert!(
            hide.tray_press.is_some(),
            "a panel inside the window leaves the press for its popup"
        );
        assert!(!hide.catch_tray_popup(&out, pressed + Duration::from_millis(751), window, 0));
        assert!(
            hide.tray_press.is_none(),
            "past its window the press is gone"
        );
    }
}
