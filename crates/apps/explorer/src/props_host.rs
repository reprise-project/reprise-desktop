//! The Properties sheet's host half: what opens it, its window and events, and what OK and Apply
//! write.

use crate::Explorer;
use crate::props_dialog::{
    ADVANCED, AttrConfirm, BIN_SHEET_H, CHANGE, ICON_PX, PROPS, Props, READ_ONLY, RESTORE,
    SET_ICON, SHEET_W, attr_form, attr_tree_picked, bin_facts, bin_page, folder_has_contents,
    general_page, rows, set_facts,
};
use repriseapp::{Child, Ctx, Event, Reply};
use reprisefs::props::{AttrMoves, FileInfo};
use reprisekit::controls::form::{self, Control};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::{filesheet, iconview};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::{Path, PathBuf};

impl Explorer {
    /// File > Properties, Alt+Enter and the menus' Properties row: the sheet for the single
    /// selected entry, the combined sheet for several, or for the place itself with nothing
    /// selected.
    pub fn properties(&mut self, ctx: &mut Ctx) -> Reply {
        if self.sv.selection.len() > 1 {
            return self.open_set_properties(ctx);
        }
        let chosen = self.sv.selection.single().and_then(|i| self.items.get(i));
        if self.place == crate::place::Place::RecycleBin
            && let Some(it) = chosen.cloned()
        {
            return self.open_bin_properties(&it, ctx);
        }
        let (path, name, title) = match chosen {
            Some(it) => (it.path.clone(), it.label().into(), it.label().into()),
            None => match self.place_sheet() {
                Some(v) => v,
                None => return Reply::none(),
            },
        };
        self.open_properties(&path, &name, &title, ctx)
    }

    /// The path, the sheet field's name and the caption's name for the current place's own sheet,
    /// or `None` where there is no sheet. Shell folders are titled by their shell names while the
    /// field keeps the real directory name, so OK cannot rename `$HOME` to "<user>'s Home".
    pub(crate) fn place_sheet(&self) -> Option<(PathBuf, String, String)> {
        use crate::place::Place;
        match &self.place {
            Place::Computer | Place::RecycleBin => None,
            place => {
                let dir = place.path()?;
                // A folder reached through a symlink gets its own sheet, not the link's.
                let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
                let name = match place {
                    Place::Folder(_) => place.title(),
                    _ => dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| place.title()),
                };
                Some((dir, name, place.title()))
            }
        }
    }

    pub(crate) fn open_properties(
        &mut self,
        path: &Path,
        name: &str,
        title: &str,
        ctx: &mut Ctx,
    ) -> Reply {
        let info = match reprisefs::props::glance_in(path, name, &self.app_index, &self.globs) {
            Ok(info) => info,
            Err(e) => {
                let text = format!("Cannot show the properties of {name}: {e}");
                return self.message_box("Properties", text, ctx);
            }
        };
        self.props = Some(self.build_props(info, title.to_string(), ctx));
        Reply::redraw_all()
    }

    fn open_bin_properties(&mut self, it: &reprisefs::listing::Item, ctx: &mut Ctx) -> Reply {
        let info =
            match reprisefs::props::glance_in(&it.path, &it.name, &self.app_index, &self.globs) {
                Ok(info) => info,
                Err(e) => {
                    let text = format!("Cannot show the properties of {}: {e}", it.name);
                    return self.message_box("Properties", text, ctx);
                }
            };
        self.walk_for_props(&info, ctx);
        let facts = bin_facts(&info, it.trashed.as_deref(), reprisefs::locale::is_12h());
        let icon = self.props_icon(&info, ctx);
        let page = bin_page(&facts, &it.name, icon);
        let sheet = PropSheet::new(&["General"], vec![page], (SHEET_W, BIN_SHEET_H));
        self.props = Some(Props {
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
            sheet,
        });
        Reply::redraw_all()
    }

    fn walk_for_props(&mut self, info: &FileInfo, ctx: &mut Ctx) {
        if info.is_dir && info.contains.is_none() {
            let path = info.path.clone();
            self.spawn_job(ctx, move || {
                let totals = reprisefs::props::folder_totals(&path);
                Box::new(move |ex: &mut Explorer, _: &mut Ctx| {
                    if let Some(p) = ex.props.as_mut()
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

    fn build_props(&mut self, info: FileInfo, title: String, ctx: &mut Ctx) -> Props {
        self.walk_for_props(&info, ctx);
        let rows = rows(&info, reprisefs::locale::is_12h());
        let icon = self.props_icon(&info, ctx);
        let icon = match info.is_shortcut {
            true => icon.map(iconview::with_shortcut_arrow),
            false => icon,
        };
        let app = info
            .opens_with_icon
            .as_deref()
            .and_then(|n| ctx.icons.get(n, 16));
        // The Hidden box stands for the leading dot, so the field leaves it out.
        let shown = filesheet::shown_name(&info.file_name(), &info.hidden_ext, info.hidden);
        let (page, height) = general_page(&rows, &shown, icon.clone(), app);
        let over = crate::props_dialog::over(&info);
        let mut sheet =
            self.tabbed_sheet(page, height, over, ctx, |tab| match (tab, &info.launcher) {
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
            });
        if info.is_dir {
            let own = info
                .own_icon
                .clone()
                .unwrap_or_else(|| filesheet::STANDARD_FOLDER_ICON.to_string());
            Self::show_folder_icon(&mut sheet, &own, ctx);
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
            info,
            sheet,
        }
    }

    fn open_set_properties(&mut self, ctx: &mut Ctx) -> Reply {
        use crate::place::Place;
        if matches!(self.place, Place::Computer | Place::RecycleBin) {
            return Reply::none();
        }
        let sel = &self.sv.selection;
        let first = sel.focus.filter(|&i| sel.is(i));
        let order = first
            .into_iter()
            .chain(sel.set.iter().copied().filter(|&i| Some(i) != first));
        let mut items = Vec::new();
        for it in order.filter_map(|i| self.items.get(i)) {
            match reprisefs::props::glance_in(&it.path, it.label(), &self.app_index, &self.globs) {
                Ok(info) => items.push(info),
                Err(e) => log::warn!("properties: {}: {e}", it.path.display()),
            }
        }
        let Some(first) = items.first() else {
            return Reply::none();
        };
        let caption = format!("{}, ...", first.name);
        self.props = self.build_set_props(items, caption, ctx);
        Reply::redraw_all()
    }

    fn build_set_props(
        &mut self,
        items: Vec<FileInfo>,
        caption: String,
        ctx: &mut Ctx,
    ) -> Option<Props> {
        let set = reprisefs::props::glance_set(&items);
        let info = items.into_iter().next()?;
        if set.walking {
            let paths = set.paths.clone();
            self.spawn_job(ctx, move || {
                let mut sum = reprisefs::props::FolderTotals::default();
                for d in paths.iter().filter(|p| p.is_dir()) {
                    let t = reprisefs::props::folder_totals(d);
                    sum.files += t.files;
                    sum.folders += t.folders;
                    sum.size += t.size;
                    sum.on_disk += t.on_disk;
                }
                Box::new(move |ex: &mut Explorer, _: &mut Ctx| {
                    if let Some(p) = ex.props.as_mut()
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
        let icon = ctx.icons.get(SET_ICON, ICON_PX);
        let facts = set_facts(&set);
        let (page, height) = general_page(&filesheet::set_rows(&facts), "", icon, None);
        let over = filesheet::Over::Set {
            document: set.has_document,
            shortcut: set.has_shortcut,
            folder: set.has_dir,
        };
        let mut sheet =
            self.tabbed_sheet(page, height, over, ctx, |_| filesheet::summary_page(None));
        Self::show_folder_icon(&mut sheet, filesheet::STANDARD_FOLDER_ICON, ctx);
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
            set: Some(set),
            sheet,
        })
    }

    fn tabbed_sheet(
        &self,
        general: form::Form,
        height: i32,
        over: filesheet::Over,
        ctx: &mut Ctx,
        own: impl FnMut(filesheet::Tab) -> form::Form,
    ) -> PropSheet {
        let tabs = filesheet::sheet_tabs(over);
        let mut sheet = filesheet::sheet(general, height, &tabs, own);
        filesheet::fill_pictures(&mut sheet, |n| ctx.icons.get(n, ICON_PX));
        sheet
    }

    fn show_folder_icon(sheet: &mut PropSheet, icon: &str, ctx: &mut Ctx) {
        let icon = &crate::icons::sheet_folder_icon(icon, &|n| ctx.icons.has(n));
        for (id, px) in [
            (filesheet::CUSTOM_ICON, ICON_PX),
            (filesheet::CUSTOM_PREVIEW, filesheet::PREVIEW_PX),
        ] {
            let canvas = ctx.icons.get(icon, px);
            for page in &mut sheet.pages {
                if let Some(it) = page.item_mut(id)
                    && let Control::Picture { canvas: c, .. } = &mut it.control
                {
                    *c = canvas.clone();
                }
            }
        }
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
            reprisefs::launcher::icon_names(&self.app_index, |n| ctx.icons.has(n))
        } else {
            FOLDER_ICON_NAMES
                .iter()
                .filter(|n| ctx.icons.has(n))
                .map(|n| n.to_string())
                .collect()
        };
        let standard = (!launcher).then(|| filesheet::STANDARD_FOLDER_ICON.to_string());
        for must in [standard, current.clone()].into_iter().flatten() {
            if !names.contains(&must) {
                names.insert(0, must);
            }
        }
        let pictures = names.iter().map(|n| ctx.icons.get(n, ICON_PX)).collect();
        let mut list = reprisekit::controls::iconlist::IconList::new(names, pictures, None);
        if let Some(c) = &current {
            list.pick(c);
        }
        let form = filesheet::change_icon_form(&ctx.icons.theme_name(), list, !launcher);
        self.dialog = Some(crate::dialogs::Dialog::ChangeIcon { form, name });
        Reply::redraw_all()
    }

    pub(crate) fn icon_chosen(&mut self, icon: Option<String>, ctx: &mut Ctx) -> Reply {
        if let Some(p) = self.props.as_mut() {
            if p.info.launcher.is_some()
                && let Some(name) = &icon
                && let Some(k) = p
                    .sheet
                    .pages
                    .iter()
                    .position(|q| q.item(filesheet::SHORTCUT_IDS[0]).is_some())
                && let Some(c) = ctx.icons.get(name, ICON_PX)
            {
                p.sheet.pages[k].set_picture("icon", iconview::with_shortcut_arrow(c));
            }
            p.icon_chosen = Some(icon);
            let shown = p.shown_icon();
            Self::show_folder_icon(&mut p.sheet, &shown, ctx);
            let on = p.dirty();
            p.sheet.set_apply_enabled(on);
        }
        Reply::redraw_all()
    }

    fn props_icon(&mut self, info: &FileInfo, ctx: &mut Ctx) -> Option<Canvas> {
        let name = if info.path.is_dir() {
            let own = reprisefs::meta::folder_icon(&info.path);
            crate::icons::sheet_folder_icon(
                own.as_deref().unwrap_or(filesheet::STANDARD_FOLDER_ICON),
                &|n| ctx.icons.has(n),
            )
        } else if let Some(sc) = std::fs::metadata(&info.path)
            .ok()
            .and_then(|m| reprisefs::apps::shortcut_at(&info.path, &m))
        {
            sc.icon().to_string()
        } else {
            let mime = reprisefs::apps::mime_of_in(&self.globs, &info.path);
            reprisefs::icons::pick_icon(mime.as_deref(), &info.path, &|n| ctx.icons.has(n))
        };
        ctx.icons.get(&name, ICON_PX)
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

    /// The sheet's event loop.
    pub fn props_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            self.props = None;
            return Reply::redraw_all();
        }
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        let clip = repriseapp::keys::form_clipboard(p.sheet.open().focused_edit(), &ev);
        let out = match ev {
            Event::Blink => {
                p.sheet.open_mut().caret = !p.sheet.open().caret;
                return Reply::redraw();
            }
            Event::Pasted(text) => p.sheet.paste(text),
            ev => match repriseapp::keys::sheet_input(&mut p.sheet, &ev, ctx) {
                Some(ev) => ev,
                None => propsheet::Event::None,
            },
        };
        self.props_outcome(out, ctx).with_clipboard(clip)
    }

    pub(crate) fn props_outcome(&mut self, out: propsheet::Event, ctx: &mut Ctx) -> Reply {
        match out {
            propsheet::Event::None => Reply::none(),
            propsheet::Event::Redraw => Reply::redraw(),
            propsheet::Event::Cancel => {
                self.props = None;
                Reply::redraw_all()
            }
            propsheet::Event::Ok => self.apply_props(true, ctx),
            propsheet::Event::Apply => self.apply_props(false, ctx),
            propsheet::Event::Page(_, ev) => {
                if let form::Event::Pressed(CHANGE) = ev {
                    let path = self.props.as_ref().map(|p| p.info.path.clone());
                    if let Some(path) = path {
                        return self.change_default(&path, ctx);
                    }
                }
                if let form::Event::Pressed(RESTORE) = ev
                    && let Some(item) = self.props.as_ref().and_then(|p| p.restore.clone())
                {
                    self.props = None;
                    let reply = self.restore_items(vec![item], ctx);
                    return Reply {
                        redraw_all: true,
                        ..reply
                    };
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
        let Some((l, page, path)) = self.props.as_ref().and_then(|p| {
            let page = filesheet::shortcut_of(&p.sheet.pages)?;
            Some((p.info.launcher.as_ref()?, page, &p.info.path))
        }) else {
            return Reply::none();
        };
        let [target, ..] = filesheet::shortcut_values(page);
        match l.find(&target, path) {
            Some(found) => Reply {
                spawn: vec![vec![
                    "reprise-explorer".into(),
                    "--select".into(),
                    found.into_os_string(),
                ]],
                ..Reply::none()
            },
            None => {
                let text = l.not_found_text(&target);
                self.message_box("Problem with Shortcut", text, ctx)
            }
        }
    }

    fn apply_props(&mut self, close: bool, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.as_ref() else {
            return Reply::none();
        };
        if let Some(set) = &p.set {
            let (paths, attrs, hidden) = (set.paths.clone(), p.attrs(), p.hidden());
            if let Some(icon) = p.icon_chosen.clone() {
                for dir in paths.iter().filter(|d| d.is_dir()) {
                    if let Err(e) = reprisefs::meta::set_folder_icon(dir, icon.as_deref()) {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            dir.display()
                        );
                    }
                }
                if attrs.is_empty() && hidden.is_none() {
                    return self.finish_set_props(paths, close, ctx);
                }
            }
            return self.apply_set_props(paths, attrs, hidden, close, ctx);
        }
        let (new_name, attrs, hidden) = (p.new_name(), p.attrs(), p.hidden());
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
                self.props = None;
                return Reply::redraw_all();
            }
            return Reply::none();
        }
        // The description goes first, while the file is where the sheet found it; the rest may
        // rename it.
        let _ = reprisefs::meta::write_summary(&p.info.path, &summary);
        if !launcher.is_empty()
            && let Err(e) = reprisefs::launcher::write_keys(&p.info.path, &launcher)
        {
            let text = format!("Cannot change {}: {e}", p.info.name);
            return self.message_box("Properties", text, ctx);
        }
        if let Some(icon) = &folder_icon
            && let Err(e) = reprisefs::meta::set_folder_icon(&p.info.path, icon.as_deref())
        {
            let text = format!("Cannot change the icon of {}: {e}", p.info.name);
            return self.message_box("Properties", text, ctx);
        }
        if new_name.is_none() && attrs.is_empty() && hidden.is_none() {
            let path = p.info.path.clone();
            return self.finish_props(&path, close, ctx);
        }
        if (!attrs.is_empty() || hidden.is_some())
            && p.info.is_dir
            && folder_has_contents(&p.info.path)
        {
            let form = attr_form(
                &attrs,
                hidden,
                filesheet::Subject::Folder,
                &self.dialog_text,
                self.forms.fonts.size_px,
                self.forms.text,
            );
            self.dialog = Some(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
                set: None,
                new_name,
                attrs,
                hidden,
                close,
                form,
            }));
            return Reply::redraw_all();
        }
        self.write_props(new_name, attrs, hidden, false, close, ctx)
    }

    fn apply_set_props(
        &mut self,
        paths: Vec<PathBuf>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        if attrs.is_empty() && hidden.is_none() {
            if close {
                self.props = None;
                return Reply::redraw_all();
            }
            return Reply::none();
        }
        if paths.iter().any(|p| p.is_dir() && folder_has_contents(p)) {
            let form = attr_form(
                &attrs,
                hidden,
                filesheet::Subject::Set { has_dir: true },
                &self.dialog_text,
                self.forms.fonts.size_px,
                self.forms.text,
            );
            self.dialog = Some(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
                set: Some(paths),
                new_name: None,
                attrs,
                hidden,
                close,
                form,
            }));
            return Reply::redraw_all();
        }
        self.write_set_props(paths, attrs, hidden, false, close, ctx)
    }

    fn write_set_props(
        &mut self,
        paths: Vec<PathBuf>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        tree: bool,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        self.spawn_job(ctx, move || {
            let w = reprisefs::props::write_set(paths, attrs, hidden, tree);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                for (from, to) in &w.renames {
                    ex.after_rename(from, to, to.is_dir(), ctx);
                }
                let failed = w.failed.first().map(|(p, e)| change_failed_text(p, e));
                let reply = ex.finish_set_props(w.now, close, ctx);
                if let Some(text) = failed {
                    ex.message_box("Properties", text, ctx);
                }
                reply
            })
        });
        Reply::none()
    }

    fn finish_set_props(&mut self, now: Vec<PathBuf>, close: bool, ctx: &mut Ctx) -> Reply {
        let reply = self.refresh(ctx);
        let caption = self.props.as_ref().map(|p| p.caption.clone());
        match caption {
            Some(caption) if !close => {
                let items = now
                    .iter()
                    .filter_map(|p| {
                        let name = p.file_name()?.to_string_lossy().into_owned();
                        reprisefs::props::glance_in(p, &name, &self.app_index, &self.globs).ok()
                    })
                    .collect();
                self.props = self.build_set_props(items, caption, ctx);
            }
            _ => self.props = None,
        }
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    fn write_props(
        &mut self,
        new_name: Option<String>,
        attrs: AttrMoves,
        hidden: Option<bool>,
        tree: bool,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let Some(info) = self.props.as_ref().map(|p| p.info.clone()) else {
            return Reply::redraw_all();
        };
        let path = if let Some(wanted) = new_name {
            let here = self.place.path().as_deref() == Some(info.path.as_path());
            match self.rename_for_props(&info, &wanted, ctx) {
                Ok(path) => {
                    if here && path != info.path {
                        self.show(crate::place::Place::from_path(&path), ctx);
                    }
                    path
                }
                Err(reply) => return *reply,
            }
        } else {
            info.path.clone()
        };
        if tree {
            self.spawn_job(ctx, move || {
                let write = reprisefs::props::write_item(&path, &attrs, hidden, true);
                Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                    ex.attrs_written(write, &path, close, ctx)
                })
            });
            return Reply::none();
        }
        let write = reprisefs::props::write_item(&path, &attrs, hidden, false);
        self.attrs_written(write, &path, close, ctx)
    }

    pub(crate) fn attrs_confirmed(&mut self, a: AttrConfirm, ctx: &mut Ctx) -> Reply {
        let tree = attr_tree_picked(&a.form);
        if let Some(paths) = a.set {
            return self.write_set_props(paths, a.attrs, a.hidden, tree, a.close, ctx);
        }
        self.write_props(a.new_name, a.attrs, a.hidden, tree, a.close, ctx)
    }

    fn attrs_written(
        &mut self,
        write: std::io::Result<()>,
        path: &Path,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let Err(e) = write else {
            return self.finish_props(path, close, ctx);
        };
        let reply = self.finish_props(path, false, ctx);
        self.message_box("Properties", change_failed_text(path, &e), ctx);
        reply
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

    fn rename_for_props(
        &mut self,
        info: &FileInfo,
        wanted: &str,
        ctx: &mut Ctx,
    ) -> Result<PathBuf, Box<Reply>> {
        let old = info.file_name();
        match reprisefs::names::verdict(&old, wanted, &[]) {
            reprisefs::names::Verdict::Cancel => Ok(info.path.clone()),
            reprisefs::names::Verdict::Refuse(text) => {
                Err(Box::new(self.message_box("Rename", text, ctx)))
            }
            reprisefs::names::Verdict::Rename(name) => {
                let Some(to) = info.path.parent().map(|d| d.join(&name)) else {
                    return Ok(info.path.clone());
                };
                // Carries the item's `.hidden` line over to the new name.
                if let Err(e) = reprisefs::hidden::rename(&info.path, &to) {
                    let text = reprisefs::names::failed_text(&old, &e);
                    return Err(Box::new(self.message_box("Rename", text, ctx)));
                }
                self.after_rename(&info.path, &to, info.is_dir, ctx);
                Ok(to)
            }
        }
    }

    fn finish_props(&mut self, path: &Path, close: bool, ctx: &mut Ctx) -> Reply {
        let reply = self.refresh(ctx);
        if close {
            self.props = None;
            return Reply::redraw_all();
        }
        let Some(p) = self.props.as_ref() else {
            return Reply {
                redraw_all: true,
                ..reply
            };
        };
        let listed = self.index_of(path).and_then(|j| self.items.get(j));
        let name = match (listed, p.name()) {
            (Some(it), _) => it.label().to_string(),
            (None, typed) if typed.is_empty() => p.info.name.clone(),
            (None, typed) => {
                let ext = p.info.hidden_ext.len();
                let full = p.wanted_name(&typed);
                full[..full.len() - ext].to_string()
            }
        };
        let caption = if p.caption == p.info.name {
            name.clone()
        } else {
            p.caption.clone()
        };
        if let Ok(info) = reprisefs::props::glance_in(path, &name, &self.app_index, &self.globs) {
            self.props = Some(self.build_props(info, caption, ctx));
        }
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    /// The sheet's Change...: the Open With chooser with "always" checked and greyed, since this
    /// one only writes the default.
    fn change_default(&mut self, path: &Path, ctx: &mut Ctx) -> Reply {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.choose_program_for(&real, true, ctx)
    }

    pub(crate) fn props_set_opens_with(&mut self, name: String, icon: Option<Canvas>) {
        let Some(p) = self.props.as_mut() else {
            return;
        };
        let page = p.general_mut();
        page.set_text("v_opens", &name);
        if let Some(it) = page.item_mut("opens_icon")
            && let Control::Picture { canvas, .. } = &mut it.control
        {
            *canvas = icon;
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
