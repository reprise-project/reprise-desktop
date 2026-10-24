//! Menu popups: layout, drawing and open-chain behaviour for context menus, submenus, tray menus and
//! the root Start menu. The host draws the returned canvas and feeds `hit` / `step_selection` / `enter_row`.

use crate::raster::Canvas;
use crate::text::split_mnemonic;
use crate::theme::Theme;

mod access;
mod draw;
mod layout;
mod popup;
mod shadow;

pub use access::{access_menu, access_row};
pub use draw::{BULLET_D, draw_menu, draw_menu_at};
pub use layout::{
    ARROW_RIGHT_INSET, BULLET_X, CHECK_X, ContextBitmaps, MenuLayout, MenuStyle, place_popup,
    place_submenu, place_submenu_inset, relist, submenu_max_rows,
};
pub use popup::{Level, MenuAction, MenuCtx, MenuKey, MenuTree, PopupMenu};
pub use shadow::{
    draw_menu_shadow, draw_shadow, draw_shadow_above, draws_own_shadow, level_shadow, shadow_reach,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Entry {
        label: String,
        icon: Option<Canvas>,
        /// Drawn in `gray_text` and never hovered when false (tray menus).
        enabled: bool,
        checked: bool,
        radio: bool,
        submenu: bool,
        mnemonic: Option<usize>,
        shortcut: Option<String>,
        /// Bold label: the menu's default row (the window menu's Close or
        /// Maximize, whichever a double-click would do).
        bold: bool,
        /// Caption-button glyph in the gutter, drawn in the text color so it
        /// inverts with the highlight; used instead of an icon.
        glyph: Option<MenuGlyph>,
    },
    Separator,
}

pub use crate::art::glyph::Glyph as MenuGlyph;

impl MenuItem {
    pub fn entry(label: impl Into<String>) -> MenuItem {
        MenuItem::Entry {
            label: label.into(),
            icon: None,
            enabled: true,
            checked: false,
            radio: false,
            submenu: false,
            mnemonic: None,
            shortcut: None,
            bold: false,
            glyph: None,
        }
    }

    pub fn item(label: &str) -> MenuItem {
        let (label, mnemonic) = split_mnemonic(label);
        MenuItem::entry(label).mnemonic(mnemonic)
    }

    pub fn icon(mut self, icon: Option<Canvas>) -> MenuItem {
        if let MenuItem::Entry { icon: i, .. } = &mut self {
            *i = icon;
        }
        self
    }
    pub fn enabled(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
    pub fn checked(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
    pub fn radio(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { radio, .. } = &mut self {
            *radio = on;
        }
        self
    }
    pub fn submenu(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { submenu, .. } = &mut self {
            *submenu = on;
        }
        self
    }
    pub fn mnemonic(mut self, at: Option<usize>) -> MenuItem {
        if let MenuItem::Entry { mnemonic, .. } = &mut self {
            *mnemonic = at;
        }
        self
    }
    pub fn shortcut(mut self, text: Option<String>) -> MenuItem {
        if let MenuItem::Entry { shortcut, .. } = &mut self {
            *shortcut = text;
        }
        self
    }
    pub fn bold(mut self, on: bool) -> MenuItem {
        if let MenuItem::Entry { bold, .. } = &mut self {
            *bold = on;
        }
        self
    }
    pub fn glyph(mut self, g: Option<MenuGlyph>) -> MenuItem {
        if let MenuItem::Entry { glyph, .. } = &mut self {
            *glyph = g;
        }
        self
    }

    pub fn access_key(&self) -> Option<char> {
        let MenuItem::Entry {
            label,
            enabled: true,
            mnemonic,
            ..
        } = self
        else {
            return None;
        };
        let ch = match mnemonic {
            Some(i) => label.chars().nth(*i),
            None => label.chars().find(|c| c.is_alphanumeric()),
        };
        ch.and_then(|c| c.to_lowercase().next())
    }

    fn selectable(&self) -> bool {
        matches!(self, MenuItem::Entry { enabled: true, .. })
    }
}

/// `label` with `{user}` replaced by `user`, so a config row can read
/// "Log Off {user}...".
pub fn expand_label(label: &str, user: &str) -> String {
    label.replace("{user}", user)
}

/// Which rows the arrow keys stop on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStops {
    /// Enabled entries only: greyed rows are passed over.
    Enabled,
    /// Every entry, greyed ones too, as the pointer lights them; Enter on a greyed row does nothing.
    Entries,
}

impl KeyStops {
    /// A look with its own menus (Glass) stops on greyed rows; Classic and Blue pass over them.
    pub fn of(theme: &Theme) -> KeyStops {
        if theme.look.style().is_some_and(|st| st.menu.is_some()) {
            KeyStops::Entries
        } else {
            KeyStops::Enabled
        }
    }
}

pub fn step_selection(
    items: &[MenuItem],
    current: Option<usize>,
    step: i32,
    stops: KeyStops,
) -> Option<usize> {
    let n = items.len() as i32;
    if n == 0 {
        return None;
    }
    let start = match current {
        Some(i) => i as i32,
        None if step > 0 => -1,
        None => n,
    };
    (1..=n)
        .map(|k| (start + k * step).rem_euclid(n) as usize)
        .find(|&i| match stops {
            KeyStops::Enabled => items[i].selectable(),
            KeyStops::Entries => matches!(items[i], MenuItem::Entry { .. }),
        })
}

pub fn default_row(items: &[MenuItem]) -> Option<usize> {
    items.iter().position(|it| {
        matches!(
            it,
            MenuItem::Entry {
                bold: true,
                enabled: true,
                ..
            }
        )
    })
}

pub fn enter_row(items: &[MenuItem], current: Option<usize>) -> Option<usize> {
    current.or_else(|| default_row(items))
}

pub fn rows_for_key(items: &[MenuItem], key: char) -> impl Iterator<Item = usize> + '_ {
    let key = key.to_lowercase().next().unwrap_or(key);
    items
        .iter()
        .enumerate()
        .filter(move |(_, it)| it.access_key() == Some(key))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests;
