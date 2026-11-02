//! A path completion's folder read on a listing thread ([`crate::listjob`]), so a mount that
//! stopped answering holds that thread and not the loop.

use reprisekit::controls::pathcomplete::{Entry, PathComplete, Want};
use reprisekit::textedit::TextEdit;
use std::path::Path;

use crate::Waker;
use crate::listjob::Job;

/// The read a [`PathComplete`] asked for, while it runs.
#[derive(Default)]
pub struct PathJob(Option<(Want, Job<Vec<Entry>>)>);

impl PathJob {
    /// Starts the read `pc` wants, if any, waking `wake` once it answers. `base` is the folder a
    /// relative folder part names from.
    pub fn start(&mut self, pc: &mut PathComplete, base: Option<&Path>, wake: &Waker) {
        let Some(want) = pc.take_want() else {
            return;
        };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let dir = reprisefs::complete::folder(&want.dir, base, home.as_deref());
        let job = Job::spawn(Some(wake), move || {
            dir.map(|d| reprisefs::complete::entries(&d))
                .unwrap_or_default()
                .into_iter()
                .map(|e| Entry {
                    name: e.name,
                    dir: e.dir,
                })
                .collect()
        });
        self.0 = Some((want, job));
    }

    /// Hands a read that answered to `pc` and starts the next it wants; true when `edit` or the
    /// list changed.
    pub fn land(
        &mut self,
        pc: &mut PathComplete,
        edit: &mut TextEdit,
        base: Option<&Path>,
        wake: &Waker,
    ) -> bool {
        let Some(entries) = self.0.as_ref().and_then(|(_, job)| job.take()) else {
            return false;
        };
        let (want, _) = self.0.take().expect("a read in flight");
        let changed = pc.set_listing(edit, &want, entries);
        self.start(pc, base, wake);
        changed
    }
}
