//! What a drag from another program carries: the mime type to ask for, the files and web links
//! decoded from its bytes, and the `Type=Link` file a dropped link becomes. Plain text is refused.
//! A document portal key ([`FILE_TRANSFER`]) is read and traded by `reprisebus::filetransfer`, so [`decode`] has no files for it.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

/// A browser's link: `url\ntitle` pairs, in UTF-16 from Firefox and
/// Chromium.
pub const MOZ_URL: &str = "text/x-moz-url";
/// One URI per line (`\r\n`-terminated), `#` lines are comments.
pub const URI_LIST: &str = "text/uri-list";
pub const FILE_TRANSFER: &str = "application/vnd.portal.filetransfer";
/// Offered, never sent, by a drag whose URI list names the files
/// themselves: Reprise's own drags, and every drag the compositor sees a
/// program outside a sandbox start (it takes the mime off a sandboxed
/// program's drag).
pub const HOST_FILES: &str = "application/x-reprise-host-files";

/// The form most file managers use: `copy` or `cut` on its own
/// line, then the `file://` URIs joined with `\n`.
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
pub const TEXT_UTF8: &str = "text/plain;charset=utf-8";
pub const FILE_MIMES: [&str; 3] = [URI_LIST, GNOME_FILES, TEXT_UTF8];

/// The schemes a dropped link may have: the rest (`javascript:`,
/// `about:`, a program's own scheme) makes no link file.
const LINK_SCHEMES: [&str; 3] = ["http", "https", "ftp"];

pub const LINK_NAME_MAX: usize = 200;

pub fn is_web_url(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(scheme, _)| LINK_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dropped {
    File(PathBuf),
    Url { url: String, title: Option<String> },
}

/// Whether a drag offering `offered` has its files read through the
/// portal key: it offers one, and is not marked [`HOST_FILES`]. A
/// sandboxed program's URI list names paths inside its sandbox.
pub fn through_portal<S: AsRef<str>>(offered: &[S]) -> bool {
    let has = |mime| offered.iter().any(|m| m.as_ref() == mime);
    has(FILE_TRANSFER) && !has(HOST_FILES)
}

pub const HTML: &str = "text/html";

/// The mime type to read a drag offering `offered` as, best first: a
/// browser's link (it names the page), then files through the portal
/// ([`through_portal`]), then a URI list. None when the drag carries none,
/// which refuses it.
///
/// it goes by the types alone. Qt on X11 offers [`MOZ_URL`] beside every
/// URI list, a file manager's file drag included, so [`MOZ_URL`] makes a
/// link only when the drag offers no URI list or also carries [`HTML`],
/// as every browser's link drag does.
pub fn pick_mime<S: AsRef<str>>(offered: &[S]) -> Option<&'static str> {
    let has = |mime| offered.iter().any(|m| m.as_ref() == mime);
    if has(MOZ_URL) && (!has(URI_LIST) || has(HTML)) {
        Some(MOZ_URL)
    } else if through_portal(offered) {
        Some(FILE_TRANSFER)
    } else {
        has(URI_LIST).then_some(URI_LIST)
    }
}

pub fn is_link_mime(mime: &str) -> bool {
    mime == MOZ_URL
}

pub fn decode(mime: &str, bytes: &[u8]) -> Vec<Dropped> {
    if mime == MOZ_URL {
        let text = utf16_or_utf8(bytes);
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        lines
            .chunks(2)
            .filter_map(|pair| {
                let title = pair
                    .get(1)
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string);
                entry(pair[0].trim(), title)
            })
            .collect()
    } else {
        String::from_utf8_lossy(bytes)
            .split('\n')
            .map(|l| l.trim_end_matches('\r').trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| entry(l, None))
            .collect()
    }
}

/// The bytes a drag of `paths` sends as `mime`, None for a mime it does not
/// offer ([`FILE_MIMES`]). The `copy`/`cut` form says `copy`: a target that moves
/// the files does the move itself. The text form leaves out a path that is
/// not UTF-8 or holds a line break, which one path per line cannot carry,
/// and is not offered when no path is left.
pub fn encode_files(paths: &[PathBuf], mime: &str) -> Option<Vec<u8>> {
    let uris = || {
        paths
            .iter()
            .map(|p| format!("file://{}", crate::trash::percent_encode(p)))
    };
    match mime {
        URI_LIST => Some(uris().map(|u| u + "\r\n").collect::<String>().into_bytes()),
        GNOME_FILES => Some(
            std::iter::once("copy".to_string())
                .chain(uris())
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes(),
        ),
        TEXT_UTF8 => {
            let text: Vec<&str> = paths
                .iter()
                .filter_map(|p| p.to_str())
                .filter(|p| !p.contains('\n'))
                .collect();
            (!text.is_empty()).then(|| text.join("\n").into_bytes())
        }
        _ => None,
    }
}

fn entry(uri: &str, title: Option<String>) -> Option<Dropped> {
    if let Some(rest) = uri.strip_prefix("file://") {
        // `file:///p` or `file://localhost/p`; another host is not here.
        let path = match rest.split_once('/') {
            Some(("" | "localhost", p)) => format!("/{p}"),
            _ => return None,
        };
        let bytes = crate::trash::percent_decode(&path);
        return Some(Dropped::File(PathBuf::from(OsStr::from_bytes(&bytes))));
    }
    if !is_web_url(uri) {
        return None;
    }
    Some(Dropped::Url {
        url: uri.to_string(),
        title,
    })
}

/// `bytes` as text: UTF-16LE when it starts with its byte order mark or
/// every second byte of the first pair is zero (ASCII in UTF-16), else
/// UTF-8. A trailing NUL goes.
fn utf16_or_utf8(bytes: &[u8]) -> String {
    let bom = bytes.starts_with(&[0xFF, 0xFE]);
    let text = if bom || (bytes.len() >= 2 && bytes[1] == 0 && bytes[0] != 0) {
        let body = if bom { &bytes[2..] } else { bytes };
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_end_matches('\0').to_string()
}

/// Cut to [`LINK_NAME_MAX`] bytes on a character boundary so " (N).desktop" fits the 255-byte file name limit.
pub fn link_name(url: &str, title: Option<&str>) -> String {
    let host = || {
        let rest = url.split_once("://")?.1;
        let host = rest.split(['/', '?', '#']).next()?;
        let host = host.rsplit('@').next()?;
        Some(host.to_string()).filter(|h| !h.is_empty())
    };
    let raw = title
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
        .or_else(host)
        .unwrap_or_else(|| "Link".into());
    let mut clean = String::new();
    for c in raw.chars().filter(|c| !c.is_control() && *c != '/') {
        if clean.len() + c.len_utf8() > LINK_NAME_MAX {
            break;
        }
        clean.push(c);
    }
    let clean = clean.trim().trim_start_matches('.').trim().to_string();
    if clean.is_empty() {
        "Link".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(p: &str) -> Dropped {
        Dropped::File(PathBuf::from(p))
    }

    #[test]
    fn a_drag_of_files_encodes_each_form_and_decodes_back() {
        let paths = [PathBuf::from("/d/my notes.txt"), PathBuf::from("/d/a%b")];
        let list = encode_files(&paths, URI_LIST).unwrap();
        assert_eq!(list, b"file:///d/my%20notes.txt\r\nfile:///d/a%25b\r\n");
        assert_eq!(
            decode(URI_LIST, &list),
            [file("/d/my notes.txt"), file("/d/a%b")]
        );
        assert_eq!(
            encode_files(&paths, GNOME_FILES).unwrap(),
            b"copy\nfile:///d/my%20notes.txt\nfile:///d/a%25b"
        );
        assert_eq!(
            encode_files(&paths, TEXT_UTF8).unwrap(),
            b"/d/my notes.txt\n/d/a%b"
        );
        let odd = [PathBuf::from("/d/two\nlines"), PathBuf::from("/d/ok")];
        assert_eq!(encode_files(&odd, TEXT_UTF8).unwrap(), b"/d/ok");
        assert_eq!(encode_files(&odd[..1], TEXT_UTF8), None);
        assert_eq!(encode_files(&paths, MOZ_URL), None);
    }

    #[test]
    fn a_link_is_read_before_a_uri_list_and_plain_text_is_refused() {
        assert_eq!(
            pick_mime(&["text/plain", URI_LIST, MOZ_URL, HTML]),
            Some(MOZ_URL)
        );
        assert_eq!(pick_mime(&["text/plain", MOZ_URL]), Some(MOZ_URL));
        assert_eq!(pick_mime(&["text/plain", URI_LIST]), Some(URI_LIST));
        assert_eq!(
            pick_mime(&["text/plain;charset=utf-8", "UTF8_STRING"]),
            None
        );
        assert!(is_link_mime(MOZ_URL) && !is_link_mime(URI_LIST));
    }

    #[test]
    fn a_uri_list_with_a_moz_url_but_no_markup_is_a_file_drag() {
        assert_eq!(
            pick_mime(&[URI_LIST, MOZ_URL, "text/plain"]),
            Some(URI_LIST)
        );
        assert_eq!(
            pick_mime(&[MOZ_URL, "text/x-moz-url-data", URI_LIST, HTML, "text/plain"]),
            Some(MOZ_URL)
        );
    }

    #[test]
    fn a_sandboxed_programs_files_are_read_through_the_portal() {
        assert_eq!(
            pick_mime(&[URI_LIST, FILE_TRANSFER, "text/plain"]),
            Some(FILE_TRANSFER)
        );
        assert_eq!(pick_mime(&[FILE_TRANSFER, MOZ_URL]), Some(MOZ_URL));
        assert_eq!(
            pick_mime(&[FILE_TRANSFER, URI_LIST, HOST_FILES]),
            Some(URI_LIST)
        );
        assert!(through_portal(&[FILE_TRANSFER]));
        assert!(!through_portal(&[FILE_TRANSFER, HOST_FILES]));
        assert!(!through_portal(&[URI_LIST, HOST_FILES]));
        assert!(!is_link_mime(FILE_TRANSFER));
        assert!(decode(FILE_TRANSFER, b"1234\0").is_empty());
    }

    #[test]
    fn a_uri_list_decodes_escapes_hosts_comments_and_crlf() {
        let list = b"# from a file manager\r\nfile:///home/u/a%20b.txt\r\n\
            file://localhost/tmp/x\r\nfile://elsewhere/tmp/y\r\n\r\n\
            https://example.org/page\r\nmailto:a@b\r\n";
        assert_eq!(
            decode(URI_LIST, list),
            [
                file("/home/u/a b.txt"),
                file("/tmp/x"),
                Dropped::Url {
                    url: "https://example.org/page".into(),
                    title: None
                },
            ]
        );
        assert_eq!(
            decode(URI_LIST, b"file:///t/%FF.txt\n"),
            [Dropped::File(PathBuf::from(OsStr::from_bytes(
                b"/t/\xFF.txt"
            )))]
        );
    }

    #[test]
    fn a_browser_link_decodes_from_utf16_with_its_title() {
        let text = "https://example.org/a\nExample Domain";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let want = [Dropped::Url {
            url: "https://example.org/a".into(),
            title: Some("Example Domain".into()),
        }];
        assert_eq!(decode(MOZ_URL, &bytes), want);
        let bare: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(MOZ_URL, &bare), want);
        assert_eq!(decode(MOZ_URL, text.as_bytes()), want);
        assert_eq!(
            decode(MOZ_URL, b"file:///tmp/a\na\nhttp://x.test/\n\n"),
            [
                file("/tmp/a"),
                Dropped::Url {
                    url: "http://x.test/".into(),
                    title: None
                }
            ]
        );
    }

    #[test]
    fn a_script_or_a_broken_link_is_no_drop() {
        assert!(decode(URI_LIST, b"javascript:alert(1)\n").is_empty());
        assert!(decode(URI_LIST, b"about:blank\n").is_empty());
        assert!(decode(MOZ_URL, b"https://a.test/\x07x\nT").is_empty());
    }

    #[test]
    fn a_link_is_named_after_its_title_else_its_host() {
        assert_eq!(
            link_name("https://example.org/a", Some("Example Domain")),
            "Example Domain"
        );
        assert_eq!(
            link_name("https://user@www.example.org:8080/a?q#f", None),
            "www.example.org:8080"
        );
        assert_eq!(link_name("https://example.org", Some("  ")), "example.org");
        assert_eq!(link_name("https://x.test/", Some("a/b\nc")), "abc");
        assert_eq!(link_name("https://x.test/", Some("..")), "Link");
        assert_eq!(link_name("https:///", None), "Link");
        let long = link_name("https://x.test/", Some(&"é".repeat(300)));
        assert_eq!((long.len(), long.chars().count()), (200, 100));
        let cjk = link_name("https://x.test/", Some(&"漢".repeat(100)));
        assert_eq!((cjk.len(), cjk.chars().count()), (198, 66));
        assert!(format!("{cjk} (99).desktop").len() <= 255);
    }

    #[test]
    fn only_a_web_url_makes_a_link() {
        assert!(is_web_url("https://a.test/x?y#z"));
        assert!(is_web_url("FTP://a.test/"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://a.test/\nX=y",
            "nope",
        ] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }
}
