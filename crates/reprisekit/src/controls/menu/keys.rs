//! What a key does to an open menu, for every host: the compositor's menus and the programs'.

use super::{KeyStops, MenuItem, enter_row, rows_for_key, step_selection};
use crate::textedit::keysym;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    Char(char),
}

impl MenuKey {
    /// The menu key in keysym `sym` typing `utf8`: the arrows, Home, End and Enter with their
    /// keypad twins, Space as Enter, Escape, and a letter or digit as an access key.
    pub fn from_keysym(sym: u32, utf8: Option<&str>) -> Option<MenuKey> {
        Some(match sym {
            keysym::UP | keysym::KP_UP => MenuKey::Up,
            keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
            keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
            keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
            keysym::HOME | keysym::KP_HOME => MenuKey::Home,
            keysym::END | keysym::KP_END => MenuKey::End,
            keysym::RETURN | keysym::KP_ENTER | keysym::SPACE => MenuKey::Enter,
            keysym::ESCAPE => MenuKey::Escape,
            _ => MenuKey::Char(
                utf8.and_then(|s| s.chars().next())
                    .filter(|c| c.is_alphanumeric())?,
            ),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAct {
    Nothing,
    Select(Option<usize>),
    SelectAnd(usize, Box<KeyAct>),
    CloseTopmost,
    CloseSubmenu,
    OpenSubmenu(usize),
    Activate(usize),
}

pub fn row_has_submenu(items: &[MenuItem], row: usize) -> bool {
    matches!(
        items.get(row),
        Some(MenuItem::Entry {
            submenu: true,
            enabled: true,
            ..
        })
    )
}

/// What `key` does to the topmost menu, whose rows are `items` with `current` highlighted:
/// Up/Down move, Home/End go to the ends, Right/Enter open a submenu (Enter runs a plain row or
/// the bold default), Left/Escape close, a letter runs its access key when one row has it and
/// moves to the next such row when several do.
pub fn key_act(
    items: &[MenuItem],
    current: Option<usize>,
    key: MenuKey,
    stops: KeyStops,
) -> KeyAct {
    let open_or_run = |row: usize| {
        if row_has_submenu(items, row) {
            KeyAct::OpenSubmenu(row)
        } else {
            KeyAct::Activate(row)
        }
    };
    match key {
        MenuKey::Down => KeyAct::Select(step_selection(items, current, 1, stops)),
        MenuKey::Up => KeyAct::Select(step_selection(items, current, -1, stops)),
        MenuKey::Home => KeyAct::Select(step_selection(items, None, 1, stops)),
        MenuKey::End => KeyAct::Select(step_selection(items, None, -1, stops)),
        MenuKey::Escape => KeyAct::CloseTopmost,
        MenuKey::Left => KeyAct::CloseSubmenu,
        MenuKey::Right => match current.filter(|&r| row_has_submenu(items, r)) {
            Some(row) => KeyAct::OpenSubmenu(row),
            None => KeyAct::Nothing,
        },
        MenuKey::Enter => match (current, enter_row(items, current)) {
            (_, None) => KeyAct::Nothing,
            (Some(row), _) => open_or_run(row),
            (None, Some(row)) => KeyAct::SelectAnd(row, Box::new(open_or_run(row))),
        },
        MenuKey::Char(ch) => {
            let rows: Vec<usize> = rows_for_key(items, ch).collect();
            match rows.as_slice() {
                [] => KeyAct::Nothing,
                [only] => KeyAct::SelectAnd(*only, Box::new(open_or_run(*only))),
                many => {
                    let next = many
                        .iter()
                        .find(|&&r| Some(r) > current)
                        .or(many.first())
                        .copied();
                    KeyAct::Select(next)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
