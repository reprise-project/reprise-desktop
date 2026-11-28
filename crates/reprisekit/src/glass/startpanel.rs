//! Reprise Glass's Start panel: [`parts`] makes the `Part::Start*` parts (the panel's glass here,
//! the rest in [`super::startparts`], Onyx's own in [`super::onyxstart`]); [`draw`] lays them out
//! per `crate::startpanel::layout`.

use super::glyph;
use super::panel::{Corners, Panel};
use super::tint::{Recolour, Surface};
use super::{Settings, caption_text};
use crate::raster::{Canvas, Insets, Rect, over_round};
use crate::startpanel::layout::{self, Layout};
use crate::startpanel::search::{self, PaneInks, ink};
pub use crate::startpanel::search::{
    HEADER_PX, INK_CUE, INK_GREYED, INK_GROUP, INK_HEADER, INK_JUMP, INK_PROGRAM, INK_STRONG,
    TEXT_PX,
};
use crate::startpanel::{
    Mode, PROGRAM_ICON, SMALL_ICON, Shape, Spot, StartPanel, TreeRow, all_programs, back,
    no_results,
};
use crate::style::{Layout as Stack, Part, PartImage, State, Style};
use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig};
use std::sync::Arc;

pub const PANEL_CORNER: &[i32] = &[5, 3, 2, 1, 1];

pub fn font() -> FontConfig {
    caption_text([Color::rgb(0, 0, 0); 2]).font()
}

pub const INK_RIGHT: u32 = 0xFFFFFF;

/// Dark Glass's cue and greyed text on the dark pane: dimmer than its text, still 4.7:1.
const DARK_GREYED: Color = Color::rgb(0xA0, 0xA0, 0xA0);

/// The left pane's and the launcher list's inks in `mode`. Dark Glass writes its text, programs and
/// jump rows in [`super::DARK_TEXT`], greys in [`DARK_GREYED`], and remaps the blue titles.
pub fn pane_inks(mode: super::Mode) -> PaneInks {
    match mode {
        super::Mode::Light => PaneInks::LIGHT,
        super::Mode::Dark => PaneInks {
            strong: super::DARK_TEXT,
            program: super::DARK_TEXT,
            jump: super::DARK_TEXT,
            cue: DARK_GREYED,
            greyed: DARK_GREYED,
            ..PaneInks::LIGHT.map(super::dark::remap_color)
        },
    }
}

pub const SHEEN_ROWS: i32 = 48;

const PANEL_EDGE: i32 = PANEL_CORNER[0] + 1;

/// The share of what the frame's plain glass lets through that the
/// panel's second layer of glass covers, in black: in Sky at the default
/// intensity the 62 of the plain glass becomes 160, and however dense the
/// plain glass, the panel stays translucent.
pub const DENSITY: f64 = 0.51;

pub(crate) const PANEL: Panel = Panel {
    corners: Corners::Cut {
        rows: PANEL_CORNER,
        bottom: false,
    },
    lines: (1.0, 1.0),
    sheen_rows: SHEEN_ROWS,
    ends: None,
    deepen: 0.0,
    density: DENSITY,
    shadow: None,
};

pub fn contrast(a: Color, b: Color) -> f64 {
    a.contrast(b)
}

/// The least contrast the right column's ink keeps against the glass: 3:1,
/// the least that large text and user-interface parts take.
pub const INK_CONTRAST: f64 = 3.0;

fn glass_px(s: &Settings) -> u32 {
    Recolour::new(s, Surface::Active).finish(PANEL.glass(s, 0.0))
}

fn shown(px: u32) -> Color {
    let a = (px >> 24).max(1);
    let ch = |sh: u32| (((px >> sh) & 0xFF) * 255 / a).min(255) as u8;
    Color::rgb(ch(16), ch(8), ch(0))
}

pub fn glass_colour(s: &Settings) -> Color {
    shown(glass_px(s))
}

pub fn lit_colours(s: &Settings) -> Vec<Color> {
    let glass = glass_px(s);
    let lit = super::startparts::hot_right();
    let w = lit.width as i32;
    let rows = at::PLACE_BASE - 9..at::PLACE_BASE;
    rows.flat_map(|y| (at::PLACE_PEN..w - at::PLACE_PEN).map(move |x| (x, y)))
        .map(|(x, y)| shown(over_round(lit.get(x, y), glass)))
        .collect()
}

pub fn right_ink(s: &Settings) -> Color {
    let white = ink(INK_RIGHT);
    if s.style == super::Style::Onyx {
        return white;
    }
    let reads = |c: Color| contrast(c, white) >= INK_CONTRAST;
    if reads(glass_colour(s)) && lit_colours(s).into_iter().all(reads) {
        white
    } else {
        ink(INK_STRONG)
    }
}

fn panel(s: &Settings, d: i32) -> PartImage {
    let (w, h) = (2 * PANEL_EDGE + 1, 2 + SHEEN_ROWS + 1 + 2);
    PartImage::new(
        super::panel::draw(&PANEL, s, w, h, d),
        &[State::Normal],
        Stack::Vertical,
    )
    .sizing(Insets::new(PANEL_EDGE, h - 3, PANEL_EDGE, 2))
    .at_scale(d)
}

/// The parts [`parts`] paints.
pub(crate) const PARTS: [Part; 25] = [
    Part::StartPanel,
    Part::StartPane,
    Part::StartJumpPane,
    Part::StartHotLeft,
    Part::StartNewPin,
    Part::StartHotAll,
    Part::StartHotSplit,
    Part::StartHotJump,
    Part::StartLitResult,
    Part::StartHotRight,
    Part::StartRuleLeft,
    Part::StartRuleRight,
    Part::StartJumpRule,
    Part::StartSearchRule,
    Part::StartShutdown,
    Part::StartShutdownArrow,
    Part::StartShutdownLight,
    Part::StartShutdownArrowLight,
    Part::StartSearchField,
    Part::StartMagnifier,
    Part::StartClear,
    Part::StartTriangleRight,
    Part::StartTriangleLeft,
    Part::StartRowArrow,
    Part::StartPictureFrame,
];

pub(crate) fn parts(s: &Settings, d: i32) -> Vec<(Part, PartImage)> {
    let mut parts = vec![(Part::StartPanel, panel(s, d))];
    parts.extend(super::startparts::parts(s, d));
    match s.style {
        super::Style::Glass => parts,
        super::Style::Onyx => super::onyxstart::parts(s, d, parts),
    }
}

mod at {
    #[cfg(test)]
    pub use crate::startpanel::search::at::*;
    pub const PROGRAM_BOX: (i32, i32) = (0, 1);
    pub const PROGRAM_BOX_H: i32 = 36;
    pub const PROGRAM_ICON: (i32, i32) = (2, 3);
    pub const PROGRAM_PEN: i32 = 40;
    pub const PROGRAM_BASE: i32 = 23;
    /// An Onyx pinned row's bold name and the grey line under it.
    pub const NAME_BASE: i32 = 16;
    pub const DETAIL_BASE: i32 = 31;
    pub const ROW_ARROW: (i32, i32) = (231, 15);
    pub const TRIANGLE: (i32, i32) = (8, 5);
    pub const ALL_PEN: i32 = 38;
    pub const BACK_PEN: i32 = 39;
    pub const ALL_BASE: i32 = 16;
    pub const PLACE_PEN: i32 = 8;
    pub const PLACE_BASE: i32 = 21;
    pub const PLACE_ARROW: (i32, i32) = (125, 13);
    pub const SHUT_W: i32 = 71;
    pub const SHUT_BASE: i32 = 15;
    pub const PICTURE: (i32, i32) = (8, 6);
    pub const PICTURE_PX: i32 = 48;
    pub const JUMP_HEAD_BASE: i32 = 13;
    pub const JUMP_RULE_Y: i32 = 8;
    pub const JUMP_RULE_GAP: i32 = 4;
    pub const JUMP_RULE_PAST: i32 = 1;
    pub const JUMP_BOX: (i32, i32) = (0, 1);
    pub const JUMP_BOX_H: i32 = 20;
    pub const JUMP_ICON: (i32, i32) = (2, 3);
    pub const JUMP_PEN: i32 = 25;
    pub const JUMP_BASE: i32 = 15;
    pub const NONE_BASE: i32 = 31;
    pub const TREE_ICON: (i32, i32) = (1, 2);
    pub const TREE_PEN: i32 = 22;
    pub const TREE_BASE: i32 = 15;
}

/// The panel's frame and left pane as drawn on a clear canvas for one size, scale and style.
/// Kept across draws (and opens), it saves the two largest parts' blits on every later draw.
#[derive(Default)]
pub struct Backing(Option<Backed>);

struct Backed {
    style: Arc<Style>,
    shape: Shape,
    panel: Rect,
    pane: Rect,
    canvas: Canvas,
}

/// The frame and left pane's rects on `l`'s canvas.
fn backing_rects(l: &Layout) -> (Rect, Rect) {
    let o = l.canvas;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height);
    (at(l.panel), at(l.pane.inset(-2 * l.scale.max(1))))
}

pub fn draw(
    canvas: &mut Canvas,
    style: &Arc<Style>,
    text: &TextRenderer,
    p: &StartPanel,
    l: &Layout,
    pressed: Option<Spot>,
    caret: bool,
) {
    let (panel, pane) = backing_rects(l);
    draw_frame(canvas, style, p.shape, panel, pane, l.scale.max(1));
    draw_contents(canvas, style, text, p, l, pressed, caret);
}

/// The panel's frame and glass over `panel` and its left pane over `pane`, on `canvas`.
fn draw_frame(canvas: &mut Canvas, style: &Style, shape: Shape, panel: Rect, pane: Rect, s: i32) {
    match shape {
        Shape::Onyx => super::onyxstart::panel(canvas, panel, s),
        _ => style.draw(canvas, panel, Part::StartPanel, State::Normal, s),
    }
    style.draw(canvas, pane, Part::StartPane, State::Normal, s);
}

/// [`draw`] onto a clear `canvas`, its frame and left pane copied from `back` when `back` holds
/// them for this size and style, else drawn and kept there.
#[allow(clippy::too_many_arguments)]
pub fn draw_backed(
    back: &mut Backing,
    canvas: &mut Canvas,
    style: &Arc<Style>,
    text: &TextRenderer,
    p: &StartPanel,
    l: &Layout,
    pressed: Option<Spot>,
    caret: bool,
) {
    let (panel, pane) = backing_rects(l);
    let fits = |b: &Backed| {
        Arc::ptr_eq(&b.style, style)
            && b.shape == p.shape
            && (b.panel, b.pane) == (panel, pane)
            && b.canvas.pixels.len() == canvas.pixels.len()
            && (b.canvas.width, b.canvas.height) == (canvas.width, canvas.height)
    };
    match &back.0 {
        Some(b) if fits(b) => canvas.pixels.copy_from_slice(&b.canvas.pixels),
        _ => {
            draw_frame(canvas, style, p.shape, panel, pane, l.scale.max(1));
            back.0 = Some(Backed {
                style: style.clone(),
                shape: p.shape,
                panel,
                pane,
                canvas: canvas.clone(),
            });
        }
    }
    draw_contents(canvas, style, text, p, l, pressed, caret);
}

/// Everything over the frame and left pane.
fn draw_contents(
    canvas: &mut Canvas,
    style: &Arc<Style>,
    text: &TextRenderer,
    p: &StartPanel,
    l: &Layout,
    pressed: Option<Spot>,
    caret: bool,
) {
    let s = l.scale.max(1);
    let o = l.canvas;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height);
    let g = search::Grow { s, u: p.text };
    let off = |r: Rect, (dx, dy): (i32, i32)| (r.x - o.x + g.len(dx), r.y - o.y + g.len(dy));
    // An arrow grows by whole steps with the text, drawn afresh at the grown scale; its centre
    // moves with the rest.
    let k = p.text.small();
    let arrows = match k {
        1 => style.clone(),
        k => style.at_scale(s * k),
    };
    let stepped = |r: Rect, (dx, dy): (i32, i32), (w, h): (i32, i32)| {
        (r.x - o.x + g.stepped(dx, w), r.y - o.y + g.stepped(dy, h))
    };
    let icon_at = |r: Rect, (dx, dy): (i32, i32), size: i32, c: &Canvas| {
        let (w, h) = (c.width as i32, c.height as i32);
        (
            r.x - o.x + g.centred(dx, size, w),
            r.y - o.y + g.centred(dy, size, h),
        )
    };
    let glyph_at =
        |r: Rect, off: (i32, i32), part: Part| stepped(r, off, search::glyph_size(style, part));
    let mode = p.mode();
    let lit = p.lit();
    let light = matches!(mode, Mode::Jump(_) | Mode::Search);
    let mut d = Draw {
        canvas,
        style,
        arrows: &arrows,
        s,
        k,
    };
    let size = g.px(TEXT_PX);
    let inks = style.pane_inks;
    let right = style
        .text_color(Part::StartPanel, State::Normal)
        .unwrap_or(ink(INK_RIGHT));

    if let Some(j) = l.jump_pane {
        d.part(at(j), Part::StartJumpPane, State::Normal);
    }
    for &r in &l.rules {
        let part = if r.x >= l.pane.right() {
            Part::StartRuleRight
        } else {
            Part::StartRuleLeft
        };
        d.part(at(r), part, State::Normal);
    }

    let jump_row = match mode {
        Mode::Jump(i) => Some(i),
        _ => None,
    };
    let row_clip = |r: Rect| at(Rect::new(r.x, r.y, r.width - 2 * s, r.height));
    for &(spot, r) in &l.spots {
        match spot {
            Spot::Program(i) => {
                let Some(row) = p.program(i) else { continue };
                let (bx, by) = off(r, at::PROGRAM_BOX);
                let b = Rect::new(bx, by, r.width, g.len(at::PROGRAM_BOX_H));
                let is_lit = lit == Some(spot) || jump_row == Some(i);
                if is_lit {
                    d.part(b, Part::StartHotLeft, State::Normal);
                    if row.jump {
                        let x = b.x + g.len(layout::SPLIT_X - layout::shift(p));
                        d.part(
                            Rect::new(x, b.y, 3 * s, b.height),
                            Part::StartHotSplit,
                            State::Normal,
                        );
                    }
                } else if row.new_pin {
                    d.part(b, Part::StartNewPin, State::Normal);
                }
                if let Some(icon) = &row.icon {
                    let (x, y) = icon_at(r, at::PROGRAM_ICON, PROGRAM_ICON, icon);
                    d.canvas.blit_over(x, y, icon);
                }
                let right = if row.jump {
                    g.len(layout::SPLIT_X - layout::shift(p))
                } else {
                    r.width - 4 * s
                };
                let clip = at(Rect::new(r.x, r.y, right, r.height));
                match row.detail.as_deref().filter(|_| p.shape == Shape::Onyx) {
                    Some(detail) => {
                        let (x, y) = off(r, (at::PROGRAM_PEN, at::NAME_BASE));
                        d.bold_line(text, x, y, clip, &row.name, inks.program, size);
                        let (x, y) = off(r, (at::PROGRAM_PEN, at::DETAIL_BASE));
                        d.line(text, x, y, clip, detail, inks.greyed, size);
                    }
                    None => {
                        let (x, y) = off(r, (at::PROGRAM_PEN, at::PROGRAM_BASE));
                        d.line(text, x, y, clip, &row.name, inks.program, size);
                    }
                }
                if row.jump {
                    let arrow = Part::StartRowArrow;
                    let (ax, ay) = at::ROW_ARROW;
                    let ax = ax - layout::shift(p);
                    d.glyph(glyph_at(r, (ax, ay), arrow), arrow);
                }
            }
            Spot::AllPrograms => {
                if lit == Some(spot) {
                    d.part(at(r), Part::StartHotAll, State::Normal);
                }
                let (label, pen, part) = if mode == Mode::Programs {
                    (&back(), at::BACK_PEN, Part::StartTriangleLeft)
                } else {
                    (&all_programs(), at::ALL_PEN, Part::StartTriangleRight)
                };
                d.glyph(glyph_at(r, at::TRIANGLE, part), part);
                let (x, y) = off(r, (pen, at::ALL_BASE));
                d.line(text, x, y, row_clip(r), label, inks.strong, size);
            }
            Spot::Tree(i) => {
                let Some(row) = p.tree.get(i) else { continue };
                if lit == Some(spot) {
                    d.part(at(r), Part::StartHotLeft, State::Normal);
                }
                let (name, icon, depth) = match row {
                    TreeRow::Folder { name, icon, .. } => (name, icon.as_ref(), 0),
                    TreeRow::App { name, icon, depth } => (name, icon.as_ref(), *depth as i32),
                };
                let indent = depth * crate::startpanel::layout::TREE_INDENT;
                if let Some(icon) = icon {
                    let off = (at::TREE_ICON.0 + indent, at::TREE_ICON.1);
                    let (x, y) = icon_at(r, off, SMALL_ICON, icon);
                    d.canvas.blit_over(x, y, icon);
                }
                let (x, y) = off(r, (at::TREE_PEN + indent, at::TREE_BASE));
                d.line(text, x, y, row_clip(r), name, inks.strong, size);
            }
            Spot::Result(gi, k) => {
                let Some(row) = p.results.get(gi).and_then(|grp| grp.rows.get(k)) else {
                    continue;
                };
                let lit = lit == Some(spot);
                let icon = row.icon.as_ref();
                let (name, detail) = (&row.name, &row.detail);
                search::result(d.canvas, style, text, at(r), icon, name, detail, lit, g);
            }
            Spot::Jump(k) => {
                let Some(row) = p.jump_row(k) else { continue };
                if lit == Some(spot) {
                    let (bx, by) = off(r, at::JUMP_BOX);
                    let b = Rect::new(bx, by, r.width, g.len(at::JUMP_BOX_H));
                    d.part(b, Part::StartHotJump, State::Normal);
                }
                if let Some(icon) = &row.icon {
                    let (x, y) = icon_at(r, at::JUMP_ICON, SMALL_ICON, icon);
                    d.canvas.blit_over(x, y, icon);
                }
                let (x, y) = off(r, (at::JUMP_PEN, at::JUMP_BASE));
                d.line(text, x, y, row_clip(r), &row.name, inks.jump, size);
            }
            Spot::Place(i) => {
                let Some(place) = p.places.get(i) else {
                    continue;
                };
                if lit == Some(spot) {
                    d.part(at(r), Part::StartHotRight, State::Normal);
                }
                let lift = layout::place_lift(p);
                let (x, y) = off(r, (at::PLACE_PEN, at::PLACE_BASE - lift));
                d.line(text, x, y, row_clip(r), &place.label, right, size);
                if place.submenu {
                    let size = (glyph::ARROW.w, glyph::ARROW.h);
                    let (ax, ay) = at::PLACE_ARROW;
                    let ax = ax - layout::narrow(p) - layout::place_inset(p);
                    d.inked(stepped(r, (ax, ay - lift), size), &glyph::ARROW, right);
                }
            }
            Spot::Search => {
                let cue = search::italic(&font());
                let focused = p.focus() == Spot::Search;
                search::field(
                    d.canvas,
                    style,
                    text,
                    &cue,
                    at(r),
                    p.edit(),
                    focused,
                    caret,
                    g,
                );
            }
            Spot::Shutdown if p.shape == Shape::Onyx => {
                let spots = [Spot::Shutdown, Spot::Lock, Spot::ShutdownArrow];
                let [Some(power), Some(lock), Some(arrow)] = spots.map(|k| l.rect(k).map(at))
                else {
                    continue;
                };
                let state = |k: Spot| match (pressed == Some(k), lit == Some(k)) {
                    (true, _) => State::Pressed,
                    (false, true) => State::Hot,
                    (false, false) => State::Normal,
                };
                let group = power.union(&arrow);
                let states = spots.map(state);
                super::onyxstart::buttons(d.canvas, group, [lock.x, arrow.x], states, s, s * k);
            }
            Spot::Shutdown => {
                // The pointer lights both halves; a key lights the half it stands on.
                let (button_lit, arrow_lit) = match lit {
                    Some(Spot::Shutdown | Spot::ShutdownArrow) if p.hot() == lit => (true, true),
                    Some(Spot::Shutdown) => (true, false),
                    Some(Spot::ShutdownArrow) => (false, true),
                    _ => (false, false),
                };
                let held = matches!(pressed, Some(Spot::Shutdown | Spot::ShutdownArrow));
                let state = |lit: bool| match (held, lit) {
                    (true, _) => State::Pressed,
                    (false, true) => State::Hot,
                    (false, false) => State::Normal,
                };
                let halves = [
                    (
                        Some(at(r)),
                        button_lit,
                        Part::StartShutdown,
                        Part::StartShutdownLight,
                    ),
                    (
                        l.rect(Spot::ShutdownArrow).map(at),
                        arrow_lit,
                        Part::StartShutdownArrow,
                        Part::StartShutdownArrowLight,
                    ),
                ];
                for (half, lit, glass, plain) in halves {
                    let Some(half) = half else { continue };
                    // The light button has no pointer look, only the keys'.
                    match (light, lit && p.hot().is_none()) {
                        (false, _) => d.part(half, glass, state(lit)),
                        (true, true) => d.part(half, plain, State::Hot),
                        (true, false) => d.part(half, plain, State::Normal),
                    }
                }
                let label = p.power.label();
                let w = text.measure(&label, false, size);
                let x = r.x - o.x + (g.len(at::SHUT_W) - w) / 2;
                let y = r.y - o.y + g.len(at::SHUT_BASE);
                let color = if light { inks.strong } else { right };
                d.line(text, x, y, at(r), &label, color, size);
            }
            Spot::Picture => {
                let place_picture = p
                    .picture()
                    .and_then(|i| p.places.get(i))
                    .and_then(|pl| pl.picture.as_ref());
                // The frame keeps its size, so the picture keeps its place in it.
                let (px, py) = match p.shape {
                    Shape::Onyx => super::onyxstart::PICTURE_AT,
                    _ => at::PICTURE,
                };
                let (fx, fy) = (r.x - o.x + px * s, r.y - o.y + py * s);
                let pic = |c: &Canvas| {
                    let (x, y) = (fx, fy);
                    let half = |n: u32| (at::PICTURE_PX * s - n as i32) / 2;
                    (x + half(c.width), y + half(c.height))
                };
                match place_picture {
                    Some(c) => {
                        let (x, y) = pic(c);
                        d.canvas.blit_over(x, y, c);
                    }
                    None => {
                        d.part(at(r), Part::StartPictureFrame, State::Normal);
                        if let Some(c) = &p.user_picture {
                            // On white, so a see-through picture hides
                            // the panel's edge behind it.
                            let (x, y) = (fx, fy);
                            let side = at::PICTURE_PX * s;
                            d.canvas
                                .fill_rect(Rect::new(x, y, side, side), Color::rgb(255, 255, 255));
                            let (x, y) = pic(c);
                            d.canvas.blit_over(x, y, c);
                        }
                    }
                }
            }
            Spot::Clear | Spot::ShutdownArrow | Spot::Lock | Spot::LogOff => {}
        }
    }

    for &(gi, r) in &l.headers {
        let pane_clip = at(l.pane);
        match mode {
            Mode::Jump(_) => {
                let Some(section) = p.jump.get(gi) else {
                    continue;
                };
                let (x, y) = off(r, (0, at::JUMP_HEAD_BASE));
                let w = d.line(text, x, y, pane_clip, &section.title, inks.jump_title, size);
                let x0 = x + w + g.len(at::JUMP_RULE_GAP);
                let x1 = r.right() + g.len(at::JUMP_RULE_PAST) + s - o.x;
                let ry = r.y - o.y + g.len(at::JUMP_RULE_Y);
                d.part(
                    Rect::new(x0, ry, x1 - x0, s),
                    Part::StartJumpRule,
                    State::Normal,
                );
            }
            Mode::Search => {
                let Some(group) = p.results.get(gi) else {
                    continue;
                };
                search::header(d.canvas, style, text, at(r), pane_clip, &group.title, g);
            }
            _ => {}
        }
    }
    if mode == Mode::Search && !p.results.iter().any(|g| !g.rows.is_empty()) {
        let w = text.measure(&no_results(), false, size);
        let pane = at(l.pane);
        let x = pane.x + (pane.width - w) / 2;
        let y = pane.y + g.len(at::NONE_BASE);
        d.line(text, x, y, pane, &no_results(), inks.greyed, size);
    }
}

struct Draw<'a> {
    canvas: &'a mut Canvas,
    style: &'a Style,
    /// The style the arrows draw from, at `s * k`.
    arrows: &'a Style,
    s: i32,
    /// The whole multiple of their 100% size the arrows draw at
    /// ([`crate::theme::TextUnits::small`]).
    k: i32,
}

impl Draw<'_> {
    fn part(&mut self, r: Rect, part: Part, state: State) {
        self.style.draw(self.canvas, r, part, state, self.s);
    }

    fn inked(&mut self, (x, y): (i32, i32), m: &glyph::Mark, color: Color) {
        let ink = crate::shape::premul(color, 0xFF);
        self.canvas
            .blit_over(x, y, &m.canvas(self.s * self.k, |_, _| ink));
    }

    fn glyph(&mut self, at: (i32, i32), part: Part) {
        search::glyph(self.canvas, self.arrows, at, part, self.s * self.k);
    }

    #[expect(clippy::too_many_arguments)]
    fn line(
        &mut self,
        face: &TextRenderer,
        x: i32,
        y: i32,
        clip: Rect,
        line: &str,
        color: Color,
        size: u32,
    ) -> i32 {
        search::line(self.canvas, face, x, y, clip, line, color, size)
    }

    #[expect(clippy::too_many_arguments)]
    fn bold_line(
        &mut self,
        face: &TextRenderer,
        x: i32,
        y: i32,
        clip: Rect,
        line: &str,
        color: Color,
        size: u32,
    ) {
        let shown = face.ellipsize(line, clip.right() - x, true, size);
        face.draw_run_over(self.canvas, x, y, clip, &shown, true, color, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::startpanel::layout::layout;
    use crate::startpanel::tests::section;
    use crate::startpanel::{PlaceRow, ProgramRow, ResultGroup, ResultRow};
    use crate::textedit::EditKey;

    const WHITE: u32 = 0xFFFF_FFFF;

    fn prog(name: &str, jump: bool) -> ProgramRow {
        ProgramRow {
            detail: None,
            name: name.into(),
            icon: None,
            jump,
            new_pin: false,
        }
    }

    fn row(name: &str) -> ResultRow {
        ResultRow {
            name: name.into(),
            ..ResultRow::default()
        }
    }

    fn sample(mode: Mode, hot: Option<Spot>) -> StartPanel {
        let mut p = StartPanel::new();
        p.pinned = vec![prog("Files", false), prog("Terminal", false)];
        p.frequent = (0..8)
            .map(|k| prog(&format!("Program {k}"), k == 1))
            .collect();
        p.places = [
            "user",
            "Documents",
            "Pictures",
            "Music",
            "Recent Items",
            "Computer",
            "Control Panel",
            "Devices and Printers",
            "Default Programs",
            "Help and Support",
        ]
        .iter()
        .enumerate()
        .map(|(k, label)| PlaceRow {
            label: label.to_string(),
            picture: None,
            rule_before: k == 4 || k == 6,
            submenu: false,
            context: false,
        })
        .collect();
        p.jump = vec![section("Recent", &["notes.txt", "todo.txt"])];
        match mode {
            Mode::Search => {
                p.key(crate::startpanel::Key::Char(EditKey::Insert("no".into())));
                p.set_results(vec![
                    ResultGroup {
                        title: "Programs (3)".into(),
                        rows: vec![row("Notepad"), row("Notes"), row("Nomad")],
                    },
                    ResultGroup {
                        title: "Files (1)".into(),
                        rows: vec![row("notes.txt")],
                    },
                ]);
            }
            m => p.set_mode(m),
        }
        p.pointer(hot);
        p
    }

    struct Drawn {
        c: Canvas,
        l: Layout,
    }

    impl Drawn {
        fn at(&self, x: i32, y: i32) -> u32 {
            self.c.get(x - self.l.canvas.x, y - self.l.canvas.y)
        }
    }

    fn render(p: &StartPanel, pressed: Option<Spot>) -> Drawn {
        render_in(&Settings::default(), p, pressed)
    }

    fn render_in(s: &Settings, p: &StartPanel, pressed: Option<Spot>) -> Drawn {
        let style = super::super::style(s);
        let text = TextRenderer::load(&font());
        let l = layout(p);
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        draw(&mut c, &style, &text, p, &l, pressed, true);
        Drawn { c, l }
    }

    #[test]
    fn a_backed_draw_matches_a_fresh_one_and_a_new_style_draws_its_own_frame() {
        let text = TextRenderer::load(&font());
        let blue = super::super::style(&Settings::default());
        let red = super::super::style(&Settings {
            color: Color::rgb(201, 24, 15),
            intensity: 75,
            ..Settings::default()
        });
        let blank = |l: &Layout| Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        let fresh = |style: &Arc<Style>, p: &StartPanel| {
            let l = layout(p);
            let mut c = blank(&l);
            draw(&mut c, style, &text, p, &l, None, true);
            c.pixels
        };
        let backed = |back: &mut Backing, style: &Arc<Style>, p: &StartPanel| {
            let l = layout(p);
            let mut c = blank(&l);
            draw_backed(back, &mut c, style, &text, p, &l, None, true);
            c.pixels
        };
        let (home, search) = (sample(Mode::Home, None), sample(Mode::Search, None));
        let jump = sample(Mode::Jump(1), Some(Spot::Jump(0)));
        let mut back = Backing::default();
        assert_eq!(backed(&mut back, &blue, &home), fresh(&blue, &home));
        assert!(back.0.is_some(), "the frame is kept");
        assert_eq!(backed(&mut back, &blue, &search), fresh(&blue, &search));
        assert_eq!(backed(&mut back, &blue, &jump), fresh(&blue, &jump));
        assert_ne!(fresh(&red, &home), fresh(&blue, &home));
        assert_eq!(
            backed(&mut back, &red, &home),
            fresh(&red, &home),
            "another style draws its own frame"
        );
    }

    fn plain(s: &Settings) -> u32 {
        let (a, p) = super::super::tint::plain(s, Surface::Active);
        let v = |c: f64| c.round() as u32;
        v(a) << 24 | v(p[0]) << 16 | v(p[1]) << 8 | v(p[2])
    }

    fn dense_fill(s: &Settings) -> u32 {
        let (a, p) = super::super::tint::plain(s, Surface::Active);
        let a = (a + (255.0 - a) * DENSITY).round();
        let v = |c: f64| c.round().min(a) as u32;
        (a as u32) << 24 | v(p[0]) << 16 | v(p[1]) << 8 | v(p[2])
    }

    fn frame_edges(s: &Settings) -> (u32, u32) {
        let st = super::super::style(s);
        let mut c = Canvas::new(600, 30);
        let all = c.bounds();
        st.draw(&mut c, all, Part::Caption, State::Normal, 1);
        (c.get(300, 0), c.get(300, 1))
    }

    fn glass(s: &Settings, w: i32, h: i32) -> Canvas {
        let st = super::super::style(s);
        let mut c = Canvas::new(w as u32, h as u32);
        let all = c.bounds();
        st.draw(&mut c, all, Part::StartPanel, State::Normal, 1);
        c
    }

    fn glasses() -> Vec<Settings> {
        let red = Settings {
            color: Color::rgb(201, 24, 15),
            intensity: 75,
            ..Settings::default()
        };
        vec![
            Settings::default(),
            red,
            Settings {
                material: super::super::Material::Smoked,
                ..red
            },
            Settings {
                material: super::super::Material::Molten,
                ..red
            },
        ]
    }

    #[test]
    fn the_panels_glass_is_the_frames_outline_and_light_line_round_dense_glass() {
        for s in glasses() {
            let (outline, light) = frame_edges(&s);
            for (w, h) in [(411, 480), (411, 476), (491, 480)] {
                let c = glass(&s, w, h);
                let at = |x: i32, y: i32| c.get(x, y);
                let what = format!("{:?} at {}, {w}x{h}", s.color, s.intensity);
                for (x, y) in [(200, 0), (0, 200), (w - 1, 200), (200, h - 1)] {
                    assert_eq!(at(x, y), outline, "{what}: the outline at ({x}, {y})");
                }
                for (x, y) in [(200, 1), (1, 200), (w - 2, 200), (200, h - 2)] {
                    assert_eq!(at(x, y), light, "{what}: the light line at ({x}, {y})");
                }
                for (x, y) in [(300, 200), (w - 3, h - 3), (2, 2 + SHEEN_ROWS)] {
                    assert_eq!(
                        at(x, y),
                        dense_fill(&s),
                        "{what}: dense glass at ({x}, {y})"
                    );
                }
                assert_eq!(at(0, h - 1), outline, "{what}: bottom left");
                assert_eq!(at(w - 1, h - 1), outline, "{what}: bottom right");
                assert_eq!(at(1, h - 2), light, "{what}: the light line's corner");
            }
        }
    }

    #[test]
    fn the_panels_top_corners_are_cut_and_outlined() {
        let s = Settings::default();
        let (outline, light) = frame_edges(&s);
        let c = glass(&s, 411, 480);
        for (y, &cut) in PANEL_CORNER.iter().enumerate() {
            let y = y as i32;
            for x in 0..cut {
                assert_eq!(c.get(x, y), 0, "({x}, {y}) is clear");
                assert_eq!(c.get(410 - x, y), 0, "({}, {y}) is clear", 410 - x);
            }
            assert_eq!(c.get(cut, y), outline, "row {y} starts on the outline");
            assert_eq!(c.get(410 - cut, y), outline, "and ends on it");
        }
        assert_eq!(c.get(4, 1), outline, "the outline's step");
        assert_eq!(c.get(5, 1), light, "the light line under the top");
        assert_eq!(c.get(1, 5), light, "the light line down the side");
    }

    #[test]
    fn the_panels_sheen_fades_down_into_the_dense_glass() {
        for s in glasses() {
            let c = glass(&s, 411, 480);
            let light = |y: i32| {
                let px = over(c.get(200, y), 0xFF3E_3E3E);
                (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF)
            };
            let rows: Vec<u32> = (2..=2 + SHEEN_ROWS).map(light).collect();
            let what = format!("{:?} {:?}", s.color, s.material);
            assert!(
                rows.windows(2).all(|w| w[0] + 2 >= w[1]),
                "{what}: the sheen only fades down, but for rounding, {rows:?}"
            );
            assert!(
                rows[0] > rows[rows.len() - 1] + 60,
                "{what}: the first row is brighter than the glass under it, {rows:?}"
            );
            assert_eq!(
                c.get(200, 2 + SHEEN_ROWS),
                dense_fill(&s),
                "{what}: then the dense glass"
            );
        }
    }

    #[test]
    fn the_panels_glass_follows_the_material_and_the_colour() {
        let fill = |s: &Settings| glass(s, 411, 480).get(300, 200);
        let [sky, red, smoked, molten] = glasses().try_into().unwrap();
        let frosted_sky = fill(&sky);
        let frosted_red = fill(&red);
        assert_ne!(frosted_sky, frosted_red, "another colour, another glass");
        let (r, b) = ((frosted_red >> 16) & 0xFF, frosted_red & 0xFF);
        assert!(r > b + 0x40, "the red glass is red: {frosted_red:08x}");
        assert_ne!(fill(&smoked), frosted_red, "Smoked changes it");
        assert_ne!(fill(&molten), frosted_red, "Molten changes it");
        let sky_smoked = Settings {
            material: super::super::Material::Smoked,
            ..sky
        };
        assert!(
            fill(&sky_smoked) >> 24 > frosted_sky >> 24,
            "Smoked lets less through"
        );
        assert!(
            fill(&molten) >> 24 < frosted_red >> 24,
            "Molten lets more through"
        );
        assert_ne!(
            fill(&sky_smoked),
            frosted_sky,
            "Smoked Sky is not Frosted Sky"
        );
    }

    use crate::pins::fnv;

    #[test]
    fn the_panels_glass_keeps_its_pixels_on_the_shared_painter() {
        let opaque = Settings {
            transparency: false,
            ..Settings::default()
        };
        let names = ["default", "red", "red smoked", "red molten", "opaque"];
        let got: Vec<_> = glasses()
            .iter()
            .chain([&opaque])
            .zip(names)
            .map(|(s, name)| (name.to_string(), fnv(&panel(s, 1).atlas)))
            .collect();
        assert_eq!(got.len(), names.len());
        crate::pins::check("glass/start_panel_glass", &got);
    }

    #[test]
    fn the_panels_glass_is_160_dense_in_sky() {
        let fill = glass(&Settings::default(), 411, 480).get(300, 200);
        assert_eq!(
            fill >> 24,
            160,
            "the frame's 62 under a layer covering DENSITY of the rest: {fill:08x}"
        );
    }

    #[test]
    fn the_panels_colour_moves_as_plain_glass_and_its_alpha_by_1_minus_density() {
        let fill = |s: &Settings| glass(s, 411, 480).get(300, 200);
        let sky = Settings::default();
        let channels = |px: u32| [24, 16, 8, 0].map(|sh| f64::from((px >> sh) & 0xFF));
        let (from, p0) = (channels(fill(&sky)), channels(plain(&sky)));
        for color in [
            sky.color,
            Color::rgb(201, 24, 15),
            Color::rgb(252, 197, 245),
        ] {
            for material in super::super::Material::ALL {
                for intensity in (0..=100).step_by(10) {
                    let s = Settings {
                        color,
                        material,
                        intensity,
                        ..sky
                    };
                    let (to, p1) = (channels(fill(&s)), channels(plain(&s)));
                    let what = format!("{color:?} {material:?} at {intensity}");
                    let (moved, plain_moved) = (to[0] - from[0], p1[0] - p0[0]);
                    assert!(
                        (moved - (1.0 - DENSITY) * plain_moved).abs() <= 1.5,
                        "{what}: alpha moves {moved}, (1 - DENSITY) of plain glass's {plain_moved}"
                    );
                    assert!(to[0] < 255.0, "{what}: the panel stays translucent");
                    for i in 1..4 {
                        let (moved, plain_moved) = (to[i] - from[i], p1[i] - p0[i]);
                        assert!(
                            (moved - plain_moved).abs() <= 1.0,
                            "{what}: channel {i} moves {moved}, exactly as plain glass's {plain_moved}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_right_columns_ink_keeps_3_to_1_on_the_glass_and_a_lit_place_in_every_setting() {
        let l = layout(&sample(Mode::Home, None));
        let rows: Vec<(i32, i32)> = (0..10)
            .filter_map(|i| l.rect(Spot::Place(i)))
            .flat_map(|r| {
                let base = r.y + at::PLACE_BASE;
                (base - 9..base).map(move |y| (r.x + at::PLACE_PEN, y))
            })
            .map(|(x, y)| (x - l.panel.x, y - l.panel.y))
            .collect();
        assert_eq!(rows.len(), 90, "the ten places' letters");
        let (top, bottom) = (2 + SHEEN_ROWS, l.panel.height - 2);
        assert!(rows.iter().all(|&(_, y)| (top..bottom).contains(&y)));
        let hot = super::super::startparts::hot_right();
        let mut dark = 0;
        let (mut worst_rest, mut worst_lit) = (f64::MAX, f64::MAX);
        let mut neither = Vec::new();
        let white = Color::rgb(255, 255, 255);
        for w in super::super::SWATCHES {
            for intensity in (0..=100).step_by(5) {
                for material in super::super::Material::ALL {
                    for transparency in [false, true] {
                        let s = Settings {
                            color: w.color,
                            intensity,
                            material,
                            transparency,
                            ..Settings::default()
                        };
                        let ink = right_ink(&s);
                        dark += usize::from(ink != white);
                        let what = format!(
                            "{} at {intensity} {material:?}, translucent {transparency}",
                            w.name
                        );
                        let px = panel(&s, 1).atlas.get(PANEL_EDGE, top);
                        assert_eq!(px >> 24 == 0xFF, !transparency, "{what}: opaque or not");
                        let lit: Vec<Color> = (at::PLACE_BASE - 9..at::PLACE_BASE)
                            .flat_map(|y| {
                                (at::PLACE_PEN..hot.width as i32 - at::PLACE_PEN)
                                    .map(move |x| crate::raster::over_round(hot.get(x, y), px))
                            })
                            .map(super::shown)
                            .collect();
                        let least = |ink: Color| {
                            let rest = contrast(super::shown(px), ink);
                            let lit = lit
                                .iter()
                                .map(|&c| contrast(c, ink))
                                .fold(f64::MAX, f64::min);
                            (rest, lit)
                        };
                        let (rest, lit_k) = least(ink);
                        assert!(
                            rest >= INK_CONTRAST,
                            "{what}: {rest:.2} on the glass, {px:08x}"
                        );
                        if lit_k >= INK_CONTRAST {
                            worst_rest = worst_rest.min(rest);
                            worst_lit = worst_lit.min(lit_k);
                            continue;
                        }
                        let other = if ink == white {
                            super::ink(INK_STRONG)
                        } else {
                            white
                        };
                        let (o_rest, o_lit) = least(other);
                        assert!(
                            o_rest.min(o_lit) < INK_CONTRAST && lit_k >= o_lit.min(o_rest),
                            "{what}: {lit_k:.2} lit, the other ink {o_rest:.2} at rest, {o_lit:.2} lit"
                        );
                        neither.push(what);
                    }
                }
            }
        }
        assert!(dark > 0, "a light glass takes the dark ink");
        assert!(worst_rest >= INK_CONTRAST && worst_lit >= INK_CONTRAST);
        assert_eq!(
            neither,
            ["frost at 65 Frosted, translucent true"],
            "the only glass where no ink keeps 3:1 on a lit place"
        );
        let sky = Settings {
            transparency: false,
            ..Settings::default()
        };
        assert_eq!(
            right_ink(&sky),
            Color::rgb(255, 255, 255),
            "Sky keeps white"
        );
    }

    #[test]
    fn a_light_glass_draws_its_places_in_the_dark_ink() {
        let s = Settings {
            color: Color::rgb(252, 251, 252),
            intensity: 100,
            ..Settings::default()
        };
        assert_eq!(right_ink(&s), ink(INK_STRONG), "Frost at 100");
        let style = super::super::style(&s);
        assert_eq!(
            style.text_color(Part::StartPanel, State::Normal),
            Some(ink(INK_STRONG)),
            "the style carries it"
        );
        if !face() {
            return;
        }
        let text = TextRenderer::load(&font());
        let p = sample(Mode::Home, None);
        let l = layout(&p);
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        draw(&mut c, &style, &text, &p, &l, None, true);
        let d = Drawn { c, l };
        let row = d.l.rect(Spot::Place(1)).unwrap();
        assert!(inks(&d, row, INK_STRONG), "Documents in black");
        assert!(!inks(&d, row, INK_RIGHT), "not in white");
    }

    fn over(px: u32, under: u32) -> u32 {
        let a = px >> 24;
        let ch = |sh: u32| {
            let (p, u) = ((px >> sh) & 0xFF, (under >> sh) & 0xFF);
            (p + (u * (255 - a) + 127) / 255).min(255) << sh
        };
        0xFF00_0000 | ch(16) | ch(8) | ch(0)
    }

    fn inks(d: &Drawn, r: Rect, rgb: u32) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| d.at(x, y) == 0xFF00_0000 | rgb))
    }

    /// Whether a pixel in `r` is within 2 of `rgb` on every channel: a thin stem's coverage can
    /// stop one short of full.
    fn inks_near(d: &Drawn, r: Rect, rgb: u32) -> bool {
        let near = |px: u32| {
            [16, 8, 0]
                .iter()
                .all(|&sh| ((px >> sh) as u8).abs_diff((rgb >> sh) as u8) <= 2)
        };
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| near(d.at(x, y))))
    }

    fn face() -> bool {
        let text = TextRenderer::load(&font());
        let found = ["Segoe UI", "Noto Sans", "DejaVu Sans"]
            .iter()
            .any(|f| text.has_face(f));
        if !found {
            crate::pins::skipped("none of Segoe UI, Noto Sans or DejaVu Sans is installed");
        }
        found
    }

    fn part_at(part: Part, x: i32, y: i32) -> u32 {
        let st = super::super::style(&Settings::default());
        let img = st.part(part).unwrap();
        let f = img.frame(State::Normal);
        img.atlas.get(f.x + x, f.y + y)
    }

    const LETS_THROUGH: u32 = 153;

    #[test]
    fn the_pieces_on_the_glass_let_it_through() {
        let mut p = sample(Mode::Home, None);
        for place in &mut p.places {
            place.label.clear();
        }
        type Case = (
            Spot,
            Option<Spot>,
            Option<Spot>,
            Part,
            State,
            &'static [(i32, i32)],
        );
        let place = Spot::Place(1);
        let (shut, arrow) = (Some(Spot::Shutdown), Some(Spot::ShutdownArrow));
        let cases: [Case; 5] = [
            (
                place,
                None,
                Some(place),
                Part::StartHotRight,
                State::Normal,
                &[(69, 5), (5, 24), (130, 10)],
            ),
            (
                place,
                None,
                shut,
                Part::StartShutdown,
                State::Normal,
                &[(4, 4), (4, 14), (67, 8)],
            ),
            (
                Spot::Shutdown,
                None,
                shut,
                Part::StartShutdown,
                State::Hot,
                &[(4, 14), (67, 14)],
            ),
            (
                Spot::Shutdown,
                shut,
                arrow,
                Part::StartShutdownArrow,
                State::Pressed,
                &[(3, 6), (18, 15)],
            ),
            (
                place,
                None,
                None,
                Part::StartRuleRight,
                State::Normal,
                &[(10, 0), (10, 1)],
            ),
        ];
        let segoe = TextRenderer::load(&font()).has_face("Segoe UI");
        let cases = cases.iter().filter(|c| segoe || c.2 != shut);
        let ruby = super::super::SWATCHES
            .iter()
            .find(|w| w.name == "ruby")
            .unwrap();
        for (color, intensity) in [
            (Settings::default().color, Settings::default().intensity),
            (ruby.color, ruby.intensity),
        ] {
            for material in super::super::Material::ALL {
                let s = Settings {
                    color,
                    intensity,
                    material,
                    ..Settings::default()
                };
                let style = super::super::style(&s);
                let what = format!("{color:?} {material:?} at {intensity}");
                for &(hot, pressed, spot, part, state, points) in cases.clone() {
                    p.pointer(Some(hot));
                    let d = render_in(&s, &p, pressed);
                    let o = d.l.canvas;
                    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height);
                    let r = match spot {
                        Some(spot) => d.l.rect(spot).unwrap(),
                        None => *d.l.rules.iter().find(|r| r.x >= d.l.pane.right()).unwrap(),
                    };
                    let mut alone = Canvas::new(o.width as u32, o.height as u32);
                    style.draw(&mut alone, at(r), part, state, 1);
                    let mut want = Canvas::new(o.width as u32, o.height as u32);
                    style.draw(&mut want, at(d.l.panel), Part::StartPanel, State::Normal, 1);
                    style.draw(&mut want, at(r), part, state, 1);
                    for &(x, y) in points {
                        let (cx, cy) = (r.x - o.x + x, r.y - o.y + y);
                        let covers = alone.get(cx, cy) >> 24;
                        assert!(
                            covers > 0 && covers <= LETS_THROUGH,
                            "{what}, {part:?} {state:?} ({x}, {y}): covers {covers} of the glass"
                        );
                        assert_eq!(
                            d.c.get(cx, cy),
                            want.get(cx, cy),
                            "{what}, {part:?} {state:?} ({x}, {y}): the part over the glass"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn home_draws_a_white_pane_on_glass_with_the_lit_row_boxed() {
        let d = render(&sample(Mode::Home, Some(Spot::Program(1))), None);
        assert_eq!(d.l.panel.height, 480, "pins and frequent rows: 480 tall");
        assert_eq!(d.at(150, 20), WHITE, "the pane is white");
        assert_eq!(
            d.at(13, 60),
            part_at(Part::StartHotLeft, 2, 11),
            "the lit row's box"
        );
        assert_eq!(d.at(13, 98), WHITE, "the next row is not boxed");
        let glass = d.at(300, 420);
        assert_eq!(glass, dense_fill(&Settings::default()), "the places' glass");
        assert!(
            glass >> 24 < 0xFF,
            "the places column is glass: {glass:08x}"
        );
        let rule = part_at(Part::StartRuleLeft, 0, 0);
        assert_eq!(d.at(150, 88), rule, "the pinned rule");
        assert_eq!(d.at(150, 395), rule, "All Programs' rule");
        assert_eq!(
            d.at(23, 441),
            part_at(Part::StartSearchField, 3, 4),
            "the field"
        );
        let tri = part_at(Part::StartTriangleRight, 1, 5);
        assert_eq!(d.at(20, 409), over(tri, WHITE), "All Programs' triangle");
        let rim = part_at(Part::StartPictureFrame, 4, 20);
        assert!(rim >> 24 > 0xC0, "the frame's rim: {rim:08x}");
        assert_eq!(d.at(308, -6), rim, "the picture's frame over clear");
    }

    #[test]
    fn a_lit_place_swaps_the_framed_picture_for_its_own() {
        let mut p = sample(Mode::Home, Some(Spot::Place(1)));
        let mut red = Canvas::new(48, 48);
        red.fill_rect(red.bounds(), Color::rgb(250, 12, 0));
        p.places[1].picture = Some(red);
        let d = render(&p, None);
        assert_eq!(d.at(336, 4), 0xFFFA_0C00, "Documents' picture");
        assert_eq!(d.at(308, -6), 0, "no frame round it");
        let plain = render(&sample(Mode::Home, None), None);
        assert_ne!(
            d.at(268, 95),
            plain.at(268, 95),
            "Documents is lit in its glass box"
        );
    }

    #[test]
    fn a_lit_place_without_a_picture_keeps_the_framed_user_picture() {
        let d = render(&sample(Mode::Home, Some(Spot::Place(1))), None);
        assert_eq!(
            d.at(308, -6),
            part_at(Part::StartPictureFrame, 4, 20),
            "the frame stays"
        );
    }

    #[test]
    fn a_see_through_user_picture_stands_on_white() {
        let mut p = sample(Mode::Home, None);
        let mut dot = Canvas::new(48, 48);
        dot.fill_rect(Rect::new(20, 20, 8, 8), Color::rgb(250, 12, 0));
        p.user_picture = Some(dot);
        let d = render(&p, None);
        let r = d.l.rect(Spot::Picture).unwrap();
        let (x, y) = (r.x + at::PICTURE.0, r.y + at::PICTURE.1);
        assert_eq!(d.at(x, y), WHITE, "the corner is white");
        assert_eq!(d.at(x + 47, y + 47), WHITE, "so is the far corner");
        assert_eq!(
            d.at(x + 24, y + 24),
            0xFFFA_0C00,
            "the picture's own pixels"
        );
    }

    #[test]
    fn the_picture_stays_on_the_last_place_lit() {
        let mut p = sample(Mode::Home, Some(Spot::Place(1)));
        let mut red = Canvas::new(48, 48);
        red.fill_rect(red.bounds(), Color::rgb(250, 12, 0));
        p.places[1].picture = Some(red);
        p.pointer(None);
        let d = render(&p, None);
        assert_eq!(d.at(336, 4), 0xFFFA_0C00, "Documents' picture, nothing lit");
        p.pointer(Some(Spot::Program(0)));
        let d = render(&p, None);
        assert_eq!(
            d.at(308, -6),
            part_at(Part::StartPictureFrame, 4, 20),
            "a program row brings the framed picture back"
        );
    }

    #[test]
    fn a_place_that_opens_a_menu_draws_a_white_arrow() {
        let mut p = sample(Mode::Home, None);
        p.places[4].submenu = true;
        let d = render(&p, None);
        let row = d.l.rect(Spot::Place(4)).unwrap();
        let (ax, ay) = (row.x + at::PLACE_ARROW.0, row.y + at::PLACE_ARROW.1);
        let mut whole = 0;
        for y in 0..7 {
            for x in 0..4 {
                let px = d.at(ax + x, ay + y);
                match glyph::ARROW.cover(x, y, 1, 0.0) {
                    1.0 => {
                        whole += 1;
                        assert_eq!(px, 0xFF00_0000 | INK_RIGHT, "({x}, {y}) white");
                    }
                    0.0 => assert_ne!(px, 0xFF00_0000 | INK_RIGHT, "({x}, {y}) clear"),
                    _ => {}
                }
            }
        }
        assert!(whole >= 6, "the arrow's whole pixels: {whole}");
        let plain = render(&sample(Mode::Home, None), None);
        assert_ne!(plain.at(ax, ay + 3), 0xFF00_0000 | INK_RIGHT, "no arrow");
    }

    #[test]
    fn the_arrows_grow_by_whole_steps_with_the_text() {
        use crate::theme::TextUnits;
        // The width and height of the pixels that differ between `a` and `b` in `r`.
        let diff_box = |a: &Drawn, b: &Drawn, r: Rect| {
            let differ: Vec<(i32, i32)> = (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| a.at(x, y) != b.at(x, y))
                .collect();
            let span = |f: fn(&(i32, i32)) -> i32| {
                let (lo, hi) = (differ.iter().map(f).min(), differ.iter().map(f).max());
                hi.zip(lo).map_or(0, |(hi, lo)| hi - lo + 1)
            };
            (span(|p| p.0), span(|p| p.1))
        };
        let boxes = |text: TextUnits| {
            let draw = |mode: Mode, submenu: bool| {
                let mut p = sample(mode, None);
                p.places[4].submenu = submenu;
                p.text = text;
                render(&p, None)
            };
            let (home, arrow, back) = (
                draw(Mode::Home, false),
                draw(Mode::Home, true),
                draw(Mode::Programs, false),
            );
            let place = home.l.rect(Spot::Place(4)).unwrap();
            let all = home.l.rect(Spot::AllPrograms).unwrap();
            let all = Rect::new(all.x, all.y, text.len(at::ALL_PEN) - 4, all.height);
            (diff_box(&home, &arrow, place), diff_box(&home, &back, all))
        };
        assert_eq!(boxes(TextUnits::ONE), ((4, 7), (6, 11)), "the 100% sizes");
        assert_eq!(
            boxes(TextUnits::new(22, 11)),
            ((8, 14), (12, 22)),
            "twice at 200%"
        );
        assert_eq!(
            boxes(TextUnits::new(16, 11)),
            ((4, 7), (6, 11)),
            "their own size below 200%"
        );
    }

    fn typed(typed: &str, then: Option<EditKey>) -> StartPanel {
        let mut p = StartPanel::new();
        p.key(crate::startpanel::Key::Char(EditKey::Insert(typed.into())));
        if let Some(k) = then {
            p.key(crate::startpanel::Key::Char(k));
        }
        p
    }

    /// The search field's column whose caret rows invert between the blink's two halves.
    fn caret_x(p: &StartPanel) -> Option<i32> {
        let style = super::super::style(&Settings::default());
        let text = TextRenderer::load(&font());
        let l = layout(p);
        let drawn = |on| {
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            draw(&mut c, &style, &text, p, &l, None, on);
            c
        };
        let (on, off) = (drawn(true), drawn(false));
        let field = l.rect(Spot::Search).unwrap();
        let (_, cy, ch) = at::CARET;
        (field.x..field.right()).find(|&x| {
            (0..ch).all(|dy| {
                let (x, y) = (x - l.canvas.x, field.y + cy + dy - l.canvas.y);
                on.get(x, y) == 0xFF00_0000 | (!off.get(x, y) & 0x00FF_FFFF)
            })
        })
    }

    #[test]
    fn a_selected_run_in_the_search_field_is_on_the_selection_blue() {
        let field = |p: &StartPanel| {
            let d = render(p, None);
            let f = d.l.rect(Spot::Search).unwrap();
            let (_, cy, ch) = at::CARET;
            inks(&d, Rect::new(f.x, f.y + cy, at::TEXT_RIGHT, ch), 0x1A6EE0)
        };
        let shift_home = EditKey::Home { select: true };
        assert!(
            field(&typed("notepad", Some(shift_home.clone()))),
            "selected"
        );
        assert!(!field(&typed("notepad", None)), "nothing selected");
        let mut away = typed("notepad", Some(shift_home));
        away.key(crate::startpanel::Key::Tab);
        assert_ne!(away.focus(), Spot::Search);
        assert!(!field(&away), "the keyboard left the field");
    }

    #[test]
    fn a_long_query_scrolls_to_keep_the_caret_in_the_field() {
        let long = "a long search for something past the field's right edge";
        let p = typed(long, None);
        let field = layout(&p).rect(Spot::Search).unwrap();
        let x = caret_x(&p).expect("the caret shows");
        assert!(
            x > field.x + at::TEXT_RIGHT / 2 && x < field.x + 1 + at::TEXT_RIGHT,
            "the caret at the end stands near the field's right, inside it: {x}"
        );
        let home = EditKey::Home { select: false };
        // The a inks its pen's column, so the caret stands left of it.
        assert_eq!(
            caret_x(&typed(long, Some(home))),
            Some(field.x + at::CARET.0 - 1)
        );
    }

    #[test]
    fn the_caret_shows_in_the_blinks_visible_half_only() {
        let p = sample(Mode::Home, None);
        let l = layout(&p);
        let field = l.rect(Spot::Search).unwrap();
        let (cx, cy, ch) = at::CARET;
        let style = super::super::style(&Settings::default());
        let text = TextRenderer::load(&font());
        let caret_at = |on: bool| {
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            draw(&mut c, &style, &text, &p, &l, None, on);
            (0..ch)
                .map(|dy| c.get(field.x + cx, field.y + cy + dy - l.canvas.y))
                .collect::<Vec<u32>>()
        };
        let black = 0xFF00_0000 | INK_STRONG;
        assert!(caret_at(true).iter().all(|&px| px == black), "on");
        assert!(caret_at(false).iter().all(|&px| px != black), "off");
    }

    #[test]
    fn a_folder_row_draws_its_icon_where_a_top_level_app_does() {
        let mut p = sample(Mode::Home, None);
        let mut red = Canvas::new(16, 16);
        red.fill_rect(red.bounds(), Color::rgb(250, 12, 0));
        p.tree = vec![TreeRow::Folder {
            name: "Accessories".into(),
            open: false,
            icon: Some(red),
        }];
        p.set_mode(Mode::Programs);
        let d = render(&p, None);
        let row = d.l.rect(Spot::Tree(0)).unwrap();
        let (x, y) = (row.x + at::TREE_ICON.0, row.y + at::TREE_ICON.1);
        assert_eq!(d.at(x, y), 0xFFFA_0C00);
        assert_eq!(d.at(x + 15, y + 15), 0xFFFA_0C00);
        assert_eq!(d.at(x + 16, y + 8), WHITE, "16 px wide");
    }

    /// Shut down's two halves in `p` at scale `d`: the button, then the arrow.
    fn shut_down_halves(
        s: &Settings,
        p: &StartPanel,
        pressed: Option<Spot>,
        d: i32,
    ) -> [Canvas; 2] {
        let style = super::super::style(s).at_scale(d);
        let text = TextRenderer::load(&font());
        let l = layout(p).scaled(d);
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        draw(&mut c, &style, &text, p, &l, pressed, true);
        [Spot::Shutdown, Spot::ShutdownArrow].map(|s| {
            let r = l.rect(s).unwrap();
            c.copy_rect(Rect::new(
                r.x - l.canvas.x,
                r.y - l.canvas.y,
                r.width,
                r.height,
            ))
        })
    }

    fn keyed(mode: Mode, to: Spot) -> StartPanel {
        let mut p = sample(mode, None);
        assert_eq!(
            p.focus_on(to),
            crate::startpanel::Event::Redraw,
            "{to:?} takes the keys"
        );
        p
    }

    #[test]
    fn a_key_lights_the_shut_down_half_it_stands_on_and_the_pointer_both() {
        lights_shut_down(&Settings::default(), "glass/start_shut_down");
        lights_shut_down(&dark(), "glass/start_shut_down_dark");
    }

    fn lights_shut_down(s: &Settings, pin: &str) {
        let states = [
            ("rest", sample(Mode::Home, None), None),
            (
                "pointer",
                sample(Mode::Home, Some(Spot::ShutdownArrow)),
                None,
            ),
            (
                "held",
                sample(Mode::Home, Some(Spot::Shutdown)),
                Some(Spot::Shutdown),
            ),
            ("key button", keyed(Mode::Home, Spot::Shutdown), None),
            ("key arrow", keyed(Mode::Home, Spot::ShutdownArrow), None),
            ("search", sample(Mode::Search, None), None),
            (
                "search pointer",
                sample(Mode::Search, Some(Spot::Shutdown)),
                None,
            ),
            (
                "search key button",
                keyed(Mode::Search, Spot::Shutdown),
                None,
            ),
            (
                "search key arrow",
                keyed(Mode::Search, Spot::ShutdownArrow),
                None,
            ),
        ];
        let mut got = Vec::new();
        for d in [1, 2] {
            let drawn: Vec<[Canvas; 2]> = states
                .iter()
                .map(|(_, p, pressed)| shut_down_halves(s, p, *pressed, d))
                .collect();
            let lit =
                |k: usize, rest: usize| [0, 1].map(|half| drawn[k][half] != drawn[rest][half]);
            assert_eq!(
                lit(1, 0),
                [true, true],
                "the pointer lights both halves at {d}"
            );
            assert_eq!(lit(2, 0), [true, true], "a press holds both halves at {d}");
            assert_eq!(lit(2, 1), [true, true], "held is not hot at {d}");
            assert_eq!(
                lit(3, 0),
                [true, false],
                "a key on the button lights it alone at {d}"
            );
            assert_eq!(
                lit(4, 0),
                [false, true],
                "a key on the arrow lights it alone at {d}"
            );
            assert_eq!(
                lit(6, 5),
                [false, false],
                "the light button stays at rest under the pointer at {d}"
            );
            assert_eq!(
                lit(7, 5),
                [true, false],
                "a key on the light button lights it alone at {d}"
            );
            assert_eq!(
                lit(8, 5),
                [false, true],
                "a key on the light arrow lights it alone at {d}"
            );
            for ((name, ..), [button, arrow]) in states.iter().zip(&drawn) {
                got.push((format!("{name} button {d}"), fnv(button)));
                got.push((format!("{name} arrow {d}"), fnv(arrow)));
            }
        }
        crate::pins::check(pin, &got);
    }

    #[test]
    fn the_power_button_reads_its_action() {
        let mut got = Vec::new();
        let mut drawn = Vec::new();
        for d in [1, 2] {
            for b in crate::startpanel::PowerButton::ALL {
                let mut p = sample(Mode::Home, None);
                p.power = b;
                let [button, arrow] = shut_down_halves(&Settings::default(), &p, None, d);
                got.push((format!("{} {d}", b.name()), fnv(&button)));
                drawn.push((button, arrow));
            }
        }
        let (rest, rest_arrow) = &drawn[0];
        for (button, arrow) in &drawn[1..4] {
            assert_ne!(button, rest, "each action has its own label");
            assert_eq!(arrow, rest_arrow, "the arrow stays");
        }
        crate::pins::check("glass/start_power_button", &got);
    }

    fn dark() -> Settings {
        Settings {
            mode: crate::glass::Mode::Dark,
            ..Settings::default()
        }
    }

    /// Dark Glass's panel at scale 1 and 2: Home with a program lit, the tree with a row lit,
    /// Search "no" with its first result lit, a jump list with a row lit, Shut down keyed in
    /// Search. The pane holds no white, and the lit row's name is in dark Glass's text.
    #[test]
    fn dark_panels_pin_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut programs = sample(Mode::Home, None);
        programs.tree = ["Calculator", "Notepad", "Paint"]
            .map(|name| TreeRow::App {
                name: name.into(),
                icon: None,
                depth: 0,
            })
            .into();
        programs.set_mode(Mode::Programs);
        programs.pointer(Some(Spot::Tree(1)));
        let states = [
            (
                "home",
                sample(Mode::Home, Some(Spot::Program(2))),
                Spot::Program(2),
            ),
            ("programs", programs, Spot::Tree(1)),
            ("search", sample(Mode::Search, None), Spot::Result(0, 0)),
            (
                "jump",
                sample(Mode::Jump(1), Some(Spot::Jump(1))),
                Spot::Jump(1),
            ),
            (
                "search key shut down",
                keyed(Mode::Search, Spot::Shutdown),
                Spot::Shutdown,
            ),
        ];
        let text = TextRenderer::load(&font());
        let ink = super::super::DARK_TEXT.argb() & 0xFF_FFFF;
        let mut got = Vec::new();
        for d in [1, 2] {
            let style = super::super::style(&dark()).at_scale(d);
            for (name, p, lit) in &states {
                let l = layout(p).scaled(d);
                let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
                draw(&mut c, &style, &text, p, &l, None, true);
                let drawn = Drawn { c, l };
                let pane = drawn.l.pane;
                assert!(
                    !inks(&drawn, pane, 0xFF_FFFF),
                    "{name} at {d}: no white on the pane"
                );
                let row = drawn.l.rect(*lit).unwrap();
                assert!(
                    inks_near(&drawn, row, ink),
                    "{name} at {d}: the lit row's name in dark Glass's text"
                );
                got.push((format!("{name} {d}"), fnv(&drawn.c)));
            }
        }
        crate::pins::check("glass/start_panel_dark", &got);
    }

    #[test]
    fn programs_mode_draws_back_and_the_tree_rows() {
        let mut p = sample(Mode::Home, None);
        p.tree = vec![
            TreeRow::Folder {
                name: "Accessories".into(),
                open: true,
                icon: None,
            },
            TreeRow::App {
                name: "Calculator".into(),
                icon: None,
                depth: 1,
            },
            TreeRow::App {
                name: "Notepad".into(),
                icon: None,
                depth: 1,
            },
        ];
        p.set_mode(Mode::Programs);
        p.pointer(Some(Spot::Tree(2)));
        let d = render(&p, None);
        let (dx, dy) = (1, 5);
        let left = part_at(Part::StartTriangleLeft, dx, dy);
        assert_ne!(left, part_at(Part::StartTriangleRight, dx, dy));
        assert_eq!(
            d.at(19 + dx, 404 + dy),
            over(left, WHITE),
            "Back's triangle"
        );
        let corner = over(part_at(Part::StartHotLeft, 2, 1), WHITE);
        assert_eq!(d.at(13, 49), corner, "the lit tree row");
        assert_eq!(d.at(13, 30), WHITE, "the folder row is not lit");
        assert!(d.l.picture.is_some(), "the places and picture stay");
        if face() {
            let name = Rect::new(52, 34, 40, 10);
            assert!(inks(&d, name, INK_STRONG), "Calculator, indented");
        }
    }

    #[test]
    fn search_mode_draws_group_headers_in_blue() {
        let d = render(&sample(Mode::Search, None), None);
        assert_eq!(d.l.pane.right(), 402, "the pane spans the right column");
        assert_eq!(
            d.at(27, 52),
            part_at(Part::StartLitResult, 2, 5),
            "the lit result"
        );
        let rule = d.at(380, 32);
        assert_eq!(
            rule,
            part_at(Part::StartSearchRule, 0, 0),
            "the header's rule"
        );
        assert_eq!(d.at(300, 100), WHITE, "no places over the pane");
        assert_eq!(d.at(269, 449), part_at(Part::StartShutdownLight, 2, 11));
        if face() {
            let header = Rect::new(17, 26, 80, 11);
            assert!(inks(&d, header, INK_GROUP), "the header's stems");
            assert!(!inks(&d, Rect::new(49, 50, 60, 12), INK_GROUP));
        }
    }

    #[test]
    fn a_result_draws_its_detail_in_grey_after_its_name() {
        let mut p = sample(Mode::Search, None);
        let bare = render(&p, None);
        p.results[1].rows[0].detail = "~/Documents".into();
        let d = render(&p, None);
        let r = d.l.rect(Spot::Result(1, 0)).unwrap();
        let name_end = r.x + 80;
        let before = Rect::new(r.x, r.y, name_end - r.x, r.height);
        let after = Rect::new(name_end, r.y, r.right() - name_end, r.height);
        assert!(
            (before.y..before.bottom())
                .all(|y| (before.x..before.right()).all(|x| d.at(x, y) == bare.at(x, y))),
            "the name and icon stay as they were"
        );
        if face() {
            assert!(inks_near(&d, after, INK_GREYED), "the folder in grey");
            assert!(!inks_near(&bare, after, INK_GREYED), "nothing without one");
        }
    }

    #[test]
    fn jump_mode_draws_the_light_pane_and_keeps_the_program_lit() {
        let mut p = sample(Mode::Jump(3), None);
        p.pointer(Some(Spot::Jump(1)));
        let d = render(&p, None);
        assert_eq!(d.l.panel.width, 491);
        assert_eq!(
            d.at(261, 109),
            part_at(Part::StartJumpPane, 2, 100),
            "the jump pane"
        );
        assert_eq!(
            d.at(13, 140),
            part_at(Part::StartHotLeft, 2, 11),
            "its row stays lit"
        );
        assert_eq!(
            d.at(230, 140),
            part_at(Part::StartHotSplit, 1, 11),
            "the split"
        );
        assert_eq!(
            d.at(276, 62),
            part_at(Part::StartHotJump, 2, 5),
            "the lit jump row"
        );
        let (outline, light) = frame_edges(&Settings::default());
        assert_eq!(d.at(489, 200), light, "the light line");
        assert_eq!(d.at(490, 200), outline, "the outline");
        if face() {
            let recent = Rect::new(274, 16, 40, 9);
            assert!(inks_near(&d, recent, INK_HEADER), "Recent");
            assert!(!inks_near(&d, recent, INK_STRONG), "not in black");
        }
    }

    /// Tasks stand under their own title and rule below the recent files, a lit task boxed as a
    /// lit file is; pinned at scale 1 and 2, at rest and with the last task lit.
    #[test]
    fn a_jump_lists_tasks_draw_under_their_own_title() {
        crate::pins::fonts();
        let mut p = sample(Mode::Jump(3), None);
        p.jump = vec![
            section("Recent", &["notes.txt", "todo.txt"]),
            section("Tasks", &["New Window", "Compose Message"]),
        ];
        let text = TextRenderer::load(&font());
        let mut got = Vec::new();
        for (name, lit) in [("tasks", None), ("task lit", Some(Spot::Jump(3)))] {
            let mut p = p.clone();
            p.pointer(lit);
            for d in [1, 2] {
                let style = super::super::style(&Settings::default()).at_scale(d);
                let l = layout(&p).scaled(d);
                let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
                draw(&mut c, &style, &text, &p, &l, None, true);
                let drawn = Drawn { c, l };
                let (tasks, head) = drawn.l.headers[1];
                assert_eq!(tasks, 1);
                let (rx, ry) = (head.right() - 2 * d, head.y + at::JUMP_RULE_Y * d);
                assert_ne!(
                    drawn.at(rx, ry),
                    drawn.at(rx, ry - 3 * d),
                    "{name} at {d}: Tasks has a rule"
                );
                let title = Rect::new(head.x, head.y + 4 * d, 30 * d, 9 * d);
                if face() {
                    assert!(inks_near(&drawn, title, INK_HEADER), "{name} at {d}: Tasks");
                }
                if lit.is_some() && d == 1 {
                    let r = drawn.l.rect(Spot::Jump(3)).unwrap();
                    assert_eq!(
                        drawn.at(r.x + 2, r.y + 6),
                        part_at(Part::StartHotJump, 2, 5),
                        "the lit task is boxed"
                    );
                }
                got.push((format!("{name} {d}"), fnv(&drawn.c)));
            }
        }
        assert_ne!(got[0].1, got[2].1, "the lit task differs from the rest");
        crate::pins::check("glass/start_panel_tasks", &got);
    }

    #[test]
    fn a_jump_panel_narrowed_to_its_room_keeps_its_header_rule_and_frame_inside() {
        let mut p = sample(Mode::Jump(3), None);
        let wide = render(&p, None);
        p.room = Some((450, 1000));
        let d = render(&p, None);
        assert_eq!(d.l.panel.width, 450);
        let ry = d.l.headers[0].1.y + at::JUMP_RULE_Y;
        let rule = wide.at(479, ry);
        assert_eq!(d.at(438, ry), rule, "the rule ends a pixel past the header");
        assert_ne!(d.at(439, ry), rule);
        let (outline, light) = frame_edges(&Settings::default());
        assert_eq!(d.at(448, 200), light, "the light line");
        assert_eq!(d.at(449, 200), outline, "the outline");
    }

    #[test]
    fn a_home_panel_narrowed_to_its_room_keeps_its_frame_and_place_arrows_inside() {
        let mut p = sample(Mode::Home, None);
        p.places[4].submenu = true;
        p.fit(crate::theme::TextUnits::ONE, (400, 1000));
        let d = render(&p, None);
        assert_eq!(d.l.panel.width, 400);
        let (outline, light) = frame_edges(&Settings::default());
        assert_eq!(d.at(398, 200), light, "the light line");
        assert_eq!(d.at(399, 200), outline, "the outline");
        let r = d.l.rect(Spot::Place(4)).unwrap();
        assert_eq!(r.right(), 393, "the row keeps its 7 px from the edge");
        if face() {
            let arrow = Rect::new(r.x + at::PLACE_ARROW.0 - 11, r.y, 8, r.height);
            assert!(
                inks_near(&d, arrow, INK_RIGHT),
                "the arrow moves in with the row"
            );
        }
    }

    #[test]
    fn a_320_px_panel_keeps_its_frame_split_and_arrows_inside() {
        let mut p = sample(Mode::Home, None);
        p.places[4].submenu = true;
        p.fit(crate::theme::TextUnits::ONE, (320, 1000));
        let d = render(&p, None);
        assert_eq!(d.l.panel.width, 320);
        let (outline, light) = frame_edges(&Settings::default());
        assert_eq!(d.at(318, 200), light, "the light line");
        assert_eq!(d.at(319, 200), outline, "the outline");
        let r = d.l.rect(Spot::Place(4)).unwrap();
        assert_eq!(r.right(), 313, "the row keeps its 7 px from the edge");
        if face() {
            let arrow = Rect::new(r.x + at::PLACE_ARROW.0 - 40, r.y, 8, r.height);
            assert!(inks_near(&d, arrow, INK_RIGHT), "the place's arrow");
        }

        p.set_mode(Mode::Jump(3));
        p.pointer(Some(Spot::Jump(1)));
        let d = render(&p, None);
        assert_eq!(d.l.panel.width, 320);
        assert_eq!(
            d.at(179, 140),
            part_at(Part::StartHotSplit, 1, 11),
            "the split moves in with the row's right"
        );
        assert_eq!(
            d.at(210, 109),
            part_at(Part::StartJumpPane, 2, 100),
            "the jump pane"
        );
        assert_eq!(
            d.at(225, 62),
            part_at(Part::StartHotJump, 2, 5),
            "the lit jump row"
        );
        assert_eq!(d.at(318, 200), light, "the light line");
        assert_eq!(d.at(319, 200), outline, "the outline");
    }

    #[test]
    fn closed_up_places_keep_their_text_and_arrow_inside_their_rows() {
        let mut p = sample(Mode::Home, None);
        p.places[4].submenu = true;
        p.room = Some((1024, 300));
        let plain = render(&p, None);
        p.pointer(Some(Spot::Place(5)));
        let d = render(&p, None);
        let row = d.l.rect(Spot::Place(5)).expect("Computer shows");
        assert_eq!(row.height, 20, "closed up");
        let mid = (row.x + 2, row.y + row.height / 2);
        assert_ne!(d.at(mid.0, mid.1), plain.at(mid.0, mid.1), "the row lights");
        if face() {
            for k in [4, 5] {
                let r = d.l.rect(Spot::Place(k)).unwrap();
                let inside = Rect::new(r.x, r.y, r.width, r.height);
                let gap = Rect::new(r.x, r.bottom(), r.width, 2);
                assert!(inks_near(&d, inside, INK_RIGHT), "place {k} inks its row");
                assert!(
                    !inks_near(&d, gap, INK_RIGHT),
                    "place {k} keeps out of the gap"
                );
            }
            let r = d.l.rect(Spot::Place(4)).unwrap();
            let arrow = Rect::new(r.x + at::PLACE_ARROW.0, r.y, 8, r.height);
            assert!(
                inks_near(&d, arrow, INK_RIGHT),
                "the arrow stands in its row"
            );
        }
    }

    /// At 200% the rows double with their text: the lit row's box runs down the grown row, a
    /// name's ink stands twice as tall and the panel is twice the size.
    #[test]
    fn at_200_percent_the_rows_and_their_text_grow() {
        let ink_rows = |d: &Drawn, r: Rect| {
            (r.y..r.bottom())
                .filter(|&y| {
                    (r.x..r.right()).any(|x| {
                        let px = d.at(x, y);
                        [16, 8, 0].iter().all(|&sh| (px >> sh) & 0xFF < 0x60)
                    })
                })
                .count()
        };
        let small = render(&sample(Mode::Home, Some(Spot::Program(1))), None);
        let mut p = sample(Mode::Home, Some(Spot::Program(1)));
        p.text = crate::theme::TextUnits::new(22, 11);
        let d = render(&p, None);
        assert_eq!(d.l.panel.height, 2 * small.l.panel.height);
        let lit = d.l.rect(Spot::Program(1)).unwrap();
        assert_ne!(
            d.at(lit.x + 4, lit.bottom() - 6),
            WHITE,
            "the box runs down the row"
        );
        let next = d.l.rect(Spot::Program(2)).unwrap();
        assert_eq!(
            d.at(next.x + 4, next.y + 40),
            WHITE,
            "the next row is not boxed"
        );
        if face() {
            let at_100 = ink_rows(&small, small.l.rect(Spot::Program(2)).unwrap());
            let at_200 = ink_rows(&d, next);
            assert!(
                at_200 >= 2 * at_100 - 2 && at_100 > 0,
                "{at_200} ink rows at 200%, {at_100} at 100%"
            );
        }
    }

    /// A kept frame drawn at 100% is not reused for the grown panel: the backed draw at 200%
    /// matches a fresh one.
    #[test]
    fn a_backed_draw_follows_the_text_size() {
        let text = TextRenderer::load(&font());
        let style = super::super::style(&Settings::default());
        let draw_at = |back: Option<&mut Backing>, p: &StartPanel| {
            let l = layout(p);
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            match back {
                Some(b) => draw_backed(b, &mut c, &style, &text, p, &l, None, true),
                None => draw(&mut c, &style, &text, p, &l, None, true),
            }
            c.pixels
        };
        let home = sample(Mode::Home, None);
        let mut grown = sample(Mode::Home, None);
        grown.text = crate::theme::TextUnits::new(22, 11);
        let mut back = Backing::default();
        draw_at(Some(&mut back), &home);
        assert_eq!(draw_at(Some(&mut back), &grown), draw_at(None, &grown));
    }

    #[test]
    fn the_faces_capitals_stand_9_and_11_rows_tall() {
        let text = TextRenderer::load(&font());
        if !text.has_face("Segoe UI") {
            crate::pins::skip!(face "Segoe UI");
        }
        let rows = |px: u32| {
            let mut c = Canvas::new(40, 40);
            let all = c.bounds();
            text.draw_run(&mut c, 2, 30, all, "H", false, Color::rgb(0, 0, 0), px);
            (0..40)
                .filter(|&y| (0..40).any(|x| c.get(x, y) >> 24 > 0x80))
                .count()
        };
        assert_eq!(rows(TEXT_PX), 9, "the panel's lines");
        assert_eq!(rows(HEADER_PX), 11, "a search group's header");
    }

    #[test]
    fn without_glass_the_panel_is_opaque() {
        let p = sample(Mode::Home, Some(Spot::Place(2)));
        let l = layout(&p);
        let text = interfaces::start_panel_text(&interfaces::theme_for(Interface::Classic));
        let opaque_glass = Settings {
            transparency: false,
            ..Settings::default()
        };
        let looks = [
            (
                interfaces::theme_for(Interface::Classic),
                Settings::default(),
            ),
            (interfaces::theme_for(Interface::Blue), Settings::default()),
            (
                interfaces::theme_with(
                    Interface::Glass,
                    crate::blue::Variant::Blue,
                    false,
                    &opaque_glass,
                ),
                opaque_glass,
            ),
        ];
        let corner = [5, 3, 2, 1, 1];
        for (theme, glass) in looks {
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            interfaces::draw_start_panel(&mut c, &theme, &glass, &text, &p, &l, None, true);
            for y in 0..l.panel.height {
                let cut = corner.get(y as usize).copied().unwrap_or(0);
                for x in cut..l.panel.width - cut {
                    let px = c.get(x, y - l.canvas.y);
                    assert_eq!(px >> 24, 0xFF, "{:?} ({x}, {y}): {px:08x}", theme.look);
                }
            }
        }
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        let glass = interfaces::theme_for(Interface::Glass);
        let d = Settings::default();
        interfaces::draw_start_panel(&mut c, &glass, &d, &text, &p, &l, None, true);
        assert!(
            c.get(300, 420 - l.canvas.y) >> 24 < 0xFF,
            "Glass is translucent"
        );
    }

    #[test]
    fn a_settings_result_draws_its_keyword_after_the_page_in_every_look() {
        crate::pins::fonts();
        let text = TextRenderer::load(&font());
        let with = |name: &str, detail: &str| {
            let mut p = sample(Mode::Search, None);
            p.set_results(vec![ResultGroup {
                title: "Settings (1)".into(),
                rows: vec![ResultRow {
                    name: name.into(),
                    icon: None,
                    detail: detail.into(),
                }],
            }]);
            p
        };
        let blank = with("", "");
        let bare = with("Advanced", "");
        let keyed = with("Advanced", "low disk space");
        let differ = |a: &Drawn, b: &Drawn| -> Vec<(i32, i32)> {
            (0..b.c.height as i32)
                .flat_map(|y| (0..b.c.width as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| a.c.get(x, y) != b.c.get(x, y))
                .map(|(x, y)| (x + b.l.canvas.x, y + b.l.canvas.y))
                .collect()
        };
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for d in [1, 2] {
                let theme = interfaces::theme_for(look).scaled(d);
                let glass = Settings::default();
                let draw = |p: &StartPanel| {
                    let l = layout(p).scaled(d);
                    let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
                    interfaces::draw_start_panel(&mut c, &theme, &glass, &text, p, &l, None, true);
                    Drawn { c, l }
                };
                let (blank, bare, keyed) = (draw(&blank), draw(&bare), draw(&keyed));
                let row = keyed.l.rect(Spot::Result(0, 0)).expect("the result row");
                let name_end = (differ(&blank, &bare).iter())
                    .map(|&(x, _)| x)
                    .max()
                    .expect("the name draws");
                let keyword = differ(&bare, &keyed);
                assert!(!keyword.is_empty(), "{look:?} at {d}: the keyword draws");
                assert!(
                    (keyword.iter()).all(|&(x, y)| row.contains(x, y) && x > name_end),
                    "{look:?} at {d}: the keyword stays in the row, right of the name"
                );
                got.push((format!("{look:?} {d}"), fnv(&keyed.c)));
            }
        }
        crate::pins::check("glass/start_panel_setting_keyword", &got);
    }

    #[test]
    fn glass_has_every_start_panel_part() {
        let st = super::super::style(&Settings::default());
        for (part, _) in parts(&Settings::default(), 1) {
            assert!(st.part(part).is_some(), "{part:?}");
        }
    }

    fn onyx() -> Settings {
        Settings {
            style: crate::glass::Style::Onyx,
            ..Settings::default()
        }
    }

    /// [`sample`] as the Onyx menu: no jump lists, a grey line under each pinned name.
    fn onyx_sample(mode: Mode, hot: Option<Spot>) -> StartPanel {
        let mut p = sample(mode, None);
        p.shape = Shape::Onyx;
        p.jump.clear();
        for (k, r) in p.pinned.iter_mut().enumerate() {
            r.detail = Some(format!("Pinned {k}"));
        }
        for r in &mut p.frequent {
            r.jump = false;
        }
        p.pointer(hot);
        p
    }

    /// The Onyx panel at scale 1 and 2: Home with a program lit, a place lit (its picture in the
    /// frame's place), the tree, Search beside the places, the power button hot, the lock button
    /// keyed with the arrow held. Its place text is white and its pinned names bold over grey.
    #[test]
    fn onyx_panels_pin_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut programs = onyx_sample(Mode::Home, None);
        programs.tree = ["Calculator", "Notepad", "Paint"]
            .map(|name| TreeRow::App {
                name: name.into(),
                icon: None,
                depth: 0,
            })
            .into();
        programs.set_mode(Mode::Programs);
        programs.pointer(Some(Spot::Tree(1)));
        let mut keyed = onyx_sample(Mode::Home, None);
        keyed.focus_on(Spot::Lock);
        let mut place_lit = onyx_sample(Mode::Home, None);
        place_lit.places[1].picture = Some({
            let mut c = Canvas::new(48, 48);
            c.fill_rect(Rect::new(8, 8, 32, 32), Color::rgb(230, 200, 90));
            c
        });
        place_lit.pointer(Some(Spot::Place(1)));
        let states = [
            (
                "home",
                onyx_sample(Mode::Home, Some(Spot::Program(3))),
                None,
            ),
            ("place", place_lit, None),
            ("programs", programs, None),
            ("search", onyx_sample(Mode::Search, None), None),
            (
                "power hot",
                onyx_sample(Mode::Home, Some(Spot::Shutdown)),
                None,
            ),
            ("lock keyed arrow held", keyed, Some(Spot::ShutdownArrow)),
        ];
        let text = TextRenderer::load(&font());
        let mut got = Vec::new();
        for d in [1, 2] {
            let style = super::super::style(&onyx()).at_scale(d);
            for (name, p, pressed) in &states {
                let l = layout(p).scaled(d);
                let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
                draw(&mut c, &style, &text, p, &l, *pressed, true);
                let drawn = Drawn { c, l };
                let place = drawn.l.rect(Spot::Place(2)).unwrap();
                assert!(
                    inks_near(&drawn, place, 0xFF_FFFF),
                    "{name} at {d}: white places"
                );
                assert_eq!(drawn.at(0, 0), 0, "{name} at {d}: the top corner is clear");
                got.push((format!("{name} {d}"), fnv(&drawn.c)));
            }
        }
        let home = &states[0].1;
        let drawn = render_in(&onyx(), home, None);
        let pinned = drawn.l.rect(Spot::Program(0)).unwrap();
        let grey = super::super::startpanel::INK_GREYED;
        assert!(
            inks_near(&drawn, pinned, grey),
            "the pinned row's grey line"
        );
        crate::pins::check("glass/onyx_start_panel", &got);
    }

    #[test]
    fn onyx_menu_draws_the_onyx_menu_in_its_mode_whatever_the_glass_mode() {
        use crate::glass::Mode::{Dark, Light};
        let p = onyx_sample(Mode::Home, Some(Spot::Program(3)));
        let l = layout(&p);
        let text = TextRenderer::load(&font());
        let drawn = |mode, onyx_menu| {
            let glass = Settings {
                mode,
                onyx_menu,
                ..onyx()
            };
            let dark = mode == Dark;
            let theme =
                interfaces::theme_with(Interface::Glass, crate::blue::Variant::Blue, dark, &glass);
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            interfaces::draw_start_panel(&mut c, &theme, &glass, &text, &p, &l, None, true);
            c.pixels
        };
        assert_eq!(
            drawn(Dark, Some(Light)),
            drawn(Light, None),
            "light in dark mode"
        );
        assert_eq!(
            drawn(Light, Some(Dark)),
            drawn(Dark, None),
            "dark in light mode"
        );
        assert_ne!(
            drawn(Dark, None),
            drawn(Light, None),
            "left out, it follows the mode"
        );
    }
}
