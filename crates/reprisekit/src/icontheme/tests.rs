use super::cache::{HIT_CAP, MISS_CAP};
use super::decode::{
    SPENT, SVG_MAX_BYTES, SVG_MAX_IMAGES, SVG_MAX_PICTURE_PIXELS, SVG_MAX_WORK, parse_xml,
    svg_over_budget, svg_side,
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
fn an_extra_folder_is_searched_as_a_hicolor_tree_then_flat() {
    let root = fixture("test", INDEX, &[]);
    let extra = root.join("extra");
    for f in [
        "hicolor/16x16/apps/tray.png",
        "hicolor/22x22@2/status/tray.png",
        "hicolor/48x48/status/tray.png",
        "hicolor/scalable/apps/tray.svg",
        "hicolor/16x16/apps/both.png",
        "both.png",
        "flat.svg",
    ] {
        let p = extra.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"").unwrap();
    }
    let t = IconTheme::load("test", std::slice::from_ref(&root));
    let at = |name: &str, px: i32| {
        let found = t.lookup_with_extra(name, px, 1, std::slice::from_ref(&extra));
        found.map(|p| p.strip_prefix(&extra).unwrap().to_path_buf())
    };
    let path = |p: &str| Some(PathBuf::from(p));
    assert_eq!(at("tray", 16), path("hicolor/16x16/apps/tray.png"));
    assert_eq!(at("tray", 44), path("hicolor/22x22@2/status/tray.png"));
    assert_eq!(at("tray", 32), path("hicolor/scalable/apps/tray.svg"));
    assert_eq!(at("both", 16), path("hicolor/16x16/apps/both.png"));
    assert_eq!(at("flat", 16), path("flat.svg"));
    fs::remove_file(extra.join("hicolor/scalable/apps/tray.svg")).unwrap();
    assert_eq!(
        at("tray", 32),
        path("hicolor/22x22@2/status/tray.png"),
        "the closest bigger size"
    );
    assert_eq!(
        at("tray", 64),
        path("hicolor/48x48/status/tray.png"),
        "then the closest smaller"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_extra_tree_walk_stops_at_its_caps() {
    let listed = std::cell::RefCell::new(Vec::new());
    let list = |dir: &std::path::Path, max: usize| {
        listed.borrow_mut().push(max);
        let names: Vec<String> = match dir.strip_prefix("/x").unwrap().components().count() {
            1 => (1..=500).map(|n| format!("{n}x{n}")).collect(),
            2 => ('a'..='z').map(String::from).collect(),
            _ => vec!["tray.png".into()],
        };
        names.into_iter().map(Into::into).take(max).collect()
    };
    let mut tree = ExtraTree::new(std::path::Path::new("/x"), list);
    let files = tree.files("tray", 16);
    assert_eq!(files.len(), EXTRA_MAX_FILES);
    assert_eq!(files[0], PathBuf::from("/x/hicolor/16x16/a/tray.png"));
    tree.files("tray", 48);
    let listed = listed.borrow();
    assert_eq!(listed.len(), EXTRA_MAX_FOLDERS, "folders listed");
    assert!(listed.iter().all(|&max| max == EXTRA_MAX_ENTRIES));
}

#[test]
fn a_peer_svg_embedding_a_raster_image_is_refused() {
    const DOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image width="16" height="16" href="data:image/png;base64,{DOT}"/></svg>"##
    );
    let p = tmp("embedding.svg");
    fs::write(&p, &svg).unwrap();
    assert!(decode(&p, 16).is_some(), "a theme's file");
    assert!(decode_bytes(&p, svg.as_bytes(), 16, true).is_some());
    assert!(decode_bytes(&p, svg.as_bytes(), 16, false).is_none());
    fs::remove_file(&p).unwrap();
}

/// An 8x8 baseline JPEG of #ff0000.
const RED_JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAARCAAIAAgDAREAAhEBAxEB/8QAFAABAAAAAAAAAAAAAAAAAAAACP/EABQQAQAAAAAAAAAAAAAAAAAAAAD/xAAVAQEBAAAAAAAAAAAAAAAAAAAHCf/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/ADoDFU3/2Q==";

fn base64(s: &str) -> Vec<u8> {
    let digit = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    } as u32;
    let digits: Vec<u32> = s.bytes().filter(|&c| c != b'=').map(digit).collect();
    digits
        .chunks(4)
        .flat_map(|q| {
            let n = (q.iter().enumerate()).fold(0, |n, (i, d)| n | d << (18 - 6 * i));
            n.to_be_bytes()[1..q.len()].to_vec()
        })
        .collect()
}

/// A percent-encoded `data:` URL of `bytes`.
fn data_url(mime: &str, bytes: &[u8]) -> String {
    let data: String = bytes.iter().map(|b| format!("%{b:02X}")).collect();
    format!("data:{mime},{data}")
}

/// A 16x16 SVG drawing `body` after defining `<image id="i">` from `bytes` of type `mime`.
fn embedding(mime: &str, bytes: &[u8], body: &str) -> String {
    let href = data_url(mime, bytes);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16"><defs><image id="i" width="16" height="16" href="{href}"/></defs>{body}</svg>"##
    )
}

/// Runs `f` and returns the pictures it decoded and the bytes usvg handed the resolver.
fn spent<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
    SPENT.with(|s| s.set((0, 0)));
    let out = f();
    let (decoded, handed) = SPENT.with(|s| s.get());
    (out, decoded, handed)
}

const USE: &str = r##"<use xlink:href="#i"/>"##;

#[test]
fn a_host_svg_embedding_a_png_or_a_jpeg_draws_its_pixels() {
    let p = tmp("embedding-raster.svg");
    let red_png = png_bytes(2, 2, &[255, 0, 0, 255].repeat(4));
    fs::write(&p, embedding("image/png", &red_png, USE)).unwrap();
    assert_eq!(decode(&p, 16).unwrap().get(8, 8), 0xFFFF_0000);

    fs::write(&p, embedding("image/jpeg", &base64(RED_JPEG), USE)).unwrap();
    let px = decode(&p, 16).unwrap().get(8, 8);
    let [a, r, g, b] = px.to_be_bytes();
    assert!(
        a == 0xFF && r > 0xF0 && g < 0x10 && b < 0x10,
        "opaque red, not {px:08x}"
    );
    fs::remove_file(&p).unwrap();
}

#[test]
fn an_svg_embedding_an_image_past_the_limits_is_refused() {
    let over = |svg: String| {
        let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &Default::default()).unwrap();
        svg_over_budget(&tree, true, 16, SVG_MAX_WORK)
    };
    let side = MAX_ICON_DIMENSION as usize;
    let wide = png_bytes(side as u32 + 1, 1, &vec![0; (side + 1) * 4]);
    assert_eq!(
        over(embedding("image/png", &wide, USE)),
        Some("an embedded image past the size limit")
    );
    let mut tall = base64(RED_JPEG);
    let sof = tall.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
    tall[sof + 5..sof + 7].copy_from_slice(&(side as u16 + 1).to_be_bytes());
    assert_eq!(
        over(embedding("image/jpeg", &tall, USE)),
        Some("an embedded image past the size limit")
    );
    let dot = png_bytes(1, 1, &[255, 0, 0, 255]);
    let mut bad_crc = dot.clone();
    bad_crc[29] ^= 0xFF;
    assert_eq!(
        over(embedding("image/png", &bad_crc, USE)),
        Some("an unreadable embedded image")
    );

    let uses = |n: usize| over(embedding("image/png", &dot, &USE.repeat(n)));
    assert_eq!(uses(SVG_MAX_IMAGES), None);
    assert_eq!(uses(SVG_MAX_IMAGES + 1), Some("too many embedded images"));

    let side = SVG_MAX_PICTURE_PIXELS.isqrt() as u32;
    let square = png_bytes(side, side, &vec![0; (side * side * 4) as usize]);
    let squares = |n: usize| over(embedding("image/png", &square, &USE.repeat(n)));
    assert_eq!(squares(1), None);
    assert_eq!(squares(2), Some("embedded pictures past the pixel budget"));

    let nested = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>"#;
    let svgs = |n: usize| {
        over(embedding(
            "image/svg+xml",
            nested.as_bytes(),
            &USE.repeat(n),
        ))
    };
    assert_eq!(svgs(SVG_MAX_IMAGES), None);
    assert_eq!(svgs(SVG_MAX_IMAGES + 1), Some("too many embedded images"));
}

#[test]
fn a_host_svg_embedding_a_gif_draws_all_but_the_gif() {
    const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
    let p = tmp("embedding-gif.svg");
    let body = r##"<rect width="8" height="16" fill="#0000ff"/><use xlink:href="#i" x="8"/>"##;
    fs::write(&p, embedding("image/gif", GIF, body)).unwrap();
    let c = decode(&p, 16).expect("the icon draws");
    assert_eq!((c.get(4, 8), c.get(12, 8)), (0xFF00_00FF, 0));
    fs::remove_file(&p).unwrap();
}

/// A 16x16 SVG whose `kib` KiB picture is drawn `10^levels` times through nested `<use>` groups.
fn fan_out(kib: usize, levels: u32) -> String {
    let mut payload = png_bytes(1, 1, &[255, 0, 0, 255]);
    payload.resize(kib << 10, 0);
    let mut groups = String::new();
    for l in 1..=levels {
        let uses = format!(r##"<use xlink:href="#l{}"/>"##, l - 1).repeat(10);
        groups += &format!(r#"<g id="l{l}">{uses}</g>"#);
    }
    let image = embedding("image/png", &payload, "");
    let image = image.replacen(r#"id="i""#, r#"id="l0""#, 1);
    image.replacen(
        "</defs>",
        &format!(r##"{groups}</defs><use xlink:href="#l{levels}"/>"##),
        1,
    )
}

#[test]
fn an_svg_drawing_its_picture_a_thousand_times_decodes_it_once() {
    let p = tmp("fan-out.svg");
    let thousand = fan_out(16, 3);
    assert!(thousand.len() < SVG_MAX_BYTES);
    let (drawn, decoded, handed) = spent(|| decode_bytes(&p, thousand.as_bytes(), 16, true));
    assert_eq!(drawn.map(|c| c.get(8, 8)), Some(0xFFFF_0000));
    assert_eq!(decoded, 1);
    assert!(
        handed < 16 << 10,
        "usvg reads the 16 KiB picture's bytes less than once, not {handed} bytes"
    );
    let (refused, decoded, _) = spent(|| decode_bytes(&p, thousand.as_bytes(), 16, false));
    assert!(refused.is_none());
    assert_eq!(decoded, 0);
}

#[test]
fn an_svg_decodes_only_the_pictures_it_draws() {
    let p = tmp("unused.svg");
    let nested = |i: usize| {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="red" data-i="{i}"/></svg>"#
        );
        data_url("image/svg+xml", svg.as_bytes())
    };
    let mut defs: String = (0..50)
        .map(|i| {
            format!(
                r#"<image id="i{i}" width="16" height="16" href="{}"/>"#,
                nested(i)
            )
        })
        .collect();
    defs += &format!(r#"<foo href="{}"/>"#, nested(50));
    let svg = |body: &str| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16"><defs>{defs}</defs><a href="{}"><rect width="8" height="16" fill="#0000ff"/></a>{body}</svg>"##,
            nested(51)
        )
    };
    let (none, decoded, _) = spent(|| decode_bytes(&p, svg("").as_bytes(), 16, true));
    assert_eq!(
        none.map(|c| (c.get(4, 8), c.get(12, 8))),
        Some((0xFF00_00FF, 0))
    );
    assert_eq!(decoded, 0, "no picture is drawn");
    let one = svg(r##"<use xlink:href="#i7" x="8"/>"##);
    let (one, decoded, _) = spent(|| decode_bytes(&p, one.as_bytes(), 16, true));
    assert_eq!(one.map(|c| c.get(12, 8)), Some(0xFFFF_0000));
    assert_eq!(decoded, 1);
}

#[test]
fn a_host_svg_draws_without_an_embedded_svg_past_the_budget() {
    let p = tmp("nested-over.svg");
    let mut nested = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><rect id="r0" width="1" height="1" fill="red"/>"#,
    );
    for l in 1..=4 {
        let uses = format!(r##"<use href="#r{}"/>"##, l - 1).repeat(10);
        nested += &format!(r#"<g id="r{l}">{uses}</g>"#);
    }
    nested += r##"</defs><use href="#r4"/></svg>"##;
    let body = r##"<rect width="8" height="16" fill="#0000ff"/><use xlink:href="#i" x="8"/>"##;
    let svg = embedding("image/svg+xml", nested.as_bytes(), body);
    let c = decode_bytes(&p, svg.as_bytes(), 16, true).expect("the icon draws");
    assert_eq!((c.get(4, 8), c.get(12, 8)), (0xFF00_00FF, 0));
}

/// Runs `f` and returns the warnings logged on this thread meanwhile.
fn warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    use std::sync::{Mutex, Once};
    use std::thread::{ThreadId, current};
    static LINES: Mutex<Vec<(ThreadId, String)>> = Mutex::new(Vec::new());
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Warn
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                let line = format!("{}: {}", r.target(), r.args());
                LINES.lock().unwrap().push((current().id(), line));
            }
        }
        fn flush(&self) {}
    }
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        log::set_logger(&Capture).expect("no other reprisekit test sets a logger");
        log::set_max_level(log::LevelFilter::Warn);
    });
    let me = current().id();
    let out = f();
    let mut lines = LINES.lock().unwrap();
    let mine = lines.iter().filter(|(t, _)| *t == me);
    let mine = mine.map(|(_, l)| l.clone()).collect();
    lines.retain(|(t, _)| *t != me);
    (out, mine)
}

#[test]
fn a_host_svg_draws_its_filtered_masks_past_the_picture_budgets_blank_without_a_warning() {
    let p = tmp("masks.svg");
    let side = SVG_MAX_PICTURE_PIXELS.isqrt() as u32 + 44;
    let head = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><filter id="a" x="0%" y="0%" width="100%" height="100%"><feColorMatrix in="SourceGraphic" values="0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0"/></filter><rect width="8" height="16" fill="#0000ff"/>"##;
    let mut masks = String::new();
    for i in 0..9u8 {
        let mut white = vec![255; (side * side * 4) as usize];
        white[0] = i;
        let href = data_url("image/png", &png_bytes(side, side, &white));
        masks += &format!(
            r##"<mask id="m{i}"><g filter="url(#a)"><image width="16" height="16" href="{href}"/></g></mask><rect x="8" width="8" height="16" fill="red" mask="url(#m{i})"/>"##
        );
    }
    let svg = format!("{head}{masks}</svg>");
    assert!(svg.len() < SVG_MAX_BYTES);
    let unmasked = format!("{head}</svg>");
    for px in [16, 48] {
        let (c, warned) = warnings(|| decode_bytes(&p, svg.as_bytes(), px, true));
        assert_eq!(warned, Vec::<String>::new());
        let c = c.expect("the icon draws");
        let unmasked = decode_bytes(&p, unmasked.as_bytes(), px, true).unwrap();
        assert!(c.pixels == unmasked.pixels, "the masked rects draw nothing");
    }
    assert!(decode_bytes(&p, svg.as_bytes(), 16, false).is_none());
}

#[test]
fn an_svg_whose_entities_expand_past_the_file_cap_is_refused() {
    let p = tmp("entities.svg");
    let doc = |decls: &str, body: &str| {
        format!(
            r#"<!DOCTYPE svg [{decls}]><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/>{body}</svg>"#
        )
    };
    let elements = doc(
        &format!(r#"<!ENTITY e "{}">"#, "<g/>".repeat(1000)),
        &"&e;".repeat(100),
    );
    let text = doc(
        &format!(r#"<!ENTITY t "{}">"#, "A".repeat(1000)),
        &format!("<desc>{}</desc>", "&t;".repeat(2000)),
    );
    for svg in [elements, text] {
        for images in [true, false] {
            assert!(decode_bytes(&p, svg.as_bytes(), 16, images).is_none());
        }
    }
    let uri = r#"<!DOCTYPE svg [<!ENTITY ns "http://www.w3.org/2000/svg">]><svg xmlns="&ns;" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#;
    assert_eq!(
        decode_bytes(&p, uri.as_bytes(), 16, false).map(|c| c.get(8, 8)),
        Some(0xFFFF_0000)
    );
}

#[test]
fn an_svg_draws_a_picture_named_through_an_entity_or_both_href_attributes() {
    let p = tmp("hrefs.svg");
    let red = data_url("image/png", &png_bytes(1, 1, &[255, 0, 0, 255]));
    let head = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16">"#;
    let entity = format!(
        r#"<!DOCTYPE svg [<!ENTITY u "{red}">]>{head}<image width="4" height="16" href="&u;"/><image x="6" width="4" height="16" href="&u;"/><image x="12" width="4" height="16" href="&u;"/></svg>"#
    );
    let (c, decoded, _) = spent(|| decode_bytes(&p, entity.as_bytes(), 16, true));
    let c = c.expect("the icon draws");
    assert_eq!(
        [2, 8, 14].map(|x| c.get(x, 8)),
        [0xFFFF_0000; 3],
        "each expansion draws"
    );
    assert_eq!(decoded, 1, "the entity's one picture");

    let both =
        format!(r#"{head}<image width="16" height="16" xlink:href="{red}" href="{red}"/></svg>"#);
    assert_eq!(
        decode_bytes(&p, both.as_bytes(), 16, true).map(|c| c.get(8, 8)),
        Some(0xFFFF_0000)
    );
}

#[test]
fn an_svg_whose_picture_href_has_an_overlong_name_is_refused() {
    let p = tmp("qname.svg");
    let red = data_url("image/png", &png_bytes(1, 1, &[255, 0, 0, 255]));
    let svg = |prefix: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image xmlns:{prefix}="http://www.w3.org/1999/xlink" width="16" height="16" {prefix}:href="{red}"/></svg>"#
        )
    };
    assert_eq!(
        decode_bytes(&p, svg("q").as_bytes(), 16, true).map(|c| c.get(8, 8)),
        Some(0xFFFF_0000)
    );
    let long = svg(&"q".repeat(70_000));
    assert!(long.len() < SVG_MAX_BYTES);
    assert!(decode_bytes(&p, long.as_bytes(), 16, true).is_none());
}

#[test]
fn cargo_lock_holds_one_png_and_one_zune_jpeg() {
    let lock =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    for name in ["png", "zune-jpeg"] {
        let entry = format!("name = \"{name}\"\n");
        assert_eq!(
            lock.matches(&entry).count(),
            1,
            "one {name}, so the embedded picture's header is read by the decoder resvg draws with"
        );
    }
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

fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
    writer.finish().unwrap();
    out
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    fs::write(path, png_bytes(w, h, rgba)).unwrap();
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

fn budget_svg(body: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16">{body}</svg>"##
    )
}

fn budget_tree(body: &str) -> resvg::usvg::Tree {
    resvg::usvg::Tree::from_str(&budget_svg(body), &Default::default()).unwrap()
}

const SHAPE: &str = r##"<rect width="14" height="14" fill="#f00"/>"##;

/// Stock icons blur a shadow and fade a layer or two; such an icon draws at full size.
#[test]
fn an_icon_with_a_blur_and_a_faded_layer_draws_at_its_size() {
    let icon = budget_tree(&format!(
        r##"<filter id="f"><feGaussianBlur stdDeviation="1"/></filter><g filter="url(#f)">{SHAPE}</g><g opacity=".5">{SHAPE}</g>"##
    ));
    assert_eq!(svg_side(&icon, true, 48, SVG_MAX_WORK), Ok(48));
    assert_eq!(svg_side(&icon, true, 256, SVG_MAX_WORK), Ok(256));
}

/// Each takes seconds to draw at its size without the budget.
#[test]
fn an_svg_whose_pixels_cost_past_the_budget_draws_smaller_and_is_scaled_up() {
    let blurs = budget_tree(&format!(
        r##"<filter id="f">{}</filter><g filter="url(#f)">{SHAPE}</g>"##,
        r#"<feGaussianBlur stdDeviation="4"/>"#.repeat(64)
    ));
    assert_eq!(svg_side(&blurs, true, 16, SVG_MAX_WORK), Ok(16));
    let side = svg_side(&blurs, true, 128, SVG_MAX_WORK).unwrap();
    assert!(
        (32..128).contains(&side),
        "drawn between 32 and 127 px, not {side}"
    );
    assert_eq!(
        svg_side(&blurs, true, 512, SVG_MAX_WORK),
        Err("past the render budget")
    );

    let layers = budget_tree(&format!(r##"<g opacity=".5">{SHAPE}</g>"##).repeat(300));
    assert!(svg_side(&layers, true, 256, SVG_MAX_WORK).is_ok_and(|s| s < 256));

    let p = tmp("blurs.svg");
    fs::write(
        &p,
        budget_svg(&format!(
            r##"<filter id="f">{}</filter><g filter="url(#f)"><rect width="16" height="16" fill="#00f"/></g>"##,
            r#"<feOffset dx="0"/>"#.repeat(100)
        )),
    )
    .unwrap();
    let c = decode(&p, 256).expect("drawn smaller");
    assert_eq!(
        (c.width, c.height, c.get(128, 128)),
        (256, 256, 0xFF00_00FF)
    );
    fs::remove_file(&p).unwrap();
}

/// Each group draws into a layer of its own, and its clip path into two more pixmaps.
#[test]
fn an_svg_of_two_hundred_clipped_layers_draws_smaller_at_96_px() {
    let groups = (0..200).map(|i| {
        format!(
            r##"<clipPath id="k{i}"><circle cx="8" cy="8" r="7.5"/></clipPath><g opacity=".95" clip-path="url(#k{i})"><rect width="16" height="16" fill="#0a0"/></g>"##
        )
    });
    let clipped = budget_tree(&groups.collect::<String>());
    assert_eq!(svg_side(&clipped, true, 48, SVG_MAX_WORK), Ok(48));
    assert!(
        svg_side(&clipped, true, 96, SVG_MAX_WORK).is_ok_and(|s| (24..96).contains(&s)),
        "drawn between 24 and 95 px"
    );
}

#[test]
fn an_svg_whose_cost_does_not_shrink_with_its_size_is_refused() {
    // The rasterizer sorts a path's crossing edges against each other.
    let zigzag = format!("M0 0{}", "L16 16L0 16L16 0L0 0".repeat(10_000));
    let crossing = budget_tree(&format!(r##"<path d="{zigzag}"/>"##));
    assert_eq!(
        svg_side(&crossing, true, 16, SVG_MAX_WORK),
        Err("past the render budget")
    );
    // 0.0005 units of dash and gap on 61 units of path: about 120 000 dashes.
    let dashes = budget_tree(
        r##"<path d="M0 0L16 16L0 16L16 0" stroke="#000" stroke-dasharray="0.00025 0.00025"/>"##,
    );
    assert_eq!(
        svg_side(&dashes, true, 16, SVG_MAX_WORK),
        Err("past the render budget")
    );
    // resvg draws a pattern tile at the device scale: 4000 px a side even at 16 px, a quarter
    // of 64.
    let tile = budget_tree(&format!(
        r##"<pattern id="p" width="4000" height="4000" patternUnits="userSpaceOnUse">{SHAPE}</pattern><rect width="16" height="16" fill="url(#p)"/>"##
    ));
    assert_eq!(
        svg_side(&tile, true, 64, SVG_MAX_WORK),
        Err("past the render budget")
    );
}

/// A shadow over a region four times the icon's size each way, as a flag theme draws one.
fn shadowed_tree() -> resvg::usvg::Tree {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256"><filter id="f" x="-200%" y="-200%" width="400%" height="400%" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceGraphic" stdDeviation="4.3"/><feOffset dy="4" result="o"/><feFlood flood-opacity=".4"/><feComposite in2="o" operator="in" result="s"/><feBlend in="SourceGraphic" in2="s"/></filter><g filter="url(#f)"><rect x="6" y="14" width="244" height="228" fill="#c00"/></g></svg>"##;
    resvg::usvg::Tree::from_str(svg, &Default::default()).unwrap()
}

/// Each primitive is priced by its kind: the shadow's offset, flood, composite and blend cost
/// a few percent of its blur.
#[test]
fn a_shadow_over_a_wide_region_draws_at_its_size_up_to_84_px() {
    let shadowed = shadowed_tree();
    for px in [16, 24, 32, 48, 64, 84] {
        assert_eq!(svg_side(&shadowed, true, px, SVG_MAX_WORK), Ok(px));
    }
    let side = svg_side(&shadowed, true, 96, SVG_MAX_WORK).unwrap();
    assert!(
        (64..96).contains(&side),
        "drawn between 64 and 95 px, not {side}"
    );
}

/// resvg's IIR blur reads a region's columns from a buffer of f64 rows; rows a multiple of
/// 256 px long land in the same cache sets.
#[test]
fn a_small_blur_over_rows_a_multiple_of_256_px_costs_more() {
    let blurs = |width: u32| {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="100">{}</filter><g filter="url(#f)">{SHAPE}</g></svg>"##,
            r#"<feGaussianBlur stdDeviation="0.5"/>"#.repeat(2)
        );
        resvg::usvg::Tree::from_str(&svg, &Default::default()).unwrap()
    };
    let (aliased, other) = (blurs(256), blurs(250));
    let ratio = least_budget(&aliased, 64) / least_budget(&other, 64);
    assert!(
        ratio > 1.5,
        "a 256 px row costs 1.5 times a 250 px one or more, not {ratio}"
    );
}

/// The primitives whose cost grows with an attribute are priced by it: a few draw, and a
/// costly one draws smaller or is refused.
#[test]
fn every_filter_primitive_is_priced_by_its_attributes() {
    let filtered = |primitive: &str| {
        budget_tree(&format!(
            r##"<rect id="r" width="8" height="8" fill="#0f0"/><filter id="f">{primitive}</filter><g filter="url(#f)">{SHAPE}</g>"##
        ))
    };
    for cheap in [
        r#"<feMorphology radius="1"/>"#,
        r#"<feConvolveMatrix order="3" kernelMatrix="0 1 0 1 1 1 0 1 0"/>"#,
        r#"<feTurbulence baseFrequency=".1" numOctaves="2"/>"#,
        r#"<feDiffuseLighting><feDistantLight azimuth="45" elevation="45"/></feDiffuseLighting>"#,
        r#"<feSpecularLighting><fePointLight x="4" y="4" z="10"/></feSpecularLighting>"#,
        r#"<feDisplacementMap in2="SourceGraphic" scale="4"/>"#,
        r##"<feImage href="#r"/>"##,
    ] {
        let tree = filtered(cheap);
        assert_eq!(svg_side(&tree, true, 48, SVG_MAX_WORK), Ok(48), "{cheap}");
    }
    let octaves = filtered(r#"<feTurbulence baseFrequency=".1" numOctaves="1000000"/>"#);
    assert_eq!(
        svg_side(&octaves, true, 16, SVG_MAX_WORK),
        Err("past the render budget")
    );
    let kernel = vec!["1"; 99 * 99].join(" ");
    let order = filtered(&format!(
        r#"<feConvolveMatrix order="99" kernelMatrix="{kernel}"/>"#
    ));
    let side = svg_side(&order, true, 48, SVG_MAX_WORK).unwrap();
    assert!(
        (12..48).contains(&side),
        "drawn between 12 and 47 px, not {side}"
    );
    // A radius reads at most the whole region around each pixel.
    let radius = filtered(r#"<feMorphology radius="1000"/>"#);
    assert_eq!(svg_side(&radius, true, 16, SVG_MAX_WORK), Ok(16));
    let side = svg_side(&radius, true, 256, SVG_MAX_WORK).unwrap();
    assert!(
        (64..256).contains(&side),
        "drawn between 64 and 255 px, not {side}"
    );
}

/// The least budget `tree` draws within at `px`, to a thousandth; 1e13 when no budget does.
fn least_budget(tree: &resvg::usvg::Tree, px: u32) -> f64 {
    let (mut lo, mut hi) = (0.0, 1e13);
    while hi - lo > 1e3 + hi * 1e-3 {
        let mid = (lo + hi) / 2.0;
        match svg_over_budget(tree, true, px, mid) {
            None => hi = mid,
            Some(_) => lo = mid,
        }
    }
    hi
}

/// A flood and three colour matrices over a region of 2e5 units a side, under `transform`.
fn wide_filter_tree(transform: &str) -> resvg::usvg::Tree {
    budget_tree(&format!(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-1e5" y="-1e5" width="2e5" height="2e5"><feFlood flood-color="#f00"/>{}</filter><g transform="{transform}"><g filter="url(#f)">{SHAPE}</g></g>"##,
        r#"<feColorMatrix type="saturate" values=".5"/>"#.repeat(3)
    ))
}

/// Work per pixel of [`wide_filter_tree`]'s layer: the layer, the filter, the flood and three
/// colour matrices.
const WIDE_FILTER_WORK: f64 = 18.0 + 25.0 + 2.0 + 3.0 * 35.0;

/// resvg 0.49's `render_group` for a filtered group under `ts` in `bound`: the bound inside it,
/// its layer's pixels and the transform inside it. None when resvg skips the group.
fn resvg_filter_layer(
    ts: resvg::tiny_skia::Transform,
    region: resvg::tiny_skia::Rect,
    bound: resvg::tiny_skia::IntRect,
) -> Option<(resvg::tiny_skia::IntRect, f64, resvg::tiny_skia::Transform)> {
    use resvg::tiny_skia::{IntRect, Transform};
    let sx = ts.sx.hypot(ts.ky);
    let sy = ts.kx.hypot(ts.sy);
    let rest = Transform::from_row(ts.sx / sx, ts.ky / sx, ts.kx / sy, ts.sy / sy, 0.0, 0.0);
    let inv = rest.invert()?;
    let bound = bound.to_rect().transform(inv)?.round_out()?;
    let ts = inv.pre_concat(ts);
    let bbox = region.transform(ts)?;
    let ibbox = IntRect::from_xywh(
        bbox.x().floor() as i32,
        bbox.y().floor() as i32,
        bbox.width().ceil().max(1.0) as u32,
        bbox.height().ceil().max(1.0) as u32,
    )?;
    let layer = IntRect::from_ltrb(
        ibbox.left().max(bound.left()),
        ibbox.top().max(bound.top()),
        ibbox.right().min(bound.right()),
        ibbox.bottom().min(bound.bottom()),
    )?;
    let dx = bbox.x() - (bbox.x() - layer.x() as f32);
    let dy = bbox.y() - (bbox.y() - layer.y() as f32);
    let shifted = Transform::from_translate(-dx, -dy).pre_concat(ts);
    Some((bound, layer.width() as f64 * layer.height() as f64, shifted))
}

/// The wide region at 16 px: its canvas bound and region.
fn wide_region() -> (resvg::tiny_skia::IntRect, resvg::tiny_skia::Rect) {
    use resvg::tiny_skia::{IntRect, Rect};
    let bound = IntRect::from_xywh(-32, -32, 80, 80).unwrap();
    (bound, Rect::from_xywh(-1e5, -1e5, 2e5, 2e5).unwrap())
}

/// resvg stretches a skewed filtered group's layer bound by about tan(angle) each way: at 89.5
/// degrees a 16 px icon asks for a layer 9000 px a side.
#[test]
fn a_steeply_skewed_filter_region_draws_smaller_or_is_refused() {
    assert_eq!(
        svg_side(&wide_filter_tree("skewX(0)"), true, 16, SVG_MAX_WORK),
        Ok(16)
    );
    for angle in [88.0, 89.0, 89.5, 89.99, 89.9999] {
        let tree = wide_filter_tree(&format!("skewX({angle})"));
        assert!(
            !matches!(svg_side(&tree, true, 16, SVG_MAX_WORK), Ok(16)),
            "skewX({angle}) is not drawn at 16 px"
        );
    }
    let svg = budget_svg(&format!(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-1e5" y="-1e5" width="2e5" height="2e5"><feFlood flood-color="#f00"/>{}</filter><g transform="skewX(89.5)"><g filter="url(#f)">{SHAPE}</g></g>"##,
        r#"<feColorMatrix type="saturate" values=".5"/>"#.repeat(3)
    ));
    assert!(decode_bytes(&tmp("skewed.svg"), svg.as_bytes(), 16, true).is_none());
}

/// The price covers every pixel of the layer resvg draws, at every angle of a sweep, and never
/// falls as a skew steepens, so no angle between two priced ones slips through.
#[test]
fn a_skewed_or_rotated_filter_region_is_priced_at_least_at_the_layer_resvg_draws() {
    use resvg::tiny_skia::Transform;
    let (bound, region) = wide_region();
    let covers = |transform: &str, ts: Transform| {
        let price = least_budget(&wide_filter_tree(transform), 16);
        if let Some((_, pixels, _)) = resvg_filter_layer(ts, region, bound) {
            assert!(
                price >= pixels * WIDE_FILTER_WORK,
                "{transform}: {price} covers {pixels} px"
            );
        }
        price
    };
    for (kind, skew) in [
        ("skewX", Transform::from_row(1.0, 0.0, 1.0, 1.0, 0.0, 0.0)),
        ("skewY", Transform::from_row(1.0, 1.0, 0.0, 1.0, 0.0, 0.0)),
    ] {
        let mut last = 0.0;
        for step in 0..900 {
            let angle = step as f32 / 10.0;
            let tan = angle.to_radians().tan();
            let ts = Transform::from_row(1.0, skew.ky * tan, skew.kx * tan, 1.0, 0.0, 0.0);
            let price = covers(&format!("{kind}({angle})"), ts);
            assert!(
                price >= last * 0.998,
                "{kind}({angle}) costs {price}, at least the {last} of a shallower one"
            );
            last = price;
        }
    }
    for angle in 0..360 {
        let ts = Transform::from_rotate_at(angle as f32, 8.0, 8.0);
        covers(&format!("rotate({angle} 8 8)"), ts);
    }
}

/// A skewed filtered group inside another stretches the bound again.
#[test]
fn nested_skewed_filter_regions_are_priced_at_least_at_the_layers_resvg_draws() {
    use resvg::tiny_skia::Transform;
    let (bound, region) = wide_region();
    for angle in [10.0f32, 30.0, 45.0, 60.0, 70.0, 75.0, 80.0] {
        let tree = budget_tree(&format!(
            r##"<filter id="f" filterUnits="userSpaceOnUse" x="-1e5" y="-1e5" width="2e5" height="2e5"><feFlood/></filter><g transform="skewX({angle})"><g filter="url(#f)"><g transform="skewY({angle})"><g filter="url(#f)">{SHAPE}</g></g></g></g>"##
        ));
        let tan = angle.to_radians().tan();
        let outer = Transform::from_row(1.0, 0.0, tan, 1.0, 0.0, 0.0);
        let (bound, first, inside) = resvg_filter_layer(outer, region, bound).unwrap();
        let inner = inside.pre_concat(Transform::from_row(1.0, tan, 0.0, 1.0, 0.0, 0.0));
        let (_, second, _) = resvg_filter_layer(inner, region, bound).unwrap();
        let pixels = first + second;
        let price = least_budget(&tree, 16);
        assert!(
            price >= pixels * (18.0 + 25.0 + 2.0),
            "skewX then skewY({angle}): {price} covers {pixels} px"
        );
    }
}

/// resvg draws an SVG picture through a pixmap the size of the one it lands in and bounds the
/// picture's layers by five times that: in a layer five times the canvas, 25 times the canvas.
#[test]
fn an_svg_picture_in_a_wide_layer_is_priced_with_the_layers_it_draws() {
    let picture = budget_svg(&format!(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-1e5" y="-1e5" width="2e5" height="2e5"><feFlood/>{}</filter><g filter="url(#f)">{SHAPE}</g>"##,
        r#"<feColorMatrix type="saturate" values=".5"/>"#.repeat(20)
    ));
    let href = data_url("image/svg+xml", picture.as_bytes());
    let at_canvas = budget_tree(&format!(r#"<image width="16" height="16" href="{href}"/>"#));
    assert_eq!(svg_side(&at_canvas, true, 16, SVG_MAX_WORK), Ok(16));
    let in_layer = budget_tree(&format!(
        r#"<g opacity=".5"><image x="-32" y="-32" width="80" height="80" href="{href}"/></g>"#
    ));
    let side = svg_side(&in_layer, true, 16, SVG_MAX_WORK).unwrap();
    assert!(
        (4..16).contains(&side),
        "drawn between 4 and 15 px, not {side}"
    );
}

/// A filter keeps each primitive's result: 200 floods over a region five times a 48 px canvas
/// cost little work but about 44 MiB of pixmaps.
#[test]
fn an_svg_whose_pixmaps_pass_the_size_limit_draws_smaller() {
    let floods = budget_tree(&format!(
        r##"<filter id="f" filterUnits="userSpaceOnUse" x="-32" y="-32" width="80" height="80">{}</filter><g filter="url(#f)">{SHAPE}</g>"##,
        r#"<feFlood flood-color="red"/>"#.repeat(200)
    ));
    assert_eq!(svg_side(&floods, true, 16, SVG_MAX_WORK), Ok(16));
    assert_eq!(
        svg_over_budget(&floods, true, 48, f64::INFINITY),
        Some("pixmaps past the size limit")
    );
    let side = svg_side(&floods, true, 48, SVG_MAX_WORK).unwrap();
    assert!(
        (12..48).contains(&side),
        "drawn between 12 and 47 px, not {side}"
    );
}

/// One budget covers usvg's build and resvg's draw.
#[test]
fn an_svg_whose_build_and_draw_together_pass_the_budget_is_refused() {
    let p = tmp("rects.svg");
    let rects = |n| budget_svg(&r##"<rect width="16" height="16" fill="#f00"/>"##.repeat(n));
    assert!(decode_bytes(&p, rects(4_000).as_bytes(), 16, true).is_some());
    let many = rects(8_000);
    let tree = resvg::usvg::Tree::from_str(&many, &Default::default()).unwrap();
    assert_eq!(
        svg_side(&tree, true, 16, SVG_MAX_WORK),
        Ok(16),
        "its draw alone fits"
    );
    let (_, built) = parse_xml(&many, SVG_MAX_WORK).unwrap();
    assert!(built < SVG_MAX_WORK, "its build alone fits");
    assert!(decode_bytes(&p, many.as_bytes(), 16, true).is_none());
}

/// An embedded SVG picture builds within what the icon's build left.
#[test]
fn an_svg_whose_pictures_build_past_the_budget_together_draws_the_last_blank() {
    let picture = |fill: &str| {
        let unused = "<g/>".repeat(10_000);
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="16"><defs>{unused}</defs><rect width="8" height="16" fill="{fill}"/></svg>"#
        )
    };
    let red = data_url("image/svg+xml", picture("red").as_bytes());
    let blue = data_url("image/svg+xml", picture("blue").as_bytes());
    let svg = budget_svg(&format!(
        r#"<image width="8" height="16" href="{red}"/><image x="8" width="8" height="16" href="{blue}"/>"#
    ));
    assert!(svg.len() < SVG_MAX_BYTES);
    let c = decode_bytes(&tmp("pictures.svg"), svg.as_bytes(), 16, true).expect("the icon draws");
    assert_eq!((c.get(4, 8), c.get(12, 8)), (0xFFFF_0000, 0));
}

/// usvg builds each of these into a tree of hundreds of thousands of nodes or more, taking
/// a second or more, before a tree can be walked.
#[test]
fn an_svg_usvg_would_build_too_much_from_is_refused_before_it_does() {
    let refused = |body: &str| parse_xml(&budget_svg(body), SVG_MAX_WORK).err();
    let why = Some("too many elements once uses and references expand");
    let mut uses = String::from(r##"<rect id="g0" width="1" height="1"/>"##);
    for i in 1..=5 {
        let copies = format!(r##"<use xlink:href="#g{}"/>"##, i - 1).repeat(10);
        uses += &format!(r##"<g id="g{i}">{copies}</g>"##);
    }
    assert_eq!(refused(&uses), why);

    let mut patterns = format!(r##"<pattern id="p0" width="16" height="16">{SHAPE}</pattern>"##);
    for i in 1..=5 {
        let fills = format!(
            r##"<rect width="16" height="16" fill="url(#p{})"/>"##,
            i - 1
        );
        patterns += &format!(
            r##"<pattern id="p{i}" width="16" height="16">{}</pattern>"##,
            fills.repeat(10)
        );
    }
    assert_eq!(
        refused(&format!(
            r##"{patterns}<rect width="16" height="16" fill="url(#p5)"/>"##
        )),
        why
    );

    let masked = format!(
        r##"<mask id="m">{}</mask>{}"##,
        SHAPE.repeat(100),
        r##"<rect width="4" height="4" mask="url(#m)"/>"##.repeat(300)
    );
    assert_eq!(refused(&masked), why);

    let vertices = "L1 1L2 2".repeat(5_000);
    let markers = format!(
        r##"<marker id="m"><rect width="1" height="1"/><rect width="1" height="1"/></marker><path d="M0 0{vertices}" style="stroke:#000;marker-mid:url(#m)"/>"##
    );
    assert_eq!(refused(&markers), why);
    // `marker:none` among a style's declarations names no marker.
    let filled = format!(
        r##"<linearGradient id="g"><stop offset="0"/></linearGradient><path d="M0 0{vertices}" style="fill:url(#g);marker:none"/>"##
    );
    assert_eq!(refused(&filled), None);
}

#[test]
fn a_marker_too_deep_to_price_on_a_shape_without_vertices_is_refused() {
    let deep = format!("{}{SHAPE}{}", "<g>".repeat(1100), "</g>".repeat(1100));
    let body = format!(
        r##"<marker id="m">{deep}</marker><rect width="1" height="1" marker-start="url(#m)"/>"##
    );
    assert_eq!(
        parse_xml(&budget_svg(&body), SVG_MAX_WORK).err(),
        Some("too many elements once uses and references expand")
    );
}

#[test]
fn an_svg_past_the_byte_cap_is_refused_before_it_is_parsed() {
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

const RED_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#f00"/></svg>"##;

/// Writes `bytes` to `root`/`rel`, making its folder.
fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// A theme icon drawn from an SVG is never decoded by `get_unread`: it answers None, is wanted
/// once however often it is asked, and answers from the cache once `put_icons` brings it. A
/// small PNG still loads in place.
#[test]
fn get_unread_leaves_an_svg_theme_icon_to_a_job_and_takes_what_it_puts() {
    let root = fixture("test", INDEX, &["32x32/apps/small.png"]);
    put(&root, "test/scalable/apps/drawn.svg", RED_SVG.as_bytes());
    write_png(
        &root.join("test/32x32/apps/small.png"),
        1,
        1,
        &[0, 0, 255, 255],
    );
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    let waits = cache.waits();
    assert_eq!(cache.get_unread("drawn", 32), None);
    assert_eq!(cache.get_unread("drawn", 32), None);
    assert_eq!(cache.waits(), waits + 2, "each pending answer counts");
    assert_eq!(cache.take_wanted(), [("drawn".to_string(), 32)]);
    assert!(cache.is_empty(), "nothing decoded");
    let small = cache.get_unread("small", 32).flatten();
    assert_eq!(small.map(|c| c.get(16, 16)), Some(0xFF00_00FF));
    assert_eq!(cache.waits(), waits + 2, "a ready answer does not count");
    let theme = cache.theme();
    let art = load_icon(&theme, "drawn", 32, cache.scale());
    assert_eq!(art.as_ref().map(|c| c.get(16, 16)), Some(0xFFFF_0000));
    cache.put_icons(&theme, vec![("drawn".into(), 32, art.clone())]);
    assert_eq!(
        cache.get_unread("drawn", 32).flatten().as_deref(),
        art.as_ref()
    );
    assert!(cache.take_wanted().is_empty());
    assert_eq!(
        cache.get_unread("drawn", 16),
        None,
        "another size is its own job"
    );
    fs::remove_dir_all(&root).unwrap();
}

/// A PNG past a few hundred px of file goes to a job like an SVG; `get_shared` still decodes
/// both where it is asked.
#[test]
fn get_unread_leaves_a_large_png_to_a_job_and_get_shared_still_decodes() {
    let root = fixture("test", INDEX, &[]);
    let big = root.join("test/32x32/apps/big.png");
    fs::create_dir_all(big.parent().unwrap()).unwrap();
    write_png(&big, 1, 1, &[0, 255, 0, 255]);
    let mut bytes = fs::read(&big).unwrap();
    bytes.resize(bytes.len() + (80 << 10), 0);
    fs::write(&big, bytes).unwrap();
    put(&root, "test/scalable/apps/drawn.svg", RED_SVG.as_bytes());
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    assert_eq!(cache.get_unread("big", 32), None);
    assert!(cache.get_shared("big", 32).is_some());
    assert!(cache.get_shared("drawn", 32).is_some());
    fs::remove_dir_all(&root).unwrap();
}

/// `has_unread` counts a name whose file is an SVG without decoding it; `has` decodes and
/// finds it broken.
#[test]
fn has_unread_counts_an_svg_without_decoding_it() {
    let root = fixture("test", INDEX, &[]);
    put(&root, "test/scalable/apps/broken.svg", b"<svg");
    let theme = || IconTheme::load("test", std::slice::from_ref(&root));
    assert!(IconCache::new(theme()).has_unread("broken"));
    assert!(!IconCache::new(theme()).has("broken"));
    assert!(!IconCache::new(theme()).has_unread("absent"));
    fs::remove_dir_all(&root).unwrap();
}

/// A name `has_unread` counted without decoding is decoded at the probe size once the host takes
/// the wanted names; one whose files decode at no size stops counting, and `take_refused` names
/// it once.
#[test]
fn a_counted_name_whose_files_never_decode_stops_counting() {
    let root = fixture("test", INDEX, &[]);
    put(&root, "test/scalable/apps/broken.svg", b"<svg");
    put(&root, "test/scalable/apps/drawn.svg", RED_SVG.as_bytes());
    let mut cache = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
    assert!(cache.has_unread("broken"));
    assert!(cache.has_unread("drawn"));
    let mut wanted = cache.take_wanted();
    wanted.sort();
    assert_eq!(
        wanted,
        [("broken".to_string(), 16), ("drawn".to_string(), 16)]
    );
    let theme = cache.theme();
    let art = (wanted.into_iter())
        .map(|(n, px)| {
            let icon = load_icon(&theme, &n, px, cache.scale());
            (n, px, icon)
        })
        .collect();
    cache.put_icons(&theme, art);
    assert!(!cache.has_unread("broken"));
    assert!(cache.has_unread("drawn"));
    assert_eq!(cache.take_refused(), ["broken".to_string()]);
    assert!(cache.take_refused().is_empty());
    assert!(cache.take_wanted().is_empty(), "each count is decoded once");
    fs::remove_dir_all(&root).unwrap();
}

/// A new theme forgets the names it had sent off: asked again, they are wanted again.
#[test]
fn a_new_theme_wants_its_pending_names_again() {
    let root = fixture("test", INDEX, &[]);
    put(&root, "test/scalable/apps/drawn.svg", RED_SVG.as_bytes());
    let theme = || IconTheme::load("test", std::slice::from_ref(&root));
    let mut cache = IconCache::new(theme());
    let old = cache.theme();
    assert_eq!(cache.get_unread("drawn", 32), None);
    assert_eq!(cache.take_wanted().len(), 1);
    cache.set_theme(theme());
    cache.put_icons(&old, vec![("drawn".into(), 32, Some(Canvas::new(32, 32)))]);
    assert_eq!(cache.get_unread("drawn", 32), None);
    assert_eq!(cache.take_wanted(), [("drawn".to_string(), 32)]);
    fs::remove_dir_all(&root).unwrap();
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

/// Adwaita's user-trash-full builds and draws at 96 px, the 48 px desktop at scale 2, for about
/// 77e6 work together; an icon of its shape and cost draws at its size.
#[test]
fn an_icon_costing_what_user_trash_full_does_draws_at_96_px() {
    let svg = trash_shaped(47, 95);
    let (_, built) = parse_xml(&svg, SVG_MAX_WORK).unwrap();
    let tree = resvg::usvg::Tree::from_str(&svg, &Default::default()).unwrap();
    assert!(
        svg_over_budget(&tree, true, 96, 75e6 - built).is_some(),
        "its build and draw pass 75e6 together"
    );
    assert_eq!(svg_side(&tree, true, 96, SVG_MAX_WORK - built), Ok(96));
}

/// An icon shaped as Adwaita's user-trash-full: masks whose content is a recoloured layer, and
/// paths under clip paths, on a 128-unit canvas.
fn trash_shaped(masks: usize, clips: usize) -> String {
    let mut body = String::from(
        r#"<filter id="a" width="100%" height="100%" x="0%" y="0%"><feColorMatrix in="SourceGraphic" values="0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0"/></filter>"#,
    );
    for i in 0..masks {
        body += &format!(
            r##"<mask id="m{i}"><g filter="url(#a)"><rect width="128" height="128" fill-opacity="0.2"/></g></mask><g mask="url(#m{i})"><path d="M20 20h88v88H20z" fill="#3d3846"/></g>"##
        );
    }
    for i in 0..clips {
        body += &format!(
            r##"<clipPath id="c{i}"><rect width="192" height="152"/></clipPath><g clip-path="url(#c{i})"><path d="M30 40h60v50H30z" fill="#9a9996"/></g>"##
        );
    }
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128">{body}</svg>"#)
}
