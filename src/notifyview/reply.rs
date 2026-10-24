//! The inline reply row: its field, keys and sending.

use reprisekit::controls::tooltip::{ReplyEvent, ReplyField, ReplyKey};
use reprisekit::textedit::{EditKey, TextEdit};

use crate::compositor::State;
use crate::dialogview::DialogKey;
use crate::tray::TrayCommand;
use crate::tray::notify::INLINE_REPLY;

use super::clock::{rearm, reply_idle_ms, restart};
use super::motion::leave;
use super::paint::paint;
use super::stack::{Item, insert, new_balloon, overflow};
use super::{
    NotifyView, REASON_DISMISSED, cap, changed, close, drop_balloon, replying, sheets, timeout_ms,
};

/// A key or the pointer in the replying balloon: its reply row gets another `REPLY_IDLE`.
pub(super) fn reply_touched(state: &mut State) {
    let sheets = sheets(state);
    if let Some(i) = replying(&state.notify) {
        restart(&mut state.notify, i, reply_idle_ms(), sheets);
        rearm(state);
    }
}

/// Open `id`'s reply row on its balloon, another open row going to `drafts`; from the history it
/// gets a balloon of its own, a full stack's newest going back to the queue.
pub(super) fn start_reply(state: &mut State, id: u32) {
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
pub(super) fn take_draft(drafts: &mut Vec<(u32, ReplyField)>, id: u32) -> ReplyField {
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
pub(super) fn collapse_idle(state: &mut State) {
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
pub(super) fn send_reply(state: &mut State, text: String) {
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

/// The caret blinked: redraw the reply field.
pub fn blink(state: &mut State) {
    paint(state);
}
