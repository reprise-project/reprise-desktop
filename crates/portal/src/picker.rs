//! The share dialog: a tree of screens and windows, a Remember check box or a
//! screenshot's Delay box, Share and Cancel (the default, so Enter never shares by
//! accident). Runs as our own child (`reprise-portal --pick`): `pick` feeds it an
//! `Input` on stdin, reads the chosen index on stdout, and kills it when the frontend
//! cancels.

use crate::dialog::{self, Prompt};
use crate::wayland::Target;
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::formscroll::FormScroll;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::updown::{self, UpDown};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::tr;
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
/// The Sources tree's least size on a small work area: two rows tall inside its edges.
const TREE_MIN: (i32, i32) = (240, 40);
/// The Sources group's least room under the tree, as at its sides.
const GROUP_FOOT: i32 = 15;
/// A room shorter than this at 100% raises the Sources group to the consent line and puts the
/// note under the group: over it, a two-line consent line and a three-line note leave no room
/// for the tree's first row.
const NOTE_UNDER: i32 = 167;
/// The raised group's gap under the consent line, and its least gap under the header icon.
const RAISED_GAP: i32 = 7;
const ICON_GAP: i32 = 4;
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
    fn title(self) -> String {
        match self {
            Purpose::Share => tr!("picker-share-title"),
            Purpose::Screenshot => tr!("picker-screenshot-title"),
        }
    }

    fn prompt(self, who: &str) -> String {
        match self {
            Purpose::Share => tr!("picker-share-prompt", who = who),
            Purpose::Screenshot => tr!("picker-screenshot-prompt", who = who),
        }
    }

    fn note(self) -> String {
        match self {
            Purpose::Share => tr!("picker-share-note"),
            Purpose::Screenshot => tr!("picker-screenshot-note"),
        }
    }

    fn accept(self) -> String {
        match self {
            Purpose::Share => tr!("picker-share"),
            Purpose::Screenshot => tr!("picker-take"),
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

/// The tree's Screens branch label, which also tells the branches apart.
fn screens_label() -> String {
    tr!("picker-screens")
}

fn build_tree(
    rows: &[Row],
    mut icon_for: impl FnMut(&str) -> Option<Canvas>,
) -> (TreeView, Vec<usize>) {
    let mut roots = Vec::new();
    let mut at = vec![0usize; rows.len()];
    let folder = icon_for("folder");
    for (screen, title) in [(true, screens_label()), (false, tr!("picker-windows"))] {
        let mut branch = Node::new(&title, true);
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
            .position(|b| (b.label == screens_label()) == r.screen)
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
    let mut items = Vec::from(dialog::wrapping_header(
        w,
        purpose.prompt(&crate::permit::who(app)),
        purpose.note(),
    ));
    items.extend([
        Item::new("group", GROUP, Control::Group(tr!("picker-sources"))),
        Item::new("tree", TREE, Control::Tree(tree)),
    ]);
    if offer_remember {
        items.push(Item::new(
            "remember",
            REMEMBER,
            Control::Check {
                label: tr!("remember-choice"),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ));
    }
    if let Some(s) = delay {
        let max = repriseconf::config::SHOT_DELAY_MAX.into();
        items.extend([
            Item::label("delay_label", DELAY_LABEL, &tr!("picker-delay")),
            Item::new(
                "delay",
                DELAY,
                Control::UpDown(UpDown::new(s.into(), 0, max)),
            )
            .named(tr!("picker-delay-name")),
            Item::label("delay_units", DELAY_UNITS, &tr!("picker-delay-units")),
        ]);
    }
    items.extend(dialog::buttons(
        GROUP.right() - 11,
        BUTTON_Y,
        ("share", &purpose.accept()),
        ("cancel", &reprisekit::kit_tr!("cancel")),
    ));
    let mut f = Form::new(items);
    f.focus_id("tree");
    f.set_consent();
    f
}

/// Lays the dialog out for `room` at 100% text: the Sources group gives up its room under the
/// tree, then the tree down to [`TREE_MIN`]; the rows and the buttons follow, and the header's
/// lines narrow to the room even past the tree. A room under [`NOTE_UNDER`] tall raises the group
/// to the consent line and puts the note under it. Returns its size.
fn lay(form: &mut Form, room: (i32, i32)) -> (i32, i32) {
    let under = room.1 < NOTE_UNDER;
    // The header icon keeps its px, so at 100% text it reaches below a one-line prompt.
    let icon = dialog::ICON_AT.1 + form.units().design(dialog::ICON_PX) + ICON_GAP;
    let raised = (dialog::PROMPT_Y + LINE_H + RAISED_GAP).max(icon);
    let lift = if under { GROUP.y - raised } else { 0 };
    // The note takes a line and Remember's gap under the group; the rows under it rise by the rest.
    let gap = REMEMBER.y - GROUP.bottom();
    let rise = if under { lift - LINE_H - gap } else { 0 };
    let foot = GROUP.bottom() - TREE.bottom() - GROUP_FOOT;
    let narrow = (CLIENT.0 - room.0).clamp(0, TREE.width - TREE_MIN.0);
    let short = (CLIENT.1 - rise - room.1).clamp(0, foot + TREE.height - TREE_MIN.1);
    let tree_short = (short - foot).max(0);
    let cut = |r: Rect, short| Rect::new(r.x, r.y - lift, r.width - narrow, r.height - short);
    let up = |r: Rect| Rect::new(r.x, r.y - short - rise, r.width, r.height);
    let buttons = dialog::buttons(
        GROUP.right() - 11 - narrow,
        BUTTON_Y - short - rise,
        ("share", ""),
        ("cancel", ""),
    );
    let right = (CLIENT.0 - narrow).min(room.0) - 11;
    let line = |x, y| Rect::new(x, y, right - x, LINE_H);
    let note = match under {
        false => line(dialog::TEXT_X, dialog::NOTE_Y),
        true => line(GROUP.x, GROUP.bottom() - lift - short + gap),
    };
    let rects = [
        ("prompt", line(dialog::TEXT_X, dialog::PROMPT_Y)),
        ("note", note),
        ("group", cut(GROUP, short)),
        ("tree", cut(TREE, tree_short)),
        ("remember", up(REMEMBER)),
        ("delay_label", up(DELAY_LABEL)),
        ("delay", up(DELAY)),
        ("delay_units", up(DELAY_UNITS)),
    ]
    .into_iter()
    .chain(buttons.map(|b| (b.id, b.rect)));
    for (id, r) in rects {
        if let Some(it) = form.item_mut(id) {
            it.rect = r;
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
    (CLIENT.0 - narrow, CLIENT.1 - short - rise)
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
        title: purpose.title(),
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
        fit: Some(FormScroll::new(&["share", "cancel"], lay).head(&["prompt"])),
        answered: false,
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
    let screen = tree.roots.get(branch)?.label == screens_label();
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

    /// On a 720x450 work area (a 1440x900 screen at scale 2) the buttons stay in the client at
    /// 100% and 200% text; at 200% the form scrolls down to the Remember or Delay row. At 150% and
    /// 200% the dialog opens with the picked source in view.
    #[test]
    fn a_short_work_area_keeps_the_buttons_in_the_window() {
        use reprisekit::frame::{Chrome, FrameGeometry};
        use reprisekit::interface::Interface;
        use reprisekit::text::TextRenderer;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 150, 200] {
                let base = reprisekit::interfaces::theme_for(i);
                let m = &base.with_text_size(pct).metrics;
                let total = (720, 450 - m.taskbar_height);
                let client = FrameGeometry::client_size_for_total(
                    m,
                    total.0,
                    total.1,
                    false,
                    Chrome::DIALOG,
                );
                for (purpose, remember, delay) in [
                    (Purpose::Share, true, None),
                    (Purpose::Screenshot, false, Some(3)),
                ] {
                    let form = build_form(purpose, &rows(), Some(2), remember, delay, "", |_| None);
                    if pct == 150 {
                        let mut p = prompt(purpose, rows(), form, None, None, Arc::default());
                        let mut ctx = crate::dialog::test_ctx(pct);
                        ctx.theme = base.clone();
                        ctx.size = client;
                        repriseapp::App::event(
                            &mut p,
                            repriseapp::MAIN,
                            repriseapp::Event::Resized,
                            &mut ctx,
                        );
                        let view = p.form.view.unwrap().rect;
                        let row = p.form.focus_part().unwrap();
                        assert!(
                            view.intersect(&row) == row,
                            "{i:?} 150% {purpose:?}: the picked source shows"
                        );
                        continue;
                    }
                    let mut p = prompt(purpose, rows(), form, None, None, Arc::default());
                    let mut ctx = crate::dialog::test_ctx(pct);
                    ctx.theme = base.clone();
                    ctx.size = client;
                    let u = ctx.forms().text;
                    let at = |p: &Prompt, id| u.rect(p.form.item(id).unwrap().rect);
                    let event = |p: &mut Prompt, ctx: &mut repriseapp::Ctx, ev| {
                        repriseapp::App::event(p, repriseapp::MAIN, ev, ctx)
                    };
                    event(&mut p, &mut ctx, repriseapp::Event::Resized);
                    let name = format!("{i:?} {pct}% {purpose:?}");
                    let fit = p.fit.as_ref().unwrap();
                    assert_eq!(fit.scrolls(&p.form), pct > 100, "{name}");
                    let view = p.form.view.unwrap().rect;
                    let row = p.form.focus_part().unwrap();
                    assert!(
                        row.height < p.form.item("tree").unwrap().rect.height,
                        "{name}: the tree's selected row"
                    );
                    assert!(
                        view.intersect(&row) == row,
                        "{name}: the picked source {row:?} shows in {view:?}"
                    );
                    let (take, cancel) = (at(&p, "share"), at(&p, "cancel"));
                    assert!(
                        cancel.right() <= client.0 && cancel.bottom() <= client.1,
                        "{name}: {cancel:?} ends inside {client:?}"
                    );
                    let foot = if remember { "remember" } else { "delay" };
                    let wheel = repriseapp::Event::Pointer(repriseapp::PointerInput::Scroll {
                        x: 4,
                        y: 4,
                        rows: 100,
                    });
                    event(&mut p, &mut ctx, wheel);
                    let shown =
                        (p.fit.as_ref().unwrap().bar(&p.form)).map_or(take.y, |b| b.bottom());
                    let foot = at(&p, foot);
                    assert!(
                        at(&p, "group").bottom() <= foot.y && foot.bottom() <= shown,
                        "{name}: {foot:?} shows by {shown}"
                    );
                    assert!(shown <= take.y, "{name}: the buttons stay clear");
                    assert_eq!(selection(&rows(), &p.form), Some(2), "the pick stays");
                    for scale in [1, 2] {
                        let t = ctx.forms().scaled(scale);
                        let text = TextRenderer::load(&t.fonts);
                        let (w, h) = (client.0 * scale, client.1 * scale);
                        let mut c = Canvas::new(w as u32, h as u32);
                        c.fill_rect(c.bounds(), t.colors.button_face);
                        form::draw(&mut c, &text, &t.paint(), &p.form, true, t.fonts.size_px);
                        p.fit.as_ref().unwrap().draw(&mut c, &t.paint(), &p.form);
                        let face = t.colors.button_face.argb();
                        for b in [take, cancel] {
                            let foot = (b.bottom() - 3) * scale..b.bottom() * scale;
                            let xs = b.x * scale..b.right() * scale;
                            assert!(
                                foot.flat_map(|y| xs.clone().map(move |x| (x, y)))
                                    .any(|(x, y)| c.get(x, y) != face),
                                "{name} x{scale}: {b:?}'s foot is drawn"
                            );
                        }
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
        let hash = reprisekit::pins::fnv;
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 200] {
                let mut ctx = crate::dialog::test_ctx(pct);
                ctx.theme = reprisekit::theme::Theme {
                    access_keys: true,
                    ..reprisekit::interfaces::theme_for(i)
                };
                ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
                ctx.size = match pct {
                    100 => CLIENT,
                    _ => (704, 346),
                };
                let built = build_form(
                    Purpose::Screenshot,
                    &rows(),
                    Some(1),
                    false,
                    Some(3),
                    "Chromium",
                    |_| None,
                );
                let mut p = prompt(
                    Purpose::Screenshot,
                    rows(),
                    built.clone(),
                    None,
                    None,
                    Arc::default(),
                );
                repriseapp::App::event(
                    &mut p,
                    repriseapp::MAIN,
                    repriseapp::Event::Resized,
                    &mut ctx,
                );
                if pct == 100 {
                    assert_eq!(
                        p.form.items, built.items,
                        "{i:?}: a client the size of the dialog keeps it as built"
                    );
                } else {
                    repriseapp::App::event(&mut p, repriseapp::MAIN, wheel(3), &mut ctx);
                    assert!(p.fit.as_ref().unwrap().offset().1 > 0, "{i:?}: scrolled");
                }
                let fit = p.fit.as_ref().unwrap();
                for scale in [1, 2] {
                    let t = ctx.forms().scaled(scale);
                    let text = TextRenderer::load(&t.fonts);
                    let (w, h) = (ctx.size.0 * scale, ctx.size.1 * scale);
                    let mut c = Canvas::new(w as u32, h as u32);
                    c.fill_rect(c.bounds(), t.colors.button_face);
                    form::draw(&mut c, &text, &t.paint(), &p.form, true, t.fonts.size_px);
                    fit.draw(&mut c, &t.paint(), &p.form);
                    let key = match pct {
                        100 => format!("{}-x{scale}", i.as_str()),
                        _ => format!("{}-{pct}-scrolled-x{scale}", i.as_str()),
                    };
                    got.push((key, hash(&c)));
                }
            }
        }
        let file = reprisekit::crate_dir!().join("tests/pins/screenshot_picker.txt");
        reprisekit::pins::check_file(&file, &got);
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
        let Some(Control::Lines(lines)) = f.item("prompt").map(|i| &i.control) else {
            panic!("prompt lines");
        };
        let text = lines.join(" ");
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

    /// The clients a short work area leaves the dialog, from 560x240 to 760x420.
    fn short_clients() -> impl Iterator<Item = (i32, i32)> {
        (560..=760)
            .step_by(50)
            .flat_map(|w| (240..=420).step_by(45).map(move |h| (w, h)))
    }

    /// The pick dialog for `purpose` fitted to `client` in look `i` at `ctx`'s text size, asked
    /// for by `app`, the source at index 2 picked and its consent gate open.
    fn fitted(
        ctx: &mut repriseapp::Ctx,
        i: reprisekit::interface::Interface,
        client: (i32, i32),
        purpose: Purpose,
        app: &str,
        chosen: Arc<Mutex<Option<Choice>>>,
    ) -> Prompt {
        fitted_with(ctx, i, client, (purpose, app), (&rows(), 2), chosen)
    }

    /// [`fitted`] listing `rows`, source `pick` picked.
    fn fitted_with(
        ctx: &mut repriseapp::Ctx,
        i: reprisekit::interface::Interface,
        client: (i32, i32),
        (purpose, app): (Purpose, &str),
        (rows, pick): (&[Row], usize),
        chosen: Arc<Mutex<Option<Choice>>>,
    ) -> Prompt {
        let (remember, delay) = match purpose {
            Purpose::Share => (true, None),
            Purpose::Screenshot => (false, Some(3)),
        };
        let form = build_form(purpose, rows, Some(pick), remember, delay, app, |_| None);
        let mut p = prompt(purpose, rows.to_vec(), form, None, None, chosen);
        ctx.theme = reprisekit::interfaces::theme_for(i);
        ctx.size = client;
        repriseapp::App::event(&mut p, repriseapp::MAIN, repriseapp::Event::Resized, ctx);
        let armed = std::time::Instant::now().checked_sub(form::CONSENT_ARM);
        p.form.consent_focused(armed.unwrap());
        p
    }

    /// The dialog's tree as a reader gets it, and what was published.
    fn published(
        p: &mut Prompt,
        ctx: &repriseapp::Ctx,
    ) -> (reprisekit::access::Tree, reprisekit::access::Published) {
        let mut tree = reprisekit::access::Tree::window("T", ctx.size);
        repriseapp::Extended::access(p, repriseapp::MAIN, &mut tree, ctx);
        let mut published = reprisekit::access::Published::default();
        published.update(tree.clone());
        (tree, published)
    }

    /// The item whose `access` part published node `id`: the nearest node up from it that is
    /// keyed by an item.
    fn owner(
        tree: &reprisekit::access::Tree,
        f: &Form,
        id: reprisekit::access::accesskit::NodeId,
    ) -> Option<usize> {
        use reprisekit::access::Tree;
        let parents: std::collections::HashMap<_, _> = (tree.nodes())
            .flat_map(|(p, n)| n.children().iter().map(move |&c| (c, p)))
            .collect();
        let mut at = id;
        loop {
            let up = *parents.get(&at)?;
            let keyed = |(k, it): (usize, &Item)| Tree::id_of(up, (it.id, k)) == at;
            if let Some(k) = f.items.iter().enumerate().position(keyed) {
                return Some(k);
            }
            at = up;
        }
    }

    /// Every node a reader can click or focus in `p` is acted on where its own control is: on
    /// the part of it in view, and on Share or Cancel only for that button.
    /// Returns how many requests acted and how many scroll their node in first.
    fn assert_acts_on_its_own_control(
        p: &mut Prompt,
        ctx: &repriseapp::Ctx,
        name: &str,
    ) -> (usize, usize) {
        use reprisekit::access::{Act, Request, Route};
        let (tree, published) = published(p, ctx);
        let px = p.form.units().px(ctx.base_px());
        let u = p.form.units();
        let fit = p.fit.as_ref().unwrap();
        let clip = fit.clip(&p.form);
        let f = &p.form;
        let button =
            |k: Option<usize>| k.is_some_and(|k| ["share", "cancel"].contains(&f.items[k].id));
        let (mut acted, mut revealed) = (0, 0);
        for (id, n) in tree.nodes() {
            for act in [Act::Click, Act::Focus] {
                let t = match published.route(Request { node: id, act }) {
                    Route::Act(t) => t,
                    Route::Reveal { .. } => {
                        revealed += 1;
                        continue;
                    }
                    Route::None => continue,
                };
                acted += 1;
                let (x, y) = (t.at.0 as i32, t.at.1 as i32);
                let hit = f.hit(x, y, &ctx.text, px).map(|h| h.item);
                let owner = owner(&tree, f, id);
                let what = format!(
                    "{name}: {act:?} on {:?} {:?} at {:?}",
                    n.role(),
                    n.label(),
                    (x, y)
                );
                assert!(
                    !button(hit) || hit == owner,
                    "{what} presses {:?}",
                    hit.map(|k| f.items[k].id)
                );
                let Some(o) = owner else {
                    assert_eq!(
                        n.role(),
                        reprisekit::access::accesskit::Role::ScrollBar,
                        "{what}"
                    );
                    assert_eq!(hit, None, "{what}: a bar is no control of the form's");
                    continue;
                };
                let it = &f.items[o];
                let held = u.rect(f.flowed(it));
                let shown = if button(Some(o)) { held } else { clip };
                assert!(shown.contains(x, y), "{what}: outside {shown:?}");
                match hit {
                    _ if f.can_focus(it.id) => {
                        assert_eq!(hit, Some(o), "{what}: lands on {}", it.id);
                    }
                    Some(k) if k != o => {
                        let r = u.rect(f.flowed(&f.items[k]));
                        let into = held.intersect(&r) == r;
                        assert!(into, "{what}: lands on {} out of {}", f.items[k].id, it.id);
                    }
                    _ => {}
                }
            }
        }
        (acted, revealed)
    }

    #[test]
    fn a_readers_click_lands_on_the_control_it_names_at_every_short_size() {
        use reprisekit::interface::Interface;
        let mut hidden = 0;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 150, 200] {
                let mut ctx = crate::dialog::test_ctx(pct);
                for client in short_clients() {
                    for purpose in [Purpose::Share, Purpose::Screenshot] {
                        let mut p = fitted(&mut ctx, i, client, purpose, "", Arc::default());
                        let name = format!("{i:?} {pct}% {client:?} {purpose:?}");
                        let (acted, revealed) = assert_acts_on_its_own_control(&mut p, &ctx, &name);
                        assert!(acted > 10, "{name}: {acted} acted");
                        if p.fit.as_ref().unwrap().scrolls(&p.form) {
                            hidden += revealed;
                            repriseapp::App::event(&mut p, repriseapp::MAIN, wheel(100), &mut ctx);
                            let name = format!("{name} scrolled");
                            assert_acts_on_its_own_control(&mut p, &ctx, &name);
                        }
                    }
                }
            }
        }
        assert!(hidden > 100, "{hidden} nodes scrolled out");
    }

    #[test]
    fn a_share_after_a_cancel_in_one_dispatch_shares_nothing() {
        use reprisekit::interface::Interface;
        let mut ctx = crate::dialog::test_ctx(100);
        let chosen = Arc::new(Mutex::new(None));
        let mut open = || {
            let i = Interface::Classic;
            fitted(&mut ctx, i, CLIENT, Purpose::Share, "", chosen.clone())
        };
        let share = |p: &mut Prompt| p.decide(form::Event::Pressed("share"));
        let mut p = open();
        assert!(
            p.decide(form::Event::Cancel).quit,
            "Cancel closes the picker"
        );
        assert!(!share(&mut p).quit, "a Share after it answers nothing");
        assert_eq!(*chosen.lock().unwrap(), None, "the Cancel stands");
        assert!(share(&mut open()).quit);
        assert!(chosen.lock().unwrap().is_some(), "a Share alone shares");
    }

    /// The narrow client moves Share to where Cancel was.
    #[test]
    fn a_readers_cancel_between_a_resize_and_its_draw_cancels() {
        use reprisekit::access::{Act, Gesture, Request, Route, accesskit::Role, gesture};
        use reprisekit::interface::Interface;
        let mut ctx = crate::dialog::test_ctx(100);
        let i = Interface::Classic;
        let full = fitted(&mut ctx, i, CLIENT, Purpose::Share, "", Arc::default());
        let centre = |id| full.form.rect(id).map(|r| r.x + r.width / 2).unwrap();
        let dx = centre("cancel") - centre("share");
        let narrow = (CLIENT.0 - dx, CLIENT.1);
        let chosen = Arc::new(Mutex::new(None));
        let mut p = fitted(&mut ctx, i, narrow, Purpose::Share, "", chosen.clone());
        let (tree, before) = published(&mut p, &ctx);
        let cancel = tree.find(Role::DefaultButton, "Cancel").unwrap();
        let click = |published: &reprisekit::access::Published| {
            let req = Request {
                node: cancel,
                act: Act::Click,
            };
            match published.route(req) {
                Route::Act(t) => match gesture(Act::Click, t) {
                    Some(Gesture::Click((x, y))) => (x as i32, y as i32),
                    g => panic!("a click on a button clicks, not {g:?}"),
                },
                r => panic!("a click on a button acts, not {r:?}"),
            }
        };

        ctx.size = CLIENT;
        repriseapp::App::event(
            &mut p,
            repriseapp::MAIN,
            repriseapp::Event::Resized,
            &mut ctx,
        );
        let px = p.form.units().px(ctx.base_px());
        let hit =
            |p: &Prompt, (x, y)| (p.form.hit(x, y, &ctx.text, px)).map(|h| p.form.items[h.item].id);
        assert_eq!(
            hit(&p, click(&before)),
            Some("share"),
            "the tree from before the resize aims Cancel's click at Share"
        );
        let (x, y) = click(&published(&mut p, &ctx).1);
        assert_eq!(hit(&p, (x, y)), Some("cancel"));
        let (button, mods) = (repriseapp::BTN_LEFT, repriseapp::Mods::default());
        let press = repriseapp::PointerInput::Press {
            x,
            y,
            button,
            time: 1,
            mods,
        };
        let release = repriseapp::PointerInput::Release { x, y, button, mods };
        let mut event = |input| {
            let ev = repriseapp::Event::Pointer(input);
            repriseapp::App::event(&mut p, repriseapp::MAIN, ev, &mut ctx)
        };
        event(press);
        assert!(event(release).quit, "the click closes the dialog");
        assert!(chosen.lock().unwrap().is_none(), "and shares nothing");
    }

    fn wheel(rows: i32) -> repriseapp::Event {
        repriseapp::Event::Pointer(repriseapp::PointerInput::Scroll { x: 4, y: 4, rows })
    }

    /// A reader's Click or Focus on any node of a scrolled dialog, carried out as the host does:
    /// a hidden node scrolls in first, then the click lands, or the keyboard moves to the
    /// control and it shows. Only Share's click shares.
    #[test]
    fn a_readers_click_on_a_hidden_source_scrolls_it_in_and_shares_nothing() {
        use reprisekit::access::{Act, Gesture, Request, Route, gesture};
        use reprisekit::interface::Interface;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [150, 200] {
                let mut ctx = crate::dialog::test_ctx(pct);
                for client in [(704, 346), (560, 240)] {
                    let share = Purpose::Share;
                    let mut probe = fitted(&mut ctx, i, client, share, "", Arc::default());
                    let (tree, _) = published(&mut probe, &ctx);
                    let nodes: Vec<_> = tree.nodes().map(|(id, n)| (id, n.clone())).collect();
                    for (id, n) in nodes {
                        for act in [Act::Click, Act::Focus] {
                            let chosen = Arc::new(Mutex::new(None));
                            let mut p = fitted(&mut ctx, i, client, share, "", chosen.clone());
                            let name = format!(
                                "{i:?} {pct}% {client:?} {act:?} on {:?} {:?}",
                                n.role(),
                                n.label()
                            );
                            let req = Request { node: id, act };
                            let mut route = published(&mut p, &ctx).1.route(req);
                            for _ in 0..4 {
                                let Route::Reveal { reveal, .. } = route else {
                                    break;
                                };
                                let b = reveal.node;
                                let node = Rect::new(
                                    b.x0 as i32,
                                    b.y0 as i32,
                                    (b.x1 - b.x0).ceil() as i32,
                                    (b.y1 - b.y0).ceil() as i32,
                                );
                                let at = (reveal.at.0 as i32, reveal.at.1 as i32);
                                let r = repriseapp::Reader::Reveal {
                                    at,
                                    row: reveal.row,
                                    node,
                                    view: reveal.view,
                                };
                                repriseapp::Extended::reader(&mut p, repriseapp::MAIN, r, &ctx);
                                route = published(&mut p, &ctx).1.route(req);
                            }
                            let Route::Act(t) = route else {
                                assert!(
                                    !matches!(route, Route::Reveal { .. }),
                                    "{name}: never comes in"
                                );
                                continue;
                            };
                            let owner = owner(&published(&mut p, &ctx).0, &p.form, id);
                            let is_share = owner.is_some_and(|k| p.form.items[k].id == "share");
                            let (x, y) = match gesture(act, t) {
                                Some(Gesture::Click((x, y))) => (x as i32, y as i32),
                                Some(Gesture::Focus((x, y))) => {
                                    let at = (x as i32, y as i32);
                                    let focus = repriseapp::Reader::Focus(at);
                                    repriseapp::Extended::reader(
                                        &mut p,
                                        repriseapp::MAIN,
                                        focus,
                                        &ctx,
                                    );
                                    assert!(chosen.lock().unwrap().is_none(), "{name}");
                                    let item = |o: usize| p.form.items[o].id;
                                    let Some(o) = owner.filter(|&o| p.form.can_focus(item(o)))
                                    else {
                                        continue;
                                    };
                                    assert_eq!(p.form.focus(), Some(o), "{name}: focused");
                                    let view = p.form.view.unwrap().rect;
                                    let part = p.form.focus_part().unwrap();
                                    let kept = ["share", "cancel"].contains(&item(o));
                                    let shown = kept || view.intersect(&part) == part;
                                    assert!(shown, "{name}: {part:?} shows in {view:?}");
                                    continue;
                                }
                                _ => continue,
                            };
                            let mods = repriseapp::Mods::default();
                            let button = repriseapp::BTN_LEFT;
                            for input in [
                                repriseapp::PointerInput::Press {
                                    x,
                                    y,
                                    button,
                                    time: 1,
                                    mods,
                                },
                                repriseapp::PointerInput::Release { x, y, button, mods },
                            ] {
                                let ev = repriseapp::Event::Pointer(input);
                                repriseapp::App::event(&mut p, repriseapp::MAIN, ev, &mut ctx);
                            }
                            let shared = chosen.lock().unwrap().is_some();
                            assert_eq!(shared, is_share, "{name}: shares only on Share");
                        }
                    }
                }
            }
        }
    }

    /// Bare letters, and Tab then Enter or Space, share nothing in a dialog scrolled at 200%.
    #[test]
    fn no_key_shares_in_a_scrolled_dialog() {
        use reprisekit::interface::Interface;
        const SPACE: u32 = 0x20;
        let key = |sym: u32, utf8: Option<&str>| {
            repriseapp::Event::Key(KeyInput {
                sym,
                utf8: utf8.map(Into::into),
                mods: repriseapp::Mods::default(),
                edit: reprisekit::textedit::key_for(sym, false, false, utf8),
            })
        };
        let mut ctx = crate::dialog::test_ctx(200);
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for purpose in [Purpose::Share, Purpose::Screenshot] {
                let scrolled = |ctx: &mut repriseapp::Ctx, chosen| {
                    let mut p = fitted(ctx, i, (704, 346), purpose, "Chromium", chosen);
                    repriseapp::App::event(&mut p, repriseapp::MAIN, wheel(3), ctx);
                    let fit = p.fit.as_ref().unwrap();
                    assert!(fit.offset().1 > 0 && fit.bar(&p.form).is_some(), "{i:?}");
                    p
                };
                let chosen = Arc::new(Mutex::new(None));
                let mut p = scrolled(&mut ctx, chosen.clone());
                for c in ['s', 't', 'r', 'd'] {
                    let sym = c as u32;
                    let reply = repriseapp::App::event(
                        &mut p,
                        repriseapp::MAIN,
                        key(sym, Some(&c.to_string())),
                        &mut ctx,
                    );
                    assert!(!reply.quit, "{i:?} {purpose:?}: {c}");
                }
                assert_eq!(*chosen.lock().unwrap(), None);
                for tabs in 1..=4 {
                    for last in [keysym::RETURN, SPACE] {
                        let chosen = Arc::new(Mutex::new(None));
                        let mut p = scrolled(&mut ctx, chosen.clone());
                        for _ in 0..tabs {
                            repriseapp::App::event(
                                &mut p,
                                repriseapp::MAIN,
                                key(keysym::TAB, None),
                                &mut ctx,
                            );
                        }
                        let utf8 = (last == SPACE).then_some(" ");
                        repriseapp::App::event(&mut p, repriseapp::MAIN, key(last, utf8), &mut ctx);
                        assert_eq!(
                            *chosen.lock().unwrap(),
                            None,
                            "{i:?} {purpose:?} {tabs} tabs, {last:#x}"
                        );
                    }
                }
            }
        }
    }

    /// A long app id at 200% wraps the consent line inside the view, every word of it shown.
    #[test]
    fn a_long_app_name_wraps_the_prompt_inside_the_view() {
        use reprisekit::interface::Interface;
        let app = "org.chromium.Chromium";
        let mut ctx = crate::dialog::test_ctx(200);
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for client in [(704, 346), (1000, 1000)] {
                let p = fitted(&mut ctx, i, client, Purpose::Share, app, Arc::default());
                let f = &p.form;
                let u = f.units();
                let px = u.px(ctx.base_px());
                let prompt = f.flowed(f.item("prompt").unwrap());
                let words = Purpose::Share.prompt(app);
                let width = u.rect(prompt).width;
                let lines = reprisekit::text::wrap(ctx.text.dialog(f.font), &words, width, px);
                let name = format!("{i:?} {client:?}");
                let short = client.1 < 1000;
                assert!(lines.len() > 1 || !short, "{name}: {lines:?}");
                assert_eq!(
                    lines.concat().replace(' ', ""),
                    words.replace(' ', ""),
                    "{name}: every word shows"
                );
                assert_eq!(
                    prompt.height,
                    lines.len() as i32 * LINE_H,
                    "{name}: the line grows to hold them"
                );
                let view = f.view.unwrap().rect;
                assert!(
                    prompt.right() <= view.right(),
                    "{name}: {prompt:?} ends in {view:?}"
                );
                let note = f.flowed(f.item("note").unwrap());
                assert!(note.y >= prompt.bottom(), "{name}: the note moves down");
            }
        }
    }

    /// The tree's picked path and its row in px while the row shows in the tree.
    fn picked_row(p: &Prompt) -> Option<(Vec<usize>, Rect)> {
        let f = &p.form;
        let tree = f.item("tree")?;
        let Control::Tree(tv) = &tree.control else {
            return None;
        };
        let row = f.focus_part_px()?;
        let shown = f.focus() == f.items.iter().position(|it| it.id == "tree");
        let picked = tv.selected.clone()?;
        (shown && row != f.units().rect(f.flowed(tree))).then_some((picked, row))
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.width / 2, r.y + r.height / 2)
    }

    /// A left click at `r`'s centre.
    fn click(p: &mut Prompt, ctx: &mut repriseapp::Ctx, r: Rect) {
        let ((x, y), mods, button) = (centre(r), repriseapp::Mods::default(), repriseapp::BTN_LEFT);
        for input in [
            repriseapp::PointerInput::Press {
                x,
                y,
                button,
                time: 1,
                mods,
            },
            repriseapp::PointerInput::Release { x, y, button, mods },
        ] {
            repriseapp::App::event(p, repriseapp::MAIN, repriseapp::Event::Pointer(input), ctx);
        }
    }

    /// At open the consent line shows whole in every look and text size on short clients, and
    /// the picked source under it, from the first row to the last of a long list. Only on a
    /// client under 230 px tall, and taller by as much as the scrollbars grew with the text, may
    /// the row or the line go short, the line at the view's top and the row the tree's first. A click on a picked row that shows (as a reader's Click lands),
    /// then a reader's Focus on it, leave it picked where it is.
    #[test]
    fn the_dialog_opens_with_the_consent_line_and_then_the_picked_source_in_view() {
        use reprisekit::interface::Interface;
        let mut long = rows();
        long.extend((0..12).map(|k| Row {
            screen: false,
            icon: String::new(),
            label: format!("Window {k}"),
        }));
        let clients = [240, 280, 300, 330, 346, 420]
            .into_iter()
            .flat_map(|h| [560, 660, 704, 760].map(|w| (w, h)))
            .chain(short_clients())
            .chain([
                (560, 230),
                (480, 230),
                (704, 220),
                (560, 200),
                (480, 180),
                (400, 160),
                (704, 200),
                (1000, 200),
            ])
            .collect::<Vec<_>>();
        let mut wrong = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let fonts = reprisekit::interfaces::theme_for(i).fonts;
            for pct in [100, 125, 150, 175, 200] {
                let mut ctx = crate::dialog::test_ctx(pct);
                ctx.text = reprisekit::text::TextRenderer::load(&fonts).into();
                for &client in &clients {
                    for purpose in [Purpose::Share, Purpose::Screenshot] {
                        for rs in [&rows()[..], &long[..]] {
                            for pick in [0, 2, rs.len() - 1] {
                                for app in ["Chromium", "org.chromium.Chromium"] {
                                    let mut p = fitted_with(
                                        &mut ctx,
                                        i,
                                        client,
                                        (purpose, app),
                                        (rs, pick),
                                        Arc::default(),
                                    );
                                    let f = &p.form;
                                    let u = f.units();
                                    let view = p.fit.as_ref().unwrap().clip(f);
                                    let line = u.rect(f.flowed(f.item("prompt").unwrap()));
                                    let tree = f.item("tree").unwrap();
                                    let Control::Tree(tv) = &tree.control else {
                                        unreachable!()
                                    };
                                    let first = tv.selected_row(&tv.visible()) == Some(tv.top);
                                    let shown = picked_row(&p);
                                    let row = shown.as_ref().map(|s| s.1);
                                    let seen = |r: Rect| view.intersect(&r) == r;
                                    let name = format!(
                                        "{i:?} {pct}% {client:?} {purpose:?} {} rows, pick \
                                         {pick}, {app}: line {line:?} row {row:?} in {view:?}",
                                        rs.len()
                                    );
                                    let bars = reprisekit::interfaces::theme_for(i)
                                        .look
                                        .controls_at_100()
                                        .scroll
                                        * (u.small() - 1);
                                    let top = client.1 < 230 + bars && line.y == view.y;
                                    let at_most = top && first && shown.is_some();
                                    let tall = top && line.height > view.height;
                                    let whole = row.is_some_and(seen);
                                    if !(seen(line) || tall) || !(whole || at_most) {
                                        wrong.push(name);
                                        continue;
                                    }
                                    let Some((picked, row)) = shown.filter(|_| whole) else {
                                        continue;
                                    };
                                    click(&mut p, &mut ctx, row);
                                    if picked_row(&p) != Some((picked.clone(), row)) {
                                        wrong.push(format!("{name}: clicked"));
                                        continue;
                                    }
                                    let focus = repriseapp::Reader::Focus(centre(row));
                                    repriseapp::Extended::reader(
                                        &mut p,
                                        repriseapp::MAIN,
                                        focus,
                                        &ctx,
                                    );
                                    if picked_row(&p) != Some((picked, row)) {
                                        wrong.push(format!("{name}: focused"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} opens:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    /// A reader's Focus on the consent line or the Sources group, which take no focus, leaves the
    /// dialog where it is while the picked source shows whole, wherever it is scrolled to.
    #[test]
    fn a_readers_focus_that_moves_nothing_scrolls_nothing() {
        use reprisekit::interface::Interface;
        let mut checked = 0;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [125, 150, 175, 200] {
                let mut ctx = crate::dialog::test_ctx(pct);
                for client in [(560, 240), (704, 300), (704, 346)] {
                    for by in -40..=40 {
                        for id in ["prompt", "group"] {
                            let mut p =
                                fitted(&mut ctx, i, client, Purpose::Share, "", Arc::default());
                            let (f, fit) = (&mut p.form, p.fit.as_mut().unwrap());
                            let row = f.focus_part_px().unwrap();
                            fit.show(f, Rect::new(row.x, row.y + by, row.width, row.height));
                            let row = f.focus_part_px().unwrap();
                            if fit.clip(f).intersect(&row) != row {
                                continue;
                            }
                            let r = f.units().rect(f.flowed(f.item(id).unwrap()));
                            let at = (r.x + 2, r.y + r.height / 2);
                            let before = fit.offset();
                            let focus = repriseapp::Reader::Focus(at);
                            repriseapp::Extended::reader(&mut p, repriseapp::MAIN, focus, &ctx);
                            let name = format!("{i:?} {pct}% {client:?} by {by} on {id}");
                            assert_eq!(p.fit.as_ref().unwrap().offset(), before, "{name}");
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "{checked} scrolls with the row whole");
    }
}
