//! Dark mode on a schedule (`[appearance] mode = "night"` or `"hours"`), in every interface. The
//! config keeps the schedule; the look in force is the config rebuilt with `mode` set to "dark" or
//! "light", put in through `configwatch::reload` as an edit of the file would be, so the
//! compositor's look, the programs' look, the generated toolkit themes and the portal's
//! color-scheme change alike.

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::config::{AppearanceMode, Config, ModeConfig, NightLightConfig};
use crate::nightlight::schedule::{self, LocalClock, Plan};
use crate::nightlight::{Local, Zone};

#[derive(Default)]
pub struct DarkMode {
    zone: Zone,
    told_no_place: bool,
    /// The schedule's tick, armed while there is a schedule, and the keys it was armed for.
    timer: Option<(RegistrationToken, (ModeConfig, NightLightConfig))>,
}

/// Whether `cfg`'s schedule is dark at `now`, and the next time that can change; None without a
/// schedule.
fn scheduled(
    cfg: &Config,
    dm: &mut DarkMode,
    now: i64,
    clock: &dyn LocalClock,
) -> Option<(bool, Option<i64>)> {
    let plan = match cfg.mode.mode {
        AppearanceMode::Light | AppearanceMode::Dark => return None,
        AppearanceMode::Night => {
            crate::nightlight::plan_of(&cfg.night_light, &mut dm.zone, &mut dm.told_no_place)
        }
        AppearanceMode::Hours => Plan::Fixed {
            from: cfg.mode.from,
            to: cfg.mode.to,
        },
    };
    Some(schedule::inside(now, &schedule::nights(plan, now, clock)))
}

fn name(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

/// `cfg` (built from `table`) dark or light: rebuilt from `table` with `[appearance] mode` set
/// to it, the schedule kept. `cfg` itself when it is already so or the rebuild fails.
fn with_dark(cfg: Config, table: &toml::Table, dark: bool) -> Config {
    if cfg.dark == dark {
        return cfg;
    }
    let mut t = table.clone();
    let appearance = t
        .entry("appearance")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let Some(a) = appearance.as_table_mut() {
        a.insert("mode".into(), toml::Value::String(name(dark).into()));
    }
    match Config::from_table(t) {
        Ok(mut c) => {
            c.mode = cfg.mode;
            c
        }
        Err(e) => {
            log::warn!("dark mode: the config does not build {}: {e}", name(dark));
            cfg
        }
    }
}

/// `cfg`, read from `table`, with the look its schedule gives now: for the start and each reload.
pub fn apply(dm: &mut DarkMode, cfg: Config, table: &toml::Table) -> Config {
    match scheduled(&cfg, dm, crate::nightlight::now(), &Local) {
        Some((dark, _)) => with_dark(cfg, table, dark),
        None => cfg,
    }
}

/// While the config has a schedule, re-read it at its next edge, and at least each minute so a
/// clock change or a resume is caught. Runs once the backend drives its outputs, and on each
/// reload: a reload that changes the schedule cancels the tick armed for the old one.
pub fn config_changed(state: &mut State) {
    let keys = (state.config.mode, state.config.night_light.clone());
    if let Some((token, armed)) = &state.dark_mode.timer {
        if *armed == keys {
            return;
        }
        state.handle.remove(*token);
        state.dark_mode.timer = None;
    }
    let now = crate::nightlight::now();
    if scheduled(&state.config, &mut state.dark_mode, now, &Local).is_none() {
        return;
    }
    let timer = state
        .handle
        .insert_source(Timer::immediate(), |_, _, state| match tick(state) {
            Some(wait) => TimeoutAction::ToDuration(wait),
            None => {
                state.dark_mode.timer = None;
                TimeoutAction::Drop
            }
        });
    match timer {
        Ok(t) => state.dark_mode.timer = Some((t, keys)),
        Err(e) => log::warn!("dark mode: no timer: {}", e.error),
    }
}

/// Put in the look the schedule gives now if it is not in force; how long until the next look,
/// None without a schedule.
fn tick(state: &mut State) -> Option<std::time::Duration> {
    let now = crate::nightlight::now();
    let minute = (60 - now.rem_euclid(60)).max(1);
    let (dark, next) = scheduled(&state.config, &mut state.dark_mode, now, &Local)?;
    if state.config.dark != dark {
        log::info!("dark mode: {} by the schedule", name(dark));
        let cfg = with_dark(state.config.clone(), &state.config_file, dark);
        crate::configwatch::reload(state, cfg);
    }
    let wait = next.map_or(minute, |t| (t - now).clamp(1, minute));
    Some(std::time::Duration::from_secs(wait as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local time is UTC.
    struct Utc;

    impl LocalClock for Utc {
        fn date(&self, t: i64) -> (i32, i32, i32) {
            (0, 0, t.div_euclid(86_400) as i32)
        }
        fn at(&self, date: (i32, i32, i32), minute: u32) -> i64 {
            date.2 as i64 * 86_400 + minute as i64 * 60
        }
    }

    const DAY0: i64 = 20_000 * 86_400;

    fn read(text: &str) -> (Config, toml::Table) {
        let (cfg, dropped, table) = Config::parse_over(text, &toml::Table::new()).unwrap();
        assert!(dropped.is_empty(), "{dropped:?}");
        (cfg, table)
    }

    const HOURS: &str =
        "[appearance]\nmode = \"hours\"\ndark_from = \"22:00\"\ndark_to = \"06:30\"\n";

    #[test]
    fn only_a_schedule_arms_the_tick_and_a_new_schedule_rearms_it() {
        let mut h = crate::harness::Harness::new();
        config_changed(&mut h.state);
        assert!(h.state.dark_mode.timer.is_none(), "no schedule: no tick");
        h.state.config = read(HOURS).0;
        config_changed(&mut h.state);
        let first = h.state.dark_mode.timer.as_ref().map(|t| t.0);
        assert!(first.is_some(), "set hours: a tick");
        config_changed(&mut h.state);
        assert_eq!(
            h.state.dark_mode.timer.as_ref().map(|t| t.0),
            first,
            "the same schedule keeps its tick"
        );
        h.state.config.mode.from = 21 * 60;
        config_changed(&mut h.state);
        let second = h.state.dark_mode.timer.as_ref().map(|t| t.0);
        assert!(
            second.is_some() && second != first,
            "other hours: a new tick"
        );
        h.state.config = read("[appearance]\nmode = \"light\"\n").0;
        config_changed(&mut h.state);
        assert!(
            h.state.dark_mode.timer.is_none(),
            "a fixed mode cancels the tick"
        );
    }

    #[test]
    fn set_hours_are_dark_across_midnight_and_name_the_next_edge() {
        let (cfg, _) = read(HOURS);
        let mut dm = DarkMode::default();
        let at =
            |dm: &mut DarkMode, h: i64, m: i64| scheduled(&cfg, dm, DAY0 + h * 3600 + m * 60, &Utc);
        assert_eq!(at(&mut dm, 21, 59), Some((false, Some(DAY0 + 22 * 3600))));
        assert_eq!(
            at(&mut dm, 22, 0),
            Some((true, Some(DAY0 + 86_400 + 6 * 3600 + 1800)))
        );
        assert_eq!(at(&mut dm, 3, 0).map(|s| s.0), Some(true));
        assert_eq!(at(&mut dm, 6, 30).map(|s| s.0), Some(false));
    }

    #[test]
    fn night_follows_night_lights_hours_even_with_night_light_off() {
        let (cfg, _) = read(
            "[appearance]\nmode = \"night\"\n\
             [night_light]\nenabled = false\nschedule = \"fixed\"\nfrom = \"20:00\"\nto = \"08:00\"\n",
        );
        let mut dm = DarkMode::default();
        let mut dark = |h: i64| scheduled(&cfg, &mut dm, DAY0 + h * 3600, &Utc).map(|s| s.0);
        assert_eq!(dark(19), Some(false));
        assert_eq!(dark(21), Some(true));
    }

    #[test]
    fn a_fixed_mode_has_no_schedule_and_the_old_glass_keys_still_schedule_glass() {
        let mut dm = DarkMode::default();
        let (fixed, _) = read("[appearance]\nmode = \"dark\"\n");
        assert_eq!(scheduled(&fixed, &mut dm, DAY0, &Utc), None);
        let (old, _) = read("[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"hours\"\n");
        assert_eq!(
            scheduled(&old, &mut dm, DAY0 + 3600, &Utc).map(|s| s.0),
            Some(true)
        );
    }

    #[test]
    fn the_look_in_force_rebuilds_every_interface_and_keeps_the_schedule() {
        for interface in ["classic", "blue", "glass"] {
            let (cfg, table) = read(&format!("{HOURS}interface = \"{interface}\"\n"));
            assert!(!cfg.dark);
            let dark = with_dark(cfg.clone(), &table, true);
            assert!(dark.dark, "{interface}");
            assert_eq!(dark.mode, cfg.mode, "the schedule stays");
            assert_eq!(dark.glass.mode, reprisekit::glass::Mode::Dark);
            let back = with_dark(dark, &table, false);
            assert_eq!(back, cfg, "{interface}");
        }
        let (classic, table) = read(HOURS);
        assert!(
            with_dark(classic, &table, true).theme.colors.is_dark(),
            "Classic's colours, and so the portal's color-scheme, follow"
        );
    }
}
