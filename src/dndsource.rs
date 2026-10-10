//! The desktop and the Start menu as the source of a drag into any program.
//!
//! Over the desktop's own targets the drag has no client focus and Reprise answers for itself
//! ([`answer`]); a release there ends the drag without any client seeing a drop, and the desktop's
//! own drop runs from an idle once the grab has let go of the pointer. The source never deletes
//! anything: a program that takes a drop as a move moves the files itself.

use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;

use reprisebus::filetransfer::Transfers;
use reprisefs::dropdata::{self, FILE_TRANSFER, HOST_FILES};
use smithay::{
    input::dnd::{DndAction, Source, SourceMetadata},
    utils::{IsAlive, Logical, Point, SERIAL_COUNTER},
};

use crate::{
    compositor::State,
    dnd::{DragSource as SeatSource, Origin},
    dnddrop::{Answer, Plan},
    dragview::{Allowed, DragSource, DropOp, DropTarget},
    input::{BTN_RIGHT, Interaction},
    keys,
};

const COPY: DndAction = DndAction::Copy;
const MOVE: DndAction = DndAction::Move;
const ASK: DndAction = DndAction::Ask;

type Offer = (String, Arc<[u8]>);

/// Reprise's source for the desktop's drag on the seat.
pub struct OwnSource {
    offers: Vec<Offer>,
    mimes: Vec<String>,
    actions: Vec<DndAction>,
    paths: Vec<PathBuf>,
    transfers: Transfers,
}

impl IsAlive for OwnSource {
    fn alive(&self) -> bool {
        true
    }
}

impl Source for OwnSource {
    fn metadata(&self) -> Option<SourceMetadata> {
        Some(SourceMetadata {
            mime_types: self.mimes.clone(),
            dnd_actions: self.actions.iter().copied().collect(),
        })
    }

    fn choose_action(&self, _action: DndAction) {}

    /// Each request is a transfer of its own: a target may read the key while the drag passes over
    /// it and again at the drop. A type it does not offer gets its pipe closed empty.
    fn send(&self, mime_type: &str, fd: OwnedFd) {
        if mime_type == FILE_TRANSFER {
            self.transfers.send_key(self.paths.clone(), move |key| {
                reprisefs::pipe::write_detached(fd, key.into())
            });
            return;
        }
        if let Some((_, bytes)) = self.offers.iter().find(|(m, _)| m == mime_type) {
            reprisefs::pipe::write_detached(fd, bytes.clone());
        }
    }

    fn drop_performed(&self) {}

    fn cancel(&self) {}

    fn finished(&self) {}
}

/// Whether a drag of `sources` may leave the desktop. A system icon's folder is never handed to
/// another program.
pub fn goes_out(sources: &[DragSource]) -> bool {
    !sources.is_empty() && !sources.iter().any(DragSource::is_system)
}

/// The actions a program may take the drop of `sources` as. A Start menu program's file belongs to
/// the system: it is only ever copied.
pub fn actions(sources: &[DragSource]) -> Vec<DndAction> {
    if sources.iter().any(|s| matches!(s, DragSource::App { .. })) {
        vec![COPY]
    } else {
        vec![COPY, MOVE, ASK]
    }
}

/// What the drag offers for `paths`, best first, each with its bytes.
pub fn offers(paths: &[PathBuf]) -> Vec<Offer> {
    dropdata::FILE_MIMES
        .iter()
        .filter_map(|mime| {
            let bytes = dropdata::encode_files(paths, mime)?;
            Some((mime.to_string(), Arc::from(bytes)))
        })
        .collect()
}

/// The mimes a drag with `offers` names, best first: the portal key ahead of them when `portal`
/// (the document portal is on the bus), and then the mark that the URI list names the files
/// themselves, last.
pub fn mimes(offers: &[Offer], portal: bool) -> Vec<String> {
    let key = portal.then(|| FILE_TRANSFER.to_string());
    let host = portal.then(|| HOST_FILES.to_string());
    key.into_iter()
        .chain(offers.iter().map(|(m, _)| m.clone()))
        .chain(host)
        .collect()
}

/// Reprise's answer to its own drag of `grabbed` (and what rides with it) over `target`, held with
/// `button` and the given modifiers; None refuses the drop there.
pub fn answer(
    grabbed: &DragSource,
    target: &DropTarget,
    button: u32,
    (ctrl, shift, alt): (bool, bool, bool),
) -> Option<Answer> {
    if button == BTN_RIGHT {
        return Some(Answer {
            action: ASK,
            plan: Plan::Ask(Some(DropOp::Move)),
        });
    }
    let natural = crate::dragview::left_drop_op(grabbed, target);
    let op = match (target, reprisefs::ops::modifier_op(ctrl, shift, alt)) {
        (DropTarget::Trash, None | Some(DropOp::Move)) => DropOp::Move,
        (DropTarget::Trash, Some(_)) => return None,
        (DropTarget::Quick(_), _) | (_, None) => natural,
        (_, Some(op)) => op,
    };
    let op = match (grabbed, op) {
        (DragSource::App { .. }, DropOp::Move) => DropOp::Copy,
        (_, op) => op,
    };
    Some(Answer {
        action: match op {
            DropOp::Move => MOVE,
            DropOp::Copy => COPY,
            DropOp::Shortcut => ASK,
        },
        plan: Plan::Put(Some(op)),
    })
}

impl State {
    /// The desktop's armed drag passed the threshold: make it the seat's, so it can drop into
    /// another program. False leaves it the desktop's own (`State::drag_finish` ends it).
    pub fn hand_over(&mut self) -> bool {
        let Some(d) = self.drags.armed.as_ref() else {
            return false;
        };
        if !goes_out(&d.sources)
            || self.dnd.running()
            || self.is_locked()
            || self.inputproto.gated()
        {
            return false;
        }
        let Some(start) = self
            .seat
            .get_pointer()
            .and_then(|p| p.grab_start_data())
            // A press that went to no client: its click grab has no focus, and the drag takes
            // its start over.
            .filter(|s| s.focus.is_none() && s.button == d.button)
        else {
            return false;
        };
        let paths: Vec<PathBuf> = d.sources.iter().map(|s| s.path().to_path_buf()).collect();
        let offers = offers(&paths);
        let transfers = Transfers::for_drag();
        let source = OwnSource {
            mimes: mimes(&offers, transfers.offered()),
            actions: actions(&d.sources),
            offers,
            paths,
            transfers,
        };
        if let Some(d) = self.drags.armed.as_mut() {
            d.seat = true;
        }
        self.settle_click(true);
        self.interaction = Interaction::Idle;
        let source = SeatSource::new(Origin::Own(Arc::new(source)));
        self.start_drag(source, None, start, SERIAL_COUNTER.next_serial());
        if !self.dnd.running() {
            if let Some(d) = self.drags.armed.as_mut() {
                d.seat = false;
            }
            return false;
        }
        true
    }

    fn own_drag(&self) -> bool {
        self.dnd.running() && self.drags.armed.as_ref().is_some_and(|d| d.seat)
    }

    /// The desktop's own drag is over Reprise's drawing at `pos`: find the target there (never one
    /// of the dragged icons), answer for it and draw the folder icon under it selected.
    pub fn own_over(&mut self, pos: Point<f64, Logical>) -> Option<&'static [&'static str]> {
        let (sources, button) = self
            .drags
            .armed
            .as_ref()
            .filter(|d| d.seat)
            .map(|d| (d.sources.clone(), d.button))?;
        let target = self.drop_target_at(&sources, pos);
        let mods = self.drag_mods();
        let answer = target.as_ref().and_then(|t| {
            answer(
                sources.first()?,
                t,
                button,
                (
                    mods & keys::MOD_CTRL != 0,
                    mods & keys::MOD_SHIFT != 0,
                    mods & keys::MOD_ALT != 0,
                ),
            )
        });
        self.drop_hilite(target.as_ref().filter(|_| answer.is_some()));
        self.drags.over = target.zip(answer);
        answer.map(|a| a.cursor())
    }

    /// The drag left Reprise's drawing for a client's surface: the target it was over shows plain
    /// again.
    pub fn own_left(&mut self) {
        if self.own_drag() {
            self.drags.over = None;
            self.drop_hilite(None);
        }
    }

    /// The seat's drag ended (under the pointer's lock: no pointer calls).
    pub fn own_drag_ended(&mut self, dropped: bool) {
        let Some(mut d) = self.drags.armed.take_if(|d| d.seat) else {
            return;
        };
        let over = self.drags.over.take();
        self.drop_hilite(None);
        self.clear_drag_image(&mut d);
        if let (true, Some((target, answer))) = (dropped, over) {
            self.handle
                .insert_idle(move |state| state.own_drop(d.sources, target, answer));
        }
    }

    fn own_drop(&mut self, sources: Vec<DragSource>, target: DropTarget, answer: Answer) {
        if self.is_locked() {
            return;
        }
        let pos = self.pointer_location();
        let at = (pos.x as i32, pos.y as i32);
        match answer.plan {
            Plan::Ask(default) => {
                self.cancel_waiting_drop();
                self.drags.pending_drop = Some((sources, target));
                self.open_drop_menu(at, default.unwrap_or(DropOp::Move), Allowed::ALL);
            }
            Plan::Put(op) => self.perform_all(&sources, &target, op.unwrap_or(DropOp::Move)),
            Plan::Link => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop;
    use crate::input::{BTN_LEFT, BTN_MIDDLE};

    const NO_MODS: (bool, bool, bool) = (false, false, false);
    const CTRL: (bool, bool, bool) = (true, false, false);
    const SHIFT: (bool, bool, bool) = (false, true, false);
    const ALT: (bool, bool, bool) = (false, false, true);

    fn icon(path: &str) -> DragSource {
        DragSource::Icon {
            id: desktop::IconId::File(PathBuf::from(path)),
            path: PathBuf::from(path),
            name: "notes.txt".into(),
        }
    }

    fn app() -> DragSource {
        DragSource::App {
            entry: reprisefs::apps::parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim\n",
                "vim.desktop",
            )
            .unwrap(),
            file: PathBuf::from("/usr/share/applications/vim.desktop"),
        }
    }

    fn put(a: Option<Answer>) -> Option<(DndAction, Plan)> {
        a.map(|a| (a.action, a.plan))
    }

    #[test]
    fn modifiers_pick_what_a_desktop_drop_does() {
        let cell = DropTarget::Desktop((2, 3));
        let at = |mods| put(answer(&icon("/d/a"), &cell, BTN_LEFT, mods));
        assert_eq!(at(NO_MODS), Some((MOVE, Plan::Put(Some(DropOp::Move)))));
        assert_eq!(at(CTRL), Some((COPY, Plan::Put(Some(DropOp::Copy)))));
        assert_eq!(at(SHIFT), Some((MOVE, Plan::Put(Some(DropOp::Move)))));
        let shortcut = Some((ASK, Plan::Put(Some(DropOp::Shortcut))));
        assert_eq!(at(ALT), shortcut);
        assert_eq!(at((true, true, false)), shortcut);
        let folder = DropTarget::Folder(PathBuf::from("/d/Reports"));
        assert_eq!(
            put(answer(&icon("/d/a"), &folder, BTN_LEFT, CTRL)),
            Some((COPY, Plan::Put(Some(DropOp::Copy))))
        );
    }

    #[test]
    fn the_middle_button_drops_as_the_left_one_does() {
        let cell = DropTarget::Desktop((2, 3));
        for mods in [NO_MODS, CTRL, ALT] {
            assert_eq!(
                put(answer(&icon("/d/a"), &cell, BTN_MIDDLE, mods)),
                put(answer(&icon("/d/a"), &cell, BTN_LEFT, mods))
            );
        }
    }

    #[test]
    fn the_right_button_asks_with_move_in_bold() {
        for target in [DropTarget::Desktop((0, 0)), DropTarget::Trash] {
            assert_eq!(
                put(answer(&icon("/d/a"), &target, BTN_RIGHT, CTRL)),
                Some((ASK, Plan::Ask(Some(DropOp::Move))))
            );
        }
    }

    #[test]
    fn the_recycle_bin_only_moves() {
        let at = |mods| put(answer(&icon("/d/a"), &DropTarget::Trash, BTN_LEFT, mods));
        assert_eq!(at(NO_MODS), Some((MOVE, Plan::Put(Some(DropOp::Move)))));
        assert_eq!(at(SHIFT), Some((MOVE, Plan::Put(Some(DropOp::Move)))));
        assert_eq!(at(CTRL), None);
        assert_eq!(at(ALT), None);
    }

    #[test]
    fn a_program_is_copied_and_quick_launch_takes_anything() {
        let cell = DropTarget::Desktop((0, 0));
        assert_eq!(
            put(answer(&app(), &cell, BTN_LEFT, NO_MODS)),
            Some((COPY, Plan::Put(Some(DropOp::Copy))))
        );
        assert_eq!(
            put(answer(&app(), &cell, BTN_LEFT, SHIFT)),
            Some((COPY, Plan::Put(Some(DropOp::Copy))))
        );
        assert_eq!(
            put(answer(&app(), &cell, BTN_LEFT, ALT)),
            Some((ASK, Plan::Put(Some(DropOp::Shortcut))))
        );
        let quick = DropTarget::Quick(1);
        for mods in [NO_MODS, CTRL, SHIFT, ALT] {
            assert_eq!(
                put(answer(&icon("/d/a"), &quick, BTN_LEFT, mods)),
                Some((ASK, Plan::Put(Some(DropOp::Shortcut))))
            );
        }
    }

    #[test]
    fn the_cursor_names_the_desktop_drop() {
        let cell = DropTarget::Desktop((0, 0));
        let cursor = |mods| {
            answer(&icon("/d/a"), &cell, BTN_LEFT, mods)
                .unwrap()
                .cursor()[0]
        };
        assert_eq!(cursor(NO_MODS), "dnd-move");
        assert_eq!(cursor(CTRL), "dnd-copy");
        assert_eq!(cursor(ALT), "dnd-link");
    }

    #[test]
    fn a_program_drags_out_as_a_copy_and_icons_as_anything() {
        assert_eq!(actions(&[app()]), [COPY]);
        assert_eq!(actions(&[icon("/d/a"), icon("/d/b")]), [COPY, MOVE, ASK]);
    }

    #[test]
    fn a_system_icon_never_leaves_the_desktop() {
        let bin = DragSource::Icon {
            id: desktop::IconId::System(desktop::Kind::RecycleBin),
            path: PathBuf::from("/home/u/.local/share/Trash/files"),
            name: "Recycle Bin".into(),
        };
        assert!(goes_out(&[icon("/d/a")]));
        assert!(goes_out(&[app()]));
        assert!(!goes_out(&[icon("/d/a"), bin]));
        assert!(!goes_out(&[]));
    }

    #[test]
    fn the_drag_offers_uris_then_gnome_files_then_paths() {
        let offers = offers(&[PathBuf::from("/d/my notes.txt")]);
        let mimes: Vec<&str> = offers.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            mimes,
            [
                "text/uri-list",
                "x-special/gnome-copied-files",
                "text/plain;charset=utf-8"
            ]
        );
        assert_eq!(&*offers[0].1, b"file:///d/my%20notes.txt\r\n");
        assert_eq!(&*offers[2].1, b"/d/my notes.txt");
        let unsayable = super::offers(&[PathBuf::from("/d/two\nlines")]);
        assert_eq!(
            unsayable.len(),
            2,
            "no plain text for a path with a line break"
        );
    }

    #[test]
    fn the_portal_key_comes_first_only_when_the_portal_is_there() {
        let offers = offers(&[PathBuf::from("/d/a")]);
        assert_eq!(
            mimes(&offers, true),
            [
                FILE_TRANSFER,
                "text/uri-list",
                "x-special/gnome-copied-files",
                "text/plain;charset=utf-8",
                HOST_FILES
            ]
        );
        assert_eq!(mimes(&offers, false), &mimes(&offers, true)[1..4]);
    }

    #[test]
    fn the_source_sends_what_it_offers_and_names_its_actions() {
        let paths = vec![PathBuf::from("/d/a.txt")];
        let offers = offers(&paths);
        let source = OwnSource {
            mimes: mimes(&offers, false),
            actions: actions(&[icon("/d/a.txt")]),
            offers,
            paths,
            transfers: Transfers::on(None),
        };
        let meta = source.metadata().unwrap();
        assert_eq!(meta.mime_types[0], "text/uri-list");
        assert_eq!(&meta.dnd_actions[..], [COPY, MOVE, ASK]);
        let (rx, tx) = std::io::pipe().unwrap();
        source.send("text/uri-list", OwnedFd::from(tx));
        let mut got = String::new();
        std::io::Read::read_to_string(&mut std::fs::File::from(OwnedFd::from(rx)), &mut got)
            .unwrap();
        assert_eq!(got, "file:///d/a.txt\r\n");
        let (rx, tx) = std::io::pipe().unwrap();
        source.send("image/png", OwnedFd::from(tx));
        let mut got = Vec::new();
        std::io::Read::read_to_end(&mut std::fs::File::from(OwnedFd::from(rx)), &mut got).unwrap();
        assert!(got.is_empty(), "a type it does not offer closes empty");
    }
}
