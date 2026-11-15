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
pub(crate) enum Encoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
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

/// The Save As encoding rows, in the combo's order.
pub(crate) const ENCODINGS: [(&str, Encoding, bool); 4] = [
    ("UTF-8", Encoding::Utf8, false),
    ("UTF-8 with BOM", Encoding::Utf8, true),
    ("UTF-16 LE", Encoding::Utf16Le, true),
    ("UTF-16 BE", Encoding::Utf16Be, true),
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
}

const BOM_UTF8: &[u8] = b"\xEF\xBB\xBF";
const BOM_UTF16LE: &[u8] = b"\xFF\xFE";
const BOM_UTF16BE: &[u8] = b"\xFE\xFF";

/// A file whose bytes do not decode in the encoding its BOM, or the lack of one, names.
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
    }
}

/// A file's text and format: a UTF-8 or UTF-16 BOM picks the encoding and is not part of the
/// text; without one the file is UTF-8.
pub(crate) fn decode(bytes: Vec<u8>) -> Result<(String, Format), Undecodable> {
    let (encoding, bom) = if bytes.starts_with(BOM_UTF8) {
        (Encoding::Utf8, BOM_UTF8.len())
    } else if bytes.starts_with(BOM_UTF16LE) {
        (Encoding::Utf16Le, BOM_UTF16LE.len())
    } else if bytes.starts_with(BOM_UTF16BE) {
        (Encoding::Utf16Be, BOM_UTF16BE.len())
    } else {
        (Encoding::Utf8, 0)
    };
    let body = &bytes[bom..];
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
    }
    .map_err(|lossy| Undecodable { encoding, lossy })?;
    let format = Format {
        encoding,
        bom: bom > 0,
        line_end: LineEnd::of(&text),
    };
    Ok((text, format))
}

/// `lines` joined by `format`'s line end, in its encoding, after its BOM.
pub(crate) fn encode(lines: &[impl AsRef<str>], format: Format) -> Vec<u8> {
    let mut out = Vec::new();
    if format.bom {
        out.extend_from_slice(match format.encoding {
            Encoding::Utf8 => BOM_UTF8,
            Encoding::Utf16Le => BOM_UTF16LE,
            Encoding::Utf16Be => BOM_UTF16BE,
        });
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
            }
        }
    }
    out
}

/// Largest file Notepad opens: the whole text lives in one edit buffer.
pub(crate) const MAX_OPEN_BYTES: u64 = 32 << 20;

/// Only a regular file up to `MAX_OPEN_BYTES` that `decode` reads, which an [`Undecodable`] inside
/// the error says otherwise; a FIFO or device fails without blocking.
pub(crate) fn read_text(path: &Path) -> std::io::Result<(String, Format)> {
    let bytes = reprisefs::ops::read_regular(path, MAX_OPEN_BYTES)?;
    decode(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
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
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("reprise").join("notepad"))
}

/// Write `lines` to `recovery-<pid>-<stem>.txt` in `dir`, made 0700 if missing.
pub(crate) fn write_recovery(
    dir: &Path,
    pid: u32,
    stem: &str,
    lines: &[impl AsRef<str>],
    format: Format,
) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join(format!("{RECOVERY_PREFIX}{pid}-{stem}.txt"));
    reprisefs::ops::write_document(&path, &encode(lines, format))?;
    Ok(path)
}

const RECOVERY_PREFIX: &str = "recovery-";

/// The recovery files in `dir`, by name.
pub(crate) fn recovered_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(RECOVERY_PREFIX) && n.ends_with(".txt"))
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_reads_each_bom_and_leaves_it_out_of_the_text() {
        let utf8 = Format {
            bom: true,
            ..Format::default()
        };
        assert_eq!(
            decode(b"\xEF\xBB\xBFhi".to_vec()).unwrap(),
            ("hi".into(), utf8)
        );
        let le = Format {
            encoding: Encoding::Utf16Le,
            bom: true,
            line_end: LineEnd::Crlf,
        };
        assert_eq!(
            decode(b"\xFF\xFEh\0\r\0\n\0".to_vec()).unwrap(),
            ("h\r\n".into(), le)
        );
        let be = Format {
            encoding: Encoding::Utf16Be,
            ..le
        };
        assert_eq!(
            decode(b"\xFE\xFF\0h\0\r\0\n".to_vec()).unwrap(),
            ("h\r\n".into(), be)
        );
        assert_eq!(decode(b"a\rb".to_vec()).unwrap().1.line_end, LineEnd::Cr);
    }

    #[test]
    fn decode_refuses_a_lone_surrogate_and_an_odd_byte() {
        let lone = decode(b"\xFF\xFEa\0\x00\xD8".to_vec()).unwrap_err();
        assert_eq!(
            (lone.encoding, lone.lossy.as_str()),
            (Encoding::Utf16Le, "a\u{fffd}")
        );
        let odd = decode(b"\xFE\xFF\0a\0".to_vec()).unwrap_err();
        assert_eq!(odd.lossy, "a\u{fffd}");
        let bad = decode(b"\xEF\xBB\xBFcaf\xe9".to_vec()).unwrap_err();
        assert_eq!(
            (bad.encoding, bad.lossy.as_str()),
            (Encoding::Utf8, "caf\u{fffd}")
        );
    }

    #[test]
    fn encode_writes_the_bom_and_each_line_end() {
        let lines = ["a", "b"];
        let cr = Format {
            line_end: LineEnd::Cr,
            ..Format::default()
        };
        assert_eq!(encode(&lines, cr), b"a\rb");
        assert_eq!(encode(&lines, cr.with_encoding_row(1)), b"\xEF\xBB\xBFa\rb");
        assert_eq!(
            encode(&lines, cr.with_encoding_row(2)),
            b"\xFF\xFEa\0\r\0b\0"
        );
        let crlf = Format {
            line_end: LineEnd::Crlf,
            ..Format::default()
        };
        assert_eq!(
            encode(&lines, crlf.with_encoding_row(3)),
            b"\xFE\xFF\0a\0\r\0\n\0b"
        );
        for row in 0..ENCODINGS.len() {
            assert_eq!(Format::default().with_encoding_row(row).encoding_row(), row);
        }
    }
}
