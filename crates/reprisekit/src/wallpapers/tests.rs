use super::*;

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in &c.pixels {
        for b in v.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn b(s: &str) -> Builtin {
    Builtin::parse(s).unwrap()
}

#[test]
fn every_variant_draws_its_pinned_picture() {
    let got: Vec<_> = Design::ALL
        .iter()
        .flat_map(|d| {
            d.variants()
                .iter()
                .map(move |v| format!("{}/{v}", d.name()))
        })
        .map(|name| {
            let h = fnv(&render(b(&name), 160, 90, SEED));
            (name, h)
        })
        .collect();
    crate::pins::check("wallpapers/variants", &got);
}

#[test]
fn each_design_draws_its_signed_off_picture_at_1080p() {
    let mut got = Vec::new();
    for name in [
        "caustic/sky",
        "reeded/sky",
        "panes/sky",
        "ridges/dawn",
        "tarn/dawn",
        "terraces/spring",
        "fold/teal",
        "contour/teal",
        "flow/night",
    ] {
        let c = render(b(name), 1920, 1080, SEED);
        assert_eq!((c.width, c.height), (1920, 1080));
        got.push((name.to_string(), fnv(&c)));
    }
    crate::pins::check("wallpapers/1080p", &got);
}

#[test]
fn hue_palettes_draw_their_pinned_pictures() {
    let got: Vec<_> = [
        "flow/hue-245",
        "flow/hue-245-dark",
        "caustic/hue-30",
        "reeded/hue-200-dark",
    ]
    .map(|name| (name.to_string(), fnv(&render(b(name), 160, 90, SEED))))
    .into();
    crate::pins::check("wallpapers/hues", &got);
}

#[test]
fn the_picture_is_the_same_at_any_thread_count() {
    // Odd sizes so the bands split unevenly.
    for name in [
        "tarn/night",
        "terraces/dusk",
        "flow/ink",
        "contour/paper",
        "panes/dark",
    ] {
        let one = with_threads(1, || render(b(name), 211, 97, 3));
        for n in [2, 3, 7] {
            let many = with_threads(n, || render(b(name), 211, 97, 3));
            assert!(one == many, "{name} on {n} threads");
        }
    }
}

#[test]
fn another_seed_draws_another_picture() {
    for d in Design::ALL {
        let x = Builtin::named(d, d.variants()[0]).unwrap();
        assert_ne!(
            fnv(&render(x, 96, 54, SEED)),
            fnv(&render(x, 96, 54, SEED + 1)),
            "{x}"
        );
    }
}

#[test]
fn every_picture_is_opaque() {
    let c = render(b("reeded/dark"), 64, 36, SEED);
    assert!(c.pixels.iter().all(|p| p >> 24 == 0xFF));
}

#[test]
fn the_cache_name_changes_with_every_input() {
    let base = cache_name(b("tarn/dusk"), 1920, 1080, 7);
    assert_eq!(base, format!("tarn-dusk-1920x1080-s7-v{VERSION}"));
    let others = [
        cache_name(b("ridges/dusk"), 1920, 1080, 7),
        cache_name(b("tarn/dawn"), 1920, 1080, 7),
        cache_name(b("tarn/dusk"), 1921, 1080, 7),
        cache_name(b("tarn/dusk"), 1920, 1081, 7),
        cache_name(b("tarn/dusk"), 1920, 1080, 8),
        cache_name(b("flow/hue-10"), 1920, 1080, 7),
    ];
    for o in &others {
        assert_ne!(*o, base);
    }
    assert_ne!(
        cache_name(b("flow/hue-10"), 9, 9, 7),
        cache_name(b("flow/hue-10-dark"), 9, 9, 7)
    );
    assert_ne!(
        cache_name(b("flow/hue-10"), 9, 9, 7),
        cache_name(b("flow/hue-11"), 9, 9, 7)
    );
}

#[test]
fn uris_parse_and_print_back() {
    for s in [
        "tarn/dusk",
        "terraces/frost",
        "caustic/hue-0",
        "reeded/hue-359-dark",
        "flow/glass",
        "flow/ember",
    ] {
        let x = Builtin::from_uri(&format!("reprise:{s}")).unwrap().unwrap();
        assert_eq!(x.to_string(), s);
        assert_eq!(x.uri(), format!("reprise:{s}"));
    }
    assert_eq!(b("contour").to_string(), "contour/teal");
    assert!(Builtin::from_uri("/usr/share/backgrounds/a.png").is_none());
}

#[test]
fn bad_names_say_what_is_wrong() {
    let err = |s: &str| Builtin::from_uri(s).unwrap().unwrap_err();
    assert!(err("reprise:lake/dusk").contains("no built-in wallpaper \"lake\""));
    assert!(err("reprise:tarn/noon").contains("tarn has no variant \"noon\""));
    assert!(err("reprise:tarn/noon").contains("dawn, day, dusk, night, blue"));
    // Hues only for the hue-driven designs, and only 0..359.
    assert!(err("reprise:tarn/hue-200").contains("no variant"));
    assert!(err("reprise:tarn/glass").contains("no variant"));
    assert!(err("reprise:flow/hue-360").contains("no variant"));
    assert!(err("reprise:flow/hue-x").contains("hue-<0..359>"));
    assert!(err("reprise:").contains("no built-in wallpaper"));
}

#[test]
fn glass_resolves_to_the_glass_hue() {
    let g = b("caustic/glass");
    assert_eq!(g.resolve(200, false).to_string(), "caustic/hue-200");
    assert_eq!(g.resolve(200, true).to_string(), "caustic/hue-200-dark");
    assert_eq!(b("flow/glass").resolve(200, true).to_string(), "flow/night");
    assert_eq!(
        b("flow/glass").resolve(200, false).to_string(),
        "flow/hue-200"
    );
    assert_eq!(b("tarn/blue").resolve(10, true), b("tarn/blue"));
}
