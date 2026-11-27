//! A PDF of raster pages: each page is one picture placed on it, deflated. Enough for cupsd's
//! filters to print what a program drew; no fonts, no vector art.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use reprisekit::raster::Canvas;
use std::io::Write;

/// A picture's 8-bit samples, deflated: a byte per pixel when every pixel is grey, else three.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub gray: bool,
    pub deflated: Vec<u8>,
}

impl Image {
    /// `c` laid over white paper, deflated a row at a time so no raw copy of it is made.
    pub fn from_canvas(c: &Canvas) -> Image {
        let (w, h) = c.device_size();
        let rgb = |p: u32| {
            let a = p >> 24;
            let paper = 255 - a;
            [(p >> 16) & 255, (p >> 8) & 255, p & 255].map(|v| (v + paper).min(255) as u8)
        };
        let gray = c.pixels.iter().all(|&p| {
            let [r, g, b] = rgb(p);
            r == g && g == b
        });
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        let mut row = Vec::with_capacity(w as usize * 3);
        for line in c.pixels.chunks(w.max(1) as usize) {
            row.clear();
            match gray {
                true => row.extend(line.iter().map(|&p| rgb(p)[0])),
                false => row.extend(line.iter().flat_map(|&p| rgb(p))),
            }
            z.write_all(&row).expect("writing to a Vec cannot fail");
        }
        Image {
            width: w,
            height: h,
            gray,
            deflated: z.finish().expect("writing to a Vec cannot fail"),
        }
    }
}

/// A page `size` points (1/72 inch) wide and tall with `image` drawn into `at`: x and y from the
/// page's top left, width and height, all in points.
pub struct Page {
    pub size: (f64, f64),
    pub image: Image,
    pub at: [f64; 4],
}

/// The document as its pages come: a catalog and the page tree for `count` pages first, then a
/// page, its content and its picture per [`Writer::page`], and at [`Writer::finish`] the
/// cross-reference table that finds them.
pub struct Writer {
    out: Vec<u8>,
    offsets: Vec<usize>,
    count: usize,
}

fn page_id(i: usize) -> usize {
    3 + 3 * i
}

impl Writer {
    pub fn new(count: usize) -> Writer {
        let mut w = Writer {
            out: b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec(),
            offsets: Vec::new(),
            count,
        };
        w.object(b"<< /Type /Catalog /Pages 2 0 R >>");
        let kids: Vec<String> = (0..count).map(|i| format!("{} 0 R", page_id(i))).collect();
        let tree = format!(
            "<< /Type /Pages /Kids [{}] /Count {count} >>",
            kids.join(" ")
        );
        w.object(tree.as_bytes());
        w
    }

    fn object(&mut self, body: &[u8]) {
        self.offsets.push(self.out.len());
        self.out
            .extend(format!("{} 0 obj\n", self.offsets.len()).as_bytes());
        self.out.extend(body);
        self.out.extend(b"\nendobj\n");
    }

    /// The next page; more than `count` pages is a bug in the caller.
    pub fn page(&mut self, p: &Page) {
        let i = (self.offsets.len() - 2) / 3;
        assert!(i < self.count, "page {i} of {}", self.count);
        let (w, h) = p.size;
        let [x, y, iw, ih] = p.at;
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R \
             /Resources << /XObject << /Im0 {} 0 R >> >> >>",
            num(w),
            num(h),
            page_id(i) + 1,
            page_id(i) + 2
        );
        self.object(page.as_bytes());
        let draw = format!(
            "q {} 0 0 {} {} {} cm /Im0 Do Q",
            num(iw),
            num(ih),
            num(x),
            num(h - y - ih)
        );
        self.object(&stream("", draw.as_bytes()));
        let img = &p.image;
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} \
             /BitsPerComponent 8 /Filter /FlateDecode ",
            img.width,
            img.height,
            if img.gray { "DeviceGray" } else { "DeviceRGB" }
        );
        self.object(&stream(&dict, &img.deflated));
    }

    pub fn finish(mut self) -> Vec<u8> {
        let xref = self.out.len();
        let size = self.offsets.len() + 1;
        self.out
            .extend(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for o in &self.offsets {
            self.out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        self.out.extend(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        self.out
    }
}

/// The document of `pages` in one go.
pub fn write(pages: &[Page]) -> Vec<u8> {
    let mut w = Writer::new(pages.len());
    for p in pages {
        w.page(p);
    }
    w.finish()
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict}/Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend(data);
    s.extend(b"\nendstream");
    s
}

/// `v` with at most two decimals, as PDF numbers are written.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use reprisekit::raster::Rect;
    use reprisekit::theme::Color;
    use std::io::Read;

    fn page(c: &Canvas) -> Page {
        Page {
            size: (595.28, 841.89),
            image: Image::from_canvas(c),
            at: [56.69, 70.87, 481.89, 700.15],
        }
    }

    fn inflate(img: &Image) -> Vec<u8> {
        let mut got = Vec::new();
        ZlibDecoder::new(&img.deflated[..])
            .read_to_end(&mut got)
            .unwrap();
        got
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn every_object_sits_where_the_cross_reference_table_says() {
        let c = Canvas::new(4, 3);
        let pdf = write(&[page(&c), page(&c)]);
        let tail = String::from_utf8_lossy(&pdf[pdf.len() - 64..]).into_owned();
        let xref: usize = tail
            .rsplit("startxref\n")
            .next()
            .and_then(|t| t.lines().next())
            .and_then(|n| n.parse().ok())
            .expect("startxref names the table");
        let table = std::str::from_utf8(&pdf[xref..]).expect("the table is text");
        assert!(table.starts_with("xref\n0 9\n"), "8 objects for two pages");
        for (n, line) in table.lines().skip(3).take(8).enumerate() {
            let at: usize = line[..10].parse().unwrap();
            assert!(
                pdf[at..].starts_with(format!("{} 0 obj", n + 1).as_bytes()),
                "object {} at {at}",
                n + 1
            );
        }
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/MediaBox [0 0 595.28 841.89]"));
    }

    #[test]
    fn a_picture_lands_at_its_place_from_the_page_top() {
        let pdf = write(&[page(&Canvas::new(2, 2))]);
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("q 481.89 0 0 700.15 56.69 70.87 cm /Im0 Do Q"),
            "y is 841.89 - 70.87 - 700.15 from the bottom"
        );
    }

    #[test]
    fn grey_ink_is_one_byte_a_pixel_over_white_and_colour_three() {
        let mut c = Canvas::new(2, 1);
        c.fill_rect(Rect::new(0, 0, 1, 1), Color::rgb(0, 0, 0));
        let img = Image::from_canvas(&c);
        assert!(img.gray);
        assert_eq!(inflate(&img), vec![0, 255], "the clear pixel is paper");
        c.fill_rect(Rect::new(1, 0, 1, 1), Color::rgb(200, 0, 0));
        let img = Image::from_canvas(&c);
        assert!(!img.gray);
        assert_eq!(inflate(&img), vec![0, 0, 0, 200, 0, 0]);
    }

    #[test]
    fn the_picture_stream_inflates_back_to_its_samples() {
        let mut c = Canvas::new(3, 2);
        c.fill_rect(Rect::new(1, 0, 1, 2), Color::rgb(0, 0, 0));
        let pdf = write(&[page(&c)]);
        let dict = find(&pdf, b"/Subtype /Image").unwrap();
        let start = dict + find(&pdf[dict..], b"stream\n").unwrap() + 7;
        let end = start + find(&pdf[start..], b"\nendstream").unwrap();
        let mut got = Vec::new();
        ZlibDecoder::new(&pdf[start..end])
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, vec![255, 0, 255, 255, 0, 255]);
    }
}
