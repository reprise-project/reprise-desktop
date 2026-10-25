//! Where the parts go at the text size, and fitting the page to its room.

use crate::classic::widgets::LINE_H;
use crate::controls::scrollbar::PageScroll;
use crate::controls::shellview::list_icon_px;
use crate::raster::Rect;
use crate::text::{self, TextRenderer, split_mnemonic};
use crate::theme::TextUnits;

use super::{Focus, TrayCustomize, Wording};

/// Where an interface puts the parts, from the client area's top-left.
#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub(super) client: (i32, i32),
    pub(super) intro: Rect,
    pub(super) prompt: Rect,
    pub(super) list: Rect,
    /// The Name and Behavior columns' widths.
    pub(super) columns: [i32; 2],
    /// How far the header starts above the frame's inner edge: the rows
    /// that hide under the frame.
    pub(super) header_under: i32,
    /// A group header's slot, and what every group after the first has
    /// above it besides.
    pub(super) group_h: i32,
    pub(super) group_gap: i32,
    /// The rule's row in a group's slot.
    pub(super) rule_y: i32,
    pub(super) row_h: i32,
    /// The highlight's and the focus rectangle's height from the row's top.
    pub(super) highlight_h: i32,
    /// The icon's and the text's box's offsets down the row.
    pub(super) icon_dy: i32,
    pub(super) text_dy: i32,
    pub(super) check: Option<(i32, i32)>,
    pub(super) restore: Rect,
    pub(super) ok: Rect,
    pub(super) cancel: Rect,
}

pub(super) const CLASSIC: Geometry = Geometry {
    client: (348, 391),
    intro: Rect::new(9, 11, 330, 3 * LINE_H),
    prompt: Rect::new(9, 66, 330, LINE_H),
    list: Rect::new(9, 82, 330, 208),
    columns: [185, 124],
    header_under: 1,
    group_h: 32,
    group_gap: 11,
    rule_y: 19,
    row_h: 18,
    highlight_h: 17,
    icon_dy: 0,
    text_dy: 1,
    check: None,
    restore: Rect::new(246, 307, 93, 23),
    ok: Rect::new(183, 359, 75, 23),
    cancel: Rect::new(264, 359, 75, 23),
};

/// The shortest Classic list a client with little room keeps: a group and three rows.
pub(super) const CLASSIC_LIST_MIN_H: i32 = 112;

impl Geometry {
    /// These parts with the intro `dy` taller: what lies under it moves down and the client
    /// grows by as much.
    fn pushed(&self, dy: i32) -> Geometry {
        let down = |r: Rect| Rect::new(r.x, r.y + dy, r.width, r.height);
        Geometry {
            client: (self.client.0, self.client.1 + dy),
            intro: Rect {
                height: self.intro.height + dy,
                ..self.intro
            },
            prompt: down(self.prompt),
            list: down(self.list),
            check: self.check.map(|(x, y)| (x, y + dy)),
            restore: down(self.restore),
            ok: down(self.ok),
            cancel: down(self.cancel),
            ..*self
        }
    }

    /// These parts with the list `dy` shorter: what lies under it moves up and the client
    /// shrinks by as much.
    fn shortened(&self, dy: i32) -> Geometry {
        let up = |r: Rect| Rect::new(r.x, r.y - dy, r.width, r.height);
        Geometry {
            client: (self.client.0, self.client.1 - dy),
            list: Rect {
                height: self.list.height - dy,
                ..self.list
            },
            restore: up(self.restore),
            ok: up(self.ok),
            cancel: up(self.cancel),
            ..*self
        }
    }

    /// These parts for text grown by `u`: rects, rows and columns grow, the icon grows with
    /// them and keeps its place down the taller row, and the header's hidden row keeps its px.
    fn grown(&self, u: TextUnits) -> Geometry {
        let row_h = u.len(self.row_h);
        Geometry {
            client: u.size(self.client),
            intro: u.rect(self.intro),
            prompt: u.rect(self.prompt),
            list: u.rect(self.list),
            columns: self.columns.map(|w| u.len(w)),
            group_h: u.len(self.group_h),
            group_gap: u.len(self.group_gap),
            rule_y: u.len(self.rule_y),
            row_h,
            highlight_h: u.len(self.highlight_h),
            icon_dy: self.icon_dy + (row_h - self.row_h - (list_icon_px(u) - ICON_PX)) / 2,
            check: self.check.map(|(x, y)| (u.len(x), u.len(y))),
            restore: u.rect(self.restore),
            ok: u.rect(self.ok),
            cancel: u.rect(self.cancel),
            ..*self
        }
    }
}

/// Glass's page. Its rows' box starts `PAGE_BODY_Y` into the list and
/// holds no group headers; `prompt` is the heading's box and `restore`
/// the link's. The column, header and highlight fields go unused.
pub(super) const GLASS: Geometry = Geometry {
    client: (1184, 602),
    intro: Rect::new(92, 52, 1000, 20),
    prompt: Rect::new(92, 16, 1000, 26),
    list: Rect::new(92, 84, 1000, 403),
    columns: [PAGE_BEHAVIOURS_X, 279],
    header_under: 0,
    group_h: 0,
    group_gap: 0,
    rule_y: 0,
    row_h: 48,
    highlight_h: 0,
    icon_dy: 5,
    text_dy: 0,
    check: Some((92, 528)),
    restore: Rect::new(92, 497, 0, 20),
    ok: Rect::new(934, 569, 76, 23),
    cancel: Rect::new(1016, 569, 76, 23),
};

/// Glass's page, from the list's top-left: the rows' box's top, the
/// column titles' box, the rule's row and its ends (from 32 px into the
/// rows' box to 33 px short of the bar), where Behaviors starts and how
/// far each title sits right of its column's start.
pub(super) const PAGE_BODY_Y: i32 = 48;

pub(super) const PAGE_TITLES: Rect = Rect::new(33, 16, 0, 20);

pub(super) const PAGE_TITLE_X: i32 = 2;

pub(super) const PAGE_RULE_Y: i32 = 34;

pub(super) const PAGE_RULE_INSET: i32 = 33;

pub(super) const PAGE_BEHAVIOURS_X: i32 = 492;

/// A page row's parts from its top-left: the icon's left edge (centred down the name's box),
/// the name's and the tip's boxes, and the combo.
pub(super) const PAGE_ICON_X: i32 = 41;

pub(super) const PAGE_NAME: Rect = Rect::new(67, 4, 400, 18);

pub(super) const PAGE_TIP: Rect = Rect::new(67, 19, 400, 18);

/// The page's check box label, from the box's left edge, and a pixel up.
pub(super) const PAGE_CHECK_LABEL: i32 = 19;

pub(super) const PAGE_COMBO: Rect = Rect::new(PAGE_BEHAVIOURS_X, 8, 279, 23);

/// The separator over the button band, from the client's top, and the band's height.
pub(super) const PAGE_BAND_Y: i32 = 559;

pub(super) const PAGE_BAND_H: i32 = 43;

/// The page's side margins, and how far OK's and Cancel's left edges sit in from its right.
pub(super) const PAGE_MARGIN: i32 = 92;

pub(super) const PAGE_OK_X: i32 = 250;

pub(super) const PAGE_CANCEL_X: i32 = 168;

/// What a narrowed page keeps right of a combo, and between a name and the combo.
pub(super) const PAGE_COMBO_RIGHT: i32 = 50;

pub(super) const PAGE_NAME_GAP: i32 = 25;

/// The narrowest page with full margins and combos: a name column 100 px wide. A page
/// narrower still takes its margins in to `PAGE_NARROW_MARGIN`, then its combos to
/// `PAGE_NARROW_COMBO`, down to `PAGE_MIN_W`.
pub(super) const PAGE_WIDE_MIN_W: i32 =
    2 * PAGE_MARGIN + PAGE_NAME.x + 100 + PAGE_NAME_GAP + PAGE_COMBO.width + PAGE_COMBO_RIGHT;

pub(super) const PAGE_NARROW_MARGIN: i32 = 24;

pub(super) const PAGE_NARROW_COMBO: i32 = 200;

pub(super) const PAGE_MIN_W: i32 = PAGE_WIDE_MIN_W
    - 2 * (PAGE_MARGIN - PAGE_NARROW_MARGIN)
    - (PAGE_COMBO.width - PAGE_NARROW_COMBO);

/// A page narrower than `PAGE_MIN_W` stacks each row: the combo goes under the name and tip,
/// `PAGE_STACKED_COMBO_Y` down a row `PAGE_STACKED_ROW_H` tall, and the name takes the width.
pub(super) const PAGE_STACKED_COMBO_Y: i32 = 42;

pub(super) const PAGE_STACKED_ROW_H: i32 = 73;

pub(super) const PAGE_STACKED_MIN_W: i32 =
    2 * PAGE_NARROW_MARGIN + PAGE_NAME.x + PAGE_NARROW_COMBO + PAGE_COMBO_RIGHT;

/// The pitch of a heading wrapped onto more lines than one.
pub(super) const PAGE_HEADING_LINE_H: i32 = 21;

/// The heading's size over the body text's: 16 px to 12.
pub(super) const HEADING_SCALE: (u32, u32) = (4, 3);

/// A row's icon and text, from the body's left edge, at 100% text; the name and the highlight
/// move over by what the icon grows.
pub(super) const ICON_X: i32 = 4;

pub(super) const NAME_X: i32 = 22;

pub(super) const HIGHLIGHT_X: i32 = 20;

/// The behaviour text in from its column's left edge.
pub(super) const CELL_X: i32 = 8;

/// Header titles in from their column button's left edge.
pub(super) const HEADER_TEXT_X: i32 = 8;

/// A group's title in from the body's left edge, bold.
pub(super) const GROUP_TEXT_X: i32 = 15;

pub(super) const ICON_PX: i32 = 16;

/// Glass's page at the text size: 100% widths after the room, how much the heading's extra
/// lines push the intro down (`head_dy`), how much both push what lies under the intro down
/// (`dy`) and how far the page is scrolled.
#[derive(Clone, Copy)]
pub(super) struct PageGeo {
    pub(super) u: TextUnits,
    width: i32,
    head_dy: i32,
    dy: i32,
    pub(super) top: i32,
    pub(super) short: i32,
}

impl PageGeo {
    pub(super) fn stacked(&self) -> bool {
        self.width < PAGE_MIN_W
    }

    pub(super) fn row_h(&self) -> i32 {
        if self.stacked() {
            PAGE_STACKED_ROW_H
        } else {
            GLASS.row_h
        }
    }

    /// A combo's top in its row.
    pub(super) fn combo_y(&self) -> i32 {
        if self.stacked() {
            PAGE_STACKED_COMBO_Y
        } else {
            PAGE_COMBO.y
        }
    }

    /// How far the page is narrower than [`PAGE_WIDE_MIN_W`].
    fn squeeze(&self) -> i32 {
        (PAGE_WIDE_MIN_W - self.width).max(0)
    }

    /// The left and right margins: in from `PAGE_MARGIN` once the page is narrower than
    /// [`PAGE_WIDE_MIN_W`].
    fn margin(&self) -> i32 {
        PAGE_MARGIN - (self.squeeze() / 2).min(PAGE_MARGIN - PAGE_NARROW_MARGIN)
    }

    /// The combos' width: narrower once the margins are at their least.
    pub(super) fn combo_w(&self) -> i32 {
        let left = self.squeeze() - 2 * (PAGE_MARGIN - self.margin());
        PAGE_COMBO.width - left.clamp(0, PAGE_COMBO.width - PAGE_NARROW_COMBO)
    }

    fn list_w(&self) -> i32 {
        self.width - 2 * self.margin()
    }

    /// Where the Behaviors column starts in the list: in from the right as the list narrows.
    /// A stacked page's combos sit under the names.
    pub(super) fn behaviours_x(&self) -> i32 {
        if self.stacked() {
            return PAGE_NAME.x;
        }
        PAGE_BEHAVIOURS_X.min(self.list_w() - self.combo_w() - PAGE_COMBO_RIGHT)
    }

    pub(super) fn name_w(&self) -> i32 {
        let right = if self.stacked() {
            self.list_w() - PAGE_COMBO_RIGHT
        } else {
            self.behaviours_x() - PAGE_NAME_GAP
        };
        PAGE_NAME.width.min(right - PAGE_NAME.x)
    }

    /// The page's parts down to the band, 100% tall.
    pub(super) fn content_h(&self) -> i32 {
        PAGE_BAND_Y + self.dy
    }

    /// How much of them shows above the band.
    pub(super) fn view_h(&self) -> i32 {
        self.content_h() - self.short
    }

    /// `r`, laid out at 100% from the page's top-left with the parts under the intro `dy`
    /// further down, grown and scrolled.
    pub(super) fn at(&self, r: Rect) -> Rect {
        let g = self.u.rect(r);
        Rect::new(g.x, g.y - self.u.len(self.top), g.width, g.height)
    }

    pub(super) fn list(&self) -> Rect {
        Rect::new(
            self.margin(),
            GLASS.list.y + self.dy,
            self.list_w(),
            GLASS.list.height,
        )
    }

    pub(super) fn prompt(&self) -> Rect {
        let r = GLASS.prompt;
        Rect::new(self.margin(), r.y, self.list_w(), r.height + self.head_dy)
    }

    pub(super) fn intro(&self, lines: i32) -> Rect {
        let r = GLASS.intro;
        Rect::new(
            self.margin(),
            r.y + self.head_dy,
            self.list_w(),
            r.height + (lines - 1) * LINE_H,
        )
    }

    pub(super) fn restore(&self) -> Rect {
        let r = GLASS.restore;
        Rect::new(self.margin(), r.y + self.dy, r.width, r.height)
    }

    /// The check box's row: as tall as the box at 100%.
    pub(super) fn check(&self, check: i32) -> Rect {
        let (_, y) = GLASS.check.unwrap_or_default();
        Rect::new(self.margin(), y + self.dy, check, check)
    }

    /// The band and the part of the page above it, from the client's top-left.
    pub(super) fn band(&self) -> Rect {
        self.u
            .rect(Rect::new(0, self.view_h(), self.width, PAGE_BAND_H))
    }

    pub(super) fn view(&self) -> Rect {
        self.u.rect(Rect::new(0, 0, self.width, self.view_h()))
    }

    pub(super) fn button(&self, from_right: i32) -> Rect {
        let ok = GLASS.ok;
        let y = self.view_h() + ok.y - PAGE_BAND_Y;
        let x = self.width - from_right + PAGE_MARGIN - self.margin();
        self.u.rect(Rect::new(x, y, ok.width, ok.height))
    }
}

/// How many lines `line_h` apart a box `h` tall, grown from whole 100% lines, holds.
pub(super) fn rows_in(h: i32, line_h: i32) -> i32 {
    ((h + line_h / 2) / line_h).max(1)
}

impl TrayCustomize {
    /// Where the parts go, grown by `metrics.text`: Classic's after its intro's lines and
    /// Restore Defaults' label as [`TrayCustomize::fit`] wrapped and sized them.
    pub(super) fn geometry(&self) -> Geometry {
        let mut g = *self.wording.geometry();
        if !self.is_page() {
            let rows = (self.intro.len() as i32 - g.intro.height / LINE_H).max(0);
            g = g.pushed(rows * LINE_H);
            if let Some(w) = self.restore_w {
                let r = g.restore;
                g.restore = Rect::new(r.right() - w, r.y, w, r.height);
            }
            if let Some((_, h)) = self.room {
                let over = g.client.1 - self.metrics.text.within(h);
                g = g.shortened(over.clamp(0, g.list.height - CLASSIC_LIST_MIN_H));
            }
        } else {
            g.row_h = self.page_geo().row_h();
        }
        g.grown(self.metrics.text)
    }

    /// The page at the text size in [`TrayCustomize::room`], as [`TrayCustomize::fit`] left it.
    pub(super) fn page_geo(&self) -> PageGeo {
        let u = self.metrics.text;
        let full = GLASS.client.0;
        let width = self
            .room
            .map_or(full, |r| u.within(r.0).clamp(PAGE_STACKED_MIN_W, full));
        let lines = self.intro.len().max(1) as i32;
        let head_dy = (self.heading.len().max(1) as i32 - 1) * PAGE_HEADING_LINE_H;
        PageGeo {
            u,
            width,
            head_dy,
            dy: head_dy + (lines - 1) * LINE_H,
            top: self.page_scroll.top,
            short: self.page_scroll.short,
        }
    }

    /// Lay the parts out around their text at `px`. Classic's intro takes the lines it wraps
    /// onto and Restore Defaults the width its label needs. The page fits
    /// [`TrayCustomize::room`]: a narrower page wraps its heading and intro onto more lines, and
    /// one taller than the room keeps its band at the client's foot and scrolls the rest.
    /// Idempotent.
    pub fn fit(&mut self, text: &TextRenderer, px: u32) {
        self.intro.clear();
        self.heading.clear();
        self.restore_w = None;
        if !self.is_page() {
            let u = self.metrics.text;
            let width = u.len(CLASSIC.intro.width);
            self.intro = text::wrap(text, self.wording.intro(), width, px);
            let label = split_mnemonic(self.wording.restore()).0;
            let need = u.design(text.measure(&label, false, px) + u.len(6));
            self.restore_w = Some(need.max(CLASSIC.restore.width));
            return;
        }
        let g = self.page_geo();
        let width = g.u.len(g.list_w());
        let heading_px = px * HEADING_SCALE.0 / HEADING_SCALE.1;
        if text.measure(Wording::HEADING, false, heading_px) > width {
            self.heading = text::wrap(text, Wording::HEADING, width, heading_px);
        }
        let intro = self.wording.intro();
        if text.measure(intro, false, px) > width {
            self.intro = text::wrap(text, intro, width, px);
        }
        let g = self.page_geo();
        let full = g.content_h() + PAGE_BAND_H;
        let room = self.room.map_or(full, |r| g.u.within(r.1));
        self.page_scroll.short = (full - room).clamp(0, g.content_h() - LINE_H);
        self.scroll_page_to(self.page_scroll.top);
    }

    /// Scroll the page so `top` 100% px of it are above the client, held to its range. True
    /// when it moved.
    pub(super) fn scroll_page_to(&mut self, top: i32) -> bool {
        let top = top.clamp(0, self.page_scroll.short);
        std::mem::replace(&mut self.page_scroll.top, top) != top
    }

    /// The span the focused part takes down the page at 100%, unscrolled, if it scrolls.
    fn focus_span(&self) -> Option<(i32, i32)> {
        let g = self.page_geo();
        let r = match self.focus {
            Focus::Combo => {
                let k = self.selected?.checked_sub(self.top)? as i32;
                let y = g.list().y + PAGE_BODY_Y + k * g.row_h() + g.combo_y();
                Rect::new(0, y, 0, PAGE_COMBO.height)
            }
            Focus::AllIcons => g.check(self.metrics.check).inset(-1),
            Focus::Restore => g.restore(),
            Focus::List | Focus::Ok | Focus::Cancel => return None,
        };
        Some((r.y, r.bottom()))
    }

    /// Scroll the page so its focused part shows whole, as little as it takes. True when it
    /// moved.
    pub(super) fn reveal_focus(&mut self) -> bool {
        let Some(span) = self.focus_span().filter(|_| self.page_scroll.short > 0) else {
            return false;
        };
        let g = self.page_geo();
        let top = self.page_scroll.top;
        let by = PageScroll::reveal((top, top + g.view_h()), span);
        self.scroll_page_to(top + by)
    }

    pub fn client(&self) -> (i32, i32) {
        if self.is_page() {
            let g = self.page_geo();
            return (g.u.len(g.width), g.band().bottom());
        }
        self.geometry().client
    }

    /// The rows' box's height: the list's inside less the header's rows
    /// that show; on the page, the list's inside under its titles.
    pub fn body_height(&self) -> i32 {
        if self.is_page() {
            let g = self.page_geo();
            return g.u.rect(g.list()).height - g.u.len(PAGE_BODY_Y) - 1;
        }
        let g = self.geometry();
        g.list.height - 4 - (self.metrics.list_header - g.header_under)
    }
}
