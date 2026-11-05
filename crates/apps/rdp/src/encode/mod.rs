//! Encoders turn stream frames into what the client is sent. They run on their own thread, so
//! a frame on the GPU never holds up the connection.

pub mod avc444;
pub mod bitmap;
pub mod choice;
pub mod colour;
pub mod ffmpeg;
pub mod hwmap;
pub mod probe;
pub mod split;

use crate::stream::{DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Rect};
use bitmap::Bitmap;
use ironrdp_server::BitmapUpdate;
use probe::Backend;
use repriseconf::remote::Quality;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub trait Encoder {
    /// What changed in `frame`, or all of it on a `keyframe`.
    fn encode(&mut self, frame: &Frame, keyframe: bool) -> Result<Encoded, EncodeError>;

    /// The quality to code at from now on, for an encoder coded at constant QP.
    fn set_quality(&mut self, _quality: Quality) {}

    /// Code both AVC444 views from the next frame on, or AVC420 only.
    fn set_avc444(&mut self, _on: bool) {}
}

pub enum Encoded {
    /// Pictures for IronRDP's bitmap codecs (RemoteFX, planar or plain bitmaps, as the client
    /// supports them).
    Bitmap(Vec<BitmapUpdate>),
    /// One H.264 access unit in Annex-B, and the parts of the picture it changes.
    Avc420 {
        data: Vec<u8>,
        regions: Vec<Rect>,
    },
    /// One picture's AVC444 views in Annex-B, and the parts of the picture they change. The
    /// auxiliary view is left out where the main view's chroma is already the picture's.
    Avc444 {
        main: Vec<u8>,
        aux: Option<Vec<u8>>,
        regions: Vec<Rect>,
    },
    Nothing,
}

#[derive(Clone, Debug)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Builds the H.264 encoder of a backend, coding at a quality.
pub type Make = Box<dyn FnMut(Backend, Quality) -> Result<Box<dyn Encoder>, EncodeError>>;

/// H.264 from the first backend of [`Switch::select`]'s order that starts and codes, each failure
/// handing on to the next; bitmaps after the last, or when the connection takes no H.264.
pub struct Switch {
    make: Make,
    order: Vec<Backend>,
    /// The index in `order` of the backend in use; past its end once all failed.
    at: usize,
    /// `order[at]`'s encoder, built on its first frame.
    h264: Option<Box<dyn Encoder>>,
    quality: Quality,
    avc444: bool,
    bitmap: Box<dyn Encoder>,
}

impl Switch {
    /// Bitmaps until [`Switch::select`] names backends.
    pub fn new(make: Make) -> Switch {
        Switch {
            make,
            order: Vec::new(),
            at: 0,
            h264: None,
            quality: Quality::Balanced,
            avc444: false,
            bitmap: Box::new(Bitmap),
        }
    }

    /// A switch that never codes H.264.
    #[cfg(test)]
    pub fn bitmaps() -> Switch {
        Switch::new(Box::new(|_, _| Err(EncodeError("no H.264 here".into()))))
    }

    #[cfg(test)]
    pub fn with_bitmap(mut self, bitmap: Box<dyn Encoder>) -> Switch {
        self.bitmap = bitmap;
        self
    }

    /// Code H.264 with `order`'s backends from the next frame on, starting over at the first;
    /// that frame opens a new encoder, so it is an IDR.
    pub fn select(&mut self, order: Vec<Backend>) {
        self.order = order;
        self.at = 0;
        self.h264 = None;
    }

    /// `frame` as H.264 when `h264` is asked for and a backend still works, else as bitmaps.
    pub fn encode(
        &mut self,
        frame: &Frame,
        mut keyframe: bool,
        h264: bool,
    ) -> Result<Encoded, EncodeError> {
        if !h264 {
            return self.bitmap.encode(frame, keyframe);
        }
        while let Some(&backend) = self.order.get(self.at) {
            let made = match self.h264.take() {
                Some(encoder) => Ok(encoder),
                None => (self.make)(backend, self.quality).map(|mut encoder| {
                    encoder.set_avc444(self.avc444);
                    encoder
                }),
            };
            let error = match made {
                Ok(mut encoder) => match encoder.encode(frame, keyframe) {
                    Ok(encoded) => {
                        self.h264 = Some(encoder);
                        return Ok(encoded);
                    }
                    Err(e) => e,
                },
                Err(e) => e,
            };
            self.at += 1;
            keyframe = true;
            let next = self.order.get(self.at).copied();
            match next {
                Some(next) => log::warn!(
                    "{} failed, using {} instead: {error}",
                    backend.label(),
                    next.label()
                ),
                None => log::warn!("H.264 failed, sending bitmaps from now on: {error}"),
            }
            if !reads(next, frame) {
                return Err(error);
            }
        }
        self.bitmap.encode(frame, keyframe)
    }

    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
        if let Some(h264) = &mut self.h264 {
            h264.set_quality(quality);
        }
    }

    pub fn set_avc444(&mut self, on: bool) {
        self.avc444 = on;
        if let Some(h264) = &mut self.h264 {
            h264.set_avc444(on);
        }
    }

    /// The backend that codes the next H.264 frame; None for bitmaps only.
    pub fn using(&self) -> Option<Backend> {
        self.order.get(self.at).copied()
    }

    /// Every backend of the order failed.
    #[cfg(test)]
    pub fn fell_back(&self) -> bool {
        !self.order.is_empty() && self.using().is_none()
    }
}

/// Whether `backend` (None: the bitmap encoder) can be handed `frame`: the CPU reads shared
/// memory and LINEAR dma-bufs, a GPU dma-bufs only.
pub fn reads(backend: Option<Backend>, frame: &Frame) -> bool {
    match (&frame.buffer, backend.and_then(Backend::gpu)) {
        (FrameBuffer::Shm { .. }, gpu) => gpu.is_none(),
        (FrameBuffer::Dmabuf { modifier, .. }, None) => *modifier == DRM_FORMAT_MOD_LINEAR,
        (FrameBuffer::Dmabuf { .. }, Some(_)) => true,
    }
}

pub struct Job {
    pub frame: Frame,
    pub keyframe: bool,
    pub h264: bool,
    pub quality: Quality,
    /// Both AVC444 views are asked for.
    pub avc444: bool,
    /// Handed back in [`Done`], to tell a result from before an activation apart.
    pub epoch: u64,
    /// A new encoder choice's backends, for [`Switch::select`].
    pub order: Option<Vec<Backend>>,
    /// Counts encoder choices; handed back in [`Done`].
    pub choice: u64,
}

pub struct Done {
    pub epoch: u64,
    pub encoded: Result<Encoded, EncodeError>,
    pub choice: u64,
    /// The backend that codes H.264 after this frame; None once every one failed.
    pub using: Option<Backend>,
}

/// The encoding thread: [`Job`]s in, [`Done`]s out in the same order. It ends when the
/// `Worker` drops.
pub struct Worker {
    jobs: std::sync::mpsc::Sender<Job>,
    pub done: UnboundedReceiver<Done>,
}

impl Worker {
    /// Start the thread with the [`Switch`] `make` builds there, so the encoder's GPU objects
    /// stay on one thread.
    pub fn spawn(make: impl FnOnce() -> Switch + Send + 'static) -> std::io::Result<Worker> {
        let (jobs, rx) = std::sync::mpsc::channel::<Job>();
        let (tx, done) = unbounded_channel();
        std::thread::Builder::new()
            .name("reprise-rdp-encode".into())
            .spawn(move || run(make(), rx, tx))?;
        Ok(Worker { jobs, done })
    }

    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

fn run(mut switch: Switch, jobs: std::sync::mpsc::Receiver<Job>, done: UnboundedSender<Done>) {
    while let Ok(job) = jobs.recv() {
        if let Some(order) = job.order {
            switch.select(order);
        }
        switch.set_quality(job.quality);
        switch.set_avc444(job.avc444);
        let encoded = switch.encode(&job.frame, job.keyframe, job.h264);
        let finished = Done {
            epoch: job.epoch,
            encoded,
            choice: job.choice,
            using: switch.using(),
        };
        if done.send(finished).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken;

    impl Encoder for Broken {
        fn encode(&mut self, _: &Frame, _: bool) -> Result<Encoded, EncodeError> {
            Err(EncodeError("the GPU hung".into()))
        }
    }

    fn sizes(encoded: Result<Encoded, EncodeError>) -> Vec<(u16, u16)> {
        match encoded {
            Ok(Encoded::Bitmap(updates)) => updates
                .iter()
                .map(|u| (u.width.get(), u.height.get()))
                .collect(),
            _ => panic!("a bitmap"),
        }
    }

    #[test]
    fn a_va_error_falls_back_to_bitmap() {
        let mut switch = Switch::new(Box::new(|_, _| Ok(Box::new(Broken))));
        switch.select(vec![Backend::X264]);
        let damage = Rect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        };
        let frame = Frame::detached(vec![0; 4 * 2 * 4], (4, 2), vec![damage]);
        assert_eq!(
            sizes(switch.encode(&frame, false, true)),
            vec![(4, 2)],
            "the frame that failed goes whole as a bitmap"
        );
        assert!(switch.fell_back());
        assert_eq!(
            sizes(switch.encode(&frame, false, true)),
            vec![(1, 1)],
            "later frames go as bitmaps of their damage"
        );
    }

    /// Each frame a [`Recorder`] coded: its backend and whether a keyframe was asked for.
    type Coded = std::sync::Arc<std::sync::Mutex<Vec<(Backend, bool)>>>;

    /// Codes an empty access unit.
    struct Recorder(Backend, Coded);

    impl Encoder for Recorder {
        fn encode(&mut self, frame: &Frame, keyframe: bool) -> Result<Encoded, EncodeError> {
            self.1.lock().unwrap().push((self.0, keyframe));
            Ok(Encoded::Avc420 {
                data: vec![0, 0, 0, 1, 0x65],
                regions: vec![Rect::full(frame.size)],
            })
        }
    }

    #[test]
    fn a_chosen_encoder_that_fails_to_open_hands_the_frame_to_the_next() {
        crate::listen::tests::capture_log();
        let coded = Coded::default();
        let log = coded.clone();
        let mut switch = Switch::new(Box::new(move |backend, _| match backend {
            Backend::Nvenc => Err(EncodeError("no CUDA".into())),
            _ => Ok(Box::new(Recorder(backend, log.clone()))),
        }));
        switch.select(vec![Backend::Nvenc, Backend::X264]);
        let frame = Frame::detached(vec![0; 4 * 2 * 4], (4, 2), vec![Rect::full((4, 2))]);
        assert!(matches!(
            switch.encode(&frame, false, true),
            Ok(Encoded::Avc420 { .. })
        ));
        switch.encode(&frame, false, true).unwrap();
        assert_eq!(
            *coded.lock().unwrap(),
            [(Backend::X264, true), (Backend::X264, false)],
            "the frame NVENC could not take is x264's first, a keyframe"
        );
        assert_eq!(switch.using(), Some(Backend::X264));
        assert_eq!(
            crate::listen::tests::times_logged(
                "NVENC H.264 failed, using Software H.264 instead: no CUDA"
            ),
            1
        );
    }

    /// A frame in a dma-buf of `modifier`, which no encoder here reads.
    fn dmabuf(modifier: u64) -> Frame {
        let mut frame = Frame::detached(Vec::new(), (4, 2), vec![Rect::full((4, 2))]);
        let fd = std::fs::File::open("/dev/null").unwrap();
        frame.buffer = FrameBuffer::Dmabuf {
            objects: vec![std::sync::Arc::new(fd.into())],
            fourcc: crate::stream::XRGB8888,
            modifier,
            planes: Vec::new(),
        };
        frame
    }

    const VAAPI: Backend = Backend::Vaapi { low_power: false };
    const Y_TILED: u64 = 0x0100_0000_0000_0002;

    #[test]
    fn a_tiled_frame_a_gpu_failed_on_is_not_handed_to_the_cpu() {
        let coded = Coded::default();
        let log = coded.clone();
        let mut switch = Switch::new(Box::new(move |backend, _| match backend {
            VAAPI => Ok(Box::new(Broken)),
            _ => Ok(Box::new(Recorder(backend, log.clone()))),
        }));
        switch.select(vec![VAAPI, Backend::X264]);
        assert!(switch.encode(&dmabuf(Y_TILED), false, true).is_err());
        assert_eq!(
            switch.using(),
            Some(Backend::X264),
            "x264 codes the next frame"
        );
        assert!(coded.lock().unwrap().is_empty(), "x264 got no tiled buffer");
        switch
            .encode(&dmabuf(DRM_FORMAT_MOD_LINEAR), true, true)
            .unwrap();
        assert_eq!(*coded.lock().unwrap(), [(Backend::X264, true)]);

        switch.select(vec![VAAPI]);
        assert!(
            switch.encode(&dmabuf(Y_TILED), false, true).is_err(),
            "nor to the bitmap encoder"
        );
        assert!(switch.fell_back());
    }

    #[test]
    fn a_shared_memory_frame_is_for_the_cpu_only() {
        let shm = Frame::detached(vec![0; 4 * 2 * 4], (4, 2), Vec::new());
        assert!(reads(Some(Backend::X264), &shm));
        assert!(reads(None, &shm));
        assert!(!reads(Some(VAAPI), &shm));
        assert!(reads(Some(VAAPI), &dmabuf(Y_TILED)));
        assert!(!reads(Some(Backend::X264), &dmabuf(Y_TILED)));
        assert!(reads(None, &dmabuf(DRM_FORMAT_MOD_LINEAR)));
    }

    #[test]
    fn an_encoder_that_failed_to_start_falls_back_on_the_first_h264_frame() {
        let worker = Worker::spawn(|| {
            Switch::new(Box::new(|_, _| Err(EncodeError("no H.264 encoder".into()))))
        })
        .unwrap();
        let mut done = worker.done;
        let frame = |damage| Frame::detached(vec![0; 4 * 2 * 4], (4, 2), vec![damage]);
        let small = Rect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        };
        for (h264, epoch) in [(false, 1), (true, 2), (true, 3)] {
            let job = Job {
                frame: frame(small),
                keyframe: false,
                h264,
                quality: Quality::Balanced,
                avc444: false,
                epoch,
                order: (epoch == 1).then(|| vec![Backend::X264]),
                choice: 1,
            };
            let _ = worker.jobs.send(job);
        }
        let bitmaps_only = done.blocking_recv().unwrap();
        assert_eq!(
            bitmaps_only.using,
            Some(Backend::X264),
            "a bitmap job leaves H.264 untried"
        );
        assert_eq!(sizes(bitmaps_only.encoded), vec![(1, 1)]);
        let first = done.blocking_recv().unwrap();
        assert_eq!(first.using, None, "the first H.264 job falls back");
        assert_eq!(sizes(first.encoded), vec![(4, 2)], "as one whole bitmap");
        let later = done.blocking_recv().unwrap();
        assert_eq!(later.using, None);
        assert_eq!(sizes(later.encoded), vec![(1, 1)]);
    }
}
