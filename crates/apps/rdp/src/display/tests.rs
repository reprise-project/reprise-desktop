//! Tests of the client's picture, one screen at a time.

use super::*;
use crate::stream::Rect;
use repriseconf::remote::LaptopScreen;

#[test]
fn a_screen_that_cannot_be_shared_tells_the_client_its_display_failed_to_start() {
    let e = unshared("the screen is not shared", "no portal");
    let ServerErrorKind::ErrorInfo(ErrorInfo::ProtocolIndependentCode(code)) = *e.kind() else {
        panic!("an error the client is told: {e:?}");
    };
    assert_eq!(code, ProtocolIndependentCode::CloseStackOnDriverFailure);
}

#[test]
fn a_grant_without_a_remote_for_every_screen_fails_to_start() {
    let mut grant = crate::portal::tests::grant();
    assert_eq!(remotes(&mut grant).unwrap().len(), 1);
    let e = remotes(&mut grant).expect_err("no remote is left");
    assert!(matches!(e.kind(), ServerErrorKind::ErrorInfo(_)), "{e:?}");
}

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
pub(super) fn links() -> (Links, watch::Sender<RemoteDesktop>, watch::Receiver<Status>) {
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

pub(super) type Frames = tokio::sync::mpsc::UnboundedSender<Frame>;
pub(super) type Sizes = tokio::sync::mpsc::UnboundedSender<SizeChange>;

pub(super) fn frame(size: (u32, u32), damage: Rect) -> Frame {
    Frame::detached(vec![0; (size.0 * size.1 * 4) as usize], size, vec![damage])
}

#[test]
fn a_bitmap_past_the_desktop_is_cut_to_it() {
    let bitmap = BitmapUpdate {
        x: 4,
        y: 0,
        width: NonZeroU16::new(8).unwrap(),
        height: NonZeroU16::new(2).unwrap(),
        format: ironrdp_server::PixelFormat::BgrX32,
        data: bytes::Bytes::from(vec![0; 8 * 2 * 4]),
        stride: std::num::NonZeroUsize::new(8 * 4).unwrap(),
    };
    let cut = on_desktop(bitmap.clone(), (6, 1), (14, 2)).expect("partly on the desktop");
    assert_eq!(
        (cut.x, cut.y, cut.width.get(), cut.height.get()),
        (10, 1, 4, 1)
    );
    assert_eq!(cut.data.len(), 4 * 4);
    assert!(
        on_desktop(bitmap, (10, 0), (14, 2)).is_none(),
        "wholly past the desktop"
    );
}

pub(super) fn bitmap_size(update: Option<DisplayUpdate>) -> (u16, u16) {
    match update {
        Some(DisplayUpdate::Bitmap(b)) => (b.width.get(), b.height.get()),
        other => panic!("a bitmap, not {other:?}"),
    }
}

pub(super) const SMALL: Rect = Rect {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
};

/// A connection's display, started at [`UNSHARED`], whose client connected as `client`.
fn display(client: ClientLayout) -> Display {
    on_console(
        client,
        console::tests::configured(LaptopScreen::Show, false).0,
    )
}

/// [`display`] on `console`.
fn on_console(client: ClientLayout, console: Console) -> Display {
    let (_, feed) = input::Input::new(Default::default());
    Display::new(
        Arc::new(Mutex::new(UNSHARED)),
        feed,
        watch::channel(None).1,
        None,
        links().0,
        Connection {
            shown: Shown::default(),
            ending: Ending::default(),
            client,
            console,
        },
    )
}

#[tokio::test]
async fn display_control_is_offered_in_lock_mode_only() {
    for (laptop_screen, locker, offered) in [
        (LaptopScreen::Lock, false, true),
        (LaptopScreen::Show, false, false),
        (LaptopScreen::Lock, true, false),
    ] {
        let console = console::tests::configured(laptop_screen, locker).0;
        let mut display = on_console(ClientLayout::default(), console);
        assert_eq!(
            display.offers_display_control().await,
            offered,
            "{laptop_screen:?} with a locker set: {locker}"
        );
    }
    let console = console::tests::configured(LaptopScreen::Lock, false).0;
    let mut display = on_console(ClientLayout::default(), console);
    assert_eq!(display.monitor_count().await, 16);
}

#[tokio::test]
async fn display_control_is_offered_to_the_first_viewer_of_a_run_only() {
    let console = console::tests::configured(LaptopScreen::Lock, false).0;
    let mut first = on_console(ClientLayout::default(), console.clone());
    assert!(first.offers_display_control().await);
    first.join();
    let mut later = on_console(ClientLayout::default(), console);
    assert!(
        !later.offers_display_control().await,
        "a later viewer's layouts would be ignored"
    );
}

/// The layouts `display` passed on to its run.
fn passed(display: &mut Display) -> Vec<Vec<console::Monitor>> {
    let rx = display.layouts_rx.as_mut().unwrap();
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[tokio::test]
async fn an_empty_or_invalid_display_control_layout_is_ignored() {
    let console = console::tests::configured(LaptopScreen::Lock, false).0;
    let mut display = on_console(ClientLayout::default(), console);
    let second = |x, y| (false, x, y, 1920, 1080);
    for (monitors, what) in [
        (vec![], "no monitors"),
        (vec![second(0, 0)], "no primary"),
        (
            vec![(true, 0, 0, 1920, 1080), (true, 1920, 0, 1920, 1080)],
            "two primaries",
        ),
        (vec![(true, 100, 0, 1920, 1080)], "a primary off 0,0"),
        (
            vec![(true, 0, 0, 1920, 1080), second(1000, 0)],
            "an overlap",
        ),
        (
            vec![(true, 0, 0, 1920, 1080), second(i32::MAX - 1000, 0)],
            "a monitor apart",
        ),
    ] {
        display.request_layout(console::tests::sent(&monitors));
        assert_eq!(
            passed(&mut display),
            Vec::<Vec<_>>::new(),
            "{what} is ignored"
        );
    }
}

#[tokio::test]
async fn a_layout_with_more_monitors_than_offered_is_ignored() {
    let console = console::tests::configured(LaptopScreen::Lock, false).0;
    let mut display = on_console(ClientLayout::default(), console);
    let row = |n: i32| -> Vec<_> { (0..n).map(|i| (i == 0, i * 200, 0, 200, 200)).collect() };
    display.request_layout(console::tests::sent(&row(17)));
    assert_eq!(
        passed(&mut display),
        Vec::<Vec<_>>::new(),
        "17 of 16 offered"
    );
    display.request_layout(console::tests::sent(&row(16)));
    assert_eq!(passed(&mut display).len(), 1, "16 are passed on");
}

#[tokio::test]
async fn request_layout_returns_without_waiting() {
    let client = crate::client::scaled(crate::client::two_monitors(), vec![], Some(150));
    let console = console::tests::configured(LaptopScreen::Lock, false).0;
    let mut display = on_console(client, console);
    let layout = console::tests::layout(&[(0, 0, 1920, 1080)]);
    display.request_layout(layout);
    let sent = display.layouts_rx.as_mut().unwrap().try_recv();
    assert_eq!(
        sent.expect("the layout waits in the channel, read by nothing yet"),
        [(0, 0, 1920, 1080, 150)]
    );
}

#[tokio::test(start_paused = true)]
async fn the_first_lock_mode_viewers_display_publishes_its_client_and_follows_its_layouts() {
    let client = crate::client::connected(crate::client::two_monitors());
    let (console, _config, status) = console::tests::configured(LaptopScreen::Lock, false);
    let mut display = on_console(client, console);
    assert!(display.join(), "it locks the console");
    let published = || status.borrow().console_layout.clone();
    assert_eq!(
        published(),
        [(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)]
    );
    display.request_layout(console::tests::layout(&[(0, 0, 1920, 1080)]));
    tokio::time::sleep(console::SETTLE * 2).await;
    assert_eq!(published(), [(0, 0, 1920, 1080, 100)]);
    drop(display);
    assert!(published().is_empty(), "the run ends with its last display");
}

#[tokio::test]
async fn a_client_with_two_monitors_is_told_the_desktop_they_span() {
    let mut display = display(crate::client::connected(crate::client::two_monitors()));
    let span = DesktopSize {
        width: 4480,
        height: 1440,
    };
    assert_eq!(display.request_initial_size(span).await, span);
    assert_eq!(display.size().await, span);
}

#[tokio::test]
async fn a_client_resized_from_the_desktop_its_monitors_span_keeps_the_new_size() {
    let mut display = display(crate::client::connected(crate::client::two_monitors()));
    let span = DesktopSize {
        width: 4480,
        height: 1440,
    };
    display.request_initial_size(span).await;
    let resized = DesktopSize {
        width: 1920,
        height: 1200,
    };
    display.told.set(resized);
    assert_eq!(display.request_initial_size(resized).await, resized);
}

#[tokio::test]
async fn a_whole_client_is_told_the_last_size() {
    let one = crate::client::two_monitors()[..1].to_vec();
    let mut display = display(crate::client::connected(one));
    let asked = DesktopSize {
        width: 2560,
        height: 1440,
    };
    assert_eq!(display.request_initial_size(asked).await, UNSHARED);
}

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

pub(super) const VAAPI: Backend = Backend::Vaapi { low_power: false };

/// The probe's find: `working`, each mapping LINEAR only.
pub(super) fn picture(working: &[Backend]) -> Arc<Picture> {
    Arc::new(Picture {
        device: PathBuf::from("/dev/null"),
        working: (working.iter())
            .map(|&b| (b, vec![DRM_FORMAT_MOD_LINEAR]))
            .collect(),
    })
}

/// The server end that keeps a pipeline open, and the events it sends the connection.
pub(super) type Client = (
    ironrdp_server::GfxServerHandle,
    tokio::sync::mpsc::UnboundedReceiver<ironrdp_server::ServerEvent>,
);

/// A pipeline the client opened with AVC420.
pub(super) fn ready_gfx() -> (Gfx, Client) {
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
    let advertise =
        ironrdp_egfx::pdu::GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[
            caps,
        ]));
    let payload = ironrdp_core::encode_vec(&advertise).unwrap();
    server.lock().unwrap().process(0, &payload).unwrap();
    (gfx, (server, events_rx))
}

/// A connection sending H.264 coded by [`Coder`]s, `working` found by the probe.
struct Coding {
    live: Live,
    coded: Coded,
    modes: Modes,
    /// The frame rate each encoder was opened at.
    opened: Opened,
    config: watch::Sender<RemoteDesktop>,
    status: watch::Receiver<Status>,
    _client: Client,
}

type Opened = Arc<Mutex<Vec<u32>>>;

fn coding(working: &[Backend]) -> Coding {
    coding_on(working, ready_gfx(), None)
}

/// [`coding`] on `gfx`, `lost` failing every frame.
fn coding_on(working: &[Backend], (gfx, client): (Gfx, Client), lost: Option<Backend>) -> Coding {
    let (_frames, frames_rx) = unbounded_channel();
    let (_sizes, sizes_rx) = unbounded_channel();
    let (coded, modes) = (Coded::default(), Modes::default());
    let (log, mode_log) = (coded.clone(), modes.clone());
    let opened = Opened::default();
    let open_log = opened.clone();
    let switch = move || {
        Switch::new(Box::new(move |backend, quality, frame_rate| {
            open_log.lock().unwrap().push(frame_rate);
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
        opened,
        config,
        status,
        _client: client,
    }
}

/// Hand `frame` to the worker as soon as the connection would and send what comes back.
pub(super) async fn code(live: &mut Live, frame: Frame) {
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
async fn a_frame_rate_change_reopens_the_encoder_on_a_keyframe() {
    let mut c = coding(&[Backend::X264]);
    code(&mut c.live, frame((4, 2), SMALL)).await;
    c.config.send_modify(|c| c.frame_rate = 120);
    c.live.follow_config();
    c.live.next_at = Instant::now();
    c.live.start(Instant::now());
    assert!(c.live.busy, "the last frame goes again with no new one");
    let done = c.live.worker.done.recv().await.unwrap();
    c.live.finish(done);
    assert_eq!(*c.opened.lock().unwrap(), [60, 120]);
    assert_eq!(
        *c.coded.lock().unwrap(),
        [(Backend::X264, true, 25), (Backend::X264, true, 25)],
        "the new encoder starts on a keyframe"
    );
    assert_eq!(c.live.interval, frame_interval(120));
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
    crate::gfx::tests::client_opens(&server);
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
