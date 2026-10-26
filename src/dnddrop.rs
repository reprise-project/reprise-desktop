//! The desktop and the taskbar as the target of a drag. Over a desktop cell,
//! a folder icon, the Recycle Bin or Quick Launch a drag has no client focus,
//! so Reprise answers its source itself.
//!
//! The desktop's own drag (`dndsource`) answers with [`Answer`] too. For
//! another program's drag Reprise accepts the mime it reads the drag as
//! ([`reprisefs::dropdata`]) and names the action the held modifiers pick
//! ([`answer`]) through `dnd::DragSource`, which sends
//! `wl_data_source.target` and keeps Smithay's cancel from the source. On
//! release (`DndGrabHandler::dropped`, then an idle: the grab holds the
//! pointer's lock) the source hears the drop and is asked for the data over
//! a pipe, read on a thread (`fsjobs`); the drag ends without reaching any
//! client. Once the data is in (files arrive as a document portal key traded
//! for paths), the source hears the drop is done with the action it took and
//! the drop runs through the desktop's put path (`State::perform_all`).
//!
//! Portal files are only ever copied: the portal lent them read-only. A key
//! that brings no files is followed by a second read of the drag's URI list.

use std::os::fd::OwnedFd;
use std::time::Duration;

use reprisefs::dropdata::{self, Dropped};
use smithay::{
    input::dnd::{DndAction, Source},
    reexports::calloop::timer::{TimeoutAction, Timer},
    reexports::wayland_server::protocol::wl_data_device_manager::DndAction as WlDndAction,
    utils::IsAlive,
};

use crate::{
    compositor::State,
    dnd::DragSource as SeatSource,
    dragview::{self, DragSource, DropOp, DropTarget},
    input::BTN_LEFT,
    keys,
};

const NONE: DndAction = DndAction::None;
const COPY: DndAction = DndAction::Copy;
const MOVE: DndAction = DndAction::Move;
const ASK: DndAction = DndAction::Ask;

/// The most a drop's data may be: a list of paths or links, never a file.
pub(crate) const MAX_BYTES: usize = 1 << 20;
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Put the files with this op; None takes [`natural_op`].
    Put(Option<DropOp>),
    /// Open the drop menu with this op's row as the default; None bolds the natural one.
    Ask(Option<DropOp>),
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    pub action: DndAction,
    pub plan: Plan,
}

impl Answer {
    pub fn cursor(&self) -> &'static [&'static str] {
        match self.plan {
            Plan::Link | Plan::Ask(Some(DropOp::Shortcut)) | Plan::Put(Some(DropOp::Shortcut)) => {
                crate::dnd::cursor_for(true, ASK, true)
            }
            Plan::Ask(_) => crate::dnd::cursor_for(true, ASK, false),
            Plan::Put(_) => crate::dnd::cursor_for(true, self.action, false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Place,
    Trash,
    Quick,
}

impl Spot {
    pub fn of(target: &DropTarget) -> Spot {
        match target {
            DropTarget::Desktop(_) | DropTarget::Folder(_) => Spot::Place,
            DropTarget::Trash => Spot::Trash,
            DropTarget::Quick(_) => Spot::Quick,
        }
    }
}

fn allows(allowed: WlDndAction, action: DndAction) -> bool {
    action != NONE && allowed.contains(action.into())
}

/// Reprise's answer to a drag of web links (`links`) or files over a `spot`,
/// held with the left button (`left`) and modifiers, from a source allowing
/// `allowed`. None refuses the drag.
///
/// On a desktop cell or folder icon Ctrl copies and Shift moves, each only
/// when the source allows it; Alt or Ctrl+Shift, and the right button, ask
/// with the drop menu (a shortcut drag with Create Shortcuts Here as default);
/// with no modifier the natural op decides at the drop. A link becomes a link
/// file, a copy. The Recycle Bin only moves (Ctrl or Alt refuses), Quick
/// Launch only copies (Shift refuses); neither takes a link.
pub fn answer(
    links: bool,
    spot: Spot,
    left: bool,
    (ctrl, shift, alt): (bool, bool, bool),
    allowed: WlDndAction,
) -> Option<Answer> {
    let first = |wanted: &[DndAction]| wanted.iter().copied().find(|a| allows(allowed, *a));
    let forced = if left {
        reprisefs::ops::modifier_op(ctrl, shift, alt)
    } else {
        None
    };
    let (plan, wanted): (Plan, &[DndAction]) = match (spot, links) {
        (Spot::Place, true) => (Plan::Link, &[COPY]),
        (_, true) => return None,
        (Spot::Trash, false) => match forced {
            None | Some(DropOp::Move) => (Plan::Put(Some(DropOp::Move)), &[MOVE]),
            Some(_) => return None,
        },
        (Spot::Quick, false) => match forced {
            Some(DropOp::Move) => return None,
            _ => (Plan::Put(Some(DropOp::Copy)), &[COPY]),
        },
        (Spot::Place, false) if !left => (Plan::Ask(None), &[ASK, MOVE, COPY]),
        (Spot::Place, false) => match forced {
            Some(DropOp::Shortcut) => (Plan::Ask(Some(DropOp::Shortcut)), &[ASK, COPY, MOVE]),
            Some(DropOp::Copy) => (Plan::Put(Some(DropOp::Copy)), &[COPY]),
            Some(DropOp::Move) => (Plan::Put(Some(DropOp::Move)), &[MOVE]),
            None => (Plan::Put(None), &[MOVE, COPY]),
        },
    };
    first(wanted).map(|action| Answer { action, plan })
}

pub fn natural_op(same_device: bool, allowed: WlDndAction) -> DropOp {
    if same_device && allows(allowed, MOVE) || !allows(allowed, COPY) {
        DropOp::Move
    } else {
        DropOp::Copy
    }
}

/// A shortcut leaves the source's file where it is, so it is reported as a copy.
pub fn reported(op: DropOp) -> DndAction {
    match op {
        DropOp::Move => MOVE,
        DropOp::Copy | DropOp::Shortcut => COPY,
    }
}

/// None when the source allows nothing that fits (the drop is cancelled). A
/// move of files already in the target's folder moves nothing, so it is
/// reported as a copy.
pub fn report(op: DropOp, in_place: bool, allowed: WlDndAction) -> Option<DndAction> {
    let action = match op {
        DropOp::Move if in_place => COPY,
        op => reported(op),
    };
    allows(allowed, action).then_some(action)
}

pub fn menu_rows(allowed: WlDndAction) -> dragview::Allowed {
    dragview::Allowed {
        copy: allows(allowed, COPY),
        moves: allows(allowed, MOVE),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settle {
    Put(DropOp),
    Ask(DropOp),
    Refuse,
}

/// `same_device` is whether the first file sits on the target's file system.
/// A link drag's files are only copied, since the source heard a copy.
pub fn settle(plan: Plan, same_device: bool, allowed: WlDndAction) -> Settle {
    let natural = natural_op(same_device, allowed);
    let (op, ask) = match plan {
        Plan::Link => (DropOp::Copy, false),
        Plan::Put(Some(op)) => (op, false),
        Plan::Put(None) => (natural, false),
        Plan::Ask(Some(op)) if allows(allowed, reported(op)) => (op, true),
        Plan::Ask(_) => (natural, true),
    };
    match (allows(allowed, reported(op)), ask) {
        (false, _) => Settle::Refuse,
        (true, false) => Settle::Put(op),
        (true, true) => Settle::Ask(op),
    }
}

/// A source that names none (never called `set_actions`) allows a copy.
pub fn source_actions(actions: &[DndAction]) -> WlDndAction {
    let mask = actions
        .iter()
        .fold(WlDndAction::empty(), |m, a| m | WlDndAction::from(*a));
    if mask.is_empty() {
        WlDndAction::Copy
    } else {
        mask
    }
}

/// Files through the portal are never moved, since the portal lent them read-only.
pub fn read_as(actions: WlDndAction, mime: Option<&str>) -> WlDndAction {
    if mime == Some(dropdata::FILE_TRANSFER) {
        actions - WlDndAction::Move
    } else {
        actions
    }
}

fn heard(mime: Option<&'static str>, answer: Option<Answer>) -> (Option<&'static str>, DndAction) {
    match (mime, answer) {
        (Some(mime), Some(a)) => (Some(mime), a.action),
        _ => (None, NONE),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Over {
    target: DropTarget,
    mime: Option<&'static str>,
    answer: Option<Answer>,
    allowed: WlDndAction,
}

pub struct Receiving {
    id: u64,
    source: SeatSource,
    mime: &'static str,
    /// The drag offers a URI list: read when a portal key brings no files.
    uris: bool,
    target: DropTarget,
    answer: Answer,
    allowed: WlDndAction,
    at: (i32, i32),
    /// The drop menu is asking about this drop: Some(false) while open, Some(true) once closed with its pick (if any) due.
    waiting: Option<bool>,
    links: Vec<(String, Option<String>)>,
    in_place: bool,
}

#[derive(Default)]
pub struct Drops {
    over: Option<Over>,
    /// A drag over a dialog's text field (`dialogdrop`).
    pub field: Option<crate::dialogdrop::FieldOver>,
    receiving: Vec<Receiving>,
    next: u64,
}

/// What cannot go on `target` is dropped: a folder on its own icon, and links anywhere but a desktop cell or folder icon.
fn split(
    items: Vec<Dropped>,
    target: &DropTarget,
) -> (Vec<DragSource>, Vec<(String, Option<String>)>) {
    let mut files = Vec::new();
    let mut links = Vec::new();
    for item in items {
        match item {
            Dropped::File(path) if matches!(target, DropTarget::Folder(d) if same_path(d, &path)) =>
                {}
            Dropped::File(path) => files.push(DragSource::file(path)),
            Dropped::Url { url, title } if Spot::of(target) == Spot::Place => {
                links.push((url, title))
            }
            Dropped::Url { .. } => {}
        }
    }
    (files, links)
}

/// A drop's items sorted against its target: read off the loop, as it stats the files and resolves
/// their folders, which a file on a mount that never answers holds.
#[derive(Debug)]
struct Sorted {
    files: Vec<DragSource>,
    links: Vec<(String, Option<String>)>,
    /// The first file sits on the target folder's file system.
    same: bool,
    /// Every file sits in the target folder already.
    in_place: bool,
    /// The files that sit in the target folder.
    here: std::collections::HashSet<std::path::PathBuf>,
}

/// `items` dropped on `target`, whose folder is `dir`.
fn sort(items: Vec<Dropped>, target: &DropTarget, dir: Option<&std::path::Path>) -> Sorted {
    let (files, links) = split(items, target);
    let same = files
        .first()
        .is_none_or(|f| dir.is_none_or(|d| reprisefs::ops::same_device(f.path(), d)));
    let paths: Vec<std::path::PathBuf> = files.iter().map(|f| f.path().to_path_buf()).collect();
    let here = match dir {
        Some(d) => crate::dragview::in_folder(paths.clone(), d),
        None => Default::default(),
    };
    let in_place = !paths.is_empty() && paths.iter().all(|p| here.contains(p));
    Sorted {
        files,
        links,
        same,
        in_place,
        here,
    }
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// A document portal key is traded for its files, which blocks on the bus: runs off the loop.
fn items(mime: &str, bytes: &[u8]) -> std::io::Result<Vec<Dropped>> {
    if mime != dropdata::FILE_TRANSFER {
        return Ok(dropdata::decode(mime, bytes));
    }
    use reprisebus::filetransfer;
    let key = filetransfer::decode_key(bytes)
        .ok_or_else(|| std::io::Error::other("no file transfer key"))?;
    let ft =
        filetransfer::shared().ok_or_else(|| std::io::Error::other("no file transfer portal"))?;
    let files = ft
        .retrieve(&key)
        .map_err(|e| std::io::Error::other(format!("file transfer {key}: {e}")))?;
    if files.is_empty() {
        return Err(std::io::Error::other(format!(
            "file transfer {key}: no files"
        )));
    }
    Ok(files.into_iter().map(Dropped::File).collect())
}

/// The types of `offered` Reprise reads `source`'s drop as: a sandboxed program's URI list
/// names paths inside its sandbox, so its files come only through the portal key.
pub(crate) fn readable(source: &SeatSource, mut offered: Vec<String>) -> Vec<String> {
    if source.sandboxed() {
        offered.retain(|m| m != dropdata::URI_LIST);
    }
    offered
}

/// The error box's title for a drop given up before its op was settled.
fn give_up_title(plan: Plan) -> &'static str {
    match plan {
        Plan::Put(Some(op)) | Plan::Ask(Some(op)) => op.failure_title(),
        Plan::Link => DropOp::Shortcut.failure_title(),
        Plan::Put(None) | Plan::Ask(None) => DropOp::Copy.failure_title(),
    }
}

/// A source gone meanwhile hears nothing.
fn end(r: Receiving, action: Option<DndAction>) {
    if !r.source.alive() {
        return;
    }
    match action {
        Some(action) => {
            r.source.choose_action(action);
            r.source.finished();
        }
        None => r.source.refuse(),
    }
}

impl State {
    /// Another program's drag is over Reprise's drawing at `pos`: answer the
    /// source for the target there and draw the folder icon under it selected.
    /// Returns the answer's cursor, or None where nothing takes the drop.
    pub fn drop_over(
        &mut self,
        source: &SeatSource,
        pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<&'static [&'static str]> {
        if let Some(cursor) = self.field_over(source, pos) {
            return Some(cursor);
        }
        let Some(target) = self.drop_target_at(&[], pos) else {
            self.drop_left();
            return None;
        };
        let meta = source.metadata().unwrap_or_default();
        let mime = dropdata::pick_mime(&readable(source, meta.mime_types));
        let allowed = read_as(source_actions(&meta.dnd_actions), mime);
        let mods = self.drag_mods();
        let left = self.drag_button() == Some(BTN_LEFT);
        let answer = mime.and_then(|mime| {
            answer(
                dropdata::is_link_mime(mime),
                Spot::of(&target),
                left,
                (
                    mods & keys::MOD_CTRL != 0,
                    mods & keys::MOD_SHIFT != 0,
                    mods & keys::MOD_ALT != 0,
                ),
                allowed,
            )
        });
        let now = heard(mime, answer);
        let before = self.dnd.drops.over.take();
        if before.as_ref().map(|o| heard(o.mime, o.answer)) != Some(now) {
            source.accepted(now.0.map(String::from));
            source.choose_action(now.1);
        }
        self.drop_hilite(answer.is_some().then_some(&target));
        self.dnd.drops.over = Some(Over {
            target,
            mime,
            answer,
            allowed,
        });
        answer.map(|a| a.cursor())
    }

    /// The drag left Reprise's targets: the source hears nothing takes it here, and the folder icon resets.
    pub fn drop_left(&mut self) {
        self.target_left();
        self.field_left();
    }

    pub(crate) fn target_left(&mut self) {
        let Some(was) = self.dnd.drops.over.take() else {
            return;
        };
        if was.answer.is_some()
            && let Some(source) = self.drag_source()
        {
            source.forget_target();
            source.choose_action(NONE);
        }
        self.drop_hilite(None);
    }

    /// Runs under the pointer's lock (no pointer calls). `dropped`: it landed on a
    /// Reprise target, whose data is asked for from an idle.
    pub fn drop_drag_ended(&mut self, source: &SeatSource, dropped: bool) {
        if self.field_drag_ended(source, dropped) {
            return;
        }
        let Some(over) = self.dnd.drops.over.take() else {
            return;
        };
        self.drop_hilite(None);
        if dropped {
            let source = source.clone();
            self.handle
                .insert_idle(move |state| state.drop_released(source, over));
        }
    }

    fn drop_released(&mut self, source: SeatSource, over: Over) {
        let (Some(answer), Some(mime)) = (over.answer, over.mime) else {
            return;
        };
        if !source.alive() {
            return;
        }
        if self.is_locked() {
            source.refuse();
            return;
        }
        let uris = source
            .metadata()
            .is_some_and(|m| m.mime_types.iter().any(|m| m == dropdata::URI_LIST));
        let pos = self.pointer_location();
        let id = self.dnd.drops.next;
        self.dnd.drops.next += 1;
        source.drop_performed();
        self.dnd.drops.receiving.push(Receiving {
            id,
            source: source.clone(),
            mime,
            uris,
            target: over.target,
            answer,
            allowed: over.allowed,
            at: (pos.x as i32, pos.y as i32),
            waiting: None,
            links: Vec::new(),
            in_place: false,
        });
        self.receive_drop(id, &source, mime);
    }

    fn receive_drop(&mut self, id: u64, source: &SeatSource, mime: &'static str) {
        let (read, write) = match std::io::pipe() {
            Ok((r, w)) => (OwnedFd::from(r), OwnedFd::from(w)),
            Err(e) => {
                log::warn!("drop: pipe: {e}");
                self.drop_received(id, Err(e));
                return;
            }
        };
        source.send(mime, write);
        let target = (self.dnd.drops.receiving.iter())
            .find(|r| r.id == id)
            .map(|r| r.target.clone());
        let Some(target) = target else {
            return;
        };
        let dir = match &target {
            DropTarget::Folder(dir) => Some(dir.clone()),
            DropTarget::Desktop(_) => self.desktop.as_ref().map(|v| v.dir.clone()),
            DropTarget::Quick(_) | DropTarget::Trash => None,
        };
        self.spawn_fs_job(move || {
            let got = reprisefs::pipe::read_bounded(&read, MAX_BYTES, READ_TIMEOUT)
                .and_then(|bytes| items(mime, &bytes))
                .map(|items| sort(items, &target, dir.as_deref()));
            Box::new(move |state| state.drop_received(id, got))
        });
        let deadline = Timer::from_duration(self.fsjobs.deadline());
        let armed = self.handle.insert_source(deadline, move |_, _, state| {
            state.drop_gave_up(id);
            TimeoutAction::Drop
        });
        if let Err(e) = armed {
            log::warn!("drop: no deadline for drop {id}: {}", e.error);
        }
    }

    /// A drop whose data is still being read and sorted at its deadline (its target folder sits
    /// on a mount that stopped answering) is refused with an error box; [`State::drop_received`]
    /// drops its answer whenever it comes.
    fn drop_gave_up(&mut self, id: u64) {
        let drops = &mut self.dnd.drops.receiving;
        let Some(i) = drops.iter().position(|r| r.id == id && r.waiting.is_none()) else {
            return;
        };
        let r = drops.remove(i);
        log::warn!(
            "drop: gave up on drop {id} after {:?}",
            self.fsjobs.deadline()
        );
        let title = give_up_title(r.answer.plan);
        end(r, None);
        let text = reprisefs::deadline::not_responding(true, false);
        self.open_dialog(crate::dialog::Dialog::error(title, text));
    }

    /// The source hears the drop is done (or cancelled) with the action taken,
    /// then the drop is carried out. A drop the menu asks about ends once answered.
    fn drop_received(&mut self, id: u64, got: std::io::Result<Sorted>) {
        let drops = &mut self.dnd.drops.receiving;
        let Some(i) = drops.iter().position(|r| r.id == id) else {
            return;
        };
        let mut r = drops.remove(i);
        let sorted = match got {
            Ok(sorted) => sorted,
            Err(e) => {
                log::warn!("drop: reading the dragged data as {}: {e}", r.mime);
                if r.mime == dropdata::FILE_TRANSFER && r.uris && r.source.alive() {
                    r.mime = dropdata::URI_LIST;
                    let source = r.source.clone();
                    self.dnd.drops.receiving.push(r);
                    self.receive_drop(id, &source, dropdata::URI_LIST);
                    return;
                }
                sort(Vec::new(), &DropTarget::Trash, None)
            }
        };
        let target = r.target.clone();
        let Sorted {
            files,
            links,
            same,
            in_place,
            here,
        } = sorted;
        let allowed = r.allowed;
        let plan = if files.is_empty() {
            Plan::Link
        } else {
            r.answer.plan
        };
        let settled = if files.is_empty() && links.is_empty() {
            Settle::Refuse
        } else {
            settle(plan, same, allowed)
        };
        match settled {
            Settle::Refuse => end(r, None),
            Settle::Put(op) => {
                let action = report(op, in_place, allowed);
                end(r, action);
                if action.is_some() {
                    self.put_links(links, &target);
                    if !files.is_empty() {
                        self.put_all(&files, &target, op, &here);
                    }
                }
            }
            // The source hears how the drop ends once the menu is answered or closed without one; the links wait too.
            Settle::Ask(default) => {
                self.cancel_waiting_drop();
                r.links = links;
                r.in_place = in_place;
                let at = r.at;
                self.dnd.drops.receiving.push(r);
                self.drags.pending_drop = Some((files, target));
                self.open_drop_menu(at, default, menu_rows(allowed));
                let drops = &mut self.dnd.drops.receiving;
                match drops.iter().position(|r| r.id == id) {
                    Some(i) if self.menus.is_empty() => {
                        let r = drops.remove(i);
                        self.drags.pending_drop = None;
                        end(r, None);
                    }
                    Some(i) => drops[i].waiting = Some(false),
                    None => {}
                }
            }
        }
    }

    fn take_waiting_drop(&mut self) -> Option<Receiving> {
        let drops = &mut self.dnd.drops.receiving;
        let i = drops.iter().position(|r| r.waiting.is_some())?;
        Some(drops.remove(i))
    }

    /// The drop menu was answered with `op` (None: Cancel). A drop from another
    /// program ends with it if its source allows and is still there, else it is
    /// cancelled; its links are made then. True when the drop goes ahead (the
    /// desktop's own drags always do).
    pub fn drop_menu_answered(&mut self, op: Option<DropOp>) -> bool {
        let Some(mut r) = self.take_waiting_drop() else {
            return op.is_some();
        };
        let action = op
            .filter(|_| r.source.alive())
            .and_then(|op| report(op, r.in_place, r.allowed));
        let (links, target) = (std::mem::take(&mut r.links), r.target.clone());
        end(r, action);
        if action.is_some() {
            self.put_links(links, &target);
        }
        action.is_some()
    }

    /// A pick that closed the menu runs right after; a drop still waiting when the loop idles was dismissed and is cancelled.
    pub fn drop_menu_closed(&mut self) {
        let mut any = false;
        for r in self
            .dnd
            .drops
            .receiving
            .iter_mut()
            .filter(|r| r.waiting.is_some())
        {
            r.waiting = Some(true);
            any = true;
        }
        if any {
            self.handle.insert_idle(|state| {
                let drops = &mut state.dnd.drops.receiving;
                while let Some(i) = drops.iter().position(|r| r.waiting == Some(true)) {
                    let r = drops.remove(i);
                    state.drags.pending_drop = None;
                    end(r, None);
                }
            });
        }
    }

    pub fn cancel_waiting_drop(&mut self) {
        while let Some(r) = self.take_waiting_drop() {
            end(r, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: WlDndAction = WlDndAction::Copy
        .union(WlDndAction::Move)
        .union(WlDndAction::Ask);
    const W_COPY: WlDndAction = WlDndAction::Copy;
    const W_MOVE: WlDndAction = WlDndAction::Move;
    const W_ASK: WlDndAction = WlDndAction::Ask;
    const W_NONE: WlDndAction = WlDndAction::empty();
    const NO_MODS: (bool, bool, bool) = (false, false, false);

    fn plan(a: Option<Answer>) -> Option<(DndAction, Plan)> {
        a.map(|a| (a.action, a.plan))
    }

    #[test]
    fn a_drop_given_up_is_titled_by_the_op_it_planned() {
        assert_eq!(
            give_up_title(Plan::Put(Some(DropOp::Move))),
            "Error Moving File or Folder"
        );
        assert_eq!(give_up_title(Plan::Link), "Error Creating Shortcut");
        assert_eq!(
            give_up_title(Plan::Ask(None)),
            "Error Copying File or Folder"
        );
    }

    #[test]
    fn a_shortcut_answer_shows_the_link_cursor_and_a_menu_the_ask() {
        let at = |action, plan| Answer { action, plan }.cursor()[0];
        assert_eq!(
            at(DndAction::Ask, Plan::Put(Some(DropOp::Shortcut))),
            "dnd-link"
        );
        assert_eq!(at(DndAction::Ask, Plan::Ask(Some(DropOp::Move))), "dnd-ask");
        assert_eq!(at(DndAction::Copy, Plan::Put(None)), "dnd-copy");
        assert_eq!(at(DndAction::Copy, Plan::Link), "dnd-link");
    }

    #[test]
    fn modifiers_pick_what_a_drop_on_the_desktop_does() {
        let left = |mods| plan(answer(false, Spot::Place, true, mods, ALL));
        assert_eq!(left(NO_MODS), Some((MOVE, Plan::Put(None))));
        assert_eq!(
            left((true, false, false)),
            Some((COPY, Plan::Put(Some(DropOp::Copy))))
        );
        assert_eq!(
            left((false, true, false)),
            Some((MOVE, Plan::Put(Some(DropOp::Move))))
        );
        let shortcut = Some((ASK, Plan::Ask(Some(DropOp::Shortcut))));
        assert_eq!(left((false, false, true)), shortcut);
        assert_eq!(left((true, true, false)), shortcut);
    }

    #[test]
    fn the_right_button_asks_with_the_natural_default() {
        for mods in [NO_MODS, (true, false, false), (false, false, true)] {
            assert_eq!(
                plan(answer(false, Spot::Place, false, mods, ALL)),
                Some((ASK, Plan::Ask(None)))
            );
        }
        assert_eq!(
            plan(answer(false, Spot::Place, false, NO_MODS, W_COPY)),
            Some((COPY, Plan::Ask(None)))
        );
    }

    #[test]
    fn a_drop_the_source_does_not_allow_is_refused() {
        assert_eq!(
            answer(false, Spot::Place, true, (false, true, false), W_COPY),
            None
        );
        assert_eq!(
            answer(false, Spot::Place, true, (true, false, false), W_MOVE),
            None
        );
        assert_eq!(answer(false, Spot::Place, true, NO_MODS, W_NONE), None);
        assert_eq!(
            plan(answer(false, Spot::Place, true, NO_MODS, W_COPY)),
            Some((COPY, Plan::Put(None)))
        );
    }

    #[test]
    fn a_web_link_always_makes_a_link_file() {
        for (left, mods) in [
            (true, NO_MODS),
            (true, (false, true, false)),
            (false, NO_MODS),
        ] {
            assert_eq!(
                plan(answer(true, Spot::Place, left, mods, ALL)),
                Some((COPY, Plan::Link))
            );
        }
        assert_eq!(answer(true, Spot::Place, true, NO_MODS, W_ASK), None);
    }

    #[test]
    fn the_recycle_bin_only_moves_and_quick_launch_only_copies() {
        let at = |spot, left, mods, allowed| plan(answer(false, spot, left, mods, allowed));
        let trash = Some((MOVE, Plan::Put(Some(DropOp::Move))));
        assert_eq!(at(Spot::Trash, true, NO_MODS, ALL), trash);
        assert_eq!(at(Spot::Trash, false, NO_MODS, ALL), trash);
        assert_eq!(at(Spot::Trash, true, (false, true, false), ALL), trash);
        assert_eq!(at(Spot::Trash, true, (true, false, false), ALL), None);
        assert_eq!(at(Spot::Trash, true, (false, false, true), ALL), None);
        assert_eq!(at(Spot::Trash, true, NO_MODS, W_COPY), None);
        let quick = Some((COPY, Plan::Put(Some(DropOp::Copy))));
        assert_eq!(at(Spot::Quick, true, NO_MODS, ALL), quick);
        assert_eq!(at(Spot::Quick, true, (false, false, true), ALL), quick);
        assert_eq!(at(Spot::Quick, true, (false, true, false), ALL), None);
        assert_eq!(at(Spot::Quick, true, NO_MODS, W_MOVE), None);
        assert_eq!(answer(true, Spot::Trash, true, NO_MODS, ALL), None);
        assert_eq!(answer(true, Spot::Quick, true, NO_MODS, ALL), None);
    }

    #[test]
    fn a_drop_settles_only_on_an_op_the_source_allows() {
        assert_eq!(settle(Plan::Link, true, ALL), Settle::Put(DropOp::Copy));
        assert_eq!(settle(Plan::Link, true, W_MOVE), Settle::Refuse);
        assert_eq!(
            settle(Plan::Put(None), true, ALL),
            Settle::Put(DropOp::Move)
        );
        assert_eq!(
            settle(Plan::Put(None), false, ALL),
            Settle::Put(DropOp::Copy)
        );
        assert_eq!(
            settle(Plan::Put(None), false, W_MOVE),
            Settle::Put(DropOp::Move)
        );
        assert_eq!(
            settle(Plan::Put(Some(DropOp::Move)), false, ALL),
            Settle::Put(DropOp::Move)
        );
        assert_eq!(
            settle(Plan::Ask(Some(DropOp::Shortcut)), true, ALL),
            Settle::Ask(DropOp::Shortcut)
        );
        assert_eq!(
            settle(Plan::Ask(Some(DropOp::Shortcut)), true, W_MOVE | W_ASK),
            Settle::Ask(DropOp::Move)
        );
        assert_eq!(
            settle(Plan::Ask(None), false, ALL),
            Settle::Ask(DropOp::Copy)
        );
        assert_eq!(settle(Plan::Put(None), true, W_NONE), Settle::Refuse);
        assert_eq!(reported(DropOp::Shortcut), COPY);
        assert_eq!(reported(DropOp::Move), MOVE);
    }

    #[test]
    fn a_folder_on_its_own_icon_and_a_stray_link_are_left_out() {
        let dir = std::path::PathBuf::from("/home/u/Desktop/Photos");
        let items = || {
            vec![
                Dropped::File(dir.clone()),
                Dropped::File("/home/u/a.txt".into()),
                Dropped::Url {
                    url: "https://a.test/".into(),
                    title: None,
                },
            ]
        };
        let (files, links) = split(items(), &DropTarget::Folder(dir.clone()));
        assert_eq!(files, [DragSource::file("/home/u/a.txt".into())]);
        assert_eq!(links.len(), 1);
        let (files, links) = split(items(), &DropTarget::Trash);
        assert_eq!(files.len(), 2);
        assert!(links.is_empty());
    }

    #[test]
    fn a_move_that_moves_nothing_is_reported_as_a_copy() {
        assert_eq!(report(DropOp::Move, false, ALL), Some(MOVE));
        assert_eq!(report(DropOp::Move, true, ALL), Some(COPY));
        assert_eq!(report(DropOp::Move, true, W_MOVE), None);
        assert_eq!(report(DropOp::Shortcut, false, ALL), Some(COPY));
        assert_eq!(report(DropOp::Copy, false, W_MOVE), None);
    }

    #[test]
    fn a_folder_reached_through_a_link_is_still_its_own_icon() {
        let t = std::env::temp_dir().join(format!("reprise-dnddrop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("real/Photos")).unwrap();
        std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
        let (files, _) = split(
            vec![Dropped::File(t.join("link/Photos"))],
            &DropTarget::Folder(t.join("real/./Photos")),
        );
        std::fs::remove_dir_all(&t).unwrap();
        assert!(files.is_empty());
    }

    /// The read job sorts a drop's files against the target folder: a file reached through a
    /// link is in it, a file elsewhere is not, and the drop is in place only when all are.
    #[test]
    fn a_drop_is_sorted_against_its_folder_in_the_read_job() {
        let t = std::env::temp_dir().join(format!("reprise-dndsort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("real")).unwrap();
        std::fs::create_dir_all(t.join("other")).unwrap();
        std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
        let (inside, outside) = (t.join("link/a.txt"), t.join("other/b.txt"));
        std::fs::write(t.join("real/a.txt"), "").unwrap();
        std::fs::write(&outside, "").unwrap();
        let target = DropTarget::Desktop((0, 0));
        let dir = t.join("real");
        let one = sort(vec![Dropped::File(inside.clone())], &target, Some(&dir));
        assert!(one.in_place && one.same);
        assert_eq!(
            one.here.into_iter().collect::<Vec<_>>(),
            std::slice::from_ref(&inside)
        );
        let two = sort(
            vec![Dropped::File(inside.clone()), Dropped::File(outside)],
            &target,
            Some(&dir),
        );
        assert!(!two.in_place);
        assert_eq!(two.here.into_iter().collect::<Vec<_>>(), [inside]);
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn files_through_the_portal_are_only_copied() {
        let portal = read_as(ALL, Some(dropdata::FILE_TRANSFER));
        assert_eq!(portal, W_COPY | W_ASK);
        assert_eq!(read_as(ALL, Some(dropdata::URI_LIST)), ALL);
        assert_eq!(
            plan(answer(false, Spot::Place, true, NO_MODS, portal)),
            Some((COPY, Plan::Put(None)))
        );
        assert_eq!(
            settle(Plan::Put(None), true, portal),
            Settle::Put(DropOp::Copy)
        );
        assert_eq!(
            answer(false, Spot::Place, true, (false, true, false), portal),
            None
        );
        assert_eq!(answer(false, Spot::Trash, true, NO_MODS, portal), None);
        assert!(!menu_rows(portal).moves);
    }

    #[test]
    fn menu_rows_follow_the_source() {
        assert_eq!(menu_rows(ALL), dragview::Allowed::ALL);
        assert_eq!(
            menu_rows(W_COPY),
            dragview::Allowed {
                copy: true,
                moves: false
            }
        );
    }

    #[test]
    fn a_plain_drop_moves_on_one_drive_and_copies_across() {
        assert_eq!(natural_op(true, ALL), DropOp::Move);
        assert_eq!(natural_op(false, ALL), DropOp::Copy);
        assert_eq!(natural_op(true, W_COPY), DropOp::Copy);
        assert_eq!(natural_op(false, W_MOVE), DropOp::Move);
    }

    #[test]
    fn the_cursor_names_what_the_drop_does() {
        let cursor = |links, left, mods| {
            answer(links, Spot::Place, left, mods, ALL)
                .unwrap()
                .cursor()[0]
        };
        assert_eq!(cursor(false, true, NO_MODS), "dnd-move");
        assert_eq!(cursor(false, true, (true, false, false)), "dnd-copy");
        assert_eq!(cursor(false, true, (false, false, true)), "dnd-link");
        assert_eq!(cursor(false, false, NO_MODS), "dnd-ask");
        assert_eq!(cursor(true, true, NO_MODS), "dnd-link");
    }

    #[test]
    fn a_source_naming_no_actions_allows_a_copy() {
        assert_eq!(source_actions(&[]), W_COPY);
        assert_eq!(source_actions(&[NONE]), W_COPY);
        assert_eq!(source_actions(&[MOVE, ASK]), W_MOVE | W_ASK);
        assert_eq!(source_actions(&[ASK, COPY, MOVE]), ALL);
    }

    #[test]
    fn the_source_hears_again_only_when_the_mime_or_action_changes() {
        let a = |action| {
            Some(Answer {
                action,
                plan: Plan::Put(None),
            })
        };
        let uris = Some(dropdata::URI_LIST);
        assert_eq!(heard(uris, a(MOVE)), (uris, MOVE));
        assert_eq!(
            heard(
                uris,
                Some(Answer {
                    action: MOVE,
                    plan: Plan::Put(Some(DropOp::Move))
                })
            ),
            heard(uris, a(MOVE))
        );
        assert_eq!(heard(uris, None), (None, NONE));
        assert_eq!(heard(None, a(COPY)), (None, NONE));
    }

    #[test]
    fn a_sandboxed_programs_uri_list_is_refused_on_the_desktop_and_its_portal_key_taken() {
        use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
        use wayland_client::{Proxy, protocol::wl_data_device_manager::DndAction as WcAction};
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let dir = crate::harness::scratch().join("sandboxed-drop");
        std::fs::create_dir_all(&dir).unwrap();
        crate::desktopview::create(&mut h.state, dir, Default::default(), None);
        let mut c = h.connect_sandboxed();
        let mut drag_over = |mimes: &[&str]| {
            let theirs = c.data_source(mimes, b"file:///home/u/.ssh/id_ed25519\r\n");
            theirs.set_actions(WcAction::Copy | WcAction::Move);
            h.roundtrip(&mut c);
            let ours: WlDataSource = (c.server)
                .object_from_protocol_id(&h.state.dh, theirs.id().protocol_id())
                .expect("the server's source");
            let source = SeatSource::new(crate::dnd::Origin::Client(ours));
            h.state.dnd.drops.over = None;
            c.side.source_targets.clear();
            h.state.drop_over(&source, (600.0, 400.0).into());
            h.roundtrip(&mut c);
            let over = h.state.dnd.drops.over.as_ref().expect("over the desktop");
            assert!(matches!(over.target, DropTarget::Desktop(_)), "{mimes:?}");
            (over.answer.is_some(), c.side.source_targets.clone())
        };
        assert_eq!(drag_over(&[dropdata::URI_LIST]), (false, vec![None]));
        assert_eq!(
            drag_over(&[dropdata::FILE_TRANSFER, dropdata::URI_LIST]),
            (true, vec![Some(dropdata::FILE_TRANSFER.to_string())])
        );
        assert_eq!(
            drag_over(&[dropdata::MOZ_URL, dropdata::URI_LIST]),
            (true, vec![Some(dropdata::MOZ_URL.to_string())]),
            "a link"
        );
    }

    #[test]
    fn a_sandboxed_programs_link_drops_its_text_in_a_dialog_field() {
        use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
        use wayland_client::{Proxy, protocol::wl_data_device_manager::DndAction as WcAction};
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::fsjobs::setup(&mut h.state);
        h.pointer_to(500.0, 400.0);
        h.state.open_dialog(crate::dialog::Dialog::run(&[]));
        let mut c = h.connect_sandboxed();
        let theirs = c.data_source(&[dropdata::URI_LIST, "text/plain"], b"https://a.test/");
        theirs.set_actions(WcAction::Copy);
        h.roundtrip(&mut c);
        let ours: WlDataSource = (c.server)
            .object_from_protocol_id(&h.state.dh, theirs.id().protocol_id())
            .expect("the server's source");
        let source = SeatSource::new(crate::dnd::Origin::Client(ours));
        let v = &h.state.dialogs.views[0];
        let r = v.layout.field.expect("the Run field");
        let at = (
            (v.x + r.x + r.width / 2) as f64,
            (v.y + r.y + r.height / 2) as f64,
        );
        assert!(
            h.state.drop_over(&source, at.into()).is_some(),
            "the field takes it"
        );
        h.roundtrip(&mut c);
        assert_eq!(c.side.source_targets, [Some("text/plain".to_string())]);
        assert!(h.state.field_drag_ended(&source, true));
        let text = |h: &crate::harness::Harness| {
            let d = &h.state.dialogs.views[0].dialog;
            d.field
                .as_ref()
                .expect("the Run field")
                .combo
                .text()
                .to_string()
        };
        let end = std::time::Instant::now() + Duration::from_secs(3);
        while text(&h).is_empty() {
            assert!(std::time::Instant::now() < end, "the text never landed");
            h.roundtrip(&mut c);
            h.run_for(5);
        }
        assert_eq!(text(&h), "https://a.test/");
    }

    /// A drag source that takes the drop's pipe and never writes, as a drop onto a folder on a
    /// mount that stopped answering never sorts. Dropping the pipes ends the read.
    #[derive(Default)]
    struct Stall {
        pipes: std::sync::Mutex<Vec<OwnedFd>>,
        cancelled: std::sync::atomic::AtomicBool,
    }

    impl IsAlive for Stall {
        fn alive(&self) -> bool {
            true
        }
    }

    impl Source for Stall {
        fn metadata(&self) -> Option<smithay::input::dnd::SourceMetadata> {
            Some(smithay::input::dnd::SourceMetadata {
                mime_types: vec![dropdata::URI_LIST.into()],
                dnd_actions: [DndAction::Move].into_iter().collect(),
            })
        }
        fn choose_action(&self, _: DndAction) {}
        fn send(&self, _: &str, fd: OwnedFd) {
            self.pipes.lock().unwrap().push(fd);
        }
        fn drop_performed(&self) {}
        fn cancel(&self) {
            (self.cancelled).store(true, std::sync::atomic::Ordering::Release);
        }
        fn finished(&self) {}
    }

    #[test]
    fn a_drop_whose_data_has_not_landed_by_its_deadline_is_refused_with_an_error_box() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.fsjobs.deadline = Some(Duration::from_millis(100));
        crate::fsjobs::setup(&mut h.state);
        let stall = std::sync::Arc::new(Stall::default());
        let source = SeatSource::new(crate::dnd::Origin::Own(stall.clone()));
        let over = Over {
            target: DropTarget::Folder("/srv/share".into()),
            mime: Some(dropdata::URI_LIST),
            answer: Some(Answer {
                action: DndAction::Move,
                plan: Plan::Put(Some(DropOp::Move)),
            }),
            allowed: W_MOVE,
        };
        h.state.drop_released(source, over);
        let titles = |h: &crate::harness::Harness| -> Vec<String> {
            (h.state.dialogs.views.iter())
                .map(|v| v.dialog.title.clone())
                .collect()
        };
        let end = std::time::Instant::now() + Duration::from_secs(3);
        while titles(&h).is_empty() {
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for the give-up"
            );
            h.run_for(5);
        }
        assert_eq!(titles(&h), ["Error Moving File or Folder"]);
        assert!(
            stall.cancelled.load(std::sync::atomic::Ordering::Acquire),
            "the source hears the drop is refused"
        );
        assert!(h.state.dnd.drops.receiving.is_empty());
        stall.pipes.lock().unwrap().clear();
        h.run_for(100);
        assert_eq!(
            titles(&h),
            ["Error Moving File or Folder"],
            "the read ending later changes nothing"
        );
    }
}
