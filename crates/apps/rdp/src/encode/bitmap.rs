//! The bitmap path: one [`BitmapUpdate`] per damaged rectangle, read from shared memory in
//! place or copied out of a mapped linear dma-buf.

use super::{EncodeError, Encoded, Encoder};
use crate::stream::{
    ABGR8888, ARGB8888, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Plane, Rect, XBGR8888, XRGB8888,
};
use bytes::Bytes;
use ironrdp_server::{BitmapUpdate, PixelFormat};
use reprisevideo::encode::dmabuf::DmabufRead;
use std::num::{NonZeroU16, NonZeroUsize};

pub struct Bitmap;

impl Encoder for Bitmap {
    fn encode(&mut self, frame: &Frame, keyframe: bool) -> Result<Encoded, EncodeError> {
        let whole = [Rect::full(frame.size)];
        let rects = if keyframe {
            &whole[..]
        } else {
            &frame.damage[..]
        };
        if rects.is_empty() {
            return Ok(Encoded::Nothing);
        }
        let updates = match &frame.buffer {
            FrameBuffer::Shm {
                data,
                fourcc,
                stride,
            } => {
                let format = pixel_format(*fourcc)?;
                let stride = *stride as usize;
                rects
                    .iter()
                    .map(|r| {
                        let (start, end) = span(*r, stride);
                        if end > data.len() {
                            return Err(outside());
                        }
                        update(*r, format, data.slice(start..end), stride)
                    })
                    .collect::<Result<_, _>>()?
            }
            FrameBuffer::Dmabuf {
                objects,
                fourcc,
                modifier,
                planes,
            } => {
                if *modifier != DRM_FORMAT_MOD_LINEAR {
                    return Err(EncodeError(format!(
                        "modifier {modifier:#x} cannot be read by the CPU"
                    )));
                }
                let format = pixel_format(*fourcc)?;
                let Plane {
                    object,
                    offset,
                    stride,
                } = planes[0];
                let stride = stride as usize;
                let len = offset as usize + stride * frame.size.1 as usize;
                let map = DmabufRead::new(&objects[object], len)?;
                let pixels = &map.bytes()[offset as usize..];
                rects
                    .iter()
                    .map(|r| {
                        let (start, end) = span(*r, stride);
                        let src = pixels.get(start..end).ok_or_else(outside)?;
                        let row = r.width as usize * 4;
                        let mut tight = Vec::with_capacity(row * r.height as usize);
                        for y in 0..r.height as usize {
                            tight.extend_from_slice(&src[y * stride..][..row]);
                        }
                        update(*r, format, tight.into(), row)
                    })
                    .collect::<Result<_, _>>()?
            }
        };
        Ok(Encoded::Bitmap(updates))
    }
}

/// The byte range rectangle `r` covers in rows of `stride` bytes.
fn span(r: Rect, stride: usize) -> (usize, usize) {
    let start = r.y as usize * stride + r.x as usize * 4;
    (
        start,
        start + (r.height as usize - 1) * stride + r.width as usize * 4,
    )
}

fn outside() -> EncodeError {
    EncodeError("damage lies outside the frame".into())
}

fn update(
    r: Rect,
    format: PixelFormat,
    data: Bytes,
    stride: usize,
) -> Result<BitmapUpdate, EncodeError> {
    let u16 = |v: u32| u16::try_from(v).map_err(|_| EncodeError(format!("{v} px is too large")));
    Ok(BitmapUpdate {
        x: u16(r.x)?,
        y: u16(r.y)?,
        width: NonZeroU16::new(u16(r.width)?).ok_or_else(outside)?,
        height: NonZeroU16::new(u16(r.height)?).ok_or_else(outside)?,
        format,
        data,
        stride: NonZeroUsize::new(stride).ok_or_else(outside)?,
    })
}

/// The bitmap format of DRM `fourcc`. The desktop is opaque, so alpha is ignored.
fn pixel_format(fourcc: u32) -> Result<PixelFormat, EncodeError> {
    match fourcc {
        XRGB8888 | ARGB8888 => Ok(PixelFormat::BgrX32),
        XBGR8888 | ABGR8888 => Ok(PixelFormat::RgbX32),
        _ => Err(EncodeError(format!("format {fourcc:#x} is not supported"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x2 frame whose pixel (x, y) is [x, y, 0, 0].
    fn frame(damage: Vec<Rect>) -> Frame {
        let data = (0..2u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 0, 0]))
            .collect();
        Frame::detached(data, (4, 2), damage)
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn each_damaged_rect_becomes_an_update() {
        let Ok(Encoded::Bitmap(updates)) =
            Bitmap.encode(&frame(vec![rect(1, 0, 2, 2), rect(3, 1, 1, 1)]), false)
        else {
            panic!("bitmap updates");
        };
        let got: Vec<_> = updates
            .iter()
            .map(|u| (u.x, u.y, u.width.get(), u.height.get(), u.stride.get()))
            .collect();
        assert_eq!(got, vec![(1, 0, 2, 2, 16), (3, 1, 1, 1, 16)]);
        assert_eq!(&updates[0].data[..8], &[1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&updates[0].data[16..24], &[1, 1, 0, 0, 2, 1, 0, 0]);
        assert_eq!(&updates[1].data[..], &[3, 1, 0, 0]);
        assert_eq!(updates[0].format, PixelFormat::BgrX32);
    }

    #[test]
    fn a_keyframe_is_the_whole_picture() {
        let Ok(Encoded::Bitmap(updates)) = Bitmap.encode(&frame(vec![rect(1, 0, 1, 1)]), true)
        else {
            panic!("bitmap updates");
        };
        assert_eq!(updates.len(), 1);
        assert_eq!((updates[0].width.get(), updates[0].height.get()), (4, 2));
    }

    #[test]
    fn a_cursor_only_frame_sends_nothing() {
        assert!(matches!(
            Bitmap.encode(&frame(vec![]), false),
            Ok(Encoded::Nothing)
        ));
    }
}
