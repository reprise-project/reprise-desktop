//! The taskbar's privacy indicators: a record dot while an output or window is captured, a microphone while an app
//! records, a camera while one films, each with the app's name in its tooltip (names come from PipeWire).

use std::thread::JoinHandle;
use std::time::Duration;

use smithay::reexports::calloop::{
    LoopHandle, RegistrationToken,
    channel::{Event, Sender, channel},
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;

const HOLD: Duration = Duration::from_secs(5);

const SKIPPED_APPS: [&str; 4] = [
    "org.PulseAudio.pavucontrol",
    "org.gnome.VolumeControl",
    "org.kde.kmixd",
    "org.kde.plasma-pa",
];

#[derive(Default)]
pub struct Privacy {
    outputs: usize,
    windows: usize,
    legacy: bool,
    sharing: bool,
    hold: Option<RegistrationToken>,
    mic_apps: Vec<String>,
    share_apps: Vec<String>,
    camera_apps: Vec<String>,
    camera_holders: Vec<crate::camdev::Holder>,
    _mic: Option<MicMonitor>,
}

struct MicMonitor {
    quit: pipewire::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for MicMonitor {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Privacy {
    pub fn start(handle: &LoopHandle<'static, State>) -> Privacy {
        start_camera_devices(handle);
        Privacy {
            _mic: start_mic_monitor(handle),
            ..Privacy::default()
        }
    }

    pub fn sharing_shown(&self) -> bool {
        self.sharing
    }

    pub fn mic_shown(&self) -> bool {
        !self.mic_apps.is_empty()
    }

    pub fn camera_shown(&self) -> bool {
        !self.camera_apps.is_empty() || !self.camera_holders.is_empty()
    }

    pub fn sharing_tip(&self) -> String {
        share_label(
            &self.share_apps,
            self.outputs > 0 || self.legacy,
            self.windows > 0,
        )
    }

    pub fn mic_tip(&self) -> String {
        using_label(&self.mic_apps, "microphone")
    }
}

/// The camera's tooltip: the PipeWire consumers by name, and each
/// process holding a device by the window of the process or the nearest
/// parent that has one, else by its program's file name.
pub fn camera_tip(state: &State) -> String {
    let p = &state.privacy;
    let mut names = p.camera_apps.clone();
    for h in &p.camera_holders {
        let window = h.chain.iter().find_map(|pid| {
            state
                .windows
                .iter()
                .find(|w| w.mapped && state.client_pid(w.id) == Some(*pid))
        });
        let name = window
            .map(|w| {
                let app = w.app();
                reprisefs::apps::entry_for_app_id(&state.app_index, &app, |_| true)
                    .map(|e| e.name.clone())
                    .unwrap_or(app)
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| h.exe.clone());
        names.push(name);
    }
    names.sort();
    names.dedup();
    using_label(&names, "camera")
}

fn using_label(apps: &[String], what: &str) -> String {
    match apps {
        [] => format!("An application is using your {what}"),
        [one] => format!("{one} is using your {what}"),
        [rest @ .., last] => format!("{} and {last} are using your {what}", rest.join(", ")),
    }
}

fn start_camera_devices(handle: &LoopHandle<'static, State>) {
    let (tx, rx) = channel::<Vec<crate::camdev::Holder>>();
    let inserted = handle.insert_source(rx, |event, _, state| {
        let Event::Msg(holders) = event else {
            return;
        };
        if holders != state.privacy.camera_holders {
            log::info!("camera devices held by {holders:?}");
            state.privacy.camera_holders = holders;
            state.redraw_taskbars();
        }
    });
    match inserted {
        Ok(_) => crate::camdev::start(tx),
        Err(e) => log::warn!("privacy: camera device channel: {e}"),
    }
}

/// A capture session opened or closed: count them again and show the
/// dot while any is live. Every change re-arms the hold, so the dot goes
/// `HOLD` after the last capture ends.
pub fn sessions_changed(state: &mut State) {
    let (outputs, windows) = state.captures.live();
    let p = &mut state.privacy;
    if (outputs, windows) != (p.outputs, p.windows) {
        log::info!("capture sessions: {outputs} on outputs, {windows} on windows");
    }
    p.outputs = outputs;
    p.windows = windows;
    refresh(state);
}

/// A wlr-screencopy frame (grim, wf-recorder): it has no session, so it
/// shows the dot for the hold.
pub fn legacy_frame(state: &mut State) {
    state.privacy.legacy = true;
    refresh(state);
}

fn refresh(state: &mut State) {
    arm_hold(state);
    let p = &mut state.privacy;
    let live = p.outputs + p.windows > 0 || p.legacy;
    if live && !p.sharing {
        p.sharing = true;
        log::info!("screen sharing indicator on");
        state.redraw_taskbars();
        crate::notifyview::suppress_changed(state);
    }
}

fn arm_hold(state: &mut State) {
    if let Some(t) = state.privacy.hold.take() {
        state.handle.remove(t);
    }
    match state
        .handle
        .insert_source(Timer::from_duration(HOLD), |_, _, state| hold_fired(state))
    {
        Ok(t) => state.privacy.hold = Some(t),
        Err(e) => log::warn!("privacy: hold timer: {e}"),
    }
}

/// The hold ran out: the dot goes unless a session is still live, which
/// keeps it for another hold.
fn hold_fired(state: &mut State) -> TimeoutAction {
    (state.privacy.outputs, state.privacy.windows) = state.captures.live();
    let p = &mut state.privacy;
    p.legacy = false;
    if p.outputs + p.windows > 0 {
        return TimeoutAction::ToDuration(HOLD);
    }
    p.hold = None;
    if p.sharing {
        p.sharing = false;
        log::info!("screen sharing indicator off");
        state.redraw_taskbars();
        crate::notifyview::suppress_changed(state);
    }
    TimeoutAction::Drop
}

/// The dot's tooltip: who is sharing what. `screen` and `window` say
/// what kind of capture is live; `apps` names the consumers, if known.
fn share_label(apps: &[String], screen: bool, window: bool) -> String {
    let what = match (screen, window) {
        (true, true) => "your screen and a window",
        (false, true) => "a window",
        _ => "your screen",
    };
    match apps {
        [] => {
            let cap = what[..1].to_uppercase() + &what[1..];
            format!(
                "{cap} {} being shared",
                if screen && window { "are" } else { "is" }
            )
        }
        [one] => format!("{one} is sharing {what}"),
        [rest @ .., last] => format!("{} and {last} are sharing {what}", rest.join(", ")),
    }
}

fn start_mic_monitor(handle: &LoopHandle<'static, State>) -> Option<MicMonitor> {
    let (tx, rx) = channel::<Apps>();
    let (quit, quit_rx) = pipewire::channel::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("mic-monitor".into())
        .spawn(move || {
            if let Err(e) = mic_thread(tx, quit_rx) {
                log::info!("privacy: PipeWire unavailable ({e}); microphone indicator disabled");
            }
        })
        .ok()?;
    let inserted = handle.insert_source(rx, |event, _, state| {
        let Event::Msg(latest) = event else {
            return;
        };
        let p = &mut state.privacy;
        if latest.mic != p.mic_apps
            || latest.share != p.share_apps
            || latest.camera != p.camera_apps
        {
            log::info!(
                "microphone in use by {:?}; sharing to {:?}; camera in use by {:?}",
                latest.mic,
                latest.share,
                latest.camera
            );
            p.mic_apps = latest.mic;
            p.share_apps = latest.share;
            p.camera_apps = latest.camera;
            state.redraw_taskbars();
        }
    });
    if let Err(e) = inserted {
        log::warn!("privacy: microphone channel: {e}");
    }
    Some(MicMonitor {
        quit,
        thread: Some(thread),
    })
}

const SCREENCAST_NODE: &str = "reprise-screencast";

#[derive(Default, Clone)]
struct NodeInfo {
    class: String,
    name: String,
    /// `application.name`, from the node, else from its client: a
    /// screencast consumer (OBS, Chromium) names only the client.
    app: String,
    app_id: String,
    client: u32,
    running: bool,
}

/// The registry view the thread keeps: nodes by id, links as
/// (output node, input node) by id, and the bound proxies that keep the
/// node state events flowing.
#[derive(Default)]
struct Graph {
    nodes: std::collections::HashMap<u32, NodeInfo>,
    /// Clients by id: their `application.name`. A screencast consumer
    /// connects over the fd xdg-desktop-portal handed it, so the global
    /// carries the portal's props and the app's own arrive in its info.
    clients: std::collections::HashMap<u32, String>,
    client_proxies: std::collections::HashMap<
        u32,
        (pipewire::client::Client, pipewire::client::ClientListener),
    >,
    links: std::collections::HashMap<u32, (u32, u32)>,
    proxies: std::collections::HashMap<u32, (pipewire::node::Node, pipewire::node::NodeListener)>,
    last: Apps,
}

/// What the thread reports: who records from a microphone, who consumes
/// a screencast and who films, by `application.name`.
#[derive(Default, Clone, PartialEq, Debug)]
struct Apps {
    mic: Vec<String>,
    share: Vec<String>,
    camera: Vec<String>,
}

impl Graph {
    /// Whether `class` is a node that produces microphone (not monitor)
    /// audio.
    fn is_mic_source(class: &str) -> bool {
        class == "Audio/Source" || class.starts_with("Audio/Source/") || class == "Audio/Duplex"
    }

    fn recording(&self) -> Vec<String> {
        self.consumers("Stream/Input/Audio", true, |o| {
            Self::is_mic_source(&o.class)
        })
    }

    /// The apps consuming a Reprise screencast, sorted, deduplicated. A
    /// paused consumer still holds the stream, so Running is not required.
    fn sharing(&self) -> Vec<String> {
        self.consumers("Stream/Input/Video", false, |o| o.name == SCREENCAST_NODE)
    }

    /// The apps filming right now: Running video capture streams fed by a
    /// video source that is not a Reprise screencast.
    fn filming(&self) -> Vec<String> {
        self.consumers("Stream/Input/Video", true, |o| {
            o.class == "Video/Source" && o.name != SCREENCAST_NODE
        })
    }

    /// Names of the `class` stream nodes (Running if `running`) fed by a
    /// link from a node `from` accepts. One pass over the links.
    fn consumers(
        &self,
        class: &str,
        running: bool,
        from: impl Fn(&NodeInfo) -> bool,
    ) -> Vec<String> {
        let mut apps: Vec<String> = self
            .links
            .values()
            .filter(|(out, _)| self.nodes.get(out).is_some_and(&from))
            .filter_map(|(_, inp)| self.nodes.get(inp))
            .filter(|n| n.class.starts_with(class) && (n.running || !running))
            .filter(|n| !SKIPPED_APPS.contains(&n.app_id.as_str()))
            .map(|n| {
                let client = self.clients.get(&n.client).filter(|c| !c.is_empty());
                match (n.app.is_empty(), client) {
                    (false, _) => n.app.clone(),
                    (true, Some(c)) => c.clone(),
                    (true, None) => "An application".to_string(),
                }
            })
            .collect();
        apps.sort();
        apps.dedup();
        apps
    }

    fn report(&self) -> Apps {
        Apps {
            mic: self.recording(),
            share: self.sharing(),
            camera: self.filming(),
        }
    }
}

fn mic_thread(
    tx: Sender<Apps>,
    quit: pipewire::channel::Receiver<()>,
) -> Result<(), pipewire::Error> {
    use pipewire::{context::ContextRc, main_loop::MainLoopRc, types::ObjectType};
    use std::cell::RefCell;
    use std::rc::Rc;

    pipewire::init();
    let main_loop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let graph = Rc::new(RefCell::new(Graph::default()));

    let report = {
        let graph = graph.clone();
        let tx = tx.clone();
        Rc::new(move || {
            let mut g = graph.borrow_mut();
            let now = g.report();
            if now != g.last {
                g.last = now.clone();
                let _ = tx.send(now);
            }
        })
    };

    let registry_weak = registry.downgrade();
    let _listener = {
        let graph = graph.clone();
        let report_add = report.clone();
        let graph_rm = graph.clone();
        let report_rm = report.clone();
        registry
            .add_listener_local()
            .global(move |obj| {
                let Some(props) = obj.props.as_ref() else {
                    return;
                };
                let get = |k: &str| props.get(k).unwrap_or("").to_string();
                match &obj.type_ {
                    ObjectType::Node => {
                        let class = get("media.class");
                        graph.borrow_mut().nodes.insert(
                            obj.id,
                            NodeInfo {
                                class: class.clone(),
                                name: get("node.name"),
                                app: get("application.name"),
                                app_id: get("application.id"),
                                client: get("client.id").parse().unwrap_or(0),
                                running: false,
                            },
                        );
                        if (class.starts_with("Stream/Input/Audio")
                            || class.starts_with("Stream/Input/Video"))
                            && let Some(registry) = registry_weak.upgrade()
                            && let Ok(node) = registry.bind::<pipewire::node::Node, _>(obj)
                        {
                            let id = obj.id;
                            let graph2 = graph.clone();
                            let report2 = report_add.clone();
                            let l = node
                                .add_listener_local()
                                .info(move |info| {
                                    let running =
                                        matches!(info.state(), pipewire::node::NodeState::Running);
                                    if let Some(n) = graph2.borrow_mut().nodes.get_mut(&id) {
                                        n.running = running;
                                    }
                                    report2();
                                })
                                .register();
                            graph.borrow_mut().proxies.insert(id, (node, l));
                        }
                    }
                    ObjectType::Client => {
                        graph
                            .borrow_mut()
                            .clients
                            .insert(obj.id, get("application.name"));
                        if let Some(registry) = registry_weak.upgrade()
                            && let Ok(client) = registry.bind::<pipewire::client::Client, _>(obj)
                        {
                            let id = obj.id;
                            let graph2 = graph.clone();
                            let report2 = report_add.clone();
                            let l = client
                                .add_listener_local()
                                .info(move |info| {
                                    let name = info
                                        .props()
                                        .and_then(|p| p.get("application.name"))
                                        .unwrap_or("")
                                        .to_string();
                                    graph2.borrow_mut().clients.insert(id, name);
                                    report2();
                                })
                                .register();
                            graph.borrow_mut().client_proxies.insert(id, (client, l));
                        }
                        report_add();
                    }
                    ObjectType::Link => {
                        let out = get("link.output.node").parse().unwrap_or(0);
                        let inp = get("link.input.node").parse().unwrap_or(0);
                        graph.borrow_mut().links.insert(obj.id, (out, inp));
                        report_add();
                    }
                    _ => {}
                }
            })
            .global_remove(move |id| {
                let mut g = graph_rm.borrow_mut();
                g.nodes.remove(&id);
                g.clients.remove(&id);
                g.client_proxies.remove(&id);
                g.links.remove(&id);
                g.proxies.remove(&id);
                drop(g);
                report_rm();
            })
            .register()
    };

    let weak = main_loop.downgrade();
    let _quit = quit.attach(main_loop.loop_(), move |_| {
        if let Some(l) = weak.upgrade() {
            l.quit();
        }
    });
    main_loop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(class: &str, app: &str, running: bool) -> NodeInfo {
        NodeInfo {
            class: class.into(),
            name: String::new(),
            app: app.into(),
            app_id: String::new(),
            client: 0,
            running,
        }
    }

    #[test]
    fn mic_capture_counts_and_monitor_capture_does_not() {
        let mut g = Graph::default();
        g.nodes.insert(1, node("Audio/Source", "", false));
        g.nodes.insert(2, node("Audio/Sink", "", false));
        g.nodes
            .insert(10, node("Stream/Input/Audio", "Chromium", true));
        g.nodes.insert(11, node("Stream/Input/Audio", "OBS", true));
        g.links.insert(100, (1, 10));
        g.links.insert(101, (2, 11));
        assert_eq!(g.recording(), vec!["Chromium".to_string()]);
    }

    #[test]
    fn a_stream_fed_by_one_link_per_channel_is_named_once() {
        let mut g = Graph::default();
        g.nodes.insert(1, node("Audio/Source", "", false));
        g.nodes
            .insert(10, node("Stream/Input/Audio", "Chromium", true));
        g.links.insert(100, (1, 10));
        g.links.insert(101, (1, 10));
        assert_eq!(g.recording(), vec!["Chromium".to_string()]);
    }

    #[test]
    fn an_idle_stream_and_a_volume_applet_are_skipped() {
        let mut g = Graph::default();
        g.nodes.insert(1, node("Audio/Source/Virtual", "", false));
        g.nodes
            .insert(10, node("Stream/Input/Audio", "Zoom", false));
        let mut pav = node("Stream/Input/Audio", "pavucontrol", true);
        pav.app_id = "org.PulseAudio.pavucontrol".into();
        g.nodes.insert(11, pav);
        g.links.insert(100, (1, 10));
        g.links.insert(101, (1, 11));
        assert!(g.recording().is_empty());
        g.nodes.get_mut(&10).unwrap().running = true;
        assert_eq!(g.recording(), vec!["Zoom".to_string()]);
    }

    #[test]
    fn a_screencast_consumer_is_named_and_a_camera_consumer_is_not() {
        let mut g = Graph::default();
        let mut cast = node("Video/Source", "", true);
        cast.name = SCREENCAST_NODE.into();
        g.nodes.insert(1, cast);
        g.nodes.insert(2, node("Video/Source", "", true));
        let mut obs = node("Stream/Input/Video", "", false);
        obs.client = 57;
        g.clients.insert(57, "obs".into());
        g.nodes.insert(10, obs);
        g.nodes
            .insert(11, node("Stream/Input/Video", "Cheese", true));
        g.links.insert(100, (1, 10));
        g.links.insert(101, (2, 11));
        assert_eq!(g.sharing(), vec!["obs".to_string()]);
        assert!(g.recording().is_empty());
    }

    #[test]
    fn a_running_camera_consumer_is_named_and_a_screencast_one_is_not() {
        let mut g = Graph::default();
        let mut cast = node("Video/Source", "", true);
        cast.name = SCREENCAST_NODE.into();
        g.nodes.insert(1, cast);
        g.nodes.insert(2, node("Video/Source", "", true));
        g.nodes.insert(10, node("Stream/Input/Video", "OBS", true));
        g.nodes
            .insert(11, node("Stream/Input/Video", "Cheese", false));
        g.links.insert(100, (1, 10));
        g.links.insert(101, (2, 11));
        assert!(
            g.filming().is_empty(),
            "a paused camera stream does not film"
        );
        g.nodes.get_mut(&11).unwrap().running = true;
        assert_eq!(g.filming(), vec!["Cheese".to_string()]);
    }

    #[test]
    fn using_labels() {
        let none: [String; 0] = [];
        assert_eq!(
            using_label(&none, "camera"),
            "An application is using your camera"
        );
        assert_eq!(
            using_label(&["Cheese".into()], "camera"),
            "Cheese is using your camera"
        );
        assert_eq!(
            using_label(
                &["Cheese".into(), "Firefox".into(), "obs".into()],
                "microphone"
            ),
            "Cheese, Firefox and obs are using your microphone"
        );
    }

    #[test]
    fn share_labels() {
        let none: [String; 0] = [];
        assert_eq!(
            share_label(&none, true, false),
            "Your screen is being shared"
        );
        assert_eq!(
            share_label(&none, true, true),
            "Your screen and a window are being shared"
        );
        assert_eq!(
            share_label(&["Chromium".into()], false, true),
            "Chromium is sharing a window"
        );
        assert_eq!(
            share_label(&["Chromium".into(), "obs".into()], true, true),
            "Chromium and obs are sharing your screen and a window"
        );
    }
}
