//! The Keyboard page's shortcut list and layouts, and their dialogs.

use crate::registry::Page;
use crate::{SettingsApp, layoutdlg, model, pages, shortcutdlg};
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::listview;

impl SettingsApp {
    pub(crate) fn fill_shortcuts(&mut self) {
        let names = &self.app_names;
        pages::fill_shortcuts(&mut self.shortcuts, &self.want, &|app| {
            names.get(app).cloned().unwrap_or_else(|| app.to_string())
        });
        let vision = &mut self.sheet.pages[Page::Vision.index()];
        crate::visionpage::show_zoom_keys(vision, &self.want.shortcuts);
    }

    /// Every key taken but `row`'s; a program's is under its list text so the conflict note names it.
    pub(crate) fn taken_keys(&self, row: Option<model::Row>) -> Vec<model::Shortcut> {
        let binds = (self.want.shortcuts.iter().enumerate())
            .filter(|(i, _)| row != Some(model::Row::Bind(*i)))
            .map(|(_, o)| o.clone());
        let name = |app: &str| {
            self.app_names
                .get(app)
                .cloned()
                .unwrap_or_else(|| app.to_string())
        };
        let programs = (self.want.programs.iter().enumerate())
            .filter(|(i, p)| row != Some(model::Row::Program(*i)) && !p.keys.is_empty())
            .map(|(_, p)| {
                model::Shortcut::new(&p.keys, &model::program_text(p, &name(&p.app)), "")
            });
        binds.chain(programs).collect()
    }

    pub(crate) fn open_shortcut(&mut self, row: Option<usize>) {
        let target = row.map(model::Row::Bind);
        let sc = row.and_then(|i| self.want.shortcuts.get(i));
        let others = self.taken_keys(target);
        self.shortcut = Some(shortcutdlg::ShortcutDialog::new(sc, others));
        self.shortcut_target = target;
    }

    pub(crate) fn edit_shortcut(&mut self) -> Reply {
        let Some(i) = self.shortcuts.selection.focus else {
            return Reply::none();
        };
        match model::row(&self.want, i) {
            Some(model::Row::Bind(j)) => self.open_shortcut(Some(j)),
            Some(r @ model::Row::Program(j)) => {
                let others = self.taken_keys(Some(r));
                let keys = &self.want.programs[j].keys;
                self.shortcut = Some(shortcutdlg::ShortcutDialog::for_program(keys, others));
                self.shortcut_target = Some(r);
            }
            Some(model::Row::Denied(_)) | None => return Reply::none(),
        }
        Reply::redraw_all()
    }

    /// OK in the Shortcut dialog: `sc` replaces the row it was opened on or is added after the binds, and a "none" row on its chord goes. Returns its list row.
    pub(crate) fn accept_shortcut(&mut self, sc: model::Shortcut) -> usize {
        let target = self.shortcut_target.take();
        let freed = (self.want.shortcuts.iter().enumerate()).position(|(i, o)| {
            target != Some(model::Row::Bind(i))
                && o.action == repriseconf::keybind::NONE
                && repriseconf::keybind::same(&o.keys, &sc.keys)
        });
        let target = match (target, freed) {
            (Some(model::Row::Bind(i)), Some(j)) if i < self.want.shortcuts.len() => {
                self.want.shortcuts.remove(j);
                Some(model::Row::Bind(if j < i { i - 1 } else { i }))
            }
            (None, Some(j)) => Some(model::Row::Bind(j)),
            (target, _) => target,
        };
        let row = match target {
            Some(model::Row::Program(j)) if j < self.want.programs.len() => {
                self.want.programs[j].keys = sc.keys;
                self.want.shortcuts.len() + j
            }
            Some(model::Row::Bind(i)) if i < self.want.shortcuts.len() => {
                self.want.shortcuts[i] = sc;
                i
            }
            _ => {
                self.want.shortcuts.push(sc);
                self.want.shortcuts.len() - 1
            }
        };
        self.shortcut = None;
        self.fill_shortcuts();
        row
    }

    pub(crate) fn remove_shortcut(&mut self) -> Reply {
        let Some(r) = (self.shortcuts.selection.focus).and_then(|i| model::row(&self.want, i))
        else {
            return Reply::none();
        };
        model::forget(&mut self.want, r);
        self.fill_shortcuts();
        self.sync_apply();
        Reply::redraw()
    }

    pub(crate) fn shortcut_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.shortcut.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => shortcutdlg::Outcome::Cancel,
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
            shortcutdlg::Outcome::None => Reply::none(),
            shortcutdlg::Outcome::Redraw => Reply::redraw(),
            shortcutdlg::Outcome::Cancel => {
                self.shortcut = None;
                self.shortcut_target = None;
                Reply::redraw_all()
            }
            shortcutdlg::Outcome::Accept(sc) => {
                let row = self.accept_shortcut(sc);
                let lay = listview::layout(
                    &self.shortcuts,
                    self.on_page(pages::shortcuts(self.area())),
                    &ctx.forms().controls(),
                );
                self.shortcuts.select(Some(row), &lay);
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    fn layouts_changed(&mut self, select: Option<usize>) -> Reply {
        pages::fill_layouts(&mut self.sheet, &self.want, &self.xkb, select);
        self.sync_apply();
        Reply::redraw()
    }

    pub(crate) fn remove_layout(&mut self) -> Reply {
        let Some(i) = pages::layout_selection(&self.sheet) else {
            return Reply::none();
        };
        self.want.remove_layout(i);
        let left = self.want.layouts().len();
        self.layouts_changed((left > 0).then(|| i.min(left - 1)))
    }

    pub(crate) fn move_layout_up(&mut self) -> Reply {
        match pages::layout_selection(&self.sheet) {
            Some(i) if self.want.move_layout_up(i) => self.layouts_changed(Some(i - 1)),
            _ => Reply::none(),
        }
    }

    pub(crate) fn add_layout_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.add_layout.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => layoutdlg::Outcome::Cancel,
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            layoutdlg::Outcome::None => Reply::none(),
            layoutdlg::Outcome::Redraw => Reply::redraw(),
            layoutdlg::Outcome::Cancel => {
                self.add_layout = None;
                Reply::redraw_all()
            }
            layoutdlg::Outcome::Accept(layout, variant) => {
                self.add_layout = None;
                if self.want.add_layout(&layout, &variant) {
                    let last = self.want.layouts().len() - 1;
                    self.layouts_changed(Some(last));
                }
                Reply::redraw_all()
            }
        }
    }
}
