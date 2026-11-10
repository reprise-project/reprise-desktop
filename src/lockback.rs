//! The login screen's background on the lock: what login.toml names, read under reprise-login
//! off the loop and composed once per covered output size.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use repriselogin::machine::{BACKGROUND_PICTURE, Background, MACHINE_CONFIG, MachineConfig};
use smithay::output::Output;

use crate::compositor::State;
use crate::raster::Canvas;
use crate::wallpaper::{Cut, Position};

/// The background last read and its pictures, kept across locks so a lock covers with it at once.
pub struct LoginBack {
    pub config: PathBuf,
    /// Where login.toml's `background = BACKGROUND_PICTURE` is read from.
    pub picture: PathBuf,
    key: Option<Key>,
    pictures: Vec<Arc<Canvas>>,
    /// The sizes the last read composed, landed or failed; a cover of another size reads again.
    asked: Vec<(u32, u32)>,
    reading: bool,
}

impl Default for LoginBack {
    fn default() -> LoginBack {
        LoginBack {
            config: MACHINE_CONFIG.into(),
            picture: BACKGROUND_PICTURE.into(),
            key: None,
            pictures: Vec::new(),
            asked: Vec::new(),
            reading: false,
        }
    }
}

impl LoginBack {
    /// A background is named and at least one of its pictures landed.
    pub fn shows(&self) -> bool {
        !self.pictures.is_empty()
    }

    fn get(&self, size: (u32, u32)) -> Option<&Arc<Canvas>> {
        self.pictures.iter().find(|p| (p.width, p.height) == size)
    }
}

/// What the pictures were composed from: the picture's path or a built-in's URI, and for a file
/// its length and change time.
#[derive(Clone, PartialEq)]
struct Key {
    path: PathBuf,
    stamp: Option<(u64, SystemTime)>,
}

/// A size an output shows in device pixels, at its scale.
type Want = ((u32, u32), i32);

fn wanted(state: &State) -> Vec<Want> {
    let mut want: Vec<Want> = (crate::logonview::shown_outputs(state).iter())
        .map(|o| (device_size(state, o), state.draw_scale_at(Some(o))))
        .collect();
    want.sort_by_key(|w| w.0);
    want.dedup_by_key(|w| w.0);
    want
}

fn device_size(state: &State, o: &Output) -> (u32, u32) {
    let b = state.layout_box(Some(o));
    let s = state.draw_scale_at(Some(o));
    ((b.width * s) as u32, (b.height * s) as u32)
}

/// Read login.toml again and compose its background for every output, unless a read is on its
/// way. Only under reprise-login: another display manager's screen has no background of ours.
pub fn read(state: &mut State) {
    if state.login_link.is_none() || state.lock.back.reading {
        return;
    }
    let b = &mut state.lock.back;
    b.reading = true;
    let (config, picture, key) = (b.config.clone(), b.picture.clone(), b.key.clone());
    let held: Vec<(u32, u32)> = (b.pictures.iter()).map(|p| (p.width, p.height)).collect();
    let want = wanted(state);
    state.lock.back.asked = want.iter().map(|w| w.0).collect();
    state.spawn_job(move || {
        let got = compose(&config, &picture, key.as_ref(), &held, &want);
        Box::new(move |state: &mut State| land(state, got))
    });
}

enum Read {
    Unset,
    Same,
    New(Key, Vec<Arc<Canvas>>),
}

fn compose(
    config: &Path,
    picture: &Path,
    key: Option<&Key>,
    held: &[(u32, u32)],
    want: &[Want],
) -> Read {
    let named = match std::fs::read_to_string(config) {
        Ok(text) => MachineConfig::parse(&text).0.login.background,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("lock: {}: {e}", config.display());
            }
            None
        }
    };
    let path = match named {
        None => return Read::Unset,
        Some(Background::Picture) => picture.to_path_buf(),
        Some(b @ Background::Builtin(_)) => match crate::greeter::background_path(&b) {
            Some(p) => p,
            None => return Read::Unset,
        },
    };
    let stamp = (crate::wallpaper::builtin(&path).is_none())
        .then(|| std::fs::metadata(&path).ok())
        .flatten()
        .and_then(|m| Some((m.len(), m.modified().ok()?)));
    let new = Key { path, stamp };
    if key == Some(&new) && want.iter().all(|(s, _)| held.contains(s)) {
        return Read::Same;
    }
    let pictures = (want.iter())
        .filter_map(|&((w, h), scale)| {
            let cut = Cut {
                position: Position::Stretch,
                w,
                h,
            };
            let loaded = crate::wallpaper::load(&new.path, Some(cut))?;
            let black = reprisekit::theme::Color::rgb(0, 0, 0);
            let shown = crate::wallpaper::composed_known(
                &loaded.image,
                w,
                h,
                Position::Stretch,
                black,
                scale as f32,
                None,
            );
            Some(Arc::new(match shown {
                std::borrow::Cow::Borrowed(_) => loaded.image,
                std::borrow::Cow::Owned(c) => c,
            }))
        })
        .collect();
    Read::New(new, pictures)
}

fn land(state: &mut State, got: Read) {
    let b = &mut state.lock.back;
    b.reading = false;
    match got {
        Read::Same => return,
        Read::Unset if b.key.is_none() => return,
        Read::Unset => {
            b.key = None;
            b.pictures.clear();
        }
        Read::New(key, pictures) => {
            b.key = Some(key);
            b.pictures = pictures;
        }
    }
    crate::lockview::refit(state);
}

/// Fill `wall` on `output` with the background, cleared while its size has none (a read for it
/// follows).
pub fn paint(state: &mut State, output: &Output, wall: reprisescene::BufferId) {
    state.water_lock_uncover(output, wall);
    let size = device_size(state, output);
    let scale = state.draw_scale_at(Some(output));
    match state.lock.back.get(size).cloned() {
        Some(c) => {
            crate::output::upload_scaled(&mut state.scene, wall, &c, scale);
        }
        None => {
            state.scene.buffer_clear(wall);
            if !state.lock.back.asked.contains(&size) {
                read(state);
            }
        }
    }
}

#[cfg(test)]
mod tests;
