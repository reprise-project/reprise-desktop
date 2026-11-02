//! reprise-diskusage FOLDER: a folder's entries largest first with a bar each, sizes filling in as
//! [`reprisefs::usage::Scan`] counts. docs/programs.md covers what the window does.

mod model;

use model::{Line, SHARE, SIZE};
use repriseapp::ctxmenu::{Popup, Step, item};
use repriseapp::detailslist::{List, Press};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Ctx, Event, KeyInput, MAIN, PointerInput, Poster, Reply, WinId,
};
use reprisefs::bincap::{self, BinLack, ConfirmKind};
use reprisefs::usage::{Kind, ROOT, Scan, Sort};
use reprisekit::access::{ROOT as ACCESS_ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::dlgtemplate::{self, FOLDER_TEXT_H, Template};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{Column, Rows};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::msgbox::Fields;
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;
use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

const CLIENT: (i32, i32) = (620, 460);
const MARGIN: i32 = 8;
const PATH_Y: i32 = 40;
const STATUS_Y: i32 = 58;
/// Where the list starts and how much room it leaves under it, at 100% text.
const LIST_Y: i32 = 78;
const COLUMNS: [(&str, i32, bool); 4] = [
    ("Name", 240, false),
    ("Size", 80, true),
    ("Share", 150, false),
    ("Files", 80, true),
];
const BUTTONS: [(&str, &str, i32); 4] = [
    ("back", "< &Back", 75),
    ("up", "&Up", 75),
    ("refresh", "&Refresh", 75),
    ("explore", "Open in &Explorer", 110),
];
const SCAN_THREADS: usize = 4;

const CONFIRM: WinId = WinId::named("confirm");
const NOTE: WinId = WinId::named("note");

/// The rows the list draws: the folder's entries and the icons of each kind.
#[derive(Default)]
struct Lines {
    lines: Vec<Line>,
    folder: Option<Canvas>,
    file: Option<Canvas>,
    drive: Option<Canvas>,
}

impl Rows for Lines {
    fn len(&self) -> usize {
        self.lines.len()
    }
    fn icon(&self, row: usize) -> Option<&Canvas> {
        match self.lines.get(row)?.kind {
            Kind::Dir => self.folder.as_ref(),
            Kind::Mount => self.drive.as_ref(),
            _ => self.file.as_ref(),
        }
    }
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        Cow::Owned(self.lines.get(row).map(|l| l.cell(col)).unwrap_or_default())
    }
    fn bar(&self, row: usize, col: usize) -> Option<f64> {
        (col == SHARE).then(|| self.lines.get(row).map_or(0.0, |l| l.share))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Open(PathBuf),
    Explore(PathBuf),
    Trash(PathBuf),
}

/// A Move to Recycle Bin's answer, from its thread.
type Trashed = (PathBuf, Result<(), String>);

struct DiskUsage {
    scan: Option<Scan>,
    /// The folder counted, and when its count started.
    root: PathBuf,
    started: Instant,
    /// The folder shown, at or under `root`.
    at: PathBuf,
    back: Vec<PathBuf>,
    sort: (Sort, bool),
    list: List<Lines>,
    form: Form,
    popup: Option<Popup<Act>>,
    confirm: Option<(PathBuf, MsgBox)>,
    note: Option<MsgBox>,
    trashed: Option<(Poster<Trashed>, Receiver<Trashed>)>,
    waker: Option<repriseapp::Waker>,
    trash_icon: Option<Canvas>,
}

fn buttons() -> Vec<Item> {
    let mut x = MARGIN;
    let mut items = Vec::new();
    for (id, label, w) in BUTTONS {
        items.push(Item::button(
            id,
            Rect::new(x, MARGIN, w, BUTTON_H),
            label,
            true,
            false,
        ));
        x += w + 6;
    }
    items.push(Item::label("path", Rect::new(MARGIN, PATH_Y, 2000, 13), ""));
    items.push(Item::label(
        "status",
        Rect::new(MARGIN, STATUS_Y, 2000, 13),
        "",
    ));
    items
}

impl DiskUsage {
    fn new(root: PathBuf) -> DiskUsage {
        let columns = COLUMNS
            .iter()
            .map(|&(t, w, right)| Column {
                right,
                ..Column::new(t, w)
            })
            .collect();
        let mut list = List::new(columns);
        list.view.frame = true;
        list.view.sort = Some((SIZE, false));
        let mut form = Form::new(buttons());
        form.set_focus(None);
        DiskUsage {
            scan: None,
            root: root.clone(),
            started: Instant::now(),
            at: root,
            back: Vec::new(),
            sort: (Sort::Size, false),
            list,
            form,
            popup: None,
            confirm: None,
            note: None,
            trashed: None,
            waker: None,
            trash_icon: None,
        }
    }

    /// Counts `root` afresh, showing it; the last count stops.
    fn rescan(&mut self, root: PathBuf) {
        self.scan = None;
        let Some(waker) = self.waker.clone() else {
            return;
        };
        let w = waker.clone();
        self.scan = Some(Scan::start(&root, SCAN_THREADS, move || w.wake()));
        self.started = Instant::now();
        // A folder that does not answer says so once the deadline passes.
        std::thread::spawn(move || {
            std::thread::sleep(reprisefs::deadline::ACTION_DEADLINE);
            waker.wake();
        });
        self.at = root.clone();
        self.root = root;
    }

    /// Shows `path`: a folder under the one counted from its count, any other counted afresh.
    fn go(&mut self, path: PathBuf, remember: bool) {
        if remember && path != self.at {
            self.back.push(self.at.clone());
        }
        // Another drive mounted inside is counted afresh: the count never enters it.
        let inside = (self.scan.as_ref()).is_some_and(|s| {
            let t = s.tree();
            t.find(&path).is_some_and(|id| t.node(id).kind == Kind::Dir)
        });
        if inside {
            self.at = path;
        } else {
            self.rescan(path);
        }
        self.list.view.top = 0;
        self.list.view.selection = Default::default();
    }

    fn up(&mut self) {
        if let Some(parent) = self.at.parent() {
            self.go(parent.to_path_buf(), true);
        }
    }

    /// Fills the rows, the path and the status from the count so far.
    fn sync(&mut self) {
        let picked = self.picked_path();
        let Some(scan) = &self.scan else {
            return;
        };
        let (answered, waited) = (
            scan.answered(),
            self.started.elapsed() >= reprisefs::deadline::ACTION_DEADLINE,
        );
        let t = scan.tree();
        let at = t.find(&self.at).unwrap_or(ROOT);
        self.list.view.rows.lines = model::lines(&t, at, self.sort.0, self.sort.1);
        let status = model::status(&t, at, answered, waited);
        drop(t);
        let keep = picked.and_then(|p| {
            let name = p.file_name()?.to_string_lossy().into_owned();
            self.list
                .view
                .rows
                .lines
                .iter()
                .position(|l| l.name == name)
        });
        self.list.view.selection = Default::default();
        self.list.view.selection.set_only(keep);
        self.form.set_text("path", &self.at.display().to_string());
        self.form.set_text("status", &status);
        self.form.set_enabled("back", !self.back.is_empty());
        self.form.set_enabled("up", self.at.parent().is_some());
    }

    fn picked(&self) -> Option<&Line> {
        self.list
            .selected()
            .and_then(|i| self.list.view.rows.lines.get(i))
    }

    fn picked_path(&self) -> Option<PathBuf> {
        self.picked().map(|l| self.at.join(&l.name))
    }

    fn list_rect(&self, ctx: &Ctx) -> Rect {
        let u = ctx.forms().text;
        let y = u.len(LIST_Y);
        let (w, h) = ctx.size;
        Rect::new(MARGIN, y, (w - 2 * MARGIN).max(0), (h - y - MARGIN).max(0))
    }

    fn act(&mut self, act: Act) -> Reply {
        match act {
            Act::Open(path) => {
                self.go(path, true);
                self.sync();
                Reply::redraw()
            }
            Act::Explore(path) => explore(&path),
            Act::Trash(path) => self.ask_trash(path),
        }
    }

    fn open_picked(&mut self) -> Reply {
        match (self.picked().filter(|l| l.opens()), self.picked_path()) {
            (Some(_), Some(path)) => self.act(Act::Open(path)),
            (None, Some(path)) => explore(&path),
            _ => Reply::none(),
        }
    }

    /// Move to Recycle Bin on `path`, asked first unless `[recycle_bin] confirm` is off.
    fn ask_trash(&mut self, path: PathBuf) -> Reply {
        if !bincap::Policy::load().confirm {
            self.trash(path);
            return Reply::redraw();
        }
        let is_dir = self.picked().is_some_and(|l| l.kind == Kind::Dir);
        let kind = ConfirmKind::of(1, is_dir);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let (title, text) = bincap::delete_prompt(kind, &name, 1, false, BinLack::None);
        let buttons = [dlgtemplate::YES, dlgtemplate::NO];
        let fields = Fields {
            text,
            icon: self.trash_icon.clone(),
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
            default: dlgtemplate::delete_default_index(&buttons, false),
            min_text_h: (kind == ConfirmKind::Folder).then_some(FOLDER_TEXT_H),
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        };
        let msg = MsgBox::on_template(title, Template::MessageBox, fields);
        self.confirm = Some((path, msg));
        Reply::redraw()
    }

    /// Moves `path` to its Recycle Bin on a thread of its own.
    fn trash(&mut self, path: PathBuf) {
        let Some((poster, _)) = &self.trashed else {
            return;
        };
        let poster = poster.clone();
        std::thread::spawn(move || {
            let done = match reprisefs::trash::Bins::system() {
                Some(bins) => bins.trash(&path).map(|_| ()),
                None => Err(std::io::Error::other("there is no Recycle Bin")),
            };
            let done = done.map_err(|e| reprisefs::dialogops::delete_failed_text(&path, &e));
            poster.post((path, done));
        });
    }

    fn wake(&mut self) -> Reply {
        let done: Vec<Trashed> = match &self.trashed {
            Some((_, rx)) => rx.try_iter().collect(),
            None => Vec::new(),
        };
        for (path, result) in done {
            match result {
                Ok(()) => {
                    if let Some(scan) = &self.scan {
                        let mut t = scan.tree();
                        if let Some(id) = t.find(&path) {
                            t.remove(id);
                        }
                    }
                }
                Err(text) => {
                    log::warn!("disk usage: {text}");
                    self.note = Some(note("Error Deleting File or Folder", text));
                }
            }
        }
        self.sync();
        Reply::redraw()
    }

    fn open_menu(&mut self, x: i32, y: i32, ctx: &Ctx, by_keyboard: bool) -> Reply {
        let (Some(line), Some(path)) = (self.picked().cloned(), self.picked_path()) else {
            return Reply::none();
        };
        let trashable = line.kind != Kind::Mount;
        let rows = vec![
            (
                item("&Open", line.opens()),
                line.opens().then(|| Act::Open(path.clone())),
            ),
            (
                item("Open in &Explorer", true),
                Some(Act::Explore(path.clone())),
            ),
            (MenuItem::Separator, None),
            (
                item("Move to &Recycle Bin", trashable),
                trashable.then_some(Act::Trash(path)),
            ),
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
        let alt = k.mods.alt;
        Some(match k.sym {
            keysym::BACKSPACE => self.button("back"),
            keysym::LEFT if alt => self.button("back"),
            keysym::UP if alt => self.button("up"),
            keysym::F5 => self.button("refresh"),
            keysym::RETURN | keysym::KP_ENTER => self.open_picked(),
            keysym::DELETE | keysym::KP_DELETE => match self.picked_path() {
                Some(p) if self.picked().is_some_and(|l| l.kind != Kind::Mount) => {
                    self.ask_trash(p)
                }
                _ => Reply::none(),
            },
            keysym::MENU => {
                let lay = self.list.layout(r, ctx);
                let row = self.list.selected().unwrap_or(0) as i32;
                let y = lay.body.y + (row - self.list.view.top as i32 + 1) * lay.row_h();
                self.open_menu(lay.body.x + 20, y, ctx, true)
            }
            _ if alt => return None,
            sym => match self.list.key(sym, r, ctx) {
                Some(true) => Reply::redraw(),
                Some(false) => Reply::none(),
                None => return None,
            },
        })
    }

    fn button(&mut self, id: &str) -> Reply {
        match id {
            "back" => {
                if let Some(p) = self.back.pop() {
                    self.go(p, false);
                }
            }
            "up" => self.up(),
            "refresh" => {
                let (root, at) = (self.root.clone(), self.at.clone());
                self.rescan(root);
                self.at = at;
            }
            "explore" => return explore(&self.at),
            _ => return Reply::none(),
        }
        self.sync();
        Reply::redraw()
    }

    /// A pointer event over the list; None when it is elsewhere.
    fn list_pointer(&mut self, ev: &PointerInput, ctx: &Ctx) -> Option<Reply> {
        let r = self.list_rect(ctx);
        let redraw = |b: bool| if b { Reply::redraw() } else { Reply::none() };
        Some(match *ev {
            PointerInput::Press {
                x, y, button, time, ..
            } if r.contains(x, y) => {
                let at = Click {
                    x,
                    y,
                    time,
                    double_ms: ctx.theme.metrics.double_click_ms,
                };
                match self.list.press(at, button, r, ctx) {
                    Press::Activate(_) => self.open_picked(),
                    Press::Row(_) if button == BTN_RIGHT => self.open_menu(x, y, ctx, false),
                    Press::None => Reply::none(),
                    _ => Reply::redraw(),
                }
            }
            PointerInput::Release { x, y, button, .. } if button == BTN_LEFT => {
                let held = self.list.view.header.held();
                let moved = self.list.release(ctx);
                let lay = self.list.layout(r, ctx);
                let over = self.list.view.hit(&lay, x, y).column();
                match held.filter(|&c| Some(c) == over).and_then(model::sort_of) {
                    Some(by) => {
                        let flipped = self.sort.0 == by && !self.sort.1;
                        self.sort = (by, flipped);
                        self.list.view.sort = Some((model::col_of(by), flipped));
                        self.sync();
                        Reply::redraw()
                    }
                    None => redraw(moved),
                }
            }
            PointerInput::Motion { x, y } => redraw(self.list.motion(x, y, r, ctx)),
            PointerInput::Scroll { x, y, rows } if r.contains(x, y) => {
                redraw(self.list.wheel(rows, r, ctx))
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
            Event::Wake => return self.wake(),
            Event::CloseRequest => {
                self.scan = None;
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
        match repriseapp::keys::form_input(&mut self.form, &ev, ctx) {
            Some(form::Event::Pressed(id)) => self.button(id),
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    fn confirm_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some((_, b)) = &mut self.confirm else {
            return Reply::none();
        };
        match b.event(&ev, &ctx.text, &ctx.forms()) {
            oneshot::Outcome::Closed(answer) => {
                let (path, _) = self.confirm.take().expect("open");
                if answer == Some(0) {
                    self.trash(path);
                }
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn note_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(b) = &mut self.note else {
            return Reply::none();
        };
        match b.event(&ev, &ctx.text, &ctx.forms()) {
            oneshot::Outcome::Closed(_) => {
                self.note = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn load_icons(&mut self, ctx: &mut Ctx) {
        let rows = &mut self.list.view.rows;
        if rows.folder.is_none() {
            rows.folder = ctx.icons.get("folder", 16);
            rows.file = ctx.icons.get("text-x-generic", 16);
            rows.drive = ctx.icons.get("drive-harddisk", 16);
            self.trash_icon = ctx.icons.get("user-trash-full", 32);
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
}

fn note(title: &str, text: String) -> MsgBox {
    MsgBox::new(
        title,
        Fields {
            text,
            icon: None,
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        },
    )
}

/// Explorer on folder `path`, or on the folder holding file `path` with it picked.
fn explore(path: &Path) -> Reply {
    let argv: Vec<OsString> = match path.is_dir() {
        true => vec!["reprise-explorer".into(), path.into()],
        false => vec!["reprise-explorer".into(), "--select".into(), path.into()],
    };
    Reply {
        spawn: vec![argv],
        ..Reply::none()
    }
}

impl App for DiskUsage {
    fn title(&self) -> String {
        "Disk Usage".into()
    }

    fn app_id(&self) -> String {
        "reprise.diskusage".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        repriseapp::start_forms().text.size(CLIENT)
    }

    fn children(&self) -> Vec<repriseapp::Child> {
        let forms = repriseapp::start_forms();
        let text = repriseapp::start_text();
        let mut out = Vec::new();
        if let Some((_, b)) = &self.confirm {
            out.push(b.child(CONFIRM, &text, &forms));
        }
        if let Some(b) = &self.note {
            out.push(b.child(NOTE, &text, &forms));
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.trashed = Some(Poster::channel(waker.clone()));
        self.waker = Some(waker);
        let root = self.root.clone();
        self.rescan(root);
        self.sync();
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.load_icons(ctx);
        match win {
            CONFIRM => self.confirm_event(ev, ctx),
            NOTE => self.note_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            CONFIRM => {
                if let Some((_, b)) = &self.confirm {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            NOTE => {
                if let Some(b) = &self.note {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            _ => self.draw_main(c, ctx),
        }
    }
}

impl repriseapp::Extended for DiskUsage {
    fn text_sized(&self) -> bool {
        true
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        (win == MAIN).then_some((420, 240))
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
            CONFIRM => {
                if let Some((_, b)) = &self.confirm {
                    b.access(tree, &ctx.text, &forms);
                }
            }
            NOTE => {
                if let Some(b) = &self.note {
                    b.access(tree, &ctx.text, &forms);
                }
            }
            _ => {
                let focus = self
                    .form
                    .access(tree, ACCESS_ROOT, &ctx.text, forms.fonts.size_px);
                let lay = self.list.layout(self.list_rect(ctx), ctx);
                let list = self.list.view.access(&lay, tree, ACCESS_ROOT, "usage");
                tree.set_focus(focus.unwrap_or(list));
            }
        }
    }
}

pub fn main(args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let root = args
        .into_iter()
        .next()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    repriseapp::run(Box::new(DiskUsage::new(root))).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
