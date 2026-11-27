//! The pictures beside one opened out of a compressed folder. The folder holds each by its
//! address (the archive's path joined with its path inside it); the one showing stands in it as
//! its copy, made when it shows into a private folder ([`Scratch`]) that goes when the next one
//! shows. The copy the viewer was opened on is Explorer's, and stays for its sweep.

use reprisefs::archive::{self, Entry, Kind, Scratch};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The most pictures of one archive folder the viewer steps through.
pub const MOST_PICTURES: usize = 65_536;

/// A picture copied out of the archive, and the holder of its folder.
#[derive(Debug)]
pub struct Copied {
    pub path: PathBuf,
    pub held: Scratch,
}

#[derive(Debug)]
struct Shown {
    address: PathBuf,
    copy: PathBuf,
    held: Option<Scratch>,
}

#[derive(Debug)]
pub struct Walk {
    pub zip: PathBuf,
    /// The archive's folder the pictures are in, as a path inside it.
    pub inner_dir: PathBuf,
    /// The archive's pictures in that folder, once listed, by address.
    entries: HashMap<PathBuf, Entry>,
    shown: Option<Shown>,
}

impl Walk {
    /// The walk for the picture at `path`, when it is a copy out of a compressed folder that
    /// says which entry it is.
    pub fn of(path: &Path) -> Option<Walk> {
        let (zip, inner) = reprisefs::archivecopy::origin_entry(path)?;
        Some(Walk {
            inner_dir: inner.parent().unwrap_or(Path::new("")).to_path_buf(),
            shown: Some(Shown {
                address: zip.join(&inner),
                copy: path.to_path_buf(),
                held: None,
            }),
            zip,
            entries: HashMap::new(),
        })
    }

    /// The entry the folder's `path` stands for, when it is an address not yet copied out.
    pub fn entry(&self, path: &Path) -> Option<&Entry> {
        self.entries.get(path)
    }

    /// Puts the address back in `pictures` for a copy no longer current (`at` moved off it),
    /// letting go of its folder.
    pub fn leave(&mut self, pictures: &mut [PathBuf], at: usize) {
        let Some(s) = self.shown.take_if(|s| pictures.get(at) != Some(&s.copy)) else {
            return;
        };
        if let Some(i) = pictures.iter().position(|p| *p == s.copy) {
            pictures[i] = s.address;
        }
    }

    /// The current picture, at address `pictures[at]`, came out as `copied`, which stands in for
    /// it while it shows.
    pub fn shown(&mut self, pictures: &mut [PathBuf], at: usize, copied: Copied) {
        let Some(address) = pictures.get(at).cloned() else {
            return;
        };
        pictures[at] = copied.path.clone();
        self.shown = Some(Shown {
            address,
            copy: copied.path,
            held: Some(copied.held),
        });
    }

    /// The archive folder's listing landed as `pictures`: their addresses, in order, with the
    /// one showing as its copy.
    pub fn listed(&mut self, pictures: Vec<Entry>) -> Vec<PathBuf> {
        self.entries = (pictures.into_iter())
            .map(|e| (self.zip.join(&e.path), e))
            .collect();
        let mut addresses: Vec<(String, PathBuf)> = (self.entries.keys())
            .map(|a| {
                (
                    a.file_name().unwrap_or_default().to_string_lossy().into(),
                    a.clone(),
                )
            })
            .collect();
        addresses.sort_by(|a, b| {
            reprisefs::pictures::natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1))
        });
        let mut out: Vec<PathBuf> = addresses.into_iter().map(|(_, a)| a).collect();
        if let Some(s) = &self.shown
            && let Some(i) = out.iter().position(|a| *a == s.address)
        {
            out[i] = s.copy.clone();
        }
        out
    }

    /// Lets the copy showing outlive the viewer, for a program it is handed to.
    pub fn hand_over_shown(&self) {
        if let Some(held) = self.shown.as_ref().and_then(|s| s.held.clone()) {
            held.hand_over();
        }
    }
}

/// The pictures by name straight inside the folder `inner_dir` of the archive listed as
/// `entries`, hidden ones left out, at most [`MOST_PICTURES`].
pub fn pictures(entries: &[Entry], inner_dir: &Path) -> Vec<Entry> {
    (entries.iter())
        .filter(|e| e.kind == Kind::File && e.path.parent() == Some(inner_dir))
        .filter(|e| {
            let name = e.path.file_name().unwrap_or_default();
            !name.as_encoded_bytes().starts_with(b".") && repriseapp::decode::is_image(&e.path)
        })
        .take(MOST_PICTURES)
        .cloned()
        .collect()
}

/// Lists the archive `zip` for the pictures of its folder `inner_dir`; none when it does not
/// list.
pub fn list(zip: &Path, inner_dir: &Path) -> Vec<Entry> {
    match archive::list(zip, &Default::default()) {
        Ok(entries) => pictures(&entries, inner_dir),
        Err(why) => {
            log::warn!("imageviewer: list {}: {why}", zip.display());
            Vec::new()
        }
    }
}

/// Copies `entry` of `zip` out into a fresh private folder, noting where it came from, unless it
/// is over `most` bytes.
pub fn copy_out(zip: &Path, entry: &Entry, most: u64) -> Result<Copied, String> {
    if entry.size > most {
        return Err(crate::load::too_large());
    }
    let held = Scratch::new().map_err(|e| e.to_string())?;
    let path = archive::copy_out(zip, entry, held.path())?;
    if let Err(e) = reprisefs::archivecopy::set_origin(&path, zip, &entry.path) {
        log::debug!(
            "imageviewer: {} keeps no note of {}: {e}",
            path.display(),
            zip.display()
        );
    }
    Ok(Copied { path, held })
}
