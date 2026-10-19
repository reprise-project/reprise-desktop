use crate::acting::Action;
use crate::{Explorer, Focus};
use repriseapp::{Ctx, KeyInput, Reply};
use reprisefs::deadline::Gate;
use reprisefs::listing::Item;
use reprisefs::names::{Verdict, failed_text, verdict};
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

    /// The text field with the keyboard: the rename box, else the address field while it has
    /// the focus. Its clipboard keys and pastes go there.
    pub(crate) fn text_field(&self) -> Option<&TextEdit> {
        if let Some((_, e)) = &self.sv.edit {
            return Some(e);
        }
        (self.focus == Focus::Address)
            .then_some(self.address.combo.edit.as_ref())
            .flatten()
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
