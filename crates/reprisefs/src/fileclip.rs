//! Files on the clipboard, offered and read by both the compositor and programs. GNOME's form says
//! cut or copy in its first line; KDE offers a URI list and says it in [`KDE_CUT`] beside it.

use std::path::PathBuf;

use crate::dropdata::{self, Dropped, GNOME_FILES, URI_LIST};

/// `1` when the URI list beside it was cut, `0` when copied.
pub const KDE_CUT: &str = "application/x-kde-cutselection";
pub const PATH_TEXT: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileClip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// Each type a clip of files is offered as, with its bytes; None with no paths.
pub fn offers(clip: &FileClip) -> Option<Vec<(&'static str, Vec<u8>)>> {
    if clip.paths.is_empty() {
        return None;
    }
    let gnome = dropdata::encode_files(&clip.paths, GNOME_FILES)?;
    let uris = gnome.strip_prefix(b"copy").unwrap_or(&gnome);
    let head: &[u8] = if clip.cut { b"cut" } else { b"copy" };
    let marker: &[u8] = if clip.cut { b"1" } else { b"0" };
    let mut out = vec![
        (GNOME_FILES, [head, uris].concat()),
        (URI_LIST, dropdata::encode_files(&clip.paths, URI_LIST)?),
        (KDE_CUT, marker.to_vec()),
    ];
    if let Some(text) = dropdata::encode_files(&clip.paths, dropdata::TEXT_UTF8) {
        out.extend(PATH_TEXT.map(|m| (m, text.clone())));
    }
    Some(out)
}

/// What a paste reads off a selection: the type holding the files, and whether
/// [`KDE_CUT`] must be read too to know a cut from a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub files: &'static str,
    pub kde_cut: bool,
}

/// None when the selection offering `offered` holds no files.
pub fn source<S: AsRef<str>>(offered: &[S]) -> Option<Source> {
    let has = |mime| offered.iter().any(|m| m.as_ref() == mime);
    if has(GNOME_FILES) {
        Some(Source {
            files: GNOME_FILES,
            kde_cut: false,
        })
    } else {
        has(URI_LIST).then(|| Source {
            files: URI_LIST,
            kde_cut: has(KDE_CUT),
        })
    }
}

/// The local files in `bytes` read as `mime`; web links and other hosts' files are skipped.
pub fn decode(mime: &str, bytes: &[u8]) -> FileClip {
    let cut = mime == GNOME_FILES && bytes.split(|&b| b == b'\n').next() == Some(b"cut");
    let paths = dropdata::decode(mime, bytes)
        .into_iter()
        .filter_map(|d| match d {
            Dropped::File(p) => Some(p),
            Dropped::Url { .. } => None,
        })
        .collect();
    FileClip { paths, cut }
}

pub fn is_cut_marker(bytes: &[u8]) -> bool {
    bytes.trim_ascii() == b"1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes<'a>(offers: &'a [(&str, Vec<u8>)], mime: &str) -> &'a [u8] {
        &offers.iter().find(|(m, _)| *m == mime).unwrap().1
    }

    #[test]
    fn a_clip_is_offered_in_every_form_and_reads_back_as_it_was() {
        let clip = FileClip {
            paths: vec!["/d/a b.txt".into(), "/d/c".into()],
            cut: true,
        };
        let offers = offers(&clip).expect("two paths");
        let mimes: Vec<&str> = offers.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            mimes,
            [GNOME_FILES, URI_LIST, KDE_CUT, PATH_TEXT[0], PATH_TEXT[1]]
        );
        assert_eq!(
            bytes(&offers, GNOME_FILES),
            b"cut\nfile:///d/a%20b.txt\nfile:///d/c"
        );
        assert_eq!(
            bytes(&offers, URI_LIST),
            b"file:///d/a%20b.txt\r\nfile:///d/c\r\n"
        );
        assert_eq!(bytes(&offers, KDE_CUT), b"1");
        assert_eq!(bytes(&offers, "text/plain"), b"/d/a b.txt\n/d/c");
        assert_eq!(decode(GNOME_FILES, bytes(&offers, GNOME_FILES)), clip);

        let copy = FileClip { cut: false, ..clip };
        let offers = super::offers(&copy).unwrap();
        assert_eq!(bytes(&offers, KDE_CUT), b"0");
        assert_eq!(decode(GNOME_FILES, bytes(&offers, GNOME_FILES)), copy);
        assert_eq!(super::offers(&FileClip::default()), None);
    }

    #[test]
    fn a_paste_reads_gnomes_form_else_the_uri_list_and_kdes_marker() {
        let gnome = Source {
            files: GNOME_FILES,
            kde_cut: false,
        };
        assert_eq!(source(&[URI_LIST, KDE_CUT, GNOME_FILES]), Some(gnome));
        assert_eq!(
            source(&[URI_LIST, "application/x-kde4-urilist", KDE_CUT]),
            Some(Source {
                files: URI_LIST,
                kde_cut: true
            })
        );
        assert_eq!(
            source(&[URI_LIST, "text/plain"]),
            Some(Source {
                files: URI_LIST,
                kde_cut: false
            })
        );
        assert_eq!(source(&["text/plain", KDE_CUT]), None);
    }

    #[test]
    fn a_uri_list_from_another_program_reads_as_a_copy_of_its_local_files() {
        let list = "# from wl-copy\r\nfile:///tmp/b%20c.txt\r\n\
            file:///tmp/%C3%A1rv%C3%ADzt%C5%B1r%C5%91.txt\nfile://LOCALHOST/tmp/e\r\n\
            file://elsewhere/tmp/f\r\nhttps://example.org/\r\n";
        assert_eq!(
            decode(URI_LIST, list.as_bytes()),
            FileClip {
                paths: vec![
                    "/tmp/b c.txt".into(),
                    "/tmp/árvíztűrő.txt".into(),
                    "/tmp/e".into()
                ],
                cut: false
            }
        );
        assert!(decode(GNOME_FILES, b"cut\nfile:///x").cut);
        assert!(!decode(GNOME_FILES, b"copy\nfile:///x").cut);
    }

    #[test]
    fn only_kdes_one_marks_a_cut() {
        assert!(is_cut_marker(b"1"));
        assert!(!is_cut_marker(b"0"));
        assert!(!is_cut_marker(b""));
    }
}
