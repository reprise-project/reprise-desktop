//! A dialog page as data: controls at fixed rects with one focus, Tab order, access keys,
//! and the pointer and key handling every dialog shares. The host feeds it keys and hits and
//! reads back `Event`s.

use std::cell::{Cell, Ref, RefCell};

use crate::art::clockface;
use crate::classic::widgets::{self, Arrow, LINE_H, ScrollHit};
use crate::controls::arrange;
use crate::controls::calendar::{self, Calendar};
use crate::controls::colorbutton::ColorButton;
use crate::controls::combo::{self, Combo};
use crate::controls::hotkey::HotKey;
use crate::controls::iconlist;
use crate::controls::listbox::{self, ListBox};
use crate::controls::metrics::ControlMetrics;
use crate::controls::multiedit;
use crate::controls::scrollbar::ThumbDrag;
use crate::controls::timefield::{self, TimeField};
use crate::controls::treeview::{self, TreeView};
use crate::controls::updown::{self, UpDown};
use crate::look::{Look, Paint};
use crate::pointer::Pointer;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont};
use crate::textedit::TextEdit;
use crate::theme::{TextUnits, Theme};
use crate::trackbar::{self, TrackBar};

pub(crate) use draw::greyed_field_scheme;
pub use draw::{draw, draw_popups, draw_tip};
use field::FieldDrop;
pub use field::FocusedEdit;
pub(crate) use field::visible_from;
pub use item::{Indent, Item, Screen};
pub use keys::{Key, ctrl_key, flat_key, shove_dir, special_key};
use tip::LabelTip;

mod access_tree;
mod draw;
mod field;
mod focus;
mod hosted;
mod item;
mod keys;
mod layout;
mod pointer;
mod state;
mod tip;

/// A check box item's rect from [`Item::check`], 13 px square.
pub const CHECK: i32 = 13;
/// A radio item's rect from [`Item::radio`], 12 px square.
pub const RADIO_ROW: i32 = 12;

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// One line of static text with its access key underlined; greyed
    /// when not enabled (its access key then does nothing).
    Label {
        text: String,
        enabled: bool,
        align: Align,
    },
    /// A note: its lines, rewrapped to its width when one is wider ([`Form::flowed`]).
    Lines(Vec<String>),
    /// A note drawn greyed, as [`Item::enabled`] leaves a [`Control::Lines`] it greys.
    GreyedLines(Vec<String>),
    Group(String),
    /// A group box title set on its own, for a title the group's 10 px inset does not place;
    /// drawn over a [`Control::Blank`] break in the group's top line.
    GroupTitle(String),
    Separator,
    /// A solid `button_shadow` rule, for where no etched divider fits (2 px wide, 16 tall).
    Rule,
    /// The rect filled with what sits behind it: the gap a control on a group box's top line
    /// punches in it. Listed after the group and before that control.
    Blank,
    /// A check box. `mixed` is `None` for a two-state box; a box with a
    /// third, indeterminate state holds `Some(true)` while it is in it
    /// (`checked` then reads false), and a click walks it indeterminate,
    /// off, on, indeterminate ([`CheckState::cycled`](crate::classic::widgets::CheckState::cycled)).
    Check {
        label: String,
        checked: bool,
        enabled: bool,
        mixed: Option<bool>,
    },
    /// A radio button. Radios sharing a `group` are exclusive; arrow keys walk within the
    /// group and stop at its ends.
    Radio {
        label: String,
        group: &'static str,
        checked: bool,
        enabled: bool,
    },
    /// A single-line edit. `first` is the draw-time scroll offset (byte index), kept by `draw`;
    /// a [`TextEdit::secret`] draws, measures and hit-tests bullets instead of the value.
    Edit {
        edit: TextEdit,
        first: Cell<usize>,
        enabled: bool,
    },
    Combo(Combo),
    ListBox {
        list: ListBox,
        /// Parallel to the items, 16 px.
        icons: Vec<Option<Canvas>>,
    },
    Tree(TreeView),
    UpDown(UpDown),
    TrackBar(TrackBar),
    /// A push button. Enter and Space press it; arrows move focus among push buttons, stopping at the ends.
    Button {
        label: String,
        enabled: bool,
        default: bool,
    },
    /// A bitmap blitted at the rect's top-left inside a sunken frame
    /// `frame` px thick (0 for none).
    Picture {
        canvas: Option<Canvas>,
        frame: i32,
    },
    /// The monitor bitmap with `screen` in its screen: a colour, a
    /// bitmap (scaled to fit) or nothing (black).
    Monitor(Screen),
    /// The Color quality hue bar, [`assets::spectrum`], in a thin sunken frame, drawn at the form's scale.
    ///
    /// [`assets::spectrum`]: crate::classic::assets::spectrum
    Spectrum,
    /// The Appearance preview or Themes sample in `theme`, with the Recycle Bin `icon` on the
    /// Themes sample; the preview fills the item's rect inside the 2 px edge, at least 346x184.
    Sample {
        themes: bool,
        theme: Theme,
        icon: Option<Canvas>,
    },
    Color(ColorButton),
    /// The month grid, [`calendar::Calendar`]; the arrows move the day.
    Calendar(Calendar),
    Clock(clockface::Face),
    Time(TimeField),
    /// The hot-key field, [`hotkey::HotKey`](HotKey). The host hands it raw keys
    /// through [`Form::hotkey_key`] before any dialog reading of them.
    HotKey(HotKey),
    Memo(Box<Memo>),
    /// A Change Icon box's list of icons, [`iconlist::IconList`]; the
    /// arrows and a click move the pick, which reports `Changed`.
    IconList(Box<iconlist::IconList>),
    /// The monitor arrangement well, [`arrange::Arrange`]: a press picks
    /// a tile and a drag moves it, as Ctrl+arrows do ([`Form::shove`]); the
    /// drop and the arrows report `Changed`.
    Arrange(Box<arrange::Arrange>),
}

/// A form's multi-line edit ([`multiedit::MultiEdit`]) with an always-present vertical
/// scrollbar. A form has no text renderer at key time, so Up and Down keep the caret's
/// character column rather than its x.
#[derive(Clone, Debug, PartialEq)]
pub struct Memo {
    pub edit: multiedit::MultiEdit,
    pub view: Cell<multiedit::View>,
    pub enabled: bool,
    /// Where the caret was when `draw` last scrolled to it; the view follows only once it moves.
    followed: Cell<Option<multiedit::Pos>>,
    /// The lines the last draw fit in the box, which the wheel scrolls
    /// against: a form has no text renderer at wheel time to lay it out.
    rows: Cell<usize>,
}

impl Memo {
    pub fn new(text: &str, enabled: bool) -> Memo {
        Memo {
            edit: multiedit::MultiEdit::new(text),
            view: Cell::new(multiedit::View::default()),
            enabled,
            followed: Cell::new(None),
            rows: Cell::new(1),
        }
    }

    /// How a memo lays its text out at font size `px`: the dialog line
    /// pitch grown by `u`, the vertical scrollbar always, the look's bar sizes `cm`.
    pub fn metrics(px: u32, cm: &ControlMetrics, u: TextUnits) -> multiedit::Metrics {
        multiedit::Metrics {
            font_px: px,
            line_h: u.len(LINE_H),
            bars_always: false,
            vbar_always: true,
            bar: cm.scroll,
            thumb_min: cm.thumb_min,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub items: Vec<Item>,
    /// The focused item. Every move goes through [`Form::set_focus`] (or
    /// `click`), since a spinner edits only while it has the focus.
    focus: Option<usize>,
    pub drag: Option<Drag>,
    /// The part the pointer rests on or holds. A held push button draws
    /// pushed only while the pointer is on it, and acts only if the
    /// release lands on it.
    pointer: Pointer<(usize, Light)>,
    /// Where the pointer last moved on the form; `None` once it left.
    point: Option<(i32, i32)>,
    /// The caret blink's shown half. reprisekit has no timer; the host flips
    /// this and redraws.
    pub caret: bool,
    pub font: DialogFont,
    /// Whether the form's left-aligned labels and lines are laid out at
    /// the column their first glyph inks in, rather than at their pen:
    /// in a font with a [`TextRenderer::lead`](crate::text::TextRenderer::lead) each starts that many
    /// columns left of its rect.
    pub labels_at_ink: bool,
    /// A drag of text resting on an edit field, from [`Form::drag_over`].
    field_drop: Option<FieldDrop>,
    /// Where the form's (0, 0) sits on the surface it is drawn on. The items' rects are laid out
    /// for 100% text from here; this offset does not grow with the text.
    pub origin: (i32, i32),
    /// The part of the surface a form that can scroll shows through: a sheet's page.
    pub view: Option<View>,
    tip: LabelTip,
    rendered: Rendered,
    /// A host shows the open combo list or colour popup on a popup ([`Hosted`](crate::controls::hosted::Hosted)): `draw` leaves
    /// it out.
    hosted: bool,
}

/// A scrolled form's window onto the surface: every item but those in `keep` draws and takes
/// the pointer only inside `rect`, laid out as an item's is (for 100% text, from the origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub rect: Rect,
    pub keep: &'static [&'static str],
}

/// What the last `draw` left for the next: canvases rendered from a control's inputs, reused
/// when the inputs repeat (a caret blink), and the look's control sizes that `hit`, `access`
/// and the keys lay out with. Not form state: compares equal and debug-prints as nothing.
#[derive(Clone, Default)]
struct Rendered {
    entries: RefCell<Vec<Rendering>>,
    controls: Cell<ControlMetrics>,
    /// The last draw's text size ([`Paint::text`]), which the items' rects grow by.
    text: Cell<TextUnits>,
    /// Per item, how far the last draw moved its left edge and widened it for text wider
    /// than its rect ([`fit::fit`](crate::controls::fit::fit)); empty while every text fits.
    fit: RefCell<Vec<(i32, i32)>>,
    /// The last draw's scale, 0 before the first: a memo keeps its
    /// horizontal scroll in device px, which `hit` and `access` read at 1x.
    scale: Cell<i32>,
    /// The last draw's text px.
    px: Cell<u32>,
    /// Whether the last draw's look lights parts under the pointer
    /// ([`crate::look::Look::lights_pointer`]); any other lights only a
    /// held push button.
    styled: Cell<bool>,
    /// The notes the last draw found taller than their rects, by item, and by how much at
    /// 100%: their rewrapped lines run below, so [`Form::flowed`] moves what lies under them.
    flow: RefCell<Vec<(usize, i32)>>,
    /// The labels the last draw ended in an ellipsis, by item.
    ellipsized: RefCell<Vec<usize>>,
    #[cfg(test)]
    renders: Cell<usize>,
}

#[derive(Clone)]
struct Rendering {
    item: usize,
    scale: i32,
    size: (i32, i32),
    from: Control,
    canvases: Vec<Canvas>,
}

impl Rendered {
    /// What `make` renders for item `item` from `from` at scale `s` and
    /// logical `size`, rendered again only when that control, scale or
    /// size differs from the last draw's ([`same_render`]).
    fn get(
        &self,
        item: usize,
        s: i32,
        size: (i32, i32),
        from: &Control,
        make: impl FnOnce() -> Vec<Canvas>,
    ) -> Ref<'_, [Canvas]> {
        let mut entries = self.entries.borrow_mut();
        let k = match entries.iter().position(|e| e.item == item) {
            Some(k)
                if entries[k].scale == s
                    && entries[k].size == size
                    && same_render(&entries[k].from, from) =>
            {
                k
            }
            found => {
                #[cfg(test)]
                self.renders.set(self.renders.get() + 1);
                let e = Rendering {
                    item,
                    scale: s,
                    size,
                    from: from.clone(),
                    canvases: make(),
                };
                match found {
                    Some(k) => {
                        entries[k] = e;
                        k
                    }
                    None => {
                        entries.push(e);
                        entries.len() - 1
                    }
                }
            }
        };
        drop(entries);
        Ref::map(self.entries.borrow(), |e| e[k].canvases.as_slice())
    }
}

/// Whether `a` renders what `b` does: equal controls, and for a sample
/// the same style art, which `Look`'s equality (by name) cannot tell
/// apart: a Glass sample in another colour or intensity has the same name.
fn same_render(a: &Control, b: &Control) -> bool {
    a == b
        && match (a, b) {
            (Control::Sample { theme: x, .. }, Control::Sample { theme: y, .. }) => {
                match (&x.look, &y.look) {
                    (Look::Styled(p), Look::Styled(q)) => std::sync::Arc::ptr_eq(p, q),
                    _ => true,
                }
            }
            _ => true,
        }
}

impl PartialEq for Rendered {
    fn eq(&self, _: &Rendered) -> bool {
        true
    }
}

impl std::fmt::Debug for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Rendered")
    }
}

/// A thumb held by the pointer: the pointer drives it until release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    TrackBar(usize),
    /// The scrollbar thumb of a list box, a tree view or an open combo's
    /// drop list, by item index.
    Bar {
        item: usize,
        thumb: ThumbDrag,
    },
    /// An edit field, by item index: the pointer is sweeping a selection
    /// through its value.
    Edit(usize),
    /// A memo's scrollbar thumb, by item index, with the bar it runs
    /// along as the press laid it out: `motion` has no `TextRenderer`.
    Memo {
        item: usize,
        thumb: ThumbDrag,
        span: multiedit::BarSpan,
    },
    Arrange(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Changed(&'static str),
    Pressed(&'static str),
    /// Enter with no button focused: the default button.
    Default,
    Cancel,
}

/// The part of a control the pointer lights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Body,
    /// A combo's drop button, lit from a drop-down list's field too.
    Drop,
    Field,
    Spin(Arrow),
    Thumb,
    Bar(ScrollHit),
}

impl Light {
    /// A thumb stays pressed until the release, wherever the pointer goes.
    fn sticky(self) -> bool {
        matches!(self, Light::Thumb | Light::Bar(ScrollHit::Thumb))
    }

    /// Whether `control` still has a part this lights: what a carried-over
    /// hot or held part is checked against before it lands on a rebuilt
    /// item, so it never lands on a control of the wrong kind.
    fn offered_by(self, control: &Control) -> bool {
        matches!(
            (control, self),
            (
                Control::Button { .. }
                    | Control::Check { .. }
                    | Control::Radio { .. }
                    | Control::Edit { .. }
                    | Control::HotKey(_)
                    | Control::Memo(_),
                Light::Body
            ) | (
                Control::Combo(_),
                Light::Drop | Light::Field | Light::Bar(_)
            ) | (Control::ListBox { .. } | Control::Tree(_), Light::Bar(_))
                | (Control::UpDown(_) | Control::Time(_), Light::Spin(_))
                | (Control::TrackBar(_), Light::Thumb)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Body,
    /// An edit field, with the byte index of the boundary under the pointer, resolved by `hit`
    /// while the scroll offset is to hand.
    Edit(usize),
    Combo(combo::Hit),
    List(listbox::Hit),
    Tree(treeview::Hit),
    UpDown(updown::Hit),
    TrackBar(trackbar::Hit),
    Swatch(usize),
    Calendar(calendar::Hit),
    Time(timefield::Hit),
    Popup,
    Memo(multiedit::Pos),
    /// A press on part `hit` of a memo's scrollbar: the view an arrow or
    /// the track scrolls to (in the device px the draw scrolls in), and
    /// the bar a grabbed thumb is then dragged along.
    MemoBar {
        to: multiedit::View,
        hit: widgets::BarHit,
        span: multiedit::BarSpan,
    },
    IconList(iconlist::Hit),
    Tile {
        tile: usize,
        at: (i32, i32),
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub item: usize,
    pub part: Part,
    pub x: i32,
}
impl Form {
    pub fn set_in(mut self, font: DialogFont) -> Form {
        self.font = font;
        self
    }

    pub fn labels_at_ink(mut self) -> Form {
        self.labels_at_ink = true;
        self
    }

    pub fn new(items: Vec<Item>) -> Form {
        let mut form = Form {
            items,
            focus: None,
            drag: None,
            pointer: Pointer::new(),
            point: None,
            caret: true,
            font: DialogFont::Ui,
            labels_at_ink: false,
            field_drop: None,
            origin: (0, 0),
            view: None,
            tip: LabelTip::default(),
            rendered: Rendered::default(),
            hosted: false,
        };
        let first = (0..form.items.len()).find(|&i| form.tab_stop(i));
        form.set_focus(first);
        form
    }

    /// Keep `p`'s look's control sizes and text size as the ones `hit`, `access` and
    /// the keys lay the controls out with. `draw` does this itself; a
    /// host that draws a copy of the form (shifted, refocused), or hits or reads
    /// [`Form::rect`] on a form not drawn yet, calls it first.
    pub fn drawn_with(&self, p: &Paint) {
        self.rendered.controls.set(p.look.controls().grown(p.text));
        self.rendered.text.set(p.text);
        self.rendered.scale.set(p.s);
        self.rendered.styled.set(p.look.lights_pointer());
    }

    /// A memo's view as `hit` and `access` read it, at 1x: its horizontal
    /// scroll is in the device px of the last draw.
    fn memo_view_1x(&self, memo: &Memo) -> multiedit::View {
        let v = memo.view.get();
        multiedit::View {
            left: v.left / self.rendered.scale.get().max(1),
            ..v
        }
    }

    /// `to`, a view worked out at 1x from [`Form::memo_view_1x`], back in
    /// the device px the draw scrolls in. A horizontal scroll it left
    /// alone keeps its device px, which 1x would round off.
    fn memo_view_device(&self, memo: &Memo, to: multiedit::View) -> multiedit::View {
        let v = memo.view.get();
        let left = if to.left == self.memo_view_1x(memo).left {
            v.left
        } else {
            to.left * self.rendered.scale.get().max(1)
        };
        multiedit::View { left, ..to }
    }

    /// The look's control sizes at the text size of the last draw.
    pub(crate) fn controls(&self) -> ControlMetrics {
        self.rendered.controls.get()
    }

    /// The text size the form was last drawn at ([`Paint::text`]). A host feeding it input
    /// measures at `units().px` of its 100% font size, as the draw did.
    pub fn units(&self) -> TextUnits {
        self.rendered.text.get()
    }
}

#[cfg(test)]
mod tests;
