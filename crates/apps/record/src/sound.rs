//! What plays, as AAC, Opus or FLAC: 16-bit stereo samples from the default sink's monitor
//! gathered into the encoder's frames, stamped by their count from where the sound starts in the
//! movie.

use ffmpeg::format::{Sample, sample::Type};
use ffmpeg::{ChannelLayout, ChannelLayoutMask, Dictionary, Packet, ffi, frame};
use repriseconf::recording::AudioCodec;

pub const RATE: u32 = 48_000;
/// Bytes in one stereo frame of 16-bit samples.
const FRAME_BYTES: usize = 4;

pub struct Sound {
    codec: AudioCodec,
    encoder: ffmpeg::encoder::audio::Encoder,
    /// Samples per channel in each frame the encoder takes.
    frame_size: usize,
    /// Samples not yet coded, left and right in turn.
    samples: Vec<i16>,
    /// The time, in samples, of the first one in `samples`; None until the first chunk.
    pts: Option<i64>,
}

/// The FFmpeg encoder of `codec`.
fn encoder_name(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Aac => "aac",
        AudioCodec::Opus => "libopus",
        AudioCodec::Flac => "flac",
    }
}

fn sample_format(codec: AudioCodec) -> Sample {
    match codec {
        AudioCodec::Aac => Sample::F32(Type::Planar),
        AudioCodec::Opus => Sample::F32(Type::Packed),
        AudioCodec::Flac => Sample::I16(Type::Packed),
    }
}

/// Whether `codec` records here: FFmpeg has its encoder, it opens, and the MP4 muxer takes it.
pub fn accepted(codec: AudioCodec) -> bool {
    let Ok(sound) = Sound::open(codec, 160, true, 5) else {
        return false;
    };
    let id: ffi::AVCodecID = sound.encoder.id().into();
    // SAFETY: a static muxer description, or null when FFmpeg lacks MP4.
    unsafe {
        let mp4 = ffi::av_guess_format(c"mp4".as_ptr(), std::ptr::null(), std::ptr::null());
        !mp4.is_null() && ffi::avformat_query_codec(mp4, id, ffi::FF_COMPLIANCE_NORMAL) == 1
    }
}

impl Sound {
    /// `bitrate_kbit` applies to AAC and Opus, `vbr` to Opus, `flac_compression` to FLAC.
    pub fn open(
        codec: AudioCodec,
        bitrate_kbit: u32,
        vbr: bool,
        flac_compression: u8,
    ) -> Result<Sound, String> {
        let label = codec.label();
        let name = encoder_name(codec);
        let found = ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| format!("{label}: FFmpeg has no {name}"))?;
        let mut ctx = ffmpeg::codec::Context::new_with_codec(found)
            .encoder()
            .audio()
            .map_err(|e| format!("{label}: {e}"))?;
        ctx.set_rate(RATE as i32);
        ctx.set_ch_layout(ChannelLayout::STEREO);
        ctx.set_format(sample_format(codec));
        ctx.set_time_base((1, RATE as i32));
        ctx.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        let mut options = Dictionary::new();
        match codec {
            AudioCodec::Aac => ctx.set_bit_rate(bitrate_kbit as usize * 1000),
            AudioCodec::Opus => {
                ctx.set_bit_rate(bitrate_kbit as usize * 1000);
                options.set("vbr", if vbr { "on" } else { "off" });
            }
            AudioCodec::Flac => {
                // SAFETY: a field of a context not yet opened.
                unsafe { (*ctx.as_mut_ptr()).compression_level = i32::from(flac_compression) };
            }
        }
        let encoder = ctx
            .open_as_with(found, options)
            .map_err(|e| format!("{label}: {e}"))?;
        let frame_size = (encoder.frame_size() as usize).max(1);
        Ok(Sound {
            codec,
            encoder,
            frame_size,
            samples: Vec::new(),
            pts: None,
        })
    }

    pub fn encoder(&self) -> &ffmpeg::encoder::audio::Encoder {
        &self.encoder
    }

    /// `pcm` (S16LE stereo), whose first sample plays `at_ms` into the movie on the first call;
    /// later chunks follow on. Packets ready go to `out`.
    pub fn code(&mut self, pcm: &[u8], at_ms: i64, out: &mut Vec<Packet>) -> Result<(), String> {
        self.pts
            .get_or_insert(at_ms.max(0) * i64::from(RATE) / 1000);
        for s in pcm.as_chunks::<FRAME_BYTES>().0 {
            self.samples.push(i16::from_le_bytes([s[0], s[1]]));
            self.samples.push(i16::from_le_bytes([s[2], s[3]]));
        }
        while self.samples.len() >= 2 * self.frame_size {
            self.send(self.frame_size, out)?;
        }
        Ok(())
    }

    /// Code what is left as a last, shorter frame and drain the encoder.
    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<(), String> {
        if !self.samples.is_empty() {
            self.send(self.samples.len() / 2, out)?;
        }
        (self.encoder.send_eof()).map_err(|e| format!("{}: {e}", self.codec.label()))?;
        self.receive(out)
    }

    /// The first `n` samples a channel as a frame.
    fn send(&mut self, n: usize, out: &mut Vec<Packet>) -> Result<(), String> {
        let format = sample_format(self.codec);
        let mut f = frame::Audio::new(format, n, ChannelLayoutMask::STEREO);
        f.set_rate(RATE);
        let samples = &self.samples[..2 * n];
        // SAFETY: a frame just allocated for `n` samples a channel in `format`: a planar frame has
        // `n` floats in each channel's plane, a packed one `2 * n` samples in its first plane.
        unsafe {
            let data = (*f.as_mut_ptr()).data;
            match format {
                Sample::F32(Type::Planar) => {
                    for (plane, to) in data[..2].iter().enumerate() {
                        let to = std::slice::from_raw_parts_mut(*to as *mut f32, n);
                        for (to, s) in to.iter_mut().zip(samples.iter().skip(plane).step_by(2)) {
                            *to = f32::from(*s) / 32768.0;
                        }
                    }
                }
                Sample::F32(Type::Packed) => {
                    let to = std::slice::from_raw_parts_mut(data[0] as *mut f32, 2 * n);
                    for (to, s) in to.iter_mut().zip(samples) {
                        *to = f32::from(*s) / 32768.0;
                    }
                }
                _ => std::ptr::copy_nonoverlapping(samples.as_ptr(), data[0] as *mut i16, 2 * n),
            }
        }
        let pts = self.pts.unwrap_or(0);
        f.set_pts(Some(pts));
        self.pts = Some(pts + n as i64);
        self.samples.drain(..2 * n);
        (self.encoder.send_frame(&f)).map_err(|e| format!("{}: {e}", self.codec.label()))?;
        self.receive(out)
    }

    fn receive(&mut self, out: &mut Vec<Packet>) -> Result<(), String> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => out.push(packet),
                Err(ffmpeg::Error::Other {
                    errno: libc::EAGAIN,
                })
                | Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(e) => return Err(format!("{}: {e}", self.codec.label())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(samples: usize) -> Vec<u8> {
        (0..samples)
            .flat_map(|i| {
                let v = ((i as f32 / 20.0).sin() * 8000.0) as i16;
                [v.to_le_bytes(), (v / 2).to_le_bytes()].concat()
            })
            .collect()
    }

    #[test]
    fn every_audio_codec_is_stamped_from_where_the_sound_starts() {
        for codec in AudioCodec::ALL {
            let mut sound = Sound::open(codec, 160, true, 5).unwrap();
            let n = sound.frame_size;
            let mut out = Vec::new();
            sound.code(&tone(n * 3), 500, &mut out).unwrap();
            sound.finish(&mut out).unwrap();
            let first = out.iter().filter_map(|p| p.pts()).min().unwrap();
            // AAC's priming and Opus's pre-skip move the first packet up to a frame earlier.
            assert!(
                (24_000 - n as i64..=24_000).contains(&first),
                "{}: 500 ms in, got {first}",
                codec.label()
            );
            assert!(out.len() >= 3, "{}", codec.label());
        }
    }

    #[test]
    fn the_mp4_muxer_takes_every_audio_codec_here() {
        for codec in AudioCodec::ALL {
            assert!(accepted(codec), "{}", codec.label());
        }
    }

    #[test]
    fn flac_keeps_the_samples_it_was_given_and_no_padding_after_them() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let n = sound.frame_size;
        let pcm = tone(n + 100);
        let mut out = Vec::new();
        sound.code(&pcm, 0, &mut out).unwrap();
        sound.finish(&mut out).unwrap();
        let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::FLAC).unwrap();
        let mut ctx = ffmpeg::codec::Context::new_with_codec(codec);
        ctx.set_parameters(ffmpeg::codec::Parameters::from(sound.encoder()))
            .unwrap();
        let mut decoder = ctx.decoder().audio().unwrap();
        let (mut got, mut decoded) = (Vec::new(), frame::Audio::empty());
        let mut take = |decoder: &mut ffmpeg::decoder::Audio| {
            while decoder.receive_frame(&mut decoded).is_ok() {
                assert_eq!(decoded.format(), Sample::I16(Type::Packed));
                (decoded.plane::<(i16, i16)>(0).iter())
                    .for_each(|&(l, r)| got.extend([l.to_le_bytes(), r.to_le_bytes()].concat()));
            }
        };
        for packet in &out {
            decoder.send_packet(packet).unwrap();
            take(&mut decoder);
        }
        decoder.send_eof().unwrap();
        take(&mut decoder);
        assert_eq!(
            got.len(),
            pcm.len(),
            "a whole frame and the last 100 samples"
        );
        assert_eq!(got, pcm);
    }
}
