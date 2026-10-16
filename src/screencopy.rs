//! wlr-screencopy-unstable-v1, the protocol side: frames of an output or
//! a region of it, into a client's shm or dmabuf buffer. `capture` does
//! the rendering; a frame here is failed if it is dropped unsubmitted.
//!
//! Follows niri's `src/protocols/screencopy.rs` (GPL-3.0-or-later),
//! without its screencast tracking: frames go straight to the handler.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use smithay::{
    backend::allocator::{Buffer, Fourcc, dmabuf::Dmabuf},
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::{wl_buffer::WlBuffer, wl_shm::Format},
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Size},
    wayland::{Dispatch2, GlobalDispatch2, dmabuf, shm},
};

const VERSION: u32 = 3;

/// The stride of an Xrgb8888 shm buffer `size` wide, as offered.
fn shm_stride(size: Size<i32, Physical>) -> i32 {
    size.w * 4
}

pub struct ScreencopyManagerState;

impl ScreencopyManagerState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrScreencopyManagerV1, ManagerGlobal> + 'static,
    {
        display.create_global::<D, ZwlrScreencopyManagerV1, _>(VERSION, ManagerGlobal);
        ScreencopyManagerState
    }
}

pub struct ManagerGlobal;
pub struct ManagerData;

impl<D> GlobalDispatch2<ZwlrScreencopyManagerV1, D> for ManagerGlobal
where
    D: Dispatch<ZwlrScreencopyManagerV1, ManagerData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrScreencopyManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ManagerData);
    }

    /// Hidden from sandboxed clients (`sandbox`).
    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "zwlr_screencopy_manager_v1")
    }
}

/// What a frame captures: the output, the region's top-left in its
/// physical pixels, the buffer size and whether the pointer is drawn.
#[derive(Clone)]
pub struct FrameInfo {
    pub output: Output,
    /// The region asked for, output-local logical; None for the whole output.
    pub region: Option<Rectangle<i32, Logical>>,
    pub region_loc: Point<i32, Physical>,
    pub buffer_size: Size<i32, Physical>,
    pub overlay_cursor: bool,
}

pub enum FrameState {
    Failed,
    Pending {
        info: FrameInfo,
        copied: Arc<AtomicBool>,
    },
}

impl<D> Dispatch2<ZwlrScreencopyManagerV1, D> for ManagerData
where
    D: Dispatch<ZwlrScreencopyFrameV1, FrameState> + 'static,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (frame, overlay_cursor, output, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                x,
                y,
                width,
                height,
                output,
            } => (
                frame,
                overlay_cursor,
                output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
            ),
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => unreachable!(),
        };
        let failed = |frame: New<ZwlrScreencopyFrameV1>, data_init: &mut DataInit<'_, D>| {
            data_init.init(frame, FrameState::Failed).failed();
        };
        let Some(output) = Output::from_resource(&output) else {
            return failed(frame, data_init);
        };
        let Some(mode) = output.current_mode() else {
            return failed(frame, data_init);
        };
        let (region_loc, buffer_size) = match region {
            None => (Point::from((0, 0)), mode.size),
            Some(rect) => {
                let transform = output.current_transform();
                let full = transform.transform_size(mode.size);
                let scale = output.current_scale().fractional_scale();
                let Some(clamped) = (rect.size.w > 0 && rect.size.h > 0)
                    .then(|| rect.to_physical_precise_round(scale))
                    .and_then(|r| r.intersection(Rectangle::from_size(full)))
                else {
                    return failed(frame, data_init);
                };
                let size = transform.invert().transform_rect_in(clamped, &full).size;
                (clamped.loc, size)
            }
        };
        let frame = data_init.init(
            frame,
            FrameState::Pending {
                info: FrameInfo {
                    output,
                    region,
                    region_loc,
                    buffer_size,
                    overlay_cursor: overlay_cursor != 0,
                },
                copied: Arc::new(AtomicBool::new(false)),
            },
        );
        frame.buffer(
            Format::Xrgb8888,
            buffer_size.w as u32,
            buffer_size.h as u32,
            shm_stride(buffer_size) as u32,
        );
        if frame.version() >= 3 {
            frame.linux_dmabuf(
                Fourcc::Xrgb8888 as u32,
                buffer_size.w as u32,
                buffer_size.h as u32,
            );
            frame.buffer_done();
        }
    }
}

/// Takes the frames clients asked to have copied.
pub trait ScreencopyHandler {
    /// Fill `screencopy` now or keep it for the output's next frame; a
    /// dropped one fails.
    fn screencopy_frame(&mut self, screencopy: Screencopy);
}

impl<D> Dispatch2<ZwlrScreencopyFrameV1, D> for FrameState
where
    D: ScreencopyHandler + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let FrameState::Pending { info, copied } = self else {
            return;
        };
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested",
            );
            return;
        }
        let size = info.buffer_size;
        let buffer = if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            if dmabuf.format().code == Fourcc::Xrgb8888
                && dmabuf.width() == size.w as u32
                && dmabuf.height() == size.h as u32
            {
                ScreencopyBuffer::Dmabuf(dmabuf.clone())
            } else {
                frame.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "invalid dmabuf parameters",
                );
                return;
            }
        } else if shm::with_buffer_contents(&buffer, |_, _, data| {
            data.format == Format::Xrgb8888
                && data.width == size.w
                && data.height == size.h
                && data.stride == shm_stride(size)
        })
        .unwrap_or(false)
        {
            ScreencopyBuffer::Shm(buffer)
        } else {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "invalid buffer",
            );
            return;
        };
        state.screencopy_frame(Screencopy {
            info: info.clone(),
            frame: frame.clone(),
            buffer,
            with_damage,
            submitted: false,
        });
    }
}

#[derive(Clone)]
pub enum ScreencopyBuffer {
    Dmabuf(Dmabuf),
    Shm(WlBuffer),
}

pub struct Screencopy {
    pub info: FrameInfo,
    frame: ZwlrScreencopyFrameV1,
    pub buffer: ScreencopyBuffer,
    /// `copy_with_damage`: wait for the output's next change.
    pub with_damage: bool,
    submitted: bool,
}

impl Drop for Screencopy {
    fn drop(&mut self) {
        if !self.submitted {
            self.frame.failed();
        }
    }
}

impl Screencopy {
    /// Tell the client the buffer holds the frame presented at `time`
    /// (monotonic), all of it damaged.
    pub fn submit(mut self, time: Duration) {
        let size = self.info.buffer_size;
        if self.with_damage {
            self.frame.damage(0, 0, size.w as u32, size.h as u32);
        }
        self.frame.flags(Flags::empty());
        let secs = time.as_secs();
        self.frame
            .ready((secs >> 32) as u32, secs as u32, time.subsec_nanos());
        self.submitted = true;
    }

    pub fn is_alive(&self) -> bool {
        self.frame.is_alive()
    }
}
