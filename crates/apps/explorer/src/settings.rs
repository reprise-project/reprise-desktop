//! Per-window preferences in `$XDG_STATE_HOME/reprise/explorer.toml`; every field has a default.

use reprisekit::controls::{explorerbar, listview};
use reprisekit::theme::TextUnits;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: String,
    pub sort: (usize, bool),
    /// The Folders tree holds the left column; off gives it to the task
    /// pane.
    pub folders: bool,
    pub status_bar: bool,
    pub toolbars: bool,
    pub address_bar: bool,
    pub bar_width: i32,
    /// The main window's client size in 100% px, grown by the text size at open.
    pub window: (i32, i32),
    /// Details view column widths in 100% px, grown by the text size when
    /// drawn; empty means the built-in defaults.
    pub columns: Vec<i32>,
    pub show_hidden: bool,
    /// Which task pane sections the user has collapsed, by title. A
    /// title that is not here is open, so a new section needs no entry.
    pub pane_open: BTreeMap<String, bool>,
    /// The square, in pixels, thumbnails must stay sharp at. The frame shows
    /// 94 logical px; a scaled screen needs more (188 for 2x). Pictures are
    /// decoded to the cache size that holds it (128, 256 or 512). Clamped to
    /// [`Settings::THUMBNAIL_SIZE`].
    pub thumbnail_size: u32,
    pub thumbnail_cache: ThumbCacheSettings,
}

/// `[thumbnail_cache]` in `explorer.toml`: whether decoded thumbnails are
/// kept in `$XDG_CACHE_HOME/thumbnails` (shared with other file managers)
/// and how much room they may take. Explorer has no page for it; edit the
/// file with Explorer closed (a closing window writes it back).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThumbCacheSettings {
    pub enabled: bool,
    /// The cap in MiB; past it the least recently used thumbnails go,
    /// whichever program wrote them.
    pub max_mb: u32,
}

impl ThumbCacheSettings {
    /// The cap's bounds in MiB. A thumbnail is at most 64 KiB of pixels at the
    /// default `thumbnail_size` and 1 MiB at the largest; raise the cap with
    /// the size.
    pub const MAX_MB: std::ops::RangeInclusive<u32> = 1..=4096;

    pub fn max_bytes(&self) -> u64 {
        u64::from(self.max_mb) << 20
    }
}

impl Default for ThumbCacheSettings {
    fn default() -> ThumbCacheSettings {
        ThumbCacheSettings {
            enabled: true,
            max_mb: 512,
        }
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            mode: "tiles".to_string(),
            sort: (0, true),
            folders: true,
            status_bar: true,
            toolbars: true,
            address_bar: true,
            bar_width: 200,
            // The folder window opens at an outer 800x600; the client
            // area that leaves under its frame, caption and menu bar is
            // 792x573, and `initial_size` wants the client size.
            window: (792, 573),
            columns: Vec::new(),
            show_hidden: false,
            pane_open: BTreeMap::new(),
            thumbnail_size: crate::thumbs::FRAME_INNER,
            thumbnail_cache: ThumbCacheSettings::default(),
        }
    }
}

impl Settings {
    /// `thumbnail_size`'s bounds: no smaller than the frame it fills,
    /// no larger than a 1 MiB thumbnail.
    pub const THUMBNAIL_SIZE: std::ops::RangeInclusive<u32> = crate::thumbs::FRAME_INNER..=512;

    /// The main window's client size where text grows by `units`.
    pub fn window_at(&self, units: TextUnits) -> (i32, i32) {
        units.size(self.window)
    }

    /// Keep client size `px`, drawn where text grows by `units`, as [`Settings::window`].
    pub fn keep_window(&mut self, px: (i32, i32), units: TextUnits) {
        self.window = (units.within(px.0), units.within(px.1));
    }

    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state"))
            })?;
        Some(base.join("reprise").join("explorer.toml"))
    }

    pub fn load() -> Settings {
        let Some(path) = Self::path() else {
            return Settings::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Settings::default();
        };
        match toml::from_str::<Settings>(&text) {
            Ok(s) => s.clamped(),
            Err(e) => {
                log::warn!("explorer settings: ignoring {}: {e}", path.display());
                Settings::default()
            }
        }
    }

    fn clamped(mut self) -> Settings {
        self.bar_width = self.bar_width.max(explorerbar::MIN_W);
        self.window = (
            self.window.0.max(repriseapp::MIN_SIZE.0),
            self.window.1.max(repriseapp::MIN_SIZE.1),
        );
        for w in &mut self.columns {
            *w = (*w).max(listview::MIN_COL_W);
        }
        let px = Self::THUMBNAIL_SIZE;
        self.thumbnail_size = self.thumbnail_size.clamp(*px.start(), *px.end());
        let mb = ThumbCacheSettings::MAX_MB;
        self.thumbnail_cache.max_mb = self.thumbnail_cache.max_mb.clamp(*mb.start(), *mb.end());
        self
    }

    pub fn save(&self) {
        if let Some(path) = Self::path() {
            self.save_to(&path);
        }
    }

    fn save_to(&self, path: &Path) {
        match toml::to_string(self) {
            Ok(text) => {
                if let Err(e) = repriseconf::write_atomic(path, &text) {
                    log::warn!("explorer settings: could not write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("explorer settings: could not serialize: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_replaces_the_file_so_an_open_reader_keeps_the_old_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise").join("explorer.toml");
        Settings::default().save_to(&path);
        let old = std::fs::read_to_string(&path).unwrap();
        let mut reader = std::fs::File::open(&path).unwrap();
        let s = Settings {
            mode: "details".to_string(),
            ..Settings::default()
        };
        s.save_to(&path);
        let mut seen = String::new();
        std::io::Read::read_to_string(&mut reader, &mut seen).unwrap();
        assert_eq!(seen, old, "the open handle still reads the old file");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn load_raises_zero_and_negative_sizes_to_their_minimums() {
        let s = toml::from_str::<Settings>(
            "bar_width = -5\nwindow = [0, -300]\ncolumns = [0, -40, 180]\n",
        )
        .unwrap()
        .clamped();
        assert_eq!(s.bar_width, explorerbar::MIN_W);
        assert_eq!(s.window, repriseapp::MIN_SIZE);
        assert_eq!(s.columns, [listview::MIN_COL_W, listview::MIN_COL_W, 180]);
        assert_eq!(Settings::default().clamped(), Settings::default());
    }

    #[test]
    fn the_window_is_kept_in_100_units_and_grows_by_the_text_size() {
        let mut s = Settings::default();
        let twice = TextUnits::new(22, 11);
        assert_eq!(s.window_at(TextUnits::ONE), (792, 573));
        assert_eq!(s.window_at(twice), (1584, 1146));
        s.keep_window((1584, 1146), twice);
        assert_eq!(s.window, (792, 573));
        s.keep_window((1001, 700), twice);
        assert_eq!(s.window, (500, 350), "never reopens larger than it closed");
        let half_again = TextUnits::new(17, 11);
        for side in [120, 573, 792, 1001] {
            s.keep_window(half_again.size((side, side)), half_again);
            assert_eq!(s.window, (side, side), "{side} comes back from 150%");
        }
    }

    #[test]
    fn the_thumbnail_cache_reads_from_its_own_table() {
        let s = Settings::default();
        assert_eq!(
            s.thumbnail_cache,
            ThumbCacheSettings {
                enabled: true,
                max_mb: 512
            }
        );
        let s = toml::from_str::<Settings>("[thumbnail_cache]\nenabled = false\n").unwrap();
        assert!(!s.thumbnail_cache.enabled);
        assert_eq!(s.thumbnail_cache.max_mb, 512);
        let cap = |mb: u32| {
            toml::from_str::<Settings>(&format!("[thumbnail_cache]\nmax_mb = {mb}\n"))
                .unwrap()
                .clamped()
                .thumbnail_cache
                .max_mb
        };
        assert_eq!(cap(0), 1);
        assert_eq!(cap(256), 256);
        assert_eq!(cap(1_000_000), 4096);
        let back = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&back).unwrap(), s);
    }

    #[test]
    fn the_thumbnail_size_defaults_to_the_frame_and_is_clamped() {
        assert_eq!(Settings::default().thumbnail_size, 94);
        let size = |px: u32| {
            toml::from_str::<Settings>(&format!("thumbnail_size = {px}\n"))
                .unwrap()
                .clamped()
                .thumbnail_size
        };
        assert_eq!(size(16), 94);
        assert_eq!(size(256), 256);
        assert_eq!(size(4000), 512);
    }

    #[test]
    fn default_is_tiles_with_every_bar_shown() {
        let s = Settings::default();
        assert_eq!(s.mode, "tiles");
        assert_eq!(s.sort, (0, true));
        assert!(s.folders && s.status_bar && s.toolbars && s.address_bar);
        assert_eq!(s.window, (792, 573));
        assert!(!s.show_hidden);
    }

    #[test]
    fn round_trips_through_toml_and_tolerates_a_partial_file() {
        let s = Settings {
            mode: "details".to_string(),
            sort: (2, false),
            columns: vec![200, 80, 100],
            show_hidden: true,
            ..Settings::default()
        };
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
        assert_eq!(
            toml::from_str::<Settings>("mode = \"list\"\n").unwrap(),
            Settings {
                mode: "list".to_string(),
                ..Settings::default()
            }
        );
        assert_eq!(toml::from_str::<Settings>("").unwrap(), Settings::default());
    }

    #[test]
    fn a_collapsed_pane_section_round_trips() {
        let mut s = Settings::default();
        s.pane_open.insert("Details".to_string(), false);
        s.pane_open.insert("Other Places".to_string(), true);
        let text = toml::to_string(&s).unwrap();
        let back = toml::from_str::<Settings>(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.pane_open.get("Details"), Some(&false));
        assert!(
            toml::from_str::<Settings>("mode = \"list\"\n")
                .unwrap()
                .pane_open
                .is_empty()
        );
    }
}
