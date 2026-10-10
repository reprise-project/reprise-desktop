//! File work off the event loop: `spawn` runs the work on a thread and hands the
//! closure it returns back to the loop over a channel, where it runs with the state.

use smithay::reexports::calloop::channel::{self, Event, Sender};

use crate::compositor::State;

pub type OnLoop = Box<dyn FnOnce(&mut State) + Send>;

/// The loop's end of the jobs' channel. None until `setup`, or when the
/// channel could not join the loop: `spawn` then runs the work inline.
#[derive(Default)]
pub struct FsJobs {
    tx: Option<Sender<OnLoop>>,
}

/// Join the jobs' channel to the loop. Runs once, after `State::new`.
pub fn setup(state: &mut State) {
    let (tx, rx) = channel::channel::<OnLoop>();
    let joined = state.handle.insert_source(rx, |ev, _, state| {
        if let Event::Msg(run) = ev {
            run_one(run, state);
        }
    });
    match joined {
        Ok(_) => state.fsjobs.tx = Some(tx),
        Err(e) => log::warn!(
            "fsjobs: no channel ({}); file work runs on the loop",
            e.error
        ),
    }
}

/// Run one finished job under a guard, so a panic leaves later jobs to run.
fn run_one(run: OnLoop, state: &mut State) {
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(state)));
    if ran.is_err() {
        log::error!("fsjobs: a job panicked");
    }
}

impl State {
    /// Run `work` on a thread; what it returns runs on the loop afterwards. Without
    /// the channel it runs inline; returns false only if the thread fails to spawn
    /// and the work is lost.
    pub fn spawn_fs_job<W>(&mut self, work: W) -> bool
    where
        W: FnOnce() -> OnLoop + Send + 'static,
    {
        let Some(tx) = self.fsjobs.tx.clone() else {
            work()(self);
            return true;
        };
        let spawned = std::thread::Builder::new()
            .name("fsjobs".into())
            .spawn(move || {
                // The loop may have gone meanwhile; the result goes with it.
                let _ = tx.send(work());
            });
        if let Err(e) = spawned {
            log::warn!("fsjobs: cannot spawn a thread ({e}); the file work was dropped");
            return false;
        }
        true
    }
}
