//! File preview: one file shown in a window of its own, as its picture, its text or a card of its
//! facts, stepped through the folder with the arrows. The host reads the file; this side routes
//! it, sizes the window, draws, takes the keys and the pointer, and names it to a screen reader.

use crate::access::{self, Tree, accesskit};
use crate::controls::multiedit::{self, Metrics, MultiEdit, View};
use crate::controls::scrollbar::ThumbDrag;
use crate::paint::Paint;
use crate::raster::{Canvas, Rect, fit_size};
use crate::text::{Align, TextRenderer};
use crate::textedit::Caret;
use crate::textedit::keysym;
use accesskit::{Node, NodeId, Role};
use std::rc::Rc;

/// Bytes of a text file read for its preview.
pub const TEXT_BYTES: usize = 256 << 10;
/// Bytes sniffed to tell a file of unknown type for text.
pub const SNIFF_BYTES: usize = 8 << 10;
/// The margin round a picture and the card, and between the text box and the window's edge.
pub const PAD: i32 = 8;
/// The card's icon edge.
pub const ICON_PX: i32 = 48;
/// The card's client size.
pub const CARD: (i32, i32) = (360, ICON_PX + 2 * PAD + 24);
/// The smallest client a picture gets, so a 16 px icon file still has a window to grab.
pub const MIN_PICTURE: (i32, i32) = (200, 120);
/// What the text box is sized for before the work area caps it.
pub const TEXT_COLS: i32 = 80;
pub const TEXT_ROWS: i32 = 30;
/// Space from the icon to the card's lines, and their pitch.
const CARD_GAP: i32 = 12;
const CARD_LINE: i32 = 16;

/// How a file's preview is read: decoded as a picture, read as text, set as a font's sample, or
/// sniffed and then shown by its cached thumbnail or its card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Picture,
    Text,
    Font,
    Other,
}

/// The route for a file of MIME type `mime`; a type that is none of the known ones is sniffed.
pub fn route(mime: Option<&str>) -> Route {
    let Some(m) = mime else {
        return Route::Other;
    };
    let text_app = [
        "application/json",
        "application/xml",
        "application/javascript",
        "application/x-shellscript",
        "application/toml",
        "application/x-yaml",
        "application/yaml",
        "application/x-desktop",
        "application/x-perl",
        "application/x-ruby",
        "application/sql",
        "application/x-subrip",
    ];
    let font_app = [
        "application/x-font-ttf",
        "application/x-font-otf",
        "application/vnd.ms-opentype",
    ];
    if m == "image/svg+xml" {
        Route::Text
    } else if m.starts_with("font/") || font_app.contains(&m) {
        Route::Font
    } else if m.starts_with("image/") {
        Route::Picture
    } else if m.starts_with("text/") || text_app.contains(&m) || m.ends_with("+xml") {
        Route::Text
    } else {
        Route::Other
    }
}

/// Whether the first bytes of a file read as text: no NUL, and UTF-8 save for a character the
/// read cut in two at the end.
pub fn looks_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

/// `bytes` as the text box shows them: invalid UTF-8 replaced, a character cut at the end dropped.
pub fn text_of(bytes: &[u8]) -> String {
    let end = match std::str::from_utf8(bytes) {
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        _ => bytes.len(),
    };
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// What the card says under the name; `size` is `None` for a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub name: String,
    pub kind: String,
    pub size: Option<String>,
    pub modified: String,
}

impl Facts {
    fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("Type: {}", self.kind)];
        if let Some(s) = &self.size {
            out.push(format!("Size: {s}"));
        }
        if !self.modified.is_empty() {
            out.push(format!("Modified: {}", self.modified));
        }
        out
    }
}

pub enum Body {
    /// The facts beside the big icon: what shows until the read lands, and for what it can't show.
    Card,
    /// Fitted to the work area; its logical size is the picture's size in the window.
    Picture(Canvas),
    /// The file's first [`TEXT_BYTES`], read-only.
    Text(MultiEdit),
}

/// What a key in the window means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Close,
    /// Open the file in its program, then close.
    Open,
    /// Move the folder's selection this many items along the view's order, held to its ends; the
    /// preview follows.
    Step(i64),
    /// Text scrolled by the given pages, or to an end.
    Page(i64),
    None,
}

/// Space and Escape close, Enter opens, Left and Up step to the previous item in the view's order
/// and Right and Down to the next, Home and End to the first and last, Page Up and Down scroll
/// text, Ctrl+Home and Ctrl+End go to its ends.
pub fn key(sym: u32, ctrl: bool, alt: bool) -> Action {
    if alt {
        return Action::None;
    }
    match sym {
        keysym::SPACE | keysym::ESCAPE => Action::Close,
        keysym::RETURN | keysym::KP_ENTER => Action::Open,
        keysym::HOME | keysym::KP_HOME if ctrl => Action::Page(i64::MIN / 2),
        keysym::END | keysym::KP_END if ctrl => Action::Page(i64::MAX / 2),
        _ if ctrl => Action::None,
        keysym::LEFT | keysym::KP_LEFT | keysym::UP | keysym::KP_UP => Action::Step(-1),
        keysym::RIGHT | keysym::KP_RIGHT | keysym::DOWN | keysym::KP_DOWN => Action::Step(1),
        keysym::HOME | keysym::KP_HOME => Action::Step(i64::MIN / 2),
        keysym::END | keysym::KP_END => Action::Step(i64::MAX / 2),
        keysym::PAGE_UP | keysym::KP_PAGE_UP => Action::Page(-1),
        keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Action::Page(1),
        _ => Action::None,
    }
}

pub struct FilePreview {
    pub facts: Facts,
    /// The card's icon, [`ICON_PX`] square.
    pub icon: Option<Rc<Canvas>>,
    pub body: Body,
    pub view: View,
    thumb: ThumbDrag,
}

impl FilePreview {
    /// The card for `facts`, until the host's read lands a body.
    pub fn new(facts: Facts, icon: Option<Rc<Canvas>>) -> FilePreview {
        FilePreview {
            facts,
            icon,
            body: Body::Card,
            view: View::default(),
            thumb: ThumbDrag::default(),
        }
    }

    pub fn set_body(&mut self, body: Body) {
        self.body = body;
        self.view = View::default();
        self.thumb = ThumbDrag::default();
    }

    /// The client size the window opens at for its body, inside `within` (the work area's share).
    pub fn size(&self, mono: &TextRenderer, m: &Metrics, within: (i32, i32)) -> (i32, i32) {
        let want = match &self.body {
            Body::Card => CARD,
            Body::Picture(p) => (
                (p.width as i32 + 2 * PAD).max(MIN_PICTURE.0),
                (p.height as i32 + 2 * PAD).max(MIN_PICTURE.1),
            ),
            Body::Text(_) => {
                let col = mono.advance('0', false, m.font_px).max(1);
                // The edge (2 px a side), the text's 1 px inset, a bar each way and the margin.
                let w = TEXT_COLS * col + 6 + m.bar + 2 * PAD;
                let h = TEXT_ROWS * m.line_h + 6 + m.bar + 2 * PAD;
                (w, h)
            }
        };
        (
            want.0.min(within.0.max(CARD.0)),
            want.1.min(within.1.max(CARD.1)),
        )
    }

    /// The text box's layout in the client `r`; `None` without text.
    pub fn text_layout(
        &self,
        mono: &TextRenderer,
        m: &Metrics,
        r: Rect,
    ) -> Option<multiedit::Layout> {
        let Body::Text(edit) = &self.body else {
            return None;
        };
        Some(multiedit::layout(mono, m, edit, r.inset(PAD), 1))
    }

    /// Text scrolled `by` rows (the wheel) in the client `r`; true when it moved.
    pub fn scroll(&mut self, by: i64, mono: &TextRenderer, m: &Metrics, r: Rect) -> bool {
        let Some(lay) = self.text_layout(mono, m, r) else {
            return false;
        };
        self.view.scroll_lines(by, &lay)
    }

    /// Text scrolled `pages` pages, [`Action::Page`]'s; true when it moved.
    pub fn page(&mut self, pages: i64, mono: &TextRenderer, m: &Metrics, r: Rect) -> bool {
        let Some(lay) = self.text_layout(mono, m, r) else {
            return false;
        };
        let lines = lay.total as i64;
        let by = pages.saturating_mul(lay.rows as i64).clamp(-lines, lines);
        self.view.scroll_lines(by, &lay)
    }

    /// A left press at `at`: a scrollbar part scrolls or takes the thumb. True when it scrolled.
    pub fn press(&mut self, at: (i32, i32), mono: &TextRenderer, m: &Metrics, r: Rect) -> bool {
        let (Some(lay), Body::Text(edit)) = (self.text_layout(mono, m, r), &self.body) else {
            return false;
        };
        let Some((bar, hit)) = multiedit::bar_hit(mono, m, edit, &lay, self.view, at.0, at.1)
        else {
            return false;
        };
        self.thumb.press(bar.axis(), hit);
        let to = self.view.press_bar(mono, m, edit, &lay, bar, hit);
        std::mem::replace(&mut self.view, to) != to
    }

    /// The pointer at `at` while the left button is held: a held thumb drags. True when it moved.
    pub fn motion(&mut self, at: (i32, i32), mono: &TextRenderer, m: &Metrics, r: Rect) -> bool {
        if !self.thumb.holding() {
            return false;
        }
        let (Some(lay), Body::Text(edit)) = (self.text_layout(mono, m, r), &self.body) else {
            return false;
        };
        match multiedit::drag_bar(mono, m, edit, &lay, self.view, &self.thumb, at) {
            Some(v) if v != self.view => {
                self.view = v;
                true
            }
            _ => false,
        }
    }

    pub fn release(&mut self) {
        self.thumb.release();
    }
}

/// The card's icon and its lines (the name first) in the client `r`.
pub fn card_layout(r: Rect, lines: usize) -> (Rect, Vec<Rect>) {
    let icon = Rect::new(r.x + PAD + 4, r.y + PAD + 4, ICON_PX, ICON_PX);
    let x = icon.right() + CARD_GAP;
    let w = (r.right() - PAD - x).max(0);
    let rows = (0..lines)
        .map(|i| Rect::new(x, icon.y + i as i32 * CARD_LINE, w, CARD_LINE))
        .collect();
    (icon, rows)
}

/// Where the picture goes in the client `r`: centred, shrunk to fit a window the user made smaller.
pub fn picture_rect(picture: &Canvas, r: Rect) -> Rect {
    let room = r.inset(PAD);
    let (w, h) = fit_size(
        picture.width,
        picture.height,
        (room.width.max(1) as u32, room.height.max(1) as u32),
    );
    let (w, h) = (w as i32, h as i32);
    Rect::new(
        room.x + (room.width - w) / 2,
        room.y + (room.height - h) / 2,
        w,
        h,
    )
}

/// Draw the window's client `r`: the face, then the picture, the text box or the card, its lines
/// in `text` at `px`.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    px: u32,
    mono: &TextRenderer,
    m: &Metrics,
    qv: &FilePreview,
    r: Rect,
) {
    let cs = p.cs;
    c.fill_rect(r, cs.button_face);
    match &qv.body {
        Body::Picture(pic) => {
            let at = picture_rect(pic, r);
            if (at.width, at.height) == (pic.width as i32, pic.height as i32) {
                c.blit_over(at.x, at.y, pic);
            } else {
                c.blit_over(
                    at.x,
                    at.y,
                    &pic.resample_box(at.width as u32, at.height as u32),
                );
            }
        }
        Body::Text(edit) => {
            let lay = multiedit::layout(mono, m, edit, r.inset(PAD), 1);
            let paint = Paint { s: 1, ..*p };
            multiedit::draw(
                c,
                mono,
                &paint,
                m,
                edit,
                qv.view,
                &lay,
                Caret::Unfocused,
                None,
            );
        }
        Body::Card => {
            let lines = qv.facts.lines();
            let (icon, rows) = card_layout(r, lines.len() + 1);
            if let Some(i) = &qv.icon {
                c.blit_over(icon.x, icon.y, i);
            }
            let name = text.ellipsize(&qv.facts.name, rows[0].width, true, px);
            text.draw(c, rows[0], &name, true, cs.window_text, Align::Left, px);
            for (line, row) in lines.iter().zip(&rows[1..]) {
                let shown = text.ellipsize(line, row.width, false, px);
                text.draw(c, *row, &shown, false, cs.window_text, Align::Left, px);
            }
        }
    }
}

/// The window's nodes under `parent`: the picture as an image, the text box, or the card's lines.
pub fn access(
    tree: &mut Tree,
    parent: NodeId,
    qv: &FilePreview,
    mono: &TextRenderer,
    m: &Metrics,
    r: Rect,
) {
    match &qv.body {
        Body::Picture(pic) => {
            let at = picture_rect(pic, r);
            tree.push(
                parent,
                "picture",
                access::titled(Role::Image, at, &qv.facts.name),
            );
        }
        Body::Text(edit) => {
            let lay = multiedit::layout(mono, m, edit, r.inset(PAD), 1);
            let id = multiedit::access(edit, qv.view, &lay, m, mono, tree, parent, "text");
            tree.set_focus(id);
        }
        Body::Card => {
            let lines = qv.facts.lines();
            let (icon, rows) = card_layout(r, lines.len() + 1);
            tree.push(
                parent,
                "icon",
                access::titled(Role::Image, icon, &qv.facts.kind),
            );
            tree.push(parent, "name", value(rows[0], &qv.facts.name));
            for (i, (line, row)) in lines.iter().zip(&rows[1..]).enumerate() {
                tree.push(parent, ("line", i), value(*row, line));
            }
        }
    }
}

/// A label reading `text` as it is: a file name's `&` is no access key.
fn value(r: Rect, text: &str) -> Node {
    let mut n = access::node(Role::Label, r);
    n.set_value(text);
    n
}

#[cfg(test)]
#[path = "filepreview_tests.rs"]
mod tests;
