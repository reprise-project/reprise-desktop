//! Where a time zone's principal city lies (tzdata's `zone1970.tab` and `zone.tab`), and which
//! zone a place is in, from the towns in `tzcity/cities.txt` (`scripts/tzcities.py`).

use std::path::Path;

const ZONEINFO: &str = "/usr/share/zoneinfo";

/// The zone name `/etc/localtime` links to ("Europe/Budapest").
pub fn system_zone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    zone_name(&target.to_string_lossy())
}

fn zone_name(path: &str) -> Option<String> {
    let (_, zone) = path.split_once("zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_string())
}

/// Latitude and longitude of `zone`, in degrees north and east.
pub fn coordinates(zone: &str) -> Option<(f64, f64)> {
    ["zone1970.tab", "zone.tab"].iter().find_map(|f| {
        let text = std::fs::read_to_string(Path::new(ZONEINFO).join(f)).ok()?;
        lookup(&text, zone)
    })
}

fn lookup(table: &str, zone: &str) -> Option<(f64, f64)> {
    entries(table).find(|(z, _)| *z == zone).map(|(_, at)| at)
}

/// Each zone and its towns of 15,000 people or more, thinned away from the zone's edges, as
/// `zone lat,lon lat,lon ...` in hundredths of a degree.
const TOWNS: &str = include_str!("tzcity/cities.txt");

/// The zone of the town nearest to `at` (degrees north and east). Should the installed tzdata
/// lack that zone, the zone whose principal city in `zone.tab` lies nearest.
pub fn nearest(at: (f64, f64)) -> Option<String> {
    let installed = |z: &String| Path::new(ZONEINFO).join(z).is_file();
    nearest_in(towns(TOWNS), at).filter(installed).or_else(|| {
        let text = std::fs::read_to_string(Path::new(ZONEINFO).join("zone.tab")).ok()?;
        nearest_in(entries(&text), at)
    })
}

fn nearest_in<'a>(
    places: impl Iterator<Item = (&'a str, (f64, f64))>,
    at: (f64, f64),
) -> Option<String> {
    places
        .map(|(zone, place)| (zone, distance(place, at)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(zone, _)| zone.to_string())
}

fn towns(table: &str) -> impl Iterator<Item = (&str, (f64, f64))> {
    table.lines().flat_map(|l| {
        let mut cols = l.split(' ');
        let zone = cols.next().unwrap_or_default();
        cols.filter_map(move |c| {
            let (lat, lon) = c.split_once(',')?;
            let hundredths = |s: &str| s.parse::<i32>().ok().map(|v| f64::from(v) / 100.0);
            Some((zone, (hundredths(lat)?, hundredths(lon)?)))
        })
    })
}

/// Each row's zone and its city's coordinates.
fn entries(table: &str) -> impl Iterator<Item = (&str, (f64, f64))> {
    table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut cols = l.split('\t').skip(1);
            let at = iso6709(cols.next()?)?;
            Some((cols.next()?, at))
        })
}

/// The angle between two places seen from the earth's centre, in radians (haversine).
fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (
        a.0.to_radians(),
        a.1.to_radians(),
        b.0.to_radians(),
        b.1.to_radians(),
    );
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * h.sqrt().min(1.0).asin()
}

/// `+4730+01905` or `+404251-0740023`: sign, degrees, minutes and optional seconds, latitude then
/// longitude.
fn iso6709(s: &str) -> Option<(f64, f64)> {
    let split = s.get(1..)?.find(['+', '-'])? + 1;
    let (lat, lon) = s.split_at(split);
    Some((angle(lat, 2)?, angle(lon, 3)?))
}

fn angle(s: &str, deg: usize) -> Option<f64> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = &s[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() < deg + 2 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| digits.get(r)?.parse::<f64>().ok();
    let d = num(0..deg)?;
    let m = num(deg..deg + 2)?;
    let sec = if digits.len() >= deg + 4 {
        num(deg + 2..deg + 4)?
    } else {
        0.0
    };
    Some(sign * (d + m / 60.0 + sec / 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
# tzdb timezone descriptions
HU,SK\t+4730+01905\tEurope/Budapest
US\t+404251-0740023\tAmerica/New_York\tEastern (most areas)
AU\t-3352+15113\tAustralia/Sydney\tNew South Wales (most areas)
";

    #[test]
    fn a_zone_reads_its_principal_citys_coordinates() {
        let near =
            |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        let bp = lookup(TABLE, "Europe/Budapest").unwrap();
        assert!(near(bp, (47.5, 19.0 + 5.0 / 60.0)), "{bp:?}");
        let ny = lookup(TABLE, "America/New_York").unwrap();
        assert!(
            near(
                ny,
                (40.0 + 42.0 / 60.0 + 51.0 / 3600.0, -(74.0 + 23.0 / 3600.0))
            ),
            "{ny:?}"
        );
        let syd = lookup(TABLE, "Australia/Sydney").unwrap();
        assert!(
            near(syd, (-(33.0 + 52.0 / 60.0), 151.0 + 13.0 / 60.0)),
            "{syd:?}"
        );
        assert_eq!(lookup(TABLE, "Europe/Nowhere"), None);
        assert_eq!(lookup(TABLE, "Europe"), None);
    }

    #[test]
    fn the_nearest_principal_city_names_the_zone() {
        let nearest = |at| nearest_in(entries(TABLE), at);
        assert_eq!(
            nearest((48.2, 16.37)).as_deref(),
            Some("Europe/Budapest"),
            "Vienna is nearest Budapest"
        );
        assert_eq!(
            nearest((42.36, -71.06)).as_deref(),
            Some("America/New_York"),
            "Boston"
        );
        assert_eq!(
            nearest((-37.81, 144.96)).as_deref(),
            Some("Australia/Sydney"),
            "Melbourne"
        );
        assert_eq!(nearest_in(entries("# nothing\n"), (0.0, 0.0)), None);
    }

    #[test]
    fn a_town_near_a_border_gets_its_own_countrys_zone() {
        let places = [
            ("Lviv", (49.84, 24.03), "Europe/Kyiv"),
            ("Vigo", (42.23, -8.72), "Europe/Madrid"),
            ("Strasbourg", (48.58, 7.75), "Europe/Paris"),
            ("Kehl", (48.57, 7.82), "Europe/Berlin"),
            ("Geneva", (46.20, 6.15), "Europe/Zurich"),
            ("Annemasse", (46.19, 6.24), "Europe/Paris"),
            ("El Paso", (31.76, -106.49), "America/Denver"),
            ("Ciudad Juarez", (31.72, -106.46), "America/Ciudad_Juarez"),
            ("Windsor", (42.30, -83.02), "America/Toronto"),
            ("Detroit", (42.33, -83.05), "America/Detroit"),
            ("Budapest", (47.50, 19.04), "Europe/Budapest"),
            ("New York", (40.71, -74.01), "America/New_York"),
            ("Sydney", (-33.87, 151.21), "Australia/Sydney"),
            ("Tokyo", (35.69, 139.69), "Asia/Tokyo"),
            ("Mumbai", (19.08, 72.88), "Asia/Kolkata"),
            ("Sao Paulo", (-23.55, -46.63), "America/Sao_Paulo"),
        ];
        for (town, at, zone) in places {
            assert_eq!(
                nearest_in(towns(TOWNS), at).as_deref(),
                Some(zone),
                "{town}"
            );
        }
    }

    #[test]
    fn the_distance_wraps_at_the_date_line() {
        let fiji = (-18.13, 178.42);
        let samoa = (-13.83, -171.76);
        let perth = (-31.95, 115.86);
        assert!(distance(fiji, samoa) < distance(fiji, perth));
    }

    #[test]
    fn the_zone_name_is_the_path_after_zoneinfo() {
        assert_eq!(
            zone_name("/usr/share/zoneinfo/Europe/Budapest").as_deref(),
            Some("Europe/Budapest")
        );
        assert_eq!(
            zone_name("../usr/share/zoneinfo/UTC").as_deref(),
            Some("UTC")
        );
        assert_eq!(zone_name("/etc/other"), None);
    }
}
