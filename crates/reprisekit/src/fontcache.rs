//! The system font scan kept in `$XDG_CACHE_HOME/reprise/fonts`, read instead of every font
//! file until a font directory, a fontconfig file or the environment naming them changes.

use fontdb::{Database, FaceInfo, Family, Language, Source, Stretch, Style, Weight};
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"RPF1";

/// The system fonts as `Database::load_system_fonts` finds them, through the cache.
pub(crate) fn system_database() -> Database {
    let path = cache_path();
    if let Some(db) = path.as_deref().and_then(|p| load(p, &env_key())) {
        return db;
    }
    // Stamped before the scan: a font installed meanwhile makes the next start scan again.
    let stamp = Stamp::new(env_key(), &watched(&fontconfig()));
    let mut db = Database::new();
    db.load_system_fonts();
    if let Some(path) = path
        && let Err(e) = store(&path, &stamp, &db)
    {
        log::warn!("font cache {}: {e}", path.display());
    }
    db
}

fn cache_path() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(cache.join("reprise").join("fonts"))
}

/// The variables `load_system_fonts` reads to find its config.
fn env_key() -> String {
    ["FONTCONFIG_FILE", "XDG_CONFIG_HOME", "HOME"]
        .map(|k| std::env::var(k).unwrap_or_default())
        .join("\n")
}

/// The fontconfig config `load_system_fonts` reads, with the files it read.
fn fontconfig() -> fontconfig_parser::FontConfig {
    let mut config = fontconfig_parser::FontConfig::default();
    if let Some(file) = std::env::var_os("FONTCONFIG_FILE") {
        let _ = config.merge_config(Path::new(&file));
        return config;
    }
    if config_home().is_none_or(|c| {
        config
            .merge_config(&c.join("fontconfig/fonts.conf"))
            .is_err()
    }) {
        let _ = config.merge_config(Path::new("/etc/fonts/local.conf"));
    }
    let _ = config.merge_config(Path::new("/etc/fonts/fonts.conf"));
    config
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// What a scan depends on: the font directories `config` names (fontdb's fixed list without
/// any), the config files it read, their directories and the conf.d directories.
fn watched(config: &fontconfig_parser::FontConfig) -> Watched {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = config
        .dirs
        .iter()
        .filter_map(|d| match d.path.strip_prefix("~") {
            Ok(rest) => Some(home.as_ref()?.join(rest)),
            Err(_) => Some(d.path.clone()),
        })
        .collect();
    if roots.is_empty() {
        roots = ["/usr/share/fonts", "/usr/local/share/fonts"]
            .map(PathBuf::from)
            .into();
        roots.extend(
            home.iter()
                .flat_map(|h| [h.join(".fonts"), h.join(".local/share/fonts")]),
        );
    }
    let mut files: Vec<PathBuf> = config.config_files.iter().cloned().collect();
    files.sort();
    let parents = files
        .iter()
        .filter_map(|f| f.parent().map(Path::to_path_buf));
    let mut others: Vec<PathBuf> = ["/etc/fonts", "/etc/fonts/conf.d"]
        .map(PathBuf::from)
        .into();
    others.extend(
        config_home()
            .iter()
            .flat_map(|c| [c.join("fontconfig"), c.join("fontconfig/conf.d")]),
    );
    others.extend(parents);
    others.extend(files);
    Watched { roots, others }
}

struct Watched {
    /// Walked for every directory below.
    roots: Vec<PathBuf>,
    others: Vec<PathBuf>,
}

type Mtime = Option<(i64, i64)>;

fn mtime(path: &Path) -> Mtime {
    let m = std::fs::metadata(path).ok()?;
    Some((m.mtime(), m.mtime_nsec()))
}

/// The modification times a scan was made at; a missing path is stamped as missing.
#[derive(Debug, PartialEq)]
struct Stamp {
    env: String,
    paths: Vec<(PathBuf, Mtime)>,
}

impl Stamp {
    fn new(env: String, watched: &Watched) -> Stamp {
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for root in &watched.roots {
            if seen.insert(std::fs::canonicalize(root).unwrap_or_else(|_| root.clone())) {
                paths.push((root.clone(), mtime(root)));
                walk(root, &mut seen, &mut paths);
            }
        }
        for p in &watched.others {
            if seen.insert(p.clone()) {
                paths.push((p.clone(), mtime(p)));
            }
        }
        Stamp { env, paths }
    }

    fn holds(&self, env: &str) -> bool {
        self.env == env && self.paths.iter().all(|(p, t)| mtime(p) == *t)
    }
}

/// Every directory below `dir`, symlinked ones followed once each, as fontdb walks them.
fn walk(dir: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<(PathBuf, Mtime)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = match entry.file_type() {
            Ok(t) if t.is_symlink() => std::fs::metadata(&path).is_ok_and(|m| m.is_dir()),
            Ok(t) => t.is_dir(),
            Err(_) => false,
        };
        if !is_dir {
            continue;
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if seen.insert(path.clone()) {
            out.push((path.clone(), mtime(&path)));
            walk(&path, seen, out);
        }
    }
}

const GENERIC: [Family<'static>; 5] = [
    Family::Serif,
    Family::SansSerif,
    Family::Cursive,
    Family::Fantasy,
    Family::Monospace,
];

/// The database `path` holds when its stamp still holds under `env`.
fn load(path: &Path, env: &str) -> Option<Database> {
    let bytes = std::fs::read(path).ok()?;
    let mut r = Reader(&bytes);
    if r.take(4)? != MAGIC {
        return None;
    }
    let env_was = r.str()?;
    let mut paths = Vec::new();
    for _ in 0..r.u32()? {
        let p = PathBuf::from(r.str()?);
        let t = match r.u8()? {
            0 => None,
            _ => Some((r.u64()? as i64, r.u64()? as i64)),
        };
        paths.push((p, t));
    }
    if !(Stamp {
        env: env_was,
        paths,
    })
    .holds(env)
    {
        return None;
    }
    let mut db = Database::new();
    let generic: Vec<String> = GENERIC.iter().map(|_| r.str()).collect::<Option<_>>()?;
    db.set_serif_family(&generic[0]);
    db.set_sans_serif_family(&generic[1]);
    db.set_cursive_family(&generic[2]);
    db.set_fantasy_family(&generic[3]);
    db.set_monospace_family(&generic[4]);
    for _ in 0..r.u32()? {
        let source = Source::File(PathBuf::from(r.str()?));
        let index = r.u32()?;
        let families = (0..r.u32()?)
            .map(|_| Some((r.str()?, Language::English_UnitedStates)))
            .collect::<Option<_>>()?;
        let post_script_name = r.str()?;
        let style = match r.u8()? {
            0 => Style::Normal,
            1 => Style::Italic,
            _ => Style::Oblique,
        };
        let weight = Weight(r.u16()?);
        let stretch = stretch(r.u16()?)?;
        let monospaced = r.u8()? != 0;
        db.push_face_info(FaceInfo {
            id: fontdb::ID::dummy(),
            source,
            index,
            families,
            post_script_name,
            style,
            weight,
            stretch,
            monospaced,
        });
    }
    r.0.is_empty().then_some(db)
}

fn stretch(n: u16) -> Option<Stretch> {
    use Stretch::*;
    [
        UltraCondensed,
        ExtraCondensed,
        Condensed,
        SemiCondensed,
        Normal,
        SemiExpanded,
        Expanded,
        ExtraExpanded,
        UltraExpanded,
    ]
    .into_iter()
    .find(|s| s.to_number() == n)
}

/// Writes `db`'s file faces under `stamp`; family languages are not kept.
fn store(path: &Path, stamp: &Stamp, db: &Database) -> std::io::Result<()> {
    let mut w = Vec::with_capacity(128 << 10);
    w.extend_from_slice(MAGIC);
    put_str(&mut w, &stamp.env);
    put_u32(&mut w, stamp.paths.len());
    for (p, t) in &stamp.paths {
        put_str(&mut w, &p.to_string_lossy());
        match t {
            None => w.push(0),
            Some((s, ns)) => {
                w.push(1);
                w.extend_from_slice(&(*s as u64).to_le_bytes());
                w.extend_from_slice(&(*ns as u64).to_le_bytes());
            }
        }
    }
    for f in &GENERIC {
        put_str(&mut w, db.family_name(f));
    }
    let faces: Vec<(&FaceInfo, &Path)> = db
        .faces()
        .filter_map(|f| match &f.source {
            Source::File(p) | Source::SharedFile(p, _) => Some((f, p.as_path())),
            Source::Binary(_) => None,
        })
        .collect();
    put_u32(&mut w, faces.len());
    for (f, p) in faces {
        put_str(&mut w, &p.to_string_lossy());
        put_u32(&mut w, f.index as usize);
        put_u32(&mut w, f.families.len());
        for (name, _) in &f.families {
            put_str(&mut w, name);
        }
        put_str(&mut w, &f.post_script_name);
        w.push(match f.style {
            Style::Normal => 0,
            Style::Italic => 1,
            Style::Oblique => 2,
        });
        w.extend_from_slice(&f.weight.0.to_le_bytes());
        w.extend_from_slice(&f.stretch.to_number().to_le_bytes());
        w.push(f.monospaced.into());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".fonts-{}", std::process::id()));
    std::fs::write(&tmp, &w)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn put_u32(w: &mut Vec<u8>, n: usize) {
    w.extend_from_slice(&(n as u32).to_le_bytes());
}

fn put_str(w: &mut Vec<u8>, s: &str) {
    put_u32(w, s.len());
    w.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-fontcache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fonts/sub")).unwrap();
        dir
    }

    fn face(path: &str, index: u32, families: &[&str], style: Style, weight: u16) -> FaceInfo {
        FaceInfo {
            id: fontdb::ID::dummy(),
            source: Source::File(path.into()),
            index,
            families: (families.iter())
                .map(|f| (f.to_string(), Language::English_UnitedStates))
                .collect(),
            post_script_name: format!("{}-{index}", families[0]),
            style,
            weight: Weight(weight),
            stretch: Stretch::SemiCondensed,
            monospaced: index == 1,
        }
    }

    type Row = (PathBuf, u32, Vec<String>, String, Style, u16, Stretch, bool);

    fn rows(db: &Database) -> Vec<Row> {
        db.faces()
            .map(|f| {
                let Source::File(p) = &f.source else {
                    panic!("faces load as files")
                };
                let names = f.families.iter().map(|(n, _)| n.clone()).collect();
                let ps = f.post_script_name.clone();
                (
                    p.clone(),
                    f.index,
                    names,
                    ps,
                    f.style,
                    f.weight.0,
                    f.stretch,
                    f.monospaced,
                )
            })
            .collect()
    }

    fn stored(dir: &Path) -> (PathBuf, Database) {
        let mut db = Database::new();
        db.push_face_info(face(
            "/srv/fonts/a.ttf",
            0,
            &["Alpha", "Alpha Text"],
            Style::Normal,
            400,
        ));
        db.push_face_info(face("/srv/fonts/b.ttc", 1, &["Beta"], Style::Oblique, 700));
        db.set_sans_serif_family("Alpha");
        let watched = Watched {
            roots: vec![dir.join("fonts"), dir.join("missing")],
            others: vec![dir.join("fonts.conf")],
        };
        let path = dir.join("cache/fonts");
        store(&path, &Stamp::new("env".into(), &watched), &db).unwrap();
        (path, db)
    }

    #[test]
    fn a_stored_scan_loads_back_face_for_face() {
        let dir = scratch("roundtrip");
        let (path, db) = stored(&dir);
        let back = load(&path, "env").expect("an unchanged stamp holds");
        assert_eq!(rows(&back), rows(&db));
        for f in &GENERIC {
            assert_eq!(back.family_name(f), db.family_name(f));
        }
        assert_eq!(back.family_name(&Family::SansSerif), "Alpha");
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(load(&path, "env").is_none(), "a cut file is scanned again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_cache_lapses_when_a_font_folder_or_config_changes() {
        let dir = scratch("lapse");
        type Change = (&'static str, fn(&Path));
        let changes: [Change; 5] = [
            ("a font added in a subfolder", |d| {
                std::fs::write(d.join("fonts/sub/c.ttf"), b"").unwrap()
            }),
            ("a new subfolder", |d| {
                std::fs::create_dir(d.join("fonts/new")).unwrap()
            }),
            ("a font folder that was missing", |d| {
                std::fs::create_dir(d.join("missing")).unwrap()
            }),
            ("a config file written", |d| {
                std::fs::write(d.join("fonts.conf"), b"<fontconfig/>").unwrap()
            }),
            ("a subfolder removed", |d| {
                std::fs::remove_dir(d.join("fonts/sub")).unwrap()
            }),
        ];
        for (what, change) in changes {
            let (path, _) = stored(&dir);
            assert!(load(&path, "env").is_some(), "holds before {what}");
            change(&dir);
            assert!(load(&path, "env").is_none(), "lapses after {what}");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("fonts/sub")).unwrap();
        }
        let (path, _) = stored(&dir);
        assert!(
            load(&path, "other env").is_none(),
            "lapses under another environment"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
