//! Context menus: window, taskbar, desktop, icon, drop and Quick Launch menus.

use smithay::output::Output;

use crate::{
    compositor::State,
    config::Action,
    desktopview::DesktopCommand,
    keys,
    raster::Rect,
    startmenu::{ContextBitmaps, MenuItem, MenuLayout, MenuStyle, place_popup},
    window::WindowId,
    winmenu::{WindowMenuRow, WindowMenuState},
};

use super::*;

impl State {
    pub(super) fn context_icon_px(&self, output: &Output) -> i32 {
        MenuStyle::context(&self.config.theme, ContextBitmaps::Icons).icon_px()
            * self.draw_scale_at(Some(output)).max(1)
    }

    pub(crate) fn push_context_menu(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: MenuKind,
        from: MenuFrom,
    ) {
        self.push_context_menu_placed(at, items, kind, from, place_popup);
    }

    pub(crate) fn push_context_menu_placed(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: MenuKind,
        from: MenuFrom,
        place: impl FnOnce((i32, i32), (i32, i32), Rect) -> (i32, i32),
    ) {
        self.push_context_menu_looked(at, items, kind, from, place, false);
    }

    /// [`State::push_context_menu_placed`], with the Shut down options look when `options_look`.
    pub(crate) fn push_context_menu_looked(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: MenuKind,
        from: MenuFrom,
        place: impl FnOnce((i32, i32), (i32, i32), Rect) -> (i32, i32),
        options_look: bool,
    ) {
        let send_to = Some(DesktopCommand::SendTo);
        if let MenuKind::Desktop { commands, .. } | MenuKind::File { commands, .. } = &kind
            && commands.contains(&send_to)
        {
            self.restat_drives();
        }
        let Some(output) = self.output_at(at.0 as f64, at.1 as f64) else {
            return;
        };
        let usable = self.usable_area(Some(&output));
        let options = options_look.then(|| self.config.theme.for_options_menu());
        let theme = options.as_ref().unwrap_or(&self.config.theme);
        let style = MenuStyle::context_for(theme, &items).fit(
            &self.text,
            &items,
            self.config.theme.fonts.size_px,
            usable.width,
        );
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let pos = place(at, (layout.width, layout.height), usable);
        self.push_menu(0, pos, layout, items, output, kind, from);
        if let Some(m) = self.menus.last_mut() {
            m.options_look = options_look;
        }
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
        let from = if from_taskbar {
            MenuFrom::Bar
        } else {
            MenuFrom::Elsewhere
        };
        self.push_context_menu(at, items, MenuKind::Window { window: w, rows }, from);
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

    pub(super) fn window_menu_action(&mut self, w: WindowId, row: WindowMenuRow) {
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
        self.push_context_menu(at, items, MenuKind::Taskbar { commands }, MenuFrom::Bar);
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
        self.push_context_menu(at, items, kind, MenuFrom::Elsewhere);
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
        self.push_context_menu(at, items, kind, MenuFrom::Elsewhere);
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
        self.push_context_menu(at, items, kind, MenuFrom::Elsewhere);
    }

    pub fn open_quick_menu(&mut self, index: usize, at: (i32, i32)) {
        let Some(file) = self.quicklaunch.get(index).map(|q| q.file.clone()) else {
            return;
        };
        self.close_menu();
        let (items, commands) = crate::taskbar::quick_menu();
        self.push_context_menu(at, items, MenuKind::Quick { file, commands }, MenuFrom::Bar);
    }
}
