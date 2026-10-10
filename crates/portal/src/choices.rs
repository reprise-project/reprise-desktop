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
use zbus::zvariant::Value;

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
pub fn parse_choices(v: &Value<'_>) -> Vec<Choice> {
    let Value::Array(a) = v else {
        log::warn!("choices: not an array ({})", v.value_signature());
        return Vec::new();
    };
    let string = |v: Option<&Value<'_>>| match v {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    };
    a.inner()
        .iter()
        .filter_map(|item| {
            // A loosely typed caller sends `av`: each item wrapped once.
            let item = match item {
                Value::Value(inner) => inner.as_ref(),
                v => v,
            };
            let Value::Structure(s) = item else {
                log::warn!("choices: dropped an item that is not a structure");
                return None;
            };
            let f = s.fields();
            let (Some(id), Some(label)) = (string(f.first()), string(f.get(1))) else {
                log::warn!("choices: dropped an item without an id and label");
                return None;
            };
            let options = match f.get(2) {
                Some(Value::Array(o)) => o
                    .inner()
                    .iter()
                    .filter_map(|p| match p {
                        Value::Structure(p) => match p.fields() {
                            [Value::Str(id), Value::Str(label)] => {
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

/// The rows for `choices` stacked from `y`, and the y under the last row.
pub fn choice_rows(
    choices: &[Choice],
    x: i32,
    y: i32,
    w: i32,
    text: &TextRenderer,
    px: u32,
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
        let label = plain.replace('&', "&&");
        if c.options.is_empty() {
            items.push(Item::new(
                IDS[i],
                Rect::new(x, y, CHECK, CHECK),
                Control::Check {
                    label,
                    checked: c.initial == "true",
                    enabled: true,
                    mixed: None,
                },
            ));
            bottom = y + CHECK;
            y += CHECK_PITCH;
        } else {
            let label_w = (text.measure(&plain, false, px) + 1).min(w / 2);
            items.push(Item::label(
                LABEL_IDS[i],
                Rect::new(x, y + (COMBO_H - LINE_H) / 2, label_w, LINE_H),
                &label,
            ));
            let cx = x + label_w + LABEL_GAP;
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
            items.push(Item::new(
                IDS[i],
                Rect::new(cx, y, x + w - cx, COMBO_H),
                Control::Combo(Combo::new(&names, start, names.len().min(DROP_ROWS))),
            ));
            bottom = y + COMBO_H;
            y += COMBO_PITCH;
        }
    }
    (items, bottom)
}

/// Shorten each choice combo's open list so it ends by `bottom`, the client's
/// height; rows past the edge could not be clicked. A shortened list scrolls.
pub fn fit_lists(form: &mut Form, bottom: i32, cm: &ControlMetrics) {
    for id in IDS {
        let Some(it) = form.item_mut(id) else {
            continue;
        };
        let field = it.rect;
        if let Control::Combo(c) = &mut it.control {
            while c.rows > 1 && combo::list_rect(c, field, cm).bottom() > bottom {
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
        let (items, bottom) = choice_rows(cs, 54, 90, 326, &text, 11);
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
        let v = Value::new(vec![
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
        let v = Value::new(vec![
            Value::new(7u32),
            Value::new((
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
        assert!(parse_choices(&Value::new(7u32)).is_empty());
    }
}
