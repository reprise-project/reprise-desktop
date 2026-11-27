//! The file dialog behind Startup Add..., the Background and Login Manager Browse... buttons,
//! Browse for a program and the Backup page's Export... and Import....

use crate::{SettingsApp, autostart, wallpapers};
use repriseapp::filedlg::{Driven, type_keep};
use reprisekit::controls::filedlg::{DlgMode, Fields, FileDlg};
use std::path::PathBuf;

use repriseapp::{Ctx, Event, Reply};
use reprisekit::tr;

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

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The file dialog's listing filter for `pick`.
fn pick_keep(
    pick: Pick,
    d: &FileDlg,
) -> impl Fn(&reprisefs::listing::Item) -> bool + Clone + Send + 'static + use<> {
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
                    title: tr!("filepick-add"),
                    accept_label: tr!("filepick-open"),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![
                        tr!("filepick-desktop-entries-desktop"),
                        tr!("filepick-all-files"),
                    ],
                },
            ),
            Pick::Wallpaper | Pick::LoginBackground => (
                wallpapers::start_dir(),
                true,
                Fields {
                    title: tr!("filepick-browse"),
                    accept_label: tr!("filepick-open"),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![wallpapers::filter(), tr!("filepick-all-files")],
                },
            ),
            Pick::SlideFolder => (
                slides.clone().unwrap_or_else(wallpapers::start_dir),
                slides.is_none(),
                Fields {
                    title: tr!("filepick-browse-for-folder"),
                    accept_label: tr!("filepick-ok"),
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
                        tr!("filepick-export")
                    } else {
                        tr!("filepick-import")
                    },
                    accept_label: if pick == Pick::Export {
                        tr!("filepick-save")
                    } else {
                        tr!("filepick-open")
                    },
                    mode: if pick == Pick::Export {
                        DlgMode::Save
                    } else {
                        DlgMode::Open
                    },
                    multiple: false,
                    filters: vec![
                        tr!("filepick-reprise-settings-toml"),
                        tr!("filepick-all-files"),
                    ],
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
        let (text, forms) = (ctx.text.clone(), ctx.forms());
        let driven =
            (self.add_host).drive_keeping(d, &ev, &text, &forms, ctx, |d| pick_keep(pick, d));
        let path = match driven {
            Driven::Reply(r) => return *r,
            Driven::Cancelled => return self.close_add(),
            Driven::Picked(path) => path,
        };
        match pick {
            Pick::Autostart => {
                if let Err(e) = autostart::add(&self.user_autostart(), &path) {
                    log::warn!("add {}: {e}", path.display());
                    self.fail(&tr!(
                        "filepick-cannot-add",
                        path = path.display().to_string(),
                        why = e.to_string()
                    ));
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
            Pick::SlideFolder => self.slide_folder(path),
            Pick::Program if !path.is_file() => {
                log::warn!("{} is not a file", path.display());
                return Reply::none();
            }
            Pick::Program => self.browsed_program(&path),
            Pick::Export | Pick::Import => self.backup_picked(pick, path),
        }
        self.sync_apply();
        self.close_add()
    }
}
