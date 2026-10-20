//! The taskbar's keyboard model: the items the keyboard walks in bar order, the
//! keys that move between and act on them, and the focus ring's rect.

use crate::raster::Rect;
use crate::taskbar::{TaskbarHit, TaskbarLayout};
use crate::textedit::keysym;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarKey {
    Prev,
    Next,
    PrevGroup,
    NextGroup,
    First,
    Last,
    Activate,
    Menu,
    Leave,
}

pub fn key(sym: u32, shift: bool) -> Option<BarKey> {
    use keysym::*;
    Some(match sym {
        LEFT | KP_LEFT | UP | KP_UP => BarKey::Prev,
        RIGHT | KP_RIGHT | DOWN | KP_DOWN => BarKey::Next,
        ISO_LEFT_TAB => BarKey::PrevGroup,
        TAB if shift => BarKey::PrevGroup,
        TAB => BarKey::NextGroup,
        HOME | KP_HOME => BarKey::First,
        END | KP_END => BarKey::Last,
        RETURN | KP_ENTER | SPACE => BarKey::Activate,
        MENU => BarKey::Menu,
        F10 if shift => BarKey::Menu,
        ESCAPE => BarKey::Leave,
        _ => return None,
    })
}

pub fn items(layout: &TaskbarLayout) -> Vec<TaskbarHit> {
    let mut v = vec![TaskbarHit::Start];
    let tools = (0..layout.pager.len())
        .map(TaskbarHit::Pager)
        .chain((0..layout.quick.len()).map(TaskbarHit::Quick))
        .chain(layout.quick_more.map(|_| TaskbarHit::QuickMore));
    let tasks = (0..layout.tasks.len()).map(TaskbarHit::Task);
    if layout.bands.tasks_first {
        v.extend(tasks);
        v.extend(tools);
    } else {
        v.extend(tools);
        v.extend(tasks);
    }
    v.extend(layout.more.map(|_| TaskbarHit::More));
    v.extend(layout.chevron.map(|_| TaskbarHit::Chevron));
    v.extend((0..layout.tray.len()).map(TaskbarHit::Tray));
    v.push(TaskbarHit::Clock);
    v.extend(layout.show_desktop.map(|_| TaskbarHit::ShowDesktop));
    v
}

fn group(h: TaskbarHit) -> u8 {
    match h {
        TaskbarHit::Start => 0,
        TaskbarHit::Pager(_) => 1,
        TaskbarHit::Quick(_) | TaskbarHit::QuickMore => 2,
        TaskbarHit::Task(_) | TaskbarHit::More => 3,
        TaskbarHit::Chevron | TaskbarHit::Tray(_) => 4,
        TaskbarHit::Clock => 5,
        TaskbarHit::ShowDesktop => 6,
        TaskbarHit::Grip(_) | TaskbarHit::Resize | TaskbarHit::Nothing => 7,
    }
}

pub fn step(items: &[TaskbarHit], at: TaskbarHit, key: BarKey) -> TaskbarHit {
    let Some(i) = items.iter().position(|&h| h == at) else {
        return items.first().copied().unwrap_or(at);
    };
    let n = items.len();
    match key {
        BarKey::Prev => items[(i + n - 1) % n],
        BarKey::Next => items[(i + 1) % n],
        BarKey::First => items[0],
        BarKey::Last => items[n - 1],
        BarKey::NextGroup => {
            let g = group(at);
            let after = (1..n).map(|k| items[(i + k) % n]);
            after.into_iter().find(|&h| group(h) != g).unwrap_or(at)
        }
        BarKey::PrevGroup => {
            let g = group(at);
            let Some(prev) = (1..n)
                .map(|k| items[(i + n - k) % n])
                .find(|&h| group(h) != g)
            else {
                return at;
            };
            let pg = group(prev);
            let mut j = items.iter().position(|&h| h == prev).unwrap_or(0);
            while group(items[(j + n - 1) % n]) == pg && (j + n - 1) % n != i {
                j = (j + n - 1) % n;
            }
            items[j]
        }
        BarKey::Activate | BarKey::Menu | BarKey::Leave => at,
    }
}

/// `at` kept on a bar whose items are now `items`: itself if present, else the
/// last item of its kind, else the item before it; None when empty.
pub fn fit(items: &[TaskbarHit], at: TaskbarHit) -> Option<TaskbarHit> {
    if items.contains(&at) {
        return Some(at);
    }
    let same = |h: &&TaskbarHit| std::mem::discriminant(*h) == std::mem::discriminant(&at);
    if let Some(&h) = items.iter().rfind(same) {
        return Some(h);
    }
    let g = group(at);
    items
        .iter()
        .rev()
        .find(|&&h| group(h) < g)
        .or(items.first())
        .copied()
}

pub fn tray_entry(items: &[TaskbarHit]) -> TaskbarHit {
    items
        .iter()
        .copied()
        .find(|h| matches!(h, TaskbarHit::Chevron | TaskbarHit::Tray(_)))
        .unwrap_or(TaskbarHit::Clock)
}

/// The rect of item `h` on `layout` at scale `s`, and its focus ring: 3 px
/// inside a button's bevel, on the edge of an icon cell.
pub fn focus_ring(layout: &TaskbarLayout, h: TaskbarHit, s: i32) -> Option<Rect> {
    let s = s.max(1);
    let get = |v: &[Rect], i: usize| v.get(i).copied();
    let (r, inset) = match h {
        TaskbarHit::Start => (Some(layout.start), 3),
        TaskbarHit::Task(i) => (get(&layout.tasks, i), 3),
        TaskbarHit::More => (layout.more, 3),
        TaskbarHit::Clock => (Some(layout.clock), 1),
        TaskbarHit::ShowDesktop => (layout.show_desktop, 1),
        TaskbarHit::Quick(i) => (get(&layout.quick, i), 0),
        TaskbarHit::QuickMore => (layout.quick_more, 0),
        TaskbarHit::Pager(i) => (get(&layout.pager, i), 0),
        TaskbarHit::Chevron => (layout.chevron, 0),
        TaskbarHit::Tray(i) => (get(&layout.tray, i), 0),
        TaskbarHit::Grip(_) | TaskbarHit::Resize | TaskbarHit::Nothing => (None, 0),
    };
    let r = r?;
    let inset = (inset * s).min((r.width.min(r.height) - 2 * s) / 2).max(0);
    Some(r.inset(inset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskbar::{Bands, TaskbarCounts, TaskbarShape};
    use crate::theme::Metrics;
    use TaskbarHit::*;

    fn bar(tasks_first: bool) -> TaskbarLayout {
        let c = TaskbarCounts {
            quick: 2,
            pager: 2,
            tasks: 2,
            tray: 2,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute_with(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            c,
            40,
            Bands {
                tasks_first,
                first_width: None,
            },
        )
    }

    #[test]
    fn the_keyboard_walks_the_bar_in_its_order() {
        assert_eq!(
            items(&bar(false)),
            [
                Start,
                Pager(0),
                Pager(1),
                Quick(0),
                Quick(1),
                Task(0),
                Task(1),
                Chevron,
                Tray(0),
                Tray(1),
                Clock,
                ShowDesktop
            ]
        );
        let swapped = items(&bar(true));
        assert_eq!(
            &swapped[1..7],
            [Task(0), Task(1), Pager(0), Pager(1), Quick(0), Quick(1)],
            "the task row first when it is the first band"
        );
    }

    #[test]
    fn the_arrows_wrap_and_tab_steps_by_group() {
        let v = items(&bar(false));
        assert_eq!(step(&v, Start, BarKey::Prev), ShowDesktop);
        assert_eq!(step(&v, ShowDesktop, BarKey::Next), Start);
        assert_eq!(step(&v, Pager(1), BarKey::Next), Quick(0));
        assert_eq!(step(&v, Pager(1), BarKey::NextGroup), Quick(0));
        assert_eq!(step(&v, Task(0), BarKey::NextGroup), Chevron);
        assert_eq!(step(&v, Tray(1), BarKey::PrevGroup), Task(0));
        assert_eq!(
            step(&v, Task(1), BarKey::PrevGroup),
            Quick(0),
            "the first item of the group before"
        );
        assert_eq!(step(&v, Start, BarKey::PrevGroup), ShowDesktop);
        assert_eq!(step(&v, Clock, BarKey::First), Start);
        assert_eq!(step(&v, Clock, BarKey::Last), ShowDesktop);
        assert_eq!(step(&v, Clock, BarKey::Activate), Clock);
    }

    #[test]
    fn keys_map_to_bar_keys() {
        use keysym::*;
        assert_eq!(key(LEFT, false), Some(BarKey::Prev));
        assert_eq!(key(DOWN, false), Some(BarKey::Next));
        assert_eq!(key(TAB, false), Some(BarKey::NextGroup));
        assert_eq!(key(TAB, true), Some(BarKey::PrevGroup));
        assert_eq!(key(ISO_LEFT_TAB, true), Some(BarKey::PrevGroup));
        assert_eq!(key(RETURN, false), Some(BarKey::Activate));
        assert_eq!(key(SPACE, false), Some(BarKey::Activate));
        assert_eq!(key(MENU, false), Some(BarKey::Menu));
        assert_eq!(key(F10, true), Some(BarKey::Menu));
        assert_eq!(key(F10, false), None, "F10 alone is not the menu key");
        assert_eq!(key(ESCAPE, false), Some(BarKey::Leave));
        assert_eq!(key(0x61, false), None);
    }

    #[test]
    fn a_focused_item_that_goes_falls_back_along_its_kind() {
        let v = items(&bar(false));
        assert_eq!(fit(&v, Task(1)), Some(Task(1)));
        assert_eq!(fit(&v, Task(5)), Some(Task(1)), "the last task left");
        assert_eq!(fit(&v, More), Some(Quick(1)), "no » on a bottom bar");
        let no_tasks: Vec<_> = v
            .iter()
            .copied()
            .filter(|h| !matches!(h, Task(_)))
            .collect();
        assert_eq!(fit(&no_tasks, Task(0)), Some(Quick(1)));
        assert_eq!(fit(&[], Start), None);
    }

    #[test]
    fn super_b_lands_on_the_chevron_then_the_tray_then_the_clock() {
        let v = items(&bar(false));
        assert_eq!(tray_entry(&v), Chevron);
        let no_chevron: Vec<_> = v.iter().copied().filter(|&h| h != Chevron).collect();
        assert_eq!(tray_entry(&no_chevron), Tray(0));
        assert_eq!(tray_entry(&[Start, Clock]), Clock);
    }

    #[test]
    fn a_ring_sits_inside_a_buttons_bevel_and_on_an_icon_cells_edge() {
        let l = bar(false);
        assert_eq!(focus_ring(&l, Start, 1), Some(l.start.inset(3)));
        assert_eq!(focus_ring(&l, Task(1), 1), Some(l.tasks[1].inset(3)));
        assert_eq!(focus_ring(&l, Tray(0), 1), Some(l.tray[0]));
        assert_eq!(focus_ring(&l, Quick(1), 1), Some(l.quick[1]));
        assert_eq!(focus_ring(&l, Task(9), 1), None);
        let l2 = l.scaled(2);
        assert_eq!(focus_ring(&l2, Start, 2), Some(l2.start.inset(6)));
    }
}
