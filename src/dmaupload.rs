//! Big pictures copied into linear dma-bufs on the thread that drew them, which the renderer
//! imports as they stand: the loop neither copies the pixels nor uploads a texture.

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use reprisescene::{BufferId, Prepared};
use smithay::backend::{
    allocator::{
        Allocator, Buffer, Fourcc, Modifier,
        dmabuf::{AsDmabuf, Dmabuf, DmabufMappingMode, DmabufSyncFlags},
        gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
    },
    renderer::{ImportDma, gles::GlesRenderer},
};

use crate::{compositor::DeskScene, output::upload_scaled, raster::Canvas};

/// Fills dma-bufs off the loop on a GBM device of its own, opened on the render node at first
/// use. The first failure stops further tries (virtio-gpu refuses linear buffers).
#[derive(Clone)]
pub struct Uploads {
    node: PathBuf,
    alloc: Arc<OnceLock<Option<Mutex<GbmAllocator<File>>>>>,
    refused: Arc<AtomicBool>,
}

impl Uploads {
    pub fn new(node: PathBuf) -> Uploads {
        Uploads {
            node,
            alloc: Arc::default(),
            refused: Arc::default(),
        }
    }

    fn allocator(&self) -> Option<&Mutex<GbmAllocator<File>>> {
        self.alloc
            .get_or_init(|| {
                let open = File::options().read(true).write(true).open(&self.node);
                let device = open.and_then(GbmDevice::new);
                match device {
                    Ok(d) => Some(Mutex::new(GbmAllocator::new(d, GbmBufferFlags::RENDERING))),
                    Err(e) => {
                        log::warn!("uploads: no GBM device on {}: {e}", self.node.display());
                        None
                    }
                }
            })
            .as_ref()
    }

    /// `canvas` in a new linear ARGB8888 dma-buf; None when one cannot be made or mapped.
    pub fn fill(&self, canvas: &Canvas) -> Option<Dmabuf> {
        let (w, h) = (canvas.width, canvas.height);
        if w == 0 || h == 0 || canvas.pixels.len() != (w * h) as usize {
            return None;
        }
        if self.refused.load(Ordering::Relaxed) {
            return None;
        }
        self.try_fill(canvas)
            .map_err(|e| {
                if !self.refused.swap(true, Ordering::Relaxed) {
                    log::warn!("uploads: none from now on: {e}");
                }
            })
            .ok()
    }

    fn try_fill(&self, canvas: &Canvas) -> Result<Dmabuf, String> {
        let (w, h) = (canvas.width, canvas.height);
        let bo = {
            let alloc = self.allocator().ok_or("no GBM device")?;
            let mut alloc = alloc.lock().map_err(|_| "poisoned")?;
            (alloc.create_buffer(w, h, Fourcc::Argb8888, &[Modifier::Linear]))
                .map_err(|e| format!("no {w}x{h} buffer: {e}"))?
        };
        let dmabuf = bo.export().map_err(|e| format!("export failed: {e}"))?;
        let stride = dmabuf.strides().next().unwrap_or(0) as usize;
        let row = w as usize * 4;
        let map = (dmabuf.map_plane(0, DmabufMappingMode::WRITE))
            .map_err(|e| format!("map failed: {e}"))?;
        if stride < row || map.length() < stride * (h as usize - 1) + row {
            return Err(format!("stride {stride} too short for {w}x{h}"));
        }
        let _ = dmabuf.sync_plane(0, DmabufSyncFlags::START | DmabufSyncFlags::WRITE);
        for (y, src) in canvas.pixels.chunks_exact(w as usize).enumerate() {
            // SAFETY: the mapping holds `h` rows of `stride` bytes (checked above), this
            // thread is the buffer's only user, and ARGB8888 is the canvas's pixels in
            // little-endian byte order.
            unsafe {
                let dst = map.ptr().cast::<u8>().add(y * stride);
                std::ptr::copy_nonoverlapping(src.as_ptr().cast::<u8>(), dst, row);
            }
        }
        let _ = dmabuf.sync_plane(0, DmabufSyncFlags::END | DmabufSyncFlags::WRITE);
        Ok(dmabuf)
    }
}

/// A canvas made ready for a scene buffer off the loop: in a dma-buf when `Uploads` could fill
/// one, else copied into pixel memory.
pub enum Staged {
    Dmabuf(Dmabuf),
    Memory(Prepared),
}

impl Staged {
    pub fn new(uploads: Option<&Uploads>, canvas: &Canvas) -> Staged {
        match uploads.and_then(|u| u.fill(canvas)) {
            Some(d) => Staged::Dmabuf(d),
            None => Staged::Memory(Prepared::new(
                &canvas.pixels,
                canvas.width as i32,
                canvas.height as i32,
            )),
        }
    }

    pub fn size(&self) -> (i32, i32) {
        match self {
            Staged::Dmabuf(d) => {
                let s = d.size();
                (s.w, s.h)
            }
            Staged::Memory(p) => p.size(),
        }
    }

    /// Show it in `sb` at `scale`, as `upload_scaled` shows `canvas`, which it was made from
    /// and which shows instead when the renderer refuses the dma-buf.
    pub fn show(
        self,
        scene: &mut DeskScene,
        renderer: Option<&mut GlesRenderer>,
        sb: BufferId,
        canvas: &Canvas,
        scale: i32,
    ) {
        let scale = scale.max(1);
        let (w, h) = self.size();
        match self {
            Staged::Dmabuf(d) => match renderer.map(|r| (r.import_dmabuf(&d, None), r)) {
                Some((Ok(t), r)) => scene.buffer_set_texture(sb, r, t),
                Some((Err(e), _)) => {
                    log::warn!("uploads: import failed: {e}");
                    return upload_scaled(scene, sb, canvas, scale);
                }
                None => return upload_scaled(scene, sb, canvas, scale),
            },
            Staged::Memory(p) => scene.buffer_set_prepared(sb, p),
        }
        scene.buffer_set_dest_size(sb, Some((w / scale, h / scale).into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::{
        egl::EGLDevice,
        renderer::{Bind, ExportMem, Texture},
    };
    use smithay::utils::Rectangle;

    #[test]
    fn a_canvas_filled_on_another_thread_reads_back_through_the_renderer() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let node = EGLDevice::device_for_display(renderer.egl_context().display())
            .ok()
            .and_then(|d| d.try_get_render_node().ok().flatten())
            .and_then(|n| n.dev_path());
        let Some(node) = node else {
            eprintln!("skipped: no render node");
            return;
        };
        // An odd width, so a padded stride shows as sheared rows.
        let mut canvas = Canvas::new(37, 5);
        for (i, p) in canvas.pixels.iter_mut().enumerate() {
            *p = 0xff00_0000 | ((i as u32 * 0x0001_0305) & 0x00ff_ffff);
        }
        let uploads = Uploads::new(node);
        let expected = canvas.pixels.clone();
        let filled = std::thread::spawn(move || uploads.try_fill(&canvas));
        let dmabuf = match filled.join().unwrap() {
            Ok(d) => d,
            Err(e) => {
                // A virtual GPU may refuse linear buffers: they then show as copies.
                eprintln!("skipped: {e}");
                return;
            }
        };
        let mut texture = renderer.import_dmabuf(&dmabuf, None).unwrap();
        let size = texture.size();
        assert_eq!((size.w, size.h), (37, 5));
        let fb = renderer.bind(&mut texture).unwrap();
        let mapping = renderer
            .copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Argb8888)
            .unwrap();
        drop(fb);
        let bytes = renderer.map_texture(&mapping).unwrap();
        let read: Vec<u32> = (bytes.as_chunks::<4>().0.iter())
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        assert_eq!(read, expected);
    }
}
