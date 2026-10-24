//! Browse dialogs: opening them and running their folder reads and thumbnails.

use std::collections::BTreeSet;

use crate::compositor::State;
use crate::dialog::{self, Dialog, DialogLayout, Outcome, Placement};

impl State {
    /// Run's Browse...: on the home folder, in the corner by the Start button, 3 px in from the screen edge.
    pub(super) fn browse_run(&mut self) {
        let start = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let pos = self.pointer_location();
        let out = self.output_at(pos.x, pos.y);
        let scale = self.draw_scale_at(out.as_ref());
        let icons = crate::filepick::icons(&mut self.icons, scale, self.config.theme.text);
        let mut picker =
            crate::filepick::FilePicker::run_command(&start, icons, self.config.recent_files);
        picker.remember_in(repriseconf::filedialog::path());
        let mut d = Dialog::browse(picker);
        let kept = self.restore_browse_in(&mut d);
        let area = self.usable_area(out.as_ref());
        let edge = out
            .as_ref()
            .map_or(self.config.taskbar.position, |o| self.bar_placement(o).edge);
        let size = DialogLayout::compute(&self.config.theme, &self.text, &d).size;
        let (x, y) = dialog::browse_by_start(area, edge, size);
        d.placement = Placement::At(x, y);
        self.open_browse(d, kept);
    }

    /// The Open With chooser's Browse...: "Open With..." on `/usr/bin`, cascaded 3 px right and 22 px below the chooser.
    pub(super) fn browse_program(&mut self) {
        let scale = self
            .dialogs
            .views
            .last()
            .map_or(1, |v| self.draw_scale_at(Some(&v.output)));
        let icons = crate::filepick::icons(&mut self.icons, scale, self.config.theme.text);
        let mut picker = crate::filepick::FilePicker::program(
            std::path::Path::new("/usr/bin"),
            icons,
            self.config.recent_files,
        );
        picker.remember_in(repriseconf::filedialog::path());
        let mut d = Dialog::browse(picker);
        d.title = "Open With...".into();
        let kept = self.restore_browse_in(&mut d);
        if let Some(chooser) = self.dialogs.views.last() {
            d.placement = Placement::At(chooser.x + 3, chooser.y + 22);
        }
        self.open_browse(d, kept);
    }

    /// Sizes Browse dialog `d` from the kept file dialog geometry; the place it goes, if any.
    fn restore_browse_in(&self, d: &mut Dialog) -> Option<(i32, i32)> {
        let layout = DialogLayout::compute(&self.config.theme, &self.text, d);
        let page = layout.page?;
        let pad = (layout.size.0 - page.width, layout.size.1 - page.height);
        self.restore_browse(d.picker_mut()?, pad)
    }

    /// Opens Browse dialog `d` over the top dialog, moved to `kept` once it shows.
    fn open_browse(&mut self, d: Dialog, kept: Option<(i32, i32)>) {
        let before = self.dialogs.views.len();
        self.open_dialog_over(d);
        let top = self.dialogs.views.len();
        if let Some((x, y)) = kept
            && top > before
        {
            self.move_dialog(top - 1, x, y);
        }
    }

    /// Runs every Browse dialog's queued folder reads off the loop, and fetches the thumbnails
    /// of the pictures it lists in Thumbnails or Tiles; replies find dialogs by id.
    pub(super) fn run_picker_requests(&mut self) {
        let jobs: Vec<(u64, crate::filepick::Request)> = self
            .dialogs
            .views
            .iter_mut()
            .filter_map(|v| Some((v.id, v.dialog.picker_mut()?.take_requests())))
            .flat_map(|(id, rs)| rs.into_iter().map(move |r| (id, r)))
            .collect();
        for (id, req) in jobs {
            self.spawn_fs_job(move || {
                let reply = req.run();
                Box::new(move |state| state.picker_reply(id, reply))
            });
        }
        let pictures: Vec<_> = self
            .dialogs
            .views
            .iter_mut()
            .filter_map(|v| Some((v.id, v.dialog.picker_mut()?.take_pictures()?)))
            .collect();
        for (id, (generation, px, paths)) in pictures {
            let to_loop = self.fsjobs.sender();
            self.pickthumbs.want(to_loop, id, generation, px, paths);
        }
    }

    /// Lists each Browse dialog showing one of `due` again.
    pub(crate) fn relist_pickers(&mut self, due: &BTreeSet<std::path::PathBuf>) {
        let mut any = false;
        for v in &mut self.dialogs.views {
            if let Some(p) = v.dialog.picker_mut() {
                any |= p.relist(due);
            }
        }
        if any {
            self.run_picker_requests();
        }
    }

    /// Thumbnails fetched for Browse dialog `id`'s listing `generation`; a dialog closed or
    /// relisted meanwhile drops them.
    pub(crate) fn picker_thumbs(
        &mut self,
        id: u64,
        generation: u64,
        done: Vec<crate::pickthumbs::Done>,
    ) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        if p.thumbs(generation, done) {
            self.redraw_dialog(i);
        }
    }

    /// A folder read for Browse dialog `id` is done; a pick acts as if the dialog were on top.
    fn picker_reply(&mut self, id: u64, reply: crate::filepick::Reply) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let scale = self.draw_scale_at(Some(&self.dialogs.views[i].output));
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        match p.reply(reply, &mut self.icons, scale) {
            Outcome::Nothing => {}
            Outcome::Redraw => {
                self.run_picker_requests();
                self.redraw_dialog(i);
            }
            pick => {
                self.raise_dialog(i);
                self.apply_dialog(pick);
            }
        }
    }
}
