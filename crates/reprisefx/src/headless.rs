//! A `GlesRenderer` with no output, on an EGL device: for tests and tools
//! that render glass offscreen.

use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;

/// A renderer on the first hardware EGL device, or on Mesa's software
/// device when `LIBGL_ALWAYS_SOFTWARE` is set (or no other is found).
/// Err says why there is none.
pub fn renderer() -> Result<GlesRenderer, String> {
    let software =
        std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some_and(|v| v != "0" && !v.is_empty());
    let devices: Vec<EGLDevice> = EGLDevice::enumerate()
        .map_err(|e| format!("no EGL devices: {e}"))?
        .collect();
    let device = devices
        .iter()
        .find(|d| d.is_software() == software)
        .or_else(|| devices.first())
        .cloned()
        .ok_or("no EGL device")?;
    // SAFETY: the device stays valid for the display's life (EGL owns
    // it), and the display is the only one made from it here.
    let display = unsafe { EGLDisplay::new(device) }.map_err(|e| format!("EGL display: {e}"))?;
    let context = EGLContext::new(&display).map_err(|e| format!("EGL context: {e}"))?;
    // SAFETY: the context was just made and is used by this renderer only.
    unsafe { GlesRenderer::new(context) }.map_err(|e| format!("GLES renderer: {e}"))
}
