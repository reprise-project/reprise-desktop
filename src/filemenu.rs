//! The context menu of a file or folder found by search: the desktop's icon menu
//! (`desktopview::icon_menu`), acting on the path as its icon would, with Open file
//! location after its Open group. Rename is greyed, as a search row has no field to
//! rename in.

use std::path::{Path, PathBuf};

use reprisefs::apps;
use reprisekit::controls::menu::MenuItem;

use crate::compositor::State;
use crate::desktop::{self, Target};
use crate::desktopview::{DesktopCommand, IconMenu, entry, icon_menu};
use crate::dragview::{DragSource, DropOp, DropTarget};

/// The menu of a folder (`m.is_dir`) or of a file of type `mime`, with `m`'s
/// launcher rows; the file verbs come from `mime`.
pub fn file_menu(m: IconMenu, mime: Option<&str>) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    let (print, edit) = if m.is_dir {
        (false, false)
    } else {
        reprisefs::props::verbs(mime, reprisefs::props::has_lp())
    };
    let (mut items, mut commands) = icon_menu(IconMenu { print, edit, ..m });
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

/// The launcher rows of a found path from its `facts`: Allow Launching for an untrusted
/// application `.desktop` file, Run as... for a trusted one.
fn found_menu(facts: &desktop::FileFacts) -> IconMenu {
    if facts.is_dir {
        return IconMenu {
            is_dir: true,
            ..IconMenu::default()
        };
    }
    IconMenu {
        allow_launching: facts.allow_launching,
        launcher: matches!(facts.target, Target::Launch(_)),
        ..IconMenu::default()
    }
}

impl State {
    /// The menu of a found `path`, built from the `facts` its search read.
    pub(crate) fn file_menu_rows(
        &self,
        path: &Path,
        facts: &desktop::FileFacts,
    ) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
        let mime = (!facts.is_dir)
            .then(|| path.file_name())
            .flatten()
            .and_then(|n| self.with_mime(|db| db.mime_of_name(&n.to_string_lossy())));
        file_menu(found_menu(facts), mime.as_deref())
    }

    /// The program run when a found `path` is opened, from its `facts`: its icon's on the desktop.
    pub(crate) fn found_open_argv(
        &self,
        path: &Path,
        facts: &desktop::FileFacts,
    ) -> Vec<std::ffi::OsString> {
        match &facts.target {
            Target::Open(dir) if facts.is_dir => apps::open_folder_argv(dir),
            Target::Open(file) => {
                let mime = (path.file_name())
                    .and_then(|n| self.with_mime(|db| db.mime_of_name(&n.to_string_lossy())));
                let default = mime
                    .as_deref()
                    .and_then(|m| reprisefs::props::default_app_entry(m, &self.app_index));
                reprisefs::props::open_file_argv(file, default, &self.config.terminal)
            }
            target => self.with_mime(|db| {
                desktop::open_argv(target, db, &self.app_index, &self.config.terminal)
            }),
        }
    }

    /// Open a found `path` as its `facts` say: a program runs, anything else opens with
    /// [`Self::found_open_argv`].
    pub(crate) fn open_found(&mut self, path: &Path, facts: &desktop::FileFacts) {
        match &facts.target {
            Target::Run(file) => self.run_program(file),
            _ => {
                let argv = self.found_open_argv(path, facts);
                self.spawn_argv_activated(&argv);
            }
        }
    }

    /// Row `cmd` of `file_menu` picked for `path`, whose `facts` built the menu.
    pub(crate) fn file_command(
        &mut self,
        cmd: DesktopCommand,
        path: PathBuf,
        facts: &desktop::FileFacts,
    ) {
        use DesktopCommand as Dc;
        let source = || vec![DragSource::file(path.clone())];
        match cmd {
            Dc::Open | Dc::Explore => self.open_found(&path, facts),
            Dc::AllowLaunching => {
                if facts.allow_launching {
                    self.allow_launching(&[path]);
                }
            }
            Dc::OpenFileLocation => self.open_file_location(&path),
            Dc::Print | Dc::Edit => self.print_or_edit(vec![path], cmd == Dc::Print),
            Dc::OpenWithApp(app) => self.open_files_with(vec![path], &app),
            Dc::ChooseProgram => {
                let d = self.choose_program(&path, false);
                self.open_dialog(d);
            }
            Dc::SendToDest(row) => {
                // The names beside the file, which a Send To to the same folder numbers past.
                let dir = path.parent().map(Path::to_path_buf);
                self.spawn_fs_job(move || {
                    let existing: Vec<String> = dir
                        .and_then(|d| std::fs::read_dir(d).ok())
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.ok()?.file_name().into_string().ok())
                        .collect();
                    Box::new(move |state: &mut State| {
                        let source = vec![DragSource::file(path)];
                        state.send_sources_to(row, &source, &existing, None);
                    })
                });
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
                let at = (p.x as i32, p.y as i32);
                let apps = self.app_index.clone();
                self.spawn_fs_job(move || {
                    let info = reprisefs::props::glance(&path, &name, &apps);
                    Box::new(move |state: &mut State| match info {
                        Ok(info) => {
                            let icon = state.file_icon_name(&path, info.is_dir);
                            state.open_properties(&info, &icon, at);
                        }
                        Err(err) => log::warn!("properties: {}: {err}", path.display()),
                    })
                });
            }
            _ => {}
        }
    }

    /// Explorer on `path`'s folder with `path` selected; the launcher's
    /// Ctrl+Enter too.
    pub(crate) fn open_file_location(&mut self, path: &Path) {
        self.spawn_argv_activated(&apps::open_location_argv(path));
    }

    fn file_icon_name(&self, path: &Path, is_dir: bool) -> String {
        self.with_mime(|db| crate::search::file_icon_name(path, is_dir, db, &self.icons))
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

    /// A found path's menu comes from the facts its search read, with no stat: a path that is
    /// gone keeps the folder or launcher rows it was found with.
    #[test]
    fn a_found_paths_menu_comes_from_its_facts() {
        let gone = Path::new("/nonexistent/reprise-found/tool.desktop");
        let rows = |facts: &desktop::FileFacts| file_menu(found_menu(facts), None).1;
        let folder = desktop::FileFacts {
            is_dir: true,
            ..desktop::FileFacts::plain(gone)
        };
        assert!(rows(&folder).contains(&Some(DesktopCommand::Explore)));
        let untrusted = desktop::FileFacts {
            allow_launching: true,
            ..desktop::FileFacts::plain(gone)
        };
        assert!(rows(&untrusted).contains(&Some(DesktopCommand::AllowLaunching)));
        assert!(
            !rows(&desktop::FileFacts::plain(gone)).contains(&Some(DesktopCommand::AllowLaunching))
        );
    }

    #[test]
    fn a_found_file_gets_the_icon_menu_with_rename_greyed() {
        let (items, commands) = file_menu(IconMenu::default(), Some("text/plain"));
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
            let (items, commands) = file_menu(
                IconMenu {
                    is_dir,
                    ..IconMenu::default()
                },
                mime,
            );
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
        let (items, commands) = file_menu(
            IconMenu {
                is_dir: true,
                ..IconMenu::default()
            },
            None,
        );
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

    #[test]
    fn a_found_untrusted_desktop_file_gets_allow_launching_until_it_is_a_launcher() {
        let dir = std::env::temp_dir().join(format!("reprise-filemenu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tool.desktop");
        std::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Tool\nExec=true\n",
        )
        .unwrap();
        let rows = |path: &Path| {
            let (items, commands) = file_menu(found_menu(&desktop::file_facts(path)), None);
            (labels(&items), commands)
        };
        let (items, commands) = rows(&path);
        let allow = commands
            .iter()
            .position(|c| *c == Some(DesktopCommand::AllowLaunching))
            .expect("an Allow Launching row");
        assert_eq!(commands[allow - 1], Some(DesktopCommand::OpenWith));
        assert!(!items.iter().any(|(l, _)| l == "Run as..."));

        apps::allow_launching(&path).unwrap();
        let (items, commands) = rows(&path);
        assert!(!commands.contains(&Some(DesktopCommand::AllowLaunching)));
        assert!(
            items.contains(&("Run as...".to_string(), false)),
            "a greyed Run as..."
        );

        let text = dir.join("notes.txt");
        std::fs::write(&text, "x").unwrap();
        assert!(
            !rows(&text)
                .1
                .contains(&Some(DesktopCommand::AllowLaunching))
        );
        let link = dir.join("link.desktop");
        std::fs::write(
            &link,
            "[Desktop Entry]\nType=Link\nName=L\nURL=https://example.org\n",
        )
        .unwrap();
        assert!(
            !rows(&link)
                .1
                .contains(&Some(DesktopCommand::AllowLaunching))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
