use std::path::{Path, PathBuf};
use std::time::Duration;

use reprisekit::startpanel::layout;
use reprisekit::startpanel::{Button, Event, Key, Mode, Spot, StartPanel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::config::{Action, PowerAction};
use crate::dialogview::DialogKey;
use crate::input::{BTN_LEFT, BTN_RIGHT};
use crate::keys;
use crate::raster::Rect;

use super::lists::jump_file;
use super::search::ResultKind;

const WHEEL_ROWS: i64 = 3;
const ACCOUNTS: &str = "reprise-accounts";

pub(super) fn key_of(k: &DialogKey) -> Option<Key> {
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

pub(super) fn spot_app(
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

impl State {
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
        let menu = key == Key::Menu;
        let mut row_menu = false;
        self.start_panel_event_with(enters, |m| {
            let event = m.key(key);
            row_menu = event != Event::None;
            event
        });
        // Without a row's menu to open, the menu key opens the search field's.
        if menu && !row_menu {
            self.open_edit_menu(None);
        }
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

    /// A reader moved the keyboard to `spot` (`StartPanel::focus_on`).
    pub fn start_panel_focus(&mut self, spot: Spot) {
        self.start_panel_event(|m| m.focus_on(spot));
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
        if (b, spot) == (Button::Right, Spot::Search) {
            self.start_panel_event(|m| m.focus_on(Spot::Search));
            let p = self.pointer_location();
            self.open_edit_menu(Some((p.x as i32, p.y as i32)));
            return;
        }
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
        let (x, y) = (
            x.floor() as i32 - v.at.0 - v.shift.0,
            y.floor() as i32 - v.at.1 - v.shift.1,
        );
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

    pub(super) fn start_spot_app(&self, s: Spot) -> Option<(usize, bool)> {
        let v = self.start_panel.as_ref()?;
        spot_app(
            s,
            &v.programs,
            (v.model.pinned.len(), v.launched),
            &v.tree_apps,
            (&v.result_apps, &v.result_kinds),
        )
    }

    pub(super) fn start_spot_rect(&self, s: Spot) -> Option<Rect> {
        let v = self.start_panel.as_ref()?;
        let r = layout::layout(&v.model).rect(s)?;
        Some(Rect::new(r.x + v.at.0, r.y + v.at.1, r.width, r.height))
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
}
