//! Task buttons in motion while a drag reorders the row: the dragged button
//! floats under the pointer (`Bar::task_float`) with the rest of its unit
//! (`taskbar::row_units`); with `[effects] taskbar_slide` on, moved buttons slide
//! to their new x (`Bar::task_slides`) and the dropped button glides into its
//! slot. They step on the frame clock (`animclock`) while anything moves.

use std::time::Instant;

use reprisekit::anim::{Ease, Slide};
use reprisescene::TreeId;

use crate::{
    animclock::Anim, compositor::State, input::Interaction, taskbar::TaskKey, taskbarview::Bar,
    window::WindowId,
};
const SLIDE_MS: u32 = 150;
const GLIDE_MS: u32 = 200;

pub type Key = TaskKey<WindowId>;

/// Each of `bar`'s task buttons by key, with its x in the last layout.
pub fn row(bar: &Bar) -> Vec<(Key, i32)> {
    (0..bar.layout.tasks.len())
        .filter_map(|i| {
            let k = crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i)?;
            Some((k, bar.layout.to_along(bar.layout.tasks[i]).x))
        })
        .collect()
}

/// The buttons of `bar` that move with button `i` in a drag (its unit).
fn unit_mates(bar: &Bar, i: usize) -> impl Iterator<Item = usize> {
    crate::taskbar::row_units(&bar.pins, &bar.runs)
        .into_iter()
        .find(|u| u.contains(&i))
        .unwrap_or(i..i + 1)
        .filter(move |&j| j != i)
}

/// Forget the slide and float of button `key` on `bar`; its window is gone.
pub fn forget(bar: &mut Bar, key: Key) {
    bar.task_slides.retain(|(k, _)| *k != key);
    if bar.task_float.is_some_and(|(k, _)| k == key) {
        bar.task_float = None;
    }
}

/// What `bar`'s next draw shows at `now`: each task button's offset off its
/// slot, and the floated button's index and x. Notes the x as drawn.
pub fn draw_state(bar: &mut Bar, now: Instant) -> (Vec<i32>, Option<(usize, i32)>) {
    let float = bar.task_float.and_then(|(k, s)| {
        let i = crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &k)?;
        Some((i, s.x(now)))
    });
    bar.task_float_drawn = float.map(|(_, x)| x);
    let mut shift = if bar.task_slides.is_empty() {
        Vec::new()
    } else {
        (0..bar.layout.tasks.len())
            .map(|i| {
                let k = crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i);
                k.and_then(|k| bar.task_slides.iter().find(|(s, _)| *s == k))
                    .map_or(0, |(_, s)| s.offset(now))
            })
            .collect()
    };
    if let Some((i, x)) = float {
        let off = x - bar.layout.to_along(bar.layout.tasks[i]).x;
        let mates: Vec<usize> = unit_mates(bar, i).collect();
        if !mates.is_empty() {
            shift.resize(bar.layout.tasks.len(), 0);
            for j in mates {
                shift[j] = off;
            }
        }
    }
    (shift, float)
}

impl State {
    /// After a drag moved the row of bar `tree` from `old` (see [`row`]), with the
    /// pins taking `order` (old indices in new order): every button outside the
    /// `dragged` unit whose x changed slides from where it is drawn. Running slides
    /// keep going (`pins_reordered` renamed their pins).
    pub fn slide_row_moved(
        &mut self,
        tree: TreeId,
        old: &[(Key, i32)],
        order: &[usize],
        dragged: Key,
    ) {
        if !self.config.effects.taskbar_slide {
            return;
        }
        let old: Vec<(Key, i32)> = old
            .iter()
            .filter_map(|&(k, x)| Some((crate::taskbar::pin_renamed(k, order)?, x)))
            .collect();
        let now = Instant::now();
        let Some(bar) = self.bar_mut(tree) else {
            return;
        };
        let unit: Vec<Key> =
            crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &dragged)
                .map(|i| {
                    unit_mates(bar, i)
                        .filter_map(|j| {
                            crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, j)
                        })
                        .collect()
                })
                .unwrap_or_default();
        bar.task_slides.retain(|(k, _)| !unit.contains(k));
        let mut started = false;
        for (k, from, to) in crate::taskbar::moved_buttons(&old, &row(bar)) {
            if k == dragged || unit.contains(&k) {
                continue;
            }
            started = true;
            match bar.task_slides.iter_mut().find(|(s, _)| *s == k) {
                Some((_, s)) => s.retarget(to, now),
                None => bar
                    .task_slides
                    .push((k, Slide::new(from, to, now, SLIDE_MS, Ease::Out))),
            }
        }
        if started {
            self.redraw_taskbar(tree);
            self.animate(Anim::TaskSlide);
        }
    }

    /// The pointer moved while task button `key` on bar `tree` is dragged, grabbed
    /// `grab` px into it: it floats under the pointer.
    pub fn slide_follow(&mut self, tree: TreeId, key: Key, grab: i32) {
        let p = self.pointer_location();
        let Some(bar) = self.bar(tree) else {
            return;
        };
        let Some(i) = crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key) else {
            return;
        };
        let pointer = self.bar_along(bar, p.x, p.y);
        let row = bar.layout.along_rects(&bar.layout.tasks);
        let unit = crate::taskbar::row_units(&bar.pins, &bar.runs)
            .into_iter()
            .find(|u| u.contains(&i))
            .unwrap_or(i..i + 1);
        let Some(x) = crate::taskbar::float_x(&row, unit, i, pointer, grab) else {
            return;
        };
        let Some(bar) = self.bar_mut(tree) else {
            return;
        };
        bar.task_float = Some((key, Slide::at(x)));
        if bar.task_float_drawn != Some(x) {
            self.animate(Anim::TaskSlide);
        }
    }

    /// The drag on bar `tree` ended with a release: the floated button glides into
    /// its slot, or lands at once with `taskbar_slide` off.
    pub fn slide_release(&mut self, tree: TreeId) {
        let slide = self.config.effects.taskbar_slide;
        let Some(bar) = self.bar_mut(tree) else {
            return;
        };
        let Some((key, at)) = bar.task_float else {
            return;
        };
        let slot = crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key)
            .and_then(|i| bar.layout.tasks.get(i))
            .map(|r| bar.layout.to_along(*r).x);
        // From where the float is on screen; a motion since the last frame has not
        // moved it yet.
        let now = Instant::now();
        let from = bar.task_float_drawn.unwrap_or(at.x(now));
        match slot.filter(|&to| slide && to != from) {
            Some(to) => {
                bar.task_float = Some((key, Slide::new(from, to, now, GLIDE_MS, Ease::InOut)));
                self.animate(Anim::TaskSlide);
            }
            None => {
                bar.task_float = None;
                self.redraw_taskbar(tree);
            }
        }
    }

    /// Settle bar `tree` at `now`: drop arrived slides and a landed float. Returns
    /// whether it wants a redraw, and whether it still moves.
    fn settle_slides(&mut self, tree: TreeId, now: Instant) -> (bool, bool) {
        let held = matches!(
            self.interaction,
            Interaction::TaskPress { bar, dragging: true, .. } if bar == tree
        );
        let Some(bar) = self.bar_mut(tree) else {
            return (false, false);
        };
        let before = bar.task_slides.len();
        bar.task_slides.retain(|(_, s)| !s.settled(now));
        let mut redraw = bar.task_slides.len() != before || !bar.task_slides.is_empty();
        if let Some((_, s)) = bar.task_float {
            if !held && s.settled(now) {
                bar.task_float = None;
                redraw = true;
            } else if bar.task_float_drawn != Some(s.x(now)) {
                redraw = true;
            }
        }
        let gliding = bar.task_float.is_some_and(|(_, s)| !s.settled(now));
        (redraw, !bar.task_slides.is_empty() || gliding)
    }

    /// One frame of every bar in motion; whether anything still moves.
    pub(crate) fn slide_step(&mut self, now: Instant) -> bool {
        let trees: Vec<TreeId> = self.bars.iter().map(|b| b.tree).collect();
        let mut moving = false;
        for tree in trees {
            let (redraw, still) = self.settle_slides(tree, now);
            if redraw {
                self.redraw_taskbar(tree);
            }
            // A step that moves nothing a whole px still asks the next frame.
            if still && let Some(o) = self.bar(tree).map(|b| b.output.clone()) {
                crate::backend::schedule(self, &o);
            }
            moving |= still;
        }
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_task_button_glides_on_its_bars_frames_until_it_lands() {
        let mut config = crate::config::Config::default();
        config.effects.taskbar_slide = true;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        h.map_toplevel(&mut c, "a.app", 300, 200);
        h.map_toplevel(&mut c, "b.app", 300, 200);
        let tree = h.state.bars[0].tree;
        let output = h.state.bars[0].output.name();
        let (key, x) = row(h.state.bar(tree).unwrap())[0];
        h.pointer_to(f64::from(x + 200), 768.0 - 15.0);
        h.state.slide_follow(tree, key, 0);
        assert!(h.state.bar(tree).unwrap().task_float.is_some(), "it floats");
        h.state.slide_release(tree);
        let end = Instant::now() + std::time::Duration::from_millis(u64::from(GLIDE_MS));
        crate::animclock::assert_steps_until(&mut h.state, Anim::TaskSlide, &[&output], end);
        assert!(h.state.bar(tree).unwrap().task_float.is_none(), "it landed");
    }
}
