//! Laying out and drawing dialogs, their icons and their access trees.

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::dialog::{self, DialogLayout, Style};
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};

use super::*;

impl State {
    pub(crate) fn redraw_top_dialog(&mut self) {
        if let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.redraw_dialog(i);
        }
    }

    /// Redraw every dialog, for a keyboard or theme change.
    pub fn redraw_dialogs(&mut self) {
        for i in 0..self.dialogs.views.len() {
            self.redraw_dialog(i);
        }
    }

    /// The caret timer fired: repaint only the field over the kept raster; full redraw if none or the scale moved.
    pub fn blink_dialog(&mut self) {
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let v = &self.dialogs.views[i];
        let scale = self.draw_scale_at(Some(&v.output)).max(1);
        let Some((still, at)) = v.still_canvas.as_ref().filter(|(_, at)| *at == scale) else {
            self.redraw_dialog(i);
            return;
        };
        let mut canvas = still.clone();
        let keyed = self.dialog_takes_typing();
        if self.caret.on {
            let theme = self.config.theme.scaled(*at);
            dialog::draw_field(
                &mut canvas,
                &theme,
                self.text.for_output(theme.scale),
                &v.layout,
                &v.dialog,
                true,
                keyed,
            );
        }
        let (buffer, tree) = (v.buffer, v.tree);
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        self.redraw_nodes(Vec::new(), &[tree.into()]);
    }

    /// Fit dialog `index` to the theme and its output's usable area and scale and lay it out again;
    /// one whose new size runs past the area's edge moves back inside.
    pub fn refit_dialog(&mut self, index: usize) {
        let Some(output) = self.dialogs.views.get(index).map(|v| v.output.clone()) else {
            return;
        };
        let area = self.usable_area(Some(&output));
        let scale = self.draw_scale_at(Some(&output));
        let State {
            config,
            text,
            dialogs,
            ..
        } = self;
        let v = &mut dialogs.views[index];
        let theme = &config.theme;
        let before = v.layout.size;
        v.dialog.fit_in(theme, text, (area.width, area.height));
        v.layout = DialogLayout::compute_at(theme, text, &v.dialog, scale);
        if v.layout.size != before
            && let Some((x, y)) = refit_origin(&v.dialog, (v.x, v.y), v.layout.size, area)
        {
            self.move_dialog(index, x, y);
        }
    }

    pub fn invalidate_dialogs(&mut self) {
        for v in &mut self.dialogs.views {
            v.icons = None;
        }
    }

    /// Draw dialog `index` again: raster, caret on the blink's shown half while typing reaches it, glass.
    pub fn redraw_dialog(&mut self, index: usize) {
        let top = index + 1 == self.dialogs.views.len();
        let active = top && self.dialog_has_keyboard();
        let keyed = top && self.dialog_takes_typing();
        let caret_on = self.caret.on && keyed;
        let waits = self.icons.waits();
        let Some(v) = self.dialogs.views.get(index) else {
            return;
        };
        let tree = v.tree;
        let was = self.scene.outputs_of(tree);
        let output = v.output.clone();
        let base = self.config.theme.clone();
        let scale = self.draw_scale_at(Some(&output)).max(1);
        let theme = base.scaled(scale);
        let wanted = crate::glass::wanted(&self.config);
        let preedit = active
            .then(|| self.ime_field_preedit(crate::caret::Editor::Dialog))
            .flatten();
        let State {
            dialogs,
            icons: cache,
            text,
            scene,
            glass,
            ..
        } = self;
        let v = &mut dialogs.views[index];
        let px = dialog::ICON_PX * scale;
        let key = icon_key(&v.dialog, scale);
        let mut undecoded = Vec::new();
        if v.icons.as_ref().map(|(k, _)| k) != Some(&key) {
            let mut get = |n: &Option<String>, px: i32| {
                n.as_deref()
                    .and_then(|n| cached(cache, n, px, &mut undecoded))
            };
            let icons = DialogIcons {
                icon: get(&key.icon, px),
                caption: get(&key.icon, theme.icon_size()),
            };
            v.icons = Some((key, icons));
        }
        let Some((_, icons)) = v.icons.as_ref() else {
            return;
        };
        // A form's focused edit field draws its caret on the blink's shown half.
        if let Some(page) = v.dialog.form_mut().map(|f| f.open_page_mut()) {
            page.caret = caret_on;
        }
        let mut canvas = dialog::draw(
            &theme,
            text.for_output(scale),
            &v.layout,
            icons.icon.as_ref(),
            icons.caption.as_ref(),
            &v.dialog,
            false,
            v.dialog.lit(),
            v.cues,
            active,
            keyed,
            preedit.as_ref(),
        );
        v.keyed = keyed;
        // An ellipsized label's tip, over everything the dialog drew.
        reprisekit::controls::form::draw_tip(&mut canvas, text.for_output(scale), &theme);
        // What the backdrop sees: the client area.
        let part = wanted
            .then(|| client_part(&canvas, &v.layout, scale))
            .flatten();
        match part {
            Some((c, at)) => {
                let shown = Some((c, at, scale));
                if v.content_shown != shown {
                    if let Some((c, at, _)) = &shown {
                        upload_scaled(scene, v.content, c, scale);
                        scene.set_position(v.content, at.x, at.y);
                    }
                    v.content_shown = shown;
                }
            }
            None => {
                if v.content_shown.take().is_some() {
                    scene.buffer_clear(v.content);
                }
            }
        }
        if v.dialog.field.is_some() {
            v.still_canvas = Some((canvas.clone(), scale));
            if caret_on {
                dialog::draw_field(
                    &mut canvas,
                    &theme,
                    text.for_output(theme.scale),
                    &v.layout,
                    &v.dialog,
                    true,
                    keyed,
                );
            }
        }
        upload_scaled(scene, v.buffer, &canvas, scale);
        let pane = glass_pane(&base, wanted, &v.layout);
        v.backdrop.set_surface(match active {
            true => reprisekit::glass::tint::Surface::Active,
            false => reprisekit::glass::tint::Surface::Inactive,
        });
        v.backdrop.set(scene, glass, v.glass_tree, pane);
        let origin = (v.x, v.y);
        let id = v.id;
        let decode = next_decode(&mut v.decodes, undecoded, std::time::Instant::now());
        if active {
            let spot = canvas.take_spot();
            self.ime_field_drawn(crate::caret::Editor::Dialog, spot, origin, scale);
        }
        match decode {
            Decode::Start(read, files) => {
                self.decode_dialog_icons(id, read, files);
                self.relook_dialog_icons(id, crate::fsjobs::STUCK);
            }
            Decode::Retry(wait) => self.relook_dialog_icons(id, wait),
            Decode::Wait => {}
        }
        self.note_icon_waits(waits, crate::icon::IconWaiter::Dialog(id));
        self.redraw_nodes(was, &[tree.into()]);
        crate::access::touch(self);
    }

    /// Decodes dialog `id`'s icon files on a file job, since one can sit on a mount that never
    /// answers, handing each to the loop as it lands so one dead file holds back no other.
    fn decode_dialog_icons(&mut self, id: u64, read: u64, files: IconFiles) {
        let tx = self.fsjobs.sender();
        self.spawn_fs_job(move || {
            let mut rest = Vec::new();
            for (f, px) in files {
                let c = reprisekit::icontheme::decode_file(std::path::Path::new(&f), px, 1);
                let art = vec![(f, px, c)];
                match &tx {
                    Some(tx) => {
                        let _ = tx.send(Box::new(move |state: &mut State| {
                            state.dialog_icons_decoded(id, read, art, false)
                        }));
                    }
                    None => rest.extend(art),
                }
            }
            Box::new(move |state: &mut State| state.dialog_icons_decoded(id, read, rest, true))
        });
    }

    /// Takes icon files job `read` decoded for dialog `id`, and draws the dialog with them.
    fn dialog_icons_decoded(
        &mut self,
        id: u64,
        read: u64,
        art: Vec<(String, i32, Option<Canvas>)>,
        done: bool,
    ) {
        let files: IconFiles = art.iter().map(|(f, px, _)| (f.clone(), *px)).collect();
        self.icons.put_files(art);
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        let v = &mut self.dialogs.views[i];
        if done {
            v.decodes.land(read);
        } else if let Some(rest) = v.decodes.reading_mut(read) {
            rest.retain(|f| !files.contains(f));
        }
        v.icons = None;
        self.redraw_dialog(i);
    }

    /// Theme icons landed while dialog `id` showed a stand-in for one: it looks its icons up again.
    pub(crate) fn dialog_icons_landed(&mut self, id: u64) {
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        self.dialogs.views[i].icons = None;
        self.redraw_dialog(i);
    }

    /// Looks dialog `id`'s icons up again in `wait`, when a decode job still running will have
    /// turned stuck and a newer one may take the files it holds back.
    fn relook_dialog_icons(&mut self, id: u64, wait: std::time::Duration) {
        let added = self
            .handle
            .insert_source(Timer::from_duration(wait), move |_, _, state| {
                if let Some(i) = state.dialog_index(id) {
                    state.dialogs.views[i].icons = None;
                    state.redraw_dialog(i);
                }
                TimeoutAction::Drop
            });
        if let Err(e) = added {
            log::warn!("dialog icon retry timer: {e}");
        }
    }

    /// Dialog `id`'s tree, placed where it stands.
    pub fn dialog_access(&self, id: u64) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.dialogs.views.iter().find(|v| v.id == id)?;
        let (w, h) = v.layout.size;
        let rect = Rect::new(v.x, v.y, w, h);
        let mut tree = crate::access::root(Role::Dialog, &v.dialog.title, rect);
        if matches!(v.dialog.style, Style::Welcome(_))
            && let Some(n) = tree.get_mut(ROOT)
        {
            n.set_modal();
        }
        let px = self.config.theme.fonts.size_px;
        let focus = dialog::access(&v.dialog, &v.layout, &self.text, px, &mut tree);
        tree.set_focus(focus);
        Some(tree)
    }
}
