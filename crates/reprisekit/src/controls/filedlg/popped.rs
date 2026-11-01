use super::{FileDlg, Layout, TOOLBAR_VIEWS};
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::controls::menu::submenu_anchor;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use std::rc::Rc;

/// The dialog's open Look in or Files of type list, its Views or context menu or an extra row's
/// open list,
/// for a host that shows it on a popup: `text` and `px` as the dialog is drawn with, in a client
/// at `(0, 0)`. Dismissing it closes it.
pub struct Popped<'a> {
    pub dlg: &'a mut FileDlg,
    pub text: Rc<TextRenderer>,
    pub px: u32,
}

#[derive(Clone, Copy)]
enum Open {
    Context,
    Views,
    LookIn,
    FileType,
    Extra,
}

impl Popped<'_> {
    fn open(&self) -> Option<Open> {
        let d = &self.dlg;
        if d.context.is_some() {
            Some(Open::Context)
        } else if d.views_open {
            Some(Open::Views)
        } else if d.lookin.open.is_some() {
            Some(Open::LookIn)
        } else if d.filetype.open.is_some() {
            Some(Open::FileType)
        } else if d.extra_list_open() {
            Some(Open::Extra)
        } else {
            None
        }
    }

    /// The extra rows' form and where its (0, 0) sits in the client: its rects and its list's
    /// stored place are its own, not the client's.
    fn extra(&self) -> Option<(&form::Form, (i32, i32))> {
        let (w, h) = self.dlg.size;
        let at = self.dlg.extra_origin(Rect::new(0, 0, w, h));
        self.dlg.extra.as_ref().map(|(f, _)| (f, at))
    }

    fn context_levels(&self) -> Vec<super::context::Level> {
        let (w, h) = self.dlg.size;
        let u = self.dlg.drawn.0.get().text;
        (self.dlg).context_levels(&self.text, self.px, Rect::new(0, 0, w, h), u)
    }

    fn layout(&self) -> Layout {
        let (w, h) = self.dlg.size;
        let cm = self.dlg.drawn.0.get();
        self.dlg
            .layout(&self.text, self.px, Rect::new(0, 0, w, h), &cm)
    }
}

impl Hosted for Popped<'_> {
    fn spots(&self) -> Vec<Spot> {
        let Some(open) = self.open() else {
            return Vec::new();
        };
        if let Open::Extra = open {
            return self.extra().map_or_else(Vec::new, |(f, (x, y))| {
                f.spots().into_iter().map(|s| s.moved(x, y)).collect()
            });
        }
        if let (Open::Context, Some(c)) = (open, &self.dlg.context) {
            let levels = self.context_levels();
            return levels
                .iter()
                .enumerate()
                .map(|(i, l)| Spot {
                    rect: l.rect,
                    anchor: match i {
                        0 => Anchor::Point(c.at.0, c.at.1),
                        _ => {
                            let row = c.sub.unwrap_or(0);
                            let (rect, dy) =
                                submenu_anchor(&levels[i - 1].layout, row).unwrap_or_default();
                            Anchor::Beside { rect, dy }
                        }
                    },
                })
                .collect();
        }
        let lay = self.layout();
        let spot = match (open, &lay.views) {
            (Open::Views, Some((rect, _))) => Spot {
                rect: *rect,
                anchor: Anchor::Below(lay.toolbar_rects[TOOLBAR_VIEWS]),
            },
            (Open::LookIn, _) => Spot {
                rect: lay.lookin.list,
                anchor: Anchor::Below(lay.lookin.field),
            },
            // Where the dialog's own draw shows it: over the field when the client ends below it.
            (Open::FileType, _) => Spot {
                rect: lay.filetype.list,
                anchor: Anchor::Point(lay.filetype_home.0, lay.filetype_home.1),
            },
            (Open::Views, None) | (Open::Extra | Open::Context, _) => return Vec::new(),
        };
        vec![spot]
    }

    fn place(&mut self, level: usize, origin: (i32, i32)) {
        if let (Some(Open::Context), Some(c)) = (self.open(), &mut self.dlg.context) {
            if let Some(at) = c.placed.get_mut(level) {
                *at = Some(origin);
            }
            return;
        }
        match self.open().filter(|_| level == 0) {
            Some(Open::Views) => self.dlg.views_at = Some(origin),
            Some(Open::LookIn) => self.dlg.lookin.list_at = Some(origin),
            Some(Open::FileType) => self.dlg.filetype.list_at = Some(origin),
            Some(Open::Context) | None => {}
            Some(Open::Extra) => {
                let (x, y) = self.extra().map_or((0, 0), |(_, at)| at);
                if let Some((f, _)) = &mut self.dlg.extra {
                    f.place(level, (origin.0 - x, origin.1 - y));
                }
            }
        }
    }

    fn shadow(&self, theme: &Theme) -> i32 {
        match self.open() {
            Some(Open::Views | Open::Context) => crate::controls::menu::level_shadow(theme),
            Some(Open::Extra) => self.extra().map_or(0, |(f, _)| f.shadow(theme)),
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
        if let Some(Open::Context) = self.open() {
            let levels = self.context_levels();
            let Some(l) = levels.get(level) else {
                return;
            };
            let keys = self.dlg.context.as_ref().is_some_and(|c| c.keys);
            let p = Paint {
                text: self.dlg.drawn.0.get().text,
                s: 1,
                ..theme.paint()
            };
            crate::controls::menu::draw_menu_at(
                c,
                (l.rect.x + by.0, l.rect.y + by.1),
                &Theme::for_draw(&p, self.px),
                text,
                &l.layout,
                &l.items,
                l.hot,
                "",
                theme.access_keys || keys,
            );
            return;
        }
        if let Some(Open::Extra) = self.open() {
            if let Some((f, (x, y))) = self.extra() {
                f.draw_level(level, c, (by.0 + x, by.1 + y), theme, text);
            }
            return;
        }
        let Some(open) = self.open().filter(|_| level == 0) else {
            return;
        };
        let lay = self.layout();
        // As the dialog's own draw painted it.
        let p = Paint {
            text: self.dlg.drawn.0.get().text,
            s: 1,
            ..theme.paint()
        };
        let moved = |r: Rect| Rect::new(r.x + by.0, r.y + by.1, r.width, r.height);
        let list = |c: &mut Canvas, combo: &Combo, l: &combo::Layout| {
            combo::draw_list(
                c,
                text,
                &p,
                &combo.items,
                combo.open,
                combo.top,
                combo.rows,
                moved(l.list),
                l.scrollbar.map(moved),
                None,
                self.px,
            );
        };
        match open {
            Open::LookIn => list(c, &self.dlg.lookin, &lay.lookin),
            Open::FileType => list(c, &self.dlg.filetype, &lay.filetype),
            Open::Extra | Open::Context => {}
            Open::Views => {
                if let Some((at, menu)) = &lay.views {
                    let at = moved(*at);
                    crate::controls::menu::draw_menu_at(
                        c,
                        (at.x, at.y),
                        &Theme::for_draw(&p, self.px),
                        text,
                        menu,
                        &self.dlg.views_menu_items(),
                        None,
                        "",
                        false,
                    );
                }
            }
        }
    }

    fn host(&mut self) {
        self.dlg.hosted = true;
        if let Some((f, _)) = &mut self.dlg.extra {
            f.host();
        }
    }

    fn dismiss(&mut self) {
        self.dlg.context = None;
        self.dlg.views_open = false;
        self.dlg.lookin.open = None;
        self.dlg.filetype.open = None;
        if let Some((f, _)) = &mut self.dlg.extra {
            f.dismiss();
        }
    }
}
