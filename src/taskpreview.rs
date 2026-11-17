//! The task button's preview panel on the scene graph. With `[previews] taskbar` on, resting the
//! pointer on a task button for the tooltip delay opens a `reprisekit::controls::taskpreview` panel on
//! the menu layer, its edge on the taskbar's on the screen side, with each window behind the button
//! shown live (`livepreview`). It replaces the button's text tip. The panel stays while the pointer is
//! on the button or panel, moves at once to another task button the pointer reaches, and goes when the
//! pointer leaves both or anything is pressed. The hovered cell shows a close button: a left click on
//! it closes that window, elsewhere on a cell it activates the window. The popup is laid out and drawn
//! in the look's own way (`interfaces::task_preview_grid`, `interfaces::draw_task_preview`); under
//! glass its canvas reaches over a shadow round the panel and the panel shows the blurred backdrop
//! (`glass::Backdrop`). Resting on a cell peeks at its window (`peek`); the peek holds across the gaps
//! between cells and ends when the pointer leaves the panel. The lock hides it, and none opens while locked.

use std::{sync::Arc, time::Duration};

use reprisekit::{
    controls::taskpreview::{self, Cell, TaskPreviewLayout},
    interfaces,
    taskbar::Edge,
};
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::Rectangle,
};

use crate::{
    barplace::BarPlacement,
    compositor::{SceneOwner, State},
    glass::{self, Backdrop, Pane, PaneShape},
    livepreview::ThumbId,
    output::upload_scaled,
    peek::{self, PeekTarget},
    raster::{Canvas, Rect},
    taskbar::{TaskKey, TaskbarHit},
    window::WindowId,
};

const SHOW_MS: u64 = reprisekit::controls::tooltip::SHOW_MS as u64;
const SMALL_ICON: i32 = 16;
const WELL_ICON: i32 = 32;

#[derive(Default)]
pub struct TaskPreviewView {
    timer: Option<RegistrationToken>,
    /// The task button waiting for its panel: its bar and index.
    pending: Option<(TreeId, usize)>,
    shown: Option<Panel>,
    peeking: bool,
    /// The windows of the `pending` button a live picture showed when it armed (a closing overview
    /// or 3D switcher): they stay awake (`State::mark_suspended`) until the panel shows them or the
    /// wait ends.
    awake: Vec<WindowId>,
    /// The pictures `hold` took out of the panel and the windows it kept awake, until `release`.
    held: Vec<ThumbId>,
    held_awake: Vec<WindowId>,
    /// The task button last pressed with the pointer, by its bar and key so it is followed through any
    /// change to the row: its panel waits until the pointer leaves it.
    pressed: Option<(TreeId, TaskKey<WindowId>)>,
}

struct Panel {
    /// The bar (`Bar::tree`) and button it belongs to, and the bar's output.
    bar: TreeId,
    button: usize,
    output: Output,
    windows: Vec<WindowId>,
    layout: TaskPreviewLayout,
    /// In `layers.menu`, tagged `SceneOwner::TaskPreview`, at the panel's top-left: the glass (when drawn translucent), the canvas, then the pictures.
    tree: TreeId,
    buffer: BufferId,
    thumbs: Vec<Option<ThumbId>>,
    hot: Option<usize>,
}

/// The pointer is over `hit` on bar `bar`. A task button starts the delay, or takes over a panel at once, except the one just pressed (`pressed`); anything else on the bar takes it down.
pub fn hover(state: &mut State, bar: TreeId, hit: TaskbarHit) {
    stop_peek(state);
    let target = match hit {
        TaskbarHit::Task(i) if state.config.previews.taskbar => (bar, i),
        _ => {
            leave(state);
            return;
        }
    };
    let pressed = state.task_preview.pressed.filter(|&(b, _)| b == bar);
    let slots = state
        .bar(bar)
        .map(|b| b.slots.as_slice())
        .unwrap_or_default();
    if pressed.is_some_and(|(_, key)| crate::taskbar::find_task(slots, &key) == Some(target.1)) {
        return;
    }
    state.task_preview.pressed = None;
    let v = &state.task_preview;
    let shown = v.shown.as_ref().map(|p| (p.bar, p.button));
    if shown == Some(target) || v.pending == Some(target) {
        return;
    }
    if shown.is_some() {
        show(state, target.0, target.1);
        return;
    }
    let slot = state.bar(target.0).and_then(|b| b.slots.get(target.1));
    let awake = (slot
        .map(|s| s.windows.as_slice())
        .unwrap_or_default()
        .iter())
    .copied()
    .filter(|&w| state.live_previews.shows(w))
    .collect();
    let was = std::mem::replace(&mut state.task_preview.awake, awake);
    state.task_preview.pending = Some(target);
    disarm(state);
    resuspend(state, was);
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(SHOW_MS)),
        |_, _, state| {
            state.task_preview.timer = None;
            let awake = std::mem::take(&mut state.task_preview.awake);
            if let Some((bar, i)) = state.task_preview.pending.take() {
                show(state, bar, i);
            }
            resuspend(state, awake);
            TimeoutAction::Drop
        },
    );
    match token {
        Ok(t) => state.task_preview.timer = Some(t),
        Err(e) => log::warn!("task preview timer: {}", e.error),
    }
}

pub fn hover_panel(state: &mut State, cell: Option<usize>) {
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    let window = cell.and_then(|i| p.windows.get(i).copied());
    if p.hot != cell {
        p.hot = cell;
        redraw(state);
    }
    match window {
        Some(w) => {
            state.task_preview.peeking = true;
            peek::hover(state, PeekTarget::Window(w));
        }
        None if !state.peek.active() => stop_peek(state),
        None => {}
    }
}

fn stop_peek(state: &mut State) {
    if std::mem::take(&mut state.task_preview.peeking) {
        peek::leave(state);
    }
}

fn disarm(state: &mut State) {
    if let Some(t) = state.task_preview.timer.take() {
        state.handle.remove(t);
    }
}

pub fn hit(state: &State, lx: f64, ly: f64) -> Option<usize> {
    let p = state.task_preview.shown.as_ref()?;
    let r = p.layout.rect;
    taskpreview::hit(&p.layout, lx.floor() as i32 - r.x, ly.floor() as i32 - r.y)
}

/// A left press on the panel, on `cell`: its close button closes its window, elsewhere activates it; the panel goes either way.
pub fn clicked(state: &mut State, cell: Option<usize>) {
    let Some((i, p)) = cell.zip(state.task_preview.shown.as_ref()) else {
        hide(state);
        return;
    };
    let Some(w) = p.windows.get(i).copied() else {
        hide(state);
        return;
    };
    let r = p.layout.rect;
    let at = state.pointer_location();
    let (cx, cy) = (at.x.floor() as i32, at.y.floor() as i32);
    let on_close = taskpreview::close_hit(&p.layout, p.hot, cx - r.x, cy - r.y) == Some(i);
    hold(state);
    hide(state);
    if on_close {
        state.close(w);
    } else if state.task_windows().contains(&w) {
        state.bring_up(w);
    }
    release(state);
}

/// Take the panel's pictures, and the windows a pending panel keeps awake, out of it before
/// something that takes it down and may bring one of its windows up; `release` drops them after,
/// so a minimized window is never suspended in between.
pub fn hold(state: &mut State) {
    release(state);
    let v = &mut state.task_preview;
    v.held_awake = std::mem::take(&mut v.awake);
    if let Some(p) = v.shown.as_mut() {
        v.held = p.thumbs.drain(..).flatten().collect();
    }
}

pub fn release(state: &mut State) {
    for id in std::mem::take(&mut state.task_preview.held) {
        state.thumb_drop(id);
    }
    let awake = std::mem::take(&mut state.task_preview.held_awake);
    resuspend(state, awake);
}

/// `windows` stopped waiting for the panel: each nothing else shows is suspended again.
fn resuspend(state: &mut State, windows: Vec<WindowId>) {
    for w in windows {
        if state.mark_suspended(w) {
            state.send_configure(w);
        }
    }
}

/// A pointer press on task button `i` of bar `bar`: its panel stays down until the pointer has
/// left the button, so a pointer resting where it clicked brings none back.
pub fn pressed(state: &mut State, bar: TreeId, i: usize) {
    let key = state
        .bar(bar)
        .and_then(|b| crate::taskbar::task_key(&b.slots, i));
    state.task_preview.pressed = key.map(|k| (bar, k));
}

/// The pointer is off the task buttons and the panel: as `hide`, and the pressed button shows its
/// panel again on the next rest.
pub fn leave(state: &mut State) {
    state.task_preview.pressed = None;
    hide(state);
}

/// Take the panel down and forget a button waiting for one. `leave` calls it on every pointer motion
/// off the task buttons, so with no panel, button or peek it touches nothing.
pub fn hide(state: &mut State) {
    let v = &state.task_preview;
    if v.pending.is_none() && v.shown.is_none() && !v.peeking {
        return;
    }
    stop_peek(state);
    state.task_preview.pending = None;
    disarm(state);
    let awake = std::mem::take(&mut state.task_preview.awake);
    resuspend(state, awake);
    let Some(p) = state.task_preview.shown.take() else {
        return;
    };
    for id in p.thumbs.into_iter().flatten() {
        state.thumb_drop(id);
    }
    state.destroy_redrawn(p.tree);
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// `w` is going away or leaving the screen: a panel showing it goes too. Called from `forget_window` and `desktops_changed`.
pub fn forget_window(state: &mut State, w: WindowId) {
    if state
        .task_preview
        .shown
        .as_ref()
        .is_some_and(|p| p.windows.contains(&w))
    {
        hide(state);
    }
}

/// The output the panel is on, or the one the bar of the button waiting for one is on.
pub fn open_on(state: &State) -> Option<Output> {
    let v = &state.task_preview;
    match (v.shown_on(), v.pending) {
        (Some(o), _) => Some(o.clone()),
        (None, Some((bar, _))) => state.bar(bar).map(|b| b.output.clone()),
        (None, None) => None,
    }
}

/// The glass under a panel `w` x `h`: the whole panel, its corners rounded as the grid's `corner` cuts them.
fn panel_pane(w: i32, h: i32, corner: &'static [i32]) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, w, h),
            corner,
            square_bottom: false,
            square_top: false,
        }],
        holes: Vec::new(),
    }
}

/// The part of output box `b` on the screen side of bar `p`: where the preview panel may go.
fn preview_bounds(b: Rect, p: &BarPlacement) -> Rect {
    let edge = p.screen_side();
    match p.edge {
        Edge::Bottom => Rect::new(b.x, b.y, b.width, edge - b.y),
        Edge::Top => Rect::new(b.x, edge, b.width, b.bottom() - edge),
        Edge::Left => Rect::new(edge, b.y, b.right() - edge, b.height),
        Edge::Right => Rect::new(b.x, b.y, edge - b.x, b.height),
    }
}

fn show(state: &mut State, bar: TreeId, i: usize) {
    hide(state);
    if state.desktop_locked() {
        return;
    }
    let live = state.task_windows();
    let Some((output, mut windows, btn)) = state.bar(bar).and_then(|b| {
        Some((
            b.output.clone(),
            b.slots.get(i)?.windows.clone(),
            *b.layout.tasks.get(i)?,
        ))
    }) else {
        return;
    };
    windows.retain(|w| live.contains(w));
    if windows.is_empty() {
        return;
    }
    let p = state.bar_placement(&output);
    let button = Rect::new(p.shown.x + btn.x, p.shown.y + btn.y, btn.width, btn.height);
    let bounds = preview_bounds(state.layout_box(Some(&output)), &p);
    let grid = interfaces::task_preview_grid(&state.config.theme);
    let layout = TaskPreviewLayout::compute(grid, windows.len(), button, bounds, p.edge);
    let scale = state.draw_scale_at(Some(&output));
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    scene.set_data(tree, Some(SceneOwner::TaskPreview));
    crate::glass::leave_out(scene, tree);
    crate::output::keep_on(scene, tree, &output);
    scene.set_position(tree, layout.rect.x, layout.rect.y);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    let area = layout.canvas();
    scene.set_position(buffer, area.x, area.y);
    let input = layout.input_area();
    scene.set_input(
        buffer,
        Input::Rect(Rectangle::new(
            (input.x, input.y).into(),
            (input.width, input.height).into(),
        )),
    );
    // The panel never reshapes while open, so its node keeps the glass alive without the handle.
    let pane = glass::wanted(&state.config)
        .then(|| panel_pane(layout.rect.width, layout.rect.height, grid.corner));
    Backdrop::new(&state.glass).set(&mut state.scene, &state.glass, glass_tree, pane);
    let thumbs = windows
        .iter()
        .enumerate()
        .map(|(k, &w)| {
            let well = layout.well(k)?;
            state.thumb_create(w, tree, well, scale)
        })
        .collect();
    state.task_preview.shown = Some(Panel {
        bar,
        button: i,
        output,
        windows,
        layout,
        tree,
        buffer,
        thumbs,
        hot: None,
    });
    redraw(state);
}

/// Icon files landed: the open preview draws its windows' icons again.
pub(crate) fn icons_landed(state: &mut State) {
    redraw(state);
}

fn redraw(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&p.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let windows = p.windows.clone();
    let has_thumb: Vec<bool> = p.thumbs.iter().map(Option::is_some).collect();
    let (layout, hot, buffer, tree) = (p.layout.scaled(scale), p.hot, p.buffer, p.tree);
    let was = state.scene.outputs_of(tree);
    let mut titles = Vec::new();
    let mut app_ids = Vec::new();
    let mut active = Vec::new();
    let live = state.active_task_window();
    for &w in &windows {
        let Some(win) = state.win(w) else {
            return;
        };
        titles.push(win.title());
        app_ids.push(win.app_id());
        active.push(live == Some(w));
    }
    let mut icon = |app_id: &str, px: i32| {
        crate::taskbarview::app_icon(&mut state.icons, &state.app_index, app_id, px)
            .map(Arc::unwrap_or_clone)
    };
    let icons: Vec<Option<Canvas>> = app_ids
        .iter()
        .map(|a| icon(a, SMALL_ICON * scale))
        .collect();
    let pictures: Vec<Option<Canvas>> = app_ids
        .iter()
        .zip(&has_thumb)
        .map(|(a, &thumb)| {
            if thumb {
                None
            } else {
                icon(a, WELL_ICON * scale)
            }
        })
        .collect();
    let cells: Vec<Cell> = (0..windows.len())
        .map(|k| Cell {
            title: &titles[k],
            icon: icons[k].as_ref(),
            active: active[k],
            picture: pictures[k].as_ref(),
        })
        .collect();
    let area = layout.canvas();
    let mut canvas = Canvas::new(area.width.max(1) as u32, area.height.max(1) as u32);
    interfaces::draw_task_preview(
        &mut canvas,
        &theme,
        state.text.for_output(theme.scale),
        &layout,
        &cells,
        hot,
    );
    upload_scaled(&mut state.scene, buffer, &canvas, scale);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

impl TaskPreviewView {
    /// The Quick Launch pins were put in `order` (old indices in new order): a pressed pinned
    /// button keeps its mark under its new pin index.
    pub fn pins_reordered(&mut self, order: &[usize]) {
        self.pressed =
            (self.pressed).and_then(|(b, k)| Some((b, crate::taskbar::pin_renamed(k, order)?)));
    }

    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Whether `w` stays awake for a panel about to show it.
    pub fn keeps_awake(&self, w: WindowId) -> bool {
        self.awake.contains(&w) || self.held_awake.contains(&w)
    }

    pub fn shown_on(&self) -> Option<&Output> {
        self.shown.as_ref().map(|p| &p.output)
    }
}

/// The panel's tree: a button per window named by its title, the hovered one with its close button.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let p = state.task_preview.shown.as_ref()?;
    let mut tree = crate::access::root(Role::Window, "Window previews", p.layout.rect);
    let titles: Vec<String> = (p.windows.iter())
        .map(|&w| state.win(w).map(|x| x.title()).unwrap_or_default())
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    taskpreview::access(&p.layout, &titles, p.hot, &mut tree, ROOT);
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_a_minimized_window_in_the_preview_panel_never_suspends_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "one", 300, 200);
        h.map_toplevel(&mut c, "two", 300, 200);
        h.state.set_minimized(a.id, true);
        h.roundtrip(&mut c);
        assert_eq!(c.side.shells[a.shell].suspended.last(), Some(&true));
        let bar = &h.state.bars[0];
        let (tree, i) = (bar.tree, bar.slots.iter().position(|s| s.windows == [a.id]));
        show(&mut h.state, tree, i.expect("its task button"));
        h.roundtrip(&mut c);
        let heard = &c.side.shells[a.shell].suspended;
        assert_eq!(heard.last(), Some(&false), "its picture wakes it");
        let heard = heard.len();
        clicked(&mut h.state, Some(0));
        h.roundtrip(&mut c);
        assert!(!h.state.win(a.id).unwrap().minimized, "it came up");
        assert!(
            !c.side.shells[a.shell].suspended[heard..].contains(&true),
            "never suspended between the pictures and the restore"
        );
    }

    #[test]
    fn pressing_a_minimized_windows_task_button_under_its_panel_never_suspends_it() {
        for pinned in [false, true] {
            let mut h = crate::harness::Harness::new();
            h.state.config.taskbar.pinned = pinned;
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            h.map_toplevel(&mut c, "two", 300, 200);
            h.state.set_minimized(a.id, true);
            h.roundtrip(&mut c);
            let bar = &h.state.bars[0];
            let i = (bar.slots.iter().position(|s| s.windows == [a.id])).expect("its task button");
            let (tree, r) = (bar.tree, bar.layout.tasks[i]);
            let origin = h.state.bar_drawn(&bar.output).shown;
            show(&mut h.state, tree, i);
            h.roundtrip(&mut c);
            let heard = &c.side.shells[a.shell].suspended;
            assert_eq!(heard.last(), Some(&false), "its picture wakes it");
            let heard = heard.len();
            h.pointer_to(
                f64::from(origin.x + r.x + r.width / 2),
                f64::from(origin.y + r.y + r.height / 2),
            );
            h.button(crate::input::BTN_LEFT, true);
            h.roundtrip(&mut c);
            assert!(
                !h.state.task_preview.is_shown(),
                "the press takes the panel down"
            );
            h.button(crate::input::BTN_LEFT, false);
            h.roundtrip(&mut c);
            assert!(
                !h.state.win(a.id).unwrap().minimized,
                "it came up, pinned {pinned}"
            );
            assert!(
                !c.side.shells[a.shell].suspended[heard..].contains(&true),
                "never suspended between the pictures and the restore, pinned {pinned}"
            );
            h.state.set_minimized(a.id, true);
            h.roundtrip(&mut c);
            let heard = c.side.shells[a.shell].suspended.last();
            assert_eq!(
                heard,
                Some(&true),
                "no picture outlives the press, pinned {pinned}"
            );
        }
    }

    #[test]
    fn after_a_click_on_a_task_button_its_panel_waits_for_the_pointer_to_come_back() {
        for pinned in [false, true] {
            let mut h = crate::harness::Harness::new();
            h.state.config.taskbar.pinned = pinned;
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let bar = &h.state.bars[0];
            let origin = h.state.bar_drawn(&bar.output).shown;
            let centre = |w| {
                let i = (bar.slots.iter().position(|s| s.windows == [w])).expect("a task button");
                let r = bar.layout.tasks[i];
                let x = f64::from(origin.x + r.x + r.width / 2);
                (i, x, f64::from(origin.y + r.y + r.height / 2))
            };
            let (ia, ax, ay) = centre(a.id);
            let (ib, bx, by) = centre(b.id);
            let on = |s: &State, i| (s.task_preview.shown.as_ref()).is_some_and(|p| p.button == i);
            h.pointer_to(ax, ay);
            h.until("the panel on a rest", |s| on(s, ia));
            // Brings it up, then minimizes it.
            for _ in 0..2 {
                h.button(crate::input::BTN_LEFT, true);
                h.button(crate::input::BTN_LEFT, false);
                h.run_for(2 * SHOW_MS);
                assert!(
                    !h.state.task_preview.is_shown(),
                    "no panel while the pointer rests where it clicked, pinned {pinned}"
                );
            }
            h.pointer_to(512.0, 300.0);
            h.pointer_to(ax, ay);
            h.until("the panel once the pointer is back", |s| on(s, ia));
            h.button(crate::input::BTN_LEFT, true);
            h.button(crate::input::BTN_LEFT, false);
            h.pointer_to(bx, by);
            h.until("the panel on the next button", |s| on(s, ib));
        }
    }

    /// The index of `w`'s task button on the first bar and the screen point at its centre.
    fn centre(h: &crate::harness::Harness, w: WindowId) -> (usize, f64, f64) {
        let bar = &h.state.bars[0];
        let origin = h.state.bar_drawn(&bar.output).shown;
        let i = (bar.slots.iter().position(|s| s.windows == [w])).expect("a task button");
        let r = bar.layout.tasks[i];
        let x = f64::from(origin.x + r.x + r.width / 2);
        (i, x, f64::from(origin.y + r.y + r.height / 2))
    }

    fn on(s: &State, i: usize) -> bool {
        (s.task_preview.shown.as_ref()).is_some_and(|p| p.button == i)
    }

    #[test]
    fn a_pressed_task_buttons_panel_stays_down_wherever_a_closing_window_moves_it() {
        for next in [true, false] {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let d = h.map_toplevel(&mut c, "three", 300, 200);
            let (_, bx, by) = centre(&h, b.id);
            h.pointer_to(bx, by);
            h.button(crate::input::BTN_LEFT, true);
            h.button(crate::input::BTN_LEFT, false);
            c.destroy(a);
            h.roundtrip(&mut c);
            let (ib, bx, by) = centre(&h, b.id);
            let (id, dx, dy) = centre(&h, d.id);
            assert_eq!((ib, id), (0, 1), "the row closed up");
            if next {
                h.pointer_to(dx, dy);
                h.until("the panel of the button that took its slot", |s| on(s, id));
                continue;
            }
            h.pointer_to(bx, by);
            h.run_for(2 * SHOW_MS);
            assert!(
                !h.state.task_preview.is_shown(),
                "no panel for the pressed button in its new slot"
            );
            h.pointer_to(512.0, 300.0);
            h.pointer_to(bx, by);
            h.until("its panel once the pointer is back", |s| on(s, ib));
        }
    }

    /// A Quick Launch pin whose app id is `id`, so a window of that app sits on its button.
    fn pin(id: &str) -> crate::quicklaunch::QuickItem {
        let text = format!("[Desktop Entry]\nType=Application\nName={id}\nExec=x\n");
        let file = format!("{id}.desktop");
        crate::quicklaunch::QuickItem {
            entry: crate::quicklaunch::parse(&text, &file.parse().unwrap(), "foot").unwrap(),
            app: reprisefs::apps::parse_desktop(&text, &file.parse().unwrap()),
            file,
        }
    }

    #[test]
    fn a_task_button_dragged_to_another_slot_keeps_its_panel_down_until_the_pointer_comes_back() {
        for pins in [false, true] {
            let mut h = crate::harness::Harness::new();
            // A task button drags only while the taskbar is pinned.
            h.state.config.taskbar.pinned = true;
            if pins {
                h.state.quicklaunch = vec![pin("one"), pin("two")];
            }
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let (ia, ax, ay) = centre(&h, a.id);
            let (ib, bx, _) = centre(&h, b.id);
            let pinned = h.state.bars[0].slots[ia].pin.is_some();
            assert_eq!(pinned, pins, "a window of a pinned app sits on its pin");
            h.pointer_to(ax, ay);
            h.button(crate::input::BTN_LEFT, true);
            for n in 1..=8 {
                h.pointer_to(ax + (bx + 10.0 - ax) * f64::from(n) / 8.0, ay);
            }
            h.button(crate::input::BTN_LEFT, false);
            h.run_for(2 * SHOW_MS);
            let (na, ax, ay) = centre(&h, a.id);
            let (nb, bx, by) = centre(&h, b.id);
            assert_eq!((na, nb), (ib, ia), "the buttons swapped, pins {pins}");
            assert!(
                !h.state.task_preview.is_shown(),
                "no panel while the pointer rests where it dropped it, pins {pins}"
            );
            h.pointer_to(bx, by);
            h.until("the panel of the button in its old slot", |s| on(s, nb));
            h.pointer_to(512.0, 300.0);
            h.pointer_to(ax, ay);
            h.until("its panel once the pointer is back", |s| on(s, na));
        }
    }

    /// A minimized window shown live by the overview or the 3D switcher, which then closes with
    /// the pointer resting on the window's task button; with the count of configures the window
    /// heard before the close.
    fn closed_over_its_button(
        view: &str,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        crate::harness::Toplevel,
        usize,
    ) {
        let config = crate::config::Config::parse(
            "[appearance]\ninterface = \"glass\"\n[effects]\nreduce_motion = true\n",
        )
        .unwrap();
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "one", 300, 200);
        h.map_toplevel(&mut c, "two", 300, 200);
        h.state.set_minimized(a.id, true);
        h.roundtrip(&mut c);
        let (_, x, y) = centre(&h, a.id);
        if view == "overview" {
            h.state.open_overview();
        } else {
            h.state.open_flip(crate::keys::MOD_LOGO);
            h.until("the stack", |s| {
                (s.switcher.stack.as_ref()).is_some_and(|st| st.revealed())
            });
        }
        h.roundtrip(&mut c);
        let heard = &c.side.shells[a.shell].suspended;
        assert_eq!(heard.last(), Some(&false), "the {view} shows it live");
        let heard = heard.len();
        h.pointer_to(x, y);
        if view == "overview" {
            h.state.close_overview();
        } else {
            h.state.switcher_escape();
            let output = h.state.bars[0].output.clone();
            h.state.flip_frame(&output);
            assert!(h.state.switcher.leaving.is_none(), "the cards landed");
        }
        h.roundtrip(&mut c);
        assert!(
            !h.state.live_previews.shows(a.id),
            "the {view} took its picture"
        );
        (h, c, a, heard)
    }

    #[test]
    fn a_minimized_window_handed_from_a_closing_overview_or_stack_to_its_panel_hears_nothing() {
        for view in ["overview", "stack"] {
            let (mut h, mut c, a, heard) = closed_over_its_button(view);
            h.until("its panel", |s| s.task_preview.is_shown());
            h.roundtrip(&mut c);
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [false; 0],
                "the {view}'s picture hands over to the panel's with no configure"
            );
            h.pointer_to(512.0, 300.0);
            h.roundtrip(&mut c);
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [true],
                "suspended once the panel goes, after the {view}"
            );
        }
    }

    #[test]
    fn a_minimized_window_waiting_for_its_panel_stays_awake_into_an_overview_opened_again() {
        let (mut h, mut c, a, heard) = closed_over_its_button("overview");
        h.state.open_overview();
        h.roundtrip(&mut c);
        assert!(h.state.live_previews.shows(a.id), "its cell shows it live");
        assert_eq!(
            c.side.shells[a.shell].suspended[heard..],
            [false; 0],
            "the wait hands over to the cell's picture with no configure"
        );
    }

    #[test]
    fn a_minimized_window_waiting_for_its_panel_is_suspended_when_the_pointer_leaves_first() {
        for view in ["overview", "stack"] {
            let (mut h, mut c, a, heard) = closed_over_its_button(view);
            h.pointer_to(512.0, 300.0);
            h.run_for(2 * SHOW_MS);
            h.roundtrip(&mut c);
            assert!(!h.state.task_preview.is_shown(), "no panel");
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [true],
                "one Suspended once nothing will show it, after the {view}"
            );
        }
    }

    #[test]
    fn the_preview_bounds_are_the_screen_side_of_the_bar() {
        use reprisekit::taskbar::Edge;
        let b = Rect::new(1920, 0, 1280, 720);
        let bottom = crate::barplace::place(b, Edge::Bottom, 30);
        assert_eq!(preview_bounds(b, &bottom), Rect::new(1920, 0, 1280, 690));
        let top = crate::barplace::place(b, Edge::Top, 30);
        assert_eq!(preview_bounds(b, &top), Rect::new(1920, 30, 1280, 690));
        let left = crate::barplace::place(b, Edge::Left, 60);
        assert_eq!(preview_bounds(b, &left), Rect::new(1980, 0, 1220, 720));
        let right = crate::barplace::place(b, Edge::Right, 60);
        assert_eq!(preview_bounds(b, &right), Rect::new(1920, 0, 1220, 720));
    }

    #[test]
    fn the_panel_glass_covers_the_panel_with_the_grid_corners() {
        let p = panel_pane(200, 150, &[3, 1, 1]);
        assert_eq!(p.shapes.len(), 1);
        assert_eq!(p.shapes[0].rect, Rect::new(0, 0, 200, 150));
        assert_eq!(p.shapes[0].corner, &[3, 1, 1], "the grid's cut");
        assert!(p.holes.is_empty(), "the pictures sit over glass");
        assert!(panel_pane(10, 10, &[]).shapes[0].corner.is_empty());
    }

    #[test]
    fn the_popups_glass_rounds_its_corners_above_scale_1_as_its_chrome_does() {
        // Above scale 1 the popup's chrome draws its corners round over 7 px; the glass under it is one shape rounded over as many.
        use reprisekit::glass::{Material, preview};
        let p = panel_pane(235, 166, preview::GRID.corner);
        let whole = vec![(Rect::new(0, 0, 235, 166), [7.0; 4])];
        for scale in [1.25, 2.0] {
            assert_eq!(
                crate::glass::drawn_shapes(&p, Material::Frosted, scale),
                whole,
                "at {scale}"
            );
        }
        assert!(crate::glass::drawn_shapes(&p, Material::Frosted, 1.0).len() > 1);
    }
}
