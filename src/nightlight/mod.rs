//! The built-in night light: `[night_light]` warms the screen by night with a gamma ramp on every output. An
//! output whose gamma another client holds is left to it; one with no gamma table is tinted by a last pass (`tint`).

mod color;
mod schedule;
mod sun;
pub mod tint;
mod zone;

use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::config::{NightLightConfig, NightSchedule};
use crate::gammacontrol::{GammaControlHandler, GammaControlState};
use crate::menuview::{MenuFrom, MenuKind};
use crate::raster::{Canvas, Rect};
use crate::startmenu::{MenuItem, MenuLayout, MenuStyle, submenu_max_rows};
use schedule::{LocalClock, Plan};

pub use color::NEUTRAL;

const SETTINGS: [&str; 3] = ["reprise-settings", "--page", "night-light"];

const ICONS: [&str; 3] = ["night-light", "weather-clear-night", "night-light-symbolic"];
const PAUSED_ICONS: [&str; 2] = ["night-light-disabled", "night-light-disabled-symbolic"];

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
    next_start: Option<i64>,
    zone: Option<String>,
    zone_place: Option<(f64, f64)>,
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
            next_start: None,
            zone: None,
            zone_place: None,
            told_no_place: false,
            outputs: Vec::new(),
            control: GammaControlState::new::<State, _>(
                dh,
                crate::sandbox::filter("zwlr_gamma_control_manager_v1"),
            ),
        }
    }
}

struct Local;

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

fn now() -> i64 {
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

/// Where sunset is worked out for: the configured place, else the time
/// zone's city.
fn place(nl: &mut NightLight) -> Option<(f64, f64)> {
    if let Some(p) = nl.cfg.location {
        return Some(p);
    }
    let zone = zone::system_zone();
    if zone != nl.zone {
        nl.zone_place = zone.as_deref().and_then(zone::coordinates);
        match (&zone, nl.zone_place) {
            (Some(z), Some((lat, lon))) => {
                log::info!("night light: sunset for {z} ({lat:.2}, {lon:.2})")
            }
            (z, _) => log::info!("night light: no coordinates for time zone {z:?}"),
        }
        nl.zone = zone;
    }
    nl.zone_place
}

/// The plan the config asks for. Sunset without a place falls back to
/// the fixed times.
fn plan(nl: &mut NightLight) -> Plan {
    let fixed = Plan::Fixed {
        from: nl.cfg.from,
        to: nl.cfg.to,
    };
    if nl.cfg.schedule == NightSchedule::Fixed {
        return fixed;
    }
    match place(nl) {
        Some((lat, lon)) => Plan::Sun { lat, lon },
        None => {
            if !nl.told_no_place {
                log::warn!(
                    "night light: no place to work out sunset for; set [night_light] latitude \
                     and longitude. Using the from and to times meanwhile."
                );
                nl.told_no_place = true;
            }
            fixed
        }
    }
}

/// Work out the temperature for now and put it on every output that does
/// not have it yet. Also takes a changed `[night_light]` section in.
pub fn update(state: &mut State) {
    let now = now();
    let cfg = state.config.night_light.clone();
    let nl = &mut state.night;
    let before = nl.kelvin;
    let shown_before = (nl.cfg.enabled, nl.paused_until.is_some());
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
        nl.next_start = schedule::next_start(now, &nights);
    } else {
        nl.kelvin = None;
        nl.next_start = None;
    }
    if nl.kelvin != before {
        match nl.kelvin {
            Some(k) => log::info!("night light: {k} K"),
            None => log::info!("night light: neutral"),
        }
    }
    let shown = (nl.cfg.enabled, nl.paused_until.is_some());
    apply_all(state);
    if shown != shown_before {
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

/// Pause until the night now on (or the next one) is over, or resume:
/// the tray cell's click and menu, and the keybind.
pub fn toggle_pause(state: &mut State) {
    let now = now();
    let nl = &mut state.night;
    if !nl.cfg.enabled {
        log::info!("night light: not enabled; nothing to pause");
        return;
    }
    if nl.paused_until.take().is_some() {
        log::info!("night light: resumed");
    } else {
        let plan = plan(nl);
        let until = schedule::pause_until(now, &schedule::nights(plan, now, &Local));
        nl.paused_until = Some(until);
        log::info!(
            "night light: paused until {}",
            crate::taskbar::time_text(until)
        );
    }
    update(state);
}

pub fn shown(nl: &NightLight) -> bool {
    nl.cfg.enabled
}

/// The tray cell, `px` square: the theme's night icon, its paused form or
/// the same icon at half strength while paused, the placeholder when the
/// theme has none.
pub fn icon(state: &mut State, px: i32) -> Canvas {
    let paused = state.night.paused_until.is_some();
    let icons = &mut state.icons;
    let mut find = |names: &[&str]| names.iter().find_map(|n| icons.get(n, px));
    if paused && let Some(c) = find(&PAUSED_ICONS) {
        return c;
    }
    let mut c = find(&ICONS).unwrap_or_else(|| reprisekit::classic::widgets::placeholder(px, px));
    if paused {
        c.fade(1, 2);
    }
    c
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Pause,
    Settings,
}

/// The tray cell's menu, opened from `cell` (layout coordinates) on
/// `output` on the screen side of the bar: Pause (or Resume), then
/// Settings....
pub fn open_menu(state: &mut State, output: &Output, cell: Rect) {
    if state.layout_box(Some(output)).width <= 0 {
        return;
    }
    let pause = if state.night.paused_until.is_some() {
        "&Resume"
    } else {
        "&Pause"
    };
    let items = vec![
        MenuItem::item(pause).bold(true),
        MenuItem::Separator,
        MenuItem::item("&Settings..."),
    ];
    let rows = vec![Some(Row::Pause), None, Some(Row::Settings)];
    let usable = state.usable_area(Some(output));
    let m = &state.config.theme.metrics;
    let style = MenuStyle::context_for(&state.config.theme, &items).fit(
        &state.text,
        &items,
        state.config.theme.fonts.size_px,
        usable.width,
    );
    let layout = MenuLayout::compute_styled(&style, &items, submenu_max_rows(m, usable.height));
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
        Row::Pause => toggle_pause(state),
        Row::Settings => state.spawn_argv_activated(&SETTINGS),
    }
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
