//! The Volume Control window's body: one column per line of the mixer,
//! side by side. The host picks the lines: the output device and each
//! program playing sound, or each recording device.
//!
//! The body is 225 px tall under an etched line. The output device's column
//! (with Mute all) is 135 px wide, every other 105. Each column stacks the
//! title, Balance trackbar between two speakers, vertical Volume trackbar and
//! a check box; the metrics are the consts below. With Advanced Controls on,
//! the first column gets an Advanced button and the body grows. Columns that
//! do not fit scroll a column at a time on a scrollbar along the bottom.
//!
//! A line with no left and right to balance (a mono node) has its
//! Balance slider greyed at the centre; a line with no device behind it
//! greys whole.
//!
//! The host owns the devices and the writes: it hands the mixer its
//! [`Line`]s with [`Mixer::sync`] and acts on the [`Event`]s at once.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    ScrollHit, ScrollParts, scrollbar_hit, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::form;
use crate::look::Paint;
use crate::look::widgets::{
    draw_button, draw_checkbox, draw_checkbox_disabled, draw_disabled_label, draw_focus_ring,
    draw_hscrollbar, draw_label, draw_trackbar, draw_vertical_trackbar,
};
use crate::pointer::{Interaction, Lit, Pointer, WHEEL_ROWS};
use crate::raster::{Canvas, Rect};
use crate::style::Part as StylePart;
use crate::text::{Align, DialogFont, TextRenderer};
use crate::theme::{TextUnits, Theme};
use crate::trackbar::{self, TrackBar};
use accesskit::{NodeId, Role};
use std::cell::Cell;

/// Volume positions: 0 to 60, a tick every tenth.
pub const STEPS: usize = 61;
/// Balance positions: 0 (left only) to 20 (right only), centred at 10.
pub const BALANCE_STEPS: usize = 21;
pub const BALANCE_CENTRE: usize = 10;
/// The output device's column.
pub const DEVICE_W: i32 = 135;
/// Any other column: its slot and its controls.
pub const COL_W: i32 = SLOT + 100;
/// Before a column other than the device's: 2 px clear, the 2 px etched
/// line, 1 px clear.
const SLOT: i32 = 5;
const SEP_X: i32 = 2;
/// Where the separators start: the device column's, and the others'.
const DEVICE_SEP_Y: i32 = 2;
const SEP_Y: i32 = 41;
/// The least width a window over the body keeps.
pub const MIN_W: i32 = 106;
/// The text starts this far into a column's controls.
const PAD: i32 = 6;
/// The sliders' channels centre on the line half a pixel right of this
/// many px into the controls: the device column's, and the others'.
const DEVICE_MID: i32 = 67;
const MID: i32 = 50;
const ROW_H: i32 = 13;
/// A 13 px text row puts its baseline 10 px down: each row's top.
const TITLE_Y: i32 = 11;
const UNDERLINE_Y: i32 = 31;
/// The device column's title line.
const UNDERLINE_W: i32 = 121;
const BALANCE_LABEL_Y: i32 = 41;
const BALANCE_Y: i32 = 59;
const BALANCE_W: i32 = 44;
/// The speakers at the balance trackbar's ends, their gap to its channel
/// and their drop below its top.
const SPEAKER: i32 = 16;
const SPEAKER_GAP: i32 = 8;
const SPEAKER_Y: i32 = 1;
const VOLUME_LABEL_Y: i32 = 94;
const TRACK_Y: i32 = 117;
/// The volume channel's height in a body that shows the whole column, and its least.
const TRACK_H: i32 = 75;
const MIN_TRACK_H: i32 = 40;
const CHANNEL_W: i32 = 4;
/// From the channel's bottom to the check box, and from the check row to
/// the body's bottom.
const CHECK_GAP: i32 = 11;
const BOTTOM: i32 = 9;
/// Between the check row and the Advanced button, the button, and what
/// the button's row adds to the body.
const ADVANCED_GAP: i32 = 8;
const ADVANCED_W: i32 = 83;
const ADVANCED_H: i32 = 23;
const ADVANCED_ROW: i32 = 27;
/// A focus ring sits this far outside the label it marks, and takes
/// this many rows more under it.
const RING_PAD: i32 = 1;
const RING_BELOW: i32 = 1;

pub const BALANCE: &str = "Balance:";
pub const VOLUME: &str = "Volume:";
pub const ADVANCED: &str = "Advanced";

/// What a column's check box stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// The output device's mute, which silences every program.
    MuteAll,
    /// A program's own mute.
    Mute,
    /// The recording device's pick: the one programs record from.
    Select,
}

impl Check {
    pub fn label(self) -> &'static str {
        match self {
            Check::MuteAll => "Mute all",
            Check::Mute => "Mute",
            Check::Select => "Select",
        }
    }
}

/// One line as the host hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// What the host knows the line by; [`Event`]s carry it.
    pub key: String,
    pub title: String,
    /// The volume position, 0 to `STEPS - 1`.
    pub volume: usize,
    /// The balance position, 0 to `BALANCE_STEPS - 1`; `None` for a line
    /// with nothing to balance.
    pub balance: Option<usize>,
    pub check: Check,
    pub checked: bool,
    /// False with no device behind it: the column greys.
    pub live: bool,
}

/// One column's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub key: String,
    pub title: String,
    pub balance: TrackBar,
    pub volume: TrackBar,
    pub check: Check,
    pub checked: bool,
    pub live: bool,
}

impl Column {
    fn new(l: &Line) -> Column {
        let mut volume = TrackBar::new(l.volume, STEPS).paged(12, 10);
        volume.enabled = l.live;
        let mut balance =
            TrackBar::new(l.balance.unwrap_or(BALANCE_CENTRE), BALANCE_STEPS).paged(5, 10);
        balance.enabled = l.live && l.balance.is_some();
        Column {
            key: l.key.clone(),
            title: l.title.clone(),
            balance,
            volume,
            check: l.check,
            checked: l.checked,
            live: l.live,
        }
    }
}

/// What the keyboard is on. Column parts carry the column's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Balance(usize),
    Volume(usize),
    Check(usize),
    Advanced,
}

/// The parts the pointer can light or hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Balance(usize),
    Volume(usize),
    Check(usize),
    Advanced,
    Bar(ScrollHit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    /// Line `.0`'s volume trackbar moved to this position.
    Volume(String, usize),
    /// Line `.0`'s balance trackbar moved to this position.
    Balance(String, usize),
    /// Line `.0`'s check box flipped to this.
    Check(String, bool),
    /// The Advanced button was pressed.
    Advanced,
}

pub struct Mixer {
    pub columns: Vec<Column>,
    /// The host's speaker icon (16 px, facing right) for the balance
    /// trackbar's ends; `None` leaves them empty.
    pub speaker: Option<Canvas>,
    /// Advanced Controls is on: the first column has its button.
    pub advanced: bool,
    /// The first column shown.
    pub first: usize,
    pub focus: Focus,
    /// Whether a key has been pressed: only then is the focus ring drawn.
    pub cues: bool,
    pub ptr: Pointer<Part>,
    /// Where the pointer last pressed or moved; `None` once it left.
    point: Option<(i32, i32)>,
    /// How far right of its left edge the held scrollbar thumb was
    /// grabbed.
    grab: i32,
    /// Whether the last draw's look lights a part under the pointer.
    styled: Cell<bool>,
}

/// Where one column's parts go.
#[derive(Clone, Debug)]
pub struct ColumnLayout {
    /// The column's controls, less its slot, down to the scrollbar.
    pub rect: Rect,
    pub title: Rect,
    /// The etched line under the output device's title.
    pub underline: Option<Rect>,
    pub balance_label: Rect,
    pub balance: trackbar::Layout,
    /// The speakers left and right of the balance trackbar.
    pub speakers: [Rect; 2],
    pub volume_label: Rect,
    pub volume: trackbar::Layout,
    pub check: Rect,
    pub check_label: Rect,
    /// The etched line in its slot; `None` on the first column shown.
    pub separator: Option<Rect>,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub body: Rect,
    /// One per column; `None` for a column scrolled out of view.
    pub columns: Vec<Option<ColumnLayout>>,
    /// Where each column scrolled out of view would lie, left or right of the body; `None` for
    /// a shown column. Only `access` reads them.
    pub beyond: Vec<Option<ColumnLayout>>,
    /// Where the shown columns lie: the body above the scrollbar, as wide as they are.
    pub view: Rect,
    /// The etched line the titles of the columns but the device's share.
    pub title_line: Option<Rect>,
    /// The first column's Advanced button, when it shows.
    pub advanced: Option<Rect>,
    /// How many columns fit side by side.
    pub shown: usize,
    pub bar: Option<Rect>,
    /// The look's shortest scrollbar thumb.
    pub bar_thumb_min: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Balance(usize, trackbar::Hit),
    Volume(usize, trackbar::Hit),
    /// A column's check box or its label.
    Check(usize),
    Advanced,
    /// A column, on nothing that acts.
    Column(usize),
    Bar(ScrollHit),
    Body,
    Outside,
}

/// The layout's lengths at the text size: the columns, their text rows, the channels and the
/// Advanced button grow with the text; the slots, the etched lines, the channel's width, the
/// speakers, the check box and the thumbs keep their size.
#[derive(Clone, Copy, Debug)]
struct Dims {
    device_w: i32,
    col_w: i32,
    min_w: i32,
    pad: i32,
    device_mid: i32,
    mid: i32,
    row_h: i32,
    title_y: i32,
    underline_y: i32,
    underline_w: i32,
    sep_y: i32,
    balance_label_y: i32,
    balance_y: i32,
    balance_w: i32,
    volume_label_y: i32,
    track_y: i32,
    track_h: i32,
    min_track_h: i32,
    check_gap: i32,
    body_h: i32,
    advanced_gap: i32,
    advanced_w: i32,
    advanced_h: i32,
    advanced_row: i32,
}

impl Dims {
    fn new(u: TextUnits) -> Dims {
        let l = |v| u.len(v);
        let (track_y, track_h, check_gap, row_h) = (l(TRACK_Y), l(TRACK_H), l(CHECK_GAP), l(ROW_H));
        Dims {
            device_w: l(DEVICE_W),
            col_w: SLOT + l(COL_W - SLOT),
            min_w: l(MIN_W),
            pad: l(PAD),
            device_mid: l(DEVICE_MID),
            mid: l(MID),
            row_h,
            title_y: l(TITLE_Y),
            underline_y: l(UNDERLINE_Y),
            underline_w: l(UNDERLINE_W),
            sep_y: l(SEP_Y),
            balance_label_y: l(BALANCE_LABEL_Y),
            balance_y: l(BALANCE_Y),
            balance_w: l(BALANCE_W),
            volume_label_y: l(VOLUME_LABEL_Y),
            track_y,
            track_h,
            min_track_h: l(MIN_TRACK_H),
            check_gap,
            body_h: track_y + track_h + check_gap + row_h + l(BOTTOM),
            advanced_gap: l(ADVANCED_GAP),
            advanced_w: l(ADVANCED_W),
            advanced_h: l(ADVANCED_H),
            advanced_row: l(ADVANCED_ROW),
        }
    }

    /// Under the columns: the Advanced button's row when it shows.
    fn below(&self, advanced: bool) -> i32 {
        if advanced { self.advanced_row } else { 0 }
    }

    /// A column's width, with its slot.
    fn extent(&self, c: &Column) -> i32 {
        if is_device(c) {
            self.device_w
        } else {
            self.col_w
        }
    }
}

/// Whether a column is the output device's, which has its own width.
fn is_device(c: &Column) -> bool {
    c.check == Check::MuteAll
}

/// The body width at text units `u` that shows the output device's column (with `device`) and
/// `others` more side by side, and at least [`min_width`].
pub fn width(device: bool, others: usize, u: TextUnits) -> i32 {
    let d = Dims::new(u);
    (i32::from(device) * d.device_w + others as i32 * d.col_w).max(d.min_w)
}

/// The least width a window over the body keeps at text units `u`.
pub fn min_width(u: TextUnits) -> i32 {
    Dims::new(u).min_w
}

/// The body height at text units `u`, with the Advanced button's row or without.
pub fn height(advanced: bool, u: TextUnits) -> i32 {
    let d = Dims::new(u);
    d.body_h + d.below(advanced)
}

/// Column `col`'s parts, its controls from `x` in a body from `y`, the
/// volume channel `track_h` tall and the column `h` tall.
#[expect(clippy::too_many_arguments)]
fn column_layout(
    col: &Column,
    text: &TextRenderer,
    px: u32,
    cm: &crate::metrics::ControlMetrics,
    d: &Dims,
    x: i32,
    y: i32,
    track_h: i32,
    h: i32,
) -> ColumnLayout {
    let device = is_device(col);
    let (w, mid) = if device {
        (d.device_w, d.device_mid)
    } else {
        (d.col_w - SLOT, d.mid)
    };
    let label_w = w - 2 * d.pad;
    let check_y = y + d.track_y + track_h + d.check_gap;
    let check_w = text.measure(col.check.label(), false, px);
    // The channels run from a pixel left of the centre line.
    let channel_x = x + mid - 1;
    let balance_x = channel_x + CHANNEL_W / 2 - d.balance_w / 2;
    let speaker_y = y + d.balance_y + SPEAKER_Y;
    let thumb = cm.vtrack_thumb;
    let row = |top: i32, w: i32| Rect::new(x + d.pad, y + top, w, d.row_h);
    ColumnLayout {
        rect: Rect::new(x, y, w, h),
        title: row(d.title_y, label_w),
        underline: device.then(|| Rect::new(x + d.pad, y + d.underline_y, d.underline_w, 2)),
        balance_label: row(d.balance_label_y, label_w),
        balance: trackbar::layout(&col.balance, balance_x, y + d.balance_y, d.balance_w),
        speakers: [
            Rect::new(
                balance_x - SPEAKER_GAP - SPEAKER,
                speaker_y,
                SPEAKER,
                SPEAKER,
            ),
            Rect::new(
                balance_x + d.balance_w + SPEAKER_GAP,
                speaker_y,
                SPEAKER,
                SPEAKER,
            ),
        ],
        volume_label: row(d.volume_label_y, label_w),
        volume: trackbar::layout_vertical_with(
            &col.volume,
            channel_x - (trackbar::vertical_width(thumb) - CHANNEL_W) / 2,
            y + d.track_y,
            track_h,
            thumb,
        ),
        check: Rect::new(
            x + d.pad,
            check_y + (d.row_h - ROW_H) / 2,
            cm.check,
            cm.check,
        ),
        check_label: Rect::new(x + d.pad + cm.check_label, check_y, check_w, d.row_h),
        separator: None,
    }
}

/// How many columns from `first` fit side by side in `w`: at least one.
fn fitting(m: &Mixer, d: &Dims, first: usize, w: i32) -> usize {
    let mut used = 0;
    let mut n = 0;
    for c in m.columns.iter().skip(first) {
        used += d.extent(c);
        if used > w && n > 0 {
            break;
        }
        n += 1;
    }
    n.max(1)
}

/// The first column shown when the last one is: as far right as the
/// columns scroll.
fn last_first_at(m: &Mixer, d: &Dims, w: i32) -> usize {
    let mut used = 0;
    let mut first = m.columns.len();
    for c in m.columns.iter().rev() {
        used += d.extent(c);
        if used > w && first < m.columns.len() {
            break;
        }
        first -= 1;
    }
    first
}

/// Lay the mixer out in `body`, grown by the theme's text units.
pub fn layout(m: &Mixer, theme: &Theme, text: &TextRenderer, body: Rect) -> Layout {
    let text = text.dialog(DialogFont::ShellDlg);
    let px = theme.fonts.size_px;
    let cm = theme.look.controls();
    let d = Dims::new(theme.text);
    let scrolls = m.columns.len() > fitting(m, &d, 0, body.width);
    let first = m.first.min(last_first_at(m, &d, body.width));
    let shown = fitting(m, &d, first, body.width).min(m.columns.len().saturating_sub(first));
    let bar_h = if scrolls { cm.scroll } else { 0 };
    let h = body.height - bar_h;
    let track_h = (h - (d.body_h - d.track_h) - d.below(m.advanced)).max(d.min_track_h);
    let mut x = body.x - m.columns[..first].iter().map(|c| d.extent(c)).sum::<i32>();
    let mut prev_device = None;
    let mut columns = Vec::with_capacity(m.columns.len());
    let mut beyond = Vec::with_capacity(m.columns.len());
    let mut right = body.x;
    for (i, c) in m.columns.iter().enumerate() {
        let left = x;
        x += d.extent(c);
        let device = is_device(c);
        let controls = if device { left } else { left + SLOT };
        let mut cl = column_layout(c, text, px, &cm, &d, controls, body.y, track_h, h);
        if i < first || i >= first + shown {
            columns.push(None);
            beyond.push(Some(cl));
            continue;
        }
        if !device && let Some(after_device) = prev_device {
            let top = if after_device { DEVICE_SEP_Y } else { d.sep_y };
            cl.separator = Some(Rect::new(left + SEP_X, body.y + top, 2, h - top));
        }
        prev_device = Some(device);
        right = x;
        columns.push(Some(cl));
        beyond.push(None);
    }
    let title_line = columns
        .iter()
        .flatten()
        .find(|cl| cl.underline.is_none())
        .map(|cl| {
            Rect::new(
                cl.title.x,
                body.y + d.underline_y,
                body.right() - cl.title.x,
                2,
            )
        });
    let advanced = match columns.first() {
        Some(Some(cl)) if m.advanced => Some(Rect::new(
            cl.check.x,
            cl.check_label.bottom() + d.advanced_gap,
            d.advanced_w,
            d.advanced_h,
        )),
        _ => None,
    };
    Layout {
        body,
        columns,
        beyond,
        view: Rect::new(body.x, body.y, (right - body.x).min(body.width), h),
        title_line,
        advanced,
        shown,
        bar: scrolls.then(|| Rect::new(body.x, body.bottom() - cm.scroll, body.width, cm.scroll)),
        bar_thumb_min: cm.thumb_min,
    }
}

fn bar_parts(m: &Mixer, lay: &Layout, bar: Rect) -> ScrollParts {
    scrollbar_parts(
        bar,
        1,
        m.first,
        lay.shown,
        m.columns.len(),
        true,
        lay.bar_thumb_min,
    )
}

pub fn hit(m: &Mixer, lay: &Layout, x: i32, y: i32) -> Hit {
    if !lay.body.contains(x, y) {
        return Hit::Outside;
    }
    if let Some(bar) = lay.bar
        && bar.contains(x, y)
        && let Some(h) = scrollbar_hit(&bar_parts(m, lay, bar), x, y, true)
    {
        return Hit::Bar(h);
    }
    if lay.advanced.is_some_and(|r| r.contains(x, y)) {
        return Hit::Advanced;
    }
    for (i, cl) in lay.columns.iter().enumerate() {
        let Some(cl) = cl else { continue };
        if let Some(h) = trackbar::hit(&cl.balance, x, y) {
            return Hit::Balance(i, h);
        }
        if let Some(h) = trackbar::hit(&cl.volume, x, y) {
            return Hit::Volume(i, h);
        }
        let row = Rect::new(
            cl.check.x,
            cl.check_label.y,
            cl.check_label.right() - cl.check.x,
            cl.check_label.height,
        );
        if row.contains(x, y) {
            return Hit::Check(i);
        }
        if Rect::new(cl.rect.x, lay.body.y, cl.rect.width, lay.body.height).contains(x, y) {
            return Hit::Column(i);
        }
    }
    Hit::Body
}

/// The body: the face, the line along its top, each shown column with
/// its separator, the title lines, the Advanced button and the
/// scrollbar. The focus ring shows once a key was pressed.
pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, m: &Mixer, lay: &Layout) {
    m.styled.set(theme.look.lights_pointer());
    let text = text.dialog(DialogFont::ShellDlg);
    let p = &Paint {
        s: 1,
        ..theme.paint()
    };
    let cs = p.cs;
    let px = theme.fonts.size_px;
    let b = lay.body;
    c.fill_rect(b, cs.button_face);
    c.etched_hline_t(b.x, b.right(), b.y, cs, 1);
    for (i, (col, cl)) in m.columns.iter().zip(&lay.columns).enumerate() {
        if let Some(cl) = cl {
            draw_column(c, text, p, m, i, col, cl, px);
        }
    }
    if let Some(r) = lay.title_line {
        c.etched_hline_t(r.x, r.right(), r.y, cs, 1);
    }
    if let Some(r) = lay.advanced {
        let live = m.first_live();
        let focus = m.cues && m.focus == Focus::Advanced;
        draw_button(
            c,
            text,
            p,
            r,
            ADVANCED,
            false,
            m.ptr.of(Part::Advanced),
            live,
            focus,
            px,
        );
    }
    if let Some(bar) = lay.bar {
        let lit = match m.ptr.lit() {
            Some((Part::Bar(h), how)) => Some((h, how)),
            _ => None,
        };
        draw_hscrollbar(c, p, bar, m.first, lay.shown, m.columns.len(), lit);
    }
}

/// The focus ring round a balance trackbar: 8 px out from its channel's
/// ends, from 2 px over the thumb to the ticks' last row.
fn balance_ring(t: &trackbar::Layout) -> Rect {
    Rect::new(t.channel.x - 8, t.bounds.y - 2, t.channel.width + 16, 28)
}

/// The focus ring round a volume trackbar: 8 px over and under its
/// channel, from 5 px left of the ticks to 9 px right of them.
fn volume_ring(t: &trackbar::Layout) -> Rect {
    Rect::new(
        t.bounds.x - 5,
        t.channel.y - 8,
        t.bounds.width + 14,
        t.channel.height + 16,
    )
}

/// The focus ring round a check box's label.
fn check_ring(label: Rect) -> Rect {
    let r = label.inset(-RING_PAD);
    Rect::new(r.x, r.y, r.width, r.height + RING_BELOW)
}

/// A label in its column, greyed with `live` off.
#[expect(clippy::too_many_arguments)]
fn column_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    live: bool,
    part: Option<StylePart>,
    px: u32,
) {
    if live {
        draw_label(c, text, p, r, label, p.cs.button_text, Align::Left, px);
    } else {
        draw_disabled_label(c, text, p, r, label, Align::Left, px, part);
    }
}

/// Column `i`: its separator, its title cut to the column (and the
/// device's line under it), the two labelled trackbars, the check row,
/// and the focus ring on whichever part has the focus.
#[expect(clippy::too_many_arguments)]
fn draw_column(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    m: &Mixer,
    i: usize,
    col: &Column,
    cl: &ColumnLayout,
    px: u32,
) {
    let cs = p.cs;
    let live = col.live;
    if let Some(s) = cl.separator {
        c.etched_vline_t(s.x, s.y, s.bottom(), cs, 1);
    }
    if let Some(u) = cl.underline {
        c.etched_divider_t(u.x, u.right() - 1, u.y, cs, 1);
    }
    // A line's title is plain text: an `&` in a program's name is no
    // access key.
    let title = text.ellipsize(&col.title, cl.title.width, false, px);
    if live {
        text.draw(c, cl.title, &title, false, cs.button_text, Align::Left, px);
    } else {
        draw_disabled_label(
            c,
            text,
            p,
            cl.title,
            &crate::text::escape_mnemonic(&title),
            Align::Left,
            px,
            None,
        );
    }
    let balanced = col.balance.enabled;
    column_label(c, text, p, cl.balance_label, BALANCE, balanced, None, px);
    if let Some(right) = &m.speaker {
        for (r, icon) in cl.speakers.into_iter().zip([&right.mirrored(), right]) {
            let x = r.x + (r.width - icon.width as i32) / 2;
            let y = r.y + (r.height - icon.height as i32) / 2;
            if balanced {
                c.blit_over(x, y, icon);
            } else {
                c.blit_over_faded(x, y, icon, 1, 2);
            }
        }
    }
    let focused = |f: Focus| m.cues && m.focus == f;
    draw_trackbar(
        c,
        p,
        &cl.balance,
        balanced,
        focused(Focus::Balance(i)),
        m.ptr.of(Part::Balance(i)),
    );
    column_label(c, text, p, cl.volume_label, VOLUME, live, None, px);
    draw_vertical_trackbar(
        c,
        p,
        &cl.volume,
        live,
        focused(Focus::Volume(i)),
        m.ptr.of(Part::Volume(i)),
    );
    if live {
        draw_checkbox(c, p, cl.check, col.checked, m.ptr.of(Part::Check(i)));
    } else {
        draw_checkbox_disabled(c, p, cl.check, col.checked);
    }
    column_label(
        c,
        text,
        p,
        cl.check_label,
        col.check.label(),
        live,
        Some(StylePart::CheckBox),
        px,
    );
    if m.cues && live {
        let ring = match m.focus {
            Focus::Balance(f) if f == i => Some(balance_ring(&cl.balance)),
            Focus::Volume(f) if f == i => Some(volume_ring(&cl.volume)),
            Focus::Check(f) if f == i => Some(check_ring(cl.check_label)),
            _ => None,
        };
        if let Some(ring) = ring {
            draw_focus_ring(c, p, ring, cs.button_text);
        }
    }
}

impl Mixer {
    pub fn new(lines: &[Line], advanced: bool) -> Mixer {
        Mixer {
            columns: lines.iter().map(Column::new).collect(),
            speaker: None,
            advanced,
            first: 0,
            focus: Focus::Volume(0),
            cues: false,
            ptr: Pointer::new(),
            point: None,
            grab: 0,
            styled: Cell::new(false),
        }
    }

    /// The body size at text units `u` that shows every column side by side.
    pub fn size(&self, u: TextUnits) -> (i32, i32) {
        let device = self.columns.first().is_some_and(is_device);
        let others = self.columns.len() - usize::from(device);
        (width(device, others, u), height(self.advanced, u))
    }

    /// Take the lines from the host. Columns are matched by key: one that
    /// stays keeps its focus, light and drag wherever it moved to; one
    /// that left takes the focus to the first column and ends its drag.
    /// A held thumb keeps its own position: the device lags the writes
    /// the drag makes. A part that moves to or from under a resting
    /// pointer turns hot or cold; `Redraw` when that shows.
    pub fn sync(&mut self, lay: &Layout, lines: &[Line]) -> Event {
        let mut columns: Vec<Column> = lines.iter().map(Column::new).collect();
        let moved: Vec<Option<usize>> = self
            .columns
            .iter()
            .map(|old| columns.iter().position(|c| c.key == old.key))
            .collect();
        let remap = |i: usize| moved.get(i).copied().flatten();
        match self.ptr.held() {
            Some(Part::Volume(i)) => {
                if let Some(j) = remap(i) {
                    columns[j].volume.set(self.columns[i].volume.pos);
                }
            }
            Some(Part::Balance(i)) => {
                if let Some(j) = remap(i) {
                    columns[j].balance.set(self.columns[i].balance.pos);
                }
            }
            _ => {}
        }
        self.focus = match self.focus {
            Focus::Balance(i) => remap(i).map_or(Focus::Volume(0), Focus::Balance),
            Focus::Volume(i) => remap(i).map_or(Focus::Volume(0), Focus::Volume),
            Focus::Check(i) => remap(i).map_or(Focus::Volume(0), Focus::Check),
            Focus::Advanced if !columns.first().is_some_and(|c| c.live) => Focus::Volume(0),
            Focus::Advanced => Focus::Advanced,
        };
        self.ptr = std::mem::take(&mut self.ptr).carry(|p| match p {
            Part::Balance(i) => remap(i).map(Part::Balance),
            Part::Volume(i) => remap(i).map(Part::Volume),
            Part::Check(i) => remap(i).map(Part::Check),
            p => Some(p),
        });
        self.columns = columns;
        self.first = self.first.min(self.last_first(lay.shown));
        self.rehit(lay)
    }

    /// Show or hide the Advanced button.
    pub fn set_advanced(&mut self, on: bool) {
        self.advanced = on;
        if !on && self.focus == Focus::Advanced {
            self.focus = Focus::Volume(0);
        }
        self.ptr = std::mem::take(&mut self.ptr).carry(|p| match p {
            Part::Advanced if !on => None,
            p => Some(p),
        });
    }

    /// Whether the Advanced button shows and takes presses: Advanced
    /// Controls is on and the first column is live.
    fn advanced_live(&self) -> bool {
        self.advanced && self.first_live()
    }

    /// Whether the first column is live, which the Advanced button follows.
    fn first_live(&self) -> bool {
        self.columns.first().is_some_and(|c| c.live)
    }

    /// Whether a thumb is held.
    pub fn dragging(&self) -> bool {
        matches!(self.ptr.held(), Some(Part::Volume(_) | Part::Balance(_)))
    }

    /// `lay` with every shown thumb where its column puts it now.
    fn current(&self, lay: &Layout) -> Layout {
        let columns = lay
            .columns
            .iter()
            .zip(&self.columns)
            .map(|(cl, c)| {
                cl.as_ref().map(|cl| ColumnLayout {
                    balance: trackbar::at_position(&cl.balance, c.balance.pos),
                    volume: trackbar::at_position(&cl.volume, c.volume.pos),
                    ..cl.clone()
                })
            })
            .collect();
        Layout {
            columns,
            ..lay.clone()
        }
    }

    fn part_at(&self, lay: &Layout, x: i32, y: i32) -> Option<Part> {
        let col = |i: usize| self.columns.get(i);
        match hit(self, lay, x, y) {
            Hit::Balance(i, trackbar::Hit::Thumb) if col(i)?.balance.enabled => {
                Some(Part::Balance(i))
            }
            Hit::Volume(i, trackbar::Hit::Thumb) if col(i)?.live => Some(Part::Volume(i)),
            Hit::Check(i) if col(i)?.live => Some(Part::Check(i)),
            Hit::Advanced if col(0)?.live => Some(Part::Advanced),
            Hit::Bar(h) => Some(Part::Bar(h)),
            _ => None,
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

    /// `Redraw` when what draws changed and the look shows it. A held
    /// scrollbar part or button shows pressed in every look.
    fn lit(&self, changed: bool) -> Event {
        let pressed = matches!(self.ptr.held(), Some(Part::Bar(_) | Part::Advanced));
        if changed && (self.styled.get() || pressed) {
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
                Some((Part::Bar(_) | Part::Advanced, Interaction::Pressed))
            )
        {
            Event::Redraw
        } else {
            self.lit(changed)
        }
    }

    fn slide_volume(&mut self, i: usize, f: impl FnOnce(&mut TrackBar) -> bool) -> Event {
        let Some(c) = self.columns.get_mut(i) else {
            return Event::None;
        };
        if c.live && f(&mut c.volume) {
            Event::Volume(c.key.clone(), c.volume.pos)
        } else {
            Event::None
        }
    }

    fn slide_balance(&mut self, i: usize, f: impl FnOnce(&mut TrackBar) -> bool) -> Event {
        let Some(c) = self.columns.get_mut(i) else {
            return Event::None;
        };
        if c.balance.enabled && f(&mut c.balance) {
            Event::Balance(c.key.clone(), c.balance.pos)
        } else {
            Event::None
        }
    }

    /// Flip column `i`'s check box. A picked Select stays picked: another
    /// line's Select is what moves the pick.
    fn flip(&mut self, i: usize) -> Event {
        let Some(c) = self.columns.get_mut(i) else {
            return Event::None;
        };
        if !c.live || (c.check == Check::Select && c.checked) {
            return Event::None;
        }
        c.checked = !c.checked;
        let (key, on) = (c.key.clone(), c.checked);
        if c.check == Check::Select {
            for (j, o) in self.columns.iter_mut().enumerate() {
                if j != i && o.check == Check::Select {
                    o.checked = false;
                }
            }
        }
        Event::Check(key, on)
    }

    /// The focus order: each live column's balance (when it has one),
    /// volume and check box, the Advanced button after the first
    /// column's.
    fn focus_order(&self) -> Vec<Focus> {
        let mut v = Vec::new();
        for (i, c) in self.columns.iter().enumerate() {
            if c.live {
                if c.balance.enabled {
                    v.push(Focus::Balance(i));
                }
                v.extend([Focus::Volume(i), Focus::Check(i)]);
                if i == 0 && self.advanced {
                    v.push(Focus::Advanced);
                }
            }
        }
        v
    }

    fn walk(&mut self, back: bool, shown: usize) {
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
        self.reveal_focus(shown);
    }

    /// The column the focus is in.
    fn focus_column(&self) -> usize {
        match self.focus {
            Focus::Balance(i) | Focus::Volume(i) | Focus::Check(i) => i,
            Focus::Advanced => 0,
        }
    }

    /// Scroll so the focused column shows.
    fn reveal_focus(&mut self, shown: usize) {
        self.show_column(self.focus_column(), shown);
    }

    fn show_column(&mut self, i: usize, shown: usize) {
        self.first = self.first.min(self.last_first(shown));
        if i < self.first {
            self.first = i;
        } else if i >= self.first + shown {
            self.first = i + 1 - shown;
        }
    }

    fn last_first(&self, shown: usize) -> usize {
        self.columns.len().saturating_sub(shown)
    }

    fn scroll_bar(&mut self, lay: &Layout, notches: i32) -> Event {
        let last = self.last_first(lay.shown) as i64;
        let first = (self.first as i64 + i64::from(notches)).clamp(0, last);
        if first as usize == self.first {
            return Event::None;
        }
        self.first = first as usize;
        self.rehit(lay);
        Event::Redraw
    }

    fn press_bar(&mut self, lay: &Layout, h: ScrollHit, x: i32) -> Event {
        let last = self.last_first(lay.shown);
        // A body that grew since the last scroll shows fewer columns
        // past the first than `first` left room for.
        self.first = self.first.min(last);
        self.first = match h {
            ScrollHit::Back => self.first.saturating_sub(1),
            ScrollHit::Forward => (self.first + 1).min(last),
            ScrollHit::PageBack => self.first.saturating_sub(lay.shown),
            ScrollHit::PageForward => (self.first + lay.shown).min(last),
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

    /// A reader's Focus on the control at (`x`, `y`): it takes the keyboard as Tab gives it,
    /// without sliding, flipping or pressing it.
    pub fn focus_at(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        let f = match hit(self, lay, x, y) {
            Hit::Balance(i, _) => Focus::Balance(i),
            Hit::Volume(i, _) => Focus::Volume(i),
            Hit::Check(i) => Focus::Check(i),
            Hit::Advanced => Focus::Advanced,
            _ => return Event::None,
        };
        if self.focus == f || !self.focus_order().contains(&f) {
            return Event::None;
        }
        self.focus = f;
        Event::Redraw
    }

    /// A reader's ScrollIntoView on column `i`: the columns scroll until it shows, the focus
    /// staying where it is.
    pub fn reveal(&mut self, lay: &Layout, i: usize) -> Event {
        if i >= self.columns.len() {
            return Event::None;
        }
        let before = self.first;
        self.show_column(i, lay.shown);
        if self.first == before {
            return Event::None;
        }
        self.rehit(lay);
        Event::Redraw
    }

    /// The left button went down at (`x`, `y`). A thumb is held, and the
    /// channel either side of it steps it one position that way; the
    /// focus moves to the part pressed. A check box or the Advanced
    /// button acts at the release.
    pub fn press(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        self.point = Some((x, y));
        let part = self.part_at(lay, x, y);
        let sticky = matches!(
            part,
            Some(Part::Volume(_) | Part::Balance(_) | Part::Bar(ScrollHit::Thumb))
        );
        let changed = self.ptr.press(part, sticky);
        match hit(self, lay, x, y) {
            Hit::Balance(i, h) if self.columns[i].balance.enabled => {
                self.focus = Focus::Balance(i);
                match h {
                    trackbar::Hit::Thumb => Event::Redraw,
                    trackbar::Hit::Left => self.slide_balance(i, |t| t.step(-1)),
                    trackbar::Hit::Right => self.slide_balance(i, |t| t.step(1)),
                }
            }
            Hit::Volume(i, h) if self.columns[i].live => {
                self.focus = Focus::Volume(i);
                match h {
                    trackbar::Hit::Thumb => Event::Redraw,
                    trackbar::Hit::Left => self.slide_volume(i, |t| t.step(-1)),
                    trackbar::Hit::Right => self.slide_volume(i, |t| t.step(1)),
                }
            }
            Hit::Check(i) if self.columns[i].live => {
                self.focus = Focus::Check(i);
                Event::Redraw
            }
            Hit::Advanced if part.is_some() => {
                self.focus = Focus::Advanced;
                Event::Redraw
            }
            Hit::Bar(h) => self.press_bar(lay, h, x),
            _ => self.lit(changed),
        }
    }

    /// The pointer moved: a held thumb follows it, and the held scrollbar
    /// thumb scrolls the columns.
    pub fn motion(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        self.point = Some((x, y));
        let shown = |i: usize| lay.columns.get(i).and_then(Option::as_ref);
        let moved = match self.ptr.held() {
            Some(Part::Volume(i)) => match shown(i) {
                Some(cl) => {
                    let pos = trackbar::position_at(&cl.volume, y);
                    self.slide_volume(i, |t| t.set(pos))
                }
                None => Event::None,
            },
            Some(Part::Balance(i)) => match shown(i) {
                Some(cl) => {
                    let pos = trackbar::position_at(&cl.balance, x);
                    self.slide_balance(i, |t| t.set(pos))
                }
                None => Event::None,
            },
            Some(Part::Bar(ScrollHit::Thumb)) => match lay.bar {
                Some(bar) => {
                    let p = bar_parts(self, lay, bar);
                    let to =
                        scrollbar_pos_at(&p, x - self.grab, lay.shown, self.columns.len(), true);
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

    /// The button came up: on the check box it held, the box flips; on
    /// the Advanced button, it is pressed.
    pub fn release(&mut self) -> Event {
        let before = self.ptr.lit();
        match self.ptr.release() {
            Some(Part::Check(i)) => self.flip(i),
            Some(Part::Advanced) => Event::Advanced,
            _ => self.lit_from(before),
        }
    }

    pub fn leave(&mut self) -> Event {
        self.point = None;
        let changed = self.ptr.leave();
        self.lit(changed)
    }

    /// The wheel turned `rows` rows at (`x`, `y`): over the scrollbar it
    /// scrolls a column per notch; over a column it steps that column's
    /// volume a position per notch (down is quieter).
    pub fn scroll(&mut self, lay: &Layout, x: i32, y: i32, rows: i32) -> Event {
        let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
        let i = match hit(self, lay, x, y) {
            Hit::Balance(i, _) | Hit::Volume(i, _) | Hit::Check(i) | Hit::Column(i) => i,
            Hit::Bar(_) => return self.scroll_bar(lay, notches),
            _ => return Event::None,
        };
        let ev = self.slide_volume(i, |t| t.step(-notches));
        let lit = self.rehit(lay);
        match ev {
            Event::None => lit,
            ev => ev,
        }
    }

    /// Tab and Shift+Tab walk the focus order. On a volume trackbar Up /
    /// Right, Page Up and Home make it louder, Down / Left, Page Down and
    /// End quieter; on a balance trackbar Left / Up, Page Up and Home move
    /// it left, the others right. Space flips the focused check box and
    /// presses the focused Advanced button, as Enter does on it. A key on
    /// a column scrolled out of view scrolls it back first.
    pub fn key(&mut self, lay: &Layout, key: form::Key) -> Event {
        use form::Key as K;
        let first = !self.cues;
        self.cues = true;
        let scrolled = self.first;
        if !matches!(key, K::Tab { .. }) {
            self.reveal_focus(lay.shown);
        }
        let ev = match (key, self.focus) {
            (K::Tab { back }, _) => {
                self.walk(back, lay.shown);
                Event::Redraw
            }
            (K::Space, Focus::Check(i)) => self.flip(i),
            (K::Space | K::Enter, Focus::Advanced) if self.advanced_live() => Event::Advanced,
            (k, Focus::Volume(i)) => match k {
                K::Up | K::Right => self.slide_volume(i, |t| t.step(1)),
                K::Down | K::Left => self.slide_volume(i, |t| t.step(-1)),
                K::PageUp => self.slide_volume(i, |t| t.step(t.page as i32)),
                K::PageDown => self.slide_volume(i, |t| t.step(-(t.page as i32))),
                K::Home => self.slide_volume(i, |t| t.set(STEPS - 1)),
                K::End => self.slide_volume(i, |t| t.set(0)),
                _ => Event::None,
            },
            (k, Focus::Balance(i)) => match k {
                K::Left | K::Up => self.slide_balance(i, |t| t.step(-1)),
                K::Right | K::Down => self.slide_balance(i, |t| t.step(1)),
                K::PageUp => self.slide_balance(i, |t| t.step(-(t.page as i32))),
                K::PageDown => self.slide_balance(i, |t| t.step(t.page as i32)),
                K::Home => self.slide_balance(i, |t| t.set(0)),
                K::End => self.slide_balance(i, |t| t.set(BALANCE_STEPS - 1)),
                _ => Event::None,
            },
            _ => Event::None,
        };
        let lit = self.rehit(lay);
        match ev {
            Event::None if first || self.first != scrolled => Event::Redraw,
            Event::None => lit,
            ev => ev,
        }
    }

    /// The mixer's `access` part: a scroll view over the columns holding a group per column
    /// named by its title (one scrolled out lies beyond the view), each with the Balance and
    /// Volume sliders, their labels and the check box; then the Advanced button. Returns the
    /// focused node, or `parent` when there are no columns.
    pub fn access(&self, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
        let mut focus = parent;
        let placed = lay.columns.iter().zip(&lay.beyond);
        let view = (!self.columns.is_empty())
            .then(|| tree.push(parent, "columns", access::node(Role::ScrollView, lay.view)));
        for (i, (col, (shown, beyond))) in self.columns.iter().zip(placed).enumerate() {
            let (Some(view), Some(cl)) = (view, shown.as_ref().or(beyond.as_ref())) else {
                continue;
            };
            let mut g = access::node(Role::Group, cl.rect);
            g.set_label(col.title.as_str());
            let g = tree.push(view, ("column", col.key.as_str()), g);
            let bl = tree.push(
                g,
                "balance-label",
                access::label(cl.balance_label, BALANCE, col.balance.enabled),
            );
            let balance = trackbar::access(&col.balance, &cl.balance, tree, g, "balance");
            let vl = tree.push(
                g,
                "volume-label",
                access::label(cl.volume_label, VOLUME, col.live),
            );
            let volume = trackbar::access(&col.volume, &cl.volume, tree, g, "volume");
            for (slider, label) in [(balance, bl), (volume, vl)] {
                if let Some(n) = tree.get_mut(slider) {
                    n.push_labelled_by(label);
                }
            }
            let check = tree.push(
                g,
                "check",
                access::toggle(
                    Role::CheckBox,
                    cl.check.union(&cl.check_label),
                    col.check.label(),
                    col.checked.into(),
                    col.live,
                ),
            );
            match self.focus {
                Focus::Balance(f) if f == i => focus = balance,
                Focus::Volume(f) if f == i => focus = volume,
                Focus::Check(f) if f == i => focus = check,
                _ => {}
            }
        }
        if let Some(r) = lay.advanced {
            let live = self.first_live();
            let b = tree.push(parent, "advanced", access::button(r, ADVANCED, live, false));
            if self.focus == Focus::Advanced {
                focus = b;
            }
        }
        focus
    }
}

/// The balance position a balance of -1 to 1 sits at.
pub fn balance_pos(b: f64) -> usize {
    let half = BALANCE_CENTRE as f64;
    (half + b * half)
        .round()
        .clamp(0.0, (BALANCE_STEPS - 1) as f64) as usize
}

/// The balance, -1 to 1, of a balance position.
pub fn balance_of(pos: usize) -> f64 {
    let half = BALANCE_CENTRE as f64;
    ((pos as f64 - half) / half).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests;
