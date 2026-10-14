//! The interfaces put together: each one's starting theme and its previews, switcher, overview,
//! snap panel and the Glass Start panel. This module picks Blue's or Glass's style by name, so it
//! sits above every interface.

use crate::interface::Interface;
use crate::look::{self, Look};
use crate::overview;
use crate::paint::{Backdrop, Paint};
use crate::raster::{Canvas, Rect};
use crate::snaplayouts;
use crate::startpanel::{self, StartPanel};
use crate::style::Style;
use crate::switcher::{self, SwitcherLayout, ThumbCell};
use crate::taskpreview::{Cell, Grid, TaskPreviewLayout};
use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig, Theme};
use crate::wallpapers::{self, Builtin, Design};
use crate::{blue, controls, glass};
use std::sync::Arc;

pub fn default_wallpaper(i: Interface) -> Option<Builtin> {
    match i {
        Interface::Classic => None,
        Interface::Blue => Builtin::named(Design::Tarn, "blue"),
        Interface::Glass => Some(Builtin {
            design: Design::Flow,
            variant: wallpapers::Variant::Glass,
        }),
    }
}

pub fn resolve_wallpaper(b: Builtin, glass: &glass::Settings) -> Builtin {
    let c = glass.color;
    let hue = wallpapers::tone::hue_of(wallpapers::tone::hex(
        (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32,
    ));
    b.resolve(hue, glass.mode == glass::Mode::Dark)
}

pub fn theme_for(i: Interface) -> Theme {
    theme_with(i, blue::Variant::Blue, &glass::Settings::default())
}

/// The theme an interface starts from, before the config's overrides;
/// Blue paints in `variant`'s colour scheme, Glass in `glass`'s colour.
/// Each interface ignores the other's argument.
pub fn theme_with(i: Interface, variant: blue::Variant, glass: &glass::Settings) -> Theme {
    match i {
        Interface::Classic => Theme::default(),
        Interface::Blue => blue_theme(variant),
        Interface::Glass => styled(glass::style(glass)),
    }
}

pub fn blue_theme(v: blue::Variant) -> Theme {
    styled(blue::style_for(v))
}

pub fn sample_theme(
    i: Interface,
    scheme: &str,
    glass: &glass::Settings,
    background: Option<Color>,
) -> Theme {
    let variant = blue::Variant::from_name(scheme).unwrap_or_default();
    let mut t = theme_with(i, variant, glass);
    if i == Interface::Classic
        && let Some(colors) = crate::schemes::scheme(scheme)
    {
        t.colors = colors;
    }
    if let Some(c) = background {
        t.colors.background = c;
    }
    t
}

/// What runs over in the forms `build` makes for each interface, its theme at every text size
/// and the text they are measured with, in Tahoma (where installed) and DejaVu Sans: one line
/// per item ([`controls::form::Form::overflows`]), empty when all the text fits. For a
/// program's audit test.
pub fn form_overflows(
    mut build: impl FnMut(Interface, &Theme, &TextRenderer) -> Vec<(String, controls::form::Form)>,
) -> Vec<String> {
    overflows(|i, theme, text| {
        let mut over = Vec::new();
        for (name, form) in build(i, theme, text) {
            form.drawn_with(&theme.paint());
            for (id, by) in form.overflows(text, theme.fonts.size_px) {
                over.push((name.clone(), id, by));
            }
        }
        over
    })
}

/// What `check` finds running over, by surface name, part and px, for each interface, its
/// theme at every text size and the text it is measured with, in Tahoma (where installed) and
/// DejaVu Sans: one line per part, empty when all the text fits. For an audit test of a
/// control that lays out its own text.
pub fn overflows(
    mut check: impl FnMut(Interface, &Theme, &TextRenderer) -> Vec<(String, &'static str, i32)>,
) -> Vec<String> {
    let mut over = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        if !text.has_face(family) {
            continue;
        }
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 125, 150, 175, 200] {
                let theme = theme_for(i).with_form_text(pct);
                for (name, id, by) in check(i, &theme, &text) {
                    over.push(format!(
                        "{family}, {}, {pct}%, {name}: {id} {by} px over",
                        i.as_str()
                    ));
                }
            }
        }
    }
    over
}

pub fn small_bar(theme: &mut Theme, glass: &glass::Settings) {
    theme.metrics.taskbar_height = glass::SMALL_TASKBAR_HEIGHT;
    theme.look = Look::Styled(glass::small_bar_style(glass));
}

fn styled(s: Arc<Style>) -> Theme {
    Theme {
        colors: s.colors.clone(),
        metrics: s.metrics.clone(),
        fonts: FontConfig::default(),
        scale: 1,
        look: Look::Styled(s),
        active: true,
        text: crate::theme::TextUnits::ONE,
    }
}

pub fn task_preview_grid(theme: &Theme) -> Grid {
    if theme.glass().is_some() {
        glass::preview::GRID
    } else {
        controls::taskpreview::CLASSIC
    }
}

/// is Classic's renderer for `theme.fonts`, unused by Glass, which sets
/// the titles in its caption face.
pub fn draw_task_preview(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskPreviewLayout,
    cells: &[Cell],
    hot: Option<usize>,
) {
    match theme.look.style() {
        Some(s) if s.glass.is_some() => {
            let size = s.caption_text.size_px * theme.scale.max(1) as u32;
            let face = look::caption_text(s);
            glass::preview::draw(canvas, s, &face, size, layout, cells, hot);
        }
        _ => controls::taskpreview::draw(canvas, theme, text, layout, cells, hot),
    }
}

pub fn switcher_grid(theme: &Theme) -> switcher::Grid {
    if theme.glass().is_some() {
        glass::switcher::GRID
    } else {
        controls::switcher::CLASSIC
    }
}

pub fn switcher_icon_grid(theme: &Theme) -> Option<switcher::Grid> {
    theme.glass().map(|_| glass::switcher::ICON_GRID)
}

#[expect(clippy::too_many_arguments)]
pub fn draw_switcher(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &SwitcherLayout,
    title: &str,
    cells: &[ThumbCell],
    selected: usize,
    hot: Option<usize>,
) {
    match theme.look.style() {
        Some(s) if s.glass.is_some() => {
            let px = layout.grid.map_or(0, |g| g.title_px);
            let px = if px == 0 { s.caption_text.size_px } else { px };
            let size = px * layout.scale.max(1) as u32;
            let face = look::caption_text(s);
            let fallback = glass_fallback(theme, layout, cells);
            let cells = with_icons(cells, fallback.as_ref());
            glass::switcher::draw(canvas, s, &face, size, layout, title, &cells, selected, hot);
        }
        _ => controls::switcher::draw_thumbnails(
            canvas, theme, text, layout, title, cells, selected, hot,
        ),
    }
}

pub fn draw_overview(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &overview::OverviewLayout,
    cells: &[overview::Cell],
    selected: usize,
    hot: Option<usize>,
) {
    match theme.look.style() {
        Some(s) if s.glass.is_some() => {
            let size = s.caption_text.size_px * layout.scale.max(1) as u32;
            let face = look::caption_text(s);
            glass::overview::draw(canvas, s, &face, size, layout, cells, selected, hot);
        }
        Some(_) => blue::overview::draw(canvas, theme, text, layout, cells, selected, hot),
        None => controls::overview::draw(canvas, theme, text, layout, cells, selected, hot),
    }
}

pub fn draw_snap_assist(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &overview::OverviewLayout,
    cells: &[overview::Cell],
    lit: Option<usize>,
    hot: Option<usize>,
) {
    match theme.look.style() {
        Some(s) if s.glass.is_some() => {
            let size = s.caption_text.size_px * layout.scale.max(1) as u32;
            let face = look::caption_text(s);
            glass::snapassist::draw(canvas, s, &face, size, layout, cells, lit, hot);
        }
        _ => controls::snapassist::draw(canvas, theme, text, layout, cells, lit, hot),
    }
}

pub fn draw_snap_picker(
    theme: &Theme,
    layout: &snaplayouts::PickerLayout,
    icons: &[(snaplayouts::Zone, &Canvas)],
    hot: Option<snaplayouts::Zone>,
) -> Canvas {
    match theme.look.style() {
        Some(s) if s.glass.is_some() => {
            let mut c = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
            glass::snappicker::draw(&mut c, s, theme.colors.hilight, layout, icons, hot);
            c
        }
        _ => controls::snappicker::draw(theme, layout, icons, hot),
    }
}

pub fn draw_switcher_badges(
    canvas: &mut Canvas,
    theme: &Theme,
    layout: &SwitcherLayout,
    cells: &[ThumbCell],
) {
    if theme.glass().is_some() {
        let fallback = glass_fallback(theme, layout, cells);
        glass::switcher::draw_badges(canvas, layout, &with_icons(cells, fallback.as_ref()));
    } else {
        controls::switcher::draw_badges(canvas, theme, layout, cells);
    }
}

fn glass_fallback(theme: &Theme, layout: &SwitcherLayout, cells: &[ThumbCell]) -> Option<Canvas> {
    cells
        .iter()
        .any(|c| c.icon.is_none())
        .then(|| crate::icon::chrome_fallback(theme, switcher::ICON_PX * layout.scale.max(1)))
}

fn with_icons<'a>(cells: &[ThumbCell<'a>], fallback: Option<&'a Canvas>) -> Vec<ThumbCell<'a>> {
    cells
        .iter()
        .map(|c| ThumbCell {
            icon: c.icon.or(fallback),
            ..*c
        })
        .collect()
}

pub fn start_panel_text(_theme: &Theme) -> TextRenderer {
    TextRenderer::load(&glass::startpanel::font())
}

#[expect(clippy::too_many_arguments)]
pub fn draw_start_panel(
    canvas: &mut Canvas,
    theme: &Theme,
    glass: &glass::Settings,
    text: &TextRenderer,
    p: &StartPanel,
    l: &startpanel::layout::Layout,
    pressed: Option<startpanel::Spot>,
    caret: bool,
) {
    let style = match &theme.look {
        Look::Styled(s) if s.glass.is_some() && glass.transparency => s.clone(),
        _ => glass::style(&glass::Settings {
            transparency: false,
            ..*glass
        })
        .at_scale(l.scale.max(1)),
    };
    glass::startpanel::draw(canvas, &style, text, p, l, pressed, caret);
    if let Some(bar) = l.tree_bar {
        let look = Look::Styled(style.clone());
        let paint = Paint {
            cs: &style.colors,
            look: &look,
            s: l.scale.max(1),
            back: Backdrop::Face,
            active: true,
            text: crate::theme::TextUnits::ONE,
        };
        let o = l.canvas;
        let sb = Rect::new(bar.x - o.x, bar.y - o.y, bar.width, bar.height);
        let rows = p.tree_rows().len();
        let shown = startpanel::layout::tree_capacity(p);
        look::widgets::draw_scrollbar(canvas, &paint, sb, p.tree_top, shown, rows, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Insets;

    #[test]
    fn each_interface_has_its_own_default_wallpaper() {
        assert_eq!(default_wallpaper(Interface::Classic), None);
        let uri = |i| default_wallpaper(i).map(|b| b.uri());
        assert_eq!(uri(Interface::Blue).as_deref(), Some("reprise:tarn/blue"));
        assert_eq!(uri(Interface::Glass).as_deref(), Some("reprise:flow/glass"));
    }

    #[test]
    fn the_glass_default_follows_the_glass_colour_and_mode() {
        let flow = default_wallpaper(Interface::Glass).unwrap();
        let mut g = glass::Settings::default();
        let light = resolve_wallpaper(flow, &g);
        assert!(
            matches!(light.variant, wallpapers::Variant::Hue { dark: false, .. }),
            "light mode tints flow: {light}"
        );
        g.color = glass::swatch("ruby").expect("ruby is a swatch");
        assert_ne!(
            resolve_wallpaper(flow, &g),
            light,
            "a new colour, a new tint"
        );
        g.mode = glass::Mode::Dark;
        assert_eq!(resolve_wallpaper(flow, &g).to_string(), "flow/night");
    }

    #[test]
    fn a_sample_takes_the_scheme_of_its_own_interface() {
        let g = glass::Settings::default();
        let brick = crate::schemes::scheme("Brick").expect("Brick is a Classic scheme");
        assert_eq!(
            sample_theme(Interface::Classic, "Brick", &g, None).colors,
            brick
        );
        assert_eq!(
            sample_theme(Interface::Blue, "Olive Green", &g, None),
            blue_theme(blue::Variant::Olive)
        );
        // A name of another interface's, or none, is the interface's own.
        assert_eq!(
            sample_theme(Interface::Blue, "Brick", &g, None),
            blue_theme(blue::Variant::Blue)
        );
        assert_eq!(
            sample_theme(Interface::Classic, "Olive Green", &g, None),
            Theme::default()
        );
        assert_eq!(
            sample_theme(Interface::Classic, "", &g, None),
            Theme::default()
        );
        let ruby = glass::Settings {
            color: glass::swatch("ruby").expect("ruby is a swatch"),
            ..g
        };
        assert_eq!(
            sample_theme(Interface::Glass, "Brick", &ruby, None),
            theme_with(Interface::Glass, blue::Variant::Blue, &ruby),
            "Glass has no scheme; its colour is the glass's"
        );
    }

    #[test]
    fn a_sample_draws_its_desktop_in_the_colour_given() {
        let text = crate::text::TextRenderer::load(&FontConfig::default());
        let teal = Color::rgb(0, 128, 128);
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let t = sample_theme(i, "", &glass::Settings::default(), Some(teal));
            let c = look::draw_sample(false, &t, 1, &text, None);
            assert_eq!(c.get(1, 1), teal.argb(), "{i:?}");
        }
        let brick = crate::schemes::scheme("Brick").expect("Brick is a Classic scheme");
        let t = sample_theme(
            Interface::Classic,
            "Brick",
            &glass::Settings::default(),
            None,
        );
        let c = look::draw_sample(false, &t, 1, &text, None);
        assert_eq!(c.get(1, 1), brick.background.argb());
    }

    #[test]
    fn the_small_bar_shrinks_a_glass_sample() {
        let g = glass::Settings::default();
        let mut t = sample_theme(Interface::Glass, "", &g, None);
        assert_ne!(t.metrics.taskbar_height, glass::SMALL_TASKBAR_HEIGHT);
        small_bar(&mut t, &g);
        assert_eq!(t.metrics.taskbar_height, 30);
    }

    #[test]
    fn the_switcher_grid_follows_the_look() {
        assert_eq!(
            switcher_grid(&theme_for(Interface::Glass)).shadow,
            glass::switcher::GRID.shadow
        );
        assert_eq!(
            switcher_grid(&theme_for(Interface::Classic)).shadow,
            Insets::ZERO
        );
        assert_eq!(
            switcher_grid(&theme_for(Interface::Blue)).shadow,
            Insets::ZERO
        );
    }

    #[test]
    fn theme_for_blue_takes_the_style_metrics_and_look() {
        let t = theme_for(Interface::Blue);
        assert_eq!(t.metrics.caption_height, 26);
        assert!(matches!(t.look, Look::Styled(_)));
        assert_eq!(theme_for(Interface::Classic), Theme::default());
    }

    #[test]
    fn theme_with_blue_takes_the_variants_look() {
        let d = glass::Settings::default();
        for v in blue::Variant::ALL {
            let t = theme_with(Interface::Blue, v, &d);
            assert_eq!(t.look, blue_theme(v).look, "{v:?}");
        }
        assert_eq!(
            theme_with(Interface::Blue, blue::Variant::Blue, &d).look,
            theme_for(Interface::Blue).look
        );
        assert_ne!(
            theme_with(Interface::Blue, blue::Variant::Silver, &d).look,
            theme_for(Interface::Blue).look
        );
        assert_eq!(
            theme_with(Interface::Classic, blue::Variant::Olive, &d),
            Theme::default()
        );
    }

    #[test]
    fn theme_for_glass_is_translucent_and_rounded() {
        let t = theme_for(Interface::Glass);
        assert_eq!(t.metrics.frame_width, 8, "glass borders are 8 px");
        assert_eq!(
            t.metrics.caption_height, 22,
            "the top band is 8 + 22 = 30 px"
        );
        assert_eq!(t.glass().map(|g| g.corner), Some(&[5, 3, 2, 1, 1, 1][..]));
        assert_eq!(
            theme_for(Interface::Blue).glass(),
            None,
            "Blue has no backdrop"
        );
        assert_eq!(Theme::default().glass(), None, "Classic has no backdrop");
    }

    #[test]
    fn glass_settings_change_the_style() {
        let a = theme_with(
            Interface::Glass,
            blue::Variant::Blue,
            &glass::Settings::default(),
        );
        let opaque = glass::Settings {
            transparency: false,
            ..Default::default()
        };
        let b = theme_with(Interface::Glass, blue::Variant::Blue, &opaque);
        // Row 10 of the caption atlas's stretched middle: plain glass,
        // clear of the outline and the light line.
        let fa = a
            .look
            .style()
            .unwrap()
            .part(crate::style::Part::Caption)
            .unwrap()
            .atlas
            .get(221, 10);
        let fb = b
            .look
            .style()
            .unwrap()
            .part(crate::style::Part::Caption)
            .unwrap()
            .atlas
            .get(221, 10);
        assert!(
            fa >> 24 < 0xff,
            "translucent glass has alpha below 255: {fa:08x}"
        );
        assert_eq!(fb >> 24, 0xff, "opaque glass is opaque: {fb:08x}");
    }
}
