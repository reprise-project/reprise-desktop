//! Page Setup: the paper a program prints on, its orientation and margins, a text's header and
//! footer, the page's geometry at a printer's resolution, and the dialog that picks them with a
//! preview of the page.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{self, Control, Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::{DialogFont, TextRenderer};
use crate::theme::Color;
use accesskit::{NodeId, Role};
use serde::{Deserialize, Serialize};

/// A paper size: its PWG media name and its portrait width and height in hundredths of a
/// millimetre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paper {
    pub media: &'static str,
    pub size: (i32, i32),
}

impl Paper {
    /// What the Size box calls it.
    pub fn label(&self) -> String {
        match PAPERS.iter().position(|p| p == self) {
            Some(0) => tr!("paper-letter"),
            Some(1) => tr!("paper-legal"),
            Some(2) => tr!("paper-executive"),
            Some(3) => tr!("paper-a3"),
            Some(4) => tr!("paper-a4"),
            Some(5) => tr!("paper-a5"),
            Some(6) => tr!("paper-envelope-10"),
            _ => tr!("paper-envelope-dl"),
        }
    }
}

pub const PAPERS: [Paper; 8] = [
    paper("na_letter_8.5x11in", (21590, 27940)),
    paper("na_legal_8.5x14in", (21590, 35560)),
    paper("na_executive_7.25x10.5in", (18415, 26670)),
    paper("iso_a3_297x420mm", (29700, 42000)),
    paper("iso_a4_210x297mm", (21000, 29700)),
    paper("iso_a5_148x210mm", (14800, 21000)),
    paper("na_number-10_4.125x9.5in", (10478, 24130)),
    paper("iso_dl_110x220mm", (11000, 22000)),
];

const fn paper(media: &'static str, size: (i32, i32)) -> Paper {
    Paper { media, size }
}

const LETTER: usize = 0;
const A4: usize = 4;

/// Territories that print on Letter; the rest of the world prints on A4.
const LETTER_TERRITORIES: [&str; 12] = [
    "US", "CA", "MX", "PR", "PH", "CO", "VE", "CL", "CR", "GT", "PA", "SV",
];

/// The paper a locale (`en_US.UTF-8`, as LC_PAPER or LANG hold it) prints on.
pub fn paper_for_locale(locale: &str) -> &'static Paper {
    let territory = locale
        .split(['.', '@'])
        .next()
        .and_then(|l| l.split_once('_'))
        .map(|(_, t)| t);
    match territory {
        Some(t) if LETTER_TERRITORIES.contains(&t) => &PAPERS[LETTER],
        _ => &PAPERS[A4],
    }
}

/// One millimetre in the hundredths a [`PageSetup`] measures in.
pub const MM: i32 = 100;
/// Hundredths of a millimetre in an inch.
const INCH: i32 = 2540;
/// The least room the margins leave for the text or picture, each way.
pub const MIN_BODY: i32 = 25 * MM;
/// The widest margin a box takes, a metre: wider than any paper, and far from overflowing.
pub const MAX_MARGIN: i32 = 1000 * MM;
const DEFAULT_MARGINS: [i32; 4] = [20 * MM, 20 * MM, 25 * MM, 25 * MM];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageSetup {
    /// The paper's PWG media name; one not in [`PAPERS`] reads as A4.
    pub paper: String,
    pub landscape: bool,
    /// Left, right, top and bottom, in hundredths of a millimetre.
    pub margins: [i32; 4],
    /// A text's header and footer, with [`printpage::band`]'s codes; empty prints none.
    ///
    /// [`printpage::band`]: crate::controls::printpage::band
    pub header: String,
    pub footer: String,
}

impl Default for PageSetup {
    fn default() -> PageSetup {
        PageSetup::on(&PAPERS[A4])
    }
}

impl PageSetup {
    pub fn on(paper: &Paper) -> PageSetup {
        PageSetup {
            paper: paper.media.into(),
            landscape: false,
            margins: DEFAULT_MARGINS,
            header: String::new(),
            footer: String::new(),
        }
    }

    pub fn paper(&self) -> &'static Paper {
        &PAPERS[self.paper_at()]
    }

    fn paper_at(&self) -> usize {
        PAPERS
            .iter()
            .position(|p| p.media == self.paper)
            .unwrap_or(A4)
    }

    /// The page's width and height as it lies, hundredths of a millimetre.
    pub fn size(&self) -> (i32, i32) {
        let (w, h) = self.paper().size;
        if self.landscape { (h, w) } else { (w, h) }
    }

    /// The part of the page inside the margins, hundredths of a millimetre from its top left.
    pub fn body(&self) -> Rect {
        let (w, h) = self.size();
        let [l, r, t, b] = self.margins;
        Rect::new(
            l,
            t,
            w.saturating_sub(l).saturating_sub(r).max(0),
            h.saturating_sub(t).saturating_sub(b).max(0),
        )
    }

    /// Each margin is within `0..=MAX_MARGIN` and together they leave at least [`MIN_BODY`]
    /// each way.
    pub fn fits(&self) -> bool {
        self.crowded().is_none()
    }

    /// The box of a margin out of range, else of the larger margin on a side that leaves no
    /// room; `None` when [`PageSetup::fits`].
    fn crowded(&self) -> Option<&'static str> {
        if let Some(i) = (0..4).find(|&i| !(0..=MAX_MARGIN).contains(&self.margins[i])) {
            return Some(MARGIN_IDS[i]);
        }
        let b = self.body();
        let [l, r, t, bottom] = self.margins;
        if b.width < MIN_BODY {
            return Some(if r > l { RIGHT } else { LEFT });
        }
        (b.height < MIN_BODY).then_some(if bottom > t { BOTTOM } else { TOP })
    }
}

/// `v` hundredths of a millimetre in device pixels at `dpi`.
pub fn px(v: i32, dpi: i32) -> i32 {
    ((v as i64 * dpi as i64 + INCH as i64 / 2) / INCH as i64) as i32
}

/// `v` hundredths of a millimetre in points (1/72 inch).
pub fn points(v: i32) -> f64 {
    v as f64 * 72.0 / INCH as f64
}

pub const PAPER: &str = "paper";
pub const PORTRAIT: &str = "portrait";
pub const LANDSCAPE: &str = "landscape";
pub const LEFT: &str = "left";
pub const RIGHT: &str = "right";
pub const TOP: &str = "top";
pub const BOTTOM: &str = "bottom";
pub const HEADER: &str = "header";
pub const FOOTER: &str = "footer";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const PREVIEW: &str = "preview";
const MARGIN_IDS: [&str; 4] = [LEFT, RIGHT, TOP, BOTTOM];

pub const CLIENT: (i32, i32) = (348, 312);
/// The rows Header and Footer add below the margins.
const HEADERS_H: i32 = 52;
const PAD: i32 = 8;
const WIDE: i32 = CLIENT.0 - 2 * PAD;
const PREVIEW_RECT: Rect = Rect::new(PAD, PAD, WIDE, 124);
const PAPER_GROUP: Rect = Rect::new(PAD, 140, WIDE, 50);
const ORIENT_GROUP: Rect = Rect::new(PAD, 198, 108, 70);
const MARGIN_GROUP: Rect = Rect::new(124, 198, CLIENT.0 - 124 - PAD, 70);
const EDIT_W: i32 = 48;
const EDIT_H: i32 = 20;
const PAPER_ROWS: usize = 8;
const BUTTONS_Y: i32 = 280;
const HEADER_Y: i32 = 276;
const ROW_GAP: i32 = 26;

/// The dialog's client size, with the Header and Footer rows when `headers`.
pub fn client(headers: bool) -> (i32, i32) {
    (CLIENT.0, CLIENT.1 + if headers { HEADERS_H } else { 0 })
}

/// The dialog's form for `s`: Size, Orientation and the four margins in millimetres, and with
/// `headers` the Header and Footer boxes.
pub fn form(s: &PageSetup, headers: bool) -> Form {
    let labels: Vec<String> = PAPERS.iter().map(Paper::label).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut paper = Item::combo(PAPER, 76, 158, 252, &labels, PAPER_ROWS);
    if let Control::Combo(c) = &mut paper.control {
        c.selected = s.paper_at();
    }
    let margin = |i: usize, x: i32, y: i32| {
        Item::edit(
            MARGIN_IDS[i],
            Rect::new(x, y, EDIT_W, EDIT_H),
            &mm_text(s.margins[i]),
            true,
        )
    };
    let label = |id, x, y, text: String| Item::label_at(id, x, y + 4, 50, &text);
    let (mx, my) = (MARGIN_GROUP.x, MARGIN_GROUP.y);
    let buttons_y = BUTTONS_Y + client(headers).1 - CLIENT.1;
    let mut items = vec![
        Item::new(PREVIEW, PREVIEW_RECT, Control::Blank),
        Item::group("papergroup", PAPER_GROUP, &tr!("pagesetup-paper")),
        Item::label_at("paperlabel", 20, 162, 54, &tr!("pagesetup-size")),
        paper,
        Item::group("orientgroup", ORIENT_GROUP, &tr!("pagesetup-orientation")),
        Item::radio(PORTRAIT, 20, 220, &tr!("pagesetup-portrait"), "orient").checked(!s.landscape),
        Item::radio(LANDSCAPE, 20, 244, &tr!("pagesetup-landscape"), "orient").checked(s.landscape),
        Item::group("margingroup", MARGIN_GROUP, &tr!("pagesetup-margins")),
        label("leftlabel", mx + 10, my + 18, tr!("pagesetup-left")),
        margin(0, mx + 58, my + 18),
        label("rightlabel", mx + 112, my + 18, tr!("pagesetup-right")),
        margin(1, mx + 160, my + 18),
        label("toplabel", mx + 10, my + 42, tr!("pagesetup-top")),
        margin(2, mx + 58, my + 42),
        label("bottomlabel", mx + 112, my + 42, tr!("pagesetup-bottom")),
        margin(3, mx + 160, my + 42),
    ];
    if headers {
        for (i, (label, id, text, value)) in [
            ("headerlabel", HEADER, tr!("pagesetup-header"), &s.header),
            ("footerlabel", FOOTER, tr!("pagesetup-footer"), &s.footer),
        ]
        .into_iter()
        .enumerate()
        {
            let y = HEADER_Y + i as i32 * ROW_GAP;
            items.push(Item::label_at(label, 20, y + 4, 54, &text));
            items.push(Item::edit(id, Rect::new(76, y, 252, EDIT_H), value, true));
        }
    }
    items.extend([
        Item::button(
            OK,
            Rect::new(
                CLIENT.0 - PAD - 2 * BUTTON_W - 6,
                buttons_y,
                BUTTON_W,
                BUTTON_H,
            ),
            &tr!("ok"),
            true,
            true,
        ),
        Item::button(
            CANCEL,
            Rect::new(CLIENT.0 - PAD - BUTTON_W, buttons_y, BUTTON_W, BUTTON_H),
            &tr!("cancel"),
            true,
            false,
        ),
    ]);
    let mut f = Form::new(items)
        .set_in(DialogFont::ShellDlg)
        .labels_at_ink();
    f.focus_id(PAPER);
    f
}

/// Millimetres as the margin boxes show them: whole ones bare, else to a hundredth.
fn mm_text(v: i32) -> String {
    if v < 0 {
        return format!("-{}", mm_text(v.saturating_neg()));
    }
    let sep = tr!("number-decimal-separator");
    match v % MM {
        0 => (v / MM).to_string(),
        r => format!("{}{sep}{:02}", v / MM, r.abs())
            .trim_end_matches('0')
            .to_string(),
    }
}

/// A margin box's text in hundredths of a millimetre: a number of millimetres with up to two
/// decimals, after a point or a comma, up to [`MAX_MARGIN`]. Anything else, or a negative, is
/// `None`.
pub fn parse_mm(s: &str) -> Option<i32> {
    let s = s.trim().replace(',', ".");
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() && frac.is_empty() || !digits(whole) || !digits(frac) || frac.len() > 2 {
        return None;
    }
    let whole: i32 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac: i32 = format!("{frac:0<2}").parse().ok()?;
    let v = whole.checked_mul(MM)?.checked_add(frac)?;
    (v <= MAX_MARGIN).then_some(v)
}

/// Why OK left the dialog open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The margin box with this id holds no number of millimetres.
    Number(&'static str),
    /// The margins leave less than [`MIN_BODY`] for the page's text or picture; the larger
    /// margin of the side that ran out.
    NoRoom(&'static str),
}

impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Refusal::Number(_) => tr!("pagesetup-not-a-number"),
            Refusal::NoRoom(_) => tr!("pagesetup-no-room", mm = MIN_BODY / MM),
        }
    }

    /// The box the user is sent back to.
    pub fn field(&self) -> &'static str {
        match self {
            Refusal::Number(id) | Refusal::NoRoom(id) => id,
        }
    }
}

/// What the form says, refused when a margin is not a number or the margins leave no room.
pub fn read(f: &Form) -> Result<PageSetup, Refusal> {
    let mut margins = [0; 4];
    for (m, id) in margins.iter_mut().zip(MARGIN_IDS) {
        *m = parse_mm(f.edit_text(id)).ok_or(Refusal::Number(id))?;
    }
    let s = PageSetup {
        margins,
        ..shown(f)
    };
    match s.crowded() {
        Some(id) => Err(Refusal::NoRoom(id)),
        None => Ok(s),
    }
}

/// What the preview draws: the paper and orientation picked, and each margin whose box holds a
/// number (else none). The header and footer as typed; empty without their boxes.
fn shown(f: &Form) -> PageSetup {
    let paper = f.combo_selected(PAPER).unwrap_or(A4).min(PAPERS.len() - 1);
    PageSetup {
        paper: PAPERS[paper].media.into(),
        landscape: f.checked(LANDSCAPE).unwrap_or(false),
        margins: MARGIN_IDS.map(|id| parse_mm(f.edit_text(id)).unwrap_or(0)),
        header: f.edit_text(HEADER).into(),
        footer: f.edit_text(FOOTER).into(),
    }
}

/// Send the user back to the box `r` names with its text selected.
pub fn refused(f: &mut Form, r: Refusal) {
    f.focus_id(r.field());
    f.select_edit_text(r.field());
}

/// The form, then the preview: the page as it lies, its shadow, and grey lines filling the part
/// inside the margins.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, f: &Form, px: u32) {
    form::draw(c, text, p, f, true, px);
    if let Some(r) = f.rect(PREVIEW) {
        draw_preview(c, p, &shown(f), r);
    }
}

const SHADOW: i32 = 3;
const PREVIEW_INSET: i32 = 6;
const LINE_GAP: i32 = 3;
/// The sheet is paper in every scheme, its text lines grey ink.
const SHEET: Color = Color::rgb(255, 255, 255);
const INK: Color = Color::rgb(160, 160, 160);

/// `s`'s page drawn as large as fits in `r` (logical px), scaled by `p.s`.
pub fn draw_preview(c: &mut Canvas, p: &Paint, s: &PageSetup, r: Rect) {
    let (w, h) = s.size();
    let room = (
        r.width - 2 * PREVIEW_INSET - SHADOW,
        r.height - 2 * PREVIEW_INSET - SHADOW,
    );
    let (sw, sh) = if room.0 as i64 * h as i64 > room.1 as i64 * w as i64 {
        ((room.1 as i64 * w as i64 / h as i64) as i32, room.1)
    } else {
        (room.0, (room.0 as i64 * h as i64 / w as i64) as i32)
    };
    let sheet = Rect::new(
        r.x + (r.width - SHADOW - sw) / 2,
        r.y + (r.height - SHADOW - sh) / 2,
        sw,
        sh,
    );
    let at = |v: Rect| v.scaled(p.s);
    let shadow = Rect::new(sheet.x + SHADOW, sheet.y + SHADOW, sw, sh);
    c.fill_rect(at(shadow), p.cs.button_dk_shadow);
    c.fill_rect(at(sheet), p.cs.window_frame);
    let paper = Rect::new(sheet.x + 1, sheet.y + 1, sw - 2, sh - 2);
    c.fill_rect(at(paper), SHEET);
    let scale = |v: i32, of: i32, len: i32| (v as i64 * len as i64 / of as i64) as i32;
    let b = s.body();
    let body = Rect::new(
        sheet.x + scale(b.x, w, sw).max(1),
        sheet.y + scale(b.y, h, sh).max(1),
        scale(b.width, w, sw),
        scale(b.height, h, sh),
    )
    .intersect(&paper);
    let mut y = body.y;
    while y < body.bottom() {
        c.fill_rect(at(Rect::new(body.x, y, body.width, 1)), INK);
        y += LINE_GAP;
    }
}

/// The form's nodes and the preview as an image; the focused node, as [`Form::access`] gives it.
pub fn access(
    f: &Form,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
) -> Option<NodeId> {
    let focus = f.access(tree, parent, text, px);
    if let Some(r) = f.rect(PREVIEW) {
        let name = tr!("pagesetup-preview");
        tree.push(parent, PREVIEW, access::titled(Role::Image, r, &name));
    }
    focus
}

#[cfg(test)]
mod tests;
