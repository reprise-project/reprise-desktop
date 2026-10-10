//! The menu bar under a window's caption: a 20 px strip with the titles side by
//! side, the open title sunken and its popup hanging from the frame's
//! bottom-left. Access-key underlines show once Alt or F10 activated the bar.
//! Looks with flat menus ([`crate::style::Style::flat_menus`]) or menu bar parts
//! ([`Part::MenuBar`], [`Part::MenuBarItem`]) draw hot and open titles their own
//! way; on an unfocused window the titles ink grey.

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::metrics::MenuTitles;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::{Part, State};
use crate::text::split_mnemonic;
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};

pub const BAR_H: i32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Title {
    pub label: String,
    pub mnemonic: Option<usize>,
}

impl Title {
    pub fn new(label: &str) -> Title {
        let (label, mnemonic) = split_mnemonic(label);
        Title { label, mnemonic }
    }

    pub fn access_key(&self) -> Option<char> {
        let ch = match self.mnemonic {
            Some(i) => self.label.chars().nth(i),
            None => self.label.chars().next(),
        };
        ch.and_then(|c| c.to_lowercase().next())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub bar: Rect,
    pub frames: Vec<Rect>,
}

impl Layout {
    pub fn compute(
        text: &TextRenderer,
        px: u32,
        client: Rect,
        titles: &[Title],
        m: &MenuTitles,
    ) -> Layout {
        let bar = Rect::new(client.x, client.y, client.width, BAR_H);
        let mut x = bar.x + m.first_x;
        let frames = titles
            .iter()
            .map(|t| {
                let w = text.measure(&t.label, false, px) + 2 * m.pad;
                let r = Rect::new(x, bar.y + m.frame_y, w, m.frame_h);
                x += w;
                r
            })
            .collect();
        Layout { bar, frames }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.frames.iter().position(|r| r.contains(x, y))
    }

    pub fn popup_origin(&self, i: usize) -> (i32, i32) {
        let r = self.frames[i];
        (r.x, r.bottom())
    }
}

/// Draw the bar: `open` is the sunken title, `hot` the one under the pointer,
/// `underline` whether access keys show. A `banded` bar sits in a rebar band
/// that paints the strip behind it.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lay: &Layout,
    titles: &[Title],
    open: Option<usize>,
    hot: Option<usize>,
    underline: bool,
    banded: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let flat = p.look.style().is_some_and(|st| st.flat_menus);
    let bm = p.look.controls().menubar;
    let m = bm.titles(banded);
    let (pad, dy) = (m.pad, m.text_dy);
    if let Some(st) = p.look.style()
        && st.has_parts(&[Part::MenuBar, Part::MenuBarItem])
    {
        return draw_framed(
            c, text, p, st, lay, titles, open, hot, underline, banded, px,
        );
    }
    if !banded {
        let strip = if flat { cs.menu_bar } else { cs.button_face };
        c.fill_rect(lay.bar.scaled(s), strip);
    }
    if !banded && bm.line > 0 {
        let b = lay.bar;
        let line = Rect::new(b.x, b.bottom() - bm.line, b.width, bm.line);
        c.fill_rect(line.scaled(s), cs.window);
    }
    let lit = open.or(hot);
    for (i, (t, r)) in titles.iter().zip(&lay.frames).enumerate() {
        let r = r.scaled(s);
        let mut ink = if flat && !p.active {
            cs.button_shadow
        } else {
            cs.menu_text
        };
        let mut o = 0;
        if flat && lit == Some(i) {
            c.fill_rect(r, cs.hilight);
            ink = cs.hilight_text;
        } else if !flat && open == Some(i) {
            c.edge_t(r, EdgeStyle::ButtonSunken, cs, s);
            o = s;
        }
        let tr = Rect::new(
            r.x + pad * s + o,
            r.y + dy * s + o,
            r.width - 2 * pad * s,
            r.height,
        );
        text.draw(c, tr, &t.label, false, ink, Align::Left, px);
        if let (true, Some(m)) = (underline, t.mnemonic)
            && let Some(baseline) = text.baseline(tr, false, px)
        {
            let (dx, w) = text.span(&t.label, m, false, px);
            c.fill_rect(Rect::new(tr.x + dx, baseline + s, w, s), ink);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_framed(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    st: &crate::style::Style,
    lay: &Layout,
    titles: &[Title],
    open: Option<usize>,
    hot: Option<usize>,
    underline: bool,
    banded: bool,
    px: u32,
) {
    use crate::look::styled::menu as look;
    let Paint { cs, s, .. } = *p;
    let m = *p.look.controls().menubar.titles(banded);
    if !banded {
        look::draw_bar(st, c, lay.bar.scaled(s), s);
    }
    let ink = if p.active { cs.menu_text } else { cs.gray_text };
    let hot = hot.filter(|_| p.active);
    for (i, (t, r)) in titles.iter().zip(&lay.frames).enumerate() {
        let r = r.scaled(s);
        if open == Some(i) {
            look::draw_title(st, c, r, State::Pressed, s);
        } else if hot == Some(i) {
            look::draw_title(st, c, r, State::Hot, s);
        }
        let tr = Rect::new(
            r.x + m.pad * s,
            r.y + m.text_dy * s,
            r.width - 2 * m.pad * s,
            r.height,
        );
        text.draw(c, tr, &t.label, false, ink, Align::Left, px);
        if let (true, Some(m)) = (underline, t.mnemonic)
            && let Some(baseline) = text.baseline(tr, false, px)
        {
            let (dx, w) = text.span(&t.label, m, false, px);
            c.fill_rect(Rect::new(tr.x + dx, baseline + s, w, s), ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn titles_sit_side_by_side_from_two_px_in() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let titles: Vec<Title> = ["&File", "&Edit", "F&ormat", "&View", "&Help"]
            .iter()
            .map(|t| Title::new(t))
            .collect();
        assert_eq!(titles[2].mnemonic, Some(1));
        assert_eq!(titles[2].access_key(), Some('o'));
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &titles,
            &MenuTitles::CLASSIC,
        );
        assert_eq!(lay.bar, Rect::new(0, 0, 300, BAR_H));
        let file = lay.frames[0];
        assert_eq!((file.x, file.y, file.height), (2, 1, 18));
        assert_eq!(file.width, text.measure("File", false, px) + 14);
        assert_eq!(lay.frames[1].x, file.right());
        assert_eq!(lay.hit(file.x + 3, 5), Some(0));
        assert_eq!(lay.hit(file.right() + 1, 5), Some(1));
        assert_eq!(lay.hit(5, 25), None);
        assert_eq!(lay.popup_origin(0), (file.x, 19));
    }

    #[test]
    fn an_unfocused_blue_bar_greys_its_titles() {
        use crate::blue::Variant;
        use crate::theme::Color;
        let text = TextRenderer::load(&FontConfig::default());
        let titles = [Title::new("&File"), Title::new("&Edit")];
        let inks = |theme: &Theme, active: bool, banded: bool| {
            let px = theme.fonts.size_px;
            let lay = Layout::compute(
                &text,
                px,
                Rect::new(0, 0, 200, BAR_H),
                &titles,
                &MenuTitles::CLASSIC,
            );
            let mut c = Canvas::new(200, BAR_H as u32);
            c.fill_rect(c.bounds(), Color::rgb(1, 2, 3));
            let p = Paint {
                active,
                ..theme.paint()
            };
            draw(
                &mut c, &text, &p, &lay, &titles, None, None, false, banded, px,
            );
            let mut seen = std::collections::HashSet::new();
            for y in 0..BAR_H {
                for x in 0..200 {
                    seen.insert(c.get(x, y));
                }
            }
            seen
        };
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let theme = crate::interfaces::blue_theme(v);
            let grey = theme.colors.button_shadow;
            for banded in [false, true] {
                let off = inks(&theme, false, banded);
                assert!(off.contains(&grey.argb()), "{v:?} banded {banded}");
                assert!(!off.contains(&0xFF00_0000), "{v:?}: no black");
                let on = inks(&theme, true, banded);
                assert!(on.contains(&0xFF00_0000), "{v:?}: black with the focus");
                assert!(!on.contains(&grey.argb()), "{v:?}: no grey with the focus");
            }
        }
        let classic = Theme::default();
        assert!(inks(&classic, false, false).contains(&0xFF00_0000));
    }

    #[test]
    fn open_title_is_sunken_with_its_text_shifted() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let titles = vec![Title::new("&File"), Title::new("&Edit")];
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 200, 40),
            &titles,
            &MenuTitles::CLASSIC,
        );
        let mut closed = Canvas::new(200, 40);
        draw(
            &mut closed,
            &text,
            &classic_paint(cs, 1),
            &lay,
            &titles,
            None,
            None,
            false,
            false,
            px,
        );
        let mut open = Canvas::new(200, 40);
        draw(
            &mut open,
            &text,
            &classic_paint(cs, 1),
            &lay,
            &titles,
            Some(0),
            None,
            true,
            false,
            px,
        );
        let f = lay.frames[0];
        assert_eq!(closed.get(f.x, f.y), cs.button_face.argb());
        assert_eq!(open.get(f.x, f.y), cs.button_shadow.argb());
        assert_eq!(
            open.get(f.right() - 1, f.bottom() - 1),
            cs.button_hilight.argb()
        );
        let ink = |c: &Canvas| {
            (f.x..f.right())
                .flat_map(|x| (f.y..f.bottom()).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.menu_text.argb())
                .min()
                .unwrap()
        };
        let (cx, cy) = ink(&closed);
        let (ox, oy) = ink(&open);
        assert_eq!((ox, oy), (cx + 1, cy + 1));
        let under = |c: &Canvas| {
            (f.y..f.bottom())
                .filter(|&y| {
                    (f.x..f.right())
                        .filter(|&x| c.get(x, y) == cs.menu_text.argb())
                        .count()
                        >= 5
                })
                .count()
        };
        assert!(under(&open) > under(&closed));
    }

    #[test]
    fn a_flat_bar_lights_its_hot_and_open_titles_in_hilight() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        for v in crate::blue::Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            let (cs, px) = (&theme.colors, theme.fonts.size_px);
            let titles = vec![Title::new("&File"), Title::new("&View")];
            let lay = Layout::compute(
                &text,
                px,
                Rect::new(0, 0, 200, 40),
                &titles,
                &MenuTitles::CLASSIC,
            );
            let band = crate::theme::Color::rgb(0xF3, 0xF4, 0xF5);
            let paint_in = |open, hot, banded| {
                let mut c = Canvas::new(200, 40);
                c.fill_rect(Rect::new(0, 0, 200, 40), band);
                draw(
                    &mut c,
                    &text,
                    &theme.paint(),
                    &lay,
                    &titles,
                    open,
                    hot,
                    false,
                    banded,
                    px,
                );
                c
            };
            let paint = |open, hot| paint_in(open, hot, false);
            let banded = paint_in(None, Some(1), true);
            assert_eq!(
                banded.get(0, 0),
                band.argb(),
                "{v:?}: a banded bar leaves the band"
            );
            assert_eq!(
                banded.get(lay.frames[1].x, lay.frames[1].y),
                cs.hilight.argb()
            );
            let f = lay.frames[1];
            let ink = |c: &Canvas, col: crate::theme::Color| {
                (f.x..f.right())
                    .flat_map(|x| (f.y..f.bottom()).map(move |y| (x, y)))
                    .filter(|&(x, y)| c.get(x, y) == col.argb())
                    .min()
            };
            let rest = paint(None, None);
            assert_eq!(rest.get(0, 0), cs.menu_bar.argb(), "{v:?}: the bar");
            assert_eq!(
                rest.get(f.x, f.y),
                cs.menu_bar.argb(),
                "{v:?}: a title at rest"
            );
            for (open, hot) in [(None, Some(1)), (Some(1), None), (Some(1), Some(0))] {
                let c = paint(open, hot);
                for (x, y) in [(f.x, f.y), (f.right() - 1, f.bottom() - 1)] {
                    assert_eq!(
                        c.get(x, y),
                        cs.hilight.argb(),
                        "{v:?} {open:?} {hot:?} at {x},{y}"
                    );
                }
                assert_eq!(
                    ink(&c, cs.hilight_text),
                    ink(&rest, cs.menu_text),
                    "{v:?} {open:?} {hot:?}: the ink stays put"
                );
                let file = lay.frames[0];
                assert_eq!(
                    c.get(file.x, file.y),
                    cs.menu_bar.argb(),
                    "{v:?}: File stays unlit"
                );
            }
        }
    }

    #[test]
    fn a_classic_bar_draws_no_hot_title() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let titles = vec![Title::new("&File"), Title::new("&View")];
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 200, 40),
            &titles,
            &MenuTitles::CLASSIC,
        );
        let paint = |hot| {
            let mut c = Canvas::new(200, 40);
            let p = classic_paint(&theme.colors, 1);
            draw(
                &mut c, &text, &p, &lay, &titles, None, hot, false, false, px,
            );
            c
        };
        assert!(paint(None) == paint(Some(1)));
    }

    #[test]
    fn blues_bars_place_their_titles() {
        let text = TextRenderer::load(&FontConfig::default());
        for v in crate::blue::Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            let (cs, px) = (&theme.colors, theme.fonts.size_px);
            let cm = theme.look.controls();
            let mut bar = Bar::new(vec![("&File", vec![]), ("&Edit", vec![])]);
            let client = Rect::new(0, 0, 200, 40);
            let file_w = text.measure("File", false, px);

            let lay = bar.layout(&text, px, client, &cm);
            assert_eq!(lay.frames[0], Rect::new(0, 0, file_w + 12, 19), "{v:?}");
            assert_eq!(lay.frames[1].x, file_w + 12);
            let mut c = Canvas::new(200, 40);
            bar.draw(&mut c, &text, &theme, &lay);
            assert_eq!(c.get(100, 18), cs.menu_bar.argb(), "{v:?}: the bar");
            assert_eq!(c.get(100, BAR_H - 1), cs.window.argb(), "{v:?}: its line");

            bar.banded = true;
            let lay = bar.layout(&text, px, client, &cm);
            assert_eq!(lay.frames[0], Rect::new(2, 1, file_w + 16, 19), "{v:?}");
            let mut c = Canvas::new(200, 40);
            c.fill_rect(c.bounds(), cs.button_face);
            bar.draw(&mut c, &text, &theme, &lay);
            assert_eq!(
                c.get(100, BAR_H - 1),
                cs.button_face.argb(),
                "{v:?}: no line"
            );
        }
    }

    #[test]
    fn blues_title_text_sits_lower_in_a_band() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let (cs, px) = (&theme.colors, theme.fonts.size_px);
        let cm = theme.look.controls();
        let ink = |banded: bool| {
            let mut bar = Bar::new(vec![("&File", vec![])]);
            bar.banded = banded;
            let lay = bar.layout(&text, px, Rect::new(0, 0, 100, 40), &cm);
            let mut c = Canvas::new(100, 40);
            bar.draw(&mut c, &text, &theme, &lay);
            (0..100)
                .flat_map(|x| (0..20).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.menu_text.argb())
                .fold((i32::MAX, i32::MAX), |(ax, ay), (x, y)| {
                    (ax.min(x), ay.min(y))
                })
        };
        assert_eq!(ink(false), (6, 4));
        assert_eq!(ink(true), (10, 6));
    }

    #[test]
    fn a_flat_bar_doubles_at_scale_2() {
        let text = TextRenderer::load(&FontConfig::default());
        let titles = vec![Title::new(""), Title::new("")];
        let lay = Layout {
            bar: Rect::new(2, 2, 60, BAR_H),
            frames: vec![Rect::new(4, 3, 20, 18), Rect::new(24, 3, 20, 18)],
        };
        let draw = |c: &mut Canvas, p: &Paint| {
            draw(c, &text, p, &lay, &titles, None, Some(1), false, false, 11)
        };
        assert!(crate::look::styled::fixture::blue_doubles_at_paint_scale_2(
            66, 26, draw
        ));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Activate,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Enter,
    Char(char, bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Ignored,
    Redraw,
    /// A row of a title's menu was chosen; `path` names it from the title's menu
    /// down (`[3, 1]` is the second row of row 3's submenu), `row` is `path[0]`.
    Chosen {
        title: usize,
        row: usize,
        path: Vec<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    pub titles: Vec<Title>,
    pub menus: Vec<crate::controls::menu::MenuTree>,
    pub open: Option<usize>,
    pub active: Option<usize>,
    pub underline: bool,
    pub hot: Option<usize>,
    pub banded: bool,
    popup: Option<crate::controls::menu::PopupMenu>,
}

fn chosen(title: usize, path: Vec<usize>) -> Action {
    Action::Chosen {
        title,
        row: path[0],
        path,
    }
}

impl Bar {
    pub fn new(entries: Vec<(&str, Vec<crate::controls::menu::MenuItem>)>) -> Bar {
        Bar::with_trees(
            entries
                .into_iter()
                .map(|(t, m)| (t, crate::controls::menu::MenuTree::flat(m)))
                .collect(),
        )
    }

    pub fn with_trees(entries: Vec<(&str, crate::controls::menu::MenuTree)>) -> Bar {
        Bar {
            titles: entries.iter().map(|(t, _)| Title::new(t)).collect(),
            menus: entries.into_iter().map(|(_, m)| m).collect(),
            open: None,
            active: None,
            underline: false,
            hot: None,
            banded: false,
            popup: None,
        }
    }

    pub fn layout(
        &self,
        text: &TextRenderer,
        px: u32,
        client: Rect,
        cm: &crate::metrics::ControlMetrics,
    ) -> Layout {
        let m = cm.menubar.titles(self.banded);
        Layout::compute(text, px, client, &self.titles, m)
    }

    pub fn hovered(&self) -> Option<usize> {
        self.popup.as_ref().and_then(|p| p.hovered())
    }

    pub fn is_active(&self) -> bool {
        self.open.is_some() || self.active.is_some()
    }

    pub fn close(&mut self) {
        self.open = None;
        self.active = None;
        self.underline = false;
        self.popup = None;
    }

    fn ctx<'a>(
        lay: &Layout,
        text: &'a TextRenderer,
        theme: &'a crate::theme::Theme,
        px: u32,
    ) -> crate::controls::menu::MenuCtx<'a> {
        crate::controls::menu::MenuCtx {
            text,
            theme,
            px,
            within: Rect::new(lay.bar.x, lay.bar.y, lay.bar.width, i32::MAX / 4),
        }
    }

    pub fn open_title(
        &mut self,
        i: usize,
        lay: &Layout,
        text: &TextRenderer,
        m: &crate::theme::Theme,
        px: u32,
        by_keyboard: bool,
    ) {
        let cx = Bar::ctx(lay, text, m, px);
        self.popup = Some(crate::controls::menu::PopupMenu::open(
            &self.menus[i],
            lay.popup_origin(i),
            &cx,
            by_keyboard,
        ));
        self.open = Some(i);
        self.active = Some(i);
        self.hot = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn key(
        &mut self,
        key: Key,
        lay: &Layout,
        text: &TextRenderer,
        m: &crate::theme::Theme,
        px: u32,
    ) -> Action {
        use crate::controls::menu::{MenuAction, MenuKey};
        let n = self.titles.len();
        if n == 0 {
            return Action::Ignored;
        }
        let menu_key = match key {
            Key::Up => Some(MenuKey::Up),
            Key::Down => Some(MenuKey::Down),
            Key::Left => Some(MenuKey::Left),
            Key::Right => Some(MenuKey::Right),
            Key::Enter => Some(MenuKey::Enter),
            Key::Escape => Some(MenuKey::Escape),
            Key::Char(c, _) => Some(MenuKey::Char(c)),
            Key::Activate => None,
        };
        if let (Some(open), Some(mk)) = (self.open, menu_key) {
            let cx = Bar::ctx(lay, text, m, px);
            let tree = &self.menus[open];
            let popup = self.popup.as_mut().expect("a menu is open");
            match popup.key(tree, mk, &cx) {
                MenuAction::Chosen(path) => {
                    self.close();
                    return chosen(open, path);
                }
                MenuAction::Redraw => return Action::Redraw,
                MenuAction::Close => {
                    self.open = None;
                    self.popup = None;
                    self.active = Some(open);
                    return Action::Redraw;
                }
                MenuAction::None => match key {
                    Key::Left | Key::Right => {
                        let next = if key == Key::Left {
                            (open + n - 1) % n
                        } else {
                            (open + 1) % n
                        };
                        self.open_title(next, lay, text, m, px, true);
                        return Action::Redraw;
                    }
                    _ => return Action::Redraw,
                },
            }
        }
        match key {
            Key::Activate => {
                if self.is_active() {
                    self.close();
                } else {
                    self.active = Some(0);
                    self.underline = true;
                }
                Action::Redraw
            }
            Key::Char(c, alt) => {
                let c = c.to_lowercase().next().unwrap_or(c);
                if alt || self.active.is_some() {
                    if let Some(i) = self.titles.iter().position(|t| t.access_key() == Some(c)) {
                        self.underline = true;
                        self.open_title(i, lay, text, m, px, true);
                        return Action::Redraw;
                    }
                    return if self.is_active() {
                        Action::Redraw
                    } else {
                        Action::Ignored
                    };
                }
                Action::Ignored
            }
            _ if !self.is_active() => Action::Ignored,
            Key::Escape => {
                self.close();
                Action::Redraw
            }
            Key::Left | Key::Right => {
                let cur = self.active.unwrap_or(0);
                self.active = Some(if key == Key::Left {
                    (cur + n - 1) % n
                } else {
                    (cur + 1) % n
                });
                Action::Redraw
            }
            Key::Down | Key::Up | Key::Enter => {
                if let Some(i) = self.active {
                    self.open_title(i, lay, text, m, px, true);
                }
                Action::Redraw
            }
        }
    }

    pub fn press(
        &mut self,
        x: i32,
        y: i32,
        lay: &Layout,
        text: &TextRenderer,
        m: &crate::theme::Theme,
        px: u32,
    ) -> Action {
        use crate::controls::menu::MenuAction;
        if let (Some(open), true) = (
            self.open,
            self.popup.as_ref().is_some_and(|p| p.contains(x, y)),
        ) {
            let cx = Bar::ctx(lay, text, m, px);
            let tree = &self.menus[open];
            let popup = self.popup.as_mut().expect("checked above");
            return match popup.click(tree, x, y, &cx) {
                MenuAction::Chosen(path) => {
                    self.close();
                    chosen(open, path)
                }
                _ => Action::Redraw,
            };
        }
        if let Some(i) = lay.hit(x, y) {
            if self.open == Some(i) {
                self.close();
            } else {
                self.open_title(i, lay, text, m, px, false);
            }
            return Action::Redraw;
        }
        if self.is_active() {
            self.close();
            return Action::Redraw;
        }
        Action::Ignored
    }

    pub fn motion(
        &mut self,
        x: i32,
        y: i32,
        lay: &Layout,
        text: &TextRenderer,
        m: &crate::theme::Theme,
        px: u32,
    ) -> Action {
        let Some(open) = self.open else {
            return Action::Ignored;
        };
        if self.popup.as_ref().is_some_and(|p| p.contains(x, y)) {
            let cx = Bar::ctx(lay, text, m, px);
            let tree = &self.menus[open];
            let popup = self.popup.as_mut().expect("checked above");
            popup.motion(tree, x, y, &cx);
            return Action::Redraw;
        }
        if let Some(i) = lay.hit(x, y)
            && self.open != Some(i)
        {
            self.open_title(i, lay, text, m, px, false);
        }
        Action::Redraw
    }

    pub fn hover(&mut self, x: i32, y: i32, lay: &Layout) -> bool {
        let hot = if self.open.is_some() {
            None
        } else {
            lay.hit(x, y)
        };
        std::mem::replace(&mut self.hot, hot) != hot
    }

    pub fn leave(&mut self) -> bool {
        self.hot.take().is_some()
    }

    pub fn draw(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        theme: &crate::theme::Theme,
        lay: &Layout,
    ) {
        let px = theme.fonts.size_px;
        let p = &theme.paint();
        let shown = self.open.or(self.active);
        draw(
            c,
            text,
            p,
            lay,
            &self.titles,
            shown,
            self.hot,
            self.underline,
            self.banded,
            px,
        );
        if let Some(popup) = &self.popup {
            popup.draw(c, theme, text, true);
        }
    }
}

impl Bar {
    pub fn access(&self, lay: &Layout, tree: &mut Tree, parent: NodeId) -> Option<NodeId> {
        let bar = tree.push(parent, "menubar", access::node(Role::MenuBar, lay.bar));
        let mut focus = None;
        for (i, (t, r)) in self.titles.iter().zip(&lay.frames).enumerate() {
            let mut n = access::node(Role::MenuItem, *r);
            n.set_label(t.label.as_str());
            n.set_has_popup(accesskit::HasPopup::Menu);
            n.set_expanded(self.open == Some(i));
            access::focusable(&mut n, true);
            let id = tree.push(bar, i, n);
            if self.open == Some(i)
                && let Some(p) = &self.popup
            {
                focus = Some(p.access(tree, id, "menu"));
            } else if self.active == Some(i) && self.open.is_none() {
                focus = Some(id);
            }
        }
        focus
    }
}

#[cfg(test)]
mod bar_tests {
    use super::*;
    use crate::controls::menu::{MenuItem, MenuTree};
    use crate::theme::{FontConfig, Theme};

    fn bar() -> Bar {
        Bar::new(vec![
            (
                "&File",
                vec![
                    MenuItem::item("&New"),
                    MenuItem::Separator,
                    MenuItem::item("E&xit"),
                ],
            ),
            ("&Edit", vec![MenuItem::item("&Undo")]),
        ])
    }

    fn nested_bar() -> Bar {
        let new = MenuTree::flat(vec![
            MenuItem::item("&Folder"),
            MenuItem::item("&Text Document"),
        ]);
        let mut file = MenuTree::new();
        file.push_sub(MenuItem::item("Ne&w"), new);
        file.push(MenuItem::Separator);
        file.push(MenuItem::item("E&xit"));
        Bar::with_trees(vec![
            ("&File", file),
            ("&Edit", MenuTree::flat(vec![MenuItem::item("&Undo")])),
        ])
    }

    #[test]
    fn keyboard_walks_titles_and_picks_rows() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (m, px) = (&theme, theme.fonts.size_px);
        let mut b = bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        assert_eq!(b.key(Key::Down, &lay, &text, m, px), Action::Ignored);
        assert_eq!(b.key(Key::Activate, &lay, &text, m, px), Action::Redraw);
        assert!(b.underline && b.active == Some(0) && b.open.is_none());
        b.key(Key::Right, &lay, &text, m, px);
        assert_eq!(b.active, Some(1));
        b.key(Key::Down, &lay, &text, m, px);
        assert_eq!((b.open, b.hovered()), (Some(1), Some(0)));
        b.key(Key::Left, &lay, &text, m, px);
        assert_eq!((b.open, b.hovered()), (Some(0), Some(0)));
        b.key(Key::Down, &lay, &text, m, px);
        assert_eq!(b.hovered(), Some(2), "skips the separator");
        assert_eq!(
            b.key(Key::Enter, &lay, &text, m, px),
            Action::Chosen {
                title: 0,
                row: 2,
                path: vec![2]
            }
        );
        assert!(!b.is_active());
        assert_eq!(
            b.key(Key::Char('e', true), &lay, &text, m, px),
            Action::Redraw
        );
        assert_eq!(b.open, Some(1));
        assert_eq!(
            b.key(Key::Char('U', false), &lay, &text, m, px),
            Action::Chosen {
                title: 1,
                row: 0,
                path: vec![0]
            }
        );
        b.key(Key::Char('f', true), &lay, &text, m, px);
        b.key(Key::Escape, &lay, &text, m, px);
        assert_eq!((b.open, b.active), (None, Some(0)));
        b.key(Key::Escape, &lay, &text, m, px);
        assert!(!b.is_active());
    }

    #[test]
    fn a_nested_row_reports_its_whole_path() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (m, px) = (&theme, theme.fonts.size_px);
        let mut b = nested_bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        b.key(Key::Char('f', true), &lay, &text, m, px);
        assert_eq!(b.hovered(), Some(0));
        assert_eq!(b.key(Key::Right, &lay, &text, m, px), Action::Redraw);
        assert_eq!(b.hovered(), Some(0), "the child's first row");
        b.key(Key::Down, &lay, &text, m, px);
        assert_eq!(
            b.key(Key::Enter, &lay, &text, m, px),
            Action::Chosen {
                title: 0,
                row: 0,
                path: vec![0, 1]
            }
        );
        b.key(Key::Char('f', true), &lay, &text, m, px);
        b.key(Key::Right, &lay, &text, m, px);
        b.key(Key::Escape, &lay, &text, m, px);
        assert_eq!(b.open, Some(0));
        assert_eq!(b.hovered(), Some(0));
        b.key(Key::Escape, &lay, &text, m, px);
        assert_eq!((b.open, b.active), (None, Some(0)));
        b.key(Key::Char('f', true), &lay, &text, m, px);
        b.key(Key::Down, &lay, &text, m, px);
        assert_eq!(b.hovered(), Some(2), "Exit");
        b.key(Key::Right, &lay, &text, m, px);
        assert_eq!(b.open, Some(1));
    }

    #[test]
    fn pointer_opens_hovers_and_chooses() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (m, px) = (&theme, theme.fonts.size_px);
        let mut b = bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        let f = lay.frames[0];
        assert_eq!(
            b.press(f.x + 3, f.y + 3, &lay, &text, m, px),
            Action::Redraw
        );
        assert_eq!(b.open, Some(0));
        let popup = b.popup.clone().unwrap();
        let (px0, py0) = popup.levels()[0].origin;
        assert_eq!((px0, py0), lay.popup_origin(0));
        let exit_row = popup.levels()[0].layout.rows[2];
        b.motion(
            px0 + exit_row.x + 5,
            py0 + exit_row.y + 5,
            &lay,
            &text,
            m,
            px,
        );
        assert_eq!(b.hovered(), Some(2));
        assert_eq!(
            b.press(
                px0 + exit_row.x + 5,
                py0 + exit_row.y + 5,
                &lay,
                &text,
                m,
                px
            ),
            Action::Chosen {
                title: 0,
                row: 2,
                path: vec![2]
            }
        );
        b.press(f.x + 3, f.y + 3, &lay, &text, m, px);
        b.press(f.x + 3, f.y + 3, &lay, &text, m, px);
        assert!(b.open.is_none());
        assert_eq!(b.press(150, 150, &lay, &text, m, px), Action::Ignored);
        let mut c = Canvas::new(300, 200);
        b.press(f.x + 3, f.y + 3, &lay, &text, m, px);
        b.draw(&mut c, &text, &theme, &lay);
        assert_eq!(
            c.get(px0 + 1, py0 + 1),
            theme.colors.button_hilight.argb(),
            "popup edge"
        );
    }

    #[test]
    fn hovering_a_nested_row_opens_its_child() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (m, px) = (&theme, theme.fonts.size_px);
        let mut b = nested_bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        let f = lay.frames[0];
        b.press(f.x + 3, f.y + 3, &lay, &text, m, px);
        let popup = b.popup.clone().unwrap();
        let (ox, oy) = popup.levels()[0].origin;
        let new_row = popup.levels()[0].layout.rows[0];
        b.motion(
            ox + new_row.x + 5,
            oy + new_row.y + new_row.height / 2,
            &lay,
            &text,
            m,
            px,
        );
        let popup = b.popup.clone().unwrap();
        assert_eq!(popup.depth(), 2);
        let child = &popup.levels()[1];
        let row = child.layout.rows[0];
        assert_eq!(
            b.press(
                child.origin.0 + row.x + 5,
                child.origin.1 + row.y + 5,
                &lay,
                &text,
                m,
                px
            ),
            Action::Chosen {
                title: 0,
                row: 0,
                path: vec![0, 0]
            }
        );
    }

    #[test]
    fn the_open_menus_row_has_the_focus() {
        use crate::access::{ROOT, Tree, accesskit::Role};
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (m, px) = (&theme, theme.fonts.size_px);
        let mut b = nested_bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        let mut tree = Tree::window("T", (300, 200));
        assert_eq!(b.access(&lay, &mut tree, ROOT), None);

        b.key(Key::Activate, &lay, &text, m, px);
        b.key(Key::Down, &lay, &text, m, px);
        b.key(Key::Right, &lay, &text, m, px);
        let mut tree = Tree::window("T", (300, 200));
        let focus = b
            .access(&lay, &mut tree, ROOT)
            .expect("the menus hold the focus");
        let row = tree.get(focus).unwrap();
        assert_eq!((row.role(), row.label()), (Role::MenuItem, Some("Folder")));
        let new = tree.find(Role::MenuItem, "New").expect("the New row");
        assert_eq!(tree.get(new).unwrap().is_expanded(), Some(true));
        let sub = tree.get(new).unwrap().children()[0];
        assert!(tree.get(sub).unwrap().children().contains(&focus));
        let file = tree.find(Role::MenuItem, "File").expect("the File title");
        assert_eq!(tree.get(file).unwrap().is_expanded(), Some(true));
    }

    #[test]
    fn the_title_under_the_pointer_is_hot_until_a_menu_opens() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let mut b = bar();
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 300, 200),
            &b.titles,
            &MenuTitles::CLASSIC,
        );
        let (f, e) = (lay.frames[0], lay.frames[1]);
        assert!(b.hover(f.x + 2, f.y + 2, &lay));
        assert_eq!(b.hot, Some(0));
        assert!(!b.hover(f.x + 3, f.y + 2, &lay), "same title: no change");
        assert!(b.hover(e.x + 2, e.y + 2, &lay));
        assert_eq!(b.hot, Some(1));
        assert!(b.hover(5, 100, &lay));
        assert_eq!(b.hot, None);
        b.hover(f.x + 2, f.y + 2, &lay);
        assert!(b.leave());
        assert!(!b.leave());
        b.hover(f.x + 2, f.y + 2, &lay);
        b.open_title(0, &lay, &text, &theme, px, false);
        assert_eq!(b.hot, None, "an open menu leaves no title hot");
        assert!(
            !b.hover(e.x + 2, e.y + 2, &lay),
            "nor does hovering while it is down"
        );
    }
}
