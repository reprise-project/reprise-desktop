//! The file dialog's places bar: the five-tile vertical strip down its
//! left edge (My Recent Documents, Desktop, My Documents, My Computer, My
//! Network Places).
//!
//! The panel and the selected tile's frame are the look's
//! ([`crate::look::widgets::placesbar`]): Classic a sunken `button_shadow`
//! panel with the selected tile inside a one-pixel sunken frame, Blue a light
//! panel in a pale frame, Glass a flat panel with the selected tile sunk into
//! a grey box.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::draw_disabled_label;
use crate::controls::iconview::wrap_label;
use crate::look::Paint;
use crate::metrics::PlacesMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};

pub const WIDTH: i32 = 87;

/// Classic's tile height, and so the pitch from one icon's top to the
/// next. A look sets its own in
/// [`PlacesMetrics::tile`](crate::metrics::PlacesMetrics::tile).
pub const TILE_H: i32 = PlacesMetrics::CLASSIC.tile;

pub const ICON_PX: i32 = 32;

const LABEL_GAP: i32 = 3;

/// The panel's own border thickness on each side, unscaled: the two
/// rings `EdgeStyle::ButtonSunken` draws (`ring_t` at `t` then `t` again
/// on the inset rect), so content starts `2 * s` px inside `r`.
const BORDER: i32 = 2;

/// One tile: its label, the icon the host looked up for it (`None` draws only
/// the label) and whether it has anywhere to go. A tile with `enabled: false`
/// draws its label in `gray_text` and refuses selection and activation.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub label: String,
    pub icon: Option<Canvas>,
    pub enabled: bool,
}

/// The bar's state: its places and which one is selected.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacesBar {
    pub places: Vec<Place>,
    pub selected: Option<usize>,
}

/// Each tile's rect inside `r`, stacked top to bottom. The panel's border
/// belongs to `draw`, so a click anywhere in a tile's band selects it.
pub fn layout(pb: &PlacesBar, r: Rect, pm: &PlacesMetrics) -> Vec<Rect> {
    (0..pb.places.len())
        .map(|i| Rect::new(r.x, r.y + i as i32 * pm.tile, r.width, pm.tile))
        .collect()
}

/// The tile under `(x, y)`, or `None` below the last one.
pub fn hit(rects: &[Rect], x: i32, y: i32) -> Option<usize> {
    rects.iter().position(|r| r.contains(x, y))
}

/// The bar: the look's panel, then each tile's icon and up to two centred
/// label lines (`iconview::wrap_label`), and the look's frame on the
/// selected tile.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, pb: &PlacesBar, r: Rect, px: u32) {
    let Paint { cs, s, .. } = *p;
    let pm = p.look.controls().places;
    let sr = r.scaled(s);
    crate::look::widgets::placesbar::draw_panel(c, p, sr);
    let content_x = sr.x + BORDER * s;
    let content_w = sr.width - 2 * BORDER * s;
    let step = px as i32 + 2 * s;
    for (i, place) in pb.places.iter().enumerate() {
        let tile_y = sr.y + i as i32 * pm.tile * s;
        let tile_h = pm.tile * s;
        if pb.selected == Some(i) {
            let band = Rect::new(sr.x, tile_y, sr.width, tile_h);
            crate::look::widgets::placesbar::draw_selected(c, p, band);
        }
        if let Some(icon) = &place.icon {
            let ix = content_x + (content_w - icon.width as i32) / 2;
            let iy = tile_y + pm.icon_top * s;
            c.blit_over(ix, iy, icon);
        }
        let max_w = content_w - 4 * s;
        let lines = wrap_label(text, &place.label, max_w, px);
        let label_y = tile_y + (pm.icon_top + ICON_PX + LABEL_GAP) * s;
        for (j, line) in lines.iter().enumerate() {
            let ly = label_y + j as i32 * step;
            let lr = Rect::new(content_x, ly, content_w, step);
            if place.enabled {
                text.draw(c, lr, line, false, cs.button_text, Align::Center, px);
            } else {
                // Etched: gray_text equals this panel's button_shadow fill.
                draw_disabled_label(c, text, p, lr, line, Align::Center, px);
            }
        }
    }
}

/// The places bar's `access` part: a list of its places at their tiles,
/// the selected one marked. Returns the node that has the focus when the
/// bar does: the selected place, else the bar.
pub fn access(
    pb: &PlacesBar,
    r: Rect,
    pm: &PlacesMetrics,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, r);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for (i, (place, tile)) in pb.places.iter().zip(layout(pb, r, pm)).enumerate() {
        let mut o = access::named(Role::ListBoxOption, tile, &place.label);
        if !place.enabled {
            o.set_disabled();
        }
        o.set_selected(pb.selected == Some(i));
        let oid = tree.push(id, i, o);
        if pb.selected == Some(i) {
            focus = oid;
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    /// A disabled tile etches its label instead of flat-greying it. This
    /// panel fills with `button_shadow`, and the Standard scheme's
    /// `gray_text` is the same (128, 128, 128), so a flat grey label
    /// would disappear into the bar entirely.
    #[test]
    fn a_disabled_tile_etches_its_label() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        assert_eq!(
            cs.gray_text.argb(),
            cs.button_shadow.argb(),
            "the premise: flat grey would be invisible here"
        );
        let mut pb = sample();
        pb.places[0].enabled = false;
        let r = Rect::new(0, 0, WIDTH, 5 * TILE_H);
        let mut c = Canvas::new(WIDTH as u32, (5 * TILE_H) as u32);
        draw(&mut c, &text, &classic_paint(cs, 1), &pb, r, px);
        let rects = layout(&pb, r, &PlacesMetrics::CLASSIC);
        // Only the label area each tile actually writes into: the panel's
        // own sunken border is `button_hilight` down the right edge, which
        // would otherwise read as an etched label on every tile.
        let band = |t: Rect| {
            let (mut hilight, mut ink) = (0, 0);
            for y in (t.y + PlacesMetrics::CLASSIC.icon_top + ICON_PX)..t.bottom() {
                for x in (t.x + BORDER)..(t.right() - BORDER) {
                    let p = c.get(x, y);
                    if p == cs.button_hilight.argb() {
                        hilight += 1;
                    } else if p == cs.button_text.argb() {
                        ink += 1;
                    }
                }
            }
            (hilight, ink)
        };
        let (disabled_hilight, disabled_ink) = band(rects[0]);
        let (enabled_hilight, enabled_ink) = band(rects[1]);
        assert!(
            disabled_hilight > 0,
            "the disabled label's white copy is what makes it readable"
        );
        assert_eq!(disabled_ink, 0, "and none of it is drawn in button_text");
        assert!(enabled_ink > 0, "an enabled tile draws its label normally");
        assert_eq!(enabled_hilight, 0, "and needs no white copy");
    }

    #[test]
    fn tiles_stack_down_the_bar() {
        const EXPECTED_TILE_H: i32 = 65;
        const EXPECTED_WIDTH: i32 = 87;
        let pb = sample();
        let r = Rect::new(0, 0, EXPECTED_WIDTH, 5 * EXPECTED_TILE_H);
        let rects = layout(&pb, r, &PlacesMetrics::CLASSIC);
        assert_eq!(rects.len(), 5);
        assert_eq!(rects[0].height, EXPECTED_TILE_H);
        assert_eq!(rects[1].y, rects[0].bottom());
        assert_eq!(rects[4].bottom(), r.bottom());
    }

    /// The selected tile draws the one-px sunken frame around its full content
    /// width; an unselected tile leaves the bar's `button_shadow` fill untouched.
    #[test]
    fn the_selected_tile_is_sunken() {
        const EXPECTED_BORDER: i32 = 2;
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut pb = sample();
        pb.selected = Some(2);
        let r = Rect::new(0, 0, WIDTH, 5 * TILE_H);
        let mut c = Canvas::new(WIDTH as u32, (5 * TILE_H) as u32);
        draw(&mut c, &text, &classic_paint(cs, 1), &pb, r, px);
        let rects = layout(&pb, r, &PlacesMetrics::CLASSIC);
        let t = rects[2];
        assert_eq!(
            c.get(t.x + EXPECTED_BORDER, t.y),
            cs.button_dk_shadow.argb(),
            "the frame's top-left corner"
        );
        assert_eq!(
            c.get(t.right() - EXPECTED_BORDER - 1, t.bottom() - 1),
            cs.button_hilight.argb(),
            "the frame's bottom-right corner"
        );
        let u = rects[0];
        assert_eq!(
            c.get(u.x + EXPECTED_BORDER, u.y),
            cs.button_shadow.argb(),
            "an unselected tile stays plain fill"
        );
    }

    #[test]
    fn hit_picks_the_tile() {
        let pb = sample();
        let r = Rect::new(0, 0, WIDTH, 5 * TILE_H);
        let rects = layout(&pb, r, &PlacesMetrics::CLASSIC);
        assert_eq!(hit(&rects, 4, rects[0].y + 2), Some(0));
        assert_eq!(hit(&rects, 4, rects[3].y + 2), Some(3));
        assert_eq!(hit(&rects, 4, r.bottom() + 5), None);
    }

    /// Under Blue a tile is 71 px and its icon sits 5 px down, so the
    /// second tile's icon starts 76 px below the bar's top.
    #[test]
    fn blues_tiles_are_71_px_with_the_icon_5_px_down() {
        use crate::theme::Color;
        const ICON: Color = Color::rgb(1, 2, 3);
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let mut pb = sample();
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(icon.bounds(), ICON);
        for p in &mut pb.places {
            p.icon = Some(icon.clone());
        }
        let r = Rect::new(0, 0, WIDTH, 338);
        let pm = theme.look.controls().places;
        let rects = layout(&pb, r, &pm);
        assert_eq!(rects[1], Rect::new(0, 71, WIDTH, 71));
        let mut c = Canvas::new(WIDTH as u32, 338);
        draw(&mut c, &text, &theme.paint(), &pb, r, theme.fonts.size_px);
        let x = WIDTH / 2;
        for (y, want) in [(4, false), (5, true), (36, true), (37, false)] {
            for tile in [0, 1, 4] {
                let y = tile * 71 + y;
                assert_eq!(c.get(x, y) == ICON.argb(), want, "tile {tile} row {y}");
            }
        }
    }

    /// Writes target/placesbar-blue-preview.png: the five places, no
    /// icons, 338 px tall.
    #[test]
    #[ignore]
    fn render_placesbar_blue() {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new(WIDTH as u32, 338);
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &sample(),
            Rect::new(0, 0, WIDTH, 338),
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "placesbar-blue-preview.png");
    }

    fn sample() -> PlacesBar {
        PlacesBar {
            places: [
                "My Recent Documents",
                "Desktop",
                "My Documents",
                "My Computer",
                "My Network Places",
            ]
            .iter()
            .map(|l| Place {
                label: (*l).into(),
                icon: None,
                enabled: true,
            })
            .collect(),
            selected: None,
        }
    }
}
