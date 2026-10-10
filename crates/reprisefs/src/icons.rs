use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
