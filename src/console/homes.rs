//! Lock mode's window moves: as it begins, each window keeps its place on the computer's own
//! outputs as its [`Home`] and goes to a Remote output; as it ends, each goes back.

use reprisekit::classic::frame::FrameGeometry;
use smithay::output::Output;

use crate::compositor::State;
use crate::raster::Rect;
use crate::virtualout::{is_remote, remote_outputs};
use crate::window::{Window, WindowId, keep_inside};

/// A rect as fractions of an area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl RelRect {
    pub fn of(r: Rect, area: Rect) -> RelRect {
        let (aw, ah) = (f64::from(area.width.max(1)), f64::from(area.height.max(1)));
        RelRect {
            x: f64::from(r.x - area.x) / aw,
            y: f64::from(r.y - area.y) / ah,
            w: f64::from(r.width) / aw,
            h: f64::from(r.height) / ah,
        }
    }

    pub fn at(self, area: Rect) -> Rect {
        let (aw, ah) = (f64::from(area.width), f64::from(area.height));
        Rect::new(
            area.x + (self.x * aw).round() as i32,
            area.y + (self.y * ah).round() as i32,
            (self.w * aw).round() as i32,
            (self.h * ah).round() as i32,
        )
    }
}

/// How a window fills its place. A window takes its home's frame back only in the same fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Normal,
    /// In a zone outside any snap group.
    Snapped,
    /// Its snap group sets its zone.
    Grouped,
    Maximized,
    Fullscreen,
}

/// Where a window was on the computer's own outputs as lock mode began.
#[derive(Clone, Debug, PartialEq)]
pub struct Home {
    /// The monitor id (`outputlayouts::monitor_ids`) of the output under its frame's centre.
    pub monitor: String,
    /// Its frame, as fractions of that output's work area; so are the other rects.
    pub rel: RelRect,
    pub fit: Fit,
    pub saved_rel: Option<RelRect>,
    pub saved_fullscreen_rel: Option<RelRect>,
    /// The frame it unsnaps to.
    pub snapped_rel: Option<RelRect>,
}

/// Lock mode began: each window keeps its home and goes to the same relative place on the
/// Remote output paired with its own ([`console_outputs`] order), else on the remote primary.
pub fn send_all(state: &mut State) {
    let remotes = remote_outputs(state);
    let Some(primary) = remotes.first().cloned() else {
        return;
    };
    let outs = console_outputs(state);
    let ids = crate::outputlayouts::monitor_ids(&outs.iter().collect::<Vec<_>>());
    for w in movable(state) {
        let Some(i) = under(state, w, &outs) else {
            continue;
        };
        let from = state.usable_area(Some(&outs[i]));
        let home = home_of(state, w, &ids[i], from);
        if let Some(win) = state.win_mut(w) {
            win.home = home;
        }
        let to = remotes.get(i).unwrap_or(&primary).clone();
        carry(state, w, from, &to, None);
    }
    state.update_layout();
}

/// Lock mode ends, before the Remote outputs go: each window goes home, else to the console
/// primary at its relative place; a snap group follows its first homed member. The caller lays out.
pub fn bring_back(state: &mut State) {
    let remotes = remote_outputs(state);
    let outs = console_outputs(state);
    let ids = crate::outputlayouts::monitor_ids(&outs.iter().collect::<Vec<_>>());
    let mut ws = movable(state);
    ws.sort_by_key(|&w| state.win(w).is_none_or(|x| x.home.is_none()));
    for w in ws {
        let home = state.win_mut(w).and_then(|x| x.home.take());
        let Some(r) = under(state, w, &remotes) else {
            continue;
        };
        let from = state.usable_area(Some(&remotes[r]));
        let at_home = (home.as_ref()).and_then(|h| ids.iter().position(|id| *id == h.monitor));
        let Some(to) = outs.get(at_home.unwrap_or(0)).cloned() else {
            continue;
        };
        carry(
            state,
            w,
            from,
            &to,
            home.as_ref().filter(|_| at_home.is_some()),
        );
    }
    for win in &mut state.windows {
        win.home = None;
    }
}

/// The Remote outputs went from the boxes `before` to `after`: each window on one keeps its
/// relative place there. A grouped window is left to its group's refit.
pub fn refit_remote(state: &mut State, before: &[(Output, Rect)], after: &[(Output, Rect)]) {
    let moves: Vec<(WindowId, Rect, Rect)> = (movable(state).into_iter())
        .filter_map(|w| {
            let (x, y) = anchor(state.win(w)?);
            let (o, old) = before.iter().find(|(_, b)| b.contains(x, y))?;
            let (_, new) = after.iter().find(|(n, _)| n == o)?;
            (old != new).then_some((w, *old, *new))
        })
        .collect();
    for (w, old, new) in moves {
        let Some(fit) = fit_of(state, w) else {
            continue;
        };
        let Some(win) = state.win_mut(w) else {
            continue;
        };
        for slot in [&mut win.saved, &mut win.saved_fullscreen, &mut win.snapped] {
            *slot = slot.map(|r| carried(r, old, new));
        }
        let frame = win.frame_rect();
        match fit {
            Fit::Maximized | Fit::Fullscreen => centre(state, w, new),
            Fit::Grouped => {}
            Fit::Snapped => state.snap_commit(w, RelRect::of(frame, old).at(new)),
            Fit::Normal => {
                let r = carried(frame, old, new);
                state.move_to(w, r.x, r.y);
            }
        }
    }
}

/// The computer's own outputs in the layout: the console primary first, then left to right.
fn console_outputs(state: &State) -> Vec<Output> {
    let primary = state.console_primary();
    let mut outs: Vec<Output> = (state.scene.outputs().iter())
        .filter(|o| !is_remote(o) && state.layout_box(Some(o)).width > 0)
        .cloned()
        .collect();
    outs.sort_by_key(|o| {
        let b = state.layout_box(Some(o));
        (primary.as_ref() != Some(o), b.x, b.y)
    });
    outs
}

/// Every mapped window but X11 menus and tooltips, minimized and on other desktops too.
fn movable(state: &State) -> Vec<WindowId> {
    (state.windows.iter())
        .filter(|w| w.mapped && !w.override_redirect)
        .map(|w| w.id)
        .collect()
}

/// Which of `outs` has `w`'s [`anchor`].
fn under(state: &State, w: WindowId, outs: &[Output]) -> Option<usize> {
    let (x, y) = anchor(state.win(w)?);
    (outs.iter()).position(|o| state.layout_box(Some(o)).contains(x, y))
}

/// The point that says which output a window is on: its frame's centre, or the origin of a
/// maximized or full screen one, whose size lags its output's until the client commits.
fn anchor(win: &Window) -> (i32, i32) {
    let f = win.frame_rect();
    if win.maximized || win.fullscreen {
        (f.x, f.y)
    } else {
        (f.x + f.width / 2, f.y + f.height / 2)
    }
}

fn fit_of(state: &State, w: WindowId) -> Option<Fit> {
    let win = state.win(w)?;
    Some(if win.fullscreen {
        Fit::Fullscreen
    } else if win.maximized {
        Fit::Maximized
    } else if state.wm.snap.group_of(w).is_some() {
        Fit::Grouped
    } else if win.snapped.is_some() {
        Fit::Snapped
    } else {
        Fit::Normal
    })
}

fn home_of(state: &State, w: WindowId, monitor: &str, area: Rect) -> Option<Home> {
    let win = state.win(w)?;
    let rel = |r: Option<Rect>| r.map(|r| RelRect::of(r, area));
    Some(Home {
        monitor: monitor.to_string(),
        rel: RelRect::of(win.unshaded_frame(), area),
        fit: fit_of(state, w)?,
        saved_rel: rel(win.saved),
        saved_fullscreen_rel: rel(win.saved_fullscreen),
        snapped_rel: rel(win.snapped),
    })
}

/// `r` at the same relative place in `to` as in `from`, its size kept, inside `to`.
fn carried(r: Rect, from: Rect, to: Rect) -> Rect {
    let at = RelRect::of(r, from).at(to);
    let (x, y) = keep_inside((at.x, at.y), (r.width, r.height), to);
    Rect::new(x, y, r.width, r.height)
}

/// `w` from the work area `from` to `to`'s: each rect where `home` has it, else at the same
/// relative place.
fn carry(state: &mut State, w: WindowId, from: Rect, to: &Output, home: Option<&Home>) {
    let area = state.usable_area(Some(to));
    let Some(fit) = fit_of(state, w) else {
        return;
    };
    let group_on_remote = (state.wm.snap.group_output(w))
        .is_some_and(|name| remote_outputs(state).iter().any(|o| o.name() == name));
    let Some(win) = state.win_mut(w) else {
        return;
    };
    let moved =
        |r: Rect, kept: Option<RelRect>| kept.map_or(carried(r, from, area), |k| k.at(area));
    win.saved = (win.saved).map(|r| moved(r, home.and_then(|h| h.saved_rel)));
    win.saved_fullscreen =
        (win.saved_fullscreen).map(|r| moved(r, home.and_then(|h| h.saved_fullscreen_rel)));
    win.snapped = (win.snapped).map(|r| moved(r, home.and_then(|h| h.snapped_rel)));
    let frame = win.unshaded_frame();
    let exact = home.filter(|h| h.fit == fit).map(|h| h.rel.at(area));
    match fit {
        Fit::Maximized | Fit::Fullscreen => {
            let b = state.layout_box(Some(to));
            centre(state, w, b);
        }
        Fit::Grouped if group_on_remote != is_remote(to) => {
            state.wm.snap.move_group(w, &to.name());
        }
        Fit::Grouped => {}
        Fit::Snapped => {
            let zone = exact.unwrap_or_else(|| RelRect::of(frame, from).at(area));
            state.snap_commit(w, zone);
        }
        Fit::Normal => match exact {
            Some(r) => put(state, w, r),
            None => {
                let r = carried(frame, from, area);
                state.move_to(w, r.x, r.y);
            }
        },
    }
}

/// Centre maximized or full screen `w` on `b` for the layout's `refit_maximized` to fill; a fit
/// here too would leave the old size at the output's origin, its centre maybe off the output.
fn centre(state: &mut State, w: WindowId, b: Rect) {
    let Some(win) = state.win_mut(w) else {
        return;
    };
    let f = win.frame_rect();
    win.x = b.x + (b.width - f.width) / 2;
    win.y = b.y + (b.height - f.height) / 2;
}

/// `w`'s frame to `r`, asking the client for `r`'s size when it differs.
fn put(state: &mut State, w: WindowId, r: Rect) {
    state.move_to(w, r.x, r.y);
    let Some(win) = state.win(w) else {
        return;
    };
    if win.shaded || win.geom.total == (r.width, r.height) {
        return;
    }
    let m = &state.config.theme.metrics;
    let (cw, ch) = FrameGeometry::client_size_for_total(m, r.width, r.height, false, win.chrome());
    state.request_size(w, cw, ch);
}

#[cfg(test)]
mod tests;
