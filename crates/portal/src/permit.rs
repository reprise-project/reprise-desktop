//! "Allow this program to ...?": the RemoteDesktop, Screenshot,
//! InputCapture, GlobalShortcuts and Background prompt, run as our own child
//! (`--permit <kind>`, see `dialog`). The parent writes an `Input` on stdin
//! (the asking window, the app's display name, whether to offer Remember,
//! the lines to list) and reads `yes` (or `yes remember`) on stdout for
//! Allow, `no` for Deny, and nothing when the prompt closed unanswered.

use crate::dialog::{self, Prompt, TEXT_X};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::{Canvas, Rect};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// The client and button row of a prompt that lists nothing; each listed
/// line moves the rows under the prompt down by `ITEM_H`.
const CLIENT: (i32, i32) = (391, 131);
const BUTTON_Y: i32 = 97;
/// The "Remember this choice" check box, under the note.
const REMEMBER_Y: i32 = 64;
const ITEM_H: i32 = 16;
const MAX_ITEMS: usize = 6;
/// The first listed line: 8 px under the prompt's row, leaving 8 px over
/// the moved note.
const ITEMS_Y: i32 = 12 + LINE_H + 8;
const ITEM_IDS: [&str; MAX_ITEMS + 1] = [
    "item0", "item1", "item2", "item3", "item4", "item5", "item6",
];

/// What the app asks for; picks the dialog's title, wording and icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    RemoteControl(Devices),
    Screenshot,
    InputCapture(Devices),
    Shortcuts,
    Background,
}

/// Which input devices a RemoteControl or InputCapture grant covers, so the
/// prompt names only those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Devices {
    Keyboard,
    Pointer,
    Both,
}

impl Devices {
    /// From the portal's device bits, which RemoteDesktop's device types and
    /// InputCapture's capabilities share. An empty mask words the prompt as
    /// both, since a consent prompt must never understate.
    pub fn from_mask(mask: u32) -> Devices {
        use crate::remotedesktop::{DEVICE_KEYBOARD, DEVICE_POINTER};
        match (mask & DEVICE_KEYBOARD != 0, mask & DEVICE_POINTER != 0) {
            (true, false) => Devices::Keyboard,
            (false, true) => Devices::Pointer,
            _ => Devices::Both,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Devices::Keyboard => "keyboard",
            Devices::Pointer => "mouse",
            Devices::Both => "keyboard and mouse",
        }
    }
}

impl Kind {
    const ALL: [Kind; 9] = [
        Kind::RemoteControl(Devices::Keyboard),
        Kind::RemoteControl(Devices::Pointer),
        Kind::RemoteControl(Devices::Both),
        Kind::Screenshot,
        Kind::InputCapture(Devices::Keyboard),
        Kind::InputCapture(Devices::Pointer),
        Kind::InputCapture(Devices::Both),
        Kind::Shortcuts,
        Kind::Background,
    ];

    fn arg(self) -> &'static str {
        match self {
            Kind::RemoteControl(Devices::Keyboard) => "remote-keyboard",
            Kind::RemoteControl(Devices::Pointer) => "remote-pointer",
            Kind::RemoteControl(Devices::Both) => "remote",
            Kind::Screenshot => "screenshot",
            Kind::InputCapture(Devices::Keyboard) => "capture-keyboard",
            Kind::InputCapture(Devices::Pointer) => "capture-pointer",
            Kind::InputCapture(Devices::Both) => "capture",
            Kind::Shortcuts => "shortcuts",
            Kind::Background => "background",
        }
    }

    fn from_arg(arg: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.arg() == arg)
    }

    fn title(self) -> &'static str {
        match self {
            Kind::RemoteControl(_) => "Remote control",
            Kind::Screenshot => "Screenshot",
            Kind::InputCapture(_) => "Input capture",
            Kind::Shortcuts => "Keyboard shortcuts",
            Kind::Background => "Running in the background",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::RemoteControl(_) => "input-keyboard",
            Kind::Screenshot => "camera-photo",
            Kind::InputCapture(_) => "input-mouse",
            Kind::Shortcuts => "input-keyboard",
            Kind::Background => "system-run",
        }
    }

    fn prompt(self, who: &str) -> String {
        match self {
            Kind::RemoteControl(d) => format!("{who} wants to control your {}.", d.noun()),
            Kind::Screenshot => format!("{who} wants to take a picture of your screen."),
            Kind::InputCapture(d) => format!("{who} wants to capture your {}.", d.noun()),
            Kind::Shortcuts => format!("{who} wants to use keyboard shortcuts."),
            Kind::Background => format!("{who} is running in the background."),
        }
    }

    fn note(self) -> &'static str {
        match self {
            Kind::RemoteControl(Devices::Keyboard) => "It will be able to type as if you did.",
            Kind::RemoteControl(Devices::Pointer) => "It will be able to click as if you did.",
            Kind::RemoteControl(Devices::Both) => {
                "It will be able to type and click as if you did."
            }
            Kind::Screenshot => "It will get a copy of everything the screen shows.",
            Kind::InputCapture(Devices::Both) => {
                "At a screen edge it takes both. Super+Shift+Esc ends it."
            }
            Kind::InputCapture(_) => "At a screen edge it takes it. Super+Shift+Esc ends it.",
            Kind::Shortcuts => "Change the keys on Reprise Settings' Keyboard page.",
            Kind::Background => "It has no open window. Deny closes it.",
        }
    }
}

/// What the user answered. `remember` is true only when the dialog
/// offered the check box and the user ticked it. `Deny` is the Deny
/// button; `Dismissed` is no answer: Esc, the window closed, a frontend
/// Close, or a prompt that failed to start or crashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Deny,
    Dismissed,
    Allow { remember: bool },
}

impl Answer {
    pub fn allowed(self) -> bool {
        matches!(self, Answer::Allow { .. })
    }

    fn parse(out: &str) -> Answer {
        match out.trim() {
            "yes" => Answer::Allow { remember: false },
            "yes remember" => Answer::Allow { remember: true },
            "no" => Answer::Deny,
            _ => Answer::Dismissed,
        }
    }
}

/// Show the `kind` prompt for `app_id`, listing `items` one per line under
/// the prompt (see `listed`), with a "Remember this choice" check box (off)
/// when `offer_remember`. Blocks the calling thread, which also scans the
/// desktop entries for the app's name. The prompt is modal on the window
/// `parent` names (see `dialog::parent_handle`); `cancel` kills the dialog
/// (a frontend Close).
pub fn ask(
    kind: Kind,
    app_id: &str,
    items: Vec<String>,
    offer_remember: bool,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Answer {
    let input = Input {
        parent,
        app: crate::remotedesktop::display_name(app_id),
        offer_remember,
        items,
    };
    ask_with(kind, input, &cancel)
}

/// `ask` with the prompt's whole input, `app` being the name to show.
pub fn ask_with(kind: Kind, mut input: Input, cancel: &AtomicBool) -> Answer {
    input.app = input.app.replace('\n', " ");
    let Some(out) = dialog::run_toml(&["--permit", kind.arg()], &input, cancel) else {
        return Answer::Dismissed;
    };
    Answer::parse(&out)
}

/// What the parent tells the prompt, as TOML on stdin.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub parent: Option<String>,
    pub app: String,
    pub offer_remember: bool,
    /// Lines listed under the prompt, such as the shortcuts asked for.
    #[serde(default)]
    pub items: Vec<String>,
}

/// The child's side; the argument after `--permit` names the kind.
pub fn run_dialog() {
    let kind = std::env::args()
        .skip_while(|a| a != "--permit")
        .nth(1)
        .and_then(|a| Kind::from_arg(&a))
        .unwrap_or(Kind::RemoteControl(Devices::Both));
    let Some(input) = dialog::read_input::<Input>() else {
        return;
    };
    let answer = Arc::new(Mutex::new(Answer::Dismissed));
    let icon = dialog::icons().get(kind.icon(), dialog::ICON_PX);
    let form = build_form(kind, &input);
    let size = client_size(&input);
    repriseapp::run(Box::new(prompt(
        kind,
        form,
        size,
        icon,
        input.parent,
        answer.clone(),
    )));
    match *answer.lock().unwrap() {
        Answer::Allow { remember: false } => println!("yes"),
        Answer::Allow { remember: true } => println!("yes remember"),
        Answer::Deny => println!("no"),
        Answer::Dismissed => {}
    }
}

/// How a prompt names the asking app: its display name, or "A program"
/// for an unsandboxed client, which sends none.
pub fn who(app: &str) -> String {
    if app.is_empty() {
        "A program".to_string()
    } else {
        app.to_string()
    }
}

/// At most `MAX_ITEMS` of `items`, each on one line, then "and N more"
/// for the rest.
fn listed(items: &[String]) -> Vec<String> {
    let mut shown: Vec<String> = items
        .iter()
        .take(MAX_ITEMS)
        .map(|i| i.replace('\n', " "))
        .collect();
    if items.len() > MAX_ITEMS {
        shown.push(format!("and {} more", items.len() - MAX_ITEMS));
    }
    shown
}

fn list_h(input: &Input) -> i32 {
    listed(&input.items).len() as i32 * ITEM_H
}

fn client_size(input: &Input) -> (i32, i32) {
    (CLIENT.0, CLIENT.1 + list_h(input))
}

/// The dialog's controls: who is asking, the listed lines, what it gets,
/// the Remember check box when offered, Allow and Deny. Deny is the default
/// button, so Enter never grants by accident, and Remember starts off.
fn build_form(kind: Kind, input: &Input) -> Form {
    let (w, _) = CLIENT;
    let dy = list_h(input);
    let [prompt, mut note] =
        dialog::header(w, kind.prompt(&who(input.app.trim())), kind.note().into());
    note.rect.y += dy;
    let mut items = vec![prompt];
    for (i, (id, line)) in ITEM_IDS.into_iter().zip(listed(&input.items)).enumerate() {
        let y = ITEMS_Y + i as i32 * ITEM_H;
        let line = reprisekit::text::escape_mnemonic(&line);
        items.push(Item::label_at(id, TEXT_X, y, w - TEXT_X - 11, &line));
    }
    items.push(note);
    if input.offer_remember {
        items.push(Item::new(
            "remember",
            Rect::new(TEXT_X, REMEMBER_Y + dy, 13, 13),
            Control::Check {
                label: "&Remember this choice".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ));
    }
    items.extend(dialog::buttons(
        w - 11,
        BUTTON_Y + dy,
        ("allow", "&Allow"),
        ("deny", "&Deny"),
    ));
    let mut f = Form::new(items);
    f.focus_id("deny");
    f
}

/// The dialog window, `size` large and modal on `parent`: Allow records the
/// answer, with Remember, into `answer`, and the Deny button records Deny.
/// Any other close leaves `answer` as it was (Dismissed).
fn prompt(
    kind: Kind,
    form: Form,
    size: (i32, i32),
    icon: Option<Canvas>,
    parent: Option<String>,
    answer: Arc<Mutex<Answer>>,
) -> Prompt {
    Prompt {
        parent,
        modal: true,
        on_cancel: Box::new({
            let answer = answer.clone();
            move |_, how| {
                if how == dialog::Refusal::Button {
                    *answer.lock().unwrap() = Answer::Deny;
                }
            }
        }),
        title: kind.title().into(),
        size,
        form,
        icon,
        accept: "allow",
        cancel: "deny",
        on_accept: Box::new(move |form| {
            let remember = form.checked("remember").unwrap_or(false);
            *answer.lock().unwrap() = Answer::Allow { remember };
            true
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prompt_fits_its_text() {
        let input = Input {
            parent: None,
            app: "Chromium".into(),
            offer_remember: true,
            items: vec!["Mute: Ctrl+M".into(), "Push to talk: Ctrl+Space".into()],
        };
        for kind in Kind::ALL {
            let over = crate::dialog::overflows(|_, _| build_form(kind, &input));
            assert!(over.is_empty(), "{kind:?}:\n{}", over.join("\n"));
        }
    }

    #[test]
    fn permit_input_survives_the_pipe() {
        for parent in [Some("h1".to_string()), None] {
            let input = Input {
                parent,
                app: "Line\none".into(),
                offer_remember: true,
                items: vec!["Mute: Ctrl+M".into()],
            };
            let back: Input = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
            assert_eq!(back, input);
        }
    }

    /// A Deskflow remote control prompt, Remember offered or not.
    fn deskflow(offer_remember: bool) -> Form {
        let input = Input {
            parent: None,
            app: "Deskflow".into(),
            offer_remember,
            items: Vec::new(),
        };
        build_form(Kind::RemoteControl(Devices::Both), &input)
    }

    #[test]
    fn deny_is_the_default_button() {
        let f = deskflow(false);
        let ids: Vec<&str> = ["prompt", "note", "allow", "deny"]
            .into_iter()
            .filter(|id| f.item(id).is_some())
            .collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(f.default_button(), f.index_of("deny"));
    }

    #[test]
    fn remember_is_offered_only_when_asked_and_starts_off() {
        assert_eq!(deskflow(false).checked("remember"), None);
        assert_eq!(deskflow(true).checked("remember"), Some(false));
    }

    #[test]
    fn only_a_ticked_remember_comes_back_as_remember() {
        assert_eq!(Answer::parse("yes\n"), Answer::Allow { remember: false });
        assert_eq!(
            Answer::parse("yes remember\n"),
            Answer::Allow { remember: true }
        );
        assert_eq!(Answer::parse("no\n"), Answer::Deny);
        assert_eq!(Answer::parse(""), Answer::Dismissed);
        assert_eq!(Answer::parse("remember"), Answer::Dismissed);
    }

    #[test]
    fn only_the_deny_button_answers_deny() {
        let answer_to = |ev| {
            let answer = Arc::new(Mutex::new(Answer::Dismissed));
            let kind = Kind::Shortcuts;
            let mut p = prompt(kind, deskflow(false), CLIENT, None, None, answer.clone());
            p.decide(ev);
            *answer.lock().unwrap()
        };
        use reprisekit::controls::form::Event;
        assert_eq!(answer_to(Event::Pressed("deny")), Answer::Deny);
        assert_eq!(
            answer_to(Event::Pressed("allow")),
            Answer::Allow { remember: false }
        );
        assert_eq!(answer_to(Event::Cancel), Answer::Dismissed);
        assert_eq!(answer_to(Event::Default), Answer::Dismissed);
    }

    #[test]
    fn remote_control_prompt_names_only_the_granted_devices() {
        let prompt = |mask| Kind::RemoteControl(Devices::from_mask(mask)).prompt("Deskflow");
        assert_eq!(prompt(1), "Deskflow wants to control your keyboard.");
        assert_eq!(prompt(2), "Deskflow wants to control your mouse.");
        assert_eq!(
            prompt(3),
            "Deskflow wants to control your keyboard and mouse."
        );
        assert_eq!(
            prompt(0),
            "Deskflow wants to control your keyboard and mouse."
        );
    }

    #[test]
    fn every_kind_survives_the_child_argument() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_arg(k.arg()), Some(k));
        }
    }

    /// Two shortcuts to list, as the GlobalShortcuts backend sends them.
    fn two_shortcuts() -> Input {
        Input {
            parent: None,
            app: "Taltos".into(),
            offer_remember: false,
            items: vec!["Hold to talk: Pause".into(), "Mute: no key".into()],
        }
    }

    #[test]
    fn a_shortcuts_prompt_lists_each_shortcut_and_grows_to_fit() {
        let input = two_shortcuts();
        let f = build_form(Kind::Shortcuts, &input);
        assert_eq!(client_size(&input), (391, 131 + 2 * ITEM_H));
        let texts: Vec<&str> = f
            .items
            .iter()
            .filter_map(|i| match &i.control {
                Control::Label { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Hold to talk: Pause"), "{texts:?}");
        assert!(texts.contains(&"Mute: no key"), "{texts:?}");
        assert_eq!(f.item("allow").unwrap().rect.y, BUTTON_Y + 2 * ITEM_H);
    }

    #[test]
    fn a_long_list_ends_in_how_many_more() {
        let items: Vec<String> = (0..10).map(|i| format!("s{i}: F{i}")).collect();
        let shown = listed(&items);
        assert_eq!(shown.len(), MAX_ITEMS + 1);
        assert_eq!(shown.last().unwrap(), "and 4 more");
    }

    #[test]
    fn the_shortcuts_prompt_says_where_to_change_the_keys() {
        assert_eq!(
            Kind::Shortcuts.prompt("Taltos"),
            "Taltos wants to use keyboard shortcuts."
        );
        assert_eq!(
            Kind::Shortcuts.note(),
            "Change the keys on Reprise Settings' Keyboard page."
        );
    }

    /// Every kind's prompt, for an app that sends no name, and note fit the
    /// text column on one line in each UI face installed.
    #[test]
    fn every_prompt_and_note_fit_the_text_column() {
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::FontConfig;
        let column = CLIENT.0 - TEXT_X - 11;
        for face in ["Tahoma", "DejaVu Sans"] {
            let cfg = FontConfig {
                family: vec![face.into()],
                ..FontConfig::default()
            };
            let text = TextRenderer::load(&cfg);
            if !text.has_face(face) {
                continue;
            }
            for kind in Kind::ALL {
                for line in [kind.prompt(&who("")), kind.note().into()] {
                    let w = text.measure(&line, false, 11);
                    assert!(
                        w <= column,
                        "{line:?} is {w} px in {face}, the column {column} px"
                    );
                }
            }
        }
    }

    /// The Shortcuts prompt drawn with two shortcuts: each listed line
    /// inked under the prompt, the note under the list, and the buttons
    /// two rows lower than in a bare prompt.
    #[test]
    fn shortcuts_prompt_pixels() {
        use reprisekit::classic::widgets::BUTTON_H;
        use reprisekit::controls::form;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let text = TextRenderer::load(&FontConfig::default());
        let input = two_shortcuts();
        let f = build_form(Kind::Shortcuts, &input);
        let (w, h) = client_size(&input);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        form::draw(&mut c, &text, &theme.paint(), &f, true, px);
        let inked = |y0: i32| {
            (y0..y0 + LINE_H)
                .any(|y| (TEXT_X..TEXT_X + 60).any(|x| c.get(x, y) == cs.button_text.argb()))
        };
        assert!(inked(12), "prompt inked on the row from y 12");
        assert!(inked(33), "first shortcut inked on the row from y 33");
        assert!(inked(49), "second shortcut inked on the row from y 49");
        assert!(inked(70), "note inked on the row from y 70");
        let y = BUTTON_Y + 2 * ITEM_H;
        assert_eq!(y, 129, "buttons two rows down, at y 129");
        assert_eq!(
            c.get(230, y + BUTTON_H - 2),
            cs.button_shadow.argb(),
            "Allow's inner shadow on its bottom row at y {}",
            y + BUTTON_H - 2
        );
        assert_eq!(
            c.get(230, y),
            cs.button_hilight.argb(),
            "Allow's raised edge at y 129"
        );
    }

    /// Writes target/permit-shortcuts.png for a visual check.
    #[test]
    #[ignore]
    fn render_shortcuts() {
        use reprisekit::controls::form;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let input = Input {
            offer_remember: true,
            items: (0..8)
                .map(|i| format!("Shortcut {i}: Ctrl+Alt+F{i}"))
                .collect(),
            ..two_shortcuts()
        };
        let f = build_form(Kind::Shortcuts, &input);
        let (w, h) = client_size(&input);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(&mut c, &text, &theme.paint(), &f, true, theme.fonts.size_px);
        reprisekit::raster::write_preview(&c, "permit-shortcuts.png");
    }
}
