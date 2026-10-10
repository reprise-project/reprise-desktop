//! Two menu bars, each over a white client: the first with View under the pointer, the second with View
//! open from the keyboard, its menu showing a submenu arrow, a check, a bullet, separators and its first
//! row lit. Beside them, a context menu with greyed rows and a lit greyed row.

use reprisekit::controls::menu::{MenuItem, MenuLayout, MenuStyle, draw_menu_at};
use reprisekit::controls::menubar::Bar;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 250;

fn bar() -> Bar {
    let view = vec![
        MenuItem::item("&Toolbars").submenu(true),
        MenuItem::item("Status &Bar").checked(true),
        MenuItem::Separator,
        MenuItem::item("T&humbnails"),
        MenuItem::item("Tile&s").checked(true).radio(true),
        MenuItem::item("&Details"),
        MenuItem::Separator,
        MenuItem::item("R&efresh"),
    ];
    Bar::new(vec![
        ("&File", vec![MenuItem::item("&New")]),
        ("&Edit", vec![MenuItem::item("&Undo")]),
        ("&View", view),
        ("&Help", vec![MenuItem::item("&About")]),
    ])
}

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let px = theme.fonts.size_px;
    let hot_client = Rect::new(r.x + 10, r.y + 10, 200, 40);
    let open_client = Rect::new(r.x + 10, r.y + 60, 200, 180);
    for client in [hot_client, open_client] {
        c.fill_rect(client, theme.colors.window);
    }
    let mut hot = bar();
    let cm = theme.look.controls();
    let lay = hot.layout(text, px, hot_client, &cm);
    let f = lay.frames[2];
    hot.hover(f.x + 3, f.y + 3, &lay);
    hot.draw(c, text, theme, &lay);

    let mut open = bar();
    let lay = open.layout(text, px, open_client, &cm);
    open.open_title(2, &lay, text, theme, px, true);
    open.underline = true;
    open.draw(c, text, theme, &lay);

    let items = vec![
        MenuItem::item("&Open"),
        MenuItem::item("&Run as...").enabled(false),
        MenuItem::Separator,
        MenuItem::item("Cu&t").enabled(false),
        MenuItem::item("&Copy"),
        MenuItem::Separator,
        MenuItem::item("P&roperties"),
    ];
    let style = MenuStyle::context_for(theme, &items).fit(text, &items, px, 300);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    draw_menu_at(
        c,
        (r.x + 260, r.y + 10),
        theme,
        text,
        &l,
        &items,
        Some(1),
        "",
        false,
    );
}
