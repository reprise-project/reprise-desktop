//! Undo and redo: a step keeps the changed rectangle before and after (the whole picture when its
//! size changed); past [`DEPTH`] steps or [`BUDGET`] bytes the oldest go, the newest stays.

use crate::bitmap::Bitmap;

pub const DEPTH: usize = 50;

/// Bytes of pixels the steps may hold together.
pub const BUDGET: usize = 256 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Patch {
        at: (i32, i32),
        before: Bitmap,
        after: Bitmap,
    },
    Whole {
        before: Bitmap,
        after: Bitmap,
    },
}

impl Step {
    fn bytes(&self) -> usize {
        let (Step::Patch { before, after, .. } | Step::Whole { before, after }) = self;
        (before.px.len() + after.px.len()) * 4
    }
}

#[derive(Clone, Debug)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    budget: usize,
}

impl Default for History {
    fn default() -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            budget: BUDGET,
        }
    }
}

impl History {
    /// Records the change from `before` to `after`; false when they are the same.
    pub fn record(&mut self, before: &Bitmap, after: &Bitmap) -> bool {
        let step = if (before.w, before.h) != (after.w, after.h) {
            Step::Whole {
                before: before.clone(),
                after: after.clone(),
            }
        } else {
            let Some(r) = before.differing(after) else {
                return false;
            };
            Step::Patch {
                at: (r.x, r.y),
                before: before.copy(r, 0),
                after: after.copy(r, 0),
            }
        };
        self.redo.clear();
        self.undo.push(step);
        let mut total: usize = self.undo.iter().map(Step::bytes).sum();
        while self.undo.len() > 1 && (self.undo.len() > DEPTH || total > self.budget) {
            total -= self.undo.remove(0).bytes();
        }
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    #[cfg(test)]
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    pub fn undo(&mut self, img: &mut Bitmap) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        apply(img, &step, false);
        self.redo.push(step);
        true
    }

    pub fn redo(&mut self, img: &mut Bitmap) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        apply(img, &step, true);
        self.undo.push(step);
        true
    }
}

fn apply(img: &mut Bitmap, step: &Step, forward: bool) {
    match step {
        Step::Patch { at, before, after } => {
            img.paste(at.0, at.1, if forward { after } else { before });
        }
        Step::Whole { before, after } => {
            *img = if forward { after } else { before }.clone();
        }
    }
}

#[cfg(test)]
use reprisekit::raster::Rect;

/// The rectangle a patch step covers, for tests.
#[cfg(test)]
fn patch_rect(h: &History) -> Option<Rect> {
    match h.undo.last()? {
        Step::Patch { at, before, .. } => Some(Rect::new(at.0, at.1, before.w, before.h)),
        Step::Whole { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmap::{BLACK, WHITE};

    #[test]
    fn undo_and_redo_walk_the_steps() {
        let mut img = Bitmap::new(8, 8, WHITE);
        let mut h = History::default();
        let start = img.clone();
        let before = img.clone();
        img.set(2, 3, BLACK);
        img.set(4, 5, BLACK);
        assert!(h.record(&before, &img));
        assert_eq!(
            patch_rect(&h),
            Some(Rect::new(2, 3, 3, 3)),
            "only the changed box is kept"
        );
        let drawn = img.clone();
        let grown = img.with_size(10, 4, WHITE);
        assert!(h.record(&img, &grown));
        img = grown.clone();
        assert!(h.undo(&mut img));
        assert_eq!(img, drawn);
        assert!(h.undo(&mut img));
        assert_eq!(img, start);
        assert!(!h.undo(&mut img));
        assert!(h.redo(&mut img) && h.redo(&mut img));
        assert_eq!(img, grown);
        assert!(!h.record(&img, &img.clone()), "no change is no step");
    }

    #[test]
    fn a_new_step_drops_the_redo_steps() {
        let mut img = Bitmap::new(4, 4, WHITE);
        let mut h = History::default();
        let b = img.clone();
        img.set(0, 0, BLACK);
        h.record(&b, &img);
        h.undo(&mut img);
        let b = img.clone();
        img.set(1, 1, BLACK);
        h.record(&b, &img);
        assert!(!h.can_redo());
    }

    #[test]
    fn steps_past_the_depth_drop_the_oldest() {
        let mut img = Bitmap::new(4, 4, WHITE);
        let mut h = History::default();
        for i in 0..DEPTH + 5 {
            let b = img.clone();
            img.set(0, 0, i as u32 | BLACK);
            h.record(&b, &img);
        }
        assert_eq!(h.depth(), DEPTH);
    }

    #[test]
    fn a_step_over_the_budget_still_undoes_alone() {
        let side = 64;
        let mut h = History {
            budget: side as usize * side as usize * 12,
            ..History::default()
        };
        let a = Bitmap::new(side, side, WHITE);
        let b = Bitmap::new(side, side, BLACK);
        h.record(&a, &b);
        h.record(&b, &a);
        h.record(&a, &b);
        assert_eq!(h.depth(), 1, "each step is two thirds of the budget");
    }
}
