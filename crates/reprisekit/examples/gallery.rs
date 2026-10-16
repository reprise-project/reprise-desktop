//! Draws every reprisekit control into PNGs in `target/`. Run with
//! `cargo run -p reprisekit --example gallery`; output names are in `main`.
//! `REPRISE_SCHEME=<name>` (a name from `schemes::names()`, or `all`) picks the
//! colour scheme. `REPRISE_GALLERY_SCALE=<n>` writes only the parts pages, drawn
//! by styles built for scale `n` (see `gallery_parts`), and the Blue tray arrow
//! page. `REPRISE_GALLERY_LOOK=classic` leaves the Blue bands blank and skips
//! the Blue, Glass and taskbar PNGs. `REPRISE_GALLERY_TEXT_SIZE=<percent>` writes
//! only a message box, a property sheet, window controls, panes and the shell
//! view's icon modes in each look at that text size, and About's system view.

#[path = "gallery_p3/mod.rs"]
mod gallery_p3;
#[path = "gallery_parts/mod.rs"]
mod gallery_parts;

use reprisekit::art::glyph::Glyph;
use reprisekit::art::{signalbars, viewsicon};
use reprisekit::classic::frame::{
    CaptionButton, Chrome, FrameGeometry, FrameState, Glow, draw_frame,
};
use reprisekit::controls::iconview::{self, Selection};
use reprisekit::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::pointer::Interaction;
// Classic helpers with no look::widgets twin.
use reprisekit::classic::widgets::{
    BUTTON_GAP, BUTTON_H, BUTTON_W, CLOSE_H, CLOSE_W, CheckState, ROW_H, draw_caption, draw_edit,
    draw_label,
};
use reprisekit::classic::{indicator, snapframe};
use reprisekit::controls::rebar;
use reprisekit::controls::{
    aboutbox, addressbar, browsefolder, colorbutton, colorpick, combo, displaypanel, dlgtemplate,
    endsession, explorerbar, filedlg, filesheet, finddlg,
    form::{self, Control, Item},
    grouplist, infotip, launcher, listbox, listview, locationprompt, menubar, mixer, msgbox,
    multiedit, notifyhistory, osd, placesbar, regionselect, replacebox, shellview, switcher,
    tabstrip, taskpane, taskpreview, toolbar, tooltip, updown, volumepopup, wizard,
};
use reprisekit::glass::shadow;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look::widgets::{
    draw_button, draw_checkbox, draw_checkbox_disabled, draw_checkbox_state, draw_combo_drop,
    draw_group_box, draw_hscrollbar, draw_marquee, draw_page, draw_progress, draw_radio,
    draw_radio_disabled, draw_scroll_corner, draw_scrollbar, draw_size_grip, draw_spin_buttons,
    draw_sunken_field, draw_tab, draw_trackbar, draw_vertical_trackbar,
};
use reprisekit::look::{self, Paint, sample};
use reprisekit::raster::{Canvas, EdgeStyle, Rect, write_preview};
use reprisekit::schemes;
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::{Caret, EditKey, Preedit, TextEdit};
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::trackbar;
use reprisekit::wallpaper::{self, Position};

fn main() {
    if let Ok(n) = std::env::var("REPRISE_GALLERY_SCALE") {
        let n: i32 = n.parse().expect("REPRISE_GALLERY_SCALE is a whole number");
        gallery_parts::write_all(n.max(1));
        write_preview(
            &render_tray_arrow_schemes(n.max(1)),
            "reprisekit-gallery-tray-arrow-blue-schemes.png",
        );
        write_preview(
            &render_orb_glow(n.max(1)),
            "reprisekit-gallery-orb-glow-glass.png",
        );
        write_volume_popups(true, n.max(1));
        return;
    }
    if let Ok(pct) = std::env::var("REPRISE_GALLERY_TEXT_SIZE") {
        let pct: u32 = pct
            .parse()
            .expect("REPRISE_GALLERY_TEXT_SIZE is a percentage");
        write_preview(
            &render_text_size(pct),
            &format!("reprisekit-gallery-text-size-{pct}.png"),
        );
        write_preview(
            &render_about_system(true, pct),
            &format!("reprisekit-gallery-about-system-{pct}.png"),
        );
        write_preview(
            &render_glass_launcher(pct),
            &format!("reprisekit-gallery-glass-launcher-{pct}.png"),
        );
        write_preview(
            &render_glass_clock_popup(pct),
            &format!("reprisekit-gallery-glass-clock-popup-{pct}.png"),
        );
        write_preview(
            &render_launchers(pct),
            &format!("reprisekit-gallery-launcher-{pct}.png"),
        );
        return;
    }
    let with_blue = std::env::var("REPRISE_GALLERY_LOOK").as_deref() != Ok("classic");
    let wanted = std::env::var("REPRISE_SCHEME").ok();
    let names: Vec<Option<&str>> = match wanted.as_deref() {
        None => vec![None],
        Some("all") => std::iter::once(None)
            .chain(schemes::names().map(Some))
            .collect(),
        Some(name) => vec![Some(name)],
    };
    for name in names {
        let mut theme = Theme::default();
        let mut out = "reprisekit-gallery.png".to_string();
        if let Some(name) = name {
            theme.colors = schemes::scheme(name)
                .unwrap_or_else(|| panic!("no scheme {name:?}; see schemes::names()"));
            out = format!("reprisekit-gallery-{}.png", slug(name));
        }
        write_preview(&render(&theme, with_blue), &out);
    }
    write_preview(&render_file_dialog(), "reprisekit-gallery-file-dialog.png");
    write_preview(
        &render_about_system(with_blue, 100),
        "reprisekit-gallery-about-system.png",
    );
    write_preview(
        &render_end_session(with_blue),
        "reprisekit-gallery-end-session.png",
    );
    write_preview(
        &render_monitors(with_blue),
        "reprisekit-gallery-monitors.png",
    );
    write_preview(
        &render_location_prompt(with_blue),
        "reprisekit-gallery-location-prompt.png",
    );
    write_preview(
        &render_stats_panel(with_blue),
        "reprisekit-gallery-stats-panel.png",
    );
    write_volume_popups(with_blue, 1);
    if with_blue {
        write_preview(
            &render_tray_arrow_schemes(1),
            "reprisekit-gallery-tray-arrow-blue-schemes.png",
        );
        write_preview(&render_orb_glow(1), "reprisekit-gallery-orb-glow-glass.png");
        write_preview(&render_glass(), "reprisekit-gallery-glass.png");
        write_preview(&render_glass_dark(), "reprisekit-gallery-glass-dark.png");
        write_preview(
            &render_glass_scale2(),
            "reprisekit-gallery-glass-scale2.png",
        );
        write_preview(
            &render_show_desktop(),
            "reprisekit-gallery-show-desktop.png",
        );
        write_preview(&render_taskbar_top(), "reprisekit-gallery-taskbar-top.png");
        write_preview(
            &render_taskbar_hidden(),
            "reprisekit-gallery-taskbar-hidden.png",
        );
        write_preview(
            &render_taskbar_edges(),
            "reprisekit-gallery-taskbar-edges.png",
        );
        write_preview(
            &render_taskbar_drag(),
            "reprisekit-gallery-taskbar-drag.png",
        );
        write_preview(
            &render_taskbar_cells(),
            "reprisekit-gallery-taskbar-cells.png",
        );
        write_preview(
            &render_taskbar_keys(),
            "reprisekit-gallery-taskbar-keys.png",
        );
        write_preview(&render_tray_arrow(), "reprisekit-gallery-tray-arrow.png");
        write_preview(
            &render_glass_preview(),
            "reprisekit-gallery-glass-preview.png",
        );
        write_preview(
            &render_glass_switcher(),
            "reprisekit-gallery-glass-switcher.png",
        );
        write_preview(&render_overview(), "reprisekit-gallery-overview.png");
        write_preview(
            &render_snap_layouts(),
            "reprisekit-gallery-snap-layouts.png",
        );
        write_preview(
            &render_glass_popups(),
            "reprisekit-gallery-glass-popups.png",
        );
        write_preview(&render_start_panel(), "reprisekit-gallery-start-panel.png");
        write_preview(&render_list_sheets(), "reprisekit-gallery-list-sheet.png");
        write_preview(
            &render_long_list_sheets(),
            "reprisekit-gallery-list-sheet-scrolled.png",
        );
        write_preview(&render_now_playing(), "reprisekit-gallery-now-playing.png");
        write_preview(
            &render_tray_customize(),
            "reprisekit-gallery-tray-customize.png",
        );
        write_preview(&render_backgrounds(), "reprisekit-gallery-backgrounds.png");
        write_preview(&render_savers(), "reprisekit-gallery-screensavers.png");
        write_preview(
            &render_caption_tabs(),
            "reprisekit-gallery-caption-tabs.png",
        );
        write_preview(
            &gallery_p3::render(),
            "reprisekit-gallery-blue-variants.png",
        );
    }
}

/// Three outputs in the monitor well plus Identify's badge, Classic then Blue and Glass.
fn render_monitors(with_blue: bool) -> Canvas {
    use reprisekit::controls::arrange::{self, Arrange, BADGE, Tile};
    let tile = |number, pos, size| Tile {
        number,
        name: format!("OUT-{number}"),
        pos,
        size,
    };
    let tiles = vec![
        tile(1, (0, 0), (1920, 1080)),
        tile(2, (1920, 0), (1280, 1024)),
        tile(3, (0, 1080), (1080, 1920)),
    ];
    let mut themes = vec![Theme::default()];
    if with_blue {
        themes.push(interfaces::theme_for(Interface::Blue));
        themes.push(interfaces::theme_for(Interface::Glass));
    }
    let (ww, wh) = (358, 108);
    let col = ww + BADGE + 30;
    let m = reprisekit::look::panel::margin(&themes[themes.len() - 1]);
    let mut c = Canvas::new(
        (themes.len() as i32 * col + 10 + m.left + m.right) as u32,
        (2 * wh + 30) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let (p, px) = (theme.paint(), theme.fonts.size_px);
        let x = 10 + i as i32 * col;
        let mut face = Canvas::new((ww + 20) as u32, (2 * wh + 30) as u32);
        face.fill_rect(face.bounds(), theme.colors.button_face);
        let mut a = Arrange::new(tiles.clone(), 1);
        let well = Rect::new(10, 10, ww, wh);
        arrange::draw(
            &mut face,
            &text,
            &p,
            &a,
            &arrange::layout(well, &a),
            true,
            px,
        );
        a.enabled = false;
        let greyed = Rect::new(10, 20 + wh, ww, wh);
        arrange::draw(
            &mut face,
            &text,
            &p,
            &a,
            &arrange::layout(greyed, &a),
            false,
            px,
        );
        c.blit_canvas(x, 0, &face);
        c.blit_canvas(x + ww + 25, 10, &arrange::draw_badge(theme, &text, 2));
    }
    c
}

/// The session-end prompt with a program that has a window and one that gave a reason.
fn render_end_session(with_blue: bool) -> Canvas {
    let fields = endsession::Fields {
        goal: endsession::Goal::LogOff,
        rows: vec![
            endsession::Row {
                name: "Untitled - Notepad".into(),
                reason: String::new(),
            },
            endsession::Row {
                name: "Text Editor".into(),
                reason: "There are unsaved documents".into(),
            },
        ],
        icon: None,
    };
    let mut themes = vec![Theme::default()];
    if with_blue {
        themes.push(interfaces::theme_for(Interface::Blue));
        themes.push(interfaces::theme_for(Interface::Glass));
    }
    let base = TextRenderer::load(&Theme::default().fonts);
    let (w, h) = endsession::client(
        &fields,
        &base,
        Theme::default().fonts.size_px,
        reprisekit::theme::TextUnits::ONE,
    );
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let (bw, bh) = endsession::client(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let f = endsession::form(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let mut box_c = Canvas::new(bw as u32, bh as u32);
        endsession::draw(&mut box_c, &text, &theme.paint(), &f, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// The location prompt for a program called Maps, focus on Deny.
fn render_location_prompt(with_blue: bool) -> Canvas {
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 4, 24, 24), Color::rgb(52, 160, 80));
    let fields = locationprompt::Fields {
        program: "Maps".into(),
        icon: Some(icon),
    };
    let mut themes = vec![Theme::default()];
    if with_blue {
        themes.push(interfaces::theme_for(Interface::Blue));
        themes.push(interfaces::theme_for(Interface::Glass));
    }
    let base = TextRenderer::load(&Theme::default().fonts);
    let (w, h) = locationprompt::client(
        &fields,
        &base,
        Theme::default().fonts.size_px,
        reprisekit::theme::TextUnits::ONE,
    );
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let (bw, bh) =
            locationprompt::client(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let f = locationprompt::form(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let mut box_c = Canvas::new(bw as u32, bh as u32);
        locationprompt::draw(&mut box_c, &text, &theme.paint(), &f, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// A message box over a property sheet over a window's controls (a menu bar with its File menu
/// open, a list view, a tree and a calendar) in each look, laid out for 100% text and drawn at
/// `pct`.
fn render_text_size(pct: u32) -> Canvas {
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let themes: Vec<Theme> = looks
        .iter()
        .map(|&i| interfaces::theme_for(i).with_text_size(pct))
        .collect();
    let u = themes[0].text;
    let notice = msgbox::Fields {
        text: "Reprise cannot find 'zzzzq'.\nCheck the spelling and try again.".into(),
        icon: None,
        buttons: vec!["OK".into(), "Cancel".into()],
        default: 0,
        min_text_h: None,
        min_w: 0,
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
    let mut c = Canvas::new(
        (themes.len() as i32 * col + 10) as u32,
        (mh + sh + ch + ph + vh + 60) as u32,
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

/// The debug overlay's panel with every row, in each look.
fn render_stats_panel(with_blue: bool) -> Canvas {
    use reprisekit::controls::statspanel::{self, Row, StatsPanel};
    let p = StatsPanel {
        title: "DP-1 (1920x1080 at 60 Hz)".into(),
        rows: vec![
            Row::new("Renders/s", "58"),
            Row::new("CPU/render", "1.4 ms avg, 3.9 max"),
            Row::new("Flip interval", "16.7 ms avg, 33.3 max"),
            Row::new("Damage", "3.2 rects, 12.5% area"),
            Row::new("Clients", "9"),
            Row::new("Windows", "4"),
            Row::new("Surfaces", "37"),
            Row::new("Memory", "84.2 MB"),
        ],
    };
    let mut themes = vec![Theme::default()];
    if with_blue {
        themes.push(interfaces::theme_for(Interface::Blue));
        themes.push(interfaces::theme_for(Interface::Glass));
    }
    let h = statspanel::height(p.rows.len());
    let step = statspanel::W + 40;
    let mut c = blurred_stripes(themes.len() as i32 * step + 20, h + 60);
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let m = reprisekit::look::panel::margin(theme);
        let panel = statspanel::draw(theme, &text, &p);
        c.blit_over(30 + i as i32 * step - m.left, 30 - m.top, &panel);
    }
    c
}

/// About Reprise's system view with a stand-in logo, in each look, at `pct` text size.
fn render_about_system(with_blue: bool, pct: u32) -> Canvas {
    let g = |label: &str, lines: &[&str]| aboutbox::Group {
        label: label.into(),
        lines: lines.iter().map(|s| s.to_string()).collect(),
    };
    let mut logo = Canvas::new(aboutbox::LOGO as u32, aboutbox::LOGO as u32);
    logo.fill_rect(Rect::new(8, 8, 48, 48), Color::rgb(23, 147, 209));
    let system = aboutbox::System {
        logo: Some(logo),
        groups: vec![
            g(
                "Reprise:",
                &["Version 0.1.0 (build a1b2c3d)", "Classic interface"],
            ),
            g(
                "Operating system:",
                &["Arch Linux", "Linux 6.9.0-1-generic (x86_64)"],
            ),
            g(
                "Computer:",
                &["reprise-vm-1", "QEMU Standard PC (Q35 + ICH9, 2009)"],
            ),
            g(
                "Processor:",
                &[
                    "Example CPU @ 2.00GHz",
                    "4 cores, 8 threads, up to 3.40 GHz",
                ],
            ),
            g("Graphics:", &["Red Hat Virtio 1.0 GPU"]),
            g("Memory:", &["16,777,216 KB", "9,542,041 KB available"]),
            g("Disk (/):", &["500.0 GB (120.0 GB free)"]),
            g("Display:", &["1920x1080 at 60 Hz"]),
            g("Session:", &["Jo Doe (jo)", "Up 3 hours, 12 minutes"]),
        ],
    };
    let mut themes = vec![Theme::default().with_text_size(pct)];
    if with_blue {
        themes.push(interfaces::theme_for(Interface::Blue).with_text_size(pct));
        themes.push(interfaces::theme_for(Interface::Glass).with_text_size(pct));
    }
    let (w, h) = themes[0].text.size(aboutbox::system_size(&system));
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let built = aboutbox::system_form(&system, &text, theme.text.base_px(px), theme.text);
        let mut box_c = Canvas::new(w as u32, h as u32);
        box_c.fill_rect(box_c.bounds(), theme.colors.button_face);
        form::draw(&mut box_c, &text, &theme.paint(), &built, true, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// The Open dialog enlarged, in Details, sorted by Size twice: files first, largest first, then folders Z to A.
fn render_file_dialog() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let mut d = filedlg::FileDlg::new(
        filedlg::Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: filedlg::DlgMode::Open,
            multiple: false,
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        },
        vec![],
    );
    let entry = |id: u64, name: &str, is_dir: bool, size: u64| shellview::Entry {
        id,
        name: name.into(),
        is_dir,
        size,
        kind: if is_dir {
            "File Folder"
        } else {
            "Text Document"
        }
        .into(),
        modified: "9/12/2026 10:13 AM".into(),
        ..Default::default()
    };
    d.set_listing(filedlg::Folder {
        id: 1,
        parent: None,
        crumbs: vec![filedlg::Crumb {
            id: 1,
            label: "Documents".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![
            entry(2, "Letters", true, 0),
            entry(3, "notes.txt", false, 1_024),
            entry(4, "Reports", true, 0),
            entry(5, "setup.log", false, 114_688),
            entry(6, "todo.txt", false, 32_768),
        ],
    });
    d.set_size((filedlg::CLIENT.0 + 168, filedlg::CLIENT.1 + 55));
    let (w, h) = d.client();
    let r = Rect::new(0, 0, w, h);
    d.click(r, Some(filedlg::Hit::ToolBar(3)), Default::default());
    d.click(r, Some(filedlg::Hit::ViewsItem(4)), Default::default());
    let size = Some(filedlg::Hit::Shell(shellview::Hit::Header(1)));
    for _ in 0..2 {
        d.click(r, size, Default::default());
        d.release();
    }
    let mut dc = Canvas::new(w as u32, h as u32);
    d.draw(&mut dc, &text, &theme.paint(), r, theme.fonts.size_px);
    let mut c = Canvas::new(w as u32 + 20, h as u32 + 20);
    c.fill_rect(c.bounds(), theme.colors.background);
    c.blit_canvas(10, 10, &dc);
    c
}

/// One picture composed Center, Tile and Stretch on the monitor, then the Appearance sample of `sample_theme` per interface.
fn render_backgrounds() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(1090, 396);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    // Red left half, white right half, a black band along the top.
    let mut pic = Canvas::new(64, 48);
    pic.fill_rect(pic.bounds(), Color::rgb(255, 255, 255));
    pic.fill_rect(Rect::new(0, 0, 32, 48), Color::rgb(200, 30, 30));
    pic.fill_rect(Rect::new(0, 0, 64, 4), Color::rgb(0, 0, 0));
    let teal = Color::rgb(0, 128, 128);
    let (_, _, sw, sh) = reprisekit::classic::assets::MONITOR_SCREEN;
    let mut items = Vec::new();
    let monitors = [
        ("center", Position::Center),
        ("tile", Position::Tile),
        ("stretch", Position::Stretch),
    ];
    for (i, (id, pos)) in monitors.into_iter().enumerate() {
        let screen = wallpaper::compose(&pic, sw as u32, sh as u32, pos, teal, 0.5);
        items.push(Item::new(
            id,
            Rect::new(10 + 211 * i as i32, 10, 191, 168),
            Control::Monitor(form::Screen::Picture(screen)),
        ));
    }
    let ruby = reprisekit::glass::Settings {
        color: reprisekit::glass::swatch("ruby").expect("ruby is a swatch"),
        ..reprisekit::glass::Settings::default()
    };
    let samples = [
        (
            "classic",
            interfaces::sample_theme(Interface::Classic, "Brick", &ruby, None),
        ),
        (
            "blue",
            interfaces::sample_theme(Interface::Blue, "Olive Green", &ruby, None),
        ),
        (
            "glass",
            interfaces::sample_theme(Interface::Glass, "", &ruby, Some(teal)),
        ),
    ];
    for (i, (id, t)) in samples.into_iter().enumerate() {
        items.push(Item::new(
            id,
            Rect::new(10 + 360 * i as i32, 198, 350, 188),
            Control::Sample {
                themes: false,
                theme: t,
                icon: None,
            },
        ));
    }
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form::Form::new(items),
        false,
        px,
    );
    c
}

/// The Screen Saver page's monitor with each saver at seed 7 after a second.
fn render_savers() -> Canvas {
    use reprisekit::saver::{Kind, Options, Saver};
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let (_, _, sw, sh) = reprisekit::classic::assets::MONITOR_SCREEN;
    let mut c = Canvas::new(10 + 211 * 4, 188);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let mut pic = Canvas::new(64, 48);
    pic.fill_rect(pic.bounds(), Color::rgb(255, 255, 255));
    pic.fill_rect(Rect::new(0, 0, 32, 48), Color::rgb(200, 30, 30));
    let mut items = Vec::new();
    let kinds = [Kind::Starfield, Kind::Mystify, Kind::Logo, Kind::Photos];
    for (i, kind) in kinds.into_iter().enumerate() {
        let mut saver = Saver::new(kind, &Options::default(), sw as u32, sh as u32, 7, &text);
        if saver.take_photo_request() {
            saver.push_photo(reprisekit::saver::fit(&pic, sw as u32, sh as u32));
        }
        for _ in 0..15 {
            saver.step();
        }
        let mut screen = Canvas::new(sw as u32, sh as u32);
        saver.draw(&mut screen);
        items.push(Item::new(
            kind.name(),
            Rect::new(10 + 211 * i as i32, 10, 191, 168),
            Control::Monitor(form::Screen::Picture(screen)),
        ));
    }
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form::Form::new(items),
        false,
        theme.fonts.size_px,
    );
    c
}

/// The right end of a 320 px bar with the Show desktop strip in each look, at rest and under the pointer.
fn render_show_desktop() -> Canvas {
    use reprisekit::taskbar::{TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(670, 140);
    let mut y = 10;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = interfaces::theme_for(i);
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let shape = theme.taskbar_shape();
        let mut layout = TaskbarLayout::compute(
            &theme.metrics,
            &shape,
            320,
            true,
            counts,
            text.measure("3:45 PM", false, theme.fonts.size_px),
        );
        // Glass combines its buttons by default: an icon-only row.
        if i == Interface::Glass {
            layout.lay_tasks(&theme.metrics, &shape, &[], TaskRow::Icons);
        }
        for (x, hot) in [(10, false), (340, true)] {
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: None,
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                show_desktop_hot: hot,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(320, layout.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
            c.blit_canvas(x, y, &bar);
        }
        y += layout.height + 8;
    }
    c
}

/// A 640 px bar along the top in Classic, Blue, Glass and Glass's small bar over the stripes; the orb's buffer goes over each Glass bar.
fn render_taskbar_top() -> Canvas {
    use reprisekit::taskbar::{
        Edge, PagerButton, TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = interfaces::theme_for(Interface::Glass);
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        interfaces::theme_for(Interface::Classic),
        interfaces::theme_for(Interface::Blue),
        interfaces::theme_for(Interface::Glass),
        small,
    ];
    let mut c = blurred_stripes(660, 200);
    let mut y = 10;
    for (i, theme) in themes.iter().enumerate() {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let shape = theme.taskbar_shape();
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &shape,
            640,
            true,
            counts,
            text.measure(clock, false, theme.fonts.size_px),
        )
        .on_edge(Edge::Top);
        let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
            title: title.into(),
            active,
            attention: false,
            icon: Some(icon.into()),
            glow: None,
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: i == 2,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![
                PagerButton::default(),
                PagerButton {
                    current: true,
                    attention: false,
                },
            ],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                task("Untitled - Notepad", true, indicator::draw_record(16)),
                task("Command Prompt", false, indicator::draw_microphone(16)),
            ],
            tray_icons: vec![Some(indicator::draw_record(16).into())],
            clock: clock.into(),
        };
        let mut bar = Canvas::new(640, layout.height as u32);
        look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
        c.blit_over(10, y, &bar);
        if let Some(r) = layout.start_orb(false) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, theme, &layout, &state);
            c.blit_over(10 + r.x, y + r.y, &orb);
        }
        y += layout.start_above.map_or(layout.height, |r| r.bottom()) + 8;
    }
    c
}

/// Each interface's bar along every edge over the stripes: 640 px along the bottom and top, then up the left and right of a 360 px output, each followed by its hidden columns.
fn render_taskbar_edges() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PagerButton, TaskEntry, TaskbarCounts, TaskbarLayout,
        TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = interfaces::theme_for(Interface::Glass);
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        interfaces::theme_for(Interface::Classic),
        interfaces::theme_for(Interface::Blue),
        interfaces::theme_for(Interface::Glass),
        small,
    ];
    let (time, date) = ("3:45 PM", "10/04/2026");
    let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
        title: title.into(),
        active,
        attention: false,
        icon: Some(icon.into()),
        glow: None,
    };
    let state = |quick: usize, two_lines: bool, icon_px: i32| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![None; quick],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        pager: vec![
            PagerButton::default(),
            PagerButton {
                current: true,
                attention: false,
            },
        ],
        pager_hot: None,
        pager_mark: Default::default(),
        task_hot: None,
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
        tasks: vec![
            task("Untitled - Notepad", true, indicator::draw_record(icon_px)),
            task("Command Prompt", false, indicator::draw_microphone(icon_px)),
        ],
        tray_icons: vec![Some(indicator::draw_record(16).into())],
        clock: if two_lines {
            format!("{time}\n{date}")
        } else {
            time.into()
        },
    };
    // Draws `l` in `theme` at `at` of `c` with the orb's buffer over it; returns the bar's canvas.
    let draw =
        |c: &mut Canvas, theme: &Theme, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let start_text = match theme.look.style() {
                Some(s) => TextRenderer::load(&s.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, l, st);
            c.blit_over(at.0, at.1, &bar);
            if let Some(r) = l.start_orb(false) {
                let mut orb = Canvas::new(r.width as u32, r.height as u32);
                look::draw_start_above(&mut orb, theme, l, st);
                c.blit_over(at.0 + r.x, at.1 + r.y, &orb);
            }
            bar
        };
    let clock_of = |theme: &Theme| ClockText {
        time: text.measure(time, false, theme.fonts.size_px),
        date: text.measure(date, false, theme.fonts.size_px),
    };
    let mut c = blurred_stripes(740, 860);
    let mut y = 24;
    for theme in &themes {
        let shape = theme.taskbar_shape();
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for edge in [Edge::Bottom, Edge::Top] {
            let span = BarSpan {
                edge,
                length: 640,
                thickness: theme.metrics.taskbar_height,
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                clock_of(theme),
                Bands::default(),
            );
            draw(
                &mut c,
                theme,
                &l,
                &state(0, l.clock_lines.is_some(), shape.task_icon),
                (10, y),
            );
            y += l.height + 22;
        }
    }
    let top = y + 10;
    let mut x = 10;
    for theme in &themes {
        let shape = theme.taskbar_shape();
        let t = shape.side_default(&theme.metrics);
        let counts = TaskbarCounts {
            quick: 3,
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for edge in [Edge::Left, Edge::Right] {
            let span = BarSpan {
                edge,
                length: 360,
                thickness: t,
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                clock_of(theme),
                Bands::default(),
            );
            let bar = draw(
                &mut c,
                theme,
                &l,
                &state(3, l.clock_lines.is_some(), shape.task_icon),
                (x, top),
            );
            c.blit_over(x + t + 4, top, &bar.copy_rect(l.reveal_line()));
            x += t + 16;
        }
        x += 14;
    }
    c
}

/// The » and ^ cells at rest, lit and with their popup open, on unlocked bars in each look: 520 px bars along the bottom, then columns up the left of a 360 px output.
fn render_taskbar_cells() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PopupCell, TaskEntry, TaskbarCounts, TaskbarLayout,
        TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = interfaces::theme_for(Interface::Glass);
    interfaces::small_bar(&mut small, &Default::default());
    let themes = [
        interfaces::theme_for(Interface::Classic),
        interfaces::theme_for(Interface::Blue),
        interfaces::theme_for(Interface::Glass),
        small,
    ];
    let cells = [
        PopupCell::default(),
        PopupCell {
            hot: true,
            open: false,
            held: false,
        },
        PopupCell {
            hot: false,
            open: true,
            held: false,
        },
    ];
    let state = |tasks: usize, icon_px: i32, cell: PopupCell| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![None; 2],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        pager: vec![],
        pager_hot: None,
        pager_mark: Default::default(),
        task_hot: None,
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        show_desktop_hot: false,
        more: cell,
        chevron: cell,
        tasks: (0..tasks)
            .map(|i| TaskEntry {
                title: "Untitled - Notepad".into(),
                active: i == 0,
                attention: false,
                icon: Some(indicator::draw_record(icon_px).into()),
                glow: None,
            })
            .collect(),
        tray_icons: vec![Some(indicator::draw_record(16).into())],
        clock: "3:45 PM".into(),
    };
    // Draws `l` in `theme` at `at` of `c` with the orb's buffer over it.
    let draw =
        |c: &mut Canvas, theme: &Theme, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let start_text = match theme.look.style() {
                Some(s) => TextRenderer::load(&s.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, l, st);
            c.blit_over(at.0, at.1, &bar);
            if let Some(r) = l.start_orb(false) {
                let mut orb = Canvas::new(r.width as u32, r.height as u32);
                look::draw_start_above(&mut orb, theme, l, st);
                c.blit_over(at.0 + r.x, at.1 + r.y, &orb);
            }
        };
    let lay = |theme: &Theme, span: BarSpan, tasks: usize| {
        let counts = TaskbarCounts {
            quick: 2,
            tasks,
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute_on(
            &theme.metrics,
            &theme.taskbar_shape(),
            span,
            false,
            counts,
            ClockText {
                time: text.measure("3:45 PM", false, theme.fonts.size_px),
                date: 0,
            },
            Bands::default(),
        )
    };
    let mut c = blurred_stripes(960, 1056);
    let mut y = 24;
    for theme in &themes {
        let icon_px = theme.taskbar_shape().task_icon;
        for cell in cells {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 520,
                thickness: theme.metrics.taskbar_height,
            };
            let l = lay(theme, span, 2);
            draw(&mut c, theme, &l, &state(2, icon_px, cell), (10, y));
            y += l.height + 22;
        }
    }
    let top = y + 10;
    let mut x = 10;
    for theme in &themes {
        let icon_px = theme.taskbar_shape().task_icon;
        let t = theme.taskbar_shape().side_default(&theme.metrics);
        for cell in cells {
            let span = BarSpan {
                edge: Edge::Left,
                length: 360,
                thickness: t,
            };
            let l = lay(theme, span, 12);
            draw(&mut c, theme, &l, &state(12, icon_px, cell), (x, top));
            x += t + 10;
        }
        x += 14;
    }
    c
}

/// The tray arrow on Classic and Blue bars with one shown and two hidden icons: along the bottom in each state, then down a left bar.
fn render_tray_arrow() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PopupCell, TaskbarCounts, TaskbarLayout, TaskbarState,
        reveal_width,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let state = |cell: PopupCell, icons: usize| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        pager: vec![],
        pager_hot: None,
        pager_mark: Default::default(),
        task_hot: None,
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        show_desktop_hot: false,
        more: Default::default(),
        chevron: cell,
        tasks: vec![],
        tray_icons: vec![Some(indicator::draw_record(16).into()); icons],
        clock: "3:45 PM".into(),
    };
    let cell = |hot, open, held| PopupCell { hot, open, held };
    let mut c = blurred_stripes(1060, 262);
    let mut x0 = 10;
    for interface in [Interface::Classic, Interface::Blue] {
        let theme = interfaces::theme_for(interface);
        let shape = theme.taskbar_shape();
        let full = reveal_width(&shape, 2);
        let lay = |span: BarSpan, reveal: i32| {
            let counts = TaskbarCounts {
                tray: 1,
                hidden: 2,
                reveal,
                chevron: true,
                ..TaskbarCounts::default()
            };
            TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                ClockText {
                    time: text.measure("3:45 PM", false, theme.fonts.size_px),
                    date: 0,
                },
                Bands::default(),
            )
        };
        let draw = |c: &mut Canvas, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &text, l, st);
            c.blit_over(at.0, at.1, &bar);
        };
        let mut y = 16;
        for (reveal, cell) in [
            (0, cell(false, false, false)),
            (full / 2, cell(false, false, false)),
            (full, cell(false, true, false)),
            (0, cell(true, false, false)),
            (full, cell(true, true, false)),
            (0, cell(true, false, true)),
        ] {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 360,
                thickness: theme.metrics.taskbar_height,
            };
            let l = lay(span, reveal);
            let st = state(cell, l.tray.len());
            draw(&mut c, &l, &st, (x0, y));
            y += l.height + 10;
        }
        let mut x = x0 + 370;
        for (reveal, cell) in [
            (0, cell(false, false, false)),
            (1, cell(false, true, false)),
        ] {
            let span = BarSpan {
                edge: Edge::Left,
                length: 200,
                thickness: 60,
            };
            let l = lay(span, reveal);
            let st = state(cell, l.tray.len());
            draw(&mut c, &l, &st, (x, 16));
            x += 70;
        }
        x0 += 540;
    }
    c
}

/// The Blue tray arrow in every colour scheme, a row each, at `n` device px per design px.
fn render_tray_arrow_schemes(n: i32) -> Canvas {
    use reprisekit::taskbar::{
        PopupCell, TaskbarCounts, TaskbarLayout, TaskbarState, reveal_width,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let cells = [
        PopupCell::default(),
        PopupCell {
            hot: true,
            ..PopupCell::default()
        },
        PopupCell {
            hot: true,
            held: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            hot: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            hot: true,
            held: true,
        },
    ];
    let (w, gap) = (200, 10);
    let cols = cells.len() as i32;
    let mut c = blurred_stripes((cols * (w + gap) + gap) * n, (7 * 40 + gap) * n);
    for (row, v) in reprisekit::blue::Variant::ALL.into_iter().enumerate() {
        let one = interfaces::blue_theme(v);
        let theme = one.scaled(n);
        let shape = theme.taskbar_shape();
        for (col, cell) in cells.into_iter().enumerate() {
            let counts = TaskbarCounts {
                tray: 1,
                hidden: 2,
                reveal: if cell.open {
                    reveal_width(&shape, 2)
                } else {
                    0
                },
                chevron: true,
                ..TaskbarCounts::default()
            };
            let l = TaskbarLayout::compute(
                &one.metrics,
                &shape,
                w,
                true,
                counts,
                text.measure("3:45 PM", false, one.fonts.size_px),
            )
            .scaled(n);
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: None,
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                show_desktop_hot: false,
                more: Default::default(),
                chevron: cell,
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16 * n).into()); l.tray.len()],
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &text, &l, &state);
            let x = gap + col as i32 * (w + gap);
            c.blit_over(x * n, (gap + row as i32 * 40) * n, &bar);
        }
    }
    c
}

/// Glass's Start orb at rest, lit and held on a 120 px bar at scale `n`, on Sky and dark glass.
fn render_orb_glow(n: i32) -> Canvas {
    use reprisekit::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    // Rows are 80 px apart so a lit glow's rows above the bar land on the stripes.
    let (w, gap, row_h) = (120, 10, 80);
    let quads = four_colour_icon(24 * n);
    let looks = [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ];
    let mut c = blurred_stripes((3 * (w + gap) + gap) * n, 4 * row_h * n);
    for (li, mode) in looks.into_iter().enumerate() {
        let one = interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &reprisekit::glass::Settings {
                mode,
                ..reprisekit::glass::Settings::default()
            },
        );
        let theme = one.scaled(n);
        let shape = theme.taskbar_shape();
        for (ii, icon) in [None, Some(quads.clone())].into_iter().enumerate() {
            for (col, (hot, held)) in [(false, false), (true, false), (true, true)]
                .into_iter()
                .enumerate()
            {
                let l = TaskbarLayout::compute(
                    &one.metrics,
                    &shape,
                    w,
                    true,
                    TaskbarCounts::default(),
                    text.measure("3:45 PM", false, one.fonts.size_px),
                )
                .scaled(n);
                let state = TaskbarState {
                    start_pressed: held,
                    start_focus: false,
                    focus: None,
                    start_hot: hot,
                    start_icon: icon.clone().map(Into::into),
                    orb_glow: 100,
                    quick_icons: vec![],
                    quick_hot: None,
                    quick_pressed: None,
                    quick_drop: None,
                    pager: vec![],
                    pager_hot: None,
                    pager_mark: Default::default(),
                    task_hot: None,
                    task_pressed: None,
                    task_hot_x: None,
                    task_shift: Vec::new(),
                    task_float: None,
                    show_desktop_hot: false,
                    more: Default::default(),
                    chevron: Default::default(),
                    tasks: vec![],
                    tray_icons: vec![],
                    clock: "3:45 PM".into(),
                };
                let mut bar = Canvas::new(l.width as u32, l.height as u32);
                look::draw_taskbar(&mut bar, &theme, &text, &text, &l, &state);
                let x = (gap + col as i32 * (w + gap)) * n;
                let y = (40 + (2 * li + ii) as i32 * row_h) * n;
                c.blit_over(x, y, &bar);
                if let Some(r) = l.start_orb(held) {
                    let mut orb = Canvas::new(r.width as u32, r.height as u32);
                    look::draw_start_above(&mut orb, &theme, &l, &state);
                    c.blit_over(x + r.x, y + r.y, &orb);
                }
            }
        }
    }
    c
}

/// A `px` square test icon of four rounded panes: red, green, blue, yellow clockwise from the top left.
fn four_colour_icon(px: i32) -> Canvas {
    let mut c = Canvas::new(px as u32, px as u32);
    let inks = [0xFFE0_3020_u32, 0xFF30_B030, 0xFF20_70E0, 0xFFF0_C020];
    let h = px / 2;
    let gap = (px / 12).max(1);
    for y in 0..px {
        for x in 0..px {
            let (right, down) = (x >= h, y >= h);
            let (lx, ly) = (x % h, y % h);
            if lx < gap / 2 + 1 && right || ly < gap / 2 + 1 && down {
                continue;
            }
            if !right && lx >= h - gap / 2 || !down && ly >= h - gap / 2 {
                continue;
            }
            let i = match (right, down) {
                (false, false) => 0,
                (true, false) => 1,
                (true, true) => 3,
                (false, true) => 2,
            };
            c.put_raw(x, y, inks[i]);
        }
    }
    c
}

/// The taskbar with keyboard focus rings (Super+T) in Classic, Blue and Glass.
fn render_taskbar_keys() -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarHit, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let spots = [
        TaskbarHit::Start,
        TaskbarHit::Quick(1),
        TaskbarHit::Task(0),
        TaskbarHit::Chevron,
        TaskbarHit::Tray(0),
        TaskbarHit::Clock,
    ];
    let mut c = blurred_stripes(1080, 580);
    let mut y = 20;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = interfaces::theme_for(i);
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            quick: 2,
            tasks: 2,
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &theme.taskbar_shape(),
            520,
            true,
            counts,
            text.measure("3:45 PM", false, theme.fonts.size_px),
        );
        let icon_px = theme.taskbar_shape().task_icon;
        for (n, spot) in spots.into_iter().enumerate() {
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: Some(spot),
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![None; 2],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: (0..2)
                    .map(|t| TaskEntry {
                        title: "Untitled - Notepad".into(),
                        active: t == 0,
                        attention: false,
                        icon: Some(indicator::draw_record(icon_px).into()),
                        glow: None,
                    })
                    .collect(),
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(layout.width as u32, layout.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
            let x = 10 + (n as i32 % 2) * 540;
            c.blit_over(x, y + (n as i32 / 2) * (layout.height + 12), &bar);
        }
        y += 3 * (layout.height + 12) + 20;
    }
    c
}

/// The line an auto-hidden bar leaves on screen in each look, along the bottom then the top.
fn render_taskbar_hidden() -> Canvas {
    use reprisekit::taskbar::{Edge, TaskbarCounts, TaskbarLayout, TaskbarState};
    const BAND: i32 = 16;
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = interfaces::theme_for(Interface::Glass);
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        interfaces::theme_for(Interface::Classic),
        interfaces::theme_for(Interface::Blue),
        interfaces::theme_for(Interface::Glass),
        small,
    ];
    let mut c = blurred_stripes(660, 20 + 8 * (BAND + 6));
    let mut y = 10;
    for theme in &themes {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        for edge in [Edge::Bottom, Edge::Top] {
            let counts = TaskbarCounts {
                tray: 1,
                ..TaskbarCounts::default()
            };
            let clock = "3:45 PM";
            let shape = theme.taskbar_shape();
            let layout = TaskbarLayout::compute(
                &theme.metrics,
                &shape,
                640,
                true,
                counts,
                text.measure(clock, false, theme.fonts.size_px),
            )
            .on_edge(edge);
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: None,
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                clock: clock.into(),
            };
            let mut bar = Canvas::new(640, layout.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
            let line = bar.copy_rect(layout.reveal_line());
            let at = if edge == Edge::Bottom {
                y + BAND - line.height as i32
            } else {
                y
            };
            c.blit_over(10, at, &line);
            y += BAND + 6;
        }
    }
    c
}

/// `name` as a file-name part: ASCII alphanumerics lowercased, the rest '-'.
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The 2 px outline an edge drag shows where the bar would land, on each edge of a small output.
fn render_taskbar_drag() -> Canvas {
    use reprisekit::taskbar::Edge;
    let (w, h, gap) = (200, 130, 10);
    let mut c = Canvas::new((gap + 4 * (w + gap)) as u32, (h + 2 * gap) as u32);
    c.fill_rect(
        Rect::new(0, 0, c.width as i32, c.height as i32),
        Color::rgb(255, 255, 255),
    );
    for (i, edge) in Edge::ALL.into_iter().enumerate() {
        let (x, y) = (gap + i as i32 * (w + gap), gap);
        c.fill_rect(Rect::new(x, y, w, h), Color::rgb(58, 110, 165));
        let bar = match edge {
            Edge::Bottom => Rect::new(0, h - 28, w, 28),
            Edge::Top => Rect::new(0, 0, w, 28),
            Edge::Left => Rect::new(0, 0, 60, h),
            Edge::Right => Rect::new(w - 60, 0, 60, h),
        };
        c.blit_canvas(
            x + bar.x,
            y + bar.y,
            &snapframe::draw(bar.width, bar.height, snapframe::BAR_BAND, 1),
        );
    }
    c
}

/// Every control on one canvas in `theme`'s colours; `with_blue` adds the Blue bands.
fn render(theme: &Theme, with_blue: bool) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    // Classic mode keeps the shorter canvas: skipping the Blue band must leave the rest where it was.
    let mut c = Canvas::new(950, if with_blue { 14229 } else { 13151 });
    c.fill_rect(Rect::new(0, 0, 950, 1660), cs.background);

    let dlg = Rect::new(10, 10, 344, 176);
    c.fill_rect(dlg, cs.button_face);
    c.edge_t(dlg, EdgeStyle::WindowRaised, cs, s);
    let cap = Rect::new(dlg.x + 3, dlg.y + 3, dlg.width - 6, 18);
    let close = Rect::new(cap.right() - 2 - CLOSE_W, cap.y + 2, CLOSE_W, CLOSE_H);
    let help = Rect::new(close.x - 2 - CLOSE_W, close.y, CLOSE_W, CLOSE_H);
    draw_caption(
        &mut c,
        &text,
        cs,
        cap,
        "Run",
        true,
        &[(close, Glyph::Close), (help, Glyph::Help)],
        px,
        s,
    );
    let idle = Rect::new(dlg.x + 3, 340, dlg.width - 6, 18);
    let idle_close = Rect::new(idle.right() - 2 - CLOSE_W, idle.y + 2, CLOSE_W, CLOSE_H);
    draw_caption(
        &mut c,
        &text,
        cs,
        idle,
        "Reprise Settings",
        false,
        &[
            (idle_close, Glyph::Close),
            (
                Rect::new(idle_close.x - 2 - CLOSE_W, idle_close.y, CLOSE_W, CLOSE_H),
                Glyph::Help,
            ),
        ],
        px,
        s,
    );
    let field = Rect::new(dlg.x + 54, dlg.y + 82, 275, ROW_H);
    let inner = draw_sunken_field(
        &mut c,
        &Paint { s, ..theme.paint() },
        field,
        theme.colors.window,
    );
    let mut edit = TextEdit::new("notepad readme.txt", false);
    edit.place(7, false);
    edit.place(0, true);
    draw_edit(
        &mut c,
        &text,
        &edit,
        inner,
        px,
        &Paint { s, ..theme.paint() },
        Caret::Shown,
        0,
    );
    draw_label(
        &mut c,
        &text,
        Rect::new(dlg.x + 11, field.y, 40, ROW_H),
        "&Open:",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    let by = dlg.bottom() - 17 - BUTTON_H;
    for (i, (label, default, enabled, pushed)) in [
        ("OK", true, true, false),
        ("Cancel", false, true, true),
        ("&Browse...", false, false, false),
    ]
    .iter()
    .enumerate()
    {
        let r = Rect::new(
            dlg.x + 86 + i as i32 * (BUTTON_W + BUTTON_GAP),
            by,
            BUTTON_W,
            BUTTON_H,
        );
        draw_button(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            r,
            label,
            *default,
            Interaction::pressed_if(*pushed),
            *enabled,
            *default,
            px,
        );
    }
    let cb = Rect::new(dlg.x + 54, dlg.y + 115, 13, 13);
    draw_checkbox(
        &mut c,
        &Paint { s, ..theme.paint() },
        cb,
        true,
        Interaction::Idle,
    );
    let mixed = Rect::new(cb.x, cb.y + 18, 13, 13);
    draw_checkbox_state(
        &mut c,
        &Paint { s, ..theme.paint() },
        mixed,
        CheckState::Mixed,
        Interaction::Idle,
    );
    draw_label(
        &mut c,
        &text,
        Rect::new(mixed.right() + 5, mixed.y, 200, 13),
        "&Read-only",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    draw_label(
        &mut c,
        &text,
        Rect::new(cb.right() + 5, cb.y, 200, 13),
        "&Always ask",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    draw_scrollbar(
        &mut c,
        &Paint { s, ..theme.paint() },
        Rect::new(dlg.x + 300, dlg.y + 110, theme.look.controls().scroll, 40),
        1,
        3,
        9,
        None,
    );

    let tiles = MenuItem::item("Ti&les").checked(true).radio(true);
    let items = [
        MenuItem::item("&Open"),
        MenuItem::item("Open &With"),
        MenuItem::Separator,
        MenuItem::item("Cu&t"),
        MenuItem::item("&Copy"),
        MenuItem::Separator,
        tiles,
        MenuItem::item("P&roperties"),
    ];
    let mut tree = MenuTree::new();
    for (i, item) in items.iter().enumerate() {
        if i == 1 {
            tree.push_sub(
                item.clone(),
                MenuTree::flat(vec![
                    MenuItem::item("&Notepad"),
                    MenuItem::item("&Browser"),
                    MenuItem::Separator,
                    MenuItem::item("C&hoose Program..."),
                ]),
            );
        } else {
            tree.push(item.clone());
        }
    }
    let cx = MenuCtx {
        text: &text,
        theme,
        px,
        within: c.bounds(),
    };
    let mut chain = PopupMenu::open(&tree, (370, 10), &cx, true);
    chain.key(&tree, MenuKey::Down, &cx);
    chain.key(&tree, MenuKey::Right, &cx);
    chain.draw(&mut c, theme, &text, true);

    c.blit_canvas(
        370,
        160,
        &tooltip::draw_tip(theme, &text, "Click here to begin"),
    );
    let balloon = tooltip::draw_balloon_tip(
        theme,
        &text,
        "Turn Off",
        "Ends your session and shuts down the computer so that you can safely turn off power.",
    );
    c.blit_canvas(10, 200, &balloon);

    let cell = |name: &str, sel: Option<Selection>| {
        iconview::draw_cell(theme, &text, name, None, true, sel, None, None)
    };
    c.blit_over(370, 260, &cell("Documents", None));
    c.blit_over(
        445,
        260,
        &cell(
            "New Text Document",
            Some(Selection {
                ring: true,
                origin: (445, 260),
            }),
        ),
    );

    c.fill_rect(Rect::new(540, 260, 44, 20), cs.button_face);
    c.blit_canvas(542, 262, &indicator::draw_record(16));
    c.blit_canvas(564, 262, &indicator::draw_microphone(16));
    {
        use reprisekit::art::powericon::{Source, draw};
        let b = |percent, charging, low, critical| Source::Battery {
            percent,
            charging,
            low,
            critical,
        };
        let icons = [
            Source::Ac,
            b(100, false, false, false),
            b(50, false, false, false),
            b(8, false, true, false),
            b(2, false, true, true),
            b(72, true, false, false),
        ];
        for (i, src) in icons.iter().enumerate() {
            c.blit_canvas(586 + 20 * i as i32, 262, &draw(*src));
        }
    }
    c.fill_rect(Rect::new(706, 260, 44, 20), cs.button_face);
    c.blit_canvas(708, 262, &indicator::draw_camera(16));
    c.blit_canvas(730, 262, &indicator::draw_location(16));

    let m = multiedit::Metrics {
        bar: theme.look.controls().scroll,
        thumb_min: theme.look.controls().thumb_min,
        ..Default::default()
    };
    let mono = TextRenderer::load(&FontConfig {
        family: vec!["Liberation Mono".into(), "DejaVu Sans Mono".into()],
        size_px: 13,
        antialias: false,
        italic: false,
        full_hinting: false,
        shell_dlg: None,
    });
    let body = (0..12)
        .map(|i| format!("Line {i}: the quick brown fox jumps over the lazy dog"))
        .collect::<Vec<_>>()
        .join("\n");
    let edit = multiedit::MultiEdit::new(&body);
    let r = Rect::new(10, 400, 500, 90);
    let lay = multiedit::layout(&mono, &m, &edit, r, s);
    let view = multiedit::View {
        top: 2,
        ..Default::default()
    };
    multiedit::draw(
        &mut c,
        &mono,
        &Paint { s, ..theme.paint() },
        &m,
        &edit,
        view,
        &lay,
        Caret::Unfocused,
    );
    if let Some(corner) = lay.corner() {
        draw_scroll_corner(&mut c, &Paint { s, ..theme.paint() }, corner, true);
    }

    let page = Rect::new(530, 30, 360, 460);
    draw_page(&mut c, &Paint { s, ..theme.paint() }, page);
    let mut strip = tabstrip::TabStrip::new(&["Themes", "Desktop", "Screen Saver"], 1);
    strip.focused = true;
    let tabs = tabstrip::layout(
        &text,
        px,
        &strip,
        page.x,
        page.y - 20,
        page.width,
        reprisekit::theme::TextUnits::ONE,
    );
    tabstrip::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &strip,
        &tabs,
        page,
        true,
        px,
    );
    let mut open = combo::Combo::new(&["Fade effect", "Scroll effect"], 0, 8);
    open.toggle();
    let mut greyed = trackbar::TrackBar::new(13, 18);
    greyed.enabled = false;
    let items = vec![
        Item::new(
            "l",
            Rect::new(545, 45, 100, 13),
            Control::Label {
                text: "&Position:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            "combo",
            Rect::new(545, 60, 150, combo::COMBO_H),
            Control::Combo(combo::Combo::new(&["Center", "Tile", "Stretch"], 2, 8)),
        ),
        Item::new(
            "open",
            Rect::new(710, 60, 150, combo::COMBO_H),
            Control::Combo(open),
        ),
        Item::new(
            "list",
            Rect::new(545, 130, 200, 80),
            Control::ListBox {
                list: listbox::ListBox::new(
                    ["(None)", "Crystal", "Dunes", "Harbour", "Meadow", "Tide"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    Some(1),
                ),
                icons: Vec::new(),
            },
        ),
        Item::new(
            "tree",
            Rect::new(545, 404, 172, 84),
            Control::Tree({
                let mut screens = treeview::Node::new("Screens:", true);
                screens.expanded = true;
                screens.children = vec![treeview::Node::new("HDMI-A-1 (1920x1080)", false)];
                let mut windows = treeview::Node::new("Windows:", true);
                windows.expanded = true;
                windows.children = vec![treeview::Node::new("Untitled - Notepad", false)];
                let mut tv =
                    treeview::TreeView::new(vec![screens, windows], treeview::Style::Plain);
                tv.selected = Some(vec![1, 0]);
                tv
            }),
        ),
        Item::new(
            "spin",
            Rect::new(760, 130, 50, 20),
            Control::UpDown(updown::UpDown::new(10, 1, 9999)),
        ),
        Item::new(
            "check",
            Rect::new(760, 160, 13, 13),
            Control::Check {
                label: "On &resume, password protect".into(),
                checked: true,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "find",
            Rect::new(545, 100, 150, 20),
            Control::Edit {
                edit: TextEdit::new("Ln 1, Col 1", true),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            "group",
            Rect::new(545, 230, 172, 64),
            Control::Group("Screen resolution".into()),
        ),
        Item::new(
            "track",
            Rect::new(590, 243, 71, trackbar::HEIGHT),
            Control::TrackBar(greyed),
        ),
        Item::new("rule", Rect::new(725, 243, 2, 16), Control::Rule),
        Item::new(
            "color",
            Rect::new(760, 230, 75, 21),
            Control::Color(colorbutton::ColorButton::new(
                cs.background,
                schemes::PALETTE.to_vec(),
            )),
        ),
        Item::radio("here", 370, 200, "&This folder only", "scope"),
        Item::radio("deep", 370, 223, "&Subfolders too", "scope").checked(true),
        Item::radio("none", 370, 246, "&Nowhere (disabled)", "scope").enabled(false),
        Item::new(
            "apply",
            Rect::new(800, 455, 75, 23),
            Control::Button {
                label: "&Apply".into(),
                enabled: false,
                default: false,
            },
        ),
    ];
    let mut form = form::Form::new(items);
    form.focus_id("combo");
    let drop_x = 545 + 3 + text.dialog(form.font).measure("Ln 1,", false, px);
    form.drag_over(drop_x, 110, &text, px);
    form::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() }.on_page(page),
        &form,
        true,
        px,
    );
    let sample = sample::draw_appearance(theme, &text, None, sample::APPEARANCE_SIZE);
    let small = sample.resample_box(173, 92);
    c.blit_canvas(545, 310, &small);
    let monitor = form::Form::new(vec![
        Item::new("spectrum", Rect::new(745, 268, 154, 15), Control::Spectrum),
        Item::new(
            "mon",
            Rect::new(730, 290, 191, 168),
            Control::Monitor(form::Screen::Color(cs.background)),
        ),
    ]);
    form::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &monitor,
        true,
        px,
    );

    let about = aboutbox::Fields {
        product: "Reprise".into(),
        version: "Version 0.1.0 (Build a1b2c3d : 6.9.0-generic)".into(),
        copyright: "".into(),
        facts: [
            "Example CPU @ 2.00GHz".into(),
            "Video:  amdgpu".into(),
            "Free space on /:  38.2 GB".into(),
        ],
        memory_label: "Physical memory available to Reprise:".into(),
        memory_value: "16,000,000 KB".into(),
        icon: None,
    };
    let about_form = aboutbox::form(&about, &text, px);
    let (aw, ah) = aboutbox::CLIENT;
    let mut about_c = Canvas::new(aw as u32, ah as u32);
    about_c.fill_rect(about_c.bounds(), cs.button_face);
    form::draw(
        &mut about_c,
        &text,
        &Paint { s, ..theme.paint() },
        &about_form,
        true,
        px,
    );
    c.blit_canvas(10, 510, &about_c);

    let places = placesbar::PlacesBar {
        places: [
            "My Recent Documents",
            "user's Home",
            "Desktop",
            "Documents",
            "Downloads",
        ]
        .iter()
        .map(|l| placesbar::Place {
            label: (*l).into(),
            icon: None,
            enabled: true,
        })
        .collect(),
        selected: Some(3),
        ..Default::default()
    };
    let places_r = Rect::new(700, 510, placesbar::WIDTH, 5 * placesbar::TILE_H);
    placesbar::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &places,
        places_r,
        px,
    );

    let find = finddlg::FindFields {
        query: "hello".into(),
        match_case: false,
        down: true,
    };
    let (fw, fh) = finddlg::FIND_CLIENT;
    let mut find_c = Canvas::new(fw as u32, fh as u32);
    find_c.fill_rect(find_c.bounds(), cs.button_face);
    let find_form = finddlg::find_form(&find);
    form::draw(
        &mut find_c,
        &text,
        &Paint { s, ..theme.paint() },
        &find_form,
        true,
        px,
    );
    c.blit_canvas(10, 870, &find_c);

    let replace = finddlg::ReplaceFields {
        query: "hello".into(),
        with: "world".into(),
        match_case: false,
    };
    let (rw, rh) = finddlg::REPLACE_CLIENT;
    let mut replace_c = Canvas::new(rw as u32, rh as u32);
    replace_c.fill_rect(replace_c.bounds(), cs.button_face);
    let replace_form = finddlg::replace_form(&replace);
    form::draw(
        &mut replace_c,
        &text,
        &Paint { s, ..theme.paint() },
        &replace_form,
        true,
        px,
    );
    c.blit_canvas(400, 870, &replace_c);

    let notice = msgbox::Fields {
        text: "Cannot find \"hello\"".into(),
        icon: None,
        buttons: vec!["OK".into()],
        default: 0,
        min_text_h: None,
        min_w: 0,
    };
    let (mw, mh) = msgbox::client(&notice, &text, px, theme.text);
    let mut msg_c = Canvas::new(mw as u32, mh as u32);
    msg_c.fill_rect(msg_c.bounds(), cs.button_face);
    let msg_form = msgbox::form(&notice, &text, px, theme.text);
    form::draw(
        &mut msg_c,
        &text,
        &Paint { s, ..theme.paint() },
        &msg_form,
        true,
        px,
    );
    c.blit_canvas(10, 990, &msg_c);

    {
        let lay = dlgtemplate::layout(
            dlgtemplate::Template::MessageBox,
            &text,
            px,
            &dlgtemplate::Fields {
                text: "Are you sure you want to send 'New Rich Text Document' to the Recycle Bin?",
                icon: true,
                field: false,
                buttons: 2,
                text_h: None,
            },
            theme.text,
        )
        .client();
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(4, 2, 24, 28), Color::rgb(80, 193, 255));
        let f = dlgtemplate::form(&lay, Some(icon), &["&Yes".into(), "&No".into()], 0);
        let mut box_c = Canvas::new(lay.size.0 as u32, lay.size.1 as u32);
        box_c.fill_rect(box_c.bounds(), cs.button_face);
        form::draw(
            &mut box_c,
            &text,
            &Paint { s, ..theme.paint() },
            &f,
            true,
            px,
        );
        c.blit_canvas(480, 3018, &box_c);
    }

    let tb = toolbar::ToolBar {
        items: vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Button {
                id: toolbar::NEW_FOLDER,
                icon: None,
                label: None,
                enabled: false,
            },
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw16()),
                label: None,
                enabled: true,
            },
        ],
        hot: Some(0),
        pressed: Some(1),
        latched: None,
        big: false,
    };
    let tb_r = Rect::new(10, 1100, 200, toolbar::HEIGHT);
    let cm = theme.look.controls();
    let tb_rects = toolbar::layout(&tb, &text, px, tb_r, &cm);
    toolbar::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &tb,
        tb_r,
        &tb_rects,
        px,
    );

    let rebar = rebar::Rebar {
        bands: vec![
            rebar::Band { height: 22 },
            rebar::Band {
                height: toolbar::height(true, &cm),
            },
            rebar::Band { height: 22 },
        ],
    };
    let rebar_r = Rect::new(240, 1095, 560, rebar::height(&rebar, &cm.rebar));
    rebar::draw(&mut c, &Paint { s, ..theme.paint() }, &rebar, rebar_r);
    let bands = rebar::layout(&rebar, rebar_r, &cm.rebar);
    let big = toolbar::ToolBar {
        items: vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: Some("Back".into()),
                enabled: true,
            },
            toolbar::Item::DropButton {
                id: toolbar::FORWARD,
                icon: None,
                // Empty, not absent: Forward reserves a label's trailing pad, so it is 46 px, not 39.
                label: Some(String::new()),
                enabled: false,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::Button {
                id: toolbar::FOLDERS,
                icon: None,
                label: Some("Folders".into()),
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw24()),
                label: None,
                enabled: true,
            },
        ],
        hot: None,
        pressed: None,
        latched: Some(4),
        big: true,
    };
    let big_rects = toolbar::layout(&big, &text, px, bands[1], &cm);
    toolbar::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &big,
        bands[1],
        &big_rects,
        px,
    );

    for (i, (mode, grouped)) in [
        (shellview::Mode::List, false),
        (shellview::Mode::Details, false),
        (shellview::Mode::Icons, false),
        (shellview::Mode::Tiles, false),
        (shellview::Mode::Thumbnails, false),
        (shellview::Mode::Tiles, true),
    ]
    .iter()
    .enumerate()
    {
        let sv = shellview::sample(*mode, *grouped);
        let col = (i % 2) as i32;
        let row = (i / 2) as i32;
        let sv_r = Rect::new(10 + col * 440, 1650 + row * 180, 420, 170);
        let sv_lay = shellview::layout(&sv, sv_r, &theme.look.controls());
        shellview::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &sv,
            &sv_lay,
            None,
            px,
        );
    }

    let lv = listview::sample();
    let lv_r = Rect::new(570, 1240, 320, 130);
    let lv_lay = listview::layout(&lv, lv_r, &theme.look.controls());
    listview::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &lv,
        &lv_lay,
        None,
        px,
    );

    let mut filedlg = filedlg::FileDlg::new(
        filedlg::Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: filedlg::DlgMode::Open,
            multiple: false,
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        },
        [
            "My Recent Documents",
            "user's Home",
            "Desktop",
            "Documents",
            "Downloads",
            "Computer",
        ]
        .iter()
        .map(|l| placesbar::Place {
            label: (*l).into(),
            icon: None,
            enabled: true,
        })
        .collect(),
    );
    filedlg.set_listing(filedlg::Folder {
        id: 1,
        parent: None,
        crumbs: vec![filedlg::Crumb {
            id: 1,
            label: "Documents".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![
            shellview::Entry {
                id: 2,
                name: "Pictures".into(),
                is_dir: true,
                kind: "File Folder".into(),
                ..Default::default()
            },
            shellview::Entry {
                id: 3,
                name: "Music".into(),
                is_dir: true,
                kind: "File Folder".into(),
                ..Default::default()
            },
            shellview::Entry {
                id: 4,
                name: "readme.txt".into(),
                size: 1_024,
                kind: "Text Document".into(),
                ..Default::default()
            },
        ],
    });
    filedlg.set_file_name("readme.txt");
    let (dw, dh) = filedlg::CLIENT;
    let mut filedlg_c = Canvas::new(dw as u32, dh as u32);
    filedlg_c.fill_rect(filedlg_c.bounds(), cs.button_face);
    filedlg.draw(
        &mut filedlg_c,
        &text,
        &Paint { s, ..theme.paint() },
        Rect::new(0, 0, dw, dh),
        px,
    );
    c.blit_canvas(10, 1185, &filedlg_c);

    let dialog_in = |mode: filedlg::DlgMode, accept: &str, multiple: bool, folders_only: bool| {
        let mut d = filedlg::FileDlg::new(
            filedlg::Fields {
                title: "Open".into(),
                accept_label: accept.into(),
                filters: if folders_only {
                    vec![]
                } else {
                    vec!["All Files".into()]
                },
                mode,
                multiple,
            },
            vec![],
        );
        let mut entries = vec![shellview::Entry {
            id: 2,
            name: "Pictures".into(),
            is_dir: true,
            kind: "File Folder".into(),
            ..Default::default()
        }];
        if !folders_only {
            for (id, name) in [(3, "a.txt"), (4, "b.txt"), (5, "c.txt")] {
                entries.push(shellview::Entry {
                    id,
                    name: name.into(),
                    kind: "Text Document".into(),
                    ..Default::default()
                });
            }
        }
        d.set_listing(filedlg::Folder {
            id: 1,
            parent: None,
            crumbs: vec![filedlg::Crumb {
                id: 1,
                label: "Documents".into(),
                depth: 0,
                icon: None,
            }],
            entries,
        });
        d
    };
    let draw_dialog = |c: &mut Canvas, d: &filedlg::FileDlg, y: i32| {
        let (w, h) = d.client();
        let mut dc = Canvas::new(w as u32, h as u32);
        dc.fill_rect(dc.bounds(), cs.button_face);
        d.draw(
            &mut dc,
            &text,
            &Paint { s, ..theme.paint() },
            Rect::new(0, 0, w, h),
            px,
        );
        c.blit_canvas(10, y, &dc);
    };
    let mut multi = dialog_in(filedlg::DlgMode::Open, "Open", true, false);
    let r = Rect::new(0, 0, filedlg::CLIENT.0, filedlg::CLIENT.1);
    let ctrl = reprisekit::selection::Mods {
        ctrl: true,
        shift: false,
    };
    for (i, mods) in [(1, Default::default()), (3, ctrl)] {
        multi.click(r, Some(filedlg::Hit::Shell(shellview::Hit::Entry(i))), mods);
    }
    draw_dialog(&mut c, &multi, 7140);
    let folder_dlg = dialog_in(filedlg::DlgMode::Folder, "Select Folder", false, true);
    draw_dialog(&mut c, &folder_dlg, 7600);
    let mut choices = dialog_in(filedlg::DlgMode::Open, "Open", false, false);
    let w = filedlg::field_w(filedlg::CLIENT.0);
    choices.set_extra(
        form::Form::new(vec![
            form::Item::check(
                "extra-check",
                filedlg::FIELD_X,
                filedlg::EXTRA_Y,
                "Open read-only",
            ),
            form::Item::combo(
                "extra-combo",
                filedlg::FIELD_X,
                filedlg::EXTRA_Y + 20,
                w,
                &["UTF-8", "Latin-1"],
                2,
            ),
        ]),
        70,
        &theme.look.controls(),
    );
    draw_dialog(&mut c, &choices, 8060);

    {
        let line = |key: &str, volume, balance, check, checked| mixer::Line {
            key: key.into(),
            title: key.into(),
            volume,
            balance,
            check,
            checked,
            live: true,
        };
        let playback = [
            line("Volume Control", 42, Some(10), mixer::Check::MuteAll, false),
            line("Firefox", 48, Some(6), mixer::Check::Mute, false),
            line("mpv", 24, None, mixer::Check::Mute, true),
        ];
        let mut speaker = Canvas::new(16, 16);
        speaker.fill_rect(Rect::new(4, 4, 6, 8), Color::rgb(64, 64, 64));
        speaker.fill_rect(Rect::new(10, 2, 4, 12), Color::rgb(128, 128, 128));
        let mut m = mixer::Mixer::new(&playback, true);
        m.focus = mixer::Focus::Balance(1);
        m.cues = true;
        m.speaker = Some(speaker.clone());
        let (w, h) = m.size(theme.text);
        let body = Rect::new(600, 7150, w, h);
        let lay = mixer::layout(&m, theme, &text, body);
        mixer::draw(&mut c, theme, &text, &m, &lay);
        let mut recording = [
            line("Microphone", 36, Some(10), mixer::Check::Select, false),
            line("Line In", 18, Some(14), mixer::Check::Select, true),
            line("Monitor", 0, None, mixer::Check::Select, false),
        ];
        recording[2].live = false;
        let mut m = mixer::Mixer::new(&recording, false);
        m.speaker = Some(speaker);
        let (w, h) = m.size(theme.text);
        let body = Rect::new(600, 7422, w, h);
        let lay = mixer::layout(&m, theme, &text, body);
        mixer::draw(&mut c, theme, &text, &m, &lay);
    }

    let swatch = |w: u32, h: u32, colour: Color| {
        let mut i = Canvas::new(w, h);
        i.fill_rect(i.bounds(), colour);
        i
    };
    let folder = Color::rgb(255, 255, 153);
    let places: Vec<combo::ListItem> = [
        ("Desktop", 0),
        ("Documents", 1),
        ("Computer", 1),
        ("Local Disk (/)", 2),
        ("Control Panel", 2),
        ("user's Home", 1),
        ("Recycle Bin", 1),
    ]
    .into_iter()
    .map(|(t, d)| combo::ListItem::shell(t, Some(swatch(16, 16, folder)), d))
    .collect();
    let go = Some(swatch(20, 20, Color::rgb(0, 128, 0)));
    let ab = addressbar::AddressBar::new(places, 1, go);
    for (i, open) in [false, true].into_iter().enumerate() {
        let band = Rect::new(0, 2200 + i as i32 * 32, 900, 22);
        c.fill_rect(
            Rect::new(0, band.y - 2, 900, band.height + 2),
            cs.button_face,
        );
        c.etched_hline_t(0, band.right(), band.y - 2, cs, s);
        let mut ab = ab.clone();
        if open {
            ab.combo.toggle();
        }
        let lay = addressbar::layout(&ab, &text, px, band, &theme.look.controls());
        addressbar::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &ab,
            &lay,
            false,
            Caret::Unfocused,
            px,
        );
        addressbar::draw_list(&mut c, &text, &Paint { s, ..theme.paint() }, &ab, &lay, px);
    }

    let strip = Rect::new(0, 2264, 900, 430);
    c.fill_rect(strip, cs.button_face);
    for (i, (title, hot, greyed)) in [
        ("Folders", false, false),
        ("Folders", true, false),
        ("Search", false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let eb = explorerbar::ExplorerBar {
            title: title.into(),
            width: 200,
            hot_close: hot,
            greyed,
        };
        let r = Rect::new(10, strip.y + 16 + i as i32 * 40, 204, 30);
        c.fill_rect(Rect::new(r.x, r.y + 22, 200, 8), cs.window);
        let lay = explorerbar::layout(&eb, r, &theme.look.controls());
        explorerbar::draw(&mut c, &text, &Paint { s, ..theme.paint() }, &eb, &lay, px);
    }
    let icon = |colour: Color| {
        let mut i = Canvas::new(16, 16);
        i.fill_rect(i.bounds(), colour);
        Some(i)
    };
    let tp = taskpane::TaskPane {
        sections: vec![
            taskpane::Section::links(
                "File and Folder Tasks",
                vec![
                    taskpane::Row::new("Make a new folder", icon(Color::rgb(255, 255, 153))),
                    taskpane::Row::new(
                        "Publish this folder to the Web",
                        icon(Color::rgb(0, 128, 0)),
                    ),
                    taskpane::Row::new("Share this folder", icon(Color::rgb(255, 204, 0))),
                ],
            ),
            taskpane::Section::lines(
                "Details",
                vec![("Documents".into(), true), ("System Folder".into(), false)],
            ),
            taskpane::Section {
                title: "Collapsed Tasks".into(),
                open: false,
                body: taskpane::Body::Links(vec![taskpane::Row::new("Nothing to see", None)]),
            },
        ],
        top: 0,
        hot: Some((0, 0)),
        hot_header: None,
    };
    let pane = Rect::new(240, strip.y + 10, 200, 300);
    let lay = taskpane::layout(&tp, &text, px, pane, &theme.look.controls());
    taskpane::draw(&mut c, &text, &Paint { s, ..theme.paint() }, &tp, &lay, px);
    let tip = infotip::draw(
        theme,
        &text,
        &[
            "Dimensions: 800 x 600".into(),
            "Type: JPEG Image".into(),
            "Size: 27.8 KB".into(),
        ],
        1,
    );
    c.blit_canvas(470, strip.y + 40, &tip);

    let swatch = |color: Color| {
        let mut ic = Canvas::new(16, 16);
        ic.fill_rect(Rect::new(2, 3, 12, 10), color);
        ic
    };
    let folder = swatch(Color::rgb(255, 224, 123));
    let open_folder = swatch(Color::rgb(174, 218, 255));
    let child = |label: &str| Node {
        icon: Some(folder.clone()),
        open_icon: Some(open_folder.clone()),
        ..Node::new(label, true)
    };
    let mut docs = child("Documents");
    docs.children = vec![child("Music"), child("Pictures")];
    docs.expanded = true;
    let mut desktop = child("Desktop");
    desktop.children = vec![
        docs,
        child("Computer with a name long enough to overflow"),
        child("user's Home"),
    ];
    desktop.expanded = true;
    {
        let mut tv = TreeView::new(vec![desktop.clone()], treeview::Style::Explorer);
        tv.selected = Some(vec![0, 0]);
        let eb = explorerbar::ExplorerBar::new("Folders");
        let bar_lay = explorerbar::layout(
            &eb,
            Rect::new(620, strip.y + 10, 204, 130),
            &theme.look.controls(),
        );
        explorerbar::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &eb,
            &bar_lay,
            px,
        );
        let b = bar_lay.body;
        let tree_r = Rect::new(b.x - 1, b.y, b.width + 1, b.height);
        let tree_lay = treeview::layout(&tv, &text, px, tree_r, &theme.look.controls());
        treeview::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &tv,
            &tree_lay,
            None,
            px,
        );
    }
    let mut tv = TreeView::new(vec![desktop], treeview::Style::Classic);
    tv.selected = Some(vec![0, 0]);
    tv.focus_ring = true;
    tv.drop_hilight = Some(4);
    tv.left = 6;
    let tree_r = Rect::new(620, strip.y + 150, 240, 110);
    let tree_lay = treeview::layout(&tv, &text, px, tree_r, &theme.look.controls());
    treeview::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &tv,
        &tree_lay,
        None,
        px,
    );

    let sheet_page = || {
        form::Form::new(vec![
            Item::new(
                "type",
                Rect::new(17, 49, 120, 13),
                Control::Label {
                    text: "Type:".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
            Item::new(
                "ro",
                Rect::new(17, 71, 13, 13),
                Control::Check {
                    label: "&Read-only".into(),
                    checked: true,
                    enabled: true,
                    mixed: None,
                },
            ),
            Item::new(
                "adv",
                Rect::new(247, 67, BUTTON_W, BUTTON_H),
                Control::Button {
                    label: "Ad&vanced...".into(),
                    enabled: true,
                    default: false,
                },
            ),
        ])
    };
    let mut sheet = PropSheet::new(
        &["General", "Sharing", "Customize"],
        vec![sheet_page(), sheet_page(), sheet_page()],
        (361, 424),
    );
    sheet.set_apply_enabled(true);
    let (sw, sh) = sheet.size;
    let mut sheet_c = Canvas::new(sw as u32, sh as u32);
    sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
    propsheet::draw(
        &mut sheet_c,
        &text,
        &Paint { s, ..theme.paint() },
        &sheet,
        true,
        px,
    );
    c.blit_canvas(20, 2570, &sheet_c);

    let seven = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut wide = PropSheet::new(
        &seven,
        seven.iter().map(|_| sheet_page()).collect(),
        (361, 424),
    );
    wide.tabs.open = 1;
    wide.fit_tabs(&text, px);
    let (sw, sh) = wide.size;
    let mut wide_c = Canvas::new(sw as u32, sh as u32);
    wide_c.fill_rect(wide_c.bounds(), cs.button_face);
    propsheet::draw(
        &mut wide_c,
        &text,
        &Paint { s, ..theme.paint() },
        &wide,
        true,
        px,
    );
    c.blit_canvas(420, 2570, &wide_c);

    {
        use reprisekit::art::neticon::{NetIcon, draw, draw32};
        let icons = [
            NetIcon::Wired,
            NetIcon::WiredUnplugged,
            NetIcon::Wireless(3),
            NetIcon::WirelessOff,
            NetIcon::Connecting,
        ];
        c.fill_rect(Rect::new(10, 3005, 410, 62), cs.button_face);
        for (i, icon) in icons.iter().enumerate() {
            let x16 = 16 + 40 * i as i32;
            c.blit_canvas(x16, 3011, &draw(*icon, false));
            c.blit_canvas(x16 + 20, 3011, &draw(*icon, true));
            let x32 = 16 + 76 * i as i32;
            c.blit_canvas(x32, 3031, &draw32(*icon, false));
            c.blit_canvas(x32 + 38, 3031, &draw32(*icon, true));
        }
        for (i, (icon, vpn)) in [(NetIcon::Wired, false), (NetIcon::Wireless(3), true)]
            .into_iter()
            .enumerate()
        {
            let mut badged = draw(icon, vpn);
            reprisekit::art::neticon::badge_limited(&mut badged);
            c.blit_canvas(216 + 20 * i as i32, 3011, &badged);
        }
    }

    {
        c.fill_rect(Rect::new(10, 3075, 410, 62), cs.button_face);
        for n in 0..=5u8 {
            signalbars::draw(&mut c, 16 + 28 * n as i32, 3081, n, true);
        }
        signalbars::draw(&mut c, 16 + 28 * 6, 3081, 5, false);
        let mut secret = TextEdit::secret();
        secret.paste("correcthorse", &[]);
        let mut kept = TextEdit::secret();
        kept.set_cue("Leave empty to keep the saved key");
        let key = form::Form::new(vec![
            Item::new(
                "key",
                Rect::new(16, 3105, 150, 20),
                Control::Edit {
                    edit: secret,
                    first: std::cell::Cell::new(0),
                    enabled: true,
                },
            ),
            Item::new(
                "kept",
                Rect::new(176, 3105, 238, 20),
                Control::Edit {
                    edit: kept,
                    first: std::cell::Cell::new(0),
                    enabled: true,
                },
            ),
        ]);
        form::draw(&mut c, &text, &Paint { s, ..theme.paint() }, &key, true, px);
    }

    let osds = [
        (
            "Volume",
            Some(osd::Meter {
                fraction: 0.45,
                enabled: true,
            }),
        ),
        (
            "Muted",
            Some(osd::Meter {
                fraction: 0.45,
                enabled: false,
            }),
        ),
        (
            "Brightness",
            Some(osd::Meter {
                fraction: 0.8,
                enabled: true,
            }),
        ),
        ("Music Player: Next", None),
    ];
    for (i, (title, meter)) in osds.iter().enumerate() {
        let panel = osd::draw(
            theme,
            &text,
            &osd::Osd {
                title: title.to_string(),
                meter: *meter,
            },
            None,
        );
        c.blit_canvas(
            20 + (i as i32 % 2) * 280,
            3145 + (i as i32 / 2) * 80,
            &panel,
        );
    }

    c.blit_canvas(20, 6190, &osd::draw_lines(theme, &text, &display_lines()));
    c.blit_canvas(
        20,
        6320,
        &displaypanel::draw(theme, &text, &display_panel(0)),
    );

    {
        let l = switcher::SwitcherLayout::compute(3, 1, Rect::new(580, 3150, 330, 107), false);
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(2, 4, 28, 24), Color::rgb(80, 193, 255));
        switcher::draw(
            &mut panel,
            theme,
            &text,
            &l,
            "Documents",
            &[Some(icon.clone()), Some(icon), None],
            1,
            None,
            None,
        );
        c.blit_canvas(l.rect.x, l.rect.y, &panel);
    }

    {
        let mut l = launcher::Launcher::new();
        l.key(launcher::Key::Char(EditKey::Insert("no".into())));
        l.set_rows(vec![launcher::Row {
            name: "Notepad".to_string(),
            icon: None,
        }]);
        l.set_files(vec![launcher::FileRow {
            name: "notes.txt".to_string(),
            folder: "~/Documents".to_string(),
            icon: None,
        }]);
        let panel = launcher::draw(theme, &text, &l, None, true, None);
        c.blit_canvas(20, 3305, &panel);
    }

    {
        use reprisekit::controls::cliphistory::{self, ClipHistory, Row};
        let mut thumb = Canvas::new(16, 16);
        thumb.fill_rect(Rect::new(0, 3, 16, 10), Color::rgb(58, 110, 165));
        let rows = [
            ("cargo build --release", None),
            ("Image, 1920x1080", Some(thumb)),
            ("hello world", None),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (label, icon))| Row {
            id: i as u64,
            label: label.into(),
            icon,
        })
        .collect();
        let m = ClipHistory::new(rows);
        c.blit_canvas(540, 5800, &cliphistory::draw(theme, &text, &m, true, None));
    }

    {
        use reprisekit::controls::emojipicker::{self, EmojiPicker, Key};
        use reprisekit::emoji::{EmojiFont, Entry, catalog};
        let font = EmojiFont::load();
        let recent = ["👍", "🎉"].into_iter().filter_map(Entry::get).collect();
        let mut m = EmojiPicker::new(recent, catalog(font.as_ref()));
        m.key(Key::Down);
        let panel = emojipicker::draw(theme, &text, font.as_ref(), &m, true, None);
        c.blit_canvas(560, 5962, &panel);
    }

    {
        c.fill_rect(Rect::new(10, 3425, 410, 120), cs.button_face);
        c.blit_canvas(20, 3435, &snapframe::draw(200, 100, snapframe::BAND, 1));
    }

    {
        let lay = tooltip::notify_layout(
            theme,
            &text,
            "Updates are ready",
            "Click here to install them.",
            &["Install", "Later"],
            None,
        );
        let pushed = Some(tooltip::NotifyHit::Button(0));
        let reply = tooltip::notify_layout(
            theme,
            &text,
            "Ana",
            "Lunch on Friday?",
            &["Reply"],
            Some(tooltip::NotifyReply {
                placeholder: "Write a reply",
                send: "Send",
            }),
        );
        let typed = reprisekit::textedit::TextEdit::new("Sounds good", false);
        let caret = reprisekit::textedit::Caret::Shown;
        c.fill_rect(Rect::new(0, 3545, 330, 235), cs.button_face);
        c.blit_over(
            10,
            3550,
            &tooltip::draw_notify_balloon(theme, &text, &lay, None, pushed, None, None),
        );
        let open = tooltip::draw_notify_balloon(
            theme,
            &text,
            &reply,
            None,
            None,
            None,
            Some((&typed, caret, None)),
        );
        c.blit_over(10, 3556 + lay.height, &open);
    }

    {
        use reprisekit::controls::grouplist::{GroupList, Row};
        c.fill_rect(Rect::new(340, 3545, 360, 230), cs.button_face);
        let item = |name: &str| Row::Item {
            name: name.into(),
            data: (),
        };
        let mut rows = vec![
            Row::Group("Recommended Programs:".into()),
            item("Notepad"),
            item("Text Editor"),
            Row::Group("Other Programs:".into()),
        ];
        rows.extend((1..=10).map(|i| item(&format!("Program {i}"))));
        let mut list = GroupList::new(rows);
        list.select(2, 11);
        let lay = grouplist::layout(
            &list,
            Rect::new(350, 3555, 338, 210),
            &theme.look.controls(),
        );
        let icon = swatch(Color::rgb(80, 193, 255));
        let icons: Vec<Option<&Canvas>> = (0..list.rows.len()).map(|_| Some(&icon)).collect();
        grouplist::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &list,
            &lay,
            &icons,
            grouplist::Shown::Focused,
            px,
        );
    }

    {
        use reprisekit::controls::openwith::{OpenWith, Program};
        let p = |id: &str, name: &str| Program {
            id: id.into(),
            name: name.into(),
            icon: None,
        };
        let mut ow = OpenWith::new(
            "New Text Document.txt",
            vec![p("notepad", "Notepad"), p("texteditor", "Text Editor")],
            vec![
                p("imageeditor", "Image Editor"),
                p("imageviewer", "Image Viewer"),
            ],
        );
        ow.select_id("texteditor");
        let (x, y) = (350, 8596);
        let lay = ow.layout(&text, px, (x, y), &theme.look.controls());
        ow.draw(
            &mut c,
            &text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &lay,
            None,
            &[],
            reprisekit::cues::Cues::NONE,
            px,
        );
    }

    c.fill_rect(Rect::new(0, 3780, 950, 150), cs.background);
    for (x, chrome, title, active, pressed) in [
        (
            10,
            Chrome::WINDOW,
            "Untitled - Notepad",
            true,
            Some(CaptionButton::Close),
        ),
        (360, Chrome::DIALOG, "Run", false, None),
    ] {
        let g =
            FrameGeometry::layout(&theme.metrics, 300, 100, false, chrome).translated(-x, -3790);
        draw_frame(
            &mut c,
            theme,
            &text,
            &g,
            &FrameState {
                title,
                active,
                maximized: false,
                pressed,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        c.fill_rect(g.client, cs.window);
    }

    if with_blue {
        let blue_theme = interfaces::theme_for(Interface::Blue);
        let style = blue_theme.look.style().unwrap();
        let caption_text = TextRenderer::load(&style.caption_text.font());
        let bcs = &blue_theme.colors;
        let band = Rect::new(0, 9050, 950, 240);
        c.fill_rect(band, bcs.background);
        for (i, (active, pressed)) in [
            (true, None),
            (false, None),
            (true, Some(CaptionButton::Close)),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 10 + i as i32 * 240;
            let y = band.y + 10;
            let g = FrameGeometry::layout(&blue_theme.metrics, 220, 80, false, Chrome::WINDOW)
                .translated(-x, -y);
            look::draw_frame(
                &mut c,
                &blue_theme,
                &text,
                &caption_text,
                &g,
                &FrameState {
                    title: "Notes",
                    active,
                    maximized: false,
                    pressed,
                    glow: Glow::lit(None),
                    icon: None,
                    tabbed: false,
                },
            );
            c.fill_rect(g.client, bcs.window);
        }
        let g = FrameGeometry::layout(&blue_theme.metrics, 320, 60, true, Chrome::WINDOW)
            .translated(-10, -(band.y + 134));
        look::draw_frame(
            &mut c,
            &blue_theme,
            &text,
            &caption_text,
            &g,
            &FrameState {
                title: "Notes",
                active: true,
                maximized: true,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        c.fill_rect(g.client, bcs.window);
        for (i, active) in [true, false].into_iter().enumerate() {
            let x = 350 + i as i32 * 300;
            let dialog = Chrome::DIALOG;
            let g = FrameGeometry::layout(&blue_theme.metrics, 280, 50, false, dialog)
                .translated(-x, -(band.y + 134));
            look::draw_dialog_frame(
                &mut c,
                &blue_theme,
                &text,
                &g,
                &FrameState {
                    title: "Run",
                    active,
                    maximized: false,
                    pressed: None,
                    glow: Glow::lit(active.then_some(CaptionButton::Close)),
                    icon: None,
                    tabbed: false,
                },
                active.then_some(Interaction::Idle),
            );
        }
    }

    c.fill_rect(Rect::new(0, 3930, 950, 678), cs.background);
    {
        use reprisekit::controls::menubar::Bar;
        let mut bar = Bar::new(vec![
            (
                "&File",
                vec![
                    MenuItem::item("&New"),
                    MenuItem::item("&Open..."),
                    MenuItem::item("&Save"),
                    MenuItem::Separator,
                    MenuItem::item("E&xit"),
                ],
            ),
            ("&Edit", vec![MenuItem::item("&Undo")]),
            ("F&ormat", vec![MenuItem::item("&Word Wrap")]),
            ("&View", vec![MenuItem::item("&Status Bar")]),
            ("&Help", vec![MenuItem::item("&About Notepad")]),
        ]);
        let client = Rect::new(10, 3940, 400, 170);
        c.fill_rect(client, cs.window);
        let lay = bar.layout(&text, px, client, &theme.look.controls());
        bar.open_title(0, &lay, &text, theme, px, true);
        bar.underline = true;
        bar.draw(&mut c, &text, theme, &lay);
    }
    {
        let mut b = colorbutton::ColorButton::new(cs.background, schemes::PALETTE.to_vec());
        b.toggle();
        let f = form::Form::new(vec![Item::new(
            "color",
            Rect::new(440, 3940, 75, 21),
            Control::Color(b),
        )]);
        form::draw(&mut c, &text, &Paint { s, ..theme.paint() }, &f, true, px);
    }
    c.blit_canvas(
        580,
        3940,
        &tooltip::draw_info_tip(
            theme,
            &text,
            "Displays the properties of the selected items, such as their size and type.",
        ),
    );

    {
        let facts = filesheet::Facts {
            is_dir: false,
            type_name: "Text Document".into(),
            opens_with: Some("Notepad".into()),
            description: None,
            location: "/home/user/Documents".into(),
            size: "1.00 KB (1,024 bytes)".into(),
            on_disk: "4.00 KB (4,096 bytes)".into(),
            contains: None,
            created: Some("Today, September 23, 2026, 9:41:07 AM".into()),
            modified: "Today, September 23, 2026, 9:41:07 AM".into(),
            accessed: "Today, September 23, 2026, 10:02:15 AM".into(),
            read_only: true,
            executable: false,
            can_set_executable: true,
            hidden: false,
        };
        let (page, height) =
            filesheet::general_page(&filesheet::rows(&facts), "readme.txt", None, None);
        let general = PropSheet::new(&["General"], vec![page], (filesheet::SHEET_W, height));
        let (gw, gh) = general.size;
        let mut general_c = Canvas::new(gw as u32, gh as u32);
        general_c.fill_rect(general_c.bounds(), cs.button_face);
        propsheet::draw(
            &mut general_c,
            &text,
            &Paint { s, ..theme.paint() },
            &general,
            true,
            px,
        );
        c.blit_canvas(10, 4120, &general_c);

        let (aw, ah) = filesheet::ATTR_SIZE;
        let mut attr_c = Canvas::new(aw as u32, ah as u32);
        attr_c.fill_rect(attr_c.bounds(), cs.button_face);
        let attr = filesheet::attr_form(
            "set read-only",
            filesheet::Subject::Folder,
            &text,
            px,
            reprisekit::theme::TextUnits::ONE,
        );
        form::draw(
            &mut attr_c,
            &text,
            &Paint { s, ..theme.paint() },
            &attr,
            true,
            px,
        );
        c.blit_canvas(400, 4120, &attr_c);

        for (x, is_dir) in [(10, true), (400, false)] {
            let subject = filesheet::Subject::single(is_dir);
            let (w, h) = filesheet::advanced_size(subject);
            let mut adv_c = Canvas::new(w as u32, h as u32);
            adv_c.fill_rect(adv_c.bounds(), cs.button_face);
            let robots = is_dir.then_some([
                reprisekit::classic::widgets::CheckState::Mixed,
                reprisekit::classic::widgets::CheckState::On,
            ]);
            let adv = filesheet::advanced_form(
                subject,
                None,
                robots,
                &text,
                px,
                reprisekit::theme::TextUnits::ONE,
            );
            form::draw(
                &mut adv_c,
                &text,
                &Paint { s, ..theme.paint() },
                &adv,
                true,
                px,
            );
            c.blit_canvas(x, 4630, &adv_c);
        }
    }
    c.blit_canvas(400, 4346, &sample::draw_themes(theme, &text, None, None));

    let bar = StatusBar {
        panels: vec![
            Panel {
                text: "4 objects".into(),
                width: 0,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
            Panel {
                text: String::new(),
                width: 76,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
            Panel {
                text: "Computer".into(),
                width: 153,
                pad: statusbar::TEXT_PAD,
                icon: Some(swatch(Color::rgb(80, 193, 255))),
            },
        ],
        grip: true,
    };
    let bar_r = Rect::new(
        0,
        c.height as i32 - statusbar::HEIGHT,
        c.width as i32,
        statusbar::HEIGHT,
    );
    statusbar::draw(
        &mut c,
        &text,
        &Paint { s, ..theme.paint() },
        &bar,
        bar_r,
        px,
    );

    {
        use reprisekit::controls::tilelist::{self, Entry, TILE_H, Tile, TileList};
        let rows = [
            ("Headphones", "Connected, battery 80%", "Connected", false),
            ("Speaker", "Not connected", "", false),
            ("Game controller", "Not connected", "", true),
        ];
        let mut list = TileList::new(TILE_H);
        list.entries = vec![
            Entry::Header("Audio".into()),
            Entry::Row(0),
            Entry::Row(1),
            Entry::Header("Input".into()),
            Entry::Row(2),
        ];
        list.select_row(Some(1));
        let r = Rect::new(10, 4940, 360, 3 * TILE_H + 2 * 31 + 4);
        let lay = tilelist::layout(&list, r, &theme.look.controls());
        tilelist::draw(
            &mut c,
            &text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &list,
            &lay,
            px,
            |c, rr, i, sel| {
                let (title, caption, tag, greyed) = rows[i];
                let tile = Tile {
                    icon: None,
                    title,
                    tag,
                    caption,
                    greyed,
                };
                tilelist::draw_tile(c, &text, cs, rr, &tile, sel, px, lay.text);
            },
        );
    }

    {
        use reprisekit::art::bticon::{draw, draw32};
        c.fill_rect(Rect::new(380, 4940, 140, 50), cs.button_face);
        c.blit_canvas(386, 4946, &draw(true));
        c.blit_canvas(406, 4946, &draw(false));
        c.blit_canvas(430, 4946, &draw32(true));
        c.blit_canvas(466, 4946, &draw32(false));
    }

    {
        use reprisekit::art::devicon::{ALL, draw};
        c.fill_rect(Rect::new(536, 4940, 364, 40), cs.button_face);
        for (i, d) in ALL.iter().enumerate() {
            c.blit_over(540 + 40 * i as i32, 4944, &draw(*d));
        }
    }

    {
        use reprisekit::art::userpic::{ALL, draw};
        c.fill_rect(Rect::new(380, 5000, 456, 64), cs.button_face);
        for (i, p) in ALL.iter().enumerate() {
            c.blit_canvas(388 + 56 * i as i32, 5008, &draw(*p, 48));
        }
    }

    {
        use reprisekit::art::printbadge::{ALL, Badge, apply, draw};
        c.fill_rect(Rect::new(380, 5000, 112, 30), cs.button_face);
        for (i, b) in ALL.iter().enumerate() {
            c.blit_over(384 + 12 * i as i32, 5004, &draw(*b));
        }
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(1, 3, 14, 10), cs.button_shadow);
        for (i, set) in [
            [Badge::Default, Badge::Paused],
            [Badge::Default, Badge::Error],
        ]
        .iter()
        .enumerate()
        {
            c.blit_over(424 + 20 * i as i32, 5004, &apply(&icon, set));
        }
    }

    // Stacked: side by side would clip the second page.
    {
        let exterior = wizard::Wizard::new(
            wizard::Style::Exterior {
                title: "Welcome to the Add Bluetooth Device Wizard".into(),
            },
            vec![],
            true,
            false,
        );
        let interior = wizard::Wizard::new(
            wizard::Style::Interior {
                title: "Searching for devices".into(),
                subtitle: "This may take a moment.".into(),
                icon: None,
            },
            vec![],
            false,
            false,
        );
        let (cw, ch) = wizard::CLIENT;
        let mut ce = Canvas::new(cw as u32, ch as u32);
        wizard::draw(
            &mut ce,
            &text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &exterior,
            true,
            px,
        );
        let mut ci = Canvas::new(cw as u32, ch as u32);
        wizard::draw(
            &mut ci,
            &text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &interior,
            true,
            px,
        );
        c.blit_over(10, 5180, &ce);
        c.blit_over(10, 5180 + ch + 10, &ci);
    }

    {
        let area = Rect::new(0, 0, 420, 240);
        let mut under = Canvas::new(420, 240);
        under.fill_rect(area, cs.background);
        under.fill_rect(Rect::new(40, 30, 300, 170), cs.button_face);
        under.fill_rect(Rect::new(40, 30, 300, 18), cs.active_title);
        let mut m = regionselect::RegionSelect::new((0, 0));
        m.press((80, 60));
        m.motion((280, 160));
        c.blit_canvas(
            520,
            5180,
            &regionselect::draw(theme, &text, &m, area, &under),
        );
    }

    {
        use reprisekit::art::clockface;
        use reprisekit::controls::{calendar, timefield};
        c.fill_rect(Rect::new(520, 5430, 420, 360), cs.button_face);
        let year = updown::UpDown::new(2026, 1970, 2099).with_buddy();
        let yl = updown::layout_for(
            &year,
            Rect::new(620, 5440, 79, 21),
            &theme.look.controls(),
            reprisekit::theme::TextUnits::ONE,
        );
        updown::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &year,
            &yl,
            false,
            None,
            px,
        );
        let mut wait = updown::UpDown::new(10, 1, 9999);
        wait.enter();
        let wl = updown::layout(720, 5440);
        updown::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &wait,
            &wl,
            true,
            None,
            px,
        );
        let mut cal = calendar::Calendar::new(2026, 9, Some(24), 0);
        cal.today = Some((2026, 9, 3));
        let cl = calendar::layout(
            Rect::new(530, 5470, calendar::W, calendar::H),
            &theme.controls(),
        );
        calendar::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &cal,
            &cl,
            true,
            px,
        );
        let face = clockface::Face {
            h: 23,
            m: 41,
            s: 24,
        };
        clockface::draw(
            &mut c,
            &Paint { s, ..theme.paint() },
            Rect::new(710, 5440, 132, 100),
            face,
        );
        let mut tf = timefield::TimeField::new(23, 41, 24, true);
        tf.select(1);
        let tl = timefield::layout(Rect::new(734, 5570, 103, 20), &theme.look.controls());
        timefield::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &tf,
            &tl,
            true,
            None,
            px,
        );
        let hm = timefield::TimeField::hm(21, 0, false);
        let hl = timefield::layout(Rect::new(734, 5596, 70, 20), &theme.look.controls());
        timefield::draw(
            &mut c,
            &text,
            &Paint { s, ..theme.paint() },
            &hm,
            &hl,
            false,
            None,
            px,
        );
        cal.day = None;
        c.blit_canvas(
            530,
            5610,
            &calendar::draw_popup(theme, &text, &cal, calendar::PopupHit::Panel),
        );
    }

    {
        use reprisekit::controls::hotkey::{self, HotKey, Mods};
        let mut held = HotKey::new(None);
        held.key(0xffe3, "Control_L", Mods::default(), true);
        held.key(0xffe9, "Alt_L", Mods::default(), true);
        let mut greyed = HotKey::from_keys("Alt+F4");
        greyed.enabled = false;
        let fields = [
            (HotKey::new(None), false),
            (held, true),
            (HotKey::from_keys("Super+Shift+b"), false),
            (greyed, false),
        ];
        for (k, (hk, focused)) in fields.iter().enumerate() {
            let r = Rect::new(734, 5630 + 26 * k as i32, 180, 20);
            hotkey::draw(
                &mut c,
                &text,
                &Paint { s, ..theme.paint() },
                hk,
                hotkey::layout(r),
                *focused,
                true,
                Interaction::Idle,
                px,
            );
        }
    }

    c.blit_canvas(
        10,
        5930,
        &notifyhistory::draw(theme, &text, &notifyhistory::sample()),
    );
    c.fill_rect(Rect::new(400, 5930, 136, 20), cs.button_face);
    for (i, (unseen, dnd)) in [(0, false), (3, false), (12, false), (0, true)]
        .into_iter()
        .enumerate()
    {
        let bell = indicator::draw_bell(16, unseen, dnd);
        c.blit_canvas(402 + 22 * i as i32, 5932, &bell);
    }
    for (i, label) in ["EN", "HU"].into_iter().enumerate() {
        let sq = indicator::draw_layout(&text, cs, label, 16, theme.fonts.size_px);
        c.blit_canvas(492 + 22 * i as i32, 5932, &sq);
    }
    c.fill_rect(Rect::new(540, 5930, 70, 20), cs.button_face);
    for (i, (pick, held)) in [
        (indicator::MouseButton::Left, false),
        (indicator::MouseButton::Middle, false),
        (indicator::MouseButton::Right, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mouse = indicator::draw_mouse_keys(16, pick, held);
        c.blit_canvas(542 + 22 * i as i32, 5932, &mouse);
    }

    {
        let side = |size: &str| replacebox::Side {
            size: size.into(),
            modified: "modified: Monday, September 21, 2026, 4:05:12 PM".into(),
            icon: None,
        };
        let file = replacebox::Fields {
            name: "notes.txt".into(),
            subject: replacebox::Subject::File {
                existing: Box::new(side("512 bytes")),
                incoming: Box::new(side("1.23 KB")),
            },
            icon: None,
            many: false,
        };
        let folder = replacebox::Fields {
            name: "Reports".into(),
            subject: replacebox::Subject::Folder,
            icon: None,
            many: true,
        };
        let mut y = 6220;
        for fields in [file, folder] {
            let (w, h) = replacebox::client(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
            let mut box_c = Canvas::new(w as u32, h as u32);
            let f = replacebox::form(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
            replacebox::draw(&mut box_c, &text, &Paint { s, ..theme.paint() }, &f, px);
            c.blit_canvas(424, y, &box_c);
            y += h + 10;
        }
    }

    {
        let l = switcher::SwitcherLayout::compute(3, 1, Rect::new(20, 6680, 428, 357), true);
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(2, 4, 28, 24), Color::rgb(80, 193, 255));
        switcher::draw(
            &mut panel,
            theme,
            &text,
            &l,
            "Terminal",
            &[Some(icon.clone()), Some(icon.clone()), None],
            1,
            None,
            Some(&icon),
        );
        c.blit_canvas(l.rect.x, l.rect.y, &panel);

        let l = taskpreview::TaskPreviewLayout::compute(
            taskpreview::CLASSIC,
            2,
            Rect::new(560, 6900, 160, 22),
            Rect::new(480, 6680, 450, 220),
            reprisekit::taskbar::Edge::Bottom,
        );
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let cells = [
            taskpreview::Cell {
                title: "user@reprise: ~",
                icon: None,
                active: true,
                picture: None,
            },
            taskpreview::Cell {
                title: "A window with a title too long for its cell",
                icon: None,
                active: false,
                picture: Some(&icon),
            },
        ];
        taskpreview::draw(&mut panel, theme, &text, &l, &cells, Some(1));
        c.blit_canvas(l.rect.x, l.rect.y, &panel);
    }

    {
        c.set_preedit(Preedit::new("nihao".into(), 2, 2));
        let field = Rect::new(20, 7090, 200, ROW_H);
        let inner = draw_sunken_field(
            &mut c,
            &Paint { s, ..theme.paint() },
            field,
            theme.colors.window,
        );
        let mut edit = TextEdit::new("say  to all", false);
        edit.place(4, false);
        draw_edit(
            &mut c,
            &text,
            &edit,
            inner,
            px,
            &Paint { s, ..theme.paint() },
            Caret::Shown,
            0,
        );
        let m = multiedit::Metrics {
            bars_always: false,
            bar: theme.look.controls().scroll,
            thumb_min: theme.look.controls().thumb_min,
            ..Default::default()
        };
        let mut edit = multiedit::MultiEdit::new("first line\nsay  to all");
        edit.place(multiedit::Pos { line: 1, col: 4 }, false);
        let r = Rect::new(240, 7086, 240, 44);
        let lay = multiedit::layout(&mono, &m, &edit, r, s);
        let view = multiedit::View::default();
        multiedit::draw(
            &mut c,
            &mono,
            &Paint { s, ..theme.paint() },
            &m,
            &edit,
            view,
            &lay,
            Caret::Shown,
        );
        c.set_preedit(None);
    }

    {
        let mut desk = Node::new("Desktop", true);
        desk.expanded = true;
        desk.children = vec![
            Node::new("Documents", true),
            Node::new("Stuff", false),
            Node::new("New Folder", false),
        ];
        let mut tv = TreeView::new(vec![desk], treeview::Style::Picker);
        tv.selected = Some(vec![0, 1]);
        tv.begin_edit(vec![0, 2]);
        let d = browsefolder::BrowseFolder::new(
            "Pick the folder to move 'a.txt' to, then click Move.".into(),
            "Move".into(),
            tv,
        );
        let lay = d.layout(&text, px, &theme.look.controls());
        let (w, h) = browsefolder::CLIENT;
        let mut dc = Canvas::new(w as u32, h as u32);
        d.draw(&mut dc, &text, &Paint { s, ..theme.paint() }, &lay, px);
        c.blit_canvas(10, 8596, &dc);
    }

    {
        let (w, h) = (200, 190);
        let mut under = Canvas::new(w as u32, h as u32);
        under.fill_rect(Rect::new(0, 0, w, h), cs.background);
        under.fill_rect(Rect::new(20, 20, 160, 100), cs.button_face);
        under.fill_rect(Rect::new(20, 20, 160, 18), cs.active_title);
        let still = colorpick::Still {
            logical: (w, h),
            width: w as u32,
            height: h as u32,
            pixels: under.pixels.clone(),
        };
        let mut m = colorpick::ColorPick::default();
        m.motion((22, 36));
        colorpick::draw(
            &mut under,
            &text,
            px,
            &m,
            &still,
            &still.backdrop(w as u32, h as u32),
        );
        c.blit_canvas(745, 8596, &under);
    }

    if with_blue {
        use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
        let blue_theme = interfaces::theme_for(Interface::Blue);
        let style = blue_theme.look.style().unwrap();
        let bcs = &blue_theme.colors;
        let band = Rect::new(0, 9312, 950, 40);
        c.fill_rect(band, bcs.background);
        let start_text = TextRenderer::load(&style.start_text.font());
        let layout = TaskbarLayout::compute(
            &blue_theme.metrics,
            &style.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 3,
                tray: 2,
                show_desktop: false,
                chevron: false,
                ..TaskbarCounts::default()
            },
            text.measure("00:00", false, blue_theme.fonts.size_px),
        );
        let tasks = vec![
            TaskEntry {
                title: "Notepad".into(),
                active: true,
                attention: false,
                icon: None,
                glow: None,
            },
            TaskEntry {
                title: "A window with a title too long for its button".into(),
                active: false,
                attention: false,
                icon: None,
                glow: None,
            },
            TaskEntry {
                title: "Explorer".into(),
                active: false,
                attention: false,
                icon: None,
                glow: None,
            },
        ];
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: Some(1),
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks,
            tray_icons: vec![
                Some(indicator::draw_record(16).into()),
                Some(indicator::draw_microphone(16).into()),
            ],
            clock: "3:45 PM".into(),
        };
        let mut bar = Canvas::new(800, 30);
        look::draw_taskbar(&mut bar, &blue_theme, &text, &start_text, &layout, &state);
        c.blit_canvas(10, band.y + 5, &bar);
    }

    {
        let l = switcher::SwitcherLayout::thumbnails(
            3,
            1,
            Rect::new(20, 9362, 448, 138),
            switcher::CLASSIC,
        );
        let canvas = l.canvas();
        let mut panel = Canvas::new(canvas.width as u32, canvas.height as u32);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(2, 4, 28, 24), Color::rgb(80, 193, 255));
        let pic = |k: usize| Some(taskpreview::fit(400, 300, l.wells[k], 1));
        let cells = [
            switcher::ThumbCell {
                icon: Some(&icon),
                picture: pic(0),
                still: None,
            },
            switcher::ThumbCell {
                icon: Some(&icon),
                picture: pic(1),
                still: None,
            },
            switcher::ThumbCell {
                icon: Some(&icon),
                picture: None,
                still: None,
            },
        ];
        switcher::draw_thumbnails(&mut panel, theme, &text, &l, "Terminal", &cells, 1, Some(2));
        for r in cells.iter().filter_map(|c| c.picture) {
            panel.fill_rect(r, Color::rgb(255, 255, 255));
        }
        let mut badges = Canvas::new(canvas.width as u32, canvas.height as u32);
        switcher::draw_badges(&mut badges, theme, &l, &cells);
        panel.blit_canvas(0, 0, &badges);
        c.blit_canvas(l.rect.x, l.rect.y, &panel);
    }

    {
        let facts = filesheet::BinFacts {
            type_name: "Text Document".into(),
            origin: "Desktop".into(),
            size: "1.00 KB".into(),
            deleted: Some("Today, September 23, 2026, 10:02:15 AM".into()),
            created: Some("Today, September 23, 2026, 9:41:07 AM".into()),
            read_only: false,
            hidden: false,
        };
        let page = filesheet::bin_page(&facts, "readme.txt", None);
        let sheet = PropSheet::new(
            &["General"],
            vec![page],
            (filesheet::SHEET_W, filesheet::BIN_SHEET_H),
        );
        let (w, h) = sheet.size;
        let mut sheet_c = Canvas::new(w as u32, h as u32);
        sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
        propsheet::draw(
            &mut sheet_c,
            &text,
            &Paint { s, ..theme.paint() },
            &sheet,
            true,
            px,
        );
        c.blit_canvas(10, 9823, &sheet_c);
    }

    {
        for (x, has_dir, read_only) in
            [(10, false, CheckState::Mixed), (400, true, CheckState::Off)]
        {
            let facts = filesheet::SetFacts {
                has_dir,
                count: if has_dir {
                    "2 Files, 2 Folders"
                } else {
                    "2 Files, 0 Folders"
                }
                .into(),
                type_text: if has_dir {
                    "Multiple Types"
                } else {
                    "All of type Text Document"
                }
                .into(),
                location: "All in /home/user/Desktop".into(),
                size: "148 bytes (148 bytes)".into(),
                on_disk: "8.00 KB (8,192 bytes)".into(),
                read_only,
                hidden: CheckState::Off,
            };
            let (page, height) =
                filesheet::general_page(&filesheet::set_rows(&facts), "", None, None);
            let sheet = PropSheet::new(&["General"], vec![page], (filesheet::SHEET_W, height));
            let (w, h) = sheet.size;
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
            propsheet::draw(
                &mut sheet_c,
                &text,
                &Paint { s, ..theme.paint() },
                &sheet,
                true,
                px,
            );
            c.blit_canvas(x, 10253, &sheet_c);
        }
    }

    {
        let rows = |is_dir| {
            let facts = filesheet::Facts {
                is_dir,
                type_name: "File Folder".into(),
                location: "/home/user".into(),
                size: "4.00 KB (4,096 bytes)".into(),
                on_disk: "4.00 KB (4,096 bytes)".into(),
                contains: Some("1 Files, 0 Folders".into()),
                ..Default::default()
            };
            filesheet::general_page(&filesheet::rows(&facts), "Reports", None, None)
        };
        for (x, tab) in [
            (10, filesheet::Tab::Sharing),
            (400, filesheet::Tab::Compatibility),
        ] {
            let (page, height) = rows(true);
            let mut sheet = filesheet::sheet(page, height, &[tab], |_| unreachable!());
            sheet.open_tab(1);
            let (w, h) = sheet.size;
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
            propsheet::draw(
                &mut sheet_c,
                &text,
                &Paint { s, ..theme.paint() },
                &sheet,
                true,
                px,
            );
            c.blit_canvas(x, 10741, &sheet_c);
        }
        let values: [String; 6] =
            ["Report", "", "Ann", "", "q3", "First draft\nfor review"].map(String::from);
        for (x, values) in [(10, Some(&values)), (400, None)] {
            let (page, height) = rows(false);
            let mut sheet = filesheet::sheet(page, height, &[filesheet::Tab::Summary], |_| {
                filesheet::summary_page(values)
            });
            sheet.open_tab(1);
            let (w, h) = sheet.size;
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
            propsheet::draw(
                &mut sheet_c,
                &text,
                &Paint { s, ..theme.paint() },
                &sheet,
                true,
                px,
            );
            c.blit_canvas(x, 11229, &sheet_c);
        }
        let (page, height) = rows(true);
        let mut sheet = filesheet::sheet(
            page,
            height,
            &[filesheet::Tab::Customize],
            |_| unreachable!(),
        );
        sheet.open_tab(1);
        let (w, h) = sheet.size;
        let mut sheet_c = Canvas::new(w as u32, h as u32);
        sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
        propsheet::draw(
            &mut sheet_c,
            &text,
            &Paint { s, ..theme.paint() },
            &sheet,
            true,
            px,
        );
        c.blit_canvas(10, 11717, &sheet_c);
        let names: Vec<String> = filesheet::FOLDER_ICON_NAMES
            .iter()
            .map(|n| n.to_string())
            .collect();
        let swatches = (0..names.len())
            .map(|i| {
                let mut ic = Canvas::new(32, 32);
                let shade = (40 + (i * 37) % 180) as u8;
                ic.fill_rect(
                    Rect::new(4, 4, 24, 24),
                    Color::rgb(shade, 90, 200 - shade / 2),
                );
                Some(ic)
            })
            .collect();
        let mut list = reprisekit::controls::iconlist::IconList::new(names, swatches, None);
        list.pick("folder-music");
        let (cw, ch) = filesheet::CHANGE_ICON_SIZE;
        let mut ci_c = Canvas::new(cw as u32, ch as u32);
        ci_c.fill_rect(ci_c.bounds(), cs.button_face);
        form::draw(
            &mut ci_c,
            &text,
            &Paint { s, ..theme.paint() },
            &filesheet::change_icon_form("hicolor", list, true),
            true,
            px,
        );
        c.blit_canvas(400, 11717, &ci_c);

        c.fill_rect(Rect::new(0, 12163, 950, 494), cs.background);
        let (page, height) = rows(false);
        let facts = filesheet::ShortcutFacts {
            name: "Firefox".into(),
            target_type: "Application".into(),
            target_location: "bin".into(),
            symlink: false,
            fields: [
                "firefox %u".into(),
                "/home/user".into(),
                "Browse the web".into(),
            ],
        };
        let sheet = filesheet::sheet(
            page,
            height,
            &filesheet::sheet_tabs(filesheet::Over::Launcher),
            |_| filesheet::shortcut_page(&facts, None),
        );
        let (w, h) = sheet.size;
        let mut sheet_c = Canvas::new(w as u32, h as u32);
        sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
        propsheet::draw(
            &mut sheet_c,
            &text,
            &Paint { s, ..theme.paint() },
            &sheet,
            true,
            px,
        );
        c.blit_canvas(10, 12171, &sheet_c);
        let mut list = reprisekit::controls::iconlist::IconList::new(
            vec!["firefox".into(), "web-browser".into()],
            vec![None, None],
            None,
        );
        list.pick("firefox");
        let mut ci_c = Canvas::new(cw as u32, ch as u32);
        ci_c.fill_rect(ci_c.bounds(), cs.button_face);
        form::draw(
            &mut ci_c,
            &text,
            &Paint { s, ..theme.paint() },
            &filesheet::change_icon_form("hicolor", list, false),
            true,
            px,
        );
        c.blit_canvas(400, 12171, &ci_c);
    }

    {
        c.fill_rect(Rect::new(400, 9823, 260, 190), cs.background);
        let ghost = |at, name| iconview::Ghost {
            at,
            name,
            icon: None,
            shortcut: false,
        };
        let ghosts = [
            ghost((0, 0), "a.txt"),
            ghost((75, 0), "Photos"),
            ghost((0, 75), "Shortcut to Notes"),
        ];
        if let Some((img, _)) = iconview::drag_image(
            theme,
            &text,
            &ghosts,
            iconview::Cell::square(iconview::CELL),
            (37, 37),
        ) {
            c.blit_over(420, 9843, &img);
        }
    }

    if with_blue {
        let blue_theme = interfaces::theme_for(Interface::Blue);
        c.fill_rect(Rect::new(0, 9575, 950, 238), cs.background);
        c.blit_canvas(
            10,
            9585,
            &look::draw_sample(false, &blue_theme, 1, &text, None),
        );
        c.blit_canvas(
            370,
            9585,
            &look::draw_sample(true, &blue_theme, 1, &text, None),
        );
    }

    if with_blue {
        let blue_theme = interfaces::theme_for(Interface::Blue);
        let bcs = &blue_theme.colors;
        let bp = Paint {
            s,
            ..blue_theme.paint()
        };
        let band = Rect::new(0, 12657, 950, 1078);
        c.fill_rect(band, bcs.button_face);

        let row_label = |c: &mut Canvas, y: i32, s: &str| {
            draw_label(
                c,
                &text,
                Rect::new(band.x + 8, y, 96, 13),
                s,
                false,
                bcs.button_text,
                Align::Left,
                px,
                1,
            );
        };
        let state_label = |c: &mut Canvas, x: i32, y: i32, w: i32, s: &str| {
            draw_label(
                c,
                &text,
                Rect::new(x, y, w, 11),
                s,
                false,
                bcs.button_text,
                Align::Left,
                9,
                1,
            );
        };

        const X0: i32 = 110;
        let mut y = band.y + 8;

        row_label(&mut c, y, "Button");
        for (i, (label, default, ptr, enabled, focus)) in [
            ("Normal", false, Interaction::Idle, true, false),
            ("Disabled", false, Interaction::Idle, false, false),
            ("Pushed", false, Interaction::Pressed, true, false),
            ("Default", true, Interaction::Idle, true, false),
            ("Focused", false, Interaction::Idle, true, true),
            ("", false, Interaction::Idle, true, false),
            ("Hot", false, Interaction::Hot, true, false),
        ]
        .into_iter()
        .enumerate()
        {
            if label.is_empty() {
                continue;
            }
            let r = Rect::new(X0 + i as i32 * 84, y, 75, 23);
            draw_button(
                &mut c, &text, &bp, r, label, default, ptr, enabled, focus, px,
            );
        }
        let cb = colorbutton::ColorButton::new(bcs.background, vec![]);
        colorbutton::draw(&mut c, &bp, Rect::new(X0 + 5 * 84, y, 75, 21), &cb);
        state_label(&mut c, X0 + 5 * 84, y + 25, 76, "Colour");
        y += 23 + 16;

        row_label(&mut c, y, "Check Box");
        for (i, (label, checked)) in [("Unchecked", false), ("Checked", true)]
            .into_iter()
            .enumerate()
        {
            let b = Rect::new(X0 + i as i32 * 80, y, 13, 13);
            draw_checkbox(&mut c, &bp, b, checked, Interaction::Idle);
            state_label(&mut c, b.x, b.bottom() + 2, 76, label);
        }
        {
            let b = Rect::new(X0 + 2 * 80, y, 13, 13);
            draw_checkbox_state(&mut c, &bp, b, CheckState::Mixed, Interaction::Idle);
            state_label(&mut c, b.x, b.bottom() + 2, 76, "Mixed");
        }
        {
            let b = Rect::new(X0 + 3 * 80, y, 13, 13);
            draw_checkbox_disabled(&mut c, &bp, b, true);
            state_label(&mut c, b.x, b.bottom() + 2, 76, "Disabled");
        }
        for (i, (label, checked, ptr)) in [
            ("Hot", false, Interaction::Hot),
            ("Pressed", false, Interaction::Pressed),
            ("Checked Hot", true, Interaction::Hot),
            ("Checked Pressed", true, Interaction::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let b = Rect::new(X0 + (4 + i as i32) * 80, y, 13, 13);
            draw_checkbox(&mut c, &bp, b, checked, ptr);
            state_label(&mut c, b.x, b.bottom() + 2, 76, label);
        }
        y += 13 + 2 + 11 + 12;

        row_label(&mut c, y, "Radio");
        let side = bp.look.controls().radio;
        for (i, (label, checked)) in [("Unchecked", false), ("Checked", true)]
            .into_iter()
            .enumerate()
        {
            let b = Rect::new(X0 + i as i32 * 80, y, side, side);
            draw_radio(&mut c, &bp, b, checked, Interaction::Idle);
            state_label(&mut c, b.x, b.bottom() + 2, 76, label);
        }
        {
            let b = Rect::new(X0 + 2 * 80, y, side, side);
            draw_radio_disabled(&mut c, &bp, b, false);
            state_label(&mut c, b.x, b.bottom() + 2, 76, "Disabled");
        }
        for (i, (label, checked, ptr)) in [
            ("Hot", false, Interaction::Hot),
            ("Pressed", false, Interaction::Pressed),
            ("Checked Hot", true, Interaction::Hot),
            ("Checked Pressed", true, Interaction::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let b = Rect::new(X0 + (3 + i as i32) * 80, y, side, side);
            draw_radio(&mut c, &bp, b, checked, ptr);
            state_label(&mut c, b.x, b.bottom() + 2, 76, label);
        }
        y += side + 2 + 11 + 12;

        row_label(&mut c, y, "Group Box");
        let g = Rect::new(X0, y, 160, 50);
        let lw = text.measure("Group", false, px);
        let label_r = Rect::new(g.x + 10, g.y - 5, lw, 13);
        draw_group_box(&mut c, &text, &bp, g, label_r, "Group", px);
        y += 50 + 16;

        row_label(&mut c, y, "Scroll Bar");
        draw_hscrollbar(&mut c, &bp, Rect::new(X0, y, 150, 17), 3, 5, 20, None);
        state_label(&mut c, X0, y + 19, 150, "Thumb");
        draw_hscrollbar(
            &mut c,
            &bp,
            Rect::new(X0 + 170, y, 150, 17),
            0,
            20,
            20,
            None,
        );
        state_label(&mut c, X0 + 170, y + 19, 150, "Full");
        draw_scrollbar(&mut c, &bp, Rect::new(X0 + 340, y, 17, 100), 8, 4, 20, None);
        state_label(&mut c, X0 + 340, y + 102, 70, "Vertical");
        draw_scrollbar(&mut c, &bp, Rect::new(X0 + 430, y, 17, 100), 2, 8, 20, None);
        state_label(&mut c, X0 + 430, y + 102, 70, "Grip");
        draw_scrollbar(
            &mut c,
            &bp,
            Rect::new(X0 + 520, y, 17, 100),
            0,
            20,
            20,
            None,
        );
        state_label(&mut c, X0 + 520, y + 102, 70, "Full");
        draw_scrollbar(&mut c, &bp, Rect::new(X0 + 690, y, 17, 83), 0, 4, 20, None);
        draw_hscrollbar(
            &mut c,
            &bp,
            Rect::new(X0 + 607, y + 83, 83, 17),
            0,
            4,
            20,
            None,
        );
        draw_scroll_corner(&mut c, &bp, Rect::new(X0 + 690, y + 83, 17, 17), true);
        state_label(&mut c, X0 + 607, y + 102, 70, "Corner");
        y += 100 + 2 + 11 + 12;

        row_label(&mut c, y, "Scroll Lit");
        use reprisekit::classic::widgets::ScrollHit;
        for (i, (label, hit, how)) in [
            ("Up Hot", ScrollHit::Back, Interaction::Hot),
            ("Up Pressed", ScrollHit::Back, Interaction::Pressed),
            ("Down Hot", ScrollHit::Forward, Interaction::Hot),
            ("Down Pressed", ScrollHit::Forward, Interaction::Pressed),
            ("Thumb Hot", ScrollHit::Thumb, Interaction::Hot),
            ("Thumb Pressed", ScrollHit::Thumb, Interaction::Pressed),
            ("Page Pressed", ScrollHit::PageForward, Interaction::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let x = X0 + i as i32 * 70;
            let lit = Some((hit, how));
            draw_scrollbar(&mut c, &bp, Rect::new(x, y, 17, 100), 2, 8, 20, lit);
            state_label(&mut c, x, y + 102, 68, label);
        }
        for (i, (label, hit, how)) in [
            ("Left Hot", ScrollHit::Back, Interaction::Hot),
            ("Right Pressed", ScrollHit::Forward, Interaction::Pressed),
            ("Page Pressed", ScrollHit::PageBack, Interaction::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let (x, yy) = (X0 + 500, y + i as i32 * 34);
            let lit = Some((hit, how));
            draw_hscrollbar(&mut c, &bp, Rect::new(x, yy, 150, 17), 12, 5, 20, lit);
            state_label(&mut c, x + 160, yy + 3, 70, label);
        }
        y += 100 + 2 + 11 + 12;

        row_label(&mut c, y, "Field");
        draw_sunken_field(&mut c, &bp, Rect::new(X0, y, 150, 21), bp.cs.window);
        state_label(&mut c, X0, y + 23, 150, "Field");
        y += 21 + 23 + 5;

        row_label(&mut c, y, "Combo Drop");
        draw_combo_drop(
            &mut c,
            &bp,
            Rect::new(X0, y, 17, 21),
            true,
            Interaction::Idle,
            false,
        );
        state_label(&mut c, X0, y + 23, 76, "Enabled");
        draw_combo_drop(
            &mut c,
            &bp,
            Rect::new(X0 + 80, y, 17, 21),
            false,
            Interaction::Idle,
            false,
        );
        state_label(&mut c, X0 + 80, y + 23, 76, "Disabled");
        let bcm = blue_theme.look.controls();
        for (i, (label, focus, enabled)) in [
            ("Focused", true, true),
            ("Normal", false, true),
            ("Disabled", false, false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut bcombo = combo::Combo::new(&["Reprise Blue", "Reprise Classic"], 0, 8);
            bcombo.enabled = enabled;
            let r = Rect::new(X0 + 180 + i as i32 * 140, y, 120, combo::COMBO_H);
            let lay = combo::layout(&bcombo, r, &bcm);
            combo::draw(
                &mut c,
                &text,
                &bp,
                &bcombo,
                &lay,
                focus,
                focus,
                Caret::Unfocused,
                combo::ComboLit::IDLE,
                px,
            );
            state_label(&mut c, r.x, y + 23, 120, label);
        }
        for (i, (label, ptr)) in [("Hot", Interaction::Hot), ("Pressed", Interaction::Pressed)]
            .into_iter()
            .enumerate()
        {
            let x = X0 + 610 + i as i32 * 60;
            draw_combo_drop(&mut c, &bp, Rect::new(x, y, 15, 17), true, ptr, false);
            state_label(&mut c, x, y + 23, 56, label);
        }
        y += 21 + 23 + 5;

        row_label(&mut c, y, "Tab");
        let page = Rect::new(X0, y + 20, 386, 150);
        draw_page(&mut c, &bp, page);
        draw_tab(
            &mut c,
            &text,
            &bp,
            Rect::new(X0, y, 60, 20),
            "Open",
            true,
            false,
            page,
            px,
        );
        draw_tab(
            &mut c,
            &text,
            &bp,
            Rect::new(X0 + 62, y, 60, 20),
            "Closed",
            false,
            false,
            page,
            px,
        );
        draw_tab(
            &mut c,
            &text,
            &bp,
            Rect::new(X0 + 124, y + 2, 60, 18),
            "Hot",
            false,
            true,
            page,
            px,
        );
        {
            let (px0, py0) = (page.x, page.y);
            let greyed_combo = |id, x, y, w, items: &[&str], at| {
                let mut cb = combo::Combo::new(items, at, 8);
                cb.enabled = false;
                Item::new(id, Rect::new(x, y, w, combo::COMBO_H), Control::Combo(cb))
            };
            let spinner = |id, r, value, buddy| {
                let mut ud = updown::UpDown::new(value, 1, 9999);
                if buddy {
                    ud = ud.with_buddy();
                }
                ud.enabled = false;
                Item::new(id, r, Control::UpDown(ud))
            };
            let items = vec![
                Item::group(
                    "saver_g",
                    Rect::new(px0 + 15, py0 + 14, 357, 72),
                    "&Screen saver",
                ),
                greyed_combo("saver", px0 + 24, py0 + 26, 172, &["(None)", "Blank"], 0),
                Item::label_at("wait_l", px0 + 25, py0 + 58, 30, "&Wait:").enabled(false),
                spinner("wait", Rect::new(px0 + 52, py0 + 55, 50, 20), 10, false),
                Item::label_at("minutes_l", px0 + 104, py0 + 58, 40, "minutes").enabled(false),
                Item::check("lock", px0 + 150, py0 + 59, "On resume, &password protect")
                    .enabled(false),
                greyed_combo(
                    "pos",
                    px0 + 205,
                    py0 + 26,
                    75,
                    &["Center", "Tile", "Stretch"],
                    2,
                ),
                Item::edit("field", Rect::new(px0 + 15, py0 + 98, 173, 20), "", false),
                spinner("year", Rect::new(px0 + 198, py0 + 97, 79, 21), 2026, true),
                Item::radio("bytes", px0 + 15, py0 + 124, "&Bytes", "unit")
                    .checked(true)
                    .enabled(false),
                Item::radio("words", px0 + 70, py0 + 124, "&Words", "unit").enabled(false),
                Item::check("sync", px0 + 198, py0 + 124, "&Synchronize")
                    .checked(true)
                    .enabled(false),
            ];
            form::draw(
                &mut c,
                &text,
                &bp.on_page(page),
                &form::Form::new(items),
                false,
                px,
            );
        }
        y += 20 + 150 + 12;

        row_label(&mut c, y, "Progress");
        draw_progress(&mut c, &bp, Rect::new(X0, y, 150, 16), 0.5, true);
        state_label(&mut c, X0, y + 18, 150, "50%");
        draw_progress(&mut c, &bp, Rect::new(X0 + 170, y, 150, 16), 0.5, false);
        state_label(&mut c, X0 + 170, y + 18, 150, "Disabled");
        y += 16 + 18 + 8;

        row_label(&mut c, y, "Trackbar");
        let tb = trackbar::TrackBar::new(9, 18);
        let lay_a = trackbar::layout(&tb, X0, y, 90);
        draw_trackbar(&mut c, &bp, &lay_a, true, false, Interaction::Idle);
        state_label(&mut c, X0, y + trackbar::HEIGHT + 2, 90, "Enabled");
        let lay_b = trackbar::layout(&tb, X0 + 110, y, 90);
        draw_trackbar(&mut c, &bp, &lay_b, false, false, Interaction::Idle);
        state_label(&mut c, X0 + 110, y + trackbar::HEIGHT + 2, 90, "Disabled");
        for (i, (label, ptr)) in [("Hot", Interaction::Hot), ("Pressed", Interaction::Pressed)]
            .into_iter()
            .enumerate()
        {
            let x = X0 + 220 + i as i32 * 110;
            draw_trackbar(
                &mut c,
                &bp,
                &trackbar::layout(&tb, x, y, 90),
                true,
                false,
                ptr,
            );
            state_label(&mut c, x, y + trackbar::HEIGHT + 2, 90, label);
        }
        y += trackbar::HEIGHT + 2 + 11 + 8;

        row_label(&mut c, y, "Spin");
        let up_a = Rect::new(X0, y, 15, 9);
        let down_a = Rect::new(X0, y + 9, 15, 9);
        draw_spin_buttons(&mut c, &bp, up_a, down_a, true, None);
        state_label(&mut c, X0, y + 20, 60, "Enabled");
        let up_b = Rect::new(X0 + 80, y, 15, 9);
        let down_b = Rect::new(X0 + 80, y + 9, 15, 9);
        draw_spin_buttons(&mut c, &bp, up_b, down_b, false, None);
        state_label(&mut c, X0 + 80, y + 20, 60, "Disabled");
        for (i, (label, enabled)) in [("Spinner", true), ("Greyed", false)]
            .into_iter()
            .enumerate()
        {
            let mut ud = updown::UpDown::new(10, 1, 9999);
            ud.enabled = enabled;
            let lay = updown::layout(X0 + 180 + i as i32 * 70, y);
            updown::draw(&mut c, &text, &bp, &ud, &lay, false, None, px);
            state_label(&mut c, lay.field.x, y + 22, 60, label);
        }
        for (i, (label, enabled)) in [("Year", true), ("Greyed", false)].into_iter().enumerate() {
            let mut year = updown::UpDown::new(2026, 1970, 2099).with_buddy();
            year.enabled = enabled;
            let r = Rect::new(X0 + 330 + i as i32 * 90, y, 79, 21);
            let lay = updown::layout_for(&year, r, &bcm, reprisekit::theme::TextUnits::ONE);
            updown::draw(&mut c, &text, &bp, &year, &lay, false, None, px);
            state_label(&mut c, r.x, y + 23, 79, label);
        }
        {
            use reprisekit::controls::timefield;
            let tf = timefield::TimeField::new(23, 41, 24, true);
            let r = Rect::new(X0 + 510, y, 103, 20);
            let lay = timefield::layout(r, &bcm);
            timefield::draw(&mut c, &text, &bp, &tf, &lay, false, None, px);
            state_label(&mut c, r.x, y + 23, 103, "Time");
        }
        use reprisekit::classic::widgets::Arrow;
        for (i, (label, arrow, how)) in [
            ("Up Hot", Arrow::Up, Interaction::Hot),
            ("Up Held", Arrow::Up, Interaction::Pressed),
            ("Dn Hot", Arrow::Down, Interaction::Hot),
            ("Dn Held", Arrow::Down, Interaction::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let x = X0 + 630 + i as i32 * 50;
            let (up, down) = (Rect::new(x, y, 15, 8), Rect::new(x, y + 9, 15, 8));
            draw_spin_buttons(&mut c, &bp, up, down, true, Some((arrow, how)));
            state_label(&mut c, x, y + 20, 48, label);
        }
        y += 21 + 23 + 5;

        row_label(&mut c, y, "Caption ?");
        let mut caption_h = 0;
        for (i, (label, how)) in [("Hot", Interaction::Hot), ("Pressed", Interaction::Pressed)]
            .into_iter()
            .enumerate()
        {
            let x = X0 + i as i32 * 220;
            let g = FrameGeometry::layout(&blue_theme.metrics, 180, 4, false, Chrome::DIALOG)
                .translated(-x, -y);
            look::draw_dialog_frame(
                &mut c,
                &blue_theme,
                &text,
                &g,
                &FrameState {
                    title: "Run",
                    active: true,
                    maximized: false,
                    pressed: None,
                    glow: Glow::lit(None),
                    icon: None,
                    tabbed: false,
                },
                Some(how),
            );
            state_label(&mut c, x, y + g.total.1 + 2, 100, label);
            caption_h = g.total.1;
        }
        y += caption_h + 2 + 11 + 8;

        use reprisekit::controls::calendar;
        row_label(&mut c, y, "Calendar");
        for (i, (label, enabled)) in [("Enabled", true), ("Greyed", false)]
            .into_iter()
            .enumerate()
        {
            let mut cal = calendar::Calendar::new(2026, 9, Some(29), 0);
            cal.enabled = enabled;
            let lay = calendar::layout(
                Rect::new(X0 + i as i32 * 200, y, calendar::W, calendar::H),
                &theme.controls(),
            );
            calendar::draw(&mut c, &text, &bp, &cal, &lay, false, px);
            state_label(&mut c, lay.frame.x, y + calendar::H + 2, 100, label);
        }
    }

    {
        let band_y = if with_blue { 13735 } else { 12657 };
        c.fill_rect(Rect::new(0, band_y, 950, 494), cs.background);
        let general = || {
            let facts = filesheet::Facts {
                type_name: "Shortcut".into(),
                description: Some("Documents".into()),
                location: "/home/user/Desktop".into(),
                size: "20 bytes (20 bytes)".into(),
                on_disk: "0 bytes".into(),
                modified: "Today, October 04, 2026, 11:39:12 AM".into(),
                accessed: "Today, October 04, 2026, 11:39:12 AM".into(),
                ..Default::default()
            };
            filesheet::general_page(&filesheet::rows(&facts), "Docs", None, None)
        };
        let link = filesheet::ShortcutFacts {
            name: "Docs".into(),
            target_type: "File Folder".into(),
            target_location: "user".into(),
            symlink: true,
            fields: ["/home/user/Documents".into(), String::new(), String::new()],
        };
        let web = filesheet::WebFacts {
            name: "Example".into(),
            url: "https://example.org/".into(),
        };
        let sheets = [
            (filesheet::Over::Symlink { runs: false }, 10),
            (filesheet::Over::WebLink, 400),
        ];
        for (over, x) in sheets {
            let (page, height) = general();
            let sheet = filesheet::sheet(
                page,
                height,
                &filesheet::sheet_tabs(over),
                |tab| match tab {
                    filesheet::Tab::WebDocument => filesheet::web_page(&web, None),
                    _ => filesheet::shortcut_page(&link, None),
                },
            );
            let (w, h) = sheet.size;
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
            propsheet::draw(
                &mut sheet_c,
                &text,
                &Paint { s, ..theme.paint() },
                &sheet,
                true,
                px,
            );
            c.blit_canvas(x, band_y + 8, &sheet_c);
        }
    }

    c
}

/// Glass's frames, taskbars, flyouts and control sheets (each in its own `render_glass_*`), over a box-blurred stripe backdrop standing in for the blurred desktop. Drawn once: Glass has one scheme.
fn render_glass() -> Canvas {
    let theme = interfaces::theme_for(Interface::Glass);
    let frames = render_glass_frames();
    let parts = [
        render_glass_task_rows(),
        render_glass_spots(),
        render_glass_choices(&theme),
        render_glass_sheet(&theme),
        render_glass_fields(&theme),
        render_glass_open_lists(&theme),
        render_glass_scrollbars(&theme),
        render_glass_ranges(&theme),
        render_glass_lists(&theme),
        render_glass_wells(&theme),
        render_glass_popup_parts(&theme),
        render_glass_menus(&theme),
        render_glass_marquees(&theme),
        render_glass_balloons(&theme),
        render_glass_flyouts(&theme),
        render_glass_items(&theme),
        render_glass_toolbars(&theme),
        render_glass_materials(reprisekit::glass::Mode::Light),
        render_glass_square_frames(),
        render_glass_client_glass(&theme),
        render_glass_peek_sheet(&theme),
    ];
    let height = frames.height + parts.iter().map(|p| p.height).sum::<u32>();
    let mut all = Canvas::new(950, height);
    all.blit_canvas(0, 0, &frames);
    let mut y = frames.height as i32;
    for p in &parts {
        all.blit_canvas(0, y, p);
        y += p.height as i32;
    }
    all
}

/// Dark Glass's captions, then the same control rows as [`render_glass`] under the dark scheme, then its Smoked and Molten captions.
fn render_glass_dark() -> Canvas {
    let theme = interfaces::theme_with(
        Interface::Glass,
        reprisekit::blue::Variant::default(),
        &reprisekit::glass::Settings {
            mode: reprisekit::glass::Mode::Dark,
            ..reprisekit::glass::Settings::default()
        },
    );
    let parts = [
        render_glass_dark_captions(&theme),
        render_glass_client_glass(&theme),
        render_glass_dark_bar(&theme),
        render_glass_choices(&theme),
        render_glass_sheet(&theme),
        render_glass_fields(&theme),
        render_glass_open_lists(&theme),
        render_glass_scrollbars(&theme),
        render_glass_ranges(&theme),
        render_glass_lists(&theme),
        render_glass_wells(&theme),
        render_glass_popup_parts(&theme),
        render_glass_menus(&theme),
        render_glass_marquees(&theme),
        render_glass_balloons(&theme),
        render_glass_flyouts(&theme),
        render_glass_items(&theme),
        render_glass_toolbars(&theme),
        render_glass_materials(reprisekit::glass::Mode::Dark),
        render_glass_peek_sheet(&theme),
    ];
    let mut all = Canvas::new(950, parts.iter().map(|p| p.height).sum::<u32>());
    let mut y = 0;
    for p in &parts {
        all.blit_canvas(0, y, p);
        y += p.height as i32;
    }
    all
}

/// Glass windows at scale 2 over the stripes, 1300x700 device px, then the Start panel and popups ([`glass_popups_at_2`]) and lit and held caption buttons ([`render_glass_scale2_buttons`]).
fn render_glass_scale2() -> Canvas {
    use reprisekit::glass::{self, Frame, Material, Settings};
    let rows = [
        Settings::default(),
        Settings {
            frame: Frame::Square,
            ..Settings::default()
        },
        Settings {
            color: glass::swatch("frost").expect("frost is a swatch"),
            intensity: 100,
            ..Settings::default()
        },
        Settings {
            color: glass::swatch("blush").expect("blush is a swatch"),
            intensity: 100,
            material: Material::Molten,
            ..Settings::default()
        },
    ];
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(1300, 700 + GLASS_SCALE2_BARS + POPUPS_AT_2);
    glass_scale2_bars(&mut c, &rows, 700);
    glass_popups_at_2(&mut c, 700 + GLASS_SCALE2_BARS, &text);
    for (row, s) in rows.iter().enumerate() {
        let theme =
            interfaces::theme_with(Interface::Glass, reprisekit::blue::Variant::default(), s);
        let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let two = theme.scaled(2);
        for (i, active) in [true, false].into_iter().enumerate() {
            let g = FrameGeometry::layout(&theme.metrics, 290, 30, false, Chrome::WINDOW).scaled(2);
            let st = FrameState {
                title: "before.txt - Notepad",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &two, &text, &caption_text, &g, &st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
            c.blit_over(20 + 640 * i as i32, 20 + 170 * row as i32, &f);
        }
    }
    let buttons = render_glass_scale2_buttons(&rows);
    let mut all = Canvas::new(c.width, c.height + buttons.height);
    all.blit_canvas(0, 0, &c);
    all.blit_canvas(0, c.height as i32, &buttons);
    all
}

/// Caption buttons at scale 2 over the stripes, 1300x480 device px, a row per setting of [`render_glass_scale2`].
fn render_glass_scale2_buttons(rows: &[reprisekit::glass::Settings]) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(1300, 120 * rows.len() as i32);
    for (row, s) in rows.iter().enumerate() {
        let theme =
            interfaces::theme_with(Interface::Glass, reprisekit::blue::Variant::default(), s);
        let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let two = theme.scaled(2);
        let state = |glow, pressed| FrameState {
            title: "Notes",
            active: true,
            maximized: false,
            pressed,
            glow: Glow::lit(glow),
            icon: None,
            tabbed: false,
        };
        let y = 12 + 120 * row as i32;
        for (i, st) in [
            state(Some(CaptionButton::Maximize), None),
            state(None, Some(CaptionButton::Close)),
        ]
        .iter()
        .enumerate()
        {
            let g = FrameGeometry::layout(&theme.metrics, 180, 10, false, Chrome::WINDOW).scaled(2);
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &two, &text, &caption_text, &g, st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
            c.blit_over(20 + 420 * i as i32, y, &f);
        }
        let g = FrameGeometry::layout(&theme.metrics, 180, 10, false, Chrome::DIALOG).scaled(2);
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_dialog_frame(
            &mut f,
            &two,
            &text,
            &g,
            &state(None, None),
            Some(Interaction::Hot),
        );
        f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        c.blit_over(860, y, &f);
    }
    c
}

/// How many rows [`glass_popups_at_2`] draws, 1300 px wide.
const POPUPS_AT_2: i32 = 1960;

/// Glass's Start panel, Alt+Tab panel and task preview at scale 2 in Sky, drawn into `c` from row `y0` down.
fn glass_popups_at_2(c: &mut Canvas, y0: i32, text: &TextRenderer) {
    use reprisekit::startpanel::{self as sp, PlaceRow, ProgramRow, Spot, StartPanel};
    let theme = interfaces::theme_for(Interface::Glass);
    let two = theme.scaled(2);
    let glass = reprisekit::glass::Settings::default();
    let face = interfaces::start_panel_text(&theme);
    let prog = |name: &str| ProgramRow {
        name: name.into(),
        icon: None,
        jump: false,
        new_pin: false,
    };
    let start = |lit: Spot| {
        let mut p = StartPanel::new();
        p.pinned = vec![prog("Files"), prog("Terminal")];
        p.frequent = ["Notepad", "Calculator", "Paint"].map(prog).to_vec();
        p.places = ["user", "Documents", "Pictures", "Music", "Computer"]
            .iter()
            .map(|l| PlaceRow {
                label: l.to_string(),
                picture: None,
                rule_before: *l == "Computer",
                submenu: false,
            })
            .collect();
        p.pointer(Some(lit));
        p
    };
    for (k, lit) in [Spot::Program(1), Spot::Shutdown].into_iter().enumerate() {
        let p = start(lit);
        let l = sp::layout::layout(&p).scaled(2);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_start_panel(&mut panel, &two, &glass, &face, &p, &l, None, true);
        if k == 0 {
            c.blit_over(20, y0 + 20, &panel);
        } else {
            let o = l.canvas;
            let b = l.rect(Spot::Shutdown).expect("Shut down");
            let a = l.rect(Spot::ShutdownArrow).expect("its arrow half");
            let both = Rect::new(
                b.x - o.x - 20,
                b.y - o.y - 20,
                a.right() - b.x + 40,
                b.height + 40,
            );
            c.blit_over(900, y0 + 20, &panel.copy_rect(both));
        }
    }
    let grid = interfaces::switcher_grid(&theme);
    let l = switcher::SwitcherLayout::thumbnails(3, 1, Rect::new(0, 0, 640, 300), grid).scaled(2);
    let pictures: Vec<Rect> = l
        .wells
        .iter()
        .map(|&w| taskpreview::fit(400, 300, w, 2))
        .collect();
    let stills: Vec<Canvas> = pictures
        .iter()
        .map(|r| {
            let mut s = Canvas::new(r.width as u32, r.height as u32);
            s.fill_rect(s.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
            s.fill_rect(Rect::new(0, 0, r.width, 12), Color::rgb(0xDD, 0xE6, 0xF2));
            s
        })
        .collect();
    let cells: Vec<switcher::ThumbCell> = pictures
        .iter()
        .zip(&stills)
        .map(|(&r, s)| switcher::ThumbCell {
            icon: None,
            picture: Some(r),
            still: Some(s),
        })
        .collect();
    let a = l.canvas();
    let mut panel = Canvas::new(a.width as u32, a.height as u32);
    interfaces::draw_switcher(
        &mut panel,
        &two,
        text,
        &l,
        "Untitled - Notepad",
        &cells,
        1,
        Some(2),
    );
    c.blit_over(20, y0 + 1100, &panel);
    let l = taskpreview::TaskPreviewLayout::compute(
        interfaces::task_preview_grid(&theme),
        2,
        Rect::new(300, 400, 60, 40),
        Rect::new(0, 0, 640, 400),
        reprisekit::taskbar::Edge::Bottom,
    )
    .scaled(2);
    let cells: Vec<taskpreview::Cell> = ["Untitled - Notepad", "notes.txt - Notepad"]
        .iter()
        .map(|&title| taskpreview::Cell {
            title,
            icon: None,
            active: false,
            picture: None,
        })
        .collect();
    let a = l.canvas();
    let mut panel = Canvas::new(a.width as u32, a.height as u32);
    interfaces::draw_task_preview(&mut panel, &two, text, &l, &cells, Some(1));
    c.blit_over(20, y0 + 1500, &panel);
}

/// How tall [`glass_scale2_bars`] is.
const GLASS_SCALE2_BARS: i32 = 4 * 92 + 90 + 580;

/// Glass taskbars at scale 2 into `c` from row `top`: a 640 px bar per setting in `rows` along the bottom, then 40 px and small bars as 70 px columns up the left of a 280 px output.
fn glass_scale2_bars(c: &mut Canvas, rows: &[reprisekit::glass::Settings], top: i32) {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PagerButton, TaskEntry, TaskRow, TaskbarCounts,
        TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let bar = |c: &mut Canvas, theme: &Theme, edge: Edge, t: i32, length: i32, at: (i32, i32)| {
        let style = theme.look.style().unwrap();
        let shape = theme.taskbar_shape();
        let start_text = TextRenderer::load(&style.start_text.font());
        let clock = ClockText {
            time: text.measure("3:45 PM", false, theme.fonts.size_px),
            date: text.measure("10/04/2026", false, theme.fonts.size_px),
        };
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 4,
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length,
            thickness: t,
        };
        let mut l = TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            counts,
            clock,
            Bands::default(),
        );
        l.lay_tasks(&theme.metrics, &shape, &[0, 2, 1, 3], TaskRow::Shared);
        let px = shape.task_icon * 2;
        let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
            title: title.into(),
            active,
            attention: false,
            glow: Some(Color::rgb(0xD0, 0x40, 0x30)),
            icon: Some(icon.into()),
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: Vec::new(),
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![
                PagerButton::default(),
                PagerButton {
                    current: true,
                    attention: false,
                },
            ],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: Some(3),
            task_pressed: Some(1),
            task_hot_x: Some(12),
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: true,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                task("", false, indicator::draw_record(px)),
                task("Command Prompt (2)", false, indicator::draw_microphone(px)),
                task("Untitled - Notepad", true, indicator::draw_record(px)),
                task("Pictures (3)", false, indicator::draw_microphone(px)),
            ],
            tray_icons: vec![Some(indicator::draw_record(32).into())],
            clock: "3:45 PM".into(),
        };
        let two = theme.scaled(2);
        let l = l.scaled(2);
        let mut canvas = Canvas::new(l.width as u32, l.height as u32);
        look::draw_taskbar(&mut canvas, &two, &text, &start_text, &l, &state);
        c.blit_over(at.0, at.1, &canvas);
    };
    let themes: Vec<Theme> = rows
        .iter()
        .map(|s| interfaces::theme_with(Interface::Glass, reprisekit::blue::Variant::default(), s))
        .collect();
    for (i, theme) in themes.iter().enumerate() {
        bar(
            c,
            theme,
            Edge::Bottom,
            40,
            640,
            (10, top + 10 + 92 * i as i32),
        );
    }
    let mut small = themes[0].clone();
    interfaces::small_bar(&mut small, &rows[0]);
    let below = top + 10 + 92 * rows.len() as i32;
    bar(c, &small, Edge::Bottom, 30, 640, (10, below));
    let side = below + 80;
    bar(c, &themes[0], Edge::Left, 70, 280, (10, side));
    bar(c, &small, Edge::Left, 70, 280, (170, side));
}

/// Dark Glass's taskbar over the stripes with two windows lacking icons: each shows the light window glyph.
fn render_glass_dark_bar(theme: &Theme) -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let start_text = TextRenderer::load(&style.start_text.font());
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &style.taskbar,
        930,
        true,
        TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        },
        text.measure("3:45 PM", false, theme.fonts.size_px),
    );
    let entry = |title: &str, active: bool| TaskEntry {
        title: title.into(),
        active,
        attention: false,
        icon: None,
        glow: None,
    };
    let state = TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        pager: vec![],
        pager_hot: None,
        pager_mark: Default::default(),
        task_hot: None,
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        tasks: vec![entry("xclock", false), entry("xeyes", true)],
        tray_icons: vec![],
        clock: "3:45 PM".into(),
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
    };
    let mut c = blurred_stripes(950, 60);
    let mut bar = Canvas::new(930, layout.height as u32);
    look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
    c.blit_over(10, 10, &bar);
    c
}

/// Two restored dark Glass windows over the stripes, 950x80, the focused one first.
fn render_glass_dark_captions(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 80);
    for (i, active) in [true, false].into_iter().enumerate() {
        let g = FrameGeometry::layout(&theme.metrics, 424, 20, false, Chrome::WINDOW);
        let st = FrameState {
            title: "before.txt - Notepad",
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
        f.fill_rect(g.client, theme.colors.button_face);
        c.blit_canvas(15 + 470 * i as i32, 10, &f);
    }
    c
}

/// Two Glass windows over the stripes, the focused one first, 950x110, each with a menu bar
/// across its client's top: the bar clear over the frame's glass, its titles glowing, the client
/// below.
fn render_glass_client_glass(theme: &Theme) -> Canvas {
    use reprisekit::controls::menubar::Bar;
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 110);
    let bar = Bar::new(vec![
        ("&File", vec![]),
        ("&Edit", vec![]),
        ("&View", vec![]),
        ("F&avorites", vec![]),
    ]);
    for (i, active) in [true, false].into_iter().enumerate() {
        let g = FrameGeometry::layout(&theme.metrics, 424, 50, false, Chrome::WINDOW);
        let themed = Theme {
            active,
            ..theme.clone()
        };
        let mut client = Canvas::new(g.client.width as u32, g.client.height as u32);
        client.fill_rect(client.bounds(), theme.colors.button_face);
        let lay = bar.layout(
            &text,
            theme.fonts.size_px,
            client.bounds(),
            &theme.controls(),
        );
        bar.draw(&mut client, &text, &themed, &lay);
        let g = g.with_glass_top(client.take_glass_top());
        let st = FrameState {
            title: "Documents",
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
        let glass = look::client_glass(theme, active).unwrap();
        for y in g.client.y..g.client.y + g.glass_top {
            for x in g.client.x..g.client.right() {
                f.put_raw(x, y, glass);
            }
        }
        f.blit_over(g.client.x, g.client.y, &client);
        c.blit_over(15 + 470 * i as i32, 10, &f);
    }
    c
}

/// An inactive window, then the sheet a peek draws in its place, over the stripes, 950x150.
fn render_glass_peek_sheet(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
    let mut c = blurred_stripes(950, 150);
    let g = FrameGeometry::layout(&theme.metrics, 400, 90, false, Chrome::WINDOW);
    let st = FrameState {
        title: "before.txt - Notepad",
        active: false,
        maximized: false,
        pressed: None,
        glow: Glow::lit(None),
        icon: None,
        tabbed: false,
    };
    let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
    f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
    c.blit_over(15, 10, &f);
    let mut sheet = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    let r = sheet.bounds();
    look::draw_sheet(&mut sheet, theme, r);
    c.blit_over(485, 10, &sheet);
    c
}

/// Smoked then Molten windows in `mode` over the stripes, 950x150.
fn render_glass_materials(mode: reprisekit::glass::Mode) -> Canvas {
    use reprisekit::glass::{Material, Settings};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(950, 150);
    for (row, material) in [Material::Smoked, Material::Molten].into_iter().enumerate() {
        let theme = interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &Settings {
                mode,
                material,
                ..Settings::default()
            },
        );
        let style = theme.look.style().unwrap();
        let caption_text = TextRenderer::load(&style.caption_text.font());
        for (i, active) in [true, false].into_iter().enumerate() {
            let g = FrameGeometry::layout(&theme.metrics, 424, 20, false, Chrome::WINDOW);
            let st = FrameState {
                title: "before.txt - Notepad",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &theme, &text, &caption_text, &g, &st);
            f.fill_rect(g.client, theme.colors.button_face);
            c.blit_canvas(15 + 470 * i as i32, 10 + 70 * row as i32, &f);
        }
    }
    c
}

/// Glass's scrollbars on its face, 950x170: upright and lying bars in each state, then two bars meeting at their corner.
fn render_glass_scrollbars(theme: &Theme) -> Canvas {
    use reprisekit::classic::widgets::ScrollHit;
    let p = theme.paint();
    let mut c = Canvas::new(950, 170);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let (hot, held) = (Interaction::Hot, Interaction::Pressed);
    let upright = [
        None,
        Some((ScrollHit::Back, hot)),
        Some((ScrollHit::Back, held)),
        Some((ScrollHit::Forward, hot)),
        Some((ScrollHit::Forward, held)),
        Some((ScrollHit::Thumb, hot)),
        Some((ScrollHit::Thumb, held)),
        Some((ScrollHit::PageForward, held)),
        Some((ScrollHit::PageForward, hot)),
    ];
    for (i, lit) in upright.into_iter().enumerate() {
        let r = Rect::new(10 + 30 * i as i32, 10, 17, 150);
        draw_scrollbar(&mut c, &p, r, 2, 6, 20, lit);
    }
    draw_scrollbar(&mut c, &p, Rect::new(280, 10, 17, 150), 0, 20, 20, None);
    let lying = [
        None,
        Some((ScrollHit::Back, hot)),
        Some((ScrollHit::Forward, held)),
        Some((ScrollHit::Thumb, hot)),
        Some((ScrollHit::PageBack, held)),
    ];
    for (i, lit) in lying.into_iter().enumerate() {
        let r = Rect::new(320, 10 + 25 * i as i32, 300, 17);
        draw_hscrollbar(&mut c, &p, r, 8, 6, 20, lit);
    }
    draw_hscrollbar(&mut c, &p, Rect::new(320, 135, 300, 17), 0, 20, 20, None);
    draw_scrollbar(&mut c, &p, Rect::new(850, 10, 17, 120), 0, 4, 20, None);
    draw_hscrollbar(&mut c, &p, Rect::new(660, 130, 190, 17), 0, 4, 20, None);
    draw_scroll_corner(&mut c, &p, Rect::new(850, 130, 17, 17), true);
    c
}

/// Glass's picture wells on the face, 950x210: full and thin wells, the colour spectrum, the appearance sample.
fn render_glass_wells(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 210);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let picture = |w: u32, h: u32| {
        let mut pic = Canvas::new(w, h);
        pic.fill_rect(pic.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        pic.fill_rect(Rect::new(8, 8, 16, 16), Color::rgb(40, 120, 220));
        Some(pic)
    };
    let form = form::Form::new(vec![
        Item::new(
            "full",
            Rect::new(10, 10, 160, 120),
            Control::Picture {
                canvas: picture(156, 116),
                frame: 2,
            },
        ),
        Item::new(
            "thin",
            Rect::new(190, 10, 160, 120),
            Control::Picture {
                canvas: picture(158, 118),
                frame: 1,
            },
        ),
        Item::new("spectrum", Rect::new(370, 10, 154, 15), Control::Spectrum),
        Item::new(
            "sample",
            Rect::new(540, 10, 350, 188),
            Control::Sample {
                themes: false,
                theme: theme.clone(),
                icon: None,
            },
        ),
    ]);
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form,
        false,
        theme.fonts.size_px,
    );
    c
}

/// Glass's list headers, list frames and status bars on the face, 950x250.
fn render_glass_lists(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let mut c = Canvas::new(950, 250);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    for (i, (sort, hot, held)) in [
        ((0, true), Some(1), None),
        ((0, false), Some(0), Some(2)),
        ((1, true), None, Some(0)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut lv = listview::sample();
        lv.sort = Some(sort);
        lv.edit = None;
        lv.header.motion(hot);
        if held.is_some() {
            lv.header.press(held, false);
        }
        let r = Rect::new(10 + 310 * i as i32, 10, 300, 130);
        let lay = listview::layout(&lv, r, &cm);
        listview::draw(&mut c, &text, &p, &lv, &lay, None, theme.fonts.size_px);
    }
    let mut icon = Canvas::new(statusbar::ICON_PX as u32, statusbar::ICON_PX as u32);
    icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(80, 193, 255));
    let h = statusbar::height(&theme.controls());
    for (i, grip) in [true, false].into_iter().enumerate() {
        let sb = StatusBar {
            panels: vec![
                Panel {
                    text: "Processes: 37".into(),
                    width: 0,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "CPU Usage: 0%".into(),
                    width: 130,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "Computer".into(),
                    width: 160,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: Some(icon.clone()),
                },
            ],
            grip,
        };
        let bar = Rect::new(10, 160 + i as i32 * (h + 10), 600, h);
        statusbar::draw(&mut c, &text, &p, &sb, bar, theme.fonts.size_px);
    }
    draw_size_grip(&mut c, &p, Rect::new(640, 160, 17, 17));
    c
}

/// Glass's selection box, 950x110: a marquee over black, white and grey patches, and over stripes as a snap outline.
fn render_glass_marquees(theme: &Theme) -> Canvas {
    let mut c = Canvas::new(950, 110);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    let p = theme.paint();
    let backs = [
        Color::rgb(0, 0, 0),
        Color::rgb(0xFF, 0xFF, 0xFF),
        Color::rgb(0x80, 0x80, 0x80),
    ];
    for (i, back) in backs.into_iter().enumerate() {
        let x = 10 + 200 * i as i32;
        c.fill_rect(Rect::new(x, 10, 180, 90), back);
        draw_marquee(
            &mut c,
            &p,
            Rect::new(x + 30, 25, 170, 60),
            Color::rgb(0, 0, 0),
        );
    }
    for i in 0..10 {
        let shade = if i % 2 == 0 { 0x30 } else { 0xD0 };
        c.fill_rect(
            Rect::new(620 + 32 * i, 10, 32, 90),
            Color::rgb(shade, shade, shade),
        );
    }
    draw_marquee(&mut c, &p, Rect::new(640, 20, 280, 70), Color::rgb(0, 0, 0));
    c
}

/// Glass's toolbars and rebar, 950x150: three Explorer strips, the file dialog's small strip and a three-band rebar.
fn render_glass_toolbars(theme: &Theme) -> Canvas {
    use toolbar::{Item, ToolBar};
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let cm = theme.look.controls();
    let mut c = Canvas::new(950, 150);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let big = |hot: usize, pressed: Option<usize>, latched: Option<usize>, history: bool| {
        let button = |id, label: Option<&str>| Item::Button {
            id,
            icon: None,
            label: label.map(str::to_string),
            enabled: true,
        };
        ToolBar {
            items: vec![
                Item::DropButton {
                    id: toolbar::BACK,
                    icon: None,
                    label: Some("Back".into()),
                    enabled: history,
                },
                Item::DropButton {
                    id: toolbar::FORWARD,
                    icon: None,
                    label: Some(String::new()),
                    enabled: history,
                },
                button(toolbar::UP, None),
                Item::Separator,
                button(toolbar::FOLDERS, Some("Folders")),
                Item::Separator,
                Item::DropButton {
                    id: toolbar::VIEWS,
                    icon: Some(viewsicon::draw24()),
                    label: None,
                    enabled: true,
                },
            ],
            hot: Some(hot),
            pressed,
            latched,
            big: true,
        }
    };
    let one_band = rebar::Rebar {
        bands: vec![rebar::Band {
            height: toolbar::height(true, &cm),
        }],
    };
    let band_h = rebar::height(&one_band, &cm.rebar);
    let strips = [
        big(0, None, Some(4), true),
        big(4, Some(1), Some(4), true),
        big(6, Some(2), None, false),
    ];
    for (i, tb) in strips.iter().enumerate() {
        let rr = Rect::new(10, 10 + i as i32 * (band_h + 12), 420, band_h);
        rebar::draw(&mut c, &p, &one_band, rr);
        let band = rebar::layout(&one_band, rr, &cm.rebar)[0];
        let rects = toolbar::layout(tb, &text, px, band, &cm);
        toolbar::draw(&mut c, &text, &p, tb, band, &rects, px);
    }
    let small = ToolBar {
        items: vec![
            Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: None,
                enabled: true,
            },
            Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            Item::Separator,
            Item::Button {
                id: toolbar::NEW_FOLDER,
                icon: None,
                label: None,
                enabled: false,
            },
            Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw16()),
                label: None,
                enabled: true,
            },
        ],
        hot: Some(0),
        pressed: Some(1),
        latched: None,
        big: false,
    };
    let strip = Rect::new(460, 14, 200, toolbar::HEIGHT);
    let rects = toolbar::layout(&small, &text, px, strip, &cm);
    toolbar::draw(&mut c, &text, &p, &small, strip, &rects, px);
    let stack = rebar::Rebar {
        bands: [22, toolbar::height(true, &cm), 22]
            .map(|height| rebar::Band { height })
            .into(),
    };
    let sr = Rect::new(460, 60, 480, rebar::height(&stack, &cm.rebar));
    rebar::draw(&mut c, &p, &stack, sr);
    if let Some(st) = theme.look.style() {
        for band in rebar::layout(&stack, sr, &cm.rebar) {
            let grip = Rect::new(band.x + 3, band.y, 3, band.height);
            st.draw(
                &mut c,
                grip,
                reprisekit::style::Part::RebarGripper,
                reprisekit::style::State::Normal,
                1,
            );
        }
    }
    c
}

/// Glass's progress bars, trackbars and a dialog's ? box, 950x430, in each state.
fn render_glass_ranges(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let mut c = Canvas::new(950, 430);
    c.fill_rect(Rect::new(0, 0, 950, 110), theme.colors.button_face);
    c.fill_rect(Rect::new(0, 200, 950, 230), theme.colors.button_face);
    let bars = [
        (0.0, true, 18),
        (0.05, true, 18),
        (0.25, true, 18),
        (0.5, true, 18),
    ];
    let more = [(1.0, true, 18), (0.5, false, 18), (0.5, true, 16)];
    for (i, (f, enabled, h)) in bars.into_iter().chain(more).enumerate() {
        let r = Rect::new(10 + 133 * i as i32, 10, 120, h);
        draw_progress(&mut c, &p, r, f, enabled);
    }
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (enabled, focused, ptr)) in [
        (true, false, idle),
        (true, false, hot),
        (true, false, held),
        (true, true, idle),
        (false, false, idle),
    ]
    .into_iter()
    .enumerate()
    {
        let lay = trackbar::layout(
            &trackbar::TrackBar::new(4, 11),
            10 + 180 * i as i32,
            50,
            160,
        );
        draw_trackbar(&mut c, &p, &lay, enabled, focused, ptr);
    }
    let stripes = blurred_stripes(950, 90);
    c.blit_canvas(0, 110, &stripes);
    use CaptionButton::Close;
    for (i, (active, help, hot, pressed)) in [
        (true, idle, None, None),
        (true, hot, None, None),
        (true, held, None, None),
        (true, idle, Some(Close), None),
        (true, idle, None, Some(Close)),
        (false, idle, None, None),
        (false, hot, None, None),
        (false, idle, Some(Close), None),
    ]
    .into_iter()
    .enumerate()
    {
        let g = FrameGeometry::layout(&theme.metrics, 100, 2, false, Chrome::DIALOG);
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        let st = FrameState {
            title: "Run",
            active,
            maximized: false,
            pressed,
            glow: Glow::lit(hot),
            icon: None,
            tabbed: false,
        };
        look::draw_dialog_frame(&mut f, theme, &text, &g, &st, Some(help));
        let (x, y) = (10 + 117 * i as i32, 125);
        c.blit_canvas(x, y, &f);
    }
    for (i, (enabled, focused, ptr)) in [
        (true, false, idle),
        (true, false, hot),
        (true, false, held),
        (true, true, idle),
        (false, false, idle),
    ]
    .into_iter()
    .enumerate()
    {
        let lay = trackbar::layout_vertical(
            &trackbar::TrackBar::new(14, 21).paged(4, 2),
            10 + 60 * i as i32,
            215,
            81,
        );
        draw_vertical_trackbar(&mut c, &p, &lay, enabled, focused, ptr);
    }
    c
}

/// Glass's menus on white: the menu bars and context menu of `gallery_p3::menu`, and a desktop menu with its View submenu open.
fn render_glass_menus(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 290);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    gallery_p3::menu::draw(&mut c, theme, &text, Rect::new(0, 0, 520, 250));
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(40, 120, 220));
    let mut view = MenuTree::new();
    view.push(MenuItem::item("&Large icons").radio(true));
    view.push(MenuItem::item("&Medium icons").radio(true).checked(true));
    view.push(MenuItem::item("&Small icons").radio(true));
    view.push(MenuItem::Separator);
    view.push(MenuItem::item("&Auto arrange icons"));
    view.push(MenuItem::item("A&lign icons to grid").checked(true));
    view.push(MenuItem::Separator);
    view.push(MenuItem::item("&Show desktop icons").checked(true));
    let mut desk = MenuTree::new();
    desk.push_sub(MenuItem::item("&View").submenu(true), view);
    desk.push_sub(MenuItem::item("S&ort by").submenu(true), MenuTree::new());
    desk.push(MenuItem::item("R&efresh"));
    desk.push(MenuItem::Separator);
    desk.push(MenuItem::item("&Paste").enabled(false));
    desk.push(MenuItem::item("Paste &shortcut").enabled(false));
    desk.push(MenuItem::item("&Undo Delete").shortcut(Some("Ctrl+Z".into())));
    desk.push(MenuItem::Separator);
    desk.push(MenuItem::item("Screen &resolution").icon(Some(icon.clone())));
    desk.push(MenuItem::item("Personali&ze").icon(Some(icon)));
    let cx = MenuCtx {
        text: &text,
        theme,
        px: theme.fonts.size_px,
        within: c.bounds(),
    };
    let mut chain = PopupMenu::open(&desk, (540, 10), &cx, true);
    chain.key(&desk, MenuKey::Right, &cx);
    for _ in 0..5 {
        chain.key(&desk, MenuKey::Down, &cx);
    }
    chain.draw(&mut c, theme, &text, true);
    c
}

/// Glass's balloons, 950x250, over white, grey and the stripes: at rest, with action buttons under a Top bar, a balloon tip and a reply row.
fn render_glass_balloons(theme: &Theme) -> Canvas {
    use reprisekit::controls::tooltip::{self, NotifyHit};
    use reprisekit::taskbar::Edge;
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(950, 250);
    c.fill_rect(Rect::new(0, 0, 320, 250), Color::rgb(0xFF, 0xFF, 0xFF));
    c.fill_rect(Rect::new(320, 0, 320, 250), Color::rgb(0x80, 0x80, 0x80));
    c.blit_canvas(640, 0, &blurred_stripes(310, 250));
    let icon = reprisekit::icon::fallback(theme, 16);
    let lay = tooltip::notify_layout(
        theme,
        &text,
        "Updates are ready",
        "Click here to install them. Your computer may restart once they are in.",
        &[],
        None,
    );
    let rest = tooltip::draw_notify_balloon(theme, &text, &lay, Some(&icon), None, None, None);
    c.blit_over(8, 8, &rest);
    let lay = tooltip::notify_layout(
        theme,
        &text,
        "New message",
        "From Ann: Lunch at noon?",
        &["Reply", "Mark as read"],
        None,
    )
    .on_edge(Edge::Top);
    for (x, hot) in [(328, NotifyHit::Close), (644, NotifyHit::Button(1))] {
        let lit =
            tooltip::draw_notify_balloon(theme, &text, &lay, Some(&icon), None, Some(hot), None);
        c.blit_over(x, 8, &lit);
    }
    let reply = tooltip::notify_layout(
        theme,
        &text,
        "New message",
        "From Ann: Lunch at noon?",
        &["Reply", "Mark as read"],
        Some(tooltip::NotifyReply {
            placeholder: "Write a reply",
            send: "Send",
        }),
    );
    let typed = reprisekit::textedit::TextEdit::new("Sounds good", false);
    let caret = reprisekit::textedit::Caret::Shown;
    let open = tooltip::draw_notify_balloon(
        theme,
        &text,
        &reply,
        Some(&icon),
        None,
        None,
        Some((&typed, caret, None)),
    );
    c.blit_over(328, 130, &open);
    let tip = tooltip::draw_balloon_tip(
        theme,
        &text,
        "Turn Off",
        "Ends your session and shuts down the computer so that you can safely turn off power.",
    );
    c.blit_over(8, 140, &tip);
    c
}

/// The Glass clock popup at `pct` text over the stripes, at 10:09:30.
fn render_glass_clock_popup(pct: u32) -> Canvas {
    use reprisekit::controls::calendar;
    let theme = interfaces::theme_for(Interface::Glass).with_text_size(pct);
    let text = TextRenderer::load(&FontConfig::default());
    let mut cal = calendar::Calendar::new(2026, 10, None, 0);
    cal.today = Some((2026, 10, 16));
    cal.now = Some((10, 9, 30));
    cal.time = Some("10:09".into());
    let panel = calendar::draw_popup(&theme, &text, &cal, calendar::PopupHit::Panel);
    let mut c = blurred_stripes(panel.width as i32 + 40, panel.height as i32 + 40);
    c.blit_over(20, 20, &panel);
    c
}

/// The flyouts as windows over the stripes: calendar, notification history, clipboard history, emoji panel and a colour button's popup.
fn render_glass_flyouts(theme: &Theme) -> Canvas {
    use reprisekit::controls::cliphistory::{self, ClipHistory, Row};
    use reprisekit::controls::{calendar, emojipicker};
    use reprisekit::emoji::{EmojiFont, Entry, catalog};
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut c = blurred_stripes(950, 840);
    let mut cal = calendar::Calendar::new(2026, 10, None, 0);
    cal.today = Some((2026, 10, 6));
    let m = reprisekit::flyout::margin(theme);
    let at =
        |c: &mut Canvas, x: i32, y: i32, panel: &Canvas| c.blit_over(x - m.left, y - m.top, panel);
    // The clock popup at 10:09:30 with the previous-month arrow lit, and at 3:45:15 PM.
    for (x, now, time, hot) in [
        (20, (10, 9, 30), "10:09", calendar::PopupHit::Prev),
        (420, (15, 45, 15), "3:45 PM", calendar::PopupHit::Panel),
    ] {
        cal.now = Some(now);
        cal.time = Some(time.into());
        at(
            &mut c,
            x,
            620,
            &calendar::draw_popup(theme, &text, &cal, hot),
        );
    }
    at(
        &mut c,
        250,
        20,
        &notifyhistory::draw(theme, &text, &notifyhistory::sample()),
    );
    let rows = ["cargo build --release", "Image, 1920x1080", "hello world"]
        .into_iter()
        .enumerate()
        .map(|(i, label)| Row {
            id: i as u64,
            label: label.into(),
            icon: None,
        })
        .collect();
    let clip = ClipHistory::new(rows);
    at(
        &mut c,
        20,
        330,
        &cliphistory::draw(theme, &text, &clip, true, None),
    );
    let font = EmojiFont::load();
    let recent = ["👍", "🎉"].into_iter().filter_map(Entry::get).collect();
    let picker = emojipicker::EmojiPicker::new(recent, catalog(font.as_ref()));
    at(
        &mut c,
        430,
        330,
        &emojipicker::draw(theme, &text, font.as_ref(), &picker, true, None),
    );
    let mut b = colorbutton::ColorButton::new(Color::rgb(58, 110, 165), schemes::PALETTE.to_vec());
    b.toggle();
    let button = Rect::new(820, 330, 75, 21);
    c.fill_rect(Rect::new(810, 320, 130, 190), theme.colors.button_face);
    colorbutton::draw(&mut c, &theme.paint(), button, &b);
    colorbutton::draw_popup(&mut c, &text, &theme.paint(), button, &b, px);
    c
}

/// Glass's item boxes on white: Details list, Folders tree, icon view and desktop icons over dark and light wallpapers.
fn render_glass_items(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let cm = theme.look.controls();
    let mut c = Canvas::new(950, 230);
    c.fill_rect(c.bounds(), theme.colors.window);
    let rows = |n: usize| -> Vec<listview::Row> {
        (0..n)
            .map(|i| listview::Row {
                icon: None,
                cells: vec![format!("Item {i}"), format!("{i} KB")],
            })
            .collect()
    };
    for (k, inactive) in [false, true].into_iter().enumerate() {
        let mut lv = listview::ListView {
            columns: vec![
                listview::Column::new("Name", 110),
                listview::Column::new("Size", 60),
            ],
            rows: rows(7),
            focus_ring: true,
            inactive,
            ..listview::ListView::default()
        };
        lv.selection.set.extend([1, 2]);
        lv.selection.focus = Some(if inactive { 5 } else { 2 });
        lv.hot = Some(4);
        let r = Rect::new(10 + k as i32 * 210, 10, 200, 150);
        let lay = listview::layout(&lv, r, &cm);
        listview::draw(&mut c, &text, &p, &lv, &lay, lv.bars.lit(), px);
    }
    let mut desk = Node::new("Desktop", true);
    desk.expanded = true;
    let mut docs = Node::new("Documents", true);
    docs.expanded = true;
    docs.children.push(Node::new("Letters", false));
    desk.children.push(docs);
    desk.children.push(Node::new("Music", true));
    desk.children.push(Node::new("Pictures", true));
    let mut tv = TreeView::new(vec![desk], treeview::Style::Explorer);
    tv.selected = Some(vec![0, 2]);
    let tl = treeview::layout(&tv, &text, px, Rect::new(430, 10, 160, 150), &cm);
    let mid = tl.body.y + 17 + 8;
    if let Some(x) =
        (tl.body.x..tl.body.x + 40).find(|&x| tv.hit(&tl, x, mid) == treeview::Hit::Expander(1))
    {
        tv.track(&tl, x, mid);
    }
    treeview::draw(&mut c, &text, &p, &tv, &tl, tv.bars.lit(), px);
    let mut sv = shellview::sample(shellview::Mode::Icons, false);
    sv.hot = Some(1);
    let sl = shellview::layout(&sv, Rect::new(600, 10, 340, 150), &cm);
    shellview::draw(&mut c, &text, &p, &sv, &sl, sv.bars.lit(), px);
    for (k, bg) in [Color::rgb(0x1F, 0x4E, 0x79), Color::rgb(0xB8, 0xD8, 0xF0)]
        .into_iter()
        .enumerate()
    {
        let at = Rect::new(10 + k as i32 * 100, 165, 90, 60);
        c.fill_rect(at, bg);
        let pick = Selection {
            ring: false,
            origin: (at.x + 7, at.y - 2),
        };
        iconview::draw_cell_at(
            &mut c,
            (at.x + 7, at.y - 2),
            iconview::Cell::square(iconview::CELL),
            theme,
            &text,
            "Computer",
            None,
            false,
            Some(pick),
            None,
        );
    }
    c
}

fn render_glass_popup_parts(theme: &Theme) -> Canvas {
    use reprisekit::controls::{calendar, infotip, tooltip};
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let mut c = Canvas::new(950, 200);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let backs = [
        Color::rgb(0, 0, 0),
        Color::rgb(0x80, 0x80, 0x80),
        Color::rgb(0xFF, 0xFF, 0xFF),
    ];
    for (i, back) in backs.into_iter().enumerate() {
        c.fill_rect(Rect::new(0, 50 * i as i32, 350, 50), back);
    }
    c.blit_canvas(0, 150, &blurred_stripes(350, 50));
    let lines: Vec<String> = ["Dimensions: 800 x 600", "Type: JPEG Image"]
        .iter()
        .map(|l| l.to_string())
        .collect();
    for i in 0..4 {
        let y = 50 * i + 8;
        c.blit_over(
            8,
            y,
            &tooltip::draw_tip(theme, &text, "Contains the files you deleted"),
        );
        if i == 3 {
            c.blit_over(200, y - 6, &infotip::draw(theme, &text, &lines, 1));
        } else {
            c.blit_over(
                200,
                y - 4,
                &tooltip::draw_tip(theme, &text, "Volume\nMuted"),
            );
        }
    }
    for (i, enabled) in [true, false].into_iter().enumerate() {
        let mut cal = calendar::Calendar::new(2026, 10, Some(4), 0);
        cal.enabled = enabled;
        let lay = calendar::layout(
            Rect::new(370 + i as i32 * 180, 10, calendar::W, calendar::H),
            &theme.controls(),
        );
        calendar::draw(&mut c, &text, &p, &cal, &lay, false, px);
    }
    // Left bar: the first place lit, the second selected; right bar: the first held, the
    // second selected under the pointer.
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(3, 5, 26, 22), Color::rgb(0xE8, 0xC0, 0x50));
    let place = |l: &str| placesbar::Place {
        label: l.into(),
        icon: Some(icon.clone()),
        enabled: true,
    };
    let tile = theme.look.controls().places.tile;
    for (i, (selected, hot, pressed)) in [(1, 0, None), (1, 1, Some(0))].into_iter().enumerate() {
        let places = placesbar::PlacesBar {
            places: vec![place("My Recent Documents"), place("Desktop")],
            selected: Some(selected),
            hot: Some(hot),
            pressed,
        };
        let r = Rect::new(740 + i as i32 * 95, 10, placesbar::WIDTH, 2 * tile);
        placesbar::draw(&mut c, &text, &p, &places, r, px);
    }
    c
}

/// The volume popup's now-playing strip in each look: playing with an artist, Pause focused (top); paused with a long title, Previous greyed and Next held (bottom).
fn render_now_playing() -> Canvas {
    use volumepopup::{NowPlaying, Transport};
    let (w, h) = (480, 430);
    let mut c = Canvas::new(3 * w as u32, 2 * h as u32);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    for (i, look) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = interfaces::theme_for(look);
        let text = TextRenderer::load(&theme.fonts);
        let playing = NowPlaying {
            title: "Evening Mix".into(),
            artist: "The Late Show Band".into(),
            playing: true,
            can_previous: true,
            can_next: true,
            can_play: true,
            can_pause: true,
        };
        let paused = NowPlaying {
            title: "A Very Long Track Title That Will Not Fit".into(),
            artist: String::new(),
            playing: false,
            can_previous: false,
            ..playing.clone()
        };
        for (row, np) in [playing, paused].into_iter().enumerate() {
            let mut vp = volumepopup::VolumePopup::new(&volumepopup::Mix {
                playing: Some(np),
                ..sample_mix(1)
            });
            let bounds = Rect::new(i as i32 * w, row as i32 * h, w, h);
            let anchor = (bounds.x + w / 2, bounds.bottom() - 4);
            let edge = reprisekit::taskbar::Edge::Bottom;
            let lay = volumepopup::layout(&vp, &theme, &text, anchor, edge, bounds);
            if row == 0 {
                vp.focus = volumepopup::Focus::Media(Transport::PlayPause);
                vp.cues = true;
            } else if let Some(st) = &lay.strip {
                let next = st.button(Transport::Next);
                vp.press(&lay, next.x + 4, next.y + 4);
            }
            volumepopup::draw(&mut c, &theme, &text, &vp, &lay);
        }
    }
    c
}

fn sample_mix(apps: usize) -> volumepopup::Mix {
    let names = [
        ("Firefox", 16, false, Color::rgb(230, 110, 30)),
        ("mpv", 8, true, Color::rgb(110, 60, 140)),
        (
            "Music Player: Evening Mix",
            12,
            false,
            Color::rgb(40, 150, 90),
        ),
        ("Video Call", 20, false, Color::rgb(50, 110, 200)),
        ("Game", 6, false, Color::rgb(200, 40, 50)),
        ("Podcasts", 14, false, Color::rgb(150, 60, 190)),
        ("Terminal Bell", 10, false, Color::rgb(60, 60, 60)),
    ];
    let mut speaker = Canvas::new(32, 32);
    speaker.fill_rect(Rect::new(2, 2, 28, 28), Color::rgb(85, 85, 85));
    volumepopup::Mix {
        master: volumepopup::Level {
            pos: 14,
            mute: false,
            live: true,
            icon: Some(speaker),
        },
        outputs: vec!["Speakers".into(), "Headphones".into()],
        output: Some(0),
        inputs: vec!["Microphone".into()],
        input: Some(0),
        playing: None,
        apps: names
            .iter()
            .take(apps)
            .map(|&(name, pos, mute, colour)| {
                let mut icon = Canvas::new(32, 32);
                icon.fill_rect(Rect::new(2, 2, 28, 28), colour);
                volumepopup::AppLevel {
                    key: name.to_lowercase(),
                    label: name.into(),
                    icon: Some(icon),
                    pos,
                    mute,
                }
            })
            .collect(),
    }
}

/// The speaker's volume popup in `theme` at scale `n`, 950x840: seven programs scrolled by one
/// with a player's strip, then two programs with the focus ring on a thumb, and the Output list
/// open.
fn render_volume_popup(theme: &Theme, n: i32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    // As a program draws: logical lengths on a canvas `n` device px each, the look's parts at `n`.
    let theme = &Theme {
        look: theme.look.at_scale(n),
        ..theme.clone()
    };
    let mut c = Canvas::with_scale(950, 840, n);
    c.fill_rect(c.bounds(), theme.colors.background);
    let edge = reprisekit::taskbar::Edge::Bottom;
    let mut seven = sample_mix(7);
    seven.playing = Some(volumepopup::NowPlaying {
        title: "Evening Mix".into(),
        artist: "The Band".into(),
        playing: true,
        can_previous: true,
        can_next: true,
        can_play: true,
        can_pause: true,
    });
    let mut vp = volumepopup::VolumePopup::new(&seven);
    vp.first = 1;
    let lay = volumepopup::layout(&vp, theme, &text, (475, 455), edge, c.bounds());
    volumepopup::draw(&mut c, theme, &text, &vp, &lay);
    let mut vp = volumepopup::VolumePopup::new(&sample_mix(2));
    vp.focus = volumepopup::Focus::Thumb(1);
    vp.cues = true;
    let lay = volumepopup::layout(&vp, theme, &text, (237, 830), edge, c.bounds());
    volumepopup::draw(&mut c, theme, &text, &vp, &lay);
    let mut m = sample_mix(2);
    m.outputs = (1..=10).map(|i| format!("Output {i}")).collect();
    let mut vp = volumepopup::VolumePopup::new(&m);
    vp.output.toggle();
    let lay = volumepopup::layout(&vp, theme, &text, (712, 830), edge, c.bounds());
    volumepopup::draw(&mut c, theme, &text, &vp, &lay);
    c
}

/// The volume popup's pages at scale `n`: Classic, and Blue and Glass `with_blue`.
fn write_volume_popups(with_blue: bool, n: i32) {
    let at = if n == 1 {
        String::new()
    } else {
        format!("-x{n}")
    };
    let mut looks = vec![("", Interface::Classic)];
    if with_blue {
        looks.extend([("-blue", Interface::Blue), ("-glass", Interface::Glass)]);
    }
    for (name, look) in looks {
        write_preview(
            &render_volume_popup(&interfaces::theme_for(look), n),
            &format!("reprisekit-gallery-volume-popup{name}{at}.png"),
        );
    }
}

/// Glass's push buttons, group box and tabs on its face, 950x160.
fn render_glass_sheet(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 160);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (label, default, ptr, enabled, focus)) in [
        ("Normal", false, idle, true, false),
        ("Hot", false, hot, true, false),
        ("Pressed", false, held, true, false),
        ("Disabled", false, idle, false, false),
        ("Default", true, idle, true, false),
        ("Focused", false, idle, true, true),
        ("Both", true, idle, true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(10 + 85 * i as i32, 10, 75, 23);
        draw_button(
            &mut c, &text, &p, r, label, default, ptr, enabled, focus, px,
        );
    }
    let inactive = Paint { active: false, ..p };
    let r = Rect::new(10 + 85 * 7, 10, 75, 23);
    draw_button(
        &mut c, &text, &inactive, r, "Inactive", true, idle, true, false, px,
    );
    let wide = Rect::new(10, 45, 160, 23);
    draw_button(
        &mut c, &text, &p, wide, "Wide", false, idle, true, false, px,
    );
    let tall = Rect::new(180, 45, 75, 50);
    draw_button(
        &mut c, &text, &p, tall, "Tall", false, idle, true, false, px,
    );
    let g = Rect::new(270, 50, 180, 90);
    let lr = Rect::new(g.x + 10, g.y - 5, 60, 13);
    draw_group_box(&mut c, &text, &p, g, lr, "&Group box", px);
    let mut strip = tabstrip::TabStrip::new(&["First", "Second", "Third"], 0);
    strip.hot = Some(2);
    strip.focused = true;
    let page = Rect::new(470, 65, 300, 85);
    let tabs = tabstrip::layout(
        &text,
        px,
        &strip,
        page.x,
        page.y - 20,
        page.width,
        reprisekit::theme::TextUnits::ONE,
    );
    draw_page(&mut c, &p, page);
    tabstrip::draw(&mut c, &text, &p, &strip, &tabs, page, true, px);
    let pg = Rect::new(page.x + 10, page.y + 20, 180, 55);
    let lr = Rect::new(pg.x + 10, pg.y - 5, 60, 13);
    draw_group_box(&mut c, &text, &p.on_page(page), pg, lr, "On the page", px);
    c
}

/// The list-mode property sheet in each look: four headings over nine pages, the third open, the list focused.
fn render_list_sheets() -> Canvas {
    let groups: &[(&str, &[&str])] = &[
        ("Look", &["Night Light", "Fonts & Icons", "Effects"]),
        ("Desktop", &["Taskbar", "Start Menu"]),
        ("Input", &["Keyboard", "Mouse"]),
        ("System", &["Power", "Advanced"]),
    ];
    render_list_sheets_of(groups, 2)
}

/// The list sheets with 24 pages, the last open and scrolled into view.
fn render_long_list_sheets() -> Canvas {
    let groups: &[(&str, &[&str])] = &[
        (
            "Look",
            &[
                "Appearance",
                "Background",
                "Screen Saver",
                "Display",
                "Night Light",
                "Fonts & Icons",
                "Effects",
            ],
        ),
        (
            "Desktop",
            &[
                "Taskbar",
                "Start Menu",
                "Launcher",
                "Window Switcher",
                "Desktops",
                "Windows",
            ],
        ),
        ("Input", &["Keyboard", "Mouse", "Pointers", "Accessibility"]),
        (
            "System",
            &[
                "Power",
                "Startup",
                "Default Programs",
                "Sound",
                "Network",
                "Updates",
                "Advanced",
            ],
        ),
    ];
    render_list_sheets_of(groups, 23)
}

/// `groups` as list sheets in each look, page `open` open and revealed.
fn render_list_sheets_of(groups: &[(&str, &[&str])], open: usize) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let count = groups.iter().map(|g| g.1.len()).sum::<usize>();
    let page = || {
        form::Form::new(vec![
            Item::group("group", Rect::new(20, 44, 358, 84), "Menus"),
            Item::check("check", 32, 66, "&Animate windows opening and closing"),
            Item::button(
                "button",
                Rect::new(291, 96, BUTTON_W, BUTTON_H),
                "Ad&vanced...",
                true,
                false,
            ),
        ])
    };
    let mut c = Canvas::new(564, 1312);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    let mut y = 10;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = interfaces::theme_for(i);
        let pages = (0..count).map(|_| page()).collect();
        let mut sheet = PropSheet::with_list(groups, pages, (398, 424));
        sheet.tabs.open = open;
        sheet.reveal_open();
        sheet.tabs.focused = true;
        sheet.open_mut().set_focus(None);
        sheet.set_apply_enabled(true);
        let (w, h) = sheet.size;
        let mut sc = Canvas::new(w as u32, h as u32);
        sc.fill_rect(sc.bounds(), theme.colors.button_face);
        propsheet::draw(
            &mut sc,
            &text,
            &theme.paint(),
            &sheet,
            true,
            theme.fonts.size_px,
        );
        c.blit_canvas(10, y, &sc);
        y += h + 10;
    }
    c
}

/// The tray's Customize dialog client in Classic and Blue side by side, Glass's under them; Classic's combo list is open.
fn render_tray_customize() -> Canvas {
    use reprisekit::controls::traycustomize::{Behaviour, Entry, TrayCustomize, Wording};
    use reprisekit::icon::IconSource;
    let text = TextRenderer::load(&FontConfig::default());
    let swatch = |argb: u32| -> std::sync::Arc<[IconSource]> {
        std::sync::Arc::from([IconSource {
            width: 16,
            height: 16,
            pixels: vec![argb; 256],
        }])
    };
    let entry = |key: &str, name: &str, argb, passive, behaviour| Entry {
        key: key.into(),
        name: name.into(),
        detail: format!("{name}: all is well"),
        icon: swatch(argb),
        passive,
        behaviour,
    };
    let mut c = Canvas::new(1204, 1006);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    let (mut x, mut y) = (10, 10);
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = interfaces::theme_for(i);
        let w = Wording::of(&theme);
        let mut t = TrayCustomize::new(
            w,
            vec![
                entry("nm-applet", "Network", 0xFF20_60C0, false, Behaviour::Auto),
                entry("volume", "Volume", 0xFF40_A040, true, Behaviour::Hide),
            ],
            vec![entry(
                "sticky",
                "Sticky Keys",
                0xFF80_8080,
                false,
                Behaviour::Show,
            )],
            (w == Wording::Glass).then_some(false),
            theme.look.controls(),
        );
        t.select(1);
        if i == Interface::Classic {
            t.combo.toggle();
        }
        let px = theme.fonts.size_px;
        let lay = t.layout(&text, px, (0, 0));
        let (cw, ch) = t.client();
        let mut sc = Canvas::new(cw as u32, ch as u32);
        t.draw(
            &mut sc,
            &text,
            &theme.paint(),
            &lay,
            reprisekit::cues::Cues::NONE,
            px,
        );
        if i == Interface::Glass {
            (x, y) = (10, 411);
        }
        c.blit_canvas(x, y, &sc);
        x += cw + 10;
    }
    c
}

/// Glass's fields on its face, 950x150: edits, drop-down lists, editable combos and spinners in each state, then every frame of the field, list and spin-button parts.
fn render_glass_fields(theme: &Theme) -> Canvas {
    use reprisekit::controls::updown;
    use reprisekit::look::widgets::draw_edit_field;
    use reprisekit::style::{Part, State};
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 150);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let edits = [
        (false, Interaction::Idle),
        (false, Interaction::Hot),
        (true, Interaction::Idle),
    ];
    for (i, (focused, ptr)) in edits.into_iter().enumerate() {
        let r = Rect::new(10 + i as i32 * 80, 10, 75, 21);
        let inner = draw_edit_field(&mut c, &p, r, theme.colors.window, focused, ptr);
        text.draw(
            &mut c,
            inner,
            "Edit",
            false,
            theme.colors.button_text,
            Align::Left,
            px,
        );
    }
    let greyed = form::Form::new(vec![Item::edit(
        "greyed",
        Rect::new(250, 10, 75, 21),
        "Greyed",
        false,
    )]);
    form::draw(&mut c, &text, &p, &greyed, false, px);
    let idle = combo::ComboLit::IDLE;
    let lit = |frame, drop| combo::ComboLit { frame, drop };
    let combos = [
        (340, 10, false, false, true, idle),
        (490, 10, false, true, true, idle),
        (640, 10, false, false, false, idle),
        (
            790,
            10,
            false,
            false,
            true,
            lit(Interaction::Hot, Interaction::Hot),
        ),
        (10, 45, true, false, true, idle),
        (160, 45, true, true, true, idle),
        (
            490,
            45,
            true,
            false,
            true,
            lit(Interaction::Hot, Interaction::Idle),
        ),
        (
            640,
            45,
            true,
            false,
            true,
            lit(Interaction::Hot, Interaction::Hot),
        ),
        (
            790,
            45,
            false,
            false,
            true,
            lit(Interaction::Pressed, Interaction::Pressed),
        ),
    ];
    for (x, y, editable, focus, enabled, lit) in combos {
        let mut cb = combo::Combo::new(&["(None)", "Blank"], 0, 8);
        cb.enabled = enabled;
        if editable {
            cb.edit = Some(reprisekit::textedit::TextEdit::new("notepad", false));
        }
        let r = Rect::new(x, y, 140, combo::COMBO_H);
        let lay = combo::layout(&cb, r, &cm);
        combo::draw(
            &mut c,
            &text,
            &p,
            &cb,
            &lay,
            focus,
            focus,
            Caret::Unfocused,
            lit,
            px,
        );
    }
    for (i, enabled) in [true, false].into_iter().enumerate() {
        let mut ud = updown::UpDown::new(10, 1, 9999);
        ud.enabled = enabled;
        let lay = updown::layout(310 + i as i32 * 60, 45);
        updown::draw(&mut c, &text, &p, &ud, &lay, false, None, px);
    }
    let st = theme.look.style().unwrap();
    let states = [State::Normal, State::Hot, State::Pressed, State::Disabled];
    let mut x = 10;
    for state in [State::Normal, State::Hot, State::Focused, State::Disabled] {
        let r = Rect::new(x, 100, 40, 21);
        c.fill_rect(r, theme.colors.window);
        st.draw(&mut c, r, Part::EditBorder, state, 1);
        x += 46;
    }
    for state in states {
        st.draw(&mut c, Rect::new(x, 100, 60, 21), Part::ComboList, state, 1);
        x += 66;
    }
    for state in states {
        let field = Rect::new(x, 100, 40, 21);
        c.fill_rect(field, theme.colors.window);
        st.draw(&mut c, field, Part::EditBorder, State::Normal, 1);
        let drop = Rect::new(
            field.right() - 2 - cm.combo_drop,
            field.y + 2,
            cm.combo_drop,
            17,
        );
        st.draw(&mut c, drop, Part::ComboDrop, state, 1);
        x += 46;
    }
    for part in [Part::SpinUp, Part::SpinDown] {
        for state in states {
            st.draw(&mut c, Rect::new(x, 100, 15, 8), part, state, 1);
            x += 20;
        }
    }
    c
}

/// Glass's open combo lists, 950x110: a drop-down list on white, and a scrolling one on the face.
fn render_glass_open_lists(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 110);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    c.fill_rect(Rect::new(0, 0, 320, 110), theme.colors.window);
    let lists: [(Rect, &[&str], usize, usize, usize); 2] = [
        (
            Rect::new(10, 10, 279, 23),
            &[
                "Show icon and notifications",
                "Hide icon and notifications",
                "Only show notifications",
            ],
            3,
            0,
            1,
        ),
        (
            Rect::new(340, 10, 200, 21),
            &["(None)", "Blank", "Bubbles", "Mystify", "Photos", "Ribbons"],
            4,
            1,
            3,
        ),
    ];
    for (r, items, rows, top, hot) in lists {
        let mut cb = combo::Combo::new(items, 0, rows);
        cb.toggle();
        cb.top = top;
        cb.open = Some(hot);
        let lay = combo::layout(&cb, r, &cm);
        let lit = combo::ComboLit {
            frame: Interaction::Pressed,
            drop: Interaction::Pressed,
        };
        combo::draw(
            &mut c,
            &text,
            &p,
            &cb,
            &lay,
            true,
            false,
            Caret::Unfocused,
            lit,
            px,
        );
        combo::draw_list(
            &mut c,
            &text,
            &p,
            &cb.items,
            cb.open,
            cb.top,
            cb.rows,
            lay.list,
            lay.scrollbar,
            None,
            px,
        );
    }
    c
}

/// Glass's dialog controls on its face, 950x120: check box, radio, static label and push button at rest and greyed, then every frame of the check box and radio parts.
fn render_glass_choices(theme: &Theme) -> Canvas {
    use reprisekit::style::{Check, Part, State};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 120);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let choices = form::Form::new(vec![
        Item::check("check", 10, 10, "&Check box").checked(true),
        Item::check("greyed_check", 10, 34, "&Greyed check box")
            .checked(true)
            .enabled(false),
        Item::radio("radio", 200, 10, "&Radio", "radio").checked(true),
        Item::radio("greyed_radio", 200, 34, "Gr&eyed radio", "greyed")
            .checked(true)
            .enabled(false),
        Item::label("greyed_label", Rect::new(380, 12, 120, 13), "Greyed label:").enabled(false),
        Item::button("ok", Rect::new(520, 8, 75, 23), "OK", true, false),
        Item::button("apply", Rect::new(605, 8, 75, 23), "&Apply", false, false),
    ]);
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &choices,
        false,
        theme.fonts.size_px,
    );
    let st = theme.look.style().unwrap();
    let states = [State::Normal, State::Hot, State::Pressed, State::Disabled];
    let mut x = 10;
    for (part, checks) in [
        (
            Part::CheckBox,
            &[Check::Unchecked, Check::Checked, Check::Mixed][..],
        ),
        (Part::Radio, &[Check::Unchecked, Check::Checked][..]),
    ] {
        for &check in checks {
            for state in states {
                st.draw_check(&mut c, Rect::new(x, 80, 13, 13), part, check, state, 1);
                x += 20;
            }
            x += 10;
        }
    }
    c
}

/// [`render_glass`]'s frames, taskbars and flyout, 950x1120.
fn render_glass_frames() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let glass_theme = interfaces::theme_for(Interface::Glass);
    let style = glass_theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 1120);
    let mut glass_frame = |x: i32, y: i32, cw: i32, ch: i32, chrome: Chrome, st: &FrameState| {
        let g = FrameGeometry::layout(&glass_theme.metrics, cw, ch, st.maximized, chrome);
        let frame = Rect::new(x, y, g.total.0, g.total.1);
        for (i, p) in shadow::place(frame, st.active, st.maximized)
            .iter()
            .enumerate()
        {
            c.blit_canvas(
                p.dest.x,
                p.dest.y,
                &stretched(&shadow::piece(st.active, i, 1), p),
            );
        }
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, &glass_theme, &text, &caption_text, &g, st);
        f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        c.blit_canvas(x, y, &f);
    };
    let st = |active: bool,
              maximized: bool,
              hot: Option<CaptionButton>,
              pressed: Option<CaptionButton>| FrameState {
        title: "Notes",
        active,
        maximized,
        pressed,
        glow: Glow::lit(hot),
        icon: None,
        tabbed: false,
    };
    let win = Chrome::WINDOW;
    glass_frame(15, 15, 424, 162, win, &st(true, false, None, None));
    glass_frame(485, 15, 424, 162, win, &st(false, false, None, None));
    glass_frame(15, 260, 910, 150, win, &st(true, true, None, None));
    use CaptionButton::{Close, Maximize, Minimize};
    for (i, b) in [Minimize, Maximize, Close].into_iter().enumerate() {
        let x = 15 + 310 * i as i32;
        glass_frame(x, 435, 284, 32, win, &st(true, false, Some(b), None));
        glass_frame(x, 515, 284, 32, win, &st(true, false, None, Some(b)));
    }
    glass_frame(15, 595, 284, 32, win, &st(false, false, Some(Close), None));
    glass_frame(
        325,
        605,
        300,
        40,
        win,
        &st(true, true, Some(Maximize), None),
    );
    let dialog = Chrome::DIALOG;
    glass_frame(635, 595, 284, 32, dialog, &st(true, false, None, None));
    glass_frame(
        325,
        955,
        300,
        40,
        win,
        &st(false, true, Some(Maximize), None),
    );
    glass_frame(
        635,
        955,
        300,
        40,
        win,
        &st(true, true, None, Some(Maximize)),
    );
    for (i, glow) in [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.25, 0.75]]
        .into_iter()
        .enumerate()
    {
        let st = FrameState {
            glow: Glow(glow),
            ..st(true, false, None, None)
        };
        glass_frame(15 + 310 * i as i32, 1040, 284, 32, win, &st);
    }

    {
        use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
        let start_text = TextRenderer::load(&style.start_text.font());
        let clock = "3:45 PM\n9/27/2026";
        let clock_w = clock
            .lines()
            .map(|l| text.measure(l, false, glass_theme.fonts.size_px))
            .max()
            .unwrap_or(0);
        let layout = TaskbarLayout::compute(
            &glass_theme.metrics,
            &style.taskbar,
            930,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 3,
                tray: 2,
                chevron: false,
                show_desktop: false,
                ..TaskbarCounts::default()
            },
            clock_w,
        );
        for (i, y) in [690, 745].into_iter().enumerate() {
            let hot = i == 1;
            let entry = |title: &str, active: bool| TaskEntry {
                title: title.into(),
                active,
                attention: false,
                icon: None,
                glow: None,
            };
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: None,
                start_hot: hot,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![],
                quick_hot: None,
                quick_pressed: None,
                quick_drop: None,
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: hot.then_some(2),
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                tasks: vec![
                    entry("Notes", false),
                    entry("Explorer", true),
                    TaskEntry {
                        glow: Some(Color::rgb(0xF0, 0xC8, 0x40)),
                        ..entry("Terminal", false)
                    },
                ],
                tray_icons: vec![
                    Some(indicator::draw_record(16).into()),
                    Some(indicator::draw_microphone(16).into()),
                ],
                clock: clock.into(),
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
            };
            let mut bar = Canvas::new(930, 40);
            look::draw_taskbar(&mut bar, &glass_theme, &text, &start_text, &layout, &state);
            c.blit_canvas(10, y, &bar);
        }
    }

    {
        use reprisekit::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarState};
        use reprisekit::trayflyout::{self, FlyoutLayout};
        let start_text = TextRenderer::load(&style.start_text.font());
        let layout = TaskbarLayout::compute(
            &glass_theme.metrics,
            &style.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 0,
                tray: 2,
                chevron: true,
                show_desktop: false,
                ..TaskbarCounts::default()
            },
            text.measure("00:00", false, glass_theme.fonts.size_px),
        );
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            tasks: vec![],
            tray_icons: vec![
                Some(indicator::draw_record(16).into()),
                Some(indicator::draw_microphone(16).into()),
            ],
            clock: "3:45 PM".into(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(800, 40);
        look::draw_taskbar(&mut bar, &glass_theme, &text, &start_text, &layout, &state);
        c.blit_over(15, 890, &bar);
        let icons: Vec<Option<Canvas>> = (0..6)
            .map(|i| {
                Some(if i % 2 == 0 {
                    indicator::draw_record(16)
                } else {
                    indicator::draw_microphone(16)
                })
            })
            .collect();
        let chevron = layout.chevron.unwrap();
        let fl = FlyoutLayout::compute(&glass_theme, icons.len());
        let fx = 15 + chevron.x + chevron.width / 2 - fl.width / 2;
        let m = reprisekit::flyout::margin(&glass_theme);
        c.blit_over(
            fx - m.left,
            890 - fl.height - m.top,
            &trayflyout::draw(&glass_theme, &text, &fl, &icons, false),
        );
        let classic = Theme::default();
        let fl = FlyoutLayout::compute(&classic, icons.len());
        c.blit_canvas(
            15,
            890 - fl.height,
            &trayflyout::draw(&classic, &text, &fl, &icons, false),
        );
    }
    c
}

/// The Glass task row in each `[taskbar] buttons` mode (combine, when_full, never), on the 40 px and small bars, over the striped backdrop, one 930 px bar each, with groups, lit, held and active buttons, the pager and the orb; the orb's buffer (`start_orb`) goes over each bar.
fn render_glass_task_rows() -> Canvas {
    use reprisekit::taskbar::{
        PagerButton, PagerMark, TaskEntry, TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let full = interfaces::theme_for(Interface::Glass);
    let small = reprisekit::glass::small_bar_style(&Default::default());
    type Row<'a> = (
        &'a [u8],
        TaskRow,
        &'a [&'a str],
        bool,
        usize,
        Option<usize>,
        bool,
        bool,
        Option<usize>,
        Option<PagerMark>,
    );
    let rows: [Row; 14] = [
        (
            &[0, 1, 2, 3],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            3,
            None,
            false,
            false,
            None,
            None,
        ),
        (
            &[1, 3, 1],
            TaskRow::Shared,
            &["Notes", "Terminal", "Explorer"],
            false,
            2,
            None,
            true,
            false,
            None,
            None,
        ),
        (
            &[1, 1, 1, 1],
            TaskRow::AtLeast(62),
            &["Notes", "Terminal", "Terminal", "Explorer"],
            false,
            3,
            None,
            false,
            true,
            None,
            None,
        ),
        (
            &[0, 1, 3],
            TaskRow::Icons,
            &["", "", ""],
            true,
            2,
            None,
            false,
            true,
            None,
            None,
        ),
        (
            &[1, 3, 1, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            true,
            3,
            Some(2),
            true,
            false,
            None,
            None,
        ),
        (
            &[1, 3, 1, 3],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            0,
            Some(1),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[1, 1, 1],
            TaskRow::AtLeast(62),
            &["Notes", "Terminal", "Explorer"],
            false,
            2,
            Some(2),
            false,
            false,
            Some(1),
            None,
        ),
        (
            &[0, 1, 1, 1],
            TaskRow::AtLeast(62),
            &["", "Notes", "Terminal", "Explorer"],
            false,
            1,
            Some(2),
            false,
            false,
            None,
            None,
        ),
        (
            &[0, 1, 0, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            3,
            Some(0),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[0, 1, 0, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            true,
            3,
            Some(0),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            false,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Light),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            false,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Tile),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            true,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Light),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            true,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Tile),
        ),
    ];
    let mut c = blurred_stripes(950, 780);
    let mut y = 0;
    for (stacks, row, titles, is_small, active, hot, lit, held, pressed, mark) in rows {
        let mut theme = full.clone();
        if is_small {
            theme.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
            theme.look = look::Look::Styled(small.clone());
        }
        let style = theme.look.style().unwrap();
        let start_text = TextRenderer::load(&style.start_text.font());
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            pager: if mark.is_some() { 4 } else { 0 },
            show_desktop: is_small,
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let clock_w = text.measure(clock, false, theme.fonts.size_px);
        let mut layout =
            TaskbarLayout::compute(&theme.metrics, &style.taskbar, 930, true, counts, clock_w);
        layout.lay_tasks(&theme.metrics, &style.taskbar, stacks, row);
        let tasks = titles
            .iter()
            .enumerate()
            .map(|(i, t)| TaskEntry {
                title: (*t).into(),
                active: i == active,
                attention: false,
                icon: Some(if i % 2 == 0 {
                    indicator::draw_record(16).into()
                } else {
                    indicator::draw_microphone(16).into()
                }),
                glow: None,
            })
            .collect();
        let state = TaskbarState {
            start_pressed: held,
            start_focus: false,
            focus: None,
            start_hot: lit,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: match mark {
                Some(_) => (0..4)
                    .map(|i| PagerButton {
                        current: i == 1,
                        attention: false,
                    })
                    .collect(),
                None => vec![],
            },
            pager_hot: mark.map(|_| 2),
            pager_mark: mark.unwrap_or_default(),
            task_hot: hot,
            task_pressed: pressed,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            tasks,
            tray_icons: vec![],
            clock: clock.into(),
            show_desktop_hot: lit,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(930, layout.height as u32);
        look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
        y -= layout.start_above.map_or(0, |r| r.y);
        c.blit_over(10, y + 5, &bar);
        if let Some(r) = layout.start_orb(held) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, &theme, &layout, &state);
            c.blit_over(10 + r.x, y + 5 + r.y, &orb);
        }
        y += layout.height + 10;
    }
    c
}

/// Glass's lit task buttons over the striped backdrop with the spot under the pointer at several offsets, one 930 px bar each; each icon is a square in its own colour that the button glows in. The orb's buffer goes over each bar.
fn render_glass_spots() -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let full = interfaces::theme_for(Interface::Glass);
    let small = reprisekit::glass::small_bar_style(&Default::default());
    let colours = [
        Color::rgb(0xF0, 0xC8, 0x40),
        Color::rgb(0x30, 0x70, 0xD0),
        Color::rgb(113, 164, 168),
        Color::rgb(0x80, 0x80, 0x80),
    ];
    let icons = [false, true].map(|small| {
        colours.map(|c| {
            let px = if small { 16 } else { 32 };
            let mut icon = Canvas::new(px, px);
            icon.fill_rect(icon.bounds(), c);
            icon
        })
    });
    let (one, labelled) = (TaskRow::Icons, TaskRow::AtLeast(62));
    type Row = (
        &'static [u8],
        TaskRow,
        bool,
        Option<usize>,
        Option<usize>,
        Option<usize>,
        i32,
    );
    let rows: [Row; 8] = [
        (&[1, 1, 3, 1], one, false, None, Some(0), None, 10),
        (&[1, 1, 3, 1], one, false, None, Some(0), None, 50),
        (&[1, 1, 3, 1], one, false, Some(1), Some(1), None, 10),
        (&[1, 1, 3, 1], one, false, None, Some(2), None, 56),
        (&[1, 1, 1, 1], labelled, false, None, Some(1), None, 20),
        (&[1, 1, 1, 1], labelled, false, None, Some(1), None, 140),
        (&[1, 1, 3, 1], one, true, None, Some(1), None, 10),
        (&[1, 1, 3, 1], one, true, None, None, Some(1), 34),
    ];
    let mut c = blurred_stripes(950, 440);
    let mut y = 0;
    for (stacks, row, is_small, active, hot, held, x) in rows {
        let mut theme = full.clone();
        if is_small {
            theme.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
            theme.look = look::Look::Styled(small.clone());
        }
        let style = theme.look.style().unwrap();
        let glow = style.task_glow.expect("Glass buttons glow").colour;
        let start_text = TextRenderer::load(&style.start_text.font());
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let clock_w = text.measure(clock, false, theme.fonts.size_px);
        let mut layout =
            TaskbarLayout::compute(&theme.metrics, &style.taskbar, 930, true, counts, clock_w);
        layout.lay_tasks(&theme.metrics, &style.taskbar, stacks, row);
        let titles = ["Notes", "Terminal", "Explorer", "Paint"];
        let tasks = (0..stacks.len())
            .map(|i| {
                let icon = &icons[usize::from(is_small)][i % colours.len()];
                TaskEntry {
                    title: if row == labelled {
                        titles[i].into()
                    } else {
                        String::new()
                    },
                    active: Some(i) == active,
                    attention: false,
                    icon: Some(icon.clone().into()),
                    glow: glow(icon),
                }
            })
            .collect();
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: hot,
            task_pressed: held,
            task_hot_x: Some(x),
            task_shift: Vec::new(),
            task_float: None,
            tasks,
            tray_icons: vec![],
            clock: clock.into(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(930, layout.height as u32);
        look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
        y -= layout.start_above.map_or(0, |r| r.y);
        c.blit_over(10, y + 5, &bar);
        if let Some(r) = layout.start_orb(false) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, &theme, &layout, &state);
            c.blit_over(10 + r.x, y + 5 + r.y, &orb);
        }
        y += layout.height + 10;
    }
    c
}

/// Glass's task preview popups over the striped backdrop: one window, then a group of three with the middle one hovered. White boxes stand in for the live pictures.
fn render_glass_preview() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = interfaces::theme_for(Interface::Glass);
    let mut c = blurred_stripes(1000, 440);
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(2, 1, 12, 14), Color::rgb(80, 193, 255));
    let mut window = Canvas::new(184, 108);
    window.fill_rect(window.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    window.fill_rect(Rect::new(0, 0, 184, 8), Color::rgb(0xDD, 0xE6, 0xF2));
    let grid = interfaces::task_preview_grid(&theme);
    let mut popup = |n: usize, button: Rect, bounds: Rect, hot: Option<usize>| {
        let l = taskpreview::TaskPreviewLayout::compute(
            grid,
            n,
            button,
            bounds,
            reprisekit::taskbar::Edge::Bottom,
        );
        let a = l.canvas();
        let mut panel = Canvas::new(a.width as u32, a.height as u32);
        let titles = [
            "Untitled - Notepad",
            "notes.txt - Notepad",
            "A title too long for its cell",
        ];
        let cells: Vec<taskpreview::Cell> = (0..n)
            .map(|i| taskpreview::Cell {
                title: titles[i % titles.len()],
                icon: Some(&icon),
                active: i == 0,
                picture: Some(&window),
            })
            .collect();
        interfaces::draw_task_preview(&mut panel, &theme, &text, &l, &cells, hot);
        c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &panel);
    };
    popup(
        1,
        Rect::new(100, 200, 60, 40),
        Rect::new(0, 0, 400, 200),
        None,
    );
    popup(
        3,
        Rect::new(600, 420, 60, 40),
        Rect::new(260, 220, 740, 200),
        Some(1),
    );
    c
}

/// Glass's Alt+Tab thumbnail panels over the striped backdrop, then the icon style. White boxes stand in for the pictures; an entry with no picture shows its icon.
fn render_glass_switcher() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = interfaces::theme_for(Interface::Glass);
    let grid = interfaces::switcher_grid(&theme);
    let mut c = blurred_stripes(1100, 900);
    let mut panel =
        |n: usize, area: Rect, selected: usize, hot: Option<usize>, grid: switcher::Grid| {
            let l = switcher::SwitcherLayout::thumbnails(n, selected, area, grid);
            let pictured = n.min(5);
            let pictures: Vec<Rect> = l.wells[..pictured]
                .iter()
                .map(|&w| taskpreview::fit(400, 300, w, 1))
                .collect();
            let stills: Vec<Canvas> = pictures
                .iter()
                .map(|r| {
                    let mut s = Canvas::new(r.width as u32, r.height as u32);
                    s.fill_rect(s.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
                    s.fill_rect(Rect::new(0, 0, r.width, 6), Color::rgb(0xDD, 0xE6, 0xF2));
                    s
                })
                .collect();
            let mut cells: Vec<switcher::ThumbCell> = pictures
                .iter()
                .zip(&stills)
                .map(|(&r, s)| switcher::ThumbCell {
                    icon: None,
                    picture: Some(r),
                    still: Some(s),
                })
                .collect();
            cells.resize(
                n,
                switcher::ThumbCell {
                    icon: None,
                    picture: None,
                    still: None,
                },
            );
            let a = l.canvas();
            let mut p = Canvas::new(a.width as u32, a.height as u32);
            interfaces::draw_switcher(
                &mut p,
                &theme,
                &text,
                &l,
                "Untitled - Notepad",
                &cells,
                selected,
                hot,
            );
            interfaces::draw_switcher_badges(&mut p, &theme, &l, &cells);
            c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &p);
        };
    panel(6, Rect::new(0, 0, 1100, 260), 1, Some(3), grid);
    panel(1, Rect::new(0, 240, 500, 220), 0, None, grid);
    panel(2, Rect::new(450, 240, 650, 220), 0, None, grid);
    panel(
        3,
        Rect::new(0, 660, 1100, 240),
        2,
        None,
        grid.sized(switcher::Size::Large),
    );
    let icons = interfaces::switcher_icon_grid(&theme).expect("Glass has an icon grid");
    let icon = |rgb: (u8, u8, u8)| {
        let mut i = Canvas::new(32, 32);
        i.fill_rect(Rect::new(4, 4, 24, 24), Color::rgb(rgb.0, rgb.1, rgb.2));
        i
    };
    let glyphs = [
        icon((40, 120, 220)),
        icon((220, 140, 40)),
        icon((60, 160, 80)),
    ];
    let mut icon_panel = |n: usize, area: Rect, selected: usize, hot: Option<usize>| {
        let l = switcher::SwitcherLayout::thumbnails(n, selected, area, icons);
        let cells: Vec<switcher::ThumbCell> = (0..n)
            .map(|k| switcher::ThumbCell {
                icon: (k % 4 != 3).then(|| &glyphs[k % 4 % 3]),
                picture: None,
                still: None,
            })
            .collect();
        let a = l.canvas();
        let mut p = Canvas::new(a.width as u32, a.height as u32);
        interfaces::draw_switcher(
            &mut p,
            &theme,
            &text,
            &l,
            "Untitled - Notepad",
            &cells,
            selected,
            hot,
        );
        c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &p);
    };
    icon_panel(9, Rect::new(0, 460, 650, 200), 2, Some(4));
    icon_panel(3, Rect::new(600, 460, 500, 200), 1, None);
    c
}

/// The Start panel over a flat blue desktop: Home, the program tree, Search, a jump list, and Home under Classic.
fn render_start_panel() -> Canvas {
    use reprisekit::startpanel::{
        self as sp, Mode, PlaceRow, ProgramRow, ResultGroup, ResultRow, Spot, StartPanel, TreeRow,
    };
    let square = |px: u32, c: Color| {
        let mut i = Canvas::new(px, px);
        let n = px as i32;
        i.fill_rect(Rect::new(n / 8, n / 8, n * 3 / 4, n * 3 / 4), c);
        i
    };
    let blue = Color::rgb(40, 120, 220);
    let prog = |name: &str, jump: bool| ProgramRow {
        name: name.into(),
        icon: Some(square(32, blue)),
        jump,
        new_pin: false,
    };
    let row = |name: &str| ResultRow {
        name: name.into(),
        icon: Some(square(16, blue)),
    };
    let app = |name: &str, depth: u8| TreeRow::App {
        name: name.into(),
        icon: Some(square(16, blue)),
        depth,
    };
    let base = || {
        let mut p = StartPanel::new();
        p.pinned = vec![prog("Text Editor", false)];
        p.frequent = [
            ("Calculator", false),
            ("Getting Started", true),
            ("Media Player", false),
            ("Paint", false),
            ("Sticky Notes", false),
            ("Screenshot", false),
            ("Remote Desktop Connection", false),
            ("Magnifier", false),
            ("Solitaire", false),
        ]
        .iter()
        .map(|&(n, j)| prog(n, j))
        .collect();
        p.places = [
            "user",
            "Documents",
            "Pictures",
            "Music",
            "Recent Items",
            "Computer",
            "Control Panel",
            "Devices and Printers",
            "Default Programs",
            "Help and Support",
        ]
        .iter()
        .enumerate()
        .map(|(k, l)| PlaceRow {
            label: l.to_string(),
            picture: (k == 1).then(|| square(48, Color::rgb(230, 200, 90))),
            rule_before: k == 4 || k == 6,
            submenu: k == 4,
        })
        .collect();
        p.user_picture = Some(square(48, Color::rgb(240, 120, 30)));
        p.tree = vec![
            app("Default Programs", 0),
            app("Web Browser", 0),
            TreeRow::Folder {
                name: "Accessories".into(),
                open: true,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
            app("Calculator", 1),
            app("Command Prompt", 1),
            app("Connect to a Projector", 1),
            app("Getting Started", 1),
            app("Math Input Panel", 1),
            app("Notepad", 1),
            app("Paint", 1),
            app("Remote Desktop Connection", 1),
            app("Run", 1),
            app("Screenshot", 1),
            app("Sound Recorder", 1),
            app("Sticky Notes", 1),
            app("Sync Center", 1),
            app("File Manager", 1),
            app("Word Processor", 1),
            app("Zip Archiver", 1),
            TreeRow::Folder {
                name: "Games".into(),
                open: false,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
            TreeRow::Folder {
                name: "Maintenance".into(),
                open: false,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
        ];
        p.jump = vec![row("Personalize"), row("notes.txt"), row("todo.txt")];
        p.jump_title = "Recent".into();
        p
    };
    let home_lit = {
        let mut p = base();
        p.pointer(Some(Spot::Program(1)));
        p
    };
    let place_lit = {
        let mut p = base();
        p.pointer(Some(Spot::Place(1)));
        p
    };
    let programs = {
        let mut p = base();
        p.set_mode(Mode::Programs);
        p.pointer(Some(Spot::Tree(4)));
        p
    };
    let search = {
        let mut p = base();
        p.key(sp::Key::Char(EditKey::Insert("no".into())));
        p.set_results(vec![
            ResultGroup {
                title: "Programs (3)".into(),
                rows: vec![row("Notepad"), row("Sticky Notes"), row("Web Browser")],
            },
            ResultGroup {
                title: "Files (2)".into(),
                rows: vec![row("notes.txt"), row("north.png")],
            },
        ]);
        p
    };
    let jump = {
        let mut p = base();
        p.set_mode(Mode::Jump(2));
        p.pointer(Some(Spot::Jump(1)));
        p
    };
    let glass = reprisekit::glass::Settings::default();
    let glass_theme = interfaces::theme_for(Interface::Glass);
    let classic = interfaces::theme_for(Interface::Classic);
    let text = interfaces::start_panel_text(&glass_theme);
    let (cell_w, cell_h) = (520, 540);
    let mut c = Canvas::new(3 * cell_w as u32, 2 * cell_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    let panels = [
        (&home_lit, &glass_theme),
        (&place_lit, &glass_theme),
        (&programs, &glass_theme),
        (&search, &glass_theme),
        (&jump, &glass_theme),
        (&home_lit, &classic),
    ];
    for (k, (p, theme)) in panels.into_iter().enumerate() {
        let l = sp::layout::layout(p);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_start_panel(&mut panel, theme, &glass, &text, p, &l, None, true);
        let (x, y) = ((k % 3) as i32 * cell_w + 10, (k / 3) as i32 * cell_h + 10);
        c.blit_over(x, y + sp::layout::OVERHANG + l.canvas.y, &panel);
    }
    c
}

fn display_lines() -> osd::Lines {
    use osd::Level;
    let line = |icon, label: &str, level| osd::Line {
        icon,
        label: label.to_string(),
        level,
    };
    osd::Lines {
        title: "Brightness".to_string(),
        lines: vec![
            line(
                &reprisekit::art::icons::LAPTOP,
                "Built-in display",
                Level::At(60),
            ),
            line(
                &reprisekit::art::icons::MONITOR,
                "DELL U2720Q",
                Level::At(40),
            ),
            line(&reprisekit::art::icons::MONITOR, "LG 27GL850", Level::Fixed),
        ],
    }
}

/// The displays flyout for [`display_lines`] with night light on, after `tabs` presses of Tab.
fn display_panel(tabs: usize) -> displaypanel::DisplayPanel {
    let rows = display_lines()
        .lines
        .into_iter()
        .map(|l| displaypanel::Row {
            label: l.label,
            icon: l.icon,
            level: l.level,
        })
        .collect();
    let night = displaypanel::Night {
        enabled: true,
        on: true,
        state: "until 07:00".to_string(),
    };
    let mut p = displaypanel::DisplayPanel::new(rows, night);
    for _ in 0..tabs {
        p.key(displaypanel::Key::Tab);
    }
    p
}

/// The media keys' panel, the displays flyout and the launcher in Glass over the stripes.
fn render_glass_popups() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = interfaces::theme_for(Interface::Glass);
    let mut c = blurred_stripes(1000, 760);
    let m = reprisekit::look::panel::margin(&theme);
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 6, 24, 20), Color::rgb(80, 193, 255));
    let meter = |fraction: f64, enabled: bool| Some(osd::Meter { fraction, enabled });
    let osds = [
        ("Volume", meter(0.45, true)),
        ("Muted", meter(0.45, false)),
        ("Brightness", meter(1.0, true)),
        ("Music Player: Next", None),
    ];
    for (i, (title, meter)) in osds.into_iter().enumerate() {
        let o = osd::Osd {
            title: title.to_string(),
            meter,
        };
        let panel = osd::draw(&theme, &text, &o, Some(&icon));
        let (x, y) = (30 + (i as i32 % 2) * 300, 30 + (i as i32 / 2) * 100);
        c.blit_over(x - m.left, y - m.top, &panel);
    }
    let panel = osd::draw_lines(&theme, &text, &display_lines());
    c.blit_over(630 - m.left, 30 - m.top, &panel);
    let fm = reprisekit::flyout::margin(&theme);
    let panel = displaypanel::draw(&theme, &text, &display_panel(2));
    c.blit_over(30 - fm.left, 430 - fm.top, &panel);
    let empty = launcher::Launcher::new();
    let panel = launcher::draw(&theme, &text, &empty, Some(&icon), true, None);
    c.blit_over(30 - m.left, 250 - m.top, &panel);
    let composing = Preedit::new("ni hao".into(), 0, 6);
    let panel = launcher::draw(&theme, &text, &empty, Some(&icon), true, composing.as_ref());
    c.blit_over(30 - m.left, 340 - m.top, &panel);
    let l = launcher_sample();
    let panel = launcher::draw(&theme, &text, &l, Some(&icon), true, None);
    c.blit_over(560 - m.left, 230 - m.top, &panel);
    c
}

/// The launcher with an answer, programs, a setting and a file, the answer lit.
fn launcher_sample() -> launcher::Launcher {
    let mut small = Canvas::new(16, 16);
    small.fill_rect(Rect::new(1, 1, 14, 14), Color::rgb(40, 120, 220));
    let mut l = launcher::Launcher::new();
    l.key(launcher::Key::Char(EditKey::Insert("no".into())));
    l.set_calc(Some(launcher::Row {
        name: "= 14".to_string(),
        icon: Some(small.clone()),
    }));
    l.set_settings(vec![launcher::Row {
        name: "Notifications".to_string(),
        icon: Some(small.clone()),
    }]);
    l.set_rows(vec![
        launcher::Row {
            name: "Notepad".to_string(),
            icon: Some(small.clone()),
        },
        launcher::Row {
            name: "Notes".to_string(),
            icon: Some(small.clone()),
        },
    ]);
    l.set_files(vec![launcher::FileRow {
        name: "notes.txt".to_string(),
        folder: "~/Documents".to_string(),
        icon: Some(small),
    }]);
    l
}

/// The Glass launcher with every kind of row at `pct` text over the stripes.
fn render_glass_launcher(pct: u32) -> Canvas {
    let theme = interfaces::theme_for(Interface::Glass).with_text_size(pct);
    let text = TextRenderer::load(&FontConfig::default());
    let m = reprisekit::look::panel::margin(&theme);
    let l = launcher_sample();
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 6, 24, 20), Color::rgb(80, 193, 255));
    let panel = launcher::draw(&theme, &text, &l, Some(&icon), true, None);
    let (w, h) = (panel.width as i32, panel.height as i32);
    let mut c = blurred_stripes(w + 60 - m.left - m.right, h + 60 - m.top - m.bottom);
    c.blit_over(30 - m.left, 30 - m.top, &panel);
    c
}

/// The Classic and Blue launchers side by side with every kind of row at `pct` text.
fn render_launchers(pct: u32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let l = launcher_sample();
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 6, 24, 20), Color::rgb(80, 193, 255));
    let panels: Vec<Canvas> = [Interface::Classic, Interface::Blue]
        .iter()
        .map(|&i| {
            let theme = interfaces::theme_for(i).with_text_size(pct);
            launcher::draw(&theme, &text, &l, Some(&icon), true, None)
        })
        .collect();
    let w = panels.iter().map(|p| p.width as i32 + 30).sum::<i32>() + 30;
    let h = panels.iter().map(|p| p.height as i32).max().unwrap_or(0) + 60;
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    let mut x = 30;
    for p in &panels {
        c.blit_over(x, 30, p);
        x += p.width as i32 + 30;
    }
    c
}

/// Glass's Square frame (`[glass] frame = "square"`) over the stripes, 950x980: windows, caption button states, dialogs, dark Glass and an overlong title.
fn render_glass_square_frames() -> Canvas {
    use reprisekit::glass::{self, square};
    let square_settings = |mode| glass::Settings {
        frame: glass::Frame::Square,
        mode,
        ..glass::Settings::default()
    };
    let themes = [glass::Mode::Light, glass::Mode::Dark].map(|m| {
        interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &square_settings(m),
        )
    });
    let text = TextRenderer::load(&FontConfig::default());
    let caption_text = TextRenderer::load(&themes[0].look.style().unwrap().caption_text.font());
    let mut c = blurred_stripes(950, 980);
    let mut frame = |theme: &Theme,
                     (x, y): (i32, i32),
                     (cw, ch): (i32, i32),
                     chrome: Chrome,
                     help: Option<Interaction>,
                     st: &FrameState| {
        let g = FrameGeometry::layout(&theme.metrics, cw, ch, st.maximized, chrome);
        let r = Rect::new(x, y, g.total.0, g.total.1);
        for (i, p) in shadow::place(r, st.active, st.maximized).iter().enumerate() {
            let piece = square::shadow_piece(st.active, i, 1);
            c.blit_canvas(p.dest.x, p.dest.y, &stretched(&piece, p));
        }
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        if chrome == Chrome::WINDOW {
            look::draw_frame(&mut f, theme, &text, &caption_text, &g, st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        } else {
            look::draw_dialog_frame(&mut f, theme, &text, &g, st, help);
        }
        c.blit_over(x, y, &f);
    };
    let st = |title: &'static str,
              active: bool,
              maximized: bool,
              hot: Option<CaptionButton>,
              pressed: Option<CaptionButton>| FrameState {
        title,
        active,
        maximized,
        pressed,
        glow: Glow::lit(hot),
        icon: None,
        tabbed: false,
    };
    let [light, dark] = &themes;
    let win = Chrome::WINDOW;
    use CaptionButton::{Close, Maximize, Minimize};
    frame(
        light,
        (15, 15),
        (424, 100),
        win,
        None,
        &st("Notes", true, false, None, None),
    );
    frame(
        light,
        (485, 15),
        (424, 100),
        win,
        None,
        &st("Notes", false, false, None, None),
    );
    frame(
        light,
        (15, 190),
        (910, 60),
        win,
        None,
        &st("Notes", true, true, None, None),
    );
    for (i, b) in [Minimize, Maximize, Close].into_iter().enumerate() {
        let x = 15 + 310 * i as i32;
        let lit = st("Notes", true, false, Some(b), None);
        frame(light, (x, 295), (284, 32), win, None, &lit);
        let held = st("Notes", true, false, None, Some(b));
        frame(light, (x, 395), (284, 32), win, None, &held);
    }
    let unfocused_lit = st("Notes", false, false, Some(Close), None);
    frame(light, (15, 495), (284, 32), win, None, &unfocused_lit);
    let dialog = Chrome::DIALOG;
    for (i, active) in [true, false].into_iter().enumerate() {
        let x = 325 + 310 * i as i32;
        frame(
            light,
            (x, 495),
            (284, 32),
            dialog,
            None,
            &st("Run", active, false, None, None),
        );
    }
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (active, ptr, close_lit)) in [
        (true, idle, None),
        (true, hot, None),
        (true, held, None),
        (true, idle, Some(Close)),
        (false, idle, None),
    ]
    .into_iter()
    .enumerate()
    {
        let at = (15 + 186 * i as i32, 595);
        let s = st("Run", active, false, close_lit, None);
        frame(light, at, (160, 2), Chrome::HELP_DIALOG, Some(ptr), &s);
    }
    let restore_lit = st("Notes", true, true, Some(Maximize), None);
    frame(light, (15, 670), (450, 40), win, None, &restore_lit);
    let close_held = st("Notes", true, true, None, Some(Close));
    frame(light, (485, 670), (450, 40), win, None, &close_held);
    frame(
        dark,
        (15, 750),
        (424, 60),
        win,
        None,
        &st("Notes", true, false, None, None),
    );
    frame(
        dark,
        (485, 750),
        (424, 60),
        win,
        None,
        &st("Notes", false, false, None, None),
    );
    let long = "A title too long to sit centred in so narrow a window";
    frame(
        light,
        (15, 890),
        (220, 20),
        win,
        None,
        &st(long, true, false, None, None),
    );
    c
}

/// `p.src` of `piece` stretched to `p.dest`'s size, as the scene shows a shadow piece.
fn stretched(piece: &Canvas, p: &shadow::Placed) -> Canvas {
    let (d, s) = (p.dest, p.src);
    let mut out = Canvas::new(d.width.max(0) as u32, d.height.max(0) as u32);
    for y in 0..d.height {
        for x in 0..d.width {
            let px = piece.get(s.x + x * s.width / d.width, s.y + y * s.height / d.height);
            out.put_raw(x, y, px);
        }
    }
    out
}

/// A `w` x `h` opaque backdrop of diagonal light and dark blue stripes, box blurred three times.
fn blurred_stripes(w: i32, h: i32) -> Canvas {
    let (wu, hu) = (w as usize, h as usize);
    let mut px: Vec<[f32; 3]> = (0..wu * hu)
        .map(|i| {
            let (x, y) = ((i % wu) as i32, (i / wu) as i32);
            if (x + y) / 24 % 2 == 0 {
                [40.0, 90.0, 160.0]
            } else {
                [190.0, 215.0, 240.0]
            }
        })
        .collect();
    let r = 6i32;
    for _ in 0..3 {
        for (dx, dy) in [(1, 0), (0, 1)] {
            let src = px.clone();
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0f32; 3];
                    for d in -r..=r {
                        let sx = (x + d * dx).clamp(0, w - 1) as usize;
                        let sy = (y + d * dy).clamp(0, h - 1) as usize;
                        let p = src[sy * wu + sx];
                        for ch in 0..3 {
                            sum[ch] += p[ch];
                        }
                    }
                    px[y as usize * wu + x as usize] = sum.map(|v| v / (2 * r + 1) as f32);
                }
            }
        }
    }
    let mut out = Canvas::new(w as u32, h as u32);
    for (i, p) in px.iter().enumerate() {
        let [r, g, b] = p.map(|v| v.round() as u8);
        out.fill_rect(
            Rect::new((i % wu) as i32, (i / wu) as i32, 1, 1),
            Color::rgb(r, g, b),
        );
    }
    out
}

/// The snap layouts panel in Classic, Blue and Glass over a 1920x1050 work area, then snap assist in the right half of a half-screen desktop. Glass's panels lie on mid-grey so the translucent sheet shows.
fn render_snap_layouts() -> Canvas {
    use reprisekit::overview::{self, OverviewLayout};
    use reprisekit::snaplayouts::{CellSpec, PickerLayout, Shape};
    let (w, h) = (920, 580);
    let mut c = Canvas::new(3 * w as u32, h as u32);
    let text = TextRenderer::load(&FontConfig::default());
    let cols = |cuts: &[i32]| -> Vec<Rect> {
        cuts.windows(2)
            .map(|p| Rect::new(p[0], 0, p[1] - p[0], 1050))
            .collect()
    };
    let spec = |zones| CellSpec {
        zones,
        ..CellSpec::default()
    };
    let fixed = vec![
        spec(cols(&[0, 960, 1920])),
        spec(cols(&[0, 1280, 1920])),
        spec(cols(&[0, 640, 1280, 1920])),
        spec(vec![
            Rect::new(0, 0, 960, 1050),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ]),
        spec(vec![
            Rect::new(0, 0, 960, 525),
            Rect::new(0, 525, 960, 525),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ]),
        spec(cols(&[0, 480, 1440, 1920])),
    ];
    let suggestion = CellSpec {
        zones: cols(&[0, 960, 1920]),
        suggestion: true,
        filled: vec![(0, "Untitled - Notepad".into())],
    };
    let bar_order = [0, 1, 3, 4, 2, 5].map(|i| fixed[i].clone());
    let cells: Vec<CellSpec> = [vec![suggestion.clone()], fixed].concat();
    let bar_cells: Vec<CellSpec> = [vec![suggestion], bar_order.to_vec()].concat();
    let mut app = Canvas::new(16, 16);
    app.fill_rect(app.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
    for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = interfaces::theme_for(i);
        let x0 = k as i32 * w;
        c.fill_rect(Rect::new(x0, 0, w, h), theme.colors.background);
        let grid = PickerLayout::compute(Shape::Grid, (1920, 1050), &cells);
        let row = PickerLayout::compute(Shape::Row, (1920, 1050), &bar_cells);
        let row_top = 12 + grid.height + 12;
        if i == Interface::Glass {
            let band = Rect::new(x0, 0, w, row_top + row.height + 12);
            c.fill_rect(band, Color::rgb(0x80, 0x80, 0x80));
        }
        let panel = interfaces::draw_snap_picker(&theme, &grid, &[((0, 0), &app)], Some((0, 1)));
        c.blit_over(x0 + (w - grid.width) / 2, 12, &panel);
        let bar = interfaces::draw_snap_picker(&theme, &row, &[((0, 0), &app)], Some((3, 1)));
        c.blit_over(x0 + (w - row.width) / 2, row_top, &bar);
        let top = row_top + row.height + 16;
        let desk = Rect::new(x0, top, w, h - top);
        let left = Rect::new(desk.x, desk.y, w / 2, desk.height);
        c.fill_rect(left, theme.colors.window);
        c.fill_rect(
            Rect::new(left.x, left.y, left.width, 18),
            theme.colors.active_title,
        );
        let zone = Rect::new(desk.x + w / 2, desk.y, w / 2, desk.height);
        let ol =
            OverviewLayout::compute_assist(2, Rect::new(0, 0, zone.width, zone.height), (w, h));
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
        let titles = ["Untitled - Notepad", "Terminal"];
        let pictures: Vec<Option<Rect>> = (0..2)
            .map(|n| Some(taskpreview::fit(800, 600, ol.well(n).unwrap(), 1)))
            .collect();
        let cells: Vec<overview::Cell> = (0..2)
            .map(|n| overview::Cell {
                title: titles[n],
                icon: Some(&icon),
                picture: pictures[n],
                stand_in: None,
            })
            .collect();
        let mut layer = Canvas::new(zone.width as u32, zone.height as u32);
        interfaces::draw_snap_assist(&mut layer, &theme, &text, &ol, &cells, None, Some(1));
        for r in pictures.iter().flatten() {
            layer.fill_rect(*r, Color::rgb(0xFF, 0xFF, 0xFF));
        }
        c.blit_over(zone.x, zone.y, &layer);
    }
    c
}

/// The window overview over a dimmed desktop in each look: five windows, the second selected, the fourth hot, the third without a picture.
fn render_overview() -> Canvas {
    use reprisekit::overview::{self, OverviewLayout};
    let (w, h) = (960, 540);
    let mut c = Canvas::new(3 * w as u32, h as u32);
    let text = TextRenderer::load(&FontConfig::default());
    for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = interfaces::theme_for(i);
        let x0 = k as i32 * w;
        c.fill_rect(Rect::new(x0, 0, w, h), theme.colors.background);
        let mut dim = Canvas::new(w as u32, h as u32);
        dim.fill_rect(dim.bounds(), Color::rgb(0, 0, 0));
        c.blit_over_faded(x0, 0, &dim, 153, 255);
        let l = OverviewLayout::compute(5, Rect::new(0, 0, w, h));
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
        let titles = [
            "Terminal",
            "Untitled - Notepad",
            "My Documents",
            "Calculator",
            "Run",
        ];
        let pictures: Vec<Option<Rect>> = (0..5)
            .map(|n| (n != 2).then(|| taskpreview::fit(800, 600, l.well(n).unwrap(), 1)))
            .collect();
        let cells: Vec<overview::Cell> = (0..5)
            .map(|n| overview::Cell {
                title: titles[n],
                icon: Some(&icon),
                picture: pictures[n],
                stand_in: None,
            })
            .collect();
        let mut layer = Canvas::new(w as u32, h as u32);
        interfaces::draw_overview(&mut layer, &theme, &text, &l, &cells, 1, Some(3));
        for r in pictures.iter().flatten() {
            layer.fill_rect(*r, Color::rgb(0xFF, 0xFF, 0xFF));
            layer.fill_rect(
                Rect::new(r.x, r.y, r.width, 8),
                Color::rgb(0xDD, 0xE6, 0xF2),
            );
        }
        c.blit_over(x0, 0, &layer);
    }
    c
}

/// Tab groups' captions in Classic, navy and silver Blue, and light and dark Glass over the stripes; the last row is Classic at scale 2.
fn render_caption_tabs() -> Canvas {
    use reprisekit::controls::captiontabs::{self, CaptionTabs, TabEntry, TabHit};
    let dark = reprisekit::glass::Settings {
        mode: reprisekit::glass::Mode::Dark,
        ..Default::default()
    };
    let themes = [
        (Theme::default(), 1),
        (interfaces::theme_for(Interface::Blue), 1),
        (interfaces::blue_theme(reprisekit::blue::Variant::Silver), 1),
        (interfaces::theme_for(Interface::Glass), 1),
        (
            interfaces::theme_with(Interface::Glass, reprisekit::blue::Variant::Blue, &dark),
            1,
        ),
        (Theme::default(), 2),
    ];
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(950, 72 * themes.len() as i32 + 40);
    let mut y = 10;
    for (base, s) in &themes {
        let theme = base.scaled(*s);
        let caption_text = match theme.look.style() {
            Some(st) => TextRenderer::load(&st.caption_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let icon = reprisekit::icon::fallback(&theme, theme.icon_size());
        let entry = |title| TabEntry {
            title,
            icon: Some(&icon),
        };
        let four = CaptionTabs {
            tabs: vec![
                entry("notes.txt - Notepad"),
                entry("user@host: ~"),
                entry("Downloads"),
                entry("report.odt"),
            ],
            selected: 1,
            hot: Some(TabHit {
                tab: 2,
                close: true,
            }),
            held: None,
            ghost: None,
        };
        let ghost = CaptionTabs {
            tabs: vec![
                entry("Calculator"),
                entry("Untitled - Paint"),
                entry("Notes"),
            ],
            selected: 0,
            hot: None,
            held: None,
            ghost: Some(2),
        };
        let narrow = CaptionTabs {
            selected: 0,
            hot: None,
            ..four.clone()
        };
        let rows: Vec<(i32, i32, bool, &CaptionTabs)> = if *s == 1 {
            vec![
                (10, 520, true, &four),
                (560, 250, false, &ghost),
                (840, 90, true, &narrow),
            ]
        } else {
            vec![(10, 450, true, &four)]
        };
        let mut h = 0;
        for (x, cw, active, tabs) in rows {
            let g = FrameGeometry::layout(&base.metrics, cw, 20, false, Chrome::WINDOW).scaled(*s);
            let st = FrameState {
                title: "",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: true,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &theme, &text, &caption_text, &g, &st);
            captiontabs::draw(&mut f, &theme, &text, &caption_text, &g, tabs, active);
            f.fill_rect(g.client, theme.colors.button_face);
            c.blit_over(x, y, &f);
            h = h.max(g.total.1);
        }
        y += h + 10;
    }
    c
}
