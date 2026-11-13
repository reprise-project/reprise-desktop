//! Theme icons and icon files [`IconCache::get_unread`] lacked, decoded on threads so the loop
//! draws no SVG. Each batch lands on the loop and wakes it: `Event::Wake` and the file dialogs
//! take what landed.

use crate::Host;
use crate::listjob::{MAX_ALL_THREADS, MAX_THREADS, STUCK};
use crate::pool::Pool;
use reprisekit::icontheme::{IconCache, decode_file, load_icon};
use reprisekit::raster::Canvas;
use std::collections::HashSet;
use std::path::Path;

/// Theme names one thread loads: a cold folder's icons land in a few batches.
const THEME_JOB: usize = 16;

/// Icon file decodes, bounded as folder listings are: a file on a mount that never answers
/// holds its thread, and past the bounds later files wait for one to come back.
static FILES: Pool = Pool::new("icon file", MAX_THREADS, MAX_ALL_THREADS, STUCK);

/// Icon files (and px) decoding or waiting for a thread: one on a mount that never answers holds
/// one thread however often it is asked for.
#[derive(Default)]
pub(crate) struct IconJobs {
    files: HashSet<(String, i32)>,
}

type Art = Vec<(String, i32, Option<Canvas>)>;

/// The icons a surface wears blank stand-ins for while they decode off the loop. The surface
/// fetches through [`IconWaits::get`] and, once [`IconWaits::landed`], wears them all again
/// through a fresh `IconWaits`.
#[derive(Debug, Default)]
pub struct IconWaits(Vec<(String, i32)>);

impl IconWaits {
    /// `name` at `px` as [`IconCache::get_unread`] reads it: Err with a blank stand-in, `name`
    /// noted, while it decodes.
    pub fn get(
        &mut self,
        icons: &mut IconCache,
        name: &str,
        px: i32,
    ) -> Result<Option<Canvas>, Canvas> {
        match icons.get_unread(name, px) {
            Some(icon) => Ok(icon.map(std::sync::Arc::unwrap_or_clone)),
            None => {
                if !self.0.iter().any(|(n, p)| n == name && *p == px) {
                    self.0.push((name.to_string(), px));
                }
                let s = icons.scale();
                Err(Canvas::new((px * s) as u32, (px * s) as u32).at_scale(s))
            }
        }
    }

    /// The first of `names` that decodes, or a blank stand-in while one ahead of it decodes.
    pub fn first(&mut self, icons: &mut IconCache, names: &[&str], px: i32) -> Option<Canvas> {
        for name in names {
            match self.get(icons, name, px) {
                Ok(None) => {}
                Ok(icon) => return icon,
                Err(blank) => return Some(blank),
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a name waited on has landed in `icons`.
    pub fn landed(&self, icons: &mut IconCache) -> bool {
        (self.0.iter()).any(|(name, px)| icons.get_unread(name, *px).is_some())
    }
}

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
    /// Starts decoding what [`IconCache::get_unread`] lacked since the last call. Runs before
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
                        host.ctx.icons.put_icons_at(&job_theme, art, scale);
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
            if !self.icon_jobs.files.insert((file.clone(), px)) {
                continue;
            }
            let tx = self.portal_done.clone();
            FILES.run(Box::new(move || {
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
            }));
        }
    }
}
