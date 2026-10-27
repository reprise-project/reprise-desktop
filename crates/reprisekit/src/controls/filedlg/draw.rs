use super::{CLIENT, FileDlg, Focus, MARGIN, cancel_label, lookin_icon};
use crate::classic::widgets::{self, edit_first_visible};
use crate::controls::combo;
use crate::controls::form;
use crate::controls::placesbar;
use crate::controls::shellview;
use crate::controls::toolbar;
use crate::look::Paint;
use crate::look::widgets::{draw_button, draw_edit_field, draw_edit_text, draw_label};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::Caret;
use crate::theme::Theme;

impl FileDlg {
    /// Draws the client area into `r`; the title bar is the host's chrome.
    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, r: Rect, px: u32) {
        let Paint { cs, s, .. } = *p;
        self.drawn_with(p);
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        let labels = self.labels();
        c.fill_rect(r.scaled(s), cs.button_face);

        draw_label(
            c,
            text,
            p,
            lay.lookin_label.scaled(s),
            &labels.lookin,
            cs.button_text,
            Align::Left,
            px,
        );
        combo::draw(
            c,
            text,
            p,
            &self.lookin,
            &lay.lookin,
            self.focus == Focus::LookIn,
            true,
            Caret::Unfocused,
            combo::ComboLit::IDLE,
            px,
        );
        if let Some(icon) = self.crumb_icon() {
            let slot = lookin_icon(lay.lookin.field, self.drawn.0.get().text).scaled(s);
            c.blit_over(
                slot.x + (slot.width - icon.width as i32).max(0) / 2,
                slot.y + (slot.height - icon.height as i32).max(0) / 2,
                icon,
            );
        }

        toolbar::draw(
            c,
            text,
            p,
            &self.toolbar,
            lay.toolbar,
            &lay.toolbar_rects,
            px,
        );

        placesbar::draw(c, text, p, &self.places, lay.places, px);

        shellview::draw(c, text, p, &self.shell, &lay.shell, None, px);

        draw_label(
            c,
            text,
            p,
            lay.filename_label.scaled(s),
            &labels.filename,
            cs.button_text,
            Align::Left,
            px,
        );
        // Its frame shows the focus; the field tracks no pointer.
        let focused = self.focus == Focus::FileName;
        let inner = draw_edit_field(
            c,
            p,
            lay.filename.scaled(s),
            cs.window,
            focused,
            Interaction::Idle,
        );
        let first = match self.name_drop {
            Some((_, first)) => first.min(self.filename.text.len()),
            None => edit_first_visible(text, &self.filename, inner.width, px, 0),
        };
        draw_edit_text(
            c,
            text,
            &self.filename,
            inner,
            px,
            p,
            Caret::new(self.focus == Focus::FileName, self.name_drop.is_none()),
            first,
        );
        if let Some((x, _)) = self.name_drop {
            let tail = &self.filename.text[first..];
            let at = first + crate::textedit::index_at(text, tail, x * s - inner.x, px);
            widgets::draw_edit_drop_point(c, text, &self.filename, inner, px, p, first, at);
        }
        draw_button(
            c,
            text,
            p,
            lay.accept.scaled(s),
            &labels.accept,
            true,
            Interaction::Idle,
            true,
            self.focus == Focus::Accept,
            px,
        );

        if self.has_types() {
            draw_label(
                c,
                text,
                p,
                lay.filetype_label.scaled(s),
                &labels.filetype,
                cs.button_text,
                Align::Left,
                px,
            );
            combo::draw(
                c,
                text,
                p,
                &self.filetype,
                &lay.filetype,
                self.focus == Focus::FileType,
                true,
                Caret::Unfocused,
                combo::ComboLit::IDLE,
                px,
            );
        }
        draw_button(
            c,
            text,
            p,
            lay.cancel.scaled(s),
            &cancel_label(),
            false,
            Interaction::Idle,
            true,
            self.focus == Focus::Cancel,
            px,
        );

        if let Some((f, _)) = &self.extra {
            let mut shifted = f.placed_at(self.extra_origin(r));
            if self.focus != Focus::Extra {
                shifted.set_focus(None);
            }
            form::draw(c, text, p, &shifted, true, px);
            f.note_draw(&shifted);
        }
        if self.lookin.open.is_some() && !self.hosted {
            combo::draw_list(
                c,
                text,
                p,
                &self.lookin.items,
                self.lookin.open,
                self.lookin.top,
                self.lookin.rows,
                lay.lookin.list,
                lay.lookin.scrollbar,
                None,
                px,
            );
        }
        if self.filetype.open.is_some() && !self.hosted {
            combo::draw_list(
                c,
                text,
                p,
                &self.filetype.items,
                self.filetype.open,
                self.filetype.top,
                self.filetype.rows,
                lay.filetype.list,
                lay.filetype.scrollbar,
                None,
                px,
            );
        }

        crate::look::widgets::draw_size_grip(c, p, lay.grip.scaled(s));

        if let Some((at, menu)) = lay.views.as_ref().filter(|_| !self.hosted) {
            crate::controls::menu::draw_menu_at(
                c,
                (at.x * s, at.y * s),
                &Theme::for_draw(p, px),
                text,
                &menu.scaled(s),
                &self.views_menu_items(),
                None,
                "",
                false,
            );
        }
    }

    /// The texts that run past their boxes in the client at its size with text at `px`, as the
    /// last [`FileDlg::draw`] or [`FileDlg::drawn_with`] laid it out, by name and how many px
    /// over, for an audit: a label into its field, a button's label to its sides less 3 px
    /// each, a place's lines past its tile, and the extra rows' [`form::Form::overflows`] and
    /// any of them past the client's right margin. The combos' values and rows, the places'
    /// lines and the extra check boxes' labels end in an ellipsis where they run out of room.
    pub fn overflows(&self, text: &TextRenderer, px: u32) -> Vec<(&'static str, i32)> {
        let cm = self.drawn.0.get();
        let u = cm.text;
        let client = Rect::new(0, 0, self.size.0, self.size.1);
        let lay = self.layout(text, px, client, &cm);
        let width = |s: &str| text.measure(&crate::text::split_mnemonic(s).0, false, px);
        let mut over = Vec::new();
        let mut put = |name: &'static str, by: i32| {
            if by > 0 {
                over.push((name, by));
            }
        };
        put(
            "look in label",
            lay.lookin_label.right() - lay.lookin.field.x,
        );
        put(
            "file name label",
            lay.filename_label.right() - lay.filename.x,
        );
        if self.has_types() {
            put(
                "files of type label",
                lay.filetype_label.right() - lay.filetype.field.x,
            );
        }
        let button = |label: &str, r: Rect| width(label) - (r.width - u.len(6));
        put("accept", button(&self.accept_label(), lay.accept));
        put("cancel", button(&cancel_label(), lay.cancel));
        put(
            "places",
            placesbar::overflow(&self.places, text, lay.places, &cm.places, px),
        );
        if let Some((f, _)) = &self.extra {
            let mut f = f.clone();
            f.origin = (0, 0);
            // The rows are laid out at 100% from the client's opening width.
            let right = u.len(CLIENT.0 - MARGIN) + client.width - u.len(CLIENT.0);
            let end = |id: &str| {
                let it = f.item(id)?;
                let r = f.rect(id)?;
                Some(match &it.control {
                    form::Control::Check { label, .. } => {
                        (r.x + cm.check_label + width(label)).min(r.right())
                    }
                    _ => r.right(),
                })
            };
            for (id, by) in f.overflows(text, px) {
                // A combo or check box that fills the room ellipsizes what is still wider.
                let cuts = matches!(
                    f.item(id).map(|it| &it.control),
                    Some(form::Control::Combo(_) | form::Control::Check { .. })
                );
                if !(cuts && f.rect(id).is_some_and(|r| r.right() >= right)) {
                    over.push((id, by));
                }
            }
            for it in &f.items {
                if let Some(by) = end(it.id).map(|e| e - right).filter(|&by| by > 0) {
                    over.push((it.id, by));
                }
            }
        }
        over
    }

    /// Lays the dialog out with `p`'s look and text size from here on, as [`FileDlg::draw`]
    /// does: for `hit`, `access` and [`FileDlg::overflows`] before a draw.
    pub fn drawn_with(&self, p: &Paint) {
        self.drawn.0.set(p.look.controls().grown(p.text));
        self.drawn.1.set(p.look.lights_pointer());
        if let Some((f, _)) = &self.extra {
            f.drawn_with(p);
        }
    }
}
