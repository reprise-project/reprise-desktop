use repriseapp::keys::AltArm;
use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::{self, Column};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::{self, Action, Bar, Key};
use reprisekit::controls::shellview::size_text;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;

use crate::cups::{Job, JobOp, JobState, PrinterState, Snapshot};
use crate::list::{List, Press};
use crate::popup::{Popup, Step, item};

pub const CLIENT: (i32, i32) = (640, 300);

const DOC_W: i32 = 220;
const STATUS_W: i32 = 80;
const OWNER_W: i32 = 90;
const PAGES_W: i32 = 50;
const SIZE_W: i32 = 70;
const SUBMITTED_W: i32 = CLIENT.0 - (DOC_W + STATUS_W + OWNER_W + PAGES_W + SIZE_W);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    Job(i32, JobOp),
    SetDefault,
    Pause,
    Resume,
    CancelAll,
    Sharing,
    Properties,
}

pub fn cells(j: &Job, is_12h: bool) -> Vec<String> {
    vec![
        if j.name.is_empty() {
            "Untitled".into()
        } else {
            j.name.clone()
        },
        j.status_word().into(),
        j.owner.clone(),
        if j.pages > 0 {
            j.pages.to_string()
        } else {
            String::new()
        },
        if j.k_octets > 0 {
            size_text(j.k_octets as u64 * 1024)
        } else {
            String::new()
        },
        if j.created > 0 {
            reprisefs::listing::date_text(j.created, is_12h)
        } else {
            String::new()
        },
    ]
}

fn document_rows(j: Option<&Job>) -> Vec<(MenuItem, Option<Outcome>)> {
    let state = j.map(|j| j.state);
    let id = j.map_or(0, |j| j.id);
    let can_pause = matches!(state, Some(JobState::Pending | JobState::Printing));
    let can_resume = state == Some(JobState::Held);
    let can_restart = j.is_some_and(Job::restartable);
    let live = j.is_some();
    let cancel = j.map_or(JobOp::Cancel, cancel_op);
    let row =
        |label: &str, on: bool, op: JobOp| (item(label, on), on.then_some(Outcome::Job(id, op)));
    vec![
        row("&Pause", can_pause, JobOp::Pause),
        row("&Resume", can_resume, JobOp::Resume),
        row("Res&tart", can_restart, JobOp::Restart),
        row("&Cancel", live, cancel),
    ]
}

/// A finished job leaves the queue only when cupsd purges it.
fn cancel_op(j: &Job) -> JobOp {
    if j.state.done() {
        JobOp::Remove
    } else {
        JobOp::Cancel
    }
}

pub struct Queue {
    pub printer: String,
    pub icon: Option<std::rc::Rc<Canvas>>,
    pub list: List,
    ids: Vec<i32>,
    jobs: Vec<Job>,
    bar: Bar,
    acts: [Vec<Option<Outcome>>; 2],
    popup: Option<Popup<Outcome>>,
    pub alt_arm: AltArm,
    status: String,
    is_12h: bool,
}

impl Queue {
    /// The open menu (the bar's or the context menu), for the host's popups.
    pub fn popup(&mut self) -> Box<dyn Hosted + '_> {
        match self.bar.open {
            Some(_) => Box::new(&mut self.bar),
            None => Box::new(&mut self.popup),
        }
    }

    pub fn new(printer: &str) -> Queue {
        let mut q = Queue {
            printer: printer.into(),
            icon: None,
            list: List::new(vec![
                Column::new("Document Name", DOC_W),
                Column::new("Status", STATUS_W),
                Column::new("Owner", OWNER_W),
                Column::new("Pages", PAGES_W),
                Column::new("Size", SIZE_W),
                Column::new("Submitted", SUBMITTED_W),
            ]),
            ids: Vec::new(),
            jobs: Vec::new(),
            bar: Bar::new(Vec::new()),
            acts: [Vec::new(), Vec::new()],
            popup: None,
            alt_arm: AltArm::default(),
            status: String::new(),
            is_12h: reprisefs::locale::is_12h(),
        };
        q.refresh(&Snapshot::default());
        q
    }

    pub fn title(&self, s: &Snapshot) -> String {
        match s.printer(&self.printer) {
            Some(p) if p.state != PrinterState::Idle || !p.accepting => {
                format!("{} - {}", self.printer, p.status_word())
            }
            _ => self.printer.clone(),
        }
    }

    fn selected_job(&self) -> Option<&Job> {
        self.list.selected().and_then(|i| self.jobs.get(i))
    }

    pub fn refresh(&mut self, s: &Snapshot) {
        let keep = self.selected_job().map(|j| j.id);
        self.jobs = s.queue_of(&self.printer);
        self.ids = self.jobs.iter().map(|j| j.id).collect();
        self.list.view.rows = self
            .jobs
            .iter()
            .map(|j| listview::Row {
                icon: self.icon.clone(),
                cells: cells(j, self.is_12h),
            })
            .collect();
        let at = keep.and_then(|k| self.ids.iter().position(|i| *i == k));
        self.list.view.selection.set_only(at);
        let n = self.jobs.len();
        self.list.view.top = self.list.view.top.min(n.saturating_sub(1));
        let docs = if n == 1 { "document" } else { "documents" };
        self.status = format!("{n} {docs} in queue");
        self.set_menus(s);
    }

    fn set_menus(&mut self, s: &Snapshot) {
        if self.bar.open.is_some() {
            return;
        }
        let p = s.printer(&self.printer);
        let stopped = p.is_some_and(|p| p.state == PrinterState::Stopped);
        let is_default = s.default.as_deref() == Some(self.printer.as_str());
        let printer_rows: Vec<(MenuItem, Option<Outcome>)> = vec![
            (
                item("Set as De&fault Printer", !is_default),
                (!is_default).then_some(Outcome::SetDefault),
            ),
            if stopped {
                (item("Resume &Printing", true), Some(Outcome::Resume))
            } else {
                (item("Pa&use Printing", true), Some(Outcome::Pause))
            },
            (
                item("Ca&ncel All Documents", !self.jobs.is_empty()),
                (!self.jobs.is_empty()).then_some(Outcome::CancelAll),
            ),
            (item("S&haring...", true), Some(Outcome::Sharing)),
            (MenuItem::Separator, None),
            (item("P&roperties", true), Some(Outcome::Properties)),
            (MenuItem::Separator, None),
            (item("&Close", true), Some(Outcome::Close)),
        ];
        let doc_rows = document_rows(self.selected_job());
        let (pm, pa): (Vec<_>, Vec<_>) = printer_rows.into_iter().unzip();
        let (dm, da): (Vec<_>, Vec<_>) = doc_rows.into_iter().unzip();
        let keep = (self.bar.active, self.bar.underline, self.bar.hot);
        self.bar = Bar::new(vec![("&Printer", pm), ("&Document", dm)]);
        (self.bar.active, self.bar.underline, self.bar.hot) = keep;
        self.acts = [pa, da];
    }

    fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar
            .layout(
                &ctx.text,
                ctx.theme.fonts.size_px,
                Rect::new(0, 0, ctx.size.0, ctx.size.1),
                &ctx.theme.controls(),
            )
            .reaching(ctx.menu_within())
    }

    fn status_bar(&self, ctx: &Ctx) -> (StatusBar, Rect) {
        let bar = StatusBar {
            panels: vec![Panel {
                text: self.status.clone(),
                width: 0,
                pad: statusbar::EXPLORER_TEXT_PAD,
                icon: None,
            }],
            grip: true,
        };
        let h = statusbar::height(&ctx.theme.controls());
        (bar, Rect::new(0, ctx.size.1 - h, ctx.size.0, h))
    }

    fn list_rect(&self, ctx: &Ctx) -> Rect {
        let top = menubar::bar_h(&ctx.theme.controls()) + 1;
        let h = statusbar::height(&ctx.theme.controls());
        Rect::new(0, top, ctx.size.0, (ctx.size.1 - top - h).max(1))
    }

    pub fn fit_columns(&mut self, ctx: &Ctx) {
        let r = self.list_rect(ctx);
        self.list.fit_last(r, ctx);
        self.list.follow(r, ctx);
    }

    fn chosen(&mut self, title: usize, row: usize, s: &Snapshot) -> Outcome {
        let out = self
            .acts
            .get(title)
            .and_then(|a| a.get(row).cloned())
            .flatten()
            .unwrap_or(Outcome::Redraw);
        self.set_menus(s);
        out
    }

    fn popup_step(&mut self, step: Step<Outcome>) -> Outcome {
        match step {
            Step::Chosen(a) => {
                self.popup = None;
                a.unwrap_or(Outcome::Redraw)
            }
            Step::Closed => {
                self.popup = None;
                Outcome::Redraw
            }
            Step::Redraw => Outcome::Redraw,
            Step::None => Outcome::None,
        }
    }

    pub fn on_grip(&self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let (sb, r) = self.status_bar(ctx);
        statusbar::hit_grip(&sb, r, x, y)
    }

    pub fn press(&mut self, at: Click, button: u32, s: &Snapshot, ctx: &Ctx) -> Outcome {
        let (x, y) = (at.x, at.y);
        self.alt_arm.interrupt();
        if let Some(p) = self.popup.as_mut() {
            let step = p.press(x, y, ctx);
            return self.popup_step(step);
        }
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        if button == BTN_LEFT {
            match self.bar.press(x, y, &lay, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, row, .. } => return self.chosen(title, row, s),
                Action::Redraw => return Outcome::Redraw,
                Action::Ignored => {}
            }
        }
        let r = self.list_rect(ctx);
        if !r.contains(x, y) {
            return Outcome::None;
        }
        let press = self.list.press(at, button, r, ctx);
        self.set_menus(s);
        match press {
            Press::Row(i) if button == BTN_RIGHT => {
                let rows = document_rows(self.jobs.get(i));
                self.popup = Popup::open(rows, x, y, ctx, false);
                Outcome::Redraw
            }
            Press::Row(_) | Press::Activate(_) | Press::Background | Press::Redraw => {
                Outcome::Redraw
            }
            Press::None => Outcome::None,
        }
    }

    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Outcome {
        if let Some(p) = self.popup.as_mut() {
            return redraw(p.motion(x, y, ctx));
        }
        let lay = self.bar_layout(ctx);
        if self.bar.open.is_some() {
            let px = ctx.theme.fonts.size_px;
            self.bar.motion(x, y, &lay, &ctx.text, &ctx.theme, px);
            return Outcome::Redraw;
        }
        let title = self.bar.hover(x, y, &lay) && ctx.styled();
        let r = self.list_rect(ctx);
        redraw(self.list.motion(x, y, r, ctx) || title)
    }

    pub fn release(&mut self, ctx: &Ctx) -> Outcome {
        redraw(self.list.release(ctx))
    }

    pub fn leave(&mut self, ctx: &Ctx) -> Outcome {
        let title = self.bar.leave() && ctx.styled();
        redraw(self.list.leave(ctx) || title)
    }

    pub fn wheel(&mut self, rows: i32, ctx: &Ctx) -> Outcome {
        let r = self.list_rect(ctx);
        redraw(self.list.wheel(rows, r, ctx))
    }

    pub fn key(&mut self, k: &KeyInput, s: &Snapshot, ctx: &Ctx) -> Outcome {
        if let Some(p) = self.popup.as_mut() {
            let step = p.key(k, ctx);
            return self.popup_step(step);
        }
        if k.sym == keysym::ALT_L || k.sym == keysym::ALT_R {
            self.alt_arm.press_alt();
            return Outcome::None;
        }
        self.alt_arm.interrupt();
        if k.sym == keysym::MENU || (k.sym == keysym::F10 && k.mods.shift) {
            self.bar.close();
            return self.keyboard_menu(ctx);
        }
        let bar_key = match k.sym {
            keysym::F10 => Some(Key::Activate),
            keysym::ESCAPE => Some(Key::Escape),
            keysym::LEFT | keysym::KP_LEFT => Some(Key::Left),
            keysym::RIGHT | keysym::KP_RIGHT => Some(Key::Right),
            keysym::UP | keysym::KP_UP => Some(Key::Up),
            keysym::DOWN | keysym::KP_DOWN => Some(Key::Down),
            keysym::RETURN | keysym::KP_ENTER => Some(Key::Enter),
            _ => k
                .utf8
                .as_deref()
                .and_then(|s| s.chars().next())
                .filter(|c| c.is_alphanumeric())
                .map(|c| Key::Char(c, k.mods.alt)),
        };
        if let Some(bk) = bar_key {
            let lay = self.bar_layout(ctx);
            let px = ctx.theme.fonts.size_px;
            match self.bar.key(bk, &lay, &ctx.text, &ctx.theme, px) {
                Action::Chosen { title, row, .. } => return self.chosen(title, row, s),
                Action::Redraw => return Outcome::Redraw,
                Action::Ignored => {}
            }
        }
        let r = self.list_rect(ctx);
        if let Some(moved) = self.list.key(k.sym, r, ctx) {
            self.set_menus(s);
            return redraw(moved);
        }
        match k.sym {
            keysym::DELETE | keysym::KP_DELETE => match self.selected_job() {
                Some(j) => Outcome::Job(j.id, cancel_op(j)),
                None => Outcome::None,
            },
            keysym::ESCAPE => Outcome::Close,
            _ => Outcome::None,
        }
    }

    /// Shift+F10 or the Menu key: the selected document's menu under its row, its first row picked.
    fn keyboard_menu(&mut self, ctx: &Ctx) -> Outcome {
        let lay = self.list.layout(self.list_rect(ctx), ctx);
        let at = self.list.selected();
        let y = lay.body.y
            + at.map_or(0, |i| i.saturating_sub(self.list.view.top) as i32) * lay.row_h();
        let rows = document_rows(self.selected_job());
        self.popup = Popup::open(rows, lay.body.x + 20, y + lay.row_h(), ctx, true);
        Outcome::Redraw
    }

    pub fn key_up(&mut self, k: &KeyInput, ctx: &Ctx) -> Outcome {
        if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
            self.alt_arm.interrupt();
            return Outcome::None;
        }
        if !self.alt_arm.release_alt() {
            return Outcome::None;
        }
        let lay = self.bar_layout(ctx);
        let px = ctx.theme.fonts.size_px;
        self.bar.key(Key::Activate, &lay, &ctx.text, &ctx.theme, px);
        Outcome::Redraw
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        let r = self.list_rect(ctx);
        c.fill_rect(r, ctx.theme.colors.window);
        self.list.draw(c, r, ctx);
        let (sb, sr) = self.status_bar(ctx);
        let p = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        statusbar::draw(c, &ctx.text, &p, &sb, sr, ctx.theme.fonts.size_px);
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
        if let Some(p) = &self.popup {
            p.draw(c, ctx);
        }
    }

    /// A reader asked to see document `row`; true when the list moved.
    pub fn reveal(&mut self, row: usize, ctx: &Ctx) -> bool {
        let r = self.list_rect(ctx);
        self.list.reveal(row, r, ctx)
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let blay = self.bar_layout(ctx);
        let menus = self.bar.access(&blay, tree, ROOT);
        let r = self.list_rect(ctx);
        let lay = self.list.layout(r, ctx);
        let row = self.list.view.access(&lay, tree, ROOT, "documents");
        let (sb, sr) = self.status_bar(ctx);
        statusbar::access(&sb, sr, ctx.theme.text, tree, ROOT);
        let popup = self.popup.as_ref().map(|p| p.access(tree, ROOT));
        tree.set_focus(popup.or(menus).unwrap_or(row));
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

    /// A press that never completes a double click.
    fn one(x: i32, y: i32) -> Click {
        Click {
            x,
            y,
            time: 0,
            double_ms: 0,
        }
    }

    const PRINTER_MENU: usize = 0;
    const DOCUMENT_MENU: usize = 1;
    use crate::cups::Printer;

    fn job(id: i32, state: JobState) -> Job {
        Job {
            id,
            printer: "Laser".into(),
            name: format!("doc{id}.pdf"),
            owner: "ann".into(),
            state,
            k_octets: 12,
            pages: 3,
            created: 0,
            preserved: false,
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            running: true,
            printers: vec![Printer {
                name: "Laser".into(),
                accepting: true,
                ..Default::default()
            }],
            default: None,
            jobs: vec![
                job(1, JobState::Printing),
                job(2, JobState::Held),
                Job {
                    printer: "Other".into(),
                    ..job(3, JobState::Pending)
                },
                job(4, JobState::Stopped),
            ],
            ..Default::default()
        }
    }

    fn ctx() -> Ctx {
        crate::tests::test_ctx(CLIENT)
    }

    fn key(sym: u32) -> KeyInput {
        KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        }
    }

    fn row_point(q: &Queue, ctx: &Ctx, i: usize) -> (i32, i32) {
        let lay = q.list.layout(q.list_rect(ctx), ctx);
        (
            lay.body.x + 20,
            lay.body.y + lay.row_h() / 2 + i as i32 * lay.row_h(),
        )
    }

    #[test]
    fn rows_are_the_printers_own_documents() {
        let mut q = Queue::new("Laser");
        q.refresh(&snap());
        assert_eq!(q.ids, [1, 2, 4]);
        assert_eq!(
            q.list.view.rows[1].cells,
            ["doc2.pdf", "Paused", "ann", "3", "12 KB", ""]
        );
        assert_eq!(q.status, "3 documents in queue");
        assert_eq!(q.title(&snap()), "Laser");
        let mut s = snap();
        s.printers[0].state = PrinterState::Stopped;
        s.printers[0].reasons = vec!["paused".into()];
        assert_eq!(q.title(&s), "Laser - Paused");
    }

    #[test]
    fn the_document_rows_follow_the_jobs_state() {
        let on = |rows: &[(MenuItem, Option<Outcome>)]| -> Vec<bool> {
            rows.iter().map(|(_, a)| a.is_some()).collect()
        };
        assert_eq!(
            on(&document_rows(Some(&job(1, JobState::Printing)))),
            [true, false, false, true]
        );
        assert_eq!(
            on(&document_rows(Some(&job(2, JobState::Held)))),
            [false, true, false, true]
        );
        assert_eq!(
            on(&document_rows(Some(&job(4, JobState::Stopped)))),
            [false, false, true, true]
        );
        let kept = Job {
            preserved: true,
            ..job(5, JobState::Aborted)
        };
        assert_eq!(on(&document_rows(Some(&kept))), [false, false, true, true]);
        assert_eq!(
            on(&document_rows(Some(&job(5, JobState::Aborted)))),
            [false, false, false, true],
            "an aborted job without its files cannot restart"
        );
        assert_eq!(on(&document_rows(None)), [false, false, false, false]);
    }

    #[test]
    fn an_aborted_document_is_listed_restarts_and_cancels_by_purging() {
        let ctx = ctx();
        let mut s = snap();
        s.aborted = vec![Job {
            preserved: true,
            ..job(3, JobState::Aborted)
        }];
        let mut q = Queue::new("Laser");
        q.refresh(&s);
        assert_eq!(q.ids, [1, 2, 3, 4]);
        assert_eq!(q.list.view.rows[2].cells[1], "Error");
        assert_eq!(q.list.view.rows[3].cells[1], "Stopped");
        assert_eq!(q.status, "4 documents in queue");
        let (x, y) = row_point(&q, &ctx, 2);
        q.press(one(x, y), BTN_LEFT, &s, &ctx);
        assert_eq!(
            q.chosen(DOCUMENT_MENU, 2, &s),
            Outcome::Job(3, JobOp::Restart)
        );
        assert_eq!(
            q.key(&key(keysym::DELETE), &s, &ctx),
            Outcome::Job(3, JobOp::Remove)
        );
    }

    #[test]
    fn a_rows_menu_and_delete_act_on_its_document() {
        let ctx = ctx();
        let s = snap();
        let mut q = Queue::new("Laser");
        q.refresh(&s);
        let (x, y) = row_point(&q, &ctx, 1);
        assert_eq!(q.press(one(x, y), BTN_RIGHT, &s, &ctx), Outcome::Redraw);
        q.key(&key(keysym::DOWN), &s, &ctx);
        assert_eq!(
            q.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Job(2, JobOp::Resume)
        );
        assert_eq!(
            q.key(&key(keysym::DELETE), &s, &ctx),
            Outcome::Job(2, JobOp::Cancel)
        );
        assert_eq!(q.key(&key(keysym::ESCAPE), &s, &ctx), Outcome::Close);
    }

    #[test]
    fn shift_f10_opens_the_documents_menu_and_f10_alone_the_menu_bar() {
        let ctx = ctx();
        let s = snap();
        let mut q = Queue::new("Laser");
        q.refresh(&s);
        let (x, y) = row_point(&q, &ctx, 1);
        q.press(one(x, y), BTN_LEFT, &s, &ctx);
        assert_eq!(q.key(&key(keysym::F10), &s, &ctx), Outcome::Redraw);
        assert!(q.popup.is_none(), "F10 alone goes to the menu bar");
        assert!(q.bar.is_active());
        let mut shift_f10 = key(keysym::F10);
        shift_f10.mods.shift = true;
        assert_eq!(q.key(&shift_f10, &s, &ctx), Outcome::Redraw);
        assert!(!q.bar.is_active(), "the menu bar lets go");
        assert_eq!(
            q.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Job(2, JobOp::Resume),
            "the held document's first live row is picked"
        );
    }

    #[test]
    fn the_printer_menu_pauses_and_resumes() {
        let ctx = ctx();
        let mut s = snap();
        let mut q = Queue::new("Laser");
        q.refresh(&s);
        assert_eq!(q.chosen(PRINTER_MENU, 1, &s), Outcome::Pause);
        s.printers[0].state = PrinterState::Stopped;
        q.refresh(&s);
        assert_eq!(q.chosen(PRINTER_MENU, 1, &s), Outcome::Resume);
        assert_eq!(q.chosen(PRINTER_MENU, 7, &s), Outcome::Close);
        assert_eq!(q.key(&key(keysym::F10), &s, &ctx), Outcome::Redraw);
        q.key(&key(keysym::RETURN), &s, &ctx);
        assert_eq!(q.bar.open, Some(PRINTER_MENU));
        q.key(&key(keysym::RIGHT), &s, &ctx);
        assert_eq!(q.bar.open, Some(DOCUMENT_MENU));
    }

    #[test]
    #[ignore]
    fn render_queue() {
        let ctx = ctx();
        let mut q = Queue::new("Laser");
        q.refresh(&snap());
        q.list.view.selection.set_only(Some(0));
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        q.draw(&mut c, &ctx);
        reprisekit::raster::write_preview(&c, "printers-queue.png");
    }
}
