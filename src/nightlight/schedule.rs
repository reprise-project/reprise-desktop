//! When the screen is warm: the nights around a moment, from sunset to
//! sunrise or between two fixed times of day, and how far into its fade
//! a moment is. Each edge fades over [`FADE`] seconds centred on it, so
//! a 21:00 start is halfway at 21:00 and fully warm at 21:15. Pure: the
//! caller passes the time and, for fixed times, the local calendar.

use super::color::NEUTRAL;
use super::sun::{Sun, day_of, sun};

pub const FADE: i64 = 30 * 60;
const HALF: i64 = FADE / 2;
const DAY: i64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plan {
    /// Sunset to sunrise at a place, degrees north and east.
    Sun { lat: f64, lon: f64 },
    /// From `from` to `to`, minutes after local midnight; `to` at or
    /// before `from` is the next day's.
    Fixed { from: u32, to: u32 },
}

pub trait LocalClock {
    fn date(&self, t: i64) -> (i32, i32, i32);
    fn at(&self, date: (i32, i32, i32), minute: u32) -> i64;
}

/// The nights overlapping the days around `now`, in order, each
/// `(start, end)` in Unix seconds.
pub fn nights(plan: Plan, now: i64, clock: &dyn LocalClock) -> Vec<(i64, i64)> {
    match plan {
        Plan::Sun { lat, lon } => sun_nights(lat, lon, now),
        Plan::Fixed { from, to } => fixed_nights(from, to, now, clock),
    }
}

fn fixed_nights(from: u32, to: u32, now: i64, clock: &dyn LocalClock) -> Vec<(i64, i64)> {
    if from == to {
        return Vec::new();
    }
    let mut out: Vec<(i64, i64)> = (-2..=2)
        .map(|k| {
            let date = clock.date(now + k * DAY);
            let start = clock.at(date, from);
            // Noon plus a day is the next date even across a DST change.
            let end_date = if to > from {
                date
            } else {
                clock.date(clock.at(date, 12 * 60) + DAY)
            };
            (start, clock.at(end_date, to))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The nights between the days of the solar days around `now`: sunset to
/// sunrise, a polar night's day counting as night whole.
fn sun_nights(lat: f64, lon: f64, now: i64) -> Vec<(i64, i64)> {
    let today = day_of(now, lon);
    let midnight = |d: i64| d * DAY - (lon * 240.0) as i64;
    let first = today - 3;
    let last = today + 3;
    let mut days: Vec<(i64, i64)> = Vec::new();
    for d in first..=last {
        let span = match sun(d, lat, lon) {
            Sun::Normal { rise, set } => (rise, set),
            Sun::PolarDay => (midnight(d), midnight(d + 1)),
            Sun::PolarNight => continue,
        };
        match days.last_mut() {
            Some(prev) if prev.1 >= span.0 => prev.1 = prev.1.max(span.1),
            _ => days.push(span),
        }
    }
    let mut out = Vec::new();
    let mut from = midnight(first);
    for (rise, set) in days {
        if rise > from {
            out.push((from, rise));
        }
        from = set;
    }
    if midnight(last + 1) > from {
        out.push((from, midnight(last + 1)));
    }
    out
}

/// How warm the screen is at `now`, 0 for neutral to 1 for the chosen
/// temperature.
pub fn level(now: i64, nights: &[(i64, i64)]) -> f64 {
    let ramp = |from: i64| ((now - from) as f64 / FADE as f64).clamp(0.0, 1.0);
    nights
        .iter()
        .map(|&(start, end)| ramp(start - HALF).min(1.0 - ramp(end - HALF)))
        .fold(0.0, f64::max)
}

pub fn kelvin(level: f64, target: u32) -> u32 {
    let n = NEUTRAL as f64;
    (n - level * (n - target as f64)).round() as u32
}

/// When a pause taken at `now` ends: once the night now fading in or on
/// (else the next one) has faded out.
pub fn pause_until(now: i64, nights: &[(i64, i64)]) -> i64 {
    nights
        .iter()
        .map(|&(_, end)| end + HALF)
        .find(|&t| t > now)
        .unwrap_or(now + DAY)
}

pub fn next_start(now: i64, nights: &[(i64, i64)]) -> Option<i64> {
    nights
        .iter()
        .map(|&(start, _)| start)
        .find(|&t| t - HALF > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Offset(i64);

    impl LocalClock for Offset {
        fn date(&self, t: i64) -> (i32, i32, i32) {
            // Days since the epoch stand in for the date; only order and
            // the next-day step matter here.
            (0, 0, (t + self.0).div_euclid(DAY) as i32)
        }
        fn at(&self, date: (i32, i32, i32), minute: u32) -> i64 {
            date.2 as i64 * DAY + minute as i64 * 60 - self.0
        }
    }

    const H: i64 = 3600;
    const MIDNIGHT: i64 = 20_000 * DAY - 2 * H;

    fn fixed(from: u32, to: u32, now: i64) -> Vec<(i64, i64)> {
        nights(Plan::Fixed { from, to }, now, &Offset(2 * H))
    }

    #[test]
    fn fixed_times_cross_midnight() {
        let now = MIDNIGHT + 12 * H;
        let n = fixed(21 * 60, 7 * 60, now);
        assert!(
            n.contains(&(MIDNIGHT + 21 * H, MIDNIGHT + DAY + 7 * H)),
            "{n:?}"
        );
        assert!(n.contains(&(MIDNIGHT - 3 * H, MIDNIGHT + 7 * H)), "{n:?}");
        let n = fixed(13 * 60, 14 * 60, now);
        assert!(n.contains(&(MIDNIGHT + 13 * H, MIDNIGHT + 14 * H)), "{n:?}");
        assert!(fixed(600, 600, now).is_empty(), "equal times are no night");
    }

    #[test]
    fn the_fade_is_centred_on_each_edge() {
        let n = fixed(21 * 60, 7 * 60, MIDNIGHT + 12 * H);
        let at = |h: i64, m: i64| level(MIDNIGHT + h * H + m * 60, &n);
        assert_eq!(at(20, 44), 0.0);
        assert_eq!(at(20, 45), 0.0);
        assert_eq!(at(21, 0), 0.5);
        assert!((at(21, 6) - 0.7).abs() < 1e-9);
        assert_eq!(at(21, 15), 1.0);
        assert_eq!(at(26, 0), 1.0, "02:00");
        assert_eq!(at(31, 0), 0.5, "07:00");
        assert_eq!(at(31, 15), 0.0);
        assert_eq!(at(12, 0), 0.0);
    }

    #[test]
    fn a_night_shorter_than_the_fade_never_reaches_full() {
        let n = fixed(21 * 60, 21 * 60 + 10, MIDNIGHT + 12 * H);
        let peak = (0..60)
            .map(|m| level(MIDNIGHT + 20 * H + 30 * 60 + m * 60, &n))
            .fold(0.0, f64::max);
        assert!((peak - (0.5 + 5.0 / 30.0)).abs() < 1e-9, "{peak}");
    }

    #[test]
    fn kelvin_runs_from_neutral_to_the_target() {
        assert_eq!(kelvin(0.0, 3400), 6500);
        assert_eq!(kelvin(1.0, 3400), 3400);
        assert_eq!(kelvin(0.5, 3500), 5000);
    }

    #[test]
    fn a_pause_lasts_until_the_night_has_faded_out() {
        let n = fixed(21 * 60, 7 * 60, MIDNIGHT + 12 * H);
        let tomorrow_7 = MIDNIGHT + DAY + 7 * H + 15 * 60;
        assert_eq!(pause_until(MIDNIGHT + 23 * H, &n), tomorrow_7);
        assert_eq!(pause_until(MIDNIGHT + 20 * H + 50 * 60, &n), tomorrow_7);
        assert_eq!(pause_until(MIDNIGHT + 12 * H, &n), tomorrow_7);
        assert_eq!(
            pause_until(MIDNIGHT + 3 * H, &n),
            MIDNIGHT + 7 * H + 15 * 60
        );
    }

    #[test]
    fn the_next_start_is_the_coming_edge() {
        let n = fixed(21 * 60, 7 * 60, MIDNIGHT + 12 * H);
        assert_eq!(next_start(MIDNIGHT + 12 * H, &n), Some(MIDNIGHT + 21 * H));
        assert_eq!(
            next_start(MIDNIGHT + 21 * H, &n),
            Some(MIDNIGHT + DAY + 21 * H)
        );
    }

    #[test]
    fn sun_nights_run_from_sunset_to_sunrise() {
        let now = 1_718_884_800;
        let (lat, lon) = (51.5074, -0.1278);
        let n = sun_nights(lat, lon, now);
        let Sun::Normal { set, .. } = sun(day_of(now, lon), lat, lon) else {
            panic!("London has a sunset");
        };
        let Sun::Normal { rise, .. } = sun(day_of(now, lon) + 1, lat, lon) else {
            panic!("and a sunrise");
        };
        assert!(n.contains(&(set, rise)), "{n:?}");
        assert_eq!(level(now, &n), 0.0);
        assert_eq!(level(set + 20 * 60, &n), 1.0);
    }

    #[test]
    fn a_polar_night_is_night_all_day_and_a_polar_day_never() {
        let (lat, lon) = (69.6492, 18.9553);
        let dec = 1_734_782_400;
        assert_eq!(level(dec, &sun_nights(lat, lon, dec)), 1.0);
        let june = 1_718_971_200;
        assert_eq!(level(june, &sun_nights(lat, lon, june)), 0.0);
    }
}
