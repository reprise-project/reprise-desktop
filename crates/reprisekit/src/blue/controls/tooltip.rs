//! The atlases a tooltip draws in Blue: none. Blue's tip is the scheme's own `info_bk` box, so
//! [`Part::Tooltip`] stays free for a look whose tip is drawn art.

use crate::blue::Tone;
use crate::style::{Part, PartImage};

pub(super) fn parts(_v: Tone) -> Vec<(Part, PartImage)> {
    Vec::new()
}
