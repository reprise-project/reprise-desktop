use reprisescene::TreeId;

use super::Bar;
use crate::{compositor::State, menuview::MenuRoot};

impl State {
    pub fn task_button_name(&self, bar: &Bar, i: usize) -> Option<(String, bool)> {
        if let Some(id) = bar.dialogs.get(i).copied().flatten() {
            let v = self.dialogs.views.iter().find(|v| v.id == id)?;
            return Some((v.dialog.title.clone(), self.active_dialog_id() == Some(id)));
        }
        let live = |w: &crate::window::Window| self.focused == Some(w.id) && !w.minimized;
        let wins: Vec<&crate::window::Window> = (bar.buttons.get(i)?.iter())
            .filter_map(|&w| self.win(w))
            .collect();
        match wins.as_slice() {
            [] => {
                let k = bar.pins.get(i).copied().flatten()?;
                self.quicklaunch
                    .get(k)
                    .map(|q| (q.entry.label.clone(), false))
            }
            [w] => Some((w.title(), live(w))),
            many => {
                let name = crate::taskbar::group_name(&many[0].app_id());
                let active = many.iter().any(|w| live(w));
                Some((format!("{name} ({})", many.len()), active))
            }
        }
    }

    fn tray_cell_name(&self, bar: TreeId, i: usize) -> String {
        let revealed = self.bar(bar).map_or(0, |b| b.layout.tray_hidden);
        let tray = self.tray.as_ref();
        let item = match crate::tray::cell_at(self, bar, i) {
            crate::tray::Cell::Hidden(_) => tray.and_then(|t| t.item_at(revealed, 0, i)),
            crate::tray::Cell::Item(k) => tray.and_then(|t| t.items.get(k)),
            crate::tray::Cell::Builtin(b) => {
                return (crate::tray::builtin::shown(self).get(b))
                    .and_then(|&b| crate::tray::builtin::label(self, b))
                    .unwrap_or_default();
            }
        };
        item.map(|it| {
            if it.tooltip.is_empty() {
                it.title.clone()
            } else {
                it.tooltip.clone()
            }
        })
        .unwrap_or_default()
    }

    pub(super) fn more_open(&self, tree: TreeId) -> bool {
        self.menu_root() == Some(MenuRoot::TaskOverflow(tree))
    }

    pub(super) fn quick_more_open(&self, tree: TreeId) -> bool {
        self.menu_root() == Some(MenuRoot::QuickOverflow(tree))
    }

    pub(super) fn chevron_open(&self, bar: &Bar) -> bool {
        bar.tray_open || crate::trayflyout::open_on(self).as_ref() == Some(&bar.output)
    }

    pub fn taskbar_access(&self, bar: &Bar) -> reprisekit::access::Tree {
        let l = &bar.layout;
        let names = crate::taskbar::TaskbarNames {
            start_open: bar.start_pressed,
            quick: (0..l.quick.len())
                .map(|i| (self.quicklaunch.get(i)).map_or(String::new(), |q| q.entry.label.clone()))
                .collect(),
            desktop: self.desktops.current,
            pager_attention: (self.pager_buttons().iter()).map(|p| p.attention).collect(),
            tasks: (0..l.tasks.len())
                .map(|i| self.task_button_name(bar, i).unwrap_or_default())
                .collect(),
            attention: (0..l.tasks.len())
                .map(|i| {
                    (bar.buttons.get(i).into_iter().flatten())
                        .any(|&w| self.win(w).is_some_and(|w| w.attention))
                })
                .collect(),
            focus: self.bar_focus(bar),
            tray: (0..l.tray.len())
                .map(|i| self.tray_cell_name(bar.tree, i))
                .collect(),
            hidden_open: self.chevron_open(bar),
            more_open: self.more_open(bar.tree),
            quick_more_open: self.quick_more_open(bar.tree),
            clock: crate::taskbar::clock_text(),
            date: crate::taskbar::date_text(),
        };
        let rect = self.bar_placement(&bar.output).shown;
        let mut tree =
            crate::access::root(reprisekit::access::accesskit::Role::Window, "Taskbar", rect);
        crate::taskbar::access(l, &names, &mut tree, reprisekit::access::ROOT);
        tree
    }
}
