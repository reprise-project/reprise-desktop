//! A colour button's Other...: the Color dialog on the button's colour, whose OK sets it as a
//! swatch pick would. The custom colours are kept in the state file.

use crate::SettingsApp;
use repriseapp::colordlg::ColorDialog;
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::colordlg::Outcome;
use reprisekit::controls::form::{self, Control};
use reprisekit::controls::propsheet;

impl SettingsApp {
    /// Whether item `id` of page `p` is a colour button.
    pub(crate) fn color_button(&self, p: usize, id: &str) -> bool {
        self.sheet
            .pages
            .get(p)
            .and_then(|f| f.item(id))
            .is_some_and(|i| matches!(i.control, Control::Color(_)))
    }

    pub(crate) fn open_color(&mut self, p: usize, id: &'static str) {
        let Some(Control::Color(b)) = self.sheet.pages[p].item(id).map(|i| &i.control) else {
            return;
        };
        self.color_dlg = Some((ColorDialog::new(b.color, self.custom_colors), p, id));
    }

    pub(crate) fn color_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some((d, p, id)) = &mut self.color_dlg else {
            return Reply::none();
        };
        let (p, id) = (*p, *id);
        let out = d.event(&ev, ctx);
        if d.dlg.custom != self.custom_colors {
            self.custom_colors = d.dlg.custom;
            if let Some(path) = &self.state_path {
                crate::winstate::save_custom_colors(path, &self.custom_colors);
            }
        }
        match out {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => {
                self.color_dlg = None;
                Reply::redraw_all()
            }
            Outcome::Ok(c) => {
                self.color_dlg = None;
                if let Some(Control::Color(b)) =
                    self.sheet.pages[p].item_mut(id).map(|i| &mut i.control)
                {
                    b.color = c;
                }
                self.sheet_event(propsheet::Event::Page(p, form::Event::Changed(id)));
                Reply::redraw_all()
            }
        }
    }
}
