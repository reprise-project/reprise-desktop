//! A Details list as the folder and queue windows use it: the `ListView` and what the pointer and keys
//! do to it (selection, scrollbars, their lit parts, the wheel), so each window keeps only what its rows mean.

use repriseapp::{BTN_LEFT, Ctx};
use reprisekit::classic::widgets::{Axis, BarHit, ScrollHit};
use reprisekit::controls::listview::{self, Column, ListView};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::selection::Mods;
use reprisekit::textedit::keysym;

pub struct List {
    pub view: ListView,
    thumb: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Row(usize),
    Background,
    Redraw,
    None,
}

impl List {
    pub fn new(columns: Vec<Column>) -> List {
        List {
            view: ListView {
                columns,
                frame: false,
                ..Default::default()
            },
            thumb: None,
        }
    }

    pub fn layout(&self, r: Rect, ctx: &Ctx) -> listview::Layout {
        listview::layout(&self.view, r, &ctx.theme.look.controls())
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

    /// A press at (`x`, `y`) inside `r`: any button selects the row under it; the left one also works the scrollbars.
    pub fn press(&mut self, x: i32, y: i32, button: u32, r: Rect, ctx: &Ctx) -> Press {
        let lit = button == BTN_LEFT
            && {
                let over = self.bar_at(x, y, r, ctx);
                let column = self.column_at(x, y, r, ctx);
                self.view.bars.press_bar(over) | self.view.header.press(column, false)
            }
            && ctx.styled();
        let lay = self.layout(r, ctx);
        let moved = |b: bool| if b || lit { Press::Redraw } else { Press::None };
        match self.view.hit(&lay, x, y) {
            listview::Hit::Row(i) => {
                self.view.select(Some(i), &lay);
                Press::Row(i)
            }
            listview::Hit::Background => {
                self.view.select(None, &lay);
                Press::Background
            }
            _ if button != BTN_LEFT => moved(false),
            listview::Hit::VBar(BarHit::Back) => moved(self.view.scroll(-1, &lay)),
            listview::Hit::VBar(BarHit::Forward) => moved(self.view.scroll(1, &lay)),
            listview::Hit::VBar(BarHit::PageBack) => {
                moved(self.view.scroll(-(lay.rows as i64), &lay))
            }
            listview::Hit::VBar(BarHit::PageForward) => {
                moved(self.view.scroll(lay.rows as i64, &lay))
            }
            listview::Hit::VBar(BarHit::Thumb(grab)) => {
                self.thumb = Some(grab);
                moved(false)
            }
            listview::Hit::HBar(BarHit::Back) => moved(self.view.hscroll(-40, &lay)),
            listview::Hit::HBar(BarHit::Forward) => moved(self.view.hscroll(40, &lay)),
            listview::Hit::HBar(BarHit::PageBack) => {
                moved(self.view.hscroll(-lay.body.width, &lay))
            }
            listview::Hit::HBar(BarHit::PageForward) => {
                moved(self.view.hscroll(lay.body.width, &lay))
            }
            _ => moved(false),
        }
    }

    /// Pointer motion: a held thumb drags, and the part under the pointer lights. True when something moved or lit.
    pub fn motion(&mut self, x: i32, y: i32, r: Rect, ctx: &Ctx) -> bool {
        let over = self.bar_at(x, y, r, ctx);
        let column = self.column_at(x, y, r, ctx);
        let lit = (self.view.bars.motion(over) | self.view.header.motion(column)) && ctx.styled();
        let Some(grab) = self.thumb else {
            return lit;
        };
        let lay = self.layout(r, ctx);
        let top = self.view.top_at(&lay, y, grab);
        let moved = top != self.view.top;
        self.view.top = top;
        moved || lit
    }

    pub fn release(&mut self, ctx: &Ctx) -> bool {
        self.thumb = None;
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
        let n = self.view.columns.len();
        if n == 0 {
            return;
        }
        let fixed: i32 = self.view.columns[..n - 1].iter().map(|c| c.width).sum();
        self.view
            .resize_column(n - 1, (lay.body.width - fixed).max(40));
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
