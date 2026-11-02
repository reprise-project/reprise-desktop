//! What a font file says of itself: its faces (one, or several in a collection) and each face's
//! names from its `name` table, read through FreeType so WOFF and WOFF2 files read as well.

use freetype::{Face, Library, ffi};
use std::path::{Path, PathBuf};

/// One face's names; an empty string is a name the file does not give.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceNames {
    /// The face's index in its file, which FreeType opens it by.
    pub index: u32,
    pub family: String,
    pub style: String,
    pub full_name: String,
    pub postscript: String,
    pub version: String,
    pub copyright: String,
}

impl FaceNames {
    /// The face's name as a person reads it: its full name, else its family and style.
    pub fn title(&self) -> String {
        match (&self.full_name, &self.style) {
            (n, _) if !n.is_empty() => n.clone(),
            (_, s) if s.is_empty() || s == "Regular" => self.family.clone(),
            (_, s) => format!("{} {s}", self.family),
        }
    }
}

/// `name` table name IDs.
const COPYRIGHT: u16 = 0;
const FAMILY: u16 = 1;
const SUBFAMILY: u16 = 2;
const FULL_NAME: u16 = 4;
const VERSION: u16 = 5;
const POSTSCRIPT: u16 = 6;
const TYPO_FAMILY: u16 = 16;
const TYPO_SUBFAMILY: u16 = 17;

/// Platform IDs: Unicode, Macintosh, Windows.
const UNICODE: u16 = 0;
const MAC: u16 = 1;
const WINDOWS: u16 = 3;
/// Windows' US English language ID.
const EN_US: u16 = 0x409;

/// One `name` table record as FreeType hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameRecord {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub name_id: u16,
    pub bytes: Vec<u8>,
}

/// The faces of the font file at `path`, in file order; `None` when FreeType cannot open it.
pub fn faces(path: &Path) -> Option<Vec<FaceNames>> {
    let lib = Library::init().ok()?;
    let first = lib.new_face(path.as_os_str(), 0).ok()?;
    let count = first.num_faces().max(1) as u32;
    let mut out = vec![names_of(&first, 0)];
    for i in 1..count {
        if let Ok(f) = lib.new_face(path.as_os_str(), i as isize) {
            out.push(names_of(&f, i));
        }
    }
    Some(out)
}

fn names_of(face: &Face, index: u32) -> FaceNames {
    let mut n = from_records(&records(face));
    n.index = index;
    if n.family.is_empty() {
        n.family = face.family_name().unwrap_or_default();
    }
    if n.style.is_empty() {
        n.style = face.style_name().unwrap_or_default();
    }
    if n.postscript.is_empty() {
        n.postscript = face.postscript_name().unwrap_or_default();
    }
    n
}

fn records(face: &Face) -> Vec<NameRecord> {
    let raw = face.raw() as *const ffi::FT_FaceRec as ffi::FT_Face;
    // SAFETY: `raw` is the live face `face` borrows for this call; FreeType only reads it here.
    let count = unsafe { ffi::FT_Get_Sfnt_Name_Count(raw) };
    (0..count)
        .filter_map(|i| {
            let mut n = std::mem::MaybeUninit::<ffi::FT_SfntName>::uninit();
            // SAFETY: `i` is under the count FreeType gave; on success it fills `n`, whose string
            // points into the face's own memory for as long as the face lives.
            let n = unsafe {
                if ffi::FT_Get_Sfnt_Name(raw, i, n.as_mut_ptr()) != 0 {
                    return None;
                }
                n.assume_init()
            };
            let bytes = if n.string.is_null() {
                Vec::new()
            } else {
                // SAFETY: FreeType gives `string_len` bytes at `string`, alive with the face.
                unsafe { std::slice::from_raw_parts(n.string, n.string_len as usize) }.to_vec()
            };
            Some(NameRecord {
                platform: n.platform_id,
                encoding: n.encoding_id,
                language: n.language_id,
                name_id: n.name_id,
                bytes,
            })
        })
        .collect()
}

/// The names `records` give: per name ID, the US English Windows record, else any Windows or
/// Unicode record, else a Macintosh one. A typographic family and subfamily (IDs 16 and 17) win
/// over the four-style ones.
pub fn from_records(records: &[NameRecord]) -> FaceNames {
    let get = |id: u16| -> String {
        let rank = |r: &NameRecord| match (r.platform, r.language) {
            (WINDOWS, EN_US) => 0,
            (WINDOWS, _) | (UNICODE, _) => 1,
            (MAC, 0) => 2,
            _ => 3,
        };
        records
            .iter()
            .filter(|r| r.name_id == id)
            .filter_map(|r| Some((rank(r), decode(r)?)))
            .filter(|(_, s)| !s.trim().is_empty())
            .min_by_key(|(k, _)| *k)
            .map(|(_, s)| s.trim().to_string())
            .unwrap_or_default()
    };
    let or = |a: String, b: String| if a.is_empty() { b } else { a };
    FaceNames {
        index: 0,
        family: or(get(TYPO_FAMILY), get(FAMILY)),
        style: or(get(TYPO_SUBFAMILY), get(SUBFAMILY)),
        full_name: get(FULL_NAME),
        postscript: get(POSTSCRIPT),
        version: get(VERSION),
        copyright: get(COPYRIGHT),
    }
}

/// A record's text: UTF-16BE on the Unicode and Windows platforms, Mac Roman read as Latin-1 on
/// Macintosh's Roman encoding; `None` for any other encoding.
fn decode(r: &NameRecord) -> Option<String> {
    match r.platform {
        UNICODE | WINDOWS => {
            let units: Vec<u16> = r
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| u16::from_be_bytes(b))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        MAC if r.encoding == 0 => Some(r.bytes.iter().map(|&b| b as char).collect()),
        _ => None,
    }
}

/// The files of installed faces whose PostScript name is `postscript`, from the font database
/// text is set from.
pub fn installed(postscript: &str) -> Vec<PathBuf> {
    if postscript.is_empty() {
        return Vec::new();
    }
    let db = super::system_fonts(&[]);
    db.faces()
        .filter(|f| f.post_script_name == postscript)
        .filter_map(|f| match &f.source {
            fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => Some(p.clone()),
            fontdb::Source::Binary(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(platform: u16, language: u16, name_id: u16, s: &str) -> NameRecord {
        NameRecord {
            platform,
            encoding: 1,
            language,
            name_id,
            bytes: s.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        }
    }

    #[test]
    fn names_prefer_us_english_windows_records_and_typographic_families() {
        let mac = NameRecord {
            platform: MAC,
            encoding: 0,
            language: 0,
            name_id: COPYRIGHT,
            bytes: b"\xa9 Mac".to_vec(),
        };
        let records = [
            mac,
            utf16(WINDOWS, 0x407, FAMILY, "Familie"),
            utf16(WINDOWS, EN_US, FAMILY, "Sample Sans Light"),
            utf16(WINDOWS, EN_US, SUBFAMILY, "Regular"),
            utf16(WINDOWS, EN_US, TYPO_FAMILY, "Sample Sans"),
            utf16(WINDOWS, EN_US, TYPO_SUBFAMILY, "Light"),
            utf16(WINDOWS, EN_US, VERSION, " Version 2.001 "),
            utf16(WINDOWS, EN_US, POSTSCRIPT, "SampleSans-Light"),
        ];
        let n = from_records(&records);
        assert_eq!(n.family, "Sample Sans");
        assert_eq!(n.style, "Light");
        assert_eq!(n.version, "Version 2.001");
        assert_eq!(n.postscript, "SampleSans-Light");
        assert_eq!(
            n.copyright, "\u{a9} Mac",
            "Mac Roman when no other record has it"
        );
        assert_eq!(n.full_name, "");
        assert_eq!(n.title(), "Sample Sans Light");
    }

    #[test]
    fn an_unknown_mac_encoding_is_skipped() {
        let r = NameRecord {
            platform: MAC,
            encoding: 1,
            language: 0,
            name_id: FAMILY,
            bytes: vec![0x82, 0xa0],
        };
        assert_eq!(from_records(&[r]).family, "");
    }

    #[test]
    fn the_test_tahoma_reads_its_name_table() {
        if !crate::text::test_tahoma() {
            return;
        }
        let dir = super::super::fonts::test_fonts_dir().expect("the test fonts");
        let bold = faces(&dir.join("tahomabd.ttf")).expect("a font");
        assert_eq!(bold.len(), 1);
        let f = &bold[0];
        assert_eq!((f.family.as_str(), f.style.as_str()), ("Tahoma", "Bold"));
        assert_eq!(f.postscript, "WineTahomaBold");
        assert!(f.version.starts_with("Version"), "{:?}", f.version);
        assert!(!f.copyright.is_empty());
        assert!(faces(&dir).is_none(), "a folder is no font");
    }

    #[test]
    fn the_test_tahoma_is_installed_from_its_folder() {
        if !crate::text::test_tahoma() {
            return;
        }
        let dir = super::super::fonts::test_fonts_dir().expect("the test fonts");
        let dir = dir.canonicalize().unwrap_or(dir);
        let face = faces(&dir.join("tahoma.ttf")).expect("a font").remove(0);
        let at = installed(&face.postscript);
        assert!(at.iter().any(|p| p.starts_with(&dir)), "{at:?}");
        assert!(installed("NoSuchFace-Regular").is_empty());
        assert!(installed("").is_empty());
    }
}
