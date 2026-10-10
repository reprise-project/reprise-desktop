//! Sunrise and sunset from a latitude and longitude, with NOAA's solar
//! calculator equations (good to about a minute between the polar
//! circles). Times are Unix seconds; a day is a solar day index, the
//! count of days since the epoch whose solar noon falls on it at that
//! longitude, so no time zone enters the sums.

/// What the sun does on one solar day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sun {
    Normal {
        rise: i64,
        set: i64,
    },
    /// Above the horizon all day.
    PolarDay,
    /// Below the horizon all day.
    PolarNight,
}

const DAY: i64 = 86_400;

pub fn day_of(t: i64, lon: f64) -> i64 {
    (t + (lon * 240.0) as i64).div_euclid(DAY)
}

fn position(t: i64) -> (f64, f64) {
    let jd = t as f64 / DAY as f64 + 2440587.5;
    let c = (jd - 2451545.0) / 36525.0;
    let l0 = (280.46646 + c * (36000.76983 + c * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + c * (35999.05029 - 0.0001537 * c);
    let e = 0.016708634 - c * (0.000042037 + 0.0000001267 * c);
    let mr = m.to_radians();
    let center = mr.sin() * (1.914602 - c * (0.004817 + 0.000014 * c))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * c)
        + (3.0 * mr).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * c).to_radians();
    let lambda = (l0 + center - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let eps0 = 23.0 + (26.0 + (21.448 - c * (46.815 + c * (0.00059 - c * 0.001813))) / 60.0) / 60.0;
    let eps = (eps0 + 0.00256 * omega.cos()).to_radians();
    let decl = (eps.sin() * lambda.sin()).asin();
    let y = (eps / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eot = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    (decl, eot)
}

/// The cosine of the hour angle at which the sun's centre is 0.833
/// degrees below the horizon (refraction plus its radius).
fn cos_hour_angle(lat: f64, decl: f64) -> f64 {
    let lat = lat.to_radians();
    90.833f64.to_radians().cos() / (lat.cos() * decl.cos()) - lat.tan() * decl.tan()
}

/// Solar noon of `day` at `lon`, and the rise (`sign` -1) or set (+1)
/// with the sun's position taken at `at`; `None` when the sun does not
/// cross the horizon then.
fn event(day: i64, lat: f64, lon: f64, at: i64, sign: f64) -> Result<i64, Sun> {
    let (decl, eot) = position(at);
    let noon = day * DAY + ((720.0 - 4.0 * lon - eot) * 60.0) as i64;
    let cos_ha = cos_hour_angle(lat, decl);
    if cos_ha > 1.0 {
        return Err(Sun::PolarNight);
    }
    if cos_ha < -1.0 {
        return Err(Sun::PolarDay);
    }
    let ha = cos_ha.acos().to_degrees();
    Ok(noon + (sign * ha * 240.0) as i64)
}

/// Sunrise and sunset of solar day `day` at `lat`, `lon` (degrees, north
/// and east positive).
pub fn sun(day: i64, lat: f64, lon: f64) -> Sun {
    let noon = day * DAY + ((720.0 - 4.0 * lon) * 60.0) as i64;
    let first = |sign| event(day, lat, lon, noon, sign);
    let (rise, set) = match (first(-1.0), first(1.0)) {
        (Ok(r), Ok(s)) => (r, s),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    // Again with the sun where it is at each event, not at noon.
    match (
        event(day, lat, lon, rise, -1.0),
        event(day, lat, lon, set, 1.0),
    ) {
        (Ok(rise), Ok(set)) => Sun::Normal { rise, set },
        (Err(e), _) | (_, Err(e)) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
        // Days from civil (Howard Hinnant's algorithm).
        let y = if mo <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (mo + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * DAY + h * 3600 + mi * 60
    }

    /// Rise and set at a place on a date, against published times (UTC),
    /// within two minutes.
    fn check(place: &str, lat: f64, lon: f64, rise: i64, set: i64) {
        let day = day_of((rise + set) / 2, lon);
        let Sun::Normal { rise: r, set: s } = sun(day, lat, lon) else {
            panic!("{place}: the sun rises and sets");
        };
        assert!(
            (r - rise).abs() <= 120,
            "{place}: rise off by {} s",
            r - rise
        );
        assert!((s - set).abs() <= 120, "{place}: set off by {} s", s - set);
    }

    #[test]
    fn sunrise_and_sunset_match_published_tables() {
        // London, midsummer: 04:43 and 21:21 BST.
        let (lat, lon) = (51.5074, -0.1278);
        check(
            "London June",
            lat,
            lon,
            utc(2024, 6, 20, 3, 43),
            utc(2024, 6, 20, 20, 21),
        );
        // London, midwinter: 08:04 and 15:53 GMT.
        check(
            "London Dec",
            lat,
            lon,
            utc(2024, 12, 21, 8, 4),
            utc(2024, 12, 21, 15, 53),
        );
        // New York, midwinter: 07:16 and 16:32 EST.
        let (lat, lon) = (40.7128, -74.0060);
        check(
            "New York",
            lat,
            lon,
            utc(2024, 12, 21, 12, 16),
            utc(2024, 12, 21, 21, 32),
        );
        // Sydney, its midwinter: 07:00 and 16:54 AEST.
        let (lat, lon) = (-33.8688, 151.2093);
        check(
            "Sydney",
            lat,
            lon,
            utc(2024, 6, 20, 21, 0),
            utc(2024, 6, 21, 6, 54),
        );
    }

    #[test]
    fn inside_the_arctic_circle_the_sun_stays_up_or_down() {
        let (lat, lon) = (69.6492, 18.9553);
        let june = day_of(utc(2024, 6, 21, 12, 0), lon);
        assert_eq!(sun(june, lat, lon), Sun::PolarDay);
        let dec = day_of(utc(2024, 12, 21, 12, 0), lon);
        assert_eq!(sun(dec, lat, lon), Sun::PolarNight);
    }

    #[test]
    fn the_solar_day_follows_the_longitude() {
        let t = utc(2024, 6, 20, 23, 0);
        // 23:00 UTC is already the next morning in Sydney.
        assert_eq!(day_of(t, 151.2), day_of(t, 0.0) + 1);
        assert_eq!(day_of(t, -74.0), day_of(t, 0.0));
    }
}
