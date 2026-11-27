//! Reprise Glass's Onyx taskbar ([`super::Style::Onyx`]): near-black glass under a dark top line and a
//! light one, glossy over its top half, with no shade under the notification area, and labelled
//! task buttons framed by a light rim, an outline and a gloss of their own. The bar keeps its
//! colour whatever `[glass] color`, intensity, material and mode say; with transparency off it
//! lies over black.
//!
//! Parts are drawn for the bar's design height (30 px, or 40 without small icons); the nine-grid
//! keeps edge rows fixed on other heights. At `d` pixels per design pixel, lines and rings are
//! `d` px wide, corners are cut by coverage and gradients are evaluated at pixel centres
//! ([`centre`]).

use super::{BAR_HEIGHT, ICON_BUTTON, SMALL_ICON_BUTTON, SMALL_TASKBAR_HEIGHT, Settings};
use crate::raster::{Canvas, Insets};
use crate::shape::{CLEAR, Px, Stops, centre, over, pack, premul, ringed};
use crate::style::{Layout, Part, PartImage, State};
use crate::taskbar::LabelledButton;
use crate::theme::Color;

const TOP_LINE: (Color, u8) = (Color::rgb(0x54, 0x56, 0x59), 222);
const LIGHT_LINE: (Color, u8) = (Color::rgb(0xB7, 0xBA, 0xBC), 223);
/// The gloss from the row under the light line to the middle of the bar.
const GLOSS_TOP: (Color, u8) = (Color::rgb(0x90, 0x94, 0x98), 212);
const GLOSS_FOOT: (Color, u8) = (Color::rgb(0x46, 0x4C, 0x52), 194);
/// The body from the middle of the bar to the row over its foot line.
const BODY_TOP: (Color, u8) = (Color::rgb(0x08, 0x11, 0x18), 168);
const BODY_FOOT: (Color, u8) = (Color::rgb(0x12, 0x1A, 0x22), 187);
const FOOT_LINE: (Color, u8) = (Color::rgb(0x02, 0x03, 0x04), 246);

/// The frame's corner radius at its outer rim, 1 px tighter per ring inward.
const BUTTON_RADIUS: f64 = 4.0;
/// A labelled button: 158 px wide, 2 px apart, its 16 px icon 12 px in and its label at 34.
const LABELLED: LabelledButton = LabelledButton {
    gap: 2,
    icon_x: 12,
    icon_box: 16,
    text_x: 34,
};
const LABELLED_WIDTH: i32 = 158;

/// Rows of the bar above and below a button frame.
const BUTTON_INSET: i32 = 1;
const BUTTON_SIZING: Insets = Insets::new(5, 4, 5, 4);

/// White shares: the rim round every frame, the light ring inside a resting frame's outline
/// (its top row brighter than its sides and foot) and the gloss over its top half.
const RIM: f64 = 0.28;
const RIM_HELD: f64 = 0.30;
const INNER_TOP: f64 = 0.48;
const INNER_SIDE: f64 = 0.15;
const REST_GLOSS: (f64, f64) = (0.34, 0.10);

/// Black shares: a resting frame's outline, a held one's, and a held frame's inner top row and
/// fill, its top half lighter than its foot.
const OUTLINE: f64 = 0.37;
const OUTLINE_HELD: f64 = 1.0;
const HELD_INNER_TOP: f64 = 0.66;
const HELD_FILL: (f64, f64, f64) = (0.50, 0.43, 0.58);

/// The light rising from a hot frame's foot: clear above [`HOT_FROM`] of the frame's height,
/// [`HOT_PEAK`] at its inner foot, and a cyan line along the inner ring's foot.
const HOT_GLOW: Color = Color::rgb(0x3C, 0xA8, 0xEC);
const HOT_FROM: f64 = 0.3;
const HOT_PEAK: f64 = 0.62;
const HOT_LINE: Color = Color::rgb(0x4F, 0xD9, 0xE2);
const HOT_LINE_SHARE: f64 = 0.8;

/// A flashing frame: amber over its whole fill, denser at the foot.
const FLASH: Color = Color::rgb(0xFF, 0xA2, 0x2A);
const FLASH_SHARE: (f64, f64) = (0.35, 0.75);

fn straight((c, a): (Color, u8)) -> Px {
    premul(c, a)
}

fn white(share: f64) -> Px {
    premul(Color::rgb(0xFF, 0xFF, 0xFF), share_alpha(share))
}

fn black(share: f64) -> Px {
    premul(Color::rgb(0, 0, 0), share_alpha(share))
}

fn share_alpha(share: f64) -> u8 {
    (share.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The bar's glass at row `y` ([`centre`]) of an `h` px bar.
fn bar_row(y: f64, h: i32) -> Px {
    let (mid, foot) = (f64::from(h / 2), f64::from(h - 1));
    if y < 0.5 {
        straight(TOP_LINE)
    } else if y < 1.5 {
        straight(LIGHT_LINE)
    } else if y < mid - 0.5 {
        let t = (y - 2.0) / (mid - 3.0);
        Stops(&[(0.0, straight(GLOSS_TOP)), (1.0, straight(GLOSS_FOOT))]).at(t)
    } else if y < foot - 0.5 {
        let t = (y - mid) / (foot - 1.0 - mid);
        Stops(&[(0.0, straight(BODY_TOP)), (1.0, straight(BODY_FOOT))]).at(t)
    } else {
        straight(FOOT_LINE)
    }
}

/// The bar, `h` px tall and 1 px wide, stretched across; opaque over black with transparency off.
pub fn taskbar(s: &Settings, h: i32, d: i32) -> PartImage {
    let mut c = Canvas::new(d as u32, (h * d) as u32);
    for y in 0..h * d {
        let glass = bar_row(centre(y, d), h);
        let px = match s.transparency {
            true => glass,
            false => over(glass, (255.0, [0.0; 3])),
        };
        for x in 0..d {
            c.put_raw(x, y, pack(px));
        }
    }
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(0, 2, 0, 1))
        .at_scale(d)
}

/// The notification area: no shade of its own.
pub fn tray_area(h: i32, d: i32) -> PartImage {
    PartImage::new(
        Canvas::new(d as u32, (h * d) as u32),
        &[State::Normal],
        Layout::Vertical,
    )
    .at_scale(d)
}

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Rest,
    Held,
    Flash,
}

const BUTTON_STATES: [State; 6] = [
    State::Normal,
    State::Hot,
    State::Checked,
    State::CheckedHot,
    State::Pressed,
    State::Attention,
];

/// The face `state` takes and whether it is lit from the foot. Active and held frames share the
/// held face.
fn look_of(state: State) -> (Face, bool) {
    match state {
        State::Hot => (Face::Rest, true),
        State::Checked | State::Pressed => (Face::Held, false),
        State::CheckedHot => (Face::Held, true),
        State::Attention => (Face::Flash, false),
        _ => (Face::Rest, false),
    }
}

/// Ring `ring` (0 the rim, 1 the outline, 2 the inner ring, 3 the fill) of a `bh` px tall frame
/// at row `y` inside it, in `face`, lit or not.
fn frame_px(face: Face, lit: bool, ring: usize, y: f64, bh: i32) -> Px {
    let half = f64::from(bh) / 2.0;
    let top_row = y < 2.5;
    let px = match (face, ring) {
        (Face::Held, 0) => white(RIM_HELD),
        (_, 0) => white(RIM),
        (Face::Held, 1) => black(OUTLINE_HELD),
        (_, 1) => black(OUTLINE),
        (Face::Held, 2) if top_row => black(HELD_INNER_TOP),
        (Face::Rest, 2) if top_row => white(INNER_TOP),
        (Face::Rest, 2) => white(INNER_SIDE),
        (Face::Held, _) => {
            let (top, mid, foot) = HELD_FILL;
            match y < half {
                true => black(top + (mid - top) * ((y - 3.0) / (half - 3.0)).clamp(0.0, 1.0)),
                false => black(foot),
            }
        }
        (Face::Flash, 2) => white(INNER_SIDE),
        (Face::Flash, _) => {
            let (top, foot) = FLASH_SHARE;
            let t = ((y - 2.0) / (f64::from(bh) - 5.0)).clamp(0.0, 1.0);
            premul(FLASH, share_alpha(top + (foot - top) * t))
        }
        (Face::Rest, _) if y < half => {
            let (top, mid) = REST_GLOSS;
            white(top + (mid - top) * ((y - 3.0) / (half - 3.0)).clamp(0.0, 1.0))
        }
        (Face::Rest, _) => CLEAR,
    };
    if !lit || ring < 2 {
        return px;
    }
    let from = f64::from(bh) * HOT_FROM;
    let t = ((y - from) / (f64::from(bh) - 3.0 - from)).clamp(0.0, 1.0);
    let glow = over(premul(HOT_GLOW, share_alpha(HOT_PEAK * t)), px);
    match ring == 2 && y > f64::from(bh) - 3.5 {
        true => over(premul(HOT_LINE, share_alpha(HOT_LINE_SHARE)), glow),
        false => glow,
    }
}

/// Task buttons `size` in each of [`BUTTON_STATES`], as a nine-grid: a frame 1 px inside the
/// bar's top and foot, its rim, outline and inner ring round a fill.
pub fn buttons((w, h): (i32, i32), d: i32) -> PartImage {
    let bh = h - 2 * BUTTON_INSET;
    let count = BUTTON_STATES.len() as i32;
    let mut c = Canvas::new((w * d) as u32, (h * d * count) as u32);
    for (i, state) in BUTTON_STATES.into_iter().enumerate() {
        let (face, lit) = look_of(state);
        let px = ringed(w, bh, BUTTON_RADIUS, 3, d, |_, y, ring| {
            frame_px(face, lit, ring, y, bh)
        });
        let top = (i as i32 * h + BUTTON_INSET) * d;
        for (k, p) in px.into_iter().enumerate() {
            let k = k as i32;
            c.put_raw(k % (w * d), top + k / (w * d), pack(p));
        }
    }
    PartImage::new(c, &BUTTON_STATES, Layout::Vertical)
        .sizing(BUTTON_SIZING)
        .at_scale(d)
}

/// `st` with Onyx's bar, notification area and task buttons for a bar `small_bar` or not, drawn
/// at `d`. Labelled buttons draw the task button frame; nothing glows in an icon's colour.
pub(super) fn restyle(
    st: crate::style::Style,
    s: &Settings,
    small_bar: bool,
    d: i32,
) -> crate::style::Style {
    let (h, w) = match small_bar {
        true => (SMALL_TASKBAR_HEIGHT, SMALL_ICON_BUTTON.0),
        false => (BAR_HEIGHT, ICON_BUTTON.0),
    };
    let s = *s;
    let mut st = st
        .with_painted(Part::Taskbar, move || taskbar(&s, h, d))
        .with_painted(Part::TrayArea, move || tray_area(h, d))
        .with_painted(Part::TaskButton, move || buttons((w, h), d))
        .without_part(Part::LabelRow)
        .without_part(Part::TaskLabel)
        .without_part(Part::TaskLabelEdge)
        .with_task_glow(None)
        .with_task_spot(None);
    st.taskbar.labelled = LABELLED;
    st.metrics.task_button_max_width = LABELLED_WIDTH;
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Material, Mode, SWATCHES, Style};

    fn onyx() -> Settings {
        Settings {
            style: Style::Onyx,
            ..Settings::default()
        }
    }

    fn state_canvas(img: &PartImage, state: State) -> Canvas {
        let f = img.frame(state);
        let mut c = Canvas::new(f.width as u32, f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                c.put_raw(x, y, img.atlas.get(f.x + x, f.y + y));
            }
        }
        c
    }

    #[test]
    fn the_onyx_bar_and_buttons_keep_their_pixels() {
        let mut got = Vec::new();
        for d in [1, 2] {
            for (h, w) in [(30, 44), (40, 60)] {
                for transparency in [true, false] {
                    let s = Settings {
                        transparency,
                        ..onyx()
                    };
                    got.push((
                        format!("{h} px bar, transparency {transparency}, at {d}"),
                        crate::pins::fnv(&taskbar(&s, h, d).atlas),
                    ));
                }
                let b = buttons((w, h), d);
                for state in BUTTON_STATES {
                    got.push((
                        format!("{h} px button {state:?} at {d}"),
                        crate::pins::fnv(&state_canvas(&b, state)),
                    ));
                }
            }
        }
        crate::pins::check("glass/onyx", &got);
    }

    #[test]
    fn the_bar_is_near_black_glass_whatever_the_glass_settings() {
        let bar = taskbar(&onyx(), 30, 1);
        let (a, p) = crate::shape::unpack(bar.atlas.get(0, 20));
        assert!(
            (160.0..=200.0).contains(&a),
            "the body lets about a third through, alpha {a}"
        );
        assert!(
            p.iter().all(|&c| c < 40.0),
            "the body is near black, got {p:?}"
        );
        let (top, gloss) = crate::shape::unpack(bar.atlas.get(0, 5));
        assert!(
            top >= a && gloss[0] > p[0],
            "the top half is a lighter gloss"
        );
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        let other = Settings {
            color: ruby,
            intensity: 90,
            material: Material::Smoked,
            mode: Mode::Dark,
            ..onyx()
        };
        assert_eq!(taskbar(&other, 30, 1).atlas.pixels, bar.atlas.pixels);
        let opaque = taskbar(
            &Settings {
                transparency: false,
                ..onyx()
            },
            30,
            2,
        );
        assert!(opaque.atlas.pixels.iter().all(|px| px >> 24 == 0xFF));
    }

    #[test]
    fn button_faces_differ_by_state_and_held_shares_the_active_face() {
        let b = buttons((44, 30), 1);
        let faces: Vec<Canvas> = BUTTON_STATES.map(|s| state_canvas(&b, s)).to_vec();
        for (i, a) in faces.iter().enumerate() {
            for (j, c) in faces.iter().enumerate().skip(i + 1) {
                let same = a.pixels == c.pixels;
                let held = [BUTTON_STATES[i], BUTTON_STATES[j]] == [State::Checked, State::Pressed];
                assert_eq!(
                    same, held,
                    "{:?} and {:?} differ unless they are active and held",
                    BUTTON_STATES[i], BUTTON_STATES[j]
                );
            }
        }
    }

    #[test]
    fn button_frames_leave_the_bar_rows_and_corners_clear() {
        let b = buttons((44, 30), 2);
        for state in BUTTON_STATES {
            let c = state_canvas(&b, state);
            let (w, h) = (c.width as i32, c.height as i32);
            for x in 0..w {
                assert_eq!(
                    c.get(x, 0) >> 24,
                    0,
                    "{state:?}: the bar's top row is clear"
                );
                assert_eq!(c.get(x, h - 1) >> 24, 0, "{state:?}: the foot row is clear");
            }
            for (x, y) in [(0, 2), (w - 1, 2), (0, h - 3), (w - 1, h - 3)] {
                assert_eq!(
                    c.get(x, y) >> 24,
                    0,
                    "{state:?}: corner ({x}, {y}) is clear"
                );
            }
            let curved = (0..8).flat_map(|y| (0..8).map(move |x| (x, y + 2)));
            assert!(
                curved
                    .map(|(x, y)| c.get(x, y) >> 24)
                    .any(|a| a > 0 && a < 0xFF),
                "{state:?}: the curved corner has partly covered pixels at scale 2"
            );
        }
    }

    #[test]
    fn button_frames_double_at_scale_2() {
        let (one, two) = (buttons((44, 30), 1), buttons((44, 30), 2));
        assert_eq!(
            one.sizing, two.sizing,
            "the nine-grid insets are design pixels"
        );
        for state in BUTTON_STATES {
            let (a, b) = (one.frame(state), two.frame(state));
            assert_eq!(
                (b.width, b.height),
                (2 * a.width, 2 * a.height),
                "{state:?}"
            );
        }
    }

    #[test]
    fn onyx_takes_its_bar_white_labels_and_framed_labelled_buttons() {
        let st = crate::glass::small_bar_style(&onyx());
        assert!(
            st.part(Part::TaskLabel).is_none(),
            "labelled buttons draw the task button"
        );
        assert!(st.task_glow.is_none() && st.task_spot().is_none());
        assert_eq!(st.taskbar.labelled, LABELLED);
        assert_eq!(crate::glass::bar_ink(&onyx()), Color::rgb(0xFF, 0xFF, 0xFF));
        let opaque_light = Settings {
            transparency: false,
            ..onyx()
        };
        assert_eq!(
            crate::glass::bar_ink(&opaque_light),
            Color::rgb(0xFF, 0xFF, 0xFF)
        );
        let glass = crate::glass::small_bar_style(&Settings::default());
        assert!(glass.task_glow.is_some(), "Glass keeps its glowing buttons");
        let attention = st.part(Part::TaskButton).unwrap();
        assert!(attention.states.contains(&State::Attention));
    }
}
