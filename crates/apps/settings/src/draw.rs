use crate::registry::Page;
use crate::{
    ADD_ITEM_WIN, ADD_WIN, CHOOSER_WIN, ERROR_WIN, LAYOUT_WIN, RENAME_WIN, SAVER_WIN, SHORTCUT_WIN,
    SettingsApp, defaultspage, displayhost, pages,
};
use repriseapp::oneshot;
use repriseapp::{Ctx, WinId};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::form;
use reprisekit::controls::listview;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};

impl SettingsApp {
    pub(crate) fn fit_columns(&mut self, cm: &ControlMetrics) {
        let a = self.area();
        let shortcuts = self.on_page(pages::shortcuts(a));
        let types = self.on_page(defaultspage::list(a));
        let locations = self.on_page(pages::locations(a));
        pages::fit_shortcut_columns(&mut self.shortcuts, cm, shortcuts);
        defaultspage::fit_columns(&mut self.types, cm, types);
        pages::fit_location_columns(&mut self.locations, cm, locations);
        // A Startup column the user dragged keeps its width until the list, its bar or the text
        // size changes.
        let list = self.on_page(pages::list(a));
        let fit = Some((list.width, cm.scroll, cm.text));
        if self.list_fit != fit {
            self.list_fit = fit;
            pages::fit_list_columns(&mut self.list, cm, list);
        }
    }

    pub(crate) fn paint(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.fit_columns(&ctx.forms().controls());
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        let form_px = forms.fonts.size_px;
        if win == ERROR_WIN {
            if let Some(b) = &self.error {
                b.draw(c, &self.dialog_theme(&ctx.theme), &self.dialog_text);
            }
            return;
        }
        if win == SHORTCUT_WIN {
            c.fill_rect(c.bounds(), cs.button_face);
            if let Some(d) = &self.shortcut {
                form::draw(
                    c,
                    &ctx.text,
                    &Paint {
                        s: 1,
                        ..forms.paint()
                    },
                    &d.form,
                    true,
                    form_px,
                );
            }
            return;
        }
        if matches!(win, LAYOUT_WIN | ADD_ITEM_WIN | RENAME_WIN | SAVER_WIN) {
            c.fill_rect(c.bounds(), cs.button_face);
            let dialog = match win {
                LAYOUT_WIN => self.add_layout.as_ref().map(|d| &d.form),
                ADD_ITEM_WIN => self.add_item.as_ref().map(|d| &d.form),
                SAVER_WIN => self.saver_dlg.as_ref().map(|d| &d.form),
                _ => self.rename.as_ref().map(|d| &d.form),
            };
            if let Some(f) = dialog {
                let p = Paint {
                    s: 1,
                    ..forms.paint()
                };
                form::draw(c, &ctx.text, &p, f, true, form_px);
            }
            return;
        }
        if win == displayhost::MONITOR_WIN {
            c.fill_rect(c.bounds(), cs.button_face);
            if let Some(q) = &self.display.flow.question {
                let p = Paint {
                    s: 1,
                    ..forms.paint()
                };
                form::draw(c, &ctx.text, &p, &q.dialog.form, true, form_px);
            }
            return;
        }
        if win == CHOOSER_WIN {
            if let Some(ch) = &self.chooser {
                let dt = self.dialog_theme(&ctx.theme);
                let lay = ch
                    .ow
                    .layout(&self.dialog_text, dt.fonts.size_px, (0, 0), &dt.controls());
                let rows: Vec<Option<&Canvas>> = ch.row_icons.iter().map(Option::as_ref).collect();
                ch.ow.draw(
                    c,
                    &self.dialog_text,
                    &Paint { s: 1, ..dt.paint() },
                    &lay,
                    ch.icon.as_ref(),
                    &rows,
                    ch.input.cues(),
                    dt.fonts.size_px,
                );
            }
            return;
        }
        if win == ADD_WIN {
            if let Some(d) = &self.add {
                d.draw(
                    c,
                    &ctx.text,
                    &Paint {
                        s: 1,
                        ..forms.paint()
                    },
                    Rect::new(0, 0, d.client().0, d.client().1),
                    form_px,
                );
            }
            return;
        }
        if self.sheet.tabs.open == Page::Pointers.index() {
            self.refresh_preview(cs.window);
        }
        c.fill_rect(c.bounds(), cs.button_face);
        propsheet::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            &self.sheet,
            true,
            form_px,
        );
        let open = self.sheet.tabs.open;
        let page = self.sheet.page_clip(&ctx.text, form_px);
        c.with_clip(page.unwrap_or(c.bounds()), |c| {
            if open == Page::Startup.index() {
                let r = self.on_page(pages::list(self.area()));
                pages::draw_list(c, &ctx.text, &forms, &self.list, r, form_px, 1);
            }
            if open == Page::Keyboard.index() {
                let r = self.on_page(pages::shortcuts(self.area()));
                pages::draw_list(c, &ctx.text, &forms, &self.shortcuts, r, form_px, 1);
            }
            if open == Page::DefaultPrograms.index() {
                let r = self.on_page(defaultspage::list(self.area()));
                pages::draw_list(c, &ctx.text, &forms, &self.types, r, form_px, 1);
            }
            if open == Page::Advanced.index() {
                let r = self.on_page(pages::locations(self.area()));
                pages::draw_list(c, &ctx.text, &forms, &self.locations, r, form_px, 1);
            }
        });
        if open == Page::DefaultPrograms.index() {
            // A combo dropped over the list covers it.
            let p = Paint {
                s: 1,
                ..forms.paint()
            };
            propsheet::draw_popups(c, &ctx.text, &p, &self.sheet, form_px);
        }
    }

    pub(crate) fn describe(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.fit_columns(&ctx.forms().controls());

        let px = ctx.theme.fonts.size_px;
        let drawn = |f: &form::Form| f.units().px(px);
        match win {
            ERROR_WIN => {
                if let Some(b) = &self.error {
                    b.access(tree, &self.dialog_text, &self.dialog_theme(&ctx.theme));
                }
            }
            SHORTCUT_WIN => {
                if let Some(d) = &self.shortcut {
                    oneshot::access_form(&d.form, tree, &ctx.text, drawn(&d.form));
                }
            }
            LAYOUT_WIN => {
                if let Some(d) = &self.add_layout {
                    oneshot::access_form(&d.form, tree, &ctx.text, drawn(&d.form));
                }
            }
            ADD_ITEM_WIN => {
                if let Some(d) = &self.add_item {
                    oneshot::access_form(&d.form, tree, &ctx.text, drawn(&d.form));
                }
            }
            RENAME_WIN => {
                if let Some(d) = &self.rename {
                    oneshot::access_form(&d.form, tree, &ctx.text, drawn(&d.form));
                }
            }
            SAVER_WIN => {
                if let Some(d) = &self.saver_dlg {
                    oneshot::access_form(&d.form, tree, &ctx.text, drawn(&d.form));
                }
            }
            displayhost::MONITOR_WIN => {
                if let Some(q) = &self.display.flow.question {
                    let f = &q.dialog.form;
                    oneshot::access_form(f, tree, &ctx.text, drawn(f));
                }
            }
            CHOOSER_WIN => {
                if let Some(ch) = &self.chooser {
                    let dt = self.dialog_theme(&ctx.theme);
                    let lay =
                        ch.ow
                            .layout(&self.dialog_text, dt.fonts.size_px, (0, 0), &dt.controls());
                    ch.ow.access(tree, &lay);
                }
            }
            ADD_WIN => {
                if let Some(d) = &self.add {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.access(tree, r, &ctx.text, ctx.forms().fonts.size_px);
                }
            }
            _ => {
                let (panel, mut focus) =
                    self.sheet
                        .access(tree, ROOT, &ctx.text, drawn(self.sheet.open()));
                if let Some((list, rect, focused)) = self.side_list() {
                    let lay = listview::layout(list, rect, &ctx.forms().controls());
                    let row = list.access(&lay, tree, panel, "side list");
                    if *focused {
                        focus = Some(row);
                    }
                }
                if let Some(f) = focus {
                    tree.set_focus(f);
                }
            }
        }
    }
}
