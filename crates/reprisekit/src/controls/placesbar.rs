//! The file dialog's places bar: the five-tile vertical strip down its
//! left edge (My Recent Documents, Desktop, My Documents, My Computer, My
//! Network Places).
//!
//! The panel and the tiles' frames are the look's
//! ([`crate::look::widgets::placesbar`]): Classic a sunken `button_shadow`
//! panel with the selected tile inside a one-pixel sunken frame, Blue a light
//! panel in a pale frame, Glass a flat panel whose tiles light, select and
//! sink in boxes of their own.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::draw_disabled_label;
use crate::classic::widgets::{BarHit, ScrollParts, scrollbar_parts};
use crate::controls::iconview::wrap_label;
use crate::look::Paint;
use crate::metrics::{ControlMetrics, PlacesMetrics};
use crate::raster::{Canvas, Rect};
use crate::style::State;
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};

pub const WIDTH: i32 = 87;

/// Classic's tile height, and so the pitch from one icon's top to the
/// next. A look sets its own in
/// [`PlacesMetrics::tile`](crate::metrics::PlacesMetrics::tile).
pub const TILE_H: i32 = PlacesMetrics::CLASSIC.tile;

/// An icon's side at 100%; [`PlacesMetrics::icon`] at the text size.
pub const ICON_PX: i32 = PlacesMetrics::CLASSIC.icon;

/// The panel's own border thickness on each side, unscaled: the two
/// rings `EdgeStyle::ButtonSunken` draws (`ring_t` at `t` then `t` again
/// on the inset rect), so content starts `2 * s` px inside `r`.
const BORDER: i32 = 2;

/// One tile: its label, the icon the host looked up for it (`None` draws only
/// the label) and whether it has anywhere to go. A tile with `enabled: false`
/// draws its label in `gray_text` and refuses selection and activation. A
/// `pinned` tile is a folder the user pinned, which has its own menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub label: String,
    pub icon: Option<Canvas>,
    pub enabled: bool,
    pub pinned: bool,
}

/// The bar's state: its places, which one is selected, which one is under the pointer, which
/// one the held button went down on, and the first tile in view when they overflow the bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacesBar {
    pub places: Vec<Place>,
    pub selected: Option<usize>,
    pub hot: Option<usize>,
    pub pressed: Option<usize>,
    pub top: usize,
}

/// The tiles' metrics fitted to `r` ([`PlacesMetrics::fit`]), the scrollbar down its right side
/// inside the border when pinned tiles make them overflow it, and the first tile in view:
/// [`PlacesBar::top`] held to the tiles there are now, 0 without a scrollbar.
fn fitted(pb: &PlacesBar, r: Rect, cm: &ControlMetrics) -> (PlacesMetrics, Option<Rect>, usize) {
    let pm = cm.places.fit(pb.places.len(), r.height);
    let pinned = pb.places.iter().any(|p| p.pinned);
    let over = pinned && pb.places.len() as i32 * pm.tile > r.height;
    let bar = over.then(|| {
        Rect::new(
            r.right() - BORDER - cm.scroll,
            r.y + BORDER,
            cm.scroll,
            r.height - 2 * BORDER,
        )
    });
    let top = pb.top.min(last_top(pb, &pm, bar, r));
    (pm, bar, top)
}

/// The furthest [`PlacesBar::top`] goes: the last tile ends the view.
fn last_top(pb: &PlacesBar, pm: &PlacesMetrics, bar: Option<Rect>, r: Rect) -> usize {
    match bar {
        Some(_) => pb.places.len().saturating_sub(in_view(pm, r)),
        None => 0,
    }
}

/// How many whole tiles show in `r`, at least one.
fn in_view(pm: &PlacesMetrics, r: Rect) -> usize {
    (r.height / pm.tile.max(1)).max(1) as usize
}

impl PlacesBar {
    /// Tile `i`'s look: held, then selected (lit or not), then lit; `None` for a plain tile.
    pub fn state(&self, i: usize) -> Option<State> {
        let (selected, hot) = (self.selected == Some(i), self.hot == Some(i));
        match () {
            _ if self.pressed == Some(i) => Some(State::Pressed),
            _ if selected && hot => Some(State::CheckedHot),
            _ if selected => Some(State::Checked),
            _ if hot => Some(State::Hot),
            _ => None,
        }
    }

    /// Lights tile `i` (`None` for none). True when that changed.
    pub fn hover(&mut self, i: Option<usize>) -> bool {
        let i = i.filter(|&i| self.places.get(i).is_some_and(|p| p.enabled));
        std::mem::replace(&mut self.hot, i) != i
    }

    /// The first tile in view moved to `top`, held to the tiles there are; true when it moved.
    fn scroll_to(&mut self, top: isize, r: Rect, cm: &ControlMetrics) -> bool {
        let (pm, bar, _) = fitted(self, r, cm);
        let most = last_top(self, &pm, bar, r);
        let top = (top.max(0) as usize).min(most);
        std::mem::replace(&mut self.top, top) != top
    }

    /// Scrolls `by` tiles (down is positive); true when the view moved.
    pub fn scroll(&mut self, by: isize, r: Rect, cm: &ControlMetrics) -> bool {
        let top = fitted(self, r, cm).2;
        self.scroll_to(top as isize + by, r, cm)
    }

    /// Scrolls as little as it takes for tile `i` to show whole; true when the view moved.
    pub fn reveal(&mut self, i: usize, r: Rect, cm: &ControlMetrics) -> bool {
        let (pm, _, now) = fitted(self, r, cm);
        let n = in_view(&pm, r);
        let top = if i < now {
            i
        } else if i >= now + n {
            i + 1 - n
        } else {
            now
        };
        self.scroll_to(top as isize, r, cm)
    }

    /// Holds `top` inside the tiles after they or `r` changed.
    pub fn clamp(&mut self, r: Rect, cm: &ControlMetrics) {
        self.scroll_to(self.top as isize, r, cm);
    }

    /// A press on part `hit` of the scrollbar: an arrow steps a tile, the track a view.
    pub fn bar_press(&mut self, hit: BarHit, r: Rect, cm: &ControlMetrics) -> bool {
        let (pm, _, _) = fitted(self, r, cm);
        let page = in_view(&pm, r) as isize;
        let by = match hit {
            BarHit::Back => -1,
            BarHit::Forward => 1,
            BarHit::PageBack => -page,
            BarHit::PageForward => page,
            BarHit::Thumb(_) | BarHit::Track => 0,
        };
        self.scroll(by, r, cm)
    }
}

/// The scrollbar's parts when the tiles overflow `r`.
pub fn bar_parts(pb: &PlacesBar, r: Rect, cm: &ControlMetrics) -> Option<ScrollParts> {
    let (pm, bar, top) = fitted(pb, r, cm);
    let n = pb.places.len();
    Some(scrollbar_parts(
        bar?,
        1,
        top,
        in_view(&pm, r),
        n,
        false,
        cm.thumb_min,
    ))
}

/// Each tile's rect inside `r`, stacked top to bottom from [`PlacesBar::top`], shrunk to fit it
/// ([`PlacesMetrics::fit`]) and cut to `r`, so a tile out of view is empty. The panel's border
/// belongs to `draw`, so a click anywhere in a tile's band selects it.
pub fn layout(pb: &PlacesBar, r: Rect, cm: &ControlMetrics) -> Vec<Rect> {
    let (pm, bar, top) = fitted(pb, r, cm);
    let w = bar.map_or(r.width, |b| b.x - r.x);
    (0..pb.places.len())
        .map(|i| {
            let y = r.y + (i as i32 - top as i32) * pm.tile;
            Rect::new(r.x, y, w, pm.tile).intersect(&r)
        })
        .collect()
}

/// The tile under `(x, y)`, or `None` below the last one.
pub fn hit(rects: &[Rect], x: i32, y: i32) -> Option<usize> {
    rects.iter().position(|r| r.contains(x, y))
}

/// The bar: the look's panel, then each tile in view: its frame for its [`PlacesBar::state`],
/// icon and up to two centred label lines (`iconview::wrap_label`), and the scrollbar when they
/// overflow. An icon larger than a shrunk tile's draws resampled to it.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, pb: &PlacesBar, r: Rect, px: u32) {
    let Paint { cs, s, .. } = *p;
    let cm = p.look.controls().grown(p.text);
    let (pm, bar, top) = fitted(pb, r, &cm);
    let sr = r.scaled(s);
    crate::look::widgets::placesbar::draw_panel(c, p, sr);
    let content_x = sr.x + BORDER * s;
    let content_w = sr.width - 2 * BORDER * s - bar.map_or(0, |b| b.width * s);
    let band_w = sr.width - bar.map_or(0, |b| (b.width + BORDER) * s);
    let step = px as i32 + 2 * s;
    let stride = px as i32 + pm.line_lead * s;
    let inner = Rect::new(
        sr.x,
        sr.y + BORDER * s,
        sr.width,
        sr.height - 2 * BORDER * s,
    );
    let tiles = |c: &mut Canvas| {
        for (i, place) in pb.places.iter().enumerate().skip(top) {
            let tile_y = sr.y + (i - top) as i32 * pm.tile * s;
            if tile_y >= sr.bottom() {
                break;
            }
            let tile_h = pm.tile * s;
            if let Some(state) = pb.state(i) {
                let band = Rect::new(sr.x, tile_y, band_w, tile_h);
                crate::look::widgets::placesbar::draw_tile(c, p, band, state);
            }
            if let Some(icon) = &place.icon {
                let side = pm.icon * s;
                let shrunk = (icon.device_size().0 as i32 > side).then(|| {
                    let w = (side / icon.scale()) as u32;
                    icon.resample_box(w, w)
                });
                let icon = shrunk.as_ref().unwrap_or(icon);
                let ix = content_x + (content_w - icon.width as i32) / 2;
                let iy = tile_y + pm.icon_top * s;
                c.blit_over(ix, iy, icon);
            }
            let max_w = content_w - 4 * s;
            let lines = wrap_label(text, &place.label, max_w, px);
            let label_y = tile_y + (pm.icon_top + pm.icon + pm.label_gap) * s;
            for (j, line) in lines.iter().enumerate() {
                let ly = label_y + j as i32 * stride;
                let lr = Rect::new(content_x, ly, content_w, step);
                if place.enabled {
                    text.draw(c, lr, line, false, cs.button_text, Align::Center, px);
                } else {
                    // Etched: gray_text equals this panel's button_shadow fill.
                    draw_disabled_label(c, text, p, lr, line, Align::Center, px);
                }
            }
        }
    };
    // Only a scrolled bar cuts its tiles at the border: a fitted one's last band may touch it.
    match bar {
        Some(_) => c.with_clip(inner, tiles),
        None => tiles(c),
    }
    if let Some(bar) = bar {
        let n = pb.places.len();
        let shown = in_view(&pm, r);
        crate::look::widgets::draw_scrollbar(c, p, bar.scaled(s), top, shown, n, None);
    }
}

/// How far the label lines reaching furthest run past their tile's bottom in `r` with text at
/// `px`, at 1x, as `draw` sets them: for an audit.
pub fn overflow(pb: &PlacesBar, text: &TextRenderer, r: Rect, cm: &ControlMetrics, px: u32) -> i32 {
    let (pm, bar, _) = fitted(pb, r, cm);
    let max_w = r.width - 2 * BORDER - 4 - bar.map_or(0, |b| b.width);
    let step = px as i32 + 2;
    let stride = px as i32 + pm.line_lead;
    pb.places
        .iter()
        .map(|place| {
            let lines = wrap_label(text, &place.label, max_w, px).len() as i32;
            let label = (lines - 1) * stride + step;
            pm.icon_top + pm.icon + pm.label_gap + label - pm.tile
        })
        .max()
        .unwrap_or(0)
}

/// The places bar's `access` part: a list of its places at their tiles,
/// the selected one marked. Returns the node that has the focus when the
/// bar does: the selected place, else the bar.
pub fn access(
    pb: &PlacesBar,
    r: Rect,
    cm: &ControlMetrics,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, r);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for (i, (place, tile)) in pb.places.iter().zip(layout(pb, r, cm)).enumerate() {
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
        let rects = layout(&pb, r, &ControlMetrics::CLASSIC);
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
        let rects = layout(&pb, r, &ControlMetrics::CLASSIC);
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
        let rects = layout(&pb, r, &ControlMetrics::CLASSIC);
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
        let rects = layout(&pb, r, &ControlMetrics::CLASSIC);
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
        let rects = layout(&pb, r, &theme.look.controls());
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

    #[test]
    fn a_tile_shows_held_then_selected_then_lit() {
        let mut pb = sample();
        pb.places[4].enabled = false;
        assert_eq!(pb.state(0), None);
        assert!(pb.hover(Some(0)), "lighting a place is a change");
        assert!(!pb.hover(Some(0)), "lighting it again is not");
        assert_eq!(pb.state(0), Some(State::Hot));
        pb.selected = Some(0);
        assert_eq!(pb.state(0), Some(State::CheckedHot));
        assert!(pb.hover(Some(1)));
        assert_eq!(pb.state(0), Some(State::Checked));
        assert_eq!(pb.state(1), Some(State::Hot));
        pb.pressed = Some(1);
        assert_eq!(pb.state(1), Some(State::Pressed));
        assert!(pb.hover(Some(4)), "a greyed place does not light");
        assert_eq!(pb.hot, None);
        assert!(!pb.hover(None));
    }

    /// Classic frames the selected tile only: a lit one stays plain.
    #[test]
    fn classic_leaves_a_lit_tile_plain() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let r = Rect::new(0, 0, WIDTH, 5 * TILE_H);
        let draw_with = |hot| {
            let mut c = Canvas::new(WIDTH as u32, (5 * TILE_H) as u32);
            let pb = PlacesBar { hot, ..sample() };
            draw(&mut c, &text, &classic_paint(&theme.colors, 1), &pb, r, 11);
            c
        };
        assert_eq!(draw_with(Some(1)).pixels, draw_with(None).pixels);
    }

    /// Under Glass a tile is 59 px, its icon 2 px down and its label's first line 1 px under
    /// the icon.
    #[test]
    fn glass_tiles_are_59_px_with_the_icon_2_px_down() {
        use crate::theme::Color;
        const ICON: Color = Color::rgb(1, 2, 3);
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let cm = theme.look.controls();
        let pm = cm.places;
        assert_eq!((pm.tile, pm.icon_top, pm.label_gap), (59, 2, 1));
        let text = TextRenderer::load(&theme.fonts);
        let mut pb = sample();
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(icon.bounds(), ICON);
        for p in &mut pb.places {
            p.icon = Some(icon.clone());
        }
        let r = Rect::new(0, 0, WIDTH, 5 * 59);
        assert_eq!(layout(&pb, r, &cm)[1], Rect::new(0, 59, WIDTH, 59));
        let px = theme.fonts.size_px;
        assert!(overflow(&pb, &text, r, &cm, px) <= 0, "two-line labels fit");
        let mut c = Canvas::new(WIDTH as u32, (5 * 59) as u32);
        draw(&mut c, &text, &theme.paint(), &pb, r, px);
        let x = WIDTH / 2;
        for (y, want) in [(1, false), (2, true), (33, true), (34, false)] {
            for tile in [0, 1, 4] {
                let y = tile * 59 + y;
                assert_eq!(c.get(x, y) == ICON.argb(), want, "tile {tile} row {y}");
            }
        }
    }

    /// At 200% text a Classic tile is 3 + 2 * 62 px with a 64 px icon 3 px down, and the label
    /// starts under that icon.
    #[test]
    fn at_200_percent_the_icon_grows_and_the_label_sits_under_it() {
        use crate::theme::{Color, TextUnits};
        const ICON: Color = Color::rgb(1, 2, 3);
        let u = TextUnits::new(22, 11);
        let theme = Theme::default();
        let p = Paint {
            text: u,
            ..classic_paint(&theme.colors, 1)
        };
        let cm = p.look.controls().grown(u);
        let pm = cm.places;
        assert_eq!((pm.tile, pm.icon), (127, 64));
        let text = TextRenderer::load(&FontConfig::default());
        let mut pb = sample();
        let mut icon = Canvas::new(64, 64);
        icon.fill_rect(icon.bounds(), ICON);
        for place in &mut pb.places {
            place.icon = Some(icon.clone());
        }
        let (w, h) = (2 * WIDTH, 5 * pm.tile);
        let r = Rect::new(0, 0, w, h);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &p, &pb, r, 22);
        let x = w / 2;
        for (y, want) in [(2, false), (3, true), (66, true), (67, false)] {
            assert_eq!(c.get(x, 127 + y) == ICON.argb(), want, "row {y}");
        }
        let ink = |y: i32| (0..w).any(|x| c.get(x, y) == theme.colors.button_text.argb());
        let label_top = (127..2 * 127)
            .find(|&y| ink(y))
            .expect("the label is drawn");
        assert!(
            label_top >= 127 + 3 + 64 + 3,
            "the label starts at {label_top}, under the icon"
        );
        assert!(
            overflow(&pb, &text, r, &cm, 22) <= 0,
            "the labels fit their tiles"
        );

        let squeezed = PlacesMetrics {
            tile: 100,
            icon: 37,
            ..pm
        };
        assert_eq!(pm.fit(5, 500), squeezed, "five tiles in 500 px");
        assert_eq!(pm.fit(5, 10).icon, 32, "never under 32 px");
        let mut c = Canvas::new(w as u32, 500);
        draw(&mut c, &text, &p, &pb, Rect::new(0, 0, w, 500), 22);
        let red = (0..w).filter(|&x| c.get(x, 100 + 3) == ICON.argb()).count();
        assert_eq!(red, 37, "a squeezed tile draws its icon at 37 px");
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

    /// Writes target/placesbar-glass-preview.png: the five places with icons, the first lit,
    /// the third selected and the fourth held.
    #[test]
    #[ignore]
    fn render_placesbar_glass() {
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(
            Rect::new(3, 5, 26, 22),
            crate::theme::Color::rgb(0xE8, 0xC0, 0x50),
        );
        let mut pb = PlacesBar {
            selected: Some(2),
            hot: Some(0),
            pressed: Some(3),
            ..sample()
        };
        for p in &mut pb.places {
            p.icon = Some(icon.clone());
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let text = TextRenderer::load(&theme.fonts);
        let r = Rect::new(0, 0, WIDTH, 5 * 59);
        let mut out = Canvas::new(WIDTH as u32, r.height as u32);
        draw(&mut out, &text, &theme.paint(), &pb, r, theme.fonts.size_px);
        crate::raster::write_preview(&out, "placesbar-glass-preview.png");
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
                pinned: false,
            })
            .collect(),
            ..PlacesBar::default()
        }
    }

    /// The six built-in tiles and `pins` pinned folders, the last pinned one missing.
    fn with_pins(pins: usize) -> PlacesBar {
        let mut pb = sample();
        pb.places.push(Place {
            label: "Computer".into(),
            enabled: true,
            ..Place::default()
        });
        for i in 0..pins {
            pb.places.push(Place {
                label: format!("Pin {i}"),
                enabled: i + 1 < pins,
                pinned: true,
                ..Place::default()
            });
        }
        pb
    }

    const BAR: Rect = Rect::new(0, 0, WIDTH, 403);

    #[test]
    fn pins_past_the_built_in_tiles_scroll_under_a_bar() {
        let cm = ControlMetrics::CLASSIC;
        let mut pb = with_pins(4);
        let rects = layout(&pb, BAR, &cm);
        let bar_x = WIDTH - BORDER - cm.scroll;
        assert_eq!(
            rects[0],
            Rect::new(0, 0, bar_x, 65),
            "tiles stop at the bar"
        );
        assert_eq!(
            rects[6],
            Rect::new(0, 390, bar_x, 13),
            "the seventh shows cut"
        );
        assert!(rects[7].is_empty(), "the eighth is out of view");
        let parts = bar_parts(&pb, BAR, &cm).expect("a scrollbar");
        assert_eq!(parts.back.x, bar_x);

        assert!(pb.scroll(1, BAR, &cm));
        assert_eq!(pb.top, 1);
        assert!(layout(&pb, BAR, &cm)[0].is_empty());
        assert!(pb.scroll(100, BAR, &cm));
        assert_eq!(pb.top, 4, "ten tiles, six in view");
        assert!(!pb.scroll(1, BAR, &cm));
        assert!(pb.reveal(0, BAR, &cm));
        assert_eq!(pb.top, 0);
        assert!(pb.reveal(8, BAR, &cm));
        assert_eq!(pb.top, 3, "the ninth tile ends the view");
        assert!(pb.bar_press(BarHit::PageBack, BAR, &cm));
        assert_eq!(pb.top, 0);
        assert!(pb.bar_press(BarHit::Forward, BAR, &cm));
        assert_eq!(pb.top, 1);

        pb.places.truncate(6);
        assert_eq!(
            layout(&pb, BAR, &cm)[0],
            Rect::new(0, 0, WIDTH, 65),
            "unpinned down to the built-in tiles, the bar shows from the first again"
        );
        pb.clamp(BAR, &cm);
        assert_eq!(pb.top, 0, "nothing to scroll once they fit");
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let blue = blue.look.controls();
        assert!(6 * blue.places.tile > 403);
        assert!(
            bar_parts(&pb, BAR, &blue).is_none(),
            "the built-in tiles alone never scroll"
        );
        assert!(bar_parts(&pb, BAR, &cm).is_none());
        assert_eq!(layout(&pb, BAR, &cm)[0].width, WIDTH);
    }

    /// FNV-1a over the pixels.
    fn hash(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in &c.pixels {
            for b in p.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    /// A bar of nine tiles drawn under each interface at 1x and 2x, scrolled to its end with a
    /// pinned tile selected. Labels are left blank so the pins hold under any font.
    #[test]
    fn a_scrolled_bar_draws_its_tiles_and_scrollbar_in_every_interface() {
        use crate::interface::Interface;
        use crate::theme::Color;
        const ICON: Color = Color::rgb(0xE8, 0xC0, 0x50);
        let want = [
            (Interface::Classic, 1, 13370575589722211660),
            (Interface::Classic, 2, 13839850585171954949),
            (Interface::Blue, 1, 15596967453792755634),
            (Interface::Blue, 2, 13641811894776302826),
            (Interface::Glass, 1, 891772286779187367),
            (Interface::Glass, 2, 11945213783426164471),
        ];
        let mut got = Vec::new();
        for (i, scale, _) in want {
            let theme = crate::interfaces::theme_for(i).scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            let cm = p.look.controls().grown(p.text);
            let mut pb = with_pins(3);
            let mut icon = Canvas::new((ICON_PX * scale) as u32, (ICON_PX * scale) as u32);
            icon.fill_rect(icon.bounds(), ICON);
            for place in &mut pb.places {
                place.label.clear();
                place.icon = Some(icon.clone());
            }
            pb.selected = Some(7);
            pb.scroll(100, BAR, &cm);
            assert_eq!(pb.top, 9 - (403 / cm.places.tile) as usize, "{i:?}@{scale}");
            let mut c = Canvas::new((WIDTH * scale) as u32, (403 * scale) as u32);
            draw(&mut c, &text, &p, &pb, BAR, theme.fonts.size_px);
            let bar = bar_parts(&pb, BAR, &cm).expect("a scrollbar");
            let thumb = bar.thumb.expect("a thumb");
            assert!(
                thumb.y > bar.track.y,
                "{i:?}@{scale}: scrolled down, the thumb is low"
            );
            let tile = layout(&pb, BAR, &cm)[pb.top];
            assert_eq!(tile.y, 0, "{i:?}@{scale}");
            let ink = c.get((tile.x + tile.width / 2) * scale, (tile.y + 10) * scale);
            assert_eq!(
                ink,
                ICON.argb(),
                "{i:?}@{scale}: the first tile in view tops it"
            );
            got.push((i, scale, hash(&c)));
        }
        let want: Vec<_> = want.to_vec();
        assert_eq!(got, want);
    }

    /// Writes target/placesbar-pinned-preview.png: nine tiles with labels under Classic, Blue
    /// and Glass, scrolled one tile down, a pinned tile selected and the last one greyed.
    #[test]
    #[ignore]
    fn render_placesbar_pinned() {
        use crate::interface::Interface;
        let mut out = Canvas::new((3 * WIDTH + 20) as u32, 403);
        for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
            .into_iter()
            .enumerate()
        {
            let theme = crate::interfaces::theme_for(i);
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            let cm = p.look.controls().grown(p.text);
            let mut pb = with_pins(3);
            let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
            icon.fill_rect(
                Rect::new(3, 5, 26, 22),
                crate::theme::Color::rgb(0xE8, 0xC0, 0x50),
            );
            for place in &mut pb.places {
                place.icon = Some(icon.clone());
            }
            pb.selected = Some(6);
            pb.scroll(1, BAR, &cm);
            let r = Rect::new(k as i32 * (WIDTH + 10), 0, WIDTH, 403);
            draw(&mut out, &text, &p, &pb, r, theme.fonts.size_px);
        }
        crate::raster::write_preview(&out, "placesbar-pinned-preview.png");
    }
}
