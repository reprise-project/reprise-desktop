//! Every part of the Blue and Glass styles in every frame, at the scale
//! `REPRISE_GALLERY_SCALE` asks for: one page per Blue scheme
//! (`target/reprisekit-gallery-parts-blue-<scheme>.png`) and one per Glass
//! setting (`target/reprisekit-gallery-parts-glass-<sky|dark|smoked>.png`).
//! Each part is a row of frames labelled by state, at their own size (at most
//! [`MAX_W`] x [`MAX_H`] design px).

use reprisekit::blue::Variant;
use reprisekit::glass::{Material, Mode, Settings};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect, write_preview};
use reprisekit::style::{Check, Part, State, Style};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig};

const PAGE_W: i32 = 1180;
const LABEL_W: i32 = 150;
const MAX_W: i32 = 280;
const MAX_H: i32 = 120;
const STATE_H: i32 = 14;
const GAP: i32 = 8;
const TEXT_PX: u32 = 11;
const LABEL_MIN: i32 = 100;

pub fn write_all(n: i32) {
    for v in Variant::ALL {
        let st = reprisekit::blue::style_for(v).at_scale(n);
        let back = interfaces::blue_theme(v).colors.button_face;
        write_preview(
            &page(&st, back, n),
            &format!("reprisekit-gallery-parts-blue-{}.png", v.as_str()),
        );
    }
    let glass = [
        ("sky", Settings::default()),
        (
            "dark",
            Settings {
                mode: Mode::Dark,
                ..Settings::default()
            },
        ),
        (
            "smoked",
            Settings {
                material: Material::Smoked,
                ..Settings::default()
            },
        ),
    ];
    for (name, s) in glass {
        let theme = interfaces::theme_with(Interface::Glass, Variant::default(), &s).scaled(n);
        let st = theme.look.style().expect("Glass is styled");
        write_preview(
            &page(st, Color::rgb(0x3A, 0x6E, 0xA5), n),
            &format!("reprisekit-gallery-parts-glass-{name}.png"),
        );
    }
}

struct Frame {
    label: String,
    size: (i32, i32),
    part: Part,
    state: State,
    nth: usize,
    check: Option<Check>,
}

impl Frame {
    fn draw(&self, st: &Style, c: &mut Canvas, r: Rect, n: i32) {
        match self.check {
            Some(k) => st.draw_check(c, r, self.part, k, self.state, n),
            None => st.draw_nth(c, r, self.part, self.state, self.nth, n),
        }
    }
}

fn frames(st: &Style, part: Part) -> Vec<Frame> {
    let img = st.part(part).expect("a part the style holds");
    img.states
        .iter()
        .enumerate()
        .map(|(i, &state)| {
            let nth = img.states[..i].iter().filter(|&&s| s == state).count();
            let check = img.checks.get(i).copied();
            let atlas = match check {
                Some(k) => img.check_frame(k, state),
                None => img.frame_nth(state, nth),
            };
            let size = (
                (atlas.width / img.scale).min(MAX_W),
                (atlas.height / img.scale).min(MAX_H),
            );
            let label = match check {
                Some(k) => format!("{k:?} {state:?}"),
                None if nth > 0 => format!("{state:?} #{nth}"),
                None => format!("{state:?}"),
            };
            Frame {
                label,
                size,
                part,
                state,
                nth,
                check,
            }
        })
        .collect()
}

fn page(st: &Style, back: Color, n: i32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut parts: Vec<Part> = st.part_keys().collect();
    parts.sort_by_key(|p| format!("{p:?}"));
    let mut placed: Vec<(String, Rect, Option<Frame>)> = Vec::new();
    let mut y = GAP;
    for part in parts {
        placed.push((
            format!("{part:?}"),
            Rect::new(GAP, y, LABEL_W, STATE_H),
            None,
        ));
        let (mut x, mut row_h) = (LABEL_W + GAP, 0);
        for f in frames(st, part) {
            let (w, h) = f.size;
            if x + w > PAGE_W && x > LABEL_W + GAP {
                (x, y) = (LABEL_W + GAP, y + row_h + GAP);
                row_h = 0;
            }
            placed.push((
                f.label.clone(),
                Rect::new(x, y, w.max(1), h.max(1)),
                Some(f),
            ));
            x += w.max(LABEL_MIN) + GAP;
            row_h = row_h.max(STATE_H + h);
        }
        y += row_h.max(STATE_H) + 2 * GAP;
    }
    let mut c = Canvas::new((PAGE_W * n) as u32, (y * n) as u32);
    c.fill_rect(c.bounds(), back);
    let ink = Color::rgb(0, 0, 0);
    for (label, r, f) in placed {
        let s = |v: i32| v * n;
        match f {
            None => text.draw(
                &mut c,
                Rect::new(s(r.x), s(r.y), s(r.width), s(STATE_H)),
                &label,
                true,
                ink,
                Align::Left,
                TEXT_PX * n as u32,
            ),
            Some(f) => {
                text.draw(
                    &mut c,
                    Rect::new(s(r.x), s(r.y), s(r.width.max(LABEL_MIN)), s(STATE_H)),
                    &label,
                    false,
                    ink,
                    Align::Left,
                    TEXT_PX * n as u32,
                );
                let to = Rect::new(s(r.x), s(r.y + STATE_H), s(r.width), s(r.height));
                f.draw(st, &mut c, to, n);
            }
        }
    }
    c
}
