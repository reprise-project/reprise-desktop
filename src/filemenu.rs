//! The context menu of a file or folder found by search: the desktop's icon menu
//! (`desktopview::icon_menu`), acting on the path, with Open file location after
//! its Open group. Rename is greyed, as a search row has no field to rename in.

use std::path::{Path, PathBuf};

use reprisefs::apps;
use reprisekit::controls::menu::MenuItem;

use crate::compositor::State;
use crate::desktopview::{DesktopCommand, IconMenu, entry, icon_menu};
use crate::dragview::{DragSource, DropOp, DropTarget};

/// The menu of a folder (`is_dir`) or of a file of type `mime`.
pub fn file_menu(is_dir: bool, mime: Option<&str>) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    let (print, edit) = if is_dir {
        (false, false)
    } else {
        reprisefs::props::verbs(mime, reprisefs::props::has_lp())
    };
    let (mut items, mut commands) = icon_menu(IconMenu {
        is_dir,
        print,
        edit,
        ..IconMenu::default()
    });
    let after_open = items
        .iter()
        .position(|i| matches!(i, MenuItem::Separator))
        .unwrap_or(items.len());
    items.insert(after_open, entry("Open f&ile location"));
    commands.insert(after_open, Some(DesktopCommand::OpenFileLocation));
    for (item, command) in items.iter_mut().zip(&commands) {
        if *command == Some(DesktopCommand::Rename) {
            *item = item.clone().enabled(false);
        }
    }
    (items, commands)
}

impl State {
    pub(crate) fn file_menu_rows(
        &self,
        path: &Path,
    ) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
        let is_dir = path.is_dir();
        let mime = (!is_dir)
            .then(|| self.with_mime(|db| apps::mime_of_in(db, path)))
            .flatten();
        file_menu(is_dir, mime.as_deref())
    }

    /// Row `cmd` of `file_menu` picked for `path`.
    pub(crate) fn file_command(&mut self, cmd: DesktopCommand, path: PathBuf) {
        use DesktopCommand as Dc;
        let source = || vec![DragSource::file(path.clone())];
        match cmd {
            Dc::Open | Dc::Explore => {
                let argv = self.with_mime(|db| {
                    reprisefs::props::open_argv_in(
                        &path,
                        db,
                        &self.app_index,
                        &self.config.terminal,
                    )
                });
                self.spawn_argv_activated(&argv);
            }
            Dc::OpenFileLocation => self.open_file_location(&path),
            Dc::Print | Dc::Edit => self.print_or_edit(vec![path], cmd == Dc::Print),
            Dc::OpenWithApp(app) => self.open_files_with(vec![path], &app),
            Dc::ChooseProgram => {
                let d = self.choose_program(&path, false);
                self.open_dialog(d);
            }
            Dc::SendToDest(row) => {
                let existing: Vec<String> = path
                    .parent()
                    .and_then(|d| std::fs::read_dir(d).ok())
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .collect();
                self.send_sources_to(row, &source(), &existing, None);
            }
            Dc::Cut | Dc::Copy => {
                if self.is_locked() {
                    return;
                }
                let clip = crate::clipboard::FileClip {
                    paths: vec![path],
                    cut: cmd == Dc::Cut,
                };
                if let Some(offers) = crate::clipboard::file_offers(&clip) {
                    self.clip_offer_each(offers);
                }
            }
            Dc::PasteInto => self.paste_files(DropTarget::Folder(path), false),
            Dc::CreateShortcut => {
                if let Some(dir) = path.parent().map(Path::to_path_buf) {
                    self.perform_all(&source(), &DropTarget::Folder(dir), DropOp::Shortcut);
                }
            }
            Dc::Trash => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.ask_delete(vec![path], name, false);
            }
            Dc::IconProperties => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let p = self.pointer_location();
                match reprisefs::props::glance(&path, &name, &self.app_index) {
                    Ok(info) => {
                        let icon = self.file_icon_name(&path);
                        self.open_properties(&info, &icon, (p.x as i32, p.y as i32));
                    }
                    Err(err) => log::warn!("properties: {}: {err}", path.display()),
                }
            }
            _ => {}
        }
    }

    /// Explorer on `path`'s folder with `path` selected; the launcher's
    /// Ctrl+Enter too.
    pub(crate) fn open_file_location(&mut self, path: &Path) {
        self.spawn_argv_activated(&apps::open_location_argv(path));
    }

    fn file_icon_name(&self, path: &Path) -> String {
        self.with_mime(|db| crate::search::file_icon_name(path, db, &self.icons))
    }

    /// `f` over the desktop's MIME database, an empty one without a desktop.
    fn with_mime<R>(&self, f: impl FnOnce(&apps::MimeDb) -> R) -> R {
        match self.desktop.as_ref() {
            Some(v) => f(&v.mime),
            None => f(&apps::MimeDb::empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[MenuItem]) -> Vec<(String, bool)> {
        items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Entry { label, enabled, .. } => Some((label.clone(), *enabled)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_found_file_gets_the_icon_menu_with_rename_greyed() {
        let (items, commands) = file_menu(false, Some("text/plain"));
        let (icon, icon_commands) = icon_menu(IconMenu {
            print: reprisefs::props::verbs(Some("text/plain"), reprisefs::props::has_lp()).0,
            edit: true,
            ..IconMenu::default()
        });
        let without_location: Vec<_> = commands
            .iter()
            .filter(|c| **c != Some(DesktopCommand::OpenFileLocation))
            .cloned()
            .collect();
        assert_eq!(without_location, icon_commands, "the icon menu's rows");
        let greyed: Vec<String> = labels(&items)
            .into_iter()
            .filter(|(_, on)| !on)
            .map(|(l, _)| l)
            .collect();
        assert_eq!(greyed, ["Rename"]);
        assert_eq!(labels(&icon).len() + 1, labels(&items).len());
        assert!(commands.contains(&Some(DesktopCommand::OpenWith)));
    }

    #[test]
    fn open_file_location_follows_the_open_group_of_a_found_file_or_folder() {
        let at = |is_dir, mime| {
            let (items, commands) = file_menu(is_dir, mime);
            let i = commands
                .iter()
                .position(|c| *c == Some(DesktopCommand::OpenFileLocation))
                .expect("an Open file location row");
            assert!(
                matches!(&items[i], MenuItem::Entry { label, enabled: true, .. } if label == "Open file location")
            );
            assert!(matches!(items[i + 1], MenuItem::Separator));
            commands[i - 1].clone()
        };
        assert_eq!(
            at(false, Some("text/plain")),
            Some(DesktopCommand::OpenWith)
        );
        assert_eq!(at(true, None), Some(DesktopCommand::Explore));
        let (_, desktop) = icon_menu(IconMenu::default());
        assert!(
            !desktop.contains(&Some(DesktopCommand::OpenFileLocation)),
            "a desktop icon's menu has no Open file location"
        );
    }

    #[test]
    fn a_found_folder_gets_explore_and_paste() {
        let (items, commands) = file_menu(true, None);
        assert!(commands.contains(&Some(DesktopCommand::Explore)));
        assert!(commands.contains(&Some(DesktopCommand::PasteInto)));
        assert!(!commands.contains(&Some(DesktopCommand::OpenWith)));
        let rename = commands
            .iter()
            .position(|c| *c == Some(DesktopCommand::Rename))
            .unwrap();
        assert!(matches!(
            items[rename],
            MenuItem::Entry { enabled: false, .. }
        ));
    }
}
