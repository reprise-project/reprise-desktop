//! The menu stack on the scene: Start menu and submenus, window, taskbar, Quick
//! Launch and desktop menus, jump lists, window lists and tray DBusMenus. Layout
//! and drawing come from `startmenu` (reprisekit); this file opens, places,
//! drives and closes them. Each open menu is a tree in `layers.menu`, tagged
//! `SceneOwner::Menu(depth)`, holding its shadow (no input) under its buffer.
//! Menus carry no glass, so the glass below never blurs a menu.

use std::{
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use reprisefs::apps::AppEntry;
use reprisekit::taskbar::{Edge, StartMenuDock};
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::{
    barplace::BarPlacement,
    compositor::{SceneOwner, State},
    config::{Action, MenuEntry, MenuTarget},
    desktop::IconId,
    desktopview::DesktopCommand,
    keys,
    output::upload_scaled,
    raster::{Canvas, Rect},
    startmenu::{
        ContextBitmaps, MenuItem, MenuLayout, MenuStyle, StartRow, draw_menu, draw_menu_shadow,
        enter_row, place_popup, place_submenu_inset, rows_for_key, shadow_reach, step_selection,
        submenu_max_rows,
    },
    taskbar::{JumpRow, QuickCommand, TaskKey, TaskbarCommand, WindowListRow},
    text::split_mnemonic,
    window::WindowId,
    winmenu::{WindowMenuRow, WindowMenuState},
};

/// A program menu's rows: indices into the app index as it stood when the menu
/// opened, so a rescan while open does not shift them.
pub struct AppRows {
    apps: Rc<[AppEntry]>,
    rows: Vec<usize>,
}

impl AppRows {
    pub fn get(&self, row: usize) -> Option<&AppEntry> {
        self.apps.get(*self.rows.get(row)?)
    }

    fn iter(&self) -> impl Iterator<Item = &AppEntry> {
        self.rows.iter().filter_map(|&i| self.apps.get(i))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct JumpTarget {
    pub name: String,
    pub launch: Option<Action>,
    pub entry: Option<AppEntry>,
    pub pin_file: Option<String>,
    pub source: Option<PathBuf>,
    pub windows: Vec<WindowId>,
}

pub enum MenuKind {
    Entries(Vec<MenuEntry>),
    Groups {
        apps: Rc<[AppEntry]>,
        groups: Vec<reprisefs::apps::Group>,
    },
    Programs(AppRows),
    Documents {
        paths: Vec<Option<PathBuf>>,
        tips: Vec<Option<String>>,
    },
    Tray {
        id: String,
        node_ids: Vec<Option<i32>>,
    },
    Window {
        window: WindowId,
        rows: Vec<Option<WindowMenuRow>>,
    },
    Taskbar {
        commands: Vec<Option<TaskbarCommand>>,
    },
    Quick {
        file: String,
        commands: Vec<Option<QuickCommand>>,
    },
    Desktop {
        commands: Vec<Option<DesktopCommand>>,
        target: Option<IconId>,
    },
    /// A file or folder search found (`filemenu`).
    File {
        commands: Vec<Option<DesktopCommand>>,
        path: PathBuf,
    },
    /// A grouped taskbar button's popup (`taskbar::window_list`): `button` is the
    /// taskbar button index it hangs off, so clicking it again toggles the list shut.
    WindowList {
        windows: Vec<WindowId>,
        rows: Vec<Option<WindowListRow>>,
        button: usize,
    },
    JumpList {
        rows: Vec<Option<JumpRow>>,
        target: Box<JumpTarget>,
        button: usize,
    },
    NotifyActions {
        id: u32,
        keys: Vec<String>,
    },
    Layouts,
    NightLight(Vec<Option<crate::nightlight::Row>>),
    StartPanel {
        rows: Vec<Option<StartRow>>,
    },
    TaskOverflow {
        bar: TreeId,
        keys: Vec<TaskKey<WindowId>>,
    },
}

/// What row `idx` of a menu stands for, resolved through the kind's own parallel
/// vector; `State::menu_activate` is a match over this.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuPick {
    Nothing,
    Run(Action),
    App(AppEntry),
    Document(PathBuf),
    Window {
        window: WindowId,
        row: WindowMenuRow,
    },
    Taskbar(TaskbarCommand),
    Quick {
        file: String,
        command: QuickCommand,
    },
    Desktop {
        command: DesktopCommand,
        target: Option<IconId>,
    },
    File {
        command: DesktopCommand,
        path: PathBuf,
    },
    WindowList {
        windows: Vec<WindowId>,
        row: WindowListRow,
    },
    Jump {
        row: JumpRow,
        target: JumpTarget,
    },
    NotifyAction {
        id: u32,
        key: String,
    },
    Layout(u32),
    NightLight(crate::nightlight::Row),
    StartPanel(StartRow),
    TaskOverflow {
        bar: TreeId,
        key: TaskKey<WindowId>,
    },
}

impl MenuKind {
    pub fn pick(&self, idx: usize) -> MenuPick {
        match self {
            MenuKind::Entries(entries) => match entries.get(idx) {
                Some(MenuEntry::Item {
                    target: MenuTarget::Action(a),
                    ..
                }) => MenuPick::Run(a.clone()),
                _ => MenuPick::Nothing,
            },
            MenuKind::Groups { .. } | MenuKind::Tray { .. } => MenuPick::Nothing,
            MenuKind::Programs(apps) => apps
                .get(idx)
                .cloned()
                .map_or(MenuPick::Nothing, MenuPick::App),
            MenuKind::Documents { paths, .. } => paths
                .get(idx)
                .cloned()
                .flatten()
                .map_or(MenuPick::Nothing, MenuPick::Document),
            MenuKind::Window { window, rows } => match rows.get(idx).copied().flatten() {
                Some(row) => MenuPick::Window {
                    window: *window,
                    row,
                },
                None => MenuPick::Nothing,
            },
            MenuKind::Taskbar { commands } => commands
                .get(idx)
                .copied()
                .flatten()
                .map_or(MenuPick::Nothing, MenuPick::Taskbar),
            MenuKind::Quick { file, commands } => match commands.get(idx).copied().flatten() {
                Some(command) => MenuPick::Quick {
                    file: file.clone(),
                    command,
                },
                None => MenuPick::Nothing,
            },
            MenuKind::Desktop { commands, target } => match commands.get(idx).cloned().flatten() {
                Some(command) => MenuPick::Desktop {
                    command,
                    target: target.clone(),
                },
                None => MenuPick::Nothing,
            },
            MenuKind::File { commands, path } => match commands.get(idx).cloned().flatten() {
                Some(command) => MenuPick::File {
                    command,
                    path: path.clone(),
                },
                None => MenuPick::Nothing,
            },
            MenuKind::WindowList { windows, rows, .. } => match rows.get(idx).copied().flatten() {
                Some(row) => MenuPick::WindowList {
                    windows: windows.clone(),
                    row,
                },
                None => MenuPick::Nothing,
            },
            MenuKind::JumpList { rows, target, .. } => match rows.get(idx).cloned().flatten() {
                Some(row) => MenuPick::Jump {
                    row,
                    target: (**target).clone(),
                },
                None => MenuPick::Nothing,
            },
            MenuKind::NotifyActions { id, keys } => match keys.get(idx) {
                Some(key) => MenuPick::NotifyAction {
                    id: *id,
                    key: key.clone(),
                },
                None => MenuPick::Nothing,
            },
            MenuKind::Layouts => MenuPick::Layout(idx as u32),
            MenuKind::NightLight(rows) => rows
                .get(idx)
                .copied()
                .flatten()
                .map_or(MenuPick::Nothing, MenuPick::NightLight),
            MenuKind::StartPanel { rows } => rows
                .get(idx)
                .copied()
                .flatten()
                .map_or(MenuPick::Nothing, MenuPick::StartPanel),
            MenuKind::TaskOverflow { bar, keys } => {
                keys.get(idx)
                    .map_or(MenuPick::Nothing, |&key| MenuPick::TaskOverflow {
                        bar: *bar,
                        key,
                    })
            }
        }
    }

    pub fn tip(&self, idx: usize) -> Option<String> {
        match self {
            MenuKind::Entries(entries) => entries.get(idx)?.tip().map(String::from),
            MenuKind::Documents { tips, .. } => tips.get(idx).cloned().flatten(),
            _ => None,
        }
    }
}

pub struct Menu {
    pub tree: TreeId,
    buffer: BufferId,
    shadow: BufferId,
    /// How far down from the menu's top its shadow may reach; None lets all of it
    /// show. Under a look that stands the Start menu on the bar it casts none (0).
    shadow_floor: Option<i32>,
    /// (width, height, shadow size, floor, scale) the shadow was last uploaded
    /// for, the floor in device px: a hover redraw leaves it, a rescale redraws it.
    shadow_drawn: Option<(i32, i32, i32, i32, i32)>,
    pub layout: MenuLayout,
    pub items: Vec<MenuItem>,
    pub x: i32,
    pub y: i32,
    pub hovered: Option<usize>,
    pub output: Output,
    pub kind: MenuKind,
    pub parent_row: Option<usize>,
    opened: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRoot {
    Window(WindowId),
    WindowList(usize),
    JumpList(usize),
    TaskOverflow(TreeId),
    Other,
}

const RECENT_DOCUMENTS: usize = 15;

const FADE_TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPending {
    Open { depth: usize, row: usize },
    CloseAbove { depth: usize },
}

#[derive(Debug, PartialEq, Eq)]
enum KeyAct {
    Nothing,
    Select(Option<usize>),
    SelectAnd(usize, Box<KeyAct>),
    CloseTopmost,
    CloseSubmenu,
    OpenSubmenu(usize),
    Activate(usize),
}

fn row_has_submenu(items: &[MenuItem], row: usize) -> bool {
    matches!(
        items.get(row),
        Some(MenuItem::Entry {
            submenu: true,
            enabled: true,
            ..
        })
    )
}

/// What `sym` does to the topmost menu, whose rows are `items` with `current`
/// highlighted: Up/Down move, Right/Enter open a submenu (Enter runs a plain row
/// or the bold default), Left/Escape close, a letter jumps to its access key
/// and runs it when unique. None for a key the menu ignores.
#[expect(non_upper_case_globals)] // xkb keysym names in patterns
fn key_act(items: &[MenuItem], current: Option<usize>, sym: u32) -> Option<KeyAct> {
    use keys::*;
    let open_or_run = |row: usize| {
        if row_has_submenu(items, row) {
            KeyAct::OpenSubmenu(row)
        } else {
            KeyAct::Activate(row)
        }
    };
    Some(match sym {
        KEY_Down | KEY_KP_Down => KeyAct::Select(step_selection(items, current, 1)),
        KEY_Up | KEY_KP_Up => KeyAct::Select(step_selection(items, current, -1)),
        KEY_Home | KEY_KP_Home => KeyAct::Select(step_selection(items, None, 1)),
        KEY_End | KEY_KP_End => KeyAct::Select(step_selection(items, None, -1)),
        KEY_Escape => KeyAct::CloseTopmost,
        KEY_Left | KEY_KP_Left => KeyAct::CloseSubmenu,
        KEY_Right | KEY_KP_Right => match current.filter(|&r| row_has_submenu(items, r)) {
            Some(row) => KeyAct::OpenSubmenu(row),
            None => KeyAct::Nothing,
        },
        KEY_Return | KEY_KP_Enter | KEY_space => match (current, enter_row(items, current)) {
            (_, None) => KeyAct::Nothing,
            (Some(row), _) => open_or_run(row),
            (None, Some(row)) => KeyAct::SelectAnd(row, Box::new(open_or_run(row))),
        },
        _ => {
            let ch = smithay::input::keyboard::Keysym::new(sym)
                .key_char()
                .filter(|c| c.is_alphanumeric())?;
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
    })
}

/// The top-left of a Start menu `size` big hung off Start at `start` on bar `p`
/// the way `dock` says. Along the bottom or top it is flush with Start's left
/// edge and kept inside `area`; beside a side bar its near edge is where `dock`
/// puts it, level with Start's top (`barplace::beside_on_output`). A hidden
/// auto-hidden bar's menu stands on its reveal line (`BarPlacement::start_dock`).
fn start_menu_at(
    dock: StartMenuDock,
    p: &BarPlacement,
    start: Rect,
    (w, h): (i32, i32),
    area: Rect,
) -> (i32, i32) {
    let (dock, bar) = p.start_dock(dock);
    match p.edge {
        Edge::Bottom => {
            let y = dock.menu_at(p.edge, (bar.y, bar.bottom()), h);
            (start.x, y.max(area.y))
        }
        Edge::Top => {
            let y = dock.menu_at(p.edge, (bar.y, bar.bottom()), h);
            (start.x, y.min(area.bottom() - h).max(bar.y))
        }
        Edge::Left | Edge::Right => (
            crate::barplace::beside_on_output(
                dock.menu_at(p.edge, (bar.x, bar.right()), w),
                w,
                bar,
                area,
            ),
            start.y.min(area.bottom() - h).max(area.y),
        ),
    }
}

fn start_menu_shadow_floor(dock: StartMenuDock, p: &BarPlacement) -> Option<i32> {
    p.start_dock(dock).0.shadow_floor(p.edge)
}

fn start_submenu_shadow_floor(dock: StartMenuDock, p: &BarPlacement, menu_top: i32) -> Option<i32> {
    let (dock, bar) = p.start_dock(dock);
    dock.submenu_shadow_floor(p.edge, bar.y, menu_top)
}

fn list_anchor(layout: &reprisekit::taskbar::TaskbarLayout, i: usize) -> Rect {
    layout
        .tasks
        .get(i)
        .copied()
        .or(layout.more)
        .unwrap_or_default()
}

impl State {
    pub(crate) fn push_menu(
        &mut self,
        depth: usize,
        (x, y): (i32, i32),
        layout: MenuLayout,
        items: Vec<MenuItem>,
        output: Output,
        kind: MenuKind,
    ) {
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.menu);
        scene.set_data(tree, Some(SceneOwner::Menu(depth)));
        crate::glass::leave_out(scene, tree);
        let shadow = scene.buffer_create(tree);
        scene.set_input(shadow, Input::Passthrough);
        let buffer = scene.buffer_create(tree);
        scene.set_position(tree, x, y);
        let fade = self.config.theme.metrics.menu_fade_ms > 0;
        if fade {
            scene.set_opacity(tree, 0.0);
        }
        self.menus.push(Menu {
            tree,
            buffer,
            shadow,
            shadow_floor: None,
            shadow_drawn: None,
            layout,
            items,
            x,
            y,
            hovered: None,
            output,
            kind,
            parent_row: None,
            opened: Instant::now(),
        });
        if fade {
            self.arm_menu_fade();
        }
        crate::autohideview::poke(self);
    }

    pub(crate) fn redraw_menu(&mut self, depth: usize) {
        let Some(m) = self.menus.get(depth) else {
            return;
        };
        let scale = self.draw_scale_at(Some(&m.output));
        let theme = self.config.theme.scaled(scale);
        let layout = m.layout.scaled(scale);
        let mut canvas = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
        draw_menu(
            &mut canvas,
            &theme,
            &self.text,
            &layout,
            &m.items,
            m.hovered,
            &self.config.startmenu_banner,
            self.menu_keys,
        );
        let size = shadow_reach(&theme);
        let floor = m.shadow_floor.map_or(layout.height + size, |f| f * scale);
        let shadow_key = (layout.width, layout.height, size, floor, scale);
        let shadow = (m.shadow_drawn != Some(shadow_key))
            .then(|| draw_menu_shadow(&theme, layout.width, layout.height, floor));
        let (buffer, shadow_buffer) = (m.buffer, m.shadow);
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        if let Some(drawn) = shadow {
            upload_scaled(&mut self.scene, shadow_buffer, &drawn, scale);
            self.menus[depth].shadow_drawn = Some(shadow_key);
        }
        self.redraw_all();
        crate::access::touch(self);
    }

    fn entry_items(&mut self, entries: &[MenuEntry], px: i32) -> Vec<MenuItem> {
        let user = crate::lock::current_user();
        entries
            .iter()
            .map(|e| match e {
                MenuEntry::Separator => MenuItem::Separator,
                MenuEntry::Item { label, icon, .. } => {
                    let (label, mnemonic) =
                        split_mnemonic(&crate::startmenu::expand_label(label, &user));
                    MenuItem::entry(label)
                        .icon(icon.as_deref().and_then(|n| self.icons.get(n, px)))
                        .submenu(e.opens_submenu())
                        .mnemonic(mnemonic)
                }
            })
            .collect()
    }

    fn submenu_open_for(&self, depth: usize, row: usize) -> bool {
        self.menus
            .get(depth + 1)
            .is_some_and(|m| m.parent_row == Some(row))
    }

    fn is_tray_menu(&self, depth: usize) -> bool {
        matches!(
            self.menus.get(depth).map(|m| &m.kind),
            Some(MenuKind::Tray { .. })
        )
    }

    fn row_opens_submenu(&self, depth: usize, idx: usize) -> bool {
        match self.menus.get(depth).map(|m| &m.kind) {
            Some(MenuKind::Entries(entries)) => entries.get(idx).is_some_and(|e| e.opens_submenu()),
            Some(MenuKind::Groups { groups, .. }) => idx < groups.len(),
            Some(
                kind @ (MenuKind::Desktop { .. }
                | MenuKind::File { .. }
                | MenuKind::Taskbar { .. }
                | MenuKind::Window { .. }
                | MenuKind::JumpList { .. }
                | MenuKind::WindowList { .. }),
            ) => match kind.pick(idx) {
                MenuPick::Desktop { command, .. } | MenuPick::File { command, .. } => {
                    command.opens_submenu()
                }
                MenuPick::Taskbar(c) => c == TaskbarCommand::Toolbars,
                MenuPick::Window { row, .. } => {
                    matches!(row, WindowMenuRow::MoveToMenu | WindowMenuRow::TabsOfMenu)
                }
                MenuPick::Jump { row, .. } => row == JumpRow::MoveToMenu,
                MenuPick::WindowList { row, .. } => row == WindowListRow::MoveToMenu,
                _ => false,
            },
            _ => false,
        }
    }

    pub fn menu_root(&self) -> Option<MenuRoot> {
        Some(match &self.menus.first()?.kind {
            MenuKind::Window { window, .. } => MenuRoot::Window(*window),
            MenuKind::WindowList { button, .. } => MenuRoot::WindowList(*button),
            MenuKind::JumpList { button, .. } => MenuRoot::JumpList(*button),
            MenuKind::TaskOverflow { bar, .. } => MenuRoot::TaskOverflow(*bar),
            _ => MenuRoot::Other,
        })
    }

    pub fn open_start_menu(&mut self, bar: TreeId) {
        self.close_menu();
        let Some((output, start)) = self.bar(bar).map(|b| (b.output.clone(), b.layout.start))
        else {
            return;
        };
        let b = self.layout_box(Some(&output));
        if b.width <= 0 {
            return;
        }
        let entries = self.config.startmenu.clone();
        let style = MenuStyle::root(&self.config.theme);
        let px = style.icon_px() * self.draw_scale_at(Some(&output)).max(1);
        let items = self.entry_items(&entries, px);
        let area = self.usable_area(Some(&output));
        let style = style.fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            area.width,
        );
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let dock = self.config.theme.taskbar_shape().start_menu;
        let p = self.bar_placement(&output);
        let start = Rect::new(
            p.shown.x + start.x,
            p.shown.y + start.y,
            start.width,
            start.height,
        );
        let at = start_menu_at(dock, &p, start, (layout.width, layout.height), area);
        self.push_menu(0, at, layout, items, output, MenuKind::Entries(entries));
        if let Some(m) = self.menus.first_mut() {
            m.shadow_floor = start_menu_shadow_floor(dock, &p);
        }
        if let Some(b) = self.bar_mut(bar) {
            b.start_pressed = true;
        }
        self.redraw_menu(0);
        self.redraw_taskbars();
    }

    pub(crate) fn classic_toggle_start_menu(&mut self, bar: TreeId) {
        if self.menus.is_empty() {
            self.open_start_menu(bar);
        } else {
            self.close_menu();
        }
    }

    pub(crate) fn classic_toggle_start_menu_from_keys(&mut self) {
        if !self.menus.is_empty() {
            self.close_menu();
            return;
        }
        let p = self.pointer_location();
        let Some(output) = self.output_at(p.x, p.y) else {
            return;
        };
        let Some(bar) = self
            .bars
            .iter()
            .find(|b| b.output == output)
            .map(|b| b.tree)
        else {
            return;
        };
        self.open_start_menu(bar);
        self.enter_menu_keys();
    }

    pub(crate) fn enter_menu_keys(&mut self) {
        if self.menu_keys {
            return;
        }
        self.menu_keys = true;
        for depth in 0..self.menus.len() {
            self.redraw_menu(depth);
        }
        self.redraw_taskbars();
    }

    fn submenu_rows(
        &mut self,
        depth: usize,
        row: usize,
    ) -> Option<(Vec<MenuItem>, MenuKind, bool)> {
        let parent = self.menus.get(depth)?;
        let output = parent.output.clone();
        match &parent.kind {
            MenuKind::Taskbar { commands } => {
                if commands.get(row).copied().flatten() != Some(TaskbarCommand::Toolbars) {
                    return None;
                }
                let (items, commands) = crate::taskbar::toolbars_menu(
                    self.bar_shared.quick_shown,
                    self.config.taskbar.pinned,
                );
                return Some((items, MenuKind::Taskbar { commands }, true));
            }
            MenuKind::Window { window, rows }
                if rows.get(row).copied().flatten() == Some(WindowMenuRow::MoveToMenu)
                    && matches!(
                        parent.items.get(row),
                        Some(MenuItem::Entry { enabled: true, .. })
                    ) =>
            {
                let window = *window;
                let own = self.win(self.home(window)).map_or(0, |h| h.desktop);
                let (items, rows) =
                    crate::winmenu::move_to_menu(self.desktops.count, own, WindowMenuRow::MoveTo);
                return Some((items, MenuKind::Window { window, rows }, true));
            }
            MenuKind::Window { window, rows }
                if rows.get(row).copied().flatten() == Some(WindowMenuRow::TabsOfMenu)
                    && matches!(
                        parent.items.get(row),
                        Some(MenuItem::Entry { enabled: true, .. })
                    ) =>
            {
                let window = *window;
                let px = self.context_icon_px(&output);
                let targets = self.tab_targets(window);
                let rows = targets
                    .into_iter()
                    .filter_map(|t| {
                        let State {
                            windows,
                            icons,
                            app_index,
                            ..
                        } = &mut *self;
                        let win = windows.iter().find(|x| x.id == t)?;
                        let icon = crate::icon::window_icon(icons, app_index, win, px);
                        Some((t, win.title(), icon.map(|i| (*i).clone())))
                    })
                    .collect();
                let (items, rows) = crate::winmenu::tabs_of_menu(rows);
                return Some((items, MenuKind::Window { window, rows }, true));
            }
            MenuKind::JumpList {
                rows,
                target,
                button,
            } if rows.get(row).cloned().flatten() == Some(JumpRow::MoveToMenu) => {
                let d = self.desktops;
                let (items, rows) =
                    crate::winmenu::move_to_menu(d.count, d.current, JumpRow::MoveTo);
                let kind = MenuKind::JumpList {
                    rows,
                    target: target.clone(),
                    button: *button,
                };
                return Some((items, kind, true));
            }
            MenuKind::WindowList {
                windows,
                rows,
                button,
            } if rows.get(row).copied().flatten() == Some(WindowListRow::MoveToMenu) => {
                let d = self.desktops;
                let (items, rows) =
                    crate::winmenu::move_to_menu(d.count, d.current, WindowListRow::MoveTo);
                let kind = MenuKind::WindowList {
                    windows: windows.clone(),
                    rows,
                    button: *button,
                };
                return Some((items, kind, true));
            }
            MenuKind::Desktop { commands, target } => {
                let target = target.clone();
                let command = commands.get(row).cloned().flatten()?;
                let apps = |s: &Self| {
                    target
                        .as_ref()
                        .map(|t| s.open_with_apps(t))
                        .unwrap_or_default()
                };
                let (items, commands) = self.desktop_submenu(command, &output, apps)?;
                return Some((items, MenuKind::Desktop { commands, target }, true));
            }
            MenuKind::File { commands, path } => {
                let path = path.clone();
                let command = commands.get(row).cloned().flatten()?;
                let apps = |s: &Self| s.open_with_apps_for(&path);
                let (items, commands) = self.desktop_submenu(command, &output, apps)?;
                return Some((items, MenuKind::File { commands, path }, true));
            }
            _ => {}
        }
        let (target, folder) = match &parent.kind {
            MenuKind::Entries(entries) => match entries.get(row) {
                Some(MenuEntry::Item { target, .. })
                    if !matches!(target, MenuTarget::Action(_)) =>
                {
                    (target.clone(), None)
                }
                _ => return None,
            },
            MenuKind::Groups { apps, groups } => {
                let g = groups.get(row)?;
                (
                    MenuTarget::Programs,
                    Some(AppRows {
                        apps: apps.clone(),
                        rows: g.apps.clone(),
                    }),
                )
            }
            _ => return None,
        };
        let style = MenuStyle::submenu(&self.config.theme);
        let px = style.icon_px() * self.draw_scale_at(Some(&output)).max(1);
        let app_items = |s: &mut State, apps: &AppRows| -> Vec<MenuItem> {
            apps.iter()
                .map(|a| {
                    MenuItem::entry(a.name.clone())
                        .icon(a.icon.as_deref().and_then(|n| s.icons.get(n, px)))
                })
                .collect()
        };
        let (items, kind) = match target {
            MenuTarget::Programs => match folder {
                Some(apps) => (app_items(self, &apps), MenuKind::Programs(apps)),
                None => {
                    let apps: Rc<[AppEntry]> = Rc::from(self.app_index.as_slice());
                    let groups = reprisefs::apps::group(&apps);
                    let folder_icon = self.icons.get("folder", px);
                    let items = groups
                        .iter()
                        .map(|g| {
                            MenuItem::entry(g.name.clone())
                                .icon(folder_icon.clone())
                                .submenu(true)
                        })
                        .collect();
                    (items, MenuKind::Groups { apps, groups })
                }
            },
            MenuTarget::Settings => {
                let apps: Rc<[AppEntry]> = Rc::from(self.app_index.as_slice());
                let rows = (0..apps.len())
                    .filter(|&i| apps[i].shown && apps[i].in_category("Settings"))
                    .collect();
                let apps = AppRows { apps, rows };
                (app_items(self, &apps), MenuKind::Programs(apps))
            }
            MenuTarget::Documents => self.documents_rows(px),
            MenuTarget::Submenu(entries) => {
                let mut items = self.entry_items(&entries, px);
                if items.is_empty() {
                    items.push(MenuItem::entry("(Empty)").enabled(false));
                }
                (items, MenuKind::Entries(entries))
            }
            MenuTarget::Action(_) => return None,
        };
        Some((items, kind, false))
    }

    fn documents_rows(&mut self, px: i32) -> (Vec<MenuItem>, MenuKind) {
        let dirs = reprisefs::places::document_dirs();
        let show_recent = self.config.recent_files;
        let recent = if show_recent {
            reprisefs::recent::RecentFiles::new().current(RECENT_DOCUMENTS)
        } else {
            Vec::new()
        };
        let mut items: Vec<MenuItem> = Vec::new();
        let mut paths: Vec<Option<PathBuf>> = Vec::new();
        let mut tips: Vec<Option<String>> = Vec::new();
        let name = |p: &PathBuf| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        for (dir, icon) in &dirs {
            items.push(MenuItem::entry(name(dir)).icon(self.icons.get(icon, px)));
            paths.push(Some(dir.clone()));
            tips.push(crate::desktop::document_tip(icon).map(String::from));
        }
        if show_recent && !dirs.is_empty() {
            items.push(MenuItem::Separator);
            paths.push(None);
            tips.push(None);
        }
        for file in &recent {
            let icon = reprisefs::icons::icon_for_file(file);
            items.push(MenuItem::entry(name(file)).icon(self.icons.get(icon, px)));
            paths.push(Some(file.clone()));
            tips.push(None);
        }
        if show_recent && recent.is_empty() {
            items.push(MenuItem::entry("(Empty)").enabled(false));
            paths.push(None);
            tips.push(None);
        }
        (items, MenuKind::Documents { paths, tips })
    }

    /// The submenu desktop row `command` opens: Arrange Icons By, New, Open
    /// With (listing `open_with`'s programs) or Send To.
    fn desktop_submenu(
        &mut self,
        command: DesktopCommand,
        output: &Output,
        open_with: impl FnOnce(&Self) -> Vec<(String, String)>,
    ) -> Option<(Vec<MenuItem>, Vec<Option<DesktopCommand>>)> {
        let px = self.context_icon_px(output);
        let (items, commands) = match command {
            DesktopCommand::ArrangeIcons => {
                crate::desktopview::arrange_menu(self.desktop_auto_arrange())
            }
            DesktopCommand::New => {
                let (mut items, commands) = crate::desktopview::new_menu();
                for (item, name) in items.iter_mut().zip(crate::desktopview::NEW_MENU_ICONS) {
                    if let (MenuItem::Entry { icon, .. }, Some(name)) = (item, name) {
                        *icon = self.icons.get(name, px);
                    }
                }
                (items, commands)
            }
            DesktopCommand::OpenWith => {
                let apps = open_with(self);
                let (mut items, commands) = crate::desktopview::open_with_menu(&apps);
                for (item, (id, _)) in items.iter_mut().zip(&apps) {
                    let name = self
                        .app_index
                        .iter()
                        .find(|a| &a.id == id)
                        .and_then(|a| a.icon.clone());
                    if let (MenuItem::Entry { icon, .. }, Some(name)) = (item, name) {
                        *icon = self.icons.get(&name, px);
                    }
                }
                (items, commands)
            }
            DesktopCommand::SendTo => {
                let view = self.desktop.as_mut()?;
                let rows = reprisefs::sendto::session_rows(view.drives.current());
                let (mut items, commands) = crate::desktopview::send_to_menu(&rows);
                for (item, r) in items.iter_mut().zip(&rows) {
                    if let MenuItem::Entry { icon, .. } = item {
                        *icon = self.icons.get(r.icon, px);
                    }
                }
                view.send_to = rows;
                (items, commands)
            }
            _ => return None,
        };
        Some((items, commands))
    }

    fn open_submenu(&mut self, depth: usize, row: usize) {
        self.schedule_menu_hover(None);
        self.close_menus_above(depth);
        let Some((items, kind, context)) = self.submenu_rows(depth, row) else {
            return;
        };
        let Some(parent) = self.menus.get(depth) else {
            return;
        };
        let Some(&local_row) = parent.layout.rows.get(row) else {
            return;
        };
        let parent_rect = Rect::new(
            parent.x,
            parent.y,
            parent.layout.width,
            parent.layout.height,
        );
        let row_rect = Rect::new(
            parent.x + local_row.x,
            parent.y + local_row.y,
            local_row.width,
            local_row.height,
        );
        let parent_inset = parent.layout.inset.right;
        let output = parent.output.clone();
        let usable = self.usable_area(Some(&output));
        let m = &self.config.theme.metrics;
        let style = if context {
            MenuStyle::context_for(&self.config.theme, &items)
        } else {
            MenuStyle::submenu(&self.config.theme)
        };
        let style = style.fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            usable.width,
        );
        let max_rows = submenu_max_rows(m, usable.height);
        let layout = MenuLayout::compute_styled(&style, &items, max_rows);
        let (x, y) = place_submenu_inset(
            parent_rect,
            row_rect,
            (layout.width, layout.height),
            usable,
            parent_inset,
            layout.inset,
        );
        let start_open = self
            .bars
            .iter()
            .any(|b| b.output == output && b.start_pressed);
        let floor = start_open
            .then(|| {
                let dock = self.config.theme.taskbar_shape().start_menu;
                let p = self.bar_placement(&output);
                start_submenu_shadow_floor(dock, &p, y)
            })
            .flatten();
        self.push_menu(depth + 1, (x, y), layout, items, output, kind);
        if let Some(m) = self.menus.last_mut() {
            m.parent_row = Some(row);
            m.shadow_floor = floor;
        }
        self.redraw_menu(depth + 1);
    }

    fn context_icon_px(&self, output: &Output) -> i32 {
        MenuStyle::context(&self.config.theme, ContextBitmaps::Icons).icon_px()
            * self.draw_scale_at(Some(output)).max(1)
    }

    pub(crate) fn push_context_menu(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: MenuKind,
    ) {
        self.push_context_menu_placed(at, items, kind, place_popup);
    }

    pub(crate) fn push_context_menu_placed(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: MenuKind,
        place: impl FnOnce((i32, i32), (i32, i32), Rect) -> (i32, i32),
    ) {
        let Some(output) = self.output_at(at.0 as f64, at.1 as f64) else {
            return;
        };
        let usable = self.usable_area(Some(&output));
        let style = MenuStyle::context_for(&self.config.theme, &items).fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            usable.width,
        );
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let pos = place(at, (layout.width, layout.height), usable);
        self.push_menu(0, pos, layout, items, output, kind);
        self.redraw_menu(0);
    }

    fn window_menu_state(&self, w: WindowId, from_taskbar: bool) -> Option<WindowMenuState> {
        let win = self.win(w)?;
        Some(WindowMenuState {
            maximized: win.maximized,
            minimized: win.minimized,
            from_taskbar,
            owned: win.is_owned() || win.dialog_tag != crate::window::DialogTag::None,
            resizable: win.is_resizable(),
            on_top: self.stays_on_top(w),
            desktops: self.desktops.count,
            sticky: self.win(self.home(w)).is_some_and(|h| h.sticky),
            tabs: self.tabbable(w).then(|| crate::winmenu::TabRows {
                targets: !self.tab_targets(w).is_empty(),
                grouped: self.tabs.groups.group_of(w).is_some(),
            }),
        })
    }

    pub fn open_window_menu(&mut self, w: WindowId, at: (i32, i32), from_taskbar: bool) {
        self.close_menu();
        let Some(state) = self.window_menu_state(w, from_taskbar) else {
            return;
        };
        let binds = &self.config.keybinds;
        let close_binds = || binds.iter().filter(|b| b.action == Action::Close);
        let close_keys = close_binds()
            .find(|b| b.keysym == keys::KEY_F4 && b.mods == keys::MOD_ALT)
            .or_else(|| close_binds().next())
            .map(|b| crate::winmenu::keybind_label(b.mods, b.keysym));
        let (items, rows) = crate::winmenu::window_menu(state, close_keys.as_deref());
        self.push_context_menu(at, items, MenuKind::Window { window: w, rows });
    }

    pub fn open_window_menu_below(&mut self, w: WindowId, from_taskbar: bool) {
        let Some(at) = self
            .win(w)
            .map(|x| (x.x + x.geom.caption.x, x.y + x.geom.caption.bottom()))
        else {
            return;
        };
        self.open_window_menu(w, at, from_taskbar);
    }

    fn window_menu_action(&mut self, w: WindowId, row: WindowMenuRow) {
        let Some(win) = self.win(w) else {
            return;
        };
        let minimized = win.minimized;
        match row {
            WindowMenuRow::Restore => {
                if minimized {
                    self.set_minimized(w, false);
                } else {
                    self.set_maximized(w, false);
                }
            }
            WindowMenuRow::Minimize => self.set_minimized(w, true),
            WindowMenuRow::Maximize => {
                if minimized {
                    self.set_minimized(w, false);
                }
                self.set_maximized(w, true);
            }
            WindowMenuRow::Move => self.begin_key_move(w),
            WindowMenuRow::Size => self.begin_key_size(w),
            WindowMenuRow::AlwaysOnTop => {
                let on = self.win(w).is_some_and(|x| x.on_top);
                self.set_on_top(w, !on);
            }
            WindowMenuRow::Close => self.close(w),
            WindowMenuRow::AllDesktops => {
                let on = !self.win(self.home(w)).is_some_and(|h| h.sticky);
                self.set_all_desktops(w, on);
            }
            WindowMenuRow::MoveTo(n) => self.move_to_desktop(&[w], n),
            WindowMenuRow::TabsOf(target) => self.join_tabs(w, target, usize::MAX, false),
            WindowMenuRow::LeaveTabs => self.leave_tabs(w),
            WindowMenuRow::MoveToMenu | WindowMenuRow::TabsOfMenu => {}
        }
    }

    pub fn open_taskbar_menu(&mut self, at: (i32, i32)) {
        self.close_menu();
        let has_windows = !self.task_windows().is_empty();
        let (items, commands) = crate::taskbar::taskbar_menu(self.bar_shared.locked, has_windows);
        self.push_context_menu(at, items, MenuKind::Taskbar { commands });
    }

    pub fn open_desktop_menu(&mut self, at: (i32, i32)) {
        self.close_menu();
        let (items, commands) = crate::desktopview::desktop_menu(
            self.clip_has_files(),
            self.config.desktop_icons,
            self.slideshow_running(),
        );
        self.desktop_menu_at = Some(at);
        let kind = MenuKind::Desktop {
            commands,
            target: None,
        };
        self.push_context_menu(at, items, kind);
    }

    pub fn open_icon_menu(&mut self, index: usize, at: (i32, i32)) {
        self.close_menu();
        let Some(target) = self
            .desktop
            .as_ref()
            .and_then(|d| d.entries.get(index))
            .map(|e| e.id())
        else {
            return;
        };
        let Some((items, commands)) = self.icon_menu_rows(index) else {
            return;
        };
        self.desktop_menu_at = None;
        let kind = MenuKind::Desktop {
            commands,
            target: Some(target),
        };
        self.push_context_menu(at, items, kind);
    }

    pub fn open_drop_menu(
        &mut self,
        at: (i32, i32),
        default: crate::dragview::DropOp,
        allowed: crate::dragview::Allowed,
    ) {
        self.close_menu();
        let system = self
            .drags
            .pending_drop
            .as_ref()
            .is_some_and(|(sources, _)| sources.iter().any(crate::dragview::DragSource::is_system));
        let (items, commands) = crate::desktopview::drop_menu(system, default, allowed);
        let kind = MenuKind::Desktop {
            commands,
            target: None,
        };
        self.push_context_menu(at, items, kind);
    }

    pub fn open_quick_menu(&mut self, index: usize, at: (i32, i32)) {
        let Some(file) = self.quicklaunch.get(index).map(|q| q.file.clone()) else {
            return;
        };
        self.close_menu();
        let (items, commands) = crate::taskbar::quick_menu();
        self.push_context_menu(at, items, MenuKind::Quick { file, commands });
    }

    fn jump_target(&self, tree: TreeId, i: usize) -> Option<JumpTarget> {
        let bar = self.bar(tree)?;
        let live = self.task_windows();
        let mut windows = bar.buttons.get(i)?.clone();
        windows.retain(|w| live.contains(w));
        let pin = bar
            .pins
            .get(i)
            .copied()
            .flatten()
            .and_then(|k| self.quicklaunch.get(k));
        let app_id = windows
            .first()
            .and_then(|&w| self.win(w))
            .map(|w| w.app_id());
        if pin.is_none() && app_id.is_none() {
            return None;
        }
        let entry = pin.and_then(|q| q.app.clone()).or_else(|| {
            let a = app_id.as_deref()?;
            reprisefs::apps::entry_for_app_id(&self.app_index, a, |_| true).cloned()
        });
        let terminal = &self.config.terminal;
        let name = match (pin, &entry) {
            (Some(q), _) => q.entry.label.clone(),
            (None, Some(e)) => e.name.clone(),
            (None, None) => crate::taskbar::group_name(app_id.as_deref().unwrap_or("")),
        };
        let launch = match (pin, &entry) {
            (Some(q), _) => Some(q.entry.action.clone()),
            (None, Some(e)) => Some(Action::Exec(reprisefs::apps::launch_argv(e, terminal))),
            (None, None) => None,
        };
        let source = match pin {
            Some(_) => None,
            None => entry
                .as_ref()
                .and_then(|e| reprisefs::apps::find_file(&e.id)),
        };
        Some(JumpTarget {
            name,
            launch,
            entry,
            pin_file: pin.map(|q| q.file.clone()),
            source,
            windows,
        })
    }

    pub(crate) fn window_icon(&mut self, w: WindowId, px: i32) -> Option<Canvas> {
        let win = self.windows.iter().find(|x| x.id == w)?;
        crate::icon::window_icon(&mut self.icons, &self.app_index, win, px)
            .map(Arc::unwrap_or_clone)
    }

    pub fn open_jump_list(&mut self, tree: TreeId, i: usize, at: (i32, i32)) {
        let Some(target) = self.jump_target(tree, i) else {
            return;
        };
        let recent = match &target.entry {
            Some(e) if self.config.recent_files => {
                reprisefs::recent::RecentFiles::new().for_app(crate::taskbar::JUMP_RECENT, e)
            }
            _ => Vec::new(),
        };
        let titles: Vec<String> = target
            .windows
            .iter()
            .filter_map(|&w| self.win(w))
            .map(|w| w.title())
            .collect();
        let (items, rows) = crate::taskbar::jump_list(
            &target.name,
            &recent,
            target.pin_file.is_some(),
            target.source.is_some(),
            &titles,
            self.desktops.count,
        );
        let Some(output) = self.output_at(at.0 as f64, at.1 as f64) else {
            return;
        };
        let px = self.context_icon_px(&output);
        let items = items
            .into_iter()
            .zip(&rows)
            .map(|(item, row)| match row {
                Some(JumpRow::Window(k)) => {
                    let icon = target
                        .windows
                        .get(*k)
                        .and_then(|&w| self.window_icon(w, px));
                    item.icon(icon)
                }
                _ => item,
            })
            .collect();
        self.close_menu();
        let kind = MenuKind::JumpList {
            rows,
            target: Box::new(target),
            button: i,
        };
        self.push_context_menu(at, items, kind);
    }

    fn jump_activate(&mut self, row: JumpRow, target: &JumpTarget) {
        let live = self.task_windows();
        let windows: Vec<WindowId> = target
            .windows
            .iter()
            .copied()
            .filter(|w| live.contains(w))
            .collect();
        match row {
            JumpRow::Recent(path) => self.open_recent_with(target.entry.as_ref(), &path),
            JumpRow::Window(k) => {
                if let Some(&w) = target.windows.get(k)
                    && live.contains(&w)
                {
                    self.bring_up(w);
                }
            }
            JumpRow::Launch => {
                if let Some(a) = &target.launch {
                    self.run_action(a, 0);
                }
            }
            JumpRow::Pin => {
                if let Some(src) = &target.source {
                    self.quick_add(src, None);
                }
            }
            JumpRow::Unpin => {
                if let Some(f) = &target.pin_file {
                    self.quick_remove(f);
                }
            }
            JumpRow::Close => {
                for w in windows {
                    self.close(w);
                }
            }
            JumpRow::MoveToMenu => {}
            JumpRow::MoveTo(n) => self.move_to_desktop(&windows, n),
        }
    }

    pub(crate) fn open_recent_with(&mut self, entry: Option<&AppEntry>, path: &std::path::Path) {
        let argv = match entry {
            Some(e) => reprisefs::apps::open_argv(e, &self.config.terminal, path),
            None => self.open_argv(path),
        };
        self.spawn_argv_activated(&argv);
    }

    pub fn open_window_list(&mut self, tree: TreeId, i: usize) {
        let Some((output, btn, mut wins)) = self.bar(tree).map(|b| {
            let btn = list_anchor(&b.layout, i);
            let wins = b.buttons.get(i).cloned().unwrap_or_default();
            (b.output.clone(), btn, wins)
        }) else {
            return;
        };
        let live = self.task_windows();
        wins.retain(|w| live.contains(w));
        if wins.len() < 2 {
            return;
        }
        self.close_menu();
        let style = MenuStyle::context(&self.config.theme, ContextBitmaps::Icons);
        let px = style.icon_px() * self.draw_scale_at(Some(&output)).max(1);
        let titles: Vec<String> = wins
            .iter()
            .map(|&w| self.win(w).map(|x| x.title()).unwrap_or_default())
            .collect();
        let (items, rows) = crate::taskbar::window_list(&titles, self.desktops.count);
        let items: Vec<MenuItem> = items
            .into_iter()
            .zip(&rows)
            .map(|(item, row)| match row {
                Some(WindowListRow::Window(k)) => {
                    let icon = self.window_icon(wins[*k], px);
                    item.icon(icon)
                }
                _ => item,
            })
            .collect();
        let usable = self.usable_area(Some(&output));
        let style = style.fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            usable.width,
        );
        let max_rows = submenu_max_rows(&self.config.theme.metrics, usable.height);
        let layout = MenuLayout::compute_styled(&style, &items, max_rows);
        let p = self.bar_placement(&output);
        let at = p.anchor_point(Rect::new(
            p.shown.x + btn.x,
            p.shown.y + btn.y,
            btn.width,
            btn.height,
        ));
        let pos = place_popup(at, (layout.width, layout.height), usable);
        let kind = MenuKind::WindowList {
            windows: wins,
            rows,
            button: i,
        };
        self.push_menu(0, pos, layout, items, output, kind);
        self.redraw_menu(0);
    }

    pub fn open_task_overflow(&mut self, tree: TreeId) {
        let Some(b) = self.bar(tree) else { return };
        let Some(more) = b.layout.more else { return };
        let output = b.output.clone();
        let (keys, rows): (Vec<_>, Vec<_>) = (b.layout.tasks.len()..b.buttons.len())
            .filter_map(|i| {
                let key = crate::taskbar::task_key(&b.buttons, &b.pins, &b.dialogs, i)?;
                let title = self.task_button_name(b, i).map(|(t, _)| t);
                let window = b.buttons.get(i)?.first().copied();
                Some((key, (title.unwrap_or_default(), window)))
            })
            .unzip();
        if keys.is_empty() {
            return;
        }
        self.close_menu();
        let style = MenuStyle::context(&self.config.theme, ContextBitmaps::Icons);
        let px = style.icon_px() * self.draw_scale_at(Some(&output)).max(1);
        let items: Vec<MenuItem> = rows
            .into_iter()
            .map(|(title, window)| {
                let icon = window.and_then(|w| self.window_icon(w, px));
                MenuItem::entry(title).icon(icon)
            })
            .collect();
        let usable = self.usable_area(Some(&output));
        let style = style.fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            usable.width,
        );
        let max_rows = submenu_max_rows(&self.config.theme.metrics, usable.height);
        let layout = MenuLayout::compute_styled(&style, &items, max_rows);
        let p = self.bar_placement(&output);
        let cell = Rect::new(
            p.shown.x + more.x,
            p.shown.y + more.y,
            more.width,
            more.height,
        );
        let pos = place_popup(p.anchor_point(cell), (layout.width, layout.height), usable);
        let kind = MenuKind::TaskOverflow { bar: tree, keys };
        self.push_menu(0, pos, layout, items, output, kind);
        self.redraw_menu(0);
        self.redraw_taskbars();
    }

    fn window_list_activate(&mut self, windows: &[WindowId], row: WindowListRow) {
        let all = self.task_windows();
        let live: Vec<WindowId> = windows
            .iter()
            .copied()
            .filter(|w| all.contains(w))
            .collect();
        match row {
            WindowListRow::Window(k) => {
                if let Some(&w) = windows.get(k)
                    && live.contains(&w)
                {
                    self.bring_up(w);
                }
            }
            WindowListRow::Minimize => {
                for w in live {
                    if self.win(w).is_some_and(|x| !x.minimized) {
                        self.set_minimized(w, true);
                    }
                }
            }
            WindowListRow::Close => {
                for w in live {
                    self.close(w);
                }
            }
            WindowListRow::MoveToMenu => {}
            WindowListRow::MoveTo(n) => self.move_to_desktop(&live, n),
        }
    }

    pub(crate) fn close_menus_above(&mut self, depth: usize) {
        self.schedule_menu_hover(None);
        crate::tray::menus_closed_above(self, depth);
        crate::tooltip::menus_closed_above(self, depth);
        while self.menus.len() > depth + 1 {
            if let Some(m) = self.menus.pop() {
                self.scene.destroy(m.tree);
            }
        }
        self.redraw_all();
        crate::access::touch(self);
    }

    /// Close every open menu. Also cancels a tray menu that was asked for
    /// and has not arrived yet, so callers may use it as "no menu from here
    /// on" even when nothing is open.
    pub fn close_menu(&mut self) {
        self.schedule_menu_hover(None);
        crate::tooltip::menus_closed(self);
        crate::tray::menus_closed(self);
        if self.menus.is_empty() {
            return;
        }
        for m in std::mem::take(&mut self.menus) {
            self.scene.destroy(m.tree);
        }
        self.menus_emptied();
    }

    fn close_topmost_menu(&mut self) {
        self.schedule_menu_hover(None);
        let Some(m) = self.menus.pop() else {
            return;
        };
        self.scene.destroy(m.tree);
        if self.menus.is_empty() {
            crate::tray::menus_closed(self);
            crate::tooltip::menus_closed(self);
            self.menus_emptied();
        } else {
            crate::tray::menus_closed_above(self, self.menus.len() - 1);
            crate::tooltip::menus_closed_above(self, self.menus.len() - 1);
            self.redraw_all();
            crate::access::touch(self);
        }
    }

    fn menus_emptied(&mut self) {
        self.menu_keys = false;
        self.drop_menu_closed();
        let held = self.start_panel_bar();
        for b in &mut self.bars {
            b.start_pressed = held == Some(b.tree);
        }
        if let Some(t) = self.menu_fade_timer.take() {
            self.handle.remove(t);
        }
        self.redraw_taskbars();
        self.redraw_all();
    }

    pub fn menus_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{self, ROOT, Tree, accesskit::Role};
        let menus = &self.menus;
        let rect_of = |m: &Menu| Rect::new(m.x, m.y, m.layout.width, m.layout.height);
        let first = menus.first()?;
        let all = menus
            .iter()
            .skip(1)
            .fold(rect_of(first), |u, m| u.union(&rect_of(m)));
        let mut tree = Tree::new(access::node(Role::Window, all));
        let (mut up, mut rows, mut focus) = (ROOT, Vec::new(), ROOT);
        for (k, m) in menus.iter().enumerate() {
            if k > 0 {
                let row = m.parent_row.or(menus[k - 1].hovered);
                up = row.and_then(|r| rows.get(r).copied()).unwrap_or(up);
            }
            let open = menus.get(k + 1).and_then(|n| n.parent_row.or(m.hovered));
            let (menu, ids, hot) = crate::startmenu::access_menu(
                &mut tree,
                up,
                k,
                &m.items,
                &m.layout,
                rect_of(m),
                m.hovered,
                open,
            );
            if k == 0
                && let Some(n) = tree.get_mut(menu)
            {
                n.set_label(stack_name(&m.kind));
            }
            focus = hot.unwrap_or(menu);
            rows = ids;
            up = menu;
        }
        tree.set_focus(focus);
        Some(tree)
    }

    pub fn close_window_menu_for(&mut self, w: WindowId) {
        if self.menu_root() == Some(MenuRoot::Window(w)) {
            self.close_menu();
        }
    }

    pub fn close_menus_off_scene(&mut self) {
        let gone = self
            .menus
            .first()
            .is_some_and(|m| !self.scene.outputs().contains(&m.output));
        if gone {
            self.close_menu();
        }
    }

    fn key_open_submenu(&mut self, depth: usize, row: usize) {
        if !self.submenu_open_for(depth, row) {
            if self.is_tray_menu(depth) {
                crate::tray::menu_hover(self, depth, Some(row));
                self.set_menu_hover(depth, Some(row));
                return;
            }
            self.open_submenu(depth, row);
        }
        let Some(items) = self.menus.get(depth + 1).map(|m| &m.items) else {
            return;
        };
        let first = step_selection(items, None, 1);
        self.set_menu_hover(depth + 1, first);
    }

    pub fn menu_key(&mut self, sym: u32) {
        let Some(depth) = self.menus.len().checked_sub(1) else {
            return;
        };
        self.schedule_menu_hover(None);
        self.enter_menu_keys();
        let act = {
            let menu = &self.menus[depth];
            key_act(&menu.items, menu.hovered, sym)
        };
        if let Some(act) = act {
            self.apply_menu_key(depth, act);
        }
    }

    fn apply_menu_key(&mut self, depth: usize, act: KeyAct) {
        match act {
            KeyAct::Nothing => {}
            KeyAct::Select(row) => self.set_menu_hover(depth, row),
            KeyAct::SelectAnd(row, then) => {
                self.set_menu_hover(depth, Some(row));
                self.apply_menu_key(depth, *then);
            }
            KeyAct::CloseTopmost => self.close_topmost_menu(),
            KeyAct::CloseSubmenu => {
                if depth > 0 {
                    self.close_menus_above(depth - 1);
                }
            }
            KeyAct::OpenSubmenu(row) => self.key_open_submenu(depth, row),
            KeyAct::Activate(row) => self.menu_activate(depth, row),
        }
    }

    pub fn menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        if self.is_tray_menu(depth) {
            crate::tray::menu_hover(self, depth, idx);
            self.set_menu_hover(depth, idx);
            return;
        }
        if let Some(i) = idx {
            let pending = if self.row_opens_submenu(depth, i) {
                (!self.submenu_open_for(depth, i)).then_some(MenuPending::Open { depth, row: i })
            } else {
                (self.menus.len() > depth + 1).then_some(MenuPending::CloseAbove { depth })
            };
            self.schedule_menu_hover(pending);
        }
        self.set_menu_hover(depth, idx);
    }

    fn schedule_menu_hover(&mut self, pending: Option<MenuPending>) {
        if self.menu_pending == pending {
            return;
        }
        self.menu_pending = pending;
        if let Some(t) = self.menu_timer.take() {
            self.handle.remove(t);
        }
        if pending.is_none() {
            return;
        }
        let ms = self.config.theme.metrics.menu_hover_ms.max(1) as u64;
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            |_, _, state| {
                state.menu_timer = None;
                state.menu_hover_fired();
                TimeoutAction::Drop
            },
        );
        match token {
            Ok(t) => self.menu_timer = Some(t),
            Err(e) => log::warn!("menu hover timer: {}", e.error),
        }
    }

    fn menu_hover_fired(&mut self) {
        match self.menu_pending.take() {
            Some(MenuPending::Open { depth, row }) => {
                let still = self
                    .menus
                    .get(depth)
                    .is_some_and(|m| m.hovered == Some(row));
                if still && !self.submenu_open_for(depth, row) {
                    self.open_submenu(depth, row);
                }
            }
            Some(MenuPending::CloseAbove { depth }) => self.close_menus_above(depth),
            None => {}
        }
    }

    pub fn clear_menu_hover(&mut self) {
        for depth in 0..self.menus.len() {
            self.set_menu_hover(depth, None);
        }
    }

    fn set_menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        let changed = match self.menus.get_mut(depth) {
            Some(m) if m.hovered != idx => {
                m.hovered = idx;
                true
            }
            _ => false,
        };
        if changed {
            self.redraw_menu(depth);
            crate::tooltip::hover_menu(self, depth, idx);
        }
    }

    fn arm_menu_fade(&mut self) {
        if self.menu_fade_timer.is_some() {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                state.menu_fade_tick()
            });
        match token {
            Ok(t) => self.menu_fade_timer = Some(t),
            Err(e) => log::warn!("menu fade timer: {}", e.error),
        }
    }

    fn menu_fade_tick(&mut self) -> TimeoutAction {
        let fade = self.config.theme.metrics.menu_fade_ms.max(1) as f32;
        let mut done = true;
        for m in &self.menus {
            let t = (m.opened.elapsed().as_secs_f32() * 1000.0 / fade).min(1.0);
            self.scene.set_opacity(m.tree, t);
            done &= t >= 1.0;
        }
        self.redraw_all();
        if done {
            self.menu_fade_timer = None;
            TimeoutAction::Drop
        } else {
            TimeoutAction::ToDuration(FADE_TICK)
        }
    }

    pub fn menu_row_at(&self, depth: usize, x: f64, y: f64) -> Option<usize> {
        let m = self.menus.get(depth)?;
        m.layout.hit(x as i32 - m.x, y as i32 - m.y)
    }

    pub fn menu_activate(&mut self, depth: usize, idx: usize) {
        if self.is_tray_menu(depth) {
            crate::tray::menu_activate(self, depth, idx);
            return;
        }
        if self.row_opens_submenu(depth, idx) {
            if self.submenu_open_for(depth, idx) {
                self.close_menus_above(depth);
            } else {
                self.open_submenu(depth, idx);
            }
            return;
        }
        let Some((pick, enabled)) = self.menus.get(depth).map(|m| {
            let enabled = matches!(
                m.items.get(idx),
                Some(MenuItem::Entry { enabled: true, .. })
            );
            (m.kind.pick(idx), enabled)
        }) else {
            return;
        };
        if !enabled {
            return;
        }
        if pick_closes_start_panel(&pick) {
            self.close_start_panel();
        }
        // A menu open with the launcher is one of its rows'.
        self.close_launcher();
        self.close_menu();
        match pick {
            MenuPick::Nothing => {}
            MenuPick::Run(a) => self.run_action(&a, 0),
            MenuPick::App(a) => self.launch_app(&a),
            MenuPick::Document(d) => {
                let argv = self.open_argv(&d);
                self.spawn_argv_activated(&argv);
            }
            MenuPick::Window { window, row } => self.window_menu_action(window, row),
            MenuPick::Taskbar(cmd) => self.taskbar_command(cmd),
            MenuPick::Quick { file, command } => match command {
                QuickCommand::Open => {
                    if let Some(k) = self.quicklaunch.iter().position(|q| q.file == file) {
                        self.run_quick(k);
                    }
                }
                QuickCommand::Delete => self.quick_remove(&file),
            },
            MenuPick::Desktop { command, target } => self.desktop_command(command, target),
            MenuPick::File { command, path } => self.file_command(command, path),
            MenuPick::WindowList { windows, row } => self.window_list_activate(&windows, row),
            MenuPick::Jump { row, target } => self.jump_activate(row, &target),
            MenuPick::NotifyAction { id, key } => crate::notifyview::invoke(self, id, &key),
            MenuPick::Layout(i) => self.set_layout(i),
            MenuPick::NightLight(row) => crate::nightlight::pick(self, row),
            MenuPick::StartPanel(row) => self.start_row(row),
            MenuPick::TaskOverflow { bar, key } => {
                let button = (self.bar(bar))
                    .and_then(|b| crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key));
                if let Some(button) = button {
                    self.task_button_click(bar, button, false);
                }
            }
        }
    }

    pub fn open_argv(&self, path: &std::path::Path) -> Vec<std::ffi::OsString> {
        let mime = reprisefs::apps::mime_of(path);
        let default = mime
            .as_deref()
            .and_then(|m| reprisefs::props::default_app_entry(m, &self.app_index));
        reprisefs::props::open_argv(path, default, &self.config.terminal)
    }

    fn taskbar_command(&mut self, cmd: TaskbarCommand) {
        use TaskbarCommand as Tc;
        match cmd {
            Tc::Cascade | Tc::TileHorizontal | Tc::TileVertical => self.arrange_windows(cmd),
            Tc::ShowDesktop => self.toggle_show_desktop(),
            Tc::Toolbars => {}
            Tc::TaskManager => {
                let cmd = self
                    .config
                    .task_manager
                    .clone()
                    .unwrap_or_else(|| format!("{} -e top", self.config.terminal));
                self.spawn_activated(&cmd);
            }
            Tc::Properties => {
                self.spawn_argv_activated(&["reprise-settings", "--page", "taskbar"]);
            }
            Tc::QuickLaunch => self.toggle_quick_shown(),
            Tc::LockTaskbar => self.toggle_taskbar_lock(),
        }
    }

    fn arrange_windows(&mut self, cmd: TaskbarCommand) {
        use TaskbarCommand as Tc;
        let p = self.pointer_location();
        let out = self.output_at(p.x, p.y);
        let usable = self.usable_area(out.as_ref());
        let mut wins: Vec<WindowId> = self
            .task_windows()
            .into_iter()
            .filter(|&w| {
                self.win(w).is_some_and(|x| {
                    !x.hidden() && self.output_at(x.x as f64 + 1.0, x.y as f64 + 1.0) == out
                })
            })
            .collect();
        wins.sort_by_key(|w| self.focus_order.iter().position(|f| f == w));
        let rects = match cmd {
            Tc::Cascade => {
                crate::arrange::cascade(wins.len(), usable, self.config.theme.metrics.cascade_step)
            }
            Tc::TileHorizontal => crate::arrange::tile_horizontal(wins.len(), usable),
            Tc::TileVertical => crate::arrange::tile_vertical(wins.len(), usable),
            _ => return,
        };
        for (&w, r) in wins.iter().zip(&rects) {
            if self.win(w).is_some_and(|x| x.maximized) {
                self.set_maximized(w, false);
            }
            let Some(chrome) = self.win(w).map(|x| x.chrome()) else {
                continue;
            };
            let (cw, ch) = reprisekit::classic::frame::FrameGeometry::client_size_for_total(
                &self.config.theme.metrics,
                r.width,
                r.height,
                false,
                chrome,
            );
            self.move_to(w, r.x, r.y);
            self.request_size(w, cw, ch);
        }
        for &w in &wins {
            self.focus_order.retain(|p| *p != w);
            self.focus_order.push(w);
            if let Some(tree) = self.win(w).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
            self.raise_children(w);
        }
        if let Some(&top) = wins.last() {
            self.focus_window(top);
        }
    }

    pub fn toggle_show_desktop(&mut self) {
        crate::peek::leave(self);
        let all = self.task_windows();
        let any_visible = all
            .iter()
            .any(|&w| self.win(w).is_some_and(|x| !x.hidden()));
        for w in all {
            let minimized = self.win(w).is_some_and(|x| x.minimized);
            if minimized != any_visible {
                self.set_minimized(w, any_visible);
            }
        }
    }

    fn quick_reload(&mut self) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        let order: Vec<String> = self.quicklaunch.iter().map(|q| q.file.clone()).collect();
        let mut items = crate::quicklaunch::scan(&dir, &self.config.terminal);
        reprisefs::quicklaunch::apply_order(&mut items, &order, |q| &q.file);
        self.quicklaunch = items;
        self.redraw_taskbars();
    }

    pub(crate) fn quick_add(&mut self, src: &std::path::Path, slot: Option<usize>) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        let Some(file) = crate::quicklaunch::add(&dir, src) else {
            return;
        };
        self.quick_reload();
        let len = self.quicklaunch.len();
        if let Some(from) = self.quicklaunch.iter().position(|q| q.file == file) {
            crate::taskbar::move_button(&mut self.quicklaunch, from, slot.unwrap_or(len));
            self.quick_order_changed();
        }
        self.redraw_taskbars();
    }

    pub(crate) fn quick_remove(&mut self, file: &str) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        if !self.quicklaunch.iter().any(|q| q.file == file) {
            return;
        }
        crate::quicklaunch::remove(&dir, file);
        self.quick_reload();
    }
}

/// Whether picking `pick` closes the Glass Start menu as well. A menu open
/// with the panel is one of its popups: a pick closes the panel too, but
/// for the panel's own rows, which decide (`start_row`). A menu key never
/// reaches the panel otherwise: Escape (`KeyAct::CloseTopmost`) and Left
/// close menus only.
fn pick_closes_start_panel(pick: &MenuPick) -> bool {
    !matches!(pick, MenuPick::StartPanel(_))
}

fn stack_name(kind: &MenuKind) -> &'static str {
    match kind {
        MenuKind::Entries(_) => "Start Menu",
        MenuKind::Window { .. } => "Window Menu",
        _ => "Context Menu",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KEY_KP_Enter, KEY_Return};

    #[test]
    fn an_overflow_rows_pick_names_its_bar_and_button() {
        use crate::taskbar::TaskKey;
        let mut scene = crate::compositor::DeskScene::new();
        let bar = scene.tree_create(scene.root());
        let w = WindowId::next();
        let kind = MenuKind::TaskOverflow {
            bar,
            keys: vec![TaskKey::Pin(2), TaskKey::Window(w), TaskKey::Dialog(9)],
        };
        assert_eq!(
            kind.pick(1),
            MenuPick::TaskOverflow {
                bar,
                key: TaskKey::Window(w)
            }
        );
        assert_eq!(kind.pick(3), MenuPick::Nothing);
    }

    #[test]
    fn an_overflow_row_finds_its_button_after_the_row_changes() {
        use crate::taskbar::{find_task, task_key};
        let mut scene = crate::compositor::DeskScene::new();
        let bar = scene.tree_create(scene.root());
        let [a, b, c] = [WindowId::next(), WindowId::next(), WindowId::next()];
        let opened = [vec![a], vec![b], vec![c]];
        let kind = MenuKind::TaskOverflow {
            bar,
            keys: (1..3)
                .filter_map(|i| task_key(&opened, &[], &[], i))
                .collect(),
        };
        let MenuPick::TaskOverflow { key, .. } = kind.pick(1) else {
            panic!("row 1 is c's button");
        };
        let a_closed = [vec![b], vec![c]];
        assert_eq!(
            find_task(&a_closed, &[], &[], &key),
            Some(1),
            "one place up"
        );
        let c_closed = [vec![a], vec![b]];
        assert_eq!(find_task(&c_closed, &[], &[], &key), None, "gone");
    }

    #[test]
    fn a_window_list_hangs_off_its_button_or_the_more_cell() {
        use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge, TaskbarCounts, TaskbarLayout};
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let span = BarSpan {
            edge: Edge::Left,
            length: 400,
            thickness: 60,
        };
        let counts = TaskbarCounts {
            tasks: 8,
            ..TaskbarCounts::default()
        };
        let shape = glass.taskbar_shape();
        let l = TaskbarLayout::compute_on(
            &glass.metrics,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        let more = l.more.expect("eight 40 px buttons overrun 400 px");
        assert_eq!(l.tasks.len(), 6);
        assert_eq!(
            list_anchor(&l, 1),
            l.tasks[1],
            "a button on the column: itself"
        );
        assert_eq!(
            list_anchor(&l, 7),
            more,
            "past the column's end: the » cell"
        );
        let p = crate::barplace::place(Rect::new(0, 0, 1920, 1080), Edge::Left, 60);
        assert_eq!(
            p.anchor_point(list_anchor(&l, 7)),
            (60, 314),
            "right of the bar, level with »"
        );
    }

    #[test]
    fn escape_in_a_start_panel_popup_closes_the_popup_only() {
        use crate::keyroute::{KeyPress, KeyRoute, Modes, route};
        let modes = Modes {
            start_panel_open: true,
            menus_open: true,
            ..Modes::default()
        };
        let escape = KeyPress {
            syms: vec![keys::KEY_Escape],
            lower: vec![keys::KEY_Escape],
            mods: 0,
            pressed: true,
        };
        assert_eq!(
            route(&modes, &escape, &[], &[]).route,
            KeyRoute::Menu(keys::KEY_Escape)
        );
        let (items, rows) = crate::startmenu::shutdown_menu(false);
        assert_eq!(
            key_act(&items, Some(0), keys::KEY_Escape),
            Some(KeyAct::CloseTopmost)
        );
        let kind = MenuKind::StartPanel { rows };
        assert!(
            !pick_closes_start_panel(&kind.pick(0)),
            "Log off is the panel's"
        );
        assert!(pick_closes_start_panel(&MenuPick::Taskbar(
            TaskbarCommand::Cascade
        )));
    }

    #[test]
    fn the_start_menu_docks_on_the_screen_side_of_the_bar() {
        use reprisekit::taskbar::{Edge, StartMenuDock};
        let screen = Rect::new(0, 0, 1920, 1080);
        let at = |dock: StartMenuDock, edge: Edge, height: i32| {
            let p = crate::barplace::place(screen, edge, 30);
            let start = Rect::new(2, p.shown.y + 4, 54, 22);
            start_menu_at(
                dock,
                &p,
                start,
                (300, height),
                crate::barplace::usable(screen, &p),
            )
            .1
        };
        assert_eq!(
            at(StartMenuDock::Into(4), Edge::Bottom, 400),
            1050 + 4 - 400
        );
        assert_eq!(at(StartMenuDock::OnTop, Edge::Bottom, 400), 1050 - 400);
        assert_eq!(at(StartMenuDock::Into(4), Edge::Bottom, 2000), 0);
        assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 400), 30 - 4);
        assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 400), 30);
        assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 1054), 30 - 4);
        assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 1060), 1080 - 1060);
        assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 1060), 1080 - 1060);
        assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 2000), 0);
    }

    #[test]
    fn a_hidden_bars_start_menu_stands_on_its_reveal_line() {
        use reprisekit::taskbar::{Edge, StartMenuDock};
        let screen = Rect::new(0, 0, 1920, 1080);
        let at = |dock: StartMenuDock, edge: Edge, height: i32| {
            let p = crate::barplace::place_hiding(screen, edge, 30, Some(1.0));
            let start = Rect::new(2, p.shown.y + 4, 54, 22);
            start_menu_at(
                dock,
                &p,
                start,
                (300, height),
                crate::barplace::usable(screen, &p),
            )
            .1
        };
        for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
            assert_eq!(
                at(dock, Edge::Bottom, 400) + 400,
                1078,
                "{dock:?}: over a bottom bar the menu's foot is the line's top"
            );
            assert_eq!(
                at(dock, Edge::Top, 400),
                2,
                "{dock:?}: under a top bar the menu's top is the line's foot"
            );
            assert_eq!(
                at(dock, Edge::Bottom, 2000),
                0,
                "{dock:?}: a tall menu stops at the top"
            );
            assert_eq!(
                at(dock, Edge::Top, 2000),
                0,
                "{dock:?}: and starts at the top"
            );
        }
        for edge in [Edge::Bottom, Edge::Right] {
            let p = crate::barplace::place_hiding(screen, edge, 30, Some(1.0));
            assert_eq!(
                start_menu_shadow_floor(StartMenuDock::Into(4), &p),
                Some(0),
                "{edge:?}"
            );
        }
        let shown = crate::barplace::place_hiding(screen, Edge::Bottom, 30, Some(0.0));
        assert_eq!(
            start_menu_shadow_floor(StartMenuDock::Into(4), &shown),
            None,
            "its shadow falls on the bar it reaches into"
        );
        let hidden = crate::barplace::place_hiding(screen, Edge::Bottom, 30, Some(1.0));
        assert_eq!(
            start_submenu_shadow_floor(StartMenuDock::Into(4), &hidden, 700),
            Some(1078 - 700)
        );
        assert_eq!(
            start_submenu_shadow_floor(StartMenuDock::Into(4), &shown, 700),
            None
        );
        let area = crate::barplace::usable(screen, &shown);
        assert_eq!(
            start_menu_at(
                StartMenuDock::Into(4),
                &shown,
                Rect::new(2, shown.shown.y + 4, 54, 22),
                (300, 400),
                area
            )
            .1,
            1050 + 4 - 400
        );
    }

    #[test]
    fn the_tray_cells_menus_pick_their_rows() {
        use crate::nightlight::Row;
        assert_eq!(MenuKind::Layouts.pick(2), MenuPick::Layout(2));
        let night = MenuKind::NightLight(vec![Some(Row::Pause), None, Some(Row::Settings)]);
        assert_eq!(night.pick(0), MenuPick::NightLight(Row::Pause));
        assert_eq!(night.pick(1), MenuPick::Nothing);
        assert_eq!(night.pick(2), MenuPick::NightLight(Row::Settings));
    }

    #[test]
    fn documents_folders_have_their_infotips() {
        let docs = MenuKind::Documents {
            paths: vec![
                Some("/home/u/Documents".into()),
                None,
                Some("/tmp/a.txt".into()),
            ],
            tips: vec![
                crate::desktop::document_tip("folder-documents").map(String::from),
                None,
                None,
            ],
        };
        assert_eq!(
            docs.tip(0).as_deref(),
            Some("Contains letters, reports, and other documents and files.")
        );
        assert_eq!(docs.tip(1), None);
        assert_eq!(docs.tip(2), None);
    }

    #[test]
    fn enter_runs_the_default_row_when_nothing_is_highlighted() {
        let rows = |enabled: bool| {
            vec![
                MenuItem::item("&Copy Here"),
                MenuItem::item("&Move Here").bold(true).enabled(enabled),
                MenuItem::item("Create &Shortcuts Here"),
                MenuItem::Separator,
                MenuItem::item("Cancel"),
            ]
        };
        for sym in [KEY_Return, KEY_KP_Enter] {
            assert_eq!(
                key_act(&rows(true), None, sym),
                Some(KeyAct::SelectAnd(1, Box::new(KeyAct::Activate(1))))
            );
            assert_eq!(
                key_act(&rows(true), Some(0), sym),
                Some(KeyAct::Activate(0))
            );
            assert_eq!(key_act(&rows(false), None, sym), Some(KeyAct::Nothing));
        }
        let plain = [MenuItem::item("&Open"), MenuItem::item("&Delete")];
        assert_eq!(key_act(&plain, None, KEY_Return), Some(KeyAct::Nothing));
        let sub = [MenuItem::item("Send &To").bold(true).submenu(true)];
        assert_eq!(
            key_act(&sub, None, KEY_Return),
            Some(KeyAct::SelectAnd(0, Box::new(KeyAct::OpenSubmenu(0))))
        );
    }

    #[test]
    fn access_keys_pick_or_cycle_and_escape_closes() {
        use crate::keys::{KEY_Escape, KEY_d, KEY_o};
        let items = [
            MenuItem::item("&Open"),
            MenuItem::item("&Delete"),
            MenuItem::item("&Details"),
        ];
        assert_eq!(
            key_act(&items, None, KEY_o),
            Some(KeyAct::SelectAnd(0, Box::new(KeyAct::Activate(0))))
        );
        assert_eq!(key_act(&items, None, KEY_d), Some(KeyAct::Select(Some(1))));
        assert_eq!(
            key_act(&items, Some(1), KEY_d),
            Some(KeyAct::Select(Some(2)))
        );
        assert_eq!(
            key_act(&items, None, KEY_Escape),
            Some(KeyAct::CloseTopmost)
        );
    }

    #[test]
    fn beside_a_side_bar_the_start_menu_hangs_level_with_start() {
        use reprisekit::taskbar::{Edge, StartMenuDock};
        let screen = Rect::new(0, 0, 1920, 1080);
        let at = |dock, edge, start: Rect, height| {
            let p = crate::barplace::place(screen, edge, 60);
            start_menu_at(
                dock,
                &p,
                start,
                (300, height),
                crate::barplace::usable(screen, &p),
            )
        };
        let start = Rect::new(2, 2, 54, 22);
        assert_eq!(
            at(StartMenuDock::Into(4), Edge::Left, start, 400),
            (60 - 4, 2),
            "4 px into a left bar, level with Start's top"
        );
        assert_eq!(at(StartMenuDock::OnTop, Edge::Left, start, 400), (60, 2));
        let start = Rect::new(1864, 2, 54, 22);
        assert_eq!(
            at(StartMenuDock::Into(4), Edge::Right, start, 400),
            (1860 + 4 - 300, 2)
        );
        assert_eq!(
            at(StartMenuDock::OnTop, Edge::Right, start, 400),
            (1860 - 300, 2)
        );
        let low = Rect::new(2, 1000, 54, 22);
        assert_eq!(at(StartMenuDock::OnTop, Edge::Left, low, 400).1, 1080 - 400);
        assert_eq!(at(StartMenuDock::OnTop, Edge::Left, start, 2000).1, 0);
        let narrow = Rect::new(1920, 0, 320, 1080);
        let side = |dock, edge| {
            let p = crate::barplace::place(narrow, edge, 60);
            let start = Rect::new(p.shown.x + 2, 2, 54, 22);
            start_menu_at(
                dock,
                &p,
                start,
                (300, 400),
                crate::barplace::usable(narrow, &p),
            )
            .0
        };
        for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
            assert_eq!(
                side(dock, Edge::Right),
                1920,
                "{dock:?}: a right bar's menu stops at the output's left edge"
            );
            assert_eq!(
                side(dock, Edge::Left),
                2240 - 300,
                "{dock:?}: a left bar's menu stops at the output's right edge"
            );
        }
        let p = crate::barplace::place_hiding(screen, Edge::Left, 60, Some(1.0));
        let area = crate::barplace::usable(screen, &p);
        assert_eq!(
            start_menu_at(StartMenuDock::Into(4), &p, start, (300, 400), area).0,
            2
        );
    }
}
