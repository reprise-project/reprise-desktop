//! H.264 through FFmpeg on a backend the probe found, at a constant QP with no B-frames: one
//! access unit out for each frame in, or both AVC444 views of it.

use super::avc444::{self, AuxLayout};
use super::colour;
use super::h264merge::Merge;
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
/// With AVC444 on one encoder, so each view predicts from its own previous picture; FFmpeg's
/// VA-API H.264 ignores it.
const AVC444_REFS: i32 = 2;
/// NVENC sessions a GeForce driver runs at once.
const NVENC_SESSIONS: usize = 8;
/// P pictures a change is still corrected in: at a constant QP an encoder leaves part of a
/// change, such as a moved window's old edge, to the next pictures, outside their damage.
const CARRIED: usize = 2;

pub struct Ffmpeg {
    backend: Backend,
    devices: Option<Devices>,
    node: PathBuf,
    frame_rate: u32,
    quality: Quality,
    avc444: bool,
    /// The picture size the AVC444 views failed at, which codes AVC420 from then on.
    no_views: Option<(u32, u32)>,
    /// The size pictures are coded at; None for each frame's own.
    target: Option<(u32, u32)>,
    /// AVC444 codes each view on its own encoder and merges them into one stream; None codes
    /// both views through one encoder, in the v1 layout. One merge lasts as long as this encoder.
    merge: Option<Merge>,
    /// `[remote_desktop] avc444_split_encoders` and the connection's screen count.
    split: (bool, usize),
    /// The journal was told how AVC444 codes its views.
    views_logged: bool,
    open: Option<Open>,
}

/// An encoder opened for one picture size, target size, pixel format and quality.
struct Open {
    quality: Quality,
    input: Input,
    encoder: ::ffmpeg::encoder::video::Encoder,
    /// The auxiliary view's own encoder when the views merge.
    aux: Option<AuxEncoder>,
    /// Pictures since the last IDR.
    since_idr: u32,
    pts: i64,
    /// The damage of the last [`CARRIED`] P pictures, oldest first, which the next picture's
    /// regions cover too.
    carried: Vec<Vec<Rect>>,
}

struct AuxEncoder {
    encoder: ::ffmpeg::encoder::video::Encoder,
    pts: i64,
    /// The main view's last IDR went out without an aux view, so the next one is an IDR.
    idr: bool,
}

enum Input {
    Avc420(Converter),
    Avc444(Views),
}

impl Input {
    fn fits(&self, size: (u32, u32), target: (u32, u32), fourcc: u32, avc444: bool) -> bool {
        match self {
            Input::Avc420(c) => {
                !avc444 && (c.size, c.target) == (size, target) && c.fourcc == fourcc
            }
            Input::Avc444(v) => {
                avc444 && (v.size, v.target) == (size, target) && v.fourcc == fourcc
            }
        }
    }
}

/// Why a frame was not coded: the AVC444 views failed, which AVC420 may still code, the two
/// view encoders or their merge failed, which one encoder may still code, or the encoder did.
enum Failed {
    Views(EncodeError),
    Merge(EncodeError),
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
            target: None,
            merge: matches!(
                backend,
                Backend::Vaapi { .. } | Backend::Nvenc | Backend::X264
            )
            .then(Merge::default),
            split: (true, 1),
            views_logged: false,
            open: None,
        })
    }

    /// Whether AVC444 may code each view on its own encoder, `[remote_desktop]
    /// avc444_split_encoders`, for one of a connection's `screens`; off codes both on one.
    pub fn set_split_encoders(&mut self, on: bool, screens: usize) {
        self.split = (on, screens);
        if !split_encoders(self.backend, on, screens) {
            self.merge = None;
        }
    }

    /// The journal line naming whether AVC444 codes its views on two encoders or on one, and why.
    fn views_line(&self) -> String {
        let label = self.backend.label();
        let (on, screens) = self.split;
        let why = if self.merge.is_some() {
            return format!("AVC444 views: two {label} encoders, merged");
        } else if !on {
            "avc444_split_encoders is off".to_string()
        } else if !split_encoders(self.backend, on, screens) {
            let sessions = 2 * screens;
            format!("{screens} screens need {sessions} sessions, NVENC has {NVENC_SESSIONS}")
        } else {
            "merging them failed".to_string()
        };
        format!("AVC444 views: one {label} encoder, {why}")
    }

    /// Frames of `size` coded at `target`, which AVC444 pads to whole macroblocks.
    fn open(
        &self,
        fourcc: u32,
        size: (u32, u32),
        target: (u32, u32),
        avc444: bool,
    ) -> Result<Open, Failed> {
        let devices = self.devices.as_ref();
        let merged = avc444 && self.merge.is_some();
        let (input, coded) = match avc444 {
            true => {
                // One encoder codes v2 in about twice v1's bytes: v1's aux view repeats the main
                // view outside the read rect, which the encoder predicts from.
                let layout = match merged {
                    true => AuxLayout::of(target),
                    false => AuxLayout::V1,
                };
                let views = Views::new(devices, &self.node, fourcc, size, target, layout)
                    .map_err(Failed::Views)?;
                (Input::Avc444(views), avc444::view_size(target))
            }
            false => {
                let c = Converter::new(devices, fourcc, size, target, self.frame_rate)?;
                (Input::Avc420(c), target)
            }
        };
        let mut input = input;
        let (pixel, frames) = match &mut input {
            Input::Avc420(c) => (c.pixel(), c.frames()),
            Input::Avc444(v) => (v.pixel(), v.frames()),
        };
        let encoder = self.encoder(coded, pixel, frames, avc444, merged)?;
        let aux = match merged {
            true => {
                let opened = self.encoder(coded, pixel, frames, avc444, merged);
                #[cfg(test)]
                let opened = match tests::aux_open_fails() {
                    true => Err(EncodeError("the aux encoder did not open".into())),
                    false => opened,
                };
                Some(AuxEncoder {
                    encoder: opened.map_err(Failed::Merge)?,
                    pts: 0,
                    idr: false,
                })
            }
            false => None,
        };
        Ok(Open {
            quality: self.quality,
            input,
            encoder,
            aux,
            since_idr: 0,
            pts: 0,
            carried: Vec::new(),
        })
    }

    /// An encoder of `coded` pictures in `pixel`, taking them from `frames` on a GPU; with
    /// `merged`, of one AVC444 view, predicted from its own last picture alone.
    fn encoder(
        &self,
        coded: (u32, u32),
        pixel: ::ffmpeg::format::Pixel,
        frames: Option<*mut ffi::AVBufferRef>,
        avc444: bool,
        merged: bool,
    ) -> Result<::ffmpeg::encoder::video::Encoder, EncodeError> {
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
                (*c).refs = if merged { 1 } else { AVC444_REFS };
            }
            if let Some(frames) = frames {
                (*c).hw_frames_ctx = ffi::av_buffer_ref(frames);
            }
        }
        ctx.open_with(options(self.backend, self.quality, avc444, merged))
            .map_err(|e| EncodeError(format!("{name}: {e}")))
    }

    fn code(&mut self, frame: &Frame, keyframe: bool, avc444: bool) -> Result<Encoded, Failed> {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        let target = self.target.unwrap_or(frame.size);
        let (size, target) = match avc444 {
            true => (frame.size, target),
            false => (hwmap::even(frame.size), hwmap::even(target)),
        };
        let damage = scaled(&frame.damage, size, target);
        let current =
            |o: &&mut Open| o.input.fits(size, target, fourcc, avc444) && o.quality == self.quality;
        let (n, carried) = match self.open.as_mut().filter(current) {
            Some(o) if !keyframe && o.since_idr + 1 < IDR_INTERVAL => {
                (o.since_idr + 1, std::mem::take(&mut o.carried))
            }
            _ => (0, Vec::new()),
        };
        let mut changed = damage.clone();
        for r in carried.iter().flatten() {
            if !changed.contains(r) {
                changed.push(*r);
            }
        }
        let regions = |n| match avc444 {
            true => avc444::regions(target, n, &changed),
            false => regions(target, n, &changed),
        };
        if regions(n).is_empty() {
            return Ok(Encoded::Nothing);
        }
        if self.open.as_mut().filter(current).is_none() {
            self.open = None;
            self.open = Some(self.open(fourcc, size, target, avc444)?);
            if avc444 && !self.views_logged {
                log::info!("{}", self.views_line());
                self.views_logged = true;
            }
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
                // A merged aux view predicts from its own last picture, so all of it is drawn.
                let drawn = match open.aux {
                    Some(_) => Rect::full(avc444::view_size(target)),
                    None => read,
                };
                let split = views.split(frame, drawn).map_err(Failed::Views)?;
                let (main, key) = next(split.main, n == 0)?;
                let since = if key { 0 } else { n };
                // LC 1 applies to the bounds of the regions, undamaged blocks between them too.
                let flat = split.flat.covers(&[read]);
                let (main, aux) = match (&mut open.aux, self.merge.as_mut()) {
                    (Some(a), Some(merge)) => {
                        #[cfg(test)]
                        tests::tap(false, &main);
                        let main = merge.main(&main).map_err(merge_failed)?;
                        a.idr |= key;
                        let aux = match flat {
                            true => None,
                            false => {
                                let sent =
                                    send(&mut a.encoder, split.aux, a.idr, &mut a.pts, codec);
                                #[cfg(test)]
                                let sent = match tests::aux_send_fails() {
                                    true => Err(EncodeError("the aux encoder failed".into())),
                                    false => sent,
                                };
                                let data = sent.map_err(Failed::Merge)?.0;
                                #[cfg(test)]
                                tests::tap(true, &data);
                                a.idr = false;
                                Some(merge.aux(&data).map_err(merge_failed)?)
                            }
                        };
                        (main, aux)
                    }
                    _ => match flat {
                        true => (main, None),
                        false => (main, Some(next(split.aux, false)?.0)),
                    },
                };
                let layout = views.layout;
                (
                    since,
                    Encoded::Avc444 {
                        main,
                        aux,
                        regions,
                        layout,
                    },
                )
            }
        };
        open.since_idr = coded.0;
        open.carried = match coded.0 {
            0 => Vec::new(),
            _ => {
                let mut carried = carried;
                carried.push(damage);
                let old = carried.len().saturating_sub(CARRIED);
                carried.split_off(old)
            }
        };
        Ok(coded.1)
    }
}

/// Whether `backend` codes each AVC444 view of `screens` on its own encoder: NVENC only while
/// two sessions per screen fit in [`NVENC_SESSIONS`].
fn split_encoders(backend: Backend, on: bool, screens: usize) -> bool {
    on && (backend != Backend::Nvenc || 2 * screens <= NVENC_SESSIONS)
}

fn merge_failed(e: super::h264merge::Error) -> Failed {
    Failed::Merge(EncodeError(e.to_string()))
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

/// The encoder's private options: constant QP and its lowest-latency settings; `merged` for an
/// AVC444 view on its own encoder.
fn options(backend: Backend, quality: Quality, avc444: bool, merged: bool) -> Dictionary {
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
    if merged && backend == Backend::X264 {
        // Weighted prediction writes tables into slice headers that the merge doesn't rewrite.
        o.set("weightp", "none");
        // Finds scrolled text past the preset's range; only one encoder per view has the CPU for it.
        o.set("x264-params", "me=umh:merange=64");
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

    /// A new target reopens the encoder on the next frame, as an IDR.
    fn set_target(&mut self, target: Option<(u32, u32)>) {
        self.target = target;
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
            Err(Failed::Merge(e)) => {
                log::warn!(
                    "{} cannot merge the AVC444 views, coding both on one encoder: {e}",
                    self.backend.label()
                );
                self.merge = None;
                self.open = None;
                self.encode(frame, true)
            }
            Err(Failed::Coding(e)) => Err(e),
        }
    }
}

impl Failed {
    fn into_error(self) -> EncodeError {
        match self {
            Failed::Views(e) | Failed::Merge(e) | Failed::Coding(e) => e,
        }
    }
}

/// `damage` to a picture of `size` as the parts of it scaled to `target` that change: grown by
/// a pixel on both sides, which the linear sample reaches.
fn scaled(damage: &[Rect], size: (u32, u32), target: (u32, u32)) -> Vec<Rect> {
    if size == target {
        return damage.to_vec();
    }
    let span = |start: u32, length: u32, from: u32, to: u32| {
        let (from, to) = (u64::from(from.max(1)), u64::from(to));
        let end = u64::from(start.saturating_add(length));
        let first = (u64::from(start.saturating_sub(1)) * to / from).saturating_sub(1);
        let last = ((end + 1) * to).div_ceil(from) + 1;
        (first.min(to) as u32, last.min(to) as u32)
    };
    damage
        .iter()
        .filter_map(|r| {
            let (x, right) = span(r.x, r.width, size.0, target.0);
            let (y, bottom) = span(r.y, r.height, size.1, target.1);
            (x < right && y < bottom).then(|| Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        })
        .collect()
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
pub(crate) mod tests {
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

    fn x264_options(avc444: bool, merged: bool) -> Vec<(String, String)> {
        let o = options(Backend::X264, Quality::Balanced, avc444, merged);
        o.iter().map(|(k, v)| (k.into(), v.into())).collect()
    }

    #[test]
    fn x264_searches_wide_motion_on_merged_views_only() {
        let qp = Quality::Balanced.qp().to_string();
        let pairs = |p: &[(&str, &str)]| -> Vec<(String, String)> {
            [("qp", qp.as_str())]
                .iter()
                .chain(p)
                .map(|&(k, v)| (k.into(), v.into()))
                .collect()
        };
        let base = [
            ("preset", "superfast"),
            ("tune", "zerolatency"),
            ("forced-idr", "1"),
        ];
        let single = [base.as_slice(), &[("sc_threshold", "0")]].concat();
        let merged = [
            single.as_slice(),
            &[("weightp", "none"), ("x264-params", "me=umh:merange=64")],
        ]
        .concat();
        assert_eq!(x264_options(false, false), pairs(&base), "AVC420");
        assert_eq!(
            x264_options(true, false),
            pairs(&single),
            "one-encoder AVC444"
        );
        assert_eq!(
            x264_options(true, true),
            pairs(&merged),
            "merged AVC444 views"
        );
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
        let last_column = Rect {
            x: 64,
            y: 0,
            width: 1,
            height: 35,
        };
        assert!(
            matches!(
                encoder.encode(&frame(size, 1, vec![last_column]), false),
                Ok(Encoded::Nothing)
            ),
            "damage only outside the coded picture codes nothing"
        );
        let edge = Rect {
            x: 60,
            y: 30,
            width: 5,
            height: 5,
        };
        let (_, regions) = avc(encoder.encode(&frame(size, 2, vec![edge]), false).unwrap());
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
    }

    /// The size of the window [`dragged_desktop`] draws.
    const DRAGGED: (u32, u32) = (120, 90);

    /// A smooth gradient with soft light curves, and a window at `at`: a light frame, a blue
    /// title bar and light text on black.
    fn dragged_desktop(x: u32, y: u32, at: (u32, u32)) -> (u8, u8, u8) {
        let (wx, wy) = (x.wrapping_sub(at.0), y.wrapping_sub(at.1));
        let (w, h) = DRAGGED;
        if wx < w && wy < h {
            return match (wx, wy) {
                (0, _) | (_, 0) => (255, 255, 255),
                _ if wx == w - 1 || wy == h - 1 => (64, 64, 64),
                (_, 1..=18) => (10, 36, (106 + wx) as u8),
                _ if wx % 6 < 4 && wy % 12 < 8 && (wx / 6 * 7 + wy / 12 * 3) % 5 < 3 => {
                    (200, 200, 200)
                }
                _ => (12, 12, 12),
            };
        }
        let (fx, fy) = (f64::from(x), f64::from(y));
        let curve = (0..3)
            .map(|k| {
                let k = f64::from(k);
                let centre = 40.0 + 60.0 * k + 18.0 * (fx / (31.0 + 9.0 * k) + k).sin();
                (1.0 - (fy - centre).abs() / 1.5).max(0.0)
            })
            .fold(0.0, f64::max);
        let mix = |base: f64, light: f64| (base + (light - base) * curve * 0.7) as u8;
        (
            mix(60.0 + fx / 3.0, 235.0),
            mix(40.0 + fy / 2.0, 235.0),
            mix(170.0 - fx / 4.0, 240.0),
        )
    }

    #[test]
    fn a_dragged_windows_old_edges_leave_no_streaks_on_the_client() {
        silence();
        let mut encoder =
            Ffmpeg::new(Backend::X264, Path::new(RENDER_NODE), 60, Quality::Speed).unwrap();
        let mut decoder = Decoder::new();
        let at = |dx: u32| (8 + dx, 8 + dx / 3);
        let window = |dx: u32| Rect {
            x: at(dx).0,
            y: at(dx).1,
            width: DRAGGED.0,
            height: DRAGGED.1,
        };
        let size = (320, 192);
        let path: Vec<u32> = (0..=192)
            .step_by(16)
            .chain((0..=192).rev().step_by(16))
            .collect();
        let mut client = vec![[0u8; 3]; (size.0 * size.1) as usize];
        let mut decoded = None;
        let mut before = path[0];
        for (i, &dx) in path.iter().enumerate() {
            let (mut frame, _) = picture(size, |x, y| dragged_desktop(x, y, at(dx)));
            if i > 0 {
                frame.damage = vec![window(before), window(dx)];
            }
            before = dx;
            let (data, regions) = avc(encoder.encode(&frame, i == 0).unwrap());
            let picture = decoder.decode(&data);
            {
                let shown = shown_420(&picture);
                for r in &regions {
                    for y in r.y..r.y + r.height {
                        for x in r.x..r.x + r.width {
                            client[(y * size.0 + x) as usize] = shown(x, y);
                        }
                    }
                }
            }
            decoded = Some(picture);
        }
        let decoded = decoded.expect("pictures");
        let shown = shown_420(&decoded);
        let last = at(*path.last().unwrap());
        let off = |p: [u8; 3], x: u32, y: u32| {
            let (r, g, b) = dragged_desktop(x, y, last);
            [r, g, b]
                .iter()
                .zip(p)
                .map(|(&s, v)| s.abs_diff(v))
                .max()
                .unwrap()
        };
        let pixels = || (0..size.1).flat_map(|y| (0..size.0).map(move |x| (x, y)));
        let client_off = pixels()
            .filter(|&(x, y)| off(client[(y * size.0 + x) as usize], x, y) > 16)
            .count();
        let decoded_off = pixels()
            .filter(|&(x, y)| off(shown(x, y), x, y) > 16)
            .count();
        assert!(
            client_off <= decoded_off,
            "the client, which copies only the regions, is as close to the screen as the last \
             decoded picture: {client_off} pixels off against {decoded_off}"
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
        let mut switch = Switch::new(Box::new(|backend, quality, frame_rate| {
            Ffmpeg::new(backend, Path::new("/dev/null"), frame_rate, quality)
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
        let mut switch = Switch::new(Box::new(|_, _, _| Ok(Box::new(x264()))))
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
            Encoded::Avc444 {
                main, aux, regions, ..
            } => (main, aux, regions),
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
            let Encoded::Avc444 {
                main,
                aux,
                regions,
                layout,
            } = encoder.encode(&frame, true).unwrap()
            else {
                panic!("AVC444 views");
            };
            assert_eq!(regions, [Rect::full(size)]);
            let aux = aux.expect("coloured lines need the auxiliary view");
            let main = nv12(&decoder.decode(&main));
            let aux = nv12(&decoder.decode(&aux));
            assert_eq!((main.width, main.height), (128, 64), "whole macroblocks");
            let back = avc444::tests::crop(&avc444::combine(layout, &main, &aux), size);
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
        let mut before: Option<(Paint, Vec<Rect>)> = None;
        for (what, pixel, damage, needs_aux) in steps {
            if let Some((pixel, regions)) = before.take() {
                let (mut still, _) = picture(size, pixel);
                still.damage.clear();
                for _ in 0..CARRIED {
                    let (main, aux, again) = views(encoder.encode(&still, false).unwrap());
                    assert_eq!(
                        again, regions,
                        "the pictures after a change send its regions"
                    );
                    decoder.decode(&main);
                    if let Some(aux) = aux {
                        decoder.decode(&aux);
                    }
                }
            }
            let (mut frame, _) = picture(size, &pixel);
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
            before = Some((pixel, regions));
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

    /// Access units in send order, true for the aux view's.
    type Units = Vec<(bool, Vec<u8>)>;

    thread_local! {
        /// The view encoders' own access units from the first [`tapped`] on.
        static TAPPED: std::cell::RefCell<Option<Units>> = const {
            std::cell::RefCell::new(None)
        };
    }

    pub(super) fn tap(aux: bool, au: &[u8]) {
        TAPPED.with(|t| (t.borrow_mut().as_mut()).map(|v| v.push((aux, au.to_vec()))));
    }

    /// The access units tapped since the last call.
    fn tapped() -> Units {
        TAPPED.with(|t| t.borrow_mut().replace(Vec::new()).unwrap_or_default())
    }

    thread_local! {
        static FAIL_AUX_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Whether the next aux encoder is to fail to open, once.
    pub(super) fn aux_open_fails() -> bool {
        FAIL_AUX_OPEN.take()
    }

    thread_local! {
        static FAIL_AUX_SEND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Whether the next aux view is to fail to code, once.
    pub(super) fn aux_send_fails() -> bool {
        FAIL_AUX_SEND.take()
    }

    fn vaapi() -> Ffmpeg {
        silence();
        let backend = (crate::encode::probe::probe().1.into_iter())
            .map(|(b, _)| b)
            .find(|b| matches!(b, Backend::Vaapi { .. }))
            .expect("VA-API H.264 on this machine");
        let mut encoder =
            Ffmpeg::new(backend, Path::new(RENDER_NODE), 60, Quality::Balanced).unwrap();
        encoder.set_avc444(true);
        encoder
    }

    fn render_node() -> gbm::Device<std::fs::File> {
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .open(RENDER_NODE);
        gbm::Device::new(file.unwrap()).unwrap()
    }

    /// A window of text that moves except in frames 13-34, a pane of 1-px coloured text that
    /// scrolls in 13-24, a grey bar alone in 25-33, all grey in 31-33, a smaller screen from 41.
    struct Desktop {
        text: Vec<[u8; 3]>,
    }

    const TEXT: (u32, u32) = (120, 400);

    impl Desktop {
        fn new() -> Desktop {
            let mut seed = 0x2545_f491_4f6c_dd1du64;
            let mut next = move || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed
            };
            let colours = [
                [220, 220, 220],
                [86, 156, 214],
                [206, 145, 120],
                [106, 153, 85],
            ];
            let text = (0..TEXT.0 * TEXT.1)
                .map(|i| {
                    let (x, y) = (i % TEXT.0, i / TEXT.0);
                    let r = next();
                    match (x % 6 < 4 && y % 12 < 8, r % 5) {
                        (true, 0 | 1) => colours[(r / 5 % 4) as usize],
                        _ => [30, 30, 30],
                    }
                })
                .collect();
            Desktop { text }
        }

        /// V2 views, then V1 views from frame 41.
        fn size(t: u32) -> (u32, u32) {
            if t < 41 { (190, 136) } else { (168, 120) }
        }

        fn pixel(&self, t: u32) -> impl Fn(u32, u32) -> [u8; 3] + '_ {
            let h = Desktop::size(t).1;
            let scroll = 3 * (t.clamp(12, 24) - 12);
            let step = (t.min(12) + t.max(34) - 34) as i32;
            let window = (20 + 3 * step, 10 + step);
            let progress = 4 * (t.clamp(24, 33) - 24);
            let grey = (31..=33).contains(&t);
            move |x, y| {
                let (wx, wy) = (x as i32 - window.0, y as i32 - window.1);
                let c = if (0..72).contains(&wx) && (0..48).contains(&wy) {
                    match ((wx + wy * 3) % 11, wy < 10) {
                        (_, true) => [43, 87, 154],
                        (0, false) => [200, 30, 30],
                        _ => [243, 243, 243],
                    }
                } else if y >= h - 24 {
                    match (x >= 100 && x < 100 + progress, (h - 16..h - 8).contains(&y)) {
                        (true, true) => [154, 154, 154],
                        _ => [58, 58, 58],
                    }
                } else if x >= 8 && x < 8 + TEXT.0 && y >= 8 && y < h - 32 {
                    self.text[((y - 8 + scroll) * TEXT.0 + x - 8) as usize]
                } else {
                    [24, 40, 64]
                };
                match grey {
                    true => [c[1]; 3],
                    false => c,
                }
            }
        }
    }

    /// The bounds of the pixels that differ between `a` and `b` in each 64-px column.
    fn changed(
        a: &dyn Fn(u32, u32) -> [u8; 3],
        b: &dyn Fn(u32, u32) -> [u8; 3],
        size: (u32, u32),
    ) -> Vec<Rect> {
        let (w, h) = size;
        (0..w.div_ceil(64))
            .filter_map(|column| {
                let xs = column * 64..(column * 64 + 64).min(w);
                let points: Vec<(u32, u32)> = (0..h)
                    .flat_map(|y| xs.clone().map(move |x| (x, y)))
                    .filter(|&(x, y)| a(x, y) != b(x, y))
                    .collect();
                let (x0, x1) = (
                    points.iter().map(|p| p.0).min()?,
                    points.iter().map(|p| p.0).max()?,
                );
                let (y0, y1) = (
                    points.iter().map(|p| p.1).min()?,
                    points.iter().map(|p| p.1).max()?,
                );
                Some(Rect {
                    x: x0,
                    y: y0,
                    width: x1 + 1 - x0,
                    height: y1 + 1 - y0,
                })
            })
            .collect()
    }

    /// One frame of the desktop: the merged views sent, and the view encoders' own.
    struct Sent {
        t: u32,
        main: Vec<u8>,
        aux: Option<Vec<u8>>,
        layout: AuxLayout,
        own_main: Option<Vec<u8>>,
        own_aux: Option<Vec<u8>>,
    }

    const KEYFRAMES: [u32; 3] = [0, 27, 32];

    /// `encoder` over the desktop's 50 frames in dma-bufs (in shared memory for x264),
    /// keyframes asked for at [`KEYFRAMES`], Sharp from frame 35, and frame 45 unchanged.
    fn code_desktop(encoder: &mut Ffmpeg) -> Vec<Sent> {
        let gbm = (encoder.backend != Backend::X264).then(render_node);
        let desktop = Desktop::new();
        tapped();
        let mut sent = Vec::new();
        for t in 0..50 {
            if t == 35 {
                encoder.set_quality(Quality::Sharp);
            }
            let size = Desktop::size(t);
            let shown = |t: u32| desktop.pixel(if t == 45 { 44 } else { t });
            let pixel = shown(t);
            let (mut frame, _bo) = match &gbm {
                Some(gbm) if encoder.backend == Backend::Nvenc => {
                    let (frame, bo) = tiled_frame(gbm, size, &pixel);
                    (frame, Some(bo))
                }
                Some(gbm) => {
                    let (frame, _, bo) =
                        crate::encode::split::tests::dmabuf_frame(gbm, size, &pixel);
                    (frame, Some(bo))
                }
                None => (picture(size, |x, y| pixel(x, y).into()).0, None),
            };
            frame.damage = match t {
                0 | 41 => vec![Rect::full(size)],
                45 => vec![],
                _ => changed(&shown(t - 1), &pixel, size),
            };
            let encoded = encoder.encode(&frame, KEYFRAMES.contains(&t)).unwrap();
            let own = tapped();
            let own_view = |aux| {
                own.iter()
                    .find(|(a, _)| *a == aux)
                    .map(|(_, au)| au.clone())
            };
            match encoded {
                Encoded::Avc444 {
                    main, aux, layout, ..
                } => sent.push(Sent {
                    t,
                    main,
                    aux,
                    layout,
                    own_main: own_view(false),
                    own_aux: own_view(true),
                }),
                Encoded::Nothing => assert_eq!(t, 45, "frame {t} codes"),
                _ => panic!("frame {t}: AVC444 views"),
            }
        }
        sent
    }

    /// A dma-buf frame of `pixel` in the driver's own layout: NVIDIA's GL samples no LINEAR one.
    fn tiled_frame(
        gbm: &gbm::Device<std::fs::File>,
        (w, h): (u32, u32),
        pixel: impl Fn(u32, u32) -> [u8; 3],
    ) -> (Frame, gbm::BufferObject<()>) {
        let flags = gbm::BufferObjectFlags::RENDERING;
        let mut bo = (gbm.create_buffer_object::<()>(w, h, gbm::Format::Xrgb8888, flags)).unwrap();
        bo.map_mut(0, 0, w, h, |m| {
            let stride = m.stride() as usize;
            let buf = m.buffer_mut();
            for y in 0..h {
                for x in 0..w {
                    let [r, g, b] = pixel(x, y);
                    buf[y as usize * stride + x as usize * 4..][..4]
                        .copy_from_slice(&[b, g, r, 255]);
                }
            }
        })
        .unwrap();
        let buffer = FrameBuffer::Dmabuf {
            objects: vec![Arc::new(bo.fd().unwrap())],
            fourcc: XRGB8888,
            modifier: bo.modifier().into(),
            planes: vec![crate::stream::Plane {
                object: 0,
                offset: bo.offset(0),
                stride: bo.stride(),
            }],
        };
        (Frame::unleased(buffer, (w, h)), bo)
    }

    /// Whether Annex B `au` holds an IDR slice.
    fn idr(au: &[u8]) -> bool {
        au.windows(4)
            .any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
    }

    fn x264_avc444() -> Ffmpeg {
        let mut encoder = x264();
        encoder.set_avc444(true);
        encoder
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn merged_avc444_views_decode_to_each_view_encoders_own_pictures() {
        assert_merged_views_decode_to_their_own_pictures(&mut vaapi());
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn merged_nvenc_avc444_views_decode_to_each_view_encoders_own_pictures() {
        let mut encoder = nvenc();
        encoder.set_avc444(true);
        assert_merged_views_decode_to_their_own_pictures(&mut encoder);
        assert!(encoder.merge.is_some(), "the views merged to the end");
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn merged_nvenc_avc444_views_turn_into_idrs_together() {
        let mut encoder = nvenc();
        encoder.set_avc444(true);
        assert_views_turn_into_idrs_together(&mut encoder);
    }

    #[test]
    fn merged_x264_avc444_views_decode_to_each_view_encoders_own_pictures() {
        let mut encoder = x264_avc444();
        assert_merged_views_decode_to_their_own_pictures(&mut encoder);
        assert!(encoder.merge.is_some(), "the views merged to the end");
    }

    fn assert_merged_views_decode_to_their_own_pictures(encoder: &mut Ffmpeg) {
        let sent = code_desktop(encoder);
        let (mut merged, mut main, mut aux) = (Decoder::new(), Decoder::new(), Decoder::new());
        let mut pictures = 0;
        for s in &sent {
            let layout = if s.t < 41 {
                AuxLayout::V2
            } else {
                AuxLayout::V1
            };
            assert_eq!(s.layout, layout, "frame {}: the layout sent", s.t);
            let own = s.own_main.as_ref().expect("the main view's own encoder");
            assert_eq!(
                nv12(&merged.decode(&s.main)),
                nv12(&main.decode(own)),
                "frame {}: main view",
                s.t
            );
            assert_eq!(s.aux.is_some(), s.own_aux.is_some(), "frame {}", s.t);
            if let (Some(a), Some(own)) = (&s.aux, &s.own_aux) {
                assert_eq!(
                    nv12(&merged.decode(a)),
                    nv12(&aux.decode(own)),
                    "frame {}: aux view",
                    s.t
                );
                pictures += 1;
            }
            pictures += 1;
        }
        let skipped: Vec<u32> = (sent.iter())
            .filter(|s| s.aux.is_none())
            .map(|s| s.t)
            .collect();
        assert!(
            skipped.contains(&32),
            "an aux view skipped on an IDR: {skipped:?}"
        );
        assert!(
            skipped.iter().any(|t| (25..31).contains(t)),
            "the grey bar skips the aux view: {skipped:?}"
        );
        assert!(pictures > 80, "{pictures} pictures");
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn merged_avc444_views_turn_into_idrs_together() {
        assert_views_turn_into_idrs_together(&mut vaapi());
    }

    #[test]
    fn merged_x264_avc444_views_turn_into_idrs_together() {
        assert_views_turn_into_idrs_together(&mut x264_avc444());
    }

    fn assert_views_turn_into_idrs_together(encoder: &mut Ffmpeg) {
        let sent = code_desktop(encoder);
        let kinds: Vec<(u32, bool, Option<bool>)> = (sent.iter())
            .map(|s| {
                let own_aux = s.own_aux.as_deref().map(idr);
                (s.t, idr(s.own_main.as_ref().unwrap()), own_aux)
            })
            .filter(|&(_, main, aux)| main || aux == Some(true))
            .collect();
        assert_eq!(
            kinds,
            [
                (0, true, Some(true)),
                (27, true, Some(true)),
                (32, true, None),
                (34, false, Some(true)),
                (35, true, Some(true)),
                (41, true, Some(true)),
            ],
            "keyframes, a new quality and a new size make both views IDRs; an aux view skipped \
             on an IDR makes the next one an IDR"
        );
        for s in sent.iter().filter(|s| idr(s.own_main.as_ref().unwrap())) {
            assert!(
                idr(&s.main),
                "frame {}: the merged main view is an IDR",
                s.t
            );
            assert!(
                !s.aux.as_deref().is_some_and(idr),
                "frame {}: one IDR a frame",
                s.t
            );
        }
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn a_merge_error_codes_both_views_on_one_encoder_from_then_on() {
        crate::listen::tests::capture_log();
        let gbm = render_node();
        let mut encoder = vaapi();
        let mut decoder = Decoder::new();
        let size = (192, 128);
        let picture =
            |t: u32| move |x: u32, y: u32| [(x * 2) as u8, (y * 3 + t) as u8, (x + t * 8) as u8];
        let mut code = |t: u32, encoder: &mut Ffmpeg| {
            let (mut frame, _, _bo) =
                crate::encode::split::tests::dmabuf_frame(&gbm, size, picture(t));
            frame.damage = vec![Rect::full(size)];
            let coded = encoder.encode(&frame, t == 0);
            let (main, aux, regions) = views(coded.unwrap_or_else(|e| panic!("frame {t}: {e}")));
            let main = decoder.decode(&main);
            if let Some(aux) = aux {
                decoder.decode(&aux);
            }
            (main.kind(), regions, tapped().len())
        };
        tapped();
        for t in 0..3 {
            assert_eq!(code(t, &mut encoder).2, 2, "frame {t}: two encoders");
        }
        // A merge that never saw the main view's IDR refuses its next picture.
        encoder.merge = Some(Merge::default());
        let (kind, regions, own) = code(3, &mut encoder);
        assert_eq!(
            (kind, regions, own),
            (::ffmpeg::picture::Type::I, vec![Rect::full(size)], 1),
            "the main view the merge refused, then the frame again on one encoder as an IDR of \
             the whole picture"
        );
        assert!(encoder.merge.is_none());
        for t in 4..8 {
            let (kind, _, own) = code(t, &mut encoder);
            assert_eq!((kind, own), (::ffmpeg::picture::Type::P, 0), "frame {t}");
        }
        encoder.set_target(Some((160, 128)));
        assert_eq!(code(8, &mut encoder).2, 0, "a reopened encoder stays alone");
        let line = "VA-API H.264 cannot merge the AVC444 views, coding both on one encoder: \
                    H.264 merge out of order: a slice before the main view's SPS and PPS";
        assert_eq!(
            crate::listen::tests::times_logged(line),
            1,
            "one journal line"
        );
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn an_aux_encoder_that_fails_to_open_codes_both_views_on_one_encoder() {
        crate::listen::tests::capture_log();
        let gbm = render_node();
        let mut encoder = vaapi();
        let mut decoder = Decoder::new();
        let size = (192, 128);
        tapped();
        FAIL_AUX_OPEN.set(true);
        for t in 0..4 {
            let pixel = |x: u32, y: u32| [(x * 2) as u8, (y * 3 + t) as u8, (x + t * 8) as u8];
            let (mut frame, _, _bo) = crate::encode::split::tests::dmabuf_frame(&gbm, size, pixel);
            frame.damage = vec![Rect::full(size)];
            let Ok(Encoded::Avc444 {
                main,
                aux,
                regions,
                layout,
            }) = encoder.encode(&frame, t == 0)
            else {
                panic!("frame {t}: AVC444 views");
            };
            let kind = decoder.decode(&main).kind();
            if let Some(aux) = aux {
                decoder.decode(&aux);
            }
            let want = match t {
                0 => ::ffmpeg::picture::Type::I,
                _ => ::ffmpeg::picture::Type::P,
            };
            assert_eq!((kind, regions), (want, vec![Rect::full(size)]), "frame {t}");
            assert_eq!(
                (layout, tapped().len()),
                (AuxLayout::V1, 0),
                "frame {t}: one encoder, the v1 layout"
            );
        }
        assert!(encoder.merge.is_none());
        let line = "VA-API H.264 cannot merge the AVC444 views, coding both on one encoder: \
                    the aux encoder did not open";
        assert_eq!(
            crate::listen::tests::times_logged(line),
            1,
            "one journal line"
        );
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn merged_v2_views_decode_through_one_decoder_to_the_picture() {
        let gbm = render_node();
        let desktop = Desktop::new();
        let size = Desktop::size(0);
        let mut encoder = vaapi();
        let mut decoder = Decoder::new();
        for t in [18, 19] {
            let pixel = desktop.pixel(t);
            let (mut frame, data, _bo) =
                crate::encode::split::tests::dmabuf_frame(&gbm, size, &pixel);
            frame.damage = vec![Rect::full(size)];
            let Ok(Encoded::Avc444 {
                main,
                aux: Some(aux),
                layout,
                ..
            }) = encoder.encode(&frame, t == 18)
            else {
                panic!("frame {t}: both AVC444 views");
            };
            assert_eq!(layout, AuxLayout::V2, "a 192-px view takes the v2 layout");
            let (main, aux) = (nv12(&decoder.decode(&main)), nv12(&decoder.decode(&aux)));
            assert_eq!((main.width, main.height), (192, 144), "whole macroblocks");
            let stride = data.len() / size.1 as usize;
            let src = avc444::Planes444::from_rgb(&data, stride, size, XRGB8888);
            let back = avc444::tests::crop(&avc444::combine_v2(&main, &aux), size);
            let (full, half) = (
                chroma_psnr(&back, &src),
                chroma_psnr(&main_only(&main, size), &src),
            );
            eprintln!("frame {t}: chroma {full:.1} dB, {half:.1} dB from the main view");
            assert!(
                full > 32.0 && full > half + 2.0,
                "frame {t}: chroma {full:.1} dB, {half:.1} dB from the main view alone"
            );
        }
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
    pub(crate) fn shown_444(p: &avc444::Planes444) -> impl Fn(u32, u32) -> [u8; 3] + '_ {
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

    /// The largest per-channel error of the patches in a picture `1 / shrink` of
    /// [`COLOUR_SIZE`], each read on a grid inside it.
    pub(crate) fn patch_error(shown: impl Fn(u32, u32) -> [u8; 3], shrink: u32) -> u8 {
        let inside = |r: Rect| {
            let (x, y, margin) = (r.x / shrink, r.y / shrink, 60 / shrink);
            let (right, bottom) = ((r.x + r.width) / shrink, (r.y + r.height) / shrink);
            let xs = (x + margin..right - margin).step_by(7);
            xs.flat_map(move |x| {
                (y + margin..bottom - margin)
                    .step_by(7)
                    .map(move |y| (x, y))
            })
        };
        (0..PATCHES.len())
            .map(|i| median_error(&shown, inside(patch(i)), PATCHES[i]))
            .max()
            .expect("patches")
    }

    /// The colour picture's pixel at (`x`, `y`) of [`COLOUR_SIZE`].
    pub(crate) fn colour_pixel() -> impl Fn(u32, u32) -> [u8; 3] {
        let rects = colour_rects();
        move |x, y| {
            let inside = |r: &Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
            (rects.iter().rev())
                .find(|(r, _)| inside(r))
                .map_or([255, 255, 255], |&(_, c)| c)
        }
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
        let error = patch_error(shown_420(&picture), 1);
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
        let Encoded::Avc444 {
            main,
            aux,
            regions,
            layout,
        } = encoder.encode(&frame, true).unwrap()
        else {
            panic!("AVC444 views");
        };
        assert_eq!(regions, [Rect::full(COLOUR_SIZE)]);
        let mut decoder = Decoder::new();
        let main = nv12(&decoder.decode(&main));
        let aux = nv12(&decoder.decode(&aux.expect("the lines need the auxiliary view")));
        let back = avc444::tests::crop(&avc444::combine(layout, &main, &aux), COLOUR_SIZE);
        let only_main = main_only(&main, COLOUR_SIZE);
        let error = patch_error(shown_444(&back), 1);
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

    /// `encoder` codes the colour picture in a GPU buffer at half its size, in AVC420 and in
    /// AVC444, with each patch's colour within 3.
    fn codes_a_half_size_target(mut encoder: Ffmpeg) {
        let gl = gl();
        let (frame, _bo) = colour_frame(&gl, false, &colour_rects()).unwrap();
        let half = (960, 540);
        encoder.set_target(Some(half));
        let (data, regions) = avc(encoder.encode(&frame, true).unwrap());
        assert_eq!(regions, [Rect::full(half)]);
        let picture = Decoder::new().decode(&data);
        assert_eq!((picture.width(), picture.height()), half);
        let error = patch_error(shown_420(&picture), 2);
        assert!(error <= 3, "AVC420: a patch is off by {error}");

        encoder.set_avc444(true);
        let Encoded::Avc444 {
            main,
            aux,
            regions,
            layout,
        } = encoder.encode(&frame, true).unwrap()
        else {
            panic!("AVC444 views");
        };
        assert_eq!(regions, [Rect::full(half)]);
        let mut decoder = Decoder::new();
        let main = nv12(&decoder.decode(&main));
        assert_eq!((main.width, main.height), (960, 544), "whole macroblocks");
        let back = match aux {
            Some(aux) => avc444::combine(layout, &main, &nv12(&decoder.decode(&aux))),
            None => main_only(&main, (main.width, main.height)),
        };
        let error = patch_error(shown_444(&avc444::tests::crop(&back, half)), 2);
        assert!(error <= 3, "AVC444: a patch is off by {error}");
    }

    #[test]
    #[ignore = "needs VA-API H.264 on /dev/dri/renderD128; run with --ignored where it has one"]
    fn vaapi_codes_a_half_size_target() {
        silence();
        let backend = (crate::encode::probe::probe().1.into_iter())
            .map(|(b, _)| b)
            .find(|b| matches!(b, Backend::Vaapi { .. }))
            .expect("VA-API H.264 on this machine");
        codes_a_half_size_target(
            Ffmpeg::new(backend, Path::new(RENDER_NODE), 60, Quality::Balanced).unwrap(),
        );
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn nvenc_codes_a_half_size_target() {
        codes_a_half_size_target(nvenc());
    }

    #[test]
    fn x264_codes_a_scaled_shm_frame() {
        let colour = colour_pixel();
        let (frame, _) = picture((1280, 720), |x, y| {
            let [r, g, b] = colour(x * 3 / 2, y * 3 / 2);
            (r, g, b)
        });
        let target = (640, 360);
        for avc444 in [false, true] {
            let mut encoder = x264();
            encoder.set_target(Some(target));
            encoder.set_avc444(avc444);
            let mut decoder = Decoder::new();
            let error = match encoder.encode(&frame, true).unwrap() {
                Encoded::Avc420 { data, regions } => {
                    assert_eq!(regions, [Rect::full(target)]);
                    let picture = decoder.decode(&data);
                    assert_eq!((picture.width(), picture.height()), target);
                    patch_error(shown_420(&picture), 3)
                }
                Encoded::Avc444 {
                    main,
                    aux,
                    regions,
                    layout,
                } => {
                    assert_eq!(regions, [Rect::full(target)]);
                    assert_eq!(layout, AuxLayout::V2);
                    let main = nv12(&decoder.decode(&main));
                    assert_eq!((main.width, main.height), (640, 368));
                    let aux = nv12(&decoder.decode(&aux.expect("the lines need the aux view")));
                    let back = avc444::combine(layout, &main, &aux);
                    let back = avc444::tests::crop(&back, target);
                    patch_error(shown_444(&back), 3)
                }
                _ => panic!("H.264"),
            };
            assert!(error <= 3, "AVC444 {avc444}: a patch is off by {error}");
        }
    }

    #[test]
    fn an_aux_view_that_fails_to_code_codes_both_views_on_one_encoder_in_v1() {
        crate::listen::tests::capture_log();
        let mut encoder = x264_avc444();
        let mut decoder = Decoder::new();
        tapped();
        for t in 0..5 {
            FAIL_AUX_SEND.set(t == 2);
            let (frame, planes) = picture((160, 96), |x, y| rgb(x, y, t));
            let Encoded::Avc444 {
                main,
                aux,
                regions,
                layout,
            } = encoder.encode(&frame, t == 0).unwrap()
            else {
                panic!("frame {t}: AVC444 views");
            };
            let main = decoder.decode(&main);
            let kind = main.kind();
            let main = nv12(&main);
            let aux = nv12(&decoder.decode(&aux.expect("a coloured picture needs the aux view")));
            let back = avc444::tests::crop(&avc444::combine(layout, &main, &aux), (160, 96));
            let expected = match t {
                0 | 1 => AuxLayout::V2,
                _ => AuxLayout::V1,
            };
            assert_eq!(layout, expected, "frame {t}: v2 on two encoders, v1 on one");
            assert!(
                chroma_psnr(&back, &planes) > 35.0,
                "frame {t}: views in {layout:?}"
            );
            let want = match t {
                0 | 2 => ::ffmpeg::picture::Type::I,
                _ => ::ffmpeg::picture::Type::P,
            };
            let own = match t {
                0 | 1 => 2,
                2 => 1,
                _ => 0,
            };
            assert_eq!(
                (kind, regions, tapped().len()),
                (want, vec![Rect::full((160, 96))], own),
                "frame {t}: two encoders, then the failed frame again as an IDR on one"
            );
        }
        assert!(encoder.merge.is_none());
        let line = "Software H.264 cannot merge the AVC444 views, coding both on one encoder: \
                    the aux encoder failed";
        assert_eq!(
            crate::listen::tests::times_logged(line),
            1,
            "one journal line"
        );
    }

    #[test]
    fn split_encoders_off_codes_both_views_on_one_encoder_in_v1() {
        crate::listen::tests::capture_log();
        let mut encoder = x264_avc444();
        encoder.set_split_encoders(false, 1);
        tapped();
        let mut decoder = Decoder::new();
        for t in 0..3 {
            let (frame, planes) = picture((160, 96), |x, y| rgb(x, y, t));
            let Encoded::Avc444 {
                main, aux, layout, ..
            } = encoder.encode(&frame, t == 0).unwrap()
            else {
                panic!("frame {t}: AVC444 views");
            };
            assert_eq!((layout, tapped().len()), (AuxLayout::V1, 0), "frame {t}");
            let main = nv12(&decoder.decode(&main));
            let aux = nv12(&decoder.decode(&aux.expect("a coloured picture needs the aux view")));
            let back = avc444::tests::crop(&avc444::combine(layout, &main, &aux), (160, 96));
            assert!(chroma_psnr(&back, &planes) > 35.0, "frame {t}");
        }
        let line = "AVC444 views: one Software H.264 encoder, avc444_split_encoders is off";
        assert_eq!(
            crate::listen::tests::times_logged(line),
            1,
            "one journal line for the encoder"
        );
    }

    #[test]
    fn the_journal_names_how_avc444_codes_its_views() {
        let mut encoder = x264_avc444();
        let line = |e: &Ffmpeg| e.views_line();
        assert_eq!(
            line(&encoder),
            "AVC444 views: two Software H.264 encoders, merged"
        );
        encoder.merge = None;
        assert_eq!(
            line(&encoder),
            "AVC444 views: one Software H.264 encoder, merging them failed"
        );
        encoder.set_split_encoders(false, 1);
        assert_eq!(
            line(&encoder),
            "AVC444 views: one Software H.264 encoder, avc444_split_encoders is off"
        );
        let mut capped = x264_avc444();
        capped.backend = Backend::Nvenc;
        capped.set_split_encoders(true, 5);
        assert_eq!(
            line(&capped),
            "AVC444 views: one NVENC H.264 encoder, 5 screens need 10 sessions, NVENC has 8"
        );
    }

    #[test]
    fn a_connection_past_four_screens_codes_each_on_one_gpu_encoder_where_sessions_are_capped() {
        assert!(split_encoders(Backend::Nvenc, true, 4), "8 sessions fit");
        assert!(
            !split_encoders(Backend::Nvenc, true, 5),
            "10 sessions don't"
        );
        for backend in [Backend::Vaapi { low_power: false }, Backend::X264] {
            assert!(
                split_encoders(backend, true, 16),
                "{backend:?} has no session cap"
            );
        }
        assert!(
            !split_encoders(Backend::X264, false, 1),
            "the key turns them off"
        );
    }

    #[test]
    fn damage_grows_to_the_target_pixels_it_reaches() {
        let r = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        let damage = [r(0, 0, 1, 1), r(100, 50, 20, 10), r(1270, 710, 10, 10)];
        assert_eq!(
            scaled(&damage, (1280, 720), (640, 360)),
            [r(0, 0, 2, 2), r(48, 23, 14, 9), r(633, 353, 7, 7)],
            "half size: a pixel and its sampled neighbours, inside the target"
        );
        assert_eq!(
            scaled(&damage[1..2], (640, 360), (1280, 720)),
            [r(197, 97, 46, 26)],
            "double size"
        );
        assert_eq!(scaled(&damage, (64, 32), (64, 32)), damage, "unscaled");
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
