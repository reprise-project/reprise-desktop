//! Linear storage for the textures of `wl_shm` surfaces.
//!
//! On Intel (iris) a GL-allocated texture is tiled and compressed, so every row copy goes
//! through a fresh staging buffer and a GPU blit. Once a surface commits a second buffer of
//! the same size into the same texture, that texture is re-backed with a linear dma-buf
//! (an EGLImage from a GBM buffer) and the renderer's row copies land straight in it. A
//! resizing window gets a new texture on every commit and is left alone until it settles.
//!
//! Only `Argb8888` and `Xrgb8888` buffers are re-backed: both are stored as `Argb8888`,
//! and an opaque buffer's alpha is ignored when drawn.

use smithay::backend::allocator::{
    Allocator, Fourcc, Modifier,
    dmabuf::AsDmabuf,
    gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::egl::{display::EGLDisplayHandle, ffi::egl};

type EGLImage = egl::types::EGLImage;
use smithay::backend::renderer::{
    BufferType, ContextId, Renderer, buffer_type,
    gles::{GlesRenderer, GlesTexture, ffi},
    utils::{CommitCounter, with_renderer_surface_state},
};
use smithay::reexports::wayland_server::{
    Resource, Weak, backend::ObjectId, protocol::wl_buffer::WlBuffer, protocol::wl_shm,
    protocol::wl_surface::WlSurface,
};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ShmTextures {
    allocator: GbmAllocator<DrmDeviceFd>,
    display: Arc<EGLDisplayHandle>,
    committed: HashMap<ObjectId, Weak<WlSurface>>,
    seen: HashMap<ObjectId, Seen>,
}

enum Seen {
    Plain(u32),
    Linear(Linear),
    Skip(u32),
}

enum Look {
    Gone,
    NotYet,
    Texture {
        texture: GlesTexture,
        buffer: WlBuffer,
        commit: CommitCounter,
        refilled: bool,
    },
}

struct Linear {
    surface: Weak<WlSurface>,
    texture: GlesTexture,
    image: EGLImage,
    display: Arc<EGLDisplayHandle>,
    commit: CommitCounter,
}

impl Drop for Linear {
    fn drop(&mut self) {
        // SAFETY: the image was created on this display and is not used
        // again.
        unsafe { egl::DestroyImageKHR(**self.display, self.image) };
    }
}

/// The linear format a buffer in `format` is re-backed with, if at all.
fn linear_format(format: wl_shm::Format) -> Option<Fourcc> {
    matches!(format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)
        .then_some(Fourcc::Argb8888)
}

impl ShmTextures {
    pub fn new(gbm: GbmDevice<DrmDeviceFd>, renderer: &GlesRenderer) -> Self {
        ShmTextures {
            allocator: GbmAllocator::new(gbm, GbmBufferFlags::RENDERING),
            display: renderer.egl_context().display().get_display_handle(),
            committed: HashMap::new(),
            seen: HashMap::new(),
        }
    }

    pub fn committed(&mut self, surface: &WlSurface) {
        self.committed
            .entry(surface.id())
            .or_insert_with(|| surface.downgrade());
    }

    /// Look at the textures of the surfaces that committed and were
    /// imported since, after a frame's elements are built and before it
    /// renders. Surfaces not imported yet stay for the next frame.
    pub fn update(&mut self, renderer: &mut GlesRenderer) {
        self.seen.retain(|_, s| match s {
            Seen::Linear(l) => l.surface.upgrade().is_ok(),
            _ => true,
        });
        if self.committed.is_empty() {
            return;
        }
        let ctx = renderer.context_id();
        let committed = std::mem::take(&mut self.committed);
        for (id, weak) in committed {
            let Ok(surface) = weak.upgrade() else {
                self.seen.remove(&id);
                continue;
            };
            match self.look(renderer, &ctx, &surface) {
                Some(()) => {}
                None => {
                    self.committed.insert(id, weak);
                }
            }
        }
    }

    fn look(
        &mut self,
        renderer: &mut GlesRenderer,
        ctx: &ContextId<GlesTexture>,
        surface: &WlSurface,
    ) -> Option<()> {
        let id = surface.id();
        let linear_commit = match self.seen.get(&id) {
            Some(Seen::Linear(l)) => Some((l.texture.tex_id(), l.commit)),
            _ => None,
        };
        let looked = with_renderer_surface_state(surface, |st| {
            let Some(buffer) = st.buffer() else {
                return Look::Gone;
            };
            let Some(texture) = st.texture::<GlesTexture>(ctx.clone()) else {
                return Look::NotYet;
            };
            let commit = st.current_commit();
            // A new buffer under no damage makes the renderer specify the
            // texture anew, which drops the linear storage.
            let refilled = linear_commit.is_some_and(|(tex, at)| {
                tex == texture.tex_id() && at != commit && st.damage_since(Some(at)).is_empty()
            });
            Look::Texture {
                texture: texture.clone(),
                buffer: (**buffer).clone(),
                commit,
                refilled,
            }
        });
        let (texture, buffer, commit, refilled) = match looked {
            None | Some(Look::Gone) => {
                self.seen.remove(&id);
                return Some(());
            }
            Some(Look::NotYet) => return None,
            Some(Look::Texture {
                texture,
                buffer,
                commit,
                refilled,
            }) => (texture, buffer, commit, refilled),
        };
        let tex = texture.tex_id();
        if !matches!(buffer_type(&buffer), Some(BufferType::Shm)) {
            self.seen.remove(&id);
            return Some(());
        }
        match self.seen.get_mut(&id) {
            Some(Seen::Linear(l)) if l.texture.tex_id() == tex && !refilled => {
                l.commit = commit;
                return Some(());
            }
            Some(Seen::Skip(t)) if *t == tex => return Some(()),
            // The second commit into one texture, or a refilled one.
            Some(Seen::Plain(t)) if *t == tex => {}
            Some(Seen::Linear(_)) if refilled => {}
            _ => {
                self.seen.insert(id, Seen::Plain(tex));
                return Some(());
            }
        }
        let seen = match self.back(renderer, surface, &buffer, texture, commit) {
            Some(l) => Seen::Linear(l),
            None => Seen::Skip(tex),
        };
        self.seen.insert(id, seen);
        Some(())
    }

    fn back(
        &mut self,
        renderer: &mut GlesRenderer,
        surface: &WlSurface,
        buffer: &WlBuffer,
        texture: GlesTexture,
        commit: CommitCounter,
    ) -> Option<Linear> {
        let display = self.display.clone();
        let allocator = &mut self.allocator;
        smithay::wayland::shm::with_buffer_contents(buffer, |ptr, len, data| {
            let fourcc = linear_format(data.format)?;
            let (w, h) = (data.width, data.height);
            if w <= 0
                || h <= 0
                || data.stride < w * 4
                || (data.offset + data.stride * h) as usize > len
            {
                return None;
            }
            let bo = allocator
                .create_buffer(w as u32, h as u32, fourcc, &[Modifier::Linear])
                .map_err(|e| log::warn!("shm texture: allocation failed: {e}"))
                .ok()?;
            let dmabuf = bo
                .export()
                .map_err(|e| log::warn!("shm texture: export failed: {e}"))
                .ok()?;
            let image = renderer
                .egl_context()
                .display()
                .create_image_from_dmabuf(&dmabuf)
                .map_err(|e| log::warn!("shm texture: import failed: {e}"))
                .ok()?;
            let linear = Linear {
                surface: surface.downgrade(),
                texture,
                image,
                display,
                commit,
            };
            let tex = linear.texture.tex_id();
            let (stride, offset) = (data.stride / 4, data.offset as usize);
            let backed = renderer
                .with_context(|gl| unsafe {
                    // SAFETY: the context is current; `ptr` maps the
                    // buffer's rows for this call (checked above).
                    gl.GetError();
                    let copied = copy_into(gl, tex, image, w, h);
                    gl.BindTexture(ffi::TEXTURE_2D, tex);
                    gl.EGLImageTargetTexture2DOES(ffi::TEXTURE_2D, image);
                    let backed = gl.GetError() == ffi::NO_ERROR;
                    if copied || !backed {
                        gl.BindTexture(ffi::TEXTURE_2D, 0);
                        return backed;
                    }
                    gl.PixelStorei(ffi::UNPACK_ROW_LENGTH, stride);
                    gl.TexSubImage2D(
                        ffi::TEXTURE_2D,
                        0,
                        0,
                        0,
                        w,
                        h,
                        ffi::BGRA_EXT,
                        ffi::UNSIGNED_BYTE,
                        ptr.add(offset) as *const _,
                    );
                    gl.PixelStorei(ffi::UNPACK_ROW_LENGTH, 0);
                    gl.BindTexture(ffi::TEXTURE_2D, 0);
                    backed
                })
                .ok()?;
            if !backed {
                log::warn!("shm texture: binding the linear image failed");
                return None;
            }
            Some(linear)
        })
        .ok()
        .flatten()
    }
}

/// Blit texture `from` into `image` on the GPU, sparing the main thread a full upload; false
/// when it could not.
///
/// # Safety
/// The context is current and `image` was created on its display.
unsafe fn copy_into(gl: &ffi::Gles2, from: u32, image: EGLImage, w: i32, h: i32) -> bool {
    if !gl.BlitFramebuffer.is_loaded() {
        return false;
    }
    // SAFETY: the caller's contract; every name made here is deleted before returning and the
    // bindings and scissor state it changes are restored.
    unsafe {
        let (mut read, mut draw) = (0, 0);
        gl.GetIntegerv(ffi::READ_FRAMEBUFFER_BINDING, &mut read);
        gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut draw);
        let scissor = gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE;
        let mut to = 0;
        gl.GenTextures(1, &mut to);
        gl.BindTexture(ffi::TEXTURE_2D, to);
        gl.EGLImageTargetTexture2DOES(ffi::TEXTURE_2D, image);
        let mut fbs = [0; 2];
        gl.GenFramebuffers(2, fbs.as_mut_ptr());
        gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, fbs[0]);
        gl.FramebufferTexture2D(
            ffi::READ_FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            from,
            0,
        );
        gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbs[1]);
        gl.FramebufferTexture2D(
            ffi::DRAW_FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            to,
            0,
        );
        let complete = gl.CheckFramebufferStatus(ffi::READ_FRAMEBUFFER)
            == ffi::FRAMEBUFFER_COMPLETE
            && gl.CheckFramebufferStatus(ffi::DRAW_FRAMEBUFFER) == ffi::FRAMEBUFFER_COMPLETE;
        if complete {
            if scissor {
                gl.Disable(ffi::SCISSOR_TEST);
            }
            gl.BlitFramebuffer(0, 0, w, h, 0, 0, w, h, ffi::COLOR_BUFFER_BIT, ffi::NEAREST);
            if scissor {
                gl.Enable(ffi::SCISSOR_TEST);
            }
        }
        gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, read as u32);
        gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, draw as u32);
        gl.DeleteFramebuffers(2, fbs.as_ptr());
        gl.BindTexture(ffi::TEXTURE_2D, 0);
        gl.DeleteTextures(1, &to);
        complete && gl.GetError() == ffi::NO_ERROR
    }
}
