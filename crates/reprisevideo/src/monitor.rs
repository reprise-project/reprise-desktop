//! What plays, captured from a sink's monitor, or the microphone, on a PipeWire connection of its
//! own.

use pipewire::properties::properties;
use pipewire::spa;
use pipewire::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pipewire::spa::pod::Pod;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::sys as spa_sys;
use pipewire::stream::{StreamFlags, StreamRc, StreamState};
use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Takes each chunk of captured samples, and the graph cycle that captured it.
pub type Samples = Box<dyn FnMut(&[u8], Instant) + Send>;

/// Run `body` on a new PipeWire connection whose loop quits on a stop from `stopped` or a core
/// error. `body` gets the loop, the core and whether a stop came.
pub fn on_pipewire(
    stopped: pipewire::channel::Receiver<()>,
    body: impl FnOnce(
        &pipewire::main_loop::MainLoopRc,
        &pipewire::core::CoreRc,
        &Cell<bool>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    connect(stopped, log::Level::Warn, body)
}

/// [`on_pipewire`], logging an error on an object other than the core at `object_errors`.
fn connect(
    stopped: pipewire::channel::Receiver<()>,
    object_errors: log::Level,
    body: impl FnOnce(
        &pipewire::main_loop::MainLoopRc,
        &pipewire::core::CoreRc,
        &Cell<bool>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let main_loop = pipewire::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pipewire::context::ContextRc::new(&main_loop, None).map_err(|e| e.to_string())?;
    let core = context.connect_rc(None).map_err(|e| e.to_string())?;
    // Attached first, so a stop also ends a wait on a PipeWire that never answers.
    let stopping = Rc::new(Cell::new(false));
    let weak = main_loop.downgrade();
    let _stop = stopped.attach(main_loop.loop_(), {
        let stopping = stopping.clone();
        move |()| {
            stopping.set(true);
            if let Some(l) = weak.upgrade() {
                l.quit();
            }
        }
    });
    let weak = main_loop.downgrade();
    let _errors = core
        .add_listener_local()
        .error(move |id, _, res, message| {
            let core = id == pipewire::core::PW_ID_CORE;
            let level = if core {
                log::Level::Warn
            } else {
                object_errors
            };
            log::log!(level, "sound: PipeWire object {id}: {message} ({res})");
            if core && let Some(l) = weak.upgrade() {
                l.quit();
            }
        })
        .register();
    body(&main_loop, &core, &stopping)
}

/// What [`capture`] listens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap<'a> {
    /// The monitor of the sink of this name; the default sink, followed as it changes, for None.
    Monitor(Option<&'a str>),
    /// The default source, followed as it changes. The stream is linked from a source, so the
    /// taskbar's microphone shows while it runs.
    Microphone,
}

/// How long the microphone's stream may stay unlinked after it streamed before its capture ends:
/// WirePlumber moves it to a new default source well within that.
const UNLINKED_FOR: Duration = Duration::from_secs(2);

/// Capture `tap` into `samples`: S16LE stereo at `rate`, in the chunks PipeWire hands over, each
/// with the time of the cycle that queued it, so chunks the thread takes late in a burst keep
/// their own times. Runs until a stop from `stopped`, a core error, for a named sink the sink
/// going or, for the microphone, its source going with none to follow; a stream error is the Err,
/// for the microphone [`NO_MICROPHONE`] before it streamed and [`MICROPHONE_LOST`] for its source
/// going after.
/// The stream is called `name` for program `app`.
pub fn capture(
    tap: Tap,
    app: &str,
    name: &str,
    rate: u32,
    samples: Samples,
    stopped: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    pipewire::init();
    connect(stopped, log::Level::Debug, |main_loop, core, _| {
        let mut props = properties! {
            *pipewire::keys::MEDIA_TYPE => "Audio",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::APP_NAME => app,
        };
        if let Tap::Monitor(target) = tap {
            props.insert("stream.capture.sink", "true");
            if let Some(target) = target {
                props.insert("target.object", target);
                props.insert("node.dont-reconnect", "true");
            }
        }
        props.insert("node.latency", format!("{}/{rate}", rate / 50));
        let stream = StreamRc::new(core.clone(), name, props).map_err(|e| e.to_string())?;
        let weak = main_loop.downgrade();
        let failed = Rc::new(RefCell::new(None));
        let unlinked = Rc::new(Cell::new(Unlinked::default()));
        let microphone = tap == Tap::Microphone;
        let _listener = stream
            .add_local_listener_with_user_data(samples)
            .state_changed({
                let (failed, unlinked) = (failed.clone(), unlinked.clone());
                move |_, _, old, new| {
                    let mut u = unlinked.get();
                    u.changed(&old, &new, Instant::now());
                    unlinked.set(u);
                    if let StreamState::Error(e) = new {
                        *failed.borrow_mut() = Some(if microphone { u.why(e) } else { e });
                        if let Some(l) = weak.upgrade() {
                            l.quit();
                        }
                    }
                }
            })
            .process(|stream, samples| {
                // SAFETY: the stream's own buffer, queued back below; `Buffer` hides its `time`.
                let raw = unsafe { stream.dequeue_raw_buffer() };
                // SAFETY: null, or a dequeued buffer the stream keeps until it is queued back.
                let Some(buffer) = (unsafe { raw.as_mut() }) else {
                    return;
                };
                // SAFETY: `pw_stream_get_nsec` reads the stream's clock, the one `time` is on.
                let now = unsafe { pipewire::sys::pw_stream_get_nsec(stream.as_raw_ptr()) };
                let at = cycle_instant(buffer.time, now, Instant::now());
                if let Some(data) = first_data(buffer) {
                    let chunk = data.chunk();
                    let (offset, size) = (chunk.offset() as usize, chunk.size() as usize);
                    if let Some(bytes) = data.data().and_then(|d| d.get(offset..offset + size)) {
                        samples(bytes, at);
                    }
                }
                // SAFETY: dequeued from this stream above.
                unsafe { stream.queue_raw_buffer(raw) };
            })
            .register()
            .map_err(|e| e.to_string())?;
        let param = format_param(rate)?;
        let pod = Pod::from_bytes(&param).ok_or("bad format pod")?;
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut [pod],
            )
            .map_err(|e| e.to_string())?;
        let _gone = microphone.then(|| {
            let (failed, weak) = (failed.clone(), main_loop.downgrade());
            let timer = main_loop.loop_().add_timer(move |_| {
                if unlinked.get().gone(Instant::now()) {
                    log::info!("sound: the microphone's source went away");
                    *failed.borrow_mut() = Some(MICROPHONE_LOST.to_string());
                    if let Some(l) = weak.upgrade() {
                        l.quit();
                    }
                }
            });
            let every = UNLINKED_FOR / 4;
            if let Err(e) = timer.update_timer(Some(every), Some(every)).into_result() {
                log::warn!("sound: a lost microphone goes unnoticed: {e}");
            }
            timer
        });
        log::info!("sound: capturing \"{name}\"");
        main_loop.run();
        let _ = stream.disconnect();
        failed.take().map_or(Ok(()), Err)
    })
}

/// The microphone's capture error when its stream failed before it ever streamed, as WirePlumber
/// fails it when no source is there.
pub const NO_MICROPHONE: &str = "No microphone was found.";

/// The microphone's capture error when its source went away after it streamed.
pub const MICROPHONE_LOST: &str =
    "The microphone was disconnected; the rest of the recording has only what played.";

/// WirePlumber's stream error when the stream's source went away with none to follow.
const NO_TARGET: &str = "no target node available";

/// Whether a stream has streamed, and since when it has been unlinked from its source after.
#[derive(Clone, Copy, Debug, Default)]
struct Unlinked {
    streamed: bool,
    since: Option<Instant>,
}

impl Unlinked {
    fn changed(&mut self, old: &StreamState, new: &StreamState, now: Instant) {
        match (old, new) {
            (StreamState::Streaming, StreamState::Paused) => self.since = Some(now),
            (_, StreamState::Streaming) => {
                self.streamed = true;
                self.since = None;
            }
            _ => {}
        }
    }

    /// The microphone's capture Err for the stream error `e`, which goes to the log as it came.
    fn why(&self, e: String) -> String {
        if !self.streamed {
            log::info!("sound: the microphone did not open: {e}");
            return NO_MICROPHONE.to_string();
        }
        log::info!("sound: the microphone stopped: {e}");
        match e.as_str() {
            NO_TARGET => MICROPHONE_LOST.to_string(),
            _ => e,
        }
    }

    fn gone(&self, now: Instant) -> bool {
        self.since
            .is_some_and(|t| now.saturating_duration_since(t) >= UNLINKED_FOR)
    }
}

/// The first data block of a dequeued `buffer`.
fn first_data(buffer: &mut pipewire::sys::pw_buffer) -> Option<&mut spa::buffer::Data> {
    // SAFETY: a dequeued buffer's spa_buffer holds `n_datas` blocks at `datas`, and `Data` is a
    // transparent `spa_data`.
    unsafe {
        let b = buffer.buffer.as_mut()?;
        if b.n_datas == 0 {
            return None;
        }
        (b.datas as *mut spa::buffer::Data).as_mut()
    }
}

/// The instant of the stream clock's `ns` when that clock reads `now_ns` at `now`; `now` when
/// PipeWire left `ns` unset.
fn cycle_instant(ns: u64, now_ns: u64, now: Instant) -> Instant {
    let ago = Duration::from_nanos(now_ns.saturating_sub(ns));
    match ns {
        0 => now,
        _ => now.checked_sub(ago).unwrap_or(now),
    }
}

fn format_param(rate: u32) -> Result<Vec<u8>, String> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::S16LE);
    info.set_rate(rate);
    info.set_channels(2);
    let mut position = [0; spa_sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    PodSerializer::serialize(
        io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|e| format!("format pod: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_keeps_the_time_of_the_cycle_that_queued_it() {
        let now = Instant::now() + Duration::from_secs(10);
        let at = cycle_instant(5_000_000_000, 5_120_000_000, now);
        assert_eq!(now - at, Duration::from_millis(120));
        assert_eq!(cycle_instant(0, 5_120_000_000, now), now, "unset");
    }

    #[test]
    fn a_stream_unlinked_after_streaming_is_gone_unless_relinked_in_time() {
        use StreamState::{Connecting, Paused, Streaming};
        let t = Instant::now();
        let mut u = Unlinked::default();
        u.changed(&Connecting, &Paused, t);
        assert!(!u.gone(t + UNLINKED_FOR), "not yet streamed");
        u.changed(&Paused, &Streaming, t);
        u.changed(&Streaming, &Paused, t);
        assert!(!u.gone(t + UNLINKED_FOR / 2));
        assert!(u.gone(t + UNLINKED_FOR));
        u.changed(&Paused, &Streaming, t + UNLINKED_FOR / 2);
        assert!(!u.gone(t + UNLINKED_FOR * 2), "moved to another source");
    }

    #[test]
    fn a_microphone_failing_before_it_streamed_was_not_found_and_after_was_disconnected() {
        use StreamState::{Connecting, Paused, Streaming};
        let unplugged = || "no target node available".to_string();
        let mut u = Unlinked::default();
        u.changed(&Connecting, &Paused, Instant::now());
        assert_eq!(u.why(unplugged()), NO_MICROPHONE);
        u.changed(&Paused, &Streaming, Instant::now());
        assert_eq!(u.why(unplugged()), MICROPHONE_LOST);
        let other = "error alloc buffers: Invalid argument";
        assert_eq!(u.why(other.into()), other, "the recorder words any other");
    }
}
