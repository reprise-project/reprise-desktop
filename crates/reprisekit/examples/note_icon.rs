//! Writes Sticky Notes' icon at each hicolor size into `data/icons/hicolor`, from
//! `reprisekit::art::noteicon`.

use reprisekit::art::noteicon;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons/hicolor");
    for n in noteicon::SIZES {
        let dir = root.join(format!("{n}x{n}/apps"));
        std::fs::create_dir_all(&dir).expect("make the size directory");
        let path = dir.join("reprise-notes.png");
        let file = std::fs::File::create(&path).expect("create the png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), n as u32, n as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&noteicon::rgba(&noteicon::draw(n)))
            .expect("png data");
        w.finish().expect("png end");
        println!("wrote {}", path.display());
    }
}
