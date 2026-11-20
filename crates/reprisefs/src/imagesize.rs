//! An image's width and height from a file's first bytes, without decoding it:
//! PNG, JPEG, GIF, BMP, WebP, JPEG XL and HEIF stills (AVIF among them).
//!
//! The size is the picture shown upright, as the decoders turn it: a JPEG's or a PNG's Exif
//! orientation (read before its image data), a JPEG XL's orientation and a HEIF's `irot`. A
//! WebP's Exif chunk follows its image data and is not read.

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
    } else if head.starts_with(&[0xff, 0x0a]) {
        jxl_codestream(&head[2..])
    } else if head.starts_with(JXL_CONTAINER) {
        jxl_container(head)
    } else if head.get(4..8) == Some(b"ftyp") {
        heif(head)
    } else {
        None
    }?;
    (dims.0 > 0 && dims.1 > 0).then_some(dims)
}

/// IHDR's size, turned by an `eXIf` chunk between it and the first IDAT.
fn png(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let size = (u32_be(head, 16)?, u32_be(head, 20)?);
    let mut at = 8usize;
    let mut turned = false;
    while let (Some(len), Some(kind)) = (u32_be(head, at), head.get(at + 4..at + 8)) {
        let body = at + 8;
        let end = body.checked_add(usize::try_from(len).ok()?)?;
        match kind {
            b"IDAT" => break,
            b"eXIf" => turned = head.get(body..end).is_some_and(exif_turned),
            _ => {}
        }
        // The chunk's data, then its CRC.
        at = end.checked_add(4)?;
    }
    Some(upright(size, turned))
}

/// Walk the segments from SOI to the first SOFn frame header, which
/// holds the height, then the width. Scan data (SOS) or EOI before it
/// means there is none.
fn jpeg(head: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    let mut turned = false;
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
            return Some(upright((w.into(), h.into()), turned));
        }
        if len < 2 {
            return None;
        }
        // The last Exif APP1 is the one that counts.
        if marker == 0xe1
            && let Some(tiff) = head
                .get(at + 2..at + len)
                .and_then(|b| b.strip_prefix(b"Exif\0\0"))
        {
            turned = exif_turned(tiff);
        }
        at += len;
    }
}

/// Whether Exif `tiff` (from its byte-order mark) tags the picture with an orientation that
/// swaps its sides (5 to 8): the first IFD0 entry for tag 0x112 that is one SHORT.
fn exif_turned(tiff: &[u8]) -> bool {
    let big = match tiff.get(..4) {
        Some(b"II*\0") => false,
        Some(b"MM\0*") => true,
        _ => return false,
    };
    let u16_at = |at| {
        if big {
            u16_be(tiff, at)
        } else {
            u16_le(tiff, at)
        }
    };
    let u32_at = |at| {
        if big {
            u32_be(tiff, at)
        } else {
            u32_le(tiff, at)
        }
    };
    let orientation = || -> Option<u16> {
        let ifd = usize::try_from(u32_at(4)?).ok()?;
        for i in 0..usize::from(u16_at(ifd)?) {
            let at = ifd + 2 + 12 * i;
            let (tag, kind, count) = (u16_at(at)?, u16_at(at + 2)?, u32_at(at + 4)?);
            if tag == 0x112 && kind == 3 && count == 1 {
                return u16_at(at + 8);
            }
        }
        None
    };
    matches!(orientation(), Some(5..=8))
}

/// `size` with its sides swapped when the picture is shown `turned` a quarter.
fn upright((w, h): (u32, u32), turned: bool) -> (u32, u32) {
    if turned { (h, w) } else { (w, h) }
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

/// The signature box a JPEG XL container starts with.
pub const JXL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// The codestream's SizeHeader, right after its `FF 0A`, then the ImageMetadata's orientation
/// (3 bits after `all_default` and `extra_fields`): a bit stream read least significant bit first.
fn jxl_codestream(code: &[u8]) -> Option<(u32, u32)> {
    let mut at = 0usize;
    let mut bits = |n: u32| -> Option<u32> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *code.get(at / 8)?;
            v |= u32::from(byte >> (at % 8) & 1) << i;
            at += 1;
        }
        Some(v)
    };
    let small = bits(1)? == 1;
    let side = |bits: &mut dyn FnMut(u32) -> Option<u32>| -> Option<u32> {
        if small {
            return Some((bits(5)? + 1) * 8);
        }
        let n = [9, 13, 18, 30][bits(2)? as usize];
        Some(bits(n)? + 1)
    };
    let h = side(&mut bits)?;
    let w = match bits(3)? {
        0 => side(&mut bits)?,
        r => {
            let (num, den) =
                [(1, 1), (12, 10), (4, 3), (3, 2), (16, 9), (5, 4), (2, 1)][r as usize - 1];
            u32::try_from(u64::from(h) * num / den).ok()?
        }
    };
    let all_default = bits(1)? == 1;
    let turned = !all_default && bits(1)? == 1 && bits(3)? + 1 >= 5;
    Some(upright((w, h), turned))
}

/// The codestream's first bytes from the boxes after the signature: a whole `jxlc`, or the
/// `jxlp` parts (each after its 4-byte index) joined in order, as a part may end mid-header.
fn jxl_container(head: &[u8]) -> Option<(u32, u32)> {
    /// Past the longest SizeHeader (68 bits) and the orientation after its `FF 0A`.
    const ENOUGH: usize = 16;
    let mut code = Vec::new();
    let mut at = JXL_CONTAINER.len();
    while code.len() < ENOUGH && at < head.len() {
        let size = u32_be(head, at)?;
        let kind = head.get(at + 4..at + 8)?;
        let (body, end) = match size {
            0 => (at + 8, head.len()),
            1 => (
                at + 16,
                at.checked_add(usize::try_from(u64::from_be_bytes(bytes(head, at + 8)?)).ok()?)?,
            ),
            n if n < 8 => return None,
            n => (at + 8, at.checked_add(n as usize)?),
        };
        let part = head.get(body.min(head.len())..end.min(head.len()))?;
        match kind {
            b"jxlc" => {
                code.extend_from_slice(part);
                break;
            }
            b"jxlp" => {
                let index = u32_be(head, body)?;
                code.extend_from_slice(part.get(4..)?);
                if index & 0x8000_0000 != 0 {
                    break;
                }
            }
            _ => {}
        }
        at = end;
    }
    code.strip_prefix(&[0xff, 0x0a]).and_then(jxl_codestream)
}

/// The `ispe` of a HEIF still's primary item, from a `meta` box among the first boxes, turned by
/// its `irot`s. Like the decoder, the last `ispe` associated counts and the turns add up.
fn heif(head: &[u8]) -> Option<(u32, u32)> {
    let meta = boxes(head).find(|b| b.0 == *b"meta")?.1;
    let (mut primary, mut iprp) = (None, None);
    for (kind, body) in boxes(meta.get(4..)?) {
        match &kind {
            b"pitm" if body.first() == Some(&0) => primary = u16_be(body, 4).map(u32::from),
            b"pitm" => primary = u32_be(body, 4),
            b"iprp" => iprp = Some(body),
            _ => {}
        }
    }
    let (primary, iprp) = (primary?, iprp?);
    let ipco: Vec<_> = boxes(boxes(iprp).find(|b| b.0 == *b"ipco")?.1).collect();
    let (mut size, mut turns) = (None, 0u32);
    for (_, ipma) in boxes(iprp).filter(|b| b.0 == *b"ipma") {
        let (wide_id, wide_index) = (*ipma.first()? != 0, ipma.get(3)? & 1 != 0);
        let mut at = 8;
        for _ in 0..u32_be(ipma, 4)? {
            let id = if wide_id {
                u32_be(ipma, at)?
            } else {
                u16_be(ipma, at)?.into()
            };
            at += if wide_id { 4 } else { 2 };
            let count = *ipma.get(at)?;
            at += 1;
            for _ in 0..count {
                let index = if wide_index {
                    usize::from(u16_be(ipma, at)? & 0x7fff)
                } else {
                    usize::from(ipma.get(at)? & 0x7f)
                };
                at += if wide_index { 2 } else { 1 };
                let property = index.checked_sub(1).and_then(|i| ipco.get(i));
                let Some((kind, body)) = property.filter(|_| id == primary) else {
                    continue;
                };
                match kind {
                    b"ispe" => size = Some((u32_be(body, 4)?, u32_be(body, 8)?)),
                    b"irot" => turns += u32::from(*body.first()? & 3),
                    _ => {}
                }
            }
        }
    }
    Some(upright(size?, turns % 2 == 1))
}

/// The ISOBMFF boxes in `b` end to end, each one's type and body, up to one cut short.
fn boxes(b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let kind = bytes::<4>(b, at.checked_add(4)?)?;
        let (start, end) = match u32_be(b, at)? {
            0 => (at + 8, b.len()),
            1 => {
                let size = u64::from_be_bytes(bytes(b, at + 8)?);
                (at + 16, at.checked_add(usize::try_from(size).ok()?)?)
            }
            n => (at + 8, at.checked_add(n as usize)?),
        };
        let body = b.get(start..end)?;
        at = end;
        Some((kind, body))
    })
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

    fn heif_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(body);
        b
    }

    fn ispe(w: u32, h: u32) -> Vec<u8> {
        heif_box(
            b"ispe",
            &[[0; 4], w.to_be_bytes(), h.to_be_bytes()].concat(),
        )
    }

    /// An AVIF's first boxes: item 2 is primary with `props` after its `ispe` (the first property
    /// the thumbnail's), item 1 a 64x64 thumbnail with an `irot`, listed first. Without `ipma`
    /// no item has a property.
    fn avif_with(w: u32, h: u32, props: &[Vec<u8>], ipma: bool) -> Vec<u8> {
        let ipco = heif_box(
            b"ipco",
            &[&[ispe(64, 64), heif_box(b"irot", &[1]), ispe(w, h)], props]
                .concat()
                .concat(),
        );
        let mut assoc = vec![0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 2, 0x81, 0x82, 0, 2];
        assoc.push(1 + props.len() as u8);
        assoc.extend((3..=3 + props.len() as u8).map(|i| 0x80 | i));
        let iprp = match ipma {
            true => [ipco, heif_box(b"ipma", &assoc)].concat(),
            false => ipco,
        };
        let meta = [
            &[0, 0, 0, 0][..],
            &heif_box(b"pitm", &[0, 0, 0, 0, 0, 2]),
            &heif_box(b"iprp", &iprp),
        ]
        .concat();
        let mut v = heif_box(b"ftyp", b"avif\0\0\0\0avifmif1miaf");
        v.extend(heif_box(b"meta", &meta));
        v.extend(heif_box(b"mdat", &[0; 16]));
        v
    }

    fn avif_head(w: u32, h: u32) -> Vec<u8> {
        avif_with(w, h, &[], true)
    }

    #[test]
    fn an_avif_states_its_primary_items_size() {
        assert_eq!(dimensions(&avif_head(4032, 3024)), Some((4032, 3024)));
        let head = avif_head(800, 600);
        assert_eq!(dimensions(&head[..40]), None, "cut short before its ispe");
        assert_eq!(
            dimensions(&avif_with(800, 600, &[], false)),
            None,
            "no ipma, no ispe for the primary item"
        );
    }

    #[test]
    fn an_avif_states_its_size_turned_by_its_irot() {
        let irot = |q: u8| heif_box(b"irot", &[q]);
        let imir = |m: u8| heif_box(b"imir", &[m]);
        let cases = [
            (vec![irot(0)], (800, 600)),
            (vec![irot(1)], (600, 800)),
            (vec![irot(2)], (800, 600)),
            (vec![irot(3)], (600, 800)),
            (vec![imir(0)], (800, 600)),
            (vec![imir(1), irot(1)], (600, 800)),
            (vec![irot(1), irot(3)], (800, 600)),
            (vec![irot(0xfd)], (600, 800)),
        ];
        for (props, want) in cases {
            assert_eq!(dimensions(&avif_with(800, 600, &props, true)), Some(want));
        }
        assert_eq!(
            dimensions(&avif_with(800, 600, &[heif_box(b"irot", &[])], true)),
            None,
            "an irot without its angle"
        );
    }

    /// Every cut and every byte of `head` set to 0, 0xff and flipped at its top bit.
    fn mangled(head: &[u8]) -> impl Iterator<Item = Vec<u8>> + '_ {
        let cuts = (0..head.len()).map(|n| head[..n].to_vec());
        let bytes = (0..head.len()).flat_map(move |i| {
            [0, 0xff, head[i] ^ 0x80].map(|b| {
                let mut v = head.to_vec();
                v[i] = b;
                v
            })
        });
        cuts.chain(bytes)
    }

    #[test]
    fn a_hostile_head_is_read_without_a_panic() {
        let heads = [
            avif_with(800, 600, &[heif_box(b"irot", &[1])], true),
            jpeg_exif(exif_le(6), 800, 600),
            png_exif(&exif_be(8)),
            jxl_oriented(1921, 1081, 0, 6),
        ];
        for head in &heads {
            for v in mangled(head) {
                let _ = dimensions(&v);
            }
        }
        // An ipma claiming 2^32 - 1 items and a full head of zero bytes after it.
        let mut iprp = heif_box(b"ipco", &ispe(8, 8));
        let mut ipma = vec![1, 0, 0, 1, 0xff, 0xff, 0xff, 0xff];
        ipma.resize(HEAD_BYTES - 200, 0);
        iprp.extend(heif_box(b"ipma", &ipma));
        let meta = [
            &[0, 0, 0, 0][..],
            &heif_box(b"pitm", &[0, 0, 0, 0, 0, 2]),
            &heif_box(b"iprp", &iprp),
        ]
        .concat();
        let mut head = heif_box(b"ftyp", b"avif\0\0\0\0avifmif1miaf");
        head.extend(heif_box(b"meta", &meta));
        assert_eq!(dimensions(&head), None);
        // An Exif IFD of 65535 entries running past its segment.
        let mut tiff = b"II*\0\x08\0\0\0\xff\xff".to_vec();
        tiff.resize(60_000, 0);
        assert_eq!(dimensions(&jpeg_exif(tiff, 800, 600)), Some((800, 600)));
    }

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

    /// A TIFF header and an IFD0 of a Make entry and an Orientation entry (SHORT, count 1).
    fn exif_le(orientation: u16) -> Vec<u8> {
        let mut v = b"II*\0\x08\0\0\0\x02\0".to_vec();
        v.extend([0x0f, 0x01, 2, 0, 4, 0, 0, 0, b'A', b'B', b'C', 0]);
        v.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        v.extend(orientation.to_le_bytes());
        v.extend([0, 0, 0, 0, 0, 0]);
        v
    }

    fn exif_be(orientation: u16) -> Vec<u8> {
        let mut v = b"MM\0*\0\0\0\x08\0\x01".to_vec();
        v.extend([0x01, 0x12, 0, 3, 0, 0, 0, 1]);
        v.extend(orientation.to_be_bytes());
        v.extend([0, 0, 0, 0, 0, 0]);
        v
    }

    /// A JPEG whose APP1 holds Exif `tiff`.
    fn jpeg_exif(tiff: Vec<u8>, w: u16, h: u16) -> Vec<u8> {
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut v = vec![0xff, 0xd8, 0xff, 0xe1];
        v.extend(((app1.len() + 2) as u16).to_be_bytes());
        v.extend(app1);
        v.extend(&jpeg_head(0xc0, 0, w, h)[2..]);
        v
    }

    /// An 800x600 PNG with an `eXIf` chunk of `tiff` before its IDAT.
    fn png_exif(tiff: &[u8]) -> Vec<u8> {
        let mut v = png_head(800, 600);
        v.extend([0; 4]);
        for (kind, body) in [(b"eXIf", tiff), (b"IDAT", &[0; 4][..])] {
            v.extend((body.len() as u32).to_be_bytes());
            v.extend(kind);
            v.extend(body);
            v.extend([0; 4]);
        }
        v
    }

    #[test]
    fn a_jpeg_or_png_states_its_size_turned_by_its_exif_orientation() {
        for (o, want) in [
            (1, (800, 600)),
            (3, (800, 600)),
            (5, (600, 800)),
            (6, (600, 800)),
            (8, (600, 800)),
            (9, (800, 600)),
        ] {
            assert_eq!(
                dimensions(&jpeg_exif(exif_le(o), 800, 600)),
                Some(want),
                "JPEG {o}"
            );
            assert_eq!(
                dimensions(&jpeg_exif(exif_be(o), 800, 600)),
                Some(want),
                "JPEG {o}"
            );
            assert_eq!(dimensions(&png_exif(&exif_le(o))), Some(want), "PNG {o}");
        }
        let mut late = png_head(800, 600);
        late.extend([0; 4]);
        for (kind, body) in [(b"IDAT", &[0; 4][..]), (b"eXIf", &exif_le(6))] {
            late.extend((body.len() as u32).to_be_bytes());
            late.extend(kind);
            late.extend(body);
            late.extend([0; 4]);
        }
        assert_eq!(
            dimensions(&late),
            Some((800, 600)),
            "an eXIf after IDAT is not read"
        );
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

    fn jxl_head(w: u32, h: u32, ratio: u32) -> Vec<u8> {
        jxl_oriented(w, h, ratio, 1)
    }

    /// A codestream signature, its SizeHeader and its ImageMetadata up to `orientation`, bits
    /// least significant first: the small form (sides in eighths up to 256) when both sides allow
    /// it, else the 9-, 13-, 18- or 30-bit form with an explicit width (`ratio` 0) or a width
    /// from `ratio`.
    fn jxl_oriented(w: u32, h: u32, ratio: u32, orientation: u32) -> Vec<u8> {
        let mut bits: Vec<bool> = Vec::new();
        let mut put = |v: u32, n: u32| bits.extend((0..n).map(|i| v >> i & 1 == 1));
        let small =
            w.is_multiple_of(8) && h.is_multiple_of(8) && w <= 256 && h <= 256 && ratio == 0;
        let side = |put: &mut dyn FnMut(u32, u32), v: u32| {
            let (sel, n) = [(0, 9), (1, 13), (2, 18), (3, 30)]
                .into_iter()
                .find(|&(_, n)| v - 1 < 1 << n)
                .unwrap();
            put(sel, 2);
            put(v - 1, n);
        };
        put(small as u32, 1);
        if small {
            put(h / 8 - 1, 5);
        } else {
            side(&mut put, h);
        }
        put(ratio, 3);
        if ratio == 0 {
            if small {
                put(w / 8 - 1, 5);
            } else {
                side(&mut put, w);
            }
        }
        match orientation {
            1 => put(1, 1),
            o => {
                put(0b10, 2);
                put(o - 1, 3);
            }
        }
        let mut v = vec![0xff, 0x0a];
        v.extend(bits.chunks(8).map(|c| {
            c.iter()
                .enumerate()
                .fold(0u8, |b, (i, &on)| b | u8::from(on) << i)
        }));
        v
    }

    fn jxl_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(kind);
        v.extend(body);
        v
    }

    #[test]
    fn a_jxl_codestream_and_container_state_their_size() {
        assert_eq!(dimensions(&jxl_head(64, 48, 0)), Some((64, 48)));
        assert_eq!(dimensions(&jxl_head(256, 8, 0)), Some((256, 8)));
        assert_eq!(dimensions(&jxl_head(1921, 1081, 0)), Some((1921, 1081)));
        assert_eq!(
            dimensions(&jxl_head(20_000, 300_000, 0)),
            Some((20_000, 300_000))
        );
        assert_eq!(
            dimensions(&jxl_head(0, 1080, 5)),
            Some((1920, 1080)),
            "16:9"
        );
        assert_eq!(
            dimensions(&jxl_head(0, 1000, 2)),
            Some((1200, 1000)),
            "12:10"
        );
        let mut file = jxl_box(b"JXL ", &[0x0d, 0x0a, 0x87, 0x0a]);
        file.extend(jxl_box(b"ftyp", b"jxl \0\0\0\0jxl "));
        file.extend(jxl_box(b"jxll", &[5]));
        file.extend(jxl_box(b"jxlc", &jxl_head(800, 600, 0)));
        assert_eq!(dimensions(&file), Some((800, 600)));
        let mut parts = file[..file.len() - jxl_head(800, 600, 0).len() - 8].to_vec();
        let mut first = 0u32.to_be_bytes().to_vec();
        first.extend(jxl_head(640, 480, 0));
        parts.extend(jxl_box(b"jxlp", &first));
        assert_eq!(dimensions(&parts), Some((640, 480)));
        let split = jxl_head(20_000, 20_000, 0);
        let mut joined = file[..file.len() - jxl_head(800, 600, 0).len() - 8].to_vec();
        for (index, part) in [
            (0u32, &split[..2]),
            (1, &split[2..5]),
            (0x8000_0002, &split[5..]),
        ] {
            let mut body = index.to_be_bytes().to_vec();
            body.extend(part);
            joined.extend(jxl_box(b"jxlp", &body));
            joined.extend(jxl_box(b"Exif", &[0; 6]));
        }
        assert_eq!(
            dimensions(&joined),
            Some((20_000, 20_000)),
            "jxlp parts join across the header"
        );
        assert_eq!(dimensions(&[0xff, 0x0a, 0x00]), None, "cut short");
    }

    #[test]
    fn a_jxl_states_its_size_turned_by_its_orientation() {
        for (o, want) in [
            (1, (64, 48)),
            (2, (64, 48)),
            (4, (64, 48)),
            (5, (48, 64)),
            (6, (48, 64)),
            (7, (48, 64)),
            (8, (48, 64)),
        ] {
            assert_eq!(dimensions(&jxl_oriented(64, 48, 0, o)), Some(want), "{o}");
        }
        assert_eq!(dimensions(&jxl_oriented(0, 1080, 5, 6)), Some((1080, 1920)));
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
