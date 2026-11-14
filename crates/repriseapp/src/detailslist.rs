//! A Details list as a program's window uses it: the `ListView` and what the pointer and keys do to
//! it (selection, scrollbars, their lit parts, the wheel, a column title, a divider drag), so each
//! window keeps only what its rows mean.

use crate::{BTN_LEFT, Ctx};
use reprisekit::classic::widgets::{Axis, ScrollHit};
use reprisekit::controls::listview::{self, Column, ListView, Row, Rows};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::selection::Mods;
use reprisekit::textedit::keysym;

pub struct List<R: Rows = Vec<Row>> {
    pub view: ListView<R>,
    /// A divider drag: the column, the pointer's x at the press and the column's width then.
    resizing: Option<(usize, i32, i32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Row(usize),
    /// A left double click on the row (Enter's twin).
    Activate(usize),
    Background,
    /// A left press on column `n`'s title.
    Column(usize),
    Redraw,
    None,
}

impl<R: Rows + Default> List<R> {
    pub fn new(columns: Vec<Column>) -> List<R> {
        List {
            view: ListView {
                columns,
                frame: false,
                ..Default::default()
            },
            resizing: None,
        }
    }

    pub fn layout(&self, r: Rect, ctx: &Ctx) -> listview::Layout {
        listview::layout(&self.view, r, &ctx.theme.controls())
    }

    /// A reader asked to see row `row`: it scrolls in and nothing is picked. True when it moved.
    pub fn reveal(&mut self, row: usize, r: Rect, ctx: &Ctx) -> bool {
        let lay = self.layout(r, ctx);
        self.view.reveal(row, &lay)
    }

    pub fn selected(&self) -> Option<usize> {
        self.view.selection.single()
    }

    pub fn select(&mut self, i: Option<usize>, r: Rect, ctx: &Ctx) {
        let lay = self.layout(r, ctx);
        self.view.select(i, &lay);
    }

    fn bar_at(&self, x: i32, y: i32, r: Rect, ctx: &Ctx) -> Option<(Axis, ScrollHit)> {
        if !ctx.styled() || !r.contains(x, y) {
            return None;
        }
        let lay = self.layout(r, ctx);
        let on = |b: Option<Rect>| b.is_some_and(|b| b.contains(x, y));
        if !on(lay.scrollbar) && !on(lay.hscroll) {
            return None;
        }
        self.view.hit(&lay, x, y).bar()
    }

    fn column_at(&self, x: i32, y: i32, r: Rect, ctx: &Ctx) -> Option<usize> {
        if !ctx.styled() || !r.contains(x, y) {
            return None;
        }
        self.view.hit(&self.layout(r, ctx), x, y).column()
    }

    /// A press `at` inside `r`: any button selects the row under it; the left one also works the scrollbars.
    pub fn press(&mut self, at: Click, button: u32, r: Rect, ctx: &Ctx) -> Press {
        let (x, y) = (at.x, at.y);
        let lit = button == BTN_LEFT
            && {
                let over = self.bar_at(x, y, r, ctx);
                let column = self.column_at(x, y, r, ctx);
                self.view.bars.press_bar(over) | self.view.header.press(column, false)
            }
            && ctx.styled();
        let lay = self.layout(r, ctx);
        let moved = |b: bool| if b || lit { Press::Redraw } else { Press::None };
        let hit = self.view.hit(&lay, x, y);
        // Only a left press pairs, so a right click between two left ones does not break the pair.
        let activated = button == BTN_LEFT && self.view.activated(hit, at).is_some();
        match hit {
            listview::Hit::Row(i) => {
                self.view.select(Some(i), &lay);
                if activated {
                    Press::Activate(i)
                } else {
                    Press::Row(i)
                }
            }
            listview::Hit::Background => {
                self.view.select(None, &lay);
                Press::Background
            }
            _ if button != BTN_LEFT => moved(false),
            listview::Hit::Header(n) => Press::Column(n),
            listview::Hit::Divider(i) => {
                let width = self.view.columns.get(i).map_or(0, |c| c.width);
                self.resizing = Some((i, x, width));
                moved(false)
            }
            _ => {
                let press = listview::Input::Press(x, y);
                moved(self.view.pointer(press, &lay, r, false).is_some())
            }
        }
    }

    /// Pointer motion: a held thumb drags, and the part under the pointer lights. True when something moved or lit.
    pub fn motion(&mut self, x: i32, y: i32, r: Rect, ctx: &Ctx) -> bool {
        if let Some((i, from, width)) = self.resizing {
            let u = ctx.theme.controls().text;
            let before = self.view.columns.get(i).map(|c| c.width);
            self.view
                .resize_column(i, u.within(u.len(width) + x - from));
            return self.view.columns.get(i).map(|c| c.width) != before;
        }
        let over = self.bar_at(x, y, r, ctx);
        let column = self.column_at(x, y, r, ctx);
        let lit = (self.view.bars.motion(over) | self.view.header.motion(column)) && ctx.styled();
        if !self.view.thumb.holding() {
            return lit;
        }
        let lay = self.layout(r, ctx);
        let drag = listview::Input::Motion(x, y);
        self.view.pointer(drag, &lay, r, false) == Some(listview::Took::Moved) || lit
    }

    pub fn release(&mut self, ctx: &Ctx) -> bool {
        self.view.thumb.release();
        self.resizing = None;
        let column = self.view.header.holding();
        self.view.header.release();
        (self.view.bars.release_bar() || column) && ctx.styled()
    }

    pub fn leave(&mut self, ctx: &Ctx) -> bool {
        (self.view.bars.leave() | self.view.header.leave()) && ctx.styled()
    }

    pub fn wheel(&mut self, rows: i32, r: Rect, ctx: &Ctx) -> bool {
        let lay = self.layout(r, ctx);
        self.view.scroll(rows as i64, &lay)
    }

    /// The arrows, Home, End and the page keys move the selection; `None` for any other key.
    pub fn key(&mut self, sym: u32, r: Rect, ctx: &Ctx) -> Option<bool> {
        let lay = self.layout(r, ctx);
        let page = lay.rows.max(1) as i64;
        let step = match sym {
            keysym::UP | keysym::KP_UP => -1,
            keysym::DOWN | keysym::KP_DOWN => 1,
            keysym::HOME | keysym::KP_HOME => i64::MIN / 2,
            keysym::END | keysym::KP_END => i64::MAX / 2,
            keysym::PAGE_UP | keysym::KP_PAGE_UP => -page,
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => page,
            _ => return None,
        };
        self.view.focus_ring = true;
        Some(self.view.step(step, &lay, Mods::default()))
    }

    /// Give the last column what the body has left, so the columns fill it.
    pub fn fit_last(&mut self, r: Rect, ctx: &Ctx) {
        let lay = self.layout(r, ctx);
        if let Some(last) = self.view.columns.len().checked_sub(1) {
            self.view.fill_column(last, lay.body.width, lay.text);
        }
    }

    pub fn follow(&mut self, r: Rect, ctx: &Ctx) {
        let lay = self.layout(r, ctx);
        self.view.follow(&lay);
    }

    pub fn draw(&self, c: &mut Canvas, r: Rect, ctx: &Ctx) {
        let lay = self.layout(r, ctx);
        listview::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.view,
            &lay,
            self.view.bars.lit(),
            ctx.theme.fonts.size_px,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BTN_RIGHT;
    use reprisekit::classic::widgets::scrollbar_parts;
    use reprisekit::theme::Theme;

    fn ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (300, 200),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            output_size: None,
            at_pick: false,
            text_size: 100,
        }
    }

    #[test]
    fn the_thumb_drags_the_rows_until_the_release_and_the_track_pages() {
        let ctx = ctx();
        let r = Rect::new(0, 0, 200, 100);
        let mut l: List = List::new(vec![Column::new("Name", 120)]);
        l.view.rows = (0..40)
            .map(|i| Row {
                icon: None,
                cells: vec![format!("row {i}")],
            })
            .collect();
        let lay = l.layout(r, &ctx);
        let sb = lay.scrollbar.expect("a vertical bar");
        let p = scrollbar_parts(sb, 1, 0, lay.rows, l.view.slots(), false, lay.thumb_min);
        let thumb = p.thumb.expect("a thumb");
        let (x, y) = (thumb.x + 2, thumb.y + 3);
        let at = |x, y, time| ctx.theme.metrics.click(x, y, time);

        assert_eq!(l.press(at(x, y, 0), BTN_LEFT, r, &ctx), Press::Redraw);
        assert!(
            l.motion(x + 40, sb.bottom(), r, &ctx),
            "off the bar's side it still steers"
        );
        let end = l.view.top;
        assert_eq!(end, 40 - lay.rows);
        l.release(&ctx);
        assert!(!l.motion(x, sb.y, r, &ctx), "the release lets go");
        assert_eq!(l.view.top, end);

        let back = sb.y + ctx.theme.controls().scroll + 2;
        l.press(at(x, back, 1000), BTN_LEFT, r, &ctx);
        assert_eq!(l.view.top, end - lay.rows, "the track pages back");
        assert_eq!(
            l.press(at(x, back, 2000), BTN_RIGHT, r, &ctx),
            Press::None,
            "only the left button works the bar"
        );
        assert_eq!(l.view.top, end - lay.rows);
    }
}
