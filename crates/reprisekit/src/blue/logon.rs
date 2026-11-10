//! Blue's welcome screen: dark bands top and bottom, the Reprise mark left of a middle rule, a
//! column of user tiles right of it, and Turn off computer in the bottom band. The lock opens its
//! one tile and puts Switch User at the bottom band's right end.

use std::cell::RefCell;
use std::rc::Rc;

use crate::art::{logo, userpic};
use crate::blue::Variant;
use crate::blue::powerpanel::{self, Glaze, Hue, Mark, RIM, TileState, glaze, title_text};
use crate::blue::scheme::Scheme;
use crate::logon::{Logon, LogonLayout, Part, Screen, UserNote, message_lines, text as strings};
use crate::paint::{Paint, draw_edit};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::shape::{
    self, CLEAR, Px, Stops, centre, ellipse, over, premul, radial, rounded, rounded4, shares,
    stroke, unpack,
};
use crate::text::{Align, TextRenderer, split_mnemonic};
use crate::textedit::{Caret, TextEdit, edit_first_visible};
use crate::theme::{Color, TextUnits, Theme};

/// Design px. The bands, the lit rule along each one's inner edge, and how far the middle rule
/// stops short of them; the gap either side of the middle rule.
const TOP_H: i32 = 72;
const BOTTOM_H: i32 = 84;
const RULE: i32 = 2;
const SPLIT_INSET: i32 = 48;
const SIDE_GAP: i32 = 24;
/// The glow's centre and reach as shares of the output's width and the middle's height.
const GLOW_CENTRE: (f64, f64) = (0.2, 0.3);
const GLOW_REACH: (f64, f64) = (0.6, 1.1);
/// The top rule's alpha along it, brightest over the glow; the middle rule's peak alpha.
const TOP_RULE: [(f64, u8); 3] = [(0.0, 90), (0.3, 255), (1.0, 40)];
const SPLIT_PEAK: u8 = 210;
/// Over a picture, the field is black at this alpha, so the white text reads on a bright one.
const PICTURE_DIM: u8 = 102;
const LOGO: i32 = 48;
const LOGO_GAP: i32 = 10;
const BEGIN_GAP: i32 = 10;
/// The wordmark: the product's name, never translated, at this many times the title's size.
const MARK: &str = "Reprise";
const MARK_SIZE: (u32, u32) = (3, 2);
const TILE_W: i32 = 330;
const TILE_PAD: i32 = 6;
const TILE_GAP: i32 = 6;
const PICTURE: i32 = 48;
const FRAME: i32 = 3;
const FRAMED: i32 = PICTURE + 2 * FRAME;
const FRAME_R: f64 = 6.0;
const TILE_H: i32 = FRAMED + 2 * TILE_PAD;
const TEXT_GAP: i32 = 10;
/// The hot band's left corners, and its alpha where it starts fading and where it is gone.
const BAND_R: f64 = 6.0;
const BAND_FADE: [(f64, u8); 3] = [(0.0, 255), (0.55, 200), (1.0, 0)];
const NAME_PX: u32 = 16;
const NAME_LINE: i32 = 22;
const LINE: i32 = 16;
const FIELD_W: i32 = 170;
const FIELD_H: i32 = 22;
const FIELD_R: f64 = 3.0;
const FIELD_PAD: i32 = 4;
const ROW_GAP: i32 = 4;
const GO: i32 = 22;
const GO_GAP: i32 = 6;
/// The arrow's shaft and head in the button's design px, and the gloss's centre and radii.
const GO_SHAFT: [(f64, f64); 2] = [(6.0, 11.0), (15.0, 11.0)];
const GO_HEAD: [(f64, f64); 3] = [(11.5, 6.5), (16.0, 11.0), (11.5, 15.5)];
const GO_GLOSS: ((f64, f64), (f64, f64)) = ((11.0, 6.0), (9.0, 6.0));
const BADGE: i32 = 16;
const BADGE_R: f64 = 3.0;
const MESSAGE_LINES: usize = 2;
const POWER: i32 = 26;
const POWER_X: i32 = 24;
const POWER_GAP: i32 = 8;

/// The output's bands and middle, and the middle rule's column.
struct Frame {
    middle: Rect,
    bottom: Rect,
    split: i32,
}

fn frame(o: Rect) -> Frame {
    let bottom = Rect::new(o.x, o.bottom() - BOTTOM_H, o.width, BOTTOM_H);
    Frame {
        middle: Rect::new(o.x, o.y + TOP_H, o.width, bottom.y - o.y - TOP_H),
        bottom,
        split: o.x + o.width / 2,
    }
}

/// The text column's lines at the theme's text size.
#[derive(Clone, Copy)]
struct Lines {
    name: i32,
    line: i32,
    field: i32,
}

fn lines(u: TextUnits) -> Lines {
    Lines {
        name: u.len(NAME_LINE),
        line: u.len(LINE),
        field: u.len(FIELD_H),
    }
}

fn noted(m: &Logon, i: usize) -> bool {
    m.users.get(i).and_then(|u| u.note).is_some()
}

/// How tall a tile stands open: its name and note, the cue, the field and the message.
fn open_height(noted: bool, k: Lines) -> i32 {
    let note = if noted { k.line } else { 0 };
    let text = k.name + note + k.line + k.field + ROW_GAP + MESSAGE_LINES as i32 * k.line;
    TILE_H.max(2 * TILE_PAD + text)
}

/// The field row's parts from `field`: the field, the arrow beside it and the badge after that.
fn field_row(m: &Logon, field: Rect, parts: &mut Vec<(Part, Rect)>) {
    let go = Rect::new(
        field.right() + GO_GAP,
        field.y + (field.height - GO) / 2,
        GO,
        GO,
    );
    parts.push((Part::Password, field));
    parts.push((Part::Submit, go));
    if m.layout.is_some() {
        let badge = Rect::new(
            go.right() + GO_GAP,
            field.y + (field.height - BADGE) / 2,
            BADGE,
            BADGE,
        );
        parts.push((Part::Layout, badge));
    }
}

/// A field `FIELD_W` wide at `(x, y)`, narrowed so its arrow still fits before `right`.
fn field_at(x: i32, y: i32, right: i32, k: Lines) -> Rect {
    let w = FIELD_W.min(right - x - GO_GAP - GO).max(0);
    Rect::new(x, y, w, k.field)
}

/// How many tiles fit in the middle with room left for one of them, noted, to stand open.
fn fit(f: &Frame, k: Lines) -> usize {
    let room = f.middle.height - 2 * SIDE_GAP - (open_height(true, k) - TILE_H);
    ((room + TILE_GAP) / (TILE_H + TILE_GAP)).max(1) as usize
}

/// How many tiles `output` shows at once: the host's [`Logon::column`].
pub fn column(theme: &Theme, output: Rect) -> usize {
    fit(&frame(output), lines(theme.text))
}

fn tile_parts(m: &Logon, f: &Frame, k: Lines, x: i32, w: i32, parts: &mut Vec<(Part, Rect)>) {
    let n = m.users.len();
    let open = match m.screen {
        Screen::Tile(i) => Some(i),
        _ => None,
    };
    let fit = fit(f, k);
    let mut first = m.first_tile.min(n.saturating_sub(fit));
    if let Some(sel) = open.or(match m.focus {
        Part::Tile(i) => Some(i),
        _ => None,
    }) {
        first = first.min(sel).max((sel + 1).saturating_sub(fit));
    }
    let shown = first..(first + fit).min(n);
    let count = shown.len() as i32;
    let rest = count * TILE_H + (count - 1).max(0) * TILE_GAP;
    let grown = open
        .filter(|i| shown.contains(i))
        .map_or(0, |i| open_height(noted(m, i), k) - TILE_H);
    let mh = f.middle.height;
    let top = ((mh - rest) / 2)
        .min(mh - SIDE_GAP - rest - grown)
        .max(SIDE_GAP);
    let mut y = f.middle.y + top;
    for i in shown {
        if open == Some(i) {
            let tile = Rect::new(x, y, w, open_height(noted(m, i), k));
            let tx = x + TILE_PAD + FRAMED + TEXT_GAP;
            let note = if noted(m, i) { k.line } else { 0 };
            let fy = y + TILE_PAD + k.name + note + k.line;
            field_row(m, field_at(tx, fy, tile.right(), k), parts);
            parts.push((Part::Tile(i), tile));
            y = tile.bottom() + TILE_GAP;
        } else {
            parts.push((Part::Tile(i), Rect::new(x, y, w, TILE_H)));
            y += TILE_H + TILE_GAP;
        }
    }
}

/// The typed screen's block: the name's label and field, the password's label, field and arrow,
/// and the message under them.
fn typed_parts(m: &Logon, f: &Frame, k: Lines, x: i32, w: i32, parts: &mut Vec<(Part, Rect)>) {
    let block = 2 * (k.line + k.field) + 2 * ROW_GAP + MESSAGE_LINES as i32 * k.line;
    let top = f.middle.y + (f.middle.height - block) / 2;
    let name = field_at(x, top + k.line, x + w, k);
    parts.push((Part::Name, name));
    let password = field_at(x, name.bottom() + ROW_GAP + k.line, x + w, k);
    field_row(m, password, parts);
}

fn design_px(theme: &Theme) -> u32 {
    theme.fonts.size_px / theme.scale.max(1) as u32
}

pub fn layout(m: &Logon, theme: &Theme, text: &TextRenderer, output: Rect) -> LogonLayout {
    let f = frame(output);
    let k = lines(theme.text);
    let x = f.split + SIDE_GAP;
    let w = TILE_W.min(output.right() - SIDE_GAP - x);
    let mut parts = Vec::new();
    match m.screen {
        Screen::Typed => typed_parts(m, &f, k, x, w, &mut parts),
        _ => tile_parts(m, &f, k, x, w, &mut parts),
    }
    let label = text.measure(&strings::turn_off(), false, design_px(theme));
    let power = Rect::new(
        output.x + POWER_X,
        f.bottom.y + (f.bottom.height - POWER) / 2,
        POWER + POWER_GAP + label,
        POWER,
    );
    if m.offers_power() {
        parts.push((Part::Power, power));
    }
    if m.unlocking() && m.offers_switch_user() {
        let w = POWER + POWER_GAP + text.measure(&switch_label(), false, design_px(theme));
        let x = output.right() - POWER_X - w;
        parts.push((
            Part::SwitchUser,
            Rect {
                x,
                width: w,
                ..power
            },
        ));
    }
    LogonLayout {
        output,
        parts,
        dialog: None,
    }
}

/// Switch User without its access-key marker: the screen draws no underlines.
fn switch_label() -> String {
    split_mnemonic(&strings::switch_user()).0
}

fn solid(c: Color) -> Px {
    premul(c, 255)
}

/// `c` at each `(position, alpha)`.
fn faded<const N: usize>(c: Color, stops: [(f64, u8); N]) -> [(f64, Px); N] {
    stops.map(|(t, a)| (t, premul(c, a)))
}

/// The screen without its text and parts, `w` x `h` design px at `d`: the bands, their lit
/// rules (the bottom one in the scheme's accent), the field with its glow and the middle rule.
pub fn backdrop(sc: &Scheme, w: i32, h: i32, d: i32) -> Canvas {
    drawn_backdrop(sc, w, h, d, true)
}

/// [`backdrop`], with the field a [`PICTURE_DIM`] black over a picture when `field` is false.
fn drawn_backdrop(sc: &Scheme, w: i32, h: i32, d: i32, field: bool) -> Canvas {
    let f = frame(Rect::new(0, 0, w, h));
    let band = solid(sc.welcome_band);
    let top_rule = faded(sc.welcome_rule, TOP_RULE);
    let bottom_rule = faded(sc.start_menu.accent, TOP_RULE);
    let glow = [
        (0.0, solid(sc.welcome_glow)),
        (1.0, solid(sc.welcome_field)),
    ];
    let split = faded(sc.welcome_rule, [(0.0, 0), (0.5, SPLIT_PEAK), (1.0, 0)]);
    let dim = premul(Color::rgb(0, 0, 0), PICTURE_DIM);
    let (split_y0, split_y1) = (f.middle.y + SPLIT_INSET, f.middle.bottom() - SPLIT_INSET);
    let k = f64::from(d);
    let mh = f64::from(f.middle.height);
    let (cx, cy) = (
        GLOW_CENTRE.0 * f64::from(w) * k,
        (f64::from(f.middle.y) + GLOW_CENTRE.1 * mh) * k,
    );
    let (rx, ry) = (GLOW_REACH.0 * f64::from(w) * k, GLOW_REACH.1 * mh * k);
    shape::draw(w, h, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let along = (centre(x as i32, d) + 0.5) / f64::from(w);
        if row < f.middle.y - RULE || row >= f.bottom.y + RULE {
            return band;
        }
        if row < f.middle.y {
            return over(Stops(&top_rule).at(along), band);
        }
        if row >= f.bottom.y {
            return over(Stops(&bottom_rule).at(along), band);
        }
        let field = match field {
            true => radial(x, y, cx, cy, rx, ry, &Stops(&glow)),
            false => dim,
        };
        if col == f.split && (split_y0..split_y1).contains(&row) {
            let t =
                (centre(y as i32, d) + 0.5 - f64::from(split_y0)) / f64::from(split_y1 - split_y0);
            return over(Stops(&split).at(t), field);
        }
        field
    })
}

/// How many backdrops [`cached_backdrop`] keeps: one per output a greeter is likely to span.
const KEPT_BACKDROPS: usize = 4;

/// [`backdrop`] for the last few schemes and sizes asked, kept since typing repaints the whole
/// screen.
fn cached_backdrop(sc: &Scheme, w: i32, h: i32, d: i32, field: bool) -> Rc<Canvas> {
    type Kept = Vec<((&'static str, i32, i32, i32, bool), Rc<Canvas>)>;
    thread_local! {
        static KEPT: RefCell<Kept> = const { RefCell::new(Vec::new()) };
    }
    let key = (sc.name, w, h, d, field);
    KEPT.with_borrow_mut(|kept| {
        if let Some((_, c)) = kept.iter().find(|(k, _)| *k == key) {
            return Rc::clone(c);
        }
        if kept.len() == KEPT_BACKDROPS {
            kept.remove(0);
        }
        let c = Rc::new(drawn_backdrop(sc, w, h, d, field));
        kept.push((key, Rc::clone(&c)));
        c
    })
}

/// A hot or open tile's band, `w` x `h` design px at `d`: rounded on the left, fading out to the
/// right.
pub fn band(sc: &Scheme, w: i32, h: i32, d: i32) -> Canvas {
    let fade = faded(sc.welcome_tile_hot, BAND_FADE);
    let k = f64::from(d);
    let (bw, bh) = (f64::from(w) * k, f64::from(h) * k);
    let r = BAND_R * k;
    shape::draw(w, h, d, |x, y| {
        let cover = rounded4(x, y, bw, bh, [r, 0.0, 0.0, r]);
        let along = centre(x as i32, d) / f64::from(w - 1);
        shape::scale(Stops(&fade).at(along), cover)
    })
}

/// `picture` at `PICTURE` design px inside a rounded `frame`-coloured rim, at `d`.
pub fn framed(picture: &Canvas, frame: Color, d: i32) -> Canvas {
    let n = (PICTURE * d) as u32;
    let pic = picture.resample_box(n, n);
    let k = f64::from(d);
    let size = f64::from(FRAMED) * k;
    let inset = f64::from(FRAME) * k;
    let rim = solid(frame);
    shape::draw(FRAMED, FRAMED, d, |x, y| {
        let outer = rounded(x, y, size, size, FRAME_R * k);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = rounded(
            x - inset,
            y - inset,
            size - 2.0 * inset,
            size - 2.0 * inset,
            (FRAME_R - f64::from(FRAME)) * k,
        );
        let (px, py) = (x as i32 - FRAME * d, y as i32 - FRAME * d);
        let fill = if (0..n as i32).contains(&px) && (0..n as i32).contains(&py) {
            unpack(pic.get(px, py))
        } else {
            CLEAR
        };
        shares(&[(rim, outer - inner), (fill, inner)])
    })
}

/// The round green arrow that submits, `GO` design px across at `d`.
pub fn go(state: TileState, d: i32) -> Canvas {
    let Glaze {
        down,
        gloss,
        face_k,
        mark_k,
    } = glaze(Hue::Green, state);
    let k = f64::from(d);
    let r = f64::from(GO) * k / 2.0;
    let pen = (2.0 * k).max(1.0);
    let at = |pts: &[(f64, f64)]| pts.iter().map(|&(a, b)| (a * k, b * k)).collect::<Vec<_>>();
    let (shaft, head) = (at(&GO_SHAFT), at(&GO_HEAD));
    let ((gx, gy), (grx, gry)) = GO_GLOSS;
    let rim = shape::scale(premul(RIM.0, RIM.1), mark_k);
    shape::draw(GO, GO, d, |x, y| {
        let outer = ellipse(x, y, r, r, r, r);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = ellipse(x, y, r, r, r - k, r - k);
        let t = (centre(y as i32, d) + 0.5) / f64::from(GO);
        let shine = radial(x, y, gx * k, gy * k, grx * k, gry * k, &Stops(&gloss));
        let fill = shape::scale(over(shine, Stops(&down).at(t)), face_k);
        let mark = stroke(x, y, &shaft, pen).max(stroke(x, y, &head, pen));
        let fill = over(shape::scale(solid(RIM.0), mark * mark_k), fill);
        shares(&[(rim, outer - inner), (fill, inner)])
    })
}

/// A field `w` x `h` design px at `d`: `fill` inside a 1 px `rim` with rounded corners.
fn field_box(fill: Color, rim: Color, w: i32, h: i32, d: i32) -> Canvas {
    let k = f64::from(d);
    let (fw, fh) = (f64::from(w) * k, f64::from(h) * k);
    shape::draw(w, h, d, |x, y| {
        let outer = rounded(x, y, fw, fh, FIELD_R * k);
        let inner = rounded(
            x - k,
            y - k,
            fw - 2.0 * k,
            fh - 2.0 * k,
            (FIELD_R - 1.0) * k,
        );
        shares(&[(solid(rim), outer - inner), (solid(fill), inner)])
    })
}

fn badge_box(fill: Color, d: i32) -> Canvas {
    let k = f64::from(d);
    let n = f64::from(BADGE) * k;
    shape::draw(BADGE, BADGE, d, |x, y| {
        shape::scale(solid(fill), rounded(x, y, n, n, BADGE_R * k))
    })
}

fn state_of(m: &Logon, part: Part, live: bool) -> TileState {
    if !live {
        return TileState::Disabled;
    }
    match m.lit {
        Some((p, Interaction::Pressed)) if p == part => TileState::Pressed,
        Some((p, Interaction::Hot)) if p == part => TileState::Hot,
        _ if m.focus == part => TileState::Focus,
        _ => TileState::Normal,
    }
}

/// Whether tile `i` wears its band: open, under the pointer, or focused on the tiles.
fn banded(m: &Logon, i: usize) -> bool {
    let part = Part::Tile(i);
    m.screen == Screen::Tile(i)
        || matches!(m.lit, Some((p, Interaction::Hot | Interaction::Pressed)) if p == part)
        || (m.screen == Screen::Tiles && m.focus == part)
}

/// What draws the screen: the canvas' scale, the output's corner, the text and its sizes.
struct Pen<'a> {
    s: i32,
    u: TextUnits,
    o: Rect,
    text: &'a TextRenderer,
    title: &'a TextRenderer,
    px: u32,
    name_px: u32,
    ink: Color,
    /// The canvas is translucent where text goes: it lies over a picture.
    over: bool,
}

impl Pen<'_> {
    fn at(&self, r: Rect) -> Rect {
        Rect::new(r.x - self.o.x, r.y - self.o.y, r.width, r.height).scaled(self.s)
    }

    /// One line of `text` in design rect `r`, cut to fit.
    fn line(&self, c: &mut Canvas, r: Rect, text: &str, bold: bool, px: u32, align: Align) {
        let r = self.at(r);
        let shown = self.text.ellipsize(text, r.width, bold, px);
        self.put(self.text, c, r, &shown, bold, px, align);
    }

    /// `text` in `with` in device rect `r`.
    #[expect(clippy::too_many_arguments)]
    fn put(
        &self,
        with: &TextRenderer,
        c: &mut Canvas,
        r: Rect,
        text: &str,
        bold: bool,
        px: u32,
        align: Align,
    ) {
        match self.over {
            true => with.draw_over(c, r, text, bold, self.ink, align, px),
            false => with.draw(c, r, text, bold, self.ink, align, px),
        }
    }

    /// `msg` wrapped to design width `w`, cut to `room` lines.
    fn wrapped(&self, msg: &str, w: i32, room: usize) -> Vec<String> {
        message_lines(self.text, msg, w * self.s, self.px, room)
    }

    /// `lines` one under another from design `(x, y)`, `w` wide.
    fn lines(
        &self,
        c: &mut Canvas,
        lines: &[String],
        (x, y, w): (i32, i32, i32),
        k: Lines,
        align: Align,
    ) {
        for (i, line) in lines.iter().enumerate() {
            let r = self.at(Rect::new(x, y + i as i32 * k.line, w, k.line));
            self.put(self.text, c, r, line, false, self.px, align);
        }
    }
}

/// `c` covers `l.output` at scale `s`; `theme` is the output's (`Theme::scaled(s)`) in a Blue
/// scheme, and `text` its renderer.
pub fn draw(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    draw_titled(c, m, l, theme, text, &powerpanel::title_renderer(), s);
}

/// [`draw`] with the wordmark in `title`, an unscaled renderer.
fn draw_titled(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    title: &TextRenderer,
    s: i32,
) {
    let sc = Variant::of(&theme.look).scheme();
    let o = l.output;
    let f = frame(o);
    let k = lines(theme.text);
    let px = design_px(theme) * s as u32;
    let pen = Pen {
        s,
        u: theme.text,
        o,
        text,
        title,
        px,
        name_px: theme.text.px(NAME_PX) * s as u32,
        ink: sc.welcome_text,
        over: m.over_picture,
    };
    let backdrop = cached_backdrop(sc, o.width, o.height, s, !m.over_picture);
    if m.over_picture {
        c.clear_rect(c.bounds());
        c.blit_over(0, 0, &backdrop);
    } else {
        c.blit_canvas(0, 0, &backdrop);
    }
    draw_mark(c, m, &f, k, &pen);
    for (i, r) in l.parts.iter().filter_map(|&(p, r)| match p {
        Part::Tile(i) => Some((i, r)),
        _ => None,
    }) {
        draw_tile(c, m, sc, i, r, k, &pen);
    }
    if m.screen == Screen::Typed
        && let Some(name) = l.rect(Part::Name)
    {
        let label = Rect::new(name.x, name.y - k.line, name.width, k.line);
        pen.line(c, label, &strings::user_name(), false, px, Align::Left);
        draw_field(c, m, theme, sc, Part::Name, name, &pen);
    }
    if let Some(field) = l.rect(Part::Password) {
        draw_password_row(c, m, l, theme, sc, field, k, &pen);
    }
    for (part, hue, mark, words) in [
        (Part::Power, Hue::Red, Mark::Off, strings::turn_off()),
        (Part::SwitchUser, Hue::Green, Mark::Arrows, switch_label()),
    ] {
        let Some(r) = l.rect(part) else { continue };
        let tile = powerpanel::tile(
            hue,
            mark,
            state_of(m, part, !m.busy || part == Part::Power),
            POWER,
            s,
        );
        let at = pen.at(r);
        c.blit_over(at.x, at.y, &tile);
        let label = Rect {
            x: r.x + POWER + POWER_GAP,
            width: r.width - POWER - POWER_GAP,
            ..r
        };
        pen.line(c, label, &words, false, px, Align::Left);
    }
}

/// The logo and wordmark right-aligned against the middle rule, with "To begin" under them
/// and, on the tiles, the message under that.
fn draw_mark(c: &mut Canvas, m: &Logon, f: &Frame, k: Lines, pen: &Pen) {
    let s = pen.s;
    let right = f.split - SIDE_GAP;
    let left = f.middle.x + SIDE_GAP;
    let block = LOGO + BEGIN_GAP + k.line;
    let top = f.middle.y + (f.middle.height - block) / 2;
    let spec = title_text();
    let t = pen.title.for_output(s);
    let size = spec.size_in(t) * MARK_SIZE.0 / MARK_SIZE.1;
    let mark_px = pen.u.px(size) * s as u32;
    let word_w = t.measure(MARK, spec.bold, mark_px);
    let word = pen.at(Rect::new(right, top, 0, LOGO));
    let word = Rect::new(word.x - word_w, word.y, word_w, word.height);
    pen.put(t, c, word, MARK, spec.bold, mark_px, Align::Left);
    let logo_x = word.x - (LOGO_GAP + LOGO) * s;
    logo::draw_any(c, logo_x, word.y, LOGO * s, pen.ink);
    if m.screen == Screen::Typed {
        return;
    }
    let begin_y = top + LOGO + BEGIN_GAP;
    let begin = Rect::new(left, begin_y, right - left, k.line);
    let words = if m.unlocking() {
        strings::unlock_begin()
    } else {
        strings::begin()
    };
    pen.line(c, begin, &words, false, pen.px, Align::Right);
    if m.screen == Screen::Tiles
        && let Some(msg) = &m.message
    {
        let w = right - left;
        let lines = pen.wrapped(msg, w, MESSAGE_LINES);
        pen.lines(
            c,
            &lines,
            (left, begin.bottom() + ROW_GAP, w),
            k,
            Align::Right,
        );
    }
}

fn draw_tile(c: &mut Canvas, m: &Logon, sc: &Scheme, i: usize, r: Rect, k: Lines, pen: &Pen) {
    let s = pen.s;
    let Some(user) = m.users.get(i) else {
        return;
    };
    let at = pen.at(r);
    if banded(m, i) {
        c.blit_over(at.x, at.y, &band(sc, r.width, r.height, s));
    }
    let n = (PICTURE * s) as u32;
    let stock;
    let picture = match &user.picture {
        Some(p) => p,
        None => {
            stock = userpic::draw(userpic::ALL[i % userpic::ALL.len()], n);
            &stock
        }
    };
    let (fx, fy) = (r.x + TILE_PAD, r.y + TILE_PAD);
    let f = pen.at(Rect::new(fx, fy, FRAMED, FRAMED));
    c.blit_over(f.x, f.y, &framed(picture, sc.welcome_tile_frame, s));
    let tx = fx + FRAMED + TEXT_GAP;
    let tw = r.right() - tx - TILE_PAD;
    let note = user.note.map(UserNote::text);
    let open = m.screen == Screen::Tile(i);
    let text_h = k.name + if note.is_some() { k.line } else { 0 };
    let mut y = if open {
        r.y + TILE_PAD
    } else {
        r.y + (TILE_H - text_h) / 2
    };
    pen.line(
        c,
        Rect::new(tx, y, tw, k.name),
        user.shown(),
        false,
        pen.name_px,
        Align::Left,
    );
    y += k.name;
    if let Some(note) = note {
        pen.line(
            c,
            Rect::new(tx, y, tw, k.line),
            &note,
            false,
            pen.px,
            Align::Left,
        );
        y += k.line;
    }
    if open {
        let cue = m
            .prompt
            .as_ref()
            .map_or_else(strings::password_cue, |q| q.text.clone());
        pen.line(
            c,
            Rect::new(tx, y, tw, k.line),
            &cue,
            false,
            pen.px,
            Align::Left,
        );
    }
}

/// The password field, the arrow and the badge beside it, and Caps Lock and the message under
/// them; on the typed screen the prompt or "Password:" labels the field.
#[expect(clippy::too_many_arguments)]
fn draw_password_row(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    sc: &Scheme,
    field: Rect,
    k: Lines,
    pen: &Pen,
) {
    let s = pen.s;
    let px = pen.px;
    let right = match m.screen {
        Screen::Tile(i) => l.rect(Part::Tile(i)).map_or(field.right(), |t| t.right()),
        _ => field.x + TILE_W.min(l.output.right() - SIDE_GAP - field.x),
    };
    if m.screen == Screen::Typed {
        let label = m
            .prompt
            .as_ref()
            .map_or_else(strings::password, |q| q.text.clone());
        let r = Rect::new(field.x, field.y - k.line, right - field.x, k.line);
        pen.line(c, r, &label, false, px, Align::Left);
    }
    draw_field(c, m, theme, sc, Part::Password, field, pen);
    if let Some(go_at) = l.rect(Part::Submit) {
        let at = pen.at(go_at);
        c.blit_over(at.x, at.y, &go(state_of(m, Part::Submit, !m.busy), s));
    }
    if let (Some(short), Some(b)) = (&m.layout, l.rect(Part::Layout)) {
        let at = pen.at(b);
        c.blit_over(at.x, at.y, &badge_box(sc.welcome_band, s));
        pen.text
            .draw(c, at, short, false, pen.ink, Align::Center, pen.px);
    }
    let w = right - field.x;
    let mut lines = Vec::new();
    if m.caps_lock {
        lines.extend(pen.wrapped(&strings::caps_lock(), w, 1));
    }
    if let Some(msg) = m.said() {
        lines.extend(pen.wrapped(&msg, w, MESSAGE_LINES - lines.len()));
    }
    pen.lines(
        c,
        &lines,
        (field.x, field.bottom() + ROW_GAP, w),
        k,
        Align::Left,
    );
}

/// A white field showing `part`'s edit; a read-only one keeps the face colour.
fn draw_field(
    c: &mut Canvas,
    m: &Logon,
    theme: &Theme,
    sc: &Scheme,
    part: Part,
    r: Rect,
    pen: &Pen,
) {
    let s = pen.s;
    let cs = &theme.colors;
    let edit: TextEdit = match part {
        Part::Name => m.name.clone(),
        _ if m.password.is_secret() => m.password.masked(),
        _ => m.password.clone(),
    };
    let fill = if edit.is_read_only() {
        cs.button_face
    } else {
        cs.window
    };
    let at = pen.at(r);
    c.blit_over(
        at.x,
        at.y,
        &field_box(fill, sc.edit_border.normal.rim, r.width, r.height, s),
    );
    let inner = Rect::new(
        at.x + FIELD_PAD * s,
        at.y + s,
        at.width - 2 * FIELD_PAD * s,
        at.height - 2 * s,
    );
    let p = Paint { s, ..theme.paint() };
    let first = edit_first_visible(pen.text, &edit, inner.width, pen.px, 0);
    let caret = Caret::new(m.focus == part && !m.busy, true);
    draw_edit(c, pen.text, &edit, inner, pen.px, &p, caret, first);
}

#[cfg(test)]
mod tests;
