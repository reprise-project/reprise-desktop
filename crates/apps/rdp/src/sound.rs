//! RDPSND: while `[remote_desktop] sound` is on, what plays in the session goes to the client in
//! 20 ms waves, Opus when the client takes it, else 16-bit PCM at 44.1 kHz.

use crate::sink::Sink;
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd::server::{NegotiatedFormat, RdpsndError};
use ironrdp_server::{
    RdpsndServerHandler, RdpsndServerMessage, ServerEvent, ServerEventSender, SoundServerFactory,
};
use repriseconf::remote::RemoteDesktop;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{mpsc::UnboundedSender, watch};
use tokio::task::AbortHandle;

const PACKET_MS: u32 = 20;
/// Bytes in one stereo frame of 16-bit samples.
const FRAME_BYTES: usize = 4;
const PCM_RATE: u32 = 44_100;
const OPUS_RATE: u32 = 48_000;
const OPUS_BIT_RATE: usize = 128_000;

fn pcm() -> AudioFormat {
    AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: PCM_RATE,
        n_avg_bytes_per_sec: PCM_RATE * FRAME_BYTES as u32,
        n_block_align: FRAME_BYTES as u16,
        bits_per_sample: 16,
        data: None,
    }
}

/// Opus packets vary in size, so the average rate is the bit rate and blocks align to a byte.
fn opus() -> AudioFormat {
    AudioFormat {
        format: WaveFormat::OPUS,
        n_channels: 2,
        n_samples_per_sec: OPUS_RATE,
        n_avg_bytes_per_sec: (OPUS_BIT_RATE / 8) as u32,
        n_block_align: 1,
        bits_per_sample: 16,
        data: None,
    }
}

/// Opus where FFmpeg has libopus, then PCM. Offered whatever `sound` says, so turning it on
/// mid-session has a format to play in.
fn offered() -> Vec<AudioFormat> {
    let opus_coder = ::ffmpeg::encoder::find_by_name("libopus").is_some();
    opus_coder.then(opus).into_iter().chain([pcm()]).collect()
}

/// The format to send among those both sides have: Opus if there, else the first.
fn preferred<'a>(common: impl IntoIterator<Item = &'a AudioFormat>) -> Option<usize> {
    let common: Vec<_> = common.into_iter().collect();
    (common.iter().position(|f| f.format == WaveFormat::OPUS)).or((!common.is_empty()).then_some(0))
}

pub struct Factory {
    config: watch::Receiver<RemoteDesktop>,
    events: Option<UnboundedSender<ServerEvent>>,
}

impl Factory {
    pub fn new(config: watch::Receiver<RemoteDesktop>) -> Factory {
        Factory {
            config,
            events: None,
        }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.events = Some(sender);
    }
}

impl SoundServerFactory for Factory {
    fn build_backend(&self) -> Box<dyn RdpsndServerHandler> {
        Box::new(Backend::new(self.config.clone(), self.events.clone(), play))
    }
}

/// Whatever keeps the sound going until dropped.
type Playing = Box<dyn Send>;
type Play = fn(&AudioFormat, UnboundedSender<ServerEvent>) -> Result<Playing, String>;

/// One connection's sound: from the client's format pick until the channel goes, plays while
/// `sound` is on.
struct Backend {
    formats: Vec<AudioFormat>,
    config: watch::Receiver<RemoteDesktop>,
    events: Option<UnboundedSender<ServerEvent>>,
    play: Play,
    playing: Arc<Mutex<Option<Playing>>>,
    follow: Option<AbortHandle>,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("formats", &self.formats)
            .finish_non_exhaustive()
    }
}

impl Backend {
    fn new(
        config: watch::Receiver<RemoteDesktop>,
        events: Option<UnboundedSender<ServerEvent>>,
        play: Play,
    ) -> Backend {
        Backend {
            formats: offered(),
            config,
            events,
            play,
            playing: Arc::default(),
            follow: None,
        }
    }

    /// Play `format` now if `sound` is on, and follow `sound` until [`RdpsndServerHandler::stop`].
    fn begin(&mut self, format: &AudioFormat) -> Result<(), String> {
        let events = self.events.clone().ok_or("no event channel")?;
        let mut config = self.config.clone();
        if config.borrow_and_update().sound {
            *lock(&self.playing) = Some((self.play)(format, events.clone())?);
        }
        let (play, playing, format) = (self.play, self.playing.clone(), format.clone());
        let follow = tokio::spawn(async move {
            while config.changed().await.is_ok() {
                let sound = config.borrow_and_update().sound;
                let mut playing = lock(&playing);
                if sound && playing.is_none() {
                    match play(&format, events.clone()) {
                        Ok(p) => *playing = Some(p),
                        Err(e) => log::warn!("sound: {e}"),
                    }
                } else if !sound && playing.take().is_some() {
                    log::info!("sound: off");
                    let _ = events.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Close));
                }
            }
        });
        self.follow = Some(follow.abort_handle());
        Ok(())
    }
}

fn lock(playing: &Mutex<Option<Playing>>) -> std::sync::MutexGuard<'_, Option<Playing>> {
    playing.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Code `format` and send what plays in the session to the client through `events`.
fn play(format: &AudioFormat, events: UnboundedSender<ServerEvent>) -> Result<Playing, String> {
    let mut coder = Coder::new(format)?;
    let rate = format.n_samples_per_sec;
    let mut packets = Packets::new(rate);
    let mut sent = 0u32;
    let sink = Sink::open(rate, move |samples| {
        packets.push(samples, |pcm| {
            let Some(wave) = coder.code(pcm) else {
                return;
            };
            let at = sent.wrapping_mul(PACKET_MS);
            sent = sent.wrapping_add(1);
            let _ = events.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(wave, at)));
        });
    })
    .map_err(|e| e.to_string())?;
    let name = if format.format == WaveFormat::OPUS {
        "Opus"
    } else {
        "PCM"
    };
    log::info!("sound: {name} at {rate} Hz");
    Ok(Box::new(sink))
}

impl RdpsndServerHandler for Backend {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_format<'a>(
        &mut self,
        common: &'a [NegotiatedFormat],
    ) -> Option<&'a NegotiatedFormat> {
        common.get(preferred(common.iter().map(NegotiatedFormat::format))?)
    }

    fn start(&mut self, format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        self.begin(format.format())
            .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn RdpsndError>)
    }

    fn stop(&mut self) {
        if let Some(f) = self.follow.take() {
            f.abort();
        }
        lock(&self.playing).take();
    }
}

struct Packets {
    bytes: usize,
    pending: Vec<u8>,
}

impl Packets {
    fn new(rate: u32) -> Packets {
        let bytes = (rate * PACKET_MS / 1000) as usize * FRAME_BYTES;
        Packets {
            bytes,
            pending: Vec::with_capacity(2 * bytes),
        }
    }

    /// Add `samples` and pass on each packet they complete.
    fn push(&mut self, samples: &[u8], mut packet: impl FnMut(&[u8])) {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / self.bytes * self.bytes;
        for p in self.pending[..whole].chunks_exact(self.bytes) {
            packet(p);
        }
        self.pending.drain(..whole);
    }
}

enum Coder {
    Pcm,
    Opus(Box<Opus>),
}

impl Coder {
    fn new(format: &AudioFormat) -> Result<Coder, String> {
        match format.format {
            WaveFormat::OPUS => Opus::open().map(|o| Coder::Opus(Box::new(o))),
            _ => Ok(Coder::Pcm),
        }
    }

    fn code(&mut self, pcm: &[u8]) -> Option<Vec<u8>> {
        match self {
            Coder::Pcm => Some(pcm.to_vec()),
            Coder::Opus(o) => o.code(pcm),
        }
    }
}

/// libopus through FFmpeg: one Opus packet for each 20 ms of samples.
struct Opus {
    encoder: ::ffmpeg::encoder::audio::Encoder,
    pts: i64,
}

impl Opus {
    fn open() -> Result<Opus, String> {
        use ::ffmpeg::format::{Sample, sample::Type};
        let codec = ::ffmpeg::encoder::find_by_name("libopus").ok_or("FFmpeg has no libopus")?;
        let mut ctx = ::ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|e| format!("libopus: {e}"))?;
        ctx.set_rate(OPUS_RATE as i32);
        ctx.set_ch_layout(::ffmpeg::ChannelLayout::STEREO);
        ctx.set_format(Sample::I16(Type::Packed));
        ctx.set_bit_rate(OPUS_BIT_RATE);
        ctx.set_time_base((1, OPUS_RATE as i32));
        let mut options = ::ffmpeg::Dictionary::new();
        options.set("application", "lowdelay");
        options.set("frame_duration", PACKET_MS.to_string());
        let encoder = ctx
            .open_as_with(codec, options)
            .map_err(|e| format!("libopus: {e}"))?;
        Ok(Opus { encoder, pts: 0 })
    }

    fn code(&mut self, pcm: &[u8]) -> Option<Vec<u8>> {
        use ::ffmpeg::format::{Sample, sample::Type};
        let samples = pcm.len() / FRAME_BYTES;
        let mut frame = ::ffmpeg::frame::Audio::new(
            Sample::I16(Type::Packed),
            samples,
            ::ffmpeg::ChannelLayoutMask::STEREO,
        );
        frame.set_rate(OPUS_RATE);
        frame.data_mut(0)[..pcm.len()].copy_from_slice(pcm);
        frame.set_pts(Some(self.pts));
        self.pts += samples as i64;
        if let Err(e) = self.encoder.send_frame(&frame) {
            log::warn!("sound: libopus: {e}");
            return None;
        }
        let mut packet = ::ffmpeg::Packet::empty();
        self.encoder
            .receive_packet(&mut packet)
            .ok()
            .and_then(|()| packet.data().map(<[u8]>::to_vec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static STARTED: AtomicUsize = AtomicUsize::new(0);
    static STOPPED: AtomicUsize = AtomicUsize::new(0);

    struct Counted;

    impl Drop for Counted {
        fn drop(&mut self) {
            STOPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counted(_: &AudioFormat, _: UnboundedSender<ServerEvent>) -> Result<Playing, String> {
        STARTED.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Counted))
    }

    async fn settle() {
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn sound_follows_the_setting_while_connected() {
        let (config, rx) = watch::channel(RemoteDesktop {
            sound: false,
            ..RemoteDesktop::default()
        });
        let (events, mut sent) = tokio::sync::mpsc::unbounded_channel();
        let mut backend = Backend::new(rx, Some(events), counted);
        assert!(
            backend.get_formats().contains(&pcm()),
            "PCM is offered with sound off"
        );
        backend.begin(&pcm()).unwrap();
        assert_eq!(STARTED.load(Ordering::SeqCst), 0, "sound off plays nothing");

        config.send_modify(|c| c.sound = true);
        settle().await;
        assert_eq!(STARTED.load(Ordering::SeqCst), 1, "turned on, it plays");

        config.send_modify(|c| c.sound = false);
        settle().await;
        assert_eq!(STOPPED.load(Ordering::SeqCst), 1, "turned off, it stops");
        assert!(
            matches!(
                sent.try_recv(),
                Ok(ServerEvent::Rdpsnd(RdpsndServerMessage::Close))
            ),
            "the client is told to close its stream"
        );

        config.send_modify(|c| c.sound = true);
        settle().await;
        assert_eq!(
            STARTED.load(Ordering::SeqCst),
            2,
            "turned on again, it plays"
        );
        backend.stop();
        assert_eq!(
            STOPPED.load(Ordering::SeqCst),
            2,
            "the channel going stops it"
        );
    }

    #[test]
    fn opus_is_picked_when_offered() {
        assert_eq!(preferred(&[pcm(), opus()]), Some(1));
        assert_eq!(preferred(&[pcm()]), Some(0));
        assert_eq!(preferred(&[]), None);
    }

    #[test]
    fn pcm_packets_are_20_ms() {
        let mut packets = Packets::new(PCM_RATE);
        let mut out = Vec::new();
        for chunk in [1000, 4096, 4096, 3] {
            packets.push(&vec![1; chunk * FRAME_BYTES], |p| out.push(p.len()));
        }
        assert_eq!(
            out,
            vec![882 * FRAME_BYTES; 10],
            "882 frames are 20 ms at 44.1 kHz"
        );
        assert_eq!(packets.pending.len(), (9195 - 8820) * FRAME_BYTES);
    }

    #[test]
    fn an_opus_packet_comes_out_for_each_20_ms() {
        let mut opus = Opus::open().expect("FFmpeg has libopus");
        let pcm = vec![0; 960 * FRAME_BYTES];
        for _ in 0..3 {
            let packet = opus.code(&pcm).expect("a packet for each 20 ms");
            assert!(!packet.is_empty());
        }
    }
}
