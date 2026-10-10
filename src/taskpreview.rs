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
    taskbar::TaskbarHit,
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

/// The pointer is over `hit` on bar `bar`. A task button starts the delay, or takes over a panel at once; anything else on the bar takes it down.
pub fn hover(state: &mut State, bar: TreeId, hit: TaskbarHit) {
    stop_peek(state);
    let target = match hit {
        TaskbarHit::Task(i) if state.config.previews.taskbar => (bar, i),
        _ => {
            hide(state);
            return;
        }
    };
    let v = &state.task_preview;
    let shown = v.shown.as_ref().map(|p| (p.bar, p.button));
    if shown == Some(target) || v.pending == Some(target) {
        return;
    }
    if shown.is_some() {
        show(state, target.0, target.1);
        return;
    }
    state.task_preview.pending = Some(target);
    disarm(state);
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(SHOW_MS)),
        |_, _, state| {
            state.task_preview.timer = None;
            if let Some((bar, i)) = state.task_preview.pending.take() {
                show(state, bar, i);
            }
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
    hide(state);
    if on_close {
        state.close(w);
    } else if state.task_windows().contains(&w) {
        state.bring_up(w);
    }
}

/// Take the panel down and forget a button waiting for one. Called on every pointer motion off the
/// task buttons, so with no panel, button or peek it touches nothing.
pub fn hide(state: &mut State) {
    let v = &state.task_preview;
    if v.pending.is_none() && v.shown.is_none() && !v.peeking {
        return;
    }
    stop_peek(state);
    state.task_preview.pending = None;
    disarm(state);
    let Some(p) = state.task_preview.shown.take() else {
        return;
    };
    for id in p.thumbs.into_iter().flatten() {
        state.thumb_drop(id);
    }
    state.scene.destroy(p.tree);
    state.redraw_all();
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

pub fn output_gone(state: &mut State) {
    let outputs = state.scene.outputs();
    let v = &state.task_preview;
    let gone = v
        .shown
        .as_ref()
        .is_some_and(|p| !outputs.contains(&p.output))
        || v.pending
            .is_some_and(|(bar, _)| state.bar(bar).is_none_or(|b| !outputs.contains(&b.output)));
    if gone {
        hide(state);
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
    if state.is_locked() {
        return;
    }
    let live = state.task_windows();
    let Some((output, mut windows, btn)) = state.bar(bar).and_then(|b| {
        Some((
            b.output.clone(),
            b.buttons.get(i)?.clone(),
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

fn redraw(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&p.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let windows = p.windows.clone();
    let has_thumb: Vec<bool> = p.thumbs.iter().map(Option::is_some).collect();
    let (layout, hot, buffer) = (p.layout.scaled(scale), p.hot, p.buffer);
    let mut titles = Vec::new();
    let mut app_ids = Vec::new();
    let mut active = Vec::new();
    for &w in &windows {
        let Some(win) = state.win(w) else {
            return;
        };
        titles.push(win.title());
        app_ids.push(win.app_id());
        active.push(state.focused == Some(w) && !win.minimized);
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
    interfaces::draw_task_preview(&mut canvas, &theme, &state.text, &layout, &cells, hot);
    upload_scaled(&mut state.scene, buffer, &canvas, scale);
    state.redraw_all();
    crate::access::touch(state);
}

impl TaskPreviewView {
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
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
