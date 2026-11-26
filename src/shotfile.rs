//! A screenshot's saved file in `[screenshot] format`, encoded by `reprisefs::stillfile`. The
//! clipboard always takes the fast PNG.

use repriseconf::config::{PngCompression, ScreenshotConfig, ShotFormat};
pub use reprisefs::stillfile::Failed;

use crate::raster::Canvas;

/// File encodes under way at once: the one running and one waiting for it. A screenshot taken past
/// them is saved as its clipboard PNG, so queued stills cannot pile up in the compositor.
pub const MAX_ENCODES: usize = 2;

/// Where a screenshot's file comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The clipboard's PNG byte for byte: `[screenshot] format` is PNG at fast compression.
    Clipboard,
    /// An encode of its own in `[screenshot] format`.
    Encode,
    /// The clipboard's PNG, saved as PNG: [`MAX_ENCODES`] are under way.
    Busy,
}

/// The file of a screenshot taken while `encodes` file encodes are under way.
pub fn plan(s: &ScreenshotConfig, encodes: usize) -> Plan {
    if s.format == ShotFormat::Png && s.png_compression == PngCompression::Fast {
        Plan::Clipboard
    } else if encodes >= MAX_ENCODES {
        Plan::Busy
    } else {
        Plan::Encode
    }
}

/// Held through each file encode, so a second Print Screen waits for the first rather than
/// both taking the cores at once.
static ENCODING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `still`'s opaque RGB in `s.format`, one encode at a time, on a thread at nice 10. Blocking: a
/// JPEG XL at high effort takes seconds.
pub fn encode(still: &Canvas, s: &ScreenshotConfig) -> Result<Vec<u8>, Failed> {
    let _one = ENCODING.lock().unwrap_or_else(|e| e.into_inner());
    std::thread::scope(|scope| {
        let low = std::thread::Builder::new()
            .name("shotfile".into())
            .spawn_scoped(scope, || {
                lower_priority();
                encode_at(still, s)
            });
        match low {
            Ok(t) => t
                .join()
                .unwrap_or_else(|_| Err(Failed::Codec("the encoder panicked".into()))),
            Err(_) => encode_at(still, s),
        }
    })
}

/// Nice 10 for the calling thread; the reprise-encode it starts inherits it.
fn lower_priority() {
    // SAFETY: plain system calls about the calling thread.
    let set = unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10) };
    if set != 0 {
        log::debug!(
            "screenshot: encoding at normal priority: {}",
            std::io::Error::last_os_error()
        );
    }
}
fn encode_at(still: &Canvas, s: &ScreenshotConfig) -> Result<Vec<u8>, Failed> {
    let (w, h) = still.device_size();
    let rgb: Vec<u8> = (still.pixels.iter())
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    reprisefs::stillfile::encode(&rgb, w, h, &s.still())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_canvas(w: u32, h: u32) -> Canvas {
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                0xff00_0000 | ((x * 4) << 16) | ((y * 5) << 8) | (((x ^ y) * 3) & 0xff)
            })
            .collect();
        Canvas::from_pixels(w, h, px)
    }

    fn rgb_of(c: &Canvas) -> Vec<u8> {
        (c.pixels.iter())
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect()
    }

    #[test]
    fn png_and_jpg_write_their_own_files() {
        let still = test_canvas(64, 48);
        for (format, magic) in [
            (ShotFormat::Png, &b"\x89PNG"[..]),
            (ShotFormat::Jpg, &[0xff, 0xd8][..]),
        ] {
            let s = ScreenshotConfig {
                format,
                ..Default::default()
            };
            assert!(encode(&still, &s).unwrap().starts_with(magic), "{format:?}");
        }
    }

    #[test]
    fn every_png_level_keeps_every_pixel() {
        let still = test_canvas(64, 48);
        let want = rgb_of(&still);
        for png_compression in PngCompression::ALL {
            let s = ScreenshotConfig {
                png_compression,
                ..Default::default()
            };
            let back = image::load_from_memory(&encode(&still, &s).unwrap()).unwrap();
            assert_eq!(back.to_rgb8().into_raw(), want, "{png_compression:?}");
        }
    }

    #[test]
    fn a_third_screenshot_under_way_keeps_its_clipboard_png() {
        let jxl = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..Default::default()
        };
        assert_eq!(plan(&jxl, 0), Plan::Encode);
        assert_eq!(
            plan(&jxl, 1),
            Plan::Encode,
            "the second waits for the first"
        );
        assert_eq!(plan(&jxl, 2), Plan::Busy);
        let fast = ScreenshotConfig {
            png_compression: PngCompression::Fast,
            ..Default::default()
        };
        assert_eq!(plan(&fast, 2), Plan::Clipboard);
        assert_eq!(
            plan(&ScreenshotConfig::default(), 0),
            Plan::Encode,
            "files default to Default compression"
        );
    }
}
