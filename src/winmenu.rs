//! The window (system) menu, shown on Alt+Space, the caption icon, and a
//! right click of the title bar or taskbar button. Pure row logic; the
//! compositor opens it and acts on the picked row.

use crate::startmenu::{MenuGlyph, MenuItem};
use crate::text::split_mnemonic;
use reprisekit::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMenuRow {
    Restore,
    Move,
    Size,
    Minimize,
    Maximize,
    /// Rolls the window up to its caption, or back down.
    Shade,
    AlwaysOnTop,
    /// Writes or removes the window rule that opens the program's windows where this one is.
    OpenHere,
    /// Opens or closes the window's mini view (`miniview`).
    MiniView,
    AllDesktops,
    /// Opens `move_to_menu`, does nothing itself.
    MoveToMenu,
    MoveTo(usize),
    /// Opens `move_to_screen_menu`, does nothing itself.
    ScreenMenu,
    /// Carries the window to the output at this index of the scene's outputs.
    MoveToScreen(usize),
    /// Opens `tabs_of_menu`, does nothing itself.
    TabsOfMenu,
    TabsOf(crate::window::WindowId),
    LeaveTabs,
    Close,
    /// Opens the program's recent files (`State::recent_menu`), does nothing itself.
    RecentMenu,
}

impl WindowMenuRow {
    pub fn opens_submenu(self) -> bool {
        matches!(
            self,
            Self::MoveToMenu | Self::ScreenMenu | Self::TabsOfMenu | Self::RecentMenu
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabRows {
    /// Another window could take this one as a tab: Add to Tabs of opens a submenu of them.
    pub targets: bool,
    pub grouped: bool,
}

/// The Always Open Here row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenHereState {
    /// A dialog: no row.
    Absent,
    /// No app id a rule could match.
    Greyed,
    Off,
    /// A rule Always Open Here wrote matches the program.
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowMenuState {
    pub maximized: bool,
    pub minimized: bool,
    /// Snapped into a zone: Restore gives back its size from before the snap.
    pub snapped: bool,
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
    pub open_here: OpenHereState,
    /// Mini View is checked: the window has one open.
    pub mini: bool,
    pub desktops: usize,
    /// Show on All Desktops is checked and Move to Desktop greyed.
    pub sticky: bool,
    /// The outputs a window can be carried to; Move to Screen shows with two or more.
    pub screens: usize,
    /// The tab group rows, after the desktop rows; None for a window that cannot be a tab.
    pub tabs: Option<TabRows>,
    /// Whether the window is rolled up, for the Roll Up/Roll Down row after Maximize; None for a
    /// window that cannot roll up at all (no frame of ours, or in a tab group).
    pub shade: Option<bool>,
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
            tr!("start-win-restore"),
            (!normal || state.snapped) && !state.owned,
            Some(MenuGlyph::Restore),
        ),
        (Move, tr!("start-win-move"), normal, None),
        (
            Size,
            tr!("start-win-size"),
            normal && state.resizable && !state.owned && state.shade != Some(true),
            None,
        ),
        (
            Minimize,
            tr!("start-win-minimize"),
            !state.minimized && !state.owned,
            Some(MenuGlyph::Minimize),
        ),
        (
            Maximize,
            tr!("start-win-maximize"),
            !state.maximized && !state.owned && state.resizable,
            Some(MenuGlyph::Maximize),
        ),
    ];
    let shade = state.shade.map(|shaded| {
        let label = if shaded {
            tr!("start-win-roll-down")
        } else {
            tr!("start-win-roll-up")
        };
        (Shade, label, shaded || normal, None, None)
    });
    let mut group = vec![(
        AlwaysOnTop,
        tr!("start-win-always-on-top"),
        !state.owned,
        None,
        None,
    )];
    if state.open_here != OpenHereState::Absent {
        let enabled = state.open_here != OpenHereState::Greyed;
        group.push((OpenHere, tr!("start-win-open-here"), enabled, None, None));
    }
    group.push((MiniView, tr!("start-win-mini-view"), true, None, None));
    if state.desktops > 1 {
        group.push((
            AllDesktops,
            tr!("start-win-all-desktops"),
            !state.owned,
            None,
            None,
        ));
        group.push((
            MoveToMenu,
            tr!("start-win-move-to-desktop"),
            !state.owned && !state.sticky,
            None,
            None,
        ));
    }
    if state.screens > 1 {
        group.push((
            ScreenMenu,
            tr!("start-win-move-to-screen"),
            true,
            None,
            None,
        ));
    }
    if let Some(t) = state.tabs {
        group.push((
            TabsOfMenu,
            tr!("start-win-add-to-tabs"),
            t.targets,
            None,
            None,
        ));
        group.push((
            LeaveTabs,
            tr!("start-win-leave-tabs"),
            t.grouped,
            None,
            None,
        ));
    }
    let close = (
        Close,
        tr!("start-win-close"),
        true,
        Some(MenuGlyph::Close),
        close_keys.map(str::to_string),
    );
    let mut items = Vec::with_capacity(11);
    let mut actions = Vec::with_capacity(11);
    for (row, label, enabled, glyph, shortcut) in rows
        .into_iter()
        .map(|(row, label, enabled, glyph)| (row, label, enabled, glyph, None))
        .chain(shade)
        .chain(group)
        .chain(std::iter::once(close))
    {
        if matches!(row, AlwaysOnTop | Close) {
            items.push(MenuItem::Separator);
            actions.push(None);
        }
        let (label, mnemonic) = split_mnemonic(&label);
        items.push(
            MenuItem::entry(label)
                .enabled(enabled)
                .checked(
                    (row == AlwaysOnTop && state.on_top)
                        || (row == OpenHere && state.open_here == OpenHereState::On)
                        || (row == MiniView && state.mini)
                        || (row == AllDesktops && state.sticky),
                )
                .submenu(row.opens_submenu())
                .mnemonic(mnemonic)
                .bold(row == default)
                .shortcut(shortcut)
                .glyph(glyph),
        );
        actions.push(Some(row));
    }
    (items, actions)
}

/// A task button's window menu `items` and `rows` with Recent Files, the submenu of the
/// program's recent files, on top.
pub fn with_recent(
    (items, rows): (Vec<MenuItem>, Vec<Option<WindowMenuRow>>),
) -> (Vec<MenuItem>, Vec<Option<WindowMenuRow>>) {
    let head = [
        MenuItem::item(&recent_files()).submenu(true),
        MenuItem::Separator,
    ];
    let items = head.into_iter().chain(items).collect();
    let rows = [Some(WindowMenuRow::RecentMenu), None]
        .into_iter()
        .chain(rows)
        .collect();
    (items, rows)
}

/// The window menu's and a group's window list's row of the program's recent files.
pub fn recent_files() -> String {
    tr!("start-win-recent-files")
}

/// The Move to Desktop submenu of windows on desktop `own`: every other of
/// the `count` desktops by its name in `names`, each doing `row(n)`. Shared with the jump list.
pub fn move_to_menu<R>(
    count: usize,
    own: usize,
    names: &[String],
    row: impl Fn(usize) -> R,
) -> (Vec<MenuItem>, Vec<Option<R>>) {
    (0..count)
        .filter(|&n| n != own)
        .map(|n| {
            let named = names.get(n).is_some_and(|s| !s.trim().is_empty());
            let item = if named {
                MenuItem::entry(crate::config::desktop_name(names, n))
            } else {
                let number = (n + 1).to_string();
                let label = if n < 9 {
                    tr!("start-win-desktop-keyed", number = number)
                } else {
                    tr!("start-win-desktop", number = number)
                };
                let (label, mnemonic) = split_mnemonic(&label);
                MenuItem::entry(label).mnemonic(mnemonic)
            };
            (item, Some(row(n)))
        })
        .unzip()
}

/// The Move to Screen submenu: a row per output in `screens` (its index in the scene's outputs and
/// its label), each carrying the window there.
pub fn move_to_screen_menu(
    screens: Vec<(usize, String)>,
) -> (Vec<MenuItem>, Vec<Option<WindowMenuRow>>) {
    screens
        .into_iter()
        .map(|(i, label)| (MenuItem::entry(label), Some(WindowMenuRow::MoveToScreen(i))))
        .unzip()
}

/// Window title `title` cut at 48 characters, as a menu row or a tip lists it.
pub fn short_title(title: String) -> String {
    match title.char_indices().nth(48) {
        Some((i, _)) => format!("{}...", &title[..i]),
        None => title,
    }
}

/// A row per window in `targets`, titled ([`short_title`]) with its icon, each joining that window's group.
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
            (
                MenuItem::entry(short_title(title)).icon(icon),
                Some(WindowMenuRow::TabsOf(w)),
            )
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(items: &[MenuItem], i: usize) -> (&str, bool, bool) {
        match &items[i] {
            MenuItem::Entry {
                label,
                enabled,
                bold,
                ..
            } => (label, *enabled, *bold),
            MenuItem::Separator | MenuItem::Heading(_) => ("-", false, false),
        }
    }

    #[test]
    fn normal_window_from_caption_defaults_to_close() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let (items, actions) = window_menu(st, Some("Alt+F4"));
        assert_eq!(items.len(), 10);
        assert_eq!(actions[5], None);
        assert_eq!(actions[8], None);
        assert_eq!(row(&items, 0), ("Restore", false, false));
        let (snapped, _) = window_menu(
            WindowMenuState {
                snapped: true,
                ..st
            },
            None,
        );
        assert_eq!(
            row(&snapped, 0),
            ("Restore", true, false),
            "a snapped window restores"
        );
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", true, false));
        assert_eq!(row(&items, 3), ("Minimize", true, false));
        assert_eq!(row(&items, 4), ("Maximize", true, false));
        assert_eq!(row(&items, 9), ("Close", true, true));
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(actions[6], Some(WindowMenuRow::AlwaysOnTop));
        assert_eq!(row(&items, 7), ("Mini View", true, false));
        assert_eq!(actions[7], Some(WindowMenuRow::MiniView));
        let (with_mini, _) = window_menu(WindowMenuState { mini: true, ..st }, None);
        assert!(matches!(
            &with_mini[7],
            MenuItem::Entry { checked: true, .. }
        ));
        assert!(matches!(
            &items[6],
            MenuItem::Entry {
                checked: false,
                mnemonic: Some(10),
                ..
            }
        ));
        assert!(matches!(
            &items[9],
            MenuItem::Entry { shortcut: Some(s), mnemonic: Some(0), .. } if s == "Alt+F4"
        ));
        assert_eq!(actions[9], Some(WindowMenuRow::Close));
    }

    #[test]
    fn maximized_window_from_taskbar_defaults_to_restore() {
        let st = WindowMenuState {
            maximized: true,
            minimized: false,
            snapped: false,
            from_taskbar: true,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let (items, _) = window_menu(st, None);
        assert_eq!(row(&items, 0), ("Restore", true, true));
        assert_eq!(row(&items, 1), ("Move", false, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 3), ("Minimize", true, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 9), ("Close", true, false));
    }

    #[test]
    fn minimized_window_can_restore_or_maximize_but_not_minimize() {
        let st = WindowMenuState {
            maximized: false,
            minimized: true,
            snapped: false,
            from_taskbar: true,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
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
            snapped: false,
            from_taskbar: false,
            owned: true,
            resizable: false,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        assert_eq!(row(&items, 0), ("Restore", false, false));
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 3), ("Minimize", false, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 6), ("Always on Top", false, false));
        assert_eq!(row(&items, 9), ("Close", true, true));
    }

    #[test]
    fn an_on_top_window_checks_always_on_top() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: true,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
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
    fn always_open_here_follows_always_on_top_checked_greyed_or_absent() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Off,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let checked =
            |items: &[MenuItem]| matches!(&items[7], MenuItem::Entry { checked: true, .. });
        let (items, actions) = window_menu(st, None);
        assert_eq!(items.len(), 11);
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(row(&items, 7), ("Always Open Here", true, false));
        assert_eq!(actions[7], Some(WindowMenuRow::OpenHere));
        assert_eq!(row(&items, 8), ("Mini View", true, false));
        assert!(!checked(&items));
        assert!(matches!(
            &items[7],
            MenuItem::Entry {
                mnemonic: Some(7),
                ..
            }
        ));
        let on = WindowMenuState {
            open_here: OpenHereState::On,
            ..st
        };
        assert!(checked(&window_menu(on, None).0));
        let greyed = WindowMenuState {
            open_here: OpenHereState::Greyed,
            ..st
        };
        assert_eq!(
            row(&window_menu(greyed, None).0, 7),
            ("Always Open Here", false, false)
        );
        let dialog = WindowMenuState {
            open_here: OpenHereState::Absent,
            ..st
        };
        let (items, actions) = window_menu(dialog, None);
        assert_eq!(items.len(), 10);
        assert!(!actions.contains(&Some(WindowMenuRow::OpenHere)));
    }

    #[test]
    fn several_desktops_add_the_desktop_rows_after_always_on_top() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 4,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let (items, actions) = window_menu(st, Some("Alt+F4"));
        assert_eq!(items.len(), 12);
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(row(&items, 8), ("Show on All Desktops", true, false));
        assert_eq!(row(&items, 9), ("Move to Desktop", true, false));
        assert_eq!(actions[8], Some(WindowMenuRow::AllDesktops));
        assert_eq!(actions[9], Some(WindowMenuRow::MoveToMenu));
        assert!(matches!(
            &items[9],
            MenuItem::Entry {
                submenu: true,
                checked: false,
                mnemonic: Some(8),
                ..
            }
        ));
        assert_eq!(actions[10], None);
        assert_eq!(row(&items, 11), ("Close", true, true));
        let (items, _) = window_menu(WindowMenuState { sticky: true, ..st }, None);
        assert!(matches!(&items[8], MenuItem::Entry { checked: true, .. }));
        assert_eq!(row(&items, 9), ("Move to Desktop", false, false));
        let (items, _) = window_menu(WindowMenuState { owned: true, ..st }, None);
        assert_eq!(row(&items, 8), ("Show on All Desktops", false, false));
        assert_eq!(row(&items, 9), ("Move to Desktop", false, false));
    }

    #[test]
    fn two_screens_add_move_to_screen_after_the_desktop_rows() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 4,
            sticky: false,
            screens: 2,
            tabs: None,
            shade: None,
        };
        let (items, actions) = window_menu(st, None);
        assert_eq!(row(&items, 9), ("Move to Desktop", true, false));
        assert_eq!(row(&items, 10), ("Move to Screen", true, false));
        assert_eq!(actions[10], Some(WindowMenuRow::ScreenMenu));
        assert!(matches!(
            &items[10],
            MenuItem::Entry {
                submenu: true,
                mnemonic: Some(11),
                ..
            }
        ));
        let (one, _) = window_menu(WindowMenuState { screens: 1, ..st }, None);
        assert_eq!(
            one.len(),
            items.len() - 1,
            "one screen: no Move to Screen row"
        );
        let (items, rows) =
            move_to_screen_menu(vec![(0, "DELL U2415".into()), (2, "HDMI-A-1".into())]);
        assert_eq!(row(&items, 0), ("DELL U2415", true, false));
        assert_eq!(row(&items, 1), ("HDMI-A-1", true, false));
        assert_eq!(
            rows,
            [
                Some(WindowMenuRow::MoveToScreen(0)),
                Some(WindowMenuRow::MoveToScreen(2))
            ]
        );
    }

    #[test]
    fn a_window_that_takes_tabs_gets_the_tab_rows_before_close() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: Some(TabRows {
                targets: true,
                grouped: false,
            }),
            shade: None,
        };
        let (items, actions) = window_menu(st, None);
        assert_eq!(items.len(), 12);
        assert_eq!(row(&items, 6), ("Always on Top", true, false));
        assert_eq!(row(&items, 8), ("Add to Tabs of", true, false));
        assert_eq!(row(&items, 9), ("Remove from Tab Group", false, false));
        assert_eq!(actions[8], Some(WindowMenuRow::TabsOfMenu));
        assert_eq!(actions[9], Some(WindowMenuRow::LeaveTabs));
        assert!(matches!(
            &items[8],
            MenuItem::Entry {
                submenu: true,
                mnemonic: Some(9),
                ..
            }
        ));
        assert!(matches!(
            &items[9],
            MenuItem::Entry {
                mnemonic: Some(16),
                ..
            }
        ));
        assert_eq!(row(&items, 11).0, "Close");
        let tabs = Some(TabRows {
            targets: false,
            grouped: true,
        });
        let (items, _) = window_menu(WindowMenuState { tabs, ..st }, None);
        assert_eq!(row(&items, 8), ("Add to Tabs of", false, false));
        assert_eq!(row(&items, 9), ("Remove from Tab Group", true, false));
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
        let (items, rows) = move_to_menu(4, 1, &[], WindowMenuRow::MoveTo);
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
        let (items, _) = move_to_menu(12, 0, &[], WindowMenuRow::MoveTo);
        assert_eq!(row(&items, 10).0, "Desktop 12");
        assert!(matches!(&items[10], MenuItem::Entry { mnemonic: None, .. }));
    }

    #[test]
    fn move_to_desktop_names_the_named_desktops() {
        let names = ["Work".to_string(), String::new(), "Chat & Mail".into()];
        let (items, _) = move_to_menu(4, 1, &names, WindowMenuRow::MoveTo);
        let labels: Vec<&str> = (0..items.len()).map(|i| row(&items, i).0).collect();
        assert_eq!(labels, ["Work", "Chat & Mail", "Desktop 4"]);
        assert!(matches!(&items[0], MenuItem::Entry { mnemonic: None, .. }));
        assert!(matches!(
            &items[2],
            MenuItem::Entry {
                mnemonic: Some(8),
                ..
            }
        ));
    }

    #[test]
    fn a_fixed_size_window_cannot_maximize_either() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: false,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
        };
        let (items, _) = window_menu(st, Some("Alt+F4"));
        assert_eq!(row(&items, 1), ("Move", true, false));
        assert_eq!(row(&items, 2), ("Size", false, false));
        assert_eq!(row(&items, 4), ("Maximize", false, false));
        assert_eq!(row(&items, 9), ("Close", true, true));
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
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Off,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: None,
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
    fn a_framed_window_rolls_up_after_maximize_and_a_rolled_one_rolls_down() {
        let st = WindowMenuState {
            maximized: false,
            minimized: false,
            snapped: false,
            from_taskbar: false,
            owned: false,
            resizable: true,
            on_top: false,
            open_here: OpenHereState::Absent,
            mini: false,
            desktops: 1,
            screens: 1,
            sticky: false,
            tabs: None,
            shade: Some(false),
        };
        let (items, actions) = window_menu(st, None);
        assert_eq!(row(&items, 5), ("Roll Up", true, false));
        assert_eq!(actions[5], Some(WindowMenuRow::Shade));
        assert!(matches!(
            &items[5],
            MenuItem::Entry {
                mnemonic: Some(5),
                ..
            }
        ));
        assert_eq!(row(&items, 6), ("-", false, false));

        let (items, _) = window_menu(
            WindowMenuState {
                shade: Some(true),
                ..st
            },
            None,
        );
        assert_eq!(row(&items, 5), ("Roll Down", true, false));
        assert!(matches!(
            &items[5],
            MenuItem::Entry {
                mnemonic: Some(7),
                ..
            }
        ));
        assert_eq!(
            row(&items, 2),
            ("Size", false, false),
            "no sizing while rolled up"
        );
        assert_eq!(
            row(&items, 4),
            ("Maximize", true, false),
            "maximize rolls it down"
        );

        let (items, _) = window_menu(
            WindowMenuState {
                maximized: true,
                ..st
            },
            None,
        );
        assert_eq!(
            row(&items, 5),
            ("Roll Up", false, false),
            "greyed while maximized"
        );
    }
}
