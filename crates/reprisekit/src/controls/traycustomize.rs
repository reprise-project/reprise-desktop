//! The tray's Customize Notifications dialog: a text, the tray items in two
//! groups (registered now, then past ones that still carry a choice), each
//! with its 16 px icon, name and behaviour, Restore Defaults, OK and Cancel.
//! The selected row's behaviour cell turns into a combo box. The host lists
//! the items and acts on [`Event`]; nothing here knows the tray.
//!
//! Classic's client area is 348x391 with a 330x208 list; Glass lays it out as
//! a 1184x575 settings page with a 1000x376 list. Sizes are in the layout
//! consts below.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::access::{
    self, Tree,
    accesskit::{NodeId, Role},
};
use crate::classic::widgets::{
    BarHit, LINE_H, bar_hit, draw_focus_ring, draw_focus_ring_screen, draw_label, scrollbar_parts,
    scrollbar_pos_at,
};
use crate::controls::combo::{self, Combo, ComboLit};
use crate::controls::form;
use crate::controls::listview::{draw_group_header, list_rule};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::ThumbDrag;
use crate::cues::Cues;
use crate::icon::IconSource;
use crate::look::Paint;
use crate::look::widgets::header as header_widgets;
use crate::look::widgets::{draw_button, draw_checkbox, draw_field_frame, draw_scrollbar};
use crate::pointer::{Axis, Interaction, Pointer};
use crate::raster::{Canvas, Rect};
use crate::style::PageInks;
use crate::text::{self, Align, TextRenderer, split_mnemonic};
use crate::textedit::Caret;
use crate::theme::Theme;

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

/// Where an interface puts the parts, from the client area's top-left.
struct Geometry {
    client: (i32, i32),
    intro: Rect,
    prompt: Rect,
    list: Rect,
    /// The Name and Behavior columns' widths.
    columns: [i32; 2],
    /// How far the header starts above the frame's inner edge: the rows
    /// that hide under the frame.
    header_under: i32,
    /// A group header's slot, and what every group after the first has
    /// above it besides.
    group_h: i32,
    group_gap: i32,
    /// The rule's row in a group's slot.
    rule_y: i32,
    row_h: i32,
    /// The highlight's and the focus rectangle's height from the row's top.
    highlight_h: i32,
    /// The icon's and the text's box's offsets down the row.
    icon_dy: i32,
    text_dy: i32,
    check: Option<(i32, i32)>,
    restore: Rect,
    ok: Rect,
    cancel: Rect,
}

const CLASSIC: Geometry = Geometry {
    client: (348, 391),
    intro: Rect::new(9, 11, 330, 3 * LINE_H),
    prompt: Rect::new(9, 66, 330, LINE_H),
    list: Rect::new(9, 82, 330, 208),
    columns: [185, 124],
    header_under: 1,
    group_h: 32,
    group_gap: 11,
    rule_y: 19,
    row_h: 18,
    highlight_h: 17,
    icon_dy: 0,
    text_dy: 1,
    check: None,
    restore: Rect::new(246, 307, 93, 23),
    ok: Rect::new(183, 359, 75, 23),
    cancel: Rect::new(264, 359, 75, 23),
};

/// Glass's page. Its rows' box starts `PAGE_BODY_Y` into the list and
/// holds no group headers; `prompt` is the heading's box and `restore`
/// the link's. The column, header and highlight fields go unused.
const GLASS: Geometry = Geometry {
    client: (1184, 575),
    intro: Rect::new(92, 52, 1000, 20),
    prompt: Rect::new(92, 16, 1000, 26),
    list: Rect::new(92, 84, 1000, 376),
    columns: [PAGE_BEHAVIOURS_X, 279],
    header_under: 0,
    group_h: 0,
    group_gap: 0,
    rule_y: 0,
    row_h: 48,
    highlight_h: 0,
    icon_dy: 5,
    text_dy: 0,
    check: Some((92, 501)),
    restore: Rect::new(92, 470, 0, 20),
    ok: Rect::new(934, 542, 76, 23),
    cancel: Rect::new(1016, 542, 76, 23),
};

/// Glass's page, from the list's top-left: the rows' box's top, the
/// column titles' box, the rule's row and its ends (from 32 px into the
/// rows' box to 33 px short of the bar), where Behaviors starts and how
/// far each title sits right of its column's start.
const PAGE_BODY_Y: i32 = 48;
const PAGE_TITLES: Rect = Rect::new(33, 16, 0, 20);
const PAGE_TITLE_X: i32 = 2;
const PAGE_RULE_Y: i32 = 34;
const PAGE_RULE_INSET: i32 = 33;
const PAGE_BEHAVIOURS_X: i32 = 492;
/// A page row's parts from its top-left: the icon, the name's and the
/// tip's boxes, and the combo.
const PAGE_ICON: (i32, i32) = (41, 5);
const PAGE_NAME: Rect = Rect::new(67, 4, 400, 18);
const PAGE_TIP: Rect = Rect::new(67, 19, 400, 18);
/// The page's check box label, from the box's left edge, and a pixel up.
const PAGE_CHECK_LABEL: i32 = 19;
const PAGE_COMBO: Rect = Rect::new(PAGE_BEHAVIOURS_X, 8, 279, 23);
/// The separator over the button band, from the client's top.
const PAGE_BAND_Y: i32 = 532;
/// The heading's size over the body text's: 16 px to 12.
const HEADING_SCALE: (u32, u32) = (4, 3);

/// A row's icon and text, from the body's left edge.
const ICON_X: i32 = 4;
const NAME_X: i32 = 22;
const HIGHLIGHT_X: i32 = 20;
/// The behaviour text in from its column's left edge.
const CELL_X: i32 = 8;
/// Header titles in from their column button's left edge.
const HEADER_TEXT_X: i32 = 8;
/// A group's title in from the body's left edge, bold.
const GROUP_TEXT_X: i32 = 15;
const ICON_PX: i32 = 16;

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
}

/// A label's layout rect on the device: its width was measured at the
/// device font size, so only the position and height scale.
fn label_rect(r: Rect, s: i32) -> Rect {
    Rect::new(r.x * s, r.y * s, r.width, r.height * s)
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
    fn is_page(&self) -> bool {
        self.wording == Wording::Glass
    }

    pub fn client(&self) -> (i32, i32) {
        self.wording.geometry().client
    }

    /// The rows' box's height: the list's inside less the header's rows
    /// that show; on the page, the list's inside under its titles.
    pub fn body_height(&self) -> i32 {
        let g = self.wording.geometry();
        if self.is_page() {
            return g.list.height - PAGE_BODY_Y - 1;
        }
        g.list.height - 4 - (self.metrics.list_header - g.header_under)
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
        let g = self.wording.geometry();
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
        let height = self.body_height();
        if i >= self.entries.len() {
            return;
        }
        self.selected = Some(i);
        self.rebuild_combo();
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

    /// Press part `f`; nothing when it is greyed.
    fn press(&mut self, f: Focus) -> Event {
        if !self.focusable(f) {
            return Event::None;
        }
        match f {
            Focus::List | Focus::Combo => Event::None,
            Focus::AllIcons => {
                self.all_icons = self.all_icons.map(|on| !on);
                self.combo.open = None;
                self.focus = Focus::AllIcons;
                Event::Redraw
            }
            Focus::Restore => {
                self.restore();
                Event::Redraw
            }
            Focus::Ok => {
                self.take_pick(self.combo.selected);
                Event::Accept(self.choices())
            }
            Focus::Cancel => Event::Cancel,
        }
    }

    pub fn key(&mut self, key: form::Key) -> Event {
        let height = self.body_height();
        use form::Key;
        if self.combo.open.is_some() {
            return match key {
                Key::Up | Key::Down => {
                    self.combo.step(key == Key::Down);
                    Event::Redraw
                }
                Key::Enter | Key::Drop => {
                    self.combo.take();
                    self.take_pick(self.combo.selected);
                    Event::Redraw
                }
                Key::Escape => {
                    self.combo.open = None;
                    Event::Redraw
                }
                _ => Event::None,
            };
        }
        match key {
            Key::Tab { back } => {
                if self.is_page() && self.focus == Focus::Combo {
                    let next = match (self.selected, back) {
                        (Some(i), false) if i + 1 < self.entries.len() => Some(i + 1),
                        (Some(i), true) if i > 0 => Some(i - 1),
                        _ => None,
                    };
                    if let Some(i) = next {
                        self.select(i);
                        return Event::Redraw;
                    }
                }
                let n = TAB_ORDER.len();
                let mut at = TAB_ORDER.iter().position(|&f| f == self.focus).unwrap_or(0);
                loop {
                    at = (if back { at + n - 1 } else { at + 1 }) % n;
                    if self.focusable(TAB_ORDER[at]) {
                        break;
                    }
                }
                self.focus = TAB_ORDER[at];
                if self.focus == Focus::List && self.selected.is_none() {
                    self.select(0);
                }
                if self.is_page() && self.focus == Focus::Combo {
                    self.select(if back { self.entries.len() - 1 } else { 0 });
                }
                Event::Redraw
            }
            Key::Escape => Event::Cancel,
            Key::Enter => match self.focus {
                Focus::Cancel | Focus::Restore => self.press(self.focus),
                _ => self.press(Focus::Ok),
            },
            Key::Space => match self.focus {
                Focus::AllIcons | Focus::Restore | Focus::Ok | Focus::Cancel => {
                    self.press(self.focus)
                }
                _ => Event::None,
            },
            Key::Access(c) => match c.to_ascii_lowercase() {
                'r' => self.press(Focus::Restore),
                'a' => self.press(Focus::AllIcons),
                _ => Event::None,
            },
            Key::Drop if self.focus == Focus::Combo => redraw_if(self.combo.toggle()),
            Key::Up | Key::Down if self.focus == Focus::Combo => {
                let changed = self.combo.step(key == Key::Down);
                self.take_pick(self.combo.selected);
                redraw_if(changed)
            }
            Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown
                if self.focus == Focus::List && !self.entries.is_empty() =>
            {
                let last = self.entries.len() - 1;
                let page = (height / self.wording.geometry().row_h).max(1) as usize;
                let at = self.selected;
                let next = match key {
                    Key::Up => at.map_or(0, |i| i.saturating_sub(1)),
                    Key::Down => at.map_or(0, |i| (i + 1).min(last)),
                    Key::Home => 0,
                    Key::End => last,
                    Key::PageUp => at.map_or(0, |i| i.saturating_sub(page)),
                    _ => at.map_or(0, |i| (i + page).min(last)),
                };
                let moved = Some(next) != at;
                self.select(next);
                redraw_if(moved)
            }
            _ => Event::None,
        }
    }

    /// Scroll the list by `rows` slots.
    pub fn scroll(&mut self, rows: i32) -> bool {
        let slots = self.slots();
        let max = self.max_top(&slots, self.body_height()) as i32;
        let top = (self.top as i32 + rows).clamp(0, max) as usize;
        let moved = top != self.top;
        self.top = top;
        moved
    }

    /// A pointer press on `hit`.
    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        if self.combo.open.is_some() {
            match hit {
                Some(Hit::Combo(combo::Hit::Row(i))) => {
                    self.combo.pick(i);
                    self.take_pick(i);
                }
                Some(Hit::Combo(h)) if h.bar().is_some() => {
                    let bar = h.bar().unwrap_or(BarHit::Track);
                    self.combo.press_bar(bar, &mut self.combo_thumb);
                    return Event::Redraw;
                }
                _ => self.combo.open = None,
            }
            return Event::Redraw;
        }
        let Some(hit) = hit else {
            return Event::None;
        };
        match hit {
            Hit::Row(i) if self.is_page() && self.list_enabled() => {
                self.focus = Focus::Combo;
                self.select(i);
                self.combo.toggle();
                Event::Redraw
            }
            Hit::Row(i) if self.list_enabled() => {
                self.focus = Focus::List;
                if self.selected != Some(i) {
                    self.select(i);
                }
                Event::Redraw
            }
            Hit::Combo(_) if self.list_enabled() => {
                self.focus = Focus::Combo;
                self.combo.toggle();
                Event::Redraw
            }
            Hit::Bar(bar) if self.list_enabled() => {
                let page = self.page() as i64;
                let by = self.thumb.press(Axis::Vertical, bar).by(1, page);
                redraw_if(self.scroll(by as i32) || matches!(bar, BarHit::Thumb(_)))
            }
            Hit::AllIcons => self.arm(Focus::AllIcons),
            Hit::Restore => self.arm(Focus::Restore),
            Hit::Ok => self.arm(Focus::Ok),
            Hit::Cancel => self.arm(Focus::Cancel),
            _ => Event::None,
        }
    }

    /// A press on push button or box `f`: a live one draws pressed until
    /// the release.
    fn arm(&mut self, f: Focus) -> Event {
        if !self.focusable(f) {
            return Event::None;
        }
        if f != Focus::Ok {
            self.focus = f;
        }
        self.pointer.press(Some(f), false);
        Event::Redraw
    }

    /// Whether the pointer holds a push button, the box or a thumb.
    pub fn holding(&self) -> bool {
        self.pointer.holding() || self.thumb.holding() || self.combo_thumb.holding()
    }

    /// The pointer at `y` with a thumb held: its list follows. `None`
    /// with none held.
    pub fn drag(&mut self, lay: &Layout, y: i32) -> Option<Event> {
        if let (Some((_, cl)), true) = (&lay.combo, self.combo_thumb.holding()) {
            return Some(redraw_if(self.combo.drag_bar(cl, &self.combo_thumb, y)));
        }
        let grab = self.thumb.grab(Axis::Vertical)?;
        let sb = lay.vbar?;
        let slots = self.slots();
        let parts = scrollbar_parts(sb, 1, self.top, lay.page, slots.len(), false, lay.thumb_min);
        let top = scrollbar_pos_at(&parts, y - grab, lay.page, slots.len(), false)
            .min(self.max_top(&slots, lay.body.height));
        let moved = top != self.top;
        self.top = top;
        Some(redraw_if(moved))
    }

    fn button(hit: Option<Hit>) -> Option<Focus> {
        match hit {
            Some(Hit::AllIcons) => Some(Focus::AllIcons),
            Some(Hit::Restore) => Some(Focus::Restore),
            Some(Hit::Ok) => Some(Focus::Ok),
            Some(Hit::Cancel) => Some(Focus::Cancel),
            _ => None,
        }
    }

    /// The pointer moved over `hit`: a held push button draws pressed
    /// only while the pointer is on it. With nothing held, the button,
    /// box, link or page combo under it draws hot.
    pub fn motion(&mut self, hit: Option<Hit>) -> Event {
        let held = self.pointer.holding();
        let mut changed = self.pointer.motion(Self::button(hit));
        if let (true, Some(open), Some(Hit::Combo(combo::Hit::Row(i)))) =
            (self.is_page(), self.combo.open, hit)
            && open != i
        {
            self.combo.open = Some(i);
            changed = true;
        }
        if !held {
            let row = match hit {
                Some(Hit::Row(i)) if self.is_page() => Some(i),
                Some(Hit::Combo(combo::Hit::Field | combo::Hit::Drop)) if self.is_page() => {
                    self.selected
                }
                _ => None,
            };
            changed |= std::mem::replace(&mut self.hot_row, row) != row;
        }
        redraw_if(changed)
    }

    /// The pointer left the dialog: nothing stays hot.
    pub fn leave(&mut self) -> Event {
        let changed = self.pointer.leave() | self.hot_row.take().is_some();
        redraw_if(changed)
    }

    /// The pointer let go over `hit`: the held push button or box acts if
    /// the release lands on it. A held thumb lets go.
    pub fn release(&mut self, hit: Option<Hit>) -> Event {
        if self.thumb.release() | self.combo_thumb.release() {
            return Event::Redraw;
        }
        if !self.pointer.holding() {
            return Event::None;
        }
        self.pointer.motion(Self::button(hit));
        let Some(f) = self.pointer.release() else {
            return Event::Redraw;
        };
        match self.press(f) {
            Event::None => Event::Redraw,
            e => {
                self.refocus();
                e
            }
        }
    }

    /// The parts with the client area's top-left at `origin`. Label
    /// widths are measured at the device font size `px`.
    pub fn layout(&self, text: &TextRenderer, px: u32, origin: (i32, i32)) -> Layout {
        if self.is_page() {
            return self.page_layout(text, px, origin);
        }
        let g = self.wording.geometry();
        let cm = &self.metrics;
        let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
        let frame = at(g.list);
        let inner = frame.inset(2);
        let header_h = cm.list_header - g.header_under;
        let full = Rect::new(
            inner.x,
            inner.y + header_h,
            inner.width,
            inner.height - header_h,
        );
        let slots = self.slots();
        let content: i32 = (0..slots.len()).map(|k| self.slot_h(&slots, k)).sum();
        let vbar = content > full.height;
        let body = Rect::new(
            full.x,
            full.y,
            full.width - if vbar { cm.scroll } else { 0 },
            full.height,
        );
        let header = Rect::new(
            inner.x,
            inner.y - g.header_under,
            body.width,
            cm.list_header,
        );
        let mut columns = [Rect::default(); 2];
        let mut x = body.x;
        for (i, w) in g.columns.into_iter().enumerate() {
            let lead = if i == 0 { g.header_under } else { 0 };
            columns[i] = Rect::new(x - lead, header.y, w + lead, header.height);
            x += w;
        }
        let top = self.top.min(slots.len());
        let mut placed = Vec::new();
        let mut y = body.y;
        for k in top..slots.len() {
            if y >= body.bottom() {
                break;
            }
            let h = self.slot_h(&slots, k);
            placed.push((slots[k], Rect::new(body.x, y, body.width, h)));
            y += h;
        }
        let combo = self
            .selected
            .filter(|_| self.list_enabled())
            .and_then(|i| placed.iter().find(|(s, _)| *s == Slot::Row(i)))
            .map(|&(Slot::Row(i) | Slot::Header(i), r)| {
                let field = Rect::new(body.x + g.columns[0] + 1, r.y, g.columns[1], combo::COMBO_H);
                (i, combo::layout(&self.combo, field, cm))
            });
        let (check, check_label) = match g.check {
            Some((cx, cy)) => {
                let b = at(Rect::new(cx, cy, cm.check, cm.check));
                let (plain, _) = split_mnemonic(ALL_ICONS);
                let l = Rect::new(
                    b.x + cm.check_label,
                    b.y,
                    text.measure(&plain, false, px),
                    cm.check,
                );
                (Some(b), Some(l))
            }
            None => (None, None),
        };
        Layout {
            client: at(Rect::new(0, 0, g.client.0, g.client.1)),
            intro: at(g.intro),
            prompt: at(g.prompt),
            frame,
            inner,
            header,
            columns,
            body,
            vbar: vbar.then(|| Rect::new(body.right(), inner.y, cm.scroll, inner.height)),
            page: self.fits_from(&slots, top, body.height),
            slots: placed,
            combo,
            check,
            check_label,
            restore: at(g.restore),
            ok: at(g.ok),
            cancel: at(g.cancel),
            thumb_min: cm.thumb_min,
            fields: Vec::new(),
            band: None,
        }
    }

    /// Glass's page: every row in view carries its combo's field, the bar
    /// is always there, and Restore is a link as wide as its words.
    fn page_layout(&self, text: &TextRenderer, px: u32, origin: (i32, i32)) -> Layout {
        let g = &GLASS;
        let cm = &self.metrics;
        let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
        let frame = at(g.list);
        let inner = frame.inset(1);
        let vbar = Rect::new(inner.right() - cm.scroll, inner.y, cm.scroll, inner.height);
        let body = Rect::new(
            inner.x,
            frame.y + PAGE_BODY_Y,
            vbar.x - inner.x,
            inner.bottom() - frame.y - PAGE_BODY_Y,
        );
        let header = Rect::new(
            frame.x + PAGE_TITLES.x,
            frame.y + PAGE_TITLES.y,
            vbar.x - PAGE_RULE_INSET - frame.x - PAGE_TITLES.x,
            PAGE_TITLES.height,
        );
        let columns = [
            Rect::new(
                header.x,
                header.y,
                PAGE_BEHAVIOURS_X - PAGE_TITLES.x,
                header.height,
            ),
            Rect::new(
                frame.x + PAGE_BEHAVIOURS_X,
                header.y,
                header.right() - frame.x - PAGE_BEHAVIOURS_X,
                header.height,
            ),
        ];
        let slots = self.slots();
        let top = self.top.min(slots.len());
        let mut placed = Vec::new();
        let mut fields = Vec::new();
        let mut y = body.y;
        for &slot in &slots[top..] {
            if y >= body.bottom() {
                break;
            }
            let r = Rect::new(body.x, y, body.width, g.row_h);
            if let Slot::Row(i) = slot {
                let f = PAGE_COMBO;
                fields.push((i, Rect::new(frame.x + f.x, y + f.y, f.width, f.height)));
            }
            placed.push((slot, r));
            y += g.row_h;
        }
        let combo = self
            .selected
            .and_then(|i| fields.iter().find(|(j, _)| *j == i))
            .map(|&(i, f)| (i, combo::layout(&self.combo, f, cm)));
        let (cx, cy) = g.check.unwrap_or_default();
        let check = at(Rect::new(cx, cy, cm.check, cm.check));
        let (plain, _) = split_mnemonic(ALL_ICONS);
        let check_label = Rect::new(
            check.x + PAGE_CHECK_LABEL,
            check.y - 1,
            text.measure(&plain, false, px),
            cm.check,
        );
        let link = at(g.restore);
        let restore = Rect::new(
            link.x,
            link.y,
            text.measure(self.wording.restore(), false, px),
            link.height,
        );
        Layout {
            client: at(Rect::new(0, 0, g.client.0, g.client.1)),
            intro: at(g.intro),
            prompt: at(g.prompt),
            frame,
            inner,
            header,
            columns,
            body,
            vbar: Some(vbar),
            page: self.fits_from(&slots, top, body.height),
            slots: placed,
            combo,
            check: Some(check),
            check_label: Some(check_label),
            restore,
            ok: at(g.ok),
            cancel: at(g.cancel),
            thumb_min: cm.thumb_min,
            fields,
            band: Some(at(Rect::new(
                0,
                PAGE_BAND_Y,
                g.client.0,
                g.client.1 - PAGE_BAND_Y,
            ))),
        }
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
        if let Some((_, cl)) = &lay.combo
            && let Some(h) = combo::hit(&self.combo, cl, x, y)
        {
            return Some(Hit::Combo(h));
        }
        for (r, h) in [
            (lay.ok, Hit::Ok),
            (lay.cancel, Hit::Cancel),
            (lay.restore, Hit::Restore),
        ] {
            if r.contains(x, y) {
                return Some(h);
            }
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label)
            && (b.contains(x, y) || l.contains(x, y))
        {
            return Some(Hit::AllIcons);
        }
        if let Some(sb) = lay.vbar.filter(|sb| sb.contains(x, y)) {
            let total = self.slots().len();
            let parts = scrollbar_parts(sb, 1, self.top, lay.page, total, false, lay.thumb_min);
            return Some(Hit::Bar(bar_hit(&parts, x, y, false)));
        }
        if self.is_page() && lay.body.contains(x, y) {
            let row = lay
                .fields
                .iter()
                .find_map(|&(i, f)| f.contains(x, y).then_some(i));
            return Some(row.map_or(Hit::List, Hit::Row));
        }
        if lay.body.contains(x, y) {
            let row = lay.slots.iter().find_map(|&(s, r)| match s {
                Slot::Row(i) if r.contains(x, y) => Some(i),
                _ => None,
            });
            return Some(row.map_or(Hit::List, Hit::Row));
        }
        lay.frame.contains(x, y).then_some(Hit::List)
    }

    /// Draw the client area at `p`'s scale. `lay` is measured at the
    /// device font size `px`. `cues` says whether the focus rectangles
    /// and access-key underlines show; an inactive window (`p.active`
    /// false) draws neither the dots nor a default ring.
    pub fn draw(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        cues: Cues,
        px: u32,
    ) {
        if self.is_page() {
            let inks = p.look.style().and_then(|st| st.page);
            let inks = inks.unwrap_or(PageInks::LIGHT);
            return self.draw_page(c, text, p, &inks, lay, cues, px);
        }
        let Paint { cs, s, active, .. } = *p;
        let dots = cues.focus && active;
        let w = self.wording;
        let g = w.geometry();
        let live = self.list_enabled();
        c.fill_rect(lay.client.scaled(s), cs.button_face);
        let lines = text::wrap(text, w.intro(), lay.intro.width * s, px);
        let pitch = lay.intro.height / (lay.intro.height / LINE_H).max(1);
        for (i, line) in lines.iter().enumerate() {
            let r = Rect::new(
                lay.intro.x,
                lay.intro.y + i as i32 * pitch,
                lay.intro.width,
                pitch,
            );
            text.draw(c, r.scaled(s), line, false, cs.button_text, Align::Left, px);
        }
        let prompt = lay.prompt.scaled(s);
        text.draw(
            c,
            prompt,
            w.prompt(),
            false,
            cs.button_text,
            Align::Left,
            px,
        );

        draw_field_frame(c, p, lay.frame.scaled(s), cs.window);
        let body = lay.body.scaled(s);
        c.with_clip(body, |c| self.draw_rows(c, text, p, lay, dots, px));
        let inner = lay.inner.scaled(s);
        c.with_clip(inner, |c| {
            let titles = w.columns();
            let hr = lay.header.scaled(s);
            for (i, r) in lay.columns.iter().enumerate() {
                let t = header_widgets::draw_thin_item(c, p, r.scaled(s), Interaction::Idle);
                let tr = Rect::new(
                    t.x + HEADER_TEXT_X * s,
                    hr.y + g.header_under * s,
                    (t.width - (2 * HEADER_TEXT_X) * s).max(0),
                    hr.height - g.header_under * s,
                );
                let title = text.ellipsize(titles[i], tr.width, false, px);
                text.draw(c, tr, &title, false, cs.button_text, Align::Left, px);
            }
            let end = lay.columns[1].right();
            let filler = Rect::new(
                end,
                lay.header.y,
                lay.inner.right() - end + 2,
                lay.header.height,
            );
            header_widgets::draw_thin_item(c, p, filler.scaled(s), Interaction::Idle);
        });
        if let Some(sb) = lay.vbar {
            let total = self.slots().len();
            draw_scrollbar(c, p, sb.scaled(s), self.top, lay.page, total, None);
        }
        if let (Some((_, cl)), true) = (&lay.combo, live) {
            let focused = self.focus == Focus::Combo && active;
            c.with_clip(body, |c| {
                combo::draw(
                    c,
                    text,
                    p,
                    &self.combo,
                    cl,
                    focused,
                    focused && dots,
                    Caret::Hidden,
                    ComboLit::IDLE,
                    px,
                );
            });
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label) {
            let (b, lr) = (b.scaled(s), label_rect(l, s));
            let label = cues.label(ALL_ICONS);
            let on = self.all_icons == Some(true);
            draw_checkbox(c, p, b, on, self.pointer.of(Focus::AllIcons));
            draw_label(
                c,
                text,
                lr,
                &label,
                false,
                cs.button_text,
                Align::Left,
                px,
                s,
            );
            if self.focus == Focus::AllIcons && dots {
                let ring = Rect::new(lr.x - s, lr.y - s, lr.width + 2 * s, lr.height + 2 * s);
                draw_focus_ring(c, ring, s, cs.button_text);
            }
        }
        let ring = |f: Focus| {
            active
                && match self.focus {
                    Focus::Restore | Focus::Cancel => f == self.focus,
                    _ => f == Focus::Ok,
                }
        };
        for (r, label, enabled, f) in [
            (lay.restore, &*cues.label(w.restore()), live, Focus::Restore),
            (lay.ok, "OK", true, Focus::Ok),
            (lay.cancel, "Cancel", true, Focus::Cancel),
        ] {
            draw_button(
                c,
                text,
                p,
                r.scaled(s),
                label,
                ring(f),
                self.pointer.of(f),
                enabled,
                dots && self.focus == f,
                px,
            );
        }
        if let (Some((_, cl)), Some(open)) = (&lay.combo, self.combo.open) {
            combo::draw_list(
                c,
                text,
                p,
                &self.combo.items,
                Some(open),
                self.combo.top,
                self.combo.rows,
                cl.list,
                cl.scrollbar,
                None,
                px,
            );
        }
    }

    /// Glass's page: white down to the band's rule, the heading, the text
    /// and the framed list with every row's combo, the link, the box and
    /// the band with OK and Cancel. A hot combo, link, box or button draws
    /// hot; the box ticked greys the combos and the link.
    #[allow(clippy::too_many_arguments)]
    fn draw_page(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        inks: &PageInks,
        lay: &Layout,
        cues: Cues,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let dots = cues.focus && active;
        let w = self.wording;
        let live = self.list_enabled();
        c.fill_rect(lay.client.scaled(s), cs.window);
        let heading_px = px * HEADING_SCALE.0 / HEADING_SCALE.1;
        let heading = Wording::HEADING;
        text.draw(
            c,
            lay.prompt.scaled(s),
            heading,
            false,
            inks.heading,
            Align::Left,
            heading_px,
        );
        let intro = lay.intro.scaled(s);
        text.draw(c, intro, w.intro(), false, cs.window_text, Align::Left, px);

        let frame = lay.frame.scaled(s);
        c.fill_rect(frame, inks.list_frame);
        c.fill_rect(lay.inner.scaled(s), cs.window);
        for (r, title) in lay.columns.iter().zip(w.columns()) {
            let tr = Rect::new(r.x + PAGE_TITLE_X, r.y, r.width - PAGE_TITLE_X, r.height);
            text.draw(
                c,
                tr.scaled(s),
                title,
                true,
                cs.window_text,
                Align::Left,
                px,
            );
        }
        let rule_x = lay.columns[0].x;
        let rule_end = lay.body.right() - PAGE_RULE_INSET;
        let rule = Rect::new(rule_x, lay.frame.y + PAGE_RULE_Y, rule_end + 1 - rule_x, 1);
        c.fill_rect(rule.scaled(s), inks.rule);
        let body = lay.body.scaled(s);
        c.with_clip(body, |c| {
            for &(slot, r) in &lay.slots {
                let Slot::Row(i) = slot else { continue };
                self.draw_page_row(c, text, p, inks, lay, i, r, dots, px);
            }
        });
        if let Some(sb) = lay.vbar {
            let total = self.slots().len();
            draw_scrollbar(c, p, sb.scaled(s), self.top, lay.page, total, None);
        }

        let link = label_rect(lay.restore, s);
        let lit = self.pointer.of(Focus::Restore);
        let ink = match (live, lit) {
            (false, _) => inks.detail,
            (true, Interaction::Idle) => inks.link,
            (true, _) => inks.link_hot,
        };
        text.draw(c, link, w.restore(), false, ink, Align::Left, px);
        if live
            && lit != Interaction::Idle
            && let Some(base) = text.baseline(link, false, px)
        {
            c.fill_rect(Rect::new(link.x, base + s, link.width, s), ink);
        }
        if self.focus == Focus::Restore && dots {
            let ring = Rect::new(link.x - s, link.y, link.width + 2 * s, link.height);
            draw_focus_ring(c, ring, s, cs.window_text);
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label) {
            let (b, lr) = (b.scaled(s), label_rect(l, s));
            let on = self.all_icons == Some(true);
            draw_checkbox(c, p, b, on, self.pointer.of(Focus::AllIcons));
            let label = cues.label(ALL_ICONS);
            draw_label(
                c,
                text,
                lr,
                &label,
                false,
                cs.window_text,
                Align::Left,
                px,
                s,
            );
            if self.focus == Focus::AllIcons && dots {
                let ring = Rect::new(lr.x - s, lr.y - s, lr.width + 2 * s, lr.height + 2 * s);
                draw_focus_ring(c, ring, s, cs.window_text);
            }
        }
        if let Some(band) = lay.band {
            let band = band.scaled(s);
            c.fill_rect(band, cs.button_face);
            c.fill_rect(Rect::new(band.x, band.y, band.width, s), inks.rule);
        }
        for (r, label, f) in [
            (lay.ok, "OK", Focus::Ok),
            (lay.cancel, "Cancel", Focus::Cancel),
        ] {
            let default = active
                && match self.focus {
                    Focus::Cancel => f == Focus::Cancel,
                    _ => f == Focus::Ok,
                };
            let ptr = self.pointer.of(f);
            draw_button(
                c,
                text,
                p,
                r.scaled(s),
                label,
                default,
                ptr,
                true,
                dots && self.focus == f,
                px,
            );
        }
        if let (Some((_, cl)), Some(open)) = (&lay.combo, self.combo.open) {
            combo::draw_list(
                c,
                text,
                p,
                &self.combo.items,
                Some(open),
                self.combo.top,
                self.combo.rows,
                cl.list,
                cl.scrollbar,
                None,
                px,
            );
        }
    }

    /// Page row `i` in its slot `r` (logical px): the icon, the name over
    /// the tip, and the row's own combo, lit as the pointer has it.
    #[allow(clippy::too_many_arguments)]
    fn draw_page_row(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        inks: &PageInks,
        lay: &Layout,
        i: usize,
        r: Rect,
        dots: bool,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let w = self.wording;
        let e = &self.entries[i];
        let x0 = lay.frame.x;
        if let Some(src) = e
            .icon
            .iter()
            .min_by_key(|src| (src.width as i32 - ICON_PX * s).abs())
        {
            let ic = src.to_canvas(ICON_PX * s);
            c.blit_over((x0 + PAGE_ICON.0) * s, (r.y + PAGE_ICON.1) * s, &ic);
        }
        for (b, line, ink) in [
            (PAGE_NAME, &e.name, cs.window_text),
            (PAGE_TIP, &e.detail, inks.detail),
        ] {
            let tr = Rect::new(x0 + b.x, r.y + b.y, b.width, b.height).scaled(s);
            let shown = text.ellipsize(line, tr.width, false, px);
            text.draw(c, tr, &shown, false, ink, Align::Left, px);
        }
        let Some(&(_, field)) = lay.fields.iter().find(|(j, _)| *j == i) else {
            return;
        };
        let mine = self.selected == Some(i);
        let row_combo;
        let (combo, cl) = match &lay.combo {
            Some((j, cl)) if *j == i => (&self.combo, *cl),
            _ => {
                let labels: Vec<&str> = w.choices().iter().map(|&b| w.label(b)).collect();
                row_combo = Combo::new(&labels, w.index(e.behaviour, e.passive), labels.len());
                (&row_combo, combo::layout(&row_combo, field, &self.metrics))
            }
        };
        let greyed;
        let combo = if self.list_enabled() {
            combo
        } else {
            greyed = Combo {
                enabled: false,
                open: None,
                ..combo.clone()
            };
            &greyed
        };
        let frame = if mine && combo.open.is_some() {
            Interaction::Pressed
        } else if self.hot_row == Some(i) && combo.enabled {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        let focused = mine && self.focus == Focus::Combo && active;
        let lit = ComboLit {
            frame,
            drop: Interaction::Idle,
        };
        combo::draw(
            c,
            text,
            p,
            combo,
            &cl,
            focused,
            focused && dots,
            Caret::Hidden,
            lit,
            px,
        );
    }

    /// The rows and group headers in view, clipped to the body.
    fn draw_rows(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        dots: bool,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let w = self.wording;
        let g = w.geometry();
        let live = self.list_enabled();
        c.fill_rect(lay.body.scaled(s), cs.window);
        let ink = if live { cs.window_text } else { cs.gray_text };
        let titles = [CURRENT, PAST];
        for &(slot, r) in &lay.slots {
            let i = match slot {
                Slot::Header(grp) => {
                    let gap = r.height - g.group_h;
                    let top = r.y + gap + g.rule_y - crate::controls::listview::GROUP_RULE_Y;
                    let hr = Rect::new(r.x, top, r.width, g.group_h).scaled(s);
                    draw_group_header(c, text, cs, hr, "", (0, 0), &list_rule(cs), px, s);
                    let tr = Rect::new(
                        r.x + GROUP_TEXT_X,
                        r.y + gap,
                        r.width - GROUP_TEXT_X,
                        g.rule_y + 1,
                    );
                    text.draw(c, tr.scaled(s), titles[grp], true, ink, Align::Left, px);
                    continue;
                }
                Slot::Row(i) => i,
            };
            let e = &self.entries[i];
            let row = r.scaled(s);
            if let Some(src) = e
                .icon
                .iter()
                .min_by_key(|src| (src.width as i32 - ICON_PX * s).abs())
            {
                let ic = src.to_canvas(ICON_PX * s);
                c.blit_over(row.x + ICON_X * s, row.y + g.icon_dy * s, &ic);
            }
            let name_r = Rect::new(
                row.x + NAME_X * s,
                row.y + g.text_dy * s,
                (g.columns[0] - NAME_X - 4) * s,
                g.highlight_h * s,
            );
            let name = text.ellipsize(&e.name, name_r.width, false, px);
            let selected = self.selected == Some(i) && live;
            let focused = active && self.focus == Focus::List;
            let mut name_ink = ink;
            if selected {
                let end = row.x + (g.columns[0] + 1) * s;
                let hx = row.x + HIGHLIGHT_X * s;
                let hl = Rect::new(hx, row.y, end - hx, g.highlight_h * s);
                c.fill_rect(hl, cs.hilight);
                name_ink = cs.hilight_text;
                if focused && dots {
                    draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
                }
            }
            text.draw(c, name_r, &name, false, name_ink, Align::Left, px);
            let bx = row.x + (g.columns[0] + CELL_X) * s;
            let br = Rect::new(
                bx,
                row.y + g.text_dy * s,
                (g.columns[1] - 2 * CELL_X) * s,
                g.highlight_h * s,
            );
            let label = w.label(w.choices()[w.index(e.behaviour, e.passive)]);
            let label = text.ellipsize(label, br.width, false, px);
            text.draw(c, br, &label, false, ink, Align::Left, px);
        }
    }

    /// The text, the list with its rows (each named by the item and
    /// valued by its behaviour), the combo, the box and the buttons; the
    /// focused part gets the focus.
    pub fn access(&self, tree: &mut Tree, lay: &Layout, text: &TextRenderer, px: u32) -> NodeId {
        let root = access::ROOT;
        let w = self.wording;
        let live = self.list_enabled();
        tree.push(root, "intro", access::label(lay.intro, w.intro(), true));
        let prompt = if self.is_page() {
            Wording::HEADING
        } else {
            w.prompt()
        };
        tree.push(root, "prompt", access::label(lay.prompt, prompt, true));
        let mut ln = access::node(Role::ListBox, lay.frame);
        access::focusable(&mut ln, live);
        let list = tree.push(root, "list", ln);
        let mut focus_row = list;
        for (i, e) in self.entries.iter().enumerate() {
            let mut n = access::node(Role::ListBoxOption, Rect::default());
            if let Some((_, r)) = lay.slots.iter().find(|(s, _)| *s == Slot::Row(i)) {
                n.set_bounds(access::bounds(*r));
            }
            n.set_label(e.name.as_str());
            n.set_value(w.label(w.choices()[w.index(e.behaviour, e.passive)]));
            n.set_selected(self.selected == Some(i));
            let id = tree.push(list, ("row", i), n);
            if self.selected == Some(i) {
                focus_row = id;
            }
        }
        let mut combo_focus = None;
        if let (Some((_, cl)), true) = (&lay.combo, live) {
            combo_focus = Some(combo::access(
                &self.combo,
                cl,
                text,
                px,
                tree,
                list,
                "combo",
            ));
        }
        let all = match (lay.check, lay.check_label, self.all_icons) {
            (Some(b), Some(l), Some(on)) => Some(tree.push(
                root,
                "all",
                access::toggle(Role::CheckBox, b.union(&l), ALL_ICONS, on.into(), true),
            )),
            _ => None,
        };
        let restore = tree.push(
            root,
            "restore",
            access::button(lay.restore, w.restore(), live, false),
        );
        let ok = tree.push(root, "ok", access::button(lay.ok, "OK", true, true));
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, "Cancel", true, false),
        );
        let focus = match self.focus {
            Focus::List => focus_row,
            Focus::Combo => combo_focus.unwrap_or(list),
            Focus::AllIcons => all.unwrap_or(ok),
            Focus::Restore => restore,
            Focus::Ok => ok,
            Focus::Cancel => cancel,
        };
        tree.set_focus(focus);
        focus
    }
}

/// Where the parts go, in logical px.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client: Rect,
    pub intro: Rect,
    pub prompt: Rect,
    /// The list's sunken frame and what is inside it.
    pub frame: Rect,
    pub inner: Rect,
    pub header: Rect,
    /// The Name and Behavior column buttons.
    pub columns: [Rect; 2],
    /// The rows' box under the header, beside the bar.
    pub body: Rect,
    pub vbar: Option<Rect>,
    /// How many slots from `top` show whole: a page.
    pub page: usize,
    /// The slots in view with their rects; the last may run past the body.
    pub slots: Vec<(Slot, Rect)>,
    /// The selected entry's combo, when its row is in view.
    pub combo: Option<(usize, combo::Layout)>,
    pub check: Option<Rect>,
    pub check_label: Option<Rect>,
    pub restore: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    pub thumb_min: i32,
    /// Each page row in view and its combo's field; empty off the page.
    pub fields: Vec<(usize, Rect)>,
    /// The page's button band under its rule.
    pub band: Option<Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass;
    use crate::interface::Interface;
    use crate::interfaces::theme_with;

    fn text() -> TextRenderer {
        TextRenderer::load(&crate::theme::FontConfig::default())
    }

    fn glass_theme() -> Theme {
        theme_with(
            Interface::Glass,
            crate::blue::Variant::Blue,
            &glass::Settings::default(),
        )
    }

    fn entry(key: &str, name: &str, passive: bool, b: Behaviour) -> Entry {
        let red = IconSource {
            width: 16,
            height: 16,
            pixels: vec![0xFFFF_0000; 256],
        };
        Entry {
            key: key.into(),
            name: name.into(),
            detail: format!("{name} is running"),
            icon: Arc::from([red]),
            passive,
            behaviour: b,
        }
    }

    fn metrics(w: Wording) -> ControlMetrics {
        match w {
            Wording::Classic => Theme::default().look.controls(),
            Wording::Glass => glass_theme().look.controls(),
        }
    }

    fn sample(w: Wording) -> TrayCustomize {
        TrayCustomize::new(
            w,
            vec![
                entry("nm-applet", "Network", false, Behaviour::Auto),
                entry("volume", "Volume", true, Behaviour::Auto),
                entry("mail", "Mail", false, Behaviour::Show),
            ],
            vec![entry("sticky", "Sticky Keys", false, Behaviour::Hide)],
            (w == Wording::Glass).then_some(false),
            metrics(w),
        )
    }

    #[test]
    fn a_behaviour_hides_by_status_only_when_left_auto() {
        assert!(Behaviour::Auto.hidden(true));
        assert!(!Behaviour::Auto.hidden(false));
        assert!(!Behaviour::Show.hidden(true), "always shown");
        assert!(Behaviour::Hide.hidden(false), "always hidden");
        assert!(Behaviour::Mute.hidden(false));
        assert!(Behaviour::Hide.balloons(), "only the icon hides");
        assert!(!Behaviour::Mute.balloons(), "icon and balloons hide");
    }

    #[test]
    fn each_interface_names_the_behaviours_in_its_own_words() {
        let c = Wording::Classic;
        let labels: Vec<_> = c.choices().iter().map(|&b| c.label(b)).collect();
        assert_eq!(labels, ["Hide when inactive", "Always hide", "Always show"]);
        let g = Wording::Glass;
        let labels: Vec<_> = g.choices().iter().map(|&b| g.label(b)).collect();
        assert_eq!(
            labels,
            [
                "Show icon and notifications",
                "Hide icon and notifications",
                "Only show notifications"
            ]
        );
        // A choice one interface has no row for shows as the row that
        // does the same to the icon.
        assert_eq!(c.index(Behaviour::Mute, false), 1, "Always hide");
        assert_eq!(g.index(Behaviour::Auto, true), 2, "Only show notifications");
        assert_eq!(g.index(Behaviour::Auto, false), 0, "Show icon...");
    }

    #[test]
    fn the_list_groups_current_items_before_past_ones() {
        let t = sample(Wording::Classic);
        assert_eq!(
            t.slots(),
            [
                Slot::Header(0),
                Slot::Row(0),
                Slot::Row(1),
                Slot::Row(2),
                Slot::Header(1),
                Slot::Row(3)
            ]
        );
        let none_past = TrayCustomize::new(
            Wording::Classic,
            vec![entry("a", "A", false, Behaviour::Auto)],
            vec![],
            None,
            metrics(Wording::Classic),
        );
        assert_eq!(none_past.slots(), [Slot::Header(0), Slot::Row(0)]);
    }

    #[test]
    fn the_classic_layout_has_its_parts_where_the_dialog_puts_them() {
        let t = sample(Wording::Classic);
        let text = text();
        let l = t.layout(&text, 11, (0, 0));
        assert_eq!(t.client(), (348, 391));
        assert_eq!(l.frame, Rect::new(9, 82, 330, 208));
        assert_eq!(
            l.header,
            Rect::new(11, 83, 326, 17),
            "a row under the frame"
        );
        assert_eq!(l.body, Rect::new(11, 100, 326, 188), "few rows: no bar");
        assert_eq!(l.columns[0], Rect::new(10, 83, 186, 17));
        assert_eq!(l.columns[1], Rect::new(196, 83, 124, 17));
        let ys: Vec<_> = l.slots.iter().map(|(_, r)| (r.y, r.height)).collect();
        assert_eq!(
            ys,
            [
                (100, 32),
                (132, 18),
                (150, 18),
                (168, 18),
                (186, 43),
                (229, 18)
            ]
        );
        assert_eq!(l.restore, Rect::new(246, 307, 93, 23));
        assert_eq!(l.ok, Rect::new(183, 359, 75, 23));
        assert_eq!(l.cancel, Rect::new(264, 359, 75, 23));
        assert!(
            l.check.is_none(),
            "Classic's switch is on the settings sheet"
        );
    }

    #[test]
    fn the_selected_rows_combo_is_its_behaviour_cell() {
        let mut t = sample(Wording::Classic);
        let text = text();
        t.select(1);
        let l = t.layout(&text, 11, (0, 0));
        let (i, cl) = l.combo.expect("a combo on the selected row");
        assert_eq!(i, 1);
        assert_eq!(cl.field, Rect::new(11 + 186, 150, 124, 21));
        assert_eq!(t.combo.value(), "Hide when inactive");
        assert_eq!(t.combo.items.len(), 3);
    }

    #[test]
    fn many_rows_bring_the_bar_beside_the_header() {
        let many: Vec<_> = (0..12)
            .map(|i| {
                entry(
                    &format!("k{i}"),
                    &format!("Item {i}"),
                    false,
                    Behaviour::Auto,
                )
            })
            .collect();
        let t = TrayCustomize::new(
            Wording::Classic,
            many,
            vec![],
            None,
            metrics(Wording::Classic),
        );
        let text = text();
        let l = t.layout(&text, 11, (0, 0));
        assert_eq!(l.vbar, Some(Rect::new(321, 84, 16, 204)));
        assert_eq!(l.body.width, 310);
        assert_eq!(l.page, 1 + 8, "the header and eight whole rows");
    }

    #[test]
    fn picking_a_behaviour_changes_only_the_selected_item() {
        let mut t = sample(Wording::Classic);
        let text = text();
        assert_eq!(t.click(Some(Hit::Row(0))), Event::Redraw);
        let l = t.layout(&text, 11, (0, 0));
        let field = l.combo.as_ref().unwrap().1.field;
        t.click(t.hit(&l, field.x + 5, field.y + 5));
        assert_eq!(t.combo.open, Some(0), "the list opens on the pick");
        let l = t.layout(&text, 11, (0, 0));
        let list = l.combo.as_ref().unwrap().1.list;
        let hit = t.hit(&l, list.x + 5, list.y + 1 + 2 * 13 + 3);
        assert_eq!(hit, Some(Hit::Combo(combo::Hit::Row(2))));
        t.click(hit);
        assert_eq!(t.entries[0].behaviour, Behaviour::Show);
        assert_eq!(t.entries[1].behaviour, Behaviour::Auto);
        assert_eq!(t.click(Some(Hit::Ok)), Event::Redraw);
        let Event::Accept(ch) = t.release(Some(Hit::Ok)) else {
            panic!("OK accepts");
        };
        assert_eq!(ch.behaviours[0], ("nm-applet".into(), Behaviour::Show));
        assert_eq!(ch.behaviours[3], ("sticky".into(), Behaviour::Hide));
        assert_eq!(ch.all_icons, None);
    }

    #[test]
    fn re_picking_the_row_an_auto_item_shows_keeps_it_auto() {
        let mut t = sample(Wording::Glass);
        t.select(1);
        assert_eq!(t.combo.selected, 2);
        t.take_pick(2);
        assert_eq!(t.entries[1].behaviour, Behaviour::Auto);
        t.take_pick(1);
        assert_eq!(t.entries[1].behaviour, Behaviour::Mute);
    }

    #[test]
    fn restore_defaults_puts_every_item_back_on_its_status() {
        let mut t = sample(Wording::Classic);
        assert_eq!(t.key(form::Key::Access('r')), Event::Redraw);
        assert!(t.entries.iter().all(|e| e.behaviour == Behaviour::Auto));
        assert_eq!(t.key(form::Key::Escape), Event::Cancel);
    }

    #[test]
    fn the_keyboard_walks_the_rows_and_steps_the_combo() {
        let mut t = sample(Wording::Classic);
        assert_eq!(t.key(form::Key::Down), Event::Redraw);
        assert_eq!(t.selected, Some(0));
        t.key(form::Key::End);
        assert_eq!(t.selected, Some(3), "into the past items");
        t.key(form::Key::Tab { back: false });
        assert_eq!(t.focus, Focus::Combo);
        t.key(form::Key::Up);
        assert_eq!(
            t.entries[3].behaviour,
            Behaviour::Auto,
            "Always hide to Hide when inactive"
        );
        t.key(form::Key::Tab { back: false });
        assert_eq!(t.focus, Focus::Restore, "Classic has no check box");
        assert_eq!(t.key(form::Key::Enter), Event::Redraw, "Enter restores");
        assert!(t.entries.iter().all(|e| e.behaviour == Behaviour::Auto));
    }

    #[test]
    fn glass_ticks_its_box_and_greys_the_list() {
        let mut t = sample(Wording::Glass);
        let text = text();
        let theme = glass_theme();
        let cm = theme.look.controls();
        let l = t.layout(&text, theme.fonts.size_px, (0, 0));
        assert_eq!(l.check, Some(Rect::new(92, 501, cm.check, cm.check)));
        t.click(Some(Hit::AllIcons));
        t.release(Some(Hit::AllIcons));
        assert_eq!(t.all_icons, Some(true));
        assert!(!t.list_enabled());
        assert_eq!(t.click(Some(Hit::Row(0))), Event::None, "greyed");
        let Event::Accept(ch) = t.key(form::Key::Enter) else {
            panic!("Enter is OK");
        };
        assert_eq!(ch.all_icons, Some(true));
    }

    /// Ink rows and columns of `c` inside `r` that differ from `bg`.
    fn ink(c: &Canvas, r: Rect, bg: u32) -> Option<(i32, i32, i32, i32)> {
        let mut out: Option<(i32, i32, i32, i32)> = None;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if c.get(x, y) != bg {
                    out = Some(match out {
                        None => (x, y, x, y),
                        Some((a, b, cc, d)) => (a.min(x), b.min(y), cc.max(x), d.max(y)),
                    });
                }
            }
        }
        out
    }

    #[test]
    fn classic_pixels_sit_where_the_dialog_draws_them() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = text();
        let px = theme.fonts.size_px;
        let mut t = sample(Wording::Classic);
        t.select(1);
        let l = t.layout(&text, px, (0, 0));
        let mut c = Canvas::new(348, 391);
        t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
        let face = cs.button_face.argb();
        let white = cs.window.argb();
        assert_eq!(c.get(100, 82), cs.button_shadow.argb());
        assert_eq!(c.get(100, 83), cs.button_dk_shadow.argb());
        assert_eq!(
            c.get(100, 84),
            face,
            "the header's top hides under the frame"
        );
        assert_eq!(c.get(100, 99), cs.button_shadow.argb());
        assert_eq!(c.get(100, 100), white);
        assert_eq!(c.get(195, 90), cs.button_shadow.argb());
        assert_eq!(c.get(196, 90), cs.button_hilight.argb());
        assert_eq!(c.get(11, 90), face, "no hilight on the first column");
        assert_eq!(
            c.get(11, 119),
            crate::theme::Color::rgb(166, 202, 240).argb()
        );
        assert_eq!(c.get(11, 118), white);
        assert_eq!(c.get(31, 150), cs.hilight.argb());
        assert_eq!(c.get(31, 166), cs.hilight.argb());
        assert_eq!(c.get(31, 167), white);
        assert_ne!(c.get(30, 150), cs.hilight.argb(), "the icon's last column");
        assert_eq!(c.get(11 + 186, 158), cs.button_shadow.argb(), "combo frame");
        assert_eq!(c.get(15, 132), 0xFFFF_0000);
        assert_eq!(c.get(14, 140), white);
        assert_eq!(c.get(246, 307), cs.button_hilight.argb());
        assert_eq!(c.get(338, 329), cs.button_dk_shadow.argb());
        assert_eq!(c.get(183, 359), cs.window_frame.argb(), "default ring");
        if !crate::text::test_tahoma() {
            return;
        }
        let intro = ink(&c, Rect::new(9, 0, 330, 30), face).unwrap();
        assert_eq!(intro.1, 13, "intro ink top");
        let prompt = ink(&c, Rect::new(9, 62, 330, 16), face).unwrap();
        assert_eq!(prompt.1, 68, "prompt ink top");
        let title = ink(&c, Rect::new(11, 101, 100, 17), white).unwrap();
        assert_eq!((title.0, title.1), (26, 106), "group title ink corner");
        let name = ink(&c, Rect::new(32, 132, 150, 17), white).unwrap();
        assert_eq!(name.1, 136, "a row's name ink top");
        assert_eq!(name.0, 11 + 22, "a row's name ink left");
    }

    #[test]
    fn the_glass_page_has_its_parts_where_the_page_puts_them() {
        let t = sample(Wording::Glass);
        let text = text();
        let theme = glass_theme();
        let l = t.layout(&text, theme.fonts.size_px, (0, 0));
        assert_eq!(t.client(), (1184, 575));
        assert_eq!(l.frame, Rect::new(92, 84, 1000, 376));
        assert_eq!(l.vbar, Some(Rect::new(1074, 85, 17, 374)), "always there");
        assert_eq!(l.body, Rect::new(93, 132, 981, 327));
        let ys: Vec<_> = l.slots.iter().map(|&(s, r)| (s, r.y, r.height)).collect();
        assert_eq!(
            ys,
            [
                (Slot::Row(0), 132, 48),
                (Slot::Row(1), 180, 48),
                (Slot::Row(2), 228, 48),
                (Slot::Row(3), 276, 48)
            ]
        );
        let fields: Vec<_> = l.fields.iter().map(|&(i, r)| (i, r)).collect();
        assert_eq!(fields[0], (0, Rect::new(584, 140, 279, 23)));
        assert_eq!(fields[3], (3, Rect::new(584, 284, 279, 23)));
        assert_eq!(l.page, 4, "every row shows");
        assert_eq!(
            l.combo.as_ref().map(|c| c.0),
            Some(0),
            "the first is focused"
        );
        assert_eq!(l.ok, Rect::new(934, 542, 76, 23));
        assert_eq!(l.cancel, Rect::new(1016, 542, 76, 23));
        assert_eq!(l.band, Some(Rect::new(0, 532, 1184, 43)));
        assert_eq!((l.restore.x, l.restore.y), (92, 470));
        assert_eq!(t.focus, Focus::Combo);
    }

    /// The page drawn in `t`'s state, with its layout.
    fn page_pixels(t: &TrayCustomize) -> (Canvas, Layout, Theme) {
        let theme = glass_theme();
        let text = text();
        let px = theme.fonts.size_px;
        let l = t.layout(&text, px, (0, 0));
        let (w, h) = t.client();
        let mut c = Canvas::new(w as u32, h as u32);
        t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
        (c, l, theme)
    }

    #[test]
    fn the_glass_page_pixels_sit_where_the_page_draws_them() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (c, _, theme) = page_pixels(&sample(Wording::Glass));
        let inks = theme.look.style().and_then(|st| st.page).unwrap();
        let (cs, white) = (&theme.colors, theme.colors.window.argb());
        let at = |r: Rect, bg: u32| ink(&c, r, bg).map(|(x, y, ..)| (x, y));
        assert_eq!(at(Rect::new(80, 10, 900, 30), white), Some((93, 23)));
        assert!(any(&c, Rect::new(80, 10, 900, 30), inks.heading.argb()));
        assert_eq!(at(Rect::new(80, 45, 1000, 25), white), Some((92, 57)));
        assert_eq!(c.get(92, 84), inks.list_frame.argb());
        assert_eq!(c.get(1091, 459), inks.list_frame.argb());
        assert_eq!(c.get(93, 85), white);
        assert_eq!(at(Rect::new(100, 95, 100, 20), white), Some((127, 106)));
        assert_eq!(
            at(Rect::new(560, 95, 100, 20), white).map(|p| p.0),
            Some(586)
        );
        assert_eq!(c.get(125, 118), inks.rule.argb());
        assert_eq!(c.get(1041, 118), inks.rule.argb());
        assert_eq!((c.get(124, 118), c.get(1042, 118)), (white, white));
        assert_eq!(
            (c.get(133, 137), c.get(148, 152)),
            (0xFFFF_0000, 0xFFFF_0000)
        );
        assert_eq!(c.get(132, 137), white);
        assert_eq!(at(Rect::new(150, 135, 300, 12), white), Some((159, 140)));
        assert_eq!(at(Rect::new(150, 151, 300, 14), white), Some((159, 155)));
        assert!(any(&c, Rect::new(150, 151, 300, 14), inks.detail.argb()));
        for y in [140, 188, 236, 284] {
            assert_eq!(c.get(700, y), 0xFF6D_6D6D, "combo top at {y}");
            assert_eq!(c.get(700, y + 22), 0xFF6D_6D6D, "combo foot at {y}");
        }
        assert_eq!(at(Rect::new(80, 470, 300, 20), white), Some((92, 475)));
        assert!(any(&c, Rect::new(80, 470, 300, 20), inks.link.argb()));
        assert_ne!(c.get(93, 502), white, "the box");
        assert_eq!(at(Rect::new(105, 495, 300, 20), white), Some((111, 502)));
        assert_eq!(c.get(500, 531), white);
        assert_eq!(c.get(500, 532), inks.rule.argb());
        assert_eq!(
            (c.get(500, 533), c.get(500, 574)),
            (cs.button_face.argb(), cs.button_face.argb())
        );
        let face = cs.button_face.argb();
        assert_eq!(
            at(Rect::new(900, 535, 284, 40), face).map(|p| p.1),
            Some(543)
        );
        assert_eq!(
            ink(&c, Rect::new(900, 535, 284, 40), face).map(|b| (b.0, b.2)),
            Some((935, 1090))
        );
    }

    /// Whether `c` has any pixel of `argb` in `r`.
    fn any(c: &Canvas, r: Rect, argb: u32) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == argb))
    }

    #[test]
    fn the_glass_box_greys_the_combos_and_the_link_but_not_the_rows() {
        let mut t = sample(Wording::Glass);
        t.all_icons = Some(true);
        let (c, l, theme) = page_pixels(&t);
        let inks = theme.look.style().and_then(|st| st.page).unwrap();
        let white = theme.colors.window.argb();
        assert_eq!(c.get(700, 140), 0xFFAB_AFB3);
        assert_eq!(c.get(700, 150), 0xFFF3_F4F4);
        assert!(any(&c, Rect::new(584, 140, 200, 23), 0xFF6B_6B6B));
        assert!(any(&c, l.restore, inks.detail.argb()), "the link greys");
        assert!(!any(&c, l.restore, inks.link.argb()));
        assert!(any(
            &c,
            Rect::new(150, 135, 300, 12),
            theme.colors.window_text.argb()
        ));
        assert_eq!(c.get(133, 137), 0xFFFF_0000);
        assert_eq!(c.get(93, 85), white);
        assert_eq!(
            t.click(Some(Hit::Row(1))),
            Event::None,
            "the combos are greyed"
        );
        assert_eq!(t.click(Some(Hit::Restore)), Event::None, "so is the link");
    }

    #[test]
    fn the_glass_pointer_lights_a_combo_and_the_link() {
        let mut t = sample(Wording::Glass);
        assert_eq!(t.motion(Some(Hit::Row(2))), Event::Redraw);
        assert_eq!(t.motion(Some(Hit::Row(2))), Event::None, "no change");
        let (c, ..) = page_pixels(&t);
        assert_eq!(c.get(750, 236), 0xFF3B_77AD);
        assert_eq!(c.get(750, 238), 0xFFE9_F5FD);
        assert_eq!(c.get(750, 188), 0xFF6D_6D6D);
        t.motion(Some(Hit::Restore));
        let (c, l, theme) = page_pixels(&t);
        let inks = theme.look.style().and_then(|st| st.page).unwrap();
        assert_eq!(c.get(750, 236), 0xFF6D_6D6D, "back at rest");
        assert!(any(&c, l.restore, inks.link_hot.argb()));
        let r = l.restore;
        let row = r.y + 15;
        assert!(
            (r.x..r.right()).all(|x| c.get(x, row) == inks.link_hot.argb()),
            "underline"
        );
        assert_eq!(t.leave(), Event::Redraw);
    }

    #[test]
    fn a_glass_combo_opens_under_a_click_and_tab_walks_the_combos() {
        let mut t = sample(Wording::Glass);
        let text = text();
        let px = glass_theme().fonts.size_px;
        let l = t.layout(&text, px, (0, 0));
        assert_eq!(t.hit(&l, 700, 200), Some(Hit::Row(1)), "row 1's combo");
        assert_eq!(t.hit(&l, 300, 200), Some(Hit::List), "beside it");
        assert_eq!(t.click(Some(Hit::Row(1))), Event::Redraw);
        assert_eq!((t.selected, t.combo.open), (Some(1), Some(2)));
        let (c, ..) = page_pixels(&t);
        assert_eq!(
            c.get(750, 188),
            0xFF2B_5B87,
            "pressed while its list is open"
        );
        let over = Some(Hit::Combo(combo::Hit::Row(0)));
        assert_eq!(t.motion(over), Event::Redraw);
        assert_eq!(t.combo.open, Some(0), "the highlight follows the pointer");
        t.key(form::Key::Escape);
        assert_eq!(t.combo.open, None);
        t.key(form::Key::Tab { back: false });
        assert_eq!((t.focus, t.selected), (Focus::Combo, Some(2)));
        t.key(form::Key::Tab { back: false });
        t.key(form::Key::Tab { back: false });
        assert_eq!(t.focus, Focus::AllIcons, "past the last combo");
        t.key(form::Key::Tab { back: true });
        assert_eq!((t.focus, t.selected), (Focus::Combo, Some(3)));
        t.key(form::Key::Up);
        assert_eq!(t.entries[3].behaviour, Behaviour::Mute, "Up and Down pick");
    }

    #[test]
    fn the_glass_list_scrolls_a_row_at_a_time() {
        let many: Vec<_> = (0..9)
            .map(|i| {
                entry(
                    &format!("k{i}"),
                    &format!("Item {i}"),
                    false,
                    Behaviour::Auto,
                )
            })
            .collect();
        let mut t = TrayCustomize::new(
            Wording::Glass,
            many,
            vec![],
            Some(false),
            metrics(Wording::Glass),
        );
        assert!(t.scroll(1));
        assert!(t.scroll(5));
        assert_eq!(t.top, 3, "nine rows, six to a page");
        assert!(!t.scroll(1));
        let l = t.layout(&text(), glass_theme().fonts.size_px, (0, 0));
        assert_eq!(l.fields[0], (3, Rect::new(584, 140, 279, 23)));
    }

    #[test]
    #[ignore]
    fn preview() {
        let text = text();
        for (name, theme, w) in [
            ("classic", Theme::default(), Wording::Classic),
            ("glass", glass_theme(), Wording::Glass),
        ] {
            let px = theme.fonts.size_px;
            let mut t = sample(w);
            t.select(1);
            let l = t.layout(&text, px, (0, 0));
            let (cw, ch) = t.client();
            let mut c = Canvas::new(cw as u32, ch as u32);
            t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
            crate::raster::write_preview(&c, &format!("traycustomize-{name}-preview.png"));
        }
        let theme = glass_theme();
        let px = theme.fonts.size_px;
        let draw = |t: &TrayCustomize, name: &str| {
            let l = t.layout(&text, px, (0, 0));
            let (cw, ch) = t.client();
            let mut c = Canvas::new(cw as u32, ch as u32);
            t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
            crate::raster::write_preview(&c, &format!("traycustomize-glass-{name}.png"));
        };
        let mut t = sample(Wording::Glass);
        t.motion(Some(Hit::Row(2)));
        t.pointer.motion(Some(Focus::Restore));
        draw(&t, "hot");
        t.click(Some(Hit::Row(0)));
        draw(&t, "open");
        let mut t = sample(Wording::Glass);
        t.all_icons = Some(true);
        draw(&t, "greyed");
    }
}
