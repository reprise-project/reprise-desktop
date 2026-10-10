//! The styled twin of each common widget in `look::widgets`: it paints the
//! widget through the style's parts and returns `false` when the style has
//! no part for it, so the forwarder falls back to Classic. One file per
//! widget.

pub mod balloon;
pub mod button;
pub mod captiontab;
pub mod checkbox;
pub mod combodrop;
pub mod explorerbar;
pub mod field;
pub mod groupbox;
pub mod header;
pub mod help;
pub mod item;
pub mod menu;
pub mod placesbar;
pub mod progress;
pub mod radio;
pub mod rebar;
pub mod scrollbar;
pub mod selbox;
pub mod spin;
pub mod statusbar;
pub mod tab;
pub mod taskpane;
pub mod toolbar;
pub mod tooltip;
pub mod trackbar;

use crate::pointer::Interaction;
use crate::style::State;

/// The frame a one-part widget draws: Disabled over whatever the pointer
/// does, then Pressed, Hot and Normal.
pub(crate) fn state(enabled: bool, ptr: Interaction) -> State {
    match (enabled, ptr) {
        (false, _) => State::Disabled,
        (true, Interaction::Pressed) => State::Pressed,
        (true, Interaction::Hot) => State::Hot,
        (true, Interaction::Idle) => State::Normal,
    }
}

/// The frame a field draws (an edit, an editable combo's field): Disabled
/// over everything, then Focused while it holds the keyboard focus, then
/// Hot while the pointer rests on it or holds it (a field has no held
/// look of its own), then Normal.
pub(crate) fn field_state(enabled: bool, focused: bool, ptr: Interaction) -> State {
    match (enabled, focused, ptr) {
        (false, _, _) => State::Disabled,
        (true, true, _) => State::Focused,
        (true, false, Interaction::Hot | Interaction::Pressed) => State::Hot,
        (true, false, Interaction::Idle) => State::Normal,
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, PartImage, Style, TextSpec};
    use crate::theme::{Color, ColorScheme, FontConfig, Metrics};

    pub fn blue_pixel(part: Part, state: crate::style::State, x: i32, y: i32) -> u32 {
        let st = crate::blue::style();
        let img = st.part(part).expect("a Blue part");
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    pub fn one_part_style(part: Part, img: PartImage) -> Style {
        let spec = TextSpec {
            family: FontConfig::default().family,
            size_px: 11,
            family_sizes: Vec::new(),
            bold: false,
            italic: false,
            antialias: false,
            full_hinting: false,
            color: Color::rgb(0, 0, 0),
            inactive: Color::rgb(0, 0, 0),
            shadow: None,
            ink_gamma: 1.0,
        };
        Style::new(
            "Test",
            ColorScheme::default(),
            Metrics::default(),
            spec.clone(),
            spec,
            Color::rgb(0, 0, 0),
            crate::taskbar::TaskbarShape::CLASSIC,
        )
        .with_part(part, img)
    }

    pub fn doubles(one: &Canvas, two: &Canvas) -> bool {
        let (w, h) = two.device_size();
        let bytes = two.to_argb_be();
        (0..h as i32).all(|y| {
            (0..w as i32).all(|x| {
                let i = 4 * (y as usize * w as usize + x as usize);
                u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) == one.get(x / 2, y / 2)
            })
        })
    }

    pub fn doubles_at_scale_1(one: &Canvas, two: &Canvas) -> bool {
        two.width == 2 * one.width
            && two.height == 2 * one.height
            && (0..two.height as i32)
                .all(|y| (0..two.width as i32).all(|x| two.get(x, y) == one.get(x / 2, y / 2)))
    }

    /// Whether `two`, a scale-1 canvas twice `one`'s size, is `one`'s design twice over as a part painted at scale 2 draws it: each pixel within a level either side of the 2x2 pixels of `one` nearest its middle, exactly theirs where `one` is flat. Pixels near one where `skip(x, y)` holds are left out. On a miss, the first pixel that broke.
    pub fn twice_over(
        one: &Canvas,
        two: &Canvas,
        skip: impl Fn(i32, i32) -> bool,
    ) -> Result<(), String> {
        let (w, h) = (one.width as i32, one.height as i32);
        if (two.width as i32, two.height as i32) != (2 * w, 2 * h) {
            return Err(format!("{}x{} is not twice {w}x{h}", two.width, two.height));
        }
        let near = |v: i32, len: i32| {
            let c = (f64::from(v) + 0.5) / 2.0 - 0.5;
            [c.floor() as i32, c.ceil() as i32].map(|n| n.clamp(0, len - 1))
        };
        for y in 0..2 * h {
            for x in 0..2 * w {
                let ([y0, y1], [x0, x1]) = (near(y, h), near(x, w));
                if [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                    .iter()
                    .any(|&(sx, sy)| skip(sx, sy))
                {
                    continue;
                }
                let got = two.get(x, y);
                let around =
                    [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(sx, sy)| one.get(sx, sy));
                let flat = ((y0 - 1).max(0)..=(y1 + 1).min(h - 1)).all(|sy| {
                    ((x0 - 1).max(0)..=(x1 + 1).min(w - 1)).all(|sx| one.get(sx, sy) == around[0])
                });
                if flat {
                    if got != around[0] {
                        return Err(format!("({x}, {y}): {got:08X}, not {:08X}", around[0]));
                    }
                    continue;
                }
                for sh in [24, 16, 8, 0] {
                    let ch = |p: u32| ((p >> sh) & 0xFF) as i32;
                    let lo = around.iter().map(|&p| ch(p)).min().unwrap_or(0);
                    let hi = around.iter().map(|&p| ch(p)).max().unwrap_or(0);
                    if !(lo - 1..=hi + 1).contains(&ch(got)) {
                        return Err(format!(
                            "({x}, {y}): {got:08X} channel {sh} outside {lo}..={hi} of {around:08X?}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether every frame of `one` (a part drawn into a `w` x `h` rect at scale 1) is `two` (painted at scale 2) drawn into that rect doubled, per [`twice_over`]; design pixels where `skip(state, nth, x, y)` holds are left out. On a miss, the frame that broke and where.
    pub fn part_twice_over(
        one: &crate::style::PartImage,
        two: &crate::style::PartImage,
        (w, h): (i32, i32),
        skip: impl Fn(crate::style::State, usize, i32, i32) -> bool,
    ) -> Result<(), String> {
        let part = Part::TaskButton;
        let (s1, s2) = (
            one_part_style(part, one.clone()),
            one_part_style(part, two.clone()),
        );
        let r = Rect::new(0, 0, w, h);
        for (i, &state) in one.states.iter().enumerate() {
            let nth = one.states[..i].iter().filter(|&&s| s == state).count();
            let mut a = Canvas::new(w as u32, h as u32);
            s1.draw_nth(&mut a, r, part, state, nth, 1);
            let mut b = Canvas::new(2 * w as u32, 2 * h as u32);
            s2.draw_nth(&mut b, r.scaled(2), part, state, nth, 2);
            twice_over(&a, &b, |x, y| skip(state, nth, x, y))
                .map_err(|e| format!("{state:?} #{nth} at {w}x{h}: {e}"))?;
        }
        Ok(())
    }

    /// Whether every frame of `st`'s `part` drawn at `s = 2` into a `w` x `h` rect `.scaled(2)` is its `s = 1` draw with each pixel a 2x2 block. On a miss, the frame that broke.
    pub fn part_doubles_at_scale_2(st: &Style, part: Part, w: i32, h: i32) -> Result<(), String> {
        let Some(img) = st.part(part) else {
            return Ok(());
        };
        let r = Rect::new(0, 0, w, h);
        for (i, &state) in img.states.iter().enumerate() {
            let nth = img.states[..i].iter().filter(|&&s| s == state).count();
            let check = img.checks.get(i).copied();
            let draw = |c: &mut Canvas, r: Rect, s: i32| match check {
                Some(k) => st.draw_check(c, r, part, k, state, s),
                None => st.draw_nth(c, r, part, state, nth, s),
            };
            let mut one = Canvas::new(w as u32, h as u32);
            draw(&mut one, r, 1);
            let mut two = Canvas::new(2 * w as u32, 2 * h as u32);
            draw(&mut two, r.scaled(2), 2);
            if !doubles_at_scale_1(&one, &two) {
                return Err(format!("{part:?} {state:?} #{nth} {check:?} at {w}x{h}"));
            }
        }
        Ok(())
    }

    /// [`doubles_at_paint_scale_2`] under Blue.
    pub fn blue_doubles_at_paint_scale_2(
        w: u32,
        h: u32,
        draw: impl Fn(&mut Canvas, &crate::look::Paint),
    ) -> bool {
        doubles_at_paint_scale_2(crate::interface::Interface::Blue, w, h, draw)
    }

    /// Whether `draw` with `look`'s paint at `s = 2` on a `2w` x `2h` scale-1 canvas is its `s = 1` draw with every pixel a 2x2 block, as the compositor makes a scale-2 draw. Panics when the `s = 1` draw leaves the face untouched.
    pub fn doubles_at_paint_scale_2(
        look: crate::interface::Interface,
        w: u32,
        h: u32,
        draw: impl Fn(&mut Canvas, &crate::look::Paint),
    ) -> bool {
        theme_doubles_at_paint_scale_2(crate::interfaces::theme_for(look), w, h, draw)
    }

    pub fn variant_doubles_at_paint_scale_2(
        v: crate::blue::Variant,
        w: u32,
        h: u32,
        draw: impl Fn(&mut Canvas, &crate::look::Paint),
    ) -> bool {
        theme_doubles_at_paint_scale_2(crate::interfaces::blue_theme(v), w, h, draw)
    }

    fn theme_doubles_at_paint_scale_2(
        mut theme: crate::theme::Theme,
        w: u32,
        h: u32,
        draw: impl Fn(&mut Canvas, &crate::look::Paint),
    ) -> bool {
        let face = theme.colors.button_face;
        let mut one = Canvas::new(w, h);
        one.fill_rect(Rect::new(0, 0, w as i32, h as i32), face);
        let blank = one.clone();
        draw(&mut one, &theme.paint());
        assert!(one != blank, "the s = 1 draw paints over the face");
        theme.scale = 2;
        let mut two = Canvas::new(2 * w, 2 * h);
        two.fill_rect(Rect::new(0, 0, 2 * w as i32, 2 * h as i32), face);
        draw(&mut two, &theme.paint());
        doubles_at_scale_1(&one, &two)
    }

    pub fn one_part_look(part: Part) -> crate::look::Look {
        let img = PartImage::new(
            solid_frames(&[Color::rgb(10, 0, 0)]),
            &[crate::style::State::Normal],
            crate::style::Layout::Vertical,
        );
        one_part_look_with(part, img)
    }

    pub fn one_part_look_with(part: Part, img: PartImage) -> crate::look::Look {
        crate::look::Look::Styled(std::sync::Arc::new(one_part_style(part, img)))
    }

    pub fn ramp(w: u32, h: u32) -> Canvas {
        let mut a = Canvas::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                a.put_raw(x, y, 0xFF00_0000 | ((x as u32 * 7) << 16) | (y as u32 * 3));
            }
        }
        a
    }

    pub fn solid_frames(inks: &[Color]) -> Canvas {
        let mut atlas = Canvas::new(4, 4 * inks.len() as u32);
        for (i, &ink) in inks.iter().enumerate() {
            atlas.fill_rect(Rect::new(0, 4 * i as i32, 4, 4), ink);
        }
        atlas
    }
}
