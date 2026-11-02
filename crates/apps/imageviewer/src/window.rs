//! The main window: the menu bar, the picture's box and the status bar, drawn and named.

use super::*;
use reprisekit::access::ROOT;
use reprisekit::controls::imageview::{self, Layout, Source};
use reprisekit::controls::menubar;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::theme::Color;

/// The status bar's panels after the name: position in the folder, dimensions, file size, zoom.
const PANELS: [i32; 4] = [80, 110, 80, 60];

impl Viewer {
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

    /// The picture's box: the client under the menu bar and over the status bar, all of it full
    /// screen.
    pub(crate) fn box_rect(&self, ctx: &Ctx) -> Rect {
        let c = self.client(ctx);
        if self.full {
            return c;
        }
        let cm = ctx.theme.controls();
        let top = menubar::bar_h(&cm);
        let bottom = if self.status_bar {
            statusbar::height(&cm)
        } else {
            0
        };
        Rect::new(c.x, c.y + top, c.width, (c.height - top - bottom).max(0))
    }

    pub(crate) fn view_layout(&self, ctx: &Ctx) -> Layout {
        imageview::layout(
            &self.view,
            self.box_rect(ctx),
            self.scale,
            &ctx.theme.controls(),
        )
    }

    pub(crate) fn zoom_step(&mut self, up: bool, at: Option<(i32, i32)>, ctx: &Ctx) {
        let lay = self.view_layout(ctx);
        self.view.zoom_step(up, at, &lay);
    }

    /// Ask the worker for the scaled copy a zoom under 100% shows, once per picture, turn and size.
    fn want_render(&mut self, lay: &Layout) {
        let Some(p) = &self.picture else {
            return;
        };
        if p.animated() || lay.scale >= 1.0 {
            return;
        }
        let key = RenderKey {
            generation: self.generation,
            turns: self.view.turns,
            size: lay.content,
        };
        if self.asked == Some(key) {
            return;
        }
        self.asked = Some(key);
        let picture = p.frames[0].0.clone();
        self.send(Job::Render { key, picture });
    }

    fn ground(&self, ctx: &Ctx) -> Color {
        if self.full {
            Color::rgb(0, 0, 0)
        } else {
            ctx.theme.colors.window
        }
    }

    pub(crate) fn draw_main(&mut self, c: &mut Canvas, ctx: &Ctx) {
        self.scale = c.scale().max(1);
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        let lay = self.view_layout(ctx);
        self.want_render(&lay);
        let ground = self.ground(ctx);
        let px = ctx.theme.fonts.size_px;
        match &self.picture {
            Some(p) => {
                let key = RenderKey {
                    generation: self.generation,
                    turns: self.view.turns,
                    size: lay.content,
                };
                let rendered = (self.rendered.as_ref())
                    .filter(|(k, _)| *k == key)
                    .map(|(_, r)| r);
                let src = Source {
                    picture: &p.frames[self.frame.min(p.frames.len() - 1)].0,
                    rendered,
                };
                imageview::draw(c, &paint, &self.view, src, &lay, ground);
            }
            None => {
                let ink = if self.full {
                    Color::rgb(0xFF, 0xFF, 0xFF)
                } else {
                    ctx.theme.colors.window_text
                };
                let msg = self.blank_text();
                imageview::draw_message(c, &ctx.text, self.box_rect(ctx), msg, (ground, ink), px);
            }
        }
        if self.full {
            return;
        }
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::draw(c, &ctx.text, &paint, &bar, r, px);
        }
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
    }

    pub(crate) fn blank_text(&self) -> &str {
        match &self.blank {
            Blank::Nothing => "No picture is open.",
            Blank::Loading => "",
            Blank::Failed(e) => e,
        }
    }

    /// The status bar's texts: name, "n of m", dimensions, file size, zoom.
    pub(crate) fn status_texts(&self, ctx: &Ctx) -> [String; 5] {
        let name = (self.folder.current())
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(p) = &self.picture else {
            return [
                name,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ];
        };
        let (w, h) = self.view.shown_size();
        let zoom = self.view_layout(ctx).scale * 100.0;
        [
            name,
            self.folder.position(),
            format!("{w} x {h}"),
            reprisefs::props::short_size(p.bytes),
            format!("{}%", zoom.round()),
        ]
    }

    pub(crate) fn status(&self, ctx: &Ctx) -> Option<(StatusBar, Rect)> {
        if !self.status_bar || self.full {
            return None;
        }
        let [name, rest @ ..] = self.status_texts(ctx);
        let panel = |text: String, width: i32| Panel {
            text,
            width,
            pad: statusbar::TEXT_PAD,
            icon: None,
        };
        let mut panels = vec![panel(name, 0)];
        panels.extend(rest.into_iter().zip(PANELS).map(|(t, w)| panel(t, w)));
        let bar = StatusBar {
            panels,
            grip: !ctx.maximized,
        };
        let h = statusbar::height(&ctx.theme.controls());
        Some((bar, Rect::new(0, ctx.size.1 - h, ctx.size.0, h)))
    }

    pub(crate) fn access_main(&self, tree: &mut Tree, ctx: &Ctx) {
        let menus = if self.full {
            None
        } else {
            self.bar.access(&self.bar_layout(ctx), tree, ROOT)
        };
        let lay = self.view_layout(ctx);
        let name = self.status_texts(ctx)[0].clone();
        let picture = imageview::access(tree, ROOT, &self.view, &lay, &name);
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::access(&bar, r, ctx.theme.text, tree, ROOT);
        }
        tree.set_focus(menus.unwrap_or(picture));
    }
}
