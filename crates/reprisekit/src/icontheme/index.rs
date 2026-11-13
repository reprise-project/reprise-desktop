use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,
    pub size: i32,
    pub scale: i32,
    pub kind: DirKind,
    pub min: i32,
    pub max: i32,
    pub threshold: i32,
}

impl DirEntry {
    pub fn matches(&self, size: i32, scale: i32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => {
                (self.size - self.threshold) <= size && size <= (self.size + self.threshold)
            }
        }
    }

    /// Whether the biggest icon here is smaller than `size` at `scale`, so it would be scaled up.
    pub fn upscales(&self, size: i32, scale: i32) -> bool {
        let biggest = match self.kind {
            DirKind::Fixed => self.size,
            DirKind::Scalable => self.max,
            DirKind::Threshold => self.size + self.threshold,
        };
        biggest * self.scale < size * scale
    }

    /// `DirectorySizeDistance` from the spec, in pixels.
    pub fn distance(&self, size: i32, scale: i32) -> i32 {
        let want = size * scale;
        match self.kind {
            DirKind::Fixed => (self.size * self.scale - want).abs(),
            DirKind::Scalable => {
                if want < self.min * self.scale {
                    self.min * self.scale - want
                } else if want > self.max * self.scale {
                    want - self.max * self.scale
                } else {
                    0
                }
            }
            DirKind::Threshold => {
                let lo = (self.size - self.threshold) * self.scale;
                let hi = (self.size + self.threshold) * self.scale;
                if want < lo {
                    lo - want
                } else if want > hi {
                    want - hi
                } else {
                    0
                }
            }
        }
    }
}

/// The keys `parse_index` reads from one section; the first of a repeated key wins.
#[derive(Default)]
struct Keys<'a> {
    inherits: Option<&'a str>,
    directories: Option<&'a str>,
    scaled_directories: Option<&'a str>,
    size: Option<&'a str>,
    scale: Option<&'a str>,
    kind: Option<&'a str>,
    threshold: Option<&'a str>,
    min_size: Option<&'a str>,
    max_size: Option<&'a str>,
}

impl<'a> Keys<'a> {
    fn set(&mut self, key: &str, value: &'a str) {
        let slot = match key {
            "Inherits" => &mut self.inherits,
            "Directories" => &mut self.directories,
            "ScaledDirectories" => &mut self.scaled_directories,
            "Size" => &mut self.size,
            "Scale" => &mut self.scale,
            "Type" => &mut self.kind,
            "Threshold" => &mut self.threshold,
            "MinSize" => &mut self.min_size,
            "MaxSize" => &mut self.max_size,
            _ => return,
        };
        slot.get_or_insert(value);
    }
}

/// Each section's keys, by section name; lines before the first header are in section "".
fn sections(text: &str) -> HashMap<&str, Keys<'_>> {
    let mut out: HashMap<&str, Keys> = HashMap::new();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s;
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.entry(section).or_default().set(k.trim(), v.trim());
        }
    }
    out
}

/// Parse a theme directory's `index.theme` into `Inherits` and directory entries, each path as
/// the index names it (relative to `theme_dir`); missing or malformed files yield nothing.
pub(super) fn parse_index(theme_dir: &Path) -> (Vec<String>, Vec<DirEntry>) {
    let Ok(text) = std::fs::read_to_string(theme_dir.join("index.theme")) else {
        return (Vec::new(), Vec::new());
    };
    let sections = sections(&text);
    let theme = sections.get("Icon Theme");
    fn list(value: Option<&str>) -> impl Iterator<Item = String> + '_ {
        value
            .into_iter()
            .flat_map(|v| v.split(','))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    let inherits: Vec<String> = list(theme.and_then(|t| t.inherits)).collect();
    let mut names: Vec<String> = list(theme.and_then(|t| t.directories)).collect();
    names.extend(list(theme.and_then(|t| t.scaled_directories)));
    let mut dirs = Vec::with_capacity(names.len());
    for name in names {
        let keys = sections.get(name.as_str());
        let num = |v: Option<&str>| v.and_then(|v| v.parse::<i32>().ok());
        let Some(size) = num(keys.and_then(|k| k.size)) else {
            continue;
        };
        let scale = num(keys.and_then(|k| k.scale)).unwrap_or(1).max(1);
        let kind = match keys.and_then(|k| k.kind) {
            Some("Fixed") => DirKind::Fixed,
            Some("Scalable") => DirKind::Scalable,
            _ => DirKind::Threshold,
        };
        let threshold = num(keys.and_then(|k| k.threshold)).unwrap_or(2);
        let (min, max) = match kind {
            DirKind::Scalable => (
                num(keys.and_then(|k| k.min_size)).unwrap_or(size),
                num(keys.and_then(|k| k.max_size)).unwrap_or(size),
            ),
            DirKind::Threshold => (size - threshold, size + threshold),
            DirKind::Fixed => (size, size),
        };
        dirs.push(DirEntry {
            path: PathBuf::from(name),
            size,
            scale,
            kind,
            min,
            max,
            threshold,
        });
    }
    (inherits, dirs)
}
