//! Files a paired device sends over Bluetooth: the name one is saved under in Downloads, the
//! Accept / Decline prompt's text and the balloons after it.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use reprisefs::names;

use crate::bz::Offer;

pub const TITLE: &str = "Bluetooth File Transfer";
pub const ACCEPT: usize = 0;
pub const BUTTONS: [&str; 2] = ["&Accept", "&Decline"];
/// The received balloon's buttons, in [`reprisetray::Msg::BalloonButton`] order.
pub const OPEN: usize = 0;
pub const BALLOON_BUTTONS: [&str; 2] = ["Open", "Show in folder"];

/// Longest saved name in bytes, under the usual 255-byte limit with room for " (99)".
const NAME_MAX: usize = 200;
/// Longest offered name the prompt and balloons show in full, in characters.
const SHOWN_MAX: usize = 60;

/// `offered` as one file name: separators and control characters become `_`, leading dots go,
/// and a name over [`NAME_MAX`] bytes keeps a short extension.
pub fn safe_name(offered: &str) -> String {
    let clean: String = offered
        .chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let clean = clean.trim_start_matches('.').trim();
    if clean.is_empty() {
        return "file".into();
    }
    if clean.len() <= NAME_MAX {
        return clean.to_string();
    }
    let ext = clean
        .rfind('.')
        .map(|i| &clean[i..])
        .filter(|e| e.len() <= 16)
        .unwrap_or("");
    let mut cut = NAME_MAX - ext.len();
    while !clean.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", &clean[..cut])
}

/// The user's Downloads folder (`XDG_DOWNLOAD_DIR`).
pub fn downloads() -> Option<PathBuf> {
    reprisefs::places::user_dir("DOWNLOAD", "Downloads")
}

/// The name obexd writes `transfer` (its object path) under in its own folder until it is
/// complete: hidden, and unique while obexd runs.
pub fn incoming_name(transfer: &str) -> String {
    let mut parts = transfer.rsplit('/');
    let (t, s) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    format!(".reprise-{}", safe_name(&format!("{s}-{t}")))
}

/// Moves the received file `from` into `dir` (made if missing) as `offered` cleaned, numbered
/// `name (2)`... when taken. The move never replaces anything at a name, a symlink included.
pub fn land(from: &Path, dir: &Path, offered: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = safe_name(offered);
    for candidate in names::numbered(OsStr::new(&name), names::paren).take(1000) {
        let to = dir.join(candidate);
        match reprisefs::trash::move_to_free(from, &to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

fn shown(name: &str) -> String {
    if name.chars().count() <= SHOWN_MAX {
        return name.to_string();
    }
    let cut: String = name.chars().take(SHOWN_MAX - 3).collect();
    format!("{cut}...")
}

/// The prompt's text: who sends what, its size when the device said, and where it goes.
pub fn prompt(o: &Offer) -> String {
    let mut lines = vec![
        format!("{} wants to send you a file.", o.sender),
        String::new(),
        format!("File: {}", shown(&safe_name(&o.name))),
    ];
    if let Some(size) = o.size {
        lines.push(format!("Size: {}", reprisefs::replace::size_text(size)));
    }
    lines.push(String::new());
    lines.push("Accepted files are saved in your Downloads folder.".into());
    lines.join("\n")
}

/// The balloon for a file saved at `path`, with [`BALLOON_BUTTONS`].
pub fn received(path: &Path, sender: &str) -> (String, String) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    (
        "File received".into(),
        format!("{} from {sender} is in Downloads.", shown(&name)),
    )
}

pub fn not_received(name: &str, sender: &str) -> (String, String) {
    (
        "File not received".into(),
        format!(
            "{} from {sender} was not received.",
            shown(&safe_name(name))
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_keeps_the_file_in_one_folder() {
        assert_eq!(safe_name("holiday.jpg"), "holiday.jpg");
        assert_eq!(safe_name("../../.bashrc"), "_.._.bashrc");
        assert_eq!(safe_name("/etc/passwd"), "_etc_passwd");
        assert_eq!(safe_name("..\\win.ini"), "_win.ini");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name("a\nb\0c"), "a_b_c");
        for empty in ["", ".", "..", "...", "   "] {
            assert_eq!(safe_name(empty), "file", "{empty:?}");
        }
    }

    #[test]
    fn a_long_name_is_cut_keeping_its_extension() {
        let long = format!("{}.jpeg", "é".repeat(150));
        let got = safe_name(&long);
        assert!(got.len() <= NAME_MAX, "{}", got.len());
        assert!(got.ends_with("é.jpeg"), "{got}");
    }

    #[test]
    fn land_numbers_a_taken_name_and_never_follows_a_link() {
        let dir = std::env::temp_dir().join(format!("reprise-bt-land-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let incoming = dir.join("in");
        std::fs::create_dir_all(&incoming).unwrap();
        let got = |name: &str| {
            let from = incoming.join(".reprise-session0-transfer0");
            std::fs::write(&from, name).unwrap();
            land(&from, &dir.join("Downloads"), name).unwrap()
        };
        let down = dir.join("Downloads");
        assert_eq!(got("photo.jpg"), down.join("photo.jpg"));
        assert_eq!(
            std::fs::read_to_string(down.join("photo.jpg")).unwrap(),
            "photo.jpg"
        );
        assert_eq!(got("photo.jpg"), down.join("photo (2).jpg"));
        let outside = dir.join("outside");
        std::os::unix::fs::symlink(&outside, down.join("link.txt")).unwrap();
        assert_eq!(got("link.txt"), down.join("link (2).txt"));
        assert!(!outside.exists(), "the dangling link was not followed");
        assert_eq!(got("../up.txt"), down.join("_up.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_incoming_name_is_hidden_and_names_the_transfer() {
        assert_eq!(
            incoming_name("/org/bluez/obex/server/session3/transfer7"),
            ".reprise-session3-transfer7"
        );
    }

    #[test]
    fn the_prompt_names_the_sender_file_and_size() {
        let o = Offer {
            transfer: "/t".into(),
            sender: "Pixel (AA:BB:CC:DD:EE:01)".into(),
            name: "holiday.jpg".into(),
            size: Some(2_500_000),
            done: None,
        };
        assert_eq!(
            prompt(&o),
            "Pixel (AA:BB:CC:DD:EE:01) wants to send you a file.\n\nFile: holiday.jpg\n\
             Size: 2.38 MB\n\nAccepted files are saved in your Downloads folder."
        );
        let no_size = Offer { size: None, ..o };
        assert!(!prompt(&no_size).contains("Size"));
    }

    #[test]
    fn balloons_name_the_file_and_sender() {
        assert_eq!(
            received(Path::new("/home/u/Downloads/a (2).jpg"), "Pixel"),
            (
                "File received".to_string(),
                "a (2).jpg from Pixel is in Downloads.".to_string()
            )
        );
        let (_, body) = not_received(&"x".repeat(100), "Pixel");
        assert!(
            body.starts_with(&format!("{}...", "x".repeat(57))),
            "{body}"
        );
    }
}
