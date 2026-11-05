//! H.264 through FFmpeg on a backend the probe found, at a constant QP with no B-frames: one
//! access unit out for each frame in, or both AVC444 views of it.

use super::avc444;
use super::colour;
use super::hwmap::{self, Converter, Devices};
use super::probe::Backend;
use super::split::Views;
use super::{EncodeError, Encoded, Encoder};
use crate::stream::{Frame, FrameBuffer, Rect};
use ::ffmpeg::{Dictionary, ffi, frame};
use repriseconf::remote::Quality;
use std::path::{Path, PathBuf};

/// Pictures from one IDR to the next when no keyframe is asked for.
const IDR_INTERVAL: u32 = 300;
/// With AVC444, so that a view predicts from its own previous picture. FFmpeg's VA-API H.264
/// ignores it and predicts from the previous frame only.
const AVC444_REFS: i32 = 2;

pub struct Ffmpeg {
    backend: Backend,
    devices: Option<Devices>,
    node: PathBuf,
    frame_rate: u32,
    quality: Quality,
    avc444: bool,
    /// The picture size the AVC444 views failed at, which codes AVC420 from then on.
    no_views: Option<(u32, u32)>,
    open: Option<Open>,
}

/// An encoder opened for one picture size, pixel format and quality.
struct Open {
    quality: Quality,
    input: Input,
    encoder: ::ffmpeg::encoder::video::Encoder,
    /// Pictures since the last IDR.
    since_idr: u32,
    pts: i64,
}

enum Input {
    Avc420(Converter),
    Avc444(Views),
}

impl Input {
    fn fits(&self, size: (u32, u32), fourcc: u32, avc444: bool) -> bool {
        match self {
            Input::Avc420(c) => !avc444 && c.size == size && c.fourcc == fourcc,
            Input::Avc444(v) => avc444 && v.size == size && v.fourcc == fourcc,
        }
    }
}

/// Why a frame was not coded: the AVC444 views failed, which AVC420 may still code, or the
/// encoder did.
enum Failed {
    Views(EncodeError),
    Coding(EncodeError),
}

impl From<EncodeError> for Failed {
    fn from(e: EncodeError) -> Failed {
        Failed::Coding(e)
    }
}

impl Ffmpeg {
    /// `backend` on render node `device`; the encoder opens on the first frame.
    pub fn new(
        backend: Backend,
        device: &Path,
        frame_rate: u32,
        quality: Quality,
    ) -> Result<Ffmpeg, EncodeError> {
        let devices = match backend.gpu() {
            Some(gpu) => Some(Devices::open(gpu, device)?),
            None => None,
        };
        Ok(Ffmpeg {
            backend,
            devices,
            node: device.to_path_buf(),
            frame_rate: frame_rate.max(1),
            quality,
            avc444: false,
            no_views: None,
            open: None,
        })
    }

    /// `size` is the picture's for AVC444, which codes it padded to whole macroblocks.
    fn open(&self, fourcc: u32, size: (u32, u32), avc444: bool) -> Result<Open, Failed> {
        let (input, coded) = match avc444 {
            true => {
                let views = Views::new(self.devices.as_ref(), &self.node, fourcc, size)
                    .map_err(Failed::Views)?;
                (Input::Avc444(views), avc444::view_size(size))
            }
            false => {
                let c = Converter::new(self.devices.as_ref(), fourcc, size, self.frame_rate)?;
                (Input::Avc420(c), size)
            }
        };
        let mut input = input;
        let (pixel, frames) = match &mut input {
            Input::Avc420(c) => (c.pixel(), c.frames()),
            Input::Avc444(v) => (v.pixel(), v.frames()),
        };
        let name = self.backend.codec();
        let codec = ::ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| EncodeError(format!("FFmpeg has no {name}")))?;
        let mut ctx = ::ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|e| EncodeError(format!("{name}: {e}")))?;
        ctx.set_width(coded.0);
        ctx.set_height(coded.1);
        ctx.set_format(pixel);
        ctx.set_time_base((1, self.frame_rate as i32));
        ctx.set_frame_rate(Some((self.frame_rate as i32, 1)));
        // Our own IDR comes first: with AVC444 a picture is up to two frames.
        ctx.set_gop(if avc444 {
            2 * IDR_INTERVAL + 1
        } else {
            IDR_INTERVAL
        });
        ctx.set_max_b_frames(0);
        ctx.set_colorspace(::ffmpeg::color::Space::BT709);
        ctx.set_color_range(colour::RANGE);
        // SAFETY: fields of a context not yet opened; it takes its own frames reference.
        unsafe {
            let c = ctx.as_mut_ptr();
            (*c).color_primaries = ffi::AVColorPrimaries::BT709;
            (*c).color_trc = ffi::AVColorTransferCharacteristic::BT709;
            // As many threads as the encoder wants; x264 slices a frame across them.
            (*c).thread_count = 0;
            if avc444 {
                (*c).refs = AVC444_REFS;
            }
            if let Some(frames) = frames {
                (*c).hw_frames_ctx = ffi::av_buffer_ref(frames);
            }
        }
        let encoder = ctx
            .open_with(options(self.backend, self.quality, avc444))
            .map_err(|e| EncodeError(format!("{name}: {e}")))?;
        Ok(Open {
            quality: self.quality,
            input,
            encoder,
            since_idr: 0,
            pts: 0,
        })
    }

    fn code(&mut self, frame: &Frame, keyframe: bool, avc444: bool) -> Result<Encoded, Failed> {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        let size = match avc444 {
            true => frame.size,
            false => hwmap::even(frame.size),
        };
        let regions = |n| match avc444 {
            true => avc444::regions(frame.size, n, &frame.damage),
            false => regions(frame.size, n, &frame.damage),
        };
        let current =
            |o: &&mut Open| o.input.fits(size, fourcc, avc444) && o.quality == self.quality;
        let n = match self.open.as_mut().filter(current) {
            Some(o) if !keyframe && o.since_idr + 1 < IDR_INTERVAL => o.since_idr + 1,
            _ => 0,
        };
        if regions(n).is_empty() {
            return Ok(Encoded::Nothing);
        }
        if self.open.as_mut().filter(current).is_none() {
            self.open = None;
            self.open = Some(self.open(fourcc, size, avc444)?);
        }
        let codec = self.backend.codec();
        let open = self.open.as_mut().expect("opened above");
        let mut next = |input, idr| send(&mut open.encoder, input, idr, &mut open.pts, codec);
        let coded = match &mut open.input {
            Input::Avc420(converter) => {
                let (data, key) = next(converter.convert(frame)?, n == 0)?;
                let since = if key { 0 } else { n };
                (
                    since,
                    Encoded::Avc420 {
                        data,
                        regions: regions(since),
                    },
                )
            }
            Input::Avc444(views) => {
                // The regions of the picture planned, which the auxiliary view is drawn for;
                // an IDR the encoder makes on its own still decodes them right.
                let regions = regions(n);
                let read = avc444::read_rect(&regions);
                let split = views.split(frame, read).map_err(Failed::Views)?;
                let (main, key) = next(split.main, n == 0)?;
                let since = if key { 0 } else { n };
                // LC 1 applies to the bounds of the regions, undamaged blocks between them too.
                let aux = match split.flat.covers(&[read]) {
                    true => None,
                    false => Some(next(split.aux, false)?.0),
                };
                (since, Encoded::Avc444 { main, aux, regions })
            }
        };
        open.since_idr = coded.0;
        Ok(coded.1)
    }
}

/// `input` through `encoder` as the next frame, an IDR if `idr`: its access unit and whether
/// it is an IDR.
fn send(
    encoder: &mut ::ffmpeg::encoder::video::Encoder,
    mut input: frame::Video,
    idr: bool,
    pts: &mut i64,
    codec: &str,
) -> Result<(Vec<u8>, bool), EncodeError> {
    input.set_pts(Some(*pts));
    *pts += 1;
    if idr {
        input.set_kind(::ffmpeg::picture::Type::I);
    }
    let err = |e: ::ffmpeg::Error| EncodeError(format!("{codec}: {e}"));
    encoder.send_frame(&input).map_err(err)?;
    let mut data = Vec::new();
    let mut key = false;
    let mut packet = ::ffmpeg::Packet::empty();
    loop {
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                data.extend_from_slice(packet.data().unwrap_or_default());
                key |= packet.is_key();
            }
            Err(::ffmpeg::Error::Other {
                errno: libc::EAGAIN,
            }) => break,
            Err(e) => return Err(err(e)),
        }
    }
    if data.is_empty() {
        return Err(EncodeError(format!("{codec} held the frame back")));
    }
    Ok((data, key))
}

/// The encoder's private options: constant QP and its lowest-latency settings.
fn options(backend: Backend, quality: Quality, avc444: bool) -> Dictionary {
    let mut o = Dictionary::new();
    o.set("qp", quality.qp().to_string());
    let (nvenc_preset, x264_preset) = match quality {
        Quality::Speed => ("p1", "ultrafast"),
        Quality::Balanced => ("p3", "superfast"),
        Quality::Sharp => ("p5", "veryfast"),
    };
    let pairs: &[(&str, &str)] = match backend {
        Backend::Vaapi { low_power } => &[
            ("rc_mode", "CQP"),
            ("low_power", if low_power { "1" } else { "0" }),
            ("async_depth", "1"),
            ("sei", "0"),
        ],
        Backend::Nvenc => &[
            ("rc", "constqp"),
            ("preset", nvenc_preset),
            ("tune", "ull"),
            ("delay", "0"),
            ("zerolatency", "1"),
            ("forced-idr", "1"),
        ],
        Backend::Vulkan => &[
            ("rc_mode", "cqp"),
            ("async_depth", "1"),
            ("tune", "ull"),
            ("content", "desktop"),
            ("units", "0"),
        ],
        Backend::X264 => &[
            ("preset", x264_preset),
            ("tune", "zerolatency"),
            ("forced-idr", "1"),
        ],
    };
    for (k, v) in pairs {
        o.set(k, v);
    }
    if avc444 && backend == Backend::X264 {
        // Each view would look like a scene cut after the other and turn into an I picture.
        o.set("sc_threshold", "0");
    }
    o
}

impl Encoder for Ffmpeg {
    /// No backend changes its QP or preset while open, so the next frame reopens the encoder
    /// with the new quality as an IDR.
    fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
    }

    /// The next frame reopens the encoder in the other mode as an IDR. Vulkan codes AVC420
    /// only.
    fn set_avc444(&mut self, on: bool) {
        self.avc444 = on;
    }

    fn encode(&mut self, frame: &Frame, keyframe: bool) -> Result<Encoded, EncodeError> {
        // The GPU draws the views from dma-bufs only; a shared-memory frame takes the AVC420 path.
        let gpu_views = self.devices.is_some();
        let shm = matches!(frame.buffer, FrameBuffer::Shm { .. });
        let avc444 = self.avc444
            && self.backend.codes_avc444()
            && self.no_views != Some(frame.size)
            && !(gpu_views && shm);
        match self.code(frame, keyframe, avc444) {
            Ok(coded) => Ok(coded),
            Err(Failed::Views(e)) => {
                log::warn!(
                    "{} cannot code AVC444, sending AVC420: {e}",
                    self.backend.label()
                );
                self.no_views = Some(frame.size);
                self.open = None;
                self.code(frame, true, false).map_err(Failed::into_error)
            }
            Err(Failed::Coding(e)) => Err(e),
        }
    }
}

impl Failed {
    fn into_error(self) -> EncodeError {
        match self {
            Failed::Views(e) | Failed::Coding(e) => e,
        }
    }
}

/// What the client updates from picture `n` after an IDR: the whole coded picture on the IDR,
/// else the damage. Both stay inside the even size the stream codes (see [`hwmap::even`]).
fn regions(size: (u32, u32), n: u32, damage: &[Rect]) -> Vec<Rect> {
    let shown = Rect::full(hwmap::even(size));
    if n == 0 {
        return vec![shown];
    }
    damage
        .iter()
        .filter_map(|r| {
            let right = (r.x + r.width).min(shown.width);
            let bottom = (r.y + r.height).min(shown.height);
            (r.x < right && r.y < bottom).then(|| Rect {
                x: r.x,
                y: r.y,
                width: right - r.x,
                height: bottom - r.y,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::Switch;
    use crate::encode::avc444;
    use crate::encode::gl::Gl;
    use crate::encode::probe::RENDER_NODE;
    use crate::stream::XRGB8888;
    use ::ffmpeg::frame;
    use std::sync::{Arc, Mutex};

    /// DRM_FORMAT_NV12, a format the encoders don't take from the screen.
    const NV12: u32 = u32::from_le_bytes(*b"NV12");

    fn silence() {
        ::ffmpeg::util::log::set_level(::ffmpeg::util::log::Level::Quiet);
    }

    fn x264() -> Ffmpeg {
        silence();
        Ffmpeg::new(Backend::X264, Path::new(RENDER_NODE), 60, Quality::Balanced).unwrap()
    }

    /// The pixel at (`x`, `y`) of picture `t`: smooth ramps and a square that moves.
    fn rgb(x: u32, y: u32, t: u32) -> (u8, u8, u8) {
        let inside = (x.wrapping_sub(t * 4) < 16) && (8..24).contains(&y);
        match inside {
            true => (240, 240, 240),
            false => ((x * 2) as u8, (y * 3) as u8, 96),
        }
    }

    fn frame(size: (u32, u32), t: u32, damage: Vec<Rect>) -> Frame {
        let (w, h) = size;
        let data = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let (r, g, b) = rgb(x, y, t);
                [b, g, r, 0]
            })
            .collect::<Vec<u8>>();
        let buffer = FrameBuffer::Shm {
            data: data.into(),
            fourcc: XRGB8888,
            stride: w * 4,
        };
        let mut frame = Frame::unleased(buffer, size);
        frame.damage = damage;
        frame
    }

    fn luma((r, g, b): (u8, u8, u8)) -> f64 {
        colour::luma(f64::from(r), f64::from(g), f64::from(b))
    }

    fn avc(encoded: Encoded) -> (Vec<u8>, Vec<Rect>) {
        match encoded {
            Encoded::Avc420 { data, regions } => (data, regions),
            _ => panic!("an access unit"),
        }
    }

    /// FFmpeg's H.264 decoder, with each frame's QPs exported.
    struct Decoder(::ffmpeg::decoder::Video);

    impl Decoder {
        fn new() -> Decoder {
            let codec = ::ffmpeg::decoder::find(::ffmpeg::codec::Id::H264).unwrap();
            let mut ctx = ::ffmpeg::codec::Context::new_with_codec(codec);
            // SAFETY: fields of a context not yet opened.
            unsafe {
                let c = ctx.as_mut_ptr();
                (*c).export_side_data |= ffi::AV_CODEC_EXPORT_DATA_VIDEO_ENC_PARAMS;
                (*c).flags |= ffi::AV_CODEC_FLAG_LOW_DELAY as i32;
            }
            Decoder(ctx.decoder().video().unwrap())
        }

        fn decode(&mut self, data: &[u8]) -> frame::Video {
            self.0.send_packet(&::ffmpeg::Packet::copy(data)).unwrap();
            let mut out = frame::Video::empty();
            self.0
                .receive_frame(&mut out)
                .expect("each access unit decodes at once");
            out
        }
    }

    /// `struct AVVideoEncParams` and `AVVideoBlockParams` from libavutil/video_enc_params.h.
    #[repr(C)]
    struct EncParams {
        nb_blocks: u32,
        blocks_offset: usize,
        block_size: usize,
        kind: i32,
        qp: i32,
        delta_qp: [[i32; 2]; 4],
    }

    #[repr(C)]
    struct BlockParams {
        src_x: i32,
        src_y: i32,
        w: i32,
        h: i32,
        delta_qp: i32,
    }

    /// The QP of a decoded picture's first macroblock.
    fn qp(picture: &frame::Video) -> i32 {
        let side = picture
            .side_data(::ffmpeg::frame::side_data::Type::VIDEO_ENC_PARAMS)
            .expect("encoding parameters");
        let bytes = side.data();
        assert!(bytes.len() >= std::mem::size_of::<EncParams>());
        // SAFETY: the side data is an AVVideoEncParams followed by its blocks, as checked
        // against its own offsets.
        unsafe {
            let params = &*(bytes.as_ptr() as *const EncParams);
            assert!(params.nb_blocks > 0);
            assert!(params.blocks_offset + std::mem::size_of::<BlockParams>() <= bytes.len());
            let block = &*(bytes.as_ptr().add(params.blocks_offset) as *const BlockParams);
            params.qp + block.delta_qp
        }
    }

    fn psnr(picture: &frame::Video, t: u32) -> f64 {
        let (w, h) = (picture.width(), picture.height());
        let (plane, stride) = (picture.data(0), picture.stride(0));
        let mut error = 0.0;
        for y in 0..h {
            for x in 0..w {
                let d = f64::from(plane[y as usize * stride + x as usize]) - luma(rgb(x, y, t));
                error += d * d;
            }
        }
        let mse = error / f64::from(w * h);
        10.0 * (255.0 * 255.0 / mse).log10()
    }

    #[test]
    fn x264_frames_decode() {
        let mut encoder = x264();
        let mut decoder = Decoder::new();
        let size = (128, 64);
        for t in 0..8 {
            let damage = vec![Rect::full(size)];
            let (data, regions) = avc(encoder.encode(&frame(size, t, damage), t == 0).unwrap());
            assert_eq!(regions, vec![Rect::full(size)]);
            let picture = decoder.decode(&data);
            assert_eq!((picture.width(), picture.height()), size);
            let db = psnr(&picture, t);
            assert!(db > 35.0, "picture {t}: {db:.1} dB");
        }
    }

    /// The `level_idc` of the first sequence parameter set in Annex B `data`.
    fn level_idc(data: &[u8]) -> Option<u8> {
        let sps = data
            .windows(4)
            .position(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 7)?;
        data.get(sps + 6).copied()
    }

    #[test]
    fn x264_picks_the_level_for_1080p_at_the_frame_rate() {
        silence();
        let size = (1920, 1080);
        for (rate, level) in [(60, 42), (144, 52)] {
            let mut encoder = Ffmpeg::new(
                Backend::X264,
                Path::new(RENDER_NODE),
                rate,
                Quality::Balanced,
            )
            .unwrap();
            let damage = vec![Rect::full(size)];
            let (data, _) = avc(encoder.encode(&frame(size, 0, damage), true).unwrap());
            assert_eq!(level_idc(&data), Some(level), "{rate} frames a second");
        }
    }

    #[test]
    fn odd_sizes_crop() {
        let mut encoder = x264();
        let mut decoder = Decoder::new();
        let size = (65, 35);
        let (data, regions) = avc(encoder.encode(&frame(size, 0, vec![]), true).unwrap());
        assert_eq!(
            regions,
            vec![Rect::full((64, 34))],
            "the IDR updates the even size below"
        );
        let picture = decoder.decode(&data);
        assert_eq!((picture.width(), picture.height()), (64, 34));
        let edge = Rect {
            x: 60,
            y: 30,
            width: 5,
            height: 5,
        };
        let (_, regions) = avc(encoder.encode(&frame(size, 1, vec![edge]), false).unwrap());
        assert_eq!(
            regions,
            vec![Rect {
                x: 60,
                y: 30,
                width: 4,
                height: 4
            }],
            "damage is clipped to the coded picture"
        );
        let last_column = Rect {
            x: 64,
            y: 0,
            width: 1,
            height: 35,
        };
        assert!(
            matches!(
                encoder.encode(&frame(size, 2, vec![last_column]), false),
                Ok(Encoded::Nothing)
            ),
            "damage only outside the coded picture codes nothing"
        );
    }

    #[test]
    fn quality_change_reaches_the_encoder() {
        let mut encoder = x264();
        let mut decoder = Decoder::new();
        let size = (64, 32);
        let corner = Rect {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        };
        let mut frames = Vec::new();
        for t in 0..5 {
            if t == 2 {
                encoder.set_quality(Quality::Sharp);
            }
            let damage = vec![if t == 0 { Rect::full(size) } else { corner }];
            let (data, regions) = avc(encoder.encode(&frame(size, t, damage), t == 0).unwrap());
            let picture = decoder.decode(&data);
            let key = picture.kind() == ::ffmpeg::picture::Type::I;
            frames.push((
                key,
                regions == [Rect::full(size)],
                (!key).then(|| qp(&picture)),
            ));
        }
        assert_eq!(
            frames,
            [
                (true, true, None),
                (false, false, Some(25)),
                (true, true, None),
                (false, false, Some(21)),
                (false, false, Some(21)),
            ],
            "the frame after the change is a whole IDR, sharp from there on"
        );
    }

    #[test]
    fn an_encoder_that_fails_to_open_falls_back_on_the_first_frame() {
        silence();
        let vaapi = Backend::Vaapi { low_power: false };
        let mut switch = Switch::new(Box::new(|backend, quality| {
            Ffmpeg::new(backend, Path::new("/dev/null"), 60, quality)
                .map(|f| Box::new(f) as Box<dyn Encoder>)
        }));
        switch.select(vec![vaapi]);
        let damage = Rect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        };
        let first = switch.encode(&frame((4, 2), 0, vec![damage]), false, true);
        assert!(switch.fell_back());
        match first {
            Ok(Encoded::Bitmap(updates)) => {
                let sizes: Vec<_> = (updates.iter())
                    .map(|u| (u.width.get(), u.height.get()))
                    .collect();
                assert_eq!(sizes, [(4, 2)], "the first frame goes whole as a bitmap");
            }
            _ => panic!("bitmaps"),
        }
    }

    /// Records whether each bitmap frame was asked for as a keyframe.
    struct Keyframes(Arc<Mutex<Vec<bool>>>);

    impl Encoder for Keyframes {
        fn encode(&mut self, _: &Frame, keyframe: bool) -> Result<Encoded, EncodeError> {
            self.0.lock().unwrap().push(keyframe);
            Ok(Encoded::Nothing)
        }
    }

    #[test]
    fn an_encoder_that_fails_on_its_first_frame_falls_back_once() {
        crate::listen::tests::capture_log();
        let keyframes = Arc::default();
        let mut switch = Switch::new(Box::new(|_, _| Ok(Box::new(x264()))))
            .with_bitmap(Box::new(Keyframes(Arc::clone(&keyframes))));
        switch.select(vec![Backend::X264]);
        let size = (4, 2);
        let mut nv12 = frame(size, 0, vec![Rect::full(size)]);
        if let FrameBuffer::Shm { fourcc, .. } = &mut nv12.buffer {
            *fourcc = NV12;
        }
        for _ in 0..3 {
            switch.encode(&nv12, false, true).unwrap();
        }
        assert!(switch.fell_back());
        assert_eq!(
            *keyframes.lock().unwrap(),
            [true, false, false],
            "one bitmap keyframe, then damage"
        );
        let line =
            format!("H.264 failed, sending bitmaps from now on: format {NV12:#x} is not supported");
        assert_eq!(
            crate::listen::tests::times_logged(&line),
            1,
            "one journal line"
        );
    }

    /// A shared-memory XRGB8888 frame of `size` holding `pixel`'s colours, its whole picture
    /// damaged, and its 4:4:4 planes.
    fn picture(
        size: (u32, u32),
        pixel: impl Fn(u32, u32) -> (u8, u8, u8),
    ) -> (Frame, avc444::Planes444) {
        let (w, h) = size;
        let data: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let (r, g, b) = pixel(x, y);
                [b, g, r, 0]
            })
            .collect();
        let planes = avc444::Planes444::from_rgb(&data, w as usize * 4, size, XRGB8888);
        let buffer = FrameBuffer::Shm {
            data: data.into(),
            fourcc: XRGB8888,
            stride: w * 4,
        };
        let mut frame = Frame::unleased(buffer, size);
        frame.damage = vec![Rect::full(size)];
        (frame, planes)
    }

    fn views(encoded: Encoded) -> (Vec<u8>, Option<Vec<u8>>, Vec<Rect>) {
        match encoded {
            Encoded::Avc444 { main, aux, regions } => (main, aux, regions),
            _ => panic!("AVC444 views"),
        }
    }

    /// A decoded 4:2:0 picture as NV12.
    fn nv12(out: &frame::Video) -> avc444::Nv12 {
        let (width, height) = (out.width(), out.height());
        let rows = |plane: usize, w: u32, h: u32| -> Vec<u8> {
            let stride = out.stride(plane);
            (0..h as usize)
                .flat_map(|y| &out.data(plane)[y * stride..][..w as usize])
                .copied()
                .collect()
        };
        let (u, v) = (
            rows(1, width / 2, height / 2),
            rows(2, width / 2, height / 2),
        );
        avc444::Nv12 {
            y: rows(0, width, height),
            uv: u.iter().zip(&v).flat_map(|(&u, &v)| [u, v]).collect(),
            width,
            height,
        }
    }

    /// The colour a client shows from `main` alone, each 2x2 block one chroma sample.
    fn main_only(main: &avc444::Nv12, (width, height): (u32, u32)) -> avc444::Planes444 {
        let at = |x: u32, y: u32, i: u32| main.uv[(y / 2 * main.width + x / 2 * 2 + i) as usize];
        let plane = |i| {
            (0..height)
                .flat_map(|y| (0..width).map(move |x| at(x, y, i)))
                .collect()
        };
        let luma = (0..height)
            .flat_map(|y| &main.y[(y * main.width) as usize..][..width as usize])
            .copied();
        avc444::Planes444 {
            y: luma.collect(),
            u: plane(0),
            v: plane(1),
            width,
            height,
        }
    }

    fn chroma_psnr(a: &avc444::Planes444, b: &avc444::Planes444) -> f64 {
        let error: f64 = (a.u.iter().zip(&b.u))
            .chain(a.v.iter().zip(&b.v))
            .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
            .sum();
        let samples = (a.u.len() + a.v.len()) as f64;
        10.0 * (255.0 * 255.0 * samples / error).log10()
    }

    /// The mean of the larger of the U and V errors over `pixels`.
    fn colour_error(a: &avc444::Planes444, b: &avc444::Planes444, pixels: &[(u32, u32)]) -> f64 {
        let off = |&(x, y): &(u32, u32)| {
            let at = (y * a.width + x) as usize;
            a.u[at].abs_diff(b.u[at]).max(a.v[at].abs_diff(b.v[at]))
        };
        pixels.iter().map(|p| f64::from(off(p))).sum::<f64>() / pixels.len() as f64
    }

    #[test]
    fn avc444_frames_decode() {
        let size = (120, 52);
        let colours = [(255, 0, 0), (0, 160, 0), (0, 0, 255), (255, 0, 255)];
        let columns = [20, 23, 27, 30, 34, 37, 41, 44];
        let rows = [30, 33, 37, 40];
        let line = |x: u32, y: u32| match (x, y) {
            (x, 20..52) if columns.contains(&x) => Some(colours[(x % 4) as usize]),
            (50..110, y) if rows.contains(&y) => Some(colours[(y % 4) as usize]),
            _ => None,
        };
        // A colour gradient above white with coloured 1-px lines of both parities.
        let (frame, src) = picture(size, |x, y| match line(x, y) {
            Some(c) => c,
            None if y < 16 => ((x * 2) as u8, (128 + y * 4) as u8, (255 - x * 2) as u8),
            None => (250, 250, 250),
        });
        let mut psnrs = Vec::new();
        for quality in [Quality::Balanced, Quality::Sharp] {
            let mut encoder = x264();
            encoder.set_quality(quality);
            encoder.set_avc444(true);
            let mut decoder = Decoder::new();
            let (main, aux, regions) = views(encoder.encode(&frame, true).unwrap());
            assert_eq!(regions, [Rect::full(size)]);
            let aux = aux.expect("coloured lines need the auxiliary view");
            let main = nv12(&decoder.decode(&main));
            let aux = nv12(&decoder.decode(&aux));
            assert_eq!((main.width, main.height), (128, 64), "whole macroblocks");
            let back = avc444::tests::crop(&avc444::combine_v1(&main, &aux), size);
            let only_main = main_only(&main, size);

            let (full, half) = (chroma_psnr(&back, &src), chroma_psnr(&only_main, &src));
            assert!(full > 36.0, "{quality:?}: chroma PSNR {full:.1} dB");
            assert!(
                full > half + 12.0,
                "{full:.1} dB against {half:.1} dB from the main view alone"
            );
            psnrs.push(full);

            let columns_at = |x: u32| (20..52).filter(|y| !rows.contains(y)).map(move |y| (x, y));
            let rows_at = |y: u32| {
                (50..110)
                    .filter(|x| !columns.contains(x))
                    .map(move |x| (x, y))
            };
            let lines: Vec<Vec<(u32, u32)>> = (columns.iter().map(|&x| columns_at(x).collect()))
                .chain(rows.iter().map(|&y| rows_at(y).collect()))
                .collect();
            let all: Vec<(u32, u32)> = lines.concat();
            let error = colour_error(&back, &src, &all);
            assert!(
                error < 8.0,
                "the lines' colour is off by {error:.1} on average"
            );
            for (i, pixels) in lines.iter().enumerate() {
                let (full, half) = (
                    colour_error(&back, &src, pixels),
                    colour_error(&only_main, &src, pixels),
                );
                assert!(
                    full < half / 2.0,
                    "line {i} is off by {full:.1}, {half:.1} from the main view alone"
                );
            }
        }
        assert!(psnrs[1] > psnrs[0], "sharp beats balanced: {psnrs:.1?}");
    }

    #[test]
    fn luma_only_changes_skip_the_aux_view() {
        let mut encoder = x264();
        encoder.set_avc444(true);
        let mut decoder = Decoder::new();
        let size = (80, 48);
        fn grey(x: u32) -> (u8, u8, u8) {
            ((x * 3) as u8, (x * 3) as u8, (x * 3) as u8)
        }
        let coloured_column =
            |x: u32| move |cx: u32, _| if cx == x { (255, 0, 64) } else { grey(cx) };
        let (first, _) = picture(size, coloured_column(5));
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        type Paint = Box<dyn Fn(u32, u32) -> (u8, u8, u8)>;
        let steps: [(&str, Paint, Rect, bool); 3] = [
            (
                "a darker grey 1-px column",
                Box::new(|x, y| match (x, y) {
                    (5, _) => (255, 0, 64),
                    (33, 0..16) => (20, 20, 20),
                    _ => grey(x),
                }),
                rect(33, 0, 1, 16),
                false,
            ),
            (
                "a red 1-px column",
                Box::new(|x, y| match (x, y) {
                    (5, _) | (41, 20..30) => (255, 0, 64),
                    (33, 0..16) => (20, 20, 20),
                    _ => grey(x),
                }),
                rect(41, 20, 1, 10),
                true,
            ),
            (
                "a red square on whole 2x2 blocks",
                Box::new(|x, y| match (x, y) {
                    (5, _) | (41, 20..30) | (64..68, 34..38) => (255, 0, 64),
                    (33, 0..16) => (20, 20, 20),
                    _ => grey(x),
                }),
                rect(64, 34, 4, 4),
                false,
            ),
        ];
        let (main, aux, _) = views(encoder.encode(&first, true).unwrap());
        assert!(
            aux.is_some(),
            "the coloured column needs the auxiliary view"
        );
        decoder.decode(&main);
        decoder.decode(&aux.unwrap());
        for (what, pixel, damage, needs_aux) in steps {
            let (mut frame, _) = picture(size, pixel);
            frame.damage = vec![damage];
            let (main, aux, regions) = views(encoder.encode(&frame, false).unwrap());
            assert_eq!(aux.is_some(), needs_aux, "{what}");
            assert_eq!(regions.len(), 1, "{what}");
            assert_eq!(
                regions[0].x % 16 + regions[0].y % 16,
                0,
                "{what}: whole macroblocks"
            );
            let picture = decoder.decode(&main);
            assert_ne!(picture.kind(), ::ffmpeg::picture::Type::I, "{what}");
            if let Some(aux) = aux {
                decoder.decode(&aux);
            }
        }
    }

    #[test]
    fn a_coloured_line_between_two_grey_changes_needs_the_aux_view() {
        let mut encoder = x264();
        encoder.set_avc444(true);
        let size = (96, 48);
        let line = |x: u32| x == 40;
        let (first, _) = picture(size, |x, _| match line(x) {
            true => (255, 0, 64),
            false => (128, 128, 128),
        });
        assert!(views(encoder.encode(&first, true).unwrap()).1.is_some());
        let changed = |x: u32, y: u32| (x < 16 && y < 16) || (x >= 80 && y >= 32);
        let (mut next, _) = picture(size, |x, y| match (line(x), changed(x, y)) {
            (true, _) => (255, 0, 64),
            (false, true) => (60, 60, 60),
            (false, false) => (128, 128, 128),
        });
        next.damage = vec![
            Rect {
                x: 0,
                y: 0,
                width: 16,
                height: 16,
            },
            Rect {
                x: 80,
                y: 32,
                width: 16,
                height: 16,
            },
        ];
        let (_, aux, regions) = views(encoder.encode(&next, false).unwrap());
        assert_eq!(regions.len(), 2);
        assert!(
            aux.is_some(),
            "the client updates the regions' bounds, and the red line inside them needs the \
             auxiliary view"
        );
    }

    #[test]
    fn turning_avc444_on_makes_the_next_frame_an_idr() {
        let mut encoder = x264();
        encoder.set_quality(Quality::Speed);
        let size = (64, 32);
        let corner = Rect {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        };
        avc(encoder.encode(&frame(size, 0, vec![]), true).unwrap());
        avc(encoder
            .encode(&frame(size, 1, vec![corner]), false)
            .unwrap());
        encoder.set_quality(Quality::Balanced);
        encoder.set_avc444(true);
        let (main, _, regions) = views(
            encoder
                .encode(&frame(size, 2, vec![corner]), false)
                .unwrap(),
        );
        assert_eq!(regions, [Rect::full(size)], "the whole picture");
        let picture = Decoder::new().decode(&main);
        assert_eq!(
            picture.kind(),
            ::ffmpeg::picture::Type::I,
            "the main view decodes alone as an IDR"
        );
    }

    #[test]
    fn avc444_comes_back_at_a_new_size_after_the_views_failed() {
        let mut encoder = x264();
        encoder.set_avc444(true);
        let size = (64, 32);
        let mut short = frame(size, 0, vec![]);
        if let FrameBuffer::Shm { data, .. } = &mut short.buffer {
            *data = data.slice(..data.len() / 2);
        }
        assert!(encoder.encode(&short, true).is_err());
        assert!(
            matches!(
                encoder.encode(&frame(size, 1, vec![]), true),
                Ok(Encoded::Avc420 { .. })
            ),
            "AVC420 at the size the views failed at"
        );
        let other = (80, 48);
        assert!(
            matches!(
                encoder.encode(&frame(other, 2, vec![]), true),
                Ok(Encoded::Avc444 { .. })
            ),
            "AVC444 again at another size"
        );
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn a_shared_memory_frame_leaves_avc444_on_for_the_next_dmabuf() {
        silence();
        let backend = (crate::encode::probe::probe().1.into_iter())
            .map(|(b, _)| b)
            .find(|b| matches!(b, Backend::Vaapi { .. }))
            .expect("VA-API H.264 on this machine");
        let mut encoder =
            Ffmpeg::new(backend, Path::new(RENDER_NODE), 60, Quality::Balanced).unwrap();
        encoder.set_avc444(true);
        let gbm = gbm::Device::new(
            std::fs::File::options()
                .read(true)
                .write(true)
                .open(RENDER_NODE)
                .unwrap(),
        )
        .unwrap();
        let size = (256, 128);
        let pixel = |x: u32, y: u32| [x as u8, (y * 2) as u8, 200];
        let (dmabuf, _, _bo) = crate::encode::split::tests::dmabuf_frame(&gbm, size, pixel);
        let kind = |e: Result<Encoded, EncodeError>| match e {
            Ok(Encoded::Avc444 { .. }) => "AVC444".to_string(),
            Ok(Encoded::Avc420 { .. }) => "AVC420".to_string(),
            Ok(_) => "other".to_string(),
            Err(e) => e.to_string(),
        };
        assert_eq!(kind(encoder.encode(&dmabuf, true)), "AVC444");
        assert!(
            encoder.encode(&frame(size, 0, vec![]), true).is_err(),
            "VA-API cannot map a shared-memory frame for AVC420 either"
        );
        assert_eq!(
            kind(encoder.encode(&dmabuf, true)),
            "AVC444",
            "the next dma-buf frame"
        );
    }

    const COLOUR_SIZE: (u32, u32) = (1920, 1080);
    /// Black, white, three greys, the primaries and secondaries and a skin tone.
    const PATCHES: [[u8; 3]; 12] = [
        [0, 0, 0],
        [255, 255, 255],
        [128, 128, 128],
        [64, 64, 64],
        [192, 192, 192],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [224, 172, 140],
        [0, 255, 255],
        [255, 0, 255],
        [255, 255, 0],
    ];
    /// 1-px columns below the patches, at odd and even x, in the colours after the greys.
    const LINES: usize = 14;

    fn patch(i: usize) -> Rect {
        let (column, row) = (i as u32 % 4, i as u32 / 4);
        Rect {
            x: 80 + column * 440,
            y: 40 + row * 300,
            width: 400,
            height: 260,
        }
    }

    fn line(i: usize) -> (Rect, [u8; 3]) {
        let rect = Rect {
            x: 100 + 7 * i as u32,
            y: 930,
            width: 1,
            height: 120,
        };
        (rect, PATCHES[5 + i % 7])
    }

    /// The colour picture as solid rectangles on white, each over the ones before.
    fn colour_rects() -> Vec<(Rect, [u8; 3])> {
        let mut rects = vec![(Rect::full(COLOUR_SIZE), [255, 255, 255])];
        rects.extend((0..PATCHES.len()).map(|i| (patch(i), PATCHES[i])));
        rects.extend((0..LINES).map(line));
        rects
    }

    /// `rects` in an XRGB8888 dma-buf on `gl`'s device: LINEAR, else the driver's choice. A
    /// buffer the CPU cannot map is cleared rectangle by rectangle on the GPU.
    fn colour_frame(
        gl: &Gl,
        linear: bool,
        rects: &[(Rect, [u8; 3])],
    ) -> Result<(Frame, gbm::BufferObject<()>), EncodeError> {
        let (w, h) = COLOUR_SIZE;
        let err = |e: std::io::Error| EncodeError(e.to_string());
        let rendering = gbm::BufferObjectFlags::RENDERING;
        let format = gbm::Format::Xrgb8888;
        let mut bo = match linear {
            // NVIDIA allocates LINEAR only for buffers it does not render to.
            true => [rendering, gbm::BufferObjectFlags::empty()]
                .into_iter()
                .find_map(|flags| {
                    let linear = [gbm::Modifier::Linear].into_iter();
                    (gl.gbm
                        .create_buffer_object_with_modifiers2::<()>(w, h, format, linear, flags))
                    .ok()
                })
                .ok_or_else(|| EncodeError("no LINEAR buffer".into()))?,
            false => (gl.gbm.create_buffer_object::<()>(w, h, format, rendering)).map_err(err)?,
        };
        let mut pixels = vec![[0u8; 3]; (w * h) as usize];
        for (r, colour) in rects {
            for y in r.y..r.y + r.height {
                pixels[(y * w + r.x) as usize..][..r.width as usize].fill(*colour);
            }
        }
        let mapped = bo.map_mut(0, 0, w, h, |m| {
            let stride = m.stride() as usize;
            let buf = m.buffer_mut();
            for (y, row) in pixels.chunks_exact(w as usize).enumerate() {
                for (x, &[r, g, b]) in row.iter().enumerate() {
                    buf[y * stride + x * 4..][..4].copy_from_slice(&[b, g, r, 255]);
                }
            }
        });
        let frame = Frame::unleased(
            FrameBuffer::Dmabuf {
                objects: vec![Arc::new(bo.fd().map_err(|e| EncodeError(e.to_string()))?)],
                fourcc: XRGB8888,
                modifier: bo.modifier().into(),
                planes: vec![crate::stream::Plane {
                    object: 0,
                    offset: bo.offset(0),
                    stride: bo.stride(),
                }],
            },
            COLOUR_SIZE,
        );
        if mapped.is_err() {
            clear(gl, &frame, rects)?;
        }
        Ok((frame, bo))
    }

    /// `rects` cleared into `frame`'s dma-buf in order.
    fn clear(gl: &Gl, frame: &Frame, rects: &[(Rect, [u8; 3])]) -> Result<(), EncodeError> {
        use glow::HasContext;
        let FrameBuffer::Dmabuf {
            objects,
            fourcc,
            modifier,
            planes,
        } = &frame.buffer
        else {
            unreachable!("a dma-buf frame");
        };
        gl.current()?;
        let image = gl.import(objects, *fourcc, *modifier, planes, frame.size)?;
        let g = &gl.gl;
        // SAFETY: GL calls on the current context; what is made here is deleted here.
        let status = unsafe {
            let texture = g.create_texture().unwrap();
            gl.bind_image(texture, image);
            let fb = g.create_framebuffer().unwrap();
            g.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            let colour = glow::COLOR_ATTACHMENT0;
            g.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                colour,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = g.check_framebuffer_status(glow::FRAMEBUFFER);
            g.enable(glow::SCISSOR_TEST);
            for (r, rgb) in rects {
                let [red, green, blue] = rgb.map(|c| f32::from(c) / 255.0);
                g.scissor(r.x as i32, r.y as i32, r.width as i32, r.height as i32);
                g.clear_color(red, green, blue, 1.0);
                g.clear(glow::COLOR_BUFFER_BIT);
            }
            g.disable(glow::SCISSOR_TEST);
            g.finish();
            g.bind_framebuffer(glow::FRAMEBUFFER, None);
            g.delete_framebuffer(fb);
            g.delete_texture(texture);
            (status, g.get_error())
        };
        gl.destroy_image(image);
        match status {
            (glow::FRAMEBUFFER_COMPLETE, glow::NO_ERROR) => Ok(()),
            other => Err(EncodeError(format!("clearing: {other:#x?}"))),
        }
    }

    /// The RGB FreeRDP shows for a full-range BT.709 sample (libfreerdp/primitives
    /// prim_internal.h).
    fn freerdp(y: u8, u: u8, v: u8) -> [u8; 3] {
        let (y, d, e) = (i32::from(y) * 256, i32::from(u) - 128, i32::from(v) - 128);
        let c = |x: i32| (x >> 8).clamp(0, 255) as u8;
        [c(y + 403 * e), c(y - 48 * d - 120 * e), c(y + 475 * d)]
    }

    /// What a client shows from a decoded 4:2:0 picture.
    fn shown_420(picture: &frame::Video) -> impl Fn(u32, u32) -> [u8; 3] + '_ {
        move |x, y| {
            let at = |plane: usize, x: u32, y: u32| {
                picture.data(plane)[y as usize * picture.stride(plane) + x as usize]
            };
            freerdp(at(0, x, y), at(1, x / 2, y / 2), at(2, x / 2, y / 2))
        }
    }

    /// What a client shows from combined 4:4:4 planes.
    fn shown_444(p: &avc444::Planes444) -> impl Fn(u32, u32) -> [u8; 3] + '_ {
        move |x, y| {
            let at = (y * p.width + x) as usize;
            freerdp(p.y[at], p.u[at], p.v[at])
        }
    }

    /// The largest per-channel difference between `want` and the median colour shown over
    /// `pixels`.
    fn median_error(
        shown: &impl Fn(u32, u32) -> [u8; 3],
        pixels: impl Iterator<Item = (u32, u32)>,
        want: [u8; 3],
    ) -> u8 {
        let samples: Vec<[u8; 3]> = pixels.map(|(x, y)| shown(x, y)).collect();
        (0..3)
            .map(|c| {
                let mut channel: Vec<u8> = samples.iter().map(|s| s[c]).collect();
                channel.sort_unstable();
                channel[channel.len() / 2].abs_diff(want[c])
            })
            .max()
            .expect("three channels")
    }

    /// The largest per-channel error of the patches, each read on a grid inside it.
    fn patch_error(shown: impl Fn(u32, u32) -> [u8; 3]) -> u8 {
        let inside = |r: Rect| {
            let xs = (r.x + 60..r.x + r.width - 60).step_by(7);
            xs.flat_map(move |x| {
                (r.y + 60..r.y + r.height - 60)
                    .step_by(7)
                    .map(move |y| (x, y))
            })
        };
        (0..PATCHES.len())
            .map(|i| median_error(&shown, inside(patch(i)), PATCHES[i]))
            .max()
            .expect("patches")
    }

    fn nvenc() -> Ffmpeg {
        silence();
        Ffmpeg::new(
            Backend::Nvenc,
            Path::new(RENDER_NODE),
            60,
            Quality::Balanced,
        )
        .expect("NVENC on this machine")
    }

    fn gl() -> Gl {
        Gl::new(Path::new(RENDER_NODE)).expect("OpenGL ES on the render node")
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn nvenc_codes_the_colour_pattern_in_full_range_bt709() {
        let gl = gl();
        let (frame, _bo) = colour_frame(&gl, false, &colour_rects()).unwrap();
        let mut encoder = nvenc();
        let data = avc(encoder.encode(&frame, true).unwrap()).0;
        let picture = Decoder::new().decode(&data);
        assert_eq!(picture.color_range(), ::ffmpeg::color::Range::JPEG);
        let error = patch_error(shown_420(&picture));
        eprintln!("largest patch error {error}");
        assert!(error <= 3, "a patch is off by {error}");
        let shm = frame_of(COLOUR_SIZE);
        assert!(
            encoder.encode(&shm, true).is_err(),
            "a shared-memory frame is refused, not copied to the GPU"
        );
        let (linear, _linear_bo) = colour_frame(&gl, true, &colour_rects()).unwrap();
        assert!(
            nvenc().encode(&linear, true).is_err(),
            "a LINEAR buffer is refused, as NVIDIA's GL does not sample one as a 2D texture"
        );
    }

    /// A shared-memory frame of `size`.
    fn frame_of(size: (u32, u32)) -> Frame {
        frame(size, 0, vec![Rect::full(size)])
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn nvenc_codes_avc444_views() {
        let gl = gl();
        let (frame, _bo) = colour_frame(&gl, false, &colour_rects()).unwrap();
        let mut encoder = nvenc();
        encoder.set_avc444(true);
        let (main, aux, regions) = views(encoder.encode(&frame, true).unwrap());
        assert_eq!(regions, [Rect::full(COLOUR_SIZE)]);
        let mut decoder = Decoder::new();
        let main = nv12(&decoder.decode(&main));
        let aux = nv12(&decoder.decode(&aux.expect("the lines need the auxiliary view")));
        let back = avc444::tests::crop(&avc444::combine_v1(&main, &aux), COLOUR_SIZE);
        let only_main = main_only(&main, COLOUR_SIZE);
        let error = patch_error(shown_444(&back));
        assert!(error <= 3, "a patch is off by {error}");
        for i in 0..LINES {
            let (r, want) = line(i);
            let pixels = || (r.y + 10..r.y + r.height - 10).map(|y| (r.x, y));
            let full = median_error(&shown_444(&back), pixels(), want);
            let half = median_error(&shown_444(&only_main), pixels(), want);
            eprintln!(
                "line {i} at x {}: off by {full}, {half} from the main view",
                r.x
            );
            assert!(
                full < half / 2,
                "line {i} is off by {full}, {half} from the main view alone"
            );
        }
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn nvenc_codes_1080p_faster_than_60_fps() {
        let gl = gl();
        let inverted: Vec<_> = (colour_rects().into_iter())
            .map(|(r, c)| (r, c.map(|v| 255 - v)))
            .collect();
        let frames = [
            colour_frame(&gl, false, &colour_rects()).unwrap(),
            colour_frame(&gl, false, &inverted).unwrap(),
        ];
        let mut encoder = nvenc();
        avc(encoder.encode(&frames[0].0, true).unwrap());
        const PICTURES: u32 = 240;
        let started = std::time::Instant::now();
        for n in 0..PICTURES {
            avc(encoder.encode(&frames[(n % 2) as usize].0, false).unwrap());
        }
        let rate = f64::from(PICTURES) / started.elapsed().as_secs_f64();
        eprintln!("NVENC codes {rate:.0} pictures a second at 1920x1080");
        assert!(rate > 60.0, "{rate:.0} pictures a second");
    }
}
