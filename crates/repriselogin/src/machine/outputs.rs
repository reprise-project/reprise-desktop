//! `[[outputs]]` in `MACHINE_CONFIG`: what the login screen runs on each connector, in the keys
//! and spellings a session's `[[outputs]]` takes.

use serde::{Deserialize, Serialize};

/// Most `[[outputs]]` tables the login screen takes; more are refused whole.
pub const MAX_OUTPUTS: usize = 16;
/// How far from the origin an output may sit, either way on each axis.
pub const MAX_POSITION: i32 = 65536;
/// The widest and tallest mode taken, in pixels.
pub const MAX_MODE_SIDE: u32 = 16384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginOutput {
    /// The connector, "DP-1".
    pub name: String,
    pub mode: Option<LoginMode>,
    /// The scale in quarters: 4 is 100%, 16 is 400%.
    pub quarters: u8,
    pub transform: Transform,
    /// The top-left in the layout; `None` places it automatically.
    pub position: Option<(i32, i32)>,
    pub primary: bool,
}

impl LoginOutput {
    /// Connector `name` at its preferred mode and scale 1.
    pub fn named(name: &str) -> LoginOutput {
        LoginOutput {
            name: name.into(),
            mode: None,
            quarters: 4,
            transform: Transform::Normal,
            position: None,
            primary: false,
        }
    }

    pub fn scale(&self) -> f32 {
        f32::from(self.quarters) / 4.0
    }
}

/// `mode = "1920x1080"` or `"1920x1080@59.94"`; the refresh is in mHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: Option<u32>,
}

impl LoginMode {
    pub fn parse(s: &str) -> Result<LoginMode, String> {
        let bad = || format!("\"{s}\" is not WIDTHxHEIGHT[@HZ]");
        let (size, hz) = match s.split_once('@') {
            Some((size, hz)) => (size, Some(hz)),
            None => (s, None),
        };
        let (w, h) = size.split_once('x').ok_or_else(bad)?;
        let side = |v: &str| {
            v.parse::<u32>()
                .ok()
                .filter(|n| (1..=MAX_MODE_SIDE).contains(n))
        };
        let (Some(width), Some(height)) = (side(w), side(h)) else {
            return Err(bad());
        };
        let refresh_mhz = match hz {
            Some(hz) => {
                let hz: f64 = hz.parse().map_err(|_| bad())?;
                if !hz.is_finite() || hz <= 0.0 || hz > 1000.0 {
                    return Err(bad());
                }
                Some((hz * 1000.0).round() as u32)
            }
            None => None,
        };
        Ok(LoginMode {
            width,
            height,
            refresh_mhz,
        })
    }
}

impl std::fmt::Display for LoginMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)?;
        let Some(mhz) = self.refresh_mhz else {
            return Ok(());
        };
        let hz = format!("{}.{:03}", mhz / 1000, mhz % 1000);
        write!(f, "@{}", hz.trim_end_matches('0').trim_end_matches('.'))
    }
}

/// A rotation or flip by its `[[outputs]] transform` name; rotations are counter-clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transform {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "90")]
    Rotate90,
    #[serde(rename = "180")]
    Rotate180,
    #[serde(rename = "270")]
    Rotate270,
    #[serde(rename = "flipped")]
    Flipped,
    #[serde(rename = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    Flipped270,
}

impl Transform {
    pub const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Transform::Normal => "normal",
            Transform::Rotate90 => "90",
            Transform::Rotate180 => "180",
            Transform::Rotate270 => "270",
            Transform::Flipped => "flipped",
            Transform::Flipped90 => "flipped-90",
            Transform::Flipped180 => "flipped-180",
            Transform::Flipped270 => "flipped-270",
        }
    }

    pub fn parse(s: &str) -> Option<Transform> {
        Transform::ALL.into_iter().find(|t| t.name() == s)
    }
}

/// A connector name: letters, digits, `-`, `_` and `.`, at most 32.
fn connector(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// A scale of 1 to 4 in quarter steps, in quarters.
fn quarters(value: &toml::Value) -> Result<u8, String> {
    let s = match *value {
        toml::Value::Float(f) => f,
        toml::Value::Integer(n) => n as f64,
        _ => return Err("expected a number".into()),
    };
    let q = s * 4.0;
    if !(4.0..=16.0).contains(&q) || q.fract() != 0.0 {
        return Err(format!("{s} is not 1 to 4 in steps of 0.25"));
    }
    Ok(q as u8)
}

fn position(value: &toml::Value) -> Result<(i32, i32), String> {
    let expected = || "expected [x, y]".to_string();
    let Some([x, y]) = value.as_array().map(Vec::as_slice) else {
        return Err(expected());
    };
    let (Some(x), Some(y)) = (x.as_integer(), y.as_integer()) else {
        return Err(expected());
    };
    let range = -i64::from(MAX_POSITION)..=i64::from(MAX_POSITION);
    if !range.contains(&x) || !range.contains(&y) {
        return Err(format!("[{x}, {y}] is past {MAX_POSITION} from the origin"));
    }
    Ok((x as i32, y as i32))
}

/// One `[[outputs]]` table, warning for each value it could not use; `None` without a name.
fn output(table: &toml::Table, at: usize, warnings: &mut Vec<String>) -> Option<LoginOutput> {
    let name = match table.get("name").map(|v| v.as_str()) {
        Some(Some(s)) if connector(s) => s,
        Some(Some(s)) => {
            warnings.push(format!(
                "outputs[{at}].name: \"{s}\" is not a connector name"
            ));
            return None;
        }
        Some(None) => {
            warnings.push(format!("outputs[{at}].name: expected a string"));
            return None;
        }
        None => {
            warnings.push(format!("outputs[{at}]: no name"));
            return None;
        }
    };
    let mut o = LoginOutput::named(name);
    for (key, value) in table {
        let set = match key.as_str() {
            "name" => Ok(()),
            "mode" => value
                .as_str()
                .ok_or_else(|| "expected a string".to_string())
                .and_then(LoginMode::parse)
                .map(|m| o.mode = Some(m)),
            "scale" => quarters(value).map(|q| o.quarters = q),
            "transform" => match value.as_str() {
                Some(s) => Transform::parse(s)
                    .map(|t| o.transform = t)
                    .ok_or_else(|| format!("\"{s}\" is not a transform")),
                None => Err("expected a string".into()),
            },
            "position" => position(value).map(|p| o.position = Some(p)),
            "primary" => value
                .as_bool()
                .map(|b| o.primary = b)
                .ok_or_else(|| "expected true or false".into()),
            _ => Err("unknown key".into()),
        };
        if let Err(w) = set {
            warnings.push(format!("outputs[{at}].{key}: {w}"));
        }
    }
    Some(o)
}

/// The `outputs` array of tables: none when it holds more than [`MAX_OUTPUTS`]; a connector
/// named twice keeps its first table, and only the first primary output stays primary.
pub(super) fn parse(value: &toml::Value, warnings: &mut Vec<String>) -> Vec<LoginOutput> {
    let Some(tables) = value.as_array() else {
        warnings.push("outputs: expected [[outputs]] tables".into());
        return Vec::new();
    };
    if tables.len() > MAX_OUTPUTS {
        warnings.push(format!(
            "outputs: {} tables, more than {MAX_OUTPUTS}",
            tables.len()
        ));
        return Vec::new();
    }
    let mut outputs: Vec<LoginOutput> = Vec::new();
    for (at, t) in tables.iter().enumerate() {
        let Some(table) = t.as_table() else {
            warnings.push(format!("outputs[{at}]: expected a table"));
            continue;
        };
        let Some(mut o) = output(table, at, warnings) else {
            continue;
        };
        if outputs.iter().any(|p| p.name == o.name) {
            warnings.push(format!("outputs[{at}]: {} is named twice", o.name));
            continue;
        }
        if o.primary && outputs.iter().any(|p| p.primary) {
            warnings.push(format!("outputs[{at}].primary: another output is primary"));
            o.primary = false;
        }
        outputs.push(o);
    }
    outputs
}

/// `outputs` as `[[outputs]]` tables, leaving out what is automatic.
pub(super) fn render(out: &mut String, outputs: &[LoginOutput]) {
    for o in outputs {
        out.push_str("\n[[outputs]]\n");
        super::line(out, "name", o.name.as_str());
        if let Some(m) = o.mode {
            super::line(out, "mode", m.to_string());
        }
        super::line(out, "scale", f64::from(o.scale()));
        if o.transform != Transform::Normal {
            super::line(out, "transform", o.transform.name());
        }
        if let Some((x, y)) = o.position {
            super::line(
                out,
                "position",
                toml::Value::Array(vec![i64::from(x).into(), i64::from(y).into()]),
            );
        }
        if o.primary {
            super::line(out, "primary", true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::MachineConfig;

    fn outputs(toml: &str) -> (Vec<LoginOutput>, Vec<String>) {
        let (config, warnings) = MachineConfig::parse(toml);
        (config.outputs, warnings)
    }

    #[test]
    fn reads_every_key_of_an_output() {
        let (got, warnings) = outputs(
            "[[outputs]]\nname = \"DP-1\"\nmode = \"2560x1440@143.912\"\nscale = 1.25\n\
             transform = \"90\"\nposition = [-1440, 0]\nprimary = true\n\
             [[outputs]]\nname = \"eDP-1\"\nscale = 2\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            got,
            [
                LoginOutput {
                    name: "DP-1".into(),
                    mode: Some(LoginMode {
                        width: 2560,
                        height: 1440,
                        refresh_mhz: Some(143_912),
                    }),
                    quarters: 5,
                    transform: Transform::Rotate90,
                    position: Some((-1440, 0)),
                    primary: true,
                },
                LoginOutput {
                    quarters: 8,
                    ..LoginOutput::named("eDP-1")
                },
            ]
        );
    }

    #[test]
    fn a_scale_off_the_quarter_steps_or_outside_1_to_4_is_refused() {
        for bad in ["1.3", "0.75", "4.25", "0", "\"2\""] {
            let (got, warnings) =
                outputs(&format!("[[outputs]]\nname = \"DP-1\"\nscale = {bad}\n"));
            assert_eq!(warnings.len(), 1, "scale = {bad}: {warnings:?}");
            assert!(
                warnings[0].starts_with("outputs[0].scale: "),
                "{warnings:?}"
            );
            assert_eq!(got, [LoginOutput::named("DP-1")], "scale = {bad} stays 1");
        }
        let (got, warnings) = outputs("[[outputs]]\nname = \"DP-1\"\nscale = 4\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(got[0].quarters, 16);
    }

    #[test]
    fn an_unknown_transform_is_refused() {
        let (got, warnings) = outputs("[[outputs]]\nname = \"DP-1\"\ntransform = \"45\"\n");
        assert_eq!(
            warnings,
            ["outputs[0].transform: \"45\" is not a transform"]
        );
        assert_eq!(got[0].transform, Transform::Normal);
    }

    #[test]
    fn more_than_sixteen_outputs_are_refused_whole() {
        let many: String = (0..=MAX_OUTPUTS)
            .map(|n| format!("[[outputs]]\nname = \"DP-{n}\"\n"))
            .collect();
        let (got, warnings) = outputs(&many);
        assert_eq!(warnings, ["outputs: 17 tables, more than 16"]);
        assert!(got.is_empty(), "{got:?}");
        let (got, warnings) = outputs(&many[..many.rfind("[[outputs]]").unwrap()]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(got.len(), MAX_OUTPUTS);
    }

    #[test]
    fn positions_past_the_bound_and_bad_modes_are_refused() {
        let (got, warnings) = outputs(
            "[[outputs]]\nname = \"DP-1\"\nposition = [65537, 0]\nmode = \"0x1080\"\n\
             [[outputs]]\nname = \"DP-2\"\nposition = [1, 2, 3]\nmode = \"1920x1080@0\"\n",
        );
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert_eq!(
            got,
            [LoginOutput::named("DP-1"), LoginOutput::named("DP-2")]
        );
        let (got, warnings) = outputs("[[outputs]]\nname = \"DP-1\"\nposition = [-65536, 65536]\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(got[0].position, Some((-65536, 65536)));
    }

    #[test]
    fn a_bad_name_a_twice_named_connector_and_a_second_primary_are_dropped() {
        let (got, warnings) = outputs(
            "[[outputs]]\nname = \"DP-1\"\nprimary = true\n\
             [[outputs]]\nname = \"../etc\"\n\
             [[outputs]]\nscale = 2\n\
             [[outputs]]\nname = \"DP-1\"\nscale = 2\n\
             [[outputs]]\nname = \"DP-2\"\nprimary = true\nwidth = 3\n",
        );
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert_eq!(
            got,
            [
                LoginOutput {
                    primary: true,
                    ..LoginOutput::named("DP-1")
                },
                LoginOutput::named("DP-2"),
            ]
        );
        let (_, warnings) = outputs("outputs = 3\n");
        assert_eq!(warnings, ["outputs: expected [[outputs]] tables"]);
    }

    #[test]
    fn outputs_round_trip_through_to_toml() {
        let c = MachineConfig {
            outputs: vec![
                LoginOutput {
                    name: "HDMI-A-1".into(),
                    mode: Some(LoginMode {
                        width: 1920,
                        height: 1080,
                        refresh_mhz: Some(59_940),
                    }),
                    quarters: 6,
                    transform: Transform::Flipped270,
                    position: Some((1920, -200)),
                    primary: true,
                },
                LoginOutput::named("eDP-1"),
                LoginOutput {
                    mode: Some(LoginMode {
                        width: 800,
                        height: 600,
                        refresh_mhz: Some(60_000),
                    }),
                    ..LoginOutput::named("DP-3")
                },
            ],
            ..MachineConfig::default()
        };
        let text = c.to_toml();
        assert!(text.contains("mode = \"1920x1080@59.94\""), "{text}");
        assert!(text.contains("mode = \"800x600@60\""), "{text}");
        assert_eq!(MachineConfig::parse(&text), (c, vec![]));
    }

    #[test]
    fn every_transform_name_parses_back() {
        for t in Transform::ALL {
            assert_eq!(Transform::parse(t.name()), Some(t));
            assert_eq!(
                serde_json::to_string(&t).unwrap(),
                format!("\"{}\"", t.name())
            );
        }
    }
}
