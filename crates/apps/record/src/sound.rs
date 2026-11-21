//! What plays, as AAC, Opus or FLAC: 16-bit stereo samples from the default sink's monitor
//! gathered into the encoder's frames, stamped by their count from where the sound starts in the
//! movie and kept to the clock each chunk came by.
//!
//! With the microphone, each capture lays its chunks on a [`Lane`] of its own. What plays sets
//! the frames and their times; the microphone's samples for the same times are added in as each
//! frame is coded, a frame waiting up to [`HOLD`] for them. Microphone sound for a time already
//! coded is dropped, and a microphone that goes quiet or away leaves the frames as they are.

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

/// How long, in samples, coding waits for the microphone to reach the end of a frame of what
/// plays before it codes the frame with what the microphone has.
const HOLD: i64 = RATE as i64 / 4;
/// The most microphone sound, in samples, kept for frames not coded yet, as while what plays
/// stalls; older sound is dropped.
const MIC_LEAD: i64 = RATE as i64 * 10;
/// The share of full scale above which the mix bends its peaks under full scale.
const KNEE: f32 = 0.9;

/// Which capture a chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Played,
    Microphone,
}

pub struct Sound {
    codec: AudioCodec,
    encoder: ffmpeg::encoder::audio::Encoder,
    /// Samples per channel in each frame the encoder takes.
    frame_size: usize,
    /// What plays: its samples are the frames coded, at its times.
    played: Lane,
    /// What the microphone hears, added into each frame at the frame's times; None records none.
    mic: Option<Lane>,
}

/// One capture's chunks laid end to end on the movie's clock while they keep within [`SLACK`] of
/// their times: a gap is filled with silence, sound running ahead is trimmed.
#[derive(Default)]
struct Lane {
    /// Samples not yet coded, left and right in turn.
    samples: Vec<i16>,
    /// The time, in samples, of the first one in `samples`; None until the first chunk.
    pts: Option<i64>,
    /// A chunk that came later than the sound so far ends, and when it plays: held until the next
    /// tells a gap before it from a late delivery.
    late: Option<(Vec<u8>, i64)>,
    /// Silence, in samples, still to lay before `waiting`.
    owed: i64,
    /// Chunks behind the owed silence, when they play, and whether each is a held chunk already
    /// placed, which is put at its time rather than placed again.
    waiting: VecDeque<(Vec<u8>, i64, bool)>,
}

impl Lane {
    fn add(&mut self, pcm: &[u8], at: i64) {
        self.pts.get_or_insert(at);
        self.waiting.push_back((pcm.to_vec(), at, false));
    }

    /// Lay at most `fill` samples of the owed silence, and the chunks waiting once it is laid.
    fn lay(&mut self, mut fill: i64) {
        loop {
            let n = self.owed.min(fill);
            self.samples.resize(self.samples.len() + 2 * n as usize, 0);
            (self.owed, fill) = (self.owed - n, fill - n);
            if self.owed > 0 {
                return;
            }
            let Some((pcm, at, placed)) = self.waiting.pop_front() else {
                return;
            };
            match placed {
                true => self.put(&pcm, at),
                false => self.place(pcm, at),
            }
        }
    }

    /// [`Lane::lay`], then the held chunk after the gap before it; true once nothing is left.
    fn settle(&mut self, fill: i64) -> bool {
        self.lay(fill);
        if self.owed > 0 || !self.waiting.is_empty() {
            return false;
        }
        let Some((held, held_at)) = self.late.take() else {
            return true;
        };
        let gap = held_at - self.end();
        match gap > SLACK {
            true => {
                self.owed = gap;
                self.waiting.push_back((held, held_at, true));
            }
            false => self.waiting.push_back((held, held_at, false)),
        }
        false
    }

    /// Put `pcm`, which plays at `at`, after the sound so far: a chunk later than that is held
    /// until the next tells a gap, which is owed as silence, and sound running ahead is trimmed.
    fn place(&mut self, pcm: Vec<u8>, at: i64) {
        if let Some((held, held_at)) = self.late.take() {
            let after = self.end() + (held.len() / FRAME_BYTES) as i64;
            let gap = (held_at - self.end()).min(at - after);
            if gap > SLACK {
                self.owed = gap;
                self.waiting.push_front((pcm, at, false));
                self.waiting.push_front((held, held_at, true));
                return;
            }
            self.put(&held, held_at);
        }
        match at - self.end() {
            ahead if ahead > SLACK => self.late = Some((pcm, at)),
            _ => self.put(&pcm, at),
        }
    }

    /// Push `pcm`, which plays at `at`, trimming as much from the front as the lane runs past
    /// `at`, so a chunk for a time already coded loses that part.
    fn put(&mut self, pcm: &[u8], at: i64) {
        let behind = self.end() - at;
        self.push(pcm);
        if behind > SLACK {
            let n = (behind as usize).min(self.samples.len() / 2);
            self.samples.drain(..2 * n);
        }
    }

    /// The time, in samples, the next sample takes.
    fn end(&self) -> i64 {
        self.pts.unwrap_or(0) + (self.samples.len() / 2) as i64
    }

    /// The time up to which the lane knows its sound, the owed silence included.
    fn covers(&self) -> Option<i64> {
        self.pts.map(|_| self.end() + self.owed)
    }

    fn push(&mut self, pcm: &[u8]) {
        for s in pcm.as_chunks::<FRAME_BYTES>().0 {
            self.samples.push(i16::from_le_bytes([s[0], s[1]]));
            self.samples.push(i16::from_le_bytes([s[2], s[3]]));
        }
    }

    /// The first `n` samples a channel, which play at the lane's time, and the time after them.
    fn take(&mut self, n: usize) -> (Vec<i16>, i64) {
        let at = self.pts.unwrap_or(0);
        self.pts = Some(at + n as i64);
        (self.samples.drain(..2 * n).collect(), at)
    }

    /// Add the samples the lane has for the times `buf` (left and right in turn, from `at`) plays
    /// into it, and drop what the lane has before its end.
    fn mix_into(&mut self, at: i64, buf: &mut [i16]) {
        let n = (buf.len() / 2) as i64;
        let Some(pts) = self.pts else {
            return;
        };
        let (from, skip) = ((pts - at).clamp(0, n), (at - pts).max(0));
        let ours = self.samples.iter().skip(2 * skip as usize);
        for (to, s) in buf[2 * from as usize..].iter_mut().zip(ours) {
            *to = mix(*to, *s);
        }
        self.skip_to(at + n);
    }

    /// Drop the samples and owed silence before time `to`; a lane with nothing up to there goes on
    /// from `to`, so a chunk for a time already coded is trimmed as sound running behind.
    fn skip_to(&mut self, to: i64) {
        let Some(pts) = self.pts else {
            return;
        };
        let before = (to - pts).max(0);
        let dropped = before.min((self.samples.len() / 2) as i64);
        self.samples.drain(..2 * dropped as usize);
        self.owed -= (before - dropped).min(self.owed);
        self.pts = Some(pts.max(to));
    }
}

/// The sum of `a` and `b`, its peaks above [`KNEE`] of full scale, or above `a` where that is
/// louder, bent smoothly under full scale, so sound and microphone loud together neither wrap
/// round nor clip flat. A silent `b` leaves `a` as it is and a quiet one moves it as little.
fn mix(a: i16, b: i16) -> i16 {
    let sum = (f32::from(a) + f32::from(b)) / 32768.0;
    let knee = KNEE.max(f32::from(a).abs() / 32768.0);
    let over = sum.abs() - knee;
    let bent = match over > 0.0 {
        true => (knee + (1.0 - knee) * (over / (1.0 - knee)).tanh()).copysign(sum),
        false => sum,
    };
    (bent * 32768.0).round() as i16
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
            played: Lane::default(),
            mic: None,
        })
    }

    /// Mix what the microphone hears into the sound.
    pub fn with_microphone(mut self) -> Sound {
        self.mic = Some(Lane::default());
        self
    }

    /// Whether any of the microphone's sound came.
    pub fn heard_microphone(&self) -> bool {
        self.mic.as_ref().is_some_and(|m| m.pts.is_some())
    }

    pub fn encoder(&self) -> &ffmpeg::encoder::audio::Encoder {
        &self.encoder
    }

    /// `pcm` (S16LE stereo) from `source`, whose first sample plays `at_ms` into the movie.
    /// Chunks follow on while they keep within [`SLACK`] of their times. Packets ready go to
    /// `out`.
    pub fn code(
        &mut self,
        source: Source,
        pcm: &[u8],
        at_ms: i64,
        out: &mut Vec<Packet>,
    ) -> Result<(), String> {
        let at = at_ms.max(0) * i64::from(RATE) / 1000;
        match (source, self.mic.as_mut()) {
            (Source::Played, _) => {
                self.played.add(pcm, at);
                self.played.lay(FILL_STEP);
            }
            (Source::Microphone, Some(mic)) => {
                mic.add(pcm, at);
                mic.lay(FILL_STEP);
                let over = (mic.samples.len() / 2) as i64 - MIC_LEAD;
                if over > 0 {
                    mic.skip_to(mic.pts.unwrap_or(0) + over);
                }
            }
            (Source::Microphone, None) => return Ok(()),
        }
        self.cut(HOLD, out)
    }

    /// Code each whole frame of what plays once the microphone covers it, or once what plays runs
    /// `hold` samples past its end.
    fn cut(&mut self, hold: i64, out: &mut Vec<Packet>) -> Result<(), String> {
        let n = self.frame_size;
        while self.played.samples.len() >= 2 * n {
            let end = self.played.pts.unwrap_or(0) + n as i64;
            let waits = (self.mic.as_ref()).is_some_and(|m| m.covers().is_none_or(|c| c < end));
            let past = (self.played.samples.len() / 2 - n) as i64;
            if waits && past < hold {
                return Ok(());
            }
            self.send(n, out)?;
        }
        Ok(())
    }

    /// Code what is left as a last, shorter frame and drain the encoder. Both lanes lay their
    /// owed silence a step at a time in turn, coded as it is covered, and the microphone's stops
    /// at the end of what plays, so a long gap is never laid at once.
    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<(), String> {
        loop {
            let played = self.played.settle(FILL_STEP);
            let heard = match self.mic.as_mut() {
                Some(mic) => (played && mic.end() >= self.played.end()) || mic.settle(FILL_STEP),
                None => true,
            };
            self.cut(if heard { 0 } else { i64::MAX }, out)?;
            if played && heard {
                break;
            }
        }
        if !self.played.samples.is_empty() {
            self.send(self.played.samples.len() / 2, out)?;
        }
        (self.encoder.send_eof()).map_err(|e| format!("{}: {e}", self.codec.label()))?;
        self.receive(out)
    }

    /// The first `n` samples a channel of what plays, the microphone's added in, as a frame.
    fn send(&mut self, n: usize, out: &mut Vec<Packet>) -> Result<(), String> {
        let (mut samples, pts) = self.played.take(n);
        if let Some(mic) = self.mic.as_mut() {
            mic.mix_into(pts, &mut samples);
        }
        let format = sample_format(self.codec);
        let mut f = frame::Audio::new(format, n, ChannelLayoutMask::STEREO);
        f.set_rate(RATE);
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
                    for (to, s) in to.iter_mut().zip(&samples) {
                        *to = f32::from(*s) / 32768.0;
                    }
                }
                _ => std::ptr::copy_nonoverlapping(samples.as_ptr(), data[0] as *mut i16, 2 * n),
            }
        }
        f.set_pts(Some(pts));
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
        sound
            .code(Source::Played, &flat(4800, 1000), 0, &mut out)
            .unwrap();
        sound
            .code(Source::Played, &flat(960, 2000), 1000, &mut out)
            .unwrap();
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
        sound
            .code(Source::Played, &flat(960, 1000), 0, &mut out)
            .unwrap();
        sound
            .code(Source::Played, &flat(960, 2000), 60_000, &mut out)
            .unwrap();
        sound
            .code(Source::Played, &flat(960, 3000), 60_020, &mut out)
            .unwrap();
        let coded = decoded(&sound, &out).len();
        assert!(coded <= 960 + 48_000, "{coded} samples coded at once");
        for i in 2..80 {
            sound
                .code(Source::Played, &flat(960, 3000), 60_000 + 20 * i, &mut out)
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
                .code(
                    Source::Played,
                    &flat(960, 1000 + i as i16),
                    20 * i + jitter,
                    &mut out,
                )
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
        sound
            .code(Source::Played, &flat(960, 1000), 0, &mut out)
            .unwrap();
        sound
            .code(Source::Played, &flat(960, 2000), 80, &mut out)
            .unwrap();
        sound
            .code(Source::Played, &flat(960, 3000), 40, &mut out)
            .unwrap();
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
            sound
                .code(Source::Played, &flat(970, 1000), 20 * i, &mut out)
                .unwrap();
            let lead = sound.played.end() - (960 * i + 970);
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
            sound
                .code(Source::Played, &tone(n * 3), 500, &mut out)
                .unwrap();
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
        sound.code(Source::Played, &pcm, 0, &mut out).unwrap();
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

    /// 20 ms chunks (960 samples) of `v` from `source`, from `ms.0` to before `ms.1`.
    fn feed(sound: &mut Sound, source: Source, v: i16, ms: (i64, i64), out: &mut Vec<Packet>) {
        for at in (ms.0..ms.1).step_by(20) {
            sound.code(source, &flat(960, v), at, out).unwrap();
        }
    }

    #[test]
    fn the_mix_adds_quiet_sound_exactly_and_bends_loud_peaks_under_full_scale() {
        assert_eq!(mix(1000, 500), 1500);
        assert_eq!(mix(-20000, 5000), -15000);
        assert_eq!(
            mix(20000, 9000),
            29000,
            "under the knee, 0.885 of full scale"
        );
        assert_eq!(mix(1000, -1000), 0);
        for a in [i16::MIN, -32000, 30000, i16::MAX] {
            assert_eq!(mix(a, 0), a, "a silent microphone leaves {a} as it is");
        }
        let loud = mix(i16::MAX, i16::MAX);
        assert!((32000..=i16::MAX).contains(&loud), "{loud}");
        let low = mix(i16::MIN, i16::MIN);
        assert!((i16::MIN..=-32000).contains(&low), "{low}");
        let bent: Vec<i16> = (0..=32767).step_by(64).map(|b| mix(20000, b)).collect();
        assert!(bent.is_sorted(), "louder never comes out quieter");
        let knee = (KNEE * 32768.0) as i16;
        assert!(mix(knee, 1) - mix(knee, 0) <= 1, "no step at the knee");
    }

    #[test]
    fn a_quiet_microphone_moves_every_sample_by_as_little() {
        for a in i16::MIN..=i16::MAX {
            for b in [-1, 1] {
                let got = mix(a, b);
                assert!(
                    (i32::from(got) - i32::from(a)).abs() <= 1,
                    "mix({a}, {b}) = {got}"
                );
            }
        }
        for a in [i16::MIN, -32000, -30000, 29500, 30000, 32000, i16::MAX] {
            let row: Vec<i16> = (i16::MIN..=i16::MAX).map(|b| mix(a, b)).collect();
            assert!(
                row.is_sorted(),
                "a louder microphone never makes {a} quieter"
            );
        }
    }

    #[test]
    fn the_microphone_is_added_to_what_plays_at_the_same_times() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        let mut out = Vec::new();
        for at in (0..500).step_by(20) {
            sound
                .code(Source::Played, &flat(960, 1000), at, &mut out)
                .unwrap();
            if (100..300).contains(&at) {
                sound
                    .code(Source::Microphone, &flat(960, 300), at, &mut out)
                    .unwrap();
            }
        }
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 24_000, "what plays sets the length");
        assert!(
            got[..4800].iter().all(|&v| v == 1000),
            "before the microphone"
        );
        assert!(got[4800..14_400].iter().all(|&v| v == 1300), "both");
        assert!(got[14_400..].iter().all(|&v| v == 1000), "after it stopped");
    }

    #[test]
    fn a_silent_microphone_holds_coding_back_no_more_than_the_hold() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        let mut out = Vec::new();
        for at in (0..2000).step_by(20) {
            sound
                .code(Source::Played, &flat(960, 1000), at, &mut out)
                .unwrap();
        }
        let coded = decoded(&sound, &out).len() as i64;
        let behind = 96_000 - coded;
        let n = sound.frame_size as i64;
        assert!(
            (HOLD..HOLD + 2 * n).contains(&behind),
            "{behind} samples not coded"
        );
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 96_000);
        assert!(got.iter().all(|&v| v == 1000), "what plays as it played");
        assert!(!sound.heard_microphone());
    }

    #[test]
    fn microphone_sound_for_a_time_already_coded_is_dropped() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        let mut out = Vec::new();
        feed(&mut sound, Source::Played, 1000, (0, 1000), &mut out);
        feed(&mut sound, Source::Microphone, 300, (100, 120), &mut out);
        feed(&mut sound, Source::Microphone, 300, (900, 920), &mut out);
        sound.finish(&mut out).unwrap();
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 48_000);
        assert!(got[4800..5760].iter().all(|&v| v == 1000), "came too late");
        assert!(got[43_200..44_160].iter().all(|&v| v == 1300), "in time");
        assert_eq!(got.iter().filter(|&&v| v == 1300).count(), 960);
    }

    #[test]
    fn a_held_chunk_whose_time_was_coded_meanwhile_loses_what_was_coded() {
        let mut lane = Lane::default();
        lane.add(&flat(960, 1), 0);
        lane.lay(FILL_STEP);
        lane.add(&flat(4800, 3), 30_000);
        lane.lay(FILL_STEP);
        assert!(lane.late.is_some(), "held until the next chunk tells a gap");
        lane.skip_to(33_000);
        lane.add(&flat(960, 4), 40_000);
        lane.lay(FILL_STEP);
        assert_eq!(lane.end(), 34_800, "the held chunk's end, at its own time");
        assert_eq!(lane.samples, vec![3; 2 * 1800], "its last 1800 samples");
    }

    #[test]
    fn a_held_chunk_behind_a_long_gap_whose_time_was_coded_meanwhile_loses_what_was_coded() {
        let mut lane = Lane::default();
        lane.add(&flat(960, 1), 0);
        lane.lay(FILL_STEP);
        lane.add(&flat(4800, 3), 100_000);
        lane.lay(FILL_STEP);
        lane.add(&flat(960, 4), 200_000);
        lane.lay(FILL_STEP);
        assert!(lane.owed > 0, "a gap over a second is laid a step a chunk");
        lane.skip_to(102_000);
        assert_eq!(lane.owed, 0);
        lane.lay(FILL_STEP);
        assert_eq!(lane.end(), 104_800, "the held chunk's end, at its own time");
        assert_eq!(lane.samples, vec![3; 2 * 2800], "its last 2800 samples");
    }

    #[test]
    fn a_long_gap_in_both_captures_is_laid_a_step_at_a_time_as_the_recording_finishes() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        let mut out = Vec::new();
        for (source, v) in [(Source::Played, 1000), (Source::Microphone, 300)] {
            sound.code(source, &flat(960, v), 0, &mut out).unwrap();
            sound
                .code(source, &flat(960, v), 120_000, &mut out)
                .unwrap();
        }
        sound.finish(&mut out).unwrap();
        let mic = sound.mic.as_ref().unwrap();
        assert!(
            mic.samples.capacity() <= 8 * FILL_STEP as usize,
            "{} samples laid",
            mic.samples.capacity() / 2
        );
        let got = decoded(&sound, &out);
        assert_eq!(got.len(), 120 * 48_000 + 960);
        assert!(got[..960].iter().all(|&v| v == 1300), "both at the start");
        assert!(got[960..120 * 48_000].iter().all(|&v| v == 0), "the gap");
        assert!(
            got[120 * 48_000..].iter().all(|&v| v == 1300),
            "both after it"
        );
    }

    #[test]
    fn the_microphone_past_the_end_of_what_plays_is_not_laid() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        let mut out = Vec::new();
        sound
            .code(Source::Played, &flat(960, 1000), 0, &mut out)
            .unwrap();
        feed(&mut sound, Source::Microphone, 300, (0, 20), &mut out);
        feed(
            &mut sound,
            Source::Microphone,
            300,
            (3_600_000, 3_600_040),
            &mut out,
        );
        sound.finish(&mut out).unwrap();
        let mic = sound.mic.as_ref().unwrap();
        assert!(mic.samples.capacity() <= 8 * FILL_STEP as usize);
        assert_eq!(decoded(&sound, &out), vec![1300; 960]);
    }

    #[test]
    fn microphone_sound_waits_ten_seconds_at_most_for_what_plays() {
        let mut sound = Sound::open(AudioCodec::Flac, 160, true, 5)
            .unwrap()
            .with_microphone();
        feed(
            &mut sound,
            Source::Microphone,
            300,
            (0, 20_000),
            &mut Vec::new(),
        );
        let mic = sound.mic.as_ref().unwrap();
        assert!((mic.samples.len() / 2) as i64 <= MIC_LEAD);
        assert_eq!(mic.end(), 20 * 48_000);
    }
}
