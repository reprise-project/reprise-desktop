//! Glass's login screen over the host's wallpaper: a picture tile with a password field and round
//! arrow, the user tiles, the error screen, the busy spinner, the mark and the power button.

use super::Settings;
use super::panel::{self, Fade, LightBox};
use super::tint::{self, Surface};
use crate::art::logo;
use crate::logon::{
    Change, Logon, LogonLayout, LogonUser, Part, Plan, Screen, Show, Slice, UserNote, Window,
    message_lines, text as strings,
};
use crate::paint::{self, Paint};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::shape::{
    self, CLEAR, Glyph, Px, Stops, centre, ellipse, glyph, over, premul, radial, ringed, rounded,
    stroke,
};
use crate::style::{Part as StylePart, State};
use crate::text::{TextRenderer, split_mnemonic};
use crate::textedit::{Caret, edit_first_visible};
use crate::theme::{Color, Theme};

/// Design px. Text rows, fields, buttons and tile cells grow with the text size; pictures, the
/// arrow and the power button keep theirs.
const PICTURE: i32 = 128;
const FRAME_PAD: i32 = 7;
const FRAME_R: f64 = 7.0;
const PICTURE_R: f64 = 3.0;
const SMALL_PICTURE: i32 = 80;
const SMALL_PAD: i32 = 5;
const SMALL_R: f64 = 5.0;
const CELL_W: i32 = 132;
const CELL_GAP: i32 = 12;
const NAME_PX: u32 = 20;
const NAME_H: i32 = 30;
const NOTE_H: i32 = 16;
const TILE_NAME_H: i32 = 20;
const FIELD_W: i32 = 210;
const FIELD_H: i32 = 24;
const FIELD_PAD: i32 = 5;
const ARROW: i32 = 26;
const ARROW_GAP: i32 = 6;
const PICTURE_GAP: i32 = 10;
const ROW_GAP: i32 = 8;
const SWITCH_GAP: i32 = 22;
const BUTTON_H: i32 = 26;
const BUTTON_MIN_W: i32 = 96;
const BUTTON_PAD: i32 = 16;
const BUTTON_R: f64 = 3.0;
const ICON: i32 = 24;
const ICON_GAP: i32 = 10;
const MESSAGE_W: i32 = 380;
const SPINNER: i32 = 28;
const BUSY_PX: u32 = 28;
const EDGE: i32 = 24;
const POWER_W: i32 = 36;
const MENU_W: i32 = 20;
const POWER_H: i32 = 26;
const MENU_ARROW: i32 = 7;
const BADGE_W: i32 = 34;
const MARK: i32 = 32;
const MARK_PX: u32 = 24;
const MARK_GAP: i32 = 8;
/// How far the block's middle sits above the screen's.
const LIFT: i32 = 24;
/// The wordmark beside the mark: the product's name, never translated.
const WORD: &str = "Reprise";
/// The dark glow under white text on the wallpaper: its reach and its peak alpha.
const HALO: i32 = 3;
const HALO_ALPHA: f64 = 150.0;
const HALO_INK: Color = Color::rgb(0x00, 0x00, 0x00);
/// The white glow round a lit picture frame: its reach and its alpha at the frame.
const GLOW: i32 = 8;
const GLOW_ALPHA: f64 = 110.0;
/// The spinner's head at step 0, radians clockwise from the top.
const SPIN_HEAD: f64 = 0.8;
/// The ring turns clockwise in steps: this many a turn, each shown this long.
pub const SPIN_STEPS: u32 = 60;
pub const SPIN_STEP_MS: u32 = 20;
/// Transitions, ms. A picked frame flies for `PICK_MS`, back for `BACK_MS` while the tiles come
/// in `STAGGER_MS` apart from it outward.
const PICK_MS: u32 = 320;
const BACK_MS: u32 = 300;
const STAGGER_MS: u32 = 30;
/// Coming forward, tiles rise into place one after another and a block rises whole; `RISE` is
/// design px.
const ENTER_TILE_MS: u32 = 240;
const ENTER_STAGGER_MS: u32 = 40;
const ENTER_STAGGERED: usize = 4;
const ENTER_BLOCK_MS: u32 = 360;
const RISE: i32 = 12;
/// The block out and the next in, half each; the busy text alone.
const CROSS_MS: u32 = 240;
const STATUS_MS: u32 = 160;

const INK: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const NOTE_INK: Color = Color::rgb(0xE4, 0xEE, 0xFA);
const FIELD_FILL: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const READ_ONLY_FILL: Color = Color::rgb(0xEC, 0xEE, 0xF2);
const PERSON_SKY: [Color; 2] = [Color::rgb(0xF2, 0xF6, 0xFB), Color::rgb(0xC4, 0xD4, 0xE6)];
const PERSON_INK: Color = Color::rgb(0x8A, 0xA2, 0xBE);

const WHITE: Px = grey(255.0);
const OUTLINE: Px = (150.0, [0.0, 8.0, 20.0]);
const OUTLINE_PRESSED: Px = (190.0, [0.0, 6.0, 16.0]);
const LINE: Px = grey(120.0);
const LINE_LIT: Px = grey(220.0);
const PRESSED_SHADE: Px = (90.0, [0.0, 10.0, 30.0]);
const LIFT_HOT: f64 = 0.22;
const LIFT_FOCUS: f64 = 0.12;
/// The sheen down a glass box: bright on top, a step at the middle, a little light at the foot.
const SHEEN: [(f64, Px); 4] = [
    (0.0, grey(110.0)),
    (0.48, grey(46.0)),
    (0.5, grey(8.0)),
    (1.0, grey(36.0)),
];
const CELL_FOCUS: LightBox = cell(150, (Color::rgb(0xFF, 0xFF, 0xFF), 60));
const CELL_HOT: LightBox = cell(90, (Color::rgb(0xFF, 0xFF, 0xFF), 34));
const CELL_PRESSED: LightBox = cell(150, (Color::rgb(0x33, 0x59, 0x8C), 40));

const RIM: Px = grey(230.0);
const RIM_FOCUS: Px = grey(255.0);
const GLOSS: Px = grey(95.0);
const DISC: [(f64, Px); 2] = [
    (0.0, ink(Color::rgb(0x6A, 0xB8, 0xF0))),
    (1.0, ink(Color::rgb(0x1C, 0x5E, 0xA8))),
];
const DISC_HOT: [(f64, Px); 2] = [
    (0.0, ink(Color::rgb(0x9C, 0xD8, 0xFF))),
    (1.0, ink(Color::rgb(0x2C, 0x7C, 0xCC))),
];
const DISC_PRESSED: [(f64, Px); 2] = [
    (0.0, ink(Color::rgb(0x3C, 0x86, 0xC4))),
    (1.0, ink(Color::rgb(0x0E, 0x3C, 0x78))),
];

const RED_OUTLINE: Px = ink(Color::rgb(0x4A, 0x08, 0x06));
const RED_LINE: Px = (130.0, [130.0, 96.0, 96.0]);
const RED_GLOSS: Px = grey(40.0);
const RED: [(f64, Px); 3] = [
    (0.0, ink(Color::rgb(0xD8, 0x4A, 0x3C))),
    (0.5, ink(Color::rgb(0xA8, 0x1C, 0x14))),
    (1.0, ink(Color::rgb(0x7A, 0x0E, 0x0A))),
];
const RED_HOT: [(f64, Px); 3] = [
    (0.0, ink(Color::rgb(0xF2, 0x6E, 0x5C))),
    (0.5, ink(Color::rgb(0xCC, 0x30, 0x24))),
    (1.0, ink(Color::rgb(0x98, 0x18, 0x10))),
];
const RED_PRESSED: [(f64, Px); 3] = [
    (0.0, ink(Color::rgb(0x8C, 0x1A, 0x12))),
    (0.5, ink(Color::rgb(0x6A, 0x0C, 0x08))),
    (1.0, ink(Color::rgb(0x56, 0x08, 0x06))),
];
const SPIN: [(f64, Px); 3] = [
    (0.0, (40.0, [40.0, 44.0, 48.0])),
    (0.6, (110.0, [92.0, 150.0, 200.0])),
    (1.0, (255.0, [220.0, 240.0, 255.0])),
];

/// A tile's light box on the tiles screen: a white outline at alpha `outline` round a flat fill.
const fn cell(outline: u8, fill: (Color, u8)) -> LightBox {
    LightBox {
        outline: (Color::rgb(0xFF, 0xFF, 0xFF), outline),
        line: None,
        fill: (fill, fill),
        radius: 4.0,
        fade: Fade::Box,
    }
}

const fn ink(c: Color) -> Px {
    (255.0, [c.r as f64, c.g as f64, c.b as f64])
}

/// White at alpha `a`, premultiplied.
const fn grey(a: f64) -> Px {
    (a, [a, a, a])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Rest,
    Hot,
    Pressed,
    Focused,
}

/// Design px in the output's space. `parts` are the layout's; the rest only `draw` uses.
#[derive(Default)]
struct Geometry {
    parts: Vec<(Part, Rect)>,
    frame: Option<(Part, Rect)>,
    name: Option<Rect>,
    note: Option<Rect>,
    notice: Option<Rect>,
    /// Per tile in view: its part, its frame, its name and its note line.
    tiles: Vec<(Part, Rect, Rect, Rect)>,
    /// The error screen's icon and text, centred together across this row.
    message: Option<Rect>,
    /// The spinner and the busy text, centred together across this row.
    busy: Option<Rect>,
    mark: Rect,
}

/// How many tiles fit across `output` at `theme`'s text size, Other user included: the host's
/// [`Logon::column`].
pub fn column(theme: &Theme, output: Rect) -> usize {
    let cell = theme.text.len(CELL_W) + CELL_GAP;
    ((output.width - 2 * EDGE + CELL_GAP) / cell).max(1) as usize
}

/// `button_w` is the width of the screen's one centred button (Switch User or OK).
fn geometry(m: &Logon, theme: &Theme, output: Rect, button_w: i32) -> Geometry {
    let u = theme.text;
    let mut g = Geometry::default();
    let cx = output.x + output.width / 2;
    let centred = |w: i32, y: i32, h: i32| Rect::new(cx - w / 2, y, w, h);
    let frame = PICTURE + 2 * FRAME_PAD;
    let (name_h, note_h) = (u.len(NAME_H), u.len(NOTE_H));
    let (field_w, field_h, button_h) = (u.len(FIELD_W), u.len(FIELD_H), u.len(BUTTON_H));
    let text_w = u.len(MESSAGE_W);
    let top = |h: i32| output.y + (output.height - h) / 2 - LIFT;

    match m.screen {
        _ if m.busy => g.busy = Some(centred(text_w, top(SPINNER), SPINNER)),
        Screen::Tile(i) => {
            let h = frame + PICTURE_GAP + name_h + note_h + ROW_GAP + field_h + note_h;
            let mut y = top(h + SWITCH_GAP + button_h);
            let r = centred(frame, y, frame);
            g.frame = Some((Part::Tile(i), r));
            g.parts.push((Part::Tile(i), r));
            y += frame + PICTURE_GAP;
            g.name = Some(centred(text_w, y, name_h));
            y += name_h;
            g.note = Some(centred(text_w, y, note_h));
            y += note_h + ROW_GAP;
            password_row(&mut g, centred(field_w, y, field_h));
            y += field_h;
            g.notice = Some(centred(text_w, y, note_h));
            y += note_h + SWITCH_GAP;
            if m.offers_switch_user() {
                g.parts
                    .push((Part::SwitchUser, centred(button_w, y, button_h)));
            }
        }
        Screen::Typed => {
            let h = frame + PICTURE_GAP + name_h + ROW_GAP + 2 * field_h + ROW_GAP + note_h;
            let listed = m.offers_switch_user();
            let mut y = top(h + if listed { SWITCH_GAP + button_h } else { 0 });
            g.frame = Some((Part::OtherUser, centred(frame, y, frame)));
            y += frame + PICTURE_GAP;
            g.name = Some(centred(text_w, y, name_h));
            y += name_h + ROW_GAP;
            g.parts.push((Part::Name, centred(field_w, y, field_h)));
            y += field_h + ROW_GAP;
            password_row(&mut g, centred(field_w, y, field_h));
            y += field_h;
            g.notice = Some(centred(text_w, y, note_h));
            y += note_h + SWITCH_GAP;
            if listed {
                g.parts
                    .push((Part::SwitchUser, centred(button_w, y, button_h)));
            }
        }
        Screen::Tiles => {
            let small = SMALL_PICTURE + 2 * SMALL_PAD;
            let (cell_w, tile_name_h) = (u.len(CELL_W), u.len(TILE_NAME_H));
            let cell_h = small + PICTURE_GAP / 2 + tile_name_h + note_h;
            let total = m.users.len() + 1;
            let fit = column(theme, output).min(total);
            let first = m.first_tile.min(total - fit);
            let row_w = fit as i32 * cell_w + (fit as i32 - 1) * CELL_GAP;
            let y = top(cell_h);
            for (k, item) in (first..first + fit).enumerate() {
                let x = cx - row_w / 2 + k as i32 * (cell_w + CELL_GAP);
                let part = if item < m.users.len() {
                    Part::Tile(item)
                } else {
                    Part::OtherUser
                };
                let tile = Rect::new(x + (cell_w - small) / 2, y, small, small);
                let name = Rect::new(x, tile.bottom() + PICTURE_GAP / 2, cell_w, tile_name_h);
                let note = Rect::new(x, name.bottom(), cell_w, note_h);
                g.tiles.push((part, tile, name, note));
                g.parts.push((part, Rect::new(x, y, cell_w, cell_h)));
            }
        }
        Screen::Message => {
            let h = 2 * name_h + SWITCH_GAP + button_h;
            let y = top(h);
            g.message = Some(centred(text_w, y, 2 * name_h));
            let ok = centred(button_w, y + 2 * name_h + SWITCH_GAP, button_h);
            g.parts.push((Part::MessageOk, ok));
        }
    }

    let bottom = output.bottom() - EDGE;
    let power = Rect::new(
        output.right() - EDGE - POWER_W - MENU_W,
        bottom - POWER_H,
        POWER_W,
        POWER_H,
    );
    if m.offers_power() {
        g.parts.push((Part::Power, power));
        g.parts.push((
            Part::PowerMenu,
            Rect::new(power.right(), power.y, MENU_W, POWER_H),
        ));
    }
    if m.layout.is_some() {
        let badge = Rect::new(output.x + EDGE, bottom - POWER_H, BADGE_W, POWER_H);
        g.parts.push((Part::Layout, badge));
    }
    g.mark = Rect::new(output.x, bottom - MARK, output.width, MARK);
    g
}

fn password_row(g: &mut Geometry, field: Rect) {
    let arrow = Rect::new(
        field.right() + ARROW_GAP,
        field.y + (field.height - ARROW) / 2,
        ARROW,
        ARROW,
    );
    g.parts.push((Part::Password, field));
    g.parts.push((Part::Submit, arrow));
}

fn button_caption(m: &Logon) -> Option<String> {
    match m.screen {
        _ if m.busy => None,
        Screen::Message => Some(plain(&strings::ok())),
        Screen::Tiles => None,
        Screen::Typed | Screen::Tile(_) if m.offers_switch_user() => {
            Some(plain(&strings::switch_user()))
        }
        Screen::Typed | Screen::Tile(_) => None,
    }
}

/// `label` without its access-key marker: the screen draws no underlines.
fn plain(label: &str) -> String {
    split_mnemonic(label).0
}

/// The UI font size in px at 100% scale.
fn ui_px(theme: &Theme) -> u32 {
    theme.fonts.size_px / theme.scale.max(1) as u32
}

pub fn layout(m: &Logon, theme: &Theme, text: &TextRenderer, output: Rect) -> LogonLayout {
    let button_w = button_caption(m).map_or(0, |caption| {
        let w = text.measure(&caption, false, ui_px(theme)) + 2 * BUTTON_PAD;
        w.max(theme.text.len(BUTTON_MIN_W))
    });
    LogonLayout {
        output,
        parts: geometry(m, theme, output, button_w).parts,
        dialog: None,
    }
}

/// The width of the screen's one centred button (Switch User or OK), as laid out.
fn button_width(l: &LogonLayout) -> i32 {
    [Part::SwitchUser, Part::MessageOk]
        .into_iter()
        .find_map(|p| l.rect(p))
        .map_or(0, |r| r.width)
}

/// `c` covers `l.output` at scale `s` and starts clear: the host lays it over the wallpaper.
/// `theme` is the output's (`Theme::scaled(s)`) and `text` its renderer.
pub fn draw(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    draw_shown(c, m, l, theme, text, s, Show::All);
}

/// [`draw`] limited to `show`. The picture frames go on last, so a frame drawn with
/// [`draw_frame`] over `Show::AllBut` its part gives the whole screen.
pub fn draw_shown(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
    show: Show,
) {
    let g = geometry(m, theme, l.output, button_width(l));
    if show != Show::Chrome {
        draw_block(c, m, l, &g, theme, text, s);
    }
    let o = l.output;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
    let ui = ui_px(theme) * s as u32;
    if let Some(r) = l.rect(Part::Power) {
        power_button(c, at(r), face(m, Part::Power), face(m, Part::PowerMenu), s);
    }
    if let (Some(r), Some(short)) = (l.rect(Part::Layout), m.layout.as_deref()) {
        glass_box(c, at(r), BUTTON_R, face(m, Part::Layout), s);
        label(c, text, at(r), short, ui, INK, s);
    }
    wordmark(c, text, at(g.mark), theme.text.px(MARK_PX) * s as u32, s);
    if show == Show::Chrome {
        return;
    }
    for f in frames(&g) {
        if show != Show::AllBut(f.part) {
            frame_at(c, m, &f, o, s);
        }
    }
}

/// `part`'s picture frame alone, as [`draw`] draws it.
pub fn draw_frame(c: &mut Canvas, m: &Logon, l: &LogonLayout, theme: &Theme, s: i32, part: Part) {
    let g = geometry(m, theme, l.output, button_width(l));
    if let Some(f) = frames(&g).into_iter().find(|f| f.part == part) {
        frame_at(c, m, &f, l.output, s);
    }
}

/// A picture frame on the screen: its part, design rect, padding and corner radius.
struct FrameAt {
    part: Part,
    rect: Rect,
    pad: i32,
    radius: f64,
}

fn frames(g: &Geometry) -> Vec<FrameAt> {
    let big = (g.frame.iter()).map(|&(part, rect)| FrameAt {
        part,
        rect,
        pad: FRAME_PAD,
        radius: FRAME_R,
    });
    let small = (g.tiles.iter()).map(|&(part, rect, _, _)| FrameAt {
        part,
        rect,
        pad: SMALL_PAD,
        radius: SMALL_R,
    });
    big.chain(small).collect()
}

fn frame_at(c: &mut Canvas, m: &Logon, f: &FrameAt, o: Rect, s: i32) {
    draw_posed(c, m, f.part, &f.pose(m, o, s), s);
}

impl FrameAt {
    fn pose(&self, m: &Logon, o: Rect, s: i32) -> Pose {
        let r = self.rect;
        Pose {
            rect: Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s),
            pad: f64::from(self.pad * s),
            radius: self.radius,
            look: Look::of(face(m, self.part)),
        }
    }
}

/// A picture frame's place on the canvas, its padding (device px), corner radius and look: what
/// a transition moves between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub rect: Rect,
    pad: f64,
    radius: f64,
    look: Look,
}

impl Pose {
    /// `a` at `u` = 0, `b` at 1.
    pub fn between(a: &Pose, b: &Pose, u: f64) -> Pose {
        let at = |x: i32, y: i32| lerp(f64::from(x), f64::from(y), u).round() as i32;
        let (ra, rb) = (a.rect, b.rect);
        Pose {
            rect: Rect::new(
                at(ra.x, rb.x),
                at(ra.y, rb.y),
                at(ra.width, rb.width),
                at(ra.height, rb.height),
            ),
            pad: lerp(a.pad, b.pad, u),
            radius: lerp(a.radius, b.radius, u),
            look: Look::between(&a.look, &b.look, u),
        }
    }

    /// The device rect the frame's drawing covers, its glow included.
    pub fn reach(&self, s: i32) -> Rect {
        self.rect.inset(-GLOW * s)
    }
}

/// `a` at `u` = 0 and `b` at 1, exactly.
fn lerp(a: f64, b: f64, u: f64) -> f64 {
    (1.0 - u) * a + u * b
}

fn lerp_px(a: Px, b: Px, u: f64) -> Px {
    let c = |k: usize| lerp(a.1[k], b.1[k], u);
    (lerp(a.0, b.0, u), [c(0), c(1), c(2)])
}

/// A glass box's face as numbers a transition can ease: the glow's share, the outline, the
/// light line, the sheen's lift and the pressed shade's share.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    glow: f64,
    outline: Px,
    line: Px,
    lift: f64,
    shade: f64,
}

impl Look {
    fn of(face: Face) -> Look {
        let (outline, line, lift, shade) = match face {
            Face::Rest => (OUTLINE, LINE, 0.0, 0.0),
            Face::Hot => (OUTLINE, LINE_LIT, LIFT_HOT, 0.0),
            Face::Focused => (OUTLINE, LINE_LIT, LIFT_FOCUS, 0.0),
            Face::Pressed => (OUTLINE_PRESSED, LINE, 0.0, 1.0),
        };
        let glow = match face {
            Face::Hot | Face::Focused => 1.0,
            Face::Rest | Face::Pressed => 0.0,
        };
        Look {
            glow,
            outline,
            line,
            lift,
            shade,
        }
    }

    fn between(a: &Look, b: &Look, u: f64) -> Look {
        Look {
            glow: lerp(a.glow, b.glow, u),
            outline: lerp_px(a.outline, b.outline, u),
            line: lerp_px(a.line, b.line, u),
            lift: lerp(a.lift, b.lift, u),
            shade: lerp(a.shade, b.shade, u),
        }
    }
}

/// `part`'s frame as [`draw`] lays it out on `m`'s screen.
pub fn pose(m: &Logon, l: &LogonLayout, theme: &Theme, s: i32, part: Part) -> Option<Pose> {
    let g = geometry(m, theme, l.output, button_width(l));
    let f = frames(&g).into_iter().find(|f| f.part == part)?;
    Some(f.pose(m, l.output, s))
}

/// `part`'s picture frame at `pose`, with no picture holding a person's outline.
pub fn draw_posed(c: &mut Canvas, m: &Logon, part: Part, pose: &Pose, s: i32) {
    let picture = user_of(m, part).and_then(|u| u.picture.as_ref());
    let r = pose.rect;
    if pose.look.glow > 0.0 {
        glow(c, r, pose.radius, pose.look.glow, s);
    }
    glass_box_look(c, r, pose.radius, &pose.look, s);
    let inner = r.inset(pose.pad.round() as i32);
    let (w, h) = (inner.width, inner.height);
    if w <= 0 || h <= 0 {
        return;
    }
    let k = f64::from(s);
    let pic = picture.map(|p| p.resample_box(w as u32, h as u32));
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let cov = rounded(fx, fy, f64::from(w), f64::from(h), PICTURE_R * k);
            if cov <= 0.0 {
                continue;
            }
            let p = match &pic {
                Some(p) => shape::unpack(p.get(x, y)),
                None => person(
                    (centre(x, s) + 0.5) / f64::from(w / s),
                    (centre(y, s) + 0.5) / f64::from(h / s),
                ),
            };
            put_over(c, inner.x + x, inner.y + y, shape::scale(p, cov));
        }
    }
}

/// How `change` plays on Glass, `m` and `l` being the screen after it.
pub fn plan(change: Change, m: &Logon, l: &LogonLayout, s: i32) -> Plan {
    let whole = |from: u32, len: u32, rise: i32| {
        vec![Slice {
            columns: None,
            window: Window::new(from, len),
            rise,
        }]
    };
    let share = |ms: u32, pct: u32| ms * pct / 100;
    let o = l.output;
    // Each tile's columns run to the middle of the gaps either side.
    let cells: Vec<(Part, i32, i32)> = (l.parts.iter())
        .filter(|(p, _)| matches!(p, Part::Tile(_) | Part::OtherUser))
        .map(|&(p, r)| (p, (r.x - o.x) * s, (r.right() - o.x) * s))
        .collect();
    let columns = |k: usize| {
        let left = (k > 0).then(|| (cells[k - 1].2 + cells[k].1) / 2);
        let right = (k + 1 < cells.len()).then(|| (cells[k].2 + cells[k + 1].1) / 2);
        Some((left.unwrap_or(i32::MIN), right.unwrap_or(i32::MAX)))
    };
    match change {
        Change::Pick(part) => Plan {
            out: Some(Window::new(0, share(PICK_MS, 35))),
            into: whole(share(PICK_MS, 40), share(PICK_MS, 60), 0),
            carried: Some((part, PICK_MS)),
        },
        Change::Back(part) if m.screen == Screen::Tiles => {
            let picked = cells.iter().position(|c| c.0 == part).unwrap_or(0);
            let into = (0..cells.len())
                .map(|k| Slice {
                    columns: columns(k),
                    window: Window::new(
                        k.abs_diff(picked) as u32 * STAGGER_MS + share(BACK_MS, 30),
                        share(BACK_MS, 70),
                    ),
                    rise: 0,
                })
                .collect();
            Plan {
                out: Some(Window::new(0, share(BACK_MS, 35))),
                into,
                carried: Some((part, BACK_MS)),
            }
        }
        Change::Enter if m.screen == Screen::Tiles => Plan {
            out: None,
            into: (0..cells.len())
                .map(|k| Slice {
                    columns: columns(k),
                    window: Window::new(
                        k.min(ENTER_STAGGERED) as u32 * ENTER_STAGGER_MS,
                        ENTER_TILE_MS,
                    ),
                    rise: RISE * s,
                })
                .collect(),
            carried: None,
        },
        Change::Enter => Plan {
            out: None,
            into: whole(0, ENTER_BLOCK_MS, RISE * s),
            carried: None,
        },
        Change::Status => Plan {
            out: Some(Window::new(0, STATUS_MS)),
            into: whole(0, STATUS_MS, 0),
            carried: None,
        },
        Change::Admin | Change::Back(_) => Plan {
            out: Some(Window::new(0, CROSS_MS)),
            into: whole(0, CROSS_MS, 0),
            carried: None,
        },
        Change::Busy | Change::Fail | Change::Ask | Change::MessageOk | Change::Idle => Plan {
            out: Some(Window::new(0, CROSS_MS / 2)),
            into: whole(CROSS_MS / 2, CROSS_MS / 2, 0),
            carried: None,
        },
    }
}

fn face(m: &Logon, part: Part) -> Face {
    match m.lit {
        Some((p, Interaction::Pressed)) if p == part => Face::Pressed,
        Some((p, Interaction::Hot)) if p == part => Face::Hot,
        _ if m.focus == part => Face::Focused,
        _ => Face::Rest,
    }
}

fn user_of(m: &Logon, part: Part) -> Option<&LogonUser> {
    match part {
        Part::Tile(i) => m.users.get(i),
        _ => None,
    }
}

/// Everything but the chrome and the picture frames.
fn draw_block(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    g: &Geometry,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    let o = l.output;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
    let u = theme.text;
    let ui = ui_px(theme) * s as u32;
    let face = |part: Part| face(m, part);
    let note = |part: Part| user_of(m, part).and_then(|u| u.note).map(UserNote::text);

    if let Some((part, _)) = g.frame {
        let shown = user_of(m, part).map_or_else(strings::other_user, |u| u.shown().to_owned());
        if let Some(r) = g.name {
            label(c, text, at(r), &shown, u.px(NAME_PX) * s as u32, INK, s);
        }
        if let (Some(r), Some(note)) = (g.note, note(part)) {
            label(c, text, at(r), &note, ui, NOTE_INK, s);
        }
    }
    if let Some(r) = g.notice {
        let notice = (m.caps_lock.then(strings::caps_lock)).or_else(|| m.standing());
        if let Some(notice) = notice {
            // A second line runs into the gap above Switch User.
            let r = at(r);
            for (k, line) in message_lines(text, &notice, r.width, ui, 2)
                .iter()
                .enumerate()
            {
                let row = Rect {
                    y: r.y + k as i32 * r.height,
                    ..r
                };
                label(c, text, row, line, ui, NOTE_INK, s);
            }
        }
    }
    for &(part, _, name, note_at) in &g.tiles {
        let lit = if m.focus == part {
            Face::Focused
        } else {
            face(part)
        };
        if lit != Face::Rest
            && let Some(cell) = l.rect(part)
        {
            lit_cell(c, at(cell), lit, s);
        }
        let shown = user_of(m, part).map_or_else(strings::other_user, |u| u.shown().to_owned());
        label(c, text, at(name), &shown, ui, INK, s);
        if let Some(words) = note(part) {
            label(c, text, at(note_at), &words, ui, NOTE_INK, s);
        }
    }

    let password_cue = m
        .prompt
        .as_ref()
        .map_or_else(strings::password_cue, |p| p.text.clone());
    let p = Paint { s, ..theme.paint() };
    for (part, cue) in [
        (Part::Name, strings::user_name_cue()),
        (Part::Password, password_cue),
    ] {
        let Some(r) = l.rect(part) else { continue };
        let state = match face(part) {
            Face::Focused => State::Focused,
            Face::Hot | Face::Pressed => State::Hot,
            Face::Rest => State::Normal,
        };
        field(c, m, part, &cue, at(r), state, text, &p, ui);
    }
    if let Some(r) = l.rect(Part::Submit) {
        arrow(c, at(r), face(Part::Submit), s);
    }
    for (part, caption) in [
        (Part::SwitchUser, plain(&strings::switch_user())),
        (Part::MessageOk, plain(&strings::ok())),
    ] {
        if let Some(r) = l.rect(part) {
            glass_box(c, at(r), BUTTON_R, face(part), s);
            label(c, text, at(r), &caption, ui, INK, s);
        }
    }
    if let Some(r) = g.message {
        let r = at(r);
        let room = r.width - (ICON + ICON_GAP) * s;
        let msg = m.message.clone().unwrap_or_default();
        let lines = message_lines(text, &msg, room, ui, 2);
        let widest = lines.iter().map(|l| text.measure(l, false, ui)).max();
        let x = r.x + (room - widest.unwrap_or(0)) / 2;
        error_icon(c, x, r.y + (r.height - ICON * s) / 2, s);
        let line_h = r.height / 2;
        let left = x + (ICON + ICON_GAP) * s;
        for (k, line) in lines.iter().enumerate() {
            let y = r.y
                + if lines.len() == 1 {
                    line_h / 2
                } else {
                    k as i32 * line_h
                };
            let row = Rect::new(left, y, r.right() - left, line_h);
            left_label(c, text, row, line, ui, INK, s);
        }
    }
    if let Some(b) = busy_row(m, g, theme, text, o, s) {
        spinner(c, b.ring.x, b.ring.y, s, m.spin);
        let left = b.ring.right() + ICON_GAP * s;
        let r = b.row;
        let row = Rect::new(left, r.y - r.height / 2, r.right() - left, 2 * r.height);
        left_label(c, text, row, &b.word, b.px, INK, s);
    }
}

/// The busy row in device px: the ring's square and the row it is centred in with the busy text.
struct Busy {
    ring: Rect,
    row: Rect,
    word: String,
    px: u32,
}

fn busy_row(
    m: &Logon,
    g: &Geometry,
    theme: &Theme,
    text: &TextRenderer,
    o: Rect,
    s: i32,
) -> Option<Busy> {
    let r = g.busy?;
    let r = Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
    let px = theme.text.px(BUSY_PX) * s as u32;
    let word = m.busy_text();
    let w = (SPINNER + ICON_GAP) * s + text.measure(&word, false, px);
    let ring = Rect::new(r.x + (r.width - w) / 2, r.y, SPINNER * s, SPINNER * s);
    Some(Busy {
        ring,
        row: r,
        word,
        px,
    })
}

/// The busy ring's device rect on the canvas `draw` fills, while the screen is busy.
pub fn spinner_rect(
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) -> Option<Rect> {
    let g = geometry(m, theme, l.output, button_width(l));
    busy_row(m, &g, theme, text, l.output, s).map(|b| b.ring)
}

/// The ring alone drawn again at `m.spin` over what `draw` left in its rect.
pub fn draw_spinner(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    if let Some(r) = spinner_rect(m, l, theme, text, s) {
        c.clear_rect(r);
        spinner(c, r.x, r.y, s, m.spin);
    }
}

/// The ring's step `busy_for` after the screen first showed busy.
pub fn spin_step(busy_for: std::time::Duration) -> u32 {
    ((busy_for.as_millis() / u128::from(SPIN_STEP_MS)) % u128::from(SPIN_STEPS)) as u32
}

fn lit_cell(c: &mut Canvas, r: Rect, face: Face, s: i32) {
    let b = match face {
        Face::Focused => &CELL_FOCUS,
        Face::Pressed => &CELL_PRESSED,
        Face::Hot | Face::Rest => &CELL_HOT,
    };
    c.blit_over(r.x, r.y, &panel::light_box(b, r.width / s, r.height / s, s));
}

/// (`u`, `v`) in the unit square.
fn person(u: f64, v: f64) -> Px {
    let head = (u - 0.5).hypot(v - 0.40) < 0.17;
    let shoulders = ((u - 0.5) / 0.32).hypot((v - 0.95) / 0.30) < 1.0;
    if head || shoulders {
        premul(PERSON_INK, 255)
    } else {
        let sky = [premul(PERSON_SKY[0], 255), premul(PERSON_SKY[1], 255)];
        Stops(&[(0.0, sky[0]), (1.0, sky[1])]).at(v)
    }
}

/// `strength` of the glow, 1 for a lit frame's.
fn glow(c: &mut Canvas, r: Rect, radius: f64, strength: f64, s: i32) {
    let reach = GLOW * s;
    let k = f64::from(s);
    let (w, h) = (f64::from(r.width), f64::from(r.height));
    // Pixels whose middle lies on the box's straight cross are inside it, where no glow shows.
    let rk = radius * k;
    let cross = |v: f64, len: f64| v >= rk && v <= len - rk;
    for y in -reach..r.height + reach {
        let py = f64::from(y) + 0.5;
        let across = py > 0.0 && py < h;
        for x in -reach..r.width + reach {
            let px = f64::from(x) + 0.5;
            if across && px > 0.0 && px < w && (cross(px, w) || cross(py, h)) {
                continue;
            }
            let dist = shape::rounded_distance(px, py, w, h, radius * k) / k;
            if dist <= 0.0 || dist >= f64::from(GLOW) {
                continue;
            }
            let a = GLOW_ALPHA * strength * (1.0 - dist / f64::from(GLOW)).powi(2);
            put_over(c, r.x + x, r.y + y, grey(a));
        }
    }
}

/// The glass colour from `[glass]`'s defaults under an outline, a light line and a sheen.
fn glass_box(c: &mut Canvas, r: Rect, radius: f64, face: Face, s: i32) {
    glass_box_look(c, r, radius, &Look::of(face), s);
}

fn glass_box_look(c: &mut Canvas, r: Rect, radius: f64, look: &Look, s: i32) {
    let (w, h) = (r.width / s, r.height / s);
    let glass = tint::plain(&Settings::default(), Surface::Active);
    let body = if look.shade > 0.0 {
        over(shape::scale(PRESSED_SHADE, look.shade), glass)
    } else {
        glass
    };
    let (outline, line, lift) = (look.outline, look.line, look.lift);
    let hh = f64::from(h);
    // The fill changes only from row to row, and `ringed` paints row by row.
    let row = std::cell::Cell::new((f64::NAN, CLEAR));
    let px = ringed(w, h, radius, 2, s, |_, y, ring| match ring {
        0 => outline,
        1 => line,
        _ if row.get().0 == y => row.get().1,
        _ => {
            let sheen = Stops(&SHEEN).at((y - 1.5) / (hh - 3.0));
            let fill = over(shape::scale(WHITE, lift), over(sheen, body));
            row.set((y, fill));
            fill
        }
    });
    lay(c, r.x, r.y, w * s, &px);
}

fn arrow(c: &mut Canvas, r: Rect, face: Face, s: i32) {
    let k = f64::from(s);
    let n = f64::from(ARROW) * k;
    let half = n / 2.0;
    let stops: &[(f64, Px)] = match face {
        Face::Pressed => &DISC_PRESSED,
        Face::Hot => &DISC_HOT,
        Face::Rest | Face::Focused => &DISC,
    };
    let rim = if face == Face::Focused {
        RIM_FOCUS
    } else {
        RIM
    };
    let shaft = [(7.0 * k, half), (17.5 * k, half)];
    let head = [
        (13.0 * k, half - 6.0 * k),
        (19.5 * k, half),
        (13.0 * k, half + 6.0 * k),
    ];
    let img = shape::draw(ARROW, ARROW, s, |x, y| {
        let disc = ellipse(x, y, half, half, half, half);
        if disc <= 0.0 {
            return CLEAR;
        }
        let inside = ellipse(x, y, half, half, half - k, half - k);
        let fill = radial(x, y, half, 0.2 * n, 1.1 * half, n, &Stops(stops));
        let gloss = ellipse(x, y, half, 0.32 * n, 0.36 * n, 0.24 * n);
        let arrow = stroke(x, y, &shaft, 2.5 * k).max(shape::polygon(x, y, &head));
        let face = over(
            shape::scale(WHITE, arrow),
            over(shape::scale(GLOSS, gloss), fill),
        );
        shape::shares(&[(rim, (disc - inside).max(0.0)), (face, inside)])
    });
    c.blit_over(r.x, r.y, &img);
}

/// One bar for both parts, each half lit on its own.
fn power_button(c: &mut Canvas, r: Rect, power: Face, menu: Face, s: i32) {
    let (w, h) = (POWER_W + MENU_W, POWER_H);
    let hh = f64::from(h);
    let k = f64::from(s);
    let px = ringed(w, h, BUTTON_R, 2, s, |x, y, ring| {
        let column = x.round() as i32;
        let half = if column < POWER_W { power } else { menu };
        match ring {
            _ if ring > 0 && column == POWER_W - 1 => RED_OUTLINE,
            0 => RED_OUTLINE,
            1 if half == Face::Focused => RIM_FOCUS,
            1 => RED_LINE,
            _ => {
                let stops: &[(f64, Px)] = match half {
                    Face::Hot => &RED_HOT,
                    Face::Pressed => &RED_PRESSED,
                    Face::Rest | Face::Focused => &RED,
                };
                let t = (y - 1.5) / (hh - 3.0);
                let gloss = if t < 0.5 { RED_GLOSS } else { CLEAR };
                over(gloss, Stops(stops).at(t))
            }
        }
    });
    lay(c, r.x, r.y, w * s, &px);

    let (cx, cy) = (f64::from(POWER_W) / 2.0 * k, hh / 2.0 * k);
    let symbol = shape::draw(POWER_W, POWER_H, s, |x, y| {
        let ring = power_ring(x, y, cx, cy, 5.5 * k, 1.8 * k);
        let bar = stroke(x, y, &[(cx, cy - 7.5 * k), (cx, cy - k)], 1.8 * k);
        shape::scale(WHITE, ring.max(bar))
    });
    c.blit_over(r.x, r.y, &symbol);
    let up = glyph(Glyph::ArrowUp, MENU_ARROW, s, 1.0, WHITE);
    let ax = r.x + (POWER_W + (MENU_W - MENU_ARROW) / 2) * s;
    let ay = r.y + (POWER_H - MENU_ARROW) / 2 * s;
    c.blit_over(ax, ay, &up);
}

/// The power symbol's ring round (`cx`, `cy`): `radius` across its middle, `width` wide, open
/// over its top.
fn power_ring(x: f64, y: f64, cx: f64, cy: f64, radius: f64, width: f64) -> f64 {
    let (outer, inner) = (radius + width / 2.0, radius - width / 2.0);
    let ring = ellipse(x, y, cx, cy, outer, outer) - ellipse(x, y, cx, cy, inner, inner);
    let (dx, dy) = (x + 0.5 - cx, y + 0.5 - cy);
    if dy < 0.0 && dx.abs() < -dy * 0.75 {
        0.0
    } else {
        ring.max(0.0)
    }
}

/// At device (`x`, `y`).
fn error_icon(c: &mut Canvas, x: i32, y: i32, s: i32) {
    let k = f64::from(s);
    let n = f64::from(ICON) * k;
    let half = n / 2.0;
    let (a, b) = (half - 4.5 * k, half + 4.5 * k);
    let img = shape::draw(ICON, ICON, s, |x, y| {
        let disc = ellipse(x, y, half, half, half, half);
        let inside = ellipse(x, y, half, half, half - 1.5 * k, half - 1.5 * k);
        let fill = radial(x, y, half, 0.25 * n, half, 0.9 * n, &Stops(&RED_HOT));
        let cross = stroke(x, y, &[(a, a), (b, b)], 2.5 * k);
        let cross = cross.max(stroke(x, y, &[(b, a), (a, b)], 2.5 * k));
        let face = over(shape::scale(WHITE, cross), fill);
        shape::shares(&[(RIM, (disc - inside).max(0.0)), (face, inside)])
    });
    c.blit_over(x, y, &img);
}

/// At device (`x`, `y`), turned `step` of [`SPIN_STEPS`]; the ring's light fades back from its
/// head.
fn spinner(c: &mut Canvas, x: i32, y: i32, s: i32, step: u32) {
    let k = f64::from(s);
    let half = f64::from(SPINNER) * k / 2.0;
    let tau = std::f64::consts::TAU;
    let head = SPIN_HEAD + f64::from(step % SPIN_STEPS) * tau / f64::from(SPIN_STEPS);
    let img = shape::draw(SPINNER, SPINNER, s, |x, y| {
        let outer = ellipse(x, y, half, half, half, half);
        let inner = ellipse(x, y, half, half, half - 4.0 * k, half - 4.0 * k);
        let turn = (x + 0.5 - half).atan2(half - y - 0.5);
        let t = 1.0 - (head - turn).rem_euclid(tau) / tau;
        shape::scale(Stops(&SPIN).at(t), (outer - inner).max(0.0))
    });
    c.blit_over(x, y, &img);
}

/// `part`'s edit in Glass's edit border at device `r`, the text drawn as every line edit's.
#[expect(clippy::too_many_arguments)]
fn field(
    c: &mut Canvas,
    m: &Logon,
    part: Part,
    cue: &str,
    r: Rect,
    state: State,
    text: &TextRenderer,
    p: &Paint,
    px: u32,
) {
    let mut edit = match part {
        Part::Name => m.name.clone(),
        _ => m.password.clone(),
    };
    edit.set_cue(cue);
    if edit.is_secret() {
        edit = edit.masked();
    }
    let s = p.s;
    let fill = if edit.is_read_only() {
        READ_ONLY_FILL
    } else {
        FIELD_FILL
    };
    c.fill_rect(r.inset(s), fill);
    if let Some(st) = p.look.style() {
        st.draw(c, r, StylePart::EditBorder, state, s);
    }
    let inner = Rect::new(
        r.x + FIELD_PAD * s,
        r.y,
        r.width - 2 * FIELD_PAD * s,
        r.height,
    );
    let first = edit_first_visible(text, &edit, inner.width, px, 0);
    let caret = Caret::new(m.focus == part && !m.busy, true);
    paint::draw_edit(c, text, &edit, inner, px, p, caret, first);
}

/// `words` centred on `r` in `color` over a dark halo, cut short to fit.
fn label(c: &mut Canvas, text: &TextRenderer, r: Rect, words: &str, px: u32, color: Color, s: i32) {
    let shown = text.ellipsize(words, r.width, false, px);
    let w = text.measure(&shown, false, px);
    inked(c, text, r, r.x + (r.width - w) / 2, &shown, px, color, s);
}

fn left_label(
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    words: &str,
    px: u32,
    color: Color,
    s: i32,
) {
    let shown = text.ellipsize(words, r.width, false, px);
    inked(c, text, r, r.x, &shown, px, color, s);
}

#[expect(clippy::too_many_arguments)]
fn inked(
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    pen: i32,
    words: &str,
    px: u32,
    color: Color,
    s: i32,
) {
    let Some(baseline) = text.baseline(r, false, px) else {
        return;
    };
    let m = HALO * s;
    let w = text.measure(words, false, px);
    let area = Rect::new(pen - m, r.y - m, w + 2 * m, r.height + 2 * m);
    if area.is_empty() {
        return;
    }
    let local = Rect::new(0, 0, area.width, area.height);
    let cov = Canvas::new(area.width as u32, area.height as u32).ink_coverage(|cc, white| {
        text.draw_run(cc, m, baseline - area.y, local, words, false, white, px);
    });
    ink_mask(c, area, &cov, color, s);
}

/// The dark halo keeps white text readable on a light wallpaper.
fn ink_mask(c: &mut Canvas, area: Rect, cov: &[u8], color: Color, s: i32) {
    let (w, h) = (area.width as usize, area.height as usize);
    let reach = (HALO * s / 2).max(1) as usize;
    let halo: Vec<u8> = blur(&blur(cov, w, h, reach), w, h, reach)
        .iter()
        .map(|&v| (2.0 * f64::from(v) * HALO_ALPHA / 255.0).min(HALO_ALPHA) as u8)
        .collect();
    let all = c.bounds();
    c.blit_mask_over(area.x, area.y, w, h, &halo, HALO_INK, all);
    c.blit_mask_over(area.x, area.y, w, h, cov, color, all);
}

fn blur(src: &[u8], w: usize, h: usize, reach: usize) -> Vec<u8> {
    let n = (2 * reach + 1) as u32;
    let mut across = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(reach), (x + reach).min(w - 1));
            let sum: u32 = src[y * w + a..=y * w + b]
                .iter()
                .map(|&v| u32::from(v))
                .sum();
            across[y * w + x] = (sum / n) as u8;
        }
    }
    let mut down = vec![0u8; w * h];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(reach), (y + reach).min(h - 1));
        for x in 0..w {
            let sum: u32 = (a..=b).map(|row| u32::from(across[row * w + x])).sum();
            down[y * w + x] = (sum / n) as u8;
        }
    }
    down
}

fn wordmark(c: &mut Canvas, text: &TextRenderer, r: Rect, px: u32, s: i32) {
    let (mark, m) = (MARK * s, HALO * s);
    let w = text.measure(WORD, false, px);
    let total = mark + MARK_GAP * s + w;
    let x = r.x + (r.width - total) / 2;
    let y = r.y + (r.height - mark) / 2;
    let area = Rect::new(x - m, y - m, mark + 2 * m, mark + 2 * m);
    let drawn = logo::coverage_any(mark as usize);
    let side = area.width as usize;
    let mut cov = vec![0u8; side * side];
    for (row, line) in drawn.chunks(mark as usize).enumerate() {
        let at = (row + m as usize) * side + m as usize;
        cov[at..at + line.len()].copy_from_slice(line);
    }
    ink_mask(c, area, &cov, INK, s);
    let word = Rect::new(x + mark + MARK_GAP * s, r.y, w, r.height);
    left_label(c, text, word, WORD, px, INK, s);
}

/// The pixels `px`, `w` device px a row, laid over `c` from device (`x`, `y`).
fn lay(c: &mut Canvas, x: i32, y: i32, w: i32, px: &[Px]) {
    for (i, &p) in px.iter().enumerate() {
        let i = i as i32;
        put_over(c, x + i % w, y + i / w, p);
    }
}

fn put_over(c: &mut Canvas, x: i32, y: i32, p: Px) {
    if p.0 <= 0.0 || x < 0 || y < 0 || x >= c.width as i32 || y >= c.height as i32 {
        return;
    }
    let under = shape::unpack(c.get(x, y));
    c.put_raw(x, y, shape::pack(over(p, under)));
}

#[cfg(test)]
mod tests;
