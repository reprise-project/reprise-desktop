//! The client's keyboard and mouse, sent into the session as a libei sender on the portal
//! session's EIS socket. Unicode keys go as `ei_text` keysyms.

use crate::geometry::{self, Region};
use crate::keymap::{self, Locks, Scancodes, Utf16};
use crate::layout::{Layout, SharedDesk};
use crate::portal::Grant;
use ironrdp_pdu::input::fast_path::SynchronizeFlags;
use ironrdp_server::{KeyboardEvent, MouseButton, MouseEvent, RdpServerInputHandler};
use reis::PendingRequestResult;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use reis::handshake::EiHandshaker;
use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

fn evdev_button(button: MouseButton) -> Option<u32> {
    match button {
        MouseButton::Left => Some(BTN_LEFT),
        MouseButton::Right => Some(BTN_RIGHT),
        MouseButton::Middle => Some(BTN_MIDDLE),
        MouseButton::X1 => Some(BTN_SIDE),
        MouseButton::X2 => Some(BTN_EXTRA),
        _ => None,
    }
}

/// xkb's Lock and Mod2 bits in a modifier mask: Caps Lock and Num Lock in every keymap.
const MOD_LOCK: u32 = 1 << 1;
const MOD_NUM: u32 = 1 << 4;

/// The libei discrete scroll of an RDP wheel rotation: both count 120 per notch, but RDP's
/// positive turn scrolls up.
pub fn wheel(value: i16) -> i32 {
    -i32::from(value)
}

/// 120ths of a notch from FreeRDP advanced-input wheel values, which carry them in 16.16 fixed
/// point. The fraction left over adds to the next value.
#[derive(Default)]
struct AinputWheel(i64);

impl AinputWheel {
    fn add(&mut self, value: i32) -> i32 {
        let total = self.0 + i64::from(value);
        self.0 = total % 0x1_0000;
        (total / 0x1_0000) as i32
    }
}

/// Something the client holds down, let go of with a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Key(u32),
    Button(u32),
    Keysym(u32),
}

/// What the client holds down, so a disconnect lets go of all of it.
#[derive(Default)]
pub struct Held(Vec<Release>);

impl Held {
    /// False when it was already down: a client's key repeat.
    pub fn press(&mut self, what: Release) -> bool {
        if self.0.contains(&what) {
            return false;
        }
        self.0.push(what);
        true
    }

    /// False when it was not down.
    pub fn release(&mut self, what: Release) -> bool {
        let before = self.0.len();
        self.0.retain(|&h| h != what);
        self.0.len() != before
    }

    pub fn release_all(&mut self) -> Vec<Release> {
        std::mem::take(&mut self.0)
    }
}

/// How long after a client move the picture's pointer may still show where it was.
const RECENT: Duration = Duration::from_millis(250);
const ONE_FRAME: Duration = Duration::from_micros(16_667);

/// Where the client put its pointer lately, in its own pixels, so the pointer position the
/// picture reports is not sent back to the client that caused it.
#[derive(Default)]
pub struct Moves(VecDeque<(Instant, (u16, u16))>);

impl Moves {
    pub fn moved(&mut self, to: (u16, u16), now: Instant) {
        while self.0.len() > 1
            && self
                .0
                .front()
                .is_some_and(|&(at, _)| now.duration_since(at) > RECENT)
        {
            self.0.pop_front();
        }
        self.0.push_back((now, to));
    }

    /// Whether the picture's pointer at `p` only follows the client's own moves: it is where the
    /// client last put it or put it lately, or the client moved it within the last frame.
    pub fn is_echo(&self, p: (u16, u16), now: Instant) -> bool {
        let Some(&(last_at, last)) = self.0.back() else {
            return false;
        };
        now.duration_since(last_at) < ONE_FRAME
            || near(p, last)
            || (self.0.iter()).any(|&(at, q)| now.duration_since(at) <= RECENT && near(p, q))
    }

    /// When an echo at `p` stops being one if the client moves no more; None when it is where
    /// the client last put the pointer, which stays an echo.
    pub fn echo_ends(&self, p: (u16, u16)) -> Option<Instant> {
        let &(last_at, last) = self.0.back()?;
        if near(p, last) {
            return None;
        }
        let lately = self.0.iter().filter(|&&(_, q)| near(p, q));
        Some(
            lately
                .map(|&(at, _)| at + RECENT)
                .fold(last_at + ONE_FRAME, Instant::max),
        )
    }

    /// The client was sent the pointer's position: its own moves no longer say where it is.
    pub fn sent(&mut self) {
        self.0.clear();
    }
}

fn near(p: (u16, u16), q: (u16, u16)) -> bool {
    p.0.abs_diff(q.0) <= 1 && p.1.abs_diff(q.1) <= 1
}

pub type SharedMoves = Arc<Mutex<Moves>>;

/// The time on tokio's clock, which tests can pause.
pub fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

/// One client input for the EIS connection.
#[derive(Debug, PartialEq)]
pub enum Event {
    Key(u32, bool),
    Keysym(u32, bool),
    Button(u32, bool),
    /// A point on the picture, in the client's pixels.
    Move(u16, u16),
    /// libei discrete scroll, 120 per notch.
    Wheel(i32, i32),
    Sync(Locks),
}

/// The id of the connection whose Synchronize sets the session's lock keys; 0 while none has.
#[derive(Clone, Default)]
pub struct SyncOwner(Arc<AtomicU64>);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl SyncOwner {
    /// Whether `id` owns sync, claiming it when nobody does.
    fn claim(&self, id: u64) -> bool {
        match self
            .0
            .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(owner) => owner == id,
        }
    }

    fn release(&self, id: u64) {
        let _ = (self.0).compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// The connection's input handler. Events queue until the portal session is open.
pub struct Input {
    id: u64,
    owner: SyncOwner,
    events: UnboundedSender<Event>,
    held: Held,
    scancodes: Scancodes,
    pressed: Utf16,
    released: Utf16,
    wheel_x: AinputWheel,
    wheel_y: AinputWheel,
    moves: SharedMoves,
    last_move: Option<(u16, u16)>,
}

/// The receiving side of an [`Input`], for the display that opens the portal session.
pub struct Feed {
    pub events: UnboundedReceiver<Event>,
    pub moves: SharedMoves,
}

impl Input {
    pub fn new(owner: SyncOwner) -> (Input, Feed) {
        let (tx, rx) = unbounded_channel();
        let moves = SharedMoves::default();
        let input = Input {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            owner,
            events: tx,
            held: Held::default(),
            scancodes: Scancodes::default(),
            pressed: Utf16::default(),
            released: Utf16::default(),
            wheel_x: AinputWheel::default(),
            wheel_y: AinputWheel::default(),
            moves: moves.clone(),
            last_move: None,
        };
        (input, Feed { events: rx, moves })
    }

    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn move_to(&mut self, x: u16, y: u16) {
        self.moves.lock().unwrap().moved((x, y), now());
        self.last_move = Some((x, y));
        self.send(Event::Move(x, y));
    }

    fn press(&mut self, what: Release) {
        if self.held.press(what) {
            self.send(as_event(what, true));
        }
    }

    fn release(&mut self, what: Release) {
        self.held.release(what);
        self.send(as_event(what, false));
    }

    fn button(&mut self, button: u32, pressed: bool) {
        match pressed {
            true => self.press(Release::Button(button)),
            false => self.release(Release::Button(button)),
        }
    }
}

fn as_release(event: &Event) -> Option<(Release, bool)> {
    match *event {
        Event::Key(k, pressed) => Some((Release::Key(k), pressed)),
        Event::Button(b, pressed) => Some((Release::Button(b), pressed)),
        Event::Keysym(s, pressed) => Some((Release::Keysym(s), pressed)),
        Event::Move(..) | Event::Wheel(..) | Event::Sync(_) => None,
    }
}

fn as_event(what: Release, pressed: bool) -> Event {
    match what {
        Release::Key(k) => Event::Key(k, pressed),
        Release::Button(b) => Event::Button(b, pressed),
        Release::Keysym(s) => Event::Keysym(s, pressed),
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.owner.release(self.id);
        for what in self.held.release_all() {
            self.send(as_event(what, false));
        }
    }
}

impl RdpServerInputHandler for Input {
    fn keyboard(&mut self, event: KeyboardEvent) {
        match event {
            KeyboardEvent::Pressed {
                code,
                extended,
                extended1,
            } => match self.scancodes.key(code, extended, extended1) {
                Some(key) => self.press(Release::Key(key)),
                None => log::debug!("scancode {code:#x} (extended {extended}) sends no key"),
            },
            KeyboardEvent::Released {
                code,
                extended,
                extended1,
            } => {
                if let Some(key) = self.scancodes.key(code, extended, extended1) {
                    self.release(Release::Key(key));
                }
            }
            KeyboardEvent::UnicodePressed(unit) => {
                if let Some(sym) = self.pressed.push(unit) {
                    self.press(Release::Keysym(sym));
                }
            }
            KeyboardEvent::UnicodeReleased(unit) => {
                if let Some(sym) = self.released.push(unit) {
                    self.release(Release::Keysym(sym));
                }
            }
            KeyboardEvent::Synchronize(flags) if self.owner.claim(self.id) => {
                self.send(Event::Sync(Locks {
                    caps: flags.contains(SynchronizeFlags::CAPS_LOCK),
                    num: flags.contains(SynchronizeFlags::NUM_LOCK),
                    scroll: flags.contains(SynchronizeFlags::SCROLL_LOCK),
                }))
            }
            KeyboardEvent::Synchronize(_) => {
                log::debug!("another viewer sets the lock keys; this Synchronize is dropped")
            }
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        match event {
            MouseEvent::Move { x, y } => self.move_to(x, y),
            MouseEvent::Button {
                x,
                y,
                button,
                pressed,
            } => match evdev_button(button) {
                Some(code) => {
                    if self.last_move != Some((x, y)) {
                        self.move_to(x, y);
                    }
                    self.button(code, pressed);
                }
                None => log::debug!("{button:?} is not sent"),
            },
            MouseEvent::ButtonRel {
                button, pressed, ..
            } => match evdev_button(button) {
                Some(code) => self.button(code, pressed),
                None => log::debug!("{button:?} is not sent"),
            },
            MouseEvent::VerticalScroll { value } => self.send(Event::Wheel(0, wheel(value))),
            MouseEvent::Scroll { x, y } => {
                let (dx, dy) = (self.wheel_x.add(x), -self.wheel_y.add(y));
                if (dx, dy) != (0, 0) {
                    self.send(Event::Wheel(dx, dy));
                }
            }
            MouseEvent::HorizontalScroll { value } => self.send(Event::Wheel(i32::from(value), 0)),
            MouseEvent::RelMove { .. } => log::debug!("relative pointer motion is not sent"),
            other => log::debug!("{other:?} is not sent"),
        }
    }
}

/// Send `feed`'s events into the session of `grant` until the client's input ends. Runs on the
/// connection's `LocalSet`.
pub async fn serve(grant: Arc<Grant>, mut events: UnboundedReceiver<Event>, desk: SharedDesk) {
    if let Err(e) = Sender::run(&grant, &mut events, &desk).await {
        log::warn!("remote input stopped: {e}");
    }
    while events.recv().await.is_some() {}
}

/// The most events other than moves a device's queue keeps; the oldest go first.
const QUEUED: usize = 64;

/// One device's events while it is away, and what the session holds on it.
#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    held: Held,
    /// Events were dropped, so a release the session waits for may be gone.
    overflowed: bool,
}

impl Queue {
    /// Queue `event`. A move replaces a move right before it and a wheel turn adds to one right
    /// before it, so a long drag or scroll takes one place in the queue; turns that cancel out go.
    fn push(&mut self, event: Event) {
        let queue = &mut self.events;
        match (queue.back_mut(), &event) {
            (Some(Event::Move(..)), Event::Move(..)) => {
                queue.pop_back();
            }
            (Some(Event::Wheel(x, y)), &Event::Wheel(dx, dy)) => {
                *x = x.saturating_add(dx);
                *y = y.saturating_add(dy);
                if (*x, *y) == (0, 0) {
                    queue.pop_back();
                }
                return;
            }
            _ => {}
        }
        queue.push_back(event);
        let is_move = |e: &Event| matches!(e, Event::Move(..));
        let mut over = (queue.iter().filter(|e| !is_move(e)).count()).saturating_sub(QUEUED);
        self.overflowed |= over > 0;
        while over > 0
            && let Some(dropped) = queue.pop_front()
        {
            if !is_move(&dropped) {
                over -= 1;
            }
        }
    }

    /// The queued events for the device that is back, without releases of what is not held or
    /// presses of what is; after an overflow the session first lets go of all it holds.
    fn drain(&mut self, ready: &mut Vec<Event>) {
        if std::mem::take(&mut self.overflowed) {
            ready.extend(
                self.held
                    .release_all()
                    .into_iter()
                    .map(|w| as_event(w, false)),
            );
        }
        for event in self.events.drain(..) {
            let kept = match as_release(&event) {
                Some((what, true)) => self.held.press(what),
                Some((what, false)) => self.held.release(what),
                None => true,
            };
            if kept {
                ready.push(event);
            }
        }
    }
}

/// Events waiting for their device, kept in order per device; None for a device the session
/// does not have.
struct Waiting {
    keyboard: Option<Queue>,
    pointer: Option<Queue>,
}

impl Waiting {
    fn new(keyboard: bool, pointer: bool) -> Waiting {
        Waiting {
            keyboard: keyboard.then(Queue::default),
            pointer: pointer.then(Queue::default),
        }
    }

    fn push(&mut self, event: Event) {
        let queue = match event {
            Event::Key(..) | Event::Keysym(..) | Event::Sync(_) => &mut self.keyboard,
            Event::Move(..) | Event::Button(..) | Event::Wheel(..) => &mut self.pointer,
        };
        if let Some(queue) = queue {
            queue.push(event);
        }
    }

    /// The events whose device is there now.
    fn take_ready(&mut self, keyboard: bool, pointer: bool) -> Vec<Event> {
        let mut ready = Vec::new();
        for (there, queue) in [(keyboard, &mut self.keyboard), (pointer, &mut self.pointer)] {
            if let Some(queue) = queue.as_mut().filter(|_| there) {
                queue.drain(&mut ready);
            }
        }
        ready
    }

    fn flush(&mut self, sender: &mut Sender) {
        let ready = self.take_ready(sender.keyboard.is_some(), sender.pointer.is_some());
        ready.into_iter().for_each(|event| sender.send(event));
    }
}

/// The EIS connection as a sender: one keyboard and one absolute pointer device.
struct Sender<'a> {
    grant: &'a Grant,
    desk: &'a SharedDesk,
    events: Option<EiEventConverter>,
    keyboard: Option<reis::event::Device>,
    pointer: Option<reis::event::Device>,
    sequence: u32,
    locks: Option<Locks>,
    wanted: Option<Locks>,
}

impl<'a> Sender<'a> {
    async fn run(
        grant: &'a Grant,
        feed: &mut UnboundedReceiver<Event>,
        desk: &'a SharedDesk,
    ) -> Result<(), String> {
        let fd = grant.eis_fd.try_clone().map_err(|e| e.to_string())?;
        let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| e.to_string())?;
        // SAFETY: the clone shares the context's socket, which stays open while any clone lives,
        // and its `as_raw_fd` always names that socket.
        let socket =
            unsafe { AsyncFd::register_with_interest(context.clone(), Interest::READABLE) }
                .map_err(|e| e.to_string())?;
        let mut handshaker =
            EiHandshaker::new("Remote Desktop", ei::handshake::ContextType::Sender);
        let mut sender = Sender {
            grant,
            desk,
            events: None,
            keyboard: None,
            pointer: None,
            sequence: 0,
            locks: None,
            wanted: None,
        };
        let mut waiting = Waiting::new(grant.keyboard, grant.pointer);
        loop {
            tokio::select! {
                readable = socket.readable() => {
                    let mut guard = readable.map_err(|e| e.to_string())?;
                    match context.read() {
                        Ok(_) => guard.clear_ready(),
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                            log::info!("the session ended remote input");
                            return Ok(());
                        }
                        Err(e) => return Err(e.to_string()),
                    }
                    while let Some(pending) = context.pending_event() {
                        let event = match pending {
                            PendingRequestResult::Request(event) => event,
                            PendingRequestResult::ParseError(e) => return Err(e.to_string()),
                            PendingRequestResult::InvalidObject(_) => continue,
                        };
                        match &mut sender.events {
                            Some(converter) => converter.handle_event(event).map_err(|e| e.to_string())?,
                            None => {
                                if let Some(resp) = handshaker.handle_event(event).map_err(|e| e.to_string())? {
                                    sender.events = Some(EiEventConverter::new(&context, resp));
                                }
                            }
                        }
                    }
                    if !sender.session_events() {
                        return Ok(());
                    }
                    waiting.flush(&mut sender);
                }
                event = feed.recv() => match event {
                    Some(event) => {
                        waiting.push(event);
                        waiting.flush(&mut sender);
                    }
                    None => {
                        let _ = context.flush();
                        return Ok(());
                    }
                },
            }
            let _ = context.flush();
        }
    }

    /// Follow the seat and its devices; false once the session disconnected.
    fn session_events(&mut self) -> bool {
        while let Some(event) = self.events.as_mut().and_then(|c| c.next_event()) {
            match event {
                EiEvent::SeatAdded(added) => added.seat.bind_capabilities(
                    DeviceCapability::Keyboard
                        | DeviceCapability::Text
                        | DeviceCapability::PointerAbsolute
                        | DeviceCapability::Button
                        | DeviceCapability::Scroll,
                ),
                EiEvent::DeviceResumed(resumed) => {
                    let device = resumed.device;
                    let keyboard = device.has_capability(DeviceCapability::Keyboard);
                    let pointer = device.has_capability(DeviceCapability::PointerAbsolute);
                    if !keyboard && !pointer {
                        continue;
                    }
                    device
                        .device()
                        .start_emulating(resumed.serial, self.sequence);
                    self.sequence = self.sequence.wrapping_add(1);
                    if keyboard {
                        self.keyboard = Some(device.clone());
                    }
                    if pointer {
                        self.pointer = Some(device);
                    }
                }
                EiEvent::DevicePaused(reis::event::DevicePaused { device, .. })
                | EiEvent::DeviceRemoved(reis::event::DeviceRemoved { device }) => {
                    for slot in [&mut self.keyboard, &mut self.pointer] {
                        if slot.as_ref() == Some(&device) {
                            *slot = None;
                        }
                    }
                }
                EiEvent::KeyboardModifiers(m) => {
                    self.locks = Some(Locks {
                        caps: m.locked & MOD_LOCK != 0,
                        num: m.locked & MOD_NUM != 0,
                        scroll: false,
                    });
                    if let Some(wanted) = self.wanted.take() {
                        self.sync(wanted);
                    }
                }
                EiEvent::Disconnected(d) => {
                    log::info!("the session ended remote input: {:?}", d.reason);
                    return false;
                }
                _ => {}
            }
        }
        true
    }

    fn send(&mut self, event: Event) {
        match event {
            Event::Key(key, pressed) => self.key(key, pressed),
            Event::Keysym(sym, pressed) => self.keysym(sym, pressed),
            Event::Button(button, pressed) => self.on_pointer(|d| {
                let state = match pressed {
                    true => ei::button::ButtonState::Press,
                    false => ei::button::ButtonState::Released,
                };
                d.interface::<ei::Button>().map(|b| b.button(button, state))
            }),
            Event::Move(x, y) => {
                let Some(pointer) = &self.pointer else {
                    return;
                };
                let desk = self.desk.lock().unwrap();
                let at = to_logical(desk.layout(), (x, y), |screen| self.region(pointer, screen));
                drop(desk);
                let Some((x, y)) = at else {
                    return;
                };
                self.on_pointer(|d| {
                    (d.interface::<ei::PointerAbsolute>())
                        .map(|p| p.motion_absolute(x as f32, y as f32))
                });
            }
            Event::Wheel(dx, dy) => self.on_pointer(|d| {
                d.interface::<ei::Scroll>()
                    .map(|s| s.scroll_discrete(dx, dy))
            }),
            Event::Sync(wanted) => self.sync(wanted),
        }
    }

    fn key(&self, key: u32, pressed: bool) {
        let Some(device) = &self.keyboard else {
            return;
        };
        if let Some(keyboard) = device.interface::<ei::Keyboard>() {
            keyboard.key(key, key_state(pressed));
            self.frame(device);
        }
    }

    fn keysym(&self, sym: u32, pressed: bool) {
        let Some(device) = &self.keyboard else {
            return;
        };
        let Some(text) = device.interface::<ei::Text>() else {
            log::debug!("the session takes no text; keysym {sym:#x} is dropped");
            return;
        };
        text.keysym(sym, key_state(pressed));
        self.frame(device);
    }

    /// Tap the lock keys that differ; waits for the session's lock state if it is not known.
    fn sync(&mut self, wanted: Locks) {
        let Some(current) = self.locks else {
            self.wanted = Some(wanted);
            return;
        };
        let current = Locks {
            scroll: wanted.scroll,
            ..current
        };
        for key in keymap::sync_toggles(current, wanted) {
            self.key(key, true);
            self.key(key, false);
        }
        self.locks = Some(wanted);
    }

    fn on_pointer(&self, emit: impl FnOnce(&reis::event::Device) -> Option<()>) {
        if let Some(device) = &self.pointer
            && emit(device).is_some()
        {
            self.frame(device);
        }
    }

    fn frame(&self, device: &reis::event::Device) {
        let serial = self.events.as_ref().map_or(0, |c| c.connection().serial());
        device.device().frame(serial, now_us());
    }

    /// Shared screen `screen`'s region on `pointer`, or where the portal said it was.
    fn region(&self, pointer: &reis::event::Device, screen: usize) -> Region {
        let screen = &self.grant.screens[screen];
        let shared = pointer
            .regions()
            .iter()
            .find(|r| r.mapping_id.is_some() && r.mapping_id == screen.mapping_id);
        match shared {
            Some(r) => Region {
                x: r.x as i32,
                y: r.y as i32,
                width: r.width,
                height: r.height,
            },
            None => Region {
                x: screen.position.0,
                y: screen.position.1,
                width: screen.size.0,
                height: screen.size.1,
            },
        }
    }
}

/// The logical layout point under client point `p`: on the screen `layout` draws there, whose
/// place in the layout `region` gives. None outside every screen.
fn to_logical(
    layout: &Layout,
    p: (u16, u16),
    region: impl Fn(usize) -> Region,
) -> Option<(f64, f64)> {
    let (screen, at) = layout.to_screen(p)?;
    Some(geometry::to_logical(
        at,
        layout.pixels(screen)?,
        region(screen),
    ))
}

fn key_state(pressed: bool) -> ei::keyboard::KeyState {
    match pressed {
        true => ei::keyboard::KeyState::Press,
        false => ei::keyboard::KeyState::Released,
    }
}

/// CLOCK_MONOTONIC in microseconds, libei's event time.
fn now_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for clock_gettime to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_pdu::input::fast_path::KeyboardFlags;
    use ironrdp_pdu::input::mouse::PointerFlags;
    use ironrdp_pdu::input::mouse_x::PointerXFlags;
    use ironrdp_pdu::input::{MousePdu, MouseXPdu};

    #[test]
    fn wheel_turns_flip_sign_in_120ths() {
        assert_eq!(wheel(120), -120);
        assert_eq!(wheel(-240), 240);
    }

    #[test]
    fn an_advanced_input_notch_down_is_120_down() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        input.mouse(MouseEvent::Scroll {
            x: 0,
            y: -120 << 16,
        });
        input.mouse(MouseEvent::Scroll { x: 120 << 16, y: 0 });
        assert_eq!(feed.events.try_recv(), Ok(Event::Wheel(0, 120)));
        assert_eq!(feed.events.try_recv(), Ok(Event::Wheel(120, 0)), "right");
    }

    fn sent(feed: &mut Feed) -> Vec<Event> {
        std::iter::from_fn(|| feed.events.try_recv().ok()).collect()
    }

    fn pointer_pdu(flags: PointerFlags, wheel: i16) -> MouseEvent {
        MouseEvent::from(MousePdu {
            flags,
            number_of_wheel_rotation_units: wheel,
            x_position: 5,
            y_position: 6,
        })
    }

    #[test]
    fn a_press_lands_where_its_pdu_puts_it() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let left = |flags| {
            MouseEvent::from(MousePdu {
                flags: PointerFlags::LEFT_BUTTON | flags,
                number_of_wheel_rotation_units: 0,
                x_position: 50,
                y_position: 60,
            })
        };
        input.mouse(MouseEvent::Move { x: 10, y: 10 });
        input.mouse(left(PointerFlags::DOWN));
        input.mouse(left(PointerFlags::empty()));
        assert_eq!(
            sent(&mut feed),
            [
                Event::Move(10, 10),
                Event::Move(50, 60),
                Event::Button(BTN_LEFT, true),
                Event::Button(BTN_LEFT, false)
            ]
        );
    }

    #[test]
    fn an_advanced_input_press_lands_where_its_pdu_puts_it() {
        use ironrdp_ainput::MouseEventFlags as A;
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let left = |flags| {
            MouseEvent::from(ironrdp_ainput::MousePdu {
                time: 0,
                flags: A::BUTTON1 | flags,
                x: 50,
                y: 60,
            })
        };
        input.mouse(MouseEvent::Move { x: 10, y: 10 });
        input.mouse(left(A::DOWN));
        input.mouse(left(A::empty()));
        assert_eq!(
            sent(&mut feed),
            [
                Event::Move(10, 10),
                Event::Move(50, 60),
                Event::Button(BTN_LEFT, true),
                Event::Button(BTN_LEFT, false)
            ]
        );
    }

    #[test]
    fn a_relative_advanced_input_click_lands_at_the_pointer() {
        use ironrdp_ainput::MouseEventFlags as A;
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let left = |flags| {
            MouseEvent::from(ironrdp_ainput::MousePdu {
                time: 0,
                flags: A::REL | A::HAVE_REL | A::BUTTON1 | flags,
                x: -3,
                y: 0,
            })
        };
        input.mouse(MouseEvent::Move { x: 10, y: 10 });
        input.mouse(left(A::DOWN));
        input.mouse(left(A::empty()));
        assert_eq!(
            sent(&mut feed),
            [
                Event::Move(10, 10),
                Event::Button(BTN_LEFT, true),
                Event::Button(BTN_LEFT, false)
            ]
        );
    }

    #[test]
    fn middle_button_reaches_the_session() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let middle = PointerFlags::MIDDLE_BUTTON_OR_WHEEL;
        input.mouse(pointer_pdu(middle | PointerFlags::DOWN, 0));
        input.mouse(pointer_pdu(middle, 0));
        assert_eq!(
            sent(&mut feed),
            [
                Event::Move(5, 6),
                Event::Button(BTN_MIDDLE, true),
                Event::Button(BTN_MIDDLE, false)
            ]
        );
    }

    #[test]
    fn side_buttons_are_back_and_forward() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let extended = |flags| {
            MouseEvent::from(MouseXPdu {
                flags,
                x_position: 5,
                y_position: 6,
            })
        };
        input.mouse(extended(PointerXFlags::BUTTON1 | PointerXFlags::DOWN));
        input.mouse(extended(PointerXFlags::BUTTON1));
        input.mouse(extended(PointerXFlags::BUTTON2 | PointerXFlags::DOWN));
        input.mouse(extended(PointerXFlags::BUTTON2));
        let advanced = |flags| {
            MouseEvent::from(ironrdp_ainput::MousePdu {
                time: 0,
                flags,
                x: 5,
                y: 6,
            })
        };
        use ironrdp_ainput::MouseEventFlags as A;
        input.mouse(advanced(A::XBUTTON1 | A::DOWN));
        input.mouse(advanced(A::XBUTTON1));
        input.mouse(advanced(A::XBUTTON2 | A::DOWN));
        input.mouse(advanced(A::XBUTTON2));
        assert_eq!(
            sent(&mut feed),
            [
                Event::Move(5, 6),
                Event::Button(BTN_SIDE, true),
                Event::Button(BTN_SIDE, false),
                Event::Button(BTN_EXTRA, true),
                Event::Button(BTN_EXTRA, false),
                Event::Button(BTN_SIDE, true),
                Event::Button(BTN_SIDE, false),
                Event::Button(BTN_EXTRA, true),
                Event::Button(BTN_EXTRA, false),
            ]
        );
    }

    #[test]
    fn horizontal_wheel_scrolls_sideways() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let decoded = |flags: u16| {
            let bytes = [flags.to_le_bytes(), [0, 0], [0, 0]].concat();
            let pdu: MousePdu = ironrdp_core::decode(&bytes).unwrap();
            MouseEvent::from(pdu)
        };
        input.mouse(decoded(0x0400 | 120));
        input.mouse(decoded(0x0400 | 0x0100 | 0x88));
        assert_eq!(
            sent(&mut feed),
            [Event::Wheel(120, 0), Event::Wheel(-120, 0)],
            "a positive turn scrolls right, a negative one left"
        );
    }

    #[test]
    fn pause_is_one_key() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        let e1 = KeyboardFlags::EXTENDED1;
        let up = KeyboardFlags::RELEASE;
        for (code, flags) in [
            (0x1D_u8, e1),
            (0x45, KeyboardFlags::empty()),
            (0x1D, e1 | up),
            (0x45, up),
        ] {
            input.keyboard(KeyboardEvent::from((code, flags)));
        }
        assert_eq!(
            sent(&mut feed),
            [Event::Key(119, true), Event::Key(119, false)]
        );
        input.keyboard(KeyboardEvent::from((0x45_u8, KeyboardFlags::empty())));
        assert_eq!(sent(&mut feed), [Event::Key(69, true)], "Num Lock alone");
    }

    #[test]
    fn a_late_device_gets_a_bounded_queue() {
        let mut waiting = Waiting::new(true, true);
        for i in 0..100 {
            waiting.push(Event::Move(i, i));
        }
        waiting.push(Event::Button(BTN_LEFT, true));
        for i in 0..100 {
            waiting.push(Event::Move(i, 0));
        }
        waiting.push(Event::Button(BTN_LEFT, false));
        assert_eq!(
            waiting.take_ready(false, true),
            [
                Event::Move(99, 99),
                Event::Button(BTN_LEFT, true),
                Event::Move(99, 0),
                Event::Button(BTN_LEFT, false)
            ],
            "the latest position before each click"
        );
        for key in 0..100 {
            waiting.push(Event::Key(key, true));
        }
        let keys = waiting.take_ready(true, false);
        assert_eq!(keys.len(), 64);
        assert_eq!(keys[0], Event::Key(36, true), "the oldest go first");

        let mut no_keyboard = Waiting::new(false, true);
        no_keyboard.push(Event::Key(30, true));
        assert_eq!(no_keyboard.take_ready(true, true), []);
    }

    #[test]
    fn a_long_scroll_waits_as_one_wheel_turn() {
        let mut waiting = Waiting::new(true, true);
        waiting.push(Event::Button(BTN_LEFT, true));
        for _ in 0..200 {
            waiting.push(Event::Wheel(0, 120));
        }
        waiting.push(Event::Wheel(-120, 0));
        waiting.push(Event::Button(BTN_LEFT, false));
        assert_eq!(
            waiting.take_ready(false, true),
            [
                Event::Button(BTN_LEFT, true),
                Event::Wheel(-120, 24_000),
                Event::Button(BTN_LEFT, false)
            ]
        );
    }

    #[test]
    fn wheel_turns_that_cancel_out_while_waiting_send_nothing() {
        let mut waiting = Waiting::new(true, true);
        waiting.push(Event::Wheel(0, 120));
        waiting.push(Event::Wheel(0, -120));
        assert_eq!(waiting.take_ready(false, true), []);
    }

    #[test]
    fn a_device_back_after_an_overflow_lets_go_of_what_it_held() {
        let mut waiting = Waiting::new(true, true);
        waiting.push(Event::Key(29, true));
        waiting.push(Event::Button(BTN_LEFT, true));
        waiting.push(Event::Button(BTN_RIGHT, true));
        assert_eq!(waiting.take_ready(true, true).len(), 3);

        waiting.push(Event::Key(29, false));
        for key in 0..40 {
            waiting.push(Event::Key(key + 100, true));
            waiting.push(Event::Key(key + 100, false));
        }
        waiting.push(Event::Button(BTN_LEFT, false));
        let keys = waiting.take_ready(true, false);
        assert_eq!(keys[0], Event::Key(29, false), "the dropped release");
        assert_eq!(keys.len(), 1 + 64);
        assert!(!keys[1..].contains(&Event::Key(29, false)));

        assert_eq!(
            waiting.take_ready(true, true),
            [Event::Button(BTN_LEFT, false)],
            "a queue that kept everything replays as it is"
        );
    }

    #[test]
    fn events_wait_for_their_own_device() {
        let mut waiting = Waiting::new(true, true);
        waiting.push(Event::Key(30, true));
        waiting.push(Event::Move(1, 2));
        waiting.push(Event::Button(BTN_LEFT, true));
        assert_eq!(waiting.take_ready(true, false), [Event::Key(30, true)]);
        assert_eq!(waiting.take_ready(true, false), []);
        assert_eq!(
            waiting.take_ready(true, true),
            [Event::Move(1, 2), Event::Button(BTN_LEFT, true)]
        );
    }

    #[test]
    fn advanced_input_wheel_fractions_add_up() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        input.mouse(MouseEvent::Scroll {
            x: 0,
            y: -(1 << 15),
        });
        assert!(
            feed.events.try_recv().is_err(),
            "half a 120th sends nothing"
        );
        input.mouse(MouseEvent::Scroll {
            x: 0,
            y: -(1 << 15),
        });
        assert_eq!(feed.events.try_recv(), Ok(Event::Wheel(0, 1)));
    }

    #[test]
    fn disconnect_releases_every_held_key_and_button() {
        let mut held = Held::default();
        held.press(Release::Key(56));
        held.press(Release::Key(15));
        held.press(Release::Button(BTN_LEFT));
        held.release(Release::Key(15));
        assert_eq!(
            held.release_all(),
            vec![Release::Key(56), Release::Button(BTN_LEFT)]
        );
        assert_eq!(held.release_all(), vec![], "nothing is held twice");
    }

    fn caps(on: bool) -> KeyboardEvent {
        let mut flags = SynchronizeFlags::empty();
        flags.set(SynchronizeFlags::CAPS_LOCK, on);
        KeyboardEvent::Synchronize(flags)
    }

    fn caps_sync(on: bool) -> Event {
        Event::Sync(Locks {
            caps: on,
            num: false,
            scroll: false,
        })
    }

    #[test]
    fn a_second_viewers_sync_is_dropped() {
        let owner = SyncOwner::default();
        let (mut first, mut first_feed) = Input::new(owner.clone());
        let (mut second, mut second_feed) = Input::new(owner);
        first.keyboard(caps(true));
        second.keyboard(caps(false));
        first.keyboard(caps(false));
        assert_eq!(
            sent(&mut first_feed),
            vec![caps_sync(true), caps_sync(false)]
        );
        assert_eq!(
            sent(&mut second_feed),
            vec![],
            "the second viewer taps no lock keys"
        );
    }

    #[test]
    fn the_next_viewer_owns_sync_once_the_owner_leaves() {
        let owner = SyncOwner::default();
        let (mut first, _first_feed) = Input::new(owner.clone());
        let (mut second, mut second_feed) = Input::new(owner);
        first.keyboard(caps(true));
        drop(first);
        second.keyboard(caps(false));
        assert_eq!(sent(&mut second_feed), vec![caps_sync(false)]);
    }

    #[test]
    fn a_viewer_that_never_synced_leaves_the_claim_alone() {
        let owner = SyncOwner::default();
        let (mut first, mut first_feed) = Input::new(owner.clone());
        let (bystander, _bystander_feed) = Input::new(owner.clone());
        let (mut third, mut third_feed) = Input::new(owner);
        first.keyboard(caps(true));
        drop(bystander);
        third.keyboard(caps(false));
        first.keyboard(caps(false));
        assert_eq!(
            sent(&mut third_feed),
            vec![],
            "the first viewer still owns sync"
        );
        assert_eq!(
            sent(&mut first_feed),
            vec![caps_sync(true), caps_sync(false)]
        );
    }

    #[test]
    fn dropping_the_handler_sends_releases_for_what_the_client_held() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        input.keyboard(KeyboardEvent::Pressed {
            code: 0x38,
            extended: false,
            extended1: false,
        });
        input.keyboard(KeyboardEvent::Pressed {
            code: 0x0F,
            extended: false,
            extended1: false,
        });
        input.keyboard(KeyboardEvent::Released {
            code: 0x0F,
            extended: false,
            extended1: false,
        });
        input.mouse(MouseEvent::Button {
            x: 0,
            y: 0,
            button: MouseButton::Left,
            pressed: true,
        });
        input.keyboard(KeyboardEvent::UnicodePressed(0xE9));
        drop(input);
        let mut sent = Vec::new();
        while let Ok(event) = feed.events.try_recv() {
            sent.push(event);
        }
        assert_eq!(
            sent,
            vec![
                Event::Key(56, true),
                Event::Key(15, true),
                Event::Key(15, false),
                Event::Move(0, 0),
                Event::Button(BTN_LEFT, true),
                Event::Keysym(0xE9, true),
                Event::Key(56, false),
                Event::Button(BTN_LEFT, false),
                Event::Keysym(0xE9, false),
            ]
        );
    }

    #[test]
    fn a_key_repeat_sends_one_press() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        for _ in 0..3 {
            input.keyboard(KeyboardEvent::Pressed {
                code: 0x1E,
                extended: false,
                extended1: false,
            });
        }
        assert_eq!(feed.events.try_recv(), Ok(Event::Key(30, true)));
        assert!(feed.events.try_recv().is_err(), "the repeats are not sent");
    }

    #[test]
    fn an_emoji_typed_as_two_units_is_one_keysym_down_and_up() {
        let (mut input, mut feed) = Input::new(SyncOwner::default());
        input.keyboard(KeyboardEvent::UnicodePressed(0xD83D));
        input.keyboard(KeyboardEvent::UnicodeReleased(0xD83D));
        input.keyboard(KeyboardEvent::UnicodePressed(0xDE00));
        input.keyboard(KeyboardEvent::UnicodeReleased(0xDE00));
        assert_eq!(feed.events.try_recv(), Ok(Event::Keysym(0x0101_F600, true)));
        assert_eq!(
            feed.events.try_recv(),
            Ok(Event::Keysym(0x0101_F600, false))
        );
    }

    #[test]
    fn the_pointer_where_the_client_put_it_is_an_echo() {
        let t = Instant::now();
        let mut moves = Moves::default();
        moves.moved((100, 100), t);
        assert!(moves.is_echo((100, 100), t + Duration::from_secs(5)));
        assert!(
            moves.is_echo((101, 99), t + Duration::from_secs(5)),
            "a pixel of rounding"
        );
    }

    #[test]
    fn a_lagging_pointer_on_the_clients_path_is_an_echo() {
        let t = Instant::now();
        let mut moves = Moves::default();
        moves.moved((10, 10), t);
        moves.moved((20, 20), t + Duration::from_millis(8));
        moves.moved((30, 30), t + Duration::from_millis(16));
        assert!(moves.is_echo((10, 10), t + Duration::from_millis(60)));
    }

    #[test]
    fn a_warp_away_from_the_client_is_sent() {
        let t = Instant::now();
        let mut moves = Moves::default();
        moves.moved((10, 10), t);
        assert!(
            moves.is_echo((500, 500), t + Duration::from_millis(5)),
            "within a frame of a client move"
        );
        assert!(!moves.is_echo((500, 500), t + Duration::from_millis(60)));
        moves.sent();
        assert!(
            !moves.is_echo((10, 10), t + Duration::from_secs(1)),
            "after a sent position, a warp back to the old spot is sent too"
        );
    }

    #[test]
    fn no_client_move_means_no_echo() {
        assert!(!Moves::default().is_echo((0, 0), Instant::now()));
    }

    #[test]
    fn input_on_the_second_monitor_lands_on_the_second_screen() {
        use crate::layout::ScreenRect;
        use ironrdp_pdu::gcc::{Monitor, MonitorFlags};
        let screen = |x, w, h| ScreenRect {
            position: (x, 0),
            logical: (w, h),
            pixels: (w, h),
        };
        let monitor = |left, right, bottom, flags| Monitor {
            left,
            top: 0,
            right,
            bottom,
            flags,
        };
        let layout = Layout::new(
            &[screen(0, 1920, 1200), screen(1920, 1920, 1080)],
            &[
                monitor(0, 2559, 1439, MonitorFlags::PRIMARY),
                monitor(2560, 4479, 1079, MonitorFlags::empty()),
            ],
        );
        let regions = [
            Region {
                x: 0,
                y: 0,
                width: 1920,
                height: 1200,
            },
            Region {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ];
        let region = |screen: usize| regions[screen];
        assert_eq!(
            to_logical(&layout, (2560 + 960, 540), region),
            Some((2880.0, 540.0))
        );
        assert_eq!(
            to_logical(&layout, (128 + 1152, 720), region),
            Some((960.0, 600.0)),
            "the primary monitor shows the first screen, scaled"
        );
        assert_eq!(
            to_logical(&layout, (10, 10), region),
            None,
            "a letterbox bar"
        );
        assert_eq!(
            to_logical(&layout, (3000, 1200), region),
            None,
            "below the second monitor"
        );
    }
}
