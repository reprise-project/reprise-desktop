//! reprise-record: records a screen, or an area of it, and with `--sound` what plays, into an MP4
//! file the compositor made and handed it open, until SIGINT or SIGTERM or until the screen goes.
//! On failure its last line on stderr is the reason. Its settings are `[recording]`; with
//! `--probe` it prints what codes here and exits.

mod args;
mod fallback;
mod movie;
mod pace;
mod portal;
mod refresh;
mod sound;

use args::{Area, Args, Command};
use fallback::{Ask, Backends, Step};
use ffmpeg::Packet;
use movie::{Coding, Movie};
use pace::{Pace, Pacer};
use repriseconf::recording::{AudioCodec, Probed, RateMode, Recording};
use repriseconf::remote::EncoderChoice;
use reprisevideo::encode::probe::{self, Backend};
use reprisevideo::encode::rate::RateControl;
use reprisevideo::encode::{Codec, choice};
use reprisevideo::stream::{self, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer};
use sound::Sound;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What PipeWire's streams name the program.
const APP: &str = "Screen Recorder";

fn main() -> ExitCode {
    if std::env::var_os("SVT_LOG").is_none() {
        // SAFETY: no other thread runs yet. SVT-AV1 prints its settings and warnings otherwise.
        unsafe { std::env::set_var("SVT_LOG", "1") };
    }
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    reprisebus::journal::log_panics();
    let args = match args::parse(std::env::args_os().skip(1)) {
        Ok(Command::Record(a)) => a,
        Ok(Command::Probe) => return probe_command(),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    // Blocked before any thread starts, so only `signals` takes them.
    let stop = block_stop_signals();
    match record(&args, stop) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::warn!("recording failed: {e}");
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// `--probe`: what [`probe_text`] finds, on stdout.
fn probe_command() -> ExitCode {
    if let Err(e) = ffmpeg::init() {
        eprintln!("FFmpeg: {e}");
        return ExitCode::FAILURE;
    }
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
    let conn = wayland_client::Connection::connect_to_env().ok();
    print!("{}", probe_text(conn.as_ref()));
    ExitCode::SUCCESS
}

/// The encoder choices that code each codec, in [`Backend::ALL`]'s order, and the audio codecs
/// that record into MP4, as [`Probed::to_text`] writes them.
fn probe_text(conn: Option<&wayland_client::Connection>) -> String {
    let (_, working) = probe::probe(conn, &Codec::ALL, probe::file_test);
    let working: Vec<_> = working.into_iter().map(|(c, b, _)| (c, b)).collect();
    probed(&working, sound::accepted).to_text()
}

/// `working` as one encoder choice a codec, and the audio codecs `accepted` takes.
fn probed(working: &[(Codec, Backend)], accepted: impl Fn(AudioCodec) -> bool) -> Probed {
    let mut video = Vec::new();
    for codec in Codec::ALL {
        for backend in Backend::ALL {
            let pair = (codec, backend.choice());
            if working.contains(&(codec, backend)) && !video.contains(&pair) {
                video.push(pair);
            }
        }
    }
    let audio = AudioCodec::ALL
        .into_iter()
        .filter(|&a| accepted(a))
        .collect();
    Probed { video, audio }
}

enum Event {
    Frame(Frame, Instant),
    Sound(Vec<u8>, Instant),
    /// SIGINT or SIGTERM.
    Stop,
    /// The screen's stream ended.
    Ended,
}

fn record(args: &Args, stop: libc::sigset_t) -> Result<(), String> {
    let into = movie_url(args.fd)?;
    ffmpeg::init().map_err(|e| format!("FFmpeg: {e}"))?;
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    let settings = repriseconf::recording::recording();
    let codec = settings.codec;
    let sound = args
        .sound
        .then(|| open_sound(&settings, Sound::open))
        .transpose()?;
    let (tx, events) = mpsc::channel();
    signals(stop, tx.clone());
    let picked = std::thread::spawn({
        let (area, output) = (args.area, args.output.clone());
        move || {
            let conn = wayland_client::Connection::connect_to_env().ok();
            let refresh = conn.as_ref().and_then(|c| refresh::refresh_hz(c, &output));
            let found = encoders(conn.as_ref(), area, codec, settings.encoder);
            (found, refresh)
        }
    });
    let cast = portal::Cast::start().map_err(|e| format!("The screen could not be shared: {e}"))?;
    let screen = (cast.screen(&args.output).cloned())
        .ok_or_else(|| format!("The screen {} is not shared.", args.output))?;
    let ((device, order), refresh) = picked.join().map_err(|_| "Finding an encoder failed.")?;
    #[cfg(feature = "fallback-test")]
    let (order, forced) = forced::order(order);
    let (first, modifiers) = first_encoder(&order, codec)?;
    let making = making(&settings, refresh, sound);
    log::info!(
        "recording {}, {} first, {:?} at {} Hz",
        args.output,
        first.label(codec),
        making.coding.rate,
        making.coding.rate_hint
    );
    let fd = cast.pipewire()?;
    let (stream, mut frames, _sizes) = stream::spawn(fd, screen.node, modifiers, APP, || {})?;
    let video_tx = tx.clone();
    std::thread::Builder::new()
        .name("reprise-record-frames".into())
        .spawn(move || {
            while let Some(frame) = frames.blocking_recv() {
                if video_tx.send(Event::Frame(frame, Instant::now())).is_err() {
                    return;
                }
            }
            let _ = video_tx.send(Event::Ended);
        })
        .map_err(|e| e.to_string())?;
    let _sound = match args.sound {
        true => Some(capture_sound(tx)?),
        false => None,
    };
    let offer = stream.offer();
    let ask = move |a: &Ask| match a.again {
        false => offer.modifiers(&a.modifiers),
        true => offer.again(&a.modifiers),
    };
    let mut file = File::new(&into, &device, order, args.area, screen.size, making);
    file.ask = Box::new(ask);
    #[cfg(feature = "fallback-test")]
    {
        file.forced = forced;
    }
    let ended = file.run(&events);
    let finished = file.finish();
    drop(cast);
    match (file.started(), ended.and(finished)) {
        (false, _) => Err("No picture of the screen came.".into()),
        (true, Err(e)) if !file.started_writing() => Err(e),
        (true, Err(e)) => {
            log::warn!("the recording ended early: {e}");
            Ok(())
        }
        (true, Ok(())) => Ok(()),
    }
}

/// The path FFmpeg opens to write the movie into `fd`, an inherited regular file. Through
/// /proc the open reaches the descriptor's own file, so a name swapped in the folder meanwhile
/// is never followed; the faststart pass reopens it the same way to read it back.
fn movie_url(fd: RawFd) -> Result<PathBuf, String> {
    // SAFETY: all-zero is a valid stat, and fstat only fills it; a closed `fd` fails.
    let (done, st) = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        (libc::fstat(fd, &mut st), st)
    };
    if done != 0 || st.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(format!(
            "Descriptor {fd} is not a file to write the recording into."
        ));
    }
    Ok(PathBuf::from(format!("/proc/self/fd/{fd}")))
}

/// The first encoder of `order`, or the reason the recording cannot start.
fn first_encoder(
    order: &[(Backend, Vec<u64>)],
    codec: Codec,
) -> Result<&(Backend, Vec<u64>), String> {
    (order.first()).ok_or_else(|| format!("No {} encoder works here.", codec.label()))
}

/// The sound encoder `r` asks for, opened by `open`, or the reason the recording cannot start.
fn open_sound(
    r: &Recording,
    open: impl FnOnce(AudioCodec, u32, bool, u8) -> Result<Sound, String>,
) -> Result<Sound, String> {
    open(
        r.audio_codec,
        r.audio_bitrate,
        r.audio_vbr,
        r.flac_compression,
    )
    .map_err(|e| {
        log::warn!("{e}");
        format!("No {} encoder works here.", r.audio_codec.label())
    })
}

/// How the movie is made.
struct Making {
    coding: Coding,
    pacer: Pacer,
    /// What plays is recorded with this encoder; None records no sound.
    sound: Option<Sound>,
}

/// The most a second Constant bitrate codes at the screen's rate, so a CPU encoder keeps up on
/// a 144 or 240 Hz screen.
const CONSTANT_SCREEN_HZ: u32 = 60;

/// `settings` for a screen refreshing at `refresh_hz` (60 when unknown): the rate hint is the
/// frame rate cap or the refresh, and Constant bitrate codes every slot at that rate, at most
/// [`CONSTANT_SCREEN_HZ`] when no cap is set.
fn making(settings: &Recording, refresh_hz: Option<u32>, sound: Option<Sound>) -> Making {
    let screen_hz = refresh_hz.unwrap_or(60);
    let rate_hint = match (settings.frame_rate, settings.rate) {
        (0, RateMode::Constant) => screen_hz.min(CONSTANT_SCREEN_HZ),
        (0, _) => screen_hz,
        (cap, _) => cap,
    };
    let (rate, pacer) = match (settings.rate, settings.frame_rate) {
        (RateMode::Constant, _) => (
            RateControl::Constant(settings.bitrate),
            Pacer::new(Some(rate_hint), true),
        ),
        (mode, cap) => {
            let rate = match mode {
                RateMode::Average => RateControl::Average(settings.bitrate),
                _ => RateControl::Quality(settings.quality),
            };
            (rate, Pacer::new((cap > 0).then_some(cap), false))
        }
    };
    let coding = Coding {
        codec: settings.codec,
        rate,
        rate_hint,
        keyframe_s: settings.keyframe_interval,
    };
    Making {
        coding,
        pacer,
        sound,
    }
}

/// The render node, and the backends that code `codec` to try for `choice` in Remote Desktop's
/// order, with the stream modifiers each maps. A hardware encoder gets an area cut out on its
/// GPU; one too small for it codes on the CPU from shared memory or linear buffers.
fn encoders(
    conn: Option<&wayland_client::Connection>,
    area: Option<Area>,
    codec: Codec,
    choice: EncoderChoice,
) -> (PathBuf, Vec<(Backend, Vec<u64>)>) {
    if !hardware_takes(area) {
        log::info!(
            "the area is under {} pixels a side, so {} codes it",
            probe::TEST_SIDE,
            Backend::Software.label(codec)
        );
        let node = PathBuf::from(probe::RENDER_NODE);
        return (node, vec![(Backend::Software, vec![DRM_FORMAT_MOD_LINEAR])]);
    }
    let (device, working) = probe::probe(conn, &[codec], probe::file_test);
    let working: Vec<_> = (working.into_iter()).map(|(_, b, m)| (b, m)).collect();
    let backends: Vec<_> = working.iter().map(|(b, _)| *b).collect();
    let order = choice::order(choice, &backends);
    let with_modifiers = (order.into_iter())
        .filter_map(|b| working.iter().find(|(w, _)| *w == b).cloned())
        .collect();
    (device, with_modifiers)
}

/// Whether a hardware encoder may code `area` (None: the whole screen): the probe proved each at
/// [`probe::TEST_SIDE`] a side, and some refuse pictures much smaller.
fn hardware_takes(area: Option<Area>) -> bool {
    let side = probe::TEST_SIDE as i32;
    area.is_none_or(|a| a.width >= side && a.height >= side)
}

/// The default sink's monitor on a thread of its own, into `tx`; it ends with the returned sender.
fn capture_sound(tx: mpsc::Sender<Event>) -> Result<pipewire::channel::Sender<()>, String> {
    let (stop, stopped) = pipewire::channel::channel();
    std::thread::Builder::new()
        .name("reprise-record-sound".into())
        .spawn(move || {
            pipewire::init();
            let samples = Box::new(move |pcm: &[u8]| {
                let _ = tx.send(Event::Sound(pcm.to_vec(), Instant::now()));
            });
            let captured = reprisevideo::monitor::capture(
                None,
                APP,
                "Screen recording",
                sound::RATE,
                samples,
                stopped,
            );
            if let Err(e) = captured {
                log::warn!("recording what plays: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(stop)
}

/// The movie being written: its encoders open and its header written on the first frame.
struct File<'a> {
    path: &'a Path,
    device: &'a Path,
    backends: Backends,
    /// Asks the screen's stream for other buffers.
    ask: Box<dyn FnMut(&Ask)>,
    #[cfg(any(test, feature = "fallback-test"))]
    forced: Option<Forced>,
    area: Option<Area>,
    /// The screen's logical size, which the area is in.
    logical: (i32, i32),
    coding: Coding,
    pacer: Pacer,
    /// The newest frame the pacer holds for its next slot.
    held: Option<Frame>,
    sound: Option<Sound>,
    movie: Option<Movie>,
    output: Option<ffmpeg::format::context::Output>,
    /// When the first frame came: the movie's time 0.
    start: Option<Instant>,
}

/// A part of a frame: its size and top-left, in frame pixels.
type Cut = ((u32, u32), (u32, u32));

const VIDEO: usize = 0;
const AUDIO: usize = 1;
/// The MP4 sample entry of H.265 with its parameter sets in the header.
const HVC1: u32 = u32::from_le_bytes(*b"hvc1");

impl<'a> File<'a> {
    fn new(
        path: &'a Path,
        device: &'a Path,
        backends: Vec<(Backend, Vec<u64>)>,
        area: Option<Area>,
        logical: (i32, i32),
        making: Making,
    ) -> File<'a> {
        File {
            path,
            device,
            backends: Backends::new(making.coding.codec, backends),
            ask: Box::new(|_| {}),
            #[cfg(any(test, feature = "fallback-test"))]
            forced: None,
            area,
            logical,
            coding: making.coding,
            pacer: making.pacer,
            held: None,
            sound: making.sound,
            movie: None,
            output: None,
            start: None,
        }
    }

    fn started(&self) -> bool {
        self.start.is_some()
    }

    fn started_writing(&self) -> bool {
        self.output.is_some()
    }

    /// Code what comes until a stop, the stream's end or an error, waking for the pacer's slots
    /// between. A backlog of frames is skipped to the newest, so a slow encoder drops pictures
    /// rather than falling behind.
    fn run(&mut self, events: &mpsc::Receiver<Event>) -> Result<(), String> {
        let mut held = None;
        loop {
            let event = match (held.take(), self.wake()) {
                (Some(e), _) => e,
                (None, None) => events.recv().map_err(|_| "every source ended")?,
                (None, Some(due)) => {
                    let now = Instant::now();
                    match (due > now).then(|| events.recv_timeout(due - now)) {
                        Some(Ok(e)) => e,
                        Some(Err(mpsc::RecvTimeoutError::Disconnected)) => {
                            return Err("every source ended".into());
                        }
                        None | Some(Err(mpsc::RecvTimeoutError::Timeout)) => {
                            self.tick(Instant::now())?;
                            continue;
                        }
                    }
                }
            };
            match event {
                Event::Frame(mut frame, mut at) => {
                    while let Ok(next) = events.try_recv() {
                        match next {
                            Event::Frame(f, t) => (frame, at) = (f, t),
                            other => {
                                held = Some(other);
                                break;
                            }
                        }
                    }
                    self.frame(frame, at)?;
                }
                Event::Sound(pcm, at) => self.sound(&pcm, at)?,
                Event::Stop => return Ok(()),
                Event::Ended => {
                    log::info!("the screen's stream ended");
                    return Ok(());
                }
            }
        }
    }

    /// The instant the pacer next wants [`File::tick`] called.
    fn wake(&self) -> Option<Instant> {
        let ms = self.pacer.wake(self.held.is_some())?;
        Some(self.start? + Duration::from_millis(ms.max(0) as u64))
    }

    fn frame(&mut self, frame: Frame, at: Instant) -> Result<(), String> {
        let (size, _) = self.cut(frame.size)?;
        if self.movie.as_ref().is_some_and(|m| !m.takes(&frame, size)) {
            return Err("The screen changed size or format.".into());
        }
        let start = *self.start.get_or_insert(at);
        if self.movie.is_none() && !self.reopen(&frame, at)? {
            return Ok(());
        }
        match self
            .pacer
            .frame(at.duration_since(start).as_millis() as i64)
        {
            Pace::Hold => self.held = Some(frame),
            Pace::Code(pts) => {
                self.held = None;
                self.code(&frame, pts, at)?;
            }
        }
        Ok(())
    }

    /// Code the held frame or repeat the last picture when the pacer's slot is due at `at`.
    fn tick(&mut self, at: Instant) -> Result<(), String> {
        let Some(start) = self.start else {
            return Ok(());
        };
        let now = at.saturating_duration_since(start).as_millis() as i64;
        let Some(pts) = self.pacer.tick(now, self.held.is_some()) else {
            return Ok(());
        };
        if let Some(frame) = self.held.take() {
            return self.code(&frame, pts, at);
        }
        let Some(movie) = self.movie.as_mut() else {
            return Ok(());
        };
        let mut packets = Vec::new();
        movie.repeat(pts, &mut packets).map_err(|e| e.to_string())?;
        self.write(packets, VIDEO)
    }

    /// Open the next backend that reads `frame`. False drops it, the movie starting over: nothing
    /// is written yet, so the first frame coded is the movie's time 0.
    fn reopen(&mut self, frame: &Frame, at: Instant) -> Result<bool, String> {
        let (size, origin) = self.cut(frame.size)?;
        match self.open_movie(frame, size, origin, at)? {
            Some(movie) => self.movie = Some(movie),
            None => {
                self.start = None;
                self.pacer.restart();
            }
        }
        Ok(self.movie.is_some())
    }

    fn code(&mut self, frame: &Frame, pts: i64, at: Instant) -> Result<(), String> {
        let mut packets = Vec::new();
        loop {
            if self.movie.is_none() && !self.reopen(frame, at)? {
                return Ok(());
            }
            let movie = self.movie.as_mut().expect("opened above");
            match movie.code(frame, pts, &mut packets) {
                Ok(()) => break,
                Err(e) if self.output.is_none() => {
                    log::warn!("{}: {e}", movie.backend().label(movie.codec()));
                    self.movie = None;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        self.write_header()?;
        self.write(packets, VIDEO)
    }

    /// The next backend in turn that reads `frame` and opens for it. None drops `frame` while the
    /// stream is asked for buffers the next backend reads.
    fn open_movie(
        &mut self,
        frame: &Frame,
        size: (u32, u32),
        origin: (u32, u32),
        at: Instant,
    ) -> Result<Option<Movie>, String> {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        loop {
            #[cfg(any(test, feature = "fallback-test"))]
            let forced = self.forced.as_ref();
            #[cfg(not(any(test, feature = "fallback-test")))]
            let forced = None;
            let reads = |b, m: &[u64]| readable(forced, b, m, frame, at);
            let backend = match self.backends.next(at, reads) {
                Step::Open(backend) => backend,
                Step::Wait(ask) => {
                    if let Some(ask) = ask {
                        if !ask.again {
                            log::warn!(
                                "{} cannot read the screen's buffers ({}); asking the stream \
                                 for buffers it maps",
                                ask.backend.label(self.coding.codec),
                                frame.buffer.describe()
                            );
                        }
                        #[cfg(any(test, feature = "fallback-test"))]
                        if let Some(f) = &mut self.forced {
                            f.asked.get_or_insert_with(Instant::now);
                        }
                        (self.ask)(&ask);
                    }
                    return Ok(None);
                }
                Step::Exhausted => {
                    let label = self.coding.codec.label();
                    return Err(format!("No {label} encoder could code the screen."));
                }
            };
            match self.open(backend, fourcc, frame.size, size, origin) {
                Ok(m) => {
                    let cut = match (self.area, m.cut_on_gpu()) {
                        (None, _) => "",
                        (Some(_), true) => ", the area cut out on the GPU",
                        (Some(_), false) => ", the area cut out on the CPU",
                    };
                    log::info!(
                        "coding {size:?} from {origin:?} with {}{cut}",
                        backend.label(self.coding.codec)
                    );
                    return Ok(Some(m));
                }
                Err(e) => log::warn!("{}: {e}", backend.label(self.coding.codec)),
            }
        }
    }

    fn open(
        &self,
        backend: Backend,
        fourcc: u32,
        frame: (u32, u32),
        size: (u32, u32),
        origin: (u32, u32),
    ) -> Result<Movie, reprisevideo::encode::EncodeError> {
        #[cfg(any(test, feature = "fallback-test"))]
        if self.forced.is_some() && backend.gpu().is_some() {
            return Err(reprisevideo::encode::EncodeError(
                "failing to open on purpose".into(),
            ));
        }
        let coding = &self.coding;
        Movie::open(backend, coding, self.device, fourcc, frame, size, origin)
    }

    /// The part of a `frame_size` frame recorded: its size and top-left in frame pixels.
    fn cut(&self, frame_size: (u32, u32)) -> Result<Cut, String> {
        match self.area {
            None => Ok((frame_size, (0, 0))),
            Some(a) => area_pixels(a, self.logical, frame_size)
                .ok_or_else(|| "The area is outside the screen.".into()),
        }
    }

    fn sound(&mut self, pcm: &[u8], at: Instant) -> Result<(), String> {
        let (Some(start), true) = (self.start, self.output.is_some()) else {
            return Ok(());
        };
        let Some(sound) = self.sound.as_mut() else {
            return Ok(());
        };
        let played = (pcm.len() / 4) as u64 * 1000 / u64::from(sound::RATE);
        let at_ms = at.duration_since(start).as_millis() as i64 - played as i64;
        let mut packets = Vec::new();
        sound.code(pcm, at_ms, &mut packets)?;
        self.write(packets, AUDIO)
    }

    fn write_header(&mut self) -> Result<(), String> {
        if self.output.is_some() {
            return Ok(());
        }
        let movie = self.movie.as_ref().expect("opened before the header");
        let failed = |e: ffmpeg::Error| format!("Cannot write the recording: {e}");
        let mut output = ffmpeg::format::output_as(self.path, "mp4").map_err(failed)?;
        let mut video = output.add_stream(movie.encoder().codec()).map_err(failed)?;
        let mut parameters = ffmpeg::codec::Parameters::from(movie.encoder());
        if movie.codec() == Codec::H265 {
            // SAFETY: a field of parameters this function owns. MP4's default `hev1` does not
            // play in QuickTime, Safari or iOS; the parameter sets are in the header either way.
            unsafe { (*parameters.as_mut_ptr()).codec_tag = HVC1 };
        }
        video.set_parameters(parameters);
        video.set_time_base(movie::TIME_BASE);
        if let Some(s) = &self.sound {
            let mut audio = output.add_stream(s.encoder().codec()).map_err(failed)?;
            audio.set_parameters(ffmpeg::codec::Parameters::from(s.encoder()));
            audio.set_time_base((1, sound::RATE as i32));
        }
        let mut options = ffmpeg::Dictionary::new();
        options.set("movflags", "+faststart");
        output.write_header_with(options).map_err(failed)?;
        self.output = Some(output);
        Ok(())
    }

    fn write(&mut self, packets: Vec<Packet>, stream: usize) -> Result<(), String> {
        let Some(output) = self.output.as_mut().filter(|_| !packets.is_empty()) else {
            return Ok(());
        };
        let from = match stream {
            VIDEO => movie::TIME_BASE.into(),
            _ => ffmpeg::Rational(1, sound::RATE as i32),
        };
        let to = output
            .stream(stream)
            .map(|s| s.time_base())
            .ok_or("no such stream")?;
        for mut p in packets {
            p.set_stream(stream);
            p.rescale_ts(from, to);
            p.write_interleaved(output)
                .map_err(|e| format!("Cannot write the recording: {e}"))?;
        }
        Ok(())
    }

    /// Code what is due and a frame still held at its slot, hold the last picture to the
    /// pacer's end, drain the encoders and close the file. The file is closed even when draining
    /// fails, so what was written plays.
    fn finish(&mut self) -> Result<(), String> {
        let Some(start) = self.start else {
            return Ok(());
        };
        let mut drained = self.tick(Instant::now());
        if let Some(frame) = self.held.take()
            && let Some(pts) = self.pacer.flush()
        {
            drained = drained.and(self.code(&frame, pts, Instant::now()));
        }
        let end = self.pacer.end(start.elapsed().as_millis() as i64);
        let mut video = Vec::new();
        drained = drained.and(match self.movie.as_mut() {
            Some(movie) => movie.finish(end, &mut video).map_err(|e| e.to_string()),
            None => Ok(()),
        });
        drained = drained.and(self.write(video, VIDEO));
        let mut audio = Vec::new();
        if let Some(sound) = self.sound.as_mut() {
            drained = drained.and(sound.finish(&mut audio));
        }
        drained = drained.and(self.write(audio, AUDIO));
        if let Some(output) = self.output.as_mut() {
            (output.write_trailer()).map_err(|e| format!("Cannot finish the recording: {e}"))?;
        }
        drained
    }
}

/// `area` of a screen `logical` in size as the pixels of its `frame` size: its size made even
/// and top-left, inside the frame. None when nothing of it is on the screen.
fn area_pixels(area: Area, logical: (i32, i32), frame: (u32, u32)) -> Option<Cut> {
    let scale = match logical.0 {
        w if w > 0 => f64::from(frame.0) / f64::from(w),
        _ => 1.0,
    };
    let px = |v: i32| (f64::from(v) * scale).round() as i64;
    let x0 = px(area.x).clamp(0, i64::from(frame.0));
    let y0 = px(area.y).clamp(0, i64::from(frame.1));
    let x1 = px(area.x + area.width).clamp(0, i64::from(frame.0));
    let y1 = px(area.y + area.height).clamp(0, i64::from(frame.1));
    let (w, h) = ((x1 - x0) as u32 & !1, (y1 - y0) as u32 & !1);
    (w > 0 && h > 0).then_some(((w, h), (x0 as u32, y0 as u32)))
}

/// Whether `backend`, mapping `modifiers`, reads `frame`, which came at `at`.
fn readable(
    forced: Option<&Forced>,
    backend: Backend,
    modifiers: &[u64],
    frame: &Frame,
    at: Instant,
) -> bool {
    #[cfg(any(test, feature = "fallback-test"))]
    if let Some(f) = forced
        && f.buffers
        && f.asked.is_none_or(|asked| at <= asked)
    {
        return backend.gpu().is_some();
    }
    #[cfg(not(any(test, feature = "fallback-test")))]
    let _ = (forced, at);
    reprisevideo::encode::reads_offered(Some(backend), modifiers, frame)
}

/// A hardware encoder failing, for a check where none works: each fails to open, and with
/// `buffers` the frames that came before the stream was asked for others count as buffers only
/// a GPU reads.
#[cfg(any(test, feature = "fallback-test"))]
#[derive(Default)]
struct Forced {
    buffers: bool,
    asked: Option<Instant>,
}

#[cfg(not(any(test, feature = "fallback-test")))]
enum Forced {}

/// `REPRISE_RECORD_FAIL_HARDWARE=1` in a build with the `fallback-test` feature: every hardware
/// encoder fails to open, a Vulkan stand-in first when the probe found none.
#[cfg(feature = "fallback-test")]
mod forced {
    use super::{Backend, DRM_FORMAT_MOD_LINEAR, Forced};

    /// A tiled modifier the stand-in offers besides LINEAR.
    const Y_TILED: u64 = 0x0100_0000_0000_0002;

    pub fn order(
        mut order: Vec<(Backend, Vec<u64>)>,
    ) -> (Vec<(Backend, Vec<u64>)>, Option<Forced>) {
        if std::env::var_os("REPRISE_RECORD_FAIL_HARDWARE").is_none_or(|v| v != "1") {
            return (order, None);
        }
        log::warn!("every hardware encoder fails to open (REPRISE_RECORD_FAIL_HARDWARE)");
        if order.iter().all(|(b, _)| b.gpu().is_none()) {
            order.insert(0, (Backend::Vulkan, vec![DRM_FORMAT_MOD_LINEAR, Y_TILED]));
        }
        let forced = Forced {
            buffers: true,
            asked: None,
        };
        (order, Some(forced))
    }
}

/// Block SIGINT and SIGTERM in this thread and every thread it starts from now on.
fn block_stop_signals() -> libc::sigset_t {
    // SAFETY: a set built and installed on this thread; the out-params are valid.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

/// Waits for the first of `set` on a thread of its own and sends a stop.
fn signals(set: libc::sigset_t, tx: mpsc::Sender<Event>) {
    let spawned = std::thread::Builder::new()
        .name("reprise-record-signals".into())
        .spawn(move || {
            let mut signal = 0;
            // SAFETY: `set` is blocked in every thread; `signal` is a valid out-param.
            if unsafe { libc::sigwait(&set, &mut signal) } == 0 {
                log::info!("stopping on signal {signal}");
            }
            let _ = tx.send(Event::Stop);
        });
    if let Err(e) = spawned {
        log::warn!("no signal thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H.264 at the default quality, unpaced, without sound.
    fn h264() -> Making {
        making(&Recording::default(), Some(60), None)
    }

    fn software() -> Vec<(Backend, Vec<u64>)> {
        vec![(Backend::Software, vec![DRM_FORMAT_MOD_LINEAR])]
    }

    fn grey(v: u8) -> Frame {
        Frame::detached(vec![v; 64 * 64 * 4], (64, 64), Vec::new())
    }

    /// Each video packet's pts in ms.
    fn video_ms(path: &Path) -> Vec<i64> {
        let mut input = ffmpeg::format::input(path).unwrap();
        let tb = input.stream(VIDEO).unwrap().time_base();
        (input.packets())
            .map(|p| p.unwrap())
            .filter(|(s, _)| s.index() == VIDEO)
            .map(|(_, p)| p.pts().unwrap() * 1000 * i64::from(tb.0) / i64::from(tb.1))
            .collect()
    }

    #[test]
    fn no_encoder_for_the_codec_fails_naming_it() {
        ffmpeg::init().unwrap();
        assert_eq!(
            first_encoder(&[], Codec::H265).unwrap_err(),
            "No H.265 encoder works here."
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp4");
        let node = PathBuf::from(probe::RENDER_NODE);
        let av1 = Recording {
            codec: Codec::Av1,
            ..Recording::default()
        };
        let mut file = File::new(
            &path,
            &node,
            Vec::new(),
            None,
            (64, 64),
            making(&av1, None, None),
        );
        assert_eq!(
            file.frame(grey(0x80), Instant::now()).unwrap_err(),
            "No AV1 encoder could code the screen."
        );
        assert!(!file.started_writing());
    }

    #[test]
    fn the_settings_pick_the_rate_hint_the_cap_and_the_fixed_rate() {
        let at = |rate, frame_rate| Recording {
            rate,
            frame_rate,
            ..Recording::default()
        };
        let aac = Sound::open(AudioCodec::Aac, 160, true, 5).unwrap();
        let m = making(&at(RateMode::Quality, 0), Some(144), Some(aac));
        assert_eq!((m.coding.rate_hint, m.pacer.wake(true)), (144, None));
        assert_eq!(m.coding.rate, RateControl::Quality(55));
        assert!(m.sound.is_some());
        let m = making(&at(RateMode::Average, 30), None, None);
        assert_eq!(
            (m.coding.rate, m.coding.rate_hint),
            (RateControl::Average(8000), 30)
        );
        assert_eq!(m.pacer, Pacer::new(Some(30), false));
        assert!(m.sound.is_none());
        let m = making(&at(RateMode::Constant, 0), None, None);
        assert_eq!(
            (m.coding.rate, m.coding.rate_hint),
            (RateControl::Constant(8000), 60)
        );
        assert_eq!(
            m.pacer,
            Pacer::new(Some(60), true),
            "at the screen's rate when none is set"
        );
        let m = making(&at(RateMode::Constant, 0), Some(144), None);
        assert_eq!(m.coding.rate_hint, 60, "a 144 Hz screen");
        assert_eq!(m.pacer, Pacer::new(Some(60), true));
        let m = making(&at(RateMode::Constant, 0), Some(50), None);
        assert_eq!(m.coding.rate_hint, 50);
        let m = making(&at(RateMode::Constant, 144), Some(144), None);
        assert_eq!(m.pacer, Pacer::new(Some(144), true), "a cap set");
    }

    #[test]
    fn no_encoder_for_the_audio_codec_fails_naming_it() {
        let opus = Recording {
            audio_codec: AudioCodec::Opus,
            audio_bitrate: 96,
            ..Recording::default()
        };
        let mut asked = None;
        let failed = open_sound(&opus, |codec, kbit, _, _| {
            asked = Some((codec, kbit));
            Err("Opus: FFmpeg has no libopus".into())
        });
        assert_eq!(failed.err().unwrap(), "No Opus encoder works here.");
        assert_eq!(asked, Some((AudioCodec::Opus, 96)));
        assert!(open_sound(&Recording::default(), Sound::open).is_ok());
    }

    #[test]
    fn a_still_screen_in_constant_mode_runs_at_the_fixed_rate() {
        ffmpeg::init().unwrap();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp4");
        let node = PathBuf::from(probe::RENDER_NODE);
        let constant = Recording {
            rate: RateMode::Constant,
            bitrate: 2000,
            frame_rate: 10,
            ..Recording::default()
        };
        let making = making(&constant, None, None);
        assert_eq!(making.coding.rate, RateControl::Constant(2000));
        let order = software();
        let mut file = File::new(&path, &node, order, None, (64, 64), making);
        let t0 = Instant::now();
        file.frame(grey(0x80), t0).unwrap();
        for i in 1..=10 {
            file.tick(t0 + Duration::from_millis(100 * i)).unwrap();
        }
        file.finish().unwrap();
        let ms = video_ms(&path);
        assert_eq!(ms, (0..=10).map(|i| i * 100).collect::<Vec<_>>());
    }

    #[test]
    fn a_frame_rate_cap_codes_the_last_change_of_a_burst_at_the_next_slot() {
        ffmpeg::init().unwrap();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp4");
        let node = PathBuf::from(probe::RENDER_NODE);
        let capped = Recording {
            frame_rate: 10,
            ..Recording::default()
        };
        let mut file = File::new(
            &path,
            &node,
            software(),
            None,
            (64, 64),
            making(&capped, None, None),
        );
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        for (ms, v) in [(0, 0x20), (30, 0x40), (60, 0x60)] {
            file.frame(grey(v), at(ms)).unwrap();
        }
        assert_eq!(file.wake(), Some(at(100)));
        file.tick(at(100)).unwrap();
        assert_eq!(file.wake(), None, "a still screen adds nothing");
        file.frame(grey(0x80), at(400)).unwrap();
        file.frame(grey(0xa0), at(430)).unwrap();
        file.finish().unwrap();
        assert_eq!(
            video_ms(&path),
            [0, 100, 400, 500, 600],
            "the change held at the stop takes the next slot, and the end is a step later"
        );
    }

    #[test]
    fn an_h265_recording_is_tagged_hvc1() {
        ffmpeg::init().unwrap();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp4");
        let node = PathBuf::from(probe::RENDER_NODE);
        let h265 = Recording {
            codec: Codec::H265,
            ..Recording::default()
        };
        let mut file = File::new(
            &path,
            &node,
            software(),
            None,
            (64, 64),
            making(&h265, None, None),
        );
        let t0 = Instant::now();
        file.frame(grey(0x20), t0).unwrap();
        file.frame(grey(0xe0), t0 + Duration::from_millis(40))
            .unwrap();
        file.finish().unwrap();
        let input = ffmpeg::format::input(&path).unwrap();
        let video = input.streams().best(ffmpeg::media::Type::Video).unwrap();
        // SAFETY: plain fields of the stream's parameters.
        let (id, tag) = unsafe {
            let p = video.parameters().as_ptr();
            ((*p).codec_id, (*p).codec_tag)
        };
        assert_eq!(id, ffmpeg::ffi::AVCodecID::HEVC);
        assert_eq!(tag, u32::from_le_bytes(*b"hvc1"));
    }

    #[test]
    fn the_probe_lists_each_choice_once_a_codec_in_the_backends_order() {
        let working = [
            (Codec::H265, Backend::Software),
            (Codec::H264, Backend::Vaapi { low_power: false }),
            (Codec::H264, Backend::Vaapi { low_power: true }),
            (Codec::H264, Backend::Software),
        ];
        let p = probed(&working, |a| a != AudioCodec::Opus);
        assert_eq!(
            p.to_text(),
            "video h264 vaapi\nvideo h264 software\nvideo h265 software\naudio aac\naudio flac\n"
        );
    }

    #[test]
    fn the_handed_file_plays_for_as_long_as_it_recorded_and_a_name_swapped_in_is_not_followed() {
        use std::os::fd::AsRawFd;
        ffmpeg::init().unwrap();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let dir = tempfile::tempdir().unwrap();
        let (name, kept, victim) = (
            dir.path().join("a.mp4"),
            dir.path().join("b.mp4"),
            dir.path().join("victim"),
        );
        std::fs::write(&victim, "keep").unwrap();
        let handed = std::fs::File::create_new(&name).unwrap();
        std::fs::rename(&name, &kept).unwrap();
        std::os::unix::fs::symlink(&victim, &name).unwrap();
        let path = movie_url(handed.as_raw_fd()).unwrap();
        let node = PathBuf::from(probe::RENDER_NODE);
        let area = Some(Area {
            x: 8,
            y: 4,
            width: 32,
            height: 24,
        });
        let x264 = vec![(Backend::Software, vec![DRM_FORMAT_MOD_LINEAR])];
        let mut file = File::new(&path, &node, x264, area, (64, 48), h264());
        let t0 = Instant::now();
        for (i, grey) in [0x20u8, 0x80, 0xe0].into_iter().enumerate() {
            let frame = Frame::detached(vec![grey; 64 * 48 * 4], (64, 48), Vec::new());
            file.frame(frame, t0 + Duration::from_millis(400 * i as u64))
                .unwrap();
        }
        file.finish().unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        let input = ffmpeg::format::input(&kept).unwrap();
        let video = input.streams().best(ffmpeg::media::Type::Video).unwrap();
        let params = video.parameters();
        // SAFETY: plain fields of the stream's parameters.
        let size = unsafe { ((*params.as_ptr()).width, (*params.as_ptr()).height) };
        assert_eq!(size, (32, 24), "the area");
        assert_eq!(input.streams().count(), 1, "no sound asked for");
        let seconds = input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
        assert!((0.8..1.5).contains(&seconds), "{seconds} s");
    }

    #[test]
    fn a_hardware_encoder_failing_to_open_drops_only_the_tiled_frames_in_flight() {
        use reprisevideo::stream::{Plane, XRGB8888};
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::Arc;
        const Y_TILED: u64 = 0x0100_0000_0000_0002;
        ffmpeg::init().unwrap();
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mp4");
        let node = PathBuf::from(probe::RENDER_NODE);
        let order = vec![
            (
                Backend::Vaapi { low_power: false },
                vec![DRM_FORMAT_MOD_LINEAR, Y_TILED],
            ),
            (Backend::Software, vec![DRM_FORMAT_MOD_LINEAR]),
        ];
        let area = Some(Area {
            x: 8,
            y: 4,
            width: 32,
            height: 24,
        });
        let mut file = File::new(&path, &node, order, area, (64, 48), h264());
        let asks = Rc::new(RefCell::new(Vec::new()));
        file.ask = Box::new({
            let asks = asks.clone();
            move |a: &Ask| {
                asks.borrow_mut()
                    .push((a.backend, a.modifiers.clone(), a.again))
            }
        });
        file.forced = Some(Forced::default());
        let tiled = || {
            let fd = std::fs::File::open("/dev/null").unwrap();
            let buffer = FrameBuffer::Dmabuf {
                objects: vec![Arc::new(fd.into())],
                fourcc: XRGB8888,
                modifier: Y_TILED,
                planes: vec![Plane {
                    object: 0,
                    offset: 0,
                    stride: 256,
                }],
            };
            Frame::unleased(buffer, (64, 48))
        };
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        file.frame(tiled(), at(0)).unwrap();
        file.frame(tiled(), at(16)).unwrap();
        assert!(!file.started(), "both tiled frames dropped");
        for (i, grey) in [0x20u8, 0x80, 0xe0].into_iter().enumerate() {
            let frame = Frame::detached(vec![grey; 64 * 48 * 4], (64, 48), Vec::new());
            file.frame(frame, at(100 + 400 * i as u64)).unwrap();
        }
        file.finish().unwrap();
        assert_eq!(
            *asks.borrow(),
            [(Backend::Software, vec![DRM_FORMAT_MOD_LINEAR], false)],
            "the stream is asked once for x264's LINEAR buffers"
        );
        let mut input = ffmpeg::format::input(&path).unwrap();
        assert_eq!(input.streams().count(), 1);
        let tb = input.stream(VIDEO).unwrap().time_base();
        let packets: Vec<_> = (input.packets()).map(|p| p.unwrap().1).collect();
        let ms: Vec<_> = (packets.iter())
            .map(|p| p.pts().unwrap() * 1000 * i64::from(tb.0) / i64::from(tb.1))
            .collect();
        assert_eq!(
            ms,
            [0, 400, 800, 801],
            "one stream from the first readable frame at time 0, held to the end"
        );
        assert!(packets[0].is_key());
    }

    #[test]
    fn an_area_goes_to_the_hardware_encoders_from_the_probes_test_side_up() {
        let area = |width, height| {
            Some(Area {
                x: 3,
                y: 5,
                width,
                height,
            })
        };
        assert!(hardware_takes(None), "the whole screen");
        assert!(hardware_takes(area(256, 256)));
        assert!(hardware_takes(area(1001, 701)), "odd sides");
        assert!(!hardware_takes(area(255, 900)), "too narrow: x264");
        assert!(!hardware_takes(area(900, 255)), "too short: x264");
    }

    #[test]
    fn a_descriptor_that_is_not_a_regular_file_is_refused() {
        let (r, _w) = std::io::pipe().unwrap();
        use std::os::fd::AsRawFd;
        assert!(movie_url(r.as_raw_fd()).is_err());
        assert!(movie_url(9999).is_err());
    }

    #[test]
    fn an_area_is_scaled_to_the_frame_and_kept_inside_it() {
        let area = |x, y, width, height| Area {
            x,
            y,
            width,
            height,
        };
        assert_eq!(
            area_pixels(area(100, 50, 301, 200), (1920, 1080), (1920, 1080)),
            Some(((300, 200), (100, 50))),
            "odd sides lose a pixel"
        );
        assert_eq!(
            area_pixels(area(100, 50, 300, 200), (1280, 720), (2560, 1440)),
            Some(((600, 400), (200, 100))),
            "at scale 2"
        );
        assert_eq!(
            area_pixels(area(-20, 1000, 100, 200), (1920, 1080), (1920, 1080)),
            Some(((80, 80), (0, 1000))),
            "cut at the screen's edges"
        );
        assert_eq!(
            area_pixels(area(101, 51, 301, 201), (1920, 1080), (1920, 1080)),
            Some(((300, 200), (101, 51))),
            "an odd origin is kept, so the picture does not move"
        );
        assert_eq!(
            area_pixels(area(2000, 0, 100, 100), (1920, 1080), (1920, 1080)),
            None
        );
    }
}
