//! `org.freedesktop.impl.portal.Screenshot` version 3: every output at its place in
//! the layout, the output or window picked in the share dialog for an interactive call or a
//! Window target, the area dragged in the compositor's region select for an Area target, or the
//! focused window for an Active Window target, as a PNG returned as a `file://` URI. A pick or a
//! drag is the consent; the rest ask through the Access prompt unless the frontend's permission
//! store settled it ([`plan`]). The files are a hand-over to the app, kept in the runtime dir for
//! `KEEP` up to `MAX_KEPT_BYTES`. PickColor opens the compositor's colour picker and needs no
//! prompt; the click is the consent.

use crate::stills::{Output, grab};
use crate::wayland::{Target, Wayland};
use reprisekit::controls::colorpick::Still;
use reprisekit::raster::Rect;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use zbus::{ObjectPath, OwnedVariant, Variant};

/// Long enough for an app that reads the file only when it sends it, as a chat attaches on send.
const KEEP: Duration = Duration::from_secs(60 * 60);
/// The most bytes the kept screenshots may hold: the runtime dir is a small tmpfs.
const MAX_KEPT_BYTES: u64 = 256 << 20;
/// Screenshot's `target` values; AvailableTargets is the union of those served.
const TARGET_SCREEN: u32 = 1;
const TARGET_WINDOW: u32 = 2;
const TARGET_AREA: u32 = 4;
const TARGET_ACTIVE_WINDOW: u32 = 8;
const AVAILABLE_TARGETS: u32 = TARGET_SCREEN | TARGET_WINDOW | TARGET_AREA | TARGET_ACTIVE_WINDOW;

/// The most pixels a desktop picture may have: 256 MiB of ARGB, two 8K outputs side by side.
const MAX_PIXELS: u64 = 1 << 26;

pub struct Screenshot {
    pub frontend: crate::Frontend,
    pub link: Option<crate::inputlink::Link>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn available_targets(&self) -> u32 {
        AVAILABLE_TARGETS
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
        let asker = self.frontend.check(&hdr).await?;
        log::info!("Screenshot for {app_id:?}");
        let plan = match plan(&options) {
            Ok(p) => p,
            Err(e) => {
                log::info!("Screenshot for {app_id:?}: {e}");
                return Ok((crate::RESPONSE_OTHER, HashMap::new()));
            }
        };
        let shot = match self
            .choose(plan, server, &handle, &asker, &app_id, &parent_window)
            .await?
        {
            Ok(Some(shot)) => shot,
            Ok(None) => {
                log::info!("Screenshot for {app_id:?} denied");
                return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
            }
            Err(e) => {
                log::warn!("Screenshot: {e}");
                return Ok((crate::RESPONSE_OTHER, HashMap::new()));
            }
        };
        match blocking::unblock(move || take(shot)).await {
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
        let asker = self.frontend.check(&hdr).await?;
        log::info!("PickColor for {app_id:?}");
        let Some(pick) = crate::inputlink::live(&self.link).and_then(|l| l.pick_color()) else {
            log::warn!("PickColor: the compositor has no colour picker");
            return Ok((crate::RESPONSE_OTHER, HashMap::new()));
        };
        Ok(picked(self.heard(server, &handle, &asker, &pick).await?))
    }
}

impl Screenshot {
    /// What `plan` takes once the user picked it or allowed it: None for a cancel or a no, Err
    /// for nothing to take.
    async fn choose(
        &self,
        plan: Plan,
        server: &zbus::ObjectServer,
        handle: &ObjectPath<'_>,
        asker: &crate::Caller,
        app_id: &str,
        parent_window: &str,
    ) -> zbus::fdo::Result<Result<Option<Shot>, String>> {
        let parent = crate::dialog::parent_handle(parent_window);
        let link = crate::inputlink::live(&self.link);
        Ok(match plan {
            Plan::Pick(types) => {
                let id = app_id.to_string();
                let picked = crate::with_request(server, handle, asker, None, move |c| {
                    pick(&id, parent, c, types)
                });
                Ok(picked.await?.map(Shot::Of))
            }
            Plan::Area => {
                let id = app_id.to_string();
                let name = blocking::unblock(move || crate::remotedesktop::display_name(&id)).await;
                match link.and_then(|l| l.pick_area(&name)) {
                    Some(p) => Ok(self.heard(server, handle, asker, &p).await?.map(Shot::Area)),
                    None => Err("the compositor has no region select".into()),
                }
            }
            Plan::Desktop { ask } => {
                let denied = ask && !self.allowed(server, handle, asker, app_id, parent).await?;
                Ok((!denied).then_some(Shot::Desktop))
            }
            Plan::ActiveWindow { ask } => {
                let Some(query) = link.and_then(|l| l.active_window()) else {
                    return Ok(Err("the compositor names no active window".into()));
                };
                // Asked before a prompt, which would take the focus.
                let Some(window) = query.answer().await else {
                    return Ok(Err("no window has the focus".into()));
                };
                let denied = ask && !self.allowed(server, handle, asker, app_id, parent).await?;
                Ok((!denied).then_some(Shot::Of(Target::Toplevel(window))))
            }
        })
    }

    /// Whether the user allows `app_id`'s screenshot in the Access prompt, which offers no Remember.
    async fn allowed(
        &self,
        server: &zbus::ObjectServer,
        handle: &ObjectPath<'_>,
        asker: &crate::Caller,
        app_id: &str,
        parent: Option<String>,
    ) -> zbus::fdo::Result<bool> {
        let id = app_id.to_string();
        crate::with_request(server, handle, asker, None, move |cancel| {
            let kind = crate::permit::Kind::Screenshot;
            crate::permit::ask(kind, &id, Vec::new(), false, parent, cancel).allowed()
        })
        .await
    }

    /// `pick`'s answer, with the `Request` at `handle` served meanwhile; None once the frontend
    /// closes it or `asker` leaves the bus.
    async fn heard<T>(
        &self,
        server: &zbus::ObjectServer,
        handle: &ObjectPath<'_>,
        asker: &crate::Caller,
        pick: &crate::inputlink::PickHandle<T>,
    ) -> zbus::fdo::Result<Option<T>> {
        let cancel = Arc::new(AtomicBool::new(false));
        let _held = crate::export_request(server, handle, cancel.clone(), asker).await?;
        let answer = futures_lite::future::or(pick.answer(), closed(&cancel)).await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        Ok(answer)
    }
}

/// None once the frontend closes the request.
async fn closed<T>(cancel: &AtomicBool) -> Option<T> {
    while !cancel.load(Ordering::Relaxed) {
        async_io::Timer::after(Duration::from_millis(100)).await;
    }
    None
}

/// PickColor's answer to a pick of 0xRRGGBB, None for a cancel: the channels are 0-1.
fn picked(rgb: Option<u32>) -> (u32, HashMap<String, OwnedVariant>) {
    let Some(rgb) = rgb else {
        return (crate::RESPONSE_CANCELLED, HashMap::new());
    };
    let [_, r, g, b] = rgb.to_be_bytes();
    let unit = |v: u8| f64::from(v) / 255.0;
    let color = Variant::from((unit(r), unit(g), unit(b)));
    let mut out = HashMap::new();
    out.insert("color".to_string(), color.try_to_owned().expect("struct"));
    (crate::RESPONSE_OK, out)
}

/// How a Screenshot call chooses what it takes, and so where its consent comes from.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// The screen or window, of the `SOURCE_*` kinds, the user picks in the share picker.
    Pick(u32),
    /// The area the user drags in the compositor's region select.
    Area,
    /// Every screen, after the Access prompt when `ask`.
    Desktop { ask: bool },
    /// The focused window, after the Access prompt when `ask`.
    ActiveWindow { ask: bool },
}

/// An interactive call picks in the share picker, and a Window target always does; an Area
/// target always drags. A Screen target or none takes every screen, and an Active Window
/// target the focused window, both asking as [`needs_consent`] says. Err for a `target` not in
/// `AVAILABLE_TARGETS`.
fn plan(options: &HashMap<String, Variant<'_>>) -> Result<Plan, String> {
    let interactive = matches!(options.get("interactive"), Some(Variant::Bool(true)));
    let ask = needs_consent(options);
    match options.get("target") {
        None if interactive => Ok(Plan::Pick(crate::SOURCE_MONITOR | crate::SOURCE_WINDOW)),
        Some(Variant::U32(TARGET_SCREEN)) if interactive => Ok(Plan::Pick(crate::SOURCE_MONITOR)),
        None | Some(Variant::U32(TARGET_SCREEN)) => Ok(Plan::Desktop { ask }),
        Some(Variant::U32(TARGET_WINDOW)) => Ok(Plan::Pick(crate::SOURCE_WINDOW)),
        Some(Variant::U32(TARGET_AREA)) => Ok(Plan::Area),
        Some(Variant::U32(TARGET_ACTIVE_WINDOW)) => Ok(Plan::ActiveWindow { ask }),
        Some(v) => Err(format!("target {v} is not served")),
    }
}

/// What [`take`] captures.
#[derive(Debug)]
enum Shot {
    /// Every output in one picture.
    Desktop,
    Of(Target),
    /// An area in logical layout coordinates.
    Area(Rect),
}

/// False only when the frontend already settled the `screenshot`
/// permission: a non-interactive call with `permission_store_checked`.
fn needs_consent(options: &HashMap<String, Variant<'_>>) -> bool {
    let flag = |key| matches!(options.get(key), Some(Variant::Bool(true)));
    flag("interactive") || !flag("permission_store_checked")
}

/// The screen or window, of the `types` kinds, the user picks for `app_id`; None on Cancel, a
/// frontend Close or nothing to list. Blocking.
fn pick(
    app_id: &str,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
    types: u32,
) -> Option<Target> {
    let wl = Wayland::connect()
        .map_err(|e| log::warn!("Screenshot: {e}"))
        .ok()?;
    let types = match wl.can_capture_toplevels() {
        true => types,
        false => types & !crate::SOURCE_WINDOW,
    };
    if types == 0 {
        log::warn!("Screenshot: the compositor captures no windows");
        return None;
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

/// Capture `shot` and write it out; blocking. An area leaves out the pointer that dragged it.
fn take(shot: Shot) -> Result<PathBuf, String> {
    let shot = match shot {
        Shot::Of(t) => grab(&mut Wayland::connect()?, &t, true)?,
        Shot::Desktop => desktop(&crate::stills::capture(true)?)?,
        Shot::Area(r) => area_of(&crate::stills::capture(false)?, r)?,
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
    let (scale, (left, top)) = frame(outputs);
    let right = outputs.iter().map(|(r, _)| r.right()).max().unwrap_or(0);
    let bottom = outputs.iter().map(|(r, _)| r.bottom()).max().unwrap_or(0);
    let at = |v: i32, origin: i32| at_scale(v - origin, scale);
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

/// The desktop picture's pixels per logical pixel, the finest any output has, and the layout's
/// top-left, where it starts.
fn frame(outputs: &[(Rect, (u32, u32))]) -> (f64, (i32, i32)) {
    let scale = outputs
        .iter()
        .filter(|(r, _)| r.width > 0)
        .map(|(r, (w, _))| f64::from(*w) / f64::from(r.width))
        .fold(1.0f64, f64::max);
    let left = outputs.iter().map(|(r, _)| r.x).min().unwrap_or(0);
    let top = outputs.iter().map(|(r, _)| r.y).min().unwrap_or(0);
    (scale, (left, top))
}

fn at_scale(v: i32, scale: f64) -> i32 {
    (f64::from(v) * scale).round() as i32
}

/// Each output's layout box and still size.
fn geometry(outputs: &[Output]) -> Vec<(Rect, (u32, u32))> {
    (outputs.iter())
        .map(|o| (o.layout, (o.still.width, o.still.height)))
        .collect()
}

/// The part of the desktop picture under `area` (layout coordinates), at its scale; Err for an
/// area on no output's place.
fn area_of(outputs: &[Output], area: Rect) -> Result<Still, String> {
    let whole = desktop(outputs)?;
    let (scale, (left, top)) = frame(&geometry(outputs));
    let (x, y) = (
        at_scale(area.x - left, scale),
        at_scale(area.y - top, scale),
    );
    let r = Rect::new(
        x,
        y,
        at_scale(area.right() - left, scale) - x,
        at_scale(area.bottom() - top, scale) - y,
    )
    .intersect(&Rect::new(0, 0, whole.width as i32, whole.height as i32));
    if r.is_empty() {
        return Err(format!("the area {area:?} is on no screen"));
    }
    let mut pixels = Vec::with_capacity(r.width as usize * r.height as usize);
    for row in r.y..r.bottom() {
        let start = row as usize * whole.width as usize + r.x as usize;
        pixels.extend_from_slice(&whole.pixels[start..start + r.width as usize]);
    }
    Ok(Still {
        logical: (area.width, area.height),
        width: r.width as u32,
        height: r.height as u32,
        pixels,
    })
}

/// Every output's still drawn at its place in one picture, black between
/// them; a still of another size than its place is resampled to it.
fn desktop(outputs: &[Output]) -> Result<Still, String> {
    let ((w, h), places) = arrange(&geometry(outputs));
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

    #[test]
    fn a_pick_answers_with_unit_channels_and_a_cancel_with_nothing() {
        let (code, r) = picked(Some(0x0080_00ff));
        assert_eq!(code, crate::RESPONSE_OK);
        let (red, green, blue): (f64, f64, f64) = r["color"].clone().try_into().unwrap();
        assert!((red - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!((green, blue), (0.0, 1.0));
        assert_eq!(r["color"].value_signature(), "(ddd)");
        assert_eq!(picked(None), (crate::RESPONSE_CANCELLED, HashMap::new()));
    }

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

    fn with_target(pairs: &[(&str, bool)], target: u32) -> HashMap<String, Variant<'static>> {
        let mut o = opts(pairs);
        o.insert("target".into(), Variant::U32(target));
        o
    }

    #[test]
    fn a_pick_or_a_drag_is_the_consent_and_the_rest_ask_unless_the_store_settled_it() {
        let both = crate::SOURCE_MONITOR | crate::SOURCE_WINDOW;
        let checked = [("permission_store_checked", true)];
        let interactive = [("interactive", true), ("permission_store_checked", true)];
        let rows = [
            (opts(&[]), Plan::Desktop { ask: true }),
            (opts(&checked), Plan::Desktop { ask: false }),
            (opts(&interactive), Plan::Pick(both)),
            (with_target(&[], TARGET_SCREEN), Plan::Desktop { ask: true }),
            (
                with_target(&checked, TARGET_SCREEN),
                Plan::Desktop { ask: false },
            ),
            (
                with_target(&interactive, TARGET_SCREEN),
                Plan::Pick(crate::SOURCE_MONITOR),
            ),
            (
                with_target(&checked, TARGET_WINDOW),
                Plan::Pick(crate::SOURCE_WINDOW),
            ),
            (
                with_target(&interactive, TARGET_WINDOW),
                Plan::Pick(crate::SOURCE_WINDOW),
            ),
            (with_target(&[], TARGET_AREA), Plan::Area),
            (with_target(&checked, TARGET_AREA), Plan::Area),
            (with_target(&interactive, TARGET_AREA), Plan::Area),
            (
                with_target(&[], TARGET_ACTIVE_WINDOW),
                Plan::ActiveWindow { ask: true },
            ),
            (
                with_target(&checked, TARGET_ACTIVE_WINDOW),
                Plan::ActiveWindow { ask: false },
            ),
            (
                with_target(&interactive, TARGET_ACTIVE_WINDOW),
                Plan::ActiveWindow { ask: true },
            ),
        ];
        for (options, want) in rows {
            assert_eq!(plan(&options), Ok(want), "{options:?}");
        }
        for unserved in [0, 16, TARGET_SCREEN | TARGET_WINDOW] {
            assert!(
                plan(&with_target(&[], unserved)).is_err(),
                "target {unserved}"
            );
        }
    }

    #[test]
    fn an_area_is_cut_from_the_desktop_picture_at_its_finest_scale() {
        // A 4x2 screen at scale 2 with a 2x2 one at scale 1 right of it, one lower.
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
        let (red, blue) = (0xffff_0000, 0xff00_00ff);
        let shot = area_of(&outputs, Rect::new(3, 1, 2, 1)).unwrap();
        assert_eq!((shot.width, shot.height), (4, 2));
        assert_eq!(shot.pixels, [red, red, blue, blue, red, red, blue, blue]);
        let shot = area_of(&outputs, Rect::new(-2, 0, 3, 1)).unwrap();
        assert_eq!((shot.width, shot.height), (2, 2), "clipped to the picture");
        assert!(area_of(&outputs, Rect::new(-9, 0, 3, 1)).is_err());
    }

    /// The Screenshot answers to each of `calls` from a portal served on `bus` with `link`.
    fn answers(
        bus: &crate::lockmode::tests::Bus,
        link: Option<crate::inputlink::Link>,
        calls: &[HashMap<String, Variant<'static>>],
    ) -> Vec<u32> {
        zbus::block_on(async {
            let caller = crate::lockmode::tests::connect(bus).await;
            caller.request_name(crate::FRONTEND).await.unwrap();
            let frontend = crate::Frontend::default();
            frontend.set(caller.unique_name().map(|n| n.to_string()));
            let shots = Screenshot {
                frontend: frontend.clone(),
                link,
            };
            let portal = zbus::connection::Builder::address(bus.1.as_str())
                .serve_at(crate::PATH, shots)
                .build()
                .await
                .unwrap();
            let _ = frontend.conn.set(portal.clone());
            let dest = portal.unique_name().unwrap().to_string();
            let mut codes = Vec::new();
            for (n, options) in calls.iter().enumerate() {
                let handle = format!("{}1_1/t{n}", crate::REQUEST_PREFIX);
                let handle = ObjectPath::try_from(handle).unwrap();
                let reply = caller
                    .call_method(
                        Some(dest.as_str()),
                        crate::PATH,
                        Some("org.freedesktop.impl.portal.Screenshot"),
                        "Screenshot",
                        &(handle, "org.example.App", "", options),
                    )
                    .await
                    .unwrap();
                let (code, _): (u32, HashMap<String, OwnedVariant>) =
                    reply.body().deserialize().unwrap();
                codes.push(code);
            }
            codes
        })
    }

    #[test]
    fn a_cancelled_drag_answers_cancelled_and_no_focused_window_takes_nothing() {
        use crate::inputlink::tests::{fake_answers, link_on};
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let compositor = fake_answers(theirs, None, "");
        let checked = [("permission_store_checked", true)];
        let calls = [
            with_target(&checked, TARGET_AREA),
            with_target(&checked, TARGET_ACTIVE_WINDOW),
        ];
        let codes = answers(&bus, Some(link_on(ours)), &calls);
        assert_eq!(codes, [crate::RESPONSE_CANCELLED, crate::RESPONSE_OTHER]);
        compositor.join().unwrap();
        assert_eq!(
            answers(&bus, None, &calls),
            [crate::RESPONSE_OTHER, crate::RESPONSE_OTHER],
            "without the compositor's link"
        );
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
