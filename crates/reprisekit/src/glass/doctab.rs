//! Glass's document tab over the frame's glass ([`Part::GlassTab`]), drawn from shapes at the
//! style's scale: washes the glass shows through when closed or hot, a near-solid face when open.

use crate::raster::Insets;
use crate::shape::{Px, Stops, centre, draw, premul, rounded4, shares};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const LEFT: i32 = 5;
const RIGHT: i32 = 5;
const W: i32 = LEFT + 1 + RIGHT;
const H: i32 = 18;
const TOP: i32 = 5;
const RADIUS: f64 = 4.0;

const STATES: [State; 3] = [State::Normal, State::Hot, State::Checked];

const RIM: Color = Color::rgb(0x1C, 0x2A, 0x3C);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const PALE_BLUE: Color = Color::rgb(0xE6, 0xF2, 0xFD);
const FACE_TOP: Color = Color::rgb(0xF6, 0xF9, 0xFC);
const FACE_BOTTOM: Color = Color::rgb(0xDE, 0xE8, 0xF2);

/// One state's line round the tab, the line inside it and its body's top and bottom.
struct Inks {
    rim: Px,
    inner: Px,
    top: Px,
    bottom: Px,
}

fn inks(state: State) -> Inks {
    match state {
        State::Hot => Inks {
            rim: premul(RIM, 0xA8),
            inner: premul(WHITE, 0xD0),
            top: premul(PALE_BLUE, 0xC0),
            bottom: premul(PALE_BLUE, 0x78),
        },
        State::Checked => Inks {
            rim: premul(RIM, 0xB8),
            inner: premul(WHITE, 0xFF),
            top: premul(FACE_TOP, 0xFF),
            bottom: premul(FACE_BOTTOM, 0xFF),
        },
        _ => Inks {
            rim: premul(RIM, 0x90),
            inner: premul(WHITE, 0x90),
            top: premul(WHITE, 0x70),
            bottom: premul(WHITE, 0x28),
        },
    }
}

pub fn parts(d: i32) -> [(Part, PartImage); 1] {
    [(Part::GlassTab, tab(d))]
}

fn tab(d: i32) -> PartImage {
    let k = f64::from(d);
    let n = STATES.len() as i32;
    let fh = H * d;
    // The outline runs past each frame's bottom: a tab has no bottom edge.
    let (w, tall) = (f64::from(W) * k, f64::from(H) * k + RADIUS * k);
    let outer_r = [RADIUS * k, RADIUS * k, 0.0, 0.0];
    let inner_r = [(RADIUS - 1.0) * k, (RADIUS - 1.0) * k, 0.0, 0.0];
    let atlas = draw(W, H * n, d, |x, y| {
        let i = (y as i32 / fh) as usize;
        let y = y - f64::from(i as i32 * fh);
        let ink = inks(STATES[i]);
        let outer = rounded4(x, y, w, tall, outer_r);
        let inner = rounded4(x - k, y - k, w - 2.0 * k, tall, inner_r);
        let body = rounded4(x - 2.0 * k, y - 2.0 * k, w - 4.0 * k, tall, [0.0; 4]);
        let t = (centre(y as i32, d) - 2.0) / f64::from(H - 3);
        let fill = Stops(&[(0.0, ink.top), (1.0, ink.bottom)]).at(t);
        shares(&[
            (ink.rim, outer - inner),
            (ink.inner, inner - body),
            (fill, body),
        ])
    });
    PartImage::new(atlas, &STATES, Layout::Vertical)
        .sizing(Insets::new(LEFT, TOP, RIGHT, 0))
        .content(Insets::new(1, 2, 0, 0))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Mode, Settings, style};
    use crate::raster::{Canvas, Rect};

    fn frame(mode: Mode, state: State, d: i32) -> Canvas {
        let st = style(&Settings {
            mode,
            ..Settings::default()
        })
        .at_scale(d);
        let img = st.part(Part::GlassTab).expect("the part is built");
        let r = img.frame(state);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        for y in 0..r.height {
            for x in 0..r.width {
                c.put_raw(x, y, img.atlas.get(r.x + x, r.y + y));
            }
        }
        c
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn glass_tab_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for d in [1, 2] {
                for state in STATES {
                    let h = fnv(&frame(mode, state, d));
                    got.push((format!("{mode:?} {state:?} {d}"), h));
                }
            }
        }
        crate::pins::check("glass/doctab", &got);
    }

    #[test]
    fn a_glass_tabs_top_corners_are_clear_and_its_sides_solid() {
        for d in [1, 2] {
            for state in STATES {
                let c = frame(Mode::Light, state, d);
                let right = W * d - 1;
                assert!(c.get(0, 0) >> 24 < 8, "{state:?} at {d}: top-left");
                assert!(c.get(right, 0) >> 24 < 8, "{state:?} at {d}: top-right");
                assert!(
                    c.get(0, 10 * d) >> 24 > 0x80,
                    "{state:?} at {d}: the left line"
                );
                assert!(
                    c.get(right, 10 * d) >> 24 > 0x80,
                    "{state:?} at {d}: the right line"
                );
            }
        }
    }

    #[test]
    fn a_glass_tabs_states_differ_and_only_the_open_one_is_solid() {
        for d in [1, 2] {
            let [n, h, o] = STATES.map(|s| frame(Mode::Light, s, d));
            assert!(n != h && h != o && n != o, "at {d}");
            let mid = (5 * d, 10 * d);
            assert_eq!(o.get(mid.0, mid.1) >> 24, 0xFF, "the open tab is solid");
            for (c, what) in [(&n, "closed"), (&h, "hot")] {
                let a = c.get(mid.0, mid.1) >> 24;
                assert!(
                    a > 0 && a < 0xFF,
                    "a {what} tab lets the glass through: {a:02x}"
                );
            }
            assert!(
                h.get(mid.0, mid.1) >> 24 > n.get(mid.0, mid.1) >> 24,
                "hot is a stronger wash than closed"
            );
        }
    }

    #[test]
    fn a_glass_tabs_curved_top_is_smooth_at_scale_2() {
        for state in STATES {
            let c = frame(Mode::Light, state, 2);
            let side = c.get(0, 20) >> 24;
            let mut fading: Vec<u32> = (0..8)
                .map(|y| c.get(0, y) >> 24)
                .filter(|&a| a > 0 && a < side)
                .collect();
            fading.dedup();
            assert!(
                fading.len() >= 2,
                "{state:?}: the curve fades in over {fading:?}"
            );
        }
    }

    #[test]
    fn a_glass_tab_at_scale_2_doubles_with_the_same_grid() {
        let one = style(&Settings::default());
        let two = one.at_scale(2);
        let (a, b) = (
            one.part(Part::GlassTab).unwrap(),
            two.part(Part::GlassTab).unwrap(),
        );
        assert_eq!(b.scale, 2);
        assert_eq!(
            (b.atlas.width, b.atlas.height),
            (2 * a.atlas.width, 2 * a.atlas.height)
        );
        assert_eq!(a.sizing, b.sizing, "design insets");
        assert_eq!(a.content, b.content, "design content");
        let mut c = Canvas::new(60, 20);
        one.draw(
            &mut c,
            Rect::new(0, 0, 60, 18),
            Part::GlassTab,
            State::Normal,
            1,
        );
        assert!(
            c.get(59, 9) >> 24 > 0x80,
            "a wide tab's right line lands at its edge"
        );
    }
}
