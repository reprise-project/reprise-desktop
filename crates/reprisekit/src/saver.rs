//! Screen savers: what each one shows and how it moves, stepped by the host and drawn into a
//! canvas of any size.
//! Time moves in fixed steps of [`STEP_MS`], so a saver looks the same
//! whatever rate the host draws at: [`Saver::advance`] takes the
//! milliseconds that passed and runs as many steps as fit.

use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

pub const STEP_MS: u64 = 33;
pub const PHOTO_FADE_MS: u64 = 1000;
pub const WORDMARK: &str = "Reprise";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Blank,
    Starfield,
    Mystify,
    Logo,
    Photos,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Blank,
        Kind::Starfield,
        Kind::Mystify,
        Kind::Logo,
        Kind::Photos,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Blank => "blank",
            Kind::Starfield => "starfield",
            Kind::Mystify => "mystify",
            Kind::Logo => "logo",
            Kind::Photos => "photos",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Blank => "Blank",
            Kind::Starfield => "Starfield",
            Kind::Mystify => "Mystify",
            Kind::Logo => "Bouncing logo",
            Kind::Photos => "Photos",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }

    pub fn animates(self) -> bool {
        matches!(self, Kind::Starfield | Kind::Mystify | Kind::Logo)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// 1 to 10: how fast Starfield, Mystify and the logo move; 5 is the
    /// usual pace.
    pub speed: u32,
    pub stars: u32,
    pub shapes: u32,
    /// Mystify and the logo: the colours change as they go (Mystify's
    /// slowly, the logo's at each bounce); off keeps the first ones.
    pub cycle_colors: bool,
    pub photo_seconds: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            speed: 5,
            stars: 200,
            shapes: 2,
            cycle_colors: true,
            photo_seconds: 10,
        }
    }
}

impl Options {
    pub const SPEED: (u32, u32) = (1, 10);
    pub const STARS: (u32, u32) = (10, 1000);
    pub const SHAPES: (u32, u32) = (1, 4);
    pub const PHOTO_SECONDS: (u32, u32) = (3, 600);

    pub fn clamped(&self) -> Options {
        let c = |v: u32, (lo, hi): (u32, u32)| v.clamp(lo, hi);
        Options {
            speed: c(self.speed, Self::SPEED),
            stars: c(self.stars, Self::STARS),
            shapes: c(self.shapes, Self::SHAPES),
            cycle_colors: self.cycle_colors,
            photo_seconds: c(self.photo_seconds, Self::PHOTO_SECONDS),
        }
    }

    fn pace(&self) -> f32 {
        self.speed as f32 / 5.0
    }
}

/// xorshift32: the savers' randomness, the same for the same seed.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) fn new(seed: u32) -> Rng {
        Rng(seed.max(1))
    }

    pub(crate) fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() >> 8) as f32 / (1u32 << 24) as f32 * (hi - lo)
    }
}

/// A star: where it points from the middle (`x`, `y` in -1..1) and how
/// far off it is (`z`, 1 far to 0 at the eye).
#[derive(Clone, Debug)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
}

const TRAIL: usize = 5;
const TRAIL_GAP: usize = 3;

#[derive(Clone, Debug)]
struct Shape {
    at: [(f32, f32); 4],
    vel: [(f32, f32); 4],
    hue: f32,
    history: Vec<[(f32, f32); 4]>,
}

const LOGO_COLORS: [Color; 6] = [
    Color::rgb(0xFF, 0xFF, 0xFF),
    Color::rgb(0x40, 0xC0, 0xFF),
    Color::rgb(0xFF, 0x60, 0x60),
    Color::rgb(0x60, 0xFF, 0x60),
    Color::rgb(0xFF, 0xD0, 0x40),
    Color::rgb(0xD0, 0x70, 0xFF),
];

#[derive(Clone, Debug, Default)]
struct Photos {
    shown: Option<Canvas>,
    incoming: Option<Canvas>,
    fade_ms: u64,
    held_ms: u64,
    asked: bool,
}

#[derive(Clone, Debug)]
enum Scene {
    Blank,
    Stars(Vec<Star>),
    Shapes(Vec<Shape>),
    Logo {
        word: Canvas,
        at: (f32, f32),
        vel: (f32, f32),
        color: usize,
    },
    Photos(Photos),
}

#[derive(Clone, Debug)]
pub struct Saver {
    pub kind: Kind,
    opts: Options,
    w: u32,
    h: u32,
    rng: Rng,
    scene: Scene,
    carry: u64,
}

impl Saver {
    pub fn new(
        kind: Kind,
        opts: &Options,
        w: u32,
        h: u32,
        seed: u32,
        text: &TextRenderer,
    ) -> Saver {
        let opts = opts.clamped();
        let (w, h) = (w.max(1), h.max(1));
        let mut rng = Rng::new(seed);
        let scene = match kind {
            Kind::Blank => Scene::Blank,
            Kind::Starfield => Scene::Stars(
                (0..opts.stars)
                    .map(|_| {
                        let z = rng.range(0.05, 1.0);
                        let (sx, sy) = (w as f32 / 2.0, h as f32 / 2.0);
                        let k = w.max(h) as f32 * 0.25 / z;
                        Star {
                            x: rng.range(-sx, sx) / k,
                            y: rng.range(-sy, sy) / k,
                            z,
                        }
                    })
                    .collect(),
            ),
            Kind::Mystify => {
                let step = w.min(h) as f32 / 180.0 * opts.pace();
                Scene::Shapes(
                    (0..opts.shapes)
                        .map(|i| {
                            let mut corner =
                                || (rng.range(0.0, w as f32), rng.range(0.0, h as f32));
                            let at = [corner(), corner(), corner(), corner()];
                            let mut heading = || {
                                let sx = if rng.next() & 1 == 0 { 1.0 } else { -1.0 };
                                let sy = if rng.next() & 1 == 0 { 1.0 } else { -1.0 };
                                (
                                    sx * rng.range(0.5, 1.5) * step,
                                    sy * rng.range(0.5, 1.5) * step,
                                )
                            };
                            let vel = [heading(), heading(), heading(), heading()];
                            Shape {
                                at,
                                vel,
                                hue: i as f32 * 360.0 / opts.shapes as f32,
                                history: Vec::new(),
                            }
                        })
                        .collect(),
                )
            }
            Kind::Logo => {
                let word = wordmark(text, (h / 9).max(8));
                let x = rng.range(0.0, (w as f32 - word.width as f32).max(0.0));
                let y = rng.range(0.0, (h as f32 - word.height as f32).max(0.0));
                let v = w.min(h) as f32 / 150.0 * opts.pace();
                Scene::Logo {
                    word,
                    at: (x, y),
                    vel: (v, v * 0.75),
                    color: 0,
                }
            }
            Kind::Photos => Scene::Photos(Photos::default()),
        };
        Saver {
            kind,
            opts,
            w,
            h,
            rng,
            scene,
            carry: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn advance(&mut self, ms: u64) {
        self.carry += ms;
        // A host that stalled for long (a suspend) skips ahead rather
        // than replaying every step.
        let steps = (self.carry / STEP_MS).min(60);
        self.carry %= STEP_MS;
        for _ in 0..steps {
            self.step();
        }
    }

    pub fn step(&mut self) {
        let (w, h) = (self.w as f32, self.h as f32);
        let pace = self.opts.pace();
        let cycle = self.opts.cycle_colors;
        let rng = &mut self.rng;
        match &mut self.scene {
            Scene::Blank => {}
            Scene::Stars(stars) => {
                for s in stars {
                    s.z -= 0.012 * pace;
                    let (px, py) = project(s, w, h);
                    if s.z <= 0.02 || px < 0.0 || py < 0.0 || px >= w || py >= h {
                        *s = Star {
                            x: rng.range(-1.0, 1.0),
                            y: rng.range(-1.0, 1.0),
                            z: 1.0,
                        };
                    }
                }
            }
            Scene::Shapes(shapes) => {
                for s in shapes {
                    s.history.insert(0, s.at);
                    s.history.truncate(TRAIL * TRAIL_GAP);
                    for (p, v) in s.at.iter_mut().zip(s.vel.iter_mut()) {
                        p.0 += v.0;
                        p.1 += v.1;
                        if p.0 < 0.0 || p.0 >= w {
                            v.0 = -v.0;
                            p.0 = p.0.clamp(0.0, w - 1.0);
                        }
                        if p.1 < 0.0 || p.1 >= h {
                            v.1 = -v.1;
                            p.1 = p.1.clamp(0.0, h - 1.0);
                        }
                    }
                    if cycle {
                        s.hue = (s.hue + 0.6) % 360.0;
                    }
                }
            }
            Scene::Logo {
                word,
                at,
                vel,
                color,
            } => {
                let (ww, wh) = (word.width as f32, word.height as f32);
                at.0 += vel.0;
                at.1 += vel.1;
                let mut bounced = false;
                if at.0 < 0.0 || at.0 + ww > w {
                    vel.0 = -vel.0;
                    at.0 = at.0.clamp(0.0, (w - ww).max(0.0));
                    bounced = true;
                }
                if at.1 < 0.0 || at.1 + wh > h {
                    vel.1 = -vel.1;
                    at.1 = at.1.clamp(0.0, (h - wh).max(0.0));
                    bounced = true;
                }
                if bounced && cycle {
                    *color = (*color + 1) % LOGO_COLORS.len();
                }
            }
            Scene::Photos(p) => {
                if p.incoming.is_some() {
                    p.fade_ms += STEP_MS;
                    if p.fade_ms >= PHOTO_FADE_MS {
                        p.shown = p.incoming.take();
                        p.fade_ms = 0;
                        p.held_ms = 0;
                    }
                } else if p.shown.is_some() {
                    p.held_ms += STEP_MS;
                }
            }
        }
    }

    pub fn take_photo_request(&mut self) -> bool {
        let photo_ms = self.opts.photo_seconds as u64 * 1000;
        let Scene::Photos(p) = &mut self.scene else {
            return false;
        };
        let due = p.incoming.is_none() && (p.shown.is_none() || p.held_ms >= photo_ms);
        if due && !p.asked {
            p.asked = true;
            return true;
        }
        false
    }

    pub fn push_photo(&mut self, picture: Canvas) {
        if let Scene::Photos(p) = &mut self.scene {
            p.incoming = Some(picture);
            p.fade_ms = 0;
            p.asked = false;
        }
    }

    /// Photos: how far the picture fading in is, 0 to 1; None while none
    /// fades. The compositor shows the pictures as two buffers and moves
    /// only the upper one's opacity.
    pub fn photo_fade(&self) -> Option<f32> {
        let Scene::Photos(p) = &self.scene else {
            return None;
        };
        p.incoming
            .as_ref()
            .map(|_| (p.fade_ms as f32 / PHOTO_FADE_MS as f32).min(1.0))
    }

    pub fn photo_wait_ms(&self) -> Option<u64> {
        let Scene::Photos(p) = &self.scene else {
            return None;
        };
        if p.incoming.is_some() {
            return Some(STEP_MS);
        }
        let photo_ms = self.opts.photo_seconds as u64 * 1000;
        p.shown
            .as_ref()
            .map(|_| photo_ms.saturating_sub(p.held_ms).max(STEP_MS))
    }

    /// The logo: its wordmark in its colour, on black. The compositor
    /// moves one buffer to [`Saver::logo_at`] and uploads the word again
    /// only when [`Saver::logo_color`] changes.
    pub fn logo(&self) -> Option<Canvas> {
        match &self.scene {
            Scene::Logo { word, color, .. } => Some(tint(word, LOGO_COLORS[*color])),
            _ => None,
        }
    }

    pub fn logo_at(&self) -> Option<(i32, i32)> {
        match &self.scene {
            Scene::Logo { at, .. } => Some((at.0 as i32, at.1 as i32)),
            _ => None,
        }
    }

    pub fn logo_color(&self) -> Option<usize> {
        match &self.scene {
            Scene::Logo { color, .. } => Some(*color),
            _ => None,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas) {
        let all = canvas.bounds();
        canvas.fill_rect(all, Color::rgb(0, 0, 0));
        let (w, h) = (self.w as f32, self.h as f32);
        match &self.scene {
            Scene::Blank => {}
            Scene::Stars(stars) => {
                let unit = (self.h / 300).max(1) as i32;
                for s in stars {
                    let (px, py) = project(s, w, h);
                    let level = ((1.0 - s.z) * 255.0).clamp(60.0, 255.0) as u8;
                    let size = if s.z < 0.3 { unit * 2 } else { unit };
                    canvas.fill_rect(
                        Rect::new(px as i32, py as i32, size, size),
                        Color::rgb(level, level, level),
                    );
                }
            }
            Scene::Shapes(shapes) => {
                let unit = (self.h / 400).max(1) as i32;
                for s in shapes {
                    let color = hue_color(s.hue);
                    let outlines = std::iter::once(&s.at).chain(
                        s.history
                            .iter()
                            .skip(TRAIL_GAP - 1)
                            .step_by(TRAIL_GAP)
                            .take(TRAIL - 1),
                    );
                    for o in outlines {
                        for i in 0..4 {
                            let (a, b) = (o[i], o[(i + 1) % 4]);
                            line(canvas, a, b, unit, color);
                        }
                    }
                }
            }
            Scene::Logo { .. } => {
                if let (Some(word), Some((x, y))) = (self.logo(), self.logo_at()) {
                    canvas.blit_canvas(x, y, &word);
                }
            }
            Scene::Photos(p) => {
                if let Some(c) = &p.shown {
                    canvas.blit_canvas(0, 0, c);
                }
                if let Some(next) = &p.incoming {
                    let level = (p.fade_ms as f32 / PHOTO_FADE_MS as f32).min(1.0);
                    if next.width == canvas.width && next.height == canvas.height {
                        canvas.mix(next, all, level);
                    }
                }
            }
        }
    }
}

fn project(s: &Star, w: f32, h: f32) -> (f32, f32) {
    let k = w.max(h) * 0.25;
    (w / 2.0 + s.x * k / s.z, h / 2.0 + s.y * k / s.z)
}

fn hue_color(hue: f32) -> Color {
    let h = (hue % 360.0) / 60.0;
    let x = ((1.0 - (h % 2.0 - 1.0).abs()) * 255.0) as u8;
    match h as u32 {
        0 => Color::rgb(255, x, 0),
        1 => Color::rgb(x, 255, 0),
        2 => Color::rgb(0, 255, x),
        3 => Color::rgb(0, x, 255),
        4 => Color::rgb(x, 0, 255),
        _ => Color::rgb(255, 0, x),
    }
}

fn line(canvas: &mut Canvas, a: (f32, f32), b: (f32, f32), unit: i32, color: Color) {
    let (mut x0, mut y0) = (a.0 as i32, a.1 as i32);
    let (x1, y1) = (b.0 as i32, b.1 as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if unit == 1 {
            canvas.put(x0, y0, color);
        } else {
            canvas.fill_rect(Rect::new(x0, y0, unit, unit), color);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

pub fn wordmark(text: &TextRenderer, size_px: u32) -> Canvas {
    let pad = (size_px / 4) as i32;
    let w = text.measure(WORDMARK, true, size_px) + 2 * pad;
    let h = (size_px as i32 * 3 / 2) + 2 * pad;
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.fill_rect(c.bounds(), Color::rgb(0, 0, 0));
    text.draw(
        &mut c,
        Rect::new(pad, pad, w - 2 * pad, h - 2 * pad),
        WORDMARK,
        true,
        Color::rgb(0xFF, 0xFF, 0xFF),
        Align::Center,
        size_px,
    );
    c
}

fn tint(white: &Canvas, color: Color) -> Canvas {
    let mut out = white.clone();
    let scale = |v: u32, c: u8| (v * c as u32 + 127) / 255;
    for p in &mut out.pixels {
        let v = *p & 0xFF;
        *p = 0xFF00_0000 | scale(v, color.r) << 16 | scale(v, color.g) << 8 | scale(v, color.b);
    }
    out
}

pub fn fit(src: &Canvas, w: u32, h: u32) -> Canvas {
    let mut out = Canvas::new(w.max(1), h.max(1));
    out.fill_rect(out.bounds(), Color::rgb(0, 0, 0));
    if src.width == 0 || src.height == 0 {
        return out;
    }
    let k = (w as f32 / src.width as f32).min(h as f32 / src.height as f32);
    let (fw, fh) = (
        ((src.width as f32 * k).round() as u32).clamp(1, w.max(1)),
        ((src.height as f32 * k).round() as u32).clamp(1, h.max(1)),
    );
    let scaled = src.resample_box(fw, fh);
    out.blit_canvas(
        (w as i32 - fw as i32) / 2,
        (h as i32 - fh as i32) / 2,
        &scaled,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    const PREVIEW: (u32, u32) = (152, 112);

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn run(kind: Kind, opts: &Options, steps: usize) -> (Saver, Canvas) {
        let mut s = Saver::new(kind, opts, PREVIEW.0, PREVIEW.1, 7, &text());
        for _ in 0..steps {
            s.step();
        }
        let mut c = Canvas::new(PREVIEW.0, PREVIEW.1);
        s.draw(&mut c);
        (s, c)
    }

    fn digest(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, p| {
            (h ^ *p as u64).wrapping_mul(0x100_0000_01b3)
        })
    }

    fn lit(c: &Canvas) -> usize {
        c.pixels.iter().filter(|&&p| p & 0xFF_FFFF != 0).count()
    }

    #[test]
    fn names_read_back_and_unknown_ones_are_refused() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
        }
        assert_eq!(Kind::parse(" Starfield "), Some(Kind::Starfield));
        assert_eq!(Kind::parse("flying toasters"), None);
    }

    #[test]
    fn options_are_held_to_their_ranges() {
        let o = Options {
            speed: 0,
            stars: 5000,
            shapes: 9,
            cycle_colors: false,
            photo_seconds: 1,
        }
        .clamped();
        assert_eq!(
            (o.speed, o.stars, o.shapes, o.photo_seconds),
            (1, 1000, 4, 3)
        );
    }

    #[test]
    fn blank_is_black() {
        let (_, c) = run(Kind::Blank, &Options::default(), 10);
        assert_eq!(lit(&c), 0);
        assert_eq!(c.get(0, 0), 0xFF00_0000);
    }

    #[test]
    fn advance_runs_whole_steps_and_keeps_the_rest() {
        let opts = Options::default();
        let mut a = Saver::new(Kind::Starfield, &opts, 152, 112, 3, &text());
        let mut b = a.clone();
        a.advance(STEP_MS * 2 + 10);
        a.advance(STEP_MS - 10);
        for _ in 0..3 {
            b.step();
        }
        let (mut ca, mut cb) = (Canvas::new(152, 112), Canvas::new(152, 112));
        a.draw(&mut ca);
        b.draw(&mut cb);
        assert_eq!(ca.pixels, cb.pixels, "three steps either way");
    }

    #[test]
    fn the_starfield_preview_frame() {
        let (_, c) = run(Kind::Starfield, &Options::default(), 20);
        assert_eq!(
            digest(&c),
            11154973060832422393,
            "starfield frame 20, seed 7"
        );
        assert_eq!(lit(&c), 213, "lit pixels");
        let (_, again) = run(Kind::Starfield, &Options::default(), 20);
        assert_eq!(again.pixels, c.pixels, "the same seed draws the same frame");
    }

    #[test]
    fn stars_fly_out_from_the_middle() {
        let opts = Options::default();
        let mut s = Saver::new(Kind::Starfield, &opts, 152, 112, 7, &text());
        let Scene::Stars(before) = s.scene.clone() else {
            unreachable!()
        };
        s.step();
        let Scene::Stars(after) = &s.scene else {
            unreachable!()
        };
        let off = |st: &Star| {
            let (x, y) = project(st, 152.0, 112.0);
            ((x - 76.0).powi(2) + (y - 56.0).powi(2)).sqrt()
        };
        for (a, b) in before.iter().zip(after) {
            if b.z < a.z {
                assert!(off(b) >= off(a), "a star coming nearer moves out");
            } else {
                assert_eq!(b.z, 1.0, "a star that left starts again far off");
            }
        }
        let respawned = after.iter().filter(|b| b.z == 1.0).count();
        assert!(respawned < 30, "{respawned} stars left in one step");
    }

    #[test]
    fn the_mystify_preview_frame() {
        let (_, c) = run(Kind::Mystify, &Options::default(), 40);
        assert_eq!(digest(&c), 18129435732843836132, "mystify frame 40, seed 7");
        assert_eq!(lit(&c), 1686, "lit pixels");
    }

    #[test]
    fn mystify_shapes_stay_on_screen_and_trail_outlines() {
        let opts = Options {
            speed: 10,
            ..Options::default()
        };
        let (s, _) = run(Kind::Mystify, &opts, 500);
        let Scene::Shapes(shapes) = &s.scene else {
            unreachable!()
        };
        assert_eq!(shapes.len(), 2);
        for sh in shapes {
            assert_eq!(sh.history.len(), TRAIL * TRAIL_GAP);
            for (x, y) in sh.at {
                assert!((0.0..152.0).contains(&x) && (0.0..112.0).contains(&y));
            }
        }
    }

    #[test]
    fn the_logo_preview_frame_and_its_bounce() {
        let (s, c) = run(Kind::Logo, &Options::default(), 30);
        if crate::text::test_tahoma() {
            assert_eq!(digest(&c), 3579411078834488872, "logo frame 30, seed 7");
        }
        let (word, (x, y)) = (s.logo().unwrap(), s.logo_at().unwrap());
        assert!(x >= 0 && y >= 0);
        assert!(x + word.width as i32 <= 152 && y + word.height as i32 <= 112);
        let mut s = Saver::new(Kind::Logo, &Options::default(), 152, 112, 7, &text());
        let first = s.logo_color();
        for _ in 0..400 {
            s.step();
        }
        assert_ne!(
            s.logo_color(),
            first,
            "a bounce in 400 steps changes the colour"
        );
        let still = Options {
            cycle_colors: false,
            ..Options::default()
        };
        let mut s = Saver::new(Kind::Logo, &still, 152, 112, 7, &text());
        for _ in 0..400 {
            s.step();
        }
        assert_eq!(s.logo_color(), Some(0));
    }

    #[test]
    fn the_logo_word_is_white_tinted_to_its_colour() {
        let mut white = Canvas::new(2, 1);
        white.pixels = vec![0xFFFF_FFFF, 0xFF80_8080];
        let t = tint(&white, Color::rgb(0x40, 0xC0, 0xFF));
        assert_eq!(t.pixels, vec![0xFF40_C0FF, 0xFF20_6080]);
    }

    #[test]
    fn photos_ask_once_fade_in_then_hold() {
        let opts = Options {
            photo_seconds: 3,
            ..Options::default()
        };
        let mut s = Saver::new(Kind::Photos, &opts, 4, 2, 1, &text());
        assert!(
            s.take_photo_request(),
            "the first picture is asked for at once"
        );
        assert!(!s.take_photo_request(), "and only once");
        assert_eq!(s.photo_wait_ms(), None, "nothing changes until it comes");
        let mut red = Canvas::new(4, 2);
        red.fill_rect(red.bounds(), Color::rgb(255, 0, 0));
        s.push_photo(red);
        for _ in 0..15 {
            s.step();
        }
        let mut c = Canvas::new(4, 2);
        s.draw(&mut c);
        assert_eq!(c.get(0, 0), 0xFF7E_0000, "red at 495 of 1000 ms over black");
        for _ in 0..16 {
            s.step();
        }
        assert_eq!(s.photo_fade(), None, "the fade has ended");
        assert_eq!(s.photo_wait_ms(), Some(3000));
        assert!(!s.take_photo_request());
        for _ in 0..91 {
            s.step();
        }
        assert!(s.take_photo_request(), "the next is asked for after 3 s");
    }

    #[test]
    fn a_photo_is_fitted_whole_and_centred() {
        let mut wide = Canvas::new(8, 2);
        wide.fill_rect(wide.bounds(), Color::rgb(0, 0, 255));
        let f = fit(&wide, 4, 4);
        assert_eq!((f.width, f.height), (4, 4));
        assert_eq!(f.get(0, 0), 0xFF00_0000, "black above");
        assert_eq!(f.get(0, 1), 0xFF00_00FF, "4x1 on row 1");
        assert_eq!(f.get(3, 1), 0xFF00_00FF);
        assert_eq!(f.get(0, 2), 0xFF00_0000, "black below");
    }
}
