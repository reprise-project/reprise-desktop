//! The share dialog: a tree of screens and windows, a Remember check box or a
//! screenshot's Delay box, Share and Cancel (the default, so Enter never shares by
//! accident). Runs as our own child (`reprise-portal --pick`): `pick` feeds it an
//! `Input` on stdin, reads the chosen index on stdout, and kills it when the frontend
//! cancels.

use crate::dialog::{self, Prompt};
use crate::wayland::Target;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::updown::{self, UpDown};
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
/// The Delay box's row, in Remember's place: its label, the up-down and its unit.
const DELAY_LABEL: Rect = Rect::new(9, 370, 44, 13);
const DELAY: Rect = Rect::new(57, 366, updown::WIDTH, updown::HEIGHT);
const DELAY_UNITS: Rect = Rect::new(DELAY.x + updown::WIDTH + 6, 370, 60, 13);
/// The Sources tree's least height on a short work area, five rows.
const TREE_MIN_H: i32 = 90;
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

/// What the row under the sources offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extra {
    Nothing,
    Remember,
    /// The seconds a screenshot waits after Take, starting at this many, 0 to
    /// [`repriseconf::config::SHOT_DELAY_MAX`].
    Delay(u8),
}

/// What the dialog's Share or Take chose.
#[derive(Debug, PartialEq, Eq)]
pub struct Picked {
    pub target: Target,
    pub remember: bool,
    /// Seconds; 0 without a Delay box.
    pub delay: u8,
}

/// Show the dialog, modal on `parent`; returns the chosen target, whether Remember was ticked and
/// the delay set, or None on Cancel or a frontend `Close`. Blocks.
pub fn pick(
    purpose: Purpose,
    app: &str,
    entries: Vec<Entry>,
    preselect: Option<usize>,
    extra: Extra,
    parent: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Option<Picked> {
    let input = Input {
        purpose,
        parent,
        app: app.replace('\n', " "),
        preselect,
        offer_remember: extra == Extra::Remember,
        delay: match extra {
            Extra::Delay(s) => Some(s),
            _ => None,
        },
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
    let c = parse_choice(&out)?;
    entries.into_iter().nth(c.index).map(|e| Picked {
        target: e.target,
        remember: c.remember,
        delay: c.delay,
    })
}

/// The dialog's answer on stdout: the row's index, then `remember` when ticked or `delay N`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Choice {
    index: usize,
    remember: bool,
    delay: u8,
}

impl Choice {
    fn line(self) -> String {
        match self {
            Choice { remember: true, .. } => format!("{} remember", self.index),
            Choice { delay: 0, .. } => self.index.to_string(),
            Choice { index, delay, .. } => format!("{index} delay {delay}"),
        }
    }
}

fn parse_choice(out: &str) -> Option<Choice> {
    let mut words = out.split_whitespace();
    let index = words.next()?.parse().ok()?;
    let mut c = Choice {
        index,
        remember: false,
        delay: 0,
    };
    match words.next() {
        Some("remember") => c.remember = true,
        Some("delay") => {
            let s: u8 = words.next()?.parse().ok()?;
            c.delay = s.min(repriseconf::config::SHOT_DELAY_MAX);
        }
        _ => {}
    }
    Some(c)
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
    /// The Delay box's first value, for a screenshot that offers one.
    #[serde(default)]
    delay: Option<u8>,
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
        delay,
        rows,
    } = input;
    let chosen: Arc<Mutex<Option<Choice>>> = Arc::new(Mutex::new(None));
    let mut icons = dialog::icons();
    let px = treeview::icon_px(dialog::start_forms().controls().text);
    let form = build_form(
        purpose,
        &rows,
        preselect,
        offer_remember,
        delay,
        app.trim(),
        |name| icons.get(name, px),
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
    if let Some(c) = *chosen.lock().unwrap() {
        println!("{}", c.line());
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
    delay: Option<u8>,
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
    if let Some(s) = delay {
        let max = repriseconf::config::SHOT_DELAY_MAX.into();
        items.extend([
            Item::label("delay_label", DELAY_LABEL, "&Delay:"),
            Item::new(
                "delay",
                DELAY,
                Control::UpDown(UpDown::new(s.into(), 0, max)),
            )
            .named("Delay"),
            Item::label("delay_units", DELAY_UNITS, "seconds"),
        ]);
    }
    items.extend(dialog::buttons(
        GROUP.right() - 11,
        BUTTON_Y,
        ("share", purpose.accept()),
        ("cancel", "Cancel"),
    ));
    let mut f = Form::new(items);
    f.focus_id("tree");
    f.set_consent();
    f
}

/// Lays the dialog out for a client `h` px tall at 100% text: short of [`CLIENT`], the Sources
/// tree gives up the difference down to [`TREE_MIN_H`] and the rows under it move up with it.
fn fit(form: &mut Form, h: i32) {
    let short = (CLIENT.1 - h).clamp(0, TREE.height - TREE_MIN_H);
    let cut = |r: Rect| Rect::new(r.x, r.y, r.width, r.height - short);
    let up = |r: Rect| Rect::new(r.x, r.y - short, r.width, r.height);
    let rects = [
        ("group", cut(GROUP)),
        ("tree", cut(TREE)),
        ("remember", up(REMEMBER)),
        ("delay_label", up(DELAY_LABEL)),
        ("delay", up(DELAY)),
        ("delay_units", up(DELAY_UNITS)),
    ];
    for (id, r) in rects {
        if let Some(it) = form.item_mut(id) {
            it.rect = r;
        }
    }
    for id in ["share", "cancel"] {
        if let Some(it) = form.item_mut(id) {
            it.rect.y = BUTTON_Y - short;
        }
    }
    if let Some(it) = form.item_mut("tree")
        && let Control::Tree(tv) = &mut it.control
    {
        let lay = reprisekit::controls::treeview::layout_without_text(
            tv,
            it.rect,
            &ControlMetrics::CLASSIC,
        );
        tv.follow(&lay);
    }
}

fn prompt(
    purpose: Purpose,
    rows: Vec<Row>,
    form: Form,
    icon: Option<Canvas>,
    parent: Option<String>,
    chosen: Arc<Mutex<Option<Choice>>>,
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
            Some(index) => {
                let delay = match form.item("delay").map(|i| &i.control) {
                    Some(Control::UpDown(u)) => u8::try_from(u.value()).unwrap_or(0),
                    _ => 0,
                };
                *chosen.lock().unwrap() = Some(Choice {
                    index,
                    remember: form.checked("remember").unwrap_or(false),
                    delay,
                });
                true
            }
            None => false,
        }),
        job: None,
        fit: Some(fit),
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
            delay: Some(4),
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
        for (purpose, remember, delay) in [
            (Purpose::Share, true, None),
            (Purpose::Screenshot, false, Some(3)),
        ] {
            let over = crate::dialog::overflows(|_, _| {
                build_form(
                    purpose,
                    &rows(),
                    Some(1),
                    remember,
                    delay,
                    "Chromium",
                    |_| None,
                )
            });
            assert!(over.is_empty(), "{purpose:?}: {}", over.join("\n"));
        }
    }

    /// On a 720x450 work area (a 1440x900 screen at scale 2, or a 720x450 one) the screenshot and
    /// share dialogs keep Take or Share and Cancel inside the window, at scale 1 and 2, under
    /// every interface, the picked source still picked.
    #[test]
    fn a_short_work_area_keeps_the_buttons_in_the_window() {
        use reprisekit::frame::{Chrome, FrameGeometry};
        use reprisekit::interface::Interface;
        use reprisekit::text::TextRenderer;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            let m = &theme.metrics;
            let total = (720, 450 - m.taskbar_height);
            let (_, h) =
                FrameGeometry::client_size_for_total(m, total.0, total.1, false, Chrome::DIALOG);
            assert!(h < CLIENT.1, "{i:?}: the dialog is taller than {h}");
            for (purpose, remember, delay) in [
                (Purpose::Share, true, None),
                (Purpose::Screenshot, false, Some(3)),
            ] {
                let form = build_form(purpose, &rows(), Some(2), remember, delay, "", |_| None);
                let mut p = prompt(purpose, rows(), form, None, None, Arc::default());
                let mut ctx = crate::dialog::test_ctx(100);
                ctx.theme = theme.clone();
                ctx.size = (CLIENT.0, h);
                repriseapp::App::event(
                    &mut p,
                    repriseapp::MAIN,
                    repriseapp::Event::Resized,
                    &mut ctx,
                );
                let at = |id| p.form.item(id).unwrap().rect;
                let (take, cancel) = (at("share"), at("cancel"));
                assert!(
                    cancel.bottom() <= h,
                    "{i:?} {purpose:?}: {cancel:?} ends by {h}"
                );
                let foot = at(if remember { "remember" } else { "delay" });
                assert!(at("group").bottom() <= foot.y && foot.bottom() <= take.y);
                assert_eq!(selection(&rows(), &p.form), Some(2), "the pick stays");
                for scale in [1, 2] {
                    let t = theme.scaled(scale);
                    let text = TextRenderer::load(&t.fonts);
                    let mut c = Canvas::new((CLIENT.0 * scale) as u32, (h * scale) as u32);
                    c.fill_rect(c.bounds(), t.colors.button_face);
                    form::draw(&mut c, &text, &t.paint(), &p.form, true, t.fonts.size_px);
                    let face = t.colors.button_face.argb();
                    for b in [take, cancel] {
                        let foot = (b.bottom() - 3) * scale..b.bottom() * scale;
                        let xs = b.x * scale..b.right() * scale;
                        assert!(
                            foot.flat_map(|y| xs.clone().map(move |x| (x, y)))
                                .any(|(x, y)| c.get(x, y) != face),
                            "{i:?} {purpose:?} x{scale}: {b:?}'s foot is drawn"
                        );
                    }
                }
            }
        }
    }

    /// The screenshot dialog with its Delay box, in each interface at scale 1 and 2.
    #[test]
    fn screenshot_picker_pixels() {
        use reprisekit::interface::Interface;
        use reprisekit::text::TextRenderer;
        reprisekit::pins::fonts();
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = reprisekit::theme::Theme {
                    access_keys: true,
                    ..reprisekit::interfaces::theme_for(i).scaled(scale)
                };
                let text = TextRenderer::load(&theme.fonts);
                let f = build_form(
                    Purpose::Screenshot,
                    &rows(),
                    Some(1),
                    false,
                    Some(3),
                    "Chromium",
                    |_| None,
                );
                let (w, h) = (CLIENT.0 * scale, CLIENT.1 * scale);
                let mut c = Canvas::new(w as u32, h as u32);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                form::draw(&mut c, &text, &theme.paint(), &f, true, theme.fonts.size_px);
                let hash = c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                    (h ^ u64::from(*p)).wrapping_mul(0x100000001b3)
                });
                got.push((format!("{}-x{scale}", i.as_str()), hash));
            }
        }
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/pins/screenshot_picker.txt"
        );
        reprisekit::pins::check_file(std::path::Path::new(file), &got);
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
        let form = build_form(
            Purpose::Share,
            &rows(),
            Some(1),
            true,
            None,
            "Chromium",
            |_| None,
        );
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

    /// The share dialog with Remember and the screenshot dialog with its Delay box, each with
    /// its accept button's access key.
    fn both_forms() -> [(form::Form, char); 2] {
        let share = build_form(
            Purpose::Share,
            &rows(),
            Some(1),
            true,
            None,
            "Chromium",
            |_| None,
        );
        let shot = build_form(
            Purpose::Screenshot,
            &rows(),
            Some(1),
            false,
            Some(3),
            "Chromium",
            |_| None,
        );
        [(share, 's'), (shot, 't')]
    }

    fn delay_of(f: &form::Form) -> Option<i32> {
        match f.item("delay").map(|i| &i.control) {
            Some(Control::UpDown(u)) => Some(u.value()),
            _ => None,
        }
    }

    #[test]
    fn a_bare_letter_shares_nothing() {
        use reprisekit::controls::form::{Event, Key};
        for (mut f, accept) in both_forms() {
            let delay = delay_of(&f);
            for c in ['s', 't', 'r', 'd'] {
                assert_eq!(f.key(Key::Char(c)), Event::None, "{c} with {accept}");
            }
            assert_eq!(f.checked("remember"), (accept == 's').then_some(false));
            assert_eq!(delay_of(&f), delay, "the Delay box keeps its value");
            assert_eq!(f.key(Key::Access(accept)), Event::Pressed("share"));
        }
    }

    #[test]
    fn tab_then_enter_or_space_shares_nothing() {
        use reprisekit::controls::form::{Event, Key};
        for (mut f, accept) in both_forms() {
            for _ in 0..4 {
                f.key(Key::Tab { back: false });
                let mut g = f.clone();
                assert_eq!(f.key(Key::Enter), Event::Pressed("cancel"), "{accept}");
                assert_ne!(g.key(Key::Space), Event::Pressed("share"), "{accept}");
            }
        }
    }

    #[test]
    fn remember_is_offered_only_when_asked_and_starts_off() {
        assert_eq!(
            build_form(Purpose::Share, &rows(), None, false, None, "", |_| None)
                .checked("remember"),
            None
        );
        assert_eq!(
            build_form(Purpose::Share, &rows(), None, true, None, "", |_| None).checked("remember"),
            Some(false)
        );
    }

    #[test]
    fn only_a_ticked_remember_comes_back_as_remember() {
        let c = |index, remember, delay| Choice {
            index,
            remember,
            delay,
        };
        assert_eq!(parse_choice("2\n"), Some(c(2, false, 0)));
        assert_eq!(parse_choice("2 remember\n"), Some(c(2, true, 0)));
        assert_eq!(parse_choice(""), None);
    }

    #[test]
    fn the_delay_set_comes_back_cut_to_10_seconds() {
        let c = |index, delay| Choice {
            index,
            remember: false,
            delay,
        };
        for choice in [c(1, 0), c(1, 3), c(0, 10)] {
            assert_eq!(parse_choice(&choice.line()), Some(choice));
        }
        assert_eq!(parse_choice("1 delay 99\n"), Some(c(1, 10)));
        assert_eq!(parse_choice("1 delay x\n"), None);
    }

    #[test]
    fn a_screenshot_offers_the_delay_and_take_hands_it_back() {
        let none = build_form(Purpose::Share, &rows(), Some(1), true, None, "", |_| None);
        assert!(none.item("delay").is_none(), "sharing waits for nothing");
        let chosen = Arc::new(Mutex::new(None));
        let form = build_form(
            Purpose::Screenshot,
            &rows(),
            Some(1),
            false,
            Some(3),
            "",
            |_| None,
        );
        let mut p = prompt(
            Purpose::Screenshot,
            rows(),
            form,
            None,
            None,
            chosen.clone(),
        );
        if let Some(Control::UpDown(u)) = p.form.item_mut("delay").map(|i| &mut i.control) {
            assert_eq!(u.value(), 3, "starts at [screenshot] delay");
            u.step(2);
        }
        let ev = p.form.key(form::Key::Access('t'));
        p.decide(ev);
        assert_eq!(
            *chosen.lock().unwrap(),
            Some(Choice {
                index: 1,
                remember: false,
                delay: 5
            })
        );
    }

    #[test]
    fn the_prompt_names_the_app() {
        let f = build_form(
            Purpose::Share,
            &rows(),
            None,
            false,
            None,
            "Chromium",
            |_| None,
        );
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
        let form = build_form(
            Purpose::Share,
            &rows(),
            Some(1),
            false,
            None,
            "Chromium",
            |_| None,
        );
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
