use crate::hz_string;

#[derive(Clone, Debug, PartialEq)]
pub struct OutputConfig {
    pub name: String,
    pub scale: f32,
    pub mode: Option<OutputMode>,
    pub adaptive_sync: Option<bool>,
    pub transform: Transform,
    pub position: Option<(i32, i32)>,
    pub primary: bool,
}

/// An output transform by its wlr-randr name (`transform = "90"`); the
/// discriminants are the `wl_output_transform` values, rotations counter-clockwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Transform {
    #[default]
    Normal = 0,
    Rotate90 = 1,
    Rotate180 = 2,
    Rotate270 = 3,
    Flipped = 4,
    Flipped90 = 5,
    Flipped180 = 6,
    Flipped270 = 7,
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

/// A mode request from `mode = "1920x1080"` or `"1920x1080@59.94"`; the refresh
/// (above 0, at most 1000 Hz) is in mHz like Smithay's `output::Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: Option<i32>,
}

impl OutputMode {
    pub fn parse(s: &str) -> Result<OutputMode, String> {
        let bad = || format!("mode {s:?} is not WIDTHxHEIGHT[@HZ]");
        let (size, hz) = match s.split_once('@') {
            Some((size, hz)) => (size, Some(hz)),
            None => (s, None),
        };
        let (w, h) = size.split_once('x').ok_or_else(bad)?;
        let width: i32 = w.trim().parse().map_err(|_| bad())?;
        let height: i32 = h.trim().parse().map_err(|_| bad())?;
        if width <= 0 || height <= 0 {
            return Err(bad());
        }
        let refresh_mhz = match hz {
            Some(hz) => {
                let hz: f64 = hz.trim().parse().map_err(|_| bad())?;
                // 1000 Hz bounds the mHz value well inside i32.
                if !hz.is_finite() || hz <= 0.0 || hz > 1000.0 {
                    return Err(bad());
                }
                Some((hz * 1000.0).round() as i32)
            }
            None => None,
        };
        Ok(OutputMode {
            width,
            height,
            refresh_mhz,
        })
    }

    /// Index of the best match among `modes` (in the output's order): same
    /// size, then the closest refresh when one was asked for, else the
    /// preferred one, else the highest refresh. None when no mode has the
    /// size.
    pub fn pick(&self, modes: &[OfferedMode]) -> Option<usize> {
        let same_size = modes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.width == self.width && m.height == self.height);
        match self.refresh_mhz {
            Some(want) => same_size.min_by_key(|(_, m)| (m.refresh_mhz - want).abs()),
            None => same_size.max_by_key(|(_, m)| (m.preferred, m.refresh_mhz)),
        }
        .map(|(i, _)| i)
    }
}

impl std::fmt::Display for OutputMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)?;
        match self.refresh_mhz {
            Some(mhz) => write!(f, "@{}", hz_string(mhz)),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfferedMode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}

/// How far from the origin a head may sit: the layout's arithmetic is i32, and
/// 2^24 leaves room for size and neighbours.
pub const MAX_POSITION: i32 = 1 << 24;

/// An output scale as the compositor runs it: the nearest quarter step,
/// clamped to 1..=4, 1 for anything that is not a number.
pub fn quarter_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        ((scale * 4.0).round() / 4.0).clamp(1.0, 4.0)
    } else {
        1.0
    }
}
