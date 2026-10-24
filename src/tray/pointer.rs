use super::{Cell, ClickKind, Orientation, TrayCommand, builtin, cell_at, leading, send};
use crate::compositor::State;
use crate::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit};
use crate::menuview::MenuKind;
use crate::raster::Rect;
use crate::taskbar::TaskbarHit;
use reprisescene::TreeId;
use smithay::input::pointer::AxisFrame;
use smithay::output::Output;

fn cell_rect(state: &State, bar: TreeId, index: usize) -> Option<(Output, Rect)> {
    let b = state.bar(bar)?;
    let r = *b.layout.tray.get(index)?;
    let abs = bar_cell(state.bar_placement(&b.output).shown, r);
    Some((b.output.clone(), abs))
}

pub fn bell_cell(state: &State, output: &Output) -> Option<Rect> {
    let k = builtin::shown(state)
        .iter()
        .position(|b| *b == builtin::Builtin::Notifications)?;
    let bar = state.bars.iter().find(|b| b.output == *output)?;
    let i =
        (0..bar.layout.tray.len()).find(|&i| cell_at(state, bar.tree, i) == Cell::Builtin(k))?;
    cell_rect(state, bar.tree, i).map(|(_, r)| r)
}

pub(crate) fn bar_cell(shown: Rect, r: Rect) -> Rect {
    Rect::new(shown.x + r.x, shown.y + r.y, r.width, r.height)
}

pub fn click(state: &mut State, bar: TreeId, index: usize, button: u32) {
    let Some((output, cell)) = cell_rect(state, bar, index) else {
        return;
    };
    let leading = leading(state);
    let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
    if let Cell::Builtin(b) = cell_at(state, bar, index) {
        if let Some(&b) = builtin::shown(state).get(b) {
            let open_here = builtin::menu_open(state, b);
            state.close_menu();
            if !open_here {
                builtin::click(state, &output, b, cell, button);
            }
        }
        return;
    }
    crate::notifycenter::close(state);
    let Some(id) = state
        .tray
        .as_ref()
        .and_then(|t| t.item_at(revealed, leading, index))
        .map(|i| i.id.clone())
    else {
        return;
    };
    let at = state.bar_placement(&output).click_point(cell);
    click_item(state, output, id, cell, at, button);
}

/// A pointer button on item `id`, whose icon sits at `icon` (layout
/// coordinates) on `output`'s bar or flyout: left activates it, middle is
/// its secondary activation, right opens its menu (or asks it for one).
/// The item hears of a click at `at`, on the bar's or flyout's edge
/// facing the screen.
pub fn click_item(
    state: &mut State,
    output: Output,
    id: String,
    icon: Rect,
    at: (i32, i32),
    button: u32,
) {
    let open_here = matches!(
        state.menus.first().map(|m| &m.kind),
        Some(MenuKind::Tray { id: open, .. }) if *open == id
    );
    if !state.menus.is_empty() {
        state.close_menu();
    }
    if open_here {
        return;
    }
    let has_menu = state
        .tray
        .as_ref()
        .and_then(|t| t.item(&id))
        .is_some_and(|i| i.menu_path.is_some());
    let Some(command) = click_command(id.clone(), at, button, has_menu, || {
        state.new_activation_token()
    }) else {
        return;
    };
    if matches!(command, TrayCommand::GetLayout { .. })
        && let Some(tray) = state.tray.as_mut()
    {
        tray.pending_menu = Some((id, output, icon, at));
        tray.submenus.clear();
    }
    send(state, command);
}

/// An activation carries a token from `mint`, so the window it shows or
/// raises can take focus.
pub(super) fn click_command(
    id: String,
    at: (i32, i32),
    button: u32,
    has_menu: bool,
    mint: impl FnOnce() -> String,
) -> Option<TrayCommand> {
    let (x, y) = at;
    let kind = match button {
        BTN_LEFT => ClickKind::Activate,
        BTN_MIDDLE => ClickKind::SecondaryActivate,
        BTN_RIGHT if has_menu => return Some(TrayCommand::GetLayout { id, parent: 0 }),
        BTN_RIGHT => ClickKind::ContextMenu,
        _ => return None,
    };
    let token = (kind != ClickKind::ContextMenu).then(mint);
    Some(TrayCommand::Click {
        id,
        x,
        y,
        kind,
        token,
    })
}

/// A wheel step on tray cell `index`: a built-in cell's goes to
/// [`builtin::scroll`], an item's straight to the item as `Scroll`, leaving
/// an open menu alone. `discrete` and `continuous` are the axis event's (see
/// [`wheel_steps`]). An item's sub-notch remainder is kept for the next event
/// over the same cell and axis; the displays cell keeps its own in
/// [`crate::displays::Displays::wheel`].
pub fn scroll(
    state: &mut State,
    bar: TreeId,
    index: usize,
    horizontal: bool,
    discrete: i32,
    continuous: f64,
) {
    if let Cell::Builtin(k) = cell_at(state, bar, index) {
        if let Some(&b) = builtin::shown(state).get(k) {
            builtin::scroll(state, b, horizontal, discrete, continuous);
        }
        return;
    }
    let leading = leading(state);
    let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    // A remainder belongs to the cell and the axis it came from; moving to
    // another icon starts the count again.
    let carried = match tray.wheel {
        Some((cell, h, r)) if cell == index && h == horizontal => r,
        _ => 0,
    };
    let (steps, residual) = wheel_steps(discrete, continuous, carried);
    tray.wheel = Some((index, horizontal, residual));
    let Some(id) = tray.item_at(revealed, leading, index).map(|i| i.id.clone()) else {
        return;
    };
    scroll_item(state, id, steps, horizontal);
}

pub fn wheel_left(state: &mut State) {
    if let Some(t) = state.tray.as_mut() {
        t.wheel = None;
    }
    state.displays.wheel = (0, 0.0);
}

/// Whether tray cell `index` on `bar` is the displays cell.
pub fn is_displays_cell(state: &State, bar: TreeId, index: usize) -> bool {
    let Cell::Builtin(k) = cell_at(state, bar, index) else {
        return false;
    };
    builtin::shown(state).get(k) == Some(&builtin::Builtin::Displays)
}

impl State {
    pub fn tray_axis(&mut self, frame: &AxisFrame) -> bool {
        let hit = self.scene_hit(self.pointer_location());
        // Over the history popup the wheel scrolls its list, in whole
        // notches (`wheel_steps` counts up as positive).
        if hit == Hit::NotifyCenter {
            let discrete = frame.v120.map_or(0, |v| v.1);
            let continuous = frame.axis.1;
            if discrete != 0 || continuous != 0.0 {
                let carried = self.notify.center_wheel;
                let (steps, residual) = wheel_steps(discrete, continuous, carried);
                self.notify.center_wheel = residual;
                crate::notifycenter::scroll(self, -i64::from(steps));
            }
            return true;
        }
        let (on_bar, in_flyout) = match hit {
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Tray(i),
            } => (Some((bar, i)), None),
            Hit::TrayFlyout { cell } => (None, Some(cell)),
            _ => {
                wheel_left(self);
                return false;
            }
        };
        let v120 = frame.v120.unwrap_or((0, 0));
        for (discrete, continuous, horizontal) in
            [(v120.1, frame.axis.1, false), (v120.0, frame.axis.0, true)]
        {
            if discrete == 0 && continuous == 0.0 {
                continue;
            }
            match (on_bar, in_flyout) {
                (Some((bar, i)), _) => scroll(self, bar, i, horizontal, discrete, continuous),
                (_, Some(Some(cell))) => {
                    crate::trayflyout::scroll(self, cell, horizontal, discrete, continuous)
                }
                _ => {}
            }
        }
        true
    }
}

/// Whole wheel notches an axis event carries, positive away from the user,
/// plus the 120ths left over for the next event.
///
/// Wayland counts a wheel step positive towards the user (down, or right),
/// with `discrete` in 120ths of a notch; a touchpad leaves it 0 and only the
/// sign of `continuous` counts. `Scroll` on the item side is Qt's angle
/// delta, positive away from the user, so the sign flips here and the
/// sub-notch remainder is carried in `residual`.
pub fn wheel_steps(discrete: i32, continuous: f64, residual: i32) -> (i32, i32) {
    if discrete != 0 {
        let total = residual.saturating_add(discrete);
        return (-(total / 120), total % 120);
    }
    if continuous > 0.0 {
        (-1, 0)
    } else if continuous < 0.0 {
        (1, 0)
    } else {
        (0, residual)
    }
}

pub fn scroll_item(state: &mut State, id: String, delta: i32, horizontal: bool) {
    if delta == 0 {
        return;
    }
    let orientation = if horizontal {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    };
    send(
        state,
        TrayCommand::Scroll {
            id,
            delta,
            orientation,
        },
    );
}
