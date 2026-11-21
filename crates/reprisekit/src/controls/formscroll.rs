//! A fixed form in a client smaller than it: its layout gives up what it can for the client, and
//! past that its `keep` items (the buttons) keep to the client's bottom-right corner while the
//! rest scrolls in a [`ScrollView`] over the client above the buttons' row. Its notes rewrap to
//! the width they get and grow, the view scrolling through what they push down. Its `head` items
//! (a prompt saying who asks) show whenever it is fitted, and the focused control's part shows
//! under them as far as the view holds both. The form is drawn at its origin (0, 0).

use std::time::Instant;

use crate::access::{Tree, accesskit::NodeId};
use crate::controls::form::{Form, View};
use crate::controls::scrollview::{self, Frame, Moved, ScrollView};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;

/// The gap at 100% between the scrolled part's bottom and the buttons' row.
const BAND_GAP: i32 = 7;

/// Lays a form out for a client of this size at 100% text from the rects it was built with,
/// giving up what it can, and returns the size the form then takes, its kept items placed for it.
pub type Lay = fn(&mut Form, (i32, i32)) -> (i32, i32);

#[derive(Clone, Debug)]
pub struct FormScroll {
    keep: &'static [&'static str],
    head: &'static [&'static str],
    lay: Lay,
    /// How far the kept items moved from where `lay` put them, at 100%.
    pinned: (i32, i32),
    /// The client in px at the form's text size.
    client: (i32, i32),
    scroll: ScrollView,
}

impl FormScroll {
    pub fn new(keep: &'static [&'static str], lay: Lay) -> FormScroll {
        FormScroll {
            keep,
            head: &[],
            lay,
            pinned: (0, 0),
            client: (0, 0),
            scroll: ScrollView::default(),
        }
    }

    /// The items fitting shows whole above the focused control's part: the scroll never leaves
    /// their top above the view's.
    pub fn head(mut self, head: &'static [&'static str]) -> FormScroll {
        self.head = head;
        self
    }

    /// Lays `form` out for `client` px at its last draw's text size and look, its notes wrapped
    /// as `text` draws them at `px`: `lay`, again beside the bar if the form still scrolls down,
    /// the kept items in the client's bottom-right corner. The scroll stays where it was as far
    /// as it still goes, then shows the focused control's part ([`Form::focus_part`]) below the
    /// head items; a tree's selected row comes up to its first row when that makes room.
    pub fn fit(&mut self, form: &mut Form, client: (i32, i32), text: &TextRenderer, px: u32) {
        let (left, top) = self.scroll.offset();
        self.moved(form, (-left, -top));
        self.scroll.across.top = 0;
        self.scroll.down.top = 0;
        self.client = client;
        let u = form.units();
        let room = (u.within(client.0), u.within(client.1));
        self.lay_in(form, room, false);
        self.flow(form, text, px);
        if self.frame(form).bars().0 {
            let bar = u.design(form.controls().scroll);
            self.lay_in(form, (room.0 - bar, room.1), true);
            self.flow(form, text, px);
        }
        let f = self.frame(form);
        let rect = match f.bars() {
            (false, false) => Rect::new(0, 0, room.0, room.1),
            _ => f.clip(),
        };
        form.view = Some(View {
            rect,
            keep: self.keep,
        });
        let by = (
            self.scroll.scroll_across(&f, left),
            self.scroll.scroll_to(&f, top),
        );
        self.moved(form, by);
        self.reveal_under_head(form);
        if !self.part_shows(form) && form.lift_focus_part() {
            self.reveal_under_head(form);
        }
    }

    /// [`FormScroll::reveal_focus`], then back as far as it takes for the head items' top-left
    /// to show.
    fn reveal_under_head(&mut self, form: &mut Form) {
        self.reveal_focus(form);
        let head = (form.items.iter())
            .filter(|it| self.head.contains(&it.id))
            .map(|it| form.flowed(it))
            .map(|r| (r.x, r.y))
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)));
        let Some((x, y)) = head else {
            return;
        };
        let f = self.frame(form);
        let (c, (left, top)) = (f.clip(), self.scroll.offset());
        let by = (
            self.scroll.scroll_across(&f, left.min(left + x - c.x)),
            self.scroll.scroll_to(&f, top.min(top + y - c.y)),
        );
        self.moved(form, by);
    }

    /// Whether the focused control's part shows whole in the view.
    fn part_shows(&self, form: &Form) -> bool {
        let clip = self.clip(form);
        form.focus_part_px().is_none_or(|r| clip.intersect(&r) == r)
    }

    /// Lays the form out in `room` with the kept items as `lay` puts them, then moves those to
    /// the client's corner: up by what the form is short of, left by what it is narrow of, and
    /// `beside` a bar out to the client's right edge.
    fn lay_in(&mut self, form: &mut Form, room: (i32, i32), beside: bool) {
        self.move_kept(form, (-self.pinned.0, -self.pinned.1));
        let size = (self.lay)(form, room);
        let client = form.units().within(self.client.0);
        let dx = match beside {
            true => client - size.0,
            false => (client - size.0).min(0),
        };
        self.pinned = (dx, (room.1 - size.1).min(0));
        self.move_kept(form, self.pinned);
    }

    /// Rewraps the form's notes for the room laid out, under a view so they grow.
    fn flow(&self, form: &mut Form, text: &TextRenderer, px: u32) {
        form.view = Some(View {
            rect: self.frame(form).inside,
            keep: self.keep,
        });
        form.lay_out_notes(text, 1, px);
    }

    /// Whether the form scrolls in the client it was last fitted to.
    pub fn scrolls(&self, form: &Form) -> bool {
        self.frame(form).bars() != (false, false)
    }

    /// How far the form is scrolled across and up, at 100%.
    pub fn offset(&self) -> (i32, i32) {
        self.scroll.offset()
    }

    /// The view's frame at 100%: the client above the buttons' row, over the form's controls
    /// but the kept ones as far as they reach, notes grown.
    fn frame(&self, form: &Form) -> Frame {
        let u = form.units();
        let room = (u.within(self.client.0), u.within(self.client.1));
        let kept = (form.items.iter()).filter(|it| self.keep.contains(&it.id));
        let bottom = (kept.map(|it| it.rect.y).min()).map_or(room.1, |y| (y - BAND_GAP).max(0));
        let (left, top) = self.scroll.offset();
        let reach = (form.items.iter())
            .filter(|it| !self.keep.contains(&it.id))
            .map(|it| form.flowed(it))
            .fold((0, 0), |(right, low), r| {
                (right.max(r.right() + left), low.max(r.bottom() + top))
            });
        Frame {
            inside: Rect::new(0, 0, room.0, bottom),
            reach,
            bar: u.design(form.controls().scroll),
            edge: 0,
        }
    }

    /// The view laid out in px at the form's text size.
    fn layout(&self, form: &Form) -> scrollview::Layout {
        let f = self.frame(form);
        let u = form.units();
        let inside = Rect::new(0, 0, self.client.0, u.len(f.inside.height));
        scrollview::layout(&f, inside, form.controls().scroll)
    }

    /// The bar down the view in px at the form's text size, while the form scrolls down.
    pub fn bar(&self, form: &Form) -> Option<Rect> {
        self.layout(form).bar
    }

    /// The bar along the view's bottom, while the form scrolls across.
    pub fn hbar(&self, form: &Form) -> Option<Rect> {
        self.layout(form).hbar
    }

    /// Where the scrolled controls show in px at the form's text size.
    pub fn clip(&self, form: &Form) -> Rect {
        self.layout(form).clip
    }

    pub fn draw(&self, c: &mut Canvas, p: &Paint, form: &Form) {
        (self.scroll).draw(c, p, &self.frame(form), &self.layout(form));
    }

    /// The form's `access` part under `parent`, its scrolled controls in the view's scroll view
    /// node while it scrolls. Returns the node that has the focus.
    pub fn access(
        &self,
        form: &Form,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> Option<NodeId> {
        let view = self.scrolls(form).then(|| {
            let (f, lay) = (self.frame(form), self.layout(form));
            self.scroll.access(&f, &lay, tree, parent, "form")
        });
        form.access_in(tree, parent, view, text, px)
    }

    /// A left press at (`x`, `y`) px: on a bar an arrow scrolls a line, the track a view and
    /// the thumb is grabbed. True when a bar took it.
    pub fn press(&mut self, form: &mut Form, x: i32, y: i32) -> bool {
        let f = self.frame(form);
        let thumb_min = form.controls().thumb_min;
        let Some(hit) = self.scroll.hit(&f, &self.layout(form), thumb_min, x, y) else {
            return false;
        };
        let by = self.scroll.press(&f, hit);
        self.moved(form, by);
        true
    }

    /// The pointer moved to (`x`, `y`): a held thumb drags the form, else the bars light the
    /// part under it. True when the form or a bar changed.
    pub fn motion(&mut self, form: &mut Form, x: i32, y: i32) -> bool {
        let (f, lay) = (self.frame(form), self.layout(form));
        let thumb_min = form.controls().thumb_min;
        if let Some(by) = self.scroll.drag(&f, &lay, thumb_min, x, y) {
            return self.moved(form, by);
        }
        let hit = self.scroll.hit(&f, &lay, thumb_min, x, y);
        self.scroll.motion(hit)
    }

    /// The left button let go: true when a thumb was held or a bar's look changed.
    pub fn release(&mut self) -> bool {
        self.scroll.release()
    }

    pub fn leave(&mut self) -> bool {
        self.scroll.leave()
    }

    /// Whether a bar holds the pointer: its thumb or an arrow is pressed.
    pub fn holding(&self) -> bool {
        self.scroll.holding()
    }

    /// Whether an arrow is held: the host ticks [`FormScroll::tick`] until the release.
    pub fn ticking(&self) -> bool {
        self.scroll.ticking()
    }

    /// A host tick at `now`: a held arrow steps once it repeats. True when the form moved.
    pub fn tick(&mut self, form: &mut Form, now: Instant) -> bool {
        let by = self.scroll.tick(&self.frame(form), now);
        self.moved(form, by)
    }

    /// The wheel turned `rows` rows (down positive) where the form's own controls took none.
    pub fn wheel(&mut self, form: &mut Form, rows: i32) -> bool {
        let by = self.scroll.wheel(&self.frame(form), rows);
        self.moved(form, by)
    }

    /// Scrolls the form so the focused control's part shows whole, as little as it takes, and
    /// not at all while it shows whole. A host calls it after a key that moved the focus or the
    /// part. True when the form moved.
    pub fn reveal_focus(&mut self, form: &mut Form) -> bool {
        let scrolled = |i: usize| !self.keep.contains(&form.items[i].id);
        if !(form.focus()).is_some_and(|i| scrolled(i) && self.scrolls(form)) {
            return false;
        }
        // A tree row's 100% rect rounds out from where it draws, and the rounding moves with the
        // scroll: the second reveal takes the px the first left.
        let mut moved = false;
        for _ in 0..2 {
            if self.part_shows(form) {
                break;
            }
            let Some(r) = form.focus_part() else { break };
            let by = self.scroll.reveal(&self.frame(form), r);
            moved |= self.moved(form, by);
        }
        moved
    }

    /// Scrolls the form as little as it takes for `r`, px at the form's text size, to show
    /// whole: a reader's ScrollIntoView on a control the view hides. True when the form moved.
    pub fn show(&mut self, form: &mut Form, r: Rect) -> bool {
        if !self.scrolls(form) {
            return false;
        }
        let r = form.units().covering(r);
        let by = self.scroll.reveal(&self.frame(form), r);
        self.moved(form, by)
    }

    /// The scrolled controls follow the view's offsets moving `by`. True when they moved.
    fn moved(&self, form: &mut Form, (dx, dy): Moved) -> bool {
        for it in (form.items.iter_mut()).filter(|it| !self.keep.contains(&it.id)) {
            it.rect.x -= dx;
            it.rect.y -= dy;
        }
        (dx, dy) != (0, 0)
    }

    fn move_kept(&self, form: &mut Form, (dx, dy): (i32, i32)) {
        for it in (form.items.iter_mut()).filter(|it| self.keep.contains(&it.id)) {
            it.rect.x += dx;
            it.rect.y += dy;
        }
    }
}

#[cfg(test)]
mod tests;
