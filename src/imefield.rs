//! Reprise's own edit fields (Run box, launcher, Start search, desktop rename,
//! dialog fields, balloon reply) as the input method's client: Reprise speaks
//! input-method-v2 itself for the field with the keyboard. The lock screen's
//! password box never takes part.
//!
//! - Hosts hand each draw the preedit (`ime_field_preedit`) and the focused
//!   edit's spot (`ime_field_drawn`); `ime_field_sync` runs once per loop turn.
//! - Input method requests pass through [`ImTap`]; its commit serial is put
//!   back by the `done`s Reprise sent.
//! - The field's keys go to the input method's grab first
//!   (`ime_takes_field_keys`); keys it sends back come via `ime_field_key`.

use std::sync::Arc;

use reprisekit::imeclient::{ImeClient, Request};
use reprisekit::textedit::{EditKey, Preedit, TextSpot};
use smithay::{
    backend::input::Keycode,
    reexports::{
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
            ChangeCause, ContentHint, ContentPurpose,
        },
        wayland_protocols_misc::zwp_input_method_v2::server::{
            zwp_input_method_manager_v2::{self, ZwpInputMethodManagerV2},
            zwp_input_method_v2::{self, ZwpInputMethodV2},
            zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource,
            backend::{
                ClientId, Handle, ObjectData, ObjectId,
                protocol::{Argument, Message},
            },
            protocol::wl_surface::WlSurface,
        },
    },
    wayland::{GlobalData, GlobalDispatch2, input_method::InputMethodSeat},
};

use crate::{
    caret::Editor, compositor::State, dialogview::DialogKey, keyroute::KeyRoute, raster::Rect,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImEvent {
    Activate,
    Deactivate,
    Surrounding {
        text: String,
        cursor: u32,
        anchor: u32,
    },
    ChangedElsewhere,
    ContentType {
        secret: bool,
        multiline: bool,
    },
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawnField {
    pub editor: Editor,
    pub spot: TextSpot,
    pub origin: (i32, i32),
}

#[derive(Default)]
pub struct FieldIme {
    client: ImeClient,
    editor: Option<Editor>,
    caret: Option<Rect>,
    origin: (i32, i32),
    dones: u32,
}

impl FieldIme {
    pub fn sync(&mut self, want: Option<DrawnField>) -> Vec<ImEvent> {
        let mut out = Vec::new();
        let editor = want.as_ref().map(|w| w.editor);
        if self.active() && editor != self.editor {
            let left = self.client.sync(None);
            self.translate(left, &mut out);
        }
        self.editor = editor;
        if let Some(w) = &want {
            self.origin = w.origin;
        }
        let reqs = self.client.sync(want.map(|w| w.spot));
        self.translate(reqs, &mut out);
        out
    }

    fn translate(&mut self, reqs: Vec<Request>, out: &mut Vec<ImEvent>) {
        for r in reqs {
            match r {
                Request::Enable => out.push(ImEvent::Activate),
                Request::Disable => {
                    self.caret = None;
                    out.push(ImEvent::Deactivate);
                }
                Request::Surrounding(s) => out.push(ImEvent::Surrounding {
                    text: s.text,
                    cursor: s.cursor as u32,
                    anchor: s.anchor as u32,
                }),
                Request::CausedElsewhere => out.push(ImEvent::ChangedElsewhere),
                Request::ContentType { secret, multiline } => {
                    out.push(ImEvent::ContentType { secret, multiline })
                }
                Request::Cursor(r) => self.caret = Some(r),
                Request::Commit => {
                    self.dones = self.dones.wrapping_add(1);
                    out.push(ImEvent::Done);
                }
            }
        }
    }

    pub fn active(&self) -> bool {
        self.client.sent().is_some()
    }

    pub fn caret(&self) -> Option<Rect> {
        let (x, y) = self.origin;
        self.caret
            .filter(|_| self.active())
            .map(|c| Rect::new(c.x + x, c.y + y, c.width, c.height))
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.client.preedit.as_ref()
    }

    pub fn forget(&mut self) -> bool {
        self.caret = None;
        self.client.reset()
    }

    pub fn new_input_method(&mut self) -> bool {
        self.dones = 0;
        self.forget()
    }

    pub fn smithay_serial(&self, serial: u32) -> u32 {
        serial.wrapping_sub(self.dones)
    }

    pub fn preedit_string(&mut self, text: Option<String>, begin: i32, end: i32) {
        self.client.preedit_string(text, begin, end);
    }

    pub fn commit_string(&mut self, text: Option<String>) {
        self.client.commit_string(text);
    }

    pub fn delete_surrounding(&mut self, before: u32, after: u32) {
        self.client.delete_surrounding(before, after);
    }

    pub fn done(&mut self) -> Vec<EditKey> {
        self.client.done()
    }
}

pub fn field_want(
    bound: bool,
    locked: bool,
    client_focus: bool,
    editor: Option<Editor>,
    drawn: Option<&DrawnField>,
) -> Option<DrawnField> {
    if !bound || locked || client_focus {
        return None;
    }
    let editor = editor?;
    drawn.filter(|d| d.editor == editor).cloned()
}

pub fn move_drawn(drawn: Option<&mut DrawnField>, editor: Editor, origin: (i32, i32)) {
    if let Some(d) = drawn.filter(|d| d.editor == editor) {
        d.origin = origin;
    }
}

pub fn spot_logical(mut spot: TextSpot, scale: i32) -> TextSpot {
    let s = scale.max(1);
    let down = |r: Rect| {
        let (x0, y0) = (r.x.div_euclid(s), r.y.div_euclid(s));
        let (x1, y1) = (
            (r.x + r.width + s - 1).div_euclid(s),
            (r.y + r.height + s - 1).div_euclid(s),
        );
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    };
    spot.field = down(spot.field);
    spot.caret = down(spot.caret);
    spot
}

pub fn field_route(route: &KeyRoute) -> bool {
    matches!(
        route,
        KeyRoute::Launcher
            | KeyRoute::StartPanel
            | KeyRoute::Dialog
            | KeyRoute::ClipView
            | KeyRoute::EmojiView
            | KeyRoute::Rename
            | KeyRoute::NotifyReply
    )
}

/// The input method manager global, Smithay's with each manager wrapped in
/// an [`ImMaker`].
pub struct ImGlobal {
    pub filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

impl GlobalDispatch2<ZwpInputMethodManagerV2, State> for ImGlobal {
    fn bind(
        &self,
        _state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwpInputMethodManagerV2>,
        data_init: &mut DataInit<'_, State>,
    ) {
        // Smithay's manager data is made only by wayland-server: a
        // stand-in made with it lends its data and goes.
        let Ok(stand_in) =
            client.create_resource::<ZwpInputMethodManagerV2, _, State>(dh, 1, GlobalData)
        else {
            data_init.init(resource, GlobalData);
            return;
        };
        let handle = dh.backend_handle();
        let inner = handle.get_object_data::<State>(stand_in.id());
        let _ = handle.destroy_object::<State>(&stand_in.id());
        match inner {
            Ok(inner) => data_init.custom_init(resource, Arc::new(ImMaker { inner })),
            Err(_) => data_init.init(resource, GlobalData),
        };
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

/// A manager's data, wrapped to tap the input method Smithay makes the
/// seat's own: Smithay keeps a second one inert, and only the request that
/// made the seat's one tells them apart.
struct ImMaker {
    inner: Arc<dyn ObjectData<State>>,
}

impl ObjectData<State> for ImMaker {
    fn request(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        msg: Message<ObjectId, std::os::fd::OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<State>>> {
        let made = match (msg.opcode, msg.args.get(1)) {
            (
                zwp_input_method_manager_v2::REQ_GET_INPUT_METHOD_OPCODE,
                Some(Argument::NewId(id)),
            ) => Some(id.clone()),
            _ => None,
        };
        let had = data.seat.input_method().has_instance();
        let inner = self.inner.clone();
        let ret = crate::textinput::forward(self, inner, handle, data, client_id, msg)?;
        let Some(id) = made.filter(|_| !had && data.seat.input_method().has_instance()) else {
            return Some(ret);
        };
        if let Ok(im) = ZwpInputMethodV2::from_id(&data.dh, id) {
            data.ime.im = Some(im);
            data.ime.field.new_input_method();
        }
        Some(Arc::new(ImTap { inner: ret }))
    }

    fn destroyed(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        object_id: ObjectId,
    ) {
        self.inner
            .clone()
            .destroyed(handle, data, client_id, object_id);
    }
}

/// The seat's input method object's data: its sends for Reprise's field go
/// here, and its commit serial reaches Smithay without Reprise's own `done`s.
struct ImTap {
    inner: Arc<dyn ObjectData<State>>,
}

impl ObjectData<State> for ImTap {
    fn request(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        mut msg: Message<ObjectId, std::os::fd::OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<State>>> {
        let opcode = msg.opcode;
        let serving = data.ime.field.active();
        let text = |a: Option<&Argument<ObjectId, std::os::fd::OwnedFd>>| match a {
            Some(Argument::Str(Some(s))) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        };
        let int = |a: Option<&Argument<ObjectId, std::os::fd::OwnedFd>>| match a {
            Some(Argument::Int(i)) => *i,
            Some(Argument::Uint(u)) => *u as i32,
            _ => 0,
        };
        let mut committed = false;
        let mut popup = None;
        match opcode {
            zwp_input_method_v2::REQ_COMMIT_STRING_OPCODE if serving => {
                data.ime.field.commit_string(text(msg.args.first()));
            }
            zwp_input_method_v2::REQ_SET_PREEDIT_STRING_OPCODE if serving => {
                data.ime.field.preedit_string(
                    text(msg.args.first()),
                    int(msg.args.get(1)),
                    int(msg.args.get(2)),
                );
            }
            zwp_input_method_v2::REQ_DELETE_SURROUNDING_TEXT_OPCODE if serving => {
                let (before, after) = (int(msg.args.first()), int(msg.args.get(1)));
                data.ime
                    .field
                    .delete_surrounding(before as u32, after as u32);
            }
            zwp_input_method_v2::REQ_COMMIT_OPCODE => {
                committed = serving;
                if let Some(Argument::Uint(serial)) = msg.args.first_mut() {
                    *serial = data.ime.field.smithay_serial(*serial);
                }
            }
            zwp_input_method_v2::REQ_GET_INPUT_POPUP_SURFACE_OPCODE => {
                if let (Some(Argument::NewId(id)), Some(Argument::Object(surface))) =
                    (msg.args.first(), msg.args.get(1))
                {
                    popup = Some((id.clone(), surface.clone()));
                }
            }
            _ => {}
        }
        let inner = self.inner.clone();
        let ret = crate::textinput::forward(self, inner, handle, data, client_id, msg);
        if let Some((id, surface)) = popup {
            data.ime_field_popup(id, surface);
        }
        if committed {
            data.ime_field_commit();
        }
        ret
    }

    fn destroyed(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        object_id: ObjectId,
    ) {
        if data.ime.im.as_ref().is_some_and(|im| im.id() == object_id) {
            data.ime.im = None;
            if data.ime.field.new_input_method()
                && let Some(e) = data.editor()
            {
                data.redraw_editor(e);
            }
        }
        self.inner
            .clone()
            .destroyed(handle, data, client_id, object_id);
    }
}

impl State {
    pub fn ime_field_preedit(&self, editor: Editor) -> Option<Preedit> {
        (self.ime.field.active() && self.editor() == Some(editor))
            .then(|| self.ime.field.preedit().cloned())
            .flatten()
    }

    pub fn ime_field_drawn(
        &mut self,
        editor: Editor,
        spot: Option<TextSpot>,
        origin: (i32, i32),
        scale: i32,
    ) {
        match spot {
            Some(spot) => {
                let mut spot = spot_logical(spot, scale);
                if editor == Editor::Rename {
                    spot.field = Rect::default();
                }
                self.ime.drawn = Some(DrawnField {
                    editor,
                    spot,
                    origin,
                });
            }
            None => {
                if self.ime.drawn.as_ref().is_some_and(|d| d.editor == editor) {
                    self.ime.drawn = None;
                }
            }
        }
    }

    pub fn ime_field_moved(&mut self, editor: Editor, origin: (i32, i32)) {
        move_drawn(self.ime.drawn.as_mut(), editor, origin);
    }

    pub fn ime_field_sync(&mut self) {
        let im = self.ime.im.clone().filter(|im| im.is_alive());
        if im.is_none() && !self.ime.field.active() {
            return;
        }
        let client_focus = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.current_focus().is_some());
        let editor = self.editor();
        let want = field_want(
            im.is_some(),
            self.is_locked(),
            client_focus,
            editor,
            self.ime.drawn.as_ref(),
        );
        let caret = self.ime.field.caret();
        let events = self.ime.field.sync(want);
        if events.is_empty() && caret == self.ime.field.caret() {
            return;
        }
        if let Some(im) = &im {
            for e in events {
                send(im, e);
            }
        }
        if !self.ime.field.active()
            && self.ime.field.forget()
            && let Some(e) = editor
        {
            self.redraw_editor(e);
        }
        self.place_ime_popup();
    }

    pub fn ime_focus_changed(&mut self, focused: Option<&WlSurface>) {
        if focused.is_some() && self.ime.field.active() && self.ime.field.forget() {
            self.handle.insert_idle(|state| {
                if let Some(e) = state.editor() {
                    state.redraw_editor(e);
                }
            });
        }
    }

    pub fn ime_takes_field_keys(&self) -> bool {
        self.ime.field.active()
            && self.seat.input_method().keyboard_grabbed()
            && self
                .seat
                .get_keyboard()
                .is_some_and(|kb| crate::textinput::ime_holds_keyboard(&kb))
    }

    pub fn ime_field_key(&mut self, key: u32, pressed: bool) -> bool {
        if !self.ime_takes_field_keys() {
            return false;
        }
        let Some(editor) = self.editor() else {
            return false;
        };
        if !pressed {
            return true;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return true;
        };
        let code = Keycode::new(key + 8);
        let syms: Vec<u32> = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // SAFETY: the state is only read; no reference outlives the lock.
            let state = unsafe { xkb.state() };
            state.key_get_syms(code).iter().map(|s| s.raw()).collect()
        });
        let k = DialogKey {
            syms,
            mods: crate::keys::mask(&keyboard.modifier_state()),
            edit: crate::editkeys::from_keyboard(&keyboard, self, code),
        };
        self.restart_caret();
        match editor {
            Editor::Launcher => self.launcher_key(&k),
            Editor::StartPanel => self.start_panel_key(&k),
            Editor::Dialog => self.dialog_key(&k),
            Editor::ClipView => self.clipview_key(&k),
            Editor::EmojiView => self.emoji_key(&k),
            Editor::Rename => self.rename_key(code, &k.syms),
            Editor::NotifyReply => crate::notifyview::key(self, &k),
        }
        true
    }

    fn ime_field_commit(&mut self) {
        let keys = self.ime.field.done();
        let Some(editor) = self.editor() else {
            return;
        };
        for key in keys {
            match editor {
                Editor::Rename => self.rename_edit(key),
                _ => {
                    let k = DialogKey {
                        syms: Vec::new(),
                        mods: 0,
                        edit: Some(key),
                    };
                    match editor {
                        Editor::Launcher => self.launcher_key(&k),
                        Editor::StartPanel => self.start_panel_key(&k),
                        Editor::Dialog => self.dialog_key(&k),
                        Editor::ClipView => self.clipview_key(&k),
                        Editor::EmojiView => self.emoji_key(&k),
                        Editor::NotifyReply => crate::notifyview::key(self, &k),
                        Editor::Rename => {}
                    }
                }
            }
        }
        self.restart_caret();
        self.redraw_editor(editor);
    }

    fn ime_field_popup(&mut self, id: ObjectId, surface: ObjectId) {
        let (Ok(role), Ok(surface)) = (
            ZwpInputPopupSurfaceV2::from_id(&self.dh, id),
            WlSurface::from_id(&self.dh, surface),
        ) else {
            return;
        };
        self.ime_adopt_popup(role, surface);
    }

    pub fn redraw_editor(&mut self, editor: Editor) {
        match editor {
            Editor::Launcher => self.redraw_launcher(),
            Editor::StartPanel => self.redraw_start_panel(),
            Editor::Dialog => self.redraw_top_dialog(),
            Editor::Rename => self.relayout_desktop(),
            Editor::ClipView => self.redraw_clipview(),
            Editor::EmojiView => self.redraw_emoji(),
            Editor::NotifyReply => crate::notifyview::blink(self),
        }
    }
}

fn send(im: &ZwpInputMethodV2, e: ImEvent) {
    match e {
        ImEvent::Activate => im.activate(),
        ImEvent::Deactivate => im.deactivate(),
        ImEvent::Surrounding {
            text,
            cursor,
            anchor,
        } => im.surrounding_text(text, cursor, anchor),
        ImEvent::ChangedElsewhere => im.text_change_cause(ChangeCause::Other),
        ImEvent::ContentType { secret, multiline } => {
            let (hint, purpose) = if secret {
                (
                    ContentHint::SensitiveData | ContentHint::HiddenText,
                    ContentPurpose::Password,
                )
            } else if multiline {
                (ContentHint::Multiline, ContentPurpose::Normal)
            } else {
                (ContentHint::None, ContentPurpose::Normal)
            };
            im.content_type(hint, purpose);
        }
        ImEvent::Done => im.done(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::Surrounding;

    fn spot(x: i32, text: &str, cursor: usize) -> TextSpot {
        TextSpot {
            field: Rect::new(100, 50, 200, 15),
            caret: Rect::new(x, 51, 1, 13),
            surrounding: Some(Surrounding {
                text: text.into(),
                cursor,
                anchor: cursor,
            }),
            secret: false,
            multiline: false,
            can_undo: false,
            selected: false,
        }
    }

    fn field(x: i32, text: &str, cursor: usize) -> DrawnField {
        DrawnField {
            editor: Editor::Launcher,
            spot: spot(x, text, cursor),
            origin: (0, 0),
        }
    }

    #[test]
    fn a_field_activates_with_its_text_and_deactivates_when_it_goes() {
        let mut f = FieldIme::default();
        assert_eq!(
            f.sync(Some(field(104, "ab", 2))),
            [
                ImEvent::Activate,
                ImEvent::Surrounding {
                    text: "ab".into(),
                    cursor: 2,
                    anchor: 2
                },
                ImEvent::ContentType {
                    secret: false,
                    multiline: false
                },
                ImEvent::Done,
            ]
        );
        assert!(f.active());
        assert_eq!(f.caret(), Some(Rect::new(104, 51, 1, 13)));
        assert_eq!(f.sync(Some(field(104, "ab", 2))), [], "nothing changed");
        assert_eq!(f.sync(None), [ImEvent::Deactivate, ImEvent::Done]);
        assert!(!f.active());
        assert_eq!(f.caret(), None);
    }

    #[test]
    fn a_caret_move_alone_sends_a_done_and_moves_the_popup() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "ab", 2)));
        assert_eq!(f.sync(Some(field(110, "ab", 2))), [ImEvent::Done]);
        assert_eq!(f.caret().map(|c| c.x), Some(110));
    }

    #[test]
    fn a_panel_that_moves_keeps_its_field_and_takes_the_caret_along() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "ab", 2)));
        let moved = DrawnField {
            origin: (500, 300),
            ..field(104, "ab", 2)
        };
        assert_eq!(f.sync(Some(moved)), [], "still the same field");
        assert_eq!(f.caret(), Some(Rect::new(604, 351, 1, 13)));
    }

    #[test]
    fn a_field_moved_without_a_redraw_takes_the_caret_along() {
        let mut f = FieldIme::default();
        let mut drawn = Some(field(104, "ab", 2));
        f.sync(drawn.clone());
        move_drawn(drawn.as_mut(), Editor::Dialog, (9, 9));
        assert_eq!(drawn.as_ref().map(|d| d.origin), Some((0, 0)));
        move_drawn(drawn.as_mut(), Editor::Launcher, (500, 300));
        assert_eq!(f.sync(drawn), [], "still the same field");
        assert_eq!(f.caret(), Some(Rect::new(604, 351, 1, 13)));
    }

    #[test]
    fn another_field_is_left_before_it_is_served() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "ab", 2)));
        let run = DrawnField {
            editor: Editor::Dialog,
            ..field(104, "ab", 2)
        };
        let events = f.sync(Some(run));
        assert_eq!(
            events[..3],
            [ImEvent::Deactivate, ImEvent::Done, ImEvent::Activate],
            "{events:?}"
        );
        assert_eq!(events.last(), Some(&ImEvent::Done));
    }

    #[test]
    fn typing_is_changed_elsewhere_and_a_commit_is_the_input_method_s() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "", 0)));
        let typed = f.sync(Some(field(110, "a", 1)));
        assert!(typed.contains(&ImEvent::ChangedElsewhere), "{typed:?}");
        f.preedit_string(Some("ni".into()), 2, 2);
        assert_eq!(f.done(), []);
        assert_eq!(f.preedit().map(|p| p.text.as_str()), Some("ni"));
        f.commit_string(Some("你".into()));
        f.preedit_string(None, 0, 0);
        assert_eq!(f.done(), [EditKey::Insert("你".into())]);
        assert_eq!(f.preedit(), None);
        let after = f.sync(Some(field(120, "a你", 4)));
        assert!(!after.contains(&ImEvent::ChangedElsewhere), "{after:?}");
    }

    #[test]
    fn the_commit_serial_reaches_smithay_without_the_field_s_dones() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "", 0)));
        f.sync(Some(field(110, "a", 1)));
        f.sync(None);
        assert_eq!(f.smithay_serial(6), 3);
        assert_eq!(f.smithay_serial(1), u32::MAX - 1, "wraps like a serial");
        f.new_input_method();
        assert_eq!(f.smithay_serial(6), 6, "a new input method saw none");
    }

    #[test]
    fn after_smithay_deactivates_the_field_activates_afresh() {
        let mut f = FieldIme::default();
        f.sync(Some(field(104, "", 0)));
        f.preedit_string(Some("ni".into()), 2, 2);
        f.done();
        assert!(f.forget(), "a preedit was showing");
        assert!(!f.active());
        assert_eq!(f.sync(Some(field(104, "", 0)))[0], ImEvent::Activate);
    }

    #[test]
    fn a_password_field_says_so_and_sends_no_text() {
        let mut f = FieldIme::default();
        let pw = DrawnField {
            spot: TextSpot {
                surrounding: None,
                secret: true,
                ..spot(104, "", 0)
            },
            ..field(104, "", 0)
        };
        assert_eq!(
            f.sync(Some(pw)),
            [
                ImEvent::Activate,
                ImEvent::ContentType {
                    secret: true,
                    multiline: false
                },
                ImEvent::Done
            ]
        );
    }

    #[test]
    fn only_the_field_with_the_keyboard_is_served_and_only_while_no_client_or_lock_has_it() {
        let drawn = field(104, "", 0);
        let want = |bound, locked, client, editor| {
            field_want(bound, locked, client, editor, Some(&drawn)).is_some()
        };
        assert!(want(true, false, false, Some(Editor::Launcher)));
        assert!(
            !want(false, false, false, Some(Editor::Launcher)),
            "no input method"
        );
        assert!(!want(true, true, false, Some(Editor::Launcher)), "locked");
        assert!(
            !want(true, false, true, Some(Editor::Launcher)),
            "a client has the keys"
        );
        assert!(
            !want(true, false, false, Some(Editor::Dialog)),
            "drawn for another field"
        );
        assert!(!want(true, false, false, None), "no field");
    }

    #[test]
    fn a_spot_drawn_at_scale_two_comes_down_to_logical_px() {
        let s = TextSpot {
            field: Rect::new(10, 20, 101, 30),
            caret: Rect::new(41, 22, 2, 26),
            ..spot(0, "", 0)
        };
        let l = spot_logical(s, 2);
        assert_eq!(l.field, Rect::new(5, 10, 51, 15));
        assert_eq!(l.caret, Rect::new(20, 11, 2, 13));
    }

    #[test]
    fn the_field_s_own_keys_go_to_the_input_method_and_binds_do_not() {
        assert!(field_route(&KeyRoute::Dialog));
        assert!(field_route(&KeyRoute::Rename));
        assert!(field_route(&KeyRoute::Launcher));
        assert!(field_route(&KeyRoute::NotifyReply));
        assert!(!field_route(&KeyRoute::Client));
        assert!(!field_route(&KeyRoute::Lock));
        assert!(!field_route(&KeyRoute::Swallow));
    }
}

#[cfg(test)]
mod harness_tests {
    use crate::harness::{Harness, ImHeard, TestClient};

    const KEY_A: u32 = 30;

    /// A harness with one output, no window, and an input method client.
    fn with_input_method() -> (Harness, TestClient) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut m = h.connect();
        m.input_method();
        h.roundtrip(&mut m);
        (h, m)
    }

    #[test]
    fn the_run_box_gets_the_input_method_back_after_a_client_without_the_keyboard_drops_its_text_input()
     {
        let (mut h, mut m) = with_input_method();
        h.state.toggle_launcher();
        h.roundtrip(&mut m);
        assert!(m.side.ims[0].active(), "the Run box activates it");
        let mut other = h.connect();
        let ti = other.text_input();
        h.roundtrip(&mut other);
        other.destroy_text_input(ti);
        h.roundtrip(&mut other);
        h.roundtrip(&mut m);
        let events = &m.side.ims[0].events;
        assert!(
            events.contains(&ImHeard::Deactivate),
            "the dropped text input deactivated it: {events:?}"
        );
        assert!(
            m.side.ims[0].active(),
            "the Run box activated it again: {events:?}"
        );
    }

    #[test]
    fn the_run_box_activates_the_seats_input_method_and_not_an_inert_one() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let (mut first, mut inert) = (h.connect(), h.connect());
        first.input_method();
        h.roundtrip(&mut first);
        inert.input_method();
        h.roundtrip(&mut inert);
        assert_eq!(inert.side.ims[0].events, [ImHeard::Unavailable]);
        first.destroy_input_method(0);
        h.roundtrip(&mut first);
        let mut seats = h.connect();
        seats.input_method();
        h.roundtrip(&mut seats);
        h.state.toggle_launcher();
        h.roundtrip(&mut seats);
        h.roundtrip(&mut inert);
        assert!(
            seats.side.ims[0].active(),
            "the seat's input method serves the Run box"
        );
        assert_eq!(
            inert.side.ims[0].events,
            [ImHeard::Unavailable],
            "the inert one hears nothing more"
        );
    }

    #[test]
    fn the_run_box_takes_the_seats_keys_through_the_input_methods_grab_and_its_echo_from_the_virtual_keyboard()
     {
        let (mut h, mut m) = with_input_method();
        h.state.toggle_launcher();
        m.grab_keyboard(0);
        let vk = m.virtual_keyboard();
        h.roundtrip(&mut m);
        assert!(h.state.ime_takes_field_keys());
        let query = |h: &Harness| h.state.launcher.as_ref().unwrap().model.query().to_string();
        h.key(KEY_A, true);
        h.key(KEY_A, false);
        h.roundtrip(&mut m);
        assert_eq!(
            m.side.ims[0].grab_keys,
            [(KEY_A, true), (KEY_A, false)],
            "the input method's grab hears the seat's key"
        );
        assert_eq!(query(&h), "", "and the field does not");
        m.vk_key(vk, KEY_A, true);
        m.vk_key(vk, KEY_A, false);
        h.roundtrip(&mut m);
        assert_eq!(query(&h), "a", "the key it sent back is typed");
        assert_eq!(m.side.ims[0].grab_keys.len(), 2, "and is not heard again");
    }
}
