//! The main window's edit box: its font, layout, status bar, drawing and access tree.

use super::*;

pub(crate) const EDIT_PX: u32 = 13;

pub(crate) fn edit_font() -> TextRenderer {
    TextRenderer::monospace(EDIT_PX)
}

/// Time/Date's stamp: the time as the LC_TIME locale writes it (12- or 24-hour), then its
/// short date.
pub(crate) fn local_stamp(secs: i64) -> String {
    use reprisefs::locale;
    let tm = locale::localtime(secs);
    let clock = if locale::is_12h() {
        c"%l:%M %p"
    } else {
        c"%H:%M"
    };
    let time = locale::strftime(&tm, clock);
    format!("{} {}", time.trim(), locale::short_date(&tm))
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
        tr!(
            "status-position",
            line = (self.edit.caret().line + 1).to_string(),
            column = multiedit::char_col(line, self.edit.caret().col).to_string(),
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

    pub(crate) fn edit_key(&mut self, key: MultiKey, ctx: &Ctx) {
        if self.edit.key(key, &self.mono, &self.metrics).edited() {
            self.dirty = true;
        }
        self.scroll_to_caret(ctx);
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
        self.view.scroll_lines(by, &lay)
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
            // Find's window keeps the selection shown without a caret.
            reprisekit::textedit::Caret::new(self.edit_focused(), self.focused && ctx.caret_on),
            self.drop_point(ctx),
        );
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
