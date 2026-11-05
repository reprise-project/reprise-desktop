//! Task Manager: the running programs by app scope, every process with its CPU and memory, and
//! the CPU and memory history, sampled by [`worker`] while the window shows.

#[cfg(test)]
reprisebus::test_session!();

mod columns;
mod pages;
mod procfs;
mod sample;
mod scopes;
mod selectcols;
mod state;
mod switchto;
mod worker;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use repriseapp::detailslist::{List, Press};
use repriseapp::keys::AltArm;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, KeyInput, PointerInput, Reply, WinId};
use reprisebus::tray::Hung;
use reprisefs::apps::AppEntry;
use reprisekit::access::Tree;
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{self, Column};
use reprisekit::controls::menu::{MenuItem, MenuTree};
use reprisekit::controls::menubar::{Action, Bar, Key};
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::perfgraph::History;
use reprisekit::controls::tabstrip::TabStrip;
use reprisekit::pointer::Click;
use reprisekit::raster::Canvas;
use reprisekit::textedit::keysym;

use columns::Column as ProcColumn;
use sample::{ProcRow, Snapshot};
use state::State;

const ASK: WinId = WinId::named("ask");
const PICK: WinId = WinId::named("pick");

/// How many samples a history graph spans.
const HISTORY: usize = 100;
/// How long a program or process has to end after End Task or End Process before End Now is
/// offered.
const END_WAIT: Duration = Duration::from_secs(5);
/// How long the pointer rests on a process before its command line shows.
const TIP_DELAY: Duration = Duration::from_millis(600);

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speed {
    High,
    #[default]
    Normal,
    Low,
    Paused,
}

impl Speed {
    const ALL: [Speed; 4] = [Speed::High, Speed::Normal, Speed::Low, Speed::Paused];

    fn interval(self) -> Option<Duration> {
        match self {
            Speed::High => Some(Duration::from_millis(500)),
            Speed::Normal => Some(Duration::from_secs(1)),
            Speed::Low => Some(Duration::from_secs(4)),
            Speed::Paused => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Speed::High => "&High",
            Speed::Normal => "&Normal",
            Speed::Low => "&Low",
            Speed::Paused => "&Paused",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Applications,
    Processes,
    Performance,
}

impl Page {
    const ALL: [Page; 3] = [Page::Applications, Page::Processes, Page::Performance];
}

/// A program on the Applications page: one app scope.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRow {
    pub unit: String,
    pub app: String,
    pub name: String,
    pub icon: Option<String>,
    /// A window of it stopped answering pings.
    pub hung: bool,
}

impl AppRow {
    pub fn status(&self) -> &'static str {
        if self.hung {
            "Not Responding"
        } else {
            "Running"
        }
    }
}

/// What End Task or End Process is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Scope(String),
    /// A pid and the process's start time, so a reused pid is not taken for it.
    Process(i32, u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ending {
    target: Target,
    name: String,
    since: Instant,
    /// End Now was offered already.
    asked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ask {
    EndProcess { pid: i32, start: u64, name: String },
    EndNow(Target),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Quit,
    Refresh,
    Speed(Speed),
    PerCore(bool),
    SelectColumns,
}

struct Tip {
    pid: i32,
    at: (i32, i32),
    since: Instant,
    shown: bool,
}

pub struct TaskMgr {
    sampler: Option<sample::Sampler>,
    worker: Option<worker::Handle>,
    speed: Speed,
    suspended: bool,
    snap: Snapshot,
    tabs: TabStrip,
    apps: List,
    app_rows: Vec<AppRow>,
    /// The windows not answering pings, as the compositor reports them.
    hung: Vec<Hung>,
    hung_rx: Option<Receiver<Vec<Hung>>>,
    procs: List,
    proc_rows: Vec<ProcRow>,
    /// Process start times by pid, from the last snapshot.
    starts: HashMap<i32, u64>,
    /// The Processes columns shown, in order, and the widths a divider drag gave them.
    columns: Vec<ProcColumn>,
    widths: HashMap<ProcColumn, i32>,
    /// The last column is to fill what the others leave.
    refit: bool,
    sort: (ProcColumn, bool),
    all_users: bool,
    uid: u32,
    cpu: History,
    cores: Vec<History>,
    mem: History,
    swap: History,
    per_core: bool,
    bar: Bar,
    form: Form,
    /// The page and size `form` was built for.
    form_for: Option<(Page, (i32, i32))>,
    ask: Option<(Ask, MsgBox)>,
    /// Select Columns, while open.
    pick: Option<Form>,
    ending: Vec<Ending>,
    entries: Arc<Vec<AppEntry>>,
    icons: HashMap<String, Option<Rc<Canvas>>>,
    tip: Option<Tip>,
    alt_arm: AltArm,
    /// The dialog look and text of the last event, for the message box's window.
    forms: Option<reprisekit::theme::Theme>,
    text: Option<Rc<reprisekit::text::TextRenderer>>,
    /// Where the state is kept, and what it last held.
    state: Option<(PathBuf, State)>,
}

impl TaskMgr {
    pub fn new(sampler: Option<sample::Sampler>, entries: Vec<AppEntry>, uid: u32) -> TaskMgr {
        let mut t = TaskMgr {
            sampler,
            worker: None,
            speed: Speed::Normal,
            suspended: false,
            snap: Snapshot::default(),
            tabs: TabStrip::new(&["Applications", "Processes", "Performance"], 0),
            apps: List::new(vec![Column::new("Task", 260), Column::new("Status", 100)]),
            app_rows: Vec::new(),
            hung: Vec::new(),
            hung_rx: None,
            procs: List::new(Vec::new()),
            proc_rows: Vec::new(),
            starts: HashMap::new(),
            columns: columns::defaults(),
            widths: HashMap::new(),
            refit: false,
            sort: (ProcColumn::Cpu, false),
            all_users: false,
            uid,
            cpu: History::new(HISTORY),
            cores: Vec::new(),
            mem: History::new(HISTORY),
            swap: History::new(HISTORY),
            per_core: false,
            bar: Bar::new(Vec::<(&str, _)>::new()),
            form: Form::new(Vec::new()),
            form_for: None,
            ask: None,
            pick: None,
            ending: Vec::new(),
            entries: Arc::new(entries),
            icons: HashMap::new(),
            tip: None,
            alt_arm: AltArm::default(),
            forms: None,
            text: None,
            state: None,
        };
        t.show_columns();
        t.apps.view.frame = true;
        t.procs.view.frame = true;
        t.set_menus();
        t
    }

    fn page(&self) -> Page {
        Page::ALL[self.tabs.open.min(Page::ALL.len() - 1)]
    }

    fn set_menus(&mut self) {
        if self.bar.open.is_some() {
            return;
        }
        let mut file = MenuTree::new();
        file.push(MenuItem::item("E&xit Task Manager"));
        let mut speed = MenuTree::new();
        for s in Speed::ALL {
            speed.push(
                MenuItem::item(s.label())
                    .radio(true)
                    .checked(self.speed == s),
            );
        }
        let mut history = MenuTree::new();
        history.push(
            MenuItem::item("&One Graph, All CPUs")
                .radio(true)
                .checked(!self.per_core),
        );
        history.push(
            MenuItem::item("One Graph &Per CPU")
                .radio(true)
                .checked(self.per_core),
        );
        let mut view = MenuTree::new();
        view.push(MenuItem::item("&Refresh Now").shortcut(Some("F5".into())));
        view.push_sub(MenuItem::item("&Update Speed"), speed);
        view.push(MenuItem::Separator);
        view.push_sub(MenuItem::item("&CPU History"), history);
        view.push(MenuItem::item("&Select Columns...").enabled(self.page() == Page::Processes));
        let keep = (self.bar.active, self.bar.underline, self.bar.hot);
        self.bar = Bar::with_trees(vec![("&File", file), ("&View", view)]);
        (self.bar.active, self.bar.underline, self.bar.hot) = keep;
    }

    fn menu_act(title: usize, path: &[usize]) -> Option<Act> {
        Some(match (title, path) {
            (0, [0]) => Act::Quit,
            (1, [0]) => Act::Refresh,
            (1, [1, i]) => Act::Speed(*Speed::ALL.get(*i)?),
            (1, [3, i]) => Act::PerCore(*i == 1),
            (1, [4]) => Act::SelectColumns,
            _ => return None,
        })
    }

    fn act(&mut self, a: Act) -> Reply {
        match a {
            Act::Quit => return quit(),
            Act::Refresh => self.send(worker::Cmd::Now),
            Act::Speed(s) => {
                self.speed = s;
                self.follow_visibility();
            }
            Act::PerCore(on) => self.per_core = on,
            Act::SelectColumns => {
                self.pick = Some(selectcols::form(&self.columns));
                return Reply::redraw_all();
            }
        }
        self.set_menus();
        self.persist();
        Reply::redraw()
    }

    /// Takes the state kept at `path`: the View menu's picks and the Processes columns.
    pub fn restore(&mut self, path: PathBuf) {
        let s = State::load(&path);
        self.speed = s.speed;
        self.per_core = s.per_core;
        if let Some(keys) = &s.columns {
            self.columns = columns::shown(keys.iter().filter_map(|k| columns::by_key(k)));
        }
        self.widths = (s.widths.iter())
            .filter_map(|(k, &w)| Some((columns::by_key(k)?, w)))
            .collect();
        self.show_columns();
        self.set_menus();
        self.state = Some((path, self.kept()));
    }

    fn kept(&self) -> State {
        let key = |c: &ProcColumn| columns::spec(*c).key.to_string();
        State {
            speed: self.speed,
            per_core: self.per_core,
            columns: (self.columns != columns::defaults())
                .then(|| self.columns.iter().map(key).collect()),
            widths: self.widths.iter().map(|(c, &w)| (key(c), w)).collect(),
        }
    }

    /// Writes the state when it changed since it was last kept.
    fn persist(&mut self) {
        let now = self.kept();
        if let Some((path, saved)) = &mut self.state
            && *saved != now
        {
            now.save(path);
            *saved = now;
        }
    }

    /// The Processes list's columns from [`Self::columns`]; the sort falls back to Image Name when
    /// its column goes.
    fn show_columns(&mut self) {
        if !self.columns.contains(&self.sort.0) {
            self.sort = (ProcColumn::Name, true);
        }
        self.procs.view.columns = (self.columns.iter())
            .map(|&c| {
                let s = columns::spec(c);
                Column {
                    right: s.right,
                    ..Column::new(s.title, self.widths.get(&c).copied().unwrap_or(s.width))
                }
            })
            .collect();
        let at = self.columns.iter().position(|&c| c == self.sort.0);
        self.procs.view.sort = at.map(|i| (i, self.sort.1));
        self.send(worker::Cmd::Disk(self.columns.contains(&ProcColumn::Disk)));
        self.refresh_procs();
        self.refit = true;
    }

    fn set_columns(&mut self, picked: Vec<ProcColumn>) {
        self.columns = columns::shown(picked);
        self.show_columns();
        self.persist();
    }

    /// Keeps the widths a divider drag left on the Processes columns, all but the last, which
    /// fills the rest.
    fn note_widths(&mut self) {
        let n = self.columns.len().saturating_sub(1);
        for (c, col) in self.columns.iter().zip(&self.procs.view.columns).take(n) {
            if col.width == columns::spec(*c).width {
                self.widths.remove(c);
            } else {
                self.widths.insert(*c, col.width);
            }
        }
        self.persist();
    }

    /// Fits each list's last column to what the others leave of `ctx`'s list area.
    fn fit_lists(&mut self, ctx: &Ctx) {
        let r = pages::layout(self, ctx).list_area;
        self.apps.fit_last(r, ctx);
        // The process list nearly always scrolls: fill beside its bar, or a horizontal one appears.
        let last = self.procs.view.columns.len() - 1;
        (self.procs.view).fill_beside_bar(last, r, &ctx.theme.controls());
        for list in [&mut self.apps, &mut self.procs] {
            list.follow(r, ctx);
        }
        self.refit = false;
    }

    fn pick_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(f) = &mut self.pick else {
            return Reply::none();
        };
        if ev == Event::CloseRequest {
            self.pick = None;
            return Reply::redraw_all();
        }
        match repriseapp::keys::form_input(f, &ev, ctx) {
            Some(form::Event::Cancel | form::Event::Pressed(selectcols::CANCEL)) => {
                self.pick = None;
                Reply::redraw_all()
            }
            Some(form::Event::Default | form::Event::Pressed(selectcols::OK)) => {
                let picked = selectcols::picked(f);
                self.pick = None;
                self.set_columns(picked);
                Reply::redraw_all()
            }
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    fn send(&self, cmd: worker::Cmd) {
        if let Some(w) = &self.worker {
            let _ = w.tx.send(cmd);
        }
    }

    /// Samples at the chosen speed while the window shows, and not at all while it is suspended.
    fn follow_visibility(&self) {
        let every = if self.suspended {
            None
        } else {
            self.speed.interval()
        };
        self.send(worker::Cmd::Every(every));
    }

    fn icon(&mut self, name: &str, ctx: &mut Ctx) -> Option<Rc<Canvas>> {
        self.icons
            .entry(name.to_string())
            .or_insert_with(|| ctx.icons.get(name, 16).map(Rc::new))
            .clone()
    }

    /// Takes in a new snapshot: the rows, the histories and what End Task waits on.
    fn take(&mut self, s: Snapshot, ctx: &mut Ctx) {
        if let Some(cpu) = s.cpu {
            self.cpu.push(cpu);
            if self.cores.len() != s.cores.len() {
                self.cores = vec![History::new(HISTORY); s.cores.len()];
            }
            for (h, v) in self.cores.iter_mut().zip(&s.cores) {
                h.push(*v);
            }
        }
        let frac = |used: u64, total: u64| {
            if total == 0 {
                0.0
            } else {
                used as f64 / total as f64
            }
        };
        self.mem.push(frac(s.mem.used(), s.mem.total));
        self.swap.push(frac(s.mem.swap_used(), s.mem.swap_total));
        self.starts = s.procs.iter().map(|p| (p.pid, p.start)).collect();
        self.snap = s;
        self.refresh_apps(ctx);
        self.refresh_procs();
        self.check_ending();
    }

    fn set_hung(&mut self, hung: Vec<Hung>, ctx: &mut Ctx) {
        if hung != self.hung {
            self.hung = hung;
            self.refresh_apps(ctx);
        }
    }

    fn refresh_apps(&mut self, ctx: &mut Ctx) {
        let keep = (self.apps.selected())
            .and_then(|i| self.app_rows.get(i))
            .map(|r| r.unit.clone());
        let entries = self.entries.clone();
        let scopes = &self.snap.scopes;
        self.app_rows = (scopes.iter())
            .map(|sc| AppRow {
                hung: not_responding(sc, scopes, &entries, &self.hung),
                ..app_row(sc, &entries)
            })
            .collect();
        let rows: Vec<listview::Row> = (0..self.app_rows.len())
            .map(|i| {
                let icon = self.app_rows[i].icon.clone();
                listview::Row {
                    icon: icon.and_then(|n| self.icon(&n, ctx)),
                    cells: vec![
                        self.app_rows[i].name.clone(),
                        self.app_rows[i].status().into(),
                    ],
                }
            })
            .collect();
        self.apps.view.rows = rows;
        let at = keep.and_then(|u| self.app_rows.iter().position(|r| r.unit == u));
        self.apps.view.selection.set_only(at);
        let n = self.app_rows.len();
        self.apps.view.top = self.apps.view.top.min(n.saturating_sub(1));
    }

    fn refresh_procs(&mut self) {
        let keep = self.selected_proc().map(|p| p.pid);
        let mut rows: Vec<ProcRow> = (self.snap.procs.iter())
            .filter(|p| self.all_users || p.uid == self.uid)
            .cloned()
            .collect();
        columns::sort(&mut rows, self.sort.0, self.sort.1);
        self.procs.view.rows = (rows.iter())
            .map(|p| listview::Row {
                icon: None,
                cells: self.columns.iter().map(|&c| columns::text(c, p)).collect(),
            })
            .collect();
        self.proc_rows = rows;
        let at = keep.and_then(|pid| self.proc_rows.iter().position(|p| p.pid == pid));
        self.procs.view.selection.set_only(at);
        let n = self.proc_rows.len();
        self.procs.view.top = self.procs.view.top.min(n.saturating_sub(1));
    }

    fn selected_proc(&self) -> Option<&ProcRow> {
        self.procs.selected().and_then(|i| self.proc_rows.get(i))
    }

    fn selected_app(&self) -> Option<&AppRow> {
        self.apps.selected().and_then(|i| self.app_rows.get(i))
    }

    fn alive(&self, t: &Target) -> bool {
        match t {
            Target::Scope(unit) => self.snap.scopes.iter().any(|s| &s.unit == unit),
            Target::Process(pid, start) => self.starts.get(pid) == Some(start),
        }
    }

    /// Drops what has ended, and offers End Now for the first that outlived [`END_WAIT`].
    fn check_ending(&mut self) {
        let ending = std::mem::take(&mut self.ending);
        self.ending = ending
            .into_iter()
            .filter(|e| self.alive(&e.target))
            .collect();
        if self.ask.is_some() {
            return;
        }
        let now = Instant::now();
        if let Some(e) =
            (self.ending.iter_mut()).find(|e| !e.asked && now.duration_since(e.since) >= END_WAIT)
        {
            e.asked = true;
            let (target, name) = (e.target.clone(), e.name.clone());
            self.ask = Some((Ask::EndNow(target), end_now_box(&name, true)));
        }
    }

    fn end_task(&mut self) -> Reply {
        let Some(row) = self.selected_app().cloned() else {
            return Reply::none();
        };
        if row.hung {
            self.ending
                .retain(|e| e.target != Target::Scope(row.unit.clone()));
            let b = end_now_box(&row.name, false);
            self.ask = Some((Ask::EndNow(Target::Scope(row.unit)), b));
            return Reply::redraw_all();
        }
        worker::kill_scope(row.unit.clone(), SIGTERM);
        self.wait_for(Target::Scope(row.unit), row.name);
        Reply::redraw()
    }

    fn wait_for(&mut self, target: Target, name: String) {
        self.ending.retain(|e| e.target != target);
        self.ending.push(Ending {
            target,
            name,
            since: Instant::now(),
            asked: false,
        });
    }

    fn switch_to(&mut self) -> Reply {
        let Some(row) = self.selected_app().cloned() else {
            return Reply::none();
        };
        let entries = self.entries.clone();
        std::thread::spawn(move || {
            match switchto::activate(|id| window_of(&row.app, &entries, id)) {
                Ok(true) => {}
                Ok(false) => log::info!("Switch To: no window of {}", row.app),
                Err(e) => log::warn!("Switch To {}: {e}", row.app),
            }
        });
        Reply::none()
    }

    fn ask_end_process(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.selected_proc().cloned() else {
            return Reply::none();
        };
        if p.uid != self.uid {
            return Reply::none();
        }
        let b = MsgBox::new(
            "Task Manager Warning",
            Fields {
                text: format!(
                    "Ending {} (PID {}) can lose the data it holds, and a program that needs it \
                     may stop working.\n\nAre you sure you want to end this process?",
                    p.name, p.pid
                ),
                icon: ctx.icons.get("dialog-warning", 32),
                buttons: vec!["&Yes".into(), "&No".into()],
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        let start = self.starts.get(&p.pid).copied().unwrap_or(0);
        self.ask = Some((
            Ask::EndProcess {
                pid: p.pid,
                start,
                name: p.name,
            },
            b,
        ));
        Reply::redraw()
    }

    fn answered(&mut self, ask: Ask, button: Option<usize>) {
        if button != Some(0) {
            if let Ask::EndNow(t) = &ask {
                self.ending.retain(|e| &e.target != t);
            }
            return;
        }
        match ask {
            Ask::EndProcess { pid, start, name } => match worker::kill_pid(pid, SIGTERM) {
                Ok(()) => self.wait_for(Target::Process(pid, start), name),
                Err(e) => log::warn!("end process {pid}: {e}"),
            },
            Ask::EndNow(Target::Scope(unit)) => worker::kill_scope(unit, SIGKILL),
            Ask::EndNow(Target::Process(pid, start)) => {
                if self.starts.get(&pid) == Some(&start)
                    && let Err(e) = worker::kill_pid(pid, SIGKILL)
                {
                    log::warn!("end process {pid} now: {e}");
                }
            }
        }
    }

    fn ask_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let theme = self.forms.clone().unwrap_or_else(|| ctx.forms());
        let Some((_, b)) = &mut self.ask else {
            return Reply::none();
        };
        match b.event(&ev, &ctx.text, &theme) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                let (ask, _) = self.ask.take().expect("asking");
                self.answered(ask, button);
                self.check_ending();
                Reply::redraw_all()
            }
        }
    }

    fn sort_by(&mut self, col: usize) {
        let Some(&c) = self.columns.get(col) else {
            return;
        };
        let ascending = match self.sort {
            (old, asc) if old == c => !asc,
            _ => columns::spec(c).ascending,
        };
        self.sort = (c, ascending);
        self.procs.view.sort = Some((col, ascending));
        self.refresh_procs();
    }

    fn form_event(&mut self, ev: form::Event, ctx: &mut Ctx) -> Reply {
        match ev {
            form::Event::Pressed(pages::END_TASK) => self.end_task(),
            form::Event::Pressed(pages::SWITCH_TO) => self.switch_to(),
            form::Event::Pressed(pages::END_PROCESS) => self.ask_end_process(ctx),
            form::Event::Changed(pages::ALL_USERS) => {
                self.all_users = self.form.checked(pages::ALL_USERS).unwrap_or(false);
                self.refresh_procs();
                Reply::redraw()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn list(&mut self) -> Option<&mut List> {
        match self.page() {
            Page::Applications => Some(&mut self.apps),
            Page::Processes => Some(&mut self.procs),
            Page::Performance => None,
        }
    }

    fn press(&mut self, at: Click, button: u32, ctx: &mut Ctx) -> Reply {
        let (x, y) = (at.x, at.y);
        self.alt_arm.interrupt();
        self.tip = None;
        let lay = pages::layout(self, ctx);
        let px = ctx.theme.fonts.size_px;
        if button == BTN_LEFT {
            match self.bar.press(x, y, &lay.bar, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, path, .. } => {
                    return match Self::menu_act(title, &path) {
                        Some(a) => self.act(a),
                        None => Reply::redraw(),
                    };
                }
                Action::Redraw => return Reply::redraw(),
                Action::Ignored => {}
            }
            if let Some(i) = reprisekit::controls::tabstrip::hit(&lay.tabs, x, y) {
                self.tabs.open = i;
                self.form_for = None;
                self.set_menus();
                return Reply::redraw();
            }
        }
        if let Some(r) = lay.list
            && r.contains(x, y)
        {
            let page = self.page();
            let list = self.list().expect("a page with a list");
            let press = list.press(at, button, r, ctx);
            return match press {
                Press::Column(c) if page == Page::Processes => {
                    self.sort_by(c);
                    Reply::redraw()
                }
                Press::Activate(_) if page == Page::Applications => self.switch_to(),
                Press::None => Reply::none(),
                _ => Reply::redraw(),
            };
        }
        if button == BTN_LEFT {
            let hit = self.form.hit(x, y, &ctx.text, px);
            let ev = self.form.click(hit);
            return self.form_event(ev, ctx);
        }
        Reply::none()
    }

    fn motion(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        let lay = pages::layout(self, ctx);
        let px = ctx.theme.fonts.size_px;
        if self.bar.open.is_some() {
            self.bar.motion(x, y, &lay.bar, &ctx.text, &ctx.theme, px);
            return Reply::redraw();
        }
        let title = self.bar.hover(x, y, &lay.bar) && ctx.styled();
        let hot = reprisekit::controls::tabstrip::hit(&lay.tabs, x, y);
        let tab = self.tabs.hover(hot) && ctx.styled();
        let mut tick = false;
        let list = match (lay.list, self.list()) {
            (Some(r), Some(l)) => l.motion(x, y, r, ctx),
            _ => false,
        };
        let tip_was = self.tip.as_ref().is_some_and(|t| t.shown);
        if let (Some(r), Page::Processes) = (lay.list, self.page()) {
            let l = self.procs.layout(r, ctx);
            let over = match self.procs.view.hit(&l, x, y) {
                listview::Hit::Row(i) => self.proc_rows.get(i).map(|p| p.pid),
                _ => None,
            };
            match over {
                Some(pid) if self.tip.as_ref().is_some_and(|t| t.pid == pid && t.shown) => {}
                Some(pid) => {
                    self.tip = Some(Tip {
                        pid,
                        at: (x, y),
                        since: Instant::now(),
                        shown: false,
                    });
                    tick = true;
                }
                None => self.tip = None,
            }
        } else {
            self.tip = None;
        }
        let hit = self.form.hit(x, y, &ctx.text, px);
        let form = !matches!(self.form.motion(x, y, hit), form::Event::None);
        let redraw = title || tab || list || form || tip_was;
        Reply {
            redraw,
            tick,
            ..Reply::none()
        }
    }

    fn key(&mut self, k: &KeyInput, ctx: &mut Ctx) -> Reply {
        if k.sym == keysym::ALT_L || k.sym == keysym::ALT_R {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        self.tip = None;
        let bar_key = match k.sym {
            keysym::F10 => Some(Key::Activate),
            keysym::ESCAPE => Some(Key::Escape),
            keysym::LEFT | keysym::KP_LEFT => Some(Key::Left),
            keysym::RIGHT | keysym::KP_RIGHT => Some(Key::Right),
            keysym::UP | keysym::KP_UP => Some(Key::Up),
            keysym::DOWN | keysym::KP_DOWN => Some(Key::Down),
            keysym::RETURN | keysym::KP_ENTER => Some(Key::Enter),
            _ => (k.utf8.as_deref())
                .and_then(|s| s.chars().next())
                .filter(|c| c.is_alphanumeric())
                .map(|c| Key::Char(c, k.mods.alt)),
        };
        if let Some(bk) = bar_key {
            let lay = pages::layout(self, ctx);
            let px = ctx.theme.fonts.size_px;
            match self.bar.key(bk, &lay.bar, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, path, .. } => {
                    return match Self::menu_act(title, &path) {
                        Some(a) => self.act(a),
                        None => Reply::redraw(),
                    };
                }
                Action::Redraw => return Reply::redraw(),
                Action::Ignored => {}
            }
        }
        if k.mods.ctrl && matches!(k.sym, keysym::TAB | keysym::ISO_LEFT_TAB) {
            let n = Page::ALL.len();
            let back = k.mods.shift || k.sym == keysym::ISO_LEFT_TAB;
            self.tabs.open = (self.tabs.open + if back { n - 1 } else { 1 }) % n;
            self.form_for = None;
            self.set_menus();
            return Reply::redraw();
        }
        if k.mods.alt
            && let Some(key) = repriseapp::keys::dialog_key(k)
        {
            let ev = self.form.key(key);
            return self.form_event(ev, ctx);
        }
        let lay = pages::layout(self, ctx);
        if let (Some(r), Some(list)) = (lay.list, self.list())
            && let Some(moved) = list.key(k.sym, r, ctx)
        {
            return redraw(moved);
        }
        match (k.sym, self.page()) {
            (keysym::F5, _) => self.act(Act::Refresh),
            (keysym::DELETE | keysym::KP_DELETE, Page::Applications) => self.end_task(),
            (keysym::DELETE | keysym::KP_DELETE, Page::Processes) => self.ask_end_process(ctx),
            (keysym::RETURN | keysym::KP_ENTER, Page::Applications) => self.switch_to(),
            _ => Reply::none(),
        }
    }

    fn key_up(&mut self, k: &KeyInput, ctx: &Ctx) -> Reply {
        if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
            self.alt_arm.interrupt();
            return Reply::none();
        }
        if !self.alt_arm.release_alt() {
            return Reply::none();
        }
        let lay = pages::layout(self, ctx);
        let px = ctx.theme.fonts.size_px;
        self.bar
            .key(Key::Activate, &lay.bar, &ctx.text, &ctx.theme, px);
        Reply::redraw()
    }

    fn tick(&mut self) -> Reply {
        let Some(t) = &mut self.tip else {
            return Reply::none();
        };
        if t.shown {
            return Reply::none();
        }
        if t.since.elapsed() >= TIP_DELAY {
            t.shown = true;
            return Reply::redraw();
        }
        Reply {
            tick: true,
            ..Reply::none()
        }
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        pages::follow_form(self, ctx);
        match ev {
            Event::Wake => {
                let got: Vec<Snapshot> = match &self.worker {
                    Some(w) => w.rx.try_iter().collect(),
                    None => Vec::new(),
                };
                for s in got {
                    self.take(s, ctx);
                }
                let hung = self.hung_rx.as_ref().and_then(|rx| rx.try_iter().last());
                if let Some(h) = hung {
                    self.set_hung(h, ctx);
                }
                pages::follow_form(self, ctx);
                Reply::redraw_all()
            }
            Event::Suspended(on) => {
                self.suspended = on;
                self.follow_visibility();
                Reply::none()
            }
            Event::CloseRequest => quit(),
            Event::Resized => {
                self.fit_lists(ctx);
                Reply::redraw()
            }
            Event::Tick => self.tick(),
            Event::Key(k) => self.key(&k, ctx),
            Event::KeyUp(k) => self.key_up(&k, ctx),
            Event::Pointer(PointerInput::Press {
                x, y, button, time, ..
            }) => {
                let at = ctx.theme.metrics.click(x, y, time);
                self.press(at, button, ctx)
            }
            Event::Pointer(PointerInput::Motion { x, y }) => self.motion(x, y, ctx),
            Event::Pointer(PointerInput::Release { .. }) => {
                let list = self.list().is_some_and(|l| l.release(ctx));
                self.note_widths();
                let ev = self.form.release();
                let r = self.form_event(ev, ctx);
                Reply {
                    redraw: r.redraw || list,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Leave) => {
                let tip = self.tip.take().is_some_and(|t| t.shown);
                let list = self.list().is_some_and(|l| l.leave(ctx));
                let title = self.bar.leave() && ctx.styled();
                let tab = self.tabs.hover(None) && ctx.styled();
                let form = !matches!(self.form.leave(), form::Event::None);
                redraw(list || title || tab || form || tip)
            }
            Event::Pointer(PointerInput::Scroll { rows, x, y }) => {
                let lay = pages::layout(self, ctx);
                match (lay.list, self.list()) {
                    (Some(r), Some(l)) if r.contains(x, y) => redraw(l.wheel(rows, r, ctx)),
                    _ => Reply::none(),
                }
            }
            _ => Reply::none(),
        }
    }
}

fn app_row(sc: &scopes::Scope, entries: &[AppEntry]) -> AppRow {
    let e = reprisefs::apps::entry_for_app_id(entries, &sc.app, |_| true);
    AppRow {
        unit: sc.unit.clone(),
        app: sc.app.clone(),
        name: e.map_or_else(|| sc.app.clone(), |e| e.name.clone()),
        icon: e.and_then(|e| e.icon.clone()),
        hung: false,
    }
}

/// Whether a window of scope `sc`'s program is among `hung`: one whose client runs in the scope,
/// or one whose client is in no scope (a sandboxed program's) and whose app id is the program's,
/// as Switch To matches it.
pub fn not_responding(
    sc: &scopes::Scope,
    scopes: &[scopes::Scope],
    entries: &[AppEntry],
    hung: &[Hung],
) -> bool {
    hung.iter().any(|(id, pid)| {
        let pid = i32::try_from(*pid).unwrap_or(0);
        sc.pids.contains(&pid)
            || (!scopes.iter().any(|s| s.pids.contains(&pid)) && window_of(&sc.app, entries, id))
    })
}

/// Whether a window reporting app id `window` belongs to the program started from entry `app`:
/// the same id, or a window whose own entry is `app`'s.
pub fn window_of(app: &str, entries: &[AppEntry], window: &str) -> bool {
    window.eq_ignore_ascii_case(app)
        || reprisefs::apps::entry_for_app_id(entries, window, |_| true)
            .is_some_and(|e| e.id.strip_suffix(".desktop").unwrap_or(&e.id) == app)
}

/// A size in KiB as a task manager writes it: "12,345 K".
pub fn kb_text(kb: u64) -> String {
    format!("{} K", grouped(kb))
}

/// `n` with a comma between each three digits.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// End Now for `name`, which `asked` to end first, or found not responding.
fn end_now_box(name: &str, asked: bool) -> MsgBox {
    let why = if asked {
        "is not responding to the request to end"
    } else {
        "is not responding"
    };
    MsgBox::new(
        format!("End Program - {name}"),
        Fields {
            text: format!(
                "{name} {why}.\n\nTo end it now, click End Now. Anything it has not saved is \
                 lost."
            ),
            icon: None,
            buttons: vec!["&End Now".into(), "Cancel".into()],
            default: 1,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        },
    )
}

fn quit() -> Reply {
    Reply {
        quit: true,
        ..Reply::none()
    }
}

fn redraw(changed: bool) -> Reply {
    if changed {
        Reply::redraw()
    } else {
        Reply::none()
    }
}

impl App for TaskMgr {
    fn title(&self) -> String {
        "Task Manager".into()
    }

    fn app_id(&self) -> String {
        "reprise.taskmgr".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        pages::CLIENT
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if self.pick.is_some() {
            kids.push(Child {
                id: PICK,
                title: "Select Columns".into(),
                size: (self.forms.as_ref())
                    .map_or(selectcols::CLIENT, |t| t.text.size(selectcols::CLIENT)),
                fixed: true,
                modal: true,
            });
        }
        if let (Some(theme), Some(text), Some((_, b))) = (&self.forms, &self.text, &self.ask) {
            kids.push(b.child(ASK, text, theme));
        }
        kids
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        if let Some(mut s) = self.sampler.take() {
            s.disk = self.columns.contains(&ProcColumn::Disk);
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = waker.clone();
            reprisebus::tray::watch_responding(move |h| {
                if tx.send(h).is_ok() {
                    wake.wake();
                }
            });
            self.hung_rx = Some(rx);
            self.worker = Some(worker::spawn(s, move || waker.wake()));
            self.follow_visibility();
        }
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.forms = Some(ctx.forms());
        self.text = Some(ctx.text.clone());
        match win {
            ASK => self.ask_event(ev, ctx),
            PICK => self.pick_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        if win == ASK {
            if let Some((_, b)) = &self.ask {
                b.draw(c, &ctx.forms(), &ctx.text);
            }
            return;
        }
        if win == PICK {
            if let Some(f) = &self.pick {
                c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
                let p = reprisekit::look::Paint {
                    s: 1,
                    ..ctx.theme.paint()
                };
                form::draw(c, &ctx.text, &p, f, true, ctx.theme.fonts.size_px);
            }
            return;
        }
        pages::draw(self, c, ctx);
    }
}

impl repriseapp::Extended for TaskMgr {
    fn min_size(&self, _win: WinId) -> Option<(i32, i32)> {
        Some(pages::MIN)
    }

    fn popup(&mut self, win: WinId, _ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        (win == repriseapp::MAIN && self.bar.open.is_some())
            .then(|| Box::new(&mut self.bar) as Box<dyn Hosted>)
    }

    fn follow_scale(&mut self, _win: WinId, scale: i32, ctx: &mut Ctx) {
        if self.icons.values().flatten().any(|c| c.scale() != scale) {
            self.icons.clear();
            self.refresh_apps(ctx);
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == ASK {
            if let Some((_, b)) = &self.ask {
                b.access(tree, &ctx.text, &ctx.forms());
            }
            return;
        }
        if win == PICK {
            if let Some(f) = &self.pick {
                oneshot::access_form(f, tree, &ctx.text, ctx.theme.fonts.size_px);
            }
            return;
        }
        pages::access(self, tree, ctx);
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let entries = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    // SAFETY: getuid has no failure case and touches no memory.
    let uid = unsafe { libc::getuid() };
    let mut app = TaskMgr::new(Some(sample::Sampler::system()), entries, uid);
    app.restore(State::path());
    repriseapp::run(Box::new(app)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
