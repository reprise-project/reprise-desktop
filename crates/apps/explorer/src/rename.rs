use crate::{Explorer, Focus};
use repriseapp::{Ctx, KeyInput, Reply};
use reprisefs::listing::Item;
use reprisefs::names::{Verdict, failed_text, verdict};
use reprisekit::textedit::{TextEdit, keysym};

/// A file's stem is selected, as `Path::file_stem` has it: a leading dot is
/// part of the stem (`.bashrc` has no extension to keep).
pub fn initial_edit(name: &str, is_dir: bool) -> TextEdit {
    let mut ed = TextEdit::new(name, true);
    if is_dir {
        return ed;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    if !stem.is_empty() && stem.len() < name.len() {
        ed.set_selection(0, stem.len());
    }
    ed
}

impl Explorer {
    pub(crate) fn after_rename(
        &mut self,
        from: &std::path::Path,
        to: &std::path::Path,
        was_dir: bool,
        ctx: &mut Ctx,
    ) {
        self.undos.push(reprisefs::undo::Undoable::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
        self.sync_undo();
        if !was_dir {
            return;
        }
        if let Some(parent) = to.parent() {
            let place = crate::place::Place::from_path(parent);
            self.refill_tree_node(&place, ctx);
        }
    }

    pub fn begin_rename(&mut self, ctx: &mut Ctx) -> Reply {
        if self.place.read_only() {
            return Reply::none();
        }
        let Some(i) = self.sv.selection.single() else {
            return Reply::none();
        };
        let Some(it) = self.items.get(i) else {
            return Reply::none();
        };
        self.focus = Focus::View;
        self.sv.edit = Some((
            i,
            initial_edit(it.label(), it.is_dir || it.shortcut.is_some()),
        ));
        self.refresh_chrome(ctx);
        Reply::redraw()
    }

    /// Tab commits rather than moving the focus: the box owns the keyboard.
    pub fn rename_key(&mut self, k: &KeyInput, ctx: &mut Ctx) -> Option<Reply> {
        self.sv.edit.as_ref()?;
        if matches!(
            k.sym,
            keysym::RETURN | keysym::KP_ENTER | keysym::TAB | keysym::ISO_LEFT_TAB
        ) {
            return Some(self.commit_rename(ctx));
        }
        if k.sym == keysym::ESCAPE {
            self.sv.edit = None;
            return Some(Reply::redraw());
        }
        let key = k.edit.clone()?;
        if let Some((_, ed)) = self.sv.edit.as_mut() {
            ed.key(key);
        }
        Some(Reply::redraw())
    }

    pub fn commit_rename(&mut self, ctx: &mut Ctx) -> Reply {
        let Some((i, ed)) = self.sv.edit.take() else {
            return Reply::none();
        };
        let Some(it) = self.items.get(i).cloned() else {
            return Reply::redraw();
        };
        if it.shortcut.is_some() {
            return self.rename_shortcut(&it, ed.text.trim(), ctx);
        }
        let existing: Vec<String> = self.items.iter().map(|x| x.name.clone()).collect();
        match verdict(&it.name, &ed.text, &existing) {
            Verdict::Cancel => Reply::redraw(),
            Verdict::Refuse(text) => self.message_box("Rename", text, ctx),
            Verdict::Rename(name) => {
                let to = match it.path.parent() {
                    Some(dir) => dir.join(&name),
                    None => return Reply::redraw(),
                };
                if let Err(e) = reprisefs::hidden::rename(&it.path, &to) {
                    log::warn!("explorer: rename {} failed: {e}", it.path.display());
                    return self.message_box("Rename", failed_text(&it.name, &e), ctx);
                }
                self.after_rename(&it.path, &to, it.is_dir, ctx);
                // `refresh` keeps the selection by path, and the old path is gone.
                let reply = self.refresh(ctx);
                if let Some(j) = self.index_of(&to) {
                    self.sv.selection.set_only(Some(j));
                    self.refresh_chrome(ctx);
                }
                reply
            }
        }
    }

    fn rename_shortcut(&mut self, it: &Item, typed: &str, ctx: &mut Ctx) -> Reply {
        if typed.is_empty() || typed == it.label() {
            return Reply::redraw();
        }
        if let Err(e) = reprisefs::apps::set_launcher_name(&it.path, typed) {
            log::warn!("explorer: rename {} failed: {e}", it.path.display());
            return self.message_box("Rename", failed_text(it.label(), &e), ctx);
        }
        self.refresh(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_opens_with_its_stem_selected_and_a_folder_with_all_of_it() {
        let ed = initial_edit("notes.txt", false);
        assert_eq!(ed.selection(), 0..5);
        let ed = initial_edit("Reports", true);
        assert_eq!(ed.selection(), 0.."Reports".len());
        let ed = initial_edit("README", false);
        assert!(ed.all_selected());
        let ed = initial_edit(".bashrc", false);
        assert!(ed.all_selected());
        let ed = initial_edit("a.tar.gz", false);
        assert_eq!(ed.selection(), 0.."a.tar".len());
    }
}
