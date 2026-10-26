use super::{
    CANCEL, CANCEL_DY, FIELD_BUTTON_GAP, FIELD_X, FILENAME_H, FILENAME_LABEL, FILENAME_Y,
    FILETYPE_H, FILETYPE_LABEL, FILETYPE_Y, FileDlg, GRIP, LABEL_DY, LOOKIN_H, LOOKIN_ICON_GAP,
    LOOKIN_ICON_PX, LOOKIN_ICON_X, LOOKIN_LABEL, LOOKIN_LABEL_X, LOOKIN_X, LOOKIN_Y, Labels,
    Layout, MARGIN, PLACES_H, ROW_LABEL_X, SHELL_H, SHELL_X, SHELL_Y, TOOLBAR_GAP, TOOLBAR_MARGIN,
    TOOLBAR_VIEWS, TOOLBAR_Y, key_around, rows, views_items,
};
use crate::classic::widgets::{self, BUTTON_H, BUTTON_W};
use crate::controls::combo;
use crate::controls::form;
use crate::controls::menu::{ContextBitmaps, MenuItem, MenuLayout, MenuStyle};
use crate::controls::metrics::ControlMetrics;
use crate::controls::placesbar;
use crate::controls::shellview;
use crate::controls::toolbar;
use crate::raster::Rect;
use crate::text::{TextRenderer, access_key, split_mnemonic};
use crate::theme::{TextUnits, Theme};
use std::borrow::Cow;
use std::rc::Rc;

/// Moves `lay`'s open list and its scroll bar so the list's corner is at `to`.
fn move_list(lay: &mut combo::Layout, to: (i32, i32)) {
    let (dx, dy) = (to.0 - lay.list.x, to.1 - lay.list.y);
    lay.list.x += dx;
    lay.list.y += dy;
    if let Some(sb) = &mut lay.scrollbar {
        sb.x += dx;
        sb.y += dy;
    }
}

impl FileDlg {
    /// File name's text area in `r`, in logical px.
    pub(super) fn name_inner(&self, text: &TextRenderer, px: u32, r: Rect) -> Rect {
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        lay.filename.inset(widgets::FIELD_PAD)
    }

    pub(super) fn layout(
        &self,
        text: &TextRenderer,
        px: u32,
        r: Rect,
        cm: &ControlMetrics,
    ) -> Layout {
        let client = r;
        let dy = self.grow(r, cm);
        let r = rows(r, cm);
        // 100% lengths at the text size.
        let u = |v: i32| cm.text.len(v);
        let label_h = px as i32 + 2;
        let lookin_label_w = text.measure(&split_mnemonic(LOOKIN_LABEL).0, false, px);
        let lookin_label = Rect::new(
            r.x + u(LOOKIN_LABEL_X),
            r.y + u(LOOKIN_Y + LABEL_DY) - 1,
            lookin_label_w,
            label_h,
        );
        let lookin_h = u(LOOKIN_H);
        let toolbar_h = toolbar::height(false, cm);
        let toolbar_y = r.y + u(TOOLBAR_Y) + (lookin_h - toolbar_h) / 2;

        // An item's width does not depend on the rect, so one pass at x 0
        // gives the natural width, and the rects shift into place after it.
        let mut toolbar_rects = toolbar::layout(
            &self.toolbar,
            text,
            px,
            Rect::new(0, toolbar_y, 0, toolbar_h),
            cm,
        );
        let toolbar_w = toolbar_rects.last().map_or(0, |b| b.right());
        let toolbar_x = r.right() - u(TOOLBAR_MARGIN) - toolbar_w;
        for b in &mut toolbar_rects {
            b.x += toolbar_x;
        }
        let toolbar = Rect::new(toolbar_x, toolbar_y, toolbar_w, toolbar_h);

        let lookin_x = r.x + u(LOOKIN_X);
        let lookin_w = (toolbar_x - u(TOOLBAR_GAP) - lookin_x).max(0);
        let lookin_field = Rect::new(lookin_x, r.y + u(LOOKIN_Y), lookin_w, lookin_h);
        let list_right = client.right() - u(MARGIN);
        let mut lookin = combo::layout(&self.lookin, lookin_field, cm);
        move_list(&mut lookin, (lookin_field.x, lookin_field.bottom()));
        combo::widen_list(&self.lookin, &mut lookin, text, px, list_right);
        if let Some(at) = self.lookin.list_at {
            move_list(&mut lookin, at);
        }
        if self.crumb_icon().is_some() {
            let shift = u(LOOKIN_ICON_X + LOOKIN_ICON_PX + LOOKIN_ICON_GAP) - 2;
            lookin.text.x += shift;
            lookin.text.width -= shift;
            lookin.highlight.x += shift;
            lookin.highlight.width -= shift;
        }

        let places = Rect::new(
            r.x + u(MARGIN),
            r.y + u(SHELL_Y),
            u(MARGIN + placesbar::WIDTH) - u(MARGIN),
            u(SHELL_Y + PLACES_H) - u(SHELL_Y) + dy,
        );
        let places_rects = placesbar::layout(&self.places, places, &cm.places);

        let label_w = text
            .measure(&split_mnemonic(FILENAME_LABEL).0, false, px)
            .max(text.measure(&split_mnemonic(FILETYPE_LABEL).0, false, px));
        let field_x = r.x + u(FIELD_X);
        // Accept and Cancel share a width, grown past 75 px for a label that needs it.
        let label_need = [&*self.accept_label(), CANCEL]
            .map(|l| text.measure(&split_mnemonic(l).0, false, px) + u(6));
        let button_w = label_need.into_iter().fold(u(BUTTON_W), i32::max);
        let button_h = u(BUTTON_H);
        let accept_x = r.right() - u(MARGIN) - button_w;
        let field_w = (accept_x - u(FIELD_BUTTON_GAP) - field_x).max(0);

        let filename_y = r.y + dy + u(FILENAME_Y);
        let filename_label = Rect::new(
            r.x + u(ROW_LABEL_X),
            filename_y + u(LABEL_DY),
            label_w,
            label_h,
        );
        let filename = Rect::new(field_x, filename_y, field_w, u(FILENAME_H));
        let accept = Rect::new(accept_x, filename_y, button_w, button_h);

        // This label sits 2 px higher against its field than the other
        // two (its baseline 13 px below the field's top).
        let filetype_y = r.y + dy + u(FILETYPE_Y);
        let filetype_label = Rect::new(
            r.x + u(ROW_LABEL_X),
            filetype_y + u(LABEL_DY) - 2,
            label_w,
            label_h,
        );
        let filetype_field = Rect::new(field_x, filetype_y, field_w, u(FILETYPE_H));
        // The row sits near the client's bottom edge, and a list drawn
        // past it could not be clicked: it opens above the field instead.
        let mut filetype = combo::layout(&self.filetype, filetype_field, cm);
        move_list(&mut filetype, (filetype_field.x, filetype_field.bottom()));
        combo::widen_list(&self.filetype, &mut filetype, text, px, list_right);
        if filetype.list.bottom() > client.bottom() {
            let to = (filetype.list.x, filetype.field.y - filetype.list.height);
            move_list(&mut filetype, to);
        }
        let filetype_home = (filetype.list.x, filetype.list.y);
        if let Some(at) = self.filetype.list_at {
            move_list(&mut filetype, at);
        }
        let cancel = Rect::new(accept_x, filetype_y + CANCEL_DY, button_w, button_h);

        let views = self.views_open.then(|| {
            let anchor = toolbar_rects[TOOLBAR_VIEWS];
            let menu = self.views_menu(text, px, r.width, cm.text);
            let (x, y) = self.views_at.unwrap_or((anchor.x, anchor.bottom()));
            (Rect::new(x, y, menu.width, menu.height), menu)
        });

        let shell = self.shell_layout(client, cm);

        Layout {
            lookin_label,
            lookin,
            toolbar,
            toolbar_rects,
            places,
            places_rects,
            filename_label,
            filename,
            accept,
            filetype_label,
            filetype,
            filetype_home,
            cancel,
            views,
            shell,
            grip: Rect::new(client.right() - GRIP, client.bottom() - GRIP, GRIP, GRIP),
        }
    }

    pub(super) fn shell_layout(&self, r: Rect, cm: &ControlMetrics) -> Rc<shellview::Layout> {
        let dy = self.grow(r, cm);
        let r = rows(r, cm);
        let u = |v: i32| cm.text.len(v);
        let shell_w = (r.width - u(SHELL_X) - u(MARGIN)).max(0);
        let shell_h = u(SHELL_Y + SHELL_H) - u(SHELL_Y) + dy;
        let shell = Rect::new(r.x + u(SHELL_X), r.y + u(SHELL_Y), shell_w, shell_h);
        self.drawn.2.get(&self.shell, shell, cm)
    }

    pub(super) fn views_menu_items(&self) -> Vec<MenuItem> {
        views_items()
            .iter()
            .map(|(label, mode)| {
                MenuItem::entry(*label)
                    .radio(true)
                    .checked(*mode == self.shell.mode)
            })
            .collect()
    }

    fn views_menu(&self, text: &TextRenderer, px: u32, max_width: i32, u: TextUnits) -> MenuLayout {
        let items = self.views_menu_items();
        let mut theme = Theme::default();
        theme.fonts.size_px = u.px(theme.fonts.size_px);
        let style =
            MenuStyle::context(&theme, ContextBitmaps::None).fit(text, &items, px, max_width);
        MenuLayout::compute_styled(&style, &items, usize::MAX)
    }

    /// The labels and the accept button with their access keys. The caller's keys (the extra
    /// rows' and the accept button's own) win; each label keeps its letter unless one of those
    /// takes it, then moves to its first free one, and a keyless accept button takes its first
    /// letter left free.
    pub(super) fn labels(&self) -> Labels<'_> {
        let mut taken: Vec<char> = (self.extra.iter())
            .flat_map(|(f, _)| f.items.iter().filter_map(form::Item::mnemonic))
            .collect();
        let accept = &self.fields.accept_label;
        taken.extend(access_key(accept));
        let mut keyed = |label: &'static str| {
            let label = key_around(label, &taken);
            taken.extend(access_key(&label));
            label
        };
        let (lookin, filename, filetype) = (
            keyed(LOOKIN_LABEL),
            keyed(FILENAME_LABEL),
            keyed(FILETYPE_LABEL),
        );
        Labels {
            lookin,
            filename,
            filetype,
            accept: match access_key(accept) {
                Some(_) => accept.into(),
                None => key_around(accept, &taken),
            },
        }
    }

    pub(super) fn accept_label(&self) -> Cow<'_, str> {
        self.labels().accept
    }
}
