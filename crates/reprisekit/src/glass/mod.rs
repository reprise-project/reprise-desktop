//! Reprise Glass: translucent chrome over a blurred backdrop. The colour,
//! its intensity and whether it is translucent at all come from `[glass]`.

pub mod art;
mod balloon;
mod bar;
mod bars;
mod buttons;
mod clock;
mod controls;
mod copygraph;
pub mod dark;
mod doctab;
mod fields;
mod flyouts;
pub mod frame;
mod glyph;
mod header;
mod items;
mod menus;
pub mod notify;
mod orb;
pub mod overview;
pub mod page;
pub mod paint;
mod panel;
mod popups;
pub mod preview;
mod scrollbar;
mod selbox;
pub mod shadow;
mod sheet;
pub mod snapassist;
pub mod snappicker;
pub mod square;
pub mod startpanel;
mod startparts;
mod statusbar;
pub mod streak;
pub mod switcher;
pub mod taskbutton;
mod taskprogress;
pub mod tint;
mod toolbar;
#[cfg(test)]
mod twice;

use crate::mutex::lock;
use std::sync::{Arc, Mutex, OnceLock};

use crate::metrics::{ControlMetrics, PlacesMetrics, RebarMetrics};
use crate::raster::Rect;
use crate::style::{
    CaptionTitle, GlassSpec, PanelSpec, Part, PartImage, State, Style, TextSpec, TitleGlow,
    TrackMarks,
};
use crate::taskbar::{ClockBox, LabelledButton, OrbRise, StartMenuDock, TaskbarShape};
use crate::theme::{CaptionGroup, Color, ColorScheme, GroupPlace, Metrics};

/// `[glass] mode`: the controls, dialogs, menus and text under the glass in their
/// light or dark faces. The glass itself keeps its colour either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Light, Mode::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// `[glass] material`: how the glass treats what lies behind it. Frosted blurs
/// and tints; Smoked lets less through, darker, with white titles in either mode;
/// Molten bends it near the edges under a bright rim. Only the chrome changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Material {
    #[default]
    Frosted,
    Smoked,
    Molten,
}

impl Material {
    pub const ALL: [Material; 3] = [Material::Frosted, Material::Smoked, Material::Molten];

    pub fn as_str(self) -> &'static str {
        match self {
            Material::Frosted => "frosted",
            Material::Smoked => "smoked",
            Material::Molten => "molten",
        }
    }

    pub fn parse(s: &str) -> Option<Material> {
        Material::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// `[glass] notification_style`: the notification sheet's shape. Halo breaks the
/// app icon out of the top-left edge in its colour; Droplet is one row until hovered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NotifyStyle {
    #[default]
    Lozenge,
    Halo,
    Droplet,
}

impl NotifyStyle {
    pub const ALL: [NotifyStyle; 3] = [
        NotifyStyle::Lozenge,
        NotifyStyle::Halo,
        NotifyStyle::Droplet,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            NotifyStyle::Lozenge => "lozenge",
            NotifyStyle::Halo => "halo",
            NotifyStyle::Droplet => "droplet",
        }
    }

    pub fn parse(s: &str) -> Option<NotifyStyle> {
        NotifyStyle::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// `[glass] frame`: the window decorations' shape. Rounded has a caption with
/// rounded corners and joined glossy buttons; Square has square corners and flat,
/// separate caption buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Frame {
    #[default]
    Rounded,
    Square,
}

impl Frame {
    pub const ALL: [Frame; 2] = [Frame::Rounded, Frame::Square];

    pub fn as_str(self) -> &'static str {
        match self {
            Frame::Rounded => "rounded",
            Frame::Square => "square",
        }
    }

    pub fn parse(s: &str) -> Option<Frame> {
        Frame::ALL.into_iter().find(|f| f.as_str() == s)
    }
}

/// The environment variable a compositor hands its clients `[glass] mode` in.
pub const MODE_ENV: &str = "REPRISE_GLASS_MODE";

/// The environment variable a compositor hands its clients `[glass] material` in.
pub const MATERIAL_ENV: &str = "REPRISE_GLASS_MATERIAL";

/// `[glass]`: the colour, its intensity (0..100), translucency, `blur`, the
/// reflection streaks, `fast_drag`, `blur_frames`, `blur_rate`, `orb_glow`,
/// `mode`, `material` and `frame`. Opaque glass needs no backdrop. Defaults: Sky
/// at 42, translucent, blur 2 with streaks, fast drag, frames behind glass shown,
/// 30 re-blurs a second, full orb glow, light, Frosted, Rounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Settings {
    pub color: Color,
    pub intensity: u8,
    pub transparency: bool,
    /// How strongly the backdrop blurs, from 1 to [`BLUR_MAX`]. Draws nothing of its
    /// own, so styles are shared across it (`cached`).
    pub blur: u8,
    /// The reflection streaks over the backdrop ([`streak`]); drawn into the
    /// backdrop, so styles are shared across it too.
    pub streaks: bool,
    /// While a window is dragged or sized, blur with fewer passes and show the glass
    /// a frame behind; full blur again once let go. Styles are shared across it.
    pub fast_drag: bool,
    /// Glass over another window's frame, a dialog's or the taskbar shows it blurred,
    /// at the cost of a blur of its own; off, glass shows what lies under that chrome.
    pub blur_frames: bool,
    /// At most how many times a second what clients draw near the glass, or desktop
    /// water, re-blurs it; 0 for every frame.
    pub blur_rate: u16,
    /// How strongly the Start orb's icon glows, 0 to 100 percent; 0 draws none.
    /// Drawn by the taskbar (`TaskbarState::orb_glow`), so styles are shared.
    pub orb_glow: u8,
    pub mode: Mode,
    pub material: Material,
    pub frame: Frame,
    pub notification: NotifyStyle,
}

pub const BLUR_MAX: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub name: &'static str,
    pub color: Color,
    pub intensity: u8,
    pub balance: tint::Balance,
}

pub use tint::SWATCHES;

pub fn swatch(name: &str) -> Option<Color> {
    SWATCHES.iter().find(|w| w.name == name).map(|w| w.color)
}

/// `[glass] color`'s colour: a swatch's name or "#RRGGBB".
pub fn color_named(s: &str) -> Option<Color> {
    if s.starts_with('#') {
        Color::parse(s)
    } else {
        swatch(s)
    }
}

pub fn swatch_names() -> impl Iterator<Item = &'static str> {
    SWATCHES.iter().map(|w| w.name)
}

pub fn swatch_name(c: Color) -> Option<&'static str> {
    SWATCHES.iter().find(|w| w.color == c).map(|w| w.name)
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            color: SWATCHES[0].color,
            intensity: 42,
            transparency: true,
            blur: 2,
            streaks: true,
            fast_drag: true,
            blur_frames: true,
            blur_rate: 30,
            orb_glow: 100,
            mode: Mode::Light,
            material: Material::Frosted,
            frame: Frame::Rounded,
            notification: NotifyStyle::default(),
        }
    }
}

/// Reprise Glass's style for `s`, built on first use and shared while among the
/// last few asked for (`cached`).
pub fn style(s: &Settings) -> Arc<Style> {
    cached(s, false)
}

/// The style for `s` under the small 30 px bar (`[taskbar] small_icons`); the
/// bar's parts at its height ([`paint::small_bar`]).
pub fn small_bar_style(s: &Settings) -> Arc<Style> {
    cached(s, true)
}

/// How many styles `cached` keeps. A style holds about half a megabyte of art;
/// the live theme holds its own, so a dropped one is only rebuilt when asked for.
/// Styles built for a scaled output are kept with it ([`Style::at_scale`]).
const STYLES_KEPT: usize = 8;

fn cached(s: &Settings, small_bar: bool) -> Arc<Style> {
    type Styles = Kept<(Settings, bool), Arc<Style>>;
    static STYLES: OnceLock<Mutex<Styles>> = OnceLock::new();
    let mut styles = lock(STYLES.get_or_init(|| Mutex::new(Kept::new(STYLES_KEPT))));
    let key = Settings {
        blur: Settings::default().blur,
        streaks: Settings::default().streaks,
        fast_drag: Settings::default().fast_drag,
        blur_frames: Settings::default().blur_frames,
        blur_rate: Settings::default().blur_rate,
        orb_glow: Settings::default().orb_glow,
        notification: NotifyStyle::default(),
        ..*s
    };
    styles.get_or_insert_with((key, small_bar), || {
        Arc::new(made(&key, small_bar, 1).with_rescale(move |n| made(&key, small_bar, n)))
    })
}

fn made(s: &Settings, small_bar: bool, scale: i32) -> Style {
    let full = build_at(s, scale);
    let full = if s.frame == Frame::Square {
        square::frame(full, scale)
    } else {
        full
    };
    if small_bar {
        small(full, s, scale)
    } else {
        full
    }
}

struct Kept<K, V> {
    cap: usize,
    entries: Vec<(K, V)>,
}

impl<K: PartialEq, V: Clone> Kept<K, V> {
    fn new(cap: usize) -> Self {
        Kept {
            cap,
            entries: Vec::new(),
        }
    }

    fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> V {
        let entry = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => self.entries.remove(i),
            None => {
                if self.entries.len() >= self.cap {
                    self.entries.remove(0);
                }
                (key, make())
            }
        };
        let v = entry.1.clone();
        self.entries.push(entry);
        v
    }
}

/// `full` with the small bar's parts in place of the 40 px bar's, its
/// buttons lit by the spot on their own frames, at the small pitch, its
/// orb standing above the bar. Its labelled buttons have no art of their
/// own: they draw its task button, spaced as elsewhere. Its drawn parts
/// are painted at `scale`, as [`build_at`]'s.
fn small(full: Style, s: &Settings, scale: i32) -> Style {
    let mut st = paint::small_bar(s, scale)
        .into_iter()
        .fold(full, |st, (part, img)| st.with_part(part, img))
        .without_part(Part::LabelRow)
        .without_part(Part::TaskLabel)
        .without_part(Part::TaskLabelEdge)
        .with_task_glow(paint::small_task_glow(s))
        .with_painted_task_spot({
            let s = *s;
            move || taskbutton::small_task_spot(&s, scale)
        });
    st.taskbar.labelled = LabelledButton::CLASSIC;
    st.taskbar.icon_button = Some(SMALL_ICON_BUTTON);
    st.taskbar.task_icon = crate::icon::ICON_SIZE;
    st.taskbar.orb_rise = Some(SMALL_ORB_RISE);
    st
}

#[cfg(test)]
pub(crate) fn build(s: &Settings) -> Style {
    build_at(s, 1)
}

/// [`build`] with the window frame, its caption buttons, the taskbar, the
/// orb and its task buttons, the Start panel and the popups painted at
/// `scale` device pixels per design pixel; every other part is drawn at
/// the design's size and magnified by the draw.
fn build_at(s: &Settings, scale: i32) -> Style {
    let is_dark = s.mode == Mode::Dark;
    let (title_ink, title) = caption_ink(s.material, s.mode);
    let lazy = |paint: fn(&Settings, i32) -> PartImage| {
        let s = *s;
        move || paint(&s, scale)
    };
    let owned = *s;
    // Dark Glass remaps only the control art; the glass keeps its colour,
    // and so does the progress bar's green fill.
    let ink = move |(part, img): (Part, PartImage)| {
        let remap = is_dark && part != Part::ProgressChunk;
        (part, if remap { dark::remap_part(img) } else { img })
    };
    let style = Style::new(
        if is_dark { "glass-dark" } else { "glass" },
        colors(s.mode),
        metrics(),
        caption_text(title_ink),
        start_text(),
        Color::rgb(0xFF, 0xFF, 0xFF),
        taskbar_shape(),
    )
    .with_glass(GlassSpec {
        corner: CORNER,
        panel: PanelSpec {
            shadow: preview::GRID.shadow,
            corner: preview::GRID.corner,
        },
        lift: [
            tint::lift(s, tint::Surface::Active),
            tint::lift(s, tint::Surface::Inactive),
        ],
    })
    .with_caption_title(title)
    .with_label_glow(label_glow(s.material, s.mode))
    .with_chrome_colors(colors(Mode::Light))
    .with_text_color(Part::CheckBox, State::Disabled, greyed_choice_text(s.mode))
    .with_text_color(Part::Radio, State::Disabled, greyed_choice_text(s.mode))
    .with_text_color(
        Part::PushButton,
        State::Disabled,
        greyed_button_text(s.mode),
    )
    .with_text_color(
        Part::EditBorder,
        State::Disabled,
        greyed_choice_text(s.mode),
    )
    .with_text_color(Part::Tooltip, State::Normal, tooltip_ink(s.mode))
    .with_text_color(
        Part::CalendarBand,
        State::Normal,
        colors(s.mode).window_text,
    )
    .with_text_color(
        Part::CalendarBand,
        State::Disabled,
        colors(s.mode).gray_text,
    )
    .with_painted(Part::Caption, lazy(frame::caption))
    .with_painted(Part::MaxCaption, lazy(frame::max_caption))
    .with_painted(Part::FrameLeft, lazy(frame::frame_left))
    .with_painted(Part::FrameRight, lazy(frame::frame_right))
    .with_painted(Part::FrameBottom, lazy(frame::frame_bottom))
    .with_painted(Part::WindowSheet, lazy(frame::sheet))
    .with_painted(Part::ClientGlass, lazy(frame::client_glass))
    .with_painted(Part::MaxClientGlass, lazy(frame::max_client_glass))
    .with_client_glass_edges(Arc::new(frame::client_glass_edges(s)))
    .with_painted(
        Part::GlassSeparator,
        lazy(|_, d| toolbar::glass_separator(d)),
    )
    .with_painted(Part::CaptionGroup, lazy(buttons::caption_group))
    .with_painted(Part::MaxCaptionGroup, lazy(buttons::max_caption_group))
    .with_painted(Part::CloseButton, lazy(buttons::dialog_close))
    .with_painted(Part::HelpButton, lazy(buttons::help_strip))
    .with_painted(Part::MinCloseGroup, lazy(buttons::min_close))
    .with_joined_help()
    .with_button_side({
        let s = *s;
        move |side| buttons::sided(&s, scale, side)
    })
    .with_painted(Part::Taskbar, lazy(|s, d| bar::taskbar(s, BAR_HEIGHT, d)))
    .with_painted(Part::StartButton, lazy(paint::start_button))
    .with_painted(Part::StartAbove, lazy(paint::start_above))
    .with_painted(
        Part::TaskButton,
        lazy(|s, d| taskbutton::buttons_with_attention(s, BUTTON, d)),
    )
    .with_painted(
        Part::TaskStack,
        lazy(|s, d| taskbutton::stacks(s, BUTTON, true, d)),
    )
    .with_painted(
        Part::LabelRow,
        lazy(|s, d| bar::label_row(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::TaskLabel,
        lazy(|s, d| taskbutton::label_with_attention(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::TaskLabelEdge,
        lazy(|s, d| taskbutton::label_edges_with_attention(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::SlotEdge,
        lazy(|s, d| bar::slot_edge(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::SlotLight,
        lazy(|_, d| bar::slot_light(ICON_BUTTON.0, d)),
    )
    .with_painted(
        Part::SlotBand,
        lazy(|s, d| bar::slot_band(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::PagerTile,
        lazy(|s, d| taskbutton::pager_tiles(s, BUTTON, d)),
    )
    .with_painted(
        Part::TrayArea,
        lazy(|s, d| bar::tray_area(s, BAR_HEIGHT, d)),
    )
    .with_painted(
        Part::ShowDesktop,
        lazy(|s, d| bar::show_desktop(s, BAR_HEIGHT, d)),
    )
    .with_task_glow(paint::task_glow(s))
    .with_start_bloom(orb::bloom)
    .with_clock_face(clock::painter(s.mode))
    .with_copy_graph(copygraph::look(s.mode))
    .with_task_progress(std::sync::Arc::new(taskprogress::paint))
    .with_painted_task_spot(move || taskbutton::task_spot(&owned, scale))
    .without_task_nudge()
    .with_greyed_field_frame()
    .with_controls(controls())
    .with_painted(Part::PreviewFrame, lazy(preview::frame_part))
    .with_painted(Part::PreviewThumb, lazy(|_, d| preview::thumb_part(d)))
    .with_painted(Part::PreviewClose, lazy(|_, d| popups::preview_close(d)))
    .with_painted(Part::SwitcherFrame, lazy(switcher::frame_part))
    .with_painted(Part::SwitcherIconFrame, lazy(switcher::icon_frame_part))
    .with_painted(Part::SwitcherSelect, lazy(|_, d| switcher::select_part(d)))
    .with_painted(Part::SwitcherHot, lazy(|_, d| switcher::hot_part(d)))
    .with_painted(Part::SwitcherShadow, lazy(|_, d| switcher::shadow_part(d)))
    .with_painted(Part::SnapFrame, lazy(snappicker::frame_part))
    .with_painted(Part::FlyoutShadow, lazy(|_, d| flyouts::shadow_part(d)));
    let style = GROUPS.into_iter().fold(style, |st, (parts, paint)| {
        st.with_painted_group(parts, move || paint(scale).into_iter().map(ink).collect())
    });
    // A program draws only some of these, so each is painted on its own.
    let style = fields::painters()
        .into_iter()
        .chain(items::painters())
        .fold(style, |st, (part, paint)| {
            st.with_painted(part, move || ink((part, paint(scale))).1)
        });
    let style = if is_dark {
        style
            .with_painted(Part::PlacesTile, move || items::dark_places(scale))
            .with_painted(Part::ListItem, move || items::dark_items(scale))
    } else {
        style
    };
    let style = style
        .with_painted_group(&startpanel::PARTS, move || startpanel::parts(&owned, scale))
        .with_picked_text_color(Part::StartPanel, State::Normal, move || {
            startpanel::right_ink(&owned)
        });
    bars::parts()
        .into_iter()
        .fold(style, |st, (part, paint)| {
            st.with_painted(part, move || ink((part, paint(scale))).1)
        })
        .with_combo_edit_border()
        .with_group_label(sheet::GROUP_LABEL)
        .with_smooth_progress()
        .with_track_marks(track_marks(s.mode))
        .with_page(page::inks(s.mode))
        .with_drop_list(fields::drop_list(s.mode))
        .with_menu(menus::look(s.mode, fields::drop_list(s.mode).shadow))
        .with_options_menu(menus::options_look(s.mode))
        .with_selection_box(selbox::BOX)
        .with_caption_tabs(caption_tabs(title_ink))
        .with_balloon(balloon::look(s.mode, fields::drop_list(s.mode).shadow))
        .with_notify(notify::look(s.mode))
        .with_flyout(flyouts::look())
        .with_painted(Part::DesktopItem, move || items::desktop_part(scale))
}

/// A group of control parts and the painter that draws them together at a scale.
type Group = (&'static [Part], fn(i32) -> Vec<(Part, PartImage)>);

/// Glass's control parts, each group painted the first time one of its parts is drawn.
const GROUPS: [Group; 11] = [
    (&[Part::CheckBox, Part::Radio], |d| {
        controls::parts(d).into()
    }),
    (
        &[
            Part::PushButton,
            Part::GroupBox,
            Part::TabItem,
            Part::TabPane,
        ],
        |d| sheet::parts(d).into(),
    ),
    (
        &[
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbV,
            Part::ScrollThumbH,
            Part::ScrollGripV,
            Part::ScrollGripH,
            Part::ScrollTrackV,
            Part::ScrollTrackH,
        ],
        |d| scrollbar::parts(d).into(),
    ),
    (
        &[
            Part::HeaderItem,
            Part::HeaderFiller,
            Part::HeaderSortArrow,
            Part::ListBorder,
            Part::WellFrame,
        ],
        |d| header::parts(d).into(),
    ),
    (&[Part::StatusBar, Part::StatusPane, Part::SizeBox], |d| {
        statusbar::parts(d).into()
    }),
    (
        &[
            Part::Tooltip,
            Part::CalendarBand,
            Part::PlacesBar,
            Part::CalendarFrame,
        ],
        |d| popups::parts(d).into(),
    ),
    (
        &[
            Part::ToolbarButton,
            Part::ToolbarDrop,
            Part::ToolbarSeparator,
            Part::Rebar,
            Part::RebarBand,
            Part::RebarGripper,
        ],
        |d| toolbar::parts(d).into(),
    ),
    (
        &[
            Part::MenuItem,
            Part::MenuCheck,
            Part::MenuCheckMark,
            Part::MenuRadioMark,
            Part::MenuBar,
            Part::MenuBarItem,
        ],
        |d| menus::parts(d).into(),
    ),
    (&[Part::BalloonClose], |d| balloon::parts(d).into()),
    (&[Part::GlassTab], |d| doctab::parts(d).into()),
    (
        &[Part::CalendarToday, Part::CalendarPrev, Part::CalendarNext],
        |d| flyouts::parts(d).into(),
    ),
];

/// Glass's caption tabs for titles inked `ink` (the focused window's, the
/// others'): rounded 3 px washes over the glass, white under dark titles
/// and black under light ones, so a title keeps its contrast in any
/// material; alphas out of FF. Under dark titles the selected tab takes 80
/// of white in a line of 50 of black, the one under the pointer 50, the
/// rest 28 and a window held over the strip 60 in the line; the close box
/// (14 px) A0 of white under the pointer. Under light titles each wash is
/// black (60, 40, 20, 50) and the line white.
fn caption_tabs(ink: [Color; 2]) -> crate::style::CaptionTabLook {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let black = Color::rgb(0, 0, 0);
    let c = ink[0];
    let light = u32::from(c.r) + u32::from(c.g) + u32::from(c.b) > 3 * 0x80;
    let (wash, line) = if light {
        (black, white)
    } else {
        (white, black)
    };
    crate::style::CaptionTabLook {
        radius: 3,
        selected: (wash, if light { 0x60 } else { 0x80 }),
        outline: (line, 0x50),
        hot: (wash, if light { 0x40 } else { 0x50 }),
        rest: (wash, if light { 0x20 } else { 0x28 }),
        ghost: (wash, if light { 0x50 } else { 0x60 }),
        close: 14,
        close_hot: (white, if light { 0x50 } else { 0xA0 }),
        close_held: (black, 0x30),
        other_ink: (ink[0], ink[1]),
    }
}

/// Dark Glass's text: EFEEEF.
pub const DARK_TEXT: Color = Color::rgb(0xEF, 0xEE, 0xEF);

/// Glass's system colours over Reprise Standard's: an EFEEEF dialog face,
/// 9D9E9E shadows (a greyed static label's grey over its white copy, and
/// a separator line), and a 2E8CFF selection. The 3D light line and the
/// menus take the face too, so a raised or sunken edge has no line darker
/// than the face inside it and a menu bar matches its dropdowns. The rest
/// stay Standard's, links (`hot_light`) in its 000080 navy among them.
///
/// The dark ones take their face and window from the light face and white
/// through the control art's remap, so a control's baked face meets the
/// painted face without a seam; EFEEEF text, 878787 greyed text, 62A8FF
/// links, the same 2E8CFF selection.
fn colors(mode: Mode) -> ColorScheme {
    let light = ColorScheme {
        button_face: DIALOG_FACE,
        button_light: DIALOG_FACE,
        menu: DIALOG_FACE,
        button_shadow: Color::rgb(0x9D, 0x9E, 0x9E),
        hilight: Color::rgb(0x2E, 0x8C, 0xFF),
        ..ColorScheme::default()
    };
    if mode == Mode::Light {
        return light;
    }
    let face = dark::remap_color(DIALOG_FACE);
    ColorScheme {
        button_face: face,
        button_light: face,
        menu: face,
        menu_bar: face,
        info_bk: face,
        window: dark::remap_color(Color::rgb(0xFF, 0xFF, 0xFF)),
        button_hilight: Color::rgb(0x48, 0x48, 0x48),
        button_shadow: Color::rgb(0x58, 0x58, 0x58),
        button_dk_shadow: Color::rgb(0x0A, 0x0A, 0x0A),
        window_frame: Color::rgb(0x58, 0x58, 0x58),
        window_text: DARK_TEXT,
        button_text: DARK_TEXT,
        menu_text: DARK_TEXT,
        info_text: DARK_TEXT,
        gray_text: Color::rgb(0x87, 0x87, 0x87),
        hot_light: Color::rgb(0x62, 0xA8, 0xFF),
        ..light
    }
}

/// A tooltip's text in `mode`: 575757, 999999 dark (575757 remapped).
fn tooltip_ink(mode: Mode) -> Color {
    match mode {
        Mode::Light => popups::TOOLTIP_INK,
        Mode::Dark => dark::remap_color(popups::TOOLTIP_INK),
    }
}

/// The trackbar's ticks in `mode`: C1C2C2, 606060 dark.
fn track_marks(mode: Mode) -> TrackMarks {
    match mode {
        Mode::Light => TRACK_MARKS,
        Mode::Dark => TrackMarks {
            ink: Color::rgb(0x5E, 0x5E, 0x5E),
            ..TRACK_MARKS
        },
    }
}

/// The trackbar's track on the layout's channel, one row under the
/// thumb's cap; C1C2C2 ticks 21 rows under the trackbar's top, 3 px long
/// and 4 px at the ends, under every `tick_every`th position.
pub const TRACK_MARKS: TrackMarks = TrackMarks {
    drop: 0,
    tick_y: 21,
    tick_len: 3,
    end_len: 4,
    end_len_v: 4,
    sparse: true,
    ink: Color::rgb(0xC1, 0xC2, 0xC2),
};

/// The dialog and sheet face.
pub const DIALOG_FACE: Color = Color::rgb(0xEF, 0xEE, 0xEF);

/// A greyed check box's, radio's, edit's or combo's text: inked once,
/// flat, in 6B6B6B, with no white copy under it. A greyed static label
/// stays etched, as in every look.
pub const GREYED_CHOICE_TEXT: Color = Color::rgb(0x6B, 0x6B, 0x6B);

/// A greyed push button's label: inked once, flat, in 838383.
pub const GREYED_BUTTON_TEXT: Color = Color::rgb(0x7F, 0x80, 0x80);

/// A greyed check box's, radio's, edit's or combo's text in `mode`: 6B6B6B, 878787 dark.
fn greyed_choice_text(mode: Mode) -> Color {
    match mode {
        Mode::Light => GREYED_CHOICE_TEXT,
        Mode::Dark => Color::rgb(0x87, 0x87, 0x87),
    }
}

/// A greyed push button's label in `mode`: 838383, 767777 dark.
fn greyed_button_text(mode: Mode) -> Color {
    match mode {
        Mode::Light => GREYED_BUTTON_TEXT,
        Mode::Dark => Color::rgb(0x76, 0x77, 0x77),
    }
}

/// The rounded corners of a Glass window, all four alike: from the outer
/// row inward, 5, 3, 2, 1, 1 and 1 px of each row lie outside the frame.
/// The corner is aliased: the first pixel inside is a full outline pixel.
pub const CORNER: &[i32] = &[5, 3, 2, 1, 1, 1];

/// Glass's control sizes: Classic's, but for a 17 px scrollbar whose
/// thumb is never shorter than its two 5 px ends, upright or lying, the
/// radio's circle in the check box's column (a pixel left of Classic's
/// circle), and a check box's and a radio's label 16 px right of the
/// box's or circle's left edge. A check box label that runs onto a second
/// line sits a pixel higher, both lines, its second line under its first.
/// A list header is 24 px with its sort arrow over the title, and a
/// status bar 23 px. An open combo list's rows are 15 px. A places bar tile is
/// 63 px, its icon 2 px down and its label 1 px under it. The clock popup
/// has a 129 px analog clock beside its month.
/// The rebar starts flush with the client's top, draws no line over its
/// first band, and closes off with its own line in the row under it; the
/// row after that is the view's, where a list's header starts.
fn controls() -> ControlMetrics {
    ControlMetrics {
        scroll: 17,
        thumb_min: 10,
        check_label: 16,
        check_wrap: 16,
        check_wrap_dy: -1,
        radio_x: -1,
        radio_label: 15,
        list_header: 24,
        header_arrow_over: true,
        status_bar: 23,
        combo_row: 15,
        flyout_border: 8,
        places: PlacesMetrics {
            tile: 59,
            icon_top: 2,
            label_gap: 1,
            line_lead: 0,
            ..PlacesMetrics::CLASSIC
        },
        clock_face: clock::SIDE,
        rebar: RebarMetrics {
            top: 0,
            first_line: false,
            closing: 2,
            band_top: 0,
        },
        ..ControlMetrics::CLASSIC
    }
}

/// Glass's pixel metrics: 8 px borders under a 22 px caption, a dialog's
/// as wide as a window's, the caption buttons joined into one strip, and a
/// 40 px taskbar.
fn metrics() -> Metrics {
    Metrics {
        frame_width: 8,
        dialog_frame_width: 8,
        caption_height: 22,
        taskbar_height: BAR_HEIGHT,
        start_button_width: 54,
        caption_group: Some(CAPTION_GROUP),
        ..Metrics::default()
    }
}

/// The caption buttons, 105x20 with their halo: cells 30, 28 and 49 px
/// wide sharing 1 px dividers, the strip's right edge 6 px in from the
/// frame's and its top 1 px down, on the frame's light line. Maximized,
/// the strip loses its top row to the screen's edge: 19 px tall at the
/// top, 2 px in from the right.
pub const CAPTION_GROUP: CaptionGroup = CaptionGroup {
    widths: [30, 28, 49],
    divider: 1,
    restored: GroupPlace {
        right: 6,
        top: 1,
        height: 20,
    },
    maximized: GroupPlace {
        right: 2,
        top: 0,
        height: 19,
    },
};

/// The caption's icon and title. Restored, the 16 px icon's top-left is
/// at window (8, 7) and the title starts at x 30 with its baseline under
/// row 18 (its capitals' last row), a white glow under it. Maximized, the
/// band loses the 8 px border left and 4 rows of the top, so both sit that
/// much further up and left: the icon at (0, 3), the title at x 22 over
/// row 14.
pub const CAPTION_TITLE: CaptionTitle = CaptionTitle {
    icon: [(0, -1), (0, 3)],
    text_x: 22,
    baseline: Some([11, 15]),
    glow: Some(paint::TITLE_GLOW),
};

/// [`CAPTION_TITLE`] with [`paint::TITLE_GLOW_DARK`]'s soft black edge
/// under the white title in place of the white glow.
pub const CAPTION_TITLE_DARK: CaptionTitle = CaptionTitle {
    glow: Some(paint::TITLE_GLOW_DARK),
    ..CAPTION_TITLE
};

/// The caption title's ink, focused and not, and its title placement with
/// the glow under it, by how bright the glass is: black over
/// [`CAPTION_TITLE`]'s white glow on light glass (Frosted and Molten in
/// light mode), white (DEDFDF unfocused, a grey that still reads over the
/// paler glass of a window that is not the focused one) over
/// [`CAPTION_TITLE_DARK`]'s black edge on dark glass (dark mode, and
/// Smoked in either mode).
pub fn caption_ink(material: Material, mode: Mode) -> ([Color; 2], CaptionTitle) {
    if light_glass(material, mode) {
        ([Color::rgb(0, 0, 0); 2], CAPTION_TITLE)
    } else {
        (
            [Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(0xDE, 0xDF, 0xDF)],
            CAPTION_TITLE_DARK,
        )
    }
}

/// The halo under labels a client inks in [`caption_ink`] over its frame's glass: the title's
/// white glow on light glass, [`paint::LABEL_HALO_DARK`] under white labels on dark glass.
pub fn label_glow(material: Material, mode: Mode) -> TitleGlow {
    match light_glass(material, mode) {
        true => paint::TITLE_GLOW,
        false => paint::LABEL_HALO_DARK,
    }
}

fn light_glass(material: Material, mode: Mode) -> bool {
    mode == Mode::Light && material != Material::Smoked
}

/// The window title: the caption face at 12 px (9 pt), no shadow, in `ink`
/// focused and not ([`caption_ink`]).
fn caption_text([color, inactive]: [Color; 2]) -> TextSpec {
    TextSpec {
        family: vec!["Segoe UI".into(), "Noto Sans".into(), "DejaVu Sans".into()],
        size_px: 12,
        family_sizes: Vec::new(),
        bold: false,
        italic: false,
        antialias: true,
        full_hinting: false,
        color,
        inactive,
        shadow: None,
        ink_gamma: 1.0,
    }
}

/// The Start label, in the caption's face, white over the bar's glass.
fn start_text() -> TextSpec {
    caption_text([Color::rgb(0xFF, 0xFF, 0xFF); 2])
}

/// Glass's taskbar shape: Start is a 54 px orb and every button runs the
/// bar's full height, the first 5 px after the orb, so at x 59; the
/// notification well runs to the screen's edge (or the Show desktop
/// strip). An icon-only task button is 60 px wide at a 62 px pitch; the
/// Show desktop strip is 15 px wide. Task buttons, labelled or not, show
/// 32 px icons. The orb shows a 24 px `start-here`;
/// its disc stays inside the bar, but its lit glow reaches above it
/// ([`ORB_RISE`]).
/// On a 40 px bar the clock takes two lines, the time's baseline 17 px
/// down and the date's 34. The Start menu stands on the bar's top edge and
/// casts no shadow on the bar. Labelled buttons abut ([`LABELLED`]).
fn taskbar_shape() -> TaskbarShape {
    TaskbarShape {
        start: Rect::new(0, 0, 0, 0),
        button_top: 0,
        button_bottom: 0,
        start_gap: 5,
        well_full_height: true,
        well_lead: 8,
        tray_gap: 4,
        tray_low: false,
        side_tray_inset: 0,
        tray_arrow: None,
        clock: ClockBox::CLASSIC,
        start_icon: 24,
        task_icon: 32,
        icon_button: Some(ICON_BUTTON),
        show_desktop_width: Some(bar::STRIP_WIDTH),
        orb: true,
        orb_rise: Some(ORB_RISE),
        two_line_clock: Some([17, 34]),
        start_menu: StartMenuDock::OnTop,
        labelled: LABELLED,
    }
}

/// Glass's labelled task buttons abut, sharing the dark line between
/// them. The icon is centred in a 32 px box 10 px in, and the title
/// starts 4 px after the box, 46 px in.
pub const LABELLED: LabelledButton = LabelledButton {
    gap: 0,
    icon_x: 10,
    icon_box: 32,
    text_x: 46,
};

/// The 40 px bar's height.
pub const BAR_HEIGHT: i32 = 40;

/// The 40 px bar's icon-only task button: its width and pitch.
pub const ICON_BUTTON: (i32, i32) = (60, 62);

/// The 40 px bar's task button frame: its width and height.
const BUTTON: (i32, i32) = (ICON_BUTTON.0, BAR_HEIGHT);

/// `[taskbar] small_icons` under Glass: a 30 px bar with icon-only task
/// buttons 44 px wide at a 46 px pitch, their icons 16 px. Classic and Blue already run their
/// own small icons at their usual height, so the key changes nothing there;
/// only Glass has a taller default bar to shrink.
pub const SMALL_TASKBAR_HEIGHT: i32 = 30;
/// The icon-only task button's width and pitch when `SMALL_TASKBAR_HEIGHT`
/// is in effect.
pub const SMALL_ICON_BUTTON: (i32, i32) = (44, 46);
/// The 40 px bar's orb: its 36 px disc inside the bar, its art reaching
/// 6 px above the bar under the pointer and held.
pub const ORB_RISE: OrbRise = OrbRise {
    rise: 0,
    diameter: 36,
    reach: 6,
};

/// The small bar's orb: the 40 px bar's 36 px disc, its top 5 px above the
/// 30 px bar (its bottom row runs off the bar's foot), its art reaching
/// 13 px above the bar under the pointer.
pub const SMALL_ORB_RISE: OrbRise = OrbRise {
    rise: 5,
    diameter: 36,
    reach: 13,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn dark_settings() -> Settings {
        Settings {
            mode: Mode::Dark,
            ..Settings::default()
        }
    }

    #[test]
    fn dark_glass_is_a_look_of_its_own() {
        assert_eq!(style(&dark_settings()).name, "glass-dark");
        assert_eq!(style(&Settings::default()).name, "glass");
        assert_eq!(
            small_bar_style(&dark_settings()).name,
            "glass-dark",
            "the small bar keeps dark"
        );
        assert_eq!(
            small_bar_style(&dark_settings()).colors,
            style(&dark_settings()).colors,
            "the small bar keeps the dark scheme"
        );
    }

    /// Links stay Standard's navy on the light face and turn 62A8FF on
    /// the dark one; either way a channel differs from the face by at
    /// least 96, the bar every Classic scheme's text clears.
    #[test]
    fn links_stand_apart_from_the_face_in_both_modes() {
        for (mode, link) in [
            (Mode::Light, Color::rgb(0x00, 0x00, 0x80)),
            (Mode::Dark, Color::rgb(0x62, 0xA8, 0xFF)),
        ] {
            let cs = colors(mode);
            assert_eq!(cs.hot_light, link, "{mode:?}");
            let (a, b) = (cs.hot_light, cs.button_face);
            let d =
                a.r.abs_diff(b.r)
                    .max(a.g.abs_diff(b.g))
                    .max(a.b.abs_diff(b.b));
            assert!(d >= 96, "{mode:?}: the link stands {d} off the face");
        }
    }

    #[test]
    fn dark_faces_are_the_light_faces_remapped() {
        let st = style(&dark_settings());
        let cs = &st.colors;
        let face = dark::remap_color(DIALOG_FACE);
        assert_eq!(cs.button_face, face, "the dark face is EFEEEF remapped");
        assert_eq!(cs.button_light, face);
        assert_eq!(cs.menu, face);
        assert_eq!(
            cs.window,
            dark::remap_color(Color::rgb(0xFF, 0xFF, 0xFF)),
            "the dark window is white remapped"
        );
        assert_eq!(cs.window_text, DARK_TEXT);
        assert_eq!(
            cs.hilight,
            Color::rgb(0x2E, 0x8C, 0xFF),
            "the selection stays 2E8CFF"
        );
    }

    #[test]
    fn dark_glass_has_its_face_window_and_button() {
        let st = style(&dark_settings());
        assert_eq!(
            st.colors.button_face,
            Color::rgb(0x29, 0x29, 0x29),
            "the dark face is 292929"
        );
        assert_eq!(
            st.colors.window,
            Color::rgb(0x35, 0x35, 0x35),
            "the dark window is 353535"
        );
        let button = st.part(Part::PushButton).unwrap();
        let f = button.frame(State::Normal);
        let centre = button.atlas.get(f.x + f.width / 2, f.y + f.height / 2);
        assert_eq!(
            centre, 0xFF1D_1C1D,
            "a normal push button's centre is 1C1C1C, its lower gloss"
        );
    }

    #[test]
    fn each_group_paints_exactly_the_parts_it_names() {
        let start = |d| startpanel::parts(&Settings::default(), d);
        let groups = GROUPS
            .into_iter()
            .chain([(&startpanel::PARTS[..], start as fn(i32) -> _)]);
        for (names, paint) in groups {
            let painted: Vec<Part> = paint(1).into_iter().map(|(p, _)| p).collect();
            assert_eq!(painted.len(), names.len(), "{names:?}");
            assert!(names.iter().all(|p| painted.contains(p)), "{names:?}");
        }
    }

    #[test]
    fn dark_glass_remaps_every_control_part_but_the_fill_and_no_chrome() {
        let (light, dimmed) = (style(&Settings::default()), style(&dark_settings()));
        let controls = controls::parts(1)
            .into_iter()
            .map(|(p, _)| p)
            .chain(fields::parts(1).into_iter().map(|(p, _)| p))
            .chain(sheet::parts(1).into_iter().map(|(p, _)| p))
            .chain(scrollbar::parts(1).into_iter().map(|(p, _)| p))
            .chain(bars::parts().into_iter().map(|(p, _)| p))
            .chain(header::parts(1).into_iter().map(|(p, _)| p))
            .chain(statusbar::parts(1).into_iter().map(|(p, _)| p))
            .chain(popups::parts(1).into_iter().map(|(p, _)| p))
            .filter(|&p| p != Part::ProgressChunk);
        for part in controls {
            let want: Vec<u32> = light
                .part(part)
                .unwrap()
                .atlas
                .pixels
                .iter()
                .map(|&p| dark::remap_px(p))
                .collect();
            assert_eq!(
                dimmed.part(part).unwrap().atlas.pixels,
                want,
                "{part:?} is its light art remapped"
            );
        }
        for part in [
            Part::Caption,
            Part::FrameLeft,
            Part::Taskbar,
            Part::StartButton,
            Part::CaptionGroup,
            Part::ProgressChunk,
        ] {
            assert_eq!(
                dimmed.part(part).unwrap().atlas.pixels,
                light.part(part).unwrap().atlas.pixels,
                "{part:?} keeps its light art"
            );
        }
    }

    #[test]
    fn a_dark_caption_title_is_white_over_a_tight_dark_edge() {
        let st = style(&dark_settings());
        assert_eq!(st.caption_text.color, Color::rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(st.caption_text.inactive, Color::rgb(0xDE, 0xDF, 0xDF));
        let glow = st.caption_title.glow.expect("and an edge");
        assert_eq!(
            glow.color,
            Color::rgb(0, 0, 0),
            "the dark title's edge is black"
        );
        assert_eq!((glow.reach)(1), 3, "and reaches 3 px past the text");
    }

    #[test]
    fn labels_on_dark_glass_get_a_black_halo_and_on_light_glass_a_white_glow() {
        let white = Color::rgb(0xFF, 0xFF, 0xFF);
        for material in [Material::Frosted, Material::Molten] {
            assert_eq!(
                label_glow(material, Mode::Light).color,
                white,
                "{material:?}"
            );
        }
        for (material, mode) in [
            (Material::Smoked, Mode::Light),
            (Material::Frosted, Mode::Dark),
            (Material::Molten, Mode::Dark),
        ] {
            let halo = label_glow(material, mode);
            let what = format!("{material:?} {mode:?}");
            assert_eq!(halo.color, Color::rgb(0, 0, 0), "{what}: a black halo");
            assert_eq!((halo.reach)(1), 4, "{what}: reaching 4 px");
        }
    }

    /// The contrast between a Smoked window's title and the edge under it
    /// over a white backdrop, focused or not, as a WCAG ratio of the
    /// title box's brightest tenth of pixels (the ink) to its darkest
    /// tenth (the edge), with that brightest tenth's luma.
    fn smoked_title_over_white(active: bool) -> (f64, f64) {
        use crate::frame::{Chrome, FrameGeometry, FrameState, Glow};
        use crate::raster::Canvas;
        use crate::text::TextRenderer;
        let s = Settings {
            material: Material::Smoked,
            ..Settings::default()
        };
        let theme = crate::interfaces::theme_with(
            crate::interface::Interface::Glass,
            crate::blue::Variant::default(),
            &s,
        );
        let st = theme.look.style().unwrap();
        let text = TextRenderer::load(&st.caption_text.font());
        let title = "before.txt - Notepad";
        let g = FrameGeometry::layout(&theme.metrics, 300, 10, false, Chrome::WINDOW);
        let fs = FrameState {
            title,
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        crate::style::frame::draw_frame(&mut f, st, &theme, &text, &g, &fs);
        let mut c = Canvas::new(g.total.0 as u32, g.client.y as u32);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        c.blit_canvas(0, 0, &f);
        let x0 = g.caption.x + CAPTION_TITLE.text_x;
        let x1 = x0 + text.measure(title, false, st.caption_text.size_px);
        let luma = |p: u32| {
            let ch = |s: u32| f64::from((p >> s) & 0xFF);
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        let mut l: Vec<f64> = (8..26)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| luma(c.get(x, y)))
            .collect();
        l.sort_by(f64::total_cmp);
        let tenth = l.len() / 10;
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (dark, bright) = (mean(&l[..tenth]), mean(&l[l.len() - tenth..]));
        let lin = |v: f64| {
            let c = v / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        ((lin(bright) + 0.05) / (lin(dark) + 0.05), bright)
    }

    #[test]
    fn an_unfocused_smoked_title_reads_over_a_white_backdrop() {
        let (focused, focused_ink) = smoked_title_over_white(true);
        let (unfocused, unfocused_ink) = smoked_title_over_white(false);
        assert!(
            focused > 10.0,
            "the focused title stands 10:1 over its edge, got {focused:.2}"
        );
        assert!(
            unfocused > 8.5,
            "the unfocused title stands 8.5:1 over its edge, got {unfocused:.2}"
        );
        assert!(
            unfocused_ink + 20.0 < focused_ink,
            "the unfocused ink is greyer: {unfocused_ink:.0} against {focused_ink:.0}"
        );
    }

    #[test]
    fn the_caption_ink_follows_how_bright_the_glass_is() {
        let black = ([Color::rgb(0, 0, 0); 2], CAPTION_TITLE);
        let white = (
            [Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(0xDE, 0xDF, 0xDF)],
            CAPTION_TITLE_DARK,
        );
        for (material, mode, want, ink) in [
            (Material::Frosted, Mode::Light, &black, "black"),
            (Material::Frosted, Mode::Dark, &white, "white"),
            (Material::Smoked, Mode::Light, &white, "white"),
            (Material::Smoked, Mode::Dark, &white, "white"),
            (Material::Molten, Mode::Light, &black, "black"),
            (Material::Molten, Mode::Dark, &white, "white"),
        ] {
            let (got, title) = caption_ink(material, mode);
            assert_eq!(got, want.0, "{material:?} {mode:?}: a {ink} title");
            assert_eq!(
                title.glow.map(|g| g.color),
                want.1.glow.map(|g| g.color),
                "{material:?} {mode:?}: the {ink} title's glow"
            );
            let st = style(&Settings {
                material,
                mode,
                ..Settings::default()
            });
            assert_eq!(
                [st.caption_text.color, st.caption_text.inactive],
                want.0,
                "{material:?} {mode:?}: the style inks its title {ink}"
            );
        }
    }

    #[test]
    fn the_mode_reads_and_writes_light_and_dark() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.as_str()), Some(m), "{m:?} round-trips");
        }
        assert_eq!(Mode::default(), Mode::Light, "light is the default");
        assert_eq!(
            Settings::default().mode,
            Mode::Light,
            "the default glass is light"
        );
        assert_eq!(Mode::parse("Dark"), None, "the spelling is lower case");
    }

    #[test]
    fn the_material_and_frame_read_and_write_their_names() {
        for m in Material::ALL {
            assert_eq!(Material::parse(m.as_str()), Some(m), "{m:?} round-trips");
        }
        for f in Frame::ALL {
            assert_eq!(Frame::parse(f.as_str()), Some(f), "{f:?} round-trips");
        }
        let d = Settings::default();
        assert_eq!(d.material, Material::Frosted, "Frosted is the default");
        assert_eq!(d.frame, Frame::Rounded, "Rounded is the default");
        assert_eq!(
            Material::parse("Smoked"),
            None,
            "the spelling is lower case"
        );
    }

    #[test]
    fn the_style_cache_keeps_the_eight_most_recently_used() {
        let mut kept = Kept::new(STYLES_KEPT);
        assert_eq!(STYLES_KEPT, 8, "styles kept");
        for i in 0..8 {
            kept.get_or_insert_with(i, || i * 10);
        }
        assert_eq!(
            kept.get_or_insert_with(0, || 99),
            0,
            "a kept value is reused"
        );
        kept.get_or_insert_with(8, || 80);
        assert_eq!(kept.entries.len(), 8, "the cache stays at its cap");
        assert_eq!(
            kept.get_or_insert_with(0, || 99),
            0,
            "the one just used stays"
        );
        assert_eq!(
            kept.get_or_insert_with(1, || 11),
            11,
            "the least recently used went"
        );
        assert_eq!(kept.entries.len(), 8, "the cache stays at its cap");
    }

    #[test]
    fn glass_dialogs_have_their_own_face_shadow_and_selection() {
        let cs = &style(&Settings::default()).colors;
        assert_eq!(
            cs.button_face,
            Color::rgb(0xEF, 0xEE, 0xEF),
            "the face is EFEEEF"
        );
        assert_eq!(
            cs.button_light,
            Color::rgb(0xEF, 0xEE, 0xEF),
            "the 3D light line is the face, EFEEEF"
        );
        assert_eq!(
            cs.menu,
            Color::rgb(0xEF, 0xEE, 0xEF),
            "menus are the face, EFEEEF"
        );
        assert_eq!(
            cs.button_shadow,
            Color::rgb(0x9D, 0x9E, 0x9E),
            "the shadow is 9D9E9E"
        );
        assert_eq!(
            cs.button_hilight,
            Color::rgb(0xFF, 0xFF, 0xFF),
            "the highlight is FFFFFF"
        );
        assert_eq!(
            cs.hilight,
            Color::rgb(0x2E, 0x8C, 0xFF),
            "the selection is 2E8CFF"
        );
        assert_eq!(
            cs.window,
            Color::rgb(0xFF, 0xFF, 0xFF),
            "the window is FFFFFF"
        );
        assert_eq!(
            cs.background,
            ColorScheme::default().background,
            "the rest stay Standard's"
        );
    }

    #[test]
    fn a_greyed_glass_button_inks_its_label_flat_838383_and_a_static_label_etches_a0a0a0() {
        use crate::classic::widgets::draw_disabled_label;
        use crate::interfaces::theme_for;
        use crate::look::widgets::draw_button;
        use crate::pointer::Interaction;
        use crate::text::{Align, TextRenderer};
        use crate::theme::FontConfig;
        let text = TextRenderer::load(&FontConfig::default());
        let theme = theme_for(crate::interface::Interface::Glass);
        let p = theme.paint();
        let count = |c: &crate::raster::Canvas, rgb: u32| {
            (0..c.height as i32)
                .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == 0xFF00_0000 | rgb)
                .count()
        };
        let mut c = crate::raster::Canvas::new(80, 30);
        c.fill_rect(c.bounds(), DIALOG_FACE);
        let (default, pushed, enabled, focus) = (false, Interaction::Idle, false, false);
        draw_button(
            &mut c,
            &text,
            &p,
            Rect::new(2, 2, 75, 23),
            "Apply",
            default,
            pushed,
            enabled,
            focus,
            11,
        );
        assert!(count(&c, 0x7F8080) > 0, "the greyed label inks 838383");
        // Inside the button's edges, nothing but the face and the label.
        let inner = (6..20)
            .flat_map(|y| (6..73).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let v = c.get(x, y) & 0xFF_FFFF;
                v == 0xFFFFFF || v == 0x9D9E9E
            })
            .count();
        assert_eq!(
            inner, 0,
            "inside the edges no FFFFFF or 9D9E9E pixel: 0, the label is not etched"
        );

        let mut c = crate::raster::Canvas::new(80, 20);
        c.fill_rect(c.bounds(), DIALOG_FACE);
        draw_disabled_label(
            &mut c,
            &text,
            &p,
            Rect::new(2, 2, 70, 14),
            "Position:",
            Align::Left,
            11,
        );
        assert!(count(&c, 0x9D9E9E) > 0, "a greyed static label inks 9D9E9E");
        assert!(count(&c, 0xFFFFFF) > 0, "over a white copy");
    }

    #[test]
    fn glass_has_a_check_box_and_a_radio_part() {
        let s = style(&Settings::default());
        assert!(
            s.part(Part::CheckBox).is_some(),
            "Glass has a check box part"
        );
        assert!(s.part(Part::Radio).is_some(), "Glass has a radio part");
        assert_eq!(
            s.text_color(Part::PushButton, State::Disabled),
            Some(GREYED_BUTTON_TEXT),
            "a greyed push button inks 838383"
        );
    }

    #[test]
    fn the_default_settings_are_sky_at_42_translucent() {
        let s = Settings::default();
        assert_eq!(s.color, Color::rgb(112, 175, 252), "default glass colour");
        assert_eq!(s.intensity, 42, "default intensity");
        assert!(s.transparency, "default transparency");
    }

    #[test]
    fn glass_has_every_frame_caption_button_and_taskbar_part() {
        let s = style(&Settings::default());
        for p in [
            Part::Caption,
            Part::MaxCaption,
            Part::FrameLeft,
            Part::FrameRight,
            Part::FrameBottom,
            Part::WindowSheet,
            Part::ClientGlass,
            Part::MaxClientGlass,
            Part::CaptionGroup,
            Part::MaxCaptionGroup,
            Part::CloseButton,
            Part::Taskbar,
            Part::StartButton,
            Part::TaskButton,
            Part::TaskStack,
            Part::LabelRow,
            Part::TaskLabel,
            Part::TaskLabelEdge,
            Part::SlotEdge,
            Part::SlotLight,
            Part::SlotBand,
            Part::PagerTile,
            Part::TrayArea,
            Part::ShowDesktop,
            Part::PreviewFrame,
            Part::PreviewThumb,
            Part::PreviewClose,
            Part::SwitcherFrame,
            Part::SwitcherIconFrame,
            Part::SwitcherSelect,
            Part::SwitcherHot,
            Part::SwitcherShadow,
            Part::SnapFrame,
        ] {
            assert!(s.part(p).is_some(), "Glass has {p:?}");
        }
    }

    #[test]
    fn the_style_is_shared_per_settings() {
        let a = Settings::default();
        let b = Settings { intensity: 80, ..a };
        // A build is deterministic in its settings, so its art is the same
        // on every ask even if the cache dropped it and built it again in
        // between (an LRU capped at 8, shared with every other test asking
        // for a style).
        assert_eq!(
            style(&a).part(Part::Caption).unwrap().atlas.pixels,
            style(&a).part(Part::Caption).unwrap().atlas.pixels,
            "same settings, same style"
        );
        assert_ne!(
            style(&a).part(Part::Caption).unwrap().atlas.pixels,
            style(&b).part(Part::Caption).unwrap().atlas.pixels,
            "new settings, new style"
        );
    }

    #[test]
    fn the_small_bar_draws_its_own_art_and_the_frames_keep_theirs() {
        let s = Settings::default();
        let (full, small) = (style(&s), small_bar_style(&s));
        // Same reasoning as above: the cache may have dropped this entry
        // for another test's style by the time this asks again, so this
        // pins the settings, not the pointer.
        assert_eq!(
            small.part(Part::Caption).unwrap().atlas.pixels,
            small_bar_style(&s)
                .part(Part::Caption)
                .unwrap()
                .atlas
                .pixels,
            "the small bar's style is built once"
        );
        for part in [Part::Caption, Part::FrameLeft, Part::FrameBottom] {
            assert_eq!(
                full.part(part).unwrap().atlas.pixels,
                small.part(part).unwrap().atlas.pixels,
                "{part:?} keeps its art"
            );
        }
        let frame = |st: &Style, part| {
            let f = st.part(part).unwrap().frame(crate::style::State::Normal);
            (f.width, f.height)
        };
        for (part, big, little) in [
            (Part::Taskbar, (106, 40), (106, 30)),
            (Part::StartButton, (54, 40), (54, 30)),
            (Part::TaskButton, (60, 40), (44, 30)),
            (Part::TaskStack, (4, 40), (4, 30)),
            (Part::TrayArea, (35, 40), (35, 30)),
            (Part::ShowDesktop, (15, 40), (15, 30)),
            (Part::SlotEdge, (2, 40), (2, 30)),
            (Part::SlotLight, (60, 7), (44, 7)),
            (Part::SlotBand, (1, 40), (1, 30)),
            (Part::PagerTile, (60, 40), (44, 30)),
        ] {
            assert_eq!(frame(&full, part), big, "{part:?} on the 40 px bar");
            assert_eq!(frame(&small, part), little, "{part:?} on the 30 px bar");
        }
        assert!(small.taskbar.orb, "Start is an orb on the small bar too");
        assert!(small.task_glow.is_some(), "its buttons glow in colour");
        assert_eq!(small.taskbar.icon_button, Some(SMALL_ICON_BUTTON));
    }

    #[test]
    fn a_1920_bar_puts_the_orb_first_the_buttons_from_59_and_the_clock_on_two_lines() {
        use crate::taskbar::{TaskbarCounts, TaskbarLayout};
        let s = style(&Settings::default());
        let counts = TaskbarCounts {
            quick: 0,
            pager: 0,
            tasks: 3,
            tray: 3,
            chevron: false,
            show_desktop: false,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&s.metrics, &s.taskbar, 1920, true, counts, 51);
        assert_eq!(l.height, 40, "a 40 px bar");
        assert_eq!(l.start, Rect::new(0, 0, 54, 40), "the orb's rect");
        assert_eq!(l.tasks[0].x, 59, "the first button");
        assert!(
            l.tasks.iter().all(|t| (t.y, t.height) == (0, 40)),
            "buttons run the bar's full height"
        );
        assert_eq!(l.clock, Rect::new(1920 - 2 - 67, 0, 67, 40), "the clock");
        assert_eq!(l.notification.right(), 1920, "the tray runs to the edge");
        assert_eq!(s.taskbar.clock_baselines(40), Some([17, 34]), "two lines");
        assert_eq!(s.taskbar.clock_baselines(30), None, "one on a 30 px bar");
        assert!(s.taskbar.orb, "Start is an orb");
    }

    #[test]
    fn swatch_looks_up_by_name_and_rejects_unknown_names() {
        assert_eq!(swatch("sky"), Some(Color::rgb(112, 175, 252)));
        assert_eq!(swatch("nonexistent"), None);
        assert_eq!(swatch("ruby"), Some(Color::rgb(201, 24, 15)));
        let names: Vec<_> = swatch_names().collect();
        assert_eq!(
            names,
            [
                "sky",
                "twilight",
                "sea",
                "leaf",
                "lime",
                "sun",
                "pumpkin",
                "ruby",
                "fuchsia",
                "blush",
                "violet",
                "lavender",
                "taupe",
                "chocolate",
                "slate",
                "frost"
            ],
            "sixteen colours, Sky first"
        );
        assert_eq!(swatch_name(Color::rgb(112, 175, 252)), Some("sky"));
        assert_eq!(swatch_name(Color::rgb(83, 83, 83)), Some("slate"));
        assert_eq!(swatch_name(Color::rgb(1, 2, 3)), None, "not a swatch");
    }
}
