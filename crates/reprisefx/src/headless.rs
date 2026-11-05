//! A `GlesRenderer` with no output, on an EGL device: for tests and tools
//! that render glass offscreen.

use std::sync::OnceLock;

use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;

/// A renderer on the first hardware EGL device, or on Mesa's software
/// device when `LIBGL_ALWAYS_SOFTWARE` is set (or no other is found).
/// Err says why there is no EGL display; a display that gives no renderer
/// panics, so a test never skips over a broken one.
pub fn renderer() -> Result<GlesRenderer, String> {
    let display = display()?;
    let context = EGLContext::new(&display).expect("an EGL context on the headless display");
    // SAFETY: the context was just made and is used by this renderer only.
    let renderer =
        unsafe { GlesRenderer::new(context) }.expect("a GLES renderer on the headless display");
    Ok(renderer)
}

/// The process's one headless display, made on first use and never dropped,
/// so each later renderer skips device enumeration and EGL initialisation.
fn display() -> Result<EGLDisplay, String> {
    static DISPLAY: OnceLock<Result<EGLDisplay, String>> = OnceLock::new();
    DISPLAY
        .get_or_init(|| {
            let software = std::env::var_os("LIBGL_ALWAYS_SOFTWARE")
                .is_some_and(|v| v != "0" && !v.is_empty());
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
            unsafe { EGLDisplay::new(device) }.map_err(|e| format!("EGL display: {e}"))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    #[test]
    fn the_display_outlives_every_renderer_made_on_it() {
        let Ok(renderer) = super::renderer() else {
            eprintln!("skipped: no EGL display");
            return;
        };
        let display = Arc::downgrade(&renderer.egl_context().display().get_display_handle());
        drop(renderer);
        assert!(
            display.upgrade().is_some(),
            "the display is still up with no renderer on it"
        );
        let again = super::renderer().expect("a second renderer");
        assert!(
            Arc::ptr_eq(
                &display.upgrade().unwrap(),
                &again.egl_context().display().get_display_handle()
            ),
            "the second renderer is on the same display"
        );
    }
}
