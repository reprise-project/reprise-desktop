//! reprise-schedtasks: the systemd user manager's timers with their schedule, next and last run
//! and status; Run Now, Enable or Disable and Properties on the one picked. docs/programs.md
//! covers what the window does.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("schedtasks");

mod model;
mod props;
mod systemd;

use model::Task;
use repriseapp::ctxmenu::{Popup, Step, item};
use repriseapp::detailslist::{List, Press};
use repriseapp::oneshot::{self, ErrorBox};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Child, Ctx, Event, KeyInput, MAIN, PointerInput, Reply, WinId,
};
use reprisekit::access::{ROOT as ACCESS_ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{Column, Rows};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;
use reprisekit::tr;
use std::borrow::Cow;
use std::ffi::OsString;
use systemd::{Bus, Cmd, Update};

const CLIENT: (i32, i32) = (660, 380);
const MARGIN: i32 = 8;
const STATUS_Y: i32 = 40;
/// Where the list starts, at 100% text.
const LIST_Y: i32 = 60;

const PROPS: WinId = WinId::named("props");

const RUN: &str = "run";
const TOGGLE: &str = "toggle";
const PROPERTIES: &str = "properties";
/// New Task..., greyed: docs/programs.md says how to add a task.
const NEW: &str = "new";
const STATUS: &str = "status";

fn columns() -> [(String, i32); 5] {
    [
        (tr!("column-name"), 170),
        (tr!("column-schedule"), 180),
        (tr!("column-next"), 110),
        (tr!("column-last"), 110),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Run,
    Toggle,
    Properties,
}

struct SchedTasks {
    bus: Option<Bus>,
    tasks: Vec<Task>,
    /// Why the manager could not list the timers.
    unanswered: Option<String>,
    list: List<Lines>,
    form: Form,
    popup: Option<Popup<Act>>,
    note: ErrorBox,
    props: Option<props::Props>,
    big_icon: Option<Canvas>,
    is_12h: bool,
}

fn buttons() -> Vec<Item> {
    let mut x = MARGIN;
    let mut items = Vec::new();
    for (id, label, w) in [
        (RUN, tr!("run"), 85),
        (TOGGLE, tr!("disable"), 75),
        (PROPERTIES, tr!("properties"), 85),
        (NEW, tr!("new-task"), 85),
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
            form,
            popup: None,
            note: ErrorBox::default(),
            props: None,
            big_icon: None,
            is_12h: reprisefs::locale::is_12h(),
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

    /// Shows `tasks`, keeping the timer picked and the open sheet's.
    fn show(&mut self, tasks: Vec<Task>) {
        let picked = self.picked().map(|t| t.timer.clone());
        self.tasks = tasks;
        self.unanswered = None;
        let keep = picked.and_then(|p| self.tasks.iter().position(|t| t.timer == p));
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
            Act::Toggle if t.can_toggle() => self.send(Cmd::Turn {
                on: !t.on(),
                timer: t.timer,
                file_state: t.file_state,
            }),
            Act::Properties => {
                self.props = Some(props::Props::new(&t, self.big_icon.clone(), self.is_12h));
                return Reply::redraw_all();
            }
            _ => {}
        }
        Reply::redraw()
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
                Update::Refused(cmd, why) => {
                    let text = match cmd {
                        Cmd::Run(unit) => tr!("refused-run", unit = unit, why = why),
                        Cmd::Turn {
                            timer, on: true, ..
                        } => {
                            tr!("refused-enable", unit = timer, why = why)
                        }
                        Cmd::Turn { timer, .. } => tr!("refused-disable", unit = timer, why = why),
                        Cmd::Refresh => continue,
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
            (item(&toggle, t.can_toggle()), Some(Act::Toggle)),
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
            sym => {
                let moved = self.list.key(sym, r, ctx)?;
                self.sync();
                match moved {
                    true => Reply::redraw(),
                    false => Reply::none(),
                }
            }
        })
    }

    /// A pointer event over the list; None when it is elsewhere.
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
            PointerInput::Motion { x, y } => redraw(self.list.motion(x, y, r, ctx)),
            PointerInput::Scroll { x, y, rows } if r.contains(x, y) => {
                redraw(self.list.wheel(rows, r, ctx))
            }
            PointerInput::ScrollAcross { x, y, cols } if r.contains(x, y) => {
                redraw(self.list.wheel_across(cols, r, ctx))
            }
            PointerInput::Leave => redraw(self.list.leave(ctx)),
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
                return Reply::redraw();
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
        // A release ends a drag on the list's header or bars and presses a held button alike.
        let released = matches!(
            ev,
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT,
                ..
            })
        ) && self.list.release(ctx);
        match repriseapp::keys::form_input(&mut self.form, &ev, ctx) {
            Some(form::Event::Pressed(RUN)) => self.act(Act::Run),
            Some(form::Event::Pressed(TOGGLE)) => self.act(Act::Toggle),
            Some(form::Event::Pressed(PROPERTIES)) => self.act(Act::Properties),
            Some(form::Event::None) | None if !released => Reply::none(),
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

    fn load_icons(&mut self, ctx: &mut Ctx) {
        if self.list.view.rows.icon.is_none() {
            self.list.view.rows.icon = ctx.icons.get(ICON, 16);
            self.big_icon = ctx.icons.get(ICON, 32);
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
            w if self.note.owns(w) => self.note.event(w, &ev, &ctx.text, &ctx.forms()),
            _ => self.main_event(ev, ctx),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            PROPS => self.draw_props(c, ctx),
            w if self.note.owns(w) => self.note.draw(c, &forms, &ctx.text),
            _ => self.draw_main(c, ctx),
        }
    }
}

impl repriseapp::Extended for SchedTasks {
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        (win == MAIN).then_some((420, 200))
    }

    fn popup(&mut self, win: WinId, _ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win != MAIN {
            return None;
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

pub fn main(_args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(SchedTasks::new())).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
