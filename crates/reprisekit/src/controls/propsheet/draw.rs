use crate::controls::form;
use crate::controls::hosted::{Hosted, Spot};
use crate::controls::{pagelist, tabstrip};
use crate::look::Paint;
use crate::look::widgets::{draw_hscrollbar, draw_page, draw_scrollbar, group_label_ink};
use crate::raster::Canvas;
use crate::text::{Align, TextRenderer};

use super::{Nav, PropSheet};

/// Draw the sheet onto a canvas already filled with face: the page, the tabs (or
/// the list and the open page's name), then the open page's controls and buttons.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sheet: &PropSheet,
    cues: bool,
    px: u32,
) {
    let s = p.s;
    // The list's scrollbar is the look's width, which the layout reads from the
    // open page: keep the look on every page so a page opened next lays the list
    // out as drawn.
    for page in &sheet.pages {
        page.drawn_with(p);
    }
    sheet.open().lay_out_notes(text, s, px);
    let l = sheet.layout(text, px);
    let page = l.page.scaled(s);
    draw_page(c, p, page);
    let nav_text = text.dialog(sheet.font);
    match (&sheet.nav, &l.list) {
        (Nav::List(list), Some(lay)) => {
            pagelist::draw(c, nav_text, p, list, &sheet.tabs, lay, cues, px);
            if let Some(name) = sheet.tabs.labels.get(sheet.tabs.open) {
                let ink = group_label_ink(p);
                nav_text.draw(c, l.title.scaled(s), name, true, ink, Align::Left, px);
            }
        }
        _ => tabstrip::draw(c, nav_text, p, &sheet.tabs, &l.tabs, l.page, cues, px),
    }
    form::draw(c, text, &p.on_page(page), sheet.open(), cues, px);
    if let Some(bar) = l.bar {
        let (visible, total) = sheet.page_span();
        let top = sheet.scroll.top as usize;
        let lit = sheet.scroll.lit();
        draw_scrollbar(
            c,
            p,
            bar.scaled(s),
            top,
            visible as usize,
            total as usize,
            lit,
        );
    }
    if let Some(bar) = l.hbar {
        let (visible, total) = sheet.page_hspan();
        let left = sheet.hscroll.top as usize;
        let lit = sheet.hscroll.lit();
        draw_hscrollbar(
            c,
            p,
            bar.scaled(s),
            left,
            visible as usize,
            total as usize,
            lit,
        );
    }
}

pub fn draw_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, sheet: &PropSheet, px: u32) {
    let page = sheet.layout(text, px).page.scaled(p.s);
    form::draw_popups(c, text, &p.on_page(page), sheet.open(), px);
}

/// The open page's combo list or colour popup.
impl Hosted for PropSheet {
    fn spots(&self) -> Vec<Spot> {
        self.open().spots()
    }
    fn place(&mut self, level: usize, origin: (i32, i32)) {
        self.open_mut().place(level, origin);
    }
    fn shadow(&self, theme: &crate::theme::Theme) -> i32 {
        self.open().shadow(theme)
    }
    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &crate::theme::Theme,
        text: &TextRenderer,
    ) {
        self.open().draw_level(level, c, by, theme, text);
    }
    fn host(&mut self) {
        for page in &mut self.pages {
            page.host();
        }
    }
    fn dismiss(&mut self) {
        self.open_mut().dismiss();
    }
}
