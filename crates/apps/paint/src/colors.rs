//! Colors > Edit Colors... and a double-click on a well: the Color dialog on the first or second
//! colour, whose OK sets that colour and the well it came from.

use super::*;
use reprisekit::controls::colordlg::Outcome;
use reprisekit::theme::Color;

/// What the dialog's OK sets: the second colour rather than the first, and the colour box's well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub secondary: bool,
    pub well: Option<usize>,
}

impl Paint {
    /// The dialog on the first colour (the second with `secondary`); `well` is the colour box's
    /// well it changes too, else the well holding that colour, if one does.
    pub(crate) fn open_colors(&mut self, secondary: bool, well: Option<usize>) {
        let px = if secondary {
            self.doc.secondary
        } else {
            self.doc.primary
        };
        let color = art::color_of(px);
        let well = well.or_else(|| self.wells.iter().position(|&c| c == color));
        let shown = well.map_or(color, |i| self.wells[i]);
        self.edit_colors = Some((
            ColorDialog::new(shown, self.custom),
            Target { secondary, well },
        ));
    }

    pub(crate) fn colors_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some((d, target)) = &mut self.edit_colors else {
            return Reply::none();
        };
        let target = *target;
        let out = d.event(&ev, ctx);
        if d.dlg.custom != self.custom {
            self.custom = d.dlg.custom;
            if let Some(path) = &self.state_path {
                state::save_custom_colors(path, &self.custom);
            }
        }
        match out {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => {
                self.edit_colors = None;
                Reply::redraw_all()
            }
            Outcome::Ok(c) => {
                self.edit_colors = None;
                self.set_color(c, target);
                Reply::redraw_all()
            }
        }
    }

    fn set_color(&mut self, c: Color, target: Target) {
        if let Some(i) = target.well {
            self.wells[i] = c;
        }
        let px = c.argb();
        if target.secondary {
            self.doc.secondary = px;
        } else {
            self.doc.primary = px;
        }
    }
}

#[cfg(test)]
mod tests;
