//! What plays, as AAC: 16-bit stereo samples from the default sink's monitor gathered into the
//! encoder's frames, stamped by their count from where the sound starts in the movie.

use ffmpeg::format::{Sample, sample::Type};
use ffmpeg::{ChannelLayout, ChannelLayoutMask, Packet, frame};

pub const RATE: u32 = 48_000;
/// Bytes in one stereo frame of 16-bit samples.
const FRAME_BYTES: usize = 4;
const BIT_RATE: usize = 160_000;

pub struct Sound {
    encoder: ffmpeg::encoder::audio::Encoder,
    /// Samples per channel in each frame the encoder takes.
    frame_size: usize,
    /// Samples not yet coded, each channel apart.
    left: Vec<f32>,
    right: Vec<f32>,
    /// The time, in samples, of the first one in `left`; None until the first chunk.
    pts: Option<i64>,
}

impl Sound {
    pub fn open() -> Result<Sound, String> {
        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC).ok_or("FFmpeg has no AAC")?;
        let mut ctx = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|e| format!("AAC: {e}"))?;
        ctx.set_rate(RATE as i32);
        ctx.set_ch_layout(ChannelLayout::STEREO);
        ctx.set_format(Sample::F32(Type::Planar));
        ctx.set_bit_rate(BIT_RATE);
        ctx.set_time_base((1, RATE as i32));
        ctx.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        let encoder = ctx.open_as(codec).map_err(|e| format!("AAC: {e}"))?;
        let frame_size = (encoder.frame_size() as usize).max(1);
        Ok(Sound {
            encoder,
            frame_size,
            left: Vec::new(),
            right: Vec::new(),
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
            let l = i16::from_le_bytes([s[0], s[1]]);
            let r = i16::from_le_bytes([s[2], s[3]]);
            self.left.push(f32::from(l) / 32768.0);
            self.right.push(f32::from(r) / 32768.0);
        }
        while self.left.len() >= self.frame_size {
            self.send(self.frame_size, out)?;
        }
        Ok(())
    }

    /// Code what is left, padded with silence to a whole frame, and drain the encoder.
    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<(), String> {
        if !self.left.is_empty() {
            let n = self.left.len();
            self.left.resize(self.frame_size, 0.0);
            self.right.resize(self.frame_size, 0.0);
            self.send(n, out)?;
        }
        self.encoder.send_eof().map_err(|e| format!("AAC: {e}"))?;
        self.receive(out)
    }

    /// The first `frame_size` samples as a frame, of which `real` are recorded rather than padding.
    fn send(&mut self, real: usize, out: &mut Vec<Packet>) -> Result<(), String> {
        let n = self.frame_size;
        let mut f = frame::Audio::new(Sample::F32(Type::Planar), n, ChannelLayoutMask::STEREO);
        f.set_rate(RATE);
        for (plane, samples) in [&self.left, &self.right].into_iter().enumerate() {
            // SAFETY: a planar frame just allocated for `n` samples a channel: each channel's
            // plane holds `n` floats. Only the first plane's size is in `linesize`.
            unsafe {
                let to = (*f.as_mut_ptr()).data[plane] as *mut f32;
                std::ptr::copy_nonoverlapping(samples.as_ptr(), to, n);
            }
        }
        let pts = self.pts.unwrap_or(0);
        f.set_pts(Some(pts));
        self.pts = Some(pts + real as i64);
        self.left.drain(..n);
        self.right.drain(..n);
        self.encoder
            .send_frame(&f)
            .map_err(|e| format!("AAC: {e}"))?;
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
                Err(e) => return Err(format!("AAC: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_stamped_from_where_the_sound_starts() {
        let mut sound = Sound::open().unwrap();
        let n = sound.frame_size;
        let mut out = Vec::new();
        let tone: Vec<u8> = (0..n * 3)
            .flat_map(|i| {
                let v = ((i as f32 / 20.0).sin() * 8000.0) as i16;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect();
        sound.code(&tone, 500, &mut out).unwrap();
        sound.finish(&mut out).unwrap();
        let first = out.iter().filter_map(|p| p.pts()).min().unwrap();
        // The encoder's priming moves its first packet a frame earlier.
        assert!(
            (24_000 - n as i64..=24_000).contains(&first),
            "500 ms in, got {first}"
        );
        assert!(out.len() >= 3);
    }
}
