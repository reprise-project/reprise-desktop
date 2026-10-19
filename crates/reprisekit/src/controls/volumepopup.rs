//! The speaker's volume popup: a raised flyout ([`crate::flyout`]) over the tray with Output and Input device combos on top and, under a separator, master's column, an etched line and a column per program playing sound.
//!
//! Columns are 96 px wide: the 32 px icon, the name ("Volume" on master), the level in percent, a vertical trackbar (one position per 5 %, loudest on top) and a 30x26 speaker button that mutes; a muted column greys its trackbar and level. Six programs fit side by side; past six a scrollbar under them scrolls them a column at a time while master stays. The device combos are 360 px wide. Lengths are at 100% text and grow with it; icons, glyphs, the thumb and the scrollbar keep their size.
//!
//! While a media player is active a strip under a second separator shows the title over the artist and Previous, Play or Pause and Next buttons (32x28, greyed when the player cannot act). Text is set in the second dialog font (`DialogFont::ShellDlg`).
//!
//! The host owns the devices and the writes: it hands the popup a [`Mix`] with [`VolumePopup::sync`] and acts on the [`Event`]s at once; there is no Apply.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    ScrollHit, ScrollParts, scrollbar_hit, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::combo::{self, Combo, ComboLit};
use crate::controls::form;
use crate::flyout;
use crate::look::Paint;
use crate::look::widgets::{
    MediaGlyph, draw_disabled_label, draw_focus_ring, draw_glyph_button, draw_hscrollbar,
    draw_label, draw_media_button, draw_vertical_trackbar,
};
use crate::metrics::ControlMetrics;
use crate::pointer::{Interaction, Lit, Pointer, WHEEL_ROWS};
use crate::raster::{Canvas, Rect};
use crate::shape::Glyph;
use crate::taskbar::Edge;
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::Caret;
use crate::theme::{TextUnits, Theme};
use crate::trackbar::{self, TrackBar};
use accesskit::{NodeId, Role};
use std::cell::Cell;

/// Trackbar positions: 0 to 20, one per 5 % of volume.
pub const STEPS: usize = 21;
pub const COL_W: i32 = 96;
/// Between two program columns.
const COL_GAP: i32 = 6;
/// Program columns shown side by side; past this a scrollbar runs under
/// them.
pub const MAX_SHOWN: usize = 6;
/// From the panel's outer edge in to its parts, all round.
const MARGIN: i32 = 12;
const COMBO_H: i32 = 21;
const COMBO_GAP: i32 = 6;
const LABEL_GAP: i32 = 8;
/// The Output and Input labels' column, at its narrowest.
const LABEL_W: i32 = 52;
/// The Output and Input combos' width.
const FIELD_W: i32 = 360;
const SEP_GAP: i32 = 10;
const SEP_H: i32 = 2;
/// Either side of the etched line between master and the programs.
const DIVIDER_GAP: i32 = 6;
/// A column, from its top: the icon, the name and level rows under it,
/// the trackbar, then the speaker button under the trackbar.
const ICON: i32 = 32;
const NAME_Y: i32 = ICON + 6;
const ROW_H: i32 = 13;
const LEVEL_Y: i32 = NAME_Y + ROW_H + 1;
const TRACK_Y: i32 = LEVEL_Y + ROW_H + 8;
const TRACK_H: i32 = 140;
/// The thumb, its width past the look's own.
const THUMB_WIDER: i32 = 4;
const THUMB_H: i32 = 15;
const MUTE_Y: i32 = TRACK_Y + TRACK_H + 8;
const MUTE_W: i32 = 30;
const MUTE_H: i32 = 26;
/// The speaker glyph's box.
const SPEAKER: i32 = 16;
const COL_H: i32 = MUTE_Y + MUTE_H;
/// A program's name and level leave 2 px clear on either side of the
/// centred text.
const NAME_PAD: i32 = 2;
const BAR_GAP: i32 = 8;
/// Rows an open device list shows before it scrolls.
const LIST_ROWS: usize = 8;
/// The now-playing strip: its height, and its transport buttons.
const STRIP_H: i32 = 34;
const MEDIA_W: i32 = 32;
const MEDIA_H: i32 = 28;
const MEDIA_GAP: i32 = 4;

pub const VOLUME: &str = "Volume";
pub const MUTE: &str = "Mute";
pub const OUTPUT: &str = "Output:";
pub const INPUT: &str = "Input:";
pub const NOW_PLAYING: &str = "Now Playing";

/// The track a media player has on, and what it can do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    /// Empty when the player names none: the title stands alone.
    pub artist: String,
    /// Playing rather than paused: the middle button pauses.
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_play: bool,
    pub can_pause: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Previous,
    PlayPause,
    Next,
}

impl Transport {
    pub const ALL: [Transport; 3] = [Transport::Previous, Transport::PlayPause, Transport::Next];
}

impl NowPlaying {
    /// Whether button `t` acts: Play / Pause by what the middle button
    /// would do now.
    pub fn can(&self, t: Transport) -> bool {
        match t {
            Transport::Previous => self.can_previous,
            Transport::Next => self.can_next,
            Transport::PlayPause if self.playing => self.can_pause,
            Transport::PlayPause => self.can_play,
        }
    }

    pub fn glyph(&self, t: Transport) -> (MediaGlyph, &'static str) {
        match t {
            Transport::Previous => (MediaGlyph::Previous, "Previous"),
            Transport::Next => (MediaGlyph::Next, "Next"),
            Transport::PlayPause if self.playing => (MediaGlyph::Pause, "Pause"),
            Transport::PlayPause => (MediaGlyph::Play, "Play"),
        }
    }
}

/// A device's volume and mute as trackbar positions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub pos: usize,
    pub mute: bool,
    /// False with no device: the column greys.
    pub live: bool,
    /// Master's 32 px icon.
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppLevel {
    /// What ties the program's streams together (`pipewire::App::key`).
    pub key: String,
    pub label: String,
    /// The program's 32 px icon.
    pub icon: Option<Canvas>,
    pub pos: usize,
    pub mute: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mix {
    /// The default sink; `live: false` greys the master column.
    pub master: Level,
    pub outputs: Vec<String>,
    /// The default output's index into `outputs`.
    pub output: Option<usize>,
    pub inputs: Vec<String>,
    pub input: Option<usize>,
    pub apps: Vec<AppLevel>,
    /// The media player's track; `None` with no player, and no strip.
    pub playing: Option<NowPlaying>,
}

/// Whose volume an [`Event`] sets: the default sink, or a program by its
/// key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Channel {
    Master,
    App(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Output,
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Close,
    Volume(Channel, usize),
    Mute(Channel, bool),
    /// A new default device: an index into `Mix::outputs` / `Mix::inputs`.
    Default(Dir, usize),
    Media(Transport),
}

/// One column: master (`key: None`) or a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub key: Option<String>,
    pub label: String,
    pub icon: Option<Canvas>,
    pub volume: TrackBar,
    pub mute: bool,
}

impl Column {
    fn master(level: &Level) -> Column {
        let mut volume = TrackBar::new(level.pos, STEPS).paged(4, 2).untick();
        volume.enabled = level.live;
        Column {
            key: None,
            label: VOLUME.into(),
            icon: level.icon.clone(),
            volume,
            mute: level.mute,
        }
    }

    fn app(a: &AppLevel) -> Column {
        Column {
            key: Some(a.key.clone()),
            label: a.label.clone(),
            icon: a.icon.clone(),
            volume: TrackBar::new(a.pos, STEPS).paged(4, 2).untick(),
            mute: a.mute,
        }
    }
}

/// What the keyboard is on. Column parts carry the column's index into
/// [`VolumePopup::columns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Output,
    Input,
    Thumb(usize),
    Mute(usize),
    Media(Transport),
}

/// The parts the pointer can light or hold. Column parts carry the
/// column's index into [`VolumePopup::columns`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Thumb(usize),
    Mute(usize),
    Combo(Dir),
    Bar(ScrollHit),
    Media(Transport),
}

pub struct VolumePopup {
    pub columns: Vec<Column>,
    pub output: Combo,
    pub input: Combo,
    /// The first program shown, an index into `columns[1..]`.
    pub first: usize,
    pub focus: Focus,
    /// Whether a key has been pressed since the popup opened: only then is
    /// the focus ring drawn.
    pub cues: bool,
    pub ptr: Pointer<Part>,
    pub playing: Option<NowPlaying>,
    /// Where the pointer last pressed or moved on the popup; `None` once
    /// it left. A key or the device moving a thumb finds the part under
    /// it again.
    point: Option<(i32, i32)>,
    grab: i32,
    /// Whether the last draw's look lights a part under the pointer.
    styled: Cell<bool>,
}

#[derive(Clone, Debug)]
pub struct ColumnLayout {
    pub rect: Rect,
    /// The column's icon; `None` on a column without one.
    pub icon: Option<Rect>,
    pub label: Rect,
    /// The level in percent, under the name.
    pub level: Rect,
    pub track: trackbar::Layout,
    /// The speaker button that mutes.
    pub mute: Rect,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub panel: Rect,
    pub output_label: Rect,
    pub output: combo::Layout,
    pub input_label: Rect,
    pub input: combo::Layout,
    pub separator: Rect,
    /// The etched line between master and the programs; `None` with no program.
    pub divider: Option<Rect>,
    /// One per `columns` entry; `None` for a program scrolled out of view.
    pub columns: Vec<Option<ColumnLayout>>,
    pub bar: Option<Rect>,
    /// The look's shortest scrollbar thumb, for the bar's parts.
    pub bar_thumb_min: i32,
    pub strip: Option<StripLayout>,
}

/// Where the now-playing strip's parts go.
#[derive(Clone, Debug)]
pub struct StripLayout {
    pub separator: Rect,
    pub rect: Rect,
    pub title: Rect,
    pub artist: Option<Rect>,
    pub buttons: [Rect; 3],
}

impl StripLayout {
    pub fn button(&self, t: Transport) -> Rect {
        self.buttons[t as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Combo(Dir, combo::Hit),
    Track(usize, trackbar::Hit),
    Mute(usize),
    Column(usize),
    Bar(ScrollHit),
    Media(Transport),
    Panel,
    Outside,
}

/// A column's parts with its top-left at (`x`, `y`), its lengths grown by `u`.
fn column_layout(col: &Column, cm: &ControlMetrics, u: TextUnits, x: i32, y: i32) -> ColumnLayout {
    let l = |v| u.len(v);
    let w = l(COL_W);
    let thumb = (cm.vtrack_thumb + THUMB_WIDER, THUMB_H);
    let track = trackbar::layout_vertical_sized(
        &col.volume,
        x + (w - trackbar::vertical_width(thumb.0)) / 2,
        y + l(TRACK_Y),
        l(TRACK_H),
        thumb,
    );
    let (mw, mh) = (l(MUTE_W), l(MUTE_H));
    ColumnLayout {
        rect: Rect::new(x, y, w, l(COL_H)),
        icon: col
            .icon
            .as_ref()
            .map(|_| Rect::new(x + (w - ICON) / 2, y + (l(ICON) - ICON) / 2, ICON, ICON)),
        label: Rect::new(x, y + l(NAME_Y), w, l(ROW_H)),
        level: Rect::new(x, y + l(LEVEL_Y), w, l(ROW_H)),
        track,
        mute: Rect::new(x + (w - mw) / 2, y + l(MUTE_Y), mw, mh),
    }
}

/// Lay the panel out with its edge nearest the taskbar centred on `anchor`, kept inside `bounds`.
/// Lengths grow by `theme.text`.
pub fn layout(
    vp: &VolumePopup,
    theme: &Theme,
    text: &TextRenderer,
    anchor: (i32, i32),
    edge: Edge,
    bounds: Rect,
) -> Layout {
    let text = text.dialog(DialogFont::ShellDlg);
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let u = theme.text;
    let l = |v| u.len(v);
    let apps = vp.columns.len() - 1;
    let shown = apps.min(MAX_SHOWN) as i32;
    let scrolls = apps > MAX_SHOWN;
    // A flyout border wider than Classic's edge moves everything in.
    let e = cm.flyout_border - flyout::EDGE;
    let margin = l(MARGIN) + e;
    let (col_w, gap) = (l(COL_W), l(COL_GAP));
    let programs_w = shown * col_w + (shown - 1).max(0) * gap;
    let divider_w = if shown > 0 {
        2 * l(DIVIDER_GAP) + SEP_H
    } else {
        0
    };
    let label_w = text
        .measure(OUTPUT, false, px)
        .max(text.measure(INPUT, false, px))
        .max(l(LABEL_W));
    let rows_w = label_w + l(LABEL_GAP) + l(FIELD_W);
    let w = 2 * margin + (col_w + divider_w + programs_w).max(rows_w);
    let bar_h = if scrolls { l(BAR_GAP) + cm.scroll } else { 0 };
    let strip_h = if vp.playing.is_some() {
        l(SEP_GAP) + SEP_H + l(SEP_GAP) + l(STRIP_H)
    } else {
        0
    };
    let h = margin
        + l(COMBO_H)
        + l(COMBO_GAP)
        + l(COMBO_H)
        + l(SEP_GAP)
        + SEP_H
        + l(SEP_GAP)
        + l(COL_H)
        + bar_h
        + strip_h
        + margin;
    // Across the bar the near edge is on the anchor; along it the panel is
    // centred on the anchor, kept inside `bounds`.
    let centred_x = (anchor.0 - w / 2).min(bounds.right() - w).max(bounds.x);
    let centred_y = (anchor.1 - h / 2).min(bounds.bottom() - h).max(bounds.y);
    let (x, y) = match edge {
        Edge::Bottom => (centred_x, (anchor.1 - h).max(bounds.y)),
        Edge::Top => (centred_x, anchor.1.min(bounds.bottom() - h).max(bounds.y)),
        Edge::Left => (anchor.0.min(bounds.right() - w).max(bounds.x), centred_y),
        Edge::Right => ((anchor.0 - w).max(bounds.x), centred_y),
    };
    let combo_h = l(COMBO_H);
    let fx = x + margin + label_w + l(LABEL_GAP);
    let oy = y + margin;
    let iy = oy + combo_h + l(COMBO_GAP);
    let row_h = l(ROW_H);
    let row = |ry: i32| Rect::new(x + margin, ry + (combo_h - row_h) / 2, label_w, row_h);
    let sy = iy + combo_h + l(SEP_GAP);
    let cy = sy + SEP_H + l(SEP_GAP);
    let cx = x + margin;
    let px0 = cx + col_w + divider_w;
    let columns = vp
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let at = match i.checked_sub(1) {
                None => Some(cx),
                Some(a) => (a >= vp.first && a < vp.first + MAX_SHOWN)
                    .then(|| px0 + (a - vp.first) as i32 * (col_w + gap)),
            };
            at.map(|ax| column_layout(c, &cm, u, ax, cy))
        })
        .collect();
    let field = |fy: i32| Rect::new(fx, fy, l(FIELD_W), combo_h);
    let foot = cy + l(COL_H) + bar_h;
    Layout {
        panel: Rect::new(x, y, w, h),
        output_label: row(oy),
        output: combo::layout_rows(&vp.output, field(oy), &cm, cm.list_row),
        input_label: row(iy),
        input: combo::layout_rows(&vp.input, field(iy), &cm, cm.list_row),
        separator: Rect::new(x + margin, sy, w - 2 * margin, SEP_H),
        divider: (shown > 0).then(|| Rect::new(cx + col_w + l(DIVIDER_GAP), cy, SEP_H, l(COL_H))),
        columns,
        bar: scrolls.then(|| Rect::new(px0, cy + l(COL_H) + l(BAR_GAP), programs_w, cm.scroll)),
        bar_thumb_min: cm.thumb_min,
        strip: vp
            .playing
            .as_ref()
            .map(|np| strip_layout(np, u, x + margin, foot + l(SEP_GAP), w - 2 * margin)),
    }
}

fn strip_layout(np: &NowPlaying, u: TextUnits, x: i32, sy: i32, w: i32) -> StripLayout {
    let l = |v| u.len(v);
    let (strip_h, row_h) = (l(STRIP_H), l(ROW_H));
    let (bw, bh, gap) = (l(MEDIA_W), l(MEDIA_H), l(MEDIA_GAP));
    let y = sy + SEP_H + l(SEP_GAP);
    let bx = x + w - (3 * bw + 2 * gap);
    let by = y + (strip_h - bh) / 2;
    let tw = bx - l(LABEL_GAP) - x;
    let (title, artist) = if np.artist.is_empty() {
        (Rect::new(x, y + (strip_h - row_h) / 2, tw, row_h), None)
    } else {
        let ty = y + (strip_h - 2 * row_h) / 2;
        (
            Rect::new(x, ty, tw, row_h),
            Some(Rect::new(x, ty + row_h, tw, row_h)),
        )
    };
    StripLayout {
        separator: Rect::new(x, sy, w, SEP_H),
        rect: Rect::new(x, y, w, strip_h),
        title,
        artist,
        buttons: [0, 1, 2].map(|i| Rect::new(bx + i * (bw + gap), by, bw, bh)),
    }
}

fn bar_parts(vp: &VolumePopup, lay: &Layout, bar: Rect) -> ScrollParts {
    let apps = vp.columns.len() - 1;
    scrollbar_parts(bar, 1, vp.first, MAX_SHOWN, apps, true, lay.bar_thumb_min)
}

pub fn hit(vp: &VolumePopup, lay: &Layout, x: i32, y: i32) -> Hit {
    let combos = [
        (Dir::Output, &vp.output, &lay.output),
        (Dir::Input, &vp.input, &lay.input),
    ];
    // An open list lies over the columns and may run past the panel.
    for (dir, c, cl) in combos {
        if c.open.is_some()
            && let Some(h) = combo::hit(c, cl, x, y)
        {
            return Hit::Combo(dir, h);
        }
    }
    if !lay.panel.contains(x, y) {
        return Hit::Outside;
    }
    for (dir, c, cl) in combos {
        if let Some(h) = combo::hit(c, cl, x, y) {
            return Hit::Combo(dir, h);
        }
    }
    if let Some(bar) = lay.bar
        && bar.contains(x, y)
        && let Some(h) = scrollbar_hit(&bar_parts(vp, lay, bar), x, y, true)
    {
        return Hit::Bar(h);
    }
    if let Some(st) = &lay.strip
        && let Some(t) = Transport::ALL
            .into_iter()
            .find(|&t| st.button(t).contains(x, y))
    {
        return Hit::Media(t);
    }
    for (i, cl) in lay.columns.iter().enumerate() {
        let Some(cl) = cl else { continue };
        if let Some(h) = trackbar::hit(&cl.track, x, y) {
            return Hit::Track(i, h);
        }
        if cl.mute.contains(x, y) {
            return Hit::Mute(i);
        }
        if cl.rect.contains(x, y) {
            return Hit::Column(i);
        }
    }
    Hit::Panel
}

/// The panel, top to bottom; the focus ring shows once a key was pressed, and an open device list draws last, over the columns.
pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, vp: &VolumePopup, lay: &Layout) {
    let text = text.dialog(DialogFont::ShellDlg);
    vp.styled.set(theme.look.lights_pointer());
    let p = &Paint {
        s: 1,
        ..theme.paint()
    };
    let cs = p.cs;
    let px = theme.fonts.size_px;
    // The panel is a program's own flyout: no blurred backdrop under it.
    let one;
    let at_1 = if theme.scale == 1 {
        theme
    } else {
        one = Theme {
            scale: 1,
            ..theme.clone()
        };
        &one
    };
    flyout::draw_shadow(at_1, c, lay.panel);
    flyout::draw_panel(at_1, c, lay.panel, cs.button_face, flyout::Backdrop::None);
    for (dir, label, r) in [
        (Dir::Output, OUTPUT, lay.output_label),
        (Dir::Input, INPUT, lay.input_label),
    ] {
        let combo = vp.combo(dir);
        if combo.enabled {
            draw_label(c, text, p, r, label, cs.button_text, Align::Right, px);
        } else {
            draw_disabled_label(c, text, p, r, label, Align::Right, px, None);
        }
        let lit = vp.ptr.of(Part::Combo(dir));
        combo::draw(
            c,
            text,
            p,
            combo,
            lay.combo(dir),
            vp.focus == combo_focus(dir),
            vp.cues,
            Caret::default(),
            ComboLit {
                frame: lit,
                drop: lit,
            },
            px,
        );
    }
    let sep = lay.separator;
    c.etched_divider_t(sep.x, sep.right() - 1, sep.y, cs, 1);
    if let Some(d) = lay.divider {
        c.etched_vline_t(d.x, d.y, d.bottom(), cs, 1);
    }
    for (i, (col, cl)) in vp.columns.iter().zip(&lay.columns).enumerate() {
        if let Some(cl) = cl {
            draw_column(c, text, p, vp, i, col, cl, px);
        }
    }
    if let Some(bar) = lay.bar {
        let lit = match vp.ptr.lit() {
            Some((Part::Bar(h), how)) => Some((h, how)),
            _ => None,
        };
        let apps = vp.columns.len() - 1;
        draw_hscrollbar(c, p, bar, vp.first, MAX_SHOWN, apps, lit);
    }
    if let (Some(np), Some(st)) = (&vp.playing, &lay.strip) {
        draw_strip(c, text, p, vp, np, st, px);
    }
    if let Some(d) = vp.open_list() {
        let (combo, cl) = (vp.combo(d), lay.combo(d));
        combo::draw_list_rows(
            c,
            text,
            p,
            &combo.items,
            combo.open,
            combo.top,
            combo.rows,
            cl.list,
            cl.scrollbar,
            None,
            px,
            cl.row_h,
        );
    }
}

/// A column's level in percent.
fn level_text(col: &Column) -> String {
    format!("{}%", col.volume.pos * 100 / (STEPS - 1))
}

#[expect(clippy::too_many_arguments)]
fn draw_column(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    vp: &VolumePopup,
    i: usize,
    col: &Column,
    cl: &ColumnLayout,
    px: u32,
) {
    let cs = p.cs;
    let live = col.volume.enabled;
    if let (Some(icon), Some(r)) = (&col.icon, cl.icon) {
        c.blit_over(r.x, r.y, icon);
    }
    if !live {
        draw_disabled_label(c, text, p, cl.label, &col.label, Align::Center, px, None);
    } else if col.key.is_none() {
        draw_label(
            c,
            text,
            p,
            cl.label,
            &col.label,
            cs.button_text,
            Align::Center,
            px,
        );
    } else {
        // A program's name is plain text: an `&` in it is no access key.
        let name = text.ellipsize(&col.label, cl.label.width - 2 * NAME_PAD, false, px);
        text.draw(c, cl.label, &name, false, cs.button_text, Align::Center, px);
    }
    let level = level_text(col);
    if live && !col.mute {
        text.draw(
            c,
            cl.level,
            &level,
            false,
            cs.button_text,
            Align::Center,
            px,
        );
    } else {
        draw_disabled_label(c, text, p, cl.level, &level, Align::Center, px, None);
    }
    draw_vertical_trackbar(
        c,
        p,
        &cl.track,
        live && !col.mute,
        vp.focus == Focus::Thumb(i),
        vp.ptr.of(Part::Thumb(i)),
    );
    let glyph = if col.mute {
        Glyph::SpeakerMuted
    } else {
        Glyph::Speaker
    };
    draw_glyph_button(
        c,
        text,
        p,
        cl.mute,
        (glyph, SPEAKER),
        live,
        vp.ptr.of(Part::Mute(i)),
        live && vp.cues && vp.focus == Focus::Mute(i),
        px,
    );
    if vp.cues && live && vp.focus == Focus::Thumb(i) {
        draw_focus_ring(c, p, cl.track.bounds, cs.button_text);
    }
}

fn draw_strip(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    vp: &VolumePopup,
    np: &NowPlaying,
    st: &StripLayout,
    px: u32,
) {
    let cs = p.cs;
    let sep = st.separator;
    c.etched_divider_t(sep.x, sep.right() - 1, sep.y, cs, 1);
    let lines = [(st.title, &np.title)]
        .into_iter()
        .chain(st.artist.map(|r| (r, &np.artist)));
    for (r, line) in lines {
        let cut = text.ellipsize(line, r.width, false, px);
        text.draw(c, r, &cut, false, cs.button_text, Align::Left, px);
    }
    for t in Transport::ALL {
        let (glyph, _) = np.glyph(t);
        draw_media_button(
            c,
            text,
            p,
            st.button(t),
            glyph,
            np.can(t),
            vp.ptr.of(Part::Media(t)),
            vp.cues && vp.focus == Focus::Media(t),
            px,
        );
    }
}

fn device_combo(items: &[String], picked: Option<usize>) -> Combo {
    let names: Vec<&str> = items.iter().map(String::as_str).collect();
    let mut c = Combo::new(&names, picked.unwrap_or(0), items.len().min(LIST_ROWS));
    c.enabled = !items.is_empty();
    c
}

/// Take a device list: the pick follows the default; a combo whose items
/// changed is built again, its list closed. Returns whether it was.
fn sync_combo(c: &mut Combo, items: &[String], picked: Option<usize>) -> bool {
    let same = c.items.len() == items.len() && c.items.iter().zip(items).all(|(a, b)| a.text == *b);
    if !same {
        *c = device_combo(items, picked);
        return true;
    }
    c.selected = picked.unwrap_or(0);
    c.enabled = !items.is_empty();
    false
}

fn combo_focus(dir: Dir) -> Focus {
    match dir {
        Dir::Output => Focus::Output,
        Dir::Input => Focus::Input,
    }
}

impl Layout {
    fn combo(&self, dir: Dir) -> &combo::Layout {
        match dir {
            Dir::Output => &self.output,
            Dir::Input => &self.input,
        }
    }
}

impl VolumePopup {
    pub fn new(mix: &Mix) -> VolumePopup {
        let mut columns = vec![Column::master(&mix.master)];
        columns.extend(mix.apps.iter().map(Column::app));
        VolumePopup {
            columns,
            output: device_combo(&mix.outputs, mix.output),
            input: device_combo(&mix.inputs, mix.input),
            first: 0,
            focus: Focus::Thumb(0),
            cues: false,
            ptr: Pointer::new(),
            playing: mix.playing.clone(),
            point: None,
            grab: 0,
            styled: Cell::new(false),
        }
    }

    pub fn channel(&self, col: usize) -> Channel {
        match &self.columns[col].key {
            None => Channel::Master,
            Some(k) => Channel::App(k.clone()),
        }
    }

    /// Take the devices and programs from the host. Programs are matched by key: one that stays keeps its focus, light and drag; one that left sends its focus to master and ends its drag. A held thumb keeps its own position since the device lags the drag's writes. Changed combos close their lists; `Redraw` when anything shown moved.
    pub fn sync(&mut self, lay: &Layout, mix: &Mix) -> Event {
        let mut columns = vec![Column::master(&mix.master)];
        columns.extend(mix.apps.iter().map(Column::app));
        let moved: Vec<Option<usize>> = self
            .columns
            .iter()
            .map(|old| match &old.key {
                None => Some(0),
                Some(k) => columns.iter().position(|c| c.key.as_ref() == Some(k)),
            })
            .collect();
        let remap = |i: usize| moved.get(i).copied().flatten();
        if let Some(Part::Thumb(i)) = self.ptr.held()
            && let Some(j) = remap(i)
        {
            columns[j].volume.set(self.columns[i].volume.pos);
        }
        let can = |t: Transport| mix.playing.as_ref().is_some_and(|np| np.can(t));
        self.focus = match self.focus {
            Focus::Thumb(i) => remap(i).map_or(Focus::Thumb(0), Focus::Thumb),
            Focus::Mute(i) => remap(i).map_or(Focus::Thumb(0), Focus::Mute),
            Focus::Media(t) if !can(t) => Focus::Thumb(0),
            f => f,
        };
        let output = sync_combo(&mut self.output, &mix.outputs, mix.output);
        let input = sync_combo(&mut self.input, &mix.inputs, mix.input);
        let bar = columns.len() - 1 > MAX_SHOWN;
        self.ptr = std::mem::take(&mut self.ptr).carry(|p| match p {
            Part::Thumb(i) => remap(i).map(Part::Thumb),
            Part::Mute(i) => remap(i).map(Part::Mute),
            Part::Combo(Dir::Output) if output => None,
            Part::Combo(Dir::Input) if input => None,
            Part::Bar(_) if !bar => None,
            Part::Media(t) if !can(t) => None,
            p => Some(p),
        });
        self.columns = columns;
        self.playing = mix.playing.clone();
        self.first = self
            .first
            .min(self.columns.len().saturating_sub(1 + MAX_SHOWN));
        self.rehit(lay)
    }

    pub fn dragging(&self) -> bool {
        matches!(self.ptr.held(), Some(Part::Thumb(_)))
    }

    /// `lay` with every shown thumb where its volume puts it now.
    fn current(&self, lay: &Layout) -> Layout {
        let columns = lay
            .columns
            .iter()
            .zip(&self.columns)
            .map(|(cl, c)| {
                cl.as_ref().map(|cl| ColumnLayout {
                    track: trackbar::at_position(&cl.track, c.volume.pos),
                    ..cl.clone()
                })
            })
            .collect();
        Layout {
            columns,
            ..lay.clone()
        }
    }

    /// The part at (`x`, `y`) on `lay` laid out by [`Self::current`].
    /// Only master greys; a greyed combo lights nothing.
    fn part_at(&self, lay: &Layout, x: i32, y: i32) -> Option<Part> {
        let live = |i: usize| self.columns.get(i).is_some_and(|c| c.volume.enabled);
        match hit(self, lay, x, y) {
            Hit::Track(i, trackbar::Hit::Thumb) if live(i) => Some(Part::Thumb(i)),
            Hit::Mute(i) if live(i) => Some(Part::Mute(i)),
            Hit::Combo(dir, _) if self.combo(dir).enabled => Some(Part::Combo(dir)),
            Hit::Bar(h) => Some(Part::Bar(h)),
            Hit::Media(t) if self.can(t) => Some(Part::Media(t)),
            _ => None,
        }
    }

    fn can(&self, t: Transport) -> bool {
        self.playing.as_ref().is_some_and(|np| np.can(t))
    }

    fn combo(&self, dir: Dir) -> &Combo {
        match dir {
            Dir::Output => &self.output,
            Dir::Input => &self.input,
        }
    }

    fn combo_mut(&mut self, dir: Dir) -> &mut Combo {
        match dir {
            Dir::Output => &mut self.output,
            Dir::Input => &mut self.input,
        }
    }

    fn open_list(&self) -> Option<Dir> {
        [Dir::Output, Dir::Input]
            .into_iter()
            .find(|&d| self.combo(d).open.is_some())
    }

    /// A press on combo `dir`'s open list: a row is picked (`Default` when it changed), an arrow or the track scrolls, the field or drop button closes it.
    fn press_list(&mut self, dir: Dir, h: combo::Hit) -> Event {
        let c = self.combo_mut(dir);
        let page = c.rows as i32;
        let rows = match h {
            combo::Hit::Row(i) => {
                return if c.pick(i) {
                    Event::Default(dir, i)
                } else {
                    Event::Redraw
                };
            }
            combo::Hit::Field | combo::Hit::Drop => {
                c.open = None;
                return Event::Redraw;
            }
            combo::Hit::Thumb(_) => return Event::None,
            combo::Hit::ScrollUp => -1,
            combo::Hit::ScrollDown => 1,
            combo::Hit::PageUp => -page,
            combo::Hit::PageDown => page,
        };
        if c.scroll(rows) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    /// A key on combo `dir`'s open list: arrows, Home / End and Page keys move the highlight, Enter takes it, Esc, Tab or the drop key close it.
    fn list_key(&mut self, dir: Dir, key: form::Key) -> Event {
        let c = self.combo_mut(dir);
        let page = c.rows as i32;
        let _ = match key {
            form::Key::Up | form::Key::Down => c.step(key == form::Key::Down),
            form::Key::Home | form::Key::End => c.jump(key == form::Key::End),
            form::Key::PageUp => c.scroll(-page),
            form::Key::PageDown => c.scroll(page),
            form::Key::Enter if c.take() => return Event::Default(dir, c.selected),
            form::Key::Enter | form::Key::Escape | form::Key::Tab { .. } | form::Key::Drop => {
                c.open = None;
                false
            }
            _ => return Event::None,
        };
        Event::Redraw
    }

    /// A key on a focused combo with its list closed: arrows, Home / End and Page keys pick at once; F4 or Alt+Down opens the list.
    fn combo_key(&mut self, dir: Dir, key: form::Key) -> Event {
        let c = self.combo_mut(dir);
        let changed = match key {
            form::Key::Up | form::Key::Left => c.step(false),
            form::Key::Down | form::Key::Right => c.step(true),
            form::Key::Home | form::Key::PageUp => c.jump(false),
            form::Key::End | form::Key::PageDown => c.jump(true),
            form::Key::Drop => {
                return if c.toggle() {
                    Event::Redraw
                } else {
                    Event::None
                };
            }
            _ => false,
        };
        if changed {
            Event::Default(dir, c.selected)
        } else {
            Event::None
        }
    }

    fn rehit(&mut self, lay: &Layout) -> Event {
        let Some((x, y)) = self.point else {
            return Event::None;
        };
        let over = self.part_at(&self.current(lay), x, y);
        let changed = self.ptr.motion(over);
        self.lit(changed)
    }

    /// `Redraw` when what draws changed and the look shows it; a held scrollbar part or transport button shows pressed in every look.
    fn lit(&self, changed: bool) -> Event {
        let bar = matches!(self.ptr.held(), Some(Part::Bar(_) | Part::Media(_)));
        if changed && (self.styled.get() || bar) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    fn lit_from(&self, before: Lit<Part>) -> Event {
        let changed = self.ptr.lit() != before;
        if changed
            && matches!(
                before,
                Some((Part::Bar(_) | Part::Media(_), Interaction::Pressed))
            )
        {
            Event::Redraw
        } else {
            self.lit(changed)
        }
    }

    fn slide(&mut self, i: usize, f: impl FnOnce(&mut TrackBar) -> bool) -> Event {
        let ch = self.channel(i);
        let t = &mut self.columns[i].volume;
        if t.enabled && f(t) {
            Event::Volume(ch, t.pos)
        } else {
            Event::None
        }
    }

    fn flip(&mut self, i: usize) -> Event {
        if !self.columns[i].volume.enabled {
            return Event::None;
        }
        let ch = self.channel(i);
        let c = &mut self.columns[i];
        c.mute = !c.mute;
        Event::Mute(ch, c.mute)
    }

    /// The focus order: Output, Input, each column's thumb and Mute, then
    /// the transport buttons. A greyed combo, column or button is no stop.
    fn focus_order(&self) -> Vec<Focus> {
        let mut v = Vec::new();
        for (f, c) in [(Focus::Output, &self.output), (Focus::Input, &self.input)] {
            if c.enabled {
                v.push(f);
            }
        }
        for (i, c) in self.columns.iter().enumerate() {
            if c.volume.enabled {
                v.extend([Focus::Thumb(i), Focus::Mute(i)]);
            }
        }
        v.extend(
            Transport::ALL
                .into_iter()
                .filter(|&t| self.can(t))
                .map(Focus::Media),
        );
        v
    }

    fn walk(&mut self, back: bool) {
        let order = self.focus_order();
        let n = order.len();
        if n == 0 {
            return;
        }
        let next = match order.iter().position(|f| *f == self.focus) {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        self.focus = order[next];
        if let Focus::Thumb(i) | Focus::Mute(i) = self.focus {
            self.reveal(i);
        }
    }

    fn reveal(&mut self, i: usize) {
        if i == 0 {
            return;
        }
        let a = i - 1;
        if a < self.first {
            self.first = a;
        } else if a >= self.first + MAX_SHOWN {
            self.first = a + 1 - MAX_SHOWN;
        }
    }

    fn last_first(&self) -> usize {
        self.columns.len().saturating_sub(1 + MAX_SHOWN)
    }

    fn scroll_bar(&mut self, lay: &Layout, notches: i32) -> Event {
        let first = (self.first as i64 + i64::from(notches)).clamp(0, self.last_first() as i64);
        if first as usize == self.first {
            return Event::None;
        }
        self.first = first as usize;
        self.rehit(lay);
        Event::Redraw
    }

    /// A press on the program scrollbar: an arrow scrolls one column, the track a page; the thumb is held `grab` px from its left edge.
    fn press_bar(&mut self, lay: &Layout, h: ScrollHit, x: i32) -> Event {
        let last = self.last_first();
        self.first = match h {
            ScrollHit::Back => self.first.saturating_sub(1),
            ScrollHit::Forward => (self.first + 1).min(last),
            ScrollHit::PageBack => self.first.saturating_sub(MAX_SHOWN),
            ScrollHit::PageForward => (self.first + MAX_SHOWN).min(last),
            ScrollHit::Thumb => {
                if let Some(bar) = lay.bar
                    && let Some(thumb) = bar_parts(self, lay, bar).thumb
                {
                    self.grab = x - thumb.x;
                }
                self.first
            }
        };
        Event::Redraw
    }

    /// The left button went down at (`x`, `y`). The thumb is where the volume puts it now; a press on the channel steps it one position; focus moves to the pressed part; a combo's field or drop button opens its list. With a list open, a press on it acts there and any other press closes it; outside the panel closes the popup.
    /// A reader's Focus on the control at (`x`, `y`): it takes the keyboard as Tab gives it, with
    /// its focus ring shown, without sliding, muting, playing or dropping anything.
    pub fn focus_at(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        let live = |i: usize| self.columns[i].volume.enabled;
        let f = match hit(self, lay, x, y) {
            Hit::Track(i, _) if live(i) => Focus::Thumb(i),
            Hit::Mute(i) if live(i) => Focus::Mute(i),
            Hit::Media(t) if self.can(t) => Focus::Media(t),
            Hit::Combo(d, combo::Hit::Field | combo::Hit::Drop) if self.combo(d).enabled => {
                combo_focus(d)
            }
            _ => return Event::None,
        };
        let cued = !self.cues;
        self.cues = true;
        if self.focus == f {
            return if cued { Event::Redraw } else { Event::None };
        }
        if let Some(d) = self.open_list() {
            self.combo_mut(d).open = None;
        }
        self.focus = f;
        Event::Redraw
    }

    pub fn press(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        self.point = Some((x, y));
        if let Some(d) = self.open_list() {
            if let Hit::Combo(hd, h) = hit(self, lay, x, y)
                && hd == d
            {
                return self.press_list(d, h);
            }
            self.combo_mut(d).open = None;
            // Nothing under it acts at the release either.
            self.ptr.clear();
            return Event::Redraw;
        }
        let part = self.part_at(lay, x, y);
        let sticky = matches!(part, Some(Part::Thumb(_) | Part::Bar(ScrollHit::Thumb)));
        let changed = self.ptr.press(part, sticky);
        let live = |i: usize| self.columns[i].volume.enabled;
        match hit(self, lay, x, y) {
            Hit::Outside => Event::Close,
            Hit::Track(i, _) | Hit::Mute(i) if !live(i) => Event::None,
            Hit::Track(i, h) => {
                self.focus = Focus::Thumb(i);
                match h {
                    trackbar::Hit::Thumb => Event::Redraw,
                    trackbar::Hit::Left => self.slide(i, |t| t.step(-1)),
                    trackbar::Hit::Right => self.slide(i, |t| t.step(1)),
                }
            }
            Hit::Mute(i) => {
                self.focus = Focus::Mute(i);
                Event::Redraw
            }
            Hit::Media(t) if self.can(t) => {
                self.focus = Focus::Media(t);
                Event::Redraw
            }
            Hit::Bar(h) => self.press_bar(lay, h, x),
            Hit::Combo(d, combo::Hit::Field | combo::Hit::Drop) if self.combo(d).enabled => {
                self.focus = combo_focus(d);
                self.combo_mut(d).toggle();
                Event::Redraw
            }
            _ => self.lit(changed),
        }
    }

    /// The pointer moved: a held thumb follows it, then the part under it is found again. A held scrollbar thumb scrolls the programs to follow the pointer's x.
    pub fn motion(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        self.point = Some((x, y));
        let moved = match self.ptr.held() {
            Some(Part::Thumb(i)) => match &lay.columns[i] {
                Some(cl) => {
                    let pos = trackbar::position_at(&cl.track, y);
                    self.slide(i, |t| t.set(pos))
                }
                None => Event::None,
            },
            Some(Part::Bar(ScrollHit::Thumb)) => match lay.bar {
                Some(bar) => {
                    let p = bar_parts(self, lay, bar);
                    let apps = self.columns.len() - 1;
                    let to = scrollbar_pos_at(&p, x - self.grab, MAX_SHOWN, apps, true);
                    if to == self.first {
                        Event::None
                    } else {
                        self.first = to;
                        Event::Redraw
                    }
                }
                None => Event::None,
            },
            _ => Event::None,
        };
        let changed = self.ptr.motion(self.part_at(&self.current(lay), x, y));
        match moved {
            Event::None => self.lit(changed),
            ev => ev,
        }
    }

    /// The button came up: on the Mute box it held, the box flips; on the
    /// transport button it held, that button acts.
    pub fn release(&mut self) -> Event {
        let before = self.ptr.lit();
        match self.ptr.release() {
            Some(Part::Mute(i)) => self.flip(i),
            Some(Part::Media(t)) => Event::Media(t),
            _ => self.lit_from(before),
        }
    }

    pub fn leave(&mut self) -> Event {
        self.point = None;
        let changed = self.ptr.leave();
        self.lit(changed)
    }

    /// The wheel turned `rows` rows ([`WHEEL_ROWS`] per notch, positive is down) at (`x`, `y`): over an open list it scrolls it, over the program scrollbar a column per notch, over a column that column (down is quieter), anywhere else master. A turn short of a notch counts as one. The focus stays.
    pub fn scroll(&mut self, lay: &Layout, x: i32, y: i32, rows: i32) -> Event {
        if let Some(d) = self.open_list()
            && lay.combo(d).list.contains(x, y)
        {
            return if self.combo_mut(d).scroll(rows) {
                Event::Redraw
            } else {
                Event::None
            };
        }
        let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
        let i = match hit(self, lay, x, y) {
            Hit::Track(i, _) | Hit::Mute(i) | Hit::Column(i) => i,
            Hit::Bar(_) => return self.scroll_bar(lay, notches),
            _ => 0,
        };
        let ev = self.slide(i, |t| t.step(-notches));
        let lit = self.rehit(lay);
        match ev {
            Event::None => lit,
            ev => ev,
        }
    }

    /// Up / Down, Page Up / Page Down, Home and End move the focused thumb (Up is louder) or pick on the focused combo; Tab and Shift+Tab walk the focus order; Space flips Mute or clicks a transport button (so does Enter); Esc closes. With a list open every key goes to the list. A key on a program scrolled out of view scrolls it back first.
    pub fn key(&mut self, lay: &Layout, key: form::Key) -> Event {
        let first = !self.cues;
        self.cues = true;
        if let Some(d) = self.open_list() {
            return self.list_key(d, key);
        }
        let shown = self.first;
        if let Focus::Thumb(i) | Focus::Mute(i) = self.focus
            && !matches!(key, form::Key::Tab { .. } | form::Key::Escape)
        {
            self.reveal(i);
        }
        let ev = match (key, self.focus) {
            (form::Key::Tab { back }, _) => {
                self.walk(back);
                Event::Redraw
            }
            (form::Key::Escape, _) => return Event::Close,
            (form::Key::Space, Focus::Mute(i)) => self.flip(i),
            (form::Key::Space | form::Key::Enter, Focus::Media(t)) if self.can(t) => {
                Event::Media(t)
            }
            (k, Focus::Output) => self.combo_key(Dir::Output, k),
            (k, Focus::Input) => self.combo_key(Dir::Input, k),
            (k, Focus::Thumb(i)) => match k {
                form::Key::Up | form::Key::Right => self.slide(i, |t| t.step(1)),
                form::Key::Down | form::Key::Left => self.slide(i, |t| t.step(-1)),
                form::Key::PageUp => self.slide(i, |t| t.step(t.page as i32)),
                form::Key::PageDown => self.slide(i, |t| t.step(-(t.page as i32))),
                form::Key::Home => self.slide(i, |t| t.set(STEPS - 1)),
                form::Key::End => self.slide(i, |t| t.set(0)),
                _ => Event::None,
            },
            _ => Event::None,
        };
        let lit = self.rehit(lay);
        // The first key shows the focus ring even when it moves nothing.
        match ev {
            Event::None if first || self.first != shown => Event::Redraw,
            Event::None => lit,
            ev => ev,
        }
    }

    /// The popup's `access` part: the Output and Input combos, a group per shown column (label, slider, Mute), and with a player a "Now Playing" group. Returns the focused node.
    pub fn access(
        &self,
        lay: &Layout,
        text: &TextRenderer,
        px: u32,
        tree: &mut Tree,
        parent: NodeId,
    ) -> NodeId {
        let text = text.dialog(DialogFont::ShellDlg);
        let mut win = access::node(Role::Window, lay.panel);
        win.set_label(VOLUME);
        let win = tree.push(parent, "volume-popup", win);
        let mut focus = win;
        for (dir, name) in [(Dir::Output, "Output"), (Dir::Input, "Input")] {
            let f = combo::access(self.combo(dir), lay.combo(dir), text, px, tree, win, name);
            if let Some(n) = tree.get_mut(Tree::id_of(win, name)) {
                n.set_label(name);
            }
            if self.focus == combo_focus(dir) {
                focus = f;
            }
        }
        for (i, (col, cl)) in self.columns.iter().zip(&lay.columns).enumerate() {
            let Some(cl) = cl else { continue };
            let live = col.volume.enabled;
            let mut g = access::node(Role::Group, cl.rect);
            // A program's name is plain text: no access key to drop.
            g.set_label(col.label.as_str());
            let g = tree.push(win, ("column", col.key.as_deref()), g);
            let mut label = access::node(Role::Label, cl.label);
            label.set_value(col.label.as_str());
            if !live {
                label.set_disabled();
            }
            let label = tree.push(g, "label", label);
            let slider = trackbar::access(&col.volume, &cl.track, tree, g, "slider");
            if let Some(n) = tree.get_mut(slider) {
                n.push_labelled_by(label);
            }
            let mut level = access::node(Role::Label, cl.level);
            level.set_value(level_text(col));
            if !live {
                level.set_disabled();
            }
            tree.push(g, "level", level);
            let mute = tree.push(
                g,
                "mute",
                access::toggle(Role::Button, cl.mute, MUTE, col.mute.into(), live),
            );
            match self.focus {
                Focus::Thumb(f) if f == i => focus = slider,
                Focus::Mute(f) if f == i => focus = mute,
                _ => {}
            }
        }
        if let (Some(np), Some(st)) = (&self.playing, &lay.strip) {
            let g = tree.push(
                win,
                "now-playing",
                access::named(Role::Group, st.rect, NOW_PLAYING),
            );
            // A track's names are plain text: an `&` in them is no access key.
            let lines = [("title", st.title, &np.title)]
                .into_iter()
                .chain(st.artist.map(|r| ("artist", r, &np.artist)));
            for (key, r, line) in lines {
                let mut n = access::node(Role::Label, r);
                n.set_value(line.as_str());
                tree.push(g, key, n);
            }
            for t in Transport::ALL {
                let (_, name) = np.glyph(t);
                let b = tree.push(
                    g,
                    ("media", t as usize),
                    access::button(st.button(t), name, np.can(t), false),
                );
                if self.focus == Focus::Media(t) {
                    focus = b;
                }
            }
        }
        focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;

    fn app(key: &str, pos: usize) -> AppLevel {
        AppLevel {
            key: key.into(),
            label: key.into(),
            icon: None,
            pos,
            mute: false,
        }
    }

    fn mix(apps: &[AppLevel]) -> Mix {
        Mix {
            master: Level {
                pos: 10,
                mute: false,
                live: true,
                icon: None,
            },
            outputs: vec!["Speakers".into(), "Headphones".into()],
            output: Some(0),
            inputs: vec!["Microphone".into()],
            input: Some(0),
            apps: apps.to_vec(),
            playing: None,
        }
    }

    fn master(pos: usize, mute: bool, live: bool) -> Mix {
        Mix {
            master: Level {
                pos,
                mute,
                live,
                icon: None,
            },
            ..Default::default()
        }
    }

    fn lay_with(vp: &VolumePopup, theme: &Theme, anchor: (i32, i32)) -> Layout {
        let text = TextRenderer::load(&theme.fonts);
        layout(
            vp,
            theme,
            &text,
            anchor,
            Edge::Bottom,
            Rect::new(0, 0, 1920, 1080),
        )
    }

    fn lay(vp: &VolumePopup) -> Layout {
        lay_with(vp, &Theme::default(), (960, 1052))
    }

    fn mid(r: Rect) -> (i32, i32) {
        (r.x + r.width / 2, r.y + r.height / 2)
    }

    fn col(l: &Layout, i: usize) -> &ColumnLayout {
        l.columns[i].as_ref().expect("column shown")
    }

    #[test]
    fn a_readers_focus_moves_to_a_slider_or_mute_without_acting() {
        let mut vp = VolumePopup::new(&mix(&[app("firefox", 16)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).track.bounds);
        let pos = vp.columns[1].volume.pos;
        assert_eq!(vp.focus_at(&l, x, y), Event::Redraw);
        assert_eq!(vp.focus, Focus::Thumb(1));
        assert_eq!(vp.columns[1].volume.pos, pos, "the thumb stays");
        let (x, y) = mid(col(&l, 0).mute);
        assert_eq!(vp.focus_at(&l, x, y), Event::Redraw);
        assert_eq!(vp.focus, Focus::Mute(0));
        assert!(!vp.columns[0].mute, "nothing is muted");
        assert_eq!(vp.focus_at(&l, x, y), Event::None);
    }

    #[test]
    fn a_readers_focus_shows_the_focus_ring() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        assert!(!vp.cues, "opened with the pointer");
        let (x, y) = mid(col(&l, 0).track.bounds);
        assert_eq!(vp.focus, Focus::Thumb(0));
        assert_eq!(vp.focus_at(&l, x, y), Event::Redraw, "the ring comes out");
        assert!(vp.cues);
        assert_eq!(vp.focus_at(&l, x, y), Event::None);
    }

    #[test]
    fn with_no_programs_the_panel_is_444_wide() {
        let vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        assert_eq!(l.panel, Rect::new(738, 711, 444, 341));
        assert_eq!(col(&l, 0).rect, Rect::new(750, 793, 96, 247));
        assert_eq!(l.divider, None, "no programs, no line after master");
    }

    #[test]
    fn the_panel_stays_inside_the_bounds() {
        let vp = VolumePopup::new(&mix(&[]));
        let theme = Theme::default();
        assert_eq!(lay_with(&vp, &theme, (1915, 1052)).panel.right(), 1920);
        assert_eq!(lay_with(&vp, &theme, (2, 1052)).panel.x, 0);
        assert_eq!(lay_with(&vp, &theme, (960, 100)).panel.y, 0);
    }

    #[test]
    fn from_a_top_bar_the_panel_hangs_under_the_anchor() {
        let vp = VolumePopup::new(&mix(&[]));
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let screen = Rect::new(0, 0, 1920, 1080);
        let l = layout(&vp, &theme, &text, (960, 28), Edge::Top, screen);
        assert_eq!(l.panel, Rect::new(738, 28, 444, 341));
        assert_eq!(col(&l, 0).rect, Rect::new(750, 110, 96, 247));
        let low = layout(&vp, &theme, &text, (960, 1000), Edge::Top, screen);
        assert_eq!(low.panel.bottom(), 1080, "kept inside the bounds");
    }

    #[test]
    fn programs_stand_102_px_apart_past_an_etched_line_after_master() {
        let vp = VolumePopup::new(&mix(&[app("firefox", 16), app("mpv", 8)]));
        let l = lay(&vp);
        assert_eq!(l.panel, Rect::new(738, 711, 444, 341), "the combos' width");
        assert_eq!(col(&l, 0).rect.x, 750);
        assert_eq!(l.divider, Some(Rect::new(852, 793, 2, 247)));
        assert_eq!(col(&l, 1).rect.x, 860);
        assert_eq!(col(&l, 2).rect.x, 962);
        assert_eq!(l.bar, None);
    }

    #[test]
    fn master_and_six_programs_fit_740_px() {
        let apps: Vec<_> = (0..6).map(|i| app(&format!("p{i}"), 10)).collect();
        let vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        assert_eq!(l.panel, Rect::new(590, 711, 740, 341));
        assert_eq!(col(&l, 6).rect.right(), l.panel.right() - 12);
        assert_eq!(l.bar, None, "six fit without a scrollbar");
    }

    #[test]
    fn a_column_stacks_icon_name_level_trackbar_and_speaker() {
        let mut a = app("firefox", 16);
        a.icon = Some(Canvas::new(32, 32));
        let vp = VolumePopup::new(&mix(&[a]));
        let l = lay(&vp);
        let c = col(&l, 1);
        let x = c.rect.x;
        assert_eq!(c.icon, Some(Rect::new(x + 32, 793, 32, 32)));
        assert_eq!(c.label, Rect::new(x, 831, 96, 13));
        assert_eq!(c.level, Rect::new(x, 845, 96, 13));
        assert_eq!(c.track.bounds, Rect::new(x + 30, 866, 35, 140));
        assert_eq!(
            (c.track.thumb.width, c.track.thumb.height),
            (25, 15),
            "a thumb 4 px wider than the look's and 15 tall"
        );
        assert!(c.track.ticks.len() == STEPS && !c.track.ticked, "no ticks");
        assert_eq!(c.mute, Rect::new(x + 33, 1014, 30, 26));
        assert_eq!(l.panel.bottom() - c.mute.bottom(), 12);
        assert_eq!(col(&l, 0).icon, None, "master without an icon has none");
    }

    #[test]
    fn the_thumb_stays_on_the_channel_at_either_end() {
        let mut vp = VolumePopup::new(&mix(&[]));
        for pos in [0, STEPS - 1] {
            vp.columns[0].volume.set(pos);
            let t = col(&lay(&vp), 0).track.clone();
            assert!(
                t.thumb.y >= t.channel.y && t.thumb.bottom() <= t.channel.bottom(),
                "position {pos}: {:?} inside {:?}",
                t.thumb,
                t.channel
            );
        }
    }

    #[test]
    fn past_six_programs_a_scrollbar_runs_under_them() {
        let apps: Vec<_> = (0..7).map(|i| app(&format!("p{i}"), 10)).collect();
        let vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        assert_eq!(l.panel, Rect::new(590, 687, 740, 365));
        assert_eq!(l.bar, Some(Rect::new(712, 769 + 247 + 8, 606, 16)));
        assert!(l.columns[1..=6].iter().all(Option::is_some));
        assert!(l.columns[7..].iter().all(Option::is_none));
    }

    #[test]
    fn the_combos_are_360_wide_right_of_the_widest_label() {
        let vp = VolumePopup::new(&mix(&[]));
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let l = lay(&vp);
        let px = theme.fonts.size_px;
        let label_w = text
            .measure(OUTPUT, false, px)
            .max(text.measure(INPUT, false, px))
            .max(52);
        assert_eq!(
            l.output.field,
            Rect::new(738 + 12 + label_w + 8, 723, 360, 21)
        );
        assert_eq!(l.input.field.y, 750);
        assert_eq!(l.separator, Rect::new(750, 781, 420, 2));
    }

    #[test]
    fn a_device_list_has_list_rows() {
        let vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let rows = ControlMetrics::CLASSIC.list_row;
        assert_eq!(l.output.row_h, rows);
        assert_eq!(l.output.list.height, 2 * rows + 2, "two outputs");
    }

    #[test]
    fn sync_keeps_a_program_by_key_and_drops_one_that_left() {
        let mut vp = VolumePopup::new(&mix(&[app("firefox", 16), app("mpv", 8)]));
        let l = lay(&vp);
        vp.focus = Focus::Mute(2);
        vp.sync(&l, &mix(&[app("mpv", 9), app("vlc", 4)]));
        assert_eq!(vp.columns.len(), 3);
        assert_eq!(vp.columns[1].key.as_deref(), Some("mpv"));
        assert_eq!(vp.columns[1].volume.pos, 9);
        assert_eq!(
            vp.focus,
            Focus::Mute(1),
            "focus follows mpv to its new place"
        );
        vp.focus = Focus::Thumb(2);
        let l = lay(&vp);
        vp.sync(&l, &mix(&[app("mpv", 9)]));
        assert_eq!(vp.focus, Focus::Thumb(0), "vlc left: focus goes to master");
    }

    #[test]
    fn sync_carries_a_held_thumb_and_ends_a_drag_on_a_program_that_left() {
        let mut vp = VolumePopup::new(&mix(&[app("firefox", 16), app("mpv", 8)]));
        vp.ptr.press(Some(Part::Thumb(2)), true);
        vp.columns[2].volume.set(12);
        let l = lay(&vp);
        vp.sync(&l, &mix(&[app("mpv", 3)]));
        assert_eq!(vp.ptr.held(), Some(Part::Thumb(1)), "the drag follows mpv");
        assert_eq!(vp.columns[1].volume.pos, 12, "the held thumb stays put");
        let l = lay(&vp);
        vp.sync(&l, &mix(&[]));
        assert_eq!(vp.ptr.held(), None, "mpv left: its drag ends");
    }

    #[test]
    fn sync_clamps_the_scroll_to_the_programs_left() {
        let apps: Vec<_> = (0..9).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        vp.first = 3;
        let l = lay(&vp);
        vp.sync(&l, &mix(&apps[..7]));
        assert_eq!(vp.first, 1);
        let l = lay(&vp);
        vp.sync(&l, &mix(&apps[..2]));
        assert_eq!(vp.first, 0);
    }

    #[test]
    fn sync_closes_a_device_list_whose_items_changed() {
        let mut vp = VolumePopup::new(&mix(&[]));
        vp.output.open = Some(1);
        vp.input.open = Some(0);
        let l = lay(&vp);
        let mut m = mix(&[]);
        m.outputs.truncate(1);
        m.input = None;
        vp.sync(&l, &m);
        assert_eq!(vp.output.open, None, "the output list lost an item");
        assert_eq!(vp.output.items.len(), 1);
        assert_eq!(vp.input.open, Some(0), "the input list is unchanged");
        m.output = Some(0);
        m.outputs.clear();
        vp.sync(&l, &m);
        assert!(!vp.output.enabled, "no outputs: greyed");
    }

    #[test]
    fn channel_names_master_or_the_program_key() {
        let vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        assert_eq!(vp.channel(0), Channel::Master);
        assert_eq!(vp.channel(1), Channel::App("mpv".into()));
    }

    #[test]
    fn hits_find_the_column_parts_and_the_combos() {
        let vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).track.thumb);
        assert_eq!(hit(&vp, &l, x, y), Hit::Track(1, trackbar::Hit::Thumb));
        let (x, y) = mid(col(&l, 1).mute);
        assert_eq!(hit(&vp, &l, x, y), Hit::Mute(1));
        let (x, y) = mid(col(&l, 1).label);
        assert_eq!(hit(&vp, &l, x, y), Hit::Column(1));
        let (x, y) = mid(l.output.drop);
        assert_eq!(
            hit(&vp, &l, x, y),
            Hit::Combo(Dir::Output, combo::Hit::Drop)
        );
        assert_eq!(hit(&vp, &l, 10, 10), Hit::Outside);
    }

    #[test]
    fn an_open_list_lies_over_the_columns() {
        let mut m = mix(&[app("mpv", 8)]);
        m.outputs = (0..6).map(|i| format!("Output {i}")).collect();
        let mut vp = VolumePopup::new(&m);
        vp.output.open = Some(0);
        let l = lay(&vp);
        let (x, y) = (l.output.list.x + 4, l.output.list.bottom() - 3);
        assert!(y > col(&l, 0).rect.y, "the list reaches the columns");
        assert_eq!(
            hit(&vp, &l, x, y),
            Hit::Combo(Dir::Output, combo::Hit::Row(5))
        );
    }

    #[test]
    fn a_greyed_popup_takes_nothing() {
        let mut vp = VolumePopup::new(&master(0, false, false));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 0).track.thumb);
        assert_eq!(vp.press(&l, x, y), Event::None);
        assert!(!vp.dragging());
        let (x, y) = mid(col(&l, 0).mute);
        assert_eq!(vp.press(&l, x, y), Event::None);
        assert_eq!(vp.release(), Event::None);
        assert!(!vp.columns[0].mute);
        assert_eq!(vp.key(&l, form::Key::Up), Event::Redraw);
        assert_eq!(vp.columns[0].volume.pos, 0);
    }

    #[test]
    fn hits_land_on_the_track_the_box_and_outside() {
        let vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let t = &col(&l, 0).track;
        let (x, y) = mid(t.thumb);
        assert_eq!(hit(&vp, &l, x, y), Hit::Track(0, trackbar::Hit::Thumb));
        assert_eq!(
            hit(&vp, &l, x, t.thumb.y - 5),
            Hit::Track(0, trackbar::Hit::Right),
            "above the thumb is louder"
        );
        assert_eq!(
            hit(&vp, &l, x, t.thumb.bottom() + 5),
            Hit::Track(0, trackbar::Hit::Left)
        );
        let (x, y) = mid(col(&l, 0).mute);
        assert_eq!(hit(&vp, &l, x, y), Hit::Mute(0));
        let (x, y) = mid(col(&l, 0).label);
        assert_eq!(hit(&vp, &l, x, y), Hit::Column(0));
        assert_eq!(hit(&vp, &l, 10, 10), Hit::Outside);
    }

    #[test]
    fn a_thumb_drag_follows_the_pointer_up_and_down() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let t = col(&l, 0).track.clone();
        let (x, y) = mid(t.thumb);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert!(vp.dragging());
        assert_eq!(
            vp.motion(&l, x, t.ticks[14]),
            Event::Volume(Channel::Master, 14)
        );
        assert_eq!(
            vp.motion(&l, x + 80, l.panel.y - 50),
            Event::Volume(Channel::Master, STEPS - 1),
            "held off the panel, the thumb still follows"
        );
        vp.sync(&l, &master(3, false, true));
        assert_eq!(
            vp.columns[0].volume.pos,
            STEPS - 1,
            "the held thumb stays put"
        );
        vp.release();
        assert!(!vp.dragging());
        vp.sync(&l, &master(3, false, true));
        assert_eq!(vp.columns[0].volume.pos, 3);
    }

    #[test]
    fn a_thumb_released_after_a_drag_is_hot_under_the_pointer() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        // The host's layout from before the drag: the thumb moves away
        // from where it says.
        let l = lay(&vp);
        let t = col(&l, 0).track.clone();
        let (x, y) = mid(t.thumb);
        vp.press(&l, x, y);
        let to = t.ticks[14];
        assert_eq!(vp.motion(&l, x, to), Event::Volume(Channel::Master, 14));
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Pressed);
        vp.release();
        assert_eq!(
            vp.ptr.of(Part::Thumb(0)),
            Interaction::Hot,
            "the thumb under the pointer is hot"
        );
        // Moved off the thumb where it is now: it goes cold.
        vp.motion(&l, x, y);
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Idle);
    }

    #[test]
    fn a_thumb_moved_by_keys_goes_cold_once_it_leaves_the_pointer() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 0).track.thumb);
        vp.motion(&l, x, y);
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Hot);
        // One position is 6 or 7 px: the 15 px thumb is still under it
        // after one.
        assert_eq!(
            vp.key(&l, form::Key::Up),
            Event::Volume(Channel::Master, 11)
        );
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Hot);
        assert_eq!(
            vp.key(&l, form::Key::Up),
            Event::Volume(Channel::Master, 12)
        );
        assert_eq!(
            vp.ptr.of(Part::Thumb(0)),
            Interaction::Idle,
            "the thumb moved 12 px up, off the pointer"
        );
        assert_eq!(
            vp.key(&l, form::Key::Down),
            Event::Volume(Channel::Master, 11)
        );
        assert_eq!(
            vp.ptr.of(Part::Thumb(0)),
            Interaction::Hot,
            "back under the pointer"
        );
    }

    #[test]
    fn a_thumb_moved_by_the_device_turns_hot_or_cold_under_the_pointer() {
        let theme = theme_for(Interface::Glass);
        let text = TextRenderer::load(&theme.fonts);
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay_with(&vp, &theme, (960, 1052));
        let mut c = Canvas::new(1920, 1080);
        draw(&mut c, &theme, &text, &vp, &l);
        let (x, y) = mid(col(&l, 0).track.thumb);
        assert_eq!(vp.motion(&l, x, y), Event::Redraw);
        assert_eq!(vp.sync(&l, &master(14, false, true)), Event::Redraw);
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Idle);
        assert_eq!(vp.sync(&l, &master(10, false, true)), Event::Redraw);
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Hot);
        assert_eq!(
            vp.sync(&l, &master(10, false, false)),
            Event::Redraw,
            "a greyed thumb lights nothing"
        );
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Idle);
        // Once the pointer left, a moved thumb has nothing to find.
        vp.sync(&l, &master(10, false, true));
        vp.leave();
        assert_eq!(vp.sync(&l, &master(11, false, true)), Event::None);
        assert_eq!(vp.sync(&l, &master(10, false, true)), Event::None);
        assert_eq!(vp.ptr.of(Part::Thumb(0)), Interaction::Idle);
    }

    #[test]
    fn a_press_on_a_stale_layout_takes_the_thumb_where_it_is_now() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        // The layout from before a key moved the thumb up three positions.
        let l = lay(&vp);
        vp.key(&l, form::Key::Up);
        vp.key(&l, form::Key::Up);
        vp.key(&l, form::Key::Up);
        let now = trackbar::at_position(&col(&l, 0).track, 13).thumb;
        let (x, y) = mid(now);
        assert_eq!(vp.press(&l, x, y), Event::Redraw, "held, not stepped");
        assert!(vp.dragging());
        assert_eq!(vp.columns[0].volume.pos, 13);
        vp.release();
        // Where the stale layout's thumb is, the channel below it steps.
        let (x, y) = mid(col(&l, 0).track.thumb);
        assert_eq!(vp.press(&l, x, y), Event::Volume(Channel::Master, 12));
        assert!(!vp.dragging());
    }

    #[test]
    fn a_click_on_the_channel_steps_toward_it() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let t = col(&l, 0).track.clone();
        let x = t.thumb.x + 3;
        assert_eq!(
            vp.press(&l, x, t.thumb.y - 10),
            Event::Volume(Channel::Master, 11)
        );
        vp.release();
        assert_eq!(
            vp.press(&l, x, t.bounds.bottom() - 2),
            Event::Volume(Channel::Master, 10)
        );
    }

    #[test]
    fn a_click_on_mute_flips_it_on_the_release() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 0).mute);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert!(!vp.columns[0].mute, "nothing flips on the press");
        assert_eq!(vp.release(), Event::Mute(Channel::Master, true));
        assert!(vp.columns[0].mute);
        // Dragged off before the release: nothing flips.
        vp.press(&l, x, y);
        vp.motion(&l, 10, 10);
        assert_eq!(vp.release(), Event::None);
        assert!(vp.columns[0].mute);
    }

    #[test]
    fn a_press_outside_closes() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        assert_eq!(vp.press(&l, 10, 10), Event::Close);
    }

    #[test]
    fn keys_move_the_volume_tab_to_mute_and_escape_closes() {
        let mut vp = VolumePopup::new(&master(10, false, true));
        let l = lay(&vp);
        let vol = |p| Event::Volume(Channel::Master, p);
        assert_eq!(vp.key(&l, form::Key::Up), vol(11));
        assert_eq!(vp.key(&l, form::Key::Down), vol(10));
        assert_eq!(vp.key(&l, form::Key::PageUp), vol(14));
        assert_eq!(vp.key(&l, form::Key::End), vol(0));
        assert_eq!(vp.key(&l, form::Key::Home), vol(STEPS - 1));
        assert_eq!(vp.key(&l, form::Key::Space), Event::None);
        assert_eq!(vp.key(&l, form::Key::Tab { back: false }), Event::Redraw);
        assert_eq!(vp.focus, Focus::Mute(0));
        assert_eq!(
            vp.key(&l, form::Key::Space),
            Event::Mute(Channel::Master, true)
        );
        assert_eq!(vp.key(&l, form::Key::Up), Event::None, "Mute has the keys");
        assert_eq!(vp.key(&l, form::Key::Escape), Event::Close);
    }

    #[test]
    fn the_first_key_shows_the_focus_ring() {
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let mut vp = VolumePopup::new(&master(STEPS - 1, false, true));
        let l = lay(&vp);
        let t = col(&l, 0).track.clone();
        let ring_px = |vp: &VolumePopup| {
            let mut c = Canvas::new(1920, 1080);
            draw(&mut c, &theme, &text, vp, &l);
            c.get(t.bounds.x, t.bounds.bottom() - 1)
        };
        assert_eq!(ring_px(&vp), theme.colors.button_face.argb());
        assert_eq!(
            vp.key(&l, form::Key::Up),
            Event::Redraw,
            "at the top already, but the ring shows"
        );
        assert_eq!(ring_px(&vp), theme.colors.button_text.argb());
    }

    #[test]
    fn access_names_the_slider_by_its_label() {
        let vp = VolumePopup::new(&master(10, true, true));
        let l = lay(&vp);
        let (mut tree, focus) = accessed(&vp, &l);
        let slider = tree.get_mut(focus).expect("the slider has the focus");
        assert_eq!(slider.role(), Role::Slider);
        assert_eq!(slider.numeric_value(), Some(10.0));
        assert_eq!(slider.orientation(), Some(accesskit::Orientation::Vertical));
        assert_eq!(slider.labelled_by().len(), 1);
    }

    #[test]
    fn sync_leaves_a_held_thumb_where_the_drag_put_it() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).track.thumb);
        vp.press(&l, x, y);
        vp.sync(&l, &mix(&[app("vlc", 1), app("mpv", 3)]));
        assert_eq!(
            vp.ptr.held(),
            Some(Part::Thumb(2)),
            "the drag moves with mpv"
        );
        assert_eq!(vp.columns[2].volume.pos, 8);
    }

    #[test]
    fn sync_ends_a_drag_on_a_program_that_left() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).track.thumb);
        vp.press(&l, x, y);
        vp.sync(&l, &mix(&[]));
        assert_eq!(vp.ptr.held(), None);
    }

    #[test]
    fn a_program_thumb_drag_writes_its_channel() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let c = col(&l, 1).clone();
        let (x, y) = mid(c.track.thumb);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert_eq!(
            vp.motion(&l, x, c.track.ticks[14]),
            Event::Volume(Channel::App("mpv".into()), 14)
        );
        assert_eq!(vp.focus, Focus::Thumb(1));
    }

    #[test]
    fn a_program_mute_flips_on_release() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).mute);
        vp.press(&l, x, y);
        assert_eq!(vp.release(), Event::Mute(Channel::App("mpv".into()), true));
    }

    #[test]
    fn a_program_column_takes_the_pointer_under_a_greyed_master() {
        let mut m = mix(&[app("mpv", 8)]);
        m.master.live = false;
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        let t = col(&l, 1).track.clone();
        assert_eq!(
            vp.press(&l, t.thumb.x + 3, t.thumb.y - 10),
            Event::Volume(Channel::App("mpv".into()), 9)
        );
        assert_eq!(vp.focus, Focus::Thumb(1));
    }

    #[test]
    fn the_wheel_over_a_column_steps_that_column() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).label);
        assert_eq!(
            vp.scroll(&l, x, y, -WHEEL_ROWS),
            Event::Volume(Channel::App("mpv".into()), 9),
            "one notch up is one position louder"
        );
        assert_eq!(
            vp.scroll(&l, x, y, -2 * WHEEL_ROWS),
            Event::Volume(Channel::App("mpv".into()), 11)
        );
        assert_eq!(vp.columns[0].volume.pos, 10, "master unchanged");
        assert_eq!(vp.focus, Focus::Thumb(0), "the focus stays");
    }

    #[test]
    fn the_wheel_elsewhere_steps_master() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(l.separator);
        assert_eq!(
            vp.scroll(&l, x, y, WHEEL_ROWS),
            Event::Volume(Channel::Master, 9)
        );
        assert_eq!(
            vp.scroll(&l, 5, 5, -3 * WHEEL_ROWS),
            Event::Volume(Channel::Master, 12),
            "outside the panel too"
        );
    }

    #[test]
    fn the_wheel_over_masters_column_steps_master() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 0).track.bounds);
        assert_eq!(
            vp.scroll(&l, x, y, -WHEEL_ROWS),
            Event::Volume(Channel::Master, 11)
        );
    }

    #[test]
    fn the_wheel_over_a_closed_device_combo_steps_master_and_keeps_the_device() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (x, y) = mid(l.output.field);
        assert_eq!(
            vp.scroll(&l, x, y, -WHEEL_ROWS),
            Event::Volume(Channel::Master, 11)
        );
        assert_eq!(vp.output.selected, 0, "the default output stays");
        let (x, y) = mid(l.input.field);
        assert_eq!(
            vp.scroll(&l, x, y, WHEEL_ROWS),
            Event::Volume(Channel::Master, 10)
        );
        assert_eq!(vp.input.selected, 0, "the default input stays");
    }

    #[test]
    fn a_turn_short_of_a_notch_still_steps_one_position() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        assert_eq!(vp.scroll(&l, 5, 5, 1), Event::Volume(Channel::Master, 9));
        assert_eq!(vp.scroll(&l, 5, 5, -1), Event::Volume(Channel::Master, 10));
    }

    #[test]
    fn a_wheel_burst_clamps_at_either_end() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        assert_eq!(
            vp.scroll(&l, 5, 5, -11 * WHEEL_ROWS),
            Event::Volume(Channel::Master, STEPS - 1),
            "11 notches from 10 clamp at 20"
        );
        assert_eq!(vp.scroll(&l, 5, 5, -WHEEL_ROWS), Event::None);
        assert_eq!(
            vp.scroll(&l, 5, 5, 99 * WHEEL_ROWS),
            Event::Volume(Channel::Master, 0)
        );
    }

    #[test]
    fn the_wheel_does_nothing_to_a_greyed_master() {
        let mut m = mix(&[]);
        m.master.live = false;
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        assert_eq!(vp.scroll(&l, 5, 5, -1), Event::None);
    }

    #[test]
    fn tab_walks_the_combos_then_each_column() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        vp.focus = Focus::Output;
        let order: Vec<Focus> = (0..6)
            .map(|_| {
                vp.key(&l, form::Key::Tab { back: false });
                vp.focus
            })
            .collect();
        assert_eq!(
            order,
            [
                Focus::Input,
                Focus::Thumb(0),
                Focus::Mute(0),
                Focus::Thumb(1),
                Focus::Mute(1),
                Focus::Output
            ]
        );
        vp.key(&l, form::Key::Tab { back: true });
        assert_eq!(vp.focus, Focus::Mute(1), "Shift+Tab walks back");
    }

    #[test]
    fn tab_passes_over_what_is_greyed() {
        let mut m = mix(&[app("mpv", 8)]);
        m.master.live = false;
        m.inputs.clear();
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        vp.focus = Focus::Output;
        vp.key(&l, form::Key::Tab { back: false });
        assert_eq!(vp.focus, Focus::Thumb(1));
    }

    #[test]
    fn keys_act_on_the_focused_program() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let mpv = || Channel::App("mpv".into());
        vp.focus = Focus::Thumb(1);
        assert_eq!(vp.key(&l, form::Key::Up), Event::Volume(mpv(), 9));
        assert_eq!(vp.key(&l, form::Key::PageDown), Event::Volume(mpv(), 5));
        vp.key(&l, form::Key::Tab { back: false });
        assert_eq!(vp.key(&l, form::Key::Space), Event::Mute(mpv(), true));
        assert_eq!(vp.columns[0].volume.pos, 10, "master unchanged");
        vp.focus = Focus::Output;
        assert_eq!(vp.key(&l, form::Key::Up), Event::None);
    }

    #[test]
    fn focusing_a_hidden_program_scrolls_it_into_view() {
        let apps: Vec<_> = (0..8).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        vp.focus = Focus::Mute(6);
        vp.key(&l, form::Key::Tab { back: false });
        assert_eq!(vp.focus, Focus::Thumb(7));
        assert_eq!(vp.first, 1);
        vp.focus = Focus::Thumb(0);
        vp.key(&l, form::Key::Tab { back: false });
        vp.key(&l, form::Key::Tab { back: false });
        assert_eq!(vp.focus, Focus::Thumb(1));
        assert_eq!(vp.first, 0, "back to the first program");
        vp.first = 2;
        assert_eq!(
            vp.key(&l, form::Key::Up),
            Event::Volume(Channel::App("p0".into()), 11)
        );
        assert_eq!(vp.first, 0, "a key on a hidden program shows it");
    }

    #[test]
    fn the_bar_arrows_scroll_the_programs() {
        let apps: Vec<_> = (0..8).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        let bar = l.bar.unwrap();
        assert_eq!(vp.press(&l, bar.right() - 4, bar.y + 4), Event::Redraw);
        vp.release();
        assert_eq!(vp.first, 1);
        let l = lay(&vp);
        assert!(l.columns[1].is_none() && l.columns[7].is_some() && l.columns[8].is_none());
        vp.press(&l, bar.right() - 4, bar.y + 4);
        vp.release();
        vp.press(&l, bar.right() - 4, bar.y + 4);
        assert_eq!(vp.first, 2, "clamped at the last page");
        vp.release();
        vp.press(&l, bar.x + 4, bar.y + 4);
        assert_eq!(vp.first, 1, "the left arrow scrolls back");
    }

    #[test]
    fn the_wheel_over_the_bar_scrolls_the_programs() {
        let apps: Vec<_> = (0..8).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        let (x, y) = mid(l.bar.unwrap());
        assert_eq!(vp.scroll(&l, x, y, WHEEL_ROWS), Event::Redraw);
        assert_eq!(vp.first, 1, "one notch down is one column on");
        assert_eq!(vp.columns[0].volume.pos, 10, "master unchanged");
        assert_eq!(vp.scroll(&l, x, y, 3 * WHEEL_ROWS), Event::Redraw);
        assert_eq!(vp.first, 2, "clamped at the last page");
        assert_eq!(vp.scroll(&l, x, y, WHEEL_ROWS), Event::None);
        assert_eq!(vp.scroll(&l, x, y, -WHEEL_ROWS), Event::Redraw);
        assert_eq!(vp.first, 1);
        assert_eq!(vp.scroll(&l, x, y, -5 * WHEEL_ROWS), Event::Redraw);
        assert_eq!(vp.first, 0);
        assert_eq!(vp.scroll(&l, x, y, -WHEEL_ROWS), Event::None);
    }

    #[test]
    fn the_bar_track_pages_and_the_thumb_drags() {
        let apps: Vec<_> = (0..14).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        let bar = l.bar.unwrap();
        vp.press(&l, bar.right() - 20, bar.y + 4);
        vp.release();
        assert_eq!(vp.first, 6, "a page is six columns");
        vp.press(&l, bar.right() - 20, bar.y + 4);
        vp.release();
        assert_eq!(vp.first, 8, "clamped at the last page");
        let l = lay(&vp);
        let thumb = bar_parts(&vp, &l, bar).thumb.unwrap();
        let (x, y) = mid(thumb);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert_eq!(vp.ptr.held(), Some(Part::Bar(ScrollHit::Thumb)));
        assert_eq!(vp.motion(&l, x - 1000, y + 60), Event::Redraw);
        assert_eq!(vp.first, 0, "held off the bar, the thumb still follows");
        assert_eq!(vp.motion(&l, x - 1000, y), Event::None, "nothing moved");
        vp.release();
        assert_eq!(vp.ptr.held(), None);
    }

    #[test]
    fn picking_a_listed_output_sets_the_default() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert!(vp.output.open.is_some());
        vp.release();
        let l = lay(&vp);
        let row = l.output.list.y + 1 + l.output.row_h + 2;
        assert_eq!(
            vp.press(&l, l.output.list.x + 10, row),
            Event::Default(Dir::Output, 1)
        );
        assert!(vp.output.open.is_none());
    }

    #[test]
    fn a_press_off_an_open_list_only_closes_it() {
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8)]));
        let l = lay(&vp);
        let (x, y) = mid(l.input.drop);
        vp.press(&l, x, y);
        vp.release();
        let l = lay(&vp);
        let (x, y) = mid(col(&l, 1).mute);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert!(!matches!(vp.release(), Event::Mute(..)));
        assert!(vp.input.open.is_none());
        assert!(!vp.columns[1].mute, "the Mute under it was not flipped");
        assert_eq!(
            vp.press(&l, 5, 5),
            Event::Close,
            "a second press outside closes"
        );
    }

    #[test]
    fn opening_one_list_closes_the_other() {
        // The Input list drops clear of the Output row; the Output list
        // would lie over the Input row.
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (x, y) = mid(l.input.drop);
        vp.press(&l, x, y);
        vp.release();
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        vp.release();
        assert!(vp.input.open.is_none(), "the first press closes Input");
        assert!(vp.output.open.is_none(), "and does nothing else");
        let l = lay(&vp);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        assert!(vp.output.open.is_some());
        assert_eq!(vp.focus, Focus::Output);
    }

    #[test]
    fn the_list_arrows_and_track_scroll_it_and_its_thumb_stays() {
        let mut m = mix(&[]);
        m.outputs = (0..12).map(|i| format!("Sink {i}")).collect();
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        vp.press(&l, x, y);
        vp.release();
        let l = lay(&vp);
        let sb = l.output.scrollbar.expect("12 devices outgrow 8 rows");
        let parts = |top| scrollbar_parts(sb, 1, top, 8, 12, false, l.output.thumb_min);
        let (x, y) = mid(parts(0).forward);
        assert_eq!(vp.press(&l, x, y), Event::Redraw);
        vp.release();
        assert_eq!(vp.output.top, 1, "the down arrow scrolls a row");
        let under = parts(1).thumb.unwrap().bottom() + 2;
        assert_eq!(vp.press(&l, x, under), Event::Redraw);
        vp.release();
        assert_eq!(vp.output.top, 4, "the track pages, clamped at the end");
        assert_eq!(vp.press(&l, x, y), Event::None, "already at the end");
        vp.release();
        let (x, y) = mid(parts(4).thumb.unwrap());
        assert_eq!(vp.press(&l, x, y), Event::None);
        assert_eq!(vp.motion(&l, x, y - 40), Event::None, "no drag");
        vp.release();
        assert_eq!(vp.output.top, 4);
        assert!(vp.output.open.is_some(), "the list stays open");
    }

    #[test]
    fn down_on_a_closed_output_picks_the_next_device() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        vp.focus = Focus::Output;
        assert_eq!(vp.key(&l, form::Key::Down), Event::Default(Dir::Output, 1));
        assert_eq!(vp.key(&l, form::Key::Down), Event::None, "already last");
        assert_eq!(vp.key(&l, form::Key::Home), Event::Default(Dir::Output, 0));
    }

    #[test]
    fn keys_on_an_open_list_move_the_highlight_and_enter_takes_it() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        vp.focus = Focus::Output;
        assert_eq!(vp.key(&l, form::Key::Drop), Event::Redraw);
        assert_eq!(vp.output.open, Some(0));
        assert_eq!(vp.key(&l, form::Key::Down), Event::Redraw);
        assert_eq!(vp.output.open, Some(1));
        assert_eq!(vp.output.selected, 0, "the highlight moves, not the pick");
        assert_eq!(vp.key(&l, form::Key::Enter), Event::Default(Dir::Output, 1));
        assert!(vp.output.open.is_none());
        vp.key(&l, form::Key::Drop);
        assert_eq!(vp.key(&l, form::Key::Enter), Event::Redraw, "no change");
        assert!(vp.output.open.is_none());
    }

    #[test]
    fn escape_closes_an_open_list_before_the_popup() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        vp.press(&l, x, y);
        vp.release();
        assert_eq!(vp.key(&l, form::Key::Escape), Event::Redraw);
        assert_eq!(vp.key(&l, form::Key::Escape), Event::Close);
    }

    #[test]
    fn the_wheel_over_an_open_list_scrolls_it() {
        let mut m = mix(&[]);
        m.outputs = (0..12).map(|i| format!("Sink {i}")).collect();
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        vp.press(&l, x, y);
        vp.release();
        let l = lay(&vp);
        assert!(l.output.scrollbar.is_some(), "12 devices outgrow 8 rows");
        let (x, y) = mid(l.output.list);
        assert_eq!(vp.scroll(&l, x, y, 2), Event::Redraw);
        assert_eq!(vp.output.top, 2);
        assert_eq!(vp.columns[0].volume.pos, 10, "master unchanged");
    }

    #[test]
    fn a_device_list_change_closes_the_open_list() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        vp.press(&l, x, y);
        vp.release();
        let mut m = mix(&[]);
        m.outputs = vec!["Speakers".into()];
        m.output = Some(0);
        vp.sync(&l, &m);
        assert!(vp.output.open.is_none());
        assert_eq!(vp.output.selected, 0);
        assert_eq!(vp.output.items.len(), 1);
    }

    #[test]
    fn no_devices_greys_the_row() {
        let mut m = mix(&[]);
        m.inputs.clear();
        m.input = None;
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        assert!(!vp.input.enabled);
        let (x, y) = mid(l.input.drop);
        assert_eq!(vp.press(&l, x, y), Event::None);
        assert!(vp.input.open.is_none());
        vp.focus = Focus::Input;
        assert_eq!(vp.key(&l, form::Key::Drop), Event::Redraw, "the first key");
        assert_eq!(vp.key(&l, form::Key::Drop), Event::None);
        assert!(vp.input.open.is_none());
    }

    #[test]
    fn sync_ends_a_press_on_a_rebuilt_combo_or_a_bar_that_went() {
        let apps: Vec<_> = (0..7).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        let l = lay(&vp);
        vp.ptr.press(Some(Part::Bar(ScrollHit::Forward)), false);
        vp.sync(&l, &mix(&apps[..7]));
        assert_eq!(
            vp.ptr.held(),
            Some(Part::Bar(ScrollHit::Forward)),
            "seven programs keep the bar and its press"
        );
        vp.sync(&l, &mix(&apps[..6]));
        assert_eq!(vp.ptr.lit(), None, "six programs: the bar went");
        vp.ptr.press(Some(Part::Combo(Dir::Output)), false);
        vp.sync(&l, &mix(&apps[..5]));
        assert_eq!(
            vp.ptr.held(),
            Some(Part::Combo(Dir::Output)),
            "the same devices keep the press"
        );
        let mut m = mix(&apps[..5]);
        m.outputs.pop();
        vp.sync(&l, &m);
        assert_eq!(vp.ptr.lit(), None, "the outputs changed: the press ends");
    }

    fn drawn(theme: &Theme, vp: &VolumePopup) -> (Canvas, Layout) {
        let text = TextRenderer::load(&theme.fonts);
        let l = lay_with(vp, theme, (960, 1052));
        let mut c = Canvas::new(1920, 1080);
        draw(&mut c, theme, &text, vp, &l);
        (c, l)
    }

    fn inked(c: &Canvas, r: Rect, ink: u32) -> usize {
        (r.x..r.right())
            .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    }

    /// With a second dialog font of 9 px a glyph loaded, "Output:" sizes the label column at 63 px and master's "Volume" inks six pens from 21 px in.
    #[test]
    fn the_text_is_set_in_the_second_dialog_font() {
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let text = TextRenderer::load(&theme.fonts).with_shell_dlg(TextRenderer::stub(px, |_| 9));
        let vp = VolumePopup::new(&mix(&[]));
        let l = layout(
            &vp,
            &theme,
            &text,
            (960, 1052),
            Edge::Bottom,
            Rect::new(0, 0, 1920, 1080),
        );
        assert_eq!(l.output_label.width, 63, "Output: in the second font");
        let mut c = Canvas::new(1920, 1080);
        draw(&mut c, &theme, &text, &vp, &l);
        let name = col(&l, 0).label;
        let ink = theme.colors.button_text.argb();
        let pens: Vec<i32> = (name.x..name.right())
            .filter(|&x| (name.y..name.bottom()).any(|y| c.get(x, y) == ink))
            .map(|x| x - name.x)
            .collect();
        assert_eq!(pens, [21, 30, 39, 48, 57, 66], "Volume's six pens");
    }

    #[test]
    fn draws_the_edge_rows_separator_and_columns() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let vp = VolumePopup::new(&mix(&[app("firefox", 16), app("mpv", 8)]));
        let (c, l) = drawn(&theme, &vp);
        let p = l.panel;
        assert_eq!(c.get(p.x + 10, p.y + 1), cs.button_hilight.argb());
        assert_eq!(c.get(p.right() - 1, p.y + 50), cs.button_dk_shadow.argb());
        let sep = l.separator;
        assert_eq!(c.get(sep.x + 5, sep.y), cs.button_shadow.argb());
        assert_eq!(c.get(sep.x + 5, sep.y + 1), cs.button_hilight.argb());
        for i in 0..3 {
            let t = &col(&l, i).track;
            assert_eq!(
                c.get(t.channel.x, t.channel.y + 2),
                cs.button_shadow.argb(),
                "column {i} channel"
            );
            assert!(
                inked(&c, col(&l, i).mute, cs.button_text.argb()) > 0,
                "column {i} draws its speaker"
            );
        }
        assert_eq!(
            c.get(l.output.field.x, l.output.field.y),
            cs.button_shadow.argb(),
            "combo frame"
        );
        let label = |r: Rect| inked(&c, r, cs.button_text.argb());
        assert!(label(l.output_label) > 0, "Output: is drawn");
        assert!(label(l.input_label) > 0, "Input: is drawn");
        assert!(
            label(l.output.text) > 0,
            "the picked output's name is drawn"
        );
    }

    #[test]
    fn a_program_icon_draws_centred_over_its_name() {
        let theme = Theme::default();
        let mut a = app("mpv", 8);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(0, 0, 32, 32), theme.colors.hilight);
        a.icon = Some(icon);
        let vp = VolumePopup::new(&mix(&[a]));
        let (c, l) = drawn(&theme, &vp);
        let r = col(&l, 1).icon.expect("the program has an icon");
        assert_eq!(c.get(r.x, r.y), theme.colors.hilight.argb());
        assert_eq!(
            c.get(r.right() - 1, r.bottom() - 1),
            theme.colors.hilight.argb()
        );
        assert_eq!(c.get(r.x - 1, r.y), theme.colors.button_face.argb());
    }

    #[test]
    fn a_long_program_name_is_cut_to_its_column() {
        let theme = Theme::default();
        let ink = theme.colors.button_text.argb();
        let mut a = app("ff", 8);
        a.label = "Firefox Developer Edition playing a very long title".into();
        let vp = VolumePopup::new(&mix(&[a, app("mpv", 8)]));
        let (c, l) = drawn(&theme, &vp);
        let next = col(&l, 2).rect;
        let label = col(&l, 1).label;
        let spill = (label.y..label.bottom())
            .filter(|&y| c.get(next.x + 1, y) == ink)
            .count();
        assert_eq!(spill, 0, "no text spills into the next column");
        let gutter = NAME_PAD;
        let left = Rect::new(label.x, label.y, gutter, label.height);
        assert_eq!(inked(&c, left, ink), 0, "clear on the left");
        let right = Rect::new(label.right() - gutter, label.y, gutter, label.height);
        assert_eq!(inked(&c, right, ink), 0, "clear on the right");
        assert!(inked(&c, label, ink) > 0, "the cut name is drawn");
    }

    #[test]
    fn seven_programs_scrolled_by_one_draw_the_scrollbar_under_them() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let apps: Vec<_> = (0..7).map(|i| app(&format!("p{i}"), 10)).collect();
        let mut vp = VolumePopup::new(&mix(&apps));
        vp.first = 1;
        let (c, l) = drawn(&theme, &vp);
        let bar = l.bar.expect("seven programs scroll");
        // The left arrow button's raised face.
        assert_eq!(c.get(bar.x + 1, bar.y + 1), cs.button_hilight.argb());
        let thumb = bar_parts(&vp, &l, bar).thumb.expect("the bar has a thumb");
        assert!(
            thumb.x > bar.x + 16,
            "scrolled by one, the thumb moved right"
        );
        assert_eq!(
            c.get(thumb.right() - 1, thumb.bottom() - 1),
            cs.button_dk_shadow.argb(),
            "the thumb's raised edge"
        );
        assert_ne!(
            c.get(bar.x + 16, bar.bottom() - 1),
            cs.button_dk_shadow.argb(),
            "no thumb at the track's left end"
        );
        assert!(l.columns[1].is_none(), "p0 scrolled out of view");
        let t = &col(&l, 6).track;
        assert_eq!(
            c.get(t.channel.x, t.channel.y + 14),
            cs.button_shadow.argb(),
            "p5 draws in the last slot"
        );
    }

    #[test]
    fn a_focused_program_thumb_draws_the_ring_once_keys_are_used() {
        let theme = Theme::default();
        let mut vp = VolumePopup::new(&mix(&[app("firefox", 20), app("mpv", 20)]));
        vp.focus = Focus::Thumb(2);
        let t = col(&lay(&vp), 2).track.clone();
        let corner = |vp: &VolumePopup| {
            let (c, _) = drawn(&theme, vp);
            c.get(t.bounds.x, t.bounds.bottom() - 1)
        };
        assert_eq!(corner(&vp), theme.colors.button_face.argb());
        vp.cues = true;
        assert_eq!(corner(&vp), theme.colors.button_text.argb());
    }

    #[test]
    fn an_open_list_draws_over_the_columns() {
        let theme = Theme::default();
        let mut m = mix(&[app("mpv", 8)]);
        m.outputs = (0..6).map(|i| format!("Output {i}")).collect();
        let mut vp = VolumePopup::new(&m);
        let l = lay(&vp);
        let (x, y) = mid(l.output.drop);
        vp.press(&l, x, y);
        vp.release();
        assert!(vp.output.open.is_some(), "the press opened the list");
        let (c, l) = drawn(&theme, &vp);
        let list = l.output.list;
        assert!(
            list.bottom() > l.separator.y,
            "the list covers the separator"
        );
        assert_eq!(
            c.get(list.x, list.y + 5),
            theme.colors.button_text.argb(),
            "the list's 1 px frame"
        );
        assert_eq!(
            c.get(list.right() - 4, l.separator.y),
            theme.colors.window.argb(),
            "the list hides the separator"
        );
    }

    #[test]
    fn greyed_master_and_empty_input_draw_greyed() {
        let theme = Theme::default();
        let mut m = mix(&[]);
        m.master.live = false;
        m.inputs.clear();
        m.input = None;
        let vp = VolumePopup::new(&m);
        let (c, l) = drawn(&theme, &vp);
        let f = l.input.field;
        assert_eq!(
            c.get(f.x + 4, f.y + 4),
            theme.colors.button_face.argb(),
            "a greyed field is face coloured"
        );
        let ink = theme.colors.button_text.argb();
        assert_eq!(inked(&c, l.input_label, ink), 0, "Input: greys");
        assert!(inked(&c, l.output_label, ink) > 0, "Output: stays black");
        assert_eq!(
            inked(&c, col(&l, 0).label, ink),
            0,
            "a greyed Volume has no black ink"
        );
    }

    #[test]
    fn no_devices_draws_both_rows_greyed_and_empty() {
        let theme = Theme::default();
        let face = theme.colors.button_face.argb();
        let mut m = mix(&[]);
        m.outputs.clear();
        m.output = None;
        m.inputs.clear();
        m.input = None;
        let vp = VolumePopup::new(&m);
        let (c, l) = drawn(&theme, &vp);
        let ink = theme.colors.button_text.argb();
        for (name, label, f) in [
            ("Output", l.output_label, &l.output),
            ("Input", l.input_label, &l.input),
        ] {
            assert_eq!(inked(&c, label, ink), 0, "{name}: its label greys");
            assert_eq!(
                inked(&c, f.text, face),
                (f.text.width * f.text.height) as usize,
                "{name}: the field is face coloured and empty"
            );
        }
        let (tree, _) = accessed(&vp, &l);
        for name in ["Output", "Input"] {
            let id = tree.find(Role::ComboBox, name).expect("the combo");
            assert!(
                tree.get(id).is_some_and(|n| n.is_disabled()),
                "{name}: disabled"
            );
        }
        assert!(
            tree.nodes().all(|(_, n)| n.role() != Role::ListBoxOption),
            "no device rows"
        );
    }

    #[test]
    fn blue_and_glass_draw_the_same_parts_in_their_own_art() {
        let classic = Theme::default();
        let vp = VolumePopup::new(&mix(&[app("firefox", 16), app("mpv", 8)]));
        let (cc, cl) = drawn(&classic, &vp);
        for look in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(look);
            let (c, l) = drawn(&theme, &vp);
            // Glass frames the panel as a window round white, 12 px wider
            // and taller about the same anchor: its columns stand 6 px
            // higher, where Blue's stand as Classic's.
            let (fill, up) = if look == Interface::Glass {
                (theme.colors.window, 6)
            } else {
                (theme.colors.button_face, 0)
            };
            assert_eq!(
                c.get(l.panel.x + 10, l.panel.y + 10),
                fill.argb(),
                "{look:?}: the panel is filled"
            );
            assert_eq!(
                l.columns[1].as_ref().map(|c| c.rect),
                cl.columns[1].as_ref().map(|c| Rect::new(
                    c.rect.x,
                    c.rect.y - up,
                    c.rect.width,
                    c.rect.height
                )),
                "{look:?}: the same geometry as Classic"
            );
            let face = theme.colors.button_face.argb();
            let thumb = |c: &Canvas, i: usize| {
                let t = col(&l, i).track.thumb;
                (t.x..t.right())
                    .flat_map(|x| (t.y..t.bottom()).map(move |y| (x, y)))
                    .map(|(x, y)| c.get(x, y))
                    .collect::<Vec<_>>()
            };
            for i in 0..3 {
                let own = thumb(&c, i);
                assert!(
                    own.iter().any(|&p| p != face),
                    "{look:?}: column {i}'s thumb draws"
                );
                assert_ne!(
                    own,
                    thumb(&cc, i),
                    "{look:?}: column {i}'s thumb wears its own art"
                );
            }
            let f = l.output.field;
            let row = |c: &Canvas| {
                (f.x..f.right())
                    .map(|x| c.get(x, f.y + 1))
                    .collect::<Vec<_>>()
            };
            assert!(
                row(&c).iter().any(|&p| p != face),
                "{look:?}: the combo draws"
            );
            assert_ne!(row(&c), row(&cc), "{look:?}: the combo wears its own art");
        }
    }

    fn accessed(vp: &VolumePopup, l: &Layout) -> (Tree, NodeId) {
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let mut tree = Tree::window("", (1920, 1080));
        let focus = vp.access(l, &text, theme.fonts.size_px, &mut tree, access::ROOT);
        (tree, focus)
    }

    #[test]
    fn an_open_device_list_has_its_rows_and_the_focus_on_the_highlighted_one() {
        let mut vp = VolumePopup::new(&mix(&[]));
        let l = lay(&vp);
        let (tree, _) = accessed(&vp, &l);
        let output = tree
            .find(Role::ComboBox, "Output")
            .expect("the Output combo");
        let rows = |tree: &Tree| {
            tree.nodes()
                .filter(|(_, n)| n.role() == Role::ListBoxOption)
                .map(|(_, n)| (n.label().unwrap_or("").to_string(), n.is_selected()))
                .collect::<Vec<_>>()
        };
        let got = rows(&tree);
        assert!(
            got.contains(&("Speakers".into(), Some(true))),
            "the picked output is selected: {got:?}"
        );
        assert!(got.contains(&("Headphones".into(), Some(false))));
        assert!(got.contains(&("Microphone".into(), Some(true))));
        assert_eq!(tree.get(output).and_then(|n| n.is_expanded()), Some(false));
        vp.focus = Focus::Output;
        vp.key(&l, form::Key::Drop);
        vp.key(&l, form::Key::Down);
        let (tree, focus) = accessed(&vp, &l);
        let row = tree.get(focus).expect("the focused node");
        assert_eq!(row.role(), Role::ListBoxOption);
        assert_eq!(
            row.label(),
            Some("Headphones"),
            "the highlighted row has the focus"
        );
        let output = tree
            .find(Role::ComboBox, "Output")
            .expect("the Output combo");
        assert_eq!(tree.get(output).and_then(|n| n.is_expanded()), Some(true));
    }

    #[test]
    fn access_lists_the_choosers_and_each_column() {
        let mut a = app("tj", 8);
        a.label = "Tom & Jerry".into();
        let mut vp = VolumePopup::new(&mix(&[app("mpv", 8), a]));
        let l = lay(&vp);
        let (tree, focus) = accessed(&vp, &l);
        let role = |tree: &Tree, id| tree.get(id).map(|n| n.role());
        assert_eq!(role(&tree, focus), Some(Role::Slider));
        assert!(tree.find(Role::ComboBox, "Output").is_some());
        assert!(tree.find(Role::ComboBox, "Input").is_some());
        for name in [VOLUME, "mpv", "Tom & Jerry"] {
            let g = tree.find(Role::Group, name).expect("a group per column");
            let kids = tree
                .get(g)
                .map(|n| n.children().to_vec())
                .unwrap_or_default();
            let roles: Vec<_> = kids.iter().map(|&k| role(&tree, k)).collect();
            assert!(roles.contains(&Some(Role::Slider)), "{name}: a slider");
            assert!(roles.contains(&Some(Role::Button)), "{name}: Mute");
        }
        vp.focus = Focus::Input;
        let (tree, focus) = accessed(&vp, &l);
        assert_eq!(Some(focus), tree.find(Role::ComboBox, "Input"));
        vp.focus = Focus::Mute(2);
        let (tree, focus) = accessed(&vp, &l);
        assert_eq!(role(&tree, focus), Some(Role::Button));
        assert_eq!(
            tree.get(focus).map(|n| n.bounds()),
            Some(Some(access::bounds(col(&l, 2).mute))),
            "the focus is on the second program's Mute"
        );
    }

    /// Writes target/volumepopup-preview.png: Classic, Blue and Glass states.
    #[test]
    #[ignore]
    fn render_volumepopup() {
        let (w, h) = (480, 420);
        let mut c = Canvas::new(4 * w as u32, 2 * h as u32);
        let classic = Theme::default();
        let blue = theme_for(Interface::Blue);
        let glass = theme_for(Interface::Glass);
        let two = || {
            let mut a = app("firefox", 16);
            a.label = "Firefox".into();
            let mut b = app("mpv", 8);
            b.mute = true;
            mix(&[a, b])
        };
        let mut focused = VolumePopup::new(&two());
        focused.focus = Focus::Thumb(2);
        focused.cues = true;
        let mut open = VolumePopup::new(&two());
        open.output.toggle();
        let mut greyed = two();
        greyed.master.live = false;
        greyed.outputs.clear();
        greyed.inputs.clear();
        let apps: Vec<_> = (0..7)
            .map(|i| app(&format!("Program {i}"), 3 * i))
            .collect();
        let mut seven = VolumePopup::new(&mix(&apps));
        seven.first = 1;
        let cases: [(&Theme, VolumePopup, Rect); 6] = [
            (&classic, focused, Rect::new(0, 0, w, h)),
            (&classic, open, Rect::new(w, 0, w, h)),
            (
                &classic,
                VolumePopup::new(&greyed),
                Rect::new(2 * w, 0, w, h),
            ),
            (&classic, seven, Rect::new(0, h, 2 * w, h)),
            (&blue, VolumePopup::new(&two()), Rect::new(2 * w, h, w, h)),
            (&glass, VolumePopup::new(&two()), Rect::new(3 * w, h, w, h)),
        ];
        for (theme, vp, r) in &cases {
            let text = TextRenderer::load(&theme.fonts);
            let anchor = (r.x + r.width / 2, r.bottom() - 4);
            let l = layout(vp, theme, &text, anchor, Edge::Bottom, *r);
            draw(&mut c, theme, &text, vp, &l);
        }
        crate::raster::write_preview(&c, "volumepopup-preview.png");
    }

    fn track(playing: bool) -> NowPlaying {
        NowPlaying {
            title: "Evening Mix".into(),
            artist: "The Band".into(),
            playing,
            can_previous: true,
            can_next: true,
            can_play: true,
            can_pause: true,
        }
    }

    fn with_track(apps: &[AppLevel], np: NowPlaying) -> Mix {
        Mix {
            playing: Some(np),
            ..mix(apps)
        }
    }

    #[test]
    fn the_middle_button_pauses_a_playing_track_and_plays_a_paused_one() {
        let mut np = track(true);
        np.can_play = false;
        assert_eq!(np.glyph(Transport::PlayPause), (MediaGlyph::Pause, "Pause"));
        assert!(np.can(Transport::PlayPause), "Pause follows CanPause");
        np.playing = false;
        assert_eq!(np.glyph(Transport::PlayPause), (MediaGlyph::Play, "Play"));
        assert!(!np.can(Transport::PlayPause), "Play follows CanPlay");
        np.can_previous = false;
        assert!(!np.can(Transport::Previous) && np.can(Transport::Next));
    }

    #[test]
    fn a_player_adds_the_strip_under_the_columns() {
        let vp = VolumePopup::new(&with_track(&[], track(true)));
        let l = lay(&vp);
        assert_eq!(l.panel, Rect::new(738, 655, 444, 397), "56 px taller");
        let st = l.strip.as_ref().expect("a strip");
        let foot = col(&l, 0).rect.bottom();
        assert_eq!(st.separator, Rect::new(750, foot + 10, 420, 2));
        assert_eq!(st.rect, Rect::new(750, foot + 22, 420, 34));
        assert_eq!(
            st.buttons,
            [
                Rect::new(1066, foot + 25, 32, 28),
                Rect::new(1102, foot + 25, 32, 28),
                Rect::new(1138, foot + 25, 32, 28),
            ]
        );
        assert_eq!(st.title, Rect::new(750, foot + 26, 308, 13));
        assert_eq!(st.artist, Some(Rect::new(750, foot + 39, 308, 13)));
        assert_eq!(st.rect.bottom() + MARGIN, l.panel.bottom());
        let mut np = track(true);
        np.artist.clear();
        let vp = VolumePopup::new(&with_track(&[], np));
        let st = lay(&vp).strip.expect("a strip");
        assert_eq!(st.title.y, st.rect.y + 10, "a lone title is centred");
        assert_eq!(st.artist, None);
    }

    #[test]
    fn under_a_scrollbar_the_strip_follows_it() {
        let apps: Vec<_> = (0..7).map(|i| app(&format!("p{i}"), 8)).collect();
        let vp = VolumePopup::new(&with_track(&apps, track(true)));
        let l = lay(&vp);
        let bar = l.bar.expect("a scrollbar");
        let st = l.strip.expect("a strip");
        assert_eq!(st.separator.y, bar.bottom() + 10);
        assert_eq!(st.rect.bottom() + MARGIN, l.panel.bottom());
    }

    #[test]
    fn a_click_on_a_transport_button_acts_on_the_release() {
        let mut vp = VolumePopup::new(&with_track(&[], track(true)));
        let l = lay(&vp);
        let (x, y) = mid(l.strip.as_ref().unwrap().button(Transport::Next));
        assert_eq!(vp.press(&l, x, y), Event::Redraw, "shows pressed");
        assert_eq!(vp.focus, Focus::Media(Transport::Next));
        assert_eq!(vp.release(), Event::Media(Transport::Next));
        // Let go off the button: nothing happens, the button lets go.
        vp.press(&l, x, y);
        assert_eq!(vp.motion(&l, x, y - 30), Event::Redraw);
        assert_eq!(vp.release(), Event::None);
    }

    #[test]
    fn a_greyed_transport_button_takes_nothing() {
        let mut np = track(false);
        np.can_previous = false;
        let mut vp = VolumePopup::new(&with_track(&[], np));
        let l = lay(&vp);
        let (x, y) = mid(l.strip.as_ref().unwrap().button(Transport::Previous));
        assert_eq!(hit(&vp, &l, x, y), Hit::Media(Transport::Previous));
        vp.press(&l, x, y);
        assert_eq!(vp.focus, Focus::Thumb(0), "the focus stays");
        assert_eq!(vp.release(), Event::None);
    }

    #[test]
    fn tab_reaches_the_live_transport_buttons_and_enter_clicks() {
        let mut np = track(true);
        np.can_next = false;
        let mut vp = VolumePopup::new(&with_track(&[], np));
        let l = lay(&vp);
        let mut seen = Vec::new();
        for _ in 0..6 {
            vp.key(&l, form::Key::Tab { back: false });
            seen.push(vp.focus);
        }
        assert_eq!(
            seen,
            [
                Focus::Mute(0),
                Focus::Media(Transport::Previous),
                Focus::Media(Transport::PlayPause),
                Focus::Output,
                Focus::Input,
                Focus::Thumb(0),
            ],
            "Next is greyed and no stop"
        );
        vp.focus = Focus::Media(Transport::PlayPause);
        assert_eq!(
            vp.key(&l, form::Key::Enter),
            Event::Media(Transport::PlayPause)
        );
        assert_eq!(
            vp.key(&l, form::Key::Space),
            Event::Media(Transport::PlayPause)
        );
    }

    #[test]
    fn sync_follows_the_player_and_drops_a_button_that_greyed() {
        let mut vp = VolumePopup::new(&with_track(&[], track(true)));
        let l = lay(&vp);
        let (x, y) = mid(l.strip.as_ref().unwrap().button(Transport::Next));
        vp.press(&l, x, y);
        let mut np = track(true);
        np.title = "Next Song".into();
        np.can_next = false;
        vp.sync(&l, &with_track(&[], np.clone()));
        assert_eq!(vp.playing, Some(np));
        assert_eq!(vp.focus, Focus::Thumb(0));
        assert_eq!(vp.release(), Event::None, "the press ended");
        vp.focus = Focus::Media(Transport::Previous);
        vp.sync(&l, &mix(&[]));
        assert_eq!(vp.playing, None);
        assert_eq!(vp.focus, Focus::Thumb(0), "the strip went");
        assert!(lay(&vp).strip.is_none());
        assert_eq!(lay(&vp).panel.height, 341, "back to the bare panel");
    }

    /// The strip in Classic: the etched separator, the title and artist,
    /// the raised buttons, and their glyphs. Play is 4 columns from 7
    /// rows down to 1, centred: from (14, 10) in a 32x28 button. Pause is
    /// two 2 px bars from (13, 10). A greyed Previous is etched, its 2 px bar
    /// in shadow at (13, 10) with a hilight copy a pixel down and right.
    #[test]
    fn the_strip_draws_its_text_and_glyph_buttons_in_classic() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut np = track(false);
        np.can_previous = false;
        let vp = VolumePopup::new(&with_track(&[], np));
        let (c, l) = drawn(&theme, &vp);
        let st = l.strip.as_ref().unwrap();
        assert_eq!(
            c.get(st.separator.x + 5, st.separator.y),
            cs.button_shadow.argb()
        );
        assert_eq!(
            c.get(st.separator.x + 5, st.separator.y + 1),
            cs.button_hilight.argb()
        );
        assert!(inked(&c, st.title, cs.button_text.argb()) > 0, "the title");
        assert!(
            inked(&c, st.artist.unwrap(), cs.button_text.argb()) > 0,
            "the artist"
        );
        let ink = cs.button_text.argb();
        let face = cs.button_face.argb();
        let play = st.button(Transport::PlayPause);
        assert_eq!(c.get(play.x, play.y), cs.button_hilight.argb(), "raised");
        let (gx, gy) = (play.x + 14, play.y + 10);
        let column = |c: &Canvas, x: i32| (gy..gy + 7).filter(|&y| c.get(x, y) == ink).count();
        assert_eq!(
            (0..5).map(|k| column(&c, gx + k)).collect::<Vec<_>>(),
            [7, 5, 3, 1, 0],
            "Play points right"
        );
        assert_eq!(c.get(gx + 3, gy + 2), face);
        let prev = st.button(Transport::Previous);
        assert_eq!(c.get(prev.x + 13, prev.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(prev.x + 14, prev.y + 17), cs.button_hilight.argb());
        assert_eq!(inked(&c, prev, ink), 0, "greyed: no black ink");
        let next = st.button(Transport::Next);
        let (nx, ny) = (next.x + 13, next.y + 10);
        assert_eq!(
            (0..6)
                .map(|k| (ny..ny + 7).filter(|&y| c.get(nx + k, y) == ink).count())
                .collect::<Vec<_>>(),
            [7, 5, 3, 1, 7, 7],
            "Next: a triangle, then a 2 px bar"
        );

        let vp = VolumePopup::new(&with_track(&[], track(true)));
        let (c, l) = drawn(&theme, &vp);
        let play = l.strip.as_ref().unwrap().button(Transport::PlayPause);
        let (px0, py0) = (play.x + 13, play.y + 10);
        assert_eq!(
            (0..6)
                .map(|k| (py0..py0 + 7).filter(|&y| c.get(px0 + k, y) == ink).count())
                .collect::<Vec<_>>(),
            [7, 7, 0, 0, 7, 7],
            "Pause: two bars"
        );
    }

    #[test]
    fn a_held_classic_transport_button_draws_pushed() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut vp = VolumePopup::new(&with_track(&[], track(false)));
        let l = lay(&vp);
        let play = l.strip.as_ref().unwrap().button(Transport::PlayPause);
        vp.press(&l, play.x + 5, play.y + 5);
        let (c, _) = drawn(&theme, &vp);
        assert_eq!(
            c.get(play.x, play.y),
            cs.window_frame.argb(),
            "pushed frame"
        );
        let ink = cs.button_text.argb();
        assert_eq!(
            c.get(play.x + 15, play.y + 11),
            ink,
            "the glyph moved a pixel"
        );
        assert_eq!(c.get(play.x + 15, play.y + 17), ink);
        assert_ne!(c.get(play.x + 14, play.y + 10), ink);
    }

    #[test]
    fn blue_and_glass_draw_the_strip_buttons_in_their_own_art() {
        let mut np = track(true);
        np.can_next = false;
        let vp = VolumePopup::new(&with_track(&[], np));
        let (cc, cl) = drawn(&Theme::default(), &vp);
        let pixels = |c: &Canvas, r: Rect| {
            (r.x..r.right())
                .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect::<Vec<_>>()
        };
        let cst = cl.strip.as_ref().unwrap();
        for look in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(look);
            let (c, l) = drawn(&theme, &vp);
            let st = l.strip.as_ref().unwrap();
            let up = if look == Interface::Glass { 6 } else { 0 };
            assert_eq!(
                st.buttons,
                cst.buttons
                    .map(|b| Rect::new(b.x, b.y - up, b.width, b.height)),
                "{look:?}: the same geometry, 6 px higher under Glass"
            );
            let ink = theme.colors.button_text.argb();
            for t in Transport::ALL {
                let r = st.button(t);
                assert_ne!(
                    pixels(&c, r),
                    pixels(&cc, cst.button(t)),
                    "{look:?}: {t:?} wears its own art"
                );
                let glyph = Rect::new(r.x + 13, r.y + 10, 6, 7);
                let inked_glyph = inked(&c, glyph, ink);
                if t == Transport::Next {
                    assert_eq!(inked_glyph, 0, "{look:?}: greyed Next is not inked");
                } else {
                    assert!(inked_glyph >= 28, "{look:?}: {t:?}'s glyph is inked");
                }
            }
            assert!(inked(&c, st.title, ink) > 0, "{look:?}: the title is drawn");
        }
    }

    #[test]
    fn access_has_the_track_and_its_buttons() {
        let mut np = track(true);
        np.title = "Tom & Jerry".into();
        np.can_previous = false;
        let mut vp = VolumePopup::new(&with_track(&[], np));
        let l = lay(&vp);
        let (tree, _) = accessed(&vp, &l);
        let g = tree
            .find(Role::Group, NOW_PLAYING)
            .expect("a Now Playing group");
        let kids = tree
            .get(g)
            .map(|n| n.children().to_vec())
            .unwrap_or_default();
        let got: Vec<_> = kids
            .iter()
            .filter_map(|&k| tree.get(k))
            .map(|n| {
                (
                    n.role(),
                    n.label().or(n.value()).unwrap_or("").to_string(),
                    n.is_disabled(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (Role::Label, "Tom & Jerry".into(), false),
                (Role::Label, "The Band".into(), false),
                (Role::Button, "Previous".into(), true),
                (Role::Button, "Pause".into(), false),
                (Role::Button, "Next".into(), false),
            ]
        );
        vp.focus = Focus::Media(Transport::Next);
        let (tree, focus) = accessed(&vp, &l);
        assert_eq!(tree.get(focus).and_then(|n| n.label()), Some("Next"));
    }

    #[test]
    fn a_muted_column_greys_its_level_and_thumb_and_crosses_its_speaker() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut muted = app("mpv", 16);
        muted.mute = true;
        let vp = VolumePopup::new(&mix(&[app("firefox", 16), muted]));
        let (c, l) = drawn(&theme, &vp);
        let ink = cs.button_text.argb();
        let (on, off) = (col(&l, 1), col(&l, 2));
        assert!(inked(&c, on.level, ink) > 0, "80% in black");
        assert_eq!(inked(&c, off.level, ink), 0, "a muted level greys");
        let thumb = |r: Rect| inked(&c, r.inset(2), cs.button_hilight.argb());
        assert_eq!(thumb(on.track.thumb), 0, "a plain face");
        assert!(thumb(off.track.thumb) > 0, "a muted thumb is speckled");
        let glyph = |cl: &ColumnLayout| {
            let r = cl.mute;
            (r.x..r.right())
                .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect::<Vec<_>>()
        };
        assert_ne!(
            glyph(on),
            glyph(off),
            "the speaker and the crossed one differ"
        );
        assert!(
            inked(&c, off.mute, ink) > 0,
            "a muted speaker is still black"
        );
    }

    #[test]
    fn a_level_reads_in_percent() {
        let vp = VolumePopup::new(&mix(&[app("mpv", 13)]));
        let l = lay(&vp);
        let (tree, _) = accessed(&vp, &l);
        let levels: Vec<_> = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::Label && n.bounds().is_some())
            .filter_map(|(_, n)| n.value().map(str::to_string))
            .filter(|v| v.ends_with('%'))
            .collect();
        assert_eq!(levels, ["50%", "65%"]);
    }

    #[test]
    fn the_speaker_button_is_a_toggle_named_mute() {
        let mut a = app("mpv", 8);
        a.mute = true;
        let vp = VolumePopup::new(&mix(&[a]));
        let l = lay(&vp);
        let (tree, _) = accessed(&vp, &l);
        let toggled: Vec<_> = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::Button && n.label() == Some(MUTE))
            .map(|(_, n)| n.toggled())
            .collect();
        assert_eq!(
            toggled,
            [
                Some(accesskit::Toggled::False),
                Some(accesskit::Toggled::True)
            ]
        );
    }

    #[test]
    fn at_200_percent_the_parts_grow_and_icons_glyphs_and_thumbs_keep_their_size() {
        let theme = Theme::default().with_form_text(200);
        let mut a = app("mpv", 8);
        a.icon = Some(Canvas::new(32, 32));
        let vp = VolumePopup::new(&mix(&[a]));
        let l = lay_with(&vp, &theme, (960, 1052));
        let one = lay(&vp);
        let (c, c1) = (col(&l, 1), col(&one, 1));
        assert_eq!(c.rect.width, 192);
        assert_eq!(c.rect.height, 494);
        assert_eq!(c.track.channel.height, 280);
        assert_eq!((c.mute.width, c.mute.height), (60, 52));
        assert_eq!(c.icon.map(|r| (r.width, r.height)), Some((32, 32)));
        let size = |r: Rect| (r.width, r.height);
        assert_eq!(size(c.track.thumb), size(c1.track.thumb));
        assert_eq!(l.output.field.width, 720);
        assert_eq!(l.output.row_h, 2 * one.output.row_h);
        assert_eq!(l.bar, None);
    }

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// Two programs, the second muted, the focus ring on the first's thumb, at scale 1 and 2.
    #[test]
    fn the_popup_is_pinned_in_each_look_at_scale_1_and_2() {
        if !crate::text::test_tahoma() {
            return;
        }
        let mut muted = app("mpv", 8);
        muted.mute = true;
        let mut vp = VolumePopup::new(&mix(&[app("firefox", 16), muted]));
        vp.focus = Focus::Thumb(1);
        vp.cues = true;
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let base = theme_for(look);
            let text = TextRenderer::load(&base.fonts);
            for s in [1, 2] {
                let theme = Theme {
                    look: base.look.at_scale(s),
                    ..base.clone()
                };
                let bounds = Rect::new(0, 0, 480, 380);
                let l = layout(&vp, &theme, &text, (240, 376), Edge::Bottom, bounds);
                let mut c = Canvas::with_scale(480, 380, s);
                draw(&mut c, &theme, &text, &vp, &l);
                got.push((look, s, fnv(&c)));
            }
        }
        assert_eq!(
            got,
            [
                (Interface::Classic, 1, 3894922826410907653),
                (Interface::Classic, 2, 1200130322201881305),
                (Interface::Blue, 1, 12055996091189837898),
                (Interface::Blue, 2, 7254310312881324019),
                (Interface::Glass, 1, 14204876082109196823),
                (Interface::Glass, 2, 18269360693100279155),
            ]
        );
    }

    #[test]
    fn beside_a_side_bar_the_panel_stands_off_its_edge_level_with_the_icon() {
        let vp = VolumePopup::new(&mix(&[]));
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let screen = Rect::new(0, 0, 1920, 1080);
        let left = layout(&vp, &theme, &text, (60, 500), Edge::Left, screen);
        assert_eq!(
            left.panel,
            Rect::new(60, 500 - 170, 444, 341),
            "its left edge on the bar's, centred on the icon"
        );
        let right = layout(&vp, &theme, &text, (1860, 1050), Edge::Right, screen);
        assert_eq!(
            right.panel,
            Rect::new(1860 - 444, 1080 - 341, 444, 341),
            "its right edge on the bar's, kept above the output's foot"
        );
    }
}
