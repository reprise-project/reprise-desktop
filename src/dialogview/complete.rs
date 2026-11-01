//! Path completion in Run and Browse's File name; folder reads run on the file jobs' threads, so
//! a mount that stopped answering holds a thread and never the loop.

use std::path::{Path, PathBuf};

use crate::textedit::TextEdit;

use reprisekit::controls::pathcomplete::{Entry, PathComplete, Want};

use super::*;

impl State {
    /// Starts the folder reads the dialogs' completions want.
    pub(crate) fn run_path_completions(&mut self) {
        let wants: Vec<(u64, Want, Option<PathBuf>)> = self
            .dialogs
            .views
            .iter_mut()
            .filter_map(|v| {
                let (pc, _, base) = completion(&mut v.dialog)?;
                Some((v.id, pc.take_want()?, base))
            })
            .collect();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        for (id, want, base) in wants {
            let dir = reprisefs::complete::folder(&want.dir, base.as_deref(), home.as_deref());
            self.spawn_fs_job(move || {
                let entries: Vec<Entry> = dir
                    .map(|d| reprisefs::complete::entries(&d))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| Entry {
                        name: e.name,
                        dir: e.dir,
                    })
                    .collect();
                Box::new(move |state: &mut State| state.path_listing(id, want, entries))
            });
        }
    }

    fn path_listing(&mut self, id: u64, want: Want, entries: Vec<Entry>) {
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        let changed = completion(&mut self.dialogs.views[i].dialog)
            .is_some_and(|(pc, edit, _)| pc.set_listing(edit, &want, entries));
        if changed {
            self.redraw_dialog(i);
        }
        self.run_path_completions();
    }
}

/// The dialog's completion, its edit, and the folder a relative path names from.
fn completion(d: &mut Dialog) -> Option<(&mut PathComplete, &mut TextEdit, Option<PathBuf>)> {
    if d.picker().is_some() {
        let p = d.picker_mut()?;
        let base = p.current().map(Path::to_path_buf);
        let (pc, edit) = p.dlg.completion();
        return Some((pc, edit, base));
    }
    let f = d.field.as_mut()?;
    Some((&mut f.complete, f.combo.edit.as_mut()?, None))
}
