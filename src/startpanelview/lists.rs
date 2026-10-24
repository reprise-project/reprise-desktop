//! The program, tree, jump and place rows, and the app icons they wear.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use reprisefs::apps::{AppEntry, Group};
use reprisekit::startpanel::{Mode, PlaceRow, ProgramRow, ResultRow, Spot, StartPanel, TreeRow};

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
            icon: icons.get(reprisefs::icons::icon_for_file(p), px),
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
    /// The panel's places for the document folders the last check left, with their `px`
    /// pictures, and `startmenu::places()` without its rules.
    pub(super) fn start_places(&mut self, px: i32, user: &str) -> (Vec<PlaceRow>, Vec<MenuEntry>) {
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

    pub(super) fn list_start_programs(&mut self) {
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
                    icon: app_icon(icons, a, px),
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

    pub(super) fn list_start_jump(&mut self, i: usize) {
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

    pub(super) fn list_start_tree(&mut self) {
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
        for row in &mut rows {
            if let TreeRow::Folder { icon, .. } = row {
                *icon = folder.clone();
            }
        }
        refill_tree(&mut rows, &map, |i| app_icon(icons, &app_index[i], px));
        replace_tree(&mut v.model, rows);
        v.tree_apps = map;
    }

    /// Icon file `file` landed: the program, tree and search rows wearing it take it.
    pub(crate) fn start_panel_icon_landed(&mut self, file: &str) {
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
        let wears = |i: &usize| app_index.get(*i).and_then(|a| a.icon.as_deref()) == Some(file);
        let programs = v.programs.iter().any(wears);
        let tree = v.tree_apps.iter().flatten().any(wears);
        let results = v.model.mode() == Mode::Search
            && (v.result_apps.iter().any(wears)
                || (v.result_settings.iter()).any(|s| s.icon(app_index).as_deref() == Some(file)));
        if programs {
            let px = PROGRAM_ICON * scale;
            let rows = v.model.pinned.iter_mut().chain(&mut v.model.frequent);
            for (row, &i) in rows.zip(&v.programs) {
                row.icon = app_icon(icons, &app_index[i], px);
            }
        }
        if tree {
            let px = TREE_ICON * scale;
            refill_tree(&mut v.model.tree, &v.tree_apps, |i| {
                app_icon(icons, &app_index[i], px)
            });
        }
        if results {
            self.start_panel_show_results();
        }
        if programs || tree || results {
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
}
