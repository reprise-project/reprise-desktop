//! Actions asked for by hand (rename, New Folder, Properties, Delete) run on a listing thread under
//! a [`Gate`]. One that has not answered within [`ACTION_DEADLINE`] gives up with an error box, and
//! its answer is dropped whenever it comes; one whose change had begun re-lists the folder then.

use crate::Explorer;
use repriseapp::listjob::Job;
use repriseapp::{Ctx, Reply};
use reprisefs::deadline::{Gate, Landing};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub(crate) use reprisefs::deadline::ACTION_DEADLINE;

/// What an action does to files, which decides what its give-up says.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Action {
    /// The error box's title.
    pub title: &'static str,
    /// The work changes files after [`Gate::change`].
    pub changes: bool,
}

impl Action {
    pub(crate) fn reads(title: &'static str) -> Action {
        Action {
            title,
            changes: false,
        }
    }

    pub(crate) fn changes(title: &'static str) -> Action {
        Action {
            title,
            changes: true,
        }
    }
}

/// When a wait on the file system gives up, with the timer that wakes the window then. Dropping it
/// ends the timer.
pub(crate) struct Deadline {
    until: Instant,
    _timer: Sender<()>,
}

impl Deadline {
    /// A deadline `after` from now that wakes the window as it passes.
    pub(crate) fn start(after: Duration, ctx: &Ctx) -> Deadline {
        let (timer, ended) = std::sync::mpsc::channel::<()>();
        let wake = ctx.wake.clone();
        let spawned = std::thread::Builder::new()
            .name("reprise-explorer-deadline".into())
            .spawn(move || {
                if ended.recv_timeout(after) == Err(RecvTimeoutError::Timeout) {
                    wake.wake();
                }
            });
        if let Err(e) = spawned {
            log::warn!("explorer: no deadline timer ({e}); the next wake checks it");
        }
        Deadline {
            until: Instant::now() + after,
            _timer: timer,
        }
    }

    pub(crate) fn passed(&self, now: Instant) -> bool {
        now >= self.until
    }
}

type Take = Box<dyn FnMut(&mut Explorer, &mut Ctx) -> Option<Reply>>;

/// An action still out after the listing wait.
pub(crate) struct Acting {
    gate: Gate,
    action: Action,
    deadline: Deadline,
    /// Runs what follows the answer once it has come; None until then.
    take: Take,
}

impl Explorer {
    /// Runs `work` for `action` on a listing thread and waits the listing wait for it: `then` runs
    /// with its answer now, or on the wake it lands on. Past the deadline an error box titled after
    /// the action says the location is not responding, and the answer is dropped.
    pub(crate) fn act<T: Send + 'static>(
        &mut self,
        ctx: &mut Ctx,
        action: Action,
        work: impl FnOnce(&Gate) -> T + Send + 'static,
        then: impl FnOnce(&mut Explorer, T, &mut Ctx) -> Reply + 'static,
    ) -> Reply {
        let gate = Gate::default();
        let g = gate.clone();
        let (late, wake) = (self.landed_late.clone(), ctx.wake.clone());
        let read = move || {
            if g.given_up() {
                return None;
            }
            let answer = work(&g);
            match g.land() {
                Landing::Landed => Some(answer),
                Landing::Refresh => {
                    log::info!("explorer: {} landed past its deadline", action.title);
                    late.store(true, Ordering::Release);
                    wake.wake();
                    None
                }
                Landing::Dropped => {
                    log::info!("explorer: {} answered past its deadline", action.title);
                    None
                }
            }
        };
        let job = match self.read_job(ctx, read) {
            Ok(Some(answer)) => return then(self, answer, ctx),
            Ok(None) => return Reply::none(),
            Err(job) => job,
        };
        self.acting.push(Acting {
            gate,
            action,
            deadline: Deadline::start(self.action_deadline, ctx),
            take: taker(job, then),
        });
        Reply::none()
    }

    /// Runs what follows each action that answered since the last wake, and gives up those past
    /// their deadline.
    pub(crate) fn take_acting(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let mut reply: Option<Reply> = None;
        if self.landed_late.swap(false, Ordering::AcqRel) {
            reply = Some(self.refresh(ctx));
        }
        let mut keep = Vec::new();
        for mut a in std::mem::take(&mut self.acting) {
            if let Some(r) = (a.take)(self, ctx) {
                reply = Some(and(reply, r));
                continue;
            }
            if a.deadline.passed(Instant::now())
                && let Some(changing) = a.gate.give_up()
            {
                log::warn!(
                    "explorer: {} gave up after {:?}",
                    a.action.title,
                    self.action_deadline
                );
                let text = reprisefs::deadline::not_responding(a.action.changes, changing);
                let r = self.message_box(a.action.title, text, ctx);
                reply = Some(and(reply, r));
                continue;
            }
            keep.push(a);
        }
        self.acting.extend(keep);
        reply
    }
}

fn and(reply: Option<Reply>, r: Reply) -> Reply {
    match reply {
        Some(was) => was.and(r),
        None => r,
    }
}

fn taker<T: Send + 'static>(
    job: Job<Option<T>>,
    then: impl FnOnce(&mut Explorer, T, &mut Ctx) -> Reply + 'static,
) -> Take {
    let mut then = Some(then);
    Box::new(move |ex, ctx| {
        let answer = job.take()?;
        let then = then.take()?;
        Some(match answer {
            Some(answer) => then(ex, answer, ctx),
            None => Reply::none(),
        })
    })
}

#[cfg(test)]
mod tests {
    use crate::Explorer;
    use crate::acting::Action;
    use crate::dialogs::Dialog;
    use crate::place::Place;
    use crate::tests::{test_ctx, test_explorer, wake_until};
    use repriseapp::{Ctx, Reply};
    use reprisefs::deadline::Gate;
    use reprisekit::textedit::TextEdit;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn message(ex: &Explorer) -> Option<(&str, &str)> {
        match &ex.dialog {
            Some(Dialog::Message { title, text }) => Some((title, text)),
            _ => None,
        }
    }

    fn rename(ex: &mut Explorer, to: &str, ctx: &mut Ctx) {
        ex.sv.selection.set_only(Some(0));
        ex.sv.edit = Some((0, TextEdit::new(to, false)));
        ex.commit_rename(ctx);
    }

    #[test]
    fn a_rename_that_answers_late_lands_on_a_later_wake() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("one.txt"), "1").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        let gate = Arc::new(RwLock::new(()));
        let closed = gate.write().expect("lock");
        ex.held = Some(gate.clone());
        rename(&mut ex, "two.txt", &mut ctx);
        assert!(dir.path().join("one.txt").exists(), "nothing renamed yet");
        assert!(ex.busy(), "a close waits for the rename");

        drop(closed);
        ex.held = None;
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        assert!(dir.path().join("two.txt").is_file());
        assert_eq!(ex.selected_paths(), [dir.path().join("two.txt")]);
        assert!(message(&ex).is_none());
    }

    #[test]
    fn a_rename_past_its_deadline_gives_up_and_its_late_answer_renames_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("one.txt"), "1").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.action_deadline = Duration::from_millis(20);
        let gate = Arc::new(RwLock::new(()));
        let closed = gate.write().expect("lock");
        ex.held = Some(gate.clone());
        rename(&mut ex, "two.txt", &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| message(ex).is_some());
        assert_eq!(
            message(&ex),
            Some((
                "Rename",
                "The location is not responding. Nothing was changed."
            ))
        );
        assert!(!ex.busy(), "a close no longer waits for it");

        drop(closed);
        ex.held = None;
        std::thread::sleep(Duration::from_millis(100));
        ex.wake(&mut ctx);
        assert!(
            dir.path().join("one.txt").exists(),
            "the late answer is dropped"
        );
        assert!(!dir.path().join("two.txt").exists());
    }

    #[test]
    fn a_change_that_lands_past_its_deadline_relists_the_folder() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.action_deadline = Duration::from_millis(20);
        ex.watch = None;
        ex.held = Some(Arc::new(RwLock::new(())));
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let (changing, changed) = std::sync::mpsc::channel::<bool>();
        let (at, made) = (dir.path().to_path_buf(), dir.path().join("late.txt"));
        let write = made.clone();
        let work = move |gate: &Gate| {
            changing.send(gate.change(&at)).expect("the test waits");
            wait.recv().expect("the test lets it go");
            std::fs::write(&write, "1").expect("write");
        };
        ex.act(
            &mut ctx,
            Action::changes("New Text Document"),
            work,
            |_, (), _| Reply::none(),
        );
        // Only a wake gives up, so the work is under way before the first wake checks the deadline.
        assert_eq!(changed.recv_timeout(crate::LIST_WAIT), Ok(true));
        wake_until(&mut ex, &mut ctx, |ex| message(ex).is_some());
        assert_eq!(
            message(&ex).map(|m| m.1),
            Some("The location is not responding. The change may still be made once it answers.")
        );

        go.send(()).expect("the work waits");
        wake_until(&mut ex, &mut ctx, |ex| ex.index_of(&made).is_some());
    }

    #[test]
    fn two_actions_past_their_deadline_raise_their_boxes_one_after_the_other() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("one.txt"), "1").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.action_deadline = Duration::from_millis(20);
        let gate = Arc::new(RwLock::new(()));
        let _closed = gate.write().expect("lock");
        ex.held = Some(gate.clone());
        rename(&mut ex, "two.txt", &mut ctx);
        ex.new_item(true, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        assert_eq!(message(&ex).map(|m| m.0), Some("Rename"));
        ex.dialog_event(repriseapp::Event::CloseRequest, &mut ctx);
        assert_eq!(message(&ex).map(|m| m.0), Some("New Folder"));
    }

    #[test]
    fn properties_that_answer_late_open_the_sheet_on_a_later_wake() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("one.txt");
        std::fs::write(&file, "1").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        let gate = Arc::new(RwLock::new(()));
        let closed = gate.write().expect("lock");
        ex.held = Some(gate.clone());
        ex.open_properties(&file, "one.txt", "one.txt", &mut ctx);
        assert!(ex.props.is_none(), "no sheet before the read lands");

        drop(closed);
        ex.held = None;
        wake_until(&mut ex, &mut ctx, |ex| ex.props.is_some());
        assert_eq!(ex.props.as_ref().map(|p| p.info.path.clone()), Some(file));
    }
}
