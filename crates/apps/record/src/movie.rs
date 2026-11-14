//! The video for the file: one backend's encoder of the codec set, its parameter sets in the
//! codec context for the container's header, fed stream frames at their wall-clock times.

use ffmpeg::{Packet, frame};
use reprisevideo::encode::hwmap::{self, Converter, Devices};
use reprisevideo::encode::plain::{Plain, Setup};
use reprisevideo::encode::probe::Backend;
use reprisevideo::encode::rate::RateControl;
use reprisevideo::encode::{Codec, EncodeError};
use reprisevideo::stream::{Frame, FrameBuffer};
use std::path::Path;

/// The encoders' time base: milliseconds, so a still screen adds no pictures.
pub const TIME_BASE: (i32, i32) = (1, 1000);

/// How the pictures are coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coding {
    pub codec: Codec,
    pub rate: RateControl,
    /// The frame rate the rate control plans for.
    pub rate_hint: u32,
    /// Seconds after a keyframe from which the next picture is one.
    pub keyframe_s: u32,
}

pub struct Movie {
    backend: Backend,
    codec: Codec,
    /// The size of the frames read.
    frame: (u32, u32),
    converter: Converter,
    encoder: Plain,
    keyframe_ms: i64,
    /// The pts of the last keyframe coded or forced.
    last_key: Option<i64>,
    /// The last picture sent, which [`Movie::repeat`] and [`Movie::finish`] send again.
    last: Option<(frame::Video, i64)>,
}

impl Movie {
    /// An encoder of `backend` on render node `device` for `frame`-sized frames of `fourcc`,
    /// read at `size` from `origin` and coded at `size` with odd sides made even. A hardware
    /// encoder gets the part cut out on its GPU.
    pub fn open(
        backend: Backend,
        coding: &Coding,
        device: &Path,
        fourcc: u32,
        frame: (u32, u32),
        size: (u32, u32),
        origin: (u32, u32),
    ) -> Result<Movie, EncodeError> {
        let devices = match backend.gpu() {
            Some(gpu) => Some(Devices::open(gpu, device)?),
            None => None,
        };
        let coded = hwmap::even(size);
        let rate_hint = coding.rate_hint.max(1);
        let mut converter = Converter::cropped(devices.as_ref(), fourcc, origin, coded, rate_hint)?;
        let setup = Setup {
            codec: coding.codec,
            rate: coding.rate,
            frame_rate: rate_hint,
            gop: rate_hint * coding.keyframe_s.max(1),
            time_base: TIME_BASE,
        };
        let frames = converter.frames();
        let encoder = Plain::open(backend, &setup, coded, converter.pixel(), frames)?;
        Ok(Movie {
            backend,
            codec: coding.codec,
            frame,
            converter,
            encoder,
            keyframe_ms: i64::from(coding.keyframe_s.max(1)) * 1000,
            last_key: None,
            last: None,
        })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Whether the part recorded is cut out on the GPU rather than the CPU.
    pub fn cut_on_gpu(&self) -> bool {
        self.converter.on_gpu()
    }

    pub fn encoder(&self) -> &ffmpeg::encoder::video::Encoder {
        self.encoder.encoder()
    }

    /// Whether frames like `frame`, read at `size`, go on into this encoder: the same sizes and
    /// format.
    pub fn takes(&self, frame: &Frame, size: (u32, u32)) -> bool {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        (fourcc, frame.size) == (self.converter.fourcc, self.frame)
            && hwmap::even(size) == self.converter.size
    }

    /// `frame` as the picture shown from `pts` ms on; the packets ready go to `out`.
    pub fn code(
        &mut self,
        frame: &Frame,
        pts: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        let mut picture = self.converter.convert(frame)?;
        self.send(&mut picture, pts, out)?;
        self.last = Some((picture, pts));
        Ok(())
    }

    /// The last picture again, shown from `pts` ms on; nothing before the first.
    pub fn repeat(&mut self, pts: i64, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        if let Some((mut picture, _)) = self.last.take() {
            let sent = self.send(&mut picture, pts, out);
            self.last = Some((picture, pts));
            sent?;
        }
        Ok(())
    }

    /// `picture` from `pts` on, a keyframe once the interval has passed since the last: the GOP
    /// counts pictures, and a capped or changing-only movie has fewer than its rate hint.
    fn send(
        &mut self,
        picture: &mut frame::Video,
        pts: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        let key = self.last_key.is_none_or(|k| pts - k >= self.keyframe_ms);
        picture.set_kind(match key {
            true => ffmpeg::picture::Type::I,
            false => ffmpeg::picture::Type::None,
        });
        let from = out.len();
        self.encoder.send(picture, pts, out)?;
        let coded = out[from..]
            .iter()
            .filter(|p| p.is_key())
            .filter_map(|p| p.pts());
        self.last_key = self.last_key.max(coded.max());
        if key {
            self.last_key = Some(pts);
        }
        Ok(())
    }

    /// Hold the last picture up to `end` ms, then drain the encoder into `out`.
    pub fn finish(&mut self, end: i64, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        if let Some((mut picture, pts)) = self.last.take()
            && end > pts
        {
            self.send(&mut picture, end, out)?;
        }
        self.encoder.finish(out)
    }
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

    fn coding(codec: Codec, rate: RateControl) -> Coding {
        Coding {
            codec,
            rate,
            rate_hint: 60,
            keyframe_s: 4,
        }
    }

    fn open_with(coding: Coding, size: (u32, u32), origin: (u32, u32)) -> Movie {
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let node = Path::new(reprisevideo::encode::probe::RENDER_NODE);
        let frame = (size.0.max(64), size.1.max(48));
        Movie::open(
            Backend::Software,
            &coding,
            node,
            XRGB8888,
            frame,
            size,
            origin,
        )
        .unwrap()
    }

    fn x264(size: (u32, u32), origin: (u32, u32)) -> Movie {
        let quality = RateControl::Quality(reprisevideo::encode::rate::DEFAULT_QUALITY);
        open_with(coding(Codec::H264, quality), size, origin)
    }

    fn software(codec: Codec, rate: RateControl, size: (u32, u32)) -> Movie {
        open_with(coding(codec, rate), size, (0, 0))
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
        assert!(!movie.cut_on_gpu());
        assert!(movie.takes(&frame((64, 48), (0, 0, 0)), (33, 21)));
        assert!(!movie.takes(&frame((64, 48), (0, 0, 0)), (40, 21)));
        assert!(
            !movie.takes(&frame((80, 48), (0, 0, 0)), (33, 21)),
            "a frame of another size"
        );
    }

    #[test]
    fn every_codec_codes_pictures_on_the_cpu() {
        for codec in Codec::ALL {
            let mut movie = software(codec, RateControl::Quality(55), (64, 64));
            let mut out = Vec::new();
            movie
                .code(&frame((64, 64), (200, 40, 40)), 0, &mut out)
                .unwrap();
            movie.finish(500, &mut out).unwrap();
            let pts: Vec<_> = out.iter().map(|p| p.pts()).collect();
            assert_eq!(pts, [Some(0), Some(500)], "{}", codec.label());
            assert!(out[0].is_key(), "{}", codec.label());
            assert_eq!(movie.codec(), codec);
        }
    }

    #[test]
    fn a_repeat_codes_the_last_picture_again_at_its_own_time() {
        let mut movie = software(Codec::H264, RateControl::Constant(2000), (64, 64));
        let mut out = Vec::new();
        movie.repeat(10, &mut out).unwrap();
        assert!(out.is_empty(), "nothing to repeat yet");
        movie
            .code(&frame((64, 64), (40, 200, 40)), 0, &mut out)
            .unwrap();
        movie.repeat(33, &mut out).unwrap();
        movie.finish(66, &mut out).unwrap();
        let pts: Vec<_> = out.iter().map(|p| p.pts()).collect();
        assert_eq!(pts, [Some(0), Some(33), Some(66)]);
    }

    #[test]
    fn the_gop_is_the_keyframe_interval_at_the_rate_hint() {
        let coding = Coding {
            codec: Codec::H264,
            rate: RateControl::Quality(55),
            rate_hint: 30,
            keyframe_s: 4,
        };
        let movie = open_with(coding, (64, 48), (0, 0));
        // SAFETY: a field of the open encoder's context.
        assert_eq!(unsafe { (*movie.encoder().as_ptr()).gop_size }, 120);
    }

    #[test]
    fn sparse_changes_get_a_keyframe_once_the_interval_has_passed() {
        let rates = [
            RateControl::Quality(55),
            RateControl::Average(2000),
            RateControl::Constant(2000),
        ];
        for (codec, rate) in Codec::ALL.into_iter().flat_map(|c| rates.map(|r| (c, r))) {
            let coding = Coding {
                codec,
                rate,
                rate_hint: 30,
                keyframe_s: 1,
            };
            let mut movie = open_with(coding, (64, 64), (0, 0));
            let mut out = Vec::new();
            for (i, ms) in (0..=2400).step_by(400).enumerate() {
                let v = 100 + 3 * i as u8;
                movie
                    .code(&frame((64, 64), (v, v, v)), ms, &mut out)
                    .unwrap();
            }
            movie.finish(2400, &mut out).unwrap();
            let keys: Vec<_> = (out.iter())
                .filter(|p| p.is_key())
                .map(|p| p.pts())
                .collect();
            assert_eq!(
                keys,
                [Some(0), Some(1200), Some(2400)],
                "{} {rate:?}: seven pictures, far under the GOP of 30",
                codec.label()
            );
        }
    }
}
