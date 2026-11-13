//! A screen capture client for tests: wlr-screencopy shots and ext-image-copy-capture sessions
//! of the first output or a listed window, into shm buffers of the size the test gives.

use std::os::{fd::AsFd, unix::net::UnixStream};
use std::sync::Arc;

use wayland_client::{
    self as wc, Connection, EventQueue,
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use super::Harness;
use super::input_client::reprise_input_v1::RepriseInputV1;
use crate::compositor::ClientState;

#[derive(Default)]
struct Side {
    globals: Vec<(u32, String, u32)>,
    /// Every ext session `buffer_size`, oldest first.
    buffer_sizes: Vec<(u32, u32)>,
    /// The listed windows, oldest first.
    toplevels: Vec<ExtForeignToplevelHandleV1>,
}

pub struct CaptureClient {
    queue: EventQueue<Side>,
    side: Side,
    shm: wl_shm::WlShm,
    output: WlOutput,
    wlr: ZwlrScreencopyManagerV1,
    sources: ExtOutputImageCaptureSourceManagerV1,
    window_sources: ExtForeignToplevelImageCaptureSourceManagerV1,
    ext: ExtImageCopyCaptureManagerV1,
    size: (i32, i32),
}

/// An ext session and its frame in flight.
pub struct ExtSession {
    session: ExtImageCopyCaptureSessionV1,
    frame: Option<ExtImageCopyCaptureFrameV1>,
}

impl Harness {
    /// A trusted client capturing the first output, which is `w`x`h`.
    pub fn connect_capture(&mut self, size: (i32, i32)) -> CaptureClient {
        self.connect_capture_as(size, None)
    }

    /// The portal's capture connection for the remote desktop server, marked through
    /// `reprise_input_v1.remote_capture`.
    pub fn connect_remote_capture(&mut self, size: (i32, i32)) -> CaptureClient {
        self.connect_capture_as(size, Some(RepriseInputV1::remote_capture))
    }

    /// The portal's capture connection for the screen recorder, marked through
    /// `reprise_input_v1.recorder_capture`.
    pub fn connect_recorder_capture(&mut self, size: (i32, i32)) -> CaptureClient {
        self.connect_capture_as(size, Some(RepriseInputV1::recorder_capture))
    }

    /// A capture connection, marked as the portal's with `mark` when given.
    fn connect_capture_as(
        &mut self,
        (w, h): (i32, i32),
        mark: Option<fn(&RepriseInputV1)>,
    ) -> CaptureClient {
        let (server, client) = UnixStream::pair().expect("socket pair");
        let mut data = ClientState::of_stream(&server, None);
        if mark.is_some() {
            data.sandbox.pid = Some(crate::sandbox::TEST_PORTAL_PID);
        }
        self.state
            .dh
            .insert_client(server, Arc::new(data))
            .expect("client");
        let conn = Connection::from_socket(client).expect("connection");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut side = Side::default();
        roundtrip(self, &mut queue, &mut side);
        let global = |interface: &str, version: u32| {
            let &(name, _, v) = (side.globals.iter())
                .find(|g| g.1 == interface)
                .unwrap_or_else(|| panic!("{interface} is offered"));
            (name, v.min(version))
        };
        let (n, v) = global("wl_shm", 1);
        let shm = registry.bind(n, v, &qh, ());
        let (n, v) = global("wl_output", 1);
        let output = registry.bind(n, v, &qh, ());
        let (n, v) = global("zwlr_screencopy_manager_v1", 3);
        let wlr = registry.bind(n, v, &qh, ());
        let (n, v) = global("ext_output_image_capture_source_manager_v1", 1);
        let sources = registry.bind(n, v, &qh, ());
        let (n, v) = global("ext_image_copy_capture_manager_v1", 1);
        let ext = registry.bind(n, v, &qh, ());
        let (n, v) = global("ext_foreign_toplevel_list_v1", 1);
        let _: ExtForeignToplevelListV1 = registry.bind(n, v, &qh, ());
        let (n, v) = global("ext_foreign_toplevel_image_capture_source_manager_v1", 1);
        let window_sources = registry.bind(n, v, &qh, ());
        if let Some(mark) = mark {
            let (n, v) = global("reprise_input_v1", 11);
            let input: RepriseInputV1 = registry.bind(n, v, &qh, ());
            mark(&input);
            input.destroy();
        }
        let mut c = CaptureClient {
            queue,
            side,
            shm,
            output,
            wlr,
            sources,
            window_sources,
            ext,
            size: (w, h),
        };
        c.roundtrip(self);
        c
    }
}

fn roundtrip(h: &mut Harness, queue: &mut EventQueue<Side>, side: &mut Side) {
    for _ in 0..4 {
        queue.flush().expect("client flush");
        h.dispatch();
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        queue.dispatch_pending(side).expect("client dispatch");
    }
}

impl CaptureClient {
    pub fn roundtrip(&mut self, h: &mut Harness) {
        roundtrip(h, &mut self.queue, &mut self.side);
    }

    /// An Xrgb8888 shm buffer the output's size.
    fn buffer(&self) -> WlBuffer {
        let (w, h) = self.size;
        let size = w * h * 4;
        // SAFETY: plain syscall; the name is a valid C string.
        let raw = unsafe { libc::memfd_create(c"reprise-test".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(raw >= 0, "memfd");
        // SAFETY: `raw` is a fresh fd nothing else owns.
        let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw) };
        let file = std::fs::File::from(fd);
        file.set_len(size as u64).expect("memfd size");
        let qh = self.queue.handle();
        let pool = self.shm.create_pool(file.as_fd(), size, &qh, ());
        let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Xrgb8888, &qh, ());
        pool.destroy();
        buffer
    }

    /// One wlr-screencopy `copy` of the whole output, as a screenshot tool takes it.
    pub fn wlr_shot(&mut self, h: &mut Harness) {
        let qh = self.queue.handle();
        let frame = self.wlr.capture_output(0, &self.output, &qh, ());
        frame.copy(&self.buffer());
        self.roundtrip(h);
        frame.destroy();
        self.roundtrip(h);
    }

    /// An ext session on the output, no frame asked yet.
    pub fn ext_session(&mut self, h: &mut Harness) -> ExtSession {
        let qh = self.queue.handle();
        let source = self.sources.create_source(&self.output, &qh, ());
        self.session_of(h, source)
    }

    /// An ext session on the window listed last, no frame asked yet.
    pub fn window_session(&mut self, h: &mut Harness) -> ExtSession {
        self.roundtrip(h);
        let qh = self.queue.handle();
        let toplevel = self.side.toplevels.last().expect("a listed window");
        let source = self.window_sources.create_source(toplevel, &qh, ());
        self.session_of(h, source)
    }

    fn session_of(&mut self, h: &mut Harness, source: ExtImageCaptureSourceV1) -> ExtSession {
        let qh = self.queue.handle();
        let session = self.ext.create_session(&source, Options::empty(), &qh, ());
        source.destroy();
        self.roundtrip(h);
        ExtSession {
            session,
            frame: None,
        }
    }

    /// One frame of `s`: the previous one goes, a new one is captured.
    pub fn ext_frame(&mut self, h: &mut Harness, s: &mut ExtSession) {
        if let Some(f) = s.frame.take() {
            f.destroy();
        }
        let qh = self.queue.handle();
        let frame = s.session.create_frame(&qh, ());
        frame.attach_buffer(&self.buffer());
        let (w, hh) = self.size;
        frame.damage_buffer(0, 0, w, hh);
        frame.capture();
        s.frame = Some(frame);
        self.roundtrip(h);
    }

    /// The buffer sizes the ext sessions were told, oldest first.
    pub fn buffer_sizes(&self) -> &[(u32, u32)] {
        &self.side.buffer_sizes
    }

    pub fn ext_end(&mut self, h: &mut Harness, mut s: ExtSession) {
        if let Some(f) = s.frame.take() {
            f.destroy();
        }
        s.session.destroy();
        self.roundtrip(h);
    }
}

impl wc::Dispatch<wl_registry::WlRegistry, ()> for Side {
    fn event(
        st: &mut Self,
        _: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &wc::QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = ev
        {
            st.globals.push((name, interface, version));
        }
    }
}

wc::delegate_noop!(Side: ignore wl_shm::WlShm);
wc::delegate_noop!(Side: ignore RepriseInputV1);
wc::delegate_noop!(Side: ignore wl_shm_pool::WlShmPool);
wc::delegate_noop!(Side: ignore WlBuffer);
wc::delegate_noop!(Side: ignore WlOutput);
wc::delegate_noop!(Side: ignore ZwlrScreencopyManagerV1);
wc::delegate_noop!(Side: ignore ZwlrScreencopyFrameV1);
wc::delegate_noop!(Side: ignore ExtOutputImageCaptureSourceManagerV1);
wc::delegate_noop!(Side: ignore ExtImageCaptureSourceV1);
wc::delegate_noop!(Side: ignore ExtImageCopyCaptureManagerV1);
wc::delegate_noop!(Side: ignore ExtImageCopyCaptureFrameV1);
wc::delegate_noop!(Side: ignore ExtForeignToplevelHandleV1);
wc::delegate_noop!(Side: ignore ExtForeignToplevelImageCaptureSourceManagerV1);

impl wc::Dispatch<ExtForeignToplevelListV1, ()> for Side {
    fn event(
        st: &mut Self,
        _: &ExtForeignToplevelListV1,
        ev: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &wc::QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = ev {
            st.toplevels.push(toplevel);
        }
    }

    wc::event_created_child!(Side, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl wc::Dispatch<ExtImageCopyCaptureSessionV1, ()> for Side {
    fn event(
        st: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        ev: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &wc::QueueHandle<Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Event::BufferSize { width, height } = ev {
            st.buffer_sizes.push((width, height));
        }
    }
}
