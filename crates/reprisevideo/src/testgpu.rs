//! What a hardware test needs from the GPU. On a host without it the test prints why and
//! passes, so an `--ignored` run passes on any host.

use crate::encode::Codec;
use crate::encode::gl::Gl;
use crate::encode::hwmap::{Devices, Gpu};
use crate::encode::probe::{self, Backend, RENDER_NODE};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(crate) enum Need {
    /// OpenGL ES on the render node and a LINEAR dma-buf from its GBM device.
    Gl,
    /// The API's device on the render node.
    Device(Gpu),
    /// A VA-API H.264 encoder that passes the probe.
    VaapiH264,
    /// An NVENC H.264 encoder that codes the probe's test frame.
    Nvenc,
}

/// Whether this host lacks `need`, after printing why; the calling test then returns.
pub(crate) fn lacks(need: Need) -> bool {
    static FOUND: [OnceLock<Option<String>>; 6] = [const { OnceLock::new() }; 6];
    let (slot, what) = match need {
        Need::Gl => (0, "OpenGL ES"),
        Need::Device(Gpu::Vaapi) => (1, "VA-API"),
        Need::Device(Gpu::Vulkan) => (2, "Vulkan"),
        Need::Device(Gpu::Cuda) => (3, "CUDA"),
        Need::VaapiH264 => (4, "VA-API H.264"),
        Need::Nvenc => (5, "NVENC"),
    };
    let Some(why) = FOUND[slot].get_or_init(|| check(need).err()) else {
        return false;
    };
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("a GPU test");
    // Straight to stderr: libtest hides a passing test's eprintln! output.
    let _ = writeln!(
        std::io::stderr(),
        "{test} skipped: no {what} on {RENDER_NODE} ({why})"
    );
    true
}

fn check(need: Need) -> Result<(), String> {
    let node = Path::new(RENDER_NODE);
    let vaapi = || {
        Devices::open(Gpu::Vaapi, node).map_err(|e| e.0)?;
        let (_, found) = probe::probe(None, &[Codec::H264], probe::live_test);
        (found
            .iter()
            .any(|(_, b, _)| matches!(b, Backend::Vaapi { .. })))
        .then_some(())
        .ok_or_else(|| "no VA-API encoder passed the probe".to_string())
    };
    match need {
        Need::Gl => {
            Gl::new(node).map_err(|e| e.0)?;
            let file = File::options().read(true).write(true).open(node);
            let gbm = gbm::Device::new(file.map_err(|e| e.to_string())?);
            (gbm.map_err(|e| e.to_string())?)
                .create_buffer_object_with_modifiers2::<()>(
                    16,
                    16,
                    gbm::Format::Xrgb8888,
                    [gbm::Modifier::Linear].into_iter(),
                    gbm::BufferObjectFlags::RENDERING,
                )
                .map(drop)
                .map_err(|e| format!("a GBM buffer: {e}"))
        }
        Need::Device(gpu) => probe::quietly(|| Devices::open(gpu, node))
            .map(drop)
            .map_err(|e| e.0),
        Need::VaapiH264 => probe::quietly(vaapi),
        Need::Nvenc => {
            probe::quietly(|| probe::live_test(Codec::H264, Backend::Nvenc, node)).map_err(|e| e.0)
        }
    }
}
