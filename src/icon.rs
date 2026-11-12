//! Window icons: the client's own (an xdg-toplevel-icon name or pixels, or an X11 window's
//! `_NET_WM_ICON` via `xwayland::helper`), else the app id's from the icon theme
//! or its `.desktop` entry (`taskbarview::app_icon`). Formats, resampling and the
//! fallback glyph are `reprisekit::icon`.

use std::sync::Arc;

use reprisekit::icon::{
    DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888, ICON_SIZE, IconSource, PREFERRED_SOURCE, convert,
};
use reprisekit::icontheme::IconCache;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::{
    wl_buffer::WlBuffer, wl_shm, wl_surface::WlSurface,
};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::xdg_toplevel_icon::{
    ToplevelIconCachedState, XdgToplevelIconHandler, XdgToplevelIconManager,
};

use crate::compositor::State;
use crate::raster::Canvas;
use crate::window::{Window, WindowId};

/// The xdg-toplevel-icon global, asking for the caption's size at scales 1 to 3.
pub fn manager(dh: &DisplayHandle) -> XdgToplevelIconManager {
    let mut m = XdgToplevelIconManager::new::<State>(dh);
    m.replace_icon_sizes([ICON_SIZE, 2 * ICON_SIZE, 3 * ICON_SIZE]);
    m
}

/// The icon `win` shows at `px`; None draws the fallback glyph. Shared with the
/// caches, so a redraw copies no pixels.
pub fn window_icon(
    icons: &mut IconCache,
    index: &[reprisefs::apps::AppEntry],
    win: &Window,
    px: i32,
) -> Option<Arc<Canvas>> {
    window_icon_waiting(icons, index, win, px).0
}

/// [`window_icon`], with the icon file or theme name it waits on when that is not ready at `px`.
pub fn window_icon_waiting(
    icons: &mut IconCache,
    index: &[reprisefs::apps::AppEntry],
    win: &Window,
    px: i32,
) -> (Option<Arc<Canvas>>, Option<String>) {
    if let Some(name) = win.icon_name.as_deref() {
        match named(icons, name, px) {
            Some(Some(icon)) => return (Some(icon), None),
            None => return (stand_in(icons, name, px), Some(name.to_string())),
            Some(None) => {}
        }
    }
    if let Some(icon) = win.own_icon(px) {
        return (Some(icon), None);
    }
    crate::taskbarview::app_icon_waiting(icons, index, &win.app_id(), px)
}

/// Theme icon `name` at `px` as [`IconCache::get_unread`] answers it, for a name a client set. A
/// name with a path separator would be taken for a file path, so it never resolves.
fn named(icons: &mut IconCache, name: &str, px: i32) -> Option<Option<Arc<Canvas>>> {
    if name.contains('/') {
        return Some(None);
    }
    icons.get_unread(name, px)
}

/// Read a wl_shm icon buffer's pixels at their native size. None for an
/// unsupported format or a buffer that does not hold what it claims.
fn from_shm(buffer: &WlBuffer) -> Option<IconSource> {
    with_buffer_contents(buffer, |ptr, len, d| {
        let format = match d.format {
            wl_shm::Format::Argb8888 => DRM_FORMAT_ARGB8888,
            wl_shm::Format::Xrgb8888 => DRM_FORMAT_XRGB8888,
            // Every other wl_shm format code is its DRM fourcc.
            f => f as u32,
        };
        // SAFETY: `with_buffer_contents` hands over the pool's mapping,
        // readable for `len` bytes while this closure runs.
        let pool = unsafe { std::slice::from_raw_parts(ptr, len) };
        let (w, h) = (d.width.max(0) as usize, d.height.max(0) as usize);
        let pixels = convert(
            pool.get(d.offset.max(0) as usize..)?,
            w,
            h,
            d.stride.max(0) as usize,
            format,
        )?;
        Some(IconSource {
            width: w as u32,
            height: h as u32,
            pixels,
        })
    })
    .ok()
    .flatten()
}

fn width_of(buffer: &WlBuffer) -> i32 {
    with_buffer_contents(buffer, |_, _, d| d.width).unwrap_or(0)
}

/// What an icon whose file is not decoded yet shows meanwhile.
pub const STAND_IN: &str = "application-x-executable";

/// Icon `name` at `px` for a surface on the loop ([`IconCache::get_unread`]): one not ready yet
/// shows [`stand_in`] until [`State::decode_wanted_icons`] lands it and the surface draws again.
pub fn unread(icons: &mut IconCache, name: &str, px: i32) -> Option<Arc<Canvas>> {
    icons
        .get_unread(name, px)
        .unwrap_or_else(|| stand_in(icons, name, px))
}

/// What `name` shows at `px` while its image is not ready: an icon file, which may sit on a mount
/// that never answers, [`STAND_IN`]; a theme icon, due within frames, nothing.
pub fn stand_in(icons: &mut IconCache, name: &str, px: i32) -> Option<Arc<Canvas>> {
    let blank = || Some(Arc::new(Canvas::new(px.max(1) as u32, px.max(1) as u32)));
    if !name.starts_with('/') {
        return blank();
    }
    icons.get_unread(STAND_IN, px).unwrap_or_else(blank)
}

/// Whether app `app_id`'s icon, as [`crate::taskbarview::app_icon`] finds it, is `key` (an icon
/// file or a theme name).
fn app_wears(index: &[reprisefs::apps::AppEntry], app_id: &str, key: &str) -> bool {
    app_id == key || reprisefs::apps::icon_for_app_id(index, app_id) == Some(key)
}

/// Whether a Quick Launch item or taskbar pin wears icon file `file`.
fn quick_wears(items: &[crate::quicklaunch::QuickItem], file: &str) -> bool {
    items.iter().any(|q| q.entry.icon.as_deref() == Some(file))
}

/// Icon files and theme names a job is decoding, each with the px asked for since its job
/// started.
pub type Decoding = std::collections::HashMap<String, Vec<i32>>;

/// A surface that drew a stand-in in an icon's place, to draw again once icons land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconWaiter {
    Bar(reprisescene::TreeId),
    Desktop,
    Switcher,
    Dialog(u64),
    Notify,
    TrayFlyout,
}

/// Icon decodes off the loop: what is decoding, what landed since the turn's end, and who waits.
#[derive(Default)]
pub struct IconJobs {
    pub decoding: Decoding,
    /// (icon file or theme name, px, image) landed, for the turn's end to hand out at once.
    landed: Vec<(String, i32, Option<Canvas>)>,
    waiters: Vec<IconWaiter>,
}

/// How a theme decode job loads one icon: [`reprisekit::icontheme::load_icon`]'s signature.
type LoadIcon = fn(&reprisekit::icontheme::IconTheme, &str, i32, i32) -> Option<Canvas>;

/// Theme icons `names` at each of their sizes from `theme`, each loaded by `load` ([`guarded`]).
fn load_theme_icons(
    theme: &reprisekit::icontheme::IconTheme,
    names: Vec<(String, Vec<i32>)>,
    scale: i32,
    load: LoadIcon,
) -> Vec<(String, i32, Option<Canvas>)> {
    (names.into_iter())
        .flat_map(|(name, sizes)| {
            sizes.into_iter().map(move |px| {
                let icon = guarded(&name, px, || load(theme, &name, px, scale));
                (name.clone(), px, icon)
            })
        })
        .collect()
}

/// `decode` run for icon `what` at `px`; a decode that panics comes back as one that failed, so
/// the icon lands as broken and its lookup moves on instead of waiting on it for good.
fn guarded(what: &str, px: i32, decode: impl FnOnce() -> Option<Canvas>) -> Option<Canvas> {
    crate::crash::guarded(decode).unwrap_or_else(|_| {
        log::error!("icons: decoding {what} at {px} px panicked");
        None
    })
}

/// Theme names one decode job takes: a cold menu's icons land in a few batches, each one redraw.
const THEME_JOB: usize = 16;

/// The jobs to start for `wanted` (path, px): one per file not decoding yet, with all its sizes.
/// A file already decoding only notes the sizes, for a job of its own once the first lands: a
/// file on a dead mount holds one job whatever surfaces and sizes ask for it.
fn plan_decodes(decoding: &mut Decoding, wanted: Vec<(String, i32)>) -> Vec<(String, Vec<i32>)> {
    let mut start: Vec<(String, Vec<i32>)> = Vec::new();
    for (file, px) in wanted {
        if let Some(asked) = decoding.get_mut(&file) {
            if !asked.contains(&px) {
                asked.push(px);
            }
            continue;
        }
        match start.iter_mut().find(|(f, _)| *f == file) {
            Some((_, sizes)) if !sizes.contains(&px) => sizes.push(px),
            Some(_) => {}
            None => start.push((file, vec![px])),
        }
    }
    for (file, sizes) in &start {
        decoding.insert(file.clone(), sizes.clone());
    }
    start
}

impl State {
    /// Hands the icons landed since the last turn to the surfaces showing them, then decodes what
    /// the loop asked [`IconCache::get_unread`] for off the loop (`plan_decodes`). Runs at each
    /// turn's end.
    pub fn decode_wanted_icons(&mut self) {
        // Without the jobs' channel a job lands inline, so its icons go out in the next round.
        for _ in 0..3 {
            self.land_icons();
            let wanted = self.icons.take_wanted();
            if wanted.is_empty() {
                return;
            }
            let (files, names): (Vec<_>, Vec<_>) =
                wanted.into_iter().partition(|(n, _)| n.starts_with('/'));
            for (file, sizes) in plan_decodes(&mut self.icon_jobs.decoding, files) {
                self.spawn_icon_decode(file, sizes);
            }
            let names = plan_decodes(&mut self.icon_jobs.decoding, names);
            for batch in names.chunks(THEME_JOB) {
                self.spawn_theme_decode(batch.to_vec());
            }
        }
    }

    /// Notes `who` to draw again once icons land if [`IconCache::get_unread`] answered None since
    /// it read `waits` ([`IconCache::waits`]).
    pub fn note_icon_waits(&mut self, waits: u64, who: IconWaiter) {
        if self.icons.waits() != waits {
            self.wait_for_icons(who);
        }
    }

    /// Notes `who` to draw again once icons land.
    pub fn wait_for_icons(&mut self, who: IconWaiter) {
        if !self.icon_jobs.waiters.contains(&who) {
            self.icon_jobs.waiters.push(who);
        }
    }

    fn spawn_icon_decode(&mut self, file: String, sizes: Vec<i32>) {
        let scale = self.icons.scale();
        let key = file.clone();
        let spawned = self.spawn_fs_job(move || {
            let path = std::path::Path::new(&file);
            let art: Vec<(i32, Option<Canvas>)> = (sizes.into_iter())
                .map(|px| {
                    let icon = guarded(&file, px, || {
                        reprisekit::icontheme::decode_file(path, px, scale)
                    });
                    (px, icon)
                })
                .collect();
            Box::new(move |state: &mut State| state.icon_decoded(file, art, scale))
        });
        // The file may sit on a mount that never answers, so it is not read here: the next draw
        // that wants it asks again.
        if !spawned {
            self.icon_jobs.decoding.remove(&key);
        }
    }

    fn icon_decoded(&mut self, file: String, art: Vec<(i32, Option<Canvas>)>, scale: i32) {
        let asked = self.icon_jobs.decoding.remove(&file).unwrap_or_default();
        let late: Vec<i32> = (asked.into_iter())
            .filter(|px| !art.iter().any(|(p, _)| p == px))
            .collect();
        let puts = art.iter().map(|(px, c)| (file.clone(), *px, c.clone()));
        self.icons.put_files_at(puts.collect(), scale);
        let landed = art.into_iter().map(|(px, c)| (file.clone(), px, c));
        self.icon_jobs.landed.extend(landed);
        if !late.is_empty() {
            self.icon_jobs.decoding.insert(file.clone(), late.clone());
            self.spawn_icon_decode(file, late);
        }
    }

    /// Loads theme icons `names` at their sizes on a thread of its own: the theme's files are the
    /// ones its lookups list, so no mount a file job waits on holds them up. With no thread they
    /// load here.
    fn spawn_theme_decode(&mut self, names: Vec<(String, Vec<i32>)>) {
        self.spawn_theme_decode_with(names, reprisekit::icontheme::load_icon);
    }

    fn spawn_theme_decode_with(&mut self, names: Vec<(String, Vec<i32>)>, load: LoadIcon) {
        let (theme, scale) = (self.icons.theme(), self.icons.scale());
        let job = {
            let (theme, names) = (theme.clone(), names.clone());
            move || -> crate::fsjobs::OnLoop {
                let art = load_theme_icons(&theme, names, scale, load);
                Box::new(move |state: &mut State| state.theme_icons_decoded(&theme, art))
            }
        };
        if !self.spawn_job(job) {
            let art = load_theme_icons(&theme, names, scale, load);
            self.theme_icons_decoded(&theme, art);
        }
    }

    fn theme_icons_decoded(
        &mut self,
        theme: &Arc<reprisekit::icontheme::IconTheme>,
        art: Vec<(String, i32, Option<Canvas>)>,
    ) {
        // From a theme replaced since: every size asked goes again, in the new theme.
        let stale = !Arc::ptr_eq(theme, &self.icons.theme());
        let mut again: Vec<(String, Vec<i32>)> = Vec::new();
        for (name, _, _) in &art {
            let Some(asked) = self.icon_jobs.decoding.remove(name) else {
                continue;
            };
            let late: Vec<i32> = (asked.into_iter())
                .filter(|px| stale || !art.iter().any(|(n, p, _)| n == name && p == px))
                .collect();
            if !late.is_empty() {
                again.push((name.clone(), late));
            }
        }
        self.icons.put_icons(theme, art.clone());
        if !stale {
            self.icon_jobs.landed.extend(art);
        }
        for (name, sizes) in &again {
            self.icon_jobs.decoding.insert(name.clone(), sizes.clone());
        }
        for batch in again.chunks(THEME_JOB) {
            self.spawn_theme_decode(batch.to_vec());
        }
    }

    /// The icons landed since the last turn: every surface showing one draws again, once.
    fn land_icons(&mut self) {
        let landed = std::mem::take(&mut self.icon_jobs.landed);
        let refused = self.icons.take_refused();
        if landed.is_empty() && refused.is_empty() {
            return;
        }
        self.desktop_names_refused(&refused);
        let keys: Vec<&str> = landed.iter().map(|(k, ..)| k.as_str()).collect();
        self.menu_icons_landed(&landed);
        self.window_icon_landed(&keys);
        if keys.iter().any(|k| quick_wears(&self.quicklaunch, k)) {
            self.redraw_taskbars();
        }
        self.start_panel_icons_landed(&keys, !refused.is_empty());
        self.launcher_icons_landed();
        crate::tray::customize_icon_landed(self, &keys);
        for who in std::mem::take(&mut self.icon_jobs.waiters) {
            match who {
                IconWaiter::Bar(tree) => self.redraw_taskbar(tree),
                IconWaiter::Desktop => self.relayout_desktop(),
                IconWaiter::Switcher => self.redraw_switcher(),
                IconWaiter::Dialog(id) => self.dialog_icons_landed(id),
                IconWaiter::Notify => crate::notifyview::icons_landed(self),
                IconWaiter::TrayFlyout => crate::trayflyout::icons_landed(self),
            }
        }
    }

    /// Icons `keys` landed: the windows wearing one redraw their caption and task button, and an
    /// open switcher, overview, snap picker or task preview draws again.
    fn window_icon_landed(&mut self, keys: &[&str]) {
        let wears = |w: &Window| {
            let named = w.icon_name.as_deref();
            (keys.iter()).any(|k| named == Some(k) || app_wears(&self.app_index, &w.app_id(), k))
        };
        let wearing: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| wears(w))
            .map(|w| w.id)
            .collect();
        if wearing.is_empty() {
            return;
        }
        for w in wearing {
            self.retitled(w, crate::retitle::Changes::ICON);
        }
        self.redraw_switcher();
        self.redraw_overview();
        crate::snappickerview::icons_landed(self);
        crate::taskpreview::icons_landed(self);
    }

    /// A commit of `w`'s surface applies a newly set icon: its name, and the buffer
    /// whose width is closest to 32 px at native size. A name the theme has wins over
    /// the buffer; with neither the app id's icon draws.
    pub fn apply_icon(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !std::mem::take(&mut win.icon_pending) {
            return;
        }
        let (name, source) = with_states(win.wl_surface(), |s| {
            let mut cached = s.cached_state.get::<ToplevelIconCachedState>();
            let icon = cached.current();
            let source = (icon.buffers().iter())
                .min_by_key(|(b, _)| (width_of(b) - PREFERRED_SOURCE).abs())
                .and_then(|(b, _)| from_shm(b));
            (icon.icon_name().map(str::to_owned), source)
        });
        let renamed = win.icon_name != name;
        win.icon_name = name;
        if (win.set_icon(source) || renamed) && win.mapped {
            self.retitled(w, crate::retitle::Changes::ICON);
        }
    }
}

/// xdg-toplevel-icon is double-buffered on the surface, so the commit applies
/// it (`apply_icon`).
impl XdgToplevelIconHandler for State {
    fn set_icon(&mut self, _toplevel: XdgToplevel, wl_surface: WlSurface) {
        if let Some(w) = self.window_for_surface(&wl_surface)
            && let Some(win) = self.win_mut(w)
        {
            win.icon_pending = true;
        }
    }
}

/// An icon root whose theme "test" draws each of `names` from a red SVG and holds each of
/// `broken` as an SVG that does not parse.
#[cfg(test)]
pub(crate) fn svg_theme(names: &[&str], broken: &[&str]) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = crate::harness::scratch().join(format!("svg-theme-{}-{n}", std::process::id()));
    let apps = root.join("test/scalable/apps");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        root.join("test/index.theme"),
        "[Icon Theme]\nName=Test\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n",
    )
    .unwrap();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#f00"/></svg>"##;
    for name in names {
        std::fs::write(apps.join(format!("{name}.svg")), svg).unwrap();
    }
    for name in broken {
        std::fs::write(apps.join(format!("{name}.svg")), "<svg").unwrap();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_icon_file_is_never_read_on_the_loop() {
        use reprisekit::icontheme::IconTheme;
        let file = "/srv/icons/viewer.png";
        let index = [reprisefs::apps::AppEntry {
            name: "Viewer".into(),
            exec_line: "viewer".into(),
            terminal: false,
            id: "org.example.Viewer".into(),
            icon: Some(file.into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
        }];
        let mut icons = IconCache::new(IconTheme::load("none", &[]));
        let app = "org.example.Viewer";
        assert!(crate::taskbarview::app_icon(&mut icons, &index, app, 16).is_none());
        assert_eq!(icons.take_wanted(), [(file.to_string(), 16)]);
        assert!(app_wears(&index, app, file));
        icons.put_file(file, 16, Some(Canvas::new(16, 16)));
        let icon = crate::taskbarview::app_icon(&mut icons, &index, app, 16);
        assert_eq!(icon.map(|c| c.width), Some(16));
    }

    #[test]
    fn a_client_icon_name_resolves_in_the_theme_but_never_as_a_path() {
        use reprisekit::icontheme::IconTheme;
        let mut icons = IconCache::new(IconTheme::load("none", &[]));
        let theme = icons.theme();
        let art = |n: &str| (n.to_string(), 16, Some(Canvas::new(16, 16)));
        icons.put_icons(&theme, vec![art("document-edit")]);
        icons.put_file("/srv/icons/viewer.png", 16, Some(Canvas::new(16, 16)));
        assert!(named(&mut icons, "document-edit", 16).flatten().is_some());
        assert_eq!(named(&mut icons, "/srv/icons/viewer.png", 16), Some(None));
        assert_eq!(named(&mut icons, "unknown-name", 16), Some(None));
    }

    #[test]
    fn an_app_icon_names_the_file_it_waits_on_until_it_lands() {
        use reprisekit::icontheme::IconTheme;
        let file = "/srv/icons/viewer.png";
        let index = [reprisefs::apps::AppEntry {
            name: "Viewer".into(),
            exec_line: "viewer".into(),
            terminal: false,
            id: "org.example.Viewer".into(),
            icon: Some(file.into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
        }];
        let mut icons = IconCache::new(IconTheme::load("none", &[]));
        let app = "org.example.Viewer";
        let waiting = |icons: &mut IconCache, app| {
            crate::taskbarview::app_icon_waiting(icons, &index, app, 16).1
        };
        assert_eq!(waiting(&mut icons, app).as_deref(), Some(file));
        assert_eq!(waiting(&mut icons, "org.example.Unknown"), None);
        icons.put_file(file, 16, Some(Canvas::new(16, 16)));
        assert_eq!(
            waiting(&mut icons, app),
            None,
            "a landed file is not waited on"
        );
    }

    #[test]
    fn a_quick_launch_icon_file_is_never_read_on_the_loop() {
        use reprisekit::icontheme::IconTheme;
        let file = "/srv/icons/editor.svg";
        let text =
            format!("[Desktop Entry]\nType=Application\nName=Editor\nExec=ed\nIcon={file}\n");
        let item = crate::quicklaunch::QuickItem {
            file: "editor.desktop".into(),
            entry: crate::quicklaunch::parse(&text, "editor.desktop", "foot").unwrap(),
            app: reprisefs::apps::parse_desktop(&text, "editor.desktop"),
        };
        let mut icons = IconCache::new(IconTheme::load("none", &[]));
        assert!(crate::taskbarview::quick_icon(&mut icons, &item, 16).is_none());
        assert_eq!(icons.take_wanted(), [(file.to_string(), 16)]);
        assert!(quick_wears(std::slice::from_ref(&item), file));
    }

    /// Two outputs, Start menu `style`, theme icons from `root`, and file jobs on threads.
    fn two_outputs(style: &str, root: &std::path::Path) -> crate::harness::Harness {
        use reprisekit::icontheme::IconTheme;
        let toml = format!("startmenu_style = \"{style}\"\n");
        let config = crate::config::Config::parse(&toml).expect("the config");
        let mut h = crate::harness::Harness::with_config(config);
        crate::fsjobs::setup(&mut h.state);
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        h.state.icons = IconCache::new(IconTheme::load("test", &[root.to_path_buf()]));
        h
    }

    /// Turns the loop until no icon decodes.
    fn land(h: &mut crate::harness::Harness) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        h.dispatch();
        while !h.state.icon_jobs.decoding.is_empty() {
            assert!(std::time::Instant::now() < end, "the icons never landed");
            h.run_for(5);
        }
    }

    fn red(c: &Canvas) -> bool {
        c.get(c.width as i32 / 2, c.height as i32 / 2) == 0xFFFF_0000
    }

    fn run_icon(h: &crate::harness::Harness) -> Option<Canvas> {
        let m = &h.state.menus[0];
        let row = (m.items.iter())
            .position(|i| matches!(i, MenuItem::Entry { label, .. } if label.contains("Run")))
            .expect("the Run row");
        match &m.items[row] {
            MenuItem::Entry { icon, .. } => icon.clone(),
            _ => None,
        }
    }

    use crate::startmenu::MenuItem;

    #[test]
    fn a_cold_svg_menu_icon_decodes_off_the_loop_and_lands_on_its_output_only() {
        let root = super::svg_theme(&["system-run"], &[]);
        let mut h = two_outputs("classic", &root);
        let bar = h.state.bars[0].tree;
        h.state.toggle_start(bar);
        let blank = run_icon(&h).expect("a stand-in in the icon's place");
        assert!(
            blank.pixels.iter().all(|&p| p == 0),
            "the stand-in is blank"
        );
        h.dispatch();
        assert!(
            h.state.icon_jobs.decoding.contains_key("system-run"),
            "a job decodes it"
        );
        crate::backend::take_asks();
        land(&mut h);
        assert!(
            run_icon(&h).is_some_and(|c| red(&c)),
            "the row took the icon"
        );
        let asked = crate::backend::take_asks();
        assert!(!asked.is_empty(), "the landing asks for a frame");
        assert!(
            asked.iter().all(|(o, _)| o == "TEST-1"),
            "only the menu's output: {asked:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cached_menu_icon_draws_when_the_menu_opens() {
        let root = super::svg_theme(&["system-run"], &[]);
        let mut h = two_outputs("classic", &root);
        let bar = h.state.bars[0].tree;
        h.state.toggle_start(bar);
        land(&mut h);
        h.state.toggle_start(bar);
        h.dispatch();
        let waits = h.state.icons.waits();
        h.state.toggle_start(bar);
        assert!(run_icon(&h).is_some_and(|c| red(&c)), "drawn on open");
        assert_eq!(h.state.icons.waits(), waits, "nothing waited on");
        assert!(h.state.icons.take_wanted().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cold_start_panel_place_takes_its_icon_when_it_lands() {
        let root = super::svg_theme(&["user-home"], &[]);
        let mut h = two_outputs("glass", &root);
        let bar = h.state.bars[0].tree;
        h.state.toggle_start(bar);
        let home = |h: &crate::harness::Harness| {
            let v = h.state.start_panel.as_ref().expect("the panel");
            v.model.places[0].picture.clone()
        };
        assert!(home(&h).is_some_and(|c| c.pixels.iter().all(|&p| p == 0)));
        crate::backend::take_asks();
        land(&mut h);
        assert!(home(&h).is_some_and(|c| red(&c)), "the place took the icon");
        let asked = crate::backend::take_asks();
        assert!(
            !asked.is_empty() && asked.iter().all(|(o, _)| o == "TEST-1"),
            "only the panel's output: {asked:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_tray_flyout_draws_again_when_its_stand_in_lands() {
        let root = super::svg_theme(&[STAND_IN], &[]);
        let mut h = two_outputs("classic", &root);
        let _tray = crate::tray::headless(&mut h.state);
        let item = crate::tray::TrayItem {
            id: "probe".into(),
            key: "probe".into(),
            ..Default::default()
        };
        h.state.tray.as_mut().unwrap().hidden.push(item.clone());
        let bar = h.state.bars[0].tree;
        crate::trayflyout::chevron_press(&mut h.state, bar, crate::input::BTN_LEFT, None);
        let draws = |h: &crate::harness::Harness| {
            let tray = h.state.tray.as_ref().unwrap();
            tray.flyout.as_ref().map(|v| v.draws)
        };
        let opened = draws(&h).expect("the flyout opened");
        crate::backend::take_asks();
        land(&mut h);
        assert!(draws(&h) > Some(opened), "the flyout drew again");
        let asked = crate::backend::take_asks();
        assert!(
            !asked.is_empty() && asked.iter().all(|(o, _)| o == "TEST-1"),
            "only the flyout's output: {asked:?}"
        );
        let px = reprisekit::icon::ICON_SIZE;
        let icon = crate::tray::item_icon(&mut h.state, &item, px);
        assert!(icon.is_some_and(|c| red(&c)), "the stand-in landed");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_launcher_file_row_takes_its_icon_once_it_lands() {
        let root = super::svg_theme(&["text-x-generic"], &[]);
        let mut h = two_outputs("classic", &root);
        h.state.toggle_launcher();
        h.state
            .show_launcher_files(vec![std::path::PathBuf::from("/srv/notes.zzq")]);
        let row = |h: &crate::harness::Harness| {
            let m = &h.state.launcher.as_ref().expect("the launcher").model;
            let at = (m.list.items.iter()).position(|n| n == "notes.zzq");
            at.and_then(|i| m.icons[i].clone())
                .expect("the file row's icon")
        };
        assert!(
            row(&h).pixels.iter().all(|&p| p == 0),
            "the stand-in is blank"
        );
        crate::backend::take_asks();
        land(&mut h);
        assert!(red(&row(&h)), "the file row took the icon once it landed");
        let asked = crate::backend::take_asks();
        assert!(
            !asked.is_empty() && asked.iter().all(|(o, _)| o == "TEST-1"),
            "only the launcher's output: {asked:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// [`reprisekit::icontheme::load_icon`], but "system-run" panics.
    fn load_panicking(
        theme: &reprisekit::icontheme::IconTheme,
        name: &str,
        px: i32,
        scale: i32,
    ) -> Option<Canvas> {
        assert_ne!(name, "system-run", "a forced panic");
        reprisekit::icontheme::load_icon(theme, name, px, scale)
    }

    #[test]
    fn a_theme_decode_that_panics_lands_its_name_as_broken_and_the_rest_of_its_batch() {
        let root = super::svg_theme(&["system-run", "user-home"], &[]);
        let mut h = two_outputs("classic", &root);
        for name in ["system-run", "user-home"] {
            assert_eq!(h.state.icons.get_unread(name, 16), None, "{name} is cold");
        }
        let wanted = h.state.icons.take_wanted();
        let names = plan_decodes(&mut h.state.icon_jobs.decoding, wanted);
        h.state.spawn_theme_decode_with(names, load_panicking);
        land(&mut h);
        assert_eq!(
            h.state.icons.get_unread("system-run", 16),
            Some(None),
            "the name that panicked decodes at no size"
        );
        let home = h.state.icons.get_unread("user-home", 16).flatten();
        assert!(home.is_some_and(|c| red(&c)), "its batch landed");
        assert!(
            h.state.icons.take_wanted().is_empty(),
            "nothing waits on it"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_theme_decode_with_no_thread_loads_on_the_loop_and_lands() {
        let root = super::svg_theme(&["system-run"], &[]);
        let mut h = two_outputs("classic", &root);
        h.state.fsjobs.refuse_threads = true;
        let bar = h.state.bars[0].tree;
        h.state.toggle_start(bar);
        assert!(run_icon(&h).is_some_and(|c| c.pixels.iter().all(|&p| p == 0)));
        h.dispatch();
        assert!(h.state.icon_jobs.decoding.is_empty(), "no job holds it");
        assert!(
            run_icon(&h).is_some_and(|c| red(&c)),
            "the row took the icon"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_icon_file_with_no_thread_is_asked_for_again() {
        let root = super::svg_theme(&[], &[]);
        let mut h = two_outputs("classic", &root);
        h.state.fsjobs.refuse_threads = true;
        let file = "/srv/icons/viewer.png";
        assert_eq!(h.state.icons.get_unread(file, 16), None);
        h.state.decode_wanted_icons();
        assert!(h.state.icon_jobs.decoding.is_empty(), "no job holds it");
        assert_eq!(h.state.icons.get_unread(file, 16), None);
        assert_eq!(h.state.icons.take_wanted(), [(file.to_string(), 16)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_decoding_holds_one_job_whatever_sizes_ask_for_it() {
        let mut decoding = Decoding::new();
        let w = |f: &str, px| (f.to_string(), px);
        let start = plan_decodes(
            &mut decoding,
            vec![
                w("/srv/a.png", 16),
                w("/srv/a.png", 32),
                w("/srv/b.png", 16),
                w("/srv/a.png", 16),
            ],
        );
        assert_eq!(
            start,
            [
                ("/srv/a.png".to_string(), vec![16, 32]),
                ("/srv/b.png".to_string(), vec![16])
            ]
        );
        let again = plan_decodes(
            &mut decoding,
            vec![w("/srv/a.png", 48), w("/srv/a.png", 16)],
        );
        assert!(again.is_empty(), "a decoding file starts no second job");
        assert_eq!(
            decoding["/srv/a.png"],
            [16, 32, 48],
            "the late size waits for the first job"
        );
    }
}
