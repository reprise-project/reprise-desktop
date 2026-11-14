//! What plays, as AAC, Opus or FLAC: 16-bit stereo samples from the default sink's monitor
//! gathered into the encoder's frames, stamped by their count from where the sound starts in the
//! movie and kept to the clock each chunk came by.

use std::collections::VecDeque;

use ffmpeg::format::{Sample, sample::Type};
use ffmpeg::{ChannelLayout, ChannelLayoutMask, Dictionary, Packet, ffi, frame};
use repriseconf::recording::AudioCodec;

pub const RATE: u32 = 48_000;
/// Bytes in one stereo frame of 16-bit samples.
const FRAME_BYTES: usize = 4;
/// How far, in samples, the sound may run from the clock before it is put back on it: a gap in
/// the capture is filled with silence, sound running ahead is trimmed.
const SLACK: i64 = RATE as i64 * 40 / 1000;
/// The most silence, in samples, one chunk codes; a longer gap is filled a step per chunk.
const FILL_STEP: i64 = RATE as i64;

pub struct Sound {
    codec: AudioCodec,
    encoder: ffmpeg::encoder::audio::Encoder,
    /// Samples per channel in each frame the encoder takes.
    frame_size: usize,
    /// Samples not yet coded, left and right in turn.
    samples: Vec<i16>,
    /// The time, in samples, of the first one in `samples`; None until the first chunk.
    pts: Option<i64>,
    /// A chunk that came later than the sound so far ends, and when it plays: held until the next
    /// tells a gap before it from a late delivery.
    late: Option<(Vec<u8>, i64)>,
    /// Silence, in samples, still to code before `waiting`.
    owed: i64,
    /// Chunks behind the owed silence and when they play; None for a held chunk already placed.
    waiting: VecDeque<(Vec<u8>, Option<i64>)>,
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
            late: None,
            owed: 0,
            waiting: VecDeque::new(),
        })
    }

    pub fn encoder(&self) -> &ffmpeg::encoder::audio::Encoder {
        &self.encoder
    }

    /// `pcm` (S16LE stereo), whose first sample plays `at_ms` into the movie. Chunks follow on
    /// while they keep within [`SLACK`] of their times. Packets ready go to `out`.
    pub fn code(&mut self, pcm: &[u8], at_ms: i64, out: &mut Vec<Packet>) -> Result<(), String> {
        let at = at_ms.max(0) * i64::from(RATE) / 1000;
        self.pts.get_or_insert(at);
        self.waiting.push_back((pcm.to_vec(), Some(at)));
        self.drain(FILL_STEP, out)
    }

    /// Code at most `fill` samples of the owed silence, and the chunks waiting once it is coded.
    fn drain(&mut self, mut fill: i64, out: &mut Vec<Packet>) -> Result<(), String> {
        loop {
            let n = self.owed.min(fill);
            self.silence(n, out)?;
            (self.owed, fill) = (self.owed - n, fill - n);
            if self.owed > 0 {
                return Ok(());
            }
            let Some((pcm, at)) = self.waiting.pop_front() else {
                return Ok(());
            };
            match at {
                Some(at) => self.place(pcm, at),
                None => self.push(&pcm),
            }
            while self.samples.len() >= 2 * self.frame_size {
                self.send(self.frame_size, out)?;
            }
        }
    }

    /// Put `pcm`, which plays at `at`, after the sound so far: a chunk later than that is held
    /// until the next tells a gap, which is owed as silence, and sound running ahead is trimmed.
    fn place(&mut self, pcm: Vec<u8>, at: i64) {
        if let Some((held, held_at)) = self.late.take() {
            let after = self.end() + (held.len() / FRAME_BYTES) as i64;
            let gap = (held_at - self.end()).min(at - after);
            if gap > SLACK {
                self.owed = gap;
                self.waiting.push_front((pcm, Some(at)));
                self.waiting.push_front((held, None));
                return;
            }
            self.push(&held);
        }
        match at - self.end() {
            ahead if ahead > SLACK => self.late = Some((pcm, at)),
            behind if behind < -SLACK => {
                self.push(&pcm);
                let n = (-behind as usize).min(self.samples.len() / 2);
                self.samples.drain(..2 * n);
            }
            _ => self.push(&pcm),
        }
    }

    /// The time, in samples, the next sample takes.
    fn end(&self) -> i64 {
        self.pts.unwrap_or(0) + (self.samples.len() / 2) as i64
    }

    fn push(&mut self, pcm: &[u8]) {
        for s in pcm.as_chunks::<FRAME_BYTES>().0 {
            self.samples.push(i16::from_le_bytes([s[0], s[1]]));
            self.samples.push(i16::from_le_bytes([s[2], s[3]]));
        }
    }

    /// `n` samples of silence, coded a frame at a time.
    fn silence(&mut self, mut n: i64, out: &mut Vec<Packet>) -> Result<(), String> {
        while n > 0 {
            let room = self.frame_size - self.samples.len() / 2 % self.frame_size;
            let take = (n as usize).min(room);
            self.samples.resize(self.samples.len() + 2 * take, 0);
            n -= take as i64;
            while self.samples.len() >= 2 * self.frame_size {
                self.send(self.frame_size, out)?;
            }
        }
        Ok(())
    }

    /// Code what is left as a last, shorter frame and drain the encoder.
    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<(), String> {
        self.drain(i64::MAX, out)?;
        if let Some((held, held_at)) = self.late.take() {
            let gap = held_at - self.end();
            if gap > SLACK {
                self.silence(gap, out)?;
            }
            self.push(&held);
        }
        while self.samples.len() >= 2 * self.frame_size {
            self.send(self.frame_size, out)?;
        }
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

    /// `samples` stereo frames of `v` in both channels.
    fn flat(samples: usize, v: i16) -> Vec<u8> {
        (0..samples)
            .flat_map(|_| [v.to_le_bytes(), v.to_le_bytes()].concat())
            .collect()
    }

    /// The left channel of FLAC `packets` from `sound`.
    fn decoded(sound: &Sound, packets: &[Packet]) -> Vec<i16> {
        let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::FLAC).unwrap();
        let mut ctx = ffmpeg::codec::Context::new_with_codec(codec);
        ctx.set_parameters(ffmpeg::codec::Parameters::from(sound.encoder()))
            .unwrap();
        let mut decoder = ctx.decoder().audio().unwrap();
        let (mut got, mut frame) = (Vec::new(), frame::Audio::empty());
        let mut take = |decoder: &mut ffmpeg::decoder::Audio| {
            while decoder.receive_frame(&mut frame).is_ok() {
                got.extend(frame.plane::<(i16, i16)>(0).iter().map(|&(l, _)| l));
            }
        };
        for packet in packets {
            decoder.send_packet(packet).unwrap();
            take(&mut decoder);
        }
        decoder.send_eof().unwrap();
        take(&mut decoder);
        got
    }

    #[test]
    fn a_gap_in_the_capture_is_filled_with_silence() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let mut out = Vec::new();
        sound.code(&flat(4800, 1000), 0, &mut out).unwrap();
        sound.code(&flat(960, 2000), 1000, &mut out).unwrap();
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 48_000 + 960);
        assert_eq!(got.iter().position(|&v| v == 2000), Some(48_000));
        assert!(got[4800..48_000].iter().all(|&v| v == 0), "silence between");
    }

    #[test]
    fn a_gap_of_a_minute_is_filled_a_second_a_chunk() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let mut out = Vec::new();
        sound.code(&flat(960, 1000), 0, &mut out).unwrap();
        sound.code(&flat(960, 2000), 60_000, &mut out).unwrap();
        sound.code(&flat(960, 3000), 60_020, &mut out).unwrap();
        let coded = decoded(&sound, &out).len();
        assert!(coded <= 960 + 48_000, "{coded} samples coded at once");
        for i in 2..80 {
            sound
                .code(&flat(960, 3000), 60_000 + 20 * i, &mut out)
                .unwrap();
        }
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 60 * 48_000 + 80 * 960);
        assert_eq!(got.iter().position(|&v| v == 2000), Some(60 * 48_000));
        assert_eq!(got.iter().position(|&v| v == 3000), Some(60 * 48_000 + 960));
    }

    #[test]
    fn a_burst_of_chunks_after_a_stall_keeps_every_sample() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let mut out = Vec::new();
        // Taken together after 300 ms, each with the time of the cycle that queued it.
        for i in 0..20 {
            let jitter = [0, 1, -1, 2][i as usize % 4];
            sound
                .code(&flat(960, 1000 + i as i16), 20 * i + jitter, &mut out)
                .unwrap();
        }
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 20 * 960, "nothing filled in or trimmed");
        assert_eq!(got.iter().position(|&v| v == 1019), Some(19 * 960));
    }

    #[test]
    fn a_chunk_delivered_late_is_not_a_gap() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let mut out = Vec::new();
        sound.code(&flat(960, 1000), 0, &mut out).unwrap();
        sound.code(&flat(960, 2000), 80, &mut out).unwrap();
        sound.code(&flat(960, 3000), 40, &mut out).unwrap();
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 3 * 960, "nothing filled in or trimmed");
        assert_eq!(got.iter().position(|&v| v == 2000), Some(960));
    }

    #[test]
    fn sound_running_ahead_of_the_clock_is_trimmed_back() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5).unwrap();
        let mut out = Vec::new();
        // Each chunk is 970 samples a 20 ms (960 samples) apart: 1% ahead, 100 ms over 10 s.
        for i in 0..500 {
            sound.code(&flat(970, 1000), 20 * i, &mut out).unwrap();
            let lead = sound.end() - (960 * i + 970);
            assert!(
                (0..=SLACK + 10).contains(&lead),
                "chunk {i}: {lead} samples ahead"
            );
        }
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out).len() as i64;
        assert!(
            (got - 480_000).abs() <= SLACK + 10,
            "{got} samples for 10 s"
        );
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
