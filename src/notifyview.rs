//! Desktop notifications: a tray balloon or a stack of sheets, the rest queued, and the history
//! popup. One stays open while shown, queued or in the history, so its actions still run there.

mod access;
mod clock;
mod layout;
mod motion;
mod paint;
mod pointer;
mod reply;
mod stack;
mod warm;

#[cfg(test)]
mod tests;

pub use access::access;
pub use pointer::{cancel_press, drag, hover, press, release};
pub use reply::{blink, blur, key, paste, reply_edit, reply_focused};
pub use stack::{output_removed, reposition};

use reprisekit::controls::notifysheet::{SheetGlass, SheetOpening, WarmJob, has_sheets};
use reprisekit::controls::tooltip::{NotifyHit, ReplyField};
use reprisekit::style::SheetShape;
use reprisekit::taskbar::Edge;
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::utils::{Logical, Rectangle};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::compositor::{SceneOwner, State};
use crate::glass::{Backdrop, Pane};
use crate::notifylog::{Log, Suppress, shows_balloon};
use crate::raster::Rect;
use crate::startmenu::MenuItem;
use crate::theme::Color;
use crate::tray::notify::{DEFAULT_ACTION, INLINE_REPLY, Urgency};
use crate::tray::{Notification, TrayCommand};

use clock::{Clock, reply_idle_ms, restart};
use layout::{Drawn, Painted, sheet_tint};
use motion::{Motion, destroy, drop_leaving, leave};
use paint::paint;
use reply::start_reply;
use stack::{Item, arrived, display_next, enqueue, fold, foldable, in_burst, slot_of, take_all};

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
const MAX_QUEUED: usize = 8;

/// How many balloons one program raises within `BURST_WINDOW` before the next folds them into one
/// "N new from App" balloon.
const BURST_AFTER: usize = 3;
const BURST_WINDOW: Duration = Duration::from_secs(10);
const BURST_LINES: usize = 3;

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
    /// Wakes at each minute boundary while `[notifications] quiet_hours` is set.
    minute_timer: Option<RegistrationToken>,
    /// The bar edge the stack was last placed by.
    edge: Option<Edge>,
    /// The output the stack was last placed on; its sheets and chip draw only inside it.
    output: Option<Output>,
    /// Outputs the stack left that still show it until their next frame.
    left: Vec<Output>,
    /// A sheet or the chip is under the pointer, drawn or at rest, as of its last move, press or
    /// paint: a sheet sliding into a closed one's slot keeps the stack held.
    pointer_on: bool,
    /// [`warm::warm_openings`] waits for the loop to go idle.
    warm_queued: bool,
    /// The opening layers' painter thread, started by the first warm job, and its result source.
    warmer: Option<std::sync::mpsc::Sender<(u64, u64, WarmJob)>>,
    warm_source: Option<RegistrationToken>,
    /// Painter threads started; past [`warm::WARM_SPAWNS`] every opening paints in frame.
    warm_spawns: u32,
    pub log: Log,
    /// The history popup, while open.
    pub center: Option<crate::notifycenter::CenterView>,
    /// A wheel's sub-notch remainder over the history popup.
    pub center_wheel: i32,
    shown: u64,
    /// The conditions that turn Do Not Disturb on by themselves, as last checked; the bell shows
    /// them.
    pub quiet: Suppress,
}

impl NotifyView {
    /// The layout rect of the output the stack is placed on; sheets and the chip clip to it.
    fn bounds(&self) -> Option<Rectangle<i32, Logical>> {
        self.output.as_ref().and_then(reprisescene::output_rect)
    }
}

/// How many notifications show at once: a stack of sheets, else one balloon.
fn visible_cap(sheets: bool) -> usize {
    if sheets { 3 } else { 1 }
}

fn sheets(state: &State) -> bool {
    has_sheets(&state.config.theme)
}

fn cap(state: &State) -> usize {
    visible_cap(sheets(state))
}

fn droplets(state: &State) -> bool {
    sheets(state) && state.config.glass.notification.shape() == SheetShape::Droplet
}

/// A tree in the overlay for a balloon or the chip, left out of the glass.
fn overlay_tree(state: &mut State) -> TreeId {
    let tree = state.scene.tree_create(state.layers.overlay);
    crate::glass::leave_out(&mut state.scene, tree);
    state.scene.set_data(tree, Some(SceneOwner::Notification));
    tree
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

/// Whether a balloon is on screen.
pub fn balloon_up(state: &State) -> bool {
    !state.notify.stack.is_empty()
}

/// Seconds since the epoch, for the history's time column.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// What holds balloons back now: Do Not Disturb, a focused full-screen window and a screen being
/// shared (with `auto_dnd`), the quiet hours.
fn suppress(state: &State) -> Suppress {
    let tm = reprisefs::locale::localtime(now());
    let minute = (tm.tm_hour * 60 + tm.tm_min) as u32;
    let focused = state.focused.and_then(|w| state.win(w));
    let auto = state.config.notifications.auto_dnd;
    Suppress {
        fullscreen_focus: auto && focused.is_some_and(|w| w.fullscreen && w.mapped && !w.minimized),
        sharing: auto && state.privacy.sharing_shown(),
        scheduled: state.config.notifications.quiet_at(minute),
        manual: state.notify.log.dnd,
    }
}

/// The focus, a capture or the clock moved: the bell redraws when the automatic Do Not Disturb
/// changed, and the balloons it held show once nothing holds them.
pub fn suppress_changed(state: &mut State) {
    let mut s = suppress(state);
    let manual = std::mem::take(&mut s.manual);
    if s != state.notify.quiet {
        state.notify.quiet = s;
        state.redraw_taskbars();
    }
    let idle = !manual && !s.automatic() && !crate::notifycenter::is_open(state);
    if !idle || state.notify.log.held.is_empty() {
        return;
    }
    for n in state.notify.log.release() {
        enqueue(&mut state.notify.queue, Item::Note(n));
    }
    display_next(state);
    changed(state);
}

/// Check [`suppress_changed`] at every minute boundary while quiet hours are set, and once now.
/// Runs at startup and on each config reload.
pub fn config_changed(state: &mut State) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let want = state.config.notifications.quiet_hours.is_some();
    match (want, state.notify.minute_timer.is_some()) {
        (true, false) => {
            let to_next_minute = || Duration::from_secs((60 - now().rem_euclid(60)).max(1) as u64);
            let timer = state.handle.insert_source(
                Timer::from_duration(to_next_minute()),
                move |_, _, state| {
                    suppress_changed(state);
                    TimeoutAction::ToDuration(to_next_minute())
                },
            );
            match timer {
                Ok(t) => state.notify.minute_timer = Some(t),
                Err(e) => log::warn!("notifications: no minute timer: {}", e.error),
            }
        }
        (false, true) => {
            if let Some(t) = state.notify.minute_timer.take() {
                state.handle.remove(t);
            }
        }
        _ => {}
    }
    suppress_changed(state);
}

/// Notification `n` arrived: into the history unless it is transient, and a balloon unless Do Not
/// Disturb holds it back or the history is open, where it shows as a row instead. One the automatic
/// Do Not Disturb holds back waits to show until it ends. A replace swaps in place, keeping a reply
/// being typed.
pub fn show(state: &mut State, n: Notification) {
    let looking = crate::notifycenter::is_open(state);
    if !n.transient {
        for id in state.notify.log.record(&n, now(), looking) {
            drop_balloon(state, id);
            crate::tray::notification_closed(state, id, REASON_EXPIRED);
        }
    }
    let s = suppress(state);
    if looking || !shows_balloon(&n, s) {
        drop_balloon(state, n.id);
        if !looking && s.holds(&n) {
            state.notify.log.hold(n);
        } else if n.transient {
            crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
        }
        changed(state);
        return;
    }
    state.notify.log.unhold(n.id);
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
    if let Some(sound) = balloon_sound(&n, state.config.notifications.sound) {
        repriseaudio::sounds::play(sound.clone(), state.config.sounds.theme.clone());
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
    v.log.unhold(id);
    if let Some(i) = v.stack.iter().position(|b| b.item.is(id)) {
        let b = v.stack.remove(i);
        leave(state, b);
        display_next(state);
        return true;
    }
    if let Some(i) = v.stack.iter().position(|b| b.item.burst_has(id)) {
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

/// Clear All in the history: every notification in it still open is dismissed, and the history
/// empties.
pub fn clear_history(state: &mut State) {
    let ids = state.notify.log.clear();
    dismiss_cleared(state, ids);
}

/// A group's Clear in the history: every notification from `app` leaves it, those still open
/// dismissed.
pub fn clear_app(state: &mut State, app: &str) {
    let ids = state.notify.log.clear_app(app);
    dismiss_cleared(state, ids);
}

/// `ids` just left the history: their balloons go and those still open are dismissed.
fn dismiss_cleared(state: &mut State, ids: Vec<u32>) {
    for id in ids {
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
        v.log.held.clear();
        let manual = Suppress {
            manual: true,
            ..Suppress::default()
        };
        let held = |item: &Item| match item {
            Item::Burst(_) => true,
            Item::Note(n) => !shows_balloon(n, manual) && !n.transient,
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
    state.notify.log.held.clear();
    let mut gone: Vec<Item> = state.notify.queue.drain(..).collect();
    gone.extend(take_down(state));
    close_transient(state, &gone);
}

/// A notification's sender left the bus: its actions have nobody to run.
pub fn sender_gone(state: &mut State, sender: &str) {
    state.notify.log.sender_gone(sender);
    crate::notifycenter::refresh(state);
}

/// What a new balloon for `n` plays with `[notifications] sound` at `on`: a low urgency one is silent.
fn balloon_sound(n: &Notification, on: bool) -> Option<&repriseaudio::sounds::Sound> {
    n.sound.as_ref().filter(|_| on && n.urgency != Urgency::Low)
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
            MenuItem::entry(label)
        })
        .collect()
}
