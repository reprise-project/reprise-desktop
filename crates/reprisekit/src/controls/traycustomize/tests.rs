use super::*;
use std::sync::Arc;

use super::geometry::{CLASSIC_LIST_MIN_H, PAGE_TIP};
use crate::access::accesskit::{Action, Role};
use crate::classic::widgets::{BarHit, LINE_H};
use crate::controls::metrics::ControlMetrics;
use crate::controls::shellview::list_icon_px;
use crate::controls::{combo, form};
use crate::cues::Cues;
use crate::glass;
use crate::icon::IconSource;
use crate::interface::Interface;
use crate::interfaces::theme_with;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{TextUnits, Theme};

fn text() -> TextRenderer {
    TextRenderer::load(&crate::theme::FontConfig::default())
}

fn glass_theme() -> Theme {
    theme_with(
        Interface::Glass,
        crate::blue::Variant::Blue,
        &glass::Settings::default(),
    )
}

fn entry(key: &str, name: &str, passive: bool, b: Behaviour) -> Entry {
    let red = IconSource {
        width: 16,
        height: 16,
        pixels: vec![0xFFFF_0000; 256],
    };
    Entry {
        key: key.into(),
        name: name.into(),
        detail: format!("{name} is running"),
        icon: Arc::from([red]),
        passive,
        behaviour: b,
    }
}

fn metrics(w: Wording) -> ControlMetrics {
    match w {
        Wording::Classic => Theme::default().look.controls(),
        Wording::Glass => glass_theme().look.controls(),
    }
}

fn sample(w: Wording) -> TrayCustomize {
    TrayCustomize::new(
        w,
        vec![
            entry("nm-applet", "Network", false, Behaviour::Auto),
            entry("volume", "Volume", true, Behaviour::Auto),
            entry("mail", "Mail", false, Behaviour::Show),
        ],
        vec![entry("sticky", "Sticky Keys", false, Behaviour::Hide)],
        (w == Wording::Glass).then_some(false),
        metrics(w),
    )
}

#[test]
fn a_readers_focus_on_a_page_row_focuses_its_combo_without_opening_or_picking() {
    use crate::access::{Act, Published, Request, Route, Tree};
    let (text, px) = (TextRenderer::load(&crate::theme::FontConfig::default()), 11);
    let mut t = sample(Wording::Glass);
    let mail = t.entries.iter().position(|e| e.name == "Mail").unwrap();
    assert_ne!(t.selected, Some(mail));
    let behaviour = t.entries[mail].behaviour;
    let lay = t.layout(&text, px, (0, 0));
    let mut tree = Tree::window("", t.client());
    t.access(&mut tree, &lay, &text, px);
    let row = tree.find(Role::ListBoxOption, "Mail").unwrap();
    let mut p = Published::default();
    p.update(tree);
    let Route::Act(target) = p.route(Request {
        node: row,
        act: Act::Focus,
    }) else {
        panic!("Mail's row is in view");
    };
    assert_eq!(target.role, Role::ComboBox);
    let (x, y) = (target.at.0 as i32, target.at.1 as i32);
    assert_eq!(t.focus_on(t.hit(&lay, x, y)), Event::Redraw);
    assert_eq!((t.focus, t.selected), (Focus::Combo, Some(mail)));
    assert_eq!(t.combo.open, None);
    assert_eq!(t.entries[mail].behaviour, behaviour);
    let lay = t.layout(&text, px, (0, 0));
    let mut tree = Tree::window("", t.client());
    let focus = t.access(&mut tree, &lay, &text, px);
    let n = tree.get(focus).unwrap();
    assert_eq!((n.role(), n.label()), (Role::ComboBox, Some("Mail")));
}

#[test]
fn a_reader_hears_every_rows_behaviour_not_only_the_selected_ones() {
    use crate::access::Tree;
    let (text, px) = (TextRenderer::load(&crate::theme::FontConfig::default()), 11);
    for (w, want) in [
        (
            Wording::Glass,
            [
                "Show icon and notifications",
                "Only show notifications",
                "Show icon and notifications",
                "Only show notifications",
            ],
        ),
        (
            Wording::Classic,
            [
                "Hide when inactive",
                "Hide when inactive",
                "Always show",
                "Always hide",
            ],
        ),
    ] {
        let t = sample(w);
        let lay = t.layout(&text, px, (0, 0));
        let mut tree = Tree::window("", t.client());
        t.access(&mut tree, &lay, &text, px);
        for (name, want) in ["Network", "Volume", "Mail", "Sticky Keys"]
            .iter()
            .zip(want)
        {
            let row = tree.find(Role::ListBoxOption, name).unwrap();
            let n = tree.get(row).unwrap();
            assert_eq!(n.description(), Some(want), "{w:?} {name}");
            if w == Wording::Glass {
                assert!(
                    !n.supports_action(Action::Click) && !n.supports_action(Action::Focus),
                    "{name} picks by its combo"
                );
                let combo = tree
                    .find(Role::ComboBox, name)
                    .expect("each row has a combo");
                let c = tree.get(combo).unwrap();
                assert!(c.supports_action(Action::Click));
                let picked = tree.nodes().find(|(id, o)| {
                    o.is_selected() == Some(true)
                        && tree.get(*id).and_then(|o| o.label()) == Some(want)
                        && c.children()
                            .iter()
                            .any(|&l| tree.get(l).is_some_and(|l| l.children().contains(id)))
                });
                assert!(picked.is_some(), "{name}'s combo shows {want}");
            }
        }
    }
}

#[test]
fn the_selected_page_rows_open_combo_keeps_its_name_and_its_rows_theirs() {
    use crate::access::Tree;
    let (text, px) = (TextRenderer::load(&crate::theme::FontConfig::default()), 11);
    let mut t = sample(Wording::Glass);
    t.select(2);
    assert!(t.combo.toggle());
    let lay = t.layout(&text, px, (0, 0));
    let mut tree = Tree::window("", t.client());
    t.access(&mut tree, &lay, &text, px);
    let combo = tree
        .find(Role::ComboBox, "Mail")
        .expect("the open combo is named");
    assert_eq!(tree.get(combo).unwrap().is_expanded(), Some(true));
    for b in Wording::Glass.choices() {
        let label = Wording::Glass.label(*b);
        assert!(tree.find(Role::ListBoxOption, label).is_some(), "{label}");
    }
}

#[test]
fn a_reader_scrolls_a_cut_page_combo_in_by_its_entry_once_the_list_is_scrolled() {
    use crate::access::{Published, Tree};
    let (text, px) = (TextRenderer::load(&crate::theme::FontConfig::default()), 11);
    let current = (0..13).map(|i| {
        entry(
            &format!("c{i}"),
            &format!("Item {i}"),
            false,
            Behaviour::Auto,
        )
    });
    let w = Wording::Glass;
    let mut t = TrayCustomize::new(w, current.collect(), Vec::new(), Some(false), metrics(w));
    t.top = 3;
    let lay = t.layout(&text, px, (0, 0));
    let mut tree = Tree::window("", t.client());
    t.access(&mut tree, &lay, &text, px);
    let mut p = Published::default();
    p.update(tree.clone());
    let cut: Vec<_> = lay
        .fields
        .iter()
        .filter_map(|&(i, _)| {
            let combo = tree.find(Role::ComboBox, &format!("Item {i}"))?;
            Some((i, p.reveal(combo)?.row))
        })
        .collect();
    assert!(!cut.is_empty(), "a combo is cut by the list's bottom");
    for (i, row) in cut {
        assert_eq!(row, i, "Item {i}'s combo scrolls in its own entry");
    }
}

#[test]
fn a_reader_scrolls_a_past_item_in_and_focuses_a_button_without_pressing_it() {
    use crate::access::{Published, Tree};
    let (text, px) = (TextRenderer::load(&crate::theme::FontConfig::default()), 11);
    let current = (0..3).map(|i| {
        entry(
            &format!("c{i}"),
            &format!("Current {i}"),
            false,
            Behaviour::Auto,
        )
    });
    let past = (0..12).map(|i| {
        entry(
            &format!("p{i}"),
            &format!("Past {i}"),
            false,
            Behaviour::Hide,
        )
    });
    let w = Wording::Classic;
    let mut t = TrayCustomize::new(w, current.collect(), past.collect(), None, metrics(w));
    let published = |t: &TrayCustomize| {
        let lay = t.layout(&text, px, (0, 0));
        let mut tree = Tree::window("", t.client());
        t.access(&mut tree, &lay, &text, px);
        let first = tree.find(Role::ListBoxOption, "Current 0").unwrap();
        let last = tree.find(Role::ListBoxOption, "Past 11").unwrap();
        let mut p = Published::default();
        p.update(tree);
        (p, first, last)
    };
    let (p, first, last) = published(&t);
    assert_eq!(p.reveal(first), None, "the first item shows");
    let r = p.reveal(last).expect("the last past item is out of view");
    assert_eq!(r.row, 14);
    let picked = t.selected;
    assert!(t.reveal(None, r.row));
    let (p, first, last) = published(&t);
    assert_eq!(p.reveal(last), None, "the last past item shows");
    assert!(p.reveal(first).is_some(), "the first is out of view");
    assert_eq!(t.selected, picked, "the pick stays");
    let lay = t.layout(&text, px, (0, 0));
    let at = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
    let (x, y) = at(lay.cancel);
    assert_eq!(t.focus_on(t.hit(&lay, x, y)), Event::Redraw);
    assert_eq!(t.focus, Focus::Cancel);
    assert!(!t.holding(), "Cancel is not pressed");
}

#[test]
fn a_behaviour_hides_by_status_only_when_left_auto() {
    assert!(Behaviour::Auto.hidden(true));
    assert!(!Behaviour::Auto.hidden(false));
    assert!(!Behaviour::Show.hidden(true), "always shown");
    assert!(Behaviour::Hide.hidden(false), "always hidden");
    assert!(Behaviour::Mute.hidden(false));
    assert!(Behaviour::Hide.balloons(), "only the icon hides");
    assert!(!Behaviour::Mute.balloons(), "icon and balloons hide");
}

#[test]
fn each_interface_names_the_behaviours_in_its_own_words() {
    let c = Wording::Classic;
    let labels: Vec<_> = c.choices().iter().map(|&b| c.label(b)).collect();
    assert_eq!(labels, ["Hide when inactive", "Always hide", "Always show"]);
    let g = Wording::Glass;
    let labels: Vec<_> = g.choices().iter().map(|&b| g.label(b)).collect();
    assert_eq!(
        labels,
        [
            "Show icon and notifications",
            "Hide icon and notifications",
            "Only show notifications"
        ]
    );
    // A choice one interface has no row for shows as the row that
    // does the same to the icon.
    assert_eq!(c.index(Behaviour::Mute, false), 1, "Always hide");
    assert_eq!(g.index(Behaviour::Auto, true), 2, "Only show notifications");
    assert_eq!(g.index(Behaviour::Auto, false), 0, "Show icon...");
}

#[test]
fn the_list_groups_current_items_before_past_ones() {
    let t = sample(Wording::Classic);
    assert_eq!(
        t.slots(),
        [
            Slot::Header(0),
            Slot::Row(0),
            Slot::Row(1),
            Slot::Row(2),
            Slot::Header(1),
            Slot::Row(3)
        ]
    );
    let none_past = TrayCustomize::new(
        Wording::Classic,
        vec![entry("a", "A", false, Behaviour::Auto)],
        vec![],
        None,
        metrics(Wording::Classic),
    );
    assert_eq!(none_past.slots(), [Slot::Header(0), Slot::Row(0)]);
}

#[test]
fn the_classic_layout_has_its_parts_where_the_dialog_puts_them() {
    let t = sample(Wording::Classic);
    let text = text();
    let l = t.layout(&text, 11, (0, 0));
    assert_eq!(t.client(), (348, 391));
    assert_eq!(l.frame, Rect::new(9, 82, 330, 208));
    assert_eq!(
        l.header,
        Rect::new(11, 83, 326, 17),
        "a row under the frame"
    );
    assert_eq!(l.body, Rect::new(11, 100, 326, 188), "few rows: no bar");
    assert_eq!(l.columns[0], Rect::new(10, 83, 186, 17));
    assert_eq!(l.columns[1], Rect::new(196, 83, 124, 17));
    let ys: Vec<_> = l.slots.iter().map(|(_, r)| (r.y, r.height)).collect();
    assert_eq!(
        ys,
        [
            (100, 32),
            (132, 18),
            (150, 18),
            (168, 18),
            (186, 43),
            (229, 18)
        ]
    );
    assert_eq!(l.restore, Rect::new(246, 307, 93, 23));
    assert_eq!(l.ok, Rect::new(183, 359, 75, 23));
    assert_eq!(l.cancel, Rect::new(264, 359, 75, 23));
    assert!(
        l.check.is_none(),
        "Classic's switch is on the settings sheet"
    );
}

/// At 200% text the Classic dialog's client, list, rows and buttons grow with the text; the
/// header keeps its one row under the frame, and the selected row's combo grows with it.
#[test]
fn a_grown_classic_dialog_grows_its_parts() {
    let mut t = sample(Wording::Classic);
    t.metrics = metrics(Wording::Classic).grown(TextUnits::new(22, 11));
    t.select(1);
    let l = t.layout(&text(), 22, (0, 0));
    assert_eq!(t.client(), (696, 782));
    assert_eq!(l.frame, Rect::new(18, 164, 660, 416));
    assert_eq!(l.header, Rect::new(20, 165, 656, 34));
    assert_eq!(l.body.y, 199);
    let hs: Vec<_> = l.slots.iter().map(|(_, r)| r.height).collect();
    assert_eq!(hs, [64, 36, 36, 36, 86, 36]);
    assert_eq!(l.ok, Rect::new(366, 718, 150, 46));
    let (_, cl) = l.combo.expect("a combo on the selected row");
    assert_eq!(cl.field, Rect::new(20 + 370 + 1, 199 + 64 + 36, 248, 42));
}

/// A row's icon is `list_icon_px` square at each text size, drawn from the smallest copy at
/// least that big; the Classic name's highlight starts where the icon ends.
#[test]
fn a_rows_icon_grows_with_the_text() {
    let (red, blue) = (0xFFFF_0000, 0xFF00_00FF);
    let source = |px: u32, argb: u32| IconSource {
        width: px,
        height: px,
        pixels: vec![argb; (px * px) as usize],
    };
    let text = text();
    for (pct, ink) in [(100, red), (125, blue), (150, blue)] {
        for (w, theme) in [
            (Wording::Classic, Theme::default()),
            (Wording::Glass, glass_theme()),
        ] {
            let theme = theme.with_text_size(pct);
            let px = theme.fonts.size_px;
            let want = list_icon_px(theme.controls().text);
            assert_eq!(want > 16, pct > 100);
            let mut e = entry("mail", "Mail", false, Behaviour::Show);
            e.icon = Arc::from([source(16, red), source(32, blue)]);
            let mut t = TrayCustomize::new(w, vec![e], Vec::new(), None, theme.controls());
            t.select(0);
            let l = t.layout(&text, px, (0, 0));
            let (cw, ch) = t.client();
            let mut c = Canvas::new(cw as u32, ch as u32);
            t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
            let (mut xs, mut ys) = (Vec::new(), Vec::new());
            for y in 0..ch {
                for x in 0..cw {
                    if c.get(x, y) == ink {
                        xs.push(x);
                        ys.push(y);
                    }
                }
            }
            let span = |v: &[i32]| v.iter().max().unwrap() - v.iter().min().unwrap() + 1;
            assert_eq!(xs.len(), (want * want) as usize, "{w:?} at {pct}%");
            assert_eq!((span(&xs), span(&ys)), (want, want), "{w:?} at {pct}%");
            if w == Wording::Classic {
                let (right, y) = (*xs.iter().max().unwrap(), ys[0] + 2);
                let hilight = theme.colors.hilight.argb();
                assert_eq!(
                    c.get(right + 1, y),
                    hilight,
                    "{pct}%: highlight after the icon"
                );
            }
        }
    }
}

#[test]
fn the_selected_rows_combo_is_its_behaviour_cell() {
    let mut t = sample(Wording::Classic);
    let text = text();
    t.select(1);
    let l = t.layout(&text, 11, (0, 0));
    let (i, cl) = l.combo.expect("a combo on the selected row");
    assert_eq!(i, 1);
    assert_eq!(cl.field, Rect::new(11 + 186, 150, 124, 21));
    assert_eq!(t.combo.value(), "Hide when inactive");
    assert_eq!(t.combo.items.len(), 3);
}

#[test]
fn many_rows_bring_the_bar_beside_the_header() {
    let many: Vec<_> = (0..12)
        .map(|i| {
            entry(
                &format!("k{i}"),
                &format!("Item {i}"),
                false,
                Behaviour::Auto,
            )
        })
        .collect();
    let t = TrayCustomize::new(
        Wording::Classic,
        many,
        vec![],
        None,
        metrics(Wording::Classic),
    );
    let text = text();
    let l = t.layout(&text, 11, (0, 0));
    assert_eq!(l.vbar, Some(Rect::new(321, 84, 16, 204)));
    assert_eq!(l.body.width, 310);
    assert_eq!(l.page, 1 + 8, "the header and eight whole rows");
}

#[test]
fn picking_a_behaviour_changes_only_the_selected_item() {
    let mut t = sample(Wording::Classic);
    let text = text();
    assert_eq!(t.click(Some(Hit::Row(0))), Event::Redraw);
    let l = t.layout(&text, 11, (0, 0));
    let field = l.combo.as_ref().unwrap().1.field;
    t.click(t.hit(&l, field.x + 5, field.y + 5));
    assert_eq!(t.combo.open, Some(0), "the list opens on the pick");
    let l = t.layout(&text, 11, (0, 0));
    let list = l.combo.as_ref().unwrap().1.list;
    let hit = t.hit(&l, list.x + 5, list.y + 1 + 2 * 13 + 3);
    assert_eq!(hit, Some(Hit::Combo(combo::Hit::Row(2))));
    t.click(hit);
    assert_eq!(t.entries[0].behaviour, Behaviour::Show);
    assert_eq!(t.entries[1].behaviour, Behaviour::Auto);
    assert_eq!(t.click(Some(Hit::Ok)), Event::Redraw);
    let Event::Accept(ch) = t.release(Some(Hit::Ok)) else {
        panic!("OK accepts");
    };
    assert_eq!(ch.behaviours[0], ("nm-applet".into(), Behaviour::Show));
    assert_eq!(ch.behaviours[3], ("sticky".into(), Behaviour::Hide));
    assert_eq!(ch.all_icons, None);
}

#[test]
fn re_picking_the_row_an_auto_item_shows_keeps_it_auto() {
    let mut t = sample(Wording::Glass);
    t.select(1);
    assert_eq!(t.combo.selected, 2);
    t.take_pick(2);
    assert_eq!(t.entries[1].behaviour, Behaviour::Auto);
    t.take_pick(1);
    assert_eq!(t.entries[1].behaviour, Behaviour::Mute);
}

#[test]
fn restore_defaults_puts_every_item_back_on_its_status() {
    let mut t = sample(Wording::Classic);
    assert_eq!(t.key(form::Key::Access('r')), Event::Redraw);
    assert!(t.entries.iter().all(|e| e.behaviour == Behaviour::Auto));
    assert_eq!(t.key(form::Key::Escape), Event::Cancel);
}

#[test]
fn the_keyboard_walks_the_rows_and_steps_the_combo() {
    let mut t = sample(Wording::Classic);
    assert_eq!(t.key(form::Key::Down), Event::Redraw);
    assert_eq!(t.selected, Some(0));
    t.key(form::Key::End);
    assert_eq!(t.selected, Some(3), "into the past items");
    t.key(form::Key::Tab { back: false });
    assert_eq!(t.focus, Focus::Combo);
    t.key(form::Key::Up);
    assert_eq!(
        t.entries[3].behaviour,
        Behaviour::Auto,
        "Always hide to Hide when inactive"
    );
    t.key(form::Key::Tab { back: false });
    assert_eq!(t.focus, Focus::Restore, "Classic has no check box");
    assert_eq!(t.key(form::Key::Enter), Event::Redraw, "Enter restores");
    assert!(t.entries.iter().all(|e| e.behaviour == Behaviour::Auto));
}

#[test]
fn glass_ticks_its_box_and_greys_the_list() {
    let mut t = sample(Wording::Glass);
    let text = text();
    let theme = glass_theme();
    let cm = theme.look.controls();
    let l = t.layout(&text, theme.fonts.size_px, (0, 0));
    assert_eq!(l.check, Some(Rect::new(92, 528, cm.check, cm.check)));
    t.click(Some(Hit::AllIcons));
    t.release(Some(Hit::AllIcons));
    assert_eq!(t.all_icons, Some(true));
    assert!(!t.list_enabled());
    assert_eq!(t.click(Some(Hit::Row(0))), Event::None, "greyed");
    let Event::Accept(ch) = t.key(form::Key::Enter) else {
        panic!("Enter is OK");
    };
    assert_eq!(ch.all_icons, Some(true));
}

/// Ink rows and columns of `c` inside `r` that differ from `bg`.
fn ink(c: &Canvas, r: Rect, bg: u32) -> Option<(i32, i32, i32, i32)> {
    let mut out: Option<(i32, i32, i32, i32)> = None;
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if c.get(x, y) != bg {
                out = Some(match out {
                    None => (x, y, x, y),
                    Some((a, b, cc, d)) => (a.min(x), b.min(y), cc.max(x), d.max(y)),
                });
            }
        }
    }
    out
}

#[test]
fn classic_pixels_sit_where_the_dialog_draws_them() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = text();
    let px = theme.fonts.size_px;
    let mut t = sample(Wording::Classic);
    t.select(1);
    let l = t.layout(&text, px, (0, 0));
    let mut c = Canvas::new(348, 391);
    t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
    let face = cs.button_face.argb();
    let white = cs.window.argb();
    assert_eq!(c.get(100, 82), cs.button_shadow.argb());
    assert_eq!(c.get(100, 83), cs.button_dk_shadow.argb());
    assert_eq!(
        c.get(100, 84),
        face,
        "the header's top hides under the frame"
    );
    assert_eq!(c.get(100, 99), cs.button_shadow.argb());
    assert_eq!(c.get(100, 100), white);
    assert_eq!(c.get(195, 90), cs.button_shadow.argb());
    assert_eq!(c.get(196, 90), cs.button_hilight.argb());
    assert_eq!(c.get(11, 90), face, "no hilight on the first column");
    assert_eq!(
        c.get(11, 119),
        crate::theme::Color::rgb(166, 202, 240).argb()
    );
    assert_eq!(c.get(11, 118), white);
    assert_eq!(c.get(31, 150), cs.hilight.argb());
    assert_eq!(c.get(31, 166), cs.hilight.argb());
    assert_eq!(c.get(31, 167), white);
    assert_ne!(c.get(30, 150), cs.hilight.argb(), "the icon's last column");
    assert_eq!(c.get(11 + 186, 158), cs.button_shadow.argb(), "combo frame");
    assert_eq!(c.get(15, 132), 0xFFFF_0000);
    assert_eq!(c.get(14, 140), white);
    assert_eq!(c.get(246, 307), cs.button_hilight.argb());
    assert_eq!(c.get(338, 329), cs.button_dk_shadow.argb());
    assert_eq!(c.get(183, 359), cs.window_frame.argb(), "default ring");
    if !crate::text::test_tahoma() {
        return;
    }
    let intro = ink(&c, Rect::new(9, 0, 330, 30), face).unwrap();
    assert_eq!(intro.1, 13, "intro ink top");
    let prompt = ink(&c, Rect::new(9, 62, 330, 16), face).unwrap();
    assert_eq!(prompt.1, 68, "prompt ink top");
    let title = ink(&c, Rect::new(11, 101, 100, 17), white).unwrap();
    assert_eq!((title.0, title.1), (26, 106), "group title ink corner");
    let name = ink(&c, Rect::new(32, 132, 150, 17), white).unwrap();
    assert_eq!(name.1, 136, "a row's name ink top");
    assert_eq!(name.0, 11 + 22, "a row's name ink left");
}

#[test]
fn the_glass_page_has_its_parts_where_the_page_puts_them() {
    let t = sample(Wording::Glass);
    let text = text();
    let theme = glass_theme();
    let l = t.layout(&text, theme.fonts.size_px, (0, 0));
    assert_eq!(t.client(), (1184, 602));
    assert_eq!(l.frame, Rect::new(92, 84, 1000, 403));
    assert_eq!(l.vbar, Some(Rect::new(1074, 85, 17, 401)), "always there");
    assert_eq!(l.body, Rect::new(93, 132, 981, 354));
    let ys: Vec<_> = l.slots.iter().map(|&(s, r)| (s, r.y, r.height)).collect();
    assert_eq!(
        ys,
        [
            (Slot::Row(0), 132, 48),
            (Slot::Row(1), 180, 48),
            (Slot::Row(2), 228, 48),
            (Slot::Row(3), 276, 48)
        ]
    );
    let fields: Vec<_> = l.fields.iter().map(|&(i, r)| (i, r)).collect();
    assert_eq!(fields[0], (0, Rect::new(584, 140, 279, 23)));
    assert_eq!(fields[3], (3, Rect::new(584, 284, 279, 23)));
    assert_eq!(l.page, 4, "every row shows");
    assert_eq!(
        l.combo.as_ref().map(|c| c.0),
        Some(0),
        "the first is focused"
    );
    assert_eq!(l.ok, Rect::new(934, 569, 76, 23));
    assert_eq!(l.cancel, Rect::new(1016, 569, 76, 23));
    assert_eq!(l.band, Some(Rect::new(0, 559, 1184, 43)));
    assert_eq!((l.restore.x, l.restore.y), (92, 497));
    assert_eq!(t.focus, Focus::Combo);
}

/// The page drawn in `t`'s state, with its layout.
fn page_pixels(t: &TrayCustomize) -> (Canvas, Layout, Theme) {
    let theme = glass_theme();
    let text = text();
    let px = theme.fonts.size_px;
    let l = t.layout(&text, px, (0, 0));
    let (w, h) = t.client();
    let mut c = Canvas::new(w as u32, h as u32);
    t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
    (c, l, theme)
}

#[test]
fn a_glass_tip_is_set_in_its_fonts_italic() {
    let text = TextRenderer::load(&crate::theme::FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..Default::default()
    });
    if !text.has_face("DejaVu Sans") {
        return;
    }
    let t = sample(Wording::Glass);
    let theme = glass_theme();
    let px = theme.fonts.size_px;
    let l = t.layout(&text, px, (0, 0));
    let (w, h) = t.client();
    let mut c = Canvas::new(w as u32, h as u32);
    t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
    let inks = theme.look.style().and_then(|st| st.page).unwrap();
    let (_, row) = l.slots[0];
    let tip = Rect::new(
        l.frame.x + PAGE_TIP.x,
        row.y + PAGE_TIP.y,
        300,
        PAGE_TIP.height,
    );
    let alone = |face: &TextRenderer| {
        let mut a = Canvas::new(w as u32, h as u32);
        a.fill_rect(tip, theme.colors.window);
        let detail = &t.entries[0].detail;
        face.draw(&mut a, tip, detail, false, inks.detail, Align::Left, px);
        a
    };
    let same = |a: &Canvas| {
        (tip.y..tip.bottom()).all(|y| (tip.x..tip.right()).all(|x| a.get(x, y) == c.get(x, y)))
    };
    assert!(
        same(&alone(text.italic())),
        "the tip in DejaVu Sans Oblique"
    );
    assert!(!same(&alone(&text)), "not upright");
}

#[test]
fn the_glass_page_pixels_sit_where_the_page_draws_them() {
    if !crate::text::test_tahoma() {
        return;
    }
    let (c, _, theme) = page_pixels(&sample(Wording::Glass));
    let inks = theme.look.style().and_then(|st| st.page).unwrap();
    let (cs, white) = (&theme.colors, theme.colors.window.argb());
    let at = |r: Rect, bg: u32| ink(&c, r, bg).map(|(x, y, ..)| (x, y));
    assert_eq!(at(Rect::new(80, 10, 900, 30), white), Some((93, 23)));
    assert!(any(&c, Rect::new(80, 10, 900, 30), inks.heading.argb()));
    assert_eq!(at(Rect::new(80, 45, 1000, 25), white), Some((92, 57)));
    assert_eq!(c.get(92, 84), inks.list_frame.argb());
    assert_eq!(c.get(1091, 486), inks.list_frame.argb());
    assert_eq!(c.get(93, 85), white);
    assert_eq!(at(Rect::new(100, 95, 100, 20), white), Some((127, 106)));
    assert_eq!(
        at(Rect::new(560, 95, 100, 20), white).map(|p| p.0),
        Some(586)
    );
    assert_eq!(c.get(125, 118), inks.rule.argb());
    assert_eq!(c.get(1041, 118), inks.rule.argb());
    assert_eq!((c.get(124, 118), c.get(1042, 118)), (white, white));
    assert_eq!(
        (c.get(133, 137), c.get(148, 152)),
        (0xFFFF_0000, 0xFFFF_0000)
    );
    assert_eq!(c.get(132, 137), white);
    assert_eq!(at(Rect::new(150, 135, 300, 12), white), Some((159, 140)));
    assert_eq!(at(Rect::new(150, 151, 300, 14), white), Some((159, 155)));
    assert!(any(&c, Rect::new(150, 151, 300, 14), inks.detail.argb()));
    for y in [140, 188, 236, 284] {
        assert_eq!(c.get(700, y), 0xFF6D_6D6D, "combo top at {y}");
        assert_eq!(c.get(700, y + 22), 0xFF6D_6D6D, "combo foot at {y}");
    }
    assert_eq!(at(Rect::new(80, 497, 300, 20), white), Some((92, 502)));
    assert!(any(&c, Rect::new(80, 497, 300, 20), inks.link.argb()));
    assert_ne!(c.get(93, 529), white, "the box");
    assert_eq!(at(Rect::new(105, 522, 300, 20), white), Some((111, 529)));
    assert_eq!(c.get(500, 558), white);
    assert_eq!(c.get(500, 559), inks.rule.argb());
    assert_eq!(
        (c.get(500, 560), c.get(500, 601)),
        (cs.button_face.argb(), cs.button_face.argb())
    );
    let face = cs.button_face.argb();
    assert_eq!(
        at(Rect::new(900, 562, 284, 40), face).map(|p| p.1),
        Some(570)
    );
    assert_eq!(
        ink(&c, Rect::new(900, 562, 284, 40), face).map(|b| (b.0, b.2)),
        Some((935, 1090))
    );
}

/// Whether `c` has any pixel of `argb` in `r`.
fn any(c: &Canvas, r: Rect, argb: u32) -> bool {
    (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == argb))
}

#[test]
fn the_glass_box_greys_the_combos_and_the_link_but_not_the_rows() {
    let mut t = sample(Wording::Glass);
    t.all_icons = Some(true);
    let (c, l, theme) = page_pixels(&t);
    let inks = theme.look.style().and_then(|st| st.page).unwrap();
    let white = theme.colors.window.argb();
    assert_eq!(c.get(700, 140), 0xFFAB_AFB3);
    assert_eq!(c.get(700, 150), 0xFFF3_F4F4);
    assert!(any(&c, Rect::new(584, 140, 200, 23), 0xFF6B_6B6B));
    assert!(any(&c, l.restore, inks.detail.argb()), "the link greys");
    assert!(!any(&c, l.restore, inks.link.argb()));
    assert!(any(
        &c,
        Rect::new(150, 135, 300, 12),
        theme.colors.window_text.argb()
    ));
    assert_eq!(c.get(133, 137), 0xFFFF_0000);
    assert_eq!(c.get(93, 85), white);
    assert_eq!(
        t.click(Some(Hit::Row(1))),
        Event::None,
        "the combos are greyed"
    );
    assert_eq!(t.click(Some(Hit::Restore)), Event::None, "so is the link");
}

#[test]
fn the_glass_pointer_lights_a_combo_and_the_link() {
    let mut t = sample(Wording::Glass);
    assert_eq!(t.motion(Some(Hit::Row(2))), Event::Redraw);
    assert_eq!(t.motion(Some(Hit::Row(2))), Event::None, "no change");
    let (c, ..) = page_pixels(&t);
    assert_eq!(c.get(750, 236), 0xFF3B_77AD);
    assert_eq!(c.get(750, 238), 0xFFE9_F5FD);
    assert_eq!(c.get(750, 188), 0xFF6D_6D6D);
    t.motion(Some(Hit::Restore));
    let (c, l, theme) = page_pixels(&t);
    let inks = theme.look.style().and_then(|st| st.page).unwrap();
    assert_eq!(c.get(750, 236), 0xFF6D_6D6D, "back at rest");
    assert!(any(&c, l.restore, inks.link_hot.argb()));
    let r = l.restore;
    let row = r.y + 15;
    assert!(
        (r.x..r.right()).all(|x| c.get(x, row) == inks.link_hot.argb()),
        "underline"
    );
    assert_eq!(t.leave(), Event::Redraw);
}

#[test]
fn a_glass_combo_opens_under_a_click_and_tab_walks_the_combos() {
    let mut t = sample(Wording::Glass);
    let text = text();
    let px = glass_theme().fonts.size_px;
    let l = t.layout(&text, px, (0, 0));
    assert_eq!(t.hit(&l, 700, 200), Some(Hit::Row(1)), "row 1's combo");
    assert_eq!(t.hit(&l, 300, 200), Some(Hit::List), "beside it");
    assert_eq!(t.click(Some(Hit::Row(1))), Event::Redraw);
    assert_eq!((t.selected, t.combo.open), (Some(1), Some(2)));
    let (c, ..) = page_pixels(&t);
    assert_eq!(
        c.get(750, 188),
        0xFF2B_5B87,
        "pressed while its list is open"
    );
    let over = Some(Hit::Combo(combo::Hit::Row(0)));
    assert_eq!(t.motion(over), Event::Redraw);
    assert_eq!(t.combo.open, Some(0), "the highlight follows the pointer");
    t.key(form::Key::Escape);
    assert_eq!(t.combo.open, None);
    t.key(form::Key::Tab { back: false });
    assert_eq!((t.focus, t.selected), (Focus::Combo, Some(2)));
    t.key(form::Key::Tab { back: false });
    t.key(form::Key::Tab { back: false });
    assert_eq!(t.focus, Focus::AllIcons, "past the last combo");
    t.key(form::Key::Tab { back: true });
    assert_eq!((t.focus, t.selected), (Focus::Combo, Some(3)));
    t.key(form::Key::Up);
    assert_eq!(t.entries[3].behaviour, Behaviour::Mute, "Up and Down pick");
}

#[test]
fn the_glass_list_scrolls_a_row_at_a_time() {
    let many: Vec<_> = (0..9)
        .map(|i| {
            entry(
                &format!("k{i}"),
                &format!("Item {i}"),
                false,
                Behaviour::Auto,
            )
        })
        .collect();
    let mut t = TrayCustomize::new(
        Wording::Glass,
        many,
        vec![],
        Some(false),
        metrics(Wording::Glass),
    );
    assert!(t.scroll(1));
    assert!(t.scroll(5));
    assert_eq!(t.top, 2, "nine rows, seven to a page");
    assert!(!t.scroll(1));
    let l = t.layout(&text(), glass_theme().fonts.size_px, (0, 0));
    assert_eq!(l.fields[0], (2, Rect::new(584, 140, 279, 23)));
}

/// At 125% (14 px) with every glyph 9 px wide, Classic's intro wraps onto a fourth line:
/// what lies under it moves down a line and the client grows by one. Restore Defaults
/// widens leftward to its label and 3 px a side.
#[test]
fn classic_fits_its_intro_and_restore_label() {
    let text = TextRenderer::stub(14, |_| 9);
    let mut t = sample(Wording::Classic);
    t.metrics = metrics(Wording::Classic).grown(TextUnits::new(14, 11));
    assert_eq!(t.client(), (443, 498));
    t.fit(&text, 14);
    assert_eq!(t.intro.len(), 4);
    let l = t.layout(&text, 14, (0, 0));
    assert_eq!(t.client(), (443, 514));
    assert_eq!(l.intro.height, 66);
    assert_eq!(l.prompt.y, 101);
    assert_eq!(
        (l.restore.x, l.restore.right(), l.restore.y),
        (279, 431, 407)
    );
    assert!(t.overflows(&text, 14).is_empty());
}

/// The sample page at 200% (22 px) in `room`, fitted with every glyph 11 px wide.
fn grown_page(room: Option<(i32, i32)>) -> (TrayCustomize, TextRenderer) {
    let text = TextRenderer::stub(22, |_| 11);
    let mut t = sample(Wording::Glass);
    t.metrics = metrics(Wording::Glass).grown(TextUnits::new(22, 11));
    t.room = room;
    t.fit(&text, 22);
    (t, text)
}

/// With room for all of it, the page at 200% is twice its 100% size, its parts with it.
#[test]
fn a_grown_glass_page_grows_its_parts() {
    let (t, text) = grown_page(None);
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (2368, 1204));
    assert_eq!(l.frame, Rect::new(184, 168, 2000, 806));
    assert_eq!(l.body.y, 168 + 96);
    assert_eq!(l.slots[1].1.y, 168 + 96 + 96, "rows twice as tall");
    assert_eq!(l.fields[0], (0, Rect::new(184 + 984, 280, 558, 46)));
    assert_eq!(l.intro, Rect::new(184, 104, 2000, 40), "one line");
    assert_eq!(l.ok, Rect::new(1868, 1138, 152, 46));
    assert_eq!(l.band, Some(Rect::new(0, 1118, 2368, 86)));
    assert_eq!(l.view, Rect::new(0, 0, 2368, 1118));
    assert_eq!(l.page_bar, None);
    assert!(t.overflows(&text, 22).is_empty());
}

/// A 1080p output at 200% has room for a 1900x1000 client: the page narrows to 1900, its
/// Behaviors column moves in and the intro wraps onto a second line that pushes the list
/// down. The band keeps to the client's foot and the page above it scrolls 115 px at 100%.
#[test]
fn a_page_bigger_than_its_room_narrows_and_scrolls_above_its_band() {
    let (t, text) = grown_page(Some((1900, 1000)));
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (1900, 1000));
    assert_eq!(t.intro.len(), 2);
    assert_eq!(l.intro, Rect::new(184, 104, 1532, 66));
    assert_eq!(l.frame, Rect::new(184, 194, 1532, 806));
    assert_eq!(l.fields[0].1.x, 184 + 874);
    assert_eq!(l.band, Some(Rect::new(0, 914, 1900, 86)));
    assert_eq!(l.view, Rect::new(0, 0, 1900, 914));
    assert_eq!(l.page_bar, Some(Rect::new(1883, 0, 17, 914)));
    assert_eq!(l.ok, Rect::new(1400, 934, 152, 46));
    assert_eq!(l.cancel.x, 1564);
    assert_eq!(t.page_scroll.short, 115);
    assert!(t.overflows(&text, 22).is_empty());
}

/// A 1024x768 output at 200% has room for a 1000x640 client: the page takes its margins in
/// to 24 px and its combos to 210 px, and OK and Cancel keep inside its right edge.
#[test]
fn a_page_on_a_small_output_takes_in_its_margins_and_combos() {
    let (t, text) = grown_page(Some((1000, 640)));
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (1000, 640));
    assert_eq!(l.frame.x, 48);
    assert_eq!(l.frame.right(), 1000 - 48);
    assert_eq!(l.prompt.x, 48);
    assert_eq!(l.fields[0].1.width, 420);
    assert_eq!(l.fields[0].1.right(), l.frame.right() - 100);
    assert_eq!(l.cancel.right(), 1000 - 48);
    assert_eq!(l.ok.right() + 12, l.cancel.x);
    assert!(l.page_bar.is_some());
    assert!(t.overflows(&text, 22).is_empty());
}

/// An 800x600 output at 200% has room for a 776x540 client, narrower than the columns need:
/// the page stacks each row's combo under its name and tip and wraps its heading.
#[test]
fn a_page_narrower_than_its_columns_stacks_its_rows() {
    let text = TextRenderer::load(&crate::theme::FontConfig::default());
    let mut t = sample(Wording::Glass);
    t.metrics = metrics(Wording::Glass).grown(TextUnits::new(22, 11));
    t.room = Some((776, 540));
    t.fit(&text, 22);
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (776, 540));
    assert_eq!(l.frame.x, 48);
    let &(_, row) = l
        .slots
        .iter()
        .find(|(s, _)| matches!(s, Slot::Row(0)))
        .expect("the first row");
    assert_eq!(row.height, 146, "73 px rows");
    let (_, combo) = l.fields[0];
    assert_eq!(
        combo,
        Rect::new(48 + 134, row.y + 84, 400, 46),
        "under the name"
    );
    assert_eq!(l.columns[1].width, 0, "no Behaviors column");
    assert!(t.heading.len() > 1, "the heading wraps");
    assert_eq!(l.intro.y, l.prompt.bottom() + 20);
    assert!(l.cancel.right() <= 776);
    assert_eq!(t.overflows(&text, 22), vec![]);
}

/// At 200% Classic's client is 696x782; with room for 640 px its list shortens and what lies
/// under it moves up, so OK and Cancel keep inside. With room for all, nothing moves.
#[test]
fn classic_shortens_its_list_to_its_room() {
    let text = TextRenderer::stub(22, |_| 11);
    let mut t = sample(Wording::Classic);
    t.metrics = metrics(Wording::Classic).grown(TextUnits::new(22, 11));
    t.fit(&text, 22);
    let full = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (696, 782));
    t.room = Some((1000, 900));
    assert_eq!(t.layout(&text, 22, (0, 0)), full);
    t.room = Some((1000, 640));
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(t.client(), (696, 640));
    assert_eq!(l.frame.height, full.frame.height - 142);
    assert_eq!(l.ok.y, full.ok.y - 142);
    assert_eq!(l.restore.y, full.restore.y - 142);
    assert!(l.cancel.bottom() < 640);
    t.room = Some((1000, 100));
    assert_eq!(
        t.client().1,
        782 - 2 * (208 - CLASSIC_LIST_MIN_H),
        "the list keeps a group and three rows"
    );
}

/// Off the list the wheel scrolls the page three lines a notch; on a list that cannot
/// scroll, the page too. The page stops at its end, the band stays.
#[test]
fn the_wheel_scrolls_the_page_off_the_list_and_where_it_stops() {
    let (mut t, text) = grown_page(Some((1900, 1000)));
    assert!(t.wheel(None, 1));
    assert_eq!(t.page_scroll.top, 39);
    let l = t.layout(&text, 22, (0, 0));
    assert_eq!(l.frame.y, 194 - 78);
    assert_eq!(l.band, Some(Rect::new(0, 914, 1900, 86)));
    assert!(t.wheel(Some(Hit::List), 1), "four rows show whole");
    assert_eq!(t.page_scroll.top, 78);
    assert!(t.wheel(Some(Hit::Row(0)), 1));
    assert_eq!(t.page_scroll.top, 115, "held at the end");
    assert!(!t.wheel(None, 1));
    assert!(t.wheel(None, -5));
    assert_eq!(t.page_scroll.top, 0);
}

/// Tab walks the combos, the box and the link; each scrolls the page as little as shows it
/// whole, down and back up.
#[test]
fn keyboard_focus_scrolls_the_page_to_show_the_focused_part() {
    let (mut t, _) = grown_page(Some((1900, 600)));
    assert_eq!((t.page_scroll.short, t.page_scroll.top), (315, 0));
    let tab = |t: &mut TrayCustomize, back: bool| {
        t.key(form::Key::Tab { back });
        (t.focus, t.selected, t.page_scroll.top)
    };
    assert_eq!(tab(&mut t, false), (Focus::Combo, Some(1), 0));
    assert_eq!(tab(&mut t, false), (Focus::Combo, Some(2), 15));
    assert_eq!(tab(&mut t, false), (Focus::Combo, Some(3), 63));
    assert_eq!(tab(&mut t, false), (Focus::AllIcons, Some(3), 298));
    assert_eq!(tab(&mut t, false), (Focus::Restore, Some(3), 298));
    assert_eq!(tab(&mut t, false), (Focus::Ok, Some(3), 298));
    t.focus = Focus::AllIcons;
    assert_eq!(tab(&mut t, true), (Focus::Combo, Some(3), 297));
    assert_eq!(tab(&mut t, true), (Focus::Combo, Some(2), 249));
}

/// The page's bar: an arrow steps a line and repeats while held, the track a view, the
/// thumb follows the pointer.
#[test]
fn the_page_bar_steps_repeats_and_drags() {
    use crate::controls::scrollbar::REPEAT_AFTER;
    let (mut t, text) = grown_page(Some((1900, 600)));
    let l = t.layout(&text, 22, (0, 0));
    let bar = l.page_bar.unwrap();
    let down = t.hit(&l, bar.x + 8, bar.bottom() - 4);
    assert_eq!(down, Some(Hit::PageBar(BarHit::Forward)));
    t.click(down);
    assert_eq!(t.page_scroll.top, LINE_H);
    assert!(t.ticking() && t.holding());
    let t0 = std::time::Instant::now();
    assert_eq!(t.tick(t0), Event::None, "the first tick starts the wait");
    assert_eq!(t.tick(t0 + REPEAT_AFTER), Event::Redraw);
    assert_eq!(t.page_scroll.top, 2 * LINE_H);
    t.release(down);
    assert!(!t.ticking());
    t.click(Some(Hit::PageBar(BarHit::PageForward)));
    assert_eq!(t.page_scroll.top, 2 * LINE_H + 257, "a view");
    t.release(None);
    let l = t.layout(&text, 22, (0, 0));
    let parts = t.page_scroll.parts(bar, 257, 545, l.thumb_min);
    let thumb = parts.thumb.unwrap();
    let grab = t.hit(&l, bar.x + 8, thumb.y + 2);
    assert!(matches!(grab, Some(Hit::PageBar(BarHit::Thumb(_)))));
    t.click(grab);
    assert_eq!(t.drag(&l, -500), Some(Event::Redraw));
    assert_eq!(t.page_scroll.top, 0);
    assert_eq!(t.release(None), Event::Redraw);
}

/// Scrolled, the page's parts draw only above the band: the band, its rule and the buttons
/// stay at the client's foot and the bar runs down the right edge, at either scale.
#[test]
fn a_scrolled_page_draws_its_parts_above_the_band() {
    let (mut t, text) = grown_page(Some((1900, 1000)));
    t.wheel(None, 1);
    let theme = glass_theme().with_text_size(200);
    let inks = theme.look.style().and_then(|st| st.page).unwrap();
    let (cs, white) = (&theme.colors, theme.colors.window.argb());
    for s in [1, 2] {
        let l = t.layout(&text, 22, (0, 0));
        let mut c = Canvas::new((1900 * s) as u32, (1000 * s) as u32);
        let scaled = theme.scaled(s);
        let p = scaled.paint();
        t.draw(&mut c, &text, &p, &l, Cues::NONE, 22);
        assert_eq!(c.get(500 * s, l.frame.y * s), inks.list_frame.argb());
        assert_eq!(c.get(500 * s, (l.frame.y - 1) * s), white);
        assert_eq!(c.get(500 * s, 914 * s), inks.rule.argb(), "the band's rule");
        assert_eq!(c.get(500 * s, 999 * s), cs.button_face.argb());
        assert_ne!(c.get(1891 * s, 400 * s), white, "the bar");
        assert_eq!(c.get(1880 * s, 400 * s), white);
    }
    let (mut t, text) = grown_page(Some((1900, 600)));
    t.wheel(None, 3);
    let l = t.layout(&text, 22, (0, 0));
    assert!(
        l.frame.bottom() > l.view.bottom(),
        "the list runs under the band"
    );
    let mut c = Canvas::new(1900, 600);
    t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, 22);
    let band = l.band.unwrap();
    for y in band.y + 1..band.bottom() {
        assert_ne!(c.get(185, y), white, "no list under the band at {y}");
    }
}

#[test]
#[ignore]
fn preview() {
    let text = text();
    for (name, theme, w) in [
        ("classic", Theme::default(), Wording::Classic),
        ("glass", glass_theme(), Wording::Glass),
    ] {
        let px = theme.fonts.size_px;
        let mut t = sample(w);
        t.select(1);
        let l = t.layout(&text, px, (0, 0));
        let (cw, ch) = t.client();
        let mut c = Canvas::new(cw as u32, ch as u32);
        t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
        crate::raster::write_preview(&c, &format!("traycustomize-{name}-preview.png"));
    }
    let theme = glass_theme();
    let px = theme.fonts.size_px;
    let draw = |t: &TrayCustomize, name: &str| {
        let l = t.layout(&text, px, (0, 0));
        let (cw, ch) = t.client();
        let mut c = Canvas::new(cw as u32, ch as u32);
        t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
        crate::raster::write_preview(&c, &format!("traycustomize-glass-{name}.png"));
    };
    let mut t = sample(Wording::Glass);
    t.motion(Some(Hit::Row(2)));
    t.pointer.motion(Some(Focus::Restore));
    draw(&t, "hot");
    t.click(Some(Hit::Row(0)));
    draw(&t, "open");
    let mut t = sample(Wording::Glass);
    t.all_icons = Some(true);
    draw(&t, "greyed");
    let theme = glass_theme().with_text_size(200);
    let px = theme.fonts.size_px;
    let mut t = sample(Wording::Glass);
    t.metrics = theme.controls();
    t.room = Some((1900, 1000));
    t.fit(&text, px);
    t.wheel(None, 1);
    let l = t.layout(&text, px, (0, 0));
    let (cw, ch) = t.client();
    let mut c = Canvas::new(cw as u32, ch as u32);
    t.draw(&mut c, &text, &theme.paint(), &l, Cues::NONE, px);
    crate::raster::write_preview(&c, "traycustomize-glass-200.png");
}
