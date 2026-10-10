//! Desktop notifications: tray balloons one at a time, the rest queued, and the history popup.
//!
//! A notification stays open (no `NotificationClosed`) while its balloon is up or queued or it is
//! in the history, so its actions still run from there. The balloon lives in `layers.overlay`,
//! tagged `SceneOwner::Notification`, so the lock screen covers it.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use reprisekit::controls::tooltip::{
    NOTIFY_BUTTONS, NotifyHit, NotifyLayout, NotifyReply, ReplyEvent, ReplyField, ReplyKey,
    draw_notify_balloon, notify_hit, notify_layout, place_notify,
};
use reprisekit::icon::IconSource;
use reprisekit::taskbar::Edge;
use reprisekit::textedit::{Caret, EditKey, TextEdit};
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::caret::Editor;
use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::input::BTN_LEFT;
use crate::notifylog::Log;
use crate::output::upload_scaled;
use crate::startmenu::MenuItem;
use crate::text::TextRenderer;
use crate::theme::Theme;
use crate::tray::notify::{DEFAULT_ACTION, INLINE_REPLY, Urgency};
use crate::tray::{Notification, TrayCommand};

/// Default and clamp for how long a balloon stays up (ms). A request of 0 ("never") is held to
/// `MAX_MS`; only a critical notification stays until dealt with.
const DEFAULT_MS: i32 = 7000;
const MIN_MS: i32 = 3000;
const MAX_MS: i32 = 20000;

/// freedesktop `NotificationClosed` reason codes.
pub const REASON_EXPIRED: u32 = 1;
pub const REASON_DISMISSED: u32 = 2;
pub const REASON_CLOSED: u32 = 3;
pub const REASON_UNDEFINED: u32 = 4;

/// Balloons waiting behind the active one; each holds the screen at least `MIN_MS`, so a flood must
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
            self.icon = n.icon.clone();
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
            Item::Note(n) => &n.icon,
            Item::Burst(b) => &b.icon,
        }
    }
}

struct Balloon {
    item: Item,
    at: (i32, i32),
    scale: i32,
    tree: TreeId,
    buffer: BufferId,
    held: Option<(NotifyHit, bool)>,
    hot: Option<NotifyHit>,
    reply: Option<ReplyField>,
}

#[derive(Default)]
pub struct NotifyView {
    active: Option<Balloon>,
    queue: VecDeque<Item>,
    recent: VecDeque<(String, Instant)>,
    /// The `default` action of the compositor's own notice; any other notice clears it.
    own_click: Option<Vec<OsString>>,
    timer: Option<RegistrationToken>,
    pub log: Log,
    /// The history popup, while open.
    pub center: Option<crate::notifycenter::CenterView>,
    /// A wheel's sub-notch remainder over the history popup.
    pub center_wheel: i32,
}

fn arm(state: &mut State, ms: Option<i32>) {
    if let Some(t) = state.notify.timer.take() {
        state.handle.remove(t);
    }
    let Some(ms) = ms.filter(|&ms| ms > 0) else {
        return;
    };
    let timer = Timer::from_duration(Duration::from_millis(ms as u64));
    match state.handle.insert_source(timer, |_, _, state| {
        state.notify.timer = None;
        expired(state);
        TimeoutAction::Drop
    }) {
        Ok(t) => state.notify.timer = Some(t),
        Err(e) => log::warn!("notification timer: {}", e.error),
    }
}

/// Whether a balloon is on screen.
pub fn balloon_up(state: &State) -> bool {
    state.notify.active.is_some()
}

/// A transient notification closes with its balloon; any other stays open in the history.
fn expired(state: &mut State) {
    let Some(b) = state.notify.active.take() else {
        return;
    };
    let gone = b.item.note().filter(|n| n.transient).map(|n| n.id);
    destroy(state, b);
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
    if state
        .notify
        .active
        .as_ref()
        .is_some_and(|b| b.item.is(n.id))
    {
        let reply = state.notify.active.take().and_then(|b| {
            let r = b.reply.clone();
            destroy(state, b);
            r
        });
        let keeps = reply.filter(|_| n.reply.is_some());
        display(state, Item::Note(n));
        if let Some(r) = keeps {
            expand(state, r);
        }
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
            || v.active.as_ref().is_some_and(|b| b.item.burst_of(&n.app));
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
    if state.notify.active.is_none() {
        display_next(state);
    }
    changed(state);
}

fn foldable(n: &Notification) -> bool {
    !n.app.is_empty() && n.id != OWN_ID && n.urgency != Urgency::Critical
}

fn in_burst(v: &NotifyView, id: u32) -> bool {
    let has = |i: &Item| matches!(i, Item::Burst(b) if b.ids.contains(&id));
    v.active.as_ref().is_some_and(|b| has(&b.item)) || v.queue.iter().any(has)
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

/// Fold `n` into its program's burst: the one on screen or queued counts it, else a new one takes
/// in the program's queued balloons and the one on screen (unless a reply is being written there),
/// in the place of the first of them.
fn fold(state: &mut State, n: Notification) {
    let app = n.app.clone();
    let mut expired_ids = Vec::new();
    if n.transient {
        expired_ids.push(n.id);
    }
    let v = &mut state.notify;
    if let Some(Item::Burst(b)) = v.active.as_mut().map(|b| &mut b.item)
        && b.app == app
    {
        b.add(&n);
        arm(state, Some(DEFAULT_MS));
        reposition(state);
    } else if let Some(Item::Burst(b)) = v.queue.iter_mut().find(|q| q.burst_of(&app)) {
        b.add(&n);
    } else {
        let mut burst = Burst::new(&app);
        let takes_active = v
            .active
            .as_ref()
            .is_some_and(|b| b.reply.is_none() && b.item.note().is_some_and(|a| a.app == app));
        if takes_active && let Some(Item::Note(a)) = v.active.as_ref().map(|b| &b.item) {
            burst.add(a);
            if a.transient {
                expired_ids.push(a.id);
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
        if takes_active {
            if let Some(b) = state.notify.active.take() {
                destroy(state, b);
            }
            display(state, Item::Burst(burst));
        } else {
            let q = &mut state.notify.queue;
            q.insert(at.unwrap_or(q.len()), Item::Burst(burst));
            while q.len() > MAX_QUEUED {
                q.pop_front();
            }
            if state.notify.active.is_none() {
                display_next(state);
            }
        }
    }
    for id in expired_ids {
        crate::tray::notification_closed(state, id, REASON_EXPIRED);
    }
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
    if state.notify.active.as_ref().is_some_and(|b| b.item.is(id)) {
        if let Some(b) = state.notify.active.take() {
            destroy(state, b);
        }
        arm(state, None);
        display_next(state);
        return true;
    }
    if let Some(Item::Burst(b)) = state.notify.active.as_mut().map(|b| &mut b.item)
        && b.ids.contains(&id)
    {
        b.ids.retain(|&i| i != id);
        if b.ids.is_empty() {
            if let Some(b) = state.notify.active.take() {
                destroy(state, b);
            }
            arm(state, None);
            display_next(state);
        } else {
            paint(state);
        }
        return false;
    }
    let mut own = false;
    state.notify.queue.retain_mut(|q| match q {
        Item::Note(n) => {
            own |= n.id == id;
            n.id != id
        }
        Item::Burst(b) => {
            b.ids.retain(|&i| i != id);
            !b.ids.is_empty()
        }
    });
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
        .active
        .as_ref()
        .and_then(|b| b.item.note())
        .filter(|n| n.id == id && n.has_action(key));
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

fn start_reply(state: &mut State, id: u32) {
    if state.notify.active.as_ref().is_some_and(|b| b.item.is(id)) {
        let focused = ReplyField {
            focused: true,
            ..ReplyField::default()
        };
        expand(state, focused);
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
    if let Some(b) = state.notify.active.take() {
        let item = b.item.clone();
        destroy(state, b);
        arm(state, None);
        state.notify.queue.push_front(item);
    }
    drop_balloon(state, id);
    display(state, Item::Note(n));
    let focused = ReplyField {
        focused: true,
        ..ReplyField::default()
    };
    expand(state, focused);
}

fn expand(state: &mut State, field: ReplyField) {
    let Some(b) = state.notify.active.as_mut() else {
        return;
    };
    if b.item.note().is_none_or(|n| n.reply.is_none()) {
        return;
    }
    let focused = field.focused;
    b.reply = Some(field);
    arm(state, None);
    if focused {
        state.restart_caret();
    }
    paint(state);
}

fn collapse(state: &mut State) {
    let Some(b) = state.notify.active.as_mut() else {
        return;
    };
    if b.reply.take().is_none() {
        return;
    }
    let ms = b.item.note().and_then(timeout_ms);
    arm(state, ms);
    paint(state);
}

/// Send `text` as the active balloon's reply: `NotificationReplied` fires, and the notification
/// closes unless it is resident, whose balloon goes.
fn send_reply(state: &mut State, text: String) {
    let Some((id, resident)) = state
        .notify
        .active
        .as_ref()
        .and_then(|b| b.item.note())
        .map(|n| (n.id, n.resident))
    else {
        return;
    };
    crate::tray::send(state, TrayCommand::Replied { id, text });
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
    let r = state.notify.active.as_ref()?.reply.as_ref()?;
    r.focused.then_some(&r.edit)
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
    let Some(field) = state.notify.active.as_mut().and_then(|b| b.reply.as_mut()) else {
        return;
    };
    let event = field.key(key);
    state.restart_caret();
    match event {
        ReplyEvent::None => {}
        ReplyEvent::Redraw => paint(state),
        ReplyEvent::Send(text) => send_reply(state, text),
        ReplyEvent::Collapse => collapse(state),
    }
}

/// Text from the clipboard into the reply field.
pub fn paste(state: &mut State, text: &str) {
    if let Some(field) = state.notify.active.as_mut().and_then(|b| b.reply.as_mut())
        && field.paste(text)
    {
        paint(state);
    }
}

/// A press anywhere off the balloon: its reply field lets go of the keyboard and stays open.
pub fn blur(state: &mut State) {
    if let Some(field) = state.notify.active.as_mut().and_then(|b| b.reply.as_mut())
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
    changed(state);
}

/// A group's Clear in the history: every notification from `app` leaves it, those still open
/// dismissed.
pub fn clear_app(state: &mut State, app: &str) {
    for id in state.notify.log.clear_app(app) {
        drop_balloon(state, id);
        crate::tray::notification_closed(state, id, REASON_DISMISSED);
    }
    changed(state);
}

/// Do Not Disturb on or off.
/// Turning it on takes down the balloons it holds back; they stay open in the history.
pub fn set_dnd(state: &mut State, on: bool) {
    state.notify.log.dnd = on;
    if on {
        let v = &mut state.notify;
        v.queue.retain(|q| !matches!(q, Item::Burst(_)));
        if v.active
            .as_ref()
            .is_some_and(|b| matches!(b.item, Item::Burst(_)))
        {
            if let Some(b) = state.notify.active.take() {
                destroy(state, b);
            }
            arm(state, None);
        }
        let held: Vec<u32> = {
            let v = &state.notify;
            v.active
                .iter()
                .map(|b| &b.item)
                .chain(v.queue.iter())
                .filter_map(Item::note)
                .filter(|n| !v.log.shows_balloon(n) && !n.transient)
                .map(|n| n.id)
                .collect()
        };
        for id in held {
            drop_balloon(state, id);
        }
        if state.notify.active.is_none() {
            display_next(state);
        }
    }
    log::info!("do not disturb {}", if on { "on" } else { "off" });
    changed(state);
}

/// The history opened over the balloons' corner: every balloon comes down.
pub fn hide_balloons(state: &mut State) {
    let mut gone: Vec<Item> = state.notify.queue.drain(..).collect();
    if let Some(b) = state.notify.active.take() {
        gone.push(b.item.clone());
        destroy(state, b);
        arm(state, None);
    }
    for n in gone.iter().filter_map(Item::note).filter(|n| n.transient) {
        crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
    }
}

/// A notification's sender left the bus: its actions have nobody to run.
pub fn sender_gone(state: &mut State, sender: &str) {
    state.notify.log.sender_gone(sender);
    crate::notifycenter::refresh(state);
}

fn balloon_hit(state: &State, x: f64, y: f64) -> Option<(NotifyLayout, NotifyHit, i32)> {
    let b = state.notify.active.as_ref()?;
    let lay = balloon_layout(state, &state.first_output()?, b, b.scale);
    let lx = ((x - f64::from(b.at.0)) * f64::from(b.scale)).floor() as i32;
    let ly = ((y - f64::from(b.at.1)) * f64::from(b.scale)).floor() as i32;
    let hit = notify_hit(&lay, lx, ly);
    Some((lay, hit, lx))
}

/// A press of `button` on the balloon at layout point (`x`, `y`); true when the pointer now holds
/// a button. Any button but the left dismisses it.
pub fn press(state: &mut State, x: f64, y: f64, button: u32) -> bool {
    let Some((lay, hit, lx)) = balloon_hit(state, x, y) else {
        return false;
    };
    let hit = if button == BTN_LEFT {
        hit
    } else {
        NotifyHit::Close
    };
    let Some(item) = state.notify.active.as_ref().map(|b| b.item.clone()) else {
        return false;
    };
    let n = match item {
        Item::Burst(burst) => {
            if let Some(b) = state.notify.active.take() {
                destroy(state, b);
            }
            arm(state, None);
            if hit == NotifyHit::Close {
                display_next(state);
            } else {
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
            if let Some(b) = state.notify.active.as_mut() {
                b.held = Some((hit, true));
            }
            paint(state);
            return true;
        }
        NotifyHit::Field => {
            let theme = state.config.theme.scaled(lay_scale(state));
            if let Some(field) = state.notify.active.as_mut().and_then(|b| b.reply.as_mut()) {
                field.click(&theme, &state.text, &lay, lx);
            }
            state.restart_caret();
            paint(state);
        }
        NotifyHit::More => open_more_menu(state, id),
    }
    false
}

fn lay_scale(state: &State) -> i32 {
    state.notify.active.as_ref().map_or(1, |b| b.scale)
}

/// The pointer moved, `over` the balloon or not: the close box, the button or the reply field under
/// it lights.
pub fn hover(state: &mut State, x: f64, y: f64, over: bool) {
    let hit = if over {
        balloon_hit(state, x, y).map(|(_, h, _)| h)
    } else {
        None
    };
    let hot = hit.filter(|h| *h != NotifyHit::Body);
    let Some(b) = state.notify.active.as_mut() else {
        return;
    };
    if b.hot != hot {
        b.hot = hot;
        paint(state);
    }
}

/// The pointer moved while it holds a balloon button: the button shows pushed only while the
/// pointer is over it.
pub fn drag(state: &mut State, x: f64, y: f64) {
    let Some((part, down)) = state.notify.active.as_ref().and_then(|b| b.held) else {
        return;
    };
    let over = balloon_hit(state, x, y).is_some_and(|(_, h, _)| h == part);
    if over != down {
        if let Some(b) = state.notify.active.as_mut() {
            b.held = Some((part, over));
        }
        paint(state);
    }
}

/// The pointer let go of a balloon button: its action runs, or Send sends, if the release is over
/// it.
pub fn release(state: &mut State, x: f64, y: f64) {
    let Some(b) = state.notify.active.as_mut() else {
        return;
    };
    let Some((part, _)) = b.held.take() else {
        return;
    };
    let over = balloon_hit(state, x, y).is_some_and(|(_, h, _)| h == part);
    paint(state);
    if !over {
        return;
    }
    let Some(b) = state.notify.active.as_ref() else {
        return;
    };
    match part {
        NotifyHit::Send => {
            if let Some(text) = b.reply.as_ref().and_then(ReplyField::to_send) {
                send_reply(state, text);
            }
        }
        NotifyHit::Button(i) => {
            let Some(n) = b.item.note() else {
                return;
            };
            let (id, key) = (n.id, n.button_actions().nth(i).map(|(k, _)| k.clone()));
            if let Some(key) = key {
                invoke(state, id, &key);
            }
        }
        _ => {}
    }
}

/// The pointer is taken away mid-press: a held balloon button lets go without acting.
pub fn cancel_press(state: &mut State) {
    if let Some(b) = state.notify.active.as_mut()
        && b.held.take().is_some()
    {
        paint(state);
    }
}

/// Open the "..." button's menu: the actions past the balloon's buttons, its right edge at the
/// balloon's left so the balloon does not cover it.
fn open_more_menu(state: &mut State, id: u32) {
    let (Some(b), Some(output)) = (state.notify.active.as_ref(), state.first_output()) else {
        return;
    };
    let Some(n) = b.item.note() else {
        return;
    };
    let rest: Vec<(String, String)> = n.button_actions().skip(NOTIFY_BUTTONS).cloned().collect();
    let lay = balloon_layout(state, &output, b, b.scale);
    let more_y = lay.more.map_or(0, |m| m.y / b.scale);
    let at = (b.at.0, b.at.1 + more_y);
    crate::notifycenter::open_actions_menu(state, id, rest, at, true);
}

fn display_next(state: &mut State) {
    if let Some(item) = state.notify.queue.pop_front() {
        display(state, item);
    }
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

fn balloon_layout(state: &State, output: &Output, b: &Balloon, scale: i32) -> NotifyLayout {
    lay_out(
        &state.config.theme.scaled(scale),
        &state.text,
        &b.item,
        b.reply.is_some(),
        state.bar_placement(output).edge,
    )
}

fn lay_out(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
) -> NotifyLayout {
    match item {
        Item::Note(n) => {
            let reply = n.reply.as_ref().filter(|_| replying).map(|r| NotifyReply {
                placeholder: &r.placeholder,
                send: &r.send,
            });
            notify_layout(theme, text, &n.summary, &n.body, &button_labels(n), reply).on_edge(edge)
        }
        Item::Burst(b) => {
            let body = b
                .lines
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            notify_layout(theme, text, &b.title(), &body, &[], None).on_edge(edge)
        }
    }
}

fn display(state: &mut State, item: Item) {
    let Some(output) = state.first_output() else {
        // Nowhere to place it; drop it so the queue does not wedge. It
        // stays open in the history unless it is transient.
        if let Some(n) = item.note().filter(|n| n.transient) {
            crate::tray::notification_closed(state, n.id, REASON_EXPIRED);
        }
        display_next(state);
        return;
    };
    let scale = state.draw_scale_at(Some(&output)).max(1);
    let ms = match &item {
        Item::Note(n) => timeout_ms(n),
        Item::Burst(_) => Some(DEFAULT_MS),
    };
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.overlay);
    crate::glass::leave_out(scene, tree);
    scene.set_data(tree, Some(SceneOwner::Notification));
    let buffer = scene.buffer_create(tree);
    state.notify.active = Some(Balloon {
        item,
        at: (0, 0),
        scale,
        tree,
        buffer,
        held: None,
        hot: None,
        reply: None,
    });
    reposition(state);
    arm(state, ms);
    crate::autohideview::poke(state);
}

fn corner(state: &State, output: &Output, b: &Balloon, scale: i32) -> (i32, i32) {
    let lay = balloon_layout(state, output, b, scale);
    let p = state.bar_placement(output);
    let area = crate::barplace::clear_of(state.usable_area(Some(output)), &p);
    place_notify(area, &lay, scale, p.edge)
}

/// The taskbar moved to another edge, an auto-hidden one changed course, or the balloon's content
/// changed size: the active balloon goes to the corner and redraws with its tail toward the bar.
pub fn reposition(state: &mut State) {
    let Some(output) = state.first_output() else {
        return;
    };
    let Some(b) = state.notify.active.as_ref() else {
        return;
    };
    let (x, y) = corner(state, &output, b, b.scale);
    let tree = b.tree;
    state.scene.set_position(tree, x, y);
    if let Some(b) = state.notify.active.as_mut() {
        b.at = (x, y);
    }
    paint(state);
}

fn paint(state: &mut State) {
    let preedit = state.ime_field_preedit(Editor::NotifyReply);
    let (Some(b), Some(output)) = (state.notify.active.as_ref(), state.first_output()) else {
        return;
    };
    let pushed = match b.held {
        Some((part, true)) => Some(part),
        _ => None,
    };
    let theme = state.config.theme.scaled(b.scale);
    let icon_px = theme.icon_size();
    let icon = crate::tray::closest(b.item.icon(), icon_px)
        .or_else(|| state.icons.get("dialog-information", icon_px));
    let edge = state.bar_placement(&output).edge;
    let lay = lay_out(&theme, &state.text, &b.item, b.reply.is_some(), edge);
    let reply = b.reply.as_ref().map(|r| {
        (
            &r.edit,
            Caret::new(r.focused, state.caret.on),
            preedit.as_ref(),
        )
    });
    let mut canvas = draw_notify_balloon(
        &theme,
        &state.text,
        &lay,
        icon.as_ref(),
        pushed,
        b.hot,
        reply,
    );
    upload_scaled(&mut state.scene, b.buffer, &canvas, b.scale);
    let (origin, scale) = (b.at, b.scale);
    let spot = canvas.take_spot();
    state.ime_field_drawn(Editor::NotifyReply, spot, origin, scale);
    state.redraw_all();
    crate::access::touch(state);
}

/// The caret blinked: redraw the reply field.
pub fn blink(state: &mut State) {
    paint(state);
}

fn destroy(state: &mut State, b: Balloon) {
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
}
