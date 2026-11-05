use std::time::Duration;

use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::window::{Kind, Window, WindowId, area_client, first_configure_size, x11_geometry};
use crate::windowrules::{Remembered, Want, WindowRule, WindowState, has_open_here, put_open_here};
use crate::winmenu::OpenHereState;

pub struct Wm {
    pub kept: crate::keptframes::KeptFrames,
    pub snap: crate::snapview::Snapping,
    pub shake: crate::shakeview::Shaking,
    pub file_dialog: crate::filedlggeom::Kept,
    /// Armed while `kept` has changes not yet queued for its file.
    save: Option<RegistrationToken>,
}

/// How long changes to the kept frames gather before they are written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

impl Wm {
    /// The kept frames and the file dialogs' kept place are read only for a `session`.
    pub fn new(session: bool) -> Wm {
        let file_dialog = if session {
            crate::filedlggeom::Kept::load()
        } else {
            crate::filedlggeom::Kept::parse(
                crate::state::path().with_file_name("filedialog.toml"),
                "",
            )
        };
        Wm {
            kept: if session {
                crate::keptframes::KeptFrames::load()
            } else {
                crate::keptframes::KeptFrames::empty(crate::keptframes::path())
            },
            snap: Default::default(),
            shake: Default::default(),
            file_dialog,
            save: None,
        }
    }
}

/// What placing a window reads of it: from a live window, or from an X11 window before it maps.
pub(crate) struct Subject {
    pub app: String,
    /// What its frame is kept under: [`Window::kept_key`].
    pub key: String,
    pub title: String,
    /// See [`Window::placeable`].
    pub placeable: bool,
    pub fixed: bool,
}

impl Window {
    /// Whether the window rules and the app's remembered frame place this
    /// window: a top-level window of an app that names itself. Dialogs
    /// keep their own placement (file dialogs their app's, `filedlggeom`),
    /// X11 menus and tooltips their own place.
    pub fn placeable(&self) -> bool {
        !self.places_itself() && !self.app().is_empty()
    }

    /// A dialog or an X11 menu or tooltip: rules never place it.
    fn places_itself(&self) -> bool {
        self.override_redirect || self.is_owned() || self.is_modal() || self.file_dialog
    }

    /// The last app id seen once the client is gone: a disconnecting client can
    /// take its surface's state with it before the window is torn down.
    pub fn app(&self) -> String {
        let live = self.app_id();
        if live.is_empty() {
            self.app_seen.clone()
        } else {
            live
        }
    }

    /// What the window's remembered frame is kept under: its app id, or `app#tag` for a window
    /// with a [`place_tag`](Window::place_tag), so each such window of the app keeps its own.
    pub fn kept_key(&self) -> String {
        match &self.place_tag {
            Some(t) => format!("{}#{t}", self.app()),
            None => self.app(),
        }
    }

    pub fn apply_rule(&mut self, want: &Want, desktops: usize) {
        if let Some(d) = want.desktop {
            match rule_desktop(d, desktops) {
                Some(d) => self.desktop = d,
                None => log::warn!(
                    "window rule for {:?}: desktop {} is past the last of {desktops}",
                    self.app(),
                    d + 1
                ),
            }
        }
        if let Some(v) = want.sticky {
            self.sticky = v;
        }
        if let Some(v) = want.always_on_top {
            self.on_top = v;
        }
    }
}

fn rule_desktop(desktop: usize, desktops: usize) -> Option<usize> {
    (desktop < desktops).then_some(desktop)
}

impl State {
    pub fn newest_of_app(&self, app_id: &str, except: Option<WindowId>) -> Option<WindowId> {
        self.windows
            .iter()
            .rev()
            .filter(|w| w.mapped && Some(w.id) != except && w.placeable())
            .find(|w| w.app() == app_id)
            .map(|w| w.id)
    }

    /// The newest mapped window whose frame is kept under `key` ([`Window::kept_key`]).
    fn newest_kept(&self, key: &str, except: Option<WindowId>) -> Option<WindowId> {
        self.windows
            .iter()
            .rev()
            .filter(|w| w.mapped && Some(w.id) != except && w.placeable())
            .find(|w| w.kept_key() == key)
            .map(|w| w.id)
    }

    pub(crate) fn plan_window(&self, w: WindowId) -> Option<Want> {
        let win = self.win(w)?;
        let subject = Subject {
            app: win.app(),
            key: win.kept_key(),
            title: win.title(),
            placeable: win.placeable(),
            fixed: !win.is_resizable(),
        };
        self.plan_for(&subject, Some(w))
    }

    /// The placement the rules, the app's remembered frame and a restoring session give `s`;
    /// `w` is its window once it has one.
    pub(crate) fn plan_for(&self, s: &Subject, w: Option<WindowId>) -> Option<Want> {
        if !s.placeable {
            return None;
        }
        let (app_id, title, fixed) = (&s.app, &s.title, s.fixed);
        let now = std::time::Instant::now();
        let first = self.newest_kept(&s.key, w).is_none();
        let mut base = Want::default();
        if self.config.remember_windows
            && !fixed
            && let Some(m) = self.wm.kept.get(&self.output_set(), &s.key)
        {
            base = match self.kept_on_screen(m) {
                Some(m) => m.want(first, self.config.accessibility.text_size),
                None => Want {
                    position: None,
                    output: None,
                    ..m.want(first, self.config.accessibility.text_size)
                },
            };
        }
        if !fixed
            && let Some(f) = self
                .session
                .regrouping
                .frame(app_id, title, std::time::Instant::now())
        {
            base = self.regroup_want(f);
        }
        if first
            && let Some(d) = self
                .session
                .restoring
                .desktop(app_id, std::time::Instant::now())
            && d < self.desktops.count
        {
            base.desktop = Some(d);
        }
        if let Some((d, monitor)) = (self.session.resnapping).place(app_id, title, now) {
            if d < self.desktops.count {
                base.desktop = Some(d);
            }
            if let Some(out) = self.output_of_monitor(monitor) {
                base.output = Some(out.name());
            }
        }
        if let Some(d) = self.session.regrouping.desktop(app_id, title, now)
            && d < self.desktops.count
        {
            base.desktop = Some(d);
        }
        let mut want = crate::windowrules::want(&self.config.window_rules, app_id, title, base);
        if fixed {
            want.size = None;
        }
        (want != Want::default()).then_some(want)
    }

    /// Kept frame `m` relative to the output its title bar lands on; None when its output is
    /// gone or its title bar is on no output.
    fn kept_on_screen(&self, m: &Remembered) -> Option<Remembered> {
        let boxes: Vec<(crate::raster::Rect, Output)> = (self.scene.outputs().iter())
            .map(|o| (self.layout_box(Some(o)), o.clone()))
            .collect();
        let (b, _) = boxes
            .iter()
            .find(|(b, o)| o.name() == m.output && b.width > 0)?;
        let (x, y) = (b.x + m.x, b.y + m.y);
        let metrics = &self.config.theme.metrics;
        let bar =
            crate::raster::Rect::new(x, y, m.width, metrics.caption_height + metrics.frame_width);
        let (on, out) = crate::window::landing(bar, &boxes)?;
        Some(Remembered {
            output: out.name(),
            x: x - on.x,
            y: y - on.y,
            ..m.clone()
        })
    }

    /// A restored tab group's frame `f` as a placement: on its output, or
    /// fitted to the primary one when that output is gone. A normal frame
    /// says so, so a state the first configure planned is not kept.
    fn regroup_want(&self, f: &Remembered) -> Want {
        let mut want = f.want(true, self.config.accessibility.text_size);
        let present = self
            .scene
            .outputs()
            .iter()
            .any(|o| o.name() == f.output && self.layout_box(Some(o)).width > 0);
        if !present {
            want.output = self.first_output().map(|o| o.name());
        }
        if !f.maximized {
            want.state = Some(WindowState::Normal);
        }
        want
    }

    /// The output `want` names, by connector or monitor id, else the one under the pointer.
    pub fn placement_output(&self, want: Option<&Want>) -> Option<Output> {
        let named = want.and_then(|w| w.output.as_deref()).and_then(|name| {
            (self.scene.outputs().iter())
                .find(|o| o.name() == name && self.layout_box(Some(o)).width > 0)
                .cloned()
                .or_else(|| self.output_of_monitor(name))
        });
        named.or_else(|| {
            let p = self.pointer_location();
            self.output_at(p.x, p.y)
        })
    }

    /// Runs before an xdg toplevel's initial configure, and again while unmapped
    /// when its title, app id or decoration changes; for an X11 window when
    /// Xwayland ties its surface, where only the size goes out now.
    pub fn configure_initial(&mut self, w: WindowId) {
        self.note_app_id(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let old = win.placement.take();
        if let Some(old) = old
            && let Some(t) = win.xdg()
        {
            t.with_pending_state(|s| {
                match old.state {
                    Some(WindowState::Fullscreen) => {
                        s.states.unset(xdg_toplevel::State::Fullscreen)
                    }
                    Some(WindowState::Maximized) => s.states.unset(xdg_toplevel::State::Maximized),
                    _ => false,
                };
                s.size = None;
            });
        }
        let want = self.plan_window(w);
        let out = self.placement_output(want.as_ref());
        let area = self.usable_area(out.as_ref());
        let metrics = &self.config.theme.metrics;
        let most = self.win(w).map(|x| area_client(metrics, area, x.chrome()));
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.placement = want.clone();
        if let (Some(t), Some(most)) = (win.xdg(), most)
            && area.width > 0
        {
            t.with_pending_state(|s| s.bounds = Some(most.into()));
        }
        let Some(want) = want else {
            if let Some((cw, ch)) = self.file_dialog_size(w)
                && let Some(t) = self.win(w).and_then(Window::xdg)
            {
                t.with_pending_state(|s| s.size = Some((cw.max(1), ch.max(1)).into()));
            }
            self.send_configure(w);
            return;
        };
        let screen = self.layout_box(out.as_ref());
        let Some(win) = self.win(w) else {
            return;
        };
        let limits = win.size_limits();
        let resizable = limits.resizable();
        let size = first_configure_size(
            &self.config.theme.metrics,
            &want,
            limits,
            screen,
            area,
            win.chrome(),
        );
        match &win.kind {
            Kind::Xdg(t) => t.with_pending_state(|s| {
                match want.state {
                    Some(WindowState::Fullscreen) => {
                        s.states.set(xdg_toplevel::State::Fullscreen);
                    }
                    Some(WindowState::Maximized) if resizable => {
                        s.states.set(xdg_toplevel::State::Maximized);
                    }
                    _ => {}
                }
                if let Some((cw, ch)) = size {
                    s.size = Some((cw.max(1), ch.max(1)).into());
                }
            }),
            Kind::X11(x) => {
                if let Some((cw, ch)) = size {
                    let at = x.last_configure().loc;
                    let (gx, gy, gw, gh) = x11_geometry(at.x, at.y, cw, ch);
                    let rect = Rectangle::new(
                        (i32::from(gx), i32::from(gy)).into(),
                        (i32::from(gw), i32::from(gh)).into(),
                    );
                    if let Err(e) = x.configure(rect) {
                        log::debug!("x11 configure: {e}");
                    }
                }
            }
        }
        self.send_configure(w);
    }

    pub fn note_app_id(&mut self, w: WindowId) {
        self.toplevels.dirty = true;
        if let Some(win) = self.win_mut(w) {
            let live = win.app_id();
            if !live.is_empty() {
                win.app_seen = live;
            }
        }
    }

    pub fn replan_unmapped(&mut self, w: WindowId) {
        if self
            .win(w)
            .and_then(|x| x.xdg().filter(|_| !x.mapped))
            .is_some_and(|t| t.is_initial_configure_sent())
        {
            self.configure_initial(w);
        }
    }

    /// Oldest first, so each app keeps its newest; the write is queued at once.
    pub fn remember_open_windows(&mut self) {
        let open: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped)
            .map(|w| w.id)
            .collect();
        for w in open {
            self.remember_window(w, false);
        }
        self.save_remembered();
    }

    /// Where `w` is: its normal frame rect relative to the output under it,
    /// and whether it is maximized. A snapped window's frame is its zone.
    pub fn frame_of(&self, w: WindowId) -> Option<Remembered> {
        let win = self.win(w)?;
        let normal = win.normal_frame();
        let (cx, cy) = (normal.x + normal.width / 2, normal.y + normal.height / 2);
        let out = self.output_at(cx as f64, cy as f64)?;
        let b = self.layout_box(Some(&out));
        Some(Remembered {
            output: out.name(),
            x: normal.x - b.x,
            y: normal.y - b.y,
            width: normal.width,
            height: normal.height,
            maximized: win.maximized,
            text_size: self.config.accessibility.text_size,
        })
    }

    /// The monitor id (`outputlayouts::monitor_ids`) of the laid-out output named `name`.
    fn monitor_of_output(&self, name: &str) -> Option<String> {
        let outs: Vec<&Output> = (self.scene.outputs().iter())
            .filter(|o| self.layout_box(Some(o)).width > 0)
            .collect();
        let ids = crate::outputlayouts::monitor_ids(&outs);
        (outs.into_iter().zip(ids)).find_map(|(o, m)| (o.name() == name).then_some(m))
    }

    /// Always Open Here's row in `w`'s window menu.
    pub(crate) fn open_here_state(&self, w: WindowId) -> OpenHereState {
        let Some(win) = self.win(w) else {
            return OpenHereState::Absent;
        };
        let app = win.app();
        if win.places_itself() || win.dialog_tag != crate::window::DialogTag::None {
            OpenHereState::Absent
        } else if app.is_empty() || app.contains(['*', '?']) {
            OpenHereState::Greyed
        } else if has_open_here(&self.config.window_rules, &app) {
            OpenHereState::On
        } else {
            OpenHereState::Off
        }
    }

    /// Always Open Here picked: removes its rule for `w`'s program, or writes one opening the
    /// program's windows where `w` is now. The live rules change at once; the config watch
    /// rereads the same from the file.
    pub(crate) fn toggle_open_here(&mut self, w: WindowId) {
        let state = self.open_here_state(w);
        let Some(app) = self.win(w).map(Window::app) else {
            return;
        };
        let rule = match state {
            OpenHereState::On => None,
            OpenHereState::Off => {
                let Some(at) = self.frame_of(w) else {
                    return;
                };
                let Some(monitor) = self.monitor_of_output(&at.output) else {
                    return;
                };
                let (on_top, sticky) = (
                    self.stays_on_top(w),
                    self.win(self.home(w)).is_some_and(|h| h.sticky),
                );
                Some(WindowRule::open_here(&app, monitor, &at, on_top, sticky))
            }
            OpenHereState::Absent | OpenHereState::Greyed => return,
        };
        log::info!("window {app:?}: Always Open Here {rule:?}");
        put_open_here(&mut self.config.window_rules, &app, rule.clone());
        crate::configwrite::save_open_here(&self.config_path, app, rule);
    }

    /// Keep where `w` is (`frame_of`) as its app's remembered frame.
    pub fn remember_window(&mut self, w: WindowId, last_only: bool) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !self.config.remember_windows || !win.placeable() || !win.is_resizable() {
            return;
        }
        let app_id = win.kept_key();
        if last_only && self.newest_kept(&app_id, Some(w)).is_some() {
            return;
        }
        let Some(entry) = self.frame_of(w) else {
            return;
        };
        let set = self.output_set();
        if self.wm.kept.store(&set, &app_id, entry.clone()) {
            log::info!("window {app_id:?}: remembered {entry:?} for {set:?}");
            self.save_remembered_soon();
        }
    }

    fn save_remembered_soon(&mut self) {
        if self.wm.save.is_some() {
            return;
        }
        let timer = Timer::from_duration(SAVE_DELAY);
        let armed = self.handle.insert_source(timer, |_, _, state| {
            state.wm.save = None;
            state.wm.kept.save();
            TimeoutAction::Drop
        });
        match armed {
            Ok(t) => self.wm.save = Some(t),
            Err(_) => self.wm.kept.save(),
        }
    }

    /// Queue the remembered frames' write now if changes wait for it: before the compositor exits.
    pub fn save_remembered(&mut self) {
        if let Some(t) = self.wm.save.take() {
            self.handle.remove(t);
            self.wm.kept.save();
        }
    }

    /// Drops every kept frame, for every set of screens, and writes the empty file.
    pub fn forget_window_frames(&mut self) {
        if let Some(t) = self.wm.save.take() {
            self.handle.remove(t);
        }
        self.wm.kept.clear();
        self.wm.kept.save();
        log::info!("window frames forgotten");
    }

    /// Rereads the kept frames and the file dialogs' kept place from their files, dropping a
    /// pending write of the old ones.
    pub fn reload_remembered(&mut self) {
        if let Some(t) = self.wm.save.take() {
            self.handle.remove(t);
        }
        let path = self.wm.kept.path.clone();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        self.wm.kept = crate::keptframes::KeptFrames::parse(path, &text);
        let path = self.wm.file_dialog.path.clone();
        let text = repriseconf::filedialog::load_from(&path);
        self.wm.file_dialog = crate::filedlggeom::Kept::parse(path, &text);
        log::info!("remembered places reread");
    }
}

#[cfg(test)]
mod tests {
    use super::rule_desktop;

    #[test]
    fn a_rule_desktop_past_the_last_is_ignored() {
        assert_eq!(rule_desktop(0, 1), Some(0));
        assert_eq!(rule_desktop(2, 3), Some(2));
        assert_eq!(
            rule_desktop(3, 3),
            None,
            "desktop 4 of 3 keeps the current one"
        );
    }
}
