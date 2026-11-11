//! What a widget paints with: the [`Look`] a theme draws in (Classic's own
//! code, or a skinned [`Style`]), the [`Paint`] a widget gets (colours,
//! look, scale, what sits behind it) and the theme's accessors for them.
//! [`crate::look`] dispatches each surface on these and re-exports them.

use crate::raster::{Canvas, Rect};
use crate::style::{GlassSpec, Style};
use crate::taskbar::TaskbarShape;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit, edit_first_visible};
use crate::theme::{ColorScheme, Theme};
use std::sync::Arc;

#[derive(Clone, Default)]
pub enum Look {
    #[default]
    Classic,
    Styled(Arc<Style>),
}

impl Look {
    pub fn style(&self) -> Option<&Style> {
        match self {
            Look::Classic => None,
            Look::Styled(s) => Some(s),
        }
    }

    pub fn at_scale(&self, n: i32) -> Look {
        match self {
            Look::Classic => Look::Classic,
            Look::Styled(s) => Look::Styled(s.at_scale(n)),
        }
    }

    pub fn controls(&self) -> crate::metrics::ControlMetrics {
        self.style().map(|s| s.controls).unwrap_or_default()
    }

    /// Whether this look draws a control under the pointer or held apart
    /// from one at rest: a style with a Hot or Pressed frame for any of
    /// the common widgets, a list header or the dialog's ? box. A host redraws for a
    /// change of the part the pointer lights only then; Classic and a
    /// style with no control art redraw nothing for hover.
    pub fn lights_pointer(&self) -> bool {
        use crate::style::{Part, State};
        const LIT: [Part; 19] = [
            Part::PushButton,
            Part::CheckBox,
            Part::Radio,
            Part::ComboDrop,
            Part::SpinUp,
            Part::SpinDown,
            Part::TrackbarThumb,
            Part::TrackbarThumbV,
            Part::TabItem,
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbV,
            Part::ScrollThumbH,
            Part::ScrollTrackV,
            Part::ScrollTrackH,
            Part::HelpButton,
            Part::HeaderItem,
        ];
        self.style().is_some_and(|st| {
            LIT.iter().any(|&part| {
                st.part(part).is_some_and(|p| {
                    p.states
                        .iter()
                        .any(|s| matches!(s, State::Hot | State::Pressed))
                })
            })
        })
    }
}

impl PartialEq for Look {
    fn eq(&self, o: &Look) -> bool {
        match (self, o) {
            (Look::Classic, Look::Classic) => true,
            (Look::Styled(a), Look::Styled(b)) => a.name == b.name,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Look::Classic => f.write_str("Classic"),
            Look::Styled(s) => write!(f, "Styled({})", s.name),
        }
    }
}

/// What sits behind a control: the dialog face, or a property sheet's
/// page drawn into the given rect (the one [`crate::look::widgets::draw_page`] got).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Backdrop {
    #[default]
    Face,
    Page(Rect),
}

/// What a widget or control needs to paint: the colours, the look, the
/// scale, what sits behind it and whether its window is the active one.
/// [`Theme::paint`] builds one; a caller painting with another scheme or
/// scale overrides a field: `&Paint { s: 2 * p.s, ..*p }`.
#[derive(Clone, Copy)]
pub struct Paint<'a> {
    pub cs: &'a ColorScheme,
    pub look: &'a Look,
    pub s: i32,
    pub back: Backdrop,
    /// Whether the window painted into has the keyboard. While it is false
    /// a default push button draws as a plain one.
    pub active: bool,
    /// How a form laid out for 100% text grows ([`Theme::text`]).
    pub text: crate::theme::TextUnits,
    /// Whether labels underline their access keys ([`Theme::access_keys`]).
    pub access_keys: bool,
}

impl<'a> Paint<'a> {
    /// `label` as drawn: without its access key's marker while the underlines hide.
    pub fn label<'l>(&self, label: &'l str) -> std::borrow::Cow<'l, str> {
        crate::cues::Cues {
            focus: false,
            keys: self.access_keys,
        }
        .label(label)
    }

    pub fn on_page(&self, page: Rect) -> Paint<'a> {
        Paint {
            back: Backdrop::Page(page),
            ..*self
        }
    }

    /// Fill `r` with what sits behind a control there: the face, and over
    /// it the page clipped to `r` when a styled page is under it. Outside
    /// the page, and under a look without its own page, this is the face.
    pub fn fill_back(&self, c: &mut Canvas, r: Rect) {
        c.fill_rect(r, self.cs.button_face);
        if let Backdrop::Page(page) = self.back
            && let Some(st) = self.look.style()
        {
            c.with_clip(r, |c| {
                st.draw_page(c, page, self.s);
            });
        }
    }
}

/// Draw `edit` left-aligned in `inner` from byte `first` (`edit_first_visible`): selection, caret
/// and, focused, the canvas' preedit with its `TextSpot`, which a secret field never shows. The
/// caret sits on the pen's column and inverts what is under it, a glyph's stem included.
#[expect(clippy::too_many_arguments)]
pub fn draw_edit(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    caret: Caret,
    first: usize,
) {
    let Paint { cs, s, .. } = *p;
    let own = edit;
    // With a preedit the text is drawn with it spliced in, selection gone (text-input-v3 removes
    // it).
    let preedit = c
        .preedit()
        .filter(|_| caret.focused() && !own.is_secret())
        .cloned();
    let composed;
    let (edit, first, run, bar_on) = match &preedit {
        Some(p) => {
            let (e, run, bar_on) = p.compose(own);
            composed = e;
            let at = composed.selection().start;
            let from = edit_first_visible(text, &composed, inner.width, px, first.min(at));
            (&composed, from, Some(run), bar_on)
        }
        None => (own, first, None, true),
    };
    let at = |i: usize| {
        if i < first {
            inner.x
        } else {
            inner.x + text.measure(&edit.text[first..i], false, px)
        }
    };
    if edit.text.is_empty() && !edit.cue().is_empty() {
        text.draw(c, inner, edit.cue(), false, cs.gray_text, Align::Left, px);
    } else {
        text.draw(
            c,
            inner,
            &edit.text[first..],
            false,
            cs.button_text,
            Align::Left,
            px,
        );
    }
    let line = text
        .line(inner, false, px)
        .map_or(inner, |(y, h)| Rect::new(inner.x, y, inner.width, h))
        .intersect(&inner);
    let sel = edit.selection();
    let sel = sel.start.max(first)..sel.end.max(first);
    if !sel.is_empty() {
        let (x0, x1) = (at(sel.start), at(sel.end).min(inner.right()));
        let run = Rect::new(x0, line.y, x1 - x0, line.height);
        c.fill_rect(run, cs.hilight);
        text.draw(
            c,
            Rect::new(x0, inner.y, inner.right() - x0, inner.height),
            &edit.text[sel.clone()],
            false,
            cs.hilight_text,
            Align::Left,
            px,
        );
    }
    if let Some(run) = run {
        let base = text
            .baseline(inner, false, px)
            .unwrap_or(inner.bottom() - 3 * s);
        let (x0, x1) = (at(run.start.max(first)), at(run.end.max(first)));
        let ul = Rect::new(x0, base + s, x1 - x0, s).intersect(&inner);
        c.fill_rect(ul, cs.button_text);
    }
    let cx = at(edit.caret()).min(inner.right() - s);
    let bar = Rect::new(cx, line.y, s, line.height);
    if caret.shown() && bar_on {
        c.invert_rect(bar);
    }
    if caret.focused() {
        c.report_spot(own.spot(inner, bar));
    }
}

#[cfg(test)]
pub(crate) fn classic_paint(cs: &ColorScheme, s: i32) -> Paint<'_> {
    static CLASSIC: Look = Look::Classic;
    Paint {
        cs,
        look: &CLASSIC,
        s,
        back: Backdrop::Face,
        active: true,
        text: crate::theme::TextUnits::ONE,
        access_keys: true,
    }
}

/// The bar a caret drew over `off` to give `on`, every pixel of it `off`'s inverted, and whether
/// it covers ink (a pixel other than `paper`).
#[cfg(test)]
#[track_caller]
pub(crate) fn inverted_bar(off: &Canvas, on: &Canvas, paper: u32) -> (Rect, bool) {
    let (w, h) = (off.width as i32, off.height as i32);
    let changed: Vec<(i32, i32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| on.get(x, y) != off.get(x, y))
        .collect();
    assert!(!changed.is_empty(), "the caret changes pixels");
    for &(x, y) in &changed {
        let want = 0xFF00_0000 | (!off.get(x, y) & 0x00FF_FFFF);
        assert_eq!(on.get(x, y), want, "({x}, {y}) is inverted");
    }
    let x0 = changed.iter().map(|p| p.0).min().unwrap();
    let x1 = changed.iter().map(|p| p.0).max().unwrap() + 1;
    let y0 = changed.iter().map(|p| p.1).min().unwrap();
    let y1 = changed.iter().map(|p| p.1).max().unwrap() + 1;
    let bar = Rect::new(x0, y0, x1 - x0, y1 - y0);
    assert_eq!(
        changed.len() as i32,
        bar.width * bar.height,
        "{bar:?} is whole"
    );
    (bar, changed.iter().any(|&(x, y)| off.get(x, y) != paper))
}

impl Theme {
    pub fn taskbar_shape(&self) -> TaskbarShape {
        self.look
            .style()
            .map_or(TaskbarShape::CLASSIC, |s| s.taskbar)
    }

    /// This theme's colours and look, at its own scale, never below 1,
    /// for a window as active as [`Theme::active`] says.
    pub fn paint(&self) -> Paint<'_> {
        Paint {
            cs: &self.colors,
            look: &self.look,
            s: self.scale.max(1),
            back: Backdrop::Face,
            active: self.active,
            text: self.text,
            access_keys: self.access_keys,
        }
    }

    pub fn glass(&self) -> Option<GlassSpec> {
        self.look.style().and_then(|s| s.glass)
    }
}
