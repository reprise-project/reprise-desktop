use super::{LoadedFace, TextRenderer};
use crate::mutex::lock;
use crate::theme::FontConfig;
use fontdb::{Database, Family, Query, Weight};
use freetype::{Face, Library};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

fn find_face(
    db: &Database,
    families: &[String],
    generic: bool,
    weight: Weight,
    style: fontdb::Style,
) -> Option<(PathBuf, u32, String, fontdb::Style)> {
    let mut fams: Vec<Family> = families.iter().map(|f| Family::Name(f.as_str())).collect();
    if generic {
        fams.push(Family::SansSerif);
    }
    let id = db.query(&Query {
        families: &fams,
        weight,
        style,
        ..Query::default()
    })?;
    let (source, index) = db.face_source(id)?;
    let path = match source {
        fontdb::Source::File(path) => path,
        fontdb::Source::SharedFile(path, _) => path,
        fontdb::Source::Binary(_) => return None,
    };
    let info = db.face(id)?;
    let family = info
        .families
        .first()
        .map(|(name, _)| name.clone())
        .unwrap_or_default();
    Some((path, index, family, info.style))
}

fn load_face(
    lib: &Library,
    db: &Database,
    families: &[String],
    generic: bool,
    weight: Weight,
    style: fontdb::Style,
) -> Option<LoadedFace> {
    let (path, index, family, matched_style) = find_face(db, families, generic, weight, style)?;
    match lib.new_face(&path, index as isize) {
        Ok(face) => {
            log::info!(
                "font {families:?} weight {weight:?} style {style:?}: {family} ({})",
                path.display()
            );
            Some(LoadedFace {
                face,
                family,
                italic: matched_style != fontdb::Style::Normal,
            })
        }
        Err(e) => {
            log::warn!("FreeType could not open {}: {e}", path.display());
            None
        }
    }
}

/// The (regular, bold) faces of `families`; the regular face again as bold when there is none.
pub(super) fn load_faces(
    lib: &Library,
    db: &Database,
    families: &[String],
    generic: bool,
    style: fontdb::Style,
) -> (Option<LoadedFace>, Option<LoadedFace>) {
    let regular = load_face(lib, db, families, generic, Weight::NORMAL, style);
    let bold = load_face(lib, db, families, generic, Weight::BOLD, style)
        .or_else(|| load_face(lib, db, families, generic, Weight::NORMAL, style));
    (regular, bold)
}

/// Switch `lib`'s TrueType hinter to full mode (stems fit across as well as down in antialiased
/// text); per-renderer library.
pub(super) fn full_hinting(lib: &Library) {
    let version: u32 = 35;
    // SAFETY: `lib.raw()` is a live library for the call; the module and
    // property names are NUL-terminated literals, and "interpreter-version"
    // takes a pointer to an FT_UInt that FreeType only reads.
    let err = unsafe {
        freetype::ffi::FT_Property_Set(
            lib.raw(),
            c"truetype".as_ptr(),
            c"interpreter-version".as_ptr(),
            (&raw const version).cast(),
        )
    };
    if err != 0 {
        log::warn!("FreeType refused full hinting (error {err}); titles stay lightly hinted");
    }
}

/// The system font database, shared by every renderer and cached on disk ([`crate::fontcache`]);
/// read again when it lacks the preferred family.
pub(crate) fn system_fonts(families: &[String]) -> Arc<Database> {
    static FONTS: Mutex<Option<Arc<Database>>> = Mutex::new(None);
    let mut fonts = lock(&FONTS);
    if let Some(db) = fonts.as_ref().filter(|db| !needs_scan(db, families)) {
        return db.clone();
    }
    let mut db = crate::fontcache::system_database();
    if let Some(dir) = test_fonts_dir() {
        load_test_fonts(&mut db, &dir);
    }
    fonts.insert(Arc::new(db)).clone()
}

/// The directory `REPRISE_TEST_FONTS` names (filled by `scripts/test-fonts.sh`), whose faces
/// replace the system's faces of the same family so pixel tests render alike on every machine.
pub(super) fn test_fonts_dir() -> Option<PathBuf> {
    std::env::var_os("REPRISE_TEST_FONTS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn load_test_fonts(db: &mut Database, dir: &std::path::Path) {
    let mut test = Database::new();
    test.load_fonts_dir(dir);
    let families: Vec<String> = test
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let shadowed: Vec<fontdb::ID> = db
        .faces()
        .filter(|f| f.families.iter().any(|(name, _)| families.contains(name)))
        .map(|f| f.id)
        .collect();
    for id in shadowed {
        db.remove_face(id);
    }
    db.load_fonts_dir(dir);
}

pub(super) fn needs_scan(db: &Database, families: &[String]) -> bool {
    families.first().is_some_and(|want| {
        !db.faces()
            .any(|f| f.families.iter().any(|(name, _)| name == want))
    })
}

/// Whether the default UI face is the Tahoma `REPRISE_TEST_FONTS` supplies, which the pixel tests
/// are pinned to; they skip without it. Cached.
pub fn test_tahoma() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let Some(dir) = test_fonts_dir() else {
            return false;
        };
        let families = FontConfig::default().family;
        let db = system_fonts(&families);
        let dir = dir.canonicalize().unwrap_or(dir);
        families.first().is_some_and(|f| f == "Tahoma")
            && find_face(&db, &families, false, Weight::NORMAL, fontdb::Style::Normal)
                .is_some_and(|(path, _, family, _)| family == "Tahoma" && path.starts_with(&dir))
    })
}

/// The second dialog font `spec` names, a family (exact match) or an absolute font file path;
/// `None` with a warning if not found.
pub(super) fn load_shell_dlg(spec: &str, cfg: &FontConfig) -> Option<TextRenderer> {
    let lib = Library::init().ok()?;
    if cfg.full_hinting {
        full_hinting(&lib);
    }
    let (regular, bold) = if spec.starts_with('/') {
        match lib.new_face(spec, 0) {
            Ok(face) => {
                log::info!("second dialog font: {spec}");
                select_any_charmap(&face);
                let loaded = |face: Face| LoadedFace {
                    family: face.family_name().unwrap_or_default(),
                    italic: false,
                    face,
                };
                (Some(loaded(face.clone())), Some(loaded(face)))
            }
            Err(e) => {
                log::warn!("second dialog font {spec}: {e}; dialogs keep the UI font");
                return None;
            }
        }
    } else {
        let families = [spec.to_string()];
        let db = system_fonts(&families);
        load_faces(&lib, &db, &families, false, fontdb::Style::Normal)
    };
    if regular.is_none() {
        log::warn!("second dialog font {spec:?} not found; dialogs keep the UI font");
        return None;
    }
    Some(TextRenderer::with_faces(regular, bold, cfg.antialias))
}

/// Select `face`'s first charmap when FreeType picked none, as for a `.fon` bitmap font indexed by
/// code page; ASCII agrees, anything else may fall to notdef.
fn select_any_charmap(face: &Face) {
    if face.raw().charmap.is_null()
        && face.num_charmaps() > 0
        && let Err(e) = face.set_charmap(&face.get_charmap(0))
    {
        log::warn!("second dialog font: no character map selected ({e})");
    }
}
