//! reprise-schedtasks: the systemd user manager's timers with their schedule, next and last run
//! and status; Run Now, Stop, Enable or Disable, Properties and View Output on the one picked;
//! New Task writes a timer of its own, and Edit and Delete change or remove one it wrote
//! ([`units`]).
//! docs/programs.md covers what the window does.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("schedtasks");

mod model;
mod newtask;
mod notice;
mod output;
mod props;
mod systemd;
mod units;

use model::Task;
use repriseapp::asks::{self, Asks};
use repriseapp::ctxmenu::{Popup, Step, item};
use repriseapp::detailslist::{List, Press};
use repriseapp::filedlg::{Driven, Host};
use repriseapp::oneshot::{self, ErrorBox, MsgBox};
use repriseapp::{App, BTN_RIGHT, Child, Ctx, Event, KeyInput, MAIN, PointerInput, Reply, WinId};
use reprisekit::access::{ROOT as ACCESS_ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::filedlg::{self, DlgMode, FileDlg};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{Column, Rows};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;
use reprisekit::tr;
use std::borrow::Cow;
use std::ffi::OsString;
use systemd::{Bus, Cmd, Update};

/// Wide enough for every column at its width, its scrollbar and its frame.
const CLIENT: (i32, i32) = (800, 400);
const MARGIN: i32 = 8;
const STATUS_Y: i32 = 40;
/// Where the list starts, at 100% text.
const LIST_Y: i32 = 60;

const PROPS: WinId = WinId::named("props");
const OUTPUT: WinId = WinId::named("output");
const NEW_TASK: WinId = WinId::named("new");
const BROWSE: WinId = WinId::named("browse");

const RUN: &str = "run";
const STOP: &str = "stop";
const TOGGLE: &str = "toggle";
const PROPERTIES: &str = "properties";
const VIEW_OUTPUT: &str = "output";
const NEW: &str = "new";
const EDIT: &str = "edit";
const DELETE: &str = "delete";
const STATUS: &str = "status";

fn columns() -> [(String, i32); 5] {
    [
        (tr!("column-name"), 220),
        (tr!("column-schedule"), 220),
        (tr!("column-next"), 120),
        (tr!("column-last"), 120),
        (tr!("column-status"), 70),
    ]
}

/// The rows the list draws: each task's cells, worked out when the tasks change.
#[derive(Default)]
struct Lines {
    cells: Vec<[String; 5]>,
    icon: Option<Canvas>,
}

impl Rows for Lines {
    fn len(&self) -> usize {
        self.cells.len()
    }
    fn icon(&self, _row: usize) -> Option<&Canvas> {
        self.icon.as_ref()
    }
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let cell = self.cells.get(row).and_then(|c| c.get(col));
        Cow::Borrowed(cell.map_or("", String::as_str))
    }
}

fn cells(t: &Task, is_12h: bool) -> [String; 5] {
    [
        t.name().to_string(),
        t.schedule(is_12h),
        t.next_text(is_12h),
        t.last_text(is_12h),
        t.status(),
    ]
}

/// What a column sorts by: its time, or its text with case ignored.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SortKey {
    Time(Option<u64>),
    Text(String),
}

fn sort_key(t: &Task, col: usize, is_12h: bool) -> SortKey {
    match col {
        2 => SortKey::Time(t.next),
        3 => SortKey::Time(t.last),
        _ => SortKey::Text(cells(t, is_12h)[col].to_lowercase()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Run,
    Stop,
    Toggle,
    Properties,
    Output,
    Edit,
    Delete,
}

/// What a question box asks: whether to delete this timer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeleteAsk(String);

struct SchedTasks {
    bus: Option<Bus>,
    tasks: Vec<Task>,
    /// Why the manager could not list the timers.
    unanswered: Option<String>,
    list: List<Lines>,
    /// The column a title click sorts by and whether it ascends; None keeps the manager's order.
    sort: Option<(usize, bool)>,
    form: Form,
    popup: Option<Popup<Act>>,
    note: ErrorBox,
    props: Option<props::Props>,
    output: Option<output::Output>,
    new_task: Option<newtask::NewTaskBox>,
    browse: Option<FileDlg>,
    browse_host: Host,
    asking: Asks<DeleteAsk>,
    /// The timer New Task just made, picked once the list shows it.
    want_pick: Option<String>,
    big_icon: Option<Canvas>,
    warn_icon: Option<Canvas>,
    is_12h: bool,
    /// The main window had its first configure ([`SchedTasks::widened`]).
    configured: bool,
}

fn buttons() -> Vec<Item> {
    let mut x = MARGIN;
    let mut items = Vec::new();
    for (id, label, w) in [
        (RUN, tr!("run"), 85),
        (STOP, tr!("stop"), 75),
        (TOGGLE, tr!("disable"), 75),
        (PROPERTIES, tr!("properties"), 85),
        (VIEW_OUTPUT, tr!("view-output"), 90),
        (NEW, tr!("new-task"), 85),
        (EDIT, tr!("edit"), 75),
        (DELETE, tr!("delete"), 75),
    ] {
        items.push(Item::button(
            id,
            Rect::new(x, MARGIN, w, BUTTON_H),
            &label,
            false,
            false,
        ));
        x += w + 6;
    }
    items.push(Item::label(
        STATUS,
        Rect::new(MARGIN, STATUS_Y, 2000, 13),
        "",
    ));
    items
}

impl SchedTasks {
    fn new() -> SchedTasks {
        let columns = columns()
            .into_iter()
            .map(|(t, w)| Column::new(&t, w))
            .collect();
        let mut list = List::new(columns);
        list.view.frame = true;
        let mut form = Form::new(buttons());
        form.set_focus(None);
        SchedTasks {
            bus: None,
            tasks: Vec::new(),
            unanswered: None,
            list,
            sort: None,
            form,
            popup: None,
            note: ErrorBox::default(),
            props: None,
            output: None,
            new_task: None,
            browse: None,
            browse_host: Host::default(),
            asking: Asks::default(),
            want_pick: None,
            big_icon: None,
            warn_icon: None,
            is_12h: reprisefs::locale::is_12h(),
            configured: false,
        }
    }

    fn send(&self, cmd: Cmd) {
        if let Some(bus) = &self.bus {
            let _ = bus.cmds.send(cmd);
        }
    }

    fn picked(&self) -> Option<&Task> {
        self.list.selected().and_then(|i| self.tasks.get(i))
    }

    /// Shows `tasks`, keeping the timer picked, the open sheet's, and the latest run seen of each.
    fn show(&mut self, mut tasks: Vec<Task>) {
        for t in &mut tasks {
            if let Some(was) = self.tasks.iter().find(|w| w.timer == t.timer) {
                t.keep_run(was);
            }
        }
        let picked = self.picked().map(|t| t.timer.clone());
        self.tasks = tasks;
        self.sort_tasks();
        self.unanswered = None;
        let made = self
            .want_pick
            .as_ref()
            .and_then(|w| self.tasks.iter().position(|t| t.timer == *w));
        if made.is_some() {
            self.want_pick = None;
        }
        let keep =
            made.or_else(|| picked.and_then(|p| self.tasks.iter().position(|t| t.timer == p)));
        self.list.view.selection = Default::default();
        self.list.view.selection.set_only(keep);
        if let Some(p) = &mut self.props {
            match self.tasks.iter().find(|t| t.timer == p.timer) {
                Some(t) => p.refresh(t, self.is_12h),
                None => self.props = None,
            }
        }
        self.sync();
    }

    fn sort_tasks(&mut self) {
        let Some((col, asc)) = self.sort else {
            return;
        };
        let is_12h = self.is_12h;
        let mut keyed: Vec<_> = std::mem::take(&mut self.tasks)
            .into_iter()
            .map(|t| (sort_key(&t, col, is_12h), t))
            .collect();
        keyed.sort_by(|a, b| if asc { a.0.cmp(&b.0) } else { b.0.cmp(&a.0) });
        self.tasks = keyed.into_iter().map(|(_, t)| t).collect();
    }

    /// A click on column `col`'s title sorts by it, a second flips it; the pick stays on its task.
    fn sort_by(&mut self, col: usize) {
        let asc = !matches!(self.sort, Some((c, true)) if c == col);
        self.sort = Some((col, asc));
        self.list.view.sort = self.sort;
        let picked = self.picked().map(|t| t.timer.clone());
        self.sort_tasks();
        let keep = picked.and_then(|p| self.tasks.iter().position(|t| t.timer == p));
        self.list.view.selection = Default::default();
        self.list.view.selection.set_only(keep);
        self.sync();
    }

    /// Fills the rows, the status line and the buttons from the tasks and the pick.
    fn sync(&mut self) {
        let is_12h = self.is_12h;
        self.list.view.rows.cells = self.tasks.iter().map(|t| cells(t, is_12h)).collect();
        let status = match &self.unanswered {
            Some(why) => tr!("status-unanswered", why = why.as_str()),
            None if self.tasks.is_empty() => tr!("status-none"),
            None => tr!("status-count", count = self.tasks.len() as i64),
        };
        self.form.set_text(STATUS, &status);
        let picked = self.picked().cloned();
        self.form
            .set_enabled(RUN, picked.as_ref().is_some_and(Task::can_run));
        self.form
            .set_enabled(STOP, picked.as_ref().is_some_and(Task::running));
        self.form.set_enabled(VIEW_OUTPUT, picked.is_some());
        self.form.set_enabled(NEW, self.bus.is_some());
        self.form
            .set_enabled(EDIT, picked.as_ref().is_some_and(|t| t.ours));
        self.form
            .set_enabled(DELETE, picked.as_ref().is_some_and(|t| t.ours));
        self.form
            .set_enabled(TOGGLE, picked.as_ref().is_some_and(Task::can_toggle));
        self.form.set_enabled(PROPERTIES, picked.is_some());
        let label = match picked.is_some_and(|t| !t.on()) {
            true => tr!("enable"),
            false => tr!("disable"),
        };
        if let Some(Control::Button { label: l, .. }) =
            self.form.item_mut(TOGGLE).map(|i| &mut i.control)
        {
            *l = label;
        }
    }

    /// At the first configure, a size kept from an earlier session that is too narrow for every
    /// column at this text size grows to the width that fits them; a wider one stays.
    fn widened(&mut self, ctx: &Ctx) -> Option<(i32, i32)> {
        if std::mem::replace(&mut self.configured, true) || ctx.maximized {
            return None;
        }
        let fit = ctx.forms().text.size(CLIENT).0;
        (ctx.size.0 < fit).then_some((fit, ctx.size.1))
    }

    fn list_rect(&self, ctx: &Ctx) -> Rect {
        let y = ctx.forms().text.len(LIST_Y);
        let (w, h) = ctx.size;
        Rect::new(MARGIN, y, (w - 2 * MARGIN).max(0), (h - y - MARGIN).max(0))
    }

    fn act(&mut self, act: Act) -> Reply {
        let Some(t) = self.picked().cloned() else {
            return Reply::none();
        };
        match act {
            Act::Run if t.can_run() => self.send(Cmd::Run(t.service)),
            Act::Stop if t.running() => self.send(Cmd::Stop(t.service)),
            Act::Toggle if t.can_toggle() => self.send(Cmd::Turn {
                on: !t.on(),
                timer: t.timer,
                file_state: t.file_state,
            }),
            Act::Properties => {
                self.props = Some(props::Props::new(&t, self.big_icon.clone(), self.is_12h));
                return Reply::redraw_all();
            }
            Act::Output => {
                self.output = Some(output::Output::new(&t.service, t.name(), self.is_12h));
                self.send(Cmd::Output(t.service));
                return Reply::redraw_all();
            }
            Act::Edit if t.ours => {
                self.open_edit(&t);
                return Reply::redraw_all();
            }
            Act::Delete if t.ours => {
                self.ask_delete(&t);
                return Reply::redraw_all();
            }
            _ => {}
        }
        Reply::redraw()
    }

    fn ask_delete(&mut self, t: &Task) {
        let b = MsgBox::new(
            tr!("delete-title"),
            Fields {
                text: tr!("delete-text", name = t.name(), unit = t.timer.as_str()),
                icon: self.warn_icon.clone(),
                buttons: vec![tr!("delete-yes"), tr!("delete-no")],
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning);
        self.asking.ask(b, DeleteAsk(t.timer.clone()));
    }

    /// Opens the box on the task its files hold, or says they no longer read as New Task wrote
    /// them.
    fn open_edit(&mut self, t: &Task) {
        if self.new_task.is_some() {
            return;
        }
        let read = std::path::Path::new(&t.file)
            .parent()
            .and_then(|dir| units::read(dir, &t.timer));
        match read {
            Some(task) => {
                let b = newtask::NewTaskBox::edit(newtask::now, self.is_12h, &t.timer, &task);
                self.new_task = Some(b);
            }
            None => self
                .note
                .show(&tr!("name"), tr!("edit-unread", name = t.name()), None),
        }
    }

    fn open_new(&mut self) {
        if self.new_task.is_none() {
            self.new_task = Some(newtask::NewTaskBox::new(newtask::now, self.is_12h));
        }
    }

    fn wake(&mut self) -> Reply {
        let updates: Vec<Update> = match &self.bus {
            Some(bus) => bus.updates.try_iter().collect(),
            None => Vec::new(),
        };
        for u in updates {
            match u {
                Update::Tasks(Ok(tasks)) => self.show(tasks),
                Update::Tasks(Err(why)) => {
                    self.unanswered = Some(why);
                    self.sync();
                }
                Update::Created(timer) => self.want_pick = Some(timer),
                Update::Output(unit, read) => {
                    if let Some(o) = self.output.as_mut().filter(|o| o.service == unit) {
                        o.show(read);
                    }
                }
                Update::Refused(cmd, why) => {
                    let text = match cmd {
                        Cmd::Run(unit) => tr!("refused-run", unit = unit, why = why),
                        Cmd::Stop(unit) => tr!("refused-stop", unit = unit, why = why),
                        Cmd::Turn {
                            timer, on: true, ..
                        } => {
                            tr!("refused-enable", unit = timer, why = why)
                        }
                        Cmd::Turn { timer, .. } => tr!("refused-disable", unit = timer, why = why),
                        Cmd::Create { .. } => tr!("refused-create", why = why),
                        Cmd::Edit { timer, .. } => tr!("refused-edit", unit = timer, why = why),
                        Cmd::Delete(timer) => tr!("refused-delete", unit = timer, why = why),
                        Cmd::Refresh | Cmd::Output(_) => continue,
                    };
                    self.note.show(&tr!("name"), text, None);
                }
            }
        }
        Reply::redraw_all()
    }

    fn open_menu(&mut self, x: i32, y: i32, ctx: &Ctx, by_keyboard: bool) -> Reply {
        let Some(t) = self.picked() else {
            return Reply::none();
        };
        let toggle = match t.on() {
            true => tr!("menu-disable"),
            false => tr!("menu-enable"),
        };
        let rows = vec![
            (item(&tr!("menu-run"), t.can_run()), Some(Act::Run)),
            (item(&tr!("menu-stop"), t.running()), Some(Act::Stop)),
            (item(&toggle, t.can_toggle()), Some(Act::Toggle)),
            (MenuItem::Separator, None),
            (item(&tr!("menu-output"), true), Some(Act::Output)),
            (item(&tr!("menu-edit"), t.ours), Some(Act::Edit)),
            (item(&tr!("menu-delete"), t.ours), Some(Act::Delete)),
            (MenuItem::Separator, None),
            (item(&tr!("menu-properties"), true), Some(Act::Properties)),
        ];
        self.popup = Popup::open(rows, x, y, ctx, by_keyboard);
        Reply::redraw()
    }

    fn popup_event(&mut self, ev: &Event, ctx: &Ctx) -> Option<Reply> {
        let popup = self.popup.as_mut()?;
        let step = match ev {
            Event::Pointer(PointerInput::Press { x, y, .. }) => popup.press(*x, *y, ctx),
            Event::Pointer(PointerInput::Motion { x, y }) => {
                return Some(match popup.motion(*x, *y, ctx) {
                    true => Reply::redraw(),
                    false => Reply::none(),
                });
            }
            Event::Key(k) => popup.key(k, ctx),
            Event::Focus(false) => Step::Closed,
            _ => return Some(Reply::none()),
        };
        Some(match step {
            Step::Chosen(act) => {
                self.popup = None;
                match act {
                    Some(a) => self.act(a),
                    None => Reply::redraw(),
                }
            }
            Step::Closed => {
                self.popup = None;
                Reply::redraw()
            }
            Step::Redraw => Reply::redraw(),
            Step::None => Reply::none(),
        })
    }

    fn key(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
        let r = self.list_rect(ctx);
        Some(match k.sym {
            keysym::F5 => {
                self.send(Cmd::Refresh);
                Reply::none()
            }
            keysym::RETURN | keysym::KP_ENTER if self.form.focus().is_none() => {
                self.act(Act::Properties)
            }
            keysym::MENU => {
                let lay = self.list.layout(r, ctx);
                let row = self.list.selected().unwrap_or(0) as i32;
                let y = lay.body.y + (row - self.list.view.top as i32 + 1) * lay.row_h();
                self.open_menu(lay.body.x + 20, y, ctx, true)
            }
            _ if k.mods.alt || self.form.focus().is_some() => return None,
            _ => {
                let moved = self.list.key(k, r, ctx)?;
                self.sync();
                match moved {
                    true => Reply::redraw(),
                    false => Reply::none(),
                }
            }
        })
    }

    /// A press or wheel turn over the list; None for the rest, which the form sees too.
    fn list_pointer(&mut self, ev: &PointerInput, ctx: &Ctx) -> Option<Reply> {
        let r = self.list_rect(ctx);
        let redraw = |b: bool| if b { Reply::redraw() } else { Reply::none() };
        Some(match *ev {
            PointerInput::Press {
                x, y, button, time, ..
            } if r.contains(x, y) => {
                self.form.set_focus(None);
                let at = Click {
                    x,
                    y,
                    time,
                    double_ms: ctx.theme.metrics.double_click_ms,
                };
                let press = self.list.press(at, button, r, ctx);
                self.sync();
                match press {
                    Press::Activate(_) => self.act(Act::Properties),
                    Press::Row(_) if button == BTN_RIGHT => self.open_menu(x, y, ctx, false),
                    Press::None => Reply::none(),
                    _ => Reply::redraw(),
                }
            }
            PointerInput::Scroll { x, y, rows } if r.contains(x, y) => {
                redraw(self.list.wheel(rows, r, ctx))
            }
            PointerInput::ScrollAcross { x, y, cols } if r.contains(x, y) => {
                redraw(self.list.wheel_across(cols, r, ctx))
            }
            _ => return None,
        })
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if let Some(reply) = self.popup_event(&ev, ctx) {
            return reply;
        }
        match &ev {
            Event::CloseRequest => {
                return Reply {
                    quit: true,
                    ..Reply::none()
                };
            }
            Event::Resized => {
                let r = self.list_rect(ctx);
                self.list.follow(r, ctx);
                return Reply {
                    size: self.widened(ctx),
                    ..Reply::redraw()
                };
            }
            Event::Key(k) => {
                if let Some(reply) = self.key(k, ctx) {
                    return reply;
                }
            }
            Event::Pointer(p) => {
                if let Some(reply) = self.list_pointer(p, ctx) {
                    return reply;
                }
            }
            _ => {}
        }
        // The list follows every motion, release and leave, and the form sees them too: a press
        // on one of its buttons acts on the release, and only while the pointer is still on it.
        let lit = match &ev {
            Event::Pointer(p) => {
                let r = self.list_rect(ctx);
                let released = self.list.follow_pointer(p, r, ctx);
                if let Some(col) = released.column {
                    self.sort_by(col);
                }
                released.redraw
            }
            _ => false,
        };
        match repriseapp::keys::form_input(&mut self.form, &ev, ctx) {
            Some(form::Event::Pressed(RUN)) => self.act(Act::Run),
            Some(form::Event::Pressed(STOP)) => self.act(Act::Stop),
            Some(form::Event::Pressed(VIEW_OUTPUT)) => self.act(Act::Output),
            Some(form::Event::Pressed(EDIT)) => self.act(Act::Edit),
            Some(form::Event::Pressed(DELETE)) => self.act(Act::Delete),
            Some(form::Event::Pressed(NEW)) => {
                self.open_new();
                Reply::redraw_all()
            }
            Some(form::Event::Pressed(TOGGLE)) => self.act(Act::Toggle),
            Some(form::Event::Pressed(PROPERTIES)) => self.act(Act::Properties),
            Some(form::Event::None) | None if !lit => Reply::none(),
            Some(_) | None => Reply::redraw(),
        }
    }

    fn props_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        let closes = match ev {
            Event::CloseRequest => true,
            Event::Blink => {
                p.sheet.open_mut().blink(ctx.caret_on);
                return Reply::redraw();
            }
            ev => match repriseapp::keys::sheet_input(&mut p.sheet, &ev, ctx) {
                Some(e) if props::Props::closes(&e) => true,
                Some(propsheet::Event::None) | None => return Reply::none(),
                Some(_) => return Reply::redraw(),
            },
        };
        if closes {
            self.props = None;
        }
        Reply::redraw_all()
    }

    fn new_task_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(d) = self.new_task.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => newtask::Outcome::Cancel,
            Event::Blink => {
                d.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            ev => match repriseapp::keys::form_input(&mut d.form, &ev, ctx) {
                Some(e) => d.event(e),
                None => return Reply::none(),
            },
        };
        match out {
            newtask::Outcome::None => Reply::none(),
            newtask::Outcome::Redraw => Reply::redraw(),
            newtask::Outcome::Browse => {
                let fields = filedlg::Fields {
                    title: tr!("browse-title"),
                    accept_label: tr!("browse-open"),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![tr!("browse-all")],
                };
                let dir = d.browse_dir();
                self.browse = Some(self.browse_host.open_last(&dir, fields, ctx.forms().text));
                Reply::redraw_all()
            }
            newtask::Outcome::Create(task) => {
                match d.editing.clone() {
                    Some((timer, was)) => self.send(Cmd::Edit { timer, was, task }),
                    None => {
                        let taken = self.tasks.iter().map(|t| t.timer.clone()).collect();
                        self.send(Cmd::Create { task, taken });
                    }
                }
                self.new_task = None;
                self.browse = None;
                Reply::redraw_all()
            }
            newtask::Outcome::Cancel => {
                self.new_task = None;
                self.browse = None;
                Reply::redraw_all()
            }
        }
    }

    fn browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(d) = &mut self.browse else {
            return Reply::none();
        };
        let (text, forms) = (ctx.text.clone(), ctx.forms());
        let picked = match self.browse_host.drive(d, &ev, &text, &forms, ctx) {
            Driven::Reply(r) => return *r,
            Driven::Cancelled => None,
            Driven::Picked(path) => Some(path),
        };
        if let (Some(path), Some(n)) = (picked, self.new_task.as_mut()) {
            n.browsed(&path);
        }
        self.browse = None;
        Reply::redraw_all()
    }

    fn question_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        let forms = ctx.forms();
        match self.asking.event(win, &ev, &ctx.text, &forms) {
            asks::Answer::None => Reply::none(),
            asks::Answer::Redraw => Reply::redraw(),
            asks::Answer::Closed(DeleteAsk(timer), Some(0)) => {
                if self.tasks.iter().any(|t| t.timer == timer && t.ours) {
                    self.send(Cmd::Delete(timer));
                }
                Reply::redraw_all()
            }
            asks::Answer::Closed(..) => Reply::redraw_all(),
        }
    }

    fn output_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(o) = self.output.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => output::Outcome::Close,
            Event::Blink => {
                o.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            ev => match repriseapp::keys::form_input(&mut o.form, &ev, ctx) {
                Some(e) => o.event(e),
                None => return Reply::none(),
            },
        };
        match out {
            output::Outcome::None => Reply::none(),
            output::Outcome::Redraw => Reply::redraw(),
            output::Outcome::Refresh => {
                let unit = o.service.clone();
                self.send(Cmd::Output(unit));
                Reply::redraw()
            }
            output::Outcome::Close => {
                self.output = None;
                Reply::redraw_all()
            }
        }
    }

    fn load_icons(&mut self, ctx: &mut Ctx) {
        if self.list.view.rows.icon.is_none() {
            self.list.view.rows.icon = ctx.icons.get(ICON, 16);
            self.big_icon = ctx.icons.get(ICON, 32);
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
            self.browse_host.icons.prime(&mut ctx.icons);
        }
    }

    fn draw_main(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        form::draw(c, &ctx.text, &paint, &self.form, true, forms.fonts.size_px);
        self.list.draw(c, self.list_rect(ctx), ctx);
        if let Some(p) = &self.popup {
            p.draw(c, ctx);
        }
    }

    fn draw_new_task(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(d) = &self.new_task {
            let paint = Paint {
                s: 1,
                ..forms.paint()
            };
            form::draw(c, &ctx.text, &paint, &d.form, true, forms.fonts.size_px);
        }
    }

    fn draw_browse(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        if let Some(d) = &self.browse {
            let paint = Paint {
                s: 1,
                ..forms.paint()
            };
            let r = Rect::new(0, 0, d.client().0, d.client().1);
            d.draw(c, &ctx.text, &paint, r, forms.fonts.size_px);
        }
    }

    fn draw_output(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(o) = &self.output {
            let paint = Paint {
                s: 1,
                ..forms.paint()
            };
            form::draw(c, &ctx.text, &paint, &o.form, true, forms.fonts.size_px);
        }
    }

    fn draw_props(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(p) = &self.props {
            let paint = Paint {
                s: 1,
                ..forms.paint()
            };
            propsheet::draw(c, &ctx.text, &paint, &p.sheet, true, forms.fonts.size_px);
        }
    }
}

/// The rows' and the sheet's icon, and the desktop entry's.
const ICON: &str = "appointment-soon";

impl App for SchedTasks {
    fn title(&self) -> String {
        tr!("name")
    }

    fn app_id(&self) -> String {
        "reprise.schedtasks".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        repriseapp::start_forms().text.size(CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        let forms = repriseapp::start_forms();
        let text = repriseapp::start_text();
        let mut out = Vec::new();
        if let Some(p) = &self.props {
            out.push(Child {
                id: PROPS,
                title: p.title(),
                size: forms.text.size(props::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(d) = &self.new_task {
            out.push(Child {
                id: NEW_TASK,
                title: match d.editing {
                    Some(_) => tr!("edit-title"),
                    None => tr!("new-title"),
                },
                size: forms.text.size(newtask::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(d) = &self.browse {
            out.push(Child {
                id: BROWSE,
                title: tr!("browse-title"),
                size: d.client(),
                fixed: false,
                modal: true,
            });
        }
        out.extend(self.asking.child(&text, &forms));
        if let Some(o) = &self.output {
            out.push(Child {
                id: OUTPUT,
                title: o.title(),
                size: forms.text.size(output::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        out.extend(self.note.child(&text, &forms));
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.bus = Some(systemd::watch(waker));
        self.sync();
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.load_icons(ctx);
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        match win {
            PROPS => self.props_event(ev, ctx),
            OUTPUT => self.output_event(ev, ctx),
            NEW_TASK => self.new_task_event(ev, ctx),
            BROWSE => self.browse_event(ev, ctx),
            w if self.asking.owns(w) => self.question_event(w, ev, ctx),
            w if self.note.owns(w) => self.note.event(w, &ev, &ctx.text, &ctx.forms()),
            _ => self.main_event(ev, ctx),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            PROPS => self.draw_props(c, ctx),
            OUTPUT => self.draw_output(c, ctx),
            NEW_TASK => self.draw_new_task(c, ctx),
            BROWSE => self.draw_browse(c, ctx),
            w if self.asking.owns(w) => self.asking.draw(c, &forms, &ctx.text),
            w if self.note.owns(w) => self.note.draw(c, &forms, &ctx.text),
            _ => self.draw_main(c, ctx),
        }
    }
}

impl repriseapp::Extended for SchedTasks {
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match (win, &self.browse) {
            (MAIN, _) => Some((420, 200)),
            (BROWSE, Some(d)) => Some(d.min_client()),
            _ => None,
        }
    }

    /// Browse covers New Task; a question covers the window it was raised over.
    fn parent(&self, win: WinId) -> Option<WinId> {
        if self.note.owns(win) {
            return self.note.parent(win);
        }
        if self.asking.owns(win) {
            return self.asking.parent(win);
        }
        Some(match win {
            BROWSE if self.new_task.is_some() => NEW_TASK,
            _ => MAIN,
        })
    }

    fn drop_form(&mut self, win: WinId) -> Option<repriseapp::DropForm<'_>> {
        let form = match win {
            NEW_TASK => &mut self.new_task.as_mut()?.form,
            _ => return None,
        };
        Some(repriseapp::DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<repriseapp::FileDrop<'_>> {
        (win == BROWSE).then_some(repriseapp::FileDrop {
            dlg: self.browse.as_mut()?,
            host: &mut self.browse_host,
        })
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win != MAIN {
            return repriseapp::reader_popup(self, win, ctx);
        }
        self.popup.as_ref()?;
        Some(Box::new(&mut self.popup))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            PROPS => {
                if let Some(p) = &self.props {
                    oneshot::access_sheet(&p.sheet, tree, &ctx.text, forms.fonts.size_px);
                }
            }
            OUTPUT => {
                if let Some(o) = &self.output {
                    oneshot::access_form(&o.form, tree, &ctx.text, forms.fonts.size_px);
                }
            }
            NEW_TASK => {
                if let Some(d) = &self.new_task {
                    oneshot::access_form(&d.form, tree, &ctx.text, forms.fonts.size_px);
                }
            }
            BROWSE => {
                if let Some(d) = &self.browse {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.access(tree, r, &ctx.text, forms.fonts.size_px);
                }
            }
            w if self.asking.owns(w) => self.asking.access(tree, &ctx.text, &forms),
            w if self.note.owns(w) => self.note.access(tree, &ctx.text, &forms),
            _ => {
                let focus = self
                    .form
                    .access(tree, ACCESS_ROOT, &ctx.text, forms.fonts.size_px);
                let lay = self.list.layout(self.list_rect(ctx), ctx);
                let list = self.list.view.access(&lay, tree, ACCESS_ROOT, "tasks");
                tree.set_focus(focus.unwrap_or(list));
            }
        }
    }
}

pub fn main(args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    if let [flag, name] = args.as_slice()
        && flag == units::FAILED
    {
        return notice::tell(&name.to_string_lossy());
    }
    if let [flag, dir, stem, boot] = args.as_slice()
        && flag == units::ONCE_PER_BOOT
    {
        let (stem, boot) = (stem.to_string_lossy(), boot.to_string_lossy());
        if let Err(e) = units::stamp_run(std::path::Path::new(dir), &stem, &boot) {
            log::error!("{stem} does not run, as its run was not stamped: {e}");
            std::process::exit(1);
        }
        return;
    }
    repriseapp::run(Box::new(SchedTasks::new())).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
