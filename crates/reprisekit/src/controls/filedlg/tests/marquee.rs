use super::*;

const BACKGROUND: Option<Hit> = Some(Hit::Shell(shellview::Hit::Background));

fn picked(d: &FileDlg) -> Vec<&str> {
    d.selected().iter().map(|e| e.name.as_str()).collect()
}

/// The list's layout, and a point on its background at the body's lower right.
fn list(d: &FileDlg) -> (Rc<shellview::Layout>, (i32, i32)) {
    let lay = d.shell_layout(R, &d.drawn.0.get());
    let corner = (lay.body.right() - 5, lay.body.bottom() - 5);
    (lay, corner)
}

fn drag_to(d: &mut FileDlg, at: (i32, i32)) -> Event {
    d.motion(&text(), 11, R, at.0, at.1)
}

#[test]
fn a_drag_from_the_background_selects_the_entries_the_band_touches() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(0), Mods::default());
    let (lay, corner) = list(&d);
    d.click_at(R, BACKGROUND, Mods::default(), corner);
    assert!(picked(&d).is_empty(), "the press clears the selection");
    let a = lay.cells[1];
    assert!(matches!(drag_to(&mut d, (a.x + 2, a.y + 2)), Event::Redraw));
    assert_eq!(picked(&d), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\" \"b.txt\"");
    assert!(d.shell.marquee.is_some());
    d.release();
    assert!(d.shell.marquee.is_none(), "the band goes on release");
    assert_eq!(picked(&d), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn ctrl_adds_the_band_to_the_selection_and_escape_puts_it_back() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    let (lay, corner) = list(&d);
    d.click_at(R, BACKGROUND, CTRL, corner);
    let c = lay.cells[3];
    drag_to(&mut d, (c.x + 2, c.y + 2));
    assert_eq!(picked(&d), ["a.txt", "c.txt"]);
    d.key(R, form::Key::Escape, None, Mods::default());
    assert_eq!(picked(&d), ["a.txt"]);
    assert!(d.shell.marquee.is_none());
}

#[test]
fn a_dialog_that_picks_one_draws_no_band() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(1), Mods::default());
    let (lay, corner) = list(&d);
    d.click_at(R, BACKGROUND, Mods::default(), corner);
    let first = lay.cells[0];
    drag_to(&mut d, (first.x + 2, first.y + 2));
    assert!(d.shell.marquee.is_none());
    assert_eq!(picked(&d), ["a.txt"]);
}

#[test]
fn past_the_lists_edge_a_tick_scrolls_it_and_the_band_takes_what_came_in() {
    let mut d = multi(DlgMode::Open, true);
    let names: Vec<String> = (0..80).map(|i| format!("{i:02}.txt")).collect();
    d.set_listing(folder(2, None, names.iter().map(|n| file(n)).collect()));
    let (lay, corner) = list(&d);
    d.click_at(R, BACKGROUND, Mods::default(), (lay.body.x + 2, corner.1));
    drag_to(&mut d, (lay.body.x + 40, lay.body.y + 2));
    assert!(!d.marquee_scrolls(R), "inside the list");
    let before = picked(&d).len();
    drag_to(&mut d, (lay.body.right() + 10, lay.body.y + 2));
    assert!(d.marquee_scrolls(R));
    assert!(d.marquee_tick(R));
    assert!(d.shell.left > 0, "List scrolls sideways");
    assert!(picked(&d).len() > before);
    assert!(
        picked(&d).contains(&"00.txt"),
        "the press's column stays in the band"
    );
}

#[test]
fn the_band_in_the_list_is_drawn_as_pinned() {
    crate::pins::fonts();
    let text = text();
    let mut got = Vec::new();
    for s in [1, 2] {
        let mut d = multi(DlgMode::Open, true);
        let (lay, corner) = list(&d);
        d.click_at(R, BACKGROUND, Mods::default(), corner);
        let a = lay.cells[1];
        drag_to(&mut d, (a.x + 6, a.y + 3));
        let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
        d.draw(
            &mut c,
            &text,
            &classic_paint(&ColorScheme::default(), s),
            R,
            11,
        );
        got.push((format!("marquee {s}"), super::context_menu::fnv(&c)));
    }
    crate::pins::check("filedlg_marquee", &got);
}
