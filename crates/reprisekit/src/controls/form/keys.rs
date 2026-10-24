use crate::controls::arrange;
use crate::controls::hotkey;
use crate::controls::iconlist;
use crate::controls::listbox;
use crate::controls::multiedit;
use crate::controls::treeview;
use crate::textedit::EditKey;

use super::item::Item;
use super::layout::icon_list_layout;
use super::state::toggle_check;
use super::{Control, Event, Form};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tab {
        back: bool,
    },
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    Enter,
    Escape,
    Backspace,
    Char(char),
    /// Alt+letter: the access key, whichever control has the focus. A
    /// bare `Char` reaches a focused edit field, list or combo first;
    /// this never does (Alt+B on Run presses Browse...).
    Access(char),
    Edit(EditKey),
    Drop,
    /// Ctrl+Enter, which a host reads only for a form: a new line in a
    /// focused memo, and Enter anywhere else.
    NewLine,
    /// Ctrl+Home, or Ctrl+End when `end`, Shift to `select`: a focused
    /// memo's caret to its first or last character, and Home or End
    /// anywhere else.
    DocEdge {
        end: bool,
        select: bool,
    },
}

/// The dialog key a keysym stands for before any editing-key reading, or `None`; one table for every host.
pub fn special_key(sym: u32, shift: bool, alt: bool) -> Option<Key> {
    use crate::textedit::keysym;
    Some(match sym {
        keysym::TAB => Key::Tab { back: shift },
        keysym::ISO_LEFT_TAB => Key::Tab { back: true },
        keysym::RETURN | keysym::KP_ENTER => Key::Enter,
        keysym::ESCAPE => Key::Escape,
        keysym::UP | keysym::KP_UP => Key::Up,
        keysym::DOWN | keysym::KP_DOWN if alt => Key::Drop,
        keysym::DOWN | keysym::KP_DOWN => Key::Down,
        keysym::PAGE_UP | keysym::KP_PAGE_UP => Key::PageUp,
        keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Key::PageDown,
        keysym::F4 => Key::Drop,
        _ => return None,
    })
}

/// The way Ctrl+arrow `sym` moves a focused arrangement's picked tile
/// ([`Form::shove`]), or `None` for any other press.
pub fn shove_dir(sym: u32, ctrl: bool, shift: bool, alt: bool) -> Option<arrange::Dir> {
    use crate::textedit::keysym;
    if !ctrl || shift || alt {
        return None;
    }
    Some(match sym {
        keysym::LEFT | keysym::KP_LEFT => arrange::Dir::Left,
        keysym::RIGHT | keysym::KP_RIGHT => arrange::Dir::Right,
        keysym::UP | keysym::KP_UP => arrange::Dir::Up,
        keysym::DOWN | keysym::KP_DOWN => arrange::Dir::Down,
        _ => return None,
    })
}

/// What `key` stands for with Ctrl held, for a host reading a form:
/// Ctrl+Enter is [`Key::NewLine`] and Ctrl+Home / Ctrl+End are
/// [`Key::DocEdge`], which `textedit::key_for` folds into Home and End.
pub fn ctrl_key(key: Key) -> Key {
    match key {
        Key::Enter => Key::NewLine,
        Key::Edit(EditKey::Home { select }) => Key::DocEdge { end: false, select },
        Key::Edit(EditKey::End { select }) => Key::DocEdge { end: true, select },
        k => k,
    }
}

/// The flat dialog key an editing key stands for when it is not going into an edit field,
/// or `None`. Public so a host with a control of its own beside a form degrades keys the same way.
pub fn flat_key(k: &EditKey) -> Option<Key> {
    Some(match k {
        EditKey::Left { .. } => Key::Left,
        EditKey::Right { .. } => Key::Right,
        EditKey::Home { .. } => Key::Home,
        EditKey::End { .. } => Key::End,
        EditKey::Backspace { .. } => Key::Backspace,
        // Space works the focused control; as Key::Char(' ') it would be taken for an access key.
        EditKey::Insert(s) if s == " " => Key::Space,
        // A one-character insert is an access key or a combo's
        // type-ahead; anything longer (a paste) is not.
        EditKey::Insert(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => return None,
            }
        }
        EditKey::Delete { .. }
        | EditKey::SelectAll
        | EditKey::Copy
        | EditKey::Cut
        | EditKey::Paste
        | EditKey::Undo
        | EditKey::DeleteSurrounding { .. } => return None,
    })
}

impl Form {
    fn track_keyed(&mut self, i: usize, moved: bool) -> Event {
        if moved {
            self.rehit_thumb(i);
        }
        self.changed(i, moved)
    }

    /// The window lost the keyboard: a focused hot-key field forgets the
    /// modifiers it saw go down, whose releases go elsewhere
    /// ([`HotKey::blur`](crate::controls::hotkey::HotKey::blur)). `true` when one was focused, to be redrawn.
    pub fn blur_hotkey(&mut self) -> bool {
        match self.focus.and_then(|i| self.items.get_mut(i)) {
            Some(Item {
                control: Control::HotKey(h),
                ..
            }) => {
                h.blur();
                true
            }
            _ => false,
        }
    }

    /// A raw key for a focused hot-key field, pressed (`down`) or released. Hosts call this
    /// before reading the key as a dialog key, and read it that way only on `None`. `name` is
    /// xkb's lower-case keysym name, `held` the seat's modifiers. See [`HotKey::key`](crate::controls::hotkey::HotKey::key).
    pub fn hotkey_key(
        &mut self,
        sym: u32,
        name: &str,
        held: hotkey::Mods,
        down: bool,
    ) -> Option<Event> {
        let i = self.focus.filter(|_| self.open_combo().is_none())?;
        let Control::HotKey(h) = &mut self.items[i].control else {
            return None;
        };
        match h.key(sym, name, held, down) {
            hotkey::Took::Pass => None,
            hotkey::Took::Redraw => Some(Event::Redraw),
            hotkey::Took::Changed => Some(Event::Changed(self.items[i].id)),
        }
    }

    /// Ctrl+arrow for a focused arrangement: its picked tile moves ([`arrange::Arrange::shove`]).
    /// `None` when something else has the focus, which then reads the press as usual.
    pub fn shove(&mut self, dir: arrange::Dir) -> Option<Event> {
        let i = self.focus.filter(|_| self.open_combo().is_none())?;
        let id = self.items[i].id;
        let Control::Arrange(a) = &mut self.items[i].control else {
            return None;
        };
        Some(if a.enabled && a.shove(dir) {
            Event::Changed(id)
        } else {
            Event::None
        })
    }

    pub fn key(&mut self, key: Key) -> Event {
        let key = match key {
            Key::NewLine => match self.focus.map(|i| &self.items[i].control) {
                Some(Control::Memo(_)) if self.open_combo().is_none() => {
                    Key::Edit(EditKey::Insert("\n".into()))
                }
                _ => Key::Enter,
            },
            Key::DocEdge { end, select } => {
                if self.open_combo().is_none()
                    && let Some(i) = self.focus
                    && let Control::Memo(m) = &mut self.items[i].control
                {
                    let to = if end {
                        multiedit::Pos {
                            line: usize::MAX,
                            col: usize::MAX,
                        }
                    } else {
                        multiedit::Pos::default()
                    };
                    return if m.edit.place(to, select) {
                        Event::Redraw
                    } else {
                        Event::None
                    };
                }
                Key::Edit(if end {
                    EditKey::End { select }
                } else {
                    EditKey::Home { select }
                })
            }
            k => k,
        };
        // An editing key goes to a focused edit field or spinner that takes it; elsewhere it
        // degrades to its flat key. Above the popup guard, since an open drop-list takes these keys too.
        if let Key::Edit(k) = &key {
            let open = self.open_combo().is_some();
            if !open
                && let Some(i) = self.focus
                && let Control::UpDown(u) = &mut self.items[i].control
                && let Some(ch) = u.key(k)
            {
                return self.changed(i, ch);
            }
            let to_field = !open
                && self.focus.is_some_and(|i| {
                    matches!(
                        self.items[i].control,
                        Control::Edit { .. } | Control::Memo(_)
                    )
                });
            if !to_field {
                return match flat_key(k) {
                    Some(flat) => self.key(flat),
                    None => Event::None,
                };
            }
        }
        if let Some(i) = self.open_combo() {
            let id = self.items[i].id;
            return match &mut self.items[i].control {
                Control::Combo(c) => match key {
                    Key::Up => {
                        c.step(false);
                        Event::Redraw
                    }
                    Key::Down => {
                        c.step(true);
                        Event::Redraw
                    }
                    Key::Home => {
                        c.jump(false);
                        Event::Redraw
                    }
                    Key::End => {
                        c.jump(true);
                        Event::Redraw
                    }
                    Key::PageUp => {
                        c.scroll(-(c.rows as i32));
                        Event::Redraw
                    }
                    Key::PageDown => {
                        c.scroll(c.rows as i32);
                        Event::Redraw
                    }
                    Key::Enter => {
                        if c.take() {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    Key::Escape | Key::Drop | Key::Tab { .. } => {
                        c.open = None;
                        Event::Redraw
                    }
                    Key::Char(ch) => {
                        let names = c.items.iter().map(|it| it.text.as_str());
                        if let Some(k) = crate::selection::type_ahead(names, c.open, ch) {
                            c.open = Some(k);
                            c.top = k.saturating_sub(c.rows - 1).min(k);
                        }
                        Event::Redraw
                    }
                    _ => Event::None,
                },
                Control::Color(b) => match key {
                    Key::Escape | Key::Enter | Key::Space | Key::Tab { .. } => {
                        b.open = false;
                        Event::Redraw
                    }
                    _ => Event::None,
                },
                _ => Event::None,
            };
        }
        match key {
            Key::Tab { back } => self.move_focus(back),
            Key::Escape => Event::Cancel,
            Key::Enter => match self.focus {
                Some(i) if matches!(self.items[i].control, Control::Button { .. }) => {
                    Event::Pressed(self.items[i].id)
                }
                _ => Event::Default,
            },
            Key::Char(ch) => self.char_key(ch),
            Key::Access(ch) => self.access_key(ch),
            Key::Edit(k) => {
                // The guard at the top of `key` sends editing keys only to a
                // focused Control::Edit.
                let i = self
                    .focus
                    .expect("an editing key only reaches a focused control");
                let id = self.items[i].id;
                let changed = match &mut self.items[i].control {
                    Control::Edit { edit, .. } => edit.key(k),
                    Control::Memo(m) => m.edit.edit_key(k),
                    _ => unreachable!("an editing key only reaches a Control::Edit or Memo"),
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            other => {
                let Some(i) = self.focus else {
                    return Event::None;
                };
                let (id, grows) = (self.items[i].id, self.items[i].grows);
                let rect = self.at(&self.items[i]);
                match (&mut self.items[i].control, other) {
                    (Control::Check { checked, mixed, .. }, Key::Space) => {
                        toggle_check(checked, mixed);
                        Event::Changed(id)
                    }
                    (Control::Memo(m), Key::Space) => {
                        if m.edit.edit_key(EditKey::Insert(" ".into())) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    (
                        Control::IconList(l),
                        key @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End),
                    ) => {
                        let step = match key {
                            Key::Up => iconlist::Step::Up,
                            Key::Down => iconlist::Step::Down,
                            Key::Left => iconlist::Step::Left,
                            Key::Right => iconlist::Step::Right,
                            Key::Home => iconlist::Step::Home,
                            _ => iconlist::Step::End,
                        };
                        let moved = l.step(step);
                        l.follow(&icon_list_layout(
                            grows,
                            rect,
                            &self.rendered.controls.get(),
                        ));
                        if moved {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (
                        Control::Arrange(a),
                        key @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End),
                    ) => {
                        let step = match key {
                            Key::Up | Key::Left => arrange::Step::Back,
                            Key::Down | Key::Right => arrange::Step::Forward,
                            Key::Home => arrange::Step::First,
                            _ => arrange::Step::Last,
                        };
                        if a.step(step) {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (Control::Memo(m), key @ (Key::Up | Key::Down)) => {
                        if m.edit.step_lines(key == Key::Down, 1) {
                            Event::Redraw
                        } else {
                            Event::None
                        }
                    }
                    (Control::Memo(m), key @ (Key::PageUp | Key::PageDown)) => {
                        let rows = m.rows.get().max(1);
                        if m.edit.step_lines(key == Key::PageDown, rows) {
                            Event::Redraw
                        } else {
                            Event::None
                        }
                    }
                    (Control::Button { .. }, Key::Space) => Event::Pressed(id),
                    (Control::Button { .. }, Key::Left | Key::Up) => self.next_button(i, true),
                    (Control::Button { .. }, Key::Right | Key::Down) => self.next_button(i, false),
                    (Control::ListBox { list, .. }, Key::Space) => {
                        let flipped = list.selected.is_some_and(|r| list.toggle(r));
                        if flipped {
                            Event::Changed(id)
                        } else {
                            Event::None
                        }
                    }
                    (Control::Radio { .. }, Key::Space) => {
                        let ch = self.pick_radio(i);
                        self.changed(i, ch)
                    }
                    (Control::Radio { .. }, Key::Up | Key::Left) => {
                        match self.next_radio(i, true) {
                            Some(j) => {
                                self.set_focus(Some(j));
                                let ch = self.pick_radio(j);
                                self.changed(j, ch)
                            }
                            None => Event::None,
                        }
                    }
                    (Control::Radio { .. }, Key::Down | Key::Right) => {
                        match self.next_radio(i, false) {
                            Some(j) => {
                                self.set_focus(Some(j));
                                let ch = self.pick_radio(j);
                                self.changed(j, ch)
                            }
                            None => Event::None,
                        }
                    }
                    (Control::Color(b), Key::Space | Key::Drop) => {
                        b.open = true;
                        b.at = None;
                        Event::Redraw
                    }
                    (Control::Combo(c), Key::Drop) => {
                        c.toggle();
                        Event::Redraw
                    }
                    (Control::Combo(c), Key::Up | Key::Left) => {
                        let ch = c.step(false);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::Down | Key::Right) => {
                        let ch = c.step(true);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::Home | Key::PageUp) => {
                        let ch = c.jump(false);
                        self.changed(i, ch)
                    }
                    (Control::Combo(c), Key::End | Key::PageDown) => {
                        let ch = c.jump(true);
                        self.changed(i, ch)
                    }
                    (Control::ListBox { list, .. }, k) => {
                        let visible =
                            listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                        let delta = match k {
                            Key::Up => -1,
                            Key::Down => 1,
                            Key::PageUp => -(visible as i64),
                            Key::PageDown => visible as i64,
                            Key::Home => i64::MIN / 2,
                            Key::End => i64::MAX / 2,
                            _ => return Event::None,
                        };
                        let ch = list.step(delta, visible);
                        self.changed(i, ch)
                    }
                    (Control::Tree(tv), k) => {
                        let lay =
                            treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                        let ch = match k {
                            Key::Up => tv.step(-1, &lay),
                            Key::Down => tv.step(1, &lay),
                            Key::PageUp => tv.step(-(lay.rows as i64), &lay),
                            Key::PageDown => tv.step(lay.rows as i64, &lay),
                            Key::Home => tv.step(i64::MIN / 2, &lay),
                            Key::End => tv.step(i64::MAX / 2, &lay),
                            Key::Right => {
                                tv.expand_selected(&lay);
                                return Event::Redraw;
                            }
                            Key::Left => {
                                let ch = tv.collapse_selected(&lay);
                                return self.changed(i, ch);
                            }
                            _ => return Event::None,
                        };
                        self.changed(i, ch)
                    }
                    (Control::Calendar(cal), k) => {
                        let delta = match k {
                            Key::Left => -1,
                            Key::Right => 1,
                            Key::Up => -7,
                            Key::Down => 7,
                            _ => return Event::None,
                        };
                        let ch = cal.step_day(delta);
                        self.changed(i, ch)
                    }
                    (Control::Time(tf), k) => match k {
                        Key::Up | Key::Down => {
                            let ch = tf.step(if k == Key::Up { 1 } else { -1 });
                            self.changed(i, ch)
                        }
                        Key::Left | Key::Right => {
                            tf.move_part(if k == Key::Left { -1 } else { 1 });
                            Event::Redraw
                        }
                        _ => Event::None,
                    },
                    (Control::UpDown(u), Key::Up) => {
                        let ch = u.step(1);
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), Key::Down) => {
                        let ch = u.step(-1);
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), Key::Backspace) => {
                        let ch = u.backspace();
                        self.changed(i, ch)
                    }
                    (Control::UpDown(u), k @ (Key::Left | Key::Right | Key::Home | Key::End)) => {
                        let k = match k {
                            Key::Left => EditKey::Left {
                                select: false,
                                word: false,
                            },
                            Key::Right => EditKey::Right {
                                select: false,
                                word: false,
                            },
                            Key::Home => EditKey::Home { select: false },
                            _ => EditKey::End { select: false },
                        };
                        u.key(&k);
                        Event::Redraw
                    }
                    (Control::TrackBar(t), Key::Left | Key::Down) => {
                        let ch = t.step(-1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::Right | Key::Up) => {
                        let ch = t.step(1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::Home) => {
                        let ch = t.set(0);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::End) => {
                        let ch = t.set(t.count - 1);
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::PageUp) => {
                        let ch = t.step(-(t.page as i32));
                        self.track_keyed(i, ch)
                    }
                    (Control::TrackBar(t), Key::PageDown) => {
                        let ch = t.step(t.page as i32);
                        self.track_keyed(i, ch)
                    }
                    // A space in a field is text; it only presses or
                    // toggles the controls that have nothing to type in.
                    (Control::Edit { edit, .. }, Key::Space) => {
                        let changed = edit.key(EditKey::Insert(" ".into()));
                        if changed {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    _ => Event::None,
                }
            }
        }
    }

    /// A typed character: a digit into a spinner, a letter into a combo or list box
    /// (type-ahead), else an access key.
    fn char_key(&mut self, ch: char) -> Event {
        if let Some(i) = self.focus {
            let id = self.items[i].id;
            let rect = self.at(&self.items[i]);
            match &mut self.items[i].control {
                Control::Memo(m) => {
                    let changed = m.edit.edit_key(EditKey::Insert(ch.to_string()));
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Edit { edit, .. } => {
                    let changed = edit.key(EditKey::Insert(ch.to_string()));
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Time(tf) if tf.enabled && ch.is_ascii_alphanumeric() => {
                    let changed = tf.char(ch);
                    return self.changed(i, changed);
                }
                Control::UpDown(u) if ch.is_ascii_digit() => {
                    let changed = u.digit(ch);
                    return if changed {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    };
                }
                Control::Combo(c) if ch.is_alphanumeric() => {
                    let names = c.items.iter().map(|it| it.text.as_str());
                    if let Some(k) = crate::selection::type_ahead(names, Some(c.selected), ch) {
                        let changed = c.pick(k);
                        return if changed {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        };
                    }
                }
                Control::ListBox { list, .. } if ch.is_alphanumeric() => {
                    let visible =
                        listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                    if list.jump(ch, visible) {
                        return Event::Changed(id);
                    }
                }
                _ => {}
            }
        }
        self.access_key(ch)
    }

    fn access_key(&mut self, ch: char) -> Event {
        let key = ch.to_lowercase().next().unwrap_or(ch);
        let Some(i) = self.items.iter().position(|it| it.mnemonic() == Some(key)) else {
            return Event::None;
        };
        let id = self.items[i].id;
        match &mut self.items[i].control {
            Control::Check { checked, mixed, .. } => {
                toggle_check(checked, mixed);
                self.set_focus(Some(i));
                Event::Changed(id)
            }
            Control::Radio { .. } => {
                self.set_focus(Some(i));
                let ch = self.pick_radio(i);
                if ch {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            Control::Button { .. } => {
                self.set_focus(Some(i));
                Event::Pressed(id)
            }
            Control::Label { .. } => self.focus_after(i),
            _ => Event::None,
        }
    }
}
