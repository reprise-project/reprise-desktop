//! Theme names Reprise draws itself. The configured theme, its inherits and `hicolor` keep their
//! own picture for such a name (or its legacy alias); where none of them has one, or theirs is
//! the same file as the generic icon beside it, Reprise's own drawing stands in. The fallback
//! themes never supply one.

use crate::art::icons::{Icon, TEXT_16, TEXT_32, TEXT_48};
use crate::raster::Canvas;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

static TEXT: [&Icon; 3] = [&TEXT_16, &TEXT_32, &TEXT_48];

pub(super) struct Own {
    /// Other spellings a theme may carry the picture under, tried after the name.
    pub(super) aliases: &'static [&'static str],
    /// The generic name a theme may give this one's file.
    generic: &'static str,
    /// Smallest first.
    icons: &'static [&'static Icon],
}

impl Own {
    pub(super) fn of(name: &str) -> Option<Own> {
        match name {
            "text-plain" => Some(Own {
                aliases: &["gnome-mime-text-plain"],
                generic: "text-x-generic",
                icons: &TEXT,
            }),
            _ => None,
        }
    }

    /// Whether theme file `path` is the generic icon in its directory: the same file through any
    /// chain of links or a hard link, or a copy of it byte for byte.
    pub(super) fn is_generic(&self, path: &Path) -> bool {
        let Ok(mine) = std::fs::metadata(path) else {
            return false;
        };
        ["png", "svg", "xpm"].iter().any(|ext| {
            let generic = path.with_file_name(format!("{}.{ext}", self.generic));
            let Ok(g) = std::fs::metadata(&generic) else {
                return false;
            };
            (g.dev(), g.ino()) == (mine.dev(), mine.ino())
                || (g.len() == mine.len()
                    && std::fs::read(&generic).ok() == std::fs::read(path).ok())
        })
    }

    /// The drawing at `dpx` device px, kept sharp: the drawing and whole multiple (up to
    /// [`MAX_CELLS`]) that fill the most of the slot, centred. A slot larger than every drawing
    /// and more than 5/4 of that fill takes a resample of the largest drawing instead.
    pub(super) fn draw(&self, dpx: i32) -> Canvas {
        let dpx = dpx.max(1);
        let mut c = Canvas::new(dpx as u32, dpx as u32);
        // Smallest first, so a tie goes to the larger drawing at fewer cells.
        let best = (self.icons.iter())
            .map(|&i| (i, i.size().0 as i32))
            .map(|(i, side)| (i, side, (dpx / side).min(MAX_CELLS)))
            .filter(|&(_, _, cells)| cells > 0)
            .max_by_key(|&(_, side, cells)| side * cells);
        let largest = self.icons[self.icons.len() - 1];
        match best {
            Some((icon, side, cells))
                if dpx <= largest.size().0 as i32 || dpx * 4 <= side * cells * 5 =>
            {
                let at = (dpx - side * cells) / 2;
                icon.paint(&mut c, at, at, cells);
                c
            }
            Some(_) => largest.canvas().resample_box(c.width, c.height),
            None => self.icons[0].canvas().resample_box(c.width, c.height),
        }
    }
}

/// The most device px one drawing pixel spans.
const MAX_CELLS: i32 = 4;
