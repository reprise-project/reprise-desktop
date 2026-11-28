use super::*;
use crate::commands::LayoutPart;
use repriseapp::KeyInput;
use reprisekit::classic::widgets::FIELD_PAD;
use reprisekit::interface::Interface;
use reprisekit::raster::Rect;

#[path = "bare_caption.rs"]
mod bare_caption;
#[path = "search_crumb.rs"]
mod search_crumb;

fn glass(dark: bool) -> Ctx {
    let mut ctx = test_ctx();
    let g = reprisekit::glass::Settings {
        mode: match dark {
            true => reprisekit::glass::Mode::Dark,
            false => reprisekit::glass::Mode::Light,
        },
        ..Default::default()
    };
    ctx.theme = reprisekit::interfaces::theme_with(Interface::Glass, Default::default(), false, &g);
    ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
    ctx
}

fn in_folder(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    (dir, ex)
}

fn key(sym: u32, utf8: Option<&str>, alt: bool) -> KeyInput {
    KeyInput {
        sym,
        utf8: utf8.map(str::to_string),
        mods: repriseapp::Mods {
            ctrl: false,
            shift: false,
            alt,
        },
        edit: reprisekit::textedit::key_for(sym, false, false, utf8),
    }
}

/// Alt pressed and released on its own.
fn alt_tap(ex: &mut Explorer, ctx: &mut Ctx) {
    ex.key(key(keysym::ALT_L, None, false), ctx);
    route(ex, MAIN, Event::KeyUp(key(keysym::ALT_L, None, false)), ctx);
}

fn shell(ex: &Explorer, ctx: &Ctx) -> window::ShellLayout {
    ex.shell_layout(ctx).expect("Glass takes the shell layout")
}

#[test]
fn the_glass_layout_puts_back_forward_address_and_search_in_one_row() {
    let mut ctx = glass(false);
    let (_dir, ex) = in_folder(&mut ctx);
    let sl = shell(&ex, &ctx);
    assert_eq!(sl.nav, Rect::new(0, 0, 792, 34));
    assert_eq!(sl.back_fwd, Rect::new(2, 3, 72, 29));
    assert_eq!(sl.address, Rect::new(76, 6, 599, 22), "the spare width");
    assert_eq!(sl.search, Rect::new(681, 6, 109, 22), "109 wide, 2 px in");
    assert_eq!(
        ex.glass_top(&ctx),
        34,
        "the nav row is clear over the glass"
    );
    assert_eq!(sl.menu, None, "the menu bar waits for Alt");
    assert_eq!(
        sl.command,
        Rect::new(0, 36, 792, 31),
        "under the 2 px closing line"
    );
    assert_eq!(sl.view.y + FIELD_PAD, 67, "the view under the command bar");
    let pane = sl.nav_pane.expect("the navigation pane shows");
    let details = sl.details.expect("the details pane shows");
    assert_eq!((pane.x, pane.y), (0, 67));
    assert_eq!(
        pane.bottom(),
        details.y,
        "the pane runs down to the details pane"
    );
    assert_eq!(details.bottom(), 573, "the details pane along the foot");
    assert_eq!(
        sl.view.x + FIELD_PAD,
        pane.right(),
        "the view right of the pane"
    );
    let lay = ex.layout(&ctx);
    assert_eq!(
        lay.tree,
        lay.bar.map(|b| b.body),
        "the pane's tree fills it"
    );
    let lay = ex.layout(&ctx);
    assert_eq!((lay.toolbar, lay.address, lay.status), (None, None, None));
    assert!(!ex.bar.banded, "the bar draws its own strip");

    ctx.size = (1200, 600);
    let wide = shell(&ex, &ctx);
    assert_eq!(wide.search.width, 109, "the search field keeps its width");
    assert_eq!(wide.address.width, sl.address.width + 408);
}

#[test]
fn blue_and_classic_keep_the_rebar_layout() {
    for i in [Interface::Classic, Interface::Blue] {
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(i);
        let (_dir, ex) = in_folder(&mut ctx);
        assert!(ex.shell_layout(&ctx).is_none(), "{i:?}");
        let cm = ctx.theme.controls();
        let rebar = window::layout(
            ctx.size,
            &ex.settings,
            ex.explorer_bar.width,
            statusbar::height(&cm),
            0,
            &cm,
        );
        assert_eq!(ex.layout(&ctx), rebar, "{i:?}");
        assert!(ex.bar.banded, "{i:?}: the bar sits in its band");
        assert!(ex.bar_shown(&ctx));
    }
}

#[test]
fn alt_shows_the_menu_bar_and_a_second_alt_hides_it() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    let view = shell(&ex, &ctx).view;
    alt_tap(&mut ex, &mut ctx);
    let sl = shell(&ex, &ctx);
    assert_eq!(sl.menu, Some(Rect::new(0, 36, 792, 21)), "a 21 px row");
    assert_eq!(sl.view.y, view.y + 21, "the view moves down under it");
    assert_eq!(ex.bar.active, Some(0), "File is lit");
    assert_eq!(
        ex.bar_layout(&ctx).bar.height,
        21,
        "the strip fills the row"
    );
    alt_tap(&mut ex, &mut ctx);
    assert_eq!(shell(&ex, &ctx).menu, None);
    assert_eq!(shell(&ex, &ctx).view, view);
}

#[test]
fn alt_f_opens_file_with_the_bar_hidden() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    assert_eq!(shell(&ex, &ctx).menu, None);
    ex.key(key('f' as u32, Some("f"), true), &mut ctx);
    assert_eq!(ex.bar.open, Some(0), "File opens");
    let row = shell(&ex, &ctx).menu.expect("the bar shows with it");
    let file = ex.bar_layout(&ctx).frames[0];
    assert!(row.contains(file.x, file.y), "File sits in the shown row");
}

#[test]
fn f10_shows_the_bar_focused_on_file() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    ex.key(key(keysym::F10, None, false), &mut ctx);
    assert_eq!((ex.bar.active, ex.bar.open), (Some(0), None));
    assert!(shell(&ex, &ctx).menu.is_some());
    ex.key(key(keysym::F10, None, false), &mut ctx);
    assert_eq!(shell(&ex, &ctx).menu, None, "F10 again hides it");
}

#[test]
fn the_bar_hides_when_its_menu_closes_and_focus_returns_to_the_view() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    ex.key(key('f' as u32, Some("f"), true), &mut ctx);
    ex.key(key(keysym::ESCAPE, None, false), &mut ctx);
    assert_eq!(ex.bar.open, None);
    assert!(
        shell(&ex, &ctx).menu.is_some(),
        "the first Escape closes File only"
    );
    ex.key(key(keysym::ESCAPE, None, false), &mut ctx);
    assert_eq!(shell(&ex, &ctx).menu, None);
    assert_eq!(ex.focus, Focus::View);

    ex.key(key('v' as u32, Some("v"), true), &mut ctx);
    assert!(ex.bar.open.is_some());
    let view = shell(&ex, &ctx).view;
    let (x, y) = (view.x + 300, view.bottom() - 20);
    press_at(&mut ex, x, y, &mut ctx);
    assert_eq!(shell(&ex, &ctx).menu, None, "a click in the view closes it");
    assert_eq!(ex.focus, Focus::View);
}

fn press_at(ex: &mut Explorer, x: i32, y: i32, ctx: &mut Ctx) {
    ex.pointer(
        PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Default::default(),
        },
        ctx,
    );
    ex.pointer(
        PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: Default::default(),
        },
        ctx,
    );
}

#[test]
fn tab_walks_the_address_search_command_bar_and_navigation_pane_and_enter_opens_organize() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    ex.run(Command::AddressFocus, &mut ctx);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::Search);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::Commands);
    assert!(ex.cmdbar.focus.is_some(), "the bar holds its first stop");
    ex.key(key(keysym::RETURN, None, false), &mut ctx);
    assert!(ex.popup.is_some(), "Organize opens");
    ex.key(key(keysym::ESCAPE, None, false), &mut ctx);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::Tree, "then the navigation pane");
    assert_eq!(ex.cmdbar.focus, None);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::View);
    ex.run(Command::ToggleLayout(LayoutPart::NavPane), &mut ctx);
    ex.run(Command::SearchFocus, &mut ctx);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    ex.key(key(keysym::TAB, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::View, "a hidden pane takes no stop");

    let sl = shell(&ex, &ctx);
    let l = ex.cmdbar_layout(sl.command, &ctx);
    let organize = l.items[0].main;
    ex.pointer(
        PointerInput::Motion {
            x: organize.x + 4,
            y: organize.y + 4,
        },
        &mut ctx,
    );
    assert!(ex.cmdbar.hot.is_some(), "Organize lights under the pointer");
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.cmdbar.hot, None);
    press_at(&mut ex, organize.x + 4, organize.y + 4, &mut ctx);
    assert!(ex.popup.is_some(), "a click opens Organize");
}

#[test]
fn layout_menu_bar_keeps_it_shown_and_persists() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    let view = 2;
    let cmd = menus::command_for(view, &[0, 0], &ex.menu_state());
    assert_eq!(
        cmd,
        Command::ToggleLayout(LayoutPart::MenuBar),
        "View > Layout"
    );
    ex.run(cmd, &mut ctx);
    assert!(ex.settings.menu_bar);
    assert!(!ex.bar.is_active());
    assert!(shell(&ex, &ctx).menu.is_some(), "shown without Alt");
    alt_tap(&mut ex, &mut ctx);
    alt_tap(&mut ex, &mut ctx);
    assert!(
        shell(&ex, &ctx).menu.is_some(),
        "Alt does not hide a kept bar"
    );
    let saved = toml::to_string(&ex.settings).expect("serialize");
    let back: Settings = toml::from_str(&saved).expect("parse");
    assert!(back.menu_bar, "{saved}");
}

#[test]
fn glass_layout_keys_do_not_follow_blue_folders_or_status_bar() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    let before = shell(&ex, &ctx);
    let panes = |s: &Settings| (s.menu_bar, s.nav_pane, s.details_pane);
    for cmd in [
        Command::ToggleFolders,
        Command::ToggleStatusBar,
        Command::ToggleToolbar(0),
        Command::ToggleToolbar(1),
    ] {
        ex.run(cmd, &mut ctx);
    }
    assert_eq!(panes(&ex.settings), (false, true, true));
    assert_eq!(
        shell(&ex, &ctx),
        before,
        "the shell layout reads none of them"
    );
    ex.run(Command::ToggleLayout(LayoutPart::NavPane), &mut ctx);
    ex.run(Command::ToggleLayout(LayoutPart::DetailsPane), &mut ctx);
    assert_eq!(panes(&ex.settings), (false, false, false));
    assert!(
        !ex.settings.folders && !ex.settings.status_bar,
        "left as Blue set them"
    );
    let back: Settings =
        toml::from_str(&toml::to_string(&ex.settings).expect("serialize")).expect("parse");
    assert_eq!(panes(&back), (false, false, false));
    assert!(!back.folders);

    let state = ex.menu_state();
    let labels: Vec<String> = (menus::bar_menus(&state)[2].1.items().iter())
        .filter_map(|it| match it {
            reprisekit::controls::menu::MenuItem::Entry { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect();
    assert!(labels.contains(&"Layout".to_string()), "{labels:?}");
    for gone in ["Toolbars", "Status Bar", "Explorer Bar"] {
        assert!(!labels.contains(&gone.to_string()), "{gone} under Glass");
    }
}

#[test]
fn the_search_cue_names_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let reports = dir.path().join("Reports");
    std::fs::create_dir(&reports).expect("mkdir");
    let mut ctx = glass(false);
    let mut ex = test_explorer(Place::Folder(reports.clone()), &mut ctx);
    assert_eq!(ex.search_field.cue_label(), "Search Reports");
    ex.run(
        Command::GoTo(Place::Folder(dir.path().to_path_buf())),
        &mut ctx,
    );
    let name = Place::Folder(dir.path().to_path_buf()).title();
    assert_eq!(ex.search_field.cue_label(), format!("Search {name}"));
}

#[test]
fn the_clear_button_empties_search_and_restores_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("report.txt"), "x").expect("write");
    std::fs::write(dir.path().join("other.txt"), "x").expect("write");
    let mut ctx = glass(false);
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.search_run = |q, cancel| {
        let mut find = std::process::Command::new("find");
        find.arg(&q.root);
        Some(reprisefs::search::collect(find, q, cancel))
    };
    let sl = shell(&ex, &ctx);
    let clear = reprisekit::controls::searchfield::clear_rect(sl.search);
    let (cx, cy) = (clear.x + clear.width / 2, clear.y + clear.height / 2);
    press_at(&mut ex, sl.search.x + 20, cy, &mut ctx);
    assert_eq!(
        ex.focus,
        Focus::Search,
        "a click in the field takes the keyboard"
    );
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(ex.items.len(), 1, "the hit alone");
    press_at(&mut ex, cx, cy, &mut ctx);
    assert_eq!(ex.search_field.text(), "");
    assert_eq!(ex.search.caption(&[]), None, "the search is over");
    assert_eq!(ex.items.len(), 2, "the folder is back");
    assert_eq!(ex.place, place);
}

#[test]
fn the_nav_row_at_the_minimum_width_keeps_search_and_one_crumb() {
    for pct in [100, 200] {
        let mut ctx = glass(false);
        ctx.theme = ctx.theme.with_text_size(pct);
        ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
        let (_dir, ex) = in_folder(&mut ctx);
        let (w, _) = ex.min_size(MAIN).expect("the shell keeps a minimum");
        let u = ctx.theme.controls().text;
        ctx.size = (w, 400);
        let sl = shell(&ex, &ctx);
        assert_eq!(
            sl.search.width,
            u.len(109),
            "{pct}%: search keeps its width"
        );
        assert_eq!(sl.address.width, u.len(window::ADDRESS_MIN), "{pct}%");
        assert!(sl.back_fwd.right() <= sl.address.x, "{pct}%");
        assert!(sl.address.right() < sl.search.x, "{pct}%");
        assert!(sl.search.right() < w, "{pct}%");
    }
}

#[test]
fn a_look_switch_with_a_crumb_menu_open_closes_it_and_swaps_layout() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    let menu = crate::commands::Menu::default();
    ex.popup = Some(window::Popup::new((100, 30), menu, &ctx, false));
    alt_tap(&mut ex, &mut ctx);
    ex.run(Command::AddressFocus, &mut ctx);
    assert_eq!(ex.focus, Focus::Address);

    ctx.theme = reprisekit::interfaces::theme_for(Interface::Blue);
    ex.pointer(PointerInput::Motion { x: 400, y: 300 }, &mut ctx);
    assert!(ex.popup.is_none(), "the menu closes");
    assert!(!ex.bar.is_active());
    assert_eq!(ex.focus, Focus::View, "the keyboard lands in the view");
    assert!(ex.shell_layout(&ctx).is_none());
    assert!(ex.layout(&ctx).toolbar.is_some(), "Blue's rebar whole");
    assert!(ex.bar.banded);
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    assert_eq!(c.take_glass_top(), 0, "nothing left on glass");

    ctx = glass(false);
    ex.pointer(PointerInput::Motion { x: 400, y: 300 }, &mut ctx);
    assert!(!ex.bar.banded);
    assert_eq!(ex.layout(&ctx).toolbar, None, "and back whole");
}

/// Device pixels of `r` (logical) on `c`.
fn crop_hash(c: &Canvas, r: Rect) -> u64 {
    let s = c.scale();
    let (dw, _) = c.device_size();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for y in r.y * s..r.bottom() * s {
        for x in r.x * s..r.right() * s {
            let v = c.pixels[(y * dw as i32 + x) as usize];
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h
}

#[test]
fn the_nav_row_pins_light_and_dark_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for dark in [false, true] {
        let mut ctx = glass(dark);
        let dir = tempfile::tempdir().expect("a temp dir");
        let docs = dir.path().join("Documents");
        std::fs::create_dir(&docs).expect("mkdir");
        let mut ex = test_explorer(Place::Folder(docs), &mut ctx);
        let look = if dark { "dark" } else { "light" };
        for state in ["rest", "hot", "typed"] {
            match state {
                "hot" => {
                    ex.nav.hot = Some(reprisekit::controls::navbuttons::Hit::Recent);
                    ex.search_field.hot = Some(reprisekit::controls::searchfield::Hit::Field);
                }
                "typed" => {
                    ex.nav.hot = None;
                    ex.search_field.hot = Some(reprisekit::controls::searchfield::Hit::Clear);
                    ex.search_field
                        .key(reprisekit::textedit::EditKey::Insert("notes".into()));
                }
                _ => {}
            }
            let sl = shell(&ex, &ctx);
            for s in [1, 2] {
                let mut c = Canvas::with_scale(792, 573, s);
                ctx.at_scale(s, |ctx| window::draw(&ex, &mut c, ctx));
                for (part, r) in [("buttons", sl.back_fwd), ("search", sl.search)] {
                    got.push((format!("{look} {state} {part} @{s}"), crop_hash(&c, r)));
                }
            }
        }
    }
    let path = reprisekit::crate_dir!().join("tests/pins/navrow.txt");
    reprisekit::pins::check_file(&path, &got);
}

#[test]
fn escape_in_the_address_edit_returns_to_the_crumbs_and_the_view() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    ex.key(key('d' as u32, Some("d"), true), &mut ctx);
    assert_eq!(ex.focus, Focus::Address);
    assert!(ex.crumbs.editing(), "Alt+D edits the address");
    ex.key(key(keysym::ESCAPE, None, false), &mut ctx);
    assert_eq!(ex.focus, Focus::View);
    assert!(!ex.crumbs.editing(), "the crumbs are back");
}

#[test]
fn a_crumb_click_goes_to_its_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let deep = dir.path().join("a").join("b");
    std::fs::create_dir_all(&deep).expect("mkdir");
    let mut ctx = glass(false);
    ctx.size = (1200, 573);
    let mut ex = test_explorer(Place::Folder(deep), &mut ctx);
    let sl = shell(&ex, &ctx);
    let px = ctx.theme.fonts.size_px;
    let cm = ctx.theme.controls();
    let bl = reprisekit::controls::breadcrumb::layout(&ex.crumbs, sl.address, &ctx.text, px, &cm);
    let a = dir.path().join("a");
    let i = (ex.crumbs.crumbs.iter())
        .position(|c| c.label == "a")
        .expect("a crumb for a");
    let seg = bl.segments.iter().find(|s| s.crumb == i).expect("shown");
    let (x, y) = (seg.button.x + 2, seg.button.y + seg.button.height / 2);
    press_at(&mut ex, x, y, &mut ctx);
    assert_eq!(ex.place, Place::Folder(a));
}

fn drag(ex: &mut Explorer, from: i32, to: i32, y: i32, ctx: &mut Ctx) {
    let press = PointerInput::Press {
        x: from,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Default::default(),
    };
    ex.pointer(press, ctx);
    ex.pointer(PointerInput::Motion { x: to, y }, ctx);
    let release = PointerInput::Release {
        x: to,
        y,
        button: BTN_LEFT,
        mods: Default::default(),
    };
    ex.pointer(release, ctx);
}

#[test]
fn the_navigation_pane_opens_158_wide_with_a_1_px_divider_grown_with_the_text() {
    let mut ctx = glass(false);
    let (_dir, ex) = in_folder(&mut ctx);
    let pane = shell(&ex, &ctx).nav_pane.expect("the pane shows");
    assert_eq!(pane.width, 159, "158 and the divider");
    let bar = ex.layout(&ctx).bar.expect("the pane's frame");
    assert_eq!(bar.splitter, Rect::new(158, pane.y, 1, pane.height));
    let cm = ctx.theme.clone().with_text_size(200).controls();
    let big = window::shell_layout((1600, 900), &ex.settings, false, 0, &cm);
    assert_eq!(
        big.nav_pane.map(|r| r.width),
        Some(317),
        "316 at 200% and the divider"
    );
}

#[test]
fn the_divider_drags_from_either_side_clamps_and_keeps_glasss_own_width() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = in_folder(&mut ctx);
    let y = shell(&ex, &ctx).nav_pane.expect("the pane").y + 40;
    let folders = ex.settings.bar_width;
    drag(&mut ex, 160, 262, y, &mut ctx);
    assert_eq!(ex.settings.nav_width, 260, "taken 2 px right of the line");
    let before = ex.settings.nav_width;
    drag(&mut ex, 259, 300, y, &mut ctx);
    assert_eq!(
        ex.settings.nav_width, before,
        "the pane's own scrollbar left of the line keeps it"
    );
    assert_eq!(shell(&ex, &ctx).nav_pane.map(|r| r.width), Some(261));
    assert_eq!(ex.settings.bar_width, folders, "Blue's Folders width stays");
    drag(&mut ex, 262, 20, y, &mut ctx);
    assert_eq!(
        ex.settings.nav_width,
        reprisekit::controls::explorerbar::MIN_W
    );
    drag(&mut ex, 100, 5000, y, &mut ctx);
    assert_eq!(
        ex.settings.nav_width,
        792 - 1 - 200,
        "the view keeps 200 px"
    );
    let shape = |ex: &Explorer, x| ex.shape_at(x, y, &ctx);
    let line = 792 - 1 - 200;
    assert_eq!(shape(&ex, line + 4), reprisekit::pointer::Shape::ColResize);
    assert_ne!(shape(&ex, line + 5), reprisekit::pointer::Shape::ColResize);
}

#[test]
fn the_navigation_pane_width_round_trips_the_state_file() {
    let s = Settings {
        nav_width: 240,
        ..Settings::default()
    };
    let text = toml::to_string(&s).expect("serialise");
    let back: Settings = toml::from_str(&text).expect("parse");
    assert_eq!((back.nav_width, back.bar_width), (240, 200));
    let old: Settings = toml::from_str("bar_width = 260\n").expect("parse");
    assert_eq!(
        old.nav_width,
        crate::settings::NAV_W,
        "an older file opens at 158"
    );
}

#[test]
fn the_navigation_divider_pins_light_and_dark_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for dark in [false, true] {
        let mut ctx = glass(dark);
        let (_dir, ex) = in_folder(&mut ctx);
        let pane = shell(&ex, &ctx).nav_pane.expect("the pane");
        let r = Rect::new(pane.right() - 4, pane.y + 8, 8, 24);
        for s in [1, 2] {
            let mut c = Canvas::with_scale(792, 573, s);
            ctx.at_scale(s, |ctx| window::draw(&ex, &mut c, ctx));
            let look = if dark { "dark" } else { "light" };
            got.push((format!("{look} divider @{s}"), crop_hash(&c, r)));
        }
    }
    let path = reprisekit::crate_dir!().join("tests/pins/navpane_divider.txt");
    reprisekit::pins::check_file(&path, &got);
}
