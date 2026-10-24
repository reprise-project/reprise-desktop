use std::cell::Cell;

use crate::classic::widgets::LINE_H;
use crate::controls::combo::{self, Combo};
use crate::controls::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::Align;
use crate::textedit::TextEdit;
use crate::theme::Color;

use super::{CHECK, Control, Memo, RADIO_ROW};

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
    /// What a reader calls a control no drawn label names ([`Item::named`]).
    pub name: Option<&'static str>,
    /// A framed picture or an icon list that grows with the text ([`Item::grows`]).
    pub grows: bool,
}

/// How an item's rect is anchored. `UnderCheck`: under a check box's label, starting
/// `check_label` right of the box. `Wraps` and `CheckWrap`: a check box whose label runs onto
/// a second line, and that line (at `check_wrap`); both sit `check_wrap_dy` below their rects.
/// `Px(n)`: `n` px right of its rect's left edge at any text size, its right edge kept, as text
/// beside a picture that keeps its size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Indent {
    #[default]
    None,
    UnderCheck,
    Wraps,
    CheckWrap,
    Px(i32),
}

impl Item {
    pub fn new(id: &'static str, rect: Rect, control: Control) -> Item {
        Item {
            id,
            rect,
            control,
            indent: Indent::None,
            name: None,
            grows: false,
        }
    }

    pub fn named(self, name: &'static str) -> Item {
        Item {
            name: Some(name),
            ..self
        }
    }

    /// A picture whose size grows with the text, its frame kept, or an icon list whose cells
    /// grow: its host hands it canvases at the grown size. Other pictures and icon lists keep
    /// their size.
    pub fn grows(self) -> Item {
        Item {
            grows: true,
            ..self
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

    pub fn indent_px(self, n: i32) -> Item {
        Item {
            indent: Indent::Px(n),
            ..self
        }
    }

    /// Where the item draws and takes the pointer at the control sizes
    /// `cm`: its rect, moved by its [`Indent`]. A wrapping check box's
    /// own rect stays put; its label moves ([`check_label_rect`](super::layout::check_label_rect)).
    pub fn placed(&self, cm: &ControlMetrics) -> Rect {
        let (dx, dy, dw) = match self.indent {
            Indent::None | Indent::Wraps => (0, 0, 0),
            Indent::UnderCheck => (cm.check_label, 0, 0),
            Indent::CheckWrap => (cm.check_wrap, cm.check_wrap_dy, 0),
            Indent::Px(n) => (n, 0, -n),
        };
        Rect {
            x: self.rect.x + dx,
            y: self.rect.y + dy,
            width: self.rect.width + dw,
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
            Control::Lines(l) | Control::GreyedLines(l) => {
                let l = std::mem::take(l);
                self.control = if on {
                    Control::Lines(l)
                } else {
                    Control::GreyedLines(l)
                };
            }
            _ => {}
        }
        self
    }

    pub(super) fn focusable(&self) -> bool {
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

    pub(crate) fn mnemonic(&self) -> Option<char> {
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
