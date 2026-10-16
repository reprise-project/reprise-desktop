//! A context menu drawn inside the window's own surface: the tree it walks, the chain
//! reprisekit lays out, and what each row does.

use repriseapp::{Ctx, KeyInput};
use reprisekit::access::Tree;
use reprisekit::access::accesskit::NodeId;
use reprisekit::controls::menu::{MenuAction, MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;

pub struct Popup<A> {
    tree: MenuTree,
    chain: PopupMenu,
    /// Parallel to the rows: `None` for a separator or a greyed row.
    actions: Vec<Option<A>>,
}

/// What a press or key on an open menu came to.
pub enum Step<A> {
    /// A row was chosen; the menu is closed.
    Chosen(Option<A>),
    Closed,
    Redraw,
    None,
}

impl<A: Clone> Popup<A> {
    /// The menu of `rows` at `(x, y)`, or `None` with nothing to show.
    pub fn open(rows: Vec<(MenuItem, Option<A>)>, x: i32, y: i32, ctx: &Ctx) -> Option<Popup<A>> {
        if rows.is_empty() {
            return None;
        }
        let (items, actions): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        let tree = MenuTree::flat(items);
        let chain = PopupMenu::open(&tree, (x, y), &menu_ctx(ctx), false);
        Some(Popup {
            tree,
            chain,
            actions,
        })
    }

    fn step(&self, act: MenuAction) -> Step<A> {
        match act {
            MenuAction::Chosen(path) => Step::Chosen(
                path.first()
                    .and_then(|&i| self.actions.get(i).cloned())
                    .flatten(),
            ),
            MenuAction::Close => Step::Closed,
            MenuAction::Redraw => Step::Redraw,
            MenuAction::None => Step::None,
        }
    }

    pub fn press(&mut self, x: i32, y: i32, ctx: &Ctx) -> Step<A> {
        let act = self.chain.click(&self.tree, x, y, &menu_ctx(ctx));
        self.step(act)
    }

    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        self.chain.motion(&self.tree, x, y, &menu_ctx(ctx))
    }

    pub fn key(&mut self, k: &KeyInput, ctx: &Ctx) -> Step<A> {
        let Some(mk) = menu_key(k) else {
            return Step::None;
        };
        let act = self.chain.key(&self.tree, mk, &menu_ctx(ctx));
        self.step(act)
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        self.chain
            .draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
    }

    pub fn access(&self, tree: &mut Tree, parent: NodeId) -> NodeId {
        self.chain.access(tree, parent, "popup")
    }
}

pub fn item(label: &str, enabled: bool) -> MenuItem {
    MenuItem::item(label).enabled(enabled)
}

fn menu_ctx(ctx: &Ctx) -> MenuCtx<'_> {
    MenuCtx {
        text: &ctx.text,
        theme: &ctx.theme,
        px: ctx.theme.fonts.size_px,
        within: Rect::new(0, 0, ctx.size.0, ctx.size.1),
    }
}

/// The key an open menu takes, if it takes this one.
fn menu_key(k: &KeyInput) -> Option<MenuKey> {
    Some(match k.sym {
        keysym::UP | keysym::KP_UP => MenuKey::Up,
        keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
        keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
        keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
        keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
        keysym::ESCAPE => MenuKey::Escape,
        _ => MenuKey::Char(k.utf8.as_ref()?.chars().next()?),
    })
}
