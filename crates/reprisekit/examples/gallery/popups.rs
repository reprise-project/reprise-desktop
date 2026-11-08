use super::{blurred_stripes, cued};
use reprisekit::controls::iconview::{self, Selection};
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::{
    colorbutton, displaypanel, launcher, listview, notifyhistory, osd, placesbar, shellview,
    volumepopup,
};
use reprisekit::interface::Interface;
use reprisekit::look::widgets::BUTTON_ICON;
use reprisekit::raster::{Canvas, Rect, write_preview};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{Caret, EditKey, Preedit, TextEdit};
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::{interfaces, schemes};

/// Glass notification sheets over the stripes: Lozenge, Halo and Droplet, each in its states and in
/// a stack with its chip; the Droplet also half open and in three arrival frames.
pub fn render_glass_notify_sheets(theme: &Theme) -> Canvas {
    use reprisekit::art::{logo, userpic};
    use reprisekit::controls::notifysheet::{
        self, SheetContent, SheetIcons, SheetLayout, chip_layout, draw_chip, place_stack,
    };
    use reprisekit::controls::tooltip::{NotifyHit, NotifyReply};
    use reprisekit::style::SheetShape;
    use reprisekit::taskbar::Edge;
    let s = theme.scale.max(1);
    let text = TextRenderer::load(&theme.fonts);
    let mut c = blurred_stripes(950 * s, 1900 * s);
    let slot = 46 * s;
    let mut app = Canvas::new(slot as u32, slot as u32);
    let mask = logo::coverage_any(slot as usize);
    let clip = app.bounds();
    app.blit_mask_over(
        0,
        0,
        slot as usize,
        slot as usize,
        &mask,
        Color::rgb(0x1F, 0x6F, 0xD8),
        clip,
    );
    let picture = userpic::draw(userpic::Picture::Sunset, slot as u32);
    let content = |title: &'static str, body: &'static str| SheetContent {
        app: "Mail",
        when: "now",
        title,
        body,
        actions: &[],
        action_icons: &[],
        reply: None,
        has_picture: false,
        has_app_icon: true,
    };
    let lay = |c: &SheetContent, expanded: bool| {
        notifysheet::sheet_layout(theme, &text, SheetShape::Lozenge, c, expanded)
            .expect("Glass draws sheets")
    };
    let draw =
        |lay: &SheetLayout, with_pic: bool, hot: Option<NotifyHit>, typed: Option<&TextEdit>| {
            let icons = SheetIcons {
                app: Some(&app),
                picture: with_pic.then_some(&picture),
                actions: &[],
            };
            let reply = typed.map(|t| (t, Caret::Shown, None));
            notifysheet::draw_sheet(theme, &text, lay, icons, None, None, hot, reply)
        };
    let rest = content(
        "New message",
        "Lunch at noon? I booked the table by the window.",
    );
    let rest_lay = lay(&rest, false);
    c.blit_over(0, 0, &draw(&rest_lay, false, None, None));
    c.blit_over(
        470 * s,
        0,
        &draw(&rest_lay, false, Some(NotifyHit::Body), None),
    );
    let actions = SheetContent {
        app: "Chat",
        title: "Ana",
        body: "Are you coming to the review this afternoon?",
        actions: &["Reply", "Later"],
        has_picture: true,
        ..content("", "")
    };
    let actions_lay = lay(&actions, false);
    let row = rest_lay.height.max(170 * s);
    c.blit_over(
        0,
        row,
        &draw(&actions_lay, true, Some(NotifyHit::Button(0)), None),
    );
    let reply = SheetContent {
        reply: Some(NotifyReply {
            placeholder: "Write a reply",
            send: "Send",
        }),
        ..actions
    };
    let typed = TextEdit::new("Sounds good", false);
    c.blit_over(
        470 * s,
        row,
        &draw(&lay(&reply, true), true, None, Some(&typed)),
    );

    let stack = [
        content("New message", "Lunch at noon?"),
        SheetContent {
            app: "Calendar",
            when: "2 min ago",
            ..content("Review at 3 PM", "Room 4, with the design team.")
        },
        SheetContent {
            app: "Downloads",
            when: "5 min ago",
            ..content("Download complete", "reprise-desktop.tar.zst")
        },
    ];
    let lays: Vec<SheetLayout> = stack.iter().map(|c| lay(c, false)).collect();
    let footprints: Vec<(i32, i32)> = lays
        .iter()
        .map(|l| (l.footprint.width / s, l.footprint.height / s))
        .collect();
    let (cw, ch) = chip_layout(theme, &text, 2);
    let usable = Rect::new(0, 340, 950, 360);
    let (spots, chip_at) = place_stack(usable, &footprints, Some((cw / s, ch / s)), Edge::Bottom);
    for (l, (x, y)) in lays.iter().zip(spots) {
        c.blit_over(
            x * s - l.footprint.x,
            y * s - l.footprint.y,
            &draw(l, false, None, None),
        );
    }
    if let Some((x, y)) = chip_at {
        c.blit_over(x * s, y * s, &draw_chip(theme, &text, 2, false));
    }

    let look = notifysheet::notify_look(theme).expect("Glass draws sheets");
    let icon_in = |colour: Color| {
        let mut icon = Canvas::new(slot as u32, slot as u32);
        icon.blit_mask_over(0, 0, slot as usize, slot as usize, &mask, colour, clip);
        icon
    };
    let red = icon_in(Color::rgb(0xD8, 0x3A, 0x2E));
    let blue = icon_in(Color::rgb(0x1F, 0x6F, 0xD8));
    let grey = icon_in(Color::rgb(0x8A, 0x8A, 0x8A));
    let halo_lay = |c: &SheetContent| {
        notifysheet::sheet_layout(theme, &text, SheetShape::Halo, c, false)
            .expect("Glass draws sheets")
    };
    let halo = |lay: &SheetLayout, icon: &Canvas, with_pic: bool| {
        let icons = SheetIcons {
            app: Some(icon),
            picture: with_pic.then_some(&picture),
            actions: &[],
        };
        let tint = (look.colour)(icon);
        notifysheet::draw_sheet(theme, &text, lay, icons, tint, None, None, None)
    };
    let singles = [
        (
            &red,
            false,
            SheetContent {
                app: "Video",
                ..content("Recording saved", "Screen recording, 2 min 14 s.")
            },
        ),
        (&blue, false, content("New message", "Lunch at noon?")),
        (
            &grey,
            false,
            SheetContent {
                app: "System",
                ..content("Updates installed", "Restart to finish.")
            },
        ),
        (
            &blue,
            true,
            SheetContent {
                app: "Chat",
                title: "Ana",
                body: "Are you coming to the review this afternoon?",
                has_picture: true,
                ..content("", "")
            },
        ),
    ];
    let mut y = 720 * s;
    for (icon, with_pic, content) in &singles {
        let l = halo_lay(content);
        c.blit_over(0, y - l.footprint.y, &halo(&l, icon, *with_pic));
        y += l.footprint.height + 18 * s;
    }
    let stack = &singles[..3];
    let lays: Vec<SheetLayout> = stack.iter().map(|(_, _, c)| halo_lay(c)).collect();
    let footprints: Vec<(i32, i32)> = lays
        .iter()
        .map(|l| (l.footprint.width / s, l.footprint.height / s))
        .collect();
    let usable = Rect::new(470, 720, 480, 600);
    let (spots, chip_at) = place_stack(usable, &footprints, Some((cw / s, ch / s)), Edge::Bottom);
    for ((l, (x, y)), (icon, _, _)) in lays.iter().zip(spots).zip(stack) {
        c.blit_over(
            x * s - l.footprint.x,
            y * s - l.footprint.y,
            &halo(l, icon, false),
        );
    }
    if let Some((x, y)) = chip_at {
        c.blit_over(x * s, y * s, &draw_chip(theme, &text, 2, false));
    }

    let drop_lay = |c: &SheetContent, expanded: bool| {
        notifysheet::sheet_layout(theme, &text, SheetShape::Droplet, c, expanded)
            .expect("Glass draws sheets")
    };
    let art = userpic::draw(userpic::Picture::Sunset, (48 * s) as u32);
    let drop_icons = |with_pic: bool| SheetIcons {
        app: Some(&app),
        picture: with_pic.then_some(&art),
        actions: &[],
    };
    let note = content(
        "New message",
        "Lunch at noon? I booked the table by the window.",
    );
    let long = SheetContent {
        app: "Chat",
        title: "Ana",
        body: "Lunch at noon? The usual place by the river. Bring the photos from the trip.",
        actions: &["Reply", "Later"],
        ..content("", "")
    };
    let song = SheetContent {
        app: "Music",
        title: "Now playing",
        body: "Harbour Lights, from the album Night Ferry",
        has_picture: true,
        ..content("", "")
    };
    let hot = Some(NotifyHit::Body);
    let draw_drop = |l: &SheetLayout, with_pic: bool, hot: Option<NotifyHit>| {
        notifysheet::draw_sheet(theme, &text, l, drop_icons(with_pic), None, None, hot, None)
    };
    let opening = notifysheet::open_layout(&drop_lay(&long, false), &drop_lay(&long, true), 0.5);
    let column: [(SheetLayout, Canvas); 4] = [
        (
            drop_lay(&note, false),
            draw_drop(&drop_lay(&note, false), false, None),
        ),
        (
            drop_lay(&long, true),
            draw_drop(&drop_lay(&long, true), false, hot),
        ),
        (
            drop_lay(&song, false),
            draw_drop(&drop_lay(&song, false), true, None),
        ),
        (
            opening,
            notifysheet::draw_sheet_opening(
                theme,
                &text,
                &drop_lay(&long, false),
                &drop_lay(&long, true),
                0.5,
                drop_icons(false),
                None,
                None,
                hot,
                None,
            ),
        ),
    ];
    let mut y = 1340 * s;
    for (l, canvas) in &column {
        c.blit_over(0, y - l.footprint.y, canvas);
        y += l.footprint.height + 18 * s;
    }
    let stack = [&note, &song, &long];
    let lays: Vec<SheetLayout> = stack.iter().map(|c| drop_lay(c, false)).collect();
    let footprints: Vec<(i32, i32)> = lays
        .iter()
        .map(|l| (l.footprint.width / s, l.footprint.height / s))
        .collect();
    let usable = Rect::new(470, 1340, 480, 270);
    let (spots, chip_at) = place_stack(usable, &footprints, Some((cw / s, ch / s)), Edge::Bottom);
    for ((l, (x, y)), sc) in lays.iter().zip(spots).zip(stack) {
        c.blit_over(
            x * s - l.footprint.x,
            y * s - l.footprint.y,
            &draw_drop(l, sc.has_picture, None),
        );
    }
    if let Some((x, y)) = chip_at {
        c.blit_over(x * s, y * s, &draw_chip(theme, &text, 2, false));
    }
    // Arrival frames by a bottom bar as the host poses them: the bead, then the canvas squashed
    // with the footprint as the bead fades.
    let bead = drop_lay(&note, false);
    let bead_canvas = draw_drop(&bead, false, None);
    let fp = bead.footprint;
    let (drop, body) = notifysheet::drip_bead(theme).unwrap();
    for (i, k) in [0.1, 0.35, 0.8].into_iter().enumerate() {
        let rest = Rect::new(500, 1640 + i as i32 * 86, fp.width / s, fp.height / s);
        let (p, alpha) = notifysheet::drip_pose(k, rest, true);
        let (kx, ky) = (
            f64::from(p.width) / f64::from(rest.width),
            f64::from(p.height) / f64::from(rest.height),
        );
        let times = |v: i32, k: f64| (f64::from(v) * k).round() as i32;
        let posed = bead_canvas.resample_box(
            times(bead_canvas.width as i32, kx).max(1) as u32,
            times(bead_canvas.height as i32, ky).max(1) as u32,
        );
        let share = notifysheet::drip_bead_share(k);
        let fade = |a: f32| (a * 255.0).round() as u32;
        c.blit_over_faded(
            p.x * s - times(fp.x, kx),
            p.y * s - times(fp.y, ky),
            &posed,
            fade(alpha * (1.0 - share)),
            255,
        );
        let q = notifysheet::drip_bead_at(p, true);
        let kb = f64::from(q.height * s) / f64::from(body.height);
        let drop = drop.resample_box(
            times(drop.width as i32, kb).max(1) as u32,
            times(drop.height as i32, kb).max(1) as u32,
        );
        c.blit_over_faded(
            q.x * s - times(body.x, kb),
            q.y * s - times(body.y, kb),
            &drop,
            fade(alpha * share),
            255,
        );
    }
    c
}

/// The Glass clock popup at `pct` text over the stripes, at 10:09:30.
pub fn render_glass_clock_popup(pct: u32) -> Canvas {
    use reprisekit::controls::calendar;
    let theme = cued(interfaces::theme_for(Interface::Glass)).with_text_size(pct);
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
pub fn render_glass_flyouts(theme: &Theme) -> Canvas {
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
pub fn render_glass_items(theme: &Theme) -> Canvas {
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

pub fn render_glass_popup_parts(theme: &Theme) -> Canvas {
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
        pinned: false,
    };
    let tile = theme.look.controls().places.tile;
    for (i, (selected, hot, pressed)) in [(1, 0, None), (1, 1, Some(0))].into_iter().enumerate() {
        let places = placesbar::PlacesBar {
            places: vec![place("My Recent Documents"), place("Desktop")],
            selected: Some(selected),
            hot: Some(hot),
            pressed,
            top: 0,
        };
        let r = Rect::new(740 + i as i32 * 95, 10, placesbar::WIDTH, 2 * tile);
        placesbar::draw(&mut c, &text, &p, &places, r, px);
    }
    c
}

/// The volume popup's now-playing strip in each look: playing with an artist, Pause focused (top); paused with a long title, Previous greyed and Next held (bottom).
pub fn render_now_playing() -> Canvas {
    use volumepopup::{NowPlaying, Transport};
    let (w, h) = (480, 430);
    let mut c = Canvas::new(3 * w as u32, 2 * h as u32);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    for (i, look) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = cued(interfaces::theme_for(look));
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
        boost: false,
    }
}

/// The speaker's volume popup in `theme` at scale `n`, 950x840: seven programs scrolled by one
/// with a player's strip, then two programs running past 100 % with master at 120 % and the focus
/// ring on a thumb, and the Output list open.
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
    let mut boosted = sample_mix(2);
    boosted.boost = true;
    boosted.master.pos = 24;
    let mut vp = volumepopup::VolumePopup::new(&boosted);
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
pub fn write_volume_popups(with_blue: bool, n: i32) {
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
            &render_volume_popup(&cued(interfaces::theme_for(look)), n),
            &format!("reprisekit-gallery-volume-popup{name}{at}.png"),
        );
    }
}

/// The monitor icon at `s` device px per icon px, for the icon buttons.
pub fn monitor_icon(s: i32) -> Canvas {
    let side = (BUTTON_ICON * s) as u32;
    let mut icon = Canvas::new(side, side);
    reprisekit::art::icons::MONITOR.paint(&mut icon, 0, 0, s);
    icon
}

pub fn display_lines() -> osd::Lines {
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
pub fn display_panel(tabs: usize) -> displaypanel::DisplayPanel {
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
pub fn render_glass_popups() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(interfaces::theme_for(Interface::Glass));
    let mut c = blurred_stripes(1000, 760);
    let m = reprisekit::look::panel::margin(&theme);
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 6, 24, 20), Color::rgb(80, 193, 255));
    let meter = |fraction: f64, enabled: bool| Some(osd::Meter::new(fraction, enabled));
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
pub fn render_glass_launcher(pct: u32) -> Canvas {
    let theme = cued(interfaces::theme_for(Interface::Glass)).with_text_size(pct);
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
pub fn render_launchers(pct: u32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let l = launcher_sample();
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 6, 24, 20), Color::rgb(80, 193, 255));
    let panels: Vec<Canvas> = [Interface::Classic, Interface::Blue]
        .iter()
        .map(|&i| {
            let theme = cued(interfaces::theme_for(i)).with_text_size(pct);
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
