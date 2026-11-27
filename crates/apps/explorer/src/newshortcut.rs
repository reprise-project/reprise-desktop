//! New > Shortcut's wizard over the folder window, or alone for the desktop
//! (`--new-shortcut DIR`): the kit's `newshortcut` pages, Browse... in the file dialog child
//! (`PROGRAM_BROWSE`, as Open With's), and the launcher written as a New's item.

use reprisekit::tr;
use std::path::PathBuf;

use repriseapp::filedlg::Driven;
use repriseapp::iconjobs::IconWaits;
use repriseapp::{Ctx, Event, Reply};
use reprisefs::apps::AppEntry;
use reprisefs::deadline::Gate;
use reprisefs::newshortcut::{self, Target};
use reprisekit::controls::filedlg::{DlgMode, Fields};
use reprisekit::controls::form;
use reprisekit::controls::newshortcut::{NewShortcut, Outcome, Picked};
use reprisekit::icontheme::IconCache;
use reprisekit::metrics::ControlMetrics;

use crate::Explorer;
use crate::acting::Action;
use crate::dialogs::{Dialog, ProgramBrowse};

/// The wizard that is up: its pages, the folder it writes into and the programs its list shows.
pub struct ShortcutWizard {
    pub dir: PathBuf,
    pub wizard: NewShortcut,
    pub apps: Vec<AppEntry>,
    /// The target Next read, which Finish writes.
    pub target: Option<Target>,
    pub browse: Option<ProgramBrowse>,
    /// The list's icons still decoding.
    pub waits: IconWaits,
}

impl ShortcutWizard {
    /// The list's icons at `cm`'s size, stand-ins while they decode off the loop.
    pub fn wear_icons(&mut self, icons: &mut IconCache, cm: &ControlMetrics) {
        self.waits = IconWaits::default();
        let px = reprisekit::controls::listbox::icon_px(cm);
        let shown = (self.apps.iter())
            .map(|a| self.waits.first(icons, &[a.icon.as_deref()?], px))
            .collect();
        self.wizard.set_icons(shown);
    }

    /// Whether an icon the list waited on landed, worn if so.
    pub fn land_icons(&mut self, icons: &mut IconCache, cm: &ControlMetrics) -> bool {
        let landed = self.waits.landed(icons);
        if landed {
            self.wear_icons(icons, cm);
        }
        landed
    }
}

impl Explorer {
    /// New > Shortcut in the place on view.
    pub fn new_shortcut(&mut self, ctx: &mut Ctx) -> Reply {
        match self.place.path().filter(|_| !self.place.read_only()) {
            Some(dir) => self.new_shortcut_in(dir, ctx),
            None => Reply::none(),
        }
    }

    pub fn new_shortcut_in(&mut self, dir: PathBuf, ctx: &mut Ctx) -> Reply {
        let mut apps: Vec<AppEntry> = (self.app_index.iter())
            .filter(|a| a.shown)
            .cloned()
            .collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        let labels = apps.iter().map(|a| a.name.clone()).collect();
        let mut w = ShortcutWizard {
            dir,
            wizard: NewShortcut::new(labels, &self.forms),
            apps,
            target: None,
            browse: None,
            waits: IconWaits::default(),
        };
        w.wear_icons(&mut ctx.icons, &self.forms.controls());
        self.raise(Dialog::NewShortcut(Box::new(w)))
    }

    /// What the wizard's form answered.
    pub(crate) fn new_shortcut_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::NewShortcut(w)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        match w.wizard.event(out, &self.forms) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => self.dismiss_up(ctx),
            Outcome::Browse => {
                let dir = w.dir.clone();
                let mut host = self.dialog_host();
                host.icons.prime(&mut ctx.icons);
                let fields = Fields {
                    title: tr!("new-shortcut-answered-browse"),
                    accept_label: tr!("new-shortcut-answered-open"),
                    filters: vec![tr!("new-shortcut-answered-all-files")],
                    mode: DlgMode::Open,
                    multiple: false,
                };
                let dlg = host.open(&dir, fields, self.forms.text);
                if let Some(Dialog::NewShortcut(w)) = self.dialogs.up_mut() {
                    w.browse = Some(ProgramBrowse { host, dlg });
                }
                Reply::redraw_all()
            }
            Outcome::Next => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                let read = match w.wizard.picked() {
                    Some(Picked::Program(i)) => match w.apps.get(i) {
                        Some(app) => Ok(Target::App(Box::new(app.clone()))),
                        None => return Reply::none(),
                    },
                    Some(Picked::Typed(text)) => newshortcut::read_typed(text, home.as_deref()),
                    None => return Reply::none(),
                };
                match read {
                    Ok(target) => {
                        let offered = target.default_name();
                        w.wizard.named(&offered, &self.forms);
                        let web = match &target {
                            Target::Web(url) => Some(url.clone()),
                            _ => None,
                        };
                        w.target = Some(target);
                        if let Some(url) = web {
                            self.fetch_title(url, offered, ctx);
                        }
                        Reply::redraw()
                    }
                    Err(why) => {
                        w.wizard.refuse(&why, &self.forms);
                        Reply::redraw()
                    }
                }
            }
            Outcome::Finish => {
                let Some(Dialog::NewShortcut(w)) = self.take_up() else {
                    return Reply::none();
                };
                self.write_shortcut(*w, ctx)
            }
        }
    }

    /// The page title of web address `url` read on a listing thread, which then names the shortcut
    /// in place of `offered` unless the wizard moved on or the name was edited.
    fn fetch_title(&mut self, url: String, offered: String, ctx: &Ctx) {
        self.spawn_read_job(ctx, move || {
            let title = newshortcut::page_title(&url);
            Box::new(move |ex: &mut Explorer, _: &mut Ctx| {
                let Some(title) = title else {
                    return Reply::none();
                };
                if let Some(Dialog::NewShortcut(w)) = ex.dialogs.up_mut()
                    && w.target == Some(Target::Web(url))
                    && w.wizard.offer_name(&offered, &title)
                {
                    return Reply::redraw_all();
                }
                Reply::none()
            })
        });
    }

    /// The launcher Finish asked for, written on a listing thread, then selected with its rename
    /// begun and recorded for Undo as a New.
    fn write_shortcut(&mut self, w: ShortcutWizard, ctx: &mut Ctx) -> Reply {
        let name = w.wizard.name().to_string();
        let browser = match &w.target {
            Some(Target::Web(url)) if newshortcut::is_http(url) => {
                newshortcut::browser_icon(&self.app_index)
            }
            _ => None,
        };
        let launcher = |t: &Target| t.launcher(&name, browser.as_deref());
        let Some(text) = w.target.as_ref().and_then(launcher) else {
            return self.message_box(
                &newshortcut_title(),
                tr!("new-shortcut-write-shortcut-a-shortcut-cannot-hold-that-name"),
                ctx,
            );
        };
        let dir = w.dir;
        let work = move |gate: &Gate| {
            if !gate.change(&dir) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let path = newshortcut::write(&dir, &name, &text)?;
            Ok((reprisefs::undo::file_id(&path), path))
        };
        self.act(
            ctx,
            Action::changes(tr!("new-shortcut-write-shortcut-new-shortcut")),
            work,
            move |ex, made, ctx| {
                ex.item_made(&tr!("new-shortcut-write-shortcut-the-shortcut"), made, ctx)
            },
        )
    }

    /// The wizard's Browse... picker: Open puts the picked file in the location field.
    pub(crate) fn shortcut_browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::NewShortcut(w)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        let Some(b) = w.browse.as_mut() else {
            return Reply::none();
        };
        let driven =
            b.host
                .drive_keeping(&mut b.dlg, &ev, &self.dialog_text, &self.forms, ctx, |_| {
                    |_: &reprisefs::listing::Item| true
                });
        match driven {
            Driven::Reply(r) => *r,
            Driven::Cancelled => {
                w.browse = None;
                Reply::redraw_all()
            }
            Driven::Picked(path) => {
                w.browse = None;
                w.wizard.set_location(&path.to_string_lossy());
                Reply::redraw_all()
            }
        }
    }
}

fn newshortcut_title() -> String {
    reprisekit::controls::newshortcut::title().to_string()
}

/// The file dialog a host keeps for Browse..., Open With's or the wizard's.
pub(crate) fn picker_of(d: &Dialog) -> Option<&ProgramBrowse> {
    match d {
        Dialog::ChooseProgram(cp) => cp.browse.as_ref(),
        Dialog::NewShortcut(w) => w.browse.as_ref(),
        _ => None,
    }
}

pub(crate) fn picker_of_mut(d: &mut Dialog) -> Option<&mut ProgramBrowse> {
    match d {
        Dialog::ChooseProgram(cp) => cp.browse.as_mut(),
        Dialog::NewShortcut(w) => w.browse.as_mut(),
        _ => None,
    }
}
