//! The Region & Language page: this account's language and formats, saved to its locale.conf on
//! Apply and taken at the next log on, and the system's, set through localed at once.

use crate::pages::{Area, GX, X, group};
use reprisefs::region::{Choice, Sample};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;

pub const LANGUAGE: &str = "region_language";
pub const FORMATS: &str = "region_formats";
pub const SYSTEM: &str = "region_system";
const SYSTEM_TEXT: &str = "region_system_text";
const SYSTEM_NOTE: &str = "region_system_note";

/// Each sample row's label id, value id and label.
const SAMPLE_ROWS: [(&str, &str, &str); 6] = [
    ("region_date_label", "region_date", "Date:"),
    ("region_time_label", "region_time", "Time:"),
    ("region_number_label", "region_number", "Numbers:"),
    ("region_currency_label", "region_currency", "Currency:"),
    ("region_measure_label", "region_measure", "Measures:"),
    ("region_paper_label", "region_paper", "Paper:"),
];

/// The Formats combo's first row: the language's own formats.
pub const SAME: &str = "Same as the language";
pub const NEXT_LOGON: &str = "Changes take effect the next time you log on.";
pub const ASK_ADMIN: &str = "Ask an administrator to change the system's language.";
pub const NO_LOCALED: &str =
    "The system's language cannot be read: systemd-localed does not answer.";
pub const SETTING: &str = "Changing the system's language...";

const LABEL_W: i32 = 80;
const VALUE_X: i32 = GX + LABEL_W + 4;
const COMBO_W: i32 = 250;
const SAMPLE_Y: i32 = 138;
const SAMPLE_DY: i32 = 18;
const SYSTEM_Y: i32 = 316;
const DROP_ROWS: usize = 12;

/// The page before the locales are read: the combos greyed until [`show_choice`].
pub fn page(a: Area) -> Form {
    let value_w = a.gright() - VALUE_X;
    let mut items = vec![
        group(a, "region_account_group", 44, 84, "Language && Formats"),
        Item::label_at("region_language_label", GX, 70, LABEL_W, "&Language:"),
        Item::combo(LANGUAGE, VALUE_X, 66, a.combo(COMBO_W), &[], DROP_ROWS),
        Item::label_at("region_formats_label", GX, 100, LABEL_W, "&Formats:"),
        Item::combo(FORMATS, VALUE_X, 96, a.combo(COMBO_W), &[SAME], DROP_ROWS),
        group(
            a,
            "region_sample_group",
            SAMPLE_Y,
            SAMPLE_ROWS.len() as i32 * SAMPLE_DY + 30,
            "Sample",
        ),
    ];
    for (i, (label_id, id, label)) in SAMPLE_ROWS.into_iter().enumerate() {
        let y = SAMPLE_Y + 22 + i as i32 * SAMPLE_DY;
        items.push(Item::label_at(label_id, GX, y, LABEL_W, label));
        items.push(Item::label_at(id, VALUE_X, y, value_w, ""));
    }
    let note_y = SAMPLE_Y + SAMPLE_ROWS.len() as i32 * SAMPLE_DY + 40;
    items.extend([
        Item::lines_at("region_note", X, note_y, a.w, &[NEXT_LOGON]),
        group(
            a,
            "region_system_group",
            SYSTEM_Y,
            112,
            "Login Screen && New Users",
        ),
        Item::new(
            SYSTEM_TEXT,
            Rect::new(GX, SYSTEM_Y + 22, a.gw(), 2 * LINE_H),
            Control::Lines(Vec::new()),
        ),
        Item::button_at(
            SYSTEM,
            GX,
            SYSTEM_Y + 52,
            170,
            "Apply to the &System...",
            false,
        ),
        Item::new(
            SYSTEM_NOTE,
            Rect::new(GX, SYSTEM_Y + 82, a.gw(), 2 * LINE_H),
            Control::GreyedLines(Vec::new()),
        ),
    ]);
    let mut page = Form::new(items);
    for id in [LANGUAGE, FORMATS] {
        page.set_enabled(id, false);
    }
    page
}

/// The locales the combos offer, as (locale, name) sorted by name. A name two locales share
/// gets the locale after it.
pub fn named(locales: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = locales
        .iter()
        .map(|(code, name)| {
            let shared = locales.iter().filter(|(_, n)| n == name).count() > 1;
            match shared {
                true => (code.clone(), format!("{name} - {code}")),
                false => (code.clone(), name.clone()),
            }
        })
        .collect();
    v.sort_by_key(|(_, name)| name.to_lowercase());
    v
}

fn row_of(locales: &[(String, String)], code: &str) -> Option<usize> {
    locales.iter().position(|(c, _)| c == code)
}

/// The name `code` has among `locales`, else the locale itself.
pub fn name_of<'a>(locales: &'a [(String, String)], code: &'a str) -> &'a str {
    row_of(locales, code).map_or(code, |i| &locales[i].1)
}

/// Fill the combos from `locales` with `want` picked, and the sample for its formats at `now`.
pub fn show_choice(page: &mut Form, locales: &[(String, String)], want: &Choice, now: i64) {
    let names: Vec<String> = locales.iter().map(|(_, n)| n.clone()).collect();
    let language = row_of(locales, &want.language).unwrap_or(0);
    page.set_combo_rows(LANGUAGE, &names, language, DROP_ROWS);
    let formats_rows: Vec<String> = std::iter::once(SAME.to_string()).chain(names).collect();
    let formats = match &want.formats {
        Some(f) => row_of(locales, f).map_or(0, |i| i + 1),
        None => 0,
    };
    page.set_combo_rows(FORMATS, &formats_rows, formats, DROP_ROWS);
    for id in [LANGUAGE, FORMATS] {
        page.set_enabled(id, !locales.is_empty());
    }
    show_sample(
        page,
        reprisefs::region::sample(want.formats_locale(), now).as_ref(),
    );
}

/// The choice the combos hold.
pub fn read_choice(page: &Form, locales: &[(String, String)]) -> Option<Choice> {
    let language = locales.get(page.combo_selected(LANGUAGE)?)?.0.clone();
    let formats = match page.combo_selected(FORMATS)? {
        0 => None,
        i => Some(locales.get(i - 1)?.0.clone()),
    };
    Some(Choice {
        formats: formats.filter(|f| *f != language),
        language,
    })
}

fn show_sample(page: &mut Form, s: Option<&Sample>) {
    let texts = match s {
        Some(s) => [
            s.date.clone(),
            s.time.clone(),
            s.number.clone(),
            s.currency.clone(),
            measures(s.metric).into(),
            paper(s.paper),
        ],
        None => Default::default(),
    };
    for ((_, id, _), text) in SAMPLE_ROWS.iter().zip(texts) {
        page.set_text(id, &text);
    }
}

fn measures(metric: bool) -> &'static str {
    match metric {
        true => "Metric",
        false => "U.S. customary",
    }
}

/// A paper size `(w, h)` in mm by its name where it has a common one.
fn paper((w, h): (u32, u32)) -> String {
    match (w, h) {
        (210, 297) => "A4".into(),
        (216, 279) => "Letter".into(),
        (216, 356) => "Legal".into(),
        _ => format!("{w} x {h} mm"),
    }
}

/// What the page knows of the system's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum System {
    /// localed's locale, and whether polkit would let this account change it.
    Read {
        locale: Vec<(String, String)>,
        may_set: bool,
    },
    /// localed does not answer.
    Unread,
}

/// Show the system's language and formats, Apply to the System lit when it may set `want` and
/// it differs; `busy` while a change is under way.
pub fn show_system(
    page: &mut Form,
    system: &System,
    locales: &[(String, String)],
    want: Option<&Choice>,
    busy: bool,
) {
    let (lines, on, note) = match system {
        System::Unread => (Vec::new(), false, NO_LOCALED),
        System::Read { locale, may_set } => {
            let now = reprisefs::region::Choice::of(locale);
            let lines = match &now {
                Some(c) => vec![
                    format!("Language: {}", name_of(locales, &c.language)),
                    format!(
                        "Formats: {}",
                        c.formats.as_deref().map_or(SAME, |f| name_of(locales, f))
                    ),
                ],
                None => vec!["Language: not set".into()],
            };
            let differs = want.is_some_and(|w| now.as_ref() != Some(w));
            let note = match (*may_set, busy) {
                (false, _) => ASK_ADMIN,
                (true, true) => SETTING,
                (true, false) => "",
            };
            (lines, *may_set && differs && !busy, note)
        }
    };
    page.set_lines(SYSTEM_TEXT, &lines);
    page.set_enabled(SYSTEM, on);
    page.set_text(SYSTEM_NOTE, note);
}

/// The page at `a` with long names, the widest sample texts and the longest note, for the fit
/// checks.
#[cfg(test)]
pub fn filled(a: Area) -> Form {
    let mut p = page(a);
    let locales = named(vec![
        (
            "es_DO.UTF-8".into(),
            "español (República Dominicana)".into(),
        ),
        ("en_US.UTF-8".into(), "English (United States)".into()),
    ]);
    let want = Choice {
        language: "es_DO.UTF-8".into(),
        formats: Some("en_US.UTF-8".into()),
    };
    show_choice(&mut p, &locales, &want, 0);
    let widest = [
        "Donnerstag, 30.09.2026",
        "07:54:40 PM",
        "1 234 567,89",
        "1 234 567,89 USD",
        measures(false),
        "182 x 257 mm",
    ];
    for ((_, id, _), text) in SAMPLE_ROWS.iter().zip(widest) {
        p.set_text(id, text);
    }
    let system = System::Read {
        locale: vec![
            ("LANG".into(), "es_DO.UTF-8".into()),
            ("LC_TIME".into(), "es_DO.UTF-8".into()),
        ],
        may_set: true,
    };
    show_system(&mut p, &system, &locales, Some(&want), false);
    p.set_text(SYSTEM_NOTE, NO_LOCALED);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(p: &Form, id: &str) -> bool {
        match p.item(id).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) => *enabled,
            Some(Control::Combo(c)) => c.enabled,
            _ => panic!("{id}"),
        }
    }

    fn locales() -> Vec<(String, String)> {
        named(vec![
            ("en_US.UTF-8".into(), "English (United States)".into()),
            ("de_DE.UTF-8".into(), "Deutsch (Deutschland)".into()),
            ("ca_ES.UTF-8".into(), "català (Espanya)".into()),
            ("ca_ES.UTF-8@valencia".into(), "català (Espanya)".into()),
        ])
    }

    #[test]
    fn locales_sort_by_name_and_a_shared_name_shows_its_locale() {
        let l = locales();
        let names: Vec<&str> = l.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "català (Espanya) - ca_ES.UTF-8",
                "català (Espanya) - ca_ES.UTF-8@valencia",
                "Deutsch (Deutschland)",
                "English (United States)",
            ]
        );
    }

    #[test]
    fn the_combos_show_and_read_back_the_choice() {
        let l = locales();
        let mut p = page(Area::MIN);
        assert_eq!(
            read_choice(&p, &[]),
            None,
            "nothing to pick before the read"
        );
        let want = Choice {
            language: "de_DE.UTF-8".into(),
            formats: Some("en_US.UTF-8".into()),
        };
        show_choice(&mut p, &l, &want, 0);
        assert_eq!(p.combo_selected(LANGUAGE), Some(2));
        assert_eq!(p.combo_selected(FORMATS), Some(4));
        assert_eq!(read_choice(&p, &l), Some(want));
        p.set_combo(FORMATS, 0);
        assert_eq!(
            read_choice(&p, &l).unwrap().formats,
            None,
            "the first row is the language's own"
        );
        p.set_combo(FORMATS, 3);
        assert_eq!(
            read_choice(&p, &l).unwrap().formats,
            None,
            "the language's own locale picked as formats is the same"
        );
    }

    #[test]
    fn the_sample_names_measures_and_paper() {
        assert_eq!(paper((210, 297)), "A4");
        assert_eq!(paper((216, 279)), "Letter");
        assert_eq!(paper((182, 257)), "182 x 257 mm");
        assert_eq!(measures(false), "U.S. customary");
        let mut p = page(Area::MIN);
        let c = Choice {
            language: "C".into(),
            formats: None,
        };
        show_choice(&mut p, &[], &c, 0);
        let number = p.item("region_number").map(|i| match &i.control {
            Control::Label { text, .. } => text.clone(),
            _ => String::new(),
        });
        assert_eq!(number.as_deref(), Some("1234567.89"));
        assert!(!lit(&p, LANGUAGE), "no locale to pick");
    }

    #[test]
    fn apply_to_the_system_lights_only_for_a_different_allowed_choice() {
        let l = locales();
        let mut p = page(Area::MIN);
        let want = Choice {
            language: "de_DE.UTF-8".into(),
            formats: None,
        };
        let read = |lang: &str, may_set| System::Read {
            locale: vec![("LANG".into(), lang.into())],
            may_set,
        };
        let lit = |p: &Form| lit(p, SYSTEM);
        let note = |p: &Form| match p.item(SYSTEM_NOTE).map(|i| &i.control) {
            Some(Control::GreyedLines(l)) => l.join(" "),
            _ => String::new(),
        };
        show_system(&mut p, &read("en_US.UTF-8", true), &l, Some(&want), false);
        assert!(lit(&p));
        assert_eq!(note(&p), "");
        show_system(&mut p, &read("de_DE.UTF-8", true), &l, Some(&want), false);
        assert!(!lit(&p), "the system already has it");
        show_system(&mut p, &read("en_US.UTF-8", false), &l, Some(&want), false);
        assert!(!lit(&p));
        assert_eq!(note(&p), ASK_ADMIN);
        show_system(&mut p, &read("en_US.UTF-8", true), &l, Some(&want), true);
        assert!(!lit(&p), "greyed while a change runs");
        assert_eq!(note(&p), SETTING);
        show_system(&mut p, &System::Unread, &l, Some(&want), false);
        assert!(!lit(&p));
        assert_eq!(note(&p), NO_LOCALED);
        let text = match p.item(SYSTEM_TEXT).map(|i| &i.control) {
            Some(Control::Lines(l)) => l.clone(),
            _ => Vec::new(),
        };
        assert!(text.is_empty());
        show_system(&mut p, &read("de_DE.UTF-8", true), &l, Some(&want), false);
        let text = match p.item(SYSTEM_TEXT).map(|i| &i.control) {
            Some(Control::Lines(l)) => l.clone(),
            _ => Vec::new(),
        };
        assert_eq!(
            text,
            [
                "Language: Deutsch (Deutschland)",
                "Formats: Same as the language"
            ]
        );
    }
}
