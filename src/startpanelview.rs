use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisefs::apps::{AppEntry, Group};
use reprisekit::startpanel::layout::{self, Layout};
use reprisekit::startpanel::{
    self, Button, Event, Key, Mode, PlaceRow, ProgramRow, ResultGroup, ResultRow, Spot, StartPanel,
    TreeRow,
};
use reprisekit::taskbar::{Edge, StartMenuDock};
use reprisekit::textedit::{Preedit, TextSpot};
use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Logical, Point};

use crate::barplace::BarPlacement;
use crate::compositor::{SceneOwner, State};
use crate::config::{Action, MenuEntry, PowerAction};
use crate::dialogview::DialogKey;
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::input::{BTN_LEFT, BTN_RIGHT};
use crate::keys;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::search::{
    CALC_ICON, FileSearch, SearchOwner, Setting, calc_row, file_row, rank_settings, word_starts,
};
use crate::startmenu::{MenuItem, StartRow, place_popup};

const PROGRAM_ICON: i32 = 32;
const PICTURE: i32 = 48;
const TREE_ICON: i32 = 16;
const WHEEL_ROWS: i64 = 3;
const SEARCH_ROWS: usize = 8;
const SEARCH_SETTINGS: usize = 4;
const RESULT_ICON: i32 = 16;
const FADE_TICK: Duration = Duration::from_millis(16);
const CROSSFADE: Duration = Duration::from_millis(350);
const JUMP_TITLE: &str = "Recent";
const ACCOUNTS: &str = "reprise-accounts";

pub struct StartPanelView {
    pub model: StartPanel,
    bar: TreeId,
    output: Output,
    /// Indices into `app_index` parallel to `model.pinned` then `model.frequent`.
    programs: Vec<usize>,
    /// Leading `model.frequent` rows from launch history; the rest are starters.
    launched: usize,
    jumps: Vec<Vec<PathBuf>>,
    new_pins: Vec<String>,
    /// Parallel to `model.tree`; None for a folder.
    tree_apps: Vec<Option<usize>>,
    result_apps: Vec<usize>,
    answer: Option<String>,
    result_settings: Vec<Setting>,
    result_kinds: Vec<ResultKind>,
    pub search: FileSearch,
    /// Wheel travel short of a notch, in 120ths.
    wheel: i32,
    /// `startmenu::places()` without its rules, parallel to `model.places`.
    places: Vec<MenuEntry>,
    pressed: Option<Spot>,
    hover: Option<RegistrationToken>,
    opened: Instant,
    fade: Option<RegistrationToken>,
    /// Layout coordinates.
    at: (i32, i32),
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
    buffer: BufferId,
    drawn: Option<Drawn>,
    rasters: Rasters,
    crossfades: Vec<Crossfade>,
    crossfade_timer: Option<RegistrationToken>,
}

/// What a panel canvas is drawn from, besides the look.
#[derive(Clone, PartialEq)]
struct RasterKey {
    model: StartPanel,
    pressed: Option<Spot>,
    caret: bool,
    preedit: Option<Preedit>,
    scale: i32,
}

/// The last two canvases drawn, so a caret blink back to a drawn state reuses its canvas.
#[derive(Default)]
struct Rasters(Vec<(RasterKey, Rc<Canvas>, Option<TextSpot>)>);

impl Rasters {
    fn find(&self, key: &RasterKey) -> Option<(Rc<Canvas>, Option<TextSpot>)> {
        let (_, canvas, spot) = self.0.iter().find(|(k, ..)| k == key)?;
        Some((canvas.clone(), spot.clone()))
    }

    fn keep(&mut self, key: RasterKey, canvas: Rc<Canvas>, spot: Option<TextSpot>) {
        if self.0.len() == 2 {
            self.0.remove(0);
        }
        self.0.push((key, canvas, spot));
    }
}

struct Drawn {
    canvas: Rc<Canvas>,
    mode: Mode,
    picture: Option<usize>,
}

struct Crossfade {
    from: Rc<Canvas>,
    rect: Rect,
    start: Instant,
}

fn key_of(k: &DialogKey) -> Option<Key> {
    let mods = k.mods & !(keys::MOD_CAPS | keys::MOD_MOD2);
    let bare = mods == 0;
    let special = k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_Return | keys::KEY_KP_Enter => Some(Key::Enter),
        keys::KEY_Escape => Some(Key::Escape),
        keys::KEY_Up | keys::KEY_KP_Up => Some(Key::Up),
        keys::KEY_Down | keys::KEY_KP_Down => Some(Key::Down),
        keys::KEY_Left | keys::KEY_KP_Left if bare => Some(Key::Left),
        keys::KEY_Right | keys::KEY_KP_Right if bare => Some(Key::Right),
        keys::KEY_Home | keys::KEY_KP_Home if bare => Some(Key::Home),
        keys::KEY_End | keys::KEY_KP_End if bare => Some(Key::End),
        keys::KEY_Tab if bare => Some(Key::Tab),
        keys::KEY_Menu => Some(Key::Menu),
        keys::KEY_F10 if mods == keys::MOD_SHIFT => Some(Key::Menu),
        _ => None,
    });
    let tab = k
        .syms
        .iter()
        .any(|&s| s == keys::KEY_Tab || s == keys::KEY_ISO_Left_Tab);
    special.or_else(|| k.edit.clone().filter(|_| !tab).map(Key::Char))
}

fn one_per_name(apps: &[AppEntry]) -> Vec<bool> {
    let mut first: HashMap<&str, usize> = HashMap::new();
    for (i, a) in apps.iter().enumerate().filter(|(_, a)| a.shown) {
        let k = first.entry(a.name.as_str()).or_insert(i);
        if a.id < apps[*k].id {
            *k = i;
        }
    }
    (0..apps.len())
        .map(|i| first.get(apps[i].name.as_str()) == Some(&i))
        .collect()
}

fn panel_matches(query: &str, apps: &[AppEntry], kept: &[bool], max: usize) -> Vec<usize> {
    reprisekit::fuzzy::rank(query, apps.iter().map(|a| a.name.as_str()))
        .into_iter()
        .filter(|&i| kept[i] && word_starts(query, &apps[i].name))
        .take(max)
        .collect()
}

fn tree_rows(
    apps: &[AppEntry],
    groups: &[Group],
    kept: &[bool],
) -> (Vec<TreeRow>, Vec<Option<usize>>) {
    let mut rows = Vec::new();
    let mut map = Vec::new();
    for g in groups {
        let listed: Vec<usize> = g.apps.iter().copied().filter(|&i| kept[i]).collect();
        if listed.is_empty() {
            continue;
        }
        rows.push(TreeRow::Folder {
            name: g.name.clone(),
            open: false,
            icon: None,
        });
        map.push(None);
        for i in listed {
            rows.push(TreeRow::App {
                name: apps[i].name.clone(),
                icon: None,
                depth: 1,
            });
            map.push(Some(i));
        }
    }
    (rows, map)
}

fn with_mime<R>(
    desktop: Option<&crate::desktopview::DesktopView>,
    f: impl FnOnce(&reprisefs::apps::MimeDb) -> R,
) -> R {
    match desktop {
        Some(d) => f(&d.mime),
        None => f(&reprisefs::apps::MimeDb::empty()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResultKind {
    Calc,
    Programs,
    Settings,
    Files,
}

fn result_groups(
    calc: Option<&str>,
    apps: &[String],
    settings: &[String],
    files: &[PathBuf],
) -> (Vec<ResultGroup>, Vec<ResultKind>) {
    let mut groups = Vec::new();
    let mut kinds = Vec::new();
    let mut group = |kind, title: String, rows: Vec<String>| {
        if !rows.is_empty() {
            groups.push(ResultGroup {
                title,
                rows: rows
                    .into_iter()
                    .map(|name| ResultRow { name, icon: None })
                    .collect(),
            });
            kinds.push(kind);
        }
    };
    group(
        ResultKind::Calc,
        "Calculator".into(),
        calc.map(str::to_string).into_iter().collect(),
    );
    let apps = &apps[..apps.len().min(SEARCH_ROWS)];
    group(
        ResultKind::Programs,
        format!("Programs ({})", apps.len()),
        apps.to_vec(),
    );
    group(
        ResultKind::Settings,
        format!("Settings ({})", settings.len()),
        settings.to_vec(),
    );
    let files = &files[..files.len().min(SEARCH_ROWS)];
    group(
        ResultKind::Files,
        format!("Files ({})", files.len()),
        files.iter().map(|p| file_name(p)).collect(),
    );
    (groups, kinds)
}

fn frequent_rows(
    launched: Vec<usize>,
    starters: &[usize],
    pinned: &[usize],
) -> (Vec<usize>, usize) {
    let n = launched.len();
    let mut rows = launched;
    for &i in starters {
        if rows.len() >= crate::launches::FREQUENT {
            break;
        }
        if !pinned.contains(&i) && !rows.contains(&i) {
            rows.push(i);
        }
    }
    (rows, n)
}

fn spot_app(
    s: Spot,
    programs: &[usize],
    (pinned, launched): (usize, usize),
    tree_apps: &[Option<usize>],
    (result_apps, kinds): (&[usize], &[ResultKind]),
) -> Option<(usize, bool)> {
    match s {
        Spot::Program(i) => Some((*programs.get(i)?, (pinned..pinned + launched).contains(&i))),
        Spot::Tree(i) => Some((tree_apps.get(i).copied().flatten()?, false)),
        Spot::Result(g, r) if kinds.get(g) == Some(&ResultKind::Programs) => {
            Some((*result_apps.get(r)?, false))
        }
        _ => None,
    }
}

fn jump_rows(
    icons: &mut crate::icontheme::IconCache,
    files: &[PathBuf],
    px: i32,
) -> Vec<ResultRow> {
    files
        .iter()
        .map(|p| ResultRow {
            name: file_name(p),
            icon: icons.get(reprisefs::icons::icon_for_file(p), px),
        })
        .collect()
}

fn jump_file(jumps: &[Vec<PathBuf>], mode: Mode, k: usize) -> Option<&Path> {
    let Mode::Jump(i) = mode else {
        return None;
    };
    jumps.get(i)?.get(k).map(PathBuf::as_path)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The tree's scroll, lit and hot rows index the old rows and are dropped.
fn replace_tree(model: &mut StartPanel, rows: Vec<TreeRow>) {
    model.tree = rows;
    model.tree_top = 0;
    if model.mode() == Mode::Programs {
        model.set_mode(Mode::Programs);
    }
}

fn panel_origin(
    dock: StartMenuDock,
    p: &BarPlacement,
    start: (i32, i32),
    area: Rect,
    (w, h): (i32, i32),
) -> (i32, i32) {
    let x = start.0.min(area.right() - w).max(area.x);
    let (dock, bar) = p.start_dock(dock);
    match p.edge {
        Edge::Bottom => {
            let y = dock
                .menu_at(p.edge, (bar.y, bar.bottom()), h)
                .max(area.y + layout::OVERHANG)
                .min(dock.menu_at(Edge::Bottom, (area.bottom(), area.bottom()), h))
                .max(area.y);
            (x, y)
        }
        Edge::Top => {
            let y = dock
                .menu_at(p.edge, (bar.y, bar.bottom()), h)
                .max(dock.menu_at(Edge::Top, (area.y, area.y), h))
                .max(bar.y + layout::OVERHANG)
                .min(area.bottom() - h)
                .max(bar.y);
            (x, y)
        }
        Edge::Left | Edge::Right => {
            let y = start
                .1
                .max(area.y + layout::OVERHANG)
                .min(area.bottom() - h)
                .max(area.y);
            let x = dock.menu_at(p.edge, (bar.x, bar.right()), w);
            (crate::barplace::beside_on_output(x, w, bar, area), y)
        }
    }
}

fn shutdown_options_at(edge: Edge, arrow: Rect, size: (i32, i32), usable: Rect) -> (i32, i32) {
    let h = size.1;
    let drops = edge != Edge::Bottom;
    let y = if drops && arrow.y + h <= usable.bottom() {
        arrow.y
    } else {
        arrow.bottom() - h
    };
    place_popup((arrow.right(), y), size, usable)
}

fn picture_paths(home: Option<&Path>, user: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = home.map(|h| h.join(".face")).into_iter().collect();
    if !user.is_empty() && !user.contains('/') {
        v.push(Path::new("/var/lib/AccountsService/icons").join(user));
    }
    v
}

/// The picture files carry no extension, so the icon cache does not take them.
fn load_picture(path: &Path, px: i32) -> Option<Canvas> {
    const MAX_BYTES: u64 = 4 << 20;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let c = reprisekit::icontheme::png_to_canvas(&bytes, 4096)
        .map_err(|e| log::debug!("user picture {}: {e}", path.display()))
        .ok()?;
    Some(c.resample_box(px as u32, px as u32))
}

fn panel_pane(l: &Layout) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: l.panel,
            corner: reprisekit::glass::startpanel::PANEL_CORNER,
            square_bottom: true,
            square_top: false,
        }],
        holes: vec![l.pane],
    }
}

fn outlived(on: &Output, outputs: &[Output]) -> bool {
    !outputs.contains(on)
}

fn takes_pointer(l: &Layout) -> impl Fn(Point<f64, Logical>) -> bool + 'static {
    let (panel, picture, origin) = (l.panel, l.picture, (l.canvas.x, l.canvas.y));
    move |p| {
        let (x, y) = (p.x.floor() as i32 + origin.0, p.y.floor() as i32 + origin.1);
        panel.contains(x, y) || picture.is_some_and(|r| r.contains(x, y))
    }
}

impl State {
    pub(crate) fn toggle_start_panel(&mut self, bar: TreeId) {
        if self.start_panel.is_some() {
            self.close_start_panel();
        } else {
            self.open_start_panel(bar);
        }
    }

    pub(crate) fn toggle_start_panel_from_keys(&mut self) {
        if self.start_panel.is_some() {
            self.close_start_panel();
            return;
        }
        if !self.menus.is_empty() {
            self.close_menu();
            return;
        }
        let p = self.pointer_location();
        let Some(output) = self.output_at(p.x, p.y) else {
            return;
        };
        if let Some(bar) = self.bars.iter().find(|b| b.output == output) {
            let bar = bar.tree;
            self.open_start_panel(bar);
        }
    }

    fn open_start_panel(&mut self, bar: TreeId) {
        self.close_menu();
        self.close_launcher();
        let Some(output) = self.bar(bar).map(|b| b.output.clone()) else {
            return;
        };
        if self.layout_box(Some(&output)).width <= 0 {
            return;
        }
        let px = PICTURE * self.draw_scale_at(Some(&output)).max(1);
        let user = crate::lock::current_user();
        let mut model = StartPanel::new();
        model.text = self.config.theme.text;
        self.refresh_recent();
        let (rows, places) = self.start_places(px, &user);
        model.places = rows;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        model.user_picture = picture_paths(home.as_deref(), &user)
            .iter()
            .find_map(|p| load_picture(p, px))
            .or_else(|| self.icons.get("avatar-default", px));
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.menu);
        scene.set_data(tree, Some(SceneOwner::StartPanel));
        crate::glass::leave_out(scene, tree);
        let glass_tree = scene.tree_create(tree);
        let buffer = scene.buffer_create(tree);
        let fade = self.config.theme.metrics.menu_fade_ms > 0;
        if fade {
            scene.set_opacity(tree, 0.0);
        }
        self.start_panel = Some(StartPanelView {
            model,
            bar,
            output,
            programs: Vec::new(),
            launched: 0,
            jumps: Vec::new(),
            new_pins: Vec::new(),
            tree_apps: Vec::new(),
            result_apps: Vec::new(),
            answer: None,
            result_settings: Vec::new(),
            result_kinds: Vec::new(),
            search: FileSearch::new(),
            wheel: 0,
            places,
            pressed: None,
            hover: None,
            opened: Instant::now(),
            fade: None,
            at: (0, 0),
            tree,
            glass_tree,
            backdrop: Backdrop::new(&self.glass),
            buffer,
            drawn: None,
            rasters: Rasters::default(),
            crossfades: Vec::new(),
            crossfade_timer: None,
        });
        self.list_start_programs();
        self.restart_caret();
        if let Some(b) = self.bar_mut(bar) {
            b.start_pressed = true;
        }
        if fade {
            self.arm_start_panel_fade();
        }
        self.redraw_start_panel();
        self.redraw_taskbars();
    }

    /// The panel's places for the document folders the last check left, with their `px`
    /// pictures, and `startmenu::places()` without its rules.
    fn start_places(&mut self, px: i32, user: &str) -> (Vec<PlaceRow>, Vec<MenuEntry>) {
        let dirs = self.recent.document_dirs();
        let mut rows = Vec::new();
        let mut places = Vec::new();
        let mut rule = false;
        for e in crate::startmenu::places(self.config.recent_files, &dirs) {
            let MenuEntry::Item { label, icon, .. } = &e else {
                rule = true;
                continue;
            };
            rows.push(PlaceRow {
                label: crate::startmenu::expand_label(label, user),
                picture: icon.as_deref().and_then(|n| self.icons.get(n, px)),
                rule_before: rule,
                submenu: e.opens_submenu(),
            });
            places.push(e);
            rule = false;
        }
        (rows, places)
    }

    /// Relists the open panel's places from the last check of the document folders, in place.
    /// A Recent Items popup moves with its row, and closes if the row is gone.
    pub(crate) fn relist_start_places(&mut self) {
        let Some(output) = self.start_panel.as_ref().map(|v| v.output.clone()) else {
            return;
        };
        let px = PICTURE * self.draw_scale_at(Some(&output)).max(1);
        let (rows, places) = self.start_places(px, &crate::lock::current_user());
        let Some(v) = self.start_panel.as_mut() else {
            return;
        };
        if v.places == places {
            return;
        }
        let old = std::mem::replace(&mut v.places, places);
        let new = &v.places;
        v.model.relist_places(rows, |i| {
            reprisekit::selection::follow(&old, new, i, |_| true)
        });
        let popup = matches!(
            self.menus.first().map(|m| &m.kind),
            Some(crate::menuview::MenuKind::Documents { dirs: false, .. })
        );
        let anchor = old
            .iter()
            .find(|e| e.opens_submenu())
            .and_then(|e| new.iter().position(|n| n == e));
        self.redraw_start_panel();
        if popup {
            match anchor.and_then(|i| self.start_spot_rect(Spot::Place(i))) {
                Some(row) => self.move_start_popup((row.right(), row.y)),
                None => self.close_menu(),
            }
        }
        let p = self.pointer_location();
        let spot = self.start_panel_hit(p.x, p.y);
        self.start_panel_pointer(spot);
    }

    /// Moves the popup open off the panel to hang at `at`, as `place_popup` puts it.
    fn move_start_popup(&mut self, at: (i32, i32)) {
        let Some(m) = self.menus.first() else {
            return;
        };
        let usable = self.usable_area(Some(&m.output));
        let (x, y) = place_popup(at, (m.layout.width, m.layout.height), usable);
        if (x, y) == (m.x, m.y) {
            return;
        }
        let tree = m.tree;
        if let Some(m) = self.menus.first_mut() {
            (m.x, m.y) = (x, y);
        }
        self.scene.set_position(tree, x, y);
        self.redraw_all();
    }

    fn list_start_programs(&mut self) {
        let Some(scale) = self
            .start_panel
            .as_ref()
            .map(|v| self.draw_scale_at(Some(&v.output)).max(1))
        else {
            return;
        };
        let State {
            start_panel,
            launches,
            app_index,
            icons,
            config,
            recent,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let pinned = launches.pinned(app_index);
        let (frequent, launched) = frequent_rows(
            launches.frequent(app_index, crate::launches::FREQUENT),
            &crate::launches::starters(app_index),
            &pinned,
        );
        let programs: Vec<usize> = pinned.iter().chain(&frequent).copied().collect();
        v.jumps = programs
            .iter()
            .map(|&i| {
                if config.recent_files {
                    recent.for_app(crate::taskbar::JUMP_RECENT, &app_index[i])
                } else {
                    Vec::new()
                }
            })
            .collect();
        let px = PROGRAM_ICON * scale;
        let mut rows = programs
            .iter()
            .zip(&v.jumps)
            .enumerate()
            .map(|(k, (&i, jumps))| {
                let a = &app_index[i];
                ProgramRow {
                    name: a.name.clone(),
                    icon: a.icon.as_deref().and_then(|n| icons.get(n, px)),
                    jump: !jumps.is_empty(),
                    new_pin: k < pinned.len() && v.new_pins.contains(&a.id),
                }
            });
        v.model.pinned = rows.by_ref().take(pinned.len()).collect();
        v.model.frequent = rows.collect();
        v.programs = programs;
        v.launched = launched;
        // A jump list or a lit row indexes the old rows.
        if matches!(v.model.mode(), Mode::Home | Mode::Jump(_)) {
            v.model.set_mode(Mode::Home);
        }
    }

    fn list_start_jump(&mut self, i: usize) {
        let Some(scale) = self
            .start_panel
            .as_ref()
            .map(|v| self.draw_scale_at(Some(&v.output)).max(1))
        else {
            return;
        };
        let px = RESULT_ICON * scale;
        let State {
            start_panel, icons, ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        v.model.jump = v
            .jumps
            .get(i)
            .map(|files| jump_rows(icons, files, px))
            .unwrap_or_default();
        v.model.jump_title = JUMP_TITLE.into();
    }

    /// Close the panel if its output left the scene: it would stay at
    /// stale coordinates taking every key and the pointer.
    pub fn close_start_panel_off_scene(&mut self) {
        if self
            .start_panel
            .as_ref()
            .is_some_and(|v| outlived(&v.output, self.scene.outputs()))
        {
            self.close_start_panel();
        }
    }

    fn list_start_tree(&mut self) {
        let Some(scale) = self
            .start_panel
            .as_ref()
            .map(|v| self.draw_scale_at(Some(&v.output)).max(1))
        else {
            return;
        };
        let State {
            start_panel,
            app_index,
            icons,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let kept = one_per_name(app_index);
        let (mut rows, map) = tree_rows(app_index, &reprisefs::apps::group(app_index), &kept);
        let px = TREE_ICON * scale;
        let folder = icons.get("folder", px);
        for (row, app) in rows.iter_mut().zip(&map) {
            match (row, app) {
                (TreeRow::App { icon, .. }, Some(i)) => {
                    *icon = app_index[*i].icon.as_deref().and_then(|n| icons.get(n, px));
                }
                (TreeRow::Folder { icon, .. }, _) => *icon = folder.clone(),
                _ => {}
            }
        }
        replace_tree(&mut v.model, rows);
        v.tree_apps = map;
    }

    /// The app index was rescanned: list the programs again, so a row
    /// never points into an index that is gone. An open tree is rebuilt
    /// closed, and anything else drops its stale tree.
    pub fn start_panel_index_changed(&mut self) {
        if self.start_panel.is_some() {
            // A popup's rows index the old app index.
            self.close_menu();
            self.list_start_programs();
            let mode = self.start_panel.as_ref().map(|v| v.model.mode());
            // The search rows index the old app index: rank the text again.
            if mode == Some(Mode::Search) {
                self.start_panel_query_changed();
            }
            if mode == Some(Mode::Programs) {
                self.list_start_tree();
            } else if let Some(v) = self.start_panel.as_mut() {
                v.model.tree.clear();
                v.tree_apps.clear();
            }
            self.redraw_start_panel();
        }
    }

    pub(crate) fn close_start_panel(&mut self) {
        let Some(mut v) = self.start_panel.take() else {
            return;
        };
        for t in [v.hover.take(), v.fade.take(), v.crossfade_timer.take()]
            .into_iter()
            .flatten()
        {
            self.handle.remove(t);
        }
        crate::tooltip::hover_start_panel(self, None);
        self.close_menu();
        v.search.stop(&self.handle);
        if self.editor().is_some() {
            self.restart_caret();
        } else {
            self.stop_caret();
        }
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        self.scene.destroy(v.tree);
        if let Some(b) = self.bar_mut(v.bar) {
            b.start_pressed = false;
        }
        self.redraw_taskbars();
        self.redraw_all();
        crate::access::touch(self);
    }

    pub fn start_panel_key(&mut self, k: &DialogKey) {
        let Some(key) = key_of(k) else {
            return;
        };
        // Right on All Programs lights the tree's first row as it enters
        // Programs, so the rows must exist before the key.
        let enters = key == Key::Right
            && self.start_panel.as_ref().is_some_and(|v| {
                v.model.mode() != Mode::Programs && v.model.lit() == Some(Spot::AllPrograms)
            });
        if enters {
            self.list_start_tree();
        }
        if let Key::Char(e) = &key {
            self.clip_edit_key(e);
        }
        self.restart_caret();
        self.start_panel_event_with(enters, |m| m.key(key));
    }

    /// Text dropped on the search field becomes the query (`dialogdrop`); `true` when it went
    /// in.
    pub fn start_panel_dropped(&mut self, text: &str) -> bool {
        let mut took = false;
        self.start_panel_event(|m| {
            let event = m.drop_query(text);
            took = event == Event::QueryChanged;
            event
        });
        took
    }

    pub fn start_panel_paste(&mut self, text: &str) {
        self.start_panel_event(|m| m.paste(text));
    }

    fn start_panel_query_changed(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let query = v.model.query().to_string();
        let kept = self.file_query_changed(SearchOwner::StartPanel, &query, SEARCH_ROWS);
        let State {
            start_panel,
            app_index,
            ..
        } = self;
        if let Some(v) = start_panel.as_mut() {
            let listed = one_per_name(app_index);
            v.result_apps = panel_matches(&query, app_index, &listed, SEARCH_ROWS);
            v.result_settings = rank_settings(&query, app_index, &v.result_apps, SEARCH_SETTINGS);
            v.answer = calc_row(&query).map(|(_, answer)| answer);
            v.search.files = kept;
        }
        self.start_panel_show_results();
    }

    pub(crate) fn start_panel_files(&mut self, paths: Vec<PathBuf>) {
        if let Some(v) = self.start_panel.as_mut() {
            v.search.files = paths;
        }
        self.start_panel_show_results();
        self.redraw_start_panel();
    }

    fn start_panel_show_results(&mut self) {
        crate::tooltip::hover_start_panel(self, None);
        let Some(scale) = self
            .start_panel
            .as_ref()
            .map(|v| self.draw_scale_at(Some(&v.output)).max(1))
        else {
            return;
        };
        let px = RESULT_ICON * scale;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let State {
            start_panel,
            app_index,
            icons,
            desktop,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let names: Vec<String> = v
            .result_apps
            .iter()
            .map(|&i| app_index[i].name.clone())
            .collect();
        let settings: Vec<String> = v
            .result_settings
            .iter()
            .map(|s| s.name(app_index))
            .collect();
        let calc = v.answer.as_deref().map(|a| format!("= {a}"));
        let (mut groups, kinds) =
            result_groups(calc.as_deref(), &names, &settings, &v.search.files);
        v.result_kinds = kinds;
        let mut icon_rows: Vec<Option<Canvas>> = Vec::new();
        if calc.is_some() {
            icon_rows.push(icons.get(CALC_ICON, px));
        }
        icon_rows.extend(
            v.result_apps
                .iter()
                .map(|&i| app_index[i].icon.as_deref().and_then(|n| icons.get(n, px))),
        );
        icon_rows.extend(
            v.result_settings
                .iter()
                .map(|s| s.icon(app_index).and_then(|n| icons.get(&n, px))),
        );
        with_mime(desktop.as_ref(), |mime| {
            icon_rows.extend(v.search.files.iter().map(|p| {
                let is_dir = v.search.is_dir(p);
                file_row(p, is_dir, home.as_deref(), mime, icons, px).icon
            }));
        });
        let rows = groups.iter_mut().flat_map(|g| g.rows.iter_mut());
        for (row, icon) in rows.zip(icon_rows) {
            row.icon = icon;
        }
        v.model.set_results(groups);
    }

    fn start_panel_run_result(&mut self, g: usize, r: usize) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        enum Run {
            Copy(String),
            App(Box<AppEntry>),
            Setting(Setting),
            File(std::path::PathBuf, crate::desktop::FileFacts),
        }
        let run = match v.result_kinds.get(g) {
            Some(ResultKind::Calc) => v.answer.clone().map(Run::Copy),
            Some(ResultKind::Programs) => v
                .result_apps
                .get(r)
                .and_then(|&a| self.app_index.get(a))
                .map(|e| Run::App(Box::new(e.clone()))),
            Some(ResultKind::Settings) => v.result_settings.get(r).copied().map(Run::Setting),
            Some(ResultKind::Files) => v
                .search
                .files
                .get(r)
                .map(|p| Run::File(p.clone(), v.search.facts_of(p))),
            None => None,
        };
        self.close_start_panel();
        match run {
            Some(Run::Copy(answer)) => self.clip_copy(&answer),
            Some(Run::App(entry)) => self.launch_app(&entry),
            Some(Run::Setting(s)) => self.open_setting(s),
            Some(Run::File(path, facts)) => self.open_found(&path, &facts),
            None => {}
        }
    }

    pub fn start_panel_wheel(&mut self, discrete: i32, continuous: f64) -> bool {
        let Some(v) = self.start_panel.as_mut() else {
            return false;
        };
        if v.model.mode() != Mode::Programs {
            return false;
        }
        let (steps, rest) = crate::pointers::wheel_steps(discrete, continuous, v.wheel);
        v.wheel = rest;
        if steps != 0 {
            // `wheel_steps` counts up as positive.
            let rows = -i64::from(steps) * WHEEL_ROWS;
            self.start_panel_event(|m| m.scroll_tree(rows));
        }
        true
    }

    pub fn start_panel_pointer(&mut self, spot: Option<Spot>) {
        if !self.menus.is_empty() {
            return;
        }
        self.start_panel_event(|m| m.pointer(spot));
    }

    pub(crate) fn start_panel_bar(&self) -> Option<TreeId> {
        self.start_panel.as_ref().map(|v| v.bar)
    }

    pub fn start_panel_press(&mut self, spot: Option<Spot>, button: u32) {
        let b = match button {
            BTN_LEFT => Button::Left,
            BTN_RIGHT => Button::Right,
            _ => return,
        };
        let Some(spot) = spot else {
            return;
        };
        if let (Button::Left, Some(v)) = (b, self.start_panel.as_mut()) {
            v.pressed = Some(spot);
        }
        let program = (b == Button::Left)
            .then(|| self.start_spot_app(spot))
            .flatten()
            .and_then(|(a, _)| self.app_index.get(a).cloned())
            .and_then(|e| Some((reprisefs::apps::find_file(&e.id)?, e)));
        match program {
            Some((file, entry)) => self.start_panel_row_press(spot, entry, file),
            None => self.start_panel_event(|m| m.press(spot, b)),
        }
        self.redraw_start_panel();
    }

    pub(crate) fn start_panel_click(&mut self, spot: Spot) {
        self.start_panel_event(|m| m.press(spot, Button::Left));
    }

    pub fn start_panel_release(&mut self) {
        if self
            .start_panel
            .as_mut()
            .is_some_and(|v| v.pressed.take().is_some())
        {
            self.redraw_start_panel();
        }
    }

    pub fn start_panel_hit(&self, x: f64, y: f64) -> Option<Spot> {
        let v = self.start_panel.as_ref()?;
        let (x, y) = (x.floor() as i32 - v.at.0, y.floor() as i32 - v.at.1);
        layout::hit(&layout::layout(&v.model), x, y)
    }

    fn start_panel_event(&mut self, f: impl FnOnce(&mut StartPanel) -> Event) {
        self.start_panel_event_with(false, f);
    }

    fn start_panel_event_with(
        &mut self,
        tree_built: bool,
        f: impl FnOnce(&mut StartPanel) -> Event,
    ) {
        let Some(v) = self.start_panel.as_mut() else {
            return;
        };
        let before = v.model.mode();
        let event = f(&mut v.model);
        let after = v.model.mode();
        if before != Mode::Programs && after == Mode::Programs && !tree_built {
            self.list_start_tree();
        }
        if let Mode::Jump(i) = after
            && before != after
        {
            self.list_start_jump(i);
        }
        match event {
            Event::None => {}
            Event::Redraw => self.redraw_start_panel(),
            Event::QueryChanged => {
                self.start_panel_query_changed();
                self.redraw_start_panel();
            }
            Event::ArmHover(s) => {
                self.arm_start_panel_hover(s);
                self.redraw_start_panel();
            }
            Event::Activate(s) => self.start_panel_activate(s),
            Event::Context(s) => self.open_start_context(s),
            Event::Close => self.close_start_panel(),
        }
        let hot = self
            .start_panel
            .as_ref()
            .filter(|v| self.menus.is_empty() && v.model.mode() == after)
            .and_then(|v| v.model.hot());
        crate::tooltip::hover_start_panel(self, hot);
    }

    pub(crate) fn start_panel_tip(&self, s: Spot) -> Option<String> {
        let v = self.start_panel.as_ref()?;
        match s {
            Spot::Place(i) => return v.places.get(i)?.tip().map(String::from),
            Spot::Jump(k) => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                let path = jump_file(&v.jumps, v.model.mode(), k)?;
                return Some(crate::search::tilde(path, home.as_deref()));
            }
            _ => {}
        }
        let (app, _) = self.start_spot_app(s)?;
        let file = reprisefs::apps::find_file(&self.app_index.get(app)?.id)?;
        let comment = reprisefs::launcher::Launcher::read(&file)?.comment;
        (!comment.trim().is_empty()).then_some(comment)
    }

    pub(crate) fn start_panel_output(&self) -> Option<Output> {
        self.start_panel.as_ref().map(|v| v.output.clone())
    }

    fn start_panel_activate(&mut self, s: Spot) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        match s {
            Spot::Program(i) => {
                let entry = v
                    .programs
                    .get(i)
                    .and_then(|&a| self.app_index.get(a))
                    .cloned();
                self.close_start_panel();
                if let Some(e) = entry {
                    self.launch_app(&e);
                }
            }
            Spot::Place(i) => match v.places.get(i).cloned() {
                Some(e) if !e.opens_submenu() => {
                    self.close_start_panel();
                    self.activate_entry(&e);
                }
                Some(_) => self.open_recent_items(i),
                None => {}
            },
            Spot::ShutdownArrow => self.open_shutdown_options(),
            Spot::Jump(k) => {
                let Mode::Jump(i) = v.model.mode() else {
                    return;
                };
                let entry = v
                    .programs
                    .get(i)
                    .and_then(|&a| self.app_index.get(a))
                    .cloned();
                let path = jump_file(&v.jumps, v.model.mode(), k).map(Path::to_path_buf);
                if let (Some(e), Some(p)) = (entry, path) {
                    self.close_start_panel();
                    self.open_recent_with(Some(&e), &p);
                }
            }
            Spot::Shutdown => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::TurnOff);
            }
            Spot::Tree(i) => {
                let entry = v
                    .tree_apps
                    .get(i)
                    .copied()
                    .flatten()
                    .and_then(|a| self.app_index.get(a))
                    .cloned();
                if let Some(e) = entry {
                    self.close_start_panel();
                    self.launch_app(&e);
                }
            }
            Spot::Result(g, r) => self.start_panel_run_result(g, r),
            Spot::Picture => {
                self.close_start_panel();
                self.run_action(&Action::Spawn(ACCOUNTS.into()), 0);
            }
            _ => {}
        }
    }

    fn start_spot_app(&self, s: Spot) -> Option<(usize, bool)> {
        let v = self.start_panel.as_ref()?;
        spot_app(
            s,
            &v.programs,
            (v.model.pinned.len(), v.launched),
            &v.tree_apps,
            (&v.result_apps, &v.result_kinds),
        )
    }

    fn start_spot_rect(&self, s: Spot) -> Option<Rect> {
        let v = self.start_panel.as_ref()?;
        let r = layout::layout(&v.model).rect(s)?;
        Some(Rect::new(r.x + v.at.0, r.y + v.at.1, r.width, r.height))
    }

    fn push_start_popup(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: crate::menuview::MenuKind,
        place: impl FnOnce((i32, i32), (i32, i32), Rect) -> (i32, i32),
    ) {
        if let Some(t) = self.start_panel.as_mut().and_then(|v| v.hover.take()) {
            self.handle.remove(t);
        }
        crate::tooltip::hover_start_panel(self, None);
        self.close_menu();
        self.push_context_menu_placed(at, items, kind, crate::menuview::MenuFrom::Bar, place);
    }

    fn open_start_context(&mut self, s: Spot) {
        let Some((app, launched)) = self.start_spot_app(s) else {
            return;
        };
        let Some(entry) = self.app_index.get(app) else {
            return;
        };
        let pinned_start = self.launches.is_pinned(&entry.id);
        let pinned_taskbar = self.quick_file_of(&entry.id).is_some();
        let (items, rows) =
            crate::startmenu::program_menu(app, pinned_start, pinned_taskbar, launched);
        let p = self.pointer_location();
        let at = if self.start_panel_hit(p.x, p.y) == Some(s) {
            (p.x as i32, p.y as i32)
        } else {
            match self.start_spot_rect(s) {
                Some(r) => (r.x, r.bottom()),
                None => return,
            }
        };
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        self.push_start_popup(at, items, kind, place_popup);
    }

    fn open_shutdown_options(&mut self) {
        let Some(arrow) = self.start_spot_rect(Spot::ShutdownArrow) else {
            return;
        };
        let Some(output) = self.start_panel_output() else {
            return;
        };
        let edge = self.bar_placement(&output).edge;
        let (items, rows) =
            crate::startmenu::shutdown_menu(!self.config.power.hibernate.is_empty());
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        let at = (arrow.right(), arrow.y);
        self.push_start_popup(at, items, kind, |_, size, usable| {
            shutdown_options_at(edge, arrow, size, usable)
        });
    }

    fn open_recent_items(&mut self, i: usize) {
        if !self.config.recent_files {
            return;
        }
        let Some(row) = self.start_spot_rect(Spot::Place(i)) else {
            return;
        };
        let Some(output) = self.start_panel.as_ref().map(|v| v.output.clone()) else {
            return;
        };
        let px = crate::startmenu::MenuStyle::context(
            &self.config.theme,
            crate::startmenu::ContextBitmaps::Icons,
        )
        .icon_px()
            * self.draw_scale_at(Some(&output)).max(1);
        self.refresh_recent();
        let (items, kind) = self.documents_rows(px, false);
        self.push_start_popup((row.right(), row.y), items, kind, place_popup);
    }

    pub(crate) fn quick_file_of(&self, id: &str) -> Option<String> {
        self.quicklaunch
            .iter()
            .find(|q| q.app.as_ref().is_some_and(|a| a.id == id))
            .map(|q| q.file.clone())
    }

    pub(crate) fn start_row(&mut self, row: StartRow) {
        let entry = |s: &State, a: usize| s.app_index.get(a).cloned();
        match row {
            StartRow::Open(a) => {
                let e = entry(self, a);
                self.close_start_panel();
                if let Some(e) = e {
                    self.launch_app(&e);
                }
            }
            StartRow::PinTaskbar(a) => {
                if let Some(src) = entry(self, a).and_then(|e| reprisefs::apps::find_file(&e.id)) {
                    self.quick_add(vec![src], None);
                }
            }
            StartRow::UnpinTaskbar(a) => {
                if let Some(f) = entry(self, a).and_then(|e| self.quick_file_of(&e.id)) {
                    self.quick_remove(&f);
                }
            }
            StartRow::PinStart(a) | StartRow::UnpinStart(a) | StartRow::Forget(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                match row {
                    StartRow::PinStart(_) => {
                        self.launches.pin(&e.id);
                        if let Some(v) = self.start_panel.as_mut() {
                            v.new_pins.push(e.id.clone());
                        }
                    }
                    StartRow::UnpinStart(_) => {
                        self.launches.unpin(&e.id);
                        if let Some(v) = self.start_panel.as_mut() {
                            v.new_pins.retain(|p| *p != e.id);
                        }
                    }
                    _ => self.launches.forget(&e.id),
                }
                crate::state::save_start(&self.launches);
                self.list_start_programs();
                self.redraw_start_panel();
            }
            StartRow::Properties(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                let p = self.pointer_location();
                self.close_start_panel();
                let Some(file) = reprisefs::apps::find_file(&e.id) else {
                    return;
                };
                match reprisefs::props::glance(&file, &e.name, &self.app_index) {
                    Ok(info) => {
                        let icon = e.icon.as_deref().unwrap_or("application-x-executable");
                        self.open_properties(&info, icon, (p.x as i32, p.y as i32));
                    }
                    Err(err) => log::warn!("properties: {}: {err}", file.display()),
                }
            }
            StartRow::LogOff => {
                self.close_start_panel();
                self.run_action(&Action::Exit, 0);
            }
            StartRow::Lock => {
                self.close_start_panel();
                self.run_action(&Action::Lock, 0);
            }
            StartRow::Restart => {
                self.close_start_panel();
                let cmd = self.config.power.reboot.clone();
                crate::sessionend::begin(
                    self,
                    reprisekit::controls::endsession::Goal::Restart,
                    Some(cmd),
                );
            }
            StartRow::Sleep => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::Standby);
            }
            StartRow::Hibernate => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::Hibernate);
            }
        }
    }

    fn arm_start_panel_hover(&mut self, s: Spot) {
        let ms = self.config.theme.metrics.menu_hover_ms.max(1) as u64;
        if let Some(old) = self.start_panel.as_mut().and_then(|v| v.hover.take()) {
            self.handle.remove(old);
        }
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            move |_, _, state| {
                if let Some(v) = state.start_panel.as_mut() {
                    v.hover = None;
                }
                if state.menus.is_empty() {
                    state.start_panel_event(|m| m.hover_fired(s));
                }
                TimeoutAction::Drop
            },
        );
        match (token, self.start_panel.as_mut()) {
            (Ok(t), Some(v)) => v.hover = Some(t),
            (Ok(t), None) => self.handle.remove(t),
            (Err(e), _) => log::warn!("start panel hover timer: {}", e.error),
        }
    }

    fn arm_start_panel_crossfade(&mut self) {
        if self
            .start_panel
            .as_ref()
            .is_none_or(|v| v.crossfade_timer.is_some())
        {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                state.redraw_start_panel();
                match state.start_panel.as_mut() {
                    Some(v) if !v.crossfades.is_empty() => TimeoutAction::ToDuration(FADE_TICK),
                    Some(v) => {
                        v.crossfade_timer = None;
                        TimeoutAction::Drop
                    }
                    None => TimeoutAction::Drop,
                }
            });
        match (token, self.start_panel.as_mut()) {
            (Ok(t), Some(v)) => v.crossfade_timer = Some(t),
            (Ok(t), None) => self.handle.remove(t),
            (Err(e), _) => log::warn!("start panel crossfade timer: {}", e.error),
        }
    }

    fn arm_start_panel_fade(&mut self) {
        let token = self
            .handle
            .insert_source(Timer::from_duration(FADE_TICK), |_, _, state| {
                let fade = state.config.theme.metrics.menu_fade_ms.max(1) as f32;
                let Some(v) = state.start_panel.as_mut() else {
                    return TimeoutAction::Drop;
                };
                let t = (v.opened.elapsed().as_secs_f32() * 1000.0 / fade).min(1.0);
                if t >= 1.0 {
                    v.fade = None;
                }
                let tree = v.tree;
                state.scene.set_opacity(tree, t);
                state.redraw_all();
                if t >= 1.0 {
                    TimeoutAction::Drop
                } else {
                    TimeoutAction::ToDuration(FADE_TICK)
                }
            });
        match (token, self.start_panel.as_mut()) {
            (Ok(t), Some(v)) => v.fade = Some(t),
            (Ok(t), None) => self.handle.remove(t),
            (Err(e), _) => log::warn!("start panel fade timer: {}", e.error),
        }
    }

    /// Relists the open panel's jump lists from the last recent read and check, the one shown
    /// in place.
    pub(crate) fn relist_start_jumps(&mut self) {
        let Some(scale) = self
            .start_panel
            .as_ref()
            .map(|v| self.draw_scale_at(Some(&v.output)).max(1))
        else {
            return;
        };
        let State {
            start_panel,
            app_index,
            icons,
            config,
            recent,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let mut changed = false;
        for (k, &app) in v.programs.iter().enumerate() {
            let files = match app_index.get(app) {
                Some(e) if config.recent_files => recent.for_app(crate::taskbar::JUMP_RECENT, e),
                _ => Vec::new(),
            };
            let Some(old) = v.jumps.get_mut(k) else {
                break;
            };
            if *old == files {
                continue;
            }
            changed = true;
            if v.model.mode() == Mode::Jump(k) {
                let rows = jump_rows(icons, &files, RESULT_ICON * scale);
                v.model.relist_jump(rows, |i| {
                    reprisekit::selection::follow(old, &files, i, |_| true)
                });
            }
            let row = match k.checked_sub(v.model.pinned.len()) {
                None => v.model.pinned.get_mut(k),
                Some(f) => v.model.frequent.get_mut(f),
            };
            if let Some(row) = row {
                row.jump = !files.is_empty();
            }
            *old = files;
        }
        if changed {
            crate::tooltip::hover_start_panel(self, None);
            self.redraw_start_panel();
        }
    }

    pub fn redraw_start_panel(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let Some(start) = self.bar(v.bar).map(|b| b.layout.start) else {
            return;
        };
        let lay = layout::layout(&v.model);
        let p = self.bar_placement(&v.output);
        let at = panel_origin(
            self.config.theme.taskbar_shape().start_menu,
            &p,
            (p.shown.x + start.x, p.shown.y + start.y),
            self.usable_area(Some(&v.output)),
            (lay.panel.width, lay.panel.height),
        );
        let scale = self.draw_scale_at(Some(&v.output)).max(1);
        let pane = crate::glass::wanted(&self.config).then(|| panel_pane(&lay));
        let sl = lay.scaled(scale);
        let fades = self.config.theme.metrics.menu_fade_ms > 0;
        let key = RasterKey {
            model: v.model.clone(),
            pressed: v.pressed,
            caret: self.caret.on,
            preedit: self.ime_field_preedit(crate::caret::Editor::StartPanel),
            scale,
        };
        let (raster, spot) = match v.rasters.find(&key) {
            Some(drawn) => drawn,
            None => {
                let theme = self.config.theme.scaled(scale);
                let mut canvas = Canvas::new(sl.canvas.width as u32, sl.canvas.height as u32);
                canvas.set_preedit(key.preedit.clone());
                reprisekit::interfaces::draw_start_panel_backed(
                    &mut self.start_backing,
                    &mut canvas,
                    &theme,
                    &self.config.glass,
                    &self.start_panel_text,
                    &v.model,
                    &sl,
                    v.pressed,
                    self.caret.on,
                );
                let spot = canvas.take_spot();
                let raster = Rc::new(canvas);
                if let Some(v) = self.start_panel.as_mut() {
                    v.rasters.keep(key, raster.clone(), spot.clone());
                }
                (raster, spot)
            }
        };
        let origin = (at.0 + lay.canvas.x, at.1 + lay.canvas.y);
        self.ime_field_drawn(crate::caret::Editor::StartPanel, spot, origin, scale);
        let State {
            start_panel,
            scene,
            glass,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        v.at = at;
        scene.set_position(v.tree, at.0, at.1);
        scene.set_position(v.buffer, lay.canvas.x, lay.canvas.y);
        scene.set_input(v.buffer, Input::Test(Box::new(takes_pointer(&lay))));
        v.backdrop.set(scene, glass, v.glass_tree, pane);
        let (mode, picture) = (v.model.mode(), v.model.picture());
        let now = Instant::now();
        if fades && let Some(old) = &v.drawn {
            let inside =
                |r: Rect| Rect::new(r.x - sl.canvas.x, r.y - sl.canvas.y, r.width, r.height);
            let mut rects = Vec::new();
            if matches!(
                (old.mode, mode),
                (Mode::Home, Mode::Programs) | (Mode::Programs, Mode::Home)
            ) {
                rects.push(inside(sl.pane));
            }
            if old.picture != picture
                && let Some(r) = sl.picture
            {
                rects.push(inside(r));
            }
            v.crossfades.extend(rects.into_iter().map(|rect| Crossfade {
                from: old.canvas.clone(),
                rect,
                start: now,
            }));
        }
        let size = (raster.width, raster.height);
        v.crossfades
            .retain(|f| now - f.start < CROSSFADE && (f.from.width, f.from.height) == size);
        let canvas = if v.crossfades.is_empty() {
            raster
        } else {
            let mut canvas = (*raster).clone();
            for f in &v.crossfades {
                let t = (now - f.start).as_secs_f32() / CROSSFADE.as_secs_f32();
                canvas.mix(&f.from, f.rect, 1.0 - t);
            }
            Rc::new(canvas)
        };
        upload_scaled(scene, v.buffer, &canvas, scale);
        v.drawn = Some(Drawn {
            canvas,
            mode,
            picture,
        });
        if !v.crossfades.is_empty() {
            self.arm_start_panel_crossfade();
        }
        self.redraw_all();
        crate::access::touch(self);
    }

    /// The look or the text size changed under the open panel: draw it again from scratch.
    pub fn retheme_start_panel(&mut self) {
        self.start_backing = Default::default();
        let text = self.config.theme.text;
        if let Some(v) = self.start_panel.as_mut() {
            v.model.text = text;
            v.rasters = Rasters::default();
        }
        self.redraw_start_panel();
    }

    pub fn start_panel_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.start_panel.as_ref()?;
        let lay = layout::layout(&v.model);
        let rect = Rect::new(v.at.0, v.at.1, lay.panel.width, lay.panel.height);
        let mut tree = crate::access::root(Role::Window, "Start menu", rect);
        let focus = startpanel::access::access(&v.model, &lay, &mut tree, ROOT);
        tree.set_focus(focus);
        Some(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::startpanel::ProgramRow;
    use reprisekit::textedit::EditKey;

    fn press(sym: u32, mods: u32) -> DialogKey {
        DialogKey {
            syms: vec![sym],
            mods,
            edit: Some(EditKey::Insert("x".into())),
        }
    }

    fn app(name: &str, category: &str) -> AppEntry {
        AppEntry {
            name: name.into(),
            exec_line: name.into(),
            terminal: false,
            id: format!("{name}.desktop"),
            icon: None,
            startup_wm_class: None,
            categories: vec![category.into()],
            mime_types: Vec::new(),
            shown: true,
        }
    }

    #[test]
    fn a_caret_blink_reuses_the_canvas_drawn_for_that_phase_until_the_panel_changes() {
        let key = |caret: bool, model: &StartPanel| RasterKey {
            model: model.clone(),
            pressed: None,
            caret,
            preedit: None,
            scale: 1,
        };
        let model = StartPanel::new();
        let (on, off) = (Rc::new(Canvas::new(2, 2)), Rc::new(Canvas::new(2, 2)));
        let mut r = Rasters::default();
        r.keep(key(true, &model), on.clone(), None);
        r.keep(key(false, &model), off.clone(), None);
        let found = |r: &Rasters, k: &RasterKey| r.find(k).map(|(c, _)| c);
        assert!(found(&r, &key(true, &model)).is_some_and(|c| Rc::ptr_eq(&c, &on)));
        assert!(found(&r, &key(false, &model)).is_some_and(|c| Rc::ptr_eq(&c, &off)));
        let mut typed = model.clone();
        typed.paste("a");
        assert!(
            found(&r, &key(true, &typed)).is_none(),
            "a changed panel draws again"
        );
        let pressed = RasterKey {
            pressed: Some(Spot::Search),
            ..key(true, &model)
        };
        assert!(found(&r, &pressed).is_none(), "a press draws again");
        r.keep(key(true, &typed), on.clone(), None);
        assert!(
            found(&r, &key(true, &model)).is_none(),
            "two canvases are kept"
        );
        assert!(found(&r, &key(false, &model)).is_some());
    }

    #[test]
    fn the_frequent_list_tops_up_with_the_starters_not_shown() {
        assert_eq!(
            frequent_rows(vec![7], &[1, 7, 2, 3], &[2]),
            (vec![7, 1, 3], 1)
        );
        let full: Vec<usize> = (10..20).collect();
        assert_eq!(
            frequent_rows(full.clone(), &[1], &[]),
            (full, 10),
            "ten rows are full"
        );
    }

    #[test]
    fn a_starter_row_offers_no_remove_from_this_list() {
        let programs = [5, 7, 1, 3];
        let at = |s| {
            spot_app(
                s,
                &programs,
                (1, 1),
                &[None, Some(9)],
                (&[4], &[ResultKind::Programs]),
            )
        };
        assert_eq!(at(Spot::Program(0)), Some((5, false)), "pinned");
        assert_eq!(at(Spot::Program(1)), Some((7, true)), "launched");
        assert_eq!(at(Spot::Program(2)), Some((1, false)), "starter");
        assert_eq!(at(Spot::Program(3)), Some((3, false)), "starter");
    }

    #[test]
    fn only_programs_tree_apps_and_program_results_get_a_program_menu() {
        let kinds = [ResultKind::Calc, ResultKind::Programs, ResultKind::Files];
        let at = |s| spot_app(s, &[5], (0, 1), &[None, Some(9)], (&[4], &kinds));
        assert_eq!(at(Spot::Tree(1)), Some((9, false)));
        assert_eq!(at(Spot::Tree(0)), None, "a folder");
        assert_eq!(at(Spot::Result(1, 0)), Some((4, false)));
        assert_eq!(at(Spot::Result(0, 0)), None, "the answer");
        assert_eq!(at(Spot::Result(2, 0)), None, "a file result");
        assert_eq!(at(Spot::Place(0)), None);
        assert_eq!(at(Spot::Program(1)), None, "past the rows");
        let files = [ResultKind::Files];
        let no_programs = spot_app(Spot::Result(0, 0), &[], (0, 0), &[], (&[], &files));
        assert_eq!(no_programs, None, "group 0 is Files");
    }

    #[test]
    fn search_groups_the_answer_programs_settings_then_files_with_counts() {
        let (groups, kinds) = result_groups(
            Some("= 14"),
            &["Notepad".into()],
            &["Mouse".into(), "Pointers".into()],
            &[PathBuf::from("/home/u/notes.txt")],
        );
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Calculator", "Programs (1)", "Settings (2)", "Files (1)"]
        );
        assert_eq!(
            kinds,
            [
                ResultKind::Calc,
                ResultKind::Programs,
                ResultKind::Settings,
                ResultKind::Files
            ]
        );
        assert_eq!(groups[0].rows[0].name, "= 14");
        assert_eq!(groups[3].rows[0].name, "notes.txt");
        let (groups, kinds) = result_groups(None, &[], &["Mouse".into()], &[]);
        assert_eq!((groups.len(), kinds), (1, vec![ResultKind::Settings]));
        assert!(result_groups(None, &[], &[], &[]).0.is_empty());
    }

    #[test]
    fn a_group_shows_at_most_eight_rows_and_counts_them() {
        let names: Vec<String> = (0..12).map(|i| format!("App {i}")).collect();
        let (groups, _) = result_groups(None, &names, &[], &[]);
        assert_eq!(groups[0].title, "Programs (8)");
        assert_eq!(groups[0].rows.len(), 8);
    }

    #[test]
    fn the_tree_lists_folders_with_their_apps_under_them() {
        let apps = [
            app("Paint", "Graphics"),
            app("Calculator", "Utility"),
            app("Gimp", "Graphics"),
        ];
        let (rows, map) = tree_rows(&apps, &reprisefs::apps::group(&apps), &[true; 3]);
        let names: Vec<&str> = rows
            .iter()
            .map(|r| match r {
                TreeRow::Folder { name, .. } | TreeRow::App { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(
            names,
            ["Accessories", "Calculator", "Graphics", "Paint", "Gimp"]
        );
        assert_eq!(map, [None, Some(1), None, Some(0), Some(2)]);
        assert!(
            rows.iter()
                .all(|r| !matches!(r, TreeRow::Folder { open: true, .. }))
        );
    }

    #[test]
    fn two_entries_of_one_name_list_once_by_the_lower_id() {
        let mut apps = vec![
            app("Paint", "Graphics"),
            app("Notepad", "Utility"),
            app("Paint", "Utility"),
            app("Notes", "Utility"),
        ];
        apps[0].id = "z-paint.desktop".into();
        apps[2].id = "a-paint.desktop".into();
        apps[3].shown = false;
        let kept = one_per_name(&apps);
        assert_eq!(kept, [false, true, true, false]);
        let (rows, map) = tree_rows(&apps, &reprisefs::apps::group(&apps), &kept);
        assert_eq!(map, [None, Some(1), Some(2)], "Graphics is left empty");
        assert!(matches!(&rows[0], TreeRow::Folder { name, .. } if name == "Accessories"));
        assert_eq!(panel_matches("pa", &apps, &kept, 8), [2]);
    }

    #[test]
    fn the_program_search_lists_names_with_a_word_starting_with_the_text() {
        let apps = [
            app("Network Connections", "Utility"),
            app("Notepad", "Utility"),
            app("Print Editor", "Utility"),
            app("Sticky Notes", "Utility"),
        ];
        let kept = one_per_name(&apps);
        assert_eq!(panel_matches("no", &apps, &kept, 8), [1, 3]);
        assert_eq!(panel_matches("no", &apps, &kept, 1), [1], "capped");
    }

    #[test]
    fn a_rebuilt_tree_drops_the_scroll_and_the_lit_row() {
        let apps: Vec<AppEntry> = (0..30)
            .map(|k| app(&format!("App {k}"), "Utility"))
            .collect();
        let (rows, _) = tree_rows(&apps, &reprisefs::apps::group(&apps), &one_per_name(&apps));
        let mut p = StartPanel::new();
        replace_tree(&mut p, rows.clone());
        p.set_mode(Mode::Programs);
        p.toggle_folder(0);
        p.scroll_tree(5);
        p.key(Key::Up);
        p.key(Key::Up);
        assert!(matches!(p.lit(), Some(Spot::Tree(_))), "{:?}", p.lit());
        assert!(p.tree_top > 0);
        replace_tree(&mut p, rows);
        assert_eq!((p.tree_top, p.lit()), (0, None));
        assert_eq!(p.mode(), Mode::Programs);
    }

    fn bar_on(screen: Rect, edge: Edge) -> (BarPlacement, Rect) {
        let p = crate::barplace::place(screen, edge, 40);
        (p, crate::barplace::usable(screen, &p))
    }

    #[test]
    fn panel_origin_follows_the_dock() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let (bottom, area) = bar_on(screen, Edge::Bottom);
        let h = layout::layout(&StartPanel::new()).panel.height;
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &bottom, (0, 0), area, (layout::W, h)),
            (0, 1040 - h)
        );
        let mut p = StartPanel::new();
        let row = ProgramRow {
            name: "Notepad".into(),
            icon: None,
            jump: false,
            new_pin: false,
        };
        p.pinned = vec![row.clone()];
        p.frequent = vec![row];
        let tall = layout::layout(&p).panel.height;
        assert_eq!(tall, h + 4);
        assert_eq!(
            panel_origin(
                StartMenuDock::OnTop,
                &bottom,
                (0, 0),
                area,
                (layout::W, tall)
            )
            .1,
            1040 - h - 4
        );
        assert_eq!(
            panel_origin(
                StartMenuDock::Into(4),
                &bottom,
                (0, 0),
                area,
                (layout::W, h)
            )
            .1,
            1044 - h
        );
        let (top, area) = bar_on(screen, Edge::Top);
        for height in [h, tall] {
            let y = panel_origin(
                StartMenuDock::OnTop,
                &top,
                (0, 0),
                area,
                (layout::W, height),
            )
            .1;
            assert_eq!(y, 40, "a {height} px panel hangs from the bar");
            assert_eq!(y - layout::OVERHANG, 14, "the picture is on the screen");
        }
        assert_eq!(
            panel_origin(StartMenuDock::Into(4), &top, (0, 0), area, (layout::W, h)).1,
            36,
            "a dock into the bar reaches up into it"
        );
        let thin = crate::barplace::place(screen, Edge::Top, 20);
        let thin_area = crate::barplace::usable(screen, &thin);
        let y = panel_origin(
            StartMenuDock::OnTop,
            &thin,
            (0, 0),
            thin_area,
            (layout::W, h),
        )
        .1;
        assert_eq!((y, y - layout::OVERHANG), (26, 0));
    }

    #[test]
    fn a_hidden_bars_panel_stands_on_its_reveal_line() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let h = layout::layout(&StartPanel::new()).panel.height;
        let at = |edge, dock| {
            let p = crate::barplace::place_hiding(screen, edge, 40, Some(1.0));
            let area = crate::barplace::usable(screen, &p);
            panel_origin(dock, &p, (0, 0), area, (layout::W, h)).1
        };
        for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
            assert_eq!(
                at(Edge::Bottom, dock) + h,
                1078,
                "{dock:?}: over a bottom bar the panel's foot is the line's top"
            );
            let y = at(Edge::Top, dock);
            assert_eq!(
                (y, y - layout::OVERHANG),
                (26, 0),
                "{dock:?}: under a top bar the panel hangs off the line, its picture's top the output's"
            );
        }
        let shown = crate::barplace::place_hiding(screen, Edge::Bottom, 40, Some(0.0));
        let area = crate::barplace::usable(screen, &shown);
        assert_eq!(
            panel_origin(StartMenuDock::Into(4), &shown, (0, 0), area, (layout::W, h)).1,
            1044 - h
        );
    }

    #[test]
    fn the_panel_stays_inside_the_usable_area() {
        let screen = Rect::new(1920, 0, 1280, 1024);
        let h = layout::layout(&StartPanel::new()).panel.height;
        for edge in [Edge::Bottom, Edge::Top] {
            let (p, area) = bar_on(screen, edge);
            assert_eq!(
                panel_origin(
                    StartMenuDock::OnTop,
                    &p,
                    (3150, 0),
                    area,
                    (layout::JUMP_W, h)
                )
                .0,
                1920 + 1280 - layout::JUMP_W,
                "{edge:?}"
            );
            let (x, y) = panel_origin(StartMenuDock::OnTop, &p, (1900, 0), area, (layout::W, h));
            assert_eq!(x, 1920, "{edge:?}");
            assert!(
                y >= area.y && y + h <= area.bottom(),
                "{edge:?}: {y} in {area:?}"
            );
        }
    }

    #[test]
    fn a_jump_rows_tip_is_its_files_path_with_the_home_folder_as_a_tilde() {
        let jumps = vec![
            Vec::new(),
            vec![
                PathBuf::from("/home/u/Docs/notes.txt"),
                PathBuf::from("/srv/share/plan.odt"),
            ],
        ];
        let home = Some(Path::new("/home/u"));
        let tip = |mode, k| jump_file(&jumps, mode, k).map(|p| crate::search::tilde(p, home));
        assert_eq!(tip(Mode::Jump(1), 0).as_deref(), Some("~/Docs/notes.txt"));
        assert_eq!(
            tip(Mode::Jump(1), 1).as_deref(),
            Some("/srv/share/plan.odt")
        );
        assert_eq!(tip(Mode::Jump(1), 2), None, "past the rows");
        assert_eq!(tip(Mode::Jump(0), 0), None);
        assert_eq!(tip(Mode::Programs, 0), None, "no jump list shown");
    }

    #[test]
    fn a_short_area_keeps_the_panels_bottom_inside() {
        let h = layout::layout(&StartPanel::new()).panel.height;
        let at = |screen: Rect, edge| {
            let (p, area) = bar_on(screen, edge);
            panel_origin(StartMenuDock::OnTop, &p, (0, 0), area, (layout::W, h)).1
        };
        // 493 tall over a 40 px bottom bar: the panel ends on the area's bottom
        // and the picture rises 9 px past its top.
        let y = at(Rect::new(0, 0, 1920, 533), Edge::Bottom);
        assert_eq!(y + h, 493, "the panel's bottom is the area's");
        assert_eq!(y - layout::OVERHANG, -9, "the picture rises past the top");
        // 470 tall under a 40 px top bar: the panel rises 6 px over the bar.
        let y = at(Rect::new(0, 0, 1920, 510), Edge::Top);
        assert_eq!((y, y + h), (34, 510));
        assert_eq!(at(Rect::new(0, 0, 1920, 340), Edge::Bottom), 0);
        assert_eq!(at(Rect::new(0, 0, 1920, 340), Edge::Top), 0);
        let screen = Rect::new(0, 0, 1920, 1080);
        let (bottom, _) = bar_on(screen, Edge::Bottom);
        let above = Rect::new(0, 0, 1920, 1000);
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &bottom, (0, 0), above, (layout::W, h)).1,
            1000 - h
        );
        let (top, _) = bar_on(screen, Edge::Top);
        let below = Rect::new(0, 80, 1920, 1000);
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &top, (0, 0), below, (layout::W, h)).1,
            80
        );
    }

    #[test]
    fn the_shutdown_options_open_on_the_screen_side_of_the_bar() {
        let usable = Rect::new(0, 40, 1920, 1040);
        let arrow = Rect::new(339, 474, 22, 22);
        let size = (120, 130);
        assert_eq!(
            shutdown_options_at(Edge::Bottom, arrow, size, usable),
            (361, 496 - 130),
            "over a bottom bar: level with the arrow's foot"
        );
        assert_eq!(
            shutdown_options_at(Edge::Top, arrow, size, usable),
            (361, 474),
            "under a top bar: level with the arrow's top"
        );
        let low = Rect::new(339, 1000, 22, 22);
        assert_eq!(
            shutdown_options_at(Edge::Top, low, size, usable),
            (361, 1022 - 130)
        );
        for edge in [Edge::Left, Edge::Right] {
            assert_eq!(
                shutdown_options_at(edge, arrow, size, usable),
                (361, 474),
                "{edge:?}: Start tops a side bar, so they drop as under a top bar"
            );
        }
    }

    #[test]
    fn the_walking_keys_win_over_the_fields_edit_key() {
        assert_eq!(key_of(&press(keys::KEY_Return, 0)), Some(Key::Enter));
        assert_eq!(key_of(&press(keys::KEY_Escape, 0)), Some(Key::Escape));
        assert_eq!(key_of(&press(keys::KEY_KP_Up, 0)), Some(Key::Up));
        assert_eq!(key_of(&press(keys::KEY_Left, 0)), Some(Key::Left));
        assert_eq!(
            key_of(&press(keys::KEY_Right, keys::MOD_MOD2)),
            Some(Key::Right),
            "Num Lock makes no chord"
        );
        assert_eq!(key_of(&press(keys::KEY_Tab, 0)), Some(Key::Tab));
        assert_eq!(key_of(&press(keys::KEY_Menu, 0)), Some(Key::Menu));
        assert_eq!(
            key_of(&press(keys::KEY_F10, keys::MOD_SHIFT)),
            Some(Key::Menu)
        );
        assert_eq!(
            key_of(&press(keys::KEY_x, 0)),
            Some(Key::Char(EditKey::Insert("x".into())))
        );
    }

    #[test]
    fn a_chorded_caret_key_edits_the_field_and_shift_tab_does_nothing() {
        let shift_left = DialogKey {
            syms: vec![keys::KEY_Left],
            mods: keys::MOD_SHIFT,
            edit: Some(EditKey::Left {
                select: true,
                word: false,
            }),
        };
        assert_eq!(
            key_of(&shift_left),
            Some(Key::Char(EditKey::Left {
                select: true,
                word: false
            }))
        );
        assert_eq!(
            key_of(&press(keys::KEY_ISO_Left_Tab, keys::MOD_SHIFT)),
            None
        );
        let shift = DialogKey {
            syms: vec![keys::KEY_Shift_L],
            mods: keys::MOD_SHIFT,
            edit: None,
        };
        assert_eq!(key_of(&shift), None, "a modifier alone does nothing");
    }

    #[test]
    fn the_user_picture_is_looked_for_in_face_then_accounts_service() {
        assert_eq!(
            picture_paths(Some(Path::new("/home/jan")), "jan"),
            [
                PathBuf::from("/home/jan/.face"),
                PathBuf::from("/var/lib/AccountsService/icons/jan")
            ]
        );
        assert_eq!(
            picture_paths(None, "../x"),
            Vec::<PathBuf>::new(),
            "a user name is never a path"
        );
    }

    #[test]
    fn the_panel_outlives_its_output_once_the_scene_drops_it() {
        let out = |name: &str| {
            Output::new(
                name.into(),
                smithay::output::PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "Reprise".into(),
                    model: "Test".into(),
                    serial_number: "0".into(),
                },
            )
        };
        let (a, b) = (out("A-1"), out("B-1"));
        assert!(!outlived(&a, &[a.clone(), b.clone()]));
        assert!(outlived(&a, &[b]), "A-1 was removed");
        assert!(outlived(&a, &[]));
    }

    #[test]
    fn the_pointer_takes_the_panel_and_the_picture_only() {
        let l = layout::layout(&StartPanel::new());
        let t = takes_pointer(&l);
        let at = |x: i32, y: i32| Point::from(((x - l.canvas.x) as f64, (y - l.canvas.y) as f64));
        assert!(t(at(0, 0)));
        assert!(t(at(l.panel.width - 1, l.panel.height - 1)));
        let pic = l.picture.expect("Home shows the picture");
        assert!(t(at(pic.x + 1, pic.y + 1)), "the picture over the top edge");
        assert!(!t(at(1, -1)), "above the panel beside the picture");
        assert!(!t(at(pic.right(), -1)), "right of the picture");
    }

    #[test]
    fn the_buffer_is_the_panel_and_the_picture_above_it() {
        let l = layout::layout(&StartPanel::new());
        let pic = l.picture.expect("Home shows the picture");
        assert_eq!(l.canvas, l.panel.union(&pic));
        assert_eq!((l.canvas.x, l.canvas.y), (0, pic.y), "the buffer's offset");
        assert_eq!(l.canvas.bottom(), l.panel.bottom());
    }

    fn panels() -> Vec<StartPanel> {
        let row = |jump| ProgramRow {
            name: "Notepad".into(),
            icon: None,
            jump,
            new_pin: false,
        };
        let home = StartPanel::new();
        let mut both = StartPanel::new();
        both.pinned = vec![row(true)];
        both.frequent = vec![row(false)];
        let mut search = both.clone();
        search.set_mode(Mode::Search);
        let mut jump = both.clone();
        jump.set_mode(Mode::Jump(0));
        vec![home, both, search, jump]
    }

    #[test]
    fn the_glass_lies_under_every_pixel_the_panel_draws_and_no_other() {
        use reprisekit::glass::{Material, Settings};
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let text = reprisekit::interfaces::start_panel_text(&theme);
        for p in panels() {
            let l = layout::layout(&p);
            let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
            reprisekit::interfaces::draw_start_panel(
                &mut c,
                &theme,
                &Settings::default(),
                &text,
                &p,
                &l,
                None,
                true,
            );
            let pane = panel_pane(&l);
            assert_eq!(
                pane.holes,
                [l.pane],
                "{:?}: the white pane is clear",
                p.mode()
            );
            let shapes = crate::glass::drawn_shapes(&pane, Material::Frosted, 1.0);
            let alpha = |x: i32, y: i32| c.get(x - l.canvas.x, y - l.canvas.y) >> 24;
            let r = l.panel;
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    let glass = shapes.iter().any(|(s, _)| s.contains(x, y));
                    assert_eq!(
                        glass,
                        alpha(x, y) > 0,
                        "{:?} {}x{}: glass at ({x}, {y})",
                        p.mode(),
                        r.width,
                        r.height
                    );
                }
            }
            let h = l.pane;
            let corner = |x: i32, y: i32| {
                (x < h.x + 5 || x >= h.right() - 5) && (y < h.y + 5 || y >= h.bottom() - 5)
            };
            for y in h.y..h.bottom() {
                for x in h.x..h.right() {
                    if !corner(x, y) {
                        assert_eq!(alpha(x, y), 255, "{:?}: pane at ({x}, {y})", p.mode());
                    }
                }
            }
        }
    }

    #[test]
    fn the_panels_glass_rounds_its_top_corners_above_scale_1_as_its_chrome_does() {
        use reprisekit::glass::Material;
        let l = layout::layout(&StartPanel::new());
        let pane = panel_pane(&l);
        for scale in [1.5, 2.0] {
            assert_eq!(
                crate::glass::drawn_shapes(&pane, Material::Frosted, scale),
                vec![(l.panel, [6.0, 6.0, 0.0, 0.0])],
                "at {scale}"
            );
        }
        assert!(crate::glass::drawn_shapes(&pane, Material::Frosted, 1.0).len() > 1);
    }

    #[test]
    fn beside_a_side_bar_the_panel_hangs_level_with_start_its_picture_on_the_screen() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let h = layout::layout(&StartPanel::new()).panel.height;
        let (left, area) = bar_on(screen, Edge::Left);
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &left, (3, 0), area, (layout::W, h)),
            (40, layout::OVERHANG),
            "on the bar's right edge, the picture's top the output's"
        );
        assert_eq!(
            panel_origin(StartMenuDock::Into(4), &left, (3, 0), area, (layout::W, h)).0,
            36
        );
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &left, (3, 1000), area, (layout::W, h)).1,
            1080 - h,
            "kept above the output's foot"
        );
        let (right, area) = bar_on(screen, Edge::Right);
        assert_eq!(
            panel_origin(
                StartMenuDock::OnTop,
                &right,
                (1883, 0),
                area,
                (layout::W, h)
            )
            .0,
            1880 - layout::W,
            "its right edge on the bar's left edge"
        );
        let narrow = Rect::new(1920, 0, 400, 1080);
        let (right, area) = bar_on(narrow, Edge::Right);
        assert_eq!(
            panel_origin(
                StartMenuDock::OnTop,
                &right,
                (2283, 0),
                area,
                (layout::W, h)
            )
            .0,
            1920,
            "a right bar's panel stops at the output's left edge"
        );
        let (left, area) = bar_on(narrow, Edge::Left);
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &left, (1923, 0), area, (layout::W, h)).0,
            1920,
            "a left bar's panel stops at the output's left edge"
        );
        let wide = Rect::new(0, 0, 440, 1080);
        let (left, area) = bar_on(wide, Edge::Left);
        assert_eq!(
            panel_origin(StartMenuDock::OnTop, &left, (3, 0), area, (layout::W, h)).0,
            440 - layout::W,
            "a left bar's panel stops at the output's right edge"
        );
        let hidden = crate::barplace::place_hiding(screen, Edge::Left, 40, Some(1.0));
        let area = crate::barplace::usable(screen, &hidden);
        assert_eq!(
            panel_origin(
                StartMenuDock::Into(4),
                &hidden,
                (3, 0),
                area,
                (layout::W, h)
            )
            .0,
            2,
            "a hidden bar's panel stands on its reveal line"
        );
    }
}
