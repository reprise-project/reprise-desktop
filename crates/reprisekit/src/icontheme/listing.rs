use super::IconCache;
use crate::raster::Canvas;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// A file listing's icon sizes in logical px at 100% text: List and Details, Icons and
/// Thumbnails, Tiles.
pub const LISTING_PX: [i32; 3] = [16, 32, 48];

/// [`LISTING_PX`] with List and Details' icon `list` px, their icon column grown with the text.
pub fn listing_px(list: i32) -> [i32; 3] {
    [list, LISTING_PX[1], LISTING_PX[2]]
}

/// One icon name's canvases at [`ListingIcons::sizes`].
pub type ListingSizes = [Option<std::rc::Rc<Canvas>>; 3];

/// A file listing's icons by name at [`ListingIcons::sizes`]: each name is loaded once and its
/// canvases shared by every entry wearing it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListingIcons {
    by_name: HashMap<String, ListingSizes>,
    device: i32,
    /// List and Details' icon size; 0 is [`LISTING_PX`]'s.
    list: i32,
    /// The sizes theme icons are fetched at; None is all three. Icon files always hold all three.
    drawn: Option<[bool; 3]>,
    /// Fetched with [`IconCache::get_unread`] ([`ListingIcons::unread`]).
    unread: bool,
    /// Names wearing a blank stand-in at some size until [`ListingIcons::landed`].
    waiting: HashSet<String>,
}

impl ListingIcons {
    /// For a host that draws on device pixels at `scale` from a scale-1 cache: every size is
    /// fetched `scale` times as large.
    pub fn device(scale: i32) -> ListingIcons {
        ListingIcons {
            device: scale,
            ..ListingIcons::default()
        }
    }

    /// List and Details' icons fetched at `list` px ([`listing_px`]).
    pub fn with_list(self, list: i32) -> ListingIcons {
        ListingIcons { list, ..self }
    }

    /// Theme icons fetched only at the sizes a view draws, by index into
    /// [`ListingIcons::sizes`]; [`ListingIcons::draw`] adds sizes.
    pub fn drawing(self, drawn: [bool; 3]) -> ListingIcons {
        let drawn = Some(drawn);
        ListingIcons { drawn, ..self }
    }

    /// For a host whose loop decodes no SVG: icons come from [`IconCache::get_unread`], and a size
    /// not ready yet wears a blank canvas until [`ListingIcons::landed`] swaps it.
    pub fn unread(self) -> ListingIcons {
        ListingIcons {
            unread: true,
            ..self
        }
    }

    /// `name` from `cache` at the sizes it is held at: a theme icon at the drawn ones, an icon
    /// file at all three.
    fn fetch(&mut self, cache: &mut IconCache, name: &str) -> ListingSizes {
        let (k, sizes, file) = (self.device.max(1), self.sizes(), name.starts_with('/'));
        let drawn = self.drawn;
        std::array::from_fn(|i| {
            (file || drawn.is_none_or(|d| d[i]))
                .then(|| one(cache, self.unread, &mut self.waiting, name, sizes[i] * k))
                .flatten()
        })
    }

    /// A view now draws the `drawn` sizes too: every theme icon is fetched at those it lacked, and
    /// each name's old and new canvases are returned for the entries to swap at once.
    pub fn draw(
        &mut self,
        cache: &mut IconCache,
        drawn: [bool; 3],
    ) -> Vec<(ListingSizes, ListingSizes)> {
        let Some(held) = self.drawn else {
            return Vec::new();
        };
        let wider = std::array::from_fn(|i| held[i] || drawn[i]);
        if wider == held {
            return Vec::new();
        }
        self.drawn = Some(wider);
        let k = self.device.max(1);
        let sizes = self.sizes();
        let mut swaps = Vec::new();
        for (name, sizes_held) in &mut self.by_name {
            if name.starts_with('/') {
                continue;
            }
            let mut new = sizes_held.clone();
            for i in (0..3).filter(|&i| wider[i] && !held[i]) {
                new[i] = one(cache, self.unread, &mut self.waiting, name, sizes[i] * k);
            }
            swaps.push((std::mem::replace(sizes_held, new.clone()), new));
        }
        swaps
    }

    /// The sizes each name is fetched at, in logical px.
    pub fn sizes(&self) -> [i32; 3] {
        match self.list {
            0 => LISTING_PX,
            list => listing_px(list),
        }
    }

    /// Fetches List and Details' icons at `list` px from now on; [`ListingIcons::refetch`]
    /// swaps the names already held. False when they already were.
    pub fn set_list(&mut self, list: i32) -> bool {
        let old = self.sizes()[0];
        self.list = list;
        old != list
    }

    pub fn get(&mut self, cache: &mut IconCache, name: &str) -> ListingSizes {
        if let Some(held) = self.by_name.get(name) {
            return held.clone();
        }
        let held = self.fetch(cache, name);
        self.by_name.insert(name.to_string(), held.clone());
        held
    }

    /// Fetches every name again from `cache`, now at another scale or list size: each name's old
    /// and new canvases. An icon file `cache` lacks at some size keeps its old ones and is
    /// returned second, for the host to decode off its loop and hand to [`ListingIcons::put`].
    pub fn refetch(
        &mut self,
        cache: &mut IconCache,
    ) -> (Vec<(ListingSizes, ListingSizes)>, Vec<String>) {
        let (k, sizes) = (self.device.max(1), self.sizes());
        let mut swaps = Vec::new();
        let mut files = Vec::new();
        let names: Vec<String> = self.by_name.keys().cloned().collect();
        for name in names {
            let new = if name.starts_with('/') {
                let put = sizes.map(|px| cache.file_put(&name, px * k));
                if put.iter().any(Option::is_none) {
                    files.push(name);
                    continue;
                }
                put.map(|c| {
                    c.flatten()
                        .map(|c| std::rc::Rc::new(Arc::unwrap_or_clone(c)))
                })
            } else {
                self.fetch(cache, &name)
            };
            let held = self.by_name.get_mut(&name).expect("a held name");
            swaps.push((std::mem::replace(held, new.clone()), new));
        }
        (swaps, files)
    }

    /// Whether a name still wears a blank stand-in.
    pub fn waits(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// Fetches the names wearing a stand-in again, once their icons landed in `cache`: each
    /// changed name's old and new canvases, and the names that decode at no size, for the host
    /// to pick again.
    pub fn landed(
        &mut self,
        cache: &mut IconCache,
    ) -> (Vec<(ListingSizes, ListingSizes)>, Vec<String>) {
        let mut swaps = Vec::new();
        let mut broken = Vec::new();
        for name in std::mem::take(&mut self.waiting) {
            let new = self.fetch(cache, &name);
            if self.waiting.contains(&name) && new == self.by_name[&name] {
                continue;
            }
            if !self.waiting.contains(&name) && new.iter().all(Option::is_none) {
                broken.push(name.clone());
            }
            let held = self.by_name.get_mut(&name).expect("a held name");
            swaps.push((std::mem::replace(held, new.clone()), new));
        }
        (swaps, broken)
    }

    /// The name whose canvases include `worn`.
    pub fn name_of(&self, worn: &std::rc::Rc<Canvas>) -> Option<&str> {
        self.by_name
            .iter()
            .find(|(_, sizes)| sizes.iter().flatten().any(|c| std::rc::Rc::ptr_eq(c, worn)))
            .map(|(name, _)| name.as_str())
    }

    /// Icon file `path` as the host decoded it at [`ListingIcons::sizes`]: its old and new canvases, or
    /// None when no entry wears it.
    pub fn put(
        &mut self,
        path: &str,
        art: [Option<Canvas>; 3],
    ) -> Option<(ListingSizes, ListingSizes)> {
        let sizes = self.by_name.get_mut(path)?;
        let new = art.map(|c| c.map(std::rc::Rc::new));
        Some((std::mem::replace(sizes, new.clone()), new))
    }
}

/// `name` at `px` from `cache`; with `unread`, a blank canvas while it is not ready, `name` noted
/// in `waiting`.
fn one(
    cache: &mut IconCache,
    unread: bool,
    waiting: &mut HashSet<String>,
    name: &str,
    px: i32,
) -> Option<Rc<Canvas>> {
    if !unread {
        return cache.get(name, px).map(Rc::new);
    }
    match cache.get_unread(name, px) {
        Some(icon) => icon.map(|c| Rc::new(Arc::unwrap_or_clone(c))),
        None => {
            waiting.insert(name.to_string());
            let s = cache.scale();
            Some(Rc::new(
                Canvas::new((px * s) as u32, (px * s) as u32).at_scale(s),
            ))
        }
    }
}
