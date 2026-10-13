use crate::listing::Item;
use std::path::{Path, PathBuf};

pub fn icon_for_file(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "tif" | "tiff" => {
            "image-x-generic"
        }
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" => "audio-x-generic",
        "mp4" | "mkv" | "avi" | "webm" | "mov" => "video-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "xz" | "zst" | "7z" | "rar" => "package-x-generic",
        "html" | "htm" => "text-html",
        _ => "text-x-generic",
    }
}

/// Theme icon names for a MIME type, best first: the freedesktop name, the legacy `gnome-mime-`
/// spelling older themes carry, then the media type's generic icon. `None` gives the generic
/// document.
pub fn icons_for_mime(mime: Option<&str>) -> Vec<String> {
    let Some(mime) = mime else {
        return vec!["text-x-generic".into()];
    };
    let flat = mime.replace('/', "-");
    let media = mime.split('/').next().unwrap_or("text");
    let generic = match media {
        "image" | "audio" | "video" | "font" => format!("{media}-x-generic"),
        "application"
            if mime.contains("zip") || mime.contains("tar") || mime.contains("compress") =>
        {
            "package-x-generic".into()
        }
        _ => "text-x-generic".into(),
    };
    vec![flat.clone(), format!("gnome-mime-{flat}"), generic]
}

/// The name to draw a file with given a theme that answers `has`: the first of [`icons_for_mime`]'s
/// chain the theme carries, else the extension's name from [`icon_for_file`], else the chain's
/// last.
pub fn pick_icon(mime: Option<&str>, path: &Path, has: &dyn Fn(&str) -> bool) -> String {
    let chain = icons_for_mime(mime);
    if let Some(name) = chain.iter().find(|n| has(n)) {
        return name.clone();
    }
    let by_ext = icon_for_file(path);
    if has(by_ext) {
        return by_ext.to_string();
    }
    chain
        .last()
        .cloned()
        .unwrap_or_else(|| "text-x-generic".to_string())
}

/// The theme icon name a listing gives `it`: a shortcut's own icon, a folder's `.directory` icon (a
/// theme name `has` answers for, or an image path), the folder `specials` ([`crate::places::special_folders`])
/// and drive rows, else [`pick_icon`] down the MIME chain.
pub fn item_icon(
    it: &Item,
    specials: &[(PathBuf, &'static str)],
    has: &dyn Fn(&str) -> bool,
) -> String {
    if let Some(sc) = &it.shortcut {
        return sc.icon().to_string();
    }
    if it.is_dir {
        if let Some(own) = crate::meta::folder_icon(&it.path)
            && can_wear(&own, has)
        {
            return own;
        }
        for (dir, icon) in specials {
            if *dir == it.path {
                return icon.to_string();
            }
        }
        return it.drive.map_or("folder", |k| k.icon()).to_string();
    }
    pick_icon(it.mime.as_deref(), &it.path, has)
}

/// Whether a folder's own icon `name` can be drawn: an image's path, or a name the theme carries
/// (`has`). Another file manager may have written one the current theme lacks.
pub fn can_wear(name: &str, has: &dyn Fn(&str) -> bool) -> bool {
    name.starts_with('/') || has(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::places::DriveKind;

    #[test]
    fn mime_icons_run_from_exact_to_generic() {
        assert_eq!(
            icons_for_mime(Some("text/plain")),
            ["text-plain", "gnome-mime-text-plain", "text-x-generic"]
        );
        assert_eq!(
            icons_for_mime(Some("application/zip"))[2],
            "package-x-generic"
        );
        assert_eq!(icons_for_mime(Some("image/png"))[2], "image-x-generic");
        assert_eq!(icons_for_mime(Some("application/pdf"))[2], "text-x-generic");
        assert_eq!(icons_for_mime(None), ["text-x-generic"]);
    }

    #[test]
    fn icon_for_file_goes_by_extension() {
        assert_eq!(icon_for_file(Path::new("/a/b.PNG")), "image-x-generic");
        assert_eq!(icon_for_file(Path::new("/a/b")), "text-x-generic");
    }

    #[test]
    fn pick_icon_walks_the_chain_past_what_the_theme_lacks() {
        let txt = Path::new("/a/notes.txt");
        let has = |n: &str| n == "gnome-mime-text-plain";
        assert_eq!(
            pick_icon(Some("text/plain"), txt, &has),
            "gnome-mime-text-plain"
        );
        let has = |n: &str| n == "text-x-generic";
        assert_eq!(pick_icon(Some("text/plain"), txt, &has), "text-x-generic");
        let has = |_: &str| true;
        assert_eq!(pick_icon(Some("text/plain"), txt, &has), "text-plain");
    }

    #[test]
    fn pick_icon_falls_back_to_the_generic_when_the_theme_has_nothing() {
        let none = |_: &str| false;
        assert_eq!(
            pick_icon(Some("image/png"), Path::new("/a/b.png"), &none),
            "image-x-generic"
        );
        assert_eq!(
            pick_icon(None, Path::new("/a/odd.zzzq"), &none),
            "text-x-generic"
        );
        assert_eq!(
            pick_icon(None, Path::new("/a/run.sh"), &|n: &str| n
                == "text-x-generic"),
            "text-x-generic"
        );
    }

    #[test]
    fn a_listing_entry_picks_its_theme_icon_by_drive_then_mime() {
        let item = |is_dir: bool, drive: Option<DriveKind>, mime: Option<&str>| Item {
            name: "a".into(),
            path: PathBuf::from("/tmp/a"),
            is_dir,
            is_link: false,
            exec: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: mime.map(str::to_string),
            kind: String::new(),
            drive,
            trashed: None,
            shortcut: None,
        };
        let all = |_: &str| true;
        let icon = |it: &Item| item_icon(it, &[], &all);
        assert_eq!(icon(&item(true, None, None)), "folder");
        assert_eq!(
            icon(&item(true, Some(DriveKind::Local), None)),
            "drive-harddisk"
        );
        assert_eq!(
            icon(&item(true, Some(DriveKind::Optical), None)),
            "media-optical"
        );
        assert_eq!(icon(&item(false, None, Some("text/plain"))), "text-plain");
        assert_eq!(icon(&item(false, None, None)), "text-x-generic");
    }

    #[test]
    fn a_folder_wears_its_own_icon() {
        let dir = tempfile::tempdir().unwrap();
        let it = Item {
            name: "Tunes".into(),
            path: dir.path().to_path_buf(),
            is_dir: true,
            is_link: false,
            exec: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: None,
            kind: String::new(),
            drive: None,
            trashed: None,
            shortcut: None,
        };
        let has = |n: &str| n == "folder-music";
        crate::meta::set_folder_icon(dir.path(), Some("folder-music")).unwrap();
        assert_eq!(item_icon(&it, &[], &has), "folder-music");
        crate::meta::set_folder_icon(dir.path(), Some("/art/cover.png")).unwrap();
        assert_eq!(item_icon(&it, &[], &has), "/art/cover.png");
        crate::meta::set_folder_icon(dir.path(), Some("no-such-icon")).unwrap();
        assert_eq!(item_icon(&it, &[], &has), "folder");
    }

    #[test]
    fn a_file_falls_past_the_names_the_theme_lacks() {
        let it = Item {
            name: "notes.txt".into(),
            path: PathBuf::from("/tmp/notes.txt"),
            is_dir: false,
            is_link: false,
            exec: false,
            hidden: false,
            size: 0,
            modified: None,
            mime: Some("text/plain".into()),
            kind: "TXT File".into(),
            drive: None,
            trashed: None,
            shortcut: None,
        };
        let themed = |n: &str| n != "text-plain";
        assert_eq!(item_icon(&it, &[], &themed), "gnome-mime-text-plain");
    }
}
