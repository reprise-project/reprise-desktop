//! A program's Color dialog: `reprisekit`'s [`colordlg`] in a child window, fed its events.

use crate::keys::{dialog_key, form_input};
use crate::{BTN_LEFT, Child, Ctx, Event, PointerInput, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::colordlg::{self, CUSTOM, ColorDlg, Outcome};
use reprisekit::controls::form::Form;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, TextUnits};

pub struct ColorDialog {
    pub dlg: ColorDlg,
    pub form: Form,
}

impl ColorDialog {
    pub fn new(color: Color, custom: [Color; CUSTOM]) -> ColorDialog {
        let dlg = ColorDlg::new(color, custom);
        ColorDialog {
            form: colordlg::form(&dlg),
            dlg,
        }
    }

    /// The dialog's window, modal over its program's, at text size `units`.
    pub fn child(&self, id: WinId, units: TextUnits) -> Child {
        Child {
            id,
            title: reprisekit::kit_tr!("colordlg-title"),
            size: units.size(colordlg::client(&self.dlg)),
            fixed: true,
            modal: true,
        }
    }

    pub fn event(&mut self, ev: &Event, ctx: &Ctx) -> Outcome {
        let (dlg, f) = (&mut self.dlg, &mut self.form);
        match ev {
            Event::CloseRequest => return Outcome::Cancel,
            Event::Key(k) => {
                if let Some(o) = dialog_key(k).and_then(|key| dlg.key(f, key)) {
                    return o;
                }
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                if let Some(o) = dlg.press(f, *x, *y) {
                    return o;
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => {
                if let Some(o) = dlg.motion(f, *x, *y) {
                    return o;
                }
            }
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT, ..
            }) if dlg.release() => return Outcome::Redraw,
            _ => {}
        }
        match form_input(f, ev, ctx) {
            Some(e) => dlg.form_event(f, e),
            None => Outcome::None,
        }
    }

    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, px: u32) {
        c.fill_rect(c.bounds(), p.cs.button_face);
        colordlg::draw_window(c, text, p, &self.form, &self.dlg, px);
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32) {
        let root = reprisekit::access::ROOT;
        if let Some(f) = colordlg::access_window(&self.form, &self.dlg, tree, root, text, px) {
            tree.set_focus(f);
        }
    }
}
