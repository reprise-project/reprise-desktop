//! Input methods (fcitx5, ibus) through input-method-v2, relayed by Smithay to the text-input-v3 of the focused client,
//! and virtual-keyboard-v1. One input method at a time; Reprise draws the candidate popup and splits the keyboard grab (`SplitGrab`).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use smithay::{
    backend::{
        input::{Event, InputEvent, KeyState, KeyboardKeyEvent, Keycode},
        renderer::utils::with_renderer_surface_state,
    },
    desktop::{PopupKeyboardGrab, utils::bbox_from_surface_tree},
    input::{
        SeatHandler,
        keyboard::{
            GrabStartData as KeyboardGrabStartData, KeyboardGrab, KeyboardInnerHandle,
            KeyboardSource, ModifiersState, SerializedMods,
        },
    },
    reexports::{
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{self, ZwpTextInputV3},
        wayland_protocols_misc::{
            zwp_input_method_v2::server::{
                zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
                zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
                zwp_input_method_v2::ZwpInputMethodV2,
                zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
            },
            zwp_virtual_keyboard_v1::server::{
                zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
                zwp_virtual_keyboard_v1::{self, ZwpVirtualKeyboardV1},
            },
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource, ResourceData,
            backend::{
                ClientId, Handle, ObjectData, ObjectId,
                protocol::{Argument, Message},
            },
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Rectangle, SERIAL_COUNTER, Serial},
    wayland::{
        Dispatch2, GlobalData, GlobalDispatch2,
        compositor::add_destruction_hook,
        input_method::{
            InputMethodHandle, InputMethodHandler, InputMethodKeyboardGrab, InputMethodSeat,
            PopupSurface,
        },
        text_input::{TextInputActivation, TextInputManagerState, TextInputSeat},
        virtual_keyboard::{
            VirtualKeyboardBackend, VirtualKeyboardDevice, VirtualKeyboardHandler,
            VirtualKeyboardSpecialEvent,
        },
    },
};

use reprisescene::SurfaceId;

use crate::{compositor::State, raster::Rect};

pub struct Ime {
    popup: Option<ImePopup>,
    pub field: crate::imefield::FieldIme,
    pub drawn: Option<crate::imefield::DrawnField>,
    pub im: Option<ZwpInputMethodV2>,
    grab: Option<InputMethodKeyboardGrab>,
    grab_object: Option<ZwpInputMethodKeyboardGrabV2>,
    heard: HeardMods,
    cursors: HashMap<ObjectId, TiState>,
    pub vk: VkKeymap,
    vk_key: bool,
    _text_input: TextInputManagerState,
}

struct ImePopup {
    surface: WlSurface,
    role: ZwpInputPopupSurfaceV2,
    popup: Option<PopupSurface>,
    node: SurfaceId,
    sent: Option<Rect>,
}

impl Ime {
    pub fn new(dh: &DisplayHandle, seat: &smithay::input::Seat<State>) -> Ime {
        // Smithay tells a second input method it is unavailable and keeps it
        // inert, so no client sees the global while one is bound.
        let bound = seat.input_method().clone();
        dh.create_global::<State, ZwpInputMethodManagerV2, _>(
            1,
            crate::imefield::ImGlobal {
                filter: Box::new(move |client| {
                    !bound.has_instance()
                        && crate::sandbox::client_may_see(client, "zwp_input_method_manager_v2")
                }),
            },
        );
        dh.create_global::<State, ZwpVirtualKeyboardManagerV1, _>(
            1,
            VkGlobal {
                filter: Box::new(crate::sandbox::filter("zwp_virtual_keyboard_manager_v1")),
            },
        );
        Ime {
            popup: None,
            field: Default::default(),
            drawn: None,
            im: None,
            grab: None,
            grab_object: None,
            heard: HeardMods::default(),
            cursors: HashMap::new(),
            vk: VkKeymap::default(),
            vk_key: false,
            _text_input: TextInputManagerState::new::<State>(dh),
        }
    }
}

#[derive(Default)]
pub struct VkKeymap {
    active: Option<Arc<str>>,
    own: Option<(String, crate::keyboard::XkbMask)>,
    sources: HashMap<VirtualKeyboardDevice, KeyboardSource>,
    echoed: HashSet<(VirtualKeyboardDevice, Keycode)>,
}

impl VkKeymap {
    pub fn forget_keymap(&mut self) {
        self.active = None;
        self.own = None;
    }
}

impl TextInputActivation for State {
    /// With no keyboard focus Smithay deactivates the input method when any
    /// client's last text input goes, under Reprise's own field too: the
    /// field activates again on the next turn.
    fn deactivated(&mut self) {
        let unfocused = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.current_focus().is_none());
        if unfocused && self.ime.field.active() {
            self.ime.field.forget();
        }
    }
}

impl VirtualKeyboardHandler for State {
    /// A key is routed as the seat's keys are (`State::route_key`), under
    /// the device's keymap and past the input method's grab; modifiers go
    /// on the xkb state as the device set them. A key the seat already
    /// holds is the input method's echo of it and goes to the client
    /// alone: the seat would absorb a second press of it, and the input
    /// method's grab would then miss its release.
    fn process_virtual_keyboard_event(&mut self, event: InputEvent<VirtualKeyboardBackend>) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        match event {
            InputEvent::Keyboard { event } => {
                crate::idle::notify_activity(self);
                let device = event.device();
                let (code, key_state) = (event.key_code(), event.state());
                let (serial, time) = (SERIAL_COUNTER.next_serial(), Event::time(&event));
                let echo = (device.clone(), code);
                let echoed = if key_state == KeyState::Pressed {
                    keyboard.pressed_keys().contains(&code) && self.ime.vk.echoed.insert(echo)
                } else {
                    self.ime.vk.echoed.remove(&echo)
                };
                self.ime.vk_key = true;
                if echoed {
                    // The press of a modifier went to the grab alone, so
                    // the client gets the seat's modifiers with the key.
                    keyboard.input_forward(self, code, key_state, serial, time, true);
                } else {
                    self.vk_keymap(&device);
                    let source = *self
                        .ime
                        .vk
                        .sources
                        .entry(device)
                        .or_insert_with(KeyboardSource::new_auxiliary);
                    self.route_key(code, key_state == KeyState::Pressed, time, source);
                }
                self.ime.vk_key = false;
            }
            InputEvent::DeviceRemoved { device } => {
                if let Some(source) = self.ime.vk.sources.remove(&device) {
                    keyboard.release_source(self, source);
                }
                self.ime.vk.echoed.retain(|(d, _)| *d != device);
            }
            InputEvent::Special(VirtualKeyboardSpecialEvent::Modifiers {
                device,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            }) => {
                self.vk_keymap(&device);
                keyboard.with_xkb_state(self, |mut ctx| {
                    ctx.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group)
                });
            }
            _ => {}
        }
    }
}

impl State {
    fn vk_keymap(&mut self, device: &VirtualKeyboardDevice) {
        let Some(keymap) = device.keymap() else {
            self.vk_restore();
            return;
        };
        if self.ime.vk.active.as_deref() == Some(&*keymap) {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if self.ime.vk.own.is_none() {
            let (Some(own), Some((mask, _))) = (self.keymap_text(), self.xkb_mask()) else {
                return;
            };
            self.ime.vk.own = Some((own, mask));
        }
        if let Some((own, _)) = &self.ime.vk.own
            && own.trim_end_matches('\0') == keymap.trim_end_matches('\0')
        {
            self.vk_restore();
            return;
        }
        match keyboard.set_keymap_from_string(self, keymap.to_string()) {
            Ok(()) => self.ime.vk.active = Some(keymap),
            Err(e) => log::warn!("virtual keyboard: could not compile its keymap: {e:?}"),
        }
    }

    pub fn vk_restore(&mut self) {
        let Some((own, mask)) = self.ime.vk.own.take() else {
            return;
        };
        if self.ime.vk.active.take().is_none() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if let Err(e) = keyboard.set_keymap_from_string(self, own) {
            log::warn!("virtual keyboard: could not restore the keymap: {e:?}");
            return;
        }
        self.set_xkb_mask(mask);
    }
}

fn keys_shut(state: &State) -> bool {
    state.is_locked() || state.inputproto.gated()
}

fn vk_request_passes(opcode: u16, shut: bool) -> bool {
    !shut
        || !matches!(
            opcode,
            zwp_virtual_keyboard_v1::REQ_KEY_OPCODE | zwp_virtual_keyboard_v1::REQ_MODIFIERS_OPCODE
        )
}

pub struct VkGlobal {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

pub struct VkManager;

pub struct VkRefused;

impl GlobalDispatch2<ZwpVirtualKeyboardManagerV1, State> for VkGlobal {
    fn bind(
        &self,
        _state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        // Smithay's dispatch wants the manager's data as wayland-server
        // makes it, which only an object it inits holds: a stand-in made
        // for the client lends its data and goes.
        let Ok(stand_in) =
            client.create_resource::<ZwpVirtualKeyboardManagerV1, _, State>(dh, 1, VkManager)
        else {
            data_init.init(resource, VkManager);
            return;
        };
        let handle = dh.backend_handle();
        let inner = handle.get_object_data::<State>(stand_in.id());
        let _ = handle.destroy_object::<State>(&stand_in.id());
        match inner {
            Ok(inner) => data_init.custom_init(resource, Arc::new(VkMaker { inner })),
            Err(_) => data_init.init(resource, VkManager),
        };
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl Dispatch2<ZwpVirtualKeyboardManagerV1, State> for VkManager {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        resource: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { id, .. }
                if keys_shut(state) =>
            {
                log::debug!("virtual keyboard refused under the lock or a capture");
                data_init.init(id, VkRefused);
            }
            request => {
                Dispatch2::request(&GlobalData, state, client, resource, request, dh, data_init)
            }
        }
    }
}

impl Dispatch2<ZwpVirtualKeyboardV1, State> for VkRefused {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardV1,
        _request: zwp_virtual_keyboard_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

struct VkMaker {
    inner: Arc<dyn ObjectData<State>>,
}

impl ObjectData<State> for VkMaker {
    fn request(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        msg: Message<ObjectId, std::os::fd::OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<State>>> {
        let inner = self.inner.clone();
        let made = forward(self, inner, handle, data, client_id, msg)?;
        if made.is::<ResourceData<ZwpVirtualKeyboardV1, VkRefused>>() {
            return Some(made);
        }
        Some(Arc::new(VkGuard {
            inner: made,
            passes: vk_passes,
            divert: vk_divert,
        }))
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

/// A virtual keyboard Smithay made, wrapped so its keys are dropped while the keys are shut.
/// Smithay's dispatch reads the object's data back as its own
/// `ResourceData`, so a request that passes goes on with that data put
/// back on the object, and the guard returns after it.
struct VkGuard<D> {
    inner: Arc<dyn ObjectData<D>>,
    passes: fn(&D, u16) -> bool,
    divert: fn(&mut D, &Message<ObjectId, std::os::fd::OwnedFd>) -> bool,
}

impl<D: 'static> ObjectData<D> for VkGuard<D> {
    fn request(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut D,
        client_id: ClientId,
        msg: Message<ObjectId, std::os::fd::OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<D>>> {
        if !(self.passes)(data, msg.opcode) || (self.divert)(data, &msg) {
            return None;
        }
        let msg = if msg.opcode == zwp_virtual_keyboard_v1::REQ_KEYMAP_OPCODE {
            let id = msg.sender_id.clone();
            match copy_keymap(msg) {
                Some(msg) => msg,
                None => {
                    handle.post_error(
                        id,
                        zwp_virtual_keyboard_v1::Error::NoKeymap as u32,
                        c"the keymap cannot be read".into(),
                    );
                    return None;
                }
            }
        } else {
            msg
        };
        let inner = self.inner.clone();
        forward(self, inner, handle, data, client_id, msg)
    }

    fn destroyed(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut D,
        client_id: ClientId,
        object_id: ObjectId,
    ) {
        self.inner
            .clone()
            .destroyed(handle, data, client_id, object_id);
    }
}

const MAX_VK_KEYMAP: u32 = 4 << 20;

/// A keymap request (`format`, `fd`, `size`) with the file swapped for a
/// sealed memfd of the bytes it holds, `size` cut to match. Smithay maps
/// the client's file for the size the client gives: a size past the
/// file's end, or a file shrunk meanwhile, faulted the compositor. None
/// for a file that cannot be read without blocking or a size past
/// [`MAX_VK_KEYMAP`].
fn copy_keymap(
    mut msg: Message<ObjectId, std::os::fd::OwnedFd>,
) -> Option<Message<ObjectId, std::os::fd::OwnedFd>> {
    let [Argument::Uint(_), Argument::Fd(fd), Argument::Uint(size)] = &mut msg.args[..] else {
        return None;
    };
    if *size > MAX_VK_KEYMAP {
        return None;
    }
    let bytes = read_head(fd, *size as usize)?;
    *fd = crate::eis::sealed_memfd(&bytes)?;
    *size = bytes.len() as u32;
    Some(msg)
}

/// Up to `len` bytes from the start of `fd`, fewer when the file is
/// shorter. Positioned reads: a pipe or a socket fails at once rather than
/// block.
fn read_head(fd: &std::os::fd::OwnedFd, len: usize) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::from(fd.try_clone().ok()?);
    let mut buf = vec![0; len];
    let mut got = 0;
    while got < len {
        match file.read_at(&mut buf[got..], got as u64) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    buf.truncate(got);
    Some(buf)
}

pub(crate) fn forward<D: 'static>(
    wrapper: Arc<dyn ObjectData<D>>,
    inner: Arc<dyn ObjectData<D>>,
    handle: &Handle,
    data: &mut D,
    client_id: ClientId,
    msg: Message<ObjectId, std::os::fd::OwnedFd>,
) -> Option<Arc<dyn ObjectData<D>>> {
    let id = msg.sender_id.clone();
    let _ = handle.set_object_data::<D>(id.clone(), inner.clone());
    let ret = inner.request(handle, data, client_id, msg);
    let _ = handle.set_object_data::<D>(id, wrapper);
    ret
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct TiState {
    enabled: bool,
    cursor: Option<Rect>,
    pending_enable: Option<bool>,
    pending_cursor: Option<Rect>,
}

impl TiState {
    fn take(&mut self, opcode: u16, ints: &[i32]) {
        match opcode {
            zwp_text_input_v3::REQ_ENABLE_OPCODE => {
                self.pending_enable = Some(true);
                self.pending_cursor = None;
            }
            zwp_text_input_v3::REQ_DISABLE_OPCODE => self.pending_enable = Some(false),
            zwp_text_input_v3::REQ_SET_CURSOR_RECTANGLE_OPCODE => {
                if let [x, y, w, h] = *ints {
                    self.pending_cursor = Some(Rect::new(x, y, w, h));
                }
            }
            zwp_text_input_v3::REQ_COMMIT_OPCODE => {
                match self.pending_enable.take() {
                    Some(true) => {
                        self.enabled = true;
                        self.cursor = None;
                    }
                    Some(false) => {
                        self.enabled = false;
                        self.cursor = None;
                    }
                    None => {}
                }
                if let Some(c) = self.pending_cursor.take() {
                    self.cursor = Some(c);
                }
            }
            _ => {}
        }
    }
}

/// A text input Smithay made, wrapped so Reprise sees its enable state and
/// cursor rectangle: Smithay keeps them to itself and hands the rectangle
/// on only to an input method's popup, which an input method makes only
/// when it has candidates to show. The panels open at that caret.
struct TiPeek {
    inner: Arc<dyn ObjectData<State>>,
}

impl ObjectData<State> for TiPeek {
    fn request(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        msg: Message<ObjectId, std::os::fd::OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<State>>> {
        let ints: Vec<i32> = msg
            .args
            .iter()
            .filter_map(|a| match a {
                Argument::Int(i) => Some(*i),
                _ => None,
            })
            .collect();
        data.ime
            .cursors
            .entry(msg.sender_id.clone())
            .or_default()
            .take(msg.opcode, &ints);
        let inner = self.inner.clone();
        forward(self, inner, handle, data, client_id, msg)
    }

    fn destroyed(
        self: Arc<Self>,
        handle: &Handle,
        data: &mut State,
        client_id: ClientId,
        object_id: ObjectId,
    ) {
        data.ime.cursors.remove(&object_id);
        self.inner
            .clone()
            .destroyed(handle, data, client_id, object_id);
    }
}

pub fn peek_text_inputs(state: &State, client: &Client) {
    let handle = state.dh.backend_handle();
    let mut inputs = Vec::new();
    let _ = handle.with_all_objects_for(client.id(), |o| {
        if o.interface().name == ZwpTextInputV3::interface().name {
            inputs.push(o);
        }
    });
    for o in inputs {
        let Ok(inner) = handle.get_object_data::<State>(o.clone()) else {
            continue;
        };
        if inner.is::<TiPeek>() {
            continue;
        }
        let _ = handle.set_object_data::<State>(o, Arc::new(TiPeek { inner }));
    }
}

fn vk_passes(state: &State, opcode: u16) -> bool {
    vk_request_passes(opcode, keys_shut(state))
}

fn vk_divert(state: &mut State, msg: &Message<ObjectId, std::os::fd::OwnedFd>) -> bool {
    match (msg.opcode, &msg.args[..]) {
        (zwp_virtual_keyboard_v1::REQ_KEY_OPCODE, [_, Argument::Uint(key), Argument::Uint(st)]) => {
            state.ime_field_key(*key, *st == 1)
        }
        _ => false,
    }
}

#[derive(Default)]
pub struct KeySplit {
    grabbed: Vec<u32>,
}

impl KeySplit {
    pub fn goes_to_grab(&mut self, keycode: u32, pressed: bool, grab: bool) -> bool {
        if pressed {
            if grab && !self.grabbed.contains(&keycode) {
                self.grabbed.push(keycode);
            }
            return grab;
        }
        let held = self.grabbed.contains(&keycode);
        self.grabbed.retain(|&k| k != keycode);
        held && grab
    }
}

/// The modifiers the input method's keyboard grab last heard, so each state reaches it once.
#[derive(Default)]
struct HeardMods(Option<SerializedMods>);

impl HeardMods {
    fn heard(&mut self, mods: SerializedMods) {
        self.0 = Some(mods);
    }

    /// `now`, unless the grab already holds it.
    fn unheard(&mut self, now: SerializedMods) -> Option<SerializedMods> {
        (self.0 != Some(now)).then(|| {
            self.0 = Some(now);
            now
        })
    }
}

pub fn popup_origin(caret: Rect, size: (i32, i32), output: Rect) -> (i32, i32) {
    let (w, h) = size;
    let below = output.bottom() - caret.bottom();
    let above = caret.y - output.y;
    let y = if below < h && above > below {
        caret.y - h
    } else {
        caret.bottom()
    };
    let x = caret.x.min(output.right() - w).max(output.x);
    (x, y)
}

struct SplitGrab {
    im: InputMethodKeyboardGrab,
    handle: InputMethodHandle,
    split: KeySplit,
    start: KeyboardGrabStartData<State>,
}

impl KeyboardGrab<State> for SplitGrab {
    fn input(
        &mut self,
        data: &mut State,
        handle: &mut KeyboardInnerHandle<'_, State>,
        keycode: Keycode,
        key_state: KeyState,
        modifiers: Option<ModifiersState>,
        serial: Serial,
        time: smithay::backend::input::InputTime,
    ) {
        let grab = self.handle.keyboard_grabbed();
        let pressed = key_state == KeyState::Pressed;
        if !data.ime.vk_key && self.split.goes_to_grab(keycode.raw(), pressed, grab) {
            self.im
                .input(data, handle, keycode, key_state, modifiers, serial, time);
            if let Some(m) = modifiers {
                data.ime.heard.heard(m.serialized);
            }
        } else {
            handle.input(data, keycode, key_state, modifiers, serial, time);
        }
    }

    fn set_focus(
        &mut self,
        data: &mut State,
        handle: &mut KeyboardInnerHandle<'_, State>,
        focus: Option<<State as SeatHandler>::KeyboardFocus>,
        serial: Serial,
    ) {
        handle.set_focus(data, focus, serial);
    }

    fn start_data(&self) -> &KeyboardGrabStartData<State> {
        &self.start
    }

    fn unset(&mut self, data: &mut State) {
        KeyboardGrab::<State>::unset(&mut self.im, data);
    }
}

enum KeyboardHolder {
    Nothing,
    /// The input method's grab, as Smithay set it or wrapped in a `SplitGrab`.
    InputMethod(Option<InputMethodKeyboardGrab>),
    Other,
}

/// Whether open client popups take back the keyboard grab from `holder`.
fn popup_grab_returns(popups_open: bool, holder: &KeyboardHolder) -> bool {
    popups_open && !matches!(holder, KeyboardHolder::Other)
}

pub fn ime_holds_keyboard(kb: &smithay::input::keyboard::KeyboardHandle<State>) -> bool {
    kb.with_grab(|_, g| g.downcast_ref::<SplitGrab>().is_some())
        .unwrap_or(false)
}

impl State {
    pub fn ime_sync_grab(&mut self) {
        self.ime_return_popup_grab();
        let handle = self.seat.input_method().clone();
        if !handle.keyboard_grabbed() {
            self.ime.grab = None;
            self.ime.heard = HeardMods::default();
            return;
        }
        let Some(kb) = self.seat.get_keyboard() else {
            return;
        };
        let fresh = kb
            .with_grab(|_, g| g.downcast_ref::<InputMethodKeyboardGrab>().cloned())
            .flatten();
        // Smithay tells a new grab the modifiers as it starts; a grab coming back from a menu's
        // hears what changed while the menu held the keyboard.
        let regained = fresh.is_none();
        let im = match fresh {
            Some(im) => {
                self.ime.heard.heard(kb.modifier_state().serialized);
                im
            }
            None if !kb.is_grabbed() => match self.ime.grab.clone() {
                Some(im) => im,
                None => return,
            },
            None => return,
        };
        self.ime.grab = Some(im.clone());
        let grab = SplitGrab {
            im,
            handle,
            split: KeySplit::default(),
            start: KeyboardGrabStartData { focus: None },
        };
        kb.set_grab(self, grab, SERIAL_COUNTER.next_serial());
        if regained {
            self.ime_grab_modifiers();
        }
    }

    /// Smithay hands an input method's grab the keyboard over a client
    /// popup's grab, and the grab object's destruction unsets whatever grab
    /// holds the keyboard: an input method dropping and retaking its grab as
    /// a menu opens left the menu without its keyboard grab, and the keys
    /// after the menu closed went to the gone menu. While the pointer still
    /// holds the popups, their keyboard grab goes back, keeping the input
    /// method's grab for when they close.
    fn ime_return_popup_grab(&mut self) {
        let (Some(kb), Some(pointer)) = (self.seat.get_keyboard(), self.seat.get_pointer()) else {
            return;
        };
        let Some(popup) = crate::popupgrab::holding(&pointer) else {
            return;
        };
        let holder = kb
            .with_grab(|_, g| {
                if let Some(im) = g.downcast_ref::<InputMethodKeyboardGrab>() {
                    KeyboardHolder::InputMethod(Some(im.clone()))
                } else if g.downcast_ref::<SplitGrab>().is_some() {
                    KeyboardHolder::InputMethod(None)
                } else {
                    KeyboardHolder::Other
                }
            })
            .unwrap_or(KeyboardHolder::Nothing);
        if !popup_grab_returns(!popup.has_ended(), &holder) {
            return;
        }
        if let KeyboardHolder::InputMethod(Some(im)) = holder {
            self.ime.grab = Some(im);
        }
        kb.set_grab(self, PopupKeyboardGrab::new(&popup), popup.serial());
    }

    /// A key that changed the modifiers went past the input method's grab
    /// (one of Reprise's own keys, or a release whose press the grab did
    /// not see): the grab hears the new state. Smithay tells it the
    /// modifiers only with a key it gets, so the Super of the Super+R that
    /// opened the Run box stayed down for it, and it passed every key on
    /// untouched until the next modifier it saw. A state the grab already
    /// heard is not sent again.
    pub fn ime_grab_modifiers(&mut self) {
        if !self.seat.input_method().keyboard_grabbed() {
            return;
        }
        let Some(kb) = self.seat.get_keyboard() else {
            return;
        };
        if !self.ime.grab_object.as_ref().is_some_and(|g| g.is_alive()) {
            self.ime.grab_object = self.find_grab_object();
        }
        let Some(grab) = &self.ime.grab_object else {
            return;
        };
        let Some(m) = self.ime.heard.unheard(kb.modifier_state().serialized) else {
            return;
        };
        grab.modifiers(
            SERIAL_COUNTER.next_serial().into(),
            m.depressed,
            m.latched,
            m.locked,
            m.layout_effective,
        );
    }

    fn find_grab_object(&self) -> Option<ZwpInputMethodKeyboardGrabV2> {
        let client = self.ime.im.as_ref()?.client()?;
        let handle = self.dh.backend_handle();
        let mut found = None;
        let _ = handle.with_all_objects_for(client.id(), |o| {
            if o.interface().name == ZwpInputMethodKeyboardGrabV2::interface().name {
                found = Some(o);
            }
        });
        ZwpInputMethodKeyboardGrabV2::from_id(&self.dh, found?).ok()
    }

    pub fn ime_commit(&mut self, surface: &WlSurface) {
        if self
            .ime
            .popup
            .as_ref()
            .is_some_and(|p| &p.surface == surface)
        {
            self.place_ime_popup();
        }
    }

    pub fn place_ime_popup(&mut self) {
        let Some(p) = self.ime.popup.as_ref() else {
            return;
        };
        let node = p.node;
        let surface = p.surface.clone();
        let mapped = p.role.is_alive()
            && with_renderer_surface_state(&surface, |s| s.buffer().is_some()).unwrap_or(false);
        let caret = match self.ime.field.caret() {
            Some(c) => mapped.then_some(c),
            None => p
                .popup
                .as_ref()
                .and_then(|pp| pp.get_parent().map(|parent| (pp, parent)))
                .filter(|_| mapped)
                .and_then(|(pp, parent)| self.caret(&parent.surface, pp.text_input_rectangle())),
        };
        let was = self.scene.coords(node);
        let Some(caret) = caret else {
            self.scene.set_enabled(node, false);
            if was.is_some_and(|(_, shown)| shown) {
                self.redraw_all();
            }
            return;
        };
        let size = bbox_from_surface_tree(&surface, (0, 0)).size;
        let out = self.layout_box(
            self.output_at(
                f64::from(caret.x + caret.width / 2),
                f64::from(caret.y + caret.height / 2),
            )
            .as_ref(),
        );
        let (x, y) = if out.width > 0 {
            popup_origin(caret, (size.w, size.h), out)
        } else {
            (caret.x, caret.bottom())
        };
        // A balloon's reply field sits on the overlay layer: its popup goes
        // there too, over the balloon's tail.
        let reply =
            self.editor() == Some(crate::caret::Editor::NotifyReply) && self.ime.field.active();
        let layer = if reply {
            self.layers.overlay
        } else {
            self.layers.menu
        };
        self.scene.reparent(node, layer);
        self.scene.set_position(node, x, y);
        self.scene.raise_to_top(node);
        self.scene.set_enabled(node, true);
        if was != Some(((x, y).into(), true)) {
            self.redraw_all();
        }
        let local = Rect::new(caret.x - x, caret.y - y, caret.width, caret.height);
        if let Some(p) = self.ime.popup.as_mut()
            && p.sent != Some(local)
        {
            p.sent = Some(local);
            p.role
                .text_input_rectangle(local.x, local.y, local.width, local.height);
        }
    }

    /// The text cursor `rect` (local to `surface`, the text input's) in
    /// layout coordinates, while `surface` is shown. A client that reports
    /// no rectangle leaves it empty at the surface's origin, so the popup
    /// still opens by the window.
    fn caret(&self, surface: &WlSurface, rect: Rectangle<i32, Logical>) -> Option<Rect> {
        let node = self.scene.surface_node(surface)?;
        let (origin, shown) = self.scene.coords(node)?;
        shown.then(|| {
            Rect::new(
                origin.x + rect.loc.x,
                origin.y + rect.loc.y,
                rect.size.w,
                rect.size.h,
            )
        })
    }

    pub fn text_caret(&self) -> Option<Rect> {
        if let Some(pp) = self.ime.popup.as_ref().and_then(|p| p.popup.as_ref())
            && let Some(parent) = pp.get_parent()
        {
            let rect = pp.text_input_rectangle();
            if !(rect.is_empty() && rect.loc == (0, 0).into()) {
                return self.caret(&parent.surface, rect);
            }
        }
        let mut found = None;
        self.seat
            .text_input()
            .with_active_text_input(|ti, surface| {
                if let Some(TiState {
                    enabled: true,
                    cursor: Some(c),
                    ..
                }) = self.ime.cursors.get(&ti.id())
                {
                    found = Some((surface.clone(), *c));
                }
            });
        let (surface, c) = found?;
        self.caret(
            &surface,
            Rectangle::new((c.x, c.y).into(), (c.width, c.height).into()),
        )
    }

    pub fn ime_adopt_popup(&mut self, role: ZwpInputPopupSurfaceV2, surface: WlSurface) {
        match self.ime.popup.as_mut() {
            Some(p) if p.surface == surface => {
                p.role = role;
                p.sent = None;
            }
            _ => {
                if let Some(old) = self.ime.popup.take() {
                    self.scene.destroy(old.node);
                }
                let node = self.scene.surface_create(self.layers.menu, surface.clone());
                self.scene.set_enabled(node, false);
                add_destruction_hook::<State, _>(&surface, |state, s| state.ime_popup_gone(s));
                self.ime.popup = Some(ImePopup {
                    surface,
                    role,
                    popup: None,
                    node,
                    sent: None,
                });
            }
        }
        self.place_ime_popup();
    }

    fn ime_popup_gone(&mut self, surface: &WlSurface) {
        if let Some(p) = self.ime.popup.take_if(|p| &p.surface == surface) {
            self.scene.destroy(p.node);
            self.redraw_all();
        }
    }
}

impl InputMethodHandler for State {
    fn new_popup(&mut self, popup: PopupSurface) {
        let (role, surface) = (popup.surface_role.clone(), popup.wl_surface().clone());
        self.ime_adopt_popup(role, surface);
        if let Some(p) = self.ime.popup.as_mut() {
            p.popup = Some(popup);
        }
        self.place_ime_popup();
    }

    fn dismiss_popup(&mut self, popup: PopupSurface) {
        if let Some(pp) = self.ime.popup.as_mut().and_then(|p| p.popup.as_mut())
            && *pp == popup
        {
            pp.set_parent(None);
        }
        self.place_ime_popup();
    }

    fn popup_repositioned(&mut self, _popup: PopupSurface) {
        if let Some(p) = self.ime.popup.as_mut() {
            p.sent = None;
        }
        self.place_ime_popup();
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.scene
            .surface_node(parent)
            .and_then(|n| self.scene.extents(n))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod guard_tests {
    use super::VkGuard;
    use smithay::reexports::wayland_server::{
        self as ws, Display, Resource,
        protocol::{wl_compositor::WlCompositor, wl_region::WlRegion},
    };
    use std::sync::Arc;
    use wayland_client::{
        self as wc, Connection, EventQueue, Proxy,
        protocol::{
            wl_compositor::WlCompositor as CCompositor, wl_region::WlRegion as CRegion, wl_registry,
        },
    };

    #[derive(Default)]
    struct Server {
        region: Option<WlRegion>,
        adds: u32,
    }
    impl ws::GlobalDispatch<WlCompositor, ()> for Server {
        fn bind(
            _: &mut Self,
            _: &ws::DisplayHandle,
            _: &ws::Client,
            new: ws::New<WlCompositor>,
            _: &(),
            init: &mut ws::DataInit<'_, Self>,
        ) {
            init.init(new, ());
        }
    }
    impl ws::Dispatch<WlCompositor, ()> for Server {
        fn request(
            st: &mut Self,
            _: &ws::Client,
            _: &WlCompositor,
            req: ws::protocol::wl_compositor::Request,
            _: &(),
            _: &ws::DisplayHandle,
            init: &mut ws::DataInit<'_, Self>,
        ) {
            if let ws::protocol::wl_compositor::Request::CreateRegion { id } = req {
                st.region = Some(init.init(id, ()));
            }
        }
    }
    impl ws::Dispatch<WlRegion, ()> for Server {
        fn request(
            st: &mut Self,
            _: &ws::Client,
            _: &WlRegion,
            req: ws::protocol::wl_region::Request,
            _: &(),
            _: &ws::DisplayHandle,
            _: &mut ws::DataInit<'_, Self>,
        ) {
            if let ws::protocol::wl_region::Request::Add { .. } = req {
                st.adds += 1;
            }
        }
    }

    #[derive(Default)]
    struct ClientSide {
        compositor: Option<u32>,
    }
    impl wc::Dispatch<wl_registry::WlRegistry, ()> for ClientSide {
        fn event(
            st: &mut Self,
            _: &wl_registry::WlRegistry,
            ev: wl_registry::Event,
            _: &(),
            _: &Connection,
            _: &wc::QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name, interface, ..
            } = ev
                && interface == "wl_compositor"
            {
                st.compositor = Some(name);
            }
        }
    }
    wc::delegate_noop!(ClientSide: ignore CCompositor);
    wc::delegate_noop!(ClientSide: ignore CRegion);

    fn turn(
        display: &mut Display<Server>,
        srv: &mut Server,
        q: &mut EventQueue<ClientSide>,
        cs: &mut ClientSide,
    ) {
        q.flush().expect("client flush");
        display.dispatch_clients(srv).expect("dispatch");
        display.flush_clients().expect("server flush");
        q.dispatch_pending(cs).expect("client dispatch");
        if let Some(guard) = q.prepare_read() {
            let _ = guard.read();
        }
        q.dispatch_pending(cs).expect("client dispatch");
    }

    #[test]
    fn a_guarded_object_still_takes_the_requests_that_pass() {
        let mut display: Display<Server> = Display::new().expect("display");
        let dh = display.handle();
        dh.create_global::<Server, WlCompositor, ()>(4, ());
        let (a, b) = std::os::unix::net::UnixStream::pair().expect("pair");
        dh.clone()
            .insert_client(a, Arc::new(NoData))
            .expect("client");
        let conn = Connection::from_socket(b).expect("connection");
        let mut q = conn.new_event_queue::<ClientSide>();
        let qh = q.handle();
        let mut srv = Server::default();
        let mut cs = ClientSide::default();
        let registry = conn.display().get_registry(&qh, ());
        turn(&mut display, &mut srv, &mut q, &mut cs);
        let name = cs.compositor.expect("the compositor global");
        let compositor: CCompositor = registry.bind(name, 4, &qh, ());
        let region = compositor.create_region(&qh, ());
        turn(&mut display, &mut srv, &mut q, &mut cs);
        let id = srv.region.as_ref().expect("the region").id();
        let handle = dh.backend_handle();
        let inner = handle.get_object_data::<Server>(id.clone()).expect("data");
        let guard = VkGuard::<Server> {
            inner,
            passes: |_, _| true,
            divert: |_, _| false,
        };
        handle
            .set_object_data::<Server>(id.clone(), Arc::new(guard))
            .expect("set");
        region.add(0, 0, 1, 1);
        turn(&mut display, &mut srv, &mut q, &mut cs);
        region.add(0, 0, 2, 2);
        turn(&mut display, &mut srv, &mut q, &mut cs);
        assert_eq!(srv.adds, 2, "both requests went through the guard");
        let back = handle.get_object_data::<Server>(id).expect("data");
        assert!(
            back.is::<VkGuard<Server>>(),
            "the guard is back on the object"
        );
        assert!(region.is_alive());
    }

    struct NoData;
    impl ws::backend::ClientData for NoData {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_popups_take_their_keyboard_grab_back_from_the_input_method() {
        use KeyboardHolder::*;
        assert!(
            popup_grab_returns(true, &Nothing),
            "unset by the grab object's end"
        );
        assert!(
            popup_grab_returns(true, &InputMethod(None)),
            "replaced by grab_keyboard"
        );
        assert!(
            !popup_grab_returns(true, &Other),
            "the popups' own grab stays"
        );
        assert!(
            !popup_grab_returns(false, &Nothing),
            "closed popups take nothing"
        );
        assert!(!popup_grab_returns(false, &InputMethod(None)));
    }

    #[test]
    fn a_text_input_caret_counts_once_committed_and_enabled() {
        use zwp_text_input_v3::{
            REQ_COMMIT_OPCODE as COMMIT, REQ_DISABLE_OPCODE as DISABLE,
            REQ_ENABLE_OPCODE as ENABLE, REQ_SET_CURSOR_RECTANGLE_OPCODE as CURSOR,
        };
        let mut ti = TiState::default();
        ti.take(ENABLE, &[]);
        ti.take(CURSOR, &[10, 20, 2, 16]);
        assert_eq!(
            (ti.enabled, ti.cursor),
            (false, None),
            "nothing before the commit"
        );
        ti.take(COMMIT, &[]);
        assert_eq!(
            (ti.enabled, ti.cursor),
            (true, Some(Rect::new(10, 20, 2, 16)))
        );
        ti.take(CURSOR, &[30, 20, 2, 16]);
        ti.take(COMMIT, &[]);
        assert_eq!(
            ti.cursor,
            Some(Rect::new(30, 20, 2, 16)),
            "a later commit moves it"
        );
        ti.take(COMMIT, &[]);
        assert_eq!(
            ti.cursor,
            Some(Rect::new(30, 20, 2, 16)),
            "a bare commit keeps it"
        );
        ti.take(DISABLE, &[]);
        ti.take(COMMIT, &[]);
        assert_eq!((ti.enabled, ti.cursor), (false, None), "disabled: no caret");
        ti.take(ENABLE, &[]);
        ti.take(COMMIT, &[]);
        assert_eq!(
            (ti.enabled, ti.cursor),
            (true, None),
            "enabled again from clean"
        );
    }

    #[test]
    fn the_grab_hears_each_modifier_state_once() {
        let shift = SerializedMods {
            depressed: 1,
            ..Default::default()
        };
        let none = SerializedMods::default();
        let mut heard = HeardMods::default();
        assert_eq!(
            heard.unheard(none),
            Some(none),
            "a grab that heard nothing hears the state"
        );
        heard.heard(shift);
        assert_eq!(
            heard.unheard(shift),
            None,
            "Shift came with its key: no duplicate"
        );
        assert_eq!(
            heard.unheard(none),
            Some(none),
            "Shift released past the grab"
        );
        assert_eq!(heard.unheard(none), None, "the release is sent once");
    }

    const OUT: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_virtual_keyboard_types_nothing_while_the_keys_are_shut() {
        use zwp_virtual_keyboard_v1::{
            REQ_DESTROY_OPCODE, REQ_KEY_OPCODE, REQ_KEYMAP_OPCODE, REQ_MODIFIERS_OPCODE,
        };
        for op in [REQ_KEY_OPCODE, REQ_MODIFIERS_OPCODE] {
            assert!(!vk_request_passes(op, true), "opcode {op} is dropped");
            assert!(vk_request_passes(op, false), "opcode {op} passes unlocked");
        }
        for op in [REQ_KEYMAP_OPCODE, REQ_DESTROY_OPCODE] {
            assert!(vk_request_passes(op, true), "opcode {op} always passes");
        }
    }

    fn keymap_msg(fd: std::os::fd::OwnedFd, size: u32) -> Message<ObjectId, std::os::fd::OwnedFd> {
        Message {
            sender_id: ObjectId::null(),
            opcode: zwp_virtual_keyboard_v1::REQ_KEYMAP_OPCODE,
            args: [Argument::Uint(1), Argument::Fd(fd), Argument::Uint(size)]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn a_keymap_goes_on_as_a_copy_of_what_its_file_holds() {
        use std::os::unix::fs::FileExt;
        let text = b"xkb_keymap {};\0";
        let fd = crate::eis::sealed_memfd(text).expect("memfd");
        let msg = copy_keymap(keymap_msg(fd, 1 << 20)).expect("a readable keymap");
        let [_, Argument::Fd(copy), Argument::Uint(size)] = &msg.args[..] else {
            panic!("the keymap's arguments");
        };
        assert_eq!(*size as usize, text.len(), "the size is the file's");
        let mut back = vec![0; text.len()];
        std::fs::File::from(copy.try_clone().unwrap())
            .read_exact_at(&mut back, 0)
            .unwrap();
        assert_eq!(back, text);
    }

    #[test]
    fn a_keymap_from_a_pipe_or_past_the_cap_is_refused() {
        let (read, _write) = std::io::pipe().unwrap();
        assert!(copy_keymap(keymap_msg(read.into(), 16)).is_none(), "a pipe");
        let fd = crate::eis::sealed_memfd(b"x").unwrap();
        assert!(
            copy_keymap(keymap_msg(fd, MAX_VK_KEYMAP + 1)).is_none(),
            "past the cap"
        );
    }

    #[test]
    fn popup_opens_under_the_caret_left_edges_aligned() {
        let caret = Rect::new(300, 200, 2, 18);
        assert_eq!(popup_origin(caret, (240, 30), OUT), (300, 218));
    }

    #[test]
    fn popup_flips_above_the_caret_at_the_bottom_edge() {
        let caret = Rect::new(300, 1050, 2, 18);
        assert_eq!(popup_origin(caret, (240, 30), OUT), (300, 1020));
    }

    #[test]
    fn popup_stays_below_when_there_is_less_room_above() {
        let caret = Rect::new(300, 10, 2, 1000);
        assert_eq!(popup_origin(caret, (240, 100), OUT), (300, 1010));
    }

    #[test]
    fn popup_moves_left_to_stay_on_the_output() {
        let caret = Rect::new(1800, 200, 2, 18);
        assert_eq!(popup_origin(caret, (240, 30), OUT), (1680, 218));
    }

    #[test]
    fn popup_uses_the_second_output_s_edges() {
        let right = Rect::new(1920, 0, 1280, 800);
        let caret = Rect::new(3100, 790, 2, 10);
        assert_eq!(popup_origin(caret, (200, 40), right), (3000, 750));
    }

    #[test]
    fn a_release_goes_where_its_press_went() {
        let mut s = KeySplit::default();
        assert!(s.goes_to_grab(30, true, true));
        assert!(!s.goes_to_grab(30, false, false));
        assert!(!s.goes_to_grab(31, true, false));
        assert!(!s.goes_to_grab(31, false, true));
        assert!(s.goes_to_grab(32, true, true));
        assert!(s.goes_to_grab(32, false, true));
    }
}
