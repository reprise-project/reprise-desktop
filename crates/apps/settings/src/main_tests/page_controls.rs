use super::*;
use reprisebus::bolt::{Device, Found, Status};

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
    assert!(a.error.is_none());
    assert_eq!(a.want, a.loaded, "the switch is no change for Apply");

    remote_read(
        &mut a,
        remotehost::Remote::default(),
        Some("Remote Desktop could not be turned on.".into()),
    );
    assert!(a.error.is_some(), "a failed switch says why");
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
        a.asking.as_ref().map(|q| q.msg.fields.text.as_str()),
        Some(apply::RUN_AS_QUESTION)
    );
    a.event(ASK_WIN, Event::Key(key(keysym::ESCAPE)), &mut ctx);
    assert!(a.asking.is_none());
    assert_eq!(a.list.checks, Some(vec![true]), "Cancel keeps it");

    a.list_key(&form::Key::Space, &cm);
    a.event(ASK_WIN, Event::Key(key(keysym::RETURN)), &mut ctx);
    assert_eq!(a.list.checks, Some(vec![false]), "OK unchecks it");
    assert!(a.dirty(), "Apply writes it");
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
    use crate::providerpage::{HELD_NOTE, Installed, LIST};
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
        (vec![true], vec![true], vec![HELD_NOTE.to_string()]),
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
    assert!(a.error.is_some(), "a failed switch says why");
    assert_eq!(a.sheet.pages[at].checked(magnifierpage::READER), Some(true));
    a.sheet.pages[at].set_check(magnifierpage::READER, false);
    a.sheet_event(propsheet::Event::Page(
        at,
        form::Event::Changed(magnifierpage::READER),
    ));
    assert!(!a.reader.running, "the next click switches again");
}

fn airplane_read(a: &mut SettingsApp, state: Option<bool>, failed: Option<String>) {
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
    airplane_read(&mut a, Some(false), None);
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(false));
    assert!(lit(&a));

    a.sheet.pages[p].set_check(pages::AIRPLANE, true);
    a.sheet_event(propsheet::Event::Page(
        p,
        form::Event::Changed(pages::AIRPLANE),
    ));
    assert!(a.airplane.acting);
    assert!(!lit(&a), "greyed while the radios turn");
    airplane_read(&mut a, Some(true), None);
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
    assert!(lit(&a));
    assert_eq!(a.want, a.loaded, "the check box is no change for Apply");

    a.airplane.acting = true;
    airplane_read(
        &mut a,
        Some(true),
        Some("Airplane mode could not be turned off.".into()),
    );
    assert!(a.error.is_some(), "a failed change says why");
    assert_eq!(a.sheet.pages[p].checked(pages::AIRPLANE), Some(true));
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
    assert!(a.error.is_none(), "a dismissed prompt says nothing");
    assert!(lit(&a));
    a.sheet_event(propsheet::Event::Page(p, form::Event::Pressed(SYSTEM)));
    region_set(&mut a, Ok(vec![("LANG".into(), "de_DE.UTF-8".into())]));
    assert!(a.error.is_none());
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
    assert!(a.error.is_some(), "a refusal says why");
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
    assert!(a.error.is_some(), "a failed change says why");
    assert!(tb_lit(&a, pages::TB_AUTHORIZE));
    assert_eq!(a.want, a.loaded, "the buttons are no change for Apply");
}
