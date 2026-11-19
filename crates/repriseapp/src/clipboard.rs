//! Copy and paste: the clipboard, the primary selection and the text types a paste reads.

use crate::pastewait::{Held, PASTE_HOLD, PasteId};
use crate::{Event, FileClip, Host, KeyInput, MAIN, Reply, WinId};
use reprisefs::pipe::{PASTE_TIMEOUT, PasteBuf};
use reprisekit::textedit::EditKey;
use smithay_client_toolkit::{
    data_device_manager::{ReadPipe, data_offer::SelectionOffer},
    reexports::calloop::{
        LoopHandle, PostAction,
        timer::{TimeoutAction, Timer},
    },
};
use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub(crate) const TEXT_MIME: &str = "text/plain;charset=utf-8";

impl Host {
    /// Whether the clipboard holds text, read off the live offer as `CLIPBOARD_TEXT` is.
    pub(crate) fn clipboard_text(&self) -> bool {
        self.selection_offer()
            .is_some_and(|o| o.with_mime_types(text_mime).is_some())
    }

    /// The clipboard for a file dialog's File name
    /// ([`Extended::drop_file_dialog`](crate::Extended::drop_file_dialog)): Ctrl+C and Ctrl+X put its
    /// selection on the clipboard before the key reaches the dialog, and Ctrl+V asks for the
    /// clipboard's text, which goes into the field. `true` when the program does not get `ev`.
    pub(crate) fn file_dialog_clipboard(&mut self, win: WinId, ev: &Event) -> bool {
        let Some(fd) = self.app.drop_file_dialog(win) else {
            return false;
        };
        match ev {
            Event::Key(KeyInput {
                edit: Some(EditKey::Copy | EditKey::Cut),
                ..
            }) => {
                let text = (fd.dlg.focused_edit())
                    .filter(|e| !e.is_secret())
                    .map(|e| e.selected_text().to_string())
                    .filter(|t| !t.is_empty());
                if let Some(t) = text {
                    self.copy(t);
                }
                false
            }
            Event::Key(KeyInput {
                edit: Some(EditKey::Paste),
                ..
            }) if fd.dlg.focused_edit().is_some() => {
                self.apply(
                    win,
                    Reply {
                        paste: true,
                        ..Reply::none()
                    },
                );
                true
            }
            Event::Pasted(text) => {
                if fd.dlg.paste(text)
                    && let Some(s) = self.slot_mut(win)
                {
                    s.dirty = true;
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn copy(&mut self, text: String) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let source =
            manager.create_copy_paste_source(&self.qh, [TEXT_MIME, "text/plain", "UTF8_STRING"]);
        source.set_selection(device, self.last_serial);
        self.copy_text = text.into_bytes().into();
        self.copy_clip = None;
        self.copy_source = Some(source);
    }

    pub(crate) fn copy_primary(&mut self, text: String) {
        self.primary.copy(&self.qh, text, self.last_serial);
    }

    pub(crate) fn paste_primary(&mut self, win: WinId) {
        if let Some(pipe) = self.primary.receive() {
            let id = self.await_paste(win);
            self.read_paste(id, pipe, PASTE_TIMEOUT, PasteBuf::default(), |buf| {
                buf.map(|b| Event::PastedPrimary(b.into_text()))
            });
        }
    }

    pub(crate) fn copy_files(&mut self, clip: FileClip) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let Some(offers) = reprisefs::fileclip::offers(&clip) else {
            return;
        };
        let source = manager.create_copy_paste_source(&self.qh, offers.iter().map(|(m, _)| *m));
        source.set_selection(device, self.last_serial);
        self.copy_clip = Some(offers);
        self.copy_text = Arc::default();
        self.copy_source = Some(source);
    }

    pub(crate) fn paste(&mut self, win: WinId) {
        let Some(offer) = self.selection_offer() else {
            return;
        };
        let Some(mime) = offer.with_mime_types(text_mime) else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime.to_string()) else {
            return;
        };
        let id = self.await_paste(win);
        self.read_paste(
            id,
            read_pipe,
            PASTE_TIMEOUT,
            PasteBuf::default(),
            move |buf| buf.map(|b| Event::Pasted(pasted_text(mime, b))),
        );
    }

    /// Read the clipboard's files for `win`: a cut marked in [`reprisefs::fileclip::KDE_CUT`]
    /// first, then the files.
    pub(crate) fn paste_files(&mut self, win: WinId) {
        use reprisefs::fileclip;
        let Some(offer) = self.selection_offer() else {
            return;
        };
        let Some(source) = offer.with_mime_types(fileclip::source) else {
            return;
        };
        let sandboxed = offer.with_mime_types(reprisefs::dropdata::sandboxed);
        if !source.kde_cut {
            return self.paste_files_as(win, None, source.files, false, sandboxed);
        }
        let Ok(read_pipe) = offer.receive(fileclip::KDE_CUT.to_string()) else {
            return;
        };
        let id = self.await_paste(win);
        let asked = Instant::now();
        let reading = read_capped(
            &self.loop_handle,
            read_pipe,
            PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                let cut = buf.is_some_and(|b| fileclip::is_cut_marker(b.bytes()));
                host.paste_files_as(win, Some((id, asked)), source.files, cut, sandboxed);
            },
        );
        if !reading {
            self.paste_wait.land(id, None);
        }
    }

    /// The files read as `mime`, for a new paste, or for paste `id` asked at `asked` and out
    /// since (a cut marker read first), whose read shares [`PASTE_TIMEOUT`] with the marker's.
    fn paste_files_as(
        &mut self,
        win: WinId,
        out: Option<(PasteId, Instant)>,
        mime: &'static str,
        kde_cut: bool,
        sandboxed: bool,
    ) {
        let pipe = (self.selection_offer()).and_then(|o| o.receive(mime.to_string()).ok());
        let Some(pipe) = pipe else {
            if let Some((id, _)) = out {
                self.paste_landed(id, None);
            }
            return;
        };
        let (id, timeout) = match out {
            Some((id, asked)) => (id, PASTE_TIMEOUT.saturating_sub(asked.elapsed())),
            None => (self.await_paste(win), PASTE_TIMEOUT),
        };
        self.read_paste(id, pipe, timeout, PasteBuf::default(), move |buf| {
            buf.map(|b| Event::PastedFiles(pasted_files(mime, b.bytes(), kde_cut, sandboxed)))
        });
    }

    pub(crate) fn copy_image(&mut self, png: Arc<Vec<u8>>) {
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let source = manager.create_copy_paste_source(&self.qh, [PNG_MIME]);
        source.set_selection(device, self.last_serial);
        self.copy_clip = Some(vec![(PNG_MIME, png.to_vec())]);
        self.copy_text = Arc::default();
        self.copy_source = Some(source);
    }

    /// Read the clipboard's picture for `win`; an empty `Event::PastedImage` when there is none
    /// or it is past [`MAX_IMAGE_BYTES`].
    pub(crate) fn paste_image(&mut self, win: WinId) {
        let pipe = self.selection_offer().and_then(|offer| {
            let mime = offer.with_mime_types(image_mime)?;
            offer.receive(mime.to_string()).ok()
        });
        let Some(pipe) = pipe else {
            self.send(win, Event::PastedImage(Vec::new()));
            return;
        };
        let id = self.await_paste(win);
        let buf = PasteBuf::capped(MAX_IMAGE_BYTES);
        self.read_paste(id, pipe, PASTE_TIMEOUT, buf, |buf| {
            Some(Event::PastedImage(
                buf.map(|b| b.bytes().to_vec()).unwrap_or_default(),
            ))
        });
    }

    pub(crate) fn selection_offer(&self) -> Option<SelectionOffer> {
        self.data_device.as_ref()?.data().selection_offer()
    }

    /// A paste for `win` is out: input waits on it until it lands, or for [`PASTE_HOLD`].
    fn await_paste(&mut self, win: WinId) -> PasteId {
        let id = self.paste_wait.begin(win);
        let timer = Timer::from_duration(PASTE_HOLD);
        let _ = self
            .loop_handle
            .insert_source(timer, move |_, _, host: &mut Host| {
                if host.paste_wait.release(id) {
                    log::info!("paste: no answer in {PASTE_HOLD:?}; input goes on");
                    host.replay_input();
                }
                TimeoutAction::Drop
            });
        id
    }

    /// Read paste `id` from `pipe` into `buf` for up to `timeout`; `make_event` turns what came
    /// (None when the read failed) into what its window gets. A pipe the loop refuses lands as
    /// nothing; the input it held goes on at the turn's end.
    fn read_paste(
        &mut self,
        id: PasteId,
        pipe: ReadPipe,
        timeout: Duration,
        buf: PasteBuf,
        make_event: impl FnOnce(Option<PasteBuf>) -> Option<Event> + 'static,
    ) {
        let reading = read_capped_in(
            &self.loop_handle,
            pipe,
            timeout,
            buf,
            move |host: &mut Host, buf| host.paste_landed(id, make_event(buf)),
        );
        if !reading {
            self.paste_wait.land(id, None);
        }
    }

    /// Paste `id`'s answer `ev` came: it goes to its window, then the input that waited on it.
    fn paste_landed(&mut self, id: PasteId, ev: Option<Event>) {
        self.paste_wait.land(id, ev);
        self.replay_input();
    }

    /// The pastes landed and the input that waited go on, up to one that asks for another paste.
    pub(crate) fn replay_input(&mut self) {
        while let Some((win, held)) = self.paste_wait.pop() {
            match held {
                Held::Event(ev) => self.send(win, ev),
                Held::Reader(req) => self.reader_request(win, req, false),
            }
        }
    }

    /// At a turn's end: drops the pastes out for windows that closed, then hands on what no paste
    /// holds any longer.
    pub(crate) fn settle_pastes(&mut self) {
        let open = |w: WinId| w == MAIN || self.slots.iter().any(|s| s.id == w);
        if self.paste_wait.close(open) {
            log::info!("paste: its window closed; dropped");
        }
        self.replay_input();
    }
}

/// Read `pipe` on the loop until its writer closes it, then hand the
/// bytes to `done`. The writer is any client: an offer past the cap, or
/// one still open after `timeout` (written to or not), is dropped and
/// `done` gets None. `done` runs once; false when the loop refuses the pipe and it never runs.
pub(crate) fn read_capped<D: 'static>(
    handle: &LoopHandle<'static, D>,
    pipe: ReadPipe,
    timeout: Duration,
    done: impl FnOnce(&mut D, Option<PasteBuf>) + 'static,
) -> bool {
    read_capped_in(handle, pipe, timeout, PasteBuf::default(), done)
}

/// [`read_capped`] into `buf`, whose cap holds.
fn read_capped_in<D: 'static>(
    handle: &LoopHandle<'static, D>,
    pipe: ReadPipe,
    timeout: Duration,
    mut buf: PasteBuf,
    done: impl FnOnce(&mut D, Option<PasteBuf>) + 'static,
) -> bool {
    type Done<D> = Rc<RefCell<Option<Box<dyn FnOnce(&mut D, Option<PasteBuf>)>>>>;
    let done: Done<D> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = |done: &Done<D>, data: &mut D, buf: Option<PasteBuf>| {
        let f = done.borrow_mut().take();
        if let Some(f) = f {
            f(data, buf);
        }
    };
    let on_read = done.clone();
    let token = handle.insert_source(pipe, move |_, pipe, data: &mut D| {
        let mut chunk = Zeroizing::new([0u8; 4096]);
        match (&**pipe).read(&mut chunk[..]) {
            Ok(0) | Err(_) => {
                finish(&on_read, data, Some(std::mem::take(&mut buf)));
                PostAction::Remove
            }
            Ok(n) if buf.push(&chunk[..n]) => PostAction::Continue,
            Ok(_) => {
                log::warn!("paste: offer too large; dropped");
                finish(&on_read, data, None);
                PostAction::Remove
            }
        }
    });
    let Ok(token) = token else {
        return false;
    };
    let remover = handle.clone();
    let _ = handle.insert_source(Timer::from_duration(timeout), move |_, _, data| {
        remover.remove(token);
        finish(&done, data, None);
        TimeoutAction::Drop
    });
    true
}

const PNG_MIME: &str = "image/png";

/// The largest picture a paste reads.
const MAX_IMAGE_BYTES: usize = 256 << 20;

/// The picture types an image paste reads, best first: PNG, then the others the decoders read.
const IMAGE_MIMES: [&str; 9] = [
    PNG_MIME,
    "image/bmp",
    "image/x-bmp",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/tiff",
    "image/x-portable-pixmap",
    "image/x-icon",
];

/// The type an image paste reads out of a selection offering `types`, if it offers a picture.
pub(crate) fn image_mime(types: &[String]) -> Option<&'static str> {
    IMAGE_MIMES
        .into_iter()
        .find(|m| types.iter().any(|t| t == m))
}

/// The text types a paste reads, best first; `STRING` is X11's Latin-1 one.
const PASTE_MIMES: [&str; 5] = [TEXT_MIME, "text/plain", "UTF8_STRING", "TEXT", "STRING"];

/// `buf` read as type `mime` of [`PASTE_MIMES`].
pub(crate) fn pasted_text(mime: &str, buf: PasteBuf) -> String {
    if mime == "STRING" {
        buf.into_latin1_text()
    } else {
        buf.into_text()
    }
}

/// The type a paste reads out of a selection offering `types`, if it offers text.
pub(crate) fn text_mime(types: &[String]) -> Option<&'static str> {
    PASTE_MIMES
        .into_iter()
        .find(|m| types.iter().any(|t| t == m))
}

thread_local! {
    /// Set on the event loop's thread before each `App::event`.
    pub(crate) static CLIPBOARD_TEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set alongside `CLIPBOARD_TEXT`.
    pub(crate) static CLIPBOARD_FILES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set alongside `CLIPBOARD_TEXT`.
    pub(crate) static CLIPBOARD_IMAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the clipboard holds text a paste ([`Reply::paste`]) would read, as of the event being
/// handled; false before a window first had the keyboard.
pub fn clipboard_has_text() -> bool {
    CLIPBOARD_TEXT.get()
}

/// Stands in for the compositor's offer in a program's tests, on the calling thread.
#[doc(hidden)]
pub fn fake_clipboard_text(on: bool) {
    CLIPBOARD_TEXT.set(on);
}

/// Whether the clipboard holds files a [`Reply::paste_files`] would read, as of the event being
/// handled; false before a window first had the keyboard.
pub fn clipboard_has_files() -> bool {
    CLIPBOARD_FILES.get()
}

/// Stands in for the compositor's offer in a program's tests, on the calling thread.
#[doc(hidden)]
pub fn fake_clipboard_files(on: bool) {
    CLIPBOARD_FILES.set(on);
}

/// Whether the clipboard holds a picture a [`Reply::paste_image`] would read, as of the event
/// being handled; false before a window first had the keyboard.
pub fn clipboard_has_image() -> bool {
    CLIPBOARD_IMAGE.get()
}

/// Stands in for the compositor's offer in a program's tests, on the calling thread.
#[doc(hidden)]
pub fn fake_clipboard_image(on: bool) {
    CLIPBOARD_IMAGE.set(on);
}

/// The files a paste of `bytes`, read as `mime`, lands. A sandboxed program's
/// ([`reprisefs::dropdata::SANDBOXED`]) lands none unless
/// [`reprisefs::fileclip::sandboxed_list_taken`] takes it.
fn pasted_files(mime: &str, bytes: &[u8], kde_cut: bool, sandboxed: bool) -> FileClip {
    let mut clip = reprisefs::fileclip::decode(mime, bytes);
    clip.cut |= kde_cut;
    if sandboxed && !reprisefs::fileclip::sandboxed_list_taken(&clip) {
        log::info!("paste: a sandboxed program's file list is refused");
        return FileClip::default();
    }
    clip
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandboxed_programs_clip_pastes_only_a_copy_of_document_portal_files() {
        use reprisefs::dropdata::{GNOME_FILES, URI_LIST, document_root};
        let doc = document_root().join("abc/x.png");
        let doc_uri = format!("file://{}", doc.display());
        let key = "file:///home/u/.ssh/id_ed25519";
        let copy = format!("copy\n{doc_uri}");
        let got = pasted_files(GNOME_FILES, copy.as_bytes(), false, true);
        assert_eq!(got.paths, [doc], "a copy of a portal file");
        assert!(!got.cut);
        for (mime, body, kde_cut, why) in [
            (
                GNOME_FILES,
                format!("copy\n{key}"),
                false,
                "a path in the sandbox",
            ),
            (
                GNOME_FILES,
                format!("copy\n{doc_uri}\n{key}"),
                false,
                "one path outside",
            ),
            (GNOME_FILES, format!("cut\n{doc_uri}"), false, "a cut"),
            (
                URI_LIST,
                format!("{doc_uri}\r\n"),
                true,
                "a cut marked beside a URI list",
            ),
        ] {
            let got = pasted_files(mime, body.as_bytes(), kde_cut, true);
            assert_eq!(got, FileClip::default(), "{why}");
        }
        let host = pasted_files(GNOME_FILES, format!("cut\n{key}").as_bytes(), false, false);
        assert!(host.cut && !host.paths.is_empty(), "a host program's cut");
    }

    fn types(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_image_paste_takes_png_first_and_skips_text() {
        let both = types(&["image/jpeg", TEXT_MIME, PNG_MIME]);
        assert_eq!(image_mime(&both), Some(PNG_MIME));
        assert_eq!(
            image_mime(&types(&["text/html", "image/bmp"])),
            Some("image/bmp")
        );
        assert_eq!(image_mime(&types(&[TEXT_MIME, "image/svg+xml"])), None);
    }
}
