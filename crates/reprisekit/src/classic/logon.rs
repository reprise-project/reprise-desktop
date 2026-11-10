//! Classic's login screen: the "Log On to Reprise" dialog centred on the desktop colour, with the
//! Reprise band, User name and Password rows, a two-line message area and three buttons. The lock
//! fixes the name and puts Switch User at the start of the button row.

use crate::art::logo;
use crate::classic::indicator;
use crate::classic::widgets::{self, BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::icon::ICON_SIZE;
use crate::logon::{Logon, LogonLayout, Part, message_lines, text as strings};
use crate::paint::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit, edit_first_visible};
use crate::theme::Theme;

/// The client area under the caption, design px at 100% text: padding, label column, rows and
/// buttons.
const CLIENT_W: i32 = 352;
const PAD: i32 = 12;
/// The band sits one face row under the caption, with an etched rule under it.
const BAND_Y: i32 = 1;
const BAND_H: i32 = 64;
const RULE: i32 = 2;
const LOGO: i32 = 40;
const MARK_GAP: i32 = 10;
const LABEL_W: i32 = 90;
const ROW_H: i32 = 20;
const ROW_GAP: i32 = 8;
const MESSAGE_LINES: usize = 2;
const BADGE_GAP: i32 = 6;
/// The band's wordmark: the product's name, never translated.
const MARK: &str = "Reprise";

/// Every rect of the dialog, design px in the output's space.
struct Geometry {
    dialog: Rect,
    caption: Rect,
    band: Rect,
    label_x: i32,
    name: Rect,
    password: Rect,
    message: Rect,
    ok: Rect,
    cancel: Rect,
    power: Rect,
    switch: Rect,
    badge: Rect,
}

/// The frame and caption come from `theme`'s metrics; the client grows with its text size.
fn geometry(theme: &Theme, output: Rect) -> Geometry {
    let u = theme.text;
    let s = theme.scale.max(1);
    let frame = theme.metrics.frame_width / s;
    let caption_h = theme.metrics.caption_height / s;
    let name_y = BAND_Y + BAND_H + RULE + PAD;
    let password_y = name_y + ROW_H + ROW_GAP;
    let message_y = password_y + ROW_H + ROW_GAP;
    let buttons_y = message_y + MESSAGE_LINES as i32 * LINE_H + ROW_GAP;
    let (cw, ch) = (u.len(CLIENT_W), u.len(buttons_y + BUTTON_H + PAD));
    let (w, h) = (cw + 2 * frame, ch + 2 * frame + caption_h);
    let dialog = Rect::new(
        output.x + (output.width - w) / 2,
        output.y + (output.height - h) / 2,
        w,
        h,
    );
    let caption = Rect::new(dialog.x + frame, dialog.y + frame, cw, caption_h);
    let client = Rect::new(caption.x, caption.bottom(), cw, ch);
    let y = |v: i32| client.y + u.len(v);
    let span = |top: i32, h: i32| (y(top), y(top + h) - y(top));
    let field_x = client.x + u.len(PAD + LABEL_W);
    let row = |top: i32| {
        let (y, h) = span(top, ROW_H);
        Rect::new(field_x, y, client.right() - u.len(PAD) - field_x, h)
    };
    let (my, mh) = span(message_y, MESSAGE_LINES as i32 * LINE_H);
    let message = Rect::new(client.x + u.len(PAD), my, cw - 2 * u.len(PAD), mh);
    let (bw, bh) = (u.len(BUTTON_W), u.len(BUTTON_H));
    let button = |right: i32| Rect::new(right - bw, y(buttons_y), bw, bh);
    let power = button(client.right() - u.len(PAD));
    let cancel = button(power.x - u.len(BUTTON_GAP));
    let ok = button(cancel.x - u.len(BUTTON_GAP));
    Geometry {
        dialog,
        caption,
        band: Rect::new(client.x, client.y + BAND_Y, cw, u.len(BAND_H)),
        label_x: client.x + u.len(PAD),
        name: row(name_y),
        password: row(password_y),
        message,
        ok,
        cancel,
        power,
        switch: Rect::new(client.x + u.len(PAD), y(buttons_y), bw, bh),
        badge: Rect::new(
            power.right() - ICON_SIZE,
            message.bottom() - ICON_SIZE,
            ICON_SIZE,
            ICON_SIZE,
        ),
    }
}

pub fn layout(m: &Logon, theme: &Theme, _text: &TextRenderer, output: Rect) -> LogonLayout {
    let g = geometry(theme, output);
    let mut parts = vec![
        (Part::Name, g.name),
        (Part::Password, g.password),
        (Part::Submit, g.ok),
        (Part::Cancel, g.cancel),
        (Part::Power, g.power),
    ];
    if m.unlocking() && m.offers_switch_user() {
        parts.push((Part::SwitchUser, g.switch));
    }
    if m.layout.is_some() {
        parts.push((Part::Layout, g.badge));
    }
    LogonLayout {
        output,
        parts,
        dialog: Some(g.dialog),
    }
}

/// `c` covers `l.output` at scale `s`; `theme` is the output's (`Theme::scaled(s)`) and `text`
/// its renderer.
pub fn draw(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    let g = geometry(theme, l.output);
    let o = l.output;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
    let px = theme.fonts.size_px * s as u32 / theme.scale.max(1) as u32;
    let p = Paint { s, ..theme.paint() };
    let cs = &theme.colors;
    if !m.over_picture {
        c.fill_rect(at(o), cs.background);
    }

    let dialog = at(g.dialog);
    c.fill_rect(dialog, cs.button_face);
    c.edge_t(dialog, EdgeStyle::WindowRaised, cs, s);
    let caption = at(g.caption);
    c.fill_rect(caption, cs.gradient_active_title);
    let ramp = widgets::caption_ramp(caption.width, 0, s);
    c.hgradient_stepped(
        Rect {
            width: ramp,
            ..caption
        },
        cs.active_title,
        cs.gradient_active_title,
    );
    let title = Rect::new(
        caption.x + 4 * s,
        caption.y,
        caption.width - 8 * s,
        caption.height,
    );
    text.draw(c, title, &m.title(), true, cs.title_text, Align::Left, px);

    let band = at(g.band);
    c.hgradient_stepped(band, cs.active_title, cs.gradient_active_title);
    let rule = Rect::new(band.x, band.bottom(), band.width, s);
    c.fill_rect(rule, cs.button_shadow);
    c.fill_rect(
        Rect {
            y: rule.bottom(),
            ..rule
        },
        cs.button_hilight,
    );
    let pad = theme.text.len(PAD) * s;
    let mark_x = band.x + pad;
    let mark_y = band.y + (band.height - LOGO * s) / 2;
    logo::draw_any(c, mark_x, mark_y, LOGO * s, cs.title_text);
    let word_x = mark_x + (LOGO + MARK_GAP) * s;
    let word = Rect::new(word_x, band.y, band.right() - pad - word_x, band.height);
    text.draw(c, word, MARK, true, cs.title_text, Align::Left, px * 2);

    let label_w = theme.text.len(LABEL_W) * s;
    let label_at = |row: Rect| Rect::new((g.label_x - o.x) * s, row.y, label_w, row.height);
    let label = |c: &mut Canvas, row: Rect, name: &str| {
        let name = text.ellipsize(name, label_w, false, px);
        let name = p.label(&name);
        let r = label_at(row);
        widgets::draw_label(c, text, r, &name, false, cs.button_text, Align::Left, px, s);
    };
    let caret = |part: Part| Caret::new(m.focus == part && !m.busy, true);
    let name = at(g.name);
    label(c, name, &strings::user_name());
    field(c, text, &p, name, &m.name, caret(Part::Name), px);
    let password = at(g.password);
    match &m.prompt {
        // The conversation's text has no access keys: an `&` in it is drawn as is.
        Some(q) => {
            let asked = text.ellipsize(&q.text, label_w, false, px);
            let r = label_at(password);
            text.draw(c, r, &asked, false, cs.button_text, Align::Left, px);
        }
        None => label(c, password, &strings::password()),
    }
    let shown = if m.password.is_secret() {
        m.password.masked()
    } else {
        m.password.clone()
    };
    field(c, text, &p, password, &shown, caret(Part::Password), px);

    let message = at(g.message);
    let badge = at(g.badge);
    let width = match m.layout {
        Some(_) => badge.x - BADGE_GAP * s - message.x,
        None => message.width,
    };
    let mut lines = Vec::new();
    if m.caps_lock {
        lines.push(
            text.ellipsize(&strings::caps_lock(), width, false, px)
                .into_owned(),
        );
    }
    if let Some(msg) = m.said() {
        let room = MESSAGE_LINES - lines.len();
        lines.extend(message_lines(text, &msg, width, px, room));
    }
    let line_h = message.height / MESSAGE_LINES as i32;
    for (i, line) in lines.iter().enumerate() {
        let r = Rect::new(message.x, message.y + i as i32 * line_h, width, line_h);
        text.draw(c, r, line, false, cs.button_text, Align::Left, px);
    }
    if let Some(short) = &m.layout {
        let b = indicator::draw_layout(text, cs, short, badge.width, px);
        c.blit_canvas(badge.x, badge.y, &b);
    }

    let default = match m.focus {
        Part::Submit | Part::Cancel | Part::Power | Part::SwitchUser => m.focus,
        _ => Part::Submit,
    };
    let switch = (l.rect(Part::SwitchUser)).map(|r| (Part::SwitchUser, r, strings::switch_user()));
    for (part, r, caption) in [
        (Part::Submit, g.ok, strings::ok()),
        (Part::Cancel, g.cancel, strings::cancel()),
        (Part::Power, g.power, strings::shut_down()),
    ]
    .into_iter()
    .chain(switch)
    {
        let enabled = !m.busy || part == Part::Power;
        widgets::draw_button(
            c,
            text,
            &p,
            at(r),
            &p.label(&caption),
            enabled && part == default,
            m.lit == Some((part, Interaction::Pressed)),
            enabled,
            m.focus == part,
            px,
        );
    }
}

/// A sunken field showing `edit`; a read-only one keeps the face colour.
fn field(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    edit: &TextEdit,
    caret: Caret,
    px: u32,
) {
    let inner = widgets::draw_sunken_field(c, p, r, p.cs.window);
    if edit.is_read_only() {
        c.fill_rect(r.inset(2 * p.s), p.cs.button_face);
    }
    let first = edit_first_visible(text, edit, inner.width, px, 0);
    widgets::draw_edit(c, text, edit, inner, px, p, caret, first);
}

#[cfg(test)]
mod tests;
