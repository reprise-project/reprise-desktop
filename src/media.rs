//! Media key backends: volume and mute through `wpctl`, backlights through logind's `SetBrightness`, external monitors over DDC/CI through `ddcutil`, transport through MPRIS, the wireless radio through NetworkManager. One worker thread runs the jobs and reports over a calloop channel so the event loop never blocks; display jobs run on a lane thread per backlight or DDC bus, so a monitor that stops answering holds up only itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use smithay::reexports::calloop::{
    RegistrationToken, channel,
    timer::{TimeoutAction, Timer},
};
use zbus::{Connection, Proxy};

use crate::compositor::State;
use crate::config::Action;

const STEP_PCT: i32 = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    VolumeStep(i32),
    Mute,
    MicMute,
    Kbd(i32),
    Player(&'static str),
    Wifi,
    Display(DisplayJob),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    Volume { level: f64, muted: bool },
    Mic { muted: bool },
    Kbd { level: f64 },
    Player { name: String, verb: &'static str },
    Wifi { on: bool },
    Display(DisplayReport),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    Brightness,
}

impl Value {
    pub const ALL: [Value; 1] = [Value::Brightness];

    pub fn title(self) -> &'static str {
        match self {
            Value::Brightness => "Brightness",
        }
    }
}

/// A display value in percent, and the lowest percent a step may reach (1 for a backlight, 0 over DDC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub level: u8,
    pub floor: u8,
}

/// A job on one display, named by connector; `seq` comes back in its report so replies apply in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayJob {
    Read {
        display: String,
        value: Value,
        seq: u64,
    },
    Set {
        display: String,
        value: Value,
        level: u8,
        seq: u64,
    },
}

impl DisplayJob {
    pub fn display(&self) -> &str {
        match self {
            DisplayJob::Read { display, .. } | DisplayJob::Set { display, .. } => display,
        }
    }

    pub fn seq(&self) -> u64 {
        match self {
            DisplayJob::Read { seq, .. } | DisplayJob::Set { seq, .. } => *seq,
        }
    }

    fn value(&self) -> Value {
        match self {
            DisplayJob::Read { value, .. } | DisplayJob::Set { value, .. } => *value,
        }
    }
}

/// `Failed` carries the level read back after a failed write, if the display still answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Read(Option<Reading>),
    Wrote,
    Failed(Option<Reading>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayReport {
    pub display: String,
    pub value: Value,
    pub seq: u64,
    pub outcome: Outcome,
}

/// Keep only the latest read and the latest set of each display and value, in queue order: a write lands on the last level asked for.
pub fn fold_display(jobs: Vec<DisplayJob>) -> Vec<DisplayJob> {
    let key = |j: &DisplayJob| {
        (
            j.display().to_owned(),
            j.value(),
            matches!(j, DisplayJob::Set { .. }),
        )
    };
    let keys: Vec<_> = jobs.iter().map(key).collect();
    jobs.into_iter()
        .enumerate()
        .filter(|(i, _)| !keys[i + 1..].contains(&keys[*i]))
        .map(|(_, j)| j)
        .collect()
}

/// The report for a display that did not answer in time: no reading, and a set that failed.
fn not_answering(job: &DisplayJob) -> DisplayReport {
    let (outcome, value) = match job {
        DisplayJob::Read { value, .. } => (Outcome::Read(None), *value),
        DisplayJob::Set { value, .. } => (Outcome::Failed(None), *value),
    };
    DisplayReport {
        display: job.display().to_owned(),
        value,
        seq: job.seq(),
        outcome,
    }
}

/// Fold a burst of jobs into the fewest with the same end state: consecutive volume and keyboard backlight steps add up, anything else keeps its order. A burst that cancels out folds to a zero step, not nothing, so the OSD shows the level it ends on.
pub fn coalesce(jobs: Vec<Job>) -> Vec<Job> {
    let mut out: Vec<Job> = Vec::new();
    for j in jobs {
        match (out.last_mut(), j) {
            (Some(Job::VolumeStep(a)), Job::VolumeStep(b)) => *a += b,
            (Some(Job::Kbd(a)), Job::Kbd(b)) => *a += b,
            (_, j) => out.push(j),
        }
    }
    out
}

pub fn parse_wpctl_volume(out: &str) -> Option<(f64, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let mut parts = rest.split_whitespace();
    let level: f64 = parts.next()?.parse().ok()?;
    let muted = parts.any(|p| p == "[MUTED]");
    Some((level.clamp(0.0, 1.0), muted))
}

/// `cur` stepped by STEP_PCT of `max` (at least 1 unit), clamped to `floor..=max`; a down press never raises a level already under `floor`.
pub fn next_level(cur: u32, max: u32, steps: i32, floor: u32) -> u32 {
    let unit = (max as i64 * STEP_PCT as i64 / 100).max(1);
    let low = if steps < 0 { floor.min(cur) } else { floor } as i64;
    (cur as i64 + unit * steps as i64).clamp(low, max as i64) as u32
}

/// `raw` of `max` in whole percent, rounded.
pub fn to_percent(raw: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    ((raw.min(max) as u64 * 100 + max as u64 / 2) / max as u64) as u8
}

/// `pct` of `max` in device units, rounded, kept within `floor..=max`.
pub fn from_percent(pct: u8, max: u32, floor: u32) -> u32 {
    let v = ((pct.min(100) as u64 * max as u64 + 50) / 100) as u32;
    v.max(floor).min(max)
}

/// `cur` percent stepped by STEP_PCT a step, by `next_level`'s rule.
pub fn step_percent(cur: u8, steps: i32, floor: u8) -> u8 {
    next_level(cur as u32, 100, steps, floor as u32) as u8
}

/// The panel backlight's floor, so brightness-down never blacks it out: 1% of `max`, at least one unit, never above `max`.
pub fn backlight_floor(max: u32) -> u32 {
    (max / 100).max(1).min(max)
}

/// A backlight's rank from its `type`: `raw` beats `platform` beats ACPI's `firmware` stub. A device with no `type` file (every keyboard backlight under `leds`) ranks last.
fn device_rank(class_dir: &std::path::Path, name: &str) -> u8 {
    match std::fs::read_to_string(class_dir.join(name).join("type"))
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("raw") => 0,
        Some("platform") => 1,
        Some("firmware") => 2,
        _ => 3,
    }
}

fn pick_device(class_dir: &std::path::Path, pick: impl Fn(&str) -> bool) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(class_dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| pick(n))
        .collect();
    names.sort_by_cached_key(|n| (device_rank(class_dir, n), n.clone()));
    names.into_iter().next()
}

fn sysfs_device(class: &str, pick: impl Fn(&str) -> bool) -> Option<(String, u32, u32)> {
    let dir = Path::new("/sys/class").join(class);
    let name = pick_device(&dir, pick)?;
    let (cur, max) = sysfs_levels(&dir, &name)?;
    Some((name, cur, max))
}

fn sysfs_levels(class_dir: &Path, name: &str) -> Option<(u32, u32)> {
    let read = |f: &str| -> Option<u32> {
        std::fs::read_to_string(class_dir.join(name).join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("brightness")?, read("max_brightness")?))
}

/// A root standing in for `/sys` for the display controls, for testing without a backlight or DDC monitor.
pub const SYSFS_ROOT_VAR: &str = "REPRISE_SYSFS_ROOT";

/// Where display jobs find their devices. Under a test root, backlight writes go to the `brightness` file instead of through logind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sysfs {
    root: PathBuf,
    direct: bool,
}

impl Sysfs {
    pub fn from_var(v: Option<std::ffi::OsString>) -> Sysfs {
        match v {
            Some(root) => Sysfs {
                root: root.into(),
                direct: true,
            },
            None => Sysfs {
                root: "/sys".into(),
                direct: false,
            },
        }
    }

    fn class(&self, c: &str) -> PathBuf {
        self.root.join("class").join(c)
    }

    /// The device behind `display`: its own backlight or DDC bus, else for a laptop panel the best backlight.
    fn device(&self, display: &str) -> Option<Panel> {
        panel_for(&self.class("drm"), display).or_else(|| {
            is_internal(display)
                .then(|| pick_device(&self.class("backlight"), |_| true))
                .flatten()
                .map(Panel::Backlight)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Panel {
    Backlight(String),
    Ddc(u32),
}

/// The panel behind `output`, from its connector `card*-<output>` under `drm`: a backlight device inside the connector (a laptop), else the connector's I2C bus. DisplayPort carries DDC/CI over its AUX channel (an `i2c-N` child); HDMI and DVI on the bus the `ddc` link names.
fn panel_for(drm: &std::path::Path, output: &str) -> Option<Panel> {
    let conn = std::fs::read_dir(drm)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("card"))
                .and_then(|n| n.split_once('-'))
                .is_some_and(|(card, name)| {
                    name == output && !card.is_empty() && card.bytes().all(|b| b.is_ascii_digit())
                })
        })?;
    let children: Vec<String> = std::fs::read_dir(&conn)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    if let Some(bl) = children
        .iter()
        .find(|n| conn.join(n).join("max_brightness").is_file())
    {
        return Some(Panel::Backlight(bl.clone()));
    }
    let bus = |n: &str| n.strip_prefix("i2c-")?.parse().ok();
    children
        .iter()
        .find_map(|n| bus(n))
        .or_else(|| {
            bus(std::fs::read_link(conn.join("ddc"))
                .ok()?
                .file_name()?
                .to_str()?)
        })
        .map(Panel::Ddc)
}

/// A laptop's own panel, whose backlight can sit outside its connector (ACPI's `acpi_video0`).
pub(crate) fn is_internal(output: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| output.starts_with(p))
}

const VCP_BRIGHTNESS: &str = "10";

pub fn parse_getvcp(out: &str) -> Option<(u32, u32)> {
    let mut parts = out.split_whitespace();
    if parts.next()? != "VCP" {
        return None;
    }
    parts.next()?;
    if parts.next()? != "C" {
        return None;
    }
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// The worker's bus handles, opened on first use and dropped by the job that fails on them, so a bus restart costs one lost press.
struct Buses {
    system: Option<Connection>,
    session: Option<Connection>,
}

impl Buses {
    async fn system(&mut self) -> Option<&Connection> {
        if self.system.is_none() {
            self.system = Connection::system().await.ok();
        }
        self.system.as_ref()
    }
    async fn session(&mut self) -> Option<&Connection> {
        if self.session.is_none() {
            self.session = Connection::session().await.ok();
        }
        self.session.as_ref()
    }
}

async fn wpctl(args: &[&str]) -> Option<String> {
    command("wpctl", args).await
}

/// Run `program` and take its stdout. Async so the job's timeout can fire while the child runs; `kill_on_drop` kills the child when it loses that race.
async fn command(program: &str, args: &[&str]) -> Option<String> {
    let out = match async_process::Command::new(program)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => {
            log::warn!("media: {program} {} did not run: {e}", args.join(" "));
            return None;
        }
    };
    if !out.status.success() {
        log::warn!("media: {program} {} failed", args.join(" "));
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn set_brightness(buses: &mut Buses, subsystem: &str, name: &str, v: u32) -> bool {
    let Some(conn) = buses.system().await else {
        return false;
    };
    let ok = match Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .await
    {
        Ok(session) => match session
            .call_method("SetBrightness", &(subsystem, name, v))
            .await
        {
            Ok(_) => true,
            Err(e) => {
                log::warn!("media: SetBrightness({subsystem}, {name}) failed: {e}");
                false
            }
        },
        Err(_) => false,
    };
    if !ok {
        buses.system = None;
    }
    ok
}

/// One device's display jobs: the DDC maximum from the last read (writes need it), and the program DDC jobs run with its leading arguments.
struct Lane {
    ddc_max: Option<u32>,
    ddcutil: Vec<String>,
}

impl Lane {
    fn new() -> Lane {
        Lane {
            ddc_max: None,
            ddcutil: vec!["ddcutil".into()],
        }
    }
}

fn backlight_read(sysfs: &Sysfs, name: &str) -> Option<Reading> {
    let (cur, max) = sysfs_levels(&sysfs.class("backlight"), name)?;
    Some(Reading {
        level: to_percent(cur, max),
        floor: 1,
    })
}

async fn backlight_write(buses: &mut Buses, sysfs: &Sysfs, name: &str, level: u8) -> bool {
    let Some((_, max)) = sysfs_levels(&sysfs.class("backlight"), name) else {
        return false;
    };
    let v = from_percent(level, max, backlight_floor(max));
    if sysfs.direct {
        let path = sysfs.class("backlight").join(name).join("brightness");
        std::fs::write(path, v.to_string()).is_ok()
    } else {
        set_brightness(buses, "backlight", name, v).await
    }
}

async fn ddcutil(lane: &Lane, args: &[&str]) -> Option<String> {
    let (program, lead) = lane.ddcutil.split_first()?;
    let args: Vec<&str> = lead
        .iter()
        .map(String::as_str)
        .chain(args.iter().copied())
        .collect();
    command(program, &args).await
}

/// Read over DDC/CI, keeping the monitor's maximum for the next write. DDC calls skip ddcutil's start-up checks and writes skip its read-back: on some monitors those cost seconds, which would outrun JOB_TIMEOUT.
async fn ddc_read(lane: &mut Lane, bus: u32) -> Option<Reading> {
    let bus = bus.to_string();
    let out = ddcutil(
        lane,
        &[
            "--bus",
            &bus,
            "--skip-ddc-checks",
            "getvcp",
            VCP_BRIGHTNESS,
            "--brief",
        ],
    )
    .await;
    let read = out.as_deref().and_then(parse_getvcp);
    lane.ddc_max = read.map(|(_, max)| max);
    let (cur, max) = read?;
    Some(Reading {
        level: to_percent(cur, max),
        floor: 0,
    })
}

async fn ddc_write(lane: &mut Lane, bus: u32, level: u8) -> bool {
    let max = match lane.ddc_max {
        Some(max) => max,
        None => match ddc_read(lane, bus).await.and(lane.ddc_max) {
            Some(max) => max,
            None => return false,
        },
    };
    let (bus, v) = (bus.to_string(), from_percent(level, max, 0).to_string());
    ddcutil(
        lane,
        &[
            "--bus",
            &bus,
            "--skip-ddc-checks",
            "--noverify",
            "setvcp",
            VCP_BRIGHTNESS,
            &v,
        ],
    )
    .await
    .is_some()
}

async fn perform_display(
    buses: &mut Buses,
    sysfs: &Sysfs,
    lane: &mut Lane,
    job: DisplayJob,
) -> DisplayReport {
    let Some(panel) = sysfs.device(job.display()) else {
        return not_answering(&job);
    };
    let outcome = match (&panel, &job) {
        (Panel::Backlight(name), DisplayJob::Read { .. }) => {
            Outcome::Read(backlight_read(sysfs, name))
        }
        (Panel::Backlight(name), DisplayJob::Set { level, .. }) => {
            if backlight_write(buses, sysfs, name, *level).await {
                Outcome::Wrote
            } else {
                Outcome::Failed(backlight_read(sysfs, name))
            }
        }
        (Panel::Ddc(bus), DisplayJob::Read { .. }) => Outcome::Read(ddc_read(lane, *bus).await),
        (Panel::Ddc(bus), DisplayJob::Set { level, .. }) => {
            if ddc_write(lane, *bus, *level).await {
                Outcome::Wrote
            } else {
                Outcome::Failed(ddc_read(lane, *bus).await)
            }
        }
    };
    DisplayReport {
        display: job.display().to_owned(),
        value: job.value(),
        seq: job.seq(),
        outcome,
    }
}

const PLAYER_NAME_MAX: usize = 64;

/// A player's name for the OSD: any session-bus peer sets `Identity`, so it is cut to PLAYER_NAME_MAX chars.
fn player_name(mut identity: String) -> String {
    if let Some((end, _)) = identity.char_indices().nth(PLAYER_NAME_MAX) {
        identity.truncate(end);
    }
    identity
}

async fn pick_player(conn: &Connection) -> Option<(String, String)> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let names: Vec<String> = dbus
        .list_names()
        .await
        .ok()?
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    let mut first = None;
    for name in names {
        let Ok(player) = Proxy::new(
            conn,
            name.as_str(),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        )
        .await
        else {
            continue;
        };
        let playing = player
            .get_property::<String>("PlaybackStatus")
            .await
            .is_ok_and(|s| s == "Playing");
        if playing || first.is_none() {
            let identity = match Proxy::new(
                conn,
                name.as_str(),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2",
            )
            .await
            {
                // The name is cosmetic: a player that answers neither the proxy nor the property still gets its key press.
                Ok(root) => root
                    .get_property::<String>("Identity")
                    .await
                    .unwrap_or_else(|_| name.clone()),
                Err(_) => name.clone(),
            };
            first = Some((name, player_name(identity)));
            if playing {
                break;
            }
        }
    }
    first
}

async fn perform(buses: &mut Buses, job: Job) -> Option<Report> {
    match job {
        Job::VolumeStep(n) => {
            let arg = format!("{}%{}", (n.abs() * STEP_PCT), if n < 0 { "-" } else { "+" });
            wpctl(&["set-volume", "@DEFAULT_AUDIO_SINK@", &arg, "-l", "1.0"]).await?;
            let (level, muted) =
                parse_wpctl_volume(&wpctl(&["get-volume", "@DEFAULT_AUDIO_SINK@"]).await?)?;
            Some(Report::Volume { level, muted })
        }
        Job::Mute => {
            wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]).await?;
            let (level, muted) =
                parse_wpctl_volume(&wpctl(&["get-volume", "@DEFAULT_AUDIO_SINK@"]).await?)?;
            Some(Report::Volume { level, muted })
        }
        Job::MicMute => {
            wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]).await?;
            let (_, muted) =
                parse_wpctl_volume(&wpctl(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).await?)?;
            Some(Report::Mic { muted })
        }
        Job::Kbd(n) => {
            let (name, cur, max) = sysfs_device("leds", |n| n.ends_with("::kbd_backlight"))?;
            // No floor: a keyboard backlight is meant to go all the way off.
            let v = next_level(cur, max, n, 0);
            set_brightness(buses, "leds", &name, v)
                .await
                .then_some(())?;
            Some(Report::Kbd {
                level: v as f64 / max.max(1) as f64,
            })
        }
        Job::Player(method) => {
            let r = player(buses.session().await?, method).await;
            if r.is_none() {
                buses.session = None;
            }
            r
        }
        Job::Wifi => {
            let r = wifi(buses.system().await?).await;
            if r.is_none() {
                buses.system = None;
            }
            r
        }
        // `worker` hands these to their lanes.
        Job::Display(_) => None,
    }
}

async fn player(conn: &Connection, method: &'static str) -> Option<Report> {
    let (bus, name) = pick_player(conn).await?;
    let player = Proxy::new(
        conn,
        bus.as_str(),
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .ok()?;
    player.call_method(method, &()).await.ok()?;
    Some(Report::Player { name, verb: method })
}

async fn wifi(conn: &Connection) -> Option<Report> {
    let nm = Proxy::new(
        conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    .ok()?;
    let on = nm.get_property::<bool>("WirelessEnabled").await.ok()?;
    nm.set_property("WirelessEnabled", !on).await.ok()?;
    Some(Report::Wifi { on: !on })
}

/// How long a job gets before the worker gives up: a wedged bus call or a `wpctl` that never exits would stall every later press.
const JOB_TIMEOUT: Duration = Duration::from_secs(2);

/// Run `fut` on this thread for at most `timeout`, catching a panic so the thread carries on; None, logged against `job`, when it ran out of time or panicked.
fn guarded<T>(
    job: &dyn std::fmt::Debug,
    timeout: Duration,
    fut: impl Future<Output = T>,
) -> Option<T> {
    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        zbus::block_on(futures_lite::future::or(async { Some(fut.await) }, async {
            async_io::Timer::after(timeout).await;
            None
        }))
    }));
    match done {
        Ok(Some(v)) => Some(v),
        Ok(None) => {
            log::warn!("media: {job:?} timed out after {timeout:?}");
            None
        }
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".into());
            log::warn!("media: {job:?} panicked ({msg}); carrying on");
            None
        }
    }
}

fn no_buses() -> Buses {
    Buses {
        system: None,
        session: None,
    }
}

fn worker(jobs: Receiver<Job>, reports: channel::Sender<Report>, sysfs: Sysfs) {
    let mut buses = no_buses();
    let mut lanes: HashMap<Panel, Sender<DisplayJob>> = HashMap::new();
    while let Ok(first) = jobs.recv() {
        let mut burst = vec![first];
        burst.extend(jobs.try_iter());
        for job in coalesce(burst) {
            if let Job::Display(j) = job {
                to_lane(&mut lanes, &sysfs, &reports, j);
                continue;
            }
            match guarded(&job, JOB_TIMEOUT, perform(&mut buses, job.clone())) {
                Some(Some(r)) => {
                    let _ = reports.send(r);
                }
                Some(None) => {}
                // A wedged socket is the usual cause; reconnect next time.
                None => buses = no_buses(),
            }
        }
    }
}

/// Queue `job` on its device's lane, starting the lane on first use; a display with no device, or a lane that cannot start, reports not answering.
fn to_lane(
    lanes: &mut HashMap<Panel, Sender<DisplayJob>>,
    sysfs: &Sysfs,
    reports: &channel::Sender<Report>,
    job: DisplayJob,
) {
    let Some(panel) = sysfs.device(job.display()) else {
        let _ = reports.send(Report::Display(not_answering(&job)));
        return;
    };
    if !lanes.contains_key(&panel) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (lane_sysfs, lane_reports) = (sysfs.clone(), reports.clone());
        let spawned = std::thread::Builder::new()
            .name("media-lane".into())
            .spawn(move || {
                run_lane(rx, &lane_sysfs, Lane::new(), JOB_TIMEOUT, |r| {
                    let _ = lane_reports.send(Report::Display(r));
                })
            });
        if spawned.is_err() {
            log::warn!("media: no lane thread for {panel:?}");
            let _ = reports.send(Report::Display(not_answering(&job)));
            return;
        }
        lanes.insert(panel.clone(), tx);
    }
    if let Err(e) = lanes[&panel].send(job) {
        lanes.remove(&panel);
        let _ = reports.send(Report::Display(not_answering(&e.0)));
    }
}

/// A lane's loop: drain the queue, fold it, run each job under `timeout`; a job that times out or panics reports not answering.
fn run_lane(
    jobs: Receiver<DisplayJob>,
    sysfs: &Sysfs,
    mut lane: Lane,
    timeout: Duration,
    report: impl Fn(DisplayReport),
) {
    let mut buses = no_buses();
    while let Ok(first) = jobs.recv() {
        let mut queue = vec![first];
        queue.extend(jobs.try_iter());
        for job in fold_display(queue) {
            let done = guarded(
                &job,
                timeout,
                perform_display(&mut buses, sysfs, &mut lane, job.clone()),
            );
            match done {
                Some(r) => report(r),
                None => {
                    buses = no_buses();
                    report(not_answering(&job));
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct MediaState {
    jobs: Option<Sender<Job>>,
    repeat: Option<RegistrationToken>,
    repeating: Option<(u32, Action)>,
}

/// Whether holding the key keeps stepping: level actions do, the toggles fire once per press.
pub fn repeats(a: &Action) -> bool {
    matches!(
        a,
        Action::VolumeUp
            | Action::VolumeDown
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::KbdBrightnessUp
            | Action::KbdBrightnessDown
    )
}

impl State {
    pub fn media_setup(&mut self) {
        let (jtx, jrx) = std::sync::mpsc::channel::<Job>();
        let (rtx, rrx) = channel::channel::<Report>();
        let added = self
            .handle
            .insert_source(rrx, |event, _, state| match event {
                channel::Event::Msg(Report::Display(d)) => crate::displays::report(state, d),
                channel::Event::Msg(r) => {
                    log::debug!("media: {r:?}");
                    state.show_osd(&r);
                }
                channel::Event::Closed => {}
            });
        if let Err(e) = added {
            log::warn!("media: no report source ({}); media keys are off", e.error);
            return;
        }
        let sysfs = Sysfs::from_var(std::env::var_os(SYSFS_ROOT_VAR));
        if sysfs.root != Path::new("/sys") {
            log::info!("media: display controls use {}", sysfs.root.display());
        }
        let spawned = std::thread::Builder::new()
            .name("media".into())
            .spawn(move || worker(jrx, rtx, sysfs));
        if spawned.is_err() {
            log::warn!("media: no worker thread; media keys are off");
            return;
        }
        self.key_input.media.jobs = Some(jtx);
        crate::displays::reread(self);
    }

    /// A media key went down: run its action, and if it steps, repeat it at the keyboard's repeat delay and rate until `media_key_up` or another press. Firmware repeats some keys but not others, so the compositor repeats all of them.
    pub fn media_key_down(&mut self, a: &Action, keycode: u32) {
        self.run_media(a);
        self.stop_media_repeat();
        if !repeats(a) {
            return;
        }
        let kb = &self.config.keyboard;
        if kb.repeat_rate <= 0 {
            return;
        }
        let delay = kb.repeat_delay.max(1) as u64;
        self.key_input.media.repeating = Some((keycode, a.clone()));
        self.arm_media_repeat(delay);
    }

    pub fn media_key_up(&mut self, keycode: u32) {
        if (self.key_input.media.repeating.as_ref()).is_some_and(|(k, _)| *k == keycode) {
            self.stop_media_repeat();
        }
    }

    pub fn stop_media_repeat(&mut self) {
        self.key_input.media.repeating = None;
        if let Some(t) = self.key_input.media.repeat.take() {
            self.handle.remove(t);
        }
    }

    fn arm_media_repeat(&mut self, ms: u64) {
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            |_, _, state| {
                let Some((_, a)) = state.key_input.media.repeating.clone() else {
                    state.key_input.media.repeat = None;
                    return TimeoutAction::Drop;
                };
                state.run_media(&a);
                let rate = state.config.keyboard.repeat_rate.max(1) as u64;
                TimeoutAction::ToDuration(Duration::from_millis((1000 / rate).max(1)))
            },
        );
        match token {
            Ok(t) => self.key_input.media.repeat = Some(t),
            Err(e) => log::warn!("media: repeat timer: {}", e.error),
        }
    }

    pub fn run_media(&mut self, a: &Action) {
        let job = match a {
            Action::VolumeUp => Job::VolumeStep(1),
            Action::VolumeDown => Job::VolumeStep(-1),
            Action::Mute => Job::Mute,
            Action::MicMute => Job::MicMute,
            Action::BrightnessUp | Action::BrightnessDown => {
                let steps = if *a == Action::BrightnessUp { 1 } else { -1 };
                crate::displays::step(self, None, Value::Brightness, steps);
                self.show_display_osd(Value::Brightness);
                return;
            }
            Action::KbdBrightnessUp => Job::Kbd(1),
            Action::KbdBrightnessDown => Job::Kbd(-1),
            Action::PlayPause => Job::Player("PlayPause"),
            Action::Next => Job::Player("Next"),
            Action::Prev => Job::Player("Previous"),
            Action::Stop => Job::Player("Stop"),
            Action::Wifi => Job::Wifi,
            _ => return,
        };
        self.send_job(job);
    }

    pub(crate) fn send_job(&self, job: Job) {
        if let Some(tx) = self.key_input.media.jobs.as_ref()
            && let Err(e) = tx.send(job)
        {
            log::warn!("media: the worker is gone; {:?} does nothing", e.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_volume_key_folds_into_one_step() {
        let jobs = vec![
            Job::VolumeStep(1),
            Job::VolumeStep(1),
            Job::VolumeStep(-1),
            Job::Mute,
            Job::VolumeStep(1),
        ];
        assert_eq!(
            coalesce(jobs),
            vec![Job::VolumeStep(1), Job::Mute, Job::VolumeStep(1)]
        );
        // A burst that cancels out still runs, so the OSD shows.
        assert_eq!(
            coalesce(vec![Job::VolumeStep(1), Job::VolumeStep(-1)]),
            vec![Job::VolumeStep(0)]
        );
    }

    #[test]
    fn getvcp_reads_the_current_and_max_brightness() {
        assert_eq!(parse_getvcp("VCP 10 C 30 100\n"), Some((30, 100)));
        assert_eq!(parse_getvcp("VCP 10 ERR\n"), None);
        assert_eq!(parse_getvcp("VCP 14 SNC x05\n"), None);
        assert_eq!(parse_getvcp(""), None);
    }

    #[test]
    fn an_output_resolves_to_its_backlight_or_its_ddc_bus() {
        let d = TempDir::new("drm");
        let mk = |p: &str| std::fs::create_dir_all(d.0.join(p)).unwrap();
        mk("card1-eDP-1/amdgpu_bl1");
        mk("card1-eDP-1/i2c-6");
        std::fs::write(d.0.join("card1-eDP-1/amdgpu_bl1/max_brightness"), "65535\n").unwrap();
        mk("card1-DP-2/i2c-8");
        std::os::unix::fs::symlink("../../../i2c-5", d.0.join("card1-DP-2/ddc")).unwrap();
        mk("card1-HDMI-A-1");
        std::os::unix::fs::symlink("../../../i2c-3", d.0.join("card1-HDMI-A-1/ddc")).unwrap();
        mk("card1");

        assert_eq!(
            panel_for(&d.0, "eDP-1"),
            Some(Panel::Backlight("amdgpu_bl1".into()))
        );
        assert_eq!(panel_for(&d.0, "DP-2"), Some(Panel::Ddc(8)));
        assert_eq!(panel_for(&d.0, "HDMI-A-1"), Some(Panel::Ddc(3)));
        assert_eq!(panel_for(&d.0, "DP-1"), None);
        assert!(is_internal("eDP-1") && !is_internal("DP-2"));
    }

    #[test]
    fn level_keys_repeat_and_toggles_do_not() {
        assert!(repeats(&Action::VolumeUp));
        assert!(repeats(&Action::BrightnessDown));
        assert!(repeats(&Action::KbdBrightnessUp));
        assert!(!repeats(&Action::Mute));
        assert!(!repeats(&Action::PlayPause));
        assert!(!repeats(&Action::Wifi));
        assert!(!repeats(&Action::Lock));
    }

    #[test]
    fn a_player_name_is_cut_to_64_chars() {
        assert_eq!(player_name("mpv".into()), "mpv");
        assert_eq!(player_name("\u{e9}".repeat(100)), "\u{e9}".repeat(64));
    }

    #[test]
    fn wpctl_volume_lines_parse() {
        assert_eq!(parse_wpctl_volume("Volume: 0.45\n"), Some((0.45, false)));
        assert_eq!(
            parse_wpctl_volume("Volume: 1.00 [MUTED]"),
            Some((1.0, true))
        );
        assert_eq!(parse_wpctl_volume("Volume: 1.20"), Some((1.0, false)));
        assert_eq!(parse_wpctl_volume("garbage"), None);
    }

    #[test]
    fn backlight_steps_are_a_twentieth_of_max_and_at_least_one() {
        let f = backlight_floor;
        assert_eq!(next_level(100, 1000, 1, f(1000)), 150);
        assert_eq!(next_level(10, 10, 1, f(10)), 10);
        assert_eq!(next_level(5, 10, 1, f(10)), 6);
        // The monitor's floor is 1% of max, at least one unit: down from the floor stays there.
        assert_eq!(next_level(1, 10, -1, f(10)), 1);
        assert_eq!(next_level(1000, 100_000, -1, f(100_000)), 1000);
        assert_eq!(next_level(5000, 100_000, -1, f(100_000)), 1000);
        // A level already under the floor is not raised by a down press.
        assert_eq!(next_level(0, 100_000, -1, f(100_000)), 0);
        assert_eq!(next_level(0, 10, -1, f(10)), 0);
        assert_eq!(next_level(1, 10, -1, 0), 0);
        assert_eq!(next_level(0, 10, -1, 0), 0);
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "reprise-media-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn device(&self, name: &str, kind: Option<&str>) {
            let d = self.0.join(name);
            std::fs::create_dir_all(&d).unwrap();
            if let Some(k) = kind {
                std::fs::write(d.join("type"), format!("{k}\n")).unwrap();
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_panels_own_backlight_beats_the_acpi_stub() {
        let d = TempDir::new("rank");
        d.device("acpi_video0", Some("firmware"));
        d.device("intel_backlight", Some("raw"));
        d.device("dell_backlight", Some("platform"));
        assert_eq!(
            pick_device(&d.0, |_| true).as_deref(),
            Some("intel_backlight")
        );

        let d = TempDir::new("rank-platform");
        d.device("acpi_video0", Some("firmware"));
        d.device("dell_backlight", Some("platform"));
        assert_eq!(
            pick_device(&d.0, |_| true).as_deref(),
            Some("dell_backlight")
        );

        let d = TempDir::new("rank-tie");
        d.device("b_backlight", Some("raw"));
        d.device("a_backlight", Some("raw"));
        assert_eq!(pick_device(&d.0, |_| true).as_deref(), Some("a_backlight"));

        let d = TempDir::new("rank-leds");
        d.device("acpi_video0", Some("firmware"));
        d.device("tpacpi::kbd_backlight", None);
        assert_eq!(pick_device(&d.0, |_| true).as_deref(), Some("acpi_video0"));
        assert_eq!(
            pick_device(&d.0, |n| n.ends_with("::kbd_backlight")).as_deref(),
            Some("tpacpi::kbd_backlight")
        );
        assert_eq!(pick_device(&d.0, |n| n == "nothing"), None);
    }

    #[test]
    fn percent_levels_convert_both_ways_and_keep_the_floor() {
        assert_eq!(to_percent(32768, 65535), 50);
        assert_eq!(to_percent(100, 100), 100);
        assert_eq!(to_percent(5, 0), 0);
        assert_eq!(from_percent(50, 65535, 655), 32768);
        assert_eq!(from_percent(0, 65535, backlight_floor(65535)), 655);
        assert_eq!(from_percent(0, 100, 0), 0);
        assert_eq!(from_percent(100, 255, 0), 255);
    }

    #[test]
    fn a_percent_step_is_five_and_holds_the_floor() {
        assert_eq!(step_percent(50, 1, 1), 55);
        assert_eq!(step_percent(98, 1, 1), 100);
        assert_eq!(step_percent(3, -1, 1), 1);
        assert_eq!(step_percent(1, -1, 1), 1);
        assert_eq!(
            step_percent(0, -1, 1),
            0,
            "a level under the floor is not raised"
        );
        assert_eq!(step_percent(2, -1, 0), 0);
        assert_eq!(step_percent(40, -2, 0), 30);
        assert_eq!(step_percent(50, 0, 1), 50);
    }

    fn read(display: &str, seq: u64) -> DisplayJob {
        DisplayJob::Read {
            display: display.into(),
            value: Value::Brightness,
            seq,
        }
    }

    fn set(display: &str, level: u8, seq: u64) -> DisplayJob {
        DisplayJob::Set {
            display: display.into(),
            value: Value::Brightness,
            level,
            seq,
        }
    }

    #[test]
    fn queued_jobs_fold_to_the_latest_of_each_kind() {
        let read = |seq| read("DP-2", seq);
        let set = |level, seq| set("DP-2", level, seq);
        assert_eq!(
            fold_display(vec![read(1), set(40, 2), set(45, 3), read(4), set(50, 5)]),
            vec![read(4), set(50, 5)]
        );
        assert_eq!(fold_display(vec![set(40, 1)]), vec![set(40, 1)]);
    }

    #[test]
    fn displays_sharing_a_lane_fold_apart() {
        assert_eq!(
            fold_display(vec![
                set("eDP-1", 40, 1),
                set("eDP-2", 60, 2),
                set("eDP-1", 45, 3)
            ]),
            vec![set("eDP-2", 60, 2), set("eDP-1", 45, 3)]
        );
    }

    #[test]
    fn a_timed_out_job_reports_the_display_as_not_answering() {
        assert_eq!(
            not_answering(&read("DP-2", 7)),
            DisplayReport {
                display: "DP-2".into(),
                value: Value::Brightness,
                seq: 7,
                outcome: Outcome::Read(None),
            }
        );
        assert_eq!(
            not_answering(&set("DP-2", 40, 8)).outcome,
            Outcome::Failed(None)
        );
    }

    #[test]
    fn the_sysfs_root_comes_from_the_environment() {
        assert_eq!(
            Sysfs::from_var(None),
            Sysfs {
                root: "/sys".into(),
                direct: false
            }
        );
        assert_eq!(
            Sysfs::from_var(Some("/tmp/fake".into())),
            Sysfs {
                root: "/tmp/fake".into(),
                direct: true
            }
        );
    }

    fn write(root: &Path, file: &str, text: &str) {
        let p = root.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn a_backlight_reads_and_writes_under_a_sysfs_root() {
        let d = TempDir::new("backlight");
        write(
            &d.0,
            "class/drm/card1-eDP-1/amdgpu_bl1/max_brightness",
            "65535",
        );
        write(&d.0, "class/backlight/amdgpu_bl1/brightness", "32768");
        write(&d.0, "class/backlight/amdgpu_bl1/max_brightness", "65535");
        let sysfs = Sysfs::from_var(Some(d.0.clone().into()));
        let mut lane = Lane::new();
        let mut buses = no_buses();
        let mut run =
            |job| zbus::block_on(perform_display(&mut buses, &sysfs, &mut lane, job)).outcome;

        assert_eq!(
            run(read("eDP-1", 1)),
            Outcome::Read(Some(Reading {
                level: 50,
                floor: 1
            }))
        );
        assert_eq!(run(set("eDP-1", 25, 2)), Outcome::Wrote);
        assert_eq!(
            std::fs::read_to_string(d.0.join("class/backlight/amdgpu_bl1/brightness")).unwrap(),
            "16384"
        );
        assert_eq!(run(read("DP-9", 3)), Outcome::Read(None));
    }

    /// A sysfs root with DP-2 on DDC bus 8, and a fake `ddcutil` running `script` in it.
    /// The lane runs it through `sh`: exec'ing a file this process just wrote can fail with ETXTBSY while another test thread forks.
    fn ddc_fixture(tag: &str, script: &str) -> (TempDir, Sysfs, Lane) {
        let d = TempDir::new(tag);
        std::fs::create_dir_all(d.0.join("class/drm/card1-DP-2/i2c-8")).unwrap();
        let bin = d.0.join("ddcutil");
        write(&d.0, "ddcutil", &format!("{script}\n"));
        let sysfs = Sysfs::from_var(Some(d.0.clone().into()));
        let lane = Lane {
            ddc_max: None,
            ddcutil: vec!["sh".into(), bin.to_str().unwrap().into()],
        };
        (d, sysfs, lane)
    }

    #[test]
    fn a_ddc_monitor_converts_its_own_maximum_to_percent() {
        let (d, sysfs, mut lane) = ddc_fixture(
            "ddc",
            r#"case "$*" in
*getvcp*) echo "VCP 10 C 30 200" ;;
*setvcp*) echo "$*" > "$(dirname "$0")/set" ;;
esac"#,
        );
        let mut buses = no_buses();
        let mut run =
            |job| zbus::block_on(perform_display(&mut buses, &sysfs, &mut lane, job)).outcome;

        assert_eq!(
            run(set("DP-2", 50, 1)),
            Outcome::Wrote,
            "a set with no maximum yet reads it first"
        );
        assert_eq!(
            std::fs::read_to_string(d.0.join("set")).unwrap().trim(),
            "--bus 8 --skip-ddc-checks --noverify setvcp 10 100"
        );
        assert_eq!(
            run(read("DP-2", 2)),
            Outcome::Read(Some(Reading {
                level: 15,
                floor: 0
            }))
        );
    }

    #[test]
    fn a_monitor_that_hangs_past_the_timeout_reports_a_failed_set() {
        let (_d, sysfs, lane) = ddc_fixture("ddc-hang", "sleep 5");
        let (jobs, rx) = std::sync::mpsc::channel();
        let (tx, reports) = std::sync::mpsc::channel();
        jobs.send(set("DP-2", 40, 9)).unwrap();
        drop(jobs);
        let started = std::time::Instant::now();
        run_lane(rx, &sysfs, lane, Duration::from_millis(200), |r| {
            tx.send(r).unwrap()
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            reports.try_iter().collect::<Vec<_>>(),
            vec![not_answering(&set("DP-2", 40, 9))]
        );
    }
}
