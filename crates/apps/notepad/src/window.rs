//! The main window's edit box: its font, layout, status bar, drawing and access tree.

use super::*;

pub(crate) const EDIT_PX: u32 = 13;

pub(crate) fn edit_font() -> TextRenderer {
    let cfg = |antialias| FontConfig {
        family: vec![
            "Lucida Console".into(),
            "Liberation Mono".into(),
            "DejaVu Sans Mono".into(),
        ],
        size_px: EDIT_PX,
        antialias,
        italic: false,
        full_hinting: false,
        shell_dlg: None,
    };
    // The first face is drawn hinted monochrome; a fallback mono blotches that way at 13 px, so it
    // is antialiased.
    let mono = TextRenderer::load(&cfg(false));
    if mono.has_face("Lucida Console") {
        mono
    } else {
        TextRenderer::load(&cfg(true))
    }
}

fn local_stamp(secs: i64) -> String {
    let tm = reprisefs::locale::localtime(secs);
    let (h12, ampm) = match tm.tm_hour {
        0 => (12, "AM"),
        1..=11 => (tm.tm_hour, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!(
        "{}:{:02} {} {}/{}/{}",
        h12,
        tm.tm_min,
        ampm,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_year + 1900
    )
}

impl Notepad {
    fn client(&self, ctx: &Ctx) -> Rect {
        Rect::new(0, 0, ctx.size.0, ctx.size.1)
    }

    pub(crate) fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar
            .layout(
                &ctx.text,
                ctx.theme.fonts.size_px,
                self.client(ctx),
                &ctx.theme.controls(),
            )
            .reaching(ctx.menu_within())
    }

    /// The status bar reserves no space of its own, so the edit gives up its height here, under
    /// a `menu_h` menu bar.
    pub(crate) fn edit_height(&self, h: i32, menu_h: i32, bar_h: i32) -> i32 {
        let bar = if self.status_bar { bar_h } else { 0 };
        h - menu_h - bar
    }

    pub(crate) fn edit_rect(&self, ctx: &Ctx) -> Rect {
        let c = self.client(ctx);
        let cm = ctx.theme.controls();
        let menu_h = menubar::bar_h(&cm);
        Rect::new(
            c.x,
            c.y + menu_h,
            c.width,
            self.edit_height(c.height, menu_h, statusbar::height(&cm)),
        )
    }

    pub(crate) fn bar_text(&self) -> String {
        let line = self
            .edit
            .lines()
            .get(self.edit.caret().line)
            .map(String::as_str)
            .unwrap_or("");
        // Col counts characters, not bytes and not tab stops.
        format!(
            "Ln {}, Col {}",
            self.edit.caret().line + 1,
            multiedit::char_col(line, self.edit.caret().col)
        )
    }

    pub(crate) fn edit_layout(&self, ctx: &Ctx) -> multiedit::Layout {
        multiedit::layout(
            &self.mono,
            &self.metrics,
            &self.edit,
            self.edit_rect(ctx),
            1,
        )
    }

    pub(crate) fn scroll_to_caret(&mut self, ctx: &Ctx) {
        let lay = self.edit_layout(ctx);
        self.view
            .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
    }

    pub(crate) fn edit_key(&mut self, key: MultiKey, ctx: &Ctx) -> bool {
        let changed = self.edit.key(key, &self.mono, &self.metrics);
        self.scroll_to_caret(ctx);
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub(crate) fn insert_time_date(&mut self, ctx: &Ctx) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let stamp = local_stamp(now);
        self.edit.paste(&stamp);
        self.dirty = true;
        self.scroll_to_caret(ctx);
    }

    pub(crate) fn scroll(&mut self, by: i64, ctx: &Ctx) -> bool {
        let lay = self.edit_layout(ctx);
        self.view.scroll_lines(by, &self.edit, lay.rows)
    }

    pub(crate) fn draw_main(&self, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        c.fill_rect(self.client(ctx), cs.button_face);
        let el = self.edit_layout(ctx);
        multiedit::draw(
            c,
            &self.mono,
            &paint,
            &self.metrics,
            &self.edit,
            self.view,
            &el,
            reprisekit::textedit::Caret::new(true, ctx.caret_on),
        );
        if let Some(at) = self.drop_point(ctx) {
            multiedit::draw_drop_point(
                c,
                &self.mono,
                &paint,
                &self.metrics,
                &self.edit,
                self.view,
                &el,
                at,
            );
        }
        if let Some(corner) = el.corner().filter(|_| !ctx.maximized && !self.status_bar) {
            draw_scroll_corner(c, &paint, corner, true);
        }
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::draw(c, &ctx.text, &paint, &bar, r, ctx.theme.fonts.size_px);
        }
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
        if let Some(menu) = &self.context {
            menu.draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
        }
    }

    pub(crate) fn status(&self, ctx: &Ctx) -> Option<(StatusBar, Rect)> {
        if !self.status_bar {
            return None;
        }
        let bar = StatusBar {
            panels: vec![
                Panel {
                    text: String::new(),
                    width: 0,
                    pad: statusbar::TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: self.bar_text(),
                    width: 356,
                    pad: statusbar::TEXT_PAD,
                    icon: None,
                },
            ],
            grip: !ctx.maximized,
        };
        let h = statusbar::height(&ctx.theme.controls());
        let r = Rect::new(0, ctx.size.1 - h, ctx.size.0, h);
        Some((bar, r))
    }

    pub(crate) fn access_main(&self, tree: &mut Tree, ctx: &Ctx) {
        let menus = self.bar.access(&self.bar_layout(ctx), tree, ROOT);
        let el = self.edit_layout(ctx);
        let edit = multiedit::access(
            &self.edit,
            self.view,
            &el,
            &self.metrics,
            &self.mono,
            tree,
            ROOT,
            "edit",
        );
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::access(&bar, r, ctx.theme.text, tree, ROOT);
        }
        let context = self.context.as_ref().map(|menu| menu.access(tree, ROOT));
        tree.set_focus(context.or(menus).unwrap_or(edit));
    }
}
