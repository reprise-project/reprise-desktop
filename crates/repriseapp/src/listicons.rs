//! The icons a folder listing wears, fetched again when its window draws at a new scale.

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};

use crate::Waker;
use reprisekit::controls::shellview::{self, Entry};
use reprisekit::icontheme::{IconCache, LISTING_PX, ListingIcons, decode_file};
use reprisekit::raster::Canvas;

type Art = Vec<(String, [Option<Canvas>; 3])>;

/// An Explorer or file dialog listing's icons: a new draw scale swaps the entries' icons without
/// listing the folder again, theme icons at once and icon files once a thread decodes them.
#[derive(Default)]
pub struct ListIcons {
    icons: ListingIcons,
    scale: i32,
    files: Option<Receiver<Art>>,
}

impl ListIcons {
    /// A new listing wears `icons`, fetched from a cache at `scale`.
    pub fn listed(&mut self, icons: ListingIcons, scale: i32) {
        *self = ListIcons {
            icons,
            scale,
            files: None,
        };
    }

    /// The window draws at `scale`, `cache`'s scale: `entries` wear its theme icons now, and the
    /// icon files `cache` lacks decode on a thread that fires `wake` ([`ListIcons::take`]).
    pub fn follow(
        &mut self,
        scale: i32,
        cache: &mut IconCache,
        entries: &mut [Entry],
        wake: &Waker,
    ) {
        if std::mem::replace(&mut self.scale, scale) == scale {
            return;
        }
        let (swaps, files) = self.icons.refetch(cache);
        shellview::swap_icons(entries, &swaps);
        // Dropping the receiver drops what a thread for the old scale still sends.
        self.files = None;
        if files.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let wake = wake.clone();
        let spawned = std::thread::Builder::new()
            .name("list icons".into())
            .spawn(move || {
                let art: Art = files
                    .into_iter()
                    .map(|f| {
                        let a = LISTING_PX.map(|px| decode_file(Path::new(&f), px, scale));
                        (f, a)
                    })
                    .collect();
                if tx.send(art).is_ok() {
                    wake.wake();
                }
            });
        match spawned {
            Ok(_) => self.files = Some(rx),
            Err(e) => log::warn!("cannot spawn an icon thread ({e}); icon files keep their scale"),
        }
    }

    /// Hands `entries` the icon files decoded for the last scale change, and `cache` keeps them
    /// at that scale: true when an entry changed. A file that no longer decodes keeps its old
    /// icons.
    pub fn take(&mut self, entries: &mut [Entry], cache: &mut IconCache) -> bool {
        let Some(art) = self.files.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.files = None;
        let art: Vec<_> = art
            .into_iter()
            .filter(|(_, a)| a.iter().any(Option::is_some))
            .collect();
        let puts = art.iter().flat_map(|(f, a)| {
            LISTING_PX
                .into_iter()
                .zip(a)
                .map(|(px, c)| (f.clone(), px, c.clone()))
        });
        cache.put_files_at(puts.collect(), self.scale);
        let swaps: Vec<_> = art
            .into_iter()
            .filter_map(|(f, a)| self.icons.put(&f, a))
            .collect();
        shellview::swap_icons(entries, &swaps);
        !swaps.is_empty()
    }
}
