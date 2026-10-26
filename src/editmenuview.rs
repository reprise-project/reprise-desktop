//! The edit menu over the compositor's own fields ([`Editor`]): a right press in a field, the
//! Menu key or Shift+F10 open [`editmenu`]'s rows as a context menu, and a pick runs as the
//! field's shortcut key, so its own clipboard and undo handling does the work.

use reprisekit::controls::editmenu::{self, Command, Live};
use reprisekit::textedit::key_for;

use crate::caret::Editor;
use crate::clipboard::{TEXT_MIMES, answer, text_mime};
use crate::compositor::State;
use crate::dialogview::DialogKey;
use crate::keyroute::KeyRoute;
use crate::menuview::{MenuFrom, MenuKind};

/// The key press a pick stands for, as the keyboard would have sent it.
pub fn pick_key(cmd: Command) -> DialogKey {
    let (sym, ctrl) = editmenu::shortcut(cmd);
    DialogKey {
        syms: vec![sym],
        mods: if ctrl { crate::keys::MOD_CTRL } else { 0 },
        edit: key_for(sym, ctrl, false, None),
    }
}

/// Where the keyboard opens the menu, in the field's own px: under the caret, kept inside the
/// field unless it reports none (the desktop's rename box).
fn caret_point(spot: &reprisekit::textedit::TextSpot) -> (i32, i32) {
    if spot.field.width == 0 {
        return (spot.caret.x, spot.caret.bottom());
    }
    editmenu::under_caret(spot)
}

impl State {
    /// Whether the clipboard holds text, offered by Reprise itself or by a client.
    pub fn clip_has_text(&self) -> bool {
        match self.own_offers() {
            Some(offers) => TEXT_MIMES.iter().any(|m| answer(&offers, m).is_some()),
            None => text_mime(self.clip_offered()).is_some(),
        }
    }

    /// What field `editor` can do right now, read off its model.
    fn editor_live(&self, editor: Editor) -> Option<Live> {
        use reprisekit::controls::form::FocusedEdit;
        let paste = self.clip_has_text();
        let line = |e: &reprisekit::textedit::TextEdit| Live::of_edit(e, paste);
        match editor {
            Editor::Dialog => match self.dialogs.views.last()?.dialog.focused_edit()? {
                FocusedEdit::Line(e) => Some(line(e)),
                FocusedEdit::Lines(m) => Some(Live {
                    undo: m.can_undo(),
                    redo: m.can_redo(),
                    selection: m.has_selection(),
                    paste,
                    secret: false,
                }),
            },
            Editor::Rename => self.desktop.as_ref()?.rename.as_ref().map(|(_, e)| line(e)),
            Editor::Launcher => self.launcher.as_ref().map(|v| line(v.model.edit())),
            Editor::StartPanel => self.start_panel.as_ref().map(|v| line(v.model.edit())),
            Editor::ClipView => self.panels.clipview.as_ref().map(|v| line(v.model.edit())),
            Editor::EmojiView => self.panels.emojiview.as_ref().map(|v| line(v.model.edit())),
            Editor::NotifyReply => crate::notifyview::reply_edit(self).map(line),
        }
    }

    /// Opens the edit menu over the focused field: at `at` (global) for a right press, else under
    /// the caret it last drew. `false` when no field has the keyboard.
    pub fn open_edit_menu(&mut self, at: Option<(i32, i32)>) -> bool {
        let Some(editor) = self.editor() else {
            return false;
        };
        let Some(live) = self.editor_live(editor) else {
            return false;
        };
        let keyboard = at.is_none();
        let at = match at {
            Some(at) => at,
            None => {
                let Some(d) = self.ime.drawn.as_ref().filter(|d| d.editor == editor) else {
                    return false;
                };
                let (x, y) = caret_point(&d.spot);
                (d.origin.0 + x, d.origin.1 + y)
            }
        };
        let from = match editor {
            Editor::StartPanel => MenuFrom::StartPanel,
            _ => MenuFrom::Elsewhere,
        };
        self.close_menu();
        let items = editmenu::items(live);
        self.push_context_menu(at, items, MenuKind::Edit { editor }, from);
        if keyboard && !self.menus.is_empty() {
            self.enter_menu_keys();
        }
        true
    }

    /// Whether the open menu is a field's edit menu.
    pub fn edit_menu_open(&self) -> bool {
        self.menus
            .first()
            .is_some_and(|m| matches!(m.kind, MenuKind::Edit { .. }))
    }

    /// The Menu key or Shift+F10 on a key routed to a field opens its edit menu; `true` when
    /// it did and the key goes no further. The launcher and the Start menu's search hand the
    /// key to a lit row's menu first and open this one themselves.
    pub fn edit_menu_key(&mut self, route: &KeyRoute, syms: &[u32], mods: u32) -> bool {
        let shift = mods & crate::keys::MOD_SHIFT != 0;
        crate::imefield::field_route(route)
            && !matches!(route, KeyRoute::Launcher | KeyRoute::StartPanel)
            && syms.iter().any(|&s| editmenu::opens(s, shift))
            && self.open_edit_menu(None)
    }

    /// A pick from the edit menu over `editor`, run as its shortcut while that field still has
    /// the keyboard.
    pub fn edit_menu_pick(&mut self, editor: Editor, cmd: Command) {
        if self.editor() != Some(editor) {
            return;
        }
        self.editor_key(editor, &pick_key(cmd));
    }

    /// `k` for field `editor`, as its key route delivers one.
    pub fn editor_key(&mut self, editor: Editor, k: &DialogKey) {
        match editor {
            Editor::Launcher => self.launcher_key(k),
            Editor::StartPanel => self.start_panel_key(k),
            Editor::Dialog => self.dialog_key(k),
            Editor::ClipView => self.clipview_key(k),
            Editor::EmojiView => self.emoji_key(k),
            Editor::NotifyReply => crate::notifyview::key(self, k),
            Editor::Rename => {
                if let Some(e) = k.edit.clone() {
                    self.rename_edit(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::raster::Rect;
    use reprisekit::textedit::{EditKey, TextEdit, keysym};

    #[test]
    fn a_pick_is_the_fields_own_shortcut() {
        let copy = pick_key(Command::Copy);
        assert_eq!((copy.syms, copy.mods), (vec![0x63], crate::keys::MOD_CTRL));
        assert_eq!(copy.edit, Some(EditKey::Copy));
        assert_eq!(pick_key(Command::Paste).edit, Some(EditKey::Paste));
        assert_eq!(pick_key(Command::Undo).edit, Some(EditKey::Undo));
        assert_eq!(pick_key(Command::SelectAll).edit, Some(EditKey::SelectAll));
        let delete = pick_key(Command::Delete);
        assert_eq!((delete.syms, delete.mods), (vec![keysym::DELETE], 0));
        assert_eq!(delete.edit, Some(EditKey::Delete { word: false }));
    }

    /// The first point of a 1024x768 output, on a 2 px grid, whose scene hit is `want`.
    fn point_where(
        h: &crate::harness::Harness,
        want: impl Fn(&crate::input::Hit) -> bool,
    ) -> (f64, f64) {
        (0..384)
            .flat_map(|y| (0..512).map(move |x| (f64::from(x * 2), f64::from(y * 2))))
            .find(|&(x, y)| want(&h.state.scene_hit((x, y).into())))
            .expect("the point is on screen")
    }

    #[test]
    fn a_right_press_in_a_panels_field_opens_its_edit_menu_and_a_pick_runs_there() {
        use crate::input::{BTN_RIGHT, Hit};
        use reprisekit::controls::{cliphistory, emojipicker, launcher};
        type Open = fn(&mut State);
        type OnField = fn(&Hit) -> bool;
        type Text = fn(&State) -> Option<(String, std::ops::Range<usize>)>;
        let panels: [(Editor, Open, OnField, Text); 3] = [
            (
                Editor::Launcher,
                |s| {
                    s.toggle_launcher();
                    s.launcher_paste("foot");
                },
                |h| {
                    matches!(
                        h,
                        Hit::Launcher {
                            hit: launcher::Hit::Field
                        }
                    )
                },
                |s| {
                    s.launcher
                        .as_ref()
                        .map(|v| (v.model.edit().text.clone(), v.model.edit().selection()))
                },
            ),
            (
                Editor::ClipView,
                |s| {
                    s.toggle_clipview();
                    s.clipview_paste("foot");
                },
                |h| {
                    matches!(
                        h,
                        Hit::ClipView {
                            hit: cliphistory::Hit::Field
                        }
                    )
                },
                |s| {
                    s.panels
                        .clipview
                        .as_ref()
                        .map(|v| (v.model.edit().text.clone(), v.model.edit().selection()))
                },
            ),
            (
                Editor::EmojiView,
                |s| {
                    s.toggle_emoji();
                    s.emoji_paste("foot");
                },
                |h| {
                    matches!(
                        h,
                        Hit::EmojiView {
                            hit: emojipicker::Hit::Field
                        }
                    )
                },
                |s| {
                    s.panels
                        .emojiview
                        .as_ref()
                        .map(|v| (v.model.edit().text.clone(), v.model.edit().selection()))
                },
            ),
        ];
        for (editor, open, on_field, text) in panels {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            open(&mut h.state);
            assert_eq!(
                text(&h.state),
                Some(("foot".into(), 4..4)),
                "{editor:?} opens"
            );
            let (x, y) = point_where(&h, on_field);
            h.pointer_to(x, y);
            h.button(BTN_RIGHT, true);
            h.button(BTN_RIGHT, false);
            assert!(
                matches!(h.state.menus.first().map(|m| &m.kind), Some(MenuKind::Edit { editor: e }) if *e == editor),
                "{editor:?}: a right press in the field opens its edit menu"
            );
            let (x, y) = point_where(
                &h,
                |hit| matches!(hit, Hit::Menu { row: Some(r), .. } if editmenu::command(&[*r]) == Some(Command::SelectAll)),
            );
            h.click(x, y);
            assert!(
                h.state.menus.is_empty(),
                "{editor:?}: the pick closes the menu"
            );
            assert_eq!(
                text(&h.state),
                Some(("foot".into(), 0..4)),
                "{editor:?}: Select All runs in the field, which stays open"
            );
        }
    }

    #[test]
    fn the_keyboard_opens_it_under_the_caret_even_in_a_box_with_no_field_rect() {
        let caret = Rect::new(30, 12, 1, 16);
        let spot = TextEdit::new("x", false).spot(Rect::new(10, 10, 100, 20), caret);
        assert_eq!(caret_point(&spot), (30, 28));
        let rename = TextEdit::new("x", false).spot(Rect::default(), caret);
        assert_eq!(caret_point(&rename), (30, 28));
    }
}
