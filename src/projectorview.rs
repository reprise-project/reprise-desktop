//! The display switcher on the scene: `reprisekit::controls::projector`'s four tiles centred on the
//! output under the pointer, on the menu layer, opened by the `display-switcher` bind with the
//! current choice lit. Each further press of the bind moves on; letting go of its modifiers after
//! a move, Enter or a click picks (`State::project`). Opened and let go without a move, it stays
//! for the arrows. Escape, a press elsewhere, the lock and a reload close it.

use reprisekit::controls::projector::{self, Event, Key, Projector, ProjectorLayout};
use reprisekit::interfaces;
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::input::BTN_LEFT;
use crate::keys;
use crate::output::upload_scaled;
use crate::raster::Rect;

/// `State::projector`: the open switcher.
#[derive(Default)]
pub struct ProjectorView {
    shown: Option<Shown>,
}

struct Shown {
    output: Output,
    model: Projector,
    /// Logical.
    layout: ProjectorLayout,
    at: (i32, i32),
    /// The bind's modifiers while still held from the press that opened it; 0 once let go.
    mods: u32,
    /// A press of the bind moved the lit tile since it opened.
    moved: bool,
    /// In `layers.menu`, tagged `SceneOwner::Projector`: the glass tree, then the buffer.
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
}

impl ProjectorView {
    pub fn is_open(&self) -> bool {
        self.shown.is_some()
    }
}

/// Where the panel of `size` goes on `area`: its middle.
fn place(area: Rect, size: (i32, i32)) -> (i32, i32) {
    (
        area.x + (area.width - size.0) / 2,
        area.y + (area.height - size.1) / 2,
    )
}

/// The `display-switcher` bind, its modifiers `mods`: open the switcher, or move to the next
/// choice while open.
pub fn bind(state: &mut State, mods: u32) {
    if state.projector.shown.is_some() {
        if let Some(s) = state.projector.shown.as_mut() {
            s.moved = true;
        }
        let e = state
            .projector
            .shown
            .as_mut()
            .map(|s| s.model.key(Key::Next));
        if let Some(e) = e {
            act(state, e);
        }
        return;
    }
    open(state, mods);
}

fn open(state: &mut State, mods: u32) {
    if state.desktop_locked() {
        return;
    }
    state.cancel_interaction();
    state.close_menu();
    crate::snappickerview::hide(state);
    let p = state.pointer_location();
    let Some(output) = state
        .output_at(p.x, p.y)
        .or_else(|| state.console_primary())
    else {
        return;
    };
    let (now, connected) = state.projection();
    let model = Projector::new(now, connected);
    let layout = ProjectorLayout::compute();
    let at = place(
        state.usable_area(Some(&output)),
        (layout.width, layout.height),
    );
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    scene.set_data(tree, Some(SceneOwner::Projector));
    crate::glass::leave_out(scene, tree);
    crate::output::keep_on(scene, tree, &output);
    scene.set_position(tree, at.0, at.1);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    state.projector.shown = Some(Shown {
        output,
        model,
        layout,
        at,
        mods,
        moved: false,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
    });
    redraw(state);
}

pub fn hide(state: &mut State) {
    let Some(s) = state.projector.shown.take() else {
        return;
    };
    state.destroy_redrawn(s.tree);
    crate::access::touch(state);
}

/// The outputs changed: a switcher whose output left closes.
pub fn output_gone(state: &mut State) {
    let outputs = state.scene.outputs();
    if (state.projector.shown.as_ref()).is_some_and(|s| !outputs.contains(&s.output)) {
        hide(state);
    }
}

fn redraw(state: &mut State) {
    let Some(s) = state.projector.shown.as_ref() else {
        return;
    };
    let tree = s.tree;
    let was = state.scene.outputs_of(tree);
    let scale = state.draw_scale_at(Some(&s.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let size = (s.layout.width, s.layout.height);
    let pane =
        glass::wanted(&state.config).then(|| glass::sheet_pane(size, projector::PANEL_CORNER));
    let canvas = interfaces::draw_projector(
        &theme,
        &s.layout.scaled(scale),
        state.text.for_output(theme.scale),
        &s.model,
    );
    let Some(s) = state.projector.shown.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, s.buffer, &canvas, scale);
    s.glass.set(scene, &state.glass, s.glass_tree, pane);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

/// The tile at layout point (`lx`, `ly`), when the switcher is open there.
pub fn hit(state: &State, lx: f64, ly: f64) -> Option<usize> {
    let s = state.projector.shown.as_ref()?;
    let x = (lx - f64::from(s.at.0)).floor() as i32;
    let y = (ly - f64::from(s.at.1)).floor() as i32;
    s.layout.hit(x, y)
}

pub fn hover(state: &mut State, tile: Option<usize>) {
    let e = state.projector.shown.as_mut().map(|s| s.model.hover(tile));
    if let Some(e) = e {
        act(state, e);
    }
}

pub fn press(state: &mut State, tile: Option<usize>, button: u32) {
    if button != BTN_LEFT {
        return;
    }
    let e = state.projector.shown.as_ref().map(|s| s.model.click(tile));
    if let Some(e) = e {
        act(state, e);
    }
}

/// A press the switcher took (`KeyRoute::Projector`), by its first keysym.
pub fn key(state: &mut State, sym: u32) {
    let key = match sym {
        keys::KEY_Left | keys::KEY_KP_Left | keys::KEY_Up | keys::KEY_KP_Up => Key::Prev,
        keys::KEY_Right
        | keys::KEY_KP_Right
        | keys::KEY_Down
        | keys::KEY_KP_Down
        | keys::KEY_Tab => Key::Next,
        keys::KEY_Return | keys::KEY_KP_Enter | keys::KEY_space => Key::Enter,
        keys::KEY_Escape => Key::Escape,
        _ => return,
    };
    let e = state.projector.shown.as_mut().map(|s| s.model.key(key));
    if let Some(e) = e {
        act(state, e);
    }
}

/// After every key: once the bind's modifiers are let go, a switcher moved since it opened picks
/// the lit choice; one never moved stays open for the arrows and Enter.
pub fn modifiers(state: &mut State) {
    let Some(s) = state.projector.shown.as_ref() else {
        return;
    };
    if s.mods == 0 {
        return;
    }
    let held = (state.seat.get_keyboard()).map_or(0, |k| keys::mask(&k.modifier_state()));
    if held & s.mods != 0 {
        return;
    }
    if s.moved {
        let e = Event::Pick(s.model.choice());
        act(state, e);
    } else if let Some(s) = state.projector.shown.as_mut() {
        s.mods = 0;
    }
}

fn act(state: &mut State, e: Event) {
    match e {
        Event::None => {}
        Event::Redraw => redraw(state),
        Event::Close => hide(state),
        Event::Pick(p) => {
            hide(state);
            state.project(p);
        }
    }
}

pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let s = state.projector.shown.as_ref()?;
    let rect = Rect::new(s.at.0, s.at.1, s.layout.width, s.layout.height);
    let mut tree = crate::access::root(Role::Window, "Display switcher", rect);
    let focus = projector::access(&s.layout, &s.model, &mut tree, ROOT);
    tree.set_focus(focus);
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;
    use reprisekit::projection::Projection;

    #[test]
    fn the_panel_sits_in_the_middle_of_the_usable_area() {
        assert_eq!(place(Rect::new(0, 0, 1920, 1050), (400, 120)), (760, 465));
        assert_eq!(
            place(Rect::new(1920, 30, 1280, 994), (400, 120)),
            (2360, 467)
        );
    }

    fn lit(h: &Harness) -> Projection {
        h.state.projector.shown.as_ref().unwrap().model.choice()
    }

    #[test]
    fn the_bind_opens_on_what_shows_and_each_press_moves_on() {
        let mut h = Harness::new();
        h.head("DP-1", 1920, 1080);
        h.head("DP-2", 1920, 1080);
        bind(&mut h.state, keys::MOD_LOGO);
        assert!(h.state.projector.is_open());
        assert_eq!(lit(&h), Projection::Extend);
        bind(&mut h.state, keys::MOD_LOGO);
        assert_eq!(lit(&h), Projection::Duplicate);
        key(&mut h.state, keys::KEY_Escape);
        assert!(!h.state.projector.is_open(), "Escape keeps what runs");
        assert_eq!(h.state.projection().0, Projection::Extend);
    }

    #[test]
    fn letting_go_without_a_move_keeps_it_open_and_enter_picks() {
        let mut h = Harness::new();
        h.head("DP-1", 1920, 1080);
        h.head("DP-2", 1920, 1080);
        bind(&mut h.state, keys::MOD_LOGO);
        modifiers(&mut h.state);
        assert!(h.state.projector.is_open(), "no move: it stays");
        key(&mut h.state, keys::KEY_Right);
        key(&mut h.state, keys::KEY_Return);
        h.dispatch();
        assert!(!h.state.projector.is_open());
        assert_eq!(h.state.projection().0, Projection::Duplicate);
    }

    #[test]
    fn letting_go_after_a_move_picks_the_lit_choice() {
        let mut h = Harness::new();
        h.head("DP-1", 1920, 1080);
        h.head("DP-2", 1920, 1080);
        bind(&mut h.state, keys::MOD_LOGO);
        bind(&mut h.state, keys::MOD_LOGO);
        bind(&mut h.state, keys::MOD_LOGO);
        assert_eq!(lit(&h), Projection::Only(0));
        modifiers(&mut h.state);
        h.dispatch();
        assert!(!h.state.projector.is_open());
        assert_eq!(h.state.projection(), (Projection::Only(0), 2));
    }
}
