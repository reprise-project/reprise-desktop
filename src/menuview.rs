//! The menu stack on the scene: Start menu and submenus, window, taskbar, Quick
//! Launch and desktop menus, jump lists, window lists and tray DBusMenus. Layout
//! and drawing come from `startmenu` (reprisekit); this module opens, places,
//! drives and closes them. Each open menu is a tree in `layers.menu` (a reply
//! field's edit menu in `layers.overlay`), tagged `SceneOwner::Menu(depth)`,
//! holding its shadow (no input) under its buffer.
//! Menus carry no glass, so the glass below never blurs a menu.

mod activate;
mod context;
mod input;
mod jump;
mod stack;
mod submenu;
mod window_list;

#[cfg(test)]
mod tests;

use std::{
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use reprisefs::apps::AppEntry;
use reprisekit::anim::UnrollFrom;
use reprisekit::taskbar::{Edge, StartMenuDock};
use reprisescene::{BufferId, NodeId, TreeId};
use smithay::output::Output;

use crate::{
    barplace::BarPlacement,
    config::{Action, MenuEntry, MenuTarget},
    desktop::IconId,
    desktopview::DesktopCommand,
    keys,
    raster::{Canvas, Rect},
    startmenu::{
        KeyStops, MenuItem, MenuLayout, StartRow, enter_row, relist, rows_for_key, step_selection,
    },
    taskbar::{JumpRow, QuickCommand, TaskKey, TaskbarCommand, WindowListRow},
    window::WindowId,
    winmenu::WindowMenuRow,
};

#[cfg(test)]
use crate::startmenu::MenuStyle;

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
        /// The document folders head the list (the Start menu's Documents, not Recent Items).
        dirs: bool,
        icon_px: i32,
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
        facts: crate::desktop::FileFacts,
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
    /// The Quick Launch band's » menu: per row, the hidden launcher's file.
    QuickOverflow {
        bar: TreeId,
        files: Vec<String>,
    },
    /// A field's edit menu (`editmenuview`), its rows `editmenu::items`.
    Edit {
        editor: crate::caret::Editor,
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
        facts: crate::desktop::FileFacts,
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
    QuickOverflow(String),
    Edit {
        editor: crate::caret::Editor,
        command: reprisekit::controls::editmenu::Command,
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
            MenuKind::File {
                commands,
                path,
                facts,
            } => match commands.get(idx).cloned().flatten() {
                Some(command) => MenuPick::File {
                    command,
                    path: path.clone(),
                    facts: facts.clone(),
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
            MenuKind::QuickOverflow { files, .. } => files
                .get(idx)
                .cloned()
                .map_or(MenuPick::Nothing, MenuPick::QuickOverflow),
            MenuKind::Edit { editor } => reprisekit::controls::editmenu::command(&[idx]).map_or(
                MenuPick::Nothing,
                |command| MenuPick::Edit {
                    editor: *editor,
                    command,
                },
            ),
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
    pub from: MenuFrom,
    pub parent_row: Option<usize>,
    /// Laid out and drawn with the style's Shut down options look ([`reprisekit::theme::Theme::for_options_menu`]).
    options_look: bool,
    opened: Instant,
    /// The animation still running since `opened`.
    opening: Option<Opening>,
    /// Where an unrolling menu draws from `x`, `y` this frame.
    shift: (i32, i32),
    icons: RowIcons,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opening {
    Fade,
    Unroll(UnrollFrom),
}

/// The side a menu opened from a bar on `edge` unrolls from.
pub(crate) fn unroll_from_bar(edge: Edge) -> UnrollFrom {
    match edge {
        Edge::Bottom => UnrollFrom::Bottom,
        Edge::Top => UnrollFrom::Top,
        Edge::Left => UnrollFrom::Left,
        Edge::Right => UnrollFrom::Right,
    }
}

/// Where a menu was opened. One opened from the taskbar, or over the Start panel, keeps an
/// auto-hidden bar shown while it is up; one opened elsewhere does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuFrom {
    Bar,
    /// Over the Start panel: unrolls facing the press, as a context menu elsewhere does.
    StartPanel,
    Elsewhere,
}

impl MenuFrom {
    pub fn holds_bar(self) -> bool {
        matches!(self, MenuFrom::Bar | MenuFrom::StartPanel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRoot {
    Window(WindowId),
    WindowList(usize),
    JumpList(usize),
    TaskOverflow(TreeId),
    QuickOverflow(TreeId),
    Other,
}

impl Menu {
    /// Relists a Documents menu or a jump list as `items` and `kind`, built afresh by
    /// `documents_rows` (with its `icons`) or `jump_relisted`, in place. Returns whether a row
    /// changed. `fit` gives the column width the new rows need (`MenuLayout::fit_column`).
    pub(crate) fn relist_to(
        &mut self,
        items: Vec<MenuItem>,
        kind: MenuKind,
        icons: RowIcons,
        fit: impl Fn(&MenuLayout, &[MenuItem]) -> i32,
    ) -> bool {
        match (&mut self.kind, kind) {
            (
                MenuKind::Documents { paths, tips, .. },
                MenuKind::Documents {
                    paths: new_paths,
                    tips: new_tips,
                    ..
                },
            ) => {
                let keys = |paths: &[Option<PathBuf>],
                            items: &[MenuItem]|
                 -> Vec<(Option<PathBuf>, bool)> {
                    paths
                        .iter()
                        .zip(items)
                        .map(|(p, it)| (p.clone(), matches!(it, MenuItem::Separator)))
                        .collect()
                };
                let (old, new) = (keys(paths, &self.items), keys(&new_paths, &items));
                if old == new {
                    return false;
                }
                let column = fit(&self.layout, &items);
                relist(
                    &mut self.items,
                    &mut self.layout,
                    &mut self.hovered,
                    &old,
                    &new,
                    items,
                    column,
                );
                *paths = new_paths;
                *tips = new_tips;
                self.icons = icons;
            }
            (MenuKind::JumpList { rows, .. }, MenuKind::JumpList { rows: new_rows, .. }) => {
                if *rows == new_rows {
                    return false;
                }
                let column = fit(&self.layout, &items);
                relist(
                    &mut self.items,
                    &mut self.layout,
                    &mut self.hovered,
                    rows,
                    &new_rows,
                    items,
                    column,
                );
                let shift = (jump_lead(rows), jump_lead(&new_rows));
                self.icons = std::mem::take(&mut self.icons).shifted(shift.0, shift.1);
                *rows = new_rows;
            }
            _ => return false,
        }
        true
    }
}

/// How many rows a jump list's recent files take: theirs and the separator under them.
fn jump_lead(rows: &[Option<JumpRow>]) -> usize {
    let listed = rows
        .iter()
        .take_while(|r| matches!(r, Some(JumpRow::Recent(_))))
        .count();
    if listed > 0 { listed + 1 } else { 0 }
}

/// An open jump list's `items` and `rows` with its recent files replaced by `recent`.
pub(crate) fn jump_relisted(
    items: &[MenuItem],
    rows: &[Option<JumpRow>],
    recent: &[PathBuf],
) -> (Vec<MenuItem>, Vec<Option<JumpRow>>) {
    let rest = jump_lead(rows);
    let (mut new_items, mut new_rows) = crate::taskbar::jump_recent_rows(recent);
    new_items.extend(items.iter().skip(rest).cloned());
    new_rows.extend(rows.iter().skip(rest).cloned());
    (new_items, new_rows)
}

#[cfg(test)]
impl Menu {
    /// An empty menu of `kind` on `output`, with buffers in `scene`.
    pub(crate) fn for_test(
        scene: &mut crate::compositor::DeskScene,
        output: Output,
        kind: MenuKind,
        from: MenuFrom,
    ) -> Menu {
        let tree = scene.tree_create(scene.root());
        let theme = crate::config::Config::default().theme;
        Menu {
            tree,
            buffer: scene.buffer_create(tree),
            shadow: scene.buffer_create(tree),
            shadow_floor: None,
            shadow_drawn: None,
            layout: MenuLayout::compute_styled(&MenuStyle::root(&theme), &[], usize::MAX),
            items: Vec::new(),
            x: 0,
            y: 0,
            hovered: None,
            output,
            kind,
            from,
            parent_row: None,
            options_look: false,
            opened: Instant::now(),
            opening: None,
            shift: (0, 0),
            icons: RowIcons::default(),
        }
    }
}

pub(crate) const RECENT_DOCUMENTS: usize = 15;

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
fn key_act(
    items: &[MenuItem],
    current: Option<usize>,
    sym: u32,
    stops: KeyStops,
) -> Option<KeyAct> {
    use keys::*;
    let open_or_run = |row: usize| {
        if row_has_submenu(items, row) {
            KeyAct::OpenSubmenu(row)
        } else {
            KeyAct::Activate(row)
        }
    };
    Some(match sym {
        KEY_Down | KEY_KP_Down => KeyAct::Select(step_selection(items, current, 1, stops)),
        KEY_Up | KEY_KP_Up => KeyAct::Select(step_selection(items, current, -1, stops)),
        KEY_Home | KEY_KP_Home => KeyAct::Select(step_selection(items, None, 1, stops)),
        KEY_End | KEY_KP_End => KeyAct::Select(step_selection(items, None, -1, stops)),
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

/// Where a menu row's icon comes from, so an icon theme change can look it up again.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RowIcon {
    Name(String),
    Window(WindowId),
}

/// The icons a menu's rows wear.
#[derive(Debug, Default)]
pub(crate) struct RowIcons {
    /// (row, source, px) for every row with a looked-up icon.
    from: Vec<(usize, RowIcon, i32)>,
    /// (row, icon file, px) for the rows showing a stand-in until a job decodes the file.
    waits: Vec<(usize, String, i32)>,
}

impl RowIcons {
    /// The rows at and past `old` move to `new`, as a jump list's do when its recent files change.
    fn shifted(mut self, old: usize, new: usize) -> RowIcons {
        self.from.retain(|(row, ..)| *row >= old);
        self.waits.retain(|(row, ..)| *row >= old);
        for row in
            (self.from.iter_mut().map(|f| &mut f.0)).chain(self.waits.iter_mut().map(|w| &mut w.0))
        {
            *row = *row - old + new;
        }
        self
    }
}

/// Puts `art`, icon file `file` decoded at `px`, on the rows of `items` that `waits` holds for
/// it. True when a row took it.
fn land_icon(
    items: &mut [MenuItem],
    waits: &mut Vec<(usize, String, i32)>,
    file: &str,
    px: i32,
    art: &Option<Canvas>,
) -> bool {
    let before = waits.len();
    waits.retain(|(row, f, p)| {
        if f != file || *p != px {
            return true;
        }
        if let Some(MenuItem::Entry { icon, .. }) = items.get_mut(*row) {
            icon.clone_from(art);
        }
        false
    });
    waits.len() != before
}

/// What Show Desktop minimises and restores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Window(WindowId),
    Dialog(u64),
}

/// `items` bottom first by their trees' place in `stack` (the windows layer's children), trees in
/// another layer first: restoring each in turn raises it, leaving them stacked as they were.
fn restore_order<T>(stack: &[NodeId], mut items: Vec<(TreeId, T)>) -> Vec<(TreeId, T)> {
    items.sort_by_key(|(tree, _)| stack.iter().position(|&n| n == NodeId::from(*tree)));
    items
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
