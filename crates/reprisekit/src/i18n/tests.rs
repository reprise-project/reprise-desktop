use super::*;

fn lang(s: &str) -> LanguageIdentifier {
    s.parse().unwrap()
}

fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |n| {
        vars.iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| v.to_string())
    }
}

const EN: &str = "
greeting = Hello, { $name }
file-menu = &File
cores = { $n ->
    [one] { $n } core
   *[other] { $n } cores
}
memory = { NUMBER($kb) } KB
ghz = up to { NUMBER($ghz, minimumFractionDigits: 2, maximumFractionDigits: 2) } GHz
";

const DE: &str = "
file-menu = &Datei
cores = { $n ->
    [one] { $n } Kern
   *[other] { $n } Kerne
}
broken = { $missing
";

#[test]
fn a_translation_wins_and_a_missing_id_falls_back_to_english() {
    let c = Catalog::from_texts(&[(lang("de"), DE), (lang("en"), EN)]);
    assert_eq!(c.tr_args("file-menu", &[]), "&Datei", "the & survives");
    assert_eq!(c.tr_args("greeting", &[("name", "Jo".into())]), "Hello, Jo");
    assert_eq!(c.tr_args("no-such-id", &[]), "no-such-id");
}

#[test]
fn plurals_follow_the_language_and_no_isolation_marks_are_added() {
    let c = Catalog::from_texts(&[(lang("de"), DE), (lang("en"), EN)]);
    assert_eq!(c.tr_args("cores", &[("n", 1.into())]), "1 Kern");
    assert_eq!(c.tr_args("cores", &[("n", 14.into())]), "14 Kerne");
    let en = Catalog::english(EN);
    assert_eq!(en.tr_args("cores", &[("n", 1.into())]), "1 core");
}

#[test]
fn numbers_group_and_take_their_fraction_digits() {
    let en = Separators {
        group: ",".into(),
        decimal: ".".into(),
    };
    let de = Separators {
        group: ".".into(),
        decimal: ",".into(),
    };
    let opts = FluentNumberOptions::default();
    assert_eq!(format_number(16_777_216.0, &opts, &en), "16,777,216");
    assert_eq!(format_number(16_777_216.0, &opts, &de), "16.777.216");
    assert_eq!(format_number(999.0, &opts, &en), "999");
    assert_eq!(format_number(-1234.5, &opts, &en), "-1,234.5");
    let two = FluentNumberOptions {
        minimum_fraction_digits: Some(2),
        maximum_fraction_digits: Some(2),
        ..Default::default()
    };
    assert_eq!(format_number(4.9, &two, &de), "4,90");
    let flat = FluentNumberOptions {
        use_grouping: false,
        ..Default::default()
    };
    assert_eq!(format_number(2026.0, &flat, &en), "2026");
}

#[test]
fn number_placeables_use_reprisekits_english_separators() {
    let c = Catalog::english(EN);
    assert_eq!(
        c.tr_args("memory", &[("kb", 16_384_000u64.into())]),
        "16,384,000 KB"
    );
    assert_eq!(c.tr_args("ghz", &[("ghz", 4.9.into())]), "up to 4.90 GHz");
}

#[test]
fn a_number_in_the_separator_strings_formats_without_the_separators() {
    let c = Catalog::english(
        "number-group-separator = { 1000 }\nnumber-decimal-separator = { \"\\u202F\" }",
    );
    let (group, decimal) = reading_separators(|| {
        (
            c.tr_args("number-group-separator", &[]),
            c.tr_args("number-decimal-separator", &[]),
        )
    });
    assert_eq!(group, "1000");
    assert_eq!(decimal, "\u{202F}");
}

#[test]
fn a_translation_that_fails_to_format_falls_back_to_english() {
    let c = Catalog::from_texts(&[
        (lang("de"), "greeting = Hallo, { $nobody }"),
        (lang("en"), EN),
    ]);
    assert_eq!(c.tr_args("greeting", &[("name", "Jo".into())]), "Hello, Jo");
}

#[test]
fn the_locale_comes_from_language_then_the_message_locale() {
    let got = requested(env_of(&[("LANGUAGE", "fr:de"), ("LANG", "pt_BR.UTF-8")]));
    assert_eq!(got, [lang("fr"), lang("de"), lang("pt-BR")]);
    let got = requested(env_of(&[
        ("LC_MESSAGES", "de_AT@euro"),
        ("LANG", "en_US.UTF-8"),
    ]));
    assert_eq!(got, [lang("de-AT")]);
    let got = requested(env_of(&[("LC_ALL", "C.UTF-8"), ("LANGUAGE", "de")]));
    assert_eq!(got, [lang("en")], "a C locale silences LANGUAGE");
    assert!(requested(env_of(&[])).is_empty());
}

#[test]
fn installed_files_load_for_the_negotiated_language_user_dir_first() {
    let root = std::env::temp_dir().join(format!("reprise-i18n-{}", std::process::id()));
    let user = root.join("user");
    let system = root.join("system");
    for (dir, text) in [(&user, "file-menu = &Benutzer\n"), (&system, DE)] {
        std::fs::create_dir_all(dir.join("de")).unwrap();
        std::fs::write(dir.join("de/demo.ftl"), text).unwrap();
    }
    std::fs::create_dir_all(system.join("fr")).unwrap();
    std::fs::write(system.join("fr/other.ftl"), "file-menu = &Fichier\n").unwrap();
    let dirs = [user, system];

    let c = Catalog::load("demo", EN, &[lang("de-DE")], &dirs);
    assert_eq!(c.tr_args("file-menu", &[]), "&Benutzer");
    assert_eq!(c.tr_args("cores", &[("n", 2.into())]), "2 Kerne");
    let c = Catalog::load("demo", EN, &[lang("fr")], &dirs);
    assert_eq!(c.tr_args("file-menu", &[]), "&File", "fr has no demo.ftl");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn reprisekits_strings_resolve_in_english_under_test() {
    assert_eq!(tr!("ok"), "OK");
    assert_eq!(tr!("edit-select-all"), "Select &All");
}
