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

const PINS: [(&str, u64); 42] = [
    ("caustic/sky", 0xaa50794ea63ca0f1),
    ("caustic/ruby", 0x5ef14c16a8415b67),
    ("caustic/green", 0xcacf251443163ad0),
    ("caustic/violet", 0xe0f8d9b9af8ea08b),
    ("caustic/dark", 0xb17b0be7f2b80d59),
    ("reeded/sky", 0x541975fbc7e24ff4),
    ("reeded/ruby", 0x13217a0160c62682),
    ("reeded/green", 0x717d7efdfd474e66),
    ("reeded/violet", 0x5f93d9582165edf8),
    ("reeded/dark", 0xba646db776ca3a69),
    ("panes/sky", 0x9642c3028b050a08),
    ("panes/ruby", 0x9b2ecba4808e6d98),
    ("panes/green", 0x7a94c71900db39f5),
    ("panes/violet", 0x094c925e8cb9d594),
    ("panes/dark", 0x1d4afa86096c3ebd),
    ("ridges/dawn", 0xb1a9e0ccc909a2ff),
    ("ridges/day", 0xfdc689a97a688cdb),
    ("ridges/dusk", 0xa932d676a948ec25),
    ("ridges/night", 0x664d877e46e0ae6c),
    ("ridges/blue", 0xe508d9d7be3cef14),
    ("tarn/dawn", 0xae078ab3ad672817),
    ("tarn/day", 0xca4aefcc3b41fdd4),
    ("tarn/dusk", 0x8a9d4ebc6dc50c70),
    ("tarn/night", 0xe76bec5490a38db6),
    ("tarn/blue", 0x776945ea0184ac1b),
    ("terraces/spring", 0xb9d126cac126942b),
    ("terraces/harvest", 0xc65242db0f01f7ba),
    ("terraces/dusk", 0x3f12307bab28f618),
    ("terraces/frost", 0x3f20cdbf47b491ce),
    ("terraces/blue", 0x9d2e68094781f9b3),
    ("fold/teal", 0x4b8253ff2ff184b7),
    ("fold/steel", 0x2e6b34e4d331dc29),
    ("fold/sand", 0x95bfbd96d2aaacfe),
    ("fold/night", 0x5267ccc1f1dbdd60),
    ("contour/teal", 0x4e0f7de2d76acb77),
    ("contour/night", 0x411994f312246830),
    ("contour/paper", 0x1697e1f05947a4ef),
    ("contour/dusk", 0x8e54df4c4de2e0d2),
    ("flow/night", 0x99b7a38fb96b9711),
    ("flow/ink", 0x95e18685d7c46371),
    ("flow/teal", 0xe65f26eb570578ee),
    ("flow/ember", 0xe6540d5213375efb),
];

#[test]
fn every_variant_draws_its_pinned_picture() {
    let mut wrong = Vec::new();
    for (name, want) in PINS {
        let got = fnv(&render(b(name), 160, 90, SEED));
        if got != want {
            wrong.push(format!("{name}: {got:#018x}, expected {want:#018x}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_pins_cover_every_design_and_variant() {
    let all: Vec<String> = Design::ALL
        .iter()
        .flat_map(|d| {
            d.variants()
                .iter()
                .map(move |v| format!("{}/{v}", d.name()))
        })
        .collect();
    assert_eq!(all.len(), PINS.len());
    for name in &all {
        assert!(PINS.iter().any(|p| p.0 == name), "{name} has no pin");
    }
}

#[test]
fn each_design_draws_its_signed_off_picture_at_1080p() {
    let want = [
        ("caustic/sky", 0x6a10b93c0c94c5f2),
        ("reeded/sky", 0x9825183588f6780c),
        ("panes/sky", 0x230764c52c9042e7),
        ("ridges/dawn", 0x05349e7b5b04946b),
        ("tarn/dawn", 0xcd095a6e17f07dde),
        ("terraces/spring", 0x04c748eb458362dc),
        ("fold/teal", 0x38bc263e8a421920),
        ("contour/teal", 0x310fa380246209c4),
        ("flow/night", 0x4c59ada6f3683658),
    ];
    for (name, want) in want {
        let c = render(b(name), 1920, 1080, SEED);
        assert_eq!((c.width, c.height), (1920, 1080));
        assert_eq!(fnv(&c), want, "{name} at 1920x1080");
    }
}

#[test]
fn hue_palettes_draw_their_pinned_pictures() {
    for (name, want) in [
        ("flow/hue-245", 0xbc4180baca3afc8fu64),
        ("flow/hue-245-dark", 0x3cdf59f6ec44a54a),
        ("caustic/hue-30", 0xe5647a12d561ad81),
        ("reeded/hue-200-dark", 0xca78d1409df09243),
    ] {
        let got = fnv(&render(b(name), 160, 90, SEED));
        assert_eq!(got, want, "{name}: {got:#018x}");
    }
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
