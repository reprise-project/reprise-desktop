//! The snap layout picker on the scene graph: one `reprisekit::controls::snappicker` panel hanging
//! under a maximize button on the menu layer, opened by resting the pointer there ([`SHOW_MS`],
//! `[window_snapping] picker`) or by the `snap-layouts` bind, which also gives it the keyboard. A
//! pick snaps the window and offers the free zones to other windows (`apply_layout`,
//! `open_snap_assist`).

use std::time::Duration;

use reprisekit::classic::frame::{CaptionButton, FrameHit};
use reprisekit::controls::snappicker::{
    self, CellSpec, Event, Key, PickerLayout, Shape, SnapPicker, Zone,
};
use reprisekit::interfaces;
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::input::BTN_LEFT;
use crate::keys;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::snap::{Suggestion, ZoneLayout};
use crate::window::WindowId;

pub const SHOW_MS: u64 = 400;

#[derive(Default)]
pub struct SnapPickerView {
    timer: Option<RegistrationToken>,
    pending: Option<WindowId>,
    shown: Option<Picker>,
}

struct Picker {
    window: WindowId,
    output: Output,
    picks: Vec<PickCell>,
    specs: Vec<CellSpec>,
    icons: Vec<(Zone, Canvas)>,
    scale: i32,
    layout: PickerLayout,
    at: (i32, i32),
    bridge: Rect,
    keys: bool,
    model: SnapPicker,
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
}

impl SnapPickerView {
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    pub fn has_keys(&self) -> bool {
        self.shown.as_ref().is_some_and(|p| p.keys)
    }
}

pub fn place(button: Rect, size: (i32, i32), usable: Rect) -> (i32, i32) {
    let (w, h) = size;
    let x = button.x + button.width / 2 - w / 2;
    let y = button.bottom();
    let x = x.min(usable.right() - w).max(usable.x);
    let y = y.min(usable.bottom() - h).max(usable.y);
    (x, y)
}

fn eligible(state: &State, w: WindowId) -> bool {
    state.win(w).is_some_and(|x| x.snappable())
}

pub fn hover_frame(state: &mut State, window: WindowId, hit: FrameHit) {
    let on_max = hit == FrameHit::Button(CaptionButton::Maximize);
    if !on_max {
        leave(state);
        return;
    }
    let v = &state.snap_picker;
    if v.shown.as_ref().is_some_and(|p| p.window == window) || v.pending == Some(window) {
        return;
    }
    leave(state);
    if !state.config.window_snapping.picker || !eligible(state, window) {
        return;
    }
    state.snap_picker.pending = Some(window);
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(SHOW_MS)),
        |_, _, state| {
            state.snap_picker.timer = None;
            if let Some(w) = state.snap_picker.pending.take() {
                show(state, w, false);
            }
            TimeoutAction::Drop
        },
    );
    match token {
        Ok(t) => state.snap_picker.timer = Some(t),
        Err(e) => log::warn!("snap picker timer: {}", e.error),
    }
}

pub fn leave(state: &mut State) {
    disarm(state);
    let p = state.pointer_location();
    let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
    if (state.snap_picker.shown.as_ref()).is_some_and(|p| !p.keys && !p.bridge.contains(x, y)) {
        hide(state);
    }
}

fn bridge(button: Rect, at: (i32, i32)) -> Rect {
    const SLACK: i32 = 4;
    Rect::new(
        button.x - SLACK,
        button.y,
        button.width + 2 * SLACK,
        (at.1 - button.y).max(0),
    )
}

fn disarm(state: &mut State) {
    state.snap_picker.pending = None;
    if let Some(t) = state.snap_picker.timer.take() {
        state.handle.remove(t);
    }
}

pub fn toggle(state: &mut State) {
    if state.snap_picker.shown.is_some() {
        hide(state);
        return;
    }
    if let Some(w) = state.focused {
        show(state, w, true);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PickCell {
    Layout(ZoneLayout),
    Suggestion(Suggestion),
}

impl PickCell {
    pub(crate) fn layout(&self) -> &ZoneLayout {
        match self {
            PickCell::Layout(l) => l,
            PickCell::Suggestion(s) => &s.layout,
        }
    }
}

pub(crate) fn cells_for(
    state: &State,
    w: WindowId,
    out: &Output,
    work: Rect,
    bar: bool,
) -> (Vec<PickCell>, Vec<CellSpec>) {
    let open = |m: WindowId| state.win(m).is_some_and(|x| x.mapped) && state.on_current_desktop(m);
    let suggestions = state.wm.snap.recents.suggestions(&out.name(), w, open);
    let builtin = || {
        if bar {
            crate::snap::bar_layouts(work)
        } else {
            crate::snap::picker_layouts(work)
        }
    };
    let title = |m: WindowId| state.win(m).map(|x| x.title()).unwrap_or_default();
    cells(
        suggestions,
        &state.config.window_snapping.layouts,
        builtin,
        work,
        title,
    )
}

pub(crate) fn cells(
    suggestions: Vec<Suggestion>,
    own: &[ZoneLayout],
    builtin: impl FnOnce() -> Vec<ZoneLayout>,
    work: Rect,
    title: impl Fn(WindowId) -> String,
) -> (Vec<PickCell>, Vec<CellSpec>) {
    let mut picks: Vec<PickCell> = suggestions.into_iter().map(PickCell::Suggestion).collect();
    let fixed = if own.is_empty() {
        builtin()
    } else {
        own.to_vec()
    };
    picks.extend(fixed.into_iter().map(PickCell::Layout));
    let specs = (picks.iter())
        .map(|p| {
            let zones = (p.layout().zones(work).iter())
                .map(|z| Rect::new(z.x - work.x, z.y - work.y, z.width, z.height))
                .collect();
            match p {
                PickCell::Layout(_) => CellSpec {
                    zones,
                    ..CellSpec::default()
                },
                PickCell::Suggestion(s) => CellSpec {
                    zones,
                    suggestion: true,
                    filled: s.filled.iter().map(|&(z, m)| (z, title(m))).collect(),
                },
            }
        })
        .collect();
    (picks, specs)
}

pub(crate) fn icons_for(state: &mut State, picks: &[PickCell], scale: i32) -> Vec<(Zone, Canvas)> {
    let mut out = Vec::new();
    for (l, p) in picks.iter().enumerate() {
        if let PickCell::Suggestion(s) = p {
            for &(z, m) in &s.filled {
                if let Some(icon) = state.window_icon(m, snappicker::ICON_PX * scale) {
                    out.push(((l, z), icon));
                }
            }
        }
    }
    out
}

pub(crate) fn landing(state: &State, out: &Output, work: Rect, cell: &PickCell) -> Vec<Rect> {
    let join = matches!(cell, PickCell::Layout(_));
    let live = state.snap_live();
    let live = |m| live.contains(&m);
    (state.wm.snap)
        .landing(&out.name(), cell.layout(), join, work, &live)
        .1
}

pub(crate) fn pick(state: &mut State, w: WindowId, out: &Output, cell: &PickCell, zone: usize) {
    match cell {
        PickCell::Layout(l) => state.apply_layout(out, l.clone(), true, w, zone),
        PickCell::Suggestion(s) => state.apply_suggestion(out, s, w, zone),
    }
}

fn show(state: &mut State, w: WindowId, keys: bool) {
    hide(state);
    if state.is_locked() || !eligible(state, w) {
        return;
    }
    let Some(win) = state.win(w) else {
        return;
    };
    let frame = win.frame_rect();
    let button =
        if win.geom.chrome.decorated && win.geom.chrome.buttons.has(CaptionButton::Maximize) {
            let b = win.geom.button_rect(CaptionButton::Maximize);
            Rect::new(frame.x + b.x, frame.y + b.y, b.width, b.height)
        } else {
            Rect::new(frame.x + frame.width / 2, frame.y, 0, 0)
        };
    let centre = (frame.x + frame.width / 2, frame.y + frame.height / 2);
    let Some(output) = state.output_at(f64::from(centre.0), f64::from(centre.1)) else {
        return;
    };
    let work = state.usable_area(Some(&output));
    if work.width <= 0 || work.height <= 0 {
        return;
    }
    let (picks, specs) = cells_for(state, w, &output, work, false);
    let layout = PickerLayout::compute(Shape::Grid, (work.width, work.height), &specs);
    let scale = state.draw_scale_at(Some(&output)).max(1);
    let icons = icons_for(state, &picks, scale);
    let at = place(button, (layout.width, layout.height), work);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    scene.set_data(tree, Some(SceneOwner::SnapPicker));
    crate::glass::leave_out(scene, tree);
    scene.set_position(tree, at.0, at.1);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    let model = SnapPicker {
        hot: keys.then(|| layout.order().first().copied()).flatten(),
    };
    state.snap_picker.shown = Some(Picker {
        window: w,
        output,
        picks,
        specs,
        icons,
        scale,
        layout,
        at,
        bridge: bridge(button, at),
        keys,
        model,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
    });
    redraw(state);
}

pub fn hide(state: &mut State) {
    disarm(state);
    let Some(p) = state.snap_picker.shown.take() else {
        return;
    };
    state.scene.destroy(p.tree);
    state.redraw_all();
    crate::access::touch(state);
}

pub fn forget_window(state: &mut State, w: WindowId) {
    if state.snap_picker.pending == Some(w) {
        disarm(state);
    }
    if state
        .snap_picker
        .shown
        .as_ref()
        .is_some_and(|p| p.window == w)
    {
        hide(state);
    }
}

pub fn output_gone(state: &mut State) {
    let outputs = state.scene.outputs();
    if (state.snap_picker.shown.as_ref()).is_some_and(|p| !outputs.contains(&p.output)) {
        hide(state);
    }
}

fn redraw(state: &mut State) {
    let Some(p) = state.snap_picker.shown.as_ref() else {
        return;
    };
    let scale = p.scale;
    let theme = state.config.theme.scaled(scale);
    let size = (p.layout.width, p.layout.height);
    let pane =
        glass::wanted(&state.config).then(|| glass::sheet_pane(size, snappicker::PANEL_CORNER));
    let icons: Vec<(Zone, &Canvas)> = p.icons.iter().map(|(z, c)| (*z, c)).collect();
    let canvas = interfaces::draw_snap_picker(&theme, &p.layout.scaled(scale), &icons, p.model.hot);
    let Some(p) = state.snap_picker.shown.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, p.buffer, &canvas, scale);
    p.glass.set(scene, &state.glass, p.glass_tree, pane);
    state.redraw_all();
    crate::access::touch(state);
}

pub fn hit(state: &State, lx: f64, ly: f64) -> Option<Zone> {
    let p = state.snap_picker.shown.as_ref()?;
    let x = (lx - f64::from(p.at.0)).floor() as i32;
    let y = (ly - f64::from(p.at.1)).floor() as i32;
    p.layout.hit(x, y)
}

pub fn hover_panel(state: &mut State, zone: Option<Zone>) {
    disarm(state);
    let Some(p) = state.snap_picker.shown.as_mut() else {
        return;
    };
    let e = p.model.hover(zone);
    apply(state, e);
}

pub fn press(state: &mut State, zone: Option<Zone>, button: u32) {
    if button != BTN_LEFT {
        hide(state);
        return;
    }
    let Some(p) = state.snap_picker.shown.as_ref() else {
        return;
    };
    let e = p.model.click(zone);
    apply(state, e);
}

pub fn key(state: &mut State, sym: u32) {
    let key = match sym {
        keys::KEY_Left | keys::KEY_KP_Left => Key::Left,
        keys::KEY_Right | keys::KEY_KP_Right | keys::KEY_Tab => Key::Right,
        keys::KEY_Up | keys::KEY_KP_Up => Key::Up,
        keys::KEY_Down | keys::KEY_KP_Down => Key::Down,
        keys::KEY_Return | keys::KEY_KP_Enter | keys::KEY_space => Key::Enter,
        keys::KEY_Escape => Key::Escape,
        _ => return,
    };
    let Some(p) = state.snap_picker.shown.as_mut() else {
        return;
    };
    let e = p.model.key(&p.layout, key);
    apply(state, e);
}

fn apply(state: &mut State, e: Event) {
    match e {
        Event::None => {}
        Event::Redraw => redraw(state),
        Event::Close => hide(state),
        Event::Pick((l, z)) => {
            let Some(p) = state.snap_picker.shown.as_ref() else {
                return;
            };
            let (w, out, cell) = (p.window, p.output.clone(), p.picks.get(l).cloned());
            hide(state);
            if let Some(cell) = cell {
                pick(state, w, &out, &cell, z);
            }
        }
    }
}

pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let p = state.snap_picker.shown.as_ref()?;
    let rect = Rect::new(p.at.0, p.at.1, p.layout.width, p.layout.height);
    let mut tree = crate::access::root(Role::Window, "Snap layouts", rect);
    let focus = snappicker::access(&p.layout, &p.specs, p.model.hot, &mut tree, ROOT);
    tree.set_focus(focus);
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_hangs_under_its_button_inside_the_usable_area() {
        let usable = Rect::new(0, 0, 1920, 1050);
        let button = Rect::new(900, 6, 16, 14);
        assert_eq!(place(button, (280, 59), usable), (768, 20));
        let right = Rect::new(1890, 6, 16, 14);
        assert_eq!(place(right, (280, 59), usable), (1640, 20));
        let low = Rect::new(900, 1040, 16, 14);
        assert_eq!(place(low, (280, 59), usable), (768, 991));
        let off = Rect::new(-50, 6, 16, 14);
        assert_eq!(place(off, (280, 59), usable).0, 0);
    }

    #[test]
    fn suggestions_lead_and_the_configs_layouts_replace_the_built_in_ones() {
        let work = Rect::new(0, 30, 1920, 1050);
        let halves = ZoneLayout::parse("1|1").unwrap();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        let w = WindowId::next();
        let s = Suggestion {
            layout: thirds.clone(),
            filled: vec![(2, w)],
        };
        let title = |m: WindowId| if m == w { "Mail".into() } else { String::new() };
        let builtin = || vec![halves.clone()];
        let (picks, specs) = cells(vec![s.clone()], &[], builtin, work, title);
        assert_eq!(
            picks,
            vec![
                PickCell::Suggestion(s.clone()),
                PickCell::Layout(halves.clone())
            ]
        );
        assert!(specs[0].suggestion);
        assert_eq!(specs[0].filled, vec![(2, "Mail".to_string())]);
        assert_eq!(
            specs[1].zones,
            vec![Rect::new(0, 0, 960, 1050), Rect::new(960, 0, 960, 1050)],
            "zones from the usable area's top-left"
        );
        let own = [thirds.clone()];
        let (picks, _) = cells(Vec::new(), &own, || unreachable!(), work, title);
        assert_eq!(picks, vec![PickCell::Layout(thirds)], "no built-in ones");
    }

    #[test]
    fn the_bridge_spans_the_caption_between_the_button_and_the_panel() {
        let button = Rect::new(440, 26, 26, 17);
        let b = bridge(button, (300, 48));
        assert_eq!(b, Rect::new(436, 26, 34, 22));
        assert!(b.contains(453, 46), "the caption's foot under the button");
        assert!(!b.contains(453, 48), "the panel itself");
        assert!(!b.contains(420, 40), "the title beside the button");
        assert_eq!(bridge(button, (300, 10)).height, 0, "a panel above");
    }
}
