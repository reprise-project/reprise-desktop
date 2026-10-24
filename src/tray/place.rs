//! Where an item sits: on the bar, or behind the ^ cell with tray overflow on.

use super::{Remembered, TrayItem};
use crate::compositor::State;
use reprisekit::controls::traycustomize::Behaviour;
use std::collections::BTreeMap;

/// Tray overflow is on: `[taskbar] tray_overflow`, on by default under
/// Glass. Off, Passive items stay off the bar and nothing shows them.
pub fn overflow_enabled(config: &crate::config::Config) -> bool {
    config.taskbar.tray_overflow
}

pub fn chevron_shown(state: &State) -> bool {
    shows_chevron(
        &state.config,
        state.tray.as_ref().map_or(0, |t| t.hidden.len()),
    )
}

pub(super) fn shows_chevron(config: &crate::config::Config, hidden: usize) -> bool {
    overflow_enabled(config) && hidden > 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Bar,
    Hidden,
}

pub fn behaviour_of(choices: &BTreeMap<String, Remembered>, item: &TrayItem) -> Behaviour {
    choices
        .get(&item.key)
        .map_or(Behaviour::Auto, |r| r.behaviour)
}

/// Where `item` goes with `behaviour`: with overflow on, behind the ^ cell
/// when the behaviour hides it (its status decides when left Auto); with
/// overflow off every item is on the bar.
pub fn placement(item: &TrayItem, behaviour: Behaviour, overflow: bool) -> Place {
    if overflow && behaviour.hidden(item.passive) {
        Place::Hidden
    } else {
        Place::Bar
    }
}

/// Whether `a` and `b` look the same in a tray cell: the same icon from
/// the same sources and the same status. The tooltip, title, menu and pid
/// are read when used, never drawn.
fn draws_alike(a: &TrayItem, b: &TrayItem) -> bool {
    a.pixmaps == b.pixmaps
        && a.named == b.named
        && a.icon_name == b.icon_name
        && a.icon_theme_path == b.icon_theme_path
        && a.passive == b.passive
}

/// Put a fresh copy of an item at `place`, taking it out of the other
/// list when it moved. A known item keeps its place in its list; a new
/// one goes last. Returns whether the tray draws differently: a new item,
/// or one whose cell changed (`draws_alike`).
pub(super) fn merge_item(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    item: TrayItem,
    place: Place,
) -> bool {
    let changed = items
        .iter()
        .chain(hidden.iter())
        .find(|i| i.id == item.id)
        .is_none_or(|old| !draws_alike(old, &item));
    let (to, from) = match place {
        Place::Bar => (items, hidden),
        Place::Hidden => (hidden, items),
    };
    from.retain(|i| i.id != item.id);
    match to.iter().position(|i| i.id == item.id) {
        Some(i) => to[i] = item,
        None => to.push(item),
    }
    changed
}

/// Put every item back where its behaviour and tray overflow put it,
/// after the choices or `[taskbar] tray_overflow` changed. Items keep
/// their order within a list; one that moves goes last in the other.
pub fn replace_all(state: &mut State) {
    let overflow = overflow_enabled(&state.config);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    arrange(&mut tray.items, &mut tray.hidden, &tray.choices, overflow);
    state.redraw_taskbars();
}

pub(super) fn arrange(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    choices: &BTreeMap<String, Remembered>,
    overflow: bool,
) {
    let at = |i: &TrayItem| placement(i, behaviour_of(choices, i), overflow);
    let (stay, to_hide): (Vec<_>, Vec<_>) = items.drain(..).partition(|i| at(i) == Place::Bar);
    let (to_bar, keep): (Vec<_>, Vec<_>) = hidden.drain(..).partition(|i| at(i) == Place::Bar);
    items.extend(stay.into_iter().chain(to_bar));
    hidden.extend(keep.into_iter().chain(to_hide));
}

/// Whether a balloon from process `pid` is held back: tray overflow is on
/// and an item that process serves hides its icon and its notifications.
pub fn mutes(state: &State, pid: Option<i32>) -> bool {
    let (Some(pid), Some(tray)) = (pid, state.tray.as_ref()) else {
        return false;
    };
    overflow_enabled(&state.config)
        && tray
            .all()
            .any(|i| i.pid == Some(pid) && !behaviour_of(&tray.choices, i).balloons())
}
