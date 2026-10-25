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
        attention: reprisekit::taskbar::Attention::None,
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
mod tests;
