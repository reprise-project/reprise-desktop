//! Unit conversion for the launcher's answer row: `5 km in mi`, `30 c to f`, `3 ft 4 in = cm`.
//! The amount in front of each unit is a `calc` expression. Unit names are case-insensitive, so
//! `mb` and `Mb` are both megabytes; bits are spelled out (`mbit`, `mbps`).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Length,
    Mass,
    Volume,
    Temperature,
    Area,
    Speed,
    Time,
    Data,
    Pressure,
    Energy,
    Power,
}

/// A unit is `factor` base units per one of it, plus `offset` (only temperatures have one):
/// metre, kilogram, litre, kelvin, square metre, metre per second, second, byte, pascal,
/// joule, watt.
struct Unit {
    symbol: &'static str,
    names: &'static [&'static str],
    kind: Kind,
    factor: f64,
    offset: f64,
}

const fn u(symbol: &'static str, names: &'static [&'static str], kind: Kind, factor: f64) -> Unit {
    Unit {
        symbol,
        names,
        kind,
        factor,
        offset: 0.0,
    }
}

const INCH: f64 = 0.0254;
const FOOT: f64 = 0.3048;
const MILE: f64 = 1609.344;
const POUND: f64 = 0.45359237;
const GALLON: f64 = 3.785411784;
const HOUR: f64 = 3600.0;

use Kind::*;

#[rustfmt::skip]
const UNITS: &[Unit] = &[
    u("nm", &["nm", "nanometer", "nanometers", "nanometre", "nanometres"], Length, 1e-9),
    u("µm", &["µm", "um", "micrometer", "micrometers", "micrometre", "micrometres", "micron", "microns"], Length, 1e-6),
    u("mm", &["mm", "millimeter", "millimeters", "millimetre", "millimetres"], Length, 1e-3),
    u("cm", &["cm", "centimeter", "centimeters", "centimetre", "centimetres"], Length, 1e-2),
    u("m", &["m", "meter", "meters", "metre", "metres"], Length, 1.0),
    u("km", &["km", "kilometer", "kilometers", "kilometre", "kilometres"], Length, 1e3),
    u("in", &["in", "inch", "inches"], Length, INCH),
    u("ft", &["ft", "foot", "feet"], Length, FOOT),
    u("yd", &["yd", "yard", "yards"], Length, 0.9144),
    u("mi", &["mi", "mile", "miles"], Length, MILE),
    u("nmi", &["nmi", "nautical mile", "nautical miles"], Length, 1852.0),

    u("mg", &["mg", "milligram", "milligrams"], Mass, 1e-6),
    u("g", &["g", "gram", "grams", "gramme", "grammes"], Mass, 1e-3),
    u("kg", &["kg", "kilo", "kilos", "kilogram", "kilograms"], Mass, 1.0),
    u("t", &["t", "tonne", "tonnes"], Mass, 1e3),
    u("oz", &["oz", "ounce", "ounces"], Mass, POUND / 16.0),
    u("lb", &["lb", "lbs", "pound", "pounds"], Mass, POUND),
    u("st", &["st", "stone", "stones"], Mass, POUND * 14.0),

    u("ml", &["ml", "milliliter", "milliliters", "millilitre", "millilitres"], Volume, 1e-3),
    u("cl", &["cl", "centiliter", "centiliters", "centilitre", "centilitres"], Volume, 1e-2),
    u("dl", &["dl", "deciliter", "deciliters", "decilitre", "decilitres"], Volume, 1e-1),
    u("l", &["l", "liter", "liters", "litre", "litres"], Volume, 1.0),
    u("cm³", &["cm³", "cm3", "cc", "cubic centimeter", "cubic centimeters", "cubic centimetre", "cubic centimetres"], Volume, 1e-3),
    u("m³", &["m³", "m3", "cubic meter", "cubic meters", "cubic metre", "cubic metres"], Volume, 1e3),
    u("ft³", &["ft³", "ft3", "cu ft", "cubic foot", "cubic feet"], Volume, FOOT * FOOT * FOOT * 1e3),
    u("tsp", &["tsp", "teaspoon", "teaspoons"], Volume, GALLON / 768.0),
    u("tbsp", &["tbsp", "tablespoon", "tablespoons"], Volume, GALLON / 256.0),
    u("fl oz", &["fl oz", "floz", "fluid ounce", "fluid ounces"], Volume, GALLON / 128.0),
    u("cup", &["cup", "cups"], Volume, GALLON / 16.0),
    u("pt", &["pt", "pint", "pints"], Volume, GALLON / 8.0),
    u("qt", &["qt", "quart", "quarts"], Volume, GALLON / 4.0),
    u("gal", &["gal", "gallon", "gallons"], Volume, GALLON),

    Unit { symbol: "°C", names: &["°c", "c", "celsius", "degc", "deg c", "degrees celsius"], kind: Temperature, factor: 1.0, offset: 273.15 },
    Unit { symbol: "°F", names: &["°f", "f", "fahrenheit", "degf", "deg f", "degrees fahrenheit"], kind: Temperature, factor: 5.0 / 9.0, offset: 459.67 * 5.0 / 9.0 },
    u("K", &["k", "kelvin", "kelvins"], Temperature, 1.0),

    u("cm²", &["cm²", "cm2", "sq cm", "square centimeter", "square centimeters", "square centimetre", "square centimetres"], Area, 1e-4),
    u("m²", &["m²", "m2", "sq m", "square meter", "square meters", "square metre", "square metres"], Area, 1.0),
    u("km²", &["km²", "km2", "sq km", "square kilometer", "square kilometers", "square kilometre", "square kilometres"], Area, 1e6),
    u("in²", &["in²", "in2", "sq in", "square inch", "square inches"], Area, INCH * INCH),
    u("ft²", &["ft²", "ft2", "sq ft", "square foot", "square feet"], Area, FOOT * FOOT),
    u("mi²", &["mi²", "mi2", "sq mi", "square mile", "square miles"], Area, MILE * MILE),
    u("ha", &["ha", "hectare", "hectares"], Area, 1e4),
    u("acre", &["acre", "acres", "ac"], Area, 4046.8564224),

    u("m/s", &["m/s", "mps", "meters per second", "metres per second"], Speed, 1.0),
    u("km/h", &["km/h", "kmh", "kph", "kmph", "kilometers per hour", "kilometres per hour"], Speed, 1e3 / HOUR),
    u("mph", &["mph", "mi/h", "miles per hour"], Speed, MILE / HOUR),
    u("ft/s", &["ft/s", "fps", "feet per second"], Speed, FOOT),
    u("kn", &["kn", "kt", "knot", "knots"], Speed, 1852.0 / HOUR),

    u("ms", &["ms", "millisecond", "milliseconds"], Time, 1e-3),
    u("s", &["s", "sec", "secs", "second", "seconds"], Time, 1.0),
    u("min", &["min", "mins", "minute", "minutes"], Time, 60.0),
    u("h", &["h", "hr", "hrs", "hour", "hours"], Time, HOUR),
    u("d", &["d", "day", "days"], Time, 24.0 * HOUR),
    u("wk", &["wk", "week", "weeks"], Time, 7.0 * 24.0 * HOUR),
    u("yr", &["yr", "yrs", "year", "years"], Time, 365.25 * 24.0 * HOUR),

    u("bit", &["bit", "bits"], Data, 0.125),
    u("kbit", &["kbit", "kbits", "kbps", "kilobit", "kilobits"], Data, 125.0),
    u("Mbit", &["mbit", "mbits", "mbps", "megabit", "megabits"], Data, 125e3),
    u("Gbit", &["gbit", "gbits", "gbps", "gigabit", "gigabits"], Data, 125e6),
    u("B", &["b", "byte", "bytes"], Data, 1.0),
    u("kB", &["kb", "kilobyte", "kilobytes"], Data, 1e3),
    u("MB", &["mb", "megabyte", "megabytes"], Data, 1e6),
    u("GB", &["gb", "gigabyte", "gigabytes"], Data, 1e9),
    u("TB", &["tb", "terabyte", "terabytes"], Data, 1e12),
    u("PB", &["pb", "petabyte", "petabytes"], Data, 1e15),
    u("KiB", &["kib", "kibibyte", "kibibytes"], Data, 1024.0),
    u("MiB", &["mib", "mebibyte", "mebibytes"], Data, 1048576.0),
    u("GiB", &["gib", "gibibyte", "gibibytes"], Data, 1073741824.0),
    u("TiB", &["tib", "tebibyte", "tebibytes"], Data, 1099511627776.0),

    u("Pa", &["pa", "pascal", "pascals"], Pressure, 1.0),
    u("hPa", &["hpa", "hectopascal", "hectopascals"], Pressure, 1e2),
    u("kPa", &["kpa", "kilopascal", "kilopascals"], Pressure, 1e3),
    u("mbar", &["mbar", "millibar", "millibars"], Pressure, 1e2),
    u("bar", &["bar", "bars"], Pressure, 1e5),
    u("psi", &["psi"], Pressure, 6894.757293168361),
    u("atm", &["atm", "atmosphere", "atmospheres"], Pressure, 101325.0),
    u("mmHg", &["mmhg"], Pressure, 133.322387415),

    u("J", &["j", "joule", "joules"], Energy, 1.0),
    u("kJ", &["kj", "kilojoule", "kilojoules"], Energy, 1e3),
    u("cal", &["cal", "calorie", "calories"], Energy, 4.184),
    u("kcal", &["kcal", "kilocalorie", "kilocalories"], Energy, 4184.0),
    u("Wh", &["wh", "watt hour", "watt hours"], Energy, HOUR),
    u("kWh", &["kwh", "kilowatt hour", "kilowatt hours"], Energy, HOUR * 1e3),

    u("W", &["w", "watt", "watts"], Power, 1.0),
    u("kW", &["kw", "kilowatt", "kilowatts"], Power, 1e3),
    u("hp", &["hp", "horsepower"], Power, 745.699_871_582_270_2),
];

/// The answer row's text and the number Enter copies: `("5 km = 3.10686 mi", "3.10686")`.
/// None unless `text` is amounts with units, a separator (`in`, `to`, `as`, `=`) and one
/// unit of the same kind.
pub fn answer(text: &str) -> Option<(String, String)> {
    let text = text.trim().to_lowercase().replace('=', " = ");
    let words: Vec<&str> = text.split_whitespace().collect();
    // The last separator whose two sides both read wins, so in `5 in in cm` the second `in`
    // separates and the first is inches.
    (1..words.len().saturating_sub(1)).rev().find_map(|i| {
        if !matches!(words[i], "in" | "to" | "as" | "=") {
            return None;
        }
        let to = unit_named(&words[i + 1..].join(" "))?;
        let from = amounts(&words[..i].join(" "))?;
        let kind = from[0].1.kind;
        let temperature = kind == Temperature;
        if to.kind != kind
            || from.iter().any(|(_, u)| u.kind != kind)
            || (temperature && from.len() > 1)
        {
            return None;
        }
        let base: f64 = from.iter().map(|(v, u)| v * u.factor + u.offset).sum();
        let result = (base - to.offset) / to.factor;
        if !result.is_finite() {
            return None;
        }
        let number = crate::calc::format_sig(result, 6);
        let shown: Vec<String> = (from.iter())
            .map(|(v, u)| format!("{} {}", crate::calc::format(*v), u.symbol))
            .collect();
        Some((
            format!("{} = {number} {}", shown.join(" "), to.symbol),
            number,
        ))
    })
}

fn unit_named(name: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|u| u.names.contains(&name))
}

/// The longest unit name `rest` starts with, ending at a word boundary, and its length.
fn unit_at(rest: &str) -> Option<(&'static Unit, usize)> {
    UNITS
        .iter()
        .flat_map(|u| u.names.iter().map(move |n| (u, *n)))
        .filter(|(_, n)| {
            rest.starts_with(n) && !rest[n.len()..].starts_with(|c: char| c.is_alphanumeric())
        })
        .map(|(u, n)| (u, n.len()))
        .max_by_key(|&(_, len)| len)
}

/// `2*3 km`, `3 ft 4 in`: each unit with the expression in front of it.
fn amounts(text: &str) -> Option<Vec<(f64, &'static Unit)>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let c = rest.chars().next()?;
        let word_start = !text[..at].ends_with(|p: char| p.is_alphabetic());
        if word_start
            && (c.is_alphabetic() || c == '°')
            && let Some((unit, len)) = unit_at(rest)
        {
            out.push((crate::calc::value(&text[start..at])?, unit));
            at += len;
            start = at;
            continue;
        }
        at += c.len_utf8();
    }
    (!out.is_empty() && text[start..].trim().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_read_and_round_to_six_digits() {
        for (text, row, copied) in [
            ("5 km in mi", "5 km = 3.10686 mi", "3.10686"),
            ("5km to miles", "5 km = 3.10686 mi", "3.10686"),
            ("1 mile = km", "1 mi = 1.60934 km", "1.60934"),
            ("500 g in lb", "500 g = 1.10231 lb", "1.10231"),
            ("1.5 h in min", "1.5 h = 90 min", "90"),
            ("100 kmh in mph", "100 km/h = 62.1371 mph", "62.1371"),
            ("10 m/s as km/h", "10 m/s = 36 km/h", "36"),
            ("1 gallon in l", "1 gal = 3.78541 l", "3.78541"),
            ("2 tbsp in ml", "2 tbsp = 29.5735 ml", "29.5735"),
            ("1 acre in square meters", "1 acre = 4046.86 m²", "4046.86"),
            ("1 ha in sq ft", "1 ha = 107639 ft²", "107639"),
            ("1 atm in psi", "1 atm = 14.6959 psi", "14.6959"),
            ("1 kWh in kJ", "1 kWh = 3600 kJ", "3600"),
            ("1 hp in kw", "1 hp = 0.7457 kW", "0.7457"),
            ("1 nmi in km", "1 nmi = 1.852 km", "1.852"),
            ("2 weeks in days", "2 wk = 14 d", "14"),
        ] {
            let want = (row.to_string(), copied.to_string());
            assert_eq!(answer(text), Some(want), "{text:?}");
        }
    }

    #[test]
    fn temperatures_shift_as_well_as_scale() {
        for (text, copied) in [
            ("30 c in f", "86"),
            ("-40 °C to °F", "-40"),
            ("212 fahrenheit in celsius", "100"),
            ("0 k in c", "-273.15"),
            ("98.6 f in c", "37"),
            ("20 c in k", "293.15"),
        ] {
            assert_eq!(
                answer(text).map(|a| a.1).as_deref(),
                Some(copied),
                "{text:?}"
            );
        }
        assert_eq!(answer("30 c in f").unwrap().0, "30 °C = 86 °F");
        assert_eq!(answer("1 c 2 c in f"), None, "temperatures do not add");
    }

    #[test]
    fn decimal_and_binary_bytes_and_bits_differ() {
        for (text, copied) in [
            ("2 gib in mb", "2147.48"),
            ("2 GB in GiB", "1.86265"),
            ("1 MiB in KiB", "1024"),
            ("1 mb in kb", "1000"),
            ("1 Mb in kB", "1000"),
            ("100 mbps in MB", "12.5"),
            ("8 bits in bytes", "1"),
            ("1 TB in GiB", "931.323"),
        ] {
            assert_eq!(
                answer(text).map(|a| a.1).as_deref(),
                Some(copied),
                "{text:?}"
            );
        }
    }

    #[test]
    fn in_is_inches_or_the_separator_by_position() {
        assert_eq!(answer("5 in in cm").unwrap().0, "5 in = 12.7 cm");
        assert_eq!(answer("12.7 cm in in").unwrap().0, "12.7 cm = 5 in");
        assert_eq!(answer("10 in to cm").unwrap().1, "25.4");
        assert_eq!(answer("3 ft 4 in to cm").unwrap().0, "3 ft 4 in = 101.6 cm");
        assert_eq!(answer("3 ft 4 in in cm").unwrap().1, "101.6");
        assert_eq!(answer("1 h 30 min in min").unwrap().1, "90");
    }

    #[test]
    fn an_expression_in_front_of_a_unit_is_worked_out() {
        assert_eq!(answer("2*3 km in m").unwrap().0, "6 km = 6000 m");
        assert_eq!(answer("(1+1) h in min").unwrap().1, "120");
        assert_eq!(answer("sqrt(16) ft in in").unwrap().1, "48");
        assert_eq!(
            answer("1/3 mi in yd").unwrap().0,
            "0.333333333333 mi = 586.667 yd"
        );
    }

    #[test]
    fn mismatched_unknown_or_incomplete_text_has_no_row() {
        for text in [
            "5 km in kg",
            "30 c in mb",
            "5 km in",
            "km in mi",
            "5 in",
            "5 km",
            "5 km 3 in mi",
            "5 km in mi km",
            "5 parsecs in km",
            "5 km in lightyears",
            "how to",
            "notes in documents",
            "1,5 km in m",
            "0 m in mm extra",
            "1 kb in kg",
        ] {
            assert_eq!(answer(text), None, "{text:?}");
        }
    }
}
