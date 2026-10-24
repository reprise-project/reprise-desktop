use super::{TrayItem, behaviour_of, overflow_enabled, replace_all};
use crate::compositor::State;
use crate::icontheme::IconCache;
use reprisekit::controls::shellview;
use reprisekit::controls::traycustomize::{self, Behaviour, Choices, Entry, TrayCustomize};
use reprisekit::icon::IconSource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    pub behaviour: Behaviour,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

fn item_name(item: &TrayItem) -> &str {
    first_line(&[&item.tooltip, &item.title, &item.key])
}

fn first_line<'a>(texts: &[&'a String]) -> &'a str {
    texts
        .iter()
        .find_map(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("")
}

pub fn customize(state: &mut State) {
    crate::trayflyout::close(state);
    let Some(tray) = state.tray.as_ref() else {
        return;
    };
    let theme = &state.config.theme;
    let wording = traycustomize::Wording::of(theme);
    let mut keys = std::collections::HashSet::new();
    let current: Vec<Entry> = tray
        .items
        .iter()
        .chain(&tray.hidden)
        .map(|i| {
            keys.insert(i.key.clone());
            // Glass names the item by its title and puts its tip under
            // that; Classic lists it by its tip.
            let (name, detail) = match wording {
                traycustomize::Wording::Glass => (
                    first_line(&[&i.title, &i.key]).to_string(),
                    first_line(&[&i.tooltip]).to_string(),
                ),
                traycustomize::Wording::Classic => (item_name(i).to_string(), String::new()),
            };
            Entry {
                key: i.key.clone(),
                name,
                detail,
                icon: if i.pixmaps.is_empty() {
                    i.named.clone()
                } else {
                    i.pixmaps.clone()
                },
                passive: i.passive,
                behaviour: behaviour_of(&tray.choices, i),
            }
        })
        .collect();
    let past_keys: Vec<(String, Remembered)> = tray
        .choices
        .iter()
        .filter(|(k, _)| !keys.contains(*k))
        .map(|(k, r)| (k.clone(), r.clone()))
        .collect();
    let all_icons =
        (wording == traycustomize::Wording::Glass).then(|| !overflow_enabled(&state.config));
    let metrics = theme.controls();
    let list = shellview::list_icon_px(metrics.text);
    let past = past_keys
        .into_iter()
        .map(|(key, r)| {
            let icon = past_icon(&mut state.icons, r.icon.as_deref(), list);
            let name = if r.name.is_empty() {
                key.clone()
            } else {
                r.name
            };
            let detail = match wording {
                traycustomize::Wording::Glass => name.clone(),
                traycustomize::Wording::Classic => String::new(),
            };
            Entry {
                name,
                detail,
                key,
                icon: icon.into(),
                passive: false,
                behaviour: r.behaviour,
            }
        })
        .collect();
    let t = TrayCustomize::new(wording, current, past, all_icons, metrics);
    state.open_dialog(crate::dialog::Dialog::tray_customize(t));
}

/// `choices` after the Customize dialog's OK picked `picked`: an Auto
/// pick forgets its key, any other keeps it with the name and icon of the
/// registered item that has it, or what was kept before.
pub(super) fn remember(
    choices: &mut BTreeMap<String, Remembered>,
    picked: &[(String, Behaviour)],
    current: &[&TrayItem],
) {
    for (key, b) in picked {
        if *b == Behaviour::Auto {
            choices.remove(key);
            continue;
        }
        let kept = choices.remove(key);
        let (name, icon) = match current.iter().find(|i| i.key == *key) {
            Some(i) => (item_name(i).to_string(), i.icon_name.clone()),
            None => kept.map_or_else(|| (key.clone(), None), |r| (r.name, r.icon)),
        };
        choices.insert(
            key.clone(),
            Remembered {
                behaviour: *b,
                name,
                icon,
            },
        );
    }
}

pub fn apply_choices(state: &mut State, picked: Choices) {
    if let Some(all) = picked.all_icons
        && all == state.config.taskbar.tray_overflow
    {
        state.config.taskbar.tray_overflow = !all;
        let key = repriseconf::key("taskbar", "tray_overflow");
        crate::configwrite::save_flag(&state.config_path, key, !all);
    }
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    let current: Vec<&TrayItem> = tray.items.iter().chain(&tray.hidden).collect();
    let mut choices = tray.choices.clone();
    remember(&mut choices, &picked.behaviours, &current);
    if choices != tray.choices {
        crate::state::save_tray_icons(&choices);
        tray.choices = choices;
    }
    replace_all(state);
}

/// Icon file `file` landed: the remembered rows of an open Customize dialog that wear it take it.
pub(crate) fn customize_icon_landed(state: &mut State, file: &str) {
    let Some(tray) = state.tray.as_ref() else {
        return;
    };
    let mut refilled = Vec::new();
    for (i, v) in state.dialogs.views.iter_mut().enumerate() {
        if let Some(t) = v.dialog.tray_mut()
            && refill_past(t, &tray.choices, &mut state.icons, file)
        {
            refilled.push(i);
        }
    }
    for i in refilled {
        state.redraw_dialog(i);
    }
}

/// Gives `t`'s remembered rows whose kept icon is `file` their icon again; true if any did.
pub(super) fn refill_past(
    t: &mut TrayCustomize,
    choices: &BTreeMap<String, Remembered>,
    icons: &mut IconCache,
    file: &str,
) -> bool {
    let current = t.current;
    let list = shellview::list_icon_px(t.metrics.text);
    let mut refilled = false;
    for e in &mut t.entries[current..] {
        if choices.get(&e.key).and_then(|r| r.icon.as_deref()) == Some(file) {
            e.icon = past_icon(icons, Some(file), list).into();
            refilled = true;
        }
    }
    refilled
}

/// A remembered item's icon for the Customize list. A file is never read on the loop: one not
/// decoded yet shows the stand-in until [`customize_icon_landed`] refills the row. It is
/// fetched at 16 and 32 px and at the list's icon px `list` and twice that, for scale 2.
pub(super) fn past_icon(icons: &mut IconCache, name: Option<&str>, list: i32) -> Vec<IconSource> {
    let name = name.unwrap_or(crate::icon::STAND_IN);
    let mut sizes = vec![16, 32, list, 2 * list];
    sizes.sort_unstable();
    sizes.dedup();
    sizes
        .into_iter()
        .filter_map(|px| crate::icon::unread(icons, name, px))
        .map(|c| IconSource {
            width: c.width,
            height: c.height,
            pixels: c.pixels.clone(),
        })
        .collect()
}
