use crate::frame::{ButtonSet, CaptionButton, FrameGeometry, FrameState, Glow};
use crate::icon;
use crate::raster::{Canvas, Rect};
use crate::style::{GlowMask, Part, State, Style, TitleGlow};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Theme};

fn button_part(kind: CaptionButton, maximized: bool) -> Part {
    match kind {
        CaptionButton::Minimize => Part::MinButton,
        CaptionButton::Maximize if maximized => Part::RestoreButton,
        CaptionButton::Maximize => Part::MaxButton,
        CaptionButton::Close => Part::CloseButton,
    }
}

pub fn draw_frame(
    canvas: &mut Canvas,
    style: &Style,
    theme: &Theme,
    caption_text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
) {
    let o = geom.origin;
    let (w, h) = geom.total;
    let s = theme.scale.max(1);
    canvas.clear_rect(Rect::new(o.0, o.1, w, h));

    let band_state = if state.active {
        State::Normal
    } else {
        State::Inactive
    };
    let caption_part = if state.maximized {
        Part::MaxCaption
    } else {
        Part::Caption
    };
    let top = Rect::new(o.0, o.1, w, geom.client.y - o.1);
    style.draw(canvas, top, caption_part, band_state, s);

    let f = geom.frame;
    if f > 0 {
        style.draw(
            canvas,
            Rect::new(o.0, geom.client.y, f, geom.client.height),
            Part::FrameLeft,
            band_state,
            s,
        );
        style.draw(
            canvas,
            Rect::new(o.0 + w - f, geom.client.y, f, geom.client.height),
            Part::FrameRight,
            band_state,
            s,
        );
        style.draw(
            canvas,
            Rect::new(o.0, o.1 + h - f, w, f),
            Part::FrameBottom,
            band_state,
            s,
        );
    }
    if geom.glass_top > 0 {
        let c = geom.client;
        let band = Rect::new(c.x, c.y, c.width, geom.glass_top);
        draw_client_glass(
            canvas,
            style,
            band,
            c.height,
            state.maximized,
            band_state,
            s,
        );
    }

    if !state.tabbed {
        draw_title(canvas, style, theme, caption_text, geom, state);
    }
    let side = crate::frame::button_side(&theme.metrics) / s;
    let grown = style.at_button_side(side);
    draw_buttons(canvas, grown.as_deref().unwrap_or(style), geom, state, s);

    canvas.clear_rect(geom.client);
}

/// Replace `band`, a client's top rows, and the client edges its outset covers with `style`'s
/// [`Part::ClientGlass`] (or [`Part::MaxClientGlass`]), its top rows on the band's top however
/// deep the band; a restored window's sides turn into its side borders down a client
/// `client_h` tall. Nothing in a style without it.
pub(crate) fn draw_client_glass(
    canvas: &mut Canvas,
    style: &Style,
    band: Rect,
    client_h: i32,
    maximized: bool,
    state: State,
    s: i32,
) {
    let part = match maximized {
        true => Part::MaxClientGlass,
        false => Part::ClientGlass,
    };
    let Some(p) = style.part(part) else {
        return;
    };
    let o = p.outset.scaled(s);
    let shown = Rect::new(
        band.x - o.left,
        band.y - o.top,
        band.width + o.left + o.right,
        band.height + o.top,
    );
    canvas.clear_rect(shown);
    let rows = p.frame(state).height / p.scale * s;
    let whole = Rect::new(band.x, band.y, band.width, band.height.max(rows));
    canvas.with_clip(shown, |c| {
        style.draw(c, whole, part, state, s);
        if let Some(edges) = style.client_glass_edges.as_ref().filter(|_| !maximized) {
            edges(c, band, client_h, state, s);
        }
    });
}

/// Where a title may run in `style`'s caption: from its left edge to the first caption button.
pub(crate) fn title_span(style: &Style, geom: &FrameGeometry, s: i32) -> (i32, i32) {
    (
        geom.caption.x + style.caption_title.text_x * s,
        geom.buttons[0].x,
    )
}

/// How far a fixed caption baseline moves at the text size: as far as a line centred in the
/// caption moves when both the caption and the text grow.
fn grown_baseline_shift(
    style: &Style,
    theme: &Theme,
    caption_text: &TextRenderer,
    geom: &FrameGeometry,
    s: i32,
) -> i32 {
    let spec = &style.caption_text;
    let base = theme.text.base_px(spec.size_px);
    if base == spec.size_px {
        return 0;
    }
    let centred =
        |h: i32, px: u32| caption_text.baseline(Rect::new(0, 0, 0, h), spec.bold, px * s as u32);
    let grown = centred(geom.caption.height, spec.size_px);
    let at_100 = centred(style.metrics.caption_height * s, base);
    grown.zip(at_100).map_or(0, |(g, b)| g - b)
}

fn draw_title(
    canvas: &mut Canvas,
    style: &Style,
    theme: &Theme,
    caption_text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
) {
    let s = theme.scale.max(1);
    let t = &style.caption_title;
    let m = usize::from(state.maximized);
    let translucent = style.glass.is_some();
    let spec = &style.caption_text;
    let (mut title_x, first_button_x) = title_span(style, geom, s);
    let size = spec.size_px * s as u32;
    if style.centred_title {
        let tw = caption_text.measure(state.title, spec.bold, size);
        let centred = geom.origin.0 + geom.total.0 / 2 - tw / 2;
        title_x = centred.min(first_button_x - tw).max(title_x);
    }
    let title_rect = Rect::new(
        title_x,
        geom.caption.y,
        (first_button_x - title_x).max(0),
        geom.caption.height,
    );
    let ink = if state.active {
        spec.color
    } else {
        spec.inactive
    };
    let title = if translucent {
        let baseline = match t.baseline {
            Some(b) => Some(
                geom.caption.y
                    + b[m] * s
                    + grown_baseline_shift(style, theme, caption_text, geom, s),
            ),
            None => caption_text.baseline(title_rect, spec.bold, size),
        };
        baseline.and_then(|baseline| {
            let run = TitleRun {
                text: caption_text,
                title: state.title,
                bold: spec.bold,
                size,
                clip: title_rect,
                baseline,
            };
            TitleMask::render(canvas, &run, geom, t.glow.as_ref(), s, m)
        })
    } else {
        None
    };
    if let (Some(title), Some(glow)) = (&title, t.glow) {
        title.glow(canvas, &glow, state.active);
    }

    let icon_px = theme.icon_size();
    let (ix, iy) = t.icon[m];
    let grown = if theme.text.is_one() {
        0
    } else {
        geom.caption.height - style.metrics.caption_height * s
    };
    let (icon_x, icon_y) = (geom.caption.x + ix * s, geom.caption.y + iy * s + grown / 2);
    let fallback;
    let im = match state.icon {
        Some(im) => im,
        None => {
            fallback = icon::chrome_fallback(theme, icon_px);
            &fallback
        }
    };
    if translucent {
        canvas.blit_over(icon_x, icon_y, im);
    } else {
        canvas.blit_canvas(icon_x, icon_y, im);
    }

    if translucent {
        if let Some(title) = title {
            title.ink(canvas, ink);
        }
        return;
    }
    if let Some(sh) = spec.shadow.as_ref().filter(|_| state.active) {
        let shadow_rect = Rect::new(
            title_rect.x + sh.dx * s,
            title_rect.y + sh.dy * s,
            title_rect.width,
            title_rect.height,
        );
        caption_text.draw_clipped(
            canvas,
            shadow_rect,
            title_rect,
            state.title,
            spec.bold,
            sh.color,
            Align::Left,
            size,
        );
    }
    caption_text.draw(
        canvas,
        title_rect,
        state.title,
        spec.bold,
        ink,
        Align::Left,
        size,
    );
}

struct TitleRun<'a> {
    text: &'a TextRenderer,
    title: &'a str,
    bold: bool,
    size: u32,
    clip: Rect,
    baseline: i32,
}

struct TitleMask {
    coverage: Vec<u8>,
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    rows: (usize, usize),
    scale: i32,
    fade_end: i32,
    fade: i32,
}

impl TitleMask {
    fn render(
        canvas: &Canvas,
        run: &TitleRun,
        geom: &FrameGeometry,
        glow: Option<&TitleGlow>,
        s: i32,
        m: usize,
    ) -> Option<TitleMask> {
        let cs = canvas.scale();
        let reach = glow.map_or(2 * s, |g| ((g.reach)(s * cs) + cs - 1) / cs);
        let shown = run
            .text
            .measure(run.title, run.bold, run.size)
            .min(run.clip.width);
        let (top, bottom) = (geom.origin.1, geom.client.y);
        let r = Rect::new(run.clip.x - reach, top, shown + 2 * reach, bottom - top);
        if shown <= 0 || r.height <= 0 || r.intersect(&canvas.bounds()).is_empty() {
            return None;
        }
        let mut scratch = Canvas::with_scale(r.width as u32, r.height as u32, cs);
        let clip = Rect::new(reach, run.clip.y - top, run.clip.width, run.clip.height);
        run.text.draw_run(
            &mut scratch,
            reach,
            run.baseline - top,
            clip,
            run.title,
            run.bold,
            Color::rgb(0xFF, 0xFF, 0xFF),
            run.size,
        );
        // White ink over clear: every inked pixel is opaque and its colour
        // channels carry the coverage (red read here).
        let coverage: Vec<u8> = scratch.pixels.iter().map(|&p| (p >> 16) as u8).collect();
        let (width, height) = scratch.device_size();
        let (width, height) = (width as usize, height as usize);
        let d = s * cs;
        let (above, below) = glow.map_or((0, 0), |g| g.rows[m]);
        let rows = (
            ((above * d) as usize).min(height),
            height.saturating_sub((below * d) as usize),
        );
        Some(TitleMask {
            coverage,
            x: r.x * cs,
            y: r.y * cs,
            width,
            height,
            rows,
            scale: d,
            fade_end: run.clip.right() * cs,
            fade: reach * cs,
        })
    }

    fn glow(&self, canvas: &mut Canvas, glow: &TitleGlow, active: bool) {
        let alpha = (glow.alpha)(&GlowMask {
            coverage: &self.coverage,
            width: self.width,
            height: self.height,
            rows: self.rows,
            active,
            scale: self.scale,
        });
        let from = self.fade_end - self.fade;
        let faded;
        let alpha: &[u8] = if self.x + self.width as i32 > from {
            let mut a = alpha.to_vec();
            for row in a.chunks_mut(self.width) {
                for (i, v) in row.iter_mut().enumerate() {
                    let left = (self.fade_end - (self.x + i as i32)).clamp(0, self.fade);
                    *v = ((*v as i32 * left + self.fade / 2) / self.fade.max(1)) as u8;
                }
            }
            faded = a;
            &faded
        } else {
            &alpha
        };
        let all = canvas.bounds();
        canvas.blit_mask_over(
            self.x,
            self.y,
            self.width,
            self.height,
            alpha,
            glow.color,
            all,
        );
    }

    fn ink(&self, canvas: &mut Canvas, ink: Color) {
        let all = canvas.bounds();
        canvas.blit_mask_over(
            self.x,
            self.y,
            self.width,
            self.height,
            &self.coverage,
            ink,
            all,
        );
    }
}

fn draw_buttons(
    canvas: &mut Canvas,
    style: &Style,
    geom: &FrameGeometry,
    state: &FrameState,
    s: i32,
) {
    let rest = if state.active {
        State::Normal
    } else {
        State::Inactive
    };
    if let Some(strip) = geom.group {
        let (part, r, glow) = match geom.chrome.buttons {
            ButtonSet::MinMaxClose if state.maximized => (Part::MaxCaptionGroup, strip, state.glow),
            ButtonSet::MinMaxClose => (Part::CaptionGroup, strip, state.glow),
            ButtonSet::MinClose => {
                let [min, _, close] = geom.buttons;
                let r = Rect::new(min.x, close.y, close.right() - min.x, close.height);
                (Part::MinCloseGroup, r.intersect(&strip), state.glow)
            }
            ButtonSet::Close => (
                Part::CloseButton,
                geom.buttons[2].intersect(&strip),
                Glow([0.0, 0.0, state.glow.of(CaptionButton::Close)]),
            ),
            ButtonSet::None => return,
        };
        if !style.buttons_over_caption {
            canvas.clear_rect(r);
        }
        let hot = if state.active {
            State::Hot
        } else {
            State::InactiveHot
        };
        // A strip lists its lit and held frames once per button.
        let n_of = |b| match geom.chrome.buttons {
            ButtonSet::MinClose => usize::from(b == CaptionButton::Close),
            _ => CaptionButton::ALL.iter().position(|&k| k == b).unwrap_or(0),
        };
        if let Some(b) = state.pressed {
            let st = if state.active {
                State::Pressed
            } else {
                State::InactiveHot
            };
            style.draw_nth(canvas, r, part, st, n_of(b), s);
        } else {
            let lit = CaptionButton::ALL.map(|b| (hot, n_of(b), glow.of(b)));
            style.draw_mixed(canvas, r, part, (rest, 0), &lit, s);
        }
        return;
    }
    for (kind, r) in CaptionButton::ALL.into_iter().zip(geom.buttons) {
        if !geom.chrome.buttons.has(kind) {
            continue;
        }
        let part = button_part(kind, state.maximized);
        if state.pressed == Some(kind) {
            style.draw(canvas, r, part, State::Pressed, s);
        } else {
            let lit = [(State::Hot, 0, state.glow.of(kind))];
            style.draw_mixed(canvas, r, part, (rest, 0), &lit, s);
        }
    }
}

/// `geom` and the point are logical, at scale 1: a frame at scale `s` is
/// this one with every pixel an `s` x `s` block.
pub fn frame_opaque_at(
    style: &Style,
    geom: &FrameGeometry,
    active: bool,
    maximized: bool,
    x: i32,
    y: i32,
) -> bool {
    if maximized {
        return true;
    }
    let o = geom.origin;
    let (w, h) = geom.total;
    let top = Rect::new(o.0, o.1, w, geom.client.y - o.1);
    let f = geom.frame;
    let bottom = Rect::new(o.0, o.1 + h - f, w, f);
    let part = if top.contains(x, y) {
        (Part::Caption, top)
    } else if f > 0 && bottom.contains(x, y) {
        (Part::FrameBottom, bottom)
    } else {
        return true;
    };
    let state = if active {
        State::Normal
    } else {
        State::Inactive
    };
    style.opaque_at(part.0, state, part.1, 1, x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Chrome, FrameGeometry, FrameHit, FrameState, Glow};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::style::{Part, State};

    fn state(active: bool, maximized: bool) -> FrameState<'static> {
        FrameState {
            title: "Notes",
            active,
            maximized,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        }
    }

    fn draw(geom: &FrameGeometry, st: &FrameState) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let mut c = Canvas::new(geom.total.0 as u32, geom.total.1 as u32);
        draw_frame(&mut c, theme.look.style().unwrap(), &theme, &text, geom, st);
        c
    }

    #[test]
    fn blue_frame_puts_the_caption_rows_in_the_top_strip() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
        assert_eq!(g.client.y, 30);
        let c = draw(&g, &state(true, false));
        let mut want = Canvas::new(g.total.0 as u32, 30);
        s.draw(
            &mut want,
            Rect::new(0, 0, g.total.0, 30),
            Part::Caption,
            State::Normal,
            1,
        );
        assert_eq!(c.get(12, 2), want.get(12, 2));
        assert_eq!(c.get(0, 0), 0, "the rounded corner stays clear");
        assert_eq!(
            c.get(g.client.x + 5, g.client.y + 5),
            0,
            "the client hole is clear"
        );
    }

    #[test]
    fn a_grown_blue_caption_draws_its_buttons_as_big_as_their_cells() {
        for (pct, side) in [(150, 28), (200, 34)] {
            let theme = theme_for(Interface::Blue).with_text_size(pct);
            let style = theme.look.style().unwrap();
            let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
            let close = g.buttons[2];
            assert_eq!((close.width, close.height), (side, side), "{pct}%");
            let text = TextRenderer::load(&style.caption_text.font());
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut c, style, &theme, &text, &g, &state(true, false));
            let grown = style.at_button_side(side).unwrap();
            let mut want = Canvas::new(side as u32, side as u32);
            let at = Rect::new(0, 0, side, side);
            grown.draw(&mut want, at, Part::CloseButton, State::Normal, 1);
            let mid = side / 2;
            for (x, y) in [(mid, 1), (1, mid), (mid, mid), (side - 2, mid)] {
                assert_eq!(
                    c.get(close.x + x, close.y + y),
                    want.get(x, y),
                    "{pct}%: close at ({x}, {y})"
                );
            }
        }
        assert!(
            theme_for(Interface::Blue)
                .look
                .style()
                .unwrap()
                .at_button_side(21)
                .is_none()
        );
    }

    #[test]
    fn a_grown_blue_caption_has_its_buttons_built_ahead_at_each_scale() {
        use crate::frame::caption_button_sides;
        use crate::paint::Look;
        let styled = |pct| {
            let theme = theme_for(Interface::Blue).with_text_size(pct);
            let Look::Styled(style) = &theme.look else {
                panic!("Blue is styled")
            };
            (style.clone(), caption_button_sides(&theme.metrics))
        };
        let (style, sides) = styled(100);
        for n in [1, 2] {
            assert!(
                style.warm_button_sides(n, &sides).is_none(),
                "100% draws its own"
            );
        }
        let (style, sides) = styled(150);
        assert_eq!(sides, [28]);
        for n in [1, 2] {
            let warm = (style.warm_button_sides(n, &sides)).expect("28 px buttons to build");
            std::thread::spawn(move || warm.paint()).join().unwrap();
            assert!(
                style.warm_button_sides(n, &sides).is_none(),
                "scale {n}: a first frame finds them built"
            );
        }
    }

    #[test]
    fn a_grown_glass_caption_draws_its_strip_as_big_as_its_cells() {
        use crate::frame::{button_side, caption_button_sides};
        for (pct, side) in [(150, 27), (200, 33)] {
            for n in [1, 2] {
                let theme = theme_for(Interface::Glass).with_text_size(pct).scaled(n);
                let style = theme.look.style().unwrap();
                assert_eq!(button_side(&theme.metrics) / n, side, "{pct}%");
                let text = TextRenderer::load(&style.caption_text.font());
                let grown = style.at_button_side(side).unwrap();
                for (maximized, part) in
                    [(false, Part::CaptionGroup), (true, Part::MaxCaptionGroup)]
                {
                    let g =
                        FrameGeometry::layout(&theme.metrics, 400, 200, maximized, Chrome::WINDOW);
                    let strip = g.group.unwrap();
                    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                    draw_frame(&mut c, style, &theme, &text, &g, &state(true, maximized));
                    let mut want = Canvas::new(strip.width as u32, strip.height as u32);
                    let at = Rect::new(0, 0, strip.width, strip.height);
                    grown.draw(&mut want, at, part, State::Normal, n);
                    for y in 0..strip.height {
                        for x in 0..strip.width {
                            assert_eq!(
                                c.get(strip.x + x, strip.y + y),
                                want.get(x, y),
                                "{pct}% at {n}, maximized {maximized}: ({x}, {y})"
                            );
                        }
                    }
                }
            }
            let theme = theme_for(Interface::Glass).with_text_size(pct);
            assert_eq!(caption_button_sides(&theme.metrics), [side]);
            let crate::paint::Look::Styled(style) = &theme.look else {
                panic!("Glass is styled")
            };
            let warm = style
                .warm_button_sides(1, &[side])
                .expect("the grown strip to build");
            warm.paint();
            assert!(style.warm_button_sides(1, &[side]).is_none());
        }
        let theme = theme_for(Interface::Glass);
        let crate::paint::Look::Styled(style) = &theme.look else {
            panic!("Glass is styled")
        };
        assert!(style.at_button_side(button_side(&theme.metrics)).is_none());
        assert!(
            style
                .warm_button_sides(1, &caption_button_sides(&theme.metrics))
                .is_none()
        );
    }

    #[test]
    fn maximized_blue_frame_uses_the_short_caption() {
        let theme = theme_for(Interface::Blue);
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, true, Chrome::WINDOW);
        assert_eq!((g.frame, g.client.y), (0, 26));
        let c = draw(&g, &state(true, true));
        assert_ne!(c.get(0, 0), 0, "a maximized caption has no rounded corner");
    }

    #[test]
    fn a_blue_caption_leaves_out_its_dialog_row_only_at_29_rows() {
        use crate::blue::Variant;
        use crate::interfaces::blue_theme;
        for v in Variant::ALL {
            let drop = v.scheme().metrics.dialog_drop.caption;
            let mut theme = blue_theme(v);
            let style = theme.look.style().unwrap();
            let text = TextRenderer::load(&style.caption_text.font());
            let base = theme.metrics.caption_height;
            let mut band = Canvas::new(400, 30);
            style.draw(
                &mut band,
                Rect::new(0, 0, 400, 30),
                Part::Caption,
                State::Normal,
                1,
            );
            for (chrome, f) in [(Chrome::WINDOW, 4), (Chrome::DIALOG, 3)] {
                let tops = [18, 25, 29, 30, 34, 40, base + 3 + f, base + 6 + f];
                for top in tops {
                    theme.metrics.caption_height = top - f;
                    let g = FrameGeometry::layout(&theme.metrics, 400, 60, false, chrome);
                    assert_eq!(g.client.y, top);
                    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                    let st = FrameState {
                        title: "",
                        ..state(true, false)
                    };
                    draw_frame(&mut c, style, &theme, &text, &g, &st);
                    let rows: Vec<usize> = if top == 29 {
                        (0..30).filter(|&r| r != drop).collect()
                    } else {
                        (0..top).map(|y| (y * 30 / top) as usize).collect()
                    };
                    for (y, r) in rows.into_iter().enumerate() {
                        assert_eq!(
                            c.get(100, y as i32),
                            band.get(100, r as i32),
                            "{v:?} {top}-row caption with a {f} px border: row {y} is band row {r}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_blue_dialogs_3_px_border_keeps_the_strips_last_columns_and_rows_0_2_3() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let part = |p: Part, w: i32, h: i32| {
            let mut c = Canvas::new(w as u32, h as u32);
            s.draw(&mut c, Rect::new(0, 0, w, h), p, State::Normal, 1);
            c
        };
        let (left, right) = (part(Part::FrameLeft, 4, 8), part(Part::FrameRight, 4, 8));
        let bottom = part(Part::FrameBottom, 40, 4);
        let g = FrameGeometry::layout(&theme.metrics, 200, 100, false, Chrome::DIALOG);
        assert_eq!(g.frame, 3);
        let c = draw(&g, &state(true, false));
        let (w, h) = g.total;
        let y = g.client.y;
        for i in 0..3 {
            assert_eq!(c.get(i, y), left.get(i + 1, 4), "left border column {i}");
            assert_eq!(
                c.get(w - 3 + i, y),
                right.get(i + 1, 4),
                "right border column {i}"
            );
        }
        for (i, row) in [0, 2, 3].into_iter().enumerate() {
            assert_eq!(
                c.get(20, h - 3 + i as i32),
                bottom.get(20, row),
                "bottom border row {i} is strip row {row}"
            );
        }
    }

    #[test]
    fn pressed_close_draws_the_pressed_frame() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
        let mut st = state(true, false);
        st.pressed = Some(CaptionButton::Close);
        let c = draw(&g, &st);
        let r = g.buttons[2];
        let mut want = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        s.draw(&mut want, r, Part::CloseButton, State::Pressed, 1);
        assert_eq!(c.get(r.x + 2, r.y + 3), want.get(r.x + 2, r.y + 3));
    }

    #[test]
    fn only_a_focused_title_casts_a_shadow() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        if !text.has_face("Trebuchet MS") && !text.has_face("DejaVu Sans") {
            return;
        }
        let shadow = s.caption_text.shadow.unwrap().color.argb();
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
        let count = |c: &Canvas| {
            (0..g.client.y)
                .flat_map(|y| (0..g.buttons[0].x).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == shadow)
                .count()
        };
        assert!(
            count(&draw(&g, &state(true, false))) > 0,
            "an active title casts one"
        );
        assert_eq!(
            count(&draw(&g, &state(false, false))),
            0,
            "an inactive title casts none"
        );
    }

    #[test]
    fn the_caption_icon_sits_2_px_in_and_4_px_down() {
        let theme = theme_for(Interface::Blue);
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(
            Rect::new(0, 0, 16, 16),
            crate::theme::Color::rgb(0xFF, 0, 0xFF),
        );
        let magenta = icon.get(0, 0);
        for maximized in [false, true] {
            let g = FrameGeometry::layout(&theme.metrics, 300, 200, maximized, Chrome::WINDOW);
            let mut st = state(true, maximized);
            st.icon = Some(&icon);
            let c = draw(&g, &st);
            let (x, y) = (g.caption.x + 2, g.caption.y + 4);
            assert_eq!(
                c.get(x, y),
                magenta,
                "icon's top-left, maximized {maximized}"
            );
            assert_ne!(
                c.get(x, y - 1),
                magenta,
                "the row above it, maximized {maximized}"
            );
            assert_eq!(c.get(x + 15, y + 15), magenta);
        }
    }

    #[test]
    fn caption_corner_pixels_are_not_opaque() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let top = Rect::new(0, 0, 300, 30);
        assert!(!s.opaque_at(Part::Caption, State::Normal, top, 1, 0, 0));
        assert!(!s.opaque_at(Part::Caption, State::Normal, top, 1, 299, 0));
        assert!(s.opaque_at(Part::Caption, State::Normal, top, 1, 5, 0));
    }

    #[test]
    fn frame_opaque_at_clears_only_the_rounded_corners() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
        let right = g.total.0 - 1;
        for active in [true, false] {
            assert!(
                !frame_opaque_at(s, &g, active, false, 0, 0),
                "top-left corner is clear"
            );
            assert!(
                !frame_opaque_at(s, &g, active, false, 1, 1),
                "(1, 1) is clear"
            );
            assert!(
                !frame_opaque_at(s, &g, active, false, right, 0),
                "top-right corner is clear"
            );
            assert!(
                frame_opaque_at(s, &g, active, false, 5, 0),
                "(5, 0) is caption"
            );
            assert!(
                frame_opaque_at(s, &g, active, false, 0, 5),
                "(0, 5) is caption"
            );
            let b = g.buttons[2];
            assert!(
                frame_opaque_at(s, &g, active, false, b.x + 1, b.y + 1),
                "close button"
            );
            assert!(
                frame_opaque_at(s, &g, active, false, 0, g.client.y),
                "left border"
            );
        }
    }

    #[test]
    fn a_maximized_caption_is_opaque_to_its_corners() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, true, Chrome::WINDOW);
        assert!(frame_opaque_at(s, &g, true, true, 0, 0));
        assert!(frame_opaque_at(s, &g, true, true, g.total.0 - 1, 0));
    }

    #[test]
    fn a_glass_frame_passes_clicks_through_all_four_cut_corners() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let g = FrameGeometry::layout(&theme.metrics, 484, 262, false, Chrome::WINDOW);
        let (w, h) = g.total;
        for active in [true, false] {
            let at = |x, y| frame_opaque_at(s, &g, active, false, x, y);
            assert!(!at(0, 0) && !at(w - 1, 0), "the top corners are clear");
            assert!(!at(0, h - 1) && !at(w - 1, h - 1), "the bottom corners too");
            assert!(
                at(5, 0) && at(w - 6, h - 1),
                "the outline beside them is not"
            );
            assert!(at(0, h - 7), "the left border above the bottom corner");
            assert!(frame_opaque_at(s, &g, active, true, 0, 0), "maximized");
        }
    }

    fn glass(w: i32, h: i32, chrome: Chrome, st: &FrameState) -> (FrameGeometry, Canvas) {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let g = if st.maximized {
            FrameGeometry::layout(&theme.metrics, w, h - 22, true, chrome)
        } else {
            FrameGeometry::layout(&theme.metrics, w - 16, h - 38, false, chrome)
        };
        assert_eq!(g.total, (w, h));
        let text = TextRenderer::load(&s.caption_text.font());
        let mut c = Canvas::new(w as u32, h as u32);
        draw_frame(&mut c, s, &theme, &text, &g, st);
        (g, c)
    }

    #[test]
    fn glass_joins_its_caption_buttons_at_the_top_right() {
        let (g, _) = glass(800, 500, Chrome::WINDOW, &state(true, false));
        assert_eq!(g.group, Some(Rect::new(689, 1, 105, 20)), "W-111, 1");
        assert_eq!(
            g.buttons,
            [
                Rect::new(689, 1, 30, 20),
                Rect::new(718, 1, 28, 20),
                Rect::new(745, 1, 49, 20),
            ],
            "cells 30, 28 and 49 wide, dividers shared"
        );
        let m = &theme_for(Interface::Glass).metrics;
        let hit = |g: &FrameGeometry, x, y| g.hit(x, y, m);
        assert_eq!(hit(&g, 689, 1), FrameHit::Button(CaptionButton::Minimize));
        assert_eq!(hit(&g, 718, 10), FrameHit::Button(CaptionButton::Minimize));
        assert_eq!(hit(&g, 719, 10), FrameHit::Button(CaptionButton::Maximize));
        assert_eq!(hit(&g, 745, 10), FrameHit::Button(CaptionButton::Maximize));
        assert_eq!(hit(&g, 746, 20), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(&g, 793, 1), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(&g, 688, 10), FrameHit::Title);
        assert_eq!(hit(&g, 750, 21), FrameHit::Title, "below the strip");
        assert!(
            matches!(hit(&g, 794, 10), FrameHit::Resize(_)),
            "right of it"
        );

        let (mx, _) = glass(1920, 1040, Chrome::WINDOW, &state(true, true));
        assert_eq!(mx.group, Some(Rect::new(1813, 0, 105, 19)), "W-107, 0");
        assert_eq!(mx.buttons[2], Rect::new(1869, 0, 51, 19));
        assert_eq!(hit(&mx, 1919, 0), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(&mx, 1919, 18), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(&mx, 1919, 19), FrameHit::Title, "below the strip");

        let (dlg, _) = glass(413, 212, Chrome::DIALOG, &state(true, false));
        assert_eq!(dlg.frame, 8, "a dialog's border is a window's");
        assert_eq!(dlg.buttons[2], Rect::new(358, 1, 49, 20), "W-55, 1");
        assert_eq!(
            dlg.help_box(),
            Rect::new(329, 1, 30, 20),
            "the ? box is 30 wide, its right divider the close box's left"
        );
    }

    #[test]
    fn the_lit_button_picks_the_whole_strip_frame() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let group = s.part(Part::CaptionGroup).unwrap();
        let cases = [
            (true, None, None, State::Normal, 0),
            (false, None, None, State::Inactive, 0),
            (true, Some(CaptionButton::Maximize), None, State::Hot, 1),
            (true, None, Some(CaptionButton::Close), State::Pressed, 2),
            (
                false,
                Some(CaptionButton::Minimize),
                None,
                State::InactiveHot,
                0,
            ),
            (
                false,
                None,
                Some(CaptionButton::Close),
                State::InactiveHot,
                2,
            ),
        ];
        for (active, hot, pressed, want, n) in cases {
            let st = FrameState {
                glow: Glow::lit(hot),
                pressed,
                ..state(active, false)
            };
            let (g, c) = glass(800, 500, Chrome::WINDOW, &st);
            let r = g.group.unwrap();
            let f = group.frame_nth(want, n);
            for (x, y) in [(0, 10), (32, 10), (65, 14), (104, 19), (50, 0)] {
                assert_eq!(
                    c.get(r.x + x, r.y + y),
                    group.atlas.get(f.x + x, f.y + y),
                    "{want:?} {n} at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_half_glow_sits_between_the_rest_and_the_lit_frame() {
        let rest = glass(800, 500, Chrome::WINDOW, &state(true, false)).1;
        let lit = glass(
            800,
            500,
            Chrome::WINDOW,
            &FrameState {
                glow: Glow::lit(Some(CaptionButton::Close)),
                ..state(true, false)
            },
        )
        .1;
        let (g, half) = glass(
            800,
            500,
            Chrome::WINDOW,
            &FrameState {
                glow: Glow([0.0, 0.0, 0.5]),
                ..state(true, false)
            },
        );
        let r = g.group.unwrap();
        let mut want = rest.clone();
        want.mix(&lit, r, 0.5);
        assert_eq!(
            half.pixels, want.pixels,
            "the strip is the rest and lit frames mixed half way"
        );
    }

    fn glass_glow(glow: Glow) -> (FrameGeometry, Canvas) {
        glass(
            800,
            500,
            Chrome::WINDOW,
            &FrameState {
                glow,
                ..state(true, false)
            },
        )
    }

    #[test]
    fn two_glows_mix_both_and_a_press_ignores_them() {
        let (g, two) = glass_glow(Glow([0.0, 0.25, 0.75]));
        let rest = glass_glow(Glow::default()).1;
        let max = glass_glow(Glow::lit(Some(CaptionButton::Maximize))).1;
        let close = glass_glow(Glow::lit(Some(CaptionButton::Close))).1;
        let mut want = rest.clone();
        want.mix_all(&[(&max, 0.25), (&close, 0.75)], g.group.unwrap());
        assert_eq!(
            two.pixels, want.pixels,
            "the strip moves a quarter toward lit maximize and three quarters toward lit close"
        );
        let pressed = FrameState {
            glow: Glow([0.0, 0.25, 0.75]),
            pressed: Some(CaptionButton::Close),
            ..state(true, false)
        };
        let plain = FrameState {
            pressed: Some(CaptionButton::Close),
            ..state(true, false)
        };
        assert_eq!(
            glass(800, 500, Chrome::WINDOW, &pressed).1.pixels,
            glass(800, 500, Chrome::WINDOW, &plain).1.pixels
        );
    }

    #[test]
    fn a_lit_neighbour_leaves_a_half_lit_button_at_half() {
        let (g, half) = glass_glow(Glow([0.5, 0.0, 0.0]));
        let with_max = glass_glow(Glow([0.5, 1.0, 0.0])).1;
        let rest = glass_glow(Glow::default()).1;
        let max = glass_glow(Glow::lit(Some(CaptionButton::Maximize))).1;
        let cell = g.buttons[0].intersect(&g.group.unwrap());
        let mut same = 0;
        for y in cell.y..cell.bottom() {
            for x in cell.x..cell.right() {
                if max.get(x, y) == rest.get(x, y) {
                    same += 1;
                    assert_eq!(
                        with_max.get(x, y),
                        half.get(x, y),
                        "({x}, {y}): minimize stays half lit beside a lit maximize"
                    );
                }
            }
        }
        assert!(
            same > cell.width * cell.height / 2,
            "most of the minimize cell is out of maximize's glow: {same}"
        );
    }

    #[test]
    fn a_half_glow_on_a_lone_button_sits_between_its_rest_and_lit_frames() {
        let theme = theme_for(Interface::Blue);
        let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
        assert_eq!(g.group, None, "Blue's buttons stand alone");
        let at = |glow: Glow| {
            draw(
                &g,
                &FrameState {
                    glow,
                    ..state(true, false)
                },
            )
        };
        let mut want = at(Glow::default());
        want.mix(
            &at(Glow::lit(Some(CaptionButton::Minimize))),
            g.buttons[0],
            0.5,
        );
        assert_eq!(
            at(Glow([0.5, 0.0, 0.0])).pixels,
            want.pixels,
            "minimize is its rest and lit frames mixed half way, over the caption"
        );
    }

    #[test]
    fn a_maximized_glass_window_draws_the_restore_strip() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let max = s.part(Part::MaxCaptionGroup).unwrap();
        let st = FrameState {
            glow: Glow::lit(Some(CaptionButton::Maximize)),
            ..state(true, true)
        };
        let (g, c) = glass(1000, 300, Chrome::WINDOW, &st);
        let r = g.group.unwrap();
        let f = max.frame_nth(State::Hot, 1);
        for (x, y) in [(36, 5), (43, 17), (65, 14), (104, 18)] {
            assert_eq!(c.get(r.x + x, r.y + y), max.atlas.get(f.x + x, f.y + y));
        }
    }

    #[test]
    fn a_glass_dialog_draws_its_lone_close_box() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let close = s.part(Part::CloseButton).unwrap();
        let group = s.part(Part::CaptionGroup).unwrap();
        let st = FrameState {
            glow: Glow::lit(Some(CaptionButton::Close)),
            ..state(true, false)
        };
        let (g, c) = glass(413, 212, Chrome::DIALOG, &st);
        let r = g.buttons[2];
        let f = close.frame(State::Hot);
        for (x, y) in [(0, 10), (1, 10), (10, 14), (48, 19)] {
            assert_eq!(c.get(r.x + x, r.y + y), close.atlas.get(f.x + x, f.y + y));
        }
        let n = group.frame(State::Normal);
        assert_ne!(
            c.get(r.x - 10, r.y + 10),
            group.atlas.get(n.x + 46, n.y + 10)
        );
    }

    fn glass_title_face() -> bool {
        let theme = theme_for(Interface::Glass);
        let text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let found = ["Segoe UI", "Noto Sans", "DejaVu Sans"]
            .iter()
            .any(|f| text.has_face(f));
        if !found {
            eprintln!("skipped: none of Segoe UI, Noto Sans or DejaVu Sans is installed");
        }
        found
    }

    fn ink_box(c: &Canvas, x0: i32, x1: i32) -> Option<(i32, i32, i32, i32)> {
        ink_box_in(c, x0, x1, 2, 28)
    }

    fn ink_box_in(c: &Canvas, x0: i32, x1: i32, y0: i32, y1: i32) -> Option<(i32, i32, i32, i32)> {
        let mut grey = Canvas::new(c.width, c.height);
        grey.fill_rect(grey.bounds(), crate::theme::Color::rgb(128, 128, 128));
        grey.blit_over(0, 0, c);
        let dark = |x, y| {
            let v = grey.get(x, y);
            ((v >> 16) & 0xFF) + ((v >> 8) & 0xFF) + (v & 0xFF) < 150
        };
        let pts: Vec<(i32, i32)> = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| dark(x, y))
            .collect();
        let xs = pts.iter().map(|p| p.0);
        let ys = pts.iter().map(|p| p.1);
        Some((xs.clone().min()?, xs.max()?, ys.clone().min()?, ys.max()?))
    }

    #[test]
    fn a_glass_title_starts_at_x_30_on_row_18_and_its_icon_at_8_7() {
        if !glass_title_face() {
            return;
        }
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), crate::theme::Color::rgb(0xFF, 0, 0xFF));
        let magenta = icon.get(0, 0);
        for (maximized, (ix, iy), baseline_row) in [(false, (8, 7), 18), (true, (0, 3), 14)] {
            let st = FrameState {
                title: "UU",
                icon: Some(&icon),
                ..state(true, maximized)
            };
            let (_, c) = glass(800, 500, Chrome::WINDOW, &st);
            assert_eq!(
                c.get(ix, iy),
                magenta,
                "the icon's top-left, maximized {maximized}"
            );
            assert_ne!(c.get(ix, iy - 1), magenta, "the row above it");
            assert_eq!(c.get(ix + 15, iy + 15), magenta, "its bottom-right");
            let (x0, _, top, bottom) = ink_box(&c, 24, 200).expect("the title has ink");
            let left = if maximized { 22 } else { 30 };
            assert!(
                (left..=left + 2).contains(&x0),
                "maximized {maximized}: the title starts at x {left}, its U's stem by {}, got {x0}",
                left + 2
            );
            assert_eq!(
                bottom, baseline_row,
                "maximized {maximized}: the U ends on its row"
            );
            assert!(
                bottom - top >= 7,
                "maximized {maximized}: capitals about 9 rows tall"
            );
        }
    }

    #[test]
    fn a_grown_glass_title_keeps_its_whole_height_inside_the_caption() {
        if !glass_title_face() {
            return;
        }
        let base = theme_for(Interface::Glass);
        let ink = |pct: u32, maximized: bool, n: i32| {
            let grown = base.with_text_size(pct);
            let g = FrameGeometry::layout(&grown.metrics, 800, 400, maximized, Chrome::WINDOW)
                .scaled(n);
            let theme = grown.scaled(n);
            let s = theme.look.style().unwrap();
            let text = TextRenderer::load(&s.caption_text.font());
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            let st = FrameState {
                title: "Hg",
                ..state(true, maximized)
            };
            draw_frame(&mut c, s, &theme, &text, &g, &st);
            let x0 = if maximized { 20 } else { 28 };
            let (_, _, top, bottom) = ink_box_in(&c, x0 * n, (x0 + 45) * n, 2 * n, g.client.y)
                .expect("the title has ink");
            (g.caption, top, bottom)
        };
        for (maximized, n) in [(false, 1), (true, 1), (false, 2)] {
            let (_, top, bottom) = ink(100, maximized, n);
            for pct in [150, 200] {
                let (caption, big_top, big_bottom) = ink(pct, maximized, n);
                let want = (bottom - top + 1) * pct as i32 / 100;
                assert!(
                    (big_bottom - big_top + 1 - want).abs() <= 2 * n,
                    "maximized {maximized}, scale {n}, {pct}%: the title is {want} rows tall, got {big_top}..={big_bottom}"
                );
                assert!(
                    big_top > caption.y && big_bottom < caption.bottom(),
                    "maximized {maximized}, scale {n}, {pct}%: rows {big_top}..={big_bottom} sit inside {caption:?}"
                );
            }
        }
    }

    #[test]
    fn a_grown_caption_keeps_its_icon_centred_on_the_growth() {
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), crate::theme::Color::rgb(0xFF, 0, 0xFF));
        let magenta = icon.get(0, 0);
        let top_row = |look: Interface, pct: u32| {
            let theme = theme_for(look).with_text_size(pct);
            let s = theme.look.style().unwrap();
            let g = FrameGeometry::layout(&theme.metrics, 800, 400, false, Chrome::WINDOW);
            let text = TextRenderer::load(&s.caption_text.font());
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            let st = FrameState {
                icon: Some(&icon),
                ..state(true, false)
            };
            draw_frame(&mut c, s, &theme, &text, &g, &st);
            let row = (0..g.client.y).find(|&y| c.get(8, y) == magenta);
            (row.expect("the icon shows"), g.caption.height)
        };
        for look in [Interface::Glass, Interface::Blue] {
            let (y, h) = top_row(look, 100);
            for pct in [150, 200] {
                let (big_y, big_h) = top_row(look, pct);
                assert_eq!(big_y - y, (big_h - h) / 2, "{look:?} {pct}%");
            }
        }
    }

    #[test]
    fn a_glass_title_glows_in_the_glass_rows_only() {
        if !glass_title_face() {
            return;
        }
        for active in [true, false] {
            let bare = glass(
                800,
                500,
                Chrome::WINDOW,
                &FrameState {
                    title: "",
                    ..state(active, false)
                },
            )
            .1;
            let titled = glass(
                800,
                500,
                Chrome::WINDOW,
                &FrameState {
                    title: "Untitled - Notepad",
                    ..state(active, false)
                },
            )
            .1;
            let alpha = |c: &Canvas, x, y| c.get(x, y) >> 24;
            for (x, y) in [(60, 4), (80, 24), (26, 14)] {
                assert!(
                    alpha(&titled, x, y) > alpha(&bare, x, y),
                    "active {active}: ({x}, {y}) glows"
                );
            }
            for y in [0, 1, 28, 29] {
                assert_eq!(
                    titled.get(80, y),
                    bare.get(80, y),
                    "active {active}: row {y} has no glow"
                );
            }
            assert_eq!(
                titled.get(250, 14),
                bare.get(250, 14),
                "active {active}: far right of the text"
            );
        }
    }

    #[test]
    fn a_long_glass_title_and_its_glow_stop_before_the_buttons() {
        if !glass_title_face() {
            return;
        }
        let long = "A title long enough to run on under the caption buttons of this window";
        for active in [true, false] {
            let (g, bare) = glass(360, 200, Chrome::WINDOW, &state(active, false));
            let titled = glass(
                360,
                200,
                Chrome::WINDOW,
                &FrameState {
                    title: long,
                    ..state(active, false)
                },
            )
            .1;
            let first = g.buttons[0].x;
            assert!(
                (2..28).any(|y| titled.get(first - 30, y) != bare.get(first - 30, y)),
                "active {active}: the glow reaches x {}",
                first - 30
            );
            for y in 0..30 {
                for x in first..first + 3 {
                    assert_eq!(
                        titled.get(x, y),
                        bare.get(x, y),
                        "active {active}: ({x}, {y}) from the first button on is the bare frame"
                    );
                }
            }
            let alpha = |c: &Canvas, x, y| (c.get(x, y) >> 24) as i32;
            for y in 2..8 {
                let near = alpha(&titled, first - 1, y) - alpha(&bare, first - 1, y);
                assert!(
                    (0..=8).contains(&near),
                    "active {active}: row {y} next to the button glows {near}/255"
                );
            }
        }
    }

    #[test]
    fn a_glass_title_in_a_40_px_rect_still_shows() {
        if !glass_title_face() {
            return;
        }
        let (g, c) = glass(
            181,
            120,
            Chrome::WINDOW,
            &FrameState {
                title: "Untitled - Notepad",
                ..state(true, false)
            },
        );
        assert_eq!(g.buttons[0].x - 30, 40, "a 40 px title rect");
        let (x0, x1, _, _) = ink_box(&c, 24, g.buttons[0].x).expect("the title has ink");
        assert!(x0 <= 32 && x1 >= 60, "ink from x {x0} to {x1}");
    }

    #[test]
    fn a_glass_title_at_scale_2_glows_on_rows_4_to_55_from_x_60() {
        if !glass_title_face() {
            return;
        }
        let base = theme_for(Interface::Glass);
        let theme = base.scaled(2);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let g = FrameGeometry::layout(&base.metrics, 784, 462, false, Chrome::WINDOW).scaled(2);
        let draw = |title| {
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            let st = FrameState {
                title,
                ..state(true, false)
            };
            draw_frame(&mut c, s, &theme, &text, &g, &st);
            c
        };
        let (bare, titled) = (draw(""), draw("Untitled - Notepad"));
        let glows = |y| (100..300).any(|x| titled.get(x, y) >> 24 > bare.get(x, y) >> 24);
        for y in 0..4 {
            assert!(!glows(y), "row {y} has no glow");
        }
        for y in [4, 30, 55] {
            assert!(glows(y), "row {y} glows");
        }
        for y in 56..60 {
            assert!(!glows(y), "row {y} has no glow");
        }
        let (x0, _, _, bottom) = ink_box_in(&titled, 48, 400, 4, 56).expect("the title has ink");
        assert!(
            (60..=64).contains(&x0),
            "the title starts at x 60, its stem by 64: {x0}"
        );
        if !text.has_face("DejaVu Sans") {
            assert_eq!(bottom, 43, "descenders end on row 43 (21 at scale 1)");
        }
    }

    #[test]
    fn a_glass_frame_drawn_strip_by_strip_is_the_whole_frame() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let g = FrameGeometry::layout(&theme.metrics, 784, 462, false, Chrome::WINDOW);
        let st = FrameState {
            title: "Untitled - Notepad",
            ..state(true, false)
        };
        let mut whole = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(&mut whole, s, &theme, &text, &g, &st);
        for r in g.strips() {
            let mut strip = Canvas::new(r.width as u32, r.height as u32);
            draw_frame(&mut strip, s, &theme, &text, &g.translated(r.x, r.y), &st);
            for y in 0..r.height {
                for x in 0..r.width {
                    assert_eq!(
                        strip.get(x, y),
                        whole.get(r.x + x, r.y + y),
                        "strip {r:?} at ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn glass_run_under_the_clients_top_opens_its_edges_beside_and_above() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        for maximized in [false, true] {
            let g = FrameGeometry::layout(&theme.metrics, 400, 300, maximized, Chrome::WINDOW);
            let open = g.with_glass_top(60);
            for active in [true, false] {
                let st = FrameState {
                    title: "",
                    ..state(active, maximized)
                };
                let draw = |g: &FrameGeometry| {
                    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                    draw_frame(&mut c, s, &theme, &text, g, &st);
                    c
                };
                let (shut, wide) = (draw(&g), draw(&open));
                let c = g.client;
                let mut under = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                let band = Rect::new(c.x, c.y, c.width, 60);
                let band_state = if active {
                    State::Normal
                } else {
                    State::Inactive
                };
                draw_client_glass(&mut under, s, band, c.height, maximized, band_state, 1);
                let glass = |x: i32, y: i32| under.get(x, y);
                let what = format!("maximized {maximized}, active {active}");
                for y in [c.y - 2, c.y - 1] {
                    let x = c.x + 100;
                    assert_ne!(shut.get(x, y), glass(x, y), "{what}: row {y} is an edge");
                    assert_eq!(wide.get(x, y), glass(x, y), "{what}: row {y} opens");
                }
                assert_eq!(
                    wide.get(c.x + 100, c.y - 3),
                    shut.get(c.x + 100, c.y - 3),
                    "{what}: the caption above the edges"
                );
                assert_eq!(
                    wide.get(c.x + 5, c.y + 5),
                    0,
                    "{what}: the client stays clear"
                );
                if maximized {
                    continue;
                }
                for x in [c.x - 2, c.x - 1, c.right(), c.right() + 1] {
                    assert_eq!(
                        wide.get(x, c.y + 30),
                        glass(x, c.y + 30),
                        "{what}: column {x} beside the glass"
                    );
                    assert_eq!(
                        wide.get(x, c.y + 60),
                        shut.get(x, c.y + 60),
                        "{what}: column {x} below it keeps its edge"
                    );
                }
                assert_eq!(
                    wide.get(c.x - 3, c.y + 30),
                    shut.get(c.x - 3, c.y + 30),
                    "{what}: the border outside the edges"
                );
            }
        }
    }

    #[test]
    fn glass_under_a_restored_clients_top_carries_on_from_the_caption_and_borders() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let g = FrameGeometry::layout(&theme.metrics, 600, 300, false, Chrome::WINDOW)
            .with_glass_top(60);
        let c = g.client;
        let band = Rect::new(c.x, c.y, c.width, g.glass_top);
        for active in [true, false] {
            let st = FrameState {
                title: "",
                ..state(active, false)
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut f, s, &theme, &text, &g, &st);
            let band_state = if active {
                State::Normal
            } else {
                State::Inactive
            };
            draw_client_glass(&mut f, s, band, c.height, false, band_state, 1);
            let near = |a: u32, b: u32| {
                (0..4).all(|i| (a >> (8 * i) & 0xFF).abs_diff(b >> (8 * i) & 0xFF) <= 6)
            };
            for x in [c.x, c.x + 40, c.x + 120, c.right() - 1, c.right() - 41] {
                let (above, below) = (f.get(x, c.y - 3), f.get(x, c.y));
                assert!(
                    near(above, below),
                    "active {active}, column {x}: {above:08X} over {below:08X}"
                );
            }
            for y in [c.y, c.y + 20, c.y + 40, c.y + 59] {
                for (border, glass) in [(c.x - 3, c.x), (c.right() + 2, c.right() - 1)] {
                    let (b, g) = (f.get(border, y), f.get(glass, y));
                    assert!(
                        near(b, g),
                        "active {active}, row {y}: border {b:08X} beside {g:08X}"
                    );
                }
            }
        }
    }

    #[test]
    fn glass_under_the_top_meets_its_side_borders_at_any_window_height() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let near = |a: u32, b: u32| {
            (0..4).all(|i| (a >> (8 * i) & 0xFF).abs_diff(b >> (8 * i) & 0xFF) <= 6)
        };
        let mut inside = Vec::new();
        for height in [120, 300, 900] {
            let g = FrameGeometry::layout(&theme.metrics, 500, height, false, Chrome::WINDOW)
                .with_glass_top(80);
            let c = g.client;
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut f, s, &theme, &text, &g, &state(true, false));
            let band = Rect::new(c.x, c.y, c.width, g.glass_top);
            draw_client_glass(&mut f, s, band, c.height, false, State::Normal, 1);
            for y in [c.y, c.y + 40, c.y + 79] {
                for (border, glass) in [(c.x - 3, c.x), (c.right() + 2, c.right() - 1)] {
                    let (b, g) = (f.get(border, y), f.get(glass, y));
                    assert!(near(b, g), "{height} tall, row {y}: {b:08X} beside {g:08X}");
                }
            }
            inside.push(f.get(c.x + 40, c.y + 60));
        }
        assert!(
            inside.windows(2).all(|w| w[0] == w[1]),
            "past the edges the glass is the same at any height: {inside:08X?}"
        );
    }

    #[test]
    fn a_frame_without_client_glass_ignores_the_glass_top() {
        for look in [Interface::Blue, Interface::Classic] {
            let theme = theme_for(look);
            let g = FrameGeometry::layout(&theme.metrics, 300, 200, false, Chrome::WINDOW);
            let text = TextRenderer::load(&theme.fonts);
            let draw = |g: &FrameGeometry| {
                let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                crate::look::draw_frame(&mut c, &theme, &text, &text, g, &state(true, false));
                c
            };
            assert_eq!(
                draw(&g).pixels,
                draw(&g.with_glass_top(50)).pixels,
                "{look:?}"
            );
            assert!(
                crate::look::client_glass(&theme, true).is_none(),
                "{look:?}"
            );
        }
    }

    #[test]
    fn a_tiny_blue_frame_draws_without_panicking() {
        let theme = theme_for(Interface::Blue);
        let g = FrameGeometry::layout(&theme.metrics, 1, 1, false, Chrome::WINDOW);
        draw(&g, &state(false, false));
    }

    #[test]
    #[ignore = "writes a preview PNG"]
    fn preview_blue_frames() {
        let theme = theme_for(Interface::Blue);
        let g = FrameGeometry::layout(&theme.metrics, 320, 120, false, Chrome::WINDOW);
        crate::raster::write_preview(&draw(&g, &state(true, false)), "blue-frame-active.png");
        crate::raster::write_preview(&draw(&g, &state(false, false)), "blue-frame-inactive.png");
    }

    #[test]
    fn a_frame_of_design_size_art_at_scale_2_doubles_the_scale_1_frame() {
        for interface in [Interface::Blue, Interface::Glass] {
            let base = theme_for(interface);
            let s = base.look.style().unwrap();
            let text = TextRenderer::load(&s.caption_text.font());
            let g = FrameGeometry::layout(&base.metrics, 300, 200, false, Chrome::WINDOW);
            for (active, maximized, glow) in [
                (true, false, Glow::default()),
                (false, false, Glow::default()),
                (true, true, Glow::lit(Some(CaptionButton::Close))),
                (true, false, Glow([0.0, 0.0, 0.5])),
                (false, false, Glow([0.5, 0.25, 0.0])),
            ] {
                let st = FrameState {
                    title: "",
                    glow,
                    ..state(active, maximized)
                };
                let draw = |theme: &Theme, g: &FrameGeometry| {
                    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                    draw_frame(&mut c, s, theme, &text, g, &st);
                    c
                };
                let one = draw(&base, &g);
                let two = draw(&base.scaled(2), &g.scaled(2));
                assert!(
                    crate::look::styled::fixture::doubles_at_scale_1(&one, &two),
                    "{interface:?} active {active}, maximized {maximized}, {glow:?}: \
                     the scale-2 frame is the scale-1 frame with every pixel doubled"
                );
            }
        }
    }
}
