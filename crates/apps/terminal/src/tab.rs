//! One terminal: a `Term` and the PTY event loop feeding it on its own thread.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, Config, Term, TermMode};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite, Pty, Shell};
use alacritty_terminal::vte::ansi::{CursorShape as TermCursor, CursorStyle, NamedColor, Rgb};
use polling::{PollMode, Poller};
use repriseapp::Waker;
use reprisekit::controls::termgrid::Snapshot;

use crate::cwd;
use crate::find::{Marks, Search};
use crate::newtab::Launch;
use crate::schemes::Scheme;
use crate::settings::{Cursor, Settings};

/// What the PTY thread hands the main loop between wakes.
#[derive(Default)]
struct Inbox {
    title: Option<Option<String>>,
    replies: Vec<Answer>,
    copied: Copied,
    status: Option<i32>,
    exited: bool,
}

/// A query the program sent that the main loop answers into the PTY.
enum Answer {
    Text(String),
    Colour(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync>),
    Size(Arc<dyn Fn(WindowSize) -> String + Send + Sync>),
}

/// Text the program stored with OSC 52 since the last pump; its loads are not answered.
#[derive(Default)]
pub struct Copied {
    pub clipboard: Option<String>,
    pub primary: Option<String>,
}

/// The `Term`'s event sink: queues what needs the main loop and wakes it.
#[derive(Clone)]
pub struct Proxy {
    inbox: Arc<Mutex<Inbox>>,
    waker: Waker,
}

impl EventListener for Proxy {
    fn send_event(&self, ev: TermEvent) {
        {
            let mut inbox = self.inbox.lock().unwrap();
            match ev {
                TermEvent::Title(t) => inbox.title = Some(Some(t)),
                TermEvent::ResetTitle => inbox.title = Some(None),
                TermEvent::PtyWrite(s) => inbox.replies.push(Answer::Text(s)),
                TermEvent::ColorRequest(i, f) => inbox.replies.push(Answer::Colour(i, f)),
                TermEvent::TextAreaSizeRequest(f) => inbox.replies.push(Answer::Size(f)),
                TermEvent::ClipboardStore(ClipboardType::Clipboard, text) => {
                    inbox.copied.clipboard = Some(text)
                }
                TermEvent::ClipboardStore(ClipboardType::Selection, text) => {
                    inbox.copied.primary = Some(text)
                }
                TermEvent::ChildExit(status) => inbox.status = status.code(),
                TermEvent::Exit => inbox.exited = true,
                TermEvent::Wakeup => {}
                _ => return,
            }
        }
        self.waker.wake();
    }
}

pub struct Tab {
    term: Arc<FairMutex<Term<Proxy>>>,
    notifier: Notifier,
    inbox: Arc<Mutex<Inbox>>,
    /// The program's own title, else its name.
    title: String,
    program: String,
    window: Cell<WindowSize>,
    exit: Option<i32>,
    pid: u32,
    /// The directory the shell last reported with OSC 7.
    osc7: Arc<Mutex<Option<PathBuf>>>,
    search: RefCell<Option<Search>>,
    /// Every write handed to the PTY, for the tests to read back.
    #[cfg(test)]
    pub written: RefCell<Vec<Vec<u8>>>,
}

impl Tab {
    /// Start `launch` in `cwd` on a PTY of `size` (cols, rows) whose cells are `cell` device px;
    /// `waker` fires whenever the PTY thread has news.
    pub fn spawn(
        launch: &Launch,
        cwd: Option<PathBuf>,
        size: (usize, usize),
        cell: (i32, i32),
        waker: Waker,
    ) -> io::Result<Tab> {
        let mut program = std::env::var("SHELL").unwrap_or_default();
        let shell = launch.argv.clone().filter(|c| !c.is_empty()).map(|c| {
            let mut words = c.into_iter().map(|w| w.to_string_lossy().into_owned());
            program = words.next().unwrap_or_default();
            Shell::new(program.clone(), words.collect())
        });
        let program = launch.name.clone().unwrap_or_else(|| {
            Path::new(&program)
                .file_name()
                .map_or_else(|| "Terminal".into(), |n| n.to_string_lossy().into_owned())
        });
        let options = tty::Options {
            shell,
            working_directory: cwd,
            drain_on_exit: true,
            env: [
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ]
            .into(),
        };
        let window = window_size(size, cell);
        let pty = tty::new(&options, window, 0)?;
        let pid = pty.child().id();
        let osc7 = Arc::new(Mutex::new(None));
        let reader = cwd::Reader {
            file: pty.file().try_clone()?,
            scan: cwd::Osc7::default(),
            host: cwd::hostname(),
            cwd: osc7.clone(),
        };
        let pty = Watched { pty, reader };
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let proxy = Proxy {
            inbox: inbox.clone(),
            waker,
        };
        let term = Term::new(
            Config::default(),
            &TermSize::new(size.0, size.1),
            proxy.clone(),
        );
        let term = Arc::new(FairMutex::new(term));
        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();
        Ok(Tab {
            term,
            notifier,
            inbox,
            title: program.clone(),
            program,
            window: Cell::new(window),
            exit: None,
            pid,
            osc7,
            #[cfg(test)]
            written: RefCell::default(),
            search: RefCell::new(None),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The directory a new tab opened from this one starts in: the last OSC 7, else the shell's.
    pub fn cwd(&self) -> Option<PathBuf> {
        let reported = self.osc7.lock().unwrap().clone();
        reported.or_else(|| cwd::cwd_of(self.pid))
    }

    /// Whether a program other than the shell holds the foreground, so closing asks first.
    pub fn busy(&self) -> bool {
        self.exit.is_none() && cwd::needs_confirm(cwd::fg_pgrp(self.pid), self.pid)
    }

    /// The grid in cells, (cols, rows).
    #[cfg(test)]
    pub fn size(&self) -> (usize, usize) {
        use alacritty_terminal::grid::Dimensions;
        let t = self.term.lock();
        (t.columns(), t.screen_lines())
    }

    /// Run `f` on the terminal, locked.
    pub fn with_term<R>(&self, f: impl FnOnce(&mut Term<Proxy>) -> R) -> R {
        f(&mut self.term.lock())
    }

    pub fn selection(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn write(&self, bytes: Cow<'static, [u8]>) {
        #[cfg(test)]
        self.written.borrow_mut().push(bytes.to_vec());
        self.notifier.notify(bytes);
    }

    pub fn resize(&self, size: (usize, usize), cell: (i32, i32)) {
        self.term.lock().resize(TermSize::new(size.0, size.1));
        let window = window_size(size, cell);
        self.window.set(window);
        let _ = self.notifier.0.send(Msg::Resize(window));
    }

    /// Scrollback and cursor style from `s`.
    pub fn configure(&self, s: &Settings) {
        let shape = match s.cursor {
            Cursor::Block => TermCursor::Block,
            Cursor::Bar => TermCursor::Beam,
            Cursor::Underline => TermCursor::Underline,
        };
        self.term.lock().set_options(Config {
            scrolling_history: s.scrollback,
            default_cursor_style: CursorStyle {
                shape,
                blinking: s.blink,
            },
            ..Config::default()
        });
    }

    pub fn snapshot(&self, scheme: &Scheme, out: &mut Snapshot) {
        let term = self.term.lock();
        let marks = match self.search.borrow_mut().as_mut() {
            Some(s) => s.marks(&term),
            None => Marks::default(),
        };
        crate::snapshot::fill_marked(&term, scheme, &marks, out);
    }

    /// Step to the next match of `text` and scroll to it; a new query starts from the view's edge.
    /// False when nothing matches.
    pub fn find(&self, text: &str, match_case: bool, down: bool) -> bool {
        let mut search = self.search.borrow_mut();
        if !search.as_ref().is_some_and(|s| s.is_for(text, match_case)) {
            *search = Search::new(text, match_case);
        }
        match search.as_mut() {
            Some(s) => s.step(&mut self.term.lock(), down),
            None => false,
        }
    }

    #[cfg(test)]
    pub fn searching(&self) -> bool {
        self.search.borrow().is_some()
    }

    /// Stop highlighting matches.
    pub fn end_find(&self) {
        *self.search.borrow_mut() = None;
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn scroll(&self, by: Scroll) {
        self.term.lock().scroll_display(by);
    }

    /// Carry out what the PTY thread queued: title changes, the program's queries (answered in
    /// `scheme`'s colours), and its exit. Returns what it stored on the clipboards.
    pub fn pump(&mut self, scheme: &Scheme) -> Copied {
        let (title, replies, copied, status, exited) = {
            let mut inbox = self.inbox.lock().unwrap();
            (
                inbox.title.take(),
                std::mem::take(&mut inbox.replies),
                std::mem::take(&mut inbox.copied),
                inbox.status,
                inbox.exited,
            )
        };
        if let Some(t) = title {
            self.title = t.filter(|t| !t.is_empty()).unwrap_or(self.program.clone());
        }
        for answer in replies {
            let text = match answer {
                Answer::Text(s) => s,
                Answer::Colour(i, f) => f(self.colour(i, scheme)),
                Answer::Size(f) => f(self.window.get()),
            };
            self.write(text.into_bytes().into());
        }
        if exited {
            self.exit = Some(status.unwrap_or(0));
        }
        copied
    }

    fn colour(&self, i: usize, scheme: &Scheme) -> Rgb {
        if let Some(set) = self.term.lock().colors()[i] {
            return set;
        }
        let ours = match i {
            0..256 => crate::snapshot::indexed(i as u8, scheme),
            i if i == NamedColor::Background as usize => scheme.bg,
            i if i == NamedColor::Cursor as usize => scheme.accents.cursor,
            _ => scheme.fg,
        };
        Rgb {
            r: ours.r,
            g: ours.g,
            b: ours.b,
        }
    }

    #[cfg(test)]
    pub fn history(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        self.term.lock().history_size()
    }

    /// The process's exit code once it has exited.
    pub fn exited(&self) -> Option<i32> {
        self.exit
    }

    pub fn title(&self) -> &str {
        &self.title
    }
}

impl Drop for Tab {
    /// The event loop ends and drops the PTY, which hangs up on the program.
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

/// The PTY with its reads passed through [`cwd::Reader`].
struct Watched {
    pty: Pty,
    reader: cwd::Reader,
}

impl EventedReadWrite for Watched {
    type Reader = cwd::Reader;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: polling::Event,
        mode: PollMode,
    ) -> io::Result<()> {
        // SAFETY: the caller's contract passes through unchanged to the PTY it wraps.
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: polling::Event,
        mode: PollMode,
    ) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut cwd::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for Watched {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for Watched {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}

fn window_size(size: (usize, usize), cell: (i32, i32)) -> WindowSize {
    let clamp = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
    let px = |v: i32| u16::try_from(v.max(1)).unwrap_or(u16::MAX);
    WindowSize {
        num_cols: clamp(size.0),
        num_lines: clamp(size.1),
        cell_width: px(cell.0),
        cell_height: px(cell.1),
    }
}
