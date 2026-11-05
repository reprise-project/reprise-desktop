use super::*;
use crate::tests::{key, press, release, test_ctx};
use repriseapp::{BTN_LEFT, BTN_RIGHT};
use reprisekit::controls::colordlg::{self, BASIC, Grid};
use reprisekit::controls::palette;
use reprisekit::textedit::keysym;

/// Draws the dialog, so its form has laid its parts out, and returns their layout.
fn shown(p: &mut Paint, ctx: &Ctx) -> colordlg::Layout {
    let (w, h) = p.children().iter().find(|c| c.id == COLORS).unwrap().size;
    let mut c = Canvas::new(w as u32, h as u32);
    p.draw(COLORS, &mut c, ctx);
    colordlg::layout(&p.edit_colors.as_ref().unwrap().0.form)
}

fn click(p: &mut Paint, ctx: &mut Ctx, win: WinId, (x, y): (i32, i32), button: u32) {
    p.event(win, press(x, y, button), ctx);
    p.event(win, release(x, y, button), ctx);
}

fn mid(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn button(p: &Paint, id: &str) -> (i32, i32) {
    mid(p.edit_colors.as_ref().unwrap().0.form.rect(id).unwrap())
}

#[test]
fn edit_colors_defines_a_custom_colour_that_ok_sets_and_paint_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reprise").join("paint.toml");
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.state_path = Some(path.clone());
    p.command(menu::title::COLORS, menu::row::EDIT_COLORS, &ctx);
    let lay = shown(&mut p, &ctx);
    let d = &p.edit_colors.as_ref().unwrap().0;
    assert_eq!(
        d.dlg.color(),
        Color::rgb(0, 0, 0),
        "it opens on the first colour"
    );
    assert!(d.dlg.picked().is_some_and(|w| w.grid == Grid::Basic));
    assert!(lay.field.is_none(), "narrow until Define");
    let at = button(&p, colordlg::DEFINE);
    click(&mut p, &mut ctx, COLORS, at, BTN_LEFT);
    let lay = shown(&mut p, &ctx);
    let bar = lay.bar.unwrap();
    click(&mut p, &mut ctx, COLORS, mid(bar), BTN_LEFT);
    let field = lay.field.unwrap().inset(2);
    click(
        &mut p,
        &mut ctx,
        COLORS,
        (field.x + 60, field.y + 30),
        BTN_LEFT,
    );
    let picked = p.edit_colors.as_ref().unwrap().0.dlg.color();
    assert_ne!(picked, Color::rgb(0, 0, 0));
    let at = button(&p, colordlg::ADD);
    click(&mut p, &mut ctx, COLORS, at, BTN_LEFT);
    assert_eq!(p.custom[0], picked);
    assert_eq!(
        state::custom_colors(&path)[0],
        picked,
        "kept as it is added"
    );
    let at = button(&p, colordlg::OK);
    click(&mut p, &mut ctx, COLORS, at, BTN_LEFT);
    assert!(p.edit_colors.is_none());
    assert_eq!(p.doc.primary, picked.argb());
    assert_eq!(p.wells[0], picked, "the black well takes the colour");
    assert_eq!(p.palette().swatches[0].1, colordlg::hex(picked));

    let mut again = Paint::new(None);
    again.custom = state::custom_colors(&path);
    again.command(menu::title::COLORS, menu::row::EDIT_COLORS, &ctx);
    shown(&mut again, &ctx);
    assert_eq!(again.edit_colors.as_ref().unwrap().0.dlg.custom[0], picked);
}

#[test]
fn a_right_double_click_on_a_well_edits_it_as_the_second_colour() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    let l = p.lay(&ctx);
    let pal = p.palette();
    let red = art::PALETTE.iter().position(|(_, n)| *n == "Red").unwrap();
    let at = mid(palette::layout(&pal, l.colors).swatches[red]);
    click(&mut p, &mut ctx, MAIN, at, BTN_RIGHT);
    assert!(p.edit_colors.is_none(), "one click only picks");
    click(&mut p, &mut ctx, MAIN, at, BTN_RIGHT);
    let (d, target) = p.edit_colors.as_ref().unwrap();
    assert_eq!(
        *target,
        Target {
            secondary: true,
            well: Some(red)
        }
    );
    assert_eq!(d.dlg.color(), Color::rgb(0xFF, 0, 0));
    let lay = shown(&mut p, &ctx);
    let blue = BASIC
        .iter()
        .position(|&c| c == Color::rgb(0, 0, 0xFF))
        .unwrap();
    click(
        &mut p,
        &mut ctx,
        COLORS,
        mid(lay.cell(Grid::Basic, blue)),
        BTN_LEFT,
    );
    let right = Event::Key(repriseapp::KeyInput {
        sym: keysym::RIGHT,
        utf8: None,
        mods: Default::default(),
        edit: Some(reprisekit::textedit::EditKey::Right {
            select: false,
            word: false,
        }),
    });
    p.event(COLORS, right, &mut ctx);
    let next = BASIC[blue + 1];
    p.event(COLORS, key(keysym::RETURN, false), &mut ctx);
    assert!(p.edit_colors.is_none(), "Enter is OK");
    assert_eq!(p.doc.secondary, next.argb());
    assert_eq!(p.doc.primary, 0xFF00_0000, "the first colour stays");
    assert_eq!(p.wells[red], next);
}

#[test]
fn cancel_changes_nothing() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.command(menu::title::COLORS, menu::row::EDIT_COLORS, &ctx);
    let lay = shown(&mut p, &ctx);
    click(
        &mut p,
        &mut ctx,
        COLORS,
        mid(lay.cell(Grid::Basic, 8)),
        BTN_LEFT,
    );
    p.event(COLORS, key(keysym::ESCAPE, false), &mut ctx);
    assert!(p.edit_colors.is_none());
    assert_eq!(p.doc.primary, 0xFF00_0000);
    assert_eq!(p.wells[0], Color::rgb(0, 0, 0));
}
