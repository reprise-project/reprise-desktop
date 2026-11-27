//! Notepad file I/O: reading, encodings and line ends.

use reprisekit::tr;
use std::path::{Path, PathBuf};

/// What a save checks its file against: any field that differs means another program wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    mtime: i64,
    mtime_nsec: i64,
    len: u64,
    ino: u64,
}

impl Stamp {
    /// The file `path` names, through a symlink, whose own times stay put when its target is
    /// rewritten. `None` when there is no file.
    pub(crate) fn of(path: &Path) -> Option<Stamp> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some(Stamp {
            mtime: m.mtime(),
            mtime_nsec: m.mtime_nsec(),
            len: m.len(),
            ino: m.ino(),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    Cp1252,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineEnd {
    #[default]
    Lf,
    Crlf,
    Cr,
}

impl LineEnd {
    /// Any `\r\n` makes a file CRLF, so a mixed file stays CRLF; a lone `\r` makes it CR only
    /// when it has no `\n` at all.
    pub(crate) fn of(text: &str) -> LineEnd {
        if text.contains("\r\n") {
            LineEnd::Crlf
        } else if text.contains('\r') && !text.contains('\n') {
            LineEnd::Cr
        } else {
            LineEnd::Lf
        }
    }

    pub(crate) fn sep(self) -> &'static str {
        match self {
            LineEnd::Lf => "\n",
            LineEnd::Crlf => "\r\n",
            LineEnd::Cr => "\r",
        }
    }

    /// `text` with its breaks as the edit splits them: CR ones become LF.
    pub(crate) fn to_edit(self, text: &str) -> std::borrow::Cow<'_, str> {
        match self {
            LineEnd::Cr => text.replace('\r', "\n").into(),
            LineEnd::Lf | LineEnd::Crlf => text.into(),
        }
    }
}

/// How a document's bytes sit on disk, which a save writes back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Format {
    pub(crate) encoding: Encoding,
    /// A UTF-16 file always has one.
    pub(crate) bom: bool,
    pub(crate) line_end: LineEnd,
}

const CP1252_NAME: &str = "Western (CP1252)";

/// The Save As encoding rows, in the combo's order.
pub(crate) const ENCODINGS: [(&str, Encoding, bool); 5] = [
    ("UTF-8", Encoding::Utf8, false),
    ("UTF-8 with BOM", Encoding::Utf8, true),
    ("UTF-16 LE", Encoding::Utf16Le, true),
    ("UTF-16 BE", Encoding::Utf16Be, true),
    (CP1252_NAME, Encoding::Cp1252, false),
];

/// The Open encoding rows under Auto-Detect, in the combo's order.
pub(crate) const READ_AS: [(&str, Encoding); 4] = [
    ("UTF-8", Encoding::Utf8),
    ("UTF-16 LE", Encoding::Utf16Le),
    ("UTF-16 BE", Encoding::Utf16Be),
    (CP1252_NAME, Encoding::Cp1252),
];

impl Format {
    /// This format's row in `ENCODINGS`.
    pub(crate) fn encoding_row(self) -> usize {
        ENCODINGS
            .iter()
            .position(|&(_, e, bom)| e == self.encoding && bom == self.bom)
            .unwrap_or(0)
    }

    /// This format with the encoding of `ENCODINGS[row]`.
    pub(crate) fn with_encoding_row(self, row: usize) -> Format {
        let (_, encoding, bom) = ENCODINGS.get(row).copied().unwrap_or(ENCODINGS[0]);
        Format {
            encoding,
            bom,
            ..self
        }
    }

    /// Whether every character of `lines` has bytes in this format's encoding.
    pub(crate) fn holds(self, lines: &[impl AsRef<str>]) -> bool {
        self.encoding != Encoding::Cp1252
            || (lines.iter()).all(|l| l.as_ref().chars().all(|c| cp1252::byte(c).is_some()))
    }

    /// This format in UTF-8 without a BOM, which holds any text.
    pub(crate) fn utf8(self) -> Format {
        Format {
            encoding: Encoding::Utf8,
            bom: false,
            ..self
        }
    }
}

const BOM_UTF8: &[u8] = b"\xEF\xBB\xBF";
const BOM_UTF16LE: &[u8] = b"\xFF\xFE";
const BOM_UTF16BE: &[u8] = b"\xFE\xFF";

/// A file whose bytes do not decode in the encoding it was read as.
#[derive(Debug)]
pub(crate) struct Undecodable {
    pub(crate) encoding: Encoding,
    /// The text with each bad sequence replaced by U+FFFD.
    pub(crate) lossy: String,
}

impl std::fmt::Display for Undecodable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not valid {}", encoding_name(self.encoding))
    }
}

impl std::error::Error for Undecodable {}

pub(crate) fn encoding_name(e: Encoding) -> &'static str {
    match e {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf16Le | Encoding::Utf16Be => "UTF-16",
        Encoding::Cp1252 => CP1252_NAME,
    }
}

fn bom_of(e: Encoding) -> &'static [u8] {
    match e {
        Encoding::Utf8 => BOM_UTF8,
        Encoding::Utf16Le => BOM_UTF16LE,
        Encoding::Utf16Be => BOM_UTF16BE,
        Encoding::Cp1252 => b"",
    }
}

/// A file's text and format, read as `read_as`, or with `None` as its UTF-8 or UTF-16 BOM says,
/// else as UTF-8 when it is valid UTF-8, else as CP1252. A BOM of the encoding read is not part
/// of the text. CP1252 gives every byte a character, so a file read as it always decodes.
pub(crate) fn decode(
    bytes: &[u8],
    read_as: Option<Encoding>,
) -> Result<(String, Format), Undecodable> {
    let sniffed = [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be]
        .into_iter()
        .find(|&e| bytes.starts_with(bom_of(e)));
    let encoding = match read_as.or(sniffed) {
        Some(e) => e,
        None if std::str::from_utf8(bytes).is_ok() => Encoding::Utf8,
        None => Encoding::Cp1252,
    };
    let bom = bom_of(encoding);
    let has_bom = !bom.is_empty() && bytes.starts_with(bom);
    let body = &bytes[if has_bom { bom.len() } else { 0 }..];
    let text = match encoding {
        Encoding::Utf8 => std::str::from_utf8(body)
            .map(str::to_string)
            .map_err(|_| String::from_utf8_lossy(body).into_owned()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let unit = if encoding == Encoding::Utf16Le {
                u16::from_le_bytes
            } else {
                u16::from_be_bytes
            };
            let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|&c| unit(c)).collect();
            match String::from_utf16(&units) {
                Ok(t) if body.len().is_multiple_of(2) => Ok(t),
                Ok(t) => Err(t + "\u{fffd}"),
                Err(_) => Err(String::from_utf16_lossy(&units)),
            }
        }
        Encoding::Cp1252 => Ok(body.iter().map(|&b| cp1252::char(b)).collect()),
    }
    .map_err(|lossy| Undecodable { encoding, lossy })?;
    let format = Format {
        encoding,
        bom: has_bom || matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be),
        line_end: LineEnd::of(&text),
    };
    Ok((text, format))
}

/// `lines` joined by `format`'s line end, in its encoding, after its BOM; `None` when the
/// encoding cannot hold a character of them.
pub(crate) fn encode(lines: &[impl AsRef<str>], format: Format) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    if format.bom {
        out.extend_from_slice(bom_of(format.encoding));
    }
    for (i, line) in lines.iter().enumerate() {
        let sep = if i > 0 { format.line_end.sep() } else { "" };
        for part in [sep, line.as_ref()] {
            match format.encoding {
                Encoding::Utf8 => out.extend_from_slice(part.as_bytes()),
                Encoding::Utf16Le => part
                    .encode_utf16()
                    .for_each(|u| out.extend_from_slice(&u.to_le_bytes())),
                Encoding::Utf16Be => part
                    .encode_utf16()
                    .for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
                Encoding::Cp1252 => {
                    for c in part.chars() {
                        out.push(cp1252::byte(c)?);
                    }
                }
            }
        }
    }
    Some(out)
}

/// Code page 1252. Its five unassigned bytes stand for the C1 controls of the same number, so
/// every byte reads as a character and writes back as itself.
mod cp1252 {
    /// The characters of 0x80..=0x9F; 0x00..=0x7F and 0xA0..=0xFF are U+0000..=U+00FF.
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}',
        '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];

    pub(super) fn char(b: u8) -> char {
        match b {
            0x80..=0x9F => HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        }
    }

    pub(super) fn byte(c: char) -> Option<u8> {
        match u32::from(c) {
            n @ (0..=0x7F | 0xA0..=0xFF) => u8::try_from(n).ok(),
            _ => (HIGH.iter().position(|&h| h == c)).and_then(|i| u8::try_from(0x80 + i).ok()),
        }
    }
}

/// The error of a write `encode` refused.
pub(crate) fn unheld(format: Format) -> std::io::Error {
    let name = encoding_name(format.encoding);
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{name} cannot hold the text"),
    )
}

/// Largest file Notepad opens: the whole text lives in one edit buffer.
pub(crate) const MAX_OPEN_BYTES: u64 = 32 << 20;

/// [`read_text_as`] detecting the encoding.
pub(crate) fn read_text(path: &Path) -> std::io::Result<(String, Format)> {
    read_text_as(path, None)
}

/// Only a regular file up to `MAX_OPEN_BYTES` that `decode` reads, which an [`Undecodable`] inside
/// the error says otherwise; a FIFO or device fails without blocking.
pub(crate) fn read_text_as(
    path: &Path,
    read_as: Option<Encoding>,
) -> std::io::Result<(String, Format)> {
    let bytes = reprisefs::ops::read_regular(path, MAX_OPEN_BYTES)?;
    decode(&bytes, read_as).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Command-line open: a missing file is a new one at that path; other failures must not adopt the path, or a save would overwrite the original.
pub(crate) fn load(path: &Path) -> std::io::Result<(String, Format)> {
    match read_text(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        r => r,
    }
}

pub(crate) fn open_failed_text(path: &Path, e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    let path = path.display().to_string();
    match e.kind() {
        ErrorKind::NotFound => tr!("open-not-found", path = path),
        ErrorKind::FileTooLarge => tr!("open-too-large", path = path),
        ErrorKind::InvalidInput | ErrorKind::IsADirectory => tr!("open-not-a-file", path = path),
        ErrorKind::PermissionDenied => tr!("open-denied", path = path),
        _ => tr!("open-failed", path = path, error = e.to_string()),
    }
}

/// Where `emergency` leaves unsaved text: `$XDG_STATE_HOME/reprise/notepad`.
pub(crate) fn recovery_dir() -> Option<PathBuf> {
    reprisefs::recovery::dir("notepad")
}

/// Write `lines` to `recovery-<pid>-<stem>.txt` in `dir`, made 0700 if missing, in `format`
/// unless that is CP1252: then in UTF-8, which holds whatever was typed.
pub(crate) fn write_recovery(
    dir: &Path,
    pid: u32,
    stem: &str,
    lines: &[impl AsRef<str>],
    format: Format,
) -> std::io::Result<PathBuf> {
    let format = match format.encoding {
        Encoding::Cp1252 => format.utf8(),
        _ => format,
    };
    let bytes = encode(lines, format).ok_or_else(|| unheld(format))?;
    reprisefs::recovery::write(dir, pid, stem, "txt", &bytes)
}

/// The recovery files in `dir`, by name.
pub(crate) fn recovered_in(dir: &Path) -> Vec<PathBuf> {
    reprisefs::recovery::found(dir, "txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(bytes: &[u8]) -> Result<(String, Format), Undecodable> {
        decode(bytes, None)
    }

    #[test]
    fn decode_reads_each_bom_and_leaves_it_out_of_the_text() {
        let utf8 = Format {
            bom: true,
            ..Format::default()
        };
        assert_eq!(auto(b"\xEF\xBB\xBFhi").unwrap(), ("hi".into(), utf8));
        let le = Format {
            encoding: Encoding::Utf16Le,
            bom: true,
            line_end: LineEnd::Crlf,
        };
        assert_eq!(auto(b"\xFF\xFEh\0\r\0\n\0").unwrap(), ("h\r\n".into(), le));
        let be = Format {
            encoding: Encoding::Utf16Be,
            ..le
        };
        assert_eq!(auto(b"\xFE\xFF\0h\0\r\0\n").unwrap(), ("h\r\n".into(), be));
        assert_eq!(auto(b"a\rb").unwrap().1.line_end, LineEnd::Cr);
    }

    #[test]
    fn decode_refuses_a_lone_surrogate_and_an_odd_byte() {
        let lone = auto(b"\xFF\xFEa\0\x00\xD8").unwrap_err();
        assert_eq!(
            (lone.encoding, lone.lossy.as_str()),
            (Encoding::Utf16Le, "a\u{fffd}")
        );
        let odd = auto(b"\xFE\xFF\0a\0").unwrap_err();
        assert_eq!(odd.lossy, "a\u{fffd}");
        let bad = auto(b"\xEF\xBB\xBFcaf\xe9").unwrap_err();
        assert_eq!(
            (bad.encoding, bad.lossy.as_str()),
            (Encoding::Utf8, "caf\u{fffd}")
        );
    }

    #[test]
    fn without_a_bom_valid_utf8_reads_as_utf8_and_anything_else_as_cp1252() {
        let (text, format) = auto("caf\u{e9}".as_bytes()).unwrap();
        assert_eq!((text.as_str(), format), ("caf\u{e9}", Format::default()));
        let (text, format) = auto(b"caf\xe9 \x80\x93\r\n").unwrap();
        assert_eq!(text, "caf\u{e9} \u{20AC}\u{201C}\r\n");
        let cp1252 = Format {
            encoding: Encoding::Cp1252,
            bom: false,
            line_end: LineEnd::Crlf,
        };
        assert_eq!(format, cp1252);
    }

    #[test]
    fn decode_reads_the_encoding_picked_over_the_one_detected() {
        let (text, format) = decode("caf\u{e9}".as_bytes(), Some(Encoding::Cp1252)).unwrap();
        assert_eq!(
            (text.as_str(), format.encoding),
            ("caf\u{c3}\u{a9}", Encoding::Cp1252)
        );
        let (text, format) = decode(b"h\0i\0", Some(Encoding::Utf16Le)).unwrap();
        assert_eq!(
            (text.as_str(), format.bom),
            ("hi", true),
            "UTF-16 saves a BOM"
        );
        let (text, format) = decode(b"\xEF\xBB\xBFhi", Some(Encoding::Utf8)).unwrap();
        assert_eq!((text.as_str(), format.bom), ("hi", true));
        let bad = decode(b"caf\xe9", Some(Encoding::Utf8)).unwrap_err();
        assert_eq!(bad.lossy, "caf\u{fffd}");
    }

    #[test]
    fn every_byte_reads_as_cp1252_and_writes_back_as_itself() {
        let bytes: Vec<u8> = (0..=255).collect();
        let (text, format) = decode(&bytes, Some(Encoding::Cp1252)).unwrap();
        assert_eq!(text.chars().count(), 256);
        let lf = Format {
            line_end: LineEnd::Lf,
            ..format
        };
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(encode(&lines, lf).as_deref(), Some(bytes.as_slice()));
    }

    #[test]
    fn cp1252_refuses_a_character_it_cannot_hold() {
        let cp1252 = Format::default().with_encoding_row(4);
        assert_eq!(cp1252.encoding, Encoding::Cp1252);
        assert!(cp1252.holds(&["caf\u{e9} \u{20AC}"]));
        assert_eq!(encode(&["\u{20AC}"], cp1252), Some(vec![0x80]));
        for c in ["\u{151}", "\u{80}", "\u{1F600}"] {
            assert!(!cp1252.holds(&["a", c]), "{c:?}");
            assert_eq!(encode(&["a", c], cp1252), None, "{c:?}");
        }
        assert!(Format::default().holds(&["\u{151}"]));
    }

    #[test]
    fn encode_writes_the_bom_and_each_line_end() {
        let lines = ["a", "b"];
        let cr = Format {
            line_end: LineEnd::Cr,
            ..Format::default()
        };
        let bytes = |f: Format| encode(&lines, f).expect("holds");
        assert_eq!(bytes(cr), b"a\rb");
        assert_eq!(bytes(cr.with_encoding_row(1)), b"\xEF\xBB\xBFa\rb");
        assert_eq!(bytes(cr.with_encoding_row(2)), b"\xFF\xFEa\0\r\0b\0");
        let crlf = Format {
            line_end: LineEnd::Crlf,
            ..Format::default()
        };
        assert_eq!(bytes(crlf.with_encoding_row(3)), b"\xFE\xFF\0a\0\r\0\n\0b");
        assert_eq!(bytes(crlf.with_encoding_row(4)), b"a\r\nb");
        for row in 0..ENCODINGS.len() {
            assert_eq!(Format::default().with_encoding_row(row).encoding_row(), row);
        }
    }

    #[test]
    fn a_cp1252_document_is_kept_for_recovery_in_utf8() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cp1252 = Format::default().with_encoding_row(4);
        let kept = write_recovery(tmp.path(), 1, "a", &["\u{151}"], cp1252).expect("write");
        assert_eq!(std::fs::read(kept).expect("read"), "\u{151}".as_bytes());
    }
}
