//! The breadcrumb address: the place's icon, then a chevron and a button per segment of the
//! place's path, the history drop and Refresh at the right end.
//!
//! A segment goes to its target; the chevron after it opens a menu of that folder's subfolders,
//! which the host fills. When the segments outgrow the field, the leading ones fold into the
//! overflow button before the first segment, the last segment always showing. A press on the
//! icon or the empty field (or the host's Alt+D, F4, Ctrl+L) swaps the crumbs for the editable
//! path field ([`AddressBar`]) with its text selected; Enter goes, Esc and focus loss return to
//! the crumbs with the place's text.

use crate::access::{self, Tree, accesskit};
use crate::controls::addressbar::{self, AddressBar};
use crate::controls::combo::{self, ListItem};
use crate::controls::form::Key;
use crate::controls::metrics::ControlMetrics;
use crate::controls::pathcomplete;
use crate::look::Paint;
use crate::look::widgets::{draw_edit_field, toolbar};
use crate::pointer::{Interaction, Shape};
use crate::raster::{Canvas, Rect};
use crate::shape::{self, Glyph, premul};
use crate::style::{Part as StylePart, State};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey};
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

/// The field's height at 100% text, Refresh's frame included.
pub const HEIGHT: i32 = 22;

/// The field's height at text `u`.
pub fn height(u: TextUnits) -> i32 {
    u.len(HEIGHT)
}

/// One segment: what it reads and the place text the field goes to when it is clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub target: String,
}

/// A part the pointer lights or presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Segment(usize),
    Chevron(usize),
    Overflow,
    History,
    Refresh,
}

#[derive(Clone, Debug)]
pub struct Breadcrumb {
    pub crumbs: Vec<Crumb>,
    /// The editable field edit mode shows, its drop list the typed-path history.
    pub edit: AddressBar,
    editing: bool,
    /// The place's text, which the field takes back when an edit is cancelled.
    place_text: String,
    pub hot: Option<Part>,
    pub pressed: Option<Part>,
    /// The part whose menu is open, drawn pressed until [`Breadcrumb::menu_closed`].
    pub open_menu: Option<Part>,
    /// The pointer is over the field, which brightens it.
    pub field_hot: bool,
}

impl Default for Breadcrumb {
    fn default() -> Self {
        Self::new()
    }
}

impl Breadcrumb {
    pub fn new() -> Breadcrumb {
        Breadcrumb {
            crumbs: Vec::new(),
            edit: AddressBar::new(Vec::new(), 0, None),
            editing: false,
            place_text: String::new(),
            hot: None,
            pressed: None,
            open_menu: None,
            field_hot: false,
        }
    }

    /// A new place: its segments, its icon and the text edit mode starts from. Leaves edit
    /// mode and closes any menu.
    pub fn set_place(&mut self, crumbs: Vec<Crumb>, icon: Option<Canvas>, text: &str) {
        self.crumbs = crumbs;
        self.place_text = text.to_string();
        self.edit.set_place(text, icon);
        self.editing = false;
        self.edit.combo.open = None;
        (self.hot, self.pressed, self.open_menu) = (None, None, None);
    }

    /// The place's icon, which the crumbs and the edit both show.
    pub fn icon(&self) -> Option<&Canvas> {
        self.edit.combo.icon.as_ref()
    }

    /// The history drop's rows, newest first.
    pub fn set_history(&mut self, rows: Vec<ListItem>) {
        let n = rows.len().max(1);
        self.edit.combo.items = rows;
        self.edit.combo.rows = n;
        self.edit.combo.open = None;
    }

    /// Swaps the crumbs for the editable field, its text selected.
    pub fn start_edit(&mut self) {
        self.editing = true;
        (self.hot, self.pressed, self.open_menu) = (None, None, None);
        if let Some(edit) = self.edit.combo.edit.as_mut() {
            edit.select_all();
        }
    }

    /// Back to the crumbs, the field holding the place's text again.
    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit.combo.open = None;
        let icon = self.edit.combo.icon.take();
        let text = self.place_text.clone();
        self.edit.set_place(&text, icon);
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    /// The host closed the menu a press opened.
    pub fn menu_closed(&mut self) {
        self.open_menu = None;
    }

    /// A press on what [`hit`] returned. `x` is the pointer's, for the caret; `px` and `s`
    /// are the ones the field was drawn with.
    pub fn press(
        &mut self,
        hit: Hit,
        x: i32,
        lay: &Layout,
        text: &TextRenderer,
        px: u32,
        s: i32,
    ) -> Event {
        match hit {
            Hit::Icon | Hit::Empty => {
                self.start_edit();
                Event::Redraw
            }
            Hit::Segment(i) => {
                self.pressed = Some(Part::Segment(i));
                Event::Redraw
            }
            Hit::Refresh => {
                self.pressed = Some(Part::Refresh);
                Event::Redraw
            }
            Hit::Chevron(i) => {
                self.open_menu = Some(Part::Chevron(i));
                Event::OpenChevron(i)
            }
            Hit::Overflow => {
                self.open_menu = Some(Part::Overflow);
                Event::OpenOverflow
            }
            Hit::History => {
                if !self.editing {
                    self.start_edit();
                }
                self.edit.complete.close();
                if !self.edit.combo.items.is_empty() {
                    self.edit.combo.toggle();
                }
                Event::OpenHistory
            }
            Hit::Edit(h) => self.edit_event(|ab| ab.click(h, x, &lay.edit, text, px, s)),
            Hit::None => Event::None,
        }
    }

    /// The button is released over what [`hit`] returned: a segment or Refresh pressed and
    /// released over itself fires.
    pub fn release(&mut self, hit: Hit) -> Event {
        let Some(pressed) = self.pressed.take() else {
            return Event::None;
        };
        match (pressed, hit) {
            (Part::Segment(i), Hit::Segment(j)) if i == j => self
                .crumbs
                .get(i)
                .map_or(Event::Redraw, |c| Event::Go(c.target.clone())),
            (Part::Refresh, Hit::Refresh) => Event::Refresh,
            _ => Event::Redraw,
        }
    }

    /// The pointer at what [`hit`] returned. Returns whether anything lit or went out.
    pub fn hover(&mut self, hit: Hit) -> bool {
        let hot = match hit {
            Hit::Segment(i) => Some(Part::Segment(i)),
            Hit::Chevron(i) => Some(Part::Chevron(i)),
            Hit::Overflow => Some(Part::Overflow),
            Hit::History => Some(Part::History),
            Hit::Refresh => Some(Part::Refresh),
            _ => None,
        };
        let field_hot = hit != Hit::None;
        let listed = match hit {
            Hit::Edit(h) => self.edit.hover(h),
            _ => false,
        };
        let moved = hot != self.hot || field_hot != self.field_hot;
        (self.hot, self.field_hot) = (hot, field_hot);
        moved || listed
    }

    /// The pointer left the field.
    pub fn leave(&mut self) -> bool {
        let lit = self.hot.is_some() || self.field_hot;
        (self.hot, self.field_hot) = (None, false);
        lit
    }

    /// An editing key into the field while editing.
    pub fn key(&mut self, k: EditKey) -> Event {
        if !self.editing {
            return Event::None;
        }
        self.edit_event(|ab| ab.key(k))
    }

    /// A key while editing ahead of the field's own editing: Enter goes, Esc closes an open list
    /// or returns to the crumbs, the list and completion keys move their lists. `None` for a key
    /// the field keeps or any key outside edit mode.
    pub fn list_key(&mut self, key: &Key) -> Option<Event> {
        if !self.editing {
            return None;
        }
        match key {
            Key::Enter => Some(self.enter()),
            Key::Escape if self.edit.combo.open.is_some() => {
                self.edit.combo.open = None;
                Some(Event::Redraw)
            }
            Key::Escape if self.edit.complete.is_open() => {
                self.edit.complete.close();
                Some(Event::Redraw)
            }
            Key::Escape => {
                self.cancel_edit();
                Some(Event::Redraw)
            }
            _ => {
                let e = self.edit.list_key(key)?;
                Some(self.edit_event(|_| e))
            }
        }
    }

    /// Enter while editing: the history row picked, else the typed text.
    pub fn enter(&mut self) -> Event {
        if !self.editing {
            return Event::None;
        }
        self.edit_event(AddressBar::enter)
    }

    /// The window's keyboard focus left the field: edit mode ends. Returns whether it was on.
    pub fn focus_lost(&mut self) -> bool {
        let was = self.editing;
        if was {
            self.edit.complete.close();
            self.cancel_edit();
        }
        was
    }

    /// A press anywhere outside the field's lists while editing; [`AddressBar::press_outside`].
    /// Returns whether the press goes no further.
    pub fn press_outside(&mut self, hit: Hit) -> bool {
        match hit {
            Hit::Edit(h) => self.edit.press_outside(h),
            _ => self.edit.press_outside(addressbar::Hit::Nothing),
        }
    }

    /// Runs `f` on the edit and words its event as the breadcrumb's. Going to typed text
    /// returns to the crumbs, which show the place until the host's [`Breadcrumb::set_place`].
    fn edit_event(&mut self, f: impl FnOnce(&mut AddressBar) -> addressbar::Event) -> Event {
        match f(&mut self.edit) {
            addressbar::Event::None => Event::None,
            addressbar::Event::Redraw => Event::Redraw,
            addressbar::Event::Navigate(t) => {
                self.edit.complete.close();
                self.cancel_edit();
                Event::Go(t)
            }
            addressbar::Event::Pick(i) => Event::Pick(i),
        }
    }
}

/// A segment's button and the chevron after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Index into [`Breadcrumb::crumbs`].
    pub crumb: usize,
    pub button: Rect,
    pub chevron: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The field's frame, Refresh outside it.
    pub frame: Rect,
    pub icon: Rect,
    /// The chevron before the first shown segment, « while segments are folded.
    pub overflow: Rect,
    /// How many leading crumbs fold into [`Layout::overflow`].
    pub folded: usize,
    pub segments: Vec<Segment>,
    /// The field between the last chevron and the history drop.
    pub empty: Rect,
    pub history: Rect,
    pub refresh: Rect,
    /// The editable field, laid over the frame.
    pub edit: addressbar::Layout,
}

/// `field` is the whole control, Refresh included; `px` the font's size at scale 1. The
/// segments that fit show from the last one back; the last one always shows, cut short when
/// it alone outgrows the field.
pub fn layout(
    b: &Breadcrumb,
    field: Rect,
    text: &TextRenderer,
    px: u32,
    cm: &ControlMetrics,
) -> Layout {
    let u = cm.text;
    let refresh_w = u.len(REFRESH_W);
    let refresh = Rect::new(field.right() - refresh_w, field.y, refresh_w, field.height);
    // The frame and Refresh share the column between them.
    let frame = Rect::new(field.x, field.y, refresh.x + 1 - field.x, field.height);
    let (y, h) = (field.y + 1, (field.height - 2).max(0));
    let history_w = u.len(HISTORY_W);
    let history = Rect::new(refresh.x + 1 - history_w, y, history_w, h);
    let side = combo::field_icon_px(u, field.height);
    let icon = Rect::new(
        field.x + u.len(ICON_X),
        field.y + (field.height - side) / 2,
        side,
        side,
    );
    let chevron_w = u.len(CHEVRON_W);
    let overflow = Rect::new(icon.right() + u.len(ICON_GAP), y, chevron_w, h);
    let pad = u.len(SEG_PAD);
    let widths: Vec<i32> = (b.crumbs.iter())
        .map(|c| text.measure(&c.label, false, px) + 2 * pad)
        .collect();
    // A segment's chevron starts on its button's last column.
    let room = history.x - overflow.right();
    let mut used = 0;
    let shown = (widths.iter().rev())
        .take_while(|&&w| {
            let fits = used == 0 || used + w + chevron_w - 1 <= room;
            used += w + chevron_w - 1;
            fits
        })
        .count();
    let folded = b.crumbs.len() - shown;
    let mut x = overflow.right();
    let segments = (folded..b.crumbs.len())
        .map(|i| {
            let w = if shown == 1 {
                widths[i].min(room - chevron_w + 1).max(2 * pad)
            } else {
                widths[i]
            };
            let button = Rect::new(x, y, w, h);
            let chevron = Rect::new(button.right() - 1, y, chevron_w, h);
            x = chevron.right();
            Segment {
                crumb: i,
                button,
                chevron,
            }
        })
        .collect();
    Layout {
        frame,
        icon,
        overflow,
        folded,
        segments,
        empty: Rect::new(x, y, (history.x - x).max(0), h),
        history,
        refresh,
        edit: addressbar::field_layout(&b.edit, frame, cm),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Icon,
    Segment(usize),
    Chevron(usize),
    Overflow,
    Empty,
    History,
    Refresh,
    /// The editable field or its lists while editing.
    Edit(addressbar::Hit),
    None,
}

impl Hit {
    pub fn shape(self, b: &Breadcrumb) -> Shape {
        match self {
            Hit::Edit(h) => h.shape(&b.edit),
            _ => Shape::Default,
        }
    }
}

pub fn hit(b: &Breadcrumb, lay: &Layout, x: i32, y: i32) -> Hit {
    if lay.refresh.contains(x, y) && !lay.frame.contains(x, y) {
        return Hit::Refresh;
    }
    if b.editing {
        if lay.edit.combo.drop.contains(x, y) {
            return Hit::History;
        }
        return match addressbar::hit(&b.edit, &lay.edit, x, y) {
            addressbar::Hit::Nothing => Hit::None,
            h => Hit::Edit(h),
        };
    }
    if !lay.frame.contains(x, y) {
        return Hit::None;
    }
    if lay.history.contains(x, y) {
        return Hit::History;
    }
    if x < lay.overflow.x {
        return Hit::Icon;
    }
    if lay.overflow.contains(x, y) {
        return Hit::Overflow;
    }
    for seg in &lay.segments {
        // The chevron wins the column it shares with its button.
        if seg.chevron.contains(x, y) {
            return Hit::Chevron(seg.crumb);
        }
        if seg.button.contains(x, y) {
            return Hit::Segment(seg.crumb);
        }
    }
    Hit::Empty
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    /// Go to this place text: a segment's target or the typed path.
    Go(String),
    /// History row `i` picked from the drop list.
    Pick(usize),
    /// The chevron after crumb `i` opened: the host fills its menu with that folder's
    /// subfolders.
    OpenChevron(usize),
    /// The overflow button opened: the host lists [`Layout::folded`]'s crumbs.
    OpenOverflow,
    /// The history drop opened in edit mode.
    OpenHistory,
    Refresh,
}

/// The field's closed part. The edit's lists are [`draw_list`], drawn last by the caller.
/// `focus` and `caret` are the edit's while editing.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    b: &Breadcrumb,
    lay: &Layout,
    focus: bool,
    caret: Caret,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let small = p.text.small();
    let ink = cs.window_text;
    let face = if b.editing {
        State::Focused
    } else if b.field_hot {
        State::Hot
    } else {
        State::Normal
    };
    draw_frame(c, p, lay.refresh, face);
    if let Some(state) = lit(b, Part::Refresh) {
        draw_box(c, p, inner(lay.refresh), state);
    }
    let refresh = shape::glyph(
        Glyph::Refresh,
        REFRESH_GLYPH * small,
        s,
        REFRESH_WEIGHT,
        premul(cs.hot_light, 255),
    );
    blit_centred(c, &refresh, lay.refresh.scaled(s));
    if b.editing {
        addressbar::draw_field(c, text, p, &b.edit, &lay.edit, focus, caret, px);
        return;
    }
    draw_frame(c, p, lay.frame, face);
    if let Some(icon) = b.icon() {
        blit_centred(c, icon, lay.icon.scaled(s));
    }
    let arrow = |c: &mut Canvas, g: Glyph, r: Rect| {
        let art = shape::glyph(g, GLYPH * small, s, 1.0, premul(ink, 255));
        blit_centred(c, &art, r.scaled(s));
    };
    if let Some(state) = lit(b, Part::Overflow) {
        draw_box(c, p, lay.overflow, state);
    }
    let lead = match (lay.folded, b.open_menu == Some(Part::Overflow)) {
        (0, false) => Glyph::ArrowRight,
        (0, true) => Glyph::ArrowDown,
        _ => Glyph::ChevronsLeft,
    };
    arrow(c, lead, lay.overflow);
    let pad = p.text.len(SEG_PAD);
    for seg in &lay.segments {
        if let Some(state) = lit(b, Part::Segment(seg.crumb)) {
            draw_box(c, p, seg.button, state);
        }
        if let Some(state) = lit(b, Part::Chevron(seg.crumb)) {
            draw_box(c, p, seg.chevron, state);
        }
        let label = &b.crumbs[seg.crumb].label;
        let room = Rect::new(
            seg.button.x + pad,
            seg.button.y,
            seg.button.width - 2 * pad,
            seg.button.height,
        );
        let shown = text.ellipsize(label, room.width * s, false, px);
        let r = room.scaled(s);
        text.draw(c, r, &shown, false, ink, Align::Left, px);
        let open = b.open_menu == Some(Part::Chevron(seg.crumb));
        let g = if open {
            Glyph::ArrowDown
        } else {
            Glyph::ArrowRight
        };
        arrow(c, g, seg.chevron);
    }
    if let Some(state) = lit(b, Part::History) {
        draw_box(c, p, lay.history, state);
    }
    arrow(c, Glyph::ArrowDown, lay.history);
}

/// How the box behind `part` draws, if at all: a segment and its chevron light together, the
/// one under the pointer blue and its partner grey when the chevron is the one lit.
fn lit(b: &Breadcrumb, part: Part) -> Option<State> {
    let partner = match part {
        Part::Segment(i) => Some(Part::Chevron(i)),
        Part::Chevron(i) => Some(Part::Segment(i)),
        _ => None,
    };
    let held = |p: Option<Part>| p.is_some_and(|p| b.pressed == Some(p) || b.open_menu == Some(p));
    if held(Some(part)) || held(partner) {
        return Some(State::Pressed);
    }
    match (b.hot == Some(part), b.hot.is_some() && b.hot == partner) {
        (true, _) => Some(State::Hot),
        (false, true) if matches!(part, Part::Segment(_)) => Some(State::Checked),
        (false, true) => Some(State::Hot),
        _ => None,
    }
}

fn has(p: &Paint, part: StylePart) -> bool {
    p.look.style().is_some_and(|st| st.part(part).is_some())
}

/// The field's frame over `r`, the look's sunken field in a look without one.
fn draw_frame(c: &mut Canvas, p: &Paint, r: Rect, state: State) {
    let r = r.scaled(p.s);
    match p.look.style().filter(|_| has(p, StylePart::CrumbField)) {
        Some(st) => st.draw(c, r, StylePart::CrumbField, state, p.s),
        None => {
            let ptr = match state {
                State::Hot => Interaction::Hot,
                _ => Interaction::Idle,
            };
            draw_edit_field(c, p, r, p.cs.window, state == State::Focused, ptr);
        }
    }
}

/// A lit box over `r`, a toolbar button's face in a look without one.
fn draw_box(c: &mut Canvas, p: &Paint, r: Rect, state: State) {
    let r = r.scaled(p.s);
    match p.look.style().filter(|_| has(p, StylePart::CrumbButton)) {
        Some(st) => st.draw(c, r, StylePart::CrumbButton, state, p.s),
        None => {
            let state = if state == State::Checked {
                State::Hot
            } else {
                state
            };
            toolbar::draw_button(c, p, r, None, state);
        }
    }
}

/// `r` inside the field's top and bottom lines.
fn inner(r: Rect) -> Rect {
    Rect::new(r.x, r.y + 1, r.width, (r.height - 2).max(0))
}

/// `art` centred in `r` (device px), an odd px left over going below.
fn blit_centred(c: &mut Canvas, art: &Canvas, r: Rect) {
    let (w, h) = (art.width as i32, art.height as i32);
    c.blit_over(r.x + (r.width - w) / 2, r.y + (r.height - h + 1) / 2, art);
}

/// The edit's completion and history lists.
pub fn draw_list(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    b: &Breadcrumb,
    lay: &Layout,
    px: u32,
) {
    addressbar::draw_list(c, text, p, &b.edit, &lay.edit, px);
}

/// A toolbar of a button per shown segment and its chevron, the overflow, history and
/// Refresh buttons; while editing, the edit as [`addressbar::access`] gives it. Returns the
/// node that has the focus when the field does.
pub fn access(
    b: &Breadcrumb,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let all = lay.frame.union(&lay.refresh);
    let bar = tree.push(parent, "address", access::node(Role::Toolbar, all));
    let button = |tree: &mut Tree, key: (&str, usize), r: Rect, name: &str| {
        tree.push(bar, key, access::button(r, name, true, false));
    };
    let focus = if b.editing {
        let focus = combo::access(&b.edit.combo, &lay.edit.combo, text, px, tree, bar, "combo");
        let combo = Tree::id_of(bar, "combo");
        if let Some(n) = tree.get_mut(Tree::id_of(combo, "edit")) {
            n.set_label(addressbar::address());
        }
        pathcomplete::access(&b.edit.complete, &lay.edit.complete, tree, combo);
        focus
    } else {
        button(
            tree,
            ("overflow", 0),
            lay.overflow,
            &tr!("breadcrumb-overflow"),
        );
        for seg in &lay.segments {
            let name = &b.crumbs[seg.crumb].label;
            button(tree, ("segment", seg.crumb), seg.button, name);
            let chevron = tr!("breadcrumb-chevron", name = name.as_str());
            button(tree, ("chevron", seg.crumb), seg.chevron, &chevron);
        }
        button(
            tree,
            ("history", 0),
            lay.history,
            &tr!("breadcrumb-history"),
        );
        bar
    };
    button(
        tree,
        ("refresh", 0),
        lay.refresh,
        &tr!("breadcrumb-refresh"),
    );
    focus
}

/// Sizes at 100% text, each grown with it.
const ICON_X: i32 = 3;
const ICON_GAP: i32 = 2;
/// A chevron's box and the overflow's, both edges included.
const CHEVRON_W: i32 = 15;
/// A segment's edge and the room between it and the label, each side.
const SEG_PAD: i32 = 5;
/// The history drop's box, its right edge Refresh's left.
const HISTORY_W: i32 = 20;
/// Refresh, both edges included.
const REFRESH_W: i32 = 25;
/// The arrow glyphs' box, and the refresh arrow's and its stroke, grown by whole steps.
const GLYPH: i32 = 9;
const REFRESH_GLYPH: i32 = 13;
const REFRESH_WEIGHT: f64 = 2.0;

#[cfg(test)]
#[path = "breadcrumb/tests.rs"]
mod tests;
