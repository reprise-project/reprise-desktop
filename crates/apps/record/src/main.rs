//! reprise-record: records a screen, or an area of it, and with `--sound` what plays, into an MP4
//! file the compositor made and handed it open, until SIGINT or SIGTERM or until the screen goes.
//! On failure its last line on stderr is the reason.

mod args;
mod movie;
mod portal;
mod sound;

use args::{Area, Args};
use ffmpeg::Packet;
use movie::Movie;
use repriseconf::remote::EncoderChoice;
use reprisevideo::encode::choice;
use reprisevideo::encode::probe::{self, Backend};
use reprisevideo::stream::{self, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer};
use sound::Sound;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

/// What PipeWire's streams name the program.
const APP: &str = "Screen Recorder";

fn main() -> ExitCode {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    reprisebus::journal::log_panics();
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(a) => a,
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
    let (tx, events) = mpsc::channel();
    signals(stop, tx.clone());
    let picked = std::thread::spawn({
        let area = args.area.is_some();
        move || encoders(area)
    });
    let cast = portal::Cast::start().map_err(|e| format!("The screen could not be shared: {e}"))?;
    let screen = (cast.screen(&args.output).cloned())
        .ok_or_else(|| format!("The screen {} is not shared.", args.output))?;
    let (device, order) = picked.join().map_err(|_| "Finding an encoder failed.")?;
    let (first, modifiers) = order.first().ok_or("No H.264 encoder works here.")?;
    log::info!("recording {} with {}", args.output, first.label());
    let fd = cast.pipewire()?;
    let (_stream, mut frames, _sizes) = stream::spawn(fd, screen.node, modifiers, APP, || {})?;
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
    let backends = order.iter().map(|(b, _)| *b).collect();
    let mut file = File::new(&into, &device, backends, args.area, screen.size, args.sound);
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

/// The render node, and the backends to try in Remote Desktop's automatic order with the stream
/// modifiers each maps. An area is cut out on the CPU, so it codes on x264 from shared memory or
/// linear buffers.
fn encoders(area: bool) -> (PathBuf, Vec<(Backend, Vec<u64>)>) {
    if area {
        let node = PathBuf::from(probe::RENDER_NODE);
        return (node, vec![(Backend::X264, vec![DRM_FORMAT_MOD_LINEAR])]);
    }
    let conn = wayland_client::Connection::connect_to_env().ok();
    let (device, working) = probe::probe(conn.as_ref());
    let backends: Vec<_> = working.iter().map(|(b, _)| *b).collect();
    let order = choice::order(EncoderChoice::Auto, &backends);
    let with_modifiers = (order.into_iter())
        .filter_map(|b| working.iter().find(|(w, _)| *w == b).cloned())
        .collect();
    (device, with_modifiers)
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
    /// The backends not yet tried, in order.
    backends: Vec<Backend>,
    area: Option<Area>,
    /// The screen's logical size, which the area is in.
    logical: (i32, i32),
    sound: Option<Sound>,
    wants_sound: bool,
    movie: Option<Movie>,
    output: Option<ffmpeg::format::context::Output>,
    /// When the first frame came: the movie's time 0.
    start: Option<Instant>,
    /// The time of the last picture coded, in ms.
    last_pts: i64,
}

/// A part of a frame: its size and top-left, in frame pixels.
type Cut = ((u32, u32), (u32, u32));

const VIDEO: usize = 0;
const AUDIO: usize = 1;

impl<'a> File<'a> {
    fn new(
        path: &'a Path,
        device: &'a Path,
        backends: Vec<Backend>,
        area: Option<Area>,
        logical: (i32, i32),
        wants_sound: bool,
    ) -> File<'a> {
        File {
            path,
            device,
            backends,
            area,
            logical,
            sound: None,
            wants_sound,
            movie: None,
            output: None,
            start: None,
            last_pts: -1,
        }
    }

    fn started(&self) -> bool {
        self.start.is_some()
    }

    fn started_writing(&self) -> bool {
        self.output.is_some()
    }

    /// Code what comes until a stop, the stream's end or an error. A backlog of frames is skipped
    /// to the newest, so a slow encoder drops pictures rather than falling behind.
    fn run(&mut self, events: &mpsc::Receiver<Event>) -> Result<(), String> {
        let mut held = None;
        loop {
            let event = match held.take() {
                Some(e) => e,
                None => events.recv().map_err(|_| "every source ended")?,
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
                    self.frame(&frame, at)?;
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

    fn frame(&mut self, frame: &Frame, at: Instant) -> Result<(), String> {
        let start = *self.start.get_or_insert(at);
        let (size, origin) = self.cut(frame.size)?;
        if self.movie.as_ref().is_some_and(|m| !m.takes(frame, size)) {
            return Err("The screen changed size or format.".into());
        }
        // Each frame is a whole picture, so the same ms twice keeps the newer.
        let pts = (at.duration_since(start).as_millis() as i64).max(self.last_pts + 1);
        let mut packets = Vec::new();
        loop {
            if self.movie.is_none() {
                self.movie = Some(self.open_movie(frame, size, origin)?);
            }
            let movie = self.movie.as_mut().expect("opened above");
            match movie.code(frame, pts, &mut packets) {
                Ok(()) => break,
                Err(e) if self.output.is_none() => {
                    log::warn!("{}: {e}", movie.backend().label());
                    self.movie = None;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        self.last_pts = pts;
        self.write_header()?;
        self.write(packets, VIDEO)
    }

    /// The next backend in turn that opens for `frame`.
    fn open_movie(
        &mut self,
        frame: &Frame,
        size: (u32, u32),
        origin: (u32, u32),
    ) -> Result<Movie, String> {
        let fourcc = match &frame.buffer {
            FrameBuffer::Dmabuf { fourcc, .. } | FrameBuffer::Shm { fourcc, .. } => *fourcc,
        };
        while !self.backends.is_empty() {
            let backend = self.backends.remove(0);
            if !reprisevideo::encode::reads(Some(backend), frame) {
                log::info!("{} cannot read the screen's buffers", backend.label());
                continue;
            }
            match Movie::open(backend, self.device, fourcc, size, origin) {
                Ok(m) => return Ok(m),
                Err(e) => log::warn!("{}: {e}", backend.label()),
            }
        }
        Err("No H.264 encoder could code the screen.".into())
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
        video.set_parameters(ffmpeg::codec::Parameters::from(movie.encoder()));
        video.set_time_base(movie::TIME_BASE);
        if self.wants_sound {
            match Sound::open() {
                Ok(s) => {
                    let mut audio = output.add_stream(s.encoder().codec()).map_err(failed)?;
                    audio.set_parameters(ffmpeg::codec::Parameters::from(s.encoder()));
                    audio.set_time_base((1, sound::RATE as i32));
                    self.sound = Some(s);
                }
                Err(e) => log::warn!("recording without sound: {e}"),
            }
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

    /// Hold the last picture to now, drain the encoders and close the file. The file is closed
    /// even when draining fails, so what was written plays.
    fn finish(&mut self) -> Result<(), String> {
        let Some(start) = self.start else {
            return Ok(());
        };
        let end = (start.elapsed().as_millis() as i64).max(self.last_pts + 1);
        let mut video = Vec::new();
        let mut drained = match self.movie.as_mut() {
            Some(movie) => movie.finish(end, &mut video).map_err(|e| e.to_string()),
            None => Ok(()),
        };
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
    use std::time::Duration;

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
        let mut file = File::new(&path, &node, vec![Backend::X264], area, (64, 48), false);
        let t0 = Instant::now();
        for (i, grey) in [0x20u8, 0x80, 0xe0].into_iter().enumerate() {
            let frame = Frame::detached(vec![grey; 64 * 48 * 4], (64, 48), Vec::new());
            file.frame(&frame, t0 + Duration::from_millis(400 * i as u64))
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
            area_pixels(area(2000, 0, 100, 100), (1920, 1080), (1920, 1080)),
            None
        );
    }
}
