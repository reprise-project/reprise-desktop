pub use repriselogin::machine::Transform;

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
    /// An ICC profile whose `vcgt` calibration goes into the output's gamma table.
    pub icc_profile: Option<std::path::PathBuf>,
}

/// A mode request from `mode = "1920x1080"` or `"1920x1080@59.94"`, read as the login screen's
/// is: sides 1 to 16384, the refresh (above 0, at most 1000 Hz) in mHz like Smithay's `output::Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: Option<i32>,
}

impl OutputMode {
    pub fn parse(s: &str) -> Result<OutputMode, String> {
        let m = repriselogin::machine::LoginMode::parse(s).map_err(|e| format!("mode {e}"))?;
        Ok(OutputMode {
            width: m.width as i32,
            height: m.height as i32,
            refresh_mhz: m.refresh_mhz.map(|r| r as i32),
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
