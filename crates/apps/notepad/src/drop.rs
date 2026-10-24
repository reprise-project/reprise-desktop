//! Text lands where dropped; of a file manager's drag the files win; a web link drops its text. Nothing is taken while a modal box is up.

use crate::Notepad;
use crate::pending::PendingAction;
use repriseapp::{Accept, Actions, App, Ctx, DragAction, DropKind, Dropped, Event, Offered, Reply};
use reprisekit::controls::multiedit::{self, Pos, View};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hover {
    pub kind: DropKind,
    text: bool,
    xy: (i32, i32),
    source: Actions,
}

impl Hover {
    /// `Ask` is answered as a copy: Notepad has no drop menu.
    fn accept(&self) -> Accept {
        Accept {
            kind: self.kind,
            actions: Actions {
                copy: true,
                move_: self.kind == DropKind::Text && self.source.move_,
                ask: true,
            },
            preferred: Some(DragAction::Copy),
        }
    }
}

fn kind_for(o: Offered) -> Option<DropKind> {
    if o.files && !o.link {
        Some(DropKind::Files)
    } else if o.text {
        Some(DropKind::Text)
    } else {
        None
    }
}

impl Notepad {
    pub(crate) fn drop_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        match ev {
            Event::DragEnter {
                x,
                y,
                offered,
                source,
                ..
            } => {
                let modal = self.children().iter().any(|c| c.modal);
                self.hover = kind_for(offered).filter(|_| !modal).map(|kind| Hover {
                    kind,
                    text: offered.text,
                    xy: (x, y),
                    source,
                });
                Reply {
                    tick: self.scroll_step(ctx).is_some(),
                    ..self.answer()
                }
            }
            Event::DragMotion { x, y, .. } => match &mut self.hover {
                Some(h) if h.xy != (x, y) => {
                    h.xy = (x, y);
                    if h.kind == DropKind::Text {
                        Reply {
                            tick: self.scroll_step(ctx).is_some(),
                            ..Reply::redraw()
                        }
                    } else {
                        Reply::none()
                    }
                }
                _ => Reply::none(),
            },
            Event::Tick => match self.scroll_step(ctx) {
                Some(view) => {
                    self.view = view;
                    Reply {
                        tick: true,
                        ..Reply::redraw()
                    }
                }
                None => Reply::none(),
            },
            // Web addresses only: their text, or nothing.
            Event::DragFiles { paths } if paths.is_empty() => {
                if let Some(h) = &mut self.hover {
                    if !h.text {
                        self.hover = None;
                    } else {
                        h.kind = DropKind::Text;
                    }
                }
                self.answer()
            }
            Event::DragLeave => {
                self.hover = None;
                Reply::redraw()
            }
            Event::DragDrop {
                x, y, data, action, ..
            } => {
                self.hover = None;
                let mut reply = match data {
                    Dropped::Text(text) => {
                        let at = self.drop_pos(x, y, ctx);
                        if self.edit.drop_text(at, &text) {
                            self.dirty = true;
                            self.scroll_to_caret(ctx);
                            self.sync_menus();
                        }
                        Reply::redraw()
                    }
                    Dropped::Files(paths) => match paths.into_iter().next() {
                        Some(p) => self.confirm_then(PendingAction::OpenFile(p)),
                        None => Reply::redraw(),
                    },
                    Dropped::Uris(_) => Reply::redraw(),
                };
                if action == DragAction::Ask {
                    reply.answer_drop = Some(Some(DragAction::Copy));
                }
                reply
            }
            _ => Reply::none(),
        }
    }

    fn answer(&self) -> Reply {
        Reply {
            redraw: true,
            ..Reply::drag_accept(self.hover.map(|h| h.accept()))
        }
    }

    fn drop_pos(&self, x: i32, y: i32, ctx: &Ctx) -> Pos {
        let lay = self.edit_layout(ctx);
        multiedit::pos_at(
            &self.mono,
            &self.metrics,
            &self.edit,
            &lay,
            self.view,
            x,
            y,
            1,
        )
    }

    fn scroll_step(&self, ctx: &Ctx) -> Option<View> {
        let h = self.hover.filter(|h| h.kind == DropKind::Text)?;
        let lay = self.edit_layout(ctx);
        multiedit::drag_scroll(
            &self.mono,
            &self.metrics,
            &self.edit,
            &lay,
            self.view,
            h.xy.0,
            h.xy.1,
            1,
        )
    }

    pub(crate) fn drop_point(&self, ctx: &Ctx) -> Option<Pos> {
        let h = self.hover.filter(|h| h.kind == DropKind::Text)?;
        Some(self.drop_pos(h.xy.0, h.xy.1, ctx))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::test_ctx;
    use crate::{MAIN, Notepad};
    use repriseapp::{Actions, App, DragAction, DropKind, Dropped, Event, Offered};
    use reprisekit::controls::form;
    use reprisekit::controls::msgbox;
    use std::path::PathBuf;

    fn enter(offered: Offered) -> Event {
        Event::DragEnter {
            x: 100,
            y: 100,
            offered,
            source: Actions::ALL,
            action: None,
            own: false,
        }
    }

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
    const LINK: Offered = Offered {
        files: true,
        link: true,
        text: true,
    };

    fn drop(data: Dropped, action: DragAction) -> Event {
        Event::DragDrop {
            x: 0,
            y: 0,
            data,
            action,
            source: Actions::ALL,
        }
    }

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("reprise-np-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn text_is_taken_as_a_copy_and_shows_the_drop_point() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        let r = np.event(MAIN, enter(TEXT), &mut ctx);
        let a = r.drag_accept.flatten().expect("text is taken");
        assert_eq!(a.kind, DropKind::Text);
        assert_eq!(a.preferred, Some(DragAction::Copy));
        assert!(a.actions.copy && a.actions.move_, "Shift moves it");
        assert_eq!(np.hover.map(|h| h.kind), Some(DropKind::Text));
        assert!(r.redraw, "the drop point shows");
    }

    #[test]
    fn a_file_managers_drag_is_taken_as_files() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        let r = np.event(MAIN, enter(FILES), &mut ctx);
        let a = r.drag_accept.flatten().expect("files are taken");
        assert_eq!(a.kind, DropKind::Files);
        assert!(!a.actions.move_, "opening a file never moves it");
    }

    #[test]
    fn a_browsers_link_is_taken_as_text() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        let r = np.event(MAIN, enter(LINK), &mut ctx);
        assert_eq!(
            r.drag_accept.flatten().map(|a| a.kind),
            Some(DropKind::Text)
        );
    }

    #[test]
    fn a_uri_list_without_files_falls_back_to_its_text() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.event(MAIN, enter(FILES), &mut ctx);
        let r = np.event(MAIN, Event::DragFiles { paths: vec![] }, &mut ctx);
        assert_eq!(
            r.drag_accept.flatten().map(|a| a.kind),
            Some(DropKind::Text)
        );

        let only_uris = Offered {
            text: false,
            ..FILES
        };
        np.event(MAIN, enter(only_uris), &mut ctx);
        let r = np.event(MAIN, Event::DragFiles { paths: vec![] }, &mut ctx);
        assert_eq!(r.drag_accept, Some(None), "nothing Notepad can open");
    }

    #[test]
    fn a_drag_is_refused_while_a_modal_box_is_up() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.about = true;
        let r = np.event(MAIN, enter(TEXT), &mut ctx);
        assert_eq!(r.drag_accept, Some(None));
        assert!(np.hover.is_none());
    }

    #[test]
    fn leaving_clears_the_drop_point() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.event(MAIN, enter(TEXT), &mut ctx);
        let r = np.event(MAIN, Event::DragLeave, &mut ctx);
        assert!(np.hover.is_none());
        assert!(r.redraw);
    }

    #[test]
    fn a_text_drag_resting_on_the_bottom_row_scrolls_until_it_moves_away() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.edit = reprisekit::controls::multiedit::MultiEdit::new(&"line\n".repeat(200));
        let lay = np.edit_layout(&ctx);
        let (x, bottom) = (lay.inner.x + 40, lay.inner.bottom() - 2);
        let over = |x, y, offered| Event::DragEnter {
            x,
            y,
            offered,
            source: Actions::ALL,
            action: None,
            own: false,
        };
        let at = |x, y| over(x, y, TEXT);
        let r = np.event(MAIN, at(x, lay.inner.y + 40), &mut ctx);
        assert!(!r.tick, "the middle of the edit rests still");
        let r = np.event(
            MAIN,
            Event::DragMotion {
                x,
                y: bottom,
                action: None,
            },
            &mut ctx,
        );
        assert!(r.tick, "the bottom row asks for a tick");
        assert_eq!(np.view.top, 0, "nothing moves before the tick");
        for top in 1..=3 {
            let r = np.event(MAIN, Event::Tick, &mut ctx);
            assert!(r.tick && r.redraw);
            assert_eq!(np.view.top, top);
        }
        np.event(
            MAIN,
            Event::DragMotion {
                x,
                y: lay.inner.y + 40,
                action: None,
            },
            &mut ctx,
        );
        let r = np.event(MAIN, Event::Tick, &mut ctx);
        assert!(!r.tick, "moved away, the ticks stop");
        assert_eq!(np.view.top, 3);

        np.event(MAIN, at(x, lay.inner.y + 2), &mut ctx);
        np.event(MAIN, Event::Tick, &mut ctx);
        assert_eq!(np.view.top, 2);
        np.event(MAIN, Event::DragLeave, &mut ctx);
        let r = np.event(MAIN, Event::Tick, &mut ctx);
        assert!(!r.tick);
        assert_eq!(np.view.top, 2);

        let r = np.event(MAIN, over(x, bottom, FILES), &mut ctx);
        assert!(!r.tick);
        assert!(!np.event(MAIN, Event::Tick, &mut ctx).tick);
        assert_eq!(np.view.top, 2);
    }

    #[test]
    fn dropped_text_lands_under_the_pointer_and_dirties_the_document() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.edit = reprisekit::controls::multiedit::MultiEdit::new("tail");
        np.event(MAIN, enter(TEXT), &mut ctx);
        np.event(
            MAIN,
            drop(Dropped::Text("head ".into()), DragAction::Copy),
            &mut ctx,
        );
        assert_eq!(
            np.edit.text(),
            "head tail",
            "(0, 0) is the first line's start"
        );
        assert!(np.dirty);
        assert!(np.hover.is_none(), "the drop ends the hover");
    }

    #[test]
    fn a_dropped_file_opens() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        let p = temp_file("drop-open.txt", "from the drop");
        np.event(
            MAIN,
            drop(Dropped::Files(vec![p.clone()]), DragAction::Copy),
            &mut ctx,
        );
        assert_eq!(np.edit.text(), "from the drop");
        assert_eq!(np.path.as_ref(), Some(&p));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn a_file_dropped_on_unsaved_changes_asks_first() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.edit = reprisekit::controls::multiedit::MultiEdit::new("mine");
        np.dirty = true;
        let p = temp_file("drop-dirty.txt", "theirs");
        np.event(
            MAIN,
            drop(Dropped::Files(vec![p.clone()]), DragAction::Copy),
            &mut ctx,
        );
        assert!(np.savechanges, "the save prompt is up");
        assert_eq!(np.edit.text(), "mine");
        np.savechanges_outcome(form::Event::Pressed(msgbox::button_id(1)));
        assert_eq!(np.edit.text(), "theirs", "No opens the dropped file");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn an_ask_drop_is_answered_as_a_copy() {
        let (mut np, mut ctx) = (Notepad::new(None), test_ctx());
        np.event(MAIN, enter(TEXT), &mut ctx);
        let r = np.event(
            MAIN,
            drop(Dropped::Text("x".into()), DragAction::Ask),
            &mut ctx,
        );
        assert_eq!(r.answer_drop, Some(Some(DragAction::Copy)));
        assert_eq!(np.edit.text(), "x");
    }
}
