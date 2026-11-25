//! Computer's drive space: each listed drive's mount is read on a listing thread of its own, so a
//! drive that does not answer (a dead mount) stays blank without holding the others. Every
//! Computer listing reads afresh; a read still out from an earlier listing is joined, not repeated.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::Explorer;
use crate::place::Place;
use repriseapp::Ctx;
use repriseapp::listjob::Job;
use reprisekit::controls::shellview::{self, Field, Space};

/// How long a Computer listing waits for the drives' space before it shows them blank.
const SPACE_WAIT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(100)
};

/// `(total, free)` bytes of a file system.
type Totals = (u64, u64);

/// The [`Totals`] of the file system at a mount, None when it cannot be read.
pub(crate) type Statvfs = fn(&Path) -> Option<Totals>;

#[derive(Default)]
pub(crate) struct Spaces {
    /// What the last reads answered, by mount.
    pub read: HashMap<PathBuf, Totals>,
    out: Vec<(PathBuf, Job<Option<Totals>>)>,
}

impl Spaces {
    /// The bytes `field` shows for the drive at `path`; 0 while it is blank.
    pub(crate) fn key(&self, path: &Path, field: Field) -> u64 {
        match (self.read.get(path), field) {
            (Some(&(total, _)), Field::Total) => total,
            (Some(&(_, free)), Field::Free) => free,
            _ => 0,
        }
    }

    /// How many reads of `at` are out.
    #[cfg(test)]
    pub(crate) fn out_for(&self, at: &Path) -> usize {
        self.out.iter().filter(|(m, _)| m == at).count()
    }
}

/// `(total, free)` as Details and a tile show it.
pub(crate) fn space((total, free): Totals) -> Space {
    Space {
        total,
        free,
        total_text: reprisefs::props::short_size(total),
        free_text: reprisefs::props::short_size(free),
    }
}

impl Explorer {
    /// Starts a read of every listed drive's space and waits [`SPACE_WAIT`] for them; the rest
    /// land at a wake ([`Explorer::take_space`]).
    pub(crate) fn read_space(&mut self, ctx: &Ctx) {
        self.spaces.read.clear();
        let mounts: Vec<PathBuf> = (self.items.iter())
            .filter(|it| it.drive.is_some() && self.drives.iter().any(|d| d.mount == it.path))
            .map(|it| it.path.clone())
            .collect();
        for m in mounts {
            if self.spaces.out.iter().any(|(at, _)| *at == m) {
                continue;
            }
            let stat = self.statvfs;
            let at = m.clone();
            let job = self.spawn_read(ctx, move || stat(&at));
            self.spaces.out.push((m, job));
        }
        self.take_space(SPACE_WAIT);
    }

    /// Lands the space reads that answer within `wait` on Computer's entries; whether any did.
    pub(crate) fn take_space(&mut self, wait: Duration) -> bool {
        if self.spaces.out.is_empty() {
            return false;
        }
        let until = Instant::now() + wait;
        let mut landed = Vec::new();
        self.spaces.out.retain(|(at, job)| {
            let left = until.saturating_duration_since(Instant::now());
            match job.wait(left) {
                Some(answer) => {
                    landed.push((at.clone(), answer));
                    false
                }
                None => true,
            }
        });
        if landed.is_empty() || self.place != Place::Computer {
            return false;
        }
        for (at, answer) in landed {
            // A drive with nothing on it to measure (an empty card reader) reads 0 bytes.
            let Some(sp) = answer.filter(|&(total, _)| total > 0) else {
                continue;
            };
            self.spaces.read.insert(at.clone(), sp);
            let Some(&i) = self.item_index.get(&at) else {
                continue;
            };
            if let Some(e) = self.sv.entries.get_mut(i) {
                e.space = Some(space(sp));
                e.detail = shellview::tile_detail(e);
            }
        }
        let field = self.sv.sort.and_then(|(col, _)| self.sv.fields.get(col));
        if matches!(field, Some(Field::Total | Field::Free)) {
            self.resort();
        }
        true
    }
}
