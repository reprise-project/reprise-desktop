//! `org.freedesktop.impl.portal.Access`: the frontend's generic "Allow
//! ...?" prompt (camera, location, running in the background), run as our
//! own child (`reprise-portal --access`, see `dialog`). The caller's
//! strings and choices go in on stdin as TOML, and the answer and the
//! picked choices come back on stdout the same way.

use crate::choices::{self, Choice};
use crate::dialog::{self, Prompt, TEXT_X};
use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{Form, Item};
use reprisekit::metrics::ControlMetrics;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::TextUnits;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const CLIENT_W: i32 = 391;
const FIRST_Y: i32 = 12;
/// The permit prompt's button row; the buttons never sit higher.
const MIN_BUTTON_Y: i32 = 97;
const BUTTON_GAP_Y: i32 = 20;
const MARGIN: i32 = 11;
/// The most lines the subtitle and the body take; a longer text ends in
/// "..." on the last line kept.
const SUBTITLE_LINES: usize = 4;
const BODY_LINES: usize = 12;
const FALLBACK_ICON: &str = "dialog-question";

pub struct Access;

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl Access {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        subtitle: String,
        body: String,
        options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let input = Input::from_call(&parent_window, title, subtitle, body, &options);
        // The Request object lets the frontend cancel the prompt.
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone()).await?;
        let out = blocking::unblock(move || ask(&input, cancel)).await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        log::info!(
            "Access for {app_id:?}: {}",
            if out.granted { "granted" } else { "denied" }
        );
        Ok(reply(&out))
    }
}

/// What the parent tells the prompt.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Input {
    pub parent: Option<String>,
    pub modal: bool,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub icon: Option<String>,
    pub grant_label: Option<String>,
    pub deny_label: Option<String>,
    pub choices: Vec<Choice>,
}

/// What the prompt answers. A killed or failed child reads as the default:
/// denied, no choices.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub granted: bool,
    pub choices: Vec<(String, String)>,
}

/// The icon names to try, in order: the caller's without `-symbolic`
/// (the theme's colour icon), the caller's as sent, then the question
/// icon.
pub fn icon_names(icon: Option<&str>) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(name) = icon.filter(|n| !n.is_empty()) {
        if let Some(plain) = name.strip_suffix("-symbolic") {
            names.push(plain.to_string());
        }
        names.push(name.to_string());
    }
    names.push(FALLBACK_ICON.to_string());
    names
}

/// The prompt's form and 100% client size for text set at `px` that grows by `u`: subtitle, body and choices stacked from line 12, Allow and Deny 20 px under the lowest row and never above the permit prompt's row at 97. Subtitle and body are capped at `SUBTITLE_LINES` and `BODY_LINES`. Deny is the default button and has the focus, so Enter never grants by accident. `cm` is the look the prompt runs in, whose combo lists are kept inside it.
pub fn build_form(
    input: &Input,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
    cm: &ControlMetrics,
) -> (Form, (i32, i32)) {
    let col = CLIENT_W - TEXT_X - MARGIN;
    let wrap_w = u.len(col);
    let mut items: Vec<Item> = Vec::new();
    let mut bottom: Option<i32> = None;
    let next_y = |bottom: Option<i32>| bottom.map_or(FIRST_Y, |b| b + LINE_H);
    for (id, s, max) in [
        ("subtitle", &input.subtitle, SUBTITLE_LINES),
        ("body", &input.body, BODY_LINES),
    ] {
        let lines = capped(
            text,
            reprisekit::text::wrap(text, s, wrap_w, px),
            max,
            wrap_w,
            px,
        );
        if lines.is_empty() {
            continue;
        }
        let y = next_y(bottom);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let h = LINE_H * refs.len() as i32;
        items.push(Item::lines(id, Rect::new(TEXT_X, y, col, h), &refs));
        bottom = Some(y + h);
    }
    if !input.choices.is_empty() {
        let (rows, end) =
            choices::choice_rows(&input.choices, TEXT_X, next_y(bottom), col, text, px, u);
        items.extend(rows);
        bottom = Some(end);
    }
    let button_y = bottom.map_or(MIN_BUTTON_Y, |b| (b + BUTTON_GAP_Y).max(MIN_BUTTON_Y));
    let grant = input
        .grant_label
        .as_deref()
        .map_or("&Allow".to_string(), dialog::mnemonic);
    let deny = input
        .deny_label
        .as_deref()
        .map_or("&Deny".to_string(), dialog::mnemonic);
    items.extend(dialog::buttons(
        CLIENT_W - MARGIN,
        button_y,
        ("allow", &grant),
        ("deny", &deny),
    ));
    let size = (CLIENT_W, button_y + BUTTON_H + MARGIN);
    let mut form = Form::new(items);
    choices::fit_lists(&mut form, size.1, cm, u);
    form.focus_id("deny");
    (form, size)
}

fn capped(
    text: &TextRenderer,
    mut lines: Vec<String>,
    max: usize,
    col: i32,
    px: u32,
) -> Vec<String> {
    if lines.len() <= max {
        return lines;
    }
    lines.truncate(max);
    if let Some(last) = lines.last_mut() {
        let dotted = format!("{}...", last.trim_end());
        *last = text.ellipsize(&dotted, col, false, px).into_owned();
    }
    lines
}

impl Input {
    fn from_call(
        parent_window: &str,
        title: String,
        subtitle: String,
        body: String,
        options: &HashMap<String, Value<'_>>,
    ) -> Input {
        let string = |k: &str| match options.get(k) {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        Input {
            parent: dialog::parent_handle(parent_window),
            modal: !matches!(options.get("modal"), Some(Value::Bool(false))),
            title,
            subtitle,
            body,
            icon: string("icon"),
            grant_label: string("grant_label"),
            deny_label: string("deny_label"),
            choices: options
                .get("choices")
                .map(choices::parse_choices)
                .unwrap_or_default(),
        }
    }
}

/// Show the prompt and wait for it; a cancel kills it. Blocks.
fn ask(input: &Input, cancel: Arc<AtomicBool>) -> Output {
    dialog::run_toml(&["--access"], input, &cancel)
        .and_then(|out| {
            toml::from_str(&out)
                .map_err(|e| log::warn!("access answer: {e}"))
                .ok()
        })
        .unwrap_or_default()
}

/// The D-Bus answer: 0 on Allow and 1 otherwise, with the choices as
/// `a(ss)` whenever the prompt reported them.
fn reply(out: &Output) -> (u32, HashMap<String, OwnedValue>) {
    let mut r = HashMap::new();
    if !out.choices.is_empty() {
        let v = Value::new(out.choices.clone())
            .try_to_owned()
            .expect("choices are plain strings");
        r.insert("choices".to_string(), v);
    }
    let code = if out.granted {
        crate::RESPONSE_OK
    } else {
        crate::RESPONSE_CANCELLED
    };
    (code, r)
}

pub fn run_dialog() {
    let Some(input) = dialog::read_input::<Input>() else {
        return;
    };
    let text = TextRenderer::load(&reprisekit::theme::Theme::default().fonts);
    let forms = dialog::start_forms();
    let cm = forms.look.controls();
    let (form, size) = build_form(&input, &text, forms.fonts.size_px, forms.text, &cm);
    let mut icons = dialog::icons();
    let icon = icon_names(input.icon.as_deref())
        .iter()
        .find_map(|n| icons.get(n, dialog::ICON_PX));
    let out = Arc::new(Mutex::new(Output::default()));
    repriseapp::run(Box::new(prompt(&input, form, size, icon, out.clone())));
    let out = out.lock().unwrap().clone();
    match toml::to_string(&out) {
        Ok(s) => print!("{s}"),
        Err(e) => log::warn!("access answer: {e}"),
    }
}

/// The window: Allow records a grant and the choices; any other close
/// records the choices alone.
fn prompt(
    input: &Input,
    form: Form,
    size: (i32, i32),
    icon: Option<Canvas>,
    out: Arc<Mutex<Output>>,
) -> Prompt {
    let (on_allow, on_deny) = (input.choices.clone(), input.choices.clone());
    let deny_out = out.clone();
    Prompt {
        title: dialog::one_line(&input.title),
        size,
        form,
        icon,
        accept: "allow",
        cancel: "deny",
        on_accept: Box::new(move |form| {
            *out.lock().unwrap() = Output {
                granted: true,
                choices: choices::choice_answers(&on_allow, form),
            };
            true
        }),
        on_cancel: Box::new(move |form, _| {
            *deny_out.lock().unwrap() = Output {
                granted: false,
                choices: choices::choice_answers(&on_deny, form),
            };
        }),
        parent: input.parent.clone(),
        modal: input.modal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::choices::Choice;
    use reprisekit::controls::form::Control;
    use reprisekit::theme::FontConfig;
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    fn form(input: &Input) -> (Form, (i32, i32)) {
        build_form(
            input,
            &TextRenderer::load(&FontConfig::default()),
            11,
            TextUnits::ONE,
            &ControlMetrics::CLASSIC,
        )
    }

    fn base() -> Input {
        Input {
            subtitle: "Portal Test wants to use the camera.".into(),
            ..Default::default()
        }
    }

    #[test]
    fn every_prompt_fits_its_text() {
        let input = Input {
            body: "It will see what your camera sees, and it can record what it sees for as \
                   long as it runs. You can change this in Settings."
                .into(),
            choices: vec![
                Choice {
                    id: "m".into(),
                    label: "Camera:".into(),
                    options: (0..3)
                        .map(|i| (format!("{i}"), format!("Integrated Camera {i}")))
                        .collect(),
                    initial: "0".into(),
                },
                Choice {
                    id: "r".into(),
                    label: "Remember this choice".into(),
                    options: Vec::new(),
                    initial: "false".into(),
                },
            ],
            ..base()
        };
        let over = crate::dialog::overflows(|theme, text| {
            let (px, u) = (theme.fonts.size_px, theme.text);
            build_form(&input, text, px, u, &theme.controls()).0
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn at_150_percent_the_text_wraps_to_the_grown_column_and_the_combo_list_fits_the_window() {
        let text = TextRenderer::load(&FontConfig::default());
        let u = TextUnits::new(17, 11);
        let input = Input {
            body: "It will see what your camera sees, and it can record what it sees for as long as it runs. You can change this in Settings.".into(),
            choices: vec![Choice {
                id: "m".into(),
                label: "Camera:".into(),
                options: (0..8).map(|i| (format!("{i}"), format!("Camera {i}"))).collect(),
                initial: "0".into(),
            }],
            ..base()
        };
        let (f, size) = build_form(&input, &text, 17, u, &ControlMetrics::CLASSIC);
        let col = u.len(CLIENT_W - TEXT_X - MARGIN);
        let Control::Lines(lines) = &f.item("body").unwrap().control else {
            panic!("the body is lines");
        };
        assert!(lines.len() > 1);
        for l in lines {
            assert!(text.measure(l, false, 17) <= col, "{l:?} fits {col} px");
        }
        let Control::Combo(c) = &f.item("choice0").unwrap().control else {
            panic!("the choice is a combo");
        };
        let field = u.rect(f.item("choice0").unwrap().rect);
        let cm = ControlMetrics::CLASSIC.grown(u);
        let list = reprisekit::controls::combo::list_rect(c, field, &cm);
        assert!(
            list.bottom() <= u.len(size.1),
            "the open list ends inside the window"
        );
    }

    #[test]
    fn a_call_becomes_input() {
        let mut options: HashMap<String, Value<'_>> = HashMap::new();
        options.insert("modal".into(), Value::Bool(false));
        options.insert("icon".into(), Value::from("camera-web-symbolic"));
        options.insert("deny_label".into(), Value::from("_No"));
        let input = Input::from_call("x11:12", "T".into(), "S".into(), "B".into(), &options);
        assert_eq!(input.parent, None);
        assert!(!input.modal);
        assert_eq!(input.icon.as_deref(), Some("camera-web-symbolic"));
        assert_eq!(input.deny_label.as_deref(), Some("_No"));
        assert_eq!(input.grant_label, None);
        let input = Input::from_call(
            "wayland:h",
            "T".into(),
            "S".into(),
            "B".into(),
            &HashMap::new(),
        );
        assert_eq!(input.parent.as_deref(), Some("h"));
        assert!(input.modal, "modal defaults to true");
    }

    #[test]
    fn granted_answers_ok_with_choices() {
        let (code, r) = reply(&Output {
            granted: true,
            choices: vec![("remember".into(), "true".into())],
        });
        assert_eq!(code, crate::RESPONSE_OK);
        assert_eq!(r["choices"].value_signature().to_string(), "a(ss)");
    }

    #[test]
    fn a_dead_child_answers_deny() {
        let (code, r) = reply(&Output::default());
        assert_eq!(code, crate::RESPONSE_CANCELLED);
        assert!(r.is_empty());
    }

    #[test]
    fn a_combo_list_fits_inside_the_prompt() {
        let options: Vec<(String, String)> = (0..12)
            .map(|i| (format!("o{i}"), format!("Option {i}")))
            .collect();
        let (f, (_, h)) = form(&Input {
            choices: vec![Choice {
                id: "m".into(),
                label: "Mode".into(),
                options,
                initial: String::new(),
            }],
            ..base()
        });
        let it = f.item("choice0").unwrap();
        let Control::Combo(c) = &it.control else {
            panic!("choice0 is a combo");
        };
        let list = reprisekit::controls::combo::list_rect(c, it.rect, &ControlMetrics::CLASSIC);
        assert!(
            list.bottom() <= h,
            "the open list ends at {} inside the {h} px client",
            list.bottom()
        );
    }

    #[test]
    fn a_bare_prompt_is_the_permit_prompts_size() {
        let (f, size) = form(&base());
        assert_eq!(size, (391, 131));
        assert_eq!(f.item("subtitle").unwrap().rect.y, 12);
        assert_eq!(f.item("allow").unwrap().rect, Rect::new(224, 97, 75, 23));
        assert_eq!(f.item("deny").unwrap().rect, Rect::new(305, 97, 75, 23));
    }

    #[test]
    fn body_and_choices_grow_the_prompt() {
        let input = Input {
            body: "one\ntwo\nthree".into(),
            choices: vec![
                Choice {
                    id: "r".into(),
                    label: "Remember".into(),
                    options: vec![],
                    initial: "false".into(),
                },
                Choice {
                    id: "m".into(),
                    label: "Mode".into(),
                    options: vec![("a".into(), "A".into())],
                    initial: "a".into(),
                },
            ],
            ..base()
        };
        let (f, size) = form(&input);
        let body = f.item("body").unwrap().rect;
        assert_eq!(
            (body.y, body.height),
            (38, 39),
            "body from line 38, three 13 px rows"
        );
        assert_eq!(
            f.item("choice0").unwrap().rect.y,
            90,
            "choices a blank line under the body"
        );
        assert_eq!(
            f.item("allow").unwrap().rect.y,
            151,
            "buttons 20 px under the combo's bottom (131)"
        );
        assert_eq!(size, (391, 185));
    }

    #[test]
    fn default_labels_and_deny_is_the_default_with_focus() {
        let (f, _) = form(&base());
        assert!(
            matches!(&f.item("allow").unwrap().control, Control::Button { label, default: false, .. } if label == "&Allow")
        );
        assert!(
            matches!(&f.item("deny").unwrap().control, Control::Button { label, default: true, .. } if label == "&Deny")
        );
        assert_eq!(
            f.default_button(),
            f.index_of("deny"),
            "Deny is the default button"
        );
        assert_eq!(f.focus(), f.index_of("deny"), "Deny has the focus");
    }

    #[test]
    fn caller_labels_are_mnemonic_labels() {
        let (f, _) = form(&Input {
            grant_label: Some("_Share".into()),
            deny_label: Some("Do_n't".into()),
            ..base()
        });
        assert!(
            matches!(&f.item("allow").unwrap().control, Control::Button { label, .. } if label == "&Share")
        );
        assert!(
            matches!(&f.item("deny").unwrap().control, Control::Button { label, .. } if label == "Do&n't")
        );
    }

    #[test]
    fn a_long_subtitle_and_body_stop_at_their_line_caps() {
        let many = |n: usize| {
            (0..n)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (f, (_, h)) = form(&Input {
            subtitle: many(9),
            body: many(40),
            ..Default::default()
        });
        let lines = |id| match &f.item(id).unwrap().control {
            Control::Lines(lines) => lines.clone(),
            c => panic!("{id} is text lines, got {c:?}"),
        };
        let sub = lines("subtitle");
        assert_eq!(sub.len(), 4, "the subtitle keeps 4 lines");
        assert_eq!(sub[3], "line 3...", "its last line ends in dots");
        let body = lines("body");
        assert_eq!(body.len(), 12, "the body keeps 12 lines");
        assert_eq!(body[11], "line 11...");
        // 12 + 4 * 13 + 13 + 12 * 13 = 233; buttons 20 under, 23 tall, 11 margin.
        assert_eq!(h, 233 + 20 + 23 + 11, "the prompt stops growing");
    }

    #[test]
    fn a_cut_line_too_wide_for_its_dots_is_shortened() {
        let word = "W".repeat(60);
        let (f, _) = form(&Input {
            subtitle: format!("{word}\n{word}\n{word}\n{word}\n{word}"),
            ..Default::default()
        });
        let Control::Lines(lines) = &f.item("subtitle").unwrap().control else {
            panic!("subtitle is text lines");
        };
        let text = TextRenderer::load(&FontConfig::default());
        assert!(lines[3].ends_with("..."), "{:?} ends in dots", lines[3]);
        assert!(
            text.measure(&lines[3], false, 11) <= 391 - 54 - 11,
            "the dotted line fits the column"
        );
    }

    #[test]
    fn caller_labels_and_the_title_are_one_line() {
        let input = Input {
            title: "Allow\naccess?".into(),
            grant_label: Some("_Share\tnow".into()),
            choices: vec![
                Choice {
                    id: "r".into(),
                    label: "Remember\nthis".into(),
                    options: vec![],
                    initial: "false".into(),
                },
                Choice {
                    id: "m".into(),
                    label: "Camera\tone".into(),
                    options: vec![("a".into(), "Built\nin".into())],
                    initial: "a".into(),
                },
            ],
            ..base()
        };
        let (f, size) = form(&input);
        assert!(
            matches!(&f.item("allow").unwrap().control, Control::Button { label, .. } if label == "&Share now")
        );
        assert!(
            matches!(&f.item("choice0").unwrap().control, Control::Check { label, .. } if label == "Remember this")
        );
        assert!(
            matches!(&f.item("choice1-label").unwrap().control, Control::Label { text, .. } if text == "Camera one")
        );
        let Control::Combo(c) = &f.item("choice1").unwrap().control else {
            panic!("choice1 is a combo");
        };
        assert_eq!(c.items[0].text, "Built in");
        let p = prompt(&input, f, size, None, Arc::default());
        assert_eq!(p.title, "Allow access?");
    }

    #[test]
    fn empty_strings_leave_their_rows_out() {
        let (f, _) = form(&Input {
            subtitle: String::new(),
            body: "only body".into(),
            ..Default::default()
        });
        assert!(f.item("subtitle").is_none());
        assert_eq!(
            f.item("body").unwrap().rect.y,
            12,
            "the body takes the first line"
        );
    }

    #[test]
    fn the_colour_icon_comes_before_the_symbolic_one() {
        assert_eq!(
            icon_names(Some("camera-web-symbolic")),
            vec!["camera-web", "camera-web-symbolic", "dialog-question"]
        );
        assert_eq!(
            icon_names(Some("camera-web")),
            vec!["camera-web", "dialog-question"]
        );
        assert_eq!(icon_names(None), vec!["dialog-question"]);
    }

    #[test]
    fn access_input_survives_the_pipe() {
        let input = Input {
            parent: Some("h".into()),
            modal: true,
            body: "a\nb & _c".into(),
            ..base()
        };
        let back: Input = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
        assert_eq!(back, input);
    }

    /// The prompt drawn: subtitle and two body lines as text rows, an
    /// unchecked box, a combo with its drop button, Allow raised and Deny
    /// ringed as the default.
    #[test]
    fn access_prompt_pixels() {
        use reprisekit::controls::form;
        use reprisekit::theme::Theme;
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let text = TextRenderer::load(&FontConfig::default());
        let input = Input {
            body: "It will see what your camera sees.\nYou can change this in Settings.".into(),
            choices: vec![
                Choice {
                    id: "r".into(),
                    label: "Remember this choice".into(),
                    options: vec![],
                    initial: "false".into(),
                },
                Choice {
                    id: "m".into(),
                    label: "Camera:".into(),
                    options: vec![("a".into(), "Built-in".into())],
                    initial: "a".into(),
                },
            ],
            ..base()
        };
        let (f, (w, h)) = build_form(&input, &text, px, TextUnits::ONE, &ControlMetrics::CLASSIC);
        assert_eq!(
            (w, h),
            (391, 172),
            "client 172 tall: buttons at 138 + 23 + 11"
        );
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        form::draw(&mut c, &text, &theme.paint(), &f, true, px);
        let inked = |y0: i32| {
            (y0..y0 + LINE_H)
                .any(|y| (TEXT_X..TEXT_X + 60).any(|x| c.get(x, y) == cs.button_text.argb()))
        };
        assert!(inked(12), "subtitle inked on the row from y 12");
        assert!(inked(38), "first body line inked on the row from y 38");
        assert!(inked(51), "second body line inked on the row from y 51");
        assert!(!inked(64), "a blank row at y 64 before the choices");
        // The check box at (54, 77): sunken edge, white well.
        assert_eq!(
            c.get(54, 77),
            cs.button_shadow.argb(),
            "check box outer edge at (54, 77)"
        );
        assert_eq!(
            c.get(60, 83),
            cs.window.argb(),
            "unchecked well is the window colour"
        );
        // The combo from y 97: sunken field, raised drop button at its right.
        let combo = f.item("choice1").unwrap().rect;
        assert_eq!(
            (combo.y, combo.right()),
            (97, 380),
            "combo field at y 97, ending at x 380"
        );
        assert_eq!(
            c.get(combo.x, combo.y),
            cs.button_shadow.argb(),
            "combo outer edge"
        );
        // 100 px in, past "Built-in": at x + 5 the text's first glyph is inked.
        assert_eq!(
            c.get(combo.x + 100, combo.y + 10),
            cs.window.argb(),
            "combo field is the window colour"
        );
        // Buttons from y 138, 20 under the combo: Allow raised, Deny ringed.
        assert_eq!(
            c.get(224, 143),
            cs.button_hilight.argb(),
            "Allow's raised edge at (224, 143)"
        );
        assert_eq!(
            c.get(305, 143),
            cs.button_text.argb(),
            "Deny's default ring at (305, 143)"
        );
        assert_eq!(
            c.get(306, 143),
            cs.button_hilight.argb(),
            "Deny's bevel inside its ring"
        );
    }

    #[test]
    #[ignore]
    fn render_access() {
        use reprisekit::controls::form;
        use reprisekit::theme::Theme;
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let input = Input {
            body: "It will see what your camera sees.\nYou can change this in Settings.".into(),
            choices: vec![
                Choice {
                    id: "r".into(),
                    label: "Remember this choice".into(),
                    options: vec![],
                    initial: "true".into(),
                },
                Choice {
                    id: "m".into(),
                    label: "Camera:".into(),
                    options: vec![("a".into(), "Built-in".into()), ("b".into(), "USB".into())],
                    initial: "b".into(),
                },
            ],
            ..base()
        };
        let (f, (w, h)) = build_form(
            &input,
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
            &theme.look.controls(),
        );
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(&mut c, &text, &theme.paint(), &f, true, theme.fonts.size_px);
        reprisekit::raster::write_preview(&c, "access-preview.png");
    }
}
