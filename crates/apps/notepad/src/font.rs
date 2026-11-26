//! Format > Font as this host runs it: the families listed, the sample's renderer for each
//! pick, and what OK does to the text box. The dialog itself is `reprisekit::controls::fontdlg`.

use super::*;
use reprisekit::controls::fontdlg::{self, Pick};
use reprisekit::theme::FontConfig;

fn sample_font(family: &str) -> TextRenderer {
    TextRenderer::load(&FontConfig {
        family: vec![family.to_string()],
        antialias: true,
        ..FontConfig::default()
    })
}

impl Notepad {
    /// The font the settings name, as the dialog picks it: a font with no family named shows
    /// the face it is set in.
    fn settings_pick(&self, families: &[String]) -> Pick {
        let family = match self.settings.font.is_empty() {
            true => window::default_family(families)
                .unwrap_or_default()
                .to_string(),
            false => self.settings.font.clone(),
        };
        Pick {
            family,
            style: self.settings.style(),
            size: self.settings.size,
        }
    }

    pub(crate) fn open_font(&mut self) {
        let families = reprisekit::text::families();
        let pick = self.settings_pick(&families);
        self.font = Some(fontdlg::form(&families, &pick));
        self.font_sample = Some((pick.family.clone(), sample_font(&pick.family)));
    }

    /// OK: the text box takes the pick and Notepad keeps it. The face plain text is set in by
    /// default is kept as no family named, so it stays drawn as that default draws it.
    pub(crate) fn apply_font(&mut self, pick: &Pick, ctx: &Ctx) {
        let families = reprisekit::text::families();
        let default = window::default_family(&families);
        self.settings.font = match default == Some(pick.family.as_str()) {
            true => String::new(),
            false => pick.family.clone(),
        };
        self.settings.bold = pick.style.bold();
        self.settings.italic = pick.style.italic();
        self.settings.size = pick.size;
        self.mono = edit_font(&self.settings);
        self.save_settings();
        self.follow(ctx);
        self.scroll_to_caret(ctx);
    }

    pub(crate) fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            settings::save(path, &self.settings);
        }
    }

    pub(crate) fn font_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let px = self.forms.fonts.size_px;
        match form_event(&mut self.font, &ev, &ctx.text, px, ctx.caret_on) {
            FormStep::Closed => {
                self.close_font();
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.font_outcome(out, ctx),
        }
    }

    fn font_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        let Some(f) = self.font.as_mut() else {
            return Reply::none();
        };
        match out {
            form::Event::Cancel | form::Event::Pressed(fontdlg::CANCEL) => {
                self.close_font();
                Reply::redraw_all()
            }
            // A field that names no font keeps the dialog up.
            form::Event::Default | form::Event::Pressed(fontdlg::OK) => {
                let Some(pick) = fontdlg::pick(f) else {
                    return Reply::none();
                };
                self.close_font();
                self.apply_font(&pick, ctx);
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            ev => {
                fontdlg::follow(f, &ev);
                let family = fontdlg::pick(f).map(|p| p.family);
                if let Some(family) = family
                    && self
                        .font_sample
                        .as_ref()
                        .is_none_or(|(had, _)| *had != family)
                {
                    let sample = sample_font(&family);
                    self.font_sample = Some((family, sample));
                }
                Reply::redraw()
            }
        }
    }

    fn close_font(&mut self) {
        self.font = None;
        self.font_sample = None;
    }

    /// The dialog with its sample at the text size, as the text box would show the pick at 100%.
    pub(crate) fn draw_font(&self, c: &mut Canvas, ctx: &Ctx) {
        let (Some(f), Some((_, face))) = (&self.font, &self.font_sample) else {
            return;
        };
        let p = Paint {
            s: 1,
            ..self.forms.paint()
        };
        c.fill_rect(c.bounds(), p.cs.button_face);
        let pick = fontdlg::pick(f);
        let style = pick.as_ref().map_or_else(Default::default, |p| p.style);
        let size = pick.as_ref().map_or(self.settings.size, |p| p.size);
        let sample = fontdlg::Sample {
            text: match style.italic() {
                true => face.italic(),
                false => face,
            },
            bold: style.bold(),
            px: reprisekit::theme::text_px(fontdlg::px(size), ctx.text_size),
        };
        let px = self.forms.fonts.size_px;
        fontdlg::draw(c, &self.dialog_text, &p, f, px, &sample);
    }

    pub(crate) fn access_font(&self, tree: &mut Tree) {
        if let Some(f) = &self.font {
            let (text, px) = (&self.dialog_text, self.forms.fonts.size_px);
            f.drawn_with(&self.forms.paint());
            if let Some(focus) = fontdlg::access(f, tree, ROOT, text, px) {
                tree.set_focus(focus);
            }
        }
    }
}
