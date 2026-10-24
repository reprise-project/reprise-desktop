use crate::registry::Page;
use crate::{Chooser, SettingsApp, defaultspage, filepick::Pick, pages, registry, sidelists};
use repriseapp::openwith::ChooserInput;
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::form;
use reprisekit::controls::openwith::{self, OpenWith, Program};

impl SettingsApp {
    pub(crate) fn ensure_defaults(&mut self) {
        if self.defaults.is_some() || registry::open(&self.sheet) != Page::DefaultPrograms {
            return;
        }
        self.defaults = Some(defaultspage::Defaults::load());
        self.refill_defaults();
    }

    pub(crate) fn refill_defaults(&mut self) {
        let Some(d) = &self.defaults else {
            return;
        };
        let keep = defaultspage::picked(&self.types, d).map(|t| t.mime.clone());
        defaultspage::fill_list(&mut self.types, d, keep.as_deref());
        defaultspage::fill(
            &mut self.sheet.pages[Page::DefaultPrograms.index()],
            d,
            &self.want.terminal,
            self.types.selection.focus,
        );
    }

    pub(crate) fn focus_types(&mut self, on: bool) {
        sidelists::hand_focus(&mut self.sheet, &mut self.types, &mut self.types_focus, on);
    }

    pub(crate) fn defaults_changed(&mut self, id: &str) -> Reply {
        if id == defaultspage::DRIVE_INSERT || id == defaultspage::SCRIPT_END {
            pages::read_one(&self.sheet, id, &mut self.want);
            self.sync_apply();
            return Reply::redraw();
        }
        let page = &self.sheet.pages[Page::DefaultPrograms.index()];
        if id == defaultspage::TERMINAL_ARGS {
            if let Some(form::Control::Edit { edit, .. }) = page.item(id).map(|i| &i.control) {
                let program = defaultspage::split_command(&self.want.terminal).0;
                self.want.terminal = defaultspage::join_command(&program, &edit.text);
            }
            self.sync_apply();
            return Reply::redraw();
        }
        let Some(d) = self.defaults.as_mut() else {
            return Reply::redraw();
        };
        if id == defaultspage::FILTER {
            if let Some(form::Control::Edit { edit, .. }) = page.item(id).map(|i| &i.control) {
                d.set_filter(&edit.text);
            }
            let keep = defaultspage::picked(&self.types, d).map(|t| t.mime.clone());
            defaultspage::fill_list(&mut self.types, d, keep.as_deref());
            self.types.top = 0;
            self.sync_apply();
            return Reply::redraw();
        }
        let row = page.combo_selected(id).unwrap_or(0);
        if id == defaultspage::TERMINAL {
            if let Some(t) = d.terminal_pick(row) {
                let args = defaultspage::split_command(&self.want.terminal).1;
                self.want.terminal = defaultspage::join_command(t, args);
            }
        } else if let Some(k) = defaultspage::KINDS.iter().find(|k| k.id == id) {
            d.pick(k, row);
        }
        self.refill_defaults();
        self.sync_apply();
        Reply::redraw()
    }

    pub(crate) fn restore_default(&mut self) -> Reply {
        let Some(d) = self.defaults.as_mut() else {
            return Reply::none();
        };
        let Some(mime) = defaultspage::picked(&self.types, d).map(|t| t.mime.clone()) else {
            return Reply::none();
        };
        d.set(&mime, None);
        self.refill_defaults();
        self.sync_apply();
        Reply::redraw()
    }

    /// Change...: the Open With chooser over the picked type, "Always use" ticked and fixed.
    pub(crate) fn open_chooser(&mut self) -> Reply {
        let Some(d) = &self.defaults else {
            return Reply::none();
        };
        let Some(t) = defaultspage::picked(&self.types, d) else {
            return Reply::none();
        };
        let apps = &d.apps;
        let default = d.current(&t.mime);
        let (recommended, others) = reprisefs::apps::chooser_groups(apps, Some(&t.mime), default);
        let program = |i: usize| Program {
            id: apps[i].id.clone(),
            name: apps[i].name.clone(),
            icon: apps[i].icon.clone(),
        };
        let name = match &t.ext {
            Some(e) => format!("{} ({e})", t.description),
            None => t.description.clone(),
        };
        let mut ow = OpenWith::new(
            &name,
            recommended.into_iter().map(program).collect(),
            others.into_iter().map(program).collect(),
        );
        if let Some(i) = default {
            ow.select_id(&apps[i].id);
        }
        ow.always = openwith::Check {
            on: true,
            enabled: false,
        };
        self.chooser = Some(Chooser {
            mime: t.mime.clone(),
            ow,
            input: ChooserInput::opened(false),
            icon: None,
            row_icons: Vec::new(),
            icons_due: true,
        });
        Reply::redraw_all()
    }

    pub(crate) fn chooser_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let dt = self.dialog_theme(&ctx.theme);
        let Some(c) = self.chooser.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => openwith::Event::Cancel,
            ev => {
                let lay =
                    c.ow.layout(&self.dialog_text, dt.fonts.size_px, (0, 0), &dt.controls());
                c.input.event(&mut c.ow, &lay, &ev, &ctx.theme.metrics)
            }
        };
        match out {
            openwith::Event::None => Reply::none(),
            openwith::Event::Browse => {
                self.open_add(Pick::Program);
                Reply::redraw_all()
            }
            openwith::Event::Redraw => Reply::redraw(),
            openwith::Event::Cancel => {
                self.chooser = None;
                Reply::redraw_all()
            }
            openwith::Event::Choose { id, .. } => {
                let mime = c.mime.clone();
                self.chooser = None;
                let dir = reprisefs::apps::user_applications_dir();
                let chosen = match self.defaults.as_mut() {
                    Some(d) => d.chosen(&id, &mime, dir.as_deref()),
                    None => Ok(id.clone()),
                };
                let id = match chosen {
                    Ok(id) => id,
                    Err(e) => {
                        log::warn!("no desktop entry for {id}: {e}");
                        self.fail(&format!("Cannot use '{id}': {e}."));
                        return Reply::redraw_all();
                    }
                };
                if let Some(d) = self.defaults.as_mut() {
                    let now = d.current(&mime).map(|i| d.apps[i].id.as_str());
                    if now != Some(id.as_str()) {
                        d.set(&mime, Some(&id));
                    }
                }
                self.refill_defaults();
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    /// Browse... picked `path`: it joins the chooser's list, selected.
    pub(crate) fn browsed_program(&mut self, path: &std::path::Path) {
        let Some(c) = self.chooser.as_mut() else {
            return;
        };
        match repriseapp::openwith::program_pick(path) {
            Some(p) => {
                c.ow.add_program(p);
                c.icons_due = true;
            }
            None => log::warn!("{} is not UTF-8, cannot run it", path.display()),
        }
    }
}
