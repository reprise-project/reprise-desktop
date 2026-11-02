use crate::acting::Action;
use crate::dialogs::Dialog;
use crate::{Explorer, Focus};
use repriseapp::{Ctx, KeyInput, Reply};
use reprisefs::batchrename;
use reprisefs::deadline::Gate;
use reprisefs::listing::Item;
use reprisefs::names::{Verdict, failed_text, verdict};
use reprisekit::controls::batchrename as renamebox;
use reprisekit::controls::form::{self, Form};
use reprisekit::textedit::{TextEdit, keysym};
use std::path::PathBuf;

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

    /// The text field with the keyboard: the rename box, else the Address or Search field while
    /// it has the focus. Its clipboard keys and pastes go there.
    pub(crate) fn text_field(&self) -> Option<&TextEdit> {
        if let Some((_, e)) = &self.sv.edit {
            return Some(e);
        }
        match self.focus {
            Focus::Address => self.address.combo.edit.as_ref(),
            Focus::Search => Some(&self.search_field.edit),
            _ => None,
        }
    }

    /// The clipboard's text into [`Explorer::text_field`]; a name takes no `/`.
    pub(crate) fn paste_text(&mut self, text: &str) -> Reply {
        let changed = match (&mut self.sv.edit, self.focus) {
            (Some((_, e)), _) => e.paste(text, &['/']),
            (None, Focus::Address) => self
                .address
                .combo
                .edit
                .as_mut()
                .is_some_and(|e| e.paste(text, &[])),
            (None, Focus::Search) => self.search_field.edit.paste(text, &[]),
            _ => false,
        };
        if changed {
            Reply::redraw()
        } else {
            Reply::none()
        }
    }

    pub fn begin_rename(&mut self, ctx: &mut Ctx) -> Reply {
        if self.place.read_only() {
            return Reply::none();
        }
        if self.sv.selection.len() > 1 {
            return self.begin_batch_rename();
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
            TextEdit::rename(it.label(), it.is_dir || it.shortcut.is_some()),
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
                let (from, dest) = (it.path.clone(), to.clone());
                let work = move |gate: &Gate| {
                    if !gate.change(&from) {
                        return Ok(());
                    }
                    reprisefs::hidden::rename(&from, &to)
                };
                self.act(
                    ctx,
                    Action::changes("Rename"),
                    work,
                    move |ex, done, ctx| ex.rename_done(&it, dest, done, ctx),
                )
            }
        }
    }

    /// A rename of `it` to `to` answered `done`.
    fn rename_done(
        &mut self,
        it: &Item,
        to: PathBuf,
        done: std::io::Result<()>,
        ctx: &mut Ctx,
    ) -> Reply {
        if let Err(e) = done {
            log::warn!("explorer: rename {} failed: {e}", it.path.display());
            return self.message_box("Rename", failed_text(&it.name, &e), ctx);
        }
        self.after_rename(&it.path, &to, it.is_dir, ctx);
        // `refresh` keeps the selection by path, and the old path is gone.
        let reply = self.refresh(ctx);
        if self.reading.is_some() {
            self.selecting = Some((self.place.clone(), to));
        } else if let Some(j) = self.index_of(&to) {
            self.sv.selection.set_only(Some(j));
            self.refresh_chrome(ctx);
        }
        reply
    }

    fn rename_shortcut(&mut self, it: &Item, typed: &str, ctx: &mut Ctx) -> Reply {
        if typed.is_empty() || typed == it.label() {
            return Reply::redraw();
        }
        let (path, typed) = (it.path.clone(), typed.to_string());
        let work = move |gate: &Gate| {
            if !gate.change(&path) {
                return Ok(());
            }
            reprisefs::apps::set_launcher_name(&path, &typed)
        };
        let label = it.label().to_string();
        let path = it.path.clone();
        self.act(
            ctx,
            Action::changes("Rename"),
            work,
            move |ex, done, ctx| {
                if let Err(e) = done {
                    log::warn!("explorer: rename {} failed: {e}", path.display());
                    return ex.message_box("Rename", failed_text(&label, &e), ctx);
                }
                ex.refresh(ctx)
            },
        )
    }
}

/// The Rename dialog for several items: the form, and the items in view order with the folder's
/// other names, which the preview is planned against.
pub struct BatchRename {
    pub form: Form,
    paths: Vec<PathBuf>,
    items: Vec<(String, bool)>,
    others: Vec<String>,
}

impl BatchRename {
    fn options(&self) -> batchrename::Options {
        let f = renamebox::fields(&self.form);
        batchrename::Options {
            scheme: match f.number {
                true => batchrename::Scheme::Number { base: f.base },
                false => batchrename::Scheme::Replace {
                    find: f.find,
                    with: f.with,
                    ignore_case: f.ignore_case,
                },
            },
            keep_ext: f.keep_ext,
        }
    }

    fn plan(&self) -> Vec<batchrename::Row> {
        let items: Vec<(&str, bool)> = self.items.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        batchrename::plan(&items, &self.others, &self.options())
    }

    /// The preview and OK follow the fields.
    pub fn replan(&mut self) {
        renamebox::follow(&mut self.form);
        let rows = self.plan();
        let shown: Vec<renamebox::Row> = rows
            .iter()
            .map(|r| renamebox::Row {
                old: r.old.clone(),
                new: r.new.clone(),
                problem: r.problem.map(|p| {
                    match p {
                        batchrename::Problem::Invalid => "not a valid name",
                        batchrename::Problem::Taken => "name in use",
                    }
                    .to_string()
                }),
            })
            .collect();
        renamebox::set_preview(&mut self.form, &shown, batchrename::ready(&rows));
    }
}

impl Explorer {
    /// F2 or Rename with several items picked. Launchers are left out: their shown name is the
    /// one inside them, not the file's.
    fn begin_batch_rename(&mut self) -> Reply {
        let picked: Vec<&Item> = self
            .sv
            .selection
            .set
            .iter()
            .filter_map(|&i| self.items.get(i))
            .filter(|it| it.shortcut.is_none())
            .collect();
        let Some(first) = picked.first() else {
            return Reply::none();
        };
        let base = match first.is_dir {
            true => first.name.clone(),
            false => std::path::Path::new(&first.name)
                .file_stem()
                .map_or_else(|| first.name.clone(), |s| s.to_string_lossy().into_owned()),
        };
        let paths: Vec<PathBuf> = picked.iter().map(|it| it.path.clone()).collect();
        let items = picked
            .iter()
            .map(|it| (it.name.clone(), it.is_dir))
            .collect();
        let others = self
            .items
            .iter()
            .filter(|it| !paths.contains(&it.path))
            .map(|it| it.name.clone())
            .collect();
        let form = renamebox::form(&renamebox::Fields {
            number: false,
            find: String::new(),
            with: String::new(),
            ignore_case: false,
            base,
            keep_ext: true,
        });
        let mut form = self.built(form);
        form.focus_id(renamebox::FIND);
        let mut b = BatchRename {
            form,
            paths,
            items,
            others,
        };
        b.replan();
        self.dialog = Some(Dialog::BatchRename(Box::new(b)));
        Reply::redraw_all()
    }

    /// What the Rename dialog's form answered.
    pub(crate) fn batch_rename_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::BatchRename(b)) = self.dialog.as_mut() else {
            return Reply::none();
        };
        match out {
            form::Event::Changed(_) => {
                b.replan();
                Reply::redraw()
            }
            form::Event::Cancel | form::Event::Pressed(renamebox::CANCEL) => {
                self.dialog = None;
                self.show_waiting(ctx)
            }
            form::Event::Default | form::Event::Pressed(renamebox::OK) => {
                let rows = b.plan();
                if !batchrename::ready(&rows) {
                    return Reply::none();
                }
                let Some(Dialog::BatchRename(b)) = self.dialog.take() else {
                    return Reply::none();
                };
                let tag = format!("reprise-rename-{}", std::process::id());
                let steps = batchrename::steps(&b.paths, &rows, &tag);
                let dirs = b.items.iter().any(|(_, d)| *d);
                let landed: Vec<PathBuf> = b
                    .paths
                    .iter()
                    .zip(&rows)
                    .map(|(p, r)| p.with_file_name(&r.new))
                    .collect();
                let at = self.place.path();
                let work = move |gate: &Gate| {
                    if let Some(at) = &at
                        && !gate.change(at)
                    {
                        return Ok(Vec::new());
                    }
                    batchrename::apply(&steps).map(|()| steps)
                };
                let reply = self.act(
                    ctx,
                    Action::changes("Rename"),
                    work,
                    move |ex, done, ctx| ex.batch_renamed(done, landed, dirs, ctx),
                );
                Reply {
                    redraw_all: true,
                    ..reply
                }
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn batch_renamed(
        &mut self,
        done: Result<Vec<(PathBuf, PathBuf)>, batchrename::Failed>,
        landed: Vec<PathBuf>,
        dirs: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let (undone, failed) = match done {
            Ok(steps) => (steps, None),
            Err(mut f) => (std::mem::take(&mut f.left), Some(f)),
        };
        self.undos.push_all(
            undone
                .into_iter()
                .map(|(from, to)| reprisefs::undo::Undoable::Rename { from, to })
                .collect(),
        );
        self.sync_undo();
        if dirs && let Some(at) = self.place.path() {
            let place = crate::place::Place::from_path(&at);
            self.refill_tree_node(&place, ctx);
        }
        let reply = self.refresh(ctx);
        if let Some(f) = failed {
            log::warn!("explorer: rename {} failed: {}", f.from.display(), f.error);
            let name = f.from.file_name().unwrap_or_default().to_string_lossy();
            return self.message_box("Rename", failed_text(&name, &f.error), ctx);
        }
        if self.reading.is_none() {
            self.select_paths(&landed);
            self.refresh_chrome(ctx);
        }
        reply
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Command;
    use crate::place::Place;
    use crate::tests::{test_ctx, test_explorer};
    use reprisekit::controls::form::Control;

    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut n: Vec<String> = std::fs::read_dir(dir)
            .expect("the folder")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        n.sort();
        n
    }

    fn form_of(ex: &mut Explorer) -> &mut Form {
        match ex.dialog.as_mut() {
            Some(Dialog::BatchRename(b)) => &mut b.form,
            _ => panic!("the Rename dialog is up"),
        }
    }

    fn type_into(ex: &mut Explorer, id: &'static str, text: &str, ctx: &mut Ctx) -> Reply {
        if let Some(Control::Edit { edit, .. }) = form_of(ex).item_mut(id).map(|i| &mut i.control) {
            *edit = TextEdit::new(text, false);
        }
        ex.dialog_outcome(form::Event::Changed(id), ctx)
    }

    fn preview(ex: &mut Explorer) -> Vec<String> {
        match &form_of(ex)
            .item(renamebox::PREVIEW)
            .expect("the list")
            .control
        {
            Control::ListBox { list, .. } => list.items.clone(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn f2_on_one_item_renames_inline_and_on_several_opens_the_dialog() {
        let dir = tempfile::tempdir().expect("a temp dir");
        for n in ["a.txt", "b.txt"] {
            std::fs::write(dir.path().join(n), "x").expect("write");
        }
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set_only(Some(0));
        ex.run(Command::Rename, &mut ctx);
        assert!(ex.sv.edit.is_some() && ex.dialog.is_none());
        ex.sv.edit = None;
        ex.run(Command::SelectAll, &mut ctx);
        ex.run(Command::Rename, &mut ctx);
        assert!(ex.sv.edit.is_none());
        assert!(matches!(ex.dialog, Some(Dialog::BatchRename(_))));
        assert_eq!(ex.children()[0].title, "Rename");
    }

    #[test]
    fn numbering_renames_in_view_order_and_undo_restores_every_name() {
        let dir = tempfile::tempdir().expect("a temp dir");
        for n in ["c.jpg", "a.png", "b.jpg", "keep.txt"] {
            std::fs::write(dir.path().join(n), n).expect("write");
        }
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        let shown: Vec<String> = ex.items.iter().map(|it| it.name.clone()).collect();
        assert_eq!(shown, ["a.png", "b.jpg", "c.jpg", "keep.txt"]);
        for i in 0..3 {
            ex.sv.selection.set.insert(i);
        }
        ex.run(Command::Rename, &mut ctx);
        let f = form_of(&mut ex);
        f.set_check(renamebox::REPLACE, false);
        f.set_check(renamebox::NUMBER, true);
        ex.dialog_outcome(form::Event::Changed(renamebox::NUMBER), &mut ctx);
        type_into(&mut ex, renamebox::BASE, "Photo", &mut ctx);
        assert_eq!(
            preview(&mut ex),
            [
                "a.png  \u{2192}  Photo (1).png",
                "b.jpg  \u{2192}  Photo (2).jpg",
                "c.jpg  \u{2192}  Photo (3).jpg",
            ]
        );
        ex.dialog_outcome(form::Event::Default, &mut ctx);
        assert!(ex.dialog.is_none());
        assert_eq!(
            names(dir.path()),
            [
                "Photo (1).png",
                "Photo (2).jpg",
                "Photo (3).jpg",
                "keep.txt"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Photo (3).jpg")).expect("read"),
            "c.jpg"
        );
        assert_eq!(ex.sv.selection.len(), 3, "the renamed items stay picked");
        assert_eq!(ex.undo, Some("Rename"));

        ex.run(Command::Undo, &mut ctx);
        assert_eq!(names(dir.path()), ["a.png", "b.jpg", "c.jpg", "keep.txt"]);
        assert_eq!(ex.undo, None, "one undo restores the whole batch");
    }

    #[test]
    fn a_clash_with_another_item_blocks_ok() {
        let dir = tempfile::tempdir().expect("a temp dir");
        for n in ["x1.txt", "x2.txt", "y1.txt"] {
            std::fs::write(dir.path().join(n), "x").expect("write");
        }
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set.extend([0, 1]);
        ex.run(Command::Rename, &mut ctx);
        type_into(&mut ex, renamebox::FIND, "x", &mut ctx);
        type_into(&mut ex, renamebox::WITH, "y", &mut ctx);
        let enabled = |ex: &mut Explorer| match &form_of(ex).item(renamebox::OK).unwrap().control {
            Control::Button { enabled, .. } => *enabled,
            _ => unreachable!(),
        };
        assert!(!enabled(&mut ex), "y1.txt is kept by the item not picked");
        ex.dialog_outcome(form::Event::Default, &mut ctx);
        assert!(
            ex.dialog.is_some(),
            "Enter does nothing while a name clashes"
        );
        assert_eq!(names(dir.path()), ["x1.txt", "x2.txt", "y1.txt"]);
        type_into(&mut ex, renamebox::WITH, "z", &mut ctx);
        assert!(enabled(&mut ex));
        ex.dialog_outcome(form::Event::Pressed(renamebox::OK), &mut ctx);
        assert_eq!(names(dir.path()), ["y1.txt", "z1.txt", "z2.txt"]);
    }
}
