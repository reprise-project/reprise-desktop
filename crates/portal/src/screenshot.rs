//! `org.freedesktop.impl.portal.Screenshot`: every output at its place in
//! the layout, or the output or window picked in the share dialog for an
//! interactive call, as a PNG returned as a `file://` URI. The files are a
//! hand-over to the app, kept in the runtime dir for `KEEP` up to `MAX_KEPT_BYTES`. PickColor
//! needs no prompt; the click is the consent.

use crate::colorpick::{Output, grab};
use crate::wayland::{Target, Wayland};
use reprisekit::controls::colorpick::Still;
use reprisekit::raster::Rect;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};
use zbus::{ObjectPath, OwnedVariant, Variant};

/// Long enough for an app that reads the file only when it sends it, as a chat attaches on send.
const KEEP: Duration = Duration::from_secs(60 * 60);
/// The most bytes the kept screenshots may hold: the runtime dir is a small tmpfs.
const MAX_KEPT_BYTES: u64 = 256 << 20;
/// The most pixels a desktop picture may have: 256 MiB of ARGB, two 8K outputs side by side.
const MAX_PIXELS: u64 = 1 << 26;

pub struct Screenshot {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    async fn screenshot(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        log::info!("Screenshot for {app_id:?}");
        let interactive = matches!(options.get("interactive"), Some(Variant::Bool(true)));
        let mut target = None;
        if interactive || needs_consent(&options) {
            let cancel = Arc::new(AtomicBool::new(false));
            crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
            let id = app_id.clone();
            let parent = crate::dialog::parent_handle(&parent_window);
            let asked = blocking::unblock(move || {
                if interactive {
                    pick(&id, parent, cancel).map(Some)
                } else {
                    let allowed = crate::permit::ask(
                        crate::permit::Kind::Screenshot,
                        &id,
                        Vec::new(),
                        false,
                        parent,
                        cancel,
                    )
                    .allowed();
                    allowed.then_some(None)
                }
            })
            .await;
            let _ = server.remove::<crate::Request, _>(handle.clone()).await;
            match asked {
                Some(t) => target = t,
                None => {
                    log::info!("Screenshot for {app_id:?} denied");
                    return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
                }
            }
        }
        match blocking::unblock(move || take(target)).await {
            Ok(path) => {
                let mut r = HashMap::new();
                let uri = format!("file://{}", path.display());
                r.insert(
                    "uri".to_string(),
                    Variant::from(uri).try_to_owned().expect("string"),
                );
                Ok((crate::RESPONSE_OK, r))
            }
            Err(e) => {
                log::warn!("Screenshot: {e}");
                Ok((crate::RESPONSE_OTHER, HashMap::new()))
            }
        }
    }

    async fn pick_color(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        log::info!("PickColor for {app_id:?}");
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let color = blocking::unblock(move || crate::colorpick::pick(&cancel)).await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        let Some(color) = color else {
            return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
        };
        let mut r = HashMap::new();
        r.insert(
            "color".to_string(),
            Variant::from(color).try_to_owned().expect("struct"),
        );
        Ok((crate::RESPONSE_OK, r))
    }
}

/// False only when the frontend already settled the `screenshot`
/// permission: a non-interactive call with `permission_store_checked`.
fn needs_consent(options: &HashMap<String, Variant<'_>>) -> bool {
    let flag = |key| matches!(options.get(key), Some(Variant::Bool(true)));
    flag("interactive") || !flag("permission_store_checked")
}

/// The screen or window the user picks for `app_id`; None on Cancel or a
/// frontend Close. Blocking.
fn pick(app_id: &str, parent: Option<String>, cancel: Arc<AtomicBool>) -> Option<Target> {
    let wl = Wayland::connect()
        .map_err(|e| log::warn!("Screenshot: {e}"))
        .ok()?;
    let mut types = crate::SOURCE_MONITOR;
    if wl.can_capture_toplevels() {
        types |= crate::SOURCE_WINDOW;
    }
    let entries = crate::share_entries(types, &wl.outputs(), &wl.toplevels());
    drop(wl);
    let (target, _) = crate::picker::pick(
        crate::picker::Purpose::Screenshot,
        &crate::remotedesktop::display_name(app_id),
        entries,
        None,
        false,
        parent,
        cancel,
    )?;
    Some(target)
}

/// Capture `target`, or every output when None, and write it out; blocking.
fn take(target: Option<Target>) -> Result<PathBuf, String> {
    let shot = match target {
        Some(t) => grab(&mut Wayland::connect()?, &t, true)?,
        None => desktop(&crate::colorpick::capture(true)?)?,
    };
    let dir = shot_dir()?;
    let now = SystemTime::now();
    prune(&dir, now, KEEP);
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (file, path) = reprisefs::ops::create_new_numbered(&dir, &format!("Screenshot {secs}.png"))
        .map_err(|e| e.to_string())?;
    if let Err(e) = write_png(file, &shot) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// `$XDG_RUNTIME_DIR/reprise-portal/screenshots`, made 0700 when missing.
fn shot_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("no XDG_RUNTIME_DIR")?;
    let dir = Path::new(&runtime).join("reprise-portal/screenshots");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// Delete the files in `dir` last written more than `keep` before `now`, then the oldest until
/// the rest fit in [`MAX_KEPT_BYTES`].
fn prune(dir: &Path, now: SystemTime, keep: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let files = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?;
            meta.is_file().then(|| (e.path(), modified, meta.len()))
        })
        .collect();
    for path in doomed(files, now, keep, MAX_KEPT_BYTES) {
        let _ = std::fs::remove_file(path);
    }
}

/// Which of `files` (path, last written, bytes) [`prune`] deletes: those older than `keep`, then
/// the oldest of the rest until their total is at most `cap`.
fn doomed(
    mut files: Vec<(PathBuf, SystemTime, u64)>,
    now: SystemTime,
    keep: Duration,
    cap: u64,
) -> Vec<PathBuf> {
    files.sort_by_key(|&(_, modified, _)| std::cmp::Reverse(modified));
    let mut total = 0u64;
    let mut out = Vec::new();
    for (path, modified, len) in files {
        let old = now.duration_since(modified).is_ok_and(|age| age > keep);
        if !old {
            total = total.saturating_add(len);
        }
        if old || total > cap {
            out.push(path);
        }
    }
    out
}

/// The desktop picture's size and each output's place in it: the layout's
/// bounding box at the scale of the output with the most pixels per
/// logical pixel, each place's edges rounded so neighbours meet.
fn arrange(outputs: &[(Rect, (u32, u32))]) -> ((u32, u32), Vec<Rect>) {
    let scale = outputs
        .iter()
        .filter(|(r, _)| r.width > 0)
        .map(|(r, (w, _))| f64::from(*w) / f64::from(r.width))
        .fold(1.0f64, f64::max);
    let left = outputs.iter().map(|(r, _)| r.x).min().unwrap_or(0);
    let top = outputs.iter().map(|(r, _)| r.y).min().unwrap_or(0);
    let right = outputs.iter().map(|(r, _)| r.right()).max().unwrap_or(0);
    let bottom = outputs.iter().map(|(r, _)| r.bottom()).max().unwrap_or(0);
    let at = |v: i32, origin: i32| (f64::from(v - origin) * scale).round() as i32;
    let places = outputs
        .iter()
        .map(|(r, _)| {
            let (x, y) = (at(r.x, left), at(r.y, top));
            Rect::new(x, y, at(r.right(), left) - x, at(r.bottom(), top) - y)
        })
        .collect();
    let size = (at(right, left).max(0) as u32, at(bottom, top).max(0) as u32);
    (size, places)
}

/// Every output's still drawn at its place in one picture, black between
/// them; a still of another size than its place is resampled to it.
fn desktop(outputs: &[Output]) -> Result<Still, String> {
    let geometry: Vec<_> = outputs
        .iter()
        .map(|o| (o.layout, (o.still.width, o.still.height)))
        .collect();
    let ((w, h), places) = arrange(&geometry);
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("no desktop picture {w}x{h}"));
    }
    let mut pixels = vec![0xff00_0000u32; w as usize * h as usize];
    for (o, place) in outputs.iter().zip(places) {
        blit(&mut pixels, (w, h), &o.still, place);
    }
    Ok(Still {
        logical: (w as i32, h as i32),
        width: w,
        height: h,
        pixels,
    })
}

/// `still` nearest-neighbour scaled into `place` of a `size` picture.
fn blit(pixels: &mut [u32], size: (u32, u32), still: &Still, place: Rect) {
    if place.width <= 0 || place.height <= 0 || still.width == 0 || still.height == 0 {
        return;
    }
    let (pw, ph) = (place.width as u64, place.height as u64);
    for dy in 0..place.height {
        let y = place.y + dy;
        if y < 0 || y >= size.1 as i32 {
            continue;
        }
        let sy = (dy as u64 * u64::from(still.height) / ph) as usize;
        for dx in 0..place.width {
            let x = place.x + dx;
            if x < 0 || x >= size.0 as i32 {
                continue;
            }
            let sx = (dx as u64 * u64::from(still.width) / pw) as usize;
            pixels[y as usize * size.0 as usize + x as usize] =
                still.pixels[sy * still.width as usize + sx];
        }
    }
}

fn write_png(file: std::fs::File, shot: &Still) -> Result<(), String> {
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), shot.width, shot.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity(shot.pixels.len() * 3);
    for px in &shot.pixels {
        let [_, r, g, b] = px.to_be_bytes();
        rgb.extend_from_slice(&[r, g, b]);
    }
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, bool)]) -> HashMap<String, Variant<'static>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Variant::from(*v)))
            .collect()
    }

    fn still(w: u32, h: u32, colour: u32) -> Still {
        Still {
            logical: (w as i32, h as i32),
            width: w,
            height: h,
            pixels: vec![colour; (w * h) as usize],
        }
    }

    #[test]
    fn outputs_side_by_side_sit_at_their_layout_places() {
        let (size, places) = arrange(&[
            (Rect::new(0, 0, 1920, 1080), (1920, 1080)),
            (Rect::new(1920, 200, 1280, 1024), (1280, 1024)),
        ]);
        assert_eq!(size, (3200, 1224));
        assert_eq!(
            places,
            [
                Rect::new(0, 0, 1920, 1080),
                Rect::new(1920, 200, 1280, 1024)
            ]
        );
    }

    #[test]
    fn the_picture_takes_the_finest_scale_and_starts_at_the_layouts_corner() {
        // A 4K panel at scale 2 left of a 1080p screen at scale 1.
        let (size, places) = arrange(&[
            (Rect::new(-1920, 0, 1920, 1080), (3840, 2160)),
            (Rect::new(0, 0, 1920, 1080), (1920, 1080)),
        ]);
        assert_eq!(size, (7680, 2160));
        assert_eq!(
            places,
            [Rect::new(0, 0, 3840, 2160), Rect::new(3840, 0, 3840, 2160)]
        );
        // At 1.5 the rounded edges of neighbours still meet.
        let (size, places) = arrange(&[
            (Rect::new(0, 0, 1281, 721), (1922, 1082)),
            (Rect::new(1281, 0, 1281, 721), (1922, 1082)),
        ]);
        assert_eq!(places[0].right(), places[1].x);
        assert_eq!(size.0 as i32, places[1].right());
    }

    #[test]
    fn the_desktop_picture_holds_each_output_and_black_between() {
        let outputs = [
            Output {
                layout: Rect::new(0, 0, 4, 2),
                still: still(8, 4, 0xffff_0000),
            },
            Output {
                layout: Rect::new(4, 1, 2, 2),
                still: still(2, 2, 0xff00_00ff),
            },
        ];
        let shot = desktop(&outputs).unwrap();
        assert_eq!((shot.width, shot.height), (12, 6));
        let at = |x: usize, y: usize| shot.pixels[y * 12 + x];
        assert_eq!(at(0, 0), 0xffff_0000);
        assert_eq!(at(7, 3), 0xffff_0000);
        assert_eq!(at(8, 0), 0xff00_0000, "above the second output");
        assert_eq!(at(8, 2), 0xff00_00ff);
        assert_eq!(at(11, 5), 0xff00_00ff);
        assert_eq!(at(0, 5), 0xff00_0000, "under the first output");
    }

    #[test]
    fn outputs_far_apart_in_the_layout_make_no_desktop_picture() {
        let one = |x: i32| Output {
            layout: Rect::new(x, 0, 1, 1),
            still: still(1, 1, 0xffff_ffff),
        };
        assert!(desktop(&[one(0), one(4095)]).is_ok());
        let far = desktop(&[one(0), one(1 << 26)]);
        assert!(far.is_err(), "a picture past MAX_PIXELS is refused");
    }

    #[test]
    fn pruning_deletes_only_screenshots_older_than_the_keep() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let write = |name: &str, age: u64| {
            let p = dir.path().join(name);
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(now - Duration::from_secs(age)).unwrap();
            p
        };
        let fresh = write("fresh.png", 10 * 60);
        let edge = write("edge.png", 60 * 60 - 5);
        let stale = write("stale.png", 60 * 60 + 5);
        let sub = dir.path().join("folder");
        std::fs::create_dir(&sub).unwrap();
        prune(dir.path(), now, KEEP);
        assert!(fresh.exists() && edge.exists() && sub.exists());
        assert!(!stale.exists());
    }

    #[test]
    fn pruning_keeps_the_folder_under_its_byte_cap() {
        let now = SystemTime::now();
        let file = |name: &str, age: u64, len: u64| {
            (PathBuf::from(name), now - Duration::from_secs(age), len)
        };
        let files = vec![
            file("middle.png", 20, 40),
            file("newest.png", 10, 50),
            file("oldest.png", 30, 30),
        ];
        assert_eq!(
            doomed(files.clone(), now, KEEP, 100),
            [PathBuf::from("oldest.png")]
        );
        assert_eq!(
            doomed(files.clone(), now, KEEP, 49),
            [
                PathBuf::from("newest.png"),
                PathBuf::from("middle.png"),
                PathBuf::from("oldest.png")
            ]
        );
        assert!(doomed(files, now, KEEP, 120).is_empty());
    }

    #[test]
    fn only_a_store_checked_non_interactive_call_skips_the_prompt() {
        assert!(needs_consent(&opts(&[])));
        assert!(needs_consent(&opts(&[("interactive", true)])));
        assert!(needs_consent(&opts(&[("permission_store_checked", false)])));
        assert!(needs_consent(&opts(&[
            ("interactive", true),
            ("permission_store_checked", true)
        ])));
        assert!(!needs_consent(&opts(&[("permission_store_checked", true)])));
        assert!(!needs_consent(&opts(&[
            ("interactive", false),
            ("permission_store_checked", true)
        ])));
    }
}
