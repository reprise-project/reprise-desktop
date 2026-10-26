use super::*;

#[test]
fn touchscreen_output_parses() {
    assert_eq!(Config::default().touchscreen.output, None);
    let c = Config::parse("[touchscreen]\noutput = \"DP-2\"\n").unwrap();
    assert_eq!(c.touchscreen.output.as_deref(), Some("DP-2"));
    let c = Config::parse("[touchscreen]\noutput = \"\"\n").unwrap();
    assert_eq!(c.touchscreen.output, None);
    assert!(Config::parse("[touchscreen]\nmap = \"DP-2\"\n").is_err());
}

#[test]
fn tablets_parse_in_order_and_the_first_match_maps() {
    assert!(Config::default().tablets.is_empty());
    let c = Config::parse(
        "[[tablets]]\nname = \"Wacom Intuos*\"\noutput = \"DP-2\"\nkeep_aspect = true\n\n[[tablets]]\noutput = \"\"\n",
    )
    .unwrap();
    assert_eq!(
        c.tablets,
        [
            TabletConfig {
                name: "Wacom Intuos*".into(),
                output: Some("DP-2".into()),
                keep_aspect: true,
            },
            TabletConfig {
                name: "*".into(),
                output: None,
                keep_aspect: false,
            },
        ]
    );
    let entry = |name| find_tablet(&c.tablets, name).map(|t| t.output.as_deref());
    assert_eq!(entry("Wacom Intuos S Pen"), Some(Some("DP-2")));
    assert_eq!(entry("Huion Tablet"), Some(None), "the catch-all entry");
    assert!(Config::parse("[[tablets]]\nmap = \"DP-2\"\n").is_err());
}

#[test]
fn outputs_section_parses_and_validates_scale() {
    let c = Config::parse(
        "[[outputs]]\nname = \"DP-1\"\nscale = 2\n\n[[outputs]]\nname = \"HDMI-A-1\"\nscale = 1\n",
    )
    .unwrap();
    assert_eq!(c.outputs.len(), 2);
    assert_eq!(c.outputs[0].name, "DP-1");
    assert_eq!(c.outputs[0].scale, 2.0);
    assert_eq!(c.scale_for_output_name("DP-1"), 2.0);
    assert_eq!(c.scale_for_output_name("eDP-1"), 1.0);
    assert!(Config::default().outputs.is_empty());
    assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 0\n").is_err());
    assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 5\n").is_err());
    assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 0.5\n").is_err());
    assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\nscale = 4.5\n").is_err());
    let at = |s: &str| {
        Config::parse(&format!("[[outputs]]\nname = \"DP-1\"\nscale = {s}\n"))
            .unwrap()
            .scale_for_output_name("DP-1")
    };
    assert_eq!(at("1.5"), 1.5);
    assert_eq!(at("1.75"), 1.75);
    assert_eq!(at("1.3"), 1.25, "rounded to the nearest quarter");
    assert_eq!(at("3.9"), 4.0);
    assert!(Config::parse("[[outputs]]\nscale = 2\n").is_err());
}

#[test]
fn adaptive_sync_is_global_with_a_per_output_override() {
    let c = Config::default();
    assert!(!c.adaptive_sync_for_output_name("DP-1"));
    let c = Config::parse(
        "adaptive_sync = true\n\n[[outputs]]\nname = \"DP-1\"\nadaptive_sync = false\n\n[[outputs]]\nname = \"DP-2\"\n",
    )
    .unwrap();
    assert!(c.adaptive_sync);
    assert!(!c.adaptive_sync_for_output_name("DP-1"));
    assert!(c.adaptive_sync_for_output_name("DP-2"));
    assert!(c.adaptive_sync_for_output_name("HDMI-A-1"));
    let c = Config::parse("[[outputs]]\nname = \"DP-1\"\nadaptive_sync = true\n").unwrap();
    assert!(c.adaptive_sync_for_output_name("DP-1"));
    assert!(!c.adaptive_sync_for_output_name("DP-2"));
}

#[test]
fn output_position_and_primary_parse() {
    let c = Config::parse(
        "[[outputs]]\nname = \"DP-1\"\nposition = [-1280, 56]\n\n\
         [[outputs]]\nname = \"HDMI-A-1\"\nprimary = true\n\n\
         [[outputs]]\nname = \"DP-2\"\nprimary = true\n",
    )
    .unwrap();
    assert_eq!(c.position_for_output_name("DP-1"), Some((-1280, 56)));
    assert_eq!(c.position_for_output_name("HDMI-A-1"), None);
    assert_eq!(c.primary_output_name(), Some("HDMI-A-1"), "the first wins");
    assert_eq!(Config::parse("").unwrap().primary_output_name(), None);
    let far = Config::parse("[[outputs]]\nname = \"DP-1\"\nposition = [0, 99999999]\n");
    assert!(far.unwrap_err().contains("position must be within"));
}

#[test]
fn output_transform_parses_by_wlr_randr_name() {
    let c = Config::parse("[[outputs]]\nname = \"DP-1\"\ntransform = \"90\"\n").unwrap();
    assert_eq!(c.transform_for_output_name("DP-1"), Transform::Rotate90);
    assert_eq!(c.transform_for_output_name("DP-2"), Transform::Normal);
    for (name, t) in [
        ("normal", Transform::Normal),
        ("180", Transform::Rotate180),
        ("270", Transform::Rotate270),
        ("flipped", Transform::Flipped),
        ("flipped-90", Transform::Flipped90),
        ("flipped-180", Transform::Flipped180),
        ("flipped-270", Transform::Flipped270),
    ] {
        assert_eq!(Transform::parse(name), Some(t), "{name}");
        assert_eq!(t.name(), name);
    }
    assert!(Config::parse("[[outputs]]\nname = \"DP-1\"\ntransform = \"45\"\n").is_err());
}

#[test]
fn an_output_mode_prints_the_spelling_it_parses_from() {
    let mode = |width, height, refresh_mhz| OutputMode {
        width,
        height,
        refresh_mhz,
    };
    for (mode, text) in [
        (mode(1920, 1080, Some(60_000)), "1920x1080@60"),
        (mode(1920, 1080, Some(59_940)), "1920x1080@59.94"),
        (mode(2560, 1440, Some(74_973)), "2560x1440@74.973"),
        (mode(1280, 800, None), "1280x800"),
    ] {
        assert_eq!(mode.to_string(), text);
        assert_eq!(OutputMode::parse(text), Ok(mode), "{text} parses back");
    }
}

#[test]
fn output_mode_parses_and_picks() {
    let c = Config::parse("[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080@59.94\"\n").unwrap();
    let m = c.mode_for_output_name("DP-1").unwrap();
    assert_eq!(
        (m.width, m.height, m.refresh_mhz),
        (1920, 1080, Some(59940))
    );
    assert_eq!(c.mode_for_output_name("DP-2"), None);
    assert_eq!(c.outputs[0].scale, 1.0);
    for bad in [
        "1920",
        "0x1080",
        "1920x1080@0",
        "axb",
        "1920x1080@fast",
        "1920x1080@inf",
        "1920x1080@1e9",
    ] {
        let text = format!("[[outputs]]\nname = \"DP-1\"\nmode = \"{bad}\"\n");
        assert!(Config::parse(&text).is_err(), "{bad}");
    }

    let offered = |width, height, refresh_mhz, preferred| OfferedMode {
        width,
        height,
        refresh_mhz,
        preferred,
    };
    let modes = [
        offered(2560, 1440, 144_000, true),
        offered(1920, 1080, 60_000, false),
        offered(1920, 1080, 120_000, false),
        offered(1920, 1080, 59_940, false),
    ];
    let any = OutputMode::parse("1920x1080").unwrap();
    assert_eq!(
        any.pick(&modes),
        Some(2),
        "highest refresh without a request"
    );
    assert_eq!(m.pick(&modes), Some(3), "closest refresh to the request");
    assert_eq!(
        OutputMode::parse("1920x1080@75").unwrap().pick(&modes),
        Some(1)
    );
    assert_eq!(OutputMode::parse("1280x720").unwrap().pick(&modes), None);
    let pref = [
        offered(1920, 1080, 120_000, false),
        offered(1920, 1080, 60_000, true),
    ];
    assert_eq!(any.pick(&pref), Some(1));
}
