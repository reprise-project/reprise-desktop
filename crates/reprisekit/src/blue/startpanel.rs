//! Reprise Blue's Start menu, drawn from a [`Scheme`]'s [`StartMenu`] colours at the output scale
//! over the rects of `crate::startpanel::bluelayout`.

use crate::blue::powerpanel::{self, TileState};
use crate::blue::scheme::{Scheme, StartMenu};
use crate::powerpanel::Choice;
use crate::raster::{Canvas, Rect};
use crate::shape::{self, CLEAR, Px, Stops};
use crate::startpanel::bluelayout::BUTTON_PEN;
use crate::startpanel::layout::Layout;
use crate::startpanel::search::Grow;
use crate::startpanel::{PROGRAM_ICON, Spot, StartPanel, all_programs, log_off, turn_off};
use crate::style::State;
use crate::text::TextRenderer;
use crate::theme::Color;

pub const TEXT_PX: u32 = 11;
pub const NAME_PX: u32 = 17;
const TOP_CORNER: f64 = 7.0;
const FOOT_CORNER: f64 = 5.0;
const ACCENT_H: i32 = 2;
const PICTURE_PX: i32 = 48;
const PICTURE_CORNER: f64 = 3.0;
/// The places' icons, design px at 100% text: they grow with the text as the program rows'
/// [`PROGRAM_ICON`] do.
pub const PLACE_ICON_PX: i32 = 24;
const TILE_PX: i32 = 24;
const ARROW: (i32, i32) = (16, 18);
/// How far a pinned row's detail line goes from the row's ink toward the column, out of 255.
const DETAIL: u8 = 0x80;
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

/// Where things sit in their rects at 100% text, in design px from the rect's top-left.
mod at {
    pub const NAME_GAP: i32 = 9;
    pub const NAME_BASE: i32 = 36;
    pub const PROGRAM_ICON: (i32, i32) = (4, 4);
    pub const PROGRAM_PEN: i32 = 42;
    pub const PROGRAM_BASE: i32 = 24;
    pub const NAME_LINE: i32 = 18;
    pub const DETAIL_LINE: i32 = 31;
    pub const ALL_BASE: i32 = 17;
    pub const ARROW_GAP: i32 = 8;
    pub const PLACE_ICON: (i32, i32) = (4, 3);
    pub const PLACE_PEN: i32 = 33;
    pub const PLACE_BASE: i32 = 19;
    pub const PLACE_ARROW: (i32, i32) = (14, 11);
    pub const TILE: (i32, i32) = (2, 1);
    pub const BUTTON_BASE: i32 = 17;
}

/// [`StartPanel::foot_labels`] as [`draw`] sets the labels with `text` at `g`: each label's
/// device width as the shortest 100% length that grows to it.
pub fn foot_labels(text: &TextRenderer, g: Grow) -> [i32; 2] {
    let size = g.px(TEXT_PX);
    [&log_off(), &turn_off()].map(|label| {
        let w = text.measure(label, false, size);
        g.u.design((w + g.s - 1) / g.s)
    })
}

/// Paints the Blue Start menu `p` laid out as `l` (device px, `l.scale` per design px) onto a
/// clear `canvas` the size of `l.canvas`, in `s`'s colours; `pressed` is the spot a button is held on.
pub fn draw(
    canvas: &mut Canvas,
    s: &Scheme,
    text: &TextRenderer,
    p: &StartPanel,
    l: &Layout,
    pressed: Option<Spot>,
) {
    let d = l.scale.max(1);
    let o = l.canvas;
    let at = |r: Rect| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height);
    let g = Grow { s: d, u: p.text };
    let m = &s.start_menu;
    let panel = at(l.panel);
    let pane = at(l.pane);
    let edge = pane.x - panel.x;
    let places = Rect::new(
        pane.right(),
        pane.y,
        panel.right() - edge - pane.right(),
        pane.height,
    );

    let band = Rect::new(panel.x, panel.y, panel.width, pane.y - panel.y);
    canvas.blit_over(band.x, band.y, &top_band(m, band.width, band.height, d));
    let foot = Rect::new(
        panel.x,
        pane.bottom(),
        panel.width,
        panel.bottom() - pane.bottom(),
    );
    canvas.blit_over(foot.x, foot.y, &foot_band(m, foot.width, foot.height, d));
    canvas.fill_rect(Rect::new(panel.x, pane.y, edge, pane.height), m.edge);
    canvas.fill_rect(Rect::new(places.right(), pane.y, edge, pane.height), m.edge);
    canvas.fill_rect(pane, s.system.window);
    canvas.fill_rect(places, m.places);
    canvas.fill_rect(
        Rect::new(places.x, places.y, d, places.height),
        m.places_edge,
    );

    for &r in &l.rules {
        let r = at(r);
        let colour = if r.x >= places.x {
            m.places_edge
        } else {
            m.rule
        };
        rule(canvas, r, colour);
    }

    if let Some(frame) = l.picture.map(at) {
        picture(canvas, m, frame, p.user_picture.as_ref(), d);
        let x = frame.right() + g.len(at::NAME_GAP);
        let y = band.y + g.len(at::NAME_BASE);
        let clip = Rect::new(x, band.y, band.right() - x - g.len(8), band.height);
        let size = g.px(NAME_PX);
        let shown = text.ellipsize(&p.user, clip.width, true, size);
        text.draw_run_over(
            canvas,
            x + d,
            y + d,
            clip,
            &shown,
            true,
            m.band_shadow,
            size,
        );
        text.draw_run_over(canvas, x, y, clip, &shown, true, m.band_ink, size);
    }

    let lit = p.lit();
    let size = g.px(TEXT_PX);
    let icon_at = |r: Rect, (dx, dy): (i32, i32), size: i32, c: &Canvas| {
        let (w, h) = (c.width as i32, c.height as i32);
        (r.x + g.centred(dx, size, w), r.y + g.centred(dy, size, h))
    };
    let first_rule = p
        .places
        .iter()
        .skip(1)
        .position(|pl| pl.rule_before)
        .map_or(p.places.len(), |k| k + 1);
    let hot = s.system.menu_hilight;
    let hot_ink = s.system.hilight_text;
    for &(spot, r) in &l.spots {
        let r = at(r);
        let is_lit = lit == Some(spot);
        // `keyed`: the label's first letter is the row's jump letter.
        let line = |canvas: &mut Canvas, pen, base, label: &str, bold, ink, keyed: bool| {
            let x = r.x + g.len(pen);
            let clip = Rect::new(x, r.y, r.right() - x - 2 * d, r.height);
            let shown = text.ellipsize(label, clip.width, bold, size);
            let base = r.y + g.len(base);
            text.draw_run_over(canvas, x, base, clip, &shown, bold, ink, size);
            if keyed && p.keys && shown.chars().next() == label.chars().next() {
                key_mark(canvas, text, x, base, clip, &shown, bold, ink, size, d);
            }
        };
        match spot {
            Spot::Program(i) => {
                let Some(row) = p.program(i) else { continue };
                if is_lit {
                    canvas.fill_rect(r, hot);
                }
                if let Some(icon) = &row.icon {
                    let (x, y) = icon_at(r, at::PROGRAM_ICON, PROGRAM_ICON, icon);
                    canvas.blit_over(x, y, icon);
                }
                let ink = if is_lit { hot_ink } else { s.system.menu_text };
                let pinned = i < p.pinned.len();
                match row.detail.as_deref().filter(|_| pinned) {
                    Some(detail) => {
                        let name = &row.name;
                        line(
                            canvas,
                            at::PROGRAM_PEN,
                            at::NAME_LINE,
                            name,
                            true,
                            ink,
                            true,
                        );
                        let grey = if is_lit {
                            hot_ink
                        } else {
                            s.system.menu_text.toward(s.system.window, DETAIL)
                        };
                        line(
                            canvas,
                            at::PROGRAM_PEN,
                            at::DETAIL_LINE,
                            detail,
                            false,
                            grey,
                            false,
                        );
                    }
                    None => line(
                        canvas,
                        at::PROGRAM_PEN,
                        at::PROGRAM_BASE,
                        &row.name,
                        pinned,
                        ink,
                        true,
                    ),
                }
            }
            Spot::AllPrograms => {
                if is_lit {
                    canvas.fill_rect(r, hot);
                }
                let ink = if is_lit { hot_ink } else { s.system.menu_text };
                let w = text.measure(&all_programs(), true, size);
                // The arrows grow by whole steps with the text, each a fresh drawing.
                let ad = d * p.text.small();
                let (aw, ah) = (ARROW.0 * ad, ARROW.1 * ad);
                let gap = g.len(at::ARROW_GAP);
                let x = r.x + (r.width - w - gap - aw) / 2;
                let clip = Rect::new(r.x, r.y, r.width, r.height);
                let y = r.y + g.len(at::ALL_BASE);
                text.draw_run_over(canvas, x, y, clip, &all_programs(), true, ink, size);
                if p.keys {
                    key_mark(
                        canvas,
                        text,
                        x,
                        y,
                        clip,
                        &all_programs(),
                        true,
                        ink,
                        size,
                        d,
                    );
                }
                let ay = r.y + (r.height - ah) / 2;
                canvas.blit_over(x + w + gap, ay, &arrow(s, ad));
            }
            Spot::Place(i) => {
                let Some(place) = p.places.get(i) else {
                    continue;
                };
                if is_lit {
                    canvas.fill_rect(r, hot);
                }
                if let Some(icon) = &place.picture {
                    let (x, y) = icon_at(r, at::PLACE_ICON, PLACE_ICON_PX, icon);
                    canvas.blit_over(x, y, icon);
                }
                let ink = if is_lit { hot_ink } else { m.places_ink };
                line(
                    canvas,
                    at::PLACE_PEN,
                    at::PLACE_BASE,
                    &place.label,
                    i < first_rule,
                    ink,
                    true,
                );
                if place.submenu {
                    let (ax, ay) = at::PLACE_ARROW;
                    let ad = d * p.text.small();
                    let mark = shape::glyph(
                        shape::Glyph::ArrowRight,
                        7,
                        ad,
                        1.0,
                        shape::premul(ink, 255),
                    );
                    let x = r.right() - g.len(ax);
                    canvas.blit_over(x, r.y + g.stepped(ay - 3, 7), &mark);
                }
            }
            Spot::LogOff | Spot::Shutdown => {
                let (choice, label) = if spot == Spot::LogOff {
                    (Choice::LogOff, &log_off())
                } else {
                    (Choice::TurnOff, &turn_off())
                };
                let state = if pressed == Some(spot) {
                    State::Pressed
                } else if is_lit {
                    State::Hot
                } else {
                    State::Normal
                };
                if state != State::Normal {
                    canvas.blit_over(r.x, r.y, &button_light(r, d, state));
                }
                let tile = tile_image(choice, p.icon_px(TILE_PX, 1), d, state);
                let (x, y) = icon_at(r, at::TILE, TILE_PX, &tile);
                canvas.blit_over(x, y, &tile);
                let pen = r.x + g.len(BUTTON_PEN);
                let base = r.y + g.len(at::BUTTON_BASE);
                let clip = Rect::new(pen, r.y, r.right() - pen, r.height);
                text.draw_run_over(
                    canvas,
                    pen + d,
                    base + d,
                    clip,
                    label,
                    false,
                    m.band_shadow,
                    size,
                );
                text.draw_run_over(canvas, pen, base, clip, label, false, m.band_ink, size);
                if p.keys {
                    let (sx, sb) = (pen + d, base + d);
                    key_mark(
                        canvas,
                        text,
                        sx,
                        sb,
                        clip,
                        label,
                        false,
                        m.band_shadow,
                        size,
                        d,
                    );
                    key_mark(
                        canvas, text, pen, base, clip, label, false, m.band_ink, size, d,
                    );
                }
            }
            _ => {}
        }
    }
}

/// The access-key underline under `shown`'s first letter, set at pen `x` on baseline `base`: `d`
/// px under the baseline and `d` thick, inside `clip`.
#[expect(clippy::too_many_arguments)]
fn key_mark(
    canvas: &mut Canvas,
    text: &TextRenderer,
    x: i32,
    base: i32,
    clip: Rect,
    shown: &str,
    bold: bool,
    ink: Color,
    size: u32,
    d: i32,
) {
    let (dx, w) = text.span(shown, 0, bold, size);
    canvas.fill_rect(Rect::new(x + dx, base + d, w, d).intersect(&clip), ink);
}

fn stops(m: &StartMenu) -> Vec<(f64, Px)> {
    m.band
        .iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

/// The top band, `w` x `h` device px: `band` down it, its top corners rounded, the accent line
/// along its bottom fading out toward the sides.
fn top_band(m: &StartMenu, w: i32, h: i32, d: i32) -> Canvas {
    let grad = stops(m);
    let grad = Stops(&grad);
    let k = f64::from(d);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let accent = shape::premul(m.accent, 255);
    let line = ACCENT_H * d;
    shape::draw(w, h, 1, |x, y| {
        let cover = shape::rounded4(
            x,
            y,
            fw,
            fh + 2.0 * TOP_CORNER * k,
            [TOP_CORNER * k, TOP_CORNER * k, 0.0, 0.0],
        );
        if cover == 0.0 {
            return CLEAR;
        }
        let base = grad.at((y + 0.5) / fh);
        let px = if y as i32 >= h - line {
            let across = 1.0 - ((x + 0.5) / fw * 2.0 - 1.0).abs();
            shape::mix(base, accent, shape::smooth(across * 3.0))
        } else {
            base
        };
        shape::scale(px, cover)
    })
}

/// The bottom band, `w` x `h` device px: `band` down it, its bottom corners rounded.
fn foot_band(m: &StartMenu, w: i32, h: i32, d: i32) -> Canvas {
    let grad = stops(m);
    let grad = Stops(&grad);
    let k = f64::from(d);
    let (fw, fh) = (f64::from(w), f64::from(h));
    shape::draw(w, h, 1, |x, y| {
        let cover = shape::rounded4(
            x,
            y + 2.0 * FOOT_CORNER * k,
            fw,
            fh + 2.0 * FOOT_CORNER * k,
            [0.0, 0.0, FOOT_CORNER * k, FOOT_CORNER * k],
        );
        shape::scale(grad.at((y + 0.5) / fh), cover)
    })
}

/// A rule: `colour` along `r`, fading in from its ends.
fn rule(canvas: &mut Canvas, r: Rect, colour: Color) {
    let line = shape::draw(r.width, r.height, 1, |x, _| {
        let t = (x + 0.5) / f64::from(r.width);
        let a = shape::smooth((1.0 - (2.0 * t - 1.0).abs()) * 4.0);
        shape::premul(colour, (a * 255.0).round() as u8)
    });
    canvas.blit_over(r.x, r.y, &line);
}

/// The user picture in a white rounded frame with a soft shadow under its right and bottom edges.
fn picture(canvas: &mut Canvas, m: &StartMenu, frame: Rect, pic: Option<&Canvas>, d: i32) {
    let k = f64::from(d);
    let (w, h) = (f64::from(frame.width), f64::from(frame.height));
    let shadow = shape::premul(m.band_shadow, 0x70);
    let white = shape::premul(WHITE, 255);
    let r = PICTURE_CORNER * k;
    let img = shape::draw(frame.width + d, frame.height + d, 1, |x, y| {
        let body = shape::rounded(x, y, w, h, r);
        let under = shape::rounded(x - k, y - k, w, h, r);
        shape::over(shape::scale(white, body), shape::scale(shadow, under))
    });
    canvas.blit_over(frame.x, frame.y, &img);
    let Some(pic) = pic else {
        return;
    };
    let side = PICTURE_PX * d;
    let x = frame.x + (frame.width - side) / 2 + (side - pic.width as i32) / 2;
    let y = frame.y + (frame.height - side) / 2 + (side - pic.height as i32) / 2;
    canvas.blit_over(x, y, pic);
}

/// All Programs' arrow: a right-pointing triangle with rounded corners, light at its top-left and
/// dark at its tip, in the Start button's greens.
fn arrow(s: &Scheme, d: i32) -> Canvas {
    let f = &s.start_button.normal;
    let (light, dark) = (shape::premul(f.inner, 255), shape::premul(f.rim, 255));
    let k = f64::from(d);
    let (w, h) = (f64::from(ARROW.0), f64::from(ARROW.1));
    let pts = [
        (1.0 * k, 1.0 * k),
        ((w - 1.0) * k, h / 2.0 * k),
        (1.0 * k, (h - 1.0) * k),
    ];
    let edge = shape::premul(Color::rgb(0x2E, 0x6E, 0x32), 255);
    shape::draw(ARROW.0, ARROW.1, d, |x, y| {
        let outer = shape::polygon(x, y, &pts);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = shape::polygon(
            x,
            y,
            &[
                (2.2 * k, 2.8 * k),
                ((w - 3.2) * k, h / 2.0 * k),
                (2.2 * k, (h - 2.8) * k),
            ],
        );
        let cx = shape::centre(x as i32, d) / w;
        let cy = shape::centre(y as i32, d) / h;
        let fill = shape::mix(light, dark, (cx * 0.7 + cy * 0.5).clamp(0.0, 1.0));
        shape::shares(&[(edge, outer - inner), (fill, inner)])
    })
}

/// The see-through box a lit or held bottom-band button stands in, so the keyboard's stop shows.
fn button_light(r: Rect, d: i32, state: State) -> Canvas {
    let fill = match state {
        State::Pressed => shape::premul(Color::rgb(0, 0, 0), 0x24),
        _ => shape::premul(WHITE, 0x38),
    };
    let (w, h) = (f64::from(r.width), f64::from(r.height));
    let corner = 3.0 * f64::from(d);
    shape::draw(r.width, r.height, 1, |x, y| {
        shape::scale(fill, shape::rounded(x, y, w, h, corner))
    })
}

/// A bottom-band button's tile, `size` design px: the power panel's tile for `choice`, lit while
/// hot or held.
fn tile_image(choice: Choice, size: i32, d: i32, state: State) -> Canvas {
    let state = match state {
        State::Hot => TileState::Hot,
        State::Pressed => TileState::Pressed,
        _ => TileState::Normal,
    };
    powerpanel::icon(choice, state, size, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::{Tone, Variant};
    use crate::startpanel::layout::layout;
    use crate::startpanel::{PlaceRow, ProgramRow, Shape};
    use crate::theme::TextUnits;

    fn prog(name: &str, detail: Option<&str>) -> ProgramRow {
        ProgramRow {
            name: name.into(),
            icon: None,
            jump: false,
            new_pin: false,
            detail: detail.map(String::from),
        }
    }

    fn sample(hot: Option<Spot>) -> StartPanel {
        let mut p = StartPanel::new();
        p.shape = Shape::Blue;
        p.user = "user".into();
        p.pinned = vec![prog("Browser", Some("Web Browser"))];
        p.frequent = (0..6)
            .map(|k| prog(&format!("Program {k}"), None))
            .collect();
        p.places = [
            ("Documents", false),
            ("Recent Items", false),
            ("Computer", false),
            ("Control Panel", true),
            ("Run...", true),
        ]
        .iter()
        .map(|&(label, rule)| PlaceRow {
            label: label.into(),
            picture: None,
            rule_before: rule,
            submenu: label == "Recent Items",
            context: false,
        })
        .collect();
        p.pointer(hot);
        p
    }

    fn render(
        v: impl Into<Tone>,
        p: &StartPanel,
        d: i32,
        pressed: Option<Spot>,
    ) -> (Canvas, Layout) {
        let text = TextRenderer::stub(TEXT_PX * d as u32, |_| 6);
        let mut p = p.clone();
        p.foot_labels = foot_labels(&text, Grow::at(d));
        let p = &p;
        let l = layout(p).scaled(d);
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        draw(&mut c, v.into().scheme(), &text, p, &l, pressed);
        (c, l)
    }

    use crate::pins::fnv;

    const STATES: [(Option<Spot>, Option<Spot>); 6] = [
        (None, None),
        (Some(Spot::Program(0)), None),
        (Some(Spot::Place(1)), None),
        (Some(Spot::AllPrograms), None),
        (Some(Spot::LogOff), None),
        (Some(Spot::Shutdown), Some(Spot::Shutdown)),
    ];

    fn hashes(v: Tone, d: i32) -> Vec<u64> {
        STATES
            .iter()
            .map(|&(hot, pressed)| fnv(&render(v, &sample(hot), d, pressed).0))
            .collect()
    }

    /// All Programs' green arrow grows by whole steps with the text: twice as wide and tall at
    /// 200% as at 100%, at scale 1 and 2.
    #[test]
    fn the_all_programs_arrow_grows_by_whole_steps_with_the_text() {
        let green_box = |u: TextUnits, d: i32| {
            let mut p = sample(None);
            p.text = u;
            let (c, l) = render(Variant::Blue, &p, d, None);
            let (_, r) = *l
                .spots
                .iter()
                .find(|(s, _)| *s == Spot::AllPrograms)
                .unwrap();
            let r = Rect::new(r.x - l.canvas.x, r.y - l.canvas.y, r.width, r.height);
            let green: Vec<(i32, i32)> = (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let [b, g, r, a] = c.get(x, y).to_le_bytes();
                    a == 0xFF && g as i32 > r as i32 + 40 && g as i32 > b as i32 + 40
                })
                .collect();
            let extent = |f: fn(&(i32, i32)) -> i32| {
                green.iter().map(f).max().unwrap() - green.iter().map(f).min().unwrap() + 1
            };
            (extent(|p| p.0), extent(|p| p.1))
        };
        for d in [1, 2] {
            let one = green_box(TextUnits::ONE, d);
            let two = green_box(TextUnits::new(22, 11), d);
            assert!(one.0 >= 10 * d, "@{d}: the arrow inks {one:?}");
            assert!(
                (two.0 - 2 * one.0).abs() <= d && (two.1 - 2 * one.1).abs() <= d,
                "@{d}: {two:?} at 200%, twice {one:?}"
            );
        }
    }

    #[test]
    fn every_state_draws_its_own_pixels_in_blue_olive_and_every_twin_at_scale_1_and_2() {
        let cases: Vec<_> = [Variant::Blue.into(), Variant::Olive.into()]
            .into_iter()
            .chain(Variant::ALL.map(|v| Tone::new(v, true)))
            .flat_map(|v| [1, 2].map(|d| (v, d)))
            .collect();
        let got: Vec<Vec<u64>> = cases.iter().map(|&(v, d)| hashes(v, d)).collect();
        for h in &got {
            let mut seen = h.clone();
            seen.dedup();
            assert_eq!(seen.len(), STATES.len(), "every state differs");
        }
        assert_ne!(got[0], got[2], "Olive draws its own colours");
        assert_ne!(got[0], got[4], "the twin draws its own colours");
        let keyed: Vec<_> = cases
            .iter()
            .zip(&got)
            .flat_map(|(&(v, d), h)| {
                STATES.iter().zip(h).map(move |((hot, pressed), &h)| {
                    (format!("{v:?} {d} hot={hot:?} pressed={pressed:?}"), h)
                })
            })
            .collect();
        crate::pins::check("blue/start_panel", &keyed);
    }

    #[test]
    fn the_corners_are_clear_and_round_at_scale_2() {
        let (c, _) = render(Variant::Blue, &sample(None), 2, None);
        let (w, h) = (c.width as i32, c.height as i32);
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(c.get(x, y) >> 24, 0, "corner ({x}, {y}) is clear");
        }
        let partial = |x0: i32, y0: i32| {
            (0..14).any(|y| {
                (0..14).any(|x| {
                    let a = c.get(x0 + x, y0 + y) >> 24;
                    a > 0 && a < 255
                })
            })
        };
        assert!(partial(0, 0), "the top corner's curve is smooth");
        assert!(partial(0, h - 14), "the bottom corner's curve is smooth");
        assert_eq!(c.get(w / 2, h / 2) >> 24, 255, "the body is solid");
    }

    #[test]
    fn a_lit_row_fills_with_the_selection_and_the_places_with_their_colour() {
        let s = Variant::Blue.scheme();
        let px =
            |c: Color| 0xFF00_0000 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b);
        let p = sample(Some(Spot::Place(2)));
        let (c, l) = render(Variant::Blue, &p, 1, None);
        let r = l.rect(Spot::Place(2)).unwrap();
        assert_eq!(c.get(r.right() - 3, r.y + 1), px(s.system.menu_hilight));
        let r = l.rect(Spot::Place(0)).unwrap();
        assert_eq!(c.get(r.right() - 3, r.y + 1), px(s.start_menu.places));
        let r = l.rect(Spot::Program(3)).unwrap();
        assert_eq!(c.get(r.right() - 3, r.y + 1), px(s.system.window));
        let accent = c.get(l.panel.width / 2, l.pane.y - 1);
        assert_eq!(accent, px(s.start_menu.accent), "the accent line's middle");
    }

    #[test]
    fn a_tile_has_three_states_a_clear_corner_and_a_smooth_curve_at_scale_2() {
        for tile in [Choice::LogOff, Choice::TurnOff] {
            let states = [State::Normal, State::Hot, State::Pressed]
                .map(|st| tile_image(tile, TILE_PX, 2, st));
            assert_ne!(states[0].pixels, states[1].pixels, "{tile:?} hot");
            assert_ne!(states[0].pixels, states[2].pixels, "{tile:?} pressed");
            let c = &states[0];
            assert_eq!((c.width, c.height), (48, 48));
            assert_eq!(c.get(0, 0) >> 24, 0, "{tile:?} corner");
            assert!(
                (0..6).any(|k| (1..255).contains(&(c.get(k, 1) >> 24))),
                "{tile:?} curve"
            );
            let one = tile_image(tile, TILE_PX, 1, State::Normal);
            assert_eq!((one.width, one.height), (24, 24));
        }
    }

    /// Opened from the keyboard, each row underlines its first letter in its ink, `d` px under
    /// the baseline and `d` thick, a lit row in the lit ink and the bottom band's buttons with
    /// their labels' shadow; nothing else moves.
    #[test]
    fn opened_from_the_keyboard_each_row_underlines_its_first_letter() {
        let s = Variant::Blue.scheme();
        let px =
            |c: Color| 0xFF00_0000 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b);
        for d in [1, 2] {
            let g = Grow::at(d);
            let lit = sample(Some(Spot::Place(2)));
            let mut keyed = lit.clone();
            keyed.keys = true;
            let (plain, _) = render(Variant::Blue, &lit, d, None);
            let (marked, l) = render(Variant::Blue, &keyed, d, None);
            let under = |spot: Spot, pen: i32, base: i32| {
                let r = l.rect(spot).unwrap();
                let (x, y) = (
                    r.x - l.canvas.x + g.len(pen),
                    r.y - l.canvas.y + g.len(base) + d,
                );
                Rect::new(x, y, 6, d)
            };
            for (spot, pen, base, ink) in [
                (
                    Spot::Place(0),
                    at::PLACE_PEN,
                    at::PLACE_BASE,
                    s.start_menu.places_ink,
                ),
                (
                    Spot::Place(2),
                    at::PLACE_PEN,
                    at::PLACE_BASE,
                    s.system.hilight_text,
                ),
                (
                    Spot::Program(3),
                    at::PROGRAM_PEN,
                    at::PROGRAM_BASE,
                    s.system.menu_text,
                ),
            ] {
                let u = under(spot, pen, base);
                for y in u.y..u.bottom() {
                    for x in u.x..u.right() {
                        assert_eq!(marked.get(x, y), px(ink), "@{d} {spot:?} at ({x}, {y})");
                    }
                }
                assert_ne!(plain.get(u.x, u.y), px(ink), "@{d} {spot:?} bare");
                assert_ne!(
                    marked.get(u.right(), u.y),
                    px(ink),
                    "@{d} {spot:?} one letter"
                );
            }
            let moved = (plain.pixels.iter().zip(&marked.pixels)).filter(|(a, b)| a != b);
            let marks = 1 + 6 + 1 + 5 + 2 * 2;
            assert_eq!(moved.count(), (marks * 6 * d) as usize, "@{d}");
        }
    }

    #[test]
    fn keyboard_underlines_pin_in_blue_and_olive_at_scale_1_and_2() {
        let got: Vec<_> = [Variant::Blue, Variant::Olive]
            .into_iter()
            .flat_map(|v| {
                [1, 2].map(|d| {
                    let mut p = sample(Some(Spot::LogOff));
                    p.keys = true;
                    (format!("{v:?} {d}"), fnv(&render(v, &p, d, None).0))
                })
            })
            .collect();
        crate::pins::check("blue/start_panel_keys", &got);
    }
}
