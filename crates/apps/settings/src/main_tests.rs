use super::*;
use crate::locations::location_answers;
use repriseapp::{BTN_LEFT, Extended, KeyInput, PointerInput};
use reprisekit::classic::widgets::BarHit;
use reprisekit::controls::{form, listview, propsheet};
use reprisekit::theme::FontConfig;
use std::sync::atomic::Ordering;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "reprise-settings-main-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

fn app(tag: &str) -> SettingsApp {
    SettingsApp::open(tmp(tag).join("config.toml"))
}

fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (10, 10),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: repriseapp::heads::HeadRequests::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

const MAIN: WinId = repriseapp::MAIN;

fn page_request(name: &str) -> instance::Request {
    instance::Request {
        action: Some((PAGE_ACTION.into(), vec![name.into()])),
        handoff: Default::default(),
    }
}

/// A left press at `time` on side list row `row`, found by walking down the list's body, in a
/// window that shows the whole sheet.
fn press_side_row(a: &mut SettingsApp, ctx: &mut Ctx, row: usize, time: u32) {
    ctx.size = (1920, 1080);
    let (list, rect, _) = a.side_list().expect("a page with a side list");
    let lay = listview::layout(list, rect, &ctx.forms().controls());
    let x = lay.body.x + 8;
    let y = (lay.body.y..lay.body.bottom())
        .find(|&y| list.hit(&lay, x, y) == listview::Hit::Row(row))
        .expect("the row is on screen");
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods: repriseapp::Mods::default(),
    };
    a.event(MAIN, Event::Pointer(press), ctx);
}

#[test]
fn a_field_copies_its_selection_and_takes_a_paste() {
    let mut a = app("field-clipboard");
    let mut ctx = test_ctx();
    a.open_page(Page::Windows, Some("snap_layouts"));
    a.event(MAIN, Event::Pasted("1|1 2\nmore".into()), &mut ctx);
    let windows = Page::Windows.index();
    let text = |a: &SettingsApp| match &a.sheet.pages[windows].item("snap_layouts").unwrap().control
    {
        form::Control::Edit { edit, .. } => edit.text.clone(),
        _ => unreachable!(),
    };
    assert_eq!(text(&a), "1|1 2", "the paste's first line goes in");
    let ctrl = |ch: u8| KeyInput {
        sym: u32::from(ch),
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: true,
            ..Default::default()
        },
        edit: reprisekit::textedit::key_for(u32::from(ch), true, false, None),
    };
    a.event(MAIN, Event::Key(ctrl(b'a')), &mut ctx);
    let r = a.event(MAIN, Event::Key(ctrl(b'c')), &mut ctx);
    assert_eq!(r.copy.as_deref(), Some("1|1 2"));
    assert!(a.event(MAIN, Event::Key(ctrl(b'v')), &mut ctx).paste);
}

fn enter(a: &mut SettingsApp, ctx: &Ctx) {
    let k = KeyInput {
        sym: reprisekit::textedit::keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    a.key(k, &ctx.forms().controls());
}

fn tab(a: &mut SettingsApp, ctx: &Ctx, back: bool) {
    let k = KeyInput {
        sym: reprisekit::textedit::keysym::TAB,
        utf8: None,
        mods: repriseapp::Mods {
            shift: back,
            ..Default::default()
        },
        edit: None,
    };
    a.key(k, &ctx.forms().controls());
}

#[test]
fn the_location_list_takes_its_tab_stop_after_its_check_on_the_advanced_page() {
    let mut a = app("advanced-tab");
    let ctx = test_ctx();
    a.open_page(Page::Advanced, None);
    assert!(
        !a.locations_focus,
        "opening the page leaves the list unfocused"
    );

    let page = a.sheet.open_mut();
    let check = page.index_of(pages::PRIVACY_LOCATION).unwrap();
    page.set_focus(Some(check));
    tab(&mut a, &ctx, false);
    assert!(
        a.locations_focus && a.locations.focus_ring,
        "Tab from the check enters the list"
    );
    assert_eq!(a.sheet.open().focus(), None, "one dotted rectangle");

    tab(&mut a, &ctx, false);
    assert!(!a.locations_focus);
    let next = a
        .sheet
        .open()
        .focus()
        .expect("a form control after the list");
    assert!(next > check, "Tab leaves the list onward, got item {next}");

    tab(&mut a, &ctx, true);
    assert!(a.locations_focus, "Shift+Tab comes back into the list");
    tab(&mut a, &ctx, true);
    assert!(!a.locations_focus);
    assert_eq!(
        a.sheet.open().focus(),
        Some(check),
        "Shift+Tab leaves it for its check"
    );
}

#[test]
fn a_double_click_on_a_file_type_opens_the_chooser_like_enter() {
    let mut a = app("types-double-click");
    let mut ctx = test_ctx();
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.refill_defaults();
    let mime = |a: &SettingsApp| a.chooser.as_ref().map(|c| c.mime.clone());

    press_side_row(&mut a, &mut ctx, 1, 1000);
    assert_eq!(mime(&a), None, "one click only picks the row");
    press_side_row(&mut a, &mut ctx, 1, 1000 + 1100);
    assert_eq!(mime(&a), None, "1100 ms apart is two clicks");
    press_side_row(&mut a, &mut ctx, 1, 1000 + 1400);
    let double = mime(&a).expect("the chooser opens on a double click");

    a.chooser = None;
    enter(&mut a, &ctx);
    assert_eq!(mime(&a), Some(double), "Enter opens it for the same type");
}

#[test]
fn picking_a_terminal_keeps_its_arguments_and_typed_arguments_follow_the_program() {
    let mut a = app("terminal-args");
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.want.terminal = "foot -e tmux".into();
    a.refill_defaults();
    let page = Page::DefaultPrograms.index();
    a.sheet.pages[page].set_combo(defaultspage::TERMINAL, 1);
    a.defaults_changed(defaultspage::TERMINAL);
    assert_eq!(a.want.terminal, "/usr/bin/kitty -e tmux");
    if let Some(form::Control::Edit { edit, .. }) = a.sheet.pages[page]
        .item_mut(defaultspage::TERMINAL_ARGS)
        .map(|i| &mut i.control)
    {
        *edit = reprisekit::textedit::TextEdit::new("--hold", false);
    }
    a.defaults_changed(defaultspage::TERMINAL_ARGS);
    assert_eq!(a.want.terminal, "/usr/bin/kitty --hold");
    assert_eq!(
        a.sheet.pages[page].combo_selected(defaultspage::TERMINAL),
        Some(1),
        "the program stays picked"
    );
}

#[test]
fn the_add_dialog_opens_at_the_text_size() {
    let mut a = app("add-grown");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    a.follow_text(&ctx);
    a.open_add(Pick::Autostart);
    let client = a
        .add
        .as_ref()
        .map(|d| d.client())
        .expect("the dialog is open");
    assert_ne!(client, filedlg::CLIENT, "the dialog grows at 200%");
    let kid = a.children().into_iter().find(|k| k.id == ADD_WIN);
    assert_eq!(
        kid.map(|k| k.size),
        Some(client),
        "its window opens at the grown size"
    );
}

#[test]
fn browse_in_the_chooser_picks_a_program_file_into_its_list() {
    let mut a = app("types-browse");
    let mut ctx = test_ctx();
    a.open_page(Page::DefaultPrograms, None);
    a.defaults = Some(defaultspage::tests::defaults());
    a.refill_defaults();
    press_side_row(&mut a, &mut ctx, 1, 1000);
    enter(&mut a, &ctx);
    assert!(a.chooser.is_some());
    let alt_b = Event::Key(KeyInput {
        sym: 'b' as u32,
        utf8: Some("b".into()),
        mods: repriseapp::Mods {
            alt: true,
            ..Default::default()
        },
        edit: reprisekit::textedit::key_for('b' as u32, false, false, Some("b")),
    });
    a.event(CHOOSER_WIN, alt_b, &mut ctx);
    assert!(
        a.add.is_some() && a.add_for == Pick::Program,
        "Browse... opens the file dialog"
    );
    let kids = a.children();
    assert!(kids.iter().any(|k| k.id == ADD_WIN && k.modal));
    assert!(
        kids.iter().any(|k| k.id == CHOOSER_WIN),
        "the chooser stays up under it"
    );
    a.browsed_program(std::path::Path::new("/opt/view"));
    let picked = a
        .chooser
        .as_ref()
        .and_then(|c| c.ow.chosen())
        .map(|p| p.id.clone());
    assert_eq!(picked.as_deref(), Some("/opt/view"));
}

#[test]
fn a_bind_on_a_none_rows_keys_takes_that_row() {
    let mut a = app("shortcut-none-row");
    let q = (a.want.shortcuts.iter())
        .position(|b| b.keys == "Super+q")
        .unwrap();
    model::forget(&mut a.want, model::Row::Bind(q));
    let n = a.want.shortcuts.len();
    a.open_shortcut(None);
    assert_eq!(
        a.accept_shortcut(model::Shortcut::new("Super+q", "lock", "")),
        q
    );
    assert_eq!(a.want.shortcuts.len(), n);
    assert_eq!(
        a.want.shortcuts[q],
        model::Shortcut::new("Super+q", "lock", "")
    );

    model::forget(&mut a.want, model::Row::Bind(q));
    a.want
        .shortcuts
        .push(model::Shortcut::new("Super+F9", "close", ""));
    a.open_shortcut(Some(n));
    assert_eq!(
        a.accept_shortcut(model::Shortcut::new("Super+q", "close", "")),
        n - 1
    );
    assert_eq!(a.want.shortcuts.len(), n);
    assert!(!a.want.shortcuts.iter().any(|b| b.action == "none"));
}

#[test]
fn a_double_click_on_a_shortcut_edits_it_like_enter() {
    let mut a = app("shortcuts-double-click");
    let mut ctx = test_ctx();
    a.open_page(Page::Keyboard, None);
    press_side_row(&mut a, &mut ctx, 0, 1000);
    assert!(a.shortcut.is_none(), "one click only picks the row");
    press_side_row(&mut a, &mut ctx, 0, 1300);
    assert!(a.shortcut.is_some(), "a double click opens the editor");
    let target = a.shortcut_target.take();
    a.shortcut = None;
    enter(&mut a, &ctx);
    assert!(a.shortcut.is_some());
    assert_eq!(a.shortcut_target, target, "Enter edits the same row");
}

fn pick_row(a: &mut SettingsApp, i: usize) {
    let page = &mut a.sheet.pages[Page::Background.index()];
    if let Some(form::Control::ListBox { list, .. }) = page
        .item_mut(backgroundpage::WALLPAPER)
        .map(|i| &mut i.control)
    {
        list.select(i, 11);
    }
    a.pick_wallpaper();
}

/// Under Blue the list starts on Blue's own built-in; a built-in
/// picked is written as `reprise:design/variant`, and "(None)" as an
/// empty image, which stands for none rather than Blue's default.
#[test]
fn the_background_list_picks_and_writes_a_builtin() {
    let dir = tmp("builtin");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[appearance]\ninterface = \"blue\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone());
    let at = backgroundpage::selected(&a.sheet.pages[Page::Background.index()]);
    assert_eq!(
        at.map(backgroundpage::row),
        Some(backgroundpage::Row::Builtin(
            backgroundpage::builtin("reprise:tarn/blue").unwrap()
        ))
    );
    let dusk = backgroundpage::builtins()
        .iter()
        .position(|b| b.to_string() == "ridges/dusk")
        .unwrap();
    assert!(
        a.wallpapers.is_empty() && a.slide_folders().is_empty(),
        "a built-in is neither a picture file nor a slideshow folder"
    );
    pick_row(&mut a, dusk + 1);
    assert_eq!(a.want.wallpaper, "reprise:ridges/dusk");
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("image = \"reprise:ridges/dusk\""), "{text}");
    pick_row(&mut a, 0);
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("image = \"\""), "{text}");
}

#[test]
fn a_slideshow_folder_survives_a_switch_to_picture_and_its_first_picture_reaches_the_monitor() {
    let dir = tmp("slide-folder");
    let (walls, trips) = (dir.join("Walls"), dir.join("Trips"));
    std::fs::create_dir_all(&walls).unwrap();
    std::fs::create_dir_all(&trips).unwrap();
    for f in ["anim.gif", "coast10.jpg", "Coast9.jpg"] {
        std::fs::write(trips.join(f), b"x").unwrap();
    }
    let path = dir.join("config.toml");
    let wall = walls.join("dunes.jpg");
    std::fs::write(&path, format!("[wallpaper]\nimage = {:?}\n", wall)).unwrap();
    let mut a = SettingsApp::open(path);
    let state = dir.join("settings.toml");
    a.state_path = Some(state.clone());
    let bg = Page::Background.index();
    let mode = |a: &mut SettingsApp, i: usize| {
        a.sheet.pages[bg].set_combo(backgroundpage::MODE, i);
        a.sheet_event(propsheet::Event::Page(
            bg,
            form::Event::Changed(backgroundpage::MODE),
        ));
    };
    let name = |p: &Path| p.to_string_lossy().into_owned();

    mode(&mut a, 1);
    assert_eq!(a.want.slideshow, name(&walls), "the wallpaper's folder");
    a.slide_folder(trips.clone());
    for _ in 0..500 {
        a.wake();
        if a.listing.is_none() && a.decoding.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        a.picture.as_ref().map(|p| p.path.clone()),
        Some(name(&trips.join("Coast9.jpg"))),
        "the desktop's first slide: no .gif, Coast9 before coast10"
    );

    mode(&mut a, 0);
    assert_eq!(a.want.slideshow, "");
    assert_eq!(winstate::slideshow(&state), name(&trips));
    mode(&mut a, 1);
    assert_eq!(a.want.slideshow, name(&trips), "the folder picked before");
}

#[test]
fn a_picture_added_to_the_slideshow_folder_becomes_the_monitors_first_slide() {
    let dir = tmp("slide-relist");
    let trips = dir.join("Trips");
    std::fs::create_dir_all(&trips).unwrap();
    std::fs::write(trips.join("coast.jpg"), b"x").unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, format!("[wallpaper]\nslideshow = {:?}\n", trips)).unwrap();
    let mut a = SettingsApp::open(path);
    let settle = |a: &mut SettingsApp| {
        for _ in 0..500 {
            a.wake();
            if a.listing.is_none() && a.decoding.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        a.picture.as_ref().map(|p| p.path.clone())
    };
    let name = |p: &Path| Some(p.to_string_lossy().into_owned());
    a.want_preview();
    assert_eq!(settle(&mut a), name(&trips.join("coast.jpg")));

    std::fs::write(trips.join("bay.jpg"), b"x").unwrap();
    a.slides_touched.store(true, Ordering::Release);
    assert_eq!(
        settle(&mut a),
        name(&trips.join("bay.jpg")),
        "bay sorts first"
    );
}

#[test]
fn the_script_open_combo_shows_and_writes_script_open() {
    let dir = tmp("script-open");
    let path = dir.join("config.toml");
    std::fs::write(&path, "terminal = \"foot\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone());
    let dp = Page::DefaultPrograms.index();
    let combo = |a: &SettingsApp| a.sheet.pages[dp].combo_selected(defaultspage::SCRIPT_OPEN);
    assert_eq!(combo(&a), Some(0), "Run it");
    let pick = |a: &mut SettingsApp, row| {
        a.sheet.pages[dp].set_combo(defaultspage::SCRIPT_OPEN, row);
        a.sheet_event(propsheet::Event::Page(
            dp,
            form::Event::Changed(defaultspage::SCRIPT_OPEN),
        ));
        assert!(a.apply());
        std::fs::read_to_string(&path).unwrap()
    };
    assert_eq!(
        pick(&mut a, 1),
        "terminal = \"foot\"\nscript_open = \"terminal\"\n"
    );
    assert_eq!(
        pick(&mut a, 0),
        "terminal = \"foot\"\n",
        "Run it is the default and removes the key"
    );
}

#[test]
fn the_script_combo_shows_and_writes_script_terminal() {
    let dir = tmp("script-end");
    let path = dir.join("config.toml");
    std::fs::write(&path, "terminal = \"foot\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone());
    let dp = Page::DefaultPrograms.index();
    let combo = |a: &SettingsApp| a.sheet.pages[dp].combo_selected(defaultspage::SCRIPT_END);
    assert_eq!(combo(&a), Some(0), "Keep its window open");
    let pick = |a: &mut SettingsApp, row| {
        a.sheet.pages[dp].set_combo(defaultspage::SCRIPT_END, row);
        a.sheet_event(propsheet::Event::Page(
            dp,
            form::Event::Changed(defaultspage::SCRIPT_END),
        ));
        assert!(a.apply());
        std::fs::read_to_string(&path).unwrap()
    };
    assert_eq!(
        pick(&mut a, 1),
        "terminal = \"foot\"\nscript_terminal = \"close\"\n"
    );
    assert_eq!(
        pick(&mut a, 0),
        "terminal = \"foot\"\n",
        "Keep its window open is the default and removes the key"
    );
}

#[test]
fn the_drive_combo_shows_and_writes_on_insert() {
    let dir = tmp("drive-insert");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[drives]\non_insert = \"mount\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone());
    let dp = Page::DefaultPrograms.index();
    assert_eq!(
        a.sheet.pages[dp].combo_selected(defaultspage::DRIVE_INSERT),
        Some(1),
        "Mount it"
    );
    a.sheet.pages[dp].set_combo(defaultspage::DRIVE_INSERT, 2);
    a.sheet_event(propsheet::Event::Page(
        dp,
        form::Event::Changed(defaultspage::DRIVE_INSERT),
    ));
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "[drives]\non_insert = \"open\"\n");
    a.sheet.pages[dp].set_combo(defaultspage::DRIVE_INSERT, 0);
    a.sheet_event(propsheet::Event::Page(
        dp,
        form::Event::Changed(defaultspage::DRIVE_INSERT),
    ));
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text, "",
        "Ask what to do is the default and removes the key"
    );
}

#[test]
fn a_drop_on_a_page_field_is_read_into_the_settings() {
    let mut a = app("drop");
    let mut ctx = test_ctx();
    let px = ctx.theme.fonts.size_px;
    a.sheet.tabs.open = Page::Advanced.index();
    let form = a.drop_form(MAIN).expect("the open page").form;
    let r = form
        .item("task_manager")
        .unwrap()
        .placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
    assert!(form.drag_over(r.x + 4, r.y + r.height / 2, &ctx.text, px));
    let changed = form.drop_text(
        &reprisekit::textedit::DropLine::Text("htop".into()),
        &ctx.text,
        px,
    );
    assert_eq!(changed, form::Event::Changed("task_manager"));
    a.event(MAIN, Event::FieldDropped(changed), &mut ctx);
    assert!(
        a.want.task_manager.contains("htop"),
        "{}",
        a.want.task_manager
    );
}

#[test]
fn a_config_written_elsewhere_shows_on_the_open_sheet_and_apply_keeps_it() {
    let path = tmp("config-changed").join("config.toml");
    std::fs::write(&path, "").unwrap();
    let mut a = SettingsApp::open(path.clone());
    a.want.slow_keys = true;
    std::fs::write(&path, "[accessibility]\nmouse_keys = true\n").unwrap();
    assert!(a.config_changed());
    let ax = &a.sheet.pages[Page::Accessibility.index()];
    assert_eq!(ax.checked("mouse_keys"), Some(true));
    let speed = &ax.item("mouse_keys_speed").unwrap().control;
    assert!(
        matches!(speed, form::Control::TrackBar(t) if t.enabled),
        "its controls ungrey"
    );
    assert_eq!(ax.checked("slow_keys"), Some(true), "a pending check stays");
    assert!(!a.config_changed(), "a file read as before changes nothing");
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("mouse_keys = true") && text.contains("slow_keys = true"),
        "{text}"
    );
}

#[test]
fn a_later_start_opens_its_page_unless_a_dialog_is_up() {
    let mut a = app("request");
    assert!(a.request(&page_request("fonts-and-icons")));
    assert_eq!(a.sheet.tabs.open, Page::FontsAndIcons.index());
    assert!(!a.request(&page_request("nowhere")));
    assert!(!a.request(&instance::Request::default()));
    a.fail("broken");
    assert!(!a.request(&page_request("startup")));
    assert_eq!(a.sheet.tabs.open, Page::FontsAndIcons.index());
}

#[test]
fn a_program_row_edits_its_keys_and_delete_forgets_it() {
    let dir = tmp("programs");
    std::fs::write(
        dir.join("config.toml"),
        "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
         description = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [portal]\nshortcuts_denied = [\"org.example.NoSuchNotes\"]\n",
    )
    .unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"));
    let n = app.want.shortcuts.len();
    assert_eq!(
        app.shortcuts.rows[n].cells,
        ["Pause", "org.example.NoSuchApp: Hold to talk"]
    );
    app.shortcuts.selection.set_only(Some(n + 1));
    app.edit_shortcut();
    assert!(app.shortcut.is_none(), "a refusal has nothing to edit");

    app.shortcuts.selection.set_only(Some(n));
    app.edit_shortcut();
    let d = app.shortcut.as_mut().expect("the dialog is up");
    assert_eq!(d.title, "Change Shortcut");
    d.event(form::Event::Pressed(shortcutdlg::CLEAR));
    let shortcutdlg::Outcome::Accept(sc) = d.event(form::Event::Pressed(shortcutdlg::OK)) else {
        panic!("no keys is a valid answer");
    };
    assert_eq!(app.accept_shortcut(sc), n);
    assert_eq!(app.want.programs[0].keys, "");
    assert_eq!(app.want.shortcuts.len(), n, "no bind was added");
    assert_eq!(app.shortcuts.rows[n].cells[0], "None");

    app.shortcuts.selection.set_only(Some(n + 1));
    app.remove_shortcut();
    assert!(app.want.denied.is_empty());
    app.shortcuts.selection.set_only(Some(n));
    app.remove_shortcut();
    assert!(app.want.programs.is_empty());
    assert_eq!(app.shortcuts.rows.len(), n);
}

#[test]
fn a_rebound_zoom_key_rewrites_the_vision_note() {
    let dir = tmp("zoom-note");
    std::fs::write(dir.join("config.toml"), "").unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"));
    let note = |app: &SettingsApp| match &app.sheet.pages[Page::Vision.index()]
        .item("mag_note")
        .unwrap()
        .control
    {
        form::Control::Lines(lines) => lines.join(" "),
        _ => panic!("the note is lines"),
    };
    assert!(note(&app).starts_with("Super+= zooms in"), "{}", note(&app));
    let i = (app.want.shortcuts.iter())
        .position(|sc| sc.action == "zoom-in")
        .expect("zoom-in is bound by default");
    app.shortcut_target = Some(model::Row::Bind(i));
    app.accept_shortcut(model::Shortcut::new("Ctrl+Alt+plus", "zoom-in", ""));
    assert!(
        note(&app).starts_with("Ctrl+Alt++ zooms in"),
        "{}",
        note(&app)
    );
}

#[test]
fn a_programs_taken_keys_name_the_program() {
    let dir = tmp("program-taken");
    std::fs::write(
        dir.join("config.toml"),
        "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
         description = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"mute\"\nkeys = \"F9\"\n",
    )
    .unwrap();
    let app = SettingsApp::open(dir.join("config.toml"));
    let taken = app.taken_keys(None);
    let n = app.want.shortcuts.len();
    assert_eq!(taken[n].action, "org.example.NoSuchApp: Hold to talk");
    assert_eq!(
        taken[n + 1].action,
        "org.example.NoSuchApp: mute",
        "no description: the id"
    );
}

/// DP-1 listed with a smaller resolution picked, Apply or OK pressed
/// and taken, so the question is up.
fn question_up(tag: &str, press: displayhost::Press) -> (SettingsApp, Ctx) {
    let mut app = app(tag);
    let mut ctx = test_ctx();
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    app.display.model.pick_resolution(1);
    app.start_display(press, &mut ctx.heads);
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "the mode is asked for");
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: true,
        },
        &mut ctx,
    );
    assert!(r.tick, "the countdown runs on the main window's ticks");
    assert!(
        app.children()
            .iter()
            .any(|c| c.id == displayhost::MONITOR_WIN),
        "Monitor Settings is up"
    );
    (app, ctx)
}

#[test]
fn the_questions_close_box_puts_the_old_mode_back_and_keeps_the_sheet() {
    let (mut app, mut ctx) = question_up("question-close", displayhost::Press::Apply);
    let r = app.event(displayhost::MONITOR_WIN, Event::CloseRequest, &mut ctx);
    assert!(!r.quit);
    assert!(app.display.flow.question.is_none());
    let back = ctx.heads.take();
    assert_eq!(back.len(), 1);
    assert_eq!(
        back[0].1,
        vec![("DP-1".to_string(), displaypage::test_head("DP-1").state())]
    );
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: back[0].0,
            ok: true,
        },
        &mut ctx,
    );
    assert!(!r.quit, "the sheet stays open after the put-back");
    assert!(!app.path.exists(), "nothing written");
}

#[test]
fn yes_after_ok_writes_the_output_and_quits() {
    let (mut app, mut ctx) = question_up("question-yes", displayhost::Press::Ok);
    let q = app.display.flow.question.as_ref().expect("the question");
    let yes = q.dialog.form.item(monitordlg::YES).expect("a Yes button");
    let r = yes.placed(&ctx.forms().controls());
    let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
    let mods = repriseapp::Mods::default();
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods,
    };
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods,
    };
    let win = displayhost::MONITOR_WIN;
    assert!(!app.event(win, Event::Pointer(press), &mut ctx).quit);
    assert!(app.event(win, Event::Pointer(release), &mut ctx).quit);
    let text = std::fs::read_to_string(&app.path).expect("config written");
    assert!(
        text.contains("name = \"DP-1\"") && text.contains("mode = \"1280x720@60\""),
        "DP-1's [[outputs]] entry has the new mode: {text}"
    );
}

/// Two outputs side by side; the second dragged under the first and
/// made the primary one. Apply writes the primary at once and asks
/// for both places in one request; Yes writes both.
#[test]
fn a_kept_arrangement_writes_every_place_and_the_primary() {
    let mut app = app("arrangement");
    let mut ctx = test_ctx();
    let mut b = displaypage::test_head("HDMI-A-1");
    b.position = (1920, 0);
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1"), b]),
        &mut ctx,
    );
    let mut well = reprisekit::controls::arrange::Arrange::new(app.display.model.tiles(), 1);
    well.tiles[1].pos = (0, 1080);
    app.display.model.set_arrangement(&well);
    app.display.model.make_primary();
    app.start_display(displayhost::Press::Apply, &mut ctx.heads);
    let text = std::fs::read_to_string(&app.path).expect("the primary is written at once");
    assert!(text.contains("primary = true"), "{text}");
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "one request");
    let places: Vec<_> = asked[0]
        .1
        .iter()
        .map(|(n, c)| (n.as_str(), c.position))
        .collect();
    assert_eq!(
        places,
        vec![("DP-1", Some((0, 0))), ("HDMI-A-1", Some((0, 1080)))]
    );
    app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: true,
        },
        &mut ctx,
    );
    let effects = app.display.flow.answer(true, &mut ctx.heads);
    app.display_effects(effects);
    let text = std::fs::read_to_string(&app.path).unwrap();
    assert_eq!(
        outputwrite::read_primary(&text).as_deref(),
        Some("HDMI-A-1")
    );
    assert!(
        text.contains("position = [0, 0]") && text.contains("position = [0, 1080]"),
        "{text}"
    );
}

#[test]
fn a_refused_vrr_on_ok_says_so_and_keeps_the_sheet() {
    let mut app = app("vrr-refused");
    let mut ctx = test_ctx();
    // The test output supports VRR and runs without it.
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    app.display.model.set_vrr(true);
    let r = app.start_display(displayhost::Press::Ok, &mut ctx.heads);
    assert!(!r.quit, "waits for the answer");
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "only VRR is asked for");
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: false,
        },
        &mut ctx,
    );
    assert!(!r.quit, "the sheet stays open");
    let text = app.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(text.as_deref(), Some(displayflow::VRR_REFUSED));
    assert!(
        app.children().iter().any(|c| c.id == ERROR_WIN),
        "the error box is up"
    );
    assert!(!app.display.model.pick().unwrap().vrr, "the box unticks");
}

#[test]
fn a_save_that_fails_shows_why_and_does_not_quit() {
    let mut app = app("save-fails");
    std::fs::write(&app.path, "[general\n").expect("write");
    let keys = outputwrite::OutputKeys {
        adaptive_sync: Some(true),
        ..outputwrite::OutputKeys::default()
    };
    let r = app.display_effects(vec![
        displayflow::Effect::Save("DP-1".into(), keys),
        displayflow::Effect::Quit,
    ]);
    assert!(!r.quit, "the error stays on screen");
    let text = app.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(
        text.as_deref(),
        Some("config.toml does not parse; the display setting was not saved.")
    );
}

/// A resize lays every page out afresh and keeps what the controls hold; the size is kept for the next run unless maximized.
#[test]
fn the_window_resizes_and_its_size_is_kept() {
    let mut app = app("resize");
    assert!(!app.fixed());
    let min = app.min_size(MAIN).unwrap();
    assert_eq!(min, (544, 528));
    assert_eq!(app.sheet.size, min, "built at its smallest");
    let (w, h) = winstate::DEFAULT;
    assert!(
        w * 10 >= min.0 * 13 && h * 10 >= min.1 * 12,
        "opens noticeably larger"
    );
    let state = tmp("resize-state").join("settings.toml");
    app.state_path = Some(state.clone());
    let startup = Page::Startup.index();
    let add = |app: &SettingsApp| app.sheet.pages[startup].item(pages::ADD).unwrap().rect;
    let before = add(&app);
    let windows = Page::Windows.index();
    let tabs = !app.sheet.pages[windows].checked("window_tabs").unwrap();
    app.sheet.pages[windows].set_check("window_tabs", tabs);

    let mut ctx = test_ctx();
    ctx.size = (900, 700);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (900, 700));
    let after = add(&app);
    assert_eq!(
        (after.x, after.y),
        (before.x + 356, before.y + 172),
        "Add... keeps to the longer, wider list's corner"
    );
    assert_eq!(app.sheet.pages[windows].checked("window_tabs"), Some(tabs));
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(winstate::load(&state, min, 100), (900, 700), "kept");

    ctx.size = (1900, 1000);
    ctx.maximized = true;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (1900, 1000));
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(
        winstate::load(&state, min, 100),
        (900, 700),
        "not the screen's"
    );

    ctx.size = (300, 200);
    ctx.maximized = false;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, min, "no smaller than its smallest");
    assert_eq!(add(&app), before);
}

#[test]
fn a_window_shorter_than_the_sheet_keeps_the_open_page_in_the_list() {
    let mut app = app("short-list");
    app.sheet.tabs.open = Page::Advanced.index();
    app.sheet.reveal_open();
    let mut ctx = test_ctx();
    let min = app.min_size(MAIN).unwrap();
    ctx.size = (min.0, 300);
    app.event(MAIN, Event::Resized, &mut ctx);
    let lay = app
        .sheet
        .layout(&ctx.text, ctx.theme.fonts.size_px)
        .list
        .expect("list mode");
    assert!(lay.scrollbar.is_some(), "the list overflows");
    let propsheet::Nav::List(list) = &app.sheet.nav else {
        panic!("list mode");
    };
    let i = list
        .rows
        .iter()
        .position(|r| *r == reprisekit::controls::pagelist::Row::Page(Page::Advanced.index()))
        .unwrap();
    let (row, inner) = (lay.rows[i], lay.inner);
    assert!(
        row.y >= inner.y && row.bottom() <= inner.bottom(),
        "Advanced's row {row:?} shows inside {inner:?}"
    );
}

/// At 200% on a 1080-line screen the window opens at its pick held to the work area; that
/// size is not kept, so the size left at 100% stands. A size the user gives it is kept.
#[test]
fn a_held_opening_size_is_not_kept() {
    let mut app = app("held");
    let min = app.min_size(MAIN).unwrap();
    let state = tmp("held-state").join("settings.toml");
    app.state_path = Some(state.clone());
    winstate::save(&state, (900, 700), 100);
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1088, 1026);
    ctx.at_pick = true;
    app.event(MAIN, Event::Resized, &mut ctx);
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(winstate::load(&state, min, 100), (900, 700));

    ctx.size = (1200, 1000);
    ctx.at_pick = false;
    app.event(MAIN, Event::Resized, &mut ctx);
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(
        winstate::kept(&state),
        winstate::Kept {
            size: (1200, 1000),
            text_size: Some(200)
        }
    );
}

#[test]
fn at_150_percent_the_window_holds_the_grown_sheet() {
    let mut app = app("resize-150");
    let mut ctx = test_ctx();
    ctx.text_size = 150;
    let u = TextUnits::new(17, 11);
    assert_eq!(ctx.forms().text, u);
    ctx.size = (1000, 900);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.min_size(MAIN), Some(u.size((544, 528))));
    let (w, h) = app.sheet.size;
    assert_eq!(app.sheet.client(), u.size((w, h)));
    assert!(
        u.len(w) <= 1000 && u.len(w + 1) > 1000,
        "{w} is the widest that fits"
    );
    assert!(
        u.len(h) <= 900 && u.len(h + 1) > 900,
        "{h} is the tallest that fits"
    );

    ctx.size = (300, 200);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (544, 528), "no smaller than its smallest");

    app.rename = Some(renamedlg::RenameDialog::new("Name"));
    let rename = app.children().into_iter().find(|c| c.id == RENAME_WIN);
    assert_eq!(rename.map(|c| c.size), Some(u.size(renamedlg::CLIENT)));
}

/// The size Settings opens at follows the text size live: its smallest sheet at 200%, the
/// opening size again back at 100%, whatever size the window took in between.
#[test]
fn the_opening_size_follows_the_text_size() {
    let mut app = app("opening-size");
    app.kept = winstate::Kept {
        size: winstate::DEFAULT,
        text_size: None,
    };
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1088, 1026);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.initial_size(), (1088, 1056));
    ctx.text_size = 100;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.initial_size(), winstate::DEFAULT);
}

/// At 200% on a 1080-line screen the work area is shorter than the smallest sheet: the
/// sheet keeps the pages it was built with and its buttons sit inside the window. Every page's
/// controls end above the window's bottom, so no page scrolls.
#[test]
fn at_200_percent_a_window_shorter_than_the_sheet_keeps_its_buttons_in_it() {
    let mut app = app("resize-200");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    let u = ctx.forms().text;
    let min = app.min_size(MAIN).unwrap();
    assert!(
        u.len(528) > 1026,
        "the smallest sheet is taller than the work area"
    );
    ctx.size = (1914, 1026);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.min_size(MAIN), Some(u.size((544, 528))), "{min:?}");
    assert_eq!(
        app.sheet.size.1, 528,
        "the pages keep their smallest height"
    );
    let client = app.sheet.client();
    assert!(client.1 <= 1026, "{client:?} fits the window");
    let mut c = Canvas::new(1914, 1026);
    app.draw(MAIN, &mut c, &ctx);
    for id in [propsheet::OK, propsheet::CANCEL, propsheet::APPLY] {
        let b = app.sheet.open().rect(id).unwrap();
        assert!(b.bottom() <= 1026, "{id} at {b:?} shows");
    }
    for pg in Page::all() {
        app.sheet.tabs.open = pg.index();
        app.draw(MAIN, &mut c, &ctx);
        let l = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
        assert_eq!(l.bar, None, "{pg:?} ends above the window's bottom");
    }
}

/// At 200% on a screen 1000 px wide the smallest sheet is wider than the window: the
/// buttons keep to the window's corner, and the page scrolls sideways to its right column.
#[test]
fn at_200_percent_a_window_narrower_than_the_sheet_scrolls_its_page_sideways() {
    let mut app = app("narrow-200");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1000, 1026);
    app.sheet.tabs.open = Page::Background.index();
    app.event(MAIN, Event::Resized, &mut ctx);
    let mut c = Canvas::new(1000, 1026);
    app.draw(MAIN, &mut c, &ctx);
    let client = app.sheet.client();
    assert!(client.0 <= 1000, "{client:?} fits the window");
    let l = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
    assert!(l.apply.right() <= 1000, "Apply at {:?} shows", l.apply);
    let hbar = l.hbar.expect("the page scrolls sideways");
    let browse = app.sheet.open().rect(backgroundpage::BROWSE).unwrap();
    assert!(
        browse.right() > hbar.right(),
        "Browse... starts past the page's edge"
    );
    app.sheet
        .wheel(Some(propsheet::Hit::PageHBar(BarHit::Track)), 100);
    let browse = app.sheet.open().rect(backgroundpage::BROWSE).unwrap();
    let bar = l.bar.map_or(hbar.right(), |b| b.x);
    assert!(browse.right() <= bar, "{browse:?} shows");
}

/// On a page cut narrow and short the Keyboard page's shortcut list runs on under the page's
/// scrollbar: a press on its arrow there scrolls the page, and the list keeps its selection.
#[test]
fn the_page_scrollbar_over_the_side_list_scrolls_the_page() {
    let mut app = app("bar-over-list");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1000, 700);
    app.sheet.tabs.open = Page::Keyboard.index();
    app.event(MAIN, Event::Resized, &mut ctx);
    let mut c = Canvas::new(1000, 700);
    app.draw(MAIN, &mut c, &ctx);
    let bar = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px).bar;
    let bar = bar.expect("the page scrolls");
    let list = app.on_page(pages::shortcuts(app.area()));
    let y = bar.bottom() - 4;
    assert!(
        list.contains(bar.x + 2, y),
        "{list:?} runs under the down arrow of {bar:?}"
    );
    let selected = app.shortcuts.selection.focus;
    let press = PointerInput::Press {
        x: bar.x + 2,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    app.event(MAIN, Event::Pointer(press), &mut ctx);
    assert!(app.sheet.page_offset().1 < 0, "the page scrolled down");
    assert_eq!(app.shortcuts.selection.focus, selected);
}

#[test]
fn the_page_argument_names_a_page() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string())).map(|a| a.0);
    assert_eq!(args(&["--page", "night-light"]), Some(Page::NightLight));
    assert_eq!(args(&["--page=fonts-and-icons"]), Some(Page::FontsAndIcons));
    assert_eq!(args(&["--page", "startup"]), Some(Page::Startup));
    assert_eq!(args(&["--page", "appearance"]), Some(Page::Appearance));
    assert_eq!(args(&["--page", "background"]), Some(Page::Background));
    assert_eq!(args(&["--page", "nowhere"]), None);
    assert_eq!(args(&[]), None);
}

#[test]
fn the_focus_argument_names_a_control_on_the_page() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string()));
    let mouse_keys = Some((Page::Accessibility, Some("mouse_keys".to_string())));
    assert_eq!(
        args(&["--page", "accessibility", "--focus", "mouse_keys"]),
        mouse_keys
    );
    assert_eq!(
        args(&["--focus=mouse_keys", "--page=accessibility"]),
        mouse_keys
    );
    assert_eq!(args(&["--focus", "mouse_keys"]), None, "no page, no focus");
}

#[test]
fn a_request_with_a_control_opens_its_page_with_the_keyboard_on_it() {
    let mut a = app("focus-request");
    let req = |page: &str, id: &str| instance::Request {
        action: Some((PAGE_ACTION.into(), vec![page.into(), id.into()])),
        handoff: Default::default(),
    };
    let focused = |a: &SettingsApp| {
        let page = a.sheet.open();
        page.focus().map(|i| page.items[i].id)
    };
    assert!(a.request(&req("accessibility", "mouse_keys")));
    assert_eq!(a.sheet.tabs.open, Page::Accessibility.index());
    assert!(
        !a.sheet.tabs.focused,
        "the page strip lets go of the keyboard"
    );
    assert_eq!(focused(&a), Some("mouse_keys"));
    assert!(
        a.request(&req("accessibility", "sticky_keys")),
        "the open page still moves the focus"
    );
    assert_eq!(focused(&a), Some("sticky_keys"));
}

#[test]
fn a_missing_config_opens_on_the_defaults_without_an_error() {
    let dir = tmp("missing");
    assert_eq!(read_config(&dir.join("config.toml")), (String::new(), None));
}

#[test]
fn a_config_that_does_not_parse_names_its_line() {
    let path = tmp("broken").join("config.toml");
    std::fs::write(&path, "[general]\nx = \n").expect("write");
    let (text, err) = read_config(&path);
    assert_eq!(text, "[general]\nx = \n");
    let err = err.expect("a parse error for the box");
    assert!(
        err.starts_with("config.toml does not parse at line 2: "),
        "the box names line 2, got {err:?}"
    );
}

#[test]
fn an_unreadable_config_is_reported() {
    // A directory where the file should be fails the read with EISDIR.
    let path = tmp("unreadable").join("config.toml");
    std::fs::create_dir_all(&path).expect("mkdir");
    let (text, err) = read_config(&path);
    assert_eq!(text, "");
    let err = err.expect("a read error for the box");
    assert!(
        err.starts_with("config.toml could not be read: "),
        "the box says the read failed, got {err:?}"
    );
}

fn forget_enabled(app: &SettingsApp) -> bool {
    match &app.sheet.pages[Page::Advanced.index()]
        .item(pages::FORGET)
        .unwrap()
        .control
    {
        form::Control::Button { enabled, .. } => *enabled,
        _ => panic!("Forget is a button"),
    }
}

#[test]
fn location_answers_list_the_prompts_then_the_portals() {
    let agent = std::collections::BTreeMap::from([("org.example.Weather".to_string(), false)]);
    let portal = vec![("org.gnome.Maps".to_string(), true)];
    let got: Vec<(String, bool, bool)> = location_answers(agent, portal)
        .into_iter()
        .map(|a| (a.id, a.allow, a.portal))
        .collect();
    assert_eq!(
        got,
        [
            ("org.example.Weather".to_string(), false, false),
            ("org.gnome.Maps".to_string(), true, true),
        ]
    );
}

/// Show the Advanced page and wait for the location read it starts.
fn read_locations_on_advanced(app: &mut SettingsApp) {
    app.sheet.tabs.open = Page::Advanced.index();
    app.ensure_locations();
    let found = app
        .locating
        .take()
        .expect("a read under way")
        .recv()
        .unwrap();
    app.show_locations(found);
}

#[test]
fn the_location_list_is_read_off_the_loop_once_the_advanced_page_shows() {
    let dir = tmp("location-lazy");
    let file = dir.join("location.toml");
    std::fs::write(&file, "[apps]\n\"org.example.NoSuchMaps\" = true\n").unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"));
    app.location_file = file;
    assert!(app.locations.rows.is_empty(), "opening reads no answers");
    app.ensure_locations();
    assert!(app.locating.is_none(), "no read while another page shows");

    app.sheet.tabs.open = Page::Advanced.index();
    app.ensure_locations();
    assert!(app.locating.is_some(), "the Advanced page starts the read");
    let cells: Vec<Vec<String>> = app.locations.rows.iter().map(|r| r.cells.clone()).collect();
    assert_eq!(cells, [vec!["Loading...".to_string(), String::new()]]);
    app.locations.selection.set_only(Some(0));
    app.sync_apply();
    assert!(!forget_enabled(&app), "Forget stays greyed while loading");

    let found = app.locating.take().unwrap().recv().unwrap();
    app.show_locations(found);
    assert_eq!(app.locations.rows.len(), 1);
    assert_eq!(
        app.locations.rows[0].cells,
        ["org.example.NoSuchMaps", "Allowed"]
    );
    assert_eq!(
        app.locations.selection.focus, None,
        "the loading row's pick is dropped"
    );
    app.ensure_locations();
    assert!(app.locating.is_none(), "the list is read once");
}

#[test]
fn forget_drops_a_location_answer_from_the_file() {
    let dir = tmp("location");
    let file = dir.join("location.toml");
    std::fs::write(
        &file,
        "[apps]\n\"org.example.NoSuchMaps\" = true\n\"org.example.NoSuchWeather\" = false\n",
    )
    .unwrap();
    let mut app = SettingsApp::open(dir.join("config.toml"));
    app.location_file = file.clone();
    read_locations_on_advanced(&mut app);
    let cells: Vec<Vec<String>> = app.locations.rows.iter().map(|r| r.cells.clone()).collect();
    assert_eq!(
        cells,
        [
            ["org.example.NoSuchMaps", "Allowed"],
            ["org.example.NoSuchWeather", "Denied"],
        ]
        .map(|r| r.map(String::from).to_vec())
    );
    assert!(!forget_enabled(&app), "Forget greys with no row picked");

    app.locations.selection.set_only(Some(0));
    app.sync_apply();
    assert!(forget_enabled(&app));
    app.forget_location();
    assert_eq!(
        repriseconf::location::load_from(&file),
        std::collections::BTreeMap::from([("org.example.NoSuchWeather".to_string(), false)])
    );
    assert_eq!(app.locations.rows.len(), 1);
    assert_eq!(app.locations.rows[0].cells[0], "org.example.NoSuchWeather");
    assert!(!app.dirty(), "Forget leaves nothing for Apply");
}
