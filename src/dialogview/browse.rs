//! Run's Browse...: the file dialog child (`reprise-portal --file`) over the Run dialog, its pick
//! filling Run's field.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use reprisefs::filechild::{Filter, Input, Kind, Output, Pattern};
use reprisekit::tr;

use crate::compositor::State;
use crate::dialog::{Dialog, Outcome};
use crate::window::WindowId;

/// A file dialog child running over dialog `over`, from the press of Browse... on.
#[derive(Clone, Debug)]
pub struct Browsing {
    /// None while the child is starting.
    pub pid: Option<i32>,
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
        title: tr!("dlg-browse-title"),
        accept_label: tr!("dlg-browse-open"),
        filters: vec![
            filter(&tr!("dlg-browse-programs"), Pattern::Exec),
            filter(&tr!("dlg-browse-all-files"), Pattern::Glob("*".into())),
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
    if answer.is_empty() {
        return Output::default();
    }
    toml::from_str(&answer)
        .map_err(|e| log::warn!("browse: answer: {e}"))
        .unwrap_or_default()
}

/// Ends the child of `b`, whose dialog left.
fn end_child(b: &Browsing) {
    let Some(pid) = b.pid else {
        return;
    };
    log::info!("browse: pid {pid} outlived its dialog; ending it");
    if let Some(fd) = &b.pidfd
        && let Err(e) = crate::screenrecord::signal(fd, libc::SIGTERM)
    {
        log::debug!("browse: pid {pid}: {e}");
    }
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
        if self.dialogs.browsing.iter().any(|b| b.over == over) {
            return;
        }
        self.dialogs.browsing.push(Browsing {
            pid: None,
            over,
            pidfd: None,
        });
        let input = input();
        let env = self.env.clone();
        let to_loop = self.fsjobs.sender();
        // The thread waits for as long as the child runs.
        let started = self.spawn_job(move || {
            let out = run_child(&env, &input, |p| {
                let pidfd = (crate::hang::process_pidfd(p))
                    .map_err(|e| log::warn!("browse: no pidfd for pid {p}: {e}"))
                    .ok()
                    .map(Arc::new);
                let b = Browsing {
                    pid: Some(p),
                    over,
                    pidfd,
                };
                if let Some(tx) = &to_loop {
                    let _ = tx.send(Box::new(move |state: &mut State| state.browse_started(b)));
                }
            });
            Box::new(move |state: &mut State| state.browse_answered(over, out))
        });
        if !started {
            self.dialogs.browsing.retain(|b| b.over != over);
        }
    }

    /// The child `b` started: its windows, mapped already or later, stand over its dialog. One
    /// whose dialog left while it started is ended.
    pub(crate) fn browse_started(&mut self, b: Browsing) {
        let (Some(pid), over) = (b.pid, b.over) else {
            return;
        };
        if self.dialog_index(over).is_none() {
            end_child(&b);
            return;
        }
        self.dialogs.browsing.retain(|x| x.over != over);
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
        for b in &gone {
            end_child(b);
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
            .find(|b| b.pid == Some(pid))
            .map(|b| b.over)
    }

    /// Whether dialog `id` waits on a Browse child, starting or up: it takes no key or press but
    /// Escape and Cancel, which close it and end the child.
    pub(super) fn under_browse(&self, id: u64) -> bool {
        self.dialogs.browsing.iter().any(|b| b.over == id) || self.window_over_dialog(id).is_some()
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

    /// The child over dialog `over` exited with `out`, or never started: the dialog comes back up
    /// with the keyboard and takes the pick.
    pub(crate) fn browse_answered(&mut self, over: u64, out: Output) {
        let Some(at) = self.dialogs.browsing.iter().position(|b| b.over == over) else {
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
            None => Outcome::Notice(Box::new(Dialog::not_utf8_program(
                &tr!("dlg-run-title"),
                &path,
            ))),
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
            pid: Some(pid),
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
        let ok = Some(crate::dialog::DialogHit::Button(0));
        h.state.dialog_press(0, ok, 0);
        h.state.dialog_release_button();
        assert_eq!(h.state.dialogs.views.len(), 1, "Run's OK did nothing");
        assert_eq!(h.state.focused, Some(t.id), "the press went to the child");
    }

    /// A Run box over `line` with a Browse child that is starting.
    fn run_with_a_child_starting(line: &str) -> (Harness, u64) {
        let mut h = Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.output("o", 1024, 768);
        h.state.open_dialog(Dialog::run(&[line.to_string()]));
        let run = h.state.dialogs.views[0].id;
        h.state.dialogs.browsing.push(Browsing {
            pid: None,
            over: run,
            pidfd: None,
        });
        crate::spawn::record();
        (h, run)
    }

    /// A click through the pointer on button `i` of Run, the only dialog.
    fn click_button(h: &mut Harness, i: usize) {
        let v = &h.state.dialogs.views[0];
        let r = v.layout.buttons[i];
        let (x, y) = (v.x + r.x + r.width / 2, v.y + r.y + r.height / 2);
        h.click(f64::from(x), f64::from(y));
    }

    #[test]
    fn run_takes_no_key_or_press_but_cancel_while_its_child_starts() {
        const ENTER: u32 = 28;
        const ESCAPE: u32 = 1;
        let (mut h, run) = run_with_a_child_starting("true");
        h.key(ENTER, true);
        h.key(ENTER, false);
        click_button(&mut h, 0);
        h.run_for(200);
        assert!(h.state.dialogs.checking.is_empty(), "no line is checked");
        assert_eq!(
            crate::spawn::recorded(),
            Vec::<String>::new(),
            "nothing runs"
        );
        assert_eq!(h.state.dialogs.views.len(), 1, "Run stays up");
        assert!(h.state.dialogs.browsing.iter().any(|b| b.over == run));

        h.key(ESCAPE, true);
        h.key(ESCAPE, false);
        assert!(h.state.dialogs.views.is_empty(), "Escape closes Run");
        assert!(h.state.dialogs.browsing.is_empty(), "and lets the child go");

        let (mut h, _) = run_with_a_child_starting("true");
        click_button(&mut h, 1);
        assert!(h.state.dialogs.views.is_empty(), "Cancel closes Run");
        assert!(h.state.dialogs.browsing.is_empty());
    }

    #[test]
    fn cancel_on_run_under_its_child_ends_the_child() {
        let mut h = Harness::new();
        let (_run, _c, t) = run_under_child(&mut h, CHILD + 4);
        h.state.move_to(t.id, 600, 450);
        click_button(&mut h, 1);
        assert!(h.state.dialogs.views.is_empty(), "Cancel closes Run");
        assert!(h.state.dialogs.browsing.is_empty(), "its child is let go");
        assert_eq!(h.state.win(t.id).and_then(|w| w.over_dialog), None);
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
            run,
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
    fn the_child_closing_under_the_taskbars_keys_hands_escape_back_to_run() {
        const ESCAPE: u32 = 1;
        let mut h = Harness::new();
        let (run, mut c, t) = run_under_child(&mut h, CHILD + 5);
        h.pointer_to(512.0, 300.0);
        h.state.focus_taskbar(false);
        c.destroy(t);
        h.roundtrip(&mut c);
        assert!(h.state.bar_shared.keys.is_some(), "the bar keeps the keys");
        assert!(!h.state.dialogs.active);
        h.key(ESCAPE, true);
        h.key(ESCAPE, false);
        assert!(h.state.bar_shared.keys.is_none());
        assert_eq!(h.state.active_dialog_id(), Some(run));
        assert!(h.state.dialogs.active, "Run has the keyboard");
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
    fn browse_pressed_twice_before_the_child_maps_starts_one_child() {
        use std::os::unix::fs::PermissionsExt;
        const ALT: u32 = 56;
        const B: u32 = 48;
        let mut h = Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.output("o", 1024, 768);
        let dir = crate::harness::scratch().join(format!("browse-twice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let started = dir.join("started");
        let _ = std::fs::remove_file(&started);
        let portal = dir.join("reprise-portal");
        let script = format!(
            "#!/bin/sh\necho $$ >> '{}'\nexec sleep 30\n",
            started.display()
        );
        std::fs::write(&portal, script).unwrap();
        std::fs::set_permissions(&portal, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        (h.state.env.login).push(("PATH", format!("{}:{path}", dir.display())));
        let starts = || {
            std::fs::read_to_string(&started)
                .map(|s| s.lines().count())
                .unwrap_or(0)
        };

        h.state.open_dialog(Dialog::run(&[]));
        h.key(ALT, true);
        for _ in 0..2 {
            h.key(B, true);
            h.key(B, false);
        }
        h.key(ALT, false);
        h.until("the child is tracked", |s| {
            !s.dialogs.browsing.is_empty() && starts() > 0
        });
        h.run_for(300);
        assert_eq!(starts(), 1, "one file dialog starts");
        assert_eq!(h.state.dialogs.browsing.len(), 1, "one child is tracked");

        h.state.close_all_dialogs();
        assert!(h.state.dialogs.browsing.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_child_leaves_runs_field_alone() {
        let mut h = Harness::new();
        h.output("o", 1024, 768);
        h.state.open_dialog(Dialog::run(&["notepad".to_string()]));
        let run = h.state.dialogs.views[0].id;
        h.state.browse_started(Browsing {
            pid: Some(CHILD + 2),
            over: run,
            pidfd: None,
        });
        let before = h.state.dialogs.views[0].dialog.field.clone();
        h.state.browse_answered(run, Output::default());
        assert_eq!(h.state.dialogs.views[0].dialog.field, before);
        assert!(h.state.dialogs.active, "Run has the keyboard");
    }
}
