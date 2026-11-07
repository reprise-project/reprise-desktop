use super::*;
use reprisekit::controls::{form, pagelist, propsheet};
use repriselogin::Interface;
use repriselogin::machine::{LoginConfig, MachineConfig, MachineRemote};
use std::cell::RefCell;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

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
    static SENT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn helper_saves(toml: &str) -> io::Result<Output> {
    SENT.with(|s| s.borrow_mut().push(toml.into()));
    Ok(exited(0, ""))
}

fn helper_refuses(_: &str) -> io::Result<Output> {
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
        }
    );
    assert!(a.error.is_none());
    assert!(!a.dirty(), "a save leaves nothing to apply");
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
    let cfg = MachineConfig::default();
    let told = |code: i32, stderr: &'static str| {
        loginhost::apply(&cfg, &move |_: &str| Ok(exited(code, stderr)))
    };
    assert_eq!(told(0, ""), Ok(()));
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
    for code in [126, 127] {
        assert_eq!(
            told(code, "Error executing command as another user\n"),
            Err("The change was not authorized.".into()),
            "pkexec exit {code}"
        );
    }
    let missing = loginhost::apply(&cfg, &|_: &str| {
        Err(io::Error::from(io::ErrorKind::NotFound))
    });
    assert_eq!(
        missing,
        Err("The login manager's settings could not be saved.\n\nentity not found".into())
    );
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
        "Remote logins to this computer are set on the Login Manager page."
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
fn the_per_user_port_is_greyed_under_reprise_login() {
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
    assert!(!lit(page, "port"));
    assert!(!lit(page, "port_label"));
    assert_eq!(
        label(page, "allow_note"),
        "The login manager's port is used."
    );
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
