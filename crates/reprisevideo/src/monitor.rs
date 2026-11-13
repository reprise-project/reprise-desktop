//! What plays, captured from a sink's monitor on a PipeWire connection of its own.

use pipewire::properties::properties;
use pipewire::spa;
use pipewire::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pipewire::spa::pod::Pod;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::sys as spa_sys;
use pipewire::stream::{StreamFlags, StreamRc, StreamState};
use std::cell::Cell;
use std::io;
use std::rc::Rc;

/// Takes each chunk of captured samples.
pub type Samples = Box<dyn FnMut(&[u8]) + Send>;

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
            log::warn!("sound: PipeWire object {id}: {message} ({res})");
            if id == pipewire::core::PW_ID_CORE
                && let Some(l) = weak.upgrade()
            {
                l.quit();
            }
        })
        .register();
    body(&main_loop, &core, &stopping)
}

/// Capture the monitor of sink `target` (the default sink, followed as it changes, for None) into
/// `samples`: S16LE stereo at `rate`, in the chunks PipeWire hands over. Runs until a stop from
/// `stopped`, a core error or, with a `target`, the sink going; the stream is called `name` for
/// program `app`.
pub fn capture(
    target: Option<&str>,
    app: &str,
    name: &str,
    rate: u32,
    samples: Samples,
    stopped: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    pipewire::init();
    on_pipewire(stopped, |main_loop, core, _| {
        let mut props = properties! {
            *pipewire::keys::MEDIA_TYPE => "Audio",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::APP_NAME => app,
            "stream.capture.sink" => "true",
        };
        if let Some(target) = target {
            props.insert("target.object", target);
            props.insert("node.dont-reconnect", "true");
        }
        props.insert("node.latency", format!("{}/{rate}", rate / 50));
        let stream = StreamRc::new(core.clone(), name, props).map_err(|e| e.to_string())?;
        let weak = main_loop.downgrade();
        let _listener = stream
            .add_local_listener_with_user_data(samples)
            .state_changed(move |_, _, _, new| {
                if let StreamState::Error(e) = new {
                    log::warn!("sound: capture: {e}");
                    if let Some(l) = weak.upgrade() {
                        l.quit();
                    }
                }
            })
            .process(|stream, samples| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let chunk = data.chunk();
                let (offset, size) = (chunk.offset() as usize, chunk.size() as usize);
                if let Some(bytes) = data.data().and_then(|d| d.get(offset..offset + size)) {
                    samples(bytes);
                }
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
        log::info!("sound: capturing \"{name}\"");
        main_loop.run();
        let _ = stream.disconnect();
        Ok(())
    })
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
