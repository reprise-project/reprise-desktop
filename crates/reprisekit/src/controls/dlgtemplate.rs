//! The fixed templates the shell's own prompts are built on (Run, delete confirmations, key aid
//! prompts, desktop notices): a width, icon margins, text wrap and a button row. [`layout`] works
//! in whole-dialog coordinates under a [`FRAME`] px frame and [`CAPTION_H`] caption, for 100%
//! text; [`Layout::grown`] puts it at the text size. A program whose frame the compositor draws
//! takes [`Layout::client`] and [`form`], which grows with the form.

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H, ROW_H};
use crate::controls::form::{Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::{TextRenderer, split_mnemonic, wrap};
use crate::theme::TextUnits;

pub const FRAME: i32 = 3;
pub const CAPTION_H: i32 = 18;
pub const ICON_PX: i32 = 32;
/// The 100% room each side of a button's label: it clears the focus ring 4 px inside the edge.
const BUTTON_LABEL_PAD: i32 = 6;

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
    /// The button labels, `&` before an access key; every button takes the widest one's width.
    pub buttons: &'a [String],
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

    /// How far right of the icon's left edge the text starts, when it keeps that at every text
    /// size: beside an icon with no field below. The icon and its gap keep their px; only the
    /// text column grows.
    fn beside_icon(&self) -> Option<i32> {
        let icon = self.icon.filter(|_| self.field.is_none())?;
        self.lines.first().map(|(r, _)| r.x - icon.x)
    }

    /// This whole-dialog layout at the text size `u`: the client area and every rect in it grow
    /// from the client's corner; the frame and caption keep theirs.
    pub fn grown(mut self, u: TextUnits) -> Layout {
        let beside = self.beside_icon();
        let (ox, oy) = (FRAME, FRAME + CAPTION_H);
        let map = |r: &mut Rect| {
            let g = u.rect(Rect::new(r.x - ox, r.y - oy, r.width, r.height));
            *r = Rect::new(g.x + ox, g.y + oy, g.width, g.height);
        };
        [&mut self.icon, &mut self.field_label, &mut self.field]
            .into_iter()
            .flatten()
            .for_each(map);
        if let Some(icon) = &mut self.icon {
            (icon.width, icon.height) = (ICON_PX, ICON_PX);
        }
        self.lines.iter_mut().for_each(|(r, _)| map(r));
        if let (Some(step), Some(icon)) = (beside, self.icon) {
            for (r, _) in &mut self.lines {
                let right = r.right();
                r.x = icon.x + step;
                r.width = right - r.x;
            }
        }
        self.buttons.iter_mut().for_each(map);
        let (cw, ch) = u.size((self.size.0 - 2 * FRAME, self.size.1 - 2 * FRAME - CAPTION_H));
        self.size = (cw + 2 * FRAME, ch + 2 * FRAME + CAPTION_H);
        self
    }
}

/// `template` laid out at 100% for text set at `px`, which grows by `u`: the text wraps at the
/// width its column grows to, and a column sized to its text reserves the 100% width that grows
/// to fit it.
pub fn layout(
    template: Template,
    text: &TextRenderer,
    px: u32,
    f: &Fields,
    u: TextUnits,
) -> Layout {
    let cm = template.metrics();
    let left = cm.left;
    let text_x = if f.icon {
        left + ICON_PX + cm.icon_gap
    } else {
        left
    };
    // Where the text starts at the text size: see `Layout::beside_icon`.
    let start = if f.icon && !f.field {
        u.len(left) + ICON_PX + cm.icon_gap
    } else {
        u.len(text_x)
    };
    let (w, wrapped) = match cm.width {
        Some(w) => (w, wrap(text, f.text, u.len(w - cm.right) - start, px)),
        None => {
            let lines = wrap(text, f.text, i32::MAX, px);
            let widest = lines
                .iter()
                .map(|l| text.measure(l, false, px))
                .max()
                .unwrap_or(0);
            (u.design(start + widest) + cm.right, lines)
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
    let n = f.buttons.len() as i32;
    let widest = f
        .buttons
        .iter()
        .map(|l| text.ink_width(&split_mnemonic(l).0, false, px))
        .max()
        .unwrap_or(0);
    let button_w = cm.button_w.max(u.design(widest) + 2 * BUTTON_LABEL_PAD);
    let total = n * button_w + (n - 1).max(0) * cm.button_pitch;
    let bx = if cm.centered {
        (w - total) / 2
    } else {
        w - cm.button_right - total
    };
    let buttons = (0..n)
        .map(|i| Rect::new(bx + i * (button_w + cm.button_pitch), y, button_w, BUTTON_H))
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
        let step = lay.beside_icon().unwrap_or(0);
        let r = Rect::new(first.x - step, first.y, first.width + step, h);
        items.push(Item::lines("text", r, &lines).indent_px(step));
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
            buttons: &["&Yes".into(), "&No".into()],
            text_h: None,
        };
        let lay = layout(Template::MessageBox, &text, px, &f, TextUnits::ONE);
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
            buttons: &["&Yes".into(), "&No".into()],
            text_h: None,
        };
        let whole = layout(Template::MultiDelete, &text, px, &f, TextUnits::ONE);
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

    #[test]
    fn a_label_wider_than_the_template_button_widens_every_button() {
        let (text, px) = rend();
        let labels = ["&Run".into(), "Open in &Notepad".into(), "Cancel".into()];
        let f = Fields {
            text: "Running it can change or delete your files.",
            icon: true,
            field: false,
            buttons: &labels,
            text_h: None,
        };
        let lay = layout(Template::MessageBox, &text, px, &f, TextUnits::ONE);
        let ink = text.ink_width("Open in Notepad", false, px);
        let w = lay.buttons[1].width;
        assert!(
            w > 80,
            "the button is wider than the template's 80 px, at {w}"
        );
        assert!(
            ink + 2 * BUTTON_LABEL_PAD <= w,
            "{ink} px of label fits {w} px"
        );
        assert!(
            lay.buttons.iter().all(|r| r.width == w),
            "one width for the row"
        );
        assert!(lay.buttons[0].x >= 0, "the row stays inside the dialog");
    }

    /// A grown Run-like template keeps its frame and caption and grows the client: its lines
    /// sit a grown pitch apart and wrap to the grown column, the field and buttons grow.
    #[test]
    fn a_grown_template_grows_the_client_inside_the_frame() {
        let (text, _) = rend();
        let f = Fields {
            text: "Type the name of a program, folder, document, or Internet resource, and Reprise will open it for you.",
            icon: true,
            field: true,
            buttons: &["OK".into(), "Cancel".into(), "&Browse...".into()],
            text_h: None,
        };
        let one = layout(Template::Run, &text, 11, &f, TextUnits::ONE);
        assert_eq!(one.clone().grown(TextUnits::ONE), one);
        let u = TextUnits::new(17, 11);
        let big = layout(Template::Run, &text, 17, &f, u).grown(u);
        assert_eq!(big.size.0, 2 * FRAME + u.len(347 - 2 * FRAME));
        let pitch = big.lines[1].0.y - big.lines[0].0.y;
        assert!(
            pitch >= u.len(LINE_H) - 1 && pitch <= u.len(LINE_H) + 1,
            "pitch {pitch}"
        );
        for (r, line) in &big.lines {
            assert!(text.measure(line, false, 17) <= r.width, "{line:?} fits");
        }
        let icon = big.icon.unwrap();
        assert_eq!((icon.width, icon.height), (ICON_PX, ICON_PX));
        let field_h = big.field.unwrap().height;
        assert!(
            (u.len(ROW_H) - 1..=u.len(ROW_H) + 1).contains(&field_h),
            "{field_h}"
        );
        let last = *big.buttons.last().unwrap();
        assert!(last.right() <= big.size.0 - FRAME && last.bottom() <= big.size.1 - FRAME);
    }

    /// A message box keeps its 32 px icon and the 16 px gap to its text at every text size, drawn
    /// by the compositor (whole-dialog layout) or by a program (the form); only the text column
    /// grows, and its lines still fit it.
    #[test]
    fn a_grown_message_box_keeps_its_icon_and_its_gap() {
        const EXPECTED_GAP: i32 = 16;
        let text = TextRenderer::load(&FontConfig::default());
        let f = Fields {
            text: "Are you sure you want to send 'a considerably longer file name.txt' to the Recycle Bin?",
            icon: true,
            field: false,
            buttons: &["&Yes".into(), "&No".into()],
            text_h: None,
        };
        for pct in [100, 150, 200] {
            let theme = Theme::default().with_form_text(pct);
            let (u, px) = (theme.text, theme.fonts.size_px);
            let lay = layout(Template::MessageBox, &text, px, &f, u);
            let whole = lay.clone().grown(u);
            let icon = whole.icon.unwrap();
            assert_eq!((icon.width, icon.height), (ICON_PX, ICON_PX), "{pct}%");
            for (r, line) in &whole.lines {
                assert_eq!(r.x - icon.right(), EXPECTED_GAP, "{pct}%: the gap");
                assert!(
                    text.measure(line, false, px) <= r.width,
                    "{pct}%: {line:?} fits"
                );
            }
            let form = form(&lay.client(), None, &["&Yes".into(), "&No".into()], 0);
            form.drawn_with(&theme.paint());
            let (icon, lines) = (form.rect("icon").unwrap(), form.rect("text").unwrap());
            assert_eq!((icon.width, icon.height), (ICON_PX, ICON_PX), "{pct}%");
            assert_eq!(
                lines.x - icon.right(),
                EXPECTED_GAP,
                "{pct}%: the form's gap"
            );
        }
    }
}
