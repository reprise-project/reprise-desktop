//! Which icon a thing in the window wears: the window's own chrome, a
//! listing entry, a drive. Everything here answers with a theme icon
//! name (or the placeholder when the theme carries none of the
//! candidates); the drawing is the controls' business.

use repriseapp::Ctx;
use reprisefs::listing::Item;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

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

/// The theme icon name for a listing entry: a shortcut's own icon, a folder's `.directory` icon (a theme name the theme carries, or an image path), the folder specials (`specials`, from `crate::place::special_folders`) and drive rows, else `reprisefs::icons::pick_icon` walking the MIME chain against what the theme carries (`has`).
pub fn entry_icon_name(
    it: &Item,
    specials: &[(PathBuf, &'static str)],
    has: &dyn Fn(&str) -> bool,
) -> String {
    if let Some(sc) = &it.shortcut {
        return sc.icon().to_string();
    }
    if it.is_dir {
        if let Some(own) = reprisefs::meta::folder_icon(&it.path)
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
    reprisefs::icons::pick_icon(it.mime.as_deref(), &it.path, has)
}

/// Whether a folder's own icon `name` can be drawn: an image's path, or
/// a name the theme carries (`has`). Another file manager may have
/// written one the current theme lacks.
fn can_wear(name: &str, has: &dyn Fn(&str) -> bool) -> bool {
    name.starts_with('/') || has(name)
}

pub fn sheet_folder_icon(icon: &str, has: &dyn Fn(&str) -> bool) -> String {
    if can_wear(icon, has) {
        icon.to_string()
    } else {
        reprisekit::controls::filesheet::STANDARD_FOLDER_ICON.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisefs::places::DriveKind;
    use std::path::{Path, PathBuf};

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
        let icon = |it: &Item| entry_icon_name(it, &[], &all);
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
    fn a_sheet_shows_the_standard_folder_for_an_icon_the_theme_lacks() {
        let has = |n: &str| n == "folder" || n == "folder-music";
        assert_eq!(sheet_folder_icon("folder-music", &has), "folder-music");
        assert_eq!(
            sheet_folder_icon("/icons/tunes.png", &has),
            "/icons/tunes.png"
        );
        assert_eq!(sheet_folder_icon("folder-elsewhere", &has), "folder");
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
        reprisefs::meta::set_folder_icon(dir.path(), Some("folder-music")).unwrap();
        assert_eq!(entry_icon_name(&it, &[], &has), "folder-music");
        reprisefs::meta::set_folder_icon(dir.path(), Some("/art/cover.png")).unwrap();
        assert_eq!(entry_icon_name(&it, &[], &has), "/art/cover.png");
        reprisefs::meta::set_folder_icon(dir.path(), Some("no-such-icon")).unwrap();
        assert_eq!(entry_icon_name(&it, &[], &has), "folder");
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
        assert_eq!(entry_icon_name(&it, &[], &themed), "gnome-mime-text-plain");
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
