//! Print Screen. Every screenshot goes to the clipboard as a PNG and,
//! with `[screenshot] save` on, to `Screenshots` under Pictures in
//! `[screenshot] format` (`shotfile`). PrtSc
//! takes the whole layout, Alt+PrtSc the focused window's frame as drawn,
//! Shift+PrtSc a rectangle dragged out on `regionview`'s overlay (or the
//! window a click lands on), Ctrl+Alt+PrtSc the focused window's client
//! content alone.
//!
//! With `[screenshot] delay` set, a key's shot ([`Take`]) is taken that many seconds after the
//! press, of what shows then; Shift+PrtSc's waits after the pick. A screenshot key pressed while
//! one waits cancels it and takes nothing, so two never stack. A lock cancels it too, and a delay
//! that ends with the desktop locked takes nothing.
//!
//! The first three take a still of each output under the area as it shows
//! now (`snapshot`) and put them together in layout order. The content
//! is the window's client tree rendered alone, so a window partly covered
//! or off screen comes out whole. Encoding and writing the file run off
//! the loop (`fsjobs`); the PNG is offered as `image/png` on the seat's
//! selection (`State::clip_offer`) once encoded, and a balloon says where
//! the file went once written.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use repriseconf::config::ShotFormat;
use reprisekit::tr;
use reprisescene::View;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::notifyview::OwnAction;
use crate::raster::{Canvas, Rect};
use crate::shotfile::{Failed, Plan};
use crate::window::WindowId;
use reprisefs::stillfile::write_into;

pub struct Shot {
    pub layout: Rect,
    /// The output scale: device pixels of `still` per logical pixel.
    pub scale: f32,
    /// Upright, as the layout sees the output.
    pub still: Canvas,
}

/// What a screenshot takes once its delay is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// PrtSc: the whole layout.
    Screen,
    /// Alt+PrtSc: the focused window's frame.
    Window,
    /// Ctrl+Alt+PrtSc: the focused window's client content.
    Content,
    /// Shift+PrtSc's pick, in layout coordinates.
    Area(Rect),
}

/// What a screenshot press does.
#[derive(Debug, PartialEq, Eq)]
enum Press {
    Now,
    After(Duration),
    /// Cancel the screenshot that waits.
    Cancel,
}

/// A press while a screenshot waits cancels it; else the shot waits `delay` seconds.
fn press(delay: u8, waiting: bool) -> Press {
    match (waiting, delay) {
        (true, _) => Press::Cancel,
        (false, 0) => Press::Now,
        (false, s) => Press::After(Duration::from_secs(s.into())),
    }
}

impl State {
    /// A screenshot key's shot of `take`: now, after `[screenshot] delay`, or, while one waits,
    /// none, that one cancelled.
    pub fn shoot(&mut self, take: Take) {
        let wait = match press(self.config.screenshot.delay, self.shots.waiting.is_some()) {
            Press::Cancel => {
                self.cancel_waiting_shot();
                return;
            }
            Press::Now => return self.take(take),
            Press::After(wait) => wait,
        };
        let armed = (self.handle).insert_source(Timer::from_duration(wait), move |_, _, state| {
            state.shots.waiting = None;
            if state.desktop_locked() {
                log::info!("screenshot: the desktop locked during its delay; nothing taken");
            } else {
                state.take(take);
            }
            TimeoutAction::Drop
        });
        match armed {
            Ok(token) => self.shots.waiting = Some(token),
            Err(e) => {
                log::warn!("screenshot: no timer for the delay: {}", e.error);
                self.take(take);
            }
        }
    }

    /// Cancel the screenshot that waits for its delay; true if one did.
    pub fn cancel_waiting_shot(&mut self) -> bool {
        let Some(token) = self.shots.waiting.take() else {
            return false;
        };
        self.handle.remove(token);
        log::info!("screenshot: the one waiting for its delay was cancelled");
        true
    }

    fn take(&mut self, take: Take) {
        #[cfg(test)]
        self.shots.taken.push(take);
        match take {
            Take::Screen => self.screenshot(false),
            Take::Window => self.screenshot(true),
            Take::Content => self.screenshot_content(),
            Take::Area(r) => self.screenshot_area(r),
        }
    }

    /// A screenshot of the whole layout, or of the focused window's frame
    /// when `window`. No-op for `window` with nothing focused.
    fn screenshot(&mut self, window: bool) {
        let area = if window {
            let Some(r) = self
                .shot_window()
                .and_then(|w| self.win(w))
                .map(|w| w.frame_rect())
            else {
                return;
            };
            r
        } else {
            self.layout_box(None)
        };
        self.screenshot_area(area);
    }

    /// The window Alt+PrtSc and Ctrl+Alt+PrtSc take: the focused one, or
    /// while an X11 menu (override-redirect) holds the focus, the window
    /// focused before it.
    fn shot_window(&self) -> Option<WindowId> {
        let f = self.focused?;
        if !self.win(f)?.override_redirect {
            return Some(f);
        }
        self.focus_order.iter().rev().copied().find(|&w| {
            self.win(w)
                .is_some_and(|x| x.mapped && !x.hidden() && !x.override_redirect)
                && self.on_current_desktop(w)
        })
    }

    /// The Screenshot portal's Active Window: the toplevel identifier of the window Alt+PrtSc
    /// takes. None while the desktop is locked.
    pub fn active_window_identifier(&self) -> Option<String> {
        if self.desktop_locked() {
            return None;
        }
        self.toplevels.identifier(self.shot_window()?)
    }

    /// A screenshot of `area`, in layout coordinates, as the screens show
    /// it: what no output covers is black.
    fn screenshot_area(&mut self, area: Rect) {
        let area = area.intersect(&self.layout_box(None));
        if area.is_empty() {
            return;
        }
        let outputs: Vec<_> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| !self.layout_box(Some(o)).intersect(&area).is_empty())
            .cloned()
            .collect();
        let mut shots = Vec::new();
        for o in &outputs {
            let shown = self.config.debug.overlay_in_captures;
            let still = self.without_debug_panels(shown, |s| s.output_still(o));
            let Some(still) = still else {
                log::warn!(
                    "screenshot: could not take a still of {}; nothing saved",
                    o.name()
                );
                return;
            };
            shots.push(Shot {
                layout: self.layout_box(Some(o)),
                scale: o.current_scale().fractional_scale() as f32,
                still,
            });
        }
        if shots.is_empty() {
            return;
        }
        self.deliver(move || compose(area, &shots));
    }

    /// Ctrl+Alt+PrtSc: the focused window's client content, rendered
    /// alone at the sharpest scale of the screens it stands on. No-op with
    /// nothing focused.
    fn screenshot_content(&mut self) {
        let Some((content, client)) = self.shot_window().and_then(|w| self.win(w)).map(|w| {
            let (x, y) = w.client_origin();
            (
                w.content,
                Rect::new(x, y, w.geom.client.width, w.geom.client.height),
            )
        }) else {
            return;
        };
        if client.is_empty() {
            return;
        }
        let scale = self
            .scene
            .outputs()
            .iter()
            .filter(|o| !self.layout_box(Some(o)).intersect(&client).is_empty())
            .map(|o| o.current_scale().fractional_scale())
            .fold(1.0f64, f64::max);
        let Some(renderer) = self.backend.as_mut().map(|b| &mut b.renderer) else {
            return;
        };
        if let Err(e) = crate::gpufence::ready(renderer) {
            return log::warn!("screenshot: {e}");
        }
        let view = View::new(
            Rectangle::new(
                (client.x, client.y).into(),
                (client.width, client.height).into(),
            ),
            scale,
        );
        let elements = self.scene.render_elements_of(renderer, content, &view);
        let size = crate::offscreen::physical_size((client.width, client.height).into(), scale);
        let still = crate::offscreen::Target::new(renderer, size, scale).and_then(|mut t| {
            t.render(renderer, &elements)?;
            t.read(renderer)
        });
        let mut still = match still {
            Ok(s) => s,
            Err(e) => {
                log::warn!("screenshot: could not render the window's content: {e}");
                return;
            }
        };
        for p in &mut still.pixels {
            *p |= 0xff00_0000;
        }
        self.deliver(move || still);
    }

    /// Encode `picture` off the loop and offer the PNG as the clipboard; with `[screenshot] save`
    /// on, then encode it in `[screenshot] format` on a job of its own (`shotfile::plan`) and
    /// write it to a new file on a file job.
    fn deliver(&mut self, picture: impl FnOnce() -> Canvas + Send + 'static) {
        self.event_sound(repriseaudio::sounds::Event::ScreenCapture);
        let shot = self.config.screenshot.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let saving = shot.save.then(|| self.shots.saves.hold());
        self.spawn_job(move || -> crate::fsjobs::OnLoop {
            let still = picture();
            let png: Arc<[u8]> = Arc::from(crate::raster::to_png(&still));
            Box::new(move |state| {
                state.clip_offer(&["image/png"], Arc::clone(&png));
                let Some(saving) = saving else {
                    return;
                };
                let ext = shot.format.extension();
                let wants_png = shot.format == ShotFormat::Png;
                match crate::shotfile::plan(&shot, state.shots.encodes.count()) {
                    Plan::Clipboard => return save(state, png, now, ext, None, saving, still),
                    Plan::Busy => {
                        let note = (!wants_png).then(|| tr!("screenshot-note-busy"));
                        let note = note.as_deref();
                        return save(state, png, now, "png", note, saving, still);
                    }
                    Plan::Encode => {}
                }
                let encoding = state.shots.encodes.hold();
                state.spawn_job(move || -> crate::fsjobs::OnLoop {
                    let file = crate::shotfile::encode(&still, &shot);
                    drop(encoding);
                    Box::new(move |state| match file {
                        Ok(bytes) => save(state, bytes.into(), now, ext, None, saving, still),
                        Err(e) => {
                            let note = match e {
                                Failed::Memory(budget) => tr!(
                                    "screenshot-note-memory",
                                    megabytes = (budget / 1_000_000).to_string()
                                ),
                                Failed::Late(limit) => tr!(
                                    "screenshot-note-late",
                                    minutes = (limit.as_secs() / 60).to_string()
                                ),
                                Failed::Outdated => tr!("screenshot-note-outdated"),
                                Failed::Unavailable(_) => tr!("screenshot-note-no-library"),
                                Failed::Killed(_) => tr!("screenshot-note-killed"),
                                Failed::Codec(e) => {
                                    tr!("screenshot-note-codec", error = e.to_string())
                                }
                            };
                            let note = (!wants_png).then_some(note.as_str());
                            save(state, png, now, "png", note, saving, still)
                        }
                    })
                });
            })
        });
    }
}

/// Work under way, counted while its [`Held`] lives.
#[derive(Clone, Default)]
pub struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn hold(&self) -> Held {
        self.0.fetch_add(1, Ordering::AcqRel);
        Held(Arc::clone(&self.0))
    }
}

/// One count of an [`InFlight`], given back when dropped, wherever the work ends or is lost.
pub struct Held(Arc<AtomicUsize>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The screenshots under way.
#[derive(Default)]
pub struct Shots {
    /// Screenshots whose file is not written yet.
    pub saves: InFlight,
    /// File encodes running or waiting for the one running.
    encodes: InFlight,
    /// The timer of the screenshot that waits for its delay.
    waiting: Option<RegistrationToken>,
    #[cfg(test)]
    taken: Vec<Take>,
}

/// How often [`when_saved`] looks.
const SAVED_POLL: Duration = Duration::from_millis(100);

/// Runs `then` once every screenshot file under way is written, or `wait` from now at most, with
/// a notice up meanwhile.
pub fn when_saved(state: &mut State, wait: Duration, then: Box<dyn FnOnce(&mut State)>) {
    let saves = state.shots.saves.clone();
    if saves.count() == 0 {
        return then(state);
    }
    log::info!(
        "screenshot: waiting for {} file(s) to be saved",
        saves.count()
    );
    crate::notifyview::notice(
        state,
        &tr!("screenshot-saving"),
        &tr!("screenshot-saving-body"),
    );
    let deadline = Instant::now() + wait;
    let then = Rc::new(Cell::new(Some(then)));
    let fire = Rc::clone(&then);
    let armed =
        (state.handle).insert_source(Timer::from_duration(SAVED_POLL), move |_, _, state| {
            if saves.count() > 0 && Instant::now() < deadline {
                return TimeoutAction::ToDuration(SAVED_POLL);
            }
            if saves.count() > 0 {
                log::warn!(
                    "screenshot: {} file(s) not saved in {wait:?}",
                    saves.count()
                );
            }
            if let Some(then) = fire.take() {
                then(state);
            }
            TimeoutAction::Drop
        });
    if let Err(e) = armed {
        log::warn!("screenshot: no timer to wait for the files: {}", e.error);
        if let Some(then) = then.take() {
            then(state);
        }
    }
}

/// Write the encoded file on a file job and tell how it went, with `note` saying why it is a PNG
/// and `still` fitted into the notice's picture once the file lands.
fn save(
    state: &mut State,
    bytes: Arc<[u8]>,
    secs: i64,
    ext: &'static str,
    note: Option<&str>,
    saving: Held,
    still: Canvas,
) {
    let note = note.map(str::to_owned);
    state.spawn_fs_job(move || {
        let saved = write_file(&bytes, secs, ext);
        let shown = match saved {
            Ok(_) => crate::tray::notify::canvas_sources(&still),
            Err(_) => Default::default(),
        };
        drop(still);
        Box::new(move |state| {
            saved_notice(state, saved, note.as_deref(), shown);
            drop(saving);
        })
    });
}

/// Tells how the screenshot's save went; a saved file goes in the recent files, and its notice
/// shows `shown`, opens the file on a click, and offers Edit in Paint and Show in Folder.
fn saved_notice(
    state: &mut State,
    saved: Result<PathBuf, String>,
    note: Option<&str>,
    shown: crate::tray::notify::Picture,
) {
    match saved {
        Ok(path) => {
            log::info!("screenshot saved to {}", path.display());
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let body = match note {
                Some(note) => format!("{name}\n{note}"),
                None => name.into_owned(),
            };
            let edit = vec!["reprise-paint".into(), path.clone().into_os_string()];
            let actions = vec![
                (
                    crate::tray::notify::DEFAULT_ACTION.to_string(),
                    String::new(),
                    OwnAction::Open(path.clone()),
                ),
                (
                    "edit".to_string(),
                    tr!("screenshot-action-edit"),
                    OwnAction::Run(edit),
                ),
                (
                    "folder".to_string(),
                    tr!("screenshot-action-folder"),
                    OwnAction::Run(reprisefs::apps::open_location_argv(&path)),
                ),
            ];
            crate::notifyview::notice_with_picture(
                state,
                &tr!("screenshot-saved"),
                &body,
                actions,
                shown,
            );
            state.record_recent(vec![path], None);
        }
        Err(e) => {
            log::warn!("screenshot not saved: {e}");
            crate::notifyview::notice(
                state,
                &tr!("screenshot-not-saved"),
                &tr!("screenshot-not-saved-body", error = e),
            );
        }
    }
}

/// Write `bytes` to a new file named for `secs` with extension `ext` in the Screenshots folder,
/// which is made if missing. Blocking.
fn write_file(bytes: &[u8], secs: i64, ext: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    let dir = reprisefs::places::screenshots_dir().ok_or_else(|| tr!("screenshot-no-home"))?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        tr!(
            "screenshot-cannot-make",
            path = dir.display().to_string(),
            error = e.to_string()
        )
    })?;
    let name = file_name(&reprisefs::locale::localtime(secs), ext);
    write_into(&dir.join(name), |f| f.write_all(bytes))
}

/// Remove the temp files a session that ended mid-write left in the Screenshots folder, off the
/// loop. One written in the last minute may be another session's under way, so it stays.
pub fn sweep_temps(state: &mut State) {
    state.spawn_fs_job(|| {
        if let Some(dir) = reprisefs::places::screenshots_dir() {
            let old = std::time::SystemTime::now() - Duration::from_secs(60);
            for path in stale_temps(&dir, old) {
                match std::fs::remove_file(&path) {
                    Ok(()) => log::info!("screenshot: removed {}", path.display()),
                    Err(e) => log::warn!("screenshot: {}: {e}", path.display()),
                }
            }
        }
        Box::new(|_| {})
    });
}

/// The screenshot temp files (`write_into`'s) in `dir` last written before `old`.
fn stale_temps(dir: &std::path::Path, old: std::time::SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(".Screenshot ") && name.ends_with(".tmp")
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < old)
        })
        .map(|e| e.path())
        .collect()
}

fn file_name(tm: &libc::tm, ext: &str) -> String {
    format!(
        "Screenshot {:04}-{:02}-{:02} {:02}-{:02}-{:02}.{ext}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// `area` of the layout from the outputs' stills, at the largest scale
/// among them: each pixel of the picture takes the still pixel under the
/// logical point at its centre, so an output at a smaller scale is
/// enlarged. A part of `area` no output covers is black.
pub fn compose(area: Rect, shots: &[Shot]) -> Canvas {
    let s = shots.iter().map(|s| s.scale).fold(1.0, f32::max);
    let px = |l: i32| (l as f32 * s).round() as i32;
    let (w, h) = (px(area.width.max(0)), px(area.height.max(0)));
    let mut out = Canvas::from_pixels(w as u32, h as u32, vec![0xff00_0000; (w * h) as usize]);
    for shot in shots {
        let (bw, bh) = (shot.still.width as i32, shot.still.height as i32);
        let r = shot.layout.intersect(&area);
        // The still's pixel under picture pixel `p` along one axis, the
        // picture starting at logical `from` and the still at `origin`.
        let still_at = |p: i32, from: i32, origin: i32| {
            ((from as f32 + (p as f32 + 0.5) / s - origin as f32) * shot.scale).floor() as i32
        };
        for y in px(r.y - area.y)..px(r.bottom() - area.y) {
            let ly = still_at(y, area.y, shot.layout.y);
            for x in px(r.x - area.x)..px(r.right() - area.x) {
                let lx = still_at(x, area.x, shot.layout.x);
                if !(0..bw).contains(&lx) || !(0..bh).contains(&ly) {
                    continue;
                }
                let p = shot.still.pixels[(ly * bw + lx) as usize];
                out.pixels[(y * w + x) as usize] = 0xff00_0000 | p;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(layout: Rect, scale: f32, w: u32, h: u32, px: &[u32]) -> Shot {
        Shot {
            layout,
            scale,
            still: Canvas::from_pixels(w, h, px.to_vec()),
        }
    }

    #[test]
    fn compose_places_outputs_by_layout_and_fills_gaps_black() {
        // A 2x2 output with a 1x1 one to its right, top-aligned.
        let a = shot(Rect::new(0, 0, 2, 2), 1.0, 2, 2, &[1, 2, 3, 4]);
        let b = shot(Rect::new(2, 0, 1, 1), 1.0, 1, 1, &[5]);
        let c = compose(Rect::new(0, 0, 3, 2), &[a, b]);
        assert_eq!((c.width, c.height), (3, 2));
        let k = 0xff00_0000;
        assert_eq!(c.pixels, [k | 1, k | 2, k | 5, k | 3, k | 4, k]);
    }

    #[test]
    fn compose_crops_to_the_area() {
        let a = shot(Rect::new(0, 0, 2, 2), 1.0, 2, 2, &[1, 2, 3, 4]);
        let c = compose(Rect::new(1, 1, 1, 1), &[a]);
        assert_eq!(c.pixels, [0xff00_0004]);
    }

    #[test]
    fn compose_enlarges_a_scale_1_output_beside_a_scale_2_one() {
        let hi = shot(Rect::new(0, 0, 1, 1), 2.0, 2, 2, &[1, 2, 3, 4]);
        let lo = shot(Rect::new(1, 0, 1, 1), 1.0, 1, 1, &[5]);
        let c = compose(Rect::new(0, 0, 2, 1), &[hi, lo]);
        assert_eq!((c.width, c.height), (4, 2));
        let k = 0xff00_0000;
        assert_eq!(
            c.pixels,
            [k | 1, k | 2, k | 5, k | 5, k | 3, k | 4, k | 5, k | 5]
        );
    }

    #[test]
    fn compose_takes_a_1_5_output_at_its_own_pixels_beside_a_scale_1_one() {
        // 2x2 logical at 1.5 is a 3x3 still; the picture is 1.5x too.
        let hi = shot(
            Rect::new(0, 0, 2, 2),
            1.5,
            3,
            3,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
        );
        let lo = shot(Rect::new(2, 0, 2, 2), 1.0, 2, 2, &[10, 11, 12, 13]);
        let c = compose(Rect::new(0, 0, 4, 2), &[hi, lo]);
        assert_eq!((c.width, c.height), (6, 3));
        let rgb: Vec<u32> = c.pixels.iter().map(|p| p & 0xff_ffff).collect();
        assert_eq!(
            rgb,
            [
                1, 2, 3, 10, 11, 11, 4, 5, 6, 12, 13, 13, 7, 8, 9, 12, 13, 13
            ]
        );
    }

    fn tm() -> libc::tm {
        // SAFETY: `libc::tm` is plain data; all zeroes is a valid value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 126;
        tm.tm_mon = 8;
        tm.tm_mday = 5;
        tm.tm_hour = 7;
        tm.tm_min = 3;
        tm.tm_sec = 9;
        tm
    }

    #[test]
    fn the_file_is_named_for_the_local_date_and_time() {
        assert_eq!(
            file_name(&tm(), "png"),
            "Screenshot 2026-09-05 07-03-09.png"
        );
    }

    #[test]
    fn the_file_name_takes_the_formats_extension() {
        assert_eq!(
            file_name(&tm(), "jxl"),
            "Screenshot 2026-09-05 07-03-09.jxl"
        );
    }

    fn names_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_saved_shot_s_notice_shows_it_offers_edit_and_its_folder_and_records_it() {
        use crate::notifyview::{OWN_ID, invoke};
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = crate::harness::scratch().join("shot-notice");
        std::fs::create_dir_all(&dir).unwrap();
        let xbel = dir.join("recently-used.xbel");
        h.state.config.recent_files = true;
        h.state.recent = crate::recentdocs::RecentDocs::reading(xbel.clone());
        let path = dir.join("Screenshot 2026-09-05 07-03-09.png");
        let mut wide = Canvas::new(40, 20);
        wide.fill_rect(Rect::new(0, 0, 40, 20), crate::theme::Color::rgb(0, 0, 255));
        std::fs::write(&path, crate::raster::to_png(&wide)).unwrap();
        let shown = crate::tray::notify::canvas_sources(&wide);
        saved_notice(&mut h.state, Ok(path.clone()), None, shown);

        let n = &h.state.notify.log.get(OWN_ID).unwrap().n;
        assert_eq!(n.summary, "Screenshot saved");
        assert_eq!(n.body, "Screenshot 2026-09-05 07-03-09.png");
        let buttons: Vec<&str> = (n.actions.iter())
            .filter(|(_, label)| !label.is_empty())
            .map(|(_, label)| label.as_str())
            .collect();
        assert_eq!(buttons, ["Edit", "Show in Folder"]);
        let picture = n.image.iter().max_by_key(|s| s.width).expect("the picture");
        let at = |x: u32, y: u32| picture.pixels[(y * picture.width + x) as usize];
        assert_eq!(picture.width, 96);
        assert_eq!(at(48, 48), 0xFF00_00FF, "the shot fills the middle");
        assert_eq!(at(48, 4), 0, "and keeps its shape, clear above");

        crate::spawn::record();
        invoke(&mut h.state, OWN_ID, "edit");
        assert_eq!(crate::spawn::recorded(), ["reprise-paint"]);
        h.until("the shot is in the recent files", |_| {
            std::fs::read_to_string(&xbel).is_ok_and(|t| t.contains("Screenshot%202026-09-05"))
        });
    }

    #[test]
    fn a_write_that_fails_leaves_nothing_and_one_that_lands_takes_a_free_number() {
        use std::io::Write;
        let dir = crate::harness::scratch().join("shot-write");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Shot.jxl");
        let failed = write_into(&path, |f| {
            f.write_all(b"half")?;
            Err(std::io::Error::other("disk gone"))
        });
        assert!(failed.unwrap_err().ends_with("disk gone"));
        assert!(names_in(&dir).is_empty(), "no file under any name");
        for _ in 0..2 {
            write_into(&path, |f| f.write_all(b"whole")).unwrap();
        }
        assert_eq!(names_in(&dir), ["Shot (2).jxl", "Shot.jxl"]);
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");
    }

    #[test]
    fn a_temp_left_by_an_ended_session_goes_and_one_under_way_stays() {
        let dir = crate::harness::scratch().join("shot-temps");
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            ".Screenshot 2026-09-05 07-03-09.jxl.00ff00ff00ff00ff.tmp",
            "Screenshot 2026-09-05 07-03-09.jxl",
            ".other.tmp",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let later = std::time::SystemTime::now() + Duration::from_secs(1);
        assert_eq!(
            stale_temps(&dir, later),
            [dir.join(".Screenshot 2026-09-05 07-03-09.jxl.00ff00ff00ff00ff.tmp")]
        );
        let earlier = std::time::SystemTime::now() - Duration::from_secs(60);
        assert!(stale_temps(&dir, earlier).is_empty(), "written just now");
    }

    #[test]
    fn a_wait_for_screenshot_files_ends_at_its_deadline() {
        let mut h = crate::harness::Harness::new();
        let _saving = h.state.shots.saves.hold();
        let ran = Rc::new(Cell::new(false));
        let set = Rc::clone(&ran);
        let wait = Duration::from_millis(300);
        when_saved(&mut h.state, wait, Box::new(move |_| set.set(true)));
        h.run_for(150);
        assert!(!ran.get(), "the file is under way");
        h.run_for(400);
        assert!(ran.get(), "past the deadline");
    }

    #[test]
    fn a_press_waits_the_delay_and_one_while_a_shot_waits_cancels_it() {
        assert_eq!(press(0, false), Press::Now);
        assert_eq!(press(3, false), Press::After(Duration::from_secs(3)));
        assert_eq!(press(3, true), Press::Cancel);
        assert_eq!(press(0, true), Press::Cancel, "a delay set to 0 meanwhile");
    }

    #[test]
    fn a_second_press_cancels_the_waiting_shot_and_a_third_waits_again() {
        let mut h = crate::harness::Harness::new();
        h.state.config.screenshot.delay = 1;
        h.state.shoot(Take::Window);
        assert!(h.state.shots.waiting.is_some(), "the first press waits");
        h.state.shoot(Take::Screen);
        assert!(h.state.shots.waiting.is_none(), "the second cancels it");
        h.state.shoot(Take::Content);
        assert!(h.state.shots.waiting.is_some(), "the third waits again");
        assert_eq!(h.state.shots.taken, [], "nothing is taken before the delay");
        h.until("the delay ends", |s| s.shots.waiting.is_none());
        assert_eq!(
            h.state.shots.taken,
            [Take::Content],
            "only the third is taken"
        );
    }

    #[test]
    fn a_lock_cancels_the_waiting_shot_and_a_delay_ending_under_it_takes_nothing() {
        let mut h = crate::harness::Harness::new();
        h.state.config.screenshot.delay = 1;
        h.state.shoot(Take::Content);
        crate::lockview::lock(&mut h.state);
        assert!(h.state.shots.waiting.is_none(), "the lock cancels it");
        h.state.shoot(Take::Screen);
        assert!(
            h.state.shots.waiting.is_some(),
            "a shot armed under the lock waits"
        );
        h.until("the delay ends", |s| s.shots.waiting.is_none());
        assert_eq!(h.state.shots.taken, [], "nothing is taken under the lock");
    }

    #[test]
    fn prtsc_over_an_open_start_menu_shoots_and_leaves_it_open() {
        const SHIFT: u32 = 42;
        const SYSRQ: u32 = 99;
        for style in [
            crate::config::StartMenuStyle::Classic,
            crate::config::StartMenuStyle::Blue,
        ] {
            let mut h = crate::harness::Harness::new();
            h.state.config.startmenu_style = style;
            h.output("TEST-1", 1024, 768);
            h.state.toggle_start_from_keys();
            h.dispatch();
            let open = |s: &State| !s.menus.is_empty() || s.start_panel.is_some();
            assert!(open(&h.state), "{style:?} Start is open");
            h.key(SYSRQ, true);
            h.key(SYSRQ, false);
            assert_eq!(h.state.shots.taken, [Take::Screen], "{style:?}");
            assert!(open(&h.state), "{style:?} Start stays open for the picture");
            h.key(SHIFT, true);
            h.key(SYSRQ, true);
            h.key(SYSRQ, false);
            h.key(SHIFT, false);
            assert!(
                h.state.panels.region.is_some(),
                "{style:?} the overlay opens"
            );
            assert!(open(&h.state), "{style:?} Start stays open under it");
        }
    }
}
