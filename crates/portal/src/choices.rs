//! A caller's extra questions on a portal dialog (Access and FileChooser
//! `choices`, D-Bus `a(ssa(ss)s)`): a check box when a choice has no
//! options, else its label and a combo box. The answers go back as
//! `a(ss)`: `"true"`/`"false"` for a check box, the option id for a combo.

use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::combo::{self, COMBO_H, Combo};
use reprisekit::controls::form::{CHECK, Control, Form, Item};
use reprisekit::metrics::ControlMetrics;
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::TextUnits;
use zbus::Variant;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub initial: String,
}

/// The form ids of the rows shown: at most eight (a form id is `'static`); more
/// choices are dropped.
const IDS: [&str; 8] = [
    "choice0", "choice1", "choice2", "choice3", "choice4", "choice5", "choice6", "choice7",
];
const LABEL_IDS: [&str; 8] = [
    "choice0-label",
    "choice1-label",
    "choice2-label",
    "choice3-label",
    "choice4-label",
    "choice5-label",
    "choice6-label",
    "choice7-label",
];
const CHECK_PITCH: i32 = 20;
const COMBO_PITCH: i32 = COMBO_H + 6;
const LABEL_GAP: i32 = 6;
const DROP_ROWS: usize = 8;

/// The choices in a call's `choices` option; a value of another type gives none.
pub fn parse_choices(v: &Variant<'_>) -> Vec<Choice> {
    let Variant::Array(a) = v else {
        log::warn!("choices: not an array ({})", v.value_signature());
        return Vec::new();
    };
    let string = |v: Option<&Variant<'_>>| match v {
        Some(Variant::Str(s)) => Some(s.to_string()),
        _ => None,
    };
    a.inner()
        .iter()
        .filter_map(|item| {
            // A loosely typed caller sends `av`: each item wrapped once.
            let item = match item {
                Variant::Variant(inner) => inner.as_ref(),
                v => v,
            };
            let Variant::Structure(s) = item else {
                log::warn!("choices: dropped an item that is not a structure");
                return None;
            };
            let f = s.fields();
            let (Some(id), Some(label)) = (string(f.first()), string(f.get(1))) else {
                log::warn!("choices: dropped an item without an id and label");
                return None;
            };
            let options = match f.get(2) {
                Some(Variant::Array(o)) => o
                    .inner()
                    .iter()
                    .filter_map(|p| match p {
                        Variant::Structure(p) => match p.fields() {
                            [Variant::Str(id), Variant::Str(label)] => {
                                Some((id.to_string(), label.to_string()))
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
                _ => {
                    log::warn!("choice {id}: options are not a(ss), shown as a check box");
                    Vec::new()
                }
            };
            Some(Choice {
                id,
                label,
                options,
                initial: string(f.get(3)).unwrap_or_default(),
            })
        })
        .collect()
}

/// The rows for `choices` stacked from `(x, y)`, and the y under the last row, in 100% units for
/// text set at `px` that grows by `u`. The combos end `w` px from `x`, or `room` px (at least
/// `w`) for one whose widest option needs it; the form's text may run `room` px from `x`. Labels
/// keep their whole text for the form to cut and tip.
pub fn choice_rows(
    choices: &[Choice],
    (x, y): (i32, i32),
    w: i32,
    room: i32,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
) -> (Vec<Item>, i32) {
    if choices.len() > IDS.len() {
        log::warn!("{} choices; showing the first {}", choices.len(), IDS.len());
    }
    let mut items = Vec::new();
    let (mut y, mut bottom) = (y, y);
    for (i, c) in choices.iter().take(IDS.len()).enumerate() {
        // A form label reads `&` as an access key; a caller's is text,
        // on one line.
        let plain = crate::dialog::one_line(&c.label);
        if c.options.is_empty() {
            // The rect spans the room so the form lets the label run that far.
            items.push(Item::new(
                IDS[i],
                Rect::new(x, y, room, CHECK),
                Control::Check {
                    label: reprisekit::text::escape_mnemonic(&plain),
                    checked: c.initial == "true",
                    enabled: true,
                    mixed: None,
                },
            ));
            bottom = y + CHECK;
            y += CHECK_PITCH;
        } else {
            let names: Vec<String> = c
                .options
                .iter()
                .map(|(_, l)| crate::dialog::one_line(l))
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let start = c
                .options
                .iter()
                .position(|(id, _)| *id == c.initial)
                .unwrap_or(0);
            let combo = Combo::new(&names, start, names.len().min(DROP_ROWS));
            let probe = Rect::new(0, 0, w, COMBO_H);
            let inset = w - combo::layout(&combo, probe, &ControlMetrics::CLASSIC)
                .text
                .width;
            let widest = names.iter().map(|n| text.measure(n, false, px)).max();
            let need = u.design(widest.unwrap_or(0)) + inset;
            // The label takes what it needs while the combo keeps its need or half the room.
            let most = room - LABEL_GAP - need.min(room / 2);
            let label_w = u.design(text.measure(&plain, false, px) + 1).min(most);
            // Whole: the form cuts a label longer than its rect and tips the rest.
            items.push(Item::label(
                LABEL_IDS[i],
                Rect::new(x, y + (COMBO_H - LINE_H) / 2, label_w, LINE_H),
                &reprisekit::text::escape_mnemonic(&plain),
            ));
            let cx = x + label_w + LABEL_GAP;
            let right = if cx + need > x + w { x + room } else { x + w };
            items.push(Item::new(
                IDS[i],
                Rect::new(cx, y, right - cx, COMBO_H),
                Control::Combo(combo),
            ));
            bottom = y + COMBO_H;
            y += COMBO_PITCH;
        }
    }
    (items, bottom)
}

/// Shorten each choice combo's open list so it ends by `bottom`, the client's
/// height; rows past the edge could not be clicked. A shortened list scrolls. `bottom` is at
/// 100%; the lists are measured as they draw, grown by `u`.
pub fn fit_lists(form: &mut Form, bottom: i32, cm: &ControlMetrics, u: TextUnits) {
    let (cm, bottom) = (cm.grown(u), u.len(bottom));
    for id in IDS {
        let Some(it) = form.item_mut(id) else {
            continue;
        };
        let field = u.rect(it.rect);
        if let Control::Combo(c) = &mut it.control {
            while c.rows > 1 && combo::list_rect(c, field, &cm).bottom() > bottom {
                c.rows -= 1;
            }
        }
    }
}

pub fn choice_answers(choices: &[Choice], form: &Form) -> Vec<(String, String)> {
    choices
        .iter()
        .zip(IDS)
        .filter_map(|(c, id)| {
            let value = if c.options.is_empty() {
                form.checked(id)?.to_string()
            } else {
                c.options.get(form.combo_selected(id)?)?.0.clone()
            };
            Some((c.id.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::form::Control;
    use reprisekit::theme::FontConfig;

    fn check(id: &str, initial: &str) -> Choice {
        Choice {
            id: id.into(),
            label: "Remember & keep".into(),
            options: vec![],
            initial: initial.into(),
        }
    }

    fn combo(id: &str, initial: &str) -> Choice {
        Choice {
            id: id.into(),
            label: "Mode".into(),
            options: vec![("a".into(), "First".into()), ("b".into(), "Second".into())],
            initial: initial.into(),
        }
    }

    fn rows(cs: &[Choice]) -> (Form, i32) {
        let text = TextRenderer::load(&FontConfig::default());
        let (items, bottom) = choice_rows(cs, (54, 90), 326, 326, &text, 11, TextUnits::ONE);
        (Form::new(items), bottom)
    }

    #[test]
    fn a_check_row_then_a_combo_row_stack_on_their_pitches() {
        let (f, bottom) = rows(&[check("remember", "true"), combo("mode", "b")]);
        assert_eq!(f.item("choice0").unwrap().rect.y, 90, "check row at y 90");
        let c = f.item("choice1").unwrap().rect;
        assert_eq!(c.y, 110, "combo row one check pitch (20) lower");
        assert_eq!(c.x + c.width, 380, "combo ends at the column's right edge");
        assert_eq!(
            f.item("choice1-label").unwrap().rect.x,
            54,
            "combo label at the text column"
        );
        assert_eq!(bottom, 131, "bottom is the combo's (110 + 21)");
    }

    /// The file dialog's rows: combos end with the fields (246 px from x) unless an option needs
    /// the room to the right margin (354 px).
    #[test]
    fn a_combo_takes_the_room_only_for_an_option_that_needs_it() {
        let text = TextRenderer::load(&FontConfig::default());
        let long = Choice {
            options: vec![(
                "w".into(),
                "Western European (Windows-1252, ISO-8859-15)".into(),
            )],
            ..combo("enc", "a")
        };
        let (items, _) = choice_rows(
            &[combo("mode", "a"), long],
            (195, 451),
            246,
            354,
            &text,
            11,
            TextUnits::ONE,
        );
        let f = Form::new(items);
        assert_eq!(f.item("choice0").unwrap().rect.right(), 195 + 246);
        assert_eq!(f.item("choice1").unwrap().rect.right(), 195 + 354);
    }

    #[test]
    fn long_labels_keep_their_whole_text_and_a_combo_keeps_its_need() {
        let text = TextRenderer::load(&FontConfig::default());
        let words = "Open the exported picture in the image viewer once it is written to disk";
        let check = Choice {
            label: words.into(),
            ..check("open", "true")
        };
        let labelled = Choice {
            label: words.into(),
            ..combo("mode", "a")
        };
        let (items, _) = choice_rows(
            &[check, labelled],
            (0, 0),
            246,
            354,
            &text,
            11,
            TextUnits::ONE,
        );
        let f = Form::new(items);
        let Control::Check { label, .. } = &f.item("choice0").unwrap().control else {
            panic!("a check box");
        };
        assert_eq!(label, words, "the check box's label keeps its whole text");
        assert_eq!(
            f.item("choice0").unwrap().rect.width,
            354,
            "the rect spans the room"
        );
        let Control::Label { text: shown, .. } = &f.item("choice1-label").unwrap().control else {
            panic!("a label");
        };
        assert_eq!(shown, words, "the combo's label keeps its whole text");
        let combo = f.item("choice1").unwrap().rect;
        assert_eq!(combo.right(), 354, "the combo ends at the room's edge");
        assert!(
            combo.width >= text.measure("Second", false, 11) + 20,
            "and keeps its need"
        );
    }

    /// The form cuts the label where the rows used to, and the cut label tips its whole text.
    #[test]
    fn a_cut_combo_label_draws_its_ellipsis_and_tips_its_whole_text() {
        use reprisekit::controls::form;
        use reprisekit::raster::Canvas;
        use reprisekit::theme::Theme;
        let text = TextRenderer::load(&FontConfig::default());
        let words = "Character encoding used to read the picture metadata of every file";
        let labelled = Choice {
            label: words.into(),
            ..combo("enc", "a")
        };
        let at = (195, 451);
        let (items, _) = choice_rows(&[labelled], at, 246, 354, &text, 11, TextUnits::ONE);
        let mut f = Form::new(items);
        let r = f.item("choice0-label").unwrap().rect;
        let mut cut = f.clone();
        let Control::Label { text: shown, .. } =
            &mut cut.item_mut("choice0-label").unwrap().control
        else {
            panic!("a label");
        };
        *shown = text.ellipsize(words, r.width, false, 11).into_owned();
        let theme = Theme::default();
        let draw = |f: &Form| {
            let mut c = Canvas::new(600, 520);
            form::draw(&mut c, &text, &theme.paint(), f, true, 11);
            c.pixels
        };
        assert!(draw(&f) == draw(&cut), "drawn as the label cut to its rect");
        f.rest_pointer(Some((r.x + 20, r.y + 5)));
        assert!(f.tip_tick(), "the blink brings the tip up");
        assert_eq!(f.label_tip().as_deref(), Some(words));
    }

    /// The form cuts a check box's label at the room's edge and tips its whole text.
    #[test]
    fn a_cut_check_label_draws_its_ellipsis_and_tips_its_whole_text() {
        use reprisekit::controls::form;
        use reprisekit::raster::Canvas;
        use reprisekit::theme::Theme;
        let text = TextRenderer::load(&FontConfig::default());
        let words = "Open the exported picture in the image viewer once it is written to disk";
        let long = Choice {
            label: words.into(),
            ..check("open", "true")
        };
        let at = (195, 451);
        let (items, _) = choice_rows(&[long], at, 246, 354, &text, 11, TextUnits::ONE);
        let mut f = Form::new(items);
        let r = f.item("choice0").unwrap().rect;
        let mut cut = f.clone();
        let Control::Check { label, .. } = &mut cut.item_mut("choice0").unwrap().control else {
            panic!("a check box");
        };
        let room = 354 - ControlMetrics::CLASSIC.check_label;
        *label = text.ellipsize(words, room, false, 11).into_owned();
        assert!(label.ends_with("..."), "{label}");
        let theme = Theme::default();
        let draw = |f: &Form| {
            let mut c = Canvas::new(600, 520);
            form::draw(&mut c, &text, &theme.paint(), f, false, 11);
            c.pixels
        };
        assert!(draw(&f) == draw(&cut), "drawn cut at the room's edge");
        f.rest_pointer(Some((r.x + 40, r.y + 5)));
        assert!(f.tip_tick(), "the blink brings the tip up");
        assert_eq!(f.label_tip().as_deref(), Some(words));
    }

    #[test]
    fn rows_start_from_initial() {
        let (f, _) = rows(&[check("remember", "true"), combo("mode", "b")]);
        assert_eq!(f.checked("choice0"), Some(true));
        assert_eq!(f.combo_selected("choice1"), Some(1));
    }

    #[test]
    fn unknown_initial_starts_on_the_first_option() {
        let (f, _) = rows(&[combo("mode", "zzz")]);
        assert_eq!(f.combo_selected("choice0"), Some(0));
    }

    #[test]
    fn a_check_label_is_plain_text() {
        let (f, _) = rows(&[check("remember", "false")]);
        assert!(matches!(
            &f.item("choice0").unwrap().control,
            Control::Check { label, checked: false, .. } if label == "Remember && keep"
        ));
    }

    #[test]
    fn only_eight_choices_are_shown() {
        let many: Vec<Choice> = (0..10).map(|i| check(&format!("c{i}"), "false")).collect();
        let (f, _) = rows(&many);
        assert!(f.item("choice7").is_some());
        assert_eq!(
            choice_answers(&many, &f).len(),
            8,
            "the answers cover the shown rows"
        );
    }

    #[test]
    fn answers_are_id_value_pairs() {
        let cs = [check("remember", "false"), combo("mode", "a")];
        let (mut f, _) = rows(&cs);
        f.set_check("choice0", true);
        f.set_combo("choice1", 1);
        assert_eq!(
            choice_answers(&cs, &f),
            vec![
                ("remember".to_string(), "true".to_string()),
                ("mode".to_string(), "b".to_string())
            ]
        );
    }

    #[test]
    fn choices_parse_from_the_bus_type() {
        let v = Variant::new(vec![
            (
                "remember".to_string(),
                "Remember".to_string(),
                Vec::<(String, String)>::new(),
                "true".to_string(),
            ),
            (
                "mode".to_string(),
                "Mode".to_string(),
                vec![("a".to_string(), "First".to_string())],
                "a".to_string(),
            ),
        ]);
        let cs = parse_choices(&v);
        assert_eq!(cs.len(), 2);
        assert_eq!(
            cs[0],
            Choice {
                id: "remember".into(),
                label: "Remember".into(),
                options: vec![],
                initial: "true".into()
            }
        );
        assert_eq!(cs[1].options, vec![("a".to_string(), "First".to_string())]);
    }

    #[test]
    fn a_choice_without_an_id_is_dropped_and_the_rest_stay() {
        let v = Variant::new(vec![
            Variant::new(7u32),
            Variant::new((
                "ok".to_string(),
                "OK".to_string(),
                Vec::<(String, String)>::new(),
                "".to_string(),
            )),
        ]);
        assert_eq!(parse_choices(&v).len(), 1);
    }

    #[test]
    fn parse_choices_ignores_a_wrong_type() {
        assert!(parse_choices(&Variant::new(7u32)).is_empty());
    }
}
