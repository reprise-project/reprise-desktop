use crate::classic::widgets::{LINE_H, bar_hit, scrollbar_parts};
use crate::controls::combo;
use crate::raster::Rect;
use crate::text::{self, TextRenderer, split_mnemonic};

use super::geometry::{
    HEADING_SCALE, PAGE_BODY_Y, PAGE_CANCEL_X, PAGE_CHECK_LABEL, PAGE_COMBO, PAGE_OK_X,
    PAGE_RULE_INSET, PAGE_TITLE_X, PAGE_TITLES, rows_in,
};
use super::{ALL_ICONS, Hit, Slot, TrayCustomize, Wording};

impl TrayCustomize {
    /// The parts with the client area's top-left at `origin`. Label
    /// widths are measured at the device font size `px`.
    pub fn layout(&self, text: &TextRenderer, px: u32, origin: (i32, i32)) -> Layout {
        if self.is_page() {
            return self.page_layout(text, px, origin);
        }
        let g = self.geometry();
        let cm = &self.metrics;
        let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
        let frame = at(g.list);
        let inner = frame.inset(2);
        let header_h = cm.list_header - g.header_under;
        let full = Rect::new(
            inner.x,
            inner.y + header_h,
            inner.width,
            inner.height - header_h,
        );
        let slots = self.slots();
        let content: i32 = (0..slots.len()).map(|k| self.slot_h(&slots, k)).sum();
        let vbar = content > full.height;
        let body = Rect::new(
            full.x,
            full.y,
            full.width - if vbar { cm.scroll } else { 0 },
            full.height,
        );
        let header = Rect::new(
            inner.x,
            inner.y - g.header_under,
            body.width,
            cm.list_header,
        );
        let mut columns = [Rect::default(); 2];
        let mut x = body.x;
        for (i, w) in g.columns.into_iter().enumerate() {
            let lead = if i == 0 { g.header_under } else { 0 };
            columns[i] = Rect::new(x - lead, header.y, w + lead, header.height);
            x += w;
        }
        let top = self.top.min(slots.len());
        let mut placed = Vec::new();
        let mut y = body.y;
        for k in top..slots.len() {
            if y >= body.bottom() {
                break;
            }
            let h = self.slot_h(&slots, k);
            placed.push((slots[k], Rect::new(body.x, y, body.width, h)));
            y += h;
        }
        let combo = self
            .selected
            .filter(|_| self.list_enabled())
            .and_then(|i| placed.iter().find(|(s, _)| *s == Slot::Row(i)))
            .map(|&(Slot::Row(i) | Slot::Header(i), r)| {
                let combo_h = cm.text.len(combo::COMBO_H);
                let field = Rect::new(body.x + g.columns[0] + 1, r.y, g.columns[1], combo_h);
                (i, combo::layout(&self.combo, field, cm))
            });
        let (check, check_label) = match g.check {
            Some((cx, cy)) => {
                let b = at(Rect::new(cx, cy, cm.check, cm.check));
                let (plain, _) = split_mnemonic(ALL_ICONS);
                let l = Rect::new(
                    b.x + cm.check_label,
                    b.y,
                    text.measure(&plain, false, px),
                    cm.check,
                );
                (Some(b), Some(l))
            }
            None => (None, None),
        };
        Layout {
            client: at(Rect::new(0, 0, g.client.0, g.client.1)),
            intro: at(g.intro),
            prompt: at(g.prompt),
            frame,
            inner,
            header,
            columns,
            body,
            vbar: vbar.then(|| Rect::new(body.right(), inner.y, cm.scroll, inner.height)),
            page: self.fits_from(&slots, top, body.height),
            slots: placed,
            combo,
            check,
            check_label,
            restore: at(g.restore),
            ok: at(g.ok),
            cancel: at(g.cancel),
            thumb_min: cm.thumb_min,
            fields: Vec::new(),
            band: None,
            view: at(Rect::new(0, 0, g.client.0, g.client.1)),
            page_bar: None,
        }
    }

    /// Glass's page: every row in view carries its combo's field, the bar
    /// is always there, and Restore is a link as wide as its words. Above the
    /// band the parts scroll with the page.
    fn page_layout(&self, text: &TextRenderer, px: u32, origin: (i32, i32)) -> Layout {
        let g = self.page_geo();
        let u = g.u;
        let cm = &self.metrics;
        let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
        let on_page = |r: Rect| at(g.at(r));
        let frame = on_page(g.list());
        let inner = frame.inset(1);
        let vbar = Rect::new(inner.right() - cm.scroll, inner.y, cm.scroll, inner.height);
        let body_y = frame.y + u.len(PAGE_BODY_Y);
        let body = Rect::new(inner.x, body_y, vbar.x - inner.x, inner.bottom() - body_y);
        let titles = u.rect(PAGE_TITLES);
        let rule_inset = u.len(PAGE_RULE_INSET);
        let header = Rect::new(
            frame.x + titles.x,
            frame.y + titles.y,
            vbar.x - rule_inset - frame.x - titles.x,
            titles.height,
        );
        let behaviours = frame.x + u.len(g.behaviours_x());
        // A stacked page has no Behaviors column: its first title takes the header.
        let split = if g.stacked() {
            header.right()
        } else {
            behaviours
        };
        let columns = [
            Rect::new(header.x, header.y, split - header.x, header.height),
            Rect::new(split, header.y, header.right() - split, header.height),
        ];
        let row_h = u.len(g.row_h());
        let field = u.rect(Rect {
            y: g.combo_y(),
            width: g.combo_w(),
            ..PAGE_COMBO
        });
        let slots = self.slots();
        let top = self.top.min(slots.len());
        let mut placed = Vec::new();
        let mut fields = Vec::new();
        let mut y = body.y;
        for &slot in &slots[top..] {
            if y >= body.bottom() {
                break;
            }
            let r = Rect::new(body.x, y, body.width, row_h);
            if let Slot::Row(i) = slot {
                let f = Rect::new(behaviours, y + field.y, field.width, field.height);
                fields.push((i, f));
            }
            placed.push((slot, r));
            y += row_h;
        }
        let combo = self
            .selected
            .and_then(|i| fields.iter().find(|(j, _)| *j == i))
            .map(|&(i, f)| (i, combo::layout(&self.combo, f, cm)));
        let row = on_page(g.check(cm.check));
        let check = Rect::new(
            row.x,
            row.y + (row.height - cm.check) / 2,
            cm.check,
            cm.check,
        );
        let (plain, _) = split_mnemonic(ALL_ICONS);
        let check_label = Rect::new(
            check.x + PAGE_CHECK_LABEL,
            row.y - 1,
            text.measure(&plain, false, px),
            row.height,
        );
        let link = on_page(g.restore());
        let restore = Rect::new(
            link.x,
            link.y,
            text.measure(self.wording.restore(), false, px),
            link.height,
        );
        let view = at(g.view());
        let page_bar = (g.short > 0)
            .then(|| Rect::new(view.right() - cm.scroll, view.y, cm.scroll, view.height));
        Layout {
            client: at(Rect::new(0, 0, view.width, g.band().bottom())),
            intro: on_page(g.intro(self.intro.len().max(1) as i32)),
            prompt: on_page(g.prompt()),
            frame,
            inner,
            header,
            columns,
            body,
            vbar: Some(vbar),
            page: self.fits_from(&slots, top, body.height),
            slots: placed,
            combo,
            check: Some(check),
            check_label: Some(check_label),
            restore,
            ok: at(g.button(PAGE_OK_X)),
            cancel: at(g.button(PAGE_CANCEL_X)),
            thumb_min: cm.thumb_min,
            fields,
            band: Some(at(g.band())),
            view,
            page_bar,
        }
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
        if let Some((_, cl)) = &lay.combo
            && let Some(h) = combo::hit(&self.combo, cl, x, y)
            && (matches!(h, combo::Hit::Row(_)) || h.bar().is_some() || lay.view.contains(x, y))
        {
            return Some(Hit::Combo(h));
        }
        if let Some(bar) = lay.page_bar.filter(|b| b.contains(x, y)) {
            let g = self.page_geo();
            let parts = self
                .page_scroll
                .parts(bar, g.view_h(), g.content_h(), lay.thumb_min);
            return Some(Hit::PageBar(bar_hit(&parts, x, y, false)));
        }
        for (r, h) in [(lay.ok, Hit::Ok), (lay.cancel, Hit::Cancel)] {
            if r.contains(x, y) {
                return Some(h);
            }
        }
        if !lay.view.contains(x, y) {
            return None;
        }
        if lay.restore.contains(x, y) {
            return Some(Hit::Restore);
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label)
            && (b.contains(x, y) || l.contains(x, y))
        {
            return Some(Hit::AllIcons);
        }
        if let Some(sb) = lay.vbar.filter(|sb| sb.contains(x, y)) {
            let total = self.slots().len();
            let parts = scrollbar_parts(sb, 1, self.top, lay.page, total, false, lay.thumb_min);
            return Some(Hit::Bar(bar_hit(&parts, x, y, false)));
        }
        if self.is_page() && lay.body.contains(x, y) {
            let row = lay
                .fields
                .iter()
                .find_map(|&(i, f)| f.contains(x, y).then_some(i));
            return Some(row.map_or(Hit::List, Hit::Row));
        }
        if lay.body.contains(x, y) {
            let row = lay.slots.iter().find_map(|&(s, r)| match s {
                Slot::Row(i) if r.contains(x, y) => Some(i),
                _ => None,
            });
            return Some(row.map_or(Hit::List, Hit::Row));
        }
        lay.frame.contains(x, y).then_some(Hit::List)
    }

    /// The texts that run past their boxes with text at `px`, by name and how many px over, for
    /// an audit: the page's run to the client's right edge at most, a button's label to its
    /// sides less 3 px each.
    pub fn overflows(&self, text: &TextRenderer, px: u32) -> Vec<(&'static str, i32)> {
        let lay = self.layout(text, px, (0, 0));
        let w = self.wording;
        let u = self.metrics.text;
        let width = |s: &str, bold: bool, px: u32| text.measure(&split_mnemonic(s).0, bold, px);
        let mut over = Vec::new();
        let mut put = |name: &'static str, by: i32| {
            if by > 0 {
                over.push((name, by));
            }
        };
        let button = |label: &str, r: Rect| width(label, false, px) - (r.width - u.len(6));
        put("ok", button("OK", lay.ok));
        put("cancel", button("Cancel", lay.cancel));
        if self.is_page() {
            let heading_px = px * HEADING_SCALE.0 / HEADING_SCALE.1;
            let heading = match self.heading.as_slice() {
                [] => width(Wording::HEADING, false, heading_px),
                lines => lines
                    .iter()
                    .map(|l| width(l, false, heading_px))
                    .max()
                    .unwrap_or(0),
            };
            put("heading", heading - lay.prompt.width);
            let intro = match self.intro.as_slice() {
                [] => width(w.intro(), false, px),
                lines => lines.iter().map(|l| width(l, false, px)).max().unwrap_or(0),
            };
            put("intro", intro - lay.intro.width);
            let right = lay.client.right();
            put(
                "all icons",
                lay.check_label.map_or(0, |l| l.right() - right),
            );
            put("restore", lay.restore.right() - right);
            for (r, title) in lay.columns.iter().zip(w.columns()) {
                if r.width == 0 {
                    continue;
                }
                let room = r.width - u.len(PAGE_TITLE_X);
                put("column", width(title, true, px) - room);
            }
            return over;
        }
        let line_h = u.len(LINE_H);
        let lines = text::wrap(text, w.intro(), lay.intro.width, px).len() as i32;
        put(
            "intro",
            (lines - rows_in(lay.intro.height, line_h)) * line_h,
        );
        put("prompt", width(w.prompt(), false, px) - lay.prompt.width);
        put("restore", button(w.restore(), lay.restore));
        let check = lay
            .check_label
            .map_or(0, |l| l.right() - lay.client.right());
        put("all icons", check);
        over
    }

    /// Every entry's row, those scrolled out of the body included, laid end to end from the
    /// first row in view.
    pub(super) fn row_rects(&self, lay: &Layout) -> Vec<Option<Rect>> {
        let mut out = vec![None; self.entries.len()];
        let slots = self.slots();
        let Some(&(first, anchor)) = lay.slots.iter().find(|(s, _)| matches!(s, Slot::Row(_)))
        else {
            return out;
        };
        let Some(k0) = slots.iter().position(|&s| s == first) else {
            return out;
        };
        let mut place = |k: usize, y: i32| {
            if let Slot::Row(i) = slots[k] {
                out[i] = Some(Rect::new(anchor.x, y, anchor.width, anchor.height));
            }
        };
        let mut y = anchor.y;
        for k in k0..slots.len() {
            place(k, y);
            y += self.slot_h(&slots, k);
        }
        let mut y = anchor.y;
        for k in (0..k0).rev() {
            y -= self.slot_h(&slots, k);
            place(k, y);
        }
        out
    }
}

/// Where the parts go, in logical px.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client: Rect,
    pub intro: Rect,
    pub prompt: Rect,
    /// The list's sunken frame and what is inside it.
    pub frame: Rect,
    pub inner: Rect,
    pub header: Rect,
    /// The Name and Behavior column buttons.
    pub columns: [Rect; 2],
    /// The rows' box under the header, beside the bar.
    pub body: Rect,
    pub vbar: Option<Rect>,
    /// How many slots from `top` show whole: a page.
    pub page: usize,
    /// The slots in view with their rects; the last may run past the body.
    pub slots: Vec<(Slot, Rect)>,
    /// The selected entry's combo, when its row is in view.
    pub combo: Option<(usize, combo::Layout)>,
    pub check: Option<Rect>,
    pub check_label: Option<Rect>,
    pub restore: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    pub thumb_min: i32,
    /// Each page row in view and its combo's field; empty off the page.
    pub fields: Vec<(usize, Rect)>,
    /// The page's button band under its rule.
    pub band: Option<Rect>,
    /// What shows of the parts that scroll: the client above the page's band.
    pub view: Rect,
    /// The page's scrollbar down the view's right edge, while the page scrolls.
    pub page_bar: Option<Rect>,
}
