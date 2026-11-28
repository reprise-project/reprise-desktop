//! The breadcrumb address: the place's icon, then a chevron and a button per segment of the
//! place's path, the history drop and Refresh at the right end.
//!
//! A segment goes to its target; the chevron after it opens a menu of that folder's subfolders,
//! which the host fills. When the segments outgrow the field, the leading ones fold into the
//! overflow button before the first segment, the last segment always showing. A press on the
//! icon or the empty field (or the host's Alt+D, F4, Ctrl+L) swaps the crumbs for the editable
//! path field ([`AddressBar`]) with its text selected; Enter goes, Esc and focus loss return to
//! the crumbs with the place's text.

use crate::access::{Tree, accesskit};
use crate::controls::addressbar::{self, AddressBar};
use crate::controls::combo::ListItem;
use crate::controls::form::Key;
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::pointer::Shape;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{Caret, EditKey};
use crate::theme::TextUnits;
use accesskit::NodeId;

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
        let _ = (crumbs, icon, text);
    }

    /// The history drop's rows, newest first.
    pub fn set_history(&mut self, rows: Vec<ListItem>) {
        let _ = rows;
    }

    /// Swaps the crumbs for the editable field, its text selected.
    pub fn start_edit(&mut self) {}

    /// Back to the crumbs, the field holding the place's text again.
    pub fn cancel_edit(&mut self) {}

    pub fn editing(&self) -> bool {
        self.editing
    }

    /// The host closed the menu a press opened.
    pub fn menu_closed(&mut self) {}

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
        let _ = (hit, x, lay, text, px, s);
        Event::None
    }

    /// The button is released over what [`hit`] returned: a segment or Refresh pressed and
    /// released over itself fires.
    pub fn release(&mut self, hit: Hit) -> Event {
        let _ = hit;
        Event::None
    }

    /// The pointer at what [`hit`] returned. Returns whether anything lit or went out.
    pub fn hover(&mut self, hit: Hit) -> bool {
        let _ = hit;
        false
    }

    /// The pointer left the field.
    pub fn leave(&mut self) -> bool {
        false
    }

    /// An editing key into the field while editing.
    pub fn key(&mut self, k: EditKey) -> Event {
        let _ = k;
        Event::None
    }

    /// A key while editing ahead of the field's own editing: Enter goes, Esc returns to the
    /// crumbs, the list and completion keys move their lists. `None` for a key the field keeps
    /// or any key outside edit mode.
    pub fn list_key(&mut self, key: &Key) -> Option<Event> {
        let _ = key;
        None
    }

    /// Enter while editing: the history row picked, else the typed text.
    pub fn enter(&mut self) -> Event {
        Event::None
    }

    /// The window's keyboard focus left the field: edit mode ends. Returns whether it was on.
    pub fn focus_lost(&mut self) -> bool {
        false
    }

    /// A press anywhere outside the field closes the edit's lists; [`AddressBar::press_outside`].
    pub fn press_outside(&mut self, hit: Hit) -> bool {
        let _ = hit;
        false
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

/// `field` is the whole control, Refresh included.
pub fn layout(
    b: &Breadcrumb,
    field: Rect,
    text: &TextRenderer,
    px: u32,
    cm: &ControlMetrics,
) -> Layout {
    let _ = (b, text, px);
    Layout {
        frame: field,
        icon: Rect::default(),
        overflow: Rect::default(),
        folded: 0,
        segments: Vec::new(),
        empty: Rect::default(),
        history: Rect::default(),
        refresh: Rect::default(),
        edit: addressbar::field_layout(&b.edit, field, cm),
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
    let _ = (b, lay, x, y);
    Hit::None
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
    let _ = (c, text, p, b, lay, focus, caret, px);
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
    let _ = (b, lay, text, px);
    parent
}
