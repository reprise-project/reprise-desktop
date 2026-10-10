//! Where the machine is, as far as the time zone says: the zone's principal city in tzdata's
//! `zone1970.tab` (then `zone.tab`, which also lists the zones that are links today).

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
    table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|cols| cols.get(2) == Some(&zone))
        .and_then(|cols| iso6709(cols[1]))
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
