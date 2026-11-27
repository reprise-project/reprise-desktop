//! reprise-diskusage FOLDER: a folder's entries largest first with a bar each, sizes filling in as
//! [`reprisefs::usage::Scan`] counts. docs/programs.md covers what the window does.

#[cfg(test)]
reprisebus::test_session!();

reprisekit::strings!("diskusage");

mod cleanup;
mod model;

use cleanup::{Cleanup, Msg};
use model::{Line, SHARE};
use repriseapp::asks::{Answer, Asks};
use repriseapp::ctxmenu::{Popup, Step, item};
use repriseapp::detailslist::{List, Press};
use repriseapp::filedlg::{Driven, Host};
use repriseapp::oneshot::{ErrorBox, MsgBox};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Ctx, Event, KeyInput, MAIN, PointerInput, Poster, Reply, WinId,
};
use reprisefs::bincap::{self, BinLack, ConfirmKind};
use reprisefs::cleanup::{Found, Part, Places};
use reprisefs::props::short_size;
use reprisefs::usage::{Kind, ROOT, Scan, Sort};
use reprisekit::access::{ROOT as ACCESS_ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::dlgtemplate::{self, FOLDER_TEXT_H, Template};
use reprisekit::controls::filedlg::{self, DlgMode, FileDlg};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{Column, Rows};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::msgbox::Fields;
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;
use reprisekit::tr;
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
fn columns() -> [(String, i32, bool); 4] {
    [
        (tr!("column-name"), 240, false),
        (tr!("column-size"), 80, true),
        (tr!("column-share"), 150, false),
        (tr!("column-files"), 80, true),
    ]
}

fn button_rows() -> [(&'static str, String, i32); 6] {
    [
        ("back", tr!("back"), 75),
        ("up", tr!("up"), 75),
        ("browse", tr!("browse"), 75),
        ("refresh", tr!("refresh"), 75),
        ("explore", tr!("explore"), 110),
        ("cleanup", tr!("cleanup"), 85),
    ]
}
const SCAN_THREADS: usize = 4;

const CONFIRM: WinId = WinId::named("confirm");
const PICK: WinId = WinId::named("pick");
const CLEANUP: WinId = WinId::named("cleanup");
const CLEAN_CONFIRM: WinId = WinId::named("clean-confirm");
const CLEANED: WinId = WinId::named("cleaned");

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
    confirm: Asks<PathBuf>,
    note: ErrorBox,
    trashed: Option<(Poster<Trashed>, Receiver<Trashed>)>,
    waker: Option<repriseapp::Waker>,
    trash_icon: Option<Canvas>,
    /// Browse...'s folder dialog while it is up.
    pick: Option<FileDlg>,
    host: Host,
    warning_icon: Option<Canvas>,
    info_icon: Option<Canvas>,
    cleanup: Option<Cleanup>,
    /// The confirm box over Clean Up, with the parts it would remove.
    clean_ask: Asks<Vec<(Part, Found)>>,
    cleaned: Option<(Poster<Msg>, Receiver<Msg>)>,
    /// The box saying what a removal freed and what it could not remove.
    cleaned_box: Asks<()>,
    /// Clean Up opens as the window does (`--cleanup`).
    cleanup_at_start: bool,
    /// Counts the Clean Up boxes opened, so a count posted for a closed one is dropped.
    cleanups: u64,
    /// A removal is running: Clean Up stays grey until it ends.
    cleaning: bool,
}

fn buttons() -> Vec<Item> {
    let mut x = MARGIN;
    let mut items = Vec::new();
    for (id, label, w) in button_rows() {
        items.push(Item::button(
            id,
            Rect::new(x, MARGIN, w, BUTTON_H),
            &label,
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
        let columns = columns()
            .into_iter()
            .map(|(t, w, right)| Column {
                right,
                ..Column::new(&t, w)
            })
            .collect();
        let mut list = List::new(columns);
        list.view.frame = true;
        list.view.sort = Some(model::arrow(Sort::Size, false));
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
            confirm: Asks::new(CONFIRM),
            note: ErrorBox::default(),
            trashed: None,
            waker: None,
            trash_icon: None,
            pick: None,
            host: Host::default(),
            warning_icon: None,
            info_icon: None,
            cleanup: None,
            clean_ask: Asks::new(CLEAN_CONFIRM).over(CLEANUP),
            cleaned: None,
            cleaned_box: Asks::new(CLEANED),
            cleanup_at_start: false,
            cleanups: 0,
            cleaning: false,
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

    /// Browse...: a folder dialog on the folder shown, which shows the folder picked.
    fn browse(&mut self, ctx: &Ctx) {
        let fields = filedlg::Fields {
            title: tr!("title-browse"),
            accept_label: tr!("ok"),
            mode: DlgMode::Folder,
            multiple: false,
            filters: Vec::new(),
        };
        self.pick = Some(self.host.open(&self.at, fields, ctx.forms().text));
    }

    fn pick_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.pick.as_mut() else {
            return Reply::none();
        };
        let (text, forms) = (ctx.text.clone(), ctx.forms());
        match self.host.drive(dlg, &ev, &text, &forms, ctx) {
            Driven::Reply(r) => *r,
            Driven::Picked(path) => {
                self.pick = None;
                self.go(path, true);
                self.sync();
                Reply::redraw_all()
            }
            Driven::Cancelled => {
                self.pick = None;
                Reply::redraw_all()
            }
        }
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
        self.form.set_enabled("cleanup", !self.cleaning);
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
        let buttons = [dlgtemplate::yes(), dlgtemplate::no()];
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
        self.confirm.ask(msg, path);
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
                None => Err(std::io::Error::other(tr!("no-recycle-bin"))),
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
                    self.note.show(&tr!("title-delete-failed"), text, None);
                }
            }
        }
        let boxes = self.wake_cleanup();
        self.sync();
        match boxes {
            true => Reply::redraw_all(),
            false => Reply::redraw(),
        }
    }

    /// Opens Clean Up on `places`, counting its parts on a thread.
    fn open_cleanup(&mut self, places: Places) -> Reply {
        let Some((poster, _)) = &self.cleaned else {
            return Reply::none();
        };
        if self.cleanup.is_some() || self.cleaning {
            return Reply::none();
        }
        self.cleanups += 1;
        let post = poster.clone();
        let c = Cleanup::new(places.clone(), self.cleanups);
        cleanup::count(places, self.cleanups, c.stop.clone(), move |m| post.post(m));
        self.cleanup = Some(c);
        Reply::redraw_all()
    }

    /// Takes Clean Up's counts, and a removal's end: the folder is counted again and a box says
    /// what was freed and names what could not go. True when one came.
    fn wake_cleanup(&mut self) -> bool {
        let msgs: Vec<Msg> = match &self.cleaned {
            Some((_, rx)) => rx.try_iter().collect(),
            None => Vec::new(),
        };
        let came = !msgs.is_empty();
        for m in msgs {
            match m {
                Msg::Found(generation, part, found) => {
                    let open = self.cleanup.as_mut();
                    if let Some(c) = open.filter(|c| c.generation == generation) {
                        c.found(part, found);
                    }
                }
                Msg::Cleaned(done) => {
                    self.cleaning = false;
                    self.show_cleaned(&done);
                    let (root, at) = (self.root.clone(), self.at.clone());
                    self.rescan(root);
                    self.at = at;
                }
            }
        }
        came
    }

    /// The box after a removal: what it freed, then each part that failed with the reason.
    fn show_cleaned(&mut self, done: &[cleanup::Cleaned]) {
        let freed = done.iter().map(|c| c.freed).sum();
        let failed: Vec<String> = (done.iter())
            .filter_map(|c| {
                let reason = c.result.as_ref().err()?;
                Some(tr!(
                    "cleanup-failed-row",
                    part = cleanup::name(c.part, true),
                    reason = reason.as_str()
                ))
            })
            .collect();
        let mut lines = vec![tr!("cleanup-freed", size = short_size(freed))];
        let (icon, sound) = match failed.is_empty() {
            true => (self.info_icon.clone(), None),
            false => {
                lines.extend([String::new(), tr!("cleanup-failed"), String::new()]);
                lines.extend(failed);
                let warning = repriseapp::sound::Event::Warning;
                (self.warning_icon.clone(), Some(warning))
            }
        };
        let fields = Fields {
            text: lines.join("\n"),
            icon,
            buttons: vec![reprisekit::kit_tr!("ok")],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        };
        let msg = MsgBox::new(tr!("cleanup-title"), fields).sounding(sound);
        self.cleaned_box.ask(msg, ());
    }

    /// The confirm box naming what Clean Up's OK removes.
    fn ask_clean(&mut self) -> Reply {
        let Some(c) = &self.cleanup else {
            return Reply::none();
        };
        let parts: Vec<(Part, Found)> = (c.picked().into_iter())
            .map(|(p, f)| (p, f.clone()))
            .collect();
        if parts.is_empty() || !self.clean_ask.is_empty() {
            return Reply::none();
        }
        let buttons = [dlgtemplate::yes(), dlgtemplate::no()];
        let fields = Fields {
            text: c.confirm_text(),
            icon: self.warning_icon.clone(),
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
            default: dlgtemplate::delete_default_index(&buttons, true),
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        };
        let msg = MsgBox::on_template(tr!("cleanup-title"), Template::MessageBox, fields);
        self.clean_ask.ask(msg, parts);
        Reply::redraw_all()
    }

    fn cleanup_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if ev == Event::CloseRequest {
            self.cleanup = None;
            self.clean_ask.close();
            return Reply::redraw_all();
        }
        let Some(c) = &mut self.cleanup else {
            return Reply::none();
        };
        if matches!(
            ev,
            Event::Key(_) | Event::Pointer(PointerInput::Press { .. })
        ) {
            c.touched = true;
        }
        let pointed = match &ev {
            Event::Pointer(PointerInput::Motion { x, y }) => c.point(*x, *y, ctx.forms().text),
            _ => false,
        };
        let out = repriseapp::keys::form_input(&mut c.form, &ev, ctx);
        c.sync();
        match out {
            Some(form::Event::Cancel | form::Event::Pressed(cleanup::CANCEL)) => {
                self.cleanup = None;
                Reply::redraw_all()
            }
            Some(form::Event::Default | form::Event::Pressed(cleanup::OK)) => self.ask_clean(),
            Some(form::Event::None) | None if !pointed => Reply::none(),
            Some(_) | None => Reply::redraw(),
        }
    }

    fn clean_confirm_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        match self.clean_ask.event(win, &ev, &ctx.text, &ctx.forms()) {
            Answer::Closed(parts, answer) => self.clean_answered(parts, answer),
            Answer::Redraw => Reply::redraw(),
            Answer::None => Reply::none(),
        }
    }

    /// The confirm box's answer: Yes closes Clean Up and removes `parts`; anything else goes back
    /// to Clean Up.
    fn clean_answered(&mut self, parts: Vec<(Part, Found)>, answer: Option<usize>) -> Reply {
        if answer == Some(0)
            && let (Some(c), Some((poster, _))) = (self.cleanup.take(), &self.cleaned)
        {
            let post = poster.clone();
            self.cleaning = true;
            cleanup::clean(c.places.clone(), parts, move |m| post.post(m));
            self.sync();
        }
        Reply::redraw_all()
    }

    fn draw_cleanup(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(cl) = &self.cleanup {
            let paint = Paint {
                s: 1,
                ..forms.paint()
            };
            form::draw(c, &ctx.text, &paint, &cl.form, true, forms.fonts.size_px);
        }
    }

    fn open_menu(&mut self, x: i32, y: i32, ctx: &Ctx, by_keyboard: bool) -> Reply {
        let (Some(line), Some(path)) = (self.picked().cloned(), self.picked_path()) else {
            return Reply::none();
        };
        let trashable = line.kind != Kind::Mount;
        let rows = vec![
            (
                item(&tr!("menu-open"), line.opens()),
                line.opens().then(|| Act::Open(path.clone())),
            ),
            (
                item(&tr!("menu-explore"), true),
                Some(Act::Explore(path.clone())),
            ),
            (MenuItem::Separator, None),
            (
                item(&tr!("menu-trash"), trashable),
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
            keysym::BACKSPACE => self.button("back", ctx),
            keysym::LEFT if alt => self.button("back", ctx),
            keysym::UP if alt => self.button("up", ctx),
            keysym::F5 => self.button("refresh", ctx),
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
            _ => match self.list.key(k, r, ctx) {
                Some(true) => Reply::redraw(),
                Some(false) => Reply::none(),
                None => return None,
            },
        })
    }

    fn button(&mut self, id: &str, ctx: &Ctx) -> Reply {
        match id {
            "browse" => {
                self.browse(ctx);
                return Reply::redraw_all();
            }
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
            "cleanup" => return self.open_cleanup(Places::system()),
            _ => return Reply::none(),
        }
        self.sync();
        Reply::redraw()
    }

    /// A press or wheel turn over the list, or the release that clicks a column title; None for
    /// the rest, which the form sees too.
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
            PointerInput::Release { button, .. } if button == BTN_LEFT => {
                let by = model::sort_of(self.list.view.header.held()?)?;
                let released = self.list.release(ctx);
                if released.column.is_none() {
                    return Some(redraw(released.redraw));
                }
                let flipped = self.sort.0 == by && !self.sort.1;
                self.sort = (by, flipped);
                self.list.view.sort = Some(model::arrow(by, flipped));
                self.sync();
                Reply::redraw()
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
        // The list follows every motion, release and leave, and the form sees them too: a press
        // on one of its buttons acts on the release, and only while the pointer is still on it.
        let lit = match &ev {
            Event::Pointer(p) => {
                let r = self.list_rect(ctx);
                self.list.follow_pointer(p, r, ctx).redraw
            }
            _ => false,
        };
        match repriseapp::keys::form_input(&mut self.form, &ev, ctx) {
            Some(form::Event::Pressed(id)) => self.button(id, ctx),
            Some(form::Event::None) | None if !lit => Reply::none(),
            Some(_) | None => Reply::redraw(),
        }
    }

    fn confirm_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        match self.confirm.event(win, &ev, &ctx.text, &ctx.forms()) {
            Answer::Closed(path, answer) => {
                if answer == Some(0) {
                    self.trash(path);
                }
                Reply::redraw_all()
            }
            Answer::Redraw => Reply::redraw(),
            Answer::None => Reply::none(),
        }
    }

    fn load_icons(&mut self, ctx: &mut Ctx) {
        let rows = &mut self.list.view.rows;
        if rows.folder.is_none() {
            rows.folder = ctx.icons.get("folder", 16);
            rows.file = ctx.icons.get("text-x-generic", 16);
            rows.drive = ctx.icons.get("drive-harddisk", 16);
            self.trash_icon = ctx.icons.get("user-trash-full", 32);
            self.warning_icon = ctx.icons.get("dialog-warning", 32);
            self.info_icon = ctx.icons.get("dialog-information", 32);
        }
        self.host.icons.prime(&mut ctx.icons);
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
        tr!("name")
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
        out.extend(self.confirm.child(&text, &forms));
        out.extend(self.note.child(&text, &forms));
        out.extend(self.cleaned_box.child(&text, &forms));
        if let Some(d) = &self.pick {
            out.push(repriseapp::Child {
                id: PICK,
                title: tr!("title-browse"),
                size: d.client(),
                fixed: false,
                modal: true,
            });
        }
        if let Some(c) = &self.cleanup {
            out.push(repriseapp::Child {
                id: CLEANUP,
                title: tr!("cleanup-title"),
                size: forms.text.size(cleanup::client(c.parts.len())),
                fixed: true,
                modal: true,
            });
        }
        out.extend(self.clean_ask.child(&text, &forms));
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.trashed = Some(Poster::channel(waker.clone()));
        self.cleaned = Some(Poster::channel(waker.clone()));
        self.waker = Some(waker);
        let root = self.root.clone();
        self.rescan(root);
        if self.cleanup_at_start {
            self.open_cleanup(Places::system());
        }
        self.sync();
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.load_icons(ctx);
        match win {
            w if self.confirm.owns(w) => self.confirm_event(w, ev, ctx),
            w if self.note.owns(w) => self.note.event(w, &ev, &ctx.text, &ctx.forms()),
            w if self.cleaned_box.owns(w) => {
                match self.cleaned_box.event(w, &ev, &ctx.text, &ctx.forms()) {
                    Answer::Closed(..) => Reply::redraw_all(),
                    Answer::Redraw => Reply::redraw(),
                    Answer::None => Reply::none(),
                }
            }
            PICK => self.pick_event(ev, ctx),
            w if self.clean_ask.owns(w) => self.clean_confirm_event(w, ev, ctx),
            CLEANUP => self.cleanup_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            w if self.confirm.owns(w) => self.confirm.draw(c, &forms, &ctx.text),
            w if self.note.owns(w) => self.note.draw(c, &forms, &ctx.text),
            w if self.cleaned_box.owns(w) => self.cleaned_box.draw(c, &forms, &ctx.text),
            PICK => {
                if let Some(d) = &self.pick {
                    let (w, h) = d.client();
                    let paint = Paint {
                        s: 1,
                        ..forms.paint()
                    };
                    let r = Rect::new(0, 0, w, h);
                    d.draw(c, &ctx.text, &paint, r, forms.fonts.size_px);
                }
            }
            w if self.clean_ask.owns(w) => self.clean_ask.draw(c, &forms, &ctx.text),
            CLEANUP => self.draw_cleanup(c, ctx),
            _ => self.draw_main(c, ctx),
        }
    }
}

impl repriseapp::Extended for DiskUsage {
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match win {
            MAIN => Some((550, 240)),
            PICK => self.pick.as_ref().map(FileDlg::min_client),
            _ => None,
        }
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<repriseapp::FileDrop<'_>> {
        if win != PICK {
            return None;
        }
        Some(repriseapp::FileDrop {
            dlg: self.pick.as_mut()?,
            host: &mut self.host,
        })
    }

    fn parent(&self, win: WinId) -> Option<WinId> {
        self.clean_ask.parent(win).or(Some(MAIN))
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
            w if self.confirm.owns(w) => self.confirm.access(tree, &ctx.text, &forms),
            w if self.note.owns(w) => self.note.access(tree, &ctx.text, &forms),
            w if self.cleaned_box.owns(w) => self.cleaned_box.access(tree, &ctx.text, &forms),
            PICK => {
                if let Some(d) = &self.pick {
                    let (w, h) = d.client();
                    let r = Rect::new(0, 0, w, h);
                    d.access(tree, r, &ctx.text, forms.fonts.size_px);
                }
            }
            w if self.clean_ask.owns(w) => self.clean_ask.access(tree, &ctx.text, &forms),
            CLEANUP => {
                if let Some(c) = &self.cleanup {
                    repriseapp::oneshot::access_form(&c.form, tree, &ctx.text, forms.fonts.size_px);
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

/// `reprise-diskusage [--cleanup] [FOLDER]`: whether Clean Up opens over the count, and the
/// folder when it is absolute.
fn parse_args(args: Vec<OsString>) -> (bool, Option<PathBuf>) {
    let mut args = args.into_iter().peekable();
    let cleanup = args.next_if(|a| a == "--cleanup").is_some();
    let root = args.next().map(PathBuf::from).filter(|p| p.is_absolute());
    (cleanup, root)
}

pub fn main(args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let (cleanup_at_start, root) = parse_args(args);
    let root = root
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let d = DiskUsage {
        cleanup_at_start,
        ..DiskUsage::new(root)
    };
    repriseapp::run(Box::new(d)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
