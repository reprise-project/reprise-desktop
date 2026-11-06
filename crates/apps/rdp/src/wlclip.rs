//! The session's side of copy and paste: an ext-data-control-v1 client on its own Wayland
//! connection and thread, driving a [`Clipboard`] with what the CLIPRDR backend sends it.

use crate::clipboard::{self, Clipboard, Desktop, Kind};
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, OwnedFormatDataResponse};
use ironrdp_server::ServerEvent;
use reprisefs::pipe::{PASTE_TIMEOUT, read_bounded, write_detached};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

/// What the thread hears: the client's side of CLIPRDR, and reads finished off the thread.
#[derive(Debug)]
pub enum Msg {
    ClientCopied(Vec<ClipboardFormat>),
    ClientAsks(ClipboardFormatId),
    ClientSent(Option<Vec<u8>>),
    Read(Option<Vec<u8>>),
    Stop,
}

#[derive(Clone, Debug)]
struct Mailbox {
    tx: mpsc::Sender<Msg>,
    wake: Option<Arc<UnixStream>>,
}

impl Mailbox {
    /// False once the thread is gone.
    fn send(&self, msg: Msg) -> bool {
        if self.tx.send(msg).is_err() {
            return false;
        }
        // A full socket already holds a wake-up.
        let _ = self.wake.as_deref().map(|mut w| w.write(&[1]));
        true
    }
}

/// The backend's end of the thread; dropping it stops the thread, which first gives up the
/// client's selection.
#[derive(Debug)]
pub struct Link {
    mailbox: Mailbox,
    rdp: UnboundedSender<ServerEvent>,
    answer_limit: Arc<AtomicUsize>,
}

impl Link {
    /// The most bytes the client's next answer can be used for.
    pub fn answer_limit(&self) -> usize {
        self.answer_limit.load(Ordering::Acquire)
    }

    pub fn send(&self, msg: Msg) {
        let ask = matches!(msg, Msg::ClientAsks(_));
        if !self.mailbox.send(msg) && ask {
            let _ = self
                .rdp
                .send(ServerEvent::Clipboard(ClipboardMessage::SendFormatData(
                    OwnedFormatDataResponse::new_error(),
                )));
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.mailbox.send(Msg::Stop);
    }
}

/// Start the thread for one connection; its events go to `rdp`. It reaches the session's
/// clipboard once `shown` is true.
pub fn start(rdp: UnboundedSender<ServerEvent>, shown: watch::Receiver<bool>) -> Link {
    let (tx, rx) = mpsc::channel();
    let mut link = Link {
        mailbox: Mailbox { tx, wake: None },
        rdp: rdp.clone(),
        answer_limit: Arc::default(),
    };
    let pair = UnixStream::pair().and_then(|(woken, wake)| {
        woken.set_nonblocking(true)?;
        wake.set_nonblocking(true)?;
        Ok((woken, wake))
    });
    let (woken, wake) = match pair {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("clipboard: {e}; copy and paste with the client is off");
            return link;
        }
    };
    link.mailbox.wake = Some(Arc::new(wake));
    let mailbox = link.mailbox.clone();
    let answer_limit = link.answer_limit.clone();
    let spawned = std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || run(rdp, mailbox, answer_limit, shown, &rx, &woken));
    if let Err(e) = spawned {
        log::warn!("clipboard: cannot spawn: {e}; copy and paste with the client is off");
    }
    link
}

fn run(
    rdp: UnboundedSender<ServerEvent>,
    mailbox: Mailbox,
    answer_limit: Arc<AtomicUsize>,
    shown: watch::Receiver<bool>,
    rx: &mpsc::Receiver<Msg>,
    woken: &UnixStream,
) {
    // What the client sends meanwhile waits in `rx`.
    if zbus::block_on(crate::listen::shown(shown)) {
        let served = connect(mailbox).and_then(|(conn, queue, wl)| {
            let app = App {
                model: Clipboard::new(rdp.clone(), answer_limit),
                wl,
                done: false,
                started: false,
            };
            serve(&conn, queue, app, rx, woken)
        });
        let Err(e) = served else {
            return;
        };
        log::warn!("clipboard: {e}; copy and paste with the client is off");
    }
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::ClientAsks(_) => {
                let _ = rdp.send(ServerEvent::Clipboard(ClipboardMessage::SendFormatData(
                    OwnedFormatDataResponse::new_error(),
                )));
            }
            Msg::Stop => return,
            _ => {}
        }
    }
}

fn connect(mailbox: Mailbox) -> Result<(Connection, EventQueue<App>, Wl), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland connection: {e}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let manager: ExtDataControlManagerV1 = (globals.bind(&qh, 1..=1, ()))
        .map_err(|_| "the compositor offers no ext_data_control_manager_v1".to_string())?;
    let seat: WlSeat = (globals.bind(&qh, 1..=1, ())).map_err(|_| "no seat".to_string())?;
    let device = manager.get_data_device(&seat, &qh, ());
    let wl = Wl {
        manager,
        device,
        qh,
        mailbox,
        offer: None,
        source: None,
        echoes: 0,
    };
    Ok((conn, queue, wl))
}

/// Runs until [`Msg::Stop`]; an error means the connection broke.
fn serve(
    conn: &Connection,
    mut queue: EventQueue<App>,
    mut app: App,
    rx: &mpsc::Receiver<Msg>,
    woken: &UnixStream,
) -> Result<(), String> {
    // The desktop's current selection is held back until the client's first copy has landed,
    // so a client that connects with something copied keeps it.
    queue.roundtrip(&mut app).map_err(|e| e.to_string())?;
    let mut running = handle(&mut app, rx);
    app.started = true;
    if app.wl.source.is_none() {
        let mimes = mimes_of(app.wl.offer.as_ref());
        app.model.desktop_copied(mimes);
    }
    while running {
        queue
            .dispatch_pending(&mut app)
            .map_err(|e| e.to_string())?;
        if app.done {
            return Err("the data control device finished".into());
        }
        match conn.flush() {
            Err(WaylandError::Io(e)) if e.kind() != ErrorKind::WouldBlock => {
                return Err(e.to_string());
            }
            Err(e @ WaylandError::Protocol(_)) => return Err(e.to_string()),
            _ => {}
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [
            pollin(guard.connection_fd().as_raw_fd()),
            pollin(woken.as_raw_fd()),
        ];
        let timeout = app.model.deadline().map_or(-1, |due| {
            let left = due.saturating_duration_since(Instant::now());
            i32::try_from(left.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX)
        });
        // SAFETY: `fds` is two live `pollfd`s, matching the count.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(format!("poll: {e}"));
            }
            continue;
        }
        if fds[0].revents != 0 {
            match guard.read() {
                Err(WaylandError::Io(e)) if e.kind() != ErrorKind::WouldBlock => {
                    return Err(e.to_string());
                }
                Err(e @ WaylandError::Protocol(_)) => return Err(e.to_string()),
                _ => {}
            }
        } else {
            drop(guard);
        }
        let mut drain = [0u8; 64];
        while matches!((&*woken).read(&mut drain), Ok(n) if n > 0) {}
        running = handle(&mut app, rx);
        app.model.expire(Instant::now(), &mut app.wl);
    }
    app.model.stop(&mut app.wl);
    queue.roundtrip(&mut app).map_err(|e| e.to_string())?;
    Ok(())
}

fn pollin(fd: i32) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

/// Passes waiting messages to the model; false on [`Msg::Stop`].
fn handle(app: &mut App, rx: &mpsc::Receiver<Msg>) -> bool {
    loop {
        let msg = match rx.try_recv() {
            Ok(msg) => msg,
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
        };
        let (model, wl) = (&mut app.model, &mut app.wl);
        match msg {
            Msg::ClientCopied(formats) => model.client_copied(&formats, wl),
            Msg::ClientAsks(format) => model.client_asks(format, wl),
            Msg::ClientSent(bytes) => model.client_sent(bytes, wl),
            Msg::Read(bytes) => model.desktop_read(bytes, wl),
            Msg::Stop => return false,
        }
    }
}

struct App {
    model: Clipboard<OwnedFd>,
    wl: Wl,
    done: bool,
    /// False until the client's first messages are handled.
    started: bool,
}

impl App {
    fn selection(&mut self, offer: Option<ExtDataControlOfferV1>) {
        if self.wl.echoes > 0 {
            self.wl.echoes -= 1;
            if let Some(o) = offer {
                o.destroy();
            }
            return;
        }
        // Someone else copied; the source's `cancelled` may come before or after this, and
        // destroys it then.
        self.wl.source = None;
        if let Some(old) = self.wl.offer.take() {
            old.destroy();
        }
        let mimes = mimes_of(offer.as_ref());
        self.wl.offer = offer;
        if self.started {
            self.model.desktop_copied(mimes);
        }
    }
}

fn mimes_of(offer: Option<&ExtDataControlOfferV1>) -> Vec<String> {
    (offer.and_then(|o| o.data::<Mutex<Vec<String>>>()))
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default()
}

struct Wl {
    manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    qh: QueueHandle<App>,
    mailbox: Mailbox,
    /// The desktop's copy, while it is not the client's.
    offer: Option<ExtDataControlOfferV1>,
    /// The client's copy, while it holds the clipboard.
    source: Option<ExtDataControlSourceV1>,
    /// `selection` events still to come for this connection's own `set_selection` requests.
    echoes: usize,
}

impl Desktop for Wl {
    type Sink = OwnedFd;

    fn offer(&mut self, mimes: &[&'static str]) {
        let source = self.manager.create_data_source(&self.qh, ());
        for m in mimes {
            source.offer(m.to_string());
        }
        self.device.set_selection(Some(&source));
        self.echoes += 1;
        if let Some(old) = self.source.replace(source) {
            old.destroy();
        }
        if let Some(offer) = self.offer.take() {
            offer.destroy();
        }
    }

    fn clear(&mut self) {
        if let Some(source) = self.source.take() {
            self.device.set_selection(None);
            self.echoes += 1;
            source.destroy();
        }
    }

    fn read(&mut self, mime: &str, kind: Kind) {
        let mailbox = self.mailbox.clone();
        let pipe = std::io::pipe();
        let (Some(offer), Ok((pipe, end))) = (&self.offer, pipe) else {
            mailbox.send(Msg::Read(None));
            return;
        };
        offer.receive(mime.into(), end.as_fd());
        drop(end);
        let pipe = OwnedFd::from(pipe);
        let mime = mime.to_string();
        std::thread::spawn(move || mailbox.send(Msg::Read(read_for_client(&pipe, &mime, kind))));
    }

    fn deliver(&mut self, sink: OwnedFd, kind: Kind, bytes: Option<Vec<u8>>) {
        let Some(bytes) = bytes else {
            return;
        };
        std::thread::spawn(move || match kind.to_desktop(&bytes) {
            Ok(converted) => write_detached(sink, converted.into()),
            Err(e) => log::warn!("clipboard: the client's {}: {e}", kind.noun()),
        });
    }
}

/// The desktop's copy of `mime` off `pipe`, converted for the client as `kind`; `None` when the
/// converted bytes are past the limit.
pub(crate) fn read_for_client(pipe: &OwnedFd, mime: &str, kind: Kind) -> Option<Vec<u8>> {
    // UTF-8 text is at most 3/2 the size of its UTF-16.
    let read_limit = match kind {
        Kind::Text => kind.limit() / 2 * 3,
        Kind::Dib | Kind::Png => kind.limit(),
    };
    match read_bounded(pipe, read_limit, PASTE_TIMEOUT) {
        Ok(bytes) => match kind.to_client(mime, &bytes) {
            Ok(wire) if wire.len() > kind.limit() => {
                clipboard::refuse(kind, "the desktop");
                None
            }
            Ok(wire) => Some(wire),
            Err(e) => {
                log::warn!("clipboard: the desktop's {}: {e}", kind.noun());
                None
            }
        },
        Err(e) if e.kind() == ErrorKind::FileTooLarge => {
            clipboard::refuse(kind, "the desktop");
            None
        }
        Err(e) => {
            log::warn!("clipboard: the desktop's {}: {e}", kind.noun());
            None
        }
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for App {
    fn event(
        app: &mut App,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id } => app.selection(id),
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            ext_data_control_device_v1::Event::Finished => app.done = true,
            _ => {}
        }
    }

    event_created_child!(App, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, Mutex::new(Vec::new())),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, Mutex<Vec<String>>> for App {
    fn event(
        _: &mut App,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        mimes: &Mutex<Vec<String>>,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            mimes.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, ()> for App {
    fn event(
        app: &mut App,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                app.model.desktop_pastes(&mime_type, fd, &mut app.wl);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if app.wl.source.as_ref() == Some(source) {
                    app.wl.source = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut App,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

delegate_noop!(App: ignore ExtDataControlManagerV1);
delegate_noop!(App: ignore WlSeat);

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Whether a paste answer reaches `rdp` within `within`.
    fn answered(
        rdp: &mut tokio::sync::mpsc::UnboundedReceiver<ServerEvent>,
        within: Duration,
    ) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if let Ok(ServerEvent::Clipboard(ClipboardMessage::SendFormatData(_))) = rdp.try_recv()
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_paste_asked_before_the_client_is_shown_the_session_waits_for_it() {
        let (tx, mut rdp) = tokio::sync::mpsc::unbounded_channel();
        let (shown, on_shown) = watch::channel(false);
        let link = start(tx, on_shown);
        link.send(Msg::ClientAsks(ClipboardFormatId::CF_UNICODETEXT));
        assert!(
            !answered(&mut rdp, Duration::from_millis(300)),
            "the session's clipboard is not reached before the client is shown it"
        );
        shown.send_replace(true);
        // The test session has no Wayland display, so the paste then ends empty.
        assert!(answered(&mut rdp, Duration::from_secs(5)));
    }

    #[test]
    fn a_paste_on_a_connection_whose_display_went_unshown_ends_empty() {
        let (tx, mut rdp) = tokio::sync::mpsc::unbounded_channel();
        let (shown, on_shown) = watch::channel(false);
        let link = start(tx, on_shown);
        drop(shown);
        link.send(Msg::ClientAsks(ClipboardFormatId::CF_UNICODETEXT));
        assert!(answered(&mut rdp, Duration::from_secs(5)));
    }
}
