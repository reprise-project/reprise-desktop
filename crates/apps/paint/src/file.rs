//! Reading and writing pictures. Anything `repriseapp::decode` reads opens; PNG, JPEG and BMP
//! save, the type following the name's extension.

use crate::bitmap::Bitmap;
use image::ImageEncoder;
use repriseapp::decode::{Budget, Decoders, Unread};
use reprisekit::tr;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Bmp,
}

pub const DEFAULT_QUALITY: u8 = 90;

impl Kind {
    pub fn of(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(Kind::Png),
            "jpg" | "jpeg" | "jpe" => Some(Kind::Jpeg),
            "bmp" | "dib" => Some(Kind::Bmp),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Bmp => "bmp",
        }
    }
}

/// `bytes` as a picture, turned upright by its Exif orientation, transparent pixels on white.
pub fn decode(bytes: &[u8]) -> Result<Bitmap, String> {
    let d = Decoders::default_set()
        .decode(bytes, (u32::MAX, u32::MAX), Budget::viewer())
        .map_err(|e| match e {
            Unread::TooLarge => tr!("picture-too-large"),
            Unread::Unreadable => tr!("unreadable"),
        })?;
    Ok(Bitmap::from_canvas(&d.canvas))
}

/// The picture in the regular file `path`; a FIFO or device is refused, never opened to read.
pub fn load(path: &Path) -> Result<Bitmap, String> {
    let cap = Budget::viewer().file;
    let bytes = reprisefs::ops::read_regular(path, cap).map_err(|e| match e.kind() {
        std::io::ErrorKind::FileTooLarge => tr!("picture-too-large"),
        _ => e.to_string(),
    })?;
    decode(&bytes)
}

/// `img` encoded as `kind`; `quality` (1-100) is JPEG's.
pub fn encode(img: &Bitmap, kind: Kind, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb();
    let (w, h) = (img.w as u32, img.h as u32);
    let mut out = Vec::new();
    let ty = image::ExtendedColorType::Rgb8;
    match kind {
        Kind::Png => image::codecs::png::PngEncoder::new(&mut out).write_image(&rgb, w, h, ty),
        Kind::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .write_image(&rgb, w, h, ty)
        }
        Kind::Bmp => image::codecs::bmp::BmpEncoder::new(&mut out).write_image(&rgb, w, h, ty),
    }
    .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Writes `img` to `path` so a failed save leaves the old file whole
/// ([`reprisefs::ops::write_document`]).
pub fn save(img: &Bitmap, path: &Path, kind: Kind, quality: u8) -> Result<(), String> {
    let bytes = encode(img, kind, quality)?;
    reprisefs::ops::write_document(path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmap::{BLACK, WHITE};

    fn sample() -> Bitmap {
        let mut b = Bitmap::new(5, 3, WHITE);
        b.set(0, 0, BLACK);
        b.set(4, 2, 0xFF12_3456);
        b
    }

    #[test]
    fn png_and_bmp_round_trip_exactly() {
        let dir = tempfile::tempdir().unwrap();
        for kind in [Kind::Png, Kind::Bmp] {
            let path = dir.path().join(format!("pic.{}", kind.ext()));
            save(&sample(), &path, kind, DEFAULT_QUALITY).unwrap();
            assert_eq!(Kind::of(&path), Some(kind));
            assert_eq!(load(&path).unwrap(), sample(), "{kind:?}");
        }
    }

    #[test]
    fn jpeg_keeps_the_size_and_quality_changes_the_bytes() {
        let mut noisy = Bitmap::new(32, 32, 0xFF33_6699);
        for i in 0..noisy.px.len() {
            noisy.px[i] ^= (i as u32).wrapping_mul(2_654_435_761) & 0x003F_3F3F;
        }
        let low = encode(&noisy, Kind::Jpeg, 10).unwrap();
        let high = encode(&noisy, Kind::Jpeg, 100).unwrap();
        assert!(low.len() < high.len());
        let back = decode(&high).unwrap();
        assert_eq!((back.w, back.h), (32, 32));
    }

    #[test]
    fn a_save_replaces_the_file_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.png");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        save(&sample(), &path, Kind::Png, DEFAULT_QUALITY).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temporary file left"
        );
    }

    #[test]
    fn a_save_through_a_symlink_writes_its_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("shared")).unwrap();
        let real = dir.path().join("shared/logo.png");
        let link = dir.path().join("logo.png");
        std::fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink("shared/logo.png", &link).unwrap();
        save(&sample(), &link, Kind::Png, DEFAULT_QUALITY).unwrap();
        let lstat = std::fs::symlink_metadata(&link).unwrap();
        assert!(lstat.file_type().is_symlink(), "the link stays a link");
        assert_eq!(load(&real).unwrap(), sample());
    }

    #[test]
    fn a_new_file_gets_the_mode_any_new_file_gets() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let probe = dir.path().join("probe");
        std::fs::File::create(&probe).unwrap();
        let path = dir.path().join("pic.png");
        save(&sample(), &path, Kind::Png, DEFAULT_QUALITY).unwrap();
        assert_eq!(mode(&path), mode(&probe));
    }

    /// An 8x4 JPEG, left half red and right half blue, tagged Exif orientation 6: shown turned a
    /// quarter clockwise, 4x8 with red on top.
    #[test]
    fn a_jpeg_tagged_to_turn_opens_upright() {
        let img = image::RgbImage::from_fn(8, 4, |x, _| {
            image::Rgb(if x < 4 { [255, 0, 0] } else { [0, 0, 255] })
        });
        // A big-endian TIFF header and one IFD entry: Orientation (0x0112), SHORT, count 1, value 6.
        let exif = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0,
            0,
        ];
        let mut bytes = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
        enc.set_exif_metadata(exif.to_vec()).unwrap();
        enc.write_image(img.as_raw(), 8, 4, image::ExtendedColorType::Rgb8)
            .unwrap();
        let b = decode(&bytes).unwrap();
        assert_eq!((b.w, b.h), (4, 8));
        let [_, r, _, bl] = b.get(1, 1).unwrap().to_be_bytes();
        assert!(r > 200 && bl < 60, "top is red");
        let [_, r, _, bl] = b.get(1, 6).unwrap().to_be_bytes();
        assert!(r < 60 && bl > 200, "bottom is blue");
    }

    /// A lossless 3x2 JPEG XL of 0xC81E1E.
    const JXL: [u8; 26] = [
        0xff, 0x0a, 0x08, 0x40, 0x10, 0x09, 0x08, 0x00, 0x01, 0x00, 0x38, 0x00, 0x4b, 0x18, 0x8b,
        0x15, 0xc2, 0x49, 0x82, 0xe5, 0x01, 0xc4, 0x38, 0x00, 0xa6, 0x02,
    ];

    #[test]
    fn a_picture_past_the_viewer_budget_is_too_large_and_junk_is_unreadable() {
        let huge =
            b"P7\nWIDTH 65535\nHEIGHT 65535\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n";
        assert_eq!(decode(huge).err(), Some(tr!("picture-too-large")));
        assert_eq!(decode(b"not a picture").err(), Some(tr!("unreadable")));
    }

    #[test]
    fn a_jpeg_xl_opens() {
        let b = decode(&JXL).unwrap();
        assert_eq!((b.w, b.h), (3, 2));
        assert!(b.px.iter().all(|&c| c == 0xFFC8_1E1E), "{:08x?}", b.px);
    }

    #[test]
    fn extensions_pick_the_kind() {
        assert_eq!(Kind::of(Path::new("/home/u/a.JPEG")), Some(Kind::Jpeg));
        assert_eq!(Kind::of(Path::new("/home/u/a.gif")), None);
        assert_eq!(Kind::of(Path::new("/home/u/a")), None);
    }
}
