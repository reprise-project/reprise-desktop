//! Browse dialogs: opening them and running their folder reads and thumbnails.

use std::collections::BTreeSet;

use crate::compositor::State;

/// The ping every Browse dialog's folder watch wakes the loop with.
pub(super) fn watch_on_loop(state: &mut State) {
    let (ping, source) = match smithay::reexports::calloop::ping::make_ping() {
        Ok(p) => p,
        Err(e) => return log::warn!("browse: no folder watch ping: {e}"),
    };
    let added = (state.handle).insert_source(source, |_, _, state| state.pickers_changed());
    match added {
        Ok(_) => state.dialogs.picker_wake = Some(ping),
        Err(e) => log::warn!("browse: {}", e.error),
    }
}
use crate::dialog::{self, Dialog, DialogLayout, Outcome, Placement};

impl State {
    /// Run's Browse...: on the folder the last one picked from, else the home folder, in the
    /// corner by the Start button, 3 px in from the screen edge.
    pub(super) fn browse_run(&mut self) {
        let start = (self.wm.file_dialog.folder.clone())
            .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
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
    fn open_browse(&mut self, mut d: Dialog, kept: Option<(i32, i32)>) {
        if let (Some(ping), Some(p)) = (self.dialogs.picker_wake.clone(), d.picker_mut()) {
            match reprisefs::folderwatch::Watch::new(move || ping.ping()) {
                Ok(w) => p.watch_with(w),
                Err(e) => log::warn!("browse: no folder watch: {e}"),
            }
        }
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
        let ops: Vec<(u64, crate::filepick::Op)> = (self.dialogs.views.iter_mut())
            .filter_map(|v| Some((v.id, v.dialog.picker_mut()?.take_ops())))
            .flat_map(|(id, ops)| ops.into_iter().map(move |op| (id, op)))
            .collect();
        for (id, op) in ops {
            self.spawn_fs_job(move || {
                let done = op.run();
                Box::new(move |state| state.picker_op_done(id, done))
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

    /// Lists each Browse dialog again whose folder watch saw an entry change, or follows its
    /// folder where it went.
    pub(crate) fn pickers_changed(&mut self) {
        let mut any = false;
        for v in &mut self.dialogs.views {
            if let Some(p) = v.dialog.picker_mut() {
                any |= p.watch_due();
            }
        }
        if any {
            self.run_picker_requests();
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
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        if p.thumbs(generation, done) {
            self.redraw_dialog(i);
        }
    }

    /// Work on Browse dialog `id`'s folder is done: its folder is read again, and a refused
    /// rename says why in a box over the dialog.
    fn picker_op_done(&mut self, id: u64, done: crate::filepick::OpDone) {
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        let refused = match &done {
            crate::filepick::OpDone::Refused { why, .. } => Some(why.clone()),
            _ => None,
        };
        p.op_done(done);
        self.run_picker_requests();
        self.redraw_dialog(i);
        if let Some(why) = refused {
            self.open_dialog_over_id(crate::dialog::Dialog::error("Rename", why), Some(id));
        }
    }

    /// A folder read for Browse dialog `id` is done; a pick acts as if the dialog were on top.
    fn picker_reply(&mut self, id: u64, reply: crate::filepick::Reply) {
        let Some(i) = self.dialog_index(id) else {
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
