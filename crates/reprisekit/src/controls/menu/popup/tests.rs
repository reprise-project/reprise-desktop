use super::super::tests::{nested_ctx, nested_tree, row_point, wide_theme};
use super::*;
use crate::text::TextRenderer;
use crate::theme::FontConfig;

fn hover_ms(theme: &Theme) -> Duration {
    Duration::from_millis(theme.metrics.menu_hover_ms as u64)
}

#[test]
fn hovering_a_submenu_row_opens_it_and_leaving_it_closes_it() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let now = Instant::now();
    let later = now + hover_ms(&theme);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    let (x, y) = row_point(&p, 0, 1);
    assert!(p.motion_at(&t, x, y, &cx, now));
    assert!(p.settle(later));
    assert_eq!(p.depth(), 2, "resting on the row opens its child");
    let (cxp, cyp) = row_point(&p, 1, 2);
    assert!(p.motion_at(&t, cxp, cyp, &cx, later));
    assert_eq!(p.levels()[0].hovered, Some(1));
    assert_eq!(p.levels()[1].hovered, Some(2));
    assert!(!p.motion_at(&t, cxp, cyp, &cx, later));
    assert_eq!(p.depth(), 2);
    assert_eq!(p.click(&t, cxp, cyp, &cx), MenuAction::Chosen(vec![1, 2]));
    let (px0, py0) = row_point(&p, 0, 0);
    assert!(p.motion_at(&t, px0, py0, &cx, later));
    assert!(p.settle(later + hover_ms(&theme)));
    assert_eq!(p.depth(), 1);
    assert_eq!(p.click(&t, 700, 560, &cx), MenuAction::Close);
}

#[test]
fn a_hovered_submenu_opens_only_once_the_hover_delay_has_passed() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let now = Instant::now();
    let delay = hover_ms(&theme);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    let (x, y) = row_point(&p, 0, 1);
    assert!(p.motion_at(&t, x, y, &cx, now));
    assert_eq!(p.depth(), 1, "the submenu waits");
    assert_eq!(p.due(), Some(now + delay));
    assert!(!p.motion_at(&t, x + 1, y, &cx, now + delay / 2));
    assert_eq!(
        p.due(),
        Some(now + delay),
        "moving along the row keeps the wait"
    );
    assert!(!p.settle(now + delay / 2));
    assert!(p.settle(now + delay));
    assert_eq!(p.depth(), 2);
    assert_eq!(p.due(), None);
}

#[test]
fn crossing_a_row_on_the_way_to_an_open_submenu_keeps_it_open() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let now = Instant::now();
    let delay = hover_ms(&theme);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    let (x, y) = row_point(&p, 0, 1);
    p.motion_at(&t, x, y, &cx, now);
    p.settle(now + delay);
    let (ax, ay) = row_point(&p, 0, 3);
    p.motion_at(&t, ax, ay, &cx, now + delay);
    assert_eq!(p.depth(), 2, "a row crossed in passing closes nothing yet");
    let (cxp, cyp) = row_point(&p, 1, 0);
    p.motion_at(&t, cxp, cyp, &cx, now + delay + delay / 2);
    assert!(!p.settle(now + delay * 3));
    assert_eq!(p.depth(), 2, "reaching the submenu drops the pending close");
    assert_eq!(
        p.levels()[0].hovered,
        Some(1),
        "the row it hangs off stays lit"
    );
}

#[test]
fn a_zero_hover_delay_opens_the_submenu_at_once() {
    let mut theme = wide_theme();
    theme.metrics.menu_hover_ms = 0;
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    let (x, y) = row_point(&p, 0, 1);
    assert!(p.motion(&t, x, y, &cx));
    assert_eq!(p.depth(), 2);
}
