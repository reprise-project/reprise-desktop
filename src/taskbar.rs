//! The taskbar's menus, button grouping and clock text. Model, layout, hit test and access live in
//! `reprisekit::taskbar`; paint is `reprisekit::look::draw_taskbar`.

pub use reprisekit::taskbar::{
    Band, Bands, PagerButton, PopupCell, TaskEntry, TaskRow, TaskbarCounts, TaskbarHit,
    TaskbarLayout, TaskbarNames, TaskbarShape, TaskbarState, access, quick_icon_origin, slot_at,
};

use crate::config::TaskbarButtons;
use crate::quicklaunch::QuickItem;
use crate::startmenu::MenuItem;
use crate::text::split_mnemonic;
use reprisekit::raster::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarCommand {
    Toolbars,
    QuickLaunch,
    Cascade,
    TileHorizontal,
    TileVertical,
    ShowDesktop,
    TaskManager,
    LockTaskbar,
    Properties,
}

/// The taskbar's right-click menu rows and, parallel to them, what each does (None at a separator).
pub fn taskbar_menu(
    locked: bool,
    has_windows: bool,
) -> (Vec<MenuItem>, Vec<Option<TaskbarCommand>>) {
    use TaskbarCommand::*;
    let rows = [
        Some(("&Toolbars", false, true, true, Toolbars)),
        None,
        Some(("Casca&de Windows", false, has_windows, false, Cascade)),
        Some((
            "Tile Windows &Horizontally",
            false,
            has_windows,
            false,
            TileHorizontal,
        )),
        Some((
            "Tile Windows &Vertically",
            false,
            has_windows,
            false,
            TileVertical,
        )),
        Some(("&Show the Desktop", false, true, false, ShowDesktop)),
        None,
        Some(("Tas&k Manager", false, true, false, TaskManager)),
        None,
        Some(("&Lock the Taskbar", locked, true, false, LockTaskbar)),
        Some(("P&roperties", false, true, false, Properties)),
    ];
    let items = rows
        .iter()
        .map(|r| match r {
            Some((label, checked, enabled, submenu, _)) => {
                menu_row(label, *checked, *enabled, *submenu)
            }
            None => MenuItem::Separator,
        })
        .collect();
    let commands = rows.iter().map(|r| r.map(|(.., cmd)| cmd)).collect();
    (items, commands)
}

/// The Toolbars submenu: Quick Launch, greyed while `pinned` puts its programs on the task row.
pub fn toolbars_menu(
    quick_shown: bool,
    pinned: bool,
) -> (Vec<MenuItem>, Vec<Option<TaskbarCommand>>) {
    (
        vec![menu_row("&Quick Launch", quick_shown, !pinned, false)],
        vec![Some(TaskbarCommand::QuickLaunch)],
    )
}

fn menu_row(label: &str, checked: bool, enabled: bool, submenu: bool) -> MenuItem {
    let (label, mnemonic) = split_mnemonic(label);
    MenuItem::entry(label)
        .enabled(enabled)
        .checked(checked)
        .submenu(submenu)
        .mnemonic(mnemonic)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickCommand {
    Open,
    Delete,
}

pub fn quick_menu() -> (Vec<MenuItem>, Vec<Option<QuickCommand>>) {
    let entry = |label: &str, bold: bool| {
        let (label, mnemonic) = split_mnemonic(label);
        MenuItem::entry(label).mnemonic(mnemonic).bold(bold)
    };
    (
        vec![
            entry("&Open", true),
            MenuItem::Separator,
            entry("&Delete", false),
        ],
        vec![Some(QuickCommand::Open), None, Some(QuickCommand::Delete)],
    )
}

/// Move Quick Launch button `from` into slot `slot` (a gap index 0..=len, as `TaskbarLayout::drop_slot`
/// counts). Returns whether the order changed.
pub fn move_button<T>(v: &mut Vec<T>, from: usize, slot: usize) -> bool {
    if from >= v.len() || slot > v.len() || slot == from || slot == from + 1 {
        return false;
    }
    let item = v.remove(from);
    let to = if slot > from { slot - 1 } else { slot };
    v.insert(to, item);
    true
}

/// Per-button width (logical px) below which the bar counts as full and same-app windows group.
pub const GROUP_BELOW: i32 = 62;

/// Group windows into taskbar buttons. `apps[i]` is window `i`'s group key (its app id). `mode`:
/// - `WhenFull`: every window is its own button if each would get at least `single_min` px in `avail`
///   (`gap` px apart); otherwise apps with two or more windows collapse into one button.
/// - `Combine`: every app's windows share one button.
/// - `Never`, `Group`: every window is its own button.
///
/// A group takes its first window's place. Returns the window indices behind each button.
pub fn plan_buttons(
    apps: &[String],
    avail: i32,
    single_min: i32,
    gap: i32,
    mode: TaskbarButtons,
) -> Vec<Vec<usize>> {
    let n = apps.len();
    if n == 0 {
        return Vec::new();
    }
    let alone = || (0..n).map(|i| vec![i]).collect();
    match mode {
        TaskbarButtons::Never | TaskbarButtons::Group => return alone(),
        TaskbarButtons::WhenFull => {
            let per = (avail - gap * (n as i32 - 1)) / n as i32;
            if per >= single_min {
                return alone();
            }
        }
        TaskbarButtons::Combine => {}
    }
    let mut groups: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, a) in apps.iter().enumerate() {
        groups.entry(a.as_str()).or_default().push(i);
    }
    let mut emitted: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut buttons = Vec::new();
    for (i, a) in apps.iter().enumerate() {
        let g = &groups[a.as_str()];
        if g.len() >= 2 {
            if emitted.insert(a.as_str()) {
                buttons.push(g.clone());
            }
        } else {
            buttons.push(vec![i]);
        }
    }
    buttons
}

/// A dialog with a task button of its own, as [`lay_slots`] places it: its id and [`Place`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogTask {
    pub id: u64,
    pub place: Place,
}

/// Where a task button sits in the row: after `anchor` pinned slots (all when past the count), then by
/// `seq` among the same anchor. A new one takes [`Place::end`]; a drag reassigns places. Session only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub anchor: usize,
    pub seq: u64,
}

impl Place {
    pub const fn end(seq: u64) -> Place {
        Place {
            anchor: usize::MAX,
            seq,
        }
    }
}

/// The next creation sequence number; windows and dialogs take one when made and the task row orders by it.
pub fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A slot of the task row: the windows behind its button (none for an idle pin or a dialog), its pinned
/// item (index into the Quick Launch list) and its dialog (by id).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub windows: Vec<usize>,
    pub pin: Option<usize>,
    pub dialog: Option<u64>,
}

/// The pinned item among `pins` that a window of `app_id` belongs to. Each rule is tried against every
/// pin before the next, case-insensitively:
/// 1. `desktop_id`, the window's `.desktop` file id, is the pin's file name;
/// 2. the pin's `StartupWMClass` is the app id;
/// 3. the pin's file name without `.desktop` is the app id.
pub fn pin_for(pins: &[QuickItem], app_id: &str, desktop_id: Option<&str>) -> Option<usize> {
    if app_id.is_empty() {
        return None;
    }
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let by_file = || {
        let id = desktop_id?;
        pins.iter().position(|p| same(&p.file, id))
    };
    let by_class = || {
        pins.iter().position(|p| {
            p.app
                .as_ref()
                .and_then(|a| a.startup_wm_class.as_deref())
                .is_some_and(|c| same(c, app_id))
        })
    };
    let by_stem = || {
        pins.iter().position(|p| {
            let stem = p.file.strip_suffix(".desktop").unwrap_or(&p.file);
            same(stem, app_id)
        })
    };
    by_file().or_else(by_class).or_else(by_stem)
}

/// The places `group` mode orders the windows keyed `apps` by: each takes its app's earliest place, so
/// an app's buttons sit together. A window with no app id keeps its own.
pub fn app_places(apps: &[String], places: &[Place]) -> Vec<Place> {
    let mut first: std::collections::HashMap<&str, Place> = std::collections::HashMap::new();
    for (a, &p) in apps.iter().zip(places) {
        if !a.is_empty() {
            let f = first.entry(a.as_str()).or_insert(p);
            if p.seq < f.seq {
                *f = p;
            }
        }
    }
    apps.iter()
        .zip(places)
        .map(|(a, &p)| first.get(a.as_str()).copied().unwrap_or(p))
        .collect()
}

/// Merge the planned buttons (`plan`, window indices per button) and `dialogs` with `pins` into the
/// task row. A pin's slot holds the buttons whose windows belong to it (`pin_of[w]`), or one empty
/// slot. Every other button and dialog goes by its place (`places[w]`; a group by its earliest window):
/// after `anchor` pins, then by `seq`.
pub fn merge_row(
    plan: Vec<Vec<usize>>,
    pins: usize,
    pin_of: &[Option<usize>],
    places: &[Place],
    dialogs: &[DialogTask],
) -> Vec<Slot> {
    let pin_of_button = |b: &[usize]| b.first().and_then(|&w| pin_of.get(w).copied().flatten());
    let place_of = |b: &[usize]| {
        b.iter()
            .filter_map(|&w| places.get(w).copied())
            .min_by_key(|p| p.seq)
            .unwrap_or(Place::end(u64::MAX))
    };
    let mut by_pin: Vec<Vec<Vec<usize>>> = vec![Vec::new(); pins];
    let mut rest: Vec<(Place, Slot)> = Vec::new();
    for b in plan {
        match pin_of_button(&b).filter(|&k| k < pins) {
            Some(k) => by_pin[k].push(b),
            None => rest.push((
                place_of(&b),
                Slot {
                    windows: b,
                    ..Slot::default()
                },
            )),
        }
    }
    rest.extend(dialogs.iter().map(|d| {
        (
            d.place,
            Slot {
                dialog: Some(d.id),
                ..Slot::default()
            },
        )
    }));
    rest.sort_by_key(|(p, _)| (p.anchor.min(pins), p.seq));
    let mut rest = rest.into_iter().peekable();
    let mut slots = Vec::new();
    for (k, buttons) in by_pin.into_iter().enumerate() {
        while let Some((_, s)) = rest.next_if(|(p, _)| p.anchor.min(pins) <= k) {
            slots.push(s);
        }
        if buttons.is_empty() {
            slots.push(Slot {
                pin: Some(k),
                ..Slot::default()
            });
        }
        slots.extend(buttons.into_iter().map(|windows| Slot {
            windows,
            pin: Some(k),
            dialog: None,
        }));
    }
    slots.extend(rest.map(|(_, s)| s));
    slots
}

/// Group the windows keyed `apps` into buttons as `mode` asks and lay them into `layout`'s task row
/// with `pins` and a button per `dialogs` id; `layout` was computed in `m` and `shape`. Windows are
/// grouped (`plan_buttons`) in the room the idle pins and dialogs leave, then merged (`merge_row`);
/// `pin_of[w]` is window `w`'s pin and `places[w]` its place. `never` stops shrinking at `GROUP_BELOW`
/// and leaves the rest out (`group` too, ordering by [`app_places`]); `combine` lays icon-only buttons,
/// an idle pin being an icon slot (stack of 0). A dialog's button never groups. Returns the slots in row order.
#[allow(clippy::too_many_arguments)]
pub fn lay_slots(
    layout: &mut TaskbarLayout,
    m: &reprisekit::theme::Metrics,
    shape: &TaskbarShape,
    apps: &[String],
    places: &[Place],
    pin_of: &[Option<usize>],
    pins: usize,
    dialogs: &[DialogTask],
    mode: TaskbarButtons,
) -> Vec<Slot> {
    let running: std::collections::HashSet<usize> = pin_of.iter().flatten().copied().collect();
    let idle = (0..pins).filter(|k| !running.contains(k)).count() as i32;
    let (_, icon_pitch) = layout.icon_step(shape);
    // Each dialog's button takes a least button and its gap off the room, so windows group just when the
    // row would go under the least a button takes: `GROUP_BELOW` wide along the bottom or top, a full
    // button's height up a side (an icon-only column uses the icon buttons' gap).
    let gap = if layout.edge.is_side() {
        layout.column_gap(shape, task_row(mode))
    } else {
        shape.labelled.gap
    };
    let least = layout.least_button(GROUP_BELOW);
    let avail = layout.tasks_limit()
        - layout.tasks_x()
        - idle * icon_pitch
        - dialogs.len() as i32 * (least + gap);
    let mut plan = plan_buttons(apps, avail, least, gap, mode);
    let grouped;
    let places = if mode == TaskbarButtons::Group {
        plan.sort_by_key(|b| places.get(b[0]).map_or(u64::MAX, |p| p.seq));
        grouped = app_places(apps, places);
        &grouped
    } else {
        places
    };
    let slots = merge_row(plan, pins, pin_of, places, dialogs);
    let stacks: Vec<u8> = slots
        .iter()
        .map(|s| match s.dialog {
            Some(_) => 1,
            None => s.windows.len().min(u8::MAX as usize) as u8,
        })
        .collect();
    layout.lay_tasks(m, shape, &stacks, task_row(mode));
    slots
}

/// A dialog's task button: title by the fallback glyph (none in an `icons_only` row), pushed in while `active`.
pub fn dialog_entry(title: &str, active: bool) -> TaskEntry {
    TaskEntry {
        title: title.to_string(),
        active,
        attention: false,
        icon: None,
        glow: None,
    }
}

/// The units a drag moves along the task row, as button index ranges: a pinned app's buttons, an
/// unpinned run with the same `runs` key (`group` mode's app ids), or a lone button. `runs` is parallel
/// to `pins`, or shorter where no button has a key.
pub fn row_units(pins: &[Option<usize>], runs: &[Option<String>]) -> Vec<std::ops::Range<usize>> {
    let run = |i: usize| runs.get(i).and_then(Option::as_ref);
    let mut units: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, p) in pins.iter().enumerate() {
        match units.last_mut() {
            Some(u) if p.is_some() && pins[u.start] == *p => u.end = i + 1,
            Some(u)
                if p.is_none()
                    && pins[u.start].is_none()
                    && run(i).is_some_and(|r| run(u.start) == Some(r)) =>
            {
                u.end = i + 1
            }
            _ => units.push(i..i + 1),
        }
    }
    units
}

/// The gap among [`row_units`] that button `from`, dragged to `at` px along the row, drops its unit
/// into (counted as `move_button` does); `tasks` in along/across form (`TaskbarLayout::along_rects`).
/// A unit passes a neighbour once the pointer, offset by the grab, crosses the neighbour's centre. In
/// place is the unit's own index.
pub fn row_drop_slot(
    tasks: &[Rect],
    pins: &[Option<usize>],
    runs: &[Option<String>],
    from: usize,
    at: i32,
) -> usize {
    let units = row_units(pins, runs);
    let span = |u: &std::ops::Range<usize>| {
        let first = *tasks.get(u.start)?;
        Some(
            tasks[u.start..u.end.min(tasks.len())]
                .iter()
                .fold(first, |a, r| a.union(r)),
        )
    };
    let rects: Vec<Rect> = units.iter().filter_map(span).collect();
    let (Some(d), Some(grabbed)) = (
        units.iter().position(|u| u.contains(&from)),
        tasks.get(from),
    ) else {
        return slot_at(&rects, at);
    };
    let Some(unit) = rects.get(d) else {
        return slot_at(&rects, at);
    };
    let past = |r: &Rect, x: i32| x >= r.x + r.width / 2;
    let lead = at - (grabbed.x - unit.x);
    let before = rects[..d].iter().filter(|r| past(r, lead)).count();
    if before < d {
        return before;
    }
    let lead = at + (unit.right() - grabbed.right());
    match rects[d + 1..].iter().filter(|r| past(r, lead)).count() {
        0 => d,
        after => d + 1 + after,
    }
}

/// A reordered row: the pins' old indices in new order, and each unpinned unit's buttons with their new anchor.
pub type RowOrder = (Vec<usize>, Vec<(std::ops::Range<usize>, usize)>);

/// The row after the unit holding button `from` moves to gap `slot`: pins in new order (old indices),
/// and each unpinned unit's first button with the pins now before it. A unit after the last pin anchors
/// at `usize::MAX`, so a pin added later still joins the others before it. None when nothing changes.
pub fn reorder_row(
    pins: &[Option<usize>],
    runs: &[Option<String>],
    from: usize,
    slot: usize,
) -> Option<RowOrder> {
    let mut units = row_units(pins, runs);
    let at = units.iter().position(|u| u.contains(&from))?;
    if !move_button(&mut units, at, slot) {
        return None;
    }
    let (mut order, mut anchors) = (Vec::new(), Vec::new());
    for u in units {
        match pins[u.start] {
            Some(k) => order.push(k),
            None => anchors.push((u, order.len())),
        }
    }
    for (_, a) in anchors.iter_mut().filter(|(_, a)| *a == order.len()) {
        *a = usize::MAX;
    }
    Some((order, anchors))
}

/// The buttons a new layout moved: each key in both `old` and `new` (key and x, in row order) whose x
/// changed, with old and new x, in `new`'s order.
pub fn moved_buttons<K: PartialEq + Copy>(
    old: &[(K, i32)],
    new: &[(K, i32)],
) -> Vec<(K, i32, i32)> {
    new.iter()
        .filter_map(|&(k, to)| {
            let &(_, from) = old.iter().find(|(o, _)| *o == k)?;
            (from != to).then_some((k, from, to))
        })
        .collect()
}

/// `key` after a reorder put the pins in `order` (old indices in new order): a pin is named by its new
/// place, None if absent from `order`; other keys stay.
pub fn pin_renamed<W>(key: TaskKey<W>, order: &[usize]) -> Option<TaskKey<W>> {
    match key {
        TaskKey::Pin(k) => order.iter().position(|&o| o == k).map(TaskKey::Pin),
        other => Some(other),
    }
}

/// A bar's pin-keyed state after a reorder (see [`pin_renamed`]): `hot`, `float` and `slides` keep
/// naming the same pins; a pin gone from `order` drops out.
pub fn pins_renamed<W: Copy, S>(
    hot: &mut Option<TaskKey<W>>,
    float: &mut Option<(TaskKey<W>, S)>,
    slides: &mut Vec<(TaskKey<W>, S)>,
    order: &[usize],
) {
    *hot = hot.and_then(|k| pin_renamed(k, order));
    *float = float
        .take()
        .and_then(|(k, s)| Some((pin_renamed(k, order)?, s)));
    *slides = std::mem::take(slides)
        .into_iter()
        .filter_map(|(k, s)| Some((pin_renamed(k, order)?, s)))
        .collect();
}

/// How far along the bar dragged task button `i` of `tasks` (along/across form) is drawn, with the
/// pointer `pointer` px along it, grabbed `grab` px in: under the pointer, its `unit` inside the row's
/// span. None for a stale `i`.
pub fn float_x(
    tasks: &[Rect],
    unit: std::ops::Range<usize>,
    i: usize,
    pointer: i32,
    grab: i32,
) -> Option<i32> {
    let r = tasks.get(i)?;
    let (head, tail) = (tasks.get(unit.start)?, tasks.get(unit.end - 1)?);
    let (first, last) = (tasks.first()?, tasks.last()?);
    let min = first.x + (r.x - head.x);
    let max = (last.right() - (tail.right() - r.x)).max(min);
    Some((pointer - grab).clamp(min, max))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressTiming {
    OnPress,
    OnRelease,
}

/// With `pinned` on, every task button acts on release so it can be dragged first; off, on the press.
pub fn press_timing(pinned: bool) -> PressTiming {
    if pinned {
        PressTiming::OnRelease
    } else {
        PressTiming::OnPress
    }
}

/// Whether a held task button is a drag: already, or the pointer `moved` past the drag distance. The lock does not stop it.
pub fn press_drags(dragging: bool, moved: bool) -> bool {
    dragging || moved
}

/// What a pointer move inside the lit task button does, for a look whose glow follows the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotStep {
    /// The bar shows this x already, or the hot button changed and its redraw shows it.
    Nothing,
    Arm,
    Wait,
}

/// The step for the pointer's x inside lit button `now`, with the bar last drawn at `drawn` and the
/// tick `armed` or not. The bar redraws at most once a tick.
pub fn spot_due(drawn: Option<i32>, now: Option<i32>, armed: bool) -> SpotStep {
    match (drawn, now) {
        (Some(d), Some(n)) if d != n => {
            if armed {
                SpotStep::Wait
            } else {
                SpotStep::Arm
            }
        }
        _ => SpotStep::Nothing,
    }
}

/// Which task button a press holds, kept across a new row layout so the release finds the same button:
/// a dialog's id, a pinned slot's pin, else the button's first window. A pinned app with a button per
/// window keys each by its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKey<W> {
    Window(W),
    Pin(usize),
    Dialog(u64),
}

/// The key of task button `i`, from the row's `buttons` (windows), `pins` and `dialogs`. None for a stale index.
pub fn task_key<W: Copy>(
    buttons: &[Vec<W>],
    pins: &[Option<usize>],
    dialogs: &[Option<u64>],
    i: usize,
) -> Option<TaskKey<W>> {
    if let Some(id) = dialogs.get(i).copied().flatten() {
        return Some(TaskKey::Dialog(id));
    }
    let pin = pins.get(i).copied().flatten();
    match pin {
        Some(k) if pins.iter().filter(|p| **p == pin).count() == 1 => Some(TaskKey::Pin(k)),
        _ => buttons
            .get(i)
            .and_then(|b| b.first())
            .map(|&w| TaskKey::Window(w))
            .or(pin.map(TaskKey::Pin)),
    }
}

/// The task button `key` names in the row as laid out now; None once gone.
pub fn find_task<W: PartialEq>(
    buttons: &[Vec<W>],
    pins: &[Option<usize>],
    dialogs: &[Option<u64>],
    key: &TaskKey<W>,
) -> Option<usize> {
    match key {
        TaskKey::Window(w) => buttons.iter().position(|b| b.contains(w)),
        TaskKey::Pin(k) => pins.iter().position(|p| *p == Some(*k)),
        TaskKey::Dialog(id) => dialogs.iter().position(|d| *d == Some(*id)),
    }
}

fn task_row(mode: TaskbarButtons) -> TaskRow {
    match mode {
        TaskbarButtons::WhenFull => TaskRow::Shared,
        TaskbarButtons::Never | TaskbarButtons::Group => TaskRow::AtLeast(GROUP_BELOW),
        TaskbarButtons::Combine => TaskRow::Icons,
    }
}

pub fn grip_cursor(edge: reprisekit::taskbar::Edge) -> &'static str {
    if edge.is_side() {
        "ns-resize"
    } else {
        "ew-resize"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JumpRow {
    Recent(std::path::PathBuf),
    /// Bring up window `k`, indexing the `titles` `jump_list` was given.
    Window(usize),
    Launch,
    Pin,
    Unpin,
    /// Opens `winmenu::move_to_menu`, does nothing itself.
    MoveToMenu,
    MoveTo(usize),
    Close,
}

pub const JUMP_RECENT: usize = 10;

/// A jump list's head: a row per recent file, then a separator unless there are none.
pub fn jump_recent_rows(recent: &[std::path::PathBuf]) -> (Vec<MenuItem>, Vec<Option<JumpRow>>) {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    for p in recent {
        let label = p
            .file_name()
            .map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy());
        items.push(MenuItem::entry(label.into_owned()));
        rows.push(Some(JumpRow::Recent(p.clone())));
    }
    if !recent.is_empty() {
        items.push(MenuItem::Separator);
        rows.push(None);
    }
    (items, rows)
}

/// A task button's jump list: the app's `recent` files (at most `JUMP_RECENT`, newest first) and a
/// separator, a group's window `titles` and a separator (a single window gets no row), the app's `name`
/// (new instance), Pin/Unpin this program (greyed when `can_pin` is false: no `.desktop` file), and for
/// a running app Move to desktop (a submenu with more than one of `desktops`) and Close window or Close
/// all windows. Returns the rows and, parallel to them, what each does (None at a separator).
pub fn jump_list(
    name: &str,
    recent: &[std::path::PathBuf],
    pinned: bool,
    can_pin: bool,
    titles: &[String],
    desktops: usize,
) -> (Vec<MenuItem>, Vec<Option<JumpRow>>) {
    let (mut items, mut rows) = jump_recent_rows(recent);
    if titles.len() > 1 {
        for (k, t) in titles.iter().enumerate() {
            items.push(MenuItem::entry(t.clone()));
            rows.push(Some(JumpRow::Window(k)));
        }
        items.push(MenuItem::Separator);
        rows.push(None);
    }
    items.push(MenuItem::entry(name.to_string()));
    rows.push(Some(JumpRow::Launch));
    if pinned {
        items.push(MenuItem::entry("Unpin this program from taskbar"));
        rows.push(Some(JumpRow::Unpin));
    } else {
        items.push(MenuItem::entry("Pin this program to taskbar").enabled(can_pin));
        rows.push(Some(JumpRow::Pin));
    }
    if !titles.is_empty() && desktops > 1 {
        items.push(MenuItem::entry("Move to desktop").submenu(true));
        rows.push(Some(JumpRow::MoveToMenu));
    }
    if !titles.is_empty() {
        let label = if titles.len() == 1 {
            "Close window"
        } else {
            "Close all windows"
        };
        items.push(MenuItem::entry(label));
        rows.push(Some(JumpRow::Close));
    }
    (items, rows)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowListRow {
    /// Bring up window `k`, indexing the `titles` `window_list` was given.
    Window(usize),
    Minimize,
    /// Opens `winmenu::move_to_menu`, does nothing itself.
    MoveToMenu,
    MoveTo(usize),
    Close,
}

/// A grouped task button's window list: the windows' `titles`, then below a separator Minimize Group,
/// Move to desktop (a submenu with more than one of `desktops`) and Close Group. Returns the rows and,
/// parallel to them, what each does (None at the separator).
pub fn window_list(
    titles: &[String],
    desktops: usize,
) -> (Vec<MenuItem>, Vec<Option<WindowListRow>>) {
    let mut items: Vec<MenuItem> = titles.iter().map(|t| MenuItem::entry(t.clone())).collect();
    let mut rows: Vec<_> = (0..titles.len())
        .map(|k| Some(WindowListRow::Window(k)))
        .collect();
    items.push(MenuItem::Separator);
    rows.push(None);
    items.push(MenuItem::entry("Minimize Group"));
    rows.push(Some(WindowListRow::Minimize));
    if desktops > 1 {
        items.push(MenuItem::entry("Move to desktop").submenu(true));
        rows.push(Some(WindowListRow::MoveToMenu));
    }
    items.push(MenuItem::entry("Close Group"));
    rows.push(Some(WindowListRow::Close));
    (items, rows)
}

/// The window a left click on a grouped button brings up with `[taskbar] pinned` on: the group's most
/// recently focused (`recency` runs oldest first, as `Server::focus_order` keeps it), or, if one is
/// `focused`, the next after it in `windows`, so repeated clicks cycle. None for an empty group.
pub fn group_click_pick<T: Copy + PartialEq>(
    windows: &[T],
    recency: &[T],
    focused: Option<T>,
) -> Option<T> {
    if let Some(i) = focused.and_then(|f| windows.iter().position(|&w| w == f)) {
        return Some(windows[(i + 1) % windows.len()]);
    }
    recency
        .iter()
        .rev()
        .copied()
        .find(|w| windows.contains(w))
        .or_else(|| windows.first().copied())
}

/// A label for a group of `app_id`'s windows: the last dotted segment (`org.gnome.Nautilus` reads
/// `Nautilus`), else the id, else `Window`.
pub fn group_name(app_id: &str) -> String {
    let base = app_id.rsplit('.').next().unwrap_or(app_id).trim();
    if base.is_empty() {
        "Window".to_string()
    } else {
        base.to_string()
    }
}

/// Local time as the LC_TIME locale's short time ("3:39 PM" or "15:39"). `main` sets LC_TIME from the environment.
pub fn clock_text() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    time_text(now)
}

/// Today as a long date ("Thursday, September 10, 2026") in the LC_TIME locale: the clock's tip and accessible description.
pub fn date_text() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    reprisefs::locale::strftime(&reprisefs::locale::localtime(now), c"%A, %B %d, %Y")
}

/// The bar's clock text: [`clock_text`], and with `date` the LC_TIME short date ("09/27/2026") on a second line.
pub fn bar_clock_text(date: bool) -> String {
    // One reading for both lines, so a redraw at midnight never pairs one day's time with the next day's date.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let time = time_text(now);
    if !date {
        return time;
    }
    format!("{time}\n{}", local_text(now, c"%x"))
}

pub fn time_text(secs: i64) -> String {
    let fmt = if reprisefs::locale::is_12h() {
        c"%l:%M %p"
    } else {
        c"%H:%M"
    };
    local_text(secs, fmt)
}

fn local_text(secs: i64, fmt: &std::ffi::CStr) -> String {
    let tm = reprisefs::locale::localtime(secs);
    reprisefs::locale::strftime(&tm, fmt).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_text_is_the_long_date() {
        let s = date_text();
        let (_, rest) = s.split_once(", ").expect("weekday");
        let (_, year) = rest.rsplit_once(", ").expect("year");
        assert_eq!(year.len(), 4);
        assert!(year.chars().all(|c| c.is_ascii_digit()));
    }
    use reprisekit::taskbar::TASK_GAP;

    #[test]
    fn spot_redraw_waits_for_the_tick() {
        assert_eq!(
            spot_due(Some(10), Some(10), false),
            SpotStep::Nothing,
            "same x"
        );
        assert_eq!(
            spot_due(Some(10), Some(11), false),
            SpotStep::Arm,
            "first move arms"
        );
        assert_eq!(
            spot_due(Some(10), Some(12), true),
            SpotStep::Wait,
            "armed: the tick draws it"
        );
        assert_eq!(
            spot_due(None, Some(12), false),
            SpotStep::Nothing,
            "a new hot button redraws anyway"
        );
    }

    #[test]
    #[ignore]
    fn render_taskbar_menu() {
        use crate::raster::Canvas;
        use crate::startmenu::{MenuLayout, MenuStyle, draw_menu};
        use crate::text::TextRenderer;
        use crate::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let (items, _) = taskbar_menu(true, false);
        let style = MenuStyle::context_for(&theme, &items).fit(
            &text,
            &items,
            theme.fonts.size_px,
            i32::MAX,
        );
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_menu(&mut c, &theme, &text, &layout, &items, None, "", false);
        crate::raster::write_preview(&c, "taskbarmenu-preview.png");
    }

    /// Writes target/windowlist-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_window_list() {
        use crate::raster::Canvas;
        use crate::startmenu::{MenuLayout, MenuStyle, draw_menu};
        use crate::text::TextRenderer;
        use crate::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let titles = ["~/projects - foot", "~/notes - foot", "htop - foot"].map(String::from);
        let (items, _) = window_list(&titles, 4);
        let style = MenuStyle::submenu(&theme).fit(&text, &items, theme.fonts.size_px, i32::MAX);
        let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_menu(&mut c, &theme, &text, &layout, &items, None, "", false);
        crate::raster::write_preview(&c, "windowlist-preview.png");
    }

    #[test]
    fn taskbar_menu_lists_its_rows() {
        let (items, commands) = taskbar_menu(true, true);
        assert_eq!(items.len(), 11);
        assert_eq!(commands[0], Some(TaskbarCommand::Toolbars));
        assert!(matches!(items[0], MenuItem::Entry { submenu: true, .. }));
        assert!(matches!(items[1], MenuItem::Separator));
        assert_eq!(commands[2], Some(TaskbarCommand::Cascade));
        assert_eq!(commands[5], Some(TaskbarCommand::ShowDesktop));
        assert!(matches!(items[6], MenuItem::Separator));
        assert_eq!(commands[7], Some(TaskbarCommand::TaskManager));
        assert!(matches!(items[8], MenuItem::Separator));
        assert_eq!(commands[9], Some(TaskbarCommand::LockTaskbar));
        assert!(matches!(items[9], MenuItem::Entry { checked: true, .. }));
        assert_eq!(commands[10], Some(TaskbarCommand::Properties));
        for it in &items[2..6] {
            assert!(matches!(it, MenuItem::Entry { enabled: true, .. }));
        }
        let (items, _) = taskbar_menu(false, false);
        assert!(matches!(items[9], MenuItem::Entry { checked: false, .. }));
        for it in &items[2..5] {
            assert!(matches!(it, MenuItem::Entry { enabled: false, .. }));
        }
        assert!(matches!(items[5], MenuItem::Entry { enabled: true, .. }));
        let (rows, cmds) = toolbars_menu(true, false);
        assert_eq!(cmds, vec![Some(TaskbarCommand::QuickLaunch)]);
        assert!(matches!(rows[0], MenuItem::Entry { checked: true, .. }));
        assert!(matches!(
            toolbars_menu(false, false).0[0],
            MenuItem::Entry {
                checked: false,
                enabled: true,
                ..
            }
        ));
        assert!(
            matches!(
                toolbars_menu(true, true).0[0],
                MenuItem::Entry { enabled: false, .. }
            ),
            "greyed with the programs pinned to the task row"
        );
    }

    #[test]
    fn clock_format() {
        // Locale-dependent: a colon between two runs of digits and no stray padding.
        let s = clock_text();
        assert_eq!(s, s.trim());
        let (h, rest) = s.split_once(':').expect("HH:MM");
        assert!(h.chars().all(|c| c.is_ascii_digit()) && !h.is_empty());
        assert!(rest.as_bytes()[..2].iter().all(u8::is_ascii_digit));
    }

    fn lay_buttons(
        layout: &mut TaskbarLayout,
        m: &reprisekit::theme::Metrics,
        shape: &TaskbarShape,
        apps: &[String],
        mode: TaskbarButtons,
    ) -> Vec<Vec<usize>> {
        lay_slots(layout, m, shape, apps, &ends(apps.len()), &[], 0, &[], mode)
            .into_iter()
            .map(|s| s.windows)
            .collect()
    }

    fn keys(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_grip_drags_along_its_bar() {
        use reprisekit::taskbar::Edge;
        assert_eq!(grip_cursor(Edge::Bottom), "ew-resize");
        assert_eq!(grip_cursor(Edge::Top), "ew-resize");
        assert_eq!(grip_cursor(Edge::Left), "ns-resize");
        assert_eq!(grip_cursor(Edge::Right), "ns-resize");
    }

    /// Up a side, when_full groups an app's windows once the column holds no more full buttons, not at
    /// `GROUP_BELOW`; along the bottom the same windows stay apart.
    #[test]
    fn a_side_bar_groups_when_its_column_is_full() {
        use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge};
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let apps = keys(&["a", "a", "b", "c", "d", "e", "f", "g", "h", "i"]);
        let counts = TaskbarCounts {
            tasks: apps.len(),
            ..TaskbarCounts::default()
        };
        let column = |length: i32| {
            let span = BarSpan {
                edge: Edge::Left,
                length,
                thickness: 60,
            };
            TaskbarLayout::compute_on(
                &m,
                &shape,
                span,
                true,
                counts,
                ClockText::default(),
                Bands::default(),
            )
        };
        let mut side = column(200);
        let plan = lay_buttons(&mut side, &m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(plan.len(), 9, "the two a windows share a button");
        // 27..370 down a 400 px column: 31 px a button, under GROUP_BELOW, yet room for ten full buttons.
        let mut tall = column(400);
        let plan = lay_buttons(&mut tall, &m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(
            plan.len(),
            10,
            "ten full buttons fit: each window keeps its own"
        );
        let mut row = TaskbarLayout::compute(&m, &shape, 1920, true, counts, 30);
        let plan = lay_buttons(&mut row, &m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(plan.len(), 10, "a 1920 px row has room for each");
    }

    /// Glass's 60 px column is icons only, buttons 2 px apart: an app's windows share a button once full
    /// buttons no longer fit, so none waits behind the » cell.
    #[test]
    fn an_icon_only_glass_column_groups_by_its_icon_buttons_gap() {
        use reprisekit::interface::Interface;
        use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge};
        let glass = reprisekit::interfaces::theme_for(Interface::Glass);
        let (m, shape) = (&glass.metrics, glass.taskbar_shape());
        let column = |tasks: usize| {
            let span = BarSpan {
                edge: Edge::Left,
                length: 800,
                thickness: 60,
            };
            let counts = TaskbarCounts {
                tasks,
                ..TaskbarCounts::default()
            };
            TaskbarLayout::compute_on(
                m,
                &shape,
                span,
                true,
                counts,
                ClockText::default(),
                Bands::default(),
            )
        };
        let l = column(1);
        assert!(l.icons_only, "Glass's 60 px column: icons only");
        let room = l.tasks_limit() - l.tasks_x();
        let (button, pitch) = l.icon_step(&shape);
        assert_eq!(pitch - button, 2, "the icon buttons' gap");
        let n = room / button;
        assert!(n * pitch - 2 > room, "{n} buttons in {room} px");
        let mut apps = keys(&["a", "a"]);
        apps.extend((2..n).map(|i| i.to_string()));
        let mut l = column(apps.len());
        let plan = lay_buttons(&mut l, m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(
            plan.len(),
            apps.len() - 1,
            "the two a windows share a button"
        );
        assert_eq!(l.tasks.len(), plan.len(), "every button shows");
        assert_eq!(l.more, None);
    }

    #[test]
    fn a_two_line_clock_puts_the_date_under_the_time() {
        assert_eq!(bar_clock_text(false), clock_text(), "one line: the time");
        let two = bar_clock_text(true);
        let (time, date) = two.split_once('\n').expect("two lines");
        assert_eq!(time, clock_text());
        assert!(
            date.chars().any(|c| c.is_ascii_digit()) && !date.contains('\n'),
            "the date is one line of digits and separators, not {date:?}"
        );
    }

    #[test]
    fn plan_buttons_groups_only_when_the_bar_is_full() {
        let apps = keys(&["a", "b", "a", "c"]);
        let when_full = TaskbarButtons::WhenFull;
        let wide = plan_buttons(&apps, 1000, GROUP_BELOW, TASK_GAP, when_full);
        assert_eq!(wide, vec![vec![0], vec![1], vec![2], vec![3]]);
        // Tight: the two-window app "a" collapses at its first slot, the singles stay put.
        let tight = plan_buttons(&apps, 120, GROUP_BELOW, TASK_GAP, when_full);
        assert_eq!(tight, vec![vec![0, 2], vec![1], vec![3]]);
    }

    #[test]
    fn plan_buttons_handles_empty_and_all_grouped() {
        let when_full = TaskbarButtons::WhenFull;
        assert!(plan_buttons(&[], 100, GROUP_BELOW, TASK_GAP, when_full).is_empty());
        let same: Vec<String> = vec!["x".into(); 5];
        let tight = plan_buttons(&same, 100, GROUP_BELOW, TASK_GAP, when_full);
        assert_eq!(tight, vec![vec![0, 1, 2, 3, 4]]);
    }

    #[test]
    fn combine_groups_every_app_with_two_or_more_windows_however_roomy() {
        let apps = keys(&["a", "b", "a", "c", "b", "a"]);
        let want = vec![vec![0, 2, 5], vec![1, 4], vec![3]];
        for avail in [120, 1000, 10_000] {
            assert_eq!(
                plan_buttons(&apps, avail, GROUP_BELOW, TASK_GAP, TaskbarButtons::Combine),
                want,
                "one button per app in first-window order at {avail} px"
            );
        }
        assert!(plan_buttons(&[], 1000, GROUP_BELOW, TASK_GAP, TaskbarButtons::Combine).is_empty());
    }

    #[test]
    fn never_gives_every_window_its_own_button_however_tight() {
        let apps = keys(&["a", "b", "a", "a"]);
        for avail in [0, 120, 1000] {
            assert_eq!(
                plan_buttons(&apps, avail, GROUP_BELOW, TASK_GAP, TaskbarButtons::Never),
                vec![vec![0], vec![1], vec![2], vec![3]],
                "one button per window at {avail} px"
            );
        }
    }

    #[test]
    fn each_buttons_mode_lays_its_own_row() {
        use crate::config::Config;
        let apps = keys(&["a", "b", "a", "c"]);
        let row = |text: &str| {
            let cfg = Config::parse(text).unwrap();
            let (m, shape) = (&cfg.theme.metrics, cfg.theme.taskbar_shape());
            let counts = TaskbarCounts {
                tasks: apps.len(),
                ..TaskbarCounts::default()
            };
            let mut l = TaskbarLayout::compute(m, &shape, 800, true, counts, 30);
            let plan = lay_buttons(&mut l, m, &shape, &apps, cfg.taskbar.buttons);
            (plan, l.tasks, l.stacks)
        };
        let (plan, tasks, stacks) = row("");
        assert_eq!(plan.len(), 4, "when_full has room for every window");
        assert_eq!(tasks[1], Rect::new(222, 4, 160, 22));
        assert_eq!(stacks, vec![1; 4]);
        let (plan, tasks, stacks) = row("[taskbar]\nbuttons = \"combine\"\n");
        assert_eq!(plan, vec![vec![0, 2], vec![1], vec![3]]);
        assert_eq!(
            tasks,
            vec![
                Rect::new(59, 4, 22, 22),
                Rect::new(84, 4, 22, 22),
                Rect::new(109, 4, 22, 22),
            ],
            "square icon buttons"
        );
        assert_eq!(stacks, vec![2, 1, 1]);
        let (plan, tasks, _) = row("[taskbar]\nbuttons = \"never\"\n");
        assert_eq!(plan.len(), 4);
        assert_eq!(tasks[1].width, 160);
    }

    #[test]
    fn never_stops_shrinking_where_when_full_would_group() {
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let apps: Vec<String> = vec!["x".into(); 12];
        let counts = TaskbarCounts {
            tasks: apps.len(),
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&m, &shape, 600, true, counts, 30);
        let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::Never);
        assert_eq!(plan.len(), 12, "a button per window");
        assert!(l.tasks.iter().all(|r| r.width == GROUP_BELOW));
        assert!(l.tasks.len() < 12, "the rest are left out");
        let mut l = TaskbarLayout::compute(&m, &shape, 600, true, counts, 30);
        let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(
            plan,
            vec![(0..12).collect::<Vec<_>>()],
            "one grouped button"
        );
        assert_eq!(l.stacks, vec![12]);
    }

    /// A top bar lays its task row after mirroring, so the buttons keep the screen-edge gap against the top.
    #[test]
    fn a_top_bar_lays_its_task_row_against_the_screen_edge() {
        use reprisekit::taskbar::Edge;
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let apps = keys(&["a", "b", "a", "c"]);
        let counts = TaskbarCounts {
            tasks: apps.len(),
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30).on_edge(Edge::Top);
        let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::WhenFull);
        assert_eq!(plan.len(), 4);
        assert_eq!(l.tasks[1], Rect::new(222, 2, 160, 22));
        assert_eq!(l.edge, Edge::Top);
    }

    fn pin(file: &str, class: Option<&str>) -> QuickItem {
        let class = class.map_or(String::new(), |c| format!("StartupWMClass={c}\n"));
        let text = format!("[Desktop Entry]\nType=Application\nName={file}\nExec=x\n{class}");
        QuickItem {
            file: file.into(),
            entry: crate::quicklaunch::parse(&text, file, "foot").unwrap(),
            app: reprisefs::apps::parse_desktop(&text, file),
        }
    }

    #[test]
    fn a_window_finds_its_pin_by_desktop_id_then_class_then_file_name() {
        let pins = [
            pin("foot.desktop", None),
            pin("footclient.desktop", None),
            pin("code.desktop", None),
            pin("vscode.desktop", Some("Code")),
            pin("org.gnome.Nautilus.desktop", None),
        ];
        assert_eq!(
            pin_for(
                &pins,
                "org.gnome.Nautilus",
                Some("org.gnome.Nautilus.desktop")
            ),
            Some(4)
        );
        assert_eq!(pin_for(&pins, "FOOT", None), Some(0), "by file name");
        assert_eq!(
            pin_for(&pins, "foot", Some("footclient.desktop")),
            Some(1),
            "the desktop id wins over a file name"
        );
        assert_eq!(
            pin_for(&pins, "code", None),
            Some(3),
            "the class wins over a file name"
        );
        assert_eq!(pin_for(&pins, "alacritty", None), None);
        assert_eq!(pin_for(&pins, "", Some("foot.desktop")), None);
    }

    fn ends(n: usize) -> Vec<Place> {
        (0..n).map(|i| Place::end(i as u64)).collect()
    }
    fn order(slots: &[Slot]) -> Vec<String> {
        slots
            .iter()
            .map(|s| match (s.pin, s.dialog) {
                (Some(k), _) => format!("p{k}"),
                (_, Some(d)) => format!("d{d}"),
                _ => format!("w{}", s.windows[0]),
            })
            .collect()
    }

    #[test]
    fn an_anchored_button_sits_after_that_many_pins() {
        let places = [Place { anchor: 0, seq: 5 }, Place { anchor: 1, seq: 6 }];
        let slots = merge_row(vec![vec![0], vec![1]], 2, &[None, None], &places, &[]);
        assert_eq!(order(&slots), ["w0", "p0", "w1", "p1"]);
    }

    #[test]
    fn a_new_window_lands_after_a_moved_one() {
        let places = [Place { anchor: 0, seq: 9 }, Place::end(3), Place::end(10)];
        let slots = merge_row(vec![vec![0], vec![1], vec![2]], 1, &[None; 3], &places, &[]);
        assert_eq!(order(&slots), ["w0", "p0", "w1", "w2"]);
    }

    #[test]
    fn an_anchor_past_the_pins_sits_at_the_end() {
        let places = [Place { anchor: 5, seq: 1 }, Place::end(2)];
        let slots = merge_row(vec![vec![0], vec![1]], 1, &[None, None], &places, &[]);
        assert_eq!(order(&slots), ["p0", "w0", "w1"]);
    }

    #[test]
    fn a_group_sits_at_its_earliest_window() {
        let places = [Place::end(4), Place::end(5), Place { anchor: 0, seq: 1 }];
        let slots = merge_row(vec![vec![0, 2], vec![1]], 1, &[None; 3], &places, &[]);
        assert_eq!(order(&slots), ["w0", "p0", "w1"]);
    }

    #[test]
    fn a_dialog_takes_its_own_place() {
        let places = [Place::end(1), Place::end(3)];
        let dialogs = [DialogTask {
            id: 7,
            place: Place::end(2),
        }];
        let slots = merge_row(vec![vec![0], vec![1]], 0, &[None, None], &places, &dialogs);
        assert_eq!(order(&slots), ["w0", "d7", "w1"]);
    }

    #[test]
    fn pinned_slots_lead_in_pin_order_and_unmatched_apps_follow() {
        let pin_of = [None, Some(0), None, Some(0), Some(0)];
        let slot = |windows: &[usize], pin: Option<usize>| Slot {
            windows: windows.to_vec(),
            pin,
            dialog: None,
        };
        let combined = merge_row(
            vec![vec![0], vec![1, 3, 4], vec![2]],
            2,
            &pin_of,
            &ends(5),
            &[],
        );
        assert_eq!(
            combined,
            [
                slot(&[1, 3, 4], Some(0)),
                slot(&[], Some(1)),
                slot(&[0], None),
                slot(&[2], None),
            ]
        );
        let plan = (0..5).map(|i| vec![i]).collect();
        assert_eq!(
            merge_row(plan, 2, &pin_of, &ends(5), &[]),
            [
                slot(&[1], Some(0)),
                slot(&[3], Some(0)),
                slot(&[4], Some(0)),
                slot(&[], Some(1)),
                slot(&[0], None),
                slot(&[2], None),
            ]
        );
        assert_eq!(
            merge_row(vec![vec![0]], 2, &[Some(0)], &ends(1), &[]),
            [slot(&[0], Some(0)), slot(&[], Some(1))]
        );
        assert_eq!(
            merge_row(Vec::new(), 2, &[], &[], &[]),
            [slot(&[], Some(0)), slot(&[], Some(1))]
        );
        assert_eq!(
            merge_row(
                vec![vec![1], vec![0]],
                0,
                &[],
                &[Place::end(1), Place::end(0)],
                &[]
            ),
            [slot(&[1], None), slot(&[0], None)]
        );
    }

    #[test]
    fn lay_slots_puts_idle_pins_in_icon_slots() {
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let apps = keys(&["foot", "kitty", "foot"]);
        let pin_of = [Some(1), None, Some(1)];
        let counts = TaskbarCounts {
            tasks: 4,
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        let slots = lay_slots(
            &mut l,
            &m,
            &shape,
            &apps,
            &ends(3),
            &pin_of,
            2,
            &[],
            TaskbarButtons::WhenFull,
        );
        let windows: Vec<Vec<usize>> = slots.iter().map(|s| s.windows.clone()).collect();
        assert_eq!(windows, [vec![], vec![0], vec![2], vec![1]]);
        assert_eq!(l.stacks, [0, 1, 1, 1]);
        assert_eq!(l.tasks[0], Rect::new(59, 4, 22, 22), "the idle pin");
        assert_eq!(l.tasks[1], Rect::new(84, 4, 160, 22));
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        lay_slots(
            &mut l,
            &m,
            &shape,
            &apps,
            &ends(3),
            &pin_of,
            2,
            &[],
            TaskbarButtons::Combine,
        );
        assert_eq!(l.stacks, [0, 2, 1]);
        assert!(l.tasks.iter().all(|r| r.width == 22));
    }

    #[test]
    fn a_dragged_button_drops_among_the_row_units() {
        let tasks = [
            Rect::new(0, 0, 60, 40),
            Rect::new(62, 0, 60, 40),
            Rect::new(124, 0, 60, 40),
            Rect::new(186, 0, 60, 40),
        ];
        let pins = [Some(0), Some(0), Some(1), None];
        assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 10), 0);
        assert_eq!(
            row_drop_slot(&tasks, &pins, &[], 3, 60),
            0,
            "pin 0 spans 0..122"
        );
        assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 61), 1);
        assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 150), 1);
        assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 160), 2);
        assert_eq!(
            row_drop_slot(&tasks, &pins, &[], 3, 400),
            2,
            "the unpinned button stays last"
        );
        let mut order = vec!["foot", "files"];
        assert!(move_button(
            &mut order,
            0,
            row_drop_slot(&tasks, &pins, &[], 3, 160)
        ));
        assert_eq!(order, ["files", "foot"]);
    }

    #[test]
    fn a_reorder_slides_the_buttons_whose_x_changed_from_their_old_x() {
        let k = TaskKey::<u8>::Window;
        let old = [(k(1), 0), (k(2), 62), (TaskKey::Pin(0), 124), (k(3), 186)];
        let new = [(k(2), 0), (k(1), 62), (TaskKey::Pin(0), 124), (k(4), 186)];
        assert_eq!(
            moved_buttons(&old, &new),
            [(k(2), 62, 0), (k(1), 0, 62)],
            "the pin stays put; a button new to the row or gone from it slides nowhere"
        );
    }

    #[test]
    fn a_reorder_names_a_pin_by_its_new_place() {
        let order = [2, 0, 1];
        assert_eq!(
            pin_renamed(TaskKey::<u8>::Pin(0), &order),
            Some(TaskKey::Pin(1))
        );
        assert_eq!(
            pin_renamed(TaskKey::<u8>::Pin(2), &order),
            Some(TaskKey::Pin(0))
        );
        assert_eq!(pin_renamed(TaskKey::<u8>::Pin(5), &order), None);
        assert_eq!(
            pin_renamed(TaskKey::Window(7u8), &order),
            Some(TaskKey::Window(7))
        );
        assert_eq!(
            pin_renamed(TaskKey::<u8>::Dialog(3), &order),
            Some(TaskKey::Dialog(3))
        );
    }

    #[test]
    fn a_reorder_keeps_the_lit_floated_and_sliding_pins() {
        let order = [1, 0];
        let mut hot = Some(TaskKey::<u8>::Pin(0));
        let mut float = Some((TaskKey::<u8>::Pin(0), 40));
        let mut slides = vec![(TaskKey::<u8>::Pin(1), 62), (TaskKey::Window(7u8), 9)];
        pins_renamed(&mut hot, &mut float, &mut slides, &order);
        assert_eq!(hot, Some(TaskKey::Pin(1)), "the lit pin is now pin 1");
        assert_eq!(float, Some((TaskKey::Pin(1), 40)), "the float is now pin 1");
        assert_eq!(
            slides,
            [(TaskKey::Pin(0), 62), (TaskKey::Window(7), 9)],
            "the neighbour slides as pin 0, a window keeps its key"
        );

        let mut hot = Some(TaskKey::<u8>::Pin(2));
        let mut float = Some((TaskKey::<u8>::Pin(2), 0));
        let mut slides = vec![(TaskKey::<u8>::Pin(2), 0)];
        pins_renamed(&mut hot, &mut float, &mut slides, &order);
        assert_eq!(
            (hot, float, slides.len()),
            (None, None, 0),
            "a pin gone from the order drops out"
        );
    }

    #[test]
    fn a_dragged_button_floats_where_it_was_grabbed_inside_the_row() {
        let tasks = [
            Rect::new(100, 0, 60, 40),
            Rect::new(162, 0, 60, 40),
            Rect::new(224, 0, 30, 40),
        ];
        assert_eq!(float_x(&tasks, 0..1, 0, 150, 20), Some(130));
        assert_eq!(
            float_x(&tasks, 0..1, 0, 50, 20),
            Some(100),
            "not before the row"
        );
        assert_eq!(
            float_x(&tasks, 0..1, 0, 400, 20),
            Some(194),
            "its right edge stops at the row's end"
        );
        assert_eq!(
            float_x(&tasks, 2..3, 2, 400, 5),
            Some(224),
            "a narrow one too"
        );
        assert_eq!(float_x(&tasks, 3..4, 3, 150, 20), None);
    }

    #[test]
    fn a_pins_buttons_move_as_one_unit() {
        let pins = [Some(0), Some(0), None, Some(1)];
        assert_eq!(row_units(&pins, &[]), [0..2, 2..3, 3..4]);
    }

    #[test]
    fn an_unpinned_button_dropped_before_the_pins_anchors_at_0() {
        let pins = [Some(0), Some(1), None];
        let (pin_order, anchors) = reorder_row(&pins, &[], 2, 0).unwrap();
        assert_eq!(pin_order, [0, 1]);
        assert_eq!(anchors, [(2..3, 0)]);
    }

    #[test]
    fn a_pin_dragged_past_an_unpinned_button_reorders_both() {
        let pins = [Some(0), None, Some(1)];
        let (pin_order, anchors) = reorder_row(&pins, &[], 0, 3).unwrap();
        assert_eq!(pin_order, [1, 0], "p1 first, then p0");
        assert_eq!(anchors, [(1..2, 0)], "w now precedes both pins");
    }

    #[test]
    fn a_pin_added_after_a_drag_joins_the_pins() {
        let pins = [Some(0), Some(1), None, None];
        let (pin_order, anchors) = reorder_row(&pins, &[], 2, 1).unwrap();
        assert_eq!(pin_order, [0, 1]);
        assert_eq!(anchors, [(2..3, 1), (3..4, usize::MAX)]);
        let places = [
            Place { anchor: 1, seq: 10 },
            Place {
                anchor: anchors[1].1,
                seq: 11,
            },
        ];
        let slots = merge_row(vec![vec![0], vec![1]], 3, &[None, None], &places, &[]);
        assert_eq!(order(&slots), ["p0", "w0", "p1", "p2", "w1"]);
    }

    #[test]
    fn a_drop_in_place_moves_nothing() {
        let pins = [Some(0), None];
        assert_eq!(reorder_row(&pins, &[], 1, 1), None);
        assert_eq!(reorder_row(&pins, &[], 1, 2), None);
    }

    #[test]
    fn with_pins_every_task_button_acts_on_release_and_without_on_press() {
        assert_eq!(press_timing(true), PressTiming::OnRelease);
        assert_eq!(press_timing(false), PressTiming::OnPress);
    }

    /// A held button is found again by its window or pin after the row is laid out anew.
    #[test]
    fn a_held_task_button_is_found_again_after_the_row_changes() {
        let buttons = vec![vec![10], vec![20], vec![]];
        let pins = vec![None, None, Some(3)];
        let second = task_key(&buttons, &pins, &[], 1).unwrap();
        let pinned = task_key(&buttons, &pins, &[], 2).unwrap();
        assert_eq!((second, pinned), (TaskKey::Window(20), TaskKey::Pin(3)));
        assert_eq!(task_key(&buttons, &pins, &[], 3), None, "stale index");
        let buttons = vec![vec![20], vec![]];
        let pins = vec![None, Some(3)];
        assert_eq!(find_task(&buttons, &pins, &[], &second), Some(0));
        assert_eq!(find_task(&buttons, &pins, &[], &pinned), Some(1));
        let buttons = vec![vec![20], vec![30, 31]];
        assert_eq!(
            find_task(&buttons, &pins, &[], &pinned),
            Some(1),
            "same slot"
        );
        assert_eq!(
            find_task(&buttons, &pins, &[], &TaskKey::Window(31)),
            Some(1)
        );
        assert_eq!(
            find_task(&buttons, &pins, &[], &TaskKey::Window(10)),
            None,
            "gone"
        );
        // A pinned slot is held by its pin even with windows, so it outlives them.
        assert_eq!(task_key(&buttons, &pins, &[], 1), Some(TaskKey::Pin(3)));
        let buttons = vec![vec![20], vec![]];
        assert_eq!(
            find_task(&buttons, &pins, &[], &pinned),
            Some(1),
            "windows gone"
        );
        let buttons = vec![vec![20], vec![30], vec![31]];
        let pins = vec![None, Some(3), Some(3)];
        assert_eq!(task_key(&buttons, &pins, &[], 2), Some(TaskKey::Window(31)));
    }

    /// The hot button is keyed like a held one: when its window closes, the neighbour sliding into its
    /// place stays dark until the pointer moves.
    #[test]
    fn a_hot_task_button_that_closes_lights_no_neighbour() {
        let buttons = vec![vec![10], vec![20], vec![30]];
        let pins = vec![None; 3];
        let hot = task_key(&buttons, &pins, &[], 1).unwrap();
        assert_eq!(find_task(&buttons, &pins, &[], &hot), Some(1));
        let buttons = vec![vec![10], vec![30]];
        assert_eq!(find_task(&buttons, &pins[..2], &[], &hot), None);
        let buttons = vec![vec![20], vec![30]];
        assert_eq!(find_task(&buttons, &pins[..2], &[], &hot), Some(0));
    }

    /// A dialog's button follows the pinned buttons and earlier windows and goes before later ones, as
    /// wide as theirs. It never groups, and its room counts when windows decide whether to.
    #[test]
    fn group_puts_a_new_window_by_its_apps_buttons() {
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let counts = TaskbarCounts {
            tasks: 6,
            ..TaskbarCounts::default()
        };
        let apps = keys(&["notepad", "explorer", "notepad", "", "", "notepad"]);
        let seqs = [0, 2, 1, 3, 4, 5].map(Place::end);
        let row = |mode| {
            let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
            order(&lay_slots(
                &mut l,
                &m,
                &shape,
                &apps,
                &seqs,
                &[],
                0,
                &[],
                mode,
            ))
        };
        assert_eq!(
            row(TaskbarButtons::Group),
            ["w0", "w2", "w5", "w1", "w3", "w4"],
            "the notepads in creation order, then explorer; no app id groups nothing"
        );
        assert_eq!(
            row(TaskbarButtons::Never),
            ["w0", "w2", "w1", "w3", "w4", "w5"],
            "never keeps creation order"
        );
    }

    #[test]
    fn a_dragged_unit_passes_a_neighbour_by_its_leading_end() {
        let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
        let pins = [None; 4];
        let run = Some("notepad".to_string());
        let runs = [run.clone(), run.clone(), run, None];
        assert_eq!(row_drop_slot(&tasks, &pins, &runs, 1, 90), 0, "in place");
        assert_eq!(
            row_drop_slot(&tasks, &pins, &runs, 1, 89),
            0,
            "the lead stays short of the lone button's centre"
        );
        assert_eq!(
            row_drop_slot(&tasks, &pins, &runs, 1, 150),
            2,
            "the pointer carried to the run's last button (210) reaches the lone button's centre"
        );
        let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
        let runs = [None, runs[0].clone(), runs[1].clone(), runs[2].clone()];
        assert_eq!(
            row_drop_slot(&tasks, &pins, &runs, 3, 150),
            1,
            "grabbed in its tail, the pointer carried to the run's first button (30) reaches the centre"
        );
        assert_eq!(row_drop_slot(&tasks, &pins, &runs, 3, 149), 0);
    }

    #[test]
    fn a_floated_unit_stays_inside_the_row() {
        let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
        assert_eq!(
            float_x(&tasks, 1..4, 2, 900, 30),
            Some(120),
            "the run's tail stops at the row's end"
        );
        assert_eq!(
            float_x(&tasks, 1..4, 2, -50, 30),
            Some(60),
            "its head stops at the row's start"
        );
    }

    #[test]
    fn a_dragged_group_run_moves_as_one_unit() {
        let pins = [Some(0), None, None, None];
        let runs = [
            None,
            Some("notepad".into()),
            Some("notepad".into()),
            Some("explorer".into()),
        ];
        assert_eq!(row_units(&pins, &runs), [0..1, 1..3, 3..4]);
        let (_, anchors) = reorder_row(&pins, &runs, 2, 3).unwrap();
        assert_eq!(
            anchors,
            [(3..4, usize::MAX), (1..3, usize::MAX)],
            "explorer, then the whole notepad run, after the pin"
        );
    }

    #[test]
    fn a_dialogs_button_goes_in_creation_order() {
        let m = reprisekit::theme::Metrics::default();
        let shape = TaskbarShape::CLASSIC;
        let counts = TaskbarCounts {
            tasks: 4,
            ..TaskbarCounts::default()
        };
        let mode = TaskbarButtons::WhenFull;
        let row = |slots: Vec<Slot>| -> Vec<(Vec<usize>, Option<usize>, Option<u64>)> {
            slots
                .into_iter()
                .map(|s| (s.windows, s.pin, s.dialog))
                .collect()
        };
        let apps = keys(&["foot", "kitty", "xterm"]);
        let seqs = [Place::end(9), Place::end(2), Place::end(6)];
        let pin_of = [Some(0), None, None];
        let dialog = [DialogTask {
            id: 7,
            place: Place::end(4),
        }];
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        let slots = lay_slots(&mut l, &m, &shape, &apps, &seqs, &pin_of, 1, &dialog, mode);
        assert_eq!(
            row(slots),
            [
                (vec![0], Some(0), None),
                (vec![1], None, None),
                (vec![], None, Some(7)),
                (vec![2], None, None),
            ]
        );
        assert_eq!(l.stacks, [1, 1, 1, 1]);
        assert_eq!(l.tasks[2].width, l.tasks[1].width);
        assert!(l.tasks[2].x > l.tasks[1].right());
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        let last = [DialogTask {
            id: 7,
            place: Place::end(10),
        }];
        let slots = lay_slots(&mut l, &m, &shape, &apps, &seqs, &pin_of, 1, &last, mode);
        assert_eq!(slots[3].dialog, Some(7), "made last, it ends the row");

        let gap = shape.labelled.gap;
        let l0 = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        let n = ((l0.tasks_limit() - l0.tasks_x() + gap) / (GROUP_BELOW + gap)) as usize;
        let same = vec![String::from("foot"); n];
        let seqs: Vec<Place> = (0..n as u64).map(|i| Place::end(i * 2)).collect();
        let mut l = l0.clone();
        let alone = lay_slots(&mut l, &m, &shape, &same, &seqs, &[], 0, &[], mode);
        assert_eq!(alone.len(), n);
        let mut l = l0.clone();
        let between = [DialogTask {
            id: 7,
            place: Place::end(1),
        }];
        let grouped = lay_slots(&mut l, &m, &shape, &same, &seqs, &[], 0, &between, mode);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].windows, (0..n).collect::<Vec<_>>());
        assert_eq!(grouped[1].dialog, Some(7));
    }

    /// End Program's button beside its window on a 28 px bar: 160x22 at x 222, three past the window's.
    /// While active it is pushed in over the dither, 16 px glyph at (5, 4), bold title from x 25 with an
    /// ellipsis inside the button; otherwise raised.
    #[test]
    fn a_dialogs_button_draws_as_a_task_button() {
        use crate::raster::Canvas;
        use crate::text::TextRenderer;
        use crate::theme::{FontConfig, Theme};
        let mut theme = Theme::default();
        theme.metrics.taskbar_height = 28;
        let cs = theme.colors.clone();
        let text = TextRenderer::load(&FontConfig::default());
        let m = &theme.metrics;
        let shape = theme.taskbar_shape();
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(m, &shape, 800, true, counts, 30);
        let apps = keys(&["hang"]);
        let slots = lay_slots(
            &mut l,
            m,
            &shape,
            &apps,
            &[Place::end(0)],
            &[],
            0,
            &[DialogTask {
                id: 7,
                place: Place::end(1),
            }],
            TaskbarButtons::WhenFull,
        );
        assert_eq!(slots[1].dialog, Some(7));
        assert_eq!(l.tasks[0], Rect::new(59, 4, 160, 22));
        let r = l.tasks[1];
        assert_eq!(r, Rect::new(222, 4, 160, 22));
        let window = || TaskEntry {
            title: "Hang".into(),
            active: false,
            attention: false,
            icon: None,
            glow: None,
        };
        let draw = |active: bool| {
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: None,
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                task_progress: Vec::new(),
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![
                    window(),
                    dialog_entry("End Program - signal-desktop", active),
                ],
                tray_icons: vec![],
                clock: "00:00".into(),
            };
            let mut c = Canvas::new(800, 28);
            reprisekit::look::draw_taskbar(&mut c, &theme, &text, &text, &l, &state);
            c
        };
        let px = |c: &Canvas, x: i32, y: i32| c.get(r.x + x, r.y + y);
        let c = draw(true);
        assert_eq!(
            px(&c, 0, 10),
            cs.button_dk_shadow.argb(),
            "pushed: dark left"
        );
        assert_eq!(px(&c, 1, 10), cs.button_shadow.argb());
        assert_eq!(
            px(&c, 10, 0),
            cs.button_dk_shadow.argb(),
            "pushed: dark top"
        );
        assert_eq!(px(&c, 2, 2), cs.button_hilight.argb(), "the dither");
        assert_eq!(px(&c, 3, 2), cs.button_face.argb());
        assert_eq!(
            px(&c, 5, 4),
            cs.button_dk_shadow.argb(),
            "the glyph's corner"
        );
        assert_eq!(px(&c, 20, 19), cs.button_dk_shadow.argb());
        assert_eq!(px(&c, 21, 12), cs.button_face.argb(), "past the glyph");
        let ink: Vec<i32> = (24..r.width)
            .filter(|&x| (5..18).any(|y| px(&c, x, y) == cs.button_text.argb()))
            .collect();
        if reprisekit::text::test_tahoma() {
            assert_eq!(ink.first(), Some(&25), "the title from x 25");
        }
        assert!(ink.last().is_some_and(|&x| x < r.width - 3), "cut inside");
        let c = draw(false);
        assert_eq!(px(&c, 0, 10), cs.button_hilight.argb(), "raised");
        assert_eq!(
            px(&c, 4, 3),
            cs.button_dk_shadow.argb(),
            "the glyph at (4, 3)"
        );
    }

    /// A held dialog button is found again by its dialog, wherever the row moved it.
    #[test]
    fn a_held_dialog_button_is_found_by_its_dialog() {
        let buttons = vec![vec![10], vec![]];
        let pins = vec![None, None];
        let dialogs = vec![None, Some(4)];
        let key = task_key(&buttons, &pins, &dialogs, 1).unwrap();
        assert_eq!(key, TaskKey::Dialog(4));
        assert_eq!(
            task_key(&buttons, &pins, &dialogs, 0),
            Some(TaskKey::Window(10))
        );
        let buttons = vec![vec![10], vec![20], vec![]];
        let dialogs = vec![None, None, Some(4)];
        assert_eq!(find_task(&buttons, &[], &dialogs, &key), Some(2));
        assert_eq!(
            find_task(&buttons, &[], &[None, None], &key),
            None,
            "closed"
        );
    }

    #[test]
    fn any_task_button_drags_once_moved() {
        assert!(press_drags(false, true));
        assert!(!press_drags(false, false), "not moved yet");
        assert!(press_drags(true, false), "a drag stays one");
    }

    #[test]
    fn jump_lists_follow_what_the_button_stands_for() {
        use std::path::PathBuf;
        let labels = |items: &[MenuItem]| -> Vec<String> {
            items
                .iter()
                .map(|i| match i {
                    MenuItem::Entry { label, .. } => label.clone(),
                    _ => "-".into(),
                })
                .collect()
        };
        let recent = [PathBuf::from("/h/b.txt"), PathBuf::from("/h/a.txt")];
        let titles = |n: usize| (1..=n).map(|k| format!("w{k}")).collect::<Vec<_>>();
        let (items, rows) = jump_list("Terminal", &recent, true, true, &titles(3), 1);
        assert_eq!(
            labels(&items),
            [
                "b.txt",
                "a.txt",
                "-",
                "w1",
                "w2",
                "w3",
                "-",
                "Terminal",
                "Unpin this program from taskbar",
                "Close all windows"
            ]
        );
        assert_eq!(rows[0], Some(JumpRow::Recent(recent[0].clone())));
        assert_eq!(rows[2], None);
        assert_eq!(
            rows[3..],
            [
                Some(JumpRow::Window(0)),
                Some(JumpRow::Window(1)),
                Some(JumpRow::Window(2)),
                None,
                Some(JumpRow::Launch),
                Some(JumpRow::Unpin),
                Some(JumpRow::Close)
            ]
        );
        let (items, rows) = jump_list("Terminal", &[], true, true, &[], 4);
        assert_eq!(
            labels(&items),
            ["Terminal", "Unpin this program from taskbar"]
        );
        assert_eq!(rows, [Some(JumpRow::Launch), Some(JumpRow::Unpin)]);
        let (items, rows) = jump_list("Editor", &[], false, true, &titles(1), 1);
        assert_eq!(
            labels(&items),
            ["Editor", "Pin this program to taskbar", "Close window"]
        );
        assert_eq!(rows[1], Some(JumpRow::Pin));
        assert!(matches!(items[1], MenuItem::Entry { enabled: true, .. }));
        let (items, _) = jump_list("Editor", &[], false, false, &titles(1), 1);
        assert!(matches!(items[1], MenuItem::Entry { enabled: false, .. }));
        let (items, rows) = jump_list("Editor", &[], false, true, &titles(2), 4);
        assert_eq!(
            labels(&items)[4..],
            [
                "Pin this program to taskbar",
                "Move to desktop",
                "Close all windows"
            ]
        );
        assert_eq!(rows[5], Some(JumpRow::MoveToMenu));
        assert!(matches!(items[5], MenuItem::Entry { submenu: true, .. }));
    }

    #[test]
    fn a_window_list_moves_the_group_only_with_several_desktops() {
        use WindowListRow::*;
        let titles = ["a", "b"].map(String::from);
        let (items, rows) = window_list(&titles, 4);
        assert_eq!(
            rows,
            [
                Some(Window(0)),
                Some(Window(1)),
                None,
                Some(Minimize),
                Some(MoveToMenu),
                Some(Close)
            ]
        );
        assert!(
            matches!(&items[4], MenuItem::Entry { submenu: true, label, .. } if label == "Move to desktop")
        );
        let (_, rows) = window_list(&titles, 1);
        assert_eq!(rows[3..], [Some(Minimize), Some(Close)]);
    }

    #[test]
    fn a_group_click_brings_up_the_latest_window_then_steps_through() {
        let group = [10, 20, 30];
        let recency = [30, 10, 20, 99];
        assert_eq!(group_click_pick(&group, &recency, Some(99)), Some(20));
        assert_eq!(group_click_pick(&group, &recency, None), Some(20));
        assert_eq!(group_click_pick(&group, &recency, Some(20)), Some(30));
        assert_eq!(group_click_pick(&group, &recency, Some(30)), Some(10));
        assert_eq!(group_click_pick(&group, &[], None), Some(10));
        assert_eq!(group_click_pick::<i32>(&[], &recency, None), None);
    }

    #[test]
    fn group_name_takes_the_last_dotted_segment() {
        assert_eq!(group_name("org.gnome.Nautilus"), "Nautilus");
        assert_eq!(group_name("Alacritty"), "Alacritty");
        assert_eq!(group_name(""), "Window");
    }

    #[test]
    fn move_button_lands_in_the_slot_and_ignores_no_ops() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        assert!(!move_button(&mut v, 1, 1));
        assert!(!move_button(&mut v, 1, 2));
        assert!(move_button(&mut v, 0, 3));
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        assert!(move_button(&mut v, 3, 0));
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        assert!(!move_button(&mut v, 9, 0));
        assert!(!move_button(&mut v, 0, 9));
    }
}
