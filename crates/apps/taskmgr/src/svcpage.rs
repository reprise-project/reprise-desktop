//! The Services page on [`TaskMgr`]: its list, the row menu, Start, Stop (asked first),
//! Restart, Start at Login and Open Log, and what the [`services`] threads post back.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};

use repriseapp::detailslist::List;
use repriseapp::oneshot::MsgBox;
use repriseapp::{Ctx, Event, Reply};
use reprisebus::services::{Failed, Manager, Reread, Verb};
use reprisekit::controls::listview::{self, Column};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::msgbox::Fields;
use reprisekit::tr;

use crate::servicelog::{self, Log};
use crate::services::{self, Job, Msg, Post, Read, Service};
use crate::{Act, Ask, Page, TaskMgr};

pub struct Svc {
    pub list: List,
    /// Every row read, sorted; the list shows those at `shown`.
    pub rows: Vec<Service>,
    pub shown: Vec<usize>,
    /// The filter field's text and Show only failed ([`services::shows`]).
    pub filter: String,
    pub failed_only: bool,
    /// The column sorted by, and whether ascending.
    pub sort: (usize, bool),
    /// Show system services.
    pub system: bool,
    /// polkit may let this user start, stop and restart system services.
    pub manage_system: bool,
    /// The services a job is under way on.
    pub busy: HashSet<(Manager, String)>,
    pub reader: Option<Sender<Read>>,
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    /// A job off the event loop ([`services::run`]).
    pub run_job: fn(Job, Post),
    /// A log read off the event loop ([`services::read_log`]).
    pub read_log: fn(Manager, String, Post),
    /// Follows the user manager's changes ([`services::follow`]), from the page's first showing.
    pub follow: fn(Post),
    /// Where the threads post, once the window started.
    pub post: Option<Post>,
    pub following: bool,
    pub log: Option<Log>,
}

impl Svc {
    pub fn new() -> Svc {
        let (tx, rx) = channel();
        let mut list = List::new(vec![
            Column::new(&tr!("col-svc-name"), 150),
            Column::new(&tr!("col-svc-description"), 190),
            Column::new(&tr!("column-status"), 70),
            Column::new(&tr!("col-svc-startup"), 75),
            Column::new(&tr!("col-svc-group"), 60),
        ]);
        list.view.frame = true;
        list.view.sort = Some((0, true));
        Svc {
            list,
            rows: Vec::new(),
            shown: Vec::new(),
            filter: String::new(),
            failed_only: false,
            sort: (0, true),
            system: false,
            manage_system: true,
            busy: HashSet::new(),
            reader: None,
            tx,
            rx,
            run_job: services::run,
            read_log: services::read_log,
            follow: services::follow,
            post: None,
            following: false,
            log: None,
        }
    }

    fn cells(s: &Service) -> Vec<String> {
        vec![
            s.shown_name(),
            s.description.clone(),
            s.status.text(),
            s.startup.text(),
            s.group(),
        ]
    }
}

impl TaskMgr {
    fn post(&self, ctx: &Ctx) -> Post {
        Post {
            tx: self.svc.tx.clone(),
            wake: ctx.wake.clone(),
        }
    }

    /// Asks for the rows again, the unit files too when `files`.
    pub(crate) fn read_services(&self, files: bool) {
        if let Some(r) = &self.svc.reader {
            let _ = r.send(Read {
                system: self.svc.system,
                files,
            });
        }
    }

    pub(crate) fn selected_service(&self) -> Option<&Service> {
        let at = self.svc.list.selected().and_then(|i| self.svc.shown.get(i));
        at.and_then(|&i| self.svc.rows.get(i))
    }

    /// Whether `verb` may run on `s` now: its state takes it, no job is under way on it, and for a
    /// system service polkit may allow it.
    pub(crate) fn service_may(&self, s: &Service, verb: Verb) -> bool {
        let busy = (self.svc.busy).contains(&(s.manager, s.name.clone()));
        let allowed = s.manager == Manager::User || self.svc.manage_system;
        s.takes(verb) && !busy && allowed
    }

    pub(crate) fn take_services(&mut self, rows: Vec<Service>, manage_system: bool) {
        self.svc.manage_system = manage_system;
        let keep = self.selected_key();
        self.svc.rows = rows;
        self.show_services(keep);
    }

    fn selected_key(&self) -> Option<(Manager, String)> {
        (self.selected_service()).map(|s| (s.manager, s.name.clone()))
    }

    /// Sorts the rows and puts those the filter lets through in the list, `keep` selected.
    fn show_services(&mut self, keep: Option<(Manager, String)>) {
        let (col, ascending) = self.svc.sort;
        let text = |s: &Service| Svc::cells(s).swap_remove(col).to_lowercase();
        let rows = &mut self.svc.rows;
        if ascending {
            rows.sort_by_cached_key(|s| (text(s), s.name.clone(), s.manager));
        } else {
            rows.sort_by_cached_key(|s| (Reverse(text(s)), s.name.clone(), s.manager));
        }
        let (filter, failed_only) = (self.svc.filter.as_str(), self.svc.failed_only);
        self.svc.shown = (0..rows.len())
            .filter(|&i| services::shows(&rows[i], filter, failed_only))
            .collect();
        let shown = || self.svc.shown.iter().map(|&i| &self.svc.rows[i]);
        self.svc.list.view.rows = shown()
            .map(|s| listview::Row {
                icon: None,
                cells: Svc::cells(s),
            })
            .collect();
        let at = keep.and_then(|(m, n)| shown().position(|s| s.key() == (m, &n)));
        self.svc.list.view.selection.set_only(at);
        let n = self.svc.shown.len();
        self.svc.list.view.top = self.svc.list.view.top.min(n.saturating_sub(1));
    }

    pub(crate) fn sort_services(&mut self, col: usize) {
        let ascending = match self.svc.sort {
            (old, asc) if old == col => !asc,
            _ => true,
        };
        self.svc.sort = (col, ascending);
        self.svc.list.view.sort = Some((col, ascending));
        let keep = self.selected_key();
        self.show_services(keep);
    }

    /// Shows the rows the filter field's `text` and Show only failed let through.
    pub(crate) fn filter_services(&mut self, text: &str, failed_only: bool) {
        self.svc.filter = text.to_string();
        self.svc.failed_only = failed_only;
        let keep = self.selected_key();
        self.show_services(keep);
    }

    pub(crate) fn set_system_services(&mut self, on: bool) {
        self.svc.system = on;
        if !on {
            let keep = self.selected_key();
            self.svc.rows.retain(|s| s.manager == Manager::User);
            self.show_services(keep);
        }
        self.read_services(true);
    }

    /// The selected service's menu: the jobs, Start at Login for a user service, and Open Log.
    pub(crate) fn service_menu(&self) -> Option<Vec<(MenuItem, Option<Act>)>> {
        let s = self.selected_service()?;
        let row = |label: String, verb: Verb| {
            let live = self.service_may(s, verb);
            (
                MenuItem::item(&label).enabled(live),
                live.then_some(Act::Service(verb)),
            )
        };
        let mut rows = vec![
            row(tr!("menu-start"), Verb::Start),
            row(tr!("menu-stop"), Verb::Stop),
            row(tr!("menu-restart"), Verb::Restart),
        ];
        if s.manager == Manager::User {
            let on = s.startup == services::Startup::Automatic;
            let verb = if on { Verb::Disable } else { Verb::Enable };
            let live = self.service_may(s, verb);
            rows.push((MenuItem::Separator, None));
            rows.push((
                MenuItem::item(&tr!("menu-start-at-login"))
                    .checked(on)
                    .enabled(live),
                live.then_some(Act::Service(verb)),
            ));
        }
        rows.push((MenuItem::Separator, None));
        rows.push((MenuItem::item(&tr!("menu-open-log")), Some(Act::OpenLog)));
        Some(rows)
    }

    pub(crate) fn open_service_menu(
        &mut self,
        x: i32,
        y: i32,
        ctx: &Ctx,
        by_keyboard: bool,
    ) -> Reply {
        let Some(rows) = self.service_menu() else {
            return Reply::none();
        };
        self.popup = repriseapp::ctxmenu::Popup::open(rows, x, y, ctx, by_keyboard);
        Reply::redraw()
    }

    /// `verb` on the selected service: Stop asks first, the rest run at once.
    pub(crate) fn service_verb(&mut self, verb: Verb, ctx: &mut Ctx) -> Reply {
        let Some(s) = self
            .selected_service()
            .filter(|s| self.service_may(s, verb))
        else {
            return Reply::none();
        };
        let job = Job {
            manager: s.manager,
            name: s.name.clone(),
            verb,
        };
        if verb != Verb::Stop {
            self.run_service_job(job, ctx);
            return Reply::redraw();
        }
        let b = MsgBox::new(
            tr!("warning-title"),
            Fields {
                text: tr!("stop-text", name = services::shown_name(&job.name)),
                icon: ctx.icons.get("dialog-warning", 32),
                buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning);
        self.asks.ask(b, Ask::StopService(job));
        Reply::redraw()
    }

    pub(crate) fn run_service_job(&mut self, job: Job, ctx: &Ctx) {
        self.svc.busy.insert((job.manager, job.name.clone()));
        (self.svc.run_job)(job, self.post(ctx));
    }

    /// Open Log on the selected service; the window shows one service at a time, and one read
    /// of its log is under way at a time.
    pub(crate) fn open_log(&mut self, ctx: &Ctx) -> Reply {
        let Some(s) = self.selected_service() else {
            return Reply::none();
        };
        let (manager, name) = (s.manager, s.name.clone());
        let same = |l: &Log| l.manager == manager && l.name == name;
        if self.svc.log.as_ref().is_some_and(|l| same(l) && l.reading) {
            return Reply::redraw_all();
        }
        let is_12h = reprisefs::locale::is_12h();
        self.svc.log = Some(Log::new(manager, &name, is_12h));
        (self.svc.read_log)(manager, name, self.post(ctx));
        Reply::redraw_all()
    }

    /// Takes what the threads posted: rows, ended jobs (telling a failure), log lines and the
    /// manager's changes, which read the rows again while the page shows.
    pub(crate) fn service_msgs(&mut self, ctx: &mut Ctx) {
        let got: Vec<Msg> = self.svc.rx.try_iter().collect();
        let mut reread = None;
        for m in got {
            match m {
                Msg::Reread(r) => reread = reread.max(Some(r == Reread::Files)),
                // A read asked before the box last changed would bring back or hide rows.
                Msg::Listed { system, .. } if system != self.svc.system => {}
                Msg::Listed {
                    rows,
                    manage_system,
                    ..
                } => self.take_services(rows, manage_system),
                Msg::Done(job, r) => {
                    self.svc.busy.remove(&(job.manager, job.name.clone()));
                    if let Err(failed) = r
                        && let Some(text) = failed_text(&job, failed)
                    {
                        let icon = ctx.icons.get("dialog-error", 32);
                        self.errors.show(&tr!("title"), text, icon);
                    }
                    self.read_services(true);
                }
                Msg::Log {
                    manager,
                    name,
                    read,
                } => {
                    if let Some(l) =
                        (self.svc.log.as_mut()).filter(|l| l.manager == manager && l.name == name)
                    {
                        l.show(read);
                    }
                }
            }
        }
        if let Some(files) = reread
            && self.page() == Page::Services
        {
            self.read_services(files);
        }
    }

    pub(crate) fn log_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(l) = self.svc.log.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => servicelog::Outcome::Close,
            Event::Blink => {
                l.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            ev => match repriseapp::keys::form_input(&mut l.form, &ev, ctx) {
                Some(e) => l.event(e),
                None => return Reply::none(),
            },
        };
        match out {
            servicelog::Outcome::None => Reply::none(),
            servicelog::Outcome::Redraw => Reply::redraw(),
            servicelog::Outcome::Refresh => {
                let (manager, name) = (l.manager, l.name.clone());
                (self.svc.read_log)(manager, name, self.post(ctx));
                Reply::redraw()
            }
            servicelog::Outcome::Close => {
                self.svc.log = None;
                Reply::redraw_all()
            }
        }
    }

    /// The Services page opened: its rows are read afresh, and the manager's changes followed
    /// from now on.
    pub(crate) fn services_shown(&mut self) {
        if self.page() != Page::Services {
            return;
        }
        if !self.svc.following
            && let Some(post) = self.svc.post.clone()
        {
            self.svc.following = true;
            (self.svc.follow)(post);
        }
        self.read_services(true);
    }
}

/// What a box says when `job` failed; nothing for a dismissed password prompt.
fn failed_text(job: &Job, failed: Failed) -> Option<String> {
    let name = services::shown_name(&job.name);
    let name = name.as_str();
    let why = match failed {
        Failed::Dismissed => return None,
        Failed::Unchanged if job.verb == Verb::Disable => {
            return Some(tr!("disable-unchanged", name = name));
        }
        Failed::Unchanged => return Some(tr!("enable-unchanged", name = name)),
        Failed::NotSent => tr!("job-not-sent"),
        Failed::Told(told) => reprisebus::told_text!(told),
    };
    Some(match job.verb {
        Verb::Start => tr!("start-failed", name = name, why = why),
        Verb::Stop => tr!("stop-failed", name = name, why = why),
        Verb::Restart => tr!("restart-failed", name = name, why = why),
        Verb::Enable => tr!("enable-failed", name = name, why = why),
        Verb::Disable => tr!("disable-failed", name = name, why = why),
    })
}
