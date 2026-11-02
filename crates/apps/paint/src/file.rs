//! Reading and writing pictures. The file type follows the name's extension; anything the image
//! decoders read opens, and PNG, JPEG and BMP save.

use crate::bitmap::Bitmap;
use image::ImageEncoder;
use std::io::Write;
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

pub fn decode(bytes: &[u8]) -> Result<Bitmap, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    Ok(Bitmap::from_rgba(
        rgba.width(),
        rgba.height(),
        rgba.as_raw(),
    ))
}

pub fn load(path: &Path) -> Result<Bitmap, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
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

/// Writes `img` to `path` through a temporary file beside it, so a failed save leaves the old
/// file whole.
pub fn save(img: &Bitmap, path: &Path, kind: Kind, quality: u8) -> Result<(), String> {
    let bytes = encode(img, kind, quality)?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile_in(dir)
        .map_err(|e| e.to_string())?;
    tmp.write_all(&bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(tmp.path(), meta.permissions());
    }
    tmp.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
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

    #[test]
    fn extensions_pick_the_kind() {
        assert_eq!(Kind::of(Path::new("/home/u/a.JPEG")), Some(Kind::Jpeg));
        assert_eq!(Kind::of(Path::new("/home/u/a.gif")), None);
        assert_eq!(Kind::of(Path::new("/home/u/a")), None);
    }
}
