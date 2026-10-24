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

fn ini_lines(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((section.clone(), k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Parse a theme directory's `index.theme` into `Inherits` and directory entries; missing or
/// malformed files yield nothing.
pub(super) fn parse_index(theme_dir: &Path) -> (Vec<String>, Vec<DirEntry>) {
    let Ok(text) = std::fs::read_to_string(theme_dir.join("index.theme")) else {
        return (Vec::new(), Vec::new());
    };
    let lines = ini_lines(&text);
    // The first of a repeated key wins.
    let mut values: HashMap<(&str, &str), &str> = HashMap::with_capacity(lines.len());
    for (s, k, v) in &lines {
        values.entry((s.as_str(), k.as_str())).or_insert(v.as_str());
    }
    let get = |section: &str, key: &str| values.get(&(section, key)).copied();
    let inherits: Vec<String> = get("Icon Theme", "Inherits")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut names: Vec<String> = get("Icon Theme", "Directories")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if let Some(scaled) = get("Icon Theme", "ScaledDirectories") {
        names.extend(
            scaled
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
    }
    let mut dirs = Vec::with_capacity(names.len());
    for name in names {
        let num = |key: &str| get(&name, key).and_then(|v| v.parse::<i32>().ok());
        let Some(size) = num("Size") else {
            continue;
        };
        let scale = num("Scale").unwrap_or(1).max(1);
        let kind = match get(&name, "Type") {
            Some("Fixed") => DirKind::Fixed,
            Some("Scalable") => DirKind::Scalable,
            _ => DirKind::Threshold,
        };
        let threshold = num("Threshold").unwrap_or(2);
        let (min, max) = match kind {
            DirKind::Scalable => (
                num("MinSize").unwrap_or(size),
                num("MaxSize").unwrap_or(size),
            ),
            DirKind::Threshold => (size - threshold, size + threshold),
            DirKind::Fixed => (size, size),
        };
        dirs.push(DirEntry {
            path: theme_dir.join(&name),
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
