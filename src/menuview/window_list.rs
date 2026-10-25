//! Window lists and the task and Quick Launch overflow menus.

use reprisescene::TreeId;
use smithay::output::Output;

use crate::{
    compositor::State,
    raster::Rect,
    startmenu::{ContextBitmaps, MenuItem, MenuStyle, place_popup},
    taskbar::WindowListRow,
    window::WindowId,
};

use super::*;

impl State {
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
        let px = self.context_icon_px(&output);
        let titles: Vec<String> = wins
            .iter()
            .map(|&w| self.win(w).map(|x| x.title()).unwrap_or_default())
            .collect();
        let (items, rows) = crate::taskbar::window_list(&titles, self.desktops.count);
        let mut waits = RowIcons::default();
        let items: Vec<MenuItem> = (items.into_iter().zip(&rows).enumerate())
            .map(|(n, (item, row))| match row {
                Some(WindowListRow::Window(k)) => {
                    let icon = self.menu_window_icon(n, wins[*k], px, &mut waits);
                    item.icon(icon)
                }
                _ => item,
            })
            .collect();
        let usable = self.usable_area(Some(&output));
        let layout = style.layout_in(&self.config.theme, &self.text, &items, usable);
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
        self.push_menu(0, pos, layout, items, output, kind, MenuFrom::Bar);
        self.set_row_icons(0, waits);
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
        let px = self.context_icon_px(&output);
        let mut waits = RowIcons::default();
        let items: Vec<MenuItem> = (rows.into_iter().enumerate())
            .map(|(n, (title, window))| {
                let icon = window.and_then(|w| self.menu_window_icon(n, w, px, &mut waits));
                MenuItem::entry(title).icon(icon)
            })
            .collect();
        let kind = MenuKind::TaskOverflow { bar: tree, keys };
        self.push_overflow(output, more, items, kind, waits);
    }

    /// The menu off the Quick Launch band's » cell: the launchers the band hides, each with its
    /// icon and name.
    pub fn open_quick_overflow(&mut self, tree: TreeId) {
        let Some(b) = self.bar(tree) else { return };
        let Some(more) = b.layout.quick_more else {
            return;
        };
        let output = b.output.clone();
        let hidden: Vec<(String, String, Option<String>)> = (self.quicklaunch)
            .iter()
            .skip(b.layout.quick.len())
            .map(|q| (q.file.clone(), q.entry.label.clone(), q.entry.icon.clone()))
            .collect();
        if hidden.is_empty() {
            return;
        }
        self.close_menu();
        let px = self.context_icon_px(&output);
        let mut waits = RowIcons::default();
        let mut files = Vec::with_capacity(hidden.len());
        let mut items = Vec::with_capacity(hidden.len());
        for (n, (file, label, icon)) in hidden.into_iter().enumerate() {
            let icon = icon.and_then(|name| self.menu_icon(n, &name, px, &mut waits));
            items.push(MenuItem::entry(label).icon(icon));
            files.push(file);
        }
        let kind = MenuKind::QuickOverflow { bar: tree, files };
        self.push_overflow(output, more, items, kind, waits);
    }

    /// Opens `items` as a menu off the bar cell `cell` (bar coordinates) on `output`.
    fn push_overflow(
        &mut self,
        output: Output,
        cell: Rect,
        items: Vec<MenuItem>,
        kind: MenuKind,
        waits: RowIcons,
    ) {
        let style = MenuStyle::context(&self.config.theme, ContextBitmaps::Icons);
        let usable = self.usable_area(Some(&output));
        let layout = style.layout_in(&self.config.theme, &self.text, &items, usable);
        let p = self.bar_placement(&output);
        let cell = Rect::new(
            p.shown.x + cell.x,
            p.shown.y + cell.y,
            cell.width,
            cell.height,
        );
        let pos = place_popup(p.anchor_point(cell), (layout.width, layout.height), usable);
        self.push_menu(0, pos, layout, items, output, kind, MenuFrom::Bar);
        self.set_row_icons(0, waits);
        self.redraw_menu(0);
        self.redraw_taskbars();
    }

    pub(super) fn window_list_activate(&mut self, windows: &[WindowId], row: WindowListRow) {
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
}
