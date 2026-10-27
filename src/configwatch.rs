//! Re-reads `config.toml` when it changes: inotify watches on the file's directory (and a symlink's target's),
//! since editors and `reprise-settings` write a temp file and rename; a pure diff says what to redo.

use std::os::fd::BorrowedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};

use crate::compositor::State;
use crate::config::{Config, StartMenuStyle};
use crate::dirwatch::{DirWatch, Want};
use crate::window::WindowId;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub fonts: bool,
    pub icons: bool,
    pub desktop: bool,
    pub keyboard: bool,
    pub cursor: bool,
    pub pointers: bool,
    /// `adaptive_sync` (top-level or per output), or an `[[outputs]]`
    /// entry: commit each output's changed keys ([`output_change`]).
    pub outputs: bool,
    pub wallpaper: WallpaperChange,
    /// `[taskbar] position`, or `side_width` on a side bar: menus and panels
    /// opened from the bars close, a balloon moves to the new corner, and a
    /// window whose caption a top bar covers moves down. Layer surfaces and
    /// maximized windows follow in `update_layout`.
    pub bar_moved: bool,
    /// `[taskbar] auto_hide`: auto-hide starts or stops
    /// (`autohideview::config_changed`) and the reserved strip changes;
    /// follow-ups are as for `bar_moved`, so a Start menu docked on a hidden
    /// bar's reveal line does not cover the whole bar.
    pub bar_hide: bool,
    pub workspaces: bool,
    pub retired: bool,
}

impl Changes {
    pub fn closes_bar_popups(&self) -> bool {
        self.bar_moved || self.bar_hide
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum WallpaperChange {
    #[default]
    None,
    Image,
    Position,
}

/// Compare two configs. Everything the compositor reads from
/// `State::config` at use needs nothing but the swap, so it is absent
/// here; `quicklaunch` is folder-backed and never re-read.
pub fn diff(old: &Config, new: &Config) -> Changes {
    Changes {
        // A new interface brings its own caption and Start fonts.
        fonts: old.theme.fonts != new.theme.fonts || old.interface != new.interface,
        icons: old.icons.theme != new.icons.theme,
        desktop: old.system_icons != new.system_icons
            || old.desktop_icons != new.desktop_icons
            || old.desktop_extensions != new.desktop_extensions,
        keyboard: old.keyboard != new.keyboard,
        cursor: old.cursor != new.cursor,
        pointers: old.mouse != new.mouse
            || old.touchpad != new.touchpad
            || old.touchscreen != new.touchscreen,
        outputs: old.adaptive_sync != new.adaptive_sync || old.outputs != new.outputs,
        wallpaper: if old.wallpaper.image != new.wallpaper.image
            || old.wallpaper.slideshow != new.wallpaper.slideshow
            || old.wallpaper.interval != new.wallpaper.interval
        {
            WallpaperChange::Image
        } else if old.wallpaper != new.wallpaper {
            WallpaperChange::Position
        } else {
            WallpaperChange::None
        },
        bar_moved: old.taskbar.position != new.taskbar.position
            || (new.taskbar.position.is_side() && old.taskbar.side_width != new.taskbar.side_width),
        bar_hide: old.taskbar.auto_hide != new.taskbar.auto_hide,
        workspaces: old.workspaces != new.workspaces,
        retired: old.retired != new.retired,
    }
}

/// What a reload redraws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Redraw {
    /// An identical config: nothing.
    Nothing,
    /// Output keys alone (`[[outputs]]`, `adaptive_sync`): the layout, when it moved.
    Layout,
    /// The look and the layout.
    All,
}

fn redraw(old: &Config, new: &Config) -> Redraw {
    let mut kept = old.clone();
    kept.outputs = new.outputs.clone();
    kept.adaptive_sync = new.adaptive_sync;
    if old == new {
        Redraw::Nothing
    } else if kept == *new {
        Redraw::Layout
    } else {
        Redraw::All
    }
}

/// Whether an outputs-only reload moves anything: an output's geometry, or the primary output
/// that holds the desktop icons, balloons and media panel.
fn lays_out(old: &Config, new: &Config, geometry_changed: bool) -> bool {
    geometry_changed || old.primary_output_name() != new.primary_output_name()
}

/// Which of one output's settings a reload changed, so it commits those
/// and nothing else: a mode set through output management and not yet
/// written survives a reload that changes another key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputChange {
    pub mode: bool,
    pub transform: bool,
    pub scale: bool,
    pub adaptive_sync: bool,
    pub position: bool,
}

impl OutputChange {
    pub fn any(self) -> bool {
        self.mode || self.transform || self.scale || self.adaptive_sync || self.position
    }
}

pub fn output_change(old: &Config, new: &Config, name: &str) -> OutputChange {
    OutputChange {
        mode: old.mode_for_output_name(name) != new.mode_for_output_name(name),
        transform: old.transform_for_output_name(name) != new.transform_for_output_name(name),
        scale: old.scale_for_output_name(name) != new.scale_for_output_name(name),
        adaptive_sync: old.adaptive_sync_for_output_name(name)
            != new.adaptive_sync_for_output_name(name),
        position: old.position_for_output_name(name) != new.position_for_output_name(name),
    }
}

/// What the config watch covers: the directory `path` names, and, when
/// `path` is a symlink, the directory of the file it resolves to, since
/// `repriseconf::write_atomic` and editors writing through the link replace
/// that file in its own directory. MOVED_TO catches the rename they do,
/// CLOSE_WRITE a write in place. Not CREATE: that fires on an empty new
/// file, before whatever wrote it has put anything in it.
fn wants(path: &Path) -> Vec<Want> {
    let Some(name) = path.file_name() else {
        return Vec::new();
    };
    let mask = WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO;
    // A relative `--config path.toml` has "" for a parent; that is the
    // working directory, which is what inotify wants spelled out.
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut want = vec![Want {
        dir: dir.clone(),
        mask,
        only: vec![name.to_os_string()],
    }];
    if let Ok(real) = std::fs::canonicalize(path)
        && let (Some(real_dir), Some(real_name)) = (real.parent(), real.file_name())
    {
        // One inotify watch per directory: a link to a file beside it adds
        // a name, not a second watch.
        if std::fs::canonicalize(&dir).is_ok_and(|d| d == real_dir) {
            if real_name != name {
                want[0].only.push(real_name.to_os_string());
            }
        } else {
            want.push(Want {
                dir: real_dir.to_path_buf(),
                mask,
                only: vec![real_name.to_os_string()],
            });
        }
    }
    want
}

/// `startup` is the file's [`stamp`] before the startup read.
pub fn setup(state: &mut State, startup: Stamp) {
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("config watch: inotify unavailable: {e}");
            return;
        }
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("config watch: no watch fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, _, state| {
        state.config_watch.drain();
        sync(state);
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("config watch: {}", e.error);
    }
    log::info!("config watch: watching {}", state.config_path.display());
    state.config_watch.watch = Some(watch);
    let path = state.config_path.clone();
    sync_with(
        state,
        move || {
            let want = wants(&path);
            // The directory need not exist yet: a first run with no
            // `~/.config/reprise` still has to notice the file appearing, and
            // inotify watches a directory, not a path that might turn up.
            if let Some(first) = want.first()
                && let Err(e) = std::fs::create_dir_all(&first.dir)
            {
                log::warn!("config watch: cannot create {}: {e}", first.dir.display());
            }
            want
        },
        Some(startup),
    );
}

/// The config file's inode, size and mtime, or None when it is missing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp(Option<(u64, u64, u64, Option<SystemTime>)>);

/// Taken before the startup read, so a write the first watch was not up for differs from it.
pub fn stamp(path: &Path) -> Stamp {
    Stamp((std::fs::metadata(path).ok()).map(|m| (m.dev(), m.ino(), m.len(), m.modified().ok())))
}

fn sync(state: &mut State) {
    let path = state.config_path.clone();
    sync_with(state, move || wants(&path), None);
}

/// Syncs the watch to what `find` returns and, after an event named the file or when the file
/// no longer matches `since`, reads the config after it, all on a file job: a config directory
/// on a mount that stopped answering holds the job, never the loop.
fn sync_with<F>(state: &mut State, find: F, since: Option<Stamp>)
where
    F: FnOnce() -> Vec<Want> + Send + 'static,
{
    let Some(job) = state.config_watch.start() else {
        return;
    };
    let path = state.config_path.clone();
    let base = state.config_file.clone();
    state.spawn_fs_job(move || {
        let want = find();
        // Before the read, so a write between the two is not missed.
        let added = job.adder.run(&want);
        let cfg = if job.hit || since.is_some_and(|s| stamp(&path) != s) {
            self::read(&path, &base)
        } else {
            None
        };
        Box::new(move |state: &mut State| {
            if state.config_watch.landed(&want, added) {
                // A later read follows; this one may be older than the file.
                let newer = state.config_watch.hit_pending();
                sync(state);
                if newer {
                    return;
                }
            }
            if let Some((cfg, notice, table)) = cfg {
                if notice != state.config_notice {
                    if let Some(body) = &notice {
                        crate::notifyview::notice(state, "Configuration", body);
                    }
                    state.config_notice = notice;
                }
                state.config_file = table;
                reload(state, cfg);
            }
        })
    });
}

/// The config at `path` with the keys that fail kept at their values in `base` (see
/// [`Config::parse_over`]), the notice naming them and the table read; None when the file cannot
/// be read or parsed at all.
pub(crate) fn read(
    path: &Path,
    base: &toml::Table,
) -> Option<(Config, Option<String>, toml::Table)> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            log::error!("config {}: {e}; keeping the old one", path.display());
            return None;
        }
    };
    match Config::parse_over(&text, base) {
        Ok((cfg, dropped, table)) => {
            log::info!("config {} changed; reloading", path.display());
            for (key, e) in &dropped {
                let kept = if repriseconf::config::kept(&table, base, key) {
                    "keeping its previous value"
                } else {
                    "using its default"
                };
                log::warn!("config {}: {key}: {e}; {kept}", path.display());
            }
            let kept = |key: &str| repriseconf::config::kept(&table, base, key);
            let notice =
                (!dropped.is_empty()).then(|| crate::config::dropped_notice(&dropped, kept));
            Some((cfg, notice, table))
        }
        Err(e) => {
            log::error!("config: {e}; keeping the old one");
            None
        }
    }
}

pub(crate) fn closes_start(old: StartMenuStyle, new: StartMenuStyle) -> bool {
    old != new
}

pub fn reload(state: &mut State, cfg: Config) {
    let redraw = redraw(&state.config, &cfg);
    if redraw == Redraw::Nothing {
        // Programs started since still get the look.
        publish_look(state);
        return;
    }
    let changes = diff(&state.config, &cfg);
    state
        .inputproto
        .shortcuts
        .configured(&state.config.global_shortcuts, &cfg.global_shortcuts);
    // The open Start menu is the old style's: close it before the swap.
    if closes_start(state.config.startmenu_style, cfg.startmenu_style) {
        state.close_start();
    }
    // The old config says which output keys changed.
    let old = std::mem::replace(&mut state.config, cfg);
    // A bar's kept picture may be of the old theme or fonts.
    for bar in &mut state.bars {
        bar.drawn = None;
    }
    // A turn between a row and a column drops the first band's width,
    // a length along the old axis.
    let bands =
        (state.bar_shared.bands).turned(old.taskbar.position, state.config.taskbar.position);
    if bands != state.bar_shared.bands {
        state.bar_shared.bands = bands;
        state.save_taskbar_state();
    }
    if old.taskbar.tray_overflow != state.config.taskbar.tray_overflow {
        crate::tray::replace_all(state);
    }
    if old.session.restore != state.config.session.restore {
        crate::session::restore_changed(state);
    }
    crate::sandbox::set_trusted(&state.config.clipboard.trusted_apps);
    crate::idle::config_changed(state);
    if old.location != state.config.location {
        crate::location::config_changed(state);
    }
    if old.capture_border != state.config.capture_border {
        crate::captureborder::refresh(state);
    }
    if old.notifications != state.config.notifications {
        crate::notifyview::suppress_changed(state);
    }
    if changes.workspaces {
        state.set_desktop_count(state.config.workspaces);
    }
    if old.window_tabs && !state.config.window_tabs {
        state.ungroup_all();
    }
    let global = state.tabs_global.take();
    state.tabs_global = crate::tabsproto::advertise(&state.dh, state.config.window_tabs, global);
    crate::nightlight::update(state);
    // Peek turned off, or its only trigger (the Show desktop strip) gone:
    // end a peek in progress and disarm a pending timer, so reload never
    // leaves windows as outlines with nothing to bring them back.
    if !crate::peek::peek_enabled(&state.config) || !state.config.taskbar.show_desktop_button {
        crate::peek::leave(state);
    }
    let flip = state.switcher.stack.is_some() || state.switcher.leaving.is_some();
    if flip && !state.flip_wanted() {
        state.switcher_cancel();
    }
    if !state.config.clipboard.history {
        state.close_clipview();
        state.clear_history();
    }
    if changes.fonts {
        state.text = crate::text::TextRenderer::load(&state.config.theme.fonts);
        state.caption_text = crate::compositor::caption_text(&state.config.theme);
        state.start_text = crate::compositor::start_text(&state.config.theme);
        state.start_panel_text = reprisekit::interfaces::start_panel_text(&state.config.theme);
        state.invalidate_desktop();
    }
    if changes.icons {
        let roots = reprisekit::icontheme::IconTheme::default_roots();
        let loaded = reprisekit::icontheme::IconTheme::load(&state.config.icons.theme, &roots);
        crate::iconwatch::apply(state, loaded);
    }
    if changes.fonts || changes.icons {
        crate::tray::invalidate_icons(state);
    }
    // A rescan relayouts too; `retheme` below finds the cells drawn.
    if changes.desktop {
        state.rescan_desktop();
    }
    if changes.cursor {
        state.cursors = crate::cursor::Cursors::new(state.config.cursor.effective());
    }
    if changes.pointers {
        state.pointers.apply_all(&state.config);
    }
    match changes.wallpaper {
        WallpaperChange::Image => state.apply_wallpaper_config(),
        WallpaperChange::Position => {
            state.refit_wallpaper();
            crate::lockview::refit(state);
        }
        WallpaperChange::None => {}
    }
    state.water_configured();
    let geometry_changed = changes.outputs && state.apply_output_config(&old);
    if changes.keyboard {
        state.apply_keyboard();
    }
    state.aids_configured();
    let (was, now) = (&old.accessibility, &state.config.accessibility);
    if (was.color_filter, was.color_filter_intensity)
        != (now.color_filter, now.color_filter_intensity)
    {
        log::info!(
            "color filter: {} at {}%",
            now.color_filter.name(),
            now.color_filter_intensity
        );
    }
    state.sync_caret_reports();
    state.mousekeys_configured();
    crate::nightlight::update(state);
    crate::notifyview::config_changed(state);
    if changes.closes_bar_popups() {
        close_bar_popups(state);
        if matches!(
            state.interaction,
            crate::input::Interaction::BarMove { .. } | crate::input::Interaction::BarWidth { .. }
        ) {
            state.cancel_interaction();
        }
    }
    if changes.bar_hide {
        crate::autohideview::config_changed(state);
    }
    if redraw == Redraw::Layout {
        // Nothing drawn changed but the outputs, which a scale warm's end lays out.
        if lays_out(&old, &state.config, geometry_changed) {
            state.update_layout();
            redraw_for_scale(state);
        }
    } else {
        // `reveal_on_*` and `taskbar_hide_ms` reach the bars through the poke
        // in `retheme`'s `redraw_taskbars`.
        retheme(state);
        // The taskbar's height follows the interface and its edge `[taskbar]
        // position`, and the outputs may run another mode, scale or
        // transform: the usable areas move.
        state.update_layout();
    }
    // The panels follow `[debug]` and are redrawn for the new look.
    crate::debugview::config_changed(state);
    if changes.bar_moved || changes.bar_hide {
        // A window whose caption the bar now covers moves down under it;
        // a maximized or fullscreen one does when it is restored.
        let moved: Vec<(WindowId, i32, i32)> = state
            .windows
            .iter()
            .filter(|w| w.mapped && !w.maximized && !w.fullscreen && !w.override_redirect)
            .filter_map(|w| {
                let r = w.frame_rect();
                let o = state.output_at(f64::from(r.x + r.width / 2), f64::from(r.y))?;
                let y = crate::window::below_top_bar(r.y, &state.bar_placement(&o));
                (y != r.y).then_some((w.id, r.x, y))
            })
            .collect();
        for (id, x, y) in moved {
            state.move_to(id, x, y);
        }
    }
    if changes.retired {
        state.config.warn_retired();
    }
    state.rehover_bars();
}

fn close_bar_popups(state: &mut State) {
    state.close_start_panel();
    state.close_menu();
    crate::calendarview::close(state);
    crate::displaysview::close(state);
    crate::trayflyout::close(state);
    crate::trayreveal::collapse_all(state);
    crate::taskpreview::hide(state);
    crate::snappickerview::hide(state);
    crate::notifycenter::close(state);
    crate::tooltip::hide(state);
}

pub fn retheme(state: &mut State) {
    publish_look(state);
    let windows: Vec<_> = state
        .windows
        .iter()
        .filter(|w| w.mapped)
        .map(|w| w.id)
        .collect();
    let mut was = Vec::new();
    for w in windows {
        let frame = state.win(w).map(crate::window::Window::frame_rect);
        if let Some((r, (_, area))) = frame.and_then(|r| Some((r, state.screen_and_area(r)?))) {
            was.push((w, r, area));
        }
        state.relayout(w);
    }
    // A look switch can change the taskbar's height and the frame's
    // metrics, both of which a maximized window fills to.
    state.refit_maximized();
    state.redraw_taskbars();
    for depth in 0..state.menus.len() {
        state.redraw_menu(depth);
    }
    state.refit_wallpaper();
    crate::layer::outputs_changed(state);
    // After the taskbar took its new size, so the work area is the new one.
    for (w, was, was_area) in was {
        state.keep_on_screen_inside(w, was, was_area);
    }
    state.relayout_desktop();
    redraw_for_scale(state);
}

/// Hands the look to programs: the session environment, the generated toolkit themes, the GTK
/// portal and the tray.
fn publish_look(state: &mut State) {
    let c = &state.config;
    // Read per launch, never published: nothing to compare.
    state.env.qt_platform_theme = c.toolkits.qt_platform_theme.clone();
    if state.env.set_look(
        c.theme.colors.to_env(),
        c.interface,
        c.blue_variant,
        (c.glass.mode, c.glass.material),
        &c.icons.theme,
        c.cursor.effective(),
    ) {
        crate::spawn::export_session_env(&state.env);
    }
    // The generated GTK theme lands in its slot before the portal names it.
    crate::toolkits::refresh(state);
    let appearance = crate::tray::settings::Appearance::live(state);
    crate::spawn::theme_portal_gtk(&state.env, appearance.gtk_theme());
    crate::tray::send(state, crate::tray::TrayCommand::Appearance(appearance));
}

/// What `retheme` draws that `update_layout` leaves at the old draw scale: the open Start
/// panel and launcher, the dialogs and the lock screen.
pub fn redraw_for_scale(state: &mut State) {
    crate::tray::sync_icon_sizes(state);
    state.retheme_start_panel();
    state.redraw_launcher();
    for i in 0..state.dialogs.views.len() {
        // Layouts depend on the font and the frame: recompute before
        // redrawing.
        state.refit_dialog(i);
        state.redraw_dialog(i);
    }
    crate::lockview::redraw(state);
    state.redraw_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The config directory's lookups stand in for a stat on a mount that stopped answering:
    /// the loop turns while they hang, and the read after them reloads once they answer.
    #[test]
    fn a_sync_whose_lookups_hang_leaves_the_loop_turning_and_reloads_after() {
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        let path = crate::harness::scratch().join("configwatch-stall/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        h.state.config_path = path.clone();
        let mut watch = DirWatch::new().unwrap();
        watch.sync(&wants(&path));
        h.state.config_watch.watch = Some(watch);
        std::fs::write(&path, "[window_snapping]\ndistance = 15\n").unwrap();
        assert!(h.state.config_watch.drain(), "the write counts");

        let (answer, stalled) = std::sync::mpsc::channel::<()>();
        let started = Instant::now();
        let looked_up = path.clone();
        sync_with(
            &mut h.state,
            move || {
                let _ = stalled.recv_timeout(Duration::from_secs(5));
                wants(&looked_up)
            },
            None,
        );
        h.run_for(50);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the loop turned while the lookups hung"
        );
        assert_ne!(
            h.state.config.window_snapping.distance, 15,
            "not reloaded yet"
        );

        answer.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while h.state.config_watch.syncing() && Instant::now() < end {
            h.run_for(10);
        }
        assert_eq!(h.state.config.window_snapping.distance, 15, "reloaded");
    }

    #[test]
    fn the_first_sync_reads_a_config_written_since_the_startup_read_and_not_an_unchanged_one() {
        let first = |write: bool| {
            let mut h = crate::harness::Harness::new();
            crate::fsjobs::setup(&mut h.state);
            let path =
                crate::harness::scratch().join(format!("configwatch-first-{write}/config.toml"));
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if !write {
                std::fs::write(&path, "[window_snapping]\ndistance = 15\n").unwrap();
            }
            let startup = stamp(&path);
            h.state.config_path = path.clone();
            h.state.config_watch.watch = Some(DirWatch::new().unwrap());
            let written = path.clone();
            sync_with(
                &mut h.state,
                move || {
                    if write {
                        std::fs::write(&written, "[window_snapping]\ndistance = 15\n").unwrap();
                    }
                    wants(&written)
                },
                Some(startup),
            );
            let end = Instant::now() + Duration::from_secs(5);
            while h.state.config_watch.syncing() && Instant::now() < end {
                h.run_for(10);
            }
            h.state.config.window_snapping.distance
        };
        assert_eq!(first(true), 15, "written before the watch: reloaded");
        assert_ne!(
            first(false),
            15,
            "unchanged since the startup read: not reread"
        );
    }

    #[test]
    fn a_reload_redraws_the_layout_for_output_keys_alone_and_nothing_for_an_identical_config() {
        let old = Config::default();
        assert_eq!(redraw(&old, &old.clone()), Redraw::Nothing);
        let mut scaled = old.clone();
        scaled.outputs.push(crate::config::OutputConfig {
            name: "HDMI-A-1".into(),
            scale: 1.25,
            mode: None,
            adaptive_sync: None,
            transform: Default::default(),
            position: None,
            primary: false,
        });
        assert_eq!(redraw(&old, &scaled), Redraw::Layout, "a scale change");
        let mut primary = scaled.clone();
        primary.outputs[0].primary = true;
        assert_eq!(redraw(&scaled, &primary), Redraw::Layout, "a new primary");
        assert!(
            lays_out(&scaled, &primary, false),
            "a new primary moves the desktop with no geometry change"
        );
        assert!(!lays_out(&old, &scaled, false), "nothing moved");
        let mut vrr = old.clone();
        vrr.adaptive_sync = !old.adaptive_sync;
        assert_eq!(redraw(&old, &vrr), Redraw::Layout, "adaptive sync");
        let mut both = scaled.clone();
        both.taskbar.auto_hide = !old.taskbar.auto_hide;
        assert_eq!(
            redraw(&old, &both),
            Redraw::All,
            "a scale change with a taskbar key"
        );
    }

    #[test]
    fn a_new_side_width_moves_a_side_bar_and_not_a_bottom_one() {
        let bottom = Config::default();
        let mut wider = bottom.clone();
        wider.taskbar.side_width = 120;
        assert_eq!(diff(&bottom, &wider), Changes::default());
        let mut left = bottom.clone();
        left.taskbar.position = reprisekit::taskbar::Edge::Left;
        let mut left_wider = left.clone();
        left_wider.taskbar.side_width = 120;
        assert_eq!(
            diff(&left, &left_wider),
            Changes {
                bar_moved: true,
                ..Changes::default()
            }
        );
    }

    #[test]
    fn a_new_taskbar_edge_closes_the_bars_popups() {
        let old = Config::default();
        let mut new = old.clone();
        new.taskbar.position = reprisekit::taskbar::Edge::Top;
        assert_eq!(
            diff(&old, &new),
            Changes {
                bar_moved: true,
                ..Changes::default()
            }
        );
        assert!(!diff(&new, &new.clone()).bar_moved);
        assert!(diff(&old, &new).closes_bar_popups());
        assert!(!diff(&new, &new.clone()).closes_bar_popups());
    }

    #[test]
    fn turning_auto_hide_on_or_off_moves_the_reserved_strip_and_closes_the_bars_popups() {
        let old = Config::default();
        let mut new = old.clone();
        new.taskbar.auto_hide = true;
        assert_eq!(
            diff(&old, &new),
            Changes {
                bar_hide: true,
                ..Changes::default()
            }
        );
        assert!(diff(&new, &old).bar_hide, "and off again");
        assert!(
            diff(&new, &old).closes_bar_popups(),
            "turning auto-hide off closes the bars' popups"
        );
        assert!(
            diff(&old, &new).closes_bar_popups(),
            "and so does turning it on"
        );
        let mut quick = new.clone();
        quick.effects.taskbar_hide_ms = 0;
        quick.taskbar.reveal_on_keys = false;
        assert_eq!(diff(&new, &quick), Changes::default());
    }

    #[test]
    fn reload_closes_an_open_start_menu_when_the_style_changes() {
        assert!(closes_start(StartMenuStyle::Glass, StartMenuStyle::Classic));
        assert!(!closes_start(StartMenuStyle::Glass, StartMenuStyle::Glass));
    }

    #[test]
    fn switching_interface_reloads_fonts() {
        let a = Config::parse("").unwrap();
        let b = Config::parse("[appearance]\ninterface = \"blue\"\n").unwrap();
        assert!(diff(&a, &b).fonts);
    }

    #[test]
    fn a_reload_reads_the_keybinds_layered_over_the_defaults() {
        let path =
            std::env::temp_dir().join(format!("reprise-keybinds-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[keybinds]]\nkeys = \"Super+Return\"\naction = \"terminal\"\n\n\
             [[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"\n",
        )
        .unwrap();
        let (cfg, _, _) = read(&path, &toml::Table::new()).expect("the file parses");
        std::fs::remove_file(&path).unwrap();
        let on = |keys: &str| {
            let (mods, keysym) = crate::config::parse_keybind(keys).unwrap();
            (cfg.keybinds.iter())
                .find(|b| b.mods == mods && b.keysym == keysym)
                .map(|b| b.action.clone())
        };
        assert_eq!(on("Super+Tab"), Some(crate::config::Action::Flip));
        assert_eq!(on("Super+q"), None);
        assert_eq!(on("Alt+F4"), Some(crate::config::Action::Close));
    }

    #[test]
    fn a_save_through_a_symlinked_config_is_seen() {
        let root = std::env::temp_dir().join(format!("reprise-configwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["conf", "dotfiles", "other"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let link = root.join("conf/config.toml");
        std::fs::write(root.join("dotfiles/config.toml"), "").unwrap();
        std::os::unix::fs::symlink("../dotfiles/config.toml", &link).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&link));

        repriseconf::write_atomic(&link, "terminal = \"foot\"\n").unwrap();
        assert!(w.drain(), "a save through the link: reload");
        std::fs::write(root.join("dotfiles/notes.txt"), "x").unwrap();
        assert!(!w.drain(), "another file beside the target: no reload");

        std::fs::write(root.join("other/config.toml"), "").unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("../other/config.toml", &link).unwrap();
        w.sync(&wants(&link));
        repriseconf::write_atomic(&link, "").unwrap();
        assert!(w.drain(), "a save through the moved link: reload");
        std::fs::write(root.join("dotfiles/config.toml"), "").unwrap();
        assert!(!w.drain(), "the old target: no reload");

        std::fs::write(root.join("conf/real.toml"), "").unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("real.toml", &link).unwrap();
        w.sync(&wants(&link));
        repriseconf::write_atomic(&link, "").unwrap();
        assert!(
            w.drain(),
            "a save through a link to a file beside it: reload"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_image_loads_and_a_new_position_refits() {
        let old = Config::parse("[wallpaper]\nimage = \"/a.jpg\"\n").unwrap();
        let image = Config::parse("[wallpaper]\nimage = \"/b.jpg\"\n").unwrap();
        let position =
            Config::parse("[wallpaper]\nimage = \"/a.jpg\"\nposition = \"tile\"\n").unwrap();
        assert_eq!(diff(&old, &image).wallpaper, WallpaperChange::Image);
        assert_eq!(diff(&old, &position).wallpaper, WallpaperChange::Position);
        assert_eq!(diff(&old, &old).wallpaper, WallpaperChange::None);
    }

    #[test]
    fn identical_configs_change_nothing() {
        let c = Config::default();
        assert_eq!(diff(&c, &c), Changes::default());
    }

    #[test]
    fn fonts_only() {
        let old = Config::default();
        let mut new = old.clone();
        new.theme.fonts.family.push("Some Font".into());
        let d = diff(&old, &new);
        assert!(d.fonts);
        assert!(!d.icons && !d.desktop && !d.keyboard);
        assert!(!d.workspaces);
    }

    #[test]
    fn icon_theme_only() {
        let old = Config::default();
        let mut new = old.clone();
        new.icons.theme = "other-theme".into();
        let d = diff(&old, &new);
        assert!(d.icons);
        assert!(!d.fonts && !d.desktop && !d.keyboard);
    }

    #[test]
    fn system_icons_desktop_icons_and_extensions_rescan() {
        let old = Config::default();
        let mut new = old.clone();
        new.system_icons = vec!["recycle-bin".into()];
        assert!(diff(&old, &new).desktop);
        let mut new = old.clone();
        new.desktop_icons = !old.desktop_icons;
        let d = diff(&old, &new);
        assert!(d.desktop);
        assert!(!d.workspaces);
        assert!(diff(&new, &old).desktop);
        let mut new = old.clone();
        new.desktop_extensions = reprisefs::names::Extensions::Show;
        assert!(diff(&old, &new).desktop);
    }

    #[test]
    fn cursor_theme_and_size_reload_the_cursor() {
        let old = Config::default();
        let mut new = old.clone();
        new.cursor.theme = "Adwaita".into();
        let d = diff(&old, &new);
        assert!(d.cursor);
        assert!(!d.pointers && !d.icons && !d.workspaces);
        let mut new = old.clone();
        new.cursor.size = 48;
        assert!(diff(&old, &new).cursor);
    }

    #[test]
    fn keyboard_layout_and_repeat() {
        let old = Config::default();
        let mut new = old.clone();
        new.keyboard.layout = "de".into();
        assert!(diff(&old, &new).keyboard);
        let mut new = old.clone();
        new.keyboard.repeat_rate = old.keyboard.repeat_rate + 1;
        assert!(diff(&old, &new).keyboard);
    }

    #[test]
    fn mouse_touchpad_and_touchscreen_reconfigure_the_devices() {
        let old = Config::default();
        let mut new = old.clone();
        new.mouse.acceleration = false;
        assert!(diff(&old, &new).pointers);
        let mut new = old.clone();
        new.touchpad.tap_to_click = false;
        assert!(diff(&old, &new).pointers);
        let mut new = old.clone();
        new.mouse.natural_scroll = true;
        assert!(diff(&old, &new).pointers);
        let mut new = old.clone();
        new.touchscreen.output = Some("DP-1".into());
        assert!(diff(&old, &new).pointers);
    }

    #[test]
    fn the_desktop_count_is_applied_at_once() {
        let new = Config {
            base: repriseconf::config::Config {
                workspaces: 1,
                ..Default::default()
            },
            ..Config::default()
        };
        assert!(diff(&Config::default(), &new).workspaces);
        assert!(!diff(&new, &new.clone()).workspaces);
    }

    #[test]
    fn a_retired_key_only_warns() {
        let new = Config::parse("renderer = \"gles2\"\n").unwrap();
        assert_eq!(
            diff(&Config::default(), &new),
            Changes {
                retired: true,
                ..Changes::default()
            }
        );
        assert!(!diff(&new, &new.clone()).retired, "no second warning");
    }

    #[test]
    fn mode_transform_scale_and_adaptive_sync_are_applied_at_once() {
        let old = Config::default();
        let mut new = old.clone();
        new.adaptive_sync = true;
        let c = diff(&old, &new);
        assert!(c.outputs);
        assert!(!c.workspaces);
        let mut new = old.clone();
        new.outputs.push(crate::config::OutputConfig {
            name: "DP-1".into(),
            scale: 1.0,
            mode: None,
            adaptive_sync: Some(true),
            transform: Default::default(),
            position: None,
            primary: false,
        });
        let c = diff(&old, &new);
        assert!(c.outputs);
        assert!(!c.workspaces, "a new entry at scale 1 changes no scale");
        let old = new.clone();
        let mut new = old.clone();
        new.outputs[0].transform = crate::config::Transform::Rotate90;
        new.outputs[0].mode = Some(crate::config::OutputMode::parse("1280x800").unwrap());
        let c = diff(&old, &new);
        assert!(c.outputs);
        assert!(!c.workspaces);
        let mut new = old.clone();
        new.outputs[0].scale = 1.5;
        let c = diff(&old, &new);
        assert!(c.outputs);
        assert!(!c.workspaces, "a scale is applied at once");
    }

    #[test]
    fn a_reload_commits_only_the_output_keys_that_changed() {
        let parse = |s: &str| Config::parse(s).unwrap();
        let old = parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@60\"\n");
        let vrr =
            parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@60\"\nadaptive_sync = true\n");
        assert_eq!(
            output_change(&old, &vrr, "DP-1"),
            OutputChange {
                adaptive_sync: true,
                ..OutputChange::default()
            }
        );
        let top =
            parse("adaptive_sync = true\n[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@60\"\n");
        assert!(
            output_change(&old, &top, "HDMI-A-1").adaptive_sync,
            "the top-level key reaches an output without an entry"
        );
        let other = parse(
            "[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@60\"\n\
             [[outputs]]\nname = \"DP-2\"\nscale = 2\n",
        );
        assert_eq!(
            output_change(&old, &other, "DP-1"),
            OutputChange::default(),
            "another output's entry leaves DP-1 alone"
        );
        assert!(output_change(&old, &other, "DP-2").scale);
        let mode = parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1280x720@60\"\n");
        let change = output_change(&old, &mode, "DP-1");
        assert_eq!(
            change,
            OutputChange {
                mode: true,
                ..OutputChange::default()
            }
        );
        assert!(change.any() && !OutputChange::default().any());
        let placed =
            parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@60\"\nposition = [0, 1080]\n");
        assert_eq!(
            output_change(&old, &placed, "DP-1"),
            OutputChange {
                position: true,
                ..OutputChange::default()
            }
        );
    }

    #[test]
    fn theme_colour_needs_no_side_work() {
        let old = Config::default();
        let mut new = old.clone();
        new.theme.colors.background = crate::theme::Color::rgb(0, 64, 64);
        new.terminal = "xterm".into();
        assert_eq!(diff(&old, &new), Changes::default());
    }
}
