//! The glass under a sheet, kept while its size and tint hold.

use crate::controls::tooltip::NotifyHit;
use crate::raster::{Canvas, Rect};
use crate::style::{NotifyLook, SheetSpec};
use crate::text::TextRenderer;
use crate::textedit::{Caret, Preedit, TextEdit};
use crate::theme::{Color, Theme};

use super::draw::{SheetIcons, draw_parts};
use super::{SheetLayout, notify_look};

/// A sheet's glass under each rim, kept while the sheet keeps its size, shape, tint and scale, so
/// a redraw for its text, caret or a lit pill paints only the parts. A host keeps one per sheet.
#[derive(Default)]
pub struct SheetGlass {
    pub(super) kept: Vec<(GlassKey, Canvas)>,
    pub(super) builds: u32,
    /// The glass a Droplet at rest shows, kept through [`SheetGlass::keep_ends`].
    pub(super) rest: Option<GlassKey>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct GlassKey {
    pub(super) spec: SheetSpec,
    pub(super) scale: i32,
    dark: bool,
}

impl GlassKey {
    pub(super) fn new(
        lay: &SheetLayout,
        tint: Option<Color>,
        lit: bool,
        s: i32,
        dark: bool,
    ) -> GlassKey {
        GlassKey {
            spec: sheet_spec(lay, tint, lit, s),
            scale: s,
            dark,
        }
    }

    pub(super) fn lit(&self) -> bool {
        self.spec.hot_close
    }

    fn under_either_rim(&self, other: &GlassKey) -> bool {
        let lit = |k: &GlassKey| GlassKey {
            spec: SheetSpec {
                hot_close: true,
                ..k.spec
            },
            ..*k
        };
        lit(self) == lit(other)
    }
}

impl SheetGlass {
    /// [`super::draw_sheet`] on the kept glass.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        theme: &Theme,
        text: &TextRenderer,
        lay: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        pushed: Option<NotifyHit>,
        hot: Option<NotifyHit>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Canvas {
        let Some(look) = notify_look(theme) else {
            return Canvas::new(lay.width.max(1) as u32, lay.height.max(1) as u32);
        };
        let s = theme.scale.max(1);
        let key = GlassKey::new(lay, tint, hot.is_some(), s, look.dark);
        self.kept.retain(|(k, _)| k.under_either_rim(&key));
        let mut c = self.get(&look, key);
        draw_parts(&mut c, theme, text, &look, lay, icons, pushed, hot, reply);
        c
    }

    pub(super) fn get(&mut self, look: &NotifyLook, key: GlassKey) -> Canvas {
        self.ensure(look, key);
        self.find(&key)
            .cloned()
            .unwrap_or_else(|| unreachable!("painted above"))
    }

    pub(super) fn ensure(&mut self, look: &NotifyLook, key: GlassKey) {
        if self.find(&key).is_none() {
            let c = (look.sheet)(look, &key.spec, key.scale);
            self.builds += 1;
            self.kept.push((key, c));
        }
    }

    pub(super) fn find(&self, key: &GlassKey) -> Option<&Canvas> {
        self.kept.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    /// Keeps only the glass of `ends` under the rim `lit`, and the glass at rest.
    pub(super) fn keep_ends(&mut self, ends: [GlassKey; 2], lit: bool) {
        let rest = self.rest;
        self.kept.retain(|(k, _)| {
            ends.iter().any(|e| k.under_either_rim(e)) && (k.lit() == lit || Some(*k) == rest)
        });
    }

    /// How many times the glass was painted.
    pub fn builds(&self) -> u32 {
        self.builds
    }

    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.kept.iter().map(|(_, c)| c.pixels.len() * 4).sum()
    }
}

pub(super) fn sheet_spec(lay: &SheetLayout, tint: Option<Color>, lit: bool, s: i32) -> SheetSpec {
    let design = |r: Rect| Rect::new(r.x / s, r.y / s, r.width / s, r.height / s);
    let sh = design(lay.sheet);
    let breakout = (lay.footprint != lay.sheet).then(|| {
        let i = design(lay.icon);
        Rect::new(i.x - sh.x, i.y - sh.y, i.width, i.height)
    });
    SheetSpec {
        shape: lay.shape,
        size: (sh.width, sh.height),
        tint,
        hot_close: lit,
        breakout,
    }
}
