//! Which look paints a surface: Classic's own draw code, or a [`Style`] for a skinned look.

use crate::art::glyph::Glyph;
use crate::classic;
use crate::frame::{ButtonSet, CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::{Style, frame as style_frame, taskbar as style_taskbar};
use crate::taskbar::{StateDamage, TaskbarLayout, TaskbarState};
use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig, Theme};
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(test)]
pub(crate) use crate::paint::classic_paint;
pub use crate::paint::{Backdrop, Look, Paint};

/// A floating panel drawn as glass under a look with glass.
pub mod panel;
pub mod sample;
/// Each common widget painted through a style's parts; `widgets` falls back to Classic when a style lacks the part.
pub(crate) mod styled;
pub mod widgets;

/// Dispatch a window frame to Classic or a skinned [`Style`]. `text` is Classic's renderer, unused by a styled look, which draws through `caption_text`. A `chrome.help` frame gets the ? box as [`draw_dialog_frame`] draws it, never lit.
pub fn draw_frame(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    caption_text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
) {
    match &theme.look {
        Look::Classic => classic::frame::draw_frame(canvas, theme, text, geom, state),
        Look::Styled(s) => {
            style_frame::draw_frame(canvas, s, theme, caption_text, geom, state);
            if geom.chrome.help {
                draw_styled_help(canvas, s, theme, geom, state, Interaction::Idle);
            }
        }
    }
}

/// Width of a sheet's fixed edges in `theme`'s look, logical (the plain glass inside may stretch from one pixel); None in a look that draws no sheet ([`draw_sheet`]).
pub fn sheet_edge(theme: &Theme) -> Option<i32> {
    let p = theme.look.style()?.part(crate::style::Part::WindowSheet)?;
    Some(p.sizing.left)
}

/// Draw a window a peek hides as a sheet over `r` (canvas pixels at the theme's scale), unfocused. Nothing in a look without one.
pub fn draw_sheet(canvas: &mut Canvas, theme: &Theme, r: Rect) {
    use crate::style::{Part, State};
    if let Some(s) = theme.look.style() {
        s.draw(
            canvas,
            r,
            Part::WindowSheet,
            State::Inactive,
            theme.scale.max(1),
        );
    }
}

/// The frame's plain glass where it runs down under a client's top
/// ([`FrameGeometry::glass_top`]), focused or not, as one premultiplied pixel; None in a look
/// whose frame never does.
pub fn client_glass(theme: &Theme, active: bool) -> Option<u32> {
    use crate::style::{Part, State};
    let p = theme.look.style()?.part(Part::MaxClientGlass)?;
    let f = p.frame(if active {
        State::Normal
    } else {
        State::Inactive
    });
    Some(p.atlas.get(f.x, f.y))
}

/// Draw the frame's glass under a client's top into `band` (canvas pixels at the theme's
/// scale): the client's rect from its top, as deep as the glass runs, a restored frame's sheen
/// carried down it from the caption's ends and in from its side borders down a client
/// `client_h` tall. Nothing in a look without [`client_glass`].
pub fn draw_client_glass(
    canvas: &mut Canvas,
    theme: &Theme,
    band: Rect,
    client_h: i32,
    maximized: bool,
) {
    let Some(s) = theme.look.style() else {
        return;
    };
    let state = match theme.active {
        true => crate::style::State::Normal,
        false => crate::style::State::Inactive,
    };
    let scale = theme.scale.max(1);
    style_frame::draw_client_glass(canvas, s, band, client_h, maximized, state, scale);
}

/// The caption title's ink, by whether the window is active, for labels drawn over clear
/// [`client_glass`]; None in a look without one.
pub fn ink_on_glass(theme: &Theme) -> Option<Color> {
    client_glass(theme, true)?;
    let t = &theme.look.style()?.caption_text;
    Some(if theme.active { t.color } else { t.inactive })
}

/// `ink` greyed for a disabled label or arrow over clear [`client_glass`]: 3/5 of the way from
/// the look's label glow, which reads over a bright backdrop where the dialog grey and its etch
/// fade into the glow; `ink` itself in a look without a glow.
pub fn greyed_on_glass(p: &Paint, ink: Color) -> Color {
    let Some(glow) = p.look.style().and_then(|s| s.label_glow) else {
        return ink;
    };
    let mix = |i: u8, g: u8| ((3 * u32::from(i) + 2 * u32::from(g) + 2) / 5) as u8;
    let g = glow.color;
    Color::rgb(mix(ink.r, g.r), mix(ink.g, g.g), mix(ink.b, g.b))
}

/// How much of a disabled icon shows over clear [`client_glass`] (num, den): more than a
/// dialog's half, as the glow under it follows its coverage.
pub const GREYED_ICON_ON_GLASS: (u32, u32) = (2, 3);

/// Lay the look's label glow (or halo) under whatever is drawn in `r` over clear
/// [`client_glass`], so labels and icons there read on any tint; nothing in a look without one.
pub fn glow_on_glass(canvas: &mut Canvas, theme: &Theme, r: Rect) {
    glow_under(canvas, &theme.paint(), r)
}

/// [`glow_on_glass`] for a widget painting with `p`.
pub fn glow_under(canvas: &mut Canvas, p: &Paint, r: Rect) {
    let Some(glow) = p.look.style().and_then(|s| s.label_glow) else {
        return;
    };
    let ink = canvas.copy_rect(r);
    let (w, h) = ink.device_size();
    let (w, h) = (w as usize, h as usize);
    let coverage: Vec<u8> = ink.pixels.iter().map(|&px| (px >> 24) as u8).collect();
    let s = canvas.scale();
    let alpha = (glow.alpha)(&crate::style::GlowMask {
        coverage: &coverage,
        width: w,
        height: h,
        rows: (0, h),
        active: p.active,
        scale: p.s * s,
    });
    canvas.clear_rect(r);
    canvas.blit_mask_over(r.x * s, r.y * s, w, h, &alpha, glow.color, r);
    canvas.blit_over(r.x, r.y, &ink);
}

/// Dispatch the taskbar to Classic or a skinned [`Style`]. `text` renders every run; `start_text` the Start label. The keyboard's item (`state.focus`) gets a dotted ring; the tray arrow lights instead.
pub fn draw_taskbar(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    start_text: &TextRenderer,
    layout: &TaskbarLayout,
    state: &TaskbarState,
) {
    draw_taskbar_on(canvas, theme, text, start_text, layout, state, false);
}

/// [`draw_taskbar`]; with `replace` a styled bar replaces what the canvas holds inside its clip,
/// its background copied from the one kept for its size.
#[allow(clippy::too_many_arguments)]
fn draw_taskbar_on(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    start_text: &TextRenderer,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    replace: bool,
) {
    let s = theme.scale.max(1);
    // The tray arrow shows focus by lighting as under the pointer, so it gets no ring.
    let ring = (state.focus)
        .filter(|&h| !(layout.tray_arrow && h == crate::taskbar::TaskbarHit::Chevron))
        .and_then(|h| crate::taskbarfocus::focus_ring(layout, h, s).map(|r| (h, r)));
    match &theme.look {
        Look::Classic => {
            classic::taskbar::draw_taskbar(canvas, theme, text, layout, state);
            if let Some((h, r)) = ring
                && h != crate::taskbar::TaskbarHit::Start
            {
                classic::widgets::draw_focus_ring(canvas, r, s, theme.colors.button_text);
            }
        }
        Look::Styled(st) => {
            style_taskbar::draw_taskbar_on(
                canvas, st, theme, text, start_text, layout, state, replace,
            );
            if let Some((_, r)) = ring {
                classic::widgets::draw_focus_ring(canvas, r, s, st.bar_text);
            }
        }
    }
}

/// Redraw `r` of `canvas`, a bar [`draw_taskbar`] drew on `layout`, for `state`: the whole bar,
/// or the [`taskbar_damage`] between the state it was drawn for and `state`.
pub fn redraw_taskbar_within(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    start_text: &TextRenderer,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    r: Rect,
) {
    canvas.with_clip(r, |c| {
        // A styled bar's background is copied over the whole of `r`.
        if theme.look.style().is_none() {
            c.clear_rect(r);
        }
        draw_taskbar_on(c, theme, text, start_text, layout, state, true);
    });
}

/// Px along the bar, times the scale, that damage reaches past a changed cell: a part's outset
/// lays art past its cell.
const DAMAGE_PAD: i32 = 4;

/// The rect of a bar redrawn on `layout` that [`TaskbarState::damage`]'s `d` reaches, across the
/// bar's whole thickness; None when nothing changed.
pub fn taskbar_damage(
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskbarLayout,
    d: StateDamage,
) -> Option<Rect> {
    let start = d.start.then(|| start_damage(theme, text, layout));
    let r = match (start, d.cells) {
        (Some(a), Some(b)) => a.union(&b),
        (a, b) => a.or(b)?,
    };
    let pad = DAMAGE_PAD * theme.scale.max(1);
    let across = if layout.edge.is_side() {
        Rect::new(0, r.y - pad, layout.width, r.height + 2 * pad)
    } else {
        Rect::new(r.x - pad, 0, r.width + 2 * pad, layout.height)
    };
    Some(across.intersect(&Rect::new(0, 0, layout.width, layout.height)))
}

/// The bar's rect that [`TaskbarState`]'s Start fields reach: Start's cell, the orb's rows and
/// Classic's focus ring.
fn start_damage(theme: &Theme, text: &TextRenderer, layout: &TaskbarLayout) -> Rect {
    let cell = layout.start_orb(true).unwrap_or(layout.start);
    match theme.look {
        Look::Classic => [false, true].into_iter().fold(cell, |r, pressed| {
            r.union(&classic::taskbar::start_ring(theme, text, layout, pressed))
        }),
        Look::Styled(_) => cell,
    }
}

/// Dispatch the orb's own buffer (`layout.start_orb`) to a skinned [`Style`]; `canvas` is sized to it. Classic's Start stays inside its bar.
pub fn draw_start_above(
    canvas: &mut Canvas,
    theme: &Theme,
    layout: &TaskbarLayout,
    state: &TaskbarState,
) {
    if let Look::Styled(s) = &theme.look {
        style_taskbar::draw_start_above(canvas, s, layout, state, theme.scale.max(1));
    }
}

/// The renderer for `style`'s caption font, loaded once per thread. A dialog and the Appearance sample get only the body renderer from their host, so they take this one.
pub fn caption_text(style: &Style) -> Rc<TextRenderer> {
    loaded(style.caption_text.font())
}

/// A renderer for `font`, loaded once per thread while the font stays the same.
fn loaded(font: FontConfig) -> Rc<TextRenderer> {
    thread_local! {
        static LOADED: RefCell<Option<(FontConfig, Rc<TextRenderer>)>> =
            const { RefCell::new(None) };
    }
    LOADED.with_borrow_mut(|slot| match slot {
        Some((f, t)) if *f == font => t.clone(),
        _ => {
            let t = Rc::new(TextRenderer::load(&font));
            *slot = Some((font, t.clone()));
            t
        }
    })
}

/// The narrowest client width at which `theme`'s caption shows `title` whole beside `chrome`'s
/// caption buttons, at scale 1.
pub fn caption_fit(theme: &Theme, title: &str, chrome: Chrome) -> i32 {
    if !chrome.decorated {
        return 0;
    }
    // The buttons sit at the caption's right, so the title's room grows one for one with the width.
    const PROBE: i32 = 1000;
    let g = FrameGeometry::layout(&theme.metrics, PROBE, 1, false, chrome);
    let (room, need) = match &theme.look {
        Look::Classic => {
            let room = if chrome.icon {
                classic::frame::title_rect(&g, 1)
            } else {
                let n = [CaptionButton::Minimize, CaptionButton::Close]
                    .into_iter()
                    .filter(|&b| chrome.buttons.has(b))
                    .count() as i32
                    + i32::from(chrome.help);
                classic::widgets::caption_title_rect(g.caption, n, 1)
            };
            let text = loaded(theme.fonts.clone());
            (room.width, text.measure(title, true, theme.fonts.size_px))
        }
        Look::Styled(s) => {
            let (left, right) = style_frame::title_span(s, &g, 1);
            let spec = &s.caption_text;
            let need = caption_text(s).measure(title, spec.bold, spec.size_px);
            (right - left, need)
        }
    };
    PROBE - room + need
}

/// A dialog's frame over a client of button face: border, caption, the minimize and close boxes `geom.chrome` has, and the ? box left of close when `help` is `Some`. Classic's dialog caption has no icon and its ? box ignores the pointer; a styled look draws its usual frame and help button part, else Classic's ? box.
pub fn draw_dialog_frame(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
    help: Option<Interaction>,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let help_box = help.map(|_| (geom.help_box(), Glyph::Help));
    match &theme.look {
        Look::Classic => {
            let all = Rect::new(geom.origin.0, geom.origin.1, geom.total.0, geom.total.1);
            c.fill_rect(all, cs.button_face);
            c.edge_t(all, EdgeStyle::WindowRaised, cs, s);
            let shown = [
                (CaptionButton::Minimize, Glyph::Minimize),
                (CaptionButton::Close, Glyph::Close),
            ]
            .into_iter()
            .filter(|&(b, _)| geom.chrome.buttons.has(b));
            let buttons: Vec<(Rect, Glyph, CaptionButton)> =
                shown.map(|(b, g)| (geom.button_rect(b), g, b)).collect();
            let drawn: Vec<(Rect, Glyph)> = (buttons.iter().map(|&(r, g, _)| (r, g)))
                .chain(help_box)
                .collect();
            classic::widgets::draw_caption(
                c,
                text,
                cs,
                geom.caption,
                state.title,
                state.active,
                &drawn,
                theme.fonts.size_px,
                s,
            );
            for (r, g, _) in buttons.iter().filter(|b| state.pressed == Some(b.2)) {
                classic::widgets::draw_caption_button(c, cs, *r, *g, true, s);
            }
        }
        Look::Styled(style) => {
            style_frame::draw_frame(c, style, theme, &caption_text(style), geom, state);
            c.fill_rect(geom.client, cs.button_face);
            if let Some(ptr) = help {
                draw_styled_help(c, style, theme, geom, state, ptr);
            }
        }
    }
}

/// A styled caption's ? box, lit for `ptr`: the style's help part (across the close box when the style joins them), or Classic's ? box when it has none.
fn draw_styled_help(
    c: &mut Canvas,
    style: &Style,
    theme: &Theme,
    geom: &FrameGeometry,
    state: &FrameState,
    ptr: Interaction,
) {
    let s = theme.scale.max(1);
    let close = CaptionButton::Close;
    let held = state.pressed == Some(close);
    // Only a dialog's lone close box joins the ? box.
    let close = (geom.chrome.buttons == ButtonSet::Close).then_some((
        geom.buttons[2],
        held,
        state.glow.of(close),
    ));
    let r = geom.help_box();
    let grown = style.at_button_side(crate::frame::button_side(&theme.metrics) / s);
    let style = grown.as_deref().unwrap_or(style);
    if !styled::help::draw_help_button(style, c, r, state.active, ptr, close, s) {
        classic::widgets::draw_caption_button(c, &theme.colors, r, Glyph::Help, false, s);
    }
}

/// The Themes or Appearance sample in `theme`'s look at scale `s`. Classic draws it whole; a styled look draws each window's frame, scrollbar and message box button through its style, and the menu bar and text stay Classic's.
pub fn draw_sample(
    themes: bool,
    theme: &Theme,
    s: i32,
    text: &TextRenderer,
    icon: Option<&Canvas>,
) -> Canvas {
    draw_sample_at(themes, theme, s, text, icon, sample::APPEARANCE_SIZE)
}

/// [`draw_sample`] with the Appearance sample at `size` (logical px); the Themes sample keeps its own size.
pub fn draw_sample_at(
    themes: bool,
    theme: &Theme,
    s: i32,
    text: &TextRenderer,
    icon: Option<&Canvas>,
    size: (i32, i32),
) -> Canvas {
    use sample::Frames;
    let s = s.max(1);
    let scaled = theme.scaled(s);
    let draw = |frames: Option<Frames>| {
        if themes {
            sample::draw_themes(&scaled, text, icon, frames)
        } else {
            sample::draw_appearance(&scaled, text, frames, size)
        }
    };
    let Some(style) = theme.look.style() else {
        return draw(None);
    };
    let caption_text = caption_text(style);
    let paint = |c: &mut Canvas, outer: Rect, title: &str, active: bool, chrome: Chrome| {
        let m = &theme.metrics;
        let (w, h) = FrameGeometry::client_size_for_total(
            m,
            outer.width / s,
            outer.height / s,
            false,
            chrome,
        );
        let g = FrameGeometry::layout(m, w, h, false, chrome).scaled(s);
        let state = FrameState {
            title,
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        // Drawn apart and laid over, so the rounded corners show the sample's desktop.
        let mut pic = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        style_frame::draw_frame(&mut pic, style, &scaled, &caption_text, &g, &state);
        if let Some(glass) = style.glass {
            glass.lift[usize::from(!active)].under(c, outer.x, outer.y, &pic);
        }
        c.blit_over(outer.x, outer.y, &pic);
        let client = Rect::new(
            outer.x + g.client.x,
            outer.y + g.client.y,
            g.client.width,
            g.client.height,
        );
        c.fill_rect(client, scaled.colors.button_face);
        client
    };
    draw(Some(&paint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::taskbar::TaskbarShape;
    use crate::theme::Theme;

    /// `title`'s caption fit in `i` leaves it exactly its room, `dialog` and `prompt` px wide in
    /// the caption's own face.
    fn fits_its_room(i: Interface, dialog: i32, prompt: i32) {
        let title = "User Accounts";
        let theme = theme_for(i);
        let fit = caption_fit(&theme, title, Chrome::DIALOG);
        let g = FrameGeometry::layout(&theme.metrics, fit, 1, false, Chrome::DIALOG);
        let room = match &theme.look {
            Look::Classic => classic::frame::title_rect(&g, 1).width,
            Look::Styled(s) => {
                let (left, right) = style_frame::title_span(s, &g, 1);
                right - left
            }
        };
        let (need, face) = match &theme.look {
            Look::Classic => {
                let t = loaded(theme.fonts.clone());
                (t.measure(title, true, theme.fonts.size_px), true)
            }
            Look::Styled(s) => {
                let font = s.caption_text.font();
                let t = caption_text(s);
                let need = t.measure(title, s.caption_text.bold, s.caption_text.size_px);
                (need, t.has_face(&font.family[0]))
            }
        };
        assert_eq!(room, need, "{i:?}: the title fills its room at the fit");
        if !face {
            return;
        }
        assert_eq!(fit, dialog, "{i:?} dialog caption");
        assert_eq!(
            caption_fit(&theme, title, Chrome::PROMPT),
            prompt,
            "{i:?} prompt caption"
        );
    }

    #[test]
    fn caption_fit_leaves_a_classic_title_exactly_its_room() {
        crate::pins::fonts();
        fits_its_room(Interface::Classic, 159, 103);
        let undecorated = Chrome {
            decorated: false,
            ..Chrome::DIALOG
        };
        assert_eq!(
            caption_fit(&Theme::default(), "User Accounts", undecorated),
            0
        );
    }

    /// Their caption faces are system fonts; the widths are checked where those are installed.
    #[test]
    fn caption_fit_leaves_a_blue_or_glass_title_exactly_its_room() {
        fits_its_room(Interface::Blue, 179, 179);
        fits_its_room(Interface::Glass, 201, 201);
    }

    /// Blue draws its controls hot and held; Classic and Glass draw them the same whatever the pointer does, so hover asks them for no redraw.
    #[test]
    fn only_a_look_with_lit_control_art_lights_the_pointer() {
        assert!(theme_for(Interface::Blue).look.lights_pointer());
        assert!(!theme_for(Interface::Classic).look.lights_pointer());
        assert!(theme_for(Interface::Glass).look.lights_pointer());
        let bare = crate::look::styled::fixture::one_part_look(crate::style::Part::Caption);
        assert!(!bare.lights_pointer(), "a style with no lit control art");
    }

    #[test]
    fn ink_on_glass_glows_in_the_titles_glow_and_keeps_its_own_pixels() {
        let ink = |c: &mut Canvas| {
            c.fill_rect(Rect::new(20, 8, 10, 4), crate::theme::Color::rgb(0, 0, 0))
        };
        let band = Rect::new(0, 0, 60, 20);
        let mut c = Canvas::new(60, 20);
        ink(&mut c);
        glow_on_glass(&mut c, &theme_for(Interface::Glass), band);
        assert_eq!(c.get(25, 10), 0xFF00_0000, "the ink stays");
        let halo = c.get(25, 6);
        assert!(halo >> 24 > 0, "a glow just above it: {halo:08X}");
        assert_eq!(halo & 0xFF, halo >> 24, "white, premultiplied");
        assert_eq!(c.get(55, 18), 0, "clear far from it");

        let mut classic = Canvas::new(60, 20);
        ink(&mut classic);
        let before = classic.pixels.clone();
        glow_on_glass(&mut classic, &theme_for(Interface::Classic), band);
        assert_eq!(classic.pixels, before, "no glow in a look without one");
    }

    #[test]
    fn only_glass_draws_a_hidden_window_as_a_sheet() {
        assert_eq!(sheet_edge(&theme_for(Interface::Glass)), Some(8));
        assert_eq!(sheet_edge(&theme_for(Interface::Classic)), None);
        assert_eq!(sheet_edge(&theme_for(Interface::Blue)), None);
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(1, 1);
        draw_sheet(&mut c, &theme, Rect::new(-50, -40, 100, 80));
        let style = theme.look.style().unwrap();
        let p = style.part(crate::style::Part::WindowSheet).unwrap();
        let f = p.frame(crate::style::State::Inactive);
        assert_eq!(
            c.get(0, 0),
            p.atlas.get(f.x + 8, f.y + 8),
            "the middle is the unfocused sheet's plain glass"
        );
    }

    #[test]
    fn a_theme_paints_at_scale_one_or_more() {
        let theme = Theme {
            scale: 0,
            ..Theme::default()
        };
        assert_eq!(theme.paint().s, 1, "a theme with scale 0 paints at scale 1");
    }

    #[test]
    fn a_theme_paints_on_the_face_until_given_a_page() {
        let theme = theme_for(Interface::Blue);
        assert_eq!(theme.paint().back, Backdrop::Face);
        let page = Rect::new(4, 5, 60, 70);
        assert_eq!(theme.paint().on_page(page).back, Backdrop::Page(page));
    }

    fn page_and_fills(theme: &Theme, page: Rect, fills: &[Rect]) -> (Canvas, Canvas) {
        let p = theme.paint();
        let mut want = Canvas::new(100, 120);
        want.fill_rect(want.bounds(), p.cs.button_face);
        widgets::draw_page(&mut want, &p, page);
        let mut got = Canvas::new(100, 120);
        got.fill_rect(got.bounds(), crate::theme::Color::rgb(0, 0, 0));
        for &r in fills {
            p.on_page(page).fill_back(&mut got, r);
        }
        (want, got)
    }

    #[test]
    fn a_blue_backdrop_inside_a_page_is_the_page_and_face_outside_it() {
        let theme = theme_for(Interface::Blue);
        let page = Rect::new(10, 10, 80, 100);
        let fills = [
            Rect::new(20, 9, 30, 4),
            Rect::new(40, 30, 13, 1),
            Rect::new(12, 105, 70, 5),
            Rect::new(80, 50, 20, 6),
        ];
        let (want, got) = page_and_fills(&theme, page, &fills);
        let face = theme.colors.button_face.argb();
        for r in fills {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    assert_eq!(got.get(x, y), want.get(x, y), "({x}, {y}) shows the page");
                }
            }
        }
        assert_eq!(got.get(95, 52), face, "right of the page is the face");
        assert_ne!(
            got.get(40, 30),
            face,
            "inside the page's fade the backdrop is not the face"
        );
    }

    #[test]
    fn a_classic_backdrop_is_the_face() {
        let theme = theme_for(Interface::Classic);
        let r = Rect::new(20, 30, 40, 3);
        let (_, got) = page_and_fills(&theme, Rect::new(10, 10, 80, 100), &[r]);
        let mut want = Canvas::new(100, 120);
        want.fill_rect(want.bounds(), crate::theme::Color::rgb(0, 0, 0));
        want.fill_rect(r, theme.colors.button_face);
        assert!(got == want, "the face fill alone");
    }

    #[test]
    fn a_glass_backdrop_inside_a_page_is_white_and_face_outside_it() {
        let theme = theme_for(Interface::Glass);
        let fills = [Rect::new(20, 30, 40, 3), Rect::new(80, 50, 20, 6)];
        let (_, got) = page_and_fills(&theme, Rect::new(10, 10, 80, 100), &fills);
        assert_eq!(got.get(40, 31), 0xFFFF_FFFF, "inside the page it is FFFFFF");
        assert_eq!(
            got.get(95, 52),
            theme.colors.button_face.argb(),
            "right of the page it is the face"
        );
    }

    #[test]
    fn looks_compare_by_style_name_and_scaling_keeps_the_look() {
        assert_eq!(Look::default(), Look::Classic);
        let t = Theme {
            look: Look::Classic,
            ..Theme::default()
        };
        assert_eq!(t.scaled(2).look, Look::Classic);
    }

    #[test]
    fn taskbar_shape_follows_the_look() {
        assert_eq!(Theme::default().taskbar_shape(), TaskbarShape::CLASSIC);
        let blue = theme_for(Interface::Blue);
        assert_eq!(blue.taskbar_shape(), crate::blue::style().taskbar);
    }

    #[test]
    fn a_classic_sample_is_classics_own() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let scaled = theme.scaled(2);
        assert_eq!(
            draw_sample(false, &theme, 2, &text, None),
            sample::draw_appearance(&scaled, &text, None, sample::APPEARANCE_SIZE)
        );
        assert_eq!(
            draw_sample(true, &theme, 1, &text, None),
            sample::draw_themes(&theme, &text, None, None)
        );
    }

    #[test]
    fn a_blue_sample_frames_its_windows_through_the_style() {
        use crate::style::{Part, State};
        let theme = theme_for(Interface::Blue);
        let style = theme.look.style().unwrap();
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_sample(false, &theme, 1, &text, None);
        for (x, y, w, state) in [(16, 8, 305, State::Inactive), (20, 31, 323, State::Normal)] {
            let mut want = Canvas::new(w as u32, 30);
            style.draw(&mut want, Rect::new(0, 0, w, 30), Part::Caption, state, 1);
            assert_eq!(c.get(x + 12, y + 2), want.get(12, 2), "{state:?} caption");
        }
        assert_eq!(
            c.get(16, 8),
            theme.colors.background.argb(),
            "a rounded corner"
        );
    }

    #[test]
    fn a_glass_sample_lifts_its_desktop_under_each_frame() {
        use crate::backdrop::Lift;
        use crate::interfaces::sample_theme;
        let text = TextRenderer::load(&FontConfig::default());
        let g = crate::glass::Settings::default();
        // The inactive window's left border and the focused one's right border, both over the desktop.
        let (inactive, active) = ((18, 60), (339, 60));
        for (desk, want) in [
            (Color::rgb(232, 232, 236), [0xFFD5_E2F0_u32, 0xFFC2_D6ED]),
            (Color::rgb(24, 30, 44), [0xFF46_4F5C_u32, 0xFF6F_7780]),
        ] {
            let theme = sample_theme(Interface::Glass, "", &g, Some(desk));
            let mut flat = theme.clone();
            let mut style = theme.look.style().unwrap().clone();
            let spec = style.glass.as_mut().unwrap();
            let lift = spec.lift;
            spec.lift = [Lift::NONE; 2];
            flat.look = Look::Styled(std::sync::Arc::new(style));
            let c = draw_sample(false, &theme, 1, &text, None);
            let c0 = draw_sample(false, &flat, 1, &text, None);
            assert_eq!(
                c.get(16, 8),
                desk.argb(),
                "{desk:?}: the desktop past a corner"
            );
            for (i, (x, y)) in [inactive, active].into_iter().enumerate() {
                let (got, before) = (c.get(x, y), c0.get(x, y));
                assert_ne!(got, before, "{desk:?}: ({x}, {y}) lifted");
                assert_eq!(got, want[i], "{desk:?}: ({x}, {y})");
                let mut d = Canvas::new(1, 1);
                d.put(0, 0, desk);
                let mut art = Canvas::new(1, 1);
                art.put_raw(0, 0, 1 << 24);
                lift[1 - i].under(&mut d, 0, 0, &art);
                let ch = |px: u32, k: u32| ((px >> k) & 0xFF) as i32;
                let lifted = d.get(0, 0);
                for k in [16, 8, 0] {
                    let (moved, by) = (
                        ch(got, k) - ch(before, k),
                        ch(lifted, k) - ch(desk.argb(), k),
                    );
                    assert!(
                        (moved * by >= 0 || moved.abs() <= 1) && moved.abs() <= by.abs() + 1,
                        "{desk:?}: ({x}, {y}) moves {moved} as its desktop does, by up to {by}"
                    );
                }
            }
        }
    }

    fn empty_taskbar_state() -> TaskbarState {
        TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            meter: None,
            clock: "00:00".into(),
        }
    }

    #[test]
    fn redrawing_the_damage_gives_the_bar_a_full_draw_gives() {
        use crate::taskbar::{Attention, BarSpan, ClockText, Edge, TaskEntry, TaskProgress};
        use crate::taskbar::{TaskRow, TaskbarCounts, TaskbarHit};
        use std::sync::Arc;
        let text = TextRenderer::load(&FontConfig::default());
        let counts = TaskbarCounts {
            quick: 2,
            pager: 2,
            tasks: 3,
            tray: 2,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let mut tray = Canvas::new(16, 16);
        tray.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(200, 40, 40));
        let tray = Arc::new(tray);
        let task = |t: &str| TaskEntry {
            title: t.into(),
            active: t == "Files",
            attention: Attention::None,
            icon: None,
            glow: None,
        };
        let base = || {
            let mut st = empty_taskbar_state();
            st.quick_icons = vec![None, None];
            st.pager = vec![Default::default(); 2];
            st.pager[0].current = true;
            st.tasks = ["Terminal", "Files", "Notes"].map(task).into();
            st.tray_icons = vec![None, Some(tray.clone())];
            st
        };
        type Change = fn(&mut TaskbarState);
        let change: Vec<(&str, Change)> = vec![
            ("Start held", |st| st.start_pressed = true),
            ("Start lit", |st| st.start_hot = true),
            ("Start focused", |st| {
                (st.start_pressed, st.start_focus) = (true, true)
            }),
            ("first lit", |st| st.task_hot = Some(0)),
            ("middle lit", |st| st.task_hot = Some(1)),
            ("last lit", |st| st.task_hot = Some(2)),
            ("spot moved", |st| {
                (st.task_hot, st.task_hot_x) = (Some(1), Some(30))
            }),
            ("held", |st| {
                (st.task_hot, st.task_pressed) = (Some(2), Some(2))
            }),
            ("focus moved on", |st| {
                st.tasks[1].active = false;
                st.tasks[2].active = true;
            }),
            ("flashing", |st| st.tasks[0].attention = Attention::FlashOn),
            ("retitled", |st| {
                st.tasks[1].title = "Files and folders".into()
            }),
            ("glowing", |st| {
                st.task_hot = Some(0);
                st.tasks[0].glow = Some(Color::rgb(40, 200, 90));
            }),
            ("progress", |st| {
                let p = TaskProgress {
                    fraction: 0.5,
                    paused: false,
                };
                st.task_progress = vec![None, Some(p), None];
            }),
            ("badge", |st| st.task_badge = vec![Some(3), None, None]),
            ("Quick Launch lit", |st| st.quick_hot = Some(1)),
            ("Quick Launch held", |st| st.quick_pressed = Some(0)),
            ("pager lit", |st| st.pager_hot = Some(1)),
            ("desktop switched", |st| {
                st.pager[0].current = false;
                st.pager[1].current = true;
            }),
            ("Show desktop lit", |st| st.show_desktop_hot = true),
            ("tray cell gone", |st| st.tray_icons[1] = None),
            ("tray cell drawn", |st| {
                st.tray_icons[0] = Some(Arc::new(Canvas::new(16, 16)))
            }),
            ("clock", |st| st.clock = "00:01".into()),
            ("keyboard on a button", |st| {
                st.focus = Some(TaskbarHit::Task(1))
            }),
            ("clock and spot", |st| {
                st.clock = "00:02".into();
                (st.task_hot, st.task_hot_x) = (Some(0), Some(12));
            }),
        ];
        let many: Vec<&str> = vec!["Notes"; 30];
        let relaid: Vec<(&str, Vec<&str>, Vec<u8>)> = vec![
            (
                "added first",
                vec!["New", "Terminal", "Files", "Notes"],
                vec![1; 4],
            ),
            (
                "added in the middle",
                vec!["Terminal", "New", "Files", "Notes"],
                vec![1; 4],
            ),
            (
                "added last",
                vec!["Terminal", "Files", "Notes", "New"],
                vec![1; 4],
            ),
            ("closed first", vec!["Files", "Notes"], vec![1; 2]),
            (
                "closed in the middle",
                vec!["Terminal", "Notes"],
                vec![1; 2],
            ),
            ("closed last", vec!["Terminal", "Files"], vec![1; 2]),
            (
                "group grown",
                vec!["Terminal", "Files (2)", "Notes"],
                vec![1, 2, 1],
            ),
            ("more than fit", many, vec![1; 30]),
        ];
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme0 = theme_for(interface);
            let start_text = match theme0.look.style() {
                Some(st) => TextRenderer::load(&st.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let shape = theme0.taskbar_shape();
            for edge in [Edge::Bottom, Edge::Top, Edge::Left] {
                let span = BarSpan {
                    edge,
                    length: if edge.is_side() { 700 } else { 1000 },
                    thickness: if edge.is_side() {
                        120
                    } else {
                        theme0.metrics.taskbar_height
                    },
                };
                let clock = ClockText { time: 40, date: 60 };
                let l = TaskbarLayout::compute_on(
                    &theme0.metrics,
                    &shape,
                    span,
                    true,
                    counts,
                    clock,
                    Default::default(),
                );
                let relay = |stacks: &[u8]| {
                    let mut l = l.clone();
                    l.lay_tasks(&theme0.metrics, &shape, stacks, TaskRow::Shared);
                    l
                };
                assert_eq!(
                    relay(&[1; 30]).more.is_some(),
                    edge.is_side(),
                    "{interface:?} {edge:?}: a column out of room ends in a » cell"
                );
                for s in [1, 2] {
                    let theme = theme0.scaled(s);
                    let layout = l.scaled(s);
                    let whole = Rect::new(0, 0, layout.width, layout.height);
                    let draw = |st: &TaskbarState, l: &TaskbarLayout| {
                        let mut c = Canvas::new(l.width as u32, l.height as u32);
                        draw_taskbar(&mut c, &theme, &text, &start_text, l, st);
                        c
                    };
                    let redraw = |c: &mut Canvas, st: &TaskbarState, l: &TaskbarLayout, r: Rect| {
                        redraw_taskbar_within(c, &theme, &text, &start_text, l, st, r);
                    };
                    let damage =
                        |a: (&TaskbarState, &TaskbarLayout), b: (&TaskbarState, &TaskbarLayout)| {
                            taskbar_damage(&theme, &text, b.1, a.0.damage(b.0, a.1, b.1))
                        };
                    let what = format!("{interface:?} {edge:?} scale {s}");
                    let from = base();
                    let at_base = draw(&from, &layout);
                    let mut fresh = Canvas::new(layout.width as u32, layout.height as u32);
                    redraw(&mut fresh, &from, &layout, whole);
                    assert!(fresh.pixels == at_base.pixels, "{what}: a whole redraw");
                    assert_eq!(
                        damage((&from, &layout), (&base(), &layout)),
                        None,
                        "{what}: the same state"
                    );
                    let changed = change.iter().map(|(name, f)| {
                        let mut to = base();
                        f(&mut to);
                        (name.to_string(), to, layout.clone())
                    });
                    let relaid = relaid.iter().map(|(name, titles, stacks)| {
                        let mut to = base();
                        to.tasks = titles.iter().map(|t| task(t)).collect();
                        (name.to_string(), to, relay(stacks).scaled(s))
                    });
                    for (name, to, to_layout) in changed.chain(relaid) {
                        let want = draw(&to, &to_layout);
                        let mut over = at_base.clone();
                        redraw(&mut over, &to, &to_layout, whole);
                        assert!(
                            over.pixels == want.pixels,
                            "{what}: {name} over another bar"
                        );
                        for (a, b, pic, goal, way) in [
                            (
                                (&from, &layout),
                                (&to, &to_layout),
                                &at_base,
                                &want,
                                "there",
                            ),
                            ((&to, &to_layout), (&from, &layout), &want, &at_base, "back"),
                        ] {
                            let r = damage(a, b)
                                .unwrap_or_else(|| panic!("{what}: {name} {way}: no damage"));
                            let mut c = pic.clone();
                            redraw(&mut c, b.0, b.1, r);
                            assert!(c.pixels == goal.pixels, "{what}: {name} {way} in {r:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_change_damages_only_its_cells_and_their_neighbours() {
        use crate::taskbar::TaskbarHit;
        use crate::taskbar::{BarSpan, ClockText, Edge, TaskEntry, TaskRow, TaskbarCounts};
        use std::sync::Arc;
        let counts = TaskbarCounts {
            tasks: 4,
            tray: 1,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge: Edge::Bottom,
            length: 1400,
            thickness: 30,
        };
        let lay = |span: BarSpan, counts: TaskbarCounts| {
            TaskbarLayout::compute_on(
                &Default::default(),
                &TaskbarShape::CLASSIC,
                span,
                true,
                counts,
                ClockText { time: 40, date: 60 },
                Default::default(),
            )
        };
        let l = lay(span, counts);
        let relay = |stacks: &[u8]| {
            let mut r = l.clone();
            r.lay_tasks(
                &Default::default(),
                &TaskbarShape::CLASSIC,
                stacks,
                TaskRow::Shared,
            );
            r
        };
        let base = || {
            let mut st = empty_taskbar_state();
            st.tasks = (0..4)
                .map(|i| TaskEntry {
                    title: format!("{i}"),
                    active: false,
                    attention: Default::default(),
                    icon: None,
                    glow: None,
                })
                .collect();
            st.tray_icons = vec![None];
            st
        };
        let damage = |f: &dyn Fn(&mut TaskbarState)| {
            let mut st = base();
            f(&mut st);
            base().damage(&st, &l, &l)
        };
        let cells = |a: usize, b: usize| Some(l.tasks[a].union(&l.tasks[b]));
        assert_eq!(damage(&|_| {}), StateDamage::default(), "nothing changed");
        let lit = damage(&|st| st.task_hot = Some(2));
        assert_eq!(lit.cells, cells(1, 3), "a lit button and its neighbours");
        assert!(!lit.start, "Start stays");
        let first = damage(&|st| st.task_hot = Some(0));
        assert_eq!(
            first.cells,
            cells(0, 1),
            "the first button has no neighbour before"
        );
        let clock = damage(&|st| st.clock = "00:01".into());
        assert_eq!(clock.cells, Some(l.clock), "the clock");
        let start = damage(&|st| st.start_hot = true);
        assert_eq!((start.start, start.cells), (true, None), "Start alone");
        let icon = Some(Arc::new(Canvas::new(1, 1)));
        let (mut a, mut b) = (base(), base());
        a.start_icon = icon.clone();
        b.start_icon = Some(Arc::new(Canvas::new(1, 1)));
        assert!(
            a.damage(&b, &l, &l).start,
            "another icon, even an equal one"
        );
        b.start_icon = icon;
        assert_eq!(
            a.damage(&b, &l, &l),
            StateDamage::default(),
            "the same icon"
        );
        let focus = damage(&|st| st.focus = Some(TaskbarHit::Clock));
        let whole = Rect::new(0, 0, l.width, l.height);
        assert_eq!(
            focus.cells,
            Some(whole),
            "the keyboard's ring redraws the bar"
        );

        let relaid = |titles: &[usize], stacks: &[u8]| {
            let mut st = base();
            st.tasks = titles
                .iter()
                .map(|&i| base().tasks.swap_remove(i))
                .collect();
            let to = relay(stacks);
            (base().damage(&st, &l, &to), to)
        };
        let (added, five) = relaid(&[0, 1, 2, 3, 3], &[1; 5]);
        assert_eq!(five.tasks[..4], l.tasks[..], "four buttons stay put");
        assert_eq!(
            added,
            StateDamage {
                start: false,
                cells: Some(l.tasks[3].union(&five.tasks[4])),
            },
            "a button added last and its neighbour"
        );
        let (closed, _) = relaid(&[1, 2, 3], &[1; 3]);
        assert_eq!(
            closed.cells,
            cells(0, 3),
            "a button closed first moves every button past it"
        );
        let (closed, _) = relaid(&[0, 1, 2], &[1; 3]);
        assert_eq!(
            closed.cells,
            cells(2, 3),
            "a button closed last and its neighbour"
        );
        let (grown, _) = relaid(&[0, 1, 2, 3], &[1, 2, 1, 1]);
        assert_eq!(grown.cells, cells(0, 2), "a group grown and its neighbours");
        let shrunk = relaid(&[0; 12], &[1; 12]).0.cells.expect("damage");
        assert!(
            shrunk.x <= l.tasks[0].x && shrunk.right() < l.tray[0].x,
            "buttons shrunk to fit redraw the task row alone: {shrunk:?}"
        );
        let wider = BarSpan {
            length: 1500,
            ..span
        };
        let quick = TaskbarCounts { quick: 1, ..counts };
        for (other, why) in [
            (lay(wider, counts), "a longer bar"),
            (lay(span, quick), "a Quick Launch button added"),
        ] {
            assert_eq!(
                base().damage(&base(), &l, &other),
                StateDamage {
                    start: true,
                    cells: Some(Rect::new(0, 0, other.width, other.height)),
                },
                "{why} redraws the bar and Start"
            );
        }
    }

    #[test]
    fn a_focused_tray_arrow_draws_lit_without_a_ring() {
        use crate::taskbar::TaskbarCounts;
        let text = TextRenderer::load(&FontConfig::default());
        let counts = TaskbarCounts {
            tray: 1,
            hidden: 2,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for interface in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(interface);
            let l = TaskbarLayout::compute(
                &theme.metrics,
                &theme.taskbar_shape(),
                800,
                true,
                counts,
                40,
            );
            assert!(l.tray_arrow, "{interface:?}");
            let draw = |st: &TaskbarState| {
                let mut c = Canvas::new(800, l.height as u32);
                draw_taskbar(&mut c, &theme, &text, &text, &l, st);
                c
            };
            let mut focused = empty_taskbar_state();
            focused.focus = Some(crate::taskbar::TaskbarHit::Chevron);
            let mut lit = empty_taskbar_state();
            lit.chevron.hot = true;
            assert!(
                draw(&focused).pixels == draw(&lit).pixels,
                "{interface:?}: focus draws the arrow lit, with no ring"
            );
        }
    }

    #[test]
    fn draw_taskbar_dispatches_by_look() {
        use crate::taskbar::TaskbarCounts;
        let text = TextRenderer::load(&FontConfig::default());
        let counts = TaskbarCounts::default();
        let state = empty_taskbar_state();

        let classic = Theme::default();
        let cl_layout = TaskbarLayout::compute(
            &classic.metrics,
            &crate::taskbar::TaskbarShape::CLASSIC,
            800,
            true,
            counts,
            40,
        );
        let mut c1 = Canvas::new(800, 30);
        draw_taskbar(&mut c1, &classic, &text, &text, &cl_layout, &state);
        let mut want1 = Canvas::new(800, 30);
        classic::taskbar::draw_taskbar(&mut want1, &classic, &text, &cl_layout, &state);
        assert_eq!(c1.get(400, 15), want1.get(400, 15));

        let blue = theme_for(Interface::Blue);
        let style = blue.look.style().unwrap();
        let start_text = TextRenderer::load(&style.start_text.font());
        let bl_layout =
            TaskbarLayout::compute(&blue.metrics, &style.taskbar, 800, true, counts, 40);
        let mut c2 = Canvas::new(800, 30);
        draw_taskbar(&mut c2, &blue, &text, &start_text, &bl_layout, &state);
        let mut want2 = Canvas::new(800, 30);
        style_taskbar::draw_taskbar(
            &mut want2,
            style,
            &blue,
            &text,
            &start_text,
            &bl_layout,
            &state,
        );
        assert_eq!(c2.get(400, 15), want2.get(400, 15));
    }

    #[test]
    fn the_start_menu_stands_on_a_glass_bar_and_reaches_4_px_into_the_others() {
        use crate::taskbar::{Edge, StartMenuDock};
        let glass = theme_for(Interface::Glass).taskbar_shape();
        let small = crate::glass::small_bar_style(&Default::default()).taskbar;
        for (shape, bar_top, what) in [(glass, 1040, "Glass"), (small, 1050, "small Glass")] {
            let dock = shape.start_menu;
            let top = dock.menu_at(Edge::Bottom, (bar_top, 1080), 500);
            assert_eq!(
                top + 500,
                bar_top,
                "{what}: the menu's foot is the bar's top"
            );
            assert_eq!(
                dock.shadow_floor(Edge::Bottom),
                Some(0),
                "{what}: the menu casts no shadow"
            );
            assert_eq!(
                dock.submenu_shadow_floor(Edge::Bottom, bar_top, top + 100),
                Some(400),
                "{what}: a submenu's shadow stops at the bar's top"
            );
        }
        for (shape, what) in [
            (theme_for(Interface::Classic).taskbar_shape(), "Classic"),
            (theme_for(Interface::Blue).taskbar_shape(), "Blue"),
        ] {
            let dock = shape.start_menu;
            assert_eq!(dock, StartMenuDock::Into(4), "{what}");
            let top = dock.menu_at(Edge::Bottom, (1050, 1080), 500);
            assert_eq!(
                top + 500,
                1054,
                "{what}: the menu's foot is 4 px into the bar"
            );
            assert_eq!(
                dock.shadow_floor(Edge::Bottom),
                None,
                "{what}: the shadow falls on the bar"
            );
            assert_eq!(
                dock.submenu_shadow_floor(Edge::Bottom, 1050, top + 100),
                None,
                "{what}: a submenu's shadow falls on the bar"
            );
        }
    }

    #[test]
    fn under_a_top_bar_the_start_menu_hangs_from_its_foot_with_all_its_shadow() {
        use crate::taskbar::{Edge, StartMenuDock};
        let glass = theme_for(Interface::Glass).taskbar_shape().start_menu;
        assert_eq!(
            glass.menu_at(Edge::Top, (0, 40), 500),
            40,
            "Glass: the menu's top is the bar's foot"
        );
        for (i, foot, what) in [
            (Interface::Classic, 28, "Classic"),
            (Interface::Blue, 30, "Blue"),
        ] {
            let dock = theme_for(i).taskbar_shape().start_menu;
            assert_eq!(
                dock.menu_at(Edge::Top, (0, foot), 500),
                foot - 4,
                "{what}: its top 4 px up into the bar"
            );
        }
        for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
            assert_eq!(
                dock.shadow_floor(Edge::Top),
                None,
                "{dock:?}: its whole shadow shows"
            );
            assert_eq!(
                dock.submenu_shadow_floor(Edge::Top, 0, 140),
                None,
                "{dock:?}: a submenu's too"
            );
        }
    }

    #[test]
    fn a_theme_paints_with_its_colours_look_and_scale() {
        let mut theme = theme_for(Interface::Blue);
        theme.scale = 2;
        let p = theme.paint();
        assert!(
            std::ptr::eq(p.cs, &theme.colors),
            "Paint borrows the theme's colours"
        );
        assert!(
            *p.look == theme.look,
            "Paint carries the theme's look (Blue)"
        );
        assert_eq!(p.s, 2, "Paint carries the theme's scale 2");
    }

    #[test]
    fn a_default_theme_paints_classic() {
        let theme = Theme::default();
        assert!(
            *theme.paint().look == Look::Classic,
            "the default theme paints Classic"
        );
    }

    /// Classic, every Blue scheme, Glass light and dark and the Square frame, by name.
    fn windowed_themes() -> Vec<(String, Theme)> {
        use crate::glass::{Frame, Mode, Settings};
        use crate::interfaces::{blue_theme, theme_with};
        let mut all = vec![("classic".to_string(), Theme::default())];
        for v in crate::blue::Variant::ALL {
            all.push((format!("blue {v:?}"), blue_theme(v)));
        }
        for (name, mode, frame) in [
            ("glass light", Mode::Light, Frame::Rounded),
            ("glass dark", Mode::Dark, Frame::Rounded),
            ("glass square", Mode::Light, Frame::Square),
        ] {
            let s = Settings {
                mode,
                frame,
                ..Settings::default()
            };
            let t = theme_with(Interface::Glass, crate::blue::Variant::Blue, &s);
            all.push((name.to_string(), t));
        }
        all
    }

    /// A windowed dialog's frame round a 240x60 client at scale `s`, untitled so no font shows.
    fn windowed_frame(
        theme: &Theme,
        s: i32,
        active: bool,
        hot: Option<CaptionButton>,
        pressed: Option<CaptionButton>,
    ) -> (Canvas, FrameGeometry) {
        let g = FrameGeometry::layout(&theme.metrics, 240, 60, false, Chrome::WINDOWED_DIALOG)
            .scaled(s);
        let theme = theme.scaled(s);
        let text = loaded(theme.fonts.clone());
        let st = FrameState {
            title: "",
            active,
            maximized: false,
            pressed,
            glow: Glow::lit(hot),
            icon: None,
            tabbed: false,
        };
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_dialog_frame(&mut c, &theme, &text, &g, &st, None);
        (c, g)
    }

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        words
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            })
    }

    const MIN: Option<CaptionButton> = Some(CaptionButton::Minimize);
    const CLOSE: Option<CaptionButton> = Some(CaptionButton::Close);

    /// A 600 px bar on `edge` whose Quick Launch band, dragged to 60 px (a row of cells down a
    /// side bar), hides some of its five buttons behind the »: the bar with the » in `cell` and without it, at scale `s`.
    fn quick_more_bar(
        interface: Interface,
        edge: crate::taskbar::Edge,
        s: i32,
        cell: crate::taskbar::PopupCell,
    ) -> (Canvas, Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText, TaskbarCounts};
        let base = theme_for(interface);
        let t = if edge.is_side() {
            base.taskbar_shape().side_default(&base.metrics)
        } else {
            base.metrics.taskbar_height
        };
        let span = BarSpan {
            edge,
            length: 600,
            thickness: t,
        };
        let counts = TaskbarCounts {
            quick: 5,
            ..TaskbarCounts::default()
        };
        let bands = Bands {
            tasks_first: false,
            first_width: Some(if edge.is_side() { 30 } else { 60 }),
        };
        let shape = base.taskbar_shape();
        let l = TaskbarLayout::compute_on(
            &base.metrics,
            &shape,
            span,
            false,
            counts,
            ClockText::default(),
            bands,
        )
        .scaled(s);
        let theme = base.scaled(s);
        let text = TextRenderer::load(&FontConfig::default());
        let draw = |l: &TaskbarLayout, cell| {
            let mut st = empty_taskbar_state();
            st.quick_more = cell;
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_taskbar(&mut c, &theme, &text, &text, l, &st);
            c
        };
        let mut bare = l.clone();
        bare.quick_more = None;
        (draw(&l, cell), draw(&bare, cell), l)
    }

    fn quick_more_cells() -> [(&'static str, crate::taskbar::PopupCell); 3] {
        use crate::taskbar::PopupCell;
        let hot = PopupCell {
            hot: true,
            ..PopupCell::default()
        };
        let open = PopupCell {
            open: true,
            ..PopupCell::default()
        };
        [("rest", PopupCell::default()), ("hot", hot), ("open", open)]
    }

    #[test]
    fn the_quick_launch_more_cell_pins_in_every_look_and_state_at_scale_1_and_2() {
        let mut got = Vec::new();
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for s in [1, 2] {
                let mut drawn = Vec::new();
                for (name, cell) in quick_more_cells() {
                    let (c, _, l) =
                        quick_more_bar(interface, crate::taskbar::Edge::Bottom, s, cell);
                    let r = l
                        .quick_more
                        .expect("a 60 px band shows two buttons and the »");
                    let cut = c.copy_rect(r);
                    assert!(
                        !drawn.contains(&fnv(&cut)),
                        "{interface:?} {name} {s}: each state draws its own cell"
                    );
                    drawn.push(fnv(&cut));
                    got.push((format!("{interface:?} {name} {s}"), fnv(&cut)));
                }
            }
        }
        crate::pins::check("look/quick_more", &got);
    }

    #[test]
    fn the_quick_launch_more_cell_is_a_quick_button_face_under_the_double_chevron_on_every_edge() {
        use crate::taskbar::{Edge, draw_more};
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
                for s in [1, 2] {
                    let theme = theme_for(interface).scaled(s);
                    for (name, cell) in quick_more_cells() {
                        let (c, mut want, l) = quick_more_bar(interface, edge, s, cell);
                        let r = l.quick_more.expect("a short band ends in a »");
                        assert!(l.quick.len() < 5, "{interface:?} {edge:?}");
                        match &theme.look {
                            Look::Classic => {
                                let cs = &theme.colors;
                                let o = if cell.open {
                                    want.edge_t(r, EdgeStyle::ThinSunken, cs, s);
                                    s
                                } else {
                                    if cell.hot {
                                        want.edge_t(r, EdgeStyle::ThinRaised, cs, s);
                                    }
                                    0
                                };
                                let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
                                draw_more(&mut want, at, cs.button_text, s);
                            }
                            Look::Styled(st) => {
                                let part = crate::style::Part::TaskButton;
                                let lit = match (cell.open, cell.hot) {
                                    (true, _) => Some(crate::style::State::Checked),
                                    (false, true) => Some(crate::style::State::Hot),
                                    _ => None,
                                };
                                if let Some(l) = lit {
                                    st.draw(&mut want, r, part, l, s);
                                }
                                let ink = lit
                                    .and_then(|l| st.text_color(part, l))
                                    .unwrap_or(st.bar_text);
                                draw_more(&mut want, r, ink, s);
                            }
                        }
                        assert!(
                            c.copy_rect(r) == want.copy_rect(r),
                            "{interface:?} {edge:?} {name} at scale {s}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn windowed_dialog_frames_pin_in_every_look_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (name, theme) in windowed_themes() {
            for s in [1, 2] {
                for (state, active, hot, pressed) in [
                    ("rest", true, None, None),
                    ("inactive", false, None, None),
                    ("minimize hot", true, MIN, None),
                    ("minimize pressed", true, None, MIN),
                    ("close hot", true, CLOSE, None),
                ] {
                    let (c, _) = windowed_frame(&theme, s, active, hot, pressed);
                    got.push((format!("{name} {state} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("look/windowed_dialog_frame", &got);
    }

    /// A window rolled up to its caption: the frame round a 240 px wide client of no height, so
    /// the bottom border closes right under the caption, untitled.
    #[test]
    fn rolled_up_window_frames_pin_in_every_look_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (name, theme) in windowed_themes() {
            for s in [1, 2] {
                let m = &theme.metrics;
                let g = FrameGeometry::layout(m, 240, 0, false, Chrome::WINDOW).scaled(s);
                assert_eq!(
                    g.total.1,
                    (m.caption_height + 2 * m.frame_width) * s,
                    "{name}: only the caption and its borders"
                );
                let theme = theme.scaled(s);
                let text = loaded(theme.fonts.clone());
                for active in [true, false] {
                    let st = FrameState {
                        title: "",
                        active,
                        maximized: false,
                        pressed: None,
                        glow: Glow::default(),
                        icon: None,
                        tabbed: false,
                    };
                    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
                    draw_frame(&mut c, &theme, &text, &text, &g, &st);
                    let foot = c.get(g.total.0 / 2, g.total.1 - 1);
                    assert!(foot >> 24 > 0, "{name} @{s}: the bottom border is drawn");
                    got.push((format!("{name} active={active} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("look/rolled_up_frame", &got);
    }

    /// A dialog's frame with its ? box round a 240x60 client at scale `s`, untitled.
    fn help_dialog_frame(
        theme: &Theme,
        s: i32,
        active: bool,
        help: Interaction,
        hot: Option<CaptionButton>,
        pressed: Option<CaptionButton>,
    ) -> Canvas {
        let g = FrameGeometry::layout(&theme.metrics, 240, 60, false, Chrome::DIALOG).scaled(s);
        let theme = theme.scaled(s);
        let text = loaded(theme.fonts.clone());
        let st = FrameState {
            title: "",
            active,
            maximized: false,
            pressed,
            glow: Glow::lit(hot),
            icon: None,
            tabbed: false,
        };
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_dialog_frame(&mut c, &theme, &text, &g, &st, Some(help));
        c
    }

    #[test]
    fn dark_glass_dialog_frames_pin_at_scale_1_and_2() {
        use crate::glass::{Frame, Mode, Settings};
        use Interaction::{Hot, Idle, Pressed};
        let mut got = Vec::new();
        for (name, frame) in [("rounded", Frame::Rounded), ("square", Frame::Square)] {
            let theme = |mode| {
                let g = Settings {
                    mode,
                    frame,
                    ..Settings::default()
                };
                crate::interfaces::theme_with(Interface::Glass, crate::blue::Variant::Blue, &g)
            };
            let (dark, light) = (theme(Mode::Dark), theme(Mode::Light));
            for s in [1, 2] {
                assert_ne!(
                    help_dialog_frame(&dark, s, true, Idle, None, None).pixels,
                    help_dialog_frame(&light, s, true, Idle, None, None).pixels,
                    "{name} at {s}: the dark frame is not the light one"
                );
                let mut seen = Vec::new();
                for (state, active, help, hot, pressed) in [
                    ("rest", true, Idle, None, None),
                    ("inactive", false, Idle, None, None),
                    ("help hot", true, Hot, None, None),
                    ("help pressed", true, Pressed, None, None),
                    ("close hot", true, Idle, CLOSE, None),
                    ("close pressed", true, Idle, None, CLOSE),
                ] {
                    let hash = fnv(&help_dialog_frame(&dark, s, active, help, hot, pressed));
                    assert!(
                        !seen.contains(&hash),
                        "{name} {state} at {s} draws like another state"
                    );
                    seen.push(hash);
                    got.push((format!("{name} {state} {s}"), hash));
                }
            }
        }
        crate::pins::check("look/dark_dialog_frame", &got);
    }

    #[test]
    fn a_windowed_dialogs_minimize_box_lights_and_presses_inside_its_box() {
        for (name, theme) in windowed_themes() {
            for s in [1, 2] {
                let (rest, g) = windowed_frame(&theme, s, true, None, None);
                let min = g.button_rect(CaptionButton::Minimize);
                let mut states = vec![("pressed", windowed_frame(&theme, s, true, None, MIN).0)];
                if theme.look.style().is_some() {
                    states.push(("hot", windowed_frame(&theme, s, true, MIN, None).0));
                }
                for (what, c) in states {
                    let differs = |r: Rect| {
                        (r.y..r.bottom())
                            .any(|y| (r.x..r.right()).any(|x| c.get(x, y) != rest.get(x, y)))
                    };
                    assert!(
                        differs(min),
                        "{name} at {s}: {what} changes the minimize box"
                    );
                    let left = Rect::new(0, 0, min.x - 3 * s, g.total.1);
                    assert!(
                        !differs(left),
                        "{name} at {s}: {what} leaves the caption left of it"
                    );
                }
            }
        }
    }

    #[test]
    fn a_rounded_glass_windowed_dialog_draws_its_min_close_strip_from_minimize_to_close() {
        use crate::style::{Part, State};
        let rounded = windowed_themes().into_iter();
        for (name, theme) in rounded.filter(|t| t.0 == "glass light" || t.0 == "glass dark") {
            for s in [1, 2] {
                let scaled = theme.scaled(s);
                let style = scaled.look.style().unwrap();
                for (active, hot, pressed, state, n) in [
                    (true, None, None, State::Normal, 0),
                    (false, None, None, State::Inactive, 0),
                    (true, MIN, None, State::Hot, 0),
                    (true, CLOSE, None, State::Hot, 1),
                    (true, None, MIN, State::Pressed, 0),
                ] {
                    let (c, g) = windowed_frame(&theme, s, active, hot, pressed);
                    let [min, _, close] = g.buttons;
                    assert_eq!(close.right() - min.x, 78 * s, "{name}: one 78 px strip");
                    let strip = Rect::new(0, 0, 78 * s, 20 * s);
                    let mut want = Canvas::new(strip.width as u32, strip.height as u32);
                    style.draw_nth(&mut want, strip, Part::MinCloseGroup, state, n, s);
                    for y in 0..strip.height {
                        for x in 0..strip.width {
                            assert_eq!(
                                c.get(min.x + x, close.y + y),
                                want.get(x, y),
                                "{name} at {s}: {state:?} {n}, ({x}, {y}) of the strip"
                            );
                        }
                    }
                }
            }
        }
    }
}
