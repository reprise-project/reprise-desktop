//! Theme data: the Reprise Standard color scheme, pixel metrics and fonts.

use crate::paint::Paint;
use crate::raster::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let v = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        let parts: Vec<u8> = s
            .split_whitespace()
            .map(|p| p.parse().ok())
            .collect::<Option<Vec<_>>>()?;
        if parts.len() != 3 {
            return None;
        }
        Some(Color::rgb(parts[0], parts[1], parts[2]))
    }

    /// Each channel's complement: what an XOR-drawn line leaves on this
    /// colour, as a focus ring's dots over the highlight.
    pub const fn inverted(self) -> Color {
        Color::rgb(255 - self.r, 255 - self.g, 255 - self.b)
    }

    pub const fn toward(self, to: Color, n: u8) -> Color {
        const fn ch(a: u8, b: u8, n: u8) -> u8 {
            ((a as u32 * (255 - n as u32) + b as u32 * n as u32 + 127) / 255) as u8
        }
        Color::rgb(
            ch(self.r, to.r, n),
            ch(self.g, to.g, n),
            ch(self.b, to.b, n),
        )
    }

    pub fn argb(self) -> u32 {
        0xFF00_0000 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub fn as_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            1.0,
        ]
    }

    pub fn luminance(self) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn contrast(self, other: Color) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// A struct of same-typed fields addressable by config name. A field may
/// carry a unit, `px` or `ms`; with units the macro also generates `scaled`,
/// which multiplies `px` fields and leaves `ms` durations as they are.
/// Fields after `extra` are of their own type, not addressable by name, and
/// scale through the function named after `=>`.
macro_rules! named_fields {
    (@scale px, $v:expr, $n:ident) => { $v * $n };
    (@scale ms, $v:expr, $n:ident) => { $v };
    (@struct $name:ident, $vty:ty, { $($field:ident = $default:expr),* }
        { $($(#[$xmeta:meta])* $xfield:ident: $xty:ty = $xdefault:expr),* }) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            $(pub $field: $vty,)*
            $($(#[$xmeta])* pub $xfield: $xty,)*
        }
        impl Default for $name {
            fn default() -> Self {
                Self::DEFAULT
            }
        }
        impl $name {
            pub const DEFAULT: Self = $name {
                $($field: $default,)*
                $($xfield: $xdefault,)*
            };

            pub fn set(&mut self, name: &str, v: $vty) -> bool {
                match name {
                    $(stringify!($field) => { self.$field = v; true })*
                    _ => false,
                }
            }

            pub fn pairs(&self) -> Vec<(&'static str, $vty)> {
                vec![$((stringify!($field), self.$field),)*]
            }
        }
    };
    ($name:ident, $vty:ty, { $($field:ident = $default:expr => $unit:ident),* $(,)? }
        $(extra { $($(#[$xmeta:meta])* $xfield:ident: $xty:ty = $xdefault:expr => $xscale:path),* $(,)? })?) => {
        named_fields!(@struct $name, $vty, { $($field = $default),* }
            { $($($(#[$xmeta])* $xfield: $xty = $xdefault),*)? });
        impl $name {
            pub fn scaled(&self, n: i32) -> Self {
                let n = n.max(1);
                $name {
                    $($field: named_fields!(@scale $unit, self.$field, n),)*
                    $($($xfield: $xscale(&self.$xfield, n),)*)?
                }
            }
        }
    };
    ($name:ident, $vty:ty, { $($field:ident = $default:expr),* $(,)? }) => {
        named_fields!(@struct $name, $vty, { $($field = $default),* } {});
    };
}

named_fields!(ColorScheme, Color, {
    active_title = Color::rgb(10, 36, 106),
    gradient_active_title = Color::rgb(166, 202, 240),
    inactive_title = Color::rgb(128, 128, 128),
    gradient_inactive_title = Color::rgb(192, 192, 192),
    title_text = Color::rgb(255, 255, 255),
    inactive_title_text = Color::rgb(212, 208, 200),
    button_face = Color::rgb(212, 208, 200),
    button_hilight = Color::rgb(255, 255, 255),
    button_light = Color::rgb(212, 208, 200),
    button_shadow = Color::rgb(128, 128, 128),
    button_dk_shadow = Color::rgb(64, 64, 64),
    button_text = Color::rgb(0, 0, 0),
    background = Color::rgb(58, 110, 165),
    window = Color::rgb(255, 255, 255),
    window_frame = Color::rgb(0, 0, 0),
    window_text = Color::rgb(0, 0, 0),
    hilight = Color::rgb(10, 36, 106),
    hilight_text = Color::rgb(255, 255, 255),
    gray_text = Color::rgb(128, 128, 128),
    menu = Color::rgb(212, 208, 200),
    menu_text = Color::rgb(0, 0, 0),
    menu_band = Color::rgb(0, 0, 128),
    menu_band_top = Color::rgb(0, 0, 0),
    gradient_menu_band = Color::rgb(0, 0, 255),
    menu_band_text = Color::rgb(255, 255, 255),
    desktop_text = Color::rgb(255, 255, 255),
    info_bk = Color::rgb(255, 255, 225),
    info_text = Color::rgb(0, 0, 0),
    hot_light = Color::rgb(0, 0, 128),
    menu_hilight = Color::rgb(10, 36, 106),
    menu_bar = Color::rgb(212, 208, 200),
});

/// The environment variable a compositor hands its clients the effective
/// colour scheme in: `field=#rrggbb` pairs separated by `;`, in
/// [`ColorScheme::pairs`] order.
pub const COLORS_ENV: &str = "REPRISE_COLORS";

impl ColorScheme {
    pub fn to_env(&self) -> String {
        self.pairs()
            .iter()
            .map(|(n, c)| format!("{n}=#{:02x}{:02x}{:02x}", c.r, c.g, c.b))
            .collect::<Vec<_>>()
            .join(";")
    }

    pub fn is_dark(&self) -> bool {
        self.window.luminance() < 0.5
    }

    /// A scheme from a [`COLORS_ENV`] value laid over Reprise Standard.
    /// Unparsable pairs and unknown names are skipped; `None` only for an
    /// empty value.
    pub fn from_env(s: &str) -> Option<ColorScheme> {
        Self::from_env_over(ColorScheme::default(), s)
    }

    pub fn from_env_over(base: ColorScheme, s: &str) -> Option<ColorScheme> {
        if s.trim().is_empty() {
            return None;
        }
        let mut cs = base;
        for pair in s.split(';') {
            if let Some((name, value)) = pair.split_once('=')
                && let Some(c) = Color::parse(value)
            {
                cs.set(name.trim(), c);
            }
        }
        Some(cs)
    }
}

named_fields!(Metrics, i32, {
    frame_width = 4 => px,
    dialog_frame_width = 3 => px,
    caption_height = 18 => px,
    caption_button_width = 16 => px,
    caption_button_height = 14 => px,
    caption_button_right_margin = 2 => px,
    caption_button_gap = 0 => px,
    taskbar_height = 28 => px,
    task_button_max_width = 160 => px,
    start_button_width = 54 => px,
    menu_item_height = 20 => px,
    menu_width = 0 => px,
    menu_shadow = 4 => px,
    startmenu_item_height = 32 => px,
    startmenu_width = 0 => px,
    startmenu_band_width = 21 => px,
    resize_border = 4 => px,
    cascade_origin = 32 => px,
    cascade_step = 24 => px,
    double_click_ms = 500 => ms,
    menu_hover_ms = 400 => ms,
    menu_fade_ms = 150 => ms,
} extra {
    caption_group: Option<CaptionGroup> = None => CaptionGroup::scaled_opt,
});

/// Caption buttons drawn as one joined strip: minimize, maximize and
/// close side by side, each neighbouring pair sharing the divider between
/// them. The strip is placed from the frame's outer top-right corner, so it
/// keeps its place whatever the border and caption sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionGroup {
    pub widths: [i32; 3],
    pub divider: i32,
    pub restored: GroupPlace,
    pub maximized: GroupPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupPlace {
    pub right: i32,
    pub top: i32,
    pub height: i32,
}

impl GroupPlace {
    fn scaled(self, n: i32) -> GroupPlace {
        GroupPlace {
            right: self.right * n,
            top: self.top * n,
            height: self.height * n,
        }
    }
}

impl CaptionGroup {
    pub fn scaled(&self, n: i32) -> CaptionGroup {
        CaptionGroup {
            widths: self.widths.map(|w| w * n),
            divider: self.divider * n,
            restored: self.restored.scaled(n),
            maximized: self.maximized.scaled(n),
        }
    }

    fn scaled_opt(g: &Option<CaptionGroup>, n: i32) -> Option<CaptionGroup> {
        g.map(|g| g.scaled(n))
    }

    /// The strip and its three cells, in `CaptionButton::ALL` order, in a
    /// frame `frame_w` px wide. The cells are where the pointer hits each
    /// button: maximized, they reach up to the frame's top and close
    /// reaches its right edge, so a pointer thrown into the screen's
    /// corner lands on close. The strip is drawn where it is.
    pub fn layout(&self, frame_w: i32, maximized: bool) -> (Rect, [Rect; 3]) {
        let p = if maximized {
            self.maximized
        } else {
            self.restored
        };
        let n = self.divider;
        let [min, max, close] = self.widths;
        let width = min + max + close - 2 * n;
        let x = frame_w - p.right - width;
        let cell = |dx: i32, w: i32| Rect::new(x + dx, p.top, w, p.height);
        let mut cells = [
            cell(0, min),
            cell(min - n, max),
            cell(min + max - 2 * n, close),
        ];
        if maximized {
            for c in &mut cells {
                *c = Rect::new(c.x, 0, c.width, c.bottom());
            }
            cells[2].width = frame_w - cells[2].x;
        }
        (Rect::new(x, p.top, width, p.height), cells)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    pub family: Vec<String>,
    pub size_px: u32,
    pub antialias: bool,
    pub italic: bool,
    /// Grid-fit stems across as well as down, so an antialiased bold
    /// title's stems land on whole pixels instead of blurring over two.
    /// Monochrome text is always fitted both ways.
    pub full_hinting: bool,
    /// The second dialog font, a family name or an absolute path to a
    /// font file: the face the dialogs asking for
    /// [`crate::text::DialogFont::ShellDlg`] are set in. `None`, or a
    /// font that is not found, sets them in `family` like the rest.
    pub shell_dlg: Option<String>,
}

impl Default for FontConfig {
    fn default() -> Self {
        FontConfig {
            family: vec!["Tahoma".into(), "DejaVu Sans".into()],
            size_px: 11,
            antialias: false,
            italic: false,
            full_hinting: false,
            shell_dlg: None,
        }
    }
}

impl FontConfig {
    /// Smallest and largest `size_px` a config may set. Glyph bitmaps grow
    /// with the square of the size, so a typo past the top would allocate
    /// gigabytes per glyph.
    pub const SIZE_PX_MIN: u32 = 6;
    pub const SIZE_PX_MAX: u32 = 72;

    pub fn scaled(&self, n: i32) -> FontConfig {
        FontConfig {
            family: self.family.clone(),
            size_px: self.size_px * n.max(1) as u32,
            antialias: self.antialias,
            italic: self.italic,
            full_hinting: self.full_hinting,
            shell_dlg: self.shell_dlg.clone(),
        }
    }
}

/// How much taller than at the default 11 px font a row that holds one
/// line set at `px` must be: the line's growth, a fifth more than the
/// font's, for the space above and below the glyphs. None for a smaller
/// font.
pub fn line_growth(px: u32) -> i32 {
    (px as i32 - 11).max(0) * 6 / 5
}

pub fn text_px(px: u32, pct: u32) -> u32 {
    ((px * pct + 50) / 100).min(FontConfig::SIZE_PX_MAX.max(px))
}

/// How a layout drawn for 100% text grows at the text size: the UI font's px at the text size
/// over its px at 100%. Rects laid out in 100% units map through it; bevels, glyph boxes and
/// pictures keep their size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextUnits {
    px: u32,
    base: u32,
}

impl Default for TextUnits {
    fn default() -> TextUnits {
        TextUnits::ONE
    }
}

impl TextUnits {
    pub const ONE: TextUnits = TextUnits { px: 1, base: 1 };

    /// Text set at `px` where the layout was drawn for `base`.
    pub fn new(px: u32, base: u32) -> TextUnits {
        if px == base || base == 0 {
            TextUnits::ONE
        } else {
            TextUnits { px, base }
        }
    }

    pub fn is_one(self) -> bool {
        self == TextUnits::ONE
    }

    /// `v` 100% px at the text size, rounded half up.
    pub fn len(self, v: i32) -> i32 {
        let (n, d) = (self.px as i64, self.base as i64);
        (2 * v as i64 * n + d).div_euclid(2 * d) as i32
    }

    /// `px` (a font size or a device px count at 100%) at the text size.
    pub fn px(self, px: u32) -> u32 {
        self.len(px as i32).max(0) as u32
    }

    /// The inverse of [`TextUnits::px`] for a font size: what `px` was at 100%.
    pub fn base_px(self, px: u32) -> u32 {
        ((2 * px as u64 * self.base as u64 + self.px as u64) / (2 * self.px as u64)) as u32
    }

    /// The shortest 100% length that grows to at least `v`: a measured text width as a 100%
    /// layout reserves it.
    pub fn design(self, v: i32) -> i32 {
        let (n, d) = (self.px as i64, self.base as i64);
        (v as i64 * d + n - 1).div_euclid(n) as i32
    }

    /// The longest 100% length that grows to at most `v`: what a layout fills of `v` grown px.
    pub fn within(self, v: i32) -> i32 {
        let d = self.design(v);
        if self.len(d) > v { d - 1 } else { d }
    }

    /// `r` with both its edges mapped, so rects that touch at 100% still touch.
    pub fn rect(self, r: Rect) -> Rect {
        let (x, y) = (self.len(r.x), self.len(r.y));
        Rect::new(x, y, self.len(r.right()) - x, self.len(r.bottom()) - y)
    }

    /// `r` moved to where its top-left maps, its size kept: a picture or a fixed control.
    pub fn at(self, r: Rect) -> Rect {
        Rect::new(self.len(r.x), self.len(r.y), r.width, r.height)
    }

    pub fn size(self, (w, h): (i32, i32)) -> (i32, i32) {
        (self.len(w), self.len(h))
    }

    /// The largest text size up to this one, never under 100%, at which `v` 100% px grow to at
    /// most `room`.
    pub fn fitting(self, v: i32, room: i32) -> TextUnits {
        if self.len(v) <= room || self.px <= self.base {
            return self;
        }
        (self.base..self.px)
            .rev()
            .map(|px| TextUnits::new(px, self.base))
            .find(|u| u.len(v) <= room)
            .unwrap_or(TextUnits::ONE)
    }
}

/// Colors, metrics and fonts plus the output scale the metrics are expressed
/// in. `Theme::default()` and the config are always at scale 1; draw code
/// receives `theme.scaled(output_scale)` and multiplies its own constants by
/// `theme.scale`.
#[derive(Clone, Debug)]
pub struct Theme {
    pub colors: ColorScheme,
    pub metrics: Metrics,
    pub fonts: FontConfig,
    pub scale: i32,
    pub look: crate::paint::Look,
    /// How forms laid out for 100% text grow at the text size; [`TextUnits::ONE`] unless
    /// [`Theme::with_text_size`] set it.
    pub text: TextUnits,
    /// Whether the window being drawn is the active one, handed on by
    /// [`Theme::paint`]. True unless a host that draws several windows
    /// sets it for one draw and puts it back after. Draw state, not
    /// configuration: two themes that differ only here are equal.
    pub active: bool,
    /// Whether access keys draw their underline, handed on by [`Theme::paint`]: the
    /// window's [`crate::cues::Cues::keys`]. Draw state set and put back as `active` is.
    pub access_keys: bool,
}

impl PartialEq for Theme {
    fn eq(&self, o: &Theme) -> bool {
        self.colors == o.colors
            && self.metrics == o.metrics
            && self.fonts == o.fonts
            && self.scale == o.scale
            && self.look == o.look
            && self.text == o.text
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            colors: ColorScheme::default(),
            metrics: Metrics::default(),
            fonts: FontConfig::default(),
            scale: 1,
            look: crate::paint::Look::Classic,
            text: TextUnits::ONE,
            active: true,
            access_keys: true,
        }
    }
}

impl Metrics {
    /// A left press at (`x`, `y`), event `time` ms, for [`crate::pointer::DoubleClick`] pairing in this theme's double-click time.
    pub fn click(&self, x: i32, y: i32, time: u32) -> crate::pointer::Click {
        crate::pointer::Click {
            x,
            y,
            time,
            double_ms: self.double_click_ms,
        }
    }
}

impl Theme {
    /// This theme with its style's Shut down options menu as its menu; the theme itself when the
    /// style has none. Call it on a scaled theme: a rescaled style is built afresh.
    pub fn for_options_menu(&self) -> Theme {
        let Some((st, m)) = self
            .look
            .style()
            .and_then(|st| Some((st, st.options_menu?)))
        else {
            return self.clone();
        };
        Theme {
            look: crate::paint::Look::Styled(std::sync::Arc::new(st.clone().with_menu(m))),
            ..self.clone()
        }
    }

    /// Metrics and font size multiplied by `n`, `scale` set to `n`, and
    /// the look's style at `n` ([`crate::paint::Look::at_scale`]). Always
    /// derived from the scale-1 theme in `Config`, never from an already
    /// scaled one.
    pub fn scaled(&self, n: i32) -> Theme {
        let n = n.max(1);
        Theme {
            colors: self.colors.clone(),
            metrics: self.metrics.scaled(n),
            fonts: self.fonts.scaled(n),
            scale: n,
            look: self.look.at_scale(n),
            text: self.text,
            active: self.active,
            access_keys: self.access_keys,
        }
    }

    /// This scale-1 theme with its text at `pct` percent (the
    /// `[accessibility] text_size`): the UI font, a styled look's caption and
    /// Start text, and the heights that hold one line of it (caption, menu
    /// rows, Start menu rows, taskbar) grow, each by [`line_growth`].
    /// Caption buttons grow as much as the caption unless the look joins them
    /// into a [`CaptionGroup`]. Classic's Start button widens with its label;
    /// frames, icons and the output scale keep their size.
    pub fn with_text_size(&self, pct: u32) -> Theme {
        if pct == 100 {
            return self.clone();
        }
        let base = self.fonts.size_px;
        let mut t = self.with_form_text(pct);
        let grow = line_growth(t.fonts.size_px) - line_growth(base);
        let m = &mut t.metrics;
        for h in [
            &mut m.caption_height,
            &mut m.menu_item_height,
            &mut m.startmenu_item_height,
            &mut m.taskbar_height,
        ] {
            *h += grow;
        }
        if m.caption_group.is_none() {
            m.caption_button_width += grow;
            m.caption_button_height += grow;
        }
        match &t.look {
            crate::paint::Look::Classic => {
                let label = m.start_button_width - 21;
                m.start_button_width = 21 + (label * pct as i32 + 50) / 100;
            }
            crate::paint::Look::Styled(s) => {
                t.look = crate::paint::Look::Styled(s.with_text_size(pct));
            }
        }
        t
    }

    /// This theme with only the UI font and [`Theme::text`] at `pct` percent: what forms laid out
    /// for 100% text draw with. Metrics and the look stay.
    pub fn with_form_text(&self, pct: u32) -> Theme {
        let mut t = self.clone();
        let base = t.fonts.size_px;
        t.fonts.size_px = text_px(base, pct);
        t.text = TextUnits::new(t.fonts.size_px, base);
        t
    }

    pub fn for_draw(p: &Paint, px: u32) -> Theme {
        Theme {
            colors: p.cs.clone(),
            metrics: Metrics::default().scaled(p.s),
            fonts: FontConfig {
                size_px: px,
                ..FontConfig::default()
            },
            scale: p.s,
            look: p.look.clone(),
            text: p.text,
            active: p.active,
            access_keys: p.access_keys,
        }
    }

    /// The look's control sizes with their text rows grown by [`Theme::text`].
    pub fn controls(&self) -> crate::metrics::ControlMetrics {
        self.look.controls().grown(self.text)
    }

    pub fn icon_size(&self) -> i32 {
        crate::icon::ICON_SIZE * self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_units_grow_lengths_and_both_edges_of_a_rect() {
        let one = TextUnits::new(11, 11);
        assert!(one.is_one());
        let r = Rect::new(7, 13, 75, 23);
        assert_eq!(one.rect(r), r);
        let big = TextUnits::new(17, 11);
        assert_eq!(big.len(11), 17);
        assert_eq!(big.len(13), 20);
        assert_eq!(big.len(-5), -8);
        assert_eq!(big.rect(r), Rect::new(11, 20, 127 - 11, 56 - 20));
        assert_eq!(big.at(r), Rect::new(11, 20, 75, 23));
        let (a, b) = (Rect::new(0, 0, 75, 23), Rect::new(81, 0, 75, 23));
        assert!(big.rect(a).right() < big.rect(b).x, "neighbours stay apart");
        assert_eq!(big.base_px(17), 11);
        for v in [0, 1, 16, 17, 100, 233] {
            let d = big.design(v);
            assert!(big.len(d) >= v && big.len(d - 1) < v, "{v} -> {d}");
        }
        assert_eq!(big.base_px(34), 22, "a device px at scale 2");
        assert_eq!(TextUnits::new(22, 11).rect(r), Rect::new(14, 26, 150, 46));
        let t = Theme::default().with_text_size(150);
        assert_eq!(t.text, big);
        assert_eq!(t.scaled(2).paint().text, big);
        assert!(Theme::default().with_text_size(100).text.is_one());
    }

    #[test]
    fn fitting_steps_the_text_size_down_to_the_room_and_no_further_than_100_percent() {
        let two = TextUnits::new(22, 11);
        assert_eq!(two.fitting(411, 822), two, "room enough");
        assert_eq!(two.fitting(411, 800), TextUnits::new(21, 11));
        assert_eq!(two.fitting(411, 800).len(411), 785);
        assert!(two.fitting(411, 400).is_one());
        let small = TextUnits::new(10, 11);
        assert_eq!(small.fitting(411, 300), small, "never grows a smaller text");
    }

    #[test]
    fn parse_hex_and_triplet() {
        assert_eq!(Color::parse("#3A6EA5"), Some(Color::rgb(58, 110, 165)));
        assert_eq!(Color::parse("10 36 106"), Some(Color::rgb(10, 36, 106)));
        assert_eq!(Color::parse("nope"), None);
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("#+12345"), None);
    }

    #[test]
    fn argb_packing() {
        assert_eq!(Color::rgb(0x12, 0x34, 0x56).argb(), 0xFF123456);
        assert_eq!(Color::rgb(255, 0, 0).as_f32(), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn defaults_match_reprise_standard() {
        let t = Theme::default();
        assert_eq!(t.colors.active_title, Color::rgb(10, 36, 106));
        assert_eq!(t.colors.gradient_active_title, Color::rgb(166, 202, 240));
        assert_eq!(t.colors.button_face, Color::rgb(212, 208, 200));
        assert_eq!(t.colors.button_dk_shadow, Color::rgb(64, 64, 64));
        assert_eq!(t.colors.background, Color::rgb(58, 110, 165));
        assert_eq!(t.metrics.frame_width, 4);
        assert_eq!(t.metrics.caption_height, 18);
        assert_eq!(t.metrics.taskbar_height, 28);
        assert_eq!(t.fonts.size_px, 11);
        assert!(!t.fonts.antialias);
    }

    #[test]
    fn the_text_size_grows_the_font_and_the_heights_that_hold_a_line() {
        let t = Theme::default();
        assert_eq!(t.with_text_size(100), t);
        let big = t.with_text_size(150);
        assert_eq!(big.fonts.size_px, 17);
        let m = &big.metrics;
        assert_eq!(
            (
                m.caption_height,
                m.menu_item_height,
                m.startmenu_item_height,
                m.taskbar_height
            ),
            (25, 27, 39, 35)
        );
        assert_eq!(m.start_button_width, 71, "the label's 33 px are 50");
        assert_eq!(m.frame_width, 4, "frames keep their size");
        assert_eq!(
            (m.caption_button_width, m.caption_button_height),
            (23, 21),
            "caption buttons grow with the caption"
        );
        assert_eq!(big.scale, 1);
        let huge = t.with_text_size(200);
        assert_eq!((huge.fonts.size_px, huge.metrics.caption_height), (22, 31));
        assert_eq!(t.with_text_size(125).fonts.size_px, 14);
        assert_eq!(t.with_text_size(175).fonts.size_px, 19);
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let style = |t: &Theme| t.look.style().unwrap().caption_text.size_px;
        let big = glass.with_text_size(150);
        assert_eq!((style(&glass), style(&big)), (12, 18));
        assert_eq!(style(&big.scaled(2)), 18, "the scale-2 style keeps it");
        assert_eq!(
            big.metrics.start_button_width, glass.metrics.start_button_width,
            "the orb keeps its size"
        );
        let buttons = |m: &Metrics| {
            (
                m.caption_button_width,
                m.caption_button_height,
                m.caption_group,
            )
        };
        for pct in [150, 200] {
            let grown = glass.with_text_size(pct);
            assert_eq!(
                buttons(&grown.metrics),
                buttons(&glass.metrics),
                "the joined strip keeps its size"
            );
        }
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let side = |pct| blue.with_text_size(pct).metrics.caption_button_height;
        assert_eq!((side(100), side(150), side(200)), (21, 28, 34));
    }

    #[test]
    fn colors_round_trip_through_the_environment() {
        let dark = crate::schemes::scheme("Dracula").unwrap();
        let s = dark.to_env();
        assert!(s.starts_with("active_title=#bd93f9;"));
        assert_eq!(ColorScheme::from_env(&s), Some(dark));
        let cs = ColorScheme::from_env("bogus=#010203;window=nope;background=#010203").unwrap();
        assert_eq!(cs.background, Color::rgb(1, 2, 3));
        assert_eq!(cs.window, Color::rgb(255, 255, 255));
        assert_eq!(ColorScheme::from_env("  "), None);
    }

    #[test]
    fn the_menu_slots_round_trip_through_the_environment() {
        let cs = ColorScheme {
            menu_hilight: Color::rgb(0x31, 0x6A, 0xC5),
            menu_bar: Color::rgb(0xEC, 0xE9, 0xD8),
            ..ColorScheme::default()
        };
        let s = cs.to_env();
        assert!(s.ends_with(";menu_hilight=#316ac5;menu_bar=#ece9d8"), "{s}");
        assert_eq!(ColorScheme::from_env(&s), Some(cs));
    }

    #[test]
    fn set_by_name() {
        let mut c = ColorScheme::default();
        assert!(c.set("background", Color::rgb(1, 2, 3)));
        assert_eq!(c.background, Color::rgb(1, 2, 3));
        assert!(!c.set("bogus", Color::rgb(1, 2, 3)));
        let mut m = Metrics::default();
        assert!(m.set("caption_height", 20));
        assert_eq!(m.caption_height, 20);
        assert!(!m.set("bogus", 1));
    }

    #[test]
    fn scaled_multiplies_every_metric_and_font_size() {
        let t = Theme::default();
        assert_eq!(t.scale, 1);
        let s = t.scaled(2);
        assert_eq!(s.scale, 2);
        assert_eq!(s.colors, t.colors);
        let m = &s.metrics;
        assert_eq!(m.frame_width, 8);
        assert_eq!(m.caption_height, 36);
        assert_eq!(m.caption_button_width, 32);
        assert_eq!(m.caption_button_height, 28);
        assert_eq!(m.caption_button_right_margin, 4);
        assert_eq!(m.taskbar_height, 56);
        assert_eq!(m.task_button_max_width, 320);
        assert_eq!(m.start_button_width, 108);
        assert_eq!(m.menu_item_height, 40);
        assert_eq!(m.menu_width, 0);
        assert_eq!(m.menu_shadow, 8);
        assert_eq!(m.startmenu_item_height, 64);
        assert_eq!(m.startmenu_width, 0);
        assert_eq!(m.startmenu_band_width, 42);
        assert_eq!(m.resize_border, 8);
        assert_eq!(m.cascade_origin, 64);
        assert_eq!(m.cascade_step, 48);
        assert_eq!(m.double_click_ms, 500);
        assert_eq!(s.fonts.size_px, 22);
        assert_eq!(s.fonts.family, t.fonts.family);
        assert_eq!(s.icon_size(), 32);
        assert_eq!(t.icon_size(), 16);
    }

    #[test]
    fn desktop_text_and_double_click_defaults() {
        let t = Theme::default();
        assert_eq!(t.colors.desktop_text, Color::rgb(255, 255, 255));
        assert_eq!(t.metrics.double_click_ms, 500);
        assert_eq!(t.scaled(2).metrics.double_click_ms, 500);
        assert_eq!(t.metrics.menu_hover_ms, 400);
        assert_eq!(t.scaled(2).metrics.menu_hover_ms, 400);
        assert_eq!(t.scaled(2).metrics.menu_fade_ms, 150);
    }

    #[test]
    fn scaled_treats_non_positive_as_one() {
        let t = Theme::default();
        assert_eq!(t.scaled(0), t);
        assert_eq!(t.scaled(-3), t);
        assert_eq!(t.scaled(1), t);
    }
}
