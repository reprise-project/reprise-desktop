//! The Start menu's rows come from the config; the menu widget itself is
//! `reprisekit::controls::menu`, re-exported here.

pub use reprisekit::controls::menu::*;

use crate::compositor::State;
use crate::config::{MenuEntry, StartMenuStyle};
use reprisefs::apps::AppEntry;
use reprisescene::TreeId;

impl State {
    /// The Start button on bar `bar` was pressed: open or close the Start
    /// menu in the configured style.
    pub fn toggle_start(&mut self, bar: TreeId) {
        match self.config.startmenu_style {
            StartMenuStyle::Classic => self.classic_toggle_start_menu(bar),
            StartMenuStyle::Glass => self.toggle_start_panel(bar),
        }
    }

    /// The Start menu from the keyboard (the Super tap, `start_menu`), in
    /// the configured style.
    pub fn toggle_start_from_keys(&mut self) {
        // Opening: an auto-hidden bar heads out first, so the menu or the
        // Start panel docks on it.
        if self.start_panel.is_none() && self.menus.is_empty() {
            let p = self.pointer_location();
            if let Some(o) = self.output_at(p.x, p.y) {
                crate::autohideview::keys(self, &o, true);
            }
        }
        match self.config.startmenu_style {
            StartMenuStyle::Classic => self.classic_toggle_start_menu_from_keys(),
            StartMenuStyle::Glass => self.toggle_start_panel_from_keys(),
        }
    }

    /// Close the Start menu in the configured style. `reload` calls it
    /// before the new config is swapped in, so it closes the old style's.
    pub fn close_start(&mut self) {
        match self.config.startmenu_style {
            StartMenuStyle::Classic => self.close_menu(),
            StartMenuStyle::Glass => self.close_start_panel(),
        }
    }
}

/// The Glass Start menu's right column, top to bottom: the home folder
/// (labelled with the user's name), the Documents, Pictures and Music
/// folders of `dirs` (the document folders that exist), a rule, Recent Items (with `recent`, `[privacy]
/// recent_files`), Computer, a rule, Control Panel, Printers, Run... and
/// Help and Support. Each row has an icon and an infotip.
pub fn places(recent: bool, dirs: &[(std::path::PathBuf, &str)]) -> Vec<MenuEntry> {
    use crate::config::{CONTROL_PANEL_TIP, HELP, MenuTarget, PRINTERS, PRINTERS_TIP};
    use reprisefs::places::ShellPlace;
    let mut v = vec![
        MenuEntry::spawn("{user}", "user-home", "reprise-explorer \"$HOME\"")
            .tipped(ShellPlace::Home.tip()),
    ];
    for (dir, icon) in dirs {
        if !matches!(
            *icon,
            "folder-documents" | "folder-pictures" | "folder-music"
        ) {
            continue;
        }
        let (Some(name), Some(path)) = (dir.file_name(), dir.to_str()) else {
            continue;
        };
        let cmd = format!("reprise-explorer {}", reprisefs::ops::shell_quote(path));
        let row = MenuEntry::spawn(&name.to_string_lossy(), icon, &cmd);
        v.push(match crate::desktop::document_tip(icon) {
            Some(tip) => row.tipped(tip),
            None => row,
        });
    }
    v.push(MenuEntry::Separator);
    if recent {
        v.push(
            MenuEntry::item(
                "Recent Items",
                "document-open-recent",
                MenuTarget::Documents,
            )
            .tipped(RECENT_TIP),
        );
    }
    v.push(
        MenuEntry::spawn("Computer", "computer", "reprise-explorer Computer")
            .tipped(ShellPlace::Computer.tip()),
    );
    v.push(MenuEntry::Separator);
    v.push(
        MenuEntry::spawn("Control Panel", "preferences-system", "reprise-settings")
            .tipped(CONTROL_PANEL_TIP),
    );
    v.push(MenuEntry::spawn("Printers", "printer", PRINTERS).tipped(PRINTERS_TIP));
    v.push(
        MenuEntry::item(
            "Run...",
            "system-run",
            MenuTarget::Action(crate::config::Action::Run),
        )
        .tipped(RUN_TIP),
    );
    v.push(MenuEntry::spawn("Help and Support", "help-browser", HELP).tipped(HELP_TIP));
    v
}

/// The infotips of the Glass Start menu's places that have no Classic
/// counterpart's.
const RECENT_TIP: &str = "Lists the files you opened most recently.";
const RUN_TIP: &str = "Opens a program, folder, document or web site by its name.";
const HELP_TIP: &str = "Opens the Reprise project page.";

/// A row of a Glass Start menu popup: a program row's context menu or the
/// Shut down options. The `usize` is an index into the app index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartRow {
    Open(usize),
    PinTaskbar(usize),
    UnpinTaskbar(usize),
    PinStart(usize),
    UnpinStart(usize),
    Forget(usize),
    Properties(usize),
    LogOff,
    Lock,
    Restart,
    Sleep,
    Hibernate,
}

/// The context menu of program `app`: Open (bold), Pin to or Unpin from
/// the taskbar and the Start menu, Remove from this list for a frequent
/// row (`in_frequent`), and Properties. Returns the rows and, parallel to
/// them, what each does (None at a separator).
pub fn program_menu(
    app: usize,
    pinned_start: bool,
    pinned_taskbar: bool,
    in_frequent: bool,
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let mut v = vec![
        (
            MenuItem::item("&Open").bold(true),
            Some(StartRow::Open(app)),
        ),
        (MenuItem::Separator, None),
        if pinned_taskbar {
            (
                MenuItem::item("Unpin from Tas&kbar"),
                Some(StartRow::UnpinTaskbar(app)),
            )
        } else {
            (
                MenuItem::item("Pin to Tas&kbar"),
                Some(StartRow::PinTaskbar(app)),
            )
        },
        if pinned_start {
            (
                MenuItem::item("Unpin from Start Men&u"),
                Some(StartRow::UnpinStart(app)),
            )
        } else {
            (
                MenuItem::item("Pin to Start Men&u"),
                Some(StartRow::PinStart(app)),
            )
        },
    ];
    if in_frequent {
        v.push((MenuItem::Separator, None));
        v.push((
            MenuItem::item("Remove from this &list"),
            Some(StartRow::Forget(app)),
        ));
    }
    v.push((MenuItem::Separator, None));
    v.push((
        MenuItem::item("P&roperties"),
        Some(StartRow::Properties(app)),
    ));
    v.into_iter().unzip()
}

/// The Shut down options: Log off, Lock, Restart, Sleep, and Hibernate
/// when `hibernate` (`[power] hibernate` is set).
pub fn shutdown_menu(hibernate: bool) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let mut v = vec![
        (MenuItem::item("&Log off"), Some(StartRow::LogOff)),
        (MenuItem::item("L&ock"), Some(StartRow::Lock)),
        (MenuItem::Separator, None),
        (MenuItem::item("&Restart"), Some(StartRow::Restart)),
        (MenuItem::Separator, None),
        (MenuItem::item("&Sleep"), Some(StartRow::Sleep)),
    ];
    if hibernate {
        v.push((MenuItem::item("&Hibernate"), Some(StartRow::Hibernate)));
    }
    v.into_iter().unzip()
}

impl State {
    pub fn activate_entry(&mut self, entry: &MenuEntry) {
        if let MenuEntry::Item {
            target: crate::config::MenuTarget::Action(a),
            ..
        } = entry
        {
            self.run_action(&a.clone(), 0);
        }
    }

    /// Start `entry` the way the Start menus and the launcher do, counting
    /// the launch for the Glass Start menu's frequently used list.
    pub fn launch_app(&mut self, entry: &AppEntry) {
        self.launches.record(&entry.id);
        crate::state::save_start(&self.launches);
        let argv = reprisefs::apps::launch_argv(entry, &self.config.terminal);
        self.spawn_argv_activated(&argv);
    }
}

#[cfg(test)]
mod preview {
    use super::*;
    use crate::config::{Config, MenuEntry, MenuTarget};
    use crate::icontheme::{IconCache, IconTheme};
    use crate::raster::{Canvas, Rect};
    use crate::text::{TextRenderer, split_mnemonic};
    use crate::theme::{FontConfig, Theme};

    fn items(icons: &mut IconCache, entries: &[MenuEntry], px: i32) -> Vec<MenuItem> {
        entries
            .iter()
            .map(|e| match e {
                MenuEntry::Separator => MenuItem::Separator,
                MenuEntry::Item { label, icon, .. } => {
                    let (label, mnemonic) = split_mnemonic(label);
                    MenuItem::entry(label)
                        .icon(icon.as_deref().and_then(|n| icons.get(n, px)))
                        .submenu(e.opens_submenu())
                        .mnemonic(mnemonic)
                }
            })
            .collect()
    }

    #[test]
    #[ignore]
    fn render_default_startmenu() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let name = std::env::var("REPRISE_PREVIEW_ICONS").unwrap_or_else(|_| "breeze".into());
        let mut icons = IconCache::new(IconTheme::load(&name, &IconTheme::default_roots()));
        let cfg = Config::default();
        let size = theme.fonts.size_px;

        let root_style = MenuStyle::root(&theme);
        let root_items = items(&mut icons, &cfg.startmenu, root_style.icon_px());
        let root_style = root_style.fit(&text, &root_items, size, i32::MAX);
        let root = MenuLayout::compute_styled(&root_style, &root_items, usize::MAX);

        let settings_row = 2;
        let MenuEntry::Item {
            target: MenuTarget::Submenu(children),
            ..
        } = &cfg.startmenu[settings_row]
        else {
            panic!("row 2 should be Settings");
        };
        let sub_style = MenuStyle::submenu(&theme);
        let sub_items = items(&mut icons, children, sub_style.icon_px());
        let sub_style = sub_style.fit(&text, &sub_items, size, i32::MAX);
        let sub = MenuLayout::compute_styled(&sub_style, &sub_items, usize::MAX);

        let usable = Rect::new(0, 0, 640, 480);
        let parent = Rect::new(0, usable.bottom() - root.height, root.width, root.height);
        let row = root.rows[settings_row];
        let row = Rect::new(parent.x + row.x, parent.y + row.y, row.width, row.height);
        let (sx, sy) = place_submenu_inset(
            parent,
            row,
            (sub.width, sub.height),
            usable,
            root.inset.right,
            sub.inset,
        );

        let mut scene = Canvas::new(usable.width as u32, usable.height as u32);
        scene.fill_rect(scene.bounds(), theme.colors.background);
        let mut c = Canvas::new(root.width as u32, root.height as u32);
        draw_menu(
            &mut c,
            &theme,
            &text,
            &root,
            &root_items,
            Some(settings_row),
            &cfg.startmenu_banner,
            true,
        );
        scene.blit_canvas(parent.x, parent.y, &c);
        let mut c = Canvas::new(sub.width as u32, sub.height as u32);
        draw_menu(&mut c, &theme, &text, &sub, &sub_items, Some(0), "", true);
        scene.blit_canvas(sx, sy, &c);

        let crop = Rect::new(
            0,
            parent.y - 8,
            (sx + sub.width + 8).min(usable.width),
            usable.height - parent.y + 8,
        );
        let mut out = Canvas::new(crop.width as u32, crop.height as u32);
        for y in 0..crop.height {
            for x in 0..crop.width {
                let px = scene.get(crop.x + x, crop.y + y);
                out.put(
                    x,
                    y,
                    crate::theme::Color::rgb((px >> 16) as u8, (px >> 8) as u8, px as u8),
                );
            }
        }
        crate::raster::write_preview(&out, "startmenu-preview.png");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `places`, the folders left out (which exist depends on
    /// the machine), a rule as "-".
    fn rows(recent: bool) -> Vec<String> {
        places(recent, &[])
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Separator => Some("-".to_string()),
                MenuEntry::Item { icon, .. }
                    if icon.as_deref().is_some_and(|i| i.starts_with("folder-")) =>
                {
                    None
                }
                MenuEntry::Item { label, .. } => Some(label.clone()),
            })
            .collect()
    }

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Entry {
                    label, mnemonic, ..
                } => {
                    let mut s: Vec<char> = label.chars().collect();
                    if let Some(at) = *mnemonic {
                        s.insert(at, '&');
                    }
                    s.into_iter().collect()
                }
                MenuItem::Separator => String::new(),
            })
            .collect()
    }

    #[test]
    fn the_program_menu_offers_what_applies() {
        let (items, _) = program_menu(0, false, true, true);
        assert_eq!(
            labels(&items),
            [
                "&Open",
                "",
                "Unpin from Tas&kbar",
                "Pin to Start Men&u",
                "",
                "Remove from this &list",
                "",
                "P&roperties"
            ]
        );
        let (_, rows2) = program_menu(0, true, false, false);
        assert!(
            rows2
                .iter()
                .flatten()
                .any(|r| matches!(r, StartRow::UnpinStart(_)))
        );
        assert!(
            !rows2
                .iter()
                .flatten()
                .any(|r| matches!(r, StartRow::Forget(_)))
        );
    }

    #[test]
    fn the_shutdown_menu_lists_hibernate_only_when_configured() {
        assert_eq!(
            labels(&shutdown_menu(false).0),
            ["&Log off", "L&ock", "", "&Restart", "", "&Sleep"]
        );
        assert!(labels(&shutdown_menu(true).0).contains(&"&Hibernate".to_string()));
    }

    #[test]
    fn every_place_has_a_tip_and_a_picture() {
        for e in places(true, &reprisefs::places::document_dir_paths()) {
            if let MenuEntry::Item {
                label, tip, icon, ..
            } = e
            {
                assert!(tip.is_some(), "{label}");
                assert!(icon.is_some_and(|i| !i.is_empty()), "{label}");
            }
        }
    }

    #[test]
    fn the_places_list_recent_items_only_with_recent_files() {
        assert_eq!(
            rows(true),
            [
                "{user}",
                "-",
                "Recent Items",
                "Computer",
                "-",
                "Control Panel",
                "Printers",
                "Run...",
                "Help and Support"
            ]
        );
        assert!(!rows(false).iter().any(|r| r == "Recent Items"));
        assert!(
            places(true, &[])
                .iter()
                .filter(|e| e.opens_submenu())
                .all(|e| matches!(e, MenuEntry::Item { label, .. } if label == "Recent Items")),
            "only Recent Items opens a list of its own"
        );
    }
}
