use super::*;
use repriseapp::Extended;

/// A look switch asks again with the new theme and no event in between, so an idle folder
/// window follows it both ways.
#[test]
fn the_folder_window_asks_for_a_bare_caption_under_glass_only() {
    let mut ctx = glass(false);
    let (_dir, ex) = in_folder(&mut ctx);
    let app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let blue = reprisekit::interfaces::theme_for(Interface::Blue);
    let classic = reprisekit::interfaces::theme_for(Interface::Classic);
    assert!(app.bare_caption(MAIN, &ctx.theme), "Glass");
    assert!(!app.bare_caption(MAIN, &blue), "Glass to Blue");
    assert!(
        app.bare_caption(MAIN, &glass(true).theme),
        "back to Glass, dark"
    );
    assert!(!app.bare_caption(MAIN, &classic), "Classic");
    let dialog = crate::dialogs::DIALOG;
    assert!(
        !app.bare_caption(dialog, &ctx.theme),
        "a box keeps its caption"
    );
}
