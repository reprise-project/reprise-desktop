//! H.264 encoders for stream frames, through FFmpeg on the backend the probe finds. Callers run
//! them on a thread of their own, so a frame on the GPU never holds up anything else.

pub mod avc444;
pub mod choice;
pub mod colour;
pub mod cuda;
pub mod dmabuf;
pub mod ffmpeg;
pub mod gl;
pub mod h264merge;
pub mod hwmap;
pub mod probe;
pub mod split;

use crate::stream::{DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Rect};
use hwmap::Gpu;
use probe::Backend;
use repriseconf::remote::Quality;
use std::sync::OnceLock;

pub trait Encoder {
    /// What changed in `frame`, or all of it on a `keyframe`.
    fn encode(&mut self, frame: &Frame, keyframe: bool) -> Result<Encoded, EncodeError>;

    /// The quality to code at from now on, for an encoder coded at constant QP.
    fn set_quality(&mut self, _quality: Quality) {}

    /// Code both AVC444 views from the next frame on, or AVC420 only.
    fn set_avc444(&mut self, _on: bool) {}

    /// Code pictures scaled to `target` from the next frame on; None codes them at their own size.
    fn set_target(&mut self, _target: Option<(u32, u32)>) {}
}

pub enum Encoded {
    /// One H.264 access unit in Annex-B, and the parts of the picture it changes.
    Avc420 {
        data: Vec<u8>,
        regions: Vec<Rect>,
    },
    /// One picture's AVC444 views in Annex-B, and the parts of the picture they change. The
    /// auxiliary view is left out where the main view's chroma is already the picture's.
    Avc444 {
        main: Vec<u8>,
        aux: Option<Vec<u8>>,
        regions: Vec<Rect>,
        layout: avc444::AuxLayout,
    },
    Nothing,
}

#[derive(Clone, Debug)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The cores the CPU conversions of every screen share, at most 8.
pub fn cpu_threads() -> usize {
    static CORES: OnceLock<usize> = OnceLock::new();
    *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(8)))
}

/// The threads a `size` picture's CPU conversion or scaling is split across.
pub fn cpu_threads_for(size: (u32, u32)) -> usize {
    threads_for(cpu_threads(), size)
}

/// A thread's share of a picture: about 2 ms of conversion on one core. Smaller shares, or a
/// split on two cores, cost more CPU in thread hand-offs than they save in time.
const PIXELS_PER_THREAD: u64 = 1_000_000;

fn threads_for(cores: usize, (w, h): (u32, u32)) -> usize {
    match cores {
        ..=2 => 1,
        _ => (u64::from(w) * u64::from(h) / PIXELS_PER_THREAD).clamp(1, cores as u64) as usize,
    }
}

/// Whether `backend` (None: the bitmap encoder) can be handed `frame`: the CPU reads shared
/// memory and LINEAR dma-bufs, a GPU dma-bufs only, VA-API LINEAR ones in [`split::PITCH_ALIGN`].
pub fn reads(backend: Option<Backend>, frame: &Frame) -> bool {
    match (&frame.buffer, backend.and_then(Backend::gpu)) {
        (FrameBuffer::Shm { .. }, gpu) => gpu.is_none(),
        (FrameBuffer::Dmabuf { modifier, .. }, None) => *modifier == DRM_FORMAT_MOD_LINEAR,
        (
            FrameBuffer::Dmabuf {
                modifier, planes, ..
            },
            Some(Gpu::Vaapi),
        ) if *modifier == DRM_FORMAT_MOD_LINEAR => {
            (planes.iter()).all(|p| p.stride.is_multiple_of(split::PITCH_ALIGN))
        }
        (FrameBuffer::Dmabuf { .. }, Some(_)) => true,
    }
}

/// [`reads`], and for a dma-buf its modifier one of the `modifiers` `backend` maps: a stream may
/// still deliver buffers of an older offer.
pub fn reads_offered(backend: Option<Backend>, modifiers: &[u64], frame: &Frame) -> bool {
    reads(backend, frame)
        && match &frame.buffer {
            FrameBuffer::Dmabuf { modifier, .. } => modifiers.contains(modifier),
            FrameBuffer::Shm { .. } => true,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dma_buf_of_a_modifier_no_longer_offered_is_not_read() {
        const Y_TILED: u64 = 0x0100_0000_0000_0002;
        let mut tiled = Frame::detached(Vec::new(), (4, 2), Vec::new());
        tiled.buffer = FrameBuffer::Dmabuf {
            objects: vec![std::sync::Arc::new(
                std::fs::File::open("/dev/null").unwrap().into(),
            )],
            fourcc: crate::stream::XRGB8888,
            modifier: Y_TILED,
            planes: Vec::new(),
        };
        let vulkan = Some(Backend::Vulkan);
        assert!(reads_offered(
            vulkan,
            &[DRM_FORMAT_MOD_LINEAR, Y_TILED],
            &tiled
        ));
        assert!(!reads_offered(vulkan, &[DRM_FORMAT_MOD_LINEAR], &tiled));
        let shm = Frame::detached(vec![0; 4 * 2 * 4], (4, 2), Vec::new());
        assert!(reads_offered(Some(Backend::X264), &[], &shm));
    }

    #[test]
    fn conversions_split_only_large_pictures_on_more_than_two_cores() {
        assert_eq!(threads_for(2, (3840, 2160)), 1, "two cores: one thread");
        assert_eq!(threads_for(8, (1024, 768)), 1, "a small one: one thread");
        assert_eq!(threads_for(8, (1920, 1080)), 2, "a million pixels a thread");
        assert_eq!(threads_for(4, (3840, 2160)), 4, "at most the cores");
        assert_eq!(threads_for(8, (0, 0)), 1);
    }

    #[test]
    fn an_unscaled_target_adds_no_pass() {
        use hwmap::Gpu;
        let (source, half) = ((1920, 1080), (960, 540));
        let cut = |gpu: Gpu, origin, source, target| {
            let (name, options) = gpu.to_nv12(origin, source, target).unwrap();
            format!("{name}={}", options.to_str().unwrap())
        };
        let options = |gpu: Gpu, target| cut(gpu, (0, 0), source, target);
        assert_eq!(
            options(Gpu::Vaapi, source),
            "scale_vaapi=format=nv12:out_color_matrix=bt709:out_range=full:\
             out_color_primaries=bt709:out_color_transfer=iec61966-2-1"
        );
        assert_eq!(
            options(Gpu::Vulkan, source),
            "libplacebo=format=nv12:colorspace=bt709:range=pc:dithering=none"
        );
        assert_eq!(
            options(Gpu::Vaapi, half),
            format!("{}:w=960:h=540", options(Gpu::Vaapi, source)),
            "the conversion scales in the same filter"
        );
        assert_eq!(
            options(Gpu::Vulkan, half),
            format!("{}:w=960:h=540", options(Gpu::Vulkan, source))
        );
        assert_eq!(
            cut(Gpu::Vaapi, (101, 51), (1061, 591), (960, 540)),
            options(Gpu::Vaapi, half),
            "VA-API reads the part from each frame's crop"
        );
        assert_eq!(
            cut(Gpu::Vulkan, (101, 51), (1061, 591), (960, 540)),
            format!(
                "{}:crop_x=101:crop_y=51:crop_w=960:crop_h=540",
                options(Gpu::Vulkan, half)
            ),
            "libplacebo is told the part, at its odd origin"
        );
        for pass in 0..split::PASSES {
            assert_eq!(
                split::fragment(pass, false, avc444::AuxLayout::V1),
                format!("#version 300 es\n#define PASS {pass}\n{}", split::FRAGMENT),
                "pass {pass} reads the picture texel by texel"
            );
            assert!(
                split::fragment(pass, true, avc444::AuxLayout::V1).contains("#define SCALED\n")
            );
        }
    }
}
