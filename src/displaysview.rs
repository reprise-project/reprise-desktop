//! The displays cell's `controls::displaypanel` on the menu layer. Closed by a second click on the
//! cell, Escape, a press elsewhere, the lock, a reload, and its output or the cell going away.

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::{self, Backdrop};
use crate::input::Interaction;
use crate::keys;
use crate::media::Value;
use crate::output::upload_scaled;
use crate::raster::Rect;
use reprisekit::access::accesskit::Role;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::displaypanel::{self, DisplayPanel, Event, Hit, Key, Layout, Row};
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;

/// `State::displays_view`: the open panel.
#[derive(Default)]
pub struct DisplaysViewState {
    view: Option<PanelView>,
}

struct PanelView {
    model: DisplayPanel,
    /// In `layers.menu`, tagged `SceneOwner::Displays`: the glass tree, then the buffer.
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// Top-left in layout coordinates.
    at: (i32, i32),
    /// The cell it opened from, in layout coordinates.
    cell: Rect,
    output: Output,
    /// Each row's display.
    connectors: Vec<String>,
    /// The wheel's carry for `displays::wheel_steps`.
    wheel: (i32, f64),
}

pub fn is_open(state: &State) -> bool {
    state.displays_view.view.is_some()
}

pub fn open_on(state: &State) -> Option<Output> {
    state.displays_view.view.as_ref().map(|v| v.output.clone())
}

/// A left click on the displays cell at `cell` on `output`'s taskbar: open the panel, or close it.
pub fn toggle(state: &mut State, output: &Output, cell: Rect) {
    if is_open(state) {
        close(state);
    } else {
        open(state, output, cell);
    }
}

/// Each display's row and connector, in the display list's order.
fn rows(state: &State) -> (Vec<Row>, Vec<String>) {
    let rows = (state.displays.lines(Value::Brightness).lines)
        .into_iter()
        .map(|l| Row {
            label: l.label,
            icon: l.icon,
            level: l.level,
        })
        .collect();
    let connectors = state
        .displays
        .list
        .iter()
        .map(|d| d.connector.clone())
        .collect();
    (rows, connectors)
}

fn open(state: &mut State, output: &Output, cell: Rect) {
    if state.is_locked() {
        return;
    }
    crate::notifycenter::close(state);
    crate::displays::reread(state);
    let (rows, connectors) = rows(state);
    let model = DisplayPanel::new(rows, crate::nightlight::panel_night(&state.night));
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    glass::leave_out(scene, tree);
    scene.set_data(tree, Some(SceneOwner::Displays));
    // Before the buffer, so it lies beneath it.
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    state.displays_view.view = Some(PanelView {
        model,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        at: (0, 0),
        cell,
        output: output.clone(),
        connectors,
        wheel: (0, 0.0),
    });
    place(state);
    redraw(state);
}

pub fn close(state: &mut State) {
    let Some(v) = state.displays_view.view.take() else {
        return;
    };
    state.scene.destroy(v.tree);
    state.redraw_all();
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// The levels or night light changed: the panel follows them, or closes with the cell.
pub fn refresh(state: &mut State) {
    if !is_open(state) {
        return;
    }
    if !crate::displays::cell_shown(state) {
        close(state);
        return;
    }
    let (rows, connectors) = rows(state);
    let night = crate::nightlight::panel_night(&state.night);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    v.model.set_rows(rows);
    v.model.set_night(night);
    v.connectors = connectors;
    place(state);
    redraw(state);
}

fn layout_of(state: &State, model: &DisplayPanel) -> Layout {
    let theme = &state.config.theme;
    displaypanel::layout(&theme.controls(), model, &state.text, theme.fonts.size_px)
}

/// Put the panel against the cell; its size changes as rows come and go.
fn place(state: &mut State) {
    let Some(v) = state.displays_view.view.as_ref() else {
        return;
    };
    let size = layout_of(state, &v.model).size;
    let bounds = state.usable_area(Some(&v.output));
    let at = crate::barplace::bar_popup(&state.bar_placement(&v.output), v.cell, size, bounds);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    v.at = at;
    state.scene.set_position(v.tree, at.0, at.1);
}

fn redraw(state: &mut State) {
    let Some(v) = state.displays_view.view.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&v.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let canvas = displaypanel::draw(&theme, state.text.for_output(theme.scale), &v.model);
    let size = layout_of(state, &v.model).size;
    let pane = glass::flyout_glass(&state.config, size);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, v.buffer, &canvas, scale);
    glass::place_flyout(scene, v.buffer, &state.config.theme, size);
    v.glass.set(scene, &state.glass, v.glass_tree, pane);
    state.redraw_all();
    crate::access::touch(state);
}

/// Carry out what the model asked for after an input.
fn act(state: &mut State, event: Event) {
    let connector = |state: &State, row: usize| {
        let v = state.displays_view.view.as_ref()?;
        v.connectors.get(row).cloned()
    };
    match event {
        Event::None => {}
        Event::Redraw => redraw(state),
        Event::Set { row, level } => {
            if let Some(c) = connector(state, row) {
                crate::displays::set(state, &c, Value::Brightness, level);
            }
        }
        Event::Step { row, steps } => {
            if let Some(c) = connector(state, row) {
                crate::displays::step(state, Some(&c), Value::Brightness, steps);
            }
        }
        // The model ticked the box at once; `refresh` puts back what night light really did.
        Event::Night(on) => {
            crate::nightlight::set_paused(state, !on);
            refresh(state);
        }
        Event::Settings => {
            close(state);
            crate::nightlight::open_settings(state);
        }
        Event::Close => close(state),
    }
}

/// Layout point (`lx`, `ly`) on the open panel, in its own coordinates.
fn local(v: &PanelView, lx: f64, ly: f64) -> (i32, i32) {
    (lx.floor() as i32 - v.at.0, ly.floor() as i32 - v.at.1)
}

/// Where layout point (`lx`, `ly`) lands on the open panel.
pub fn hit(state: &State, lx: f64, ly: f64) -> Hit {
    let Some(v) = state.displays_view.view.as_ref() else {
        return Hit::Panel;
    };
    let (x, y) = local(v, lx, ly);
    displaypanel::hit(&layout_of(state, &v.model), x, y)
}

/// The pointer moved onto `hit` on the panel, or off it (None).
pub fn hover(state: &mut State, hit: Option<Hit>) {
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let hot = hit.unwrap_or(Hit::Panel);
    if std::mem::replace(&mut v.model.hot, hot) != hot {
        redraw(state);
    }
}

/// A left press on the panel; a slider it takes hold of follows the pointer (`drag`) until the
/// release.
pub fn press(state: &mut State, lx: f64, ly: f64) {
    let Some(v) = state.displays_view.view.as_ref() else {
        return;
    };
    let lay = layout_of(state, &v.model);
    let (x, y) = local(v, lx, ly);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let event = v.model.press(&lay, x, y);
    if v.model.held().is_some() {
        state.interaction = Interaction::DisplaysSlide;
    }
    act(state, event);
}

/// The pointer at layout `lx` while a slider is held.
pub fn drag(state: &mut State, lx: f64) {
    let Some(v) = state.displays_view.view.as_ref() else {
        return;
    };
    let lay = layout_of(state, &v.model);
    let (x, _) = local(v, lx, 0.0);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let event = v.model.drag(&lay, x);
    act(state, event);
}

pub fn release(state: &mut State) {
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    v.model.release();
    redraw(state);
}

/// A wheel event at layout (`lx`, `ly`) over the panel: the display whose row is there steps.
pub fn wheel(state: &mut State, lx: f64, ly: f64, discrete: i32, continuous: f64) {
    let Some(v) = state.displays_view.view.as_ref() else {
        return;
    };
    let lay = layout_of(state, &v.model);
    let (x, y) = local(v, lx, ly);
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let steps = crate::displays::wheel_steps(discrete, continuous, &mut v.wheel);
    let event = v.model.wheel(&lay, x, y, steps);
    act(state, event);
}

pub fn key(state: &mut State, k: &DialogKey) {
    let Some(key) = key_of(k) else {
        return;
    };
    let Some(v) = state.displays_view.view.as_mut() else {
        return;
    };
    let event = v.model.key(key);
    act(state, event);
}

fn key_of(k: &DialogKey) -> Option<Key> {
    let shift = k.mods & keys::MOD_SHIFT != 0;
    k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_ISO_Left_Tab => Some(Key::BackTab),
        keys::KEY_Tab if shift => Some(Key::BackTab),
        keys::KEY_Tab => Some(Key::Tab),
        keys::KEY_Left | keys::KEY_KP_Left => Some(Key::Left),
        keys::KEY_Right | keys::KEY_KP_Right => Some(Key::Right),
        keys::KEY_Up | keys::KEY_KP_Up => Some(Key::Up),
        keys::KEY_Down | keys::KEY_KP_Down => Some(Key::Down),
        keys::KEY_Page_Up | keys::KEY_KP_Page_Up => Some(Key::PageUp),
        keys::KEY_Page_Down | keys::KEY_KP_Page_Down => Some(Key::PageDown),
        keys::KEY_Home | keys::KEY_KP_Home => Some(Key::Home),
        keys::KEY_End | keys::KEY_KP_End => Some(Key::End),
        keys::KEY_space => Some(Key::Space),
        keys::KEY_Return | keys::KEY_KP_Enter => Some(Key::Enter),
        keys::KEY_Escape => Some(Key::Escape),
        _ => None,
    })
}

/// The panel's tree for a screen reader, the focused control focused.
pub fn access(state: &State) -> Option<Tree> {
    let v = state.displays_view.view.as_ref()?;
    Some(panel_tree(&v.model, &layout_of(state, &v.model), v.at))
}

fn panel_tree(model: &DisplayPanel, lay: &Layout, at: (i32, i32)) -> Tree {
    let rect = Rect::new(at.0, at.1, lay.size.0, lay.size.1);
    let mut tree = crate::access::root(Role::Window, "Displays", rect);
    let focus = displaypanel::access(model, lay, &mut tree, ROOT);
    tree.set_focus(focus);
    tree
}

/// The layout changed: a panel whose output went away goes with it.
pub fn output_gone(state: &mut State) {
    let gone = (state.displays_view.view.as_ref())
        .is_some_and(|v| !state.scene.outputs().contains(&v.output));
    if gone {
        close(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::access::accesskit::Affine;
    use reprisekit::art::icons;
    use reprisekit::controls::displaypanel::Night;
    use reprisekit::controls::osd::Level;
    use reprisekit::metrics::ControlMetrics;
    use reprisekit::text::TextRenderer;

    fn press(sym: u32, mods: u32) -> Option<Key> {
        key_of(&DialogKey {
            syms: vec![sym],
            mods,
            edit: None,
        })
    }

    #[test]
    fn shift_tab_walks_back_and_the_keypad_steps() {
        assert_eq!(press(keys::KEY_ISO_Left_Tab, 0), Some(Key::BackTab));
        assert_eq!(press(keys::KEY_Tab, keys::MOD_SHIFT), Some(Key::BackTab));
        assert_eq!(press(keys::KEY_Tab, 0), Some(Key::Tab));
        assert_eq!(press(keys::KEY_KP_Right, 0), Some(Key::Right));
        assert_eq!(press(keys::KEY_KP_Page_Up, 0), Some(Key::PageUp));
        assert_eq!(press(keys::KEY_a, 0), None);
    }

    #[test]
    fn the_panel_tree_sits_at_the_panel_and_focuses_the_first_slider() {
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let row = |label: &str, level| Row {
            label: label.into(),
            icon: &icons::MONITOR,
            level: Level::At(level),
        };
        let night = Night {
            enabled: true,
            on: true,
            state: String::new(),
        };
        let model = DisplayPanel::new(
            vec![row("Built-in display", 60), row("DELL U2720Q", 40)],
            night,
        );
        let lay = displaypanel::layout(&ControlMetrics::CLASSIC, &model, &text, 11);
        let t = panel_tree(&model, &lay, (1500, 700));
        let root = t.get(ROOT).expect("root");
        assert_eq!(root.label(), Some("Displays"));
        assert_eq!(
            root.transform().map(|a| a.as_coeffs()),
            Some(Affine::translate((1500.0, 700.0)).as_coeffs())
        );
        let focused = t.get(t.focus()).expect("the focused node");
        assert_eq!(focused.role(), Role::Slider);
        assert_eq!(focused.label(), Some("Built-in display"));
    }
}
