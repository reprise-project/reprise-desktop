//! Media key backends: volume and mute through `wpctl`, backlights through logind's `SetBrightness`, external monitors over DDC/CI through `ddcutil`, transport through MPRIS, the wireless radio through NetworkManager. One worker thread runs the jobs and reports over a calloop channel so the event loop never blocks.

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
    /// Signed steps on the brightness of `output` (a connector name, the one under the pointer), or of the best backlight when `None`.
    Backlight {
        output: Option<String>,
        steps: i32,
    },
    Kbd(i32),
    Player(&'static str),
    Wifi,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    Volume { level: f64, muted: bool },
    Mic { muted: bool },
    Brightness { level: f64 },
    Kbd { level: f64 },
    Player { name: String, verb: &'static str },
    Wifi { on: bool },
}

/// Fold a burst of jobs into the fewest with the same end state: consecutive volume, keyboard backlight and same-output brightness steps add up, anything else keeps its order. A burst that cancels out folds to a zero step, not nothing, so the OSD shows the level it ends on.
pub fn coalesce(jobs: Vec<Job>) -> Vec<Job> {
    let mut out: Vec<Job> = Vec::new();
    for j in jobs {
        match (out.last_mut(), j) {
            (Some(Job::VolumeStep(a)), Job::VolumeStep(b)) => *a += b,
            (
                Some(Job::Backlight { output: a, steps }),
                Job::Backlight {
                    output: b,
                    steps: n,
                },
            ) if *a == b => *steps += n,
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
    let name = pick_device(std::path::Path::new(&format!("/sys/class/{class}")), pick)?;
    let (cur, max) = sysfs_levels(class, &name)?;
    Some((name, cur, max))
}

fn sysfs_levels(class: &str, name: &str) -> Option<(u32, u32)> {
    let read = |f: &str| -> Option<u32> {
        std::fs::read_to_string(format!("/sys/class/{class}/{name}/{f}"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("brightness")?, read("max_brightness")?))
}

#[derive(Debug, PartialEq)]
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

async fn backlight_step(buses: &mut Buses, name: &str, steps: i32) -> Option<Report> {
    let (cur, max) = sysfs_levels("backlight", name)?;
    let v = next_level(cur, max, steps, backlight_floor(max));
    set_brightness(buses, "backlight", name, v)
        .await
        .then_some(())?;
    Some(Report::Brightness {
        level: v as f64 / max.max(1) as f64,
    })
}

/// Step the brightness of the monitor on I2C bus `bus` over DDC/CI. Each call skips ddcutil's start-up DDC checks and the write skips its read-back: on some monitors those cost seconds, which would outrun JOB_TIMEOUT. A step that lands where it started skips the write.
async fn ddc_step(bus: u32, steps: i32) -> Option<Report> {
    let bus = bus.to_string();
    let (cur, max) = parse_getvcp(
        &command(
            "ddcutil",
            &[
                "--bus",
                &bus,
                "--skip-ddc-checks",
                "getvcp",
                VCP_BRIGHTNESS,
                "--brief",
            ],
        )
        .await?,
    )?;
    // No floor: a monitor at its lowest brightness still shows the picture.
    let v = next_level(cur, max, steps, 0);
    if v != cur {
        let v = v.to_string();
        command(
            "ddcutil",
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
        .await?;
    }
    Some(Report::Brightness {
        level: v as f64 / max.max(1) as f64,
    })
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
        Job::Backlight { output, steps } => {
            let panel = output
                .as_deref()
                .and_then(|o| panel_for(std::path::Path::new("/sys/class/drm"), o));
            match panel {
                Some(Panel::Ddc(bus)) => ddc_step(bus, steps).await,
                Some(Panel::Backlight(name)) => backlight_step(buses, &name, steps).await,
                None if output.as_deref().is_none_or(is_internal) => {
                    let (name, _, _) = sysfs_device("backlight", |_| true)?;
                    backlight_step(buses, &name, steps).await
                }
                None => {
                    log::warn!("media: {output:?} has neither a backlight nor a DDC bus");
                    None
                }
            }
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

async fn perform_timed(buses: &mut Buses, job: Job) -> Option<Option<Report>> {
    futures_lite::future::or(async { Some(perform(buses, job).await) }, async {
        async_io::Timer::after(JOB_TIMEOUT).await;
        None
    })
    .await
}

fn worker(jobs: Receiver<Job>, reports: channel::Sender<Report>) {
    let mut buses = Buses {
        system: None,
        session: None,
    };
    while let Ok(first) = jobs.recv() {
        let mut burst = vec![first];
        while let Ok(j) = jobs.try_recv() {
            burst.push(j);
        }
        for job in coalesce(burst) {
            // A panic in a job would otherwise end the thread and leave the media keys dead for the session.
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                zbus::block_on(perform_timed(&mut buses, job.clone()))
            }));
            match done {
                Ok(Some(Some(r))) => {
                    let _ = reports.send(r);
                }
                Ok(Some(None)) => {}
                Ok(None) => {
                    log::warn!("media: {job:?} timed out after {JOB_TIMEOUT:?}");
                    // A wedged socket is the usual cause; reconnect next time.
                    buses.system = None;
                    buses.session = None;
                }
                Err(p) => {
                    let msg = p
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| p.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "non-string panic".into());
                    log::warn!("media: {job:?} panicked ({msg}); the worker carries on");
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
        let added = self.handle.insert_source(rrx, |event, _, state| {
            if let channel::Event::Msg(r) = event {
                log::debug!("media: {r:?}");
                state.show_osd(&r);
            }
        });
        if let Err(e) = added {
            log::warn!("media: no report source ({}); media keys are off", e.error);
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("media".into())
            .spawn(move || worker(jrx, rtx));
        if spawned.is_err() {
            log::warn!("media: no worker thread; media keys are off");
            return;
        }
        self.key_input.media.jobs = Some(jtx);
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

    fn pointer_output(&self) -> Option<String> {
        let p = self.pointer_location();
        self.output_at(p.x, p.y).map(|o| o.name())
    }

    pub fn run_media(&mut self, a: &Action) {
        let job = match a {
            Action::VolumeUp => Job::VolumeStep(1),
            Action::VolumeDown => Job::VolumeStep(-1),
            Action::Mute => Job::Mute,
            Action::MicMute => Job::MicMute,
            Action::BrightnessUp | Action::BrightnessDown => Job::Backlight {
                output: self.pointer_output(),
                steps: if *a == Action::BrightnessUp { 1 } else { -1 },
            },
            Action::KbdBrightnessUp => Job::Kbd(1),
            Action::KbdBrightnessDown => Job::Kbd(-1),
            Action::PlayPause => Job::Player("PlayPause"),
            Action::Next => Job::Player("Next"),
            Action::Prev => Job::Player("Previous"),
            Action::Stop => Job::Player("Stop"),
            Action::Wifi => Job::Wifi,
            _ => return,
        };
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
    fn brightness_steps_fold_per_output() {
        let bl = |o: &str, steps| Job::Backlight {
            output: Some(o.into()),
            steps,
        };
        assert_eq!(
            coalesce(vec![
                bl("DP-2", 1),
                bl("DP-2", 1),
                bl("eDP-1", -1),
                bl("DP-2", 1)
            ]),
            vec![bl("DP-2", 2), bl("eDP-1", -1), bl("DP-2", 1)]
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
}
