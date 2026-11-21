use super::*;
use crate::access::{self, Act, Published, Request, Route, accesskit::Role};
use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{Control, Item};
use crate::interface::Interface;
use crate::pins::fnv;
use crate::theme::Theme;

const SIZE: (i32, i32) = (200, 241);
const KEEP: &[&str] = &["ok", "cancel"];
const OK: Rect = Rect::new(36, 209, BUTTON_W, BUTTON_H);
const CANCEL: Rect = Rect::new(116, 209, BUTTON_W, BUTTON_H);
const CHECK: Rect = Rect::new(9, 180, 150, 13);

fn as_built(_: &mut Form, _: (i32, i32)) -> (i32, i32) {
    SIZE
}

/// The note and the group narrow to the room they get, down to 150, and the buttons move left
/// with them.
fn narrowing(form: &mut Form, room: (i32, i32)) -> (i32, i32) {
    let w = room.0.clamp(150, SIZE.0);
    let by = SIZE.0 - w;
    for (id, r) in [
        ("note", Rect::new(9, 9, w - 18, 13)),
        ("group", Rect::new(9, 30, w - 18, 140)),
        ("ok", Rect { x: OK.x - by, ..OK }),
        (
            "cancel",
            Rect {
                x: CANCEL.x - by,
                ..CANCEL
            },
        ),
    ] {
        if let Some(it) = form.item_mut(id) {
            it.rect = r;
        }
    }
    (w, SIZE.1)
}

fn check(id: &'static str, r: Rect, label: &str) -> Item {
    Item::new(
        id,
        r,
        Control::Check {
            label: label.into(),
            checked: true,
            enabled: true,
            mixed: None,
        },
    )
}

/// Controls that end before the bar in a client the form's width.
fn form() -> Form {
    let button = |id, r, label: &str| {
        Item::new(
            id,
            r,
            Control::Button {
                label: label.into(),
                enabled: true,
                default: id == "cancel",
            },
        )
    };
    Form::new(vec![
        Item::label("note", Rect::new(9, 9, 170, 13), "Pick one of these."),
        Item::new(
            "group",
            Rect::new(9, 30, 170, 140),
            Control::Group("Rows".into()),
        ),
        check("first", Rect::new(20, 48, 150, 13), "&First"),
        check("check", CHECK, "&Keep it"),
        button("ok", OK, "OK"),
        button("cancel", CANCEL, "Cancel"),
    ])
}

fn classic() -> Theme {
    crate::interfaces::theme_for(Interface::Classic)
}

/// `f` fitted to `client` px under `theme` at `pct`% text.
fn fit_form(
    mut f: Form,
    theme: &Theme,
    pct: u32,
    client: (i32, i32),
    lay: Lay,
) -> (Form, FormScroll) {
    let t = theme.with_form_text(pct);
    f.drawn_with(&t.paint());
    let text = TextRenderer::load(&t.fonts);
    let mut fs = FormScroll::new(KEEP, lay);
    fs.fit(&mut f, client, &text, t.fonts.size_px);
    (f, fs)
}

fn fitted(theme: &Theme, pct: u32, client: (i32, i32), lay: Lay) -> (Form, FormScroll) {
    fit_form(form(), theme, pct, client, lay)
}

fn refit(f: &mut Form, fs: &mut FormScroll, client: (i32, i32)) {
    let t = classic();
    fs.fit(f, client, &TextRenderer::load(&t.fonts), t.fonts.size_px);
}

fn rect(f: &Form, id: &str) -> Rect {
    f.item(id).unwrap().rect
}

#[test]
fn a_form_that_fits_is_left_as_built() {
    let (f, fs) = fitted(&classic(), 100, SIZE, as_built);
    assert!(!fs.scrolls(&f));
    assert_eq!(f.view.map(|v| v.rect), Some(Rect::new(0, 0, 200, 241)));
    assert_eq!((fs.bar(&f), fs.hbar(&f)), (None, None));
    assert_eq!(f.items, form().items);
}

#[test]
fn a_short_client_pins_the_buttons_and_scrolls_the_rest_under_the_bar() {
    let (mut f, mut fs) = fitted(&classic(), 100, (200, 141), as_built);
    assert!(fs.scrolls(&f));
    assert_eq!(rect(&f, "ok"), Rect { y: 109, ..OK });
    assert_eq!(rect(&f, "cancel"), Rect { y: 109, ..CANCEL });
    let bar = classic().controls().scroll;
    assert_eq!(
        f.view,
        Some(View {
            rect: Rect::new(0, 0, 200 - bar, 102),
            keep: KEEP,
        })
    );
    assert_eq!(fs.bar(&f), Some(Rect::new(200 - bar, 0, bar, 102)));
    assert_eq!(fs.hbar(&f), None, "the controls end before the bar");

    assert!(fs.wheel(&mut f, 1000));
    assert_eq!(
        fs.offset().1,
        CHECK.bottom() - 102,
        "the range ends at the last control"
    );
    assert_eq!(rect(&f, "check").bottom(), 102);
    assert_eq!(rect(&f, "ok").y, 109, "the buttons stay");
    assert!(!fs.wheel(&mut f, 1), "at the end");

    refit(&mut f, &mut fs, (200, 191));
    assert_eq!(
        fs.offset().1,
        CHECK.bottom() - 152,
        "a taller client holds the top to its range"
    );
    assert_eq!(rect(&f, "check").bottom(), 152);
    refit(&mut f, &mut fs, SIZE);
    assert!(!fs.scrolls(&f));
    assert_eq!(
        f.items,
        form().items,
        "a client the form fits takes it back as built"
    );
}

#[test]
fn a_narrow_client_scrolls_across_too() {
    let (mut f, mut fs) = fitted(&classic(), 100, (150, 241), as_built);
    let hbar = fs.hbar(&f).expect("a bar along the view");
    assert!(
        fs.bar(&f).is_some(),
        "which takes the view's foot from the controls"
    );
    assert_eq!(hbar.bottom(), 202);
    let thumb_min = f.controls().thumb_min;
    let frame = fs.frame(&f);
    let end = fs.scroll.hbar_parts(&frame, hbar, thumb_min).forward;
    for _ in 0..10 {
        assert!(fs.press(&mut f, end.x + 2, end.y + 2));
        fs.release();
    }
    assert_eq!(fs.offset().0, frame.range().0);
    assert_eq!(fs.clip(&f).right(), rect(&f, "group").right());
}

#[test]
fn the_focused_control_scrolls_into_view() {
    let (mut f, mut fs) = fitted(&classic(), 100, (200, 141), as_built);
    f.focus_id("check");
    assert!(fs.reveal_focus(&mut f));
    let view = f.view.unwrap().rect;
    assert_eq!(
        rect(&f, "check").bottom(),
        view.bottom(),
        "as little as it takes"
    );
    f.focus_id("first");
    assert!(fs.reveal_focus(&mut f));
    assert_eq!((fs.offset().1, rect(&f, "first").y), (48, 0));
    f.focus_id("ok");
    assert!(!fs.reveal_focus(&mut f), "a kept button always shows");
}

#[test]
fn a_fit_shows_the_focused_control() {
    let mut f = form();
    f.focus_id("check");
    let (f, fs) = fit_form(f, &classic(), 100, (200, 141), as_built);
    assert_eq!(rect(&f, "check").bottom(), f.view.unwrap().rect.bottom());
    assert!(fs.offset().1 > 0);
}

/// `f` fitted at 100% to `client`, its `head` shown above the focus.
fn fit_under(mut f: Form, head: &'static [&'static str], client: (i32, i32)) -> (Form, FormScroll) {
    let t = classic();
    f.drawn_with(&t.paint());
    let mut fs = FormScroll::new(KEEP, as_built).head(head);
    fs.fit(
        &mut f,
        client,
        &TextRenderer::load(&t.fonts),
        t.fonts.size_px,
    );
    (f, fs)
}

#[test]
fn a_fit_shows_the_focused_control_only_as_far_as_the_head_stays_in_view() {
    let mut f = form();
    f.focus_id("check");
    let (f, fs) = fit_under(f, &["note"], (200, 141));
    let view = f.view.unwrap().rect;
    assert_eq!((fs.offset().1, rect(&f, "note").y), (9, view.y));
    assert!(rect(&f, "check").bottom() > view.bottom(), "the head wins");
}

#[test]
fn a_fit_lifts_a_trees_selected_row_to_its_first_to_show_it_under_the_head() {
    use crate::controls::treeview::{self, Node, Style, TreeView};
    let tree = Rect::new(20, 48, 150, 110);
    for pick in [6, 12] {
        let mut root = Node::new("Rows", true);
        root.expanded = true;
        root.children = (0..12)
            .map(|k| Node::new(&format!("Row {k}"), false))
            .collect();
        let mut tv = TreeView::new(vec![root], Style::Plain);
        let lay = treeview::layout_without_text(&tv, tree, &classic().controls());
        tv.select(pick, &lay);
        assert!(
            tv.top > 0 && tv.top < pick,
            "the tree shows row {pick} low in its rows"
        );
        let mut f = form();
        f.items.retain(|it| it.id != "first");
        f.items.push(Item::new("tree", tree, Control::Tree(tv)));
        f.focus_id("tree");
        let (f, fs) = fit_under(f, &["note"], (200, 141));
        let Some(Control::Tree(tv)) = f.item("tree").map(|it| &it.control) else {
            unreachable!()
        };
        assert_eq!(
            tv.top, pick,
            "row {pick} is the tree's first, the last past its end"
        );
        let view = f.view.unwrap().rect;
        let row = f.focus_part().unwrap();
        assert!(rect(&f, "note").y >= view.y, "the head shows");
        assert_eq!(view.intersect(&row), row, "{row:?} shows in {view:?}");
        assert!(fs.offset().1 > 0);
    }
}

#[test]
fn the_bars_arrow_steps_a_line_its_track_a_view_and_its_thumb_drags() {
    let (mut f, mut fs) = fitted(&classic(), 100, (200, 141), as_built);
    let range = CHECK.bottom() - 102;
    let bar = fs.bar(&f).unwrap();
    let (x, below) = (bar.x + 2, bar.bottom() - 2);
    assert!(!fs.press(&mut f, bar.x - 2, below), "off the bar");
    assert!(fs.press(&mut f, x, below));
    assert_eq!(fs.offset().1, LINE_H);
    assert!(fs.ticking(), "the arrow repeats while held");
    assert!(fs.release());
    assert!(fs.press(&mut f, x, bar.bottom() - bar.width - 4));
    assert_eq!(
        fs.offset().1,
        range,
        "the track pages a view, held to the range"
    );
    fs.release();
    assert!(fs.wheel(&mut f, -1000));
    let thumb_min = f.controls().thumb_min;
    let parts = fs.scroll.bar_parts(&fs.frame(&f), bar, thumb_min);
    let thumb = parts.thumb.unwrap();
    assert!(fs.press(&mut f, x, thumb.y + 2));
    assert!(fs.holding());
    assert!(fs.motion(&mut f, x, bar.bottom() + 100));
    assert_eq!(fs.offset().1, range, "dragged to the end");
    assert!(fs.release());
    assert!(!fs.holding());
}

#[test]
fn the_layout_gives_up_room_beside_the_bar_once_the_form_scrolls() {
    let (f, fs) = fitted(&classic(), 100, (190, 141), narrowing);
    assert!(fs.scrolls(&f));
    assert_eq!(fs.hbar(&f), None);
    let view = f.view.unwrap().rect;
    assert!(rect(&f, "note").right() + 9 <= view.right());
    assert_eq!(
        rect(&f, "cancel").right() + 9,
        190,
        "the buttons keep to the corner"
    );
    let (f, fs) = fitted(&classic(), 100, (190, 241), narrowing);
    assert!(!fs.scrolls(&f));
    assert_eq!(
        rect(&f, "note").width,
        172,
        "no bar, no room given up for it"
    );
}

#[test]
fn a_note_that_rewraps_taller_scrolls_to_its_last_line_and_what_it_pushed_down() {
    crate::pins::fonts();
    let mut f = form();
    let words = "These words run on past one line once the note narrows beside the bar.";
    f.item_mut("note").unwrap().control = Control::Lines(vec![words.into()]);
    let (mut f, mut fs) = fit_form(f, &classic(), 100, (150, 141), narrowing);
    let grown = f.flowed(f.item("note").unwrap());
    assert!(grown.height > 13, "the note wraps: {grown:?}");
    fs.wheel(&mut f, 1000);
    let check = f.flowed(f.item("check").unwrap());
    assert_eq!(check.bottom(), fs.frame(&f).clip().bottom(), "{check:?}");
}

#[test]
fn at_grown_text_the_view_ends_at_the_bar_and_over_the_buttons_in_px() {
    let (f, fs) = fitted(&classic(), 200, (380, 300), as_built);
    let u = f.units();
    let bar = fs.bar(&f).unwrap();
    let view = u.rect(f.view.unwrap().rect);
    assert_eq!((view.bottom(), bar.bottom()), (bar.height, bar.height));
    assert!(
        (bar.x - 1..=bar.x).contains(&view.right()),
        "{view:?} {bar:?}"
    );
    let cancel = u.rect(rect(&f, "cancel"));
    assert!(
        cancel.right() <= 380 && cancel.bottom() <= 300,
        "{cancel:?}"
    );
    assert!(cancel.y > view.bottom());
}

#[test]
fn a_reader_click_on_a_hidden_control_scrolls_it_in_and_lands_on_it() {
    let (mut f, mut fs) = fitted(&classic(), 100, (200, 141), as_built);
    let t = classic();
    let (text, px) = (TextRenderer::load(&t.fonts), t.fonts.size_px);
    let published = |f: &Form, fs: &FormScroll| {
        let mut tree = Tree::window("T", (200, 141));
        fs.access(f, &mut tree, access::ROOT, &text, px);
        let nodes: Vec<_> = tree.nodes().map(|(id, n)| (id, n.clone())).collect();
        let mut p = Published::default();
        p.update(tree);
        (p, nodes)
    };
    let (p, nodes) = published(&f, &fs);
    let find = |role, label: &str| {
        (nodes.iter())
            .find(|(_, n)| n.role() == role && n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap()
    };
    let (_, view) = (nodes.iter())
        .find(|(_, n)| n.role() == Role::ScrollView)
        .unwrap();
    assert!(view.clips_children());
    let cancel = find(Role::DefaultButton, "Cancel");
    assert!(
        !view.children().contains(&cancel),
        "a kept button hangs off the view"
    );
    let hidden = find(Role::CheckBox, "Keep it");
    let click = Request {
        node: hidden,
        act: Act::Click,
    };
    let Route::Reveal { reveal, then } = p.route(click) else {
        panic!("a hidden check box is revealed first");
    };
    assert!(then && reveal.view);
    let n = reveal.node;
    let (w, h) = (n.x1 - n.x0, n.y1 - n.y0);
    let r = Rect::new(n.x0 as i32, n.y0 as i32, w as i32, h as i32);
    assert!(fs.show(&mut f, r));
    let (p, _) = published(&f, &fs);
    let Route::Act(t) = p.route(click) else {
        panic!("in view, the click acts");
    };
    let hit = f.hit(t.at.0 as i32, t.at.1 as i32, &text, px);
    assert_eq!(hit.map(|h| f.items[h.item].id), Some("check"));
}

fn drawn(theme: &Theme, f: &Form, fs: &FormScroll, s: i32, client: (i32, i32)) -> Canvas {
    let t = theme.scaled(s);
    let text = TextRenderer::load(&t.fonts);
    let p = t.paint();
    let mut c = Canvas::new((client.0 * s) as u32, (client.1 * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    crate::controls::form::draw(&mut c, &text, &p, f, true, t.fonts.size_px);
    fs.draw(&mut c, &p, f);
    c
}

#[test]
fn a_scrolled_form_pins_at_scale_1_and_2() {
    crate::pins::fonts();
    let client = (200, 141);
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let (mut f, mut fs) = fitted(&theme, 100, client, as_built);
        for state in ["top", "end"] {
            if state == "end" {
                fs.wheel(&mut f, 1000);
            }
            for s in [1, 2] {
                let c = drawn(&theme, &f, &fs, s, client);
                let bar = fs.bar(&f).unwrap().scaled(s);
                let face = theme.colors.button_face.argb();
                let inked = |r: Rect| {
                    (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) != face))
                };
                assert!(inked(bar), "{i:?} {state} x{s}: the bar draws");
                let gap = Rect::new(0, bar.bottom(), bar.x, (109 - 102) * s);
                assert!(
                    !inked(gap),
                    "{i:?} {state} x{s}: nothing shows under the view"
                );
                got.push((format!("{} {state} {s}", i.as_str()), fnv(&c)));
            }
        }
    }
    crate::pins::check("form_scroll", &got);
}
