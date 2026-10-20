//! The toolbar and tab strip over the grid: over the frame's glass in a look that has it, plain
//! bars otherwise. A window with its tabs in the caption has no strip; its toolbar leads with
//! the new-tab split button. The pointer on them, and the new-tab menu.

use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput, PointerInput, Reply};
use reprisekit::access::accesskit::NodeId;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::hosted::Chain;
use reprisekit::controls::menu::{MenuAction, MenuCtx, MenuKey, MenuTree, PopupMenu};
use reprisekit::controls::rebar;
use reprisekit::controls::tabstrip::{self, AdderPart, DocHit, DocLayout, Drag, TabStrip};
use reprisekit::controls::toolbar::{self, Item, Part, ToolBar};
use reprisekit::look::{self, Paint};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;
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

pub struct Chrome {
    pub bar: ToolBar,
    pub press: Option<Press>,
    pub menu: Option<Popup>,
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

/// The new-tab menu, open.
pub struct Popup {
    tree: MenuTree,
    chain: PopupMenu,
    choices: Vec<Option<Choice>>,
}

impl Popup {
    /// The menu's nodes under the window; returns the node to focus.
    pub fn access(&self, tree: &mut Tree) -> NodeId {
        self.chain.access(tree, ROOT, "newtab")
    }
}

impl Chain for Popup {
    fn chain(&self) -> &PopupMenu {
        &self.chain
    }
    fn chain_mut(&mut self) -> &mut PopupMenu {
        &mut self.chain
    }
}

pub struct Layout {
    pub toolbar: Rect,
    pub buttons: Vec<Rect>,
    pub strip: DocLayout,
    pub grid: Rect,
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

/// Where the cells go: the client under the chrome.
pub fn grid_rect(ctx: &Ctx, caption: bool) -> Rect {
    let (tb, st) = heights(&ctx.theme, caption);
    let (w, h) = ctx.size;
    Rect::new(0, tb + st, w, (h - tb - st).max(0))
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
    Layout {
        toolbar,
        buttons,
        strip: tabstrip::doc_layout(&ctx.text, px, strip, area, ctx.theme.text),
        grid: grid_rect(ctx, caption),
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
                let Some(press) = self.w.chrome.press.take() else {
                    return Reply::none();
                };
                self.w.tabs.strip.held = false;
                self.w.tabs.strip.dragging = None;
                self.w.chrome.bar.pressed = None;
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
        let tree = MenuTree::flat(items);
        let chain = PopupMenu::open(&tree, at, &menu_ctx(ctx), false);
        self.w.chrome.menu = Some(Popup {
            tree,
            chain,
            choices,
        });
        self.w.full = true;
    }

    fn menu_pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        let Some(m) = &mut self.w.chrome.menu else {
            return Reply::none();
        };
        let action = match p {
            PointerInput::Motion { x, y } => match m.chain.motion(&m.tree, x, y, &menu_ctx(ctx)) {
                true => MenuAction::Redraw,
                false => MenuAction::None,
            },
            PointerInput::Press { x, y, .. } => m.chain.click(&m.tree, x, y, &menu_ctx(ctx)),
            _ => MenuAction::None,
        };
        self.menu_action(action)
    }

    pub(crate) fn menu_key(&mut self, k: &KeyInput, ctx: &Ctx) -> Reply {
        let key = match k.sym {
            keysym::ESCAPE => MenuKey::Escape,
            keysym::UP | keysym::KP_UP => MenuKey::Up,
            keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
            keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
            keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
            keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
            _ => match k.utf8.as_deref().and_then(|s| s.chars().next()) {
                Some(c) if c.is_alphanumeric() => MenuKey::Char(c),
                _ => return Reply::none(),
            },
        };
        let Some(m) = &mut self.w.chrome.menu else {
            return Reply::none();
        };
        let action = m.chain.key(&m.tree, key, &menu_ctx(ctx));
        self.menu_action(action)
    }

    fn menu_action(&mut self, action: MenuAction) -> Reply {
        match action {
            MenuAction::None => Reply::none(),
            MenuAction::Redraw => Reply::redraw(),
            MenuAction::Close => self.close_menu(),
            MenuAction::Chosen(path) => {
                let choice = self
                    .w
                    .chrome
                    .menu
                    .as_ref()
                    .and_then(|m| m.choices.get(path[0]).cloned().flatten());
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
            m.chain
                .draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
        }
    }
}

fn menu_ctx(ctx: &Ctx) -> MenuCtx<'_> {
    MenuCtx {
        text: &ctx.text,
        theme: &ctx.theme,
        px: ctx.theme.fonts.size_px,
        within: Rect::new(0, 0, ctx.size.0, ctx.size.1),
    }
}
