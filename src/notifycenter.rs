use std::rc::Rc;

use reprisekit::controls::grouplist;
use reprisekit::controls::listview;
use reprisekit::controls::menu::place_popup;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::notifyhistory::{self, Entry, Event, History, Hit, Key};
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop};
use crate::input::{BTN_LEFT, BTN_RIGHT};
use crate::menuview::{MenuFrom, MenuKind};
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::startmenu::{MenuLayout, MenuStyle};
use crate::tray::notify::DEFAULT_ACTION;

pub struct CenterView {
    model: History,
    /// Layout coordinates.
    at: (i32, i32),
    scale: i32,
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// The last left press on a row and its time (ms), for a double-click.
    last_row_press: Option<(usize, u32)>,
    output: Output,
}

pub fn is_open(state: &State) -> bool {
    state.notify.center.is_some()
}

pub fn open_on(state: &State) -> Option<Output> {
    state.notify.center.as_ref().map(|v| v.output.clone())
}

pub fn toggle(state: &mut State, output: &Output, cell: Rect) {
    if is_open(state) {
        close(state);
        return;
    }
    open(state, output, cell);
}

fn open(state: &mut State, output: &Output, cell: Rect) {
    if state.is_locked() {
        return;
    }
    let usable = state.usable_area(Some(output));
    let cm = state.config.theme.look.controls();
    let size = (notifyhistory::width(&cm), notifyhistory::layout(&cm).height);
    let at = crate::barplace::bar_popup(&state.bar_placement(output), cell, size, usable);
    let scale = state.draw_scale_at(Some(output)).max(1);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    crate::glass::leave_out(scene, tree);
    scene.set_data(tree, Some(SceneOwner::NotifyCenter));
    // Before the buffer, so it lies beneath it.
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    scene.set_position(tree, at.0, at.1);
    state.notify.center = Some(CenterView {
        model: History::new(),
        at,
        scale,
        tree,
        glass_tree,
        glass: Backdrop::new(&state.glass),
        buffer,
        last_row_press: None,
        output: output.clone(),
    });
    state.notify.log.unseen = 0;
    crate::notifyview::hide_balloons(state);
    state.redraw_taskbars();
    refresh(state);
}

pub fn reveal(state: &mut State, app: &str) {
    if !is_open(state) {
        let Some(output) = state.first_output() else {
            return;
        };
        let cell = crate::tray::bell_cell(state, &output).unwrap_or_else(|| {
            let shown = state.bar_placement(&output).shown;
            let w = notifyhistory::width(&state.config.theme.look.controls());
            Rect::new(shown.right() - w, shown.y, 1, shown.height)
        });
        open(state, &output, cell);
    }
    if let Some(v) = state.notify.center.as_mut() {
        v.model.reveal(app);
    }
    redraw(state);
}

pub fn close(state: &mut State) {
    let Some(v) = state.notify.center.take() else {
        return;
    };
    state.scene.destroy(v.tree);
    state.redraw_all();
    crate::autohideview::poke(state);
    crate::access::touch(state);
}

pub fn refresh(state: &mut State) {
    let Some(scale) = state.notify.center.as_ref().map(|v| v.scale) else {
        return;
    };
    let px = listview::ICON_PX * scale;
    let log = &state.notify.log;
    let entries: Vec<Entry> = log
        .entries
        .iter()
        .map(|e| Entry {
            id: e.n.id,
            app: e.n.app.clone(),
            icon: crate::tray::closest(&e.n.icon, px)
                .or_else(|| state.icons.get("dialog-information", px))
                .map(Rc::new),
            message: if e.n.summary.is_empty() {
                e.n.body.lines().next().unwrap_or_default().to_string()
            } else {
                e.n.summary.clone()
            },
            time: crate::taskbar::time_text(e.time),
            live: e.can_invoke(DEFAULT_ACTION),
        })
        .collect();
    let dnd = log.dnd;
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    v.model.set_entries(entries);
    v.model.dnd = dnd;
    redraw(state);
}

fn redraw(state: &mut State) {
    let Some(v) = state.notify.center.as_ref() else {
        return;
    };
    let theme = state.config.theme.scaled(v.scale);
    let canvas = notifyhistory::draw(&theme, &state.text, &v.model);
    let cm = state.config.theme.look.controls();
    let size = (notifyhistory::width(&cm), notifyhistory::layout(&cm).height);
    let pane = glass::flyout_glass(&state.config, size);
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    let scene = &mut state.scene;
    upload_scaled(scene, v.buffer, &canvas, v.scale);
    glass::place_flyout(scene, v.buffer, &state.config.theme, size);
    v.glass.set(scene, &state.glass, v.glass_tree, pane);
    state.redraw_all();
    crate::access::touch(state);
}

/// The open history's tree, the selected row focused.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let v = state.notify.center.as_ref()?;
    let cm = state.config.theme.look.controls();
    let size = (notifyhistory::width(&cm), notifyhistory::layout(&cm).height);
    let rect = Rect::new(v.at.0, v.at.1, size.0, size.1);
    let mut tree = crate::access::root(Role::Window, "Notifications", rect);
    let focus = notifyhistory::access(&v.model, &cm, &mut tree, ROOT);
    tree.set_focus(focus);
    Some(tree)
}

/// A reader focused the row at layout point (`x`, `y`): select it without the click that would
/// fold a header.
pub fn select_at(state: &mut State, x: f64, y: f64) {
    let cm = state.config.theme.look.controls();
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    let Hit::List(grouplist::Hit::Row(i)) = hit(v, x, y, &cm) else {
        return;
    };
    if v.model.select(i) {
        redraw(state);
    }
}

fn hit(v: &CenterView, x: f64, y: f64, cm: &ControlMetrics) -> Hit {
    let lx = (x - f64::from(v.at.0)).floor() as i32;
    let ly = (y - f64::from(v.at.1)).floor() as i32;
    notifyhistory::hit(&v.model, lx, ly, cm)
}

pub fn press(state: &mut State, x: f64, y: f64, button: u32, time: u32) -> bool {
    let threshold = state.config.theme.metrics.double_click_ms.max(0) as u32;
    let cm = state.config.theme.look.controls();
    let Some(v) = state.notify.center.as_mut() else {
        return false;
    };
    let h = hit(v, x, y, &cm);
    let row = match h {
        Hit::List(grouplist::Hit::Row(i)) => Some(i),
        _ => None,
    };
    if button == BTN_RIGHT {
        if let Some(id) = row.and_then(|i| v.model.note_at(i)) {
            v.model.press(h);
            redraw(state);
            open_row_menu(state, id, (x as i32, y as i32));
        }
        return false;
    }
    if button != BTN_LEFT {
        return false;
    }
    let double = match (row, v.last_row_press) {
        (Some(i), Some((j, t))) => i == j && time.wrapping_sub(t) <= threshold,
        _ => false,
    };
    v.last_row_press = if double { None } else { row.map(|i| (i, time)) };
    let invoke = row
        .filter(|&i| double && v.model.is_live(i))
        .and_then(|i| v.model.note_at(i));
    let event = v.model.press(h);
    let holding = v.model.holding();
    apply(state, event);
    if let Some(id) = invoke {
        apply(state, Event::Invoke(id));
    }
    holding
}

pub fn drag(state: &mut State, x: f64, y: f64) {
    let cm = state.config.theme.look.controls();
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    let over = hit(v, x, y, &cm);
    if v.model.motion(over) {
        redraw(state);
    }
}

pub fn release(state: &mut State, x: f64, y: f64) {
    let cm = state.config.theme.look.controls();
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    let over = hit(v, x, y, &cm);
    let event = v.model.release(over);
    apply(state, event);
}

pub fn cancel_press(state: &mut State) {
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    if v.model.cancel_press() {
        redraw(state);
    }
}

/// `steps` notches, positive down.
pub fn scroll(state: &mut State, steps: i64) {
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    if v.model.scroll(3 * steps) {
        redraw(state);
    }
}

pub fn key(state: &mut State, sym: u32, mods: u32) {
    let Some(key) = key_of(sym, mods) else {
        return;
    };
    let Some(v) = state.notify.center.as_mut() else {
        return;
    };
    let event = v.model.key(key);
    apply(state, event);
}

#[expect(non_upper_case_globals)] // xkb keysym names in patterns
fn key_of(sym: u32, mods: u32) -> Option<Key> {
    use crate::keys::*;
    Some(match sym {
        KEY_Up | KEY_KP_Up => Key::Up,
        KEY_Down | KEY_KP_Down => Key::Down,
        KEY_Left | KEY_KP_Left => Key::Left,
        KEY_Right | KEY_KP_Right => Key::Right,
        KEY_Page_Up | KEY_KP_Page_Up => Key::PageUp,
        KEY_Page_Down | KEY_KP_Page_Down => Key::PageDown,
        KEY_Home | KEY_KP_Home => Key::Home,
        KEY_End | KEY_KP_End => Key::End,
        KEY_Return | KEY_KP_Enter => Key::Enter,
        KEY_Delete | KEY_KP_Delete => Key::Delete,
        KEY_Menu => Key::Menu,
        KEY_F10 if mods == MOD_SHIFT => Key::Menu,
        KEY_Escape => Key::Escape,
        _ => return None,
    })
}

fn apply(state: &mut State, event: Event) {
    match event {
        Event::None => {}
        Event::Redraw => redraw(state),
        Event::Dnd => {
            let on = state.notify.center.as_ref().is_some_and(|v| v.model.dnd);
            crate::notifyview::set_dnd(state, on);
        }
        Event::Clear => crate::notifyview::clear_history(state),
        Event::ClearGroup(app) => crate::notifyview::clear_app(state, &app),
        Event::Dismiss(id) => crate::notifyview::forget(state, id),
        Event::Invoke(id) => crate::notifyview::invoke(state, id, DEFAULT_ACTION),
        Event::Menu(id) => {
            let cm = state.config.theme.look.controls();
            let Some(v) = state.notify.center.as_ref() else {
                return;
            };
            let row = v.model.list.selected.and_then(|i| v.model.row_rect(i, &cm));
            if let Some(r) = row {
                open_row_menu(state, id, (v.at.0 + r.x, v.at.1 + r.bottom()));
            }
        }
        Event::Close => close(state),
    }
}

fn open_row_menu(state: &mut State, id: u32, at: (i32, i32)) {
    let Some(e) = state.notify.log.get(id) else {
        return;
    };
    let actions: Vec<(String, String)> =
        e.n.actions
            .iter()
            .filter(|(k, _)| e.can_invoke(k))
            .cloned()
            .collect();
    open_actions_menu(state, id, actions, at, false);
}

pub fn open_actions_menu(
    state: &mut State,
    id: u32,
    actions: Vec<(String, String)>,
    at: (i32, i32),
    left_of: bool,
) {
    if actions.is_empty() {
        return;
    }
    state.close_menu();
    let items = crate::notifyview::action_items(&actions);
    let Some(output) = state.output_at(f64::from(at.0), f64::from(at.1)) else {
        return;
    };
    let usable = state.usable_area(Some(&output));
    let style = MenuStyle::context_for(&state.config.theme, &items).fit(
        &state.text,
        &items,
        state.config.theme.fonts.size_px,
        usable.width,
    );
    let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let x = if left_of { at.0 - layout.width } else { at.0 };
    let pos = place_popup((x, at.1), (layout.width, layout.height), usable);
    let keys = actions.into_iter().map(|(k, _)| k).collect();
    let kind = MenuKind::NotifyActions { id, keys };
    state.push_menu(0, pos, layout, items, output, kind, MenuFrom::Elsewhere);
    state.redraw_menu(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::*;

    #[test]
    fn shift_f10_opens_the_row_menu_like_the_menu_key() {
        assert_eq!(key_of(KEY_F10, MOD_SHIFT), Some(Key::Menu));
        assert_eq!(key_of(KEY_Menu, 0), Some(Key::Menu));
        assert_eq!(key_of(KEY_F10, 0), None);
        assert_eq!(key_of(KEY_F10, MOD_SHIFT | MOD_CTRL), None);
    }
}
