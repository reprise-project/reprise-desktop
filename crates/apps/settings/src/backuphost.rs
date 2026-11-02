//! The Backup page's Export... and Import..., and the question box that confirms a step before it
//! writes.

use crate::backup::{self, Bundle, Roots};
use crate::filepick::Pick;
use crate::registry::Page;
use crate::{SettingsApp, pages};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::msgbox;
use std::path::{Path, PathBuf};

/// What OK in the question box goes on to do.
pub(crate) enum Then {
    Export(PathBuf),
    Import(PathBuf, Bundle),
}

/// The question box: its text, OK and Cancel, and what OK does.
pub(crate) struct Ask {
    pub(crate) msg: MsgBox,
    then: Then,
}

/// The text Import asks with: the file, what it replaces and where the current files go.
pub(crate) fn import_question(file: &Path, bundle: &Bundle, roots: &Roots) -> String {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let mut text = format!("Import {name}? It replaces:\n");
    for e in bundle.replaces() {
        text.push_str(&format!("    {}\n", e.what));
    }
    text.push_str(&format!(
        "The current files are kept in {}.",
        roots.before_import().display()
    ));
    text
}

impl SettingsApp {
    pub(crate) fn ask(&mut self, text: String, then: Then) {
        let msg = MsgBox::new(
            "Reprise Settings",
            msgbox::Fields {
                text,
                icon: self.display.icon.clone(),
                buttons: vec!["OK".into(), "Cancel".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.asking = Some(Ask { msg, then });
    }

    pub(crate) fn ask_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let theme = self.dialog_theme(&ctx.theme);
        let Some(a) = &mut self.asking else {
            return Reply::none();
        };
        match a.msg.event(&ev, &self.dialog_text, &theme) {
            oneshot::Outcome::Closed(button) => {
                let then = self.asking.take().map(|a| a.then);
                if let (Some(0), Some(then)) = (button, then) {
                    self.carry_out(then);
                }
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn carry_out(&mut self, then: Then) {
        match then {
            Then::Export(path) => self.export(&path),
            Then::Import(path, bundle) => self.import(&path, &bundle),
        }
    }

    /// The file dialog picked `path` for Export or Import.
    pub(crate) fn backup_picked(&mut self, pick: Pick, path: PathBuf) {
        match pick {
            Pick::Export if path.exists() => {
                let text = format!(
                    "{} already exists.\nDo you want to replace it?",
                    path.display()
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
            Ok(()) => self.backup_status(&format!("Exported to {}.", path.display())),
            Err(why) => self.fail(&why),
        }
    }

    fn import(&mut self, path: &Path, bundle: &Bundle) {
        match backup::import(bundle, &Roots::current()) {
            Ok(_) => {
                self.backup_status(&format!("Imported {}.", path.display()));
                self.config_changed();
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
    }
}
