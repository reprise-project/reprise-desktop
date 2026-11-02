//! The window's tabs. The open tab lives in the window itself; each other tab keeps its folder,
//! history, view, selection and scroll in a [`TabState`] until it opens again and re-lists.

use crate::place::Place;
use crate::{BTN_LEFT, Explorer};
use repriseapp::{Ctx, Reply};
use reprisefs::history::History;
use reprisekit::access::Tree;
use reprisekit::controls::shellview::{self, Mode};
use reprisekit::controls::tabstrip::{self, AdderPart, DocHit, DocLayout, Drag};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

pub const BTN_MIDDLE: u32 = 0x112;
/// How far a held tab moves before it is dragged, logical px.
const DRAG_PX: i32 = 4;

pub(crate) struct TabState {
    place: Place,
    history: History<Place>,
    mode: Mode,
    sort: Option<(usize, bool)>,
    selected: Vec<PathBuf>,
    /// The view's `top` and `left`.
    scroll: (usize, i32),
}

impl TabState {
    pub(crate) fn at(place: Place, mode: Mode, sort: Option<(usize, bool)>) -> TabState {
        TabState {
            history: History::new(place.clone()),
            place,
            mode,
            sort,
            selected: Vec::new(),
            scroll: (0, 0),
        }
    }
}

/// What a left press in the strip is holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabPress {
    /// A tab, `grab` px right of its left edge, pressed at `x`.
    Tab {
        tab: usize,
        x: i32,
        grab: i32,
    },
    Close(usize),
    Add,
}

impl Explorer {
    /// The strip laid out, while the window has more than one tab.
    pub(crate) fn tab_layout(&self, ctx: &Ctx) -> Option<DocLayout> {
        let area = self.layout(ctx).tabs?;
        let px = ctx.theme.fonts.size_px;
        let strip = &self.tabs.strip;
        Some(tabstrip::doc_layout(
            &ctx.text,
            px,
            strip,
            area,
            ctx.theme.text,
        ))
    }

    pub(crate) fn draw_tabs(&self, c: &mut Canvas, on_glass: bool, ctx: &Ctx) {
        let Some(l) = self.tab_layout(ctx) else {
            return;
        };
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        match on_glass {
            true => c.clear_rect(l.area),
            false => c.fill_rect(l.area, paint.cs.button_face),
        }
        let px = ctx.theme.fonts.size_px;
        tabstrip::doc_draw(c, &ctx.text, &paint, &self.tabs.strip, &l, on_glass, px);
    }

    pub(crate) fn access_tabs(&self, tree: &mut Tree, ctx: &Ctx) {
        if let Some(l) = self.tab_layout(ctx) {
            tabstrip::doc_access(&self.tabs.strip, &l, tree, reprisekit::access::ROOT, "tabs");
        }
    }

    /// The open tab as it stands, its history taken out of the window.
    fn take_tab(&mut self) -> TabState {
        let history = std::mem::replace(&mut self.history, History::new(self.place.clone()));
        TabState {
            place: self.place.clone(),
            history,
            mode: self.sv.mode,
            sort: self.sv.sort,
            selected: self.selected_paths(),
            scroll: (self.sv.top, self.sv.left),
        }
    }

    /// Opens `t` in the window: its folder lists again, then its selection and scroll come back.
    fn put_tab(&mut self, t: TabState, ctx: &mut Ctx) -> Reply {
        self.popup = None;
        self.sv.edit = None;
        self.history = t.history;
        self.sv.set_mode(t.mode);
        self.sv.sort = t.sort;
        self.tab_view = Some((t.place.clone(), t.selected, t.scroll));
        let reply = self.show(t.place, ctx);
        self.follow_tab_view(ctx);
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    /// Gives a tab that opened its selection and scroll back once its listing has landed.
    pub(crate) fn follow_tab_view(&mut self, ctx: &mut Ctx) {
        if self.reading.is_some() {
            return;
        }
        let Some((place, selected, (top, left))) = self.tab_view.take() else {
            return;
        };
        if place != self.place {
            return;
        }
        self.select_paths(&selected);
        self.restore_scroll(top, left, ctx);
        self.refresh_chrome(ctx);
    }

    pub(crate) fn switch_tab(&mut self, to: usize, ctx: &mut Ctx) -> Reply {
        let from = self.tabs.strip.open;
        if to == from || to >= self.tabs.list.len() {
            return Reply::none();
        }
        let blank = TabState::at(self.place.clone(), self.sv.mode, self.sv.sort);
        self.tabs.list[from] = self.take_tab();
        self.tabs.select(to);
        let t = std::mem::replace(&mut self.tabs.list[to], blank);
        self.put_tab(t, ctx)
    }

    /// Ctrl+T: the folder shown again in a new tab with its own history.
    pub(crate) fn new_tab(&mut self, ctx: &mut Ctx) -> Reply {
        let t = TabState::at(self.place.clone(), self.sv.mode, self.sv.sort);
        self.tabs.push(t, self.place.title());
        self.switch_tab(self.tabs.list.len() - 1, ctx)
    }

    /// Middle-click on a folder and its menu's Open in New Tab: a tab behind the open one.
    pub(crate) fn open_in_new_tab(&mut self, place: Place) -> Reply {
        let label = place.title();
        let t = TabState::at(place, self.sv.mode, self.sv.sort);
        self.tabs.push(t, label);
        Reply::redraw_all()
    }

    /// Closes tab `i`; closing the last one closes the window.
    pub(crate) fn close_tab(&mut self, i: usize, ctx: &mut Ctx) -> Reply {
        if self.tabs.list.len() <= 1 {
            return self.close(ctx);
        }
        let open = self.tabs.strip.open;
        self.tabs.close(i);
        self.tab_press = None;
        if i != open {
            return Reply::redraw_all();
        }
        let to = self.tabs.strip.open;
        let blank = TabState::at(self.place.clone(), self.sv.mode, self.sv.sort);
        let t = std::mem::replace(&mut self.tabs.list[to], blank);
        self.put_tab(t, ctx)
    }

    pub(crate) fn cycle_tab(&mut self, back: bool, ctx: &mut Ctx) -> Reply {
        let mut strip = self.tabs.strip.clone();
        if !strip.cycle(back) {
            return Reply::none();
        }
        self.switch_tab(strip.open, ctx)
    }

    /// A press in the strip; `None` when it landed elsewhere.
    pub(crate) fn tab_strip_press(
        &mut self,
        x: i32,
        y: i32,
        button: u32,
        ctx: &mut Ctx,
    ) -> Option<Reply> {
        let l = self.tab_layout(ctx).filter(|l| l.area.contains(x, y))?;
        let hit = tabstrip::doc_hit(&l, x, y);
        Some(match (button, hit) {
            (BTN_MIDDLE, DocHit::Tab(i) | DocHit::Close(i)) => self.close_tab(i, ctx),
            (BTN_LEFT, DocHit::Tab(i)) => {
                let grab = x - l.tabs[i].x;
                self.tab_press = Some(TabPress::Tab { tab: i, x, grab });
                Reply {
                    redraw: true,
                    ..self.switch_tab(i, ctx)
                }
            }
            (BTN_LEFT, DocHit::Close(i)) => {
                let strip = &mut self.tabs.strip;
                (strip.hot_close, strip.held) = (Some(i), true);
                self.tab_press = Some(TabPress::Close(i));
                Reply::redraw()
            }
            (BTN_LEFT, DocHit::Add) => {
                let strip = &mut self.tabs.strip;
                (strip.hot_adder, strip.held) = (Some(AdderPart::Add), true);
                self.tab_press = Some(TabPress::Add);
                Reply::redraw()
            }
            _ => Reply::none(),
        })
    }

    /// The pointer moved: a held tab follows it, else the strip lights what it is over. `Ok`
    /// when the strip took the motion, else whether its lit part changed.
    pub(crate) fn tab_strip_motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Result<Reply, bool> {
        let Some(l) = self.tab_layout(ctx) else {
            return Err(false);
        };
        if let Some(TabPress::Tab { tab, x: x0, grab }) = self.tab_press {
            let strip = &mut self.tabs.strip;
            if strip.dragging.is_none() && (x - x0).abs() >= DRAG_PX {
                strip.dragging = Some(Drag { tab, grab, x });
            }
            if strip.dragging.is_some() {
                self.tabs.drag_to(&l, x);
                return Ok(Reply::redraw());
            }
            return Ok(Reply::none());
        }
        let inside = l.area.contains(x, y);
        let hit = match inside {
            true => tabstrip::doc_hit(&l, x, y),
            false => DocHit::Empty,
        };
        let lit = self.tabs.strip.doc_hover(hit);
        match inside {
            true => Ok(Reply {
                redraw: lit,
                ..Reply::none()
            }),
            false => Err(lit),
        }
    }

    /// The release of a press the strip took; `None` when it took none.
    pub(crate) fn tab_strip_release(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Option<Reply> {
        let press = self.tab_press.take()?;
        let l = self.tab_layout(ctx);
        let hit = l
            .as_ref()
            .filter(|l| l.area.contains(x, y))
            .map_or(DocHit::Empty, |l| tabstrip::doc_hit(l, x, y));
        let strip = &mut self.tabs.strip;
        strip.held = false;
        strip.dragging = None;
        strip.hot_close = None;
        strip.hot_adder = None;
        Some(match press {
            TabPress::Close(i) if hit == DocHit::Close(i) => self.close_tab(i, ctx),
            TabPress::Add if hit == DocHit::Add => self.new_tab(ctx),
            _ => Reply::redraw(),
        })
    }

    /// A middle press: on a tab it closes it, on a folder in the view it opens it behind.
    pub(crate) fn middle_press(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        self.popup = None;
        if let Some(reply) = self.tab_strip_press(x, y, BTN_MIDDLE, ctx) {
            return reply;
        }
        let lay = self.layout(ctx);
        if !lay.view.contains(x, y) || self.sv.edit.is_some() {
            return Reply::none();
        }
        let sl = self.view_layout(lay.view, ctx);
        match shellview::hit(&self.sv, &sl, x, y) {
            shellview::Hit::Entry(i) => match self.items.get(i).and_then(crate::folder_place) {
                Some(place) => self.open_in_new_tab(place),
                None => Reply::none(),
            },
            _ => Reply::none(),
        }
    }

    pub(crate) fn tab_strip_leave(&mut self) {
        if self.tab_press.is_none() {
            self.tabs.strip.doc_hover(DocHit::Empty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Command;
    use crate::tests::{test_ctx, test_explorer};

    fn folders() -> (tempfile::TempDir, [PathBuf; 3]) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let names = ["one", "two", "three"].map(|n| dir.path().join(n));
        for d in &names {
            std::fs::create_dir(d).expect("mkdir");
            for f in ["a.txt", "b.txt", "c.txt"] {
                std::fs::write(d.join(f), "x").expect("write");
            }
        }
        for i in 0..80 {
            std::fs::write(names[1].join(format!("z{i:02}.txt")), "x").expect("write");
        }
        (dir, names)
    }

    fn labels(ex: &Explorer) -> Vec<String> {
        ex.tabs.strip.labels.clone()
    }

    #[test]
    fn ctrl_t_opens_the_same_folder_and_ctrl_w_closes_back_to_one() {
        let (_dir, [one, ..]) = folders();
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(one.clone()), &mut ctx);
        assert_eq!(labels(&ex), ["one"]);
        assert!(ex.layout(&ctx).tabs.is_none(), "one tab shows no strip");
        ex.run(Command::NewTab, &mut ctx);
        assert_eq!(labels(&ex), ["one", "one"]);
        assert_eq!(ex.tabs.strip.open, 1);
        assert_eq!(ex.place, Place::Folder(one.clone()));
        assert!(!ex.history.can_back(), "a new tab starts its own history");
        let lay = ex.layout(&ctx);
        let strip = lay.tabs.expect("two tabs show the strip");
        assert_eq!(strip.y, lay.rebar.bottom());
        assert_eq!(lay.etch_y, strip.bottom(), "the strip closes the rebar");
        ex.run(Command::CloseTab, &mut ctx);
        assert_eq!(labels(&ex), ["one"]);
        let reply = ex.run(Command::CloseTab, &mut ctx);
        assert!(reply.quit, "the last tab closes the window");
    }

    #[test]
    fn each_tab_keeps_its_folder_history_view_selection_and_scroll() {
        let (_dir, [one, two, three]) = folders();
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(one.clone()), &mut ctx);
        ex.go(Place::Folder(two.clone()), &mut ctx);
        ex.run(Command::Mode(Mode::Details), &mut ctx);
        ex.select_paths(&[two.join("b.txt")]);
        ex.restore_scroll(4, 0, &ctx);
        assert_eq!(ex.sv.top, 4, "Details scrolls by rows");

        ex.run(Command::NewTab, &mut ctx);
        ex.go(Place::Folder(three.clone()), &mut ctx);
        ex.run(Command::Mode(Mode::List), &mut ctx);
        ex.select_paths(&[three.join("c.txt")]);
        assert_eq!(labels(&ex), ["two", "three"]);

        ex.run(Command::NextTab { back: true }, &mut ctx);
        assert_eq!(ex.tabs.strip.open, 0);
        assert_eq!(ex.place, Place::Folder(two.clone()));
        assert_eq!(ex.sv.mode, Mode::Details);
        assert_eq!(ex.selected_paths(), [two.join("b.txt")]);
        assert_eq!(ex.sv.top, 4, "its scroll came back");
        assert!(ex.history.can_back(), "the first tab's history came back");
        ex.run(Command::Back, &mut ctx);
        assert_eq!(ex.place, Place::Folder(one.clone()));

        ex.run(Command::NextTab { back: false }, &mut ctx);
        assert_eq!(ex.place, Place::Folder(three.clone()));
        assert_eq!(ex.sv.mode, Mode::List);
        assert_eq!(ex.selected_paths(), [three.join("c.txt")]);
        assert!(ex.history.can_back(), "its own history: two, then three");
        ex.run(Command::NextTab { back: false }, &mut ctx);
        assert_eq!(ex.place, Place::Folder(one), "Ctrl+Tab goes round");
    }

    #[test]
    fn a_folder_opens_in_a_tab_behind_and_closing_the_open_tab_opens_its_neighbour() {
        let (_dir, [one, two, _]) = folders();
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(_dir.path().to_path_buf()), &mut ctx);
        let i = ex.index_of(&two).expect("two is listed");
        ex.sv.selection.set_only(Some(i));
        ex.run(Command::OpenInNewTab, &mut ctx);
        assert_eq!(ex.tabs.strip.open, 0, "the new tab opens behind");
        assert_eq!(ex.place, Place::Folder(_dir.path().to_path_buf()));
        assert_eq!(labels(&ex)[1], "two");

        let lay = ex.layout(&ctx);
        let view = lay.view.inset(reprisekit::classic::widgets::FIELD_PAD);
        let j = ex.index_of(&one).expect("one is listed");
        let sl = ex.view_layout(lay.view, &ctx);
        let cell = sl.cells[j];
        assert!(view.contains(cell.x + 4, cell.y + 4));
        ex.pointer_inner(
            repriseapp::PointerInput::Press {
                x: cell.x + 4,
                y: cell.y + 4,
                button: BTN_MIDDLE,
                time: 0,
                mods: Default::default(),
            },
            &mut ctx,
        );
        assert_eq!(
            labels(&ex)[2],
            "one",
            "a middle-click opens a folder behind"
        );
        assert_eq!(ex.tabs.strip.open, 0);

        ex.switch_tab(1, &mut ctx);
        assert_eq!(ex.place, Place::Folder(two));
        ex.close_tab(1, &mut ctx);
        assert_eq!(ex.place, Place::Folder(one), "its right neighbour opens");
        assert_eq!(labels(&ex).len(), 2);

        let l = ex.tab_layout(&ctx).expect("the strip");
        let t = l.tabs[0];
        ex.pointer_inner(
            repriseapp::PointerInput::Press {
                x: t.x + 10,
                y: t.y + 5,
                button: BTN_MIDDLE,
                time: 0,
                mods: Default::default(),
            },
            &mut ctx,
        );
        assert_eq!(labels(&ex), ["one"], "a middle-click on a tab closes it");
        assert!(ex.layout(&ctx).tabs.is_none());
    }

    #[test]
    fn a_tab_dragged_past_its_neighbour_takes_its_place() {
        let (_dir, [one, two, _]) = folders();
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(one), &mut ctx);
        ex.open_in_new_tab(Place::Folder(two));
        let l = ex.tab_layout(&ctx).expect("the strip");
        let (a, b) = (l.tabs[0], l.tabs[1]);
        let y = a.y + 5;
        let press = repriseapp::PointerInput::Press {
            x: a.x + 10,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Default::default(),
        };
        ex.pointer_inner(press, &mut ctx);
        ex.pointer_inner(
            repriseapp::PointerInput::Motion {
                x: b.x + b.width - 5,
                y,
            },
            &mut ctx,
        );
        assert_eq!(labels(&ex), ["two", "one"]);
        assert_eq!(ex.tabs.strip.open, 1, "the dragged tab stays open");
        ex.pointer_inner(
            repriseapp::PointerInput::Release {
                x: b.x + b.width - 5,
                y,
                button: BTN_LEFT,
                mods: Default::default(),
            },
            &mut ctx,
        );
        assert!(ex.tabs.strip.dragging.is_none());
    }
}
