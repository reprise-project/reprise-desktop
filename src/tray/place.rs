//! Where an item sits: on the bar, or behind the ^ cell with tray overflow on.

use super::{Notification, Remembered, TrayItem};
use crate::compositor::State;
use reprisekit::controls::traycustomize::Behaviour;
use std::collections::BTreeMap;

/// How many keys [`Order`] keeps; past it the oldest key no registered item has goes.
const ORDER_KEYS: usize = 64;

/// The tray's item order across sessions: item keys in the order the tray first saw them, kept as
/// `tray_order` in the state file. Both lists sort by it, so an item's cell does not depend on
/// which program registered first at login.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order(Vec<String>);

impl Order {
    pub fn new(keys: Vec<String>) -> Self {
        Order(keys)
    }

    pub fn keys(&self) -> &[String] {
        &self.0
    }

    /// Where `key` sorts; an unknown key sorts last.
    pub fn rank(&self, key: &str) -> usize {
        self.0.iter().position(|k| k == key).unwrap_or(self.0.len())
    }

    /// Adds `key` at the end if new, dropping the oldest key `registered` says no item has once
    /// over [`ORDER_KEYS`]. True if the order changed.
    pub fn note(&mut self, key: &str, registered: impl Fn(&str) -> bool) -> bool {
        if self.0.iter().any(|k| k == key) {
            return false;
        }
        self.0.push(key.to_string());
        if self.0.len() > ORDER_KEYS
            && let Some(at) = self.0.iter().position(|k| k != key && !registered(k))
        {
            self.0.remove(at);
        }
        true
    }

    fn sort(&self, list: &mut [TrayItem]) {
        list.sort_by_cached_key(|i| self.rank(&i.key));
    }
}

/// Tray overflow is on: `[taskbar] tray_overflow`, on by default under
/// Glass. Off, every item sits on the bar.
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
/// known item keeps its cell; one entering a list goes after every item `order` ranks with or
/// before it.
pub(super) fn merge_item(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    order: &Order,
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
    from.retain(|i| i.id != item.id);
    match to.iter().position(|i| i.id == item.id) {
        Some(i) => to[i] = item,
        None => {
            let rank = order.rank(&item.key);
            let at = to.partition_point(|i| order.rank(&i.key) <= rank);
            to.insert(at, item);
        }
    }
    changed
}

/// Put every item back where its behaviour and tray overflow put it,
/// after the choices or `[taskbar] tray_overflow` changed, each list in
/// the tray's [`Order`].
pub fn replace_all(state: &mut State) {
    let overflow = overflow_enabled(&state.config);
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    arrange(
        &mut tray.items,
        &mut tray.hidden,
        &tray.choices,
        &tray.order,
        overflow,
    );
    state.redraw_taskbars();
}

pub(super) fn arrange(
    items: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    choices: &BTreeMap<String, Remembered>,
    order: &Order,
    overflow: bool,
) {
    let at = |i: &TrayItem| placement(i, behaviour_of(choices, i), overflow);
    let (stay, to_hide): (Vec<_>, Vec<_>) = items.drain(..).partition(|i| at(i) == Place::Bar);
    let (to_bar, keep): (Vec<_>, Vec<_>) = hidden.drain(..).partition(|i| at(i) == Place::Bar);
    items.extend(stay.into_iter().chain(to_bar));
    hidden.extend(keep.into_iter().chain(to_hide));
    order.sort(items);
    order.sort(hidden);
}

/// Whether balloon `n` is held back: tray overflow is on and a Mute choice is for its program.
pub fn mutes(state: &State, n: &Notification) -> bool {
    let Some(tray) = state.tray.as_ref() else {
        return false;
    };
    overflow_enabled(&state.config) && muted(&tray.choices, tray.all(), n)
}

/// Whether a Mute in `choices` is for the program sending `n`: one of `items` under such a choice,
/// or the choice itself, runs as the same app, or, where one side knows no app, has its program
/// key or (an item only) its process.
pub(super) fn muted<'a>(
    choices: &BTreeMap<String, Remembered>,
    items: impl IntoIterator<Item = &'a TrayItem>,
    n: &Notification,
) -> bool {
    let sends = |app: Option<&str>, key: &str, pid: Option<i32>| match (app, n.app_id.as_deref()) {
        (Some(a), Some(b)) => a == b,
        _ => key == n.key || pid.is_some_and(|p| n.pid == Some(p)),
    };
    let mutes = |r: &Remembered| !r.behaviour.balloons();
    let item = |i: &TrayItem| choices.get(&i.key).is_some_and(mutes);
    (items.into_iter()).any(|i| item(i) && sends(i.app_id.as_deref(), &i.key, i.pid))
        || (choices.iter()).any(|(key, r)| mutes(r) && sends(r.app_id.as_deref(), key, None))
}
