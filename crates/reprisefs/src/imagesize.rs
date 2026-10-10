//! An image's width and height from a file's first bytes, without decoding it:
//! PNG, JPEG, GIF, BMP and WebP.

/// How much of a file [`dimensions`] may need. Every format but JPEG states
/// its size in the first 30 bytes; a JPEG's frame header follows APPn segments
/// (up to 64 KiB each), and 256 KiB covers four. Beyond that: `None`.
pub const HEAD_BYTES: usize = 256 << 10;

/// The width and height `head` (a file's first bytes) states, `None` for
/// a format this does not read, a header cut short, or a zero side.
pub fn dimensions(head: &[u8]) -> Option<(u32, u32)> {
    let dims = if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(head)
    } else if head.starts_with(&[0xff, 0xd8]) {
        jpeg(head)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some((u16_le(head, 6)?.into(), u16_le(head, 8)?.into()))
    } else if head.starts_with(b"BM") {
        bmp(head)
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        webp(head)
    } else {
        None
    }?;
    (dims.0 > 0 && dims.1 > 0).then_some(dims)
}

fn png(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(12..16) != Some(b"IHDR") {
        return None;
    }
    Some((u32_be(head, 16)?, u32_be(head, 20)?))
}

/// Walk the segments from SOI to the first SOFn frame header, which
/// holds the height, then the width. Scan data (SOS) or EOI before it
/// means there is none.
fn jpeg(head: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *head.get(at)? != 0xff {
            return None;
        }
        // Any number of 0xFF fill bytes may precede a marker.
        while *head.get(at)? == 0xff {
            at += 1;
        }
        let marker = head[at];
        at += 1;
        match marker {
            // Markers with no segment after them: TEM, RSTn, a stray SOI.
            0x01 | 0xd0..=0xd8 => continue,
            0xd9 | 0xda => return None,
            _ => {}
        }
        let len = usize::from(u16_be(head, at)?);
        // SOF0..SOF15, less DHT (C4), JPG (C8) and DAC (CC).
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let h = u16_be(head, at + 3)?;
            let w = u16_be(head, at + 5)?;
            return Some((w.into(), h.into()));
        }
        if len < 2 {
            return None;
        }
        at += len;
    }
}

/// The DIB header after the 14-byte file header: the old 12-byte core
/// header has 16-bit sides, every later one signed 32-bit sides, a
/// negative height marking a top-down bitmap.
fn bmp(head: &[u8]) -> Option<(u32, u32)> {
    if u32_le(head, 14)? == 12 {
        return Some((u16_le(head, 18)?.into(), u16_le(head, 20)?.into()));
    }
    let w = i32::try_from(u32_le(head, 18)?).ok()?;
    let h = u32_le(head, 22)? as i32;
    Some((u32::try_from(w).ok()?, h.unsigned_abs()))
}

/// The first chunk after the RIFF header: a lossy key frame (`VP8 `), a
/// lossless bitstream (`VP8L`) or the extended header (`VP8X`) that
/// states the canvas.
fn webp(head: &[u8]) -> Option<(u32, u32)> {
    let data = head.get(20..)?;
    match head.get(12..16)? {
        b"VP8 " => {
            // A 3-byte frame tag, the 9D 01 2A start code, then 14-bit
            // sides under 2 bits of scaling.
            if data.get(3..6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let w = u16_le(data, 6)? & 0x3fff;
            let h = u16_le(data, 8)? & 0x3fff;
            Some((w.into(), h.into()))
        }
        b"VP8L" => {
            // The 0x2F signature, then width - 1 and height - 1 in 14
            // bits each, least significant first.
            if *data.first()? != 0x2f {
                return None;
            }
            let bits = u32_le(data, 1)?;
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => {
            // Flags and three reserved bytes, then canvas width - 1 and
            // height - 1 in 24 bits each.
            let u24 = |at: usize| -> Option<u32> {
                let b = data.get(at..at + 3)?;
                Some(u32::from_le_bytes([b[0], b[1], b[2], 0]))
            };
            Some((u24(4)? + 1, u24(7)? + 1))
        }
        _ => None,
    }
}

fn bytes<const N: usize>(head: &[u8], at: usize) -> Option<[u8; N]> {
    head.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u16_be(head: &[u8], at: usize) -> Option<u16> {
    bytes(head, at).map(u16::from_be_bytes)
}

fn u16_le(head: &[u8], at: usize) -> Option<u16> {
    bytes(head, at).map(u16::from_le_bytes)
}

fn u32_be(head: &[u8], at: usize) -> Option<u32> {
    bytes(head, at).map(u32::from_be_bytes)
}

fn u32_le(head: &[u8], at: usize) -> Option<u32> {
    bytes(head, at).map(u32::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_head(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v.extend([8, 6, 0, 0, 0]);
        v
    }

    /// SOI, an APP0 and an APP1 of `app1` bytes, then a baseline or
    /// progressive frame header.
    fn jpeg_head(sof: u8, app1: usize, w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8];
        v.extend([0xff, 0xe0, 0x00, 0x10]);
        v.extend(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        v.extend([0xff, 0xe1]);
        v.extend(((app1 + 2) as u16).to_be_bytes());
        v.extend(std::iter::repeat_n(0xaa, app1));
        // Fill bytes before the marker are allowed.
        v.extend([0xff, 0xff, sof, 0x00, 0x11, 0x08]);
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        v
    }

    fn bmp_head(dib: u32, w: i32, h: i32) -> Vec<u8> {
        let mut v = b"BM".to_vec();
        v.extend([0; 12]);
        v.extend(dib.to_le_bytes());
        if dib == 12 {
            v.extend((w as u16).to_le_bytes());
            v.extend((h as u16).to_le_bytes());
        } else {
            v.extend(w.to_le_bytes());
            v.extend(h.to_le_bytes());
        }
        v.extend([1, 0, 24, 0]);
        v
    }

    fn webp_head(fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WEBP".to_vec();
        v.extend(fourcc);
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(data);
        v
    }

    #[test]
    fn a_png_states_its_size_in_ihdr() {
        assert_eq!(dimensions(&png_head(800, 600)), Some((800, 600)));
    }

    #[test]
    fn a_jpeg_is_scanned_past_its_app_segments_to_the_frame() {
        assert_eq!(dimensions(&jpeg_head(0xc0, 40, 800, 600)), Some((800, 600)));
        // A progressive frame, behind a full-size 64 KiB Exif segment.
        let big = jpeg_head(0xc2, 65533, 4032, 3024);
        assert_eq!(dimensions(&big), Some((4032, 3024)));
    }

    #[test]
    fn a_jpeg_without_a_frame_before_its_scan_has_no_size() {
        let mut v = vec![0xff, 0xd8, 0xff, 0xc4, 0x00, 0x04, 0, 0];
        v.extend([0xff, 0xda, 0x00, 0x08]);
        assert_eq!(dimensions(&v), None);
    }

    #[test]
    fn a_gif_states_its_logical_screen() {
        let mut v = b"GIF89a".to_vec();
        v.extend(320u16.to_le_bytes());
        v.extend(200u16.to_le_bytes());
        v.extend([0xf7, 0, 0]);
        assert_eq!(dimensions(&v), Some((320, 200)));
    }

    #[test]
    fn a_bmp_reads_either_header_and_a_top_down_height() {
        assert_eq!(dimensions(&bmp_head(40, 640, 480)), Some((640, 480)));
        assert_eq!(dimensions(&bmp_head(124, 640, -480)), Some((640, 480)));
        assert_eq!(dimensions(&bmp_head(12, 32, 16)), Some((32, 16)));
        assert_eq!(dimensions(&bmp_head(40, -640, 480)), None);
    }

    #[test]
    fn a_webp_reads_each_first_chunk() {
        let mut lossy = vec![0x30, 0x01, 0x00, 0x9d, 0x01, 0x2a];
        // A scaling code in the top 2 bits is not part of the width.
        lossy.extend((0x4000u16 | 550).to_le_bytes());
        lossy.extend(368u16.to_le_bytes());
        assert_eq!(dimensions(&webp_head(b"VP8 ", &lossy)), Some((550, 368)));

        let bits: u32 = (400 - 1) | ((301 - 1) << 14);
        let mut lossless = vec![0x2f];
        lossless.extend(bits.to_le_bytes());
        assert_eq!(dimensions(&webp_head(b"VP8L", &lossless)), Some((400, 301)));

        let mut ext = vec![0x10, 0, 0, 0];
        ext.extend(&(70_000u32 - 1).to_le_bytes()[..3]);
        ext.extend(&(2_000u32 - 1).to_le_bytes()[..3]);
        assert_eq!(dimensions(&webp_head(b"VP8X", &ext)), Some((70_000, 2_000)));
    }

    #[test]
    fn a_cut_short_or_foreign_head_has_no_size() {
        let cases: [Vec<u8>; 7] = [
            Vec::new(),
            png_head(800, 600)[..20].to_vec(),
            // The frame header lies past the end of what was read.
            jpeg_head(0xc0, 1000, 800, 600)[..500].to_vec(),
            bmp_head(40, 640, 480)[..20].to_vec(),
            b"GIF89a\x40".to_vec(),
            b"not an image at all, only some text".to_vec(),
            png_head(0, 600),
        ];
        for head in cases {
            assert_eq!(dimensions(&head), None, "{head:?}");
        }
    }
}
