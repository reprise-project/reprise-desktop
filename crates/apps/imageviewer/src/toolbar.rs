//! The toolbar under the menu bar: each button runs a menu row, greys with it and names it in
//! its tip and to a screen reader.

use super::*;
use menu::{row, title};
use repriseapp::tip::Rest;
use reprisekit::controls::menubar;
use reprisekit::controls::rebar::{self, Band, Rebar};
use reprisekit::controls::toolbar::{self, Item, ToolBar};
use reprisekit::controls::tooltip;
use reprisekit::look;

const TIP_DELAY: Duration = Duration::from_millis(tooltip::SHOW_MS as u64);

/// A button: its id, the menu row it runs, and the icon names it tries in turn.
type Button = (&'static str, (usize, usize), &'static [&'static str]);

/// The buttons left to right; `None` is a separator.
const BUTTONS: [Option<Button>; 14] = [
    Some(("previous", (title::VIEW, row::PREVIOUS), &["go-previous"])),
    Some(("next", (title::VIEW, row::NEXT), &["go-next"])),
    None,
    Some(("zoomin", (title::VIEW, row::ZOOM_IN), &["zoom-in"])),
    Some(("zoomout", (title::VIEW, row::ZOOM_OUT), &["zoom-out"])),
    Some(("fit", (title::VIEW, row::FIT), &["zoom-fit-best"])),
    Some(("actual", (title::VIEW, row::ACTUAL), &["zoom-original"])),
    None,
    Some((
        "rotateccw",
        (title::IMAGE, row::ROTATE_CCW),
        &["object-rotate-left"],
    )),
    Some((
        "rotatecw",
        (title::IMAGE, row::ROTATE_CW),
        &["object-rotate-right"],
    )),
    None,
    Some((
        "slideshow",
        (title::VIEW, row::SLIDESHOW),
        &["view-presentation", "media-playback-start"],
    )),
    None,
    Some((
        "delete",
        (title::EDIT, row::DELETE),
        &["edit-delete", "user-trash"],
    )),
];

/// The toolbar with the button a left press holds and the pointer's rest for a tip.
pub(crate) struct Tools {
    pub(crate) bar: ToolBar,
    pub(crate) held: Option<usize>,
    pub(crate) rest: Option<Rest<usize>>,
    pub(crate) tip_up: bool,
}

impl Tools {
    pub(crate) fn new() -> Tools {
        Tools {
            bar: ToolBar {
                items: Vec::new(),
                hot: None,
                pressed: None,
                latched: None,
                big: true,
            },
            held: None,
            rest: None,
            tip_up: false,
        }
    }
}

pub(crate) struct Chrome {
    /// The rebar behind the toolbar, under the menu bar, and its band.
    pub(crate) rebar: Rect,
    pub(crate) band: Rect,
    pub(crate) buttons: Vec<Rect>,
    /// Where the line closing the rebar off runs.
    pub(crate) etch_y: i32,
    /// The first row under the chrome.
    pub(crate) bottom: i32,
}

fn rebar_of(cm: &reprisekit::metrics::ControlMetrics) -> Rebar {
    Rebar {
        bands: vec![Band {
            height: toolbar::height(true, cm),
        }],
    }
}

impl Viewer {
    /// The buttons with their icons from `ctx.icons`, greyed as their rows are.
    pub(crate) fn build_toolbar(&mut self, ctx: &mut Ctx) {
        self.tools.bar.items = BUTTONS
            .iter()
            .map(|b| match b {
                Some((id, (t, r), icons)) => Item::Button {
                    id,
                    icon: (ctx.icons.rank(icons).into_iter())
                        .find_map(|n| ctx.icons.get(n, toolbar::BIG_ICON)),
                    label: None,
                    enabled: self.row_live(*t, *r),
                },
                None => Item::Separator,
            })
            .collect();
    }

    fn row_live(&self, t: usize, r: usize) -> bool {
        matches!(self.bar.menus[t][r], MenuItem::Entry { enabled: true, .. })
    }

    /// Grey the buttons whose rows are greyed.
    pub(crate) fn sync_toolbar(&mut self) {
        let live: Vec<bool> = BUTTONS
            .iter()
            .map(|b| b.is_some_and(|(_, (t, r), _)| self.row_live(t, r)))
            .collect();
        for (item, on) in self.tools.bar.items.iter_mut().zip(live) {
            if let Item::Button { enabled, .. } = item {
                *enabled = on;
            }
        }
    }

    /// The toolbar's place under the menu bar; none full screen.
    pub(crate) fn chrome(&self, ctx: &Ctx) -> Option<Chrome> {
        if self.full {
            return None;
        }
        let cm = ctx.theme.controls();
        let rb = rebar_of(&cm);
        let top = menubar::bar_h(&cm);
        let r = Rect::new(0, top, ctx.size.0, rebar::height(&rb, &cm.rebar));
        let band = rebar::layout(&rb, r, &cm.rebar)[0];
        let px = ctx.theme.fonts.size_px;
        let buttons = toolbar::layout(&self.tools.bar, &ctx.text, px, band, &cm);
        Some(Chrome {
            rebar: r,
            band,
            buttons,
            etch_y: r.bottom(),
            bottom: r.bottom() + cm.rebar.closing,
        })
    }

    /// The rebar and its buttons: over the frame's glass in a look whose menu bar is, plain
    /// otherwise.
    pub(crate) fn draw_toolbar(&self, c: &mut Canvas, ctx: &Ctx, ch: &Chrome, paint: &Paint) {
        let (bar, px, band) = (&self.tools.bar, ctx.theme.fonts.size_px, ch.band);
        match look::ink_on_glass(&ctx.theme) {
            Some(ink) => {
                c.clear_rect(ch.rebar);
                toolbar::draw_over_glass(c, &ctx.text, paint, bar, band, &ch.buttons, ink, px);
                look::glow_on_glass(c, &ctx.theme, ch.rebar);
                toolbar::draw_glass_separators(c, paint, bar, &ch.buttons);
                c.report_glass_top(ch.etch_y);
            }
            None => {
                rebar::draw(c, paint, &rebar_of(&ctx.theme.controls()), ch.rebar);
                toolbar::draw(c, &ctx.text, paint, bar, band, &ch.buttons, px);
            }
        }
        rebar::draw_closing_line(c, paint, 0, ctx.size.0, ch.etch_y);
    }

    /// Button `i`'s row: its name and key.
    fn row_of(&self, i: usize) -> Option<(&str, Option<&str>)> {
        let (_, (t, r), _) = BUTTONS.get(i).copied().flatten()?;
        match &self.bar.menus[t][r] {
            MenuItem::Entry {
                label, shortcut, ..
            } => Some((label, shortcut.as_deref())),
            _ => None,
        }
    }

    /// The tip of button `i`: its row's name and key.
    pub(crate) fn tip_text(&self, i: usize) -> Option<String> {
        let (name, key) = self.row_of(i)?;
        Some(match key {
            Some(key) => tr!("button-tip", name = name.to_string(), key = key.to_string()),
            None => name.to_string(),
        })
    }

    /// The menu row button `i` runs.
    pub(crate) fn button_row(i: usize) -> Option<(usize, usize)> {
        BUTTONS.get(i).copied().flatten().map(|(_, row, _)| row)
    }

    pub(crate) fn draw_tip(&self, c: &mut Canvas, ctx: &Ctx) {
        let Some(rest) = self.tools.rest.filter(|_| self.tools.tip_up) else {
            return;
        };
        let Some(text) = self.tip_text(rest.on) else {
            return;
        };
        let tc = tooltip::draw_tip_at(&ctx.theme, &ctx.text, &text, c.scale());
        let size = (tc.width as i32, tc.height as i32);
        let (x, y) = tooltip::place(rest.at, size, self.client(ctx), true);
        c.blit_over(x, y, &tc);
    }

    pub(crate) fn access_toolbar(&self, tree: &mut Tree, ctx: &Ctx) {
        let Some(ch) = self.chrome(ctx) else {
            return;
        };
        let names: Vec<String> = (0..BUTTONS.len())
            .map(|i| {
                self.row_of(i)
                    .map_or_else(String::new, |(name, _)| name.to_string())
            })
            .collect();
        let bar = &self.tools.bar;
        toolbar::access(
            bar,
            ch.rebar,
            &ch.buttons,
            &names,
            tree,
            reprisekit::access::ROOT,
            "toolbar",
        );
    }

    /// The pointer moved to `(x, y)`: the hot button and the tip's rest follow it. Whether the
    /// toolbar changed and whether a tip waits on ticks.
    pub(crate) fn toolbar_motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> (bool, bool) {
        let on = self
            .chrome(ctx)
            .and_then(|ch| toolbar::hit(&self.tools.bar, &ch.buttons, x, y));
        let t = &mut self.tools;
        let lit = std::mem::replace(&mut t.bar.hot, on) != on;
        let (was, up) = (t.rest, t.tip_up);
        t.rest = Rest::moved(
            was,
            on.filter(|_| t.held.is_none()),
            (x, y),
            up,
            Instant::now(),
        );
        t.tip_up = up && t.rest == was;
        (lit || (up && !t.tip_up), t.rest.is_some() && !t.tip_up)
    }

    /// A left press on the toolbar: a button held until its release. `None` when the press is not
    /// on the chrome.
    pub(crate) fn toolbar_press(&mut self, x: i32, y: i32, ctx: &Ctx) -> Option<Reply> {
        let ch = self.chrome(ctx)?;
        if !Rect::new(0, ch.rebar.y, ch.rebar.width, ch.bottom - ch.rebar.y).contains(x, y) {
            return None;
        }
        let up = self.hide_tip();
        let hit = toolbar::hit(&self.tools.bar, &ch.buttons, x, y);
        if hit.is_none() {
            return Some(if up { Reply::redraw() } else { Reply::none() });
        }
        self.tools.held = hit;
        self.tools.bar.pressed = hit;
        Some(Reply::redraw())
    }

    /// The left button let go: a held button runs its row when the pointer is still on it.
    pub(crate) fn toolbar_release(&mut self, x: i32, y: i32, ctx: &Ctx) -> Option<Reply> {
        let held = self.tools.held.take()?;
        self.tools.bar.pressed = None;
        let on = self
            .chrome(ctx)
            .and_then(|ch| toolbar::hit(&self.tools.bar, &ch.buttons, x, y));
        match (on == Some(held), Viewer::button_row(held)) {
            (true, Some((t, r))) => Some(Reply {
                redraw: true,
                ..self.command(t, r, ctx)
            }),
            _ => Some(Reply::redraw()),
        }
    }

    /// The pointer left the window or the window lost focus: no hot button and no tip. Whether the
    /// toolbar changed.
    pub(crate) fn toolbar_leave(&mut self) -> bool {
        let hot = self.tools.bar.hot.take().is_some();
        self.hide_tip() || hot
    }

    /// Drops the tip and its rest; `true` when it was up.
    pub(crate) fn hide_tip(&mut self) -> bool {
        self.tools.rest = None;
        std::mem::take(&mut self.tools.tip_up)
    }

    /// A tick while the pointer rests on a button: the tip comes up once the rest is long enough.
    pub(crate) fn toolbar_tick(&mut self) -> Reply {
        let t = &mut self.tools;
        let Some(rest) = t.rest.filter(|_| !t.tip_up) else {
            return Reply::none();
        };
        if rest.due(Instant::now(), TIP_DELAY) {
            t.tip_up = true;
            return Reply::redraw();
        }
        Reply {
            tick: true,
            ..Reply::none()
        }
    }
}
