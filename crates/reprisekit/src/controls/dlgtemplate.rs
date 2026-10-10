//! The fixed templates the shell's own prompts are built on (Run, delete confirmations, key aid
//! prompts, desktop notices): a width, icon margins, text wrap and a button row. [`layout`] works
//! in whole-dialog coordinates under a [`FRAME`] px frame and [`CAPTION_H`] caption; a program
//! whose frame the compositor draws takes [`Layout::client`] and [`form`].

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H, ROW_H};
use crate::controls::form::{Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::{TextRenderer, wrap};

pub const FRAME: i32 = 3;
pub const CAPTION_H: i32 = 18;
pub const ICON_PX: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Run,
    MessageBox,
    MultiDelete,
    ShellNotice,
    KeyAid,
}

struct Metrics {
    width: Option<i32>,
    left: i32,
    top: i32,
    icon_gap: i32,
    line_pitch: i32,
    field_gap: i32,
    button_gap: i32,
    right: i32,
    button_right: i32,
    field_right: i32,
    bottom: i32,
    centered: bool,
    text_dy: i32,
    button_w: i32,
    button_pitch: i32,
}

const KEY_AID_TOP: i32 = 9;
pub const KEY_AID_TEXT_H: i32 = 7 * LINE_H;
const KEY_AID_BUTTONS: i32 = 129;

impl Template {
    fn metrics(self) -> Metrics {
        match self {
            Template::Run => Metrics {
                width: Some(347),
                left: 14,
                top: 19,
                icon_gap: 11,
                line_pitch: LINE_H,
                field_gap: 10,
                button_gap: 33,
                right: 14,
                button_right: 14,
                field_right: 15,
                bottom: 20,
                centered: false,
                text_dy: 0,
                button_w: BUTTON_W,
                button_pitch: BUTTON_GAP,
            },
            Template::MessageBox => Metrics {
                width: Some(419),
                left: 17,
                top: 17,
                icon_gap: 16,
                line_pitch: LINE_H,
                field_gap: 10,
                button_gap: 26,
                right: 24,
                button_right: 13,
                field_right: 24,
                bottom: 14,
                centered: false,
                text_dy: 0,
                button_w: 80,
                button_pitch: 7,
            },
            Template::MultiDelete => Metrics {
                width: Some(419),
                left: 17,
                top: 17,
                icon_gap: 16,
                line_pitch: LINE_H,
                field_gap: 10,
                button_gap: 26,
                right: 14,
                button_right: 14,
                field_right: 14,
                bottom: 16,
                centered: false,
                text_dy: 7,
                button_w: 87,
                button_pitch: 7,
            },
            Template::KeyAid => Metrics {
                width: Some(410),
                left: 18,
                top: KEY_AID_TOP,
                icon_gap: 0,
                line_pitch: LINE_H,
                field_gap: 10,
                button_gap: KEY_AID_BUTTONS - KEY_AID_TOP - KEY_AID_TEXT_H - 21,
                right: 18,
                button_right: 83,
                field_right: 18,
                bottom: 11,
                centered: false,
                text_dy: 0,
                button_w: 75,
                button_pitch: 12,
            },
            Template::ShellNotice => Metrics {
                width: None,
                left: 14,
                top: 12,
                icon_gap: 19,
                line_pitch: LINE_H,
                field_gap: 10,
                button_gap: 17,
                right: 10,
                button_right: 10,
                field_right: 10,
                bottom: 14,
                centered: true,
                text_dy: 0,
                button_w: BUTTON_W,
                button_pitch: BUTTON_GAP,
            },
        }
    }
}

pub struct Fields<'a> {
    pub text: &'a str,
    pub icon: bool,
    pub field: bool,
    pub buttons: usize,
    pub text_h: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub size: (i32, i32),
    pub icon: Option<Rect>,
    pub lines: Vec<(Rect, String)>,
    pub field_label: Option<Rect>,
    pub field: Option<Rect>,
    pub buttons: Vec<Rect>,
}

impl Layout {
    pub fn client(mut self) -> Layout {
        let (dx, dy) = (FRAME, FRAME + CAPTION_H);
        let mv = |r: &mut Rect| {
            r.x -= dx;
            r.y -= dy;
        };
        [&mut self.icon, &mut self.field_label, &mut self.field]
            .into_iter()
            .flatten()
            .for_each(mv);
        self.lines.iter_mut().for_each(|(r, _)| mv(r));
        self.buttons.iter_mut().for_each(mv);
        self.size = (self.size.0 - 2 * FRAME, self.size.1 - 2 * FRAME - CAPTION_H);
        self
    }
}

pub fn layout(template: Template, text: &TextRenderer, px: u32, f: &Fields) -> Layout {
    let cm = template.metrics();
    let left = cm.left;
    let text_x = if f.icon {
        left + ICON_PX + cm.icon_gap
    } else {
        left
    };
    let (w, wrapped) = match cm.width {
        Some(w) => (w, wrap(text, f.text, w - cm.right - text_x, px)),
        None => {
            let lines = wrap(text, f.text, i32::MAX, px);
            let widest = lines
                .iter()
                .map(|l| text.measure(l, false, px))
                .max()
                .unwrap_or(0);
            (text_x + widest + cm.right, lines)
        }
    };
    let body_y = FRAME + CAPTION_H + cm.top;
    let right = w - cm.right;
    let icon = f.icon.then(|| Rect::new(left, body_y, ICON_PX, ICON_PX));
    let text_span = (wrapped.len() as i32 - 1).max(0) * cm.line_pitch + LINE_H;
    let text_y = match icon {
        Some(r) if cm.centered && text_span < r.height => r.y + (r.height - text_span) / 2,
        _ => body_y + cm.text_dy,
    };
    let lines: Vec<(Rect, String)> = wrapped
        .into_iter()
        .enumerate()
        .filter(|(_, s)| !cm.centered || !s.is_empty())
        .map(|(i, s)| {
            (
                Rect::new(
                    text_x,
                    text_y + i as i32 * cm.line_pitch,
                    right - text_x,
                    LINE_H,
                ),
                s,
            )
        })
        .collect();
    let text_bottom = body_y + text_span.max(f.text_h.unwrap_or(0));
    let content_bottom = text_bottom.max(icon.map(|r| r.bottom()).unwrap_or(0));
    let mut y = content_bottom;
    let (mut field_label, mut field) = (None, None);
    if f.field {
        y += cm.field_gap;
        field_label = Some(Rect::new(left, y, text_x - left, ROW_H));
        field = Some(Rect::new(text_x, y, w - cm.field_right - text_x, ROW_H));
        y += ROW_H;
    }
    y += cm.button_gap;
    let n = f.buttons as i32;
    let total = n * cm.button_w + (n - 1).max(0) * cm.button_pitch;
    let bx = if cm.centered {
        (w - total) / 2
    } else {
        w - cm.button_right - total
    };
    let buttons = (0..n)
        .map(|i| {
            Rect::new(
                bx + i * (cm.button_w + cm.button_pitch),
                y,
                cm.button_w,
                BUTTON_H,
            )
        })
        .collect();
    Layout {
        size: (w, y + BUTTON_H + cm.bottom),
        icon,
        lines,
        field_label,
        field,
        buttons,
    }
}

pub const BUTTON_IDS: [&str; 4] = ["btn0", "btn1", "btn2", "btn3"];

/// A form over a client-side `lay` with no field: the icon, top-aligned text and `buttons` left to
/// right, the `default`-th ringed and focused. Panics past four buttons.
pub fn form(lay: &Layout, icon: Option<Canvas>, buttons: &[String], default: usize) -> Form {
    let mut items = Vec::new();
    if let Some(r) = lay.icon {
        items.push(Item::picture("icon", r, icon));
    }
    if let Some((first, _)) = lay.lines.first() {
        let lines: Vec<&str> = lay.lines.iter().map(|(_, s)| s.as_str()).collect();
        let h = lay
            .lines
            .last()
            .map_or(first.height, |(r, _)| r.bottom() - first.y);
        items.push(Item::lines(
            "text",
            Rect::new(first.x, first.y, first.width, h),
            &lines,
        ));
    }
    for (i, (r, label)) in lay.buttons.iter().zip(buttons).enumerate() {
        items.push(Item::button(BUTTON_IDS[i], *r, label, true, i == default));
    }
    let mut form = Form::new(items);
    form.focus_id(BUTTON_IDS[default]);
    form
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{FontConfig, Theme};

    fn rend() -> (TextRenderer, u32) {
        (
            TextRenderer::load(&FontConfig::default()),
            Theme::default().fonts.size_px,
        )
    }

    #[test]
    fn a_short_confirm_keeps_the_template_width() {
        const EXPECTED_W: i32 = 419;
        const EXPECTED_ICON_X: i32 = 17;
        const EXPECTED_BUTTON_W: i32 = 80;
        const EXPECTED_BUTTON_RIGHT: i32 = 13;
        let (text, px) = rend();
        let f = Fields {
            text: "Are you sure you want to delete 'a'?",
            icon: true,
            field: false,
            buttons: 2,
            text_h: None,
        };
        let lay = layout(Template::MessageBox, &text, px, &f);
        assert_eq!(lay.size.0, EXPECTED_W);
        assert_eq!(lay.icon.map(|r| r.x), Some(EXPECTED_ICON_X));
        assert_eq!(lay.lines.len(), 1);
        let no = lay.buttons[1];
        assert_eq!(no.width, EXPECTED_BUTTON_W);
        assert_eq!(EXPECTED_W - no.right(), EXPECTED_BUTTON_RIGHT);
        assert_eq!(lay.buttons[0].right() + 7, no.x, "7 px between the buttons");
    }

    #[test]
    fn the_client_layout_drops_the_frame_and_caption() {
        let (text, px) = rend();
        let f = Fields {
            text: "Are you sure you want to delete these 2 items?",
            icon: true,
            field: false,
            buttons: 2,
            text_h: None,
        };
        let whole = layout(Template::MultiDelete, &text, px, &f);
        let client = whole.clone().client();
        assert_eq!(client.size, (whole.size.0 - 6, whole.size.1 - 24));
        let (a, b) = (whole.buttons[0], client.buttons[0]);
        assert_eq!((b.x, b.y), (a.x - 3, a.y - 21));
        let form = form(&client, None, &["&Yes".into(), "&No".into()], 0);
        assert_eq!(
            form.item(BUTTON_IDS[1]).map(|it| it.rect),
            Some(client.buttons[1])
        );
    }
}
