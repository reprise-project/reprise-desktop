//! A drag over a program's form ([`crate::App::drop_form`]): its single-line edit fields take
//! text, and files as their paths on one line. The host shows the drop caret, scrolls a field
//! while the drag rests at its edge, puts a drop in at the drop caret, and tells the program
//! with `Event::FieldDropped`. A file dialog's File name field ([`crate::App::drop_file_dialog`])
//! takes text the same way, and files as their names.

use std::path::Path;

use crate::{Accept, Actions, DragAction, DropKind, Dropped, Event, Reply};
use reprisekit::controls::filedlg::FileDlg;
use reprisekit::controls::form::{self, Form};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;

/// A form whose edit fields take drops, and the font size it is drawn at when that is not the
/// theme's.
pub struct DropForm<'a> {
    pub form: &'a mut Form,
    pub px: Option<u32>,
}

/// A file dialog whose File name field takes drops, and the host that knows its open folder.
pub struct FileDrop<'a> {
    pub dlg: &'a mut FileDlg,
    pub host: &'a crate::filedlg::Host,
}

/// What takes a drag: a form's fields or a file dialog's File name.
pub(crate) trait Target {
    /// Shows the drop caret at (`x`, `y`) when a field there takes the drag.
    fn drag_over(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool;
    fn drag_leave(&mut self) -> bool;
    /// The drag rests in a field's edge band with text hidden past it.
    fn scrolls(&self, text: &TextRenderer, px: u32) -> bool;
    fn drag_tick(&mut self, text: &TextRenderer, px: u32) -> bool;
    /// Puts `data` in the field under the drag: `Changed` with its id, else `None`.
    fn drop(&mut self, data: &Dropped, text: &TextRenderer, px: u32) -> form::Event;
}

impl Target for Form {
    fn drag_over(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        Form::drag_over(self, x, y, text, px)
    }
    fn drag_leave(&mut self) -> bool {
        Form::drag_leave(self)
    }
    fn scrolls(&self, text: &TextRenderer, px: u32) -> bool {
        self.drag_scroll(text, px).is_some()
    }
    fn drag_tick(&mut self, text: &TextRenderer, px: u32) -> bool {
        Form::drag_tick(self, text, px)
    }
    fn drop(&mut self, data: &Dropped, text: &TextRenderer, px: u32) -> form::Event {
        self.drop_text(&line(data), text, px)
    }
}

/// The id [`FileDrop`] reports a changed File name with.
pub const FILE_NAME: &str = "filename";

impl FileDrop<'_> {
    fn client(&self) -> Rect {
        let (w, h) = self.dlg.client();
        Rect::new(0, 0, w, h)
    }
}

impl Target for FileDrop<'_> {
    fn drag_over(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        let r = self.client();
        self.dlg.drag_over(text, px, r, x, y)
    }
    fn drag_leave(&mut self) -> bool {
        self.dlg.drag_leave()
    }
    fn scrolls(&self, text: &TextRenderer, px: u32) -> bool {
        self.dlg.drag_scroll(text, px, self.client()).is_some()
    }
    fn drag_tick(&mut self, text: &TextRenderer, px: u32) -> bool {
        let r = self.client();
        self.dlg.drag_tick(text, px, r)
    }
    fn drop(&mut self, data: &Dropped, text: &TextRenderer, px: u32) -> form::Event {
        let r = self.client();
        let took = match data {
            Dropped::Text(t) => self.dlg.drop_text(text, px, r, t),
            Dropped::Files(paths) => {
                let dir = self.host.folder.and_then(|id| self.host.folders.path(id));
                self.dlg.drop_names(&file_names(paths, dir))
            }
            Dropped::Uris(_) => false,
        };
        match took {
            true => form::Event::Changed(FILE_NAME),
            false => form::Event::None,
        }
    }
}

/// Dropped files as File name takes them: a file in the open folder `dir` by its name, any
/// other by its full path.
pub fn file_names(paths: &[std::path::PathBuf], dir: Option<&Path>) -> Vec<String> {
    paths
        .iter()
        .map(|p| match p.file_name() {
            Some(name) if dir.is_some() && p.parent() == dir => name.to_string_lossy().into_owned(),
            _ => p.to_string_lossy().into_owned(),
        })
        .collect()
}

/// A drag over a window with a drop form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hover {
    kind: Option<DropKind>,
    /// It also offers text, read when its URI list names no files.
    text: bool,
    source: Actions,
    at: (i32, i32),
}

/// What a field reads a drag as: a file manager's files as paths, anything else with text as
/// its text (a browser's link included).
pub fn kind_for(o: crate::Offered) -> Option<DropKind> {
    if o.files && !o.link {
        Some(DropKind::Files)
    } else if o.text {
        Some(DropKind::Text)
    } else {
        None
    }
}

/// What the field answers: a copy, or a move of text the source lets go; `Ask` is answered
/// as a copy.
fn accept(h: &Hover) -> Option<Accept> {
    Some(accept_as(h.kind?, h.source))
}

/// A field's answer to a drag read as `kind` from a source allowing `source`.
pub fn accept_as(kind: DropKind, source: Actions) -> Accept {
    Accept {
        kind,
        actions: Actions {
            copy: true,
            move_: kind == DropKind::Text && source.move_,
            ask: true,
        },
        preferred: Some(DragAction::Copy),
    }
}

/// The line a field takes for `data`: text as it came, files as their paths
/// ([`reprisekit::textedit::paths_line`]).
pub fn line(data: &Dropped) -> String {
    match data {
        Dropped::Text(t) => t.clone(),
        Dropped::Files(paths) => reprisekit::textedit::paths_line(paths),
        Dropped::Uris(_) => String::new(),
    }
}

/// The answer to a drag at `h.at`: taken where a field lies under it, a tick asked for while
/// it rests in a field's edge band.
fn over(form: &mut dyn Target, h: &Hover, text: &TextRenderer, px: u32) -> Reply {
    let takes = h.kind.is_some() && form.drag_over(h.at.0, h.at.1, text, px);
    Reply {
        redraw: true,
        tick: takes && form.scrolls(text, px),
        ..Reply::drag_accept(accept(h).filter(|_| takes))
    }
}

/// One event of a drag over `form`, or a tick while one rests on it. The reply, and what a drop
/// changed for the program to hear as `Event::FieldDropped`; None for an event this does not
/// handle, which goes to the program.
pub(crate) fn event(
    form: &mut dyn Target,
    hover: &mut Option<Hover>,
    ev: &Event,
    text: &TextRenderer,
    px: u32,
) -> Option<(Reply, form::Event)> {
    let reply = match ev {
        Event::DragEnter {
            x,
            y,
            offered,
            source,
            ..
        } => {
            let h = Hover {
                kind: kind_for(*offered),
                text: offered.text,
                source: *source,
                at: (*x, *y),
            };
            *hover = Some(h);
            over(form, &h, text, px)
        }
        Event::DragMotion { x, y, .. } => {
            let h = hover.as_mut()?;
            h.at = (*x, *y);
            let h = *h;
            over(form, &h, text, px)
        }
        // Web addresses only: their text, or nothing.
        Event::DragFiles { paths } if paths.is_empty() => {
            let h = hover.as_mut()?;
            h.kind = h.text.then_some(DropKind::Text);
            let h = *h;
            over(form, &h, text, px)
        }
        Event::DragFiles { .. } => Reply::none(),
        Event::Tick if hover.is_some() => {
            if !form.drag_tick(text, px) {
                return None;
            }
            let h = (*hover)?;
            Reply {
                tick: form.scrolls(text, px),
                ..over(form, &h, text, px)
            }
        }
        Event::DragLeave => {
            *hover = None;
            Reply {
                redraw: form.drag_leave(),
                ..Reply::none()
            }
        }
        Event::DragDrop { data, action, .. } => {
            *hover = None;
            let changed = form.drop(data, text, px);
            form.drag_leave();
            let mut reply = Reply::redraw();
            if *action == DragAction::Ask {
                let took = changed != form::Event::None;
                reply.answer_drop = Some(took.then_some(DragAction::Copy));
            }
            return Some((reply, changed));
        }
        _ => return None,
    };
    Some((reply, form::Event::None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Offered;
    use reprisekit::controls::form::{Control, Item};
    use reprisekit::raster::Rect;
    use reprisekit::textedit::TextEdit;
    use std::cell::Cell;
    use std::path::PathBuf;

    const PX: u32 = 11;
    const TEXT: Offered = Offered {
        files: false,
        link: false,
        text: true,
    };
    const FILES: Offered = Offered {
        files: true,
        link: false,
        text: true,
    };

    fn tr() -> TextRenderer {
        TextRenderer::load(&reprisekit::theme::FontConfig::default())
    }

    fn field_form(value: &str) -> Form {
        let mut f = Form::new(vec![
            Item::label("lbl", Rect::new(10, 10, 60, 13), "&Open:"),
            Item::new(
                "cmd",
                Rect::new(80, 10, 100, 20),
                Control::Edit {
                    edit: TextEdit::new(value, false),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
        ]);
        f.set_focus(None);
        f
    }

    fn enter(x: i32, offered: Offered) -> Event {
        Event::DragEnter {
            x,
            y: 15,
            offered,
            source: Actions::ALL,
            action: None,
            own: false,
        }
    }

    fn drop(data: Dropped, action: DragAction) -> Event {
        Event::DragDrop {
            x: 0,
            y: 0,
            data,
            action,
            source: Actions::ALL,
        }
    }

    fn field(f: &Form) -> &TextEdit {
        match &f.items[1].control {
            Control::Edit { edit, .. } => edit,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn text_over_a_field_is_taken_and_lands_at_the_drop_caret_selected() {
        let (t, mut f, mut h) = (tr(), field_form("ab"), None);
        let x = 83 + t.measure("a", false, PX);
        let (r, _) = event(&mut f, &mut h, &enter(x, TEXT), &t, PX).unwrap();
        let a = r.drag_accept.flatten().expect("the field takes text");
        assert_eq!(
            (a.kind, a.preferred),
            (DropKind::Text, Some(DragAction::Copy))
        );
        assert!(a.actions.move_, "Shift moves text");
        assert!(r.redraw, "the drop caret shows");
        let (_, changed) = event(
            &mut f,
            &mut h,
            &drop(Dropped::Text("X".into()), DragAction::Copy),
            &t,
            PX,
        )
        .unwrap();
        assert_eq!(changed, form::Event::Changed("cmd"));
        assert_eq!(field(&f).text, "aXb");
        assert_eq!(field(&f).selected_text(), "X");
        assert_eq!(f.focus(), Some(1));
        assert!(h.is_none());
    }

    #[test]
    fn a_file_managers_drag_lands_as_its_paths_on_one_line() {
        let (t, mut f, mut h) = (tr(), field_form(""), None);
        let (r, _) = event(&mut f, &mut h, &enter(120, FILES), &t, PX).unwrap();
        let a = r.drag_accept.flatten().unwrap();
        assert_eq!(a.kind, DropKind::Files);
        assert!(!a.actions.move_, "a path never moves its file");
        let paths = vec![
            PathBuf::from("/usr/bin/foot"),
            PathBuf::from("/tmp/my notes.txt"),
        ];
        let (r, _) = event(
            &mut f,
            &mut h,
            &drop(Dropped::Files(paths), DragAction::Ask),
            &t,
            PX,
        )
        .unwrap();
        assert_eq!(field(&f).text, "/usr/bin/foot '/tmp/my notes.txt'");
        assert_eq!(
            r.answer_drop,
            Some(Some(DragAction::Copy)),
            "an asked drop is a copy"
        );
    }

    #[test]
    fn web_addresses_fall_back_to_their_text_or_nothing() {
        let (t, mut f, mut h) = (tr(), field_form(""), None);
        event(&mut f, &mut h, &enter(120, FILES), &t, PX);
        let files = Event::DragFiles { paths: vec![] };
        let (r, _) = event(&mut f, &mut h, &files, &t, PX).unwrap();
        assert_eq!(
            r.drag_accept.flatten().map(|a| a.kind),
            Some(DropKind::Text)
        );
        let uris_only = Offered {
            text: false,
            ..FILES
        };
        event(&mut f, &mut h, &enter(120, uris_only), &t, PX);
        let (r, _) = event(&mut f, &mut h, &files, &t, PX).unwrap();
        assert_eq!(r.drag_accept, Some(None));
    }

    #[test]
    fn a_drag_off_the_fields_is_refused_and_a_drop_there_does_nothing() {
        let (t, mut f, mut h) = (tr(), field_form("ab"), None);
        let (r, _) = event(&mut f, &mut h, &enter(30, TEXT), &t, PX).unwrap();
        assert_eq!(r.drag_accept, Some(None));
        let (r, changed) = event(
            &mut f,
            &mut h,
            &drop(Dropped::Text("X".into()), DragAction::Ask),
            &t,
            PX,
        )
        .unwrap();
        assert_eq!(changed, form::Event::None);
        assert_eq!(
            r.answer_drop,
            Some(None),
            "an asked drop with nowhere to land is cancelled"
        );
        assert_eq!(field(&f).text, "ab");
    }

    #[test]
    fn a_drag_resting_at_a_fields_edge_ticks_until_it_leaves() {
        let (t, mut f, mut h) = (
            tr(),
            field_form("abcdefghijklmnopqrstuvwxyz0123456789"),
            None,
        );
        let (r, _) = event(&mut f, &mut h, &enter(178, TEXT), &t, PX).unwrap();
        assert!(r.tick, "the right band asks for a tick");
        let (r, _) = event(&mut f, &mut h, &Event::Tick, &t, PX).unwrap();
        assert!(r.redraw && r.tick);
        let mid = Event::DragMotion {
            x: 130,
            y: 15,
            action: None,
        };
        let (r, _) = event(&mut f, &mut h, &mid, &t, PX).unwrap();
        assert!(!r.tick, "the middle rests still");
        assert!(
            event(&mut f, &mut h, &Event::Tick, &t, PX).is_none(),
            "a tick with nothing to scroll goes to the program"
        );
        event(&mut f, &mut h, &Event::DragLeave, &t, PX);
        assert!(event(&mut f, &mut h, &Event::Tick, &t, PX).is_none());
    }

    #[test]
    fn a_dropped_file_is_named_in_the_open_folder_and_given_by_path_from_another() {
        let dir = Path::new("/home/me/Documents");
        let paths = vec![
            PathBuf::from("/home/me/Documents/my notes.txt"),
            PathBuf::from("/etc/hosts"),
        ];
        assert_eq!(
            file_names(&paths, Some(dir)),
            ["my notes.txt", "/etc/hosts"]
        );
        assert_eq!(
            file_names(&paths, None),
            ["/home/me/Documents/my notes.txt", "/etc/hosts"]
        );
    }

    #[test]
    fn a_file_dropped_on_a_file_dialogs_name_field_replaces_it() {
        use reprisekit::controls::filedlg::{CLIENT, DlgMode, FIELD_X, Fields, Hit};
        let t = tr();
        let px = reprisekit::theme::Theme::default().fonts.size_px;
        let tmp = std::env::temp_dir();
        let mut host = crate::filedlg::Host::default();
        let mut dlg = host.open(
            &tmp,
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                filters: vec![],
                mode: DlgMode::Open,
                multiple: false,
            },
        );
        dlg.set_file_name("old.txt");
        let r = Rect::new(0, 0, CLIENT.0, CLIENT.1);
        let x = FIELD_X + 20;
        let y = (0..CLIENT.1)
            .find(|&y| dlg.hit(&t, px, r, x, y) == Some(Hit::FileName))
            .unwrap();
        let mut file = FileDrop {
            dlg: &mut dlg,
            host: &host,
        };
        let mut h = None;
        let enter = Event::DragEnter {
            x,
            y,
            offered: FILES,
            source: Actions::ALL,
            action: None,
            own: false,
        };
        let (r, _) = event(&mut file, &mut h, &enter, &t, px).unwrap();
        assert_eq!(
            r.drag_accept.flatten().map(|a| a.kind),
            Some(DropKind::Files)
        );
        let data = Dropped::Files(vec![tmp.join("new.txt")]);
        let (_, changed) = event(&mut file, &mut h, &drop(data, DragAction::Copy), &t, px).unwrap();
        assert_eq!(changed, form::Event::Changed(FILE_NAME));
        assert_eq!(dlg.file_name(), "new.txt");
    }
}
