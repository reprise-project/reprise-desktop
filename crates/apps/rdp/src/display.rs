//! The client's picture and pointer: each shared screen as H.264 on its own surface or as bitmaps.
//! The portal session opens on the first activation after the login.

mod screens;
#[cfg(test)]
mod tests;

use crate::client::ClientLayout;
use crate::console::{self, Console, Joined};
use crate::control::Status;
use crate::encode::choice;
use crate::encode::ffmpeg::Ffmpeg;
use crate::encode::probe::Backend;
use crate::encode::{Done, Encoded, Encoder, Job, Switch, Worker};
use crate::gfx::{Gfx, Prepared, Video};
use crate::input::{self, Feed, SharedMoves};
use crate::layout::{Desk, Kind, ScreenRect};
use crate::listen::{Ending, Shown};
use crate::portal::{self, Grant};
use crate::stream::{
    self, CursorImage, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Offer, SizeChange,
};
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use ironrdp_pdu::pointer::PointerPositionAttribute;
use ironrdp_pdu::rdp::server_error_info::ProtocolIndependentCode;
use ironrdp_server::{
    BitmapUpdate, DesktopSize, DisplayContext, DisplayUpdate, ErrorInfo, RGBAPointer,
    RdpServerDisplay, RdpServerDisplayUpdates, ServerError, ServerErrorExt, ServerErrorKind,
    ServerResult,
};
use repriseconf::remote::{EncoderChoice, Quality, RemoteDesktop};
use screens::{Joint, Lives, Pointer, gone_with_all};
use std::collections::VecDeque;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

/// The size a client is first told before any picture was shared.
pub const UNSHARED: DesktopSize = DesktopSize {
    width: 1024,
    height: 768,
};
/// The most monitors a client's Display Control layout may give, as many as its Client Monitor
/// Data holds.
const MAX_MONITORS: u32 = 16;
/// The largest pointer a client takes without the large pointer capability.
const MAX_POINTER: usize = 96;
/// How long a frame the encoder cannot read waits before the stream is asked to negotiate
/// again, and as long again before the next encoder takes over.
const HOLD: Duration = Duration::from_secs(1);

/// A desktop size shared between the parts of the server that read it.
pub type ToldSize = Arc<Mutex<DesktopSize>>;

/// The size one connection told its client, and the size last told on any connection, which
/// the next connection starts at.
#[derive(Clone)]
struct Told {
    size: ToldSize,
    last: ToldSize,
}

impl Told {
    fn new(last: ToldSize) -> Told {
        let size = Arc::new(Mutex::new(*last.lock().unwrap()));
        Told { size, last }
    }

    fn get(&self) -> DesktopSize {
        *self.size.lock().unwrap()
    }

    fn set(&self, size: DesktopSize) {
        *self.size.lock().unwrap() = size;
        *self.last.lock().unwrap() = size;
    }

    /// The desktop a client was offered for its monitors; the next connection still starts at
    /// `last`.
    fn offered(&self, size: DesktopSize) {
        *self.size.lock().unwrap() = size;
    }
}

/// The H.264 encoders found at start.
#[derive(Default)]
pub struct Picture {
    /// The render node they code on.
    pub device: PathBuf,
    /// The backends that coded the probe's test frame, best first, each with the dma-buf
    /// modifiers it maps, LINEAR first.
    pub working: Vec<(Backend, Vec<u64>)>,
}

impl Picture {
    pub fn backends(&self) -> Vec<Backend> {
        self.working.iter().map(|(b, _)| *b).collect()
    }

    fn modifiers(&self, backend: Backend) -> &[u64] {
        (self.working.iter())
            .find(|(b, _)| *b == backend)
            .map_or(&[DRM_FORMAT_MOD_LINEAR], |(_, m)| m.as_slice())
    }
}

/// The probe's find, None while it runs.
pub type Probed = watch::Receiver<Option<Arc<Picture>>>;

/// The probe's find once it is in; nothing works if the probe thread died.
pub async fn probed(probed: &mut Probed) -> Arc<Picture> {
    let found = probed.wait_for(Option::is_some).await;
    found.ok().and_then(|p| p.clone()).unwrap_or_default()
}

/// The settings a connection follows while it runs, and the status it reports its encoder in.
#[derive(Clone)]
pub struct Links {
    pub config: watch::Receiver<RemoteDesktop>,
    pub status: watch::Sender<Status>,
    /// How many [`Live`]s run; the last to drop puts the configured encoder back in `status`.
    pub lives: Arc<AtomicUsize>,
}

/// What the stream offers while H.264 is sent: the tiled modifiers when there are any. LINEAR
/// stays out, as the producer keeps a format the new offer still allows.
fn h264_modifiers(importable: &[u64]) -> Vec<u64> {
    let tiled: Vec<u64> = (importable.iter())
        .copied()
        .filter(|&m| m != DRM_FORMAT_MOD_LINEAR)
        .collect();
    match tiled.is_empty() {
        true => vec![DRM_FORMAT_MOD_LINEAR],
        false => tiled,
    }
}

/// Sharp and balanced code AVC444 where the client takes it; speed keeps to AVC420.
fn sharp_text(quality: Quality) -> bool {
    quality != Quality::Speed
}

/// The time between two encoded frames at `frame_rate` frames per second, rounded to the
/// nanosecond.
pub fn frame_interval(frame_rate: u32) -> Duration {
    let rate = u64::from(frame_rate.max(1));
    Duration::from_nanos((1_000_000_000 + rate / 2) / rate)
}

pub struct Display {
    told: Told,
    picture: Probed,
    /// The connection's graphics pipeline, when an H.264 encoder works or the probe still runs.
    gfx: Option<Gfx>,
    session: Option<Session>,
    /// The picture's state between activations; out while an [`Updates`] sends from it.
    live: Arc<Mutex<Option<Lives>>>,
    /// The client's input, until the portal session it goes to is open.
    input: Option<Feed>,
    moves: SharedMoves,
    links: Links,
    /// Until the portal session is granted.
    shown: Option<Shown>,
    ending: Ending,
    client: ClientLayout,
    /// The size the client asked for on activation.
    client_size: Option<DesktopSize>,
    console: Console,
    /// Its place in the console's run, from the first activation on.
    joined: Option<Joined>,
    /// The client's Display Control layouts, read by its run once it joins.
    layouts: UnboundedSender<Vec<console::Monitor>>,
    layouts_rx: Option<UnboundedReceiver<Vec<console::Monitor>>>,
}

/// What a connection tells its display: how to report the session shown and its screens gone,
/// the monitors its client sent, and the console its run of viewers is on.
pub struct Connection {
    pub shown: Shown,
    pub ending: Ending,
    pub client: ClientLayout,
    pub console: Console,
}

impl Display {
    /// A connection that starts at `last`, the size last told on any connection.
    pub fn new(
        last: ToldSize,
        input: Feed,
        picture: Probed,
        gfx: Option<Gfx>,
        links: Links,
        connection: Connection,
    ) -> Display {
        let (layouts, layouts_rx) = unbounded_channel();
        Display {
            told: Told::new(last),
            picture,
            gfx,
            links,
            session: None,
            live: Arc::default(),
            moves: input.moves.clone(),
            input: Some(input),
            shown: Some(connection.shown),
            ending: connection.ending,
            client: connection.client,
            client_size: None,
            console: connection.console,
            joined: None,
            layouts,
            layouts_rx: Some(layouts_rx),
        }
    }

    /// Joins the console's run once and says whether it locks the console. Its first viewer
    /// publishes the client's monitors and follows its Display Control layouts.
    fn join(&mut self) -> bool {
        if self.joined.is_none() {
            let size = self.client_size.unwrap_or_else(|| self.told.get());
            let joined = self.console.join(self.client.console_layout(size));
            if let Some(layouts) = self.layouts_rx.take() {
                joined.follow(layouts);
            }
            self.joined = Some(joined);
        }
        self.joined.as_ref().is_some_and(Joined::lock)
    }
}

struct Session {
    _grant: Arc<Grant>,
    _streams: Vec<stream::Stream>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        self.told.get()
    }

    /// Runs on every activation: a client confirming the desktop its monitors span gets it, any
    /// other the size this connection told last, which starts at [`Display::new`]'s `last`.
    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        self.client_size = Some(client_size);
        let confirmed = (client_size.width, client_size.height);
        if self.client.desktop() == Some(confirmed) {
            self.told.offered(client_size);
        }
        self.told.get()
    }

    /// Lock mode's screens follow its first viewer's layout; a later viewer's layouts and shared
    /// physical screens' sizes are not followed.
    async fn offers_display_control(&mut self) -> bool {
        self.console.would_own()
    }

    async fn monitor_count(&mut self) -> u32 {
        MAX_MONITORS
    }

    /// Called under the server's display lock, so it only passes the layout on.
    fn request_layout(&mut self, layout: DisplayControlMonitorLayout) {
        let scale = self.client.scale();
        match console::from_display_control(&layout, MAX_MONITORS, scale) {
            Ok(monitors) => {
                let _ = self.layouts.send(monitors);
            }
            Err(why) => log::warn!("Display Control layout ignored: {why}"),
        }
    }

    async fn updates(
        &mut self,
        _: DisplayContext,
    ) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        if self.session.is_none() {
            let lock = self.join();
            let picture = probed(&mut self.picture).await;
            let mut grant =
                (portal::open().await).map_err(|e| unshared("the screen is not shared", e))?;
            for screen in &grant.screens {
                log::info!(
                    "sharing {} at {:?}, {}x{} logical",
                    screen.mapping_id.as_deref().unwrap_or("the screen"),
                    screen.position,
                    screen.size.0,
                    screen.size.1
                );
            }
            let fds = remotes(&mut grant)?;
            self.ending.granted(lock, &grant.screens);
            let gone = gone_with_all(self.ending.clone(), grant.screens.len());
            let mut streams = Vec::new();
            for (screen, fd) in grant.screens.iter().zip(fds) {
                let (stream, frames, sizes) =
                    stream::spawn(fd, screen.node, &[DRM_FORMAT_MOD_LINEAR], gone.clone())
                        .map_err(|e| unshared("the screen stream did not start", e))?;
                streams.push((stream, frames, sizes));
            }
            let screens = (grant.screens.iter())
                .map(|s| ScreenRect {
                    position: s.position,
                    logical: s.size,
                    pixels: s.size,
                })
                .collect();
            let joint = Joint::new(Desk::new(screens, &self.client.monitors()));
            let grant = Arc::new(grant);
            if let Some(feed) = self.input.take() {
                tokio::task::spawn_local(input::serve(
                    grant.clone(),
                    feed.events,
                    joint.desk.clone(),
                ));
            }
            if let Some(shown) = self.shown.take() {
                shown.send();
            }
            let mut lives = Vec::new();
            let mut kept = Vec::new();
            for (screen, (stream, frames, sizes)) in streams.into_iter().enumerate() {
                let path = self.gfx.as_ref().map(|gfx| H264 {
                    gfx: gfx.clone(),
                    offer: stream.offer(),
                    picture: picture.clone(),
                    modifiers: vec![DRM_FORMAT_MOD_LINEAR],
                    on: false,
                });
                let device = picture.device.clone();
                let worker = Worker::spawn(move || {
                    Switch::new(Box::new(move |backend, quality, frame_rate| {
                        Ffmpeg::new(backend, &device, frame_rate, quality)
                            .map(|f| Box::new(f) as Box<dyn Encoder>)
                    }))
                })
                .map_err(|e| ServerError::io("no encoding thread", e))?;
                let live = Live::new(
                    frames,
                    sizes,
                    self.told.clone(),
                    worker,
                    path,
                    self.moves.clone(),
                    self.links.clone(),
                );
                lives.push(live.on(&joint, screen));
                kept.push(stream);
            }
            self.session = Some(Session {
                _grant: grant,
                _streams: kept,
            });
            *self.live.lock().unwrap() = Some(Lives::new(lives));
        }
        let mut live = (self.live.lock().unwrap().take())
            .ok_or_else(|| ServerError::reason("display", "the picture is already being sent"))?;
        live.activate();
        Ok(Box::new(Updates {
            live: Some(live),
            slot: self.live.clone(),
        }))
    }
}

/// Each of `grant`'s screens' PipeWire remote, one per screen.
fn remotes(grant: &mut Grant) -> ServerResult<Vec<std::os::fd::OwnedFd>> {
    let fds = grant.take_pipewire_fds();
    if fds.len() != grant.screens.len() {
        let counts = format!("{} remotes for {} screens", fds.len(), grant.screens.len());
        return Err(unshared("the screen streams did not start", counts));
    }
    Ok(fds)
}

/// The error a display that cannot share the screen ends with: the client is told the display
/// failed to start, so it does not reconnect at once to the same failure.
fn unshared(what: &str, e: impl std::fmt::Display) -> ServerError {
    log::warn!("{what}: {e}");
    let failed = ProtocolIndependentCode::CloseStackOnDriverFailure;
    ServerError::new(
        "display",
        ServerErrorKind::ErrorInfo(ErrorInfo::ProtocolIndependentCode(failed)),
    )
}

/// Sends from the [`Lives`] and puts them back when the activation ends.
struct Updates {
    live: Option<Lives>,
    slot: Arc<Mutex<Option<Lives>>>,
}

impl Drop for Updates {
    fn drop(&mut self) {
        *self.slot.lock().unwrap() = self.live.take();
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        match self.live.as_mut() {
            Some(live) => Ok(live.next().await),
            None => Ok(None),
        }
    }
}

/// The H.264 path of one screen.
struct H264 {
    gfx: Gfx,
    offer: Offer,
    picture: Arc<Picture>,
    /// What the stream offers while the backend in use codes.
    modifiers: Vec<u64>,
    /// H.264 is being sent: the surface is up and the stream offers `modifiers`.
    on: bool,
}

/// A pointer position held back as an echo: where the picture has it, where that is in the
/// client's pixels, and when to look at it again.
#[derive(Clone, Copy)]
struct HeldWarp {
    until: Instant,
    position: (i32, i32),
    at: (u16, u16),
}

/// One shared screen's picture on its way to the client.
struct Live {
    /// This screen's index in `joint`'s desk.
    screen: usize,
    joint: Joint,
    relaid: watch::Receiver<u64>,
    /// The stream ended; the other screens go on.
    ended: bool,
    frames: UnboundedReceiver<Frame>,
    sizes: UnboundedReceiver<SizeChange>,
    told: Told,
    worker: Worker,
    /// None when no H.264 encoder works.
    h264: Option<H264>,
    config: watch::Receiver<RemoteDesktop>,
    status: watch::Sender<Status>,
    lives: Arc<AtomicUsize>,
    quality: Quality,
    frame_rate: u32,
    choice: EncoderChoice,
    /// Counts encoder choices, so a frame coded before the latest is told apart.
    choices: u64,
    /// The latest choice's backends, until a job takes them to the worker.
    order: Option<Vec<Backend>>,
    /// The backend that codes H.264; None while bitmaps are sent.
    using: Option<Backend>,
    interval: Duration,
    /// The newest frame, kept to repaint a client after an activation.
    last: Option<Frame>,
    /// The frame to encode next, carrying the damage of every frame merged into it.
    pending: Option<Frame>,
    /// A frame is with the worker.
    busy: bool,
    /// Since when `pending` is a buffer the encoder cannot read, and how often the stream was
    /// asked again since.
    unreadable: Option<(Instant, u32)>,
    /// When the next encode may start.
    next_at: Instant,
    /// Counts activations and resizes; an encode started before the latest is not sent.
    epoch: u64,
    queue: VecDeque<DisplayUpdate>,
    keyframe: bool,
    held: Option<HeldWarp>,
    /// The client's own pointer moves, which are not sent back to it.
    moves: SharedMoves,
    /// The last frame failed to reach the client; it is tried again, logged once.
    failing: bool,
    /// A resize held until the client opens the graphics pipeline ([`crate::gfx::NEGOTIATION`]);
    /// `last` is then of a size the client was not told and is not repainted.
    held_resize: Option<(u32, u32)>,
    /// The H.264 the client was last sent, AVC420 or AVC444, logged when it changes.
    coding: Option<&'static str>,
}

impl Live {
    fn new(
        frames: UnboundedReceiver<Frame>,
        sizes: UnboundedReceiver<SizeChange>,
        told: Told,
        worker: Worker,
        h264: Option<H264>,
        moves: SharedMoves,
        links: Links,
    ) -> Live {
        let Links {
            mut config,
            status,
            lives,
        } = links;
        lives.fetch_add(1, Ordering::Relaxed);
        let cfg = *config.borrow_and_update();
        let joint = Joint::alone();
        let mut live = Live {
            screen: 0,
            relaid: joint.relaid.subscribe(),
            joint,
            ended: false,
            frames,
            sizes,
            told,
            worker,
            h264,
            config,
            status,
            lives,
            quality: cfg.quality,
            frame_rate: cfg.frame_rate,
            choice: cfg.encoder,
            choices: 0,
            order: None,
            using: None,
            interval: frame_interval(cfg.frame_rate),
            last: None,
            pending: None,
            busy: false,
            unreadable: None,
            next_at: Instant::now(),
            epoch: 0,
            queue: VecDeque::new(),
            keyframe: true,
            held: None,
            moves,
            failing: false,
            held_resize: None,
            coding: None,
        };
        live.choose(cfg.encoder);
        live
    }

    /// This Live as screen `screen` of the connection's `joint`.
    fn on(mut self, joint: &Joint, screen: usize) -> Live {
        self.screen = screen;
        self.joint = joint.clone();
        self.relaid = joint.relaid.subscribe();
        self
    }

    /// The client (re)started showing the desktop: everything is sent again.
    fn activate(&mut self) {
        self.queue.clear();
        self.keyframe = true;
        self.epoch += 1;
        {
            let mut seat = self.joint.seat.lock().unwrap();
            seat.pointer = Pointer::Unknown;
            seat.position = None;
        }
        self.held = None;
        if self.held_resize.is_none() {
            self.pending = self.last.clone();
        }
        self.next_at = Instant::now();
    }

    /// The next update; None once the stream ended. Cancel safe: what was taken off the
    /// channels is kept in `self` before any await.
    async fn next(&mut self) -> Option<DisplayUpdate> {
        if self.ended {
            return None;
        }
        loop {
            if let Some(update) = self.queue.pop_front() {
                return Some(update);
            }
            self.start(Instant::now());
            if !self.queue.is_empty() {
                continue;
            }
            let wait = self.pending.is_some() && !self.busy;
            let at = tokio::time::Instant::from_std(self.next_at);
            let held = self.held.map(|h| tokio::time::Instant::from_std(h.until));
            let opening = self
                .held_resize
                .and(self.h264.as_ref())
                .map(|p| p.gfx.clone());
            let negotiated = async {
                if let Some(gfx) = &opening {
                    gfx.negotiated().await;
                }
            };
            tokio::select! {
                biased;
                Ok(()) = self.config.changed() => self.follow_config(),
                Ok(()) = self.relaid.changed() => self.repaint(),
                size = self.sizes.recv() => match size {
                    Some(s) => {
                        self.resize(s.size);
                    }
                    None => break,
                },
                done = self.worker.done.recv() => match done {
                    Some(d) => self.finish(d),
                    None => break,
                },
                frame = self.frames.recv() => match frame {
                    Some(f) => self.take(f),
                    None => break,
                },
                () = tokio::time::sleep_until(at), if wait => {}
                () = tokio::time::sleep_until(held.unwrap_or(at)), if held.is_some() => {
                    if let Some(h) = self.held.take() {
                        self.place_pointer(h.position, h.at);
                    }
                }
                () = negotiated, if opening.is_some() => self.opened(),
            }
        }
        log::info!("the screen stream ended");
        self.ended = true;
        None
    }

    /// The client opened the graphics pipeline or ran out of time: the held resize goes now, or
    /// the frame it held back is painted, as another screen told the client the size first.
    fn opened(&mut self) {
        if let Some(size) = self.held_resize.take()
            && !self.resize(size)
        {
            self.repaint();
        }
    }

    /// Send the last frame again, whole: another screen moved this one's surface.
    fn repaint(&mut self) {
        self.relaid.borrow_and_update();
        self.keyframe = true;
        if self.pending.is_none() && self.held_resize.is_none() {
            self.pending = self.last.clone();
        }
    }

    /// Change the connection's desk with `change`, telling the other screens when it moved
    /// them; the desktop it spans now.
    fn relayout(&mut self, change: impl FnOnce(&mut Desk) -> bool) -> DesktopSize {
        let mut desk = self.joint.desk.lock().unwrap();
        if change(&mut desk) {
            self.joint.relaid.send_modify(|n| *n += 1);
            self.relaid.borrow_and_update();
        }
        let (width, height) = desk.layout().desktop;
        DesktopSize { width, height }
    }

    /// Lay every screen out at its own pixels, as bitmaps are; the client is resized to it, or
    /// else this screen is painted again.
    fn go_whole(&mut self) {
        log::info!("laying the screens out whole for bitmaps");
        let want = self.relayout(Desk::whole);
        match self.told.get() == want {
            true => self.repaint(),
            false => self.tell(want),
        }
    }

    fn whole(&self) -> bool {
        self.joint.desk.lock().unwrap().layout().kind == Kind::Whole
    }

    /// Take the quality, frame rate and encoder the config now has, keeping the client. A new
    /// frame rate reopens the encoder, so the next frame is a keyframe.
    fn follow_config(&mut self) {
        let cfg = *self.config.borrow_and_update();
        let recode = (cfg.quality, cfg.frame_rate) != (self.quality, self.frame_rate);
        self.quality = cfg.quality;
        self.frame_rate = cfg.frame_rate;
        self.interval = frame_interval(cfg.frame_rate);
        if cfg.encoder != self.choice {
            self.choose(cfg.encoder);
        } else if recode {
            self.keyframe = true;
        } else {
            return;
        }
        if self.pending.is_none() && self.held_resize.is_none() {
            self.pending = self.last.clone();
        }
    }

    /// Code with `choice` from the next frame on, a keyframe.
    fn choose(&mut self, choice: EncoderChoice) {
        self.choice = choice;
        self.choices += 1;
        let order = match &self.h264 {
            Some(path) => choice::order(choice, &path.picture.backends()),
            None => Vec::new(),
        };
        let first = order.first().copied();
        self.order = Some(order);
        self.keyframe = true;
        self.unreadable = None;
        self.use_backend(first);
    }

    /// Follow the backend H.264 is coded with now: the stream offers what it maps, the status
    /// names it, and with none the surface closes so bitmaps show.
    fn use_backend(&mut self, backend: Option<Backend>) {
        self.using = backend;
        let Some(path) = &mut self.h264 else {
            return;
        };
        match backend {
            Some(b) => {
                path.modifiers = h264_modifiers(path.picture.modifiers(b));
                if path.on {
                    path.offer.modifiers(&path.modifiers);
                }
            }
            None if path.on => {
                log::info!("sending bitmaps");
                self.coding = None;
                path.on = false;
                path.gfx.close(self.screen);
                path.offer.modifiers(&[DRM_FORMAT_MOD_LINEAR]);
                self.keyframe = true;
            }
            None => {}
        }
        show_encoder(&self.status, &choice::label(self.choice, backend));
    }

    /// Queue a resize when the client was told another size, or hold it while the client opens
    /// the graphics pipeline; true if it was either.
    fn resize(&mut self, size: (u32, u32)) -> bool {
        self.held_resize = None;
        let screen = self.screen;
        let want = self.relayout(|desk| desk.resized(screen, size));
        if self.told.get() == want {
            return false;
        }
        if self.h264.as_ref().is_some_and(|p| p.gfx.negotiating()) {
            self.held_resize = Some(size);
            self.pending = None;
            return true;
        }
        if self.last.as_ref().is_some_and(|f| f.size != size) {
            self.last = None;
        }
        self.tell(want);
        true
    }

    /// Queue a resize of the client to `want`.
    fn tell(&mut self, want: DesktopSize) {
        self.told.set(want);
        self.pending = None;
        self.epoch += 1;
        self.queue.push_back(DisplayUpdate::Resize(want));
        self.keyframe = true;
    }

    /// Take `frame`, merged with any newer ones already waiting, as the next to encode.
    fn take(&mut self, mut frame: Frame) {
        while let Ok(newer) = self.frames.try_recv() {
            frame = merge(frame, newer);
        }
        if self.resize(frame.size) {
            self.last = Some(frame);
            return;
        }
        self.pointer_updates(&frame);
        self.last = Some(frame.clone());
        self.pending = Some(match self.pending.take() {
            Some(older) => merge(older, frame),
            None => frame,
        });
    }

    /// Hand the pending frame to the worker if it is idle and the frame interval has passed.
    fn start(&mut self, now: Instant) {
        if self.busy || now < self.next_at {
            return;
        }
        let Some(frame) = self.pending.take() else {
            self.unreadable = None;
            return;
        };
        if frame.damage.is_empty() && !self.keyframe {
            self.unreadable = None;
            return;
        }
        let (mut h264, mut avc444) = (false, false);
        let desk = self.joint.desk.clone();
        let desk = desk.lock().unwrap();
        let layout = desk.layout();
        let target = layout.target(self.screen);
        if let Some(path) = &mut self.h264
            && self.using.is_some()
        {
            match path.gfx.prepare(layout, self.screen) {
                Prepared::Ready {
                    new,
                    avc444: offered,
                } => {
                    if !path.on {
                        log::info!("sending H.264");
                        path.on = true;
                        path.offer.modifiers(&path.modifiers);
                        show_encoder(&self.status, &choice::label(self.choice, self.using));
                    }
                    self.keyframe |= new;
                    h264 = true;
                    avc444 = offered && sharp_text(self.quality);
                }
                Prepared::Behind => {
                    self.pending = Some(frame);
                    self.next_at = now + self.interval;
                    return;
                }
                Prepared::Unavailable => {
                    if path.on {
                        log::info!("the graphics pipeline closed; sending bitmaps");
                        path.on = false;
                        path.offer.modifiers(&[DRM_FORMAT_MOD_LINEAR]);
                        self.keyframe = true;
                    }
                    show_encoder(&self.status, &choice::label(self.choice, None));
                }
            }
        }
        let fitted = layout.kind != Kind::Whole;
        drop(desk);
        if !h264 && fitted {
            match self.h264.as_ref().is_some_and(|p| p.gfx.negotiating()) {
                true => {
                    self.pending = Some(frame);
                    self.next_at = now + self.interval;
                }
                false => self.go_whole(),
            }
            return;
        }
        let (reader, readable) = match (&self.h264, self.using) {
            (Some(path), Some(b)) if h264 => (Some(b), path.picture.modifiers(b)),
            _ => (None, &[DRM_FORMAT_MOD_LINEAR][..]),
        };
        if !reads(&frame, reader, readable) {
            self.hold(frame, now, h264);
            return;
        }
        self.unreadable = None;
        // A target that only evens the frame is no scale: AVC444 codes it unscaled.
        let target = (target.filter(|_| h264))
            .map(|t| (u32::from(t.w), u32::from(t.h)))
            .filter(|&t| t != crate::encode::hwmap::even(frame.size));
        self.worker.send(Job {
            frame,
            keyframe: std::mem::take(&mut self.keyframe),
            h264,
            quality: self.quality,
            frame_rate: self.frame_rate,
            avc444,
            target,
            epoch: self.epoch,
            order: self.order.take(),
            choice: self.choices,
        });
        self.busy = true;
        self.next_at = now + self.interval;
    }

    /// Keep `frame`, which the encoder cannot read, for the stream's next one: renegotiate every
    /// [`HOLD`], and after two the next encoder in the order takes over (bitmaps after the last).
    fn hold(&mut self, frame: Frame, now: Instant, h264: bool) {
        let buffer = match &frame.buffer {
            FrameBuffer::Dmabuf { modifier, .. } => format!("modifier {modifier:#x}"),
            FrameBuffer::Shm { .. } => "shared memory".to_string(),
        };
        self.pending = Some(frame);
        self.next_at = now + self.interval;
        let (since, asks) = *self.unreadable.get_or_insert((now, 0));
        let held = now.saturating_duration_since(since);
        let reader = match (h264, self.using) {
            (true, Some(b)) => b.label(),
            _ => "The bitmap encoder",
        };
        if held >= 2 * HOLD && h264 {
            self.unreadable = None;
            self.skip_backend(&buffer);
        } else if held >= HOLD * (asks + 1) {
            self.unreadable = Some((since, asks + 1));
            if asks == 0 {
                log::warn!(
                    "{reader} cannot read the stream's buffers ({buffer}); asking the stream again"
                );
            }
            if let Some(path) = &self.h264 {
                match h264 {
                    true => path.offer.again(&path.modifiers),
                    false => path.offer.again(&[DRM_FORMAT_MOD_LINEAR]),
                }
            }
        }
    }

    /// Code with the backend after the one in use, which cannot read the stream's `buffer`,
    /// from the next frame on, a keyframe; bitmaps after the last.
    fn skip_backend(&mut self, buffer: &str) {
        let (Some(stuck), Some(path)) = (self.using, &self.h264) else {
            return;
        };
        let order = choice::order(self.choice, &path.picture.backends());
        let rest: Vec<Backend> = (order.into_iter())
            .skip_while(|&b| b != stuck)
            .skip(1)
            .collect();
        let next = rest.first().copied();
        match next {
            Some(b) => log::warn!(
                "{} still cannot read {buffer}, using {} instead",
                stuck.label(),
                b.label()
            ),
            None => log::warn!(
                "{} still cannot read {buffer}, sending bitmaps",
                stuck.label()
            ),
        }
        self.order = Some(rest);
        self.keyframe = true;
        self.use_backend(next);
    }

    /// Send what the worker made of a frame.
    fn finish(&mut self, done: Done) {
        self.busy = false;
        let current = done.choice == self.choices;
        let switched = current && done.using != self.using;
        if switched && self.h264.is_some() {
            self.use_backend(done.using);
        }
        if done.epoch != self.epoch {
            return;
        }
        let failed = match done.encoded {
            Ok(Encoded::Bitmap(updates)) => {
                match self.whole() {
                    true => {
                        let desk = self.joint.desk.lock().unwrap();
                        let layout = desk.layout();
                        let at = layout.target(self.screen).map_or((0, 0), |t| (t.x, t.y));
                        let desktop = layout.desktop;
                        drop(desk);
                        (self.queue).extend(
                            (updates.into_iter())
                                .filter_map(|u| on_desktop(u, at, desktop))
                                .map(DisplayUpdate::Bitmap),
                        );
                    }
                    false => self.go_whole(),
                }
                false
            }
            Ok(Encoded::Avc420 { data, regions }) => {
                self.coding("AVC420");
                !self.send(Video::Avc420(&data), &regions)
            }
            Ok(Encoded::Avc444 { main, aux, regions }) => {
                self.coding("AVC444");
                let aux = aux.as_deref();
                !self.send(Video::Avc444 { main: &main, aux }, &regions)
            }
            Ok(Encoded::Nothing) => false,
            Err(e) => {
                if !switched && !self.failing {
                    log::warn!("encoding a frame: {e}");
                }
                true
            }
        };
        self.failing = failed;
        if failed {
            self.keyframe = true;
            if self.pending.is_none() && self.held_resize.is_none() {
                self.pending = self.last.clone();
            }
        }
    }

    fn coding(&mut self, kind: &'static str) {
        if self.coding != Some(kind) {
            log::info!("sending {kind}");
            self.coding = Some(kind);
        }
    }

    fn send(&self, video: Video<'_>, regions: &[stream::Rect]) -> bool {
        let qp = self.quality.qp();
        (self.h264.as_ref()).is_some_and(|path| path.gfx.send(self.screen, video, regions, qp))
    }

    /// The pointer as this screen's `frame` has it. A screen the pointer is not over leaves it
    /// to the screen it is over.
    fn pointer_updates(&mut self, frame: &Frame) {
        let Some(cursor) = &frame.cursor else {
            return;
        };
        let mut seat = self.joint.seat.lock().unwrap();
        let pointer = match &cursor.image {
            Some(image) => Pointer::Shown(image.clone()),
            None if seat.over.is_none_or(|s| s == self.screen) => Pointer::Hidden,
            None => return,
        };
        seat.over = cursor.image.as_ref().map(|_| self.screen);
        if pointer != seat.pointer {
            self.queue.push_back(match &pointer {
                Pointer::Shown(image) => DisplayUpdate::RGBAPointer(rdp_pointer(image)),
                _ => DisplayUpdate::HidePointer,
            });
            seat.pointer = pointer;
        }
        self.held = None;
        if cursor.image.is_some() && seat.position != Some((self.screen, cursor.position)) {
            drop(seat);
            let clamp = |v: i32, max: u32| v.clamp(0, max.saturating_sub(1) as i32) as u32;
            let p = (
                clamp(cursor.position.0, frame.size.0),
                clamp(cursor.position.1, frame.size.1),
            );
            let layout = self.joint.desk.lock().unwrap();
            let Some(at) = layout.layout().to_client(self.screen, p) else {
                return;
            };
            drop(layout);
            self.place_pointer(cursor.position, at);
        }
    }

    /// Send the pointer's position unless it echoes the client's own moves. An echo is looked
    /// at again a frame after its window ends, as a static screen brings no frame to do it.
    fn place_pointer(&mut self, position: (i32, i32), at: (u16, u16)) {
        let mut moves = self.moves.lock().unwrap();
        if moves.is_echo(at, input::now()) {
            self.held = (moves.echo_ends(at)).map(|end| HeldWarp {
                until: end + self.interval,
                position,
                at,
            });
            return;
        }
        moves.sent();
        drop(moves);
        self.joint.seat.lock().unwrap().position = Some((self.screen, position));
        self.queue
            .push_back(DisplayUpdate::PointerPosition(PointerPositionAttribute {
                x: at.0,
                y: at.1,
            }));
    }
}

impl Drop for Live {
    /// The last connection ended: the status names what the config picks again.
    fn drop(&mut self) {
        if self.lives.fetch_sub(1, Ordering::Relaxed) > 1 {
            return;
        }
        let working = (self.h264.as_ref()).map_or_else(Vec::new, |p| p.picture.backends());
        let label = choice::configured(self.config.borrow().encoder, &working);
        show_encoder(&self.status, &label);
    }
}

fn show_encoder(status: &watch::Sender<Status>, label: &str) {
    status.send_if_modified(|s| {
        let changed = s.encoder != label;
        s.encoder = label.into();
        changed
    });
}

/// Whether `backend` (None: the bitmap encoder), mapping `modifiers`, reads `frame`'s buffer;
/// the stream may still deliver buffers of an older offer.
fn reads(frame: &Frame, backend: Option<Backend>, modifiers: &[u64]) -> bool {
    crate::encode::reads(backend, frame)
        && match &frame.buffer {
            FrameBuffer::Dmabuf { modifier, .. } => modifiers.contains(modifier),
            FrameBuffer::Shm { .. } => true,
        }
}

/// Bitmap `u` moved by `at` and cut to `desktop`; None when none of it is on the desktop.
fn on_desktop(u: BitmapUpdate, at: (u16, u16), desktop: (u16, u16)) -> Option<BitmapUpdate> {
    let x = u32::from(u.x) + u32::from(at.0);
    let y = u32::from(u.y) + u32::from(at.1);
    let left = |at: u32, side: u16, len: NonZeroU16| {
        let room = u32::from(side).checked_sub(at)?;
        NonZeroU16::new(u16::try_from(room.min(u32::from(len.get()))).ok()?)
    };
    let (width, height) = (left(x, desktop.0, u.width)?, left(y, desktop.1, u.height)?);
    let mut cut = u.sub(0, 0, width, height)?;
    (cut.x, cut.y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
    Some(cut)
}

/// `newer` carrying `older`'s damage too, as its bounding box past
/// [`stream::MAX_DAMAGE_RECTS`] rectangles, and its pointer when it has none.
fn merge(older: Frame, mut newer: Frame) -> Frame {
    if newer.size == older.size {
        newer.damage.extend(older.damage);
        if newer.damage.len() > stream::MAX_DAMAGE_RECTS as usize {
            newer.damage = vec![bounds(&newer.damage)];
        }
    } else {
        newer.damage = vec![stream::Rect::full(newer.size)];
    }
    if newer.cursor.is_none() {
        newer.cursor = older.cursor;
    }
    newer
}

/// The smallest rectangle holding every one of `rects`, which is not empty.
fn bounds(rects: &[stream::Rect]) -> stream::Rect {
    let x = rects.iter().map(|r| r.x).min().unwrap_or(0);
    let y = rects.iter().map(|r| r.y).min().unwrap_or(0);
    let right = rects.iter().map(|r| r.x + r.width).max().unwrap_or(0);
    let bottom = rects.iter().map(|r| r.y + r.height).max().unwrap_or(0);
    stream::Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

/// `image` as an RDP pointer: cropped to its visible part (keeping the hotspot inside and at
/// most [`MAX_POINTER`] square), straight alpha, rows bottom-up.
fn rdp_pointer(image: &CursorImage) -> RGBAPointer {
    let (w, h) = (image.width as usize, image.height as usize);
    let pixel = |x: usize, y: usize| &image.bgra[(y * w + x) * 4..][..4];
    let hot = |v: i32, len: usize| (v.max(0) as usize).min(len.saturating_sub(1));
    let (hot_x, hot_y) = (hot(image.hotspot.0, w), hot(image.hotspot.1, h));
    let (mut right, mut bottom) = (hot_x + 1, hot_y + 1);
    for y in 0..h {
        for x in 0..w {
            if pixel(x, y)[3] != 0 {
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    let (right, bottom) = (right.min(MAX_POINTER), bottom.min(MAX_POINTER));
    let mut data = Vec::with_capacity(right * bottom * 4);
    for y in (0..bottom).rev() {
        for x in 0..right {
            let p = pixel(x, y);
            let a = u32::from(p[3]);
            let straight = |c: u8| match a {
                0 => 0,
                _ => ((u32::from(c) * 255 + a / 2) / a).min(255) as u8,
            };
            data.extend_from_slice(&[straight(p[0]), straight(p[1]), straight(p[2]), p[3]]);
        }
    }
    RGBAPointer {
        cache_index: 0,
        width: right as u16,
        height: bottom as u16,
        hot_x: hot_x.min(right - 1) as u16,
        hot_y: hot_y.min(bottom - 1) as u16,
        data,
    }
}

/// A display that shares nothing, for tests of the connection itself.
#[cfg(test)]
pub struct Grey;

#[cfg(test)]
#[async_trait::async_trait]
impl RdpServerDisplay for Grey {
    async fn size(&mut self) -> DesktopSize {
        UNSHARED
    }

    async fn updates(
        &mut self,
        _: DisplayContext,
    ) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        struct Nothing;
        #[async_trait::async_trait]
        impl RdpServerDisplayUpdates for Nothing {
            async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
                std::future::pending().await
            }
        }
        Ok(Box::new(Nothing))
    }
}
