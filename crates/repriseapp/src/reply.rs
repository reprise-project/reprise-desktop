//! A program's answer to an event, and the spawns it asks for.

use crate::{Accept, DragAction, DragOut, FileClip, WinId};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// Run `argv` as a process of its own in working directory `dir` (the caller's when `None`), no
/// shell between, reaped in the background: a [`Reply::spawn`] for a caller that needs the exec's
/// error, as a program file that will not start reports it. With a `scope` the child waits before
/// its exec until it is in that app scope ([`reprisefs::scope::hold`]), so this blocks on the
/// session bus.
pub fn spawn_in(argv: &[OsString], dir: Option<&Path>, scope: Option<&str>) -> std::io::Result<()> {
    spawn_then(argv, dir, scope, |_| {})
}

/// [`spawn_in`]'s spawn, running `then` with the child's pid on the reaper thread before the
/// wait, so the pid cannot be reused under it.
pub(crate) fn spawn_then(
    argv: &[OsString],
    dir: Option<&Path>,
    scope: Option<&str>,
    then: impl FnOnce(u32) + Send + 'static,
) -> std::io::Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command line",
        ));
    };
    let mut c = Command::new(program);
    c.args(args)
        .env_remove(reprisebus::instance::KEYBOARD_LAUNCH_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    if let Some(app) = scope {
        reprisefs::scope::hold(&mut c, app)?;
    }
    let mut child = c.spawn()?;
    std::thread::spawn(move || {
        then(child.id());
        let _ = child.wait();
    });
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub redraw: bool,
    pub redraw_all: bool,
    pub quit: bool,
    pub copy: Option<String>,
    /// Arrives as `Event::Pasted` on the window this reply is for.
    pub paste: bool,
    /// Offered as the primary selection, the text a middle click pastes.
    pub copy_primary: Option<String>,
    /// Arrives as `Event::PastedPrimary` on the window this reply is for.
    pub paste_primary: bool,
    /// Run each argument vector as a process of its own, no shell
    /// between, detached and reaped in the background. No activation
    /// token is sent: a launched window takes focus as a child of the
    /// focused caller.
    pub spawn: Vec<Vec<OsString>>,
    /// The desktop entry id `spawn` runs as: each process starts in that app's scope
    /// (`reprisefs::scope`).
    pub spawn_app: Option<String>,
    pub copy_files: Option<FileClip>,
    /// Arrives as `Event::PastedFiles` on the window this reply is for.
    pub paste_files: bool,
    /// What another process handed over with a request to show a window, and
    /// that window: its token is presented for it, so the compositor focuses
    /// it, and a start by key brings out its cues. Dropped while that window
    /// is not shown.
    pub activate: Option<(WinId, reprisebus::instance::Handoff)>,
    /// Start a drag of files from the window this reply is for. It must
    /// answer a pointer press still held (the host uses that press's
    /// serial); the compositor refuses it otherwise.
    pub start_drag: Option<Box<DragOut>>,
    /// Ask again for the running drag's image ([`App::drag_image`](crate::App::drag_image)) at the
    /// scale it shows at, for art that landed since it was drawn.
    pub redraw_drag: bool,
    /// The answer to a drag over the window: `Some(None)` refuses it,
    /// `Some(Some(a))` takes it as `a`, `None` leaves the last answer.
    pub drag_accept: Option<Option<Accept>>,
    /// The user's choice for an `Ask` drop: `Some(Some(Copy | Move))`
    /// finishes it with that action, `Some(None)` (or `Ask`) cancels it.
    pub answer_drop: Option<Option<DragAction>>,
    /// Send the window this reply is for one `Event::Tick` in `TICK_MS`,
    /// for something that moves while the user holds still (a drag
    /// resting at an edit's edge scrolls it). A window that wants the
    /// next asks again in its answer to the tick; asking while one is on
    /// its way adds none.
    pub tick: bool,
    /// Resize the window this reply is for from its lower-right corner,
    /// the way its size grip does. It must answer a left press still held
    /// (the host uses that press's serial); the compositor refuses it
    /// otherwise.
    pub resize: bool,
    /// Make the main window this client size (logical px), for a window that
    /// grows with what it shows, held inside the compositor's bounds. A size that
    /// changes the window's sends it `Event::Resized`. It stands until the
    /// compositor asks for another size; a configure that only repeats the last
    /// size asked for leaves it.
    pub size: Option<(i32, i32)>,
    /// More of this program's windows to draw again, besides the one this reply is for.
    pub redraw_wins: Vec<WinId>,
    /// Bring this program's window to the front with the keyboard: the host asks xdg-activation
    /// for a token on the last input's serial and presents it for that window.
    pub raise: Option<WinId>,
}

impl Reply {
    pub fn redraw() -> Reply {
        Reply {
            redraw: true,
            ..Default::default()
        }
    }
    pub fn none() -> Reply {
        Reply::default()
    }
    pub fn redraw_all() -> Reply {
        Reply {
            redraw_all: true,
            ..Default::default()
        }
    }
    pub fn resize() -> Reply {
        Reply {
            resize: true,
            ..Default::default()
        }
    }
    pub fn start_drag(out: DragOut) -> Reply {
        Reply {
            start_drag: Some(Box::new(out)),
            ..Default::default()
        }
    }
    pub fn drag_accept(accept: Option<Accept>) -> Reply {
        Reply {
            drag_accept: Some(accept),
            ..Default::default()
        }
    }
    pub fn answer_drop(action: Option<DragAction>) -> Reply {
        Reply {
            answer_drop: Some(action),
            ..Default::default()
        }
    }
    /// This reply with the clipboard half `clip` read for the same event
    /// ([`keys::form_clipboard`](crate::keys::form_clipboard)) added: `clip`'s copy goes first, and
    /// either one asking to paste asks.
    pub fn with_clipboard(self, clip: Reply) -> Reply {
        Reply {
            copy: clip.copy.or(self.copy),
            paste: clip.paste || self.paste,
            ..self
        }
    }

    /// This reply and `then`, the answer of another job landing in the same wake, so neither
    /// loses its effects: flags join, each spawn keeps its own app scope, and for a one-off
    /// (a copy, a drag) this reply's goes first.
    pub fn and(self, then: Reply) -> Reply {
        let Reply {
            redraw,
            redraw_all,
            quit,
            copy,
            paste,
            copy_primary,
            paste_primary,
            spawn,
            spawn_app,
            copy_files,
            paste_files,
            activate,
            start_drag,
            redraw_drag,
            drag_accept,
            answer_drop,
            tick,
            resize,
            size,
            mut redraw_wins,
            raise,
        } = then;
        let (spawn, spawn_app) = if spawn.is_empty() {
            (self.spawn, self.spawn_app)
        } else if self.spawn.is_empty() {
            (spawn, spawn_app)
        } else if self.spawn_app == spawn_app {
            (self.spawn.into_iter().chain(spawn).collect(), spawn_app)
        } else {
            let scoped = |argvs: Vec<Vec<OsString>>, app: Option<String>| {
                argvs
                    .into_iter()
                    .map(move |argv| reprisefs::scope::in_scope(&argv, app.as_deref()))
            };
            let mine = scoped(self.spawn, self.spawn_app);
            (mine.chain(scoped(spawn, spawn_app)).collect(), None)
        };
        Reply {
            redraw: self.redraw || redraw,
            redraw_all: self.redraw_all || redraw_all,
            quit: self.quit || quit,
            copy: self.copy.or(copy),
            paste: self.paste || paste,
            copy_primary: self.copy_primary.or(copy_primary),
            paste_primary: self.paste_primary || paste_primary,
            spawn,
            spawn_app,
            copy_files: self.copy_files.or(copy_files),
            paste_files: self.paste_files || paste_files,
            activate: self.activate.or(activate),
            start_drag: self.start_drag.or(start_drag),
            redraw_drag: self.redraw_drag || redraw_drag,
            drag_accept: self.drag_accept.or(drag_accept),
            answer_drop: self.answer_drop.or(answer_drop),
            tick: self.tick || tick,
            resize: self.resize || resize,
            size: self.size.or(size),
            redraw_wins: {
                let mut wins = self.redraw_wins;
                wins.append(&mut redraw_wins);
                wins
            },
            raise: self.raise.or(raise),
        }
    }
}
