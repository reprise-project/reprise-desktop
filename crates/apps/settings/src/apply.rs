//! OK, Apply and Cancel: writing `config.toml` and the autostart files, and rereading them.

use crate::registry::Page;
use crate::{
    SettingsApp, autostart, defaultspage, displaypage, menupage, model, pages, quickpage,
    read_config,
};
use repriseapp::oneshot::{self, MsgBox};
use reprisekit::controls::msgbox;
use std::path::PathBuf;

use repriseapp::{Ctx, Event, Reply};

impl SettingsApp {
    pub(crate) fn user_autostart(&self) -> PathBuf {
        autostart::user_dir(&reprisefs::autostart::config_home())
    }

    /// Re-read the autostart directories; unapplied checks are dropped, the files changed under them.
    pub(crate) fn rescan(&mut self) {
        // An entry a unit of the session starts is not listed: its check could not stop the unit.
        let units = reprisebus::units::session_programs();
        self.entries = reprisefs::autostart::scan(
            &reprisefs::autostart::system_dirs(),
            &reprisefs::autostart::config_home(),
        );
        self.entries.retain(|e| {
            !reprisefs::autostart::left_to_systemd(e, &units, reprisefs::autostart::find_program)
        });
        // Checked means it starts here: a system entry fenced off by
        // `OnlyShowIn` is listed unchecked, the way the compositor skips it.
        self.applied_checks = self
            .entries
            .iter()
            .map(|e| e.runs_on(&self.desktops))
            .collect();
        let checks = self.applied_checks.clone();
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.sync_remove();
    }

    fn checks(&self) -> Vec<bool> {
        self.list.checks.clone().unwrap_or_default()
    }

    pub(crate) fn dirty(&self) -> bool {
        self.want != self.loaded
            || self.checks() != self.applied_checks
            || self.display.model.dirty()
            || self.defaults.as_ref().is_some_and(|d| d.dirty())
    }

    /// Remove works on user entries only; a system entry has no file of the user's.
    fn sync_remove(&mut self) {
        let on = self
            .list
            .selection
            .focus
            .and_then(|i| self.entries.get(i))
            .is_some_and(|e| e.user);
        self.sheet.pages[Page::Startup.index()].set_enabled(pages::REMOVE, on);
    }

    pub(crate) fn sync_apply(&mut self) {
        self.sync_remove();
        self.sync_forget();
        menupage::sync(
            &mut self.sheet.pages[Page::StartMenu.index()],
            &self.want.startmenu,
        );
        quickpage::sync(
            &mut self.sheet.pages[Page::StartMenu.index()],
            self.quick.len(),
        );
        if let Some(d) = &self.defaults {
            defaultspage::sync_buttons(
                &mut self.sheet.pages[Page::DefaultPrograms.index()],
                d,
                self.types.selection.focus,
            );
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    /// Write `config.toml` and the autostart files whose check moved. False when a write failed; the sheet stays dirty.
    pub(crate) fn apply(&mut self) -> bool {
        if let Err(reason) = model::validate(&self.want, &self.loaded) {
            self.fail(&reason);
            return false;
        }
        let mut ok = true;
        if self.want != self.loaded {
            // Re-read under the lock: the user's editor or reprise-power may
            // have written since the sheet opened.
            let saved = repriseconf::edit(&self.path, |text| {
                model::save(text, &self.want, &self.loaded)
            });
            match saved {
                // A file that stopped parsing is not ours to rewrite: it holds keys the sheet does not know.
                Ok(Err(e)) => {
                    log::warn!(
                        "{} does not parse, refusing to rewrite it: {e}",
                        self.path.display()
                    );
                    self.fail("config.toml does not parse; nothing was written.");
                    ok = false;
                }
                Ok(Ok(_)) => self.loaded = self.want.clone(),
                Err(e) => {
                    log::warn!("write {}: {e}", self.path.display());
                    self.fail("config.toml could not be written.");
                    ok = false;
                }
            }
        }
        let dir = self.user_autostart();
        let checks = self.checks();
        let mut changed = false;
        let mut failed_autostart = false;
        for (i, e) in self.entries.iter().enumerate() {
            let (Some(&want), Some(&have)) = (checks.get(i), self.applied_checks.get(i)) else {
                continue;
            };
            if want == have {
                continue;
            }
            match autostart::set_enabled(&dir, e, want) {
                Ok(()) => changed = true,
                Err(err) => {
                    log::warn!("autostart {}: {err}", e.id);
                    failed_autostart = true;
                    ok = false;
                }
            }
        }
        if failed_autostart {
            self.fail("The autostart entry could not be written.");
        }
        if changed {
            self.rescan();
        }
        if !self.apply_defaults() {
            ok = false;
        }
        self.sync_apply();
        ok
    }

    fn apply_defaults(&mut self) -> bool {
        let Some(d) = self.defaults.as_mut().filter(|d| d.dirty()) else {
            return true;
        };
        let written = reprisefs::props::edit_defaults(&d.edits).and_then(|()| {
            let path = reprisefs::props::user_mimeapps()?;
            std::fs::read_to_string(path)
        });
        match written {
            Ok(now) => {
                d.applied(now);
                self.refill_defaults();
                true
            }
            Err(e) => {
                log::warn!("mimeapps.list: {e}");
                self.fail("The default programs could not be written to mimeapps.list.");
                false
            }
        }
    }

    /// Show `reason` in the error box; the first failure wins.
    pub(crate) fn fail(&mut self, reason: &str) {
        if self.error.is_none() {
            // `dialog-error` is the theme's, so it is `None` until the
            // first event primed it and the box draws without a column.
            self.error = Some(MsgBox::new(
                "Reprise Settings",
                msgbox::Fields {
                    text: reason.to_string(),
                    icon: self.err_icon.clone(),
                    buttons: vec!["OK".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                    max_w: repriseapp::dialog_max_w(),
                },
            ));
        }
    }

    pub(crate) fn error_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let theme = self.dialog_theme(&ctx.theme);
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &theme) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    /// Reread `config.toml` after a write from elsewhere: the controls the user has not moved show
    /// the file's values, moved ones keep theirs, and Apply still writes only the moved ones. False
    /// when the file is unreadable, does not parse, or reads as the sheet already had it.
    pub(crate) fn config_changed(&mut self) -> bool {
        let (text, err) = read_config(&self.path);
        if err.is_some() || text.parse::<toml_edit::DocumentMut>().is_err() {
            return false;
        }
        let fresh = model::load(&text);
        if fresh == self.loaded {
            return false;
        }
        model::rebase(&mut self.want, &self.loaded, &fresh);
        self.loaded = fresh;
        pages::write(&mut self.sheet, &self.want, &self.xkb);
        self.menu_icons_due = true;
        self.fill_shortcuts();
        self.fill_wallpapers();
        self.want_preview();
        self.sync_apply();
        true
    }

    pub(crate) fn revert(&mut self) {
        self.want = self.loaded.clone();
        pages::write(&mut self.sheet, &self.want, &self.xkb);
        self.menu_icons_due = true;
        self.fill_shortcuts();
        let checks = self.applied_checks.clone();
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.display.model.reset();
        displaypage::refresh(&mut self.sheet, &self.display.model);
        if let Some(d) = &mut self.defaults {
            d.revert();
        }
        self.refill_defaults();
        self.fill_wallpapers();
        self.want_preview();
        self.show_monitor();
        self.sync_apply();
    }

    /// The Interface combo moved off `from`: the scheme and the unset defaults follow the new interface ([`model::switch_interface`]); picking back the loaded one restores the loaded scheme and glass colour.
    pub(crate) fn interface_moved(&mut self, from: &str) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        model::switch_interface(&mut self.want, from, &text, &self.loaded);
        pages::write_forms(&mut self.sheet, &self.want);
    }
}
