//! The Start menu's rows come from the config; the menu widget itself is
//! `reprisekit::controls::menu`, re-exported here.

pub use reprisekit::controls::menu::*;

use crate::compositor::State;
use crate::config::{MenuEntry, StartMenuStyle, StartPlace};
use reprisefs::apps::AppEntry;
use reprisekit::powerpanel::{Choice, Leave};
use reprisekit::tr;
use reprisescene::TreeId;

impl State {
    /// The Start button on bar `bar` was pressed: open or close the Start
    /// menu in the configured style.
    pub fn toggle_start(&mut self, bar: TreeId) {
        match self.config.startmenu_style {
            StartMenuStyle::Classic => self.classic_toggle_start_menu(bar),
            StartMenuStyle::Blue | StartMenuStyle::Glass => self.toggle_start_panel(bar),
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
            StartMenuStyle::Blue | StartMenuStyle::Glass => self.toggle_start_panel_from_keys(),
        }
    }

    /// Start opened from the keyboard on its program list: All Programs as a click on it opens
    /// it, or the Classic menu.
    pub fn open_start_programs(&mut self) {
        if self.start_panel.is_none() && self.menus.is_empty() {
            self.toggle_start_from_keys();
        }
        if self.start_panel.is_some() {
            self.start_panel_click(reprisekit::startpanel::Spot::AllPrograms);
        }
    }

    /// Program `app`'s context menu ([`program_menu`]), from its pins and where its desktop file is.
    pub(crate) fn program_menu_of(
        &self,
        app: usize,
        in_frequent: bool,
    ) -> Option<(Vec<MenuItem>, Vec<Option<StartRow>>)> {
        let entry = self.app_index.get(app)?;
        let pinned_start = self.launches.is_pinned(&entry.id);
        let pinned_taskbar = self.quick_file_of(&entry.id).is_some();
        let uninstall = (self.app_uninstall.get(&entry.id).copied()).unwrap_or(Uninstall::Absent);
        let on_desktop = (self.desktop.as_ref()).is_some_and(|d| {
            let name = copied_name(&entry.id, &reprisefs::apps::default_dirs());
            d.entries.iter().any(|e| e.name == name)
        });
        Some(program_menu(
            entry.id.as_str(),
            pinned_start,
            pinned_taskbar,
            in_frequent,
            on_desktop,
            uninstall,
        ))
    }

    /// Close the Start menu in the configured style. `reload` calls it
    /// before the new config is swapped in, so it closes the old style's.
    pub fn close_start(&mut self) {
        match self.config.startmenu_style {
            StartMenuStyle::Classic => self.close_menu(),
            StartMenuStyle::Blue | StartMenuStyle::Glass => self.close_start_panel(),
        }
    }
}

/// The Glass Start menu's right column, top to bottom: the home folder
/// (labelled with the user's name), the Documents, Pictures and Music
/// folders of `dirs` (the document folders that exist), a rule, Recent Items (with `recent`, `[privacy]
/// recent_files`), Computer, a rule, Control Panel, Printers, Run... and
/// Help and Support. Each row has an icon and an infotip. Of the places `startmenu_places` names,
/// only those in `shown` are listed.
pub fn places(
    recent: bool,
    dirs: &[(std::path::PathBuf, &str)],
    shown: &[StartPlace],
) -> Vec<MenuEntry> {
    use reprisefs::places::ShellPlace;
    let mut v = vec![
        MenuEntry::spawn("{user}", "user-home", "reprise-explorer \"$HOME\"")
            .tipped(&ShellPlace::Home.tip()),
    ];
    let has = |p| shown.contains(&p);
    v.extend(
        [
            StartPlace::Documents,
            StartPlace::Pictures,
            StartPlace::Music,
        ]
        .into_iter()
        .filter_map(|p| document_place(dirs, shown, p)),
    );
    v.push(MenuEntry::Separator);
    if recent {
        v.push(recent_row());
    }
    v.extend(has(StartPlace::Computer).then(computer_row));
    v.push(MenuEntry::Separator);
    v.extend(has(StartPlace::ControlPanel).then(control_panel_row));
    v.extend(has(StartPlace::Printers).then(printers_row));
    v.extend(has(StartPlace::Run).then(run_row));
    v.push(help_row());
    v
}

/// The Blue Start menu's right column: Documents, Recent Items (with `recent`), Pictures, Music,
/// Computer, a rule, Control Panel, Printers, a rule, Help and Support, Run.... Rows and `shown`
/// as in [`places`].
pub fn blue_places(
    recent: bool,
    dirs: &[(std::path::PathBuf, &str)],
    shown: &[StartPlace],
) -> Vec<MenuEntry> {
    let has = |p| shown.contains(&p);
    let mut v: Vec<MenuEntry> = document_place(dirs, shown, StartPlace::Documents)
        .into_iter()
        .collect();
    if recent {
        v.push(recent_row());
    }
    v.extend(
        [StartPlace::Pictures, StartPlace::Music]
            .into_iter()
            .filter_map(|p| document_place(dirs, shown, p)),
    );
    v.extend(has(StartPlace::Computer).then(computer_row));
    v.push(MenuEntry::Separator);
    v.extend(has(StartPlace::ControlPanel).then(control_panel_row));
    v.extend(has(StartPlace::Printers).then(printers_row));
    v.push(MenuEntry::Separator);
    v.push(help_row());
    v.extend(has(StartPlace::Run).then(run_row));
    v
}

/// The row of document place `p` when `shown` holds it and its folder is one of `dirs`.
fn document_place(
    dirs: &[(std::path::PathBuf, &str)],
    shown: &[StartPlace],
    p: StartPlace,
) -> Option<MenuEntry> {
    let icon = match p {
        StartPlace::Documents => "folder-documents",
        StartPlace::Pictures => "folder-pictures",
        StartPlace::Music => "folder-music",
        _ => return None,
    };
    if !shown.contains(&p) {
        return None;
    }
    document_row(dirs, icon)
}

/// The row opening the document folder of `dirs` whose icon is `icon`, if it exists.
fn document_row(dirs: &[(std::path::PathBuf, &str)], icon: &str) -> Option<MenuEntry> {
    let (dir, icon) = dirs.iter().find(|(_, i)| *i == icon)?;
    let (name, path) = (dir.file_name()?, dir.to_str()?);
    let cmd = format!("reprise-explorer {}", reprisefs::ops::shell_quote(path));
    let row = MenuEntry::spawn(&name.to_string_lossy(), icon, &cmd);
    Some(match crate::desktop::document_tip(icon) {
        Some(tip) => row.tipped(&tip),
        None => row,
    })
}

fn recent_row() -> MenuEntry {
    use crate::config::MenuTarget;
    MenuEntry::item(
        &tr!("start-recent-items"),
        "document-open-recent",
        MenuTarget::Documents,
    )
    .tipped(&tr!("start-tip-recent"))
}

fn computer_row() -> MenuEntry {
    MenuEntry::spawn(
        &tr!("start-computer"),
        "computer",
        "reprise-explorer Computer",
    )
    .tipped(&reprisefs::places::ShellPlace::Computer.tip())
}

fn control_panel_row() -> MenuEntry {
    MenuEntry::spawn(
        &tr!("start-control-panel"),
        "preferences-system",
        "reprise-settings",
    )
    .tipped(crate::config::CONTROL_PANEL_TIP)
}

fn printers_row() -> MenuEntry {
    use crate::config::{PRINTERS, PRINTERS_TIP};
    MenuEntry::spawn(&tr!("start-printers"), "printer", PRINTERS).tipped(PRINTERS_TIP)
}

fn run_row() -> MenuEntry {
    use crate::config::MenuTarget;
    MenuEntry::item(
        &tr!("start-run"),
        "system-run",
        MenuTarget::Action(crate::config::Action::Run),
    )
    .tipped(&tr!("start-tip-run"))
}

fn help_row() -> MenuEntry {
    MenuEntry::spawn(
        &tr!("start-help-support"),
        "help-browser",
        crate::config::HELP,
    )
    .tipped(&tr!("start-tip-help"))
}

/// A row of a Glass Start menu popup: a program row's or a place's context menu or the
/// Shut down options. The `String` is the program's desktop entry id, so a
/// rescan while the menu is up leaves the row on its program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartRow {
    Open(String),
    PinTaskbar(String),
    UnpinTaskbar(String),
    PinStart(String),
    UnpinStart(String),
    Forget(String),
    OpenFileLocation(String),
    AddToDesktop(String),
    /// A greyed Add to desktop: the desktop holds a copy.
    OnDesktop,
    Uninstall(String),
    /// A greyed Uninstall: its infotip says why.
    CannotUninstall,
    Properties(String),
    /// Open or Explore on a place: what a click on it does.
    OpenPlace(MenuEntry),
    FolderProperties(std::path::PathBuf),
    /// Computer's Properties: About Reprise.
    ComputerProperties,
    SwitchUser,
    Disconnect,
    LogOff,
    Lock,
    ShutDown,
    Restart,
    /// Opens the targets to restart into as a submenu.
    RestartInto,
    Sleep,
    Hibernate,
    /// Install the prepared offline update, then turn off or restart.
    InstallUpdates(reprisekit::controls::endsession::Goal),
    /// A row of the menu of a file on program `String`'s jump list ([`jump_file_menu`]).
    JumpFile(
        JumpFileRow,
        String,
        std::path::PathBuf,
        Option<crate::menuview::JumpListAt>,
    ),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpFileRow {
    Open,
    OpenLocation,
    Pin,
    Unpin,
    /// Takes the file off the program's recent files.
    Remove,
}

/// The menu of file `path` on program `app`'s jump list: Open (bold), Open file location, then
/// Unpin from this list for a `pinned` file, else Pin to this list and Remove from this list.
/// `list` is the task button jump list the file is on, opened again after a change.
pub fn jump_file_menu(
    app: &str,
    path: &std::path::Path,
    pinned: bool,
    list: Option<crate::menuview::JumpListAt>,
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    use JumpFileRow::*;
    let row = |r: JumpFileRow| Some(StartRow::JumpFile(r, app.into(), path.into(), list));
    let mut v = vec![
        (MenuItem::item(&tr!("start-open")).bold(true), row(Open)),
        (
            MenuItem::item(&tr!("start-open-location")),
            row(OpenLocation),
        ),
        (MenuItem::Separator, None),
    ];
    if pinned {
        v.push((MenuItem::item(&tr!("start-jump-unpin")), row(Unpin)));
    } else {
        v.push((MenuItem::item(&tr!("start-jump-pin")), row(Pin)));
        v.push((MenuItem::item(&tr!("start-remove-from-list")), row(Remove)));
    }
    v.into_iter().unzip()
}

/// The kind of a two-column Start menu place with a context menu: a folder, or Computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceMenu {
    Folder(std::path::PathBuf),
    Computer,
}

impl PlaceMenu {
    /// The menu of place `entry` of [`places`] or [`blue_places`], by its icon: the home folder,
    /// the document folders of `dirs` and Computer have one.
    pub fn of(entry: &MenuEntry, dirs: &[(std::path::PathBuf, &str)]) -> Option<PlaceMenu> {
        let MenuEntry::Item {
            icon: Some(icon), ..
        } = entry
        else {
            return None;
        };
        match icon.as_str() {
            "computer" => Some(PlaceMenu::Computer),
            "user-home" => std::env::var_os("HOME").map(|h| PlaceMenu::Folder(h.into())),
            _ => (dirs.iter())
                .find(|(_, i)| i == icon)
                .map(|(d, _)| PlaceMenu::Folder(d.clone())),
        }
    }
}

/// Place `entry`'s context menu as `kind` has it: Open (bold), Explore for a folder, and
/// Properties, the folder's sheet or Computer's About Reprise. Rows as in [`program_menu`].
pub fn place_menu(entry: &MenuEntry, kind: &PlaceMenu) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let open = StartRow::OpenPlace(entry.clone());
    let mut v = vec![(
        MenuItem::item(&tr!("start-place-open")).bold(true),
        Some(open.clone()),
    )];
    let properties = match kind {
        PlaceMenu::Folder(dir) => {
            v.push((MenuItem::item(&tr!("start-place-explore")), Some(open)));
            StartRow::FolderProperties(dir.clone())
        }
        PlaceMenu::Computer => StartRow::ComputerProperties,
    };
    v.push((MenuItem::Separator, None));
    v.push((
        MenuItem::item(&tr!("start-place-properties")),
        Some(properties),
    ));
    v.into_iter().unzip()
}

/// Whether a program's menu offers Uninstall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uninstall {
    Offered,
    /// Shown greyed, with an infotip saying why.
    Greyed,
    Absent,
}

/// The name Add to desktop copies `id`'s file under: its file name in `dirs` (`kde-foo.desktop`
/// stored as `kde/foo.desktop` is `foo.desktop`), else the id.
fn copied_name(id: &reprisefs::apps::DesktopId, dirs: &[std::path::PathBuf]) -> String {
    (reprisefs::apps::find_file_in(dirs, id).as_deref())
        .and_then(std::path::Path::file_name)
        .map_or_else(|| id.as_str().into(), |n| n.to_string_lossy().into_owned())
}

pub(crate) fn on_desktop_tip() -> String {
    tr!("start-tip-on-desktop")
}

pub(crate) fn no_packagekit_tip() -> String {
    tr!("start-tip-no-packagekit")
}

impl Uninstall {
    /// What the menu of a program `remover` removes offers (None: no desktop file): a Flatpak
    /// app (only with `flatpak`), a system package (greyed without `packagekit`) or a launcher an
    /// app installed is uninstalled; the user's own entry is not.
    pub fn of(
        remover: Option<reprisefs::software::Remover>,
        packagekit: bool,
        flatpak: bool,
    ) -> Uninstall {
        use reprisefs::software::Remover;
        match remover {
            Some(Remover::Flatpak(_)) if flatpak => Uninstall::Offered,
            Some(Remover::Flatpak(_)) => Uninstall::Absent,
            Some(Remover::Launcher) => Uninstall::Offered,
            Some(Remover::Package) if packagekit => Uninstall::Offered,
            Some(Remover::Package) => Uninstall::Greyed,
            Some(Remover::None) | None => Uninstall::Absent,
        }
    }
}

/// What each of `apps`' menus offers for Uninstall, by id, from where its desktop file is.
/// Reads the file system: runs with the app index's scan, never from a click.
pub fn uninstall_offers(
    apps: &[reprisefs::apps::AppEntry],
) -> std::collections::HashMap<reprisefs::apps::DesktopId, Uninstall> {
    use reprisefs::software::{flatpak_present, packagekit_present, remover_among};
    let installs = reprisefs::flatpak::installations();
    let (packagekit, flatpak) = (packagekit_present(), flatpak_present());
    (apps.iter())
        .map(|a| {
            let file = reprisefs::apps::find_file(&a.id);
            let remover = file.map(|f| remover_among(&f, &installs));
            (a.id.clone(), Uninstall::of(remover, packagekit, flatpak))
        })
        .collect()
}

/// The context menu of program `app`: Open (bold), Open file location, Pin to or Unpin from
/// the taskbar and the Start menu, Add to desktop (greyed when `on_desktop`), Remove from this
/// list for a frequent row (`in_frequent`), Uninstall as `uninstall` has it, and Properties.
/// Returns the rows and, parallel to them, what each does (None at a separator).
pub fn program_menu(
    app: &str,
    pinned_start: bool,
    pinned_taskbar: bool,
    in_frequent: bool,
    on_desktop: bool,
    uninstall: Uninstall,
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let mut v = vec![
        (
            MenuItem::item(&tr!("start-open")).bold(true),
            Some(StartRow::Open(app.into())),
        ),
        (
            MenuItem::item(&tr!("start-open-location")),
            Some(StartRow::OpenFileLocation(app.into())),
        ),
        (MenuItem::Separator, None),
        if pinned_taskbar {
            (
                MenuItem::item(&tr!("start-unpin-taskbar")),
                Some(StartRow::UnpinTaskbar(app.into())),
            )
        } else {
            (
                MenuItem::item(&tr!("start-pin-taskbar")),
                Some(StartRow::PinTaskbar(app.into())),
            )
        },
        if pinned_start {
            (
                MenuItem::item(&tr!("start-unpin-start")),
                Some(StartRow::UnpinStart(app.into())),
            )
        } else {
            (
                MenuItem::item(&tr!("start-pin-start")),
                Some(StartRow::PinStart(app.into())),
            )
        },
        if on_desktop {
            (
                MenuItem::item(&tr!("start-add-to-desktop")).enabled(false),
                Some(StartRow::OnDesktop),
            )
        } else {
            (
                MenuItem::item(&tr!("start-add-to-desktop")),
                Some(StartRow::AddToDesktop(app.into())),
            )
        },
    ];
    if in_frequent {
        v.push((MenuItem::Separator, None));
        v.push((
            MenuItem::item(&tr!("start-remove-from-list")),
            Some(StartRow::Forget(app.into())),
        ));
    }
    v.push((MenuItem::Separator, None));
    match uninstall {
        Uninstall::Offered => v.push((
            MenuItem::item(&tr!("start-uninstall")),
            Some(StartRow::Uninstall(app.into())),
        )),
        Uninstall::Greyed => v.push((
            MenuItem::item(&tr!("start-uninstall")).enabled(false),
            Some(StartRow::CannotUninstall),
        )),
        Uninstall::Absent => {}
    }
    v.push((
        MenuItem::item(&tr!("start-properties")),
        Some(StartRow::Properties(app.into())),
    ));
    v.into_iter().unzip()
}

/// `menu` ([`shutdown_menu`]) with a Shut down row before Restart, for a power button that does
/// something else.
pub fn with_shut_down(
    menu: (Vec<MenuItem>, Vec<Option<StartRow>>),
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let (mut items, mut rows) = menu;
    let at = (rows.iter().position(|r| *r == Some(StartRow::Restart))).unwrap_or(items.len());
    items.insert(at, MenuItem::item(&tr!("start-shut-down")));
    rows.insert(at, Some(StartRow::ShutDown));
    (items, rows)
}

/// `menu` with Shut down after its last row, in Sleep's group when it has one, else in a group
/// of its own: the Onyx menu's arrow, whose buttons hold no Shut down.
pub fn with_shut_down_last(
    menu: (Vec<MenuItem>, Vec<Option<StartRow>>),
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    let (mut items, mut rows) = menu;
    if !rows.contains(&Some(StartRow::Sleep)) {
        items.push(MenuItem::Separator);
        rows.push(None);
    }
    items.push(MenuItem::item(&tr!("start-shut-down")));
    rows.push(Some(StartRow::ShutDown));
    (items, rows)
}

/// The Shut down options: Install updates and shut down and Install updates and restart while an
/// offline update is prepared (`updates`), Disconnect or a live Switch user (`leave`), Log off,
/// Lock, Restart, Restart into with `restart_into`, then, unless Disconnect is offered, Sleep, and
/// Hibernate when `hibernate` is set.
pub fn shutdown_menu(
    hibernate: bool,
    leave: Leave,
    restart_into: bool,
    updates: bool,
) -> (Vec<MenuItem>, Vec<Option<StartRow>>) {
    use reprisekit::controls::endsession::Goal;
    let mut v = Vec::new();
    let remote = leave == Leave::Disconnect;
    if updates {
        v.extend([
            (
                MenuItem::item(&tr!("start-install-shut-down")),
                Some(StartRow::InstallUpdates(Goal::TurnOff)),
            ),
            (
                MenuItem::item(&tr!("start-install-restart")),
                Some(StartRow::InstallUpdates(Goal::Restart)),
            ),
            (MenuItem::Separator, None),
        ]);
    }
    match leave {
        Leave::Disconnect => v.push((
            MenuItem::item(&Choice::Disconnect.label()),
            Some(StartRow::Disconnect),
        )),
        Leave::SwitchUser { live: true } => v.push((
            MenuItem::item(&tr!("start-switch-user")),
            Some(StartRow::SwitchUser),
        )),
        Leave::SwitchUser { live: false } => {}
    }
    v.extend([
        (
            MenuItem::item(&tr!("start-log-off")),
            Some(StartRow::LogOff),
        ),
        (MenuItem::item(&tr!("start-lock")), Some(StartRow::Lock)),
        (MenuItem::Separator, None),
        (
            MenuItem::item(&tr!("start-restart")),
            Some(StartRow::Restart),
        ),
    ]);
    if restart_into {
        v.push((
            MenuItem::item(&tr!("start-restart-into")).submenu(true),
            Some(StartRow::RestartInto),
        ));
    }
    if !remote {
        v.extend([
            (MenuItem::Separator, None),
            (MenuItem::item(&tr!("start-sleep")), Some(StartRow::Sleep)),
        ]);
        if hibernate {
            v.push((
                MenuItem::item(&tr!("start-hibernate")),
                Some(StartRow::Hibernate),
            ));
        }
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
        self.launch_entry(entry);
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
        places(recent, &[], &StartPlace::ALL)
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
                MenuItem::Separator | MenuItem::Heading(_) => String::new(),
            })
            .collect()
    }

    #[test]
    fn uninstall_follows_what_removes_the_program() {
        use reprisefs::software::{Installation, Remover};
        let flatpak = Some(Remover::Flatpak(Installation::User));
        assert_eq!(Uninstall::of(flatpak, false, true), Uninstall::Offered);
        assert_eq!(
            Uninstall::of(flatpak, true, false),
            Uninstall::Absent,
            "a Flatpak app without the flatpak program"
        );
        assert_eq!(
            Uninstall::of(Some(Remover::Launcher), false, false),
            Uninstall::Offered
        );
        assert_eq!(
            Uninstall::of(Some(Remover::Package), true, false),
            Uninstall::Offered
        );
        assert_eq!(
            Uninstall::of(Some(Remover::Package), false, true),
            Uninstall::Greyed
        );
        assert_eq!(
            Uninstall::of(Some(Remover::None), true, true),
            Uninstall::Absent
        );
        assert_eq!(
            Uninstall::of(None, true, true),
            Uninstall::Absent,
            "no desktop file"
        );
    }

    #[test]
    fn the_program_menu_offers_what_applies() {
        let (items, _) = program_menu("a", false, true, true, false, Uninstall::Absent);
        assert_eq!(
            labels(&items),
            [
                "&Open",
                "Open f&ile location",
                "",
                "Unpin from Tas&kbar",
                "Pin to Start Men&u",
                "Add to &desktop",
                "",
                "Remove from this &list",
                "",
                "P&roperties"
            ]
        );
        let (_, rows2) = program_menu("a", true, false, false, false, Uninstall::Absent);
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
    fn a_jump_list_files_menu_pins_a_recent_file_and_unpins_a_pinned_one() {
        let path = std::path::Path::new("/home/u/notes.txt");
        let (items, rows) = jump_file_menu("mail.desktop", path, false, None);
        assert_eq!(
            labels(&items),
            [
                "&Open",
                "Open f&ile location",
                "",
                "Pi&n to this list",
                "Remove from this &list"
            ]
        );
        let row = |r| {
            Some(StartRow::JumpFile(
                r,
                "mail.desktop".into(),
                path.into(),
                None,
            ))
        };
        assert_eq!(rows[3], row(JumpFileRow::Pin));
        assert_eq!(rows[4], row(JumpFileRow::Remove));
        let (items, rows) = jump_file_menu("mail.desktop", path, true, None);
        assert_eq!(
            labels(&items),
            ["&Open", "Open f&ile location", "", "Unpi&n from this list"]
        );
        assert_eq!(rows[3], row(JumpFileRow::Unpin));
    }

    #[test]
    fn the_program_menu_opens_the_file_location_and_adds_to_the_desktop() {
        let row = |items: &[MenuItem], label: &str| {
            labels(items).iter().position(|l| l == label).expect(label)
        };
        let (items, rows) =
            program_menu("c.desktop", false, false, false, false, Uninstall::Absent);
        let at = row(&items, "Open f&ile location");
        assert_eq!(
            rows[at],
            Some(StartRow::OpenFileLocation("c.desktop".into()))
        );
        let at = row(&items, "Add to &desktop");
        assert_eq!(rows[at], Some(StartRow::AddToDesktop("c.desktop".into())));
        assert!(items[at].selectable());

        let (items, rows) = program_menu("c.desktop", false, false, false, true, Uninstall::Absent);
        let at = row(&items, "Add to &desktop");
        assert!(
            !items[at].selectable(),
            "greyed while the desktop holds a copy"
        );
        assert_eq!(rows[at], Some(StartRow::OnDesktop));
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        assert_eq!(kind.tip(at).as_deref(), Some(on_desktop_tip().as_str()));
    }

    #[test]
    fn add_to_desktop_greys_while_the_desktop_lists_the_programs_file() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::desktopview::create(&mut h.state, "/d".into(), Default::default(), None);
        h.state.app_index = vec![AppEntry {
            plain_name: None,
            name: "Zoom".into(),
            exec_line: "zoom".into(),
            terminal: false,
            id: "Zoom.desktop".parse().unwrap(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
            actions: Vec::new(),
        }];
        let offered = |h: &crate::harness::Harness| {
            let (_, rows) = h.state.program_menu_of(0, false).unwrap();
            rows.contains(&Some(StartRow::AddToDesktop("Zoom.desktop".into())))
        };
        assert!(offered(&h));
        let path = std::path::PathBuf::from("/d/Zoom.desktop");
        h.state.desktop.as_mut().unwrap().entries = vec![crate::desktop::DesktopEntry {
            name: "Zoom.desktop".into(),
            label: "Zoom".into(),
            icon: String::new(),
            target: crate::desktop::Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: crate::desktop::Kind::File,
            is_dir: false,
            allow_launching: true,
            picture: None,
            zip: None,
        }];
        assert!(!offered(&h), "a copy on the desktop greys the row");
    }

    #[test]
    fn a_program_from_a_subfolder_is_copied_under_its_file_name() {
        let apps = std::env::temp_dir().join(format!("reprise-copied-{}", std::process::id()));
        std::fs::create_dir_all(apps.join("kde")).unwrap();
        std::fs::write(apps.join("kde/foo.desktop"), "").unwrap();
        let dirs = [apps.clone()];
        assert_eq!(
            copied_name(&"kde-foo.desktop".parse().unwrap(), &dirs),
            "foo.desktop"
        );
        assert_eq!(
            copied_name(&"gone.desktop".parse().unwrap(), &dirs),
            "gone.desktop",
            "the id while no file holds it"
        );
        std::fs::remove_dir_all(&apps).unwrap();
    }

    #[test]
    fn the_program_menu_pins_in_every_look_at_scale_1_and_2() {
        use reprisekit::interface::Interface;
        reprisekit::pins::fonts();
        use reprisekit::pins::fnv;
        let mut got = Vec::new();
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let base = reprisekit::interfaces::theme_for(interface);
            let text = reprisekit::text::TextRenderer::load(&base.fonts);
            for (name, on_desktop) in [("add to desktop", false), ("on the desktop", true)] {
                let (items, _) =
                    program_menu("a", false, false, true, on_desktop, Uninstall::Offered);
                let room = (i32::MAX, i32::MAX);
                let layout = MenuStyle::context_for(&base, &items).layout_within(
                    &text,
                    &items,
                    base.fonts.size_px,
                    room,
                );
                for s in [1, 2] {
                    let l = layout.scaled(s);
                    let mut c = reprisekit::raster::Canvas::new(l.width as u32, l.height as u32);
                    let (theme, text) = (base.scaled(s), text.for_output(s));
                    draw_menu(&mut c, &theme, text, &l, &items, Some(1), "", true);
                    got.push((format!("{interface:?} {name} {s}"), fnv(&c)));
                }
            }
        }
        let pins = reprisekit::crate_dir!().join("tests/pins/program_menu.txt");
        reprisekit::pins::check_file(&pins, &got);
    }

    #[test]
    fn every_program_menu_row_has_its_own_access_key() {
        let (items, _) = program_menu("a", false, false, true, false, Uninstall::Offered);
        let keys: Vec<char> = labels(&items)
            .iter()
            .filter_map(|l| {
                let at = l.find('&')?;
                l[at + 1..].chars().next().map(|c| c.to_ascii_lowercase())
            })
            .collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(keys.len(), unique.len(), "{:?}", labels(&items));
        assert_eq!(keys.len(), 8, "every row but the separators");
    }

    #[test]
    fn uninstall_sits_above_properties_and_greys_with_its_reason() {
        let (items, rows) = program_menu("c", false, false, false, false, Uninstall::Offered);
        let at = labels(&items).iter().position(|l| l == "U&ninstall...");
        assert_eq!(at, Some(items.len() - 2), "{:?}", labels(&items));
        assert_eq!(rows[items.len() - 2], Some(StartRow::Uninstall("c".into())));
        assert!(items[items.len() - 2].selectable());

        let (items, rows) = program_menu("c", false, false, false, false, Uninstall::Greyed);
        assert!(!items[items.len() - 2].selectable(), "greyed");
        assert_eq!(rows[items.len() - 2], Some(StartRow::CannotUninstall));
    }

    #[test]
    fn the_shutdown_menu_lists_hibernate_only_when_configured() {
        assert_eq!(
            labels(&shutdown_menu(false, Leave::SwitchUser { live: false }, false, false).0),
            ["&Log off", "L&ock", "", "&Restart", "", "&Sleep"]
        );
        assert!(
            labels(&shutdown_menu(true, Leave::SwitchUser { live: false }, false, false).0)
                .contains(&"&Hibernate".to_string())
        );
    }

    #[test]
    fn a_power_button_doing_something_else_puts_shut_down_before_restart() {
        let (items, rows) = with_shut_down(shutdown_menu(
            false,
            Leave::SwitchUser { live: false },
            false,
            false,
        ));
        assert_eq!(
            labels(&items),
            [
                "&Log off",
                "L&ock",
                "",
                "Shu&t down",
                "&Restart",
                "",
                "&Sleep"
            ]
        );
        assert_eq!(rows[3], Some(StartRow::ShutDown));
    }

    #[test]
    fn onyxs_arrow_ends_with_shut_down_in_sleeps_group_or_its_own() {
        let (items, rows) = with_shut_down_last(shutdown_menu(
            true,
            Leave::SwitchUser { live: true },
            false,
            false,
        ));
        assert_eq!(
            labels(&items),
            [
                "S&witch user",
                "&Log off",
                "L&ock",
                "",
                "&Restart",
                "",
                "&Sleep",
                "&Hibernate",
                "Shu&t down"
            ]
        );
        assert_eq!(rows.last(), Some(&Some(StartRow::ShutDown)));
        let (items, _) = with_shut_down_last(shutdown_menu(false, Leave::Disconnect, false, false));
        assert_eq!(labels(&items)[items.len() - 2..], ["", "Shu&t down"]);
    }

    #[test]
    fn the_shutdown_menu_offers_switch_user_first_under_reprise_login() {
        let (items, rows) = shutdown_menu(false, Leave::SwitchUser { live: true }, false, false);
        assert_eq!(labels(&items)[..2], ["S&witch user", "&Log off"]);
        assert_eq!(rows[0], Some(StartRow::SwitchUser));
    }

    #[test]
    fn the_shutdown_menu_offers_disconnect_first_on_the_remote_side() {
        let (items, rows) = shutdown_menu(false, Leave::Disconnect, false, false);
        assert_eq!(labels(&items)[..2], ["&Disconnect", "&Log off"]);
        assert_eq!(
            rows[..3],
            [
                Some(StartRow::Disconnect),
                Some(StartRow::LogOff),
                Some(StartRow::Lock)
            ]
        );
        assert!(!rows.contains(&Some(StartRow::SwitchUser)));
    }

    #[test]
    fn a_prepared_update_heads_the_shutdown_menu_with_its_two_install_rows() {
        use reprisekit::controls::endsession::Goal;
        let (items, rows) = shutdown_menu(false, Leave::SwitchUser { live: false }, false, true);
        assert_eq!(
            labels(&items)[..4],
            [
                "Install &updates and shut down",
                "Install updates and r&estart",
                "",
                "&Log off"
            ]
        );
        assert_eq!(
            rows[..2],
            [
                Some(StartRow::InstallUpdates(Goal::TurnOff)),
                Some(StartRow::InstallUpdates(Goal::Restart))
            ]
        );
    }

    #[test]
    fn the_shutdown_menu_offers_no_sleep_or_hibernate_on_the_remote_side() {
        let (items, rows) = shutdown_menu(true, Leave::Disconnect, false, false);
        assert_eq!(
            labels(&items),
            ["&Disconnect", "&Log off", "L&ock", "", "&Restart"]
        );
        assert!(!rows.contains(&Some(StartRow::Sleep)));
        assert!(!rows.contains(&Some(StartRow::Hibernate)));
    }

    #[test]
    fn every_place_has_a_tip_and_a_picture() {
        for e in places(
            true,
            &reprisefs::places::document_dir_paths(),
            &StartPlace::ALL,
        ) {
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
            places(true, &[], &StartPlace::ALL)
                .iter()
                .filter(|e| e.opens_submenu())
                .all(|e| matches!(e, MenuEntry::Item { label, .. } if label == "Recent Items")),
            "only Recent Items opens a list of its own"
        );
    }

    #[test]
    fn the_blue_places_put_recent_items_under_documents_and_run_last() {
        let dirs = [
            (std::path::PathBuf::from("/h/Documents"), "folder-documents"),
            (std::path::PathBuf::from("/h/Music"), "folder-music"),
            (std::path::PathBuf::from("/h/Pictures"), "folder-pictures"),
        ];
        let labels = |recent| -> Vec<String> {
            blue_places(recent, &dirs, &StartPlace::ALL)
                .iter()
                .map(|e| match e {
                    MenuEntry::Separator => "-".to_string(),
                    MenuEntry::Item { label, .. } => label.clone(),
                })
                .collect()
        };
        assert_eq!(
            labels(true),
            [
                "Documents",
                "Recent Items",
                "Pictures",
                "Music",
                "Computer",
                "-",
                "Control Panel",
                "Printers",
                "-",
                "Help and Support",
                "Run..."
            ]
        );
        assert!(!labels(false).iter().any(|r| r == "Recent Items"));
    }

    #[test]
    fn a_place_left_out_of_the_shown_list_is_not_listed() {
        let dirs = [
            (std::path::PathBuf::from("/h/Documents"), "folder-documents"),
            (std::path::PathBuf::from("/h/Music"), "folder-music"),
        ];
        let shown = [StartPlace::Music, StartPlace::Run];
        let labels = |v: Vec<MenuEntry>| -> Vec<String> {
            v.iter()
                .map(|e| match e {
                    MenuEntry::Separator => "-".to_string(),
                    MenuEntry::Item { label, .. } => label.clone(),
                })
                .collect()
        };
        assert_eq!(
            labels(places(true, &dirs, &shown)),
            [
                "{user}",
                "Music",
                "-",
                "Recent Items",
                "-",
                "Run...",
                "Help and Support"
            ]
        );
        assert_eq!(
            labels(blue_places(false, &dirs, &shown)),
            ["Music", "-", "-", "Help and Support", "Run..."],
            "the panel draws two rules in a row as one"
        );
    }
}
