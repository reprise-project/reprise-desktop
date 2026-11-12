//! Theme icons and icon files [`IconCache::get_unread`] lacked, decoded on threads so the loop
//! draws no SVG. Each batch lands on the loop and wakes it: `Event::Wake` and the file dialogs
//! take what landed.

use crate::Host;
use reprisekit::icontheme::{IconCache, decode_file, load_icon};
use reprisekit::raster::Canvas;
use std::collections::HashSet;
use std::path::Path;

/// Theme names one thread loads: a cold folder's icons land in a few batches.
const THEME_JOB: usize = 16;

/// Icon files (and px) a thread is decoding: one on a mount that never answers holds one thread
/// however often it is asked for.
#[derive(Default)]
pub(crate) struct IconJobs {
    files: HashSet<(String, i32)>,
}

type Art = Vec<(String, i32, Option<Canvas>)>;

/// `load` for each of `wanted`; a load that panics comes back as one that failed, so its name
/// moves on instead of waiting for good.
fn load_all(wanted: Vec<(String, i32)>, load: impl Fn(&str, i32) -> Option<Canvas>) -> Art {
    (wanted.into_iter())
        .map(|(name, px)| {
            let run = std::panic::AssertUnwindSafe(|| load(&name, px));
            let icon = std::panic::catch_unwind(run).unwrap_or_else(|_| {
                log::error!("icons: decoding {name} at {px} px panicked");
                None
            });
            (name, px, icon)
        })
        .collect()
}

/// Loads what `icons` wanted ([`IconCache::take_wanted`]) here and puts it in: for a program's
/// tests, which run no loop. True when something was wanted.
pub fn decode_now(icons: &mut IconCache) -> bool {
    let wanted = icons.take_wanted();
    if wanted.is_empty() {
        return false;
    }
    let (theme, scale) = (icons.theme(), icons.scale());
    let (files, names): (Vec<_>, Vec<_>) =
        wanted.into_iter().partition(|(n, _)| n.starts_with('/'));
    let art = load_all(names, |n, px| load_icon(&theme, n, px, scale));
    icons.put_icons(&theme, art);
    let files = load_all(files, |f, px| decode_file(Path::new(f), px, scale));
    icons.put_files(files);
    true
}

impl Host {
    /// Starts threads for what [`IconCache::get_unread`] lacked since the last call. Runs before
    /// each turn, so what a draw or event asked for decodes while the loop waits.
    pub(crate) fn decode_wanted_icons(&mut self) {
        let wanted = self.ctx.icons.take_wanted();
        if wanted.is_empty() {
            return;
        }
        let (theme, scale) = (self.ctx.icons.theme(), self.ctx.icons.scale());
        let (files, names): (Vec<_>, Vec<_>) =
            wanted.into_iter().partition(|(n, _)| n.starts_with('/'));
        for batch in names.chunks(THEME_JOB) {
            let (job_theme, job) = (theme.clone(), batch.to_vec());
            let tx = self.portal_done.clone();
            let spawned = std::thread::Builder::new()
                .name("theme icons".into())
                .spawn(move || {
                    let art = load_all(job, |n, px| load_icon(&job_theme, n, px, scale));
                    let _ = tx.send(Box::new(move |host: &mut Host| {
                        host.ctx.icons.put_icons(&job_theme, art);
                        host.ctx.wake.wake();
                    }));
                });
            if let Err(e) = spawned {
                log::warn!("cannot spawn an icon thread ({e}); theme icons decode on the loop");
                let art = load_all(batch.to_vec(), |n, px| load_icon(&theme, n, px, scale));
                self.ctx.icons.put_icons(&theme, art);
                self.ctx.wake.wake();
            }
        }
        for (file, px) in files {
            let key = (file.clone(), px);
            if !self.icon_jobs.files.insert(key.clone()) {
                continue;
            }
            let tx = self.portal_done.clone();
            let spawned = std::thread::Builder::new()
                .name("icon file".into())
                .spawn(move || {
                    let art = load_all(vec![(file, px)], |f, px| {
                        decode_file(Path::new(f), px, scale)
                    });
                    let _ = tx.send(Box::new(move |host: &mut Host| {
                        for (file, px, _) in &art {
                            host.icon_jobs.files.remove(&(file.clone(), *px));
                        }
                        host.ctx.icons.put_files_at(art, scale);
                        host.ctx.wake.wake();
                    }));
                });
            // Not read here: the file may sit on a mount that never answers. The next draw that
            // wants it asks again.
            if let Err(e) = spawned {
                log::warn!("cannot spawn an icon thread ({e})");
                self.icon_jobs.files.remove(&key);
            }
        }
    }
}
