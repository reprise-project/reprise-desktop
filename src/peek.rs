//! Peek: resting the pointer on a live preview in a task button's panel (or
//! on the Show desktop strip), or selecting an Alt+Tab entry, for `PEEK_MS` turns
//! off the `content` tree of every other window (every window, for the strip or
//! the Desktop entry). Under Glass with transparency on, each becomes a sheet of
//! the frame's glass with title and caption buttons gone (`State::is_sheet`);
//! otherwise its bare frame stays. Popups and subsurfaces go with `content`.
//! Leaving puts back the enabled states the peek found.
//!
//! The target draws above every other window while the peek runs (`hoist`,
//! render only); focus and stacking keep the real order. A minimized target is
//! shown in place for the peek and goes again when it moves or ends. The Alt+Tab
//! switcher arms a peek with [`arm`] (which checks `[switcher] peek`) and ends
//! it with [`leave`]. A peek at a window keeps its X11 menus and tooltips
//! (override-redirect windows of the same client) on screen (`goes_with`).

use std::time::Duration;

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use reprisescene::NodeId;

use crate::{compositor::State, config::Config, window::WindowId};

const PEEK_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekTarget {
    Desktop,
    Window(WindowId),
}

impl PeekTarget {
    /// A peek at this target turns `w`'s content off. `goes_with(t)`: `w`
    /// belongs with window `t` (one of `t`'s menus or tooltips), so a peek
    /// at `t` keeps it.
    fn hides(self, w: WindowId, goes_with: impl Fn(WindowId) -> bool) -> bool {
        match self {
            PeekTarget::Desktop => true,
            PeekTarget::Window(t) => w != t && !goes_with(t),
        }
    }
}

/// The peek waiting on its timer or running, with every content tree it turned
/// off (and its prior enabled state) and the minimized target it turned on.
#[derive(Default)]
pub struct Peek {
    timer: Option<RegistrationToken>,
    pending: Option<PeekTarget>,
    running: Option<PeekTarget>,
    hidden: Vec<(WindowId, bool)>,
    shown: Option<WindowId>,
}

#[derive(Debug, PartialEq, Eq)]
enum Hovered {
    Same,
    Arm,
    /// A running peek moves to the new target at once: undo this, then
    /// hide for the new target.
    Switch(Undo),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Undo {
    disarm: bool,
    restore: Vec<(WindowId, bool)>,
    /// The minimized target the peek showed, to turn off again if still minimized.
    shown: Option<WindowId>,
}

impl Peek {
    pub fn active(&self) -> bool {
        self.running.is_some()
    }

    /// The running peek turned `w`'s content off.
    pub fn hides(&self, w: WindowId) -> bool {
        self.running.is_some() && self.hidden.iter().any(|&(h, _)| h == w)
    }

    fn hovered(&mut self, t: PeekTarget) -> Hovered {
        match self.running {
            Some(r) if r == t => Hovered::Same,
            Some(_) => {
                self.running = Some(t);
                Hovered::Switch(Undo {
                    disarm: false,
                    restore: std::mem::take(&mut self.hidden),
                    shown: self.shown.take(),
                })
            }
            None if self.pending == Some(t) => Hovered::Same,
            None => {
                self.pending = Some(t);
                Hovered::Arm
            }
        }
    }

    fn fired(&mut self) -> Option<PeekTarget> {
        let t = self.pending.take()?;
        self.running = Some(t);
        Some(t)
    }

    /// Whether `w`, whose content is `enabled` now, is hidden by the
    /// running peek (`goes_with` as in `PeekTarget::hides`). The first
    /// answer for a window records `enabled` so the end of the peek can put
    /// it back; later ones keep that record.
    fn hide(&mut self, w: WindowId, enabled: bool, goes_with: impl Fn(WindowId) -> bool) -> bool {
        if !self.running.is_some_and(|t| t.hides(w, goes_with)) {
            return false;
        }
        if !self.hidden.iter().any(|&(h, _)| h == w) {
            self.hidden.push((w, enabled));
        }
        true
    }

    fn left(&mut self) -> Undo {
        self.running = None;
        Undo {
            disarm: self.pending.take().is_some(),
            restore: std::mem::take(&mut self.hidden),
            shown: self.shown.take(),
        }
    }

    /// `w` is going: a waiting peek at it is cancelled, a running one ends,
    /// and otherwise its content alone goes back and leaves the record.
    fn forgot(&mut self, w: WindowId) -> Undo {
        let target = PeekTarget::Window(w);
        if self.pending == Some(target) || self.running == Some(target) {
            return self.left();
        }
        match self.hidden.iter().position(|&(h, _)| h == w) {
            Some(i) => Undo {
                disarm: false,
                restore: vec![self.hidden.remove(i)],
                shown: None,
            },
            None => Undo::default(),
        }
    }
}

/// The pointer rests on `target`: start or restart the timer, or move a running
/// peek to it at once. Only ends any peek while `[taskbar] peek` is off.
pub fn hover(state: &mut State, target: PeekTarget) {
    if !peek_enabled(&state.config) {
        leave(state);
        return;
    }
    arm(state, target);
}

/// `hover` without the taskbar's key: the caller decides whether peek is on.
pub fn arm(state: &mut State, target: PeekTarget) {
    match state.peek.hovered(target) {
        Hovered::Same => {}
        Hovered::Arm => {
            disarm(state);
            let token = state.handle.insert_source(
                Timer::from_duration(Duration::from_millis(PEEK_MS)),
                |_, _, state| {
                    state.peek.timer = None;
                    if state.peek.fired().is_some() {
                        hide_all(state);
                    }
                    TimeoutAction::Drop
                },
            );
            match token {
                Ok(t) => state.peek.timer = Some(t),
                Err(e) => log::warn!("peek timer: {}", e.error),
            }
        }
        Hovered::Switch(undo) => {
            apply(state, undo);
            hide_all(state);
        }
    }
}

pub fn leave(state: &mut State) {
    let undo = state.peek.left();
    apply(state, undo);
}

/// `w` mapped: a running peek that hides it turns its content off too.
pub fn window_mapped(state: &mut State, w: WindowId) {
    hide_one(state, w);
    hoist(state);
}

/// `w` is going away: drop it from the record with its content back, and end or
/// cancel a peek at it.
pub fn forget_window(state: &mut State, w: WindowId) {
    let undo = state.peek.forgot(w);
    apply(state, undo);
}

fn disarm(state: &mut State) {
    if let Some(t) = state.peek.timer.take() {
        state.handle.remove(t);
    }
}

fn hide_all(state: &mut State) {
    let all: Vec<WindowId> = state.windows.iter().map(|w| w.id).collect();
    for w in all {
        hide_one(state, w);
    }
    show_target(state);
    hoist(state);
}

/// Draw a running peek's target, and the menus and tooltips that go with it,
/// above every other window (`Scene::set_hoist` into the top of `layers.on_top`).
/// A fullscreen target stays in its own layer.
fn hoist(state: &mut State) {
    let mut nodes = Vec::new();
    if let Some(PeekTarget::Window(t)) = state.peek.running {
        let wanted: Vec<_> = (state.windows.iter())
            .filter(|x| x.id == t || goes_with(state, x.id, t))
            .map(|x| NodeId::from(x.tree))
            .collect();
        for layer in [state.layers.windows, state.layers.on_top] {
            nodes.extend((state.scene.children(layer).iter()).filter(|n| wanted.contains(n)));
        }
    }
    let on_top = state.layers.on_top;
    state.scene.set_hoist(on_top, nodes);
}

fn show_target(state: &mut State) {
    let Some(PeekTarget::Window(t)) = state.peek.running else {
        return;
    };
    let Some(tree) = state.win(t).filter(|x| x.mapped).map(|x| x.tree) else {
        return;
    };
    if state.on_current_desktop(t) && state.minimized_along_chain(t) {
        state.scene.set_enabled(tree, true);
        state.peek.shown = Some(t);
        state.redraw_all();
    }
}

fn stays_hidden(state: &State, w: WindowId) -> bool {
    state.win(w).is_some_and(|x| x.mapped)
        && !state.minimized_along_chain(w)
        && (state.peek.running).is_some_and(|t| t.hides(w, |t| goes_with(state, w, t)))
}

fn hide_one(state: &mut State, w: WindowId) {
    let Some(content) = state.win(w).filter(|x| x.mapped).map(|x| x.content) else {
        return;
    };
    if state.minimized_along_chain(w) {
        return;
    }
    let enabled = state.scene.enabled(content);
    // `hide` asks only about the running peek's own window.
    let goes = match state.peek.running {
        Some(PeekTarget::Window(t)) => goes_with(state, w, t),
        _ => false,
    };
    if state.peek.hide(w, enabled, |_| goes) {
        state.scene.set_enabled(content, false);
        // A sheet the switch from another target left (`put_back`) is
        // drawn already.
        if state.is_sheet(w) && !state.win(w).is_some_and(|x| x.sheeted) {
            state.redraw_frame(w);
        }
        state.redraw_all();
    }
}

/// `w` is an X11 menu or tooltip (override-redirect) of `t`'s client: set
/// transient for `t`, or from the same process as the X11 window `t`.
fn goes_with(state: &State, w: WindowId, t: WindowId) -> bool {
    if !state.win(w).is_some_and(|x| x.override_redirect) {
        return false;
    }
    if state.parent_window(w) == Some(t) {
        return true;
    }
    state.win(t).is_some_and(|x| x.x11().is_some())
        && state
            .client_pid(w)
            .is_some_and(|p| state.client_pid(t) == Some(p))
}

/// Stop the timer if asked, put the recorded content states back, and turn off
/// the shown target again if still minimized (restored during the peek, it stays).
fn apply(state: &mut State, undo: Undo) {
    if undo.disarm {
        disarm(state);
    }
    put_back(state, &undo.restore);
    if let Some(t) = undo.shown
        && state.minimized_along_chain(t)
        && let Some(tree) = state.win(t).map(|x| x.tree)
    {
        state.scene.set_enabled(tree, false);
        state.redraw_all();
    }
    hoist(state);
}

/// Set each window's content back to its recorded enabled state, and a
/// sheet's frame back unless the running peek (switched to a new target)
/// keeps it a sheet: `hide_all` turns its content off again next.
fn put_back(state: &mut State, restore: &[(WindowId, bool)]) {
    for &(w, enabled) in restore {
        if let Some(content) = state.win(w).map(|x| x.content) {
            state.scene.set_enabled(content, enabled);
        }
        if state.win(w).is_some_and(|x| x.sheeted) && !stays_hidden(state, w) {
            state.redraw_frame(w);
        }
    }
    if !restore.is_empty() {
        state.redraw_all();
    }
}

pub fn peek_enabled(config: &Config) -> bool {
    config.taskbar.peek
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripPeek {
    Hover(PeekTarget),
    Leave,
    Nothing,
}

pub fn strip_peek(config: &Config, on: bool) -> StripPeek {
    match (peek_enabled(config), on) {
        // Leaving always ends a peek, even with peek turned off in the
        // meantime: `Peek::left` is a no-op with nothing running.
        (_, false) => StripPeek::Leave,
        (false, true) => StripPeek::Nothing,
        (true, true) => StripPeek::Hover(PeekTarget::Desktop),
    }
}

pub fn show_desktop_hover(state: &mut State, on: bool) {
    match strip_peek(&state.config, on) {
        StripPeek::Hover(t) => hover(state, t),
        StripPeek::Leave => leave(state),
        StripPeek::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `n`th of ten window ids, the same for a whole test.
    fn w(n: usize) -> WindowId {
        thread_local! {
            static IDS: Vec<WindowId> = (0..10).map(|_| WindowId::next()).collect();
        }
        IDS.with(|ids| ids[n])
    }

    /// A window that belongs with no other.
    fn alone(_: WindowId) -> bool {
        false
    }

    #[test]
    fn switcher_peek_is_independent_of_the_taskbar_key() {
        // [taskbar] peek off must not stop Alt+Tab's peek: the gate is
        // the caller's. peek_enabled reads only the taskbar key.
        let c = Config::parse("[taskbar]\npeek = false\n[switcher]\npeek = true\n").unwrap();
        assert!(!peek_enabled(&c));
        assert!(c.switcher.peek);
    }

    #[test]
    fn peek_follows_its_taskbar_key_in_every_look() {
        let on = |text: &str| peek_enabled(&Config::parse(text).unwrap());
        assert!(
            on("[appearance]\ninterface = \"glass\"\n"),
            "on under Glass"
        );
        assert!(!on(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\npeek = false\n"
        ));
        assert!(!on(""), "off under Classic");
        assert!(on("[taskbar]\npeek = true\n"), "turned on under Classic");
    }

    #[test]
    fn the_show_desktop_strip_peeks_at_the_desktop_while_peek_is_on() {
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        assert_eq!(
            strip_peek(&glass, true),
            StripPeek::Hover(PeekTarget::Desktop)
        );
        assert_eq!(strip_peek(&glass, false), StripPeek::Leave);
        let off = Config::parse("[appearance]\ninterface = \"glass\"\n[taskbar]\npeek = false\n")
            .unwrap();
        assert_eq!(
            strip_peek(&off, true),
            StripPeek::Nothing,
            "no peek on hover"
        );
        assert_eq!(strip_peek(&off, false), StripPeek::Leave);
    }

    #[test]
    fn strip_peek_leaves_on_the_pointer_going_no_matter_the_peek_key() {
        let glass = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        let off = Config::parse("[appearance]\ninterface = \"glass\"\n[taskbar]\npeek = false\n")
            .unwrap();
        for (config, label) in [(&glass, "peek on"), (&off, "peek off")] {
            assert_eq!(strip_peek(config, false), StripPeek::Leave, "{label}");
        }
    }

    #[test]
    fn a_hover_arms_the_timer_once_and_a_new_target_rearms_it() {
        let mut p = Peek::default();
        assert_eq!(p.hovered(PeekTarget::Window(w(1))), Hovered::Arm);
        assert_eq!(p.hovered(PeekTarget::Window(w(1))), Hovered::Same);
        assert_eq!(p.hovered(PeekTarget::Window(w(2))), Hovered::Arm);
        assert!(!p.active(), "nothing runs before the timer fires");
        assert_eq!(p.fired(), Some(PeekTarget::Window(w(2))));
        assert!(p.active());
        assert_eq!(p.fired(), None, "the timer starts a target once");
    }

    #[test]
    fn a_leave_before_the_timer_cancels_the_peek() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Desktop);
        assert_eq!(
            p.left(),
            Undo {
                disarm: true,
                restore: vec![],
                shown: None
            }
        );
        assert_eq!(p.fired(), None);
        assert!(!p.active());
        assert_eq!(p.left(), Undo::default(), "a second leave does nothing");
    }

    #[test]
    fn a_window_peek_hides_every_window_but_its_own_and_desktop_hides_all() {
        let t = PeekTarget::Window(w(1));
        assert!(!t.hides(w(1), alone));
        assert!(t.hides(w(2), alone));
        assert!(PeekTarget::Desktop.hides(w(1), alone));
    }

    #[test]
    fn a_window_peek_keeps_the_menus_that_go_with_the_window() {
        let t = PeekTarget::Window(w(1));
        let menu_of_1 = |o| o == w(1);
        assert!(!t.hides(w(5), menu_of_1));
        assert!(
            PeekTarget::Window(w(2)).hides(w(5), menu_of_1),
            "another window's peek hides the menu"
        );
        assert!(PeekTarget::Desktop.hides(w(5), menu_of_1));
    }

    #[test]
    fn nothing_hides_while_no_peek_runs() {
        let mut p = Peek::default();
        assert!(!p.hide(w(2), true, alone));
        p.hovered(PeekTarget::Desktop);
        assert!(!p.hide(w(2), true, alone), "a waiting peek hides nothing");
        assert_eq!(p.left().restore, vec![]);
    }

    #[test]
    fn a_leave_restores_the_states_the_peek_found() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Window(w(1)));
        p.fired();
        assert!(
            !p.hide(w(1), true, alone),
            "the peeked window keeps its content"
        );
        assert!(p.hide(w(2), true, alone));
        assert!(p.hide(w(3), false, alone));
        // A repeated hide of the same window keeps its first record (a remap
        // passes through `forgot` first, so it records afresh).
        assert!(p.hide(w(2), false, alone));
        assert_eq!(
            p.left(),
            Undo {
                disarm: false,
                restore: vec![(w(2), true), (w(3), false)],
                shown: None
            }
        );
        assert!(!p.active());
    }

    #[test]
    fn a_window_is_a_sheet_from_its_hide_until_the_peek_moves_off_it_or_ends() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Window(w(1)));
        p.fired();
        assert!(!p.hides(w(2)), "not before its content goes");
        p.hide(w(2), true, alone);
        assert!(p.hides(w(2)));
        assert!(!p.hides(w(1)), "the target never is");
        p.hovered(PeekTarget::Window(w(2)));
        assert!(!p.hides(w(2)), "the new target is itself again");
        p.hide(w(1), true, alone);
        assert!(p.hides(w(1)), "the old target is one");
        p.left();
        assert!(!p.hides(w(1)), "none once the peek ends");
    }

    #[test]
    fn a_new_target_switches_a_running_peek_at_once() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Window(w(1)));
        p.fired();
        p.hide(w(2), true, alone);
        assert_eq!(p.hovered(PeekTarget::Window(w(1))), Hovered::Same);
        assert_eq!(
            p.hovered(PeekTarget::Window(w(2))),
            Hovered::Switch(Undo {
                disarm: false,
                restore: vec![(w(2), true)],
                shown: None
            })
        );
        assert!(p.active());
        assert!(
            p.hide(w(1), true, alone),
            "the old target hides under the new one"
        );
        assert!(!p.hide(w(2), true, alone));
        assert_eq!(p.left().restore, vec![(w(1), true)]);
    }

    #[test]
    fn a_shown_minimized_target_goes_back_when_the_peek_moves_or_ends() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Window(w(1)));
        p.fired();
        p.shown = Some(w(1));
        let Hovered::Switch(undo) = p.hovered(PeekTarget::Window(w(2))) else {
            panic!("a running peek switches");
        };
        assert_eq!(undo.shown, Some(w(1)), "moving on hides the old target");
        p.shown = Some(w(2));
        assert_eq!(p.left().shown, Some(w(2)), "a leave hides the target");
        assert_eq!(p.left().shown, None, "a second leave hides nothing");
    }

    #[test]
    fn a_closed_window_leaves_the_record_with_its_state_back() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Desktop);
        p.fired();
        p.hide(w(2), true, alone);
        p.hide(w(3), true, alone);
        assert_eq!(
            p.forgot(w(2)),
            Undo {
                disarm: false,
                restore: vec![(w(2), true)],
                shown: None
            }
        );
        assert_eq!(p.forgot(w(9)), Undo::default());
        assert!(p.active());
        assert_eq!(p.left().restore, vec![(w(3), true)]);
    }

    #[test]
    fn losing_the_peeked_window_ends_or_cancels_its_peek() {
        let mut p = Peek::default();
        p.hovered(PeekTarget::Window(w(1)));
        assert_eq!(
            p.forgot(w(1)),
            Undo {
                disarm: true,
                restore: vec![],
                shown: None
            }
        );
        assert_eq!(p.fired(), None);

        p.hovered(PeekTarget::Window(w(1)));
        p.fired();
        p.hide(w(2), true, alone);
        assert_eq!(p.forgot(w(1)).restore, vec![(w(2), true)]);
        assert!(!p.active());
    }
}
