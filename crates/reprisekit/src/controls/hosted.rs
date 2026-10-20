//! A menu or drop list a window shows on popups of its own, one per level, so it shows whole past
//! the window's edges. The window keeps the model and its input; a host borrows it to place each
//! level where its compositor put it and to draw the levels apart from the window.

use crate::controls::menu::{PopupMenu, level_shadow};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

/// What a level's popup opens off: in the owner's coordinates for a root, in the parent level's
/// for a submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Right and down from the point, flipping or sliding to stay on screen.
    Point(i32, i32),
    /// Under the rect's left end, or over the rect when there is no room below.
    Below(Rect),
    /// Right of a parent's row, or left of it when there is no room: `rect` spans the row, and
    /// the level's top sits `dy` below the rect's.
    Beside { rect: Rect, dy: i32 },
}

/// One level as its model has it: `rect` in the owner's coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub rect: Rect,
    pub anchor: Anchor,
}

impl Spot {
    /// The spot `dx`, `dy` further right and down, its anchor along with it.
    pub fn moved(self, dx: i32, dy: i32) -> Spot {
        let m = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.width, r.height);
        Spot {
            rect: m(self.rect),
            anchor: match self.anchor {
                Anchor::Point(x, y) => Anchor::Point(x + dx, y + dy),
                Anchor::Below(r) => Anchor::Below(m(r)),
                Anchor::Beside { rect, dy: d } => Anchor::Beside {
                    rect: m(rect),
                    dy: d,
                },
            },
        }
    }
}

/// A model a host shows on popups.
pub trait Hosted {
    /// Its levels, root first.
    fn spots(&self) -> Vec<Spot>;
    /// Moves `level` to `origin`, owner's coordinates, where the compositor put it.
    fn place(&mut self, level: usize, origin: (i32, i32));
    /// How far right and down of a level's rect its shadow reaches, logical px.
    fn shadow(&self, theme: &Theme) -> i32;
    /// Level `level` with its shadow, moved `by` logical px. Access keys underline as
    /// `theme.access_keys` says, as in the owner's draw.
    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &Theme,
        text: &TextRenderer,
    );
    /// The owner's own draw leaves the levels out from now on.
    fn host(&mut self);
    /// Closes it without a pick: the compositor dismissed the popups.
    fn dismiss(&mut self);
}

impl<T: Hosted + ?Sized> Hosted for &mut T {
    fn spots(&self) -> Vec<Spot> {
        (**self).spots()
    }
    fn place(&mut self, level: usize, origin: (i32, i32)) {
        (**self).place(level, origin);
    }
    fn shadow(&self, theme: &Theme) -> i32 {
        (**self).shadow(theme)
    }
    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &Theme,
        text: &TextRenderer,
    ) {
        (**self).draw_level(level, c, by, theme, text);
    }
    fn host(&mut self) {
        (**self).host();
    }
    fn dismiss(&mut self) {
        (**self).dismiss();
    }
}

/// A menu chain, or a model holding one beside what it belongs to.
pub trait Chain {
    fn chain(&self) -> &PopupMenu;
    fn chain_mut(&mut self) -> &mut PopupMenu;
}

/// An open menu held in an `Option`: dismissing it empties the option.
impl<T: Chain> Hosted for Option<T> {
    fn spots(&self) -> Vec<Spot> {
        self.as_ref().map(|t| t.chain().spots()).unwrap_or_default()
    }
    fn place(&mut self, level: usize, origin: (i32, i32)) {
        if let Some(t) = self {
            t.chain_mut().place(level, origin);
        }
    }
    fn shadow(&self, theme: &Theme) -> i32 {
        level_shadow(theme)
    }
    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &Theme,
        text: &TextRenderer,
    ) {
        if let Some(t) = self {
            t.chain()
                .draw_level(level, by, c, theme, text, theme.access_keys);
        }
    }
    fn host(&mut self) {
        if let Some(t) = self {
            t.chain_mut().host();
        }
    }
    fn dismiss(&mut self) {
        *self = None;
    }
}
