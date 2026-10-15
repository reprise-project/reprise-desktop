//! Host glue for `shake`: a caption drag feeds the detector, and a shake
//! minimizes or puts back the other windows on the dragged one's desktop
//! and output.

use std::time::Instant;

use smithay::utils::{Logical, Point};

use crate::compositor::State;
use crate::input::Interaction;
use crate::shake::{Detector, Memo, Toggle};
use crate::window::WindowId;

#[derive(Debug, Default)]
pub struct Shaking {
    /// The caption drag under way and its detector.
    drag: Option<(WindowId, Detector)>,
    memo: Option<Memo<WindowId>>,
}

/// A move of `w` began; `caption` when a held button on its title bar started it.
pub fn begin(state: &mut State, w: WindowId, caption: bool) {
    state.wm.shake.drag = (caption && state.config.window_shake).then(|| (w, Detector::default()));
}

/// The pointer moved to `pos` while `w` is dragged.
pub fn motion(state: &mut State, w: WindowId, pos: Point<f64, Logical>) {
    let Some((dragged, detector)) = state.wm.shake.drag.as_mut() else {
        return;
    };
    if *dragged != w || !detector.feed((pos.x, pos.y), Instant::now()) {
        return;
    }
    if state.switcher.panel.is_some() || state.switcher.stack.is_some() {
        return;
    }
    shake(state, w, pos);
}

/// Whether the drag of `w` is shaking: its snap outline, snap bar and tab
/// drop stay away meanwhile, so a shake never ends in a snap or a group.
pub fn shaking(state: &State, w: WindowId) -> bool {
    (state.wm.shake.drag.as_ref())
        .is_some_and(|(d, detector)| *d == w && detector.shaking(Instant::now()))
}

/// Takes down the snap outline, snap bar and tab drop `track_move` showed.
pub fn hold_drops(state: &mut State) {
    state.end_tab_drop();
    state.hide_snap_outline();
    crate::snapbarview::end(state);
    if let Interaction::Move { snap, .. } = &mut state.interaction {
        *snap = None;
    }
}

fn shake(state: &mut State, w: WindowId, pos: Point<f64, Logical>) {
    let others = others(state, w, pos);
    let minimized = |x| state.win(x).is_some_and(|x| x.minimized);
    match crate::shake::toggle(state.wm.shake.memo.as_ref(), w, minimized, &others) {
        Toggle::Minimize(list) => {
            let mut done = Vec::new();
            for x in list {
                state.set_minimized(x, true);
                if state.win(x).is_some_and(|x| x.minimized) {
                    done.push(x);
                }
            }
            log::info!("shake {w:?}: minimized {}", done.len());
            state.wm.shake.memo = Some(Memo {
                window: w,
                minimized: done,
            });
        }
        Toggle::Restore(list) => {
            log::info!("shake {w:?}: restoring {}", list.len());
            for x in list {
                state.set_minimized(x, false);
            }
            state.wm.shake.memo = None;
            state.focus_window(w);
        }
        Toggle::Nothing => {}
    }
}

/// The windows a shake of `w` minimizes, bottom to top: task windows on the
/// output under the pointer, visible and not kept on top.
fn others(state: &State, w: WindowId, pos: Point<f64, Logical>) -> Vec<WindowId> {
    let home = state.home(w);
    let out = state.output_at(pos.x, pos.y);
    let mut list: Vec<WindowId> = (state.task_windows().into_iter())
        .filter(|&x| x != home)
        .filter(|&x| {
            state.win(x).is_some_and(|x| {
                let r = x.frame_rect();
                let centre = (r.x + r.width / 2, r.y + r.height / 2);
                !x.hidden()
                    && !x.on_top
                    && !x.skips_taskbar()
                    && state.output_at(f64::from(centre.0), f64::from(centre.1)) == out
            })
        })
        .collect();
    let at = |x: &WindowId| state.focus_order.iter().position(|f| f == x);
    list.sort_by_key(at);
    list
}
