//! The textures an shm capture renders into and the pixel-pack buffers they are read into, kept
//! between captures so a screencast does not allocate and free two GPU buffers on the loop each
//! frame. A slot sits in the pool only while no capture holds it, so a capture still in flight
//! never shares its buffer.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Offscreen,
            gles::{GlesRenderer, GlesTexture, ffi},
        },
    },
    utils::{Physical, Size, Transform},
};

/// How long a free slot is kept before it is freed.
pub const IDLE: Duration = Duration::from_secs(5);
/// Free slots kept at most; past it the longest free goes.
const KEPT: usize = 4;

/// What a slot fits: one is reused only for a capture of the same size and format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    pub size: Size<i32, Physical>,
    pub fourcc: Fourcc,
}

/// Free items by key, oldest first.
pub struct Pool<T> {
    free: Vec<(Key, T, Instant)>,
}

impl<T> Default for Pool<T> {
    fn default() -> Pool<T> {
        Pool { free: Vec::new() }
    }
}

impl<T> Pool<T> {
    /// The item freed last for `key`.
    pub fn take(&mut self, key: Key) -> Option<T> {
        let i = self.free.iter().rposition(|(k, ..)| *k == key)?;
        Some(self.free.remove(i).1)
    }

    pub fn give(&mut self, key: Key, item: T, now: Instant) {
        self.free.push((key, item, now));
        if self.free.len() > KEPT {
            self.free.remove(0);
        }
    }

    /// Drops the items free for `IDLE` by `now`; when the next of the rest is due.
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.free
            .retain(|(_, _, at)| now.saturating_duration_since(*at) < IDLE);
        self.free.iter().map(|(_, _, at)| *at + IDLE).min()
    }
}

/// Buffer names dropped without the renderer, deleted the next time it is at hand.
type Dead = Rc<RefCell<Vec<u32>>>;

struct Pbo {
    id: u32,
    dead: Dead,
}

impl Drop for Pbo {
    fn drop(&mut self) {
        self.dead.borrow_mut().push(self.id);
    }
}

/// A texture to render a capture into and a buffer its pixels are read into.
pub struct Slot {
    key: Key,
    texture: GlesTexture,
    pbo: Pbo,
}

#[derive(Default)]
pub struct Readbacks {
    pool: Pool<Slot>,
    dead: Dead,
    expiring: bool,
}

impl Readbacks {
    /// A free slot for `key`, or a new one.
    pub fn slot(&mut self, renderer: &mut GlesRenderer, key: Key) -> Result<Slot, String> {
        self.bury(renderer)?;
        match self.pool.take(key) {
            Some(slot) => Ok(slot),
            None => Slot::new(renderer, key, &self.dead),
        }
    }

    /// Return `slot` once its capture is read: the GPU is done with it.
    pub fn give(&mut self, slot: Slot, now: Instant) {
        self.pool.give(slot.key, slot, now);
    }

    /// Frees the slots idle for `IDLE`; when the next is due, if any is left.
    pub fn expire(&mut self, renderer: &mut GlesRenderer, now: Instant) -> Option<Instant> {
        let next = self.pool.expire(now);
        if let Err(e) = self.bury(renderer) {
            log::warn!("capture buffers: {e}");
        }
        next
    }

    /// Whether an expiry timer has to be started: true once until `disarm`.
    pub fn arm(&mut self) -> bool {
        !std::mem::replace(&mut self.expiring, true)
    }

    pub fn disarm(&mut self) {
        self.expiring = false;
    }

    fn bury(&mut self, renderer: &mut GlesRenderer) -> Result<(), String> {
        let ids = std::mem::take(&mut *self.dead.borrow_mut());
        if ids.is_empty() {
            return Ok(());
        }
        renderer
            .with_context(|gl| {
                // SAFETY: the context is current inside `with_context`; the names are buffers of it
                // that nothing uses any more.
                unsafe { gl.DeleteBuffers(ids.len() as i32, ids.as_ptr()) }
            })
            .map_err(|e| e.to_string())
    }
}

impl Slot {
    fn new(renderer: &mut GlesRenderer, key: Key, dead: &Dead) -> Result<Slot, String> {
        let Key { size, fourcc } = key;
        if size.w <= 0 || size.h <= 0 {
            return Err(format!("no capture of {}x{}", size.w, size.h));
        }
        let texture: GlesTexture = renderer
            .create_buffer(fourcc, size.to_logical(1).to_buffer(1, Transform::Normal))
            .map_err(|e| e.to_string())?;
        let len = size.w as isize * size.h as isize * 4;
        let id = renderer
            .with_context(|gl| {
                let mut id = 0;
                // SAFETY: the context is current inside `with_context`; the pack binding goes back
                // to none, as the renderer leaves it.
                unsafe {
                    gl.GenBuffers(1, &mut id);
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, id);
                    gl.BufferData(
                        ffi::PIXEL_PACK_BUFFER,
                        len,
                        std::ptr::null(),
                        ffi::STREAM_DRAW,
                    );
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
                }
                id
            })
            .map_err(|e| e.to_string())?;
        Ok(Slot {
            key,
            texture,
            pbo: Pbo {
                id,
                dead: Rc::clone(dead),
            },
        })
    }

    pub fn texture(&mut self) -> &mut GlesTexture {
        &mut self.texture
    }

    /// Queues the read of the texture into the buffer, as 4 bytes a pixel in the slot's format,
    /// rows from the top. The bytes are there once the renderer's next fence is reached.
    pub fn read(&self, renderer: &mut GlesRenderer) -> Result<(), String> {
        let Size { w, h, .. } = self.key.size;
        let (tex, pbo) = (self.texture.tex_id(), self.pbo.id);
        let error = renderer
            .with_context(|gl| {
                // SAFETY: the context is current inside `with_context`; `tex` and `pbo` are live
                // objects of it, the buffer holds `w * h` pixels of 4 bytes, and the framebuffer
                // and pack bindings go back to none.
                unsafe {
                    gl.GetError();
                    let mut fbo = 0;
                    gl.GenFramebuffers(1, &mut fbo);
                    gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);
                    gl.FramebufferTexture2D(
                        ffi::FRAMEBUFFER,
                        ffi::COLOR_ATTACHMENT0,
                        ffi::TEXTURE_2D,
                        tex,
                        0,
                    );
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, pbo);
                    gl.ReadBuffer(ffi::COLOR_ATTACHMENT0);
                    gl.ReadPixels(
                        0,
                        0,
                        w,
                        h,
                        ffi::BGRA_EXT,
                        ffi::UNSIGNED_BYTE,
                        std::ptr::null_mut(),
                    );
                    gl.ReadBuffer(ffi::NONE);
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
                    gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
                    gl.DeleteFramebuffers(1, &fbo);
                    gl.GetError()
                }
            })
            .map_err(|e| e.to_string())?;
        match error {
            ffi::NO_ERROR => Ok(()),
            e => Err(format!("reading the capture back failed: GL error {e:#x}")),
        }
    }

    /// Runs `with` on the bytes `read` left in the buffer. The GPU must have finished the read:
    /// mapping an unfinished one waits for it.
    pub fn bytes<R>(
        &self,
        renderer: &mut GlesRenderer,
        with: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, String> {
        let len = self.key.size.w as usize * self.key.size.h as usize * 4;
        let pbo = self.pbo.id;
        renderer
            .with_context(|gl| {
                // SAFETY: the context is current inside `with_context`; `pbo` holds `len` bytes,
                // the mapping is read only while mapped, and the pack binding goes back to none.
                unsafe {
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, pbo);
                    let ptr = gl.MapBufferRange(
                        ffi::PIXEL_PACK_BUFFER,
                        0,
                        len as isize,
                        ffi::MAP_READ_BIT,
                    );
                    let out = (!ptr.is_null()).then(|| {
                        let out = with(std::slice::from_raw_parts(ptr as *const u8, len));
                        gl.UnmapBuffer(ffi::PIXEL_PACK_BUFFER);
                        out
                    });
                    gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
                    out
                }
            })
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the capture buffer could not be mapped".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(w: i32, h: i32) -> Key {
        Key {
            size: (w, h).into(),
            fourcc: Fourcc::Xrgb8888,
        }
    }

    #[test]
    fn a_freed_item_is_taken_again_for_the_same_key() {
        let mut pool = Pool::default();
        let t = Instant::now();
        assert_eq!(pool.take(key(4, 4)), None);
        pool.give(key(4, 4), 1, t);
        pool.give(key(4, 4), 2, t);
        assert_eq!(pool.take(key(4, 4)), Some(2), "the one freed last");
        assert_eq!(pool.take(key(4, 4)), Some(1));
        assert_eq!(pool.take(key(4, 4)), None, "taken items leave the pool");
    }

    #[test]
    fn an_item_is_kept_for_its_size_and_format_only() {
        let mut pool = Pool::default();
        let t = Instant::now();
        pool.give(key(4, 4), 1, t);
        assert_eq!(pool.take(key(8, 4)), None, "another size");
        let argb = Key {
            fourcc: Fourcc::Argb8888,
            ..key(4, 4)
        };
        assert_eq!(pool.take(argb), None, "another format");
        assert_eq!(pool.take(key(4, 4)), Some(1));
    }

    #[test]
    fn items_idle_past_the_limit_are_dropped() {
        let mut pool = Pool::default();
        let t = Instant::now();
        pool.give(key(4, 4), 1, t);
        pool.give(key(8, 8), 2, t + Duration::from_secs(1));
        assert_eq!(pool.expire(t + IDLE / 2), Some(t + IDLE), "none due yet");
        assert_eq!(pool.free.len(), 2);
        assert_eq!(
            pool.expire(t + IDLE),
            Some(t + Duration::from_secs(1) + IDLE),
            "the first is freed, the second due a second later"
        );
        assert_eq!(pool.take(key(4, 4)), None);
        assert_eq!(pool.expire(t + Duration::from_secs(1) + IDLE), None);
        assert_eq!(pool.free.len(), 0);
    }

    #[test]
    fn past_the_cap_the_longest_free_goes() {
        let mut pool = Pool::default();
        let t = Instant::now();
        for i in 0..=KEPT as i32 {
            pool.give(key(i + 1, 1), i, t);
        }
        assert_eq!(pool.free.len(), KEPT);
        assert_eq!(pool.take(key(1, 1)), None, "the first freed is gone");
        assert_eq!(pool.take(key(2, 1)), Some(1));
    }

    fn renderer() -> Option<GlesRenderer> {
        match reprisefx::headless::renderer() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("skipped: {e}");
                None
            }
        }
    }

    fn fill(r: &mut GlesRenderer, slot: &mut Slot, colour: [f32; 4]) {
        use smithay::backend::renderer::{Bind, Frame, Renderer};
        use smithay::utils::Rectangle;
        let size = slot.key.size;
        let mut fb = r.bind(slot.texture()).unwrap();
        let mut frame = r.render(&mut fb, size, Transform::Normal).unwrap();
        frame
            .clear(colour.into(), &[Rectangle::from_size(size)])
            .unwrap();
        let _ = frame.finish().unwrap();
    }

    fn read(r: &mut GlesRenderer, slot: &Slot) -> Vec<u8> {
        slot.read(r).unwrap();
        let fence = crate::gpufence::fence_now(r).unwrap();
        assert!(crate::gpufence::reached_within(
            &fence,
            Duration::from_secs(5)
        ));
        slot.bytes(r, <[u8]>::to_vec).unwrap()
    }

    #[test]
    fn a_reused_slot_reads_the_new_capture() {
        let Some(mut r) = renderer() else {
            return;
        };
        let mut readbacks = Readbacks::default();
        let k = key(3, 2);
        let mut slot = readbacks.slot(&mut r, k).unwrap();
        fill(&mut r, &mut slot, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            read(&mut r, &slot),
            [0, 0, 255, 255].repeat(6),
            "red as B, G, R, A"
        );
        let pbo = slot.pbo.id;
        readbacks.give(slot, Instant::now());

        let mut slot = readbacks.slot(&mut r, k).unwrap();
        assert_eq!(slot.pbo.id, pbo, "the freed slot comes back");
        fill(&mut r, &mut slot, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(read(&mut r, &slot), [255, 0, 0, 255].repeat(6), "blue");
    }

    #[test]
    fn a_slot_reads_what_copy_framebuffer_reads() {
        use smithay::backend::renderer::{Bind, ExportMem};
        use smithay::utils::Rectangle;
        let Some(mut r) = renderer() else {
            return;
        };
        let mut readbacks = Readbacks::default();
        let k = Key {
            size: (5, 3).into(),
            fourcc: Fourcc::Argb8888,
        };
        let mut slot = readbacks.slot(&mut r, k).unwrap();
        fill(&mut r, &mut slot, [0.25, 0.5, 0.75, 0.5]);
        let ours = read(&mut r, &slot);
        let fb = r.bind(slot.texture()).unwrap();
        let mapping = r
            .copy_framebuffer(&fb, Rectangle::from_size((5, 3).into()), k.fourcc)
            .unwrap();
        drop(fb);
        let theirs = r.map_texture(&mapping).unwrap().to_vec();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn a_dropped_slot_frees_its_buffer_on_the_next_take() {
        let Some(mut r) = renderer() else {
            return;
        };
        let mut readbacks = Readbacks::default();
        let slot = readbacks.slot(&mut r, key(2, 2)).unwrap();
        drop(slot);
        assert_eq!(readbacks.dead.borrow().len(), 1);
        let _slot = readbacks.slot(&mut r, key(2, 2)).unwrap();
        assert!(readbacks.dead.borrow().is_empty());
    }
}
