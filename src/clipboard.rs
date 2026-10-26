//! The seat's selections: clipboard and primary selection. Reprise's own edit
//! fields copy by offering text ([`State::clip_copy`]) and paste by asking the
//! owner over a pipe ([`State::clip_paste`]); the event loop hands the text over
//! once it has all arrived, since reading in place would deadlock against a
//! client waiting on our flush. Desktop icons Cut or Copied go out as a selection
//! too ([`file_offers`]). X11 programs share both selections through Xwayland's
//! window manager (`xwayland`), held as [`Owner::X11`].

use std::sync::Arc;

use smithay::{
    input::Seat,
    reexports::calloop::{
        Interest, Mode, PostAction, RegistrationToken,
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    wayland::selection::{
        SelectionHandler, SelectionSource, SelectionTarget,
        data_device::{
            SelectionRequestError, clear_data_device_selection,
            current_data_device_selection_userdata, request_data_device_client_selection,
            set_data_device_selection,
        },
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
    },
    xwayland::xwm::XwmId,
};

use reprisefs::dropdata::GNOME_FILES;
pub use reprisefs::fileclip::FileClip;
use reprisefs::fileclip::{self, KDE_CUT};
use reprisefs::pipe::{PASTE_TIMEOUT, PasteBuf};

use crate::compositor::State;

pub const TEXT_MIMES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "TEXT",
];

pub type Offer = (String, Arc<[u8]>);

pub type Offers = Arc<[Offer]>;

#[derive(Clone, Debug)]
pub enum Owner {
    Reprise(Offers),
    X11(XwmId),
}

impl Owner {
    pub fn is_x11(&self, xwm: XwmId) -> bool {
        matches!(self, Owner::X11(id) if *id == xwm)
    }
}

pub struct Clip {
    pub primary: PrimarySelectionState,
    /// Read through [`State::clip_offered`].
    pub(crate) offered: Vec<String>,
    paste: Option<Paste>,
}

impl Clip {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> Clip {
        Clip {
            primary: PrimarySelectionState::new::<State>(dh),
            offered: Vec::new(),
            paste: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteTarget {
    Dialog(u64),
    Rename(usize),
    Launcher,
    StartPanel,
    ClipView,
    EmojiView,
    NotifyReply,
}

pub fn file_offers(clip: &FileClip) -> Option<Vec<Offer>> {
    let offers = fileclip::offers(clip)?;
    Some(
        offers
            .into_iter()
            .map(|(m, b)| (m.to_string(), Arc::from(b)))
            .collect(),
    )
}

pub fn own_files(offers: &[Offer]) -> Option<FileClip> {
    answer(offers, GNOME_FILES).map(|b| fileclip::decode(GNOME_FILES, b))
}

struct Paste {
    pipe: RegistrationToken,
    timer: RegistrationToken,
}

pub fn text_mime(offered: &[String]) -> Option<&'static str> {
    TEXT_MIMES
        .into_iter()
        .find(|m| offered.iter().any(|o| o == m))
}

pub fn text_offers(text: &Arc<[u8]>) -> Vec<Offer> {
    TEXT_MIMES
        .iter()
        .map(|m| (m.to_string(), text.clone()))
        .collect()
}

pub fn answer<'a>(offers: &'a [Offer], mime: &str) -> Option<&'a Arc<[u8]>> {
    offers.iter().find(|(m, _)| m == mime).map(|(_, b)| b)
}

#[derive(Debug, PartialEq, Eq)]
enum Got {
    More,
    Done,
    TooLarge,
}

fn read_some(mut pipe: impl std::io::Read, buf: &mut PasteBuf) -> Got {
    let mut chunk = [0u8; 16 * 1024];
    let read = match pipe.read(&mut chunk) {
        Ok(0) => Got::Done,
        Ok(n) if buf.push(&chunk[..n]) => Got::More,
        Ok(_) => Got::TooLarge,
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Got::More,
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Got::More,
        Err(e) => {
            log::debug!("paste: read: {e}");
            Got::Done
        }
    };
    zeroize::Zeroize::zeroize(&mut chunk);
    read
}

impl SelectionHandler for State {
    type SelectionUserData = Owner;

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _: Seat<Self>,
    ) {
        let mimes = source.map(|s| s.mime_types());
        if ty == SelectionTarget::Clipboard {
            self.clip.offered = mimes.clone().unwrap_or_default();
        }
        self.wayland_selection_to_x11(ty, mimes);
        if ty == SelectionTarget::Clipboard {
            self.clipboard_changed();
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _: Seat<Self>,
        owner: &Owner,
    ) {
        match owner {
            Owner::Reprise(offers) => {
                if let Some(bytes) = answer(offers, &mime_type) {
                    reprisefs::pipe::write_detached(fd, bytes.clone());
                }
            }
            Owner::X11(_) => self.x11_selection_to_wayland(ty, mime_type, fd),
        }
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.clip.primary
    }
}

impl State {
    pub fn clip_copy(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clip_offer(&TEXT_MIMES, Arc::from(text.as_bytes()));
    }

    pub fn clip_offer(&mut self, mimes: &[&str], bytes: Arc<[u8]>) {
        let offers = mimes
            .iter()
            .map(|m| (m.to_string(), bytes.clone()))
            .collect();
        self.clip_offer_each(offers);
    }

    pub fn clip_offer_each(&mut self, offers: Vec<Offer>) {
        let mimes: Vec<String> = offers.iter().map(|(m, _)| m.clone()).collect();
        self.clip.offered.clear();
        self.wayland_selection_to_x11(SelectionTarget::Clipboard, Some(mimes.clone()));
        set_data_device_selection(
            &self.dh,
            &self.seat,
            mimes,
            Owner::Reprise(Offers::from(offers)),
        );
        self.clipboard_changed();
    }

    pub fn clip_clear(&mut self) {
        self.clip.offered.clear();
        self.wayland_selection_to_x11(SelectionTarget::Clipboard, None);
        clear_data_device_selection(&self.dh, &self.seat);
        self.clipboard_changed();
    }

    /// Someone else's selection or a new one of Reprise's. Smithay tells
    /// `new_selection` before the seat holds the new source, so the history
    /// record waits for the next idle pass.
    pub(crate) fn clipboard_changed(&mut self) {
        self.relayout_desktop();
        self.handle.insert_idle(|state| state.record_selection());
    }

    /// The types a client's clipboard offers. Smithay drops a destroyed source (its client
    /// gone) without a `new_selection`, so a source no longer held offers nothing.
    pub(crate) fn clip_offered(&self) -> &[String] {
        if self.clip.offered.is_empty() || self.clipboard_held() {
            &self.clip.offered
        } else {
            &[]
        }
    }

    pub(crate) fn clipboard_held(&self) -> bool {
        if current_data_device_selection_userdata(&self.seat).is_some() {
            return true;
        }
        let Ok(null) = std::fs::File::open("/dev/null") else {
            return true;
        };
        // No source offers the empty type, so a held one refuses it and is sent nothing.
        !matches!(
            request_data_device_client_selection(&self.seat, String::new(), null.into()),
            Err(SelectionRequestError::NoSelection)
        )
    }

    pub fn own_file_clip(&self) -> Option<FileClip> {
        own_files(&self.own_offers()?)
    }

    pub fn clip_has_files(&self) -> bool {
        match self.own_offers() {
            // No stat: a file gone since is skipped by the paste.
            Some(offers) => own_files(&offers).is_some_and(|c| !c.paths.is_empty()),
            None => fileclip::source(self.clip_offered()).is_some(),
        }
    }

    /// Read the files on the clipboard and hand them to `done` once all arrived. A cut marked
    /// in [`KDE_CUT`] is read first, then the files.
    pub fn clip_files(&mut self, done: impl FnOnce(&mut State, FileClip) + 'static) {
        if let Some(offers) = self.own_offers() {
            if let Some(clip) = own_files(&offers) {
                done(self, clip);
            }
            return;
        }
        let Some(source) = fileclip::source(self.clip_offered()) else {
            log::debug!("paste: the clipboard offers no files");
            return;
        };
        let read_files = move |state: &mut State, kde_cut: bool| {
            state.clip_read(source.files, move |state, buf| {
                let mut clip = fileclip::decode(source.files, buf.bytes());
                clip.cut |= kde_cut;
                done(state, clip)
            });
        };
        if source.kde_cut {
            self.clip_read(KDE_CUT, move |state, buf| {
                read_files(state, fileclip::is_cut_marker(buf.bytes()))
            });
        } else {
            read_files(self, false);
        }
    }

    pub fn own_offers(&self) -> Option<Offers> {
        current_data_device_selection_userdata(&self.seat).and_then(|o| match &*o {
            Owner::Reprise(offers) => Some(offers.clone()),
            Owner::X11(_) => None,
        })
    }

    fn x11_clipboard(&self) -> bool {
        current_data_device_selection_userdata(&self.seat)
            .is_some_and(|o| matches!(*o, Owner::X11(_)))
    }

    pub fn clip_paste(&mut self, target: PasteTarget) {
        self.abandon_paste();
        if let Some(offers) = self.own_offers() {
            if let Some(text) = TEXT_MIMES.into_iter().find_map(|m| answer(&offers, m)) {
                let text = String::from_utf8_lossy(text).into_owned();
                self.deliver_paste(target, text);
            }
            return;
        }
        let Some(mime) = text_mime(self.clip_offered()) else {
            log::debug!(
                "paste: the clipboard offers no text ({:?})",
                self.clip_offered()
            );
            return;
        };
        self.clip_read(mime, move |state, buf| {
            let text = buf.into_text();
            if !text.is_empty() {
                state.deliver_paste(target, text);
            }
        });
    }

    fn clip_read(&mut self, mime: &str, done: impl FnOnce(&mut State, PasteBuf) + 'static) {
        self.abandon_paste();
        let (read, write) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("paste: pipe: {e}");
                return;
            }
        };
        if let Err(e) = self.request_clipboard(mime, write.into()) {
            log::debug!("paste: {e}");
            return;
        }
        let mut buf = PasteBuf::default();
        let mut done = Some(done);
        let pipe = self.handle.insert_source(
            Generic::new(read, Interest::READ, Mode::Level),
            move |_, pipe, state| {
                let got = read_some(&**pipe, &mut buf);
                if got == Got::More {
                    return Ok(PostAction::Continue);
                }
                // Before `done`, which may start the next read.
                if let Some(p) = state.clip.paste.take() {
                    state.handle.remove(p.timer);
                }
                if got == Got::TooLarge {
                    log::warn!("paste: the clipboard is larger than a paste; dropped");
                } else if let Some(done) = done.take() {
                    done(state, std::mem::take(&mut buf));
                }
                Ok(PostAction::Remove)
            },
        );
        let timer =
            self.handle
                .insert_source(Timer::from_duration(PASTE_TIMEOUT), |_, _, state| {
                    log::warn!("paste: the clipboard's owner did not finish in time");
                    if let Some(p) = state.clip.paste.take() {
                        state.handle.remove(p.pipe);
                    }
                    TimeoutAction::Drop
                });
        match (pipe, timer) {
            (Ok(pipe), Ok(timer)) => self.clip.paste = Some(Paste { pipe, timer }),
            (pipe, timer) => {
                log::warn!("paste: cannot watch the pipe");
                for t in [pipe.ok(), timer.ok()].into_iter().flatten() {
                    self.handle.remove(t);
                }
            }
        }
    }

    pub(crate) fn request_clipboard(
        &mut self,
        mime: &str,
        fd: std::os::fd::OwnedFd,
    ) -> Result<(), String> {
        if self.x11_clipboard() {
            self.x11_selection_to_wayland(SelectionTarget::Clipboard, mime.into(), fd);
            Ok(())
        } else {
            request_data_device_client_selection(&self.seat, mime.into(), fd)
                .map_err(|e| e.to_string())
        }
    }

    fn abandon_paste(&mut self) {
        if let Some(p) = self.clip.paste.take() {
            self.handle.remove(p.pipe);
            self.handle.remove(p.timer);
        }
    }

    fn focused_paste_target(&self) -> Option<PasteTarget> {
        use crate::caret::Editor;
        match self.editor()? {
            Editor::Dialog => Some(PasteTarget::Dialog(self.dialogs.views.last()?.id)),
            Editor::Rename => self
                .desktop
                .as_ref()
                .and_then(|d| d.rename.as_ref())
                .map(|(i, _)| PasteTarget::Rename(*i)),
            Editor::Launcher => Some(PasteTarget::Launcher),
            Editor::StartPanel => Some(PasteTarget::StartPanel),
            Editor::ClipView => Some(PasteTarget::ClipView),
            Editor::EmojiView => Some(PasteTarget::EmojiView),
            Editor::NotifyReply => Some(PasteTarget::NotifyReply),
        }
    }

    fn selected_text(&self) -> String {
        use crate::caret::Editor;
        match self.editor() {
            Some(Editor::Dialog) => self
                .dialogs
                .views
                .last()
                .and_then(|v| v.dialog.focused_edit())
                .and_then(|e| e.copy_text()),
            Some(Editor::Rename) => self
                .desktop
                .as_ref()
                .and_then(|d| d.rename.as_ref())
                .map(|(_, edit)| edit.selected_text().to_string()),
            Some(Editor::Launcher) => self
                .launcher
                .as_ref()
                .map(|v| v.model.edit().selected_text().to_string()),
            Some(Editor::StartPanel) => self
                .start_panel
                .as_ref()
                .map(|v| v.model.edit().selected_text().to_string()),
            Some(Editor::ClipView) => self
                .panels
                .clipview
                .as_ref()
                .map(|v| v.model.edit().selected_text().to_string()),
            Some(Editor::EmojiView) => self
                .panels
                .emojiview
                .as_ref()
                .map(|v| v.model.edit().selected_text().to_string()),
            Some(Editor::NotifyReply) => {
                crate::notifyview::reply_edit(self).map(|e| e.selected_text().to_string())
            }
            None => None,
        }
        .unwrap_or_default()
    }

    pub fn clip_edit_key(&mut self, key: &reprisekit::textedit::EditKey) {
        use reprisekit::textedit::EditKey;
        match key {
            EditKey::Paste => {
                if let Some(target) = self.focused_paste_target() {
                    self.clip_paste(target);
                }
            }
            EditKey::Copy | EditKey::Cut => {
                let text = self.selected_text();
                self.clip_copy(&text);
            }
            _ => {}
        }
    }

    fn deliver_paste(&mut self, target: PasteTarget, text: String) {
        let mut text = text;
        if self.focused_paste_target() != Some(target) {
            log::debug!("paste: the field it was asked for lost the keyboard; dropped");
        } else {
            match target {
                PasteTarget::Dialog(_) => self.dialog_paste(&text),
                PasteTarget::Rename(_) => self.rename_paste(&text),
                PasteTarget::Launcher => self.launcher_paste(&text),
                PasteTarget::StartPanel => self.start_panel_paste(&text),
                PasteTarget::ClipView => self.clipview_paste(&text),
                PasteTarget::EmojiView => self.emoji_paste(&text),
                PasteTarget::NotifyReply => crate::notifyview::paste(self, &text),
            }
        }
        zeroize::Zeroize::zeroize(&mut text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_paste_asks_for_the_best_text_type_offered() {
        assert_eq!(
            text_mime(&strs(&["TEXT", "text/plain", "image/png"])),
            Some("text/plain")
        );
        assert_eq!(
            text_mime(&strs(&["UTF8_STRING", "text/plain;charset=utf-8"])),
            Some("text/plain;charset=utf-8")
        );
        assert_eq!(text_mime(&strs(&["image/png", "text/uri-list"])), None);
    }

    #[test]
    fn each_offered_type_answers_with_its_own_bytes() {
        let text: Arc<[u8]> = Arc::from(&b"hello"[..]);
        let png: Arc<[u8]> = Arc::from(&b"\x89PNG"[..]);
        let mut offers = text_offers(&text);
        offers.push(("image/png".into(), png.clone()));
        assert_eq!(offers.len(), TEXT_MIMES.len() + 1);
        for m in TEXT_MIMES {
            assert_eq!(answer(&offers, m).map(|b| &b[..]), Some(&b"hello"[..]));
        }
        assert_eq!(answer(&offers, "image/png"), Some(&png));
        assert_eq!(answer(&offers, "text/html"), None);
    }

    #[test]
    fn desktop_files_are_offered_in_every_form_and_read_back_as_cut() {
        let clip = FileClip {
            paths: vec!["/d/a b.txt".into()],
            cut: true,
        };
        let offers = file_offers(&clip).expect("one path");
        assert_eq!(answer(&offers, KDE_CUT).map(|b| &b[..]), Some(&b"1"[..]));
        assert_eq!(own_files(&offers), Some(clip));
        assert_eq!(file_offers(&FileClip::default()), None, "nothing to offer");
    }

    /// Turns the loop until the clipboard history holds `n` entries.
    fn recorded(h: &mut crate::harness::Harness, n: usize) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while h.state.panels.history.history.entries().count() < n {
            assert!(std::time::Instant::now() < end, "never recorded");
            h.run_for(5);
        }
    }

    #[test]
    fn a_destroyed_source_leaves_its_recorded_copy_and_paste_greys_without_one() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        h.map_toplevel(&mut c, "test.app", 300, 200);
        let source = c.copy(&["text/plain"], b"kept");
        h.roundtrip(&mut c);
        recorded(&mut h, 1);
        source.destroy();
        h.roundtrip(&mut c);
        assert!(h.state.clip_has_text(), "the recorded copy stays pastable");
        let own = h.state.own_offers().expect("Reprise holds the clipboard");
        assert_eq!(
            answer(&own, "text/plain").map(|b| &b[..]),
            Some(&b"kept"[..])
        );

        let source = c.copy(&["text/plain"], b"");
        h.roundtrip(&mut c);
        assert!(h.state.clip_has_text(), "the client's text is pastable");
        h.run_for(20);
        source.destroy();
        h.roundtrip(&mut c);
        assert!(
            !h.state.clip_has_text(),
            "a destroyed source with nothing recorded offers no text"
        );

        c.copy(&["text/uri-list"], b"file:///tmp/a\r\n");
        h.roundtrip(&mut c);
        assert!(h.state.clip_has_files(), "the client's files are pastable");
        drop(c);
        h.run_for(50);
        assert!(!h.state.clip_has_files(), "a gone client offers no files");
        assert!(!h.state.clip_has_text(), "nor the older recorded text");
    }

    #[test]
    fn a_copy_outlives_the_program_that_made_it_unless_history_is_off() {
        const UTF8: &str = "text/plain;charset=utf-8";
        for history in [true, false] {
            let text = if history {
                ""
            } else {
                "[clipboard]\nhistory = false\n"
            };
            let config = crate::config::Config::parse(text).unwrap();
            let mut h = crate::harness::Harness::with_config(config);
            h.output("TEST-1", 1024, 768);
            let mut a = h.connect();
            h.map_toplevel(&mut a, "test.a", 300, 200);
            a.copy(&[UTF8], b"from a");
            h.roundtrip(&mut a);
            if history {
                recorded(&mut h, 1);
            } else {
                h.run_for(20);
            }
            drop(a);
            h.run_for(50);

            let mut b = h.connect();
            b.data_device();
            h.map_toplevel(&mut b, "test.b", 300, 200);
            h.roundtrip(&mut b);
            if history {
                assert!(
                    b.selection_mimes().iter().any(|m| m == UTF8),
                    "b hears a text selection: {:?}",
                    b.selection_mimes()
                );
                assert_eq!(h.paste(&mut b, UTF8), b"from a");
            } else {
                assert_eq!(
                    b.side.selection, None,
                    "nothing is offered with history off"
                );
            }
        }
    }

    #[test]
    fn a_paste_reads_until_the_owner_closes() {
        let (r, mut w) = std::io::pipe().unwrap();
        let mut buf = PasteBuf::default();
        std::io::Write::write_all(&mut w, b"abc").unwrap();
        assert_eq!(read_some(&r, &mut buf), Got::More);
        std::io::Write::write_all(&mut w, b"def").unwrap();
        drop(w);
        assert_eq!(read_some(&r, &mut buf), Got::More);
        assert_eq!(read_some(&r, &mut buf), Got::Done);
        assert_eq!(buf.bytes(), b"abcdef");
    }

    #[test]
    fn a_paste_past_the_limit_is_dropped() {
        let mut buf = PasteBuf::default();
        let big = vec![b'x'; reprisefs::pipe::MAX_PASTE_BYTES];
        assert!(buf.push(&big));
        assert_eq!(read_some(&b"y"[..], &mut buf), Got::TooLarge);
        assert_eq!(buf.bytes().len(), reprisefs::pipe::MAX_PASTE_BYTES);
    }
}
