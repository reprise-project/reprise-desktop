//! The main page's file sheet and Blue sections.

use super::cued;
use super::popups::monitor_icon;
use reprisekit::classic::frame::{Chrome, FrameGeometry, FrameState, Glow};
// Classic helpers with no look::widgets twin.
use reprisekit::classic::widgets::{CheckState, draw_label};
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::iconview;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::{
    colorbutton, combo, filesheet, shellview, switcher, taskpreview, updown,
};
use reprisekit::interface::Interface;
use reprisekit::look::widgets::{
    ICON_BUTTON_W, draw_button, draw_checkbox, draw_checkbox_disabled, draw_checkbox_state,
    draw_combo_drop, draw_group_box, draw_hscrollbar, draw_icon_button, draw_page, draw_progress,
    draw_radio, draw_radio_disabled, draw_scroll_corner, draw_scrollbar, draw_spin_buttons,
    draw_sunken_field, draw_tab, draw_trackbar,
};
use reprisekit::look::{self, Paint};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::Caret;
use reprisekit::theme::{Color, Theme};
use reprisekit::{interfaces, trackbar};

/// Switcher thumbnails, file property sheets, drag ghosts and the list view.
pub fn page_files(mut c: Canvas, theme: &Theme, text: &TextRenderer) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

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
        switcher::draw_thumbnails(&mut panel, theme, text, &l, "Terminal", &cells, 1, Some(2));
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
            text,
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
            let icon = Some(filesheet::set_picture(s));
            let (page, height) =
                filesheet::general_page(&filesheet::set_rows(&facts), "", icon, None);
            let sheet = PropSheet::new(&["General"], vec![page], (filesheet::SHEET_W, height));
            let (w, h) = sheet.size;
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
            propsheet::draw(
                &mut sheet_c,
                text,
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
                text,
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
                text,
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
            text,
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
            text,
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
            text,
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
            text,
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
            text,
            &ghosts,
            iconview::Cell::square(iconview::CELL),
            (37, 37),
        ) {
            c.blit_over(420, 9843, &img);
        }
    }

    {
        c.fill_rect(Rect::new(670, 9823, 270, 190), cs.background);
        let sv = shellview::sample(shellview::Mode::List, false);
        let lay = shellview::layout(&sv, Rect::new(0, 0, 250, 150), &theme.controls());
        let rows: Vec<shellview::DragRow> = [2, 3, 5]
            .iter()
            .map(|&index| shellview::DragRow {
                index,
                icon: sv.entries[index].icon.as_deref(),
            })
            .collect();
        let at = (lay.cells[2].x + 30, lay.cells[2].y + 8);
        if let Some((img, _)) = shellview::drag_image(theme, text, &sv, &lay, &rows, at) {
            c.blit_over(690, 9843, &img);
        }
    }

    c
}

/// The Blue sample windows and control band, then the shortcut property sheets.
pub fn page_blue(mut c: Canvas, theme: &Theme, text: &TextRenderer, with_blue: bool) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    if with_blue {
        let blue_theme = cued(interfaces::theme_for(Interface::Blue));
        c.fill_rect(Rect::new(0, 9575, 950, 238), cs.background);
        c.blit_canvas(
            10,
            9585,
            &look::draw_sample(false, &blue_theme, 1, text, None),
        );
        c.blit_canvas(
            370,
            9585,
            &look::draw_sample(true, &blue_theme, 1, text, None),
        );
    }

    if with_blue {
        let blue_theme = cued(interfaces::theme_for(Interface::Blue));
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
                text,
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
                text,
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
                &mut c, text, &bp, r, label, default, ptr, enabled, focus, px,
            );
        }
        let cb = colorbutton::ColorButton::new(bcs.background, vec![]);
        colorbutton::draw(&mut c, &bp, Rect::new(X0 + 5 * 84, y, 75, 21), &cb);
        state_label(&mut c, X0 + 5 * 84, y + 25, 76, "Colour");
        let icon = monitor_icon(s);
        for (i, ptr) in [Interaction::Idle, Interaction::Pressed]
            .into_iter()
            .enumerate()
        {
            let r = Rect::new(X0 + 7 * 84 + i as i32 * 30, y, ICON_BUTTON_W, 23);
            draw_icon_button(&mut c, text, &bp, r, &icon, true, ptr, false, px);
        }
        state_label(&mut c, X0 + 7 * 84, y + 25, 76, "Icon");
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
        draw_group_box(&mut c, text, &bp, g, label_r, "Group", px);
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
                text,
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
            text,
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
            text,
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
            text,
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
                text,
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
            updown::draw(&mut c, text, &bp, &ud, &lay, false, None, px);
            state_label(&mut c, lay.field.x, y + 22, 60, label);
        }
        for (i, (label, enabled)) in [("Year", true), ("Greyed", false)].into_iter().enumerate() {
            let mut year = updown::UpDown::new(2026, 1970, 2099).with_buddy();
            year.enabled = enabled;
            let r = Rect::new(X0 + 330 + i as i32 * 90, y, 79, 21);
            let lay = updown::layout_for(&year, r, &bcm, reprisekit::theme::TextUnits::ONE);
            updown::draw(&mut c, text, &bp, &year, &lay, false, None, px);
            state_label(&mut c, r.x, y + 23, 79, label);
        }
        {
            use reprisekit::controls::timefield;
            let tf = timefield::TimeField::new(23, 41, 24, true);
            let r = Rect::new(X0 + 510, y, 103, 20);
            let lay = timefield::layout(r, &bcm);
            timefield::draw(&mut c, text, &bp, &tf, &lay, false, None, px);
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
                text,
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
            calendar::draw(&mut c, text, &bp, &cal, &lay, false, px);
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
                text,
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
