use std::ops::RangeInclusive;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::raw::c_int;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use reis::eis;
use reis::enumflags2::BitFlags;
use reis::request::{Device, DeviceCapability, EisRequest, EisRequestConverter, Seat};
use smithay::{
    backend::input::{Axis, AxisSource, ButtonState, InputTime},
    input::{keyboard::Keycode, pointer::AxisFrame},
    reexports::calloop::{
        Interest, Mode, PostAction, RegistrationToken,
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Point},
};

use crate::compositor::State;
use crate::inputcapture::CaptureId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sender,
    Receiver,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EisId(u64);

pub struct Devices {
    pub pointer: Option<Device>,
    pub pointer_abs: Option<Device>,
    pub keyboard: Option<Device>,
    caps: BitFlags<DeviceCapability>,
}

impl Devices {
    pub fn iter(&self) -> impl Iterator<Item = &Device> {
        [&self.pointer, &self.pointer_abs, &self.keyboard]
            .into_iter()
            .flatten()
    }

    fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Take every device out of the client's device map. A set is always
    /// removed whole: leaving one behind would let the next bind stack a
    /// second set on top of it.
    fn remove_all(&self) {
        for d in self.iter() {
            d.remove();
        }
    }
}

enum Phase {
    Handshake(reis::handshake::EisHandshaker),
    Connected(EisRequestConverter),
}

pub struct EisClient {
    id: EisId,
    pub kind: Kind,
    allowed: BitFlags<DeviceCapability>,
    pub capture: Option<CaptureId>,
    context: eis::Context,
    relay: Relay,
    phase: Phase,
    seat: Option<Seat>,
    devices: Option<Devices>,
    pressed_keys: Vec<u32>,
    pressed_buttons: Vec<u32>,
    source: RegistrationToken,
    writer: Option<RegistrationToken>,
    retry: Option<RegistrationToken>,
    /// The last `pump` stopped at `BATCH`, so reis may still hold parsed
    /// requests; the socket stays unread until they are handled.
    backlog: bool,
}

impl EisClient {
    pub fn devices(&self) -> Option<&Devices> {
        self.devices.as_ref()
    }

    fn write_out(&mut self) -> std::io::Result<bool> {
        loop {
            let reis_done = match self.context.flush() {
                Ok(()) => true,
                Err(rustix::io::Errno::AGAIN) => false,
                Err(e) => return Err(e.into()),
            };
            if !self.relay.forward()? {
                return Ok(false);
            }
            if reis_done {
                return Ok(true);
            }
        }
    }
}

#[derive(Default)]
pub struct EisState {
    clients: Vec<EisClient>,
    next_id: u64,
}

impl EisState {
    pub fn get(&self, id: EisId) -> Option<&EisClient> {
        self.clients.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: EisId) -> Option<&mut EisClient> {
        self.clients.iter_mut().find(|c| c.id == id)
    }
}

/// Forwards reis's handshake_version at once: an ei client says nothing until
/// it has it, so no read would ever trigger the forward.
fn open(
    stream: UnixStream,
) -> Result<(Relay, eis::Context, reis::handshake::EisHandshaker), String> {
    let (mut relay, theirs) = Relay::new(stream).map_err(|e| format!("eis relay: {e}"))?;
    let context = eis::Context::new(theirs).map_err(|e| format!("eis context: {e}"))?;
    let handshaker = reis::handshake::EisHandshaker::new(&context, 1);
    relay.forward().map_err(|e| format!("eis handshake: {e}"))?;
    Ok((relay, context, handshaker))
}

/// From Linux 6.16's `asm-generic/socket.h`; libc does not export it yet.
const SO_PASSRIGHTS: c_int = 83;

/// Make the peer's `sendmsg` fail with EPERM when it carries SCM_RIGHTS: reis keeps
/// every fd a peer sends until the context drops, and no ei request takes one.
/// A kernel without the option fails here and the client is refused.
fn refuse_fds(sock: &UnixStream) -> std::io::Result<()> {
    let off: c_int = 0;
    // SAFETY: `sock` is an open socket and `off` a live c_int of the given size.
    let r = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_PASSRIGHTS,
            &off as *const c_int as *const libc::c_void,
            size_of::<c_int>() as libc::socklen_t,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

const READ_CAP: usize = 16 * 1024;

/// Sits between the client's socket and reis, which reads until EAGAIN: a fast sender
/// would otherwise keep one read going without end. reis gets one end of a
/// socketpair instead, fed at most `READ_CAP` bytes per `fill`.
struct Relay {
    peer: UnixStream,
    inner: UnixStream,
    out: Vec<u8>,
    /// The fds that go with the first byte of `out`.
    out_fds: Vec<OwnedFd>,
}

impl Relay {
    fn new(peer: UnixStream) -> std::io::Result<(Relay, UnixStream)> {
        let (inner, theirs) = UnixStream::pair()?;
        peer.set_nonblocking(true)?;
        inner.set_nonblocking(true)?;
        let relay = Relay {
            peer,
            inner,
            out: Vec::new(),
            out_fds: Vec::new(),
        };
        Ok((relay, theirs))
    }

    fn fill(&self, context: &eis::Context) -> std::io::Result<usize> {
        use std::io::{ErrorKind, Read, Write};
        let mut buf = [0u8; READ_CAP];
        let n = match (&self.peer).read(&mut buf) {
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut off = 0;
        while off < n {
            match (&self.inner).write(&buf[off..n]) {
                Ok(k) => off += k,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if context.read()? == 0 {
                        return Err(ErrorKind::WouldBlock.into());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        context.read()?;
        Ok(n)
    }

    fn forward(&mut self) -> std::io::Result<bool> {
        use rustix::io::Errno;
        use rustix::net::{
            RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
            SendAncillaryMessage, SendFlags,
        };
        use std::io::{IoSlice, IoSliceMut};
        use std::mem::MaybeUninit;
        loop {
            if self.out.is_empty() {
                let mut buf = [0u8; 4096];
                let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(32))];
                let mut cmsg = RecvAncillaryBuffer::new(&mut space);
                let got = match rustix::net::recvmsg(
                    &self.inner,
                    &mut [IoSliceMut::new(&mut buf)],
                    &mut cmsg,
                    RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
                ) {
                    Ok(r) => r.bytes,
                    Err(Errno::AGAIN) => return Ok(true),
                    Err(e) => return Err(e.into()),
                };
                if got == 0 {
                    return Ok(true);
                }
                self.out.extend_from_slice(&buf[..got]);
                for m in cmsg.drain() {
                    if let RecvAncillaryMessage::ScmRights(fds) = m {
                        self.out_fds.extend(fds);
                    }
                }
            }
            let fds: Vec<_> = self.out_fds.iter().map(|f| f.as_fd()).collect();
            let mut space = vec![MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(fds.len()))];
            let mut cmsg = SendAncillaryBuffer::new(&mut space);
            if !fds.is_empty() {
                cmsg.push(SendAncillaryMessage::ScmRights(&fds));
            }
            match rustix::net::sendmsg(
                &self.peer,
                &[IoSlice::new(&self.out)],
                &mut cmsg,
                SendFlags::NOSIGNAL | SendFlags::DONTWAIT,
            ) {
                Ok(n) => {
                    self.out.drain(..n);
                    self.out_fds.clear();
                }
                Err(Errno::AGAIN) => return Ok(false),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// Requests one `pump` handles. A sender that writes faster than that
/// leaves the rest for a later loop iteration, so outputs and other clients
/// still get their turn.
const BATCH: usize = 256;

fn connected(
    client: &mut EisClient,
    resp: reis::handshake::EisHandshakeResp,
) -> Result<(), String> {
    let want = match client.kind {
        Kind::Sender => eis::handshake::ContextType::Sender,
        Kind::Receiver => eis::handshake::ContextType::Receiver,
    };
    if resp.context_type != want {
        return Err(format!(
            "client asked for {:?}, portal said {:?}",
            resp.context_type, client.kind
        ));
    }
    let conv = EisRequestConverter::new(&client.context, resp, 1);
    let handle = conv.handle().clone();
    if !handle.has_interface("ei_seat") || !handle.has_interface("ei_device") {
        return Err("client lacks ei_seat or ei_device".into());
    }
    client.phase = Phase::Connected(conv);
    client.seat = Some(handle.add_seat(Some("default"), client.allowed));
    log::info!(
        "eis: {:?} client {:?} connected",
        client.kind,
        handle.name()
    );
    Ok(())
}

impl State {
    pub fn eis_connect(
        &mut self,
        fd: OwnedFd,
        kind: Kind,
        capture: Option<CaptureId>,
        devices: u32,
    ) -> Result<EisId, String> {
        let stream = UnixStream::from(fd);
        refuse_fds(&stream).map_err(|e| format!("cannot refuse fds on the EIS socket: {e}"))?;
        let (relay, context, handshaker) = open(stream)?;
        let watched = relay
            .peer
            .try_clone()
            .map_err(|e| format!("cannot watch the EIS socket: {e}"))?;
        let st = &mut self.inputproto.eis;
        st.next_id += 1;
        let id = EisId(st.next_id);
        let source = self
            .handle
            .insert_source(
                Generic::new(watched, Interest::READ, Mode::Level),
                move |ready, _, state: &mut State| {
                    if ready.error {
                        state.eis_disconnect(id);
                    } else if ready.readable {
                        state.eis_service(id);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| format!("cannot watch the EIS socket: {}", e.error))?;
        self.inputproto.eis.clients.push(EisClient {
            id,
            kind,
            allowed: allowed_caps(devices),
            capture,
            context,
            relay,
            phase: Phase::Handshake(handshaker),
            seat: None,
            devices: None,
            pressed_keys: Vec::new(),
            pressed_buttons: Vec::new(),
            source,
            writer: None,
            retry: None,
            backlog: false,
        });
        log::info!("eis: new {kind:?} client");
        Ok(id)
    }

    pub fn eis_disconnect(&mut self, id: EisId) {
        let Some(c) = self.inputproto.eis.get(id) else {
            return;
        };
        let capture = c.capture.filter(|_| c.kind == Kind::Receiver);
        self.eis_release_held(id);
        if let Some(cap) = capture {
            self.capture_receiver_gone(cap);
        }
        let clients = &mut self.inputproto.eis.clients;
        let Some(i) = clients.iter().position(|c| c.id == id) else {
            return;
        };
        let mut c = clients.remove(i);
        for t in [Some(c.source), c.writer, c.retry].into_iter().flatten() {
            self.handle.remove(t);
        }
        if let Phase::Connected(conv) = &c.phase {
            conv.handle()
                .disconnected(eis::connection::DisconnectReason::Disconnected, None);
            let _ = c.relay.forward();
        }
        log::info!("eis: {:?} client gone", c.kind);
    }

    fn eis_release_held(&mut self, id: EisId) {
        let Some(c) = self.inputproto.eis.get_mut(id) else {
            return;
        };
        let buttons = std::mem::take(&mut c.pressed_buttons);
        let keys = std::mem::take(&mut c.pressed_keys);
        let time = InputTime::now();
        let was = std::mem::replace(&mut self.inputproto.injecting, true);
        for b in buttons {
            self.handle_button(b, ButtonState::Released, time);
        }
        for k in keys {
            self.handle_key(evdev_key(k), false, time);
        }
        self.inputproto.injecting = was;
    }

    fn eis_service(&mut self, id: EisId) {
        if let Err(e) = self.eis_pump(id) {
            log::info!("eis: {e}");
            self.eis_disconnect(id);
        }
    }

    /// Read up to `READ_CAP` bytes and act on up to `BATCH` requests. Past
    /// the budget the socket stays unread, so the sender blocks, and a
    /// timer fires in 1 ms for the rest. An error means the client is done.
    fn eis_pump(&mut self, id: EisId) -> Result<(), String> {
        let Some(c) = self.inputproto.eis.get_mut(id) else {
            return Ok(());
        };
        if !c.backlog {
            c.relay.fill(&c.context).map_err(|e| format!("read: {e}"))?;
        }
        let mut budget = BATCH;
        let backlog = loop {
            if budget == 0 {
                break true;
            }
            budget -= 1;
            let Some(c) = self.inputproto.eis.get_mut(id) else {
                return Ok(());
            };
            let queued = match &mut c.phase {
                // What the converter made of earlier requests goes first.
                Phase::Connected(conv) => conv.next_request(),
                Phase::Handshake(_) => None,
            };
            if let Some(r) = queued {
                if let EisRequest::Disconnect = r {
                    return Err("client disconnected".into());
                }
                self.eis_handle(id, r);
                continue;
            }
            let Some(result) = c.context.pending_request() else {
                break false;
            };
            let req = match (result, &mut c.phase) {
                (reis::PendingRequestResult::Request(r), _) => r,
                (reis::PendingRequestResult::ParseError(e), _) => {
                    return Err(format!("parse: {e}"));
                }
                (reis::PendingRequestResult::InvalidObject(id), Phase::Handshake(_)) => {
                    return Err(format!("invalid object {id} in handshake"));
                }
                (reis::PendingRequestResult::InvalidObject(id), Phase::Connected(_)) => {
                    log::debug!("eis: unknown object {id}");
                    continue;
                }
            };
            let done = match &mut c.phase {
                Phase::Handshake(hs) => hs
                    .handle_request(req)
                    .map_err(|e| format!("handshake: {e}"))?,
                Phase::Connected(conv) => {
                    conv.handle_request(req)
                        .map_err(|e| format!("request: {e}"))?;
                    None
                }
            };
            if let Some(resp) = done {
                connected(c, resp)?;
            }
        };
        let Some(c) = self.inputproto.eis.get_mut(id) else {
            return Ok(());
        };
        c.backlog = backlog;
        if backlog && c.retry.is_none() {
            let timer = Timer::from_duration(Duration::from_millis(1));
            let retry = self
                .handle
                .insert_source(timer, move |_, _, state: &mut State| {
                    if let Some(c) = state.inputproto.eis.get_mut(id) {
                        c.retry = None;
                        state.eis_service(id);
                    }
                    TimeoutAction::Drop
                });
            c.retry = retry.ok();
        }
        self.eis_flush(id);
        Ok(())
    }

    pub fn eis_flush(&mut self, id: EisId) -> bool {
        let Some(c) = self.inputproto.eis.get_mut(id) else {
            return false;
        };
        let result = c.write_out();
        if matches!(result, Ok(false))
            && c.writer.is_none()
            && let Ok(watched) = c.relay.peer.try_clone()
        {
            let writer = self.handle.insert_source(
                Generic::new(watched, Interest::WRITE, Mode::Level),
                move |_, _, state: &mut State| Ok(state.eis_writable(id)),
            );
            c.writer = writer.ok();
        }
        result.unwrap_or(false)
    }

    fn eis_writable(&mut self, id: EisId) -> PostAction {
        let Some(c) = self.inputproto.eis.get_mut(id) else {
            return PostAction::Remove;
        };
        if let Ok(false) = c.write_out() {
            return PostAction::Continue;
        }
        c.writer = None;
        PostAction::Remove
    }

    fn eis_handle(&mut self, id: EisId, r: EisRequest) {
        match r {
            EisRequest::Bind(b) => {
                let Some(c) = self.inputproto.eis.get_mut(id) else {
                    return;
                };
                // A bind replaces the last one's set: libei's unbind is a
                // bind with no capabilities, and a re-bind with different
                // ones must not leave the old devices behind.
                if let Some(d) = c.devices.take() {
                    d.remove_all();
                }
                let caps = b.capabilities & c.allowed;
                let devices = self.eis_make_devices(&b.seat, caps);
                if let Some(c) = self.inputproto.eis.get_mut(id) {
                    c.devices = (!devices.is_empty()).then_some(devices);
                }
            }
            EisRequest::DeviceClosed(_) => {
                if let Some(d) = self
                    .inputproto
                    .eis
                    .get_mut(id)
                    .and_then(|c| c.devices.take())
                {
                    d.remove_all();
                }
            }
            other => self.eis_inject(id, other),
        }
    }

    /// `injecting` is set throughout: a RemoteDesktop sender drives this desktop,
    /// so it reaches local clients even while an InputCapture holds the devices.
    fn eis_inject(&mut self, id: EisId, r: EisRequest) {
        let Some(c) = self.inputproto.eis.get(id) else {
            return;
        };
        if c.kind != Kind::Sender {
            log::debug!("eis: receiver sent {r:?}");
            return;
        }
        crate::idle::notify_activity(self);
        let was = std::mem::replace(&mut self.inputproto.injecting, true);
        self.eis_inject_inner(id, r);
        self.inputproto.injecting = was;
    }

    fn eis_inject_inner(&mut self, id: EisId, r: EisRequest) {
        let time = InputTime::now();
        match r {
            EisRequest::PointerMotion(m) => {
                let delta: Point<f64, Logical> = (m.dx as f64, m.dy as f64).into();
                let pos = self.pointer_location() + delta;
                self.pointer_to(pos, time);
            }
            EisRequest::PointerMotionAbsolute(m) => {
                let pos = (m.dx_absolute as f64, m.dy_absolute as f64).into();
                self.pointer_to(pos, time);
            }
            EisRequest::Button(b) => {
                let pressed = b.state == eis::button::ButtonState::Press;
                let Some(c) = self.inputproto.eis.get_mut(id) else {
                    return;
                };
                if !track(&mut c.pressed_buttons, BUTTONS, b.button, pressed) {
                    log::debug!("eis: ignoring button {:#x}", b.button);
                    return;
                }
                let state = if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                self.handle_button(b.button, state, time);
            }
            EisRequest::ScrollDelta(s) => {
                let mut frame = AxisFrame::new(time).source(AxisSource::Continuous);
                if s.dx != 0.0 {
                    frame = frame.value(Axis::Horizontal, s.dx as f64);
                }
                if s.dy != 0.0 {
                    frame = frame.value(Axis::Vertical, s.dy as f64);
                }
                self.eis_axis(frame);
            }
            EisRequest::ScrollDiscrete(s) => {
                // libei's discrete unit is 120 per notch, the same as
                // Wayland's v120.
                let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
                for (axis, d) in [
                    (Axis::Horizontal, s.discrete_dx),
                    (Axis::Vertical, s.discrete_dy),
                ] {
                    if d != 0 {
                        frame = frame.value(axis, d as f64 / 120.0 * 15.0).v120(axis, d);
                    }
                }
                self.eis_axis(frame);
            }
            EisRequest::ScrollStop(_) | EisRequest::ScrollCancel(_) => {
                // A sender's scroll carries no finger source, so a stop
                // has nothing to end.
            }
            EisRequest::KeyboardKey(k) => {
                let pressed = k.state == eis::keyboard::KeyState::Press;
                let Some(c) = self.inputproto.eis.get_mut(id) else {
                    return;
                };
                if !track(&mut c.pressed_keys, KEYS, k.key, pressed) {
                    log::debug!("eis: ignoring keycode {}", k.key);
                    return;
                }
                self.handle_key(evdev_key(k.key), pressed, time);
            }
            // Every motion, button and scroll above ends its own frame.
            EisRequest::Frame(_) => {}
            other => log::debug!("eis: ignoring {other:?}"),
        }
    }

    fn eis_axis(&mut self, frame: AxisFrame) {
        if frame.axis == (0.0, 0.0) {
            return;
        }
        self.handle_axis(frame);
    }

    fn eis_make_devices(&mut self, seat: &Seat, caps: BitFlags<DeviceCapability>) -> Devices {
        let plan = device_plan(caps);
        let pointer = plan.pointer.map(|rel| {
            seat.add_device(
                Some("Reprise pointer"),
                eis::device::DeviceType::Virtual,
                rel,
                |_| {},
            )
        });
        let pointer_abs = plan.pointer_abs.map(|abs| {
            let outputs: Vec<(
                String,
                smithay::utils::Rectangle<i32, smithay::utils::Logical>,
                f32,
            )> = self
                .scene
                .outputs()
                .iter()
                .filter_map(|o| {
                    let r = reprisescene::output_rect(o)?;
                    let scale = o.current_scale().fractional_scale() as f32;
                    Some((o.name(), r, scale))
                })
                .filter(|(name, r, _)| {
                    // libei regions are unsigned, and ScreenCast streams share these
                    // mapping ids: an output at a negative coordinate gets no region.
                    if r.loc.x < 0 || r.loc.y < 0 {
                        log::warn!(
                            "eis: output {name} at ({}, {}) has no region",
                            r.loc.x,
                            r.loc.y
                        );
                        return false;
                    }
                    r.size.w > 0 && r.size.h > 0
                })
                .collect();
            seat.add_device(
                Some("Reprise absolute pointer"),
                eis::device::DeviceType::Virtual,
                abs,
                |dev| {
                    for (name, r, scale) in &outputs {
                        dev.device().region_mapping_id(name);
                        dev.device().region(
                            r.loc.x as u32,
                            r.loc.y as u32,
                            r.size.w as u32,
                            r.size.h as u32,
                            *scale,
                        );
                    }
                },
            )
        });
        let keyboard = plan.keyboard.then(|| {
            let keymap = self.keymap_fd();
            seat.add_device(
                Some("Reprise keyboard"),
                eis::device::DeviceType::Virtual,
                DeviceCapability::Keyboard.into(),
                |dev| {
                    if let (Some((fd, size)), Some(kb)) =
                        (&keymap, dev.interface::<eis::Keyboard>())
                    {
                        kb.keymap(eis::keyboard::KeymapType::Xkb, *size, fd.as_fd());
                    }
                },
            )
        });
        let devices = Devices {
            pointer,
            pointer_abs,
            keyboard,
            caps,
        };
        for d in devices.iter() {
            d.resumed();
        }
        devices
    }

    /// The size counts the trailing NUL: the client hands the mapping straight to
    /// `xkb_keymap_new_from_string`.
    fn keymap_fd(&mut self) -> Option<(OwnedFd, u32)> {
        use smithay::input::keyboard::xkb;
        let keyboard = self.seat.get_keyboard()?;
        let text = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // SAFETY: the keymap is only read here; no reference to it outlives the lock.
            unsafe { xkb.keymap() }.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
        });
        let mut bytes = text.into_bytes();
        bytes.push(0);
        let fd = sealed_memfd(&bytes)?;
        Some((fd, bytes.len() as u32))
    }

    /// libei sends a keymap and regions once, at device creation, so a keymap or
    /// layout change needs fresh devices.
    pub fn eis_readvertise(&mut self) {
        let ids: Vec<EisId> = self.inputproto.eis.clients.iter().map(|c| c.id).collect();
        for id in ids {
            let Some(c) = self.inputproto.eis.get_mut(id) else {
                continue;
            };
            let Some(seat) = c.seat.clone() else {
                continue;
            };
            if let Some(d) = c.devices.take() {
                let caps = d.caps;
                let capture = c.capture.filter(|_| c.kind == Kind::Receiver);
                d.remove_all();
                let devices = self.eis_make_devices(&seat, caps);
                if let Some(c) = self.inputproto.eis.get_mut(id) {
                    c.devices = Some(devices);
                }
                if let Some(cap) = capture {
                    self.capture_devices_replaced(cap);
                }
            }
            self.eis_flush(id);
        }
    }
}

pub(crate) fn evdev_key(code: u32) -> Keycode {
    Keycode::new(code + 8)
}

pub(crate) fn sealed_memfd(bytes: &[u8]) -> Option<OwnedFd> {
    let name = c"reprise-keymap";
    // SAFETY: `name` is a NUL-terminated C string literal.
    let raw =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if raw < 0 {
        return None;
    }
    // SAFETY: memfd_create succeeded, so `raw` is an open fd nothing else owns.
    let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
    std::io::Write::write_all(&mut file, bytes).ok()?;
    let fd = OwnedFd::from(file);
    // SAFETY: `raw` is open, owned by `fd`.
    let sealed = unsafe {
        libc::fcntl(
            raw,
            libc::F_ADD_SEALS,
            libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE | libc::F_SEAL_SEAL,
        )
    };
    if sealed < 0 {
        return None;
    }
    Some(fd)
}

pub(crate) const KEYS: RangeInclusive<u32> = 0..=0x2ff;
const BUTTONS: RangeInclusive<u32> = 0x100..=0x117;
const MAX_HELD: usize = 32;

pub(crate) fn track(
    held: &mut Vec<u32>,
    valid: RangeInclusive<u32>,
    code: u32,
    pressed: bool,
) -> bool {
    if !valid.contains(&code) {
        return false;
    }
    if !pressed {
        held.retain(|&x| x != code);
    } else if !held.contains(&code) {
        if held.len() >= MAX_HELD {
            return false;
        }
        held.push(code);
    }
    true
}

pub fn allowed_caps(devices: u32) -> BitFlags<DeviceCapability> {
    use crate::inputproto::{DEVICE_KEYBOARD, DEVICE_POINTER};
    let mut caps = BitFlags::empty();
    if devices & DEVICE_KEYBOARD != 0 {
        caps |= DeviceCapability::Keyboard;
    }
    if devices & DEVICE_POINTER != 0 {
        caps |= DeviceCapability::Pointer
            | DeviceCapability::PointerAbsolute
            | DeviceCapability::Button
            | DeviceCapability::Scroll;
    }
    caps
}

#[derive(Debug, PartialEq, Eq)]
struct DevicePlan {
    pointer: Option<BitFlags<DeviceCapability>>,
    pointer_abs: Option<BitFlags<DeviceCapability>>,
    keyboard: bool,
}

fn device_plan(caps: BitFlags<DeviceCapability>) -> DevicePlan {
    let rel =
        caps & (DeviceCapability::Pointer | DeviceCapability::Button | DeviceCapability::Scroll);
    let abs = caps
        & (DeviceCapability::PointerAbsolute | DeviceCapability::Button | DeviceCapability::Scroll);
    DevicePlan {
        pointer: (!rel.is_empty()).then_some(rel),
        pointer_abs: caps
            .contains(DeviceCapability::PointerAbsolute)
            .then_some(abs),
        keyboard: caps.contains(DeviceCapability::Keyboard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_byte(sock: &UnixStream, fd: Option<c_int>) -> std::io::Result<()> {
        let mut byte = [0u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr() as *mut libc::c_void,
            iov_len: 1,
        };
        let mut cmsg = [0u64; 4];
        // SAFETY: an all-zero msghdr is valid; every pointer set below outlives
        // the sendmsg call, and the cmsg buffer is large enough for one fd.
        let r = unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            if let Some(fd) = fd {
                msg.msg_control = cmsg.as_mut_ptr() as *mut libc::c_void;
                msg.msg_controllen = libc::CMSG_SPACE(size_of::<c_int>() as u32) as usize;
                let h = libc::CMSG_FIRSTHDR(&msg);
                (*h).cmsg_level = libc::SOL_SOCKET;
                (*h).cmsg_type = libc::SCM_RIGHTS;
                (*h).cmsg_len = libc::CMSG_LEN(size_of::<c_int>() as u32) as usize;
                std::ptr::write_unaligned(libc::CMSG_DATA(h) as *mut c_int, fd);
            }
            libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL)
        };
        if r < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Counted by link name, so fds other tests open meanwhile do not count.
    fn fds_to(target: &std::path::Path) -> usize {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
            .filter(|l| l == target)
            .count()
    }

    #[test]
    fn an_eis_socket_refuses_fds_from_its_peer() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        refuse_fds(&ours).unwrap();
        let context = eis::Context::new(ours).unwrap();
        let name = c"eis-refuses-fds";
        // SAFETY: `name` is a NUL-terminated C string literal.
        let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        assert!(raw >= 0, "memfd_create");
        // SAFETY: `raw` is a fresh fd nothing else owns.
        let payload = unsafe { OwnedFd::from_raw_fd(raw) };
        let target = std::fs::read_link(format!("/proc/self/fd/{}", payload.as_raw_fd())).unwrap();
        for _ in 0..64 {
            let err = send_byte(&theirs, Some(payload.as_raw_fd())).unwrap_err();
            assert_eq!(err.raw_os_error(), Some(libc::EPERM));
        }
        send_byte(&theirs, None).unwrap();
        assert_eq!(context.read().unwrap(), 1);
        assert_eq!(fds_to(&target), 1, "only the payload's own fd is open");
    }

    fn relay() -> (UnixStream, Relay, eis::Context) {
        let (client, ours) = UnixStream::pair().unwrap();
        let (relay, theirs) = Relay::new(ours).unwrap();
        (client, relay, eis::Context::new(theirs).unwrap())
    }

    #[test]
    fn a_fill_hands_reis_at_most_read_cap_bytes() {
        use std::io::Write;
        let (mut client, relay, context) = relay();
        client.write_all(&[0u8; 3 * READ_CAP]).unwrap();
        drop(client);
        for _ in 0..3 {
            assert_eq!(relay.fill(&context).unwrap(), READ_CAP);
            assert_eq!(context.read().unwrap(), 0, "the pair holds nothing more");
        }
        let eof = relay.fill(&context).unwrap_err();
        assert_eq!(eof.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    fn recv_with_fds(sock: &UnixStream) -> (usize, Vec<OwnedFd>) {
        use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags};
        let mut buf = vec![0u8; 64 * 1024];
        let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
        let mut cmsg = RecvAncillaryBuffer::new(&mut space);
        let r = rustix::net::recvmsg(
            sock,
            &mut [std::io::IoSliceMut::new(&mut buf)],
            &mut cmsg,
            RecvFlags::DONTWAIT,
        )
        .unwrap();
        let mut fds = Vec::new();
        for m in cmsg.drain() {
            if let RecvAncillaryMessage::ScmRights(f) = m {
                fds.extend(f);
            }
        }
        (r.bytes, fds)
    }

    #[test]
    fn a_new_client_gets_the_handshake_without_speaking_first() {
        let (client, ours) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let _open = open(ours).unwrap();
        let mut buf = [0u8; 64];
        let n = std::io::Read::read(&mut &client, &mut buf)
            .expect("handshake_version waits on the client's socket");
        assert!(n >= 16, "one whole ei message, got {n} bytes");
    }

    #[test]
    fn a_forward_passes_bytes_and_fds_to_the_client() {
        let (client, ours) = UnixStream::pair().unwrap();
        let (mut relay, theirs) = Relay::new(ours).unwrap();
        let (a, _b) = UnixStream::pair().unwrap();
        send_byte(&theirs, Some(a.as_raw_fd())).unwrap();
        assert!(relay.forward().unwrap());
        let (n, fds) = recv_with_fds(&client);
        assert_eq!(n, 1);
        assert_eq!(fds.len(), 1, "one fd arrives with the byte");
    }

    #[test]
    fn a_full_client_socket_keeps_the_rest_until_it_has_room() {
        use std::io::{ErrorKind, Write};
        let (client, ours) = UnixStream::pair().unwrap();
        let (mut relay, mut theirs) = Relay::new(ours).unwrap();
        theirs.set_nonblocking(true).unwrap();
        let chunk = [7u8; 4096];
        let mut written = 0;
        loop {
            match theirs.write(&chunk) {
                Ok(n) => written += n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if !relay.forward().unwrap() {
                        break;
                    }
                }
                Err(e) => panic!("{e}"),
            }
        }
        assert!(!relay.forward().unwrap(), "the client's socket is full");
        let mut received = 0;
        while received < written {
            received += recv_with_fds(&client).0;
            relay.forward().unwrap();
        }
        assert_eq!(received, written);
        assert!(relay.forward().unwrap());
    }

    #[test]
    fn a_keymap_memfd_is_sealed() {
        let fd = sealed_memfd(b"xkb_keymap {};\0").unwrap();
        // SAFETY: `fd` is an open memfd.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        let all = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE | libc::F_SEAL_SEAL;
        assert_eq!(seals & all, all);
        let size = std::fs::File::from(fd).metadata().unwrap().len();
        assert_eq!(size, 15);
    }

    #[test]
    fn a_pointer_only_bind_gets_the_relative_pointer_alone() {
        let plan = device_plan(
            DeviceCapability::Pointer | DeviceCapability::Button | DeviceCapability::Scroll,
        );
        assert_eq!(
            plan.pointer,
            Some(DeviceCapability::Pointer | DeviceCapability::Button | DeviceCapability::Scroll)
        );
        assert_eq!(plan.pointer_abs, None);
        assert!(!plan.keyboard);
    }

    #[test]
    fn a_keyboard_only_bind_gets_no_pointer() {
        let plan = device_plan(DeviceCapability::Keyboard.into());
        assert_eq!(plan.pointer, None);
        assert_eq!(plan.pointer_abs, None);
        assert!(plan.keyboard);
    }

    /// A pointer-only grant never yields a keyboard, whatever the client
    /// binds; the mask is enforced, not reported.
    #[test]
    fn the_grant_mask_caps_what_a_bind_gets() {
        let pointer_only = allowed_caps(crate::inputproto::DEVICE_POINTER);
        assert!(!pointer_only.contains(DeviceCapability::Keyboard));
        let asked: BitFlags<DeviceCapability> =
            DeviceCapability::Keyboard | DeviceCapability::Pointer | DeviceCapability::Button;
        let plan = device_plan(asked & pointer_only);
        assert!(!plan.keyboard);
        assert!(plan.pointer.is_some());
        assert_eq!(allowed_caps(0), BitFlags::empty());
        assert!(
            allowed_caps(crate::inputproto::DEVICE_KEYBOARD | crate::inputproto::DEVICE_POINTER)
                .contains(DeviceCapability::PointerAbsolute | DeviceCapability::Keyboard)
        );
    }

    #[test]
    fn an_empty_bind_gets_nothing() {
        let plan = device_plan(BitFlags::empty());
        assert_eq!(plan.pointer, None);
        assert_eq!(plan.pointer_abs, None);
        assert!(!plan.keyboard);
    }

    #[test]
    fn an_absolute_bind_carries_the_button_and_scroll_caps() {
        let plan = device_plan(DeviceCapability::PointerAbsolute | DeviceCapability::Button);
        assert_eq!(plan.pointer, Some(DeviceCapability::Button.into()));
        assert_eq!(
            plan.pointer_abs,
            Some(DeviceCapability::PointerAbsolute | DeviceCapability::Button)
        );
        assert!(!plan.keyboard);
    }

    #[test]
    fn a_key_outside_key_max_is_dropped() {
        let mut held = Vec::new();
        assert!(!track(&mut held, KEYS, 0x300, true));
        assert!(!track(&mut held, KEYS, u32::MAX, true));
        assert!(held.is_empty());
        assert!(track(&mut held, KEYS, 30, true));
        assert_eq!(held, [30]);
    }

    #[test]
    fn a_button_outside_the_mouse_range_is_dropped() {
        let mut held = Vec::new();
        assert!(!track(&mut held, BUTTONS, 0xff, true));
        assert!(!track(&mut held, BUTTONS, 0x118, true));
        assert!(track(&mut held, BUTTONS, 0x110, true));
        assert!(track(&mut held, BUTTONS, 0x110, false));
        assert!(held.is_empty());
    }

    #[test]
    fn held_keys_stop_at_32_and_a_release_frees_a_slot() {
        let mut held = Vec::new();
        for code in 0..MAX_HELD as u32 {
            assert!(track(&mut held, KEYS, code, true));
        }
        assert!(!track(&mut held, KEYS, 100, true), "a 33rd key is dropped");
        assert!(
            track(&mut held, KEYS, 0, true),
            "a repeat press of a held key passes"
        );
        assert_eq!(held.len(), MAX_HELD);
        assert!(track(&mut held, KEYS, 0, false));
        assert!(track(&mut held, KEYS, 100, true));
        assert_eq!(held.len(), MAX_HELD);
    }
}
