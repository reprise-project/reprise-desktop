//! A screen capture client for tests: wlr-screencopy shots and ext-image-copy-capture sessions
//! of the first output, into shm buffers of the size the test gives.

use std::os::{fd::AsFd, unix::net::UnixStream};
use std::sync::Arc;

use wayland_client::{
    self as wc, Connection, EventQueue,
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    },
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use super::Harness;
use crate::compositor::ClientState;

#[derive(Default)]
struct Side {
    globals: Vec<(u32, String, u32)>,
}

pub struct CaptureClient {
    queue: EventQueue<Side>,
    side: Side,
    shm: wl_shm::WlShm,
    output: WlOutput,
    wlr: ZwlrScreencopyManagerV1,
    sources: ExtOutputImageCaptureSourceManagerV1,
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
    pub fn connect_capture(&mut self, (w, h): (i32, i32)) -> CaptureClient {
        let (server, client) = UnixStream::pair().expect("socket pair");
        let data = ClientState::of_stream(&server, None);
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
        let mut c = CaptureClient {
            queue,
            side,
            shm,
            output,
            wlr,
            sources,
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
wc::delegate_noop!(Side: ignore wl_shm_pool::WlShmPool);
wc::delegate_noop!(Side: ignore WlBuffer);
wc::delegate_noop!(Side: ignore WlOutput);
wc::delegate_noop!(Side: ignore ZwlrScreencopyManagerV1);
wc::delegate_noop!(Side: ignore ZwlrScreencopyFrameV1);
wc::delegate_noop!(Side: ignore ExtOutputImageCaptureSourceManagerV1);
wc::delegate_noop!(Side: ignore ExtImageCaptureSourceV1);
wc::delegate_noop!(Side: ignore ExtImageCopyCaptureManagerV1);
wc::delegate_noop!(Side: ignore ExtImageCopyCaptureSessionV1);
wc::delegate_noop!(Side: ignore ExtImageCopyCaptureFrameV1);
