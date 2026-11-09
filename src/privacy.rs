//! The taskbar's privacy indicators: a record dot while the screen is captured or an app holds
//! input, a microphone while an app records, a camera while one films, each naming its apps.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    LoopHandle, RegistrationToken,
    channel::{Event, Sender, channel},
    timer::{TimeoutAction, Timer},
};

use smithay::reexports::wayland_server::Client;

use crate::{captureborder::Captured, compositor::State};

#[cfg(not(test))]
const HOLD: Duration = Duration::from_secs(5);
/// Short, so tests wait out the dot's linger.
#[cfg(test)]
const HOLD: Duration = Duration::from_millis(500);

/// How long the dot shows after a program's second still within this time of its first, so
/// captures repeated slower than a recording (a screenshot loop) show and a single one does not.
#[cfg(not(test))]
const STILL_HOLD: Duration = Duration::from_secs(3);
#[cfg(test)]
const STILL_HOLD: Duration = Duration::from_millis(300);

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
    /// The app ids of the apps whose RemoteDesktop sessions hold input, sorted, "" for an unnamed
    /// one (`control_changed`).
    controlling: Vec<String>,
    /// The programs that took a still within `STILL_HOLD`, and when.
    takers: Vec<(Taker, Instant)>,
    /// While `STILL_HOLD` after a repeated still: whether one showed a screen and one a window.
    still: Option<(bool, bool)>,
    still_until: Option<Instant>,
    still_timer: Option<RegistrationToken>,
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

    /// Whether a capture is shared: the dot shows, and balloons wait.
    pub fn sharing_shown(&self) -> bool {
        self.sharing
    }

    /// Whether the tray's dot shows: a capture is shared, a process repeats stills, or an app
    /// controls this computer.
    pub fn dot_shown(&self) -> bool {
        self.sharing || self.still.is_some() || !self.controlling.is_empty()
    }

    pub fn mic_shown(&self) -> bool {
        !self.mic_apps.is_empty()
    }

    pub fn camera_shown(&self) -> bool {
        !self.camera_apps.is_empty() || !self.camera_holders.is_empty()
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

/// The dot's tooltip: what is shared, and which apps control this computer, a line each.
pub fn sharing_tip(state: &State) -> String {
    let p = &state.privacy;
    let mut lines = Vec::new();
    if p.sharing {
        lines.push(share_label(&p.share_apps, p.screen, p.window));
    } else if let Some((screen, window)) = p.still {
        lines.push(still_label(screen, window));
    }
    if !p.controlling.is_empty() {
        let mut names: Vec<String> = (p.controlling.iter())
            .map(|id| app_name(state, id))
            .collect();
        names.sort();
        names.dedup();
        lines.push(control_label(&names));
    }
    lines.join("\n")
}

fn still_label(screen: bool, window: bool) -> String {
    match (screen, window) {
        (true, true) => "Your screen and a window were captured",
        (false, true) => "A window was captured",
        _ => "Your screen was captured",
    }
    .into()
}

/// Who took a still: its executable, so a shell loop of one-shot screenshot tools counts as one
/// taker, else its pid.
#[derive(Debug, PartialEq, Eq)]
enum Taker {
    Exe(std::path::PathBuf),
    Pid(i32),
}

impl Taker {
    fn of(pid: i32) -> Taker {
        std::fs::read_link(format!("/proc/{pid}/exe")).map_or(Taker::Pid(pid), Taker::Exe)
    }
}

/// `client` was handed a frame of a `window` or a screen; its program's second within `STILL_HOLD`
/// lights the dot. Called after the fill, so the frame that lights the dot never shows it.
pub fn frame_taken(state: &mut State, client: Option<&Client>, window: bool) {
    let Some(pid) = client
        .and_then(|c| c.get_data::<crate::compositor::ClientState>())
        .and_then(|d| d.sandbox.pid)
    else {
        return;
    };
    let taker = Taker::of(pid);
    let now = Instant::now();
    let p = &mut state.privacy;
    p.takers
        .retain(|(_, at)| now.duration_since(*at) < STILL_HOLD);
    let again = p.takers.iter().any(|(t, _)| *t == taker);
    p.takers.retain(|(t, _)| *t != taker);
    p.takers.push((taker, now));
    if !again {
        return;
    }
    let was = p.still;
    let (screen, win) = was.unwrap_or_default();
    p.still = Some((screen || !window, win || window));
    p.still_until = Some(Instant::now() + STILL_HOLD);
    if p.still_timer.is_none() {
        let timer = Timer::from_duration(STILL_HOLD);
        match state
            .handle
            .insert_source(timer, |_, _, state| still_fired(state))
        {
            Ok(t) => state.privacy.still_timer = Some(t),
            Err(e) => log::warn!("privacy: still timer: {e}"),
        }
    }
    if state.privacy.still != was {
        state.redraw_taskbars();
    }
}

fn still_fired(state: &mut State) -> TimeoutAction {
    let p = &mut state.privacy;
    if let Some(until) = p.still_until
        && until > Instant::now()
    {
        return TimeoutAction::ToInstant(until);
    }
    p.still = None;
    p.still_until = None;
    p.still_timer = None;
    state.redraw_taskbars();
    TimeoutAction::Drop
}

/// An app id's name in its desktop entry, else the id; an empty id is "An application".
fn app_name(state: &State, id: &str) -> String {
    if id.is_empty() {
        return "An application".into();
    }
    reprisefs::apps::entry_for_app_id(&state.app_index, id, |_| true)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| id.to_string())
}

fn control_label(apps: &[String]) -> String {
    match apps {
        [] => "An application is controlling this computer".into(),
        [one] => format!("{one} is controlling this computer"),
        [rest @ .., last] => format!(
            "{} and {last} are controlling this computer",
            rest.join(", ")
        ),
    }
}

/// An app's RemoteDesktop sender bound or lost a device, or its keys object came or went:
/// the dot shows while any app holds input.
pub fn control_changed(state: &mut State) {
    let mut apps: Vec<String> = (state.inputproto.eis.app_senders())
        .chain(state.inputproto.keys.apps())
        .map(String::from)
        .collect();
    apps.sort();
    apps.dedup();
    if apps == state.privacy.controlling {
        return;
    }
    log::info!("apps controlling input: {apps:?}");
    state.privacy.controlling = apps;
    state.redraw_taskbars();
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
    let ext = !state.captures.targets(true).is_empty();
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
    client: u32,
    running: bool,
    /// `stream.monitor`: a level meter's peak stream, which records nothing.
    monitor: bool,
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

impl NodeInfo {
    /// A node from its registry global, whose props hold only PipeWire's node global keys.
    fn from_global<'a>(get: impl Fn(&str) -> Option<&'a str>) -> NodeInfo {
        let get = |k: &str| get(k).unwrap_or("").to_string();
        NodeInfo {
            class: get("media.class"),
            name: get("node.name"),
            app: get("application.name"),
            client: get("client.id").parse().unwrap_or(0),
            running: false,
            monitor: false,
        }
    }

    /// Takes in the full props a bound node's info event carries.
    fn note_props<'a>(&mut self, get: impl Fn(&str) -> Option<&'a str>) {
        self.monitor = matches!(get("stream.monitor"), Some("true" | "1"));
    }
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

    /// The apps a Reprise screencast is shared with, by the cast node's `application.name` (only
    /// its owner, the portal, sets it), else the consumer's own. A paused consumer still counts.
    fn sharing(&self) -> Vec<String> {
        let mut apps: Vec<String> = (self.links.values())
            .filter_map(|(out, inp)| Some((self.nodes.get(out)?, self.nodes.get(inp)?)))
            .filter(|(o, i)| o.name == SCREENCAST_NODE && i.class.starts_with("Stream/Input/Video"))
            .map(|(o, i)| match o.app.is_empty() {
                false => o.app.clone(),
                true => self.consumer_name(i),
            })
            .collect();
        apps.sort();
        apps.dedup();
        apps
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
            .filter(|n| n.class.starts_with(class) && (n.running || !running) && !n.monitor)
            .map(|n| self.consumer_name(n))
            .collect();
        apps.sort();
        apps.dedup();
        apps
    }

    /// A consumer by its own `application.name`, from its node, else its client.
    fn consumer_name(&self, n: &NodeInfo) -> String {
        let client = self.clients.get(&n.client).filter(|c| !c.is_empty());
        match (n.app.is_empty(), client) {
            (false, _) => n.app.clone(),
            (true, Some(c)) => c.clone(),
            (true, None) => "An application".to_string(),
        }
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
                        let node = NodeInfo::from_global(|k| props.get(k));
                        let class = node.class.clone();
                        graph.borrow_mut().nodes.insert(obj.id, node);
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
                                    let props = info.props().filter(|_| {
                                        info.change_mask()
                                            .contains(pipewire::node::NodeChangeMask::PROPS)
                                    });
                                    if let Some(n) = graph2.borrow_mut().nodes.get_mut(&id) {
                                        n.running = running;
                                        if let Some(p) = props {
                                            n.note_props(|k| p.get(k));
                                        }
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
            client: 0,
            running,
            monitor: false,
        }
    }

    type Keys = &'static [(&'static str, &'static str)];

    /// pavucontrol's Input Devices meter on a microphone: its node global, then the props its
    /// info event adds (PipeWire 1.6, pipewire-pulse).
    const METER_GLOBAL: Keys = &[
        ("object.serial", "66"),
        ("factory.id", "7"),
        ("client.id", "59"),
        ("client.api", "pipewire-pulse"),
        ("application.name", "PulseAudio Volume Control"),
        ("node.name", "PulseAudio Volume Control"),
        ("media.class", "Stream/Input/Audio"),
    ];
    const METER_INFO: Keys = &[
        ("application.id", "org.PulseAudio.pavucontrol"),
        ("application.process.binary", "pavucontrol"),
        ("media.name", "Peak detect"),
        ("resample.peaks", "true"),
        ("stream.monitor", "true"),
        ("target.object", "55"),
    ];

    /// `pw-record --target testmic`: its node global, then the props its info event adds.
    const RECORD_GLOBAL: Keys = &[
        ("object.serial", "83"),
        ("factory.id", "9"),
        ("client.id", "59"),
        ("application.name", "pw-record"),
        ("node.name", "pw-record"),
        ("media.class", "Stream/Input/Audio"),
    ];
    const RECORD_INFO: Keys = &[
        ("media.category", "Capture"),
        ("media.role", "Music"),
        ("media.name", "/tmp/rec.wav"),
        ("stream.is-live", "true"),
        ("target.object", "testmic"),
    ];

    /// A running stream as the registry and then its info event describe it.
    fn stream(global: Keys, info: Keys) -> NodeInfo {
        let find = |set: Keys| move |k: &str| set.iter().find(|(key, _)| *key == k).map(|p| p.1);
        let mut n = NodeInfo::from_global(find(global));
        n.running = true;
        n.note_props(find(info));
        n
    }

    #[test]
    fn a_level_meter_does_not_light_the_microphone_and_a_recording_does() {
        let mut g = Graph::default();
        g.nodes.insert(55, node("Audio/Source", "", false));
        g.nodes.insert(60, stream(METER_GLOBAL, METER_INFO));
        g.links.insert(100, (55, 60));
        assert_eq!(
            g.recording(),
            Vec::<String>::new(),
            "a level meter records nothing"
        );
        g.nodes.insert(61, stream(RECORD_GLOBAL, RECORD_INFO));
        g.links.insert(101, (55, 61));
        assert_eq!(g.recording(), vec!["pw-record".to_string()]);
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
    fn an_idle_stream_is_skipped() {
        let mut g = Graph::default();
        g.nodes.insert(1, node("Audio/Source/Virtual", "", false));
        g.nodes
            .insert(10, node("Stream/Input/Audio", "Zoom", false));
        g.links.insert(100, (1, 10));
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
    fn a_screencast_is_named_by_the_portals_word_on_its_node() {
        let mut g = Graph::default();
        let mut cast = node("Video/Source", "Chromium", true);
        cast.name = SCREENCAST_NODE.into();
        g.nodes.insert(1, cast);
        g.nodes
            .insert(10, node("Stream/Input/Video", "Volume Control", false));
        g.links.insert(100, (1, 10));
        assert_eq!(g.sharing(), vec!["Chromium".to_string()]);
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
        assert!(!h.state.privacy.sharing_shown(), "no sharing");
        assert!(
            h.state.privacy.dot_shown(),
            "each one shows the dot for a moment"
        );
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
        assert_eq!(sharing_tip(&h.state), "Your screen is being shared");
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

    fn dot(h: &crate::harness::Harness) -> Option<String> {
        use crate::tray::builtin::{Builtin, label, shown};
        (shown(&h.state).contains(&Builtin::Sharing))
            .then(|| label(&h.state, Builtin::Sharing).unwrap())
    }

    #[test]
    fn a_single_screenshot_shows_no_dot() {
        let mut h = capturing();
        let mut c = h.connect_capture(SIZE);
        c.wlr_shot(&mut h);
        assert_eq!(dot(&h), None, "one wlr still");
        h.run_for(STILL_HOLD.mul_f32(1.2).as_millis() as u64);
        let mut s = c.ext_session(&mut h);
        c.ext_frame(&mut h, &mut s);
        c.ext_end(&mut h, s);
        assert_eq!(dot(&h), None, "one ext still, after the last one's hold");
    }

    #[test]
    fn two_runs_of_one_program_are_one_taker() {
        let run = || {
            std::process::Command::new("sleep")
                .arg("5")
                .spawn()
                .unwrap()
        };
        let (mut a, mut b) = (run(), run());
        let taker = |c: &std::process::Child| Taker::of(c.id() as i32);
        assert_eq!(taker(&a), taker(&b), "a shell loop of one-shot tools");
        assert_ne!(taker(&a), Taker::of(std::process::id() as i32));
        for c in [&mut a, &mut b] {
            let _ = c.kill();
            let _ = c.wait();
        }
        assert_eq!(taker(&a), Taker::Pid(a.id() as i32), "a process gone");
    }

    #[test]
    fn screenshots_in_a_loop_show_the_dot_from_the_second() {
        let mut h = capturing();
        h.connect_capture(SIZE).wlr_shot(&mut h);
        assert_eq!(dot(&h), None, "the first still");
        for _ in 0..3 {
            h.run_for(STILL_HOLD.mul_f32(0.6).as_millis() as u64);
            h.connect_capture(SIZE).wlr_shot(&mut h);
            assert_eq!(
                dot(&h).as_deref(),
                Some("Your screen was captured"),
                "each still after the first, from one process"
            );
        }
        assert!(
            !h.state.privacy.sharing_shown(),
            "balloons don't wait for it"
        );
        let last = std::time::Instant::now();
        let left =
            (last + STILL_HOLD.mul_f32(1.3)).saturating_duration_since(std::time::Instant::now());
        h.run_for(left.as_millis() as u64);
        assert_eq!(dot(&h), None, "the dot goes once they stop");
    }

    #[test]
    fn an_apps_sender_shows_the_dot_naming_the_app_while_it_has_a_device() {
        use crate::inputproto::DEVICE_KEYBOARD;
        let mut h = capturing();
        let peer = h.ei_keyboard_as(
            crate::eis::Kind::Sender,
            false,
            DEVICE_KEYBOARD,
            "org.example.Remote",
        );
        assert_eq!(
            dot(&h).as_deref(),
            Some("org.example.Remote is controlling this computer")
        );
        assert!(
            !h.state.privacy.sharing_shown(),
            "balloons don't wait for it"
        );
        h.ei_close(peer);
        assert_eq!(dot(&h), None, "the dot goes with the sender");
    }

    #[test]
    fn an_apps_keys_object_shows_the_dot_while_it_lives() {
        let mut h = capturing();
        let keys = h.state.keys_create(Some(String::new()));
        assert_eq!(
            dot(&h).as_deref(),
            Some("An application is controlling this computer")
        );
        h.state.keys_destroy(keys);
        assert_eq!(dot(&h), None);
    }

    #[test]
    fn the_remote_desktop_servers_input_shows_no_dot() {
        let mut h = capturing();
        let _peer = h.ei_remote_session();
        let _keys = h.state.keys_create(None);
        assert_eq!(dot(&h), None);
    }

    #[test]
    fn only_an_older_portals_create_keys_counts_as_an_apps() {
        let mut h = capturing();
        let mut portal = h.connect_portal();
        let server = portal.create_keys(10);
        h.roundtrip(&mut portal);
        assert_eq!(dot(&h), None, "a v10 create_keys is the server's");
        let old = portal.create_keys(9);
        h.roundtrip(&mut portal);
        assert_eq!(
            dot(&h).as_deref(),
            Some("An application is controlling this computer")
        );
        let app = portal.create_app_keys("org.example.Remote");
        h.roundtrip(&mut portal);
        assert_eq!(
            dot(&h).as_deref(),
            Some("An application and org.example.Remote are controlling this computer")
        );
        for k in [server, old, app] {
            k.destroy();
        }
        h.roundtrip(&mut portal);
        assert_eq!(dot(&h), None);
    }

    #[test]
    fn a_shared_screen_and_a_controlling_app_share_the_tooltip() {
        let mut h = capturing();
        let mut c = h.connect_capture(SIZE);
        let mut s = c.ext_session(&mut h);
        c.ext_frame(&mut h, &mut s);
        c.ext_frame(&mut h, &mut s);
        let _keys = h.state.keys_create(Some("org.example.Remote".into()));
        assert_eq!(
            dot(&h).as_deref(),
            Some("Your screen is being shared\norg.example.Remote is controlling this computer")
        );
    }

    #[test]
    fn screenshots_every_few_seconds_after_a_cast_do_not_hold_balloons() {
        let mut h = capturing();
        let ended = cast_with_held_balloon(&mut h);
        for k in 1..=5 {
            run_until(&mut h, ended, 0.4 * k as f32);
            screenshot(&mut h);
        }
        assert!(!h.state.privacy.sharing_shown(), "no sharing");
        assert!(crate::notifyview::balloon_up(&h.state), "the balloon shows");
    }
}
