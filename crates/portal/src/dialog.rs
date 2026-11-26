//! The portal's dialogs. Each runs as our own child (`reprise-portal --pick`,
//! `--permit <kind>`, `--access`, `--file`, `--appchoose`), so
//! its Wayland connection closes with it and a toolkit failure cannot take
//! the daemon down. `run_toml` is the parent's end for most: the child reads
//! one TOML document on stdin (`read_input`), including the asking window's
//! handle, and opens owned by and modal to that window. `--appchoose` keeps
//! stdin open through `run_live`: the request and each refill are NUL-separated
//! TOML documents, read with `docs`. `Prompt` is the child's window: a 32 px
//! icon at (12, 12), two text lines from x 54 on lines 12 and 38, and accept
//! and cancel buttons at the bottom right.

use repriseapp::keys::form_input;
use repriseapp::{App, BTN_LEFT, Ctx, Event, PointerInput, Reader, Reply, Waker, WinId};
use reprisekit::access::Tree;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::formscroll::FormScroll;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;
use reprisekit::theme::Theme;
use std::io::{BufRead, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

/// Run our own binary with `args`, write `input` to its stdin and return its
/// stdout once it exits 0. None when it fails to start or exits non-zero, or
/// when `cancel` is set (a frontend Close), which kills it. Blocks until the child is gone.
pub fn run(args: &[&str], input: &str, cancel: &AtomicBool) -> Option<String> {
    run_cmd(own_exe(), args, input, None, cancel)
}

/// `run`, keeping stdin open: `input` and each document arriving on `updates`
/// are written with a trailing NUL until the child exits or the sender drops.
pub fn run_live(
    args: &[&str],
    input: &str,
    updates: Receiver<String>,
    cancel: &AtomicBool,
) -> Option<String> {
    run_cmd(own_exe(), args, input, Some(updates), cancel)
}

/// Our running image through /proc/self/exe, which still execs after an
/// upgrade replaced the file, where the path would be " (deleted)".
fn own_exe() -> Command {
    let mut cmd = Command::new("/proc/self/exe");
    cmd.arg0("reprise-portal");
    cmd
}

/// `run` for any command. Stdout is read on a thread so a large answer never
/// stalls the child; with `updates`, stdin stays open as in `run_live`.
fn run_cmd(
    mut cmd: Command,
    args: &[&str],
    input: &str,
    updates: Option<Receiver<String>>,
    cancel: &AtomicBool,
) -> Option<String> {
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = spawn_tied(cmd)
        .map_err(|e| log::warn!("dialog {args:?}: {e}"))
        .ok()?;
    let mut stdin = child.stdin.take();
    if let Some(s) = stdin.as_mut() {
        let _ = s.write_all(input.as_bytes());
        if updates.is_some() {
            let _ = s.write_all(b"\0");
        }
    }
    if updates.is_none() {
        stdin = None;
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut out).map(|_| out)
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if let Some(rx) = &updates {
                    loop {
                        match rx.try_recv() {
                            Ok(doc) => {
                                if let Some(s) = stdin.as_mut() {
                                    let _ = s.write_all(doc.as_bytes());
                                    let _ = s.write_all(b"\0");
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                stdin = None;
                                break;
                            }
                        }
                    }
                }
                if cancel.load(Ordering::SeqCst) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    };
    let out = reader.join().ok()?.ok()?;
    status.success().then_some(out)
}

/// Stdin's NUL-separated documents in order, empty and non-UTF-8 ones skipped.
pub fn docs(r: impl BufRead) -> impl Iterator<Item = String> {
    r.split(0)
        .map_while(Result::ok)
        .filter_map(|d| String::from_utf8(d).ok())
        .filter(|d| !d.is_empty())
}

/// A caller's text for a one-line place: control characters become spaces.
pub fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A GTK mnemonic label as a form label: `&` is doubled, `_x` makes x the
/// access key, `__` is a literal `_`.
pub fn mnemonic(label: &str) -> String {
    let label = one_line(label);
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str("&&"),
            '_' if chars.peek() == Some(&'_') => {
                chars.next();
                out.push('_');
            }
            '_' => out.push('&'),
            c => out.push(c),
        }
    }
    out
}

/// The xdg-foreign handle after `wayland:` in `parent_window`; anything else
/// gives None and the dialog opens ownerless.
pub fn parent_handle(parent_window: &str) -> Option<String> {
    parent_window
        .strip_prefix("wayland:")
        .filter(|h| !h.is_empty())
        .map(str::to_string)
}

pub fn run_toml<T: serde::Serialize>(
    args: &[&str],
    input: &T,
    cancel: &AtomicBool,
) -> Option<String> {
    let text = toml::to_string(input)
        .map_err(|e| log::warn!("dialog {args:?} input: {e}"))
        .ok()?;
    run(args, &text, cancel)
}

pub fn read_input<T: serde::de::DeserializeOwned>() -> Option<T> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).ok()?;
    toml::from_str(&text)
        .map_err(|e| log::warn!("dialog input: {e}"))
        .ok()
}

/// Spawn `cmd` with SIGTERM as its parent-death signal, so a dialog left by a
/// killed portal closes. The signal follows the spawning thread, which polls
/// until the child exits, so it fires only when the portal dies.
fn spawn_tied(mut cmd: Command) -> std::io::Result<Child> {
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: the closure runs in the forked child before exec and calls
    // only prctl, getppid and _exit, which are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    cmd.spawn()
}

pub const TEXT_X: i32 = 54;
pub const ICON_PX: i32 = 32;
pub const ICON_AT: (i32, i32) = (12, 12);
/// The header's two lines' tops.
pub const PROMPT_Y: i32 = 12;
pub const NOTE_Y: i32 = 38;

/// The theme a prompt's form draws with, for building it before `repriseapp::run`: what
/// `Ctx::forms` gives once it runs.
pub fn start_forms() -> Theme {
    repriseapp::start_theme().with_form_text(repriseconf::current().accessibility.text_size)
}

/// The icon theme repriseapp loads. A prompt resolves its icons before its
/// window opens, since the first frame is drawn before any event.
pub fn icons() -> IconCache {
    IconCache::new(IconTheme::load(
        &std::env::var(reprisekit::icontheme::ENV).unwrap_or_else(|_| "hicolor".into()),
        &IconTheme::default_roots(),
    ))
}

/// The two lines beside the header icon in a client `w` wide: `prompt` on
/// line 12, `note` on line 38, both ending 11 px from the right. Both are
/// plain text: an `&` (in an app's name) draws as itself.
pub fn header(w: i32, prompt: String, note: String) -> [Item; 2] {
    let line = |id, y, text: String| {
        Item::new(
            id,
            Rect::new(TEXT_X, y, w - TEXT_X - 11, LINE_H),
            Control::Label {
                text: reprisekit::text::escape_mnemonic(&text),
                enabled: true,
                align: Align::Left,
            },
        )
    };
    [line("prompt", PROMPT_Y, prompt), line("note", NOTE_Y, note)]
}

/// [`header`] as notes that rewrap to their width and grow, for a form under a view that
/// scrolls through what they push down ([`FormScroll`]): a word too long for a line breaks.
pub fn wrapping_header(w: i32, prompt: String, note: String) -> [Item; 2] {
    header(w, String::new(), String::new()).map(|it| {
        let text = if it.id == "prompt" { &prompt } else { &note };
        Item::new(it.id, it.rect, Control::Lines(vec![text.clone()]))
    })
}

/// The two buttons on row `y`: `cancel` is the default (so Enter can't accept
/// by accident) and ends at x `right`, `accept` sits a gap before it.
pub fn buttons(
    right: i32,
    y: i32,
    accept: (&'static str, &str),
    cancel: (&'static str, &str),
) -> [Item; 2] {
    let button = |(id, label): (&'static str, &str), x, default| {
        Item::new(
            id,
            Rect::new(x, y, BUTTON_W, BUTTON_H),
            Control::Button {
                label: label.into(),
                enabled: true,
                default,
            },
        )
    };
    [
        button(accept, right - 2 * BUTTON_W - BUTTON_GAP, false),
        button(cancel, right - BUTTON_W, true),
    ]
}

/// How a prompt closed without accepting: `cancel` pressed (click, access
/// key, Enter or Space on it, Enter anywhere on a consent form) or dismissed (Esc, Enter with no
/// button focused, window close).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Button,
    Dismissed,
}

pub type OnCancel = Box<dyn FnMut(&Form, Refusal)>;

/// What a prompt's form gets once its [`Job`] is done.
pub type Landed = Box<dyn FnOnce(&mut Form) + Send>;

/// Work a prompt runs on a thread of its own once its window opens, so a slow one never holds the
/// window back; what it returns is applied to the form on the UI thread.
pub struct Job {
    run: Option<Box<dyn FnOnce() -> Landed + Send>>,
    landed: Arc<Mutex<Option<Landed>>>,
}

impl Job {
    pub fn new(run: impl FnOnce() -> Landed + Send + 'static) -> Job {
        Job {
            run: Some(Box::new(run)),
            landed: Arc::default(),
        }
    }
}

/// A prompt's window: `icon` over `form`, fixed at `size` (100% text), owned by `parent`
/// and, when `modal`, blocking it. `accept` runs `on_accept`, which returns
/// whether the prompt is answered; any other close runs `on_cancel` and quits.
/// The first answer or close stands: input queued behind it in the same dispatch
/// runs neither callback.
pub struct Prompt {
    pub title: String,
    pub size: (i32, i32),
    pub form: Form,
    pub icon: Option<Canvas>,
    pub accept: &'static str,
    pub cancel: &'static str,
    pub on_accept: Box<dyn FnMut(&Form) -> bool>,
    pub on_cancel: OnCancel,
    pub parent: Option<String>,
    pub modal: bool,
    pub job: Option<Job>,
    /// For a prompt whose form can give up room when the work area holds less than `size`:
    /// lays it out for the client it got and scrolls what still does not fit.
    pub fit: Option<FormScroll>,
    /// Set by the first answer or close; a builder starts it false.
    pub answered: bool,
}

impl Prompt {
    pub fn decide(&mut self, ev: form::Event) -> Reply {
        match ev {
            form::Event::Pressed(id) if id == self.accept => {
                if self.answered {
                    Reply::none()
                } else if (self.on_accept)(&self.form) {
                    self.answered = true;
                    Reply {
                        quit: true,
                        ..Default::default()
                    }
                } else {
                    Reply::none()
                }
            }
            form::Event::Pressed(id) if id == self.cancel => self.cancelled(Refusal::Button),
            form::Event::Default | form::Event::Cancel => self.cancelled(Refusal::Dismissed),
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    /// What the scrolling form's bars take before the form: a press on one and the pointer
    /// while one holds it.
    fn scroll_input(&mut self, ev: &Event) -> Option<Reply> {
        let form = &mut self.form;
        let fit = self.fit.as_mut().filter(|f| f.scrolls(form))?;
        let moved = match *ev {
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) if fit.press(form, x, y) => true,
            Event::Pointer(PointerInput::Motion { x, y }) if fit.holding() => {
                fit.motion(form, x, y)
            }
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT, ..
            }) if fit.holding() => fit.release(),
            _ => return None,
        };
        Some(Reply {
            redraw: moved,
            tick: fit.ticking(),
            ..Default::default()
        })
    }

    /// The scrolling form after the form took `ev`, its focus `before` at (`focus`,
    /// [`Form::focus_part`]): a bar lights under the pointer, the wheel the form's controls left
    /// scrolls it, a held arrow steps on the tick, and a key that moved the focus scrolls it into
    /// view.
    fn after_form(&mut self, ev: &Event, before: Focus, mut reply: Reply) -> Reply {
        let form = &mut self.form;
        let Some(fit) = self.fit.as_mut().filter(|f| f.scrolls(form)) else {
            return reply;
        };
        reply.redraw |= match *ev {
            Event::Pointer(PointerInput::Motion { x, y }) => fit.motion(form, x, y),
            Event::Pointer(PointerInput::Leave) => fit.leave(),
            Event::Pointer(PointerInput::Scroll { rows, .. }) if !reply.redraw => {
                fit.wheel(form, rows)
            }
            Event::Tick if fit.ticking() => fit.tick(form, std::time::Instant::now()),
            Event::Key(_) if focus(form) != before => fit.reveal_focus(form),
            _ => false,
        };
        reply.tick |= fit.ticking();
        reply
    }

    fn cancelled(&mut self, how: Refusal) -> Reply {
        if std::mem::replace(&mut self.answered, true) {
            return Reply::none();
        }
        (self.on_cancel)(&self.form, how);
        Reply {
            quit: true,
            ..Default::default()
        }
    }
}

impl App for Prompt {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn app_id(&self) -> String {
        "reprise.portal".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        start_forms().text.size(self.size)
    }

    fn start(&mut self, waker: Waker) -> repriseapp::Start {
        if let Some(job) = &mut self.job
            && let Some(run) = job.run.take()
        {
            let landed = job.landed.clone();
            let spawned = std::thread::Builder::new()
                .name("prompt-job".into())
                .spawn(move || {
                    let done = run();
                    *landed.lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
                    waker.wake();
                });
            if let Err(e) = spawned {
                log::warn!("prompt: no thread for its job: {e}");
            }
        }
        repriseapp::Start::Continue
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::CloseRequest => self.cancelled(Refusal::Dismissed),
            Event::Wake => {
                let landed = (self.job.as_ref())
                    .and_then(|j| j.landed.lock().unwrap_or_else(|e| e.into_inner()).take());
                match landed {
                    Some(apply) => {
                        apply(&mut self.form);
                        Reply::redraw()
                    }
                    None => Reply::none(),
                }
            }
            // A text size change: a fixed window takes only the size it is asked for, held to the
            // work area.
            Event::Resized => {
                let forms = ctx.forms();
                if let Some(fit) = &mut self.fit {
                    self.form.drawn_with(&Paint {
                        s: 1,
                        ..forms.paint()
                    });
                    fit.fit(&mut self.form, ctx.size, &ctx.text, forms.fonts.size_px);
                }
                Reply {
                    size: Some(forms.text.size(self.size)),
                    redraw: true,
                    ..Default::default()
                }
            }
            ev => match self.scroll_input(&ev) {
                Some(r) => r,
                None => {
                    let before = focus(&self.form);
                    let reply = match form_input(&mut self.form, &ev, ctx) {
                        Some(e) => self.decide(e),
                        None => Reply::none(),
                    };
                    self.after_form(&ev, before, reply)
                }
            },
        }
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        c.fill_rect(c.bounds(), forms.colors.button_face);
        form::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            &self.form,
            true,
            px,
        );
        let (left, top) = self.fit.as_ref().map_or((0, 0), FormScroll::offset);
        if let Some(ic) = self.icon.as_ref() {
            let u = forms.text;
            let clip = match &self.form.view {
                Some(v) => u.rect(v.rect),
                None => c.bounds(),
            };
            let (x, y) = (u.len(ICON_AT.0 - left), u.len(ICON_AT.1 - top));
            c.blit_over_clipped(x, y, ic, clip);
        }
        if let Some(fit) = &self.fit {
            fit.draw(
                c,
                &Paint {
                    s: 1,
                    ..forms.paint()
                },
                &self.form,
            );
        }
    }
}

impl repriseapp::Extended for Prompt {
    fn fixed(&self) -> bool {
        true
    }

    fn foreign_parent(&self) -> Option<String> {
        self.parent.clone()
    }

    fn modal(&self) -> bool {
        self.modal
    }

    /// The dialog caption even when the request names no window to own it.
    fn caption(&self, _win: WinId) -> repriseapp::Caption {
        repriseapp::Caption::Fixed
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let px = self.form.units().px(ctx.base_px());
        let Some(fit) = &self.fit else {
            repriseapp::oneshot::access_form(&self.form, tree, &ctx.text, px);
            return;
        };
        if let Some(f) = fit.access(&self.form, tree, reprisekit::access::ROOT, &ctx.text, px) {
            tree.set_focus(f);
        }
    }

    /// A reader's Focus moves the keyboard as Tab would and scrolls it into view; its
    /// ScrollIntoView scrolls a list's row in, or the form to the control the view hides.
    fn reader(&mut self, _win: WinId, act: Reader, ctx: &Ctx) -> bool {
        let px = self.form.units().px(ctx.base_px());
        let (form, text) = (&mut self.form, &ctx.text);
        match act {
            Reader::Reveal {
                at: (x, y),
                row,
                node,
                view,
            } => {
                (!view && form.reveal(x, y, row, text, px) != form::Event::None)
                    || self.fit.as_mut().is_some_and(|f| f.show(form, node))
            }
            Reader::Focus((x, y)) => {
                let moved = form.focus_at(x, y, text, px) != form::Event::None;
                let shown = self.fit.as_mut().is_some_and(|f| f.reveal_focus(form));
                moved || shown
            }
        }
    }
}

/// Where a form's keyboard is: the focused control and the part of it the keyboard rests on.
type Focus = (Option<usize>, Option<Rect>);

fn focus(form: &Form) -> Focus {
    (form.focus(), form.focus_part())
}

/// What of the form `build` makes for each interface's theme at each text size, in Tahoma and
/// in DejaVu Sans, does not fit its text ([`Form::overflows`]), one line each.
#[cfg(test)]
pub fn overflows(
    mut build: impl FnMut(&Theme, &reprisekit::text::TextRenderer) -> Form,
) -> Vec<String> {
    use reprisekit::interface::Interface;
    use reprisekit::text::TextRenderer;
    let mut over = Vec::new();
    for family in ["Tahoma", "DejaVu Sans"] {
        let text = TextRenderer::load(&reprisekit::theme::FontConfig {
            family: vec![family.into()],
            ..Default::default()
        });
        if !text.has_face(family) {
            continue;
        }
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 125, 150, 175, 200] {
                let theme = reprisekit::interfaces::theme_for(i).with_form_text(pct);
                let form = build(&theme, &text);
                form.drawn_with(&theme.paint());
                for (id, by) in form.overflows(&text, theme.fonts.size_px) {
                    over.push(format!(
                        "{family}, {}, {pct}%: {id} {by} px over",
                        i.as_str()
                    ));
                }
            }
        }
    }
    over
}

/// A program context at `text_size` for the dialogs' tests.
#[cfg(test)]
pub fn test_ctx(text_size: u32) -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
            .into(),
        icons: IconCache::new(IconTheme::load("hicolor", &[])),
        caret_on: true,
        size: (300, 300),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::time::{Duration, Instant};

    #[test]
    fn a_text_size_change_resizes_a_prompt_to_its_grown_size() {
        let size = (360, 140);
        let mut p = Prompt {
            title: "Allow?".into(),
            size,
            form: Form::new(Vec::new()),
            icon: None,
            accept: "allow",
            cancel: "deny",
            on_accept: Box::new(|_| true),
            on_cancel: Box::new(|_, _| {}),
            parent: None,
            modal: false,
            job: None,
            fit: None,
            answered: false,
        };
        let mut ctx = test_ctx(175);
        let reply = p.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        let want = ctx.forms().text.size(size);
        assert_ne!(want, size);
        assert_eq!(
            reply.size,
            Some(want),
            "the fixed window asks for its 175% size"
        );
        let reply = p.event(repriseapp::MAIN, Event::Resized, &mut test_ctx(100));
        assert_eq!(reply.size, Some(size), "and its own size back at 100%");
    }

    #[test]
    fn a_prompts_job_runs_off_its_ui_thread_and_lands_on_the_next_wake() {
        let ui = std::thread::current().id();
        let mut p = Prompt {
            title: "Allow?".into(),
            size: (360, 140),
            form: Form::new(header(360, "p".into(), "n".into()).into()),
            icon: None,
            accept: "allow",
            cancel: "deny",
            on_accept: Box::new(|_| true),
            on_cancel: Box::new(|_, _| {}),
            parent: None,
            modal: false,
            job: Some(Job::new(move || {
                assert_ne!(std::thread::current().id(), ui, "not on the UI thread");
                Box::new(|f: &mut Form| f.set_text("note", "done"))
            })),
            fit: None,
            answered: false,
        };
        let mut ctx = test_ctx(100);
        assert!(!p.event(repriseapp::MAIN, Event::Wake, &mut ctx).redraw);
        p.start(Waker::noop());
        let start = Instant::now();
        while p.job.as_ref().unwrap().landed.lock().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "the job lands");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(p.event(repriseapp::MAIN, Event::Wake, &mut ctx).redraw);
        let Some(Control::Label { text, .. }) = p.form.item("note").map(|i| &i.control) else {
            panic!("the note is a label");
        };
        assert_eq!(text, "done");
    }

    #[test]
    fn a_mnemonic_label_becomes_an_access_key() {
        assert_eq!(mnemonic("_Share"), "&Share");
        assert_eq!(mnemonic("A & B"), "A && B");
        assert_eq!(mnemonic("x__y"), "x_y");
        assert_eq!(mnemonic("Allow"), "Allow");
        assert_eq!(mnemonic("_Allow\nonce\tnow"), "&Allow once now");
    }

    #[test]
    fn one_line_turns_control_characters_into_spaces() {
        assert_eq!(one_line("Use the\ncamera?\r\tNow"), "Use the camera?  Now");
        assert_eq!(one_line("plain"), "plain");
    }

    #[test]
    fn only_a_wayland_parent_gives_a_handle() {
        assert_eq!(parent_handle("wayland:abc"), Some("abc".to_string()));
        assert_eq!(parent_handle("x11:1f"), None);
        assert_eq!(parent_handle("wayland:"), None);
        assert_eq!(parent_handle(""), None);
    }

    #[test]
    fn an_ampersand_in_the_prompt_draws_as_itself() {
        let [prompt, _] = header(391, "R&D Tool wants to see your screen.".into(), "n".into());
        let Control::Label { text, .. } = &prompt.control else {
            panic!("the prompt is a label");
        };
        assert_eq!(
            reprisekit::text::split_mnemonic(text),
            ("R&D Tool wants to see your screen.".to_string(), None)
        );
    }

    #[test]
    fn prompt_lines_and_buttons_sit_on_the_shared_grid() {
        let [prompt, note] = header(391, "p".into(), "n".into());
        assert_eq!(prompt.rect, Rect::new(54, 12, 326, 13), "prompt line");
        assert_eq!(note.rect, Rect::new(54, 38, 326, 13), "note line");
        let [ok, cancel] = buttons(380, 97, ("ok", "OK"), ("cancel", "Cancel"));
        assert_eq!(ok.rect, Rect::new(224, 97, 75, 23), "accept button");
        assert_eq!(cancel.rect, Rect::new(305, 97, 75, 23), "cancel button");
        assert!(matches!(ok.control, Control::Button { default: false, .. }));
        assert!(matches!(
            cancel.control,
            Control::Button { default: true, .. }
        ));
    }

    #[test]
    fn a_large_answer_does_not_block_the_child() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "head -c 300000 /dev/zero | tr '\\0' a"]);
            let out = run_cmd(cmd, &[], "", None, &AtomicBool::new(false));
            let _ = tx.send(out.map(|s| s.len()));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(10));
        assert_eq!(got, Ok(Some(300000)), "the whole answer, in time");
    }

    #[test]
    fn docs_split_on_nul_and_skip_empty_ones() {
        let raw: &[u8] = b"a = 1\n\0\0b = 2\n\0tail";
        let got: Vec<String> = docs(raw).collect();
        assert_eq!(
            got,
            vec![
                "a = 1\n".to_string(),
                "b = 2\n".to_string(),
                "tail".to_string()
            ]
        );
    }

    #[test]
    fn docs_drop_a_non_utf8_document_and_keep_the_rest() {
        let raw: &[u8] = b"a = 1\n\0bad = \xff\xfe\n\0b = 2\n";
        let got: Vec<String> = docs(raw).collect();
        assert_eq!(got, vec!["a = 1\n".to_string(), "b = 2\n".to_string()]);
    }

    #[test]
    fn a_live_child_gets_the_input_then_each_update() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("second".to_string()).unwrap();
        drop(tx);
        let out = run_cmd(
            Command::new("cat"),
            &[],
            "first",
            Some(rx),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(out, "first\0second\0");
    }

    #[test]
    fn child_gets_sigterm_when_its_spawning_thread_exits() {
        let mut child = std::thread::spawn(|| {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            spawn_tied(cmd).expect("spawn sleep")
        })
        .join()
        .unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the child should die with SIGTERM once its spawner is gone");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.signal(), Some(libc::SIGTERM), "killed by SIGTERM");
    }
}
