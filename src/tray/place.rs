//! Where an item sits: on the bar, or behind the ^ cell with tray overflow on.

use super::{Remembered, TrayItem};
use crate::compositor::State;
use reprisekit::controls::traycustomize::Behaviour;
use std::collections::{BTreeMap, HashMap};

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
/// when the behaviour hides it (its status decides when left Auto) and,
/// unless muted, it does not need attention; with overflow off every item
/// is on the bar.
pub fn placement(item: &TrayItem, behaviour: Behaviour, overflow: bool) -> Place {
    let calls = item.needs_attention && behaviour.balloons();
    if overflow && behaviour.hidden(item.passive) && !calls {
        Place::Hidden
    } else {
        Place::Bar
    }
}

/// What a fresh copy of an item changes in its tray cell. The tooltip, title, menu and pid are
/// read when used, never drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    None,
    /// Only Passive or NeedsAttention: the icons drawn for each status stay valid.
    Status,
    /// A new item, or other icon sources, attention icon or overlay.
    Icon,
}

fn change(old: &TrayItem, new: &TrayItem) -> Change {
    let icon_alike = old.pixmaps == new.pixmaps
        && old.named == new.named
        && old.icon_name == new.icon_name
        && old.icon_theme_path == new.icon_theme_path
        && old.attention == new.attention
        && old.overlay == new.overlay;
    if !icon_alike {
        Change::Icon
    } else if old.passive != new.passive || old.needs_attention != new.needs_attention {
        Change::Status
    } else {
        Change::None
    }
}

/// Put a fresh copy of an item at `place`, taking it out of the other list when it moved. A
/// known item keeps its place in its list; one back in the hidden list goes where it left it
/// (`left`, by item id); a new one goes last.
pub(super) fn merge_item(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    left: &mut HashMap<String, usize>,
    item: TrayItem,
    place: Place,
) -> Change {
    let changed = (items.iter().chain(hidden.iter()))
        .find(|i| i.id == item.id)
        .map_or(Change::Icon, |old| change(old, &item));
    let (to, from) = match place {
        Place::Bar => (items, hidden),
        Place::Hidden => (hidden, items),
    };
    if let Some(at) = from.iter().position(|i| i.id == item.id) {
        from.remove(at);
        if place == Place::Bar {
            left.insert(item.id.clone(), at);
        }
    }
    match to.iter().position(|i| i.id == item.id) {
        Some(i) => to[i] = item,
        None => {
            let at = (place == Place::Hidden).then(|| left.remove(&item.id));
            to.insert(at.flatten().map_or(to.len(), |at| at.min(to.len())), item);
        }
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
