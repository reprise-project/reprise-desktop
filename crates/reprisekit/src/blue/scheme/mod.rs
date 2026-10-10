//! Blue colour schemes as named colours and metrics: one [`Scheme`] const per scheme, reached
//! through [`Variant::scheme`]. A [`Face`] names the colours one state of a part is drawn from.

mod ember;
mod navy;
mod obsidian;
mod olive;
mod sapphire;
mod silver;
mod slate;

pub use ember::EMBER;
pub use navy::NAVY;
pub use obsidian::OBSIDIAN;
pub use olive::OLIVE;
pub use sapphire::SAPPHIRE;
pub use silver::SILVER;
pub use slate::SLATE;

use super::Variant;
use crate::blue::trayarrow::{
    ArrowRecipe, BEAD, BOLD, DIAGONAL, DOWN, FINE, OUTLINED, Shading, banded, lit,
};
use crate::theme::{Color, ColorScheme};

pub fn scheme(v: Variant) -> &'static Scheme {
    match v {
        Variant::Blue => &NAVY,
        Variant::Olive => &OLIVE,
        Variant::Silver => &SILVER,
        Variant::Slate => &SLATE,
        Variant::Sapphire => &SAPPHIRE,
        Variant::Obsidian => &OBSIDIAN,
        Variant::Ember => &EMBER,
    }
}

impl Variant {
    pub fn scheme(self) -> &'static Scheme {
        scheme(self)
    }
}

/// (position, colour) stops, linear between, end colours held past the ends.
/// Positions run 0..=1 over the face at rest; pixel `i` of `n` sits at `(i + 0.5) / n`.
pub type Grad = &'static [(f32, Color)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub rim: Color,
    pub inner: Color,
    /// A translucent ring outside the rim and its alpha out of 255, where
    /// the part has one (a push button's soft shadow).
    pub halo: Option<(Color, u8)>,
    /// The fill top to bottom, through the part's stretchable middle
    /// columns, rim rows included.
    pub down: Grad,
    /// The fill left to right, through the part's stretchable middle rows,
    /// rim columns included.
    pub across: Grad,
    /// The colour drawn over the fill (glyph, tick, dot, glow); None if none.
    pub mark: Option<Color>,
}

/// The frame pixel a dialog's thinner frame leaves out, per border: `left`
/// counts outside in, `right` inside out, `bottom` and `caption` top down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogDrop {
    pub left: usize,
    pub right: usize,
    pub bottom: usize,
    pub caption: usize,
}

/// Chrome sizes that differ between schemes, in design px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub frame: i32,
    pub dialog_frame: i32,
    pub caption_height: i32,
    pub dialog_caption_height: i32,
    pub max_caption_height: i32,
    pub caption_left: i32,
    pub caption_right: i32,
    pub caption_button: i32,
    pub caption_button_gap: i32,
    pub caption_button_right: i32,
    pub title_px: i32,
    pub bottom_corner: i32,
    pub taskbar_height: i32,
    pub start_left: i32,
    pub start_middle: i32,
    pub start_right: i32,
    pub task_height: i32,
    pub task_left: i32,
    pub task_right: i32,
    pub tray_edge: i32,
    pub dialog_drop: DialogDrop,
    pub radii: Radii,
}

impl Metrics {
    pub fn start_width(&self) -> i32 {
        self.start_left + self.start_middle + self.start_right
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct System {
    pub active_title: Color,
    pub gradient_active_title: Color,
    pub inactive_title: Color,
    pub gradient_inactive_title: Color,
    pub title_text: Color,
    pub inactive_title_text: Color,
    pub button_face: Color,
    pub button_hilight: Color,
    pub button_light: Color,
    pub button_shadow: Color,
    pub button_dk_shadow: Color,
    pub button_text: Color,
    pub background: Color,
    pub window: Color,
    pub window_frame: Color,
    pub window_text: Color,
    pub hilight: Color,
    pub hilight_text: Color,
    pub gray_text: Color,
    pub menu: Color,
    pub menu_text: Color,
    pub desktop_text: Color,
    pub info_bk: Color,
    pub info_text: Color,
    pub menu_hilight: Color,
    pub menu_bar: Color,
    pub hot_light: Color,
}

impl System {
    /// These slots over Reprise Standard's for the rest (the menu bands).
    pub fn colors(&self) -> ColorScheme {
        ColorScheme {
            active_title: self.active_title,
            gradient_active_title: self.gradient_active_title,
            inactive_title: self.inactive_title,
            gradient_inactive_title: self.gradient_inactive_title,
            title_text: self.title_text,
            inactive_title_text: self.inactive_title_text,
            button_face: self.button_face,
            button_hilight: self.button_hilight,
            button_light: self.button_light,
            button_shadow: self.button_shadow,
            button_dk_shadow: self.button_dk_shadow,
            button_text: self.button_text,
            background: self.background,
            window: self.window,
            window_frame: self.window_frame,
            window_text: self.window_text,
            hilight: self.hilight,
            hilight_text: self.hilight_text,
            gray_text: self.gray_text,
            menu: self.menu,
            menu_text: self.menu_text,
            desktop_text: self.desktop_text,
            info_bk: self.info_bk,
            info_text: self.info_text,
            menu_hilight: self.menu_hilight,
            menu_bar: self.menu_bar,
            hot_light: self.hot_light,
            ..ColorScheme::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inks {
    pub title: Color,
    pub title_inactive: Color,
    pub title_shadow: Color,
    pub start_label: Color,
    pub start_shadow: Color,
    pub bar_text: Color,
    pub task_normal: Color,
    pub task_hot: Color,
    pub task_checked: Color,
    pub task_checked_hot: Color,
    pub task_pressed: Color,
    pub greyed_label: Color,
    pub group_label: Color,
    pub calendar_letters: Color,
    pub toolbar_pressed: Color,
    pub toolbar_disabled: Color,
    pub toolbar_drop_disabled: Color,
    pub rebar_label: Color,
    pub task_title: Color,
    pub task_title_hot: Color,
    pub task_link: Color,
    pub task_link_hot: Color,
    pub track_tick: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionFaces {
    pub normal: Face,
    pub inactive: Face,
}

impl CaptionFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaxCaptionFaces {
    pub normal: Face,
    pub inactive: Face,
}

impl MaxCaptionFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The left border, 4 px; `across` runs outside in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLeftFaces {
    pub normal: Face,
    pub inactive: Face,
}

impl FrameLeftFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The right border, 4 px; `across` runs inside out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRightFaces {
    pub normal: Face,
    pub inactive: Face,
}

impl FrameRightFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The bottom border, 4 rows; `down` runs top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameBottomFaces {
    pub normal: Face,
    pub inactive: Face,
}

impl FrameBottomFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The minimize caption button, 21x21; `mark` is the glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub inactive: Face,
}

impl MinButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The maximize caption button, 21x21; `mark` is the glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaxButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub inactive: Face,
}

impl MaxButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The restore caption button, 21x21; `mark` is the glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestoreButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub inactive: Face,
}

impl RestoreButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The close caption button, 21x21, the red one; `mark` is the cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloseButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub inactive: Face,
}

impl CloseButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskbarFaces {
    pub normal: Face,
}

impl TaskbarFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
}

impl StartButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub checked: Face,
    pub checked_hot: Face,
    pub pressed: Face,
}

impl TaskButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("checked", Some(&self.checked)),
            ("checked_hot", Some(&self.checked_hot)),
            ("pressed", Some(&self.pressed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The notification area: `down` is its band, `across` its 3 px left edge then the band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrayAreaFaces {
    pub normal: Face,
}

impl TrayAreaFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub default: Face,
    pub focused: Face,
}

impl PushButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("default", Some(&self.default)),
            ("focused", Some(&self.focused)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The 13 px check box; `mark` on a checked or mixed face is the tick or square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckBoxFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub normal_checked: Face,
    pub hot_checked: Face,
    pub pressed_checked: Face,
    pub disabled_checked: Face,
    pub normal_mixed: Face,
    pub hot_mixed: Face,
    pub pressed_mixed: Face,
    pub disabled_mixed: Face,
}

impl CheckBoxFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("normal_checked", Some(&self.normal_checked)),
            ("hot_checked", Some(&self.hot_checked)),
            ("pressed_checked", Some(&self.pressed_checked)),
            ("disabled_checked", Some(&self.disabled_checked)),
            ("normal_mixed", Some(&self.normal_mixed)),
            ("hot_mixed", Some(&self.hot_mixed)),
            ("pressed_mixed", Some(&self.pressed_mixed)),
            ("disabled_mixed", Some(&self.disabled_mixed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The 13 px radio; `mark` on a checked face is the dot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub normal_checked: Face,
    pub hot_checked: Face,
    pub pressed_checked: Face,
    pub disabled_checked: Face,
}

impl RadioFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
            ("normal_checked", Some(&self.normal_checked)),
            ("hot_checked", Some(&self.hot_checked)),
            ("pressed_checked", Some(&self.pressed_checked)),
            ("disabled_checked", Some(&self.disabled_checked)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupBoxFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl GroupBoxFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A scrollbar arrow button, 17x17; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArrowUpFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollArrowUpFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A scrollbar arrow button, 17x17; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArrowDownFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollArrowDownFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A scrollbar arrow button, 17x17; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArrowLeftFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollArrowLeftFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A scrollbar arrow button, 17x17; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArrowRightFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollArrowRightFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollThumbVFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollThumbVFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollThumbHFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollThumbHFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The grip lines on a vertical thumb. None where a scheme draws nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollGripVFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Option<Face>,
}

impl ScrollGripVFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", self.disabled.as_ref()),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The grip lines on a horizontal thumb. None where a scheme draws nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollGripHFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Option<Face>,
}

impl ScrollGripHFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", self.disabled.as_ref()),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A vertical scrollbar track; `across` runs left to right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTrackVFaces {
    pub normal: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollTrackVFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A horizontal scrollbar track; `down` runs top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTrackHFaces {
    pub normal: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ScrollTrackHFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditBorderFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl EditBorderFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListBorderFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl ListBorderFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A combo box drop button, 15x17; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboDropFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl ComboDropFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A tab, 12x18, its top corners rounded over [`Radii::tab_item`]. `rim` is
/// the outline down its sides and over its top, `mark` (where lit) the
/// outline over its top three rows, `down` the body over all 18 rows (a lit
/// tab's first two rows are its light top band), `across` a tint multiplied
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabItemFaces {
    pub normal: Face,
    pub hot: Face,
    pub checked: Face,
}

impl TabItemFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("checked", Some(&self.checked)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The tab page under the tabs: `rim` is its border, `inner` the line inside
/// it, `down` the body's fade over its height, `across` the soft shadow
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabPaneFaces {
    pub normal: Face,
}

impl TabPaneFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressBarFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl ProgressBarFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressChunkFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl ProgressChunkFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackbarTrackFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl TrackbarTrackFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackbarThumbFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl TrackbarThumbFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackbarTrackVFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl TrackbarTrackVFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackbarThumbVFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl TrackbarThumbVFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A spinner's up button, 15x8; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpinUpFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl SpinUpFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A spinner's down button, 15x8; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpinDownFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

impl SpinDownFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The caption help button, 21x21; `mark` is the question mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HelpButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub inactive: Face,
}

impl HelpButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("inactive", Some(&self.inactive)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeBoxFaces {
    pub normal: Face,
}

impl SizeBoxFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalendarBandFaces {
    pub normal: Face,
    pub disabled: Face,
}

impl CalendarBandFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("disabled", Some(&self.disabled)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A toolbar button; at rest it is flat. Clear (nothing drawn) when normal or disabled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarButtonFaces {
    pub hot: Face,
    pub pressed: Face,
    pub checked: Face,
    pub checked_hot: Face,
}

impl ToolbarButtonFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("checked", Some(&self.checked)),
            ("checked_hot", Some(&self.checked_hot)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A toolbar drop-down arrow segment. Clear (nothing drawn) when normal or disabled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarDropFaces {
    pub hot: Face,
    pub pressed: Face,
    pub checked: Face,
    pub checked_hot: Face,
}

impl ToolbarDropFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
            ("checked", Some(&self.checked)),
            ("checked_hot", Some(&self.checked_hot)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarSeparatorFaces {
    pub normal: Face,
}

impl ToolbarSeparatorFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RebarFaces {
    pub normal: Face,
}

impl RebarFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RebarBandFaces {
    pub normal: Face,
}

impl RebarBandFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderItemFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
}

impl HeaderItemFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderFillerFaces {
    pub normal: Face,
}

impl HeaderFillerFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A header sort arrow; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderSortArrowFaces {
    pub normal: Face,
    pub checked: Face,
}

impl HeaderSortArrowFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("checked", Some(&self.checked)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusBarFaces {
    pub normal: Face,
}

impl StatusBarFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusPaneFaces {
    pub normal: Face,
}

impl StatusPaneFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The task pane background; `down` runs its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskPaneFaces {
    pub normal: Face,
}

impl TaskPaneFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskGroupHeaderFaces {
    pub normal: Face,
}

impl TaskGroupHeaderFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskGroupBodyFaces {
    pub normal: Face,
}

impl TaskGroupBodyFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A task pane group's collapse button; `mark` is the chevron.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskCollapseFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
}

impl TaskCollapseFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// A task pane group's expand button; `mark` is the chevron.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskExpandFaces {
    pub normal: Face,
}

impl TaskExpandFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplorerBarHeaderFaces {
    pub normal: Face,
}

impl ExplorerBarHeaderFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// The explorer bar close button; `mark` is the cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplorerBarCloseFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
}

impl ExplorerBarCloseFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![
            ("normal", Some(&self.normal)),
            ("hot", Some(&self.hot)),
            ("pressed", Some(&self.pressed)),
        ];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacesBarFaces {
    pub normal: Face,
}

impl PlacesBarFaces {
    pub fn all(&self) -> Vec<(&'static str, &Face)> {
        let v = vec![("normal", Some(&self.normal))];
        v.into_iter()
            .filter_map(|(s, f)| f.map(|f| (s, f)))
            .collect()
    }
}

/// Corner radii in design px, for parts whose rounding differs by scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub caption: f32,
    pub tab_item: f32,
    pub task_group_header: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheme {
    pub name: &'static str,
    pub system: System,
    pub inks: Inks,
    pub metrics: Metrics,
    pub caption: CaptionFaces,
    pub max_caption: MaxCaptionFaces,
    pub frame_left: FrameLeftFaces,
    pub frame_right: FrameRightFaces,
    pub frame_bottom: FrameBottomFaces,
    pub min_button: MinButtonFaces,
    pub max_button: MaxButtonFaces,
    pub restore_button: RestoreButtonFaces,
    pub close_button: CloseButtonFaces,
    pub taskbar: TaskbarFaces,
    pub start_button: StartButtonFaces,
    pub task_button: TaskButtonFaces,
    pub tray_area: TrayAreaFaces,
    pub push_button: PushButtonFaces,
    pub check_box: CheckBoxFaces,
    pub radio: RadioFaces,
    pub group_box: GroupBoxFaces,
    pub scroll_arrow_up: ScrollArrowUpFaces,
    pub scroll_arrow_down: ScrollArrowDownFaces,
    pub scroll_arrow_left: ScrollArrowLeftFaces,
    pub scroll_arrow_right: ScrollArrowRightFaces,
    pub scroll_thumb_v: ScrollThumbVFaces,
    pub scroll_thumb_h: ScrollThumbHFaces,
    pub scroll_grip_v: ScrollGripVFaces,
    pub scroll_grip_h: ScrollGripHFaces,
    pub scroll_track_v: ScrollTrackVFaces,
    pub scroll_track_h: ScrollTrackHFaces,
    pub edit_border: EditBorderFaces,
    pub list_border: Option<ListBorderFaces>,
    pub combo_drop: ComboDropFaces,
    pub tab_item: TabItemFaces,
    pub tab_pane: TabPaneFaces,
    pub progress_bar: ProgressBarFaces,
    pub progress_chunk: ProgressChunkFaces,
    pub trackbar_track: TrackbarTrackFaces,
    pub trackbar_thumb: TrackbarThumbFaces,
    pub trackbar_track_v: TrackbarTrackVFaces,
    pub trackbar_thumb_v: TrackbarThumbVFaces,
    pub spin_up: SpinUpFaces,
    pub spin_down: SpinDownFaces,
    pub help_button: HelpButtonFaces,
    pub size_box: SizeBoxFaces,
    pub calendar_band: CalendarBandFaces,
    pub toolbar_button: ToolbarButtonFaces,
    pub toolbar_drop: ToolbarDropFaces,
    pub toolbar_separator: ToolbarSeparatorFaces,
    pub rebar: RebarFaces,
    pub rebar_band: RebarBandFaces,
    pub header_item: HeaderItemFaces,
    pub header_filler: HeaderFillerFaces,
    pub header_sort_arrow: HeaderSortArrowFaces,
    pub status_bar: StatusBarFaces,
    pub status_pane: StatusPaneFaces,
    pub task_pane: TaskPaneFaces,
    pub task_group_header: TaskGroupHeaderFaces,
    pub task_group_body: TaskGroupBodyFaces,
    pub task_collapse: TaskCollapseFaces,
    pub task_expand: TaskExpandFaces,
    pub explorer_bar_header: ExplorerBarHeaderFaces,
    pub explorer_bar_close: ExplorerBarCloseFaces,
    pub places_bar: PlacesBarFaces,
    pub tray_arrow: ArrowRecipe,
}

impl Scheme {
    pub fn faces(&self) -> Vec<(&'static str, &'static str, &Face)> {
        let mut v = Vec::new();
        v.extend(
            self.caption
                .all()
                .into_iter()
                .map(|(s, f)| ("caption", s, f)),
        );
        v.extend(
            self.max_caption
                .all()
                .into_iter()
                .map(|(s, f)| ("max_caption", s, f)),
        );
        v.extend(
            self.frame_left
                .all()
                .into_iter()
                .map(|(s, f)| ("frame_left", s, f)),
        );
        v.extend(
            self.frame_right
                .all()
                .into_iter()
                .map(|(s, f)| ("frame_right", s, f)),
        );
        v.extend(
            self.frame_bottom
                .all()
                .into_iter()
                .map(|(s, f)| ("frame_bottom", s, f)),
        );
        v.extend(
            self.min_button
                .all()
                .into_iter()
                .map(|(s, f)| ("min_button", s, f)),
        );
        v.extend(
            self.max_button
                .all()
                .into_iter()
                .map(|(s, f)| ("max_button", s, f)),
        );
        v.extend(
            self.restore_button
                .all()
                .into_iter()
                .map(|(s, f)| ("restore_button", s, f)),
        );
        v.extend(
            self.close_button
                .all()
                .into_iter()
                .map(|(s, f)| ("close_button", s, f)),
        );
        v.extend(
            self.taskbar
                .all()
                .into_iter()
                .map(|(s, f)| ("taskbar", s, f)),
        );
        v.extend(
            self.start_button
                .all()
                .into_iter()
                .map(|(s, f)| ("start_button", s, f)),
        );
        v.extend(
            self.task_button
                .all()
                .into_iter()
                .map(|(s, f)| ("task_button", s, f)),
        );
        v.extend(
            self.tray_area
                .all()
                .into_iter()
                .map(|(s, f)| ("tray_area", s, f)),
        );
        v.extend(
            self.push_button
                .all()
                .into_iter()
                .map(|(s, f)| ("push_button", s, f)),
        );
        v.extend(
            self.check_box
                .all()
                .into_iter()
                .map(|(s, f)| ("check_box", s, f)),
        );
        v.extend(self.radio.all().into_iter().map(|(s, f)| ("radio", s, f)));
        v.extend(
            self.group_box
                .all()
                .into_iter()
                .map(|(s, f)| ("group_box", s, f)),
        );
        v.extend(
            self.scroll_arrow_up
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_arrow_up", s, f)),
        );
        v.extend(
            self.scroll_arrow_down
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_arrow_down", s, f)),
        );
        v.extend(
            self.scroll_arrow_left
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_arrow_left", s, f)),
        );
        v.extend(
            self.scroll_arrow_right
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_arrow_right", s, f)),
        );
        v.extend(
            self.scroll_thumb_v
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_thumb_v", s, f)),
        );
        v.extend(
            self.scroll_thumb_h
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_thumb_h", s, f)),
        );
        v.extend(
            self.scroll_grip_v
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_grip_v", s, f)),
        );
        v.extend(
            self.scroll_grip_h
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_grip_h", s, f)),
        );
        v.extend(
            self.scroll_track_v
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_track_v", s, f)),
        );
        v.extend(
            self.scroll_track_h
                .all()
                .into_iter()
                .map(|(s, f)| ("scroll_track_h", s, f)),
        );
        v.extend(
            self.edit_border
                .all()
                .into_iter()
                .map(|(s, f)| ("edit_border", s, f)),
        );
        if let Some(p) = &self.list_border {
            v.extend(p.all().into_iter().map(|(s, f)| ("list_border", s, f)));
        }
        v.extend(
            self.combo_drop
                .all()
                .into_iter()
                .map(|(s, f)| ("combo_drop", s, f)),
        );
        v.extend(
            self.tab_item
                .all()
                .into_iter()
                .map(|(s, f)| ("tab_item", s, f)),
        );
        v.extend(
            self.tab_pane
                .all()
                .into_iter()
                .map(|(s, f)| ("tab_pane", s, f)),
        );
        v.extend(
            self.progress_bar
                .all()
                .into_iter()
                .map(|(s, f)| ("progress_bar", s, f)),
        );
        v.extend(
            self.progress_chunk
                .all()
                .into_iter()
                .map(|(s, f)| ("progress_chunk", s, f)),
        );
        v.extend(
            self.trackbar_track
                .all()
                .into_iter()
                .map(|(s, f)| ("trackbar_track", s, f)),
        );
        v.extend(
            self.trackbar_thumb
                .all()
                .into_iter()
                .map(|(s, f)| ("trackbar_thumb", s, f)),
        );
        v.extend(
            self.trackbar_track_v
                .all()
                .into_iter()
                .map(|(s, f)| ("trackbar_track_v", s, f)),
        );
        v.extend(
            self.trackbar_thumb_v
                .all()
                .into_iter()
                .map(|(s, f)| ("trackbar_thumb_v", s, f)),
        );
        v.extend(
            self.spin_up
                .all()
                .into_iter()
                .map(|(s, f)| ("spin_up", s, f)),
        );
        v.extend(
            self.spin_down
                .all()
                .into_iter()
                .map(|(s, f)| ("spin_down", s, f)),
        );
        v.extend(
            self.help_button
                .all()
                .into_iter()
                .map(|(s, f)| ("help_button", s, f)),
        );
        v.extend(
            self.size_box
                .all()
                .into_iter()
                .map(|(s, f)| ("size_box", s, f)),
        );
        v.extend(
            self.calendar_band
                .all()
                .into_iter()
                .map(|(s, f)| ("calendar_band", s, f)),
        );
        v.extend(
            self.toolbar_button
                .all()
                .into_iter()
                .map(|(s, f)| ("toolbar_button", s, f)),
        );
        v.extend(
            self.toolbar_drop
                .all()
                .into_iter()
                .map(|(s, f)| ("toolbar_drop", s, f)),
        );
        v.extend(
            self.toolbar_separator
                .all()
                .into_iter()
                .map(|(s, f)| ("toolbar_separator", s, f)),
        );
        v.extend(self.rebar.all().into_iter().map(|(s, f)| ("rebar", s, f)));
        v.extend(
            self.rebar_band
                .all()
                .into_iter()
                .map(|(s, f)| ("rebar_band", s, f)),
        );
        v.extend(
            self.header_item
                .all()
                .into_iter()
                .map(|(s, f)| ("header_item", s, f)),
        );
        v.extend(
            self.header_filler
                .all()
                .into_iter()
                .map(|(s, f)| ("header_filler", s, f)),
        );
        v.extend(
            self.header_sort_arrow
                .all()
                .into_iter()
                .map(|(s, f)| ("header_sort_arrow", s, f)),
        );
        v.extend(
            self.status_bar
                .all()
                .into_iter()
                .map(|(s, f)| ("status_bar", s, f)),
        );
        v.extend(
            self.status_pane
                .all()
                .into_iter()
                .map(|(s, f)| ("status_pane", s, f)),
        );
        v.extend(
            self.task_pane
                .all()
                .into_iter()
                .map(|(s, f)| ("task_pane", s, f)),
        );
        v.extend(
            self.task_group_header
                .all()
                .into_iter()
                .map(|(s, f)| ("task_group_header", s, f)),
        );
        v.extend(
            self.task_group_body
                .all()
                .into_iter()
                .map(|(s, f)| ("task_group_body", s, f)),
        );
        v.extend(
            self.task_collapse
                .all()
                .into_iter()
                .map(|(s, f)| ("task_collapse", s, f)),
        );
        v.extend(
            self.task_expand
                .all()
                .into_iter()
                .map(|(s, f)| ("task_expand", s, f)),
        );
        v.extend(
            self.explorer_bar_header
                .all()
                .into_iter()
                .map(|(s, f)| ("explorer_bar_header", s, f)),
        );
        v.extend(
            self.explorer_bar_close
                .all()
                .into_iter()
                .map(|(s, f)| ("explorer_bar_close", s, f)),
        );
        v.extend(
            self.places_bar
                .all()
                .into_iter()
                .map(|(s, f)| ("places_bar", s, f)),
        );
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::style_for;
    use crate::style::{Part, State};

    fn style_inks(v: Variant) -> Vec<Color> {
        let st = style_for(v);
        let ink = |p, s| st.text_color(p, s).unwrap();
        let task = |s| st.text_color(Part::TaskButton, s).unwrap_or(st.bar_text);
        let shadow = |t: &crate::style::TextSpec| t.shadow.as_ref().unwrap().color;
        vec![
            st.caption_text.color,
            st.caption_text.inactive,
            shadow(&st.caption_text),
            st.start_text.color,
            shadow(&st.start_text),
            st.bar_text,
            task(State::Normal),
            task(State::Hot),
            task(State::Checked),
            task(State::CheckedHot),
            task(State::Pressed),
            ink(Part::PushButton, State::Disabled),
            ink(Part::GroupBox, State::Normal),
            ink(Part::CalendarBand, State::Normal),
            ink(Part::ToolbarButton, State::Pressed),
            ink(Part::ToolbarButton, State::Disabled),
            ink(Part::ToolbarDrop, State::Disabled),
            ink(Part::RebarBand, State::Normal),
            ink(Part::TaskGroupHeader, State::Normal),
            ink(Part::TaskGroupHeader, State::Hot),
            ink(Part::TaskGroupBody, State::Normal),
            ink(Part::TaskGroupBody, State::Hot),
            st.track_marks.ink,
        ]
    }

    fn inks(s: &Scheme) -> Vec<Color> {
        let i = &s.inks;
        vec![
            i.title,
            i.title_inactive,
            i.title_shadow,
            i.start_label,
            i.start_shadow,
            i.bar_text,
            i.task_normal,
            i.task_hot,
            i.task_checked,
            i.task_checked_hot,
            i.task_pressed,
            i.greyed_label,
            i.group_label,
            i.calendar_letters,
            i.toolbar_pressed,
            i.toolbar_disabled,
            i.toolbar_drop_disabled,
            i.rebar_label,
            i.task_title,
            i.task_title_hot,
            i.task_link,
            i.task_link_hot,
            i.track_tick,
        ]
    }

    #[test]
    fn each_variant_reaches_its_own_scheme() {
        for v in Variant::ALL {
            assert_eq!(v.scheme().name, v.as_str());
        }
    }

    #[test]
    fn every_style_takes_its_schemes_system_colours_and_inks() {
        for v in Variant::ALL {
            let s = v.scheme();
            assert_eq!(s.system.colors(), style_for(v).colors, "{v:?}");
            assert_eq!(inks(s), style_inks(v), "{v:?}");
        }
    }

    #[test]
    fn every_face_is_filled() {
        for v in Variant::ALL {
            let s = v.scheme();
            let faces = s.faces();
            assert!(faces.len() > 150, "{v:?}: {} faces", faces.len());
            for (part, state, f) in faces {
                for (axis, g) in [("down", f.down), ("across", f.across)] {
                    assert!(!g.is_empty(), "{v:?} {part}.{state}.{axis} is empty");
                    assert!(
                        g.windows(2).all(|w| w[0].0 < w[1].0),
                        "{v:?} {part}.{state}.{axis} out of order"
                    );
                    assert!(
                        g.iter().all(|&(t, _)| (0.0..=1.0).contains(&t)),
                        "{v:?} {part}.{state}.{axis} outside 0..=1"
                    );
                }
                // The bare cross and a dark tray edge's outer column are black.
                let black_rim =
                    (part, state) == ("explorer_bar_close", "normal") || part == "tray_area";
                assert!(
                    black_rim || f.rim != Color::rgb(0, 0, 0),
                    "{v:?} {part}.{state} rim"
                );
            }
        }
    }

    #[test]
    fn metrics_per_scheme() {
        for (v, start_right, task_right, title) in [
            (Variant::Blue, 20, 4, 13),
            (Variant::Olive, 20, 4, 13),
            (Variant::Silver, 20, 4, 13),
            (Variant::Slate, 22, 12, 11),
            (Variant::Sapphire, 22, 12, 13),
            (Variant::Obsidian, 22, 12, 13),
            (Variant::Ember, 22, 13, 11),
        ] {
            let m = v.scheme().metrics;
            assert_eq!(m.start_right, start_right, "{v:?}");
            assert_eq!(m.task_right, task_right, "{v:?}");
            assert_eq!(m.title_px, title, "{v:?}");
            assert_eq!(m.start_width(), 77 + start_right, "{v:?}");
            let st = style_for(v);
            let task = st.part(Part::TaskButton).unwrap().frame(State::Normal);
            assert_eq!(m.task_left + 1 + m.task_right, task.width, "{v:?}");
            assert_eq!(m.task_height, task.height, "{v:?}");
            let caption = st.part(Part::Caption).unwrap();
            assert_eq!(caption.sizing.left, m.caption_left, "{v:?}");
            assert_eq!(caption.sizing.right, m.caption_right, "{v:?}");
            assert_eq!(caption.frame(State::Normal).height, m.caption_height);
            assert_eq!(st.metrics.caption_button_width, m.caption_button);
            assert_eq!(st.metrics.caption_button_gap, m.caption_button_gap);
            assert_eq!(
                st.metrics.caption_button_right_margin,
                m.caption_button_right
            );
            assert_eq!(st.metrics.frame_width, m.frame);
            assert_eq!(st.metrics.dialog_frame_width, m.dialog_frame);
            assert_eq!(st.metrics.taskbar_height, m.taskbar_height);
        }
    }

    #[test]
    fn radii_follow_the_parts() {
        for v in Variant::ALL {
            let r = v.scheme().metrics.radii;
            assert!(r.caption > 4.0, "{v:?}: {}", r.caption);
            assert!(r.tab_item > 0.0, "{v:?}: {}", r.tab_item);
            assert!(r.task_group_header >= 0.0, "{v:?}");
        }
    }
}
