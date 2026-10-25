//! Pushing, drawing, fading and closing menus on the stack, and their row icons.

use std::{sync::Arc, time::Instant};

use reprisekit::anim::{Unroll, UnrollFrom};
use reprisescene::{Input, NodeId, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::{
    compositor::{SceneOwner, State},
    config::{MenuAnimation, MenuEntry},
    output::upload_scaled,
    raster::{Canvas, Rect},
    startmenu::{MenuItem, MenuLayout, MenuStyle, draw_menu, draw_menu_shadow, shadow_reach},
    taskbar::{JumpRow, TaskbarCommand, WindowListRow},
    text::split_mnemonic,
    window::WindowId,
    winmenu::WindowMenuRow,
};

use super::*;

impl State {
    /// Icon `name` at `px` for menu row `row` ([`crate::icon::unread`]): an icon file not decoded
    /// yet shows the stand-in and goes on `waits` for [`State::menu_icon_landed`].
    pub(super) fn menu_icon(
        &mut self,
        row: usize,
        name: &str,
        px: i32,
        icons: &mut RowIcons,
    ) -> Option<Canvas> {
        icons.from.push((row, RowIcon::Name(name.to_string()), px));
        let icon = self.icons.get_unread(name, px).unwrap_or_else(|| {
            icons.waits.push((row, name.to_string(), px));
            self.icons.get_shared(crate::icon::STAND_IN, px)
        });
        icon.map(Arc::unwrap_or_clone)
    }

    /// Window `w`'s icon at `px` for menu row `row`: one waiting on its app's icon file shows the
    /// stand-in and goes on `waits` for [`State::menu_icon_landed`].
    pub(super) fn menu_window_icon(
        &mut self,
        row: usize,
        w: WindowId,
        px: i32,
        icons: &mut RowIcons,
    ) -> Option<Canvas> {
        let win = self.windows.iter().find(|x| x.id == w)?;
        icons.from.push((row, RowIcon::Window(w), px));
        let (icon, file) =
            crate::icon::window_icon_waiting(&mut self.icons, &self.app_index, win, px);
        if let Some(file) = file {
            icons.waits.push((row, file, px));
        }
        icon.map(Arc::unwrap_or_clone)
    }

    pub(crate) fn set_row_icons(&mut self, depth: usize, icons: RowIcons) {
        if let Some(m) = self.menus.get_mut(depth) {
            m.icons = icons;
        }
    }

    /// Looks every open menu row's icon up again, after the icon theme changed on disk. The
    /// caller redraws the menus.
    pub(crate) fn relook_menu_icons(&mut self) {
        for depth in 0..self.menus.len() {
            let from = std::mem::take(&mut self.menus[depth].icons.from);
            let mut icons = RowIcons::default();
            let mut looked = Vec::with_capacity(from.len());
            for (row, source, px) in from {
                let icon = match &source {
                    RowIcon::Name(name) => self.menu_icon(row, name, px, &mut icons),
                    RowIcon::Window(w) => {
                        if !self.windows.iter().any(|x| x.id == *w) {
                            icons.from.push((row, source, px));
                            continue;
                        }
                        self.menu_window_icon(row, *w, px, &mut icons)
                    }
                };
                looked.push((row, icon));
            }
            let m = &mut self.menus[depth];
            for (row, art) in looked {
                if let Some(MenuItem::Entry { icon, .. }) = m.items.get_mut(row) {
                    *icon = art;
                }
            }
            m.icons = icons;
        }
    }

    /// Icon file `file` decoded at `px`: the menu rows waiting on it take it.
    pub(crate) fn menu_icon_landed(&mut self, file: &str, px: i32, art: &Option<Canvas>) {
        for depth in 0..self.menus.len() {
            let m = &mut self.menus[depth];
            if land_icon(&mut m.items, &mut m.icons.waits, file, px, art) {
                self.redraw_menu(depth);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_menu(
        &mut self,
        depth: usize,
        (x, y): (i32, i32),
        layout: MenuLayout,
        items: Vec<MenuItem>,
        output: Output,
        kind: MenuKind,
        from: MenuFrom,
    ) {
        // A balloon's reply field sits on the overlay layer: its edit menu goes there too.
        let layer = match kind {
            MenuKind::Edit {
                editor: crate::caret::Editor::NotifyReply,
            } => self.layers.overlay,
            _ => self.layers.menu,
        };
        let scene = &mut self.scene;
        let tree = scene.tree_create(layer);
        scene.set_data(tree, Some(SceneOwner::Menu(depth)));
        crate::glass::leave_out(scene, tree);
        let shadow = scene.buffer_create(tree);
        scene.set_input(shadow, Input::Passthrough);
        let buffer = scene.buffer_create(tree);
        scene.set_position(tree, x, y);
        let rect = Rect::new(x, y, layout.width, layout.height);
        let opening = match self.config.menu_opening() {
            MenuAnimation::None => None,
            MenuAnimation::Fade => Some(Opening::Fade),
            MenuAnimation::Slide => Some(Opening::Unroll(match self.menus.last() {
                Some(parent) if depth > 0 => UnrollFrom::beside(
                    rect,
                    Rect::new(
                        parent.x,
                        parent.y,
                        parent.layout.width,
                        parent.layout.height,
                    ),
                ),
                _ if from == MenuFrom::Bar => unroll_from_bar(self.bar_placement(&output).edge),
                _ => {
                    let p = self.pointer_location();
                    UnrollFrom::facing(rect, (p.x as i32, p.y as i32))
                }
            })),
        };
        if opening == Some(Opening::Fade) {
            self.scene.set_opacity(tree, 0.0);
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
            from,
            parent_row: None,
            options_look: false,
            opened: Instant::now(),
            opening,
            shift: (0, 0),
            icons: RowIcons::default(),
        });
        if opening.is_some() {
            self.open_menu_frame(self.menus.len() - 1, 0.0);
            self.arm_menu_fade();
        }
        crate::autohideview::poke(self);
    }

    pub(crate) fn redraw_menu(&mut self, depth: usize) {
        let Some(m) = self.menus.get(depth) else {
            return;
        };
        let tree = m.tree;
        let was = self.scene.outputs_of(tree);
        let scale = self.draw_scale_at(Some(&m.output));
        let theme = self.config.theme.scaled(scale);
        let theme = if m.options_look {
            theme.for_options_menu()
        } else {
            theme
        };
        let layout = m.layout.scaled(scale);
        let mut canvas = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
        draw_menu(
            &mut canvas,
            &theme,
            self.text.for_output(scale),
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
        self.redraw_nodes(was, &[tree.into()]);
        crate::access::touch(self);
    }

    pub(super) fn entry_items(
        &mut self,
        entries: &[MenuEntry],
        px: i32,
    ) -> (Vec<MenuItem>, RowIcons) {
        let user = crate::text::escape_mnemonic(&crate::lock::current_user());
        let mut waits = RowIcons::default();
        let items = (entries.iter().enumerate())
            .map(|(row, e)| match e {
                MenuEntry::Separator => MenuItem::Separator,
                MenuEntry::Item { label, icon, .. } => {
                    let (label, mnemonic) =
                        split_mnemonic(&crate::startmenu::expand_label(label, &user));
                    MenuItem::entry(label)
                        .icon(
                            icon.as_deref()
                                .and_then(|n| self.menu_icon(row, n, px, &mut waits)),
                        )
                        .submenu(e.opens_submenu())
                        .mnemonic(mnemonic)
                }
            })
            .collect();
        (items, waits)
    }

    pub(super) fn submenu_open_for(&self, depth: usize, row: usize) -> bool {
        self.menus
            .get(depth + 1)
            .is_some_and(|m| m.parent_row == Some(row))
    }

    pub(super) fn is_tray_menu(&self, depth: usize) -> bool {
        matches!(
            self.menus.get(depth).map(|m| &m.kind),
            Some(MenuKind::Tray { .. })
        )
    }

    pub(super) fn row_opens_submenu(&self, depth: usize, idx: usize) -> bool {
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
            MenuKind::QuickOverflow { bar, .. } => MenuRoot::QuickOverflow(*bar),
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
        let (items, waits) = self.entry_items(&entries, px);
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
        self.push_menu(
            0,
            at,
            layout,
            items,
            output,
            MenuKind::Entries(entries),
            MenuFrom::Bar,
        );
        if let Some(m) = self.menus.first_mut() {
            m.shadow_floor = start_menu_shadow_floor(dock, &p);
        }
        self.set_row_icons(0, waits);
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

    pub(crate) fn close_menus_above(&mut self, depth: usize) {
        self.schedule_menu_hover(None);
        crate::tray::menus_closed_above(self, depth);
        crate::tooltip::menus_closed_above(self, depth);
        if self.menus.len() > depth + 1 {
            crate::recentwatch::lists_closed(self);
        }
        while self.menus.len() > depth + 1 {
            if let Some(m) = self.menus.pop() {
                self.destroy_redrawn(m.tree);
            }
        }
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

    pub(super) fn close_topmost_menu(&mut self) {
        self.schedule_menu_hover(None);
        let Some(m) = self.menus.pop() else {
            return;
        };
        self.destroy_redrawn(m.tree);
        if self.menus.is_empty() {
            crate::tray::menus_closed(self);
            crate::tooltip::menus_closed(self);
            self.menus_emptied();
        } else {
            crate::recentwatch::lists_closed(self);
            crate::tray::menus_closed_above(self, self.menus.len() - 1);
            crate::tooltip::menus_closed_above(self, self.menus.len() - 1);
            crate::access::touch(self);
        }
    }

    fn menus_emptied(&mut self) {
        crate::recentwatch::lists_closed(self);
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

    /// Shows menu `idx` at `t` (0 to 1) of its opening; at 1 the opening ends.
    fn open_menu_frame(&mut self, idx: usize, t: f32) {
        let reach = shadow_reach(&self.config.theme);
        let Some(m) = self.menus.get_mut(idx) else {
            return;
        };
        let done = t >= 1.0;
        match m.opening {
            None => {}
            Some(Opening::Fade) => self.scene.set_opacity(m.tree, t),
            Some(Opening::Unroll(from)) => {
                let size = (m.layout.width + reach, m.layout.height + reach);
                let Unroll { shift, clip } = Unroll::at(from, size, t);
                m.shift = if done { (0, 0) } else { shift };
                self.scene
                    .set_position(m.tree, m.x + m.shift.0, m.y + m.shift.1);
                self.scene.set_clip(
                    m.tree,
                    (!done).then(|| {
                        smithay::utils::Rectangle::new(
                            (clip.x, clip.y).into(),
                            (clip.width, clip.height).into(),
                        )
                    }),
                );
            }
        }
        if done {
            m.opening = None;
        }
    }

    fn menu_fade_tick(&mut self) -> TimeoutAction {
        let ms = self.config.theme.metrics.menu_fade_ms.max(1) as f32;
        let mut done = true;
        let trees: Vec<NodeId> = self.menus.iter().map(|m| m.tree.into()).collect();
        let was = self.outputs_of_nodes(&trees);
        for idx in 0..self.menus.len() {
            if self.menus[idx].opening.is_none() {
                continue;
            }
            let t = (self.menus[idx].opened.elapsed().as_secs_f32() * 1000.0 / ms).min(1.0);
            self.open_menu_frame(idx, t);
            done &= t >= 1.0;
        }
        self.redraw_nodes(was, &trees);
        if done {
            self.menu_fade_timer = None;
            TimeoutAction::Drop
        } else {
            TimeoutAction::ToDuration(FADE_TICK)
        }
    }
}
