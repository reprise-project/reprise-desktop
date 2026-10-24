use super::*;
use crate::access::accesskit::{HasPopup, NodeId, Orientation, Role};
use crate::access::{self, Tree};
use crate::raster::Rect;

/// What the taskbar's `access` part says, parallel to the layout's rects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskbarNames {
    pub start_open: bool,
    pub quick: Vec<String>,
    pub desktop: usize,
    pub pager_attention: Vec<bool>,
    pub tasks: Vec<(String, bool)>,
    pub attention: Vec<bool>,
    pub tray: Vec<String>,
    /// The flyout or a tray arrow's run is open: the ^ cell reads expanded (a tray arrow is then named Hide).
    pub hidden_open: bool,
    pub more_open: bool,
    pub quick_more_open: bool,
    pub clock: String,
    pub date: String,
    pub focus: Option<TaskbarHit>,
}

pub const ATTENTION: &str = "Wants attention";
/// Name and tip of the Quick Launch band's » cell.
pub const QUICK_MORE_NAME: &str = "Show more Quick Launch buttons";

/// The taskbar's `access` part, in taskbar coordinates; titles and tips are taken as they are (an `&` is text).
pub fn access(layout: &TaskbarLayout, names: &TaskbarNames, tree: &mut Tree, parent: NodeId) {
    let button = |r: Rect, name: &str| {
        let mut n = access::node(Role::Button, r);
        if !name.is_empty() {
            n.set_label(name);
        }
        access::focusable(&mut n, true);
        n
    };
    let mut focus = None;
    let mut mark = |hit: TaskbarHit, id: NodeId| {
        if names.focus == Some(hit) {
            focus = Some(id);
        }
    };
    let start_rect = layout
        .disc_past_bar()
        .map_or(layout.start, |d| layout.start.union(&d));
    let mut start = button(start_rect, "Start");
    start.set_has_popup(HasPopup::Menu);
    start.set_expanded(names.start_open);
    mark(TaskbarHit::Start, tree.push(parent, "start", start));
    let vertical = layout.edge.is_side();
    let bar = |tree: &mut Tree, key: &str, rects: &[Rect], name: &str| {
        let all = rects.iter().skip(1).fold(rects[0], |u, r| u.union(r));
        let mut n = access::named(Role::Toolbar, all, name);
        if vertical {
            n.set_orientation(Orientation::Vertical);
        }
        tree.push(parent, key, n)
    };
    let wants = |n: &mut crate::access::accesskit::Node, on: Option<&bool>| {
        if on == Some(&true) {
            n.set_description(ATTENTION);
        }
    };
    let quick_cells: Vec<Rect> = layout
        .quick
        .iter()
        .copied()
        .chain(layout.quick_more)
        .collect();
    // The bands read in the order they are drawn: the pager, then Quick Launch, with the task
    // row before both when it is the first band.
    let tools = |tree: &mut Tree, mark: &mut dyn FnMut(TaskbarHit, NodeId)| {
        if !layout.pager.is_empty() {
            let id = bar(tree, "pager", &layout.pager, "Desktops");
            for (i, r) in layout.pager.iter().enumerate() {
                let mut n = button(*r, &format!("Desktop {}", i + 1));
                n.set_toggled((i == names.desktop).into());
                wants(&mut n, names.pager_attention.get(i));
                mark(TaskbarHit::Pager(i), tree.push(id, i, n));
            }
        }
        if !quick_cells.is_empty() {
            let id = bar(tree, "quick", &quick_cells, "Quick Launch");
            for (i, (r, name)) in layout.quick.iter().zip(&names.quick).enumerate() {
                mark(TaskbarHit::Quick(i), tree.push(id, i, button(*r, name)));
            }
            if let Some(r) = layout.quick_more {
                let mut n = button(r, QUICK_MORE_NAME);
                n.set_has_popup(HasPopup::Menu);
                n.set_expanded(names.quick_more_open);
                mark(TaskbarHit::QuickMore, tree.push(id, "more", n));
            }
        }
    };
    let tasks = |tree: &mut Tree, mark: &mut dyn FnMut(TaskbarHit, NodeId)| {
        if !layout.tasks.is_empty() {
            let id = bar(tree, "tasks", &layout.tasks, "Running Applications");
            for (i, (r, (title, active))) in layout.tasks.iter().zip(&names.tasks).enumerate() {
                let mut n = button(*r, title);
                n.set_toggled((*active).into());
                wants(&mut n, names.attention.get(i));
                mark(TaskbarHit::Task(i), tree.push(id, i, n));
            }
        }
    };
    if layout.bands.tasks_first {
        tasks(tree, &mut mark);
        tools(tree, &mut mark);
    } else {
        tools(tree, &mut mark);
        tasks(tree, &mut mark);
    }
    if let Some(r) = layout.more {
        let mut n = button(r, "Show more windows");
        n.set_has_popup(HasPopup::Menu);
        n.set_expanded(names.more_open);
        mark(TaskbarHit::More, tree.push(parent, "more", n));
    }
    if let Some(r) = layout.chevron {
        // A tray arrow opens the run in place: no popup, and named for
        // what pressing it does.
        let mut n = if !layout.tray_arrow {
            let mut n = button(r, "Show hidden icons");
            n.set_has_popup(HasPopup::Dialog);
            n
        } else if names.hidden_open {
            button(r, "Hide")
        } else {
            button(r, "Show hidden icons")
        };
        n.set_expanded(names.hidden_open);
        mark(TaskbarHit::Chevron, tree.push(parent, "chevron", n));
    }
    if !layout.tray.is_empty() {
        let id = bar(tree, "tray", &layout.tray, "Notification Area");
        for (i, (r, name)) in layout.tray.iter().zip(&names.tray).enumerate() {
            mark(TaskbarHit::Tray(i), tree.push(id, i, button(*r, name)));
        }
    }
    let mut clock = button(layout.clock, &names.clock);
    if !names.date.is_empty() {
        clock.set_description(names.date.as_str());
    }
    mark(TaskbarHit::Clock, tree.push(parent, "clock", clock));
    if let Some(r) = layout.show_desktop {
        let id = tree.push(parent, "show_desktop", button(r, "Show desktop"));
        mark(TaskbarHit::ShowDesktop, id);
    }
    if let Some(id) = focus {
        tree.set_focus(id);
    }
}
