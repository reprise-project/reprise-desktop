//! The window (system) menu, shown on Alt+Space, the caption icon, and a
//! right click of the title bar or taskbar button. Pure row logic; the
//! compositor opens it and acts on the picked row.

use crate::startmenu::{MenuGlyph, MenuItem};
use crate::text::split_mnemonic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMenuRow {
    Restore,
    Move,
    Size,
    Minimize,
    Maximize,
    AlwaysOnTop,
    AllDesktops,
    /// Opens `move_to_menu`, does nothing itself.
    MoveToMenu,
    MoveTo(usize),
    /// Opens `tabs_of_menu`, does nothing itself.
    TabsOfMenu,
    TabsOf(crate::window::WindowId),
    LeaveTabs,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabRows {
    /// Another window could take this one as a tab: Add to Tabs of opens a submenu of them.
    pub targets: bool,
    pub grouped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowMenuState {
    pub maximized: bool,
    pub minimized: bool,
    /// Opened from the taskbar button: the bold default row is then Maximize
    /// (or Restore); from the caption it is Close.
    pub from_taskbar: bool,
    /// The client set a parent or tagged a help dialog: Restore, Size, Minimize
    /// and Maximize grey out regardless of state.
    pub owned: bool,
    /// A fixed-size window greys out Size and Maximize.
    pub resizable: bool,
    /// Always on Top is checked. An owned window shows its owner's state greyed.
    pub on_top: bool,
    pub desktops: usize,
    /// Show on All Desktops is checked and Move to Desktop greyed.
    pub sticky: bool,
    /// The tab group rows, after the desktop rows; None for a window that cannot be a tab.
    pub tabs: Option<TabRows>,
}

/// `close_keys` is the accelerator shown after Close ("Alt+F4"). Returns the
/// drawable rows and, parallel to them, what each does (None at the separator).
pub fn window_menu(
    state: WindowMenuState,
    close_keys: Option<&str>,
) -> (Vec<MenuItem>, Vec<Option<WindowMenuRow>>) {
    use WindowMenuRow::*;
    let normal = !state.maximized && !state.minimized;
    let default = if !state.from_taskbar {
        Close
    } else if normal {
        Maximize
    } else {
        Restore
    };
    let rows = [
        (
            Restore,
            "&Restore",
            !normal && !state.owned,
            Some(MenuGlyph::Restore),
        ),
        (Move, "&Move", normal, None),
        (
            Size,
            "&Size",
            normal && state.resizable && !state.owned,
            None,
        ),
        (
            Minimize,
            "Mi&nimize",
            !state.minimized && !state.owned,
            Some(MenuGlyph::Minimize),
        ),
        (
            Maximize,
            "Ma&ximize",
            !state.maximized && !state.owned && state.resizable,
            Some(MenuGlyph::Maximize),
        ),
    ];
    let mut group = vec![(AlwaysOnTop, "Always on &Top", !state.owned, None, None)];
    if state.desktops > 1 {
        group.push((
            AllDesktops,
            "Show on &All Desktops",
            !state.owned,
            None,
            None,
        ));
        group.push((
            MoveToMenu,
            "Move to &Desktop",
            !state.owned && !state.sticky,
            None,
            None,
        ));
    }
    if let Some(t) = state.tabs {
        group.push((TabsOfMenu, "Add to Ta&bs of", t.targets, None, None));
        group.push((LeaveTabs, "Remove from Tab &Group", t.grouped, None, None));
    }
    let close = (
        Close,
        "&Close",
        true,
        Some(MenuGlyph::Close),
        close_keys.map(str::to_string),
    );
    let mut items = Vec::with_capacity(11);
    let mut actions = Vec::with_capacity(11);
    for (row, label, enabled, glyph, shortcut) in rows
        .into_iter()
        .map(|(row, label, enabled, glyph)| (row, label, enabled, glyph, None))
        .chain(group)
        .chain(std::iter::once(close))
    {
        if matches!(row, AlwaysOnTop | Close) {
            items.push(MenuItem::Separator);
            actions.push(None);
        }
        let (label, mnemonic) = split_mnemonic(label);
        items.push(
            MenuItem::entry(label)
                .enabled(enabled)
                .checked(
                    (row == AlwaysOnTop && state.on_top) || (row == AllDesktops && state.sticky),
                )
                .submenu(matches!(row, MoveToMenu | TabsOfMenu))
                .mnemonic(mnemonic)
                .bold(row == default)
                .shortcut(shortcut)
                .glyph(glyph),
        );
        actions.push(Some(row));
    }
    (items, actions)
}

/// The Move to Desktop submenu of windows on desktop `own`: every other of
/// the `count` desktops, each doing `row(n)`. Shared with the jump list.
pub fn move_to_menu<R>(
    count: usize,
    own: usize,
    row: impl Fn(usize) -> R,
) -> (Vec<MenuItem>, Vec<Option<R>>) {
    (0..count)
        .filter(|&n| n != own)
        .map(|n| {
            let label = if n < 9 {
                format!("Desktop &{}", n + 1)
            } else {
                format!("Desktop {}", n + 1)
            };
            let (label, mnemonic) = split_mnemonic(&label);
            (MenuItem::entry(label).mnemonic(mnemonic), Some(row(n)))
        })
        .unzip()
}

/// A row per window in `targets`, titled (cut at 48 characters) with its icon, each joining that window's group.
pub fn tabs_of_menu(
    targets: Vec<(
        crate::window::WindowId,
        String,
        Option<crate::raster::Canvas>,
    )>,
) -> (Vec<MenuItem>, Vec<Option<WindowMenuRow>>) {
    targets
        .into_iter()
        .map(|(w, title, icon)| {
            let label = match title.char_indices().nth(48) {
                Some((i, _)) => format!("{}...", &title[..i]),
                None => title,
            };
            (
                MenuItem::entry(label).icon(icon),
                Some(WindowMenuRow::TabsOf(w)),
            )
        })
        .unzip()
}

pub fn keybind_label(mods: u32, keysym: u32) -> String {
    use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};
    let mut out = String::new();
    for (bit, name) in [
        (MOD_CTRL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
        (MOD_LOGO, "Super"),
    ] {
        if mods & bit != 0 {
            out.push_str(name);
            out.push('+');
        }
    }
    let name = keysym_name(keysym);
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => out.extend(c.to_uppercase()),
        _ => out.push_str(&name),
    }
    out
}

pub fn keysym_name(keysym: u32) -> String {
    use smithay::input::keyboard::{Keysym, xkb};
    xkb::keysym_get_name(Keysym::new(keysym))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KEY_F4, KEY_c, KEY_q, MOD_ALT, MOD_CTRL, MOD_LOGO};

    fn row(items: &[MenuItem], i: usize) -> (&str, bool, bool) {
        match &items[i] {
            MenuItem::Entry {
                label,
                enabled,
                bold,
                ..
            } => (label, *enabled, *bold),
            MenuItem::Separator => ("-", false, false),
        }
    }

    #[test]
    fn normal_window_from_caption_defaults_to_close() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, actions) = window_menu(st, Some("Alt+F4"));
        assert_eq!(items.len(), 9);
        assert_eq!(actions[5], None);
        assert_eq!(actions[7], None);
        assert_eq!(row(&items, 0), ("Restore", false, false));
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", true, false));
        assert_eq!(row(&items, 3), ("Minimize", true, false));
        assert_eq!(row(&items, 4), ("Maximize", true, false));
        assert_eq!(row(&items, 8), ("Close", true, true));
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(actions[6], Some(WindowMenuRow::AlwaysOnTop));
        assert!(matches!(
            &items[6],
            MenuItem::Entry {
                checked: false,
                mnemonic: Some(10),
                ..
            }
        ));
        assert!(matches!(
            &items[8],
            MenuItem::Entry { shortcut: Some(s), mnemonic: Some(0), .. } if s == "Alt+F4"
        ));
        assert_eq!(actions[8], Some(WindowMenuRow::Close));
    }

    #[test]
    fn maximized_window_from_taskbar_defaults_to_restore() {
        let st = WindowMenuState {
            maximized: true,
            minimized: false,
            from_taskbar: true,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, None);
        assert_eq!(row(&items, 0), ("Restore", true, true));
        assert_eq!(row(&items, 1), ("Move", false, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 3), ("Minimize", true, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 8), ("Close", true, false));
    }

    #[test]
    fn minimized_window_can_restore_or_maximize_but_not_minimize() {
        let st = WindowMenuState {
            maximized: false,
            minimized: true,
            from_taskbar: true,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, None);
        assert_eq!(row(&items, 0), ("Restore", true, true));
        assert_eq!(row(&items, 3), ("Minimize", false, false));
        assert_eq!(row(&items, 4), ("Maximize", true, false));
    }

    #[test]
    fn a_dialog_menu_offers_move_and_close_only() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: true,
            resizable: false,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        assert_eq!(row(&items, 0), ("Restore", false, false));
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 3), ("Minimize", false, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 6), ("Always on Top", false, false));
        assert_eq!(row(&items, 8), ("Close", true, true));
    }

    #[test]
    fn an_on_top_window_checks_always_on_top() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: true,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        assert!(matches!(
            &items[6],
            MenuItem::Entry {
                checked: true,
                enabled: true,
                ..
            }
        ));
    }

    #[test]
    fn several_desktops_add_the_desktop_rows_after_always_on_top() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 4,
            sticky: false,
            tabs: None,
        };
        let (items, actions) = window_menu(st, Some("Alt+F4"));
        assert_eq!(items.len(), 11);
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(row(&items, 7), ("Show on All Desktops", true, false));
        assert_eq!(row(&items, 8), ("Move to Desktop", true, false));
        assert_eq!(actions[7], Some(WindowMenuRow::AllDesktops));
        assert_eq!(actions[8], Some(WindowMenuRow::MoveToMenu));
        assert!(matches!(
            &items[8],
            MenuItem::Entry {
                submenu: true,
                checked: false,
                mnemonic: Some(8),
                ..
            }
        ));
        assert_eq!(actions[9], None);
        assert_eq!(row(&items, 10), ("Close", true, true));
        let (items, _) = window_menu(WindowMenuState { sticky: true, ..st }, None);
        assert!(matches!(&items[7], MenuItem::Entry { checked: true, .. }));
        assert_eq!(row(&items, 8), ("Move to Desktop", false, false));
        let (items, _) = window_menu(WindowMenuState { owned: true, ..st }, None);
        assert_eq!(row(&items, 7), ("Show on All Desktops", false, false));
        assert_eq!(row(&items, 8), ("Move to Desktop", false, false));
    }

    #[test]
    fn a_window_that_takes_tabs_gets_the_tab_rows_before_close() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: Some(TabRows {
                targets: true,
                grouped: false,
            }),
        };
        let (items, actions) = window_menu(st, None);
        assert_eq!(items.len(), 11);
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(row(&items, 7), ("Add to Tabs of", true, false));
        assert_eq!(row(&items, 8), ("Remove from Tab Group", false, false));
        assert_eq!(actions[7], Some(WindowMenuRow::TabsOfMenu));
        assert_eq!(actions[8], Some(WindowMenuRow::LeaveTabs));
        assert!(matches!(
            &items[7],
            MenuItem::Entry {
                submenu: true,
                mnemonic: Some(9),
                ..
            }
        ));
        assert!(matches!(
            &items[8],
            MenuItem::Entry {
                mnemonic: Some(16),
                ..
            }
        ));
        assert_eq!(row(&items, 10).0, "Close");
        let tabs = Some(TabRows {
            targets: false,
            grouped: true,
        });
        let (items, _) = window_menu(WindowMenuState { tabs, ..st }, None);
        assert_eq!(row(&items, 7), ("Add to Tabs of", false, false));
        assert_eq!(row(&items, 8), ("Remove from Tab Group", true, false));
    }

    #[test]
    fn tabs_of_lists_each_window_by_its_title() {
        let (a, b) = (
            crate::window::WindowId::next(),
            crate::window::WindowId::next(),
        );
        let long = "x".repeat(60);
        let (items, rows) = tabs_of_menu(vec![(a, "Notes".into(), None), (b, long, None)]);
        assert_eq!(row(&items, 0).0, "Notes");
        assert_eq!(row(&items, 1).0, format!("{}...", "x".repeat(48)));
        assert_eq!(
            rows,
            [
                Some(WindowMenuRow::TabsOf(a)),
                Some(WindowMenuRow::TabsOf(b))
            ]
        );
    }

    #[test]
    fn move_to_desktop_lists_the_other_desktops() {
        let (items, rows) = move_to_menu(4, 1, WindowMenuRow::MoveTo);
        let labels: Vec<&str> = (0..items.len()).map(|i| row(&items, i).0).collect();
        assert_eq!(labels, ["Desktop 1", "Desktop 3", "Desktop 4"]);
        assert_eq!(
            rows,
            [
                Some(WindowMenuRow::MoveTo(0)),
                Some(WindowMenuRow::MoveTo(2)),
                Some(WindowMenuRow::MoveTo(3))
            ]
        );
        assert!(matches!(
            &items[1],
            MenuItem::Entry {
                mnemonic: Some(8),
                ..
            }
        ));
        let (items, _) = move_to_menu(12, 0, WindowMenuRow::MoveTo);
        assert_eq!(row(&items, 10).0, "Desktop 12");
        assert!(matches!(&items[10], MenuItem::Entry { mnemonic: None, .. }));
    }

    #[test]
    fn a_fixed_size_window_cannot_maximize_either() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: false,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 8), ("Close", true, true));
    }

    /// Writes target/winmenu-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_window_menu() {
        use crate::startmenu::{MenuLayout, MenuStyle, draw_menu};
        use crate::text::TextRenderer;
        use crate::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            desktops: 1,
            sticky: false,
            tabs: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        let style = MenuStyle::context_for(&theme, &items).fit(
            &text,
            &items,
            theme.fonts.size_px,
            i32::MAX,
        );
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = crate::raster::Canvas::new(layout.width as u32, layout.height as u32);
        draw_menu(&mut c, &theme, &text, &layout, &items, Some(0), "", true);
        crate::raster::write_preview(&c, "winmenu-preview.png");
    }

    #[test]
    fn keybind_labels_read_like_accelerators() {
        assert_eq!(keybind_label(MOD_ALT, KEY_F4), "Alt+F4");
        assert_eq!(keybind_label(MOD_CTRL, KEY_q), "Ctrl+Q");
        assert_eq!(keybind_label(MOD_LOGO, KEY_c), "Super+C");
    }
}
