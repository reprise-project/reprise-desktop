//! The Start Menu and Quick Launch pages with their Add Item and Rename dialogs.

use crate::registry::Page;
use crate::{SettingsApp, additem, menupage, quickpage, renamedlg, startmenu};
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::openwith;

impl SettingsApp {
    pub(crate) fn menu_button(&mut self, id: &str) -> Reply {
        let (page, menu, cm) = (
            &mut self.sheet.pages[Page::StartMenu.index()],
            &mut self.want.startmenu,
            &self.controls,
        );
        let done = match id {
            menupage::FOLDER => menupage::new_folder(page, menu, cm),
            menupage::REMOVE => menupage::remove(page, menu, cm),
            menupage::UP => menupage::shift(page, menu, -1, cm),
            menupage::DOWN => menupage::shift(page, menu, 1, cm),
            _ => false,
        };
        self.menu_changed(done)
    }

    fn menu_changed(&mut self, done: bool) -> Reply {
        if !done {
            return Reply::redraw();
        }
        self.menu_icons_due = true;
        self.sync_apply();
        Reply::redraw_all()
    }

    pub(crate) fn add_item_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.add_item.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => additem::Outcome::Cancel,
                Event::Blink => {
                    d.form.blink(ctx.caret_on);
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            additem::Outcome::None => Reply::none(),
            additem::Outcome::Redraw => Reply::redraw(),
            additem::Outcome::Cancel => {
                self.add_item = None;
                Reply::redraw_all()
            }
            additem::Outcome::Accept(v) if self.add_to_quick => {
                self.add_item = None;
                self.quick_add(&v)
            }
            additem::Outcome::Accept(v) => {
                self.add_item = None;
                let icon = v.icon.as_deref().unwrap_or("");
                let e = startmenu::Entry::program(&v.name, icon, v.command);
                let done = menupage::insert(
                    &mut self.sheet.pages[Page::StartMenu.index()],
                    &mut self.want.startmenu,
                    e,
                    &self.controls,
                )
                .is_some();
                self.menu_changed(done)
            }
        }
    }

    pub(crate) fn rename_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.rename.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => renamedlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.blink(ctx.caret_on);
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            renamedlg::Outcome::None => Reply::none(),
            renamedlg::Outcome::Redraw => Reply::redraw(),
            renamedlg::Outcome::Cancel => {
                self.rename = None;
                Reply::redraw_all()
            }
            renamedlg::Outcome::Accept(label) => {
                self.rename = None;
                let done = menupage::rename(
                    &mut self.sheet.pages[Page::StartMenu.index()],
                    &mut self.want.startmenu,
                    &label,
                );
                self.menu_changed(done)
            }
        }
    }

    /// Look up icons a change left missing; runs after every event.
    pub(crate) fn prime_icons(&mut self, ctx: &mut Ctx) {
        let mut get = |name: &str| ctx.icons.get(name, 16);
        if self.menu_icons_due {
            menupage::set_icons(
                &mut self.sheet.pages[Page::StartMenu.index()],
                &self.want.startmenu,
                &mut get,
            );
            self.menu_icons_due = false;
        }
        if self.quick_icons_due {
            quickpage::set_icons(
                &mut self.sheet.pages[Page::StartMenu.index()],
                &self.quick,
                &mut get,
            );
            self.quick_icons_due = false;
        }
        if let Some(d) = self.add_item.as_mut().filter(|d| d.icons_due) {
            d.set_icons(&mut get);
        }
        if self.location_icons_due {
            for (row, name) in self.locations.rows.iter_mut().zip(&self.location_icons) {
                row.icon = name
                    .as_deref()
                    .and_then(&mut get)
                    .or_else(|| get(openwith::PROGRAM_ICON))
                    .map(std::rc::Rc::new);
            }
            self.location_icons_due = false;
        }
        if let Some(c) = self.chooser.as_mut().filter(|c| c.icons_due) {
            c.row_icons =
                c.ow.list
                    .rows
                    .iter()
                    .map(|r| openwith::icon_name(r).and_then(&mut get))
                    .collect();
            c.icon = ctx.icons.get(openwith::ICON, 32);
            c.icons_due = false;
        }
    }

    pub(crate) fn quick_reload(&mut self, file: Option<&str>) {
        self.quick = self
            .quick_dir
            .as_deref()
            .map(quickpage::load)
            .unwrap_or_default();
        let pick = file.and_then(|f| self.quick.iter().position(|l| l.file == f));
        quickpage::fill(
            &mut self.sheet.pages[Page::StartMenu.index()],
            &self.quick,
            pick,
        );
        self.quick_icons_due = true;
    }

    fn quick_add(&mut self, v: &additem::NewItem) -> Reply {
        let Some(dir) = self.quick_dir.clone() else {
            return Reply::redraw_all();
        };
        let written = quickpage::add(&dir, v).and_then(|file| {
            self.quick.push(quickpage::Launcher {
                file: file.clone(),
                name: v.name.clone(),
                icon: None,
            });
            quickpage::store(&dir, &self.quick).map(|()| file)
        });
        let file = match written {
            Ok(f) => Some(f),
            Err(e) => {
                log::warn!("quick launch add {:?}: {e}", v.name);
                self.fail("The Quick Launch button could not be written.");
                None
            }
        };
        self.quick_reload(file.as_deref());
        self.sync_apply();
        Reply::redraw_all()
    }

    pub(crate) fn quick_button(&mut self, id: &str) -> Reply {
        let page = &self.sheet.pages[Page::StartMenu.index()];
        let (Some(dir), Some(i)) = (self.quick_dir.clone(), quickpage::selected(page)) else {
            return Reply::none();
        };
        let Some(file) = self.quick.get(i).map(|l| l.file.clone()) else {
            return Reply::none();
        };
        let mut pick = Some(file.clone());
        let done = match id {
            quickpage::REMOVE => {
                self.quick.remove(i);
                pick = self
                    .quick
                    .get(i)
                    .or(self.quick.last())
                    .map(|l| l.file.clone());
                quickpage::remove(&dir, &file).and_then(|()| quickpage::store(&dir, &self.quick))
            }
            quickpage::UP if i > 0 => {
                self.quick.swap(i, i - 1);
                quickpage::store(&dir, &self.quick)
            }
            quickpage::DOWN if i + 1 < self.quick.len() => {
                self.quick.swap(i, i + 1);
                quickpage::store(&dir, &self.quick)
            }
            _ => return Reply::none(),
        };
        if let Err(e) = done {
            log::warn!("quick launch {id} {file}: {e}");
            self.fail("The Quick Launch folder could not be written.");
        }
        self.quick_reload(pick.as_deref());
        self.sync_apply();
        Reply::redraw()
    }
}
