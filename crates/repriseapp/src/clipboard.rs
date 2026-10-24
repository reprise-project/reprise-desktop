//! Copy and paste: the clipboard, the primary selection and the text types a paste reads.

use crate::{Event, FileClip, Host, KeyInput, Reply, WinId};
use reprisefs::pipe::PasteBuf;
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
use std::time::Duration;
use zeroize::Zeroizing;

pub(crate) const TEXT_MIME: &str = "text/plain;charset=utf-8";

impl Host {
    /// Whether the clipboard holds text, read off the live offer as `CLIPBOARD_TEXT` is.
    pub(crate) fn clipboard_text(&self) -> bool {
        self.selection_offer()
            .is_some_and(|o| o.with_mime_types(text_mime).is_some())
    }

    /// The clipboard for a file dialog's File name
    /// ([`App::drop_file_dialog`](crate::App::drop_file_dialog)): Ctrl+C and Ctrl+X put its
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
            self.read_offer(win, pipe, |buf| Event::PastedPrimary(buf.into_text()));
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
        self.read_offer(win, read_pipe, move |buf| {
            Event::Pasted(pasted_text(mime, buf))
        });
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
        if !source.kde_cut {
            return self.paste_files_as(win, source.files, false);
        }
        let Ok(read_pipe) = offer.receive(fileclip::KDE_CUT.to_string()) else {
            return;
        };
        read_capped(
            &self.loop_handle,
            read_pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                let cut = buf.is_some_and(|b| fileclip::is_cut_marker(b.bytes()));
                host.paste_files_as(win, source.files, cut);
            },
        );
    }

    fn paste_files_as(&mut self, win: WinId, mime: &'static str, kde_cut: bool) {
        let Some(offer) = self.selection_offer() else {
            return;
        };
        let Ok(read_pipe) = offer.receive(mime.to_string()) else {
            return;
        };
        self.read_offer(win, read_pipe, move |buf| {
            let mut clip = reprisefs::fileclip::decode(mime, buf.bytes());
            clip.cut |= kde_cut;
            Event::PastedFiles(clip)
        });
    }

    pub(crate) fn selection_offer(&self) -> Option<SelectionOffer> {
        self.data_device.as_ref()?.data().selection_offer()
    }

    fn read_offer(
        &mut self,
        win: WinId,
        read_pipe: ReadPipe,
        make_event: impl FnOnce(PasteBuf) -> Event + 'static,
    ) {
        read_capped(
            &self.loop_handle,
            read_pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                if let Some(buf) = buf {
                    host.send(win, make_event(buf));
                }
            },
        );
    }
}

/// Read `pipe` on the loop until its writer closes it, then hand the
/// bytes to `done`. The writer is any client: an offer past the cap, or
/// one still open after `timeout` (written to or not), is dropped and
/// `done` gets None. `done` runs once, unless the loop refuses the pipe.
pub(crate) fn read_capped<D: 'static>(
    handle: &LoopHandle<'static, D>,
    pipe: ReadPipe,
    timeout: Duration,
    done: impl FnOnce(&mut D, Option<PasteBuf>) + 'static,
) {
    type Done<D> = Rc<RefCell<Option<Box<dyn FnOnce(&mut D, Option<PasteBuf>)>>>>;
    let done: Done<D> = Rc::new(RefCell::new(Some(Box::new(done))));
    let finish = |done: &Done<D>, data: &mut D, buf: Option<PasteBuf>| {
        let f = done.borrow_mut().take();
        if let Some(f) = f {
            f(data, buf);
        }
    };
    let mut buf = PasteBuf::default();
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
        return;
    };
    let remover = handle.clone();
    let _ = handle.insert_source(Timer::from_duration(timeout), move |_, _, data| {
        remover.remove(token);
        finish(&done, data, None);
        TimeoutAction::Drop
    });
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
