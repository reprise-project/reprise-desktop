//! Another program's drag over one of Reprise's text fields. A dialog's field (Run's command
//! line, a form's edit field) shows a drop caret, scrolls while the drag rests at its edge, and
//! takes the drop as text, or files as their paths on one line. The launcher's and the Start
//! menu's search fields take it whole as the query; a desktop icon's rename box puts it in at
//! the drop caret. Files go into these three by name. The data comes over a pipe as
//! `dnddrop`'s does; the drop is always a copy.

use std::os::fd::OwnedFd;

use reprisefs::dropdata::{self, Dropped};
use smithay::{
    input::dnd::{DndAction, Source},
    reexports::wayland_server::protocol::wl_data_device_manager::DndAction as WlDndAction,
    utils::{IsAlive, Logical, Point},
};

use crate::{compositor::State, dnd::DragSource as SeatSource, input::Hit};

/// The text mimes a field reads, best first.
const TEXT_MIMES: [&str; 3] = [dropdata::TEXT_UTF8, "text/plain", "UTF8_STRING"];

/// A drag over a field, and the mime the drop is read as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldOver {
    field: Field,
    mime: &'static str,
}

/// The field a drag rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    /// A dialog's field, by `DialogView::id`.
    Dialog(u64),
    Launcher,
    StartPanel,
    /// The rename box of desktop icon `index`.
    Rename(usize),
}

/// The mime a field reads a drag offering `offered` as: a file manager's URI list (its paths),
/// else UTF-8 text (a browser's link drops its text).
pub fn field_mime<S: AsRef<str>>(offered: &[S]) -> Option<&'static str> {
    let has = |mime: &str| offered.iter().any(|m| m.as_ref() == mime);
    let link = dropdata::pick_mime(offered).is_some_and(dropdata::is_link_mime);
    if has(dropdata::URI_LIST) && !link {
        return Some(dropdata::URI_LIST);
    }
    TEXT_MIMES.into_iter().find(|m| has(m))
}

/// The line a field takes for a drop read as `mime`: a URI list's files and links, a path
/// holding a space quoted ([`reprisekit::textedit::paths_line`]); text as it came.
pub fn field_line(mime: &str, bytes: &[u8]) -> String {
    if mime != dropdata::URI_LIST {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let items: Vec<String> = dropdata::decode(mime, bytes)
        .into_iter()
        .map(|d| match d {
            Dropped::File(p) => p.to_string_lossy().into_owned(),
            Dropped::Url { url, .. } => url,
        })
        .collect();
    reprisekit::textedit::paths_line(&items)
}

/// The line a name field (a search query, a rename box) takes for a drop read as `mime`: a URI
/// list's files by their names and links whole, space-separated; text as it came.
pub fn name_line(mime: &str, bytes: &[u8]) -> String {
    if mime != dropdata::URI_LIST {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let names: Vec<String> = dropdata::decode(mime, bytes)
        .into_iter()
        .map(|d| match d {
            Dropped::File(p) => p
                .file_name()
                .unwrap_or(p.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Dropped::Url { url, .. } => url,
        })
        .collect();
    names.join(" ")
}

impl State {
    /// The top dialog's field under `pos` takes another program's drag: it shows the drop caret
    /// and the source hears a copy. Returns the copy cursor, or None where no field takes it.
    pub fn field_over(
        &mut self,
        source: &SeatSource,
        pos: Point<f64, Logical>,
    ) -> Option<&'static [&'static str]> {
        let taken = self.field_under(source, pos);
        let Some(over) = taken else {
            self.field_left();
            return None;
        };
        self.target_left();
        let before = self.dnd.drops.field.replace(over);
        if let Some(b) = before
            && b.field != over.field
        {
            self.field_let_go(b);
        }
        if before.map(|b| b.mime) != Some(over.mime) {
            source.accepted(Some(over.mime.to_string()));
            source.choose_action(DndAction::Copy);
        }
        Some(crate::dnd::cursor_for(true, DndAction::Copy, false))
    }

    fn field_under(&mut self, source: &SeatSource, pos: Point<f64, Logical>) -> Option<FieldOver> {
        if self.is_locked() {
            return None;
        }
        let meta = source.metadata()?;
        let mime = field_mime(&meta.mime_types)?;
        let allowed = crate::dnddrop::source_actions(&meta.dnd_actions);
        if !allowed.contains(WlDndAction::Copy) {
            return None;
        }
        let field = match self.scene_hit(pos) {
            Hit::Dialog { index, .. } => self.dialog_field_under(index, pos)?,
            Hit::Launcher {
                hit: reprisekit::controls::launcher::Hit::Field,
            } => Field::Launcher,
            Hit::StartPanel {
                spot: Some(reprisekit::startpanel::Spot::Search),
            } => Field::StartPanel,
            Hit::Desktop { .. } => self.rename_field_under(pos)?,
            _ => return None,
        };
        Some(FieldOver { field, mime })
    }

    fn dialog_field_under(&mut self, index: usize, pos: Point<f64, Logical>) -> Option<Field> {
        if index + 1 != self.dialogs.views.len() {
            return None;
        }
        let px = self.config.dialog_theme.fonts.size_px;
        let v = &mut self.dialogs.views[index];
        let at = (pos.x as i32 - v.x, pos.y as i32 - v.y);
        let takes = v.dialog.drag_over(&v.layout, at.0, at.1, &self.text, px);
        let field = Field::Dialog(v.id);
        if takes || self.dnd.drops.field.is_some_and(|f| f.field == field) {
            self.redraw_dialog(index);
        }
        takes.then_some(field)
    }

    /// The rename box under `pos`: it shows a drop caret at the character there.
    fn rename_field_under(&mut self, pos: Point<f64, Logical>) -> Option<Field> {
        let view = self.desktop.as_ref()?;
        let (index, edit) = view.rename.as_ref()?;
        let cell = view.cells.get(*index)?;
        let theme = &self.config.theme;
        let lay = crate::desktop::edit_layout(theme, &self.text, edit);
        let (x, y) = (pos.x as i32 - cell.x, pos.y as i32 - cell.y);
        if !lay.rect.contains(x, y) {
            return None;
        }
        let at = lay.index_at(&self.text, theme.fonts.size_px, x, y);
        let index = *index;
        let view = self.desktop.as_mut()?;
        if view.rename_drop.replace(at) != Some(at) {
            self.relayout_desktop();
        }
        Some(Field::Rename(index))
    }

    /// The drag left the field: its drop caret goes, and the source hears nothing takes it here.
    pub fn field_left(&mut self) {
        let Some(was) = self.dnd.drops.field.take() else {
            return;
        };
        self.field_let_go(was);
        if let Some(source) = self.drag_source() {
            source.forget_target();
            source.choose_action(DndAction::None);
        }
    }

    /// The drag timer: a drag resting at a dialog field's edge scrolls it a step.
    pub fn field_tick(&mut self) {
        let Some(FieldOver {
            field: Field::Dialog(id),
            ..
        }) = self.dnd.drops.field
        else {
            return;
        };
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        let px = self.config.dialog_theme.fonts.size_px;
        let v = &mut self.dialogs.views[i];
        if v.dialog.drag_tick(&v.layout, &self.text, px) {
            self.redraw_dialog(i);
        }
    }

    /// The drag ended over a field (`true`): when `dropped`, its data is asked for from an idle,
    /// and the field keeps its drop caret and view until the data is in. Runs under the
    /// pointer's lock.
    pub fn field_drag_ended(&mut self, source: &SeatSource, dropped: bool) -> bool {
        let Some(over) = self.dnd.drops.field.take() else {
            return false;
        };
        if dropped {
            let source = source.clone();
            self.handle
                .insert_idle(move |state| state.field_released(source, over));
        } else {
            self.field_let_go(over);
        }
        true
    }

    /// The field `over` names stops showing a drop caret.
    fn field_let_go(&mut self, over: FieldOver) {
        match over.field {
            Field::Dialog(id) => {
                if let Some(i) = self.dialog_index(id)
                    && self.dialogs.views[i].dialog.drag_leave()
                {
                    self.redraw_dialog(i);
                }
            }
            Field::Rename(_) => {
                if self
                    .desktop
                    .as_mut()
                    .is_some_and(|d| d.rename_drop.take().is_some())
                {
                    self.relayout_desktop();
                }
            }
            Field::Launcher | Field::StartPanel => {}
        }
    }

    /// The field is still there to take a drop.
    fn field_alive(&self, field: Field) -> bool {
        match field {
            Field::Dialog(id) => self.dialog_index(id).is_some(),
            Field::Launcher => self.launcher.is_some(),
            Field::StartPanel => self.start_panel.is_some(),
            Field::Rename(i) => self
                .desktop
                .as_ref()
                .and_then(|d| d.rename.as_ref())
                .is_some_and(|(r, _)| *r == i),
        }
    }

    fn field_released(&mut self, source: SeatSource, over: FieldOver) {
        if !source.alive() {
            self.field_let_go(over);
            return;
        }
        if self.is_locked() || !self.field_alive(over.field) {
            source.refuse();
            self.field_let_go(over);
            return;
        }
        let (read, write) = match std::io::pipe() {
            Ok((r, w)) => (OwnedFd::from(r), OwnedFd::from(w)),
            Err(e) => {
                log::warn!("field drop: pipe: {e}");
                source.refuse();
                self.field_let_go(over);
                return;
            }
        };
        source.drop_performed();
        source.send(over.mime, write);
        self.spawn_fs_job(move || {
            let got = reprisefs::pipe::read_bounded(
                &read,
                crate::dnddrop::MAX_BYTES,
                crate::dnddrop::READ_TIMEOUT,
            );
            Box::new(move |state| state.field_received(source, over, got))
        });
    }

    fn field_received(
        &mut self,
        source: SeatSource,
        over: FieldOver,
        got: std::io::Result<Vec<u8>>,
    ) {
        let bytes = match got {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("field drop: reading the dragged data as {}: {e}", over.mime);
                Vec::new()
            }
        };
        let took = match over.field {
            Field::Dialog(id) => self.dialog_field_dropped(id, &field_line(over.mime, &bytes)),
            Field::Launcher => self.launcher_dropped(&name_line(over.mime, &bytes)),
            Field::StartPanel => self.start_panel_dropped(&name_line(over.mime, &bytes)),
            Field::Rename(_) => self.rename_dropped(&name_line(over.mime, &bytes)),
        };
        self.field_let_go(over);
        if source.alive() {
            if took {
                source.choose_action(DndAction::Copy);
                source.finished();
            } else {
                source.refuse();
            }
        }
    }

    fn dialog_field_dropped(&mut self, id: u64, line: &str) -> bool {
        let px = self.config.dialog_theme.fonts.size_px;
        let Some(i) = self.dialog_index(id) else {
            return false;
        };
        let v = &mut self.dialogs.views[i];
        let outcome = v.dialog.drop_text(&v.layout, line, &self.text, px);
        v.dialog.drag_leave();
        let took = outcome != crate::dialog::Outcome::Nothing;
        if took && i + 1 == self.dialogs.views.len() {
            self.apply_dialog(outcome);
        } else {
            self.redraw_dialog(i);
        }
        took
    }

    /// Text dropped on the rename box goes in at the drop caret, selected, without a `/`.
    fn rename_dropped(&mut self, line: &str) -> bool {
        let line = line.replace('/', "");
        let took = self.desktop.as_mut().is_some_and(|d| {
            let at = d.rename_drop.take();
            let edit = d.rename.as_mut().map(|(_, e)| e);
            at.zip(edit).is_some_and(|(at, e)| e.drop_text(at, &line))
        });
        if took {
            self.restart_caret();
        }
        self.relayout_desktop();
        took
    }

    fn dialog_index(&self, id: u64) -> Option<usize> {
        self.dialogs.views.iter().position(|v| v.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_managers_drag_reads_as_paths_and_anything_else_with_text_as_text() {
        let files = [dropdata::URI_LIST, dropdata::TEXT_UTF8];
        assert_eq!(field_mime(&files), Some(dropdata::URI_LIST));
        let link = [
            dropdata::MOZ_URL,
            dropdata::URI_LIST,
            dropdata::HTML,
            "text/plain",
        ];
        assert_eq!(
            field_mime(&link),
            Some("text/plain"),
            "a link drops its text"
        );
        assert_eq!(field_mime(&["UTF8_STRING", "TEXT"]), Some("UTF8_STRING"));
        assert_eq!(field_mime(&["image/png"]), None);
    }

    #[test]
    fn a_uri_list_drops_its_paths_on_one_line() {
        let list = b"file:///usr/bin/foot\r\nfile:///tmp/my%20notes.txt\r\n";
        assert_eq!(
            field_line(dropdata::URI_LIST, list),
            "/usr/bin/foot '/tmp/my notes.txt'"
        );
        assert_eq!(field_line(dropdata::TEXT_UTF8, "ls -l".as_bytes()), "ls -l");
    }

    #[test]
    fn a_name_field_takes_dropped_files_by_their_names() {
        let list = b"file:///usr/bin/foot\r\nfile:///tmp/my%20notes.txt\r\n";
        assert_eq!(name_line(dropdata::URI_LIST, list), "foot my notes.txt");
        assert_eq!(name_line(dropdata::TEXT_UTF8, b"a/b"), "a/b");
    }
}
