//! The display switcher (Super+P): its model, access part and Classic's and Blue's painter, a tile
//! per [`Projection::ALL`] choice. The geometry and pictures are the root [`crate::projector`],
//! re-exported here.

pub use crate::projector::*;

use crate::access::accesskit::{NodeId, Role, Toggled};
use crate::access::{self, Tree};
use crate::flyout;
use crate::projection::Projection;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Next,
    Prev,
    Enter,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Pick(Projection),
    Close,
}

/// The lit tile (the keys' choice), the hot one (under the pointer) and which can be picked, by
/// index into [`Projection::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Projector {
    pub lit: usize,
    pub hot: Option<usize>,
    pub enabled: [bool; 4],
}

impl Projector {
    /// Lit on `current` (the first tile when it has none), the choices `connected` displays allow
    /// enabled.
    pub fn new(current: Projection, connected: usize) -> Projector {
        Projector {
            lit: current.index().unwrap_or(0),
            hot: None,
            enabled: Projection::ALL.map(|p| p.available(connected)),
        }
    }

    pub fn choice(&self) -> Projection {
        Projection::ALL[self.lit]
    }

    /// Next and Prev light the next or previous enabled tile, round past the ends; Enter picks
    /// the lit tile when it is enabled, Escape closes.
    pub fn key(&mut self, key: Key) -> Event {
        let n = self.enabled.len();
        let step = match key {
            Key::Escape => return Event::Close,
            Key::Enter if self.enabled[self.lit] => return Event::Pick(self.choice()),
            Key::Enter => return Event::None,
            Key::Next => 1,
            Key::Prev => n - 1,
        };
        let next = (1..=n)
            .map(|k| (self.lit + k * step) % n)
            .find(|&i| self.enabled[i]);
        match next {
            Some(i) if i != self.lit => {
                self.lit = i;
                Event::Redraw
            }
            _ => Event::None,
        }
    }

    /// The pointer is on `tile` (None: off every tile); a greyed tile takes no hot frame.
    pub fn hover(&mut self, tile: Option<usize>) -> Event {
        let tile = tile.filter(|&i| self.enabled[i]);
        if tile == self.hot {
            return Event::None;
        }
        self.hot = tile;
        Event::Redraw
    }

    pub fn click(&self, tile: Option<usize>) -> Event {
        match tile {
            Some(i) if self.enabled[i] => Event::Pick(Projection::ALL[i]),
            _ => Event::None,
        }
    }
}

/// Classic's and Blue's switcher, `layout` in device pixels at `theme.scale`, captions in `text`:
/// the look's flyout panel, each tile's picture over its caption, the lit tile ringed 2 px in
/// `hilight`, the hot one 1 px; a greyed tile in `gray_text` with its screens dark.
pub fn draw(theme: &Theme, text: &TextRenderer, layout: &ProjectorLayout, p: &Projector) -> Canvas {
    let cs = &theme.colors;
    let s = layout.scale.max(1);
    let size = theme.fonts.size_px;
    let mut c = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
    let all = Rect::new(0, 0, layout.width, layout.height);
    flyout::draw_panel(theme, &mut c, all, cs.button_face, flyout::Backdrop::None);
    for (i, choice) in Projection::ALL.into_iter().enumerate() {
        let (ink, caption_ink) = if p.enabled[i] {
            let ink = Ink {
                frame: cs.button_text,
                screen: Some(cs.background),
                mark: cs.window,
            };
            (ink, cs.button_text)
        } else {
            let ink = Ink {
                frame: cs.gray_text,
                screen: None,
                mark: cs.gray_text,
            };
            (ink, cs.gray_text)
        };
        draw_picture(&mut c, text, size, layout.picture(i), choice, &ink, s);
        let caption = layout.caption(i);
        let label = choice.label();
        let label = text.ellipsize(&label, caption.width, false, size);
        text.draw(
            &mut c,
            caption,
            &label,
            false,
            caption_ink,
            Align::Center,
            size,
        );
        let ring = if i == p.lit {
            2
        } else if p.hot == Some(i) {
            1
        } else {
            0
        };
        if ring > 0 {
            c.ring_t(layout.tiles[i], cs.hilight, cs.hilight, ring * s);
        }
    }
    c
}

/// The switcher's access part: a radio button per tile named by its caption, the lit one checked
/// and selected, a greyed one disabled. Returns the lit tile's node.
pub fn access(layout: &ProjectorLayout, p: &Projector, tree: &mut Tree, parent: NodeId) -> NodeId {
    let mut lit = parent;
    for (i, (choice, tile)) in Projection::ALL.into_iter().zip(layout.tiles).enumerate() {
        let on = i == p.lit;
        let label = choice.label();
        let mut node = access::toggle(
            Role::RadioButton,
            tile,
            &label,
            Toggled::from(on),
            p.enabled[i],
        );
        node.set_selected(on);
        let id = tree.push(parent, i, node);
        if on {
            lit = id;
        }
    }
    lit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::{draw_projector, theme_for};
    use crate::theme::{Color, FontConfig};

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn any_px(c: &Canvas, r: Rect, f: impl Fn(u32) -> bool) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| f(c.get(x, y))))
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn it_opens_lit_on_the_current_choice_with_the_unreachable_ones_greyed() {
        let p = Projector::new(Projection::Only(1), 2);
        assert_eq!((p.lit, p.hot, p.enabled), (3, None, [true; 4]));
        assert_eq!(p.choice(), Projection::Only(1));
        let one = Projector::new(Projection::Only(0), 1);
        assert_eq!(one.enabled, [false, false, true, false]);
        assert_eq!(Projector::new(Projection::Only(4), 5).lit, 0, "no tile");
    }

    #[test]
    fn next_and_prev_skip_greyed_tiles_and_wrap() {
        let mut p = Projector {
            lit: 2,
            hot: None,
            enabled: [true, false, true, false],
        };
        assert_eq!(p.key(Key::Next), Event::Redraw);
        assert_eq!(p.lit, 0, "past the greyed Only 2, round to Extend");
        p.key(Key::Next);
        assert_eq!(p.lit, 2, "past the greyed Duplicate");
        p.key(Key::Prev);
        assert_eq!(p.lit, 0);
        p.key(Key::Prev);
        assert_eq!(p.lit, 2, "round from the first back to the last enabled");
        let mut alone = Projector::new(Projection::Only(0), 1);
        assert_eq!(alone.key(Key::Next), Event::None, "the only enabled tile");
    }

    #[test]
    fn enter_picks_the_lit_tile_unless_greyed_and_escape_closes() {
        let mut p = Projector::new(Projection::Duplicate, 2);
        assert_eq!(p.key(Key::Enter), Event::Pick(Projection::Duplicate));
        let mut stale = Projector::new(Projection::Extend, 1);
        assert_eq!(
            stale.key(Key::Enter),
            Event::None,
            "Extend with one display"
        );
        stale.key(Key::Next);
        assert_eq!(stale.key(Key::Enter), Event::Pick(Projection::Only(0)));
        assert_eq!(stale.key(Key::Escape), Event::Close);
    }

    #[test]
    fn the_pointer_frames_an_enabled_tile_and_a_click_picks_it() {
        let mut p = Projector::new(Projection::Only(0), 1);
        assert_eq!(p.hover(Some(2)), Event::Redraw);
        assert_eq!(p.hover(Some(2)), Event::None);
        assert_eq!(p.hover(Some(0)), Event::Redraw, "off onto a greyed tile");
        assert_eq!(p.hot, None);
        assert_eq!(p.hover(None), Event::None);
        assert_eq!(p.click(Some(2)), Event::Pick(Projection::Only(0)));
        assert_eq!(p.click(Some(3)), Event::None, "greyed");
        assert_eq!(p.click(None), Event::None);
        assert_eq!(p.lit, 2, "the pointer leaves the lit tile alone");
    }

    #[test]
    fn classic_rings_the_lit_tile_2_px_and_the_hot_one_1_px() {
        let t = Theme::default();
        let cs = &t.colors;
        let l = ProjectorLayout::compute();
        let p = Projector {
            hot: Some(2),
            ..Projector::new(Projection::Extend, 2)
        };
        let c = draw(&t, &text(), &l, &p);
        assert_eq!((c.width as i32, c.height as i32), (432, 102));
        assert_eq!(c.get(1, 1), cs.button_hilight.argb(), "the raised edge");
        let (lit, hot, plain) = (l.tiles[0], l.tiles[2], l.tiles[1]);
        assert_eq!(c.get(lit.x + 1, lit.y + 40), cs.hilight.argb());
        assert_eq!(c.get(lit.x + 2, lit.y + 40), cs.button_face.argb());
        assert_eq!(c.get(hot.x, hot.y + 40), cs.hilight.argb());
        assert_eq!(c.get(hot.x + 1, hot.y + 40), cs.button_face.argb());
        assert_eq!(c.get(plain.x, plain.y + 40), cs.button_face.argb());
    }

    #[test]
    fn only_1_lights_screen_1_and_leaves_screen_2_dark() {
        let t = Theme::default();
        let cs = &t.colors;
        let l = ProjectorLayout::compute();
        let c = draw(&t, &text(), &l, &Projector::new(Projection::Only(0), 2));
        let [one, two] = screens(l.picture(2), 1);
        let low = |r: Rect| c.get(r.x + 3, r.bottom() - 3);
        assert_eq!(low(one), cs.background.argb());
        assert_eq!(low(two), cs.button_face.argb());
        assert_eq!(
            c.get(two.x, two.y + 5),
            cs.button_text.argb(),
            "its outline"
        );
        let [one, two] = screens(l.picture(3), 1);
        assert_eq!(
            (low(one), low(two)),
            (cs.button_face.argb(), cs.background.argb())
        );
    }

    #[test]
    fn extend_bands_across_the_gap_and_duplicate_shows_a_window_on_each() {
        let t = Theme::default();
        let cs = &t.colors;
        let l = ProjectorLayout::compute();
        let c = draw(&t, &text(), &l, &Projector::new(Projection::Extend, 2));
        let [one, two] = screens(l.picture(0), 1);
        let gap = (one.right() + two.x) / 2;
        assert_eq!(c.get(gap, one.y + 20), cs.window.argb(), "the band");
        assert_eq!(c.get(gap, one.y + 5), cs.button_face.argb());
        let [one, two] = screens(l.picture(1), 1);
        for s in [one, two] {
            assert_eq!(c.get(s.x + 10, s.y + 16), cs.window.argb(), "{s:?}");
        }
    }

    #[test]
    fn a_greyed_tile_draws_in_gray_text_with_dark_screens() {
        let t = Theme::default();
        let cs = &t.colors;
        let l = ProjectorLayout::compute();
        let c = draw(&t, &text(), &l, &Projector::new(Projection::Only(0), 1));
        let caption = l.caption(0);
        assert!(any_px(&c, caption, |px| px == cs.gray_text.argb()));
        assert!(!any_px(&c, caption, |px| px == cs.button_text.argb()));
        assert!(!any_px(&c, l.picture(0), |px| px == cs.background.argb()));
        assert!(any_px(&c, l.caption(2), |px| px == cs.button_text.argb()));
        assert!(any_px(&c, l.picture(2), |px| px == cs.background.argb()));
    }

    #[test]
    fn blue_rings_the_lit_tile_in_its_scheme() {
        let t = theme_for(Interface::Blue);
        let l = ProjectorLayout::compute();
        let c = draw(&t, &text(), &l, &Projector::new(Projection::Duplicate, 2));
        let lit = l.tiles[1];
        let hilight = crate::blue::Variant::Blue.scheme().system.hilight;
        assert_eq!(c.get(lit.x + 1, lit.y + 40), hilight.argb());
    }

    #[test]
    fn glass_cuts_its_corners_and_fades_a_greyed_tile() {
        let t = theme_for(Interface::Glass);
        let ink = t.look.style().unwrap().caption_text.color.argb();
        let l = ProjectorLayout::compute();
        let c = draw_projector(&t, &l, &text(), &Projector::new(Projection::Only(0), 1));
        assert_eq!(c.get(0, 0) >> 24, 0, "the cut corner is clear");
        assert_ne!(c.get(l.width / 2, 6) >> 24, 0, "the sheet has its tint");
        let fill = |r: Rect| c.get(r.x + 3, r.bottom() - 3);
        assert_eq!(fill(screens(l.picture(2), 1)[0]), t.colors.hilight.argb());
        assert_ne!(fill(screens(l.picture(0), 1)[0]), t.colors.hilight.argb());
        assert!(any_px(&c, l.caption(2), |px| px == ink));
        assert!(!any_px(&c, l.caption(0), |px| px == ink), "faded");
    }

    #[test]
    fn glass_draws_the_lit_and_hot_tiles_apart_from_a_plain_one() {
        let t = theme_for(Interface::Glass);
        let l = ProjectorLayout::compute();
        let text = text();
        let p = Projector {
            hot: Some(1),
            ..Projector::new(Projection::Only(1), 2)
        };
        let c = draw_projector(&t, &l, &text, &p);
        let corner = |i: usize| c.get(l.tiles[i].x + 3, l.tiles[i].y + 3);
        let (lit, hot, plain) = (corner(3), corner(1), corner(0));
        assert!(lit != plain && hot != plain && lit != hot);
    }

    #[test]
    fn scale_2_draws_at_twice_the_size() {
        let text = text();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let t = theme_for(i).scaled(2);
            let l = ProjectorLayout::compute().scaled(2);
            let c = draw_projector(&t, &l, &text, &Projector::new(Projection::Extend, 2));
            assert_eq!((c.width, c.height), (864, 204), "{i:?}");
            let [one, _] = screens(l.picture(0), 2);
            let want = match i {
                Interface::Glass => t.colors.hilight,
                _ => t.colors.background,
            };
            assert_eq!(c.get(one.x + 6, one.bottom() - 6), want.argb(), "{i:?}");
        }
    }

    #[test]
    fn each_state_is_pinned_in_every_look_at_scale_1_and_2() {
        crate::pins::fonts();
        let text = text();
        let states = [
            ("lit", Projector::new(Projection::Extend, 2)),
            (
                "hot",
                Projector {
                    hot: Some(2),
                    ..Projector::new(Projection::Duplicate, 2)
                },
            ),
            ("greyed", Projector::new(Projection::Only(0), 1)),
        ];
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for s in [1, 2] {
                let t = theme_for(i).scaled(s);
                let l = ProjectorLayout::compute().scaled(s);
                for (state, p) in &states {
                    let c = draw_projector(&t, &l, &text, p);
                    got.push((format!("{i:?} {s} {state}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("controls/projector", &got);
    }

    #[test]
    fn access_names_a_radio_per_tile_and_checks_the_lit_one() {
        let l = ProjectorLayout::compute();
        let mut tree = Tree::window("Display switcher", (l.width, l.height));
        let p = Projector::new(Projection::Only(0), 1);
        let lit = access(&l, &p, &mut tree, access::ROOT);
        assert_eq!(tree.find(Role::RadioButton, "Only 1"), Some(lit));
        let n = tree.get(lit).unwrap();
        assert_eq!(n.toggled(), Some(Toggled::True));
        assert_eq!(n.is_selected(), Some(true));
        let extend = tree.find(Role::RadioButton, "Extend").unwrap();
        let n = tree.get(extend).unwrap();
        assert!(n.is_disabled());
        assert_eq!(n.toggled(), Some(Toggled::False));
        assert!(tree.find(Role::RadioButton, "Only 2").is_some());
        assert!(tree.find(Role::RadioButton, "Duplicate").is_some());
    }

    #[test]
    #[ignore]
    fn render_projector() {
        let text = text();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for s in [1, 2] {
                let p = Projector {
                    hot: Some(1),
                    enabled: [true, true, true, false],
                    ..Projector::new(Projection::Only(0), 2)
                };
                let t = theme_for(i).scaled(s);
                let l = ProjectorLayout::compute().scaled(s);
                let c = draw_projector(&t, &l, &text, &p);
                let mut bg = Canvas::new(c.width, c.height);
                bg.fill_rect(bg.bounds(), Color::rgb(0x80, 0x80, 0x80));
                bg.blit_over(0, 0, &c);
                crate::raster::write_preview(&bg, &format!("projector-{i:?}-{s}.png"));
            }
        }
    }
}
