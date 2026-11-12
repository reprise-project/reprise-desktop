//! The program, tree, jump and place rows, and the app icons they wear.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use reprisefs::apps::{AppEntry, Group};
use reprisekit::startpanel::{
    Mode, PlaceRow, ProgramRow, ResultRow, Shape, Spot, StartPanel, TreeRow,
};

use crate::compositor::State;
use crate::config::MenuEntry;
use crate::raster::Canvas;

use super::{PICTURE, RESULT_ICON};

const PROGRAM_ICON: i32 = 32;
const TREE_ICON: i32 = 16;
const JUMP_TITLE: &str = "Recent";

pub(super) fn one_per_name(apps: &[AppEntry]) -> Vec<bool> {
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

pub(super) fn tree_rows(
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

pub(super) fn frequent_rows(
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

/// The grey line under a pinned row's name in the Blue menu ([`reprisefs::apps::subtitle`]).
fn subtitle(a: &AppEntry) -> Option<String> {
    let file = reprisefs::apps::find_file(&a.id)?;
    reprisefs::apps::subtitle(&reprisefs::apps::read_desktop_file(&file).ok()?)
}

/// An app's icon for a panel row, its file never read on the loop ([`crate::icon::unread`]).
pub(super) fn app_icon(
    icons: &mut crate::icontheme::IconCache,
    a: &AppEntry,
    px: i32,
) -> Option<Canvas> {
    (a.icon.as_deref()).and_then(|n| unread(icons, n, px))
}

pub(super) fn unread(
    icons: &mut crate::icontheme::IconCache,
    name: &str,
    px: i32,
) -> Option<Canvas> {
    crate::icon::unread(icons, name, px).map(std::sync::Arc::unwrap_or_clone)
}

/// Whether `icon` is a blank stand-in for an icon still decoding.
fn blank(icon: &Option<Canvas>) -> bool {
    icon.as_ref()
        .is_some_and(|c| c.pixels.iter().all(|&p| p == 0))
}

/// Whether any icon `m` shows is a blank stand-in.
fn shows_stand_in(m: &StartPanel) -> bool {
    let tree = m.tree.iter().map(|r| match r {
        TreeRow::Folder { icon, .. } | TreeRow::App { icon, .. } => icon,
    });
    let results = m.results.iter().flat_map(|g| &g.rows).map(|r| &r.icon);
    (m.pinned.iter().chain(&m.frequent).map(|r| &r.icon))
        .chain(m.places.iter().map(|r| &r.picture))
        .chain(m.jump.iter().map(|r| &r.icon))
        .chain(tree)
        .chain(results)
        .chain(std::iter::once(&m.user_picture))
        .any(blank)
}

/// The logical px of a place row's picture: the Blue menu's are 24 px icons.
fn place_px(shape: Shape) -> i32 {
    match shape {
        Shape::Glass => PICTURE,
        Shape::Blue => reprisekit::blue::startpanel::PLACE_ICON_PX,
    }
}

/// Gives each app row of `rows` (whose apps `map` holds) the icon `icon` finds for its app.
pub(super) fn refill_tree(
    rows: &mut [TreeRow],
    map: &[Option<usize>],
    mut icon: impl FnMut(usize) -> Option<Canvas>,
) {
    for (row, app) in rows.iter_mut().zip(map) {
        if let (TreeRow::App { icon: worn, .. }, Some(i)) = (row, app) {
            *worn = icon(*i);
        }
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
            icon: unread(icons, reprisefs::icons::icon_for_file(p), px),
        })
        .collect()
}

pub(super) fn jump_file(jumps: &[Vec<PathBuf>], mode: Mode, k: usize) -> Option<&Path> {
    let Mode::Jump(i) = mode else {
        return None;
    };
    jumps.get(i)?.get(k).map(PathBuf::as_path)
}

pub(super) fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The tree's scroll, lit and hot rows index the old rows and are dropped.
pub(super) fn replace_tree(model: &mut StartPanel, rows: Vec<TreeRow>) {
    model.tree = rows;
    model.tree_top = 0;
    if model.mode() == Mode::Programs {
        model.set_mode(Mode::Programs);
    }
}

impl State {
    /// The panel's places for the document folders the last check left, with their pictures at
    /// `scale` (the Blue menu's 24 px icons), and `startmenu::places()` or `blue_places()` without
    /// its rules.
    pub(super) fn start_places(
        &mut self,
        shape: Shape,
        scale: i32,
        user: &str,
    ) -> (Vec<PlaceRow>, Vec<MenuEntry>) {
        let dirs = self.recent.document_dirs();
        let recent = self.config.recent_files;
        let entries = match shape {
            Shape::Glass => crate::startmenu::places(recent, &dirs),
            Shape::Blue => crate::startmenu::blue_places(recent, &dirs),
        };
        let px = place_px(shape) * scale;
        let mut rows = Vec::new();
        let mut places = Vec::new();
        let mut rule = false;
        for e in entries {
            let MenuEntry::Item { label, icon, .. } = &e else {
                rule = true;
                continue;
            };
            rows.push(PlaceRow {
                label: crate::startmenu::expand_label(label, user),
                picture: icon.as_deref().and_then(|n| unread(&mut self.icons, n, px)),
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
        let scale = self.draw_scale_at(Some(&output)).max(1);
        let Some(shape) = self.start_panel.as_ref().map(|v| v.model.shape) else {
            return;
        };
        let (rows, places) = self.start_places(shape, scale, &crate::lock::current_user());
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

    pub(super) fn list_start_programs(&mut self) {
        let Some(scale) = self.start_panel_scale() else {
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
        let blue = v.model.shape == Shape::Blue;
        v.jumps = programs
            .iter()
            .map(|&i| {
                if config.recent_files && !blue {
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
                let pin = k < pinned.len();
                ProgramRow {
                    name: a.name.clone(),
                    icon: app_icon(icons, a, px),
                    jump: !jumps.is_empty(),
                    new_pin: pin && v.new_pins.contains(&a.id),
                    detail: (blue && pin).then(|| subtitle(a)).flatten(),
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

    pub(super) fn list_start_jump(&mut self, i: usize) {
        let Some(scale) = self.start_panel_scale() else {
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

    pub(super) fn list_start_tree(&mut self) {
        let Some(scale) = self.start_panel_scale() else {
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
        let folder = unread(icons, "folder", px);
        for row in &mut rows {
            if let TreeRow::Folder { icon, .. } = row {
                *icon = folder.clone();
            }
        }
        refill_tree(&mut rows, &map, |i| app_icon(icons, &app_index[i], px));
        replace_tree(&mut v.model, rows);
        v.tree_apps = map;
    }

    /// Icons `keys` (icon files or theme names) landed: if the open panel shows a blank stand-in
    /// ([`crate::icon::stand_in`]), a file landed, or names `repick` turned out not to decode,
    /// every icon it shows is looked up again (file rows pick their names again), and the panel
    /// draws again if one changed.
    pub(crate) fn start_panel_icons_landed(&mut self, keys: &[&str], repick: bool) {
        let Some(scale) = self.start_panel_scale() else {
            return;
        };
        let file = repick || keys.iter().any(|k| k.starts_with('/'));
        if !file && !(self.start_panel.as_ref()).is_some_and(|v| shows_stand_in(&v.model)) {
            return;
        }
        let State {
            start_panel,
            app_index,
            icons,
            start_pictures,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let before = v.model.clone();
        let px = PROGRAM_ICON * scale;
        let rows = v.model.pinned.iter_mut().chain(&mut v.model.frequent);
        for (row, &i) in rows.zip(&v.programs) {
            row.icon = app_icon(icons, &app_index[i], px);
        }
        let px = TREE_ICON * scale;
        let folder = (v.model.tree.iter())
            .any(|r| matches!(r, TreeRow::Folder { .. }))
            .then(|| unread(icons, "folder", px))
            .flatten();
        for row in &mut v.model.tree {
            if let TreeRow::Folder { icon, .. } = row {
                icon.clone_from(&folder);
            }
        }
        refill_tree(&mut v.model.tree, &v.tree_apps, |i| {
            app_icon(icons, &app_index[i], px)
        });
        let px = place_px(v.model.shape) * scale;
        for (row, e) in v.model.places.iter_mut().zip(&v.places) {
            if let MenuEntry::Item { icon, .. } = e {
                row.picture = icon.as_deref().and_then(|n| unread(icons, n, px));
            }
        }
        if let Mode::Jump(i) = v.model.mode()
            && let Some(files) = v.jumps.get(i)
        {
            let px = RESULT_ICON * scale;
            for (row, f) in v.model.jump.iter_mut().zip(files) {
                row.icon = unread(icons, reprisefs::icons::icon_for_file(f), px);
            }
        }
        let px = PICTURE * scale;
        if start_pictures.shown(px).is_none() {
            v.model.user_picture = unread(icons, "avatar-default", px);
        }
        let mut results = (v.model.results.iter()).flat_map(|g| &g.rows);
        if v.model.mode() == Mode::Search && (file || results.any(|r| blank(&r.icon))) {
            self.start_panel_show_results();
        }
        if self.start_panel.as_ref().is_some_and(|v| v.model != before) {
            self.redraw_start_panel();
        }
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

    /// Relists the open panel's jump lists from the last recent read and check, the one shown
    /// in place.
    pub(crate) fn relist_start_jumps(&mut self) {
        let Some(scale) = self.start_panel_scale() else {
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
}
