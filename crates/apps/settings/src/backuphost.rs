//! The Backup page's Export... and Import..., and the question box that confirms a step (these,
//! Restore Defaults and turning off Run As) before it writes.

use crate::backup::{self, Bundle, Roots};
use crate::filepick::Pick;
use crate::registry::Page;
use crate::{SettingsApp, pages};
use repriseapp::asks::Answer;
use repriseapp::oneshot::MsgBox;
use repriseapp::{Ctx, Event, Reply, WinId};
use reprisekit::controls::msgbox;
use reprisekit::tr;
use std::path::{Path, PathBuf};

/// What OK in a question box goes on to do.
pub(crate) enum Then {
    Export(PathBuf),
    Import(PathBuf, Bundle),
    RestoreDefaults(Page),
    NewCertificate,
    /// Uncheck this Startup row.
    StopAutostart(usize),
}

/// The text Import asks with: the file, what it replaces and where the current files go.
pub(crate) fn import_question(file: &Path, bundle: &Bundle, roots: &Roots) -> String {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let (replaces, removes) = (bundle.replaces(), bundle.removes());
    let lines = |head: &str, list: &[&backup::Entry]| {
        (list.iter()).fold(format!("{head}\n"), |t, e| {
            t + &format!("    {}\n", (e.what)())
        })
    };
    let mut parts = Vec::new();
    if !replaces.is_empty() || removes.is_empty() {
        parts.push(lines(&tr!("backuphost-it-replaces"), &replaces));
    }
    if !removes.is_empty() {
        parts.push(lines(&tr!("backuphost-it-removes"), &removes));
    }
    tr!(
        "backuphost-import-question",
        name = name,
        lists = parts.concat(),
        kept = roots.before_import().display().to_string()
    )
}

impl SettingsApp {
    /// Asks `text` with OK and Cancel, once the questions before it are answered.
    pub(crate) fn ask(&mut self, text: String, then: Then) {
        let msg = MsgBox::new(
            tr!("backuphost-reprise-settings"),
            msgbox::Fields {
                text,
                icon: self.display.icon.clone(),
                buttons: vec![tr!("backuphost-ok"), tr!("backuphost-cancel")],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.asking.ask(msg, then);
    }

    pub(crate) fn ask_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        let theme = self.dialog_theme(&ctx.theme);
        match self.asking.event(win, &ev, &self.dialog_text, &theme) {
            Answer::Closed(then, button) => {
                if button == Some(0) {
                    self.carry_out(then);
                }
                Reply::redraw_all()
            }
            Answer::Redraw => Reply::redraw(),
            Answer::None => Reply::none(),
        }
    }

    fn carry_out(&mut self, then: Then) {
        match then {
            Then::Export(path) => self.export(&path),
            Then::Import(path, bundle) => self.import(&path, &bundle),
            Then::RestoreDefaults(p) => self.restore_defaults(p),
            Then::NewCertificate => self.remote_act(crate::remotehost::Act::NewCertificate),
            Then::StopAutostart(row) => {
                if let Some(c) = self.list.checks.as_mut().and_then(|c| c.get_mut(row)) {
                    *c = false;
                }
                self.sync_apply();
            }
        }
    }

    /// The file dialog picked `path` for Export or Import.
    pub(crate) fn backup_picked(&mut self, pick: Pick, path: PathBuf) {
        match pick {
            Pick::Export if path.exists() => {
                let text = tr!(
                    "backuphost-replace-question",
                    path = path.display().to_string()
                );
                self.ask(text, Then::Export(path));
            }
            Pick::Export => self.export(&path),
            Pick::Import => match backup::read(&path) {
                Ok(bundle) => {
                    let text = import_question(&path, &bundle, &Roots::current());
                    self.ask(text, Then::Import(path, bundle));
                }
                Err(why) => self.fail(&why),
            },
            _ => {}
        }
    }

    fn export(&mut self, path: &Path) {
        match backup::export_to(path, &Roots::current()) {
            Ok(()) => self.backup_status(&tr!(
                "backuphost-exported",
                path = path.display().to_string()
            )),
            Err(why) => self.fail(&why),
        }
    }

    fn import(&mut self, path: &Path, bundle: &Bundle) {
        match backup::import(bundle, &Roots::current()) {
            Ok(_) => {
                self.backup_status(&tr!(
                    "backuphost-imported",
                    path = path.display().to_string()
                ));
                self.config_changed();
                self.reread_display_files();
                self.scan_profiles();
                self.quick_reload(None);
            }
            Err(why) => self.fail(&why),
        }
    }

    fn backup_status(&mut self, text: &str) {
        self.sheet.pages[Page::Backup.index()].set_text(pages::BACKUP_STATUS, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_asks_with_what_it_replaces_and_where_the_old_files_go() {
        let roots = Roots {
            config: "/home/u/.config/reprise".into(),
            state: "/home/u/.local/state/reprise".into(),
        };
        let text = "[reprise_settings]\nversion = 1\n[files]\n\
                    \"config/config.toml\" = \"\"\n\"state/displays.toml\" = \"\"\n";
        let bundle = Bundle::parse(text).unwrap();
        assert_eq!(
            import_question(Path::new("/home/u/s.toml"), &bundle, &roots),
            "Import s.toml? It replaces:\n    Settings and keyboard shortcuts (config.toml)\n    \
             Display arrangements\nThe current files are kept in \
             /home/u/.config/reprise/reprise-settings-before-import.toml."
        );
        let undo = "absent = [\"state/displays.toml\"]\n[reprise_settings]\nversion = 1\n[files]\n";
        let bundle = Bundle::parse(undo).unwrap();
        assert_eq!(
            import_question(Path::new("/home/u/s.toml"), &bundle, &roots),
            "Import s.toml? It removes:\n    Display arrangements\nThe current files are kept in \
             /home/u/.config/reprise/reprise-settings-before-import.toml."
        );
    }
}
