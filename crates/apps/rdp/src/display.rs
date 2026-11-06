//! The client's picture: frames as H.264 on the graphics pipeline or as bitmaps, the pointer as
//! pointer updates. The portal session opens on the first activation after the login.

use crate::control::Status;
use crate::encode::choice;
use crate::encode::ffmpeg::Ffmpeg;
use crate::encode::probe::Backend;
use crate::encode::{Done, Encoded, Encoder, Job, Switch, Worker};
use crate::gfx::{Gfx, Prepared, Video};
use crate::input::{self, Feed, SharedMoves};
use crate::listen::{Ending, Shown};
use crate::portal::{self, Grant};
use crate::stream::{
    self, CursorImage, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Offer, SizeChange,
};
use ironrdp_pdu::pointer::PointerPositionAttribute;
use ironrdp_server::{
    DesktopSize, DisplayContext, DisplayUpdate, RGBAPointer, RdpServerDisplay,
    RdpServerDisplayUpdates, ServerError, ServerErrorExt, ServerResult,
};
use repriseconf::remote::{EncoderChoice, Quality, RemoteDesktop};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch;

/// The size a client is first told before any picture was shared.
pub const UNSHARED: DesktopSize = DesktopSize {
    width: 1024,
    height: 768,
};
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
    live: Arc<Mutex<Option<Live>>>,
    /// The client's input, until the portal session it goes to is open.
    input: Option<Feed>,
    moves: SharedMoves,
    links: Links,
    /// Until the portal session is granted.
    shown: Option<Shown>,
    ending: Ending,
}

impl Display {
    /// A connection that starts at `last`, the size last told on any connection.
    pub fn new(
        last: ToldSize,
        input: Feed,
        picture: Probed,
        gfx: Option<Gfx>,
        links: Links,
        shown: Shown,
        ending: Ending,
    ) -> Display {
        Display {
            told: Told::new(last),
            picture,
            gfx,
            links,
            session: None,
            live: Arc::default(),
            moves: input.moves.clone(),
            input: Some(input),
            shown: Some(shown),
            ending,
        }
    }
}

struct Session {
    _grant: Arc<Grant>,
    _stream: stream::Stream,
}

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        self.told.get()
    }

    async fn updates(
        &mut self,
        _: DisplayContext,
    ) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        if self.session.is_none() {
            let picture = probed(&mut self.picture).await;
            let grant = portal::open()
                .await
                .map_err(|e| ServerError::reason("the screen is not shared", e.to_string()))?;
            log::info!(
                "sharing {} at {:?}, {}x{} logical",
                grant.mapping_id.as_deref().unwrap_or("the screen"),
                grant.position,
                grant.size.0,
                grant.size.1
            );
            let ending = self.ending.clone();
            let (stream, frames, sizes) =
                stream::spawn(&grant, &[DRM_FORMAT_MOD_LINEAR], move || {
                    ending.screen_gone()
                })
                .map_err(|e| {
                    ServerError::reason("the screen stream did not start", e.to_string())
                })?;
            let grant = Arc::new(grant);
            if let Some(feed) = self.input.take() {
                tokio::task::spawn_local(input::serve(
                    grant.clone(),
                    feed.events,
                    self.told.size.clone(),
                ));
            }
            if let Some(shown) = self.shown.take() {
                shown.send();
            }
            let path = self.gfx.as_ref().map(|gfx| H264 {
                gfx: gfx.clone(),
                offer: stream.offer(),
                picture: picture.clone(),
                modifiers: vec![DRM_FORMAT_MOD_LINEAR],
                on: false,
            });
            let device = picture.device.clone();
            let frame_rate = self.links.config.borrow().frame_rate;
            let worker = Worker::spawn(move || {
                Switch::new(Box::new(move |backend, quality| {
                    Ffmpeg::new(backend, &device, frame_rate, quality)
                        .map(|f| Box::new(f) as Box<dyn Encoder>)
                }))
            })
            .map_err(|e| ServerError::io("no encoding thread", e))?;
            self.session = Some(Session {
                _grant: grant,
                _stream: stream,
            });
            *self.live.lock().unwrap() = Some(Live::new(
                frames,
                sizes,
                self.told.clone(),
                worker,
                path,
                self.moves.clone(),
                self.links.clone(),
            ));
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

/// Sends from a [`Live`] and puts it back when the activation ends.
struct Updates {
    live: Option<Live>,
    slot: Arc<Mutex<Option<Live>>>,
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

/// What the client shows of the pointer.
#[derive(PartialEq)]
enum Pointer {
    Unknown,
    Hidden,
    Shown(CursorImage),
}

/// The H.264 path of a connection.
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

struct Live {
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
    pointer: Pointer,
    /// The pointer position last sent to the client.
    position: Option<(i32, i32)>,
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
        let mut live = Live {
            frames,
            sizes,
            told,
            worker,
            h264,
            config,
            status,
            lives,
            quality: cfg.quality,
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
            pointer: Pointer::Unknown,
            position: None,
            held: None,
            moves,
            failing: false,
            held_resize: None,
            coding: None,
        };
        live.choose(cfg.encoder);
        live
    }

    /// The client (re)started showing the desktop: everything is sent again.
    fn activate(&mut self) {
        self.queue.clear();
        self.keyframe = true;
        self.epoch += 1;
        self.pointer = Pointer::Unknown;
        self.position = None;
        self.held = None;
        if self.held_resize.is_none() {
            self.pending = self.last.clone();
        }
        self.next_at = Instant::now();
    }

    /// The next update; None once the stream ended. Cancel safe: what was taken off the
    /// channels is kept in `self` before any await.
    async fn next(&mut self) -> Option<DisplayUpdate> {
        loop {
            if let Some(update) = self.queue.pop_front() {
                return Some(update);
            }
            self.start(Instant::now());
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
                () = negotiated, if opening.is_some() => {
                    if let Some(size) = self.held_resize.take() {
                        self.resize(size);
                    }
                }
            }
        }
        log::info!("the screen stream ended");
        None
    }

    /// Take the quality, frame rate and encoder the config now has, keeping the client.
    fn follow_config(&mut self) {
        let cfg = *self.config.borrow_and_update();
        let requality = cfg.quality != self.quality;
        self.quality = cfg.quality;
        self.interval = frame_interval(cfg.frame_rate);
        if cfg.encoder != self.choice {
            self.choose(cfg.encoder);
        } else if requality {
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
                path.gfx.close();
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
        let want = DesktopSize {
            width: u16::try_from(size.0).unwrap_or(u16::MAX),
            height: u16::try_from(size.1).unwrap_or(u16::MAX),
        };
        self.held_resize = None;
        if self.told.get() == want {
            return false;
        }
        if self.h264.as_ref().is_some_and(|p| p.gfx.negotiating()) {
            self.held_resize = Some(size);
            self.pending = None;
            return true;
        }
        self.told.set(want);
        if self.last.as_ref().is_some_and(|f| f.size != size) {
            self.last = None;
        }
        self.pending = None;
        self.epoch += 1;
        self.queue.push_back(DisplayUpdate::Resize(want));
        self.keyframe = true;
        true
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
        if let Some(path) = &mut self.h264
            && self.using.is_some()
        {
            match path.gfx.prepare(frame.size) {
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
        let (reader, readable) = match (&self.h264, self.using) {
            (Some(path), Some(b)) if h264 => (Some(b), path.picture.modifiers(b)),
            _ => (None, &[DRM_FORMAT_MOD_LINEAR][..]),
        };
        if !reads(&frame, reader, readable) {
            self.hold(frame, now, h264);
            return;
        }
        self.unreadable = None;
        self.worker.send(Job {
            frame,
            keyframe: std::mem::take(&mut self.keyframe),
            h264,
            quality: self.quality,
            avc444,
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
                (self.queue).extend(updates.into_iter().map(DisplayUpdate::Bitmap));
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
        (self.h264.as_ref()).is_some_and(|path| path.gfx.send(video, regions, self.quality.qp()))
    }

    fn pointer_updates(&mut self, frame: &Frame) {
        let Some(cursor) = &frame.cursor else {
            return;
        };
        let pointer = match &cursor.image {
            Some(image) => Pointer::Shown(image.clone()),
            None => Pointer::Hidden,
        };
        if pointer != self.pointer {
            self.queue.push_back(match &pointer {
                Pointer::Shown(image) => DisplayUpdate::RGBAPointer(rdp_pointer(image)),
                _ => DisplayUpdate::HidePointer,
            });
            self.pointer = pointer;
        }
        self.held = None;
        if cursor.image.is_some() && self.position != Some(cursor.position) {
            let clamp = |v: i32, max: u32| v.clamp(0, max.saturating_sub(1) as i32) as u16;
            let at = (
                clamp(cursor.position.0, frame.size.0),
                clamp(cursor.position.1, frame.size.1),
            );
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
        self.position = Some(position);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    fn live(told: DesktopSize) -> (Live, ToldSize, Frames, Sizes) {
        live_on(&Arc::new(Mutex::new(told)))
    }

    /// A connection started from `last`, with the size it told its client.
    fn live_on(last: &ToldSize) -> (Live, ToldSize, Frames, Sizes) {
        let (frames, frames_rx) = unbounded_channel();
        let (sizes, sizes_rx) = unbounded_channel();
        let told = Told::new(last.clone());
        let worker = Worker::spawn(Switch::bitmaps).unwrap();
        let live = Live::new(
            frames_rx,
            sizes_rx,
            told.clone(),
            worker,
            None,
            SharedMoves::default(),
            links().0,
        );
        (live, told.size, frames, sizes)
    }

    /// Links to the default config, with its sender and the status they report to.
    fn links() -> (Links, watch::Sender<RemoteDesktop>, watch::Receiver<Status>) {
        let (config_tx, config) = watch::channel(RemoteDesktop::default());
        let (status, status_rx) = watch::channel(Status::default());
        let lives = Arc::default();
        (
            Links {
                config,
                status,
                lives,
            },
            config_tx,
            status_rx,
        )
    }

    type Frames = tokio::sync::mpsc::UnboundedSender<Frame>;
    type Sizes = tokio::sync::mpsc::UnboundedSender<SizeChange>;

    fn frame(size: (u32, u32), damage: Rect) -> Frame {
        Frame::detached(vec![0; (size.0 * size.1 * 4) as usize], size, vec![damage])
    }

    fn bitmap_size(update: Option<DisplayUpdate>) -> (u16, u16) {
        match update {
            Some(DisplayUpdate::Bitmap(b)) => (b.width.get(), b.height.get()),
            other => panic!("a bitmap, not {other:?}"),
        }
    }

    const SMALL: Rect = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };

    #[tokio::test]
    async fn an_activation_repaints_the_last_frame() {
        let (mut live, _told, frames, _sizes) = live(DesktopSize {
            width: 4,
            height: 2,
        });
        frames.send(frame((4, 2), SMALL)).unwrap();
        assert_eq!(
            bitmap_size(live.next().await),
            (4, 2),
            "the first frame is whole"
        );
        frames.send(frame((4, 2), SMALL)).unwrap();
        assert_eq!(bitmap_size(live.next().await), (1, 1));
        live.activate();
        assert_eq!(
            bitmap_size(live.next().await),
            (4, 2),
            "repainted with no new frame"
        );
    }

    #[tokio::test]
    async fn a_size_change_resizes_once_and_repaints_at_the_new_size() {
        let (mut live, told, frames, sizes) = live(DesktopSize {
            width: 4,
            height: 2,
        });
        frames.send(frame((4, 2), SMALL)).unwrap();
        bitmap_size(live.next().await);
        sizes.send(SizeChange { size: (2, 2) }).unwrap();
        assert!(matches!(
            live.next().await,
            Some(DisplayUpdate::Resize(DesktopSize {
                width: 2,
                height: 2
            }))
        ));
        live.activate();
        frames.send(frame((2, 2), SMALL)).unwrap();
        assert_eq!(
            bitmap_size(live.next().await),
            (2, 2),
            "a whole frame at the new size"
        );
        assert_eq!(
            *told.lock().unwrap(),
            DesktopSize {
                width: 2,
                height: 2
            }
        );
    }

    #[tokio::test]
    async fn a_second_viewers_resize_leaves_the_first_viewers_size_alone() {
        let last = Arc::new(Mutex::new(DesktopSize {
            width: 4,
            height: 2,
        }));
        let (_first, first_told, _first_frames, _first_sizes) = live_on(&last);
        let (mut second, second_told, frames, sizes) = live_on(&last);
        frames.send(frame((4, 2), SMALL)).unwrap();
        bitmap_size(second.next().await);
        sizes.send(SizeChange { size: (2, 2) }).unwrap();
        assert!(matches!(
            second.next().await,
            Some(DisplayUpdate::Resize(_))
        ));
        let small = DesktopSize {
            width: 2,
            height: 2,
        };
        assert_eq!(*second_told.lock().unwrap(), small);
        assert_eq!(
            *first_told.lock().unwrap(),
            DesktopSize {
                width: 4,
                height: 2
            },
            "the first viewer keeps 4x2"
        );
        assert_eq!(
            *last.lock().unwrap(),
            small,
            "the next viewer starts at 2x2"
        );
    }

    /// Fails its `n`th encode, then encodes as bitmaps again.
    struct FailsOnce {
        n: usize,
        bitmap: crate::encode::bitmap::Bitmap,
    }

    impl Encoder for FailsOnce {
        fn encode(
            &mut self,
            frame: &Frame,
            keyframe: bool,
        ) -> Result<Encoded, crate::encode::EncodeError> {
            self.n = self.n.wrapping_sub(1);
            match self.n {
                0 => Err(crate::encode::EncodeError("the frame was lost".into())),
                _ => self.bitmap.encode(frame, keyframe),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_suppressed_warp_is_sent_once_the_echo_window_ends() {
        let (mut live, _told, frames, _sizes) = live(DesktopSize {
            width: 64,
            height: 64,
        });
        live.keyframe = false;
        let start = tokio::time::Instant::now();
        live.moves.lock().unwrap().moved((10, 10), start.into_std());
        let mut warped = frame((64, 64), SMALL);
        warped.damage.clear();
        warped.cursor = Some(stream::Cursor {
            position: (40, 40),
            image: Some(CursorImage {
                width: 1,
                height: 1,
                hotspot: (0, 0),
                bgra: bytes::Bytes::from_static(&[255; 4]),
            }),
        });
        frames.send(warped).unwrap();
        assert!(matches!(
            live.next().await,
            Some(DisplayUpdate::RGBAPointer(_))
        ));
        let update = tokio::time::timeout(Duration::from_secs(1), live.next()).await;
        match update.expect("the held-back warp is sent with no new frame") {
            Some(DisplayUpdate::PointerPosition(p)) => assert_eq!((p.x, p.y), (40, 40)),
            other => panic!("a pointer position, not {other:?}"),
        }
        let waited = start.elapsed();
        assert!(
            waited >= 2 * live.interval && waited <= 2 * live.interval + Duration::from_millis(1),
            "one frame of echo window and one frame interval, not {waited:?}"
        );
        let again = tokio::time::timeout(Duration::from_secs(1), live.next()).await;
        assert!(again.is_err(), "the warp is sent once");
    }

    #[tokio::test]
    async fn a_frame_that_failed_is_sent_again_on_a_static_screen() {
        let (frames, frames_rx) = unbounded_channel();
        let (_sizes, sizes_rx) = unbounded_channel();
        let told = Told::new(Arc::new(Mutex::new(DesktopSize {
            width: 4,
            height: 2,
        })));
        let switch = || {
            Switch::bitmaps().with_bitmap(Box::new(FailsOnce {
                n: 2,
                bitmap: crate::encode::bitmap::Bitmap,
            }))
        };
        let mut live = Live::new(
            frames_rx,
            sizes_rx,
            told,
            Worker::spawn(switch).unwrap(),
            None,
            SharedMoves::default(),
            links().0,
        );
        frames.send(frame((4, 2), SMALL)).unwrap();
        assert_eq!(bitmap_size(live.next().await), (4, 2));
        frames.send(frame((4, 2), SMALL)).unwrap();
        let repaint = tokio::time::timeout(Duration::from_secs(5), live.next()).await;
        assert_eq!(
            bitmap_size(repaint.expect("a repaint with no new frame")),
            (4, 2),
            "the last frame goes again, whole"
        );
    }

    /// Each frame a [`Coder`] coded: its backend, whether a keyframe was asked for, its QP.
    type Coded = Arc<Mutex<Vec<(Backend, bool, u8)>>>;

    /// Codes an empty access unit over the whole frame, noting whether AVC444 was asked for;
    /// a `lost` one fails every frame.
    struct Coder {
        backend: Backend,
        quality: Quality,
        avc444: bool,
        coded: Coded,
        modes: Modes,
        lost: bool,
    }

    type Modes = Arc<Mutex<Vec<bool>>>;

    impl Encoder for Coder {
        fn encode(
            &mut self,
            frame: &Frame,
            keyframe: bool,
        ) -> Result<Encoded, crate::encode::EncodeError> {
            if self.lost {
                return Err(crate::encode::EncodeError("the VA context was lost".into()));
            }
            let qp = self.quality.qp();
            self.modes.lock().unwrap().push(self.avc444);
            self.coded
                .lock()
                .unwrap()
                .push((self.backend, keyframe, qp));
            Ok(Encoded::Avc420 {
                data: vec![0, 0, 0, 1, 0x65],
                regions: vec![Rect::full(frame.size)],
            })
        }

        fn set_quality(&mut self, quality: Quality) {
            self.quality = quality;
        }

        fn set_avc444(&mut self, on: bool) {
            self.avc444 = on;
        }
    }

    const VAAPI: Backend = Backend::Vaapi { low_power: false };

    /// The probe's find: `working`, each mapping LINEAR only.
    fn picture(working: &[Backend]) -> Arc<Picture> {
        Arc::new(Picture {
            device: PathBuf::from("/dev/null"),
            working: (working.iter())
                .map(|&b| (b, vec![DRM_FORMAT_MOD_LINEAR]))
                .collect(),
        })
    }

    /// The server end that keeps a pipeline open, and the events it sends the connection.
    type Client = (
        ironrdp_server::GfxServerHandle,
        tokio::sync::mpsc::UnboundedReceiver<ironrdp_server::ServerEvent>,
    );

    /// A pipeline the client opened with AVC420.
    fn ready_gfx() -> (Gfx, Client) {
        use ironrdp_egfx::pdu::{CapabilitiesV81Flags, CapabilitySet};
        opened_gfx(CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED,
        })
    }

    /// A pipeline the client opened with `caps`.
    fn opened_gfx(caps: ironrdp_egfx::pdu::CapabilitySet) -> (Gfx, Client) {
        use ironrdp_dvc::DvcProcessor;
        use ironrdp_egfx::pdu::CapabilitiesAdvertisePdu;
        let gfx = Gfx::default();
        let (events, events_rx) = unbounded_channel();
        let mut factory = gfx.factory();
        factory.set_sender(events);
        let (_bridge, server) = gfx.open();
        server.lock().unwrap().start(0).unwrap();
        let advertise = ironrdp_egfx::pdu::GfxPdu::CapabilitiesAdvertise(
            CapabilitiesAdvertisePdu::from_typed(&[caps]),
        );
        let payload = ironrdp_core::encode_vec(&advertise).unwrap();
        server.lock().unwrap().process(0, &payload).unwrap();
        (gfx, (server, events_rx))
    }

    /// A connection sending H.264 coded by [`Coder`]s, `working` found by the probe.
    struct Coding {
        live: Live,
        coded: Coded,
        modes: Modes,
        config: watch::Sender<RemoteDesktop>,
        status: watch::Receiver<Status>,
        _client: Client,
    }

    fn coding(working: &[Backend]) -> Coding {
        coding_on(working, ready_gfx(), None)
    }

    /// [`coding`] on `gfx`, `lost` failing every frame.
    fn coding_on(
        working: &[Backend],
        (gfx, client): (Gfx, Client),
        lost: Option<Backend>,
    ) -> Coding {
        let (_frames, frames_rx) = unbounded_channel();
        let (_sizes, sizes_rx) = unbounded_channel();
        let (coded, modes) = (Coded::default(), Modes::default());
        let (log, mode_log) = (coded.clone(), modes.clone());
        let switch = move || {
            Switch::new(Box::new(move |backend, quality| {
                Ok(Box::new(Coder {
                    backend,
                    quality,
                    avc444: false,
                    coded: log.clone(),
                    modes: mode_log.clone(),
                    lost: Some(backend) == lost,
                }))
            }))
        };
        let path = H264 {
            gfx,
            offer: Offer::detached(),
            picture: picture(working),
            modifiers: vec![DRM_FORMAT_MOD_LINEAR],
            on: false,
        };
        let (links, config, status) = links();
        let live = Live::new(
            frames_rx,
            sizes_rx,
            Told::new(Arc::new(Mutex::new(DesktopSize {
                width: 4,
                height: 2,
            }))),
            Worker::spawn(switch).unwrap(),
            Some(path),
            SharedMoves::default(),
            links,
        );
        Coding {
            live,
            coded,
            modes,
            config,
            status,
            _client: client,
        }
    }

    /// Hand `frame` to the worker as soon as the connection would and send what comes back.
    async fn code(live: &mut Live, frame: Frame) {
        live.take(frame);
        live.next_at = Instant::now();
        live.start(Instant::now());
        assert!(live.busy, "the frame went to the worker");
        let done = live.worker.done.recv().await.unwrap();
        live.finish(done);
    }

    #[tokio::test]
    async fn a_quality_change_recodes_a_static_screen() {
        let mut c = coding(&[Backend::X264]);
        code(&mut c.live, frame((4, 2), SMALL)).await;
        c.config.send_modify(|c| c.quality = Quality::Sharp);
        c.live.follow_config();
        c.live.next_at = Instant::now();
        c.live.start(Instant::now());
        assert!(c.live.busy, "the last frame goes again with no new one");
        let done = c.live.worker.done.recv().await.unwrap();
        c.live.finish(done);
        assert_eq!(
            *c.coded.lock().unwrap(),
            [(Backend::X264, true, 25), (Backend::X264, true, 21)],
            "balanced, then a sharp keyframe"
        );
    }

    #[tokio::test]
    async fn quality_picks_avc444_only_when_offered() {
        use ironrdp_egfx::pdu::{CapabilitiesV107Flags, CapabilitySet};
        let v10_7 = CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::empty(),
        };
        for (offered, gfx) in [(false, ready_gfx()), (true, opened_gfx(v10_7))] {
            let mut c = coding_on(&[Backend::X264], gfx, None);
            for quality in [Quality::Balanced, Quality::Speed, Quality::Sharp] {
                c.config.send_modify(|c| c.quality = quality);
                c.live.follow_config();
                code(&mut c.live, frame((4, 2), SMALL)).await;
            }
            assert_eq!(
                *c.modes.lock().unwrap(),
                [offered, false, offered],
                "balanced and sharp code AVC444 when the client offers it, speed never"
            );
        }
    }

    #[tokio::test]
    async fn a_live_encoder_change_reopens_on_the_next_frame() {
        let mut c = coding(&[VAAPI, Backend::X264]);
        assert_eq!(c.status.borrow().encoder, "VA-API H.264");
        code(&mut c.live, dmabuf(DRM_FORMAT_MOD_LINEAR)).await;
        code(&mut c.live, dmabuf(DRM_FORMAT_MOD_LINEAR)).await;
        c.config
            .send_modify(|c| c.encoder = EncoderChoice::Software);
        c.live.follow_config();
        assert_eq!(c.status.borrow().encoder, "Software H.264");
        code(&mut c.live, frame((4, 2), SMALL)).await;
        assert_eq!(
            *c.coded.lock().unwrap(),
            [
                (VAAPI, true, 25),
                (VAAPI, false, 25),
                (Backend::X264, true, 25)
            ],
            "the first frame after the change is a keyframe from x264"
        );
        assert!(c.live.h264.as_ref().unwrap().on, "H.264 goes on");

        c.config.send_modify(|c| c.encoder = EncoderChoice::None);
        c.live.follow_config();
        assert_eq!(c.status.borrow().encoder, "RemoteFX");
        assert!(
            !c.live.h264.as_ref().unwrap().on,
            "the surface closes at once"
        );
        code(&mut c.live, frame((4, 2), SMALL)).await;
        assert_eq!(c.coded.lock().unwrap().len(), 3, "no H.264 with none");
        assert_eq!(
            bitmap_size(c.live.queue.pop_front()),
            (4, 2),
            "a whole bitmap"
        );

        c.config.send_modify(|c| c.encoder = EncoderChoice::Auto);
        c.live.follow_config();
        code(&mut c.live, dmabuf(DRM_FORMAT_MOD_LINEAR)).await;
        assert_eq!(c.coded.lock().unwrap()[3], (VAAPI, true, 25));
        assert!(c.live.h264.as_ref().unwrap().on, "the surface opens again");
        assert_eq!(c.status.borrow().encoder, "VA-API H.264");
    }

    const Y_TILED: u64 = 0x0100_0000_0000_0002;

    /// A frame in a dma-buf of `modifier`, which the test encoders never read.
    fn dmabuf(modifier: u64) -> Frame {
        let mut frame = frame((4, 2), SMALL);
        let fd = std::fs::File::open("/dev/null").unwrap();
        frame.buffer = FrameBuffer::Dmabuf {
            objects: vec![Arc::new(fd.into())],
            fourcc: stream::XRGB8888,
            modifier,
            planes: Vec::new(),
        };
        frame
    }

    #[tokio::test]
    async fn a_frame_another_encoder_cannot_read_waits_for_one_it_can() {
        let mut c = coding(&[Backend::X264]);
        c.live.take(dmabuf(Y_TILED));
        c.live.start(Instant::now());
        assert!(!c.live.busy, "x264 is not handed a tiled buffer");
        assert!(c.live.pending.is_some(), "it waits for the next frame");
        code(&mut c.live, frame((4, 2), SMALL)).await;
        assert_eq!(
            *c.coded.lock().unwrap(),
            [(Backend::X264, true, 25)],
            "the next readable frame goes out"
        );
        assert!(c.live.unreadable.is_none());
    }

    #[tokio::test]
    async fn a_hold_that_never_resolves_ends_in_the_next_encoder_within_two_seconds() {
        crate::listen::tests::capture_log();
        let modifier = 0x0100_0000_0000_0042;
        let mut c = coding(&[VAAPI, Backend::X264]);
        c.live.take(dmabuf(modifier));
        let t0 = Instant::now();
        let step = |live: &mut Live, ms: u64| {
            live.start(t0 + Duration::from_millis(ms));
            assert!(!live.busy, "nothing reads a tiled buffer");
        };
        for ms in [0, 500, 1000, 1500] {
            step(&mut c.live, ms);
        }
        assert_eq!(c.live.using, Some(VAAPI), "VA-API keeps it for two seconds");
        step(&mut c.live, 2000);
        assert_eq!(c.live.using, Some(Backend::X264));
        assert_eq!(c.status.borrow().encoder, "Software H.264");
        assert_eq!(c.live.order, Some(vec![Backend::X264]));
        assert!(c.live.keyframe, "x264 starts on a keyframe");
        let asked = "VA-API H.264 cannot read the stream's buffers (modifier 0x100000000000042); \
                     asking the stream again";
        assert_eq!(crate::listen::tests::times_logged(asked), 1);
        assert_eq!(
            crate::listen::tests::times_logged(
                "VA-API H.264 still cannot read modifier 0x100000000000042, \
                 using Software H.264 instead"
            ),
            1
        );

        for ms in [2500, 3000, 3500, 4000, 4500] {
            step(&mut c.live, ms);
        }
        assert_eq!(c.live.using, None, "bitmaps after the last encoder");
        assert!(!c.live.h264.as_ref().unwrap().on, "the surface closes");
        assert_eq!(c.status.borrow().encoder, choice::BITMAP_LABEL);
        code(&mut c.live, frame((4, 2), SMALL)).await;
        assert!(c.coded.lock().unwrap().is_empty(), "no H.264 was coded");
        assert_eq!(
            bitmap_size(c.live.queue.pop_front()),
            (4, 2),
            "a whole bitmap"
        );
    }

    #[tokio::test]
    async fn a_va_error_on_a_tiled_frame_goes_on_with_x264_once_the_stream_is_linear() {
        crate::listen::tests::capture_log();
        let mut c = coding_on(&[VAAPI, Backend::X264], ready_gfx(), Some(VAAPI));
        c.live.h264.as_mut().unwrap().picture = Arc::new(Picture {
            device: PathBuf::from("/dev/null"),
            working: vec![
                (VAAPI, vec![DRM_FORMAT_MOD_LINEAR, Y_TILED]),
                (Backend::X264, vec![DRM_FORMAT_MOD_LINEAR]),
            ],
        });
        c.live.choose(EncoderChoice::Auto);
        code(&mut c.live, dmabuf(Y_TILED)).await;
        assert_eq!(c.live.using, Some(Backend::X264));
        assert_eq!(c.status.borrow().encoder, "Software H.264");
        assert_eq!(
            c.live.h264.as_ref().unwrap().modifiers,
            [DRM_FORMAT_MOD_LINEAR],
            "the stream is offered LINEAR"
        );
        assert!(
            c.coded.lock().unwrap().is_empty(),
            "x264 got no tiled buffer"
        );
        c.live.next_at = Instant::now();
        c.live.start(Instant::now());
        assert!(!c.live.busy, "the tiled frame waits for a LINEAR one");
        code(&mut c.live, dmabuf(DRM_FORMAT_MOD_LINEAR)).await;
        assert_eq!(
            *c.coded.lock().unwrap(),
            [(Backend::X264, true, 25)],
            "x264 codes the first LINEAR frame, a keyframe"
        );
        assert_eq!(
            crate::listen::tests::times_logged(
                "VA-API H.264 failed, using Software H.264 instead: the VA context was lost"
            ),
            1
        );
        assert_eq!(
            crate::listen::tests::times_logged("encoding a frame: the VA context was lost"),
            0,
            "one line for the switch"
        );
    }

    #[tokio::test]
    async fn a_shared_memory_frame_waits_on_a_gpu_encoder() {
        let mut c = coding(&[VAAPI]);
        c.live.take(frame((4, 2), SMALL));
        c.live.start(Instant::now());
        assert!(!c.live.busy, "VA-API is not handed shared memory");
        code(&mut c.live, dmabuf(DRM_FORMAT_MOD_LINEAR)).await;
        assert_eq!(*c.coded.lock().unwrap(), [(VAAPI, true, 25)]);
    }

    #[tokio::test]
    async fn bitmaps_ask_the_stream_for_linear_every_hold() {
        crate::listen::tests::capture_log();
        let mut c = coding(&[Backend::X264]);
        c.config.send_modify(|c| c.encoder = EncoderChoice::None);
        c.live.follow_config();
        c.live.take(dmabuf(0x0100_0000_0000_0043));
        let t0 = Instant::now();
        for ms in (0..=3500).step_by(500) {
            c.live.start(t0 + Duration::from_millis(ms));
            assert!(!c.live.busy);
        }
        assert_eq!(c.live.unreadable, Some((t0, 3)), "asked at 1, 2 and 3 s");
        assert_eq!(
            crate::listen::tests::times_logged(
                "The bitmap encoder cannot read the stream's buffers \
                 (modifier 0x100000000000043); asking the stream again"
            ),
            1,
            "logged once"
        );
    }

    #[test]
    fn merged_damage_past_the_rect_limit_is_its_bounds() {
        let rects = |x0: u32| -> Vec<Rect> {
            (0..9)
                .map(|i| Rect {
                    x: x0 + i,
                    y: i,
                    width: 1,
                    height: 1,
                })
                .collect()
        };
        let with = |damage| Frame::detached(vec![0; 64 * 64 * 4], (64, 64), damage);
        let merged = merge(with(rects(0)), with(rects(30)));
        assert_eq!(
            merged.damage,
            [Rect {
                x: 0,
                y: 0,
                width: 39,
                height: 9,
            }]
        );
        let few = merge(with(rects(0)[..8].to_vec()), with(rects(30)[..8].to_vec()));
        assert_eq!(few.damage.len(), 16, "up to the limit each rectangle stays");
    }

    #[tokio::test]
    async fn a_client_without_avc420_shows_bitmaps_until_it_disconnects() {
        let (_frames, frames_rx) = unbounded_channel();
        let (_sizes, sizes_rx) = unbounded_channel();
        let (links, _config, status) = links();
        let path = H264 {
            gfx: Gfx::default(),
            offer: Offer::detached(),
            picture: picture(&[Backend::X264]),
            modifiers: vec![DRM_FORMAT_MOD_LINEAR],
            on: false,
        };
        let mut live = Live::new(
            frames_rx,
            sizes_rx,
            Told::new(Arc::new(Mutex::new(DesktopSize {
                width: 4,
                height: 2,
            }))),
            Worker::spawn(Switch::bitmaps).unwrap(),
            Some(path),
            SharedMoves::default(),
            links,
        );
        code(&mut live, frame((4, 2), SMALL)).await;
        assert_eq!(bitmap_size(live.queue.pop_front()), (4, 2));
        assert_eq!(status.borrow().encoder, choice::BITMAP_LABEL);
        drop(live);
        assert_eq!(
            status.borrow().encoder,
            "Software H.264",
            "the configured encoder once the client is gone"
        );
    }

    #[tokio::test]
    async fn a_frame_rate_change_sets_the_interval() {
        let (mut live, _told, frames, _sizes) = live(DesktopSize {
            width: 4,
            height: 2,
        });
        let (links, config, _status) = links();
        live.config = links.config;
        assert_eq!(live.interval, frame_interval(60));
        config.send_modify(|c| c.frame_rate = 30);
        frames.send(frame((4, 2), SMALL)).unwrap();
        bitmap_size(live.next().await);
        assert_eq!(live.interval, frame_interval(30));
    }

    /// A connection with H.264 on `gfx`, first told [`UNSHARED`].
    fn opening(gfx: &Gfx) -> (Live, Frames, Sizes) {
        let (frames, frames_rx) = unbounded_channel();
        let (sizes, sizes_rx) = unbounded_channel();
        let path = H264 {
            gfx: gfx.clone(),
            offer: Offer::detached(),
            picture: picture(&[Backend::X264]),
            modifiers: vec![DRM_FORMAT_MOD_LINEAR],
            on: false,
        };
        let live = Live::new(
            frames_rx,
            sizes_rx,
            Told::new(Arc::new(Mutex::new(UNSHARED))),
            Worker::spawn(Switch::bitmaps).unwrap(),
            Some(path),
            SharedMoves::default(),
            links().0,
        );
        (live, frames, sizes)
    }

    fn is_resize(update: Option<DisplayUpdate>) -> bool {
        matches!(
            update,
            Some(DisplayUpdate::Resize(DesktopSize {
                width: 4,
                height: 2
            }))
        )
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_resize_waits_for_the_graphics_pipeline_to_open() {
        use ironrdp_dvc::DvcProcessor;
        use ironrdp_egfx::pdu::{CapabilitiesAdvertisePdu, CapabilitiesV81Flags, CapabilitySet};
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.open();
        let (mut live, frames, _sizes) = opening(&gfx);
        let start = tokio::time::Instant::now();
        frames.send(frame((4, 2), SMALL)).unwrap();
        let early = tokio::time::timeout(Duration::from_millis(100), live.next()).await;
        assert!(
            early.is_err(),
            "no resize while the client opens the pipeline"
        );
        let advertise = ironrdp_egfx::pdu::GfxPdu::CapabilitiesAdvertise(
            CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            }]),
        );
        let payload = ironrdp_core::encode_vec(&advertise).unwrap();
        server.lock().unwrap().process(0, &payload).unwrap();
        assert!(
            is_resize(live.next().await),
            "the resize follows the opening"
        );
        assert!(start.elapsed() < crate::gfx::NEGOTIATION);
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_resize_waits_at_most_the_negotiation_time() {
        let gfx = Gfx::default();
        let (_bridge, _server) = gfx.open();
        let (mut live, frames, _sizes) = opening(&gfx);
        let start = tokio::time::Instant::now();
        frames.send(frame((4, 2), SMALL)).unwrap();
        assert!(is_resize(live.next().await));
        assert!(start.elapsed() >= crate::gfx::NEGOTIATION);
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_resize_repaints_no_frame_of_the_new_size() {
        let gfx = Gfx::default();
        let (_bridge, _server) = gfx.open();
        let (mut live, _frames, _sizes) = opening(&gfx);
        live.take(frame((4, 2), SMALL));
        assert_eq!(live.held_resize, Some((4, 2)));
        live.finish(Done {
            epoch: live.epoch,
            encoded: Err(crate::encode::EncodeError("lost".into())),
            choice: live.choices,
            using: live.using,
        });
        assert!(live.pending.is_none(), "a failed frame repaints nothing");
        live.activate();
        assert!(live.pending.is_none(), "an activation repaints nothing");
    }

    #[tokio::test]
    async fn a_fallback_shows_in_the_encoder_status() {
        let (_frames, frames_rx) = unbounded_channel();
        let (_sizes, sizes_rx) = unbounded_channel();
        let (links, _config, status) = links();
        links
            .status
            .send_modify(|s| s.encoder = choice::BITMAP_LABEL.into());
        let path = H264 {
            gfx: Gfx::default(),
            offer: Offer::detached(),
            picture: picture(&[Backend::X264]),
            modifiers: vec![DRM_FORMAT_MOD_LINEAR],
            on: false,
        };
        let mut live = Live::new(
            frames_rx,
            sizes_rx,
            Told::new(Arc::new(Mutex::new(UNSHARED))),
            Worker::spawn(Switch::bitmaps).unwrap(),
            Some(path),
            SharedMoves::default(),
            links,
        );
        assert_eq!(
            status.borrow().encoder,
            "Software H.264",
            "a connection with H.264 shows it again"
        );
        live.finish(Done {
            epoch: live.epoch,
            encoded: Ok(Encoded::Nothing),
            choice: live.choices,
            using: None,
        });
        assert_eq!(status.borrow().encoder, "RemoteFX (no H.264 encoder)");
    }

    #[tokio::test]
    async fn the_encoder_label_is_reset_when_the_last_connection_ends() {
        let (links, _config, status) = links();
        let start = |links: Links| {
            let path = H264 {
                gfx: Gfx::default(),
                offer: Offer::detached(),
                picture: picture(&[Backend::X264]),
                modifiers: vec![DRM_FORMAT_MOD_LINEAR],
                on: false,
            };
            Live::new(
                unbounded_channel().1,
                unbounded_channel().1,
                Told::new(Arc::new(Mutex::new(UNSHARED))),
                Worker::spawn(Switch::bitmaps).unwrap(),
                Some(path),
                SharedMoves::default(),
                links,
            )
        };
        let mut first = start(links.clone());
        let second = start(links);
        first.finish(Done {
            epoch: first.epoch,
            encoded: Ok(Encoded::Nothing),
            choice: first.choices,
            using: None,
        });
        assert_eq!(status.borrow().encoder, choice::BITMAP_LABEL);
        drop(second);
        assert_eq!(
            status.borrow().encoder,
            choice::BITMAP_LABEL,
            "the first connection still sends bitmaps"
        );
        drop(first);
        assert_eq!(status.borrow().encoder, "Software H.264");
    }

    #[test]
    fn h264_offers_the_tiled_modifiers_or_else_linear() {
        const X_TILED: u64 = 0x0100_0000_0000_0001;
        assert_eq!(
            h264_modifiers(&[DRM_FORMAT_MOD_LINEAR, X_TILED, Y_TILED]),
            vec![X_TILED, Y_TILED]
        );
        assert_eq!(
            h264_modifiers(&[DRM_FORMAT_MOD_LINEAR]),
            vec![DRM_FORMAT_MOD_LINEAR],
            "LINEAR when nothing tiled imports"
        );
    }

    #[test]
    fn a_60_fps_frame_interval_is_16_666_667_ns() {
        assert_eq!(frame_interval(60), Duration::from_nanos(16_666_667));
    }

    #[test]
    fn a_30_fps_frame_interval_is_33_333_333_ns() {
        assert_eq!(frame_interval(30), Duration::from_nanos(33_333_333));
    }

    #[tokio::test]
    async fn coalesced_damage_is_the_union() {
        let (mut live, _told, frames, _sizes) = live(DesktopSize {
            width: 4,
            height: 2,
        });
        frames.send(frame((4, 2), SMALL)).unwrap();
        bitmap_size(live.next().await);
        let wide = Rect {
            x: 2,
            y: 1,
            width: 2,
            height: 1,
        };
        frames.send(frame((4, 2), SMALL)).unwrap();
        frames.send(frame((4, 2), wide)).unwrap();
        let first = bitmap_size(live.next().await);
        assert_eq!(live.queue.len(), 1, "both rects came out of one encode");
        let mut sizes = vec![first, bitmap_size(live.next().await)];
        sizes.sort();
        assert_eq!(sizes, vec![(1, 1), (2, 1)]);
    }

    #[test]
    fn the_pointer_is_cropped_straightened_and_bottom_up() {
        // 4x3, visible only in the top-left 2x2: a half-transparent premultiplied grey over
        // an opaque red row.
        let mut bgra = vec![0u8; 4 * 3 * 4];
        bgra[..4].copy_from_slice(&[64, 64, 64, 128]);
        bgra[16..20].copy_from_slice(&[0, 0, 255, 255]);
        bgra[20..24].copy_from_slice(&[0, 0, 255, 255]);
        let ptr = rdp_pointer(&CursorImage {
            width: 4,
            height: 3,
            hotspot: (0, 0),
            bgra: bgra.into(),
        });
        assert_eq!((ptr.width, ptr.height, ptr.hot_x, ptr.hot_y), (2, 2, 0, 0));
        assert_eq!(
            ptr.data,
            vec![
                0, 0, 255, 255, 0, 0, 255, 255, // the bottom row first
                128, 128, 128, 128, 0, 0, 0, 0,
            ]
        );
    }
}
