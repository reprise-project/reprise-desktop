//! The application launcher on the scene: one `reprisekit::controls::launcher` panel centred on the output under the pointer, on the menu layer, fed every key press while open. On every change of the typed text it works out the calculator's answer (`search::calc_row`), ranks the Start menu's app index (`State::app_index`, by `search::rank_apps`) and finds settings pages (`search::rank_settings`). Enter on the answer copies it; a program starts as from the Start menu; a settings row opens the settings program on its page.
//! From two typed characters it also searches the home folder for files through `search::FileSearch`, unless `[launcher] search_files` is off.
//! Unless `[launcher] windows` is off it finds open windows on every desktop by title or program name (`search::rank_windows`); Enter brings one up as its task button would.
//! The panel's tree in `layers.menu` is tagged `SceneOwner::Launcher` and holds the glass tree (its `Backdrop` under a glass look), then the buffer, which takes the pointer on the panel only: its shadow lets the pointer through.
//! A right press or the menu key on a program or file row opens its context menu (`open_launcher_context`); picking a row of it closes the panel.
//! `Action::Launcher` (Super+Space by default) opens it and its own bind closes it; `KeyRoute::Launcher` hands it every other press. Locking closes it.

use std::path::PathBuf;

use reprisekit::controls::launcher::{self, Event, FileRow, Hit, Key, Launcher, Pick, Row};
use reprisekit::fuzzy;
use reprisekit::look::panel;
use reprisescene::{BufferId, Input, TreeId};
use smithay::utils::Rectangle;

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::keys;
use crate::menuview::{MenuFrom, MenuKind};
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::search::{
    CALC_ICON, FileSearch, SearchOwner, Setting, WindowName, calc_row, file_icon_name, file_row,
    file_row_icon, rank_apps, rank_settings, rank_windows, windows_first,
};
use crate::window::WindowId;

pub struct LauncherView {
    pub model: Launcher,
    matches: Vec<usize>,
    /// The windows the window rows show, in order.
    windows: Vec<WindowId>,
    /// The answer the calculator's row copies, while it is shown.
    answer: Option<String>,
    settings: Vec<Setting>,
    /// The file search; its `files` are parallel to the model's file rows.
    pub search: FileSearch,
    area: Rect,
    at: (i32, i32),
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
    buffer: BufferId,
}

/// The launcher's key for a press the filter caught: Enter (Ctrl+Enter
/// for a file row's folder), Escape, the arrows, the menu key or
/// Shift+F10, or an edit key for the field; None for a key it does not use.
fn key_of(k: &DialogKey) -> Option<Key> {
    let ctrl = k.mods & keys::MOD_CTRL != 0;
    let special = k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_Menu => Some(Key::Menu),
        keys::KEY_F10 if k.mods == keys::MOD_SHIFT => Some(Key::Menu),
        keys::KEY_Return | keys::KEY_KP_Enter if ctrl => Some(Key::CtrlEnter),
        keys::KEY_Return | keys::KEY_KP_Enter => Some(Key::Enter),
        keys::KEY_Escape => Some(Key::Escape),
        keys::KEY_Up | keys::KEY_KP_Up => Some(Key::Up),
        keys::KEY_Down | keys::KEY_KP_Down => Some(Key::Down),
        _ => None,
    });
    special.or_else(|| k.edit.clone().map(Key::Char))
}

fn panel_origin(area: Rect, (width, height): (i32, i32)) -> (i32, i32) {
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    (x.max(area.x), y.max(area.y))
}

impl State {
    pub fn toggle_launcher(&mut self) {
        if self.launcher.is_some() {
            self.close_launcher();
            return;
        }
        let pos = self.pointer_location();
        let out = self
            .output_at(pos.x, pos.y)
            .or_else(|| self.scene.outputs().first().cloned());
        let area = self.usable_area(out.as_ref());
        if area.width <= 0 {
            return;
        }
        let tree = self.scene.tree_create(self.layers.menu);
        self.scene.set_data(tree, Some(SceneOwner::Launcher));
        crate::glass::leave_out(&mut self.scene, tree);
        if let Some(o) = &out {
            crate::output::keep_on(&mut self.scene, tree, o);
        }
        let glass_tree = self.scene.tree_create(tree);
        let buffer = self.scene.buffer_create(tree);
        self.launcher = Some(LauncherView {
            model: Launcher::new(),
            matches: Vec::new(),
            windows: Vec::new(),
            answer: None,
            settings: Vec::new(),
            search: FileSearch::new(),
            area,
            at: (area.x, area.y),
            tree,
            glass_tree,
            backdrop: Backdrop::new(&self.glass),
            buffer,
        });
        self.restart_caret();
        self.redraw_launcher();
    }

    pub fn close_launcher(&mut self) {
        let Some(mut v) = self.launcher.take() else {
            return;
        };
        v.search.stop(&self.handle);
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        self.destroy_redrawn(v.tree);
        // A dialog's field under it gets the blink back.
        if self.editor().is_some() {
            self.restart_caret();
        } else {
            self.stop_caret();
        }
        crate::access::touch(self);
    }

    pub fn launcher_key(&mut self, k: &DialogKey) {
        let Some(key) = key_of(k) else {
            return;
        };
        // Copy and Cut offer the field's selection; Paste asks for the
        // clipboard's text, which lands in `launcher_paste`.
        if let Key::Char(e) = &key {
            self.clip_edit_key(e);
        }
        let menu = key == Key::Menu;
        let Some(v) = self.launcher.as_mut() else {
            return;
        };
        let event = v.model.key(key);
        self.restart_caret();
        // Without a row's menu to open, the menu key opens the field's.
        if menu && event == Event::None && self.open_edit_menu(None) {
            return;
        }
        self.apply_launcher(event);
    }

    pub fn launcher_paste(&mut self, text: &str) {
        let Some(v) = self.launcher.as_mut() else {
            return;
        };
        let event = v.model.paste(text);
        self.apply_launcher(event);
    }

    /// Text dropped on the field becomes the query (`dialogdrop`); `true` when it went in.
    pub fn launcher_dropped(&mut self, text: &str) -> bool {
        let Some(v) = self.launcher.as_mut() else {
            return false;
        };
        let event = v.model.drop_query(text);
        let took = event == Event::QueryChanged;
        self.apply_launcher(event);
        took
    }

    pub fn launcher_hit(&self, lx: f64, ly: f64) -> Hit {
        let Some(v) = self.launcher.as_ref() else {
            return Hit::Panel;
        };
        let (x, y) = (lx.floor() as i32 - v.at.0, ly.floor() as i32 - v.at.1);
        launcher::hit(&v.model, x, y, &self.config.theme)
    }

    pub fn launcher_clicked(&mut self, hit: Hit) {
        if let Hit::Row(p) = hit {
            self.apply_launcher(Event::Launch(p));
        } else {
            self.restart_caret();
            self.redraw_launcher();
        }
    }

    /// A reader's focus on row `p`: it takes the highlight, and nothing starts.
    pub fn launcher_highlight(&mut self, p: Pick) {
        if let Some(v) = self.launcher.as_mut()
            && v.model.highlight(p)
        {
            self.redraw_launcher();
        }
    }

    /// A right press on the panel: a program or file row opens its menu.
    pub fn launcher_right_clicked(&mut self, hit: Hit) {
        let Hit::Row(p) = hit else {
            return;
        };
        let Some(v) = self.launcher.as_mut() else {
            return;
        };
        let event = v.model.right_press(p);
        self.apply_launcher(event);
    }

    /// Row `pick`'s context menu: a program row's is the Start menu's, a
    /// file row's `filemenu`'s. It opens at the pointer when that is on the
    /// row, else under the row.
    fn open_launcher_context(&mut self, pick: Pick) {
        self.redraw_launcher();
        let Some(v) = self.launcher.as_ref() else {
            return;
        };
        let p = self.pointer_location();
        let at = if self.launcher_hit(p.x, p.y) == Hit::Row(pick) {
            (p.x as i32, p.y as i32)
        } else {
            let line = v.model.list.selected.unwrap_or(0);
            match launcher::row_rect(&v.model, line, &self.config.theme) {
                Some(r) => (v.at.0 + r.x, v.at.1 + r.bottom()),
                None => return,
            }
        };
        let (items, kind) = match pick {
            Pick::App(i) => {
                let Some(&app) = v.matches.get(i) else {
                    return;
                };
                let Some(entry) = self.app_index.get(app) else {
                    return;
                };
                let pinned_start = self.launches.is_pinned(&entry.id);
                let pinned_taskbar = self.quick_file_of(&entry.id).is_some();
                let (items, rows) =
                    crate::startmenu::program_menu(app, pinned_start, pinned_taskbar, false);
                (items, MenuKind::StartPanel { rows })
            }
            Pick::File(i) => {
                let Some(path) = v.search.files.get(i).cloned() else {
                    return;
                };
                let facts = v.search.facts_of(&path);
                let (items, commands) = self.file_menu_rows(&path, &facts);
                (
                    items,
                    MenuKind::File {
                        commands,
                        path,
                        facts,
                    },
                )
            }
            Pick::Calc | Pick::Window(_) | Pick::Setting(_) => return,
        };
        self.close_menu();
        self.push_context_menu(at, items, kind, MenuFrom::Elsewhere);
    }

    fn apply_launcher(&mut self, event: Event) {
        match event {
            Event::None => self.redraw_launcher(),
            Event::QueryChanged => {
                self.rerank_launcher();
                self.launcher_query_changed();
                self.redraw_launcher();
            }
            Event::Launch(pick) => {
                let Some(v) = self.launcher.as_ref() else {
                    return;
                };
                let apps = &self.app_index;
                enum Launch {
                    App(reprisefs::apps::AppEntry),
                    File(crate::desktop::FileFacts),
                    Copy(String),
                    Setting(Setting),
                    Window(WindowId),
                }
                let launch = match pick {
                    Pick::Calc => v.answer.clone().map(Launch::Copy),
                    Pick::Setting(i) => v.settings.get(i).copied().map(Launch::Setting),
                    Pick::Window(i) => v.windows.get(i).copied().map(Launch::Window),
                    Pick::App(i) => v
                        .matches
                        .get(i)
                        .and_then(|&ai| apps.get(ai))
                        .cloned()
                        .map(Launch::App),
                    Pick::File(i) => v
                        .search
                        .files
                        .get(i)
                        .map(|p| Launch::File(v.search.facts_of(p))),
                };
                self.close_launcher();
                match launch {
                    Some(Launch::App(entry)) => self.launch_app(&entry),
                    Some(Launch::File(facts)) => self.open_found(&facts),
                    Some(Launch::Copy(answer)) => self.clip_copy(&answer),
                    Some(Launch::Setting(s)) => self.open_setting(s),
                    Some(Launch::Window(w)) => self.bring_up(w),
                    None => {}
                }
            }
            Event::OpenFolder(i) => {
                let path = self
                    .launcher
                    .as_ref()
                    .and_then(|v| v.search.files.get(i).cloned());
                self.close_launcher();
                if let Some(path) = path {
                    self.open_file_location(&path);
                }
            }
            Event::Context(pick) => self.open_launcher_context(pick),
            Event::Close => self.close_launcher(),
        }
    }

    fn launcher_scale(&self) -> i32 {
        let Some(area) = self.launcher.as_ref().map(|v| v.area) else {
            return 1;
        };
        let out = self.output_at(
            (area.x + area.width / 2) as f64,
            (area.y + area.height / 2) as f64,
        );
        self.draw_scale_at(out.as_ref()).max(1)
    }

    /// The app index was rescanned (`appindex`): rank the typed text
    /// against the new one, so `matches` never points into an index that
    /// is gone.
    pub fn launcher_index_changed(&mut self) {
        if self.launcher.is_some() {
            self.rerank_launcher();
            self.redraw_launcher();
        }
    }

    /// Work out the answer, rank the app index and find the settings for
    /// the typed text, and hand the rows to the model. The rows' icons are
    /// loaded at the scale of the panel's output; a scale change
    /// mid-typing is picked up by the next key.
    fn rerank_launcher(&mut self) {
        let scale = self.launcher_scale();
        let candidates = if self.config.launcher_windows {
            self.launcher_window_candidates()
        } else {
            Vec::new()
        };
        let State {
            launcher,
            app_index,
            icons,
            windows,
            ..
        } = self;
        let Some(v) = launcher.as_mut() else {
            return;
        };
        let query = v.model.query().to_string();
        let px = 16 * scale;
        let names: Vec<WindowName> = (candidates.iter())
            .filter_map(|&id| windows.iter().find(|w| w.id == id))
            .map(|w| {
                let app_id = w.app_id();
                let app = reprisefs::apps::entry_for_app_id(app_index, &app_id, |_| true)
                    .map_or(app_id.clone(), |e| e.name.clone());
                WindowName {
                    title: w.title(),
                    app,
                }
            })
            .collect();
        let found = rank_windows(&query, &names, launcher::MAX_WINDOWS);
        let calc = calc_row(&query);
        v.model.set_calc(calc.as_ref().map(|(name, _)| Row {
            name: name.clone(),
            icon: unread(icons, CALC_ICON, px),
        }));
        v.answer = calc.map(|(_, answer)| answer);
        let matches = rank_apps(&query, app_index, launcher::MAX_ROWS);
        let best_app = (matches.first()).and_then(|&i| fuzzy::tier(&query, &app_index[i].name));
        let first = windows_first(found.first().map(|h| h.1), best_app);
        v.windows = found.iter().map(|&(i, _)| candidates[i]).collect();
        let window_rows = (found.iter())
            .map(|&(i, _)| Row {
                name: names[i].title.clone(),
                icon: (windows.iter().find(|w| w.id == candidates[i]))
                    .and_then(|w| crate::icon::window_icon(icons, app_index, w, px))
                    .map(std::sync::Arc::unwrap_or_clone),
            })
            .collect();
        v.model.set_windows(window_rows, first);
        v.settings = rank_settings(&query, app_index, &matches, launcher::MAX_SETTINGS);
        let settings = v
            .settings
            .iter()
            .map(|s| Row {
                name: s.name(app_index),
                icon: s.icon(app_index).and_then(|n| unread(icons, &n, px)),
            })
            .collect();
        v.model.set_settings(settings);
        let rows: Vec<Row> = matches
            .iter()
            .map(|&ai| {
                let a = &app_index[ai];
                Row {
                    name: a.name.clone(),
                    icon: a.icon.as_deref().and_then(|n| unread(icons, n, px)),
                }
            })
            .collect();
        v.matches = matches;
        v.model.set_rows(rows);
    }

    /// Icons landed or names turned out to decode at no size: every row looks its icon up again
    /// (a file row picks its name again), and the panel draws again if one changed.
    pub(crate) fn launcher_icons_landed(&mut self) {
        let px = 16 * self.launcher_scale();
        let State {
            launcher,
            app_index,
            icons,
            windows,
            desktop,
            ..
        } = self;
        let Some(v) = launcher.as_mut() else {
            return;
        };
        let windows = (v.windows.iter())
            .map(|id| {
                (windows.iter().find(|w| w.id == *id))
                    .and_then(|w| crate::icon::window_icon(icons, app_index, w, px))
                    .map(std::sync::Arc::unwrap_or_clone)
            })
            .collect();
        let apps = (v.matches.iter())
            .map(|&i| {
                let name = app_index.get(i).and_then(|a| a.icon.as_deref());
                name.and_then(|n| unread(icons, n, px))
            })
            .collect();
        let settings = (v.settings.iter())
            .map(|s| s.icon(app_index).and_then(|n| unread(icons, &n, px)))
            .collect();
        let empty;
        let mime = match desktop.as_ref() {
            Some(d) => &d.mime,
            None => {
                empty = reprisefs::apps::MimeDb::empty();
                &empty
            }
        };
        let files = (v.search.files.iter())
            .map(|p| file_row_icon(p, v.search.is_dir(p), mime, icons, px))
            .collect();
        let refill = launcher::Refill {
            calc: (v.answer.as_ref()).and_then(|_| unread(icons, CALC_ICON, px)),
            windows,
            apps,
            settings,
            files,
        };
        if v.model.refill_icons(refill) {
            self.redraw_launcher();
        }
    }

    /// The text changed: stop the search in flight, narrow the file rows
    /// shown to the new text, and arm the timer for the next search. Under
    /// `MIN_CHARS`, or with file search turned off, the file rows go.
    fn launcher_query_changed(&mut self) {
        let Some(v) = self.launcher.as_ref() else {
            return;
        };
        let query = v.model.query().to_string();
        let kept = self.file_query_changed(SearchOwner::Launcher, &query, launcher::MAX_FILES);
        self.show_launcher_files(kept);
    }

    pub(crate) fn show_launcher_files(&mut self, paths: Vec<PathBuf>) {
        let px = 16 * self.launcher_scale();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let State {
            launcher,
            icons,
            desktop,
            ..
        } = self;
        let Some(v) = launcher.as_mut() else {
            return;
        };
        let empty;
        let mime = match desktop.as_ref() {
            Some(d) => &d.mime,
            None => {
                empty = reprisefs::apps::MimeDb::empty();
                &empty
            }
        };
        let rows: Vec<FileRow> = paths
            .iter()
            .map(|p| file_row(p, v.search.is_dir(p), home.as_deref(), mime, icons, px))
            .collect();
        v.search.files = paths;
        v.model.set_files(rows);
    }

    pub fn redraw_launcher(&mut self) {
        let Some(v) = self.launcher.as_ref() else {
            return;
        };
        let tree = v.tree;
        let was = self.scene.outputs_of(tree);
        let lay = launcher::layout(&v.model, &self.config.theme);
        let at = panel_origin(v.area, (lay.width, lay.height));
        let out = self.output_at(
            (at.0 + lay.width / 2) as f64,
            (at.1 + lay.height / 2) as f64,
        );
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let theme = self.config.theme.scaled(scale);
        // The canvas reaches past the panel by the glass shadow, if any.
        let m = panel::margin(&self.config.theme);
        let pane = match self.config.theme.glass() {
            Some(g) if crate::glass::wanted(&self.config) => Some(Pane {
                shapes: vec![PaneShape {
                    rect: Rect::new(0, 0, lay.width, lay.height),
                    corner: g.panel.corner,
                    square_bottom: false,
                    square_top: false,
                }],
                holes: Vec::new(),
            }),
            _ => None,
        };
        let preedit = self.ime_field_preedit(crate::caret::Editor::Launcher);
        let State {
            launcher,
            icons,
            desktop,
            app_index,
            scene,
            glass,
            text,
            caret,
            windows,
            ..
        } = self;
        let Some(v) = launcher.as_mut() else {
            return;
        };
        v.at = at;
        scene.set_position(v.tree, at.0, at.1);
        scene.set_position(v.buffer, -m.left, -m.top);
        scene.set_input(
            v.buffer,
            Input::Rect(Rectangle::new(
                (m.left, m.top).into(),
                (lay.width, lay.height).into(),
            )),
        );
        v.backdrop.set(scene, glass, v.glass_tree, pane);
        let icon_name = match v.model.picked() {
            Some(Pick::Calc) => Some(CALC_ICON.to_string()),
            Some(Pick::Setting(i)) => v.settings.get(i).and_then(|s| s.icon(app_index)),
            Some(Pick::Window(_)) => None,
            Some(Pick::App(i)) => v
                .matches
                .get(i)
                .and_then(|&ai| app_index.get(ai))
                .and_then(|a| a.icon.clone()),
            Some(Pick::File(i)) => v.search.files.get(i).map(|p| {
                let empty;
                let mime = match desktop.as_ref() {
                    Some(d) => &d.mime,
                    None => {
                        empty = reprisefs::apps::MimeDb::empty();
                        &empty
                    }
                };
                file_icon_name(p, v.search.is_dir(p), mime, icons)
            }),
            None => None,
        }
        .unwrap_or_else(|| "system-search".to_string());
        let window_icon = match v.model.picked() {
            Some(Pick::Window(i)) => (v.windows.get(i))
                .and_then(|&id| windows.iter().find(|w| w.id == id))
                .and_then(|w| crate::icon::window_icon(icons, app_index, w, launcher::ICON * scale))
                .map(std::sync::Arc::unwrap_or_clone),
            _ => None,
        };
        let icon = window_icon
            .or_else(|| unread(icons, &icon_name, launcher::ICON * scale))
            .unwrap_or_else(|| reprisekit::icon::fallback(&theme, launcher::ICON * scale));
        let mut canvas = launcher::draw(
            &theme,
            text.for_output(scale),
            &v.model,
            Some(&icon),
            caret.on,
            preedit.as_ref(),
        );
        upload_scaled(scene, v.buffer, &canvas, scale);
        let spot = canvas.take_spot();
        let origin = (at.0 - m.left, at.1 - m.top);
        self.ime_field_drawn(crate::caret::Editor::Launcher, spot, origin, scale);
        self.redraw_nodes(was, &[tree.into()]);
        crate::access::touch(self);
    }

    /// The windows the launcher can find, the most recently focused first: those
    /// with a task button on any desktop.
    fn launcher_window_candidates(&self) -> Vec<WindowId> {
        let listed = |w: &crate::window::Window| {
            w.mapped && !w.is_owned() && (w.minimized || !w.skips_taskbar())
        };
        let recent = self.focus_order.iter().rev().copied();
        let rest = self.windows.iter().map(|w| w.id);
        let mut out: Vec<WindowId> = Vec::new();
        for id in recent.chain(rest) {
            if !out.contains(&id) && self.win(id).is_some_and(listed) {
                out.push(id);
            }
        }
        out
    }

    pub fn launcher_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.launcher.as_ref()?;
        let lay = launcher::layout(&v.model, &self.config.theme);
        let rect = Rect::new(v.at.0, v.at.1, lay.width, lay.height);
        let mut tree = crate::access::root(Role::Window, "Launcher", rect);
        let px = self.config.theme.fonts.size_px;
        let theme = &self.config.theme;
        let focus = launcher::access(&v.model, &self.text, px, &mut tree, ROOT, theme);
        tree.set_focus(focus);
        Some(tree)
    }
}

/// Icon `name` for a launcher row, its file never read on the loop ([`crate::icon::unread`]).
fn unread(
    icons: &mut crate::icontheme::IconCache,
    name: &str,
    px: i32,
) -> Option<crate::raster::Canvas> {
    crate::icon::unread(icons, name, px).map(std::sync::Arc::unwrap_or_clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::EditKey;

    #[test]
    fn enter_escape_the_arrows_and_the_menu_keys_win_over_the_fields_edit_key() {
        let press = |sym: u32, mods: u32| DialogKey {
            syms: vec![sym],
            mods,
            edit: Some(EditKey::Insert("x".into())),
        };
        assert_eq!(key_of(&press(keys::KEY_Return, 0)), Some(Key::Enter));
        assert_eq!(
            key_of(&press(keys::KEY_KP_Enter, keys::MOD_CTRL)),
            Some(Key::CtrlEnter)
        );
        assert_eq!(key_of(&press(keys::KEY_Escape, 0)), Some(Key::Escape));
        assert_eq!(key_of(&press(keys::KEY_KP_Up, 0)), Some(Key::Up));
        assert_eq!(key_of(&press(keys::KEY_Down, 0)), Some(Key::Down));
        assert_eq!(key_of(&press(keys::KEY_Menu, 0)), Some(Key::Menu));
        assert_eq!(
            key_of(&press(keys::KEY_F10, keys::MOD_SHIFT)),
            Some(Key::Menu)
        );
        assert_ne!(key_of(&press(keys::KEY_F10, 0)), Some(Key::Menu));
        assert_eq!(
            key_of(&press(keys::KEY_x, 0)),
            Some(Key::Char(EditKey::Insert("x".into())))
        );
        let shift = DialogKey {
            syms: vec![keys::KEY_Shift_L],
            mods: keys::MOD_SHIFT,
            edit: None,
        };
        assert_eq!(key_of(&shift), None, "a modifier alone does nothing");
    }

    #[test]
    fn the_panel_centres_in_the_usable_area_and_keeps_its_corner_inside() {
        let area = Rect::new(100, 0, 1820, 1040);
        assert_eq!(
            panel_origin(area, (launcher::W, 48)),
            (100 + (1820 - launcher::W) / 2, (1040 - 48) / 2)
        );
        let small = Rect::new(10, 20, 300, 100);
        assert_eq!(panel_origin(small, (420, 400)), (10, 20));
    }

    fn typed(h: &mut crate::harness::Harness, text: &str) {
        let v = h.state.launcher.as_mut().expect("the launcher is open");
        let event = v.model.key(Key::Char(EditKey::Insert(text.into())));
        h.state.apply_launcher(event);
    }

    /// An editor minimized on desktop 2 and a terminal focused on desktop 1.
    fn two_windows(h: &mut crate::harness::Harness) -> (crate::harness::TestClient, WindowId) {
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let editor = h.map_toplevel(&mut c, "org.example.Editor", 300, 200);
        c.side.shells[editor.shell]
            .toplevel
            .set_title("notes.txt - Editor".into());
        c.side.shells[editor.shell].surface.commit();
        let term = h.map_toplevel(&mut c, "org.example.Term", 300, 200);
        h.roundtrip(&mut c);
        h.state.set_minimized(editor.id, true);
        h.state.move_to_desktop(&[editor.id], 1);
        h.state.focus_window(term.id);
        (c, editor.id)
    }

    #[test]
    fn typing_finds_an_open_window_and_enter_brings_it_up_on_its_desktop() {
        let mut h = crate::harness::Harness::new();
        let (_c, editor) = two_windows(&mut h);
        h.state.toggle_launcher();
        typed(&mut h, "notes");
        let v = h.state.launcher.as_ref().unwrap();
        assert_eq!(v.windows, [editor], "found by its title");
        assert_eq!(v.model.picked(), Some(Pick::Window(0)));
        h.state.apply_launcher(Event::Launch(Pick::Window(0)));
        assert!(h.state.launcher.is_none(), "the launcher closes");
        let win = h.state.win(editor).unwrap();
        assert!(!win.minimized, "the window comes back");
        assert_eq!(h.state.desktops.current, 1, "on its own desktop");
        assert_eq!(h.state.focused, Some(editor));
    }

    #[test]
    fn with_windows_turned_off_the_launcher_lists_none() {
        let mut h = crate::harness::Harness::with_config(crate::config::Config {
            base: repriseconf::config::Config {
                launcher_windows: false,
                ..Default::default()
            },
            ..Default::default()
        });
        let _c = two_windows(&mut h);
        h.state.toggle_launcher();
        typed(&mut h, "notes");
        let v = h.state.launcher.as_ref().unwrap();
        assert!(v.windows.is_empty());
        assert!(
            !v.model
                .list
                .items
                .iter()
                .any(|i| i == launcher::WINDOWS_HEADING)
        );
    }

    #[test]
    fn a_cold_calculator_icon_lands_on_its_row() {
        let root = crate::icon::svg_theme(&[CALC_ICON], &[]);
        let mut h = crate::icon::tests::two_outputs("classic", &root);
        h.state.toggle_launcher();
        typed(&mut h, "2+3");
        let calc = |h: &crate::harness::Harness| {
            let m = &h.state.launcher.as_ref().expect("the launcher").model;
            assert_eq!(m.pick(0), Some(Pick::Calc));
            m.icons[0].clone().expect("the calculator's icon")
        };
        assert!(
            calc(&h).pixels.iter().all(|&p| p == 0),
            "the stand-in is blank"
        );
        crate::icon::tests::land(&mut h);
        assert!(crate::icon::tests::red(&calc(&h)), "the row took the icon");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_launcher_asks_for_frames_only_on_its_output() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        h.pointer_to(500.0, 400.0);
        h.state.toggle_launcher();
        assert!(h.state.launcher.is_some());
        for step in ["redraw", "close"] {
            crate::backend::take_asks();
            match step {
                "redraw" => h.state.redraw_launcher(),
                _ => h.state.close_launcher(),
            }
            let asked = crate::backend::take_asks();
            assert!(!asked.is_empty(), "{step}: the launcher asks for a frame");
            assert!(
                asked.iter().all(|(o, _)| o == "TEST-1"),
                "{step}: only TEST-1, the launcher's output: {asked:?}"
            );
        }
    }
}
