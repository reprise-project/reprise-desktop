use std::collections::BTreeMap;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::window::{Kind, Window, WindowId, area_client, first_configure_size, x11_geometry};
use crate::windowrules::{Remembered, Want, WindowState};

pub struct Wm {
    pub remembered: BTreeMap<String, Remembered>,
    pub snap: crate::snapview::Snapping,
    pub shake: crate::shakeview::Shaking,
    pub file_dialog: crate::filedlggeom::Kept,
}

impl Wm {
    /// `p`'s remembered frames; the file dialogs' kept place is read only for a `session`.
    pub fn from_persisted(p: &crate::state::Persisted, session: bool) -> Wm {
        let file_dialog = if session {
            crate::filedlggeom::Kept::load()
        } else {
            crate::filedlggeom::Kept {
                path: crate::state::path().with_file_name("filedialog.toml"),
                geometry: None,
            }
        };
        Wm {
            remembered: p.window_geometry.clone(),
            snap: Default::default(),
            shake: Default::default(),
            file_dialog,
        }
    }
}

/// What placing a window reads of it: from a live window, or from an X11 window before it maps.
pub(crate) struct Subject {
    pub app: String,
    pub title: String,
    /// See [`Window::placeable`].
    pub placeable: bool,
    pub fixed: bool,
}

impl Window {
    /// Whether the window rules and the app's remembered frame place this
    /// window: a top-level window of an app that names itself. Dialogs
    /// keep their own placement (file dialogs the shared one, `filedlggeom`),
    /// X11 menus and tooltips their own place.
    pub fn placeable(&self) -> bool {
        !self.override_redirect
            && !self.is_owned()
            && !self.is_modal()
            && !self.file_dialog
            && !self.app().is_empty()
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

    pub(crate) fn plan_window(&self, w: WindowId) -> Option<Want> {
        let win = self.win(w)?;
        let subject = Subject {
            app: win.app(),
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
        let first = self.newest_of_app(app_id, w).is_none();
        let mut base = Want::default();
        if self.config.remember_windows
            && !fixed
            && let Some(m) = self.wm.remembered.get(app_id)
        {
            base = m.want(first, self.config.accessibility.text_size);
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
        if let Some(d) = self
            .session
            .regrouping
            .desktop(app_id, title, std::time::Instant::now())
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

    pub fn placement_output(&self, want: Option<&Want>) -> Option<Output> {
        let named = want.and_then(|w| w.output.as_deref()).and_then(|name| {
            self.scene
                .outputs()
                .iter()
                .find(|o| o.name() == name && self.layout_box(Some(o)).width > 0)
                .cloned()
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

    /// Oldest first, so each app keeps its newest.
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
    }

    /// Where `w` is: its normal frame rect relative to the output under it,
    /// and whether it is maximized. A snapped window's frame is its zone.
    pub fn frame_of(&self, w: WindowId) -> Option<Remembered> {
        let win = self.win(w)?;
        let normal = match (win.fullscreen, win.maximized) {
            (true, false) => win.saved_fullscreen,
            (_, true) => win.saved,
            _ => None,
        }
        .unwrap_or(win.frame_rect());
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

    /// Keep where `w` is (`frame_of`) as its app's remembered frame.
    pub fn remember_window(&mut self, w: WindowId, last_only: bool) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !self.config.remember_windows || !win.placeable() || !win.is_resizable() {
            return;
        }
        let app_id = win.app();
        if last_only && self.newest_of_app(&app_id, Some(w)).is_some() {
            return;
        }
        let Some(entry) = self.frame_of(w) else {
            return;
        };
        if self.wm.remembered.get(&app_id) != Some(&entry) {
            log::info!("window {app_id:?}: remembered {entry:?}");
            self.wm.remembered.insert(app_id, entry);
            crate::state::save_window_geometry(&self.wm.remembered);
        }
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
