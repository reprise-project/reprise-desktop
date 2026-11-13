//! H.264 for the file: one backend's encoder at a constant quality with its parameter sets in the
//! codec context, for the container's header, fed stream frames at their wall-clock times.

use ffmpeg::{Dictionary, Packet, ffi, frame};
use reprisevideo::encode::EncodeError;
use reprisevideo::encode::hwmap::{self, Converter, Devices};
use reprisevideo::encode::probe::Backend;
use reprisevideo::stream::{Frame, FrameBuffer};
use std::path::Path;

/// The encoders' time base: milliseconds, so a still screen adds no pictures.
pub const TIME_BASE: (i32, i32) = (1, 1000);
/// The frame rate the encoders plan their rate control for.
const RATE_HINT: u32 = 60;
/// Pictures from one IDR to the next.
const GOP: u32 = 250;
/// x264's constant rate factor and the hardware encoders' constant QP.
const CRF: &str = "23";
const QP: &str = "24";

pub struct Movie {
    backend: Backend,
    converter: Converter,
    encoder: ffmpeg::encoder::video::Encoder,
    /// The last picture sent, which [`Movie::finish`] holds up to the end.
    last: Option<(frame::Video, i64)>,
}

impl Movie {
    /// An encoder of `backend` on render node `device` for frames of `fourcc` read at `size`
    /// from `origin`, coded at `size` with odd sides made even.
    pub fn open(
        backend: Backend,
        device: &Path,
        fourcc: u32,
        size: (u32, u32),
        origin: (u32, u32),
    ) -> Result<Movie, EncodeError> {
        let devices = match backend.gpu() {
            Some(gpu) => Some(Devices::open(gpu, device)?),
            None => None,
        };
        let coded = hwmap::even(size);
        let mut converter = Converter::new(devices.as_ref(), fourcc, coded, coded, RATE_HINT)?;
        if origin != (0, 0) {
            converter.crop_at(origin)?;
        }
        let name = backend.codec();
        let codec = ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| EncodeError(format!("FFmpeg has no {name}")))?;
        let mut ctx = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|e| EncodeError(format!("{name}: {e}")))?;
        ctx.set_width(coded.0);
        ctx.set_height(coded.1);
        ctx.set_format(converter.pixel());
        ctx.set_time_base(TIME_BASE);
        ctx.set_frame_rate(Some((RATE_HINT as i32, 1)));
        ctx.set_gop(GOP);
        ctx.set_max_b_frames(0);
        ctx.set_colorspace(ffmpeg::color::Space::BT709);
        ctx.set_color_range(reprisevideo::encode::colour::RANGE);
        ctx.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        let frames = converter.frames();
        // SAFETY: fields of a context not yet opened; it takes its own frames reference.
        unsafe {
            let c = ctx.as_mut_ptr();
            (*c).color_primaries = ffi::AVColorPrimaries::BT709;
            (*c).color_trc = ffi::AVColorTransferCharacteristic::BT709;
            (*c).thread_count = 0;
            if let Some(frames) = frames {
                (*c).hw_frames_ctx = ffi::av_buffer_ref(frames);
            }
        }
        let encoder = ctx
            .open_with(options(backend))
            .map_err(|e| EncodeError(format!("{name}: {e}")))?;
        Ok(Movie {
            backend,
            converter,
            encoder,
            last: None,
        })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn encoder(&self) -> &ffmpeg::encoder::video::Encoder {
        &self.encoder
    }

    /// Whether frames like `frame` go on into this encoder: the same size and format.
    pub fn takes(&self, frame: &Frame, size: (u32, u32)) -> bool {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        fourcc == self.converter.fourcc && hwmap::even(size) == self.converter.size
    }

    /// `frame` as the picture shown from `pts` ms on; the packets ready go to `out`.
    pub fn code(
        &mut self,
        frame: &Frame,
        pts: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        let picture = self.converter.convert(frame)?;
        self.send(picture, pts, out)
    }

    /// Hold the last picture up to `end` ms, then drain the encoder into `out`.
    pub fn finish(&mut self, end: i64, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        if let Some((picture, pts)) = self.last.take()
            && end > pts
        {
            self.send(picture, end, out)?;
        }
        let err = |e| EncodeError(format!("{}: {e}", self.backend.codec()));
        self.encoder.send_eof().map_err(err)?;
        self.receive(out)
    }

    fn send(
        &mut self,
        mut picture: frame::Video,
        pts: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        picture.set_pts(Some(pts));
        let codec = self.backend.codec();
        (self.encoder.send_frame(&picture)).map_err(|e| EncodeError(format!("{codec}: {e}")))?;
        self.last = Some((picture, pts));
        self.receive(out)
    }

    fn receive(&mut self, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => out.push(packet),
                Err(ffmpeg::Error::Other {
                    errno: libc::EAGAIN,
                })
                | Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(e) => return Err(EncodeError(format!("{}: {e}", self.backend.codec()))),
            }
        }
    }
}

/// Each backend's options for a file: x264 at a constant rate factor, the hardware encoders at a
/// constant QP.
fn options(backend: Backend) -> Dictionary {
    let mut o = Dictionary::new();
    let pairs: &[(&str, &str)] = match backend {
        Backend::Vaapi { low_power } => &[
            ("rc_mode", "CQP"),
            ("qp", QP),
            ("low_power", if low_power { "1" } else { "0" }),
        ],
        Backend::Nvenc => &[("rc", "constqp"), ("qp", QP), ("preset", "p4")],
        Backend::Vulkan => &[("rc_mode", "cqp"), ("qp", QP)],
        Backend::X264 => &[("preset", "veryfast"), ("crf", CRF)],
    };
    for (k, v) in pairs {
        o.set(k, v);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisevideo::stream::XRGB8888;

    /// A `size` frame whose every pixel is (`r`, `g`, `b`).
    fn frame(size: (u32, u32), (r, g, b): (u8, u8, u8)) -> Frame {
        let data = (0..size.0 * size.1).flat_map(|_| [b, g, r, 0]).collect();
        Frame::detached(data, size, Vec::new())
    }

    fn x264(size: (u32, u32), origin: (u32, u32)) -> Movie {
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let node = Path::new(reprisevideo::encode::probe::RENDER_NODE);
        Movie::open(Backend::X264, node, XRGB8888, size, origin).unwrap()
    }

    #[test]
    fn the_parameter_sets_are_in_the_header_and_pictures_keep_their_times() {
        let mut movie = x264((64, 48), (0, 0));
        // SAFETY: a field of the open encoder's context.
        let extradata = unsafe { (*movie.encoder().as_ptr()).extradata_size };
        assert!(extradata > 0, "SPS and PPS for the container's header");
        let mut out = Vec::new();
        movie
            .code(&frame((64, 48), (200, 40, 40)), 0, &mut out)
            .unwrap();
        movie
            .code(&frame((64, 48), (40, 200, 40)), 250, &mut out)
            .unwrap();
        movie.finish(1000, &mut out).unwrap();
        let pts: Vec<_> = out.iter().map(|p| p.pts()).collect();
        assert_eq!(pts, [Some(0), Some(250), Some(1000)]);
        assert!(out[0].is_key());
    }

    #[test]
    fn an_odd_area_is_coded_one_pixel_smaller_and_read_from_its_origin() {
        let movie = x264((33, 21), (8, 4));
        let size = (movie.encoder().width(), movie.encoder().height());
        assert_eq!(size, (32, 20));
        assert!(movie.takes(&frame((64, 48), (0, 0, 0)), (33, 21)));
        assert!(!movie.takes(&frame((64, 48), (0, 0, 0)), (40, 21)));
    }
}
