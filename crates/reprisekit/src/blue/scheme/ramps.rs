//! Stop positions a ramp has in every scheme, and [`grad`], which pairs positions with a scheme's
//! colours. A ramp whose positions differ between schemes keeps them inline in each scheme.

use crate::theme::Color;

/// Pairs each position with the `0xRRGGBB` colour at the same index.
pub const fn grad<const N: usize>(at: [f32; N], rgb: [u32; N]) -> [(f32, Color); N] {
    let mut out = [(0.0, Color::rgb(0, 0, 0)); N];
    let mut i = 0;
    while i < N {
        let c = rgb[i];
        out[i] = (at[i], Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
        i += 1;
    }
    out
}

/// A [`super::Grad`] from `stops![position: 0xRRGGBB, ...]`, or from a shared position list and
/// one colour per position: `stops![ramps::NAME; 0xRRGGBB, ...]`.
macro_rules! stops {
    ($at:path; $($rgb:literal),+ $(,)?) => {
        &$crate::blue::scheme::ramps::grad($at, [$($rgb),+])
    };
    ($($at:literal: $rgb:literal),+ $(,)?) => {
        &$crate::blue::scheme::ramps::grad([$($at),+], [$($rgb),+])
    };
}
pub(super) use stops;

pub const FRAME_LEFT_NORMAL_ACROSS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
pub const FRAME_LEFT_INACTIVE_DOWN: [f32; 1] = [0.0161];
pub const TASKBAR_NORMAL_ACROSS: [f32; 1] = [0.125];
pub const CHECK_BOX_NORMAL_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_NORMAL_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_HOT_DOWN: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_HOT_ACROSS: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_PRESSED_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_PRESSED_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_NORMAL_CHECKED_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_NORMAL_CHECKED_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_HOT_CHECKED_DOWN: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_HOT_CHECKED_ACROSS: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_PRESSED_CHECKED_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_PRESSED_CHECKED_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_NORMAL_MIXED_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_NORMAL_MIXED_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_HOT_MIXED_DOWN: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_HOT_MIXED_ACROSS: [f32; 8] = [
    0.0385, 0.1154, 0.1923, 0.2692, 0.7308, 0.8077, 0.8846, 0.9615,
];
pub const CHECK_BOX_PRESSED_MIXED_DOWN: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const CHECK_BOX_PRESSED_MIXED_ACROSS: [f32; 4] = [0.0385, 0.1154, 0.8846, 0.9615];
pub const RADIO_NORMAL_DOWN: [f32; 6] = [0.0385, 0.1154, 0.1923, 0.8077, 0.8846, 0.9615];
pub const RADIO_NORMAL_ACROSS: [f32; 6] = [0.0385, 0.1154, 0.1923, 0.8077, 0.8846, 0.9615];
pub const RADIO_PRESSED_DOWN: [f32; 7] = [0.0385, 0.1154, 0.1923, 0.5, 0.8077, 0.8846, 0.9615];
pub const RADIO_PRESSED_ACROSS: [f32; 7] = [0.0385, 0.1154, 0.1923, 0.5, 0.8077, 0.8846, 0.9615];
pub const RADIO_DISABLED_DOWN: [f32; 1] = [0.0385];
pub const RADIO_DISABLED_ACROSS: [f32; 1] = [0.0385];
pub const RADIO_NORMAL_CHECKED_DOWN: [f32; 6] = [0.0385, 0.1154, 0.1923, 0.8077, 0.8846, 0.9615];
pub const RADIO_NORMAL_CHECKED_ACROSS: [f32; 6] = [0.0385, 0.1154, 0.1923, 0.8077, 0.8846, 0.9615];
pub const RADIO_PRESSED_CHECKED_DOWN: [f32; 7] =
    [0.0385, 0.1154, 0.1923, 0.5, 0.8077, 0.8846, 0.9615];
pub const RADIO_PRESSED_CHECKED_ACROSS: [f32; 7] =
    [0.0385, 0.1154, 0.1923, 0.5, 0.8077, 0.8846, 0.9615];
pub const RADIO_DISABLED_CHECKED_DOWN: [f32; 1] = [0.0385];
pub const RADIO_DISABLED_CHECKED_ACROSS: [f32; 1] = [0.0385];
pub const GROUP_BOX_NORMAL_DOWN: [f32; 1] = [0.0714];
pub const GROUP_BOX_NORMAL_ACROSS: [f32; 1] = [0.0714];
pub const GROUP_BOX_DISABLED_DOWN: [f32; 1] = [0.0714];
pub const GROUP_BOX_DISABLED_ACROSS: [f32; 1] = [0.0714];
pub const SCROLL_GRIP_V_NORMAL_DOWN: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_V_NORMAL_ACROSS: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_V_HOT_DOWN: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_V_HOT_ACROSS: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_V_PRESSED_DOWN: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_V_PRESSED_ACROSS: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_V_DISABLED_DOWN: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_V_DISABLED_ACROSS: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_H_NORMAL_DOWN: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_H_NORMAL_ACROSS: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_H_HOT_DOWN: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_H_HOT_ACROSS: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_H_PRESSED_DOWN: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_H_PRESSED_ACROSS: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_GRIP_H_DISABLED_DOWN: [f32; 4] = [0.0714, 0.2143, 0.7857, 0.9286];
pub const SCROLL_GRIP_H_DISABLED_ACROSS: [f32; 8] = [
    0.0625, 0.1875, 0.3125, 0.4375, 0.5625, 0.6875, 0.8125, 0.9375,
];
pub const SCROLL_TRACK_V_NORMAL_DOWN: [f32; 1] = [0.5];
pub const SCROLL_TRACK_V_PRESSED_DOWN: [f32; 1] = [0.5];
pub const SCROLL_TRACK_V_DISABLED_DOWN: [f32; 1] = [0.5];
pub const SCROLL_TRACK_H_NORMAL_ACROSS: [f32; 1] = [0.5];
pub const SCROLL_TRACK_H_PRESSED_ACROSS: [f32; 1] = [0.5];
pub const SCROLL_TRACK_H_DISABLED_ACROSS: [f32; 1] = [0.5];
pub const EDIT_BORDER_NORMAL_DOWN: [f32; 1] = [0.1];
pub const EDIT_BORDER_NORMAL_ACROSS: [f32; 1] = [0.1];
pub const EDIT_BORDER_DISABLED_DOWN: [f32; 4] = [0.1, 0.3, 0.7, 0.9];
pub const EDIT_BORDER_DISABLED_ACROSS: [f32; 4] = [0.1, 0.3, 0.7, 0.9];
pub const TAB_ITEM_CHECKED_DOWN: [f32; 4] = [0.0833, 0.1666, 0.1667, 0.9722];
pub const TAB_ITEM_CHECKED_ACROSS: [f32; 1] = [0.0];
pub const TAB_PANE_NORMAL_ACROSS: [f32; 2] = [0.0, 1.0];
pub const PROGRESS_BAR_NORMAL_DOWN: [f32; 7] =
    [0.0263, 0.0789, 0.1316, 0.1842, 0.8684, 0.9211, 0.9737];
pub const PROGRESS_BAR_NORMAL_ACROSS: [f32; 8] = [
    0.0556, 0.1667, 0.2778, 0.3889, 0.6111, 0.7222, 0.8333, 0.9444,
];
pub const PROGRESS_BAR_DISABLED_DOWN: [f32; 7] =
    [0.0263, 0.0789, 0.1316, 0.1842, 0.8684, 0.9211, 0.9737];
pub const PROGRESS_BAR_DISABLED_ACROSS: [f32; 8] = [
    0.0556, 0.1667, 0.2778, 0.3889, 0.6111, 0.7222, 0.8333, 0.9444,
];
pub const TRACKBAR_TRACK_NORMAL_DOWN: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
pub const TRACKBAR_TRACK_NORMAL_ACROSS: [f32; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
pub const TRACKBAR_TRACK_DISABLED_DOWN: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
pub const TRACKBAR_TRACK_DISABLED_ACROSS: [f32; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
pub const TRACKBAR_THUMB_DISABLED_ACROSS: [f32; 5] = [0.0455, 0.1364, 0.6818, 0.8636, 0.9545];
pub const TRACKBAR_TRACK_V_NORMAL_DOWN: [f32; 4] = [0.1, 0.3, 0.5, 0.9];
pub const TRACKBAR_TRACK_V_NORMAL_ACROSS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
pub const TRACKBAR_TRACK_V_DISABLED_DOWN: [f32; 4] = [0.1, 0.3, 0.5, 0.9];
pub const TRACKBAR_TRACK_V_DISABLED_ACROSS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
pub const TRACKBAR_THUMB_V_NORMAL_ACROSS: [f32; 8] = [
    0.0227, 0.0682, 0.1136, 0.1591, 0.8409, 0.8864, 0.9318, 0.9773,
];
pub const TRACKBAR_THUMB_V_HOT_ACROSS: [f32; 8] = [
    0.0227, 0.0682, 0.1136, 0.1591, 0.8409, 0.8864, 0.9318, 0.9773,
];
pub const TRACKBAR_THUMB_V_PRESSED_ACROSS: [f32; 8] = [
    0.0227, 0.0682, 0.1136, 0.1591, 0.8409, 0.8864, 0.9318, 0.9773,
];
pub const TRACKBAR_THUMB_V_DISABLED_ACROSS: [f32; 8] = [
    0.0227, 0.0682, 0.1136, 0.1591, 0.8409, 0.8864, 0.9318, 0.9773,
];
pub const SIZE_BOX_NORMAL_DOWN: [f32; 9] = [
    0.0455, 0.1364, 0.2273, 0.4091, 0.5, 0.5909, 0.7727, 0.8636, 0.9545,
];
pub const SIZE_BOX_NORMAL_ACROSS: [f32; 9] = [
    0.0455, 0.1364, 0.2273, 0.4091, 0.5, 0.5909, 0.7727, 0.8636, 0.9545,
];
pub const CALENDAR_BAND_NORMAL_DOWN: [f32; 1] = [0.5];
pub const CALENDAR_BAND_NORMAL_ACROSS: [f32; 1] = [0.5];
pub const CALENDAR_BAND_DISABLED_DOWN: [f32; 1] = [0.5];
pub const CALENDAR_BAND_DISABLED_ACROSS: [f32; 1] = [0.5];
pub const TOOLBAR_BUTTON_PRESSED_DOWN: [f32; 6] = [0.0132, 0.0395, 0.0921, 0.8816, 0.9605, 0.9868];
pub const TOOLBAR_BUTTON_CHECKED_DOWN: [f32; 4] = [0.0132, 0.0395, 0.9605, 0.9868];
pub const TOOLBAR_BUTTON_CHECKED_ACROSS: [f32; 4] = [0.0556, 0.1667, 0.8333, 0.9444];
pub const TOOLBAR_BUTTON_CHECKED_HOT_ACROSS: [f32; 5] = [0.0556, 0.1667, 0.6111, 0.8333, 0.9444];
pub const TOOLBAR_DROP_PRESSED_DOWN: [f32; 6] = [0.0132, 0.0395, 0.0921, 0.8816, 0.9605, 0.9868];
pub const TOOLBAR_DROP_CHECKED_DOWN: [f32; 4] = [0.0132, 0.0395, 0.9605, 0.9868];
pub const TOOLBAR_DROP_CHECKED_ACROSS: [f32; 4] = [0.0556, 0.1667, 0.8333, 0.9444];
pub const TOOLBAR_DROP_CHECKED_HOT_ACROSS: [f32; 5] = [0.0556, 0.1667, 0.6111, 0.8333, 0.9444];
pub const TOOLBAR_SEPARATOR_NORMAL_DOWN: [f32; 1] = [0.625];
pub const TOOLBAR_SEPARATOR_NORMAL_ACROSS: [f32; 1] = [0.4167];
pub const REBAR_BAND_NORMAL_DOWN: [f32; 2] = [0.25, 0.75];
pub const REBAR_BAND_NORMAL_ACROSS: [f32; 1] = [0.5];
pub const HEADER_FILLER_NORMAL_ACROSS: [f32; 1] = [0.5];
pub const HEADER_SORT_ARROW_NORMAL_DOWN: [f32; 1] = [0.375];
pub const HEADER_SORT_ARROW_NORMAL_ACROSS: [f32; 1] = [0.0556];
pub const HEADER_SORT_ARROW_CHECKED_DOWN: [f32; 1] = [0.375];
pub const HEADER_SORT_ARROW_CHECKED_ACROSS: [f32; 1] = [0.0556];
pub const TASK_PANE_NORMAL_DOWN: [f32; 2] = [0.0011, 0.9989];
pub const TASK_PANE_NORMAL_ACROSS: [f32; 1] = [0.5];
pub const TASK_GROUP_BODY_NORMAL_DOWN: [f32; 2] = [0.25, 0.75];
pub const TASK_GROUP_BODY_NORMAL_ACROSS: [f32; 3] = [0.1667, 0.5, 0.8333];
pub const EXPLORER_BAR_HEADER_NORMAL_DOWN: [f32; 2] = [0.25, 0.75];
pub const EXPLORER_BAR_HEADER_NORMAL_ACROSS: [f32; 2] = [0.0025, 0.9975];
pub const EXPLORER_BAR_CLOSE_NORMAL_DOWN: [f32; 1] = [0.3235];
pub const EXPLORER_BAR_CLOSE_NORMAL_ACROSS: [f32; 1] = [0.325];
pub const PLACES_BAR_NORMAL_DOWN: [f32; 6] = [0.0294, 0.0882, 0.1471, 0.8529, 0.9118, 0.9706];
pub const PLACES_BAR_NORMAL_ACROSS: [f32; 6] = [0.0294, 0.0882, 0.1471, 0.8529, 0.9118, 0.9706];
pub const LIST_BORDER_NORMAL_DOWN: [f32; 1] = [0.1];
pub const LIST_BORDER_NORMAL_ACROSS: [f32; 1] = [0.1];
pub const LIST_BORDER_DISABLED_DOWN: [f32; 4] = [0.1, 0.3, 0.7, 0.9];
pub const LIST_BORDER_DISABLED_ACROSS: [f32; 4] = [0.1, 0.3, 0.7, 0.9];
