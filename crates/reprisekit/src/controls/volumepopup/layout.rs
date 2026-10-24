use crate::classic::widgets::{ScrollHit, ScrollParts, scrollbar_hit, scrollbar_parts};
use crate::controls::combo;
use crate::flyout;
use crate::metrics::ControlMetrics;
use crate::raster::Rect;
use crate::taskbar::Edge;
use crate::text::{DialogFont, TextRenderer};
use crate::theme::{TextUnits, Theme};
use crate::trackbar;

use super::{
    BAR_GAP, COL_GAP, COL_H, COL_W, COMBO_GAP, COMBO_H, Column, DIVIDER_GAP, Dir, FIELD_W, ICON,
    INPUT, LABEL_GAP, LABEL_W, LEVEL_Y, MARGIN, MAX_SHOWN, MEDIA_GAP, MEDIA_H, MEDIA_W, MUTE_H,
    MUTE_W, MUTE_Y, NAME_Y, NowPlaying, OUTPUT, ROW_H, SEP_GAP, SEP_H, STRIP_H, THUMB_H,
    THUMB_WIDER, TRACK_H, TRACK_Y, Transport, VolumePopup,
};

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
pub(super) fn column_layout(
    col: &Column,
    cm: &ControlMetrics,
    u: TextUnits,
    x: i32,
    y: i32,
) -> ColumnLayout {
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

pub(super) fn strip_layout(np: &NowPlaying, u: TextUnits, x: i32, sy: i32, w: i32) -> StripLayout {
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

pub(super) fn bar_parts(vp: &VolumePopup, lay: &Layout, bar: Rect) -> ScrollParts {
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

impl Layout {
    pub(super) fn combo(&self, dir: Dir) -> &combo::Layout {
        match dir {
            Dir::Output => &self.output,
            Dir::Input => &self.input,
        }
    }
}
