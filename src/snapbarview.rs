//! The snap bar on the scene: while a title bar is dragged, the output under the pointer shows `reprisekit::controls::snapbar`'s row of snap layouts at its usable area's top, on the menu layer. A release over a free tile snaps the window as the flyout's pick does (`snappickerview::pick`); any other release, Escape or a lost grab slides the bar out.

use std::time::Instant;

use reprisekit::controls::snapbar::{self, SnapBar, Stage};
use reprisekit::controls::snappicker::{CellSpec, PickerLayout, Shape, Zone};
use reprisekit::interfaces;
use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::glass::{self, Backdrop, Pane, PaneShape};
use crate::input::Interaction;
use crate::livepreview::ThumbId;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::snappickerview::{PickCell, cells_for, icons_for};
use crate::window::WindowId;

const THUMB_W: i32 = 250;
const THUMB_DY: i32 = 8;

#[derive(Default)]
pub struct SnapBarView {
    shown: Option<Bar>,
    leaving: Vec<Bar>,
}

struct Bar {
    window: WindowId,
    output: Output,
    work: Rect,
    model: SnapBar,
    picks: Vec<PickCell>,
    layout: PickerLayout,
    icons: Vec<(Zone, Canvas)>,
    scale: i32,
    hot: Option<Zone>,
    live: bool,
    drawn: Option<Option<Zone>>,
    /// Where the bar was last placed, so a settled bar is not placed again every frame.
    placed: Option<Rect>,
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// The window's live picture under the pointer, its tree and its height.
    thumb: Option<(TreeId, ThumbId, i32)>,
}

/// A settled motion of a move of `w` with the pointer at (`x`, `y`): the bar of the output under it follows; returns the lit zone's rect where a pick lands, which the caller outlines instead of any drop.
pub fn motion(state: &mut State, w: WindowId, x: f64, y: f64) -> Option<Rect> {
    let on = state.config.window_snapping.enabled
        && state.config.window_snapping.bar
        && matches!(state.interaction, Interaction::Move { window, bar: true, .. } if window == w)
        && state.win(w).is_some_and(|x| x.snappable());
    let out = state.output_exactly_at(x, y).filter(|_| on);
    let now = Instant::now();
    if (state.snap_bar.shown.as_ref())
        .is_some_and(|b| b.window != w || Some(&b.output) != out.as_ref())
    {
        leave(state, now);
    }
    let out = out?;
    let (px, py) = (x.floor() as i32, y.floor() as i32);
    if state.snap_bar.shown.is_none() {
        if !snapbar::wanted(state.usable_area(Some(&out)), py) {
            return None;
        }
        create(state, w, &out, now);
    }
    let b = state.snap_bar.shown.as_mut()?;
    b.model.motion(px, py, now);
    let r = b.model.rect(now);
    b.live = b.model.live(now);
    let hot = b.live.then(|| b.layout.hit(px - r.x, py - r.y)).flatten();
    let changed = hot != b.hot;
    b.hot = hot;
    let cell = hot.and_then(|(l, z)| Some((b.picks.get(l)?.clone(), z)));
    let work = b.work;
    let zone = cell.and_then(|(c, z)| {
        let zones = crate::snappickerview::landing(state, &out, work, &c);
        zones.get(z).copied()
    });
    if changed {
        set_thumb(state, hot.is_some());
    }
    move_thumb(state, px, py);
    place(state);
    crate::backend::schedule(state, &out);
    zone
}

/// Where the bar over `out` keeps a release from dropping (`SnapBar::no_drop`), for `snap::target_at`.
pub fn bar_rect(state: &State, out: &Output) -> Option<Rect> {
    let b = state.snap_bar.shown.as_ref().filter(|b| b.output == *out)?;
    let r = b.model.no_drop(Instant::now());
    (!r.is_empty()).then_some(r)
}

/// Whether layout point (`x`, `y`) is on the bar shown over `out`: there the bar wins over a caption under it.
pub fn holds(state: &State, out: &Output, x: f64, y: f64) -> bool {
    (state.snap_bar.shown.as_ref())
        .filter(|b| b.output == *out)
        .is_some_and(|b| b.model.holds(x.floor() as i32, y.floor() as i32))
}

pub fn take_pick(state: &mut State) -> Option<(Output, PickCell, usize)> {
    let b = state.snap_bar.shown.as_ref()?;
    let pick = b
        .hot
        .and_then(|(l, z)| Some((b.output.clone(), b.picks.get(l)?.clone(), z)));
    end(state);
    pick
}

/// The bar shown slides out where it is while the drag goes on elsewhere (`leaving`); on an output gone it goes at once.
fn leave(state: &mut State, now: Instant) {
    set_thumb(state, false);
    let Some(mut b) = state.snap_bar.shown.take() else {
        return;
    };
    if !state.scene.outputs().contains(&b.output) {
        state.scene.destroy(b.tree);
        state.redraw_all();
        return;
    }
    b.hot = None;
    b.model.end(now);
    place_bar(state, &mut b);
    crate::backend::schedule(state, &b.output);
    state.snap_bar.leaving.push(b);
}

/// The drag ended or was cancelled: the window shows again and the bar slides out; bars on a gone output go at once.
pub fn end(state: &mut State) {
    let outputs = state.scene.outputs().to_vec();
    let (kept, gone): (Vec<Bar>, Vec<Bar>) = std::mem::take(&mut state.snap_bar.leaving)
        .into_iter()
        .partition(|b| outputs.contains(&b.output));
    state.snap_bar.leaving = kept;
    for b in gone {
        state.scene.destroy(b.tree);
        state.redraw_all();
    }
    set_thumb(state, false);
    let Some(b) = state.snap_bar.shown.as_mut() else {
        return;
    };
    let out = b.output.clone();
    if !state.scene.outputs().contains(&out) {
        destroy(state);
        return;
    }
    b.hot = None;
    b.model.end(Instant::now());
    place(state);
    crate::backend::schedule(state, &out);
}

/// `output` is about to render: a finished bar (released, cancelled, or taken by a maximize, snap or lock) slides out; one that just settled lights the tile under the pointer; place it, ask for the next frame while it slides, take it down once out.
pub fn frame(state: &mut State, output: &Output) {
    frame_leaving(state, output);
    let Some(b) = state
        .snap_bar
        .shown
        .as_ref()
        .filter(|b| b.output == *output)
    else {
        return;
    };
    let moving =
        matches!(state.interaction, Interaction::Move { window, .. } if window == b.window);
    if !moving && (b.model.stage() != Stage::Hidden || b.thumb.is_some()) {
        end(state);
    }
    let now = Instant::now();
    let Some(b) = state.snap_bar.shown.as_ref() else {
        return;
    };
    if b.model.gone(now) {
        destroy(state);
        return;
    }
    // The slide down just settled under a still pointer: the tiles take it now, as a motion there would.
    if moving && !b.live && b.model.live(now) {
        let w = b.window;
        state.track_snap(w);
    }
    let Some(b) = state.snap_bar.shown.as_ref() else {
        return;
    };
    let more = !b.model.settled(now);
    place(state);
    if more {
        crate::backend::schedule(state, output);
    }
}

fn frame_leaving(state: &mut State, output: &Output) {
    let now = Instant::now();
    let mut more = false;
    for mut b in std::mem::take(&mut state.snap_bar.leaving) {
        if b.output != *output {
            state.snap_bar.leaving.push(b);
            continue;
        }
        if b.model.gone(now) {
            state.scene.destroy(b.tree);
            state.redraw_all();
            continue;
        }
        place_bar(state, &mut b);
        more = true;
        state.snap_bar.leaving.push(b);
    }
    if more {
        crate::backend::schedule(state, output);
    }
}

pub fn forget_window(state: &mut State, w: WindowId) {
    if state.snap_bar.shown.as_ref().is_some_and(|b| b.window == w) {
        destroy(state);
    }
}

fn create(state: &mut State, w: WindowId, out: &Output, now: Instant) {
    let work = state.usable_area(Some(out));
    if work.width <= 0 || work.height <= 0 {
        return;
    }
    let (picks, specs): (Vec<PickCell>, Vec<CellSpec>) = cells_for(state, w, out, work, true);
    let layout = PickerLayout::compute(Shape::Row, (work.width, work.height), &specs);
    let scale = state.draw_scale_at(Some(out)).max(1);
    let icons = icons_for(state, &picks, scale);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    glass::leave_out(scene, tree);
    scene.set_enabled(tree, false);
    // Before the buffer, so it lies beneath it.
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    scene.set_input(buffer, Input::Passthrough);
    state.snap_bar.shown = Some(Bar {
        window: w,
        output: out.clone(),
        work,
        model: SnapBar::new(work, (layout.width, layout.height), now),
        picks,
        layout,
        icons,
        scale,
        hot: None,
        live: false,
        drawn: None,
        placed: None,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        thumb: None,
    });
}

fn destroy(state: &mut State) {
    set_thumb(state, false);
    if let Some(b) = state.snap_bar.shown.take() {
        state.scene.destroy(b.tree);
        state.redraw_all();
    }
}

fn place(state: &mut State) {
    if let Some(mut b) = state.snap_bar.shown.take() {
        place_bar(state, &mut b);
        state.snap_bar.shown = Some(b);
    }
}

/// Put `b` where its slide is: its tree at the part inside the usable area, the buffer showing that part of the row (redrawn only when the lit tile changed), the glass under it. Nothing to do when neither moved nor the lit tile changed.
fn place_bar(state: &mut State, b: &mut Bar) {
    let now = Instant::now();
    let r = b.model.rect(now);
    if b.placed == Some(r) && b.drawn == Some(b.hot) {
        return;
    }
    b.placed = Some(r);
    let visible = r.intersect(&b.work);
    let (tree, buffer, s) = (b.tree, b.buffer, b.scale);
    if visible.is_empty() {
        state.scene.set_enabled(tree, false);
        return;
    }
    if b.drawn != Some(b.hot) {
        let theme = state.config.theme.scaled(s);
        let icons: Vec<(Zone, &Canvas)> = b.icons.iter().map(|(z, c)| (*z, c)).collect();
        let full = interfaces::draw_snap_picker(&theme, &b.layout.scaled(s), &icons, b.hot);
        upload_scaled(&mut state.scene, buffer, &full, s);
    }
    let pane = glass::wanted(&state.config).then(|| Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, visible.width, visible.height),
            corner: reprisekit::snaplayouts::PANEL_CORNER,
            square_bottom: false,
            square_top: r.y < visible.y,
        }],
        holes: Vec::new(),
    });
    b.drawn = Some(b.hot);
    let scene = &mut state.scene;
    let f = f64::from(s);
    scene.buffer_set_source_box(
        buffer,
        Some(Rectangle::new(
            (
                f64::from(visible.x - r.x) * f,
                f64::from(visible.y - r.y) * f,
            )
                .into(),
            (f64::from(visible.width) * f, f64::from(visible.height) * f).into(),
        )),
    );
    scene.buffer_set_dest_size(buffer, Some((visible.width, visible.height).into()));
    scene.set_enabled(tree, true);
    scene.set_position(tree, visible.x, visible.y);
    scene.raise_to_top(tree);
    b.glass.set(scene, &state.glass, b.glass_tree, pane);
}

/// Over a zone the dragged window hides behind its live picture; off it the window shows again, unless it left the screen meanwhile.
fn set_thumb(state: &mut State, on: bool) {
    let Some(b) = state.snap_bar.shown.as_mut() else {
        return;
    };
    let w = b.window;
    let scale = b.scale;
    match (on, b.thumb.take()) {
        (true, None) => {
            let Some(win) = state.win(w) else {
                return;
            };
            let (cw, ch) = (win.client_w.max(1), win.client_h.max(1));
            let wtree = win.tree;
            let tree = state.scene.tree_create(state.layers.overlay);
            glass::leave_out(&mut state.scene, tree);
            let well = Rect::new(0, 0, THUMB_W, THUMB_W * ch / cw);
            // A window with nothing to show yet stays as it is.
            let Some(id) = state.thumb_create(w, tree, well, scale) else {
                state.scene.destroy(tree);
                return;
            };
            state.scene.set_enabled(wtree, false);
            if let Some(b) = state.snap_bar.shown.as_mut() {
                b.thumb = Some((tree, id, well.height));
            }
        }
        (false, Some((tree, id, _))) => {
            state.thumb_drop(id);
            state.scene.destroy(tree);
            let here = state.on_current_desktop(w);
            if let Some(win) = state.win(w) {
                let tree = win.tree;
                let shows = win.mapped && !win.minimized && here;
                state.scene.set_enabled(tree, shows);
            }
        }
        (_, kept) => {
            b.thumb = kept;
            return;
        }
    }
    state.redraw_all();
}

/// The caller schedules the bar's output; a thumbnail reaching past it redraws the others too.
fn move_thumb(state: &mut State, x: i32, y: i32) {
    let Some(b) = state.snap_bar.shown.as_ref() else {
        return;
    };
    let Some((tree, _, h)) = b.thumb else {
        return;
    };
    let r = Rect::new(x - THUMB_W / 2, y + THUMB_DY, THUMB_W, h);
    let inside = state.layout_box(Some(&b.output)).intersect(&r) == r;
    state.scene.set_position(tree, r.x, r.y);
    if !inside {
        state.redraw_all();
    }
}
