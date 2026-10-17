//! Printers: a task pane beside a Details list of the CUPS printers, in client coordinates.

use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput};
use reprisekit::access::{ROOT, Tree};
use reprisekit::art::printbadge::{self, Badge};
use reprisekit::controls::listview::{self, Column};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::taskpane::{self, Body, Section, TaskPane};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::{TextEdit, keysym};

use crate::cups::{Printer, PrinterState, Snapshot};
use crate::list::{List, Press};
use crate::popup::{Popup, Step, item};

pub const CLIENT: (i32, i32) = (900, 480);

const PANE_W: i32 = 200;
const LIST_X: i32 = 210;
const LIST_Y: i32 = 1;

const NAME_W: i32 = 170;
const DOCS_W: i32 = 84;
const STATUS_W: i32 = 130;
const COMMENT_W: i32 = 110;
const LOCATION_W: i32 = 100;
const MODEL_W: i32 = CLIENT.0 - LIST_X - (NAME_W + DOCS_W + STATUS_W + COMMENT_W + LOCATION_W);

const TASKS: usize = 0;
const DETAILS: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Add,
    Refresh,
    Open(String),
    SetDefault(String),
    Pause(String),
    Resume(String),
    CancelAll(String),
    Sharing(String),
    Properties(String),
    Delete(String),
    Rename { old: String, new: String },
    BadName(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Out(Outcome),
    Rename,
}

fn badges(p: &Printer, default: bool) -> Vec<Badge> {
    let mut b = Vec::new();
    if default {
        b.push(Badge::Default);
    }
    if p.state == PrinterState::Stopped {
        b.push(if p.paused() {
            Badge::Paused
        } else {
            Badge::Error
        });
    }
    b
}

pub fn cells(p: &Printer, s: &Snapshot) -> Vec<String> {
    let docs = s.jobs_of(&p.name).count();
    vec![
        p.name.clone(),
        docs.to_string(),
        p.status_word(),
        p.info.clone(),
        p.location.clone(),
        p.make_model.clone(),
    ]
}

fn actions(p: &Printer, s: &Snapshot, menu: bool) -> Vec<(String, bool, Option<Act>)> {
    let name = &p.name;
    let out = |o: Outcome| Some(Act::Out(o));
    let is_default = s.default.as_deref() == Some(name.as_str());
    let jobs = s.jobs_of(name).count() > 0;
    let stopped = p.state == PrinterState::Stopped;
    let mut rows = Vec::new();
    if menu {
        rows.push(("&Open".to_string(), true, out(Outcome::Open(name.clone()))));
        rows.push((String::new(), false, None));
        rows.push((
            "Set as De&fault Printer".into(),
            !is_default,
            out(Outcome::SetDefault(name.clone())),
        ));
        rows.push(if stopped {
            (
                "Resume &Printing".into(),
                true,
                out(Outcome::Resume(name.clone())),
            )
        } else {
            (
                "Pa&use Printing".into(),
                true,
                out(Outcome::Pause(name.clone())),
            )
        });
        rows.push((
            "Ca&ncel All Documents".into(),
            jobs,
            out(Outcome::CancelAll(name.clone())),
        ));
        rows.push((
            "S&haring...".into(),
            true,
            out(Outcome::Sharing(name.clone())),
        ));
        rows.push((String::new(), false, None));
        rows.push(("&Delete".into(), true, out(Outcome::Delete(name.clone()))));
        rows.push(("Rena&me".into(), true, Some(Act::Rename)));
        rows.push((String::new(), false, None));
        rows.push((
            "P&roperties".into(),
            true,
            out(Outcome::Properties(name.clone())),
        ));
        return rows;
    }
    rows.push((
        "See what's printing".into(),
        true,
        out(Outcome::Open(name.clone())),
    ));
    if !is_default {
        rows.push((
            "Set as default printer".into(),
            true,
            out(Outcome::SetDefault(name.clone())),
        ));
    }
    rows.push(if stopped {
        (
            "Resume printing".into(),
            true,
            out(Outcome::Resume(name.clone())),
        )
    } else {
        (
            "Pause printing".into(),
            true,
            out(Outcome::Pause(name.clone())),
        )
    });
    rows.push((
        "Cancel all documents".into(),
        jobs,
        out(Outcome::CancelAll(name.clone())),
    ));
    rows.push((
        "Share this printer".into(),
        true,
        out(Outcome::Sharing(name.clone())),
    ));
    rows.push(("Rename this printer".into(), true, Some(Act::Rename)));
    rows.push((
        "Delete this printer".into(),
        true,
        out(Outcome::Delete(name.clone())),
    ));
    rows.push((
        "Set printer properties".into(),
        true,
        out(Outcome::Properties(name.clone())),
    ));
    rows
}

fn details(p: Option<&Printer>, s: &Snapshot) -> Vec<(String, bool)> {
    if !s.running {
        return vec![
            ("The printing service (CUPS) is not running".into(), true),
            (String::new(), false),
            (
                "Install cups and enable cups.socket to manage printers.".into(),
                false,
            ),
        ];
    }
    match p {
        Some(p) => {
            let docs = s.jobs_of(&p.name).count();
            let mut l = vec![(p.name.clone(), true)];
            if s.default.as_deref() == Some(p.name.as_str()) {
                l.push(("Default printer".into(), false));
            }
            l.push((format!("Status: {}", p.status_word()), false));
            l.push((format!("Documents: {docs}"), false));
            if !p.make_model.is_empty() {
                l.push((format!("Model: {}", p.make_model), false));
            }
            if !p.location.is_empty() {
                l.push((format!("Location: {}", p.location), false));
            }
            l
        }
        None if s.printers.is_empty() => vec![
            ("Printers".into(), true),
            (String::new(), false),
            ("There are no printers yet. Add one to print.".into(), false),
        ],
        None => vec![("Printers".into(), true), ("System Folder".into(), false)],
    }
}

/// A name a printer can take: spaces turn into underscores (cupsd takes
/// none), and the characters it refuses are refused. `Ok(None)` for an
/// empty or unchanged name, which renames nothing.
pub fn check_name(old: &str, typed: &str, s: &Snapshot) -> Result<Option<String>, String> {
    let name = typed.trim().replace(' ', "_");
    if name.is_empty() || name == old {
        return Ok(None);
    }
    if name.len() > 127 {
        return Err("A printer name can be at most 127 characters long.".into());
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | '#' | '?' | '\'' | '"' | '\t'))
    {
        return Err("A printer name cannot contain any of these characters: / \\ # ? ' \"".into());
    }
    if s.printers
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(&name) && p.name != old)
    {
        return Err(format!("There is already a printer called {name}."));
    }
    Ok(Some(name))
}

pub struct Folder {
    pub pane: TaskPane,
    pub list: List,
    names: Vec<String>,
    popup: Option<Popup<Act>>,
    tasks: Vec<Option<Act>>,
    popup_for: Option<String>,
}

impl Default for Folder {
    fn default() -> Folder {
        Folder::new()
    }
}

impl Folder {
    pub fn new() -> Folder {
        Folder {
            pane: TaskPane {
                sections: vec![
                    Section::links("Printer Tasks", Vec::new()),
                    Section::lines("Details", Vec::new()),
                ],
                ..Default::default()
            },
            list: List::new(vec![
                Column::new("Name", NAME_W),
                Column::new("Documents", DOCS_W),
                Column::new("Status", STATUS_W),
                Column::new("Comments", COMMENT_W),
                Column::new("Location", LOCATION_W),
                Column::new("Model", MODEL_W),
            ]),
            names: Vec::new(),
            popup: None,
            tasks: Vec::new(),
            popup_for: None,
        }
    }

    pub fn refresh(&mut self, s: &Snapshot, icon: Option<&Canvas>) {
        let keep = self.selected_name().map(str::to_string);
        self.names = s.printers.iter().map(|p| p.name.clone()).collect();
        self.list.view.rows = s
            .printers
            .iter()
            .map(|p| {
                let is_default = s.default.as_deref() == Some(p.name.as_str());
                listview::Row {
                    icon: icon
                        .map(|i| std::rc::Rc::new(printbadge::apply(i, &badges(p, is_default)))),
                    cells: cells(p, s),
                }
            })
            .collect();
        let at = keep.and_then(|k| self.names.iter().position(|n| *n == k));
        self.list.view.selection.set_only(at);
        let n = self.list.view.rows.len();
        self.list.view.top = self.list.view.top.min(n.saturating_sub(1));
        // A rename box whose printer went away goes with it.
        if self.list.view.edit.as_ref().is_some_and(|(i, _)| *i >= n) {
            self.list.view.edit = None;
        }
        self.set_pane(s);
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.list
            .selected()
            .and_then(|i| self.names.get(i))
            .map(String::as_str)
    }

    fn selected<'a>(&self, s: &'a Snapshot) -> Option<&'a Printer> {
        s.printer(self.selected_name()?)
    }

    pub fn select_name(&mut self, name: &str, ctx: &Ctx, s: &Snapshot) {
        let i = self.names.iter().position(|n| n == name);
        let (_, r) = self.layout(ctx);
        self.list.select(i, r, ctx);
        self.set_pane(s);
    }

    fn set_pane(&mut self, s: &Snapshot) {
        let mut rows = vec![taskpane::Row {
            icon: None,
            text: "Add a printer".into(),
            enabled: s.running,
        }];
        let mut acts = vec![Some(Act::Out(Outcome::Add))];
        if let Some(p) = self.selected(s) {
            for (text, enabled, act) in actions(p, s, false) {
                rows.push(taskpane::Row {
                    icon: None,
                    text,
                    enabled,
                });
                acts.push(act);
            }
        }
        self.tasks = acts;
        self.pane.sections[TASKS].body = Body::Links(rows);
        self.pane.sections[DETAILS].body = Body::Lines(details(self.selected(s), s));
        self.pane.hot = None;
    }

    /// The task pane and the list, the pane widening with the text.
    pub fn layout(&self, ctx: &Ctx) -> (Rect, Rect) {
        let (w, h) = ctx.size;
        let (pane_w, list_x) = (ctx.theme.text.len(PANE_W), ctx.theme.text.len(LIST_X));
        (
            Rect::new(0, 0, pane_w, h),
            Rect::new(list_x, LIST_Y, (w - list_x).max(1), (h - LIST_Y).max(1)),
        )
    }

    pub fn fit_columns(&mut self, ctx: &Ctx) {
        let (_, r) = self.layout(ctx);
        self.list.fit_last(r, ctx);
        self.list.follow(r, ctx);
    }

    fn pane_layout(&self, ctx: &Ctx) -> taskpane::Layout {
        let (pane_r, _) = self.layout(ctx);
        taskpane::layout(
            &self.pane,
            &ctx.text,
            ctx.theme.fonts.size_px,
            pane_r,
            &ctx.theme.controls(),
        )
    }

    pub fn begin_rename(&mut self) -> Outcome {
        let Some(i) = self.list.selected() else {
            return Outcome::None;
        };
        let Some(name) = self.names.get(i) else {
            return Outcome::None;
        };
        self.list.view.edit = Some((i, TextEdit::new(name, true)));
        Outcome::Redraw
    }

    fn commit_rename(&mut self, s: &Snapshot) -> Outcome {
        let Some((i, ed)) = self.list.view.edit.take() else {
            return Outcome::None;
        };
        let Some(old) = self.names.get(i).cloned() else {
            return Outcome::Redraw;
        };
        match check_name(&old, &ed.text, s) {
            Ok(None) => Outcome::Redraw,
            Ok(Some(new)) => Outcome::Rename { old, new },
            Err(why) => Outcome::BadName(why),
        }
    }

    fn act(&mut self, act: Option<Act>) -> Outcome {
        match act {
            Some(Act::Out(o)) => o,
            Some(Act::Rename) => self.begin_rename(),
            None => Outcome::Redraw,
        }
    }

    pub fn press(&mut self, at: Click, button: u32, s: &Snapshot, ctx: &Ctx) -> Outcome {
        let (x, y) = (at.x, at.y);
        if let Some(p) = self.popup.as_mut() {
            return match p.press(x, y, ctx) {
                Step::Chosen(a) => {
                    self.popup = None;
                    self.act(a)
                }
                Step::Closed => {
                    self.popup = None;
                    Outcome::Redraw
                }
                Step::Redraw => Outcome::Redraw,
                Step::None => Outcome::None,
            };
        }
        let (pane_r, list_r) = self.layout(ctx);
        if let Some((i, _)) = &self.list.view.edit {
            let lay = self.list.layout(list_r, ctx);
            let r = self
                .list
                .view
                .edit_rect(&lay, *i, &ctx.text, ctx.theme.fonts.size_px);
            if r.contains(x, y) {
                return Outcome::None;
            }
            let out = self.commit_rename(s);
            if out != Outcome::Redraw {
                return out;
            }
        }
        if list_r.contains(x, y) {
            let press = self.list.press(at, button, list_r, ctx);
            self.set_pane(s);
            return match press {
                Press::Row(i) if button == BTN_RIGHT => {
                    self.open_menu(Some(i), x, y, s, ctx, false);
                    Outcome::Redraw
                }
                Press::Row(_) => Outcome::Redraw,
                Press::Activate(i) => match self.names.get(i) {
                    Some(n) => Outcome::Open(n.clone()),
                    None => Outcome::Redraw,
                },
                Press::Background => {
                    if button == BTN_RIGHT {
                        self.open_menu(None, x, y, s, ctx, false);
                    }
                    Outcome::Redraw
                }
                Press::Redraw => Outcome::Redraw,
                Press::None => Outcome::None,
            };
        }
        if pane_r.contains(x, y) && button == BTN_LEFT {
            let lay = self.pane_layout(ctx);
            let hit = taskpane::hit(&self.pane, &lay, x, y);
            return match self.pane.click(hit) {
                taskpane::Event::Row(TASKS, j) => {
                    let a = self.tasks.get(j).cloned().flatten();
                    self.act(a)
                }
                taskpane::Event::None => Outcome::None,
                _ => Outcome::Redraw,
            };
        }
        Outcome::None
    }

    fn open_menu(
        &mut self,
        row: Option<usize>,
        x: i32,
        y: i32,
        s: &Snapshot,
        ctx: &Ctx,
        by_keyboard: bool,
    ) {
        let rows: Vec<(MenuItem, Option<Act>)> = match row.and_then(|i| self.names.get(i)) {
            Some(name) => match s.printer(name) {
                Some(p) => actions(p, s, true)
                    .into_iter()
                    .map(|(label, enabled, act)| {
                        if label.is_empty() {
                            (MenuItem::Separator, None)
                        } else {
                            let bold = label == "&Open";
                            (item(&label, enabled).bold(bold), act.filter(|_| enabled))
                        }
                    })
                    .collect(),
                None => Vec::new(),
            },
            None => vec![
                (
                    item("&Add Printer", s.running),
                    s.running.then_some(Act::Out(Outcome::Add)),
                ),
                (item("R&efresh", true), Some(Act::Out(Outcome::Refresh))),
            ],
        };
        self.popup_for = row.and_then(|i| self.names.get(i).cloned());
        self.popup = Popup::open(rows, x, y, ctx, by_keyboard);
    }

    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Outcome {
        if let Some(p) = self.popup.as_mut() {
            return redraw(p.motion(x, y, ctx));
        }
        let (pane_r, list_r) = self.layout(ctx);
        let moved = self.list.motion(x, y, list_r, ctx);
        let hit = pane_r.contains(x, y).then(|| {
            let lay = self.pane_layout(ctx);
            taskpane::hit(&self.pane, &lay, x, y)
        });
        let header = match hit {
            Some(taskpane::Hit::Header(i)) => Some(i),
            _ => None,
        };
        let link = match hit {
            Some(taskpane::Hit::Row(i, j)) => Some((i, j)),
            _ => None,
        };
        let lit = self.pane.hot_header != header || self.pane.hot != link;
        self.pane.hot_header = header;
        self.pane.hot = link;
        redraw(moved || lit)
    }

    pub fn release(&mut self, ctx: &Ctx) -> Outcome {
        redraw(self.list.release(ctx))
    }

    pub fn leave(&mut self, ctx: &Ctx) -> Outcome {
        let lit = self.pane.hot.is_some() || self.pane.hot_header.is_some();
        self.pane.hot = None;
        self.pane.hot_header = None;
        redraw(self.list.leave(ctx) || lit)
    }

    pub fn wheel(&mut self, rows: i32, ctx: &Ctx) -> Outcome {
        let (_, r) = self.layout(ctx);
        redraw(self.list.wheel(rows, r, ctx))
    }

    pub fn key(&mut self, k: &KeyInput, s: &Snapshot, ctx: &Ctx) -> Outcome {
        if let Some(p) = self.popup.as_mut() {
            return match p.key(k, ctx) {
                Step::Chosen(a) => {
                    self.popup = None;
                    self.act(a)
                }
                Step::Closed => {
                    self.popup = None;
                    Outcome::Redraw
                }
                Step::Redraw => Outcome::Redraw,
                Step::None => Outcome::None,
            };
        }
        if self.list.view.edit.is_some() {
            match k.sym {
                keysym::RETURN | keysym::KP_ENTER | keysym::TAB => return self.commit_rename(s),
                keysym::ESCAPE => {
                    self.list.view.edit = None;
                    return Outcome::Redraw;
                }
                _ => {}
            }
            if let (Some(key), Some((_, ed))) = (k.edit.clone(), self.list.view.edit.as_mut()) {
                ed.key(key);
            }
            return Outcome::Redraw;
        }
        let (_, r) = self.layout(ctx);
        if let Some(moved) = self.list.key(k.sym, r, ctx) {
            self.set_pane(s);
            return redraw(moved);
        }
        let name = self.selected_name().map(str::to_string);
        match (k.sym, name) {
            (keysym::RETURN | keysym::KP_ENTER, Some(n)) if k.mods.alt => Outcome::Properties(n),
            (keysym::RETURN | keysym::KP_ENTER, Some(n)) => Outcome::Open(n),
            (keysym::DELETE | keysym::KP_DELETE, Some(n)) => Outcome::Delete(n),
            (keysym::F2, Some(_)) => self.begin_rename(),
            (keysym::F5, _) => Outcome::Refresh,
            (keysym::MENU, _) => self.menu_key(r, s, ctx),
            (keysym::F10, _) if k.mods.shift => self.menu_key(r, s, ctx),
            _ => Outcome::None,
        }
    }

    fn menu_key(&mut self, r: Rect, s: &Snapshot, ctx: &Ctx) -> Outcome {
        let lay = self.list.layout(r, ctx);
        let at = self.list.selected();
        let y = at.map_or(lay.body.y, |i| {
            lay.body.y + i.saturating_sub(self.list.view.top) as i32 * lay.row_h()
        });
        self.open_menu(at, lay.body.x + 20, y + lay.row_h(), s, ctx, true);
        Outcome::Redraw
    }

    pub fn editing(&self) -> bool {
        self.list.view.edit.is_some()
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        let (pane_r, list_r) = self.layout(ctx);
        c.fill_rect(c.bounds(), ctx.theme.colors.window);
        let pl = self.pane_layout(ctx);
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        taskpane::draw(
            c,
            &ctx.text,
            &paint,
            &self.pane,
            &pl,
            ctx.theme.fonts.size_px,
        );
        taskpane::draw_gap(
            c,
            &paint,
            &pl,
            Rect::new(
                pane_r.right(),
                pane_r.y,
                list_r.x - pane_r.right(),
                pane_r.height,
            ),
        );
        self.list.draw(c, list_r, ctx);
        if let Some(p) = &self.popup {
            p.draw(c, ctx);
        }
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let (_, list_r) = self.layout(ctx);
        let pl = self.pane_layout(ctx);
        taskpane::access(&self.pane, &pl, tree, ROOT);
        let lay = self.list.layout(list_r, ctx);
        let row = self.list.view.access(&lay, tree, ROOT, "printers");
        let popup = self.popup.as_ref().map(|p| p.access(tree, ROOT));
        tree.set_focus(popup.unwrap_or(row));
    }
}

fn redraw(changed: bool) -> Outcome {
    if changed {
        Outcome::Redraw
    } else {
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cups::{Job, JobState};

    /// A press that never completes a double click.
    fn one(x: i32, y: i32) -> Click {
        Click {
            x,
            y,
            time: 0,
            double_ms: 0,
        }
    }

    pub(crate) fn printer(name: &str) -> Printer {
        Printer {
            name: name.into(),
            info: format!("{name} comment"),
            location: "Office".into(),
            make_model: "Generic IPP Everywhere".into(),
            accepting: true,
            ..Default::default()
        }
    }

    fn snap() -> Snapshot {
        let mut paused = printer("Laser");
        paused.state = PrinterState::Stopped;
        paused.reasons = vec!["paused".into()];
        Snapshot {
            running: true,
            printers: vec![printer("Inkjet"), paused],
            default: Some("Inkjet".into()),
            jobs: vec![Job {
                id: 4,
                printer: "Laser".into(),
                owner: "ann".into(),
                state: JobState::Held,
                ..Default::default()
            }],
        }
    }

    fn ctx() -> Ctx {
        crate::tests::test_ctx(CLIENT)
    }

    fn folder(icon: Option<&Canvas>) -> Folder {
        let mut f = Folder::new();
        f.refresh(&snap(), icon);
        f
    }

    fn row_point(f: &Folder, ctx: &Ctx, i: usize) -> (i32, i32) {
        let (_, r) = f.layout(ctx);
        let lay = f.list.layout(r, ctx);
        (
            lay.body.x + 20,
            lay.body.y + lay.row_h() / 2 + i as i32 * lay.row_h(),
        )
    }

    fn key(sym: u32) -> KeyInput {
        KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        }
    }

    fn task_texts(f: &Folder) -> Vec<(String, bool)> {
        match &f.pane.sections[TASKS].body {
            Body::Links(rows) => rows.iter().map(|r| (r.text.clone(), r.enabled)).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rows_show_documents_status_and_the_rest() {
        let f = folder(None);
        let cells: Vec<Vec<String>> = f.list.view.rows.iter().map(|r| r.cells.clone()).collect();
        assert_eq!(
            cells[1],
            [
                "Laser",
                "1",
                "Paused",
                "Laser comment",
                "Office",
                "Generic IPP Everywhere"
            ]
        );
        assert_eq!(cells[0][1], "0");
        assert_eq!(cells[0][2], "Ready");
    }

    #[test]
    fn the_icons_wear_the_default_and_state_badges() {
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), reprisekit::theme::Color::rgb(0, 0, 255));
        let f = folder(Some(&icon));
        let inkjet = f.list.view.rows[0].icon.as_deref().unwrap();
        let laser = f.list.view.rows[1].icon.as_deref().unwrap();
        assert_eq!(
            inkjet,
            &printbadge::apply(&icon, &[Badge::Default]),
            "the default printer's check"
        );
        assert_eq!(laser, &printbadge::apply(&icon, &[Badge::Paused]));
        let mut err = printer("x");
        err.state = PrinterState::Stopped;
        err.reasons = vec!["media-empty-error".into()];
        assert_eq!(badges(&err, false), [Badge::Error]);
    }

    #[test]
    fn the_tasks_follow_the_selection() {
        let ctx = ctx();
        let s = snap();
        let mut f = folder(None);
        assert_eq!(task_texts(&f), [("Add a printer".to_string(), true)]);
        let (x, y) = row_point(&f, &ctx, 1);
        assert_eq!(f.press(one(x, y), BTN_LEFT, &s, &ctx), Outcome::Redraw);
        let texts: Vec<String> = task_texts(&f).into_iter().map(|t| t.0).collect();
        assert_eq!(
            texts,
            [
                "Add a printer",
                "See what's printing",
                "Set as default printer",
                "Resume printing",
                "Cancel all documents",
                "Share this printer",
                "Rename this printer",
                "Delete this printer",
                "Set printer properties"
            ]
        );
        let (x, y) = row_point(&f, &ctx, 0);
        f.press(one(x, y), BTN_LEFT, &s, &ctx);
        let t = task_texts(&f);
        assert!(!t.iter().any(|(s, _)| s == "Set as default printer"));
        assert!(t.contains(&("Cancel all documents".to_string(), false)));
        assert!(t.contains(&("Pause printing".to_string(), true)));
        match &f.pane.sections[DETAILS].body {
            Body::Lines(l) => {
                assert_eq!(l[0], ("Inkjet".to_string(), true));
                assert_eq!(l[1], ("Default printer".to_string(), false));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_double_click_on_a_printer_opens_it() {
        let ctx = ctx();
        let s = snap();
        let mut f = folder(None);
        let (x, y) = row_point(&f, &ctx, 1);
        let at = |time| ctx.theme.metrics.click(x, y, time);
        assert_eq!(f.press(at(1000), BTN_LEFT, &s, &ctx), Outcome::Redraw);
        assert_eq!(
            f.press(at(1100), BTN_LEFT, &s, &ctx),
            Outcome::Open("Laser".into())
        );
    }

    #[test]
    fn a_task_link_acts_on_the_selection() {
        let ctx = ctx();
        let s = snap();
        let mut f = folder(None);
        let (x, y) = row_point(&f, &ctx, 1);
        f.press(one(x, y), BTN_LEFT, &s, &ctx);
        let lay = f.pane_layout(&ctx);
        let r = lay.rows[TASKS][3];
        assert_eq!(
            f.press(one(r.x + 4, r.y + 4), BTN_LEFT, &s, &ctx),
            Outcome::Resume("Laser".into())
        );
        let r = lay.rows[TASKS][6];
        assert_eq!(
            f.press(one(r.x + 4, r.y + 4), BTN_LEFT, &s, &ctx),
            Outcome::Redraw
        );
        assert!(f.editing(), "Rename this printer opened the box");
    }

    #[test]
    fn the_context_menu_acts_and_greys_what_does_not_apply() {
        let ctx = ctx();
        let s = snap();
        let mut f = folder(None);
        let (x, y) = row_point(&f, &ctx, 0);
        assert_eq!(f.press(one(x, y), BTN_RIGHT, &s, &ctx), Outcome::Redraw);
        assert!(f.popup.is_some());
        f.key(&key(keysym::DOWN), &s, &ctx);
        f.key(&key(keysym::DOWN), &s, &ctx);
        assert_eq!(
            f.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Pause("Inkjet".into())
        );
        assert!(f.popup.is_none());
        let (_, r) = f.layout(&ctx);
        f.press(one(r.x + 40, r.bottom() - 20), BTN_RIGHT, &s, &ctx);
        f.key(&key(keysym::DOWN), &s, &ctx);
        assert_eq!(f.key(&key(keysym::RETURN), &s, &ctx), Outcome::Add);
    }

    #[test]
    fn keys_open_delete_and_rename() {
        let ctx = ctx();
        let s = snap();
        let mut f = folder(None);
        assert_eq!(f.key(&key(keysym::DOWN), &s, &ctx), Outcome::Redraw);
        assert_eq!(
            f.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Open("Inkjet".into())
        );
        assert_eq!(
            f.key(&key(keysym::DELETE), &s, &ctx),
            Outcome::Delete("Inkjet".into())
        );
        let mut alt = key(keysym::RETURN);
        alt.mods.alt = true;
        assert_eq!(f.key(&alt, &s, &ctx), Outcome::Properties("Inkjet".into()));
        assert_eq!(f.key(&key(keysym::F2), &s, &ctx), Outcome::Redraw);
        if let Some((_, ed)) = f.list.view.edit.as_mut() {
            *ed = TextEdit::new("Front Desk", false);
        }
        assert_eq!(
            f.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Rename {
                old: "Inkjet".into(),
                new: "Front_Desk".into()
            }
        );
        assert!(!f.editing());
        f.key(&key(keysym::F2), &s, &ctx);
        assert_eq!(f.key(&key(keysym::ESCAPE), &s, &ctx), Outcome::Redraw);
        assert!(!f.editing());
    }

    #[test]
    fn names_are_checked_before_a_rename() {
        let s = snap();
        assert_eq!(check_name("Inkjet", "Inkjet", &s), Ok(None));
        assert_eq!(check_name("Inkjet", "  ", &s), Ok(None));
        assert_eq!(
            check_name("Inkjet", "Big Office", &s),
            Ok(Some("Big_Office".into()))
        );
        assert!(check_name("Inkjet", "a/b", &s).is_err());
        assert!(
            check_name("Inkjet", "laser", &s).is_err(),
            "taken, in any case"
        );
        assert!(check_name("Inkjet", &"x".repeat(128), &s).is_err());
    }

    #[test]
    fn details_say_when_cups_is_not_running() {
        let mut f = Folder::new();
        f.refresh(&Snapshot::default(), None);
        assert_eq!(task_texts(&f), [("Add a printer".to_string(), false)]);
        match &f.pane.sections[DETAILS].body {
            Body::Lines(l) => assert_eq!(l[0].0, "The printing service (CUPS) is not running"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    #[ignore]
    fn render_folder() {
        let ctx = ctx();
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(1, 4, 14, 8), ctx.theme.colors.button_shadow);
        let mut f = folder(Some(&icon));
        f.list.view.selection.set_only(Some(1));
        f.set_pane(&snap());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        f.draw(&mut c, &ctx);
        reprisekit::raster::write_preview(&c, "printers-folder.png");
    }
}
