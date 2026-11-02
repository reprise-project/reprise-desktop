//! Stills of the outputs for Screenshot: every output frozen through
//! ext-image-copy-capture as ARGB with its place in the layout.

use crate::wayland::{Target, Wayland};
use reprisekit::controls::colorpick::Still;
use reprisekit::raster::Rect;
use wayland_client::protocol::wl_shm::Format;

pub(crate) struct Output {
    pub(crate) layout: Rect,
    pub(crate) still: Still,
}

/// A still of every output with its place in the layout; `cursor` paints
/// the pointer into them.
pub(crate) fn capture(cursor: bool) -> Result<Vec<Output>, String> {
    let mut wl = Wayland::connect()?;
    let mut stills = Vec::new();
    for out in wl.outputs() {
        let still = grab(&mut wl, &Target::Output(out.name.clone()), cursor)
            .map_err(|e| format!("{}: {e}", out.name))?;
        stills.push(Output {
            layout: Rect::new(out.x, out.y, out.width, out.height),
            still: Still {
                logical: (out.width, out.height),
                ..still
            },
        });
    }
    Ok(stills)
}

/// One frame of `target` as ARGB at its buffer size; `logical` is left
/// at the buffer size for the caller to set.
pub(crate) fn grab(wl: &mut Wayland, target: &Target, cursor: bool) -> Result<Still, String> {
    let session = wl.open_session(target, cursor)?;
    let c = wl.constraints();
    let format = c.pick_format().ok_or("no 32 bpp shm format")?;
    let buf = wl.shm_buffer(c.width, c.height, format)?;
    // A source always has a first frame to give; two seconds is for a
    // compositor busy with something else.
    wl.capture(&session, &buf, std::time::Duration::from_secs(2))
        .map_err(|e| format!("capture failed: {e:?}"))?;
    let bytes = buf.read()?;
    let row = buf.stride as usize;
    let mut pixels = Vec::with_capacity((c.width * c.height) as usize);
    for y in 0..c.height as usize {
        let (words, _) = bytes[y * row..].as_chunks::<4>();
        pixels.extend(
            words[..c.width as usize]
                .iter()
                .map(|&w| to_argb(w, format)),
        );
    }
    Ok(Still {
        logical: (c.width as i32, c.height as i32),
        width: c.width,
        height: c.height,
        pixels,
    })
}

fn to_argb(px: [u8; 4], format: Format) -> u32 {
    let (r, g, b) = match format {
        Format::Argb8888 | Format::Xrgb8888 => (px[2], px[1], px[0]),
        _ => (px[0], px[1], px[2]),
    };
    u32::from_be_bytes([0xff, r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_read_back_in_either_byte_order() {
        assert_eq!(to_argb([3, 2, 1, 0], Format::Xrgb8888), 0xff01_0203);
        assert_eq!(to_argb([1, 2, 3, 0], Format::Xbgr8888), 0xff01_0203);
    }
}
