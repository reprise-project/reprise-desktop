//! Routing events: the sheet's controls, its keys and each dialog window.

use crate::registry::Page;
use crate::{
    ADD_ITEM_WIN, ADD_WIN, CHOOSER_WIN, ERROR_WIN, LAYOUT_WIN, RENAME_WIN, SAVER_WIN, SHORTCUT_WIN,
    SettingsApp, additem, appearancepage, backgroundpage, defaultspage, displayhost, displaypage,
    filepick::Pick, layoutdlg, menupage, pages, quickpage, renamedlg, saverdlg, saverpage,
};
use repriseapp::keys::{dialog_key, form_clipboard};
use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, Event, KeyInput, PointerInput, Reply, WinId};
use reprisekit::controls::form;
use reprisekit::controls::listview;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::propsheet;

impl SettingsApp {
    pub(crate) fn sheet_event(&mut self, ev: propsheet::Event) -> Reply {
        match ev {
            propsheet::Event::None => Reply::none(),
            propsheet::Event::Redraw => Reply::redraw(),
            propsheet::Event::Page(p, form::Event::Changed(id)) => {
                if p == Page::Display.index()
                    && displaypage::read(&mut self.display.model, &self.sheet.pages[p], id)
                {
                    return self.display_changed();
                }
                if p == Page::DefaultPrograms.index() {
                    return self.defaults_changed(id);
                }
                if p == Page::Tablet.index() {
                    return self.tablet_changed(id);
                }
                // Only the control just worked is read back: reading all would write clamped values for keys the control cannot show.
                let from = self.want.interface.clone();
                pages::read_one(&self.sheet, id, &mut self.want);
                if self.want.interface != from {
                    self.interface_moved(&from);
                }
                if id == saverpage::SAVER {
                    let row = self.sheet.pages[Page::ScreenSaver.index()].combo_selected(id);
                    saverpage::pick(&mut self.want, row.unwrap_or(0));
                }
                if id == saverpage::WAIT
                    && let Some(form::Control::UpDown(u)) = self.sheet.pages
                        [Page::ScreenSaver.index()]
                    .item(id)
                    .map(|i| &i.control)
                {
                    saverpage::set_wait(&mut self.want, u.value() as i64);
                }
                if id == backgroundpage::MODE {
                    self.pick_mode();
                    self.want_preview();
                }
                if id == backgroundpage::WALLPAPER {
                    self.pick_wallpaper();
                    self.want_preview();
                }
                pages::follow(&mut self.sheet, &self.want);
                self.show_monitor();
                self.sync_apply();
                Reply::redraw()
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) => match id {
                pages::EDIT_FILE => self.edit_file(),
                displaypage::IDENTIFY => self.identify(),
                pages::TRAY_CUSTOMIZE => {
                    // Off the loop: the call waits on the compositor.
                    std::thread::spawn(|| {
                        if let Err(e) = reprisebus::tray::customize() {
                            log::warn!("tray Customize: {e}");
                        }
                    });
                    Reply::none()
                }
                pages::ADD => {
                    self.open_add(Pick::Autostart);
                    Reply::redraw_all()
                }
                saverpage::SETTINGS => {
                    let kind = saverpage::kind(&self.want);
                    self.saver_dlg = Some(saverdlg::SaverDialog::new(kind, &self.want));
                    Reply::redraw_all()
                }
                saverpage::PREVIEW => {
                    let name = saverpage::kind(&self.want).name().to_string();
                    // Off the loop: the call waits on the compositor.
                    std::thread::spawn(move || {
                        if let Err(e) = reprisebus::tray::preview_screen_saver(&name) {
                            log::warn!("screen saver Preview: {e}");
                        }
                    });
                    Reply::none()
                }
                backgroundpage::BROWSE => {
                    let pick = if backgroundpage::slideshow(&self.want) {
                        Pick::SlideFolder
                    } else {
                        Pick::Wallpaper
                    };
                    self.open_add(pick);
                    Reply::redraw_all()
                }
                pages::REMOVE => self.remove_selected(),
                defaultspage::CHANGE => self.open_chooser(),
                defaultspage::RESTORE => self.restore_default(),
                pages::FORGET => self.forget_location(),
                pages::THUMBS_CLEAR if self.clearing.is_none() => {
                    // Off the loop: a full cache is thousands of files.
                    self.clearing = Some(self.spawn(repriseapp::thumbs::clear_cache));
                    pages::show_clearing(
                        &mut self.sheet.pages[Page::Advanced.index()],
                        pages::Clearing::Running,
                    );
                    Reply::redraw()
                }
                pages::SC_ADD => {
                    self.open_shortcut(None);
                    Reply::redraw_all()
                }
                pages::SC_EDIT => self.edit_shortcut(),
                pages::SC_REMOVE => self.remove_shortcut(),
                pages::KB_ADD => {
                    self.add_layout = Some(layoutdlg::LayoutDialog::new(&self.xkb));
                    Reply::redraw_all()
                }
                pages::KB_REMOVE => self.remove_layout(),
                pages::KB_UP => self.move_layout_up(),
                menupage::ADD | quickpage::ADD => {
                    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
                    self.add_item = Some(additem::AddItemDialog::new(apps, &self.want.terminal));
                    self.add_to_quick = id == quickpage::ADD;
                    Reply::redraw_all()
                }
                quickpage::REMOVE | quickpage::UP | quickpage::DOWN => self.quick_button(id),
                menupage::RENAME => {
                    let page = &self.sheet.pages[Page::StartMenu.index()];
                    match menupage::selected_label(page, &self.want.startmenu) {
                        Some(l) => {
                            self.rename = Some(renamedlg::RenameDialog::new(&l));
                            Reply::redraw_all()
                        }
                        None => Reply::none(),
                    }
                }
                menupage::FOLDER | menupage::REMOVE | menupage::UP | menupage::DOWN => {
                    self.menu_button(id)
                }
                _ => Reply::redraw(),
            },
            propsheet::Event::Page(..) => Reply::redraw(),
            propsheet::Event::Apply => {
                if self.apply() {
                    self.display.pending = Some(displayhost::Press::Apply);
                }
                Reply::redraw()
            }
            propsheet::Event::Ok => {
                // A failed write leaves the sheet open and dirty; otherwise the outputs' answer decides when it closes (`displayflow`).
                if self.apply() {
                    self.display.pending = Some(displayhost::Press::Ok);
                }
                Reply::redraw()
            }
            propsheet::Event::Cancel => {
                // Quits at once unless a request to the outputs is out,
                // whose answer is put back first.
                self.revert();
                self.display.pending = Some(displayhost::Press::Cancel);
                Reply::redraw()
            }
        }
    }

    pub(crate) fn key(&mut self, k: KeyInput, cm: &ControlMetrics) -> Reply {
        let ctrl_tab = propsheet::page_step(k.sym, k.mods.ctrl, k.mods.shift);
        if let Some(dir) = form::shove_dir(k.sym, k.mods.ctrl, k.mods.shift, k.mods.alt)
            && let Some(ev) = self.sheet.shove(dir)
        {
            return self.sheet_event(ev);
        }
        let Some(key) = dialog_key(&k) else {
            return Reply::none();
        };
        if ctrl_tab.is_some() {
            self.blur_side_lists();
            let ev = self.sheet.key(key, ctrl_tab);
            self.tab_opened();
            return self.sheet_event(ev);
        }
        if let Some((_, _, focus)) = self.side_list() {
            let held = *focus;
            if let form::Key::Tab { back } = key {
                return self.side_tab(back);
            }
            if held {
                // On the Startup and locations lists Enter stays the
                // sheet's OK.
                if key == form::Key::Enter
                    && let Some(r) = self.list_activate()
                {
                    return r;
                }
                if let Some(key) = pages::list_key(&key) {
                    return self.list_key(&key, cm);
                }
            }
        }
        let ev = self.sheet.key(key, None);
        self.sheet_event(ev)
    }

    pub(crate) fn route(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        self.fit_columns(&ctx.forms().controls());
        if self.err_icon.is_none() {
            self.err_icon = ctx.icons.get("dialog-error", 32);
        }
        if self.display.icon.is_none() {
            self.display.icon = ctx.icons.get("dialog-question", 32);
        }
        if self.wallpaper_icons.is_none() {
            let none = ctx
                .icons
                .get("process-stop", 16)
                .or_else(|| ctx.icons.get("dialog-cancel", 16));
            self.wallpaper_icons = Some((none, ctx.icons.get("image-x-generic", 16)));
            self.folder_icon = ctx.icons.get("folder", 16);
            self.fill_wallpapers();
            let (scale, glass) = (ctx.icons.scale(), appearancepage::glass(&self.want));
            self.thumbing = Some(self.spawn(move || {
                backgroundpage::builtins()
                    .into_iter()
                    .map(|b| Some(backgroundpage::thumb(b, &glass, scale)))
                    .collect()
            }));
        }
        self.add_host.icons.prime(&mut ctx.icons);
        if let Some(reason) = self.open_error.take() {
            self.fail(&reason);
        }
        if win == ERROR_WIN {
            return self.error_event(ev, ctx);
        }
        if win == ADD_WIN {
            return self.add_event(ev, ctx);
        }
        if win == SHORTCUT_WIN {
            return self.shortcut_event(ev, ctx);
        }
        if win == LAYOUT_WIN {
            return self.add_layout_event(ev, ctx);
        }
        if win == ADD_ITEM_WIN {
            return self.add_item_event(ev, ctx);
        }
        if win == RENAME_WIN {
            return self.rename_event(ev, ctx);
        }
        if win == SAVER_WIN {
            return self.saver_dlg_event(ev, ctx);
        }
        if win == displayhost::MONITOR_WIN {
            return self.monitor_event(ev, ctx);
        }
        if win == CHOOSER_WIN {
            return self.chooser_event(ev, ctx);
        }
        let px = self.sheet.open().units().px(ctx.theme.fonts.size_px);
        if self.sheet.tabs.open != Page::Startup.index() {
            self.focus_list(false);
        }
        if self.sheet.tabs.open != Page::Keyboard.index() {
            self.focus_shortcuts(false);
        }
        if self.sheet.tabs.open != Page::DefaultPrograms.index() {
            self.focus_types(false);
        }
        if self.sheet.tabs.open != Page::Advanced.index() {
            self.focus_locations(false);
        }
        match ev {
            Event::Heads(heads) => self.heads_changed(heads),
            Event::HeadsApplied { serial, ok } => self.heads_applied(serial, ok, &mut ctx.heads),
            Event::Tick => {
                let mut r = self.display_tick(&mut ctx.heads);
                self.saver_ticking = false;
                if self.saver_tick(ctx) {
                    r.tick = true;
                    r.redraw = true;
                }
                if self.sheet.ticking() {
                    r.redraw |=
                        self.sheet.tick(std::time::Instant::now()) == propsheet::Event::Redraw;
                    r.tick = true;
                }
                r
            }
            Event::CloseRequest => self.display_close(&mut ctx.heads),
            Event::Blink => {
                self.sheet.open_mut().blink(ctx.caret_on);
                Reply::redraw()
            }
            Event::Key(k) => {
                let field = self.sheet.open().focused_edit();
                let clip = form_clipboard(field, &Event::Key(k.clone()));
                self.key(k, &ctx.forms().controls()).with_clipboard(clip)
            }
            Event::Pasted(text) => {
                let ev = self.sheet.paste(text);
                self.sheet_event(ev)
            }
            Event::FieldDropped(e) => {
                let ev = self.sheet.dropped(e);
                self.sheet_event(ev)
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) => {
                // A combo's list dropped over the side list covers it.
                let dropped = self.sheet.open().popup_open();
                if !dropped && self.over_side_list(x, y, ctx) {
                    return self.list_press(x, y, time, ctx);
                }
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let was = self.sheet.tabs.open;
                let ev = self.sheet.click(hit);
                if self.sheet.tabs.focused {
                    self.blur_side_lists();
                } else if self.sheet.tabs.open != was {
                    self.blur_side_lists();
                    self.tab_opened();
                }
                let r = self.sheet_event(ev);
                Reply {
                    tick: r.tick || self.sheet.ticking(),
                    ..r
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => {
                let lit = self.list_hover(x, y, ctx);
                if let Some(r) = self.list_motion(x, y, &ctx.forms().controls()) {
                    return r;
                }
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let ev = self.sheet.motion(x, y, hit);
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Leave) => {
                let lit = self.list_unhover(false, ctx);
                let ev = self.sheet.leave();
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let ev = self.sheet.wheel(hit, rows);
                if ev != propsheet::Event::None {
                    return self.sheet_event(ev);
                }
                if !self.over_side_list(x, y, ctx) {
                    return Reply::none();
                }
                let Some((list, rect, _)) = self.side_list() else {
                    return Reply::none();
                };
                let lay = listview::layout(list, rect, &ctx.forms().controls());
                if list.scroll(rows as i64, &lay) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            Event::Pointer(PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                let lit = self.list_unhover(true, ctx);
                if let Some(r) = self.list_release(x, y, &ctx.forms().controls()) {
                    return r;
                }
                let ev = self.sheet.release();
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_RIGHT,
                ..
            }) => {
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let ev = self.sheet.right_press(hit);
                self.sheet_event(ev)
            }
            _ => Reply::none(),
        }
    }
}
