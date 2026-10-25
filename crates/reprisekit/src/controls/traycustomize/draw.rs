use crate::classic::widgets::{LINE_H, draw_focus_ring, draw_focus_ring_screen, draw_label};
use crate::controls::combo::{self, Combo, ComboLit};
use crate::controls::listview::{draw_group_header, list_rule};
use crate::controls::shellview::list_icon_px;
use crate::cues::Cues;
use crate::icon::IconSource;
use crate::look::Paint;
use crate::look::widgets::header as header_widgets;
use crate::look::widgets::{draw_button, draw_checkbox, draw_field_frame, draw_scrollbar};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::PageInks;
use crate::text::{self, Align, TextRenderer};
use crate::textedit::Caret;

use super::geometry::{
    CELL_X, GLASS, GROUP_TEXT_X, HEADER_TEXT_X, HEADING_SCALE, HIGHLIGHT_X, ICON_PX, ICON_X,
    NAME_X, PAGE_HEADING_LINE_H, PAGE_ICON_X, PAGE_NAME, PAGE_RULE_INSET, PAGE_RULE_Y, PAGE_TIP,
    PAGE_TITLE_X, rows_in,
};
use super::layout::Layout;
use super::{ALL_ICONS, CURRENT, Focus, PAST, Slot, TrayCustomize, Wording};
use crate::controls::label_rect;

/// `sources` drawn `px` square: the smallest at least that big, else the largest.
pub(super) fn icon_at(sources: &[IconSource], px: i32) -> Option<Canvas> {
    let fits = |src: &&IconSource| src.width as i32 >= px;
    sources
        .iter()
        .filter(fits)
        .min_by_key(|src| src.width)
        .or_else(|| sources.iter().max_by_key(|src| src.width))
        .map(|src| src.to_canvas(px))
}

impl TrayCustomize {
    /// Draw the client area at `p`'s scale. `lay` is measured at the
    /// device font size `px`. `cues` says whether the focus rectangles
    /// and access-key underlines show; an inactive window (`p.active`
    /// false) draws neither the dots nor a default ring.
    pub fn draw(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        cues: Cues,
        px: u32,
    ) {
        if self.is_page() {
            let inks = p.look.style().and_then(|st| st.page);
            let inks = inks.unwrap_or(PageInks::LIGHT);
            return self.draw_page(c, text, p, &inks, lay, cues, px);
        }
        let Paint { cs, s, active, .. } = *p;
        let dots = cues.focus && active;
        let w = self.wording;
        let g = self.geometry();
        let live = self.list_enabled();
        c.fill_rect(lay.client.scaled(s), cs.button_face);
        let lines = text::wrap(text, w.intro(), lay.intro.width * s, px);
        let line_h = self.metrics.text.len(LINE_H);
        let pitch = lay.intro.height / rows_in(lay.intro.height, line_h);
        for (i, line) in lines.iter().enumerate() {
            let r = Rect::new(
                lay.intro.x,
                lay.intro.y + i as i32 * pitch,
                lay.intro.width,
                pitch,
            );
            text.draw(c, r.scaled(s), line, false, cs.button_text, Align::Left, px);
        }
        let prompt = lay.prompt.scaled(s);
        text.draw(
            c,
            prompt,
            w.prompt(),
            false,
            cs.button_text,
            Align::Left,
            px,
        );

        draw_field_frame(c, p, lay.frame.scaled(s), cs.window);
        let body = lay.body.scaled(s);
        c.with_clip(body, |c| self.draw_rows(c, text, p, lay, dots, px));
        let inner = lay.inner.scaled(s);
        c.with_clip(inner, |c| {
            let titles = w.columns();
            let hr = lay.header.scaled(s);
            for (i, r) in lay.columns.iter().enumerate() {
                let t = header_widgets::draw_thin_item(c, p, r.scaled(s), Interaction::Idle);
                let tr = Rect::new(
                    t.x + HEADER_TEXT_X * s,
                    hr.y + g.header_under * s,
                    (t.width - (2 * HEADER_TEXT_X) * s).max(0),
                    hr.height - g.header_under * s,
                );
                let title = text.ellipsize(titles[i], tr.width, false, px);
                text.draw(c, tr, &title, false, cs.button_text, Align::Left, px);
            }
            let end = lay.columns[1].right();
            let filler = Rect::new(
                end,
                lay.header.y,
                lay.inner.right() - end + 2,
                lay.header.height,
            );
            header_widgets::draw_thin_item(c, p, filler.scaled(s), Interaction::Idle);
        });
        if let Some(sb) = lay.vbar {
            let total = self.slots().len();
            draw_scrollbar(c, p, sb.scaled(s), self.top, lay.page, total, None);
        }
        if let (Some((_, cl)), true) = (&lay.combo, live) {
            let focused = self.focus == Focus::Combo && active;
            c.with_clip(body, |c| {
                combo::draw(
                    c,
                    text,
                    p,
                    &self.combo,
                    cl,
                    focused,
                    focused && dots,
                    Caret::Hidden,
                    ComboLit::IDLE,
                    px,
                );
            });
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label) {
            let (b, lr) = (b.scaled(s), label_rect(l, s));
            let label = cues.label(ALL_ICONS);
            let on = self.all_icons == Some(true);
            draw_checkbox(c, p, b, on, self.pointer.of(Focus::AllIcons));
            draw_label(
                c,
                text,
                lr,
                &label,
                false,
                cs.button_text,
                Align::Left,
                px,
                s,
            );
            if self.focus == Focus::AllIcons && dots {
                let ring = Rect::new(lr.x - s, lr.y - s, lr.width + 2 * s, lr.height + 2 * s);
                draw_focus_ring(c, ring, s, cs.button_text);
            }
        }
        let ring = |f: Focus| {
            active
                && match self.focus {
                    Focus::Restore | Focus::Cancel => f == self.focus,
                    _ => f == Focus::Ok,
                }
        };
        for (r, label, enabled, f) in [
            (lay.restore, &*cues.label(w.restore()), live, Focus::Restore),
            (lay.ok, "OK", true, Focus::Ok),
            (lay.cancel, "Cancel", true, Focus::Cancel),
        ] {
            draw_button(
                c,
                text,
                p,
                r.scaled(s),
                label,
                ring(f),
                self.pointer.of(f),
                enabled,
                dots && self.focus == f,
                px,
            );
        }
        self.draw_open_list(c, text, p, lay, px);
    }

    /// Glass's page: white down to the band's rule, the heading, the text
    /// and the framed list with every row's combo, the link, the box and
    /// the band with OK and Cancel. A hot combo, link, box or button draws
    /// hot; the box ticked greys the combos and the link.
    #[allow(clippy::too_many_arguments)]
    fn draw_page(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        inks: &PageInks,
        lay: &Layout,
        cues: Cues,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        c.fill_rect(lay.client.scaled(s), cs.window);
        c.with_clip(lay.view.scaled(s), |c| {
            self.draw_page_parts(c, text, p, inks, lay, cues, px)
        });
        if let Some(bar) = lay.page_bar {
            let g = self.page_geo();
            let (top, visible, total) = (g.top as usize, g.view_h() as usize, g.content_h());
            let lit = self.page_scroll.lit();
            draw_scrollbar(c, p, bar.scaled(s), top, visible, total as usize, lit);
        }
        if let Some(band) = lay.band {
            let band = band.scaled(s);
            c.fill_rect(band, cs.button_face);
            c.fill_rect(Rect::new(band.x, band.y, band.width, s), inks.rule);
        }
        let dots = cues.focus && active;
        for (r, label, f) in [
            (lay.ok, "OK", Focus::Ok),
            (lay.cancel, "Cancel", Focus::Cancel),
        ] {
            let default = active
                && match self.focus {
                    Focus::Cancel => f == Focus::Cancel,
                    _ => f == Focus::Ok,
                };
            let ptr = self.pointer.of(f);
            draw_button(
                c,
                text,
                p,
                r.scaled(s),
                label,
                default,
                ptr,
                true,
                dots && self.focus == f,
                px,
            );
        }
        self.draw_open_list(c, text, p, lay, px);
    }

    /// The page's parts above the band, in its view: the heading, the text, the list, the link
    /// and the box.
    #[allow(clippy::too_many_arguments)]
    fn draw_page_parts(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        inks: &PageInks,
        lay: &Layout,
        cues: Cues,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let dots = cues.focus && active;
        let w = self.wording;
        let live = self.list_enabled();
        let heading_px = px * HEADING_SCALE.0 / HEADING_SCALE.1;
        let u = self.metrics.text;
        let pitch = u.len(PAGE_HEADING_LINE_H);
        let one = [Wording::HEADING];
        let lines: Vec<&str> = match self.heading.as_slice() {
            [] => one.to_vec(),
            lines => lines.iter().map(String::as_str).collect(),
        };
        for (k, line) in lines.into_iter().enumerate() {
            let p = lay.prompt;
            let first = p.height - (self.heading.len().max(1) as i32 - 1) * pitch;
            let r = Rect::new(p.x, p.y + k as i32 * pitch, p.width, first);
            text.draw(
                c,
                r.scaled(s),
                line,
                false,
                inks.heading,
                Align::Left,
                heading_px,
            );
        }
        let intro = lay.intro.scaled(s);
        if self.intro.is_empty() {
            text.draw(c, intro, w.intro(), false, cs.window_text, Align::Left, px);
        }
        let first = self.metrics.text.rect(GLASS.intro).height * s;
        let pitch = self.metrics.text.len(LINE_H) * s;
        for (k, line) in self.intro.iter().enumerate() {
            let r = Rect::new(intro.x, intro.y + k as i32 * pitch, intro.width, first);
            text.draw(c, r, line, false, cs.window_text, Align::Left, px);
        }

        let frame = lay.frame.scaled(s);
        c.fill_rect(frame, inks.list_frame);
        c.fill_rect(lay.inner.scaled(s), cs.window);
        for (r, title) in lay.columns.iter().zip(w.columns()) {
            if r.width == 0 {
                continue;
            }
            let dx = self.metrics.text.len(PAGE_TITLE_X);
            let tr = Rect::new(r.x + dx, r.y, r.width - dx, r.height);
            text.draw(
                c,
                tr.scaled(s),
                title,
                true,
                cs.window_text,
                Align::Left,
                px,
            );
        }
        let rule_x = lay.columns[0].x;
        let rule_end = lay.body.right() - u.len(PAGE_RULE_INSET);
        let rule = Rect::new(
            rule_x,
            lay.frame.y + u.len(PAGE_RULE_Y),
            rule_end + 1 - rule_x,
            1,
        );
        c.fill_rect(rule.scaled(s), inks.rule);
        let body = lay.body.intersect(&lay.view).scaled(s);
        c.with_clip(body, |c| {
            for &(slot, r) in &lay.slots {
                let Slot::Row(i) = slot else { continue };
                self.draw_page_row(c, text, p, inks, lay, i, r, dots, px);
            }
        });
        if let Some(sb) = lay.vbar {
            let total = self.slots().len();
            draw_scrollbar(c, p, sb.scaled(s), self.top, lay.page, total, None);
        }

        let link = label_rect(lay.restore, s);
        let lit = self.pointer.of(Focus::Restore);
        let ink = match (live, lit) {
            (false, _) => inks.detail,
            (true, Interaction::Idle) => inks.link,
            (true, _) => inks.link_hot,
        };
        text.draw(c, link, w.restore(), false, ink, Align::Left, px);
        if live
            && lit != Interaction::Idle
            && let Some(base) = text.baseline(link, false, px)
        {
            c.fill_rect(Rect::new(link.x, base + s, link.width, s), ink);
        }
        if self.focus == Focus::Restore && dots {
            let ring = Rect::new(link.x - s, link.y, link.width + 2 * s, link.height);
            draw_focus_ring(c, ring, s, cs.window_text);
        }
        if let (Some(b), Some(l)) = (lay.check, lay.check_label) {
            let (b, lr) = (b.scaled(s), label_rect(l, s));
            let on = self.all_icons == Some(true);
            draw_checkbox(c, p, b, on, self.pointer.of(Focus::AllIcons));
            let label = cues.label(ALL_ICONS);
            draw_label(
                c,
                text,
                lr,
                &label,
                false,
                cs.window_text,
                Align::Left,
                px,
                s,
            );
            if self.focus == Focus::AllIcons && dots {
                let ring = Rect::new(lr.x - s, lr.y - s, lr.width + 2 * s, lr.height + 2 * s);
                draw_focus_ring(c, ring, s, cs.window_text);
            }
        }
    }

    /// Page row `i` in its slot `r` (logical px): the icon, the name over
    /// the tip, and the row's own combo, lit as the pointer has it.
    #[allow(clippy::too_many_arguments)]
    fn draw_page_row(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        inks: &PageInks,
        lay: &Layout,
        i: usize,
        r: Rect,
        dots: bool,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let w = self.wording;
        let e = &self.entries[i];
        let x0 = lay.frame.x;
        let u = self.metrics.text;
        let icon = list_icon_px(u);
        if let Some(ic) = icon_at(&e.icon, icon * s) {
            let dy = u.len(PAGE_NAME.y) + (u.len(PAGE_NAME.height) - icon) / 2;
            c.blit_over((x0 + u.len(PAGE_ICON_X)) * s, (r.y + dy) * s, &ic);
        }
        let name_w = self.page_geo().name_w();
        for (b, line, ink, face) in [
            (PAGE_NAME, &e.name, cs.window_text, text),
            (PAGE_TIP, &e.detail, inks.detail, text.italic()),
        ] {
            let b = u.rect(Rect { width: name_w, ..b });
            let tr = Rect::new(x0 + b.x, r.y + b.y, b.width, b.height).scaled(s);
            let shown = face.ellipsize(line, tr.width, false, px);
            face.draw(c, tr, &shown, false, ink, Align::Left, px);
        }
        let Some(&(_, field)) = lay.fields.iter().find(|(j, _)| *j == i) else {
            return;
        };
        let mine = self.selected == Some(i);
        let row_combo;
        let (combo, cl) = match &lay.combo {
            Some((j, cl)) if *j == i => (&self.combo, *cl),
            _ => {
                let labels: Vec<&str> = w.choices().iter().map(|&b| w.label(b)).collect();
                row_combo = Combo::new(&labels, w.index(e.behaviour, e.passive), labels.len());
                (&row_combo, combo::layout(&row_combo, field, &self.metrics))
            }
        };
        let greyed;
        let combo = if self.list_enabled() {
            combo
        } else {
            greyed = Combo {
                enabled: false,
                open: None,
                ..combo.clone()
            };
            &greyed
        };
        let frame = if mine && combo.open.is_some() {
            Interaction::Pressed
        } else if self.hot_row == Some(i) && combo.enabled {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        let focused = mine && self.focus == Focus::Combo && active;
        let lit = ComboLit {
            frame,
            drop: Interaction::Idle,
        };
        combo::draw(
            c,
            text,
            p,
            combo,
            &cl,
            focused,
            focused && dots,
            Caret::Hidden,
            lit,
            px,
        );
    }

    /// The rows and group headers in view, clipped to the body.
    fn draw_rows(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        dots: bool,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let w = self.wording;
        let g = self.geometry();
        let live = self.list_enabled();
        c.fill_rect(lay.body.scaled(s), cs.window);
        let ink = if live { cs.window_text } else { cs.gray_text };
        let titles = [CURRENT, PAST];
        for &(slot, r) in &lay.slots {
            let i = match slot {
                Slot::Header(grp) => {
                    let gap = r.height - g.group_h;
                    let top = r.y + gap + g.rule_y - crate::controls::listview::GROUP_RULE_Y;
                    let hr = Rect::new(r.x, top, r.width, g.group_h).scaled(s);
                    draw_group_header(c, text, cs, hr, "", (0, 0), &list_rule(cs), px, s);
                    let tr = Rect::new(
                        r.x + GROUP_TEXT_X,
                        r.y + gap,
                        r.width - GROUP_TEXT_X,
                        g.rule_y + 1,
                    );
                    text.draw(c, tr.scaled(s), titles[grp], true, ink, Align::Left, px);
                    continue;
                }
                Slot::Row(i) => i,
            };
            let e = &self.entries[i];
            let row = r.scaled(s);
            let icon = list_icon_px(self.metrics.text);
            let grew = icon - ICON_PX;
            if let Some(ic) = icon_at(&e.icon, icon * s) {
                c.blit_over(row.x + ICON_X * s, row.y + g.icon_dy * s, &ic);
            }
            let name_x = NAME_X + grew;
            let name_r = Rect::new(
                row.x + name_x * s,
                row.y + g.text_dy * s,
                (g.columns[0] - name_x - 4) * s,
                g.highlight_h * s,
            );
            let name = text.ellipsize(&e.name, name_r.width, false, px);
            let selected = self.selected == Some(i) && live;
            let focused = active && self.focus == Focus::List;
            let mut name_ink = ink;
            if selected {
                let end = row.x + (g.columns[0] + 1) * s;
                let hx = row.x + (HIGHLIGHT_X + grew) * s;
                let hl = Rect::new(hx, row.y, end - hx, g.highlight_h * s);
                c.fill_rect(hl, cs.hilight);
                name_ink = cs.hilight_text;
                if focused && dots {
                    draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
                }
            }
            text.draw(c, name_r, &name, false, name_ink, Align::Left, px);
            let bx = row.x + (g.columns[0] + CELL_X) * s;
            let br = Rect::new(
                bx,
                row.y + g.text_dy * s,
                (g.columns[1] - 2 * CELL_X) * s,
                g.highlight_h * s,
            );
            let label = w.label(w.choices()[w.index(e.behaviour, e.passive)]);
            let label = text.ellipsize(label, br.width, false, px);
            text.draw(c, br, &label, false, ink, Align::Left, px);
        }
    }

    /// The selected row's combo list, while open, over everything else.
    fn draw_open_list(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        px: u32,
    ) {
        if let (Some((_, cl)), Some(open)) = (&lay.combo, self.combo.open) {
            combo::draw_list(
                c,
                text,
                p,
                &self.combo.items,
                Some(open),
                self.combo.top,
                self.combo.rows,
                cl.list,
                cl.scrollbar,
                None,
                px,
            );
        }
    }
}
