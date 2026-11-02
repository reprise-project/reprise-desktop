//! Glass's dark mode on a schedule (`[glass] mode = "night"` or `"hours"`). The config keeps the
//! schedule; the mode in force is the config rebuilt with `mode` set to "dark" or "light", put in
//! through `configwatch::reload` as an edit of the file would be, so the look, the programs' look
//! and the portal's color-scheme change alike.

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::config::{Config, DarkSchedule};
use crate::nightlight::schedule::{self, LocalClock, Plan};
use crate::nightlight::{Local, Zone};
use reprisekit::glass::Mode;
use reprisekit::interface::Interface;

#[derive(Default)]
pub struct DarkMode {
    zone: Zone,
    told_no_place: bool,
}

/// The mode `cfg`'s schedule gives at `now` and the next time that can change; None without a
/// schedule or outside Glass.
fn scheduled(
    cfg: &Config,
    dm: &mut DarkMode,
    now: i64,
    clock: &dyn LocalClock,
) -> Option<(Mode, Option<i64>)> {
    if cfg.interface != Interface::Glass {
        return None;
    }
    let plan = match cfg.glass_dark.schedule {
        DarkSchedule::Off => return None,
        DarkSchedule::Night => {
            crate::nightlight::plan_of(&cfg.night_light, &mut dm.zone, &mut dm.told_no_place)
        }
        DarkSchedule::Hours => Plan::Fixed {
            from: cfg.glass_dark.from,
            to: cfg.glass_dark.to,
        },
    };
    let (dark, next) = schedule::inside(now, &schedule::nights(plan, now, clock));
    Some((if dark { Mode::Dark } else { Mode::Light }, next))
}

/// `cfg` (built from `table`) with `mode` in force: rebuilt from `table` with `[glass] mode` set to
/// it, the schedule kept. `cfg` itself when it has the mode already or the rebuild fails.
fn with_mode(cfg: Config, table: &toml::Table, mode: Mode) -> Config {
    if cfg.glass.mode == mode {
        return cfg;
    }
    let mut t = table.clone();
    let glass = t
        .entry("glass")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let Some(g) = glass.as_table_mut() {
        g.insert("mode".into(), toml::Value::String(mode.as_str().into()));
    }
    match Config::from_table(t) {
        Ok(mut c) => {
            c.glass_dark = cfg.glass_dark;
            c
        }
        Err(e) => {
            log::warn!("dark mode: the config does not build in {mode:?} mode: {e}");
            cfg
        }
    }
}

/// `cfg`, read from `table`, with the mode its schedule gives now: for the start and each reload.
pub fn apply(dm: &mut DarkMode, cfg: Config, table: &toml::Table) -> Config {
    match scheduled(&cfg, dm, crate::nightlight::now(), &Local) {
        Some((mode, _)) => with_mode(cfg, table, mode),
        None => cfg,
    }
}

/// Re-read the schedule at its next edge, and at least each minute so a clock change or a resume
/// is caught. Runs once the backend drives its outputs.
pub fn setup(state: &mut State) {
    let first = tick(state);
    let timer = state
        .handle
        .insert_source(Timer::from_duration(first), |_, _, state| {
            TimeoutAction::ToDuration(tick(state))
        });
    if let Err(e) = timer {
        log::warn!("dark mode: no timer: {}", e.error);
    }
}

/// Put in the mode the schedule gives now if it is not in force; how long until the next look.
fn tick(state: &mut State) -> std::time::Duration {
    let now = crate::nightlight::now();
    let minute = (60 - now.rem_euclid(60)).max(1);
    let Some((mode, next)) = scheduled(&state.config, &mut state.dark_mode, now, &Local) else {
        return std::time::Duration::from_secs(minute as u64);
    };
    if state.config.glass.mode != mode {
        log::info!("dark mode: {} by the schedule", mode.as_str());
        let cfg = with_mode(state.config.clone(), &state.config_file, mode);
        crate::configwatch::reload(state, cfg);
    }
    let wait = next.map_or(minute, |t| (t - now).clamp(1, minute));
    std::time::Duration::from_secs(wait as u64)
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

    const HOURS: &str = "[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"hours\"\ndark_from = \"22:00\"\ndark_to = \"06:30\"\n";

    #[test]
    fn set_hours_are_dark_across_midnight_and_name_the_next_edge() {
        let (cfg, _) = read(HOURS);
        let mut dm = DarkMode::default();
        let at =
            |dm: &mut DarkMode, h: i64, m: i64| scheduled(&cfg, dm, DAY0 + h * 3600 + m * 60, &Utc);
        assert_eq!(
            at(&mut dm, 21, 59),
            Some((Mode::Light, Some(DAY0 + 22 * 3600)))
        );
        assert_eq!(
            at(&mut dm, 22, 0),
            Some((Mode::Dark, Some(DAY0 + 86_400 + 6 * 3600 + 1800)))
        );
        assert_eq!(at(&mut dm, 3, 0).map(|s| s.0), Some(Mode::Dark));
        assert_eq!(at(&mut dm, 6, 30).map(|s| s.0), Some(Mode::Light));
    }

    #[test]
    fn night_follows_night_lights_hours_even_with_night_light_off() {
        let (cfg, _) = read(
            "[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"night\"\n\
             [night_light]\nenabled = false\nschedule = \"fixed\"\nfrom = \"20:00\"\nto = \"08:00\"\n",
        );
        let mut dm = DarkMode::default();
        let mut mode = |h: i64| scheduled(&cfg, &mut dm, DAY0 + h * 3600, &Utc).map(|s| s.0);
        assert_eq!(mode(19), Some(Mode::Light));
        assert_eq!(mode(21), Some(Mode::Dark));
    }

    #[test]
    fn no_schedule_or_another_interface_leaves_the_mode_alone() {
        let mut dm = DarkMode::default();
        let (fixed, _) = read("[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"dark\"\n");
        assert_eq!(scheduled(&fixed, &mut dm, DAY0, &Utc), None);
        let (classic, _) = read("[glass]\nmode = \"hours\"\n");
        assert_eq!(scheduled(&classic, &mut dm, DAY0, &Utc), None);
    }

    #[test]
    fn the_mode_in_force_rebuilds_the_look_and_keeps_the_schedule() {
        let (cfg, table) = read(HOURS);
        assert_eq!(cfg.glass.mode, Mode::Light);
        let light_scheme = cfg.theme.colors.is_dark();
        let dark = with_mode(cfg.clone(), &table, Mode::Dark);
        assert_eq!(dark.glass.mode, Mode::Dark);
        assert_eq!(dark.glass_dark, cfg.glass_dark, "the schedule stays");
        assert!(
            dark.theme.colors.is_dark() && !light_scheme,
            "the colours, and so the portal's color-scheme, follow"
        );
        let back = with_mode(dark, &table, Mode::Light);
        assert_eq!(back, cfg);
    }
}
