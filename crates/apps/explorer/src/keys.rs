//! Where a key goes. An open menu owns the keyboard; otherwise the
//! window's accelerators answer first (they are [`Command`]s, and the
//! table in [`crate::commands`] is the only place that says what a key
//! means), then the menu bar's F10, Escape and Alt+letter access keys,
//! then the Address field, Tab's focus ring and whatever pane has the
//! focus. Nothing here changes the model directly: every branch ends in
//! a [`Command`] and [`Explorer::run`].

use crate::commands::{Command, command_for_key_in};
use crate::menus;
use crate::window::Popup;
use crate::{Explorer, Focus};
use repriseapp::{Ctx, KeyInput, Reply};
use reprisekit::controls::addressbar;
use reprisekit::controls::iconview::IconMove;
use reprisekit::controls::menu::{MenuAction, MenuKey};
use reprisekit::controls::menubar::{Action, Key};
use reprisekit::controls::shellview;
use reprisekit::controls::treeview;
use reprisekit::selection::Mods;
use reprisekit::textedit::keysym::{self, ALT_L, ALT_R, ISO_LEFT_TAB, SPACE};

impl Explorer {
    pub fn key_inner(&mut self, k: KeyInput, ctx: &mut Ctx) -> Reply {
        // Alt on its own arms the menu bar (the release fires it);
        // any other key is what it is and cancels the arm.
        if k.sym == ALT_L || k.sym == ALT_R {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();

        if self.popup.is_some() {
            return self.popup_key(k, ctx);
        }

        self.sync_menus();
        // Escape abandons a drag in progress: the band goes
        // and the selection or the column width goes back to what the
        // press found. The button is still down, so the state is cleared
        // here and the release finds nothing left to commit.
        if k.sym == keysym::ESCAPE && self.cancel_drag() {
            self.refresh_chrome(ctx);
            return Reply::redraw();
        }
        // The rename box swallows everything while it is up: the menu
        // bar's access keys would eat the letters otherwise.
        if let Some(reply) = self.rename_key(&k, ctx) {
            return reply;
        }

        // An open menu owns the keyboard outright: its arrows, Enter,
        // Escape and access keys are the menu's, not the window's.
        if self.bar.open.is_some()
            && let Some(reply) = self.bar_key(&k, ctx)
        {
            return reply;
        }

        // The accelerators, ahead of the bar's access keys: none of them
        // is a title's access key, and the bar must not swallow Alt+D
        // while it sits merely active.
        let cmd = command_for_key_in(
            k.sym,
            k.mods.ctrl,
            k.mods.shift,
            k.mods.alt,
            self.focus,
            self.sv.edit.is_some(),
        );
        if cmd != Command::Nothing {
            if self.bar.is_active() {
                self.bar.close();
            }
            return self.run(cmd, ctx);
        }

        // An Alt+letter is never text, so the field lets it through to
        // the bar's access keys below.
        if self.focus == Focus::Address && !k.mods.alt {
            if matches!(k.sym, keysym::RETURN | keysym::KP_ENTER) {
                let ev = self.address.enter();
                return self.address_event(ev, ctx);
            }
            // Escape: the field goes back to where we actually are
            // and the focus drops into the view.
            if k.sym == keysym::ESCAPE {
                let icon = ctx.icons.get(self.place.icon_name(), 16);
                self.address.set_place(&self.place.address(), icon);
                self.focus = Focus::View;
                return Reply::redraw();
            }
            if let Some(ek) = k.edit.clone() {
                self.address.key(ek);
                return Reply::redraw();
            }
        }

        if let Some(reply) = self.bar_key(&k, ctx) {
            return reply;
        }

        // Never while a rename is in progress: `rename_key` commits on
        // Tab and returns above, and the focus must not move out from
        // under an open box in any case.
        if matches!(k.sym, keysym::TAB | ISO_LEFT_TAB) && self.sv.edit.is_none() {
            let back = k.mods.shift || k.sym == ISO_LEFT_TAB;
            self.focus = self.next_focus(back);
            return Reply::redraw();
        }

        match self.focus {
            Focus::Tree => self.tree_key(k, ctx),
            Focus::View => self.view_key(k, ctx),
            Focus::Address => Reply::none(),
        }
    }

    /// The menu bar's turn at a key: F10, Escape, its navigation while active, and Alt+letter access keys. `None` when it wanted nothing.
    fn bar_key(&mut self, k: &KeyInput, ctx: &mut Ctx) -> Option<Reply> {
        let lay = self.bar_layout(ctx);
        let (m, px) = (ctx.theme.clone(), ctx.theme.fonts.size_px);
        let bar_key = match k.sym {
            // Shift+F10 is the context menu key, not the menu bar's.
            keysym::F10 if !k.mods.shift => Some(Key::Activate),
            keysym::ESCAPE => Some(Key::Escape),
            keysym::LEFT | keysym::KP_LEFT if self.bar.is_active() => Some(Key::Left),
            keysym::RIGHT | keysym::KP_RIGHT if self.bar.is_active() => Some(Key::Right),
            keysym::UP | keysym::KP_UP if self.bar.is_active() => Some(Key::Up),
            keysym::DOWN | keysym::KP_DOWN if self.bar.is_active() => Some(Key::Down),
            keysym::RETURN | keysym::KP_ENTER if self.bar.is_active() => Some(Key::Enter),
            _ => k
                .utf8
                .as_deref()
                .and_then(|s| s.chars().next())
                .filter(|c| c.is_alphanumeric())
                .map(|c| Key::Char(c, k.mods.alt)),
        }?;
        match self.bar.key(bar_key, &lay, &ctx.text, &m, px) {
            Action::Chosen { title, path, .. } => {
                let cmd = menus::command_for(title, &path, &self.menu_state());
                Some(self.run(cmd, ctx))
            }
            Action::Redraw => Some(Reply::redraw()),
            Action::Ignored => None,
        }
    }

    fn next_focus(&self, back: bool) -> Focus {
        // The left column's pane, when it takes the keyboard.
        let bar = self.settings.folders.then_some(Focus::Tree);
        match (self.focus, back) {
            (Focus::Address, false) => bar.unwrap_or(Focus::View),
            (Focus::Tree, false) => Focus::View,
            (Focus::View, false) => Focus::Address,
            (Focus::Address, true) => Focus::View,
            (Focus::Tree, true) => Focus::Address,
            (Focus::View, true) => bar.unwrap_or(Focus::Address),
        }
    }

    /// Shift+F10 and the Menu key: the context menu of whatever has the focus, opened at the focused item's lower-left corner (the tree's node, the view's entry, else the background menu).
    pub fn context_menu_key(&mut self, ctx: &mut Ctx) -> Reply {
        let lay = self.layout(ctx);
        let px = ctx.theme.fonts.size_px;
        if self.focus == Focus::Tree {
            let Some(frame) = lay.tree else {
                return Reply::none();
            };
            let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.look.controls());
            let rows = self.tree.visible();
            // The selected row's lower-left, when it is on screen.
            let at = self
                .tree
                .selected_row(&rows)
                .filter(|i| *i >= self.tree.top && *i < self.tree.top + tl.rows)
                .map(|i| {
                    let below = (i - self.tree.top) as i32 + 1;
                    (tl.body.x + 8, tl.body.y + below * self.tree.row_h())
                })
                .unwrap_or((tl.body.x + 8, tl.body.y + 8));
            let node = self
                .tree
                .selected
                .clone()
                .and_then(|p| self.tree.node(&p))
                .map(|n| (n.expanded, n.has_children))
                .unwrap_or((false, false));
            let folder = self.node_folder().is_some();
            let menu = menus::tree_menu(node.0, node.1, folder);
            self.popup = Some(Popup::new(at, menu, lay.client, ctx));
            return Reply::redraw();
        }
        let sl = self.view_layout(lay.view, ctx);
        let cell = self
            .sv
            .selection
            .focus
            .filter(|i| self.sv.selection.set.contains(i))
            .and_then(|i| sl.cells.get(i).map(|c| (i, *c)));
        match cell {
            Some((i, c)) => self.entry_menu_at(i, (c.x + 8, c.bottom()), ctx),
            None => {
                let menu = menus::background_menu(&self.menu_state());
                let at = (lay.view.x + 8, lay.view.y + 8);
                self.popup = Some(Popup::new(at, menu, lay.client, ctx));
                Reply::redraw()
            }
        }
    }

    fn tree_key(&mut self, k: KeyInput, ctx: &mut Ctx) -> Reply {
        let Some(frame) = self.layout(ctx).tree else {
            return Reply::none();
        };
        let tl = treeview::layout(
            &self.tree,
            &ctx.text,
            ctx.theme.fonts.size_px,
            frame,
            &ctx.theme.look.controls(),
        );
        let rows = tl.rows as i64;
        let moved = match k.sym {
            keysym::UP | keysym::KP_UP => self.tree.step(-1, &tl),
            keysym::DOWN | keysym::KP_DOWN => self.tree.step(1, &tl),
            keysym::PAGE_UP | keysym::KP_PAGE_UP => self.tree.step(-rows, &tl),
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => self.tree.step(rows, &tl),
            keysym::HOME | keysym::KP_HOME => self.tree.step(i64::MIN / 2, &tl),
            keysym::END | keysym::KP_END => self.tree.step(i64::MAX / 2, &tl),
            keysym::LEFT | keysym::KP_LEFT => self.tree.collapse_selected(&tl),
            keysym::RIGHT | keysym::KP_RIGHT => {
                if let Some(p) = self.tree.expand_selected(&tl) {
                    self.fill_node(p, ctx);
                }
                return Reply::redraw();
            }
            _ => return Reply::none(),
        };
        if !moved {
            return Reply::none();
        }
        match self.tree.selected.clone().and_then(|p| self.place_of(&p)) {
            Some(p) if p != self.place => self.go(p, ctx),
            _ => Reply::redraw(),
        }
    }

    fn view_key(&mut self, k: KeyInput, ctx: &mut Ctx) -> Reply {
        let frame = self.layout(ctx).view;
        let sl = self.view_layout(frame, ctx);
        let mods = Mods {
            ctrl: k.mods.ctrl,
            shift: k.mods.shift,
        };
        let dir = match k.sym {
            keysym::UP | keysym::KP_UP => Some(IconMove::Up),
            keysym::DOWN | keysym::KP_DOWN => Some(IconMove::Down),
            keysym::LEFT | keysym::KP_LEFT => Some(IconMove::Left),
            keysym::RIGHT | keysym::KP_RIGHT => Some(IconMove::Right),
            keysym::HOME | keysym::KP_HOME => Some(IconMove::Home),
            keysym::END | keysym::KP_END => Some(IconMove::End),
            _ => None,
        };
        if let Some(dir) = dir {
            let moved = shellview::step(&mut self.sv, dir, &sl, mods);
            if moved {
                self.refresh_chrome(ctx);
            }
            return Reply::redraw();
        }
        if matches!(k.sym, keysym::RETURN | keysym::KP_ENTER) {
            return self.open_selected(ctx);
        }
        // Ctrl+Space: the focused entry joins or leaves the
        // selection without disturbing the rest. A bare space is a
        // type-ahead character, so it goes on below.
        if k.sym == SPACE
            && k.mods.ctrl
            && let Some(i) = self.sv.selection.focus
        {
            self.sv.selection.click(
                i,
                Mods {
                    ctrl: true,
                    shift: false,
                },
            );
            self.refresh_chrome(ctx);
            return Reply::redraw();
        }
        let ch = k
            .utf8
            .as_deref()
            .and_then(|s| s.chars().next())
            .filter(|c| c.is_alphanumeric() && !k.mods.ctrl && !k.mods.alt);
        if let Some(ch) = ch
            && shellview::type_ahead(&mut self.sv, ch)
        {
            shellview::follow(&mut self.sv, &sl);
            self.refresh_chrome(ctx);
            return Reply::redraw();
        }
        Reply::none()
    }

    /// A key while a context menu or a toolbar drop is open: arrows, Enter and Escape go to the chain, Right opens a submenu row.
    fn popup_key(&mut self, k: KeyInput, ctx: &mut Ctx) -> Reply {
        let key = match k.sym {
            keysym::ESCAPE => MenuKey::Escape,
            keysym::UP | keysym::KP_UP => MenuKey::Up,
            keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
            keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
            keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
            keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
            _ => match k.utf8.as_deref().and_then(|s| s.chars().next()) {
                Some(c) if c.is_alphanumeric() => MenuKey::Char(c),
                _ => return Reply::none(),
            },
        };
        let client = self.layout(ctx).client;
        let Some(p) = self.popup.as_mut() else {
            return Reply::none();
        };
        match p.key(key, ctx, client) {
            MenuAction::Chosen(path) => {
                let cmd = p.command(&path);
                self.popup = None;
                self.run(cmd, ctx)
            }
            MenuAction::Close => {
                self.popup = None;
                Reply::redraw()
            }
            MenuAction::Redraw => Reply::redraw(),
            MenuAction::None => Reply::none(),
        }
    }

    pub fn address_event(&mut self, ev: addressbar::Event, ctx: &mut Ctx) -> Reply {
        match ev {
            addressbar::Event::Navigate(text) => {
                let reply = self.run(Command::Navigate(text), ctx);
                // The focus drops into the view once the address
                // takes, so Backspace there is Up and not a deletion. A
                // failure leaves the caret in the field behind the box.
                if self.dialog.is_none() {
                    self.focus = Focus::View;
                }
                reply
            }
            addressbar::Event::Pick(i) => match self.address_places.get(i).cloned() {
                Some(p) => {
                    self.focus = Focus::View;
                    self.run(Command::GoTo(p), ctx)
                }
                None => Reply::redraw(),
            },
            addressbar::Event::Redraw => Reply::redraw(),
            addressbar::Event::None => Reply::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Place;
    use crate::tests::test_ctx;
    use reprisekit::controls::shellview;

    fn key(sym: u32, utf8: Option<&str>, ctrl: bool, shift: bool, alt: bool) -> KeyInput {
        KeyInput {
            sym,
            utf8: utf8.map(|s| s.to_string()),
            mods: repriseapp::Mods { ctrl, shift, alt },
            edit: reprisekit::textedit::key_for(sym, ctrl, shift, utf8),
        }
    }

    #[test]
    fn tab_commits_an_in_place_rename_and_leaves_the_focus_in_the_view() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("a.txt"), "x").expect("write");
        let mut ctx = test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set_only(Some(0));

        // F2, then type a new stem over the selected one.
        ex.key_inner(key(keysym::F2, None, false, false, false), &mut ctx);
        assert!(ex.sv.edit.is_some(), "the box is open");
        for ch in ["b", "b"] {
            ex.key_inner(
                key(
                    ch.bytes().next().unwrap() as u32,
                    Some(ch),
                    false,
                    false,
                    false,
                ),
                &mut ctx,
            );
        }

        ex.key_inner(key(keysym::TAB, None, false, false, false), &mut ctx);
        assert!(ex.sv.edit.is_none(), "Tab closed the box");
        assert!(dir.path().join("bb.txt").exists(), "and wrote the name");
        assert!(!dir.path().join("a.txt").exists());
        assert_eq!(ex.focus, Focus::View, "the focus stayed in the view");

        // The next key is the view's again, not the vanished box's.
        ex.key_inner(key(0x0062, Some("b"), false, false, false), &mut ctx);
        assert!(
            ex.sv.edit.is_none(),
            "a letter type-aheads, it does not edit"
        );

        // Shift+Tab commits too.
        ex.sv.selection.set_only(Some(0));
        ex.key_inner(key(keysym::F2, None, false, false, false), &mut ctx);
        ex.key_inner(key(ISO_LEFT_TAB, None, false, true, false), &mut ctx);
        assert!(ex.sv.edit.is_none());
    }

    #[test]
    fn alt_enter_shows_the_selected_files_properties_and_enter_opens_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("a.txt"), "x").expect("write");
        let mut ctx = test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set_only(Some(0));

        ex.key_inner(key(ALT_L, None, false, false, true), &mut ctx);
        let reply = ex.key_inner(key(keysym::RETURN, None, false, false, true), &mut ctx);
        assert!(reply.spawn.is_empty(), "Alt+Enter launches nothing");
        assert!(ex.props.is_some(), "Alt+Enter put up the sheet");

        ex.props = None;
        let reply = ex.key_inner(key(keysym::RETURN, None, false, false, false), &mut ctx);
        assert!(ex.props.is_none(), "a bare Enter puts up no sheet");
        assert_eq!(reply.spawn.len(), 1, "a bare Enter opens the file");
    }

    #[test]
    fn the_context_menu_key_opens_at_the_focused_entrys_cell() {
        let dir = tempfile::tempdir().expect("a temp dir");
        for name in ["a.txt", "b.txt"] {
            std::fs::write(dir.path().join(name), "x").expect("write");
        }
        let mut ctx = test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set_only(Some(1));

        let lay = ex.layout(&ctx);
        let sl = shellview::layout(&ex.sv, lay.view, &ctx.theme.look.controls());
        let cell = sl.cells[1];
        ex.key_inner(
            key(reprisekit::textedit::keysym::F10, None, false, true, false),
            &mut ctx,
        );
        let popup = ex.popup.as_ref().expect("the menu is up");
        assert_eq!(popup.origin(), (cell.x + 8, cell.bottom()));

        // With nothing selected it is the background menu, at the
        // view's own corner.
        ex.popup = None;
        ex.sv.selection.clear();
        ex.key_inner(key(keysym::MENU, None, false, false, false), &mut ctx);
        let popup = ex.popup.as_ref().expect("the background menu is up");
        assert_eq!(popup.origin(), (lay.view.x + 8, lay.view.y + 8));
    }
}
