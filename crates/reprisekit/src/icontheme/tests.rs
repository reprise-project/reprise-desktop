use super::cache::{HIT_CAP, MISS_CAP};
use super::decode::{
    SVG_MAX_BYTES, SVG_MAX_LAYERS, SVG_MAX_NODES, SVG_MAX_PRIMITIVES, svg_over_budget,
};
use super::*;
use crate::raster::Canvas;
use std::fs;

/// At 200% text an icon the theme had only at 16 px for a 32 px slot draws at 32 px, a 48 px
/// one for a 64 px slot at 64 px; at 100%, or when it already fits, it is kept.
#[test]
fn a_smaller_icon_grows_with_the_text_up_to_its_slot() {
    use crate::theme::TextUnits;
    let two = TextUnits::new(22, 11);
    let side = |w: u32, want: i32, u: TextUnits| grown_icon(Canvas::new(w, w), want, u).width;
    assert_eq!(side(16, 32, two), 32);
    assert_eq!(side(48, 64, two), 64);
    assert_eq!(side(64, 64, two), 64);
    assert_eq!(side(16, 32, TextUnits::ONE), 16);
}

/// A fresh empty directory, a different one on every call so parallel tests never share
/// one; the caller removes it.
fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("reprise-icons-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A throwaway icon root with one theme; the caller removes it.
fn fixture(theme: &str, index: &str, files: &[&str]) -> PathBuf {
    let root = scratch();
    let dir = root.join(theme);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("index.theme"), index).unwrap();
    for f in files {
        let p = dir.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"").unwrap();
    }
    root
}

const INDEX: &str = "[Icon Theme]\nName=Test\nInherits=hicolor\nDirectories=16x16/apps,32x32/apps,scalable/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\nContext=Applications\n\n[32x32/apps]\nSize=32\nType=Fixed\n\n[scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n";

/// A lookup lists each directory once and probes the listing after: a file that appears
/// later is not seen until reload (the cache remembers misses anyway).
#[test]
fn lookups_probe_cached_listings() {
    let root = fixture("test", INDEX, &["16x16/apps/present.png"]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert!(t.lookup("absent", 16, 1).is_none());
    let listed = || t.listings.iter().filter(|l| l.get().is_some()).count();
    assert_eq!(listed(), t.dirs.len() + t.roots.len(), "every dir and root");
    assert_eq!(
        t.lookup("present", 16, 1),
        Some(root.join("test/16x16/apps/present.png"))
    );
    fs::write(root.join("test/16x16/apps/late.png"), b"").unwrap();
    assert!(
        t.lookup("late", 16, 1).is_none(),
        "listed before it existed"
    );
    let extra = root.join("extra");
    fs::create_dir_all(&extra).unwrap();
    assert!(
        t.lookup_with_extra("fresh", 16, 1, std::slice::from_ref(&extra))
            .is_none()
    );
    fs::write(extra.join("fresh.png"), b"").unwrap();
    assert_eq!(
        t.lookup_with_extra("fresh", 16, 1, std::slice::from_ref(&extra)),
        Some(extra.join("fresh.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

/// A listing names every entry but a directory and never stats one: a symlink is taken by
/// name even when its target is gone.
#[test]
fn listing_skips_directories_without_following_symlinks() {
    let root = fixture("test", INDEX, &["16x16/apps/plain.png"]);
    let dir = root.join("test/16x16/apps");
    std::os::unix::fs::symlink("plain.png", dir.join("link.png")).unwrap();
    std::os::unix::fs::symlink("gone.png", dir.join("dangling.png")).unwrap();
    fs::create_dir(dir.join("sub.png")).unwrap();
    let mut names: Vec<OsString> = list_files(&dir).into_iter().collect();
    names.sort();
    assert_eq!(names, ["dangling.png", "link.png", "plain.png"]);
    fs::remove_dir_all(&root).unwrap();
}

/// A root without the theme's `index.theme` still serves the directories another root's
/// index names, ahead of later roots.
#[test]
fn a_root_without_the_index_serves_the_themes_directories() {
    let system = fixture("test", INDEX, &["16x16/apps/both.png"]);
    let user = scratch();
    for f in ["test/16x16/apps/mine.png", "test/16x16/apps/both.png"] {
        let p = user.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"").unwrap();
    }
    let t = IconTheme::load("test", &[user.clone(), system.clone()]);
    assert_eq!(
        t.lookup("mine", 16, 1),
        Some(user.join("test/16x16/apps/mine.png"))
    );
    assert_eq!(
        t.lookup("both", 16, 1),
        Some(user.join("test/16x16/apps/both.png"))
    );
    fs::remove_dir_all(&user).unwrap();
    fs::remove_dir_all(&system).unwrap();
}

#[test]
fn a_repeated_key_keeps_its_first_value_and_a_repeated_directory_counts_once() {
    let index = "[Icon Theme]\nDirectories=16x16/apps,16x16/apps\n\n[16x16/apps]\nSize=16\nSize=32\nType=Fixed\n";
    let root = fixture("test", index, &[]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    let test: Vec<_> = t.dirs[t.groups[0].clone()].iter().map(|d| d.size).collect();
    assert_eq!(test, [16]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn parses_directories_and_inherits() {
    let root = fixture("test", INDEX, &[]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(t.chain, vec!["test".to_string(), "hicolor".to_string()]);
    assert_eq!(t.dirs.len(), 3);
    let d = &t.dirs[0];
    assert_eq!(d.size, 16);
    assert_eq!(d.scale, 1);
    assert_eq!(d.kind, DirKind::Fixed);
    let s = &t.dirs[2];
    assert_eq!(s.kind, DirKind::Scalable);
    assert_eq!((s.min, s.max), (8, 512));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn exact_size_beats_closest_size() {
    let root = fixture(
        "test",
        INDEX,
        &["16x16/apps/firefox.png", "32x32/apps/firefox.png"],
    );
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("firefox", 16, 1),
        Some(root.join("test/16x16/apps/firefox.png"))
    );
    assert_eq!(
        t.lookup("firefox", 32, 1),
        Some(root.join("test/32x32/apps/firefox.png"))
    );
    // Both the 16 and 32 directories are at distance 8; 32x32 wins because it scales down.
    assert_eq!(
        t.lookup("firefox", 24, 1),
        Some(root.join("test/32x32/apps/firefox.png"))
    );
    // Each size keeps its own search order once the others have been asked.
    assert_eq!(
        t.lookup("firefox", 16, 1),
        Some(root.join("test/16x16/apps/firefox.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_far_bigger_icon_beats_a_near_smaller_one() {
    let root = fixture(
        "test",
        "[Icon Theme]\nName=Test\nDirectories=16x16/apps,128x128/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[128x128/apps]\nSize=128\nType=Fixed\n",
        &["16x16/apps/net.png", "128x128/apps/net.png"],
    );
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("net", 32, 1),
        Some(root.join("test/128x128/apps/net.png"))
    );
    // Nothing bigger: the nearest smaller one still serves.
    assert_eq!(
        t.lookup("net", 200, 1),
        Some(root.join("test/128x128/apps/net.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn scalable_dir_matches_any_size_in_range() {
    let root = fixture("test", INDEX, &["scalable/apps/gimp.svg"]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("gimp", 16, 1),
        Some(root.join("test/scalable/apps/gimp.svg"))
    );
    assert_eq!(
        t.lookup("gimp", 512, 1),
        Some(root.join("test/scalable/apps/gimp.svg"))
    );
    assert_eq!(t.lookup("nope", 16, 1), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn png_beats_svg_in_the_same_directory() {
    let root = fixture("test", INDEX, &["16x16/apps/a.svg", "16x16/apps/a.png"]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("a", 16, 1),
        Some(root.join("test/16x16/apps/a.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn scale_two_prefers_scaled_directory() {
    let index = "[Icon Theme]\nName=T\nDirectories=16x16/apps,16x16@2/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[16x16@2/apps]\nSize=16\nScale=2\nType=Fixed\n";
    let root = fixture("test", index, &["16x16/apps/x.png", "16x16@2/apps/x.png"]);
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("x", 16, 2),
        Some(root.join("test/16x16@2/apps/x.png"))
    );
    assert_eq!(
        t.lookup("x", 16, 1),
        Some(root.join("test/16x16/apps/x.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn themed_larger_icon_beats_inherited_exact_size() {
    // "test" has only a 22x22 dir but inherits "hicolor" with an exact 16x16 dir: the pass
    // finishes in "test" first, so 22x22 wins.
    let root = scratch();
    let test_index = "[Icon Theme]\nName=Test\nInherits=hicolor\nDirectories=22x22/apps\n\n[22x22/apps]\nSize=22\nType=Fixed\n";
    let hicolor_index =
        "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
    for (theme, index, file) in [
        ("test", test_index, "22x22/apps/foo.png"),
        ("hicolor", hicolor_index, "16x16/apps/foo.png"),
    ] {
        let dir = root.join(theme);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.theme"), index).unwrap();
        let p = dir.join(file);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"").unwrap();
    }
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup("foo", 16, 1),
        Some(root.join("test/22x22/apps/foo.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn fallback_theme_inherits_are_followed() {
    // The configured theme is a bare "hicolor"; "Adwaita" only inherits from "Legacy", so
    // the fallback pass must walk that Inherits line.
    let root = scratch();
    let hicolor_index =
        "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
    let adwaita_index = "[Icon Theme]\nName=Adwaita\nInherits=Legacy,hicolor\nDirectories=symbolic/apps\n\n[symbolic/apps]\nSize=16\nType=Fixed\n";
    let legacy_index = "[Icon Theme]\nName=Legacy\nInherits=hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
    for (theme, index, file) in [
        ("hicolor", hicolor_index, None),
        ("Adwaita", adwaita_index, None),
        ("Legacy", legacy_index, Some("16x16/apps/system-run.png")),
    ] {
        let dir = root.join(theme);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.theme"), index).unwrap();
        if let Some(file) = file {
            let p = dir.join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
    }
    let t = IconTheme::load("hicolor", std::slice::from_ref(&root));
    assert_eq!(
        t.chain,
        vec![
            "hicolor".to_string(),
            "Adwaita".to_string(),
            "Legacy".to_string()
        ]
    );
    assert_eq!(
        t.lookup("system-run", 16, 1),
        Some(root.join("Legacy/16x16/apps/system-run.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn falls_back_to_bare_file_in_root() {
    let root = fixture("test", INDEX, &[]);
    fs::write(root.join("bare.png"), b"").unwrap();
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(t.lookup("bare", 16, 1), Some(root.join("bare.png")));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn extra_roots_are_consulted_first() {
    let root = fixture("test", INDEX, &["16x16/apps/tray.png"]);
    let extra = root.join("extra");
    fs::create_dir_all(&extra).unwrap();
    fs::write(extra.join("tray.png"), b"").unwrap();
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        t.lookup_with_extra("tray", 16, 1, std::slice::from_ref(&extra)),
        Some(extra.join("tray.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_theme_still_has_hicolor_in_chain() {
    let root = scratch();
    let t = IconTheme::load("does-not-exist", &[root]);
    assert_eq!(
        t.chain,
        vec!["does-not-exist".to_string(), "hicolor".to_string()]
    );
    assert!(t.dirs.is_empty());
}

#[test]
fn size_matching_rules() {
    let fixed = DirEntry {
        path: PathBuf::new(),
        size: 16,
        scale: 1,
        kind: DirKind::Fixed,
        min: 16,
        max: 16,
        threshold: 2,
    };
    assert!(fixed.matches(16, 1));
    assert!(!fixed.matches(17, 1));
    assert!(!fixed.matches(16, 2));
    assert_eq!(fixed.distance(20, 1), 4);
    let thr = DirEntry {
        kind: DirKind::Threshold,
        ..fixed.clone()
    };
    assert!(thr.matches(18, 1));
    assert!(!thr.matches(19, 1));
    assert_eq!(thr.distance(19, 1), 1);
    let scalable = DirEntry {
        kind: DirKind::Scalable,
        min: 8,
        max: 64,
        ..fixed.clone()
    };
    assert!(scalable.matches(8, 1));
    assert!(scalable.matches(64, 1));
    assert!(!scalable.matches(65, 1));
    assert_eq!(scalable.distance(4, 1), 4);
    assert_eq!(scalable.distance(100, 1), 36);
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let dir = scratch();
    dir.join(name)
}

#[test]
fn decodes_png_and_premultiplies() {
    let p = tmp("two.png");
    // Opaque red, half-transparent green, transparent, opaque blue.
    write_png(
        &p,
        2,
        2,
        &[255, 0, 0, 255, 0, 255, 0, 128, 9, 9, 9, 0, 0, 0, 255, 255],
    );
    let c = decode(&p, 2).unwrap();
    assert_eq!((c.width, c.height), (2, 2));
    assert_eq!(c.get(0, 0), 0xFFFF_0000);
    assert_eq!(c.get(1, 0), 0x8000_8000);
    assert_eq!(c.get(0, 1), 0x0000_0000);
    assert_eq!(c.get(1, 1), 0xFF00_00FF);
    fs::remove_file(&p).unwrap();
}

#[test]
fn decodes_png_and_resamples_to_requested_size() {
    let p = tmp("four.png");
    let mut rgba = Vec::new();
    for _ in 0..16 {
        rgba.extend_from_slice(&[0, 0, 255, 255]);
    }
    write_png(&p, 4, 4, &rgba);
    let c = decode(&p, 2).unwrap();
    assert_eq!((c.width, c.height), (2, 2));
    assert_eq!(c.get(1, 1), 0xFF00_00FF);
    fs::remove_file(&p).unwrap();
}

#[test]
fn downscaling_crops_to_the_ink_first() {
    // 8x8 with a 4x4 opaque blue square in the middle: at 4 px the
    // square fills the whole icon instead of its centre 2x2.
    let p = tmp("ink-padded.png");
    let mut rgba = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let ink = (2..6).contains(&x) && (2..6).contains(&y);
            rgba.extend_from_slice(if ink {
                &[0, 0, 255, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    write_png(&p, 8, 8, &rgba);
    let c = decode(&p, 4).unwrap();
    assert_eq!((c.width, c.height), (4, 4));
    assert!((0..4).all(|y| (0..4).all(|x| c.get(x, y) == 0xFF00_00FF)));
    fs::remove_file(&p).unwrap();

    // A wide ink (8x2 at the top of 8x8) keeps its shape: 4x1, centred.
    let p = tmp("ink-wide.png");
    let mut rgba = Vec::new();
    for y in 0..8 {
        for _ in 0..8 {
            rgba.extend_from_slice(if y < 2 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    write_png(&p, 8, 8, &rgba);
    let c = decode(&p, 4).unwrap();
    let red_rows: Vec<i32> = (0..4).filter(|&y| c.get(0, y) == 0xFFFF_0000).collect();
    assert_eq!(red_rows, vec![1]);
    assert_eq!(c.get(3, 1), 0xFFFF_0000);
    fs::remove_file(&p).unwrap();
}

#[test]
fn decodes_svg_at_requested_size() {
    let p = tmp("rect.svg");
    fs::write(
        &p,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##,
    )
    .unwrap();
    let c = decode(&p, 4).unwrap();
    assert_eq!((c.width, c.height), (4, 4));
    assert_eq!(c.get(2, 2), 0xFFFF_0000);
    fs::remove_file(&p).unwrap();
}

/// Each renders in well under a millisecond at 48 px once its cost is cut, and takes
/// seconds without.
#[test]
fn an_svg_past_the_render_budget_is_refused() {
    let svg = |body: &str| {
        let text = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48">{body}</svg>"##
        );
        resvg::usvg::Tree::from_data(text.as_bytes(), &Default::default()).unwrap()
    };
    let blur = r##"<defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter></defs>"##;
    let shape = r##"<rect width="40" height="40" fill="#f00"/>"##;
    let icon = svg(&format!(
        r##"{blur}<g filter="url(#f)">{shape}</g><g opacity=".5">{shape}</g>"##
    ));
    assert_eq!(svg_over_budget(&icon), None);

    let wide = svg(&format!(
        r##"<defs><filter id="f"><feMorphology radius="40"/></filter></defs><g filter="url(#f)">{shape}</g>"##
    ));
    assert_eq!(
        svg_over_budget(&wide),
        Some("an unbounded filter primitive")
    );
    let layers = svg(&format!(r##"<g opacity=".5">{shape}</g>"##).repeat(SVG_MAX_LAYERS + 1));
    assert_eq!(svg_over_budget(&layers), Some("too many layers"));
    let blurs = svg(&format!(
        r##"{blur}{}"##,
        format!(r##"<g filter="url(#f)">{shape}</g>"##).repeat(SVG_MAX_PRIMITIVES + 1)
    ));
    assert_eq!(svg_over_budget(&blurs), Some("too many filter primitives"));
    // 0.01 units of dash on a 1000-unit line: 100 000 dashes.
    let dashes = svg(r##"<path d="M0 0H1000" stroke="#000" stroke-dasharray="0.005 0.005"/>"##);
    assert_eq!(svg_over_budget(&dashes), Some("too many path segments"));
    // A mask is drawn once per element using it.
    let masked = svg(&format!(
        r##"<defs><mask id="m">{}</mask></defs>{}"##,
        shape.repeat(100),
        r##"<rect mask="url(#m)" width="4" height="4"/>"##.repeat(SVG_MAX_NODES / 100)
    ));
    assert_eq!(svg_over_budget(&masked), Some("too many nodes"));

    // Past the byte cap, a file is refused before it is parsed.
    let p = tmp("padded.svg");
    let head = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16"/>"##;
    fs::write(&p, format!("{head}</svg>")).unwrap();
    assert!(decode(&p, 16).is_some());
    fs::write(&p, format!("{head}{}</svg>", " ".repeat(SVG_MAX_BYTES))).unwrap();
    assert!(decode(&p, 16).is_none());
    fs::remove_file(&p).unwrap();
}

/// An `<image href>` naming a file on disk is not followed: the SVG renders without it.
#[test]
fn an_svg_does_not_read_the_files_it_names() {
    let png = tmp("secret.png");
    write_png(&png, 2, 2, &[0, 0, 255, 255].repeat(4));
    let p = tmp("leak.svg");
    fs::write(
        &p,
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image xlink:href="{}" width="4" height="4"/></svg>"##,
            png.display()
        ),
    )
    .unwrap();
    let c = decode(&p, 4).unwrap();
    assert!(
        c.pixels.iter().all(|&px| px == 0),
        "the file's pixels leaked in"
    );
    fs::remove_file(&p).unwrap();
    fs::remove_file(&png).unwrap();
}

/// Icons a job loaded from the cache's theme answer `get` without a lookup; ones loaded
/// from a theme the cache no longer looks names up in are dropped.
#[test]
fn icons_put_in_from_the_caches_theme_answer_without_a_lookup() {
    let root = fixture("test", INDEX, &[]);
    let png = root.join("test/32x32/apps/app.png");
    fs::create_dir_all(png.parent().unwrap()).unwrap();
    write_png(&png, 1, 1, &[0, 0, 255, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let theme = cache.theme();
    let icon = load_icon(&theme, "app", 32, cache.scale());
    assert!(icon.is_some());
    let old = Arc::new(IconTheme::load("test", std::slice::from_ref(&root)));
    cache.put_icons(&old, vec![("app".into(), 32, None)]);
    cache.put_icons(&theme, vec![("app".into(), 32, icon.clone())]);
    fs::remove_file(&png).unwrap();
    assert_eq!(cache.get("app", 32), icon);
    fs::remove_dir_all(&root).unwrap();
}

/// An icon file decoded elsewhere answers `get` without the file being read again.
#[test]
fn a_file_put_in_answers_without_reading_the_file() {
    let root = fixture("test", INDEX, &[]);
    let pic = root.join("cover.png");
    write_png(&pic, 1, 1, &[255, 0, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let name = pic.to_str().unwrap();
    let decoded = decode_file(&pic, 16, cache.scale());
    assert!(decoded.is_some());
    cache.put_file(name, 16, decoded.clone());
    fs::remove_file(&pic).unwrap();
    assert_eq!(cache.get(name, 16), decoded);
    assert_eq!(
        cache.get(name, 32),
        None,
        "a size not put in reads the file"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn get_unread_never_reads_a_file_and_wants_each_once() {
    let mut cache = IconCache::new(IconTheme::load("none", &[]));
    let file = "/srv/icons/app.png";
    assert_eq!(cache.get_unread(file, 16), None);
    assert_eq!(cache.get_unread(file, 16), None);
    assert_eq!(cache.get_unread(file, 32), None);
    assert_eq!(
        cache.take_wanted(),
        [(file.to_string(), 16), (file.to_string(), 32)]
    );
    assert!(cache.take_wanted().is_empty());
    cache.put_file(file, 16, Some(Canvas::new(16, 16)));
    cache.put_file(file, 32, None);
    assert_eq!(
        cache.get_unread(file, 16).map(|c| c.map(|c| c.width)),
        Some(Some(16))
    );
    assert_eq!(
        cache.get_unread(file, 32),
        Some(None),
        "a failed decode is not tried again"
    );
    assert_eq!(cache.get_unread("no-such-name", 16), Some(None));
    assert!(cache.take_wanted().is_empty());
}

/// `file_put` answers only what was put in, never reading the file: a size not put in is
/// None though the file is there to decode.
#[test]
fn file_put_never_reads_the_file() {
    let root = fixture("test", INDEX, &[]);
    let pic = root.join("cover.png");
    write_png(&pic, 1, 1, &[255, 0, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let name = pic.to_str().unwrap();
    let decoded = decode_file(&pic, 16, cache.scale());
    cache.put_file(name, 16, decoded.clone());
    assert_eq!(
        cache.file_put(name, 16).flatten().as_deref(),
        decoded.as_ref()
    );
    assert_eq!(cache.file_put(name, 32), None);
    cache.put_file(name, 48, None);
    assert_eq!(
        cache.file_put(name, 48),
        Some(None),
        "a failed decode is kept"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_new_theme_keeps_the_icon_files_and_the_ones_still_wanted() {
    let mut cache = IconCache::new(IconTheme::load("none", &[]));
    cache.put_file("/srv/icons/kept.png", 16, Some(Canvas::new(16, 16)));
    assert_eq!(cache.get_unread("/srv/icons/late.png", 16), None);
    cache.get_shared("missing-name", 16);
    cache.set_theme(IconTheme::load("other", &[]));
    assert_eq!(cache.theme_name(), "other");
    assert!(cache.file_put("/srv/icons/kept.png", 16).is_some());
    assert_eq!(cache.len(), 1, "the theme lookups go");
    assert_eq!(
        cache.take_wanted(),
        [("/srv/icons/late.png".to_string(), 16)]
    );
}

/// A file put in and then dropped at the file cap is gone from `file_put`, and `file_drops`
/// says so; a failed decode put in is a miss and stays.
#[test]
fn a_file_dropped_at_the_file_cap_shows_in_file_drops() {
    let root = fixture("test", INDEX, &[]);
    let pic = root.join("cover.png");
    write_png(&pic, 1, 1, &[255, 0, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    cache.file_cap = 4;
    let name = pic.to_str().unwrap();
    cache.put_file(name, 16, decode_file(&pic, 16, cache.scale()));
    cache.put_file("/nowhere/broken.png", 16, None);
    let drops = cache.file_drops();
    for px in 17..20 {
        cache.put_file(name, px, decode_file(&pic, px, cache.scale()));
    }
    assert!(cache.file_put(name, 16).flatten().is_some());
    assert_eq!(cache.file_drops(), drops, "under the cap");
    cache.put_file(name, 20, decode_file(&pic, 20, cache.scale()));
    assert_eq!(cache.file_put(name, 16), None);
    assert_eq!(cache.file_drops(), drops + 1);
    assert_eq!(cache.file_put("/nowhere/broken.png", 16), Some(None));
    fs::remove_dir_all(&root).unwrap();
}

/// One read's files past the file cap all land, and theme lookups past the hit cap leave
/// them in place.
#[test]
fn a_read_past_the_file_cap_keeps_every_file() {
    let root = fixture("test", INDEX, &[]);
    let pic = root.join("cover.png");
    write_png(&pic, 1, 1, &[255, 0, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    cache.file_cap = 4;
    let name = pic.to_str().unwrap();
    let read: Vec<_> = (16..22)
        .map(|px| (name.to_string(), px, decode_file(&pic, px, cache.scale())))
        .collect();
    cache.put_files(read);
    assert!(
        (16..22).all(|px| cache.file_put(name, px).flatten().is_some()),
        "every file of the read is in"
    );
    let drops = cache.file_drops();
    let icon = Arc::new(decode_file(&pic, 16, 1).unwrap());
    for i in 0..=HIT_CAP {
        cache.insert((format!("app{i}"), 16, 1), Some(icon.clone()));
    }
    assert!((16..22).all(|px| cache.file_put(name, px).flatten().is_some()));
    assert_eq!(cache.file_drops(), drops);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn decode_rejects_missing_and_garbage() {
    assert!(decode(Path::new("/nonexistent/x.png"), 16).is_none());
    let p = tmp("garbage.png");
    fs::write(&p, b"not a png").unwrap();
    assert!(decode(&p, 16).is_none());
    let s = tmp("garbage.svg");
    fs::write(&s, b"<not xml").unwrap();
    assert!(decode(&s, 16).is_none());
    fs::remove_file(&p).unwrap();
    fs::remove_file(&s).unwrap();
}

#[test]
fn decode_picks_the_format_from_the_content() {
    let p = tmp("png-inside.svg");
    write_png(&p, 2, 2, &[0, 0, 255, 255].repeat(4));
    let c = decode(&p, 4).unwrap();
    assert_eq!(c.get(1, 1), 0xFF00_00FF);
    let s = tmp("svg-inside.png");
    fs::write(
        &s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##,
    )
    .unwrap();
    assert_eq!(decode(&s, 4).unwrap().get(1, 1), 0xFFFF_0000);
    fs::remove_file(&p).unwrap();
    fs::remove_file(&s).unwrap();
}

/// Fixed 24 and 48 directories plus a Scalable 16..512 one holding a PNG named `.svg`: a 16
/// px request is in the scalable range, and its file is scaled down to 16.
#[test]
fn a_size_only_the_scalable_range_covers_loads_its_file_scaled() {
    let index = "[Icon Theme]\nName=T\nDirectories=24x24/mimetypes,48x48/mimetypes,128x128/mimetypes\n\n[24x24/mimetypes]\nSize=24\nType=Fixed\n\n[48x48/mimetypes]\nSize=48\nType=Fixed\n\n[128x128/mimetypes]\nSize=128\nMinSize=16\nMaxSize=512\nType=Scalable\n";
    let root = fixture("test", index, &[]);
    let dir = root.join("test");
    for (sub, side, rgba) in [
        ("24x24/mimetypes/pic.png", 24, [255, 0, 0, 255]),
        ("48x48/mimetypes/pic.png", 48, [0, 255, 0, 255]),
        ("128x128/mimetypes/pic.svg", 128, [0, 0, 255, 255]),
    ] {
        let p = dir.join(sub);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_png(&p, side, side, &rgba.repeat((side * side) as usize));
    }
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    for (px, want) in [(16, 0xFF00_00FF), (24, 0xFFFF_0000), (48, 0xFF00_FF00)] {
        let c = cache
            .get("pic", px)
            .unwrap_or_else(|| panic!("pic at {px}"));
        assert_eq!(
            (c.width, c.get(px / 2, px / 2)),
            (px as u32, want),
            "{px} px"
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

/// The chain runs through the theme's own parents before `hicolor`, even when `hicolor` has
/// the exact size.
#[test]
fn inherited_themes_come_before_hicolor() {
    let root = scratch();
    let test_index = "[Icon Theme]\nName=Test\nInherits=parent\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
    let parent_index =
        "[Icon Theme]\nName=Parent\nDirectories=32x32/apps\n\n[32x32/apps]\nSize=32\nType=Fixed\n";
    let hicolor_index =
        "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n";
    for (theme, index, files) in [
        ("test", test_index, &[][..]),
        ("parent", parent_index, &["32x32/apps/both.png"][..]),
        (
            "hicolor",
            hicolor_index,
            &["16x16/apps/both.png", "16x16/apps/last.png"][..],
        ),
    ] {
        let dir = root.join(theme);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.theme"), index).unwrap();
        for f in files {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
    }
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(t.chain, ["test", "parent", "hicolor"]);
    assert_eq!(
        t.lookup("both", 16, 1),
        Some(root.join("parent/32x32/apps/both.png"))
    );
    assert_eq!(
        t.lookup("last", 16, 1),
        Some(root.join("hicolor/16x16/apps/last.png"))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn cache_resolves_by_name_and_absolute_path_and_caches_misses() {
    let root = fixture("test", INDEX, &[]);
    let icon = root.join("test/16x16/apps/cached.png");
    fs::create_dir_all(icon.parent().unwrap()).unwrap();
    write_png(&icon, 1, 1, &[0, 255, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    // By name through the theme.
    let c = cache.get("cached", 16).unwrap();
    assert_eq!((c.width, c.height), (16, 16));
    assert_eq!(c.get(8, 8), 0xFF00_FF00);
    // By absolute path.
    let c = cache.get(icon.to_str().unwrap(), 8).unwrap();
    assert_eq!(c.width, 8);
    // A miss is remembered: creating the file afterwards does not change the answer.
    assert!(cache.get("late", 16).is_none());
    write_png(
        &root.join("test/16x16/apps/late.png"),
        1,
        1,
        &[0, 0, 0, 255],
    );
    assert!(cache.get("late", 16).is_none());
    // A hit is remembered too: deleting the file keeps the decoded icon.
    fs::remove_file(&icon).unwrap();
    assert!(cache.get("cached", 16).is_some());
    // Different size is a different key.
    assert!(cache.get("cached", 32).is_none());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_scaled_cache_loads_the_device_size_and_keeps_the_logical_one() {
    let root = fixture("test", INDEX, &[]);
    let dir = root.join("test");
    fs::create_dir_all(dir.join("16x16/apps")).unwrap();
    fs::create_dir_all(dir.join("32x32/apps")).unwrap();
    write_png(&dir.join("16x16/apps/app.png"), 1, 1, &[255, 0, 0, 255]);
    write_png(&dir.join("32x32/apps/app.png"), 1, 1, &[0, 255, 0, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let one = cache.get("app", 16).unwrap();
    assert_eq!(
        (one.width, one.scale(), one.get(0, 0)),
        (16, 1, 0xFFFF_0000)
    );
    cache.set_scale(2);
    let two = cache.get("app", 16).unwrap();
    assert_eq!((two.width, two.height, two.scale()), (16, 16, 2));
    assert_eq!(two.device_size(), (32, 32));
    assert_eq!(two.get(0, 0), 0xFF00_FF00, "the 32 px file");
    fs::remove_dir_all(&root).unwrap();
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn png_to_canvas_names_the_size_it_refuses() {
    let p = tmp("bound.png");
    write_png(&p, 3, 2, &[0u8; 24]);
    let bytes = fs::read(&p).unwrap();
    fs::remove_file(&p).unwrap();
    assert_eq!(png_to_canvas(&bytes, 2).unwrap_err(), "refusing a 3x2 png");
    assert_eq!(png_to_canvas(&bytes, 3).unwrap().width, 3);
    assert!(
        png_to_canvas(b"not a png", 16)
            .unwrap_err()
            .starts_with("bad png header")
    );
}

#[test]
fn decode_rejects_oversized_images() {
    // A real, well-formed image past the bound.
    let wide = tmp("wide.png");
    let past = MAX_ICON_DIMENSION as usize + 1;
    write_png(&wide, past as u32, 1, &vec![255u8; past * 4]);
    assert!(decode(&wide, 16).is_none());
    fs::remove_file(&wide).unwrap();
    // A valid 1x1 file whose IHDR claims 60000x60000: rejected before a 13 GiB buffer is
    // allocated.
    let p = tmp("huge.png");
    write_png(&p, 1, 1, &[255, 0, 0, 255]);
    let mut bytes = fs::read(&p).unwrap();
    // 8 byte signature, 4 byte length, 4 byte "IHDR", then width, height.
    bytes[16..20].copy_from_slice(&60000u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&60000u32.to_be_bytes());
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    fs::write(&p, &bytes).unwrap();
    assert!(decode(&p, 16).is_none());
    fs::remove_file(&p).unwrap();
}

#[test]
fn decodes_paletted_png() {
    let p = tmp("indexed.png");
    let file = fs::File::create(&p).unwrap();
    let mut enc = png::Encoder::new(file, 1, 1);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(vec![0, 0, 255]);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(&[0]).unwrap();
    drop(writer);
    let c = decode(&p, 1).unwrap();
    assert_eq!(c.get(0, 0), 0xFF00_00FF);
    fs::remove_file(&p).unwrap();
}

#[test]
fn cache_rejects_paths_that_are_not_decodable_icons() {
    let root = fixture("test", INDEX, &[]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    // An absolute path without an icon extension is refused before opening; reading
    // /dev/zero would never end.
    assert!(cache.get("/dev/zero", 16).is_none());
    // A relative path is not a theme icon name.
    assert!(cache.get("../../dev/zero", 16).is_none());
    assert!(cache.get("apps/cached.png", 16).is_none());
    fs::remove_dir_all(&root).unwrap();
}

/// A FIFO or device with an icon extension is opened but not read.
#[test]
fn decode_refuses_files_that_are_not_regular() {
    let dir = scratch();
    let fifo = dir.join("x.png");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: `c` is a NUL-terminated path that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(decode(&fifo, 16).is_none());
    let zero = dir.join("zero.png");
    std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
    assert!(decode(&zero, 16).is_none());
    fs::remove_dir_all(&dir).unwrap();
}

/// Peer-chosen names that miss are remembered up to a cap, then dropped together; hits stay.
#[test]
fn misses_are_bounded_and_never_evict_hits() {
    let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
    write_png(
        &root.join("test/32x32/apps/app.png"),
        1,
        1,
        &[255, 0, 0, 255],
    );
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    assert!(cache.get("app", 32).is_some());
    for i in 0..(MISS_CAP * 3) {
        assert!(cache.get(&format!("nope-{i}"), 32).is_none());
    }
    assert!(cache.len() <= MISS_CAP + 1, "{} entries", cache.len());
    assert_eq!(cache.counts.iter().sum::<usize>(), cache.len());
    assert!(cache.get("app", 32).is_some());
    fs::remove_dir_all(&root).unwrap();
}

/// `has` walks the theme once per name; the answer stays if the root vanishes.
#[test]
fn has_remembers_its_answer_per_name() {
    let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
    write_png(
        &root.join("test/32x32/apps/app.png"),
        1,
        1,
        &[255, 0, 0, 255],
    );
    let cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    assert!(cache.has("app"));
    assert!(!cache.has("nope"));
    fs::remove_dir_all(&root).unwrap();
    assert!(cache.has("app"), "answered from memory, not the disk");
    assert!(!cache.has("nope"));
}

/// A file that fails to decode gives way to the name's next-best size, then to the next
/// theme's file; the lookup still names the best file.
#[test]
fn a_broken_file_gives_way_to_the_next_candidate() {
    let root = fixture(
        "test",
        INDEX,
        &[
            "32x32/apps/app.png",
            "32x32/apps/gone.png",
            "16x16/apps/app.png",
        ],
    );
    let (test, hicolor) = (root.join("test"), root.join("hicolor"));
    fs::write(test.join("32x32/apps/app.png"), b"not a png").unwrap();
    fs::write(test.join("32x32/apps/gone.png"), b"not a png").unwrap();
    write_png(&test.join("16x16/apps/app.png"), 1, 1, &[255, 0, 0, 255]);
    fs::create_dir_all(hicolor.join("48x48/apps")).unwrap();
    fs::write(
        hicolor.join("index.theme"),
        "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
    )
    .unwrap();
    write_png(
        &hicolor.join("48x48/apps/gone.png"),
        1,
        1,
        &[0, 0, 255, 255],
    );
    let theme = IconTheme::load("test", std::slice::from_ref(&root));
    assert_eq!(
        theme.lookup("app", 32, 1),
        Some(test.join("32x32/apps/app.png"))
    );
    let mut cache = IconCache::new(theme);
    assert_eq!(
        cache.get("app", 32).map(|c| c.get(16, 16)),
        Some(0xFFFF_0000)
    );
    assert_eq!(
        cache.get("gone", 32).map(|c| c.get(16, 16)),
        Some(0xFF00_00FF)
    );
    fs::remove_dir_all(&root).unwrap();
}

/// A name whose every file fails to decode is not carried, so a caller moves on to its next
/// name; one broken size among good ones still counts.
#[test]
fn has_answers_no_for_a_name_with_only_broken_files() {
    let root = fixture(
        "test",
        INDEX,
        &[
            "32x32/apps/broken.png",
            "32x32/apps/half.png",
            "16x16/apps/legacy.xpm",
        ],
    );
    let apps = root.join("test/32x32/apps");
    fs::write(apps.join("broken.png"), b"not a png").unwrap();
    fs::write(apps.join("half.png"), b"not a png").unwrap();
    write_png(
        &root.join("test/16x16/apps/half.png"),
        1,
        1,
        &[255, 0, 0, 255],
    );
    let cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    assert!(!cache.has("broken"));
    assert!(!cache.has("legacy"), "xpm is not decoded");
    assert!(cache.has("half"));
    fs::remove_dir_all(&root).unwrap();
}

/// The first fetch at the probe size (16 device px, or the host's) takes the image `has`
/// decoded without reading the file; other sizes still read it.
#[test]
fn a_fetch_at_the_probe_size_takes_what_has_decoded() {
    for (set, probe, other) in [(None, 16, 32), (Some(48), 48, 16)] {
        for scale in [1, 2] {
            let tag = format!("probe{probe}-{scale}");
            let root = fixture("test", INDEX, &["16x16/apps/app.png"]);
            write_png(
                &root.join("test/16x16/apps/app.png"),
                1,
                1,
                &[255, 0, 0, 255],
            );
            let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
            cache.set_scale(scale);
            if let Some(px) = set {
                cache.set_probe_px(px);
            }
            assert!(cache.has("app"));
            fs::remove_dir_all(&root).unwrap();
            let got = cache.get("app", probe / scale).expect("the probed image");
            assert_eq!(
                (got.device_size(), got.scale(), got.get(0, 0)),
                ((probe as u32, probe as u32), scale, 0xFFFF_0000),
                "{tag}"
            );
            assert!(
                cache.get("app", other).is_none(),
                "{tag}: {other} px reads the disk"
            );
        }
    }
}

/// A listing drawing one size fetches a theme icon at it alone; `draw` adds the sizes a new
/// view wants and hands the swaps, keeping the canvases already worn.
#[test]
fn a_listing_fetches_the_drawn_sizes_and_adds_a_new_views() {
    let root = fixture("test", INDEX, &["16x16/apps/app.png", "32x32/apps/app.png"]);
    for (f, px) in [("16x16/apps/app.png", 16), ("32x32/apps/app.png", 32)] {
        write_png(
            &root.join("test").join(f),
            px,
            px,
            &vec![255; (px * px * 4) as usize],
        );
    }
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let mut icons = ListingIcons::default().drawing([false, false, true]);
    let held = icons.get(&mut cache, "app");
    let width = |s: &ListingSizes| s.clone().map(|c| c.map(|c| c.width));
    assert_eq!(width(&held), [None, None, Some(48)]);
    assert!(icons.draw(&mut cache, [false, false, true]).is_empty());

    let swaps = icons.draw(&mut cache, [true, false, false]);
    assert_eq!(swaps.len(), 1);
    let (old, new) = &swaps[0];
    assert!(std::rc::Rc::ptr_eq(
        old[2].as_ref().unwrap(),
        held[2].as_ref().unwrap()
    ));
    assert_eq!(width(new), [Some(16), None, Some(48)]);
    assert!(std::rc::Rc::ptr_eq(
        new[2].as_ref().unwrap(),
        held[2].as_ref().unwrap()
    ));
    assert_eq!(
        width(&icons.get(&mut cache, "other-app")),
        [None, None, None]
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_shared_hit_is_the_cached_canvas_itself() {
    let root = fixture("test", INDEX, &["32x32/apps/app.png"]);
    write_png(
        &root.join("test/32x32/apps/app.png"),
        1,
        1,
        &[255, 0, 0, 255],
    );
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let a = cache.get_shared("app", 32).unwrap();
    let b = cache.get_shared("app", 32).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(cache.get("app", 32).unwrap().get(0, 0), 0xFFFF_0000);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn cache_keys_on_the_extra_roots_too() {
    // Two tray items publishing "app" from different IconThemePath directories must not
    // share an entry.
    let root = fixture("test", INDEX, &[]);
    let (a, b) = (root.join("a"), root.join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    write_png(&a.join("app.png"), 1, 1, &[255, 0, 0, 255]);
    write_png(&b.join("app.png"), 1, 1, &[0, 0, 255, 255]);
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let ca = cache
        .get_with_extra("app", 8, std::slice::from_ref(&a))
        .unwrap();
    let cb = cache
        .get_with_extra("app", 8, std::slice::from_ref(&b))
        .unwrap();
    assert_eq!(ca.get(4, 4), 0xFFFF_0000);
    assert_eq!(cb.get(4, 4), 0xFF00_00FF);
    fs::remove_dir_all(&root).unwrap();
}

/// `theme` reading every size directory from disk, as without any `icon-theme.cache`.
fn listing_only(theme: IconTheme) -> IconTheme {
    for home in &theme.homes {
        assert!(home.state.set(HomeState::Direct).is_ok());
    }
    theme
}

/// Writes `theme_dir/icon-theme.cache` as `gtk-update-icon-cache` would: every directory
/// below it holding a regular `.png`, `.svg` or `.xpm` file, symlinks followed.
fn write_cache(theme_dir: &Path) {
    use std::collections::BTreeMap;
    let mut dirs: Vec<String> = Vec::new();
    let mut icons: BTreeMap<String, Vec<(u16, u16)>> = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(rel) = pending.pop() {
        let mut found: BTreeMap<String, u16> = BTreeMap::new();
        for entry in fs::read_dir(theme_dir.join(&rel)).unwrap() {
            let file = entry.unwrap().file_name().into_string().unwrap();
            let path = theme_dir.join(&rel).join(&file);
            if path.is_dir() {
                pending.push(rel.join(&file));
            } else if !rel.as_os_str().is_empty() && path.is_file() {
                for (ext, flag) in EXTENSIONS.iter().zip(EXTENSION_FLAGS) {
                    if let Some(stem) = file.strip_suffix(&format!(".{ext}")) {
                        *found.entry(stem.to_string()).or_default() |= flag;
                    }
                }
            }
        }
        if !found.is_empty() {
            let d = dirs.len() as u16;
            dirs.push(rel.to_str().unwrap().to_string());
            for (name, flags) in found {
                icons.entry(name).or_default().push((d, flags));
            }
        }
    }
    let buckets = icons.len().max(1) as u32;
    let mut out: Vec<u8> = vec![0, 1, 0, 0];
    let put = |out: &mut Vec<u8>, at: usize, v: u32| {
        out[at..at + 4].copy_from_slice(&v.to_be_bytes());
    };
    out.extend([0; 8]);
    put(&mut out, 4, 12);
    out.extend(buckets.to_be_bytes());
    out.extend(std::iter::repeat_n(0xff, 4 * buckets as usize));
    for (name, images) in &icons {
        let head = 16 + 4 * (themecache::hash(name.as_bytes()) % buckets) as usize;
        let at = out.len();
        out.extend(&out[head..head + 4].to_vec());
        out.extend([0; 8]);
        put(&mut out, head, at as u32);
        let len = out.len() as u32;
        put(&mut out, at + 4, len);
        out.extend(name.as_bytes());
        out.push(0);
        let len = out.len() as u32;
        put(&mut out, at + 8, len);
        out.extend((images.len() as u32).to_be_bytes());
        for (d, flags) in images {
            out.extend(d.to_be_bytes());
            out.extend(flags.to_be_bytes());
            out.extend([0; 4]);
        }
    }
    let list = out.len();
    put(&mut out, 8, list as u32);
    out.extend((dirs.len() as u32).to_be_bytes());
    out.extend(std::iter::repeat_n(0, 4 * dirs.len()));
    for (i, dir) in dirs.iter().enumerate() {
        let len = out.len() as u32;
        put(&mut out, list + 4 + 4 * i, len);
        out.extend(dir.as_bytes());
        out.push(0);
    }
    fs::write(theme_dir.join(themecache::FILE), out).unwrap();
}

fn set_mtime(path: &Path, t: std::time::SystemTime) {
    fs::File::open(path).unwrap().set_modified(t).unwrap();
}

/// Every directory under `dir`, `dir` included.
fn all_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    let mut i = 0;
    while i < out.len() {
        for e in fs::read_dir(&out[i]).unwrap().map(Result::unwrap) {
            if e.file_type().unwrap().is_dir() {
                out.push(e.path());
            }
        }
        i += 1;
    }
    out
}

/// A system root with `test` (fresh cache) inheriting `base` (no cache) inheriting `hicolor`
/// (cache stale in two directories), and a user root whose `test` has a cache older than
/// its directory.
fn cached_fixture() -> (PathBuf, PathBuf) {
    let top = scratch();
    let (system, user) = (top.join("system"), top.join("user"));
    let dirs = "16x16/apps,22x22/apps,24x24/apps,32x32/apps,48x48/apps,64x64/apps,scalable/apps,16x16@2/apps";
    let sections = "[16x16/apps]\nSize=16\nType=Fixed\n\n[22x22/apps]\nSize=22\nType=Fixed\n\n[24x24/apps]\nSize=24\n\n[32x32/apps]\nSize=32\nType=Fixed\n\n[48x48/apps]\nSize=48\nType=Fixed\n\n[64x64/apps]\nSize=64\nType=Fixed\n\n[scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[16x16@2/apps]\nSize=16\nScale=2\nType=Fixed\n";
    let themes: [(&Path, &str, &str, &[&str]); 4] = [
        (
            &system,
            "test",
            "base",
            &[
                "16x16/apps/a.png",
                "16x16/apps/b.svg",
                "16x16/apps/both.png",
                "16x16/apps/both.svg",
                "24x24/apps/t.xpm",
                "24x24/apps/t.png",
                "32x32/apps/a.png",
                "48x48/apps/x.svgz",
                "scalable/apps/s.svg",
                "scalable/apps/a.svg",
                "16x16@2/apps/a.png",
                "16x16@2/apps/b.png",
            ],
        ),
        (&user, "test", "", &["16x16/apps/u.png", "32x32/apps/a.svg"]),
        (
            &system,
            "base",
            "hicolor",
            &[
                "16x16/apps/b.png",
                "16x16/apps/base.png",
                "32x32/apps/base.svg",
                "scalable/apps/u.svg",
            ],
        ),
        (
            &system,
            "hicolor",
            "",
            &[
                "16x16/apps/h.png",
                "16x16/apps/gone.png",
                "48x48/apps/h.png",
                "scalable/apps/h.svg",
                "scalable/apps/t.svg",
            ],
        ),
    ];
    for (root, theme, inherits, files) in themes {
        let dir = root.join(theme);
        fs::create_dir_all(&dir).unwrap();
        if root == system {
            let index = format!(
                "[Icon Theme]\nName={theme}\nInherits={inherits}\nDirectories={dirs}\n\n{sections}"
            );
            fs::write(dir.join("index.theme"), index).unwrap();
        }
        for f in files {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"").unwrap();
        }
    }
    std::os::unix::fs::symlink("gone.png", system.join("test/16x16/apps/dangling.png")).unwrap();
    fs::write(system.join("bare.png"), b"").unwrap();
    for theme in [
        system.join("test"),
        system.join("hicolor"),
        user.join("test"),
    ] {
        write_cache(&theme);
    }
    let written = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000);
    let before = written - std::time::Duration::from_secs(1000);
    for dir in all_dirs(&top) {
        set_mtime(&dir, before);
    }
    for theme in [
        system.join("test"),
        system.join("hicolor"),
        user.join("test"),
    ] {
        set_mtime(&theme.join(themecache::FILE), written);
    }
    // Changed after the caches were written.
    fs::remove_file(system.join("hicolor/16x16/apps/gone.png")).unwrap();
    fs::write(system.join("hicolor/48x48/apps/late.png"), b"").unwrap();
    fs::create_dir_all(system.join("test/22x22/apps")).unwrap();
    fs::write(system.join("test/22x22/apps/new.png"), b"").unwrap();
    // Backdated like a copy keeping times: only its new parent shows the change.
    set_mtime(&system.join("test/22x22/apps"), before);
    set_mtime(&system.join("test"), before);
    set_mtime(
        &user.join("test"),
        written + std::time::Duration::from_secs(1),
    );
    (system, user)
}

/// A file a lookup can use: `decode` refuses anything but a regular file.
fn regular(p: PathBuf) -> Option<PathBuf> {
    p.is_file().then_some(p)
}

const NAMES: [&str; 17] = [
    "a",
    "b",
    "both",
    "t",
    "s",
    "u",
    "x",
    "base",
    "h",
    "gone",
    "late",
    "new",
    "dangling",
    "bare",
    "index",
    "icon-theme",
    "nothing",
];

#[test]
fn a_theme_cache_picks_the_files_the_listings_pick() {
    let (system, user) = cached_fixture();
    let roots = [user.clone(), system.clone()];
    let cached = IconTheme::load("test", &roots);
    let listed = listing_only(IconTheme::load("test", &roots));
    for name in NAMES {
        for size in [8, 16, 20, 22, 24, 32, 40, 48, 64, 96, 256, 600] {
            for scale in [1, 2] {
                let want = listed.find_with_extra(name, size, scale, &[], regular);
                let got = cached.find_with_extra(name, size, scale, &[], regular);
                assert_eq!(got, want, "{name} at {size}@{scale}");
                if name != "dangling" {
                    assert_eq!(
                        cached.lookup(name, size, scale),
                        listed.lookup(name, size, scale),
                        "{name} at {size}@{scale}"
                    );
                }
            }
        }
    }
    assert_eq!(
        cached.lookup("late", 48, 1),
        Some(system.join("hicolor/48x48/apps/late.png"))
    );
    assert_eq!(
        cached.lookup("new", 22, 1),
        Some(system.join("test/22x22/apps/new.png"))
    );
    assert_eq!(
        cached.lookup("u", 16, 1),
        Some(user.join("test/16x16/apps/u.png"))
    );
    // The listing names a dangling link that never decodes; the cache skips it.
    assert_eq!(
        listed.lookup("dangling", 16, 1),
        Some(system.join("test/16x16/apps/dangling.png"))
    );
    assert_eq!(cached.lookup("dangling", 16, 1), None);
    fs::remove_dir_all(system.parent().unwrap()).unwrap();
}

#[test]
fn an_icon_installed_or_removed_after_a_lookup_shows_once_reloaded() {
    let (system, user) = cached_fixture();
    let mut icons = IconCache::new(IconTheme::load("test", &[user.clone(), system.clone()]));
    let apps = system.join("test/16x16/apps");
    let t = icons.theme();
    assert_eq!(t.lookup("fresh", 16, 1), None);
    assert_eq!(
        t.lookup("t", 16, 1),
        Some(system.join("test/24x24/apps/t.png"))
    );
    assert_eq!(t.lookup("b", 16, 1), Some(apps.join("b.svg")));
    assert!(t.watch_dirs().contains(&apps));
    fs::write(apps.join("fresh.png"), b"").unwrap();
    fs::write(apps.join("t.png"), b"").unwrap();
    fs::remove_file(apps.join("b.svg")).unwrap();
    assert_eq!(t.lookup("fresh", 16, 1), None, "read once until reloaded");
    icons.reload();
    let t = icons.theme();
    assert_eq!(t.lookup("fresh", 16, 1), Some(apps.join("fresh.png")));
    assert_eq!(t.lookup("t", 16, 1), Some(apps.join("t.png")));
    assert_eq!(
        t.lookup("b", 16, 1),
        Some(system.join("test/16x16@2/apps/b.png"))
    );
    fs::remove_file(apps.join("fresh.png")).unwrap();
    assert_eq!(t.reload().lookup("fresh", 16, 1), None);
    fs::remove_dir_all(system.parent().unwrap()).unwrap();
}

#[test]
fn watched_directories_are_the_roots_homes_and_each_size_directory_or_its_parent() {
    let (system, user) = cached_fixture();
    let unmade = user.join("not-made-yet");
    let t = IconTheme::load("test", &[unmade.clone(), user.clone(), system.clone()]);
    let w = t.watch_dirs();
    for dir in [
        unmade,
        user.clone(),
        system.clone(),
        user.join("test"),
        system.join("hicolor"),
        system.join("test/16x16/apps"),
        system.join("test/22x22/apps"),
        system.join("hicolor/48x48/apps"),
    ] {
        assert!(w.contains(&dir), "{dir:?} watched");
    }
    // hicolor has no 22x22: a 22x22/apps made later shows in hicolor itself.
    assert!(!w.contains(&system.join("hicolor/22x22")));
    assert!(!w.contains(&user.join("test/48x48/apps")));
    assert!(!w.contains(&user.join("base")), "a root without the theme");
    assert_eq!(w.len(), w.iter().collect::<HashSet<_>>().len(), "each once");
    fs::remove_dir_all(system.parent().unwrap()).unwrap();
}

#[test]
fn warming_reads_every_directory_and_keeps_the_answers() {
    let (system, user) = cached_fixture();
    let roots = [user.clone(), system.clone()];
    let (cold, warm) = (
        IconTheme::load("test", &roots),
        IconTheme::load("test", &roots),
    );
    warm.warm();
    assert!(warm.listings.iter().all(|l| l.get().is_some()));
    for name in NAMES {
        for (size, scale) in [(16, 1), (22, 1), (48, 1), (16, 2)] {
            assert_eq!(
                warm.lookup(name, size, scale),
                cold.lookup(name, size, scale)
            );
        }
    }
    fs::remove_dir_all(system.parent().unwrap()).unwrap();
}

#[test]
fn only_directories_that_exist_and_changed_since_the_cache_are_listed() {
    let (system, user) = cached_fixture();
    let t = IconTheme::load("test", &[user.clone(), system.clone()]);
    assert!(t.lookup("nothing", 16, 1).is_none());
    let listing = |dir: &Path| {
        let i = t.dirs.iter().position(|d| d.path == dir).unwrap();
        t.listings[i].get().unwrap()
    };
    let names = |dir: &Path| match listing(dir) {
        Listing::Names(n) => {
            let mut n: Vec<_> = n.iter().map(|f| f.to_str().unwrap()).collect();
            n.sort();
            Some(n)
        }
        Listing::Cached(_) => None,
    };
    assert!(names(&system.join("test/16x16/apps")).is_none());
    assert!(names(&system.join("hicolor/scalable/apps")).is_none());
    assert_eq!(
        names(&system.join("test/22x22/apps")),
        Some(vec!["new.png"])
    );
    assert_eq!(
        names(&system.join("hicolor/48x48/apps")),
        Some(vec!["h.png", "late.png"])
    );
    assert_eq!(
        names(&system.join("hicolor/16x16/apps")),
        Some(vec!["h.png"])
    );
    assert_eq!(names(&user.join("test/16x16/apps")), Some(vec!["u.png"]));
    assert!(
        !t.dirs.iter().any(|d| d.path.starts_with(user.join("base"))),
        "a root without the theme adds none of its directories"
    );
    let base = t
        .homes
        .iter()
        .find(|h| h.path == system.join("base"))
        .unwrap();
    let HomeState::Listed(entries) = base.state() else {
        panic!("base has no cache");
    };
    let mut above: Vec<String> = entries
        .lock()
        .unwrap()
        .keys()
        .map(|k| k.to_str().unwrap().to_string())
        .collect();
    above.sort();
    assert_eq!(
        above,
        ["", "16x16", "32x32", "scalable"],
        "only parents that exist"
    );
    fs::remove_dir_all(system.parent().unwrap()).unwrap();
}
