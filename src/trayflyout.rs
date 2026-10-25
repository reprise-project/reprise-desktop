//! The hidden icons' flyout on the scene graph: a `reprisekit::trayflyout` panel on the menu layer, opened from the
//! taskbar's ^ cell, listing the tray items whose status is Passive.

use crate::barplace::BarPlacement;
use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::input::BTN_LEFT;
use crate::output::upload_scaled;
use crate::raster::Rect;
use reprisekit::icon::ICON_SIZE;
use reprisekit::taskbar::Edge;
use reprisekit::trayflyout::{self, FlyoutLayout};
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use std::sync::Arc;

pub struct FlyoutView {
    /// In `layers.menu`, tagged `SceneOwner::TrayFlyout`: the glass tree,
    /// then the buffer.
    tree: TreeId,
    glass_tree: TreeId,
    /// The glass under the frame; empty unless the look has glass drawn
    /// translucent.
    glass: Backdrop,
    buffer: BufferId,
    output: Output,
    layout: FlyoutLayout,
    at: (i32, i32),
    ids: Vec<String>,
    wheel: Option<(usize, bool, i32)>,
    link_hot: bool,
}

/// Where a flyout `size` big goes from the ^ `cell` (layout coordinates)
/// on bar `p`: centred on it along the bar, on the screen side of the
/// bar, clamped inside `usable`.
pub fn place(cell: Rect, size: (i32, i32), p: &BarPlacement, usable: Rect) -> (i32, i32) {
    let (w, h) = size;
    let anchor = if p.edge.is_side() {
        Rect::new(cell.x, cell.y + cell.height / 2 - h / 2, cell.width, h)
    } else {
        Rect::new(cell.x + cell.width / 2 - w / 2, cell.y, w, cell.height)
    };
    crate::barplace::bar_popup(p, anchor, (w, h), usable)
}

fn view(state: &mut State) -> Option<&mut FlyoutView> {
    state.tray.as_mut()?.flyout.as_mut()
}

pub fn open_on(state: &State) -> Option<Output> {
    Some(state.tray.as_ref()?.flyout.as_ref()?.output.clone())
}

/// A press on bar `bar`'s ^ cell: a left click opens the flyout unless it
/// was open from this same ^ before the press (`was_on`, its output then),
/// which closed it. Open on another output, the press moves it here.
pub fn chevron_press(state: &mut State, bar: TreeId, button: u32, was_on: Option<Output>) {
    let Some(output) = state.bar(bar).map(|b| b.output.clone()) else {
        return;
    };
    if button == BTN_LEFT && was_on.as_ref() != Some(&output) {
        open(state, output);
    }
}

fn open(state: &mut State, output: Output) {
    close(state);
    if !crate::tray::chevron_shown(state) {
        return;
    }
    let bar = bar_on(state, &output);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    scene.set_data(tree, Some(SceneOwner::TrayFlyout));
    crate::glass::leave_out(scene, tree);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    let v = FlyoutView {
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        output,
        layout: FlyoutLayout::compute(&state.config.theme, 0),
        at: (0, 0),
        ids: Vec::new(),
        wheel: None,
        link_hot: false,
    };
    let Some(tray) = state.tray.as_mut() else {
        state.scene.destroy(tree);
        return;
    };
    tray.flyout = Some(v);
    // The ^ draws pressed while the flyout is open; its bar's redraw
    // lists the items (`bar_redrawn`).
    match bar {
        Some(bar) => state.redraw_taskbar(bar),
        None => refresh(state),
    }
}

fn bar_on(state: &State, output: &Output) -> Option<TreeId> {
    state
        .bars
        .iter()
        .find(|b| b.output == *output)
        .map(|b| b.tree)
}

pub fn close(state: &mut State) {
    let Some(v) = state.tray.as_mut().and_then(|t| t.flyout.take()) else {
        return;
    };
    state.destroy_redrawn(v.tree);
    if let Some(bar) = bar_on(state, &v.output) {
        state.redraw_taskbar(bar);
    }
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// Bars were drawn again (`output`'s, or every one for None): the ^ may
/// have moved or gone, the hidden items, the look or the scale changed.
/// When the open flyout is on such a bar, list the items again from the
/// ^ as now drawn, or close when there is no ^ any more.
pub fn bar_redrawn(state: &mut State, output: Option<&Output>) {
    let Some(on) = open_on(state) else { return };
    if output.is_none_or(|o| *o == on) {
        refresh(state);
    }
}

/// List the items again from the ^ cell as it is now drawn, or close
/// when there is no ^ cell any more.
fn refresh(state: &mut State) {
    let Some(output) = open_on(state) else {
        return;
    };
    let chevron = state
        .bars
        .iter()
        .find(|b| b.output == output)
        .and_then(|b| b.layout.chevron);
    let (true, Some(cell)) = (crate::tray::chevron_shown(state), chevron) else {
        close(state);
        return;
    };
    let ids: Vec<String> = state
        .tray
        .as_ref()
        .map(|t| t.hidden.iter().map(|i| i.id.clone()).collect())
        .unwrap_or_default();
    let layout = FlyoutLayout::compute(&state.config.theme, ids.len());
    let p = state.bar_placement(&output);
    let cell = crate::tray::bar_cell(p.shown, cell);
    let usable = state.usable_area(Some(&output));
    let at = place(cell, (layout.width, layout.height), &p, usable);
    let Some(v) = view(state) else { return };
    v.layout = layout;
    v.at = at;
    v.ids = ids;
    redraw(state);
}

/// The open flyout's tree: a button per hidden item, named by its
/// tooltip, else its title.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let tray = state.tray.as_ref()?;
    let v = tray.flyout.as_ref()?;
    let rect = Rect::new(v.at.0, v.at.1, v.layout.width, v.layout.height);
    let mut tree = crate::access::root(Role::Window, "Hidden icons", rect);
    let names: Vec<String> = tray
        .hidden
        .iter()
        .map(crate::tray::TrayItem::name)
        .collect();
    trayflyout::access(&v.layout, &names, &mut tree, ROOT);
    Some(tree)
}

fn redraw(state: &mut State) {
    let Some(output) = open_on(state) else { return };
    let scale = state.draw_scale_at(Some(&output)).max(1);
    let items: Vec<crate::tray::TrayItem> = state
        .tray
        .as_ref()
        .map(|t| t.hidden.clone())
        .unwrap_or_default();
    let px = ICON_SIZE * scale;
    let icons: Vec<_> = items
        .iter()
        .map(|it| crate::tray::item_icon(state, it, px).map(Arc::unwrap_or_clone))
        .collect();
    let theme = state.config.theme.scaled(scale);
    let Some(size) = view(state).map(|v| (v.layout.width, v.layout.height)) else {
        return;
    };
    let pane = glass::flyout_glass(&state.config, size);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    let Some(v) = tray.flyout.as_mut() else {
        return;
    };
    let canvas = trayflyout::draw(
        &theme,
        state.text.for_output(scale),
        &v.layout.scaled(scale),
        &icons,
        v.link_hot,
    );
    let scene = &mut state.scene;
    let tree = v.tree;
    let was = scene.outputs_of(tree);
    upload_scaled(scene, v.buffer, &canvas, scale);
    glass::place_flyout(scene, v.buffer, &state.config.theme, size);
    scene.set_position(tree, v.at.0, v.at.1);
    v.glass.set(scene, &state.glass, v.glass_tree, pane);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

pub fn hit(state: &State, lx: f64, ly: f64) -> Option<usize> {
    let v = state.tray.as_ref()?.flyout.as_ref()?;
    let x = (lx - f64::from(v.at.0)).floor() as i32;
    let y = (ly - f64::from(v.at.1)).floor() as i32;
    v.layout.hit(x, y)
}

/// The pointer moved, onto flyout cell `cell` (`None`: off the flyout or
/// between its cells): the Customize... link, one past the icons, draws
/// underlined while the pointer rests on it.
pub fn hover(state: &mut State, cell: Option<usize>) {
    let Some(v) = view(state) else { return };
    let hot = v.layout.link.is_some() && cell == Some(v.ids.len());
    if std::mem::replace(&mut v.link_hot, hot) != hot {
        redraw(state);
    }
}

/// A press on the flyout: on an icon it closes the flyout and goes to the
/// item as a press on its bar icon would; on the panel around the icons it
/// does nothing.
pub fn press(state: &mut State, cell: Option<usize>, button: u32) {
    let Some(v) = view(state) else { return };
    if cell == Some(v.ids.len()) && v.layout.link.is_some() {
        if button == BTN_LEFT {
            crate::tray::customize(state);
        }
        return;
    }
    let Some((id, r)) = cell.and_then(|i| Some((v.ids.get(i)?.clone(), *v.layout.cells.get(i)?)))
    else {
        return;
    };
    let output = v.output.clone();
    let cell = Rect::new(v.at.0 + r.x, v.at.1 + r.y, r.width, r.height);
    let icon = Rect::new(
        cell.x + (r.width - ICON_SIZE) / 2,
        cell.y + (r.height - ICON_SIZE) / 2,
        ICON_SIZE,
        ICON_SIZE,
    );
    let at = click_point(cell, state.bar_placement(&output).edge);
    close(state);
    crate::notifycenter::close(state);
    crate::tray::click_item(state, output, id, icon, at, button);
}

/// Where a click on flyout cell `cell` (layout coordinates) reaches its
/// item: on the cell's edge facing the screen, level with its middle, as a
/// click on the bar reaches it on the bar's.
fn click_point(cell: Rect, edge: Edge) -> (i32, i32) {
    let (cx, cy) = (cell.x + cell.width / 2, cell.y + cell.height / 2);
    match edge {
        Edge::Bottom => (cx, cell.y),
        Edge::Top => (cx, cell.bottom()),
        Edge::Left => (cell.right(), cy),
        Edge::Right => (cell.x, cy),
    }
}

/// A wheel event over `cell`: whole notches go to its item, the sub-notch
/// remainder stays for the next event over the same cell and axis.
pub fn scroll(state: &mut State, cell: usize, horizontal: bool, discrete: i32, continuous: f64) {
    let Some(v) = view(state) else { return };
    let carried = match v.wheel {
        Some((c, h, r)) if c == cell && h == horizontal => r,
        _ => 0,
    };
    let (steps, residual) = crate::tray::wheel_steps(discrete, continuous, carried);
    v.wheel = Some((cell, horizontal, residual));
    let Some(id) = v.ids.get(cell).cloned() else {
        return;
    };
    crate::tray::scroll_item(state, id, steps, horizontal);
}

pub fn output_gone(state: &mut State) {
    if open_on(state).is_some_and(|o| !state.scene.outputs().contains(&o)) {
        close(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flyout_click_reaches_the_item_on_the_cells_screen_side_edge() {
        let cell = Rect::new(1700, 990, 24, 24);
        assert_eq!(click_point(cell, Edge::Bottom), (1712, 990), "its top");
        assert_eq!(click_point(cell, Edge::Top), (1712, 1014), "its foot");
        assert_eq!(
            click_point(cell, Edge::Left),
            (1724, 1002),
            "its right side"
        );
        assert_eq!(
            click_point(cell, Edge::Right),
            (1700, 1002),
            "its left side"
        );
    }

    #[test]
    fn the_flyout_sits_centred_on_the_screen_side_of_the_chevron() {
        use crate::barplace;
        let screen = Rect::new(0, 0, 1920, 1080);
        let bottom = barplace::place(screen, Edge::Bottom, 40);
        let usable = barplace::usable(screen, &bottom);
        let cell = Rect::new(1700, 1052, 16, 16);
        assert_eq!(place(cell, (104, 56), &bottom, usable), (1656, 984));
        let cell = Rect::new(1900, 1052, 16, 16);
        assert_eq!(place(cell, (104, 56), &bottom, usable), (1816, 984));
        assert_eq!(place(cell, (104, 2000), &bottom, usable).1, 0);
        let top = barplace::place(screen, Edge::Top, 40);
        let usable = barplace::usable(screen, &top);
        let cell = Rect::new(1700, 12, 16, 16);
        assert_eq!(place(cell, (104, 56), &top, usable), (1656, 40));
        // Beside a left bar it stands off the bar's right edge, centred
        // down the ^.
        let left = barplace::place(screen, Edge::Left, 60);
        let usable = barplace::usable(screen, &left);
        let cell = Rect::new(2, 900, 16, 16);
        assert_eq!(place(cell, (104, 56), &left, usable), (60, 880));
    }

    #[test]
    fn a_shrinking_flyout_asks_the_output_its_old_shadow_covered() {
        use crate::tray::{TrayEvent, TrayItem};
        use std::os::fd::AsRawFd;
        let config = crate::config::Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        let mut h = crate::harness::Harness::with_config(config);
        let top = h.output("TEST-1", 1024, 768);
        let below = h.output("TEST-2", 1024, 768);
        below.change_current_state(None, None, None, Some((0, 768).into()));
        let (tx, wake, _heard) = crate::tray::headless(&mut h.state);
        for id in ["a", "b", "c", "d", "e"] {
            let item = TrayItem {
                id: id.into(),
                key: id.into(),
                service: id.into(),
                path: "/StatusNotifierItem".into(),
                title: id.into(),
                tooltip: String::new(),
                icon_name: None,
                icon_theme_path: None,
                pixmaps: Arc::from([]),
                named: Arc::from([]),
                menu_path: None,
                passive: true,
                pid: None,
            };
            tx.send(TrayEvent::Added(item)).expect("send");
        }
        // SAFETY: `wake` is an open eventfd and the buffer a live 8-byte local.
        let wrote = unsafe { libc::write(wake.as_raw_fd(), (&1u64 as *const u64).cast(), 8) };
        assert_eq!(wrote, 8);
        h.dispatch();
        open(&mut h.state, top);
        let theme = h.state.config.theme.clone();
        let two_rows = FlyoutLayout::compute(&theme, 5);
        let at = (400, 768 - two_rows.height);
        let v = view(&mut h.state).expect("the flyout is open");
        (v.layout, v.at) = (two_rows, at);
        redraw(&mut h.state);
        let v = view(&mut h.state).expect("the flyout is open");
        let tree = v.tree;
        assert_eq!(
            h.state.scene.outputs_of(tree).len(),
            2,
            "the flyout's shadow below its bottom edge reaches TEST-2"
        );

        let v = view(&mut h.state).expect("the flyout is open");
        v.layout = FlyoutLayout::compute(&theme, 1);
        crate::backend::take_asks();
        redraw(&mut h.state);
        let asked = crate::backend::take_asks();
        assert_eq!(
            h.state.scene.outputs_of(tree).len(),
            1,
            "one row shorter, the flyout and its shadow stay on TEST-1"
        );
        assert!(
            asked.iter().any(|(o, _)| o == "TEST-2"),
            "TEST-2 is asked to take the old shadow away: {asked:?}"
        );
    }
}
