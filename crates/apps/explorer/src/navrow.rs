//! The shell layout's nav row and command bar under the pointer: Back, Forward and the recent
//! places, the breadcrumb address (the Address field's edit while it holds the keyboard), the
//! search field with its clear button, and the command bar's commands. Every branch ends in a [`Command`] and [`Explorer::run`], or in
//! the Address field's own events.

use crate::commands::{Command, Menu};
use crate::window::Popup;
use crate::{Explorer, Focus};
use repriseapp::{Ctx, Reply};
use reprisekit::controls::addressbar;
use reprisekit::controls::breadcrumb;
use reprisekit::controls::commandbar;
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::navbuttons;
use reprisekit::controls::searchfield;
use reprisekit::pointer::Shape;

impl Explorer {
    /// The nav row's state for a new place: Back and Forward, the crumbs and the search cue.
    pub(crate) fn sync_nav_place(&mut self, text: &str, icon: Option<reprisekit::raster::Canvas>) {
        let crumbs = crate::chrome::crumbs_for(&self.place, &self.drives, &self.shares.shares);
        self.crumbs.set_place(crumbs, icon, text);
        self.search_crumb = None;
        self.search_field.set_cue(&self.place.title());
        self.nav.can_back = self.can_back();
        self.nav.can_forward = self.history.can_forward();
    }

    /// While a search runs or shows results the crumbs are one segment naming it
    /// ([`crate::chrome::search_crumb`]) behind the magnifier, and the folder's crumbs come back
    /// after it; true when they changed.
    pub(crate) fn sync_search_crumb(&mut self) -> bool {
        let want = self.search.asked().map(|a| &a.place);
        if want == self.search_crumb.as_ref() {
            return false;
        }
        let want = want.cloned();
        self.crumbs.crumbs = match &want {
            Some(p) => vec![crate::chrome::search_crumb(p, &self.shares.shares)],
            None => crate::chrome::crumbs_for(&self.place, &self.drives, &self.shares.shares),
        };
        self.crumbs.searching = want.is_some();
        self.search_crumb = want;
        (self.crumbs.hot, self.crumbs.pressed, self.crumbs.open_menu) = (None, None, None);
        true
    }

    /// A left press on the nav row or the command bar; `None` off them.
    pub(crate) fn nav_row_press(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Option<Reply> {
        let sl = self.shell_layout(ctx)?;
        if sl.command.contains(x, y) {
            let l = self.cmdbar_layout(sl.command, ctx);
            return Some(match commandbar::hit(&self.cmdbar, &l, x, y) {
                Some(hit) => self.command_bar_press(hit, sl.command, false, ctx),
                None => Reply::none(),
            });
        }
        if !sl.nav.contains(x, y) {
            return None;
        }
        let px = ctx.theme.fonts.size_px;
        let nb = sl.buttons;
        if let Some(h) = navbuttons::hit(&nb, x, y) {
            self.nav.can_back = self.can_back();
            self.nav.can_forward = self.history.can_forward();
            return Some(match self.nav.press(Some(h)) {
                navbuttons::Event::OpenRecent => self.open_recent(nb.recent, ctx),
                _ => Reply::redraw(),
            });
        }
        if let Some(h) = searchfield::nav_hit(&self.search_field, sl.search, ctx.theme.text, x, y) {
            if h == searchfield::Hit::Clear {
                self.search_field.press_clear();
            } else {
                self.focus = Focus::Search;
                self.search_field
                    .nav_click(sl.search, ctx.theme.text, x, &ctx.text, px, 1);
            }
            return Some(Reply::redraw());
        }
        if !sl.address.contains(x, y) {
            return Some(Reply::none());
        }
        let cm = ctx.theme.controls();
        if self.focus == Focus::Address {
            let al = addressbar::field_layout(&self.address, sl.address, &cm);
            let hit = addressbar::hit(&self.address, &al, x, y);
            let ev = self.address.click(hit, x, &al, &ctx.text, px, 1);
            return Some(self.address_event(ev, ctx));
        }
        let bl = breadcrumb::layout(&self.crumbs, sl.address, &ctx.text, px, &cm);
        let hit = breadcrumb::hit(&self.crumbs, &bl, x, y);
        if matches!(hit, breadcrumb::Hit::Icon | breadcrumb::Hit::Empty) {
            return Some(self.run(Command::AddressFocus, ctx));
        }
        let ev = self.crumbs.press(hit, x, &bl, &ctx.text, px, 1);
        Some(self.crumb_event(ev, &bl, ctx))
    }

    /// The button came up: Back or Forward fire over the one they went down on, a crumb over
    /// its own segment, the clear button over itself. `None` when the nav row held nothing.
    pub(crate) fn nav_row_release(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Option<Reply> {
        let sl = self.shell_layout(ctx)?;
        if self.nav.pressed.is_some() {
            let h = navbuttons::hit(&sl.buttons, x, y);
            return Some(match self.nav.release(h) {
                navbuttons::Event::Back => self.run(Command::Back, ctx),
                navbuttons::Event::Forward => self.run(Command::Forward, ctx),
                _ => Reply::redraw(),
            });
        }
        if self.search_field.clearing {
            let h = searchfield::nav_hit(&self.search_field, sl.search, ctx.theme.text, x, y);
            if !self.search_field.release(h) {
                return Some(Reply::redraw());
            }
            return Some(self.cancel_search(ctx).unwrap_or_else(Reply::redraw));
        }
        self.crumbs.pressed?;
        let px = ctx.theme.fonts.size_px;
        let bl = breadcrumb::layout(
            &self.crumbs,
            sl.address,
            &ctx.text,
            px,
            &ctx.theme.controls(),
        );
        let ev = self
            .crumbs
            .release(breadcrumb::hit(&self.crumbs, &bl, x, y));
        // The search's one segment searches again.
        if matches!(ev, breadcrumb::Event::Go(_)) && self.search_crumb.is_some() {
            return Some(self.run(Command::Refresh, ctx));
        }
        Some(self.crumb_event(ev, &bl, ctx))
    }

    /// The pointer moved over the window: the nav row's parts light under it. True when one
    /// changed.
    pub(crate) fn nav_row_hover(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let Some(sl) = self.shell_layout(ctx) else {
            return false;
        };
        let px = ctx.theme.fonts.size_px;
        let nb = navbuttons::hit(&sl.buttons, x, y);
        let mut lit = self.nav.hover(nb);
        let l = self.cmdbar_layout(sl.command, ctx);
        let hot = commandbar::hit(&self.cmdbar, &l, x, y);
        lit |= std::mem::replace(&mut self.cmdbar.hot, hot) != hot;
        let sf = searchfield::nav_hit(&self.search_field, sl.search, ctx.theme.text, x, y);
        lit |= self.search_field.hover(sf);
        if sl.address.contains(x, y) && self.focus != Focus::Address {
            let bl = breadcrumb::layout(
                &self.crumbs,
                sl.address,
                &ctx.text,
                px,
                &ctx.theme.controls(),
            );
            lit |= self.crumbs.hover(breadcrumb::hit(&self.crumbs, &bl, x, y));
        } else {
            lit |= self.crumbs.leave();
        }
        lit
    }

    /// The pointer left the window.
    pub(crate) fn nav_row_leave(&mut self) {
        self.nav.leave();
        self.cmdbar.hot = None;
        self.search_field.hover(None);
        self.crumbs.leave();
    }

    /// The pointer's shape over the nav row; `None` off it.
    pub(crate) fn nav_row_shape(&self, x: i32, y: i32, ctx: &Ctx) -> Option<Shape> {
        let sl = self.shell_layout(ctx).filter(|sl| sl.nav.contains(x, y))?;
        if searchfield::nav_hit(&self.search_field, sl.search, ctx.theme.text, x, y)
            == Some(searchfield::Hit::Field)
        {
            return Some(Shape::Text);
        }
        if !sl.address.contains(x, y) {
            return Some(Shape::Default);
        }
        let px = ctx.theme.fonts.size_px;
        let cm = ctx.theme.controls();
        if self.focus == Focus::Address {
            let al = addressbar::field_layout(&self.address, sl.address, &cm);
            return Some(addressbar::hit(&self.address, &al, x, y).shape(&self.address));
        }
        let bl = breadcrumb::layout(&self.crumbs, sl.address, &ctx.text, px, &cm);
        Some(breadcrumb::hit(&self.crumbs, &bl, x, y).shape(&self.crumbs))
    }

    fn crumb_event(
        &mut self,
        ev: breadcrumb::Event,
        bl: &breadcrumb::Layout,
        ctx: &mut Ctx,
    ) -> Reply {
        match ev {
            breadcrumb::Event::Go(text) => self.run(Command::Navigate(text), ctx),
            breadcrumb::Event::Refresh => self.run(Command::Refresh, ctx),
            breadcrumb::Event::Pick(i) => self.address_event(addressbar::Event::Pick(i), ctx),
            breadcrumb::Event::OpenChevron(i) => {
                let seg = bl.segments.iter().find(|s| s.crumb == i);
                let r = seg.map_or(bl.overflow, |s| s.chevron);
                self.open_crumb_menu(i, (r.x, r.bottom()), ctx)
            }
            breadcrumb::Event::OpenOverflow => self.open_folded_crumbs(bl, ctx),
            breadcrumb::Event::OpenHistory => {
                let reply = self.run(Command::AddressFocus, ctx);
                let ev = self
                    .address
                    .list_key(&reprisekit::controls::form::Key::Drop);
                match ev {
                    Some(ev) => reply.and(self.address_event(ev, ctx)),
                    None => reply,
                }
            }
            breadcrumb::Event::None => Reply::none(),
            breadcrumb::Event::Redraw => Reply::redraw(),
        }
    }

    /// The crumbs folded into «, nearest first, each going to its place.
    fn open_folded_crumbs(&mut self, bl: &breadcrumb::Layout, ctx: &mut Ctx) -> Reply {
        let at = (bl.overflow.x, bl.overflow.bottom());
        let Some(popup) = self.folded_crumbs_menu(bl.folded, at, ctx) else {
            return Reply::redraw();
        };
        self.popup = Some(popup);
        Reply::redraw()
    }

    /// The recent places under the drop: those ahead farthest first, this one marked, then
    /// those behind, nearest first.
    fn open_recent(&mut self, under: reprisekit::raster::Rect, ctx: &Ctx) -> Reply {
        let mut m = Menu::default();
        let ahead = self.history.forward_list();
        for (n, p) in ahead.iter().enumerate().rev() {
            m.row(MenuItem::entry(self.title_of(p)), Command::GoForward(n + 1));
        }
        let here = MenuItem::entry(self.title_of(&self.place))
            .checked(true)
            .radio(true);
        m.row(here, Command::Nothing);
        for (title, cmd) in self.back_rows() {
            m.row(MenuItem::entry(title), cmd);
        }
        self.popup = Some(Popup::new((under.x, under.bottom()), m, ctx, false));
        Reply::redraw()
    }
}
