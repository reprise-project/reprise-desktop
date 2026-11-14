//! An encoder of one codec opened by name on a backend, for a file or the probe's test encode:
//! BT.709 full range, no B-frames, its parameter sets in the codec context.

use super::colour;
use super::probe::Backend;
use super::rate::{self, RateControl};
use super::{Codec, EncodeError};
use ::ffmpeg::format::Pixel;
use ::ffmpeg::{Dictionary, Packet, ffi, frame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub codec: Codec,
    pub rate: RateControl,
    /// The frame rate the rate control plans for.
    pub frame_rate: u32,
    /// Pictures from one keyframe to the next.
    pub gop: u32,
    pub time_base: (i32, i32),
}

pub struct Plain {
    name: &'static str,
    encoder: ::ffmpeg::encoder::video::Encoder,
}

impl Plain {
    /// `backend`'s encoder of `setup.codec` for `coded` pictures in `pixel`, taken from the
    /// hardware `frames` on a GPU.
    pub fn open(
        backend: Backend,
        setup: &Setup,
        coded: (u32, u32),
        pixel: Pixel,
        frames: Option<*mut ffi::AVBufferRef>,
    ) -> Result<Plain, EncodeError> {
        let name = backend.encoder(setup.codec);
        let err = |e: ::ffmpeg::Error| EncodeError(format!("{name}: {e}"));
        let codec = ::ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| EncodeError(format!("FFmpeg has no {name}")))?;
        let mut ctx = ::ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(err)?;
        ctx.set_width(coded.0);
        ctx.set_height(coded.1);
        ctx.set_format(pixel);
        ctx.set_time_base(setup.time_base);
        ctx.set_frame_rate(Some((setup.frame_rate.max(1) as i32, 1)));
        ctx.set_gop(setup.gop);
        ctx.set_max_b_frames(0);
        ctx.set_colorspace(::ffmpeg::color::Space::BT709);
        ctx.set_color_range(colour::RANGE);
        ctx.set_flags(::ffmpeg::codec::Flags::GLOBAL_HEADER);
        let rate = rate::rate(backend, setup.codec, setup.rate);
        rate::apply(&rate, &mut ctx);
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
        let mut options = Dictionary::new();
        for (k, v) in &rate.options {
            options.set(k, v);
        }
        let encoder = ctx.open_with(options).map_err(err)?;
        Ok(Plain { name, encoder })
    }

    pub fn encoder(&self) -> &::ffmpeg::encoder::video::Encoder {
        &self.encoder
    }

    /// `picture` as the one shown from `pts` on; the packets ready go to `out`.
    pub fn send(
        &mut self,
        picture: &mut frame::Video,
        pts: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        picture.set_pts(Some(pts));
        (self.encoder.send_frame(picture)).map_err(|e| self.error(e))?;
        self.receive(out)
    }

    /// Drain the encoder into `out`.
    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        self.encoder.send_eof().map_err(|e| self.error(e))?;
        self.receive(out)
    }

    fn receive(&mut self, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => out.push(packet),
                Err(::ffmpeg::Error::Other {
                    errno: libc::EAGAIN,
                })
                | Err(::ffmpeg::Error::Eof) => return Ok(()),
                Err(e) => return Err(self.error(e)),
            }
        }
    }

    fn error(&self, e: ::ffmpeg::Error) -> EncodeError {
        EncodeError(format!("{}: {e}", self.name))
    }
}
