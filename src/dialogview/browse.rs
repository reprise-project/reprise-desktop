//! Run's Browse...: the file dialog child (`reprise-portal --file`) over the Run dialog, its pick
//! filling Run's field.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use reprisefs::filechild::{Filter, Input, Kind, Output, Pattern};

use crate::compositor::State;
use crate::dialog::{Dialog, Outcome};
use crate::window::WindowId;

/// A file dialog child running over dialog `over`.
#[derive(Clone, Debug)]
pub struct Browsing {
    pub pid: i32,
    pub over: u64,
    /// Opened while the child was ours and unreaped, so a signal never reaches a reused pid.
    pub pidfd: Option<Arc<OwnedFd>>,
}

/// The child's input: Programs (files with an execute bit, which wear the program icon) and All
/// Files, starting in the folder the last Browse dialog was confirmed in.
pub fn input() -> Input {
    let filter = |label: &str, p: Pattern| Filter {
        label: label.into(),
        patterns: vec![p],
    };
    Input {
        kind: Some(Kind::Open),
        title: "Browse".into(),
        accept_label: "&Open".into(),
        filters: vec![
            filter("Programs", Pattern::Exec),
            filter("All Files", Pattern::Glob("*".into())),
        ],
        app_id: crate::filedlggeom::OWN_APP.into(),
        exec_icon: true,
        ..Default::default()
    }
}

/// Starts the child with `input` and returns its pick, or a cancel when it fails or is killed;
/// `started` hears its pid first. Blocks until the child exits.
fn run_child(env: &crate::spawn::SessionEnv, input: &Input, started: impl FnOnce(i32)) -> Output {
    let text = match toml::to_string(input) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("browse: input: {e}");
            return Output::default();
        }
    };
    let mut child = match crate::spawn::spawn_piped(&["reprise-portal", "--file"], env) {
        Ok(Some(c)) => c,
        Ok(None) => return Output::default(),
        Err(e) => {
            log::error!("browse: reprise-portal --file: {e}");
            return Output::default();
        }
    };
    started(child.id() as i32);
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(text.as_bytes())
    {
        log::warn!("browse: writing the input: {e}");
    }
    let mut answer = String::new();
    if let Some(mut out) = child.stdout.take()
        && let Err(e) = out.read_to_string(&mut answer)
    {
        log::warn!("browse: reading the answer: {e}");
    }
    if let Err(e) = child.wait() {
        log::warn!("browse: {e}");
    }
    toml::from_str(&answer)
        .map_err(|e| log::warn!("browse: answer: {e}"))
        .unwrap_or_default()
}

impl State {
    /// Run's Browse...: on the folder the last Browse dialog was confirmed in, else Documents or
    /// home.
    pub(super) fn browse_run(&mut self) {
        let Some(over) = self.dialogs.views.last().map(|v| v.id) else {
            return;
        };
        if let Some(w) = self.window_over_dialog(over) {
            self.focus_window(w);
            return;
        }
        let input = input();
        let env = self.env.clone();
        let to_loop = self.fsjobs.sender();
        // The thread waits for as long as the child runs.
        self.spawn_job(move || {
            let mut pid = None;
            let out = run_child(&env, &input, |p| {
                pid = Some(p);
                let pidfd = (crate::hang::process_pidfd(p))
                    .map_err(|e| log::warn!("browse: no pidfd for pid {p}: {e}"))
                    .ok()
                    .map(Arc::new);
                let b = Browsing {
                    pid: p,
                    over,
                    pidfd,
                };
                if let Some(tx) = &to_loop {
                    let _ = tx.send(Box::new(move |state: &mut State| state.browse_started(b)));
                }
            });
            Box::new(move |state: &mut State| {
                if let Some(pid) = pid {
                    state.browse_answered(pid, out);
                }
            })
        });
    }

    /// The child `b` started: its windows, mapped already or later, stand over its dialog.
    pub(crate) fn browse_started(&mut self, b: Browsing) {
        let (pid, over) = (b.pid, b.over);
        self.dialogs.browsing.push(b);
        let windows: Vec<WindowId> = (self.windows.iter())
            .filter(|w| self.client_pid(w.id) == Some(pid))
            .map(|w| w.id)
            .collect();
        for w in windows {
            if let Some(win) = self.win_mut(w) {
                win.over_dialog = Some(over);
            }
        }
        self.refresh_owned();
        self.restack();
    }

    /// Dialog `id` left the stack: the Browse child over it is ended, and its windows stand over
    /// nothing.
    pub(super) fn browse_over_gone(&mut self, id: u64) {
        let (gone, kept) = (std::mem::take(&mut self.dialogs.browsing).into_iter())
            .partition::<Vec<_>, _>(|b| b.over == id);
        self.dialogs.browsing = kept;
        if gone.is_empty() {
            return;
        }
        for b in gone {
            log::info!("browse: pid {} outlived its dialog; ending it", b.pid);
            if let Some(fd) = &b.pidfd
                && let Err(e) = crate::screenrecord::signal(fd, libc::SIGTERM)
            {
                log::debug!("browse: pid {}: {e}", b.pid);
            }
        }
        for w in &mut self.windows {
            if w.over_dialog == Some(id) {
                w.over_dialog = None;
            }
        }
        self.refresh_owned();
        self.restack();
    }

    /// The dialog a window of process `pid` stands over: the one its Browse... started it from.
    pub(crate) fn browse_over(&self, pid: Option<i32>) -> Option<u64> {
        let pid = pid?;
        (self.dialogs.browsing.iter())
            .find(|b| b.pid == pid)
            .map(|b| b.over)
    }

    /// The top mapped window standing over dialog `id`, which takes the keyboard in its place.
    pub(crate) fn window_over_dialog(&self, id: u64) -> Option<WindowId> {
        (self.focus_order.iter().rev().copied())
            .chain(self.windows.iter().map(|w| w.id))
            .find(|&w| {
                self.win(w)
                    .is_some_and(|x| x.mapped && x.over_dialog == Some(id))
            })
    }

    /// The child of `pid` exited with `out`: its dialog comes back up with the keyboard and takes
    /// the pick.
    pub(crate) fn browse_answered(&mut self, pid: i32, out: Output) {
        let Some(at) = self.dialogs.browsing.iter().position(|b| b.pid == pid) else {
            return;
        };
        let b = self.dialogs.browsing.remove(at);
        let Some(i) = self.dialog_index(b.over) else {
            return;
        };
        if self.window_over_dialog(b.over).is_none() {
            self.raise_dialog(i);
        }
        let Some(path) = (out.paths.first()).map(|p| PathBuf::from(OsStr::from_bytes(p))) else {
            return;
        };
        let Some(i) = self.dialog_index(b.over) else {
            return;
        };
        let outcome = match path.to_str() {
            Some(picked) => {
                let quoted = reprisefs::ops::shell_quote(picked);
                self.dialogs.views[i].dialog.set_field(&quoted)
            }
            // The field holds text, so a non-UTF-8 name cannot go into it.
            None => Outcome::Notice(Box::new(Dialog::not_utf8_program("Run", &path))),
        };
        match outcome {
            Outcome::Redraw => {
                self.restart_caret();
                self.redraw_dialog(i);
            }
            Outcome::Notice(d) => self.open_dialog_over_id(*d, Some(b.over)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    /// Above the kernel's pid limit, so no real process answers to it.
    const CHILD: i32 = 0x7EAD_0001;

    #[test]
    fn browse_asks_for_programs_by_their_execute_bit() {
        let run = input();
        assert_eq!(run.title, "Browse");
        assert_eq!(run.kind, Some(Kind::Open));
        assert!(!run.folder_mode && !run.multiple);
        assert_eq!(run.accept_label, "&Open");
        assert_eq!(
            run.filters,
            [
                Filter {
                    label: "Programs".into(),
                    patterns: vec![Pattern::Exec],
                },
                Filter {
                    label: "All Files".into(),
                    patterns: vec![Pattern::Glob("*".into())],
                },
            ]
        );
        assert_eq!(run.filter, 0, "it opens on Programs");
        assert!(run.exec_icon);
        assert_eq!(run.app_id, "reprise");
        assert!(run.start.is_empty(), "Run's opens where the last one left");
    }

    /// Run with a Browse... child of `pid` mapped over it.
    fn run_under_child(
        h: &mut Harness,
        pid: i32,
    ) -> (u64, crate::harness::TestClient, crate::harness::Toplevel) {
        h.output("o", 1024, 768);
        h.state.open_dialog(Dialog::run(&[]));
        let run = h.state.dialogs.views[0].id;
        h.state.browse_started(Browsing {
            pid,
            over: run,
            pidfd: None,
        });
        crate::mapfocus::grant(pid);
        let mut c = h.connect_as(pid);
        let t = h.map_toplevel(&mut c, "reprise", 400, 300);
        (run, c, t)
    }

    #[test]
    fn the_child_stands_over_run_with_the_keyboard_and_run_takes_no_press() {
        let mut h = Harness::new();
        let (run, _c, t) = run_under_child(&mut h, CHILD);
        let s = &h.state;
        assert_eq!(s.win(t.id).and_then(|w| w.over_dialog), Some(run));
        let tree = s.win(t.id).unwrap().tree;
        assert_eq!(
            s.scene.parent(tree),
            Some(s.layers.menu),
            "over the dialogs"
        );
        assert_eq!(s.focused, Some(t.id), "the child has the keyboard");
        assert!(!s.dialogs.active, "Run lacks the keyboard");
        assert!(!s.task_windows().contains(&t.id), "no task button");
        h.state.set_minimized(t.id, true);
        assert!(
            !h.state.win(t.id).unwrap().minimized,
            "it does not minimize"
        );
        let cancel = Some(crate::dialog::DialogHit::Button(1));
        h.state.dialog_press(0, cancel, 0);
        h.state.dialog_release_button();
        assert_eq!(h.state.dialogs.views.len(), 1, "Run's Cancel did nothing");
        assert_eq!(h.state.focused, Some(t.id), "the press went to the child");
    }

    #[test]
    fn the_childs_pick_lands_quoted_in_runs_field_once_it_closes() {
        let mut h = Harness::new();
        let (run, mut c, t) = run_under_child(&mut h, CHILD + 1);
        c.destroy(t);
        h.roundtrip(&mut c);
        assert!(h.state.dialogs.active, "Run has the keyboard back");
        assert_eq!(h.state.focused, None);
        h.state.browse_answered(
            CHILD + 1,
            Output {
                paths: vec![b"/home/u/bin/my tool".to_vec()],
                ..Default::default()
            },
        );
        assert!(h.state.dialogs.browsing.is_empty());
        let v = &h.state.dialogs.views[0];
        assert_eq!(v.id, run);
        let field = v.dialog.field.as_ref().unwrap().combo.text();
        assert_eq!(field, "'/home/u/bin/my tool'");
    }

    #[test]
    fn a_welcome_panel_clearing_run_lets_its_child_go() {
        let mut h = Harness::new();
        let (_run, _c, t) = run_under_child(&mut h, CHILD + 3);
        h.state
            .open_dialog(Dialog::log_off(reprisekit::powerpanel::Leave::SwitchUser {
                live: false,
            }));
        assert!(h.state.dialogs.browsing.is_empty());
        assert_eq!(h.state.win(t.id).and_then(|w| w.over_dialog), None);
        assert!(
            h.state.task_windows().contains(&t.id),
            "its window is a window of its own again"
        );
    }

    #[test]
    fn a_cancelled_child_leaves_runs_field_alone() {
        let mut h = Harness::new();
        h.output("o", 1024, 768);
        h.state.open_dialog(Dialog::run(&["notepad".to_string()]));
        let run = h.state.dialogs.views[0].id;
        h.state.browse_started(Browsing {
            pid: CHILD + 2,
            over: run,
            pidfd: None,
        });
        let before = h.state.dialogs.views[0].dialog.field.clone();
        h.state.browse_answered(CHILD + 2, Output::default());
        assert_eq!(h.state.dialogs.views[0].dialog.field, before);
        assert!(h.state.dialogs.active, "Run has the keyboard");
    }
}
