//! "Allow this program to ...?": the RemoteDesktop, Screenshot,
//! InputCapture, GlobalShortcuts, Background and Wallpaper prompt, run as our own child
//! (`--permit <kind>`, see `dialog`). The parent writes an `Input` on stdin
//! (the asking window, the app's display name, whether to offer Remember,
//! the lines to list) and reads `yes` (or `yes remember`) on stdout for
//! Allow, `no` for Deny, and nothing when the prompt closed unanswered.

use crate::dialog::{self, Prompt, TEXT_X};
use reprisekit::classic::assets::MONITOR_SIZE;
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item, Screen};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::tr;
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
/// The space a preview takes: the monitor and a line's gap under it.
const PREVIEW_H: i32 = MONITOR_SIZE.1 + LINE_H;
const PREVIEW_MAX: (u32, u32) = (304, 224);
/// The preview's status line, right of the monitor: its gap from it, its drop below the
/// monitor's top and its height in lines.
const STATUS_GAP: i32 = 8;
const STATUS_DY: i32 = 16;
const STATUS_LINES: i32 = 4;
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
    /// A new desktop picture.
    Wallpaper,
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

    fn arg(self) -> &'static str {
        match self {
            Devices::Keyboard => "keyboard",
            Devices::Pointer => "pointer",
            Devices::Both => "both",
        }
    }
}

impl Kind {
    const ALL: [Kind; 10] = [
        Kind::RemoteControl(Devices::Keyboard),
        Kind::RemoteControl(Devices::Pointer),
        Kind::RemoteControl(Devices::Both),
        Kind::Screenshot,
        Kind::InputCapture(Devices::Keyboard),
        Kind::InputCapture(Devices::Pointer),
        Kind::InputCapture(Devices::Both),
        Kind::Shortcuts,
        Kind::Background,
        Kind::Wallpaper,
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
            Kind::Wallpaper => "wallpaper",
        }
    }

    fn from_arg(arg: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.arg() == arg)
    }

    fn title(self) -> String {
        match self {
            Kind::RemoteControl(_) => tr!("permit-remote-title"),
            Kind::Screenshot => tr!("permit-screenshot-title"),
            Kind::InputCapture(_) => tr!("permit-capture-title"),
            Kind::Shortcuts => tr!("permit-shortcuts-title"),
            Kind::Background => tr!("permit-background-title"),
            Kind::Wallpaper => tr!("permit-wallpaper-title"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::RemoteControl(_) => "input-keyboard",
            Kind::Screenshot => "camera-photo",
            Kind::InputCapture(_) => "input-mouse",
            Kind::Shortcuts => "input-keyboard",
            Kind::Background => "system-run",
            Kind::Wallpaper => "preferences-desktop-wallpaper",
        }
    }

    fn prompt(self, who: &str) -> String {
        match self {
            Kind::RemoteControl(d) => tr!("permit-remote-prompt", who = who, devices = d.arg()),
            Kind::Screenshot => tr!("permit-screenshot-prompt", who = who),
            Kind::InputCapture(d) => tr!("permit-capture-prompt", who = who, devices = d.arg()),
            Kind::Shortcuts => tr!("permit-shortcuts-prompt", who = who),
            Kind::Background => tr!("permit-background-prompt", who = who),
            Kind::Wallpaper => tr!("permit-wallpaper-prompt", who = who),
        }
    }

    fn note(self) -> String {
        match self {
            Kind::RemoteControl(Devices::Keyboard) => tr!("permit-remote-note-keyboard"),
            Kind::RemoteControl(Devices::Pointer) => tr!("permit-remote-note-pointer"),
            Kind::RemoteControl(Devices::Both) => tr!("permit-remote-note-both"),
            Kind::Screenshot => tr!("permit-screenshot-note"),
            Kind::InputCapture(Devices::Both) => tr!("permit-capture-note-both"),
            Kind::InputCapture(_) => tr!("permit-capture-note-one"),
            Kind::Shortcuts => tr!("permit-shortcuts-note"),
            Kind::Background => tr!("permit-background-note"),
            Kind::Wallpaper => tr!("permit-wallpaper-note"),
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
        preview: None,
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

/// The permission store entry, table and id, keeping the apps whose Allow was remembered. Reprise
/// Settings lists and forgets them, so the format (app id to `["yes", grant...]`) is shared with it.
#[derive(Clone, Copy, Debug)]
pub struct Store {
    pub table: &'static str,
    pub id: &'static str,
}

const STORE_YES: &str = "yes";

/// How a prompt backed by a [`Store`] gets the user's consent.
#[derive(Debug, PartialEq, Eq)]
enum Consent {
    /// An Allow remembered in the store: no prompt.
    Remembered,
    Ask {
        offer_remember: bool,
    },
}

/// The consent `app_id` needs for `grant`, given `stored`, its store lookup: a stored `yes` is
/// remembered when the words after it hold all of `grant`. Remember is offered only where an
/// answer can be kept: an empty app id has nothing to file it under, an unreachable store no place.
fn consent(
    app_id: &str,
    grant: &[&str],
    stored: impl FnOnce() -> zbus::Result<Option<Vec<String>>>,
) -> Consent {
    if app_id.is_empty() {
        return Consent::Ask {
            offer_remember: false,
        };
    }
    let covers = |p: &[String]| match p.split_first() {
        Some((yes, held)) => yes == STORE_YES && grant.iter().all(|g| held.iter().any(|h| h == g)),
        None => false,
    };
    match stored() {
        Ok(Some(p)) if covers(&p) => Consent::Remembered,
        Ok(_) => Consent::Ask {
            offer_remember: true,
        },
        Err(e) => {
            log::warn!("no permission store for {app_id:?}: {e}");
            Consent::Ask {
                offer_remember: false,
            }
        }
    }
}

/// `ask_with`, unless `store` remembers `app_id`'s Allow of `grant`; true when allowed. An Allow
/// with Remember is kept in `store` as `yes` and `grant`, a Deny never. Blocking.
pub fn ask_remembered(
    kind: Kind,
    store: Store,
    app_id: &str,
    grant: &[&str],
    mut input: Input,
    cancel: &AtomicBool,
) -> bool {
    use reprisebus::permissions::{app_permission, set_app_permission};
    let lookup = || app_permission(store.table, store.id, app_id);
    input.offer_remember = match consent(app_id, grant, lookup) {
        Consent::Remembered => {
            log::info!("{kind:?} for {app_id:?}: the permission store allows it");
            return true;
        }
        Consent::Ask { offer_remember } => offer_remember,
    };
    match ask_with(kind, input, cancel) {
        Answer::Allow { remember: true } => {
            let kept: Vec<&str> = std::iter::once(STORE_YES)
                .chain(grant.iter().copied())
                .collect();
            match set_app_permission(store.table, store.id, app_id, &kept) {
                Ok(()) => log::info!("{kind:?}: remembered the Allow for {app_id:?}"),
                Err(e) => log::warn!("{kind:?}: could not remember {app_id:?}: {e}"),
            }
            true
        }
        a => a.allowed(),
    }
}

/// What the parent tells the prompt, as TOML on stdin.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Input {
    pub parent: Option<String>,
    pub app: String,
    pub offer_remember: bool,
    /// Lines listed under the prompt, such as the shortcuts asked for.
    #[serde(default)]
    pub items: Vec<String>,
    /// A picture file shown on a monitor under the listed lines.
    #[serde(default)]
    pub preview: Option<String>,
}

/// The preview's status line while the picture decodes, and when it cannot be shown.
fn loading() -> String {
    tr!("permit-preview-loading")
}

fn cannot_show() -> String {
    tr!("permit-preview-cannot-show")
}

/// The picture at `path` fitted inside [`PREVIEW_MAX`], None where the desktop would show none.
fn preview(path: &std::path::Path) -> Option<Canvas> {
    let file = std::fs::File::open(path).ok()?;
    crate::wallpaperpreview::preview(file, PREVIEW_MAX)
        .map_err(|why| log::info!("the Wallpaper prompt shows no preview: {why:?}"))
        .ok()
}

/// The decoded preview on the monitor, or when there is none, [`CANNOT_SHOW`]. Allow lights only
/// for a picture shown.
fn show_preview(form: &mut Form, picture: Option<Canvas>) {
    let status = match picture {
        Some(picture) => {
            if let Some(Control::Monitor(screen)) = form.item_mut("preview").map(|i| &mut i.control)
            {
                *screen = Screen::Picture(picture);
            }
            form.set_enabled("allow", true);
            Vec::new()
        }
        None => vec![cannot_show()],
    };
    form.set_lines("status", &status);
}

fn allow_enabled(form: &Form) -> bool {
    matches!(
        form.item("allow").map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )
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
    let mut p = prompt(kind, form, size, icon, input.parent, answer.clone());
    p.job = input.preview.map(|path| {
        dialog::Job::new(move || {
            let picture = preview(std::path::Path::new(&path));
            Box::new(move |form: &mut Form| show_preview(form, picture))
        })
    });
    repriseapp::run(Box::new(p)).unwrap_or_else(|e| e.exit());
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
        tr!("permit-unknown-app")
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
        shown.push(tr!("permit-more", count = items.len() - MAX_ITEMS));
    }
    shown
}

fn list_h(input: &Input) -> i32 {
    listed(&input.items).len() as i32 * ITEM_H
}

/// How far the rows under the prompt move down: the listed lines and the preview.
fn grown_h(input: &Input) -> i32 {
    list_h(input)
        + if input.preview.is_some() {
            PREVIEW_H
        } else {
            0
        }
}

fn client_size(input: &Input) -> (i32, i32) {
    (CLIENT.0, CLIENT.1 + grown_h(input))
}

/// The dialog's controls: who is asking, the listed lines, the preview, what it gets, the
/// Remember check box when offered, Allow and Deny. Deny is the default button, so Enter never
/// grants by accident, and Remember starts off. A consent form ([`Form::set_consent`]): a bare
/// letter presses nothing. A preview starts black beside [`loading`], with Allow greyed until
/// [`show_preview`] shows the picture.
fn build_form(kind: Kind, input: &Input) -> Form {
    let (w, _) = CLIENT;
    let dy = grown_h(input);
    let [prompt, mut note] = dialog::header(w, kind.prompt(&who(input.app.trim())), kind.note());
    note.rect.y += dy;
    let mut items = vec![prompt];
    for (i, (id, line)) in ITEM_IDS.into_iter().zip(listed(&input.items)).enumerate() {
        let y = ITEMS_Y + i as i32 * ITEM_H;
        let line = reprisekit::text::escape_mnemonic(&line);
        items.push(Item::label_at(id, TEXT_X, y, w - TEXT_X - 11, &line));
    }
    if input.preview.is_some() {
        let (mw, mh) = MONITOR_SIZE;
        let at = Rect::new(TEXT_X, ITEMS_Y + list_h(input), mw, mh);
        items.push(Item::new("preview", at, Control::Monitor(Screen::Black)));
        let x = at.right() + STATUS_GAP;
        let status = Rect::new(x, at.y + STATUS_DY, w - 11 - x, STATUS_LINES * LINE_H);
        let loading = Control::Lines(vec![loading()]);
        items.push(Item::new("status", status, loading).live());
    }
    items.push(note);
    if input.offer_remember {
        items.push(Item::new(
            "remember",
            Rect::new(TEXT_X, REMEMBER_Y + dy, 13, 13),
            Control::Check {
                label: tr!("remember-choice"),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ));
    }
    items.extend(dialog::buttons(
        w - 11,
        BUTTON_Y + dy,
        ("allow", &tr!("allow")),
        ("deny", &tr!("deny")),
    ));
    let mut f = Form::new(items);
    f.focus_id("deny");
    f.set_consent();
    if input.preview.is_some() {
        f.set_enabled("allow", false);
    }
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
        title: kind.title(),
        size,
        form,
        icon,
        accept: "allow",
        cancel: "deny",
        on_accept: Box::new(move |form| {
            if !allow_enabled(form) {
                return false;
            }
            let remember = form.checked("remember").unwrap_or(false);
            *answer.lock().unwrap() = Answer::Allow { remember };
            true
        }),
        job: None,
        fit: None,
        answered: false,
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
            preview: None,
        };
        for kind in Kind::ALL {
            let over = crate::dialog::overflows(|_, _| build_form(kind, &input));
            assert!(over.is_empty(), "{kind:?}:\n{}", over.join("\n"));
        }
        let previewed = Input {
            preview: Some("/proc/1/fd/9".into()),
            ..input
        };
        for status in [loading(), cannot_show()] {
            let over = crate::dialog::overflows(|_, _| {
                let mut f = build_form(Kind::Wallpaper, &previewed);
                if status == cannot_show() {
                    show_preview(&mut f, None);
                }
                f
            });
            assert!(over.is_empty(), "{status:?}:\n{}", over.join("\n"));
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
                preview: None,
            };
            let back: Input = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
            assert_eq!(back, input);
        }
    }

    #[test]
    fn a_bare_letter_answers_no_permission_prompt() {
        use reprisekit::controls::form::{Event, Key};
        let input = Input {
            app: "Chromium".into(),
            offer_remember: true,
            ..Default::default()
        };
        for kind in Kind::ALL {
            let mut f = build_form(kind, &input);
            for ch in ['a', 'r', 'd'] {
                assert_eq!(f.key(Key::Char(ch)), Event::None, "{kind:?}: {ch}");
            }
            assert_eq!(f.checked("remember"), Some(false), "{kind:?}");
            assert_eq!(f.key(Key::Access('a')), Event::Pressed("allow"), "{kind:?}");
        }
    }

    #[test]
    fn up_then_enter_answers_no_permission_prompt_with_allow() {
        use reprisekit::controls::form::{Event, Key};
        for kind in Kind::ALL {
            let mut f = build_form(kind, &Input::default());
            f.key(Key::Up);
            assert_eq!(f.key(Key::Enter), Event::Pressed("deny"), "{kind:?}");
            f.key(Key::Tab { back: true });
            assert_ne!(f.key(Key::Space), Event::Pressed("allow"), "{kind:?}");
        }
    }

    /// A Deskflow remote control prompt, Remember offered or not.
    fn deskflow(offer_remember: bool) -> Form {
        let input = Input {
            parent: None,
            app: "Deskflow".into(),
            offer_remember,
            items: Vec::new(),
            preview: None,
        };
        build_form(Kind::RemoteControl(Devices::Both), &input)
    }

    #[test]
    fn a_preview_puts_a_monitor_under_the_file_name_and_moves_the_rows_under_it() {
        let input = Input {
            app: "Wallpapers".into(),
            items: vec!["dunes.jpg".into()],
            preview: Some("/pictures/dunes.jpg".into()),
            ..Default::default()
        };
        let kind = Kind::Wallpaper;
        let f = build_form(kind, &input);
        let monitor = f.item("preview").unwrap();
        assert_eq!(monitor.control, Control::Monitor(Screen::Black));
        assert_eq!(monitor.rect, Rect::new(TEXT_X, ITEMS_Y + ITEM_H, 191, 168));
        let dy = ITEM_H + PREVIEW_H;
        assert_eq!(f.item("allow").unwrap().rect.y, BUTTON_Y + dy);
        assert_eq!(client_size(&input), (CLIENT.0, CLIENT.1 + dy));
        let bare = Input {
            preview: None,
            ..input
        };
        assert!(build_form(kind, &bare).item("preview").is_none());
        assert_eq!(client_size(&bare), (CLIENT.0, CLIENT.1 + ITEM_H));
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
    fn a_press_after_the_prompt_answered_or_closed_changes_nothing() {
        use reprisekit::controls::form::Event;
        let answer_to = |events: [Event; 2]| {
            let answer = Arc::new(Mutex::new(Answer::Dismissed));
            let kind = Kind::Shortcuts;
            let mut p = prompt(kind, deskflow(false), CLIENT, None, None, answer.clone());
            let [first, second] = events;
            assert!(p.decide(first).quit, "the first answer closes the prompt");
            assert!(!p.decide(second).quit, "a second answers nothing");
            *answer.lock().unwrap()
        };
        let (allow, deny) = (Event::Pressed("allow"), Event::Pressed("deny"));
        assert_eq!(answer_to([deny, allow]), Answer::Deny);
        assert_eq!(answer_to([allow, deny]), Answer::Allow { remember: false });
        assert_eq!(answer_to([Event::Cancel, allow]), Answer::Dismissed);
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
            preview: None,
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
                for line in [kind.prompt(&who("")), kind.note()] {
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

    /// A Wallpaper prompt with a preview, Remember offered.
    fn wallpaper() -> (Input, Form) {
        let input = Input {
            app: "Wallpapers".into(),
            offer_remember: true,
            items: vec!["dunes.jxl".into()],
            preview: Some("/proc/1/fd/9".into()),
            ..Default::default()
        };
        let f = build_form(Kind::Wallpaper, &input);
        (input, f)
    }

    /// What pressing Allow on `f` answers.
    fn allow_answers(f: Form) -> Answer {
        let answer = Arc::new(Mutex::new(Answer::Dismissed));
        let mut p = prompt(Kind::Wallpaper, f, CLIENT, None, None, answer.clone());
        p.decide(reprisekit::controls::form::Event::Pressed("allow"));
        *answer.lock().unwrap()
    }

    fn status(f: &Form) -> Vec<String> {
        match &f.item("status").unwrap().control {
            Control::Lines(l) => l.clone(),
            other => panic!("the status is lines, not {other:?}"),
        }
    }

    /// 304x224 of a vertical gradient, a decoded preview.
    fn picture() -> Canvas {
        let mut c = Canvas::new(PREVIEW_MAX.0, PREVIEW_MAX.1);
        for (i, p) in c.pixels.iter_mut().enumerate() {
            let y = (i as u32 / PREVIEW_MAX.0) as u8;
            *p = 0xFF00_0000 | u32::from(y) << 8 | 0x40;
        }
        c
    }

    #[test]
    fn a_previewed_prompt_greys_allow_until_the_picture_shows() {
        use reprisekit::controls::form::{Event, Key};
        let (_, mut f) = wallpaper();
        assert!(!allow_enabled(&f));
        assert_eq!(status(&f), [loading()]);
        assert_ne!(f.key(Key::Access('a')), Event::Pressed("allow"));
        assert_eq!(allow_answers(f.clone()), Answer::Dismissed);
        show_preview(&mut f, Some(picture()));
        assert!(allow_enabled(&f));
        assert!(status(&f).is_empty());
        assert_eq!(
            f.item("preview").unwrap().control,
            Control::Monitor(Screen::Picture(picture()))
        );
        assert_eq!(f.key(Key::Access('a')), Event::Pressed("allow"));
        assert_eq!(allow_answers(f), Answer::Allow { remember: false });
    }

    #[test]
    fn a_picture_that_cannot_be_previewed_cannot_be_allowed() {
        use reprisekit::controls::form::{Event, Key};
        let (_, mut f) = wallpaper();
        show_preview(&mut f, None);
        assert!(!allow_enabled(&f));
        assert_eq!(status(&f), [cannot_show()]);
        assert_eq!(
            f.item("preview").unwrap().control,
            Control::Monitor(Screen::Black)
        );
        assert_ne!(f.key(Key::Access('a')), Event::Pressed("allow"));
        assert_eq!(allow_answers(f), Answer::Dismissed);
    }

    #[test]
    fn a_prompt_without_a_preview_allows_at_once() {
        let input = Input {
            app: "Wallpapers".into(),
            ..Default::default()
        };
        let f = build_form(Kind::Wallpaper, &input);
        assert!(allow_enabled(&f) && f.item("status").is_none());
        assert_eq!(allow_answers(f), Answer::Allow { remember: false });
    }

    /// The Wallpaper prompt loading its preview, showing it and unable to show it, in each
    /// interface at 100% and 150% text.
    #[test]
    fn wallpaper_prompt_pixels() {
        use reprisekit::controls::form;
        use reprisekit::interface::Interface;
        use reprisekit::text::TextRenderer;
        reprisekit::pins::fonts();
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 150] {
                let theme = reprisekit::interfaces::theme_for(i).with_form_text(pct);
                let text = TextRenderer::load(&theme.fonts);
                for state in ["loading", "shown", "cannot-show"] {
                    let (input, mut f) = wallpaper();
                    match state {
                        "shown" => show_preview(&mut f, Some(picture())),
                        "cannot-show" => show_preview(&mut f, None),
                        _ => {}
                    }
                    let (w, h) = theme.text.size(client_size(&input));
                    let mut c = Canvas::new(w as u32, h as u32);
                    c.fill_rect(c.bounds(), theme.colors.button_face);
                    form::draw(&mut c, &text, &theme.paint(), &f, true, theme.fonts.size_px);
                    let hash = c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                        (h ^ u64::from(*p)).wrapping_mul(0x100000001b3)
                    });
                    got.push((format!("{}-{pct}-{state}", i.as_str()), hash));
                }
            }
        }
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/pins/wallpaper_prompt.txt"
        );
        reprisekit::pins::check_file(std::path::Path::new(file), &got);
    }

    /// Writes target/permit-wallpaper-<state>.png for a visual check.
    #[test]
    #[ignore]
    fn render_wallpaper() {
        use reprisekit::controls::form;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        for state in ["loading", "shown", "cannot-show"] {
            let (input, mut f) = wallpaper();
            match state {
                "shown" => show_preview(&mut f, Some(picture())),
                "cannot-show" => show_preview(&mut f, None),
                _ => {}
            }
            let (w, h) = client_size(&input);
            let mut c = Canvas::new(w as u32, h as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            form::draw(&mut c, &text, &theme.paint(), &f, true, theme.fonts.size_px);
            reprisekit::raster::write_preview(&c, &format!("permit-wallpaper-{state}.png"));
        }
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

    fn store(perms: &[&str]) -> impl FnOnce() -> zbus::Result<Option<Vec<String>>> {
        let perms = perms.iter().map(|p| p.to_string()).collect();
        move || Ok(Some(perms))
    }

    #[test]
    fn a_stored_yes_skips_the_prompt() {
        assert_eq!(
            consent("org.deskflow.deskflow", &[], store(&["yes"])),
            Consent::Remembered
        );
    }

    #[test]
    fn a_stored_yes_skips_the_prompt_only_for_what_it_granted() {
        let app = "org.deskflow.deskflow";
        let ask = Consent::Ask {
            offer_remember: true,
        };
        let both = ["keyboard", "pointer"];
        assert_eq!(
            consent(app, &["pointer"], store(&["yes", "pointer"])),
            Consent::Remembered
        );
        assert_eq!(
            consent(app, &["pointer"], store(&["yes", "keyboard", "pointer"])),
            Consent::Remembered
        );
        assert_eq!(
            consent(app, &both, store(&["yes", "pointer"])),
            ask,
            "a wider ask"
        );
        assert_eq!(consent(app, &["keyboard"], store(&["yes", "pointer"])), ask);
        assert_eq!(
            consent(app, &["pointer"], store(&["yes"])),
            ask,
            "a bare yes grants no device"
        );
        assert_eq!(consent(app, &["pointer"], store(&["no", "pointer"])), ask);
    }

    #[test]
    fn no_stored_yes_asks_with_remember() {
        let ask = Consent::Ask {
            offer_remember: true,
        };
        assert_eq!(consent("org.deskflow.deskflow", &[], || Ok(None)), ask);
        assert_eq!(consent("org.deskflow.deskflow", &[], store(&["no"])), ask);
        assert_eq!(consent("org.deskflow.deskflow", &[], store(&["ask"])), ask);
        assert_eq!(consent("org.deskflow.deskflow", &[], store(&[])), ask);
    }

    #[test]
    fn an_empty_app_id_asks_without_remember_and_without_a_lookup() {
        let lookup = || -> zbus::Result<Option<Vec<String>>> { panic!("looked up") };
        assert_eq!(
            consent("", &[], lookup),
            Consent::Ask {
                offer_remember: false
            }
        );
    }

    #[test]
    fn an_unreachable_store_asks_without_remember() {
        let lookup = || Err(zbus::Error::Failure("no store".into()));
        assert_eq!(
            consent("org.deskflow.deskflow", &[], lookup),
            Consent::Ask {
                offer_remember: false
            }
        );
    }
}
