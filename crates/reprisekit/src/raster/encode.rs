use super::Canvas;

/// Write `canvas` as an opaque RGB PNG named `name` under the workspace's
/// `target/` and print where. For the ignored preview tests and the
/// gallery; panics when the file cannot be written.
pub fn write_preview(canvas: &Canvas, name: &str) {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .find(|p| p.join("target").is_dir())
        .unwrap_or(manifest);
    let path = root.join("target").join(name);
    std::fs::write(&path, to_png(canvas)).unwrap();
    eprintln!("wrote {}", path.display());
}

/// `canvas` as an opaque RGB PNG file's bytes at its device size; alpha is
/// dropped. Fast compression: a screen-sized canvas encodes on the
/// compositor's loop.
pub fn to_png(canvas: &Canvas) -> Vec<u8> {
    let mut bytes = Vec::new();
    let (w, h) = canvas.device_size();
    let mut enc = png::Encoder::new(&mut bytes, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let rgb: Vec<u8> = canvas
        .pixels
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    // Writing into a Vec only fails on a size png rejects, which a
    // canvas's u32 sides and matching pixel count never are.
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&rgb).expect("png data");
    w.finish().expect("png end");
    bytes
}
