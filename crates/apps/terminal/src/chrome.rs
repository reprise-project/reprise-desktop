//! The toolbar and tab strip over the grid: over the frame's glass in a look that has it, plain
//! bars otherwise. A window with its tabs in the caption has no strip; its toolbar leads with
//! the new-tab split button. The pointer on them, and the new-tab menu.

use repriseapp::ctxmenu::{Popup, Step};
use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput, PointerInput, Reply};
use reprisekit::controls::rebar;
use reprisekit::controls::tabstrip::{self, AdderPart, DocHit, DocLayout, Drag, TabStrip};
use reprisekit::controls::termgrid;
use reprisekit::controls::toolbar::{self, Item, Part, ToolBar};
use reprisekit::look::{self, Paint};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::Theme;

use crate::Terminal;
use crate::newtab::{self, Choice};
use crate::sshhosts;

pub const NEW: &str = "new";
pub const COPY: &str = "copy";
pub const PASTE: &str = "paste";
pub const FIND: &str = "find";
pub const SETTINGS: &str = "settings";

pub const BTN_MIDDLE: u32 = 0x112;
/// How far a held tab moves before it is dragged, logical px.
const DRAG_PX: i32 = 4;
/// The terminal background between the chrome and the first row, logical px.
pub const GRID_GAP: i32 = 4;

pub struct Chrome {
    pub bar: ToolBar,
    pub press: Option<Press>,
    /// The new-tab menu, open.
    pub menu: Option<Popup<Choice>>,
    /// The chrome is drawn again next frame.
    pub dirty: bool,
    /// The theme and active state the chrome was last drawn in.
    pub drawn: Option<(Theme, bool)>,
}

impl Chrome {
    pub fn new() -> Chrome {
        Chrome {
            bar: ToolBar {
                items: Vec::new(),
                hot: None,
                pressed: None,
                latched: None,
                big: true,
            },
            press: None,
            menu: None,
            dirty: true,
            drawn: None,
        }
    }
}

/// What a left press is holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A tab, `grab` px right of its left edge, pressed at `x`.
    Tab {
        tab: usize,
        x: i32,
        grab: i32,
    },
    Close(usize),
    Add,
    Button(usize),
}

pub struct Layout {
    pub toolbar: Rect,
    pub buttons: Vec<Rect>,
    pub strip: DocLayout,
    /// The cells, [`GRID_GAP`] under the band.
    pub grid: Rect,
    /// The scrollback bar, from the band down the right edge.
    pub bar: Rect,
    /// The terminal background between the band and the grid.
    pub gap: Rect,
    /// The chrome's height: the toolbar and the strip.
    pub band: i32,
}

/// Copy, Paste and Find, then Settings (laid out at the right edge); with the tabs in the
/// caption, New Tab and its menu first.
pub fn toolbar_items(ctx: &mut Ctx, caption: bool) -> Vec<Item> {
    let mut icon = |icons: &[&str]| {
        icons
            .iter()
            .find_map(|n| ctx.icons.get(n, toolbar::BIG_ICON))
    };
    let new = Item::DropButton {
        id: NEW,
        icon: icon(&["tab-new", "list-add"]),
        label: Some("New Tab".into()),
        enabled: true,
    };
    let mut button = |id, label: &str, icons: &[&str]| Item::Button {
        id,
        icon: icon(icons),
        label: Some(label.into()),
        enabled: true,
    };
    let lead = match caption {
        true => vec![new, Item::Separator],
        false => Vec::new(),
    };
    lead.into_iter()
        .chain([
            button(COPY, "Copy", &["edit-copy"]),
            button(PASTE, "Paste", &["edit-paste"]),
            button(FIND, "Find", &["edit-find", "system-search"]),
            button(
                SETTINGS,
                "Settings",
                &["preferences-system", "preferences-other"],
            ),
        ])
        .collect()
}

pub fn on_glass(ctx: &Ctx) -> bool {
    look::client_glass(&ctx.theme, true).is_some()
}

/// The toolbar's and the strip's heights; no strip with the tabs in the caption.
fn heights(theme: &Theme, caption: bool) -> (i32, i32) {
    (
        toolbar::height(true, &theme.controls()),
        if caption {
            0
        } else {
            tabstrip::doc_height(theme.text)
        },
    )
}

/// The client height the chrome takes in `theme`.
pub fn band_height(theme: &Theme, caption: bool) -> i32 {
    let (tb, st) = heights(theme, caption);
    tb + st
}

/// Where the first row starts: the chrome's height and the gap under it.
pub fn grid_top(theme: &Theme, caption: bool) -> i32 {
    band_height(theme, caption) + GRID_GAP
}

/// The client under the chrome: the cells under the gap, and the scrollback bar beside them.
fn below(ctx: &Ctx, caption: bool) -> (Rect, Rect) {
    let band = band_height(&ctx.theme, caption);
    let (w, h) = ctx.size;
    let area = Rect::new(0, band, w, (h - band).max(0));
    let (cells, bar) = termgrid::split(area, ctx.theme.controls().scroll);
    let gap = GRID_GAP.min(cells.height);
    let grid = Rect::new(cells.x, cells.y + gap, cells.width, cells.height - gap);
    (grid, bar)
}

/// Where the cells go: the client under the chrome and the gap, left of the scrollback bar.
pub fn grid_rect(ctx: &Ctx, caption: bool) -> Rect {
    below(ctx, caption).0
}

pub fn bar_rect(ctx: &Ctx, caption: bool) -> Rect {
    below(ctx, caption).1
}

pub fn layout(bar: &ToolBar, strip: &TabStrip, caption: bool, ctx: &Ctx) -> Layout {
    let (tb, st) = heights(&ctx.theme, caption);
    let w = ctx.size.0;
    let px = ctx.theme.fonts.size_px;
    let toolbar = Rect::new(0, 0, w, tb);
    let mut buttons = toolbar::layout(bar, &ctx.text, px, toolbar, &ctx.theme.controls());
    let lead = buttons.first().map_or(0, |r| r.x);
    if let Some(last) = buttons.last_mut() {
        last.x = (toolbar.right() - lead - last.width).max(last.x);
    }
    let area = Rect::new(0, tb, w, st);
    let (grid, bar) = below(ctx, caption);
    Layout {
        toolbar,
        buttons,
        strip: tabstrip::doc_layout(&ctx.text, px, strip, area, ctx.theme.text),
        grid,
        bar,
        gap: Rect::new(0, tb + st, grid.width, grid.y - tb - st),
        band: tb + st,
    }
}

impl Terminal {
    pub(crate) fn chrome_layout(&self, ctx: &Ctx) -> Layout {
        layout(&self.w.chrome.bar, &self.w.tabs.strip, self.w.caption, ctx)
    }

    /// The toolbar, then the strip under it, each in its look's own way.
    pub(crate) fn draw_chrome(&self, c: &mut Canvas, ctx: &Ctx, lay: &Layout) {
        let px = ctx.theme.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        let bar = &self.w.chrome.bar;
        let ink = on_glass(ctx)
            .then(|| look::ink_on_glass(&ctx.theme))
            .flatten();
        let strip = lay.strip.area;
        match ink {
            Some(ink) => {
                c.clear_rect(Rect::new(0, 0, lay.toolbar.width, lay.band));
                let (r, rects) = (lay.toolbar, &lay.buttons);
                toolbar::draw_over_glass(c, &ctx.text, &paint, bar, r, rects, ink, px);
                look::glow_on_glass(c, &ctx.theme, r);
                toolbar::draw_glass_separators(c, &paint, bar, rects);
            }
            None => {
                // A style's toolbar leaves its background to the rebar behind it.
                let band = rebar::Rebar {
                    bands: vec![rebar::Band {
                        height: lay.toolbar.height,
                    }],
                };
                rebar::draw(c, &paint, &band, lay.toolbar);
                toolbar::draw(c, &ctx.text, &paint, bar, lay.toolbar, &lay.buttons, px);
                c.fill_rect(strip, paint.cs.button_face);
            }
        }
        if !self.w.caption {
            let tabs = &self.w.tabs.strip;
            tabstrip::doc_draw(c, &ctx.text, &paint, tabs, &lay.strip, ink.is_some(), px);
        }
    }

    pub(crate) fn chrome_pointer(&mut self, p: PointerInput, ctx: &mut Ctx) -> Reply {
        let lay = self.chrome_layout(ctx);
        if self.w.chrome.menu.is_some() {
            return self.menu_pointer(p, ctx);
        }
        let strip_hit = |x, y| match lay.strip.area.contains(x, y) {
            true => tabstrip::doc_hit(&lay.strip, x, y),
            false => DocHit::Empty,
        };
        match p {
            PointerInput::Motion { x, y } => {
                if let Some(Press::Tab { tab, x: x0, grab }) = self.w.chrome.press {
                    if self.w.tabs.strip.dragging.is_none() && (x - x0).abs() >= DRAG_PX {
                        self.w.tabs.strip.dragging = Some(Drag { tab, grab, x });
                    }
                    if self.w.tabs.strip.dragging.is_some() {
                        self.w.tabs.drag_to(&lay.strip, x);
                        return self.chrome_changed();
                    }
                    return Reply::none();
                }
                let hot = toolbar::hit(&self.w.chrome.bar, &lay.buttons, x, y);
                let lit = std::mem::replace(&mut self.w.chrome.bar.hot, hot) != hot;
                if self.w.tabs.strip.doc_hover(strip_hit(x, y)) || lit {
                    return self.chrome_changed();
                }
                Reply::none()
            }
            PointerInput::Press { x, y, button, .. } => {
                let hit = strip_hit(x, y);
                if let Some((r, part)) = self.new_button_at(&lay, x, y, ctx)
                    && (button == BTN_RIGHT || (button == BTN_LEFT && part == Part::Drop))
                {
                    self.open_menu((r.x, r.bottom()), ctx);
                    return Reply::redraw();
                }
                match (button, hit) {
                    (BTN_MIDDLE, DocHit::Tab(i) | DocHit::Close(i)) => self.ask_close(i, ctx),
                    (BTN_RIGHT, DocHit::Add) | (BTN_LEFT, DocHit::Menu) => {
                        self.open_menu((lay.strip.menu.x, lay.strip.menu.bottom()), ctx);
                        Reply::redraw()
                    }
                    (BTN_LEFT, _) => self.left_press(x, y, hit, &lay),
                    _ => Reply::none(),
                }
            }
            PointerInput::Release { x, y, button, .. } if button == BTN_LEFT => {
                let Some(press) = self.chrome_let_go() else {
                    return Reply::none();
                };
                let changed = self.chrome_changed();
                match press {
                    Press::Close(i) if strip_hit(x, y) == DocHit::Close(i) => {
                        self.ask_close(i, ctx)
                    }
                    Press::Add if strip_hit(x, y) == DocHit::Add => self.new_tab(&Choice::Shell),
                    Press::Button(i)
                        if toolbar::hit(&self.w.chrome.bar, &lay.buttons, x, y) == Some(i) =>
                    {
                        changed.and(self.button(i))
                    }
                    _ => changed,
                }
            }
            PointerInput::Leave if self.w.chrome.press.is_none() => {
                self.w.chrome.bar.hot = None;
                self.w.tabs.strip.doc_hover(DocHit::Empty);
                self.chrome_changed()
            }
            _ => Reply::none(),
        }
    }

    fn left_press(&mut self, x: i32, y: i32, hit: DocHit, lay: &Layout) -> Reply {
        if let Some(i) = toolbar::hit(&self.w.chrome.bar, &lay.buttons, x, y) {
            self.w.chrome.bar.pressed = Some(i);
            self.w.chrome.press = Some(Press::Button(i));
            return self.chrome_changed();
        }
        let strip = &mut self.w.tabs.strip;
        match hit {
            DocHit::Close(i) => {
                (strip.hot_close, strip.held) = (Some(i), true);
                self.w.chrome.press = Some(Press::Close(i));
                self.chrome_changed()
            }
            DocHit::Add => {
                (strip.hot_adder, strip.held) = (Some(AdderPart::Add), true);
                self.w.chrome.press = Some(Press::Add);
                self.chrome_changed()
            }
            DocHit::Tab(i) => {
                let grab = x - lay.strip.tabs[i].x;
                self.w.chrome.press = Some(Press::Tab { tab: i, x, grab });
                if i == self.w.tabs.strip.open {
                    return self.chrome_changed();
                }
                self.w.tabs.select(i);
                self.switched()
            }
            DocHit::Menu | DocHit::Empty => Reply::none(),
        }
    }

    /// The new-tab split button's rect and half under `(x, y)`, when that is where it is.
    fn new_button_at(&self, lay: &Layout, x: i32, y: i32, ctx: &Ctx) -> Option<(Rect, Part)> {
        let bar = &self.w.chrome.bar;
        let (i, part) = toolbar::hit_part(bar, &lay.buttons, x, y, &ctx.theme.controls())?;
        match bar.items.get(i) {
            Some(Item::DropButton { id: NEW, .. }) => Some((lay.buttons[i], part)),
            _ => None,
        }
    }

    fn button(&mut self, i: usize) -> Reply {
        let id = match self.w.chrome.bar.items.get(i) {
            Some(Item::Button { id, .. } | Item::DropButton { id, .. }) => *id,
            _ => return Reply::none(),
        };
        match id {
            NEW => self.new_tab(&Choice::Shell),
            COPY => self.copy(),
            PASTE => self.paste(),
            FIND => self.open_find(),
            SETTINGS => self.open_options(),
            _ => Reply::none(),
        }
    }

    /// Ends a press on the toolbar or the tab strip, a dragged tab left where the drag put it; the
    /// press it ended.
    pub(crate) fn chrome_let_go(&mut self) -> Option<Press> {
        let press = self.w.chrome.press.take()?;
        self.w.tabs.strip.held = false;
        self.w.tabs.strip.dragging = None;
        self.w.chrome.bar.pressed = None;
        Some(press)
    }

    pub(crate) fn chrome_changed(&mut self) -> Reply {
        self.w.chrome.dirty = true;
        Reply::redraw()
    }

    /// Open the new-tab menu at `at`, the hosts read from `~/.ssh/config` afresh.
    fn open_menu(&mut self, at: (i32, i32), ctx: &Ctx) {
        let hosts = std::env::var_os("HOME")
            .map(|h| sshhosts::read(&std::path::Path::new(&h).join(".ssh")))
            .unwrap_or_default();
        let (items, choices) = newtab::menu(&self.settings, &hosts);
        let rows = items.into_iter().zip(choices).collect();
        self.w.chrome.menu = Popup::open(rows, at.0, at.1, ctx, false);
        self.w.full = true;
    }

    fn menu_pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        let Some(m) = &mut self.w.chrome.menu else {
            return Reply::none();
        };
        let step = match p {
            PointerInput::Motion { x, y } if m.motion(x, y, ctx) => Step::Redraw,
            PointerInput::Press { x, y, .. } => m.press(x, y, ctx),
            _ => Step::None,
        };
        self.menu_step(step)
    }

    pub(crate) fn menu_key(&mut self, k: &KeyInput, ctx: &Ctx) -> Reply {
        let Some(m) = &mut self.w.chrome.menu else {
            return Reply::none();
        };
        let step = m.key(k, ctx);
        self.menu_step(step)
    }

    fn menu_step(&mut self, step: Step<Choice>) -> Reply {
        match step {
            Step::None => Reply::none(),
            Step::Redraw => Reply::redraw(),
            Step::Closed => self.close_menu(),
            Step::Chosen(choice) => {
                let closed = self.close_menu();
                match choice {
                    Some(c) => closed.and(self.new_tab(&c)),
                    None => closed,
                }
            }
        }
    }

    fn close_menu(&mut self) -> Reply {
        self.w.chrome.menu = None;
        self.w.full = true;
        Reply::redraw()
    }

    pub(crate) fn draw_menu(&self, c: &mut Canvas, ctx: &Ctx) {
        if let Some(m) = &self.w.chrome.menu {
            m.draw(c, ctx);
        }
    }
}
