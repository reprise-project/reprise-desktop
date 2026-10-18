//! Print Screen. Every screenshot goes to the clipboard as a PNG and,
//! with `[screenshot] save` on, to `Screenshots` under Pictures. PrtSc
//! takes the whole layout, Alt+PrtSc the focused window's frame as drawn,
//! Shift+PrtSc a rectangle dragged out on `regionview`'s overlay (or the
//! window a click lands on), Ctrl+Alt+PrtSc the focused window's client
//! content alone.
//!
//! The first three take a still of each output under the area as it shows
//! now (`snapshot`) and put them together in layout order. The content
//! is the window's client tree rendered alone, so a window partly covered
//! or off screen comes out whole. Encoding and writing the file run off
//! the loop (`fsjobs`); the PNG is offered as `image/png` on the seat's
//! selection (`State::clip_offer`) once encoded, and a balloon says where
//! the file went once written.

use std::path::PathBuf;
use std::sync::Arc;

use reprisescene::View;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::raster::{Canvas, Rect};
use crate::window::WindowId;

pub struct Shot {
    pub layout: Rect,
    /// The output scale: device pixels of `still` per logical pixel.
    pub scale: f32,
    /// Upright, as the layout sees the output.
    pub still: Canvas,
}

impl State {
    /// A screenshot of the whole layout, or of the focused window's frame
    /// when `window`. No-op for `window` with nothing focused.
    pub fn screenshot(&mut self, window: bool) {
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

    /// A screenshot of `area`, in layout coordinates, as the screens show
    /// it: what no output covers is black.
    pub fn screenshot_area(&mut self, area: Rect) {
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
    pub fn screenshot_content(&mut self) {
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
    /// on, then write it to a new file on a file job.
    fn deliver(&mut self, picture: impl FnOnce() -> Canvas + Send + 'static) {
        let save = self.config.screenshot.save;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.spawn_job(move || -> crate::fsjobs::OnLoop {
            let png: Arc<[u8]> = Arc::from(crate::raster::to_png(&picture()));
            Box::new(move |state| {
                state.clip_offer(&["image/png"], Arc::clone(&png));
                if save {
                    state.spawn_fs_job(move || {
                        let saved = write_file(&png, now);
                        Box::new(move |state| saved_notice(state, saved))
                    });
                }
            })
        });
    }
}

/// Tells how the screenshot's save went.
fn saved_notice(state: &mut State, saved: Result<PathBuf, String>) {
    match saved {
        Ok(path) => {
            log::info!("screenshot saved to {}", path.display());
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let dir = path.parent().unwrap_or(std::path::Path::new("/"));
            crate::notifyview::notice_opening(
                state,
                "Screenshot saved",
                &format!("{name}\nClick here to open the Screenshots folder."),
                vec!["reprise-explorer".into(), dir.as_os_str().to_owned()],
            );
        }
        Err(e) => {
            log::warn!("screenshot not saved: {e}");
            crate::notifyview::notice(
                state,
                "Screenshot not saved",
                &format!("{e}\nThe picture is on the clipboard."),
            );
        }
    }
}

/// Write `png` to a new file named for `secs` in the Screenshots folder,
/// which is made if missing. Blocking.
fn write_file(png: &[u8], secs: i64) -> Result<PathBuf, String> {
    use std::io::Write;
    let pictures = reprisefs::places::user_dir("PICTURES", "Pictures").filter(|p| p.is_dir());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = folder(pictures, home).ok_or("There is no home folder to save it in.")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot make {}: {e}", dir.display()))?;
    let name = file_name(&reprisefs::locale::localtime(secs));
    let (mut file, path) = reprisefs::ops::create_new_numbered(&dir, &name)
        .map_err(|e| format!("Cannot write in {}: {e}", dir.display()))?;
    if let Err(e) = file.write_all(png).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("Cannot write {}: {e}", path.display()));
    }
    Ok(path)
}

/// `Screenshots` in the Pictures folder when there is one, else in home.
fn folder(pictures: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    pictures.or(home).map(|d| d.join("Screenshots"))
}

fn file_name(tm: &libc::tm) -> String {
    format!(
        "Screenshot {:04}-{:02}-{:02} {:02}-{:02}-{:02}.png",
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

    #[test]
    fn the_file_is_named_for_the_local_date_and_time() {
        // SAFETY: `libc::tm` is plain data; all zeroes is a valid value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 126;
        tm.tm_mon = 8;
        tm.tm_mday = 5;
        tm.tm_hour = 7;
        tm.tm_min = 3;
        tm.tm_sec = 9;
        assert_eq!(file_name(&tm), "Screenshot 2026-09-05 07-03-09.png");
    }

    #[test]
    fn the_folder_is_under_pictures_else_home() {
        let pics = Some(PathBuf::from("/h/Pictures"));
        let home = Some(PathBuf::from("/h"));
        assert_eq!(
            folder(pics, home.clone()),
            Some(PathBuf::from("/h/Pictures/Screenshots"))
        );
        assert_eq!(folder(None, home), Some(PathBuf::from("/h/Screenshots")));
        assert_eq!(folder(None, None), None);
    }
}
