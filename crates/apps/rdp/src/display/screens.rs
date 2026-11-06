//! The screens of one connection: their Lives, the layout and pointer they share, and their end.

use super::Live;
use crate::layout::{Desk, Pairing, ScreenRect, SharedDesk};
use crate::listen::Ending;
use crate::stream::CursorImage;
use ironrdp_server::DisplayUpdate;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Runs once every one of `screens` streams went, so the connection ends as its screens gone, and
/// not while one still shows.
pub(super) fn gone_with_all(ending: Ending, screens: usize) -> impl Fn() + Clone + Send + 'static {
    let left = Arc::new(AtomicUsize::new(screens));
    move || {
        if left.fetch_sub(1, Ordering::AcqRel) == 1 {
            ending.screen_gone();
        }
    }
}

/// Each shared screen's [`Live`], one stream each; the connection goes on while any streams.
pub(super) struct Lives {
    screens: Vec<Live>,
    /// A lock-mode grant's: its updates end only once the connection's run releases them.
    held: Option<Ending>,
}

impl Lives {
    pub(super) fn new(screens: Vec<Live>, held: Option<Ending>) -> Lives {
        Lives { screens, held }
    }

    pub(super) fn activate(&mut self) {
        self.screens.iter_mut().for_each(Live::activate);
    }

    /// The next update of any screen; None once every screen's stream ended, and a held run
    /// released them. Cancel safe, as [`Live::next`] is.
    pub(super) async fn next(&mut self) -> Option<DisplayUpdate> {
        loop {
            // Each turn starts at the next screen, so a busy one cannot keep the others waiting.
            self.screens.rotate_left(1);
            let running: Vec<_> = (self.screens.iter_mut())
                .filter(|live| !live.ended)
                .map(|live| Box::pin(live.next()))
                .collect();
            if running.is_empty() {
                if let Some(ending) = &self.held {
                    ending.released().await;
                }
                return None;
            }
            let (update, _, _) = futures_util::future::select_all(running).await;
            if update.is_some() {
                return update;
            }
        }
    }
}

/// What the client shows of the pointer.
#[derive(PartialEq, Default)]
pub(super) enum Pointer {
    #[default]
    Unknown,
    Hidden,
    Shown(CursorImage),
}

/// The pointer the client shows, which every screen of a connection updates.
#[derive(Default)]
pub(super) struct Seat {
    pub(super) pointer: Pointer,
    /// The screen whose stream last had the pointer over it.
    pub(super) over: Option<usize>,
    /// The screen and its stream pixel the pointer was last sent at.
    pub(super) position: Option<(usize, (i32, i32))>,
}

/// What the screens of one connection share: where each lands on the client, and its pointer.
#[derive(Clone)]
pub(super) struct Joint {
    pub(super) desk: SharedDesk,
    /// Counts layout changes, so the other screens paint their surfaces again.
    pub(super) relaid: Arc<watch::Sender<u64>>,
    pub(super) seat: Arc<Mutex<Seat>>,
}

impl Joint {
    pub(super) fn new(desk: Desk) -> Joint {
        Joint {
            desk: Arc::new(Mutex::new(desk)),
            relaid: Arc::new(watch::Sender::new(0)),
            seat: Arc::default(),
        }
    }

    /// The client's monitors are `client` now; the screens lay out again when that moved them.
    pub(super) fn set_client(&self, client: &[ironrdp_pdu::gcc::Monitor]) {
        if self.desk.lock().unwrap().set_client(client) {
            self.relaid.send_modify(|n| *n += 1);
        }
    }

    /// One screen that has not streamed yet, and no client monitors.
    pub(super) fn alone() -> Joint {
        let unknown = ScreenRect {
            position: (0, 0),
            logical: (0, 0),
            pixels: (0, 0),
        };
        Joint::new(Desk::new(vec![unknown], &[], Pairing::ByPlace))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{
        Frames, SMALL, Sizes, VAAPI, bitmap_size, code, frame, links, picture, ready_gfx,
    };
    use crate::display::*;
    use crate::stream::Rect;
    use futures_util::FutureExt;
    use tokio::sync::mpsc::unbounded_channel;

    /// Codes an empty access unit over its whole target, noting each frame's backend and target.
    struct Fitted {
        backend: Backend,
        target: Option<(u32, u32)>,
        fits: Fits,
    }

    type Fits = Arc<Mutex<Vec<(Backend, Option<(u32, u32)>)>>>;

    impl Encoder for Fitted {
        fn encode(
            &mut self,
            frame: &Frame,
            _: bool,
        ) -> Result<Encoded, crate::encode::EncodeError> {
            self.fits.lock().unwrap().push((self.backend, self.target));
            Ok(Encoded::Avc420 {
                data: vec![0, 0, 0, 1, 0x65],
                regions: vec![Rect::full(self.target.unwrap_or(frame.size))],
            })
        }

        fn set_target(&mut self, target: Option<(u32, u32)>) {
            self.target = target;
        }
    }

    /// A switch of [`Fitted`] encoders noting into `fits`; `refused` will not open.
    fn fitted(fits: &Fits, refused: Option<Backend>) -> impl FnOnce() -> Switch + Send + use<> {
        let fits = fits.clone();
        move || {
            Switch::new(Box::new(move |backend, _, _| {
                match Some(backend) == refused {
                    true => Err(crate::encode::EncodeError(
                        "no encoding session left".into(),
                    )),
                    false => Ok(Box::new(Fitted {
                        backend,
                        target: None,
                        fits: fits.clone(),
                    })),
                }
            }))
        }
    }

    /// One connection's screens, each with the sender of its frames.
    struct Screens {
        lives: Vec<Live>,
        frames: Vec<Frames>,
        _sizes: Vec<Sizes>,
        told: ToldSize,
        joint: Joint,
    }

    /// The Lives of `screens` for a client that sent `client` and was told `told`, H.264 on
    /// `gfx` with `working` when given, each screen coding with the switch `switch` makes for it.
    fn screens<S: FnOnce() -> Switch + Send + 'static>(
        screens: &[ScreenRect],
        client: &[ironrdp_pdu::gcc::Monitor],
        told: (u16, u16),
        gfx: Option<(&Gfx, &[Backend])>,
        switch: impl Fn(usize) -> S,
    ) -> Screens {
        let joint = Joint::new(Desk::new(screens.to_vec(), client, Pairing::InOrder));
        let told = Told::new(Arc::new(Mutex::new(DesktopSize {
            width: told.0,
            height: told.1,
        })));
        let links = links().0;
        let (mut lives, mut frames, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
        for screen in 0..screens.len() {
            let (frames_tx, frames_rx) = unbounded_channel();
            let (sizes_tx, sizes_rx) = unbounded_channel();
            let path = gfx.map(|(gfx, working)| H264 {
                gfx: gfx.clone(),
                offer: Offer::detached(),
                picture: picture(working),
                modifiers: vec![DRM_FORMAT_MOD_LINEAR],
                on: false,
            });
            let live = Live::new(
                frames_rx,
                sizes_rx,
                told.clone(),
                Worker::spawn(switch(screen)).unwrap(),
                path,
                SharedMoves::default(),
                links.clone(),
            );
            lives.push(live.on(&joint, screen));
            frames.push(frames_tx);
            sizes.push(sizes_tx);
        }
        Screens {
            lives,
            frames,
            _sizes: sizes,
            told: told.size,
            joint,
        }
    }

    fn screen(x: i32, y: i32, w: u32, h: u32) -> ScreenRect {
        ScreenRect {
            position: (x, y),
            logical: (w, h),
            pixels: (w, h),
        }
    }

    /// A primary 800x400 client monitor with a 400x200 one to its right.
    fn two_monitors() -> Vec<ironrdp_pdu::gcc::Monitor> {
        use ironrdp_pdu::gcc::{Monitor, MonitorFlags};
        vec![
            Monitor {
                left: 0,
                top: 0,
                right: 799,
                bottom: 399,
                flags: MonitorFlags::PRIMARY,
            },
            Monitor {
                left: 800,
                top: 0,
                right: 1199,
                bottom: 199,
                flags: MonitorFlags::empty(),
            },
        ]
    }

    /// A `size` frame in a LINEAR dma-buf, which the test encoders never read.
    fn linear(size: (u32, u32)) -> Frame {
        let mut frame = frame(size, Rect::full(size));
        let fd = std::fs::File::open("/dev/null").unwrap();
        frame.buffer = FrameBuffer::Dmabuf {
            objects: vec![Arc::new(fd.into())],
            fourcc: stream::XRGB8888,
            modifier: DRM_FORMAT_MOD_LINEAR,
            planes: Vec::new(),
        };
        frame
    }

    /// The surface of each frame `pdus` carry, in order.
    fn surfaces_drawn(pdus: &[ironrdp_egfx::pdu::GfxPdu]) -> Vec<u16> {
        use ironrdp_egfx::pdu::GfxPdu;
        (pdus.iter())
            .filter_map(|pdu| match pdu {
                GfxPdu::WireToSurface1(w) => Some(w.surface_id),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_picture_from_each_screen_goes_to_its_own_surface() {
        use ironrdp_egfx::pdu::GfxPdu;
        let (gfx, (_server, mut events)) = ready_gfx();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &two_monitors(),
            (1200, 400),
            Some((&gfx, &[Backend::X264])),
            |_| fitted(&fits, None),
        );
        code(&mut c.lives[1], frame((400, 200), SMALL)).await;
        code(&mut c.lives[0], frame((400, 200), SMALL)).await;
        let pdus = crate::gfx::tests::sent(&mut events);
        let made: Vec<_> = (pdus.iter())
            .filter_map(|pdu| match pdu {
                GfxPdu::CreateSurface(c) => Some((c.surface_id, c.width, c.height)),
                _ => None,
            })
            .collect();
        assert_eq!(made, [(0, 400, 200), (1, 800, 400)]);
        assert_eq!(
            surfaces_drawn(&pdus),
            [0, 1],
            "screen 1 first, then screen 0"
        );
        assert_eq!(
            *fits.lock().unwrap(),
            [(Backend::X264, None), (Backend::X264, Some((800, 400)))],
            "each screen is coded at its target, a target of its own size unscaled"
        );
        assert!(
            c.lives.iter().all(|l| l.queue.is_empty()),
            "a paired client keeps the desktop its monitors span"
        );
        assert_eq!(
            *c.told.lock().unwrap(),
            DesktopSize {
                width: 1200,
                height: 400
            }
        );
    }

    #[tokio::test]
    async fn an_odd_sized_screen_alone_is_coded_unscaled() {
        let (gfx, (_server, _events)) = ready_gfx();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 1365, 767)],
            &[],
            (1365, 767),
            Some((&gfx, &[Backend::X264])),
            |_| fitted(&fits, None),
        );
        code(&mut c.lives[0], frame((1365, 767), SMALL)).await;
        assert_eq!(
            *fits.lock().unwrap(),
            [(Backend::X264, None)],
            "its even target is no scale"
        );
    }

    #[tokio::test]
    async fn an_encoder_that_will_not_open_falls_back_for_that_screen_only() {
        let (gfx, (_server, mut events)) = ready_gfx();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &[],
            (800, 200),
            Some((&gfx, &[VAAPI, Backend::X264])),
            |screen| fitted(&fits, (screen == 1).then_some(VAAPI)),
        );
        for live in &mut c.lives {
            code(live, linear((400, 200))).await;
        }
        assert_eq!(c.lives[0].using, Some(VAAPI));
        assert_eq!(c.lives[1].using, Some(Backend::X264));
        assert!(c.lives.iter().all(|l| l.h264.as_ref().unwrap().on));
        let coded: Vec<Backend> = fits.lock().unwrap().iter().map(|f| f.0).collect();
        assert_eq!(coded, [VAAPI, Backend::X264]);
        let pdus = crate::gfx::tests::sent(&mut events);
        assert_eq!(surfaces_drawn(&pdus), [0, 1], "both screens show H.264");
    }

    #[tokio::test]
    async fn a_screen_whose_stream_ends_leaves_the_others_running() {
        let mut c = screens(
            &[screen(0, 0, 4, 2), screen(4, 0, 4, 2)],
            &[],
            (8, 2),
            None,
            |_| Switch::bitmaps,
        );
        let mut lives = Lives::new(std::mem::take(&mut c.lives), None);
        drop(c.frames.pop());
        let mut sent = 0;
        for _ in 0..3 {
            c.frames[0].send(frame((4, 2), SMALL)).unwrap();
            bitmap_size(next_of(&mut lives).await);
            sent += 1;
            assert!(
                lives.screens.iter().any(|l| l.screen == 1 && l.ended),
                "the second screen's stream ended"
            );
        }
        assert_eq!(sent, 3, "the first screen keeps sending");
        c.frames.clear();
        assert!(next_of(&mut lives).await.is_none(), "over once both ended");
    }

    #[tokio::test]
    async fn a_lock_mode_grants_screens_all_gone_end_once_its_run_releases_them() {
        let mut c = screens(&[screen(0, 0, 4, 2)], &[], (4, 2), None, |_| {
            Switch::bitmaps
        });
        let ending = Ending::default();
        let mut lives = Lives::new(std::mem::take(&mut c.lives), Some(ending.clone()));
        c.frames.clear();
        let early = tokio::time::timeout(Duration::from_millis(200), lives.next()).await;
        assert!(early.is_err(), "held while the run decides how it ends");
        ending.release();
        assert!(next_of(&mut lives).await.is_none());
    }

    async fn next_of(lives: &mut Lives) -> Option<DisplayUpdate> {
        let next = tokio::time::timeout(Duration::from_secs(5), lives.next());
        next.await.expect("an update or the end")
    }

    #[test]
    fn the_connection_ends_as_its_screens_gone_once_every_stream_went() {
        let ending = Ending::default();
        let gone = gone_with_all(ending.clone(), 2);
        gone();
        assert_eq!(
            ending.end(),
            crate::listen::End::Left,
            "one screen still streams"
        );
        gone();
        assert_eq!(ending.end(), crate::listen::End::ScreenGone);
    }

    #[tokio::test]
    async fn bitmap_clients_get_each_screen_at_its_offset() {
        let mut c = screens(
            &[screen(0, 0, 4, 2), screen(4, 0, 2, 2)],
            &[],
            (6, 2),
            None,
            |_| Switch::bitmaps,
        );
        c.frames[1].send(frame((2, 2), Rect::full((2, 2)))).unwrap();
        match c.lives[1].next().await {
            Some(DisplayUpdate::Bitmap(b)) => {
                assert_eq!((b.x, b.y, b.width.get(), b.height.get()), (4, 0, 2, 2));
            }
            other => panic!("a bitmap, not {other:?}"),
        }
        c.frames[0].send(frame((4, 2), Rect::full((4, 2)))).unwrap();
        match c.lives[0].next().await {
            Some(DisplayUpdate::Bitmap(b)) => assert_eq!((b.x, b.y), (0, 0)),
            other => panic!("a bitmap, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_paired_screen_that_falls_back_to_bitmaps_resizes_the_client_to_whole() {
        let (gfx, (_server, _events)) = ready_gfx();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &two_monitors(),
            (1200, 400),
            Some((&gfx, &[Backend::X264])),
            |screen| fitted(&fits, (screen == 1).then_some(Backend::X264)),
        );
        code(&mut c.lives[0], frame((400, 200), SMALL)).await;
        code(&mut c.lives[1], frame((400, 200), SMALL)).await;
        assert_eq!(c.lives[1].using, None, "screen 1 has no H.264 left");
        let whole = DesktopSize {
            width: 800,
            height: 200,
        };
        assert!(
            matches!(
                c.lives[1].queue.pop_front(),
                Some(DisplayUpdate::Resize(size)) if size == whole
            ),
            "the client is resized to every screen at its own pixels, no bitmap first"
        );
        assert!(c.lives[1].queue.is_empty());
        assert_eq!(*c.told.lock().unwrap(), whole);
        assert_eq!(c.joint.desk.lock().unwrap().layout().kind, Kind::Whole);
        assert!(
            c.lives[0].relaid.has_changed().unwrap(),
            "the other screen paints again"
        );
    }

    #[tokio::test]
    async fn a_paired_client_opening_the_pipeline_waits_for_it_rather_than_bitmaps() {
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.open();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &two_monitors(),
            (1200, 400),
            Some((&gfx, &[Backend::X264])),
            |_| fitted(&fits, None),
        );
        let live = &mut c.lives[0];
        live.take(frame((400, 200), SMALL));
        live.start(Instant::now());
        assert!(!live.busy, "no bitmap while the client opens the pipeline");
        assert!(live.pending.is_some(), "the frame waits");
        assert_eq!(c.joint.desk.lock().unwrap().layout().kind, Kind::Paired);
        crate::gfx::tests::client_opens(&server);
        live.next_at = Instant::now();
        live.start(Instant::now());
        assert!(
            live.busy,
            "the frame goes as H.264 once the pipeline is open"
        );
    }

    #[tokio::test]
    async fn new_client_monitors_resize_the_client_to_the_desktop_they_span() {
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &two_monitors(),
            (1200, 400),
            None,
            |_| Switch::bitmaps,
        );
        let mut resized = two_monitors();
        resized[1].right = 1399;
        c.joint.set_client(&resized);
        let next = tokio::time::timeout(Duration::from_secs(5), c.lives[0].next());
        assert!(matches!(
            next.await.expect("an update"),
            Some(DisplayUpdate::Resize(DesktopSize {
                width: 1400,
                height: 400
            }))
        ));
        assert!(c.lives[1].next().now_or_never().is_none(), "told once");
    }

    #[tokio::test]
    async fn a_held_resize_another_screen_told_first_repaints_its_screen() {
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.open();
        let fits = Fits::default();
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &[],
            (400, 200),
            Some((&gfx, &[Backend::X264])),
            |_| fitted(&fits, None),
        );
        for live in &mut c.lives {
            live.take(frame((400, 200), SMALL));
            assert_eq!(live.held_resize, Some((400, 200)));
        }
        crate::gfx::tests::client_opens(&server);
        c.lives[0].opened();
        let whole = DesktopSize {
            width: 800,
            height: 200,
        };
        assert!(matches!(
            c.lives[0].queue.front(),
            Some(DisplayUpdate::Resize(size)) if *size == whole
        ));
        c.lives.iter_mut().for_each(Live::activate);
        assert!(
            c.lives[1].pending.is_none(),
            "screen 1 still holds its resize"
        );
        c.lives[1].opened();
        assert!(c.lives[1].queue.is_empty(), "the client was told already");
        assert!(
            c.lives[1].pending.is_some(),
            "screen 1 paints the frame it held back"
        );
    }

    #[tokio::test]
    async fn a_bitmap_client_with_two_monitors_is_resized_to_whole() {
        let mut c = screens(
            &[screen(0, 0, 400, 200), screen(400, 0, 400, 200)],
            &two_monitors(),
            (1200, 400),
            None,
            |_| Switch::bitmaps,
        );
        c.frames[0].send(frame((400, 200), SMALL)).unwrap();
        assert!(matches!(
            c.lives[0].next().await,
            Some(DisplayUpdate::Resize(DesktopSize {
                width: 800,
                height: 200
            }))
        ));
    }

    #[tokio::test]
    async fn the_pointer_shows_from_the_screen_it_is_over() {
        let mut c = screens(
            &[screen(0, 0, 4, 2), screen(4, 0, 4, 2)],
            &[],
            (8, 2),
            None,
            |_| Switch::bitmaps,
        );
        let image = CursorImage {
            width: 1,
            height: 1,
            hotspot: (0, 0),
            bgra: bytes::Bytes::from_static(&[255; 4]),
        };
        let at = |position, image: Option<&CursorImage>| {
            let mut f = frame((4, 2), SMALL);
            f.cursor = Some(stream::Cursor {
                position,
                image: image.cloned(),
            });
            f
        };
        c.lives[1].pointer_updates(&at((1, 1), Some(&image)));
        assert!(matches!(
            c.lives[1].queue.pop_front(),
            Some(DisplayUpdate::RGBAPointer(_))
        ));
        assert!(matches!(
            c.lives[1].queue.pop_front(),
            Some(DisplayUpdate::PointerPosition(p)) if (p.x, p.y) == (5, 1)
        ));
        c.lives[0].pointer_updates(&at((9, 1), None));
        assert!(
            c.lives[0].queue.is_empty(),
            "a screen the pointer is not over leaves it shown"
        );
        c.lives[1].pointer_updates(&at((5, 1), None));
        assert!(matches!(
            c.lives[1].queue.pop_front(),
            Some(DisplayUpdate::HidePointer)
        ));
    }
}
