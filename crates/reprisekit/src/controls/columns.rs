//! The column strip under Volume Control's mixer and the tray volume popup: columns kept by key
//! across a sync, output combos with one list open at a time, a focus walk and a column scroll.

use crate::classic::widgets::{ScrollHit, ScrollParts, scrollbar_pos_at};
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::pointer::{Interaction, Lit, Pointer, WHEEL_ROWS};
use crate::trackbar::TrackBar;
use std::cell::Cell;

/// What a strip keeps of the pointer besides its [`Pointer`].
#[derive(Default)]
pub(crate) struct Hover {
    /// Where the pointer last pressed or moved; `None` once it left. A key or a sync finds the
    /// part under it again.
    pub point: Option<(i32, i32)>,
    /// How far right of its left edge the held scrollbar thumb was grabbed.
    pub grab: i32,
    /// Whether the last draw's look lights a part under the pointer.
    pub styled: Cell<bool>,
}

/// A strip's column as [`Remap::sync`] matches it.
pub(crate) trait Column {
    type Key: PartialEq;
    fn key(&self) -> &Self::Key;
    fn output(&mut self) -> &mut Option<Combo>;
}

/// A column's trackbar, for the thumb a sync carries.
pub(crate) type Bar<C> = fn(&mut C) -> &mut TrackBar;

/// Where each column of a strip went across a sync.
pub(crate) struct Remap {
    /// Old column to new, by key; `None` for one that left.
    moved: Vec<Option<usize>>,
    /// New columns whose output combo took new rows, which closed its list.
    rebuilt: Vec<bool>,
    /// New columns with an output combo.
    combo: Vec<bool>,
}

impl Remap {
    /// Match `new` to `old` by key. A held thumb keeps its position since the device lags the
    /// drag's writes; an output combo keeps its open list while its `rows` stay.
    pub fn sync<C: Column>(
        old: &mut [C],
        new: &mut [C],
        held: Option<(usize, Bar<C>)>,
        rows: impl Fn(usize, &C) -> Option<(Vec<String>, Option<usize>)>,
        list_rows: usize,
    ) -> Remap {
        let moved: Vec<Option<usize>> = old
            .iter()
            .map(|o| new.iter().position(|c| c.key() == o.key()))
            .collect();
        if let Some((i, bar)) = held
            && let Some(j) = moved.get(i).copied().flatten()
        {
            let pos = bar(&mut old[i]).pos;
            bar(&mut new[j]).set(pos);
        }
        let mut rebuilt = vec![false; new.len()];
        for (o, j) in old.iter_mut().zip(&moved) {
            if let (Some(j), Some(mut c)) = (*j, o.output().take())
                && new[j].output().is_some()
                && let Some((names, picked)) = rows(j, &new[j])
            {
                rebuilt[j] = c.sync_names(&names, picked, list_rows);
                *new[j].output() = Some(c);
            }
        }
        let combo = new.iter_mut().map(|c| c.output().is_some()).collect();
        Remap {
            moved,
            rebuilt,
            combo,
        }
    }

    /// Where old column `i` went.
    pub fn get(&self, i: usize) -> Option<usize> {
        self.moved.get(i).copied().flatten()
    }

    /// Whether new column `j` has an output combo.
    pub fn has_combo(&self, j: usize) -> bool {
        self.combo.get(j).copied().unwrap_or(false)
    }

    /// Where old column `i`'s output combo went while it kept its rows: a part held or lit on
    /// it stays.
    pub fn combo_kept(&self, i: usize) -> Option<usize> {
        self.get(i)
            .filter(|&j| self.has_combo(j) && !self.rebuilt[j])
    }
}

/// The two events every strip's input returns.
pub(crate) trait StripEvent: PartialEq {
    const NONE: Self;
    const REDRAW: Self;
}

/// A wheel turn of `rows` rows ([`WHEEL_ROWS`] per notch) in notches; a turn short of a notch
/// counts as one.
pub(crate) fn notches(rows: i32) -> i32 {
    rows.signum() * (rows.abs() / WHEEL_ROWS).max(1)
}

/// A strip's state and parts, for the input both controls share. Columns from
/// [`Self::PINNED`] on scroll a column at a time.
pub(crate) trait Strip {
    type Part: Copy + Eq;
    type Focus: Copy + Eq;
    /// A combo on the strip.
    type Pick: Copy + Eq;
    type Layout;
    type Event: StripEvent;

    /// Columns at the strip's start that stay in view.
    const PINNED: usize;

    fn count(&self) -> usize;
    fn hover_mut(&mut self) -> &mut Hover;
    fn ptr_mut(&mut self) -> &mut Pointer<Self::Part>;
    fn focus_mut(&mut self) -> &mut Self::Focus;
    fn cues_mut(&mut self) -> &mut bool;
    /// The first scrolling column shown, counted from [`Self::PINNED`].
    fn first_mut(&mut self) -> &mut usize;
    /// Scrolling columns shown side by side on `lay`.
    fn shown(&self, lay: &Self::Layout) -> usize;
    /// `lay` with every shown thumb where its trackbar puts it now.
    fn current(&self, lay: &Self::Layout) -> Self::Layout;
    /// The part at (`x`, `y`) on a [`Self::current`] layout.
    fn part_at(&self, lay: &Self::Layout, x: i32, y: i32) -> Option<Self::Part>;
    /// Whether held part `p` shows pressed in every look.
    fn pressed(p: Self::Part) -> bool;
    /// The combos, in the order an open list is looked for.
    fn picks(&self) -> impl Iterator<Item = Self::Pick>;
    fn pick(&self, p: Self::Pick) -> Option<&Combo>;
    fn pick_mut(&mut self, p: Self::Pick) -> Option<&mut Combo>;
    /// The combo and its part at (`x`, `y`).
    fn combo_hit(&self, lay: &Self::Layout, x: i32, y: i32) -> Option<(Self::Pick, combo::Hit)>;
    fn combo_layout(lay: &Self::Layout, p: Self::Pick) -> Option<&combo::Layout>;
    /// The column combo `p` sits in; `None` for one off the columns.
    fn pick_column(p: Self::Pick) -> Option<usize>;
    /// The event for row `row` picked on combo `p`.
    fn picked(&self, p: Self::Pick, row: usize) -> Self::Event;
    /// The Tab order; a greyed part is no stop.
    fn focus_order(&self) -> Vec<Self::Focus>;
    /// The column focus `f` lies in; `None` for one off the columns.
    fn focus_column(f: Self::Focus) -> Option<usize>;
    /// The scrollbar's parts, while it shows.
    fn bar(&self, lay: &Self::Layout) -> Option<ScrollParts>;

    fn open_list(&self) -> Option<Self::Pick> {
        self.picks()
            .find(|&p| self.pick(p).is_some_and(|c| c.open.is_some()))
    }

    fn close_list(&mut self) {
        if let Some(p) = self.open_list()
            && let Some(c) = self.pick_mut(p)
        {
            c.open = None;
        }
    }

    /// Combo `p` takes `f`, and its act becomes a redraw or, for a pick, the pick's event.
    fn on_combo(&mut self, p: Self::Pick, f: impl FnOnce(&mut Combo) -> combo::Act) -> Self::Event {
        match self.pick_mut(p).map_or(combo::Act::None, f) {
            combo::Act::None => Self::Event::NONE,
            combo::Act::Redraw => Self::Event::REDRAW,
            combo::Act::Picked(row) => self.picked(p, row),
        }
    }

    fn rehit(&mut self, lay: &Self::Layout) -> Self::Event {
        let Some((x, y)) = self.hover_mut().point else {
            return Self::Event::NONE;
        };
        let over = self.part_at(&self.current(lay), x, y);
        let changed = self.ptr_mut().motion(over);
        self.lit(changed)
    }

    /// `Redraw` when what draws changed and the look shows it; a held [`Self::pressed`] part
    /// shows in every look.
    fn lit(&mut self, changed: bool) -> Self::Event {
        let pressed = self.ptr_mut().held().is_some_and(Self::pressed);
        if changed && (self.hover_mut().styled.get() || pressed) {
            Self::Event::REDRAW
        } else {
            Self::Event::NONE
        }
    }

    fn lit_from(&mut self, before: Lit<Self::Part>) -> Self::Event {
        let changed = self.ptr_mut().lit() != before;
        if changed && matches!(before, Some((p, Interaction::Pressed)) if Self::pressed(p)) {
            Self::Event::REDRAW
        } else {
            self.lit(changed)
        }
    }

    /// Step the focus along [`Self::focus_order`], scrolling its column into view.
    fn walk(&mut self, lay: &Self::Layout, back: bool) {
        let order = self.focus_order();
        let n = order.len();
        if n == 0 {
            return;
        }
        let next = match order.iter().position(|&f| f == *self.focus_mut()) {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        *self.focus_mut() = order[next];
        self.reveal_focus(lay);
    }

    fn reveal_focus(&mut self, lay: &Self::Layout) {
        if let Some(i) = Self::focus_column(*self.focus_mut()) {
            self.show_column(lay, i);
        }
    }

    /// Scroll so column `i` shows.
    fn show_column(&mut self, lay: &Self::Layout, i: usize) {
        let Some(a) = i.checked_sub(Self::PINNED) else {
            return;
        };
        let (shown, last) = (self.shown(lay), self.last_first(lay));
        let first = self.first_mut();
        *first = (*first).min(last);
        if a < *first {
            *first = a;
        } else if a >= *first + shown {
            *first = a + 1 - shown;
        }
    }

    fn last_first(&self, lay: &Self::Layout) -> usize {
        self.count().saturating_sub(Self::PINNED + self.shown(lay))
    }

    /// After a sync: the columns scroll back as far as they now must, and the part under the
    /// pointer is found again.
    fn synced(&mut self, lay: &Self::Layout) -> Self::Event {
        let last = self.last_first(lay);
        let first = self.first_mut();
        *first = (*first).min(last);
        self.rehit(lay)
    }

    /// The wheel on the scrollbar: a column per notch.
    fn scroll_bar(&mut self, lay: &Self::Layout, notches: i32) -> Self::Event {
        let last = self.last_first(lay) as i64;
        let first = (*self.first_mut() as i64 + i64::from(notches)).clamp(0, last) as usize;
        if first == *self.first_mut() {
            return Self::Event::NONE;
        }
        *self.first_mut() = first;
        // A list scrolled out of view would go on taking the keys unseen.
        if let Some(a) = self
            .open_list()
            .and_then(Self::pick_column)
            .and_then(|i| i.checked_sub(Self::PINNED))
            && !(first..first + self.shown(lay)).contains(&a)
        {
            self.close_list();
        }
        self.rehit(lay);
        Self::Event::REDRAW
    }

    /// A press on the scrollbar: an arrow scrolls one column, the track a page; the thumb is
    /// held where it was grabbed.
    fn press_bar(&mut self, lay: &Self::Layout, h: ScrollHit, x: i32) -> Self::Event {
        let (shown, last) = (self.shown(lay), self.last_first(lay));
        // A body that grew since the last scroll shows fewer columns past the first than
        // `first` left room for.
        let first = (*self.first_mut()).min(last);
        *self.first_mut() = first;
        *self.first_mut() = match h {
            ScrollHit::Back => first.saturating_sub(1),
            ScrollHit::Forward => (first + 1).min(last),
            ScrollHit::PageBack => first.saturating_sub(shown),
            ScrollHit::PageForward => (first + shown).min(last),
            ScrollHit::Thumb => {
                if let Some(thumb) = self.bar(lay).and_then(|p| p.thumb) {
                    self.hover_mut().grab = x - thumb.x;
                }
                first
            }
        };
        Self::Event::REDRAW
    }

    /// The held scrollbar thumb follows the pointer's `x`.
    fn drag_bar(&mut self, lay: &Self::Layout, x: i32) -> Self::Event {
        let Some(p) = self.bar(lay) else {
            return Self::Event::NONE;
        };
        let count = self.count() - Self::PINNED;
        let to = scrollbar_pos_at(&p, x - self.hover_mut().grab, self.shown(lay), count, true);
        if to == *self.first_mut() {
            Self::Event::NONE
        } else {
            *self.first_mut() = to;
            Self::Event::REDRAW
        }
    }

    /// A reader's Focus on `f`: it takes the keyboard as Tab gives it, with its focus ring
    /// shown. A part off the focus order takes nothing.
    fn focus_to(&mut self, f: Self::Focus) -> Self::Event {
        if !self.focus_order().contains(&f) {
            return Self::Event::NONE;
        }
        let cued = !*self.cues_mut();
        *self.cues_mut() = true;
        if *self.focus_mut() == f {
            return if cued {
                Self::Event::REDRAW
            } else {
                Self::Event::NONE
            };
        }
        self.close_list();
        *self.focus_mut() = f;
        Self::Event::REDRAW
    }

    /// A press at (`x`, `y`) on a [`Self::current`] layout while a list is open: on the list it
    /// acts there; anywhere else it closes the list and nothing under it acts at the release.
    /// `None` with no list open.
    fn press_open_list(&mut self, lay: &Self::Layout, x: i32, y: i32) -> Option<Self::Event> {
        let open = self.open_list()?;
        if let Some((p, h)) = self.combo_hit(lay, x, y)
            && p == open
        {
            return Some(self.on_combo(p, |c| c.press_list(h)));
        }
        self.close_list();
        self.ptr_mut().clear();
        Some(Self::Event::REDRAW)
    }

    /// The wheel over an open list scrolls it; `None` off one.
    fn scroll_list(
        &mut self,
        lay: &Self::Layout,
        x: i32,
        y: i32,
        rows: i32,
    ) -> Option<Self::Event> {
        let p = self.open_list()?;
        if !Self::combo_layout(lay, p).is_some_and(|cl| cl.list.contains(x, y)) {
            return None;
        }
        Some(if self.pick_mut(p).is_some_and(|c| c.scroll(rows)) {
            Self::Event::REDRAW
        } else {
            Self::Event::NONE
        })
    }

    /// A key: with a list open it goes to the list; otherwise, with `reveal`, a focused column
    /// scrolled out of view scrolls back first, then `act` takes it. The first key shows the
    /// focus ring even when it moves nothing.
    fn key_with(
        &mut self,
        lay: &Self::Layout,
        key: form::Key,
        reveal: bool,
        act: impl FnOnce(&mut Self, form::Key) -> Self::Event,
    ) -> Self::Event {
        let first = !*self.cues_mut();
        *self.cues_mut() = true;
        if let Some(p) = self.open_list() {
            return self.on_combo(p, |c| c.list_key(key));
        }
        let scrolled = *self.first_mut();
        if reveal {
            self.reveal_focus(lay);
        }
        let ev = act(self, key);
        let lit = self.rehit(lay);
        if ev != Self::Event::NONE {
            ev
        } else if first || *self.first_mut() != scrolled {
            Self::Event::REDRAW
        } else {
            lit
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Col {
        key: &'static str,
        volume: TrackBar,
        output: Option<Combo>,
    }

    impl Column for Col {
        type Key = &'static str;
        fn key(&self) -> &&'static str {
            &self.key
        }
        fn output(&mut self) -> &mut Option<Combo> {
            &mut self.output
        }
    }

    fn col(key: &'static str, pos: usize) -> Col {
        Col {
            key,
            volume: TrackBar::new(pos, 21),
            output: None,
        }
    }

    fn sync(old: &mut [Col], new: &mut [Col], held: usize) -> Remap {
        Remap::sync(old, new, Some((held, |c| &mut c.volume)), |_, _| None, 8)
    }

    #[test]
    fn a_sync_carries_a_held_thumb_by_key_and_drops_one_whose_column_left() {
        let mut old = [col("a", 3), col("b", 15)];
        let mut new = [col("b", 5), col("c", 7)];
        let r = sync(&mut old, &mut new, 1);
        assert_eq!((r.get(0), r.get(1)), (None, Some(0)));
        assert_eq!(
            new[0].volume.pos, 15,
            "b's held thumb keeps its own position"
        );
        assert_eq!(new[1].volume.pos, 7);

        let mut old = [col("a", 3), col("b", 15)];
        let mut new = [col("b", 5), col("c", 7)];
        let r = sync(&mut old, &mut new, 0);
        assert_eq!(r.get(0), None, "a left");
        assert_eq!(
            (new[0].volume.pos, new[1].volume.pos),
            (5, 7),
            "the thumb held on a column that left moves nothing"
        );
    }
}
