//! The Properties sheet's host half: what opens it, its window and events, and what OK and Apply
//! write.

use crate::Explorer;
use crate::acting::Action;
use crate::dialogs::SheetId;
use crate::props_dialog::{
    ADVANCED, AttrConfirm, BIN_SHEET_H, CHANGE, ICON_PX, PROPS, Props, READ_ONLY, RESTORE, SHEET_W,
    attr_form, attr_tree_picked, bin_facts, bin_page, folder_has_contents, general_page, rows,
    set_facts,
};
use repriseapp::iconjobs::IconWaits;
use repriseapp::{Child, Ctx, Event, Reply};
use reprisefs::deadline::Gate;
use reprisefs::props::{AttrMoves, FileInfo, SetInfo};
use reprisekit::controls::form::{self, Control};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::{filesheet, iconview};
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set once the sheet that holds it closes, which stops the sheet's folder walk.
#[derive(Default)]
pub(crate) struct WalkStop(Arc<AtomicBool>);

impl WalkStop {
    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for WalkStop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Explorer {
    /// File > Properties, Alt+Enter and the menus' Properties row: the sheet for the single
    /// selected entry, the combined sheet for several, or for the place itself with nothing
    /// selected (the Recycle Bin's is Recycle Bin Properties).
    pub fn properties(&mut self, ctx: &mut Ctx) -> Reply {
        if self.sv.selection.len() > 1 {
            return self.open_set_properties(ctx);
        }
        let chosen = self.sv.selection.single().and_then(|i| self.items.get(i));
        if self.place == crate::place::Place::RecycleBin {
            return match chosen.cloned() {
                Some(it) => self.open_bin_item_properties(it, ctx),
                None => self.open_bin_props(ctx),
            };
        }
        if let Some(it) = chosen {
            let (path, name) = (it.path.clone(), it.label().to_string());
            return self.open_properties(&path, &name, &name, ctx);
        }
        let (place, shares) = (self.place.clone(), self.shares.shares.clone());
        let (apps, globs) = (self.app_index.clone(), self.globs.clone());
        let scale = ctx.icons.scale();
        let work = move |_: &Gate| {
            let (path, name, title) = place_sheet_of(&place, &shares)?;
            Some((glance(&path, &name, &apps, &globs, scale), name, title))
        };
        self.act(
            ctx,
            Action::reads("Properties"),
            work,
            |ex, read, ctx| match read {
                Some((glanced, name, title)) => ex.glanced(glanced, &name, &title, ctx),
                None => Reply::none(),
            },
        )
    }

    /// The path, the sheet field's name and the caption's name for the current place's own sheet,
    /// or `None` where there is no sheet ([`place_sheet_of`]).
    #[cfg(test)]
    pub(crate) fn place_sheet(&self) -> Option<(PathBuf, String, String)> {
        place_sheet_of(&self.place, &self.shares.shares)
    }

    /// The sheet for `path`, its field showing `name` and its caption `title`, once a listing
    /// thread has read it.
    pub(crate) fn open_properties(
        &mut self,
        path: &Path,
        name: &str,
        title: &str,
        ctx: &mut Ctx,
    ) -> Reply {
        let (path, name, title) = (path.to_path_buf(), name.to_string(), title.to_string());
        let (apps, globs) = (self.app_index.clone(), self.globs.clone());
        let read = name.clone();
        let scale = ctx.icons.scale();
        let work = move |_: &Gate| glance(&path, &read, &apps, &globs, scale);
        self.act(
            ctx,
            Action::reads("Properties"),
            work,
            move |ex, glanced, ctx| ex.glanced(glanced, &name, &title, ctx),
        )
    }

    /// `--properties PATH`'s sheet, for a path no listing shows: its name, a launcher's or a
    /// Start menu entry's own included, is read with the rest of the sheet.
    pub(crate) fn open_path_properties(&mut self, path: &Path, ctx: &mut Ctx) -> Reply {
        let path = path.to_path_buf();
        let (apps, globs) = (self.app_index.clone(), self.globs.clone());
        let scale = ctx.icons.scale();
        let work = move |_: &Gate| {
            let name = match menu_entry(&path, &apps) {
                Some(e) => e.name.clone(),
                None => reprisefs::listing::item_at(&path, &globs)
                    .label()
                    .to_string(),
            };
            (glance(&path, &name, &apps, &globs, scale), name)
        };
        self.act(
            ctx,
            Action::reads("Properties"),
            work,
            |ex, (glanced, name), ctx| ex.glanced(glanced, &name, &name, ctx),
        )
    }

    /// `--properties PATH...`'s sheet: the combined sheet when two or more of `paths` read, else
    /// the first path's own.
    pub(crate) fn open_paths_properties(&mut self, paths: &[PathBuf], ctx: &mut Ctx) -> Reply {
        let [first, ..] = paths else {
            return Reply::none();
        };
        if paths.len() == 1 {
            return self.open_path_properties(first, ctx);
        }
        let (paths, globs) = (paths.to_vec(), self.globs.clone());
        let apps = self.app_index.clone();
        let work = move |_: &Gate| {
            let chosen: Vec<(PathBuf, String)> = (paths.iter())
                .map(|p| {
                    let label = reprisefs::listing::item_at(p, &globs).label().to_string();
                    (p.clone(), label)
                })
                .collect();
            (glance_set_items(&chosen, &apps, &globs), paths)
        };
        self.act(
            ctx,
            Action::reads("Properties"),
            work,
            |ex, (items, paths), ctx| match items.first() {
                Some(first) if items.len() > 1 => {
                    let caption = format!("{}, ...", first.name);
                    ex.open_sheet(SheetRead::Set(items, caption), ctx)
                }
                _ => ex.open_path_properties(&paths[0], ctx),
            },
        )
    }

    fn glanced(
        &mut self,
        glanced: std::io::Result<Glanced>,
        name: &str,
        title: &str,
        ctx: &mut Ctx,
    ) -> Reply {
        match glanced {
            Ok(g) => self.open_sheet(SheetRead::One(g, title.to_string()), ctx),
            Err(e) => {
                let text = format!("Cannot show the properties of {name}: {e}");
                self.message_box("Properties", text, ctx)
            }
        }
    }

    fn open_bin_item_properties(&mut self, it: reprisefs::listing::Item, ctx: &mut Ctx) -> Reply {
        let (path, name) = (it.path.clone(), it.name.clone());
        let (apps, globs) = (self.app_index.clone(), self.globs.clone());
        let scale = ctx.icons.scale();
        let work = move |_: &Gate| glance(&path, &name, &apps, &globs, scale);
        self.act(
            ctx,
            Action::reads("Properties"),
            work,
            move |ex, glanced, ctx| ex.bin_glanced(glanced, &it, ctx),
        )
    }

    fn bin_glanced(
        &mut self,
        glanced: std::io::Result<Glanced>,
        it: &reprisefs::listing::Item,
        ctx: &mut Ctx,
    ) -> Reply {
        match glanced {
            Ok(g) => self.open_sheet(SheetRead::Bin(g, it.clone()), ctx),
            Err(e) => {
                let text = format!("Cannot show the properties of {}: {e}", it.name);
                self.message_box("Properties", text, ctx)
            }
        }
    }

    /// Puts up the sheet `read` builds once every theme icon it wears is ready: till then it
    /// waits in `props_due` for them to land off the loop ([`repriseapp::iconjobs`]).
    fn open_sheet(&mut self, read: SheetRead, ctx: &mut Ctx) -> Reply {
        self.due_sheet(read, None, ctx)
    }

    /// [`Explorer::open_sheet`] for the sheet read again after a write for `was`
    /// ([`Explorer::sheet_showing`]): the boxes it raised stay up over it. The read is dropped
    /// once no such sheet is up, or while another file's sheet waits to replace it.
    fn refresh_sheet(
        &mut self,
        id: SheetId,
        was: &[PathBuf],
        read: SheetRead,
        ctx: &mut Ctx,
    ) -> Reply {
        let Some(id) = self.sheet_showing(id, was) else {
            return Reply::redraw_all();
        };
        if matches!(self.props_due, Some((_, _, None))) {
            return Reply::redraw_all();
        }
        self.due_sheet(read, Some(id.opened), ctx)
    }

    /// The sheet a write for `was` that sheet `id` asked for reads again: `id` while it is up,
    /// else a sheet opened since on the same `was`.
    fn sheet_showing(&self, id: SheetId, was: &[PathBuf]) -> Option<SheetId> {
        let p = self.props.as_ref()?;
        let shown = match &p.set {
            Some(set) => &set.paths[..],
            None => std::slice::from_ref(&p.info.path),
        };
        (self.sheet_up(id) || shown == was).then(|| p.id())
    }

    fn due_sheet(&mut self, read: SheetRead, refresh: Option<u64>, ctx: &mut Ctx) -> Reply {
        if let SheetRead::One(g, _) | SheetRead::Bin(g, _) = &read {
            ctx.icons.put_files_at(g.files.clone(), g.scale);
        }
        self.props_due = Some((read, IconWaits::default(), refresh));
        self.build_due(ctx)
    }

    /// The waiting sheet goes up if its icons are ready, and its folder walks start.
    fn build_due(&mut self, ctx: &mut Ctx) -> Reply {
        let Some((read, _, refresh)) = self.props_due.take() else {
            return Reply::none();
        };
        if refresh.is_some_and(|n| self.props.as_ref().map(|p| p.opened) != Some(n)) {
            return Reply::none();
        }
        let mut waits = IconWaits::default();
        let mut props = match &read {
            SheetRead::One(g, title) => Some(self.build_props(g, title.clone(), &mut waits, ctx)),
            SheetRead::Bin(g, it) => Some(self.build_bin_props(g, it, &mut waits, ctx)),
            SheetRead::Set(items, caption) => {
                self.build_set_props(items, caption.clone(), &mut waits, ctx)
            }
        };
        if !waits.is_empty() {
            self.props_due = Some((read, waits, refresh));
            return Reply::none();
        }
        // The sheet this one replaces goes the one way a sheet goes, taking its boxes.
        let reply = match self.props {
            Some(_) if refresh.is_none() => self.close_props(ctx),
            _ => Reply::redraw_all(),
        };
        if let Some(p) = &mut props {
            p.opened = refresh.unwrap_or_else(|| self.next_sheet());
        }
        self.props = props;
        match (&read, self.props.as_ref().and_then(|p| p.set.clone())) {
            (SheetRead::Set(..), Some(set)) => self.walk_set_for_props(set, ctx),
            (SheetRead::One(g, _) | SheetRead::Bin(g, _), _) => self.walk_for_props(&g.info, ctx),
            _ => {}
        }
        reply
    }

    /// A sheet waiting for its icons goes up once they landed.
    pub(crate) fn take_due_sheet(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let (_, waits, _) = self.props_due.as_ref()?;
        waits.landed(&mut ctx.icons).then(|| self.build_due(ctx))
    }

    fn build_bin_props(
        &self,
        g: &Glanced,
        it: &reprisefs::listing::Item,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
    ) -> Props {
        let info = g.info.clone();
        let is_12h = reprisefs::locale::is_12h();
        let facts = bin_facts(&info, it.trashed.as_deref(), is_12h, &self.shares.shares);
        let icon = props_icon(&g.icon, &info.path, waits, ctx);
        let page = bin_page(&facts, &it.name, icon);
        let sheet = PropSheet::new(&["General"], vec![page], (SHEET_W, BIN_SHEET_H));
        Props {
            opened_read_only: filesheet::read_only(&sheet.pages[0]),
            opened_name: String::new(),
            caption: it.name.clone(),
            restore: Some(crate::fileops::BinItem {
                path: it.path.clone(),
                record: it.trashed.as_deref().cloned(),
            }),
            info,
            set: None,
            robots: None,
            adv: None,
            summary: None,
            own_icon: None,
            icon_chosen: None,
            picks: Default::default(),
            checksums: Default::default(),
            walk: Default::default(),
            opened: 0,
            sheet,
        }
    }

    fn walk_for_props(&mut self, info: &FileInfo, ctx: &mut Ctx) {
        let Some(stop) = self.props.as_ref().map(|p| p.walk.flag()) else {
            return;
        };
        if info.is_dir && info.contains.is_none() {
            let path = info.path.clone();
            self.spawn_read_job(ctx, move || {
                let totals = reprisefs::props::folder_totals_unless(&path, &stop);
                Box::new(move |ex: &mut Explorer, _: &mut Ctx| {
                    if let Some(p) = ex.props.as_mut()
                        && let Some(totals) = totals
                        && p.set.is_none()
                        && p.info.path == path
                        && p.info.contains.is_none()
                    {
                        p.set_totals(totals);
                    }
                    Reply::redraw_all()
                })
            });
        }
    }

    fn build_props(
        &self,
        glanced: &Glanced,
        title: String,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
    ) -> Props {
        let info = glanced.info.clone();
        let rows = rows(&info, reprisefs::locale::is_12h());
        let icon = props_icon(&glanced.icon, &info.path, waits, ctx);
        let icon = match info.is_shortcut {
            true => icon.map(iconview::with_shortcut_arrow),
            false => icon,
        };
        let app =
            (info.opens_with_icon.as_deref()).and_then(|n| waits.first(&mut ctx.icons, &[n], 16));
        // The Hidden box stands for the leading dot, so the field leaves it out.
        let shown = filesheet::shown_name(&info.file_name(), &info.hidden_ext, info.hidden);
        let (page, height) = general_page(&rows, &shown, icon.clone(), app);
        let over = crate::props_dialog::over(&info);
        let mut sheet = self.tabbed_sheet(page, height, over, waits, ctx, |tab| {
            match (tab, &info.launcher) {
                (filesheet::Tab::Shortcut, Some(l)) => {
                    filesheet::shortcut_page(&shortcut_facts(&info, l), icon.clone())
                }
                (filesheet::Tab::WebDocument, Some(l)) => {
                    let facts = filesheet::WebFacts {
                        name: info.name.clone(),
                        url: l.target.clone(),
                    };
                    filesheet::web_page(&facts, icon.clone())
                }
                _ => filesheet::summary_page(info.summary.as_ref()),
            }
        });
        if info.is_dir {
            let own = info.own_icon.as_deref();
            wear_folder_icon(
                &mut sheet,
                own.unwrap_or(filesheet::STANDARD_FOLDER_ICON),
                waits,
                ctx,
            );
        }
        Props {
            opened_read_only: filesheet::read_only(&sheet.pages[0]),
            opened_name: shown,
            caption: title,
            restore: None,
            set: None,
            robots: filesheet::robots_states(info.robots.map(|r| r.map(Some))),
            adv: None,
            summary: info.summary.clone(),
            own_icon: match &info.launcher {
                Some(l) => l.icon.clone(),
                None => info.own_icon.clone(),
            },
            icon_chosen: None,
            picks: Default::default(),
            checksums: Default::default(),
            walk: Default::default(),
            opened: 0,
            info,
            sheet,
        }
    }

    fn open_set_properties(&mut self, ctx: &mut Ctx) -> Reply {
        use crate::place::Place;
        if matches!(
            self.place,
            Place::Computer | Place::RecycleBin | Place::Network(_)
        ) {
            return Reply::none();
        }
        let sel = &self.sv.selection;
        let first = sel.focus.filter(|&i| sel.is(i));
        let order = first
            .into_iter()
            .chain(sel.set.iter().copied().filter(|&i| Some(i) != first));
        let chosen: Vec<(PathBuf, String)> = order
            .filter_map(|i| self.items.get(i))
            .map(|it| (it.path.clone(), it.label().to_string()))
            .collect();
        let (apps, globs) = (self.app_index.clone(), self.globs.clone());
        let work = move |_: &Gate| glance_set_items(&chosen, &apps, &globs);
        self.act(ctx, Action::reads("Properties"), work, |ex, items, ctx| {
            let Some(first) = items.first() else {
                return Reply::none();
            };
            let caption = format!("{}, ...", first.name);
            ex.open_sheet(SheetRead::Set(items, caption), ctx)
        })
    }

    /// The combined sheet's walk of its folders' totals, while it is up.
    fn walk_set_for_props(&mut self, set: SetInfo, ctx: &mut Ctx) {
        let Some(stop) = self.props.as_ref().map(|p| p.walk.flag()) else {
            return;
        };
        if set.walking {
            let paths = set.paths;
            self.spawn_read_job(ctx, move || {
                let sum = paths.iter().filter(|p| p.is_dir()).try_fold(
                    reprisefs::props::FolderTotals::default(),
                    |mut sum, d| {
                        let t = reprisefs::props::folder_totals_unless(d, &stop)?;
                        sum.files += t.files;
                        sum.folders += t.folders;
                        sum.size += t.size;
                        sum.on_disk += t.on_disk;
                        Some(sum)
                    },
                );
                Box::new(move |ex: &mut Explorer, _: &mut Ctx| {
                    if let Some(p) = ex.props.as_mut()
                        && let Some(sum) = sum
                        && p.set
                            .as_ref()
                            .is_some_and(|s| s.paths == paths && s.walking)
                    {
                        p.set_totals(sum);
                    }
                    Reply::redraw_all()
                })
            });
        }
    }

    fn build_set_props(
        &self,
        items: &[FileInfo],
        caption: String,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
    ) -> Option<Props> {
        let set = reprisefs::props::glance_set(items);
        let info = items.first()?.clone();
        let icon = Some(filesheet::set_picture(ctx.icons.scale()));
        let facts = set_facts(&set);
        let (page, height) = general_page(&filesheet::set_rows(&facts), "", icon, None);
        let over = filesheet::Over::Set {
            document: set.has_document,
            shortcut: set.has_shortcut,
            folder: set.has_dir,
        };
        let mut sheet = self.tabbed_sheet(page, height, over, waits, ctx, |_| {
            filesheet::summary_page(None)
        });
        wear_folder_icon(&mut sheet, filesheet::STANDARD_FOLDER_ICON, waits, ctx);
        Some(Props {
            opened_read_only: facts.read_only,
            opened_name: String::new(),
            caption,
            restore: None,
            info,
            robots: filesheet::robots_states(set.robots),
            adv: None,
            summary: None,
            own_icon: None,
            icon_chosen: None,
            picks: Default::default(),
            checksums: Default::default(),
            walk: Default::default(),
            opened: 0,
            set: Some(set),
            sheet,
        })
    }

    fn tabbed_sheet(
        &self,
        general: form::Form,
        height: i32,
        over: filesheet::Over,
        waits: &mut IconWaits,
        ctx: &mut Ctx,
        own: impl FnMut(filesheet::Tab) -> form::Form,
    ) -> PropSheet {
        let tabs = filesheet::sheet_tabs(over);
        let mut sheet = filesheet::sheet(general, height, &tabs, own);
        filesheet::fill_pictures(&mut sheet, |n| waits.first(&mut ctx.icons, &[n], ICON_PX));
        sheet
    }

    fn change_icon(&mut self, ctx: &mut Ctx) -> Reply {
        use reprisekit::controls::filesheet::FOLDER_ICON_NAMES;
        let Some(p) = self.props.as_ref() else {
            return Reply::none();
        };
        let launcher = p.info.launcher.is_some();
        let current = match launcher {
            true => p.icon_chosen.clone().unwrap_or_else(|| p.own_icon.clone()),
            false => Some(p.shown_icon()),
        };
        let name = p
            .info
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut names: Vec<String> = if launcher {
            reprisefs::launcher::icon_names(&self.app_index, |n| ctx.icons.has_unread(n))
        } else {
            FOLDER_ICON_NAMES
                .iter()
                .filter(|n| ctx.icons.has_unread(n))
                .map(|n| n.to_string())
                .collect()
        };
        let standard = (!launcher).then(|| filesheet::STANDARD_FOLDER_ICON.to_string());
        for must in [standard, current.clone()].into_iter().flatten() {
            if !names.contains(&must) {
                names.insert(0, must);
            }
        }
        let mut waits = IconWaits::default();
        let pictures = (names.iter())
            .map(|n| waits.first(&mut ctx.icons, &[n], ICON_PX))
            .collect();
        let mut list = reprisekit::controls::iconlist::IconList::new(names, pictures, None);
        if let Some(c) = &current {
            list.pick(c);
        }
        let form = filesheet::change_icon_form(&ctx.icons.theme_name(), list, !launcher);
        self.raise(crate::dialogs::Dialog::ChangeIcon { form, name, waits })
    }

    pub(crate) fn icon_chosen(&mut self, icon: Option<String>, ctx: &mut Ctx) -> Reply {
        if let Some(p) = self.props.as_mut() {
            p.icon_chosen = Some(icon);
            wear_picks(p, ctx);
            let on = p.dirty();
            p.sheet.set_apply_enabled(on);
        }
        Reply::redraw_all()
    }

    /// The sheet wears the icons its picks waited on once they landed: true when it did.
    pub(crate) fn land_sheet_icons(&mut self, ctx: &mut Ctx) -> bool {
        let Some(p) = self.props.as_mut() else {
            return false;
        };
        let landed = p.picks.waits.landed(&mut ctx.icons);
        if landed {
            wear_picks(p, ctx);
        }
        landed
    }

    /// Whether a process started for one box (`--properties`, `--open-with`, `--bin-properties`)
    /// is done: its sheet is closed and no box or work is left.
    pub(crate) fn sheet_closed(&self) -> bool {
        self.props.is_none()
            && self.bin_props.is_none()
            && self.props_due.is_none()
            && self.dialogs.is_empty()
            && !self.busy()
    }

    /// The sheet's child window, while it is up.
    pub fn props_child(&self) -> Option<Child> {
        let p = self.props.as_ref()?;
        Some(Child {
            id: PROPS,
            title: p.title(),
            size: self.forms.text.size(p.size()),
            fixed: true,
            modal: false,
        })
    }

    pub fn draw_props(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if let Some(p) = &self.props {
            propsheet::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..forms.paint()
                },
                &p.sheet,
                true,
                forms.fonts.size_px,
            );
        }
    }

    /// Closes the sheet, the one way it goes. Its boxes, up or waiting, leave first with their
    /// cancel answer ([`Explorer::dismiss`]), so none is answered for a sheet that is gone.
    pub(crate) fn close_props(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::redraw_all();
        for d in (self.dialogs).withdraw(|d| d.sheet() == Some(PROPS)) {
            reply = reply.and(self.dismiss(d, ctx));
        }
        self.props = None;
        reply
    }

    /// The sheet's event loop.
    pub fn props_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            return self.close_props(ctx);
        }
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        let clip = repriseapp::keys::form_clipboard(p.sheet.open().focused_edit(), &ev);
        let out = match ev {
            Event::Blink => {
                let on = !p.sheet.open().caret;
                p.sheet.open_mut().blink(on);
                return Reply::redraw();
            }
            Event::Pasted(text) => p.sheet.paste(text),
            ev => match repriseapp::keys::sheet_input(&mut p.sheet, &ev, ctx) {
                Some(ev) => ev,
                None => propsheet::Event::None,
            },
        };
        let reply = self.props_outcome(out, ctx).with_clipboard(clip);
        reply.and(self.checksums_opened(ctx))
    }

    pub(crate) fn props_outcome(&mut self, out: propsheet::Event, ctx: &mut Ctx) -> Reply {
        match out {
            propsheet::Event::None | propsheet::Event::Link(_) | propsheet::Event::Search => {
                Reply::none()
            }
            propsheet::Event::Redraw => Reply::redraw(),
            propsheet::Event::Cancel => self.close_props(ctx),
            propsheet::Event::Ok => self.apply_props(true, ctx),
            propsheet::Event::Apply => self.apply_props(false, ctx),
            propsheet::Event::Page(_, ev) => {
                if let Some(r) = self.checksums_event(ev, ctx) {
                    return r;
                }
                if let form::Event::Pressed(CHANGE) = ev {
                    let path = self.props.as_ref().map(|p| p.info.path.clone());
                    if let Some(path) = path {
                        return self.change_default(&path, ctx);
                    }
                }
                if let form::Event::Pressed(RESTORE) = ev
                    && let Some(item) = self.props.as_ref().and_then(|p| p.restore.clone())
                {
                    let closed = self.close_props(ctx);
                    return closed.and(self.restore_items(vec![item], ctx));
                }
                if let form::Event::Pressed(filesheet::CHANGE_ICON) = ev {
                    return self.change_icon(ctx);
                }
                if let form::Event::Pressed(filesheet::FIND_TARGET) = ev {
                    return self.find_target(ctx);
                }
                if let form::Event::Pressed(ADVANCED) = ev {
                    let subject = match self.props.as_ref() {
                        Some(Props { set: Some(s), .. }) => {
                            filesheet::Subject::Set { has_dir: s.has_dir }
                        }
                        p => filesheet::Subject::single(p.is_some_and(|p| p.info.is_dir)),
                    };
                    let robots = self.props.as_ref().and_then(|p| p.robots_shown());
                    return self.advanced_attributes(subject, robots, ctx);
                }
                if let Some(p) = self.props.as_mut() {
                    let on = p.dirty();
                    p.sheet.set_apply_enabled(on);
                }
                Reply::redraw()
            }
        }
    }

    fn find_target(&mut self, ctx: &mut Ctx) -> Reply {
        let Some((l, page, path, sheet)) = self.props.as_ref().and_then(|p| {
            let page = filesheet::shortcut_of(&p.sheet.pages)?;
            Some((p.info.launcher.clone()?, page, p.info.path.clone(), p.id()))
        }) else {
            return Reply::none();
        };
        let [target, ..] = filesheet::shortcut_values(page);
        let work = move |_: &Gate| match l.find(&target, &path) {
            Some(found) => Ok(found),
            None => Err(l.not_found_text(&target)),
        };
        let title = "Problem with Shortcut";
        self.act(
            ctx,
            Action::reads(title).of(sheet),
            work,
            move |ex, found, ctx| match found {
                Ok(found) => Reply {
                    spawn: vec![vec![
                        "reprise-explorer".into(),
                        "--select".into(),
                        found.into_os_string(),
                    ]],
                    ..Reply::none()
                },
                Err(text) => ex.message_over(Some(sheet), title, text, ctx),
            },
        )
    }

    fn apply_props(&mut self, close: bool, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.as_ref() else {
            return Reply::none();
        };
        let sheet = p.id();
        if let Some(set) = &p.set {
            let (paths, attrs, hidden) = (set.paths.clone(), p.attrs(), p.hidden());
            let Some(icon) = p.icon_chosen.clone() else {
                return self.apply_set_props(paths, attrs, hidden, close, sheet, ctx);
            };
            let dirs = paths.clone();
            let work = move |gate: &Gate| {
                if !dirs.first().is_some_and(|d| gate.change(d)) {
                    return;
                }
                for dir in dirs.iter().filter(|d| d.is_dir()) {
                    if let Err(e) = reprisefs::meta::set_folder_icon(dir, icon.as_deref()) {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            dir.display()
                        );
                    }
                }
            };
            return self.act(
                ctx,
                Action::changes("Properties").of(sheet),
                work,
                move |ex, (), ctx| {
                    if attrs.is_empty() && hidden.is_none() {
                        return ex.finish_set_props(paths.clone(), &paths, close, sheet, ctx);
                    }
                    ex.apply_set_props(paths, attrs, hidden, close, sheet, ctx)
                },
            );
        }
        let new_name = match p.new_name() {
            Ok(name) => name,
            Err(text) => return self.message_over(Some(sheet), "Rename", text, ctx),
        };
        let (attrs, hidden) = (p.attrs(), p.hidden());
        let summary = p.summary_moves();
        let folder_icon = p.folder_icon_move();
        let launcher = p.launcher_edits();
        if new_name.is_none()
            && attrs.is_empty()
            && hidden.is_none()
            && summary.is_empty()
            && folder_icon.is_none()
            && launcher.is_empty()
        {
            if close {
                return self.close_props(ctx);
            }
            return Reply::none();
        }
        let info = p.info.clone();
        let (path, name) = (p.info.path.clone(), p.info.name.clone());
        let look_inside = (!attrs.is_empty() || hidden.is_some()) && p.info.is_dir;
        let at = path.clone();
        let work = move |gate: &Gate| -> Result<bool, String> {
            if !gate.change(&at) {
                return Ok(false);
            }
            // The description goes first, while the file is where the sheet found it; the rest
            // may rename it.
            if let Err(e) = reprisefs::meta::write_summary(&at, &summary) {
                return Err(format!("Cannot change {name}: {e}"));
            }
            if !launcher.is_empty()
                && let Err(e) = reprisefs::launcher::write_keys(&at, &launcher)
            {
                return Err(format!("Cannot change {name}: {e}"));
            }
            if let Some(icon) = &folder_icon
                && let Err(e) = reprisefs::meta::set_folder_icon(&at, icon.as_deref())
            {
                return Err(format!("Cannot change the icon of {name}: {e}"));
            }
            Ok(look_inside && folder_has_contents(&at))
        };
        self.act(
            ctx,
            Action::changes("Properties").of(sheet),
            work,
            move |ex, pre, ctx| {
                let ask = match pre {
                    Ok(ask) => ask,
                    Err(text) => return ex.message_over(Some(sheet), "Properties", text, ctx),
                };
                if new_name.is_none() && attrs.is_empty() && hidden.is_none() {
                    return ex.finish_props(&path, &path, close, sheet, ctx);
                }
                if !ask {
                    return ex.write_props(info, sheet, new_name, attrs, hidden, false, close, ctx);
                }
                // The question goes with its sheet ([`Explorer::close_props`]).
                if !ex.sheet_up(sheet) {
                    return ex.finish_props(&path, &path, close, sheet, ctx);
                }
                let form = attr_form(
                    &attrs,
                    hidden,
                    filesheet::Subject::Folder,
                    &ex.dialog_text,
                    ex.forms.fonts.size_px,
                    ex.forms.text,
                );
                ex.raise(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
                    set: None,
                    new_name,
                    attrs,
                    hidden,
                    close,
                    form,
                }))
            },
        )
    }

    fn apply_set_props(
        &mut self,
        paths: Vec<PathBuf>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        close: bool,
        sheet: SheetId,
        ctx: &mut Ctx,
    ) -> Reply {
        if attrs.is_empty() && hidden.is_none() {
            if close {
                return self.close_props(ctx);
            }
            return Reply::none();
        }
        let looked = paths.clone();
        let work = move |_: &Gate| looked.iter().any(|p| p.is_dir() && folder_has_contents(p));
        self.act(
            ctx,
            Action::reads("Properties").of(sheet),
            work,
            move |ex, ask, ctx| {
                if !ask {
                    return ex.write_set_props(paths, attrs, hidden, false, close, sheet, ctx);
                }
                // The question goes with its sheet ([`Explorer::close_props`]).
                if !ex.sheet_up(sheet) {
                    return ex.finish_set_props(paths.clone(), &paths, close, sheet, ctx);
                }
                let form = attr_form(
                    &attrs,
                    hidden,
                    filesheet::Subject::Set { has_dir: true },
                    &ex.dialog_text,
                    ex.forms.fonts.size_px,
                    ex.forms.text,
                );
                ex.raise(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
                    set: Some(paths),
                    new_name: None,
                    attrs,
                    hidden,
                    close,
                    form,
                }))
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_set_props(
        &mut self,
        paths: Vec<PathBuf>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        tree: bool,
        close: bool,
        sheet: SheetId,
        ctx: &mut Ctx,
    ) -> Reply {
        let was = paths.clone();
        let work = move |gate: &Gate| {
            if !paths.first().is_some_and(|p| gate.change(p)) {
                return None;
            }
            Some(reprisefs::props::write_set(paths, attrs, hidden, tree))
        };
        self.act(
            ctx,
            Action::changes("Properties").of(sheet),
            work,
            move |ex, w, ctx| {
                let Some(w) = w else {
                    return Reply::none();
                };
                for (from, to) in &w.renames {
                    ex.after_rename(from, to, to.is_dir(), ctx);
                }
                let failed = w.failed.first().map(|(p, e)| change_failed_text(p, e));
                // Raised before Apply reads the sheet again, which keeps its boxes; after OK
                // closed it, the box covers the folder window.
                let shown = match &failed {
                    Some(text) if !close => {
                        ex.message_over(Some(sheet), "Properties", text.clone(), ctx)
                    }
                    _ => Reply::none(),
                };
                let reply = ex.finish_set_props(w.now, &was, close, sheet, ctx);
                match failed {
                    Some(text) if close => reply.and(ex.message_box("Properties", text, ctx)),
                    _ => reply.and(shown),
                }
            },
        )
    }

    /// The folder is read again after a set's write for `was`, now at `now`; sheet `sheet`,
    /// while it is up, closes on OK, and the sheet showing `was` is read again
    /// ([`Explorer::sheet_showing`]).
    fn finish_set_props(
        &mut self,
        now: Vec<PathBuf>,
        was: &[PathBuf],
        close: bool,
        sheet: SheetId,
        ctx: &mut Ctx,
    ) -> Reply {
        let reply = self.refresh(ctx);
        let Some(target) = self.sheet_showing(sheet, was) else {
            return Reply {
                redraw_all: true,
                ..reply
            };
        };
        let caption = self.props.as_ref().map(|p| p.caption.clone());
        let shown = match caption {
            Some(caption) if !(close && target == sheet) => {
                let chosen: Vec<(PathBuf, String)> = (now.iter())
                    .filter_map(|p| {
                        Some((p.clone(), p.file_name()?.to_string_lossy().into_owned()))
                    })
                    .collect();
                let (apps, globs) = (self.app_index.clone(), self.globs.clone());
                let work = move |_: &Gate| glance_set_items(&chosen, &apps, &globs);
                let was = was.to_vec();
                self.act(
                    ctx,
                    Action::reads("Properties").of(target),
                    work,
                    move |ex, items, ctx| {
                        ex.refresh_sheet(target, &was, SheetRead::Set(items, caption), ctx)
                    },
                )
            }
            _ => self.close_props(ctx),
        };
        Reply {
            redraw_all: true,
            ..reply.and(shown)
        }
    }

    /// Writes the name and attributes sheet `sheet` asked for to `info`'s file.
    #[allow(clippy::too_many_arguments)]
    fn write_props(
        &mut self,
        info: FileInfo,
        sheet: SheetId,
        new_name: Option<std::ffi::OsString>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        tree: bool,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let old = info.file_name();
        let to = new_name.map(|name| info.path.with_file_name(name));
        let here = self.place.path().as_deref() == Some(info.path.as_path());
        let from = info.path.clone();
        let work = move |gate: &Gate| -> Option<Result<_, String>> {
            if !gate.change(&from) {
                return None;
            }
            // Carries the item's `.hidden` line over to the new name.
            if let Some(to) = &to
                && let Err(e) = reprisefs::hidden::rename(&from, to)
            {
                return Some(Err(reprisefs::names::failed_text(&old, &e)));
            }
            let path = to.clone().unwrap_or(from);
            let write = reprisefs::props::write_item(&path, &attrs, hidden, tree);
            Some(Ok((to, path, write)))
        };
        self.act(
            ctx,
            Action::changes("Properties").of(sheet),
            work,
            move |ex, done, ctx| {
                let (renamed, path, write) = match done {
                    None => return Reply::none(),
                    Some(Err(text)) => return ex.message_over(Some(sheet), "Rename", text, ctx),
                    Some(Ok(done)) => done,
                };
                if let Some(to) = &renamed {
                    ex.after_rename(&info.path, to, info.is_dir, ctx);
                    if here && *to != info.path {
                        let place = crate::place::Place::from_path(to);
                        ex.history.replace(place.clone());
                        ex.show(place, ctx);
                    }
                }
                ex.attrs_written(write, &path, &info.path, close, sheet, ctx)
            },
        )
    }

    pub(crate) fn attrs_confirmed(&mut self, a: AttrConfirm, ctx: &mut Ctx) -> Reply {
        let tree = attr_tree_picked(&a.form);
        let Some((info, sheet)) = self.props.as_ref().map(|p| (p.info.clone(), p.id())) else {
            return Reply::redraw_all();
        };
        if let Some(paths) = a.set {
            return self.write_set_props(paths, a.attrs, a.hidden, tree, a.close, sheet, ctx);
        }
        let (new_name, attrs, hidden) = (a.new_name, a.attrs, a.hidden);
        self.write_props(info, sheet, new_name, attrs, hidden, tree, a.close, ctx)
    }

    /// The attribute write to `path`, the file sheet `sheet` showed at `was`, landed.
    pub(crate) fn attrs_written(
        &mut self,
        write: std::io::Result<()>,
        path: &Path,
        was: &Path,
        close: bool,
        sheet: SheetId,
        ctx: &mut Ctx,
    ) -> Reply {
        let Err(e) = write else {
            return self.finish_props(path, was, close, sheet, ctx);
        };
        // Raised before the sheet is read again, which keeps its boxes.
        let text = change_failed_text(path, &e);
        let shown = self.message_over(Some(sheet), "Properties", text, ctx);
        shown.and(self.finish_props(path, was, false, sheet, ctx))
    }

    pub(crate) fn attrs_cancelled(&mut self) -> Reply {
        if let Some(p) = self.props.as_mut() {
            let was = p.opened_read_only;
            p.general_mut().set_check_state(READ_ONLY, was);
            let hidden = match &p.set {
                Some(set) => filesheet::shared_state(set.hidden),
                None => filesheet::shared_state(Some(p.info.hidden)),
            };
            p.general_mut().set_check_state(filesheet::HIDDEN, hidden);
            p.adv = None;
            let on = p.dirty();
            p.sheet.set_apply_enabled(on);
        }
        Reply::redraw_all()
    }

    pub(crate) fn advanced_chosen(&mut self, form: &form::Form) -> Reply {
        if let Some(p) = self.props.as_mut() {
            p.adv = Some(filesheet::robots_chosen(form));
            let on = p.dirty();
            p.sheet.set_apply_enabled(on);
        }
        Reply::redraw_all()
    }

    /// The folder is read again after a write to `path`, the file sheet `sheet` showed at `was`;
    /// sheet `sheet`, while it is up, closes on OK, and the sheet showing `was` is read again
    /// ([`Explorer::sheet_showing`]).
    fn finish_props(
        &mut self,
        path: &Path,
        was: &Path,
        close: bool,
        sheet: SheetId,
        ctx: &mut Ctx,
    ) -> Reply {
        let reply = self.refresh(ctx);
        let target = self.sheet_showing(sheet, &[was.to_path_buf()]);
        let Some((p, target)) = self.props.as_ref().zip(target) else {
            return Reply {
                redraw_all: true,
                ..reply
            };
        };
        if close && target == sheet {
            return self.close_props(ctx);
        }
        let listed = self.index_of(path).and_then(|j| self.items.get(j));
        // Another sheet's Name field holds the name it opened with, not the one written.
        let name = match (listed, p.name()) {
            (Some(it), _) => Some(it.label().to_string()),
            _ if target != sheet => None,
            (None, typed) if typed.is_empty() => Some(p.info.name.clone()),
            (None, typed) => {
                let ext = p.info.hidden_ext.len();
                let full = p.wanted_name(&typed);
                Some(full[..full.len() - ext].to_string())
            }
        };
        let caption = (p.caption != p.info.name).then(|| p.caption.clone());
        let (path, apps, globs) = (
            path.to_path_buf(),
            self.app_index.clone(),
            self.globs.clone(),
        );
        let scale = ctx.icons.scale();
        let work = move |_: &Gate| {
            let name = name.unwrap_or_else(|| {
                let it = reprisefs::listing::item_at(&path, &globs);
                it.label().to_string()
            });
            (glance(&path, &name, &apps, &globs, scale), name)
        };
        let was = vec![was.to_path_buf()];
        let sheet = self.act(
            ctx,
            Action::reads("Properties"),
            work,
            move |ex, (glanced, name), ctx| match glanced {
                Ok(g) => {
                    let caption = caption.unwrap_or(name);
                    ex.refresh_sheet(target, &was, SheetRead::One(g, caption), ctx)
                }
                _ => Reply::redraw_all(),
            },
        );
        Reply {
            redraw_all: true,
            ..reply.and(sheet)
        }
    }

    /// The sheet's Change...: the Open With chooser with "always" checked and greyed, since this
    /// one only writes the default.
    fn change_default(&mut self, path: &Path, ctx: &mut Ctx) -> Reply {
        self.choose_program_for(path, true, ctx)
    }

    /// The sheet's opens-with line names the program Change... picked, and wears its icon.
    pub(crate) fn props_set_opens_with(
        &mut self,
        name: String,
        icon: Option<String>,
        ctx: &mut Ctx,
    ) {
        let Some(p) = self.props.as_mut() else {
            return;
        };
        p.general_mut().set_text("v_opens", &name);
        p.picks.opens_icon = Some(icon);
        wear_picks(p, ctx);
    }
}

/// What Change Icon's and Change...'s picks wear stand-ins for, till their icons land.
#[derive(Default)]
pub(crate) struct Picks {
    /// The icon of the program Change... picked.
    opens_icon: Option<Option<String>>,
    waits: IconWaits,
}

/// The sheet wears the icons of its picks: Change Icon's on the Shortcut page or Customize's
/// picture and preview, and Change...'s program beside its name. Stand-ins while they decode.
fn wear_picks(p: &mut Props, ctx: &mut Ctx) {
    let shown = p.shown_icon();
    let waits = &mut p.picks.waits;
    *waits = IconWaits::default();
    let shortcut =
        (p.sheet.pages.iter()).position(|q| q.item(filesheet::SHORTCUT_IDS[0]).is_some());
    if p.info.launcher.is_some()
        && let Some(Some(name)) = &p.icon_chosen
        && let Some(k) = shortcut
    {
        match waits.get(&mut ctx.icons, name, ICON_PX) {
            Ok(Some(c)) | Err(c) => {
                p.sheet.pages[k].set_picture("icon", iconview::with_shortcut_arrow(c));
            }
            Ok(None) => {}
        }
    }
    let customize = (p.sheet.pages.iter()).any(|q| q.item(filesheet::CUSTOM_ICON).is_some());
    if p.icon_chosen.is_some() && customize {
        wear_folder_icon(&mut p.sheet, &shown, waits, ctx);
    }
    if let Some(icon) = &p.picks.opens_icon {
        let canvas = (icon.as_deref()).and_then(|n| waits.first(&mut ctx.icons, &[n], 16));
        if let Some(it) = p.general_mut().item_mut("opens_icon")
            && let Control::Picture { canvas: c, .. } = &mut it.control
        {
            *c = canvas;
        }
    }
}

fn shortcut_facts(info: &FileInfo, l: &reprisefs::launcher::Launcher) -> filesheet::ShortcutFacts {
    filesheet::ShortcutFacts {
        name: info.name.clone(),
        target_type: l.target_type.clone(),
        target_location: l.location.clone(),
        symlink: l.kind == reprisefs::launcher::Kind::Symlink,
        fields: l.fields(),
    }
}

fn change_failed_text(path: &Path, e: &dyn std::fmt::Display) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    format!("Cannot change {}: {e}", name.to_string_lossy())
}

/// The path, the sheet field's name and the caption's name for `place`'s own sheet, or `None`
/// where there is no sheet. Shell folders are titled by their shell names while the field keeps
/// the real directory name, so OK cannot rename `$HOME` to "<user>'s Home".
fn place_sheet_of(
    place: &crate::place::Place,
    shares: &[reprisefs::netshare::Share],
) -> Option<(PathBuf, String, String)> {
    use crate::place::Place;
    match place {
        Place::Computer | Place::RecycleBin => None,
        place => {
            let dir = place.path()?;
            // A folder reached through a symlink gets its own sheet, not the link's.
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            let name = match place {
                Place::Folder(_) => place.title_in(shares),
                _ => dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| place.title()),
            };
            Some((dir, name, place.title_in(shares)))
        }
    }
}

/// A file's sheet facts and what its sheet icon is drawn from, read on a listing thread with the
/// icon files the sheet wears, decoded at `scale`.
pub(crate) struct Glanced {
    info: FileInfo,
    icon: SheetIcon,
    files: Vec<(String, i32, Option<Canvas>)>,
    scale: i32,
}

/// What a sheet is built from once read, kept till the theme icons it wears are ready.
pub(crate) enum SheetRead {
    /// A file's or folder's sheet, and its caption.
    One(Glanced, String),
    /// A Recycle Bin item's.
    Bin(Glanced, reprisefs::listing::Item),
    /// The combined sheet over several items, and its caption.
    Set(Vec<FileInfo>, String),
}

enum SheetIcon {
    /// A folder, with its own `.directory` icon.
    Folder(Option<String>),
    /// A shortcut's icon.
    Shortcut(String),
    /// A file of this type.
    File(Option<String>),
}

fn glance(
    path: &Path,
    name: &str,
    apps: &[reprisefs::apps::AppEntry],
    globs: &reprisefs::apps::MimeDb,
    scale: i32,
) -> std::io::Result<Glanced> {
    let info = reprisefs::props::glance_in(path, name, apps, globs)?;
    let icon = if info.path.is_dir() {
        SheetIcon::Folder(reprisefs::meta::folder_icon(&info.path))
    } else if let Some(sc) = std::fs::metadata(&info.path)
        .ok()
        .and_then(|m| reprisefs::apps::shortcut_at(&info.path, &m))
    {
        SheetIcon::Shortcut(sc.icon().to_string())
    } else if let Some(e) = menu_entry(&info.path, apps) {
        SheetIcon::Shortcut(e.icon.clone().unwrap_or_else(|| PROGRAM_ICON.into()))
    } else {
        SheetIcon::File(reprisefs::apps::mime_of_in(globs, &info.path))
    };
    let own = match &icon {
        SheetIcon::Folder(own) => own.as_deref(),
        SheetIcon::Shortcut(name) => Some(name.as_str()),
        SheetIcon::File(_) => None,
    };
    let mut wanted = vec![(own, ICON_PX), (info.opens_with_icon.as_deref(), 16)];
    if info.is_dir {
        let own = info.own_icon.as_deref();
        wanted.extend([(own, ICON_PX), (own, filesheet::PREVIEW_PX)]);
    }
    let mut files: Vec<(String, i32, Option<Canvas>)> = Vec::new();
    for (file, px) in wanted {
        if let Some(f) = file.filter(|f| f.starts_with('/'))
            && !files.iter().any(|(g, p, _)| g == f && *p == px)
        {
            let art = reprisekit::icontheme::decode_file(Path::new(f), px, scale);
            files.push((f.to_string(), px, art));
        }
    }
    Ok(Glanced {
        info,
        icon,
        files,
        scale,
    })
}

const PROGRAM_ICON: &str = "application-x-executable";

/// The Start menu's entry read from the file at `path`, as the menu names and draws it.
fn menu_entry<'a>(
    path: &Path,
    apps: &'a [reprisefs::apps::AppEntry],
) -> Option<&'a reprisefs::apps::AppEntry> {
    menu_entry_in(path, apps, &reprisefs::apps::default_dirs())
}

/// [`menu_entry`] with the applications folders `dirs`.
pub(crate) fn menu_entry_in<'a>(
    path: &Path,
    apps: &'a [reprisefs::apps::AppEntry],
    dirs: &[PathBuf],
) -> Option<&'a reprisefs::apps::AppEntry> {
    let file = reprisefs::apps::DesktopId::from_file_name(path.file_name()?.to_str()?)?;
    (apps.iter())
        .filter(|e| e.id.as_str().ends_with(file.as_str()))
        .find(|e| reprisefs::apps::find_file_in(dirs, &e.id).as_deref() == Some(path))
}

/// Each of `chosen` (path and label) that answers, for the combined sheet.
fn glance_set_items(
    chosen: &[(PathBuf, String)],
    apps: &[reprisefs::apps::AppEntry],
    globs: &reprisefs::apps::MimeDb,
) -> Vec<FileInfo> {
    let glance = |(path, label): &(PathBuf, String)| {
        reprisefs::props::glance_in(path, label, apps, globs)
            .inspect_err(|e| log::warn!("properties: {}: {e}", path.display()))
            .ok()
    };
    chosen.iter().filter_map(glance).collect()
}

/// The sheet's picture of `icon` at `path`: the first name the theme has that decodes, a blank
/// stand-in noted in `waits` while one decodes off the loop.
fn props_icon(
    icon: &SheetIcon,
    path: &Path,
    waits: &mut IconWaits,
    ctx: &mut Ctx,
) -> Option<Canvas> {
    crate::icons::picked_icon(waits, &mut ctx.icons, ICON_PX, |has| match icon {
        SheetIcon::Folder(own) => crate::icons::sheet_folder_icon(
            own.as_deref().unwrap_or(filesheet::STANDARD_FOLDER_ICON),
            has,
        ),
        SheetIcon::Shortcut(name) => name.clone(),
        SheetIcon::File(mime) => reprisefs::icons::pick_icon(mime.as_deref(), path, has),
    })
}

/// Customize's picture and preview wear `icon`, fetched by `get` at each size.
fn show_folder_icon(
    sheet: &mut PropSheet,
    icon: &str,
    mut get: impl FnMut(&str, i32) -> Option<Canvas>,
) {
    for (id, px) in [
        (filesheet::CUSTOM_ICON, ICON_PX),
        (filesheet::CUSTOM_PREVIEW, filesheet::PREVIEW_PX),
    ] {
        let canvas = get(icon, px);
        for page in &mut sheet.pages {
            if let Some(it) = page.item_mut(id)
                && let Control::Picture { canvas: c, .. } = &mut it.control
            {
                *c = canvas.clone();
            }
        }
    }
}

/// [`show_folder_icon`] with `icon`, or the standard folder where the theme lacks it or it lands
/// broken; stand-ins noted in `waits` while it decodes.
fn wear_folder_icon(sheet: &mut PropSheet, icon: &str, waits: &mut IconWaits, ctx: &mut Ctx) {
    let mut icon = crate::icons::sheet_folder_icon(icon, &|n| ctx.icons.has_unread(n));
    if waits.get(&mut ctx.icons, &icon, ICON_PX) == Ok(None) {
        ctx.icons.refuse(&icon);
        icon = filesheet::STANDARD_FOLDER_ICON.to_string();
    }
    show_folder_icon(sheet, &icon, |n, px| waits.first(&mut ctx.icons, &[n], px));
}

/// Change Icon's list wears the icons its stand-ins waited on once they landed, and a name that
/// landed broken leaves the list: true when it changed.
pub(crate) fn land_change_icons(
    form: &mut form::Form,
    waits: &mut IconWaits,
    icons: &mut IconCache,
) -> bool {
    if !waits.landed(icons) {
        return false;
    }
    *waits = IconWaits::default();
    let Some(list) = filesheet::change_icon_list_mut(form) else {
        return false;
    };
    let mut gone = Vec::new();
    for (name, icon) in list.names.iter().zip(&mut list.icons) {
        match waits.get(icons, name, ICON_PX) {
            Ok(None) if icon.is_some() => gone.push(name.clone()),
            Ok(art) => *icon = art,
            Err(blank) => *icon = Some(blank),
        }
    }
    filesheet::change_icon_drop(form, &gone);
    true
}
