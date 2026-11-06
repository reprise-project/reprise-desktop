//! Copy and paste with the client over CLIPRDR: [`Clipboard`] routes them between the client and
//! the session's clipboard (a [`Desktop`], served by `wlclip`), moving data only on a paste.

use crate::wlclip;
use image::codecs::bmp::BmpDecoder;
use image::codecs::png::PngDecoder;
use image::{DynamicImage, ImageDecoder, ImageFormat};
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

pub const TEXT_MAX: usize = 16 << 20;
pub const PICTURE_MAX: usize = 64 << 20;
/// How long the client has to answer a paste before it ends empty.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// The desktop's UTF-8 text types, best first; the client's text is offered as all of them.
pub const TEXT_MIMES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
];
/// Latin-1 text, read when a copy offers no UTF-8 type.
const LATIN1_MIME: &str = "STRING";
pub const PNG_MIME: &str = "image/png";

/// The id this server's format lists give the registered "PNG" format.
const PNG_ID: ClipboardFormatId = ClipboardFormatId(0xC0A0);
const PNG_NAME: &str = "PNG";

/// A clipboard format both sides can carry, named for its bytes on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// CF_UNICODETEXT: UTF-16LE with CRLF line ends and a NUL.
    Text,
    Dib,
    Png,
}

impl Kind {
    pub fn limit(self) -> usize {
        match self {
            Kind::Text => TEXT_MAX,
            Kind::Dib | Kind::Png => PICTURE_MAX,
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Dib | Kind::Png => "picture",
        }
    }

    /// The client's bytes for what the desktop holds as `mime`.
    pub fn to_client(self, mime: &str, desktop: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            Kind::Text if mime == LATIN1_MIME => Ok(utf16_text(
                desktop
                    .iter()
                    .map(|&b| char::from(b))
                    .collect::<String>()
                    .as_bytes(),
            )),
            Kind::Text => Ok(utf16_text(desktop)),
            Kind::Dib => png_to_dib(desktop),
            Kind::Png => Ok(desktop.to_vec()),
        }
    }

    /// The desktop's bytes for what the client sent in this kind.
    pub fn to_desktop(self, client: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            Kind::Text => Ok(utf8_text(client)),
            Kind::Dib => dib_to_png(client),
            Kind::Png => Ok(client.to_vec()),
        }
    }
}

/// Logs that `kind` from `from` was over its limit and is not passed on.
pub fn refuse(kind: Kind, from: &str) {
    log::warn!(
        "clipboard: {from}'s {} is over {} MiB; refused",
        kind.noun(),
        kind.limit() >> 20
    );
}

/// The session's clipboard as [`Clipboard`] drives it.
pub trait Desktop {
    /// A pasting program's end of the transfer; dropping it ends the paste.
    type Sink;
    /// Hold the clipboard for the client, offering `mimes`.
    fn offer(&mut self, mimes: &[&'static str]);
    /// Give up the client's selection if it still holds the clipboard.
    fn clear(&mut self);
    /// Read the desktop's copy as `mime` and convert it for the client as `kind`; the bytes come
    /// back through [`Clipboard::desktop_read`].
    fn read(&mut self, mime: &str, kind: Kind);
    /// Convert the client's `bytes` of `kind` and write them to `sink`; `None` ends the paste
    /// empty.
    fn deliver(&mut self, sink: Self::Sink, kind: Kind, bytes: Option<Vec<u8>>);
}

/// Where the client's latest copy keeps text and pictures.
#[derive(Clone, Copy, Debug, Default)]
struct ClientFormats {
    text: Option<ClipboardFormatId>,
    dib: Option<ClipboardFormatId>,
    png: Option<ClipboardFormatId>,
}

impl ClientFormats {
    fn of(formats: &[ClipboardFormat]) -> ClientFormats {
        let has = |id| formats.iter().any(|f| f.id == id).then_some(id);
        let png = formats
            .iter()
            .find(|f| f.name.as_ref().is_some_and(|n| n.value() == PNG_NAME))
            .map(|f| f.id);
        ClientFormats {
            text: has(ClipboardFormatId::CF_UNICODETEXT),
            dib: has(ClipboardFormatId::CF_DIB),
            png,
        }
    }

    fn mimes(self) -> Vec<&'static str> {
        let mut mimes = Vec::new();
        if self.text.is_some() {
            mimes.extend(TEXT_MIMES);
        }
        if self.dib.is_some() || self.png.is_some() {
            mimes.push(PNG_MIME);
        }
        mimes
    }

    /// The format to ask the client for and its kind, for a program pasting `mime`.
    fn for_mime(self, mime: &str) -> Option<(ClipboardFormatId, Kind)> {
        if TEXT_MIMES.contains(&mime) {
            return self.text.map(|id| (id, Kind::Text));
        }
        if mime != PNG_MIME {
            return None;
        }
        (self.png.map(|id| (id, Kind::Png))).or(self.dib.map(|id| (id, Kind::Dib)))
    }
}

/// The formats the client is offered for a desktop copy of `mimes`.
fn formats_for(mimes: &[String]) -> Vec<ClipboardFormat> {
    let mut formats = Vec::new();
    if text_mime(mimes).is_some() {
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
    }
    if mimes.iter().any(|m| m == PNG_MIME) {
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        formats.push(
            ClipboardFormat::new(PNG_ID).with_name(ClipboardFormatName::new_static(PNG_NAME)),
        );
    }
    formats
}

fn text_mime(mimes: &[String]) -> Option<&'static str> {
    (TEXT_MIMES.into_iter().chain([LATIN1_MIME])).find(|t| mimes.iter().any(|m| m == t))
}

/// A program's paste waiting on the client. The first in the queue has its request out.
struct Paste<S> {
    /// `None` once the client copied again; its answer is then dropped.
    sink: Option<S>,
    kind: Kind,
    format: ClipboardFormatId,
}

/// One connection's copy and paste. CLIPRDR carries one data request each way at a time, so
/// both directions queue.
pub struct Clipboard<S> {
    rdp: UnboundedSender<ServerEvent>,
    client: ClientFormats,
    client_owns: bool,
    /// The types of the desktop's copy the client was last offered.
    desktop: Vec<String>,
    /// The client's requests; the first is being read.
    asked: VecDeque<Option<(String, Kind)>>,
    reading: bool,
    pastes: VecDeque<Paste<S>>,
    /// When the client's answer to the first paste is due, while its request is out.
    deadline: Option<Instant>,
    /// Until when the answer to a request that timed out may still come. Nothing is sent
    /// meanwhile, so an answer then is that one, and is dropped.
    late: Option<Instant>,
    /// The limit of the request out, 0 when none is; the CLIPRDR backend drops a larger answer
    /// without copying it. Set before the request is sent.
    answer_limit: Arc<AtomicUsize>,
}

impl<S> Clipboard<S> {
    pub fn new(rdp: UnboundedSender<ServerEvent>, answer_limit: Arc<AtomicUsize>) -> Clipboard<S> {
        Clipboard {
            rdp,
            client: ClientFormats::default(),
            client_owns: false,
            desktop: Vec::new(),
            asked: VecDeque::new(),
            reading: false,
            pastes: VecDeque::new(),
            deadline: None,
            late: None,
            answer_limit,
        }
    }

    fn send(&self, message: ClipboardMessage) {
        let _ = self.rdp.send(ServerEvent::Clipboard(message));
    }

    /// The desktop's clipboard changed to a copy offering `mimes` (none when it was cleared).
    pub fn desktop_copied(&mut self, mimes: Vec<String>) {
        self.client_owns = false;
        self.send(ClipboardMessage::SendInitiateCopy(formats_for(&mimes)));
        self.desktop = mimes;
    }

    /// The client copied something offered as `formats`.
    pub fn client_copied(&mut self, formats: &[ClipboardFormat], d: &mut impl Desktop<Sink = S>) {
        log::debug!("clipboard: the client offers {formats:?}");
        let mut pastes = std::mem::take(&mut self.pastes).into_iter();
        if self.deadline.is_some()
            && let Some(mut asked) = pastes.next()
        {
            if let Some(sink) = asked.sink.take() {
                d.deliver(sink, asked.kind, None);
            }
            self.pastes.push_back(asked);
        }
        for p in pastes {
            if let Some(sink) = p.sink {
                d.deliver(sink, p.kind, None);
            }
        }
        self.client = ClientFormats::of(formats);
        let mimes = self.client.mimes();
        if !mimes.is_empty() {
            d.offer(&mimes);
            self.client_owns = true;
            self.desktop.clear();
        } else if self.client_owns {
            d.clear();
            self.client_owns = false;
        }
    }

    /// The client pastes `format`.
    pub fn client_asks(&mut self, format: ClipboardFormatId, d: &mut impl Desktop<Sink = S>) {
        let kind = match format {
            ClipboardFormatId::CF_UNICODETEXT => Some(Kind::Text),
            ClipboardFormatId::CF_DIB => Some(Kind::Dib),
            PNG_ID => Some(Kind::Png),
            _ => None,
        };
        let mime = kind.and_then(|k| match k {
            Kind::Text => text_mime(&self.desktop),
            Kind::Dib | Kind::Png => self
                .desktop
                .iter()
                .any(|m| m == PNG_MIME)
                .then_some(PNG_MIME),
        });
        self.asked
            .push_back(kind.zip(mime).map(|(k, m)| (m.to_string(), k)));
        self.read_next(d);
    }

    /// What the desktop's copy held for the client's first request, converted, or `None`.
    pub fn desktop_read(&mut self, bytes: Option<Vec<u8>>, d: &mut impl Desktop<Sink = S>) {
        if !self.reading {
            return;
        }
        self.reading = false;
        self.asked.pop_front();
        self.answer(bytes);
        self.read_next(d);
    }

    fn answer(&self, bytes: Option<Vec<u8>>) {
        let response = match bytes {
            Some(bytes) => OwnedFormatDataResponse::new_data(bytes),
            None => OwnedFormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn read_next(&mut self, d: &mut impl Desktop<Sink = S>) {
        while !self.reading
            && let Some(first) = self.asked.front()
        {
            match first {
                Some((mime, kind)) => {
                    d.read(mime, *kind);
                    self.reading = true;
                }
                None => {
                    self.asked.pop_front();
                    self.answer(None);
                }
            }
        }
    }

    /// A program pastes the client's copy as `mime` into `sink`.
    pub fn desktop_pastes(&mut self, mime: &str, sink: S, d: &mut impl Desktop<Sink = S>) {
        let Some((format, kind)) = self.client.for_mime(mime) else {
            d.deliver(sink, Kind::Text, None);
            return;
        };
        self.pastes.push_back(Paste {
            sink: Some(sink),
            kind,
            format,
        });
        self.request_next();
    }

    /// The client's answer to the oldest request still out: its bytes, or `None` when it had
    /// none or the backend dropped it.
    pub fn client_sent(&mut self, bytes: Option<Vec<u8>>, d: &mut impl Desktop<Sink = S>) {
        if self.late.take().is_some() {
            self.request_next();
            return;
        }
        if self.deadline.take().is_none() {
            return;
        }
        if let Some(paste) = self.pastes.pop_front()
            && let Some(sink) = paste.sink
        {
            let bytes = bytes.filter(|b| {
                let fits = b.len() <= paste.kind.limit();
                if !fits {
                    refuse(paste.kind, "the client");
                }
                fits
            });
            d.deliver(sink, paste.kind, bytes);
        }
        self.request_next();
    }

    fn request_next(&mut self) {
        if self.deadline.is_some() || self.late.is_some() {
            return;
        }
        let Some(first) = self.pastes.front() else {
            self.answer_limit.store(0, Ordering::Release);
            return;
        };
        self.answer_limit
            .store(first.kind.limit(), Ordering::Release);
        self.deadline = Some(Instant::now() + ANSWER_TIMEOUT);
        self.send(ClipboardMessage::SendInitiatePaste(first.format));
    }

    /// When [`Clipboard::expire`] has something to do: an answer due, or the wait for a late
    /// one ending.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline.or(self.late)
    }

    /// Past the deadline the first paste ends empty, and the next request waits up to another
    /// [`ANSWER_TIMEOUT`] for the late answer, which is dropped.
    pub fn expire(&mut self, now: Instant, d: &mut impl Desktop<Sink = S>) {
        if self.late.is_some_and(|until| now >= until) {
            self.late = None;
            self.request_next();
            return;
        }
        if self.deadline.is_none_or(|due| now < due) {
            return;
        }
        self.deadline = None;
        self.late = Some(now + ANSWER_TIMEOUT);
        self.answer_limit.store(0, Ordering::Release);
        log::warn!(
            "clipboard: the client did not answer a paste within {} s",
            ANSWER_TIMEOUT.as_secs()
        );
        if let Some(paste) = self.pastes.pop_front()
            && let Some(sink) = paste.sink
        {
            d.deliver(sink, paste.kind, None);
        }
    }

    /// The client left: its selection goes and every waiting paste ends empty.
    pub fn stop(&mut self, d: &mut impl Desktop<Sink = S>) {
        if self.client_owns {
            d.clear();
            self.client_owns = false;
        }
        for p in std::mem::take(&mut self.pastes) {
            if let Some(sink) = p.sink {
                d.deliver(sink, p.kind, None);
            }
        }
        self.asked.clear();
        self.deadline = None;
        self.late = None;
    }
}

/// CF_UNICODETEXT for UTF-8 `text`.
fn utf16_text(text: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(text)
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    (text.encode_utf16().chain([0]))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// UTF-8 with LF line ends for CF_UNICODETEXT `data`, up to its NUL.
fn utf8_text(data: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = (data.as_chunks::<2>().0.iter())
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
        .replace("\r\n", "\n")
        .into_bytes()
}

fn fits(w: u32, h: u32) -> Result<(), String> {
    if u64::from(w) * u64::from(h) * 4 > PICTURE_MAX as u64 {
        return Err(format!(
            "a {w}x{h} picture is over {} MiB",
            PICTURE_MAX >> 20
        ));
    }
    Ok(())
}

fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, String> {
    let decoder =
        BmpDecoder::new_without_file_header(Cursor::new(dib)).map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    fits(w, h)?;
    let picture = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let mut png = Vec::new();
    (picture.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)).map_err(|e| e.to_string())?;
    Ok(png)
}

const BITMAPINFOHEADER: u32 = 40;
/// 72 dpi.
const PIXELS_PER_METRE: i32 = 2835;

/// A bottom-up 32-bit BI_RGB DIB, alpha in the fourth byte.
fn png_to_dib(png: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = PngDecoder::new(Cursor::new(png)).map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    fits(w, h)?;
    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let size = w * h * 4;
    let mut dib = Vec::with_capacity((BITMAPINFOHEADER + size) as usize);
    dib.extend(BITMAPINFOHEADER.to_le_bytes());
    dib.extend((w as i32).to_le_bytes());
    dib.extend((h as i32).to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(0u32.to_le_bytes());
    dib.extend(size.to_le_bytes());
    dib.extend(PIXELS_PER_METRE.to_le_bytes());
    dib.extend(PIXELS_PER_METRE.to_le_bytes());
    dib.extend([0u8; 8]);
    for row in rgba.rows().rev() {
        for p in row {
            dib.extend([p[2], p[1], p[0], p[3]]);
        }
    }
    Ok(dib)
}

/// The client's answer's bytes when it carries some within `limit`; checked before copying.
fn take_answer(response: &FormatDataResponse<'_>, limit: usize) -> Option<Vec<u8>> {
    if response.is_error() {
        return None;
    }
    let len = response.data().len();
    if len > limit {
        if limit > 0 {
            log::warn!(
                "clipboard: the client's answer is over {} MiB; refused",
                limit >> 20
            );
        }
        return None;
    }
    Some(response.data().to_vec())
}

/// Gives each connection's server a [`Backend`] that starts the desktop side once the client's
/// clipboard channel is up; nothing crosses until the client is shown the session.
pub struct Factory {
    events: Option<UnboundedSender<ServerEvent>>,
    shown: watch::Receiver<bool>,
}

impl Factory {
    /// `shown` is the connection's [`crate::listen::Shown`].
    pub fn new(shown: watch::Receiver<bool>) -> Factory {
        Factory {
            events: None,
            shown,
        }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.events = Some(sender);
    }
}

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend {
            events: self.events.clone(),
            shown: self.shown.clone(),
            desktop: None,
        })
    }
}

impl CliprdrServerFactory for Factory {}

/// Passes the client's side of CLIPRDR to `wlclip`'s thread, which stops when this drops.
#[derive(Debug)]
struct Backend {
    events: Option<UnboundedSender<ServerEvent>>,
    shown: watch::Receiver<bool>,
    desktop: Option<wlclip::Link>,
}

ironrdp_core::impl_as_any!(Backend);

impl Backend {
    fn tell(&self, message: wlclip::Msg) {
        if let Some(d) = &self.desktop {
            d.send(message);
        }
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        if self.desktop.is_none()
            && let Some(events) = &self.events
        {
            self.desktop = Some(wlclip::start(events.clone(), self.shown.clone()));
        }
    }

    fn on_request_format_list(&mut self) {}

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        self.tell(wlclip::Msg::ClientCopied(formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.tell(wlclip::Msg::ClientAsks(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if let Some(d) = &self.desktop {
            d.send(wlclip::Msg::ClientSent(take_answer(
                &response,
                d.answer_limit(),
            )));
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[derive(Default)]
    struct Fake {
        offered: Vec<Vec<&'static str>>,
        cleared: usize,
        reads: Vec<(String, Kind)>,
        delivered: Vec<(u32, Kind, Option<Vec<u8>>)>,
    }

    impl Desktop for Fake {
        type Sink = u32;
        fn offer(&mut self, mimes: &[&'static str]) {
            self.offered.push(mimes.to_vec());
        }
        fn clear(&mut self) {
            self.cleared += 1;
        }
        fn read(&mut self, mime: &str, kind: Kind) {
            self.reads.push((mime.into(), kind));
        }
        fn deliver(&mut self, sink: u32, kind: Kind, bytes: Option<Vec<u8>>) {
            self.delivered.push((sink, kind, bytes));
        }
    }

    fn session() -> (Clipboard<u32>, Fake, mpsc::UnboundedReceiver<ServerEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Clipboard::new(tx, Arc::default()), Fake::default(), rx)
    }

    fn sent(rx: &mut mpsc::UnboundedReceiver<ServerEvent>) -> Vec<ClipboardMessage> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|e| match e {
                ServerEvent::Clipboard(m) => m,
                other => panic!("only clipboard events are sent, got {other:?}"),
            })
            .collect()
    }

    fn mimes(m: &[&str]) -> Vec<String> {
        m.iter().map(|s| s.to_string()).collect()
    }

    fn ids(formats: &[ClipboardFormat]) -> Vec<(u32, Option<String>)> {
        (formats.iter())
            .map(|f| (f.id.0, f.name.as_ref().map(|n| n.value().to_string())))
            .collect()
    }

    fn client_text() -> Vec<ClipboardFormat> {
        vec![
            ClipboardFormat::new(ClipboardFormatId::CF_TEXT),
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]
    }

    #[test]
    fn utf16_text_round_trips() {
        let text = "a line\nG clef \u{1D11E}, smile \u{1F600}\r\nend";
        let wire = utf16_text(text.as_bytes());
        let units: Vec<u16> = (wire.as_chunks::<2>().0.iter())
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        let expected: Vec<u16> = "a line\r\nG clef \u{1D11E}, smile \u{1F600}\r\nend"
            .encode_utf16()
            .chain([0])
            .collect();
        assert_eq!(
            units, expected,
            "LF becomes CRLF, surrogate pairs, a closing NUL"
        );
        assert_eq!(
            utf8_text(&wire),
            "a line\nG clef \u{1D11E}, smile \u{1F600}\nend".as_bytes(),
            "CRLF comes back as LF"
        );
        assert_eq!(
            Kind::Text.to_client("STRING", b"caf\xe9").unwrap(),
            utf16_text("café".as_bytes()),
            "STRING is Latin-1"
        );
        let mut trailing = utf16_text(b"x");
        trailing.extend([b'y', 0, 0xFF]);
        assert_eq!(utf8_text(&trailing), b"x", "text ends at the NUL");
    }

    /// A 3x2 DIB: top row red, green, blue; bottom row white, black, grey. A 32 bpp row's
    /// reserved bytes are 0x40.
    fn dib(bpp: u16, top_down: bool) -> Vec<u8> {
        let rows: [[[u8; 3]; 3]; 2] = [
            [[255, 0, 0], [0, 255, 0], [0, 0, 255]],
            [[255, 255, 255], [0, 0, 0], [128, 128, 128]],
        ];
        let stride = (3 * usize::from(bpp) / 8).next_multiple_of(4);
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(3i32.to_le_bytes());
        d.extend((if top_down { -2i32 } else { 2 }).to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(bpp.to_le_bytes());
        d.extend([0u8; 24]);
        let order: Vec<usize> = if top_down { vec![0, 1] } else { vec![1, 0] };
        for r in order {
            let start = d.len();
            for [red, green, blue] in rows[r] {
                d.extend([blue, green, red]);
                if bpp == 32 {
                    d.push(0x40);
                }
            }
            d.resize(start + stride, 0);
        }
        d
    }

    fn rgba_of_png(png: &[u8]) -> Vec<u8> {
        image::load_from_memory_with_format(png, ImageFormat::Png)
            .unwrap()
            .into_rgba8()
            .into_raw()
    }

    #[test]
    fn a_dib_becomes_png_and_back() {
        let rgb = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 255],
            [0, 0, 0],
            [128, 128, 128],
        ];
        let opaque: Vec<u8> = rgb.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect();
        let bgra_bottom_up: Vec<u8> = (rgb.chunks(3).rev().flatten())
            .flat_map(|&[r, g, b]| [b, g, r, 255])
            .collect();
        for (bpp, top_down) in [(24, false), (24, true), (32, false), (32, true)] {
            let png = Kind::Dib.to_desktop(&dib(bpp, top_down)).unwrap();
            assert_eq!(
                rgba_of_png(&png),
                opaque,
                "{bpp} bpp, top-down {top_down}: the PNG holds the DIB's pixels, top row first, \
                 opaque whatever a 32 bpp DIB's reserved byte holds"
            );
            let back = Kind::Dib.to_client(PNG_MIME, &png).unwrap();
            assert_eq!(
                &back[..16],
                &[40, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 32, 0]
            );
            assert_eq!(
                back[40..],
                bgra_bottom_up,
                "{bpp} bpp, top-down {top_down}: and back, bottom row first"
            );
        }

        let mut translucent = Vec::new();
        let mut enc = png::Encoder::new(&mut translucent, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[10, 20, 30, 0x80]).unwrap();
        w.finish().unwrap();
        assert_eq!(
            Kind::Dib.to_client(PNG_MIME, &translucent).unwrap()[40..],
            [30, 20, 10, 0x80],
            "a PNG's alpha goes in the DIB's fourth byte"
        );
    }

    #[test]
    fn a_dib_too_big_for_its_body_is_refused_before_decoding() {
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(5000i32.to_le_bytes());
        d.extend(5000i32.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(32u16.to_le_bytes());
        d.extend([0u8; 24]);
        d.extend([0u8; 16]);
        let e = Kind::Dib.to_desktop(&d).unwrap_err();
        assert_eq!(
            e, "a 5000x5000 picture is over 64 MiB",
            "the size check comes before the pixels are allocated"
        );
    }

    #[test]
    fn formats_offered_follow_the_wayland_offer() {
        let (mut c, _, mut rx) = session();
        c.desktop_copied(mimes(&["text/html", "UTF8_STRING", "image/png"]));
        c.desktop_copied(mimes(&["STRING"]));
        c.desktop_copied(mimes(&["image/jpeg", "text/uri-list"]));
        let lists: Vec<_> = (sent(&mut rx).into_iter())
            .map(|m| match m {
                ClipboardMessage::SendInitiateCopy(f) => ids(&f),
                other => panic!("a format list, got {other:?}"),
            })
            .collect();
        assert_eq!(
            lists,
            [
                vec![(13, None), (8, None), (0xC0A0, Some("PNG".to_string()))],
                vec![(13, None)],
                vec![],
            ]
        );
    }

    #[test]
    fn nothing_is_read_until_the_client_asks() {
        let (mut c, mut d, mut rx) = session();
        c.desktop_copied(mimes(&[
            "text/plain",
            "text/plain;charset=utf-8",
            "image/png",
        ]));
        sent(&mut rx);
        assert!(d.reads.is_empty(), "a desktop copy reads nothing");
        c.client_asks(ClipboardFormatId::CF_UNICODETEXT, &mut d);
        c.client_asks(ClipboardFormatId::CF_DIB, &mut d);
        assert_eq!(
            d.reads,
            [("text/plain;charset=utf-8".into(), Kind::Text)],
            "one read at a time, the best text type"
        );
        c.desktop_read(Some(b"t".to_vec()), &mut d);
        assert_eq!(d.reads[1], ("image/png".into(), Kind::Dib));
        c.desktop_read(None, &mut d);
        let answers: Vec<_> = (sent(&mut rx).into_iter())
            .map(|m| match m {
                ClipboardMessage::SendFormatData(r) => (r.is_error(), r.data().to_vec()),
                other => panic!("an answer, got {other:?}"),
            })
            .collect();
        assert_eq!(answers, [(false, b"t".to_vec()), (true, vec![])]);

        let (mut c, mut d, mut rx) = session();
        let mut formats = client_text();
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        c.client_copied(&formats, &mut d);
        assert_eq!(d.offered.len(), 1);
        assert!(d.offered[0].contains(&"UTF8_STRING") && d.offered[0].contains(&"image/png"));
        assert!(
            sent(&mut rx).is_empty(),
            "a client copy asks the client for nothing"
        );
        c.desktop_pastes("image/png", 7, &mut d);
        c.desktop_pastes("text/plain;charset=utf-8", 8, &mut d);
        let asked = |rx: &mut _| -> Vec<u32> {
            (sent(rx).into_iter())
                .map(|m| match m {
                    ClipboardMessage::SendInitiatePaste(f) => f.0,
                    other => panic!("a paste request, got {other:?}"),
                })
                .collect()
        };
        assert_eq!(asked(&mut rx), [8], "one request at a time");
        c.client_sent(Some(vec![1]), &mut d);
        assert_eq!(asked(&mut rx), [13]);
        assert_eq!(d.delivered, [(7, Kind::Dib, Some(vec![1]))]);
    }

    #[test]
    fn oversize_is_refused() {
        let (mut c, mut d, mut rx) = session();
        c.client_copied(&client_text(), &mut d);
        c.desktop_pastes("UTF8_STRING", 1, &mut d);
        c.desktop_pastes("TEXT", 2, &mut d);
        c.client_sent(Some(vec![b'a'; TEXT_MAX + 2]), &mut d);
        c.client_sent(Some(vec![b'a'; TEXT_MAX]), &mut d);
        sent(&mut rx);
        assert_eq!(
            d.delivered[0],
            (1, Kind::Text, None),
            "over 16 MiB ends the paste empty"
        );
        assert_eq!(d.delivered[1].2.as_ref().map(Vec::len), Some(TEXT_MAX));

        let desktop_text = |len: usize| {
            let (pipe, end) = std::io::pipe().unwrap();
            let writer = std::thread::spawn(move || {
                use std::io::Write;
                let _ = (&end).write_all(&vec![b'a'; len]);
            });
            let pipe = std::os::fd::OwnedFd::from(pipe);
            let wire = crate::wlclip::read_for_client(&pipe, "UTF8_STRING", Kind::Text);
            drop(pipe);
            writer.join().unwrap();
            wire.map(|w| w.len())
        };
        assert_eq!(
            desktop_text(TEXT_MAX / 2),
            None,
            "8 MiB of ASCII is over 16 MiB as UTF-16 with its NUL, and refused"
        );
        assert_eq!(desktop_text(TEXT_MAX / 2 - 1), Some(TEXT_MAX));

        let limit = TEXT_MAX;
        let answer = |b: &[u8]| OwnedFormatDataResponse::new_data(b.to_vec());
        assert_eq!(
            take_answer(&answer(&vec![0; limit + 1]), limit),
            None,
            "an answer over the limit of the request out is not copied"
        );
        assert_eq!(take_answer(&answer(b"ab"), limit), Some(b"ab".to_vec()));
        assert_eq!(
            take_answer(&answer(b"ab"), 0),
            None,
            "an answer with no request out is not copied"
        );

        let mut big = Vec::new();
        let mut enc = png::Encoder::new(&mut big, 4097, 4096);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&vec![0; 4096 * 4097_usize.div_ceil(8)])
            .unwrap();
        w.finish().unwrap();
        assert!(
            Kind::Dib.to_client(PNG_MIME, &big).is_err(),
            "a PNG whose DIB is over 64 MiB is refused"
        );
    }

    /// A session with a picture paste (sink 1) and a text paste (sink 2) waiting, after the
    /// first request timed out.
    fn timed_out() -> (
        Clipboard<u32>,
        Fake,
        mpsc::UnboundedReceiver<ServerEvent>,
        Arc<AtomicUsize>,
    ) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let limit = Arc::new(AtomicUsize::new(0));
        let mut c = Clipboard::new(tx, limit.clone());
        let mut d = Fake::default();
        let mut formats = client_text();
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        c.client_copied(&formats, &mut d);
        c.desktop_pastes("image/png", 1, &mut d);
        c.desktop_pastes("UTF8_STRING", 2, &mut d);
        assert_eq!(limit.load(Ordering::Acquire), PICTURE_MAX);
        let due = c.deadline().expect("a request is out");
        c.expire(due - Duration::from_millis(1), &mut d);
        assert!(d.delivered.is_empty(), "nothing ends before the deadline");
        c.expire(due, &mut d);
        assert_eq!(
            d.delivered,
            [(1, Kind::Dib, None)],
            "the stalled paste ends empty"
        );
        assert_eq!(
            asked(&mut rx),
            [8],
            "the next request waits for the late answer"
        );
        assert_eq!(
            limit.load(Ordering::Acquire),
            0,
            "a late answer is not copied"
        );
        (c, d, rx, limit)
    }

    fn asked(rx: &mut mpsc::UnboundedReceiver<ServerEvent>) -> Vec<u32> {
        (sent(rx).into_iter())
            .map(|m| match m {
                ClipboardMessage::SendInitiatePaste(f) => f.0,
                other => panic!("a paste request, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_paste_the_client_never_answers_holds_up_only_itself() {
        let (mut c, mut d, mut rx, limit) = timed_out();
        let late_until = c.deadline().expect("the wait for a late answer");
        c.expire(late_until, &mut d);
        assert_eq!(
            asked(&mut rx),
            [13],
            "the next request goes out after the wait"
        );
        assert_eq!(limit.load(Ordering::Acquire), TEXT_MAX);
        c.client_sent(Some(b"text".to_vec()), &mut d);
        assert_eq!(d.delivered[1], (2, Kind::Text, Some(b"text".to_vec())));

        c.desktop_pastes("UTF8_STRING", 3, &mut d);
        c.client_sent(Some(b"more".to_vec()), &mut d);
        assert_eq!(
            d.delivered[2],
            (3, Kind::Text, Some(b"more".to_vec())),
            "pastes after it are answered in turn"
        );
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn a_late_answer_is_dropped_and_the_next_paste_gets_its_own() {
        let (mut c, mut d, mut rx, limit) = timed_out();
        c.client_sent(None, &mut d);
        assert_eq!(d.delivered.len(), 1, "the late answer is dropped");
        assert_eq!(
            asked(&mut rx),
            [13],
            "the next request goes out once it came"
        );
        c.client_sent(Some(b"text".to_vec()), &mut d);
        assert_eq!(d.delivered[1], (2, Kind::Text, Some(b"text".to_vec())));
        assert_eq!(c.deadline(), None);
        assert_eq!(limit.load(Ordering::Acquire), 0);
    }

    #[test]
    fn disconnect_clears_the_clients_selection() {
        let (mut c, mut d, _rx) = session();
        c.client_copied(&client_text(), &mut d);
        c.desktop_pastes("text/plain", 3, &mut d);
        c.stop(&mut d);
        assert_eq!(d.cleared, 1);
        assert_eq!(d.delivered, [(3, Kind::Text, None)], "a waiting paste ends");

        let (mut c, mut d, _rx) = session();
        c.client_copied(&client_text(), &mut d);
        c.desktop_copied(mimes(&["text/plain"]));
        c.stop(&mut d);
        assert_eq!(d.cleared, 0, "a later desktop copy stays");
    }
}
