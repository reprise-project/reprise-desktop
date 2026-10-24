use crate::controls::colorbutton::{self, ColorButton};
use crate::controls::combo;
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

use super::{Control, Form};

/// The open combo's list or colour button's popup; dismissing it closes it.
impl Hosted for Form {
    fn spots(&self) -> Vec<Spot> {
        let Some(i) = self.open_combo() else {
            return Vec::new();
        };
        let it = &self.items[i];
        let field = self.at(it);
        match &it.control {
            Control::Combo(c) => vec![Spot {
                rect: combo::layout(c, field, &self.controls()).list,
                anchor: Anchor::Below(field),
            }],
            Control::Color(b) => vec![Spot {
                rect: colorbutton::popup_rect(field, b),
                anchor: Anchor::Below(colorbutton::popup_anchor(field)),
            }],
            _ => Vec::new(),
        }
    }

    fn place(&mut self, level: usize, origin: (i32, i32)) {
        let Some(i) = self.open_combo().filter(|_| level == 0) else {
            return;
        };
        match &mut self.items[i].control {
            Control::Combo(c) => c.list_at = Some(origin),
            Control::Color(b) => b.at = Some(origin),
            _ => {}
        }
    }

    fn shadow(&self, theme: &Theme) -> i32 {
        match self.open_combo().map(|i| &self.items[i].control) {
            Some(Control::Color(_)) => crate::controls::menu::level_shadow(theme),
            _ => combo::list_shadow(theme),
        }
    }

    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &Theme,
        text: &TextRenderer,
    ) {
        let Some(i) = self.open_combo().filter(|_| level == 0) else {
            return;
        };
        let it = &self.items[i];
        // As the form's own draw painted it.
        let p = Paint {
            text: self.rendered.text.get(),
            s: self.rendered.scale.get().max(1),
            ..theme.paint()
        };
        let moved = |r: Rect| Rect::new(r.x + by.0, r.y + by.1, r.width, r.height);
        let px = match self.rendered.px.get() {
            0 => theme.fonts.size_px,
            px => px,
        };
        let text = text.dialog(self.font);
        match &it.control {
            Control::Combo(combo) => {
                let lay = combo::layout(combo, self.at(it), &self.controls());
                combo::draw_list(
                    c,
                    text,
                    &p,
                    &combo.items,
                    combo.open,
                    combo.top,
                    combo.rows,
                    moved(lay.list),
                    lay.scrollbar.map(moved),
                    self.bar_lit(i),
                    px,
                );
            }
            Control::Color(b) => {
                let b = ColorButton {
                    at: b.at.map(|(x, y)| (x + by.0, y + by.1)),
                    ..b.clone()
                };
                colorbutton::draw_popup(c, text, &p, moved(self.at(it)), &b, px);
            }
            _ => {}
        }
    }

    fn host(&mut self) {
        self.hosted = true;
    }

    fn dismiss(&mut self) {
        self.close_popups();
    }
}
