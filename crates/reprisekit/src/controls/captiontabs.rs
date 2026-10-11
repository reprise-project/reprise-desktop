use crate::access::{self, Tree, accesskit};
use crate::frame::{CaptionTab, FrameGeometry};
use crate::look::widgets::{caption_tab_close, draw_caption_tab};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use accesskit::{NodeId, Role};

/// Logical px.
pub const TAB_MAX: i32 = 200;
/// When even this does not fit, the tabs take the grab room too.
pub const TAB_MIN: i32 = 48;
/// Caption kept free right of the tabs to drag the frame by.
pub const GRAB: i32 = 40;

#[derive(Clone, Copy, Debug)]
pub struct TabEntry<'a> {
    pub title: &'a str,
    pub icon: Option<&'a Canvas>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabHit {
    pub tab: usize,
    pub close: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CaptionTabs<'a> {
    pub tabs: Vec<TabEntry<'a>>,
    pub selected: usize,
    pub hot: Option<TabHit>,
    pub held: Option<TabHit>,
    /// The tab standing in for a window held over the caption, at the slot
    /// a release puts it in.
    pub ghost: Option<usize>,
}

impl CaptionTabs<'_> {
    /// The tab after the selected one, round to the first after the last;
    /// with `backwards` the one before it. The ghost is never a step.
    pub fn step(&self, backwards: bool) -> usize {
        let real: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| !self.is_ghost(i))
            .collect();
        let n = real.len();
        if n == 0 {
            return 0;
        }
        let at = real
            .iter()
            .position(|&i| i >= self.selected)
            .unwrap_or(n - 1);
        real[if backwards {
            (at + n - 1) % n
        } else {
            (at + 1) % n
        }]
    }

    pub fn hover(&mut self, hit: Option<TabHit>) -> bool {
        let changed = self.hot != hit;
        self.hot = hit;
        changed
    }

    fn is_ghost(&self, i: usize) -> bool {
        self.ghost == Some(i)
    }

    fn tab(&self, i: usize, close: Option<Rect>) -> CaptionTab<'_> {
        let e = self.tabs[i];
        let ghost = self.is_ghost(i);
        let on = |h: Option<TabHit>| h.is_some_and(|h| h.tab == i);
        let body = if self.held
            == Some(TabHit {
                tab: i,
                close: false,
            }) {
            Interaction::Pressed
        } else if on(self.hot) {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        let shut = TabHit {
            tab: i,
            close: true,
        };
        let close_ptr = match (self.held == Some(shut), self.hot == Some(shut)) {
            (true, true) => Interaction::Pressed,
            (_, true) => Interaction::Hot,
            _ => Interaction::Idle,
        };
        let at_rest = |j: usize| {
            j != self.selected && !self.is_ghost(j) && !on_tab(self.hot, j) && !on_tab(self.held, j)
        };
        CaptionTab {
            title: e.title,
            icon: e.icon,
            selected: i == self.selected && !ghost,
            ptr: body,
            ghost,
            close: close
                .filter(|_| !ghost && (on(self.hot) || on(self.held)))
                .map(|r| (r, close_ptr)),
            divider: i + 1 < self.tabs.len() && at_rest(i) && at_rest(i + 1),
        }
    }
}

fn on_tab(h: Option<TabHit>, j: usize) -> bool {
    h.is_some_and(|h| h.tab == j)
}

/// In the frame's coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub tabs: Vec<Rect>,
    pub closes: Vec<Option<Rect>>,
}

/// `theme` at the scale `geom` is in: logical for the hit test, scaled to draw.
pub fn layout(theme: &Theme, geom: &FrameGeometry, n: usize) -> Layout {
    let s = theme.scale.max(1);
    let cap = geom.caption;
    let (x0, x1) = (cap.x + 2 * s, geom.title_end() - 2 * s);
    let room = (x1 - x0).max(0);
    let count = n.max(1) as i32;
    let share = ((room - GRAB * s) / count).clamp(TAB_MIN * s, TAB_MAX * s);
    let w = if share * count > room {
        room / count
    } else {
        share
    };
    let margin = ((cap.height - theme.icon_size() - 6 * s) / 2).max(0);
    let (y, h) = (cap.y + margin, cap.height - 2 * margin);
    let (cw, ch) = caption_tab_close(&theme.paint(), &theme.metrics);
    let tabs: Vec<Rect> = (0..n as i32)
        .map(|i| Rect::new(x0 + i * w, y, w, h))
        .collect();
    let closes = tabs
        .iter()
        .map(|t| {
            (t.width >= 3 * cw)
                .then(|| Rect::new(t.right() - 2 * s - cw, t.y + (t.height - ch) / 2, cw, ch))
        })
        .collect();
    Layout { tabs, closes }
}

pub fn hit(l: &Layout, x: i32, y: i32) -> Option<TabHit> {
    let tab = l.tabs.iter().position(|r| r.contains(x, y))?;
    let close = l.closes[tab].is_some_and(|r| r.contains(x, y));
    Some(TabHit { tab, close })
}

/// The tab at `x`, the first or last beyond either end. Laid out with a
/// ghost tab, the slot the ghost takes under the pointer.
pub fn slot_at(l: &Layout, x: i32) -> usize {
    let n = l.tabs.len();
    if n == 0 {
        return 0;
    }
    l.tabs.iter().position(|r| x < r.right()).unwrap_or(n - 1)
}

pub fn draw(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    caption_text: &TextRenderer,
    geom: &FrameGeometry,
    tabs: &CaptionTabs,
    window_active: bool,
) {
    if geom.caption.intersect(&c.bounds()).is_empty() {
        return;
    }
    let l = layout(theme, geom, tabs.tabs.len());
    let p = theme.paint();
    for (i, &r) in l.tabs.iter().enumerate() {
        let tab = tabs.tab(i, l.closes[i]);
        c.with_clip(r, |c| {
            draw_caption_tab(
                c,
                text,
                caption_text,
                &p,
                r,
                &tab,
                window_active,
                theme.fonts.size_px,
            )
        });
    }
}

pub fn access(
    tabs: &CaptionTabs,
    l: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> Vec<NodeId> {
    let all = l.tabs.iter().fold(None, |u: Option<Rect>, r| {
        Some(u.map_or(*r, |u| u.union(r)))
    });
    let list = tree.push(
        parent,
        key,
        access::node(Role::TabList, all.unwrap_or_default()),
    );
    tabs.tabs
        .iter()
        .zip(&l.tabs)
        .enumerate()
        .map(|(i, (e, r))| {
            let mut n = access::named(Role::Tab, *r, e.title);
            n.set_selected(i == tabs.selected && !tabs.is_ghost(i));
            tree.push(list, i, n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Chrome, FrameState, Glow};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::{Color, FontConfig};

    fn geom(theme: &Theme, cw: i32) -> FrameGeometry {
        FrameGeometry::layout(&theme.metrics, cw, 40, false, Chrome::WINDOW)
    }

    fn tabs(n: usize, selected: usize) -> CaptionTabs<'static> {
        CaptionTabs {
            tabs: ["Alpha", "Beta", "Gamma", "Delta"][..n]
                .iter()
                .map(|t| TabEntry {
                    title: t,
                    icon: None,
                })
                .collect(),
            selected,
            ..Default::default()
        }
    }

    fn draw_over(theme: &Theme, cw: i32, t: &CaptionTabs, strip: bool) -> (Canvas, Layout) {
        let base = FrameGeometry::layout(&theme.metrics, cw, 40, false, Chrome::WINDOW);
        let g = base.scaled(theme.scale);
        let scaled = theme.scaled(theme.scale);
        let text = TextRenderer::load(&FontConfig::default());
        let caption_text = match theme.look.style() {
            Some(st) => TextRenderer::load(&st.caption_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let st = FrameState {
            title: "",
            active: true,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: true,
        };
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        crate::look::draw_frame(&mut c, &scaled, &text, &caption_text, &g, &st);
        if strip {
            draw(&mut c, &scaled, &text, &caption_text, &g, t, true);
        }
        (c, layout(&scaled, &g, t.tabs.len()))
    }

    fn over(under: u32, color: Color, alpha: u8) -> u32 {
        let a = u32::from(alpha);
        let ch = |shift: u32, s: u8| {
            (((under >> shift) & 0xFF) * (255 - a) + u32::from(s) * a + 127) / 255
        };
        (ch(24, 255) << 24) | (ch(16, color.r) << 16) | (ch(8, color.g) << 8) | ch(0, color.b)
    }

    #[test]
    fn tabs_share_the_strip_less_the_grab_room_up_to_their_widest() {
        let theme = Theme::default();
        let g = geom(&theme, 600);
        let l = layout(&theme, &g, 2);
        let x0 = g.caption.x + 2;
        assert_eq!(l.tabs[0].x, x0, "2 px into the caption");
        assert_eq!(
            l.tabs[0].width, TAB_MAX,
            "two tabs on a wide caption: the widest"
        );
        assert_eq!(l.tabs[1].x, x0 + TAB_MAX);
        let l = layout(&theme, &g, 4);
        let room = g.title_end() - 2 - x0;
        assert_eq!(l.tabs[0].width, (room - GRAB) / 4);
        assert!(
            l.tabs[3].right() <= g.title_end() - 2 - GRAB,
            "the grab room stays free"
        );
        assert_eq!((l.tabs[0].y, l.tabs[0].height), (g.caption.y, 18));
    }

    #[test]
    fn tabs_that_cannot_keep_their_narrowest_share_the_whole_strip() {
        let theme = Theme::default();
        let g = geom(&theme, 160);
        let l = layout(&theme, &g, 4);
        let room = g.title_end() - 2 - (g.caption.x + 2);
        assert!(room / 4 < TAB_MIN);
        assert_eq!(l.tabs[0].width, room / 4);
        assert_eq!(l.closes[0], None, "too narrow for a close box");
    }

    #[test]
    fn the_close_box_sits_2_px_in_from_the_right_and_centred() {
        let theme = Theme::default();
        let g = geom(&theme, 600);
        let l = layout(&theme, &g, 2);
        let (t, b) = (l.tabs[0], l.closes[0].unwrap());
        assert_eq!((b.width, b.height), (16, 14));
        assert_eq!(b.right(), t.right() - 2);
        assert_eq!(b.y, t.y + 2);
        let glass = theme_for(Interface::Glass);
        let g = geom(&glass, 600);
        let l = layout(&glass, &g, 2);
        let b = l.closes[0].unwrap();
        assert_eq!((b.width, b.height), (14, 14), "a style's own close box");
    }

    #[test]
    fn the_hit_test_finds_the_tab_and_its_close_box() {
        let theme = Theme::default();
        let g = geom(&theme, 600);
        let l = layout(&theme, &g, 3);
        let (t, b) = (l.tabs[1], l.closes[1].unwrap());
        let on = |tab, close| Some(TabHit { tab, close });
        assert_eq!(hit(&l, t.x + 5, t.y + 5), on(1, false));
        assert_eq!(hit(&l, b.x + 1, b.y + 1), on(1, true));
        assert_eq!(
            hit(&l, l.tabs[2].right() + 1, t.y + 5),
            None,
            "the grab room"
        );
        assert_eq!(slot_at(&l, t.x + 1), 1);
        assert_eq!(slot_at(&l, 0), 0);
        assert_eq!(slot_at(&l, 10_000), 2);
    }

    #[test]
    fn step_goes_round_and_skips_the_ghost() {
        let mut t = tabs(3, 2);
        assert_eq!(t.step(false), 0);
        assert_eq!(t.step(true), 1);
        t.ghost = Some(2);
        t.selected = 1;
        assert_eq!(t.step(false), 0, "the third tab is a ghost");
        t.ghost = Some(1);
        t.selected = 2;
        assert_eq!(t.step(true), 0, "the ghost between them is no step");
        assert_eq!(t.step(false), 0);
        let lit = Some(TabHit {
            tab: 0,
            close: false,
        });
        assert!(t.hover(lit));
        assert!(!t.hover(lit));
    }

    #[test]
    fn classic_sinks_the_selected_tab_raises_the_lit_one_and_etches_the_rest() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut t = tabs(4, 0);
        t.hot = Some(TabHit {
            tab: 1,
            close: false,
        });
        let (c, l) = draw_over(&theme, 700, &t, true);
        let (sel, hot, rest) = (l.tabs[0], l.tabs[1], l.tabs[2]);
        assert_eq!(c.get(sel.x, sel.y), cs.button_shadow.argb(), "sunken");
        assert_eq!(
            c.get(sel.right() - 1, sel.bottom() - 1),
            cs.button_hilight.argb()
        );
        assert_eq!(c.get(hot.x, hot.y + 3), cs.button_hilight.argb(), "raised");
        assert_eq!(c.get(hot.right() - 1, hot.y + 3), cs.button_shadow.argb());
        assert_eq!(c.get(rest.right() - 2, rest.y + 1), cs.button_shadow.argb());
        assert_eq!(
            c.get(rest.right() - 1, rest.y + 1),
            cs.button_hilight.argb()
        );
        assert_ne!(c.get(rest.right() - 2, rest.y), cs.button_shadow.argb());
        let b = l.closes[1].unwrap();
        assert_eq!(c.get(b.x + 2, b.y + 2), cs.button_face.argb());
        assert_eq!(c.get(b.x, b.y), cs.button_hilight.argb());
        let (bare, _) = draw_over(&theme, 700, &t, false);
        let r = l.closes[2].unwrap();
        assert_eq!(
            c.get(r.x + 2, r.y + 2),
            bare.get(r.x + 2, r.y + 2),
            "a tab at rest shows no close box"
        );
    }

    #[test]
    fn classic_rings_the_ghost_in_the_caption_ink_and_doubles_at_scale_2() {
        let theme = Theme::default();
        let mut t = tabs(3, 0);
        t.ghost = Some(2);
        let (c, l) = draw_over(&theme, 600, &t, true);
        let g = l.tabs[2];
        let ink = theme.colors.title_text.argb();
        assert_eq!(c.get(g.x, g.y), ink, "the ghost's dotted ring");
        assert_ne!(c.get(g.x + 1, g.y), ink);
        t.ghost = Some(1);
        let (c, l) = draw_over(&theme, 600, &t, true);
        let (g, last) = (l.tabs[1], l.tabs[2]);
        assert_eq!(c.get(g.x, g.y), ink, "a ghost between two tabs rings there");
        assert_ne!(c.get(last.x, last.y), ink, "the tab after it is a real one");
        let two = Theme {
            scale: 2,
            ..Theme::default()
        };
        let (c, l) = draw_over(&two, 600, &tabs(2, 0), true);
        let sel = l.tabs[0];
        let shadow = theme.colors.button_shadow.argb();
        let x0 = geom(&theme, 600).caption.x + 2;
        assert_eq!((sel.x, sel.width), (2 * x0, 2 * TAB_MAX));
        assert_eq!(c.get(sel.x + 1, sel.y + 1), shadow, "the edge 2 px thick");
        assert_ne!(c.get(sel.x + 2, sel.y + 2), shadow);
    }

    #[test]
    fn blue_washes_the_selected_tab_white_inside_its_line() {
        let theme = theme_for(Interface::Blue);
        let look = theme.look.style().unwrap().caption_tabs.unwrap();
        let mut t = tabs(3, 0);
        t.hot = Some(TabHit {
            tab: 1,
            close: true,
        });
        let (bare, l) = draw_over(&theme, 700, &t, false);
        let (c, _) = draw_over(&theme, 700, &t, true);
        let white = Color::rgb(0xFF, 0xFF, 0xFF);
        let sel = l.tabs[0];
        let (x, y) = (sel.x + 1, sel.y + sel.height / 2);
        assert_eq!(c.get(x, y), over(bare.get(x, y), white, look.selected.1));
        let (x, y) = (sel.x + 6, sel.y);
        let washed = over(bare.get(x, y), white, look.selected.1);
        assert_eq!(c.get(x, y), over(washed, white, look.outline.1));
        assert_eq!(
            c.get(sel.x, sel.y),
            bare.get(sel.x, sel.y),
            "the corner is cut"
        );
        let b = l.closes[1].unwrap();
        let (x, y) = (b.x + 1, b.y + 1);
        let lit = over(bare.get(x, y), white, look.hot.1);
        assert_eq!(c.get(x, y), over(lit, white, look.close_hot.1));
        let rest = l.tabs[2];
        let (x, y) = (rest.x + 1, rest.y + rest.height / 2);
        assert_eq!(c.get(x, y), bare.get(x, y));
    }

    #[test]
    fn glass_washes_every_tab_over_the_glass_alpha_and_all() {
        let theme = theme_for(Interface::Glass);
        let look = theme.look.style().unwrap().caption_tabs.unwrap();
        let white = Color::rgb(0xFF, 0xFF, 0xFF);
        assert_eq!(look.rest.0, white, "dark titles: white washes");
        let t = tabs(3, 1);
        let (bare, l) = draw_over(&theme, 700, &t, false);
        let (c, _) = draw_over(&theme, 700, &t, true);
        let (rest, sel) = (l.tabs[0], l.tabs[1]);
        let (x, y) = (rest.x + 1, rest.y + rest.height / 2);
        assert!(
            bare.get(x, y) >> 24 < 0xFF,
            "the glass is see-through there"
        );
        assert_eq!(c.get(x, y), over(bare.get(x, y), white, look.rest.1));
        let (x, y) = (sel.x + 1, sel.y + sel.height / 2);
        assert_eq!(c.get(x, y), over(bare.get(x, y), white, look.selected.1));
        let dark = crate::interfaces::theme_with(
            Interface::Glass,
            crate::blue::Variant::Blue,
            &crate::glass::Settings {
                mode: crate::glass::Mode::Dark,
                ..Default::default()
            },
        );
        let look = dark.look.style().unwrap().caption_tabs.unwrap();
        assert_eq!(
            look.rest.0,
            Color::rgb(0, 0, 0),
            "light titles: black washes"
        );
    }

    #[test]
    fn the_strip_reads_as_a_tab_list_with_the_shown_tab_selected() {
        let theme = Theme::default();
        let g = geom(&theme, 600);
        let mut t = tabs(3, 1);
        t.ghost = Some(2);
        let l = layout(&theme, &g, 3);
        let mut tree = Tree::window("w", (600, 60));
        let ids = access(&t, &l, &mut tree, crate::access::ROOT, "tabs");
        let node = |i: usize| tree.get(ids[i]).unwrap();
        assert_eq!(node(0).role(), Role::Tab);
        assert_eq!(node(0).label(), Some("Alpha"));
        assert_eq!(node(1).is_selected(), Some(true));
        assert_eq!(node(0).is_selected(), Some(false));
        assert_eq!(
            node(2).is_selected(),
            Some(false),
            "the ghost is never selected"
        );
    }
}
