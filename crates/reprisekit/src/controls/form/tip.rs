//! The tip an ellipsized label shows of its whole text.

use super::item::Item;
use super::layout::{check_box, check_label_rect, glyph_dy, radio_box, radio_label_rect};
use super::{Control, Form};

/// The label the pointer rests on and how many blinks ([`Form::tip_tick`]) it has rested there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LabelTip {
    item: Option<usize>,
    ticks: u8,
}

/// The blinks an ellipsized label's tip stays up for (about 5 s at the host's 530 ms blink).
pub(super) const TIP_TICKS: u8 = 10;

impl Form {
    /// The label, else the check box or radio, under (`x`, `y`), if any: an ellipsized label
    /// shows its whole text in a tip ([`Form::tip_tick`]).
    pub(super) fn label_at(&self, x: i32, y: i32) -> Option<usize> {
        let cm = self.rendered.controls.get();
        let w = self.units().len(200);
        let shown = |it: &Item| self.clip_of(it).is_none_or(|c| c.contains(x, y));
        let label = self.items.iter().position(|it| {
            matches!(it.control, Control::Label { .. }) && self.at(it).contains(x, y) && shown(it)
        });
        label.or_else(|| {
            self.items.iter().position(|it| {
                let r = self.at(it);
                let over = match it.control {
                    Control::Check { .. } => {
                        check_box(r, &cm, glyph_dy(it, r)).contains(x, y)
                            || check_label_rect(r, it.indent, &cm, w).contains(x, y)
                    }
                    Control::Radio { .. } => {
                        radio_box(r, &cm, glyph_dy(it, r)).contains(x, y)
                            || radio_label_rect(r, &cm, w).contains(x, y)
                    }
                    _ => false,
                };
                over && shown(it)
            })
        })
    }

    /// The pointer now rests on label `on` (or none): a new rest starts its count again.
    /// Whether a shown tip went away.
    pub(super) fn rest_tip(&mut self, on: Option<usize>) -> bool {
        if self.tip.item == on {
            return false;
        }
        let was = self.tip_shown();
        self.tip = LabelTip { item: on, ticks: 0 };
        was
    }

    fn tip_shown(&self) -> bool {
        self.tip
            .item
            .is_some_and(|i| self.rendered.ellipsized.borrow().contains(&i))
            && (1..=TIP_TICKS).contains(&self.tip.ticks)
    }

    /// One caret blink went by: an ellipsized label the pointer rests on shows its whole text
    /// in a tip from the next blink on, for [`TIP_TICKS`] blinks. Whether the tip came or went.
    pub fn tip_tick(&mut self) -> bool {
        let before = self.tip_shown();
        if self.tip.item.is_some() {
            self.tip.ticks = self.tip.ticks.saturating_add(1);
        }
        before != self.tip_shown()
    }

    /// Whether the pointer rests on a label whose tip is still to come or still up: a host
    /// keeps calling [`Form::tip_tick`] on each blink while this holds.
    pub fn tip_waiting(&self) -> bool {
        self.tip.item.is_some() && self.tip.ticks <= TIP_TICKS
    }

    /// A caret blink for a host that times it: the caret shows as `on`, and the label tip counts
    /// the blink ([`Form::tip_tick`]). Whether the tip came or went.
    pub fn blink(&mut self, on: bool) -> bool {
        self.caret = on;
        self.tip_tick()
    }

    /// The pointer rests at `at` (`None`: it left) for a host that hands the form no other
    /// pointer input: only the label tip follows it. Whether a shown tip went away.
    pub fn rest_pointer(&mut self, at: Option<(i32, i32)>) -> bool {
        self.point = at;
        self.rest_tip(at.and_then(|(x, y)| self.label_at(x, y)))
    }

    /// Take over `old`'s pointer rest and tip count when the host builds the same form again.
    /// The label is matched by id.
    pub fn carry_tip(&mut self, old: &Form) {
        self.point = old.point;
        let id = old.tip.item.and_then(|i| old.items.get(i)).map(|it| it.id);
        self.tip = LabelTip {
            item: id.and_then(|id| self.items.iter().position(|it| it.id == id)),
            ticks: old.tip.ticks,
        };
    }

    /// The whole text of the ellipsized label whose tip is up ([`Form::tip_tick`]).
    pub fn label_tip(&self) -> Option<String> {
        if !self.tip_shown() {
            return None;
        }
        match &self.items.get(self.tip.item?)?.control {
            Control::Label { text, .. }
            | Control::Check { label: text, .. }
            | Control::Radio { label: text, .. } => Some(crate::text::split_mnemonic(text).0),
            _ => None,
        }
    }
}
