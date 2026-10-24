//! The user picture, read off the loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::compositor::State;
use crate::raster::Canvas;

use super::PICTURE;

pub(super) fn picture_paths(home: Option<&Path>, user: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = home.map(|h| h.join(".face")).into_iter().collect();
    if !user.is_empty() && !user.contains('/') {
        v.push(Path::new("/var/lib/AccountsService/icons").join(user));
    }
    v
}

/// The picture files carry no extension, so the icon cache does not take them.
fn load_picture(path: &Path) -> Option<Canvas> {
    const MAX_BYTES: u64 = 4 << 20;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    reprisekit::icontheme::png_to_canvas(&bytes, 4096)
        .map_err(|e| log::debug!("user picture {}: {e}", path.display()))
        .ok()
}

/// The first of `paths` that decodes, at each of `sizes` device pixels.
pub(super) fn read_pictures(paths: &[PathBuf], sizes: &[i32]) -> Vec<(i32, Option<Canvas>)> {
    let picture = paths.iter().find_map(|p| load_picture(p));
    (sizes.iter())
        .map(|&px| {
            let c = picture
                .as_ref()
                .map(|c| c.resample_box(px as u32, px as u32));
            (px, c)
        })
        .collect()
}

/// The user picture at each size Start shows it, read on a file job: `~/.face` lives in the
/// home folder, which may be a mount that never answers.
#[derive(Default)]
pub struct UserPictures {
    /// What the last read found at each device-pixel size, None where it found no picture.
    read: HashMap<i32, Option<Canvas>>,
    reads: crate::fsjobs::Reads,
}

impl UserPictures {
    /// The picture the last read found at `px`; None before any read at that size.
    pub(super) fn at(&self, px: i32) -> Option<Option<&Canvas>> {
        self.read.get(&px).map(Option::as_ref)
    }

    /// The picture to show at `px`: the read at `px`, else the largest picture read at another
    /// size resampled while `px` is still being read. None shows avatar-default.
    pub(super) fn shown(&self, px: i32) -> Option<Canvas> {
        match self.at(px) {
            Some(found) => found.cloned(),
            None => (self.read.values().flatten())
                .max_by_key(|c| c.width)
                .map(|c| c.resample_box(px as u32, px as u32)),
        }
    }

    /// The read's id and the sizes it takes, or None while one runs and is not stuck.
    pub(super) fn start(&mut self, mut sizes: Vec<i32>, now: Instant) -> Option<(u64, Vec<i32>)> {
        let id = self.reads.start(now, ())?;
        sizes.sort_unstable();
        sizes.dedup();
        Some((id, sizes))
    }

    /// Keeps what read `id` found, unless a newer read landed first.
    pub(super) fn land(&mut self, id: u64, found: Vec<(i32, Option<Canvas>)>) -> bool {
        let current = self.reads.land(id);
        if current {
            self.read.extend(found);
        }
        current
    }
}

impl State {
    /// Reads the user picture at `sizes` device pixels on a file job; an open panel shows what
    /// it finds. Each Start open reads again, so a new picture shows at the next.
    pub fn read_user_pictures(&mut self, sizes: Vec<i32>) {
        let Some((id, sizes)) = self.start_pictures.start(sizes, Instant::now()) else {
            return;
        };
        self.spawn_fs_job(move || {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let paths = picture_paths(home.as_deref(), &crate::lock::current_user());
            let found = read_pictures(&paths, &sizes);
            Box::new(move |state: &mut State| {
                if !state.start_pictures.land(id, found) {
                    return;
                }
                let Some(p) = state.start_panel.as_ref() else {
                    return;
                };
                let px = PICTURE * state.draw_scale_at(Some(&p.output)).max(1);
                let picture = (state.start_pictures.shown(px))
                    .or_else(|| state.icons.get("avatar-default", px));
                if let Some(p) = state.start_panel.as_mut()
                    && p.model.user_picture != picture
                {
                    p.model.user_picture = picture;
                    state.redraw_start_panel();
                }
            })
        });
    }

    /// Reads the user picture ahead at the Start sizes of each of draw `scales`.
    pub fn warm_user_pictures(&mut self, scales: &[i32]) {
        self.read_user_pictures(scales.iter().map(|n| PICTURE * n).collect());
    }
}
