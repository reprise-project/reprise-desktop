//! A look's notification sheet, its stack by the tray and the "+N more" chip
//! past it. Classic and Blue show the balloon in [`super::tooltip`] instead.

mod draw;
mod glass;
mod layout;
mod opening;
mod stack;

#[cfg(test)]
mod tests;

pub use draw::{SheetIcons, draw_sheet};
pub use glass::SheetGlass;
pub use layout::{open_layout, sheet_layout};
pub use opening::{
    SheetOpening, WarmJob, WarmLayers, build_warm, draw_sheet_opening, opening_shows_close,
};
pub use stack::{
    Point, RISE, chip_label, chip_layout, draw_chip, drip_bead, drip_bead_at, drip_bead_share,
    drip_corner, drip_ms, drip_pose, place_stack, pose,
};

use crate::access::Tree;
use crate::access::accesskit::NodeId;
use crate::raster::Rect;
use crate::style::{NotifyLook, SheetShape, Well};
use crate::text::TextRenderer;
use crate::theme::Theme;

use super::tooltip::{
    NotifyHit, NotifyParts, NotifyReply, NotifySays, ReplyField, access_parts, parts_hit,
};
use draw::field_text;

pub fn notify_look(theme: &Theme) -> Option<NotifyLook> {
    theme.look.style().and_then(|st| st.notify)
}

pub fn has_sheets(theme: &Theme) -> bool {
    notify_look(theme).is_some()
}

/// What a sheet shows. `when` is the app line's time ("now"); `has_picture` and
/// `has_app_icon` say which of [`SheetIcons`] the host will pass.
pub struct SheetContent<'a> {
    pub app: &'a str,
    pub when: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: &'a [&'a str],
    pub reply: Option<NotifyReply<'a>>,
    pub has_picture: bool,
    pub has_app_icon: bool,
}

/// A sheet in device px from its canvas's top-left; the canvas is `footprint` (`sheet` and any
/// well standing out of it) grown by the look's shadow margin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetLayout {
    pub width: i32,
    pub height: i32,
    pub shape: SheetShape,
    pub expanded: bool,
    pub sheet: Rect,
    pub footprint: Rect,
    pub icon: Rect,
    pub well: Well,
    pub badge: Option<Rect>,
    pub picture: Option<Rect>,
    pub parts: NotifyParts,
    pub lines: Vec<String>,
    pub app_px: u32,
    pub title_px: u32,
    pub body_px: u32,
    app_line: String,
    app_box: Rect,
    title: String,
    labels: Vec<String>,
    line_h: i32,
    pill_px: u32,
}

/// A press on the sheet's reply field at canvas x `x`: it takes the keyboard and the
/// caret goes to the nearest character boundary.
pub fn sheet_click(
    lay: &SheetLayout,
    theme: &Theme,
    text: &TextRenderer,
    reply: &mut ReplyField,
    x: i32,
) {
    reply.click_text(text, field_text(lay, theme.scale.max(1)), lay.body_px, x);
}

/// `None` on the shadow margin and beside a well standing out of the sheet. The close spot
/// takes the pointer whether or not the button shows.
pub fn sheet_hit(lay: &SheetLayout, x: i32, y: i32) -> Option<NotifyHit> {
    let on = lay.sheet.contains(x, y) || (lay.footprint != lay.sheet && lay.icon.contains(x, y));
    on.then(|| parts_hit(&lay.parts, x, y))
}

/// The sheet's `access` part ([`access_parts`]) for `lay` laid out at scale 1,
/// its canvas at `at` in the access root.
#[expect(clippy::too_many_arguments)]
pub fn access_sheet(
    key: impl std::hash::Hash,
    lay: &SheetLayout,
    at: (i32, i32),
    says: &NotifySays,
    reply: Option<&ReplyField>,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let parts = lay.parts.offset(at.0, at.1);
    access_parts(key, &parts, says, reply, text, px, tree, parent)
}
