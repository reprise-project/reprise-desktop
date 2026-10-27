//! The share dialog: a tree of screens and windows, a Remember check box, Share
//! and Cancel (the default, so Enter never shares by accident). Runs as our own
//! child (`reprise-portal --pick`): `pick` feeds it an `Input` on stdin, reads
//! the chosen index on stdout, and kills it when the frontend cancels.

use crate::dialog::{self, Prompt};
use crate::wayland::Target;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::treeview::{Node, TreeView};
use reprisekit::raster::{Canvas, Rect};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct Entry {
    pub label: String,
    pub icon: String,
    pub screen: bool,
    pub target: Target,
}

pub const CLIENT: (i32, i32) = (391, 441);
const GROUP: Rect = Rect::new(9, 66, 368, 288);
const TREE: Rect = Rect::new(24, 81, 338, 210);
const BUTTON_Y: i32 = 409;
const REMEMBER: Rect = Rect::new(9, 366, 13, 13);
const HEADER_ICON: &str = "preferences-desktop-display";
pub const SCREEN_ICON: &str = "computer";
const FALLBACK_ICON: &str = "application-x-executable";

/// What the picked screen or window is for, which words the dialog.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    #[default]
    Share,
    Screenshot,
}

impl Purpose {
    fn title(self) -> &'static str {
        match self {
            Purpose::Share => "Share your screen",
            Purpose::Screenshot => "Screenshot",
        }
    }

    fn prompt(self, who: &str) -> String {
        match self {
            Purpose::Share => format!("{who} wants to see your screen or a window."),
            Purpose::Screenshot => format!("{who} wants a picture of a screen or a window."),
        }
    }

    fn note(self) -> &'static str {
        match self {
            Purpose::Share => "Choose one to share. It will see everything shown on it.",
            Purpose::Screenshot => "Choose one. It will get a copy of what it shows.",
        }
    }

    fn accept(self) -> &'static str {
        match self {
            Purpose::Share => "&Share",
            Purpose::Screenshot => "&Take",
        }
    }
}

/// Show the dialog, modal on `parent`; returns the chosen target and whether
/// Remember was ticked, or None on Cancel or a frontend `Close`. Blocks.
pub fn pick(
    purpose: Purpose,
    app: &str,
    entries: Vec<Entry>,
    preselect: Option<usize>,
    offer_remember: bool,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Option<(Target, bool)> {
    let input = Input {
        purpose,
        parent,
        app: app.replace('\n', " "),
        preselect,
        offer_remember,
        rows: entries
            .iter()
            .map(|e| Row {
                screen: e.screen,
                icon: e.icon.clone(),
                label: e.label.replace(['\n', '\t'], " "),
            })
            .collect(),
    };
    let out = dialog::run_toml(&["--pick"], &input, &cancel)?;
    let (i, remember) = parse_choice(&out)?;
    entries.into_iter().nth(i).map(|e| (e.target, remember))
}

fn parse_choice(out: &str) -> Option<(usize, bool)> {
    let mut words = out.split_whitespace();
    let i = words.next()?.parse().ok()?;
    let remember = words.next() == Some("remember");
    Some((i, remember))
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
struct Row {
    screen: bool,
    icon: String,
    label: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Input {
    #[serde(default)]
    purpose: Purpose,
    parent: Option<String>,
    app: String,
    preselect: Option<usize>,
    offer_remember: bool,
    rows: Vec<Row>,
}

pub fn run_dialog() {
    let Some(input) = dialog::read_input::<Input>() else {
        return;
    };
    if input.rows.is_empty() {
        return;
    }
    let Input {
        purpose,
        parent,
        app,
        preselect,
        offer_remember,
        rows,
    } = input;
    let chosen: Arc<Mutex<Option<(usize, bool)>>> = Arc::new(Mutex::new(None));
    let mut icons = dialog::icons();
    let form = build_form(
        purpose,
        &rows,
        preselect,
        offer_remember,
        app.trim(),
        |name| icons.get(name, 16),
    );
    let icon = icons.get(HEADER_ICON, dialog::ICON_PX);
    repriseapp::run(Box::new(prompt(
        purpose,
        rows,
        form,
        icon,
        parent,
        chosen.clone(),
    )))
    .unwrap_or_else(|e| e.exit());
    let picked = *chosen.lock().unwrap();
    match picked {
        Some((i, false)) => println!("{i}"),
        Some((i, true)) => println!("{i} remember"),
        None => {}
    }
}

fn build_tree(
    rows: &[Row],
    mut icon_for: impl FnMut(&str) -> Option<Canvas>,
) -> (TreeView, Vec<usize>) {
    let mut roots = Vec::new();
    let mut at = vec![0usize; rows.len()];
    let folder = icon_for("folder");
    for (screen, title) in [(true, "Screens:"), (false, "Windows:")] {
        let mut branch = Node::new(title, true);
        branch.icon = folder.clone();
        branch.expanded = true;
        for (i, r) in rows.iter().enumerate().filter(|(_, r)| r.screen == screen) {
            let mut n = Node::new(&r.label, false);
            n.icon = icon_for(&r.icon).or_else(|| icon_for(FALLBACK_ICON));
            at[i] = branch.children.len();
            branch.children.push(n);
        }
        if !branch.children.is_empty() {
            roots.push(branch);
        }
    }
    let tv = TreeView::new(roots, reprisekit::controls::treeview::Style::Plain);
    let mut index = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let branch = tv
            .roots
            .iter()
            .position(|b| (b.label == "Screens:") == r.screen)
            .expect("branch");
        let before: usize = tv.roots[..branch]
            .iter()
            .map(|b| 1 + b.children.len())
            .sum();
        index.push(before + 1 + at[i]);
    }
    (tv, index)
}

fn build_form(
    purpose: Purpose,
    rows: &[Row],
    preselect: Option<usize>,
    offer_remember: bool,
    app: &str,
    icon_for: impl FnMut(&str) -> Option<Canvas>,
) -> Form {
    let (w, _) = CLIENT;
    let (mut tree, index) = build_tree(rows, icon_for);
    // `select` reads only rows in view; any look's sizes serve.
    let cm = ControlMetrics::CLASSIC;
    let lay = reprisekit::controls::treeview::layout_without_text(&tree, TREE, &cm);
    let start = preselect.and_then(|i| index.get(i).copied()).unwrap_or(1);
    tree.select(start, &lay);
    let mut items = Vec::from(dialog::header(
        w,
        purpose.prompt(&crate::permit::who(app)),
        purpose.note().into(),
    ));
    items.extend([
        Item::new("group", GROUP, Control::Group("Sources".into())),
        Item::new("tree", TREE, Control::Tree(tree)),
    ]);
    if offer_remember {
        items.push(Item::new(
            "remember",
            REMEMBER,
            Control::Check {
                label: "&Remember this choice".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ));
    }
    items.extend(dialog::buttons(
        GROUP.right() - 11,
        BUTTON_Y,
        ("share", purpose.accept()),
        ("cancel", "Cancel"),
    ));
    let mut f = Form::new(items);
    f.focus_id("tree");
    f
}

fn prompt(
    purpose: Purpose,
    rows: Vec<Row>,
    form: Form,
    icon: Option<Canvas>,
    parent: Option<String>,
    chosen: Arc<Mutex<Option<(usize, bool)>>>,
) -> Prompt {
    Prompt {
        parent,
        modal: true,
        on_cancel: Box::new(|_, _| {}),
        title: purpose.title().into(),
        size: CLIENT,
        form,
        icon,
        accept: "share",
        cancel: "cancel",
        on_accept: Box::new(move |form| match selection(&rows, form) {
            Some(i) => {
                let remember = form.checked("remember").unwrap_or(false);
                *chosen.lock().unwrap() = Some((i, remember));
                true
            }
            None => false,
        }),
    }
}

fn selection(rows: &[Row], form: &Form) -> Option<usize> {
    let tree = match &form.item("tree")?.control {
        Control::Tree(tv) => tv,
        _ => return None,
    };
    let path = tree.selected.as_ref()?;
    let [branch, child] = path[..] else {
        return None;
    };
    let screen = tree.roots.get(branch)?.label == "Screens:";
    rows.iter()
        .enumerate()
        .filter(|(_, r)| r.screen == screen)
        .nth(child)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::KeyInput;
    use repriseapp::keys::dialog_key;
    use reprisekit::controls::form;
    use reprisekit::textedit::keysym;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                screen: false,
                icon: "chromium".into(),
                label: "gdm 0 - Chromium".into(),
            },
            Row {
                screen: true,
                icon: SCREEN_ICON.into(),
                label: "Virtual-1 (1920x1080)".into(),
            },
            Row {
                screen: false,
                icon: "".into(),
                label: "foot".into(),
            },
        ]
    }

    #[test]
    fn pick_input_survives_the_pipe() {
        let input = Input {
            purpose: Purpose::Screenshot,
            parent: Some("h".into()),
            app: "Chromium".into(),
            preselect: Some(1),
            offer_remember: false,
            rows: vec![Row {
                screen: false,
                icon: "foot".into(),
                label: "a\tb\nc".into(),
            }],
        };
        let back: Input = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn screens_come_first_and_indexes_map_back() {
        let (tv, index) = build_tree(&rows(), |_| None);
        let labels: Vec<String> = tv
            .visible()
            .iter()
            .map(|r| tv.node(&r.path).unwrap().label.clone())
            .collect();
        assert_eq!(
            labels,
            [
                "Screens:",
                "Virtual-1 (1920x1080)",
                "Windows:",
                "gdm 0 - Chromium",
                "foot"
            ]
        );
        assert_eq!(index, [3, 1, 4]);
    }

    #[test]
    fn alt_letter_is_an_access_key() {
        let key = |sym: u32, utf8: &str, alt: bool| KeyInput {
            sym,
            utf8: Some(utf8.into()),
            mods: repriseapp::Mods {
                ctrl: false,
                shift: false,
                alt,
            },
            edit: reprisekit::textedit::key_for(sym, false, false, Some(utf8)),
        };
        assert_eq!(
            dialog_key(&key(0x61, "a", true)),
            Some(form::Key::Access('a'))
        );
        assert_eq!(
            dialog_key(&key(0x41, "A", true)),
            Some(form::Key::Access('A'))
        );
        assert_eq!(
            dialog_key(&key(0x61, "a", false)),
            Some(form::Key::Edit(reprisekit::textedit::EditKey::Insert(
                "a".into()
            )))
        );
        assert_eq!(
            dialog_key(&KeyInput {
                sym: keysym::ESCAPE,
                utf8: None,
                mods: repriseapp::Mods {
                    ctrl: false,
                    shift: false,
                    alt: true
                },
                edit: None,
            }),
            Some(form::Key::Escape)
        );
    }

    #[test]
    fn the_picker_fits_its_text() {
        for purpose in [Purpose::Share, Purpose::Screenshot] {
            let over = crate::dialog::overflows(|_, _| {
                build_form(purpose, &rows(), Some(1), true, "Chromium", |_| None)
            });
            assert!(over.is_empty(), "{purpose:?}: {}", over.join("\n"));
        }
    }

    #[test]
    #[ignore]
    fn render_picker() {
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = reprisekit::text::TextRenderer::load(&FontConfig::default());
        let form = build_form(Purpose::Share, &rows(), Some(1), true, "Chromium", |_| None);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &form,
            true,
            theme.fonts.size_px,
        );
        reprisekit::raster::write_preview(&c, "picker-preview.png");
    }

    #[test]
    fn remember_is_offered_only_when_asked_and_starts_off() {
        assert_eq!(
            build_form(Purpose::Share, &rows(), None, false, "", |_| None).checked("remember"),
            None
        );
        assert_eq!(
            build_form(Purpose::Share, &rows(), None, true, "", |_| None).checked("remember"),
            Some(false)
        );
    }

    #[test]
    fn only_a_ticked_remember_comes_back_as_remember() {
        assert_eq!(parse_choice("2\n"), Some((2, false)));
        assert_eq!(parse_choice("2 remember\n"), Some((2, true)));
        assert_eq!(parse_choice(""), None);
    }

    #[test]
    fn the_prompt_names_the_app() {
        let f = build_form(Purpose::Share, &rows(), None, false, "Chromium", |_| None);
        let Some(Control::Label { text, .. }) = f.item("prompt").map(|i| &i.control) else {
            panic!("prompt label");
        };
        assert!(
            text.starts_with("Chromium wants"),
            "prompt starts with the app name, got {text:?}"
        );
    }

    #[test]
    fn enter_on_the_preselected_tree_shares_nothing() {
        let chosen = Arc::new(Mutex::new(None));
        let form = build_form(Purpose::Share, &rows(), Some(1), false, "Chromium", |_| {
            None
        });
        let mut p = prompt(Purpose::Share, rows(), form, None, None, chosen.clone());
        assert_eq!(
            selection(&rows(), &p.form),
            Some(1),
            "the last pick starts selected"
        );
        let ev = p.form.key(form::Key::Enter);
        p.decide(ev);
        assert_eq!(*chosen.lock().unwrap(), None, "Enter shares nothing");
        assert_eq!(p.form.default_button(), p.form.index_of("cancel"));
    }

    #[test]
    fn a_branch_row_is_no_selection() {
        let (mut tree, index) = build_tree(&rows(), |_| None);
        let lay = reprisekit::controls::treeview::layout_without_text(
            &tree,
            TREE,
            &ControlMetrics::CLASSIC,
        );
        tree.select(index[2], &lay);
        let mut form = Form::new(vec![Item::new("tree", TREE, Control::Tree(tree))]);
        assert_eq!(selection(&rows(), &form), Some(2));
        if let Some(Control::Tree(tv)) = form.item_mut("tree").map(|i| &mut i.control) {
            tv.select(0, &lay);
        }
        assert_eq!(selection(&rows(), &form), None);
    }
}
