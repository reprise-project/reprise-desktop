//! Which look paints a surface: Classic's own draw code, or a [`Style`] for a skinned look.

use crate::art::glyph::Glyph;
use crate::classic;
use crate::frame::{ButtonSet, CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::{Style, frame as style_frame, taskbar as style_taskbar};
use crate::taskbar::{TaskbarLayout, TaskbarState};
use crate::text::TextRenderer;
use crate::theme::{FontConfig, Theme};
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
    let p = theme.look.style()?.part(Part::ClientGlass)?;
    let f = p.frame(if active {
        State::Normal
    } else {
        State::Inactive
    });
    Some(p.atlas.get(f.x, f.y))
}

/// Lay the caption title's glow (or dark Glass's edge) under whatever is drawn in `r` over clear
/// [`client_glass`], so labels and icons there read on any tint; nothing in a look without one.
pub fn glow_on_glass(canvas: &mut Canvas, theme: &Theme, r: Rect) {
    let Some(glow) = theme.look.style().and_then(|s| s.caption_title.glow) else {
        return;
    };
    let ink = canvas.copy_rect(r);
    let (w, h) = ink.device_size();
    let (w, h) = (w as usize, h as usize);
    let coverage: Vec<u8> = ink.pixels.iter().map(|&p| (p >> 24) as u8).collect();
    let s = canvas.scale();
    let alpha = (glow.alpha)(&crate::style::GlowMask {
        coverage: &coverage,
        width: w,
        height: h,
        rows: (0, h),
        active: theme.active,
        scale: theme.scale.max(1) * s,
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
            style_taskbar::draw_taskbar(canvas, st, theme, text, start_text, layout, state);
            if let Some((_, r)) = ring {
                classic::widgets::draw_focus_ring(canvas, r, s, st.bar_text);
            }
        }
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
                let n =
                    i32::from(chrome.buttons.has(CaptionButton::Close)) + i32::from(chrome.help);
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

/// A dialog's frame over a client of button face: border, caption, the close box when `geom.chrome` has one, and the ? box left of it when `help` is `Some`. Classic's dialog caption has no icon and its ? box ignores the pointer; a styled look draws its usual frame and help button part, else Classic's ? box.
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
            let close = geom.chrome.buttons.has(CaptionButton::Close);
            let buttons: Vec<(Rect, Glyph)> = close
                .then_some((geom.buttons[2], Glyph::Close))
                .into_iter()
                .chain(help_box)
                .collect();
            classic::widgets::draw_caption(
                c,
                text,
                cs,
                geom.caption,
                state.title,
                state.active,
                &buttons,
                theme.fonts.size_px,
                s,
            );
            if close && state.pressed == Some(CaptionButton::Close) {
                classic::widgets::draw_caption_button(
                    c,
                    cs,
                    geom.buttons[2],
                    Glyph::Close,
                    true,
                    s,
                );
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

    #[test]
    fn caption_fit_leaves_a_title_exactly_its_room_in_every_interface() {
        let title = "User Accounts";
        for (i, dialog, prompt) in [
            (Interface::Classic, 159, 103),
            (Interface::Blue, 179, 179),
            (Interface::Glass, 201, 201),
        ] {
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
                    let need = t.measure(title, true, theme.fonts.size_px);
                    (need, crate::text::test_tahoma())
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
                continue;
            }
            assert_eq!(fit, dialog, "{i:?} dialog caption");
            assert_eq!(
                caption_fit(&theme, title, Chrome::PROMPT),
                prompt,
                "{i:?} prompt caption"
            );
        }
        let undecorated = Chrome {
            decorated: false,
            ..Chrome::DIALOG
        };
        assert_eq!(caption_fit(&Theme::default(), title, undecorated), 0);
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
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
}
