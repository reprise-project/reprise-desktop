use super::*;
use crate::interface::Interface;
use crate::pins::fnv;

const BAR: i32 = 16;

fn frame(reach: (i32, i32)) -> Frame {
    Frame {
        inside: Rect::new(10, 20, 100, 80),
        reach,
        bar: BAR,
        edge: 0,
    }
}

#[test]
fn a_bar_shows_only_where_the_content_reaches_past_the_edge() {
    assert_eq!(
        frame((110, 100)).bars(),
        (false, false),
        "content that fits"
    );
    assert_eq!(frame((90, 140)).bars(), (true, false));
    assert_eq!(
        frame((100, 140)).bars(),
        (true, true),
        "the bar down takes the room the content reached into"
    );
    assert_eq!(frame((140, 90)).bars(), (true, true), "and the bar along");
    assert_eq!(frame((140, 84)).bars(), (false, true));
}

#[test]
fn the_range_and_spans_count_the_bars() {
    let f = frame((140, 140));
    assert_eq!(f.clip(), Rect::new(10, 20, 100 - BAR, 80 - BAR));
    assert_eq!(f.range(), (140 - (110 - BAR), 140 - (100 - BAR)));
    assert_eq!(f.span(), (80 - BAR, 140 - 20));
    let f = Frame { edge: 4, ..f };
    assert_eq!(f.span().0, 80 - BAR + 4, "a bevel the bar measures with");
    assert_eq!(frame((110, 100)).range(), (0, 0));
}

#[test]
fn a_scroll_is_held_to_the_range_and_returns_how_far_it_went() {
    let f = frame((90, 140));
    let mut v = ScrollView::default();
    assert_eq!(v.scroll_to(&f, 100), 40);
    assert_eq!(v.offset(), (0, 40));
    assert_eq!(v.scroll_to(&f, -5), -40);
    assert_eq!(v.wheel(&f, 2), (0, 2 * LINE_H));
    assert_eq!(
        v.wheel(&frame((140, 84)), 1).0,
        LINE_H,
        "across when only that way"
    );
}

#[test]
fn a_rect_out_of_view_scrolls_in_as_little_as_it_takes() {
    let f = frame((140, 200));
    let mut v = ScrollView::default();
    let clip = f.clip();
    assert_eq!(
        v.reveal(&f, Rect::new(20, 120, 10, 10)),
        (0, 130 - clip.bottom())
    );
    assert_eq!(v.reveal(&f, Rect::new(20, 30, 10, 10)), (0, 0), "in view");
    assert_eq!(
        v.reveal(&f, Rect::new(130, 0, 10, 10)),
        (140 - clip.right(), -clip.y),
        "a rect above and right of the view"
    );
}

#[test]
fn the_bars_arrow_steps_its_track_pages_and_its_thumb_drags() {
    let f = frame((90, 200));
    let lay = layout(&f, Rect::new(10, 20, 100, 80), BAR);
    let bar = lay.bar.unwrap();
    assert_eq!(lay.hbar, None);
    assert_eq!(lay.clip, Rect::new(10, 20, 100 - BAR, 80));
    let mut v = ScrollView::default();
    let x = bar.x + 4;
    let hit = |v: &ScrollView, y| v.hit(&f, &lay, 8, x, y).unwrap();
    let down = hit(&v, bar.bottom() - 2);
    assert_eq!(v.press(&f, down), (0, LINE_H));
    assert!(v.ticking());
    assert!(v.release());
    let track = hit(&v, bar.bottom() - BAR - 2);
    assert_eq!(v.press(&f, track), (0, f.span().0));
    v.release();
    v.scroll_to(&f, 0);
    let thumb = v.bar_parts(&f, bar, 8).thumb.unwrap();
    let grab = hit(&v, thumb.y + 1);
    v.press(&f, grab);
    assert!(v.holding());
    let to = v.drag(&f, &lay, 8, x, bar.bottom() + 50);
    assert_eq!(to, Some((0, f.range().1)), "dragged to the end");
    assert!(v.release());
    assert_eq!(v.drag(&f, &lay, 8, x, 0), None, "nothing held");
}

#[test]
fn the_view_is_a_scroll_view_node_with_a_scroll_bar_per_bar() {
    let f = frame((140, 200));
    let lay = layout(&f, Rect::new(10, 20, 100, 80), BAR);
    let mut v = ScrollView::default();
    v.scroll_to(&f, 30);
    let mut tree = Tree::window("T", (200, 200));
    let id = v.access(&f, &lay, &mut tree, access::ROOT, "page");
    let n = tree.get(id).unwrap();
    assert_eq!(n.role(), Role::ScrollView);
    assert!(n.clips_children());
    assert_eq!(n.bounds(), Some(access::bounds(lay.clip)));
    let bars: Vec<_> = (tree.nodes())
        .filter(|(_, n)| n.role() == Role::ScrollBar)
        .map(|(_, n)| (n.orientation(), n.numeric_value(), n.max_numeric_value()))
        .collect();
    let (across, down) = f.range();
    assert_eq!(
        bars,
        [
            (Some(Orientation::Vertical), Some(30.0), Some(down.into())),
            (
                Some(Orientation::Horizontal),
                Some(0.0),
                Some(across.into())
            ),
        ]
    );
}

#[test]
fn both_bars_pin_at_scale_1_and_2() {
    let f = frame((140, 200));
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let theme = crate::interfaces::theme_for(i).scaled(s);
            let p = theme.paint();
            let w = crate::interfaces::theme_for(i).controls().scroll;
            let lay = layout(&f, Rect::new(10, 20, 100, 80), w);
            let mut v = ScrollView::default();
            v.scroll_to(&f, 40);
            let mut c = Canvas::new((120 * s) as u32, (110 * s) as u32);
            c.fill_rect(c.bounds(), p.cs.button_face);
            v.draw(&mut c, &p, &f, &lay);
            let face = p.cs.button_face.argb();
            let inked = |r: Rect| {
                let r = r.scaled(s);
                (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) != face))
            };
            assert!(
                inked(lay.bar.unwrap()) && inked(lay.hbar.unwrap()),
                "{i:?} x{s}"
            );
            assert!(!inked(lay.clip), "{i:?} x{s}: the clip is the host's");
            got.push((format!("{} {s}", i.as_str()), fnv(&c)));
        }
    }
    crate::pins::check("scroll_view", &got);
}
