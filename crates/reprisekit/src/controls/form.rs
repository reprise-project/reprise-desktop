//! A dialog page as data: controls at fixed rects with one focus, Tab order, access keys,
//! and the pointer and key handling every dialog shares. The host feeds it keys and hits and
//! reads back `Event`s.

use std::cell::{Cell, Ref, RefCell};
use zeroize::Zeroize;

use crate::access::{self, Tree, accesskit};
use crate::art::clockface;
use crate::classic::widgets::{
    self, Arrow, Axis, CheckState, LINE_H, ScrollHit, draw_edit, draw_focus_ring, draw_label,
    edit_first_visible,
};
use crate::controls::arrange;
use crate::controls::calendar::{self, Calendar};
use crate::controls::colorbutton::{self, ColorButton};
use crate::controls::combo::{self, Combo};
use crate::controls::hotkey::{self, HotKey};
use crate::controls::iconlist;
use crate::controls::listbox::{self, ListBox};
use crate::controls::metrics::ControlMetrics;
use crate::controls::multiedit;
use crate::controls::scrollbar::ThumbDrag;
use crate::controls::timefield::{self, TimeField};
use crate::controls::treeview::{self, TreeView};
use crate::controls::updown::{self, UpDown};
use crate::look::widgets::{
    draw_button, draw_checkbox_state, draw_checkbox_state_disabled, draw_disabled_field,
    draw_disabled_label, draw_edit_field, draw_group_box, draw_radio, draw_radio_disabled,
    draw_trackbar, draw_well, group_label_ink,
};
use crate::look::{Look, Paint};
use crate::pointer::{Interaction, Lit, Pointer, WHEEL_ROWS};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::{Caret, EditKey, TextEdit};
use crate::theme::{Color, ColorScheme, Theme};
use crate::trackbar::{self, TrackBar};
use accesskit::{NodeId, Role};

/// A check box item's rect from [`Item::check`], 13 px square.
pub const CHECK: i32 = 13;
/// A radio item's rect from [`Item::radio`], 12 px square.
const RADIO_ROW: i32 = 12;

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// One line of static text with its access key underlined; greyed
    /// when not enabled (its access key then does nothing).
    Label {
        text: String,
        enabled: bool,
        align: Align,
    },
    Lines(Vec<String>),
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
    /// off, on, indeterminate ([`CheckState::cycled`]).
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
    /// The hot-key field, [`hotkey::HotKey`]. The host hands it raw keys
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
    /// pitch, the vertical scrollbar always, the look's bar sizes `cm`.
    pub fn metrics(px: u32, cm: &ControlMetrics) -> multiedit::Metrics {
        multiedit::Metrics {
            font_px: px,
            line_h: LINE_H,
            bars_always: false,
            vbar_always: true,
            bar: cm.scroll,
            thumb_min: cm.thumb_min,
            ..Default::default()
        }
    }
}

/// A form's focused edit control ([`Form::focused_edit`]): a one-line
/// field or a memo's lines.
#[derive(Clone, Copy, Debug)]
pub enum FocusedEdit<'a> {
    Line(&'a TextEdit),
    Lines(&'a multiedit::MultiEdit),
}

impl FocusedEdit<'_> {
    /// The selection as Copy and Cut hand it to the clipboard: `None`
    /// from a secret field, which never hands its text out.
    pub fn copy_text(&self) -> Option<String> {
        match self {
            FocusedEdit::Line(e) if e.is_secret() => None,
            FocusedEdit::Line(e) => Some(e.selected_text().to_string()),
            FocusedEdit::Lines(e) => Some(e.selected_text()),
        }
    }
}

/// The scheme a greyed field's value is inked in: `None` for a live field or when the look keeps `button_text`.
pub(crate) fn greyed_field_scheme(p: &Paint, enabled: bool) -> Option<ColorScheme> {
    (!enabled)
        .then(|| p.look.style())
        .flatten()
        .and_then(|st| {
            st.text_color(
                crate::style::Part::EditBorder,
                crate::style::State::Disabled,
            )
        })
        .map(|ink| ColorScheme {
            button_text: ink,
            ..p.cs.clone()
        })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Black,
    Color(Color),
    Picture(Canvas),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: &'static str,
    /// Where the host laid the item out; [`Item::placed`] is where it
    /// draws and takes the pointer.
    pub rect: Rect,
    pub control: Control,
    pub indent: Indent,
}

/// How an item's rect is anchored. `UnderCheck`: under a check box's label, starting
/// `check_label` right of the box. `Wraps` and `CheckWrap`: a check box whose label runs onto
/// a second line, and that line (at `check_wrap`); both sit `check_wrap_dy` below their rects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Indent {
    #[default]
    None,
    UnderCheck,
    Wraps,
    CheckWrap,
}

impl Item {
    pub fn new(id: &'static str, rect: Rect, control: Control) -> Item {
        Item {
            id,
            rect,
            control,
            indent: Indent::None,
        }
    }

    pub fn under_check(self) -> Item {
        Item {
            indent: Indent::UnderCheck,
            ..self
        }
    }

    pub fn wraps(self) -> Item {
        Item {
            indent: Indent::Wraps,
            ..self
        }
    }

    pub fn check_wrap(self) -> Item {
        Item {
            indent: Indent::CheckWrap,
            ..self
        }
    }

    /// Where the item draws and takes the pointer at the control sizes
    /// `cm`: its rect, moved by its [`Indent`]. A wrapping check box's
    /// own rect stays put; its label moves ([`check_label_rect`]).
    pub fn placed(&self, cm: &ControlMetrics) -> Rect {
        let (dx, dy) = match self.indent {
            Indent::None | Indent::Wraps => (0, 0),
            Indent::UnderCheck => (cm.check_label, 0),
            Indent::CheckWrap => (cm.check_wrap, cm.check_wrap_dy),
        };
        Rect {
            x: self.rect.x + dx,
            y: self.rect.y + dy,
            ..self.rect
        }
    }

    pub fn label(id: &'static str, rect: Rect, text: &str) -> Item {
        Item::aligned(id, rect, text, Align::Left)
    }

    pub fn label_at(id: &'static str, x: i32, y: i32, w: i32, text: &str) -> Item {
        Item::label(id, Rect::new(x, y, w, LINE_H), text)
    }

    pub fn value_at(id: &'static str, x: i32, y: i32, w: i32, text: &str) -> Item {
        Item::aligned(id, Rect::new(x, y, w, LINE_H), text, Align::Right)
    }

    pub fn aligned(id: &'static str, rect: Rect, text: &str, align: Align) -> Item {
        Item::new(
            id,
            rect,
            Control::Label {
                text: text.into(),
                enabled: true,
                align,
            },
        )
    }

    pub fn lines(id: &'static str, rect: Rect, text: &[&str]) -> Item {
        Item::new(
            id,
            rect,
            Control::Lines(text.iter().map(|s| s.to_string()).collect()),
        )
    }

    pub fn lines_at(id: &'static str, x: i32, y: i32, w: i32, text: &[&str]) -> Item {
        Item::lines(id, Rect::new(x, y, w, LINE_H * text.len() as i32), text)
    }

    pub fn group(id: &'static str, rect: Rect, title: &str) -> Item {
        Item::new(id, rect, Control::Group(title.into()))
    }

    pub fn group_title(id: &'static str, x: i32, y: i32, w: i32, title: &str) -> Item {
        Item::new(
            id,
            Rect::new(x, y, w, LINE_H),
            Control::GroupTitle(title.into()),
        )
    }

    pub fn rule(id: &'static str, rect: Rect) -> Item {
        Item::new(id, rect, Control::Rule)
    }

    /// A bitmap with no frame; `None` draws the magenta placeholder.
    pub fn picture(id: &'static str, rect: Rect, canvas: Option<Canvas>) -> Item {
        Item::new(id, rect, Control::Picture { canvas, frame: 0 })
    }

    pub fn button(id: &'static str, rect: Rect, text: &str, enabled: bool, default: bool) -> Item {
        Item::new(
            id,
            rect,
            Control::Button {
                label: text.into(),
                enabled,
                default,
            },
        )
    }

    /// A push button at `(x, y)`, `w` wide and `BUTTON_H` tall, never the
    /// default one: the dialog around it owns that.
    pub fn button_at(id: &'static str, x: i32, y: i32, w: i32, text: &str, enabled: bool) -> Item {
        let rect = Rect::new(x, y, w, crate::classic::widgets::BUTTON_H);
        Item::button(id, rect, text, enabled, false)
    }

    pub fn check(id: &'static str, x: i32, y: i32, text: &str) -> Item {
        Item::new(
            id,
            Rect::new(x, y, CHECK, CHECK),
            Control::Check {
                label: text.into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        )
    }

    pub fn radio(id: &'static str, x: i32, y: i32, text: &str, group: &'static str) -> Item {
        Item::new(
            id,
            Rect::new(x, y, RADIO_ROW, RADIO_ROW),
            Control::Radio {
                label: text.into(),
                group,
                checked: false,
                enabled: true,
            },
        )
    }

    pub fn combo(id: &'static str, x: i32, y: i32, w: i32, items: &[&str], rows: usize) -> Item {
        Item::new(
            id,
            Rect::new(x, y, w, combo::COMBO_H),
            Control::Combo(Combo::new(items, 0, rows)),
        )
    }

    pub fn memo(id: &'static str, rect: Rect, text: &str, enabled: bool) -> Item {
        Item::new(id, rect, Control::Memo(Box::new(Memo::new(text, enabled))))
    }

    pub fn edit(id: &'static str, rect: Rect, text: &str, enabled: bool) -> Item {
        Item::new(
            id,
            rect,
            Control::Edit {
                edit: TextEdit::new(text, false),
                first: Cell::new(0),
                enabled,
            },
        )
    }

    pub fn checked(mut self, on: bool) -> Item {
        if let Control::Check { checked, .. } | Control::Radio { checked, .. } = &mut self.control {
            *checked = on;
        }
        self
    }

    /// The item live or greyed; a control with no greyed state of its own
    /// here is returned unchanged.
    pub fn enabled(mut self, on: bool) -> Item {
        match &mut self.control {
            Control::Label { enabled, .. }
            | Control::Check { enabled, .. }
            | Control::Radio { enabled, .. }
            | Control::Edit { enabled, .. }
            | Control::Button { enabled, .. } => *enabled = on,
            Control::HotKey(h) => h.enabled = on,
            Control::Memo(m) => m.enabled = on,
            Control::Arrange(a) => a.enabled = on,
            _ => {}
        }
        self
    }

    fn focusable(&self) -> bool {
        match &self.control {
            Control::Check { enabled, .. }
            | Control::Radio { enabled, .. }
            | Control::Button { enabled, .. }
            | Control::Edit { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            Control::UpDown(u) => u.enabled,
            Control::Color(b) => b.enabled,
            Control::TrackBar(t) => t.enabled,
            Control::Calendar(cal) => cal.enabled,
            Control::Time(tf) => tf.enabled,
            Control::HotKey(h) => h.enabled,
            Control::Memo(m) => m.enabled,
            Control::IconList(l) => !l.names.is_empty(),
            Control::Arrange(a) => a.enabled && !a.tiles.is_empty(),
            Control::ListBox { .. } | Control::Tree(_) => true,
            _ => false,
        }
    }

    fn mnemonic(&self) -> Option<char> {
        let label = match &self.control {
            Control::Label { text, enabled, .. } if *enabled => text,
            Control::Check { label, enabled, .. } if *enabled => label,
            Control::Radio { label, enabled, .. } if *enabled => label,
            Control::Button { label, enabled, .. } if *enabled => label,
            _ => return None,
        };
        crate::text::access_key(label)
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
    /// in a font with a [`TextRenderer::lead`] each starts that many
    /// columns left of its rect.
    pub labels_at_ink: bool,
    /// A drag of text resting on an edit field, from [`Form::drag_over`].
    field_drop: Option<FieldDrop>,
    rendered: Rendered,
}

/// The edit field a drag rests on and where, in form px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldDrop {
    item: usize,
    x: i32,
    y: i32,
}

/// What the last `draw` left for the next: canvases rendered from a control's inputs, reused
/// when the inputs repeat (a caret blink), and the look's control sizes that `hit`, `access`
/// and the keys lay out with. Not form state: compares equal and debug-prints as nothing.
#[derive(Clone, Default)]
struct Rendered {
    entries: RefCell<Vec<Rendering>>,
    controls: Cell<ControlMetrics>,
    /// The last draw's scale, 0 before the first: a memo keeps its
    /// horizontal scroll in device px, which `hit` and `access` read at 1x.
    scale: Cell<i32>,
    /// Whether the last draw's look lights parts under the pointer
    /// ([`crate::look::Look::lights_pointer`]); any other lights only a
    /// held push button.
    styled: Cell<bool>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tab {
        back: bool,
    },
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    Enter,
    Escape,
    Backspace,
    Char(char),
    /// Alt+letter: the access key, whichever control has the focus. A
    /// bare `Char` reaches a focused edit field, list or combo first;
    /// this never does (Alt+B on Run presses Browse...).
    Access(char),
    Edit(EditKey),
    Drop,
    /// Ctrl+Enter, which a host reads only for a form: a new line in a
    /// focused memo, and Enter anywhere else.
    NewLine,
    /// Ctrl+Home, or Ctrl+End when `end`, Shift to `select`: a focused
    /// memo's caret to its first or last character, and Home or End
    /// anywhere else.
    DocEdge {
        end: bool,
        select: bool,
    },
}

/// The dialog key a keysym stands for before any editing-key reading, or `None`; one table for every host.
pub fn special_key(sym: u32, shift: bool, alt: bool) -> Option<Key> {
    use crate::textedit::keysym;
    Some(match sym {
        keysym::TAB => Key::Tab { back: shift },
        keysym::ISO_LEFT_TAB => Key::Tab { back: true },
        keysym::RETURN | keysym::KP_ENTER => Key::Enter,
        keysym::ESCAPE => Key::Escape,
        keysym::UP | keysym::KP_UP => Key::Up,
        keysym::DOWN | keysym::KP_DOWN if alt => Key::Drop,
        keysym::DOWN | keysym::KP_DOWN => Key::Down,
        keysym::PAGE_UP | keysym::KP_PAGE_UP => Key::PageUp,
        keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Key::PageDown,
        keysym::F4 => Key::Drop,
        _ => return None,
    })
}

/// The way Ctrl+arrow `sym` moves a focused arrangement's picked tile
/// ([`Form::shove`]), or `None` for any other press.
pub fn shove_dir(sym: u32, ctrl: bool, shift: bool, alt: bool) -> Option<arrange::Dir> {
    use crate::textedit::keysym;
    if !ctrl || shift || alt {
        return None;
    }
    Some(match sym {
        keysym::LEFT | keysym::KP_LEFT => arrange::Dir::Left,
        keysym::RIGHT | keysym::KP_RIGHT => arrange::Dir::Right,
        keysym::UP | keysym::KP_UP => arrange::Dir::Up,
        keysym::DOWN | keysym::KP_DOWN => arrange::Dir::Down,
        _ => return None,
    })
}

/// What `key` stands for with Ctrl held, for a host reading a form:
/// Ctrl+Enter is [`Key::NewLine`] and Ctrl+Home / Ctrl+End are
/// [`Key::DocEdge`], which `textedit::key_for` folds into Home and End.
pub fn ctrl_key(key: Key) -> Key {
    match key {
        Key::Enter => Key::NewLine,
        Key::Edit(EditKey::Home { select }) => Key::DocEdge { end: false, select },
        Key::Edit(EditKey::End { select }) => Key::DocEdge { end: true, select },
        k => k,
    }
}

/// The flat dialog key an editing key stands for when it is not going into an edit field,
/// or `None`. Public so a host with a control of its own beside a form degrades keys the same way.
pub fn flat_key(k: &EditKey) -> Option<Key> {
    Some(match k {
        EditKey::Left { .. } => Key::Left,
        EditKey::Right { .. } => Key::Right,
        EditKey::Home { .. } => Key::Home,
        EditKey::End { .. } => Key::End,
        EditKey::Backspace { .. } => Key::Backspace,
        // Space works the focused control; as Key::Char(' ') it would be taken for an access key.
        EditKey::Insert(s) if s == " " => Key::Space,
        // A one-character insert is an access key or a combo's
        // type-ahead; anything longer (a paste) is not.
        EditKey::Insert(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => return None,
            }
        }
        EditKey::Delete { .. }
        | EditKey::SelectAll
        | EditKey::Copy
        | EditKey::Cut
        | EditKey::Paste
        | EditKey::Undo
        | EditKey::DeleteSurrounding { .. } => return None,
    })
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
            rendered: Rendered::default(),
        };
        let first = (0..form.items.len()).find(|&i| form.tab_stop(i));
        form.set_focus(first);
        form
    }

    /// Keep `p`'s look's control sizes as the ones `hit`, `access` and
    /// the keys lay the controls out with. `draw` does this itself; a
    /// host that draws a copy of the form (shifted, refocused) calls it on
    /// the form it keeps.
    pub(crate) fn drawn_with(&self, p: &Paint) {
        self.rendered.controls.set(p.look.controls());
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

    pub(crate) fn controls(&self) -> ControlMetrics {
        self.rendered.controls.get()
    }

    pub(crate) fn styled(&self) -> bool {
        self.rendered.styled.get()
    }

    pub fn holding(&self) -> bool {
        self.pointer.holding()
    }

    pub fn held_item(&self) -> Option<usize> {
        self.pointer.held().map(|(i, _)| i)
    }

    pub fn lit(&self, i: usize, part: Light) -> Interaction {
        self.pointer.of((i, part))
    }

    fn bar_lit(&self, i: usize) -> Lit<ScrollHit> {
        match self.pointer.lit() {
            Some(((j, Light::Bar(hit)), how)) if j == i => Some((hit, how)),
            _ => None,
        }
    }

    fn spin_lit(&self, i: usize) -> Lit<Arrow> {
        match self.pointer.lit() {
            Some(((j, Light::Spin(a)), how)) if j == i => Some((a, how)),
            _ => None,
        }
    }

    /// The part `hit` lights, if it lights one: a live control's part the
    /// look draws apart under the pointer.
    fn light(&self, hit: Option<Hit>) -> Option<(usize, Light)> {
        let h = hit?;
        let l = match (&self.items.get(h.item)?.control, h.part) {
            (
                Control::Button { enabled: true, .. }
                | Control::Check { enabled: true, .. }
                | Control::Radio { enabled: true, .. },
                Part::Body,
            ) => Light::Body,
            (Control::Edit { enabled: true, .. }, Part::Edit(_)) => Light::Body,
            (Control::HotKey(h), Part::Body) if h.enabled => Light::Body,
            (Control::Memo(m), Part::Memo(_)) if m.enabled => Light::Body,
            (Control::IconList(_), Part::IconList(_)) => Light::Body,
            (Control::Arrange(a), Part::Tile { .. }) if a.enabled => Light::Body,
            (Control::Combo(c), Part::Combo(ch)) if c.enabled => match ch {
                combo::Hit::Field if c.edit.is_some() => Light::Field,
                combo::Hit::Drop | combo::Hit::Field => Light::Drop,
                combo::Hit::ScrollUp => Light::Bar(ScrollHit::Back),
                combo::Hit::ScrollDown => Light::Bar(ScrollHit::Forward),
                combo::Hit::PageUp => Light::Bar(ScrollHit::PageBack),
                combo::Hit::PageDown => Light::Bar(ScrollHit::PageForward),
                combo::Hit::Thumb(_) => Light::Bar(ScrollHit::Thumb),
                combo::Hit::Row(_) => return None,
            },
            (Control::ListBox { .. }, Part::List(lh)) => match lh {
                listbox::Hit::ScrollUp => Light::Bar(ScrollHit::Back),
                listbox::Hit::ScrollDown => Light::Bar(ScrollHit::Forward),
                listbox::Hit::PageUp => Light::Bar(ScrollHit::PageBack),
                listbox::Hit::PageDown => Light::Bar(ScrollHit::PageForward),
                listbox::Hit::Thumb(_) => Light::Bar(ScrollHit::Thumb),
                listbox::Hit::Row(_) | listbox::Hit::Check(_) => return None,
            },
            (Control::Tree(_), Part::Tree(treeview::Hit::VBar(b))) => Light::Bar(b.part()?),
            (Control::UpDown(u), Part::UpDown(uh)) if u.enabled => match uh {
                updown::Hit::Up => Light::Spin(Arrow::Up),
                updown::Hit::Down => Light::Spin(Arrow::Down),
                updown::Hit::Edit(_) => return None,
            },
            (Control::Time(tf), Part::Time(th)) if tf.enabled => match th {
                timefield::Hit::Up => Light::Spin(Arrow::Up),
                timefield::Hit::Down => Light::Spin(Arrow::Down),
                timefield::Hit::Part(_) => return None,
            },
            (Control::TrackBar(t), Part::TrackBar(trackbar::Hit::Thumb)) if t.enabled => {
                Light::Thumb
            }
            _ => return None,
        };
        Some((h.item, l))
    }

    /// What the pointer lights that the last draw's look shows: every lit
    /// part under a style, only a held push button under Classic.
    fn shown(&self) -> Lit<(usize, Light)> {
        let ((i, part), how) = self.pointer.lit()?;
        let control = &self.items.get(i)?.control;
        if !self.rendered.styled.get() {
            let pushed = how == Interaction::Pressed && matches!(control, Control::Button { .. });
            return pushed.then_some(((i, part), how));
        }
        Some(((i, part), how))
    }

    /// The pointer left the form: the part it rested on goes cold. A held
    /// part stays held until the release.
    pub fn leave(&mut self) -> Event {
        let before = self.shown();
        self.point = None;
        self.pointer.leave();
        self.redraw_if(before)
    }

    /// Let go of the pointer outright, held part and drag included: the
    /// page is going out of view (another tab opened) and will not see
    /// the release.
    pub fn cancel_pointer(&mut self) {
        self.pointer.clear();
        self.point = None;
        // A held tile goes back where the press found it.
        if let Some(Drag::Arrange(i)) = self.drag.take()
            && let Some(Control::Arrange(a)) = self.items.get_mut(i).map(|it| &mut it.control)
        {
            a.cancel_held();
        }
    }

    /// Take over `old`'s hot or held item when the host rebuilds this page under the pointer.
    /// Items are re-matched by id; the part is dropped when the id is gone or the item no longer offers that light.
    pub fn carry_pointer(&mut self, old: &Form) {
        self.point = old.point;
        self.pointer = old.pointer.carry(|(i, light)| {
            let id = old.items.get(i)?.id;
            let j = self.items.iter().position(|it| it.id == id)?;
            light
                .offered_by(&self.items[j].control)
                .then_some((j, light))
        });
    }

    /// Move each control to the rect of `template`'s item with the same id, moved by `offset`;
    /// a control without one stays. Lets a resized window rebuild the page and keep what the controls hold.
    pub fn take_layout(&mut self, template: &Form, offset: (i32, i32)) {
        for it in &mut self.items {
            if let Some(t) = template.items.iter().find(|t| t.id == it.id) {
                it.rect = Rect::new(
                    t.rect.x + offset.0,
                    t.rect.y + offset.1,
                    t.rect.width,
                    t.rect.height,
                );
            }
        }
    }

    /// Trackbar `i`'s thumb moved without the pointer (a key, the host):
    /// with the pointer resting on the trackbar, its thumb turns hot when
    /// it moved under it and cold when it moved away.
    fn rehit_thumb(&mut self, i: usize) {
        let Some((x, y)) = self.point else {
            return;
        };
        let Some(it) = self.items.get(i) else {
            return;
        };
        let Control::TrackBar(t) = &it.control else {
            return;
        };
        let r = it.placed(&self.rendered.controls.get());
        let lay = trackbar::layout(t, r.x, r.y, r.width);
        if !lay.bounds.contains(x, y) {
            return;
        }
        let on = t.enabled && lay.thumb.contains(x, y);
        self.pointer.motion(on.then_some((i, Light::Thumb)));
    }

    fn track_keyed(&mut self, i: usize, moved: bool) -> Event {
        if moved {
            self.rehit_thumb(i);
        }
        self.changed(i, moved)
    }

    /// `Redraw` when what the look shows of the pointer differs from
    /// `before`.
    fn redraw_if(&self, before: Lit<(usize, Light)>) -> Event {
        if self.shown() == before {
            Event::None
        } else {
            Event::Redraw
        }
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn combo_selected(&self, id: &str) -> Option<usize> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Combo(c)) => Some(c.selected),
            _ => None,
        }
    }

    pub fn track_pos(&self, id: &str) -> Option<usize> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::TrackBar(t)) => Some(t.pos),
            _ => None,
        }
    }

    pub fn checked(&self, id: &str) -> Option<bool> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Check { checked, .. }) | Some(Control::Radio { checked, .. }) => {
                Some(*checked)
            }
            _ => None,
        }
    }

    pub fn set_text(&mut self, id: &str, s: &str) {
        if let Some(Control::Label { text, .. }) = self.item_mut(id).map(|i| &mut i.control) {
            *text = s.to_string();
        }
    }

    pub fn set_lines(&mut self, id: &str, s: &[String]) {
        if let Some(Control::Lines(lines)) = self.item_mut(id).map(|i| &mut i.control) {
            *lines = s.to_vec();
        }
    }

    pub fn set_picture(&mut self, id: &str, c: Canvas) {
        if let Some(Control::Picture { canvas, .. }) = self.item_mut(id).map(|i| &mut i.control) {
            *canvas = Some(c);
        }
    }

    /// Check or clear check box or radio `id`; a three-state box leaves
    /// its indeterminate state.
    pub fn set_check(&mut self, id: &str, on: bool) {
        match self.item_mut(id).map(|i| &mut i.control) {
            Some(Control::Check { checked, mixed, .. }) => {
                *checked = on;
                if let Some(m) = mixed {
                    *m = false;
                }
            }
            // A radio's own dot only: the host sets its group's others.
            Some(Control::Radio { checked, .. }) => *checked = on,
            _ => {}
        }
    }

    pub fn check_state(&self, id: &str) -> Option<CheckState> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Check {
                mixed: Some(true), ..
            }) => Some(CheckState::Mixed),
            Some(Control::Check { checked: true, .. }) => Some(CheckState::On),
            Some(Control::Check { .. }) => Some(CheckState::Off),
            _ => None,
        }
    }

    /// Put check box `id` in `state`. A two-state box reads `Mixed` as
    /// off.
    pub fn set_check_state(&mut self, id: &str, state: CheckState) {
        if let Some(Control::Check { checked, mixed, .. }) =
            self.item_mut(id).map(|i| &mut i.control)
        {
            *checked = state == CheckState::On;
            if let Some(m) = mixed {
                *m = state == CheckState::Mixed;
            }
        }
    }

    pub fn set_combo(&mut self, id: &str, selected: usize) {
        if let Some(Control::Combo(c)) = self.item_mut(id).map(|i| &mut i.control) {
            c.selected = selected.min(c.items.len().saturating_sub(1));
        }
    }

    /// Move trackbar `id` to `pos`, clamped to its last tick. Its thumb
    /// turns hot or cold when it moved to or from under the pointer, or
    /// was greyed or enabled under it.
    pub fn set_track(&mut self, id: &str, pos: usize) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        if let Control::TrackBar(t) = &mut self.items[i].control {
            t.set(pos);
            self.rehit_thumb(i);
        }
    }

    /// Enable or grey control `id`; a control with no greyed state (a
    /// group, a picture, a paragraph) is left alone.
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        match self.item_mut(id).map(|i| &mut i.control) {
            Some(Control::Label { enabled, .. })
            | Some(Control::Check { enabled, .. })
            | Some(Control::Radio { enabled, .. })
            | Some(Control::Button { enabled, .. })
            | Some(Control::Edit { enabled, .. }) => *enabled = on,
            Some(Control::Combo(c)) => c.enabled = on,
            Some(Control::UpDown(u)) => u.enabled = on,
            Some(Control::Color(b)) => b.enabled = on,
            Some(Control::TrackBar(t)) => t.enabled = on,
            Some(Control::Calendar(cal)) => cal.enabled = on,
            Some(Control::Time(tf)) => tf.enabled = on,
            Some(Control::HotKey(h)) => h.enabled = on,
            Some(Control::Memo(m)) => m.enabled = on,
            _ => {}
        }
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|i| i.id == id)
    }

    pub fn default_button(&self) -> Option<usize> {
        self.items.iter().position(|i| {
            matches!(
                &i.control,
                Control::Button {
                    default: true,
                    enabled: true,
                    ..
                }
            )
        })
    }

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    /// Move the focus to item `to`, or off the form with `None`, as the keyboard does: a spinner
    /// being left shows its clamped value, one arriving selects its number. `click` goes through here.
    pub fn set_focus(&mut self, to: Option<usize>) {
        if to == self.focus {
            return;
        }
        if let Some(u) = self.updown_at(self.focus) {
            u.leave();
        }
        if let Some(Control::HotKey(h)) = self
            .focus
            .and_then(|i| self.items.get_mut(i))
            .map(|i| &mut i.control)
        {
            h.blur();
        }
        self.focus = to;
        if let Some(u) = self.updown_at(to) {
            u.enter();
        }
    }

    fn updown_at(&mut self, i: Option<usize>) -> Option<&mut UpDown> {
        match &mut self.items.get_mut(i?)?.control {
            Control::UpDown(u) => Some(u),
            _ => None,
        }
    }

    pub fn focus_id(&mut self, id: &str) {
        let to = self.index_of(id).filter(|&i| self.items[i].focusable());
        self.set_focus(to);
    }

    fn open_combo(&self) -> Option<usize> {
        self.items.iter().position(|i| {
            matches!(&i.control, Control::Combo(c) if c.open.is_some())
                || matches!(&i.control, Control::Color(b) if b.open)
        })
    }

    pub fn popup_open(&self) -> bool {
        self.open_combo().is_some()
    }

    fn close_popups(&mut self) -> bool {
        let mut closed = false;
        for i in &mut self.items {
            match &mut i.control {
                Control::Combo(c) if c.open.is_some() => {
                    c.open = None;
                    closed = true;
                }
                Control::Color(b) if b.open => {
                    b.open = false;
                    closed = true;
                }
                _ => {}
            }
        }
        closed
    }

    fn move_focus(&mut self, back: bool) -> Event {
        let n = self.items.len();
        if n == 0 {
            return Event::None;
        }
        let start = self.focus.unwrap_or(if back { 0 } else { n - 1 });
        for k in 1..=n {
            let i = if back {
                (start + n - k) % n
            } else {
                (start + k) % n
            };
            if self.tab_stop(i) {
                self.set_focus(Some(i));
                return Event::Redraw;
            }
        }
        Event::None
    }

    /// Tab without `move_focus`'s wrap: `false` when the focus is already at that end, where a
    /// property sheet takes it onto its own OK / Cancel / Apply.
    pub fn tab_no_wrap(&mut self, back: bool) -> bool {
        let Some(start) = self.focus else {
            return false;
        };
        let next = if back {
            (0..start).rev().find(|&i| self.tab_stop(i))
        } else {
            (start + 1..self.items.len()).find(|&i| self.tab_stop(i))
        };
        match next {
            Some(i) => {
                self.set_focus(Some(i));
                true
            }
            None => false,
        }
    }

    /// Focus the form's first or last tab stop, for a host handing focus back. `false` if there is none.
    pub fn focus_edge(&mut self, last: bool) -> bool {
        let n = self.items.len();
        let found = if last {
            (0..n).rev().find(|&i| self.tab_stop(i))
        } else {
            (0..n).find(|&i| self.tab_stop(i))
        };
        self.set_focus(found);
        found.is_some()
    }

    fn focus_after(&mut self, i: usize) -> Event {
        for j in i + 1..self.items.len() {
            if self.tab_stop(j) {
                self.set_focus(Some(j));
                return Event::Redraw;
            }
        }
        Event::None
    }

    fn changed(&self, i: usize, changed: bool) -> Event {
        if changed {
            Event::Changed(self.items[i].id)
        } else {
            Event::Redraw
        }
    }

    fn pick_radio(&mut self, i: usize) -> bool {
        let Control::Radio { group, .. } = &self.items[i].control else {
            return false;
        };
        let group = *group;
        let mut changed = false;
        for (k, it) in self.items.iter_mut().enumerate() {
            if let Control::Radio {
                group: g, checked, ..
            } = &mut it.control
            {
                if *g != group {
                    continue;
                }
                let want = k == i;
                if *checked != want {
                    *checked = want;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Whether Tab stops here. A radio group is one stop: the checked
    /// radio, or the first enabled one when none is checked.
    fn tab_stop(&self, i: usize) -> bool {
        if !self.items[i].focusable() {
            return false;
        }
        let Control::Radio { group, .. } = &self.items[i].control else {
            return true;
        };
        let mut first = None;
        for (k, it) in self.items.iter().enumerate() {
            if let Control::Radio {
                group: g,
                checked,
                enabled,
                ..
            } = &it.control
            {
                if g != group || !*enabled {
                    continue;
                }
                if *checked {
                    return k == i;
                }
                first.get_or_insert(k);
            }
        }
        first == Some(i)
    }

    /// The next enabled radio in `from`'s group, in item order, without wrapping and excluding
    /// `from` itself; `None` at the group's ends.
    fn next_radio(&self, from: usize, back: bool) -> Option<usize> {
        let Control::Radio { group, .. } = &self.items[from].control else {
            return None;
        };
        let same = |i: &usize| {
            matches!(
                &self.items[*i].control,
                Control::Radio {
                    group: g,
                    enabled: true,
                    ..
                } if g == group
            )
        };
        if back {
            (0..from).rev().find(same)
        } else {
            (from + 1..self.items.len()).find(same)
        }
    }

    /// An arrow on the push button at `from`: focus goes to the previous or next enabled push
    /// button, stopping at the row's ends.
    fn next_button(&mut self, from: usize, back: bool) -> Event {
        let button = |i: &usize| {
            matches!(
                self.items[*i].control,
                Control::Button { enabled: true, .. }
            )
        };
        let to = if back {
            (0..from).rev().find(button)
        } else {
            (from + 1..self.items.len()).find(button)
        };
        match to {
            Some(j) => {
                self.set_focus(Some(j));
                Event::Redraw
            }
            None => Event::None,
        }
    }

    /// The focused edit control, a field or a memo.
    pub fn focused_edit(&self) -> Option<FocusedEdit<'_>> {
        match &self.items.get(self.focus?)?.control {
            Control::Edit { edit, .. } => Some(FocusedEdit::Line(edit)),
            Control::Memo(m) => Some(FocusedEdit::Lines(&m.edit)),
            _ => None,
        }
    }

    /// A host's clipboard text for the focused edit field, taken as
    /// [`TextEdit::paste`] takes it. The text is zeroized once used, since
    /// the field may be a password.
    pub fn paste(&mut self, mut text: String) -> Event {
        let field = self.focus.filter(|_| self.open_combo().is_none());
        let ev = match field.map(|i| (self.items[i].id, &mut self.items[i].control)) {
            Some((id, Control::Memo(m))) => {
                if m.edit.paste(&text) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            Some((id, Control::Edit { edit, .. })) => {
                if edit.paste(&text, &[]) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            _ => Event::None,
        };
        text.zeroize();
        ev
    }

    /// A drag at (`x`, `y`): over an enabled, unmasked edit field or editable combo the field
    /// shows a drop caret there and takes the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        self.drag_leave();
        let Some(Hit { item, part, .. }) = self.hit(x, y, text, px) else {
            return false;
        };
        let cm = self.controls();
        let it = &mut self.items[item];
        let placed = it.placed(&cm);
        let takes = match (part, &mut it.control) {
            (Part::Edit(_), Control::Edit { edit, .. }) => !edit.is_secret(),
            (Part::Combo(combo::Hit::Field), Control::Combo(c)) => {
                c.drag_over(&combo::layout(c, placed, &cm), x, y)
            }
            _ => false,
        };
        if takes {
            self.field_drop = Some(FieldDrop { item, x, y });
        }
        takes
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        let Some(d) = self.field_drop.take() else {
            return false;
        };
        if let Some(Control::Combo(c)) = self.items.get_mut(d.item).map(|it| &mut it.control) {
            c.drag_leave();
        }
        true
    }

    /// The scroll offset one step on for the field under a resting drag
    /// ([`crate::textedit::drag_scroll`], a [`LINE_H`] band at each side), None when it stays.
    pub fn drag_scroll(&self, text: &TextRenderer, px: u32) -> Option<usize> {
        let d = self.field_drop?;
        let it = self.items.get(d.item)?;
        let text = text.dialog(self.font);
        let cm = self.controls();
        match &it.control {
            Control::Edit { edit, first, .. } => {
                let inner = it.placed(&cm).inset(widgets::FIELD_PAD);
                crate::textedit::drag_scroll(
                    text,
                    &edit.text,
                    visible_from(edit, first),
                    inner.width,
                    LINE_H,
                    d.x - inner.x,
                    px,
                )
            }
            Control::Combo(c) => c.drag_scroll(&combo::layout(c, it.placed(&cm), &cm), text, px),
            _ => None,
        }
    }

    /// Take [`Form::drag_scroll`]'s step, for a host's tick; `true` when the field moved.
    pub fn drag_tick(&mut self, text: &TextRenderer, px: u32) -> bool {
        let Some(to) = self.drag_scroll(text, px) else {
            return false;
        };
        match self
            .field_drop
            .and_then(|d| self.items.get(d.item))
            .map(|it| &it.control)
        {
            Some(Control::Edit { first, .. }) => first.set(to),
            Some(Control::Combo(c)) => c.first.set(to),
            _ => return false,
        }
        true
    }

    /// A drag dropped `line` on the field under it: it goes in at the drop caret as
    /// [`TextEdit::drop_text`] puts it, and the field takes the focus. `Changed` with the field's
    /// id, or `None` when no field was taking the drag or nothing went in.
    pub fn drop_text(
        &mut self,
        line: &crate::textedit::DropLine,
        text: &TextRenderer,
        px: u32,
    ) -> Event {
        let Some(d) = self.field_drop.take() else {
            return Event::None;
        };
        let hit = self.hit(d.x, d.y, text, px).filter(|h| h.item == d.item);
        let cm = self.controls();
        let dialog = text.dialog(self.font);
        let it = &mut self.items[d.item];
        let placed = it.placed(&cm);
        let took = match (hit.map(|h| h.part), &mut it.control) {
            (Some(Part::Edit(at)), Control::Edit { edit, .. }) => edit.drop_text(at, line),
            (_, Control::Combo(c)) => c.drop_text(&combo::layout(c, placed, &cm), line, dialog, px),
            _ => false,
        };
        if !took {
            return Event::None;
        }
        let id = it.id;
        self.set_focus(Some(d.item));
        Event::Changed(id)
    }

    /// The window lost the keyboard: a focused hot-key field forgets the
    /// modifiers it saw go down, whose releases go elsewhere
    /// ([`HotKey::blur`]). `true` when one was focused, to be redrawn.
    pub fn blur_hotkey(&mut self) -> bool {
        match self.focus.and_then(|i| self.items.get_mut(i)) {
            Some(Item {
                control: Control::HotKey(h),
                ..
            }) => {
                h.blur();
                true
            }
            _ => false,
        }
    }

    /// A raw key for a focused hot-key field, pressed (`down`) or released. Hosts call this
    /// before reading the key as a dialog key, and read it that way only on `None`. `name` is
    /// xkb's lower-case keysym name, `held` the seat's modifiers. See [`HotKey::key`].
    pub fn hotkey_key(
        &mut self,
        sym: u32,
        name: &str,
        held: hotkey::Mods,
        down: bool,
    ) -> Option<Event> {
        let i = self.focus.filter(|_| self.open_combo().is_none())?;
        let Control::HotKey(h) = &mut self.items[i].control else {
            return None;
        };
        match h.key(sym, name, held, down) {
            hotkey::Took::Pass => None,
            hotkey::Took::Redraw => Some(Event::Redraw),
            hotkey::Took::Changed => Some(Event::Changed(self.items[i].id)),
        }
    }

    /// Ctrl+arrow for a focused arrangement: its picked tile moves ([`arrange::Arrange::shove`]).
    /// `None` when something else has the focus, which then reads the press as usual.
    pub fn shove(&mut self, dir: arrange::Dir) -> Option<Event> {
        let i = self.focus.filter(|_| self.open_combo().is_none())?;
        let id = self.items[i].id;
        let Control::Arrange(a) = &mut self.items[i].control else {
            return None;
        };
        Some(if a.enabled && a.shove(dir) {
            Event::Changed(id)
        } else {
            Event::None
        })
    }

    pub fn key(&mut self, key: Key) -> Event {
        let key = match key {
            Key::NewLine => match self.focus.map(|i| &self.items[i].control) {
                Some(Control::Memo(_)) if self.open_combo().is_none() => {
                    Key::Edit(EditKey::Insert("\n".into()))
                }
                _ => Key::Enter,
            },
            Key::DocEdge { end, select } => {
                if self.open_combo().is_none()
                    && let Some(i) = self.focus
                    && let Control::Memo(m) = &mut self.items[i].control
                {
                    let to = if end {
                        multiedit::Pos {
                            line: usize::MAX,
                            col: usize::MAX,
                        }
                    } else {
                        multiedit::Pos::default()
                    };
                    return if m.edit.place(to, select) {
                        Event::Redraw
                    } else {
                        Event::None
                    };
                }
                Key::Edit(if end {
                    EditKey::End { select }
                } else {
                    EditKey::Home { select }
                })
            }
            k => k,
        };
        // An editing key goes to a focused edit field or spinner that takes it; elsewhere it
        // degrades to its flat key. Above the popup guard, since an open drop-list takes these keys too.
        if let Key::Edit(k) = &key {
            let open = self.open_combo().is_some();
            if !open
                && let Some(i) = self.focus
                && let Control::UpDown(u) = &mut self.items[i].control
                && let Some(ch) = u.key(k)
            {
                return self.changed(i, ch);
            }
            let to_field = !open
                && self.focus.is_some_and(|i| {
                    matches!(
                        self.items[i].control,
                        Control::Edit { .. } | Control::Memo(_)
                    )
                });
            if !to_field {
                return match flat_key(k) {
                    Some(flat) => self.key(flat),
                    None => Event::None,
                };
            }
        }
        if let Some(i) = self.open_combo() {
            let id = self.items[i].id;
            return match &mut self.items[i].control {
                Control::Combo(c) => match key {
                    Key::Up => {
                        c.step(false);
                        Event::Redraw
                    }
                    Key::Down => {
                        c.step(true);
                        Event::Redraw
                    }
                    Key::Home => {
                        c.jump(false);
                        Event::Redraw
                    }
                    Key::End => {
                        c.jump(true);
                        Event::Redraw
                    }
                    Key::PageUp => {
                        c.scroll(-(c.rows as i32));
                        Event::Redraw
                    }
                    Key::PageDown => {
                        c.scroll(c.rows as i32);
                        Event::Redraw
                    }
                    Key::Enter => {
                        if c.take() {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    Key::Escape | Key::Drop | Key::Tab { .. } => {
                        c.open = None;
                        Event::Redraw
                    }
                    Key::Char(ch) => {
                        let names = c.items.iter().map(|it| it.text.as_str());
                        if let Some(k) = crate::selection::type_ahead(names, c.open, ch) {
                            c.open = Some(k);
                            c.top = k.saturating_sub(c.rows - 1).min(k);
                        }
                        Event::Redraw
                    }
                    _ => Event::None,
                },
                Control::Color(b) => match key {
                    Key::Escape | Key::Enter | Key::Space | Key::Tab { .. } => {
                        b.open = false;
                        Event::Redraw
                    }
                    _ => Event::None,
                },
                _ => Event::None,
            };
        }
        match key {
            Key::Tab { back } => self.move_focus(back),
            Key::Escape => Event::Cancel,
            Key::Enter => match self.focus {
                Some(i) if matches!(self.items[i].control, Control::Button { .. }) => {
                    Event::Pressed(self.items[i].id)
                }
                _ => Event::Default,
            },
            Key::Char(ch) => self.char_key(ch),
            Key::Access(ch) => self.access_key(ch),
            Key::Edit(k) => {
                // The guard at the top of `key` sends editing keys only to a
                // focused Control::Edit.
                let i = self
                    .focus
                    .expect("an editing key only reaches a focused control");
                let id = self.items[i].id;
                let changed = match &mut self.items[i].control {
                    Control::Edit { edit, .. } => edit.key(k),
                    Control::Memo(m) => m.edit.edit_key(k),
                    _ => unreachable!("an editing key only reaches a Control::Edit or Memo"),
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            other => {
                let Some(i) = self.focus else {
                    return Event::None;
                };
                let id = self.items[i].id;
                let rect = self.items[i].placed(&self.rendered.controls.get());
                match (&mut self.items[i].control, other) {
                    (Control::Check { checked, mixed, .. }, Key::Space) => {
                        toggle_check(checked, mixed);
                        Event::Changed(id)
                    }
                    (Control::Memo(m), Key::Space) => {
                        if m.edit.edit_key(EditKey::Insert(" ".into())) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    (
                        Control::IconList(l),
                        key @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End),
                    ) => {
                        let step = match key {
                            Key::Up => iconlist::Step::Up,
                            Key::Down => iconlist::Step::Down,
                            Key::Left => iconlist::Step::Left,
                            Key::Right => iconlist::Step::Right,
                            Key::Home => iconlist::Step::Home,
                            _ => iconlist::Step::End,
                        };
                        let moved = l.step(step);
                        l.follow(&iconlist::layout(rect, &self.rendered.controls.get()));
                        if moved {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (
                        Control::Arrange(a),
                        key @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End),
                    ) => {
                        let step = match key {
                            Key::Up | Key::Left => arrange::Step::Back,
                            Key::Down | Key::Right => arrange::Step::Forward,
                            Key::Home => arrange::Step::First,
                            _ => arrange::Step::Last,
                        };
                        if a.step(step) {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (Control::Memo(m), key @ (Key::Up | Key::Down)) => {
                        if m.edit.step_lines(key == Key::Down, 1) {
                            Event::Redraw
                        } else {
                            Event::None
                        }
                    }
                    (Control::Memo(m), key @ (Key::PageUp | Key::PageDown)) => {
                        let rows = m.rows.get().max(1);
                        if m.edit.step_lines(key == Key::PageDown, rows) {
                            Event::Redraw
                        } else {
                            Event::None
                        }
                    }
                    (Control::Button { .. }, Key::Space) => Event::Pressed(id),
                    (Control::Button { .. }, Key::Left | Key::Up) => self.next_button(i, true),
                    (Control::Button { .. }, Key::Right | Key::Down) => self.next_button(i, false),
                    (Control::ListBox { list, .. }, Key::Space) => {
                        let flipped = list.selected.is_some_and(|r| list.toggle(r));
                        if flipped {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (Control::Radio { .. }, Key::Space) => {
                        let ch = self.pick_radio(i);
                        self.changed(i, ch)
                    }
                    (Control::Radio { .. }, Key::Up | Key::Left) => {
                        match self.next_radio(i, true) {
                            Some(j) => {
                                self.set_focus(Some(j));
                                let ch = self.pick_radio(j);
                                self.changed(j, ch)
                            }
                            None => Event::None,
                        }
                    }
                    (Control::Radio { .. }, Key::Down | Key::Right) => {
                        match self.next_radio(i, false) {
                            Some(j) => {
                                self.set_focus(Some(j));
                                let ch = self.pick_radio(j);
                                self.changed(j, ch)
                            }
                            None => Event::None,
                        }
                    }
                    (Control::Color(b), Key::Space | Key::Drop) => {
                        b.open = true;
                        Event::Redraw
                    }
                    (Control::Combo(c), Key::Drop) => {
                        c.toggle();
                        Event::Redraw
                    }
                    (Control::Combo(c), Key::Up | Key::Left) => {
                        let ch = c.step(false);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::Down | Key::Right) => {
                        let ch = c.step(true);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::Home | Key::PageUp) => {
                        let ch = c.jump(false);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::End | Key::PageDown) => {
                        let ch = c.jump(true);
                        self.changed(i, ch)
                    }
                    (Control::ListBox { list, .. }, k) => {
                        let visible =
                            listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                        let delta = match k {
                            Key::Up => -1,
                            Key::Down => 1,
                            Key::PageUp => -(visible as i64),
                            Key::PageDown => visible as i64,
                            Key::Home => i64::MIN / 2,
                            Key::End => i64::MAX / 2,
                            _ => return Event::None,
                        };
                        let ch = list.step(delta, visible);
                        self.changed(i, ch)
                    }
                    (Control::Tree(tv), k) => {
                        let lay =
                            treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                        let ch = match k {
                            Key::Up => tv.step(-1, &lay),
                            Key::Down => tv.step(1, &lay),
                            Key::PageUp => tv.step(-(lay.rows as i64), &lay),
                            Key::PageDown => tv.step(lay.rows as i64, &lay),
                            Key::Home => tv.step(i64::MIN / 2, &lay),
                            Key::End => tv.step(i64::MAX / 2, &lay),
                            Key::Right => {
                                tv.expand_selected(&lay);
                                return Event::Redraw;
                            }
                            Key::Left => {
                                let ch = tv.collapse_selected(&lay);
                                return self.changed(i, ch);
                            }
                            _ => return Event::None,
                        };
                        self.changed(i, ch)
                    }
                    (Control::Calendar(cal), k) => {
                        let delta = match k {
                            Key::Left => -1,
                            Key::Right => 1,
                            Key::Up => -7,
                            Key::Down => 7,
                            _ => return Event::None,
                        };
                        let ch = cal.step_day(delta);
                        self.changed(i, ch)
                    }
                    (Control::Time(tf), k) => match k {
                        Key::Up | Key::Down => {
                            let ch = tf.step(if k == Key::Up { 1 } else { -1 });
                            self.changed(i, ch)
                        }
                        Key::Left | Key::Right => {
                            tf.move_part(if k == Key::Left { -1 } else { 1 });
                            Event::Redraw
                        }
                        _ => Event::None,
                    },
                    (Control::UpDown(u), Key::Up) => {
                        let ch = u.step(1);
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), Key::Down) => {
                        let ch = u.step(-1);
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), Key::Backspace) => {
                        let ch = u.backspace();
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), k @ (Key::Left | Key::Right | Key::Home | Key::End)) => {
                        let k = match k {
                            Key::Left => EditKey::Left {
                                select: false,
                                word: false,
                            },
                            Key::Right => EditKey::Right {
                                select: false,
                                word: false,
                            },
                            Key::Home => EditKey::Home { select: false },
                            _ => EditKey::End { select: false },
                        };
                        u.key(&k);
                        Event::Redraw
                    }
                    (Control::TrackBar(t), Key::Left | Key::Down) => {
                        let ch = t.step(-1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::Right | Key::Up) => {
                        let ch = t.step(1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::Home) => {
                        let ch = t.set(0);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::End) => {
                        let ch = t.set(t.count - 1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::PageUp) => {
                        let ch = t.step(-(t.page as i32));
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::PageDown) => {
                        let ch = t.step(t.page as i32);
                        self.track_keyed(i, ch)
                    }
                    // A space in a field is text; it only presses or
                    // toggles the controls that have nothing to type in.
                    (Control::Edit { edit, .. }, Key::Space) => {
                        let changed = edit.key(EditKey::Insert(" ".into()));
                        if changed {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    _ => Event::None,
                }
            }
        }
    }

    /// A typed character: a digit into a spinner, a letter into a combo or list box
    /// (type-ahead), else an access key.
    fn char_key(&mut self, ch: char) -> Event {
        if let Some(i) = self.focus {
            let id = self.items[i].id;
            let rect = self.items[i].placed(&self.rendered.controls.get());
            match &mut self.items[i].control {
                Control::Memo(m) => {
                    let changed = m.edit.edit_key(EditKey::Insert(ch.to_string()));
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Edit { edit, .. } => {
                    let changed = edit.key(EditKey::Insert(ch.to_string()));
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Time(tf) if tf.enabled && ch.is_ascii_alphanumeric() => {
                    let changed = tf.char(ch);
                    return self.changed(i, changed);
                }
                Control::UpDown(u) if ch.is_ascii_digit() => {
                    let changed = u.digit(ch);
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Combo(c) if ch.is_alphanumeric() => {
                    let names = c.items.iter().map(|it| it.text.as_str());
                    if let Some(k) = crate::selection::type_ahead(names, Some(c.selected), ch) {
                        let changed = c.pick(k);
                        return if changed {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        };
                    }
                }
                Control::ListBox { list, .. } if ch.is_alphanumeric() => {
                    let visible =
                        listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                    if list.jump(ch, visible) {
                        return Event::Changed(id);
                    }
                }
                _ => {}
            }
        }
        self.access_key(ch)
    }

    fn access_key(&mut self, ch: char) -> Event {
        let key = ch.to_lowercase().next().unwrap_or(ch);
        let Some(i) = self.items.iter().position(|it| it.mnemonic() == Some(key)) else {
            return Event::None;
        };
        let id = self.items[i].id;
        match &mut self.items[i].control {
            Control::Check { checked, mixed, .. } => {
                toggle_check(checked, mixed);
                self.set_focus(Some(i));
                Event::Changed(id)
            }
            Control::Radio { .. } => {
                self.set_focus(Some(i));
                let ch = self.pick_radio(i);
                if ch {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            Control::Button { .. } => {
                self.set_focus(Some(i));
                Event::Pressed(id)
            }
            Control::Label { .. } => self.focus_after(i),
            _ => Event::None,
        }
    }

    pub fn hit(&self, x: i32, y: i32, text: &TextRenderer, px: u32) -> Option<Hit> {
        let cm_all = self.rendered.controls.get();
        if let Some(i) = self.open_combo() {
            let it = &self.items[i];
            match &it.control {
                Control::Combo(c) => {
                    let lay = combo::layout(c, it.placed(&cm_all), &self.rendered.controls.get());
                    if let Some(h) = combo::hit(c, &lay, x, y) {
                        return Some(Hit {
                            item: i,
                            part: Part::Combo(h),
                            x,
                        });
                    }
                }
                Control::Color(_) => match colorbutton::hit_popup(it.placed(&cm_all), x, y) {
                    Some(colorbutton::PopupHit::Swatch(k)) => {
                        return Some(Hit {
                            item: i,
                            part: Part::Swatch(k),
                            x,
                        });
                    }
                    Some(colorbutton::PopupHit::Face) => {
                        return Some(Hit {
                            item: i,
                            part: Part::Popup,
                            x,
                        });
                    }
                    None => {}
                },
                _ => {}
            }
        }
        let text = text.dialog(self.font);
        for (i, it) in self.items.iter().enumerate().rev() {
            let part = match &it.control {
                Control::Combo(c) => {
                    let lay = combo::layout(c, it.placed(&cm_all), &self.rendered.controls.get());
                    match combo::hit(c, &lay, x, y) {
                        Some(h) => Part::Combo(h),
                        None => continue,
                    }
                }
                Control::ListBox { list, .. } => {
                    let lay =
                        listbox::layout(list, it.placed(&cm_all), &self.rendered.controls.get());
                    match listbox::hit(list, &lay, x, y) {
                        Some(h) => Part::List(h),
                        None if it.placed(&cm_all).contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Tree(tv) => {
                    if !it.placed(&cm_all).contains(x, y) {
                        continue;
                    }
                    let lay = treeview::layout_without_text(
                        tv,
                        it.placed(&cm_all),
                        &self.rendered.controls.get(),
                    );
                    Part::Tree(tv.hit(&lay, x, y))
                }
                Control::Calendar(cal) => {
                    match calendar::hit(cal, &calendar::layout(it.placed(&cm_all)), x, y) {
                        Some(h) => Part::Calendar(h),
                        None => continue,
                    }
                }
                Control::Time(tf) => {
                    let cm = self.rendered.controls.get();
                    match timefield::hit(
                        tf,
                        &timefield::layout(it.placed(&cm_all), &cm),
                        x,
                        y,
                        text,
                        px,
                    ) {
                        Some(h) => Part::Time(h),
                        None => continue,
                    }
                }
                Control::Clock(_) => continue,
                Control::HotKey(_) => {
                    if !hotkey::hit(hotkey::layout(it.placed(&cm_all)), x, y) {
                        continue;
                    }
                    Part::Body
                }
                Control::UpDown(u) => {
                    let lay =
                        updown::layout_for(u, it.placed(&cm_all), &self.rendered.controls.get());
                    match updown::hit(u, &lay, x, y, text, px) {
                        Some(h) => Part::UpDown(h),
                        None => continue,
                    }
                }
                Control::TrackBar(t) => {
                    let lay = trackbar::layout(
                        t,
                        it.placed(&cm_all).x,
                        it.placed(&cm_all).y,
                        it.placed(&cm_all).width,
                    );
                    match trackbar::hit(&lay, x, y) {
                        Some(h) => Part::TrackBar(h),
                        None => continue,
                    }
                }
                Control::Check { .. } => {
                    let cm = self.rendered.controls.get();
                    let r = it.placed(&cm_all);
                    if check_box(r, &cm).contains(x, y)
                        || check_label_rect(r, it.indent, &cm, 200).contains(x, y)
                    {
                        Part::Body
                    } else {
                        continue;
                    }
                }
                Control::Radio { .. } => {
                    let cm = self.rendered.controls.get();
                    let r = it.placed(&cm_all);
                    if radio_box(r, &cm).contains(x, y)
                        || radio_label_rect(r, &cm, 200).contains(x, y)
                    {
                        Part::Body
                    } else {
                        continue;
                    }
                }
                Control::IconList(l) => {
                    let lay = iconlist::layout(it.placed(&cm_all), &cm_all);
                    match l.hit(&lay, &cm_all, x, y) {
                        Some(h) => Part::IconList(h),
                        None if lay.frame.contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Arrange(a) => {
                    let lay = arrange::layout(it.placed(&cm_all), a);
                    match a.hit(&lay, x, y) {
                        Some(tile) => Part::Tile { tile, at: (x, y) },
                        None if lay.frame.contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Memo(memo) => {
                    let r = it.placed(&cm_all);
                    if !r.contains(x, y) {
                        continue;
                    }
                    let text = text.dialog(self.font);
                    let m = Memo::metrics(px, &cm_all);
                    let lay = multiedit::layout(text, &m, &memo.edit, r, 1);
                    let view = self.memo_view_1x(memo);
                    let bar = multiedit::bar_hit(text, &m, &memo.edit, &lay, view, x, y).and_then(
                        |(bar, h)| {
                            let span = multiedit::bar_span(text, &m, &memo.edit, &lay, view, bar)?;
                            Some((bar, h, span))
                        },
                    );
                    match bar {
                        Some((bar, hit, span)) => {
                            let to = view.press_bar(text, &m, &memo.edit, &lay, bar, hit);
                            Part::MemoBar {
                                to: self.memo_view_device(memo, to),
                                hit,
                                span,
                            }
                        }
                        None => {
                            let at = multiedit::pos_at(text, &m, &memo.edit, &lay, view, x, y, 1);
                            Part::Memo(at)
                        }
                    }
                }
                Control::Edit {
                    edit,
                    first,
                    enabled,
                } => {
                    if !it.placed(&cm_all).contains(x, y) {
                        continue;
                    }
                    if !*enabled {
                        // A greyed field is just face. `click` rejects it
                        // anyway, so there is nothing to measure here.
                        Part::Body
                    } else {
                        // The pen starts FIELD_PAD inside the sunken frame
                        // and at the field's scroll offset, so the index is
                        // counted from there, not from the value's start.
                        let hidden;
                        let shown = if edit.is_secret() {
                            hidden = masked(edit);
                            &hidden
                        } else {
                            edit
                        };
                        let inner_x = it.placed(&cm_all).x + crate::classic::widgets::FIELD_PAD;
                        let from = visible_from(shown, first);
                        let dx = (x - inner_x).max(0);
                        let at = crate::textedit::index_at(text, &shown.text[from..], dx, px);
                        // Through the bullets and back, so `click` places
                        // the caret in the value, not in what hides it.
                        Part::Edit(if edit.is_secret() {
                            unmasked_index(edit, from + at)
                        } else {
                            from + at
                        })
                    }
                }
                Control::Label { .. }
                | Control::Lines(_)
                | Control::Group(_)
                | Control::GroupTitle(_)
                | Control::Separator
                | Control::Rule
                | Control::Blank => continue,
                _ => {
                    if it.placed(&cm_all).contains(x, y) {
                        Part::Body
                    } else {
                        continue;
                    }
                }
            };
            return Some(Hit { item: i, part, x });
        }
        None
    }

    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        let lit = self.light(hit);
        self.pointer
            .press(lit, lit.is_some_and(|(_, l)| l.sticky()));
        let open = self.open_combo();
        let Some(h) = hit else {
            return if self.close_popups() {
                Event::Redraw
            } else {
                Event::None
            };
        };
        if open.is_some_and(|o| o != h.item) {
            self.close_popups();
        }
        let i = h.item;
        let id = self.items[i].id;
        let rect = self.items[i].placed(&self.rendered.controls.get());
        if self.items[i].focusable() {
            self.set_focus(Some(i));
        }
        match (&mut self.items[i].control, h.part) {
            (
                Control::Button { enabled, .. }
                | Control::Check { enabled, .. }
                | Control::Radio { enabled, .. },
                Part::Body,
            ) => {
                if *enabled {
                    // These act on the release (`release`), not the
                    // press; until then the pointer holds them, and they
                    // draw pressed.
                    Event::Redraw
                } else {
                    Event::None
                }
            }
            (Control::IconList(l), Part::IconList(h)) => {
                let lay = iconlist::layout(rect, &self.rendered.controls.get());
                let picked = matches!(h, iconlist::Hit::Cell(_));
                if l.press(h, &lay) && picked {
                    Event::Changed(self.items[i].id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Arrange(a), Part::Tile { tile, at }) if a.enabled => {
                let lay = arrange::layout(rect, a);
                a.press(&lay, tile, at.0, at.1);
                self.drag = Some(Drag::Arrange(i));
                Event::Redraw
            }
            (Control::Memo(memo), Part::Memo(at)) => {
                if !memo.enabled {
                    return Event::None;
                }
                memo.edit.place(at, false);
                Event::Redraw
            }
            (Control::Memo(memo), Part::MemoBar { to, hit, span }) => {
                if !memo.enabled {
                    return Event::None;
                }
                let moved = memo.view.get() != to;
                memo.view.set(to);
                let mut thumb = ThumbDrag::default();
                thumb.press(span.axis(), hit);
                if thumb.holding() {
                    self.drag = Some(Drag::Memo {
                        item: i,
                        thumb,
                        span,
                    });
                }
                if moved { Event::Redraw } else { Event::None }
            }
            (Control::Edit { edit, enabled, .. }, Part::Edit(at)) => {
                if !*enabled {
                    return Event::None;
                }
                edit.place(at, false);
                self.drag = Some(Drag::Edit(i));
                Event::Redraw
            }
            (Control::Combo(c), Part::Combo(ch)) => match ch {
                combo::Hit::Drop | combo::Hit::Field => {
                    c.toggle();
                    Event::Redraw
                }
                combo::Hit::Row(r) => {
                    if c.pick(r) {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    }
                }
                combo::Hit::ScrollUp
                | combo::Hit::ScrollDown
                | combo::Hit::PageUp
                | combo::Hit::PageDown
                | combo::Hit::Thumb(_) => {
                    let mut thumb = ThumbDrag::default();
                    if let Some(bar) = ch.bar() {
                        c.press_bar(bar, &mut thumb);
                    }
                    self.grab_bar(i, thumb);
                    Event::Redraw
                }
            },
            (Control::ListBox { list, .. }, Part::List(lh)) => {
                let visible = listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                match lh {
                    listbox::Hit::Row(r) => {
                        if list.select(r, visible) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    listbox::Hit::Check(r) => {
                        list.select(r, visible);
                        list.toggle(r);
                        Event::Changed(id)
                    }
                    listbox::Hit::ScrollUp
                    | listbox::Hit::ScrollDown
                    | listbox::Hit::PageUp
                    | listbox::Hit::PageDown
                    | listbox::Hit::Thumb(_) => {
                        let mut thumb = ThumbDrag::default();
                        if let Some(bar) = lh.bar() {
                            let by = thumb.press(Axis::Vertical, bar).by(1, visible as i64);
                            list.scroll(by as i32, visible);
                        }
                        self.grab_bar(i, thumb);
                        Event::Redraw
                    }
                }
            }
            (Control::ListBox { .. }, _) => Event::Redraw,
            (Control::Tree(tv), Part::Tree(th)) => {
                let lay = treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                match th {
                    treeview::Hit::Row(r) => {
                        if tv.select(r, &lay) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    treeview::Hit::Expander(r) => {
                        tv.toggle(r, &lay);
                        Event::Redraw
                    }
                    treeview::Hit::VBar(bar) => {
                        let mut thumb = ThumbDrag::default();
                        let by = thumb.press(Axis::Vertical, bar).by(1, lay.rows as i64);
                        if by != 0 {
                            tv.scroll(by, &lay);
                        }
                        self.grab_bar(i, thumb);
                        Event::Redraw
                    }
                    // A form's tree never gets a horizontal bar (see
                    // `treeview::layout_without_text`).
                    treeview::Hit::HBar(_) | treeview::Hit::Nothing => Event::Redraw,
                }
            }
            (Control::Tree(_), _) => Event::Redraw,
            (Control::Calendar(cal), Part::Calendar(calendar::Hit::Day(d))) => {
                if !cal.enabled {
                    return Event::None;
                }
                let ch = cal.pick(d);
                self.changed(i, ch)
            }
            (Control::Calendar(_), Part::Calendar(_)) => Event::Redraw,
            (Control::Time(tf), Part::Time(th)) => {
                if !tf.enabled {
                    return Event::None;
                }
                match th {
                    timefield::Hit::Part(k) => {
                        tf.select(k);
                        Event::Redraw
                    }
                    timefield::Hit::Up => {
                        let ch = tf.step(1);
                        self.changed(i, ch)
                    }
                    timefield::Hit::Down => {
                        let ch = tf.step(-1);
                        self.changed(i, ch)
                    }
                }
            }
            (Control::UpDown(u), Part::UpDown(uh)) => {
                if !u.enabled {
                    return Event::None;
                }
                let changed = match uh {
                    updown::Hit::Up => u.step(1),
                    updown::Hit::Down => u.step(-1),
                    updown::Hit::Edit(at) => {
                        u.place(at);
                        false
                    }
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::TrackBar(t), Part::TrackBar(th)) => {
                let changed = match th {
                    trackbar::Hit::Thumb => {
                        self.drag = Some(Drag::TrackBar(i));
                        false
                    }
                    trackbar::Hit::Left => t.step(-1),
                    trackbar::Hit::Right => t.step(1),
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Color(b), Part::Body) => {
                if b.toggle() {
                    Event::Redraw
                } else {
                    Event::None
                }
            }
            (Control::Color(b), Part::Swatch(k)) => {
                if b.pick(k) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Color(_), Part::Popup) => Event::None,
            (Control::HotKey(h), Part::Body) if h.enabled => Event::Redraw,
            _ => Event::None,
        }
    }

    /// The pointer moved to (`x`, `y`); `over` is what it is over now.
    pub fn motion(&mut self, x: i32, y: i32, over: Option<Hit>) -> Event {
        self.point = Some((x, y));
        let before = self.shown();
        self.pointer.motion(self.light(over));
        let lit = self.redraw_if(before);
        if let Some(Hit {
            item,
            part: Part::Combo(combo::Hit::Row(r)),
            ..
        }) = over
            && self.drag.is_none()
            && let Some(Control::Combo(c)) = self.items.get_mut(item).map(|it| &mut it.control)
            && c.hover(r)
        {
            return Event::Redraw;
        }
        let Some(drag) = self.drag else {
            return lit;
        };
        let i = match drag {
            Drag::TrackBar(i)
            | Drag::Bar { item: i, .. }
            | Drag::Edit(i)
            | Drag::Memo { item: i, .. }
            | Drag::Arrange(i) => i,
        };
        if let (Control::Memo(memo), Drag::Memo { thumb, span, .. }) =
            (&self.items[i].control, drag)
        {
            let Some(to) = span.drag(self.memo_view_1x(memo), &thumb, (x, y)) else {
                return lit;
            };
            let to = self.memo_view_device(memo, to);
            if to == memo.view.get() {
                return lit;
            }
            memo.view.set(to);
            return Event::Redraw;
        }
        let rect = self.items[i].placed(&self.rendered.controls.get());
        let id = self.items[i].id;
        match (&mut self.items[i].control, drag) {
            (Control::Edit { edit, .. }, Drag::Edit(_)) => {
                // The host hit-tested the pointer already, so the index
                // under it arrives in `over`; past either edge the sweep
                // runs to that end of the value.
                let to = match over {
                    Some(h) if h.item == i => match h.part {
                        Part::Edit(at) => at,
                        _ => return lit,
                    },
                    _ if x < rect.x => 0,
                    _ => edit.text.len(),
                };
                if edit.place(to, true) {
                    return Event::Redraw;
                }
            }
            (Control::TrackBar(t), Drag::TrackBar(_)) => {
                let lay = trackbar::layout(t, rect.x, rect.y, rect.width);
                if t.set(trackbar::position_at(&lay, x)) {
                    return Event::Changed(id);
                }
            }
            (Control::Arrange(a), Drag::Arrange(_)) => {
                if a.drag(x, y) {
                    return Event::Redraw;
                }
            }
            (Control::ListBox { list, .. }, Drag::Bar { thumb, .. }) => {
                let Some(grab) = thumb.grab(Axis::Vertical) else {
                    return lit;
                };
                let lay = listbox::layout(list, rect, &self.rendered.controls.get());
                let top = listbox::top_at(list, &lay, y, grab);
                if top != list.top {
                    list.top = top;
                    return Event::Redraw;
                }
            }
            (Control::Combo(c), Drag::Bar { thumb, .. }) => {
                let lay = combo::layout(c, rect, &self.rendered.controls.get());
                if c.drag_bar(&lay, &thumb, y) {
                    return Event::Redraw;
                }
            }
            (Control::Tree(tv), Drag::Bar { thumb, .. }) => {
                let Some(grab) = thumb.grab(Axis::Vertical) else {
                    return lit;
                };
                let lay = treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                let top = tv.top_at(&lay, y, grab);
                if top != tv.top {
                    tv.top = top;
                    return Event::Redraw;
                }
            }
            _ => {}
        }
        lit
    }

    /// The second press of a double click. On a list box row the row is
    /// taken and the dialog's default button acts; in an edit field the
    /// word under the pointer is selected. Anywhere else it is a plain
    /// press.
    pub fn double_click(&mut self, hit: Option<Hit>) -> Event {
        if let Some(Hit {
            item,
            part: Part::Edit(at),
            ..
        }) = hit
            && let Control::Edit {
                edit,
                enabled: true,
                ..
            } = &mut self.items[item].control
        {
            edit.select_word_at(at);
            self.set_focus(Some(item));
            self.drag = None;
            return Event::Redraw;
        }
        if let Some(Hit {
            item,
            part: Part::List(listbox::Hit::Row(_)),
            ..
        }) = hit
            && matches!(self.items[item].control, Control::ListBox { .. })
        {
            self.click(hit);
            return Event::Default;
        }
        self.click(hit)
    }

    /// The wheel turned `rows` rows (down positive) over `hit`. An open combo's list scrolls
    /// wherever the pointer is; an unfocused closed combo ignores the wheel.
    pub fn wheel(&mut self, hit: Option<Hit>, rows: i32) -> Event {
        if let Some(i) = self.open_combo()
            && let Control::Combo(c) = &mut self.items[i].control
        {
            return if c.scroll(rows) {
                Event::Redraw
            } else {
                Event::None
            };
        }
        let Some(h) = hit else {
            return Event::None;
        };
        let i = h.item;
        let cm = self.rendered.controls.get();
        let rect = self.items[i].placed(&cm);
        let focused = self.focus() == Some(i);
        let moved = match &mut self.items[i].control {
            Control::Combo(_) if !focused => false,
            Control::Combo(c) => {
                let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
                let mut changed = false;
                for _ in 0..notches.abs() {
                    changed |= c.step(notches > 0);
                }
                return if changed {
                    Event::Changed(self.items[i].id)
                } else {
                    Event::None
                };
            }
            Control::ListBox { list, .. } => {
                let visible = listbox::layout(list, rect, &cm).visible;
                list.scroll(rows, visible)
            }
            Control::Tree(tv) => {
                let lay = treeview::layout_without_text(tv, rect, &cm);
                tv.scroll(rows as i64, &lay)
            }
            Control::IconList(l) => {
                let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
                l.scroll(notches as i64, &iconlist::layout(rect, &cm))
            }
            Control::Memo(memo) if memo.enabled => {
                let mut view = memo.view.get();
                let moved = view.scroll_lines(rows as i64, &memo.edit, memo.rows.get());
                memo.view.set(view);
                moved
            }
            _ => false,
        };
        if moved { Event::Redraw } else { Event::None }
    }

    fn grab_bar(&mut self, i: usize, thumb: ThumbDrag) {
        if thumb.holding() {
            self.drag = Some(Drag::Bar { item: i, thumb });
        }
    }

    /// The pointer let go. A held push button, check box or radio acts only when released over
    /// it; everything else acted on the press.
    pub fn release(&mut self) -> Event {
        if let Some(Drag::Arrange(i)) = self.drag.take()
            && let Some(it) = self.items.get_mut(i)
            && let Control::Arrange(a) = &mut it.control
            && a.drop_held()
        {
            self.pointer.release();
            return Event::Changed(it.id);
        }
        let before = self.shown();
        let acts = |f: &Form, i: usize| {
            matches!(
                f.items.get(i).map(|it| &it.control),
                Some(Control::Button { .. } | Control::Check { .. } | Control::Radio { .. })
            )
        };
        let held = self.pointer.held().is_some_and(|(i, _)| acts(self, i));
        match self.pointer.release() {
            Some((i, Light::Body)) if acts(self, i) => self.act(i),
            _ if held => Event::Redraw,
            _ => self.redraw_if(before),
        }
    }

    /// A push button, check box or radio released over: a greyed one does nothing.
    fn act(&mut self, i: usize) -> Event {
        let id = self.items[i].id;
        match &mut self.items[i].control {
            Control::Button { enabled: true, .. } => Event::Pressed(id),
            Control::Check {
                checked,
                enabled: true,
                mixed,
                ..
            } => {
                toggle_check(checked, mixed);
                Event::Changed(id)
            }
            Control::Radio { enabled: true, .. } => {
                // Same convention as the keys: re-picking the radio that
                // is already checked mutates nothing, so it is a Redraw.
                let ch = self.pick_radio(i);
                self.changed(i, ch)
            }
            _ => Event::Redraw,
        }
    }
}

/// A field's scroll offset, guarded: `first` is a draw-time cache, and
/// the value under it may have been replaced since the last frame set
/// it. An offset past the end, or inside a character, starts again at 0.
pub(crate) fn visible_from(edit: &TextEdit, first: &Cell<usize>) -> usize {
    let n = first.get();
    if n <= edit.text.len() && edit.text.is_char_boundary(n) {
        n
    } else {
        0
    }
}

const BULLET: &str = "\u{25cf}";

/// The bullets a masked field shows, with caret and anchor mapped by char index; `first`
/// indexes these bullets. Secret itself, so the field shows no preedit and
/// reports no text to an input method.
fn masked(edit: &TextEdit) -> TextEdit {
    let at = |i: usize| edit.text[..i].chars().count() * BULLET.len();
    let mut m = TextEdit::secret();
    m.text.push_str(&BULLET.repeat(edit.text.chars().count()));
    m.set_selection(at(edit.anchor()), at(edit.caret()));
    m
}

fn unmasked_index(edit: &TextEdit, at: usize) -> usize {
    edit.text
        .char_indices()
        .nth(at / BULLET.len())
        .map_or(edit.text.len(), |(i, _)| i)
}

fn check_box(r: Rect, cm: &ControlMetrics) -> Rect {
    Rect::new(r.x, r.y, cm.check, cm.check)
}

/// A check box's label: `cm.check_label` right of its item rect's left
/// edge, `w` wide, as tall as the item rect; `cm.check_wrap_dy` lower for
/// a label that runs onto a second line (`indent` [`Indent::Wraps`]).
fn check_label_rect(r: Rect, indent: Indent, cm: &ControlMetrics, w: i32) -> Rect {
    let dy = if indent == Indent::Wraps {
        cm.check_wrap_dy
    } else {
        0
    };
    Rect::new(r.x + cm.check_label, r.y + dy, w, r.height)
}

/// A click, Space or access key on a check box: a two-state box flips, a
/// three-state one takes the next of indeterminate, off, on.
fn toggle_check(checked: &mut bool, mixed: &mut Option<bool>) {
    let state = match (*mixed, *checked) {
        (Some(true), _) => CheckState::Mixed,
        (_, true) => CheckState::On,
        (_, false) => CheckState::Off,
    };
    let next = state.cycled(mixed.is_some());
    *checked = next == CheckState::On;
    if let Some(m) = mixed {
        *m = next == CheckState::Mixed;
    }
}

/// A radio button's circle: `cm.radio` px square at its item rect's top,
/// `cm.radio_x` right of its left edge.
fn radio_box(r: Rect, cm: &ControlMetrics) -> Rect {
    Rect::new(r.x + cm.radio_x, r.y, cm.radio, cm.radio)
}

/// A radio button's label: `cm.radio_label` right of its item rect's left
/// edge, `w` wide, as tall as the item rect or the circle, whichever is
/// taller, so a 13 px circle's label sits on the same row as a check box's.
fn radio_label_rect(r: Rect, cm: &ControlMetrics, w: i32) -> Rect {
    Rect::new(r.x + cm.radio_label, r.y, w, r.height.max(cm.radio))
}

/// A check box's or radio's one-line label at `lr`'s origin (device px,
/// as tall as the item's row), greyed as the style's `part` greys it when
/// not `enabled`, with the focus dots round it when `cue`.
#[expect(clippy::too_many_arguments)]
fn draw_choice_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lr: Rect,
    label: &str,
    enabled: bool,
    part: crate::style::Part,
    cue: bool,
    px: u32,
) {
    let (cs, s) = (p.cs, p.s);
    let (plain, _) = crate::text::split_mnemonic(label);
    let lw = text.measure(&plain, false, px);
    let r = Rect::new(lr.x, lr.y, lw + 2 * s, lr.height);
    if enabled {
        draw_label(c, text, r, label, false, cs.button_text, Align::Left, px, s);
    } else {
        draw_disabled_label(c, text, p, r, label, Align::Left, px, Some(part));
    }
    if cue {
        draw_focus_ring(
            c,
            Rect::new(r.x - s, r.y, r.width + 2 * s, r.height),
            s,
            cs.button_text,
        );
    }
}

/// Draw the form onto `c`, already filled with face (and the page `p.back` names on a
/// sheet). Popups draw last. `cues` shows the dotted focus rectangles, hidden until the
/// keyboard is used.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, cues: bool, px: u32) {
    let Paint { cs, s, .. } = *p;
    form.drawn_with(p);
    let cm = p.look.controls();
    let text = text.dialog(form.font);
    let at_ink = |r: Rect| {
        if form.labels_at_ink {
            Rect::new(r.x - text.lead() * s, r.y, r.width, r.height)
        } else {
            r
        }
    };
    for (i, it) in form.items.iter().enumerate() {
        let focused = form.focus == Some(i);
        let cue = focused && cues;
        let r = it.placed(&cm).scaled(s);
        match &it.control {
            Control::Label {
                text: label,
                enabled,
                align,
            } => {
                let r = if *align == Align::Left { at_ink(r) } else { r };
                if *enabled {
                    draw_label(c, text, r, label, false, cs.button_text, *align, px, s);
                } else {
                    draw_disabled_label(c, text, p, r, label, *align, px, None);
                }
            }
            Control::Lines(lines) => {
                let r = at_ink(r);
                for (k, line) in lines.iter().enumerate() {
                    let lr = Rect::new(r.x, r.y + k as i32 * LINE_H * s, r.width, LINE_H * s);
                    text.draw(c, lr, line, false, cs.button_text, Align::Left, px);
                }
            }
            Control::GroupTitle(title) => {
                let dy = p.look.style().map_or(0, |st| st.group_label.nudge.1);
                let r = at_ink(Rect::new(r.x, r.y + dy * s, r.width, r.height));
                let ink = group_label_ink(p);
                draw_label(c, text, r, title, false, ink, Align::Left, px, s);
            }
            Control::Group(label) => {
                // A group box's label is laid out at its ink, 10 px in:
                // the UI font's pen sits there, the second dialog font's
                // a column left. The line's break opens 2 px before the
                // pen.
                let lw = text.measure(&crate::text::split_mnemonic(label).0, false, px);
                let lx = r.x + (10 - text.lead()) * s;
                let lr = Rect::new(lx, r.y - 5 * s, lw, LINE_H * s);
                draw_group_box(c, text, p, r, lr, label, px);
            }
            Control::Separator => c.etched_divider_t(r.x, r.right() - s, r.y, cs, s),
            Control::Rule => c.fill_rect(r, cs.button_shadow),
            Control::Blank => p.fill_back(c, r),
            Control::Check {
                label,
                checked,
                enabled,
                mixed,
            } => {
                let b = check_box(it.placed(&cm), &cm).scaled(s);
                let state = match (*mixed, *checked) {
                    (Some(true), _) => CheckState::Mixed,
                    (_, true) => CheckState::On,
                    (_, false) => CheckState::Off,
                };
                if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    draw_checkbox_state(c, p, b, state, ptr);
                } else {
                    draw_checkbox_state_disabled(c, p, b, state);
                }
                let lr = check_label_rect(it.placed(&cm), it.indent, &cm, 0).scaled(s);
                let part = crate::style::Part::CheckBox;
                draw_choice_label(c, text, p, lr, label, *enabled, part, cue, px);
            }
            Control::Radio {
                label,
                checked,
                enabled,
                ..
            } => {
                let b = radio_box(it.placed(&cm), &cm).scaled(s);
                if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    draw_radio(c, p, b, *checked, ptr);
                } else {
                    draw_radio_disabled(c, p, b, *checked);
                }
                let lr = radio_label_rect(it.placed(&cm), &cm, 0).scaled(s);
                let part = crate::style::Part::Radio;
                draw_choice_label(c, text, p, lr, label, *enabled, part, cue, px);
            }
            Control::Edit {
                edit,
                first,
                enabled,
            } => {
                let inner = if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    let inner = draw_edit_field(c, p, r, cs.window, focused, ptr);
                    c.fill_rect(inner, cs.window);
                    inner
                } else {
                    draw_disabled_field(c, p, r)
                };
                // A field without the focus hides its selection until the
                // focus comes back.
                let hidden;
                let unselected;
                let shown = if edit.is_secret() {
                    hidden = masked(edit);
                    &hidden
                } else if !focused && edit.has_selection() {
                    let mut e = edit.clone();
                    let caret = e.caret();
                    e.set_selection(caret, caret);
                    unselected = e;
                    &unselected
                } else {
                    edit
                };
                // Kept in step here because only this place holds what computes it; `inner` is the text
                // area in device px.
                // A drag resting on the field scrolls it (`drag_tick`), so the caret no longer pins
                // the view.
                let drop = form.field_drop.filter(|d| d.item == i);
                let from = visible_from(shown, first);
                let from = match drop {
                    Some(_) => from,
                    None => edit_first_visible(text, shown, inner.width, px, from),
                };
                first.set(from);
                let greyed = greyed_field_scheme(p, *enabled);
                let ep = match &greyed {
                    Some(gcs) => Paint { cs: gcs, ..*p },
                    None => *p,
                };
                draw_edit(
                    c,
                    text,
                    shown,
                    inner,
                    px,
                    &ep,
                    Caret::new(focused && *enabled, form.caret && drop.is_none()),
                    from,
                );
                if let Some(d) = drop {
                    let tail = &shown.text[from..];
                    let at = from + crate::textedit::index_at(text, tail, d.x * s - inner.x, px);
                    widgets::draw_edit_drop_point(c, text, shown, inner, px, p, from, at);
                }
            }
            Control::Combo(combo) => {
                let lay = combo::layout(combo, it.placed(&cm), &cm);
                let drop = form.pointer.of((i, Light::Drop));
                let frame = match drop {
                    Interaction::Idle => form.pointer.of((i, Light::Field)),
                    lit => lit,
                };
                combo::draw(
                    c,
                    text,
                    p,
                    combo,
                    &lay,
                    focused,
                    cue,
                    Caret::Unfocused,
                    combo::ComboLit { frame, drop },
                    px,
                );
            }
            Control::ListBox { list, icons } => {
                let lay = listbox::layout(list, it.placed(&cm), &cm);
                listbox::draw(c, text, p, list, &lay, icons, form.bar_lit(i), px);
            }
            Control::Tree(tv) => {
                let lay = treeview::layout_without_text(tv, it.placed(&cm), &cm);
                let bars = form
                    .bar_lit(i)
                    .map(|(hit, how)| ((Axis::Vertical, hit), how));
                treeview::draw(c, text, p, tv, &lay, bars, px);
            }
            Control::UpDown(u) => {
                let lay = updown::layout_for(u, it.placed(&cm), &cm);
                updown::draw(c, text, p, u, &lay, focused, form.spin_lit(i), px);
            }
            Control::TrackBar(t) => {
                let at = it.placed(&cm);
                let lay = trackbar::layout(t, at.x, at.y, at.width);
                draw_trackbar(
                    c,
                    p,
                    &lay,
                    t.enabled,
                    focused,
                    form.pointer.of((i, Light::Thumb)),
                );
                if cue {
                    draw_focus_ring(
                        c,
                        Rect::new(r.x - 4 * s, r.y - 2 * s, r.width + 8 * s, r.height + 4 * s),
                        s,
                        cs.button_text,
                    );
                }
            }
            Control::Button {
                label,
                enabled,
                default,
            } => {
                let ring = if form
                    .focus
                    .is_some_and(|f| matches!(form.items[f].control, Control::Button { .. }))
                {
                    focused
                } else {
                    *default
                };
                draw_button(
                    c,
                    text,
                    p,
                    r,
                    label,
                    ring,
                    form.pointer.of((i, Light::Body)),
                    *enabled,
                    cue,
                    px,
                );
            }
            Control::Picture { canvas, frame } => {
                let at = (r.x + frame * s, r.y + frame * s);
                match frame {
                    2 => draw_well(c, p, r, EdgeStyle::ButtonSunken),
                    1 => draw_well(c, p, r, EdgeStyle::ThinSunken),
                    _ => {}
                }
                if let Some(pic) = canvas {
                    c.blit_over(at.0, at.1, pic);
                }
            }
            Control::Spectrum => {
                draw_well(c, p, r, EdgeStyle::ThinSunken);
                let bar =
                    form.rendered
                        .get(i, s, (it.rect.width, it.rect.height), &it.control, || {
                            vec![crate::classic::assets::spectrum(s)]
                        });
                c.blit_canvas(r.x + s, r.y + s, &bar[0]);
            }
            Control::Monitor(screen) => {
                let (sx, sy, sw, sh) = crate::classic::assets::MONITOR_SCREEN;
                let area = Rect::new(r.x + sx * s, r.y + sy * s, sw * s, sh * s);
                let pics =
                    form.rendered
                        .get(i, s, (it.rect.width, it.rect.height), &it.control, || {
                            let fitted = match screen {
                                Screen::Picture(pic) => {
                                    pic.resample_box(area.width as u32, area.height as u32)
                                }
                                _ => Canvas::new(0, 0),
                            };
                            let bezel = crate::classic::assets::monitor();
                            let bezel =
                                bezel.resample_box(bezel.width * s as u32, bezel.height * s as u32);
                            vec![fitted, bezel]
                        });
                match screen {
                    Screen::Black => c.fill_rect(area, Color::rgb(0, 0, 0)),
                    Screen::Color(col) => c.fill_rect(area, *col),
                    Screen::Picture(_) => c.blit_canvas(area.x, area.y, &pics[0]),
                }
                c.blit_over(r.x, r.y, &pics[1]);
            }
            Control::Sample {
                themes,
                theme,
                icon,
            } => {
                let size = (it.rect.width - 4, it.rect.height - 4);
                let pic = form.rendered.get(i, s, size, &it.control, || {
                    vec![crate::look::draw_sample_at(
                        *themes,
                        theme,
                        s,
                        text,
                        icon.as_ref(),
                        size,
                    )]
                });
                draw_well(c, p, r, EdgeStyle::ButtonSunken);
                c.blit_canvas(r.x + 2 * s, r.y + 2 * s, &pic[0]);
            }
            Control::Color(b) => colorbutton::draw(c, p, it.placed(&cm), b),
            Control::Calendar(cal) => {
                let lay = calendar::layout(it.placed(&cm));
                calendar::draw(c, text, p, cal, &lay, cue, px);
            }
            Control::Clock(face) => clockface::draw(c, p, it.placed(&cm), *face),
            Control::Time(tf) => {
                let lay = timefield::layout(it.placed(&cm), &cm);
                timefield::draw(c, text, p, tf, &lay, focused, form.spin_lit(i), px);
            }
            Control::HotKey(h) => {
                let ptr = form.pointer.of((i, Light::Body));
                let field = hotkey::layout(it.placed(&cm));
                hotkey::draw(c, text, p, h, field, focused, form.caret, ptr, px);
            }
            Control::IconList(l) => {
                let lay = iconlist::layout(it.placed(&cm), &cm);
                iconlist::draw(c, p, l, &lay, cue, cm.thumb_min);
            }
            Control::Arrange(a) => {
                let lay = arrange::layout(it.placed(&cm), a);
                arrange::draw(c, text, p, a, &lay, cue, px);
            }
            Control::Memo(memo) => {
                let m = Memo::metrics(px, &cm);
                let lay = multiedit::layout(text, &m, &memo.edit, r, s);
                memo.rows.set(lay.rows);
                let mut view = memo.view.get();
                let caret = memo.edit.caret();
                if memo.followed.get() != Some(caret) {
                    view.follow(text, &m, &memo.edit, &lay, s);
                    memo.view.set(view);
                    memo.followed.set(Some(caret));
                }
                let greyed = (!memo.enabled).then(|| {
                    let base = greyed_field_scheme(p, false).unwrap_or_else(|| cs.clone());
                    ColorScheme {
                        window: cs.button_face,
                        ..base
                    }
                });
                let mp = match &greyed {
                    Some(gcs) => Paint { cs: gcs, ..*p },
                    None => *p,
                };
                let caret = Caret::new(focused && memo.enabled, form.caret);
                multiedit::draw(c, text, &mp, &m, &memo.edit, view, &lay, caret);
            }
        }
    }
    draw_open_popups(c, text, p, form, px);
}

/// The form's open popups, which [`draw`] puts over the other controls. A host drawing its
/// own control over the form draws them again after it.
pub fn draw_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    draw_open_popups(c, text.dialog(form.font), p, form, px);
}

fn draw_open_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    let cm = p.look.controls();
    for (i, it) in form.items.iter().enumerate() {
        match &it.control {
            Control::Combo(combo) if combo.open.is_some() => {
                let lay = combo::layout(combo, it.placed(&cm), &cm);
                combo::draw_list(
                    c,
                    text,
                    p,
                    &combo.items,
                    combo.open,
                    combo.top,
                    combo.rows,
                    lay.list,
                    lay.scrollbar,
                    form.bar_lit(i),
                    px,
                );
            }
            Control::Color(b) if b.open => {
                colorbutton::draw_popup(c, text, p, it.placed(&cm), b, px)
            }
            _ => {}
        }
    }
}

impl Form {
    /// The form's `access` part: one node per control under `parent`, each inside the group box
    /// whose rect holds it. A field without a label is labelled by the static text before it.
    /// Decoration is left out. Returns the node that has the focus.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> Option<NodeId> {
        let mut groups: Vec<(Rect, NodeId)> = Vec::new();
        let mut label: Option<NodeId> = None;
        let mut focus = None;
        let cm = self.rendered.controls.get();
        let text = text.dialog(self.font);
        for (i, it) in self.items.iter().enumerate() {
            let r = it.placed(&cm);
            let within = |g: &Rect| {
                g.contains(r.x, r.y) && r.right() <= g.right() && r.bottom() <= g.bottom()
            };
            let up = groups
                .iter()
                .rev()
                .find(|(g, _)| within(g))
                .map_or(parent, |(_, id)| *id);
            let key = (it.id, i);
            let (target, bare) = match &it.control {
                Control::Label {
                    text: t, enabled, ..
                } => {
                    label = Some(tree.push(up, key, access::label(r, t, *enabled)));
                    continue;
                }
                Control::Lines(lines) => {
                    label = Some(tree.push(up, key, access::label(r, &lines.join(" "), true)));
                    continue;
                }
                Control::GroupTitle(t) => {
                    label = Some(tree.push(up, key, access::label(r, t, true)));
                    continue;
                }
                Control::Group(title) => {
                    let id = tree.push(up, key, access::named(Role::Group, r, title));
                    groups.push((r, id));
                    label = None;
                    continue;
                }
                Control::Separator
                | Control::Rule
                | Control::Blank
                | Control::Picture { .. }
                | Control::Monitor(_)
                | Control::Spectrum
                | Control::Sample { .. }
                | Control::Clock(_) => continue,
                Control::Check {
                    label: l,
                    checked,
                    enabled,
                    mixed,
                } => {
                    let lw = text.measure(&crate::text::split_mnemonic(l).0, false, px);
                    let cm = self.rendered.controls.get();
                    let b = check_box(r, &cm).union(&check_label_rect(r, it.indent, &cm, lw + 2));
                    let state = match (mixed, checked) {
                        (Some(true), _) => accesskit::Toggled::Mixed,
                        (_, c) => (*c).into(),
                    };
                    let n = access::toggle(Role::CheckBox, b, l, state, *enabled);
                    (tree.push(up, key, n), None)
                }
                Control::Radio {
                    label: l,
                    checked,
                    enabled,
                    ..
                } => {
                    let lw = text.measure(&crate::text::split_mnemonic(l).0, false, px);
                    let cm = self.rendered.controls.get();
                    let b = radio_box(r, &cm).union(&radio_label_rect(r, &cm, lw + 2));
                    let n = access::toggle(Role::RadioButton, b, l, (*checked).into(), *enabled);
                    (tree.push(up, key, n), None)
                }
                Control::Button {
                    label: l,
                    enabled,
                    default,
                } => {
                    let n = access::button(r, l, *enabled, *default);
                    (tree.push(up, key, n), None)
                }
                Control::IconList(l) => {
                    let lay = iconlist::layout(r, &cm);
                    let id = iconlist::access(l, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Arrange(a) => {
                    let lay = arrange::layout(r, a);
                    let id = arrange::access(a, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Memo(memo) => {
                    let m = Memo::metrics(px, &cm);
                    let lay = multiedit::layout(text, &m, &memo.edit, r, 1);
                    let id = multiedit::access(
                        &memo.edit,
                        self.memo_view_1x(memo),
                        &lay,
                        &m,
                        text,
                        tree,
                        up,
                        key,
                    );
                    (id, Some(id))
                }
                Control::Edit {
                    edit,
                    first,
                    enabled,
                } => {
                    // A masked field reads out its bullets, never the value.
                    let shown = if edit.is_secret() {
                        masked(edit)
                    } else {
                        edit.clone()
                    };
                    let from = visible_from(&shown, first);
                    let id = widgets::access_edit(
                        &shown,
                        from,
                        edit.is_secret(),
                        r,
                        *enabled,
                        text,
                        px,
                        tree,
                        up,
                        key,
                    );
                    (id, Some(id))
                }
                Control::Combo(c) => {
                    let lay = combo::layout(c, r, &self.rendered.controls.get());
                    let target = combo::access(c, &lay, text, px, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::ListBox { list, .. } => {
                    let lay = listbox::layout(list, r, &self.rendered.controls.get());
                    let target = listbox::access(list, &lay, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::Tree(tv) => {
                    let lay = treeview::layout_without_text(tv, r, &self.rendered.controls.get());
                    let target = treeview::access(tv, &lay, text, px, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::UpDown(u) => {
                    let cm = self.rendered.controls.get();
                    let id = updown::access(u, &updown::layout_for(u, r, &cm), tree, up, key);
                    (id, Some(id))
                }
                Control::TrackBar(t) => {
                    let lay = trackbar::layout(t, r.x, r.y, r.width);
                    let id = trackbar::access(t, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Color(b) => {
                    let id = colorbutton::access(b, r, tree, up, key);
                    (id, Some(id))
                }
                Control::Calendar(cal) => {
                    let id = calendar::access(cal, &calendar::layout(r), text, px, tree, up, key);
                    (id, Some(id))
                }
                Control::Time(tf) => {
                    let cm = self.rendered.controls.get();
                    let lay = timefield::layout(r, &cm);
                    let id = timefield::access(tf, &lay, text, px, tree, up, key);
                    (id, Some(id))
                }
                Control::HotKey(h) => {
                    let focused = self.focus == Some(i);
                    let id = hotkey::access(h, hotkey::layout(r), focused, text, px, tree, up, key);
                    (id, Some(id))
                }
            };
            if let (Some(l), Some(n)) = (label, bare.and_then(|b| tree.get_mut(b))) {
                n.push_labelled_by(l);
            }
            label = None;
            if self.focus == Some(i) {
                focus = Some(target);
            }
        }
        focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    /// A memo takes typing, Space, a pasted line break and Ctrl+Enter as text; Enter is still
    /// the dialog's default and Tab still leaves it. It draws in a 2 px sunken frame with a
    /// 16 px vertical scrollbar; a greyed memo takes no key.
    #[test]
    fn a_memo_edits_lines_and_keeps_its_scrollbar() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut f = Form::new(vec![
            Item::memo("m", Rect::new(0, 0, 263, 80), "", true),
            Item::button("ok", Rect::new(0, 90, 75, 23), "OK", true, true),
        ]);
        let memo = |f: &Form| match &f.items[0].control {
            Control::Memo(m) => m.edit.text(),
            _ => unreachable!(),
        };
        assert_eq!(f.key(Key::Char('a')), Event::Changed("m"));
        f.key(Key::Space);
        assert_eq!(f.paste("b\ncd".into()), Event::Changed("m"));
        assert_eq!(memo(&f), "a b\ncd");
        f.key(Key::Up);
        f.key(Key::Char('X'));
        assert_eq!(memo(&f), "a Xb\ncd", "the caret kept its column");
        assert_eq!(f.key(Key::Enter), Event::Default);
        assert_eq!(f.key(Key::NewLine), Event::Changed("m"));
        assert_eq!(memo(&f), "a X\nb\ncd", "Ctrl+Enter breaks the line");
        let edge = |end| Key::DocEdge { end, select: false };
        assert_eq!(f.key(edge(false)), Event::Redraw);
        f.key(Key::Char('<'));
        f.key(edge(true));
        f.key(Key::Char('>'));
        assert_eq!(memo(&f), "<a X\nb\ncd>", "Ctrl+Home and Ctrl+End");
        assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus(), Some(1));
        assert_eq!(
            f.key(Key::NewLine),
            Event::Pressed("ok"),
            "off the memo it is Enter"
        );

        let mut c = Canvas::new(263, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        let text = TextRenderer::load(&FontConfig::default());
        draw(&mut c, &text, &classic_paint(cs, 1), &f, false, 11);
        assert_eq!(c.get(0, 0), cs.button_shadow.argb(), "the sunken frame");
        assert_eq!(c.get(10, 40), cs.window.argb(), "the white box");
        assert_ne!(c.get(260 - 8, 40), cs.window.argb(), "the scrollbar");
        assert_eq!(c.get(263 - 2 - 16 - 1, 40), cs.window.argb());

        let mut greyed = Form::new(vec![Item::memo("m", Rect::new(0, 0, 263, 80), "", false)]);
        assert_eq!(greyed.focus(), None);
        assert_eq!(greyed.key(Key::Char('a')), Event::None);
    }

    /// At scale 2 a memo's selection ends where its text does, at the
    /// width the device font measures; a line scrolled sideways to its
    /// caret shows its end, and a click on a character lands on it.
    #[test]
    fn a_memo_at_scale_two_lines_its_selection_and_clicks_up_with_its_text() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = tr();
        let mut f = Form::new(vec![Item::memo("m", Rect::new(0, 0, 120, 40), "abc", true)]);
        f.key(Key::Edit(EditKey::SelectAll));
        let shot = |f: &Form| {
            let mut c = Canvas::new(240, 80);
            draw(&mut c, &t, &classic_paint(cs, 2), f, false, 2 * PX);
            c
        };
        let c = shot(&f);
        // The text starts a device px inside the 2 px (4 device px) edge.
        let x0 = 4 + 2;
        let row = 4 + 2 + 6;
        let lit: Vec<i32> = (0..240)
            .filter(|&x| c.get(x, row) == cs.hilight.argb())
            .collect();
        let w = t.measure("abc", false, 2 * PX);
        assert_eq!(lit.first(), Some(&x0));
        assert_eq!(lit.last(), Some(&(x0 + w - 1)), "the run is {w} px wide");

        // A line three boxes long, the caret at its end: the view scrolls
        // it left, and a click at the box's left lands on a character
        // there, not near the start.
        let long = "m".repeat(80);
        let Control::Memo(m) = &mut f.items[0].control else {
            unreachable!()
        };
        m.edit = multiedit::MultiEdit::new(&long);
        m.edit.place(
            multiedit::Pos {
                line: 0,
                col: long.len(),
            },
            false,
        );
        shot(&f);
        let left = match &f.items[0].control {
            Control::Memo(m) => m.view.get().left,
            _ => unreachable!(),
        };
        let cell = t.measure("m", false, PX);
        let hit = f.hit(3 + cell / 2, 10, &t, PX).expect("the memo");
        let Part::Memo(at) = hit.part else {
            panic!("a text position, not {:?}", hit.part)
        };
        let want = (left / 2 / cell) as usize;
        assert!(
            at.col.abs_diff(want) <= 1,
            "the click lands at column {} for a view {left} device px in, near {want}",
            at.col
        );
    }

    /// A press on a memo's scrollbar scrolls it and leaves the caret
    /// where it was; the next draw keeps the view there.
    #[test]
    fn a_press_on_a_memos_scrollbar_scrolls_without_moving_the_caret() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = tr();
        let text: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
        let r = Rect::new(0, 0, 200, 80);
        let mut f = Form::new(vec![Item::memo("m", r, &text.join("\n"), true)]);
        let shot = |f: &Form| {
            let mut c = Canvas::new(200, 80);
            draw(&mut c, &t, &classic_paint(cs, 1), f, false, PX);
        };
        shot(&f);
        let memo = |f: &Form| match &f.items[0].control {
            Control::Memo(m) => (m.edit.caret(), m.view.get().top),
            _ => unreachable!(),
        };
        let start = multiedit::Pos::default();
        let lay = multiedit::layout(
            &t,
            &Memo::metrics(PX, &ControlMetrics::CLASSIC),
            &{
                let Control::Memo(m) = &f.items[0].control else {
                    unreachable!()
                };
                m.edit.clone()
            },
            r,
            1,
        );
        let vs = lay.vscroll.expect("the memo keeps its vertical bar");
        for (y, top) in [(vs.bottom() - 4, 1), (vs.bottom() - 20, 1 + lay.rows)] {
            let hit = f.hit(vs.x + 4, y, &t, PX);
            assert_eq!(f.click(hit), Event::Redraw);
            f.release();
            shot(&f);
            assert_eq!(memo(&f), (start, top), "a press at y {y}");
        }
    }

    /// A memo's thumb, grabbed, follows the pointer down its bar, on it or
    /// off to the side, until the release; the caret stays put. At scale
    /// 2 the horizontal thumb dragged to the track's end shows the long
    /// line's tail, in device px.
    #[test]
    fn a_memos_thumb_drags_its_view() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = tr();
        let lines: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
        let r = Rect::new(0, 0, 200, 80);
        let mut f = Form::new(vec![Item::memo("m", r, &lines.join("\n"), true)]);
        let shot = |f: &Form, s: i32| {
            let mut c = Canvas::new(200 * s as u32, 80 * s as u32);
            draw(&mut c, &t, &classic_paint(cs, s), f, false, s as u32 * PX);
        };
        let memo = |f: &Form| match &f.items[0].control {
            Control::Memo(m) => (m.edit.caret(), m.view.get()),
            _ => unreachable!(),
        };
        let parts = |f: &Form, bar| {
            let Control::Memo(m) = &f.items[0].control else {
                unreachable!()
            };
            let mm = Memo::metrics(PX, &ControlMetrics::CLASSIC);
            let lay = multiedit::layout(&t, &mm, &m.edit, r, 1);
            let v = f.memo_view_1x(m);
            (
                lay.rows,
                multiedit::bar_parts(&t, &mm, &m.edit, &lay, v, bar).unwrap(),
            )
        };
        shot(&f, 1);
        let (rows, p) = parts(&f, multiedit::ScrollBar::Vertical);
        let thumb = p.thumb.expect("12 lines overflow the box");
        let (gx, gy) = (thumb.x + 4, thumb.y + 3);
        assert_eq!(
            f.click(f.hit(gx, gy, &t, PX)),
            Event::None,
            "the press moves nothing"
        );
        assert!(
            matches!(f.drag, Some(Drag::Memo { .. })),
            "the press grabs the thumb"
        );
        let dy = p.track.height - thumb.height;
        assert_eq!(f.motion(gx, gy + dy, None), Event::Redraw);
        assert_eq!(
            memo(&f).1.top,
            12 - rows,
            "the track's end shows the last page"
        );
        assert_eq!(f.motion(gx - 300, gy, None), Event::Redraw);
        assert_eq!(memo(&f).1.top, 0, "off the bar its y still steers");
        f.release();
        assert_eq!(f.motion(gx, gy + dy, None), Event::None, "let go, it stays");
        assert_eq!(
            memo(&f),
            (multiedit::Pos::default(), multiedit::View::default())
        );

        let Control::Memo(m) = &mut f.items[0].control else {
            unreachable!()
        };
        m.edit = multiedit::MultiEdit::new(&"m".repeat(80));
        shot(&f, 2);
        let (_, p) = parts(&f, multiedit::ScrollBar::Horizontal);
        let thumb = p.thumb.expect("the line overflows the box");
        f.click(f.hit(thumb.x + 2, thumb.y + 4, &t, PX));
        assert_eq!(
            f.motion(p.track.right() + 50, thumb.y + 4, None),
            Event::Redraw
        );
        let Control::Memo(m) = &f.items[0].control else {
            unreachable!()
        };
        let mm = Memo::metrics(PX, &ControlMetrics::CLASSIC);
        let lay = multiedit::layout(&t, &mm, &m.edit, r, 1);
        let total = multiedit::widest_px(&t, &mm, &m.edit) + 2;
        assert_eq!(memo(&f).1.left, 2 * (total - lay.inner.width));
    }

    /// A focused memo is the form's focused edit, so the host's clipboard
    /// copies its selection and its caret blinks; a password field hands
    /// nothing out.
    #[test]
    fn a_focused_memo_is_the_focused_edit() {
        let mut f = Form::new(vec![
            Item::memo("m", Rect::new(0, 0, 263, 80), "one\ntwo", true),
            Item::edit("pw", Rect::new(0, 90, 100, 20), "secret", true),
        ]);
        f.key(Key::Edit(EditKey::SelectAll));
        let copied = f.focused_edit().and_then(|e| e.copy_text());
        assert_eq!(copied.as_deref(), Some("one\ntwo"));
        if let Some(Control::Edit { edit, .. }) = f.item_mut("pw").map(|i| &mut i.control) {
            *edit = TextEdit::secret();
            edit.paste("abc", &[]);
            edit.key(EditKey::SelectAll);
        }
        f.focus_id("pw");
        assert!(f.focused_edit().is_some());
        assert_eq!(f.focused_edit().and_then(|e| e.copy_text()), None);
    }

    /// A form set in the second dialog font draws its items in it; the
    /// same form in the UI font draws the UI font's.
    #[test]
    fn a_form_draws_in_its_dialog_font() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let stub = TextRenderer::stub(11, |ch| if ch == 'E' { 7 } else { 8 });
        let text = TextRenderer::load(&FontConfig::default()).with_shell_dlg(stub);
        let ink = |font: DialogFont| -> Vec<(i32, i32)> {
            let form = Form::new(vec![Item::label_at("l", 10, 10, 60, "EN")]).set_in(font);
            let mut c = Canvas::new(80, 30);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &text, &classic_paint(cs, 1), &form, false, 11);
            (0..80)
                .flat_map(|x| (0..30).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
                .collect()
        };
        let pens: Vec<i32> = ink(DialogFont::ShellDlg).iter().map(|p| p.0).collect();
        assert_eq!(pens, vec![10, 17]);
        assert!(
            ink(DialogFont::Ui).len() > 2,
            "the UI font inks whole glyphs"
        );
    }

    /// A group box's label, and a label in a form that lays labels out at their ink, ink from
    /// the same column in the second dialog font as in the UI font. In the second font the line's
    /// break opens 7 px in, in the UI font 8.
    #[test]
    fn labels_at_their_ink_start_on_the_same_column_in_either_dialog_font() {
        let path = "/usr/share/wine/fonts/sserife.fon";
        if !crate::text::test_tahoma() || !std::path::Path::new(path).exists() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let tahoma = TextRenderer::load(&FontConfig::default());
        let second = TextRenderer::load(&FontConfig {
            shell_dlg: Some(path.into()),
            ..FontConfig::default()
        });
        let frame = Rect::new(20, 30, 200, 60);
        let items = || {
            vec![
                Item::group("g", frame, "Sound playback"),
                Item::label_at("l", 7, 100, 60, "Fi&nd what:"),
            ]
        };
        let draw_with = |text: &TextRenderer, form: Form| {
            let mut c = Canvas::new(240, 120);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, text, &classic_paint(cs, 1), &form, false, 11);
            c
        };
        // The first column with label ink in rows `ys`.
        let first_ink = |c: &Canvas, ys: std::ops::Range<i32>| {
            (0..240).find(|&x| ys.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
        };
        // The first column of the frame's top line that is not its shadow.
        let gap = |c: &Canvas| {
            (frame.x..frame.right()).find(|&x| c.get(x, frame.y) != cs.button_shadow.argb())
        };
        let at_ink = || {
            Form::new(items())
                .set_in(DialogFont::ShellDlg)
                .labels_at_ink()
        };
        for (name, text, break_at) in [("second", &second, 27), ("Tahoma", &tahoma, 28)] {
            let c = draw_with(text, at_ink());
            assert_eq!(first_ink(&c, 22..38), Some(30), "{name}: the group label");
            assert_eq!(gap(&c), Some(break_at), "{name}: the break");
            assert_eq!(first_ink(&c, 95..115), Some(7), "{name}: the label");
        }
        let at_pen = Form::new(items()).set_in(DialogFont::ShellDlg);
        let c = draw_with(&second, at_pen);
        assert_eq!(first_ink(&c, 95..115), Some(8), "a label at its pen");
    }

    /// A check box's label starts 5 px past the 13 px box: "Read-only"
    /// inks its first column 18 px right of the box's left edge, and a
    /// three-state box draws its indeterminate state with the checkerboard.
    #[test]
    fn a_check_boxs_label_starts_five_px_past_the_box() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let mut form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only")]);
        let mut c = Canvas::new(100, 40);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &form, false, px);
        let ink = (24..60).find(|&x| (8..26).any(|y| c.get(x, y) == cs.button_text.argb()));
        assert_eq!(ink, Some(28));
        if let Some(Control::Check { mixed, .. }) = form.item_mut("ro").map(|i| &mut i.control) {
            *mixed = Some(true);
        }
        draw(&mut c, &text, &classic_paint(cs, 1), &form, false, px);
        assert_eq!(c.get(12, 12), cs.window.argb());
        assert_eq!(c.get(13, 12), cs.button_face.argb(), "the checkerboard");
        assert_eq!(form.check_state("ro"), Some(CheckState::Mixed));
    }

    /// A greyed Blue edit fills with what sits behind it inside its line: the page's own
    /// pixels on a page, the face on the dialog.
    #[test]
    fn a_greyed_blue_edit_shows_the_page_or_the_face_behind_it() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let p = theme.paint();
        let face = theme.colors.button_face;
        let page = Rect::new(9, 49, 386, 495);
        let greyed = |id, r| {
            Item::new(
                id,
                r,
                Control::Edit {
                    edit: TextEdit::new("", false),
                    first: std::cell::Cell::new(0),
                    enabled: false,
                },
            )
        };
        let high = Rect::new(33, 100, 185, 23);
        let low = Rect::new(24, 512, 173, 20);
        let form = Form::new(vec![greyed("high", high), greyed("low", low)]);
        let mut bare = Canvas::new(400, 560);
        bare.fill_rect(bare.bounds(), face);
        crate::look::widgets::draw_page(&mut bare, &p, page);
        let mut c = bare.clone();
        draw(&mut c, &text, &p.on_page(page), &form, false, px);
        for (r, top, bottom) in [
            (high, 0xFFFB_FCFB, 0xFFFA_FBFA),
            (low, 0xFFF5_F3ED, 0xFFF5_F3ED),
        ] {
            let inside = r.inset(1);
            for y in inside.y..inside.bottom() {
                for x in inside.x..inside.right() {
                    assert_eq!(c.get(x, y), bare.get(x, y), "({x}, {y}) is the page");
                }
            }
            assert_eq!(
                c.get(inside.x + 5, inside.y),
                top,
                "{r:?}: the page's top row"
            );
            let last = inside.bottom() - 1;
            assert_eq!(
                c.get(inside.x + 5, last),
                bottom,
                "{r:?}: the page's bottom row"
            );
            assert_eq!(
                c.get(r.x, r.y + 10),
                crate::blue::Variant::Blue
                    .scheme()
                    .edit_border
                    .normal
                    .rim
                    .argb(),
                "{r:?}: the field's line"
            );
        }
        let mut dialog = Canvas::new(400, 560);
        dialog.fill_rect(dialog.bounds(), face);
        draw(&mut dialog, &text, &p, &form, false, px);
        let inside = high.inset(1);
        for y in inside.y..inside.bottom() {
            for x in inside.x..inside.right() {
                assert_eq!(dialog.get(x, y), face.argb(), "({x}, {y}) is the face");
            }
        }
    }

    /// In Blue a radio shares a check box's column: for a box at x 10 and
    /// a radio item at x 11 (where Classic's circle is, a pixel right of
    /// the box), the circle starts at x 10 too, and "Read-only" and "Up"
    /// both ink their first column 16 px past it, at 26.
    #[test]
    fn a_blue_check_box_and_radio_share_a_column() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let form = Form::new(vec![
            Item::check("ro", 10, 10, "&Read-only"),
            Item::radio("up", 11, 40, "&Up", "dir"),
        ]);
        let mut c = Canvas::new(100, 60);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &theme.paint(), &form, false, px);
        let ink = |rows: std::ops::Range<i32>| {
            (24..60).find(|&x| rows.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
        };
        let left = |y0: i32| {
            (0..24).find(|&x| (y0..y0 + 13).any(|y| c.get(x, y) != cs.button_face.argb()))
        };
        assert_eq!(left(10), Some(10), "the box starts at x 10");
        assert_eq!(left(40), Some(10), "the circle starts at x 10");
        assert_eq!(ink(8..26), Some(26), "the check box label inks from x 26");
        assert_eq!(ink(38..56), Some(26), "the radio label inks from x 26");
    }

    /// In Glass too a radio shares a check box's column: for a box at x 10
    /// and a radio item at x 11, the 12 px circle starts at x 10, and
    /// "Read-only" and "Up" both ink their first column 16 px past it.
    #[test]
    fn a_glass_check_box_and_radio_share_a_column() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let form = Form::new(vec![
            Item::check("ro", 10, 10, "&Read-only"),
            Item::radio("up", 11, 40, "&Up", "dir"),
        ]);
        let mut c = Canvas::new(100, 60);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &theme.paint(), &form, false, px);
        let ink = |rows: std::ops::Range<i32>| {
            (24..60).find(|&x| rows.clone().any(|y| c.get(x, y) == cs.button_text.argb()))
        };
        let left = |y0: i32| {
            (0..24).find(|&x| (y0..y0 + 13).any(|y| c.get(x, y) != cs.button_face.argb()))
        };
        assert_eq!(left(10), Some(10), "the box starts at x 10");
        assert_eq!(left(40), Some(10), "the circle starts at x 10");
        assert_eq!(ink(8..26), Some(26), "the check box label inks from x 26");
        assert_eq!(ink(38..56), Some(26), "the radio label inks from x 26");
    }

    /// A greyed plain static label (`Control::Label`, not a check box or
    /// radio) keeps the etched pair under Blue, the same as Classic: a
    /// white copy a pixel down and right under the grey.
    #[test]
    fn a_greyed_blue_static_label_is_etched() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let form = Form::new(vec![
            Item::label_at("wait", 10, 10, 70, "Wait:").enabled(false),
        ]);
        let label = Rect::new(10, 10, 70, LINE_H);
        let count = |c: &Canvas, ink: u32| {
            (label.y..label.bottom())
                .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let mut c = Canvas::new(100, 40);
        c.fill_rect(c.bounds(), blue.colors.button_face);
        draw(&mut c, &text, &blue.paint(), &form, false, PX);
        assert!(count(&c, 0xFFFF_FFFF) > 0, "white pixels sit at +1,+1");
    }

    /// A greyed Blue check box inks its label once, flat in the greyed ink, with
    /// no white copy under it; the same box in Classic keeps the white
    /// copy a pixel down and right.
    #[test]
    fn a_greyed_blue_check_box_label_is_flat() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only").enabled(false)]);
        let label = Rect::new(24, 8, 70, 18);
        let count = |c: &Canvas, ink: u32| {
            (label.y..label.bottom())
                .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let mut c = Canvas::new(100, 40);
        c.fill_rect(c.bounds(), blue.colors.button_face);
        draw(&mut c, &text, &blue.paint(), &form, false, PX);
        let grey = crate::blue::Variant::Blue.scheme().inks.greyed_label.argb();
        assert!(count(&c, grey) > 0, "the label inks the greyed label ink");
        assert_eq!(count(&c, 0xFFFF_FFFF), 0, "no white pixel under the label");
        let classic = Theme::default();
        let mut c = Canvas::new(100, 40);
        c.fill_rect(c.bounds(), classic.colors.button_face);
        draw(&mut c, &text, &classic.paint(), &form, false, PX);
        assert!(
            count(&c, classic.colors.button_hilight.argb()) > 0,
            "Classic keeps the white copy"
        );
    }

    /// A greyed Blue radio button inks its label once, flat in the greyed ink,
    /// the same ink the art uses for a greyed check box label.
    #[test]
    fn a_greyed_blue_radio_label_is_flat() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let form = Form::new(vec![Item::radio("up", 10, 10, "&Up", "dir").enabled(false)]);
        let label = Rect::new(26, 8, 70, 18);
        let count = |c: &Canvas, ink: u32| {
            (label.y..label.bottom())
                .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let mut c = Canvas::new(100, 40);
        c.fill_rect(c.bounds(), blue.colors.button_face);
        draw(&mut c, &text, &blue.paint(), &form, false, PX);
        let grey = crate::blue::Variant::Blue.scheme().inks.greyed_label.argb();
        assert!(count(&c, grey) > 0, "the label inks the greyed label ink");
        assert_eq!(count(&c, 0xFFFF_FFFF), 0, "no white pixel under the label");
    }

    /// A greyed Glass check box and radio ink their labels once, flat in
    /// 6B6B6B, with no white copy under them.
    #[test]
    fn greyed_glass_check_box_and_radio_labels_are_flat_6b6b6b() {
        let text = TextRenderer::load(&FontConfig::default());
        if !crate::text::test_tahoma() {
            return;
        }
        let form = Form::new(vec![
            Item::check("ro", 10, 10, "&Read-only").enabled(false),
            Item::radio("up", 10, 40, "&Up", "dir").enabled(false),
        ]);
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let mut c = Canvas::new(100, 70);
        c.fill_rect(c.bounds(), glass.colors.button_face);
        draw(&mut c, &text, &glass.paint(), &form, false, PX);
        for (what, label) in [
            ("check box", Rect::new(24, 8, 70, 18)),
            ("radio", Rect::new(26, 38, 70, 18)),
        ] {
            let count = |ink: u32| {
                (label.y..label.bottom())
                    .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                    .filter(|&(x, y)| c.get(x, y) == ink)
                    .count()
            };
            assert!(count(0xFF6B_6B6B) > 0, "the {what} label inks 6B6B6B");
            assert_eq!(
                count(0xFFFF_FFFF),
                0,
                "no white pixel under the {what} label"
            );
        }
    }

    /// A greyed three-state box in its indeterminate state draws the
    /// look's greyed mixed box: Blue's CAC8BB square 3 px inside the box.
    #[test]
    fn a_greyed_mixed_blue_check_box_shows_its_square() {
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let mut form = Form::new(vec![Item::check("ro", 10, 10, "&Read-only").enabled(false)]);
        if let Some(Control::Check { mixed, .. }) = form.item_mut("ro").map(|i| &mut i.control) {
            *mixed = Some(true);
        }
        let mut c = Canvas::new(100, 40);
        draw(&mut c, &tr(), &blue.paint(), &form, false, PX);
        let faces = &crate::blue::Variant::Blue.scheme().check_box;
        let grey = 0xFF00_0000 | faces.disabled_mixed.rim.argb();
        let mark = 0xFF00_0000 | faces.disabled_mixed.mark.unwrap().argb();
        assert_eq!(c.get(10, 10), grey, "the greyed frame");
        assert_eq!(c.get(13, 13), mark, "the square starts 3 px in");
        assert_ne!(c.get(12, 12), mark, "the well round the square");
    }

    /// A Blue radio's label is centred on its 13 px circle, as a check
    /// box's is on its box: "&Up" inks its last row and its underline on
    /// the same rows below either item's top.
    #[test]
    fn a_blue_radio_label_sits_on_the_check_box_labels_row() {
        let text = tr();
        if !crate::text::test_tahoma() {
            return;
        }
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cs = &blue.colors;
        let form = Form::new(vec![
            Item::check("c", 10, 10, "&Up"),
            Item::radio("r", 10, 40, "&Up", "g"),
        ]);
        let mut c = Canvas::new(100, 70);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &blue.paint(), &form, false, PX);
        let last_ink = |top: i32| {
            (top..top + 20)
                .filter(|&y| (26..60).any(|x| c.get(x, y) == cs.button_text.argb()))
                .max()
                .map(|y| y - top)
        };
        assert!(last_ink(10).is_some(), "the check box label inks");
        assert_eq!(
            last_ink(40),
            last_ink(10),
            "the radio label's last row matches the check box label's"
        );
    }

    /// A form hit-tests its radios at the sizes of the look it last drew
    /// in: after a Blue draw, the 13 px circle's first column, a pixel
    /// left of the item, and the label 15 px in both hit the radio; before
    /// any draw, Classic's 12 px circle at the item and 17 px offset leave
    /// both columns empty.
    #[test]
    fn a_form_hits_its_radios_at_its_last_draws_sizes() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let f = radio_form();
        let up = f.index_of("up");
        assert_eq!(f.hit(9, 12, &tr(), PX).map(|h| h.item), None);
        assert_eq!(f.hit(25, 12, &tr(), PX).map(|h| h.item), None);
        let mut c = Canvas::new(120, 120);
        draw(&mut c, &tr(), &theme.paint(), &f, false, PX);
        assert_eq!(
            f.hit(9, 12, &tr(), PX).map(|h| h.item),
            up,
            "the circle's first column"
        );
        assert_eq!(
            f.hit(25, 12, &tr(), PX).map(|h| h.item),
            up,
            "the label 15 px in"
        );
    }

    /// `access` and `hit` follow the look of the form's latest draw: a
    /// Blue draw sets the labels 15 px (radio) and 16 px (check box) in,
    /// and a later Classic draw puts them back at 17 px and 18 px.
    #[test]
    fn a_forms_access_and_hit_follow_its_latest_draw() {
        use crate::access::{ROOT, Tree};
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let classic = Theme::default();
        let text = tr();
        let f = Form::new(vec![
            Item::radio("up", 10, 10, "&Up", "dir"),
            Item::check("case", 10, 40, "Match &case"),
        ]);
        let (up, case) = (f.index_of("up"), f.index_of("case"));
        // The right edge of the radio's and the check box's nodes, whose
        // bounds run from the box to the end of the label.
        let right_edges = || {
            let mut tree = Tree::window("Find", (200, 80));
            f.access(&mut tree, ROOT, &text, PX);
            let x1 = |role, name| {
                let id = tree.find(role, name).expect("the node");
                tree.get(id).unwrap().bounds().expect("bounds").x1
            };
            (
                x1(Role::RadioButton, "Up"),
                x1(Role::CheckBox, "Match case"),
            )
        };
        let hit = |x, y| f.hit(x, y, &text, PX).map(|h| h.item);
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &text, &classic.paint(), &f, false, PX);
        let (radio, check) = right_edges();
        draw(&mut c, &text, &blue.paint(), &f, false, PX);
        assert_eq!(
            right_edges(),
            (radio - 2.0, check - 2.0),
            "Blue labels end 2 px further left"
        );
        assert_eq!(hit(25, 12), up, "the radio label 15 px in");
        assert_eq!(hit(26, 42), case, "the check box label 16 px in");
        draw(&mut c, &text, &classic.paint(), &f, false, PX);
        assert_eq!(right_edges(), (radio, check), "Classic's edges again");
        assert_eq!(hit(25, 12), None, "nothing 15 px in");
        assert_eq!(hit(27, 12), up, "the radio label 17 px in");
        assert_eq!(hit(27, 42), None, "nothing 17 px into the check box");
        assert_eq!(hit(28, 42), case, "the check box label 18 px in");
    }

    #[test]
    fn set_check_moves_a_radios_own_dot_only() {
        let mut f = radio_form();
        f.set_check("up", true);
        f.set_check("down", false);
        assert_eq!(f.checked("up"), Some(true));
        assert_eq!(f.checked("down"), Some(false));
        assert_eq!(f.checked("all"), Some(true), "another group stays");
    }

    fn radio_form() -> Form {
        Form::new(vec![
            Item::radio("up", 10, 10, "&Up", "dir"),
            Item::radio("down", 10, 32, "&Down", "dir").checked(true),
            Item::radio("all", 10, 60, "&All", "range").checked(true),
            Item::new(
                "ok",
                Rect::new(10, 90, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
        ])
    }

    fn checked(f: &Form, id: &str) -> bool {
        matches!(
            f.item(id).map(|i| &i.control),
            Some(Control::Radio { checked: true, .. })
        )
    }

    #[test]
    fn positioned_constructors_take_the_row_and_button_heights() {
        assert_eq!(
            Item::label_at("l", 5, 6, 70, "A").rect,
            Rect::new(5, 6, 70, 13)
        );
        let v = Item::value_at("v", 5, 6, 70, "1");
        assert_eq!(v.rect, Rect::new(5, 6, 70, 13));
        assert!(matches!(
            v.control,
            Control::Label {
                align: Align::Right,
                ..
            }
        ));
        let p = Item::lines_at("p", 5, 6, 70, &["a", "b", "c"]);
        assert_eq!(p.rect, Rect::new(5, 6, 70, 39), "three 13 px rows");
        let b = Item::button_at("b", 5, 6, 75, "OK", true);
        assert_eq!(b.rect, Rect::new(5, 6, 75, 23));
        assert!(matches!(b.control, Control::Button { default: false, .. }));
    }

    #[test]
    fn picking_a_radio_clears_its_group_and_leaves_others_alone() {
        let mut f = radio_form();
        f.focus_id("up");
        assert_eq!(f.key(Key::Space), Event::Changed("up"));
        assert!(checked(&f, "up"));
        assert!(!checked(&f, "down"), "the group is exclusive");
        assert!(checked(&f, "all"), "the other group is untouched");
    }

    #[test]
    fn arrows_walk_within_a_group_and_stop_at_its_ends() {
        let mut f = radio_form();
        f.focus_id("up");
        // "down" is already this group's checked radio, so landing on it
        // mutates nothing: Redraw, not Changed.
        assert_eq!(f.key(Key::Down), Event::Redraw);
        assert_eq!(f.focus, f.index_of("down"));
        assert!(checked(&f, "down"));
        assert_eq!(f.key(Key::Down), Event::None);
        assert_eq!(f.focus, f.index_of("down"));
        assert!(checked(&f, "all"), "the next group is untouched");
        assert_eq!(f.key(Key::Up), Event::Changed("up"));
        assert_eq!(f.focus, f.index_of("up"));
        assert!(checked(&f, "up"));
        assert!(!checked(&f, "down"));
        assert_eq!(f.key(Key::Up), Event::None);
        assert_eq!(f.focus, f.index_of("up"));
        assert!(checked(&f, "up"));
    }

    #[test]
    fn tab_treats_a_group_as_one_stop_and_enters_at_the_checked_radio() {
        let mut f = radio_form();
        // "down" is checked, so it is the group's stop, not "up".
        assert_eq!(f.focus, f.index_of("down"));
        assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus, f.index_of("all"), "skips the rest of the group");
        assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus, f.index_of("ok"));
    }

    #[test]
    fn tab_without_the_wrap_stops_at_the_ends() {
        let mut f = radio_form();
        while f.tab_no_wrap(false) {}
        let last = f.focus;
        assert_eq!(last, f.index_of("ok"));
        assert!(!f.tab_no_wrap(false));
        assert_eq!(f.focus, last);
        while f.tab_no_wrap(true) {}
        assert_eq!(f.focus, f.index_of("down"), "the checked radio is the stop");
        assert!(!f.tab_no_wrap(true));
        assert!(f.focus_edge(true));
        assert_eq!(f.focus, last);
        assert!(f.focus_edge(false));
        assert_eq!(f.focus, f.index_of("down"));
    }

    #[test]
    fn a_radios_access_key_picks_it() {
        let mut f = radio_form();
        f.focus_id("ok");
        assert_eq!(f.key(Key::Char('u')), Event::Changed("up"));
        assert!(checked(&f, "up"));
        assert_eq!(f.focus, f.index_of("up"));
    }

    #[test]
    fn a_radios_label_is_part_of_its_hit_area() {
        let f = radio_form();
        assert_eq!(f.hit(12, 12, &tr(), PX).map(|h| h.item), f.index_of("up"));
        assert_eq!(f.hit(30, 12, &tr(), PX).map(|h| h.item), f.index_of("up"));
    }

    #[test]
    fn a_disabled_radio_is_skipped_by_tab_and_arrows() {
        let mut f = radio_form();
        if let Control::Radio { enabled, .. } = &mut f.items[2].control {
            *enabled = false;
        }
        f.focus_id("down");
        // "all" is disabled, so Down stays put rather than landing on it.
        assert_eq!(f.key(Key::Down), Event::None);
        assert_eq!(f.focus, f.index_of("down"));
    }

    #[test]
    fn clicking_a_radio_picks_it_and_clears_its_sibling() {
        let mut f = radio_form();
        let h = f.hit(12, 12, &tr(), PX);
        assert_eq!(h.map(|h| h.item), f.index_of("up"));
        assert_eq!(f.click(h), Event::Redraw, "the press only holds it");
        assert!(!checked(&f, "up"));
        assert_eq!(f.release(), Event::Changed("up"));
        assert!(checked(&f, "up"));
        assert!(!checked(&f, "down"), "the group is exclusive");
        assert!(checked(&f, "all"), "the other group is untouched");
        assert_eq!(f.focus, f.index_of("up"));
    }

    #[test]
    fn clicking_an_already_checked_radio_changes_nothing() {
        let mut f = radio_form();
        let h = f.hit(30, 38, &tr(), PX);
        assert_eq!(h.map(|h| h.item), f.index_of("down"));
        // "down" is already this group's pick, so nothing mutates.
        assert_eq!(f.click(h), Event::Redraw);
        assert_eq!(f.release(), Event::Redraw);
        assert!(checked(&f, "down"));
        assert!(!checked(&f, "up"));
    }

    #[test]
    fn clicking_a_disabled_radio_does_nothing() {
        let mut f = radio_form();
        if let Control::Radio { enabled, .. } = &mut f.items[2].control {
            *enabled = false;
        }
        f.focus_id("down");
        let before = f.focus;
        let h = f.hit(12, 62, &tr(), PX);
        assert_eq!(h.map(|h| h.item), f.index_of("all"), "it is still hit");
        assert_eq!(f.click(h), Event::None);
        assert!(!f.holding(), "a disabled radio is not held");
        assert_eq!(f.release(), Event::None);
        assert!(checked(&f, "all"));
        assert!(checked(&f, "down"));
        assert!(!checked(&f, "up"));
        assert_eq!(before, f.focus, "a disabled radio does not take focus");
    }

    /// A group with one enabled radio has nowhere to step: the walk is
    /// confined to the group and does not wrap, so both arrows run off
    /// an end and do nothing at all.
    #[test]
    fn an_arrow_on_a_lone_radio_does_nothing() {
        let mut f = Form::new(vec![
            Item::radio("solo", 10, 10, "&Solo", "only").checked(true),
            Item::new(
                "ok",
                Rect::new(10, 40, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
        ]);
        f.focus_id("solo");
        assert_eq!(f.key(Key::Down), Event::None);
        assert_eq!(f.key(Key::Up), Event::None);
        assert_eq!(f.focus, f.index_of("solo"));
        assert!(checked(&f, "solo"));
    }

    /// A save prompt's row: Yes is the default, a greyed button sits
    /// between it and No, and a combo stands before the row.
    fn button_row_form() -> Form {
        Form::new(vec![
            Item::new(
                "combo",
                Rect::new(10, 10, 150, 21),
                Control::Combo(Combo::new(&["Alpha", "Beta", "Gamma"], 0, 8)),
            ),
            Item::button("yes", Rect::new(10, 40, 75, 23), "&Yes", true, true),
            Item::button("greyed", Rect::new(90, 40, 75, 23), "&Later", false, false),
            Item::button("no", Rect::new(170, 40, 75, 23), "&No", true, false),
            Item::button("cancel", Rect::new(250, 40, 75, 23), "Cancel", true, false),
        ])
    }

    #[test]
    fn arrows_walk_the_push_buttons_skip_greyed_ones_and_stop_at_the_ends() {
        let mut f = button_row_form();
        f.focus_id("yes");
        assert_eq!(
            f.key(Key::Left),
            Event::None,
            "Left on the first button stops"
        );
        assert_eq!(f.key(Key::Up), Event::None, "Up on the first button stops");
        assert_eq!(f.focus, f.index_of("yes"), "the focus stays on Yes");
        assert_eq!(f.key(Key::Right), Event::Redraw);
        assert_eq!(
            f.focus,
            f.index_of("no"),
            "Right skips the greyed button to No"
        );
        assert_eq!(f.key(Key::Down), Event::Redraw);
        assert_eq!(f.focus, f.index_of("cancel"), "Down moves on to Cancel");
        assert_eq!(
            f.key(Key::Right),
            Event::None,
            "Right on the last button stops"
        );
        assert_eq!(
            f.key(Key::Down),
            Event::None,
            "Down on the last button stops"
        );
        assert_eq!(f.focus, f.index_of("cancel"), "the focus stays on Cancel");
        assert_eq!(f.key(Key::Up), Event::Redraw);
        assert_eq!(f.focus, f.index_of("no"), "Up moves back to No");
        assert_eq!(f.key(Key::Left), Event::Redraw);
        assert_eq!(
            f.focus,
            f.index_of("yes"),
            "Left moves back to Yes, not the combo"
        );
        let right = Key::Edit(EditKey::Right {
            select: false,
            word: false,
        });
        assert_eq!(f.key(right), Event::Redraw);
        assert_eq!(f.focus, f.index_of("no"), "an editing Right moves to No");
        assert_eq!(f.key(Key::Enter), Event::Pressed("no"), "Enter presses No");
    }

    #[test]
    fn an_arrow_on_a_combo_still_steps_the_combo() {
        let mut f = button_row_form();
        f.focus_id("combo");
        assert_eq!(f.key(Key::Down), Event::Changed("combo"));
        assert_eq!(f.focus, f.index_of("combo"), "the focus stays on the combo");
        assert_eq!(f.key(Key::Right), Event::Changed("combo"));
        assert_eq!(f.focus, f.index_of("combo"), "the focus stays on the combo");
    }

    #[test]
    fn an_arrow_to_a_button_takes_the_default_ring_from_the_default() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut f = form();
        f.focus_id("ok");
        assert_eq!(f.key(Key::Right), Event::Redraw);
        assert_eq!(
            f.focus,
            f.index_of("cancel"),
            "Right moves from OK to Cancel"
        );
        let mut c = Canvas::new(200, 150);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
        assert_eq!(
            c.get(90, 115),
            cs.button_text.argb(),
            "Cancel's left edge is the black ring"
        );
        assert_eq!(
            c.get(10, 115),
            cs.button_hilight.argb(),
            "OK's left edge is the plain white edge"
        );
    }

    /// Space works the focused control even when the host sends editing
    /// keys: it arrives as a one-character insert, and the general arm
    /// in `flat_key` would degrade it to `Key::Char(' ')`, which the
    /// access-key search then swallows.
    #[test]
    fn space_as_an_editing_key_still_works_the_focused_control() {
        let mut f = radio_form();
        f.focus_id("up");
        assert!(!checked(&f, "up"));
        assert_eq!(
            f.key(Key::Edit(EditKey::Insert(" ".into()))),
            Event::Changed("up")
        );
        assert!(checked(&f, "up"), "space picked the focused radio");

        // A space in a focused edit field is still a space: it never
        // reaches flat_key at all.
        let mut e = edit_form();
        assert_eq!(
            e.key(Key::Edit(EditKey::Insert(" ".into()))),
            Event::Changed("find")
        );
        assert!(matches!(&e.items[1].control, Control::Edit { edit, .. } if edit.text == " "));
    }

    /// The font size the dialogs are drawn at.
    const PX: u32 = 11;
    /// A renderer for the tests that hit-test or draw text.
    fn tr() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn edit_form() -> Form {
        Form::new(vec![
            Item::new(
                "lbl",
                Rect::new(10, 10, 60, 13),
                Control::Label {
                    text: "Fi&nd what:".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
            Item::new(
                "find",
                Rect::new(80, 10, 100, 20),
                Control::Edit {
                    edit: TextEdit::new(String::new(), false),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
            Item::new(
                "ok",
                Rect::new(10, 40, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
        ])
    }

    #[test]
    fn typing_reaches_a_focused_edit_instead_of_an_access_key() {
        let mut f = edit_form();
        assert_eq!(f.focus, Some(1), "the edit is the first focusable item");
        assert_eq!(f.key(Key::Char('o')), Event::Changed("find"));
        assert_eq!(f.key(Key::Char('k')), Event::Changed("find"));
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "ok"));
        assert_eq!(f.focus, Some(1));
    }

    #[test]
    fn a_paste_into_a_field_takes_its_first_line_only() {
        let mut f = edit_form();
        assert_eq!(f.paste("pw\nsecond".into()), Event::Changed("find"));
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "pw"));
        f.focus_id("ok");
        assert_eq!(f.paste("x".into()), Event::None, "a button takes no paste");
    }

    /// Alt+letter is an access key even while a field has the focus
    /// (Alt+B on Run presses Browse...), so it never types into it.
    #[test]
    fn an_access_key_skips_a_focused_edit() {
        let mut f = edit_form();
        if let Control::Button { label, .. } = &mut f.items[2].control {
            *label = "&OK".into();
        }
        f.key(Key::Char('x'));
        assert_eq!(f.key(Key::Access('O')), Event::Pressed("ok"));
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "x"));
        assert_eq!(f.key(Key::Access('q')), Event::None);
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "x"));
    }

    #[test]
    fn edit_keys_reach_the_field_and_dialog_keys_still_do_not() {
        let mut f = edit_form();
        f.key(Key::Char('a'));
        f.key(Key::Char('b'));
        assert_eq!(
            f.key(Key::Edit(EditKey::Backspace { word: false })),
            Event::Changed("find")
        );
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "a"));
        assert_eq!(
            f.key(Key::Edit(EditKey::Home { select: true })),
            Event::Changed("find")
        );
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.has_selection()));
        f.key(Key::Edit(EditKey::End { select: false }));
        assert_eq!(f.key(Key::Space), Event::Changed("find"));
        assert!(matches!(&f.items[1].control, Control::Edit { edit, .. } if edit.text == "a "));
        assert_eq!(f.key(Key::Enter), Event::Default);
        assert_eq!(f.key(Key::Escape), Event::Cancel);
        f.focus_id("ok");
        assert_eq!(f.key(Key::Char('n')), Event::Redraw);
        assert_eq!(f.focus, Some(1));
    }

    /// A tree's thumb arms a drag, and the sweep scrolls the tree by
    /// where the thumb lands, the way a list box's does.
    #[test]
    fn a_trees_thumb_drags() {
        use crate::controls::treeview::{self, Node, TreeView};
        let t = tr();
        let mut root = Node::new("root", true);
        root.children = (0..30)
            .map(|i| Node::new(&format!("n{i}"), false))
            .collect();
        root.expanded = true;
        let tv = TreeView::new(vec![root], treeview::Style::Classic);
        let rect = Rect::new(0, 0, 150, 100);
        let mut f = Form::new(vec![Item::new("tree", rect, Control::Tree(tv))]);
        let (sb, rows, grab_y) = {
            let Control::Tree(tv) = &f.items[0].control else {
                panic!()
            };
            let lay = treeview::layout_without_text(tv, rect, &ControlMetrics::CLASSIC);
            let sb = lay.scrollbar.expect("31 rows overflow 100 px");
            let p = crate::classic::widgets::scrollbar_parts(
                sb,
                1,
                0,
                lay.rows,
                31,
                false,
                lay.thumb_min,
            );
            (sb, lay.rows, p.thumb.expect("a thumb").y + 2)
        };
        let h = f.hit(sb.x + 8, grab_y, &t, PX).expect("the thumb");
        assert_eq!(
            h.part,
            Part::Tree(treeview::Hit::VBar(crate::classic::widgets::BarHit::Thumb(
                2
            )))
        );
        assert_eq!(f.click(Some(h)), Event::Redraw);
        let held = match f.drag {
            Some(Drag::Bar { item: 0, thumb }) => thumb.grab(Axis::Vertical),
            _ => None,
        };
        assert_eq!(held, Some(2), "the tree's thumb, grabbed 2 px in");
        assert_eq!(f.motion(sb.x + 8, sb.bottom(), None), Event::Redraw);
        let Control::Tree(tv) = &f.items[0].control else {
            panic!()
        };
        assert_eq!(tv.top, 31 - rows, "dragged to the end: the last page");
        f.release();
        assert_eq!(f.drag, None);
    }

    #[test]
    fn clicking_an_edit_places_the_caret_and_dragging_extends_it() {
        let t = tr();
        let mut f = edit_form();
        let Control::Edit { edit, .. } = &mut f.items[1].control else {
            panic!("the fixture's second item is the edit")
        };
        *edit = TextEdit::new("Hello world", false);
        // The pen starts FIELD_PAD inside the frame, so 30 px along the
        // text area lands somewhere in the middle of the value.
        let x = 80 + crate::classic::widgets::FIELD_PAD + 30;
        let h = f.hit(x, 15, &t, PX).expect("the edit is hit");
        assert_eq!(h.item, 1);
        let Part::Edit(at) = h.part else {
            panic!("{:?} is not an edit hit", h.part)
        };
        assert_eq!(at, crate::textedit::index_at(&t, "Hello world", 30, PX));
        assert!(at > 0 && at < "Hello world".len(), "a boundary inside it");
        assert!(f.hit(300, 300, &t, PX).is_none());

        assert_eq!(f.click(Some(h)), Event::Redraw);
        let Control::Edit { edit, .. } = &f.items[1].control else {
            panic!()
        };
        assert_eq!(
            edit.caret(),
            at,
            "the click put the caret under the pointer"
        );
        assert!(!edit.has_selection(), "and dropped the selection");
        assert_eq!(f.drag, Some(Drag::Edit(1)), "the press armed a sweep");

        let back = 80 + crate::classic::widgets::FIELD_PAD + 10;
        let over = f.hit(back, 15, &t, PX);
        let Some(Part::Edit(to)) = over.map(|h| h.part) else {
            panic!("the drag is still over the edit")
        };
        assert!(to < at);
        assert_eq!(f.motion(back, 15, over), Event::Redraw);
        let Control::Edit { edit, .. } = &f.items[1].control else {
            panic!()
        };
        assert_eq!(edit.selection(), to..at, "the drag swept back to `to`");
    }

    fn field(f: &Form) -> (&TextEdit, usize) {
        match &f.items[1].control {
            Control::Edit { edit, first, .. } => (edit, first.get()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_drag_over_a_field_drops_its_text_at_the_drop_caret() {
        let t = tr();
        let mut f = edit_form();
        f.paste("hello world".into());
        f.set_focus(None);
        let inner_x = 80 + crate::classic::widgets::FIELD_PAD;
        let x = inner_x + t.measure("hello", false, PX);
        assert!(!f.drag_over(30, 15, &t, PX), "a label takes no drop");
        assert!(f.drag_over(x, 15, &t, PX));
        assert_eq!(
            f.drop_text(&crate::textedit::DropLine::Text(" big".into()), &t, PX),
            Event::Changed("find")
        );
        assert_eq!(f.focus(), Some(1), "the field takes the focus");
        assert_eq!(field(&f).0.text, "hello big world");
        assert_eq!(field(&f).0.selected_text(), " big");
        assert_eq!(
            f.drop_text(&crate::textedit::DropLine::Text("x".into()), &t, PX),
            Event::None,
            "the drop ended the drag"
        );
    }

    #[test]
    fn a_masked_or_greyed_field_takes_no_drop() {
        let t = tr();
        let mut f = edit_form();
        f.items[1].control = Control::Edit {
            edit: TextEdit::secret(),
            first: Cell::new(0),
            enabled: true,
        };
        assert!(!f.drag_over(120, 15, &t, PX));
        let mut f = edit_form();
        f.set_enabled("find", false);
        assert!(!f.drag_over(120, 15, &t, PX));
        assert!(!f.drag_leave());
    }

    #[test]
    fn a_drag_resting_at_a_fields_edge_scrolls_it_without_moving_the_caret() {
        let t = tr();
        let theme = Theme::default();
        let mut f = edit_form();
        f.paste("abcdefghijklmnopqrstuvwxyz0123456789".into());
        let mut c = Canvas::new(200, 80);
        draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
        let end = field(&f).1;
        assert!(end > 0, "the caret at the end scrolled the value");
        assert!(f.drag_over(130, 15, &t, PX));
        assert_eq!(f.drag_scroll(&t, PX), None, "the middle rests still");
        assert!(f.drag_over(82, 15, &t, PX));
        assert_eq!(
            f.drag_scroll(&t, PX),
            Some(end - 1),
            "nothing moves before the tick"
        );
        for k in 1..=3 {
            assert!(f.drag_tick(&t, PX));
            draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
            assert_eq!(field(&f).1, end - k, "the draw keeps the scrolled view");
        }
        let caret = field(&f).0.caret();
        assert!(f.drag_over(178, 15, &t, PX));
        assert!(f.drag_tick(&t, PX));
        assert_eq!(field(&f).1, end - 2);
        assert!(f.drag_leave());
        assert!(!f.drag_tick(&t, PX), "no drag, no step");
        draw(&mut c, &t, &classic_paint(&theme.colors, 1), &f, true, PX);
        assert_eq!(
            field(&f).1,
            end,
            "without a drag the caret pins the view again"
        );
        assert_eq!(field(&f).0.caret(), caret);
    }

    #[test]
    fn the_drop_caret_is_a_bar_where_the_text_would_land() {
        let t = tr();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut f = edit_form();
        f.paste("hello world".into());
        f.set_focus(None);
        let inner_x = 80 + crate::classic::widgets::FIELD_PAD;
        let x = inner_x + t.measure("hello", false, PX);
        let shot = |f: &Form| {
            let mut c = Canvas::new(200, 40);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &t, &classic_paint(cs, 1), f, true, PX);
            c
        };
        let before = shot(&f);
        f.drag_over(x, 15, &t, PX);
        let after = shot(&f);
        let changed: Vec<(i32, i32)> = (0..40)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
            .collect();
        assert!(!changed.is_empty());
        assert!(
            changed.iter().all(|&(cx, _)| cx == x),
            "one column at the drop caret: {changed:?}"
        );
        assert!(
            changed
                .iter()
                .all(|&(cx, cy)| after.get(cx, cy) == cs.button_text.argb())
        );
        assert!(changed.len() >= 8, "a line tall");
    }

    /// `first` is the field's scroll offset and `draw` owns it: typing
    /// past the right edge has to bring the caret back into view on the
    /// next frame, with no host bookkeeping in between.
    #[test]
    fn typing_past_the_right_edge_scrolls_the_value_into_view() {
        let t = tr();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut f = edit_form();
        for ch in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
            f.key(Key::Char(ch));
        }
        let first = |f: &Form| match &f.items[1].control {
            Control::Edit { first, .. } => first.get(),
            other => panic!("{other:?}"),
        };
        assert_eq!(first(&f), 0, "nothing has drawn yet");

        let mut c = Canvas::new(200, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &t, &classic_paint(cs, 1), &f, true, PX);
        let from = first(&f);
        assert!(from > 0, "the value scrolled to follow the caret");
        // What shows fits the text area, caret included.
        let Control::Edit { edit, .. } = &f.items[1].control else {
            panic!()
        };
        let inner_w = f.items[1].rect.width - 2 * crate::classic::widgets::FIELD_PAD;
        assert!(t.measure(&edit.text[from..edit.caret()], false, PX) <= inner_w);

        f.key(Key::Edit(EditKey::Home { select: false }));
        draw(&mut c, &t, &classic_paint(cs, 1), &f, true, PX);
        assert_eq!(first(&f), 0, "the caret at the start shows the start");
    }

    /// A focused password field shows no preedit and hands an input
    /// method no text, only that it is secret.
    #[test]
    fn a_masked_field_reports_a_secret_spot_and_shows_no_preedit() {
        let t = tr();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut f = edit_form();
        let Control::Edit { edit, .. } = &mut f.items[1].control else {
            panic!("the fixture's second item is the edit")
        };
        *edit = TextEdit::secret();
        edit.paste("abc", &[]);
        assert!(f.focused_edit().is_some(), "the edit has the focus");
        let shot = |preedit| {
            let mut c = Canvas::new(200, 80);
            c.set_preedit(preedit);
            draw(&mut c, &t, &classic_paint(cs, 1), &f, false, PX);
            c
        };
        let mut with = shot(crate::textedit::Preedit::new("nihao".into(), 5, 5));
        assert_eq!(with.pixels, shot(None).pixels);
        let spot = with.take_spot().expect("a focused field reports");
        assert!(spot.secret);
        assert_eq!(spot.surrounding, None);
    }

    /// A masked field draws bullets instead of its value: two values of
    /// the same length draw the same pixels, and masking one of them
    /// changes what it draws.
    #[test]
    fn masked_edit_draws_bullets() {
        if !crate::text::test_tahoma() {
            return;
        }
        let t = tr();
        let theme = Theme::default();
        let cs = &theme.colors;
        let shot = |value: &str, masked: bool| {
            let mut f = edit_form();
            let Control::Edit { edit, .. } = &mut f.items[1].control else {
                panic!("the fixture's second item is the edit")
            };
            *edit = if masked {
                TextEdit::secret()
            } else {
                TextEdit::new("", false)
            };
            edit.paste(value, &[]);
            // The caret blink would differ with the bullets' width.
            f.caret = false;
            let mut c = Canvas::new(200, 80);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &t, &classic_paint(cs, 1), &f, false, PX);
            c
        };
        assert_eq!(
            shot("abc", true).pixels,
            shot("xyz", true).pixels,
            "the value does not show through the mask"
        );
        assert_ne!(
            shot("abc", true).pixels,
            shot("abc", false).pixels,
            "and the bullets are not the value"
        );
        // Something is drawn: an empty field would pass both of those.
        let bullets = shot("abc", true);
        let field = edit_form().items[1].rect;
        let ink = (field.x + 4..field.right() - 4)
            .flat_map(|x| (field.y + 4..field.bottom() - 4).map(move |y| (x, y)))
            .filter(|&(x, y)| bullets.get(x, y) == cs.button_text.argb())
            .count();
        assert!(ink > 20, "three bullets' worth of ink, not {ink} px");
    }

    #[test]
    fn a_disabled_edit_is_skipped_by_tab() {
        let mut f = edit_form();
        if let Control::Edit { enabled, .. } = &mut f.items[1].control {
            *enabled = false;
        }
        let mut f = Form::new(f.items);
        assert_eq!(f.focus, Some(2), "focus starts on OK");
        assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus, Some(2), "OK is the only stop");
    }

    /// `hit` must not report `Part::Edit` for a disabled field, nor measure text for it;
    /// Replace greys both its fields until it has a query.
    #[test]
    fn a_disabled_edit_hits_as_body() {
        let t = tr();
        let mut f = edit_form();
        let r = f.items[1].rect;
        let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
        let live = f.hit(x, y, &t, PX).expect("the field is under the pointer");
        assert!(
            matches!(live.part, Part::Edit(_)),
            "an enabled field still reports the byte under the pointer"
        );
        if let Control::Edit { enabled, .. } = &mut f.items[1].control {
            *enabled = false;
        }
        let dead = f
            .hit(x, y, &t, PX)
            .expect("the rect is still under the pointer");
        assert_eq!(dead.item, 1, "the same item is still hit");
        assert_eq!(dead.part, Part::Body);
    }

    /// A check box's label rect starts at the scaled label x and is the
    /// text's device width plus 2 px per scale step, so at scale 2 the
    /// focus ring ends 5 px past the 22 px text, not twice its width out.
    #[test]
    fn a_check_boxs_focus_ring_at_scale_two_spans_the_scaled_label() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let label = "Display delete confirmation";
        let mut f = Form::new(vec![Item::check("c", 10, 10, label)]);
        f.focus = Some(0);
        // The ring's rightmost inked column at scale `s`.
        let right = |s: i32| {
            let mut c = Canvas::new(600, 80);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(
                &mut c,
                &text,
                &classic_paint(cs, s),
                &f,
                true,
                11 * s as u32,
            );
            (0..600)
                .rev()
                .find(|&x| (0..80).any(|y| c.get(x, y) != cs.button_face.argb()))
                .expect("the check box draws")
        };
        // The ring runs from 1 px left of the label rect to 1 px past it;
        // the rect starts at the scaled label x and is the text plus 2 px.
        let x = 10 + classic_paint(cs, 1).look.controls().check_label;
        for s in [1, 2] {
            let want = s * x + text.measure(label, false, 11 * s as u32) + 3 * s - 1;
            assert_eq!(right(s), want, "the ring ends at x {want} at scale {s}");
        }
    }

    #[test]
    fn a_disabled_check_greys_its_box_not_just_its_label() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let f = Form::new(vec![Item::new(
            "c",
            Rect::new(10, 10, 13, 13),
            Control::Check {
                label: "Do not move files to the Recycle Bin".into(),
                checked: false,
                enabled: false,
                mixed: None,
            },
        )]);
        let mut c = Canvas::new(240, 40);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
        // Face inside the sunken edge, where an enabled box is window.
        assert_eq!(c.get(13, 13), cs.button_face.argb());
    }

    #[test]
    fn set_track_clamps_to_the_last_tick() {
        let mut f = Form::new(vec![Item::new(
            "t",
            Rect::new(10, 10, 71, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(5, 10)),
        )]);
        f.set_track("t", 99);
        let Some(Control::TrackBar(t)) = f.item("t").map(|i| &i.control) else {
            panic!("the only item is the trackbar")
        };
        assert_eq!(t.pos, 9, "the last of 10 ticks");
    }

    #[test]
    fn a_disabled_trackbar_is_not_a_tab_stop() {
        let mut tb = TrackBar::new(5, 10);
        tb.enabled = false;
        let f = Form::new(vec![
            Item::new(
                "t",
                Rect::new(10, 10, 71, trackbar::HEIGHT),
                Control::TrackBar(tb),
            ),
            Item::new(
                "ok",
                Rect::new(10, 50, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
        ]);
        assert_eq!(f.focus, Some(1), "the trackbar is skipped");
    }

    #[test]
    fn an_editing_key_outside_a_field_drives_the_control_it_stood_for() {
        // Arrows reach the form as EditKeys, so a focused combo must
        // still step: the form degrades what is not going into a field.
        let (mut flat, mut rich) = (form(), form());
        flat.focus_id("combo");
        rich.focus_id("combo");
        assert_eq!(flat.key(Key::Right), Event::Changed("combo"));
        assert_eq!(
            rich.key(Key::Edit(EditKey::Right {
                select: false,
                word: false
            })),
            Event::Changed("combo")
        );
        assert_eq!(flat.key(Key::Left), Event::Changed("combo"));
        assert_eq!(
            rich.key(Key::Edit(EditKey::Left {
                select: false,
                word: false
            })),
            Event::Changed("combo")
        );
        assert_eq!(flat, rich, "the degrade left the form in the same state");
        // An editing key with no dialog meaning stays dropped.
        assert_eq!(
            rich.key(Key::Edit(EditKey::Delete { word: false })),
            Event::None
        );
    }

    #[test]
    fn an_editing_key_reaches_an_open_drop_list() {
        // The popup guard returns before the focused control is
        // consulted, so the degrade has to happen above it.
        let (mut flat, mut rich) = (form(), form());
        flat.focus_id("combo");
        rich.focus_id("combo");
        flat.key(Key::Drop);
        rich.key(Key::Drop);
        assert_eq!(flat.key(Key::End), Event::Redraw);
        assert_eq!(
            rich.key(Key::Edit(EditKey::End { select: false })),
            Event::Redraw
        );
        assert!(
            matches!(&rich.items[1].control, Control::Combo(c) if c.open == Some(2)),
            "End walked the open list to its last row"
        );
        assert_eq!(flat, rich);
    }

    fn form() -> Form {
        Form::new(vec![
            Item::new(
                "lbl",
                Rect::new(10, 10, 60, 13),
                Control::Label {
                    text: "&Theme:".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
            Item::new(
                "combo",
                Rect::new(10, 30, 150, 21),
                Control::Combo(Combo::new(&["Alpha", "Beta", "Gamma"], 0, 8)),
            ),
            Item::new(
                "check",
                Rect::new(10, 60, 13, 13),
                Control::Check {
                    label: "&Password protect".into(),
                    checked: false,
                    enabled: true,
                    mixed: None,
                },
            ),
            Item::new(
                "spin",
                Rect::new(10, 80, 50, 20),
                Control::UpDown(UpDown::new(10, 1, 99)),
            ),
            Item::new(
                "ok",
                Rect::new(10, 110, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
            Item::new(
                "cancel",
                Rect::new(90, 110, 75, 23),
                Control::Button {
                    label: "Cancel".into(),
                    enabled: true,
                    default: false,
                },
            ),
        ])
    }

    #[test]
    fn tab_walks_the_focusable_items_and_keys_reach_the_focused_one() {
        let mut f = form();
        assert_eq!(f.focus, Some(1), "the label is skipped");
        assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus, Some(2));
        assert_eq!(f.key(Key::Space), Event::Changed("check"));
        assert!(matches!(
            f.items[2].control,
            Control::Check { checked: true, .. }
        ));
        assert_eq!(f.key(Key::Tab { back: true }), Event::Redraw);
        assert_eq!(f.key(Key::Down), Event::Changed("combo"));
        assert_eq!(f.key(Key::Drop), Event::Redraw);
        assert_eq!(f.key(Key::Down), Event::Redraw, "moves the open highlight");
        assert_eq!(f.key(Key::Enter), Event::Changed("combo"));
        assert!(
            matches!(&f.items[1].control, Control::Combo(c) if c.selected == 2 && c.open.is_none())
        );
        assert_eq!(f.key(Key::Enter), Event::Default);
        assert_eq!(f.key(Key::Escape), Event::Cancel);
        f.focus = Some(4);
        assert_eq!(f.key(Key::Char('p')), Event::Changed("check"));
        assert_eq!(f.key(Key::Char('T')), Event::Redraw);
        assert_eq!(f.focus, Some(1));
        f.focus_id("spin");
        assert_eq!(f.key(Key::Char('4')), Event::Changed("spin"));
        assert!(matches!(&f.items[3].control, Control::UpDown(u) if u.value() == 4));
        assert_eq!(f.key(Key::Up), Event::Changed("spin"));
        f.focus_id("cancel");
        assert_eq!(f.key(Key::Enter), Event::Pressed("cancel"));
    }

    fn spin(f: &Form) -> &UpDown {
        match &f.item("spin").unwrap().control {
            Control::UpDown(u) => u,
            _ => unreachable!("spin is an up-down"),
        }
    }

    #[test]
    fn a_spinner_left_and_tabbed_back_to_replaces_its_number() {
        let mut f = form();
        f.focus_id("spin");
        f.key(Key::Char('3'));
        f.key(Key::Char('0'));
        assert_eq!(spin(&f).value(), 30);
        f.key(Key::Tab { back: false });
        assert!(!spin(&f).editing(), "leaving drops the typed digits");
        f.key(Key::Tab { back: true });
        assert_eq!(f.key(Key::Char('1')), Event::Changed("spin"));
        assert_eq!((spin(&f).value(), spin(&f).shown().as_str()), (1, "1"));
        // Digits past the range clamp, and leaving shows the clamped value.
        f.key(Key::Char('2'));
        f.key(Key::Char('3'));
        assert_eq!((spin(&f).value(), spin(&f).shown().as_str()), (99, "123"));
        f.set_focus(None);
        assert_eq!(spin(&f).shown(), "99");
    }

    #[test]
    fn tabbing_into_a_spinner_selects_its_number() {
        let mut f = form();
        f.focus_id("check");
        f.key(Key::Tab { back: false });
        assert_eq!(f.focus(), f.index_of("spin"));
        assert!(spin(&f).editing());
        // A host's editing keys: the digit replaces the selection, Left
        // steps the caret back over it, and the next digit goes in there.
        let ins = |s: &str| Key::Edit(EditKey::Insert(s.into()));
        assert_eq!(f.key(ins("5")), Event::Changed("spin"));
        assert_eq!(f.key(ins("7")), Event::Changed("spin"));
        let left = EditKey::Left {
            select: false,
            word: false,
        };
        assert_eq!(f.key(Key::Edit(left)), Event::Redraw);
        assert_eq!(f.key(ins("1")), Event::Changed("spin"));
        assert_eq!(spin(&f).shown(), "517");
        assert_eq!(spin(&f).value(), 99);
        // A letter is no digit: it is the check box's access key.
        assert_eq!(f.key(ins("p")), Event::Changed("check"));
    }

    #[test]
    fn clicking_a_spinner_puts_the_caret_under_the_pointer() {
        let mut f = form();
        // The field's white part runs from x 12; "10" sits right-aligned
        // against x 41, so x 13 is before its first digit.
        let h = f.hit(13, 90, &tr(), PX).unwrap();
        assert_eq!(h.part, Part::UpDown(updown::Hit::Edit(0)));
        assert_eq!(f.click(Some(h)), Event::Redraw);
        assert_eq!(f.focus(), f.index_of("spin"));
        f.key(Key::Char('2'));
        assert_eq!(spin(&f).shown(), "210", "typed in front, not over it");
        // Its up arrow steps the value and selects it for the next digit.
        let up = f.hit(50, 83, &tr(), PX).unwrap();
        assert_eq!(f.click(Some(up)), Event::Redraw, "already at the top");
        f.focus_id("ok");
        f.click(Some(up));
        assert_eq!(spin(&f).shown(), "99");
        f.key(Key::Char('4'));
        assert_eq!(spin(&f).value(), 4);
    }

    #[test]
    fn clicks_land_on_parts_and_popups_cover_the_rest() {
        let mut f = form();
        let h = f.hit(20, 65, &tr(), PX).unwrap();
        assert_eq!((h.item, h.part), (2, Part::Body));
        assert_eq!(f.click(Some(h)), Event::Redraw);
        assert_eq!(f.release(), Event::Changed("check"));
        let h = f.hit(60, 65, &tr(), PX).unwrap();
        assert_eq!(h.item, 2);
        let drop = f.hit(150, 40, &tr(), PX).unwrap();
        assert_eq!(drop.part, Part::Combo(combo::Hit::Drop));
        assert_eq!(f.click(Some(drop)), Event::Redraw);
        let row = f.hit(20, 51 + 1 + 13 + 5, &tr(), PX).unwrap();
        assert_eq!((row.item, row.part), (1, Part::Combo(combo::Hit::Row(1))));
        assert_eq!(f.click(Some(row)), Event::Changed("combo"));
        f.key(Key::Drop);
        assert_eq!(f.click(None), Event::Redraw);
        let ok = f.hit(20, 120, &tr(), PX).unwrap();
        assert_eq!(f.click(Some(ok)), Event::Redraw);
        assert_eq!(f.held_item(), Some(ok.item));
        assert_eq!(f.lit(ok.item, Light::Body), Interaction::Pressed);
        assert_eq!(f.release(), Event::Pressed("ok"));
        assert!(!f.holding());
        assert_eq!(f.click(Some(ok)), Event::Redraw);
        assert_eq!(f.motion(300, 300, None), Event::Redraw);
        assert_eq!(f.held_item(), Some(ok.item));
        assert_eq!(f.lit(ok.item, Light::Body), Interaction::Idle);
        assert_eq!(f.release(), Event::Redraw);
        assert!(!f.holding());
    }

    #[test]
    fn a_rule_is_a_solid_shadow_rect_nothing_can_hit() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let f = Form::new(vec![Item::new(
            "rule",
            Rect::new(20, 10, 2, 16),
            Control::Rule,
        )]);
        let mut c = Canvas::new(60, 40);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &f, false, 11);
        assert_eq!(c.get(20, 10), cs.button_shadow.argb());
        assert_eq!(c.get(21, 25), cs.button_shadow.argb());
        assert_eq!(c.get(22, 25), cs.button_face.argb(), "2 px wide");
        assert_eq!(c.get(20, 26), cs.button_face.argb(), "16 px tall");
        assert_eq!(f.hit(21, 20, &text, 11), None, "a rule is not a target");
    }

    #[test]
    fn draws_focus_and_the_default_ring() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut f = form();
        let mut c = Canvas::new(200, 150);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &f, true, 11);
        // OK wears the default ring while the combo has the focus.
        assert_eq!(c.get(10, 115), cs.button_text.argb());
        assert_eq!(c.get(90, 115), cs.button_hilight.argb());
        f.focus_id("cancel");
        let mut c2 = Canvas::new(200, 150);
        draw(&mut c2, &text, &classic_paint(cs, 1), &f, true, 11);
        assert_eq!(c2.get(90, 115), cs.button_text.argb());
        assert_eq!(c2.get(10, 115), cs.button_hilight.argb());
        // The dotted rectangle stays hidden until the keyboard is used,
        // so the same focus without cues keeps the ring and drops the
        // dots.
        let ring = Rect::new(
            90,
            110,
            crate::classic::widgets::BUTTON_W,
            crate::classic::widgets::BUTTON_H,
        )
        .inset(4);
        let dotted =
            |c: &Canvas| (ring.x..ring.right()).any(|x| c.get(x, ring.y) == cs.button_text.argb());
        assert!(dotted(&c2), "with cues Cancel is dotted");
        let mut c3 = Canvas::new(200, 150);
        c3.fill_rect(c3.bounds(), cs.button_face);
        draw(&mut c3, &text, &classic_paint(cs, 1), &f, false, 11);
        assert_eq!(c3.get(90, 115), cs.button_text.argb(), "the ring stays");
        assert!(!dotted(&c3), "the dots wait for a key");
    }

    #[test]
    fn an_edit_fills_by_its_state_and_blinks_only_when_focused() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut f = edit_form();
        // The field is (80, 10, 100, 20) and the pen starts FIELD_PAD
        // inside it, so an empty field's caret is the column at x 83 and
        // (120, 20) is clear interior. The text is empty, so no glyph can
        // land on either sample.
        let (caret_x, caret_y) = (83, 15);
        let (fill_x, fill_y) = (120, 20);
        let shot = |f: &Form| {
            let mut c = Canvas::new(200, 80);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &text, &classic_paint(cs, 1), f, true, 11);
            c
        };
        assert_eq!(f.focus, Some(1));
        assert!(f.caret, "a new form starts with the caret shown");
        let c = shot(&f);
        assert_eq!(c.get(fill_x, fill_y), cs.window.argb());
        assert_eq!(c.get(caret_x, caret_y), cs.button_text.argb());
        f.caret = false;
        let c = shot(&f);
        assert_eq!(c.get(caret_x, caret_y), cs.window.argb(), "no caret");
        assert_eq!(c.get(fill_x, fill_y), cs.window.argb());
        f.caret = true;
        f.focus_id("ok");
        let c = shot(&f);
        assert_eq!(
            c.get(caret_x, caret_y),
            cs.window.argb(),
            "an unfocused field does not blink"
        );
        if let Control::Edit { enabled, .. } = &mut f.items[1].control {
            *enabled = false;
        }
        f.focus = Some(1);
        let c = shot(&f);
        assert_eq!(c.get(fill_x, fill_y), cs.button_face.argb());
        assert_eq!(
            c.get(caret_x, caret_y),
            cs.button_face.argb(),
            "a disabled field does not blink"
        );
    }

    /// An Open With list: a letter jumps to the next row starting with
    /// it (the focus stays on the list), Enter is the default button and
    /// a double click on a row is Enter.
    #[test]
    fn a_list_box_takes_type_ahead_and_a_double_click_is_the_default() {
        let names = ["Write", "Notepad", "Picture Editor"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut f = Form::new(vec![
            Item::new(
                "apps",
                Rect::new(10, 10, 100, 60),
                Control::ListBox {
                    list: ListBox::new(names, Some(0)),
                    icons: vec![None; 3],
                },
            ),
            Item::new(
                "ok",
                Rect::new(10, 80, 75, 23),
                Control::Button {
                    label: "OK".into(),
                    enabled: true,
                    default: true,
                },
            ),
        ]);
        f.focus_id("apps");
        assert_eq!(f.key(Key::Char('p')), Event::Changed("apps"));
        let list = |f: &Form| match &f.items[0].control {
            Control::ListBox { list, .. } => list.selected,
            _ => unreachable!(),
        };
        assert_eq!(list(&f), Some(2));
        assert_eq!(f.key(Key::Char('z')), Event::None);
        assert_eq!(f.key(Key::Enter), Event::Default);
        let row = f.hit(30, 12 + 17, &tr(), PX).unwrap();
        assert_eq!(row.part, Part::List(listbox::Hit::Row(1)));
        assert_eq!(f.double_click(Some(row)), Event::Default);
        assert_eq!(list(&f), Some(1));
    }

    /// A list with check boxes: a click on a row's box selects the row
    /// and flips it, a click on its label only selects, and Space flips
    /// the selected row.
    #[test]
    fn a_checked_list_box_flips_on_its_box_and_on_space() {
        let names = ["Volume Control", "Firefox"].map(String::from).to_vec();
        let mut list = ListBox::new(names, Some(0));
        list.checks = vec![true, true];
        let mut f = Form::new(vec![Item::new(
            "shown",
            Rect::new(10, 10, 150, 60),
            Control::ListBox {
                list,
                icons: vec![None; 2],
            },
        )]);
        f.focus_id("shown");
        let state = |f: &Form| match &f.items[0].control {
            Control::ListBox { list, .. } => (list.selected, list.checks.clone()),
            _ => unreachable!(),
        };
        let box1 = f.hit(12 + 3 + 5, 12 + 17 + 2 + 5, &tr(), PX).unwrap();
        assert_eq!(box1.part, Part::List(listbox::Hit::Check(1)));
        assert_eq!(f.click(Some(box1)), Event::Changed("shown"));
        assert_eq!(state(&f), (Some(1), vec![true, false]));
        let label0 = f.hit(60, 12 + 5, &tr(), PX).unwrap();
        assert_eq!(f.click(Some(label0)), Event::Changed("shown"));
        assert_eq!(state(&f), (Some(0), vec![true, false]), "only selected");
        assert_eq!(f.key(Key::Space), Event::Changed("shown"));
        assert_eq!(state(&f), (Some(0), vec![false, false]));
    }

    /// Two Glass samples whose looks share a name but not their art (the
    /// glass in another colour) compare equal as controls, yet the second
    /// renders afresh: a sample is kept only while its style is the same.
    #[test]
    fn a_sample_in_other_glass_art_renders_afresh() {
        use crate::glass;
        use crate::interface::Interface;
        let t = tr();
        let sky = crate::interfaces::theme_for(Interface::Glass);
        let ruby = Theme {
            look: Look::Styled(glass::style(&glass::Settings {
                color: glass::swatch("ruby").expect("ruby is a swatch"),
                ..glass::Settings::default()
            })),
            ..sky.clone()
        };
        let sample = |theme: Theme| Control::Sample {
            themes: false,
            theme,
            icon: None,
        };
        assert_eq!(sample(sky.clone()), sample(ruby.clone()), "one look name");
        let mut f = Form::new(vec![Item::new(
            "sample",
            Rect::new(0, 0, 350, 188),
            sample(sky),
        )]);
        let shot = |f: &Form| {
            let mut c = Canvas::new(360, 190);
            draw(
                &mut c,
                &t,
                &classic_paint(&Theme::default().colors, 1),
                f,
                false,
                PX,
            );
            c
        };
        let first = shot(&f);
        f.items[0].control = sample(ruby);
        let second = shot(&f);
        assert_eq!(f.rendered.renders.get(), 2, "the ruby sample renders");
        assert_ne!(first.pixels, second.pixels, "the ruby glass shows");
    }

    /// A monitor's fitted picture and bezel and a sample render once per
    /// input: a redraw with the same control and scale, such as a caret
    /// blink, blits what the first draw rendered, and a changed control or
    /// scale renders afresh.
    #[test]
    fn monitor_and_sample_render_once_per_input() {
        let t = tr();
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut pic = Canvas::new(40, 30);
        pic.fill_rect(pic.bounds(), Color::rgb(0, 128, 0));
        let mut f = Form::new(vec![
            Item::new(
                "monitor",
                Rect::new(0, 0, 191, 168),
                Control::Monitor(Screen::Picture(pic)),
            ),
            Item::new(
                "sample",
                Rect::new(200, 0, 350, 188),
                Control::Sample {
                    themes: false,
                    theme: Theme::default(),
                    icon: None,
                },
            ),
        ]);
        let shot = |f: &Form, s: i32| {
            let mut c = Canvas::new(560 * s as u32, 190 * s as u32);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &t, &classic_paint(cs, s), f, false, PX);
            c
        };
        let first = shot(&f, 1);
        assert_eq!(f.rendered.renders.get(), 2, "one render per control");
        f.caret = !f.caret;
        assert_eq!(shot(&f, 1).pixels, first.pixels, "a redraw draws the same");
        assert_eq!(f.rendered.renders.get(), 2, "a redraw renders nothing");

        shot(&f, 2);
        assert_eq!(f.rendered.renders.get(), 4, "a new scale renders both");
        let Some(Item {
            control: Control::Sample { theme, .. },
            ..
        }) = f.item_mut("sample")
        else {
            panic!("the sample is the second item")
        };
        theme.colors.active_title = Color::rgb(128, 0, 0);
        let changed = shot(&f, 2);
        assert_eq!(
            f.rendered.renders.get(),
            5,
            "a new theme renders the sample"
        );
        assert_eq!(
            changed.pixels,
            shot(&Form::new(f.items.clone()), 2).pixels,
            "the kept sample draws what a fresh render does"
        );
    }

    #[test]
    fn the_calendar_and_time_field_take_clicks_and_keys() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut f = Form::new(vec![
            Item::new(
                "cal",
                Rect::new(27, 87, calendar::W, calendar::H),
                Control::Calendar(Calendar::new(2026, 9, Some(24), 0)),
            ),
            Item::new(
                "time",
                Rect::new(249, 198, 103, 20),
                Control::Time(TimeField::new(23, 41, 24, true)),
            ),
        ]);
        let day = |f: &Form| match &f.items[0].control {
            Control::Calendar(c) => c.day,
            _ => None,
        };
        // The 1st of September 2026 is a Tuesday: row 0, column 2.
        let hit = f.hit(84, 112, &text, 11);
        assert_eq!(f.click(hit), Event::Changed("cal"));
        assert_eq!(day(&f), Some(1));
        assert_eq!(f.key(Key::Down), Event::Changed("cal"));
        assert_eq!(day(&f), Some(8));
        let hit = f.hit(340, 200, &text, 11);
        assert_eq!(f.click(hit), Event::Changed("time"));
        assert_eq!(f.focus, Some(1));
        let hour = |f: &Form| match &f.items[1].control {
            Control::Time(t) => t.h,
            _ => 99,
        };
        assert_eq!(hour(&f), 12, "11 PM steps to 12 PM, inside its half");
        assert_eq!(f.key(Key::Char('3')), Event::Changed("time"));
        assert_eq!(hour(&f), 15);
        f.set_enabled("cal", false);
        let hit = f.hit(84, 130, &text, 11);
        assert_eq!(f.click(hit), Event::None, "a greyed calendar picks nothing");
    }

    /// The tree a form gives a screen reader: its group holds the radios
    /// inside it, a field is labelled by the text before it, labels lose
    /// their `&`, a check box carries its state and the focused field is
    /// the focus.
    #[test]
    fn a_forms_tree_nests_labels_and_reports_state_and_focus() {
        use crate::access::{ROOT, Tree};
        let mut form = Form::new(vec![
            Item::label_at("namelabel", 10, 10, 80, "&Name:"),
            Item::edit("name", Rect::new(10, 26, 120, 20), "abc", true),
            Item::group("dir", Rect::new(6, 50, 200, 60), "Direction"),
            Item::radio("up", 16, 70, "&Up", "dir").checked(true),
            Item::check("case", 10, 120, "Match &case").checked(true),
            Item::button("ok", Rect::new(10, 140, 75, 23), "OK", false, true),
        ]);
        form.focus_id("name");
        let text = tr();
        let mut tree = Tree::window("Find", (220, 170));
        let focus = form.access(&mut tree, ROOT, &text, 11);

        let edit = tree
            .nodes()
            .find(|(_, n)| n.role() == Role::TextInput)
            .map(|(id, _)| id)
            .expect("a text input");
        assert_eq!(focus, Some(edit), "the focused field is the focus");
        let label = tree.get(edit).unwrap().labelled_by()[0];
        assert_eq!(tree.get(label).unwrap().value(), Some("Name:"));

        let group = tree.find(Role::Group, "Direction").expect("the group");
        let up = tree.find(Role::RadioButton, "Up").expect("the radio");
        assert!(tree.get(group).unwrap().children().contains(&up));
        assert_eq!(
            tree.get(up).unwrap().toggled(),
            Some(accesskit::Toggled::True)
        );

        let case = tree.find(Role::CheckBox, "Match case").expect("the check");
        assert_eq!(
            tree.get(case).unwrap().toggled(),
            Some(accesskit::Toggled::True)
        );
        assert!(tree.find(Role::DefaultButton, "OK").is_some());
    }

    /// A closed combo still lists its rows with the picked one selected,
    /// which is where a reader takes the combo's value from.
    #[test]
    fn a_closed_combo_lists_its_rows_with_the_pick_selected() {
        use crate::access::{ROOT, Tree};
        let mut item = Item::combo("renderer", 10, 10, 120, &["auto", "gles2", "vulkan"], 3);
        if let Control::Combo(c) = &mut item.control {
            c.selected = 1;
        }
        let form = Form::new(vec![item]);
        let mut tree = Tree::window("T", (200, 100));
        let focus = form.access(&mut tree, ROOT, &tr(), 11).expect("focus");
        assert_eq!(tree.get(focus).unwrap().role(), Role::ComboBox);
        let picked: Vec<_> = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::ListBoxOption && n.is_selected() == Some(true))
            .map(|(_, n)| n.label().unwrap_or_default().to_string())
            .collect();
        assert_eq!(picked, ["gles2"]);
    }

    /// A notch over a focused closed combo picks the next item down or up;
    /// past either end, or without the focus, it does nothing.
    #[test]
    fn the_wheel_over_a_focused_closed_combo_steps_its_pick() {
        let items = ["a", "b", "c", "d"];
        let mut f = Form::new(vec![
            Item::combo("c", 10, 10, 120, &items, 3),
            Item::check("check", 10, 50, "Check"),
        ]);
        f.focus_id("check");
        let on = at(&f, 20, 15);
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "not focused");
        assert_eq!(f.combo_selected("c"), Some(0));
        f.focus_id("c");
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Changed("c"));
        assert_eq!(f.combo_selected("c"), Some(1));
        assert_eq!(f.wheel(on, 2 * WHEEL_ROWS), Event::Changed("c"));
        assert_eq!(f.combo_selected("c"), Some(3), "two notches, two items");
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "already last");
        assert_eq!(f.wheel(on, -WHEEL_ROWS), Event::Changed("c"));
        assert_eq!(f.combo_selected("c"), Some(2));
        assert_eq!(f.wheel(None, WHEEL_ROWS), Event::None, "off the combo");
    }

    /// With the list open the wheel scrolls it, wherever the pointer is,
    /// and leaves the pick alone.
    #[test]
    fn the_wheel_scrolls_an_open_combo_list() {
        let items = ["a", "b", "c", "d", "e", "f"];
        let mut f = Form::new(vec![Item::combo("c", 10, 10, 120, &items, 3)]);
        let on = at(&f, 20, 15);
        f.click(on);
        assert!(f.popup_open());
        assert_eq!(f.wheel(None, 2), Event::Redraw);
        let Control::Combo(c) = &f.items[0].control else {
            unreachable!()
        };
        assert_eq!((c.top, c.selected), (2, 0));
        assert_eq!(f.wheel(None, WHEEL_ROWS), Event::Redraw);
        assert_eq!(f.wheel(None, WHEEL_ROWS), Event::None, "at the bottom");
        assert!(f.popup_open(), "the list stays open");
    }

    /// The wheel over a memo scrolls its view a line a row and leaves the
    /// caret; the next draw keeps the view there. A greyed memo stays.
    #[test]
    fn the_wheel_scrolls_a_memos_view_and_not_its_caret() {
        let theme = Theme::default();
        let t = tr();
        let text: Vec<String> = (0..12).map(|k| format!("line {k}")).collect();
        let r = Rect::new(0, 0, 200, 80);
        let mut f = Form::new(vec![Item::memo("m", r, &text.join("\n"), true)]);
        let shot = |f: &Form| {
            let mut c = Canvas::new(200, 80);
            draw(&mut c, &t, &classic_paint(&theme.colors, 1), f, false, PX);
        };
        let memo = |f: &Form| match &f.items[0].control {
            Control::Memo(m) => (m.edit.caret(), m.view.get().top, m.rows.get()),
            _ => unreachable!(),
        };
        shot(&f);
        let rows = memo(&f).2;
        assert!(
            rows > 1 && rows < 12,
            "the box shows part of the text: {rows}"
        );
        let on = at(&f, 20, 20);
        assert!(matches!(on.map(|h| h.part), Some(Part::Memo(_))));
        let start = multiedit::Pos::default();
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Redraw);
        assert_eq!(memo(&f), (start, 3, rows), "three lines a notch");
        shot(&f);
        assert_eq!(
            memo(&f).1,
            3,
            "the draw leaves the view where the wheel put it"
        );
        f.wheel(on, 20 * WHEEL_ROWS);
        assert_eq!(memo(&f).1, 12 - rows, "the last line stops at the bottom");
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "at the bottom");
        f.wheel(on, -20 * WHEEL_ROWS);
        assert_eq!(memo(&f).1, 0);
        f.set_enabled("m", false);
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "a greyed memo");
        assert_eq!(memo(&f).1, 0);
    }

    /// Page Down in a focused memo takes the caret down the rows the box
    /// shows, and the next draw scrolls the view to
    /// it; Page Up takes it back.
    #[test]
    fn page_keys_move_a_memos_caret_a_page() {
        let theme = Theme::default();
        let t = tr();
        let text: Vec<String> = (0..20).map(|k| format!("line {k}")).collect();
        let mut f = Form::new(vec![Item::memo("m", Rect::new(0, 0, 200, 80), "", true)]);
        let shot = |f: &Form| {
            let mut c = Canvas::new(200, 80);
            draw(&mut c, &t, &classic_paint(&theme.colors, 1), f, false, PX);
        };
        let memo = |f: &Form| match &f.items[0].control {
            Control::Memo(m) => (m.edit.caret(), m.view.get().top, m.rows.get()),
            _ => unreachable!(),
        };
        assert_eq!(f.paste(text.join("\n")), Event::Changed("m"));
        shot(&f);
        let rows = memo(&f).2;
        assert!(
            rows > 1 && rows < 20,
            "the box shows part of the text: {rows}"
        );
        f.key(Key::DocEdge {
            end: false,
            select: false,
        });
        shot(&f);
        assert_eq!(memo(&f).1, 0, "Ctrl+Home shows the first line");
        assert_eq!(f.key(Key::PageDown), Event::Redraw);
        let (caret, ..) = memo(&f);
        assert_eq!((caret.line, caret.col), (rows, 0), "a page down");
        shot(&f);
        assert_eq!(memo(&f).1, 1, "the view follows the caret to the last row");
        for _ in 0..20 / rows {
            f.key(Key::PageDown);
        }
        assert_eq!(memo(&f).0.line, 19, "held to the last line");
        assert_eq!(f.key(Key::PageDown), Event::None, "at the last line");
        f.key(Key::DocEdge {
            end: false,
            select: false,
        });
        f.key(Key::PageDown);
        assert_eq!(f.key(Key::PageUp), Event::Redraw);
        assert_eq!(memo(&f).0.line, 0, "Page Up takes it back");
        shot(&f);
        assert_eq!(memo(&f).1, 0);
    }

    /// A notch over a Change Icon list scrolls it one column, down to the
    /// right, held to the ends; the pick stays.
    #[test]
    fn the_wheel_scrolls_an_icon_list_a_column_a_notch() {
        let n = 40;
        let icons = iconlist::IconList::new(
            (0..n).map(|k| format!("i{k}")).collect(),
            vec![None; n],
            Some(0),
        );
        let r = Rect::new(0, 0, 267, 198);
        let mut f = Form::new(vec![Item::new(
            "icons",
            r,
            Control::IconList(Box::new(icons)),
        )]);
        let on = at(&f, 20, 20);
        let left = |f: &Form| match &f.items[0].control {
            Control::IconList(l) => (l.left, l.selected),
            _ => unreachable!(),
        };
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::Redraw);
        assert_eq!(left(&f), (1, Some(0)), "one notch, one column");
        assert_eq!(f.wheel(on, 2 * WHEEL_ROWS), Event::Redraw);
        assert_eq!(left(&f), (3, Some(0)), "ten columns, seven in view");
        assert_eq!(f.wheel(on, WHEEL_ROWS), Event::None, "at the last column");
        assert_eq!(f.wheel(on, -WHEEL_ROWS), Event::Redraw);
        assert_eq!(left(&f), (2, Some(0)));
        assert_eq!(f.wheel(None, -WHEEL_ROWS), Event::None, "off the list");
    }

    /// An open combo's highlight follows the pointer down its list, past
    /// a greyed row, and the pick stays until a click.
    #[test]
    fn an_open_combo_highlight_follows_the_pointer() {
        let mut item = Item::combo("c", 10, 10, 120, &["a", "b", "c", "d"], 4);
        if let Control::Combo(c) = &mut item.control {
            c.items[2].greyed = true;
        }
        let mut f = Form::new(vec![item]);
        f.click(at(&f, 20, 15));
        let open = |f: &Form| match &f.items[0].control {
            Control::Combo(c) => (c.open, c.selected),
            _ => unreachable!(),
        };
        assert_eq!(open(&f), (Some(0), 0));
        // Classic's 13 px rows start under the list's top line at 32.
        let row = |r: i32| (20, 32 + 13 * r + 6);
        let (x, y) = row(1);
        assert_eq!(f.motion(x, y, at(&f, x, y)), Event::Redraw);
        assert_eq!(open(&f), (Some(1), 0));
        assert_eq!(
            f.motion(x, y + 1, at(&f, x, y + 1)),
            Event::None,
            "same row"
        );
        let (x, y) = row(2);
        f.motion(x, y, at(&f, x, y));
        assert_eq!(open(&f), (Some(1), 0), "the greyed row is passed over");
        let (x, y) = row(3);
        f.motion(x, y, at(&f, x, y));
        assert_eq!(open(&f), (Some(3), 0));
        assert_eq!(f.click(at(&f, x, y)), Event::Changed("c"));
        assert_eq!(open(&f), (None, 3));
    }

    /// [`form`] with a trackbar under its buttons.
    fn lit_form() -> Form {
        let mut f = form();
        f.items.push(Item::new(
            "track",
            Rect::new(10, 150, 100, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(3, 10)),
        ));
        f
    }

    /// `f` drawn through `theme`'s look on its face; the draw also tells
    /// the form whether that look lights parts under the pointer.
    fn drawn(f: &Form, theme: &Theme) -> Canvas {
        let mut c = Canvas::new(200, 200);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        draw(&mut c, &tr(), &theme.paint(), f, false, PX);
        c
    }

    /// What (`x`, `y`) hits on `f`.
    fn at(f: &Form, x: i32, y: i32) -> Option<Hit> {
        f.hit(x, y, &tr(), PX)
    }

    /// A point on `part` of item `i`, scanning the item's rect.
    fn point_on(f: &Form, i: usize, part: Part) -> (i32, i32) {
        let r = f.items[i].rect;
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .find(|&(x, y)| at(f, x, y).is_some_and(|h| h.item == i && h.part == part))
            .expect("the part is on the item")
    }

    fn blue() -> Theme {
        crate::interfaces::theme_for(crate::interface::Interface::Blue)
    }

    #[test]
    fn under_blue_a_motion_lights_the_control_under_it_and_a_leave_clears_it() {
        let mut f = lit_form();
        drawn(&f, &blue());
        let (ok, check) = (f.index_of("ok").unwrap(), f.index_of("check").unwrap());
        assert_eq!(f.motion(20, 120, at(&f, 20, 120)), Event::Redraw);
        assert_eq!(f.lit(ok, Light::Body), Interaction::Hot);
        assert_eq!(
            f.motion(30, 121, at(&f, 30, 121)),
            Event::None,
            "a move within the button changes nothing"
        );
        assert_eq!(f.motion(60, 65, at(&f, 60, 65)), Event::Redraw);
        assert_eq!(f.lit(check, Light::Body), Interaction::Hot);
        assert_eq!(f.lit(ok, Light::Body), Interaction::Idle);
        assert_eq!(f.leave(), Event::Redraw);
        assert_eq!(f.lit(check, Light::Body), Interaction::Idle);
        assert_eq!(f.leave(), Event::None, "nothing left to clear");
    }

    /// An editable Blue combo lights its drop button only from the button:
    /// with the pointer on its field the button draws at rest, on the
    /// button it draws hot.
    #[test]
    fn an_editable_blue_combo_lights_its_drop_only_from_the_drop() {
        let mut cb = Combo::new(&["a", "b"], 0, 4);
        cb.edit = Some(TextEdit::new("text", false));
        let mut f = Form::new(vec![Item::new(
            "c",
            Rect::new(10, 10, 140, combo::COMBO_H),
            Control::Combo(cb),
        )]);
        let theme = blue();
        drawn(&f, &theme);
        let lay = combo::layout(
            match &f.items[0].control {
                Control::Combo(c) => c,
                _ => unreachable!(),
            },
            f.items[0].rect,
            &theme.look.controls(),
        );
        let drop_at = |ptr| {
            let mut c = Canvas::new(200, 200);
            c.fill_rect(c.bounds(), theme.colors.window);
            crate::look::widgets::draw_combo_drop(
                &mut c,
                &theme.paint(),
                lay.drop,
                true,
                ptr,
                true,
            );
            c
        };
        let same = |a: &Canvas, b: &Canvas| {
            let r = lay.drop;
            (r.y..r.bottom()).all(|y| (r.x..r.right()).all(|x| a.get(x, y) == b.get(x, y)))
        };
        let (fx, fy) = point_on(&f, 0, Part::Combo(combo::Hit::Field));
        f.motion(fx, fy, at(&f, fx, fy));
        assert_eq!(f.lit(0, Light::Field), Interaction::Hot, "the field lights");
        assert!(
            same(&drawn(&f, &theme), &drop_at(Interaction::Idle)),
            "with the pointer on the field the drop draws Normal"
        );
        let (dx, dy) = point_on(&f, 0, Part::Combo(combo::Hit::Drop));
        f.motion(dx, dy, at(&f, dx, dy));
        assert!(
            same(&drawn(&f, &theme), &drop_at(Interaction::Hot)),
            "with the pointer on the drop it draws Hot"
        );
    }

    #[test]
    fn under_classic_hover_asks_no_redraw_but_a_held_button_dragged_off_does() {
        let mut f = lit_form();
        drawn(&f, &Theme::default());
        assert_eq!(f.motion(20, 120, at(&f, 20, 120)), Event::None);
        assert_eq!(f.leave(), Event::None);
        let ok = at(&f, 20, 120);
        assert_eq!(f.click(ok), Event::Redraw);
        assert_eq!(f.motion(300, 300, None), Event::Redraw, "dragged off");
        assert_eq!(f.motion(20, 120, ok), Event::Redraw, "back on");
        assert_eq!(f.release(), Event::Pressed("ok"));
    }

    /// Hot outranks the default ring; a held button dragged off draws as
    /// at rest, and pushed again back on.
    #[test]
    fn a_blue_form_draws_the_hot_button_and_a_held_one_dragged_off_at_rest() {
        let theme = blue();
        let mut f = lit_form();
        let rest = drawn(&f, &theme);
        let ok = at(&f, 20, 120);
        f.motion(20, 120, ok);
        let hot = drawn(&f, &theme);
        for (x, y) in [(7, 2), (3, 10)] {
            let (h, r) = (hot.get(10 + x, 110 + y), rest.get(10 + x, 110 + y));
            assert_ne!(h, r, "the hot ring lights ({x}, {y})");
        }
        f.click(ok);
        let held = drawn(&f, &theme);
        let (h, r) = (held.get(10 + 10, 110 + 3), rest.get(10 + 10, 110 + 3));
        assert!(h & 0xFF < r & 0xFF, "the pushed face is darker at its top");
        f.motion(300, 300, None);
        let off = drawn(&f, &theme);
        let r = f.items[f.index_of("ok").unwrap()].rect;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                assert_eq!(off.get(x, y), rest.get(x, y), "({x}, {y}) is at rest");
            }
        }
        f.motion(20, 120, ok);
        assert!(drawn(&f, &theme) == held, "back on, it is pushed again");
    }

    /// A Blue check box lights from its label; held, it draws its Pressed
    /// frame with no tick yet, and the release over it ticks it and leaves
    /// it hot.
    #[test]
    fn a_blue_check_box_draws_pressed_while_held_and_ticks_on_the_release() {
        let theme = blue();
        let mut f = lit_form();
        drawn(&f, &theme);
        use crate::style::{Check, Part, State};
        let box_px = |c: &Canvas, x: i32, y: i32| c.get(10 + x, 60 + y);
        let frame = |check: Check, state: State, x: i32, y: i32| {
            let mut c = Canvas::new(13, 13);
            let r = Rect::new(0, 0, 13, 13);
            crate::blue::style().draw_check(&mut c, r, Part::CheckBox, check, state, 1);
            c.get(x, y)
        };
        f.motion(60, 65, at(&f, 60, 65));
        let hot = drawn(&f, &theme);
        let lit = frame(Check::Unchecked, State::Hot, 1, 1);
        assert_eq!(box_px(&hot, 1, 1), lit, "the orange ring");
        assert_eq!(
            box_px(&hot, 6, 6),
            frame(Check::Unchecked, State::Hot, 6, 6),
            "no tick yet"
        );
        assert_eq!(f.click(at(&f, 60, 65)), Event::Redraw);
        let held = drawn(&f, &theme);
        assert_eq!(
            box_px(&held, 1, 1),
            frame(Check::Unchecked, State::Pressed, 1, 1),
            "the pressed well"
        );
        assert_eq!(
            box_px(&held, 6, 6),
            frame(Check::Unchecked, State::Pressed, 6, 6),
            "the pressed fill, no tick"
        );
        assert_eq!(f.release(), Event::Changed("check"));
        let after = drawn(&f, &theme);
        assert_eq!(box_px(&after, 1, 1), lit, "hot again");
        let tick = crate::blue::Variant::Blue
            .scheme()
            .check_box
            .normal_checked
            .mark;
        assert_eq!(
            box_px(&after, 6, 7),
            0xFF00_0000 | tick.unwrap().argb(),
            "the tick"
        );
    }

    /// Check boxes and radios act on the release, as push buttons do, in
    /// every look: the press holds one, a release over it toggles it, a
    /// release after dragging off changes nothing, and dragging back on
    /// arms it again. A radio already picked stays picked; a greyed box
    /// takes neither the press nor the release.
    #[test]
    fn a_check_box_and_a_radio_toggle_on_the_release_over_them_in_every_look() {
        use crate::interface::Interface;
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(look);
            let mut f = Form::new(vec![
                Item::check("check", 10, 10, "&Check"),
                Item::radio("up", 10, 40, "&Up", "dir"),
                Item::radio("down", 10, 70, "&Down", "dir").checked(true),
            ]);
            drawn(&f, &theme);
            for id in ["check", "up"] {
                let i = f.index_of(id).unwrap();
                let (x, y) = point_on(&f, i, Part::Body);
                let on = at(&f, x, y);
                let was = f.checked(id).unwrap();
                assert_eq!(f.click(on), Event::Redraw, "{look:?} {id}: held");
                assert_eq!(f.lit(i, Light::Body), Interaction::Pressed);
                assert_eq!(f.checked(id), Some(was), "{look:?} {id}: the press waits");
                f.motion(300, 300, None);
                assert_eq!(f.lit(i, Light::Body), Interaction::Idle, "dragged off");
                assert_eq!(f.release(), Event::Redraw, "{look:?} {id}: released off");
                assert_eq!(f.checked(id), Some(was), "{look:?} {id}: unchanged");
                f.click(on);
                f.motion(300, 300, None);
                f.motion(x, y, on);
                assert_eq!(f.lit(i, Light::Body), Interaction::Pressed, "back on");
                assert_eq!(f.release(), Event::Changed(id), "{look:?} {id}");
                assert_eq!(f.checked(id), Some(!was), "{look:?} {id}: toggled");
                assert_eq!(f.lit(i, Light::Body), Interaction::Hot, "{look:?} {id}");
            }
            assert_eq!(f.checked("down"), Some(false), "{look:?}: up cleared down");
            let up = at(&f, 12, 42);
            f.click(up);
            assert_eq!(f.release(), Event::Redraw, "{look:?}: up is already picked");
            assert_eq!(f.checked("up"), Some(true), "{look:?}: up stays picked");
            f.set_enabled("check", false);
            let check = at(&f, 12, 12);
            assert_eq!(f.click(check), Event::None, "{look:?}: greyed");
            assert!(!f.holding(), "{look:?}: a greyed box is not held");
            assert_eq!(f.release(), Event::None, "{look:?}: greyed");
            assert_eq!(
                f.checked("check"),
                Some(true),
                "{look:?}: greyed, unchanged"
            );
        }
    }

    /// A spinner's button lights alone, hot then pressed while held.
    #[test]
    fn a_blue_spin_button_lights_hot_and_pressed() {
        let theme = blue();
        let mut f = lit_form();
        drawn(&f, &theme);
        let i = f.index_of("spin").unwrap();
        let (x, y) = point_on(&f, i, Part::UpDown(updown::Hit::Up));
        assert_eq!(f.motion(x, y, at(&f, x, y)), Event::Redraw);
        assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Hot)));
        let up = match &f.items[i].control {
            Control::UpDown(u) => updown::layout_for(u, f.items[i].rect, &theme.look.controls()).up,
            _ => unreachable!("spin is an up-down"),
        };
        let edge = |state| {
            let st = crate::blue::style();
            let img = st.part(crate::style::Part::SpinUp).unwrap();
            let f = img.frame(state);
            img.atlas.get(f.x, f.y + 3)
        };
        let hot = drawn(&f, &theme);
        assert_eq!(
            hot.get(up.x, up.y + 3),
            edge(crate::style::State::Hot),
            "the hot button's edge"
        );
        f.click(at(&f, x, y));
        assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Pressed)));
        let held = drawn(&f, &theme);
        assert_eq!(
            held.get(up.x, up.y + 3),
            edge(crate::style::State::Pressed),
            "the held button's edge"
        );
        assert_eq!(f.release(), Event::Redraw, "back to hot");
        assert_eq!(f.spin_lit(i), Some((Arrow::Up, Interaction::Hot)));
    }

    /// A held trackbar thumb stays pressed wherever the pointer drags it.
    #[test]
    fn a_held_thumb_stays_pressed_off_it_until_the_release() {
        let mut f = lit_form();
        drawn(&f, &blue());
        let i = f.index_of("track").unwrap();
        let (x, y) = point_on(&f, i, Part::TrackBar(trackbar::Hit::Thumb));
        f.motion(x, y, at(&f, x, y));
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
        f.click(at(&f, x, y));
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Pressed);
        f.motion(x, 300, None);
        assert_eq!(
            f.lit(i, Light::Thumb),
            Interaction::Pressed,
            "off the thumb"
        );
        f.release();
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle, "released off it");
    }

    #[test]
    fn a_trackbar_thumb_moved_by_keys_or_the_host_follows_the_pointer() {
        let mut f = lit_form();
        drawn(&f, &blue());
        let i = f.index_of("track").unwrap();
        f.set_focus(Some(i));
        let (x, y) = point_on(&f, i, Part::TrackBar(trackbar::Hit::Thumb));
        f.motion(x, y, at(&f, x, y));
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
        // A position is 9 or 10 px; the pointer is on the thumb's left
        // column.
        assert_eq!(f.key(Key::Right), Event::Changed("track"));
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle, "moved off it");
        assert_eq!(f.key(Key::Left), Event::Changed("track"));
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot, "back under it");
        f.set_track("track", 5);
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle);
        f.set_track("track", 3);
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Hot);
        f.leave();
        f.set_track("track", 4);
        f.set_track("track", 3);
        assert_eq!(f.lit(i, Light::Thumb), Interaction::Idle);
    }

    /// A host rebuilds the open page's `Form` on every model change. `carry_pointer` puts back
    /// what the old one held, so a held check box, spin button or thumb keeps drawing pressed
    /// and its release still resolves.
    #[test]
    fn carry_pointer_keeps_a_held_check_spin_and_thumb_pressed_across_a_rebuild() {
        let theme = blue();

        let mut f = lit_form();
        drawn(&f, &theme);
        let check = f.index_of("check").unwrap();
        assert_eq!(f.click(at(&f, 60, 65)), Event::Redraw);
        let mut rebuilt = lit_form();
        rebuilt.carry_pointer(&f);
        drawn(&rebuilt, &theme);
        assert_eq!(rebuilt.held_item(), Some(check));
        assert_eq!(rebuilt.lit(check, Light::Body), Interaction::Pressed);
        assert_eq!(rebuilt.release(), Event::Changed("check"));
        assert_eq!(rebuilt.checked("check"), Some(true));
        assert_eq!(rebuilt.lit(check, Light::Body), Interaction::Hot);

        let mut f = lit_form();
        drawn(&f, &theme);
        let spin_i = f.index_of("spin").unwrap();
        let (x, y) = point_on(&f, spin_i, Part::UpDown(updown::Hit::Up));
        f.motion(x, y, at(&f, x, y));
        assert_eq!(f.click(at(&f, x, y)), Event::Changed("spin"));
        assert_eq!(spin(&f).value(), 11);
        // A real host rebuilds the page from the model the press just
        // changed, so the rebuilt spin already reads the stepped value.
        let mut rebuilt = lit_form();
        rebuilt.items[spin_i].control = f.items[spin_i].control.clone();
        rebuilt.carry_pointer(&f);
        drawn(&rebuilt, &theme);
        assert_eq!(
            rebuilt.spin_lit(spin_i),
            Some((Arrow::Up, Interaction::Pressed))
        );
        assert_eq!(rebuilt.release(), Event::Redraw, "back to hot");
        assert_eq!(
            rebuilt.spin_lit(spin_i),
            Some((Arrow::Up, Interaction::Hot))
        );
        assert_eq!(spin(&rebuilt).value(), 11, "release did not step it again");

        let mut f = lit_form();
        drawn(&f, &theme);
        let track = f.index_of("track").unwrap();
        let (x, y) = point_on(&f, track, Part::TrackBar(trackbar::Hit::Thumb));
        f.motion(x, y, at(&f, x, y));
        f.click(at(&f, x, y));
        f.motion(x, 300, None);
        assert_eq!(
            f.lit(track, Light::Thumb),
            Interaction::Pressed,
            "off it, still sticky"
        );
        let mut rebuilt = lit_form();
        rebuilt.carry_pointer(&f);
        drawn(&rebuilt, &theme);
        assert_eq!(
            rebuilt.lit(track, Light::Thumb),
            Interaction::Pressed,
            "sticky pressed survives the rebuild"
        );
        rebuilt.release();
        assert_eq!(
            rebuilt.lit(track, Light::Thumb),
            Interaction::Idle,
            "released off it"
        );
    }

    /// A page rebuilt under the pointer (a keystroke in one of its
    /// edits) keeps a hot edit and an editable combo's hot field lit.
    #[test]
    fn carry_pointer_keeps_a_hot_edit_and_an_editable_combo_field_lit() {
        let form = || {
            let mut cb = Combo::new(&["a"], 0, 4);
            cb.edit = Some(TextEdit::new("text", false));
            Form::new(vec![
                Item::edit("edit", Rect::new(10, 10, 100, 21), "x", true),
                Item::new(
                    "combo",
                    Rect::new(10, 40, 140, combo::COMBO_H),
                    Control::Combo(cb),
                ),
            ])
        };
        let theme = blue();
        for (id, part, light) in [
            ("edit", Part::Edit(0), Light::Body),
            ("combo", Part::Combo(combo::Hit::Field), Light::Field),
        ] {
            let mut f = form();
            drawn(&f, &theme);
            let i = f.index_of(id).unwrap();
            let (x, y) = (f.items[i].rect.x + 20, f.items[i].rect.y + 10);
            let hit = at(&f, x, y);
            assert_eq!(
                hit.map(|h| (h.item, std::mem::discriminant(&h.part))),
                Some((i, std::mem::discriminant(&part))),
                "{id}: the point is on its field"
            );
            f.motion(x, y, hit);
            assert_eq!(f.lit(i, light), Interaction::Hot, "{id} lights");
            let mut rebuilt = form();
            rebuilt.carry_pointer(&f);
            assert_eq!(
                rebuilt.lit(i, light),
                Interaction::Hot,
                "{id} stays hot across the rebuild"
            );
        }
    }

    /// A rebuild that drops the held item (its id is gone, or its part no
    /// longer exists on it) leaves the new form with nothing held, rather
    /// than pointing the pressed look at a different control.
    #[test]
    fn carry_pointer_drops_a_held_item_the_rebuild_no_longer_has() {
        let mut f = lit_form();
        drawn(&f, &blue());
        assert_eq!(f.click(at(&f, 20, 120)), Event::Redraw, "ok held");
        assert_eq!(f.held_item(), f.index_of("ok"));

        let mut rebuilt = Form::new(
            lit_form()
                .items
                .into_iter()
                .filter(|it| it.id != "ok")
                .collect(),
        );
        rebuilt.carry_pointer(&f);
        assert!(!rebuilt.holding(), "the held button no longer exists");
        assert_eq!(rebuilt.pointer.hot(), None);
    }
}
