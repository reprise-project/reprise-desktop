//! Desktop notifications: a tray balloon or a stack of sheets, the rest queued, and the history
//! popup. One stays open while shown, queued or in the history, so its actions still run there.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisekit::anim::{Ease, Tween};
use reprisekit::controls::notifysheet::{
    RISE, SheetContent, SheetGlass, SheetIcons, SheetLayout, SheetOpening, WarmJob, WarmLayers,
    access_sheet, build_warm, chip_layout, draw_chip, drip_bead, drip_bead_at, drip_bead_share,
    drip_corner, drip_ms, drip_pose, has_sheets, open_layout, opening_shows_close, place_stack,
    pose, sheet_click, sheet_hit, sheet_layout,
};
use reprisekit::controls::tooltip::{
    NOTIFY_BUTTONS, NotifyHit, NotifyLayout, NotifyParts, NotifyReply, NotifySays, ReplyEvent,
    ReplyField, ReplyKey, access_parts, draw_notify_balloon, notify_hit, notify_layout,
    place_notify,
};
use reprisekit::glass::notify::WELL_CORNER;
use reprisekit::icon::IconSource;
use reprisekit::style::{NotifyLook, SheetShape};
use reprisekit::taskbar::Edge;
use reprisekit::textedit::{Caret, EditKey, TextEdit};
use reprisescene::{BufferId, Input, NodeId, TreeId};
use smithay::reexports::calloop::{
    RegistrationToken, channel,
    timer::{TimeoutAction, Timer},
};

use crate::caret::Editor;
use crate::compositor::{DeskScene, SceneOwner, State};
use crate::config::EffectsConfig;
use crate::dialogview::DialogKey;
use crate::glass::{Backdrop, GlassCtx, Pane, PaneShape};
use crate::input::BTN_LEFT;
use crate::notifylog::Log;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::startmenu::MenuItem;
use crate::text::TextRenderer;
use crate::theme::{Color, Theme};
use crate::tray::notify::{DEFAULT_ACTION, INLINE_REPLY, Urgency};
use crate::tray::{Notification, TrayCommand};

/// Default and clamp for how long a balloon stays up (ms). A request of 0 ("never") is held to
/// `MAX_MS`; only a critical notification stays until dealt with.
const DEFAULT_MS: i32 = 7000;
const MIN_MS: i32 = 3000;
const MAX_MS: i32 = 20000;

/// An open reply row with no key or pointer in the balloon for this long collapses, and the balloon
/// times out as usual so the queue behind it moves.
const REPLY_IDLE: Duration = Duration::from_secs(120);

/// How often a sheet's app line ("2 min ago") is redrawn.
const AGE_TICK: Duration = Duration::from_secs(60);

const NOTIFY_TICK: Duration = Duration::from_millis(16);

/// Logical px.
const TINT_PX: i32 = 48;

/// freedesktop `NotificationClosed` reason codes.
pub const REASON_EXPIRED: u32 = 1;
pub const REASON_DISMISSED: u32 = 2;
pub const REASON_CLOSED: u32 = 3;
pub const REASON_UNDEFINED: u32 = 4;

/// Balloons waiting behind the ones shown; each holds the screen at least `MIN_MS`, so a flood must
/// not park for hours.
pub const MAX_QUEUED: usize = 8;

/// How many balloons one program raises within `BURST_WINDOW` before the next folds them into one
/// "N new from App" balloon.
pub const BURST_AFTER: usize = 3;
pub const BURST_WINDOW: Duration = Duration::from_secs(10);
const BURST_LINES: usize = 3;

#[derive(Clone, Debug)]
enum Item {
    Note(Notification),
    Burst(Burst),
}

#[derive(Clone, Debug, Default)]
struct Burst {
    app: String,
    icon: Vec<IconSource>,
    app_icon: Vec<IconSource>,
    /// Oldest first.
    ids: Vec<u32>,
    /// The newest summaries, newest first.
    lines: VecDeque<String>,
}

impl Burst {
    fn new(app: &str) -> Burst {
        Burst {
            app: app.to_string(),
            ..Burst::default()
        }
    }

    fn add(&mut self, n: &Notification) {
        if self.icon.is_empty() {
            self.icon = n.balloon_icon().to_vec();
        }
        if self.app_icon.is_empty() {
            self.app_icon = n.app_icon.clone();
        }
        self.ids.push(n.id);
        let line = if n.summary.is_empty() {
            n.body.lines().next().unwrap_or_default()
        } else {
            &n.summary
        };
        self.lines.push_front(line.to_string());
        self.lines.truncate(BURST_LINES);
    }

    fn title(&self) -> String {
        format!("{} new from {}", self.ids.len(), self.app)
    }
}

impl Item {
    fn note(&self) -> Option<&Notification> {
        match self {
            Item::Note(n) => Some(n),
            Item::Burst(_) => None,
        }
    }

    fn is(&self, id: u32) -> bool {
        self.note().is_some_and(|n| n.id == id)
    }

    fn burst_of(&self, app: &str) -> bool {
        matches!(self, Item::Burst(b) if b.app == app)
    }

    fn icon(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => n.balloon_icon(),
            Item::Burst(b) => &b.icon,
        }
    }

    fn app_icon(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => &n.app_icon,
            Item::Burst(b) => &b.app_icon,
        }
    }

    fn has_app_icon(&self) -> bool {
        !self.app_icon().is_empty()
    }

    fn image(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => &n.image,
            Item::Burst(_) => &[],
        }
    }
}

struct Balloon {
    item: Item,
    /// The canvas's top-left for a balloon, the footprint's for a sheet; logical.
    at: (i32, i32),
    scale: i32,
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Option<Backdrop>,
    buffer: BufferId,
    shown_at: Instant,
    clock: Clock,
    motion: Motion,
    /// As last painted; hits and the "..." menu read it.
    drawn: Option<Drawn>,
    /// `motion.open` as last painted.
    open_drawn: f64,
    /// The glass under the sheet at rest, logical px from `glass_tree`.
    pane: Option<Pane>,
    held: Option<(NotifyHit, bool)>,
    hot: Option<NotifyHit>,
    reply: Option<ReplyField>,
    pointer_in: bool,
    /// Counts the balloons shown, so each is a new node to a screen reader.
    serial: u64,
    /// `None` until read under a look with sheets.
    tint: Option<Option<Color>>,
    /// A Droplet's opening layers while it opens.
    opening: SheetOpening,
    /// The glass bead a Droplet drips from while it arrives.
    bead: Option<Bead>,
    /// The last warm job sent for `opening`; a result for an earlier one is dropped.
    warm_gen: u64,
    glass: SheetGlass,
    /// What the buffer shows; `None` redraws it on the next paint.
    painted: Option<Painted>,
}

/// `body` and `size` are logical, `body` from the canvas's top-left.
struct Bead {
    buffer: BufferId,
    body: Rect,
    size: (i32, i32),
}

impl Balloon {
    /// Where the pointer finds it at `now`: `at` moved by the slide still running, logical.
    fn hit_at(&self, now: Instant) -> (i32, i32) {
        (
            self.at.0,
            self.at.1 + self.motion.offset(now).round() as i32,
        )
    }
}

/// A balloon's time on screen: running toward `due`, or paused with `left` to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clock {
    due: Option<Instant>,
    left: Option<Duration>,
}

impl Clock {
    /// `None` never runs out.
    fn start(ms: Option<i32>, now: Instant) -> Clock {
        Clock {
            due: ms
                .filter(|&ms| ms > 0)
                .map(|ms| now + Duration::from_millis(ms as u64)),
            left: None,
        }
    }

    fn pause(&mut self, now: Instant) {
        if let Some(due) = self.due.take() {
            self.left = Some(due.saturating_duration_since(now));
        }
    }

    fn resume(&mut self, now: Instant) {
        if let Some(left) = self.left.take() {
            self.due = Some(now + left);
        }
    }

    fn due(&self) -> Option<Instant> {
        self.due
    }
}

/// A sheet's way in, out and between slots, and a Droplet's opening.
#[derive(Clone, Copy, Debug)]
struct Motion {
    /// 0 hidden, 1 at rest.
    k: Tween,
    /// Logical px in y from the rest position.
    offset: Tween,
    /// 0 collapsed, 1 expanded.
    open: Tween,
    /// `k` runs a Droplet's arrival, posed by `drip_pose`.
    drip: bool,
}

/// A Droplet opens while the pointer is on it or its reply row is open.
fn open_target(hovered: bool, replying: bool) -> f64 {
    if hovered || replying { 1.0 } else { 0.0 }
}

impl Motion {
    fn arrive(e: &EffectsConfig, now: Instant) -> Motion {
        let ms = if e.opens() { e.window_open_ms } else { 0 };
        Motion {
            k: Tween::new(0.0, 1.0, now, ms, Ease::OutCubic),
            ..Motion::settled_at(now)
        }
    }

    /// A Droplet's arrival; the overshoot lives in `drip_pose`, so `k` runs linear.
    fn drip(e: &EffectsConfig, now: Instant) -> Motion {
        let ms = if e.opens() {
            drip_ms(e.window_open_ms)
        } else {
            0
        };
        Motion {
            k: Tween::new(0.0, 1.0, now, ms, Ease::Linear),
            drip: true,
            ..Motion::settled_at(now)
        }
    }

    fn settled_at(now: Instant) -> Motion {
        Motion {
            k: Tween::new(1.0, 1.0, now, 0, Ease::OutCubic),
            offset: Tween::new(0.0, 0.0, now, 0, Ease::OutCubic),
            open: Tween::new(0.0, 0.0, now, 0, Ease::OutCubic),
            drip: false,
        }
    }

    fn leave(&mut self, e: &EffectsConfig, now: Instant) {
        let ms = if e.closes() { e.window_close_ms } else { 0 };
        self.k.retarget(0.0, now, ms);
        self.drip = false;
        let open = self.open.value(now);
        self.open = Tween::new(open, open, now, 0, Ease::OutCubic);
    }

    /// Opens toward `to` over the window open or close time; false when already headed there.
    fn aim_open(&mut self, to: f64, e: &EffectsConfig, now: Instant) -> bool {
        if self.open.target() == to {
            return false;
        }
        let ms = match (to > self.open.value(now), e.opens(), e.closes()) {
            (true, true, _) => e.window_open_ms,
            (false, _, true) => e.window_close_ms,
            _ => 0,
        };
        self.open.retarget(to, now, ms);
        true
    }

    /// The rest moved by `-dy`: the sheet starts where it was and eases to its new rest.
    fn slide_by(&mut self, dy: f64, now: Instant, ms: u32) {
        self.offset = Tween::new(self.offset(now) + dy, 0.0, now, ms, Ease::OutCubic);
    }

    fn offset(&self, now: Instant) -> f64 {
        self.offset.value(now)
    }

    /// Arrived and at its slot; a Droplet may still be opening.
    fn placed(&self, now: Instant) -> bool {
        self.k.settled(now) && self.offset.settled(now)
    }

    fn settled(&self, now: Instant) -> bool {
        self.placed(now) && self.open.settled(now)
    }
}

/// A sheet's slot moved from `was` to `now_at`, each (top, height) logical: it slides by the move
/// of its anchored edge, the top in a stack growing down, else the bottom.
fn follow_slot(
    m: &mut Motion,
    was: (i32, i32),
    now_at: (i32, i32),
    grows_down: bool,
    now: Instant,
    ms: u32,
) {
    let edge = |(y, h): (i32, i32)| if grows_down { y } else { y + h };
    let dy = edge(was) - edge(now_at);
    if dy != 0 {
        m.slide_by(f64::from(dy), now, ms);
    }
}

/// The "+N more" chip past a full stack of sheets; a press opens the history.
struct Chip {
    tree: TreeId,
    buffer: BufferId,
    /// Logical.
    at: (i32, i32),
    size: (i32, i32),
    more: usize,
    hot: bool,
}

#[derive(Default)]
pub struct NotifyView {
    /// Newest first, nearest the tray.
    stack: Vec<Balloon>,
    leaving: Vec<Balloon>,
    chip: Option<Chip>,
    queue: VecDeque<Item>,
    recent: VecDeque<(String, Instant)>,
    /// Text typed into reply rows that collapsed idle, by notification id; Reply brings it back.
    drafts: Vec<(u32, ReplyField)>,
    /// The `default` action of the compositor's own notice; any other notice clears it.
    own_click: Option<Vec<OsString>>,
    timer: Option<RegistrationToken>,
    age_timer: Option<RegistrationToken>,
    tick_timer: Option<RegistrationToken>,
    /// The bar edge the stack was last placed by.
    edge: Option<Edge>,
    /// A sheet or the chip is under the pointer, drawn or at rest, as of its last move, press or
    /// paint: a sheet sliding into a closed one's slot keeps the stack held.
    pointer_on: bool,
    /// [`warm_openings`] waits for the loop to go idle.
    warm_queued: bool,
    /// The opening layers' painter thread, started by the first warm job, and its result source.
    warmer: Option<std::sync::mpsc::Sender<(u64, u64, WarmJob)>>,
    warm_source: Option<RegistrationToken>,
    pub log: Log,
    /// The history popup, while open.
    pub center: Option<crate::notifycenter::CenterView>,
    /// A wheel's sub-notch remainder over the history popup.
    pub center_wheel: i32,
    shown: u64,
}

/// How many notifications show at once: a stack of sheets, else one balloon.
pub fn visible_cap(sheets: bool) -> usize {
    if sheets { 3 } else { 1 }
}

fn sheets(state: &State) -> bool {
    has_sheets(&state.config.theme)
}

fn cap(state: &State) -> usize {
    visible_cap(sheets(state))
}

/// The queue's head while `len` shown and those taken stay under `cap`.
fn fill(len: usize, cap: usize, queue: &mut VecDeque<Item>) -> Vec<Item> {
    let mut came = Vec::new();
    while len + came.len() < cap
        && let Some(item) = queue.pop_front()
    {
        came.push(item);
    }
    came
}

fn slot_of(items: &[&Item], id: u32) -> Option<usize> {
    items.iter().position(|i| i.is(id))
}

/// The stack notes from `app` a new burst takes: all but one with an open reply (the bool).
fn fold_from(items: &[(&Item, bool)], app: &str) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, (item, replying))| !replying && item.note().is_some_and(|n| n.app == app))
        .map(|(i, _)| i)
        .collect()
}

fn chip_count(queued: usize, cap: usize) -> Option<usize> {
    (queued > 0 && cap > 1).then_some(queued)
}

fn take_all<T>(stack: &mut Vec<T>, leaving: &mut Vec<T>) -> Vec<T> {
    stack.drain(..).chain(leaving.drain(..)).collect()
}

/// The balloon with the reply row open; at most one has it.
fn replying(v: &NotifyView) -> Option<usize> {
    v.stack.iter().position(|b| b.reply.is_some())
}

fn index_of(v: &NotifyView, serial: u64) -> Option<usize> {
    v.stack.iter().position(|b| b.serial == serial)
}

fn hovered(v: &NotifyView) -> bool {
    v.pointer_on && !v.stack.is_empty()
}

/// Balloon `i`'s clock runs `ms` from now, held while the pointer is on a stack of sheets.
fn restart(v: &mut NotifyView, i: usize, ms: Option<i32>, sheets: bool) {
    let now = Instant::now();
    let paused = sheets && hovered(v);
    let Some(b) = v.stack.get_mut(i) else {
        return;
    };
    b.clock = Clock::start(ms, now);
    if paused {
        b.clock.pause(now);
    }
}

fn reply_idle_ms() -> Option<i32> {
    Some(REPLY_IDLE.as_millis() as i32)
}

/// Holds every clock while the pointer is on a stack of sheets, else lets them run; true on a change.
fn hold_clocks(state: &mut State) -> bool {
    let hold = sheets(state) && hovered(&state.notify);
    let now = Instant::now();
    let mut moved = false;
    for b in &mut state.notify.stack {
        let was = b.clock;
        if hold {
            b.clock.pause(now);
        } else {
            b.clock.resume(now);
        }
        moved |= b.clock != was;
    }
    moved
}

/// Arms `timer` for the earliest clock in the stack.
fn rearm(state: &mut State) {
    if let Some(t) = state.notify.timer.take() {
        state.handle.remove(t);
    }
    let Some(next) = state
        .notify
        .stack
        .iter()
        .filter_map(|b| b.clock.due())
        .min()
    else {
        return;
    };
    let wait = next.saturating_duration_since(Instant::now());
    match state
        .handle
        .insert_source(Timer::from_duration(wait), |_, _, state| {
            state.notify.timer = None;
            due(state);
            TimeoutAction::Drop
        }) {
        Ok(t) => state.notify.timer = Some(t),
        Err(e) => log::warn!("notification timer: {}", e.error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    Expire,
    /// The reply row has sat untouched for `REPLY_IDLE`.
    Collapse,
    /// The reply row is open with the pointer in the balloon: another `REPLY_IDLE`.
    Wait,
}

fn due_step(replying: bool, pointer_in: bool) -> Due {
    match (replying, pointer_in) {
        (false, _) => Due::Expire,
        (true, false) => Due::Collapse,
        (true, true) => Due::Wait,
    }
}

fn due(state: &mut State) {
    let now = Instant::now();
    let ripe: Vec<u64> = state
        .notify
        .stack
        .iter()
        .filter(|b| b.clock.due().is_some_and(|d| d <= now))
        .map(|b| b.serial)
        .collect();
    let sheets = sheets(state);
    for serial in ripe {
        let Some(i) = index_of(&state.notify, serial) else {
            continue;
        };
        let b = &state.notify.stack[i];
        match due_step(b.reply.is_some(), b.pointer_in) {
            Due::Expire => expired(state, i),
            Due::Collapse => collapse_idle(state),
            Due::Wait => restart(&mut state.notify, i, reply_idle_ms(), sheets),
        }
    }
    rearm(state);
}

/// A key or the pointer in the replying balloon: its reply row gets another `REPLY_IDLE`.
fn reply_touched(state: &mut State) {
    let sheets = sheets(state);
    if let Some(i) = replying(&state.notify) {
        restart(&mut state.notify, i, reply_idle_ms(), sheets);
        rearm(state);
    }
}

/// Whether a balloon is on screen.
pub fn balloon_up(state: &State) -> bool {
    !state.notify.stack.is_empty()
}

/// A transient notification closes with its balloon; any other stays open in the history.
fn expired(state: &mut State, i: usize) {
    if i >= state.notify.stack.len() {
        return;
    }
    let b = state.notify.stack.remove(i);
    let gone = b.item.note().filter(|n| n.transient).map(|n| n.id);
    leave(state, b);
    if let Some(id) = gone {
        crate::tray::notification_closed(state, id, REASON_EXPIRED);
    }
    display_next(state);
}

/// Seconds since the epoch, for the history's time column.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Notification `n` arrived: into the history unless it is transient, and a balloon unless Do Not
/// Disturb holds it back or the history is open, where it shows as a row instead. A replace swaps
/// in place, keeping a reply being typed.
pub fn show(state: &mut State, n: Notification) {
    let looking = crate::notifycenter::is_open(state);
    if !n.transient {
        for id in state.notify.log.record(&n, now(), looking) {
            drop_balloon(state, id);
            crate::tray::notification_closed(state, id, REASON_EXPIRED);
        }
    }
    if looking || !state.notify.log.shows_balloon(&n) {
        drop_balloon(state, n.id);
        if n.transient {
            crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
        }
        changed(state);
        return;
    }
    let shown: Vec<&Item> = state.notify.stack.iter().map(|b| &b.item).collect();
    if let Some(i) = slot_of(&shown, n.id) {
        replace(state, i, n);
        changed(state);
        return;
    }
    let queued = state.notify.queue.iter_mut().find(|q| q.is(n.id));
    if let Some(slot) = queued {
        *slot = Item::Note(n);
        changed(state);
        return;
    }
    if in_burst(&state.notify, n.id) {
        changed(state);
        return;
    }
    if foldable(&n) {
        let v = &mut state.notify;
        let due = arrived(&mut v.recent, &n.app, Instant::now());
        let open = v.queue.iter().any(|q| q.burst_of(&n.app))
            || v.stack.iter().any(|b| b.item.burst_of(&n.app));
        if due || open {
            fold(state, n);
            changed(state);
            return;
        }
    }
    for id in enqueue(&mut state.notify.queue, Item::Note(n)) {
        if state.notify.log.get(id).is_none() {
            crate::tray::notification_closed(state, id, REASON_UNDEFINED);
        }
    }
    display_next(state);
    changed(state);
}

/// Balloon `i` takes `n` in its slot: its clock starts over, with no new arrival, and a reply
/// being typed stays when `n` still offers one.
fn replace(state: &mut State, i: usize, n: Notification) {
    let sheets = sheets(state);
    let ms = timeout_ms(&n);
    let scale = state.notify.stack[i].scale;
    let tint = sheet_tint(&state.config.theme, &n.app_icon, scale);
    let v = &mut state.notify;
    v.shown += 1;
    let serial = v.shown;
    let focused = renew(&mut v.stack[i], n, serial, tint);
    restart(
        v,
        i,
        if focused.is_some() {
            reply_idle_ms()
        } else {
            ms
        },
        sheets,
    );
    if focused == Some(true) {
        state.restart_caret();
    }
    paint(state);
}

/// `b` shows `n` as new, on the same tree; returns its kept reply row's focus.
fn renew(
    b: &mut Balloon,
    n: Notification,
    serial: u64,
    tint: Option<Option<Color>>,
) -> Option<bool> {
    if n.reply.is_none() {
        b.reply = None;
    }
    b.item = Item::Note(n);
    b.tint = tint;
    b.shown_at = Instant::now();
    b.serial = serial;
    b.held = None;
    b.painted = None;
    b.reply.as_ref().map(|r| r.focused)
}

fn foldable(n: &Notification) -> bool {
    !n.app.is_empty() && n.id != OWN_ID && n.urgency != Urgency::Critical
}

fn in_burst(v: &NotifyView, id: u32) -> bool {
    let has = |i: &Item| matches!(i, Item::Burst(b) if b.ids.contains(&id));
    v.stack.iter().any(|b| has(&b.item)) || v.queue.iter().any(has)
}

fn arrived(recent: &mut VecDeque<(String, Instant)>, app: &str, at: Instant) -> bool {
    while recent
        .front()
        .is_some_and(|(_, t)| at.duration_since(*t) > BURST_WINDOW)
    {
        recent.pop_front();
    }
    recent.push_back((app.to_string(), at));
    recent.iter().filter(|(a, _)| a == app).count() > BURST_AFTER
}

/// Fold `n` into its program's burst, shown or queued; else a new burst takes the program's notes
/// (but a reply being written), in the oldest shown one's slot or the first queued one's place.
fn fold(state: &mut State, n: Notification) {
    let app = n.app.clone();
    let mut expired_ids = Vec::new();
    if n.transient {
        expired_ids.push(n.id);
    }
    let sheets = sheets(state);
    let v = &mut state.notify;
    if let Some(i) = v.stack.iter().position(|b| b.item.burst_of(&app)) {
        join_burst(&mut v.stack[i], &n, &state.config.theme);
        v.stack[i].painted = None;
        restart(v, i, Some(DEFAULT_MS), sheets);
        paint(state);
    } else if let Some(Item::Burst(b)) = v.queue.iter_mut().find(|q| q.burst_of(&app)) {
        b.add(&n);
    } else {
        let mut burst = Burst::new(&app);
        let shown: Vec<(&Item, bool)> = v
            .stack
            .iter()
            .map(|b| (&b.item, b.reply.is_some()))
            .collect();
        let on_screen = fold_from(&shown, &app);
        for &i in on_screen.iter().rev() {
            if let Item::Note(a) = &v.stack[i].item {
                burst.add(a);
                if a.transient {
                    expired_ids.push(a.id);
                }
            }
        }
        let (taken, at) = take_queued(&mut v.queue, &app);
        for q in &taken {
            burst.add(q);
            if q.transient {
                expired_ids.push(q.id);
            }
        }
        burst.add(&n);
        if !on_screen.is_empty() {
            let b = new_balloon(state, Item::Burst(burst));
            for gone in fold_in(&mut state.notify.stack, &on_screen, b) {
                leave(state, gone);
            }
            crate::autohideview::poke(state);
        } else {
            let q = &mut state.notify.queue;
            q.insert(at.unwrap_or(q.len()), Item::Burst(burst));
            while q.len() > MAX_QUEUED {
                q.pop_front();
            }
        }
        display_next(state);
    }
    for id in expired_ids {
        crate::tray::notification_closed(state, id, REASON_EXPIRED);
    }
}

/// Takes the `taken` indices (ascending) out of `stack` and puts `burst` in the slot of the last.
fn fold_in<T>(stack: &mut Vec<T>, taken: &[usize], burst: Option<T>) -> Vec<T> {
    let gone: Vec<T> = taken.iter().rev().map(|&i| stack.remove(i)).collect();
    if let (Some(b), Some(&oldest)) = (burst, taken.last()) {
        stack.insert(oldest + 1 - taken.len(), b);
    }
    gone
}

fn take_queued(queue: &mut VecDeque<Item>, app: &str) -> (Vec<Notification>, Option<usize>) {
    let mut taken = Vec::new();
    let mut at = None;
    let mut i = 0;
    queue.retain(|q| {
        let mine = q.note().is_some_and(|n| n.app == app);
        if mine {
            at.get_or_insert(i);
            if let Item::Note(n) = q {
                taken.push(n.clone());
            }
        } else {
            i += 1;
        }
        !mine
    });
    (taken, at)
}

fn changed(state: &mut State) {
    state.redraw_taskbars();
    crate::notifycenter::refresh(state);
}

/// The bus counts ids up from 1, so no client's notification shares this one.
const OWN_ID: u32 = u32::MAX;

const OWN_APP: &str = "Reprise";

/// Show a notice from the compositor itself, up for the longest time a balloon stays.
pub fn notice(state: &mut State, summary: &str, body: &str) {
    state.notify.own_click = None;
    show_own(state, summary, body, false);
}

/// `notice`, and a click on the balloon runs `argv`.
pub fn notice_opening(state: &mut State, summary: &str, body: &str, argv: Vec<OsString>) {
    state.notify.own_click = Some(argv);
    show_own(state, summary, body, true);
}

fn show_own(state: &mut State, summary: &str, body: &str, opens: bool) {
    let mut n = Notification::new(OWN_ID, summary.to_string(), body.to_string());
    n.app = OWN_APP.to_string();
    n.expire_timeout = 0;
    if opens {
        n.actions = vec![(DEFAULT_ACTION.to_string(), String::new())];
    }
    show(state, n);
}

/// End notification `id` with `reason`: its balloon goes, it is closed in the history (taken out of
/// it for `REASON_CLOSED`, the sender's own close), and `NotificationClosed` fires if it was still
/// open.
pub fn close(state: &mut State, id: u32, reason: u32) {
    let mut open = drop_balloon(state, id);
    let log = &mut state.notify.log;
    open |= if reason == REASON_CLOSED {
        log.remove(id)
    } else {
        log.close(id)
    };
    if open {
        crate::tray::notification_closed(state, id, reason);
    }
    changed(state);
}

/// Take notification `id` out of the history (Delete on its row): its balloon goes, and it is
/// dismissed if it was still open.
pub fn forget(state: &mut State, id: u32) {
    let open = drop_balloon(state, id) | state.notify.log.remove(id);
    if open {
        crate::tray::notification_closed(state, id, REASON_DISMISSED);
    }
    changed(state);
}

/// Takes `id`'s balloon down without closing the notification; true when it had one of its own.
fn drop_balloon(state: &mut State, id: u32) -> bool {
    let v = &mut state.notify;
    if let Some(i) = v.stack.iter().position(|b| b.item.is(id)) {
        let b = v.stack.remove(i);
        leave(state, b);
        display_next(state);
        return true;
    }
    let holds = |b: &Balloon| matches!(&b.item, Item::Burst(b) if b.ids.contains(&id));
    if let Some(i) = v.stack.iter().position(holds) {
        let mut empty = false;
        if let Item::Burst(b) = &mut v.stack[i].item {
            b.ids.retain(|&i| i != id);
            empty = b.ids.is_empty();
        }
        if empty {
            let b = v.stack.remove(i);
            leave(state, b);
            display_next(state);
        } else {
            paint(state);
        }
        return false;
    }
    let mut own = false;
    let before = v.queue.len();
    v.queue.retain_mut(|q| match q {
        Item::Note(n) => {
            own |= n.id == id;
            n.id != id
        }
        Item::Burst(b) => {
            b.ids.retain(|&i| i != id);
            !b.ids.is_empty()
        }
    });
    if v.queue.len() != before {
        paint(state);
    }
    own
}

/// Run action `key` of notification `id`, from its balloon, its "..." menu or the history:
/// `ActivationToken` fires with a trusted token the program can raise its window with, then
/// `ActionInvoked`; the sender's next window may take focus, and the notification closes unless it
/// is resident.
pub fn invoke(state: &mut State, id: u32, key: &str) {
    if key == INLINE_REPLY {
        start_reply(state, id);
        return;
    }
    let v = &state.notify;
    let on_screen = v
        .stack
        .iter()
        .filter_map(|b| b.item.note())
        .find(|n| n.id == id && n.has_action(key));
    let logged = v.log.get(id).filter(|e| e.can_invoke(key)).map(|e| &e.n);
    let Some(n) = on_screen.or(logged) else {
        return;
    };
    let (resident, pid) = (n.resident, n.pid);
    if id == OWN_ID {
        if let Some(argv) = state.notify.own_click.clone() {
            state.spawn_argv_activated(&argv);
        }
    } else {
        if let Some(pid) = pid {
            crate::mapfocus::grant(pid);
        }
        let key = key.to_string();
        let token = Some(state.new_activation_token());
        crate::tray::send(state, TrayCommand::ActionInvoked { id, key, token });
    }
    if resident {
        drop_balloon(state, id);
        changed(state);
    } else {
        close(state, id, REASON_DISMISSED);
    }
}

/// Open `id`'s reply row on its balloon, another open row going to `drafts`; from the history it
/// gets a balloon of its own, a full stack's newest going back to the queue.
fn start_reply(state: &mut State, id: u32) {
    if let Some(i) = state.notify.stack.iter().position(|b| b.item.is(id)) {
        if replying(&state.notify).is_some_and(|r| r != i) {
            collapse_idle(state);
        }
        let field = take_draft(&mut state.notify.drafts, id);
        expand(state, i, field);
        return;
    }
    let Some(n) = state
        .notify
        .log
        .get(id)
        .filter(|e| e.can_invoke(INLINE_REPLY))
        .map(|e| e.n.clone())
    else {
        return;
    };
    crate::notifycenter::close(state);
    collapse_idle(state);
    drop_balloon(state, id);
    let room = cap(state) - 1;
    for b in overflow(&mut state.notify.stack, room, |_| false) {
        state.notify.queue.push_front(b.item.clone());
        leave(state, b);
    }
    let Some(b) = new_balloon(state, Item::Note(n)) else {
        paint(state);
        return;
    };
    insert(state, 0, b);
    let field = take_draft(&mut state.notify.drafts, id);
    expand(state, 0, field);
}

/// `id`'s draft, focused, or an empty focused field.
fn take_draft(drafts: &mut Vec<(u32, ReplyField)>, id: u32) -> ReplyField {
    let field = drafts
        .iter()
        .position(|(i, _)| *i == id)
        .map(|at| drafts.remove(at).1);
    ReplyField {
        focused: true,
        ..field.unwrap_or_default()
    }
}

fn expand(state: &mut State, i: usize, field: ReplyField) {
    let sheets = sheets(state);
    let replies = state
        .notify
        .stack
        .get(i)
        .is_some_and(|b| b.item.note().is_some_and(|n| n.reply.is_some()));
    if replies {
        let focused = field.focused;
        state.notify.stack[i].reply = Some(field);
        restart(&mut state.notify, i, reply_idle_ms(), sheets);
        if focused {
            state.restart_caret();
        }
    }
    paint(state);
}

fn collapse(state: &mut State) -> Option<ReplyField> {
    let sheets = sheets(state);
    let i = replying(&state.notify)?;
    let b = &mut state.notify.stack[i];
    let field = b.reply.take()?;
    let ms = b.item.note().and_then(timeout_ms);
    restart(&mut state.notify, i, ms, sheets);
    paint(state);
    Some(field)
}

/// The reply row sat idle: it collapses, any text typed kept for the next Reply on this
/// notification while the history holds it.
fn collapse_idle(state: &mut State) {
    let id = replying(&state.notify)
        .and_then(|i| state.notify.stack[i].item.note())
        .map(|n| n.id);
    let Some(field) = collapse(state) else {
        return;
    };
    let Some(id) = id.filter(|_| field.to_send().is_some()) else {
        return;
    };
    let v = &mut state.notify;
    v.drafts
        .retain(|(i, _)| *i != id && v.log.get(*i).is_some());
    v.drafts.push((
        id,
        ReplyField {
            focused: false,
            ..field
        },
    ));
}

/// Send `text` as the replying balloon's reply: `NotificationReplied` fires, the history marks it
/// replied, and the notification closes unless it is resident, whose balloon goes.
fn send_reply(state: &mut State, text: String) {
    let v = &state.notify;
    let Some((id, resident)) = replying(v)
        .and_then(|i| v.stack[i].item.note())
        .map(|n| (n.id, n.resident))
    else {
        return;
    };
    crate::tray::send(state, TrayCommand::Replied { id, text });
    state.notify.log.mark_replied(id);
    if resident {
        drop_balloon(state, id);
        changed(state);
    } else {
        close(state, id, REASON_DISMISSED);
    }
}

/// Whether the reply field has the keyboard.
pub fn reply_focused(state: &State) -> bool {
    reply_edit(state).is_some()
}

/// The reply field's text while it has the keyboard.
pub fn reply_edit(state: &State) -> Option<&TextEdit> {
    let v = &state.notify;
    let r = v.stack.get(replying(v)?)?.reply.as_ref()?;
    r.focused.then_some(&r.edit)
}

fn reply_field(v: &mut NotifyView) -> Option<&mut ReplyField> {
    let i = replying(v)?;
    v.stack[i].reply.as_mut()
}

/// A press the key route sent to the focused reply field: Enter sends, Escape collapses, an editing
/// key edits. Paste asks for the clipboard's text, which lands in `paste`.
pub fn key(state: &mut State, k: &DialogKey) {
    use crate::keys;
    let special = k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_Return | keys::KEY_KP_Enter => Some(ReplyKey::Enter),
        keys::KEY_Escape => Some(ReplyKey::Escape),
        _ => None,
    });
    let Some(key) = special.or_else(|| k.edit.clone().map(ReplyKey::Edit)) else {
        return;
    };
    if let ReplyKey::Edit(e) = &key {
        state.clip_edit_key(e);
        if matches!(e, EditKey::Paste | EditKey::Copy) {
            return;
        }
    }
    let Some(field) = reply_field(&mut state.notify) else {
        return;
    };
    let event = field.key(key);
    state.restart_caret();
    match event {
        ReplyEvent::None => {}
        ReplyEvent::Redraw => paint(state),
        ReplyEvent::Send(text) => send_reply(state, text),
        ReplyEvent::Collapse => {
            collapse(state);
        }
    }
    reply_touched(state);
}

/// Text from the clipboard into the reply field.
pub fn paste(state: &mut State, text: &str) {
    if let Some(field) = reply_field(&mut state.notify)
        && field.paste(text)
    {
        paint(state);
    }
    reply_touched(state);
}

/// A press anywhere off the balloons: the reply field lets go of the keyboard and stays open.
pub fn blur(state: &mut State) {
    if let Some(field) = reply_field(&mut state.notify)
        && std::mem::replace(&mut field.focused, false)
    {
        paint(state);
    }
}

/// Clear All in the history: every notification in it still open is dismissed, and the history
/// empties.
pub fn clear_history(state: &mut State) {
    for id in state.notify.log.clear() {
        drop_balloon(state, id);
        crate::tray::notification_closed(state, id, REASON_DISMISSED);
    }
    drop_leaving(state);
    changed(state);
}

/// A group's Clear in the history: every notification from `app` leaves it, those still open
/// dismissed.
pub fn clear_app(state: &mut State, app: &str) {
    for id in state.notify.log.clear_app(app) {
        drop_balloon(state, id);
        crate::tray::notification_closed(state, id, REASON_DISMISSED);
    }
    drop_leaving(state);
    changed(state);
}

/// Do Not Disturb on or off. Turning it on takes down the balloons it holds back, which stay open
/// in the history, and the sheets still leaving.
pub fn set_dnd(state: &mut State, on: bool) {
    state.notify.log.dnd = on;
    if on {
        let v = &mut state.notify;
        let log = &v.log;
        let held = |item: &Item| match item {
            Item::Burst(_) => true,
            Item::Note(n) => !log.shows_balloon(n) && !n.transient,
        };
        v.queue.retain(|q| !held(q));
        let (gone, kept): (Vec<Balloon>, Vec<Balloon>) = std::mem::take(&mut v.stack)
            .into_iter()
            .partition(|b| held(&b.item));
        v.stack = kept;
        for b in gone {
            destroy(state, b);
        }
        drop_leaving(state);
        display_next(state);
    }
    log::info!("do not disturb {}", if on { "on" } else { "off" });
    changed(state);
}

/// Every balloon and sheet comes down at once with the chip, no exit motion; returns their items.
fn take_down(state: &mut State) -> Vec<Item> {
    let v = &mut state.notify;
    let all = take_all(&mut v.stack, &mut v.leaving);
    let mut items = Vec::with_capacity(all.len());
    for b in all {
        items.push(b.item.clone());
        destroy(state, b);
    }
    paint(state);
    items
}

fn close_transient(state: &mut State, items: &[Item]) {
    for n in items.iter().filter_map(Item::note).filter(|n| n.transient) {
        crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
    }
}

/// The history opened over the balloons' corner: every balloon comes down.
pub fn hide_balloons(state: &mut State) {
    let mut gone: Vec<Item> = state.notify.queue.drain(..).collect();
    gone.extend(take_down(state));
    close_transient(state, &gone);
}

/// A notification's sender left the bus: its actions have nobody to run.
pub fn sender_gone(state: &mut State, sender: &str) {
    state.notify.log.sender_gone(sender);
    crate::notifycenter::refresh(state);
}

/// The balloon under layout point (`x`, `y`) where it is drawn now: its index, layout, the part
/// hit and its device x in the canvas.
fn stack_hit(state: &State, x: f64, y: f64) -> Option<(usize, Drawn, NotifyHit, i32)> {
    let now = Instant::now();
    hit_in(&state.notify, x, y, |b| b.hit_at(now))
}

/// `stack_hit` with each balloon at `place`, logical.
fn hit_in(
    v: &NotifyView,
    x: f64,
    y: f64,
    place: impl Fn(&Balloon) -> (i32, i32),
) -> Option<(usize, Drawn, NotifyHit, i32)> {
    v.stack.iter().enumerate().find_map(|(i, b)| {
        let drawn = b.drawn.as_ref()?;
        let (ox, oy) = drawn.origin();
        let (ax, ay) = place(b);
        let lx = ((x - f64::from(ax)) * f64::from(b.scale)).floor() as i32 + ox;
        let ly = ((y - f64::from(ay)) * f64::from(b.scale)).floor() as i32 + oy;
        let (w, h) = drawn.size();
        if !(0..w).contains(&lx) || !(0..h).contains(&ly) {
            return None;
        }
        let hit = drawn.hit(lx, ly)?;
        Some((i, drawn.clone(), hit, lx))
    })
}

/// The chip's rect, logical, when the pointer at (`x`, `y`) is on it.
fn chip_hit(v: &NotifyView, x: f64, y: f64) -> Option<Rect> {
    let c = v.chip.as_ref()?;
    let r = Rect::new(c.at.0, c.at.1, c.size.0, c.size.1);
    r.contains(x.floor() as i32, y.floor() as i32).then_some(r)
}

/// A press of `button` on a balloon or the chip at layout point (`x`, `y`); true when the pointer
/// now holds a button. Any button but the left dismisses a balloon.
pub fn press(state: &mut State, x: f64, y: f64, button: u32) -> bool {
    let target = press_target(&mut state.notify, x, y, Instant::now());
    if hold_clocks(state) {
        rearm(state);
    }
    let Some((i, drawn, hit, lx)) = target else {
        if let Some(cell) = chip_hit(&state.notify, x, y)
            && let Some(output) = state.first_output()
        {
            crate::notifycenter::toggle(state, &output, cell);
        }
        return false;
    };
    reply_touched(state);
    let hit = if button == BTN_LEFT {
        hit
    } else {
        NotifyHit::Close
    };
    let n = match state.notify.stack[i].item.clone() {
        Item::Burst(burst) => {
            let b = state.notify.stack.remove(i);
            leave(state, b);
            if hit == NotifyHit::Close {
                display_next(state);
            } else {
                paint(state);
                crate::notifycenter::reveal(state, &burst.app);
            }
            return false;
        }
        Item::Note(n) => n,
    };
    let id = n.id;
    match hit {
        NotifyHit::Close => close(state, id, REASON_DISMISSED),
        NotifyHit::Body => {
            if n.has_action(DEFAULT_ACTION) {
                invoke(state, id, DEFAULT_ACTION);
            } else {
                close(state, id, REASON_DISMISSED);
            }
        }
        NotifyHit::Button(_) | NotifyHit::Send => {
            state.notify.stack[i].held = Some((hit, true));
            paint(state);
            return true;
        }
        NotifyHit::Field => {
            let theme = state.config.theme.scaled(lay_scale(state));
            if let Some(field) = state.notify.stack[i].reply.as_mut() {
                match &drawn {
                    Drawn::Balloon(lay) => field.click(&theme, &state.text, lay, lx),
                    Drawn::Sheet(lay) => sheet_click(lay, &theme, &state.text, field, lx),
                }
            }
            state.restart_caret();
            paint(state);
        }
        NotifyHit::More => open_more_menu(state, id),
    }
    false
}

/// The replying balloon's scale, else the newest's.
fn lay_scale(state: &State) -> i32 {
    let v = &state.notify;
    replying(v)
        .or((!v.stack.is_empty()).then_some(0))
        .map_or(1, |i| v.stack[i].scale)
}

/// The pointer moved, `over` the stack or not: the part under it lights (on a sheet, its close
/// button too) and sheets hold their clocks; crossing the replying balloon restarts `REPLY_IDLE`.
pub fn hover(state: &mut State, x: f64, y: f64, over: bool) {
    let (mut dirty, touched) = point_at(&mut state.notify, x, y, over, Instant::now());
    dirty |= aim_open(state);
    if hold_clocks(state) {
        rearm(state);
    }
    if dirty {
        paint(state);
    }
    if touched {
        reply_touched(state);
    }
}

/// `hover`'s marks on the stack and chip; returns whether to repaint and whether the pointer
/// crossed the replying balloon.
fn point_at(v: &mut NotifyView, x: f64, y: f64, over: bool, now: Instant) -> (bool, bool) {
    let under = over.then(|| hit_in(v, x, y, |b| b.hit_at(now))).flatten();
    let on_chip = over && under.is_none() && chip_hit(v, x, y).is_some();
    let (mut dirty, mut touched) = (false, false);
    for (i, b) in v.stack.iter_mut().enumerate() {
        let mine = under.as_ref().filter(|(j, ..)| *j == i);
        let hot = mine
            .filter(|(_, d, h, _)| *h != NotifyHit::Body || matches!(d, Drawn::Sheet(_)))
            .map(|(_, _, h, _)| *h);
        let was_in = std::mem::replace(&mut b.pointer_in, mine.is_some());
        if b.hot != hot {
            b.hot = hot;
            dirty = true;
        }
        touched |= b.reply.is_some() && (mine.is_some() || was_in);
    }
    if let Some(c) = v.chip.as_mut()
        && c.hot != on_chip
    {
        c.hot = on_chip;
        dirty = true;
    }
    v.pointer_on = under.is_some() || on_chip;
    (dirty, touched)
}

/// The still pointer at (`x`, `y`) holds the stack while a sheet is under it, drawn or at rest,
/// or the chip, and lets it go otherwise; true on a change.
fn hold_at(v: &mut NotifyView, x: f64, y: f64, now: Instant) -> bool {
    let on = hit_in(v, x, y, |b| b.hit_at(now)).is_some()
        || hit_in(v, x, y, |b| b.at).is_some()
        || chip_hit(v, x, y).is_some();
    std::mem::replace(&mut v.pointer_on, on) != on
}

/// `stack_hit` for a press, which takes the hold first. A close spot takes no press while its sheet
/// slides into place, sparing the sheet sliding up, or before an opening Droplet's close shows.
fn press_target(
    v: &mut NotifyView,
    x: f64,
    y: f64,
    now: Instant,
) -> Option<(usize, Drawn, NotifyHit, i32)> {
    hold_at(v, x, y, now);
    hit_in(v, x, y, |b| b.hit_at(now)).filter(|(i, _, hit, _)| {
        let m = &v.stack[*i].motion;
        *hit != NotifyHit::Close
            || m.placed(now) && (m.open.settled(now) || opening_shows_close(m.open.value(now)))
    })
}

fn held(v: &NotifyView) -> Option<(usize, NotifyHit, bool)> {
    v.stack
        .iter()
        .enumerate()
        .find_map(|(i, b)| b.held.map(|(part, down)| (i, part, down)))
}

/// The pointer moved while it holds a balloon button: the button shows pushed only while the
/// pointer is over it.
pub fn drag(state: &mut State, x: f64, y: f64) {
    let Some((i, part, down)) = held(&state.notify) else {
        return;
    };
    let over = stack_hit(state, x, y).is_some_and(|(j, _, h, _)| j == i && h == part);
    if over != down {
        state.notify.stack[i].held = Some((part, over));
        paint(state);
    }
}

/// The pointer let go of a balloon button: its action runs, or Send sends, if the release is over
/// it.
pub fn release(state: &mut State, x: f64, y: f64) {
    let Some((i, part, _)) = held(&state.notify) else {
        return;
    };
    state.notify.stack[i].held = None;
    let over = stack_hit(state, x, y).is_some_and(|(j, _, h, _)| j == i && h == part);
    paint(state);
    if !over {
        return;
    }
    let b = &state.notify.stack[i];
    match part {
        NotifyHit::Send => {
            if let Some(text) = b.reply.as_ref().and_then(ReplyField::to_send) {
                send_reply(state, text);
            }
        }
        NotifyHit::Button(k) => {
            let Some(n) = b.item.note() else {
                return;
            };
            let (id, key) = (n.id, n.button_actions().nth(k).map(|(k, _)| k.clone()));
            if let Some(key) = key {
                invoke(state, id, &key);
            }
        }
        _ => {}
    }
}

/// The pointer is taken away mid-press: a held balloon button lets go without acting.
pub fn cancel_press(state: &mut State) {
    if let Some((i, ..)) = held(&state.notify) {
        state.notify.stack[i].held = None;
        paint(state);
    }
}

/// Open the "..." button's menu: the actions past the balloon's buttons, its right edge at the
/// balloon's left so the balloon does not cover it.
fn open_more_menu(state: &mut State, id: u32) {
    let Some(b) = state.notify.stack.iter().find(|b| b.item.is(id)) else {
        return;
    };
    let (Some(n), Some(drawn)) = (b.item.note(), b.drawn.as_ref()) else {
        return;
    };
    let rest: Vec<(String, String)> = n.button_actions().skip(NOTIFY_BUTTONS).cloned().collect();
    let oy = drawn.origin().1;
    let more_y = drawn.parts().more.map_or(0, |m| (m.y - oy) / b.scale);
    let at = (b.at.0, b.at.1 + more_y);
    crate::notifycenter::open_actions_menu(state, id, rest, at, true);
}

/// Shows the queue's head in every free slot, nearest the tray as the queue is newer than the
/// stack, then repaints the stack and chip.
fn display_next(state: &mut State) {
    let cap = cap(state);
    loop {
        let came = fill(state.notify.stack.len(), cap, &mut state.notify.queue);
        if came.is_empty() {
            break;
        }
        for item in came {
            if let Some(b) = new_balloon(state, item) {
                insert(state, 0, b);
            }
        }
    }
    paint(state);
}

/// Returns the ids of the single balloons dropped past `MAX_QUEUED`; a burst's are in the history.
fn enqueue(queue: &mut VecDeque<Item>, item: Item) -> Vec<u32> {
    queue.push_back(item);
    let mut dropped = Vec::new();
    while queue.len() > MAX_QUEUED {
        if let Some(Item::Note(old)) = queue.pop_front() {
            dropped.push(old.id);
        }
    }
    dropped
}

/// `None` for a critical notification, which stays until dealt with.
fn timeout_ms(n: &Notification) -> Option<i32> {
    if n.urgency == Urgency::Critical {
        return None;
    }
    Some(match n.expire_timeout {
        t if t < 0 => DEFAULT_MS,
        0 => MAX_MS,
        t => t.clamp(MIN_MS, MAX_MS),
    })
}

fn button_labels(n: &Notification) -> Vec<&str> {
    n.button_actions()
        .map(|(k, l)| match (l.is_empty(), k.as_str()) {
            (false, _) => l.as_str(),
            (true, INLINE_REPLY) => "Reply",
            (true, _) => k.as_str(),
        })
        .collect()
}

/// What a balloon's buffer shows, but for a reply row, which always redraws.
#[derive(PartialEq)]
struct Painted {
    drawn: Drawn,
    open: f64,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
}

#[derive(Clone, PartialEq)]
enum Drawn {
    Balloon(NotifyLayout),
    Sheet(SheetLayout),
}

impl Drawn {
    /// The canvas, device px.
    fn size(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(l) => (l.width, l.height),
            Drawn::Sheet(l) => (l.width, l.height),
        }
    }

    /// `None` off a sheet: on its shadow.
    fn hit(&self, x: i32, y: i32) -> Option<NotifyHit> {
        match self {
            Drawn::Balloon(l) => Some(notify_hit(l, x, y)),
            Drawn::Sheet(l) => sheet_hit(l, x, y),
        }
    }

    fn parts(&self) -> NotifyParts {
        match self {
            Drawn::Balloon(l) => l.parts(),
            Drawn::Sheet(l) => l.parts.clone(),
        }
    }

    /// Where the tree sits in the canvas, device px: a sheet's tree is at its footprint.
    fn origin(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(_) => (0, 0),
            Drawn::Sheet(l) => (l.footprint.x, l.footprint.y),
        }
    }

    /// What the stack places, device px: a sheet's footprint, a balloon's canvas.
    fn footprint(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(l) => (l.width, l.height),
            Drawn::Sheet(l) => (l.footprint.width, l.footprint.height),
        }
    }
}

/// A sheet when the look has them, else a balloon; `when` is the sheet's app line time.
#[expect(clippy::too_many_arguments)]
fn lay_out(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    expanded: bool,
    when: &str,
) -> Drawn {
    let labels;
    let burst;
    let (app, title, body, actions, reply) = match item {
        Item::Note(n) => {
            labels = button_labels(n);
            let reply = n.reply.as_ref().filter(|_| replying).map(|r| NotifyReply {
                placeholder: &r.placeholder,
                send: &r.send,
            });
            (
                n.app.as_str(),
                n.summary.as_str(),
                n.body.as_str(),
                &labels[..],
                reply,
            )
        }
        Item::Burst(b) => {
            let lines: Vec<&str> = b.lines.iter().map(String::as_str).collect();
            burst = (b.title(), lines.join("\n"));
            (
                b.app.as_str(),
                burst.0.as_str(),
                burst.1.as_str(),
                &[][..],
                None,
            )
        }
    };
    let content = SheetContent {
        app,
        when,
        title,
        body,
        actions,
        reply,
        has_picture: !item.image().is_empty(),
        has_app_icon: item.has_app_icon(),
    };
    match sheet_layout(theme, text, shape, &content, expanded || replying) {
        Some(lay) => Drawn::Sheet(lay),
        None => {
            Drawn::Balloon(notify_layout(theme, text, title, body, actions, reply).on_edge(edge))
        }
    }
}

/// A Droplet's collapsed and expanded layouts; `None` for any other shape or a look without
/// sheets.
fn droplet_ends(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    when: &str,
) -> Option<[SheetLayout; 2]> {
    if shape != SheetShape::Droplet {
        return None;
    }
    let lay =
        |replying, expanded| lay_out(theme, text, item, replying, edge, shape, expanded, when);
    match (lay(false, false), lay(replying, true)) {
        (Drawn::Sheet(shut), Drawn::Sheet(open)) => Some([shut, open]),
        _ => None,
    }
}

/// `item` laid out `t` (0..1) of the way open. A Droplet sheet also returns the collapsed and
/// expanded layouts it opens between.
#[expect(clippy::too_many_arguments)]
fn lay_out_open(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    when: &str,
    t: f64,
) -> (Drawn, Option<[SheetLayout; 2]>) {
    let Some([shut, open]) = droplet_ends(theme, text, item, replying, edge, shape, when) else {
        return (
            lay_out(theme, text, item, replying, edge, shape, false, when),
            None,
        );
    };
    let lay = match t {
        t if t <= 0.0 => shut.clone(),
        t if t >= 1.0 => open.clone(),
        t => open_layout(&shut, &open, t),
    };
    (Drawn::Sheet(lay), Some([shut, open]))
}

/// The glass under a sheet, logical px from the glass tree's origin at the footprint.
fn sheet_glass(lay: &SheetLayout, look: &NotifyLook, scale: i32) -> Pane {
    let fp = lay.footprint;
    match lay.shape {
        SheetShape::Lozenge | SheetShape::Droplet => crate::glass::sheet_pane(
            (fp.width / scale, fp.height / scale),
            (look.corner)(lay.shape),
        ),
        SheetShape::Halo => {
            let logical = |r: Rect| {
                Rect::new(
                    (r.x - fp.x) / scale,
                    (r.y - fp.y) / scale,
                    r.width / scale,
                    r.height / scale,
                )
            };
            let (icon, sheet) = (logical(lay.icon), logical(lay.sheet));
            let well = Rect::new(icon.x, icon.y, icon.width, sheet.y - icon.y);
            Pane {
                shapes: vec![
                    PaneShape {
                        rect: well,
                        corner: &WELL_CORNER,
                        square_bottom: true,
                        square_top: false,
                    },
                    PaneShape {
                        rect: sheet,
                        corner: (look.corner)(lay.shape),
                        square_bottom: false,
                        square_top: false,
                    },
                ],
                holes: vec![],
            }
        }
    }
}

/// `None` for a look without sheets, `Some(None)` for a grey icon or none.
fn sheet_tint(theme: &Theme, app_icon: &[IconSource], scale: i32) -> Option<Option<Color>> {
    let look = theme.look.style().and_then(|st| st.notify)?;
    Some(crate::tray::closest(app_icon, TINT_PX * scale).and_then(|icon| (look.colour)(&icon)))
}

/// Points each Droplet's opening at its [`open_target`]; true when one turned.
fn aim_open(state: &mut State) -> bool {
    if !sheets(state) || state.config.glass.notification.shape() != SheetShape::Droplet {
        return false;
    }
    let (now, e) = (Instant::now(), state.config.effects);
    let mut turned = false;
    for b in &mut state.notify.stack {
        let to = open_target(b.pointer_in, b.reply.is_some());
        turned |= b.motion.aim_open(to, &e, now);
    }
    turned
}

/// A sheet's app line time: "now", then minutes, then hours.
fn relative(d: Duration) -> String {
    match d.as_secs() / 60 {
        0 => "now".to_string(),
        m @ 1..60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

/// Redraws the sheets' app lines every `AGE_TICK` while one is drawn.
fn arm_age(state: &mut State) {
    if state.notify.age_timer.is_some() {
        return;
    }
    match state
        .handle
        .insert_source(Timer::from_duration(AGE_TICK), |_, _, state| {
            paint(state);
            TimeoutAction::ToDuration(AGE_TICK)
        }) {
        Ok(t) => state.notify.age_timer = Some(t),
        Err(e) => log::warn!("notification age timer: {}", e.error),
    }
}

fn disarm_age(state: &mut State) {
    if let Some(t) = state.notify.age_timer.take() {
        state.handle.remove(t);
    }
}

/// A balloon for `item`, its clock started (and held if the pointer is on the sheets); `None`
/// with no output to place it on, where a transient notification closes.
fn new_balloon(state: &mut State, item: Item) -> Option<Balloon> {
    let Some(output) = state.first_output() else {
        // It stays open in the history unless it is transient.
        if let Some(n) = item.note().filter(|n| n.transient) {
            crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
        }
        return None;
    };
    let scale = state.draw_scale_at(Some(&output)).max(1);
    let ms = match &item {
        Item::Note(n) => timeout_ms(n),
        Item::Burst(_) => Some(DEFAULT_MS),
    };
    let now = Instant::now();
    let mut clock = Clock::start(ms, now);
    if sheets(state) && hovered(&state.notify) {
        clock.pause(now);
    }
    let e = &state.config.effects;
    let motion = match (sheets(state), state.config.glass.notification.shape()) {
        (false, _) => Motion::settled_at(now),
        (true, SheetShape::Droplet) => Motion::drip(e, now),
        (true, SheetShape::Lozenge | SheetShape::Halo) => Motion::arrive(e, now),
    };
    let tint = sheet_tint(&state.config.theme, item.app_icon(), scale);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.overlay);
    crate::glass::leave_out(scene, tree);
    scene.set_data(tree, Some(SceneOwner::Notification));
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    state.notify.shown += 1;
    Some(Balloon {
        item,
        at: (0, 0),
        scale,
        tree,
        glass_tree,
        backdrop: None,
        buffer,
        shown_at: now,
        clock,
        motion,
        drawn: None,
        open_drawn: 0.0,
        pane: None,
        held: None,
        hot: None,
        reply: None,
        pointer_in: false,
        serial: state.notify.shown,
        tint,
        opening: SheetOpening::default(),
        bead: None,
        warm_gen: 0,
        glass: SheetGlass::default(),
        painted: None,
    })
}

fn insert(state: &mut State, at: usize, b: Balloon) {
    let v = &mut state.notify;
    v.stack.insert(at.min(v.stack.len()), b);
    restack(&mut state.scene, &v.stack, &v.leaving, Instant::now());
    crate::autohideview::poke(state);
}

/// More balloons than the look shows (after an interface switch): the newest go back to the
/// queue's head, the replying one kept.
fn trim(state: &mut State) {
    let cap = cap(state);
    for b in overflow(&mut state.notify.stack, cap, |b| b.reply.is_some()) {
        state.notify.queue.push_front(b.item.clone());
        destroy(state, b);
    }
}

/// Takes the newest of `stack` (newest first) until `cap` are left, sparing those `keep` holds;
/// newest first. The queue then stays newer than the stack, which `display_next` relies on.
fn overflow<T>(stack: &mut Vec<T>, cap: usize, keep: impl Fn(&T) -> bool) -> Vec<T> {
    let mut gone = Vec::new();
    let mut i = 0;
    while stack.len() > cap && i < stack.len() {
        if keep(&stack[i]) {
            i += 1;
        } else {
            gone.push(stack.remove(i));
        }
    }
    gone
}

/// The bar, a balloon's size, the look or the outputs changed: the stack refills to the look's cap
/// at the tray's corner, or with no output left every balloon comes down.
pub fn reposition(state: &mut State) {
    if state.first_output().is_none() {
        let gone = take_down(state);
        close_transient(state, &gone);
        return;
    }
    trim(state);
    retint(&mut state.notify.stack, &state.config.theme);
    for b in &mut state.notify.stack {
        b.painted = None;
    }
    display_next(state);
}

/// An output went away: sheets still leaving go at once (`update_layout` has moved the stack).
pub fn output_removed(state: &mut State) {
    drop_leaving(state);
}

/// The look may have changed: a balloon shown under Classic or Blue takes its Glass tint. An icon
/// is decoded only for a balloon not yet tinted, as autohide repositions often.
fn retint(stack: &mut [Balloon], theme: &Theme) {
    let sheets = theme.look.style().and_then(|st| st.notify).is_some();
    for b in stack {
        if !sheets || b.tint.is_none() {
            b.tint = sheet_tint(theme, b.item.app_icon(), b.scale);
        }
    }
}

/// Burst `b` takes `n`; its first app icon tints it.
fn join_burst(b: &mut Balloon, n: &Notification, theme: &Theme) {
    if let Item::Burst(burst) = &mut b.item {
        if burst.app_icon.is_empty() {
            b.tint = sheet_tint(theme, &n.app_icon, b.scale);
        }
        burst.add(n);
    }
}

/// Lays out, places and draws every balloon and the chip, and arms the clocks and the app line
/// tick for what is drawn.
fn paint(state: &mut State) {
    hold_clocks(state);
    rearm(state);
    aim_open(state);
    if state.notify.stack.is_empty() {
        state.notify.pointer_on = false;
        drop_chip(state);
        disarm_age(state);
        return;
    }
    if !lay_stack(state, false) {
        return;
    }
    let p = state.pointer_location();
    if hold_at(&mut state.notify, p.x, p.y, Instant::now()) && hold_clocks(state) {
        rearm(state);
    }
    if replying(&state.notify).is_none() {
        state.ime_field_drawn(Editor::NotifyReply, None, (0, 0), 1);
    }
    let v = &state.notify;
    if v.stack
        .iter()
        .any(|b| matches!(b.drawn, Some(Drawn::Sheet(_))))
    {
        arm_age(state);
    } else {
        disarm_age(state);
    }
    state.redraw_all();
    crate::access::touch(state);
    arm_tick(state);
    warm_openings(state);
}

/// Lays out and places every balloon and the chip. All are drawn, or with `opening_only` just
/// the sheets whose opening moved since; false with no output.
fn lay_stack(state: &mut State, opening_only: bool) -> bool {
    let Some(output) = state.first_output() else {
        return false;
    };
    let p = state.bar_placement(&output);
    let area = crate::barplace::clear_of(state.usable_area(Some(&output)), &p);
    let shape = state.config.glass.notification.shape();
    let now = Instant::now();
    let laid: Vec<(Drawn, Option<[SheetLayout; 2]>, f64)> = state
        .notify
        .stack
        .iter()
        .map(|b| {
            let t = b.motion.open.value(now);
            let (d, ends) = lay_out_open(
                &state.config.theme.scaled(b.scale),
                &state.text,
                &b.item,
                b.reply.is_some(),
                p.edge,
                shape,
                &relative(b.shown_at.elapsed()),
                t,
            );
            (d, ends, t)
        })
        .collect();
    let chip_scale = state.draw_scale_at(Some(&output)).max(1);
    let chip = chip_count(state.notify.queue.len(), cap(state)).map(|more| {
        let theme = state.config.theme.scaled(chip_scale);
        let (w, h) = chip_layout(&theme, &state.text, more);
        (more, (w / chip_scale, h / chip_scale))
    });
    let footprints: Vec<(i32, i32)> = laid
        .iter()
        .zip(&state.notify.stack)
        .map(|((d, ..), b)| {
            let (w, h) = d.footprint();
            (w / b.scale, h / b.scale)
        })
        .collect();
    let (spots, chip_at) = place_stack(area, &footprints, chip.map(|c| c.1), p.edge);
    let e = state.config.effects;
    let slide_ms = if e.opens() { e.window_open_ms } else { 0 };
    let same_edge = state.notify.edge.replace(p.edge) == Some(p.edge);
    let opening = state
        .notify
        .stack
        .iter()
        .any(|b| !b.motion.open.settled(now));
    for (i, ((d, ends, t), spot)) in laid.into_iter().zip(spots).enumerate() {
        let b = &mut state.notify.stack[i];
        let at = match &d {
            Drawn::Balloon(lay) => place_notify(area, lay, b.scale, p.edge),
            Drawn::Sheet(_) => spot,
        };
        let sheet = matches!(b.drawn, Some(Drawn::Sheet(_)));
        if !opening
            && same_edge
            && let Some(old @ Drawn::Sheet(_)) = &b.drawn
            && b.at.0 == at.0
        {
            let span = |d: &Drawn| d.footprint().1 / b.scale;
            let (was, now_at) = ((b.at.1, span(old)), (at.1, span(&d)));
            follow_slot(
                &mut b.motion,
                was,
                now_at,
                p.edge == Edge::Top,
                now,
                slide_ms,
            );
        }
        if opening_only && sheet && b.open_drawn == t {
            b.at = at;
            continue;
        }
        paint_one(state, i, d, ends, t, at);
    }
    match chip.zip(chip_at) {
        Some(((more, size), at)) => draw_chip_at(state, more, at, size, chip_scale),
        None => drop_chip(state),
    }
    true
}

/// A sheet whose app icon is missing shows the stock one in its slot, unless its picture fills it.
fn slot_needs_icon(lay: &SheetLayout, has_picture: bool) -> bool {
    !has_picture || lay.picture != Some(lay.icon)
}

/// The app icon and picture sheet `b` shows on `lay`, device px.
fn sheet_icons(
    icons: &mut crate::icontheme::IconCache,
    b: &Balloon,
    lay: &SheetLayout,
) -> (Option<Canvas>, Option<Canvas>) {
    let slot = lay.icon.width;
    let picture = crate::tray::closest(b.item.image(), slot);
    let app_px = lay.badge.map_or(slot, |r| r.width);
    let app = crate::tray::closest(b.item.app_icon(), app_px).or_else(|| {
        slot_needs_icon(lay, picture.is_some())
            .then(|| icons.get("dialog-information", slot))
            .flatten()
    });
    (app, picture)
}

/// Once the loop is idle, sends each Droplet at rest to have its layers for its next move warmed
/// off the loop's thread: under the lit rim while collapsed, the plain one while open.
fn warm_openings(state: &mut State) {
    let droplets = sheets(state) && state.config.glass.notification.shape() == SheetShape::Droplet;
    if !droplets || state.notify.warm_queued {
        return;
    }
    state.notify.warm_queued = true;
    state.handle.insert_idle(|state| {
        state.notify.warm_queued = false;
        let now = Instant::now();
        let Some(edge) = state.notify.edge else {
            return;
        };
        let shape = state.config.glass.notification.shape();
        for i in 0..state.notify.stack.len() {
            let mut opening = std::mem::take(&mut state.notify.stack[i].opening);
            warm_one(state, i, &mut opening, edge, shape, now);
            state.notify.stack[i].opening = opening;
        }
    });
}

fn warm_one(
    state: &mut State,
    i: usize,
    opening: &mut SheetOpening,
    edge: Edge,
    shape: SheetShape,
    now: Instant,
) {
    let b = &state.notify.stack[i];
    // A reply field's caret blinks, which would rebuild the layers on every blink.
    if !b.motion.settled(now) || b.reply.is_some() || !matches!(b.drawn, Some(Drawn::Sheet(_))) {
        return;
    }
    let theme = state.config.theme.scaled(b.scale);
    let when = relative(b.shown_at.elapsed());
    let Some([shut, open]) = droplet_ends(&theme, &state.text, &b.item, false, edge, shape, &when)
    else {
        return;
    };
    let lit = b.open_drawn < 1.0;
    let (app, picture) = sheet_icons(&mut state.icons, b, &open);
    let icons = SheetIcons {
        app: app.as_ref(),
        picture: picture.as_ref(),
    };
    let tint = b.tint.flatten();
    let Some(job) = opening.warm_job(&b.glass, &theme, &shut, &open, icons, tint, lit) else {
        return;
    };
    let Some(tx) = warmer(state) else {
        return;
    };
    let b = &mut state.notify.stack[i];
    b.warm_gen += 1;
    if tx.send((b.serial, b.warm_gen, job)).is_err() {
        opening.forget_job();
        lose_warmer(&mut state.notify);
    }
}

/// The painter thread ended, a job's panic among the causes: the jobs it held are asked again of
/// the next one.
fn lose_warmer(v: &mut NotifyView) {
    v.warmer = None;
    for b in &mut v.stack {
        b.opening.forget_job();
    }
}

/// The sender to the thread that paints opening layers, started on first use; its results land
/// through [`land_warm`].
fn warmer(state: &mut State) -> Option<std::sync::mpsc::Sender<(u64, u64, WarmJob)>> {
    if let Some(tx) = &state.notify.warmer {
        return Some(tx.clone());
    }
    if let Some(gone) = state.notify.warm_source.take() {
        state.handle.remove(gone);
    }
    let (done, landed) = channel::channel::<(u64, u64, WarmLayers)>();
    let inserted = state
        .handle
        .insert_source(landed, |event, _, state| match event {
            channel::Event::Msg((serial, generation, built)) => {
                land_warm(&mut state.notify.stack, serial, generation, built);
            }
            channel::Event::Closed => lose_warmer(&mut state.notify),
        });
    match inserted {
        Ok(token) => state.notify.warm_source = Some(token),
        Err(e) => {
            log::warn!("notify: no warm channel: {}", e.error);
            return None;
        }
    }
    let (tx, jobs) = std::sync::mpsc::channel::<(u64, u64, WarmJob)>();
    let spawned = std::thread::Builder::new()
        .name("notify-warm".into())
        .spawn(move || {
            let mut text: Option<(reprisekit::theme::FontConfig, TextRenderer)> = None;
            for (serial, generation, job) in jobs {
                if text.as_ref().is_none_or(|(fonts, _)| fonts != job.fonts()) {
                    text = Some((job.fonts().clone(), TextRenderer::load(job.fonts())));
                }
                let Some((_, text)) = &text else {
                    unreachable!("loaded above");
                };
                if done
                    .send((serial, generation, build_warm(job, text)))
                    .is_err()
                {
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("notify: no warm thread: {e}");
        return None;
    }
    state.notify.warmer = Some(tx.clone());
    Some(tx)
}

/// Gives balloon `serial` the layers its warm job `generation` painted, unless it left or sent a
/// later job since; true when taken.
fn land_warm(stack: &mut [Balloon], serial: u64, generation: u64, built: WarmLayers) -> bool {
    match stack.iter_mut().find(|b| b.serial == serial) {
        Some(b) if b.warm_gen == generation => {
            b.opening.install(&mut b.glass, built);
            true
        }
        _ => false,
    }
}

/// Draws balloon `i` as `drawn` at `at`, logical, a Droplet part way open `t` of the way between
/// its `ends`; one drawn alike before is only placed.
fn paint_one(
    state: &mut State,
    i: usize,
    drawn: Drawn,
    ends: Option<[SheetLayout; 2]>,
    t: f64,
    at: (i32, i32),
) {
    let mut opening = std::mem::take(&mut state.notify.stack[i].opening);
    let mut kept = std::mem::take(&mut state.notify.stack[i].glass);
    let b = &state.notify.stack[i];
    let preedit = b
        .reply
        .as_ref()
        .and_then(|_| state.ime_field_preedit(Editor::NotifyReply));
    let pushed = match b.held {
        Some((part, true)) => Some(part),
        _ => None,
    };
    let looks = Painted {
        drawn: drawn.clone(),
        open: t,
        pushed,
        hot: b.hot,
    };
    let fresh = b.reply.is_some() || b.painted.as_ref() != Some(&looks);
    let scale = b.scale;
    let theme = state.config.theme.scaled(scale);
    let reply = b.reply.as_ref().map(|r| {
        (
            &r.edit,
            Caret::new(r.focused, state.caret.on),
            preedit.as_ref(),
        )
    });
    let (mut canvas, pane) = match &drawn {
        Drawn::Balloon(lay) => {
            let canvas = fresh.then(|| {
                let icon_px = theme.icon_size();
                let icon = crate::tray::closest(b.item.icon(), icon_px)
                    .or_else(|| state.icons.get("dialog-information", icon_px));
                draw_notify_balloon(
                    &theme,
                    &state.text,
                    lay,
                    icon.as_ref(),
                    pushed,
                    b.hot,
                    reply,
                )
            });
            (canvas, None)
        }
        Drawn::Sheet(lay) => {
            let canvas = fresh.then(|| {
                let (app, picture) = sheet_icons(&mut state.icons, b, lay);
                let icons = SheetIcons {
                    app: app.as_ref(),
                    picture: picture.as_ref(),
                };
                let builds = opening.builds();
                let canvas = match &ends {
                    Some([shut, open]) => opening.draw(
                        &mut kept,
                        &theme,
                        &state.text,
                        shut,
                        open,
                        t,
                        icons,
                        b.tint.flatten(),
                        pushed,
                        b.hot,
                        reply,
                    ),
                    None => kept.draw(
                        &theme,
                        &state.text,
                        lay,
                        icons,
                        b.tint.flatten(),
                        pushed,
                        b.hot,
                        reply,
                    ),
                };
                if opening.builds() > builds {
                    log::info!("notify: an opening painted its layers in the frame");
                }
                canvas
            });
            let look = theme.look.style().and_then(|st| st.notify);
            let pane = look
                .filter(|_| crate::glass::wanted(&state.config))
                .map(|look| sheet_glass(lay, &look, scale));
            (canvas, pane)
        }
    };
    let replying = b.reply.is_some();
    let (ox, oy) = drawn.origin();
    let (glass_tree, buffer) = (b.glass_tree, b.buffer);
    if let Some(canvas) = &canvas {
        upload_scaled(&mut state.scene, buffer, canvas, scale);
        let input = match &drawn {
            Drawn::Balloon(_) => Input::All,
            Drawn::Sheet(lay) => {
                let lay = lay.clone();
                Input::Test(Box::new(move |p| {
                    let at = |v: f64| (v * f64::from(scale)).floor() as i32;
                    sheet_hit(&lay, at(p.x), at(p.y)).is_some()
                }))
            }
        };
        state.scene.set_input(buffer, input);
    }
    let (edge, scale_from) = (state.notify.edge, state.config.effects.window_scale);
    let State {
        notify,
        scene,
        glass,
        ..
    } = state;
    let b = &mut notify.stack[i];
    b.opening = opening;
    b.glass = kept;
    b.painted = Some(looks);
    b.at = at;
    b.drawn = Some(drawn);
    b.open_drawn = t;
    if pane.is_some() {
        b.backdrop.get_or_insert_with(|| Backdrop::new(glass));
    } else if let Some(mut gone) = b.backdrop.take() {
        gone.set(scene, glass, glass_tree, None);
    }
    b.pane = pane;
    if b.motion.drip
        && b.bead.is_none()
        && drip_bead_share(b.motion.k.value(Instant::now())) > 0.0
        && let Some((c, body)) = drip_bead(&theme)
    {
        let buffer = scene.buffer_create(b.tree);
        scene.set_input(buffer, Input::Passthrough);
        upload_scaled(scene, buffer, &c, scale);
        let logical =
            |r: Rect| Rect::new(r.x / scale, r.y / scale, r.width / scale, r.height / scale);
        b.bead = Some(Bead {
            buffer,
            body: logical(body),
            size: (c.width as i32 / scale, c.height as i32 / scale),
        });
    }
    pose_balloon(scene, glass, b, Instant::now(), edge, scale_from);
    if replying && let Some(canvas) = canvas.as_mut() {
        let origin = (at.0 - ox / scale, at.1 - oy / scale);
        let spot = canvas.take_spot();
        state.ime_field_drawn(Editor::NotifyReply, spot, origin, scale);
    }
}

/// Draws the chip saying `more` wait at `at`, `size` logical.
fn draw_chip_at(state: &mut State, more: usize, at: (i32, i32), size: (i32, i32), scale: i32) {
    let c = match state.notify.chip.take() {
        Some(c) => c,
        None => {
            let scene = &mut state.scene;
            let tree = scene.tree_create(state.layers.overlay);
            crate::glass::leave_out(scene, tree);
            scene.set_data(tree, Some(SceneOwner::Notification));
            let buffer = scene.buffer_create(tree);
            Chip {
                tree,
                buffer,
                at,
                size,
                more,
                hot: false,
            }
        }
    };
    let canvas = draw_chip(&state.config.theme.scaled(scale), &state.text, more, c.hot);
    upload_scaled(&mut state.scene, c.buffer, &canvas, scale);
    state.scene.set_position(c.tree, at.0, at.1);
    state.notify.chip = Some(Chip {
        at,
        size,
        more,
        ..c
    });
}

fn drop_chip(state: &mut State) {
    if let Some(c) = state.notify.chip.take() {
        state.scene.destroy(c.tree);
        state.redraw_all();
    }
}

/// The balloons' tree, kept while no balloon is up: an alert added to a tree a reader already
/// holds is announced, one in a new tree is not.
pub fn access(state: &State) -> reprisekit::access::Tree {
    use reprisekit::access::{ROOT, accesskit::Role};
    const NAME: &str = "Notifications";
    let v = &state.notify;
    let Some(output) = state.first_output().filter(|_| !v.stack.is_empty()) else {
        return crate::access::root(Role::Window, NAME, Rect::default());
    };
    let theme = &state.config.theme;
    let edge = state.bar_placement(&output).edge;
    let shape = state.config.glass.notification.shape();
    let now = Instant::now();
    let laid: Vec<(Drawn, Rect)> = v
        .stack
        .iter()
        .map(|b| {
            let when = relative(b.shown_at.elapsed());
            let (d, _) = lay_out_open(
                theme,
                &state.text,
                &b.item,
                b.reply.is_some(),
                edge,
                shape,
                &when,
                b.motion.open.value(now),
            );
            let ((ox, oy), (w, h)) = (d.origin(), d.size());
            (d, Rect::new(b.at.0 - ox, b.at.1 - oy, w, h))
        })
        .collect();
    let chip = v
        .chip
        .as_ref()
        .map(|c| (c.more, Rect::new(c.at.0, c.at.1, c.size.0, c.size.1)));
    let rect = laid
        .iter()
        .map(|(_, r)| *r)
        .chain(chip.map(|c| c.1))
        .reduce(|a, r| a.union(&r))
        .unwrap_or_default();
    let mut tree = crate::access::root(Role::Window, NAME, rect);
    let px = theme.fonts.size_px;
    let mut focus = None;
    for (b, (drawn, r)) in v.stack.iter().zip(&laid) {
        let off = (r.x - rect.x, r.y - rect.y);
        let labels;
        let lines;
        let says = match &b.item {
            Item::Note(n) => {
                labels = button_labels(n);
                NotifySays {
                    app: &n.app,
                    title: &n.summary,
                    body: &n.body,
                    actions: &labels,
                    opens: n.has_action(DEFAULT_ACTION),
                    urgent: n.urgency == Urgency::Critical,
                }
            }
            Item::Burst(burst) => {
                lines = (
                    burst.title(),
                    Vec::from_iter(burst.lines.iter().map(String::as_str)).join("\n"),
                );
                NotifySays {
                    app: "",
                    title: &lines.0,
                    body: &lines.1,
                    actions: &[],
                    opens: true,
                    urgent: false,
                }
            }
        };
        let reply = b.reply.as_ref();
        let node = match drawn {
            Drawn::Balloon(lay) => access_parts(
                b.serial,
                &lay.parts().offset(off.0, off.1),
                &says,
                reply,
                &state.text,
                px,
                &mut tree,
                ROOT,
            ),
            Drawn::Sheet(lay) => access_sheet(
                b.serial,
                lay,
                off,
                &says,
                reply,
                &state.text,
                px,
                &mut tree,
                ROOT,
            ),
        };
        if reply.is_some() || focus.is_none() {
            focus = Some(node);
        }
    }
    if let Some((more, r)) = chip {
        let at = Rect::new(r.x - rect.x, r.y - rect.y, r.width, r.height);
        let name = format!("{more} more notifications");
        tree.push(
            ROOT,
            "chip",
            reprisekit::access::button(at, &name, true, false),
        );
    }
    tree.set_focus(focus.unwrap_or(ROOT));
    tree
}

/// The caret blinked: redraw the reply field.
pub fn blink(state: &mut State) {
    paint(state);
}

/// Places `b` as its motion stands at `now` by the bar on `edge`: posed about its rest, faded, its
/// glass following. A Droplet arriving swells from a bead at the tray-side cap.
fn pose_balloon(
    scene: &mut DeskScene,
    glass: &Rc<GlassCtx>,
    b: &mut Balloon,
    now: Instant,
    edge: Option<Edge>,
    scale_from: f64,
) {
    let Some(drawn) = &b.drawn else {
        return;
    };
    let s = b.scale;
    let ((ox, oy), (w, h), (fw, fh)) = (drawn.origin(), drawn.size(), drawn.footprint());
    let (ox, oy) = (ox / s, oy / s);
    let y = b.at.1 + b.motion.offset(now).round() as i32;
    let rest = Rect::new(b.at.0 - ox, y - oy, w / s, h / s);
    let k = b.motion.k.value(now);
    let ratio = |to: i32, from: i32| f64::from(to) / f64::from(from.max(1));
    let times = |v: i32, k: f64| (f64::from(v) * k).round() as i32;
    let tray_right = edge != Some(Edge::Left);
    let (r, inset, alpha, pane, bead) = if b.motion.drip && k < 1.0 {
        let fp = Rect::new(b.at.0, y, fw / s, fh / s);
        let (p, alpha) = drip_pose(k, fp, tray_right);
        let (kx, ky) = (ratio(p.width, fp.width), ratio(p.height, fp.height));
        let inset = (times(ox, kx), times(oy, ky));
        let r = Rect::new(
            p.x - inset.0,
            p.y - inset.1,
            times(rest.width, kx),
            times(rest.height, ky),
        );
        let pane = b
            .pane
            .as_ref()
            .map(|_| crate::glass::sheet_pane((p.width, p.height), drip_corner(p.height)));
        let bead = (drip_bead_at(p, tray_right), drip_bead_share(k), p);
        (r, inset, alpha, pane, Some(bead))
    } else {
        let toward_tray = if edge == Some(Edge::Top) { -1 } else { 1 };
        let (r, alpha) = pose(k, rest, RISE, toward_tray, scale_from);
        let inset = (
            times(ox, ratio(r.width, rest.width)),
            times(oy, ratio(r.height, rest.height)),
        );
        let pane = b
            .pane
            .as_ref()
            .map(|pane| crate::windowanim::scale_pane(pane, rest, r));
        (r, inset, alpha, pane, None)
    };
    scene.set_position(b.tree, r.x + inset.0, r.y + inset.1);
    scene.set_position(b.buffer, -inset.0, -inset.1);
    scene.buffer_set_dest_size(b.buffer, Some((r.width, r.height).into()));
    scene.set_opacity(b.tree, alpha);
    let share = bead.map_or(0.0, |(_, share, _)| share);
    scene.set_opacity(b.buffer, 1.0 - share);
    match (&b.bead, bead) {
        (Some(bd), Some((q, share, p))) if share > 0.0 => {
            let k = f64::from(q.height) / f64::from(bd.body.height.max(1));
            scene.set_position(
                bd.buffer,
                q.x - p.x - times(bd.body.x, k),
                q.y - p.y - times(bd.body.y, k),
            );
            let size = (times(bd.size.0, k), times(bd.size.1, k));
            scene.buffer_set_dest_size(bd.buffer, Some(size.into()));
            scene.set_opacity(bd.buffer, share);
        }
        (Some(_), _) => {
            if let Some(gone) = b.bead.take() {
                scene.destroy(gone.buffer);
            }
        }
        (None, _) => {}
    }
    if let (Some(backdrop), Some(pane)) = (b.backdrop.as_mut(), pane) {
        backdrop.set(scene, glass, b.glass_tree, Some(pane));
    }
}

fn still(v: &NotifyView, now: Instant) -> bool {
    v.stack
        .iter()
        .chain(&v.leaving)
        .all(|b| b.motion.settled(now))
}

fn arm_tick(state: &mut State) {
    if state.notify.tick_timer.is_some() || still(&state.notify, Instant::now()) {
        return;
    }
    match state
        .handle
        .insert_source(Timer::from_duration(NOTIFY_TICK), |_, _, state| {
            if tick(state) {
                TimeoutAction::ToDuration(NOTIFY_TICK)
            } else {
                state.notify.tick_timer = None;
                TimeoutAction::Drop
            }
        }) {
        Ok(t) => state.notify.tick_timer = Some(t),
        Err(e) => log::warn!("notification motion timer: {}", e.error),
    }
}

/// Redraws the Droplets opening, with the stack following their height and the hold the still
/// pointer, poses every sheet and takes down those done leaving; true while any still moves.
fn tick(state: &mut State) -> bool {
    let now = Instant::now();
    let v = &state.notify;
    if v.stack
        .iter()
        .any(|b| b.motion.open.value(now) != b.open_drawn)
    {
        if lay_stack(state, true) {
            let p = state.pointer_location();
            if hold_at(&mut state.notify, p.x, p.y, now) && hold_clocks(state) {
                rearm(state);
            }
        }
        if state
            .notify
            .stack
            .iter()
            .all(|b| b.motion.open.settled(now))
        {
            crate::access::touch(state);
        }
    }
    let (edge, scale_from) = (state.notify.edge, state.config.effects.window_scale);
    let State {
        notify,
        scene,
        glass,
        ..
    } = state;
    for b in notify.stack.iter_mut().chain(notify.leaving.iter_mut()) {
        pose_balloon(scene, glass, b, now, edge, scale_from);
    }
    let (gone, kept): (Vec<Balloon>, Vec<Balloon>) = std::mem::take(&mut notify.leaving)
        .into_iter()
        .partition(|b| b.motion.settled(now));
    notify.leaving = kept;
    restack(scene, &notify.stack, &notify.leaving, now);
    for b in gone {
        destroy(state, b);
    }
    state.redraw_all();
    let moving = !still(&state.notify, now);
    let p = state.pointer_location();
    if !moving && hold_at(&mut state.notify, p.x, p.y, now) && hold_clocks(state) {
        rearm(state);
    }
    if !moving {
        warm_openings(state);
    }
    moving
}

/// Sheet `b` leaves its slot: it eases out without the pointer, then goes. A balloon goes at once.
fn leave(state: &mut State, mut b: Balloon) {
    let now = Instant::now();
    b.motion.leave(&state.config.effects, now);
    if !matches!(b.drawn, Some(Drawn::Sheet(_))) || b.motion.settled(now) {
        destroy(state, b);
        return;
    }
    state.scene.set_input(b.buffer, Input::Passthrough);
    state.notify.leaving.push(b);
    let v = &state.notify;
    restack(&mut state.scene, &v.stack, &v.leaving, now);
    crate::autohideview::poke(state);
    crate::access::touch(state);
    arm_tick(state);
}

/// A sheet's band in the overlay, lowest first: leaving ones draw under arriving ones, and both
/// under those resting or sliding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Band {
    Leaving,
    Arriving,
    Resting,
}

fn band(m: &Motion, leaving: bool, now: Instant) -> Band {
    if leaving {
        Band::Leaving
    } else if !m.k.settled(now) {
        Band::Arriving
    } else {
        Band::Resting
    }
}

/// Orders the trees of `stack` and `leaving` by `band` within the overlay slots they already
/// hold, keeping their scene order within a band. A sheet's blur pane lives in its tree.
fn restack(scene: &mut DeskScene, stack: &[Balloon], leaving: &[Balloon], now: Instant) {
    let mut trees: Vec<(Band, usize, NodeId)> = stack
        .iter()
        .map(|b| (band(&b.motion, false, now), b.tree))
        .chain(leaving.iter().map(|b| (Band::Leaving, b.tree)))
        .filter_map(|(band, tree)| {
            let at = scene
                .children(scene.parent(tree)?)
                .iter()
                .position(|c| *c == tree.into())?;
            Some((band, at, tree.into()))
        })
        .collect();
    trees.sort_by_key(|&(band, at, _)| (band, at));
    let order: Vec<NodeId> = trees.into_iter().map(|t| t.2).collect();
    scene.reorder_in_place(&order);
}

fn drop_leaving(state: &mut State) {
    for b in std::mem::take(&mut state.notify.leaving) {
        destroy(state, b);
    }
}

fn destroy(state: &mut State, mut b: Balloon) {
    if let Some(mut gone) = b.backdrop.take() {
        gone.set(&mut state.scene, &state.glass, b.glass_tree, None);
    }
    state.scene.destroy(b.tree);
    state.redraw_all();
    crate::autohideview::poke(state);
    crate::access::touch(state);
}

/// The menu rows for `actions`: each label, or its key when the sender gave none (`default` reads
/// "Open", `inline-reply` "Reply").
pub fn action_items(actions: &[(String, String)]) -> Vec<MenuItem> {
    actions
        .iter()
        .map(|(k, l)| {
            let label = match (l.is_empty(), k.as_str()) {
                (false, _) => l.clone(),
                (true, DEFAULT_ACTION) => "Open".to_string(),
                (true, INLINE_REPLY) => "Reply".to_string(),
                (true, _) => k.clone(),
            };
            MenuItem::entry(label.replace('&', "&&"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::glass::notify::HALO_CORNER;
    use reprisekit::interface::Interface;
    use reprisekit::interfaces::theme_for;

    fn text() -> TextRenderer {
        TextRenderer::load(&reprisekit::theme::FontConfig::default())
    }

    #[test]
    fn glass_lays_out_a_sheet_and_classic_a_balloon() {
        let item = Item::Note(note(1, "Mail"));
        let lay = |t: &Theme| {
            lay_out(
                t,
                &text(),
                &item,
                false,
                Edge::Bottom,
                SheetShape::Lozenge,
                false,
                "now",
            )
        };
        assert!(matches!(lay(&theme_for(Interface::Glass)), Drawn::Sheet(_)));
        assert!(matches!(lay(&Theme::default()), Drawn::Balloon(_)));
    }

    #[test]
    fn a_lozenge_pane_is_its_footprint_at_the_tree_origin() {
        let t = theme_for(Interface::Glass).scaled(2);
        let item = Item::Note(note(1, "Mail"));
        let Drawn::Sheet(lay) = lay_out(
            &t,
            &text(),
            &item,
            false,
            Edge::Bottom,
            SheetShape::Lozenge,
            false,
            "now",
        ) else {
            panic!()
        };
        let pane = sheet_glass(&lay, t.look.style().unwrap().notify.as_ref().unwrap(), 2);
        assert_eq!(pane.shapes.len(), 1);
        assert_eq!(
            pane.shapes[0].rect,
            Rect::new(0, 0, 360, lay.footprint.height / 2)
        );
        assert_eq!(pane.shapes[0].corner.len(), 25);
    }

    fn halo_sheet_layout(scale: i32) -> (SheetLayout, NotifyLook) {
        let t = theme_for(Interface::Glass).scaled(scale);
        let item = Item::Note(note(1, "Mail"));
        let Drawn::Sheet(lay) = lay_out(
            &t,
            &text(),
            &item,
            false,
            Edge::Bottom,
            SheetShape::Halo,
            false,
            "now",
        ) else {
            panic!()
        };
        (lay, t.look.style().unwrap().notify.unwrap())
    }

    #[test]
    fn the_halo_pane_covers_the_breakout_without_overlap() {
        let (lay, look) = halo_sheet_layout(1);
        let pane = sheet_glass(&lay, &look, 1);
        assert_eq!(pane.shapes.len(), 2);
        assert_eq!(pane.shapes[0].rect, Rect::new(9, 0, 50, 25));
        assert!(pane.shapes.iter().all(|s| s.rect.x >= 0 && s.rect.y >= 0));
        assert!(!pane.shapes[0].rect.intersects(pane.shapes[1].rect));
        assert_eq!(pane.shapes[1].rect.bottom(), lay.footprint.height);
    }

    #[test]
    fn the_halo_pane_is_the_well_top_and_the_sheet_in_logical_px() {
        let (lay, look) = halo_sheet_layout(2);
        let pane = sheet_glass(&lay, &look, 2);
        let [well, sheet] = &pane.shapes[..] else {
            panic!("{pane:?}")
        };
        assert_eq!(
            (well.rect, well.corner.len(), well.square_bottom),
            (Rect::new(9, 0, 50, 25), WELL_CORNER.len(), true)
        );
        assert_eq!(
            (sheet.rect, sheet.corner.len(), sheet.square_bottom),
            (
                Rect::new(0, 25, 360, lay.sheet.height / 2),
                HALO_CORNER.len(),
                false
            )
        );
    }

    #[test]
    fn a_coloured_app_icon_tints_its_sheet_and_a_grey_one_does_not() {
        let icon = |px: u32| {
            [IconSource {
                width: 48,
                height: 48,
                pixels: vec![px; 48 * 48],
            }]
        };
        let glass = theme_for(Interface::Glass);
        assert_eq!(
            sheet_tint(&glass, &icon(0xFFD0_3030), 2),
            Some(Some(Color::rgb(0xD0, 0x30, 0x30)))
        );
        assert_eq!(sheet_tint(&glass, &icon(0xFF80_8080), 1), Some(None));
        assert_eq!(sheet_tint(&glass, &[], 1), Some(None));
        assert_eq!(sheet_tint(&Theme::default(), &icon(0xFFD0_3030), 1), None);
    }

    fn red_icon() -> Vec<IconSource> {
        vec![IconSource {
            width: 48,
            height: 48,
            pixels: vec![0xFFD0_3030; 48 * 48],
        }]
    }

    #[test]
    fn a_burst_takes_its_tint_from_its_first_app_icon() {
        let glass = theme_for(Interface::Glass);
        let mut scene = crate::compositor::DeskScene::new();
        let mut b = balloon(&mut scene, 1);
        b.item = Item::Burst(Burst::new("Mail"));
        join_burst(&mut b, &note(2, "Mail"), &glass);
        assert_eq!(b.tint, Some(None));
        let mut red = note(3, "Mail");
        red.app_icon = red_icon();
        join_burst(&mut b, &red, &glass);
        assert_eq!(b.tint, Some(Some(Color::rgb(0xD0, 0x30, 0x30))));
        let mut blue = note(4, "Mail");
        blue.app_icon = vec![IconSource {
            width: 48,
            height: 48,
            pixels: vec![0xFF30_60D0; 48 * 48],
        }];
        join_burst(&mut b, &blue, &glass);
        assert_eq!(
            b.tint,
            Some(Some(Color::rgb(0xD0, 0x30, 0x30))),
            "the first icon's"
        );
    }

    #[test]
    fn a_balloon_shown_before_a_switch_to_glass_takes_its_tint() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut b = balloon(&mut scene, 1);
        let mut n = note(1, "Mail");
        n.app_icon = red_icon();
        b.item = Item::Note(n);
        let mut stack = [b];
        retint(&mut stack, &Theme::default());
        assert_eq!(stack[0].tint, None);
        retint(&mut stack, &theme_for(Interface::Glass));
        assert_eq!(stack[0].tint, Some(Some(Color::rgb(0xD0, 0x30, 0x30))));
        retint(&mut stack, &Theme::default());
        assert_eq!(stack[0].tint, None, "back to Classic");
    }

    #[test]
    fn a_tinted_balloon_keeps_its_tint_through_a_reposition_under_glass() {
        let glass = theme_for(Interface::Glass);
        let mut scene = crate::compositor::DeskScene::new();
        let mut b = balloon(&mut scene, 1);
        let mut n = note(1, "Mail");
        n.app_icon = red_icon();
        b.item = Item::Note(n);
        b.tint = Some(Some(Color::rgb(0x30, 0x60, 0xD0)));
        let mut stack = [b];
        retint(&mut stack, &glass);
        assert_eq!(
            stack[0].tint,
            Some(Some(Color::rgb(0x30, 0x60, 0xD0))),
            "the icon is not read again"
        );
        stack[0].tint = Some(None);
        retint(&mut stack, &glass);
        assert_eq!(stack[0].tint, Some(None), "a grey icon is not read again");
    }

    #[test]
    fn relative_time_reads_now_then_minutes_then_hours() {
        let s = |n| relative(Duration::from_secs(n));
        assert_eq!(
            [s(0), s(59), s(61), s(3601)],
            ["now", "now", "1 min ago", "1 h ago"]
        );
    }

    fn note(id: u32, app: &str) -> Notification {
        let mut n = Notification::new(id, format!("n{id}"), "x".into());
        n.app = app.into();
        n
    }

    #[test]
    fn the_queue_drops_its_oldest_past_the_cap() {
        let mut q = VecDeque::new();
        let mut dropped = Vec::new();
        for id in 1..=(MAX_QUEUED as u32 + 2) {
            dropped.extend(enqueue(&mut q, Item::Note(note(id, ""))));
        }
        assert_eq!(dropped, vec![1, 2]);
        assert_eq!(q.len(), MAX_QUEUED);
        assert!(q.front().is_some_and(|i| i.is(3)));
    }

    #[test]
    fn a_critical_balloon_never_times_out() {
        let mut n = Notification::new(1, "Hi".into(), String::new());
        assert_eq!(timeout_ms(&n), Some(DEFAULT_MS));
        n.expire_timeout = 0;
        assert_eq!(timeout_ms(&n), Some(MAX_MS));
        n.expire_timeout = 100;
        assert_eq!(timeout_ms(&n), Some(MIN_MS));
        n.urgency = Urgency::Critical;
        assert_eq!(timeout_ms(&n), None);
    }

    /// An open reply row collapses once idle, but not while the pointer rests in the balloon; a
    /// collapsed balloon expires on its own timeout.
    #[test]
    fn an_idle_reply_row_collapses_unless_the_pointer_is_in_the_balloon() {
        assert_eq!(due_step(true, false), Due::Collapse);
        assert_eq!(due_step(true, true), Due::Wait);
        assert_eq!(due_step(false, true), Due::Expire);
        assert_eq!(due_step(false, false), Due::Expire);
        assert_eq!(REPLY_IDLE, Duration::from_secs(120));
    }

    #[test]
    fn reply_brings_back_its_notifications_draft_focused() {
        let typed = |t: &str| ReplyField {
            edit: TextEdit::new(t, false),
            focused: false,
        };
        let mut drafts = vec![(1, typed("see you")), (2, typed("ok"))];
        let back = take_draft(&mut drafts, 1);
        assert_eq!((back.edit.text.as_str(), back.focused), ("see you", true));
        assert_eq!(drafts.len(), 1);
        let fresh = take_draft(&mut drafts, 3);
        assert!(fresh.edit.text.is_empty() && fresh.focused);
    }

    #[test]
    fn buttons_skip_the_default_action_and_menu_rows_name_every_action() {
        let mut n = Notification::new(1, "Hi".into(), String::new());
        n.actions = vec![
            ("default".into(), String::new()),
            ("open".into(), "Open".into()),
            ("x".into(), String::new()),
            (INLINE_REPLY.into(), String::new()),
        ];
        assert_eq!(button_labels(&n), ["Open", "x", "Reply"]);
        let rows = action_items(&n.actions);
        assert_eq!(rows[0], MenuItem::entry("Open"));
        assert_eq!(rows[2], MenuItem::entry("x"));
        assert_eq!(rows[3], MenuItem::entry("Reply"));
        let amp = action_items(&[("k".into(), "Save & Quit".into())]);
        assert_eq!(amp[0], MenuItem::entry("Save && Quit"));
    }

    #[test]
    fn the_fourth_from_one_program_in_the_window_bursts() {
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let mut r = VecDeque::new();
        assert!(!arrived(&mut r, "Mail", at(0)));
        assert!(!arrived(&mut r, "Chat", at(1)));
        assert!(!arrived(&mut r, "Mail", at(2)));
        assert!(!arrived(&mut r, "Mail", at(3)));
        assert!(arrived(&mut r, "Mail", at(4)), "the fourth");
        assert!(!arrived(&mut r, "Chat", at(5)));
        let mut r = VecDeque::new();
        for s in [0, 4, 8] {
            arrived(&mut r, "Mail", at(s));
        }
        assert!(!arrived(&mut r, "Mail", at(11)), "the first fell out");
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn a_burst_takes_its_programs_queued_balloons() {
        let mut q: VecDeque<Item> = [
            note(1, "Chat"),
            note(2, "Mail"),
            note(3, "Chat"),
            note(4, "Mail"),
        ]
        .into_iter()
        .map(Item::Note)
        .collect();
        let (taken, at) = take_queued(&mut q, "Mail");
        assert_eq!(taken.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(at, Some(1));
        assert!(q.iter().all(|i| i.note().is_some_and(|n| n.app == "Chat")));
        assert_eq!(take_queued(&mut q, "Mail"), (Vec::new(), None));
    }

    #[test]
    fn a_burst_counts_and_lists_its_newest() {
        let mut b = Burst::new("Mail");
        for id in 1..=5 {
            b.add(&note(id, "Mail"));
        }
        assert_eq!(b.title(), "5 new from Mail");
        assert_eq!(b.lines, ["n5", "n4", "n3"]);
        let mut empty = note(6, "Mail");
        empty.summary.clear();
        empty.body = "first line\nsecond".into();
        b.add(&empty);
        assert_eq!(b.lines[0], "first line");
    }

    #[test]
    fn a_burst_keeps_the_first_app_icon_and_has_no_picture() {
        let src = |w: u32| IconSource {
            width: w,
            height: w,
            pixels: vec![0; (w * w) as usize],
        };
        let mut first = note(1, "Mail");
        first.image = vec![src(32)];
        let mut second = note(2, "Mail");
        second.app_icon = vec![src(16)];
        let mut b = Burst::new("Mail");
        b.add(&first);
        assert!(!Item::Burst(b.clone()).has_app_icon());
        b.add(&second);
        let item = Item::Burst(b);
        assert_eq!(item.app_icon()[0].width, 16);
        assert_eq!(item.icon()[0].width, 32, "Classic draws the first picture");
        assert!(item.image().is_empty());
    }

    fn items(ids: &[u32]) -> Vec<Item> {
        ids.iter().map(|&i| Item::Note(note(i, "Mail"))).collect()
    }

    #[test]
    fn glass_shows_three_and_queues_the_fourth() {
        assert_eq!((visible_cap(true), visible_cap(false)), (3, 1));
        let mut q: VecDeque<Item> = items(&[4, 5]).into();
        assert!(fill(3, 3, &mut q).is_empty());
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn a_leaving_sheet_lets_the_queue_head_in() {
        let mut q: VecDeque<Item> = items(&[4, 5]).into();
        let came = fill(2, 3, &mut q);
        assert!(came.len() == 1 && came[0].is(4));
        assert!(q.len() == 1 && q[0].is(5));
    }

    #[test]
    fn hovering_pauses_every_clock_and_leaving_resumes() {
        let t0 = Instant::now();
        let mut c = Clock::start(Some(7000), t0);
        c.pause(t0 + Duration::from_secs(2));
        assert_eq!(c.due(), None);
        c.resume(t0 + Duration::from_secs(10));
        assert_eq!(c.due(), Some(t0 + Duration::from_secs(15)));
    }

    #[test]
    fn a_critical_clock_never_runs() {
        assert_eq!(Clock::start(None, Instant::now()).due(), None);
    }

    #[test]
    fn the_chip_counts_the_queue() {
        use reprisekit::controls::notifysheet::chip_label;
        assert_eq!(
            (chip_count(5, 3), chip_count(0, 3), chip_count(5, 1)),
            (Some(5), None, None)
        );
        assert_eq!(chip_label(5), "+5 more");
    }

    #[test]
    fn a_replaced_visible_note_keeps_its_slot() {
        let shown = items(&[1, 2, 3]);
        let refs: Vec<&Item> = shown.iter().collect();
        assert_eq!((slot_of(&refs, 2), slot_of(&refs, 9)), (Some(1), None));
    }

    #[test]
    fn a_burst_takes_every_same_app_sheet_but_a_reply() {
        let (a, b, c, d) = (
            Item::Note(note(1, "Mail")),
            Item::Note(note(2, "Chat")),
            Item::Note(note(3, "Mail")),
            Item::Note(note(4, "Mail")),
        );
        assert_eq!(
            fold_from(&[(&a, false), (&b, false), (&c, true), (&d, false)], "Mail"),
            vec![0, 3]
        );
    }

    fn balloon(scene: &mut crate::compositor::DeskScene, id: u32) -> Balloon {
        let tree = scene.tree_create(scene.root());
        Balloon {
            item: Item::Note(note(id, "Mail")),
            at: (0, 0),
            scale: 1,
            tree,
            glass_tree: scene.tree_create(tree),
            backdrop: None,
            buffer: scene.buffer_create(tree),
            shown_at: Instant::now() - Duration::from_secs(300),
            clock: Clock::start(Some(DEFAULT_MS), Instant::now()),
            motion: Motion::settled_at(Instant::now()),
            drawn: None,
            open_drawn: 0.0,
            pane: None,
            held: None,
            hot: None,
            reply: None,
            pointer_in: false,
            serial: u64::from(id),
            tint: None,
            opening: SheetOpening::default(),
            bead: None,
            warm_gen: 0,
            glass: SheetGlass::default(),
            painted: None,
        }
    }

    /// Layers a worker painted for `b`'s Droplet, the job asking for them, and whether `b` would
    /// still send that job.
    fn warm_layers(b: &Balloon) -> (WarmLayers, impl Fn(&mut Balloon) -> bool + use<>) {
        let (theme, text) = (theme_for(Interface::Glass), text());
        let ends = droplet_ends(
            &theme,
            &text,
            &b.item,
            false,
            Edge::Bottom,
            SheetShape::Droplet,
            "now",
        );
        let [shut, open] = ends.unwrap();
        let icons = SheetIcons::default();
        let job = SheetOpening::default()
            .warm_job(
                &SheetGlass::default(),
                &theme,
                &shut,
                &open,
                icons,
                None,
                true,
            )
            .unwrap();
        let wants = move |b: &mut Balloon| {
            b.opening
                .warm_job(
                    &b.glass,
                    &theme,
                    &shut,
                    &open,
                    SheetIcons::default(),
                    None,
                    true,
                )
                .is_some()
        };
        (build_warm(job, &text), wants)
    }

    #[test]
    fn a_warm_result_for_an_earlier_job_is_dropped() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut stack = vec![balloon(&mut scene, 1)];
        stack[0].warm_gen = 2;
        let (built, wants) = warm_layers(&stack[0]);
        assert!(!land_warm(&mut stack, 1, 1, built));
        assert!(wants(&mut stack[0]), "nothing installed");
        let (built, wants) = warm_layers(&stack[0]);
        assert!(land_warm(&mut stack, 1, 2, built));
        assert!(!wants(&mut stack[0]), "the current job's layers are in");
    }

    #[test]
    fn a_warm_result_for_a_balloon_gone_is_dropped() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut stack = vec![balloon(&mut scene, 1)];
        let (built, _) = warm_layers(&stack[0]);
        assert!(!land_warm(&mut stack, 9, 0, built));
    }

    #[test]
    fn a_sliding_sheet_takes_the_pointer_where_it_is_drawn() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut b = balloon(&mut scene, 1);
        b.at = (100, 400);
        let t0 = Instant::now();
        b.motion.slide_by(90.0, t0, 250);
        assert_eq!(b.hit_at(t0), (100, 490));
        assert_eq!(b.hit_at(t0 + Duration::from_millis(300)), (100, 400));
    }

    /// Three Glass sheets stacked up from y 600, newest nearest the tray; returns the slot pitch.
    fn sheets_at(scene: &mut crate::compositor::DeskScene) -> (NotifyView, i32) {
        let theme = theme_for(Interface::Glass);
        let mut stack: Vec<Balloon> = [3, 2, 1].map(|id| balloon(scene, id)).into();
        let mut pitch = 0;
        for (i, b) in stack.iter_mut().enumerate() {
            let d = lay_out(
                &theme,
                &text(),
                &b.item,
                false,
                Edge::Bottom,
                SheetShape::Lozenge,
                false,
                "now",
            );
            pitch = d.footprint().1 + 8;
            b.at = (100, 600 - i as i32 * pitch);
            b.drawn = Some(d);
        }
        let v = NotifyView {
            stack,
            ..NotifyView::default()
        };
        (v, pitch)
    }

    #[test]
    fn closing_the_outermost_sheet_under_a_still_pointer_lets_the_stack_go() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, pitch) = sheets_at(&mut scene);
        let (x, y) = (200.0, f64::from(600 - 2 * pitch + 20));
        let now = Instant::now();
        point_at(&mut v, x, y, true, now);
        assert!(hovered(&v), "held while the pointer is on a sheet");
        v.stack.remove(2);
        assert!(hold_at(&mut v, x, y, now));
        assert!(!hovered(&v), "nothing rests under the pointer");
    }

    #[test]
    fn a_sheet_sliding_into_a_closed_slot_keeps_the_stack_held() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, pitch) = sheets_at(&mut scene);
        let (x, y) = (200.0, f64::from(600 - pitch + 20));
        let now = Instant::now();
        point_at(&mut v, x, y, true, now);
        v.stack.remove(1);
        v.stack[1].at.1 += pitch;
        v.stack[1].motion.slide_by(-f64::from(pitch), now, 250);
        assert!(!hold_at(&mut v, x, y, now));
        assert!(
            hovered(&v),
            "held while the older sheet slides under the pointer"
        );
    }

    #[test]
    fn a_halo_breakout_holds_the_stack_and_the_corner_beside_it_does_not() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut b = balloon(&mut scene, 1);
        b.at = (100, 600);
        b.drawn = Some(Drawn::Sheet(halo_sheet_layout(1).0));
        let mut v = NotifyView {
            stack: vec![b],
            ..NotifyView::default()
        };
        let now = Instant::now();
        let (well, beside) = ((130.0, 610.0), (400.0, 610.0));
        point_at(&mut v, well.0, well.1, true, now);
        assert!(hovered(&v), "the well above the sheet is the sheet");
        assert!(!hold_at(&mut v, well.0, well.1, now));
        assert!(hold_at(&mut v, beside.0, beside.1, now));
        assert!(!hovered(&v), "the bare corner right of the well lets go");
    }

    #[test]
    fn a_sheet_sliding_away_from_a_still_pointer_lets_the_stack_go_once_settled() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, pitch) = sheets_at(&mut scene);
        let (x, y) = (200.0, f64::from(600 - 2 * pitch + 20));
        let now = Instant::now();
        point_at(&mut v, x, y, true, now);
        v.stack.remove(0);
        for b in &mut v.stack {
            b.at.1 += pitch;
            b.motion.slide_by(-f64::from(pitch), now, 250);
        }
        assert!(!hold_at(&mut v, x, y, now));
        assert!(hovered(&v), "held while a sheet is drawn under the pointer");
        assert!(hold_at(&mut v, x, y, now + Duration::from_millis(300)));
        assert!(
            !hovered(&v),
            "let go once the sheets settle clear of the pointer"
        );
    }

    #[test]
    fn a_sheet_arriving_under_a_still_pointer_holds_the_stack() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, pitch) = sheets_at(&mut scene);
        let (x, y) = (200.0, f64::from(600 - pitch + 20));
        assert!(hold_at(&mut v, x, y, Instant::now()));
        assert!(hovered(&v), "held with no motion over the stack");
    }

    #[test]
    fn a_press_with_no_motion_before_it_holds_the_stack() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, pitch) = sheets_at(&mut scene);
        let (x, y) = (200.0, f64::from(600 - pitch + 20));
        let hit = press_target(&mut v, x, y, Instant::now());
        assert!(matches!(hit, Some((1, _, NotifyHit::Body, _))));
        assert!(hovered(&v), "held from the press on");
    }

    /// Layout point inside sheet `i`'s close spot, at its rest place.
    fn close_spot(v: &NotifyView, i: usize) -> (f64, f64) {
        let b = &v.stack[i];
        let d = b.drawn.as_ref().unwrap();
        let (c, (ox, oy)) = (d.parts().close, d.origin());
        (
            f64::from(b.at.0 + c.x - ox + 2),
            f64::from(b.at.1 + c.y - oy + 2),
        )
    }

    #[test]
    fn a_picture_beside_the_slot_leaves_the_slot_to_the_stock_icon() {
        let theme = theme_for(Interface::Glass);
        let mut n = note(1, "Music");
        n.image = vec![IconSource {
            width: 48,
            height: 48,
            pixels: vec![0xFF00_00FF; 48 * 48],
        }];
        let lay = |shape| match lay_out(
            &theme,
            &text(),
            &Item::Note(n.clone()),
            false,
            Edge::Bottom,
            shape,
            false,
            "now",
        ) {
            Drawn::Sheet(l) => l,
            Drawn::Balloon(_) => unreachable!("Glass draws sheets"),
        };
        assert!(!slot_needs_icon(&lay(SheetShape::Lozenge), true));
        assert!(slot_needs_icon(&lay(SheetShape::Droplet), true));
        assert!(slot_needs_icon(&lay(SheetShape::Halo), true));
        assert!(slot_needs_icon(&lay(SheetShape::Lozenge), false));
    }

    #[test]
    fn a_droplet_opening_under_a_still_pointer_keeps_the_stack_held() {
        let mut scene = crate::compositor::DeskScene::new();
        let theme = theme_for(Interface::Glass);
        let mut b = balloon(&mut scene, 1);
        if let Item::Note(n) = &mut b.item {
            n.actions = vec![("later".into(), "Later".into())];
            n.body = "Lunch at noon? The usual place by the river. Bring the photos.".into();
        }
        let lay = |t| {
            lay_out_open(
                &theme,
                &text(),
                &b.item,
                false,
                Edge::Bottom,
                SheetShape::Droplet,
                "now",
                t,
            )
            .0
        };
        let (shut, half) = (lay(0.0), lay(0.5));
        let bottom = 1000;
        b.at = (1550, bottom - shut.footprint().1);
        b.drawn = Some(shut);
        let mut v = NotifyView {
            stack: vec![b],
            ..NotifyView::default()
        };
        let (x, y) = (1700.0, f64::from(bottom - 10));
        let now = Instant::now();
        point_at(&mut v, x, y, true, now);
        v.stack[0].at.1 = bottom - half.footprint().1;
        v.stack[0].drawn = Some(half);
        assert!(!hold_at(&mut v, x, y, now));
        assert!(hovered(&v), "held while the sheet grows away from the bar");
        assert!(matches!(
            press_target(&mut v, x, y, now),
            Some((0, _, NotifyHit::Body, _))
        ));
    }

    #[test]
    fn a_close_press_on_an_opening_droplet_is_taken() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, _) = sheets_at(&mut scene);
        let now = Instant::now();
        let began = now - Duration::from_millis(100);
        v.stack[0].motion.open = Tween::new(0.0, 1.0, began, 250, Ease::OutCubic);
        let (x, y) = close_spot(&v, 0);
        assert!(matches!(
            press_target(&mut v, x, y, now),
            Some((0, _, NotifyHit::Close, _))
        ));
    }

    #[test]
    fn a_close_press_before_an_opening_droplet_shows_its_close_is_ignored() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, _) = sheets_at(&mut scene);
        let now = Instant::now();
        v.stack[0].motion.open = Tween::new(0.0, 1.0, now, 250, Ease::OutCubic);
        let (x, y) = close_spot(&v, 0);
        assert!(press_target(&mut v, x, y, now).is_none());
    }

    #[test]
    fn a_droplet_dismissed_mid_open_is_gone_on_its_close_time() {
        let (t0, e) = (Instant::now(), glass_effects());
        let mut m = Motion::settled_at(t0);
        m.aim_open(1.0, &e, t0);
        let t1 = t0 + Duration::from_millis(10);
        m.leave(&e, t1);
        let gone = t1 + Duration::from_millis(u64::from(e.window_close_ms));
        assert!(e.window_close_ms < e.window_open_ms - 10);
        assert!(m.settled(gone));
    }

    #[test]
    fn a_close_press_on_a_sliding_sheet_is_ignored_and_its_body_still_takes_one() {
        let mut scene = crate::compositor::DeskScene::new();
        let (mut v, _) = sheets_at(&mut scene);
        let now = Instant::now();
        let (x, y) = close_spot(&v, 0);
        assert!(matches!(
            press_target(&mut v, x, y, now),
            Some((0, _, NotifyHit::Close, _))
        ));
        v.stack[0].motion.slide_by(-10.0, now, 250);
        let y = y - 10.0;
        assert!(press_target(&mut v, x, y, now).is_none());
        assert!(matches!(
            press_target(&mut v, 200.0, y + 20.0, now),
            Some((0, _, NotifyHit::Body, _))
        ));
    }

    fn ids(items: &[Item]) -> Vec<u32> {
        items
            .iter()
            .filter_map(|i| i.note().map(|n| n.id))
            .collect()
    }

    #[test]
    fn the_newest_sheet_stays_nearest_the_tray_across_caps_3_1_3() {
        let mut stack = items(&[3, 2, 1]);
        let mut queue: VecDeque<Item> = items(&[4]).into();
        let mut set_cap = |stack: &mut Vec<Item>, cap| {
            for item in overflow(stack, cap, |_| false) {
                queue.push_front(item);
            }
            for item in fill(stack.len(), cap, &mut queue) {
                stack.insert(0, item);
            }
            ids(&queue.iter().cloned().collect::<Vec<_>>())
        };
        assert_eq!(set_cap(&mut stack, 1), vec![2, 3, 4]);
        assert_eq!(ids(&stack), vec![1]);
        assert_eq!(set_cap(&mut stack, 3), vec![4]);
        assert_eq!(ids(&stack), vec![3, 2, 1]);
    }

    #[test]
    fn a_smaller_cap_keeps_the_replying_sheet() {
        let mut stack = vec![1, 2, 3];
        assert_eq!(overflow(&mut stack, 1, |&n| n == 2), vec![1, 3]);
        assert_eq!(stack, vec![2]);
    }

    #[test]
    fn a_replace_redraws_in_its_slot_and_leaves_the_others() {
        let mut scene = crate::compositor::DeskScene::new();
        let mut stack: Vec<Balloon> = [1, 2, 3].map(|id| balloon(&mut scene, id)).into();
        let trees: Vec<TreeId> = stack.iter().map(|b| b.tree).collect();
        let mut n = note(2, "Mail");
        n.summary = "edited".into();
        renew(&mut stack[1], n, 9, None);
        let ids: Vec<u32> = stack
            .iter()
            .filter_map(|b| b.item.note())
            .map(|n| n.id)
            .collect();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(
            stack.iter().map(|b| b.tree).collect::<Vec<_>>(),
            trees,
            "same trees, no arrival"
        );
        assert_eq!(stack[1].item.note().unwrap().summary, "edited");
        assert_eq!(stack[1].serial, 9);
        assert!(
            stack[1].shown_at.elapsed() < Duration::from_secs(60),
            "its age starts over"
        );
        assert_eq!((stack[0].serial, stack[2].serial), (1, 3));
    }

    #[test]
    fn a_new_burst_takes_the_oldest_taken_slot() {
        let mut stack = vec!['a', 'b', 'c', 'd', 'e'];
        let gone = fold_in(&mut stack, &[1, 3], Some('B'));
        assert_eq!(stack, ['a', 'c', 'B', 'e']);
        assert_eq!(gone, ['d', 'b']);
        let mut stack = vec!['a', 'b', 'c'];
        fold_in(&mut stack, &[2], Some('B'));
        assert_eq!(stack, ['a', 'b', 'B']);
    }

    #[test]
    fn hiding_balloons_clears_every_visible_sheet() {
        let (mut stack, mut leaving) = (vec![1, 2, 3], vec![9]);
        assert_eq!(take_all(&mut stack, &mut leaving), vec![1, 2, 3, 9]);
        assert!(stack.is_empty() && leaving.is_empty());
    }

    fn glass_effects() -> EffectsConfig {
        EffectsConfig::default_for(Interface::Glass)
    }

    #[test]
    fn with_window_animations_off_sheets_appear_settled() {
        let t0 = Instant::now();
        let off = EffectsConfig {
            window_animation: false,
            ..glass_effects()
        };
        let m = Motion::arrive(&off, t0);
        assert!(m.settled(t0));
        assert_eq!(m.k.value(t0), 1.0);
    }

    #[test]
    fn a_sheet_arrives_over_the_window_open_time_and_leaves_over_the_close_time() {
        let (t0, e) = (Instant::now(), glass_effects());
        let mut m = Motion::arrive(&e, t0);
        assert_eq!(m.k.value(t0), 0.0);
        assert!(
            !m.settled(t0 + Duration::from_millis(249))
                && m.settled(t0 + Duration::from_millis(250))
        );
        let t1 = t0 + Duration::from_millis(300);
        m.leave(&e, t1);
        assert!(!m.settled(t1 + Duration::from_millis(199)));
        assert_eq!(m.k.value(t1 + Duration::from_millis(200)), 0.0);
    }

    #[test]
    fn a_slide_retargets_from_its_current_offset() {
        let t0 = Instant::now();
        let mut m = Motion::settled_at(t0);
        m.slide_by(104.0, t0, 250);
        assert_eq!(396.0 + m.offset(t0), 500.0, "starts where it was");
        let mid = t0 + Duration::from_millis(100);
        let seen = 396.0 + m.offset(mid);
        m.slide_by(-104.0, mid, 250);
        assert!((500.0 + m.offset(mid) - seen).abs() < 1e-9, "no jump");
        assert_eq!(m.offset(mid + Duration::from_millis(250)), 0.0);
    }

    #[test]
    fn a_repaint_in_the_same_slot_starts_no_slide_and_arms_no_tick() {
        let t0 = Instant::now();
        let mut scene = crate::compositor::DeskScene::new();
        let mut v = NotifyView::default();
        v.stack.push(balloon(&mut scene, 1));
        follow_slot(
            &mut v.stack[0].motion,
            (400, 100),
            (400, 100),
            false,
            t0,
            250,
        );
        assert!(still(&v, t0), "settled, so no tick is armed");
        follow_slot(
            &mut v.stack[0].motion,
            (400, 100),
            (296, 100),
            false,
            t0,
            250,
        );
        assert!(!still(&v, t0));
        assert_eq!(v.stack[0].motion.offset(t0), 104.0);
    }

    #[test]
    fn a_sheet_growing_in_place_keeps_its_anchored_edge_still() {
        let t0 = Instant::now();
        let mut up = Motion::settled_at(t0);
        follow_slot(&mut up, (400, 100), (360, 140), false, t0, 250);
        assert!(up.settled(t0), "a stack growing up holds the bottom");
        let mut down = Motion::settled_at(t0);
        follow_slot(&mut down, (40, 100), (40, 140), true, t0, 250);
        assert!(down.settled(t0), "a stack growing down holds the top");
        follow_slot(&mut down, (40, 100), (80, 100), true, t0, 250);
        assert_eq!(down.offset(t0), -40.0);
    }

    #[test]
    fn a_hovered_droplet_opens_and_a_replying_one_stays_open() {
        assert_eq!(
            [
                open_target(true, false),
                open_target(false, true),
                open_target(false, false)
            ],
            [1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn a_droplet_arrives_over_the_drip_time() {
        let (t0, e) = (Instant::now(), glass_effects());
        let m = Motion::drip(&e, t0);
        assert!(
            !m.settled(t0 + Duration::from_millis(599))
                && m.settled(t0 + Duration::from_millis(600))
        );
        let off = EffectsConfig {
            window_animation: false,
            ..e
        };
        assert!(Motion::drip(&off, t0).settled(t0));
    }

    #[test]
    fn a_leaving_droplet_poses_like_any_sheet() {
        let (t0, e) = (Instant::now(), glass_effects());
        let mut m = Motion::drip(&e, t0);
        assert!(m.drip);
        m.leave(&e, t0 + Duration::from_millis(100));
        assert!(!m.drip);
    }

    #[test]
    fn a_droplet_opens_over_the_open_time_and_closes_over_the_close_time() {
        let (t0, e) = (Instant::now(), glass_effects());
        let ms = Duration::from_millis;
        let mut m = Motion::settled_at(t0);
        assert!(m.aim_open(1.0, &e, t0));
        assert!(!m.aim_open(1.0, &e, t0 + ms(10)), "already on its way");
        assert!(!m.settled(t0 + ms(249)) && m.settled(t0 + ms(250)));
        assert_eq!(m.open.value(t0 + ms(250)), 1.0);
        let t1 = t0 + ms(300);
        m.aim_open(0.0, &e, t1);
        assert!(!m.settled(t1 + ms(199)));
        assert_eq!(m.open.value(t1 + ms(200)), 0.0);
        let off = EffectsConfig {
            window_animation: false,
            ..e
        };
        let mut m = Motion::settled_at(t0);
        m.aim_open(1.0, &off, t0);
        assert!(m.settled(t0));
    }

    fn droplet_note() -> Item {
        let mut n = note(1, "Chat");
        n.body = "Lunch at noon?\nThe usual place.\nBring the photos.".into();
        Item::Note(n)
    }

    fn droplet_at(t: f64, shape: SheetShape) -> (SheetLayout, Option<[SheetLayout; 2]>) {
        let (drawn, ends) = lay_out_open(
            &theme_for(Interface::Glass),
            &text(),
            &droplet_note(),
            false,
            Edge::Bottom,
            shape,
            "now",
            t,
        );
        let Drawn::Sheet(lay) = drawn else {
            panic!("a sheet")
        };
        (lay, ends)
    }

    #[test]
    fn an_opening_droplet_lays_out_between_its_ends() {
        let (shut, ends) = droplet_at(0.0, SheetShape::Droplet);
        assert!(!shut.expanded);
        assert_eq!(shut.sheet.height, 68);
        assert_eq!(ends.as_ref().map(|[c, _]| c), Some(&shut));
        let (open, ends) = droplet_at(1.0, SheetShape::Droplet);
        assert!(open.expanded);
        assert_eq!(ends.as_ref().map(|[_, e]| e), Some(&open));
        let (half, ends) = droplet_at(0.5, SheetShape::Droplet);
        let [c, e] = ends.expect("both ends while it opens");
        assert_eq!((c, &e), (shut, &open));
        assert!(half.sheet.height > 68 && half.sheet.height < e.sheet.height);
        let (lozenge, none) = droplet_at(0.5, SheetShape::Lozenge);
        assert!(none.is_none() && !lozenge.expanded);
    }

    #[test]
    fn a_droplet_opening_by_the_taskbar_keeps_its_bottom_still() {
        let t0 = Instant::now();
        let area = Rect::new(0, 0, 1920, 1040);
        let size = |t| {
            let fp = droplet_at(t, SheetShape::Droplet).0.footprint;
            (fp.width, fp.height)
        };
        let (shut, open) = (size(0.0), size(1.0));
        let slot = |s: (i32, i32)| place_stack(area, &[s], None, Edge::Bottom).0[0];
        let (was, now_at) = (slot(shut), slot(open));
        assert_eq!(was.1 + shut.1, now_at.1 + open.1);
        let mut m = Motion::settled_at(t0);
        follow_slot(&mut m, (was.1, shut.1), (now_at.1, open.1), false, t0, 250);
        assert!(m.settled(t0), "no dip toward the taskbar");
    }

    #[test]
    fn a_droplet_pane_is_its_footprint_with_the_droplet_corner() {
        let t = theme_for(Interface::Glass).scaled(2);
        let Drawn::Sheet(lay) = lay_out(
            &t,
            &text(),
            &droplet_note(),
            false,
            Edge::Bottom,
            SheetShape::Droplet,
            false,
            "now",
        ) else {
            panic!()
        };
        let pane = sheet_glass(&lay, t.look.style().unwrap().notify.as_ref().unwrap(), 2);
        assert_eq!(pane.shapes.len(), 1);
        assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, 360, 68));
        assert_eq!(
            pane.shapes[0].corner,
            &reprisekit::glass::notify::DROPLET_CORNER[..]
        );
    }

    #[test]
    fn critical_and_unnamed_notifications_never_fold() {
        assert!(foldable(&note(1, "Mail")));
        assert!(!foldable(&note(1, "")));
        let mut crit = note(1, "Mail");
        crit.urgency = Urgency::Critical;
        assert!(!foldable(&crit));
        let mut own = note(OWN_ID, OWN_APP);
        own.id = OWN_ID;
        assert!(!foldable(&own));
    }

    /// Each tree's index among the scene root's children, bottom first.
    fn depth(scene: &crate::compositor::DeskScene, b: &Balloon) -> usize {
        let root = scene.root();
        let at = scene
            .children(root)
            .iter()
            .position(|c| *c == b.tree.into());
        at.expect("a sheet's tree under the root")
    }

    #[test]
    fn a_closed_sheet_goes_under_every_sheet_in_the_stack() {
        let mut scene = crate::compositor::DeskScene::new();
        let now = Instant::now();
        let mut stack: Vec<Balloon> = (1..=3).map(|id| balloon(&mut scene, id)).collect();
        let mut gone = stack.remove(1);
        gone.motion.leave(&glass_effects(), now);
        let leaving = vec![gone];
        restack(&mut scene, &stack, &leaving, now);
        let low = depth(&scene, &leaving[0]);
        assert!(stack.iter().all(|b| depth(&scene, b) > low));
        assert_eq!(low, 0, "the leaving tree is the lowest of the three");
    }

    #[test]
    fn a_foreign_overlay_node_between_sheets_keeps_its_index() {
        let mut scene = crate::compositor::DeskScene::new();
        let now = Instant::now();
        let mut gone = balloon(&mut scene, 1);
        let foreign = scene.tree_create(scene.root());
        let stack = vec![balloon(&mut scene, 2)];
        gone.motion.leave(&glass_effects(), now);
        let mut new = balloon(&mut scene, 3);
        new.motion = Motion::arrive(&glass_effects(), now);
        let stack = vec![new, stack.into_iter().next().unwrap()];
        restack(&mut scene, &stack, &[gone], now);
        let root = scene.root();
        let at = scene
            .children(root)
            .iter()
            .position(|c| *c == foreign.into());
        assert_eq!(at, Some(1), "the foreign node stays second from the bottom");
    }

    #[test]
    fn an_arriving_sheet_sits_under_resting_ones_and_over_leaving_ones() {
        let mut scene = crate::compositor::DeskScene::new();
        let now = Instant::now();
        let mut leaving = vec![balloon(&mut scene, 1)];
        leaving[0].motion.leave(&glass_effects(), now);
        let mut stack = vec![balloon(&mut scene, 2), balloon(&mut scene, 3)];
        let mut new = balloon(&mut scene, 4);
        new.motion = Motion::arrive(&glass_effects(), now);
        stack.insert(0, new);
        restack(&mut scene, &stack, &leaving, now);
        let order: Vec<usize> = [&leaving[0], &stack[0], &stack[1], &stack[2]]
            .iter()
            .map(|b| depth(&scene, b))
            .collect();
        assert_eq!(
            order,
            vec![0, 1, 2, 3],
            "leaving, arriving, then the resting two"
        );
        let done = now + Duration::from_secs(10);
        assert_eq!(band(&stack[0].motion, false, done), Band::Resting);
    }
}
