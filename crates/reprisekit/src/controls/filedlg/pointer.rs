use super::{
    DlgMode, Event, FileDlg, Focus, Hit, TOOLBAR_BACK, TOOLBAR_NEW_FOLDER, TOOLBAR_UP,
    TOOLBAR_VIEWS, rows, views_items,
};
use crate::classic::widgets::BarHit;
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::controls::metrics::ControlMetrics;
use crate::controls::placesbar;
use crate::controls::shellview;
use crate::controls::toolbar;
use crate::pointer::{Nav, WHEEL_ROWS};
use crate::raster::Rect;
use crate::selection::Mods;
use crate::text::TextRenderer;

fn combo_click(combo: &mut Combo, h: combo::Hit) {
    match h {
        combo::Hit::Field | combo::Hit::Drop => {
            combo.toggle();
        }
        combo::Hit::Row(i) => {
            combo.pick(i);
        }
        combo::Hit::ScrollUp => {
            combo.scroll(-1);
        }
        combo::Hit::ScrollDown => {
            combo.scroll(1);
        }
        combo::Hit::PageUp => {
            combo.scroll(-(combo.rows as i32));
        }
        combo::Hit::PageDown => {
            combo.scroll(combo.rows as i32);
        }
        combo::Hit::Thumb(_) => {}
    }
}

impl FileDlg {
    pub fn hit(&self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> Option<Hit> {
        let lay_cm = self.drawn.0.get();
        let lay = self.layout(text, px, r, &lay_cm);
        if let Some((p, menu)) = &lay.views {
            return menu.hit(x - p.x, y - p.y).map(Hit::ViewsItem);
        }
        // An open Files of type list is drawn over the extra rows, so it takes
        // the click first unless an extra combo has its list open.
        if self.filetype.open.is_some()
            && !self.extra_list_open()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if let Some((f, _)) = &self.extra
            && let Some(h) = f.hit(
                x - r.x,
                y - rows(r, &lay_cm).y - self.grow(r, &lay_cm),
                text,
                px,
            )
        {
            return Some(Hit::Extra(h));
        }
        if let Some(i) = toolbar::hit(&self.toolbar, &lay.toolbar_rects, x, y) {
            return Some(Hit::ToolBar(i));
        }
        if self.filetype.open.is_some()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if lay.filename.contains(x, y) {
            return Some(Hit::FileName);
        }
        if self.has_types()
            && let Some(h) = combo::hit(&self.filetype, &lay.filetype, x, y)
        {
            return Some(Hit::FileType(h));
        }
        if lay.accept.contains(x, y) {
            return Some(Hit::Accept);
        }
        if lay.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        // Cancel's right end overlaps the grip's square.
        if lay.grip.contains(x, y) {
            return Some(Hit::Grip);
        }
        if let Some(i) = placesbar::hit(&lay.places_rects, x, y) {
            return Some(Hit::Places(i));
        }
        // A dropped Look in list overhangs the shell view, whose `hit` answers
        // `Background` anywhere in its frame, so test the list first.
        if self.lookin.open.is_some()
            && let Some(h) = combo::hit(&self.lookin, &lay.lookin, x, y)
        {
            return Some(Hit::LookIn(h));
        }
        let h = shellview::hit(&self.shell, &lay.shell, x, y);
        if !matches!(h, shellview::Hit::Nothing) {
            return Some(Hit::Shell(h));
        }
        if let Some(h) = combo::hit(&self.lookin, &lay.lookin, x, y) {
            return Some(Hit::LookIn(h));
        }
        None
    }

    /// A reader asked to see entry `row` of the list at (`x`, `y`): the shell view, an open drop
    /// list, or a list on the extra rows, scrolls it in and nothing is picked.
    pub fn reveal(
        &mut self,
        text: &TextRenderer,
        px: u32,
        r: Rect,
        (x, y): (i32, i32),
        row: usize,
    ) -> Event {
        let cm = self.drawn.0.get();
        let moved = match self.hit(text, px, r, x, y) {
            Some(Hit::Shell(_)) => {
                let lay = self.shell_layout(r, &cm);
                shellview::reveal(&mut self.shell, row, &lay)
            }
            Some(Hit::LookIn(_)) => self.lookin.reveal(row),
            Some(Hit::FileType(_)) => self.filetype.reveal(row),
            Some(Hit::Extra(_)) => {
                let (fx, fy) = (x - r.x, y - rows(r, &cm).y - self.grow(r, &cm));
                (self.extra.as_mut())
                    .is_some_and(|(f, _)| f.reveal(fx, fy, row, text, px) != form::Event::None)
            }
            _ => false,
        };
        if moved { Event::Redraw } else { Event::None }
    }

    /// A reader's Focus on the control at (`x`, `y`): the keyboard moves there as Tab or the
    /// access key would, without opening, accepting or cancelling anything.
    pub fn focus_at(&mut self, text: &TextRenderer, px: u32, r: Rect, (x, y): (i32, i32)) -> Event {
        let cm = self.drawn.0.get();
        let focus = match self.hit(text, px, r, x, y) {
            Some(Hit::FileName) => Focus::FileName,
            Some(Hit::FileType(_)) => Focus::FileType,
            Some(Hit::LookIn(_)) => Focus::LookIn,
            Some(Hit::Accept) => Focus::Accept,
            Some(Hit::Cancel) => Focus::Cancel,
            Some(Hit::Shell(_)) => Focus::Shell,
            Some(Hit::Extra(_)) => {
                let (fx, fy) = (x - r.x, y - rows(r, &cm).y - self.grow(r, &cm));
                let Some((f, _)) = &mut self.extra else {
                    return Event::None;
                };
                let moved = f.focus_at(fx, fy, text, px) != form::Event::None;
                if !moved && self.focus == Focus::Extra {
                    return Event::None;
                }
                Focus::Extra
            }
            _ => return Event::None,
        };
        if focus == self.focus && focus != Focus::Extra {
            return Event::None;
        }
        self.lookin.open = None;
        self.filetype.open = None;
        self.focus = focus;
        Event::Redraw
    }

    /// Dispatches a hit from `hit` into the sub-control it named and
    /// returns the resulting `Event`. `r` is what `hit` was given.
    pub fn click(&mut self, r: Rect, hit: Option<Hit>, mods: Mods) -> Event {
        let before = self.filetype.selected;
        let cm = self.drawn.0.get();
        let ev = self.click_inner(r, hit, mods, &cm);
        self.filter_event(before, ev)
    }

    fn click_inner(&mut self, r: Rect, hit: Option<Hit>, mods: Mods, cm: &ControlMetrics) -> Event {
        if self.views_open {
            self.views_open = false;
            return match hit {
                Some(Hit::ViewsItem(i)) => {
                    if let Some((_, mode)) = views_items().get(i) {
                        self.shell.set_mode(*mode);
                    }
                    Event::Redraw
                }
                _ => Event::Redraw,
            };
        }
        match hit {
            Some(Hit::ToolBar(i)) => self.toolbar_click(i),
            Some(Hit::FileName) => {
                self.focus = Focus::FileName;
                Event::Redraw
            }
            Some(Hit::FileType(h)) => {
                self.focus = Focus::FileType;
                combo_click(&mut self.filetype, h);
                Event::Redraw
            }
            Some(Hit::LookIn(h)) => self.lookin_click(h),
            Some(Hit::Accept) => {
                self.focus = Focus::Accept;
                self.activate(true)
            }
            Some(Hit::Cancel) => Event::Cancel,
            Some(Hit::Grip) => Event::Resize,
            Some(Hit::Places(i)) => {
                if !self.places.places.get(i).is_some_and(|p| p.enabled) {
                    return Event::None;
                }
                self.focus = Focus::Places;
                self.places.selected = Some(i);
                self.places.pressed = Some(i);
                Event::NeedPlace(i)
            }
            Some(Hit::Shell(h)) => {
                if let shellview::Hit::Header(i) = h {
                    self.shell.header.press(Some(i), false);
                    shellview::sort_by_column(&mut self.shell, i);
                }
                let lay = self.shell_layout(r, cm);
                self.shell_click(h, &lay, mods)
            }
            Some(Hit::ViewsItem(_)) => Event::None,
            Some(Hit::Extra(h)) => {
                self.focus = Focus::Extra;
                let Some((f, _)) = &mut self.extra else {
                    return Event::None;
                };
                let a = f.click(Some(h));
                let b = f.release();
                if matches!((a, b), (form::Event::None, form::Event::None)) {
                    Event::None
                } else {
                    Event::Redraw
                }
            }
            None => Event::None,
        }
    }

    fn toolbar_click(&mut self, i: usize) -> Event {
        match i {
            TOOLBAR_BACK => self.nav(Nav::Back),
            TOOLBAR_UP => self.up(),
            TOOLBAR_NEW_FOLDER => Event::CreateFolder("New Folder".into()),
            TOOLBAR_VIEWS => {
                self.views_open = true;
                self.views_at = None;
                Event::Redraw
            }
            _ => Event::None,
        }
    }

    fn lookin_click(&mut self, h: combo::Hit) -> Event {
        self.focus = Focus::LookIn;
        if let combo::Hit::Row(i) = h {
            combo_click(&mut self.lookin, h);
            if let Some(c) = self.crumbs.get(i) {
                return Event::NeedListing(c.id);
            }
        } else {
            combo_click(&mut self.lookin, h);
        }
        Event::Redraw
    }

    fn shell_click(&mut self, h: shellview::Hit, lay: &shellview::Layout, mods: Mods) -> Event {
        self.focus = Focus::Shell;
        match h {
            // A hit taken before `set_listing` shrank the folder names no
            // entry, and lands as one on the background.
            shellview::Hit::Entry(i) => {
                // Whatever is selected goes into File name, directory or not:
                // `activate` prefers the field, so a stale name there would
                // accept that file instead of entering the folder.
                if i < self.shell.entries.len() {
                    let mods = self.sel_mods(mods);
                    self.shell.selection.click(i, mods);
                    self.sync_name_field();
                }
            }
            // Both bars' arrows and tracks. The thumbs are not dragged
            // here: the dialog tracks no press but the toolbar's.
            shellview::Hit::VBar(BarHit::Back)
            | shellview::Hit::VBar(BarHit::Forward)
            | shellview::Hit::VBar(BarHit::PageBack)
            | shellview::Hit::VBar(BarHit::PageForward)
            | shellview::Hit::HBar(BarHit::Back)
            | shellview::Hit::HBar(BarHit::Forward)
            | shellview::Hit::HBar(BarHit::PageBack)
            | shellview::Hit::HBar(BarHit::PageForward) => {
                let sv = &mut self.shell;
                match h {
                    shellview::Hit::VBar(BarHit::Back) => shellview::scroll(sv, -1, lay),
                    shellview::Hit::VBar(BarHit::Forward) => shellview::scroll(sv, 1, lay),
                    shellview::Hit::VBar(BarHit::PageBack) => shellview::page(sv, -1, lay),
                    shellview::Hit::VBar(BarHit::PageForward) => shellview::page(sv, 1, lay),
                    shellview::Hit::HBar(BarHit::Back) => shellview::hstep(sv, -1, lay),
                    shellview::Hit::HBar(BarHit::Forward) => shellview::hstep(sv, 1, lay),
                    shellview::Hit::HBar(BarHit::PageBack) => shellview::hpage(sv, -1, lay),
                    _ => shellview::hpage(sv, 1, lay),
                };
            }
            _ => {}
        }
        Event::Redraw
    }

    /// A double click on an entry enters a folder, or accepts a file
    /// (outside folder mode); anywhere else it is one more click.
    pub fn double_click(&mut self, r: Rect, hit: Option<Hit>) -> Event {
        let Some(Hit::Shell(shellview::Hit::Entry(i))) = hit else {
            return self.click(r, hit, Mods::default());
        };
        let Some(entry) = self.shell.entries.get(i) else {
            return Event::None;
        };
        let (id, name, is_dir) = (entry.id, entry.name.clone(), entry.is_dir);
        self.focus = Focus::Shell;
        self.shell.selection.set_only(Some(i));
        if is_dir {
            return self.enter(id, &name);
        }
        if self.fields.mode == DlgMode::Folder {
            return Event::Redraw;
        }
        Event::Accept(vec![name])
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`. An open
    /// list scrolls wherever the pointer is (extra rows' lists go to their
    /// form); otherwise the shell view scrolls, and a focused closed Look in
    /// or Files of type under the pointer steps per notch like Down and Up.
    pub fn wheel(&mut self, r: Rect, hit: Option<Hit>, rows: i32) -> Event {
        let before = self.filetype.selected;
        let ev = self.wheel_inner(r, hit, rows);
        self.filter_event(before, ev)
    }

    fn wheel_inner(&mut self, r: Rect, hit: Option<Hit>, rows: i32) -> Event {
        let redraw = |moved: bool| if moved { Event::Redraw } else { Event::None };
        if self.views_open {
            return Event::None;
        }
        for c in [&mut self.lookin, &mut self.filetype] {
            if c.open.is_some() {
                return redraw(c.scroll(rows));
            }
        }
        if self.extra_list_open() || matches!(hit, Some(Hit::Extra(_))) {
            let h = match hit {
                Some(Hit::Extra(h)) => Some(h),
                _ => None,
            };
            return match &mut self.extra {
                Some((f, _)) => redraw(!matches!(f.wheel(h, rows), form::Event::None)),
                None => Event::None,
            };
        }
        let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
        let step = |c: &mut Combo| {
            let mut changed = false;
            for _ in 0..notches.abs() {
                changed |= c.step(notches > 0);
            }
            changed
        };
        match hit {
            Some(Hit::Shell(_)) => {
                let cm = self.drawn.0.get();
                let lay = self.shell_layout(r, &cm);
                redraw(shellview::scroll(&mut self.shell, rows, &lay))
            }
            Some(Hit::FileType(_)) if self.focus == Focus::FileType => {
                redraw(step(&mut self.filetype))
            }
            Some(Hit::LookIn(_)) if self.focus == Focus::LookIn => {
                if !step(&mut self.lookin) {
                    return Event::None;
                }
                match self.crumbs.get(self.lookin.selected) {
                    Some(c) => Event::NeedListing(c.id),
                    None => Event::Redraw,
                }
            }
            _ => Event::None,
        }
    }

    /// Ends a press on a toolbar button, a Details column title or a place, the only
    /// presses the dialog tracks.
    pub fn release(&mut self) -> Event {
        let column = self.shell.header.holding();
        self.shell.header.release();
        let place = self.places.pressed.take().is_some();
        if self.toolbar.pressed.take().is_some() || ((column || place) && self.drawn.1.get()) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    /// The pointer moved to (`x`, `y`) in `r`: the Details column title or the place
    /// under it lights, or the held title follows it. A redraw only when
    /// that changed and the look draws it lit.
    /// The pointer resting on a cut label of the extra rows times its tip ([`FileDlg::blink`]).
    pub fn motion(&mut self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> Event {
        let hit = self.hit(text, px, r, x, y);
        let at = self.extra_point(r, x, y);
        let hid = (self.extra.as_mut()).is_some_and(|(f, _)| f.rest_pointer(Some(at)));
        match self.hover(hit) {
            Event::None if hid => Event::Redraw,
            e => e,
        }
    }

    /// (`x`, `y`) in `r` as the extra rows' form takes it.
    fn extra_point(&self, r: Rect, x: i32, y: i32) -> (i32, i32) {
        let (ox, oy) = self.extra_origin(r);
        (x - ox, y - oy)
    }

    /// Where the extra rows' form has its (0, 0) when the dialog draws in `r`.
    pub(super) fn extra_origin(&self, r: Rect) -> (i32, i32) {
        let cm = self.drawn.0.get();
        (r.x, rows(r, &cm).y + self.grow(r, &cm))
    }

    /// One caret blink went by: it times the tip of a cut label in the extra rows
    /// ([`form::Form::tip_tick`]), which [`FileDlg::draw`] reports to its canvas.
    pub fn blink(&mut self) -> Event {
        if (self.extra.as_mut()).is_some_and(|(f, _)| f.tip_tick()) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    /// [`FileDlg::motion`] for a host that hit-tested the point already.
    /// An open combo's highlight follows the pointer down its list.
    pub fn hover(&mut self, hit: Option<Hit>) -> Event {
        let followed = match hit {
            Some(Hit::LookIn(combo::Hit::Row(i))) => self.lookin.hover(i),
            Some(Hit::FileType(combo::Hit::Row(i))) => self.filetype.hover(i),
            Some(Hit::Extra(form::Hit {
                item,
                part: form::Part::Combo(combo::Hit::Row(i)),
                ..
            })) => match self.extra.as_mut().and_then(|(f, _)| f.items.get_mut(item)) {
                Some(form::Item {
                    control: form::Control::Combo(c),
                    ..
                }) => c.hover(i),
                _ => false,
            },
            _ => false,
        };
        if followed {
            return Event::Redraw;
        }
        let column = match hit {
            Some(Hit::Shell(shellview::Hit::Header(i))) => Some(i),
            _ => None,
        };
        let place = match hit {
            Some(Hit::Places(i)) => Some(i),
            _ => None,
        };
        let changed = self.shell.header.motion(column) | self.places.hover(place);
        self.lit(changed)
    }

    pub fn leave(&mut self) -> Event {
        let hid = (self.extra.as_mut()).is_some_and(|(f, _)| f.rest_pointer(None));
        let changed = self.shell.header.leave() | self.places.hover(None);
        match self.lit(changed) {
            Event::None if hid => Event::Redraw,
            e => e,
        }
    }

    fn lit(&self, changed: bool) -> Event {
        if changed && self.drawn.1.get() {
            Event::Redraw
        } else {
            Event::None
        }
    }
}
