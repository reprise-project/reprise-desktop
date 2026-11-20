//! Where an AVIF's AV1 pictures lie and how they are shown, read from its ISOBMFF boxes (HEIF,
//! ISO/IEC 23008-12, with MIAF's rules): the primary item, or the first sample of an image
//! sequence when there is none; a grid of tiles; the alpha auxiliary image and whether the colour
//! is premultiplied by it; `colr`, `clli`, `mdcv`, `irot` and `imir`. Item data is borrowed from
//! the file in at most [`MAX_EXTENTS`] extents an item, and an extent running to the end of the
//! file must be its item's only one. Items are looked up by id in maps built once, so a file of
//! many items parses in time linear in its length. `clap`, `pasp` and any other property are not
//! applied.

use std::collections::BTreeMap;

/// An AVIF's first picture and how to show it.
pub struct Avif<'a> {
    /// The picture's size before any turn: the primary item's `ispe` (which a grid's output size
    /// must equal), or a sequence's sample entry.
    pub size: (u32, u32),
    pub colour: Image<'a>,
    pub alpha: Option<Image<'a>>,
    /// The colour samples are multiplied by alpha.
    pub premultiplied: bool,
    pub nclx: Option<Nclx>,
    pub icc: Option<&'a [u8]>,
    /// `clli`'s brightest pixel, in nits.
    pub max_cll: Option<u16>,
    /// `mdcv`'s mastering display peak, in 1/10000 nits.
    pub mastering_peak: Option<u32>,
    /// `irot` and `imir` in the order they apply.
    pub transforms: Vec<Transform>,
}

pub enum Image<'a> {
    Coded(Coded<'a>),
    /// Tiles of one size, row by row, cropped to `size` at the right and bottom.
    Grid {
        cols: u32,
        rows: u32,
        size: (u32, u32),
        tiles: Vec<Coded<'a>>,
    },
}

/// An AV1 picture's bytes: its extents in order.
pub struct Coded<'a>(pub Vec<&'a [u8]>);

/// `colr`'s CICP code points (ITU-T H.273).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    /// Quarter turns anticlockwise.
    Turn(u8),
    /// `imir` mode 0.
    FlipTopBottom,
    /// `imir` mode 1.
    FlipLeftRight,
}

/// The most extents an item's data may lie in.
pub const MAX_EXTENTS: u16 = 16;

/// An `auxC` type naming an alpha plane.
const ALPHA: [&[u8]; 2] = [
    b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha",
    b"urn:mpeg:hevc:2015:auxid:1",
];

/// The first picture of AVIF `file`, None for a file this cannot read: not ISOBMFF, cut short,
/// a picture that is not AV1, or extents outside the file.
pub fn parse(file: &[u8]) -> Option<Avif<'_>> {
    let top = children(file)?;
    if top.first()?.0 != *b"ftyp" {
        return None;
    }
    let found = |kind: &[u8; 4]| top.iter().find(|b| b.0 == *kind).map(|b| b.1);
    if let Some(meta) = found(b"meta")
        && let Some(still) = still(file, meta)
    {
        return still;
    }
    sequence(file, found(b"moov")?)
}

/// The boxes in `b`, end to end: each one's type and body. None when one runs past `b`.
fn children(b: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut r = Bytes { b, at: 0 };
    while r.at < b.len() {
        let start = r.at;
        let size = r.u32()?;
        let kind: [u8; 4] = r.take(4)?.try_into().ok()?;
        let end = match size {
            0 => b.len(),
            1 => start.checked_add(usize::try_from(r.u64()?).ok()?)?,
            n => start.checked_add(n as usize)?,
        };
        if kind == *b"uuid" {
            r.take(16)?;
        }
        out.push((kind, b.get(r.at..end)?));
        r.at = end;
    }
    Some(out)
}

/// A reader of big-endian fields, each None past the end.
struct Bytes<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }

    /// An unsigned field of `n` bytes: 0, 4 or 8 in `iloc`.
    fn uint(&mut self, n: u8) -> Option<u64> {
        match n {
            0 => Some(0),
            4 => self.u32().map(u64::from),
            8 => self.u64(),
            _ => None,
        }
    }

    /// A u16 for `version` 0, else a u32.
    fn id(&mut self, version: u8) -> Option<u32> {
        if version == 0 {
            self.u16().map(u32::from)
        } else {
            self.u32()
        }
    }

    /// A full box's version, its flags skipped.
    fn version(&mut self) -> Option<u8> {
        let v = self.u8()?;
        self.take(3)?;
        Some(v)
    }

    fn rest(&mut self) -> &'a [u8] {
        let s = self.b.get(self.at..).unwrap_or(&[]);
        self.at = self.b.len();
        s
    }
}

struct Reference {
    kind: [u8; 4],
    from: u32,
    to: Vec<u32>,
}

/// The primary item of `meta`'s body: Some(None) when it holds one that cannot be read, None
/// when it has none (a sequence's `meta` may hold no picture).
fn still<'a>(file: &'a [u8], meta: &'a [u8]) -> Option<Option<Avif<'a>>> {
    let mut r = Bytes { b: meta, at: 0 };
    r.version()?;
    let boxes = children(r.rest())?;
    let found = |kind: &[u8; 4]| boxes.iter().find(|b| b.0 == *kind).map(|b| b.1);
    let primary = {
        let mut r = Bytes {
            b: found(b"pitm")?,
            at: 0,
        };
        let v = r.version()?;
        r.id(v)?
    };
    Some(item_picture(file, &boxes, primary))
}

fn item_picture<'a>(
    file: &'a [u8],
    boxes: &[([u8; 4], &'a [u8])],
    primary: u32,
) -> Option<Avif<'a>> {
    let found = |kind: &[u8; 4]| boxes.iter().find(|b| b.0 == *kind).map(|b| b.1);
    let types = item_types(found(b"iinf")?)?;
    let idat = found(b"idat");
    let locations = locations(file, idat, found(b"iloc")?)?;
    let refs = found(b"iref").map_or(Some(Vec::new()), references)?;
    let (props, assoc) = properties(found(b"iprp")?)?;
    let type_of = |id: u32| types.get(&id).copied();
    let data = |id: u32| locations.get(&id).map(|e| Coded(e.clone()));
    let props_of = |id: u32| -> Vec<([u8; 4], &'a [u8])> {
        let indices = assoc.get(&id).into_iter().flatten();
        indices
            .filter_map(|&i| props.get(usize::from(i).checked_sub(1)?).copied())
            .collect()
    };
    let image = |id: u32| -> Option<(Image<'a>, Option<(u32, u32)>)> {
        match &type_of(id)? {
            b"av01" => Some((Image::Coded(data(id)?), None)),
            b"grid" => {
                let extents = data(id)?.0;
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(extents.iter().map(|e| e.len()).sum())
                    .ok()?;
                extents.iter().for_each(|e| bytes.extend_from_slice(e));
                let mut r = Bytes { b: &bytes, at: 0 };
                let _version = r.u8()?;
                let wide = r.u8()? & 1 != 0;
                let rows = u32::from(r.u8()?) + 1;
                let cols = u32::from(r.u8()?) + 1;
                let mut side = || {
                    if wide {
                        r.u32()
                    } else {
                        r.u16().map(u32::from)
                    }
                };
                let size = (side()?, side()?);
                let tiles = refs
                    .iter()
                    .find(|x| x.kind == *b"dimg" && x.from == id)?
                    .to
                    .iter()
                    .map(|&t| (type_of(t)? == *b"av01").then(|| data(t))?)
                    .collect::<Option<Vec<_>>>()?;
                if tiles.len() as u64 != u64::from(rows) * u64::from(cols) {
                    return None;
                }
                let grid = Image::Grid {
                    cols,
                    rows,
                    size,
                    tiles,
                };
                Some((grid, Some(size)))
            }
            _ => None,
        }
    };
    let (colour, grid_size) = image(primary)?;
    let mut avif = Avif {
        size: (0, 0),
        colour,
        alpha: None,
        premultiplied: false,
        nclx: None,
        icc: None,
        max_cll: None,
        mastering_peak: None,
        transforms: Vec::new(),
    };
    let mut ispe = None;
    for (kind, body) in props_of(primary) {
        let mut r = Bytes { b: body, at: 0 };
        match &kind {
            b"ispe" => {
                r.version()?;
                ispe = Some((r.u32()?, r.u32()?));
            }
            b"colr" => colr(&mut avif, body),
            b"clli" => avif.max_cll = r.u16(),
            b"mdcv" => {
                r.take(16)?;
                avif.mastering_peak = r.u32();
            }
            b"irot" => avif.transforms.push(Transform::Turn(r.u8()? & 3)),
            b"imir" => avif.transforms.push(if r.u8()? & 1 == 0 {
                Transform::FlipTopBottom
            } else {
                Transform::FlipLeftRight
            }),
            _ => {}
        }
    }
    avif.size = match (ispe, grid_size) {
        (Some(s), Some(g)) if s != g => return None,
        (Some(s), _) | (None, Some(s)) => s,
        (None, None) => return None,
    };
    let is_alpha = |id: u32| {
        props_of(id).iter().any(|(kind, body)| {
            let mut r = Bytes { b: body, at: 0 };
            *kind == *b"auxC"
                && r.version().is_some()
                && ALPHA.contains(&r.rest().split(|&b| b == 0).next().unwrap_or(&[]))
        })
    };
    let alpha = refs
        .iter()
        .filter(|x| x.kind == *b"auxl" && x.to.contains(&primary))
        .map(|x| x.from)
        .find(|&id| is_alpha(id));
    if let Some(id) = alpha {
        avif.alpha = Some(image(id)?.0);
        avif.premultiplied = refs.iter().any(|x| {
            x.kind == *b"prem"
                && ((x.from == primary && x.to.contains(&id))
                    || (x.from == id && x.to.contains(&primary)))
        });
    }
    Some(avif)
}

/// Reads a `colr` body into `avif`: CICP code points, or an ICC profile.
fn colr<'a>(avif: &mut Avif<'a>, body: &'a [u8]) {
    let mut r = Bytes { b: body, at: 0 };
    match r.take(4) {
        Some(b"nclx") => {
            let nclx = (|| {
                Some(Nclx {
                    primaries: r.u16()?,
                    transfer: r.u16()?,
                    matrix: r.u16()?,
                    full_range: r.u8()? & 0x80 != 0,
                })
            })();
            avif.nclx = avif.nclx.or(nclx);
        }
        Some(b"prof" | b"rICC") => avif.icc = avif.icc.or(Some(r.rest())),
        _ => {}
    }
}

/// `iinf`'s item types by id, the first `infe` of an id kept; items of `infe` versions before 2
/// have no type and are left out.
fn item_types(iinf: &[u8]) -> Option<BTreeMap<u32, [u8; 4]>> {
    let mut r = Bytes { b: iinf, at: 0 };
    let v = r.version()?;
    r.id(v)?;
    let mut out = BTreeMap::new();
    for (kind, body) in children(r.rest())? {
        if kind != *b"infe" {
            continue;
        }
        let mut r = Bytes { b: body, at: 0 };
        let v = r.version()?;
        if v < 2 {
            continue;
        }
        let id = if v == 2 {
            u32::from(r.u16()?)
        } else {
            r.u32()?
        };
        r.u16()?;
        out.entry(id).or_insert(r.take(4)?.try_into().ok()?);
    }
    Some(out)
}

/// `iloc`'s items by id, each one's extents in `file` (construction method 0) or `idat` (1), the
/// first entry of an id kept. None past [`MAX_EXTENTS`], or for an extent running to the end
/// beside others.
fn locations<'a>(
    file: &'a [u8],
    idat: Option<&'a [u8]>,
    iloc: &[u8],
) -> Option<BTreeMap<u32, Vec<&'a [u8]>>> {
    let mut r = Bytes { b: iloc, at: 0 };
    let v = r.version()?;
    let sizes = r.u8()?;
    let (offset_size, length_size) = (sizes >> 4, sizes & 15);
    let sizes = r.u8()?;
    let base_size = sizes >> 4;
    let index_size = if v >= 1 { sizes & 15 } else { 0 };
    let count = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let id = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
        let method = if v >= 1 { r.u16()? & 15 } else { 0 };
        r.u16()?;
        let base = r.uint(base_size)?;
        let extents = r.u16()?;
        if extents > MAX_EXTENTS {
            return None;
        }
        let within = match method {
            0 => Some(file),
            1 => idat,
            _ => None,
        };
        let mut found = Vec::new();
        for _ in 0..extents {
            r.uint(index_size)?;
            let offset = usize::try_from(base.checked_add(r.uint(offset_size)?)?).ok()?;
            let len = usize::try_from(r.uint(length_size)?).ok()?;
            if len == 0 && extents > 1 {
                return None;
            }
            if let Some(within) = within {
                let end = if len == 0 {
                    within.len()
                } else {
                    offset.checked_add(len)?
                };
                found.push(within.get(offset..end)?);
            }
        }
        if within.is_some() {
            out.entry(id).or_insert(found);
        }
    }
    Some(out)
}

fn references(iref: &[u8]) -> Option<Vec<Reference>> {
    let mut r = Bytes { b: iref, at: 0 };
    let v = r.version()?;
    let mut out = Vec::new();
    for (kind, body) in children(r.rest())? {
        let mut r = Bytes { b: body, at: 0 };
        let from = r.id(v)?;
        let n = r.u16()?;
        let to = (0..n).map(|_| r.id(v)).collect::<Option<_>>()?;
        out.push(Reference { kind, from, to });
    }
    Some(out)
}

/// The properties of an `iprp` body (`ipco`'s boxes, numbered from 1) and each item's property
/// numbers by id (`ipma`), in the order they are listed.
#[allow(clippy::type_complexity)]
fn properties(iprp: &[u8]) -> Option<(Vec<([u8; 4], &[u8])>, BTreeMap<u32, Vec<u16>>)> {
    let boxes = children(iprp)?;
    let ipco = children(boxes.iter().find(|b| b.0 == *b"ipco")?.1)?;
    let mut assoc: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
    for (_, ipma) in boxes.iter().filter(|b| b.0 == *b"ipma") {
        let mut r = Bytes { b: ipma, at: 0 };
        let v = r.u8()?;
        let wide = r.take(3)?[2] & 1 != 0;
        for _ in 0..r.u32()? {
            let id = r.id(v)?;
            let n = r.u8()?;
            let indices = (0..n)
                .map(|_| {
                    if wide {
                        r.u16().map(|i| i & 0x7fff)
                    } else {
                        r.u8().map(|i| u16::from(i & 0x7f))
                    }
                })
                .collect::<Option<Vec<_>>>()?;
            assoc.entry(id).or_default().extend(indices);
        }
    }
    Some((ipco, assoc))
}

/// The first sample of a sequence's first AV1 picture track, with the first sample of its alpha
/// track (an `auxv` track whose `auxl` reference names it).
fn sequence<'a>(file: &'a [u8], moov: &'a [u8]) -> Option<Avif<'a>> {
    let tracks = children(moov)?
        .into_iter()
        .filter(|b| b.0 == *b"trak")
        .map(|b| track(file, b.1))
        .collect::<Option<Vec<_>>>()?;
    let colour = tracks
        .iter()
        .find(|t| matches!(&t.handler, b"pict" | b"vide") && t.sample.is_some())?;
    let alpha = tracks
        .iter()
        .find(|t| t.handler == *b"auxv" && t.sample.is_some() && t.aux_of.contains(&colour.id));
    let mut avif = Avif {
        size: colour.size,
        colour: Image::Coded(Coded(vec![colour.sample?])),
        alpha: alpha.and_then(|a| Some(Image::Coded(Coded(vec![a.sample?])))),
        premultiplied: false,
        nclx: None,
        icc: None,
        max_cll: None,
        mastering_peak: None,
        transforms: Vec::new(),
    };
    for (kind, body) in &colour.entry_boxes {
        let mut r = Bytes { b: body, at: 0 };
        match kind {
            b"colr" => colr(&mut avif, body),
            b"clli" => avif.max_cll = r.u16(),
            b"mdcv" => {
                r.take(16)?;
                avif.mastering_peak = r.u32();
            }
            _ => {}
        }
    }
    Some(avif)
}

struct Track<'a> {
    id: u32,
    handler: [u8; 4],
    /// The tracks an `auxl` reference names.
    aux_of: Vec<u32>,
    /// The first sample, when the first sample entry is AV1.
    sample: Option<&'a [u8]>,
    size: (u32, u32),
    /// The boxes inside the sample entry.
    entry_boxes: Vec<([u8; 4], &'a [u8])>,
}

fn track<'a>(file: &'a [u8], trak: &'a [u8]) -> Option<Track<'a>> {
    let boxes = children(trak)?;
    let found = |within: &[([u8; 4], &'a [u8])], kind: &[u8; 4]| {
        within.iter().find(|b| b.0 == *kind).map(|b| b.1)
    };
    let id = {
        let mut r = Bytes {
            b: found(&boxes, b"tkhd")?,
            at: 0,
        };
        let v = r.version()?;
        r.take(if v == 1 { 16 } else { 8 })?;
        r.u32()?
    };
    let mut aux_of = Vec::new();
    if let Some(tref) = found(&boxes, b"tref") {
        for (kind, body) in children(tref)? {
            if kind == *b"auxl" {
                let mut r = Bytes { b: body, at: 0 };
                while let Some(t) = r.u32() {
                    aux_of.push(t);
                }
            }
        }
    }
    let mdia = children(found(&boxes, b"mdia")?)?;
    let handler = found(&mdia, b"hdlr")?.get(8..12)?.try_into().ok()?;
    let minf = children(found(&mdia, b"minf")?)?;
    let stbl = children(found(&minf, b"stbl")?)?;
    let mut track = Track {
        id,
        handler,
        aux_of,
        sample: None,
        size: (0, 0),
        entry_boxes: Vec::new(),
    };
    let mut r = Bytes {
        b: found(&stbl, b"stsd")?,
        at: 0,
    };
    r.version()?;
    r.u32()?;
    let Some(&(kind, entry)) = children(r.rest())?.first() else {
        return Some(track);
    };
    if kind != *b"av01" {
        return Some(track);
    }
    let mut r = Bytes { b: entry, at: 24 };
    track.size = (u32::from(r.u16()?), u32::from(r.u16()?));
    track.entry_boxes = children(entry.get(78..)?)?;
    let size = {
        let mut r = Bytes {
            b: found(&stbl, b"stsz")?,
            at: 0,
        };
        r.version()?;
        match r.u32()? {
            0 => {
                (r.u32()? > 0).then_some(())?;
                r.u32()?
            }
            n => n,
        }
    };
    let offset = if let Some(stco) = found(&stbl, b"stco") {
        let mut r = Bytes { b: stco, at: 0 };
        r.version()?;
        (r.u32()? > 0).then_some(())?;
        u64::from(r.u32()?)
    } else {
        let mut r = Bytes {
            b: found(&stbl, b"co64")?,
            at: 0,
        };
        r.version()?;
        (r.u32()? > 0).then_some(())?;
        r.u64()?
    };
    let offset = usize::try_from(offset).ok()?;
    track.sample = Some(file.get(offset..offset.checked_add(size as usize)?)?);
    Some(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avif_pictures::{Item, full, grid, ispe, replaced_in_meta, single};

    fn picture() -> Vec<u8> {
        single(vec![0x12, 0, 0x0a, 0x0a], vec![ispe(16, 16)])
    }

    /// An `iloc` of one item, item 1, with `extents` extents of `offset` and `length` fields
    /// `sizes` bytes wide (0 or 4), each at offset 0 and one byte long.
    fn iloc(sizes: u8, extents: u16) -> Vec<u8> {
        let mut b = vec![sizes << 4 | sizes, 0];
        b.extend(1u16.to_be_bytes());
        b.extend(1u16.to_be_bytes());
        b.extend(0u16.to_be_bytes());
        b.extend(extents.to_be_bytes());
        for _ in 0..extents {
            if sizes == 4 {
                b.extend(0u32.to_be_bytes());
                b.extend(1u32.to_be_bytes());
            }
        }
        full(b"iloc", &b)
    }

    #[test]
    fn an_item_of_many_extents_or_of_whole_file_extents_beside_others_is_unreadable() {
        let with = |sizes, extents| replaced_in_meta(&picture(), b"iloc", &iloc(sizes, extents));
        let item = |file: &[u8]| match parse(file).map(|a| a.colour) {
            Some(Image::Coded(c)) => Some(c.0.len()),
            _ => None,
        };
        assert_eq!(item(&with(4, MAX_EXTENTS)), Some(16));
        assert_eq!(item(&with(4, MAX_EXTENTS + 1)), None);
        assert_eq!(item(&with(0, 1)), Some(1), "one extent, the whole file");
        assert_eq!(
            item(&with(0, 2)),
            None,
            "two whole-file extents in zero-width fields"
        );
        assert_eq!(
            item(&with(0, u16::MAX)),
            None,
            "a 6-byte entry naming the whole file 65535 times"
        );
    }

    #[test]
    fn a_grid_of_many_tiles_parses_in_time_linear_in_its_items() {
        let tiles: Vec<u32> = (2..2 + 255 * 255).collect();
        let mut items: Vec<Item> = tiles
            .iter()
            .map(|&id| Item::av01(id, vec![0], vec![]))
            .collect();
        items.push(Item {
            id: 1,
            kind: *b"grid",
            data: grid(255, 255, 255, 255),
            props: vec![ispe(255, 255)],
        });
        let file = crate::avif_pictures::still(&items, 1, &[(b"dimg", 1, &tiles)]);
        let started = std::time::Instant::now();
        let avif = parse(&file).expect("the grid");
        let took = started.elapsed();
        assert!(matches!(avif.colour, Image::Grid { ref tiles, .. } if tiles.len() == 255 * 255));
        assert!(took.as_secs() < 5, "parsed in {took:?}");
    }
}
