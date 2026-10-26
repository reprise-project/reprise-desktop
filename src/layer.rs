use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{PopupKind, PopupManager},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point},
    wayland::{
        compositor::with_states,
        shell::{
            wlr_layer::{
                Anchor, KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface,
                LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::PopupSurface,
        },
    },
};

use reprisescene::{SurfaceId, TreeId};

use crate::compositor::{SceneOwner, State};
use crate::raster::Rect;

pub struct LayerShell {
    pub state: WlrLayerShellState,
    pub surfaces: Vec<LayerSurface>,
    pub focused: Option<WlSurface>,
    usable: Vec<(Output, Rect, Rect)>,
}

impl LayerShell {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> LayerShell {
        LayerShell {
            state: WlrLayerShellState::new_with_filter::<State, _>(
                dh,
                crate::sandbox::filter("zwlr_layer_shell_v1"),
            ),
            surfaces: Vec::new(),
            focused: None,
            usable: Vec::new(),
        }
    }

    pub fn usable(&self, output: &Output, full: Rect) -> Option<Rect> {
        self.usable
            .iter()
            .find(|(o, f, _)| o == output && *f == full)
            .map(|(_, _, u)| *u)
    }
}

pub struct LayerSurface {
    pub surface: WlrLayerSurface,
    /// Layer surfaces never migrate.
    pub output: Output,
    tree: TreeId,
    popups: Vec<(WlSurface, SurfaceId)>,
    seen: Option<Wants>,
    pub mapped: bool,
}

impl LayerSurface {
    fn wl(&self) -> &WlSurface {
        self.surface.wl_surface()
    }

    fn exclusive(&self) -> bool {
        self.seen
            .is_some_and(|w| w.keyboard == KeyboardInteractivity::Exclusive)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wants {
    placement: Placement,
    layer: Layer,
    keyboard: KeyboardInteractivity,
}

fn wants(ls: &WlrLayerSurface) -> Wants {
    ls.with_cached_state(|s| Wants {
        placement: Placement {
            anchor: s.anchor,
            zone: s.exclusive_zone.into(),
            edge: s.exclusive_edge,
            margin: [s.margin.top, s.margin.right, s.margin.bottom, s.margin.left],
            size: (s.size.w, s.size.h),
        },
        layer: s.layer,
        keyboard: s.keyboard_interactivity,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub anchor: Anchor,
    /// > 0 reserves, 0 does not, -1 ignores other surfaces' zones.
    pub zone: i32,
    pub edge: Option<Anchor>,
    /// Top, right, bottom, left.
    pub margin: [i32; 4],
    /// 0 stretches between the anchors.
    pub size: (i32, i32),
}

impl Placement {
    /// The client's values held to `full`, the output they are placed on: a zone up to its longer
    /// side, margins and size up to its extent on their axis.
    pub fn clamped(self, full: Rect) -> Placement {
        let (fw, fh) = (full.width.max(0), full.height.max(0));
        let [top, right, bottom, left] = self.margin;
        Placement {
            zone: self.zone.clamp(-1, fw.max(fh)),
            margin: [
                top.clamp(-fh, fh),
                right.clamp(-fw, fw),
                bottom.clamp(-fh, fh),
                left.clamp(-fw, fw),
            ],
            size: (self.size.0.clamp(0, fw), self.size.1.clamp(0, fh)),
            ..self
        }
    }
}

impl Default for Placement {
    fn default() -> Placement {
        Placement {
            anchor: Anchor::empty(),
            zone: 0,
            edge: None,
            margin: [0; 4],
            size: (0, 0),
        }
    }
}

pub fn place(p: &Placement, full: Rect, usable: Rect) -> Rect {
    let b = if p.zone == -1 { full } else { usable };
    let [top, right, bottom, left] = p.margin;
    let (mut w, mut h) = p.size;
    let a = p.anchor;
    let x = if w == 0 {
        w = b.width - (left + right);
        b.x + left
    } else if a.contains(Anchor::LEFT) && a.contains(Anchor::RIGHT) {
        b.x + b.width / 2 - w / 2
    } else if a.contains(Anchor::LEFT) {
        b.x + left
    } else if a.contains(Anchor::RIGHT) {
        b.x + b.width - w - right
    } else {
        b.x + b.width / 2 - w / 2
    };
    let y = if h == 0 {
        h = b.height - (top + bottom);
        b.y + top
    } else if a.contains(Anchor::TOP) && a.contains(Anchor::BOTTOM) {
        b.y + b.height / 2 - h / 2
    } else if a.contains(Anchor::TOP) {
        b.y + top
    } else if a.contains(Anchor::BOTTOM) {
        b.y + b.height - h - bottom
    } else {
        b.y + b.height / 2 - h / 2
    };
    Rect::new(x, y, w.max(0), h.max(0))
}

/// The edge a positive zone is on: the one the client named, else the one
/// anchor, or the anchor with both of its neighbours. None otherwise.
pub fn exclusive_edge(p: &Placement) -> Option<Anchor> {
    if p.zone <= 0 {
        return None;
    }
    if p.edge.is_some_and(|e| !e.is_empty()) {
        return p.edge;
    }
    let (t, b, l, r) = (Anchor::TOP, Anchor::BOTTOM, Anchor::LEFT, Anchor::RIGHT);
    [t, b, l, r].into_iter().find(|&e| {
        let across = if e == t || e == b { l | r } else { t | b };
        p.anchor == e || p.anchor == e | across
    })
}

pub fn reserve(p: &Placement, mut usable: Rect) -> Rect {
    let [top, right, bottom, left] = p.margin;
    let z = p.zone;
    match exclusive_edge(p) {
        Some(Anchor::TOP) => {
            usable.y = usable.y.saturating_add(z.saturating_add(top));
            usable.height = usable.height.saturating_sub(z.saturating_add(top));
        }
        Some(Anchor::BOTTOM) => {
            usable.height = usable.height.saturating_sub(z.saturating_add(bottom));
        }
        Some(Anchor::LEFT) => {
            usable.x = usable.x.saturating_add(z.saturating_add(left));
            usable.width = usable.width.saturating_sub(z.saturating_add(left));
        }
        Some(Anchor::RIGHT) => {
            usable.width = usable.width.saturating_sub(z.saturating_add(right));
        }
        _ => {}
    }
    usable.width = usable.width.max(0);
    usable.height = usable.height.max(0);
    usable
}

fn raise_desktop(state: &mut State) {
    let icons = state
        .scene
        .children(state.layers.background)
        .iter()
        .copied()
        .find(|&n| state.scene.data(n) == Some(&SceneOwner::Desktop));
    if let Some(n) = icons {
        state.scene.raise_to_top(n);
    }
}

fn tree_for(state: &State, layer: Layer) -> TreeId {
    match layer {
        Layer::Background => state.layers.background,
        Layer::Bottom => state.layers.bottom,
        Layer::Top => state.layers.top,
        Layer::Overlay => state.layers.overlay,
    }
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layers_shell.state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        // The client may leave the output to us: the one under the pointer.
        let output = output.as_ref().and_then(Output::from_resource).or_else(|| {
            let p = self.pointer_location();
            self.output_at(p.x, p.y)
        });
        let Some(output) = output else {
            log::warn!("layer surface without an output; closing");
            surface.send_close();
            return;
        };
        let tree = self.scene.tree_create(tree_for(self, layer));
        self.scene.set_data(tree, Some(SceneOwner::Layer));
        self.scene
            .surface_create(tree, surface.wl_surface().clone());
        // A wallpaper surface lands in the same tree as the desktop icons and
        // would stack above them.
        if layer == Layer::Background {
            raise_desktop(self);
        }
        log::debug!("layer surface {namespace:?} on layer {layer:?}");
        self.layers_shell.surfaces.push(LayerSurface {
            surface,
            output,
            tree,
            popups: Vec::new(),
            seen: None,
            mapped: false,
        });
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(i) = self
            .layers_shell
            .surfaces
            .iter()
            .position(|l| l.surface == surface)
        else {
            return;
        };
        let l = self.layers_shell.surfaces.remove(i);
        // It redraws no bar, so `popup_gone` steps the bars itself.
        crate::autohideview::popup_gone(self, l.wl());
        self.scene.destroy(l.tree);
        if self.layers_shell.focused.as_ref() == Some(l.wl()) {
            self.layers_shell.focused = None;
            self.focus_next_layer_or_top();
        }
        arrange(self, &l.output);
        self.redraw_all();
    }
}

impl State {
    /// A panel's popups stay inside its output; their offsets are from the panel's origin.
    pub(crate) fn layer_popup_bounds(&self, root: &WlSurface) -> Option<(Rect, (i32, i32))> {
        let l = self.layer_of(root)?;
        let at = self.scene.position(l.tree).unwrap_or_default();
        Some((self.layout_box(Some(&l.output)), (at.x, at.y)))
    }

    fn layer_of(&self, surface: &WlSurface) -> Option<&LayerSurface> {
        self.layers_shell
            .surfaces
            .iter()
            .find(|l| l.wl() == surface)
    }

    fn focus_next_layer_or_top(&mut self) {
        let next = self
            .layers_shell
            .surfaces
            .iter()
            .find(|l| l.mapped && l.exclusive())
            .map(|l| l.wl().clone());
        match next {
            Some(s) => focus_layer(self, &s),
            None => self.focus_top(),
        }
    }
}

pub fn commit(state: &mut State, surface: &WlSurface) {
    if let Some(root) = popup_root(state, surface) {
        sync_popups(state, &root);
        return;
    }
    let Some(i) = state
        .layers_shell
        .surfaces
        .iter()
        .position(|l| l.wl() == surface)
    else {
        return;
    };
    let initial_sent = with_states(surface, |s| {
        s.data_map
            .get::<LayerSurfaceData>()
            .is_some_and(|d| d.lock().is_ok_and(|d| d.initial_configure_sent))
    });
    let now = wants(&state.layers_shell.surfaces[i].surface);
    // The initial commit carries no buffer and asks for the first configure; a
    // later null buffer unmaps.
    let has_buffer =
        with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
    let l = &mut state.layers_shell.surfaces[i];
    let before = l.seen.replace(now);
    let was_mapped = l.mapped;
    l.mapped = has_buffer && initial_sent;
    let mapped = l.mapped;
    let (output, tree, wl) = (l.output.clone(), l.tree, l.wl().clone());
    if before.map(|b| b.layer) != Some(now.layer) {
        let parent = tree_for(state, now.layer);
        state.scene.reparent(tree, parent);
        if now.layer == Layer::Background {
            raise_desktop(state);
        }
    }
    if mapped && !was_mapped {
        if matches!(now.layer, Layer::Top | Layer::Overlay) {
            let zone = now.placement.clamped(state.layout_box(Some(&output))).zone;
            crate::autohideview::popup_mapped(state, &output, &wl, zone);
        }
        if now.keyboard == KeyboardInteractivity::Exclusive {
            focus_layer(state, &wl);
        }
        state.redraw_taskbars();
    } else if !mapped && was_mapped {
        crate::autohideview::popup_gone(state, &wl);
        state.forget_pointer_focus(&wl);
        if state.layers_shell.focused.as_ref() == Some(&wl) {
            state.layers_shell.focused = None;
            state.focus_next_layer_or_top();
        }
        state.redraw_taskbars();
    } else if mapped && before.map(|b| b.keyboard) != Some(now.keyboard) {
        update_keyboard_interactivity(state, &wl, now.keyboard);
    }
    if !initial_sent || before != Some(now) || mapped != was_mapped {
        arrange(state, &output);
    }
    if mapped && !was_mapped {
        // A surface mapping under a resting pointer takes it at once, or
        // a full-screen overlay would get no enter and its first click
        // would land on the window beneath.
        let p = state.pointer_location();
        state.repoint(p, smithay::backend::input::InputTime::now());
    }
}

fn update_keyboard_interactivity(state: &mut State, wl: &WlSurface, mode: KeyboardInteractivity) {
    if mode == KeyboardInteractivity::Exclusive {
        focus_layer(state, wl);
    } else if state.layers_shell.focused.as_ref() == Some(wl) && mode == KeyboardInteractivity::None
    {
        state.layers_shell.focused = None;
        state.focus_next_layer_or_top();
    }
}

fn popup_root(state: &State, surface: &WlSurface) -> Option<WlSurface> {
    let kind = state.popups.find_popup(surface)?;
    let root = smithay::desktop::find_popup_root_surface(&kind).ok()?;
    state.layer_of(&root).map(|l| l.wl().clone())
}

pub fn sync_popups(state: &mut State, root: &WlSurface) {
    let Some(i) = state
        .layers_shell
        .surfaces
        .iter()
        .position(|l| l.wl() == root)
    else {
        return;
    };
    let tree = state.layers_shell.surfaces[i].tree;
    let open: Vec<(PopupKind, Point<i32, Logical>)> =
        PopupManager::popups_for_surface(root).collect();
    let mut old = std::mem::take(&mut state.layers_shell.surfaces[i].popups);
    let mut kept = Vec::new();
    for (popup, offset) in open {
        let surface = popup.wl_surface().clone();
        let node = match old.iter().position(|(s, _)| *s == surface) {
            Some(i) => old.swap_remove(i).1,
            None => state.scene.surface_create(tree, surface.clone()),
        };
        let at = offset - popup.geometry().loc;
        state.scene.set_position(node, at.x, at.y);
        state.scene.raise_to_top(node);
        kept.push((surface, node));
    }
    for (_, node) in old {
        state.scene.destroy(node);
    }
    state.layers_shell.surfaces[i].popups = kept;
}

pub fn popup_destroyed(state: &mut State, root: &WlSurface) {
    if state.layer_of(root).is_some() {
        sync_popups(state, root);
        state.redraw_all();
    }
}

pub fn arrange(state: &mut State, output: &Output) {
    if configure(state, output) {
        state.refit_maximized();
        state.relayout_desktop();
    }
}

/// Configure every layer surface on `output` and record what is left of the
/// output for ordinary windows. Returns whether that area changed.
fn configure(state: &mut State, output: &Output) -> bool {
    let full = state.layout_box(Some(output));
    if full.width <= 0 {
        return false;
    }
    let mut usable = crate::barplace::usable(full, &state.bar_placement(output));
    for layer in [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background] {
        for exclusive in [true, false] {
            for l in state.layers_shell.surfaces.iter() {
                // A surface that has not had its initial commit yet cannot
                // be configured.
                let Some(w) = l.seen else {
                    continue;
                };
                if l.output != *output || w.layer != layer || (w.placement.zone > 0) != exclusive {
                    continue;
                }
                let placement = w.placement.clamped(full);
                let b = place(&placement, full, usable);
                state.scene.set_position(l.tree, b.x, b.y);
                l.surface
                    .with_pending_state(|s| s.size = Some((b.width, b.height).into()));
                let initial_sent = with_states(l.wl(), |s| {
                    s.data_map
                        .get::<LayerSurfaceData>()
                        .is_some_and(|d| d.lock().is_ok_and(|d| d.initial_configure_sent))
                });
                if initial_sent {
                    l.surface.send_pending_configure();
                } else {
                    l.surface.send_configure();
                }
                if l.mapped {
                    usable = reserve(&placement, usable);
                }
            }
        }
    }
    log::debug!("arrange {full:?} -> usable {usable:?}");
    let shell = &mut state.layers_shell;
    let changed = match shell.usable.iter_mut().find(|(o, _, _)| o == output) {
        Some(entry) => {
            let changed = entry.2 != usable || entry.1 != full;
            *entry = (output.clone(), full, usable);
            changed
        }
        None => {
            shell.usable.push((output.clone(), full, usable));
            true
        }
    };
    state.redraw_all();
    changed
}

pub fn outputs_changed(state: &mut State) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    let gone: Vec<WlrLayerSurface> = state
        .layers_shell
        .surfaces
        .iter()
        .filter(|l| !outputs.contains(&l.output))
        .map(|l| l.surface.clone())
        .collect();
    for ls in gone {
        ls.send_close();
        state.layer_destroyed(ls);
    }
    state
        .layers_shell
        .usable
        .retain(|(o, _, _)| outputs.contains(o));
    for o in &outputs {
        configure(state, o);
    }
    state.refit_maximized();
    crate::lockview::refit(state);
}

/// Whether a mapped layer surface currently owns the keyboard outright, in
/// which case focusing a window must not steal it.
pub fn holds_keyboard(state: &State) -> bool {
    state
        .layers_shell
        .focused
        .as_ref()
        .and_then(|f| state.layer_of(f))
        .is_some_and(|l| l.mapped && l.exclusive())
}

pub fn focus_layer(state: &mut State, wl: &WlSurface) {
    state.layers_shell.focused = Some(wl.clone());
    if state.is_locked() {
        return;
    }
    state.enter_keyboard(Some(wl.clone()));
}

pub fn press(state: &mut State, surface: &WlSurface) {
    let root = popup_root(state, surface).unwrap_or_else(|| surface.clone());
    if state
        .layer_of(&root)
        .and_then(|l| l.seen)
        .is_some_and(|w| w.keyboard != KeyboardInteractivity::None)
    {
        focus_layer(state, &root);
    }
}

/// Close the flyout `surface` is part of at once, for a reader's click on what it covers: a
/// mapped Top or Overlay layer surface that holds the keyboard outright. Its client hears
/// `closed`; the scene drops it now. False, closing nothing, for any other surface.
pub fn close_flyout(state: &mut State, surface: &WlSurface) -> bool {
    let root = popup_root(state, surface).unwrap_or_else(|| surface.clone());
    let Some(l) = state.layer_of(&root) else {
        return false;
    };
    let over = l
        .seen
        .is_some_and(|w| matches!(w.layer, Layer::Top | Layer::Overlay));
    if !(l.mapped && l.exclusive() && over) {
        return false;
    }
    let ls = l.surface.clone();
    ls.send_close();
    state.layer_destroyed(ls);
    true
}

pub fn is_mapped_layer(state: &State, surface: &WlSurface) -> bool {
    state.layer_of(surface).is_some_and(|l| l.mapped)
}

impl State {
    fn forget_pointer_focus(&mut self, surface: &WlSurface) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if pointer.current_focus().is_some_and(|f| f == *surface) {
            let p = self.pointer_location();
            self.pointer_motion(None, p, smithay::backend::input::InputTime::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::taskbar::Edge;

    fn bar(anchor: Anchor, zone: i32, size: (i32, i32)) -> Placement {
        Placement {
            anchor,
            zone,
            size,
            ..Placement::default()
        }
    }

    const FULL: Rect = Rect::new(0, 0, 1920, 1080);

    fn after_bar(edge: Edge) -> Rect {
        crate::barplace::usable(FULL, &crate::barplace::place(FULL, edge, 40))
    }

    #[test]
    fn a_top_panel_stretches_and_reserves_its_height() {
        let usable = after_bar(Edge::Bottom);
        let p = bar(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, 30, (0, 30));
        assert_eq!(place(&p, FULL, usable), Rect::new(0, 0, 1920, 30));
        assert_eq!(reserve(&p, usable), Rect::new(0, 30, 1920, 1010));
    }

    #[test]
    fn a_bottom_panel_sits_above_the_taskbar_strip() {
        let usable = after_bar(Edge::Bottom);
        let mut p = bar(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, 24, (0, 24));
        p.margin = [0, 0, 6, 0];
        assert_eq!(place(&p, FULL, usable), Rect::new(0, 1010, 1920, 24));
        assert_eq!(reserve(&p, usable), Rect::new(0, 0, 1920, 1010));
    }

    #[test]
    fn panels_stack_inside_what_a_top_taskbar_leaves() {
        let usable = after_bar(Edge::Top);
        let p = bar(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, 30, (0, 30));
        assert_eq!(
            place(&p, FULL, usable),
            Rect::new(0, 40, 1920, 30),
            "under the bar"
        );
        assert_eq!(reserve(&p, usable), Rect::new(0, 70, 1920, 1010));
        let p = bar(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, 24, (0, 24));
        assert_eq!(place(&p, FULL, usable), Rect::new(0, 1056, 1920, 24));
        assert_eq!(reserve(&p, usable), Rect::new(0, 40, 1920, 1016));
    }

    #[test]
    fn a_zone_of_minus_one_covers_the_whole_output() {
        let usable = Rect::new(0, 30, 1920, 1010);
        let p = bar(Anchor::all(), -1, (0, 0));
        assert_eq!(place(&p, FULL, usable), FULL);
        assert_eq!(reserve(&p, usable), usable);
    }

    #[test]
    fn an_unanchored_surface_is_centred_and_reserves_nothing() {
        let p = bar(Anchor::empty(), 50, (200, 100));
        assert_eq!(place(&p, FULL, FULL), Rect::new(860, 490, 200, 100));
        assert_eq!(exclusive_edge(&p), None);
        assert_eq!(reserve(&p, FULL), FULL);
    }

    #[test]
    fn a_huge_zone_leaves_an_empty_usable_area_not_a_negative_one() {
        let p = bar(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, 5000, (0, 30));
        assert_eq!(reserve(&p, FULL).height, 0);
        assert_eq!(reserve(&p.clamped(FULL), FULL).height, 0);
        let p = bar(Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, 5000, (48, 0));
        assert_eq!(reserve(&p, FULL).width, 0);
    }

    #[test]
    fn an_overflowing_margin_does_not_panic() {
        let mut p = bar(
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            i32::MAX,
            (0, 30),
        );
        p.margin = [1, 0, 0, 0];
        assert_eq!(reserve(&p, FULL).height, 0);
        let mut p = bar(Anchor::TOP | Anchor::LEFT, 0, (i32::MIN, i32::MAX));
        p.margin = [i32::MAX, i32::MIN, i32::MAX, i32::MAX];
        let p = p.clamped(FULL);
        assert_eq!(p.size, (0, 1080));
        let b = place(&p, FULL, FULL);
        assert!(
            b.width >= 0 && b.height >= 0,
            "a rect of no negative size: {b:?}"
        );
    }

    #[test]
    fn the_exclusive_edge_comes_from_the_anchors_or_the_client() {
        let left = bar(Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, 48, (48, 0));
        assert_eq!(exclusive_edge(&left), Some(Anchor::LEFT));
        assert_eq!(reserve(&left, FULL), Rect::new(48, 0, 1872, 1080));
        let mut corner = bar(Anchor::TOP | Anchor::RIGHT, 20, (100, 20));
        assert_eq!(exclusive_edge(&corner), None);
        corner.edge = Some(Anchor::RIGHT);
        assert_eq!(reserve(&corner, FULL), Rect::new(0, 0, 1900, 1080));
    }
}
