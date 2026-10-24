//! The queue, the stack of shown balloons and bursts folding one program's notifications.

use reprisekit::controls::notifysheet::{SheetGlass, SheetOpening, has_sheets};
use reprisekit::icon::IconSource;
use reprisekit::style::SheetShape;
use std::collections::VecDeque;
use std::time::Instant;

use crate::compositor::State;
use crate::theme::Theme;
use crate::tray::Notification;
use crate::tray::notify::Urgency;

use super::clock::{held_clock, restart};
use super::layout::sheet_tint;
use super::motion::{Motion, destroy, drop_leaving, leave, restack};
use super::paint::paint;
use super::{
    BURST_AFTER, BURST_LINES, BURST_WINDOW, Balloon, DEFAULT_MS, MAX_QUEUED, NotifyView, OWN_ID,
    REASON_EXPIRED, cap, close_transient, droplets, overlay_tree, sheets, take_down, timeout_ms,
};

#[derive(Clone, Debug)]
pub(super) enum Item {
    Note(Notification),
    Burst(Burst),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Burst {
    pub(super) app: String,
    icon: Vec<IconSource>,
    app_icon: Vec<IconSource>,
    /// Oldest first.
    pub(super) ids: Vec<u32>,
    /// The newest summaries, newest first.
    pub(super) lines: VecDeque<String>,
}

impl Burst {
    pub(super) fn new(app: &str) -> Burst {
        Burst {
            app: app.to_string(),
            ..Burst::default()
        }
    }

    pub(super) fn add(&mut self, n: &Notification) {
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

    pub(super) fn title(&self) -> String {
        format!("{} new from {}", self.ids.len(), self.app)
    }
}

impl Item {
    pub(super) fn note(&self) -> Option<&Notification> {
        match self {
            Item::Note(n) => Some(n),
            Item::Burst(_) => None,
        }
    }

    pub(super) fn is(&self, id: u32) -> bool {
        self.note().is_some_and(|n| n.id == id)
    }

    pub(super) fn burst_has(&self, id: u32) -> bool {
        matches!(self, Item::Burst(b) if b.ids.contains(&id))
    }

    pub(super) fn burst_of(&self, app: &str) -> bool {
        matches!(self, Item::Burst(b) if b.app == app)
    }

    pub(super) fn icon(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => n.balloon_icon(),
            Item::Burst(b) => &b.icon,
        }
    }

    pub(super) fn app_icon(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => &n.app_icon,
            Item::Burst(b) => &b.app_icon,
        }
    }

    pub(super) fn has_app_icon(&self) -> bool {
        !self.app_icon().is_empty()
    }

    pub(super) fn image(&self) -> &[IconSource] {
        match self {
            Item::Note(n) => &n.image,
            Item::Burst(_) => &[],
        }
    }
}

/// The queue's head while `len` shown and those taken stay under `cap`.
pub(super) fn fill(len: usize, cap: usize, queue: &mut VecDeque<Item>) -> Vec<Item> {
    let mut came = Vec::new();
    while len + came.len() < cap
        && let Some(item) = queue.pop_front()
    {
        came.push(item);
    }
    came
}

pub(super) fn slot_of(items: &[&Item], id: u32) -> Option<usize> {
    items.iter().position(|i| i.is(id))
}

/// The stack notes from `app` a new burst takes: all but one with an open reply (the bool).
pub(super) fn fold_from(items: &[(&Item, bool)], app: &str) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, (item, replying))| !replying && item.note().is_some_and(|n| n.app == app))
        .map(|(i, _)| i)
        .collect()
}

pub(super) fn chip_count(queued: usize, cap: usize) -> Option<usize> {
    (queued > 0 && cap > 1).then_some(queued)
}

pub(super) fn take_all<T>(stack: &mut Vec<T>, leaving: &mut Vec<T>) -> Vec<T> {
    stack.drain(..).chain(leaving.drain(..)).collect()
}

pub(super) fn foldable(n: &Notification) -> bool {
    !n.app.is_empty() && n.id != OWN_ID && n.urgency != Urgency::Critical
}

pub(super) fn in_burst(v: &NotifyView, id: u32) -> bool {
    v.stack.iter().any(|b| b.item.burst_has(id)) || v.queue.iter().any(|q| q.burst_has(id))
}

pub(super) fn arrived(recent: &mut VecDeque<(String, Instant)>, app: &str, at: Instant) -> bool {
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
pub(super) fn fold(state: &mut State, n: Notification) {
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
pub(super) fn fold_in<T>(stack: &mut Vec<T>, taken: &[usize], burst: Option<T>) -> Vec<T> {
    let gone: Vec<T> = taken.iter().rev().map(|&i| stack.remove(i)).collect();
    if let (Some(b), Some(&oldest)) = (burst, taken.last()) {
        stack.insert(oldest + 1 - taken.len(), b);
    }
    gone
}

pub(super) fn take_queued(
    queue: &mut VecDeque<Item>,
    app: &str,
) -> (Vec<Notification>, Option<usize>) {
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

/// Shows the queue's head in every free slot, nearest the tray as the queue is newer than the
/// stack, then repaints the stack and chip.
pub(super) fn display_next(state: &mut State) {
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
pub(super) fn enqueue(queue: &mut VecDeque<Item>, item: Item) -> Vec<u32> {
    queue.push_back(item);
    let mut dropped = Vec::new();
    while queue.len() > MAX_QUEUED {
        if let Some(Item::Note(old)) = queue.pop_front() {
            dropped.push(old.id);
        }
    }
    dropped
}

/// A balloon for `item`, its clock started (and held if the pointer is on the sheets); `None`
/// with no output to place it on, where a transient notification closes.
pub(super) fn new_balloon(state: &mut State, item: Item) -> Option<Balloon> {
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
    let clock = held_clock(&state.notify, ms, sheets(state), now);
    let e = &state.config.effects;
    let motion = match (sheets(state), state.config.glass.notification.shape()) {
        (false, _) => Motion::settled_at(now),
        (true, SheetShape::Droplet) => Motion::drip(e, now),
        (true, SheetShape::Lozenge | SheetShape::Halo) => Motion::arrive(e, now),
    };
    let tint = sheet_tint(&state.config.theme, item.app_icon(), scale);
    let tree = overlay_tree(state);
    let glass_tree = state.scene.tree_create(tree);
    let buffer = state.scene.buffer_create(tree);
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

pub(super) fn insert(state: &mut State, at: usize, b: Balloon) {
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
pub(super) fn overflow<T>(stack: &mut Vec<T>, cap: usize, keep: impl Fn(&T) -> bool) -> Vec<T> {
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
    let (sheets, droplets) = (sheets(state), droplets(state));
    shed(&mut state.notify.stack, sheets, droplets);
    for b in &mut state.notify.stack {
        b.painted = None;
    }
    display_next(state);
}

/// Frees what the look no longer draws: a Droplet's opening layers once sheets are not Droplets,
/// and the kept glass once there are no sheets.
pub(super) fn shed(stack: &mut [Balloon], sheets: bool, droplets: bool) {
    for b in stack {
        if !droplets {
            b.opening = SheetOpening::default();
        }
        if !sheets {
            b.glass = SheetGlass::default();
        }
    }
}

/// An output went away: sheets still leaving go at once (`update_layout` has moved the stack).
pub fn output_removed(state: &mut State) {
    drop_leaving(state);
}

/// The look may have changed: a balloon shown under Classic or Blue takes its Glass tint. An icon
/// is decoded only for a balloon not yet tinted, as autohide repositions often.
pub(super) fn retint(stack: &mut [Balloon], theme: &Theme) {
    let sheets = has_sheets(theme);
    for b in stack {
        if !sheets || b.tint.is_none() {
            b.tint = sheet_tint(theme, b.item.app_icon(), b.scale);
        }
    }
}

/// Burst `b` takes `n`; its first app icon tints it.
pub(super) fn join_burst(b: &mut Balloon, n: &Notification, theme: &Theme) {
    if let Item::Burst(burst) = &mut b.item {
        if burst.app_icon.is_empty() {
            b.tint = sheet_tint(theme, &n.app_icon, b.scale);
        }
        burst.add(n);
    }
}
