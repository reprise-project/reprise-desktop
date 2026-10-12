//! What a widget paints with: the [`Look`] a theme draws in (Classic's own
//! code, or a skinned [`Style`]), the [`Paint`] a widget gets (colours,
//! look, scale, what sits behind it) and the theme's accessors for them.
//! [`crate::look`] dispatches each surface on these and re-exports them.

use crate::raster::{Canvas, Rect};
use crate::style::{GlassSpec, Style};
use crate::taskbar::TaskbarShape;
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
    /// Whether the window painted into has the keyboard. A style with
    /// [`Style::default_needs_active`] draws a default push button as a
    /// plain one while it is false.
    pub active: bool,
    /// How a form laid out for 100% text grows ([`Theme::text`]).
    pub text: crate::theme::TextUnits,
}

impl<'a> Paint<'a> {
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
    }
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
        }
    }

    pub fn glass(&self) -> Option<GlassSpec> {
        self.look.style().and_then(|s| s.glass)
    }
}
