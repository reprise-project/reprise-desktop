//! The built-in night light: `[night_light]` warms the screen by night with a gamma ramp on every output. An
//! output whose gamma another client holds is left to it; one with no gamma table is tinted by a last pass (`tint`).

mod color;
pub mod schedule;
mod sun;
pub mod tint;

use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::config::{NightLightConfig, NightSchedule};
use crate::gammacontrol::{GammaControlHandler, GammaControlState};
use crate::menuview::{MenuFrom, MenuKind};
use crate::raster::Rect;
use crate::startmenu::{MenuItem, MenuStyle};
use schedule::{LocalClock, Plan};

pub use color::NEUTRAL;

const SETTINGS: [&str; 3] = ["reprise-settings", "--page", "night-light"];

/// The entries per channel a client's ramp has on an output with no gamma
/// table: the size wlr-gamma-control clients usually use.
const TINT_RAMP_SIZE: u32 = 256;

struct OutputGamma {
    output: Output,
    applied: Option<u32>,
    /// Apply again even if `applied` matches: the output came back on, or
    /// a client let go.
    dirty: bool,
    /// Said once in the log: tinted by the frame pass, or the CRTC
    /// refused the ramp.
    told_tint: bool,
    told_refused: bool,
}

pub struct NightLight {
    cfg: NightLightConfig,
    pub paused_until: Option<i64>,
    pub kelvin: Option<u32>,
    /// When the warming now on is gone (the night's end plus its fade); None while not warming.
    warm_until: Option<i64>,
    next_start: Option<i64>,
    zone: Zone,
    told_no_place: bool,
    outputs: Vec<OutputGamma>,
    pub control: GammaControlState,
}

impl NightLight {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> NightLight {
        NightLight {
            cfg: NightLightConfig::default(),
            paused_until: None,
            kelvin: None,
            warm_until: None,
            next_start: None,
            zone: Zone::default(),
            told_no_place: false,
            outputs: Vec::new(),
            control: GammaControlState::new::<State, _>(
                dh,
                crate::sandbox::filter("zwlr_gamma_control_manager_v1"),
            ),
        }
    }
}

pub struct Local;

impl LocalClock for Local {
    fn date(&self, t: i64) -> (i32, i32, i32) {
        let tm = reprisefs::locale::localtime(t);
        (tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
    }

    fn at(&self, d: (i32, i32, i32), minute: u32) -> i64 {
        let (h, m) = ((minute / 60) as i32, (minute % 60) as i32);
        reprisefs::locale::mktime(d.0, d.1, d.2, h, m, 0)
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn to_next_minute() -> std::time::Duration {
    std::time::Duration::from_secs((60 - now().rem_euclid(60)).max(1) as u64)
}

/// Work out the first temperature and re-read the schedule at every
/// minute boundary from now on. Runs once the backend drives its outputs.
pub fn setup(state: &mut State) {
    let timer =
        state
            .handle
            .insert_source(Timer::from_duration(to_next_minute()), |_, _, state| {
                update(state);
                TimeoutAction::ToDuration(to_next_minute())
            });
    if let Err(e) = timer {
        log::warn!("night light: no minute timer: {}", e.error);
    }
    update(state);
}

/// The time zone last looked up and its city's place.
#[derive(Default)]
pub struct Zone {
    name: Option<String>,
    place: Option<(f64, f64)>,
}

impl Zone {
    /// Where sunset is worked out for: `location` when set, else the time zone's city.
    fn place(&mut self, location: Option<(f64, f64)>) -> Option<(f64, f64)> {
        if location.is_some() {
            return location;
        }
        let zone = crate::tzcity::system_zone();
        if zone != self.name {
            self.place = zone.as_deref().and_then(crate::tzcity::coordinates);
            match (&zone, self.place) {
                (Some(z), Some((lat, lon))) => {
                    log::info!("sunset and sunrise for {z} ({lat:.2}, {lon:.2})")
                }
                (z, _) => log::info!("no coordinates for time zone {z:?}"),
            }
            self.name = zone;
        }
        self.place
    }
}

/// The plan `cfg` asks for, whether or not night light is on. Sunset without a place falls back to
/// the fixed times; `told` is whether that was said in the log already.
pub fn plan_of(cfg: &NightLightConfig, zone: &mut Zone, told: &mut bool) -> Plan {
    let fixed = Plan::Fixed {
        from: cfg.from,
        to: cfg.to,
    };
    if cfg.schedule == NightSchedule::Fixed {
        return fixed;
    }
    match zone.place(cfg.location) {
        Some((lat, lon)) => Plan::Sun { lat, lon },
        None => {
            if !*told {
                log::warn!(
                    "night light: no place to work out sunset for; set [night_light] latitude \
                     and longitude. Using the from and to times meanwhile."
                );
                *told = true;
            }
            fixed
        }
    }
}

fn plan(nl: &mut NightLight) -> Plan {
    plan_of(&nl.cfg, &mut nl.zone, &mut nl.told_no_place)
}

/// Work out the temperature for now and put it on every output that does
/// not have it yet. Also takes a changed `[night_light]` section in.
pub fn update(state: &mut State) {
    let now = now();
    let cfg = state.config.night_light.clone();
    let nl = &mut state.night;
    let before = nl.kelvin;
    let shown_before = (nl.cfg.enabled, nl.paused_until.is_some(), warming(nl));
    if cfg != nl.cfg {
        if cfg.enabled != nl.cfg.enabled || cfg.schedule != nl.cfg.schedule {
            nl.paused_until = None;
        }
        nl.cfg = cfg;
    }
    if nl.paused_until.is_some_and(|t| t <= now) {
        nl.paused_until = None;
    }
    if nl.cfg.enabled {
        let plan = plan(nl);
        let nights = schedule::nights(plan, now, &Local);
        let level = if nl.paused_until.is_some() {
            0.0
        } else {
            schedule::level(now, &nights)
        };
        let k = schedule::kelvin(level, nl.cfg.temperature);
        nl.kelvin = (k < NEUTRAL).then_some(k);
        nl.warm_until = warming(nl).then(|| schedule::pause_until(now, &nights, &Local));
        nl.next_start = schedule::next_start(now, &nights);
    } else {
        nl.kelvin = None;
        nl.warm_until = None;
        nl.next_start = None;
    }
    if nl.kelvin != before {
        match nl.kelvin {
            Some(k) => log::info!("night light: {k} K"),
            None => log::info!("night light: neutral"),
        }
    }
    let shown = (nl.cfg.enabled, nl.paused_until.is_some(), warming(nl));
    apply_all(state);
    if shown != shown_before {
        crate::displaysview::refresh(state);
        state.redraw_taskbars();
    }
}

fn apply_all(state: &mut State) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for o in &outputs {
        apply(state, o);
    }
}

/// Put the current temperature on `output` unless it has it already or
/// a client holds its gamma.
fn apply(state: &mut State, output: &Output) {
    if state.night.control.held(output) {
        return;
    }
    let kelvin = state.night.kelvin;
    let nl = &mut state.night;
    let i = match nl.outputs.iter().position(|o| &o.output == output) {
        Some(i) => i,
        None => {
            nl.outputs.push(OutputGamma {
                output: output.clone(),
                applied: None,
                dirty: true,
                told_tint: false,
                told_refused: false,
            });
            nl.outputs.len() - 1
        }
    };
    let o = &nl.outputs[i];
    if o.applied == kelvin && !o.dirty {
        return;
    }
    let took = match crate::backend::gamma_size(state, output) {
        Some(size) => {
            let ramp = kelvin.map(|k| color::ramp(size, k));
            let took = crate::backend::set_gamma(state, output, ramp.as_deref());
            let o = &mut state.night.outputs[i];
            if !took && !o.told_refused {
                o.told_refused = true;
                log::warn!(
                    "night light: {} did not take the gamma ramp; trying again each minute",
                    output.name()
                );
            }
            took
        }
        None => {
            let o = &mut state.night.outputs[i];
            if kelvin.is_some() && !o.told_tint {
                o.told_tint = true;
                log::info!(
                    "night light: {} has no gamma table; tinting its frames instead",
                    output.name()
                );
            }
            let gains = kelvin.map(|k| color::whitepoint(k).map(|g| g as f32));
            if tint::set(output, gains) {
                crate::backend::schedule(state, output);
            }
            true
        }
    };
    if took {
        let o = &mut state.night.outputs[i];
        o.applied = kelvin;
        o.dirty = false;
    }
}

/// The kernel refused a gamma ramp on `output`: a client holding its gamma is told and let
/// go, and night light puts its own ramp back on its next try.
pub fn gamma_failed(state: &mut State, output: &Output) {
    if state.night.control.fail(output) {
        log::info!(
            "night light: {}'s gamma client was refused; ours goes back on",
            output.name()
        );
    }
    if let Some(o) = state.night.outputs.iter_mut().find(|o| &o.output == output) {
        o.dirty = true;
    }
}

pub fn output_on(state: &mut State, output: &Output) {
    if let Some(o) = state.night.outputs.iter_mut().find(|o| &o.output == output) {
        o.dirty = true;
    }
    apply(state, output);
}

pub fn outputs_changed(state: &mut State) {
    let live: Vec<Output> = state.scene.outputs().to_vec();
    state.night.outputs.retain(|o| live.contains(&o.output));
    state.night.control.retain_outputs(&live);
    apply_all(state);
}

/// How long a pause from the displays cell's menu lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    Hour,
    /// Until the night now on (or the next one) is over.
    Tomorrow,
}

/// When a `pause` taken at `now` ends, given the schedule's `nights`.
fn pause_end(pause: Pause, now: i64, nights: &[(i64, i64)], clock: &dyn LocalClock) -> i64 {
    match pause {
        Pause::Hour => now + 3600,
        Pause::Tomorrow => schedule::pause_until(now, nights, clock),
    }
}

/// Pause for `pause`'s span from now; nothing while night light is off. A pause outlives a
/// reload that keeps `enabled` and `schedule`, never a restart.
pub fn pause(state: &mut State, pause: Pause) {
    let now = now();
    let nl = &mut state.night;
    if !nl.cfg.enabled {
        log::info!("night light: not enabled; nothing to pause");
        return;
    }
    let plan = plan(nl);
    let until = pause_end(pause, now, &schedule::nights(plan, now, &Local), &Local);
    nl.paused_until = Some(until);
    log::info!(
        "night light: paused until {}",
        crate::taskbar::time_text(until)
    );
    update(state);
}

pub fn resume(state: &mut State) {
    if state.night.paused_until.take().is_some() {
        log::info!("night light: resumed");
        update(state);
    }
}

/// Resume, else pause until tomorrow: the keybind and the displays panel's check box.
pub fn toggle_pause(state: &mut State) {
    if state.night.paused_until.is_some() {
        resume(state);
    } else {
        pause(state, Pause::Tomorrow);
    }
}

/// Pause (`paused` true) or resume; nothing while night light is off or already so.
pub fn set_paused(state: &mut State, paused: bool) {
    let nl = &state.night;
    if nl.cfg.enabled && nl.paused_until.is_some() != paused {
        toggle_pause(state);
    }
}

pub fn enabled(nl: &NightLight) -> bool {
    nl.cfg.enabled
}

/// Enabled, not paused, and the screen warmer than neutral now.
pub fn warming(nl: &NightLight) -> bool {
    nl.cfg.enabled && nl.paused_until.is_none() && nl.kelvin.is_some()
}

pub fn panel_night(nl: &NightLight) -> reprisekit::controls::displaypanel::Night {
    reprisekit::controls::displaypanel::Night {
        enabled: nl.cfg.enabled,
        on: nl.cfg.enabled && nl.paused_until.is_none(),
        state: panel_text(
            nl.cfg.enabled,
            nl.paused_until,
            nl.warm_until,
            nl.next_start,
        ),
    }
}

/// The text beside the panel's check box: off, the pause's end, the warming's end, else the next start.
fn panel_text(
    enabled: bool,
    paused_until: Option<i64>,
    warm_until: Option<i64>,
    next_start: Option<i64>,
) -> String {
    let time = crate::taskbar::time_text;
    if !enabled {
        return "Off".into();
    }
    match (paused_until, warm_until, next_start) {
        (Some(t), _, _) => format!("Paused until {}", time(t)),
        (None, Some(t), _) => format!("until {}", time(t)),
        (None, None, Some(t)) => format!("from {}", time(t)),
        (None, None, None) => String::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Pause(Pause),
    Resume,
    Settings,
}

/// The displays cell's menu rows: while paused, Resume and the time it resumes by itself; else the
/// two pauses; Settings... last. Settings... alone while night light is off.
fn menu(enabled: bool, paused_until: Option<i64>) -> (Vec<MenuItem>, Vec<Option<Row>>) {
    if !enabled {
        return (
            vec![MenuItem::item("&Settings...").bold(true)],
            vec![Some(Row::Settings)],
        );
    }
    let (mut items, mut rows) = match paused_until {
        Some(t) => (
            vec![
                MenuItem::item("&Resume Night Light").bold(true),
                MenuItem::entry(format!("Resumes at {}", crate::taskbar::time_text(t)))
                    .enabled(false),
            ],
            vec![Some(Row::Resume), None],
        ),
        None => (
            vec![
                MenuItem::item("Disable for 1 &hour").bold(true),
                MenuItem::item("Disable until &tomorrow"),
            ],
            vec![
                Some(Row::Pause(Pause::Hour)),
                Some(Row::Pause(Pause::Tomorrow)),
            ],
        ),
    };
    items.extend([MenuItem::Separator, MenuItem::item("&Settings...")]);
    rows.extend([None, Some(Row::Settings)]);
    (items, rows)
}

/// The displays cell's menu (`menu`), opened from `cell` (layout coordinates) on `output` on the
/// screen side of the bar.
pub fn open_menu(state: &mut State, output: &Output, cell: Rect) {
    if state.layout_box(Some(output)).width <= 0 {
        return;
    }
    let (items, rows) = menu(state.night.cfg.enabled, state.night.paused_until);
    let usable = state.usable_area(Some(output));
    let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
        &state.config.theme,
        &state.text,
        &items,
        usable,
    );
    let (x, y) = crate::barplace::bar_popup(
        &state.bar_placement(output),
        cell,
        (layout.width, layout.height),
        usable,
    );
    state.push_menu(
        0,
        (x, y),
        layout,
        items,
        output.clone(),
        MenuKind::NightLight(rows),
        MenuFrom::Bar,
    );
    state.redraw_menu(0);
}

pub fn pick(state: &mut State, row: Row) {
    match row {
        Row::Pause(p) => pause(state, p),
        Row::Resume => resume(state),
        Row::Settings => open_settings(state),
    }
}

pub fn open_settings(state: &mut State) {
    state.spawn_argv_activated(&SETTINGS);
}

pub fn tip(nl: &NightLight) -> String {
    tip_text(nl.paused_until, nl.kelvin, nl.next_start)
}

/// The tooltip for a pause ending at `paused_until`, else the schedule's
/// `kelvin` now, else its next start.
fn tip_text(paused_until: Option<i64>, kelvin: Option<u32>, next_start: Option<i64>) -> String {
    if let Some(t) = paused_until {
        return format!(
            "Night light is paused until {}",
            crate::taskbar::time_text(t)
        );
    }
    match (kelvin, next_start) {
        (Some(k), _) => format!("Night light: {k} K"),
        (None, Some(t)) => format!("Night light starts at {}", crate::taskbar::time_text(t)),
        (None, None) => "Night light".into(),
    }
}

impl GammaControlHandler for State {
    fn gamma_control_state(&mut self) -> &mut GammaControlState {
        &mut self.night.control
    }

    fn gamma_size(&mut self, output: &Output) -> Option<u32> {
        crate::backend::output_power(self, output)?;
        Some(crate::backend::gamma_size(self, output).map_or(TINT_RAMP_SIZE, |s| s as u32))
    }

    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> bool {
        let took = if crate::backend::gamma_size(self, output).is_some() {
            crate::backend::set_gamma(self, output, ramp.as_deref())
        } else {
            if tint::set(output, ramp.as_deref().and_then(tint::gains_of)) {
                crate::backend::schedule(self, output);
            }
            true
        };
        if ramp.is_none() {
            log::info!(
                "night light: a client let go of {}'s gamma; ours goes back on",
                output.name()
            );
            if let Some(o) = self.night.outputs.iter_mut().find(|o| &o.output == output) {
                o.applied = None;
                o.dirty = true;
            }
            apply(self, output);
        }
        took
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[MenuItem]) -> Vec<(String, bool)> {
        (items.iter())
            .map(|i| match i {
                MenuItem::Entry { label, enabled, .. } => (label.clone(), *enabled),
                _ => ("-".into(), false),
            })
            .collect()
    }

    #[test]
    fn the_menu_offers_two_pauses_then_resume_and_its_time_while_paused() {
        let (items, rows) = menu(true, None);
        assert_eq!(
            labels(&items),
            [
                ("Disable for 1 hour".into(), true),
                ("Disable until tomorrow".into(), true),
                ("-".into(), false),
                ("Settings...".into(), true),
            ]
        );
        assert_eq!(
            rows,
            [
                Some(Row::Pause(Pause::Hour)),
                Some(Row::Pause(Pause::Tomorrow)),
                None,
                Some(Row::Settings)
            ]
        );
        let (items, rows) = menu(true, Some(0));
        let shown = labels(&items);
        assert_eq!(shown[0], ("Resume Night Light".into(), true));
        assert_eq!(
            shown[1],
            (
                format!("Resumes at {}", crate::taskbar::time_text(0)),
                false
            ),
            "the resume time is a greyed row"
        );
        assert_eq!(rows[..2], [Some(Row::Resume), None]);
        assert_eq!(menu(false, None).1, [Some(Row::Settings)]);
    }

    #[test]
    fn an_hour_pause_ends_an_hour_on_whatever_the_schedule() {
        let nights = [(1000, 50_000)];
        assert_eq!(pause_end(Pause::Hour, 2000, &nights, &Local), 5600);
        assert_eq!(
            pause_end(Pause::Tomorrow, 2000, &nights, &Local),
            schedule::pause_until(2000, &nights, &Local)
        );
    }

    #[test]
    fn a_pause_resumes_and_outlives_a_reload_that_keeps_the_schedule() {
        let mut h = crate::harness::Harness::new();
        h.state.config.night_light.enabled = true;
        update(&mut h.state);
        pause(&mut h.state, Pause::Hour);
        let until = h.state.night.paused_until.expect("paused");
        assert!((until - now() - 3600).abs() <= 2, "an hour from now");
        h.state.config.night_light.temperature += 100;
        update(&mut h.state);
        assert_eq!(h.state.night.paused_until, Some(until), "a reload keeps it");
        resume(&mut h.state);
        assert_eq!(h.state.night.paused_until, None);
        pause(&mut h.state, Pause::Tomorrow);
        assert!(h.state.night.paused_until.is_some_and(|t| t > now()));
        h.state.config.night_light.enabled = false;
        update(&mut h.state);
        assert_eq!(h.state.night.paused_until, None, "turning it off ends it");
        pause(&mut h.state, Pause::Hour);
        assert_eq!(h.state.night.paused_until, None, "nothing pauses while off");
    }

    #[test]
    fn the_panel_says_until_when_warming_paused_or_off() {
        assert_eq!(panel_text(false, None, None, None), "Off");
        assert!(panel_text(true, Some(0), None, None).starts_with("Paused until "));
        assert!(panel_text(true, None, Some(0), Some(0)).starts_with("until "));
        assert!(panel_text(true, None, None, Some(0)).starts_with("from "));
        assert_eq!(panel_text(true, None, None, None), "");
    }

    #[test]
    fn the_tip_says_pause_then_temperature_then_start() {
        assert_eq!(tip_text(None, Some(3400), None), "Night light: 3400 K");
        assert_eq!(tip_text(None, None, None), "Night light");
        let paused = tip_text(Some(0), Some(3400), None);
        assert!(
            paused.starts_with("Night light is paused until "),
            "a pause wins over the temperature: {paused}"
        );
        let later = tip_text(None, None, Some(0));
        assert!(later.starts_with("Night light starts at "), "{later}");
    }
}
