//! Identify's numbers: the Display page names the outputs in its order
//! (`org.reprise.Tray.Identify`), and each one in the layout shows its
//! number, from 1, as a `reprisekit::controls::arrange` badge in its
//! middle for 2 s, on a glass backdrop under a look with glass. Copies
//! (Duplicate) share one box and one badge with all their numbers. The
//! badges sit in `layers.overlay` and let the pointer through. A second
//! Identify replaces the first.

use std::time::Duration;

use reprisekit::controls::arrange::{self, BADGE};
use reprisekit::look::panel;
use reprisescene::{Input, TreeId};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::output::upload_scaled;
use crate::raster::Rect;

const HOLD: Duration = Duration::from_secs(2);

/// The badges while shown, and the timer that takes them away.
#[derive(Default)]
pub struct IdentifyView {
    badges: Vec<Badge>,
    timer: Option<RegistrationToken>,
}

struct Badge {
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
}

/// Where a badge goes on an output whose layout box is `b`: its middle.
fn badge_origin(b: Rect) -> (i32, i32) {
    (b.x + (b.width - BADGE) / 2, b.y + (b.height - BADGE) / 2)
}

/// `numbered` (an output, its box and number) as one entry per box, in first-seen order, with
/// the numbers of every output showing that box in rising order.
fn by_box<O>(numbered: Vec<(O, Rect, u32)>) -> Vec<(O, Rect, Vec<u32>)> {
    let mut boxes: Vec<(O, Rect, Vec<u32>)> = Vec::new();
    for (out, b, n) in numbered {
        match boxes.iter_mut().find(|(_, seen, _)| *seen == b) {
            Some((_, _, numbers)) => numbers.push(n),
            None => boxes.push((out, b, vec![n])),
        }
    }
    for (_, _, numbers) in &mut boxes {
        numbers.sort_unstable();
    }
    boxes
}

/// Number each output named in `names`, the first 1, unless its side is
/// locked (`locked_on`); the rest show nothing.
pub fn show(state: &mut State, names: &[String]) {
    hide(state);
    let numbered = (state.scene.outputs().iter())
        .filter_map(|out| {
            let k = names.iter().position(|n| *n == out.name())?;
            let b = state.layout_box(Some(out));
            (!state.locked_on(out) && b.width > 0).then(|| (out.clone(), b, k as u32 + 1))
        })
        .collect();
    for (out, b, numbers) in by_box(numbered) {
        let scale = state.draw_scale_at(Some(&out)).max(1);
        let theme = state.config.theme.scaled(scale);
        let canvas = arrange::draw_badge(&theme, state.text.for_output(theme.scale), &numbers);
        let m = panel::margin(&state.config.theme);
        let at = badge_origin(b);
        let pane = match state.config.theme.glass() {
            Some(g) if crate::glass::wanted(&state.config) => Some(Pane {
                shapes: vec![PaneShape {
                    rect: Rect::new(0, 0, BADGE, BADGE),
                    corner: g.panel.corner,
                    square_bottom: false,
                    square_top: false,
                }],
                holes: Vec::new(),
            }),
            _ => None,
        };
        let scene = &mut state.scene;
        let tree = scene.tree_create(state.layers.overlay);
        crate::glass::leave_out(scene, tree);
        scene.set_position(tree, at.0, at.1);
        let glass_tree = scene.tree_create(tree);
        let mut backdrop = Backdrop::new(&state.glass);
        backdrop.set(scene, &state.glass, glass_tree, pane);
        let buffer = scene.buffer_create(tree);
        scene.set_position(buffer, -m.left, -m.top);
        scene.set_input(buffer, Input::Passthrough);
        upload_scaled(scene, buffer, &canvas, scale);
        state.identify.badges.push(Badge {
            tree,
            glass_tree,
            backdrop,
        });
    }
    if state.identify.badges.is_empty() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(HOLD), |_, _, state| {
            state.identify.timer = None;
            hide(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => state.identify.timer = Some(t),
        Err(e) => log::warn!("identify: no hide timer: {}", e.error),
    }
    state.redraw_all();
}

/// Take the badges away and stop their timer.
pub fn hide(state: &mut State) {
    if let Some(t) = state.identify.timer.take() {
        state.handle.remove(t);
    }
    if state.identify.badges.is_empty() {
        return;
    }
    for mut b in std::mem::take(&mut state.identify.badges) {
        b.backdrop
            .set(&mut state.scene, &state.glass, b.glass_tree, None);
        state.scene.destroy(b.tree);
    }
    state.redraw_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_badge_sits_in_the_middle_of_its_output() {
        assert_eq!(badge_origin(Rect::new(0, 0, 1920, 1080)), (860, 440));
        assert_eq!(badge_origin(Rect::new(1920, -200, 1280, 1024)), (2460, 212));
    }

    #[test]
    fn copies_showing_one_box_share_a_badge_with_their_numbers() {
        let (a, b) = (Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 1024));
        let got = by_box(vec![("DP-2", a, 3), ("HDMI-A-1", b, 2), ("DP-1", a, 1)]);
        assert_eq!(got, [("DP-2", a, vec![1, 3]), ("HDMI-A-1", b, vec![2])]);
    }
}
