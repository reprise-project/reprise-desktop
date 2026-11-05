//! Hardware sensors: every `/sys/class/hwmon` chip with its temperatures, fans, voltages, power
//! and currents. [`scan`] finds the chips and their inputs; [`read`] takes their values.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
    Celsius,
    Rpm,
    Volts,
    Watts,
    Amps,
}

impl Unit {
    /// The hwmon file prefix, and what one of its raw units is in this unit.
    fn file(self) -> (&'static str, f64) {
        match self {
            Unit::Celsius => ("temp", 1e-3),
            Unit::Rpm => ("fan", 1.0),
            Unit::Volts => ("in", 1e-3),
            Unit::Watts => ("power", 1e-6),
            Unit::Amps => ("curr", 1e-3),
        }
    }

    const ALL: [Unit; 5] = [
        Unit::Celsius,
        Unit::Rpm,
        Unit::Volts,
        Unit::Watts,
        Unit::Amps,
    ];

    /// `v` written with its unit: "45.0 °C", "1,200 RPM", "1.23 V", "6.5 W", "0.50 A".
    pub fn text(self, v: f64) -> String {
        match self {
            Unit::Celsius => format!("{v:.1} \u{b0}C"),
            Unit::Rpm => format!("{} RPM", crate::grouped(v.round().max(0.0) as u64)),
            Unit::Volts => format!("{v:.2} V"),
            Unit::Watts => format!("{v:.1} W"),
            Unit::Amps => format!("{v:.2} A"),
        }
    }
}

/// One input of a chip: where its value is read and what it is called.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub unit: Unit,
    pub label: String,
    file: PathBuf,
}

/// A chip and its inputs, found by [`scan`].
#[derive(Clone, Debug, PartialEq)]
pub struct ChipLayout {
    pub name: String,
    /// The device the chip measures, resolved, to tie it to a GPU.
    pub device: Option<PathBuf>,
    pub inputs: Vec<Input>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub unit: Unit,
    pub label: String,
    pub value: f64,
}

/// A chip's values as read now. `asleep`: its device is runtime suspended, and reading it would
/// wake it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chip {
    pub name: String,
    pub device: Option<PathBuf>,
    pub asleep: bool,
    pub readings: Vec<Reading>,
}

impl Chip {
    pub fn first(&self, unit: Unit) -> Option<f64> {
        self.readings
            .iter()
            .find(|r| r.unit == unit)
            .map(|r| r.value)
    }
}

/// The inputs among a chip directory's file names: each `<kind><n>_input` (power also as
/// `_average`), labelled by `<kind><n>_label` through `label`, by kind and then number.
pub fn inputs(dir: &Path, files: &[String], label: impl Fn(&str) -> Option<String>) -> Vec<Input> {
    let mut out: Vec<(Unit, u32, Input)> = Vec::new();
    for unit in Unit::ALL {
        let (prefix, _) = unit.file();
        for f in files {
            let Some(rest) = f.strip_prefix(prefix) else {
                continue;
            };
            let Some((n, suffix)) = rest.split_once('_') else {
                continue;
            };
            let Ok(num) = n.parse::<u32>() else {
                continue;
            };
            let input = format!("{prefix}{n}_input");
            let wanted = match suffix {
                "input" => true,
                "average" => unit == Unit::Watts && !files.contains(&input),
                _ => false,
            };
            if !wanted {
                continue;
            }
            let name = format!("{prefix}{n}");
            out.push((
                unit,
                num,
                Input {
                    unit,
                    label: label(&format!("{name}_label")).unwrap_or(name),
                    file: dir.join(f),
                },
            ));
        }
    }
    out.sort_by_key(|(unit, n, _)| (*unit, *n));
    out.into_iter().map(|(_, _, i)| i).collect()
}

/// Every chip under `root` (`/` on a live system) with an input, by hwmon number.
pub fn scan(root: &Path) -> Vec<ChipLayout> {
    let class = root.join("sys/class/hwmon");
    let Ok(dir) = std::fs::read_dir(&class) else {
        return Vec::new();
    };
    let mut chips: Vec<(u32, ChipLayout)> = dir
        .flatten()
        .filter_map(|e| {
            let n: u32 = e
                .file_name()
                .to_str()?
                .strip_prefix("hwmon")?
                .parse()
                .ok()?;
            let at = class.join(e.file_name());
            let files: Vec<String> = std::fs::read_dir(&at)
                .ok()?
                .flatten()
                .filter_map(|f| f.file_name().into_string().ok())
                .collect();
            let label = |f: &str| super::disk::read_trim(&at.join(f)).filter(|s| !s.is_empty());
            let inputs = inputs(&at, &files, label);
            if inputs.is_empty() {
                return None;
            }
            let name =
                super::disk::read_trim(&at.join("name")).unwrap_or_else(|| format!("hwmon{n}"));
            Some((
                n,
                ChipLayout {
                    name,
                    device: std::fs::canonicalize(at.join("device")).ok(),
                    inputs,
                },
            ))
        })
        .collect();
    chips.sort_by_key(|c| c.0);
    chips.into_iter().map(|(_, c)| c).collect()
}

/// Whether the device at `dev` is runtime suspended.
pub fn suspended(dev: &Path) -> bool {
    super::disk::read_trim(&dev.join("power/runtime_status")).as_deref() == Some("suspended")
}

/// Each chip's values now; an input that fails to read (a sensor that is off) is left out, and a
/// chip whose device sleeps is not read at all.
pub fn read(chips: &[ChipLayout]) -> Vec<Chip> {
    chips
        .iter()
        .map(|c| {
            let asleep = c.device.as_deref().is_some_and(suspended);
            let readings = if asleep {
                Vec::new()
            } else {
                (c.inputs.iter())
                    .filter_map(|i| {
                        let raw: f64 = super::disk::read_trim(&i.file)?.parse().ok()?;
                        Some(Reading {
                            unit: i.unit,
                            label: i.label.clone(),
                            value: raw * i.unit.file().1,
                        })
                    })
                    .collect()
            };
            Chip {
                name: c.name.clone(),
                device: c.device.clone(),
                asleep,
                readings,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(root: &Path, n: u32, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = root.join(format!("sys/class/hwmon/hwmon{n}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
        for (f, v) in files {
            std::fs::write(d.join(f), format!("{v}\n")).unwrap();
        }
        d
    }

    #[test]
    fn chips_read_in_their_units_with_their_labels() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        chip(r, 0, "AC", &[("uevent", "")]);
        chip(
            r,
            6,
            "k10temp",
            &[
                ("temp1_input", "45250"),
                ("temp1_label", "Tctl"),
                ("temp3_input", "40000"),
            ],
        );
        chip(
            r,
            4,
            "thinkpad",
            &[
                ("fan2_input", "0"),
                ("fan1_input", "2950"),
                ("temp1_input", "51000"),
                ("temp1_label", "CPU"),
                ("pwm1", "128"),
            ],
        );
        chip(
            r,
            5,
            "amdgpu",
            &[
                ("in0_input", "1218"),
                ("in0_label", "vddgfx"),
                ("power1_average", "6500000"),
            ],
        );
        let gpu = r.join("sys/devices/pci0000:00/0000:07:00.0");
        std::fs::create_dir_all(&gpu).unwrap();
        std::os::unix::fs::symlink(&gpu, r.join("sys/class/hwmon/hwmon5/device")).unwrap();
        let layout = scan(r);
        let names: Vec<&str> = layout.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["thinkpad", "amdgpu", "k10temp"], "AC has no inputs");
        let chips = read(&layout);
        let got = |c: &Chip| -> Vec<(String, String)> {
            (c.readings.iter())
                .map(|x| (x.label.clone(), x.unit.text(x.value)))
                .collect()
        };
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            got(&chips[0]),
            pairs(&[
                ("CPU", "51.0 \u{b0}C"),
                ("fan1", "2,950 RPM"),
                ("fan2", "0 RPM")
            ])
        );
        assert_eq!(
            got(&chips[1]),
            pairs(&[("vddgfx", "1.22 V"), ("power1", "6.5 W")])
        );
        assert_eq!(chips[1].device, Some(std::fs::canonicalize(&gpu).unwrap()));
        assert_eq!(chips[2].first(Unit::Celsius), Some(45.25));
        assert_eq!(got(&chips[2])[1].0, "temp3");
    }

    #[test]
    fn a_sleeping_device_is_not_read_and_a_failing_input_is_left_out() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let d = chip(
            r,
            1,
            "amdgpu",
            &[("temp1_input", "50000"), ("temp2_input", "x")],
        );
        let layout = scan(r);
        assert_eq!(read(&layout)[0].readings.len(), 1);
        let dev = r.join("dev");
        std::fs::create_dir_all(dev.join("power")).unwrap();
        std::fs::write(dev.join("power/runtime_status"), "suspended\n").unwrap();
        std::os::unix::fs::symlink(&dev, d.join("device")).unwrap();
        let chips = read(&scan(r));
        assert!(chips[0].asleep);
        assert!(chips[0].readings.is_empty());
    }
}
