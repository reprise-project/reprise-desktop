//! The tray's Customize Notifications dialog: a text, the tray items in two
//! groups (registered now, then past ones that still carry a choice), each
//! with its icon (16 px at 100% text), name and behaviour, Restore Defaults, OK and Cancel.
//! The selected row's behaviour cell turns into a combo box. The host lists
//! the items and acts on [`Event`]; nothing here knows the tray.
//!
//! Classic's client area is 348x391 with a 330x208 list; Glass lays it out as
//! a 1184x602 settings page with a 1000x403 list. Sizes are in the layout
//! consts in `geometry`, at 100% text; both grow with the text size. A page wider or
//! taller than the room its host has ([`TrayCustomize::fit`]) narrows its
//! list and wraps its text, then stacks each row's combo under its name, and
//! scrolls above the button band; Classic's list grows shorter.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::classic::widgets::BarHit;
use crate::controls::combo::{self, Combo};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::{PageScroll, ThumbDrag};
use crate::icon::IconSource;
use crate::pointer::Pointer;
use crate::theme::Theme;
use geometry::{CLASSIC, GLASS, Geometry};

mod access_tree;
mod draw;
mod geometry;
mod input;
mod layout;
pub use layout::Layout;

/// What one tray item does with its icon and its balloons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Behaviour {
    /// Follow the item's status: behind the ^ cell while it is Passive,
    /// on the bar otherwise. Every item starts here.
    #[default]
    Auto,
    /// Behind the ^ cell whatever its status; its balloons still show.
    Hide,
    /// On the bar whatever its status.
    Show,
    /// Behind the ^ cell, and its balloons do not show.
    Mute,
}

impl Behaviour {
    /// Whether an item whose status is `passive` waits behind the ^ cell.
    pub fn hidden(self, passive: bool) -> bool {
        match self {
            Behaviour::Auto => passive,
            Behaviour::Show => false,
            Behaviour::Hide | Behaviour::Mute => true,
        }
    }

    /// Whether the item's balloons show.
    pub fn balloons(self) -> bool {
        self != Behaviour::Mute
    }
}

/// Which interface's words the dialog speaks: Classic's (and Blue's) or
/// Glass's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording {
    Classic,
    Glass,
}

impl Wording {
    /// The words for `theme`'s look.
    pub fn of(theme: &Theme) -> Wording {
        if theme.glass().is_some() {
            Wording::Glass
        } else {
            Wording::Classic
        }
    }

    /// The behaviour combo's rows, in order.
    pub fn choices(self) -> &'static [Behaviour] {
        match self {
            Wording::Classic => &[Behaviour::Auto, Behaviour::Hide, Behaviour::Show],
            Wording::Glass => &[Behaviour::Show, Behaviour::Mute, Behaviour::Hide],
        }
    }

    /// The combo row that stands for `b` on an item whose status is
    /// `passive`: Classic has no row of its own for Mute and shows it as
    /// Always hide; Glass has none for Auto and shows what the status
    /// does now.
    pub fn index(self, b: Behaviour, passive: bool) -> usize {
        let shown = match (self, b) {
            (Wording::Classic, Behaviour::Mute) => Behaviour::Hide,
            (Wording::Glass, Behaviour::Auto) if passive => Behaviour::Hide,
            (Wording::Glass, Behaviour::Auto) => Behaviour::Show,
            (_, b) => b,
        };
        self.choices().iter().position(|&c| c == shown).unwrap_or(0)
    }

    /// The label of combo row `b`.
    pub fn label(self, b: Behaviour) -> &'static str {
        match (self, b) {
            (Wording::Classic, Behaviour::Auto) => "Hide when inactive",
            (Wording::Classic, Behaviour::Hide | Behaviour::Mute) => "Always hide",
            (Wording::Classic, Behaviour::Show) => "Always show",
            (Wording::Glass, Behaviour::Show | Behaviour::Auto) => "Show icon and notifications",
            (Wording::Glass, Behaviour::Mute) => "Hide icon and notifications",
            (Wording::Glass, Behaviour::Hide) => "Only show notifications",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Wording::Classic => "Customize Notifications",
            Wording::Glass => "Notification Area Icons",
        }
    }

    fn intro(self) -> &'static str {
        match self {
            Wording::Classic => {
                "Reprise keeps the icons of busy and urgent items on the taskbar and \
                 keeps the idle ones behind the arrow. Pick another behavior for any item below."
            }
            Wording::Glass => {
                "A hidden icon and its notifications won't tell you about changes. To \
                 see hidden icons at any time, click the arrow beside the notification area."
            }
        }
    }

    /// Glass's page heading.
    const HEADING: &'static str = "Choose which icons and notifications appear on the taskbar";

    fn prompt(self) -> &'static str {
        match self {
            Wording::Classic => "Select an item, then choose how it behaves:",
            Wording::Glass => "Select an item, then choose its behavior:",
        }
    }

    fn columns(self) -> [&'static str; 2] {
        match self {
            Wording::Classic => ["Name", "Behavior"],
            Wording::Glass => ["Icons", "Behaviors"],
        }
    }

    fn restore(self) -> &'static str {
        match self {
            Wording::Classic => "&Restore Defaults",
            Wording::Glass => "Restore default icon behaviors",
        }
    }

    fn geometry(self) -> &'static Geometry {
        match self {
            Wording::Classic => &CLASSIC,
            Wording::Glass => &GLASS,
        }
    }
}

/// The Glass check box's label.
pub const ALL_ICONS: &str = "&Always show all icons and notifications on the taskbar";
pub const CURRENT: &str = "Current Items";
pub const PAST: &str = "Past Items";

/// One tray item on the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The item's stable identity, what its choice is kept under.
    pub key: String,
    pub name: String,
    /// What its tip says; Glass draws it under the name.
    pub detail: String,
    /// Its icon at the sizes it came in; empty draws none.
    pub icon: Arc<[IconSource]>,
    /// Its status is Passive now; a past item's is false.
    pub passive: bool,
    pub behaviour: Behaviour,
}

/// The part holding the keyboard focus, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    /// The selected row's behaviour combo.
    Combo,
    AllIcons,
    Restore,
    Ok,
    Cancel,
}

const TAB_ORDER: [Focus; 6] = [
    Focus::List,
    Focus::Combo,
    Focus::AllIcons,
    Focus::Restore,
    Focus::Ok,
    Focus::Cancel,
];

/// A slot of the list: a group's header (0 current, 1 past) or an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Header(usize),
    Row(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    /// The list past its rows, or a group's header.
    List,
    Combo(combo::Hit),
    Bar(BarHit),
    /// The page's own scrollbar, while the page scrolls.
    PageBar(BarHit),
    AllIcons,
    Restore,
    Ok,
    Cancel,
}

/// What OK hands the host: every listed item's behaviour by key, and the
/// check box when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choices {
    pub behaviours: Vec<(String, Behaviour)>,
    pub all_icons: Option<bool>,
}

/// What the host does after an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Accept(Choices),
    Cancel,
}

fn redraw_if(changed: bool) -> Event {
    if changed { Event::Redraw } else { Event::None }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrayCustomize {
    pub wording: Wording,
    /// The current items, then the past ones.
    pub entries: Vec<Entry>,
    /// How many of `entries` are current.
    pub current: usize,
    pub selected: Option<usize>,
    /// The first slot in view.
    pub top: usize,
    /// The Glass check box, ticked when tray overflow is off; None in
    /// Classic, whose switch is the settings sheet's.
    pub all_icons: Option<bool>,
    pub focus: Focus,
    /// The selected row's behaviour combo.
    pub combo: Combo,
    /// The look's control sizes, which the list's header height and the
    /// combo come from; the host sets them again when the look changes.
    pub metrics: ControlMetrics,
    /// The push button, box or link the pointer holds or rests on.
    pointer: Pointer<Focus>,
    /// The page row whose combo the pointer rests on (Glass).
    hot_row: Option<usize>,
    /// The list's thumb, and the open combo list's, while dragged.
    thumb: ThumbDrag,
    combo_thumb: ThumbDrag,
    /// The most client the host has room for, in px at the text size; `None` has room for all.
    /// [`TrayCustomize::fit`] lays the page out in it; Classic's list shortens to it.
    pub room: Option<(i32, i32)>,
    /// The intro as [`TrayCustomize::fit`] wrapped it; empty before. The page draws an empty
    /// one on one line; Classic's grows past three lines by the rest.
    intro: Vec<String>,
    /// The page's heading as [`TrayCustomize::fit`] wrapped it; empty when it fits one line.
    heading: Vec<String>,
    /// Restore Defaults' width at 100%, as wide as [`TrayCustomize::fit`] found its label
    /// needs; `None` before.
    restore_w: Option<i32>,
    /// The page's scroll above its band, in a client shorter than the page.
    page_scroll: PageScroll,
}

impl TrayCustomize {
    /// The dialog over `current` then `past` items, nothing selected, the
    /// focus on the list; Glass's page puts it on the first row's combo.
    pub fn new(
        wording: Wording,
        current: Vec<Entry>,
        past: Vec<Entry>,
        all_icons: Option<bool>,
        metrics: ControlMetrics,
    ) -> TrayCustomize {
        let n = current.len();
        let mut entries = current;
        entries.extend(past);
        let mut t = TrayCustomize {
            wording,
            entries,
            current: n,
            selected: None,
            top: 0,
            all_icons,
            focus: Focus::List,
            combo: Combo::new(&[], 0, 1),
            metrics,
            pointer: Pointer::new(),
            hot_row: None,
            thumb: ThumbDrag::default(),
            combo_thumb: ThumbDrag::default(),
            room: None,
            intro: Vec::new(),
            heading: Vec::new(),
            restore_w: None,
            page_scroll: PageScroll::default(),
        };
        if t.is_page() {
            t.focus = Focus::Combo;
            t.select(0);
            t.refocus();
        }
        t
    }

    /// Speak `w`'s words in its layout, as the look changes under the
    /// open dialog: the list back at its top, the combo closed, and the
    /// focus where a new dialog in that look puts it.
    pub fn set_wording(&mut self, w: Wording) {
        if w == self.wording {
            return;
        }
        self.wording = w;
        self.top = 0;
        self.page_scroll = PageScroll::default();
        self.intro.clear();
        self.heading.clear();
        self.hot_row = None;
        self.focus = if self.is_page() {
            Focus::Combo
        } else {
            Focus::List
        };
        match self.selected {
            Some(i) => self.select(i),
            None if self.is_page() => self.select(0),
            None => {}
        }
        self.refocus();
    }

    /// Whether the dialog is laid out as Glass's settings page.
    pub fn is_page(&self) -> bool {
        self.wording == Wording::Glass
    }

    /// How many slots from `top` show whole: a page.
    fn page(&self) -> usize {
        self.fits_from(&self.slots(), self.top, self.body_height())
    }

    /// The list, its combo and Restore Defaults are live: not while the
    /// check box shows every icon anyway.
    pub fn list_enabled(&self) -> bool {
        self.all_icons != Some(true)
    }

    /// The list's slots: each non-empty group's header before its rows;
    /// the page's rows run on with no headers.
    pub fn slots(&self) -> Vec<Slot> {
        if self.is_page() {
            return (0..self.entries.len()).map(Slot::Row).collect();
        }
        let mut out = Vec::with_capacity(self.entries.len() + 2);
        let groups = [(0, 0..self.current), (1, self.current..self.entries.len())];
        for (g, range) in groups {
            if range.is_empty() {
                continue;
            }
            out.push(Slot::Header(g));
            out.extend(range.map(Slot::Row));
        }
        out
    }

    /// How tall slot `k` of `slots` is.
    fn slot_h(&self, slots: &[Slot], k: usize) -> i32 {
        let g = self.geometry();
        match slots[k] {
            Slot::Header(_) if k > 0 => g.group_h + g.group_gap,
            Slot::Header(_) => g.group_h,
            Slot::Row(_) => g.row_h,
        }
    }

    /// How many slots from `top` fit whole in `height` px (at least one).
    fn fits_from(&self, slots: &[Slot], top: usize, height: i32) -> usize {
        let mut y = 0;
        let mut n = 0;
        for k in top..slots.len() {
            y += self.slot_h(slots, k);
            if y > height {
                break;
            }
            n += 1;
        }
        n.max(1)
    }

    /// The largest `top` whose slots from there on all fit in `height`.
    fn max_top(&self, slots: &[Slot], height: i32) -> usize {
        let mut y = 0;
        let mut top = slots.len();
        while top > 0 {
            y += self.slot_h(slots, top - 1);
            if y > height {
                break;
            }
            top -= 1;
        }
        top
    }

    /// The combo for the selected entry, its list closed.
    fn rebuild_combo(&mut self) {
        let Some(e) = self.selected.and_then(|i| self.entries.get(i)) else {
            return;
        };
        let w = self.wording;
        let labels: Vec<&str> = w.choices().iter().map(|&b| w.label(b)).collect();
        self.combo = Combo::new(&labels, w.index(e.behaviour, e.passive), labels.len());
    }

    /// Select entry `i`, scrolled into view.
    pub fn select(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        self.selected = Some(i);
        self.rebuild_combo();
        self.show(i);
    }

    /// A reader asked to see row `i` of the list `hit` names: the open combo's list, else the
    /// items, scrolls it in and nothing is picked. True when it moved.
    pub fn reveal(&mut self, hit: Option<Hit>, i: usize) -> bool {
        if let Some(Hit::Combo(_)) = hit {
            return self.combo.reveal(i);
        }
        let top = self.top;
        if i < self.entries.len() {
            self.show(i);
        }
        self.top != top
    }

    /// A reader's Focus on what `hit` names: it takes the keyboard as Tab gives it, without
    /// opening, pressing or picking anything. A page row's field selects its row and focuses
    /// the combo.
    pub fn focus_on(&mut self, hit: Option<Hit>) -> Event {
        if let Some(Hit::Row(i)) = hit
            && self.is_page()
            && self.list_enabled()
        {
            self.combo.open = None;
            self.focus = Focus::Combo;
            self.select(i);
            return Event::Redraw;
        }
        let f = match hit {
            Some(Hit::Row(_) | Hit::List) => Focus::List,
            Some(Hit::Combo(_)) => Focus::Combo,
            Some(Hit::AllIcons) => Focus::AllIcons,
            Some(Hit::Restore) => Focus::Restore,
            Some(Hit::Ok) => Focus::Ok,
            Some(Hit::Cancel) => Focus::Cancel,
            _ => return Event::None,
        };
        if !self.focusable(f) || self.focus == f {
            return Event::None;
        }
        self.combo.open = None;
        self.focus = f;
        Event::Redraw
    }

    /// Scroll so entry `i` shows whole, with its group's header when that comes right above.
    fn show(&mut self, i: usize) {
        let height = self.body_height();
        let slots = self.slots();
        let Some(k) = slots.iter().position(|&s| s == Slot::Row(i)) else {
            return;
        };
        let want = if k > 0 && matches!(slots[k - 1], Slot::Header(_)) {
            k - 1
        } else {
            k
        };
        if want < self.top {
            self.top = want;
        }
        while self.top < k && self.fits_from(&slots, self.top, height) <= k - self.top {
            self.top += 1;
        }
    }

    /// The selected row took combo row `i`: its behaviour becomes that
    /// row's, unless that is the row it already showed.
    fn take_pick(&mut self, i: usize) {
        let w = self.wording;
        if let Some(e) = self.selected.and_then(|s| self.entries.get_mut(s))
            && w.index(e.behaviour, e.passive) != i
            && let Some(&b) = w.choices().get(i)
        {
            e.behaviour = b;
        }
    }

    /// Every item back to its status, the check box as it was.
    fn restore(&mut self) {
        for e in &mut self.entries {
            e.behaviour = Behaviour::Auto;
        }
        self.rebuild_combo();
    }

    pub fn choices(&self) -> Choices {
        Choices {
            behaviours: self
                .entries
                .iter()
                .map(|e| (e.key.clone(), e.behaviour))
                .collect(),
            all_icons: self.all_icons,
        }
    }

    fn focusable(&self, f: Focus) -> bool {
        match f {
            Focus::List => !self.is_page() && self.list_enabled() && !self.entries.is_empty(),
            Focus::Combo => self.list_enabled() && self.selected.is_some(),
            Focus::AllIcons => self.all_icons.is_some(),
            Focus::Restore => self.list_enabled(),
            Focus::Ok | Focus::Cancel => true,
        }
    }

    fn refocus(&mut self) {
        if !self.focusable(self.focus) {
            self.focus = Focus::Ok;
        }
    }
}

#[cfg(test)]
mod tests;
