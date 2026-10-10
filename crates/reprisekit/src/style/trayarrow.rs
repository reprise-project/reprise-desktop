//! The frames of a round tray arrow button ([`crate::style::Style::tray_arrow`]), painted at the
//! style's scale. It sits in `style` because core cannot reach the looks that paint it.

use crate::raster::Canvas;

pub(crate) const SIZE: i32 = 19;

#[derive(Clone)]
pub(crate) struct TrayArrowArt {
    pub(crate) scale: i32,
    pub(crate) rest: Canvas,
    pub(crate) hot: Canvas,
    pub(crate) pressed: Canvas,
    pub(crate) open_rest: Canvas,
    pub(crate) open_hot: Canvas,
    pub(crate) open_pressed: Canvas,
}

impl std::fmt::Debug for TrayArrowArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayArrowArt")
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}
