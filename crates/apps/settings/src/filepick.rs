//! The file dialog behind Startup Add..., the Background Browse... buttons and Browse for a program.

use crate::{SettingsApp, autostart, wallpapers};
use repriseapp::filedlg::{Resolved, Step, type_keep};
use reprisekit::controls::filedlg::{DlgMode, Event as FileEvent, Fields, FileDlg};
use std::path::PathBuf;

use repriseapp::{Ctx, Event, Reply};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    Autostart,
    Wallpaper,
    SlideFolder,
    /// The Open With chooser's Browse...: a program file.
    Program,
}

/// The file dialog's listing filter for `pick`.
fn pick_keep(
    pick: Pick,
    d: &FileDlg,
) -> impl Fn(&reprisefs::listing::Item) -> bool + Clone + Send + 'static {
    let typed = type_keep(d);
    let programs = repriseapp::openwith::programs_keep(d.filter());
    move |i| match pick {
        Pick::Program => programs(i),
        _ => typed(i),
    }
}

impl SettingsApp {
    /// Open the file dialog for `pick`: Add... at home, Browse... at Pictures or, for a program, its folder.
    pub(crate) fn open_add(&mut self, pick: Pick) {
        self.add_for = pick;
        let (dir, fields) = match pick {
            Pick::Autostart => (
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/")),
                Fields {
                    title: "Add".into(),
                    accept_label: "Open".into(),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![
                        "Desktop Entries (*.desktop)".into(),
                        "All Files (*.*)".into(),
                    ],
                },
            ),
            Pick::Wallpaper => (
                wallpapers::start_dir(),
                Fields {
                    title: "Browse".into(),
                    accept_label: "Open".into(),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![wallpapers::FILTER.into(), "All Files (*.*)".into()],
                },
            ),
            Pick::SlideFolder => (
                wallpapers::expand(&self.want.slideshow)
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(wallpapers::start_dir),
                Fields {
                    title: "Browse for Folder".into(),
                    accept_label: "OK".into(),
                    mode: DlgMode::Folder,
                    multiple: false,
                    filters: Vec::new(),
                },
            ),
            Pick::Program => (
                PathBuf::from(repriseapp::openwith::PROGRAMS_DIR),
                repriseapp::openwith::browse_fields(),
            ),
        };
        // The sheet's last draw carries the text size the dialog grows by.
        let u = self.sheet.open().units();
        self.add = Some(self.add_host.open(&dir, fields, u));
    }

    fn close_add(&mut self) -> Reply {
        self.add = None;
        Reply::redraw_all()
    }

    pub(crate) fn add_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let pick = self.add_for;
        let Some(d) = &mut self.add else {
            return Reply::none();
        };
        let host = &mut self.add_host;
        let keep = pick_keep(pick, d);
        host.ensure_listed(d, &mut ctx.icons, &keep);
        match host.event(
            d,
            &ev,
            &ctx.text,
            &ctx.forms(),
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed | Step::Out(FileEvent::Cancel) => self.close_add(),
            Step::None => Reply::none(),
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::Out(FileEvent::NeedListing(id)) => {
                host.list(d, id, &mut ctx.icons, &keep);
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                let keep = pick_keep(pick, d);
                if let Some(id) = host.folder {
                    host.list(d, id, &mut ctx.icons, &keep);
                }
                Reply::redraw()
            }
            Step::Out(FileEvent::Accept(names)) => {
                let Some(name) = names.into_iter().next() else {
                    return Reply::none();
                };
                match host.resolve(&name) {
                    Ok(Resolved::Folder(_, path)) if pick == Pick::SlideFolder => {
                        self.slide_folder(path);
                        self.sync_apply();
                        self.close_add()
                    }
                    Ok(Resolved::Folder(id, _)) => {
                        host.list(d, id, &mut ctx.icons, &keep);
                        d.clear_file_name();
                        Reply::redraw()
                    }
                    Ok(Resolved::File(path)) => {
                        match pick {
                            Pick::Autostart => {
                                if let Err(e) = autostart::add(&self.user_autostart(), &path) {
                                    log::warn!("add {}: {e}", path.display());
                                    self.fail(&format!("Cannot add '{}': {e}.", path.display()));
                                }
                                self.rescan();
                            }
                            Pick::Wallpaper => match wallpapers::refusal(&path) {
                                Some(why) => self.fail(&why),
                                None => self.browsed(path),
                            },
                            Pick::Program if !path.is_file() => {
                                log::warn!("{} is not a file", path.display());
                                return Reply::none();
                            }
                            Pick::Program => self.browsed_program(&path),
                            Pick::SlideFolder => {}
                        }
                        self.sync_apply();
                        self.close_add()
                    }
                    Err(msg) => {
                        log::warn!("add {name}: {msg}");
                        self.fail(&msg);
                        self.close_add()
                    }
                }
            }
            Step::Out(_) => Reply::redraw(),
        }
    }
}
