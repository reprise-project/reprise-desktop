use super::*;
use reprisebus::airplane::Mode;
use reprisebus::bolt::{Device, Found, Status};
use reprisekit::controls::hosted::{Anchor, Hosted};
use reprisekit::raster::Rect;

#[test]
fn the_script_open_combo_shows_and_writes_script_open() {
    let dir = tmp("script-open");
    let path = dir.join("config.toml");
    std::fs::write(&path, "terminal = \"foot\"\n").unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
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
    let mut a = SettingsApp::open(path.clone(), "us".into());
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
    let mut a = SettingsApp::open(path.clone(), "us".into());
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

fn remote_label(a: &SettingsApp, id: &str) -> String {
    match &a.sheet.pages[Page::RemoteDesktop.index()]
        .item(id)
        .unwrap()
        .control
    {
        form::Control::Label { text, .. } => text.clone(),
        c => panic!("{id} is a label, not {c:?}"),
    }
}

#[test]
fn a_remote_desktop_server_whose_unit_failed_says_why_on_the_page() {
    let mut a = app("remote");
    let failed = remotehost::Remote {
        enabled: true,
        stopped: Some("A library it needs did not load.".into()),
        ..remotehost::Remote::default()
    };
    remote_read(&mut a, failed, None);
    assert_eq!(
        remote_label(&a, "status"),
        "Not running. A library it needs did not load."
    );
    let starting = remotehost::Remote {
        enabled: true,
        ..remotehost::Remote::default()
    };
    remote_read(&mut a, starting, None);
    assert_eq!(remote_label(&a, "status"), "Not answering");
}

#[test]
fn the_remote_desktop_page_follows_what_the_server_says() {
    let mut a = app("remote");
    let status = reprisebus::remote::Status {
        listening: "0.0.0.0:3389".into(),
        clients: vec![reprisebus::remote::Client {
            address: "192.168.1.30:51234".into(),
            user: "user".into(),
            since: 1_762_000_000,
        }],
        encoder: "VA-API H.264".into(),
        encoders: vec!["vaapi".into(), "none".into()],
        fingerprint: "3F:9A".into(),
        thumbprint: "3f9a".into(),
    };
    let serving = remotehost::Remote {
        enabled: true,
        status: Some(status),
        address: Some("192.168.1.20".into()),
        ..remotehost::Remote::default()
    };
    remote_read(&mut a, serving, None);
    let page = &a.sheet.pages[Page::RemoteDesktop.index()];
    assert_eq!(page.checked(pages::REMOTE_ALLOW), Some(true));
    assert_eq!(
        remote_label(&a, "status"),
        "Listening on port 3389 at 192.168.1.20"
    );
    assert_eq!(remote_label(&a, "encoder"), "Video: VA-API H.264");
    assert_eq!(page.edit_text("fingerprint"), "3F:9A");
    assert!(!a.error.is_shown());
    assert_eq!(a.want, a.loaded, "the switch is no change for Apply");
    a.sheet.pages[Page::RemoteDesktop.index()].set_combo(pages::REMOTE_PRINT, 1);
    let picked = form::Event::Changed(pages::REMOTE_PRINT);
    a.sheet_event(propsheet::Event::Page(Page::RemoteDesktop.index(), picked));
    let page = &a.sheet.pages[Page::RemoteDesktop.index()];
    assert_eq!(page.edit_text("fingerprint"), "3f9a", "the thumbprint");
    assert_eq!(a.want, a.loaded, "the pick is no change for Apply");

    remote_read(
        &mut a,
        remotehost::Remote::default(),
        Some("Remote Desktop could not be turned on.".into()),
    );
    assert!(a.error.is_shown(), "a failed switch says why");
    let page = &a.sheet.pages[Page::RemoteDesktop.index()];
    assert_eq!(page.checked(pages::REMOTE_ALLOW), Some(false));
    assert_eq!(remote_label(&a, "status"), "Off");
    assert_eq!(page.edit_text("fingerprint"), "");
}

#[test]
fn other_on_the_colour_button_opens_the_color_dialog_whose_ok_sets_the_desktop() {
    use reprisekit::controls::colordlg::{BASIC, Grid, Well};
    let mut a = app("othercolor");
    let state = tmp("othercolor-state").join("settings.toml");
    a.state_path = Some(state.clone());
    let mut ctx = test_ctx();
    let p = Page::Background.index();
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Pressed("background"),
    ));
    assert!(a.children().iter().any(|c| c.id == COLOR_WIN));
    let (d, ..) = a.color_dlg.as_mut().unwrap();
    d.dlg.pick(Well {
        grid: Grid::Basic,
        i: 28,
    });
    d.dlg.add_custom();
    let enter = Event::Key(key_input(keysym::RETURN, Mods::default(), None));
    a.event(COLOR_WIN, enter, &mut ctx);
    assert!(a.color_dlg.is_none(), "Enter is OK");
    assert_eq!(BASIC[28], reprisekit::theme::Color::rgb(0, 0, 0xFF));
    assert_eq!(a.want.background, "#0000FF");
    assert_eq!(winstate::custom_colors(&state)[0], BASIC[28]);
    let _ = std::fs::remove_dir_all(state.parent().unwrap());
}

#[test]
fn an_error_box_mid_drag_lets_go_of_the_color_dialogs_field() {
    use reprisekit::controls::colordlg;
    let mut a = app("box-drops-color-drag");
    let mut ctx = test_ctx();
    let p = Page::Background.index();
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Pressed("background"),
    ));
    let (d, ..) = a.color_dlg.as_mut().expect("the Color dialog");
    d.dlg
        .form_event(&mut d.form, form::Event::Pressed(colordlg::DEFINE));
    d.form.drawn_with(&ctx.theme.paint());
    let field = colordlg::layout(&d.form).field.expect("widened").inset(2);
    let press = PointerInput::Press {
        x: field.x + 60,
        y: field.y + 30,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    a.event(COLOR_WIN, Event::Pointer(press), &mut ctx);
    let hsl = |a: &SettingsApp| a.color_dlg.as_ref().expect("still open").0.dlg.hsl();
    let held = hsl(&a);
    fail_under_box(&mut a, &ctx, "refused");
    close_box(&mut a, &mut ctx);
    let corner = PointerInput::Motion {
        x: field.x,
        y: field.y,
    };
    a.event(COLOR_WIN, Event::Pointer(corner), &mut ctx);
    assert_eq!(hsl(&a), held, "the colour stays where the press put it");
}

#[test]
fn remove_greys_on_a_user_file_standing_in_for_a_system_entry() {
    let mut a = app("stand-in");
    let parse = |id, text, user| reprisefs::autostart::parse(id, text, user).unwrap();
    let sys = parse("vol", "[Desktop Entry]\nName=Volume\nExec=vol\n", false);
    let shadow = parse(
        "vol",
        "[Desktop Entry]\nName=Volume\nExec=vol\nHidden=true\nX-Reprise-Override=true\n",
        true,
    );
    let mine = parse("mine", "[Desktop Entry]\nName=Mine\nExec=mine\n", true);
    a.entries = reprisefs::autostart::merge(vec![sys], vec![shadow, mine]);
    a.applied_checks = vec![true, false];
    pages::fill_list(&mut a.list, &a.entries, &[true, false]);
    let remove_lit = |a: &SettingsApp| match &a.sheet.pages[Page::Startup.index()]
        .item(pages::REMOVE)
        .unwrap()
        .control
    {
        form::Control::Button { enabled, .. } => *enabled,
        c => panic!("Remove is a button, not {c:?}"),
    };
    let row = |a: &SettingsApp, id: &str| a.entries.iter().position(|e| e.id == id);
    a.list.selection.focus = row(&a, "vol");
    a.sync_apply();
    assert!(
        !remove_lit(&a),
        "the unchecked system entry is unchecked, not removed"
    );
    a.list.selection.focus = row(&a, "mine");
    a.sync_apply();
    assert!(remove_lit(&a), "the user's own entry");
}

#[test]
fn unchecking_run_as_asks_first_and_only_ok_unchecks_it() {
    let mut a = app("runas");
    let mut ctx = test_ctx();
    let cm = ctx.forms().controls();
    a.entries = vec![
        reprisefs::autostart::parse(
            autostart::RUN_AS,
            "[Desktop Entry]\nName=Run As\nExec=reprise-polkit\n",
            false,
        )
        .unwrap(),
    ];
    a.applied_checks = vec![true];
    pages::fill_list(&mut a.list, &a.entries, &[true]);
    a.sheet.tabs.open = Page::Startup.index();
    a.list.selection.focus = Some(0);
    let key = |sym| key_input(sym, Mods::default(), None);

    a.list_key(&form::Key::Space, &cm);
    assert_eq!(
        a.list.checks,
        Some(vec![true]),
        "checked until the question is answered"
    );
    assert_eq!(
        a.asking.up().map(|(b, _)| b.fields.text.as_str()),
        Some(apply::run_as_question().as_str())
    );
    let win = a.asking.win().expect("the question is up");
    a.event(win, Event::Key(key(keysym::ESCAPE)), &mut ctx);
    assert!(a.asking.is_empty());
    assert_eq!(a.list.checks, Some(vec![true]), "Cancel keeps it");

    a.list_key(&form::Key::Space, &cm);
    let win = a.asking.win().expect("the question is up again");
    a.event(win, Event::Key(key(keysym::RETURN)), &mut ctx);
    assert_eq!(a.list.checks, Some(vec![false]), "OK unchecks it");
    assert!(a.dirty(), "Apply writes it");
}

#[test]
fn a_question_asked_while_one_is_up_comes_after_it_and_each_answer_runs_its_own_step() {
    let mut a = app("two-questions");
    let mut ctx = test_ctx();
    let entry = |name: &str| {
        let text = format!("[Desktop Entry]\nName={name}\nExec={name}\n");
        reprisefs::autostart::parse(&format!("{name}.desktop"), &text, false).unwrap()
    };
    a.entries = vec![entry("one"), entry("two")];
    a.applied_checks = vec![true, true];
    pages::fill_list(&mut a.list, &a.entries, &[true, true]);
    a.ask(
        "Stop one?".into(),
        crate::backuphost::Then::StopAutostart(0),
    );
    a.ask(
        "Stop two?".into(),
        crate::backuphost::Then::StopAutostart(1),
    );
    let key = |sym| Event::Key(key_input(sym, Mods::default(), None));
    let up = |a: &SettingsApp| {
        let (b, _) = a.asking.up()?;
        Some((b.fields.text.clone(), a.asking.win()?))
    };

    let (text, win) = up(&a).expect("a question is up");
    assert_eq!(text, "Stop one?", "the second question waits");
    a.event(win, key(keysym::ESCAPE), &mut ctx);
    assert_eq!(
        a.list.checks,
        Some(vec![true, true]),
        "Cancel stops nothing"
    );

    let (text, win) = up(&a).expect("the second question is up after the first's answer");
    assert_eq!(text, "Stop two?");
    a.event(win, key(keysym::RETURN), &mut ctx);
    assert_eq!(
        a.list.checks,
        Some(vec![true, false]),
        "OK stops the entry its own question named"
    );
    assert!(up(&a).is_none());
}

/// The Search page's content note waits for its read, then says what the contents check starts;
/// the note and the check grey while localsearch is missing, the check while file search is off.
#[test]
fn the_content_search_note_and_check_grey_without_localsearch() {
    let mut a = app("contents");
    let at = Page::Search.index();
    let note = |a: &SettingsApp| match &a.sheet.pages[at]
        .item("launcher_contents_note")
        .unwrap()
        .control
    {
        form::Control::Lines(l) => (l.join(" "), true),
        form::Control::GreyedLines(l) => (l.join(" "), false),
        _ => panic!("lines"),
    };
    let check = |a: &SettingsApp| match a.sheet.pages[at]
        .item("launcher_contents")
        .map(|i| &i.control)
    {
        Some(form::Control::Check { enabled, .. }) => *enabled,
        _ => panic!("a check box"),
    };
    assert_eq!(note(&a), (String::new(), true), "blank until read");
    assert!(check(&a), "lit until read");
    let read = |a: &mut SettingsApp, installed| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(installed).unwrap();
        a.contents.job = Some(rx);
        assert!(a.contents_done());
    };
    read(&mut a, false);
    assert_eq!(
        note(&a),
        (
            "Install localsearch to find files by their contents.".to_string(),
            false
        )
    );
    assert!(!check(&a), "greyed without localsearch");
    read(&mut a, true);
    assert_eq!(
        note(&a),
        (
            "The first search starts localsearch, which then indexes your files in the background."
                .to_string(),
            true
        )
    );
    assert!(check(&a));
    assert!(!a.contents_done(), "nothing more to read");
    let mut off = a.want.clone();
    off.launcher_files = false;
    crate::pages::follow(&mut a.sheet, &off);
    assert!(!check(&a), "greyed with file search off");
}

/// The Search page lists the installed providers once read; a tick and Move Down write
/// `[launcher] providers` and `providers_off` on Apply.
#[test]
fn the_program_results_list_ticks_and_orders_the_providers() {
    use crate::providerpage::{DOWN, Installed, LIST, UP};
    let mut a = app("providers");
    a.open_page(Page::Search, None);
    let at = Page::Search.index();
    let list = |a: &SettingsApp| match &a.sheet.pages[at].item(LIST).unwrap().control {
        form::Control::ListBox { list, .. } => (list.items.clone(), list.checks.clone()),
        _ => panic!("a list"),
    };
    a.ensure_providers();
    assert_eq!(list(&a).0, ["Loading..."]);
    let installed = |id: &str, name: &str| Installed {
        provider: reprisefs::searchproviders::Provider {
            desktop_id: id.into(),
            bus_name: format!("org.example.{name}"),
            object_path: "/".into(),
            default_disabled: false,
        },
        name: name.into(),
        icon: None,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(vec![
        installed("f.desktop", "Files"),
        installed("c.desktop", "Calculator"),
    ])
    .unwrap();
    a.providers.job = Some(rx);
    assert!(a.providers_done());
    assert_eq!(
        list(&a),
        (vec!["Calculator".into(), "Files".into()], vec![true, true])
    );
    assert!(!a.dirty(), "listing writes nothing");

    if let Some(form::Control::ListBox { list, .. }) =
        a.sheet.pages[at].item_mut(LIST).map(|i| &mut i.control)
    {
        list.selected = Some(0);
    }
    a.sheet_event(propsheet::Event::Page(at, form::Event::Changed(LIST)));
    assert!(!a.dirty(), "picking a row writes nothing");
    a.sheet_event(propsheet::Event::Page(at, form::Event::Pressed(DOWN)));
    assert_eq!(list(&a).0, ["Files", "Calculator"]);
    if let Some(form::Control::ListBox { list, .. }) =
        a.sheet.pages[at].item_mut(LIST).map(|i| &mut i.control)
    {
        assert_eq!(list.selected, Some(1), "the moved row stays picked");
        list.toggle(1);
    }
    a.sheet_event(propsheet::Event::Page(at, form::Event::Changed(LIST)));
    assert_eq!(list(&a).1, [true, false]);
    let up = &a.sheet.pages[at].item(UP).unwrap().control;
    assert!(matches!(up, form::Control::Button { enabled: true, .. }));
    assert!(a.apply());
    let text = std::fs::read_to_string(&a.path).unwrap();
    assert!(
        text.contains("providers = [\"f.desktop\", \"c.desktop\"]")
            && text.contains("providers_off = [\"c.desktop\"]"),
        "{text}"
    );
}

/// Files' provider starts the file indexer: its row greys with a note while the contents check
/// is off and lights as soon as the check is ticked.
#[test]
fn the_files_provider_greys_until_content_search_is_on() {
    use crate::providerpage::{Installed, LIST, held_note};
    let mut a = app("providers-held");
    a.open_page(Page::Search, None);
    let at = Page::Search.index();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(vec![Installed {
        provider: reprisefs::searchproviders::Provider {
            desktop_id: "org.gnome.Nautilus.desktop".into(),
            bus_name: "org.gnome.Nautilus".into(),
            object_path: "/".into(),
            default_disabled: false,
        },
        name: "Files".into(),
        icon: None,
    }])
    .unwrap();
    a.ensure_providers();
    a.providers.job = Some(rx);
    assert!(a.providers_done());
    let row = |a: &SettingsApp| match &a.sheet.pages[at].item(LIST).unwrap().control {
        form::Control::ListBox { list, .. } => (
            list.checks.clone(),
            list.greyed.clone(),
            list.detail.clone(),
        ),
        _ => panic!("a list"),
    };
    assert_eq!(
        row(&a),
        (vec![true], vec![true], vec![held_note().to_string()]),
        "ticked, greyed and noted"
    );
    if let Some(form::Control::Check { checked, .. }) = a.sheet.pages[at]
        .item_mut("launcher_contents")
        .map(|i| &mut i.control)
    {
        *checked = true;
    }
    a.sheet_event(propsheet::Event::Page(
        at,
        form::Event::Changed("launcher_contents"),
    ));
    assert!(a.want.launcher_contents);
    assert!(a.ensure_providers(), "the list follows the check");
    assert_eq!(row(&a), (vec![true], vec![false], vec![String::new()]));
}

/// The Taskbar page's calendar events note says what to install, greyed with its check box,
/// while evolution-data-server is missing, and is blank otherwise.
#[test]
fn the_calendar_events_row_greys_without_evolution_data_server() {
    let mut a = app("events");
    let at = Page::Taskbar.index();
    let row = |a: &SettingsApp, id: &str| match &a.sheet.pages[at].item(id).unwrap().control {
        form::Control::Label { text, enabled, .. } => (text.clone(), *enabled),
        form::Control::Check { enabled, .. } => (String::new(), *enabled),
        _ => panic!("a label or a check box"),
    };
    assert_eq!(row(&a, "taskbar_events_note"), (String::new(), true));
    let read = |a: &mut SettingsApp, installed| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(installed).unwrap();
        a.events.job = Some(rx);
        assert!(a.events_done());
    };
    read(&mut a, false);
    assert_eq!(
        row(&a, "taskbar_events_note"),
        ("(install evolution-data-server)".to_string(), false)
    );
    assert!(!row(&a, "taskbar_calendar_events").1, "the check box greys");
    read(&mut a, true);
    assert_eq!(row(&a, "taskbar_events_note"), (String::new(), true));
    assert!(row(&a, "taskbar_calendar_events").1);
}

fn reader_read(a: &mut SettingsApp, reader: readerhost::Reader, failed: Option<String>) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(readerhost::Reading { reader, failed }).unwrap();
    a.reader_job = Some(rx);
    a.reader_done();
}

#[test]
fn two_failures_in_one_wake_show_two_boxes_in_turn() {
    let mut a = app("twofailures");
    let (tx, rx) = std::sync::mpsc::channel();
    let failed = Some("Remote Desktop could not be turned on.".into());
    let remote = remotehost::Remote::default();
    tx.send(remotehost::Reading { remote, failed }).unwrap();
    a.remote_job = Some(rx);
    let (tx, rx) = std::sync::mpsc::channel();
    let failed = Some("The screen reader could not be started.".into());
    let reader = readerhost::Reader::default();
    tx.send(readerhost::Reading { reader, failed }).unwrap();
    a.reader_job = Some(rx);
    a.wake();
    let up = |a: &SettingsApp| a.error.shown().map(|b| b.fields.text.clone());
    assert_eq!(
        up(&a).as_deref(),
        Some("Remote Desktop could not be turned on.")
    );
    a.error.close();
    assert_eq!(
        up(&a).as_deref(),
        Some("The screen reader could not be started.")
    );
}

/// The screen reader's boxes show what was read, grey with a note while Orca is missing, and
/// moving one writes nothing to config.toml.
#[test]
fn the_screen_reader_boxes_follow_the_reads_and_grey_without_orca() {
    let mut a = app("reader");
    let at = Page::MagnifierFilters.index();
    let enabled = |a: &SettingsApp, id| match a.sheet.pages[at].item(id).map(|i| &i.control) {
        Some(form::Control::Check { enabled, .. }) => *enabled,
        _ => panic!("{id} is a check box"),
    };
    let missing = |a: &SettingsApp| match &a.sheet.pages[at].item("reader_missing").unwrap().control
    {
        form::Control::Lines(l) => l.clone(),
        _ => panic!("lines"),
    };

    reader_read(&mut a, readerhost::Reader::default(), None);
    assert!(!enabled(&a, magnifierpage::READER));
    assert!(!enabled(&a, magnifierpage::READER_LOGON));
    assert_eq!(missing(&a), ["Install Orca to use the screen reader."]);

    let running = readerhost::Reader {
        installed: true,
        running: true,
    };
    reader_read(&mut a, running, None);
    let page = &a.sheet.pages[at];
    assert_eq!(page.checked(magnifierpage::READER), Some(true));
    assert!(enabled(&a, magnifierpage::READER));
    assert!(missing(&a).is_empty());

    a.sheet.pages[at].set_check(magnifierpage::READER, false);
    a.sheet_event(propsheet::Event::Page(
        at,
        form::Event::Changed(magnifierpage::READER),
    ));
    assert!(!a.reader.running, "shown off until the next read");
    assert_eq!(a.want, a.loaded, "the switch is no change for Apply");

    a.sheet.pages[at].set_check(magnifierpage::READER, true);
    a.sheet_event(propsheet::Event::Page(
        at,
        form::Event::Changed(magnifierpage::READER),
    ));
    assert_eq!(
        a.sheet.pages[at].checked(magnifierpage::READER),
        Some(false),
        "a click while the switch runs is undone"
    );

    reader_read(
        &mut a,
        running,
        Some("The screen reader could not be started.".into()),
    );
    assert!(a.error.is_shown(), "a failed switch says why");
    assert_eq!(a.sheet.pages[at].checked(magnifierpage::READER), Some(true));
    a.sheet.pages[at].set_check(magnifierpage::READER, false);
    a.sheet_event(propsheet::Event::Page(
        at,
        form::Event::Changed(magnifierpage::READER),
    ));
    assert!(!a.reader.running, "the next click switches again");
}

fn airplane_read(a: &mut SettingsApp, state: Option<Mode>, failed: Option<String>) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::airplanehost::Reading { state, failed })
        .unwrap();
    a.airplane.job = Some(rx);
    assert!(a.airplane_done());
}

fn lit(a: &SettingsApp) -> bool {
    match &a.sheet.pages[Page::Network.index()]
        .item(pages::AIRPLANE)
        .unwrap()
        .control
    {
        form::Control::Check { enabled, .. } => *enabled,
        c => panic!("airplane mode is a check box, not {c:?}"),
    }
}

#[test]
fn the_airplane_check_box_acts_at_once_and_follows_the_radios() {
    let mut a = app("airplane");
    let p = Page::Network.index();
    airplane_read(&mut a, Some(Mode::Off), None);
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(false));
    assert!(lit(&a));

    a.sheet.pages[p].set_check(pages::AIRPLANE, true);
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Changed(pages::AIRPLANE),
    ));
    assert!(a.airplane.acting);
    assert!(!lit(&a), "greyed while the radios turn");
    airplane_read(&mut a, Some(Mode::On), None);
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
    assert!(lit(&a));
    assert_eq!(a.want, a.loaded, "the check box is no change for Apply");

    a.airplane.acting = true;
    airplane_read(
        &mut a,
        Some(Mode::On),
        Some("Airplane mode could not be turned off.".into()),
    );
    assert!(a.error.is_shown(), "a failed change says why");
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
}

#[test]
fn under_a_hardware_kill_the_airplane_check_box_greys_and_a_click_sends_nothing() {
    let mut a = app("airplane");
    let p = Page::Network.index();
    airplane_read(&mut a, Some(Mode::Killed), None);
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
    assert!(
        !lit(&a),
        "greyed while a hardware switch holds the radios off"
    );

    a.sheet.pages[p].set_check(pages::AIRPLANE, false);
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Changed(pages::AIRPLANE),
    ));
    assert!(!a.airplane.acting, "no change goes out");
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
    assert!(!lit(&a));
}

fn region_ready(a: &mut SettingsApp, user: reprisefs::region::Choice, system_lang: &str) {
    let locales = vec![
        (
            "de_DE.UTF-8".to_string(),
            "Deutsch (Deutschland)".to_string(),
        ),
        ("en_US.UTF-8".into(), "English (United States)".into()),
    ];
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::regionhost::Reading {
        locales,
        user,
        system: crate::regionpage::System::Read {
            locale: vec![("LANG".into(), system_lang.into())],
            may_set: true,
        },
    })
    .unwrap();
    a.region_read = Some(rx);
    assert!(a.region_done());
}

#[test]
fn localed_unread_is_read_again_when_the_page_opens_again() {
    use crate::regionpage::System;
    let mut a = app("region-reread");
    a.sheet.tabs.open = Page::RegionLanguage.index();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::regionhost::Reading {
        locales: Vec::new(),
        user: reprisefs::region::Choice {
            language: "en_US.UTF-8".into(),
            formats: None,
        },
        system: System::Unread,
    })
    .unwrap();
    a.region_read = Some(rx);
    a.ensure_region();
    assert!(a.region_done());
    a.ensure_region();
    assert!(
        a.region.as_ref().unwrap().reread.is_none(),
        "not while it stays open"
    );
    a.sheet.tabs.open = Page::Thunderbolt.index();
    a.ensure_region();
    a.sheet.tabs.open = Page::RegionLanguage.index();
    a.ensure_region();
    assert!(a.region.as_ref().unwrap().reread.is_some(), "opened again");
    let (tx, rx) = std::sync::mpsc::channel();
    let read = System::Read {
        locale: vec![("LANG".into(), "de_DE.UTF-8".into())],
        may_set: true,
    };
    tx.send(read.clone()).unwrap();
    a.region.as_mut().unwrap().reread = Some(rx);
    assert!(a.region_done());
    assert_eq!(a.region.as_ref().unwrap().system, read);
}

#[test]
fn ok_after_a_language_change_closes_once_accountsservice_has_it() {
    use crate::regionpage::LANGUAGE;
    let mut a = app("region-ok");
    a.region_conf = tmp("region-ok-conf").join("locale.conf");
    let english = reprisefs::region::Choice {
        language: "en_US.UTF-8".into(),
        formats: None,
    };
    region_ready(&mut a, english, "en_US.UTF-8");
    let p = Page::RegionLanguage.index();
    a.sheet.pages[p].set_combo(LANGUAGE, 0);
    a.sheet_event(propsheet::Event::Page(p, form::Event::Changed(LANGUAGE)));
    a.sheet_event(propsheet::Event::Ok);
    let mut ctx = test_ctx();
    assert!(
        !a.event(MAIN, Event::Blink, &mut ctx).quit,
        "OK waits for AccountsService"
    );
    assert!(a.wake().quit, "the call ended: the sheet closes");
}

#[test]
fn a_close_while_localed_sets_the_language_hides_the_sheet_and_quits_once_it_is_told() {
    const REFUSED: &str = "You are not allowed to make this change.";
    let english = reprisefs::region::Choice {
        language: "en_US.UTF-8".into(),
        formats: None,
    };
    for set in [
        Ok(vec![("LANG".into(), "de_DE.UTF-8".into())]),
        Err(Some(REFUSED.into())),
    ] {
        let refused = set.is_err();
        let mut a = app("region-close");
        region_ready(&mut a, english.clone(), "en_US.UTF-8");
        let (tx, rx) = std::sync::mpsc::channel();
        a.region_set = Some(rx);
        let mut ctx = test_ctx();
        assert!(
            !a.event(MAIN, Event::CloseRequest, &mut ctx).quit,
            "refused {refused}: the close waits for localed"
        );
        assert!(a.hidden(), "refused {refused}: the sheet is gone meanwhile");
        tx.send(set).unwrap();
        let r = a.wake();
        if !refused {
            assert!(r.quit, "the language is set: Settings ends");
            continue;
        }
        assert!(!r.quit, "the refusal is read first");
        let text = a.error.shown().map(|b| b.fields.text.clone());
        assert_eq!(text.as_deref(), Some(REFUSED));
        let win = a.error.win().unwrap();
        assert!(
            a.event(win, Event::CloseRequest, &mut ctx).quit,
            "the box closed: Settings ends"
        );
    }
}

#[test]
fn a_close_while_boltd_authorizes_waits_for_its_answer() {
    let mut a = app("tb-close");
    let (tx, rx) = std::sync::mpsc::channel();
    a.thunderbolt.job = Some(rx);
    let mut ctx = test_ctx();
    assert!(!a.event(MAIN, Event::CloseRequest, &mut ctx).quit);
    tx.send(crate::thunderbolthost::Reading {
        found: None,
        failed: None,
    })
    .unwrap();
    assert!(a.wake().quit, "boltd answered: Settings ends");
}

fn region_set(a: &mut SettingsApp, set: crate::regionhost::Set) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(set).unwrap();
    a.region_set = Some(rx);
    assert!(a.region_done());
}

#[test]
fn region_and_language_saves_locale_conf_on_apply_and_sets_the_system_at_once() {
    use crate::regionpage::{LANGUAGE, SYSTEM};
    let mut a = app("region");
    a.region_conf = tmp("region-conf").join("locale.conf");
    let p = Page::RegionLanguage.index();
    let english = reprisefs::region::Choice {
        language: "en_US.UTF-8".into(),
        formats: None,
    };
    region_ready(&mut a, english.clone(), "en_US.UTF-8");
    assert_eq!(a.sheet.pages[p].combo_selected(LANGUAGE), Some(1));
    assert!(!a.dirty(), "a read is no change");
    let lit = |a: &SettingsApp| match &a.sheet.pages[p].item(SYSTEM).unwrap().control {
        form::Control::Button { enabled, .. } => *enabled,
        c => panic!("Apply to the System is a button, not {c:?}"),
    };
    assert!(!lit(&a), "the system already has the page's choice");

    a.sheet.pages[p].set_combo(LANGUAGE, 0);
    a.sheet_event(propsheet::Event::Page(p, form::Event::Changed(LANGUAGE)));
    assert!(a.dirty());
    assert!(lit(&a));
    a.revert();
    assert_eq!(a.sheet.pages[p].combo_selected(LANGUAGE), Some(1));
    assert!(!a.dirty());

    a.sheet.pages[p].set_combo(LANGUAGE, 0);
    a.sheet_event(propsheet::Event::Page(p, form::Event::Changed(LANGUAGE)));
    assert!(a.apply());
    assert_eq!(
        std::fs::read_to_string(&a.region_conf).unwrap(),
        "LANG=de_DE.UTF-8\n"
    );
    assert!(!a.dirty());

    a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(SYSTEM)));
    assert!(a.region.as_ref().unwrap().setting);
    assert!(!lit(&a), "greyed while localed works");
    region_set(&mut a, Err(None));
    assert!(!a.error.is_shown(), "a dismissed prompt says nothing");
    assert!(lit(&a));
    a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(SYSTEM)));
    region_set(&mut a, Ok(vec![("LANG".into(), "de_DE.UTF-8".into())]));
    assert!(!a.error.is_shown());
    assert!(!lit(&a), "the system now has it");
    a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(SYSTEM)));
    assert!(
        !a.region.as_ref().unwrap().setting,
        "nothing to set when the system has it"
    );

    a.region.as_mut().unwrap().system = crate::regionpage::System::Read {
        locale: vec![("LANG".into(), "en_US.UTF-8".into())],
        may_set: true,
    };
    a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(SYSTEM)));
    region_set(
        &mut a,
        Err(Some("You are not allowed to make this change.".into())),
    );
    assert!(a.error.is_shown(), "a refusal says why");
}

fn thunderbolt_read(a: &mut SettingsApp, found: Option<Found>, failed: Option<String>) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::thunderbolthost::Reading { found, failed })
        .unwrap();
    a.thunderbolt.job = Some(rx);
    assert!(a.thunderbolt_done());
}

fn tb_lit(a: &SettingsApp, id: &str) -> bool {
    match &a.sheet.pages[Page::Thunderbolt.index()]
        .item(id)
        .unwrap()
        .control
    {
        form::Control::Button { enabled, .. } => *enabled,
        c => panic!("{id} is a button, not {c:?}"),
    }
}

#[test]
fn each_change_boltd_signals_shows_the_newest() {
    let mut a = app("thunderbolt");
    let (tx, rx) = std::sync::mpsc::channel();
    a.thunderbolt.follow = Some(rx);
    assert!(!a.thunderbolt_done(), "nothing signalled yet");
    let found = |domains| Found {
        domains,
        devices: Vec::new(),
    };
    tx.send(None).unwrap();
    tx.send(Some(found(1))).unwrap();
    assert!(a.thunderbolt_done());
    assert_eq!(a.thunderbolt.found, Some(found(1)));
    tx.send(Some(found(1))).unwrap();
    assert!(!a.thunderbolt_done(), "the same list again changes nothing");
    drop(tx);
    a.thunderbolt_done();
    assert!(a.thunderbolt.follow.is_none());
}

fn label(f: &form::Form, id: &str) -> String {
    match &f.item(id).unwrap().control {
        form::Control::Label { text, .. } => text.clone(),
        c => panic!("{id} is a label, not {c:?}"),
    }
}

#[test]
fn a_first_read_without_boltd_or_a_radio_reaches_the_page() {
    let mut a = app("no-bolt");
    let (tx, rx) = std::sync::mpsc::channel();
    a.thunderbolt.follow = Some(rx);
    tx.send(None).unwrap();
    assert!(
        a.thunderbolt_done(),
        "the first read shows, whatever it says"
    );
    let page = &a.sheet.pages[Page::Thunderbolt.index()];
    assert_eq!(
        label(page, "thunderbolt_status"),
        "The bolt service isn't available, so no device can be listed."
    );
    match &page.item(pages::TB_LIST).unwrap().control {
        form::Control::ListBox { list, .. } => assert!(list.items.is_empty(), "no Loading... row"),
        c => panic!("the devices are a list box, not {c:?}"),
    }
    tx.send(None).unwrap();
    assert!(!a.thunderbolt_done(), "the same read again changes nothing");

    airplane_read(&mut a, None, None);
    let page = &a.sheet.pages[Page::Network.index()];
    assert_eq!(
        label(page, "airplane_status"),
        "This computer has no radio to turn off."
    );
    assert!(!lit(&a));
    assert!(
        matches!(
            page.item("airplane_note").unwrap().control,
            form::Control::Label { enabled: false, .. }
        ),
        "the note greys"
    );
}

#[test]
fn a_follow_that_ended_starts_again_only_when_the_page_opens_again() {
    let mut a = app("thunderbolt");
    a.sheet.tabs.open = Page::Thunderbolt.index();
    a.ensure_thunderbolt();
    assert_eq!(a.thunderbolt.follows, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    a.thunderbolt.follow = Some(rx);
    tx.send(None).unwrap();
    drop(tx);
    a.thunderbolt_done();
    assert!(a.thunderbolt.follow.is_none(), "no system bus");
    a.ensure_thunderbolt();
    a.ensure_thunderbolt();
    assert_eq!(a.thunderbolt.follows, 1, "not on each event while open");
    a.sheet.tabs.open = Page::Display.index();
    a.ensure_thunderbolt();
    a.sheet.tabs.open = Page::Thunderbolt.index();
    a.ensure_thunderbolt();
    assert_eq!(a.thunderbolt.follows, 2, "opened again");
}

#[test]
fn authorize_acts_at_once_on_the_picked_device_and_says_why_it_failed() {
    let mut a = app("thunderbolt");
    let p = Page::Thunderbolt.index();
    let dock = Device {
        path: "/org/freedesktop/bolt/devices/dock".into(),
        uid: "dock".into(),
        name: "Dock".into(),
        vendor: "Acme".into(),
        status: Status::Connected,
        stored: false,
        auto: false,
    };
    let found = Found {
        domains: 1,
        devices: vec![dock],
    };
    thunderbolt_read(&mut a, Some(found.clone()), None);
    let _ = a.thunderbolt_pressed(pages::TB_AUTHORIZE);
    assert!(!a.thunderbolt.acting, "nothing picked, nothing done");

    if let Some(form::Control::ListBox { list, .. }) = a.sheet.pages[p]
        .item_mut(pages::TB_LIST)
        .map(|i| &mut i.control)
    {
        list.select(0, 12);
    }
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Changed(pages::TB_LIST),
    ));
    assert!(tb_lit(&a, pages::TB_AUTHORIZE));
    assert!(!tb_lit(&a, pages::TB_FORGET), "not enrolled");

    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Pressed(pages::TB_AUTHORIZE),
    ));
    assert!(a.thunderbolt.acting);
    assert!(
        !tb_lit(&a, pages::TB_AUTHORIZE),
        "greyed while boltd answers"
    );
    thunderbolt_read(
        &mut a,
        Some(found),
        Some("Acme Dock could not be authorized.".into()),
    );
    assert!(a.error.is_shown(), "a failed change says why");
    assert!(tb_lit(&a, pages::TB_AUTHORIZE));
    assert_eq!(a.want, a.loaded, "the buttons are no change for Apply");
}

fn capture_status(a: &SettingsApp) -> String {
    match &a.sheet.pages[Page::ScreenCapture.index()]
        .item(capturepage::STATUS)
        .unwrap()
        .control
    {
        form::Control::Label { text, .. } => text.clone(),
        c => panic!("the status is a label, not {c:?}"),
    }
}

fn codec_rows_greyed(a: &SettingsApp) -> Vec<bool> {
    match &a.sheet.pages[Page::ScreenCapture.index()]
        .item(capturepage::CODEC)
        .unwrap()
        .control
    {
        form::Control::Combo(c) => c.items.iter().map(|i| i.greyed).collect(),
        c => panic!("the codec is a combo, not {c:?}"),
    }
}

#[test]
fn the_capture_status_reads_checking_then_the_codec_missing() {
    use repriseconf::recording::{AudioCodec, Codec, Probed};
    let path = tmp("capture").join("config.toml");
    std::fs::write(&path, "[recording]\ncodec = \"av1\"\n").unwrap();
    let mut a = SettingsApp::open(path, "us".into());
    a.ensure_capture();
    assert_eq!(capture_status(&a), "", "no probe before the page opens");
    a.sheet.tabs.open = Page::ScreenCapture.index();
    a.ensure_capture();
    assert_eq!(capture_status(&a), "Checking encoders...");
    assert_eq!(codec_rows_greyed(&a), [false; 3], "nothing known yet");

    let (tx, rx) = std::sync::mpsc::channel();
    let h264 = Probed {
        video: vec![(Codec::H264, repriseconf::remote::EncoderChoice::Software)],
        audio: vec![AudioCodec::Aac],
    };
    tx.send(capturepage::ProbeState::Done(h264.clone()))
        .unwrap();
    a.capture_job = Some(rx);
    assert!(a.capture_done());
    assert_eq!(capture_status(&a), "No AV1 encoder works here.");
    assert_eq!(codec_rows_greyed(&a), [false, true, true]);

    a.want.screenshot_format = "jxl".into();
    pages::follow(&mut a.sheet, &a.want);
    assert_eq!(
        codec_rows_greyed(&a),
        [false; 3],
        "the page was built again"
    );
    a.ensure_capture();
    assert_eq!(
        codec_rows_greyed(&a),
        [false, true, true],
        "and greyed again after the event"
    );
    assert_eq!(
        a.capture_probe,
        capturepage::ProbeState::Done(h264),
        "one probe a run"
    );
}

#[test]
fn paired_devices_and_notifying_programs_show_once_their_read_answers() {
    let mut a = app("page-reads");
    let mut ctx = test_ctx();
    a.sheet.tabs.open = Page::ScreenSaverLock.index();
    a.event(MAIN, Event::Blink, &mut ctx);
    let (tx, rx) = std::sync::mpsc::channel();
    a.away_reading = Some(rx);
    let phone = reprisebus::bluez::Device {
        path: "/org/bluez/hci0/dev_00_11".into(),
        address: "00:11:22:33:44:55".into(),
        name: "Phone".into(),
        paired: true,
        connected: false,
    };
    tx.send(vec![phone]).unwrap();
    assert!(a.event(MAIN, Event::Wake, &mut ctx).redraw);
    let away = &a.sheet.pages[Page::ScreenSaverLock.index()];
    let rows = match &away.item(saverpage::AWAY).unwrap().control {
        form::Control::Combo(c) => c.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
        c => panic!("the away device is a combo, not {c:?}"),
    };
    assert_eq!(rows[1..], ["Phone"]);

    a.sheet.tabs.open = Page::Notifications.index();
    a.event(MAIN, Event::Blink, &mut ctx);
    let (tx, rx) = std::sync::mpsc::channel();
    a.notify_reading = Some(rx);
    tx.send(vec![("org.example.Mail".into(), "Mail".into())])
        .unwrap();
    assert!(a.event(MAIN, Event::Wake, &mut ctx).redraw);
    let names: Vec<&str> = a.notify_programs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mail"]);
}

#[test]
fn a_rescan_keeps_an_unapplied_startup_check_on_its_entry() {
    let mut a = app("startup-pending");
    let parse = |id, text| reprisefs::autostart::parse(id, text, true).unwrap();
    let one = parse("one", "[Desktop Entry]\nName=One\nExec=one\n");
    let two = parse("two", "[Desktop Entry]\nName=Two\nExec=two\n");
    a.entries = vec![one.clone(), two.clone()];
    a.applied_checks = vec![true, true];
    pages::fill_list(&mut a.list, &a.entries, &[true, false]);
    let pending = a.pending_checks();
    assert_eq!(pending, [("two".to_string(), false)]);
    let new = parse("new", "[Desktop Entry]\nName=New\nExec=new\n");
    a.entries = vec![new, one, two];
    a.applied_checks = vec![true, true, true];
    a.show_startup(&pending);
    assert_eq!(a.list.checks, Some(vec![true, true, false]));
    assert!(a.dirty(), "Apply stays lit for it");
}

#[test]
fn a_startup_remove_that_fails_says_why() {
    let mut a = app("startup-remove");
    let system = "[Desktop Entry]\nName=Volume\nExec=vol\n";
    a.entries = vec![reprisefs::autostart::parse("vol", system, false).unwrap()];
    a.applied_checks = vec![true];
    pages::fill_list(&mut a.list, &a.entries, &[true]);
    a.list.selection.focus = Some(0);
    a.remove_selected();
    let text = a.error.shown().map(|b| b.fields.text.clone());
    assert_eq!(
        text.as_deref(),
        Some("Cannot remove 'Volume': a system entry has no user file to remove.")
    );
    assert_eq!(a.entries.len(), 1, "the row stays");
}

#[test]
fn an_empty_photos_folder_is_listed_once_while_the_page_stays_open() {
    let mut a = app("saver-photos");
    let ctx = test_ctx();
    let folder = tmp("saver-photos-empty");
    a.want.saver = "photos".into();
    a.want.blank_after = 10;
    a.want.saver_folder = folder.display().to_string();
    a.sheet.tabs.open = Page::ScreenSaverLock.index();
    let listed = |a: &SettingsApp| {
        matches!(
            &a.saver_photos,
            Some((_, crate::saverhost::Photos::Listed(p))) if p.is_empty()
        )
    };
    let started = std::time::Instant::now();
    while !listed(&a) {
        assert!(started.elapsed().as_secs() < 5, "the folder is listed");
        a.saver_tick(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    a.saver_tick(&ctx);
    assert!(listed(&a), "an empty listing is kept, not listed again");
}

fn press(a: &mut SettingsApp, ctx: &mut Ctx, (x, y): (i32, i32)) {
    let mods = Mods::default();
    let button = BTN_LEFT;
    let down = PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods,
    };
    a.event(MAIN, Event::Pointer(down), ctx);
    let up = PointerInput::Release { x, y, button, mods };
    a.event(MAIN, Event::Pointer(up), ctx);
}

/// What every check box on the open page holds.
fn checks(a: &SettingsApp) -> Vec<(&'static str, bool)> {
    let page = a.sheet.open();
    page.items
        .iter()
        .filter_map(|i| Some((i.id, page.checked(i.id)?)))
        .collect()
}

/// Drops `id`'s list on the open page by a press on its arrow, and shows it as the host does: on
/// a popup placed where it asks. The list's rect and row height.
fn drop_by_pointer(a: &mut SettingsApp, ctx: &mut Ctx, id: &str) -> (Rect, i32) {
    let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    a.draw(MAIN, &mut c, ctx);
    let toggle = |a: &mut SettingsApp| match a.sheet.open_mut().item_mut(id).map(|i| &mut i.control)
    {
        Some(form::Control::Combo(c)) => c.toggle(),
        _ => panic!("{id} is a combo box"),
    };
    toggle(a);
    let field = match a.sheet.spots().first().map(|s| s.anchor) {
        Some(Anchor::Below(field)) => field,
        other => panic!("{id}'s list hangs under its field, not {other:?}"),
    };
    toggle(a);
    press(a, ctx, (field.right() - 4, field.y + field.height / 2));
    assert!(
        a.sheet.open().popup_open(),
        "a press on {id}'s arrow drops its list"
    );
    let mut hosted = a.popup(MAIN, ctx).expect("the list shows on a popup");
    hosted.host();
    let spot = hosted.spots()[0];
    hosted.place(0, (spot.rect.x, spot.rect.y));
    let list = hosted.spots()[0].rect;
    drop(hosted);
    a.draw(MAIN, &mut c, ctx);
    let rows = match a.sheet.open().item(id).map(|i| &i.control) {
        Some(form::Control::Combo(c)) => c.rows.min(c.items.len()) as i32,
        _ => unreachable!(),
    };
    (list, (list.height - 2) / rows)
}

/// Picks a row of the open page's combo `id` by the pointer, `slot` rows down its dropped list or
/// else the first live row in view that isn't picked: the row lights under the pointer, the click
/// picks it and closes the list, and no check box under the list changes.
fn pick_by_pointer(a: &mut SettingsApp, ctx: &mut Ctx, id: &str, slot: Option<usize>) {
    let (list, row_h) = drop_by_pointer(a, ctx, id);
    let Some(form::Control::Combo(c)) = a.sheet.open().item(id).map(|i| &i.control) else {
        unreachable!()
    };
    let top = c.top;
    let slot = slot.unwrap_or_else(|| {
        (0..c.rows.min(c.items.len()))
            .find(|&k| top + k != c.selected && !c.items[top + k].greyed)
            .expect("a live row to pick")
    });
    let row = top + slot;
    let combo = |a: &SettingsApp| match a.sheet.open().item(id).map(|i| &i.control) {
        Some(form::Control::Combo(c)) => c.open,
        _ => None,
    };
    let before = checks(a);
    let at = (list.x + 8, list.y + 1 + slot as i32 * row_h + row_h / 2);
    a.event(MAIN, Event::Pointer(PointerInput::Leave), ctx);
    let over = PointerInput::Motion { x: at.0, y: at.1 };
    a.event(MAIN, Event::Pointer(over), ctx);
    assert_eq!(
        combo(a),
        Some(row),
        "{id}'s row {row} lights under the pointer"
    );
    press(a, ctx, at);
    assert!(!a.sheet.open().popup_open(), "the click closes {id}'s list");
    assert_eq!(
        a.sheet.open().combo_selected(id),
        Some(row),
        "{id} picks row {row}"
    );
    assert_eq!(checks(a), before, "no check box under {id}'s list changed");
}

fn maximized(i: Interface, text_size: u32) -> Ctx {
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(i);
    ctx.text_size = text_size;
    ctx.size = (1920, 1027);
    ctx.maximized = true;
    ctx
}

#[test]
fn the_text_size_list_lights_and_picks_the_row_under_the_pointer() {
    for i in [Interface::Classic, Interface::Blue] {
        for size in [100, 200] {
            let mut a = app("text-size-pointer");
            let mut ctx = maximized(i, size);
            a.sheet.tabs.open = Page::TextKeys.index();
            a.event(MAIN, Event::Resized, &mut ctx);
            pick_by_pointer(&mut a, &mut ctx, "text_size", Some(2));
        }
    }
}

#[test]
fn a_press_beside_the_text_size_list_only_closes_it() {
    let mut a = app("text-size-beside");
    let mut ctx = maximized(Interface::Blue, 100);
    a.sheet.tabs.open = Page::TextKeys.index();
    a.event(MAIN, Event::Resized, &mut ctx);
    let (list, _) = drop_by_pointer(&mut a, &mut ctx, "text_size");
    let px = a.sheet.open().units().px(ctx.theme.fonts.size_px);
    let sticky = a
        .sheet
        .open()
        .items
        .iter()
        .position(|i| i.id == "sticky_keys");
    let beside = (0..list.x)
        .rev()
        .flat_map(|x| (list.y..list.bottom()).map(move |y| (x, y)))
        .find(|&(x, y)| {
            matches!(a.sheet.hit(x, y, &ctx.text, px),
                Some(propsheet::Hit::Control(h)) if Some(h.item) == sticky)
        })
        .expect("Use StickyKeys lies beside the list");
    press(&mut a, &mut ctx, beside);
    assert!(!a.sheet.open().popup_open(), "the press closes the list");
    assert_eq!(a.sheet.open().checked("sticky_keys"), Some(false));
    press(&mut a, &mut ctx, beside);
    assert_eq!(
        a.sheet.open().checked("sticky_keys"),
        Some(true),
        "with the list closed the press ticks it"
    );
}

#[test]
fn every_page_combo_picks_the_row_under_the_pointer() {
    let mut a = app("every-combo-pointer");
    let mut ctx = maximized(Interface::Blue, 100);
    a.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    let mut picked = 0;
    for page in Page::all() {
        a.sheet.tabs.open = page.index();
        a.event(MAIN, Event::Resized, &mut ctx);
        let ids: Vec<&'static str> = a.sheet.open().items.iter().map(|i| i.id).collect();
        for id in ids {
            // A pick can build the page again (Interface), so each combo is looked up afresh.
            let live = match a.sheet.open().item(id).map(|i| &i.control) {
                Some(form::Control::Combo(c)) => c.enabled && c.items.len() > 1,
                _ => false,
            };
            if !live {
                continue;
            }
            pick_by_pointer(&mut a, &mut ctx, id, None);
            picked += 1;
        }
    }
    assert!(picked > 10, "only {picked} combos tried");
}

#[test]
fn a_window_rule_added_and_removed_on_its_page_is_written_around_the_files_comments() {
    use crate::{ruledlg, rulespage};
    let dir = tmp("window-rules");
    let path = dir.join("config.toml");
    let file = "terminal = \"foot\"  # mine\n\n# shells\n[[window_rules]]\napp_id = \"foot\"\nsize = [800, 600]  # big\n";
    std::fs::write(&path, file).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let p = Page::WindowRules.index();
    let press = |a: &mut SettingsApp, id| {
        a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(id)));
    };
    press(&mut a, rulespage::ADD);
    assert!(a.children().iter().any(|c| c.id == RULE_WIN));
    let enter = || Event::Key(key_input(keysym::RETURN, Mods::default(), None));
    a.event(RULE_WIN, enter(), &mut ctx);
    assert!(
        a.rule_dlg.is_some(),
        "Enter with no app id leaves the dialog up"
    );
    for ch in "kitty".chars() {
        let text = ch.to_string();
        let key = key_input(ch as u32, Mods::default(), Some(&text));
        a.event(RULE_WIN, Event::Key(key), &mut ctx);
    }
    a.rule_dlg
        .as_mut()
        .unwrap()
        .0
        .form
        .set_check(ruledlg::ON_TOP, true);
    a.event(RULE_WIN, enter(), &mut ctx);
    assert!(a.rule_dlg.is_none(), "Enter is OK");
    assert_eq!(
        rulespage::picked(&a.sheet.pages[p]),
        Some(1),
        "the new rule is picked"
    );
    assert!(a.apply());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        format!("{file}\n[[window_rules]]\napp_id = \"kitty\"\nalways_on_top = true\n")
    );

    press(&mut a, rulespage::REMOVE);
    assert!(a.apply());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    press(&mut a, rulespage::REMOVE);
    assert!(a.want.window_rules.is_empty());
    assert!(a.apply());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "terminal = \"foot\"  # mine\n\n# shells\n",
        "the comment above the list stays"
    );
}

#[test]
fn a_rule_always_open_here_adds_or_removes_under_a_pending_edit_stays_so_after_apply() {
    use crate::{ruledlg, rulespage};
    let dir = tmp("window-rules-merge");
    let path = dir.join("config.toml");
    let file =
        "[[window_rules]]\napp_id = \"foot\"\n\n[[window_rules]]\napp_id = \"reprise.notepad\"\n";
    std::fs::write(&path, file).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let p = Page::WindowRules.index();
    let mut edit_first = |a: &mut SettingsApp, check| {
        let rules = a.want.window_rules.clone();
        rulespage::fill(&mut a.sheet.pages[p], &rules, Some(0));
        a.sheet_event(propsheet::Event::Page(
            p,
            form::Event::Pressed(rulespage::EDIT),
        ));
        let d = &mut a.rule_dlg.as_mut().expect("Edit opens the dialog").0;
        d.form.set_check(check, true);
        let enter = Event::Key(key_input(keysym::RETURN, Mods::default(), None));
        a.event(RULE_WIN, enter, &mut ctx);
        assert!(a.rule_dlg.is_none(), "Enter is OK");
    };
    let rules = |path: &std::path::Path| {
        let doc = std::fs::read_to_string(path).unwrap().parse().unwrap();
        (crate::windowrules::read(&doc).into_iter())
            .map(|r| (r.app_id, r.desktop, r.always_on_top))
            .collect::<Vec<_>>()
    };
    let foot = |desktop, on_top| ("foot".to_string(), desktop, on_top);
    let notepad = || ("reprise.notepad".to_string(), None, None);

    edit_first(&mut a, ruledlg::DESKTOP);
    let menu = "\n[[window_rules]]\napp_id = \"firefox\"\nfrom_menu = true\n";
    std::fs::write(&path, format!("{file}{menu}")).unwrap();
    assert!(a.config_changed());
    assert!(a.apply());
    assert_eq!(
        rules(&path),
        [
            foot(Some(1), None),
            notepad(),
            ("firefox".into(), None, None)
        ]
    );

    edit_first(&mut a, ruledlg::ON_TOP);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace(menu, "")).unwrap();
    assert!(a.config_changed());
    assert!(a.apply());
    assert_eq!(
        rules(&path),
        [foot(Some(1), Some(true)), notepad()],
        "the rule unticked meanwhile stays gone"
    );
}

#[test]
fn the_rule_dialog_writes_the_rule_it_opened_on_after_the_list_moves_under_it() {
    use crate::{ruledlg, rulespage};
    let dir = tmp("window-rules-moved");
    let path = dir.join("config.toml");
    let menu = "[[window_rules]]\napp_id = \"firefox\"\nfrom_menu = true\n\n";
    let rest =
        "[[window_rules]]\napp_id = \"foot\"\n\n[[window_rules]]\napp_id = \"reprise.notepad\"\n";
    std::fs::write(&path, format!("{menu}{rest}")).unwrap();
    let mut a = SettingsApp::open(path.clone(), "us".into());
    let mut ctx = test_ctx();
    let p = Page::WindowRules.index();
    let open_on = |a: &mut SettingsApp, row, edit: bool| {
        let rules = a.want.window_rules.clone();
        rulespage::fill(&mut a.sheet.pages[p], &rules, Some(row));
        a.sheet_event(propsheet::Event::Page(
            p,
            form::Event::Pressed(rulespage::EDIT),
        ));
        let d = &mut a.rule_dlg.as_mut().expect("Edit opens the dialog").0;
        if edit {
            d.form.set_check(ruledlg::STICKY, true);
        }
    };
    let mut ok = |a: &mut SettingsApp| {
        let enter = Event::Key(key_input(keysym::RETURN, Mods::default(), None));
        a.event(RULE_WIN, enter, &mut ctx);
        assert!(a.rule_dlg.is_none(), "Enter is OK");
    };
    let rules = |a: &SettingsApp| {
        (a.want.window_rules.iter())
            .map(|r| (r.app_id.clone(), r.sticky))
            .collect::<Vec<_>>()
    };
    let after = [
        ("foot".to_string(), None),
        ("reprise.notepad".into(), Some(true)),
    ];

    open_on(&mut a, 2, true);
    std::fs::write(&path, rest).unwrap();
    assert!(a.config_changed());
    ok(&mut a);
    assert_eq!(rules(&a), after, "the edit lands on its rule, a row up now");

    assert!(a.apply());
    open_on(&mut a, 1, false);
    std::fs::write(&path, "[[window_rules]]\napp_id = \"foot\"\n").unwrap();
    assert!(a.config_changed());
    ok(&mut a);
    assert_eq!(
        rules(&a),
        [("foot".to_string(), None)],
        "OK on an unedited rule removed meanwhile does not bring it back"
    );
}

/// A left press and release at (`x`, `y`) at event time `time`, as the window reads them.
fn click_at(a: &mut SettingsApp, ctx: &mut Ctx, (x, y): (i32, i32), time: u32) -> Reply {
    let mods = Mods::default();
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods,
    };
    let reply = a.event(MAIN, Event::Pointer(press), ctx);
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods,
    };
    a.event(MAIN, Event::Pointer(release), ctx);
    reply
}

/// A point in control `id` of the open page whose hit `part` accepts.
fn point_in(a: &SettingsApp, ctx: &Ctx, id: &str, part: impl Fn(form::Part) -> bool) -> (i32, i32) {
    let px = a.sheet.open().units().px(ctx.theme.fonts.size_px);
    let r = a.sheet.open().rect(id).expect("the control");
    (r.x..r.right())
        .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
        .find(|&(x, y)| {
            matches!(a.sheet.hit(x, y, &ctx.text, px),
                Some(propsheet::Hit::Control(h)) if part(h.part))
        })
        .expect("the part shows")
}

#[test]
fn a_double_click_selects_a_number_fields_digits_and_a_list_rows_leaves_the_sheet_open() {
    use reprisekit::controls::updown;
    let mut a = app("double-click-digits");
    let mut ctx = test_ctx();
    ctx.size = (1920, 1080);
    a.open_page(Page::VirtualDesktops, None);
    a.event(MAIN, Event::Resized, &mut ctx);
    let digits = point_in(&a, &ctx, "desktops", |p| {
        matches!(p, form::Part::UpDown(updown::Hit::Edit(_)))
    });
    click_at(&mut a, &mut ctx, digits, 1000);
    click_at(&mut a, &mut ctx, digits, 1200);
    let Some(form::Control::UpDown(u)) = a.sheet.open().item("desktops").map(|i| &i.control) else {
        panic!("the number of desktops is a number field");
    };
    assert_eq!(
        u.edit().map(|e| e.selection()),
        Some(0..u.shown().len()),
        "the digits are selected"
    );

    a.open_page(Page::Search, None);
    a.event(MAIN, Event::Resized, &mut ctx);
    let row = point_in(&a, &ctx, crate::providerpage::LIST, |p| {
        matches!(
            p,
            form::Part::List(reprisekit::controls::listbox::Hit::Row(0))
        )
    });
    click_at(&mut a, &mut ctx, row, 3000);
    let reply = click_at(&mut a, &mut ctx, row, 3200);
    assert!(!reply.quit, "the sheet stays open");
}

/// A schedule's state now is the compositor's: under "night" or "hours" the Appearance sample
/// wears the look the session is in, dark or light, and turns with it before the next draw.
#[test]
fn under_a_schedule_the_sample_wears_the_sessions_look_now() {
    let mut a = app("scheduled-sample");
    a.open_page(Page::Appearance, None);
    let mut want = a.want.clone();
    want.interface = "blue".into();
    want.mode = "hours".into();
    a.want = want.clone();
    crate::pages::follow(&mut a.sheet, &want);
    let sample_dark = |a: &SettingsApp| match &a.sheet.pages[Page::Appearance.index()]
        .item(crate::appearancepage::SAMPLE)
        .unwrap()
        .control
    {
        form::Control::Sample { theme, .. } => theme.colors.is_dark(),
        _ => panic!("a sample"),
    };
    let mut ctx = test_ctx();
    ctx.size = a.initial_size();
    let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    repriseapp::set_session_dark(true);
    a.draw(MAIN, &mut c, &ctx);
    assert!(sample_dark(&a), "dark while the session is dark");
    repriseapp::set_session_dark(false);
    a.draw(MAIN, &mut c, &ctx);
    assert!(!sample_dark(&a), "light again once the session is light");
}
