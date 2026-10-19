//! The hidden icons flyout: tray items kept off the bar, as a grid of 16 px icons opened by the ^
//! cell, with a Customize... link under it.

use crate::access::accesskit::{Action, NodeId, Role};
use crate::access::{self, Tree};
use crate::icon::ICON_SIZE;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

pub const COLS: usize = 4;
/// A cell's side: the menu panel's, and Glass's, whose icons sit further apart.
pub const CELL: i32 = 24;
pub const GLASS_CELL: i32 = 32;
const MENU_INSET: i32 = 4;
const GLASS_PAD: i32 = 2;
pub const LINK_H: i32 = 41;
pub const LINK: &str = "Customize...";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlyoutLayout {
    pub width: i32,
    pub height: i32,
    pub cells: Vec<Rect>,
    pub frame: Option<i32>,
    pub link: Option<Rect>,
}

impl FlyoutLayout {
    pub fn compute(theme: &Theme, count: usize) -> FlyoutLayout {
        let frame = crate::flyout::look(theme).map(|_| crate::flyout::border(theme));
        let inset = frame.map_or(MENU_INSET, |f| f + GLASS_PAD);
        let cell = if frame.is_some() { GLASS_CELL } else { CELL };
        let n = count.max(1);
        let cols = if frame.is_some() { COLS } else { n.min(COLS) } as i32;
        let rows = n.div_ceil(COLS) as i32;
        let link = frame.map(|_| Rect::new(inset, inset + rows * cell, cols * cell, LINK_H));
        let cells = (0..count)
            .map(|i| {
                let (c, r) = ((i % COLS) as i32, (i / COLS) as i32);
                Rect::new(inset + c * cell, inset + r * cell, cell, cell)
            })
            .collect();
        FlyoutLayout {
            width: 2 * inset + cols * cell,
            height: 2 * inset + rows * cell + link.map_or(0, |l| l.height),
            cells,
            frame,
            link,
        }
    }

    pub fn scaled(&self, n: i32) -> FlyoutLayout {
        let n = n.max(1);
        FlyoutLayout {
            width: self.width * n,
            height: self.height * n,
            cells: self.cells.iter().map(|r| r.scaled(n)).collect(),
            frame: self.frame.map(|f| f * n),
            link: self.link.map(|r| r.scaled(n)),
        }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.cells
            .iter()
            .position(|r| r.contains(x, y))
            .or_else(|| {
                self.link
                    .filter(|l| l.contains(x, y))
                    .map(|_| self.cells.len())
            })
    }
}

pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    layout: &FlyoutLayout,
    icons: &[Option<Canvas>],
    link_hot: bool,
) -> Canvas {
    let panel = draw_panel(theme, text, layout, icons, link_hot);
    crate::flyout::shadowed(theme, panel)
}

fn draw_panel(
    theme: &Theme,
    text: &TextRenderer,
    layout: &FlyoutLayout,
    icons: &[Option<Canvas>],
    link_hot: bool,
) -> Canvas {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let mut c = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
    let all = Rect::new(0, 0, layout.width, layout.height);
    match (layout.frame, theme.look.style()) {
        (Some(f), Some(style)) => {
            crate::flyout::draw_border(&mut c, style, all, f, s);
            c.fill_rect(all.inset(f), cs.window);
        }
        _ => {
            c.fill_rect(all, cs.menu);
            c.edge_t(all, EdgeStyle::WindowRaised, cs, s);
        }
    }
    let px = ICON_SIZE * s;
    for (r, icon) in layout.cells.iter().zip(icons) {
        if let Some(ic) = icon {
            c.blit_canvas(r.x + (r.width - px) / 2, r.y + (r.height - px) / 2, ic);
        }
    }
    if let (Some(l), Some(f)) = (layout.link, layout.frame) {
        let page = theme.look.style().and_then(|st| st.page);
        // The band runs from the panel's left edge to its right and foot.
        let band = Rect::new(f, l.y, layout.width - 2 * f, layout.height - f - l.y);
        if let Some(inks) = page {
            c.fill_rect(band, inks.band);
            for (k, row) in inks.band_rule.iter().enumerate() {
                let y = band.y + k as i32 * s;
                c.fill_rect(Rect::new(band.x, y, band.width, s), *row);
            }
        }
        let px = theme.fonts.size_px;
        let w = text.measure(LINK, false, px);
        let tr = Rect::new(band.x + (band.width - w) / 2, band.y, w, band.height);
        crate::flyout::draw_link(&mut c, text, &theme.paint(), tr, LINK, link_hot, px);
    }
    c
}

/// The flyout's `access` part, from its top-left: one button per cell,
/// named from `names` (the item's tip). The flyout takes no keyboard
/// focus, so neither do its buttons.
pub fn access(layout: &FlyoutLayout, names: &[String], tree: &mut Tree, parent: NodeId) {
    for (i, (r, name)) in layout.cells.iter().zip(names).enumerate() {
        let mut n = access::node(Role::Button, *r);
        if !name.is_empty() {
            n.set_label(name.as_str());
        }
        n.add_action(Action::Click);
        tree.push(parent, i, n);
    }
    if let Some(l) = layout.link {
        let mut n = access::named(Role::Link, l, LINK);
        n.add_action(Action::Click);
        tree.push(parent, "link", n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass;
    use crate::interface::Interface;
    use crate::interfaces::theme_with;

    fn text() -> TextRenderer {
        TextRenderer::load(&crate::theme::FontConfig::default())
    }

    fn glass_theme() -> Theme {
        theme_with(
            Interface::Glass,
            crate::blue::Variant::Blue,
            &glass::Settings::default(),
        )
    }

    #[test]
    fn four_icons_a_row_then_the_next_row() {
        let l = FlyoutLayout::compute(&Theme::default(), 6);
        assert_eq!((l.width, l.height), (4 + 4 * 24 + 4, 4 + 2 * 24 + 4));
        assert_eq!(l.cells[0], Rect::new(4, 4, 24, 24));
        assert_eq!(l.cells[3], Rect::new(76, 4, 24, 24));
        assert_eq!(l.cells[4], Rect::new(4, 28, 24, 24));
        assert_eq!(l.hit(80, 10), Some(3));
        assert_eq!(l.hit(30, 30), Some(5));
        assert_eq!(l.hit(80, 30), None, "past the last item");
        assert_eq!(l.frame, None);
    }

    #[test]
    fn a_short_row_is_only_as_wide_as_its_icons() {
        let l = FlyoutLayout::compute(&Theme::default(), 2);
        assert_eq!((l.width, l.height), (4 + 2 * 24 + 4, 4 + 24 + 4));
    }

    #[test]
    fn under_glass_32_px_cells_sit_inside_an_8_px_frame() {
        let l = FlyoutLayout::compute(&glass_theme(), 1);
        assert_eq!(l.frame, Some(8));
        assert_eq!(l.cells[0], Rect::new(10, 10, 32, 32));
        assert_eq!((l.width, l.height), (148, 93));
        assert_eq!(l.link, Some(Rect::new(10, 42, 128, 41)));
        assert_eq!(l.hit(50, 50), Some(1), "the link is one past the cells");
        assert_eq!(l.hit(50, 20), None, "past the only icon");
        assert!(
            FlyoutLayout::compute(&Theme::default(), 1).link.is_none(),
            "the menu panel has no link"
        );
    }

    #[test]
    fn the_menu_panel_is_raised_with_icons_centred() {
        let theme = Theme::default();
        let l = FlyoutLayout::compute(&theme, 1);
        let icon = Canvas::from_pixels(16, 16, vec![0xFFFF_0000; 256]);
        let c = draw_panel(&theme, &text(), &l, &[Some(icon)], false);
        let cs = &theme.colors;
        assert_eq!(c.get(1, 1), cs.button_hilight.argb(), "raised edge");
        assert_eq!(c.get(3, 3), cs.menu.argb(), "panel");
        assert_eq!(c.get(8, 8), 0xFFFF_0000, "icon 4 px into its cell");
        assert_eq!(c.get(7, 7), cs.menu.argb());
        assert_eq!(c.get(23, 23), 0xFFFF_0000);
        assert_eq!(c.get(24, 24), cs.menu.argb());
    }

    #[test]
    fn the_glass_flyout_has_translucent_borders_round_an_opaque_panel() {
        let theme = glass_theme();
        let l = FlyoutLayout::compute(&theme, 1);
        let c = draw_panel(&theme, &text(), &l, &[None], false);
        let alpha = |x, y| c.get(x, y) >> 24;
        for (x, y) in [(20, 0), (0, 20), (147, 20), (20, 92)] {
            assert!(alpha(x, y) > 0, "outline at ({x}, {y})");
        }
        for (x, y) in [(20, 3), (3, 20), (144, 20), (20, 89)] {
            assert!(alpha(x, y) < 255, "glass at ({x}, {y})");
        }
        assert_eq!(c.get(0, 0) >> 24, 0, "rounded corner");
        assert_eq!(c.get(20, 20), theme.colors.window.argb(), "panel");
    }

    #[test]
    fn a_glass_icon_sits_centred_in_its_32_px_cell() {
        let theme = glass_theme();
        let l = FlyoutLayout::compute(&theme, 2);
        let icon = || Some(Canvas::from_pixels(16, 16, vec![0xFFFF_0000; 256]));
        let c = draw_panel(&theme, &text(), &l, &[icon(), icon()], false);
        let window = theme.colors.window.argb();
        for x0 in [18, 50] {
            assert_eq!(c.get(x0, 18), 0xFFFF_0000, "icon at ({x0}, 18)");
            assert_eq!(c.get(x0 + 15, 33), 0xFFFF_0000);
            assert_eq!((c.get(x0 - 1, 18), c.get(x0 + 16, 33)), (window, window));
        }
    }

    #[test]
    fn the_glass_flyout_at_scale_2_doubles_the_scale_1_flyout() {
        let mut theme = glass_theme();
        let l = FlyoutLayout::compute(&theme, 5);
        let l = FlyoutLayout { link: None, ..l };
        let one = draw_panel(&theme, &text(), &l, &[None, None, None, None, None], false);
        theme.scale = 2;
        let two = draw_panel(
            &theme,
            &text(),
            &l.scaled(2),
            &[None, None, None, None, None],
            false,
        );
        assert!(
            crate::look::styled::fixture::doubles_at_scale_1(&one, &two),
            "the scale-2 frame and panel are the scale-1 pixels, each 2x2"
        );
    }

    #[test]
    fn the_glass_link_sits_centred_on_a_shaded_band_and_underlines_when_hot() {
        let theme = glass_theme();
        let inks = theme.look.style().and_then(|st| st.page).unwrap();
        let l = FlyoutLayout::compute(&theme, 2);
        let c = draw_panel(&theme, &text(), &l, &[None, None], false);
        assert_eq!(c.get(8, 42), inks.band_rule[0].argb());
        assert_eq!(c.get(139, 46), inks.band_rule[4].argb());
        assert_eq!(c.get(8, 47), inks.band.argb());
        assert_eq!(c.get(60, 84), inks.band.argb(), "the band's last row");
        assert_eq!(c.get(60, 41), theme.colors.window.argb(), "the grid");
        let link = inks.link.argb();
        assert!((42..85).any(|y| (8..140).any(|x| c.get(x, y) == link)));
        let band = inks.band.argb();
        let ink: Vec<(i32, i32)> = (47..85)
            .flat_map(|y| (8..140).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != band)
            .collect();
        let (x0, x1) = (
            ink.iter().map(|p| p.0).min().unwrap(),
            ink.iter().map(|p| p.0).max().unwrap(),
        );
        let (y0, y1) = (
            ink.iter().map(|p| p.1).min().unwrap(),
            ink.iter().map(|p| p.1).max().unwrap(),
        );
        assert!(
            ((x0 - 8) - (139 - x1)).abs() <= 3,
            "centred across: {x0}..{x1}"
        );
        assert!(
            ((y0 - 42) - (84 - y1)).abs() <= 2,
            "centred down: {y0}..{y1}"
        );
        let hot = draw_panel(&theme, &text(), &l, &[None, None], true);
        let under = (y1 + 1..85)
            .find(|&y| (x0 + 1..x1).all(|x| hot.get(x, y) == link))
            .expect("an underline under the link");
        assert_eq!(under, y1 + 2, "a row clear under the glyphs");
    }

    #[test]
    fn the_glass_flyout_casts_the_window_shadow() {
        let theme = glass_theme();
        let l = FlyoutLayout::compute(&theme, 1);
        let c = draw(&theme, &text(), &l, &[None], false);
        let m = crate::flyout::margin(&theme);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (l.width + m.left + m.right, l.height + m.top + m.bottom)
        );
        assert!(c.get(m.left - 1, m.top + 40) >> 24 > 0, "shade left of it");
        assert_eq!(c.get(m.left + 20, m.top + 20), theme.colors.window.argb());
        let classic = draw(
            &Theme::default(),
            &text(),
            &FlyoutLayout::compute(&Theme::default(), 1),
            &[None],
            false,
        );
        assert_eq!(
            (classic.width, classic.height),
            (32, 32),
            "no shadow in Classic"
        );
    }

    #[test]
    fn each_cell_is_a_named_button() {
        let l = FlyoutLayout::compute(&Theme::default(), 2);
        let mut t = Tree::window("Hidden icons", (l.width, l.height));
        access(&l, &["Battery".into(), "Mail".into()], &mut t, access::ROOT);
        assert!(t.find(Role::Button, "Battery").is_some());
        assert!(t.find(Role::Button, "Mail").is_some());
        let l = FlyoutLayout::compute(&glass_theme(), 2);
        let mut t = Tree::window("Hidden icons", (l.width, l.height));
        access(&l, &["Battery".into(), "Mail".into()], &mut t, access::ROOT);
        assert!(t.find(Role::Link, "Customize...").is_some());
    }

    #[test]
    fn a_reader_can_press_each_cell_and_the_link() {
        let l = FlyoutLayout::compute(&glass_theme(), 2);
        let mut t = Tree::window("Hidden icons", (l.width, l.height));
        access(&l, &["Battery".into(), "Mail".into()], &mut t, access::ROOT);
        for (role, name) in [
            (Role::Button, "Battery"),
            (Role::Button, "Mail"),
            (Role::Link, "Customize..."),
        ] {
            let n = t.get(t.find(role, name).unwrap()).unwrap();
            assert!(n.supports_action(Action::Click), "{name} takes a click");
        }
    }
}
