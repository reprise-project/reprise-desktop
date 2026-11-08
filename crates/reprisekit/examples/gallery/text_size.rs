use super::cued;
use reprisekit::art::viewsicon;
use reprisekit::classic::frame::{Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::{listbox, listview, menubar, msgbox, shellview, toolbar};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, Theme};

/// A message box over a property sheet over a window's controls (a menu bar with its File menu
/// open, a list view, a tree and a calendar), then a window's and a dialog's frame, in each look,
/// laid out for 100% text and drawn at `pct`.
pub fn render_text_size(pct: u32) -> Canvas {
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let themes: Vec<Theme> = looks
        .iter()
        .map(|&i| cued(interfaces::theme_for(i)).with_text_size(pct))
        .collect();
    let u = themes[0].text;
    let notice = msgbox::Fields {
        text: "Reprise cannot find 'zzzzq'.\nCheck the spelling and try again.".into(),
        icon: None,
        buttons: vec!["OK".into(), "Cancel".into()],
        default: 0,
        min_text_h: None,
        min_w: 0,
        max_w: 0,
    };
    let page = || {
        form::Form::new(vec![
            Item::group("g", Rect::new(14, 38, 262, 94), "Options"),
            Item::label_at("nl", 24, 58, 60, "&Name:"),
            Item::edit("name", Rect::new(84, 54, 180, 21), "Reprise", true),
            Item::check("c", 24, 84, "&Show the clock").checked(true),
            Item::radio("r1", 24, 104, "&Small icons", "size").checked(true),
            Item::radio("r2", 138, 104, "&Large icons", "size"),
            Item::label_at("sl", 14, 146, 60, "&Size:"),
            Item::combo("size", 84, 142, 120, &["Small", "Medium", "Large"], 3),
            Item::new(
                "list",
                Rect::new(14, 172, 262, 3 * listbox::ROW_H + 4),
                Control::ListBox {
                    list: listbox::ListBox::new(
                        vec!["Clock".into(), "Volume".into(), "Network".into()],
                        Some(0),
                    ),
                    icons: Vec::new(),
                },
            ),
        ])
    };
    let sheet_size = (290, 270);
    let (sw, sh) = u.size(sheet_size);
    let base_px = Theme::default().fonts.size_px;
    let text0 = TextRenderer::load(&themes[0].fonts);
    let (mw, mh) = u.size(msgbox::client(&notice, &text0, u.px(base_px), u));
    let col = sw.max(mw) + 10;
    let (cw, ch) = u.size(TEXT_SIZE_CONTROLS);
    let (pw, ph) = u.size(TEXT_SIZE_PANES);
    let (vw, vh) = u.size(TEXT_SIZE_VIEWS);
    let col = col.max(cw + 10).max(pw + 10).max(vw + 10);
    let fh = themes
        .iter()
        .map(|t| {
            FrameGeometry::layout(&t.metrics, 100, 20, false, Chrome::WINDOW)
                .total
                .1
        })
        .max()
        .unwrap_or(0);
    let mut c = Canvas::new(
        (themes.len() as i32 * col + 10) as u32,
        (mh + sh + ch + ph + vh + 2 * fh + 80) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let x = 10 + i as i32 * col;
        let (bw, bh) = u.size(msgbox::client(&notice, &text, px, u));
        let mut box_c = Canvas::new(bw as u32, bh as u32);
        box_c.fill_rect(box_c.bounds(), theme.colors.button_face);
        let f = msgbox::form(&notice, &text, px, u);
        form::draw(&mut box_c, &text, &theme.paint(), &f, true, px);
        c.blit_canvas(x, 10, &box_c);
        let sheet = PropSheet::new(&["General"], vec![page()], sheet_size);
        let mut sheet_c = Canvas::new(sw as u32, sh as u32);
        sheet_c.fill_rect(sheet_c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut sheet_c, &text, &theme.paint(), &sheet, true, px);
        c.blit_canvas(x, mh + 20, &sheet_c);
        c.blit_canvas(x, mh + sh + 30, &text_size_controls(theme, &text));
        c.blit_canvas(x, mh + sh + ch + 40, &text_size_panes(theme, &text));
        c.blit_canvas(x, mh + sh + ch + ph + 50, &text_size_views(theme, &text));
        let caption_text = theme.look.style().map_or_else(
            || TextRenderer::load(&theme.fonts),
            |st| TextRenderer::load(&st.caption_text.font()),
        );
        let top = mh + sh + ch + ph + vh + 60;
        for (row, chrome) in [Chrome::WINDOW, Chrome::HELP_DIALOG]
            .into_iter()
            .enumerate()
        {
            let g = FrameGeometry::layout(&theme.metrics, col - 10, 20, false, chrome)
                .translated(-x, -(top + row as i32 * (fh + 10)));
            let st = FrameState {
                title: "Notes",
                active: true,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            look::draw_frame(&mut c, theme, &text, &caption_text, &g, &st);
            c.fill_rect(g.client, theme.colors.window);
        }
    }
    c
}

/// The shell views' panel in `render_text_size`, 100% px.
const TEXT_SIZE_VIEWS: (i32, i32) = (700, 250);

/// Explorer's big toolbar over the shell view in Icons, grouped Tiles and Thumbnails, laid out
/// with `theme`'s controls grown by its text.
fn text_size_views(theme: &Theme, text: &TextRenderer) -> Canvas {
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let u = theme.text;
    let (w, h) = u.size(TEXT_SIZE_VIEWS);
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let p = theme.paint();
    let labelled = |id, label: &str| toolbar::Item::DropButton {
        id,
        icon: None,
        label: Some(label.into()),
        enabled: true,
    };
    let tb = toolbar::ToolBar {
        items: vec![
            labelled(toolbar::BACK, "Back"),
            toolbar::Item::Separator,
            toolbar::Item::Button {
                id: toolbar::FOLDERS,
                icon: None,
                label: Some("Folders".into()),
                enabled: true,
            },
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw24()),
                label: None,
                enabled: true,
            },
        ],
        hot: None,
        pressed: None,
        latched: Some(2),
        big: true,
    };
    let strip = Rect::new(0, 0, w, toolbar::height(true, &cm));
    let rects = toolbar::layout(&tb, text, px, strip, &cm);
    toolbar::draw(&mut c, text, &p, &tb, strip, &rects, px);
    let views = [
        (shellview::Mode::Icons, false),
        (shellview::Mode::Tiles, true),
        (shellview::Mode::Thumbnails, false),
    ];
    let vw = (w - 4 * 4) / 3;
    for (i, (mode, grouped)) in views.into_iter().enumerate() {
        let sv = shellview::sample(mode, grouped);
        let r = Rect::new(
            4 + i as i32 * (vw + 4),
            strip.bottom() + 4,
            vw,
            h - strip.bottom() - 8,
        );
        let lay = shellview::layout(&sv, r, &cm);
        shellview::draw(&mut c, text, &p, &sv, &lay, None, px);
    }
    c
}

/// The panes' panel in `render_text_size`, 100% px.
const TEXT_SIZE_PANES: (i32, i32) = (600, 330);

/// A grouped tile list over a task pane, beside a mixer, laid out with `theme`'s controls grown by
/// its text.
fn text_size_panes(theme: &Theme, text: &TextRenderer) -> Canvas {
    use reprisekit::controls::mixer;
    use reprisekit::controls::taskpane::{self, Row, Section, TaskPane};
    use reprisekit::controls::tilelist::{self, Entry, TILE_H, Tile, TileList};
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let u = theme.text;
    let (w, h) = u.size(TEXT_SIZE_PANES);
    let mut c = Canvas::new(w as u32, h as u32);
    let cs = &theme.colors;
    c.fill_rect(c.bounds(), cs.button_face);
    let p = theme.paint();
    let mut list = TileList::new(TILE_H);
    list.entries = vec![Entry::Header("Audio".into()), Entry::Row(0), Entry::Row(1)];
    list.select_row(Some(1));
    let lr = Rect::new(4, 4, u.len(220), u.len(31 + 2 * TILE_H + 4));
    let lay = tilelist::layout(&list, lr, &cm);
    let rows = [
        ("Headphones", "Connected", "80%"),
        ("Speaker", "Not connected", ""),
    ];
    tilelist::draw(&mut c, text, &p, &list, &lay, px, |c, rr, i, sel| {
        let (title, caption, tag) = rows[i];
        let tile = Tile {
            icon: None,
            title,
            tag,
            caption,
            greyed: false,
        };
        tilelist::draw_tile(c, text, cs, rr, &tile, sel, px, lay.text);
    });
    let tp = TaskPane {
        sections: vec![
            Section::links(
                "Network Tasks",
                vec![Row::new("View available networks", None)],
            ),
            Section::lines("Details", vec![("Local Area Connection".into(), true)]),
        ],
        ..Default::default()
    };
    let pr = Rect::new(4, lr.bottom() + 6, u.len(220), h - lr.bottom() - 10);
    let pl = taskpane::layout(&tp, text, px, pr, &cm);
    taskpane::draw(&mut c, text, &p, &tp, &pl, px);
    let line = |key: &str, volume, check| mixer::Line {
        key: key.into(),
        title: key.into(),
        volume,
        balance: Some(10),
        check,
        checked: false,
        live: true,
        boost: false,
    };
    let m = mixer::Mixer::new(
        &[
            line("Volume Control", 42, mixer::Check::MuteAll),
            line("Firefox", 48, mixer::Check::Mute),
        ],
        true,
    );
    let (mw, mh) = m.size(u);
    let body = Rect::new(lr.right() + 8, 4, mw, mh);
    let ml = mixer::layout(&m, theme, text, body);
    mixer::draw(&mut c, theme, text, &m, &ml);
    c
}

/// The window controls' panel in `render_text_size`, 100% px.
const TEXT_SIZE_CONTROLS: (i32, i32) = (420, 330);

/// A menu bar with its File menu open over a grouped list view, a tree and a calendar, laid out
/// with `theme`'s controls grown by its text.
fn text_size_controls(theme: &Theme, text: &TextRenderer) -> Canvas {
    use reprisekit::controls::calendar;
    use reprisekit::controls::menubar::Bar;
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let u = theme.text;
    let (w, h) = u.size(TEXT_SIZE_CONTROLS);
    let mut c = Canvas::new(w as u32, h as u32);
    let cs = &theme.colors;
    c.fill_rect(c.bounds(), cs.button_face);
    let p = theme.paint();
    let mut lv = listview::sample();
    lv.edit = None;
    lv.groups = vec![listview::Group {
        title: "Documents".into(),
        first: 2,
    }];
    let top = menubar::bar_h(&cm);
    let lr = Rect::new(4, top + 4, w - 8, u.len(130));
    listview::draw(
        &mut c,
        text,
        &p,
        &lv,
        &listview::layout(&lv, lr, &cm),
        None,
        px,
    );
    let mut docs = Node::new("My Documents", true);
    docs.expanded = true;
    docs.children = vec![Node::new("Letters", false), Node::new("Photos", true)];
    let mut tv = TreeView::new(
        vec![docs, Node::new("Recycle Bin", false)],
        treeview::Style::Classic,
    );
    tv.selected = Some(vec![0, 1]);
    let tr = Rect::new(4, lr.bottom() + 6, u.len(190), u.len(130));
    let tl = treeview::layout(&tv, text, px, tr, &cm);
    treeview::draw(&mut c, text, &p, &tv, &tl, None, px);
    let mut cal = calendar::Calendar::new(2026, 9, Some(24), 0);
    cal.today = Some((2026, 9, 3));
    let (gw, gh) = calendar::size(&cm);
    let gl = calendar::layout(Rect::new(tr.right() + 8, tr.y, gw, gh), &cm);
    calendar::draw(&mut c, text, &p, &cal, &gl, false, px);
    let mut bar = Bar::new(vec![
        (
            "&File",
            vec![
                MenuItem::item("&New"),
                MenuItem::item("&Open..."),
                MenuItem::Separator,
                MenuItem::item("E&xit"),
            ],
        ),
        ("&Edit", vec![MenuItem::item("&Undo")]),
        ("&View", vec![MenuItem::item("&Status Bar")]),
    ]);
    let lay = bar.layout(text, px, Rect::new(0, 0, w, h), &cm);
    bar.open_title(0, &lay, text, theme, px, true);
    bar.underline = true;
    bar.draw(&mut c, text, theme, &lay);
    c
}
