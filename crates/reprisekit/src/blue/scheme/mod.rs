//! Blue colour schemes as named colours and metrics: one [`Scheme`] const per scheme, reached
//! through [`Variant::scheme`]. A [`Face`] names the colours one state of a part is drawn from.

mod carbon;
mod cobalt;
mod ember;
mod navy;
mod obsidian;
mod olive;
mod ramps;
mod sapphire;
mod silver;
mod slate;
mod steel;

pub use carbon::CARBON;
pub use cobalt::COBALT;
pub use ember::EMBER;
pub use navy::NAVY;
pub use obsidian::OBSIDIAN;
pub use olive::OLIVE;
pub use sapphire::SAPPHIRE;
pub use silver::SILVER;
pub use slate::SLATE;
pub use steel::STEEL;

use super::Variant;
use crate::blue::trayarrow::{
    ArrowRecipe, BEAD, BOLD, DIAGONAL, DOWN, FINE, OUTLINED, Shading, banded, lit,
};
use crate::theme::{Color, ColorScheme};
use ramps::stops;

pub fn scheme(v: Variant) -> &'static Scheme {
    match v {
        Variant::Blue => &NAVY,
        Variant::Olive => &OLIVE,
        Variant::Silver => &SILVER,
        Variant::Slate => &SLATE,
        Variant::Sapphire => &SAPPHIRE,
        Variant::Obsidian => &OBSIDIAN,
        Variant::Ember => &EMBER,
        Variant::Cobalt => &COBALT,
        Variant::Carbon => &CARBON,
        Variant::Steel => &STEEL,
    }
}

/// `v`'s scheme, or its dark twin when `dark`.
pub fn scheme_in(v: Variant, dark: bool) -> &'static Scheme {
    if dark { twin(v) } else { scheme(v) }
}

/// `v`'s dark twin; a variant without one yet keeps its light scheme.
fn twin(v: Variant) -> &'static Scheme {
    scheme(v)
}

impl Variant {
    pub fn scheme(self) -> &'static Scheme {
        scheme(self)
    }
}

/// (position, colour) stops, linear between, end colours held past the ends.
/// Positions run 0..=1 over the face at rest; pixel `i` of `n` sits at `(i + 0.5) / n`.
pub type Grad = &'static [(f32, Color)];

/// `g` as opaque premultiplied [`crate::shape::Stops`] positions.
pub(crate) fn stops(g: Grad) -> Vec<(f64, crate::shape::Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), crate::shape::premul(c, 255)))
        .collect()
}

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

/// A task button whose window wants attention, in every scheme: orange, a light band under its top
/// rim and a light left column, deepening to a burnt rim at its foot and right side.
pub const TASK_ATTENTION: Face = Face {
    rim: Color::rgb(0xA4, 0x4E, 0x08),
    inner: Color::rgb(0xFF, 0xB8, 0x66),
    halo: None,
    down: &[
        (0.02, Color::rgb(0xB8, 0x5A, 0x0C)),
        (0.06, Color::rgb(0xFF, 0xC8, 0x7C)),
        (0.10, Color::rgb(0xFF, 0xAE, 0x4C)),
        (0.18, Color::rgb(0xFA, 0x9A, 0x2E)),
        (0.26, Color::rgb(0xF6, 0x90, 0x20)),
        (0.86, Color::rgb(0xF2, 0x88, 0x1A)),
        (0.94, Color::rgb(0xDE, 0x72, 0x10)),
        (0.98, Color::rgb(0xA4, 0x4E, 0x08)),
    ],
    across: &[
        (0.0278, Color::rgb(0xB8, 0x5A, 0x0C)),
        (0.0833, Color::rgb(0xFF, 0xBA, 0x68)),
        (0.1389, Color::rgb(0xFA, 0x9E, 0x36)),
        (0.1944, Color::rgb(0xF6, 0x94, 0x22)),
        (0.6944, Color::rgb(0xF6, 0x94, 0x22)),
        (0.8611, Color::rgb(0xF2, 0x8C, 0x1C)),
        (0.9167, Color::rgb(0xE2, 0x7A, 0x14)),
        (0.9722, Color::rgb(0xA0, 0x4C, 0x0A)),
    ],
    mark: None,
};

/// The title on a [`TASK_ATTENTION`] button.
pub const TASK_ATTENTION_INK: Color = Color::rgb(0xFF, 0xFF, 0xFF);

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
    pub menu_band: Color,
    pub gradient_menu_band: Color,
    pub menu_band_top: Color,
}

impl System {
    /// These slots over Reprise Standard's for the rest (the band's text).
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
            menu_band: self.menu_band,
            gradient_menu_band: self.gradient_menu_band,
            menu_band_top: self.menu_band_top,
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

/// A window frame part, drawn `inactive` while its window is not active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameFaces {
    pub normal: Face,
    pub inactive: Face,
}

/// A caption button, also drawn `inactive` while its window is not active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
    pub inactive: Face,
}

/// A part with one look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestFaces {
    pub normal: Face,
}

/// A part that lights under the pointer and shows pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LitFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub checked: Face,
    pub checked_hot: Face,
    pub pressed: Face,
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

/// A part that greys out and has no pointer states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GreyedFaces {
    pub normal: Face,
    pub disabled: Face,
}

/// A [`LitFaces`] part that also greys out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LitGreyedFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Face,
}

/// The grip lines on a scrollbar thumb; `disabled` is None where the scheme draws none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollGripFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub disabled: Option<Face>,
}

/// A scrollbar track; it has no hot state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTrackFaces {
    pub normal: Face,
    pub pressed: Face,
    pub disabled: Face,
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

/// The caption help button, 21x21; `mark` is the question mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HelpButtonFaces {
    pub normal: Face,
    pub hot: Face,
    pub pressed: Face,
    pub inactive: Face,
}

/// A toolbar button or drop-down segment: flat at rest, nothing drawn while normal or disabled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarFaces {
    pub hot: Face,
    pub pressed: Face,
    pub checked: Face,
    pub checked_hot: Face,
}

/// A header sort arrow; `mark` is the arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderSortArrowFaces {
    pub normal: Face,
    pub checked: Face,
}

/// The Start menu's colours; rows lit and the left column take the [`System`] colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartMenu {
    /// Down the bands over and under the columns.
    pub band: Grad,
    /// The frame beside the columns.
    pub edge: Color,
    /// The line under the top band, at its middle.
    pub accent: Color,
    /// The user's name and the bottom buttons' labels, over `band_shadow` 1 px down and right.
    pub band_ink: Color,
    pub band_shadow: Color,
    /// The right column, its left edge and rules, and its labels.
    pub places: Color,
    pub places_edge: Color,
    pub places_ink: Color,
    /// The left column's rules.
    pub rule: Color,
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
    pub caption: FrameFaces,
    pub max_caption: FrameFaces,
    /// The left border, 4 px; `across` runs outside in.
    pub frame_left: FrameFaces,
    /// The right border, 4 px; `across` runs inside out.
    pub frame_right: FrameFaces,
    /// The bottom border, 4 rows; `down` runs top to bottom.
    pub frame_bottom: FrameFaces,
    /// The minimize caption button, 21x21; `mark` is the glyph.
    pub min_button: CaptionButtonFaces,
    /// The maximize caption button, 21x21; `mark` is the glyph.
    pub max_button: CaptionButtonFaces,
    /// The restore caption button, 21x21; `mark` is the glyph.
    pub restore_button: CaptionButtonFaces,
    /// The close caption button, 21x21, the red one; `mark` is the cross.
    pub close_button: CaptionButtonFaces,
    pub taskbar: RestFaces,
    pub start_button: LitFaces,
    pub task_button: TaskButtonFaces,
    /// The notification area: `down` is its band, `across` its 3 px left edge then the band.
    pub tray_area: RestFaces,
    pub push_button: PushButtonFaces,
    pub check_box: CheckBoxFaces,
    pub radio: RadioFaces,
    pub group_box: GreyedFaces,
    /// A scrollbar arrow button, 17x17; `mark` is the arrow.
    pub scroll_arrow_up: LitGreyedFaces,
    /// A scrollbar arrow button, 17x17; `mark` is the arrow.
    pub scroll_arrow_down: LitGreyedFaces,
    /// A scrollbar arrow button, 17x17; `mark` is the arrow.
    pub scroll_arrow_left: LitGreyedFaces,
    /// A scrollbar arrow button, 17x17; `mark` is the arrow.
    pub scroll_arrow_right: LitGreyedFaces,
    pub scroll_thumb_v: LitGreyedFaces,
    pub scroll_thumb_h: LitGreyedFaces,
    /// The grip lines on a vertical thumb.
    pub scroll_grip_v: ScrollGripFaces,
    /// The grip lines on a horizontal thumb.
    pub scroll_grip_h: ScrollGripFaces,
    /// A vertical scrollbar track; `across` runs left to right.
    pub scroll_track_v: ScrollTrackFaces,
    /// A horizontal scrollbar track; `down` runs top to bottom.
    pub scroll_track_h: ScrollTrackFaces,
    pub edit_border: GreyedFaces,
    pub list_border: Option<GreyedFaces>,
    /// A combo box drop button, 15x17; `mark` is the arrow.
    pub combo_drop: LitGreyedFaces,
    pub tab_item: TabItemFaces,
    /// The tab page under the tabs: `rim` is its border, `inner` the line inside
    /// it, `down` the body's fade over its height, `across` the soft shadow
    pub tab_pane: RestFaces,
    pub progress_bar: GreyedFaces,
    pub progress_chunk: GreyedFaces,
    pub trackbar_track: GreyedFaces,
    pub trackbar_thumb: LitGreyedFaces,
    pub trackbar_track_v: GreyedFaces,
    pub trackbar_thumb_v: LitGreyedFaces,
    /// A spinner's up button, 15x8; `mark` is the arrow.
    pub spin_up: LitGreyedFaces,
    /// A spinner's down button, 15x8; `mark` is the arrow.
    pub spin_down: LitGreyedFaces,
    pub help_button: HelpButtonFaces,
    pub size_box: RestFaces,
    pub calendar_band: GreyedFaces,
    pub toolbar_button: ToolbarFaces,
    /// The drop-down arrow segment beside a toolbar button.
    pub toolbar_drop: ToolbarFaces,
    pub toolbar_separator: RestFaces,
    pub rebar: RestFaces,
    pub rebar_band: RestFaces,
    pub header_item: LitFaces,
    pub header_filler: RestFaces,
    pub header_sort_arrow: HeaderSortArrowFaces,
    pub status_bar: RestFaces,
    pub status_pane: RestFaces,
    /// The task pane background; `down` runs its height.
    pub task_pane: RestFaces,
    pub task_group_header: RestFaces,
    pub task_group_body: RestFaces,
    /// A task pane group's collapse button; `mark` is the chevron.
    pub task_collapse: LitFaces,
    /// A task pane group's expand button; `mark` is the chevron.
    pub task_expand: RestFaces,
    pub explorer_bar_header: RestFaces,
    /// The explorer bar close button; `mark` is the cross.
    pub explorer_bar_close: LitFaces,
    pub places_bar: RestFaces,
    pub tray_arrow: ArrowRecipe,
    pub start_menu: StartMenu,
    /// The welcome screen: its dark top and bottom bands, the field between them with its glow
    /// on the left, the lit rules along the bands and down the middle, and its text.
    pub welcome_band: Color,
    pub welcome_field: Color,
    pub welcome_glow: Color,
    pub welcome_rule: Color,
    pub welcome_text: Color,
    /// The band behind a hot or open user tile, and the frame round each user's picture.
    pub welcome_tile_hot: Color,
    pub welcome_tile_frame: Color,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::style_for;
    use crate::style::{Part, State};

    /// Every face of a part, named by state.
    trait States {
        fn all(&self) -> Vec<(&'static str, &Face)>;
    }

    /// Destructures without `..`, so a state added to a type fails to build until it is listed.
    macro_rules! states {
        ($($t:ident { $($f:ident),* $(; $o:ident)? })*) => {$(
            impl States for $t {
                fn all(&self) -> Vec<(&'static str, &Face)> {
                    let $t { $($f,)* $($o,)? } = self;
                    #[allow(unused_mut)]
                    let mut v = vec![$((stringify!($f), $f)),*];
                    $(v.extend($o.as_ref().map(|f| (stringify!($o), f)));)?
                    v
                }
            }
        )*};
    }

    states! {
        FrameFaces { normal, inactive }
        CaptionButtonFaces { normal, hot, pressed, disabled, inactive }
        RestFaces { normal }
        LitFaces { normal, hot, pressed }
        TaskButtonFaces { normal, hot, checked, checked_hot, pressed }
        PushButtonFaces { normal, hot, pressed, disabled, default, focused }
        CheckBoxFaces {
            normal, hot, pressed, disabled, normal_checked, hot_checked, pressed_checked,
            disabled_checked, normal_mixed, hot_mixed, pressed_mixed, disabled_mixed
        }
        RadioFaces {
            normal, hot, pressed, disabled, normal_checked, hot_checked, pressed_checked,
            disabled_checked
        }
        GreyedFaces { normal, disabled }
        LitGreyedFaces { normal, hot, pressed, disabled }
        ScrollGripFaces { normal, hot, pressed; disabled }
        ScrollTrackFaces { normal, pressed, disabled }
        TabItemFaces { normal, hot, checked }
        HelpButtonFaces { normal, hot, pressed, inactive }
        ToolbarFaces { hot, pressed, checked, checked_hot }
        HeaderSortArrowFaces { normal, checked }
    }

    impl<T: States> States for Option<T> {
        fn all(&self) -> Vec<(&'static str, &Face)> {
            self.as_ref().map_or_else(Vec::new, T::all)
        }
    }

    /// (part, state, face) for every face of `s`. Destructures without `..`, so a part added to
    /// [`Scheme`] fails to build until it is listed here.
    fn faces(s: &Scheme) -> Vec<(&'static str, &'static str, &Face)> {
        macro_rules! parts {
            ($($p:ident),*; $($skip:ident),*) => {{
                let Scheme { $($p,)* $($skip: _,)* } = s;
                [$((stringify!($p), $p.all())),*]
            }};
        }
        parts![
            caption, max_caption, frame_left, frame_right, frame_bottom, min_button, max_button,
            restore_button, close_button, taskbar, start_button, task_button, tray_area,
            push_button, check_box, radio, group_box, scroll_arrow_up, scroll_arrow_down,
            scroll_arrow_left, scroll_arrow_right, scroll_thumb_v, scroll_thumb_h, scroll_grip_v,
            scroll_grip_h, scroll_track_v, scroll_track_h, edit_border, list_border, combo_drop,
            tab_item, tab_pane, progress_bar, progress_chunk, trackbar_track, trackbar_thumb,
            trackbar_track_v, trackbar_thumb_v, spin_up, spin_down, help_button, size_box,
            calendar_band, toolbar_button, toolbar_drop, toolbar_separator, rebar, rebar_band,
            header_item, header_filler, header_sort_arrow, status_bar, status_pane, task_pane,
            task_group_header, task_group_body, task_collapse, task_expand, explorer_bar_header,
            explorer_bar_close, places_bar;
            name, system, inks, metrics, tray_arrow, start_menu, welcome_band, welcome_field,
            welcome_glow, welcome_rule, welcome_text, welcome_tile_hot, welcome_tile_frame
        ]
        .into_iter()
        .flat_map(|(p, fs)| fs.into_iter().map(move |(st, f)| (p, st, f)))
        .collect()
    }

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
            let faces = faces(s);
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
            (Variant::Cobalt, 22, 12, 13),
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

    #[test]
    fn the_blue_schemes_start_band_is_the_standard_navy() {
        let (blue, standard) = (
            Variant::Blue.scheme().system.colors(),
            ColorScheme::default(),
        );
        assert_eq!(blue.menu_band, standard.menu_band);
        assert_eq!(blue.gradient_menu_band, standard.gradient_menu_band);
        assert_eq!(blue.menu_band_top, standard.menu_band_top);
    }

    /// Each scheme's band is its own, and dark enough all along for the white banner.
    #[test]
    fn every_scheme_tints_its_start_band_dark_under_a_white_banner() {
        let band = |v: Variant| {
            let s = v.scheme().system;
            [s.menu_band, s.gradient_menu_band, s.menu_band_top]
        };
        let luma =
            |c: Color| 0.299 * f64::from(c.r) + 0.587 * f64::from(c.g) + 0.114 * f64::from(c.b);
        for (i, v) in Variant::ALL.into_iter().enumerate() {
            for c in band(v) {
                assert!(luma(c) < 140.0, "{v:?} band {c:?} is dark");
            }
            for w in &Variant::ALL[..i] {
                assert_ne!(band(v), band(*w), "{v:?} and {w:?} share a band");
            }
        }
    }
}
