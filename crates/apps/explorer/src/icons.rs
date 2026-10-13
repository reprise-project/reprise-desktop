//! Which icon the window's own chrome and a folder's Properties sheet wear; a listing entry's
//! comes from `reprisefs::icons::item_icon`.

use repriseapp::Ctx;
use reprisekit::raster::Canvas;

/// A chrome icon: the first of `names` the theme carries, else the
/// magenta placeholder. Only the window's own buttons and links go
/// through this; a file's icon falls back down its MIME chain instead.
pub fn chrome_icon(ctx: &mut Ctx, names: &[&str], px: i32) -> Option<Canvas> {
    for n in names {
        if let Some(c) = ctx.icons.get(n, px) {
            return Some(c);
        }
    }
    log::debug!("icon theme has none of {names:?} at {px} px: placeholder");
    Some(reprisekit::classic::widgets::placeholder(px, px))
}

pub fn sheet_folder_icon(icon: &str, has: &dyn Fn(&str) -> bool) -> String {
    if reprisefs::icons::can_wear(icon, has) {
        icon.to_string()
    } else {
        reprisekit::controls::filesheet::STANDARD_FOLDER_ICON.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_sheet_shows_the_standard_folder_for_an_icon_the_theme_lacks() {
        let has = |n: &str| n == "folder" || n == "folder-music";
        assert_eq!(sheet_folder_icon("folder-music", &has), "folder-music");
        assert_eq!(
            sheet_folder_icon("/icons/tunes.png", &has),
            "/icons/tunes.png"
        );
        assert_eq!(sheet_folder_icon("folder-elsewhere", &has), "folder");
    }

    /// Names Explorer asks a real theme for, through a real `IconCache`. Skipped where no theme is installed (CI).
    #[test]
    fn the_installed_theme_answers_for_a_text_file() {
        let theme = reprisekit::icontheme::IconTheme::load(
            &std::env::var(reprisekit::icontheme::ENV).unwrap_or_else(|_| "hicolor".into()),
            &reprisekit::icontheme::IconTheme::default_roots(),
        );
        let mut icons = reprisekit::icontheme::IconCache::new(theme);
        if !icons.has("folder") {
            eprintln!("no icon theme installed: skipping");
            return;
        }
        for (mime, path) in [
            (Some("text/plain"), "/tmp/notes.txt"),
            (Some("application/x-shellscript"), "/tmp/run.sh"),
            (None, "/tmp/odd.zzzq"),
        ] {
            let name = reprisefs::icons::pick_icon(mime, Path::new(path), &|n| icons.has(n));
            assert!(
                icons.get(&name, 16).is_some(),
                "{path} resolved to {name}, which the theme does not carry"
            );
        }
    }
}
