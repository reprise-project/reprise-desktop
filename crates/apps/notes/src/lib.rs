#[cfg(test)]
reprisebus::test_session!();

mod input;
mod store;

reprisekit::strings!("notes");

use repriseapp::oneshot::{MsgBox, Outcome};
use repriseapp::{App, Caption, Child, Ctx, Event, Extended, MAIN, Reply, WinId};
use reprisebus::instance::{self, Claim};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::editmenu;
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::menu::MenuCtx;
use reprisekit::controls::msgbox;
use reprisekit::controls::multiedit::{self, Metrics, MultiEdit, View};
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::stickynote::{self, Colour, Strip};
use reprisekit::frame::Grip;
use reprisekit::look::Paint;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use reprisekit::tr;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use store::{Autosave, Saved};

const BUS_NAME: &str = "org.reprise.Notes";
/// What an autostart's start hands a copy already running: nothing to do.
const RESTORE_ACTION: &str = "restore";
const CONFIRM: WinId = WinId::named("confirm");
/// A new note's client size, logical px.
const NOTE_SIZE: (i32, i32) = (200, 180);
const MIN_NOTE: (i32, i32) = (120, 80);
/// The note text's px at 100% text size.
const TEXT_PX: u32 = 13;
/// The most of a note's first line its window title shows.
const TITLE_CHARS: usize = 40;

struct Note {
    id: String,
    win: WinId,
    edit: MultiEdit,
    view: View,
    colour: Colour,
    size: (i32, i32),
    strip: Strip,
    /// Where the held selection drag's pointer last was; ticks scroll from there.
    dragging: Option<(i32, i32)>,
    thumb: ThumbDrag,
    clicks: DoubleClick<()>,
    /// The window has the keyboard: its caret shows.
    focused: bool,
    save: Autosave,
}

impl Note {
    fn saved(&self) -> Saved {
        Saved {
            id: self.id.clone(),
            text: self.edit.text(),
            colour: self.colour,
            size: Some(self.size),
        }
    }

    fn is_empty(&self) -> bool {
        self.edit.lines().iter().all(|l| l.trim().is_empty())
    }

    fn title(&self) -> String {
        let first = self
            .edit
            .lines()
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty());
        match first {
            Some(l) => l.chars().take(TITLE_CHARS).collect(),
            None => tr!("untitled"),
        }
    }
}

struct Notes {
    /// Where the notes are kept; None (no home) keeps them for this run only.
    dir: Option<PathBuf>,
    notes: Vec<Note>,
    /// The number the next note's window takes ([`repriseapp::window_id`]).
    next_win: usize,
    metrics: Metrics,
    /// The right-click menu, open on note `menu_win`.
    menu: Option<editmenu::Menu>,
    menu_win: WinId,
    /// The delete prompt and the note it asks about.
    confirm: Option<(WinId, MsgBox)>,
    requests: Option<instance::Requests>,
    /// Started by autostart: with no notes kept, end at once.
    restoring: bool,
    /// The theme and font message boxes size their windows with, as of the last event.
    forms: Theme,
    dialog_text: Rc<TextRenderer>,
    warn_icon: Option<Canvas>,
}

impl Notes {
    fn new(dir: Option<PathBuf>, restoring: bool) -> Notes {
        let mut n = Notes {
            notes: Vec::new(),
            next_win: 0,
            metrics: Metrics {
                bars_always: false,
                flat: true,
                wrap: true,
                ..Metrics::default()
            },
            menu: None,
            menu_win: MAIN,
            confirm: None,
            requests: None,
            restoring,
            forms: Theme::default(),
            dialog_text: Rc::new(TextRenderer::load(&Default::default())),
            warn_icon: None,
            dir,
        };
        let saved = n.dir.as_deref().map(store::load).unwrap_or_default();
        for s in saved {
            n.add(s);
        }
        n
    }

    fn add(&mut self, s: Saved) -> WinId {
        let win = repriseapp::window_id(self.next_win);
        self.next_win += 1;
        self.notes.push(Note {
            id: s.id,
            win,
            edit: MultiEdit::new(&s.text),
            view: View::default(),
            colour: s.colour,
            size: s.size.unwrap_or(NOTE_SIZE),
            strip: Strip::default(),
            dragging: None,
            thumb: ThumbDrag::default(),
            clicks: DoubleClick::default(),
            focused: false,
            save: Autosave::default(),
        });
        win
    }

    fn note(&self, win: WinId) -> Option<&Note> {
        self.notes.iter().find(|n| n.win == win)
    }

    fn note_mut(&mut self, win: WinId) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.win == win)
    }

    /// A new empty note in `colour`, written at once so it is kept even if nothing is typed.
    fn new_note(&mut self, colour: Colour) -> WinId {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let taken = |id: &str| {
            self.notes.iter().any(|n| n.id == id)
                || self
                    .dir
                    .as_ref()
                    .is_some_and(|d| d.join(format!("{id}.txt")).exists())
        };
        let id = store::new_id(now, taken);
        let win = self.add(Saved {
            id,
            text: String::new(),
            colour,
            size: None,
        });
        self.write(win);
        win
    }

    fn write(&mut self, win: WinId) {
        let (Some(dir), Some(n)) = (&self.dir, self.note(win)) else {
            return;
        };
        if let Err(e) = store::write(dir, &n.saved()) {
            log::warn!("note {}: could not save: {e}", n.id);
        }
    }

    /// Note `win` changed: its write waits for a pause.
    fn changed(&mut self, win: WinId) {
        if let Some(n) = self.note_mut(win) {
            n.save.changed(Instant::now());
        }
    }

    /// Writes every note whose pause came; whether any still waits.
    fn save_due(&mut self, now: Instant) -> bool {
        let due: Vec<WinId> = (self.notes.iter_mut())
            .filter_map(|n| n.save.due(now).then_some(n.win))
            .collect();
        for w in due {
            self.write(w);
        }
        self.notes.iter().any(|n| n.save.pending())
    }

    /// Writes every note with a change waiting, now.
    fn save_all(&mut self) {
        let waiting: Vec<WinId> = (self.notes.iter_mut())
            .filter_map(|n| n.save.flush().then_some(n.win))
            .collect();
        for w in waiting {
            self.write(w);
        }
    }

    /// The x on `win`: an empty note goes at once, one with text asks first.
    fn ask_delete(&mut self, win: WinId) -> Reply {
        let Some(n) = self.note(win) else {
            return Reply::none();
        };
        if n.is_empty() {
            return self.delete(win);
        }
        self.confirm = Some((
            win,
            MsgBox::new(
                tr!("name"),
                msgbox::Fields {
                    text: tr!("confirm-delete"),
                    icon: self.warn_icon.clone(),
                    buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                    max_w: repriseapp::dialog_max_w(),
                },
            ),
        ));
        Reply::redraw_all()
    }

    /// Note `win` and its files go; the program ends with the last note.
    fn delete(&mut self, win: WinId) -> Reply {
        let Some(i) = self.notes.iter().position(|n| n.win == win) else {
            return Reply::none();
        };
        let n = self.notes.remove(i);
        if let Some(dir) = &self.dir {
            store::remove(dir, &n.id);
        }
        if self.menu_on(win) {
            self.menu = None;
        }
        Reply {
            quit: self.notes.is_empty(),
            ..Reply::redraw_all()
        }
    }

    /// Another start asked for a window: a new note, raised with its token. An autostart's
    /// start asks for nothing.
    fn requests(&mut self) -> Reply {
        let mut reply = Reply::none();
        while let Some(req) = self.requests.as_ref().and_then(instance::Requests::next) {
            if req
                .action
                .as_ref()
                .is_some_and(|(a, _)| a == RESTORE_ACTION)
            {
                continue;
            }
            let colour = self.notes.last().map_or(Colour::default(), |n| n.colour);
            let win = self.new_note(colour);
            reply = Reply {
                activate: Some((win, req.handoff)),
                ..Reply::redraw_all()
            };
        }
        reply
    }

    fn follow(&mut self, ctx: &Ctx) {
        self.forms = ctx.forms();
        self.dialog_text = ctx.text.clone();
        self.metrics.follow_look(&ctx.theme.look.controls());
        let px = reprisekit::theme::text_px(TEXT_PX, ctx.text_size);
        self.metrics.font_px = px;
        self.metrics.line_h = px as i32 + 3;
    }

    fn menu_on(&self, win: WinId) -> bool {
        self.menu.is_some() && self.menu_win == win
    }

    fn menu_ctx<'a>(&self, ctx: &'a Ctx) -> MenuCtx<'a> {
        MenuCtx {
            text: &ctx.text,
            theme: &ctx.theme,
            px: ctx.theme.fonts.size_px,
            within: ctx.menu_within(),
        }
    }

    fn client(ctx: &Ctx) -> Rect {
        Rect::new(0, 0, ctx.size.0, ctx.size.1)
    }

    fn edit_layout(&self, n: &Note, ctx: &Ctx) -> multiedit::Layout {
        let text = stickynote::layout(Notes::client(ctx)).text;
        multiedit::layout(&ctx.text, &self.metrics, &n.edit, text, 1)
    }

    fn draw_note(&self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let Some(n) = self.note(win) else {
            return;
        };
        let cs = stickynote::scheme(&ctx.theme.colors, n.colour);
        let paint = Paint {
            s: 1,
            cs: &cs,
            ..ctx.theme.paint()
        };
        stickynote::draw(c, &paint, Notes::client(ctx), n.colour, n.strip);
        let el = self.edit_layout(n, ctx);
        multiedit::draw(
            c,
            &ctx.text,
            &paint,
            &self.metrics,
            &n.edit,
            n.view,
            &el,
            reprisekit::textedit::Caret::new(n.focused, ctx.caret_on),
            None,
        );
        if let Some(menu) = self.menu.as_ref().filter(|_| self.menu_win == win) {
            menu.draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
        }
    }

    fn access_note(&self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let Some(n) = self.note(win) else {
            return;
        };
        stickynote::access(&stickynote::layout(Notes::client(ctx)), tree, ROOT);
        let el = self.edit_layout(n, ctx);
        let edit = multiedit::access(
            &n.edit,
            n.view,
            &el,
            &self.metrics,
            &ctx.text,
            tree,
            ROOT,
            "edit",
        );
        let menu = (self.menu.as_ref())
            .filter(|_| self.menu_win == win)
            .map(|m| m.access(tree, ROOT));
        tree.set_focus(menu.unwrap_or(edit));
    }

    fn confirm_event(&mut self, ev: Event) -> Reply {
        let Some((win, b)) = &mut self.confirm else {
            return Reply::none();
        };
        let win = *win;
        match b.event(&ev, &self.dialog_text, &self.forms) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Closed(pick) => {
                self.confirm = None;
                match pick {
                    Some(0) => self.delete(win),
                    _ => Reply::redraw_all(),
                }
            }
        }
    }
}

impl App for Notes {
    fn title(&self) -> String {
        tr!("name")
    }

    fn app_id(&self) -> String {
        "reprise.notes".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        NOTE_SIZE
    }

    fn children(&self) -> Vec<Child> {
        let mut kids: Vec<Child> = (self.notes.iter())
            .map(|n| Child {
                id: n.win,
                title: n.title(),
                size: n.size,
                fixed: false,
                modal: false,
            })
            .collect();
        if let Some((_, b)) = &self.confirm {
            kids.push(b.child(CONFIRM, &self.dialog_text, &self.forms));
        }
        kids
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        if let Some(r) = &self.requests {
            let w = waker.clone();
            r.wake_with(move || w.wake());
            waker.wake();
        }
        if self.notes.is_empty() {
            if self.restoring {
                return repriseapp::Start::Exit(0);
            }
            self.new_note(Colour::default());
        }
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.follow(ctx);
        if self.warn_icon.is_none() {
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
        }
        let reply = match (win, ev) {
            (_, Event::Wake) => self.requests(),
            (CONFIRM, ev) => self.confirm_event(ev),
            (MAIN, Event::CloseRequest) => Reply::redraw(),
            (_, Event::CloseRequest) => {
                self.save_all();
                Reply {
                    quit: true,
                    ..Reply::none()
                }
            }
            (w, ev) if self.note(w).is_some() => self.note_event(w, ev, ctx),
            _ => Reply::none(),
        };
        let pending = self.save_due(Instant::now());
        Reply {
            tick: reply.tick || (pending && self.note(win).is_some()),
            ..reply
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.follow(ctx);
        if win == CONFIRM {
            if let Some((_, b)) = &self.confirm {
                b.draw(c, &self.forms, &self.dialog_text);
            }
            return;
        }
        self.draw_note(win, c, ctx);
    }
}

impl Extended for Notes {
    fn hidden(&self) -> bool {
        true
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn parent(&self, win: WinId) -> Option<WinId> {
        match win {
            CONFIRM => self.confirm.as_ref().map(|(w, _)| *w),
            _ => None,
        }
    }

    fn caption(&self, win: WinId) -> Caption {
        match win {
            CONFIRM => Caption::Prompt,
            _ => Caption::None,
        }
    }

    fn grip(&self, win: WinId, (x, y): (i32, i32), (w, h): (i32, i32)) -> Option<Grip> {
        if self.note(win).is_none() || self.menu_on(win) {
            return None;
        }
        stickynote::grip(&stickynote::layout(Rect::new(0, 0, w, h)), x, y)
    }

    fn place_tag(&self, win: WinId) -> Option<String> {
        self.note(win).map(|n| format!("note-{}", n.id))
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        self.note(win).map(|_| MIN_NOTE)
    }

    fn emergency(&mut self) {
        self.save_all();
    }

    fn popup(&mut self, win: WinId, _ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        (self.menu_win == win).then(|| Box::new(&mut self.menu) as Box<dyn Hosted + '_>)
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.follow(ctx);
        if win == CONFIRM {
            if let Some((_, b)) = &self.confirm {
                b.access(tree, &self.dialog_text, &self.forms);
            }
            return;
        }
        self.access_note(win, tree, ctx);
    }
}

/// Runs the program; `args` are the command line after the program's name. `--restore` is the
/// login's start: it shows the kept notes, or ends at once when there are none.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let restoring = args.iter().any(|a| a == "--restore");
    let action = restoring.then(|| (RESTORE_ACTION.to_string(), Vec::new()));
    let requests = match instance::claim(BUS_NAME, action) {
        Claim::First(r) => Some(r),
        Claim::Handed => return,
        Claim::Alone => None,
    };
    let mut notes = Notes::new(store::dir(), restoring);
    notes.requests = requests;
    notes.forms = repriseapp::start_forms();
    notes.dialog_text = repriseapp::start_text();
    repriseapp::run(Box::new(notes)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
