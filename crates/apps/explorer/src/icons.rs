//! Which icon the window's own chrome, its dialogs and a folder's Properties sheet wear; a listing
//! entry's comes from `reprisefs::icons::item_icon`.

use repriseapp::iconjobs::IconWaits;
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;
use std::path::{Path, PathBuf};

/// A chrome icon: the first of `names` the theme carries, else the magenta placeholder; a blank
/// stand-in noted in `waits` while a name decodes off the loop. Only the window's own buttons,
/// links and boxes go through this; a file's icon falls back down its MIME chain instead.
pub fn chrome_icon_unread(
    waits: &mut IconWaits,
    icons: &mut IconCache,
    names: &[&str],
    px: i32,
) -> Option<Canvas> {
    (waits.first(icons, names, px)).or_else(|| placeholder(names, px, icons.scale()))
}

fn placeholder(names: &[&str], px: i32, scale: i32) -> Option<Canvas> {
    log::debug!("icon theme has none of {names:?} at {px} px: placeholder");
    let s = scale.max(1);
    let device = (px * s) as u32;
    Some(
        reprisekit::classic::widgets::placeholder(px, px)
            .resample_box(device, device)
            .at_scale(s),
    )
}

/// The icon of the name `pick` picks by what the theme has: a blank stand-in noted in `waits`
/// while it decodes, and a picked name that decodes at no size refused and picked again.
pub fn picked_icon(
    waits: &mut IconWaits,
    icons: &mut IconCache,
    px: i32,
    pick: impl Fn(&dyn Fn(&str) -> bool) -> String,
) -> Option<Canvas> {
    let mut tried = Vec::new();
    loop {
        let name = pick(&|n| icons.has_unread(n));
        if tried.contains(&name) {
            return None;
        }
        match waits.get(icons, &name, px) {
            Ok(None) => {
                icons.refuse(&name);
                tried.push(name);
            }
            Ok(icon) => return icon,
            Err(blank) => return Some(blank),
        }
    }
}

/// Where an [`IconSlot`] finds its icon.
#[derive(Debug)]
pub enum Pick {
    /// [`chrome_icon_unread`] of these names.
    Chrome(Vec<String>),
    /// The icon a file of `mime` at `path` wears, or the folder's.
    File {
        path: PathBuf,
        mime: Option<String>,
        is_dir: bool,
    },
}

impl Pick {
    pub fn chrome(names: &[&str]) -> Pick {
        Pick::Chrome(names.iter().map(|n| n.to_string()).collect())
    }
}

/// One icon a dialog wears: a blank stand-in while its name decodes off the loop, the icon once
/// [`IconSlot::land`] sees it landed.
#[derive(Debug)]
pub struct IconSlot {
    pick: Pick,
    px: i32,
    waits: IconWaits,
    icon: Option<Canvas>,
}

impl IconSlot {
    pub fn new(icons: &mut IconCache, pick: Pick, px: i32) -> IconSlot {
        let mut slot = IconSlot {
            pick,
            px,
            waits: IconWaits::default(),
            icon: None,
        };
        slot.wear(icons);
        slot
    }

    pub fn chrome(icons: &mut IconCache, names: &[&str], px: i32) -> IconSlot {
        IconSlot::new(icons, Pick::chrome(names), px)
    }

    pub fn icon(&self) -> Option<Canvas> {
        self.icon.clone()
    }

    fn wear(&mut self, icons: &mut IconCache) {
        let (waits, px) = (&mut self.waits, self.px);
        *waits = IconWaits::default();
        self.icon = match &self.pick {
            Pick::Chrome(names) => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                chrome_icon_unread(waits, icons, &names, px)
            }
            Pick::File { is_dir: true, .. } => waits.first(icons, &["folder"], px),
            Pick::File { path, mime, .. } => picked_icon(waits, icons, px, |has| {
                reprisefs::icons::pick_icon(mime.as_deref(), path, has)
            }),
        };
    }

    /// Wears the icon again once a name it waited on landed: true when it did.
    pub fn land(&mut self, icons: &mut IconCache) -> bool {
        if !self.waits.landed(icons) {
            return false;
        }
        self.wear(icons);
        true
    }
}

/// `form`'s picture `id` wears `slot`'s icon once it landed: true when it did.
pub fn land_picture(
    form: &mut reprisekit::controls::form::Form,
    id: &str,
    slot: &mut IconSlot,
    icons: &mut IconCache,
) -> bool {
    if !slot.land(icons) {
        return false;
    }
    if let Some(it) = form.item_mut(id)
        && let reprisekit::controls::form::Control::Picture { canvas, .. } = &mut it.control
    {
        *canvas = slot.icon();
    }
    true
}

/// The [`Pick`] of a file at `path`, typed by its name alone, or of a folder.
pub fn file_pick(globs: &reprisefs::apps::MimeDb, path: &Path, is_dir: bool) -> Pick {
    let name = path.file_name().map(|n| n.to_string_lossy());
    Pick::File {
        path: path.to_path_buf(),
        mime: name.and_then(|n| globs.mime_of_name(&n)),
        is_dir,
    }
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
    fn the_placeholder_is_drawn_at_the_cache_scale() {
        let mut ctx = crate::tests::test_ctx();
        ctx.icons.set_scale(2);
        let mut waits = IconWaits::default();
        let c = chrome_icon_unread(&mut waits, &mut ctx.icons, &["no-such-icon"], 24)
            .expect("a placeholder");
        assert_eq!((c.width, c.height, c.scale()), (24, 24, 2));
        assert_eq!(c.device_size(), (48, 48));
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
