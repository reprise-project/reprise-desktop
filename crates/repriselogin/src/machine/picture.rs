use image::error::{ImageError, LimitErrorKind};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder as _, ImageFormat};

use super::{MAX_PICTURE, PICTURE_MAX_PIXELS, PICTURE_MAX_SIDE};

const NOT_A_PICTURE: &str = "the picture is not a PNG, JPEG or BMP image the login screen can show";

/// `bytes` decoded, with the orientation that turns them upright, when `reprise-login-settings`
/// takes them: a PNG, JPEG or BMP of at most [`MAX_PICTURE`] bytes, [`PICTURE_MAX_SIDE`] a side
/// and [`PICTURE_MAX_PIXELS`] pixels, within `image`'s default memory limit, that decodes whole.
/// Settings runs it before asking for a password.
pub fn decode_picture(bytes: &[u8]) -> Result<(DynamicImage, Orientation), String> {
    if bytes.len() > MAX_PICTURE {
        return Err(format!("the picture is over {} MB", MAX_PICTURE >> 20));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| NOT_A_PICTURE.to_string())?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp)
    ) {
        return Err(NOT_A_PICTURE.into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(PICTURE_MAX_SIDE);
    limits.max_image_height = Some(PICTURE_MAX_SIDE);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(refused)?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > PICTURE_MAX_PIXELS {
        return Err(format!(
            "the picture has more than {} million pixels",
            PICTURE_MAX_PIXELS / 1_000_000
        ));
    }
    limits.reserve(decoder.total_bytes()).map_err(refused)?;
    decoder.set_limits(limits).map_err(refused)?;
    let orientation = (decoder.orientation()).unwrap_or(Orientation::NoTransforms);
    let picture = DynamicImage::from_decoder(decoder).map_err(refused)?;
    Ok((picture, orientation))
}

fn refused(e: ImageError) -> String {
    match e {
        ImageError::Limits(l) if matches!(l.kind(), LimitErrorKind::DimensionError) => {
            format!("the picture is wider or taller than {PICTURE_MAX_SIDE} pixels")
        }
        ImageError::Limits(_) => "the picture has too many pixels to decode".into(),
        ImageError::Unsupported(_) => NOT_A_PICTURE.into(),
        _ => "the picture is damaged or cut short".into(),
    }
}
