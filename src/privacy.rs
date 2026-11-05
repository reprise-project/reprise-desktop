//! The taskbar's privacy indicators: a record dot while an output or window is captured, a microphone while an app
//! records, a camera while one films, each with the app's name in its tooltip (names come from PipeWire).

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    LoopHandle, RegistrationToken,
    channel::{Event, Sender, channel},
    timer::{TimeoutAction, Timer},
};

use crate::{captureborder::Captured, compositor::State};

#[cfg(not(test))]
const HOLD: Duration = Duration::from_secs(5);
/// Short, so tests wait out the dot's linger.
#[cfg(test)]
const HOLD: Duration = Duration::from_millis(500);

const SKIPPED_APPS: [&str; 4] = [
    "org.PulseAudio.pavucontrol",
    "org.gnome.VolumeControl",
    "org.kde.kmixd",
    "org.kde.plasma-pa",
];

#[derive(Default)]
pub struct Privacy {
    /// What the last ongoing captures showed: a screen or area, a window.
    screen: bool,
    window: bool,
    /// What the ongoing captures show now, as `screen` and `window`: none once they end.
    ongoing: (bool, bool),
    /// Whether an ext session was ongoing at the last `note_live`.
    ext: bool,
    /// When a capture was last seen live; the linger runs `HOLD` from here.
    live_at: Option<Instant>,
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
        share_label(&self.share_apps, self.screen, self.window)
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

/// A capture started, asked for a frame or ended: the dot shows while one
/// is ongoing (`captureborder::ongoing`), so a screenshot shows none.
/// Only a change in what is ongoing re-arms the hold, so the dot goes
/// `HOLD` after the last ongoing capture ends: an ext session when it goes,
/// a wlr recorder at its last frame rather than when the border lets it go.
pub fn captures_changed(state: &mut State) {
    let at = Instant::now();
    note_live(state, at);
    let ongoing = kinds(&crate::captureborder::ongoing(state));
    let p = &mut state.privacy;
    if ongoing == p.ongoing {
        return;
    }
    p.ongoing = ongoing;
    log::info!(
        "ongoing captures: screen {}, window {}",
        ongoing.0,
        ongoing.1
    );
    let live = ongoing.0 || ongoing.1;
    let from = if live {
        (p.screen, p.window) = ongoing;
        at
    } else if p.sharing {
        p.live_at.unwrap_or(at)
    } else {
        return;
    };
    arm_hold(state, from);
    let p = &mut state.privacy;
    if live && !p.sharing {
        p.sharing = true;
        log::info!("screen sharing indicator on");
        state.redraw_taskbars();
        crate::notifyview::suppress_changed(state);
    }
}

/// Moves `live_at` to `at` while an ext session is live or has just gone
/// or a wlr frame waits, and to a wlr recording's newest frame.
fn note_live(state: &mut State, at: Instant) {
    let ext = !state.captures.targets().is_empty();
    let waiting = !state.captures.wlr_waiting().is_empty();
    let recorded = crate::captureborder::last_recorded(state);
    let p = &mut state.privacy;
    if ext || p.ext || waiting {
        p.live_at = Some(at);
    }
    p.live_at = p.live_at.max(recorded);
    p.ext = ext;
}

/// Runs the hold `HOLD` from `from`, at once if that has passed.
fn arm_hold(state: &mut State, from: Instant) {
    if let Some(t) = state.privacy.hold.take() {
        state.handle.remove(t);
    }
    match state
        .handle
        .insert_source(Timer::from_deadline(from + HOLD), |_, _, state| {
            hold_fired(state)
        }) {
        Ok(t) => state.privacy.hold = Some(t),
        Err(e) => log::warn!("privacy: hold timer: {e}"),
    }
}

/// The hold ran out: the dot goes unless a session is still live, which
/// keeps it for another hold, or one was live less than `HOLD` ago.
fn hold_fired(state: &mut State) -> TimeoutAction {
    if !crate::captureborder::ongoing(state).is_empty() {
        return TimeoutAction::ToDuration(HOLD);
    }
    let at = Instant::now();
    note_live(state, at);
    if let Some(end) = state.privacy.live_at.map(|t| t + HOLD)
        && end > at
    {
        return TimeoutAction::ToInstant(end);
    }
    let p = &mut state.privacy;
    p.hold = None;
    if p.sharing {
        p.sharing = false;
        log::info!("screen sharing indicator off");
        state.redraw_taskbars();
        crate::notifyview::suppress_changed(state);
    }
    TimeoutAction::Drop
}

/// Whether `targets` show a screen or an area of one, and a window.
fn kinds<O, W>(targets: &[Captured<O, W>]) -> (bool, bool) {
    let window = |t: &Captured<O, W>| matches!(t, Captured::Window(_));
    (
        targets.iter().any(|t| !window(t)),
        targets.iter().any(window),
    )
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

    const SIZE: (i32, i32) = (64, 48);

    fn capturing() -> crate::harness::Harness {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", SIZE.0, SIZE.1);
        h
    }

    fn note(id: u32) -> crate::tray::Notification {
        let mut n = crate::tray::Notification::new(id, format!("n{id}"), "x".into());
        n.app = "Mail".into();
        n.key = "Mail".into();
        n
    }

    #[test]
    fn screenshots_back_to_back_are_not_sharing_and_balloons_still_show() {
        let mut h = capturing();
        for _ in 0..3 {
            h.connect_capture(SIZE).wlr_shot(&mut h);
            let mut c = h.connect_capture(SIZE);
            let mut s = c.ext_session(&mut h);
            c.ext_frame(&mut h, &mut s);
            c.ext_end(&mut h, s);
        }
        assert!(!h.state.privacy.sharing_shown(), "no sharing dot");
        assert!(crate::captureborder::ongoing(&h.state).is_empty());
        crate::notifyview::show(&mut h.state, note(1));
        assert!(crate::notifyview::balloon_up(&h.state), "the balloon shows");
    }

    #[test]
    fn a_session_asking_for_frames_is_sharing_and_its_held_balloons_show_once_it_ends() {
        let mut h = capturing();
        let mut c = h.connect_capture(SIZE);
        let mut s = c.ext_session(&mut h);
        c.ext_frame(&mut h, &mut s);
        assert!(
            !h.state.privacy.sharing_shown(),
            "one frame is a screenshot"
        );
        c.ext_frame(&mut h, &mut s);
        assert!(h.state.privacy.sharing_shown(), "the second frame shares");
        assert_eq!(h.state.privacy.sharing_tip(), "Your screen is being shared");
        crate::notifyview::show(&mut h.state, note(1));
        assert!(
            !crate::notifyview::balloon_up(&h.state),
            "held while shared"
        );
        c.ext_end(&mut h, s);
        let ended = std::time::Instant::now();
        assert!(
            h.state.privacy.sharing_shown(),
            "the dot outlasts the capture"
        );
        run_until(&mut h, ended, 1.1);
        assert!(!h.state.privacy.sharing_shown());
        assert!(
            crate::notifyview::balloon_up(&h.state),
            "the held balloon shows once sharing ends"
        );
    }

    #[test]
    fn one_client_copying_frames_in_a_row_is_sharing() {
        let mut h = capturing();
        let mut c = h.connect_capture(SIZE);
        c.wlr_shot(&mut h);
        assert!(!h.state.privacy.sharing_shown());
        c.wlr_shot(&mut h);
        assert!(h.state.privacy.sharing_shown(), "a recording");
        assert_eq!(crate::captureborder::ongoing(&h.state).len(), 1);
        crate::notifyview::show(&mut h.state, note(1));
        assert!(
            !crate::notifyview::balloon_up(&h.state),
            "held while recorded"
        );
        assert_eq!(h.state.notify.log.held.len(), 1);
    }

    /// A screen cast with a balloon held under its dot, ended: when it ended.
    fn cast_with_held_balloon(h: &mut crate::harness::Harness) -> std::time::Instant {
        let mut c = h.connect_capture(SIZE);
        let mut s = c.ext_session(h);
        c.ext_frame(h, &mut s);
        c.ext_frame(h, &mut s);
        crate::notifyview::show(&mut h.state, note(1));
        assert!(
            !crate::notifyview::balloon_up(&h.state),
            "held while shared"
        );
        c.ext_end(h, s);
        std::time::Instant::now()
    }

    /// A screenshot through a one-frame ext session, as grim and the portal take it, and one
    /// through wlr-screencopy.
    fn screenshot(h: &mut crate::harness::Harness) {
        let mut c = h.connect_capture(SIZE);
        let mut s = c.ext_session(h);
        c.ext_frame(h, &mut s);
        c.ext_end(h, s);
        c.wlr_shot(h);
    }

    /// Turns of the loop until `HOLD` times `holds` after `from`.
    fn run_until(h: &mut crate::harness::Harness, from: std::time::Instant, holds: f32) {
        let left =
            (from + HOLD.mul_f32(holds)).saturating_duration_since(std::time::Instant::now());
        h.run_for(left.as_millis() as u64);
    }

    #[test]
    fn a_screenshot_during_the_linger_does_not_extend_it() {
        let mut h = capturing();
        let ended = cast_with_held_balloon(&mut h);
        run_until(&mut h, ended, 0.4);
        screenshot(&mut h);
        assert!(h.state.privacy.sharing_shown(), "the dot lingers");
        run_until(&mut h, ended, 1.2);
        assert!(
            !h.state.privacy.sharing_shown(),
            "the dot goes HOLD after the cast ends"
        );
        assert!(
            crate::notifyview::balloon_up(&h.state),
            "the held balloon shows once the dot goes"
        );
    }

    #[test]
    fn a_wlr_recording_that_stops_loses_the_dot_5_s_after_its_last_frame() {
        let mut h = capturing();
        let mut c = h.connect_capture(SIZE);
        for _ in 0..3 {
            c.wlr_shot(&mut h);
            h.run_for(40);
        }
        c.wlr_shot(&mut h);
        let last = std::time::Instant::now();
        assert!(h.state.privacy.sharing_shown(), "a recording");
        run_until(&mut h, last, 0.9);
        assert!(h.state.privacy.sharing_shown(), "the dot lingers");
        run_until(&mut h, last, 1.1);
        assert!(
            !h.state.privacy.sharing_shown(),
            "the dot goes HOLD after the last frame"
        );
    }

    #[test]
    fn screenshots_every_few_seconds_after_a_cast_do_not_hold_the_dot() {
        let mut h = capturing();
        let ended = cast_with_held_balloon(&mut h);
        for k in 1..=5 {
            run_until(&mut h, ended, 0.4 * k as f32);
            screenshot(&mut h);
        }
        assert!(!h.state.privacy.sharing_shown(), "no sharing dot");
        assert!(crate::notifyview::balloon_up(&h.state), "the balloon shows");
    }
}
