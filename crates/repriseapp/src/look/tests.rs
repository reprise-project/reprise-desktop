use super::*;

#[test]
fn theme_from_env_with_no_interface_is_the_default_theme() {
    assert_eq!(
        theme_from_env(None, None, None, None, None),
        Theme::default(),
        "no REPRISE_INTERFACE and no REPRISE_COLORS must give Theme::default()"
    );
}

#[test]
fn theme_from_env_with_blue_takes_the_blue_style() {
    let t = theme_from_env(Some("blue"), None, None, None, None);
    let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    assert_eq!(
        t.colors, blue.colors,
        "blue interface with no REPRISE_COLORS must keep Blue's own colours"
    );
    assert!(
        matches!(t.look, reprisekit::look::Look::Styled(_)),
        "blue interface must draw through the styled look"
    );
}

#[test]
fn theme_from_env_picks_the_blue_variant() {
    use reprisekit::blue::Variant;
    use reprisekit::interfaces::blue_theme;
    for v in Variant::ALL {
        let t = theme_from_env(Some("blue"), Some(v.as_str()), None, None, None);
        assert_eq!(t.look, blue_theme(v).look, "{v:?}");
        assert_eq!(t.colors, blue_theme(v).colors, "{v:?}");
    }
    for unknown in [None, Some("green"), Some("Silver")] {
        assert_eq!(
            theme_from_env(Some("blue"), unknown, None, None, None).look,
            blue_theme(Variant::Blue).look,
            "{unknown:?} is navy"
        );
    }
    assert_eq!(
        theme_from_env(Some("classic"), Some("olive"), None, None, None),
        Theme::default(),
        "the variant means nothing outside Blue"
    );
}

#[test]
fn the_glass_mode_picks_dark_glass_and_anything_else_is_light() {
    let dark = theme_from_env(Some("glass"), None, Some("dark"), None, None);
    let light = theme_from_env(Some("glass"), None, None, None, None);
    assert_ne!(dark.look, light.look, "dark Glass is another look");
    assert_eq!(dark.look.style().unwrap().name, "glass-dark");
    for odd in [Some("Dark"), Some(""), Some("dim")] {
        assert_eq!(
            theme_from_env(Some("glass"), None, odd, None, None).look,
            light.look,
            "{odd:?} is light"
        );
    }
    assert_eq!(
        theme_from_env(Some("classic"), None, Some("dark"), None, None).look,
        theme_from_env(Some("classic"), None, None, None, None).look,
        "Classic ignores the glass mode"
    );
}

#[test]
fn smoked_glass_inks_labels_on_its_glass_white_in_either_mode() {
    for mode in ["light", "dark"] {
        let theme = theme_from_env(Some("glass"), None, Some(mode), Some("smoked"), None);
        assert_eq!(
            reprisekit::look::ink_on_glass(&theme),
            Some(reprisekit::theme::Color::rgb(0xFF, 0xFF, 0xFF)),
            "{mode}"
        );
    }
    let frosted = theme_from_env(Some("glass"), None, Some("light"), None, None);
    assert_eq!(
        reprisekit::look::ink_on_glass(&frosted),
        Some(reprisekit::theme::Color::rgb(0, 0, 0)),
        "light Frosted inks black"
    );
}

#[test]
fn a_glass_material_change_through_the_portal_relooks() {
    let mut look = Look {
        interface: Some("glass".into()),
        ..Look::default()
    };
    let ink = |l: &Look| reprisekit::look::ink_on_glass(&l.theme());
    let before = ink(&look);
    assert!(look.set(reprisebus::appearance::GLASS_MATERIAL, "smoked".into()));
    assert_ne!(ink(&look), before, "the ink on glass follows");
}

#[test]
fn a_glass_mode_change_through_the_portal_relooks() {
    let mut look = Look {
        interface: Some("glass".into()),
        ..Look::default()
    };
    let before = look.theme().look;
    assert!(
        look.set(reprisebus::appearance::GLASS_MODE, "dark".into()),
        "the change is seen"
    );
    assert_ne!(look.theme().look, before, "and the look moves");
    assert!(
        !look.set(reprisebus::appearance::GLASS_MODE, "dark".into()),
        "the same value again is no change"
    );
}

#[test]
fn a_new_blue_variant_rebuilds_the_theme() {
    use reprisebus::appearance as a;
    let mut look = Look {
        interface: Some("blue".into()),
        ..Look::default()
    };
    assert!(look.set(a::BLUE_VARIANT, "silver".into()));
    assert!(!look.set(a::BLUE_VARIANT, "silver".into()));
    assert_eq!(
        look.theme().look,
        reprisekit::interfaces::blue_theme(reprisekit::blue::Variant::Silver).look
    );
}

#[test]
fn a_new_second_dialog_font_reaches_the_themes_fonts() {
    use reprisebus::appearance as a;
    let mut look = Look::default();
    assert!(look.set(a::SHELL_DLG, "Tahoma".into()));
    assert!(!look.set(a::SHELL_DLG, "Tahoma".into()));
    assert_eq!(look.theme().fonts.shell_dlg.as_deref(), Some("Tahoma"));
    assert!(look.set(a::SHELL_DLG, String::new()), "unset is a change");
    assert_eq!(look.theme().fonts.shell_dlg, None);
}

#[test]
fn the_same_fonts_share_one_renderer_and_other_fonts_load_another() {
    let fonts = reprisekit::theme::FontConfig::default();
    assert!(Rc::ptr_eq(&text_for(&fonts), &text_for(&fonts)));
    let other = reprisekit::theme::FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..fonts.clone()
    };
    let changed = text_for(&other);
    assert!(Rc::ptr_eq(&changed, &text_for(&other)));
    assert!(!Rc::ptr_eq(&changed, &text_for(&fonts)));
}

#[test]
fn the_configured_font_family_reaches_the_themes_fonts() {
    use reprisebus::appearance as a;
    let mut look = Look::default();
    let default = look.theme().fonts.family;
    assert!(look.set(a::FONT_FAMILY, "DejaVu Sans, Noto Sans".into()));
    assert!(!look.set(a::FONT_FAMILY, "DejaVu Sans,Noto Sans".into()));
    assert_eq!(look.theme().fonts.family, ["DejaVu Sans", "Noto Sans"]);
    assert!(look.set(a::FONT_FAMILY, String::new()), "unset is a change");
    assert_eq!(look.theme().fonts.family, default);
}

#[test]
fn the_configured_antialias_reaches_the_themes_fonts() {
    use reprisebus::appearance as a;
    let mut look = Look::default();
    assert!(!look.theme().fonts.antialias);
    assert!(look.set(a::FONT_ANTIALIAS, "true".into()));
    assert!(!look.set(a::FONT_ANTIALIAS, "true".into()));
    assert!(!look.set(a::FONT_ANTIALIAS, "smooth".into()));
    assert!(look.theme().fonts.antialias);
    assert!(look.set(a::FONT_ANTIALIAS, "false".into()));
    assert!(!look.theme().fonts.antialias);
}

#[test]
fn the_configured_double_click_time_reaches_the_themes_metrics() {
    use reprisebus::appearance as a;
    let mut look = Look::default();
    assert_eq!(look.theme().metrics.double_click_ms, 500);
    assert!(look.set(a::DOUBLE_CLICK_MS, "900".into()));
    assert!(!look.set(a::DOUBLE_CLICK_MS, "900".into()));
    assert!(!look.set(a::DOUBLE_CLICK_MS, "slow".into()));
    assert_eq!(look.theme().metrics.double_click_ms, 900);
}

#[test]
fn a_new_colour_scheme_rebuilds_the_theme_over_the_same_interface() {
    use reprisebus::appearance as a;
    let tokyo = reprisekit::schemes::scheme("Tokyo Night").unwrap();
    let mut look = Look {
        interface: Some("classic".into()),
        ..Look::default()
    };
    assert!(
        look.set(a::COLORS, tokyo.to_env()),
        "a first scheme is a change"
    );
    assert!(
        !look.set(a::COLORS, tokyo.to_env()),
        "the same scheme again is none"
    );
    assert!(!look.set("font", "Tahoma".into()), "an unknown key is none");
    assert_eq!(
        look.theme().colors,
        tokyo,
        "the theme takes Tokyo Night's colours"
    );
    assert!(look.set(a::ICON_THEME, "Papirus".into()));
    assert_eq!(look.icon_theme, "Papirus");
}

#[test]
fn theme_from_env_applies_colors_over_the_blue_style() {
    let t = theme_from_env(Some("blue"), None, None, None, Some("window_text=#010203"));
    let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    assert_eq!(
        t.colors.window_text,
        reprisekit::theme::Color::rgb(1, 2, 3),
        "the REPRISE_COLORS pair must win over Blue's own colour"
    );
    assert_eq!(
        t.colors.button_face, blue.colors.button_face,
        "a colour not named in REPRISE_COLORS must stay Blue's own"
    );
}
