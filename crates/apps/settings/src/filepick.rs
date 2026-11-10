//! The file dialog behind Startup Add..., the Background and Login Manager Browse... buttons,
//! Browse for a program and the Backup page's Export... and Import....

use crate::{SettingsApp, autostart, wallpapers};
use repriseapp::filedlg::{Resolved, Step, type_keep};
use reprisekit::controls::filedlg::{DlgMode, Event as FileEvent, Fields, FileDlg};
use std::path::PathBuf;

use repriseapp::{Ctx, Event, Reply};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    Autostart,
    Wallpaper,
    /// The Login Manager page's Browse...: the login screen's picture.
    LoginBackground,
    SlideFolder,
    /// The Open With chooser's Browse...: a program file.
    Program,
    /// The Backup page's Export... and Import...: a settings bundle.
    Export,
    Import,
}

const BUNDLE_FILTER: &str = "Reprise Settings (*.toml)";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
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
    /// Open the file dialog for `pick`: Add... and Browse... where Settings' last dialog was
    /// confirmed (else at home and Pictures), Browse for Folder at the slide show's folder and,
    /// for a program, its folder.
    pub(crate) fn open_add(&mut self, pick: Pick) {
        self.add_for = pick;
        let slides = wallpapers::expand(&self.want.slideshow).filter(|p| p.is_dir());
        // `last`: the dialog opens where Settings' last one was confirmed, `dir` without one.
        let (dir, last, fields) = match pick {
            Pick::Autostart => (
                home(),
                true,
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
            Pick::Wallpaper | Pick::LoginBackground => (
                wallpapers::start_dir(),
                true,
                Fields {
                    title: "Browse".into(),
                    accept_label: "Open".into(),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![wallpapers::FILTER.into(), "All Files (*.*)".into()],
                },
            ),
            Pick::SlideFolder => (
                slides.clone().unwrap_or_else(wallpapers::start_dir),
                slides.is_none(),
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
                false,
                repriseapp::openwith::browse_fields(),
            ),
            Pick::Export | Pick::Import => (
                home(),
                true,
                Fields {
                    title: if pick == Pick::Export {
                        "Export"
                    } else {
                        "Import"
                    }
                    .into(),
                    accept_label: if pick == Pick::Export { "Save" } else { "Open" }.into(),
                    mode: if pick == Pick::Export {
                        DlgMode::Save
                    } else {
                        DlgMode::Open
                    },
                    multiple: false,
                    filters: vec![BUNDLE_FILTER.into(), "All Files (*.*)".into()],
                },
            ),
        };
        // The sheet's last draw carries the text size the dialog grows by.
        let u = self.sheet.open().units();
        let mut d = match last {
            true => self.add_host.open_last(&dir, fields, u),
            false => self.add_host.open(&dir, fields, u),
        };
        if pick == Pick::Export {
            d.set_file_name(&crate::backup::suggested_name(crate::backup::today()));
        }
        self.add = Some(d);
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
                            Pick::LoginBackground => match wallpapers::refusal(&path) {
                                Some(why) => self.fail(&why),
                                None => self.login_picture(path),
                            },
                            Pick::Program if !path.is_file() => {
                                log::warn!("{} is not a file", path.display());
                                return Reply::none();
                            }
                            Pick::Program => self.browsed_program(&path),
                            Pick::Export | Pick::Import => self.backup_picked(pick, path),
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
