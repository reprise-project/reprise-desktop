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

    #[test]
    fn the_keyboard_opens_it_under_the_caret_even_in_a_box_with_no_field_rect() {
        let caret = Rect::new(30, 12, 1, 16);
        let spot = TextEdit::new("x", false).spot(Rect::new(10, 10, 100, 20), caret);
        assert_eq!(caret_point(&spot), (30, 28));
        let rename = TextEdit::new("x", false).spot(Rect::default(), caret);
        assert_eq!(caret_point(&rename), (30, 28));
    }
}
