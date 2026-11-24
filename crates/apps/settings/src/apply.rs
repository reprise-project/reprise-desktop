//! OK, Apply and Cancel: writing `config.toml`, the autostart files, the proxy and the machine
//! settings, and rereading them.

use crate::backgroundpage::{self, OwnKey};
use crate::backuphost::Then;
use crate::registry::{self, Page};
use crate::{
    SettingsApp, autostart, defaultspage, displaypage, menupage, model, pages, quickpage,
    read_config, startmenu, take,
};
use repriseconf::Key;
use reprisekit::controls::propsheet;
use std::path::PathBuf;

use repriseapp::{Ctx, Event, Reply, WinId};

pub(crate) const RUN_AS_QUESTION: &str = "Turn off Run As?\nFrom the next log on, programs that need an administrator password (Software Updates, Network Connections, drives) cannot ask for one.";

/// What Restore Defaults takes out of `config.toml` on a page: its keys off their defaults or set
/// in the file (on Start Menu, the menu's too) and, on Background, the connected monitors' own
/// wallpaper keys as (connector, key).
#[derive(Debug, Default)]
pub(crate) struct Restorable {
    pub keys: Vec<Key>,
    pub outputs: Vec<(String, OwnKey)>,
}

impl Restorable {
    fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.outputs.is_empty()
    }
}

/// The question Restore Defaults asks on page `p` before taking `r` out of `config.toml`; a
/// monitor's own key reads "outputs.wallpaper (DP-1)".
pub(crate) fn restore_question(p: Page, r: &Restorable) -> String {
    let mut text = format!(
        "Restore the defaults on the {} page?\nThese settings are taken out of config.toml:\n",
        p.label()
    );
    for k in &r.keys {
        match k.table {
            "" => text.push_str(&format!("    {}\n", k.name)),
            t => text.push_str(&format!("    {t}.{}\n", k.name)),
        }
    }
    for (name, key) in &r.outputs {
        text.push_str(&format!("    outputs.{} ({name})\n", key.key()));
    }
    text.pop();
    text
}

/// The keys the file or the sheet holds off the defaults, against the file's interface's and
/// Classic's, worked out once per [`SettingsApp::sync_restore`].
#[derive(Default)]
struct Apart([Option<Vec<Key>>; 2]);

impl SettingsApp {
    pub(crate) fn user_autostart(&self) -> PathBuf {
        autostart::user_dir(&reprisexdg::config_home().unwrap_or_default())
    }

    /// Re-read the autostart directories; an unapplied check stays on its entry.
    pub(crate) fn rescan(&mut self) {
        let pending = self.pending_checks();
        self.entries = reprisefs::autostart::scan(
            &reprisexdg::config_dirs(),
            &reprisexdg::config_home().unwrap_or_default(),
        );
        // An entry a unit of the session starts is not listed: its check could not stop the unit.
        let units = &self.session_units;
        self.entries.retain(|e| {
            !reprisefs::autostart::left_to_systemd(e, units, reprisefs::autostart::find_program)
        });
        // Checked means it starts here: a system entry fenced off by
        // `OnlyShowIn` is listed unchecked, the way the compositor skips it.
        self.applied_checks = self
            .entries
            .iter()
            .map(|e| e.runs_on(&self.desktops))
            .collect();
        self.show_startup(&pending);
    }

    /// The session's units were read: list the Startup entries without the ones they start.
    pub(crate) fn units_done(&mut self) -> bool {
        let Some(rx) = &self.units_reading else {
            return false;
        };
        let units = match take(rx) {
            Ok(None) => return false,
            Ok(Some(units)) => units,
            Err(()) => Default::default(),
        };
        self.units_reading = None;
        self.session_units = units;
        self.rescan();
        true
    }

    /// The Startup checks moved since the last Apply, by entry id.
    pub(crate) fn pending_checks(&self) -> Vec<(String, bool)> {
        let checks = self.list.checks.iter().flatten();
        (self.entries.iter().zip(checks).zip(&self.applied_checks))
            .filter(|((_, want), have)| want != have)
            .map(|((e, want), _)| (e.id.clone(), *want))
            .collect()
    }

    /// List the entries with their applied checks, each of `pending` put back on its entry.
    pub(crate) fn show_startup(&mut self, pending: &[(String, bool)]) {
        let mut checks = self.applied_checks.clone();
        for (id, want) in pending {
            if let Some(i) = self.entries.iter().position(|e| e.id == *id) {
                checks[i] = *want;
            }
        }
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.sync_remove();
    }

    /// Startup row `row`'s check just flipped. Run As going off flips back on until the question's OK.
    pub(crate) fn startup_toggled(&mut self, row: usize) {
        if registry::open(&self.sheet) != Page::Startup
            || self
                .entries
                .get(row)
                .is_none_or(|e| e.id != autostart::RUN_AS)
            || self.list.checks.as_ref().and_then(|c| c.get(row)) != Some(&false)
        {
            return;
        }
        self.list.toggle(row);
        self.ask(RUN_AS_QUESTION.into(), Then::StopAutostart(row));
    }

    fn checks(&self) -> Vec<bool> {
        self.list.checks.clone().unwrap_or_default()
    }

    pub(crate) fn dirty(&self) -> bool {
        self.want != self.loaded
            || self.checks() != self.applied_checks
            || self.display.model.dirty()
            || self.defaults.as_ref().is_some_and(|d| d.dirty())
            || self.login_dirty()
            || self.proxy_dirty()
            || self.region.as_ref().is_some_and(|r| r.dirty())
    }

    /// Remove works on the user's own entries only; a system entry, or a user file standing in
    /// for one, is unchecked instead.
    fn sync_remove(&mut self) {
        let on = self
            .list
            .selection
            .focus
            .and_then(|i| self.entries.get(i))
            .is_some_and(|e| e.user && !e.stand_in);
        self.sheet.pages[Page::Startup.index()].set_enabled(pages::REMOVE, on);
    }

    pub(crate) fn sync_apply(&mut self) {
        self.sync_remove();
        self.follow_away();
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
        (self.sheet).set_apply_enabled(dirty && self.display.primary_job.is_none());
        self.sync_restore();
    }

    /// Restore Defaults on every page whose controls write config keys, greyed where none is
    /// off its default.
    pub(crate) fn sync_restore(&mut self) {
        let mut apart = Apart::default();
        for p in Page::all() {
            let i = p.index();
            let keys = pages::keys_of(&self.sheet.pages[i]);
            self.sheet.set_restore(i, !keys.is_empty());
            let on = keys.iter().any(|k| self.in_file.contains(k))
                || !self.off_default(keys, &mut apart).is_empty()
                || !self.own_wallpapers(p).is_empty()
                || !self.menu_keys(p).is_empty();
            self.sheet.pages[i].set_enabled(propsheet::RESTORE, on);
        }
    }

    /// The defaults Restore Defaults puts back for `keys`: the file's interface's, Classic's
    /// when the interface itself is among them.
    fn defaults_for(&self, keys: &[Key]) -> &'static model::Settings {
        let interface = match keys.contains(&model::INTERFACE) {
            true => "",
            false => &self.loaded.interface,
        };
        model::defaults(interface)
    }

    /// The keys page `p`'s controls write that the file sets or the sheet holds off their
    /// defaults, and the monitors' own wallpaper keys it restores.
    pub(crate) fn restorable(&self, p: Page) -> Restorable {
        let keys = pages::keys_of(&self.sheet.pages[p.index()]);
        let off = self.off_default(keys.clone(), &mut Apart::default());
        Restorable {
            keys: (keys.into_iter())
                .filter(|k| off.contains(k) || self.in_file.contains(k))
                .chain(self.menu_keys(p))
                .collect(),
            outputs: self.own_wallpapers(p),
        }
    }

    /// On Start Menu, the menu keys the file sets, or the one Apply writes when only the sheet
    /// holds a menu off the default.
    fn menu_keys(&self, p: Page) -> Vec<Key> {
        if p != Page::StartMenu {
            return Vec::new();
        }
        let set: Vec<Key> = [startmenu::STARTMENU, startmenu::CHANGES]
            .into_iter()
            .filter(|k| self.in_file.contains(k))
            .collect();
        match set.is_empty() && self.want.startmenu != startmenu::default_menu() {
            true => vec![startmenu::CHANGES],
            false => set,
        }
    }

    /// On Background, each connected monitor's own `wallpaper` and `wallpaper_position` the
    /// file or the sheet sets, as "All monitors" takes them away: a monitor not connected keeps
    /// its own.
    fn own_wallpapers(&self, p: Page) -> Vec<(String, OwnKey)> {
        if p != Page::Background {
            return Vec::new();
        }
        let set = |name: &str, key: OwnKey| {
            [&self.loaded, &self.want].iter().any(|s| {
                (s.output_wallpapers.iter()).any(|o| o.name == name && !key.of(o).is_empty())
            })
        };
        let mut out = Vec::new();
        for name in self.connected_names() {
            for key in [OwnKey::Wallpaper, OwnKey::Position] {
                if set(&name, key) {
                    out.push((name.clone(), key));
                }
            }
        }
        out
    }

    /// Those of `keys` that the file or the sheet holds off [`Self::defaults_for`] them.
    fn off_default(&self, keys: Vec<Key>, apart: &mut Apart) -> Vec<Key> {
        let classic = keys.contains(&model::INTERFACE);
        let def = self.defaults_for(&keys);
        let off = apart.0[usize::from(classic)].get_or_insert_with(|| {
            let mut off = model::keys_apart(&self.loaded, def);
            off.extend(model::keys_apart(&self.want, def));
            off
        });
        keys.into_iter().filter(|k| off.contains(k)).collect()
    }

    /// Restore Defaults on the open page: ask with the keys it takes out of `config.toml`.
    pub(crate) fn ask_restore(&mut self) {
        let p = registry::open(&self.sheet);
        let r = self.restorable(p);
        if r.is_empty() {
            return;
        }
        self.ask(restore_question(p, &r), Then::RestoreDefaults(p));
    }

    /// Take page `p`'s keys out of `config.toml` the way Apply writes a default, and show the
    /// defaults on the page; the other pages' unapplied changes stay unapplied.
    pub(crate) fn restore_defaults(&mut self, p: Page) {
        let Restorable { keys, outputs } = self.restorable(p);
        let def = self.defaults_for(&keys);
        let mut next = self.loaded.clone();
        model::copy_keys(&keys, def, &mut next);
        let own = |s: &mut model::Settings| {
            for (name, key) in &outputs {
                backgroundpage::set_for_monitor(s, name, *key, String::new());
            }
        };
        own(&mut next);
        let saved = repriseconf::edit(&self.path, |text| {
            model::save(text, &next, &self.loaded).and_then(|t| model::without_keys(&t, &keys))
        });
        match saved {
            Ok(Ok(text)) => self.in_file = model::keys_in(&text),
            Ok(Err(e)) => {
                log::warn!(
                    "{} does not parse, refusing to rewrite it: {e}",
                    self.path.display()
                );
                return self.fail("config.toml does not parse; nothing was written.");
            }
            Err(e) => {
                log::warn!("write {}: {e}", self.path.display());
                return self.fail("config.toml could not be written.");
            }
        }
        model::copy_keys(&keys, def, &mut self.want);
        own(&mut self.want);
        if keys
            .iter()
            .any(|k| [startmenu::STARTMENU, startmenu::CHANGES].contains(k))
        {
            self.want.startmenu = startmenu::default_menu();
        }
        self.config_changed();
        pages::write(&mut self.sheet, &self.want, &self.xkb, &self.system_layout);
        self.fill_wallpapers();
        self.want_preview();
        self.sync_apply();
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
                Ok(Ok(text)) => {
                    self.loaded = self.want.clone();
                    self.in_file = model::keys_in(&text);
                }
                Err(e) => {
                    log::warn!("write {}: {e}", self.path.display());
                    self.fail("config.toml could not be written.");
                    ok = false;
                }
            }
        }
        let dir = self.user_autostart();
        let config_dirs = reprisexdg::config_dirs();
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
            match autostart::set_enabled(&dir, e, want, &self.desktops, &config_dirs) {
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
        if !self.apply_login() {
            ok = false;
        }
        if !self.apply_proxy() {
            ok = false;
        }
        if !self.apply_region() {
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

    /// Show `reason` in an error box, behind the boxes already up or waiting.
    pub(crate) fn fail(&mut self, reason: &str) {
        // `dialog-error` is the theme's, so it is `None` until the
        // first event primed it and the box draws without a column.
        let icon = self.err_icon.clone();
        let over = self.dialog_up().unwrap_or(repriseapp::MAIN);
        self.error
            .show_over(over, "Reprise Settings", reason.to_string(), icon);
    }

    pub(crate) fn error_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        let theme = self.dialog_theme(&ctx.theme);
        self.error.event(win, &ev, &self.dialog_text, &theme)
    }

    /// Reread `config.toml` after a write from elsewhere: the controls the user has not moved show
    /// the file's values, moved ones keep theirs, and Apply still writes only the moved ones. False
    /// when the file is unreadable, does not parse, or reads as the sheet already had it and sets
    /// the same keys.
    pub(crate) fn config_changed(&mut self) -> bool {
        let (text, err) = read_config(&self.path);
        if err.is_some() || text.parse::<toml_edit::DocumentMut>().is_err() {
            return false;
        }
        let in_file = model::keys_in(&text);
        let keys_moved = in_file != self.in_file;
        self.in_file = in_file;
        let fresh = model::load(&text);
        if fresh == self.loaded {
            if keys_moved {
                self.sync_restore();
            }
            return keys_moved;
        }
        model::rebase(&mut self.want, &self.loaded, &fresh);
        self.loaded = fresh;
        pages::write(&mut self.sheet, &self.want, &self.xkb, &self.system_layout);
        self.menu_icons_due = true;
        self.fill_shortcuts();
        self.fill_wallpapers();
        self.want_preview();
        self.sync_apply();
        true
    }

    pub(crate) fn revert(&mut self) {
        self.want = self.loaded.clone();
        pages::write(&mut self.sheet, &self.want, &self.xkb, &self.system_layout);
        self.menu_icons_due = true;
        self.fill_shortcuts();
        let checks = self.applied_checks.clone();
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.display.model.reset();
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
        if let Some(d) = &mut self.defaults {
            d.revert();
        }
        self.refill_defaults();
        self.revert_login();
        self.revert_proxy();
        self.revert_region();
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
