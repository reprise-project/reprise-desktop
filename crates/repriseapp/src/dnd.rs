//! The look-neutral half of drag and drop between programs: the actions a
//! drag can end in, the mime types a drag of files offers and a drop takes,
//! and their bytes. The host in `lib.rs` wires these to `wl_data_device`.
//!
//! A drag of files offers, best first, a document portal key
//! (`application/vnd.portal.filetransfer`, when the portal is there),
//! `text/uri-list`, `x-special/gnome-copied-files` and
//! `text/plain;charset=utf-8` (the paths, one per line), then the mark that
//! its URI list names the files themselves. A drop takes files as
//! `text/uri-list`, files and web links together as `text/x-moz-url` or a
//! URI list (`reprisefs::dropdata`), and text as UTF-8 plain text.
//! A sandboxed program's drag that offers a portal key has its files read
//! through the portal in place of its URI list, which names paths inside
//! the sandbox. Its URI list is taken only for document portal files
//! ([`held_drop`]), and its files are only ever copied.

use crate::URI_LIST;
use reprisekit::raster::Canvas;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wayland_client::protocol::wl_data_device_manager::DndAction;

/// `FILE_MIMES` is what a drag of files offers after the portal key;
/// `TEXT_UTF8` is also the text mime a drop prefers.
pub use reprisefs::dropdata::{
    Dropped as Uri, FILE_MIMES, FILE_TRANSFER, HOST_FILES, MOZ_URL, SANDBOXED, STAGED, TEXT_UTF8,
    encode_files,
};

/// What a drag of files offers, best first: the portal key and the mark
/// only when `portal`.
pub fn source_mimes(portal: bool) -> Vec<&'static str> {
    portal
        .then_some(FILE_TRANSFER)
        .into_iter()
        .chain(FILE_MIMES)
        .chain(portal.then_some(HOST_FILES))
        .collect()
}

/// Whether the files of a drag offering `mimes` come through the portal:
/// never for the program's `own` drag.
pub fn through_portal(mimes: &[String], own: bool) -> bool {
    !own && reprisefs::dropdata::through_portal(mimes)
}

/// The text mimes a drop takes, best first.
const TEXT_MIMES: [&str; 3] = [TEXT_UTF8, "text/plain", "UTF8_STRING"];

/// What a drop does with the data. Wayland has no link action: a shortcut
/// drag arrives as `Ask`, and the target offers its drop menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragAction {
    Copy,
    Move,
    Ask,
}

impl DragAction {
    pub fn to_wl(self) -> DndAction {
        match self {
            DragAction::Copy => DndAction::Copy,
            DragAction::Move => DndAction::Move,
            DragAction::Ask => DndAction::Ask,
        }
    }

    /// The action the compositor picked, None for none. A set with several
    /// bits reads as its first, in the order copy, move, ask.
    pub fn from_wl(a: DndAction) -> Option<DragAction> {
        if a.contains(DndAction::Copy) {
            Some(DragAction::Copy)
        } else if a.contains(DndAction::Move) {
            Some(DragAction::Move)
        } else if a.contains(DndAction::Ask) {
            Some(DragAction::Ask)
        } else {
            None
        }
    }
}

/// A set of actions a drop target takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions {
    pub copy: bool,
    pub move_: bool,
    pub ask: bool,
}

impl Actions {
    pub const ALL: Actions = Actions {
        copy: true,
        move_: true,
        ask: true,
    };

    pub fn to_wl(self) -> DndAction {
        let mut a = DndAction::empty();
        if self.copy {
            a |= DndAction::Copy;
        }
        if self.move_ {
            a |= DndAction::Move;
        }
        if self.ask {
            a |= DndAction::Ask;
        }
        a
    }

    pub fn from_wl(a: DndAction) -> Actions {
        Actions {
            copy: a.contains(DndAction::Copy),
            move_: a.contains(DndAction::Move),
            ask: a.contains(DndAction::Ask),
        }
    }

    /// These, less a move when `copy_only`: the files come through the portal, which lent them
    /// read-only, or from a sandboxed program.
    pub fn read_as(self, copy_only: bool) -> Actions {
        Actions {
            move_: self.move_ && !copy_only,
            ..self
        }
    }

    pub fn has(self, action: DragAction) -> bool {
        match action {
            DragAction::Copy => self.copy,
            DragAction::Move => self.move_,
            DragAction::Ask => self.ask,
        }
    }
}

/// Which kind of data a drop takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    Files,
    /// Files and web links; an entry that is neither (`javascript:`,
    /// another host's file) is skipped.
    Uris,
    Text,
}

/// What a drag over a window carries, read from its mime types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offered {
    pub files: bool,
    /// A browser's link (`text/x-moz-url`). A URI list is `files` even
    /// when it holds web addresses: read it as `DropKind::Uris` to keep them.
    pub link: bool,
    pub text: bool,
}

impl Offered {
    pub fn from_mimes(mimes: &[String]) -> Offered {
        Offered {
            files: mime_for(mimes, DropKind::Files).is_some() || has(mimes, FILE_TRANSFER),
            link: mimes.iter().any(|m| m == MOZ_URL),
            text: mime_for(mimes, DropKind::Text).is_some(),
        }
    }
}

/// The mime a drop of `kind` receives from an offer of `mimes`, if any.
pub fn mime_for(mimes: &[String], kind: DropKind) -> Option<&'static str> {
    let want: &[&'static str] = match kind {
        DropKind::Files => &[URI_LIST],
        DropKind::Uris => &[MOZ_URL, URI_LIST],
        DropKind::Text => &TEXT_MIMES,
    };
    want.iter().copied().find(|w| mimes.iter().any(|m| m == w))
}

fn has(mimes: &[String], mime: &str) -> bool {
    mimes.iter().any(|m| m == mime)
}

/// The mime the files of a drag offering `mimes` are read ahead of its
/// drop in: the portal key when `portal`, else the names of a staged drag's
/// files, else the URI list. None without files.
pub fn peek_mime(mimes: &[String], portal: bool) -> Option<&'static str> {
    if portal {
        Some(FILE_TRANSFER)
    } else if has(mimes, STAGED) {
        Some(STAGED)
    } else {
        has(mimes, URI_LIST).then_some(URI_LIST)
    }
}

/// The mime a drop of `kind` is read in, `files` when the files read ahead
/// of the drop were there. A `DropKind::Uris` drop then reads its URI list
/// even when a browser's link is offered too; a drag of web links alone is
/// read as the link, which names the page. With `portal`, the portal key
/// replaces the URI list.
pub fn drop_mime(
    mimes: &[String],
    kind: DropKind,
    files: bool,
    portal: bool,
) -> Option<&'static str> {
    let mime = if kind == DropKind::Uris && files && has(mimes, URI_LIST) {
        Some(URI_LIST)
    } else {
        mime_for(mimes, kind)
    };
    let portal = kind != DropKind::Text && portal;
    match mime {
        Some(URI_LIST) | None if portal => Some(FILE_TRANSFER),
        m => m,
    }
}

/// A target's answer to a drag over it: take `kind` with one of `actions`,
/// `preferred` when the held modifiers pick none (it must be in `actions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accept {
    pub kind: DropKind,
    pub actions: Actions,
    pub preferred: Option<DragAction>,
}

impl Accept {
    /// `set_actions`'s two arguments; a preferred action outside `actions`
    /// is dropped (sending it is a protocol error).
    pub fn wl_actions(&self) -> (DndAction, DndAction) {
        let preferred = self
            .preferred
            .filter(|p| self.actions.has(*p))
            .map_or(DndAction::empty(), DragAction::to_wl);
        (self.actions.to_wl(), preferred)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// The `file://` URIs of a URI list as paths; other URIs are skipped.
    Files(Vec<PathBuf>),
    /// The files and web links of a `DropKind::Uris` drop, in order.
    Uris(Vec<Uri>),
    Text(String),
}

/// A drop of `kind` that brought `paths` through the portal.
pub fn portal_drop(paths: Vec<PathBuf>, kind: DropKind) -> Dropped {
    match kind {
        DropKind::Uris => Dropped::Uris(paths.into_iter().map(Uri::File).collect()),
        DropKind::Files | DropKind::Text => Dropped::Files(paths),
    }
}

/// Decode a drop's bytes, received as `kind` in `mime` (not the portal key).
pub fn decode_drop(bytes: &[u8], kind: DropKind, mime: &str) -> Dropped {
    match kind {
        DropKind::Files => Dropped::Files(reprisefs::fileclip::decode(URI_LIST, bytes).paths),
        DropKind::Uris => Dropped::Uris(reprisefs::dropdata::decode(mime, bytes)),
        DropKind::Text => Dropped::Text(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// A drop read from a sandboxed program's URI list (`sandboxed`), None when it names anything
/// but files the document portal exported: any other path is one inside its sandbox.
pub fn held_drop(data: Dropped, sandboxed: bool) -> Option<Dropped> {
    let taken = match &data {
        _ if !sandboxed => true,
        Dropped::Files(paths) => held_paths(paths),
        Dropped::Uris(items) => reprisefs::dropdata::all_exported(items),
        Dropped::Text(_) => true,
    };
    taken.then_some(data)
}

/// Whether a sandboxed program's `paths` are taken: each one the document portal exported.
pub fn held_paths(paths: &[PathBuf]) -> bool {
    !paths.is_empty()
        && paths
            .iter()
            .all(|p| reprisefs::dropdata::portal_exported(p))
}

/// A drag of files a window starts: the paths, the image under the pointer,
/// and the point of the image the pointer holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragOut {
    pub paths: Vec<PathBuf>,
    pub image: Canvas,
    /// Logical px from the image's top-left.
    pub grab: (i32, i32),
    /// Set when the files are written after the drag starts: the drag offers only Copy, no
    /// portal key and [`reprisefs::dropdata::STAGED`].
    pub staged: Option<Staged>,
}

/// Files a drag names before they are written. Each request for them, before the drop or at it
/// and as often as a target asks, waits with no time limit for [`Staged::done`], then gets the
/// paths, or its pipe closed empty when they did not come out. The host tells how the drag ended
/// through [`Staged::end`].
#[derive(Clone)]
pub struct Staged(Arc<StagedShared>);

struct StagedShared {
    state: Mutex<StagedState>,
    wake: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct StagedState {
    /// The pipes of the requests waiting, each with the bytes it is sent.
    waiting: Vec<(std::os::fd::OwnedFd, Arc<[u8]>)>,
    out: Option<bool>,
    ended: Option<bool>,
}

/// The most requests one staged drag holds a pipe for; more are closed empty.
const MAX_WAITING: usize = 16;

impl Staged {
    /// `wake` runs when the drag ends.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Staged {
        Staged(Arc::new(StagedShared {
            state: Mutex::default(),
            wake: Box::new(wake),
        }))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, StagedState> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The files are there (`ok`) or will not be: each waiting request gets its bytes, or its
    /// pipe closed empty, and so does each later one.
    pub fn done(&self, ok: bool) {
        let waiting = {
            let mut s = self.state();
            s.out = Some(ok);
            std::mem::take(&mut s.waiting)
        };
        for (fd, bytes) in waiting.into_iter().filter(|_| ok) {
            reprisefs::pipe::write_detached(fd, bytes);
        }
    }

    /// A target asks for `bytes` on `fd`, sent once the files are out: the host's part.
    pub fn send(&self, fd: std::os::fd::OwnedFd, bytes: Arc<[u8]>) {
        let mut s = self.state();
        match s.out {
            Some(true) => reprisefs::pipe::write_detached(fd, bytes),
            Some(false) => {}
            None if s.ended == Some(false) || s.waiting.len() >= MAX_WAITING => {}
            None => s.waiting.push((fd, bytes)),
        }
    }

    /// The drag ended, `taken` when a target took the drop: the host's part. A drag nobody took
    /// closes the requests waiting.
    pub fn end(&self, taken: bool) {
        {
            let mut s = self.state();
            if s.ended.is_some() {
                return;
            }
            s.ended = Some(taken);
            if !taken {
                s.waiting.clear();
            }
        }
        (self.0.wake)();
    }

    /// None while the drag runs; then whether a target took the drop.
    pub fn ended(&self) -> Option<bool> {
        self.state().ended
    }
}

impl PartialEq for Staged {
    fn eq(&self, other: &Staged) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Staged {}

impl std::fmt::Debug for Staged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Staged")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GNOME_FILES;

    fn mimes(m: &[&str]) -> Vec<String> {
        m.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn actions_map_to_their_wayland_bits() {
        assert_eq!(Actions::ALL.to_wl(), DndAction::all());
        assert_eq!(Actions::default().to_wl(), DndAction::empty());
        let cm = Actions {
            copy: true,
            move_: true,
            ask: false,
        };
        assert_eq!(cm.to_wl(), DndAction::Copy | DndAction::Move);
        for a in [DragAction::Copy, DragAction::Move, DragAction::Ask] {
            assert_eq!(DragAction::from_wl(a.to_wl()), Some(a));
        }
        assert_eq!(DragAction::from_wl(DndAction::empty()), None);
        assert_eq!(Actions::from_wl(DndAction::all()), Actions::ALL);
        assert_eq!(Actions::from_wl(cm.to_wl()), cm);
    }

    #[test]
    fn accept_sends_its_preferred_action_only_when_it_is_taken() {
        let a = Accept {
            kind: DropKind::Files,
            actions: Actions {
                copy: true,
                move_: false,
                ask: true,
            },
            preferred: Some(DragAction::Copy),
        };
        assert_eq!(
            a.wl_actions(),
            (DndAction::Copy | DndAction::Ask, DndAction::Copy)
        );
        let outside = Accept {
            preferred: Some(DragAction::Move),
            ..a
        };
        assert_eq!(outside.wl_actions().1, DndAction::empty());
        let none = Accept {
            preferred: None,
            ..a
        };
        assert_eq!(none.wl_actions().1, DndAction::empty());
    }

    #[test]
    fn files_come_as_a_uri_list_and_text_as_utf8_first() {
        let m = mimes(&[
            "UTF8_STRING",
            "text/plain",
            TEXT_UTF8,
            URI_LIST,
            GNOME_FILES,
        ]);
        assert_eq!(mime_for(&m, DropKind::Files), Some(URI_LIST));
        assert_eq!(mime_for(&m, DropKind::Text), Some(TEXT_UTF8));
        let plain = mimes(&["UTF8_STRING", "text/plain"]);
        assert_eq!(mime_for(&plain, DropKind::Text), Some("text/plain"));
        assert_eq!(mime_for(&plain, DropKind::Files), None);
        // The GNOME clipboard form alone is not a drop of files.
        assert_eq!(mime_for(&mimes(&[GNOME_FILES]), DropKind::Files), None);
    }

    #[test]
    fn offered_names_what_a_drag_carries() {
        assert_eq!(
            Offered::from_mimes(&mimes(&FILE_MIMES)),
            Offered {
                files: true,
                link: false,
                text: true
            }
        );
        assert_eq!(
            Offered::from_mimes(&mimes(&["image/png"])),
            Offered::default()
        );
    }

    #[test]
    fn the_plain_text_form_leaves_out_paths_it_cannot_carry() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let paths = vec![
            PathBuf::from("/tmp/two\nlines"),
            PathBuf::from(OsStr::from_bytes(b"/tmp/\xff")),
            PathBuf::from("/tmp/ok"),
        ];
        assert_eq!(encode_files(&paths, TEXT_UTF8).unwrap(), b"/tmp/ok");
        let uris = encode_files(&paths, URI_LIST).unwrap();
        assert_eq!(
            decode_drop(&uris, DropKind::Files, URI_LIST),
            Dropped::Files(paths)
        );
    }

    #[test]
    fn a_dropped_uri_list_decodes_its_files_only() {
        let bytes =
            b"# a comment\r\nfile:///tmp/a%20b.txt\r\nhttps://example.org/\r\nfile:///home/c\r\n";
        assert_eq!(
            decode_drop(bytes, DropKind::Files, URI_LIST),
            Dropped::Files(vec![
                PathBuf::from("/tmp/a b.txt"),
                PathBuf::from("/home/c")
            ])
        );
        assert_eq!(
            decode_drop("h\u{e9}".as_bytes(), DropKind::Text, TEXT_UTF8),
            Dropped::Text("h\u{e9}".into())
        );
    }

    #[test]
    fn what_a_drag_sends_comes_back_as_the_same_paths() {
        let paths = vec![PathBuf::from("/tmp/100% sure/#1.txt")];
        let bytes = encode_files(&paths, URI_LIST).unwrap();
        assert_eq!(
            decode_drop(&bytes, DropKind::Files, URI_LIST),
            Dropped::Files(paths)
        );
    }

    #[test]
    fn a_uri_drop_keeps_files_and_web_links_and_skips_the_rest() {
        let m = mimes(&[URI_LIST, MOZ_URL, TEXT_UTF8]);
        assert_eq!(mime_for(&m, DropKind::Uris), Some(MOZ_URL));
        assert!(Offered::from_mimes(&m).link);
        assert_eq!(drop_mime(&m, DropKind::Uris, false, false), Some(MOZ_URL));
        assert_eq!(drop_mime(&m, DropKind::Uris, true, false), Some(URI_LIST));
        assert_eq!(
            drop_mime(&mimes(&[MOZ_URL]), DropKind::Uris, true, false),
            Some(MOZ_URL)
        );
        let list = mimes(&[URI_LIST]);
        assert_eq!(mime_for(&list, DropKind::Uris), Some(URI_LIST));
        assert!(!Offered::from_mimes(&list).link);
        assert_eq!(
            decode_drop(
                b"# c\r\nhttps://a.org/\r\nfile:///tmp/x\r\njavascript:alert(1)\r\n",
                DropKind::Uris,
                URI_LIST
            ),
            Dropped::Uris(vec![
                Uri::Url {
                    url: "https://a.org/".into(),
                    title: None
                },
                Uri::File(PathBuf::from("/tmp/x")),
            ])
        );
    }

    #[test]
    fn a_drag_offers_the_portal_key_first_only_when_the_portal_is_there() {
        assert_eq!(
            source_mimes(true),
            [FILE_TRANSFER, URI_LIST, GNOME_FILES, TEXT_UTF8, HOST_FILES]
        );
        assert_eq!(source_mimes(false), FILE_MIMES);
    }

    #[test]
    fn a_staged_drags_files_are_peeked_by_name_and_dropped_as_their_uri_list() {
        let m = mimes(&[STAGED, URI_LIST, TEXT_UTF8]);
        assert_eq!(peek_mime(&m, false), Some(STAGED));
        assert_eq!(drop_mime(&m, DropKind::Uris, true, false), Some(URI_LIST));
    }

    #[test]
    fn a_sandboxed_programs_files_come_through_the_portal() {
        let m = mimes(&[FILE_TRANSFER, URI_LIST, TEXT_UTF8]);
        assert!(through_portal(&m, false));
        // Our own drag, and one marked as naming the files themselves,
        // are read as their URI list.
        assert!(!through_portal(&m, true));
        let host = mimes(&[FILE_TRANSFER, URI_LIST, HOST_FILES]);
        assert!(!through_portal(&host, false));
        assert_eq!(peek_mime(&m, true), Some(FILE_TRANSFER));
        assert_eq!(peek_mime(&host, false), Some(URI_LIST));
        assert_eq!(peek_mime(&mimes(&[TEXT_UTF8]), false), None);
        for kind in [DropKind::Files, DropKind::Uris] {
            for files in [false, true] {
                assert_eq!(drop_mime(&m, kind, files, true), Some(FILE_TRANSFER));
                assert_eq!(drop_mime(&m, kind, files, false), Some(URI_LIST));
            }
        }
        assert_eq!(drop_mime(&m, DropKind::Text, false, true), Some(TEXT_UTF8));
        let key = mimes(&[FILE_TRANSFER]);
        assert!(Offered::from_mimes(&key).files);
        assert_eq!(
            drop_mime(&key, DropKind::Files, true, true),
            Some(FILE_TRANSFER)
        );
        // A browser's link still names its page.
        let link = mimes(&[MOZ_URL, FILE_TRANSFER, URI_LIST]);
        assert_eq!(drop_mime(&link, DropKind::Uris, false, true), Some(MOZ_URL));
        assert_eq!(
            drop_mime(&link, DropKind::Uris, true, true),
            Some(FILE_TRANSFER)
        );
    }

    #[test]
    fn files_through_the_portal_are_never_moved() {
        assert_eq!(
            Actions::ALL.read_as(true),
            Actions {
                copy: true,
                move_: false,
                ask: true
            }
        );
        assert_eq!(Actions::ALL.read_as(false), Actions::ALL);
    }

    #[test]
    fn files_from_the_portal_drop_as_the_kind_asked_for() {
        let paths = vec![PathBuf::from("/run/user/1000/doc/1a2b/x.txt")];
        assert_eq!(
            portal_drop(paths.clone(), DropKind::Files),
            Dropped::Files(paths.clone())
        );
        assert_eq!(
            portal_drop(paths.clone(), DropKind::Uris),
            Dropped::Uris(vec![Uri::File(paths[0].clone())])
        );
    }

    #[test]
    fn a_sandboxed_programs_uri_list_drops_only_document_portal_files() {
        let doc = reprisefs::dropdata::document_root().join("abc/x.png");
        let key = PathBuf::from("/home/u/.ssh/id_ed25519");
        let list = |paths: &[&PathBuf]| {
            let uris: String = paths
                .iter()
                .map(|p| format!("file://{}\r\n", p.display()))
                .collect();
            uris.into_bytes()
        };
        for kind in [DropKind::Files, DropKind::Uris] {
            let held = |paths: &[&PathBuf], sandboxed| {
                held_drop(decode_drop(&list(paths), kind, URI_LIST), sandboxed)
            };
            assert!(held(&[&doc], true).is_some(), "{kind:?}: a portal file");
            assert_eq!(held(&[&key], true), None, "{kind:?}: a path in the sandbox");
            assert_eq!(
                held(&[&doc, &key], true),
                None,
                "{kind:?}: one path outside"
            );
            assert!(held(&[&key], false).is_some(), "{kind:?}: a host program's");
        }
        let text = Dropped::Text("file:///home/u/.ssh/id_ed25519".into());
        assert_eq!(held_drop(text.clone(), true), Some(text), "text is text");
    }

    #[test]
    fn each_request_of_a_staged_drag_waits_for_its_files_and_hears_whether_they_came_out() {
        use std::io::Read;
        let pipe = || {
            let (r, w) = std::io::pipe().unwrap();
            (r, std::os::fd::OwnedFd::from(w))
        };
        let read = |mut r: std::io::PipeReader| {
            let mut got = Vec::new();
            r.read_to_end(&mut got).unwrap();
            got
        };
        let woke = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woke.clone();
        let staged = Staged::new(move || {
            w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let (before, fd) = pipe();
        staged.send(fd, Arc::from(&b"file:///a"[..]));
        let (again, fd) = pipe();
        staged.send(fd, Arc::from(&b"file:///a"[..]));
        staged.done(true);
        assert_eq!(read(before), b"file:///a", "a read before the drop waits");
        assert_eq!(read(again), b"file:///a", "and so does a second");
        let (after, fd) = pipe();
        staged.send(fd, Arc::from(&b"file:///a"[..]));
        assert_eq!(
            read(after),
            b"file:///a",
            "one after they are out gets them at once"
        );
        assert_eq!(staged.ended(), None);
        staged.end(true);
        staged.end(false);
        assert_eq!(staged.ended(), Some(true), "a drag ends once");
        assert_eq!(woke.load(std::sync::atomic::Ordering::Relaxed), 1);

        let failed = Staged::new(|| {});
        let (empty, fd) = pipe();
        failed.send(fd, Arc::from(&b"file:///a"[..]));
        failed.done(false);
        assert!(read(empty).is_empty(), "a copy that failed sends nothing");

        let untaken = Staged::new(|| {});
        let (closed, fd) = pipe();
        untaken.send(fd, Arc::from(&b"file:///a"[..]));
        untaken.end(false);
        assert!(
            read(closed).is_empty(),
            "a drag nobody took closes its pipes"
        );
    }
}
