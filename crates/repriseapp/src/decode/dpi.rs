//! The resolution a picture file states for itself: a PNG's pHYs chunk, a JPEG's JFIF density or
//! its EXIF XResolution. Only the across resolution is read; pictures with unequal ones are rare
//! and print with square pixels.

/// Resolutions outside this range are taken as no resolution: a file that says 1 dpi means none.
const PLAUSIBLE: std::ops::RangeInclusive<u32> = 24..=9600;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The dots per inch `bytes` says it was made at; `None` when it says nothing usable.
pub fn dpi(bytes: &[u8]) -> Option<u32> {
    let dpi = if bytes.starts_with(PNG_SIGNATURE) {
        png(&bytes[PNG_SIGNATURE.len()..])
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg(&bytes[2..])
    } else {
        None
    }?;
    PLAUSIBLE.contains(&dpi).then_some(dpi)
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// pHYs before the first IDAT; its unit 1 is the metre.
fn png(mut b: &[u8]) -> Option<u32> {
    loop {
        let len = be32(b, 0)? as usize;
        let kind = b.get(4..8)?;
        let data = b.get(8..8usize.checked_add(len)?)?;
        match kind {
            b"pHYs" if data.len() == 9 && data[8] == 1 => {
                let per_metre = be32(data, 0)? as u64;
                return u32::try_from((per_metre * 254 + 5000) / 10000).ok();
            }
            b"IDAT" | b"IEND" => return None,
            _ => b = b.get(len.checked_add(12)?..)?,
        }
    }
}

/// The first of JFIF and EXIF, in file order, that states a resolution, before the scan.
fn jpeg(mut b: &[u8]) -> Option<u32> {
    loop {
        if b.first() != Some(&0xFF) {
            return None;
        }
        let marker = *b.get(1)?;
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let len = be16(b, 2)? as usize;
        let data = b.get(4..2 + len.max(2))?;
        let found = match marker {
            0xE0 => jfif(data),
            0xE1 => data.strip_prefix(b"Exif\0\0").and_then(exif),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
        b = b.get(2 + len..)?;
    }
}

/// JFIF's density: unit 1 dots per inch, 2 per centimetre, 0 an aspect ratio only.
fn jfif(d: &[u8]) -> Option<u32> {
    let d = d.strip_prefix(b"JFIF\0")?;
    let x = be16(d, 3)?;
    match d.get(2)? {
        1 => Some(x),
        2 => Some((x * 254 + 50) / 100),
        _ => None,
    }
}

const X_RESOLUTION: u16 = 0x011A;
const RESOLUTION_UNIT: u16 = 0x0128;
const RATIONAL: u16 = 5;

/// IFD0's XResolution in its ResolutionUnit: 2 (the default) inches, 3 centimetres.
fn exif(t: &[u8]) -> Option<u32> {
    let little = match t.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = t.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    let (mut res, mut unit) = (None, 2);
    for i in 0..u16_at(ifd)? as usize {
        let e = ifd + 2 + i * 12;
        match u16_at(e)? {
            X_RESOLUTION if u16_at(e + 2)? == RATIONAL => {
                let at = u32_at(e + 8)? as usize;
                let (num, den) = (u32_at(at)? as u64, u32_at(at + 4)? as u64);
                res = (den > 0).then(|| (num + den / 2) / den);
            }
            RESOLUTION_UNIT => unit = u16_at(e + 8)?,
            _ => {}
        }
    }
    let res = res?;
    let dpi = match unit {
        2 => res,
        3 => (res * 254 + 50) / 100,
        _ => return None,
    };
    u32::try_from(dpi).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut b = PNG_SIGNATURE.to_vec();
        for (kind, data) in chunks {
            b.extend((data.len() as u32).to_be_bytes());
            b.extend(*kind);
            b.extend(data);
            b.extend([0; 4]);
        }
        b
    }

    fn phys(per_metre: u32, unit: u8) -> Vec<u8> {
        let mut d = per_metre.to_be_bytes().to_vec();
        d.extend(per_metre.to_be_bytes());
        d.push(unit);
        d
    }

    #[test]
    fn a_png_states_its_dpi_in_phys_before_its_pixels() {
        let ihdr = (b"IHDR", vec![0; 13]);
        let ok = png_with(&[ihdr.clone(), (b"pHYs", phys(11811, 1)), (b"IDAT", vec![])]);
        assert_eq!(dpi(&ok), Some(300), "11811 per metre is 300 dpi");
        let late = png_with(&[ihdr.clone(), (b"IDAT", vec![]), (b"pHYs", phys(11811, 1))]);
        assert_eq!(dpi(&late), None, "pHYs after IDAT is not read");
        let aspect = png_with(&[ihdr.clone(), (b"pHYs", phys(1, 0)), (b"IDAT", vec![])]);
        assert_eq!(dpi(&aspect), None, "unit 0 is an aspect ratio only");
        let none = png_with(&[ihdr, (b"IDAT", vec![])]);
        assert_eq!(dpi(&none), None);
    }

    fn jpeg_with(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        for (marker, data) in segments {
            b.extend([0xFF, *marker]);
            b.extend(((data.len() + 2) as u16).to_be_bytes());
            b.extend(data);
        }
        b.extend([0xFF, 0xDA, 0, 2]);
        b
    }

    fn jfif_density(unit: u8, x: u16) -> Vec<u8> {
        let mut d = b"JFIF\0\x01\x02".to_vec();
        d.push(unit);
        d.extend(x.to_be_bytes());
        d.extend(x.to_be_bytes());
        d.extend([0, 0]);
        d
    }

    /// A big-endian EXIF block whose IFD0 holds XResolution `num/den` and ResolutionUnit `unit`.
    fn exif_res(num: u32, den: u32, unit: u16) -> Vec<u8> {
        let mut t = b"MM\0*".to_vec();
        t.extend(8u32.to_be_bytes());
        t.extend(2u16.to_be_bytes());
        let rational_at = 8 + 2 + 2 * 12 + 4;
        t.extend(X_RESOLUTION.to_be_bytes());
        t.extend(RATIONAL.to_be_bytes());
        t.extend(1u32.to_be_bytes());
        t.extend((rational_at as u32).to_be_bytes());
        t.extend(RESOLUTION_UNIT.to_be_bytes());
        t.extend(3u16.to_be_bytes());
        t.extend(1u32.to_be_bytes());
        t.extend(unit.to_be_bytes());
        t.extend([0, 0]);
        t.extend(0u32.to_be_bytes());
        t.extend(num.to_be_bytes());
        t.extend(den.to_be_bytes());
        let mut d = b"Exif\0\0".to_vec();
        d.extend(t);
        d
    }

    #[test]
    fn a_jpeg_states_its_dpi_in_jfif_or_exif() {
        assert_eq!(dpi(&jpeg_with(&[(0xE0, jfif_density(1, 150))])), Some(150));
        assert_eq!(
            dpi(&jpeg_with(&[(0xE0, jfif_density(2, 118))])),
            Some(300),
            "118 per centimetre is 300 dpi"
        );
        let aspect_then_exif = jpeg_with(&[(0xE0, jfif_density(0, 1)), (0xE1, exif_res(72, 1, 2))]);
        assert_eq!(
            dpi(&aspect_then_exif),
            Some(72),
            "JFIF without a unit defers to EXIF"
        );
        assert_eq!(dpi(&jpeg_with(&[(0xE1, exif_res(1200, 10, 3))])), Some(305));
        assert_eq!(dpi(&jpeg_with(&[(0xE0, jfif_density(0, 1))])), None);
    }

    #[test]
    fn an_implausible_or_cut_short_file_states_no_dpi() {
        assert_eq!(
            dpi(&jpeg_with(&[(0xE0, jfif_density(1, 1))])),
            None,
            "1 dpi"
        );
        let cut = jpeg_with(&[(0xE1, exif_res(300, 1, 2))]);
        assert_eq!(dpi(&cut[..30]), None);
        assert_eq!(dpi(b"GIF89a"), None);
    }
}
