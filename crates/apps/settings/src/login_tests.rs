use super::*;
use loginhost::Withdraw;
use repriseapp::heads::Head;
use reprisekit::controls::{form, pagelist, propsheet};
use repriselogin::Interface;
use repriselogin::machine::{
    BACKGROUND_PICTURE, Background, LoginConfig, LoginMode, LoginOutput, MachineConfig,
    MachineRemote, Transform, file_tag, parse_save_request,
};
use std::cell::RefCell;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};
use std::time::Duration;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "reprise-settings-login-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

/// The app on a config of its own, its machine settings read from `dir`'s `login.toml` and its
/// fingerprint from `dir`'s `fingerprint`.
fn app(dir: &Path) -> SettingsApp {
    let mut a = SettingsApp::open(dir.join("config.toml"), "us".into());
    a.login_files = loginhost::Files {
        config: dir.join("login.toml"),
        fingerprint: dir.join("fingerprint"),
    };
    a
}

fn login(a: &SettingsApp) -> &form::Form {
    &a.sheet.pages[Page::LoginManager.index()]
}

fn open_login(a: &mut SettingsApp) {
    assert!(a.open_page(Page::LoginManager, None));
    a.ensure_login();
    a.wake();
}

fn combo_text(f: &form::Form, id: &str) -> String {
    match &f
        .item(id)
        .unwrap_or_else(|| panic!("{id} is on the page"))
        .control
    {
        form::Control::Combo(c) => c.items[c.selected].text.clone(),
        c => panic!("{id} is a combo, not {c:?}"),
    }
}

fn combo_rows(f: &form::Form, id: &str) -> Vec<String> {
    match &f.item(id).unwrap().control {
        form::Control::Combo(c) => c.items.iter().map(|i| i.text.clone()).collect(),
        c => panic!("{id} is a combo, not {c:?}"),
    }
}

fn updown(f: &form::Form, id: &str) -> i32 {
    match &f.item(id).unwrap().control {
        form::Control::UpDown(u) => u.value(),
        c => panic!("{id} is an up-down, not {c:?}"),
    }
}

fn label(f: &form::Form, id: &str) -> String {
    match &f
        .item(id)
        .unwrap_or_else(|| panic!("{id} is on the page"))
        .control
    {
        form::Control::Label { text, .. } => text.clone(),
        c => panic!("{id} is a label, not {c:?}"),
    }
}

fn lines(f: &form::Form, id: &str) -> Vec<String> {
    match &f.item(id).unwrap().control {
        form::Control::Lines(l) | form::Control::GreyedLines(l) => l.clone(),
        c => panic!("{id} is a note, not {c:?}"),
    }
}

fn lit(f: &form::Form, id: &str) -> bool {
    match &f.item(id).unwrap().control {
        form::Control::UpDown(u) => u.enabled,
        form::Control::Label { enabled, .. }
        | form::Control::Check { enabled, .. }
        | form::Control::Radio { enabled, .. }
        | form::Control::Button { enabled, .. } => *enabled,
        form::Control::TrackBar(t) => t.enabled,
        c => panic!("{id}: {c:?}"),
    }
}

/// Control `id` of page `p` moved: the sheet hears of it as a click or key would tell it.
fn changed(a: &mut SettingsApp, p: Page, id: &'static str) {
    a.sheet_event(propsheet::Event::Page(p.index(), form::Event::Changed(id)));
}

fn pick(a: &mut SettingsApp, id: &'static str, row: usize) {
    let page = &mut a.sheet.pages[Page::LoginManager.index()];
    page.set_combo(id, row);
    changed(a, Page::LoginManager, id);
}

fn exited(code: i32, stderr: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(code << 8),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

thread_local! {
    /// The settings of each save request a helper stand-in got, the tag each expected and the
    /// picture each carried.
    static SENT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TAGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PICTURES: RefCell<Vec<Option<Vec<u8>>>> = const { RefCell::new(Vec::new()) };
    /// The file a stand-in for another administrator writes to while the prompt is up, and what.
    static MEANWHILE: RefCell<Option<(PathBuf, &'static str)>> = const { RefCell::new(None) };
}

/// Keeps what a helper stand-in was sent.
fn record(input: &[u8]) {
    let req = parse_save_request(input).expect("a save request");
    SENT.with(|s| s.borrow_mut().push(req.toml.into()));
    TAGS.with(|t| t.borrow_mut().push(req.tag.into()));
    PICTURES.with(|p| p.borrow_mut().push(req.picture.map(<[u8]>::to_vec)));
}

fn helper_saves(input: &[u8], _: &Withdraw) -> io::Result<Output> {
    record(input);
    Ok(exited(0, ""))
}

/// The helper when the file was rewritten while its prompt was up: it finds the tag changed.
fn helper_finds_it_changed(input: &[u8], _: &Withdraw) -> io::Result<Output> {
    record(input);
    if let Some((path, text)) = MEANWHILE.with(|m| m.borrow_mut().take()) {
        std::fs::write(path, text)?;
    }
    Ok(exited(
        3,
        "/etc/reprise/login.toml changed since the settings were read\n",
    ))
}

fn helper_refuses(_: &[u8], _: &Withdraw) -> io::Result<Output> {
    Ok(exited(
        2,
        "remote_desktop.port: 0 is not between 1 and 65535\n",
    ))
}

#[test]
fn the_login_manager_page_reads_the_machine_settings() {
    let dir = tmp("read");
    std::fs::write(
        dir.join("login.toml"),
        "[login]\ninterface = \"blue\"\nshow_users = false\nautologin_user = \"anna\"\n\n\
         [remote_desktop]\nenabled = true\nport = 3390\ngroup = \"rdp\"\nmax_sessions = 3\n\
         end_disconnected_after = 240\n",
    )
    .unwrap();
    std::fs::write(dir.join("fingerprint"), "3F:9A:01\n").unwrap();
    let mut a = app(&dir);
    open_login(&mut a);
    let page = login(&a);
    assert_eq!(combo_text(page, "login_interface"), "Blue");
    assert_eq!(page.checked("login_show_users"), Some(false));
    assert_eq!(combo_text(page, "login_autologin"), "anna");
    assert_eq!(page.checked("login_remote"), Some(true));
    assert_eq!(updown(page, "login_port"), 3390);
    assert_eq!(
        lines(page, "login_group_note"),
        ["Members of the group rdp can log on remotely."]
    );
    assert_eq!(updown(page, "login_sessions"), 3);
    assert_eq!(combo_text(page, "login_end_after"), "4 hours");
    assert_eq!(page.edit_text("login_fingerprint"), "3F:9A:01");
    assert!(!a.dirty(), "reading the page is no change for Apply");
}

#[test]
fn automatic_logon_offers_the_accounts_the_login_screen_lists() {
    let dir = tmp("users");
    let mut a = app(&dir);
    a.login_source = repriselogin::users::sample;
    open_login(&mut a);
    assert_eq!(
        combo_rows(login(&a), "login_autologin"),
        ["Nobody", "anna", "old", "zoe"]
    );
}

#[test]
fn without_the_file_the_page_shows_the_defaults_and_no_fingerprint() {
    let dir = tmp("defaults");
    let mut a = app(&dir);
    open_login(&mut a);
    let page = login(&a);
    assert_eq!(combo_text(page, "login_interface"), "The last one used");
    assert_eq!(page.checked("login_show_users"), Some(true));
    assert_eq!(combo_text(page, "login_autologin"), "Nobody");
    assert_eq!(page.checked("login_remote"), Some(false));
    assert_eq!(updown(page, "login_port"), 3389);
    assert_eq!(updown(page, "login_sessions"), 8);
    assert_eq!(combo_text(page, "login_end_after"), "Never");
    assert_eq!(page.edit_text("login_fingerprint"), "Not running");
    for id in ["login_port", "login_sessions"] {
        assert!(!lit(page, id), "{id} greys while remote logins are off");
    }
}

#[test]
fn apply_sends_the_edited_machine_settings_to_the_helper() {
    let dir = tmp("apply");
    let mut a = app(&dir);
    a.login_run = helper_saves;
    open_login(&mut a);
    pick(&mut a, "login_interface", 2);
    let page = &mut a.sheet.pages[Page::LoginManager.index()];
    page.set_check("login_remote", true);
    changed(&mut a, Page::LoginManager, "login_remote");
    assert!(lit(login(&a), "login_port"), "the port lights with the box");
    if let Some(form::Control::UpDown(u)) = a.sheet.pages[Page::LoginManager.index()]
        .item_mut("login_port")
        .map(|i| &mut i.control)
    {
        u.set(3390);
    }
    changed(&mut a, Page::LoginManager, "login_port");
    pick(&mut a, "login_end_after", 2);
    assert!(a.dirty());

    SENT.with(|s| s.borrow_mut().clear());
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    let sent = SENT.with(|s| s.borrow().clone());
    assert_eq!(sent.len(), 1, "one save");
    let (got, warnings) = MachineConfig::parse(&sent[0]);
    assert_eq!(warnings, Vec::<String>::new());
    assert_eq!(
        got,
        MachineConfig {
            login: LoginConfig {
                interface: Some(Interface::Glass),
                ..LoginConfig::default()
            },
            remote: MachineRemote {
                enabled: true,
                port: 3390,
                end_disconnected_after_min: Some(60),
                ..MachineRemote::default()
            },
            outputs: Vec::new(),
        }
    );
    assert!(a.error.is_none());
    assert!(!a.dirty(), "a save leaves nothing to apply");
}

fn helper_dismissed(input: &[u8], _: &Withdraw) -> io::Result<Output> {
    record(input);
    Ok(exited(126, ""))
}

fn sent_interfaces() -> Vec<Option<Interface>> {
    SENT.with(|s| s.borrow().clone())
        .iter()
        .map(|t| MachineConfig::parse(t).0.login.interface)
        .collect()
}

/// The app on `tag`'s files with the page open, `run` as the helper and nothing sent yet.
fn saving_app(tag: &str, run: loginhost::Run) -> SettingsApp {
    saving_app_in(&tmp(tag), run)
}

fn saving_app_in(dir: &Path, run: loginhost::Run) -> SettingsApp {
    let mut a = app(dir);
    a.login_run = run;
    open_login(&mut a);
    SENT.with(|s| s.borrow_mut().clear());
    a
}

/// An event that carries out a pending OK or Cancel press, as the next one in the loop does.
fn settle(a: &mut SettingsApp, ctx: &mut Ctx) -> Reply {
    a.event(repriseapp::MAIN, Event::Blink, ctx)
}

/// Apply with Glass picked; tests run the save at once, and its end is read on the next wake.
fn apply_glass(a: &mut SettingsApp) {
    pick(a, "login_interface", 2);
    a.sheet_event(propsheet::Event::Apply);
    assert!(a.login_job.is_some(), "the save is under way");
}

#[test]
fn an_edit_made_during_a_save_stays_for_the_next_apply() {
    let mut a = saving_app("edit-during", helper_saves);
    apply_glass(&mut a);
    pick(&mut a, "login_interface", 0);
    a.wake();
    assert_eq!(combo_text(login(&a), "login_interface"), "Classic");
    assert!(
        a.login_job.is_none(),
        "no second password prompt on its own"
    );
    assert_eq!(sent_interfaces(), [Some(Interface::Glass)]);
    assert!(a.dirty(), "Apply stays lit for the edit");
}

#[test]
fn apply_pressed_again_during_a_save_sends_the_edit_after_it() {
    let mut a = saving_app("apply-during", helper_saves);
    apply_glass(&mut a);
    pick(&mut a, "login_interface", 0);
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    a.wake();
    assert_eq!(
        sent_interfaces(),
        [Some(Interface::Glass), Some(Interface::Classic)],
        "the edit goes in a second save"
    );
    assert!(a.login_job.is_none(), "no third save");
    assert!(!a.dirty());
}

#[test]
fn ok_during_a_save_closes_once_the_edits_after_it_are_saved() {
    let mut a = saving_app("ok-during", helper_saves);
    let mut ctx = crate::tests::test_ctx();
    apply_glass(&mut a);
    pick(&mut a, "login_interface", 1);
    a.sheet_event(propsheet::Event::Ok);
    assert!(!settle(&mut a, &mut ctx).quit, "OK waits for the save");
    assert!(!a.wake().quit, "an edit is left to save");
    assert!(a.wake().quit, "nothing left: the sheet closes");
    assert_eq!(
        sent_interfaces(),
        [Some(Interface::Glass), Some(Interface::Blue)]
    );
}

#[test]
fn ok_during_a_failed_save_keeps_the_sheet_open_with_why() {
    let mut a = saving_app("ok-fails", helper_dismissed);
    let mut ctx = crate::tests::test_ctx();
    apply_glass(&mut a);
    a.sheet_event(propsheet::Event::Ok);
    assert!(!settle(&mut a, &mut ctx).quit);
    assert!(!a.wake().quit, "the sheet stays open");
    let text = a.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(text.as_deref(), Some("The change was not authorized."));
    assert!(a.dirty(), "the change stays to apply");
    assert!(!a.wake().quit, "the close it waited on is spent");
}

#[test]
fn cancel_or_a_close_during_a_save_closes_when_it_ends_saved_or_not() {
    let runs = [
        ("saved", helper_saves as loginhost::Run),
        ("dismissed", helper_dismissed),
    ];
    for ((tag, run), cancel) in runs.into_iter().flat_map(|r| [(r, true), (r, false)]) {
        let mut a = saving_app(tag, run);
        let mut ctx = crate::tests::test_ctx();
        apply_glass(&mut a);
        pick(&mut a, "login_interface", 1);
        let r = if cancel {
            a.sheet_event(propsheet::Event::Cancel);
            assert_eq!(
                combo_text(login(&a), "login_interface"),
                "Glass",
                "{tag}: Cancel drops the edit, not the save"
            );
            settle(&mut a, &mut ctx)
        } else {
            a.event(repriseapp::MAIN, Event::CloseRequest, &mut ctx)
        };
        assert!(
            !r.quit,
            "{tag}, cancel {cancel}: the close waits for the save"
        );
        assert!(a.wake().quit, "{tag}, cancel {cancel}: the sheet closes");
        assert!(
            a.error.is_none(),
            "{tag}, cancel {cancel}: no box over a closing sheet"
        );
        assert_eq!(
            sent_interfaces(),
            [Some(Interface::Glass)],
            "{tag}: one save"
        );
    }
}

#[test]
fn the_open_page_reads_the_fingerprint_again_as_the_front_starts_and_stops() {
    let dir = tmp("fingerprint-poll");
    let mut a = app(&dir);
    open_login(&mut a);
    assert_eq!(login(&a).edit_text("login_fingerprint"), "Not running");
    std::fs::write(dir.join("fingerprint"), "3F:9A:01\n").unwrap();
    a.wake();
    a.wake();
    assert_eq!(login(&a).edit_text("login_fingerprint"), "3F:9A:01");
    std::fs::remove_file(dir.join("fingerprint")).unwrap();
    a.wake();
    a.wake();
    assert_eq!(login(&a).edit_text("login_fingerprint"), "Not running");
}

#[test]
fn a_save_shows_the_fingerprint_the_front_wrote_since() {
    let dir = tmp("fingerprint-after");
    let mut a = app(&dir);
    a.login_run = helper_saves;
    open_login(&mut a);
    assert_eq!(login(&a).edit_text("login_fingerprint"), "Not running");
    std::fs::write(dir.join("fingerprint"), "3F:9A:01\n").unwrap();
    apply_glass(&mut a);
    a.wake();
    assert_eq!(login(&a).edit_text("login_fingerprint"), "3F:9A:01");
}

#[test]
fn a_close_after_a_save_remembers_the_window() {
    let dir = tmp("close-geometry");
    let mut a = app(&dir);
    a.login_run = helper_saves;
    open_login(&mut a);
    let state = dir.join("settings.toml");
    a.state_path = Some(state.clone());
    let mut ctx = crate::tests::test_ctx();
    apply_glass(&mut a);
    a.window_dirty = true;
    a.sheet_event(propsheet::Event::Ok);
    assert!(!settle(&mut a, &mut ctx).quit);
    assert!(a.wake().quit);
    assert!(state.exists(), "the window's place is saved");
}

#[test]
fn a_refused_helper_shows_its_reasons() {
    let dir = tmp("refused");
    let mut a = app(&dir);
    a.login_run = helper_refuses;
    open_login(&mut a);
    pick(&mut a, "login_interface", 0);
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    let text = a.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(
        text.as_deref(),
        Some(
            "The login manager refused the settings:\n\n\
             remote_desktop.port: 0 is not between 1 and 65535"
        )
    );
    assert!(a.dirty(), "the change stays to apply");
}

#[test]
fn each_helper_exit_tells_its_own_message() {
    use loginhost::Unsaved;
    let cfg = MachineConfig::default();
    let exits = |code: i32, stderr: &'static str| {
        loginhost::apply(&cfg, None, "none", &move |_: &[u8]| {
            Ok(exited(code, stderr))
        })
    };
    let told = |code: i32, stderr: &'static str| {
        exits(code, stderr).map_err(|e| match e {
            Unsaved::Failed(why) => why,
            Unsaved::Changed => panic!("exit {code} is no changed file"),
        })
    };
    assert_eq!(told(0, ""), Ok(()));
    assert_eq!(
        exits(
            3,
            "/etc/reprise/login.toml changed since the settings were read\n"
        ),
        Err(Unsaved::Changed)
    );
    assert_eq!(
        told(1, "/etc/reprise/login.toml: Read-only file system\n"),
        Err("The login manager's settings could not be saved.\n\n\
             /etc/reprise/login.toml: Read-only file system"
            .into())
    );
    assert_eq!(
        told(
            2,
            "login.interface: \"aqua\" is not classic, blue or glass\n"
        ),
        Err("The login manager refused the settings:\n\n\
             login.interface: \"aqua\" is not classic, blue or glass"
            .into())
    );
    assert_eq!(
        told(
            126,
            "Error executing command as another user: Request dismissed\n"
        ),
        Err("The change was not authorized.".into()),
        "the prompt dismissed"
    );
    assert_eq!(
        told(
            127,
            "Error executing command as another user: Not authorized\n\n\
             This incident has been reported.\n"
        ),
        Err("The change was not authorized.".into()),
        "a polkit denial"
    );
    assert_eq!(
        told(127, "No authentication agent found.\n"),
        Err("The change was not authorized.\n\nNo authentication agent found.".into())
    );
    let missing = loginhost::apply(&cfg, None, "none", &|_: &[u8]| {
        Err(io::Error::from(io::ErrorKind::NotFound))
    });
    assert_eq!(
        missing,
        Err(Unsaved::Failed(
            "The login manager's settings could not be saved.\n\nentity not found".into()
        ))
    );
}

#[test]
fn a_missing_helper_is_named_and_pkexec_never_runs() {
    let missing = loginhost::apply(
        &MachineConfig::default(),
        None,
        "none",
        &|input: &[u8]| {
            let pkexec = Path::new("/usr/bin/pkexec");
            let helper = Path::new("/nonexistent/reprise-login-settings");
            loginhost::run_helper(pkexec, helper, input, &Withdraw::default())
        },
    );
    assert_eq!(
        missing,
        Err(loginhost::Unsaved::Failed(
            "The login manager's settings could not be saved.\n\n\
             /nonexistent/reprise-login-settings is not installed."
                .into()
        ))
    );
}

fn error_text(a: &SettingsApp) -> Option<String> {
    a.error.as_ref().map(|b| b.fields.text.clone())
}

#[test]
fn an_unreadable_or_invalid_file_is_never_saved_over() {
    let files = [
        ("not-toml", Some("[login\nshow_users = false\n")),
        (
            "bad-value",
            Some("[remote_desktop]\nenabled = true\nport = 0\n"),
        ),
        ("a-directory", None),
    ];
    for (tag, text) in files {
        let dir = tmp(&format!("unreadable-{tag}"));
        let path = dir.join("login.toml");
        match text {
            Some(t) => std::fs::write(&path, t).unwrap(),
            None => std::fs::create_dir(&path).unwrap(),
        }
        let mut a = app(&dir);
        a.login_run = helper_saves;
        open_login(&mut a);
        let opened = error_text(&a).unwrap_or_default();
        let (head, why) = opened.split_once("\n\n").unwrap_or((&opened, ""));
        let at = path.display();
        let expected_head = match tag {
            "a-directory" => format!(
                "The login manager's settings in {at} could not be read, so this page doesn't \
                 save over them."
            ),
            _ => format!(
                "The login manager's settings in {at} have values this page can't use, so it \
                 doesn't save over them. Correct or remove these lines as root, then press \
                 Apply again:"
            ),
        };
        assert_eq!(head, expected_head, "{tag}: told when the page opens");
        match tag {
            "bad-value" => assert_eq!(why, "remote_desktop.port: 0 is not between 1 and 65535"),
            "not-toml" => assert!(
                why.starts_with("TOML parse error") && !why.contains('\n'),
                "a parse error on one line: {why}"
            ),
            _ => assert_eq!(why, "Is a directory (os error 21)"),
        }

        a.error = None;
        SENT.with(|s| s.borrow_mut().clear());
        pick(&mut a, "login_interface", 2);
        assert!(a.dirty(), "{tag}: the edit lights Apply");
        a.sheet_event(propsheet::Event::Apply);
        a.wake();
        assert_eq!(SENT.with(|s| s.borrow().len()), 0, "{tag}: nothing is sent");
        assert_eq!(error_text(&a), Some(opened), "{tag}: told again on Apply");
        assert!(a.dirty(), "{tag}: the edit stays");

        a.error = None;
        if text.is_none() {
            std::fs::remove_dir(&path).unwrap();
        }
        std::fs::write(&path, "[login]\ninterface = \"blue\"\n").unwrap();
        a.sheet_event(propsheet::Event::Apply);
        a.wake();
        assert_eq!(
            SENT.with(|s| s.borrow().len()),
            0,
            "{tag}: the fixed file reloads first"
        );
        assert_eq!(error_text(&a).as_deref(), Some(CHANGED), "{tag}");
        assert_eq!(combo_text(login(&a), "login_interface"), "Blue", "{tag}");
        assert!(!a.dirty(), "{tag}: the page holds the fixed file");

        a.error = None;
        pick(&mut a, "login_interface", 2);
        a.sheet_event(propsheet::Event::Apply);
        a.wake();
        assert_eq!(
            SENT.with(|s| s.borrow().len()),
            1,
            "{tag}: the next Apply saves"
        );
        assert_eq!(error_text(&a), None, "{tag}");
    }
}

#[test]
fn the_error_at_open_has_the_stop_icon_when_a_wake_is_the_first_event() {
    let dir = tmp("open-icon");
    std::fs::write(dir.join("login.toml"), "[login\n").unwrap();
    let mut ctx = crate::tests::test_ctx();
    let stop = Canvas::new(32, 32);
    let theme = ctx.icons.theme();
    ctx.icons.put_icons(
        &theme,
        vec![("dialog-error".into(), 32, Some(stop.clone()))],
    );
    let mut a = app(&dir);
    assert!(a.open_page(Page::LoginManager, None));
    a.event(repriseapp::MAIN, Event::Wake, &mut ctx);
    let icon = a
        .error
        .as_ref()
        .map(|b| b.fields.icon.as_ref() == Some(&stop));
    assert_eq!(icon, Some(true), "the box at open has dialog-error");
}

const CHANGED: &str = "Your changes were not saved: the login manager's settings changed since \
                       this page opened. The page now shows the new settings; make your changes \
                       again.";

#[test]
fn a_file_changed_since_the_page_opened_is_not_overwritten() {
    let dir = tmp("changed-since");
    let path = dir.join("login.toml");
    std::fs::write(&path, "[login]\ninterface = \"blue\"\n").unwrap();
    let mut a = app(&dir);
    a.login_run = helper_saves;
    open_login(&mut a);
    SENT.with(|s| s.borrow_mut().clear());
    pick(&mut a, "login_interface", 2);
    std::fs::write(
        &path,
        "[login]\ninterface = \"blue\"\nautologin_user = \"zoe\"\n",
    )
    .unwrap();
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(SENT.with(|s| s.borrow().len()), 0, "nothing is sent");
    assert_eq!(error_text(&a).as_deref(), Some(CHANGED));
    assert_eq!(combo_text(login(&a), "login_autologin"), "zoe");
    assert_eq!(combo_text(login(&a), "login_interface"), "Blue");
    assert!(!a.dirty(), "the page holds the file as it now reads");

    a.error = None;
    pick(&mut a, "login_interface", 2);
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    let sent = SENT.with(|s| s.borrow().clone());
    assert_eq!(sent.len(), 1, "an edit on the new file saves");
    assert_eq!(
        MachineConfig::parse(&sent[0])
            .0
            .login
            .autologin_user
            .as_deref(),
        Some("zoe"),
        "the other change is kept"
    );
}

#[test]
fn picking_the_hand_edited_end_after_row_back_restores_its_minutes() {
    let dir = tmp("end-after-back");
    std::fs::write(
        dir.join("login.toml"),
        "[remote_desktop]\nenabled = true\nend_disconnected_after = 90\n",
    )
    .unwrap();
    let mut a = app(&dir);
    open_login(&mut a);
    assert_eq!(combo_text(login(&a), "login_end_after"), "90 minutes");
    pick(&mut a, "login_end_after", 1);
    assert!(a.dirty());
    pick(&mut a, "login_end_after", 5);
    assert!(!a.dirty(), "90 minutes again is the file's own value");
}

#[test]
fn the_login_manager_row_shows_only_where_reprise_login_is_installed() {
    let dir = tmp("absent");
    let mut a = app(&dir);
    let shown = |a: &mut SettingsApp| {
        let list = a.sheet.list_mut().unwrap();
        list.shows(pagelist::Pick::Page(Page::LoginManager.index()))
    };
    assert!(shown(&mut a));
    a.absent = vec![Page::LoginManager];
    a.search("");
    assert!(!shown(&mut a));
    a.search("login");
    assert!(!shown(&mut a), "a query does not bring it back");
    a.search("");
    assert!(!a.open_page(Page::LoginManager, None));
    assert_ne!(registry::open(&a.sheet), Page::LoginManager);
}

fn remote_read(a: &mut SettingsApp, remote: remotehost::Remote) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(remotehost::Reading {
        remote,
        failed: None,
    })
    .unwrap();
    a.remote_job = Some(rx);
    assert!(a.remote_done());
}

fn managed() -> remotehost::Remote {
    remotehost::Remote {
        manager: true,
        machine_fingerprint: Some("AB:CD".into()),
        ..remotehost::Remote::default()
    }
}

fn remote(a: &SettingsApp) -> &form::Form {
    &a.sheet.pages[Page::RemoteDesktop.index()]
}

#[test]
fn under_reprise_login_the_allow_box_is_a_note() {
    let dir = tmp("managed");
    let mut a = app(&dir);
    remote_read(&mut a, managed());
    let page = remote(&a);
    assert!(page.item(pages::REMOTE_ALLOW).is_none(), "no switch");
    assert_eq!(
        label(page, "managed_note"),
        "Remote logins are set on the Login Manager page."
    );
    assert_eq!(page.edit_text("fingerprint"), "AB:CD", "the machine's");
    assert!(lit(page, "quality"), "the session's settings still apply");
    assert!(!lit(page, pages::REMOTE_REGENERATE));

    remote_read(&mut a, remotehost::Remote::default());
    let page = remote(&a);
    assert_eq!(page.checked(pages::REMOTE_ALLOW), Some(false));
    assert!(page.item("managed_note").is_none());
}

#[test]
fn the_per_user_port_is_hidden_under_reprise_login() {
    let dir = tmp("managed-port");
    let mut a = app(&dir);
    remote_read(
        &mut a,
        remotehost::Remote {
            enabled: true,
            ..remotehost::Remote::default()
        },
    );
    assert!(lit(remote(&a), "port"));
    remote_read(&mut a, managed());
    let page = remote(&a);
    for id in ["port", "port_label", "allow_note"] {
        assert!(page.item(id).is_none(), "{id} is not on the page");
    }
}

#[test]
fn second_client_radio_writes_the_key() {
    let dir = tmp("second-client");
    let mut a = app(&dir);
    let page = remote(&a);
    assert_eq!(
        page.checked("second_client_join"),
        Some(true),
        "join unless set"
    );
    assert_eq!(
        label(page, "second_client_label"),
        "When another client connects:"
    );
    let page = &mut a.sheet.pages[Page::RemoteDesktop.index()];
    page.set_check("second_client_join", false);
    page.set_check("second_client_replace", true);
    changed(&mut a, Page::RemoteDesktop, "second_client_replace");
    assert!(a.apply());
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(
        text.contains("second_client = \"replace\""),
        "second_client = \"replace\" is written:\n{text}"
    );
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert_eq!(rd.second_client, repriseconf::remote::SecondClient::Replace);
}

#[test]
fn a_save_names_the_file_it_read_so_a_newer_one_is_not_replaced() {
    let text = "[login]\ninterface = \"blue\"\n";
    for file in [Some(text), None] {
        let dir = tmp(&format!("tag-{}", file.is_some()));
        if let Some(t) = file {
            std::fs::write(dir.join("login.toml"), t).unwrap();
        }
        let mut a = saving_app_in(&dir, helper_saves);
        TAGS.with(|t| t.borrow_mut().clear());
        apply_glass(&mut a);
        a.wake();
        assert_eq!(
            TAGS.with(|t| t.borrow().clone()),
            [file_tag(file.map(str::as_bytes))],
            "the save expects the file its reread found, a missing one too"
        );
    }
}

#[test]
fn a_file_changed_while_the_prompt_was_up_is_shown_and_not_overwritten() {
    let dir = tmp("changed-during");
    let path = dir.join("login.toml");
    std::fs::write(&path, "[remote_desktop]\nenabled = true\n").unwrap();
    let mut a = saving_app_in(&dir, helper_finds_it_changed);
    let turned_off = "[remote_desktop]\nenabled = false\n";
    MEANWHILE.with(|m| *m.borrow_mut() = Some((path.clone(), turned_off)));
    apply_glass(&mut a);
    a.wake();
    assert_eq!(error_text(&a).as_deref(), Some(CHANGED));
    assert_eq!(login(&a).checked("login_remote"), Some(false));
    assert_eq!(
        combo_text(login(&a), "login_interface"),
        "The last one used"
    );
    assert!(!a.dirty(), "the page holds the file as it is now");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), turned_off);
}

#[test]
fn a_withdrawn_save_stops_its_helper_at_once() {
    let dir = tmp("withdraw");
    let (helper, ran) = (dir.join("helper"), dir.join("ran"));
    std::fs::write(&helper, format!("touch {}\nexec sleep 30\n", ran.display())).unwrap();
    let withdraw = Withdraw::default();
    let w = withdraw.clone();
    let start = std::time::Instant::now();
    let run =
        std::thread::spawn(move || loginhost::run_helper(Path::new("/bin/sh"), &helper, b"", &w));
    while !ran.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    withdraw.stop();
    let out = run.join().unwrap().unwrap();
    assert_eq!(out.status.signal(), Some(9), "killed: {out:?}");
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_save_withdrawn_before_pkexec_starts_never_starts_it() {
    let dir = tmp("withdraw-early");
    let (helper, ran) = (dir.join("helper"), dir.join("ran"));
    std::fs::write(&helper, format!("touch {}\nexec sleep 30\n", ran.display())).unwrap();
    let withdraw = Withdraw::default();
    withdraw.stop();
    let start = std::time::Instant::now();
    let out = loginhost::run_helper(Path::new("/bin/sh"), &helper, b"", &withdraw);
    assert!(out.is_err(), "nothing ran: {out:?}");
    assert!(!ran.exists());
    assert!(start.elapsed() < Duration::from_secs(5));
}

fn head(name: &str, mode: (i32, i32, i32), scale: f64, position: (i32, i32)) -> Head {
    let m = repriseapp::heads::HeadMode {
        width: mode.0,
        height: mode.1,
        refresh_mhz: mode.2,
        preferred: false,
    };
    Head {
        name: name.into(),
        modes: vec![m],
        current: Some(0),
        enabled: true,
        position,
        scale,
        ..Head::default()
    }
}

/// The app with the page open on `dir`'s files, its Display page listing DP-1 rotated at 125%
/// left of eDP-1 at 200%, with `config` as config.toml and no saved layouts.
fn with_screens_in(dir: &Path, config: &str) -> SettingsApp {
    std::fs::write(dir.join("config.toml"), config).unwrap();
    let mut a = saving_app_in(dir, helper_saves);
    a.display.layouts = dir.join("displays.toml");
    let mut dp = head("DP-1", (2560, 1440, 143_912), 1.25, (0, 0));
    dp.transform = repriseapp::heads::Transform::Rotate90;
    a.heads_changed(vec![
        dp,
        head("eDP-1", (2880, 1800, 60_000), 2.0, (1440, 0)),
    ]);
    a
}

const EDP_PRIMARY: &str = "[[outputs]]\nname = \"eDP-1\"\nprimary = true\n";

/// [`with_screens_in`] with eDP-1 primary in config.toml.
fn with_screens(tag: &str) -> SettingsApp {
    with_screens_in(&tmp(tag), EDP_PRIMARY)
}

fn sent_primary(i: usize) -> Vec<String> {
    (sent_config(i).outputs.into_iter())
        .filter(|o| o.primary)
        .map(|o| o.name)
        .collect()
}

fn press(a: &mut SettingsApp, id: &'static str) {
    a.sheet_event(propsheet::Event::Page(
        Page::LoginManager.index(),
        form::Event::Pressed(id),
    ));
}

fn sent_config(i: usize) -> MachineConfig {
    let sent = SENT.with(|s| s.borrow().clone());
    let (c, warnings) = MachineConfig::parse(&sent[i]);
    assert_eq!(warnings, Vec::<String>::new());
    c
}

#[test]
fn use_my_display_settings_sends_each_screen_as_it_runs() {
    let mut a = with_screens("use-displays");
    assert_eq!(label(login(&a), "login_screens"), "Screens: automatic");
    assert!(!lit(login(&a), "login_reset_displays"), "nothing to reset");
    press(&mut a, "login_use_displays");
    assert_eq!(
        label(login(&a), "login_screens"),
        "Screens: 2, from your display settings"
    );
    assert!(lit(login(&a), "login_reset_displays"));
    assert!(a.dirty(), "Apply saves them");
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(
        sent_config(0).outputs,
        [
            LoginOutput {
                name: "DP-1".into(),
                mode: Some(LoginMode {
                    width: 2560,
                    height: 1440,
                    refresh_mhz: Some(143_912),
                }),
                quarters: 5,
                transform: Transform::Rotate90,
                position: Some((0, 0)),
                primary: false,
            },
            LoginOutput {
                name: "eDP-1".into(),
                mode: Some(LoginMode {
                    width: 2880,
                    height: 1800,
                    refresh_mhz: Some(60_000),
                }),
                quarters: 8,
                transform: Transform::Normal,
                position: Some((1440, 0)),
                primary: true,
            },
        ]
    );
    assert_eq!(PICTURES.with(|p| p.borrow().clone()), [None]);
    assert!(!a.dirty());
}

#[test]
fn reset_sends_no_screens_so_each_runs_its_preferred_mode() {
    let dir = tmp("reset-displays");
    std::fs::write(
        dir.join("login.toml"),
        "[[outputs]]\nname = \"DP-1\"\nscale = 2\n",
    )
    .unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    assert_eq!(
        label(login(&a), "login_screens"),
        "Screens: 1, from your display settings"
    );
    press(&mut a, "login_reset_displays");
    assert_eq!(label(login(&a), "login_screens"), "Screens: automatic");
    assert!(!lit(login(&a), "login_reset_displays"));
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(sent_config(0).outputs, []);
}

#[test]
fn use_my_display_settings_without_screens_says_so() {
    let mut a = saving_app("no-screens", helper_saves);
    press(&mut a, "login_use_displays");
    assert_eq!(
        error_text(&a).as_deref(),
        Some("No screens are listed, so there are no display settings to use.")
    );
    assert!(!a.dirty());
}

#[test]
fn the_background_offers_the_default_and_each_built_in() {
    let mut a = saving_app("background-rows", helper_saves);
    let rows = combo_rows(login(&a), "login_background");
    assert_eq!(rows[0], "Default");
    assert!(rows.contains(&"Tarn (dusk)".to_string()), "{rows:?}");
    assert!(!rows.contains(&"Picture".to_string()), "no picture yet");
    assert_eq!(combo_text(login(&a), "login_background"), "Default");
    let dusk = rows.iter().position(|r| r == "Tarn (dusk)").unwrap();
    pick(&mut a, "login_background", dusk);
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(
        sent_config(0).login.background,
        Some(Background::Builtin("reprise:tarn/dusk".into()))
    );

    let dir = tmp("background-default");
    std::fs::write(
        dir.join("login.toml"),
        "[login]\nbackground = \"reprise:tarn/dusk\"\n",
    )
    .unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    assert_eq!(combo_text(login(&a), "login_background"), "Tarn (dusk)");
    pick(&mut a, "login_background", 0);
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(sent_config(0).login.background, None, "Default");
}

#[test]
fn a_browsed_picture_goes_with_the_next_save_and_then_shows_as_picture() {
    let dir = tmp("background-picture");
    let photo = dir.join("sunset.png");
    std::fs::write(&photo, png()).unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    a.login_picture(photo.clone());
    assert_eq!(combo_text(login(&a), "login_background"), "sunset.png");
    assert!(a.dirty());
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(
        sent_config(0).login.background,
        Some(Background::Picture),
        "the settings name the copy"
    );
    assert_eq!(PICTURES.with(|p| p.borrow().clone()), [Some(png())]);
    assert!(!a.dirty(), "the picture went with the save");

    std::fs::write(
        dir.join("login.toml"),
        format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n"),
    )
    .unwrap();
    let a = saving_app_in(&dir, helper_saves);
    assert_eq!(combo_text(login(&a), "login_background"), "Picture");
}

#[test]
fn picking_another_background_drops_the_browsed_picture() {
    let dir = tmp("background-drop");
    let photo = dir.join("sunset.png");
    std::fs::write(&photo, png()).unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    a.login_picture(photo);
    assert!(a.dirty());
    pick(&mut a, "login_background", 0);
    assert!(!a.dirty(), "back to the file's Default");
    assert!(
        !combo_rows(login(&a), "login_background").contains(&"sunset.png".to_string()),
        "the picture's row goes"
    );
}

#[test]
fn a_picture_past_the_helpers_limit_is_refused_when_browsed() {
    let dir = tmp("background-huge");
    let photo = dir.join("huge.png");
    let f = std::fs::File::create(&photo).unwrap();
    f.set_len(repriselogin::machine::MAX_PICTURE as u64 + 1)
        .unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    a.login_picture(photo);
    assert_eq!(
        error_text(&a).as_deref(),
        Some("'huge.png' is larger than 32 MB, too large for the login screen.")
    );
    assert!(!a.dirty());
}

/// A 4x3 picture as a PNG.
fn png() -> Vec<u8> {
    let mut out = io::Cursor::new(Vec::new());
    (image::RgbImage::from_pixel(4, 3, image::Rgb([10, 120, 200])))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn a_picture_the_helper_would_refuse_is_named_before_the_password() {
    let whole = png();
    let mut wide = io::Cursor::new(Vec::new());
    (image::GrayImage::new(16385, 1).write_to(&mut wide, image::ImageFormat::Png)).unwrap();
    for (name, bytes, why) in [
        (
            "cut.png",
            whole[..whole.len() - 20].to_vec(),
            "the picture is damaged or cut short",
        ),
        (
            "wide.png",
            wide.into_inner(),
            "the picture is wider or taller than 16384 pixels",
        ),
    ] {
        let dir = tmp(&format!("background-refused-{name}"));
        let photo = dir.join(name);
        std::fs::write(&photo, &bytes).unwrap();
        let mut a = saving_app_in(&dir, helper_saves);
        a.login_picture(photo.clone());
        a.sheet_event(propsheet::Event::Apply);
        a.wake();
        assert!(
            SENT.with(|s| s.borrow().is_empty()),
            "{name}: no password asked"
        );
        assert_eq!(
            error_text(&a),
            Some(format!(
                "The login manager's settings could not be saved.\n\n{} can't be the login \
                 screen's background: {why}.",
                photo.display()
            ))
        );
        assert!(a.dirty(), "{name}: the pick stays");
    }
}

#[test]
fn use_my_display_settings_marks_the_primary_the_session_runs() {
    use repriseconf::displays::{Layout, Layouts, MonitorLayout, outputs_fingerprint};
    let dir = tmp("primary-saved-layout");
    let config = EDP_PRIMARY;
    let (parsed, _) = repriseconf::config::Config::parse_lenient(config).unwrap();
    let outputs = parsed.outputs;
    let monitor = |primary| MonitorLayout {
        enabled: true,
        mode: None,
        position: (0, 0),
        scale: 1.0,
        transform: repriseconf::config::Transform::Normal,
        primary,
    };
    let layouts = Layouts {
        outputs_seen: outputs_fingerprint(&outputs),
        list: vec![Layout::new(vec![
            ("DP-1".into(), monitor(true)),
            ("eDP-1".into(), monitor(false)),
        ])],
    };
    std::fs::write(dir.join("displays.toml"), layouts.render()).unwrap();
    let mut a = with_screens_in(&dir, config);
    press(&mut a, "login_use_displays");
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(
        sent_primary(0),
        ["DP-1"],
        "the saved layout's over config.toml's"
    );

    let mut a = with_screens_in(&tmp("primary-none"), "");
    press(&mut a, "login_use_displays");
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(
        sent_primary(0),
        Vec::<String>::new(),
        "none picked, none sent"
    );
}

#[test]
fn a_picture_browsed_while_a_save_waits_goes_with_the_next_apply() {
    let dir = tmp("browse-during-save");
    std::fs::write(
        dir.join("login.toml"),
        format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n"),
    )
    .unwrap();
    let photo = dir.join("lake.png");
    std::fs::write(&photo, png()).unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    apply_glass(&mut a);
    a.login_picture(photo);
    a.wake();
    assert_eq!(combo_text(login(&a), "login_background"), "lake.png");
    assert!(a.dirty(), "the picture is left to save");
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert_eq!(PICTURES.with(|p| p.borrow().clone()), [None, Some(png())]);
}

#[test]
fn a_file_that_is_no_picture_is_refused_when_browsed_whatever_its_name() {
    let dir = tmp("background-gif");
    let photo = dir.join("cat.png");
    std::fs::write(&photo, b"GIF89a\x04\x00\x03\x00").unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    a.login_picture(photo);
    assert_eq!(
        error_text(&a).as_deref(),
        Some("'cat.png' is not a PNG, JPEG or BMP picture, which the login screen shows.")
    );
    assert!(!a.dirty());
}

#[test]
fn settings_the_helper_would_refuse_are_never_sent() {
    let dir = tmp("refused-before-pkexec");
    let mut a = with_screens_in(&dir, "");
    let mut heads = a.display.connected.clone();
    heads[0].name = "DP 1; reboot".into();
    a.heads_changed(heads);
    press(&mut a, "login_use_displays");
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    assert!(SENT.with(|s| s.borrow().is_empty()), "no password asked");
    let why = error_text(&a).expect("why it was not saved");
    assert!(
        why.starts_with("The login manager's settings could not be saved.\n\n"),
        "{why}"
    );
    assert!(a.dirty(), "the edit stays");
}

#[test]
fn the_saved_picture_stays_offered_after_another_background_is_picked() {
    let dir = tmp("background-saved-row");
    std::fs::write(
        dir.join("login.toml"),
        format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n"),
    )
    .unwrap();
    let mut a = saving_app_in(&dir, helper_saves);
    pick(&mut a, "login_background", 0);
    assert_eq!(combo_text(login(&a), "login_background"), "Default");
    let rows = combo_rows(login(&a), "login_background");
    let picture = (rows.iter().position(|r| r == "Picture"))
        .unwrap_or_else(|| panic!("a Picture row in {rows:?}"));
    pick(&mut a, "login_background", picture);
    assert!(!a.dirty(), "back to the saved picture");
}

#[test]
fn every_display_transform_keeps_its_name_on_the_login_screen() {
    for t in repriseapp::heads::Transform::ALL {
        let login = Transform::parse(t.name()).map(Transform::name);
        assert_eq!(login, Some(t.name()), "{t:?}");
    }
}

#[test]
fn the_animate_box_reads_and_saves_the_login_screens_animation() {
    let dir = tmp("animation");
    std::fs::write(dir.join("login.toml"), "[login]\nanimation = false\n").unwrap();
    let mut a = app(&dir);
    a.login_run = helper_saves;
    open_login(&mut a);
    assert_eq!(login(&a).checked("login_animation"), Some(false));
    let page = &mut a.sheet.pages[Page::LoginManager.index()];
    page.set_check("login_animation", true);
    changed(&mut a, Page::LoginManager, "login_animation");
    SENT.with(|s| s.borrow_mut().clear());
    a.sheet_event(propsheet::Event::Apply);
    a.wake();
    let sent = SENT.with(|s| s.borrow().clone());
    assert_eq!(sent.len(), 1, "one save");
    assert!(MachineConfig::parse(&sent[0]).0.login.animation);
    assert!(
        !sent[0].contains("animation"),
        "on is the default: {}",
        sent[0]
    );
}
