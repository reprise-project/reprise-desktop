use super::*;
use reprisebus::testbus::{bus, connect};
use std::sync::{Arc, Mutex};

/// logind's restart-into calls: answers `can_entry` and `can_firmware`, lists `ids`, keeps every
/// target it is asked to set, and holds the targets set in its properties.
struct FakeLogind {
    can_entry: &'static str,
    can_firmware: &'static str,
    ids: Vec<String>,
    set: Arc<Mutex<Vec<String>>>,
    held: Arc<Mutex<Held>>,
    /// Clearing a target is refused unless the call allows a password prompt.
    clear_asks: bool,
}

/// What `RebootToBootLoaderEntry` and `RebootToFirmwareSetup` say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    pub entry: String,
    pub firmware: bool,
    /// Both properties fail to read.
    pub unreadable: bool,
}

fn unreadable(held: &Held) -> zbus::fdo::Result<()> {
    match held.unreadable {
        true => Err(zbus::fdo::Error::Failed("unreadable".into())),
        false => Ok(()),
    }
}

fn prompts(header: &zbus::message::Header<'_>) -> bool {
    (header.primary().flags()).contains(zbus::message::Flags::ALLOW_INTERACTIVE_AUTH)
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeLogind {
    fn can_reboot_to_boot_loader_entry(&self) -> String {
        self.can_entry.into()
    }

    fn can_reboot_to_firmware_setup(&self) -> String {
        self.can_firmware.into()
    }

    #[zbus(property)]
    fn boot_loader_entries(&self) -> Vec<String> {
        self.ids.clone()
    }

    #[zbus(property)]
    fn reboot_to_boot_loader_entry(&self) -> zbus::fdo::Result<String> {
        let held = self.held.lock().unwrap();
        unreadable(&held)?;
        Ok(held.entry.clone())
    }

    #[zbus(property)]
    fn reboot_to_firmware_setup(&self) -> zbus::fdo::Result<bool> {
        let held = self.held.lock().unwrap();
        unreadable(&held)?;
        Ok(held.firmware)
    }

    /// Refused where `CanRebootToBootLoaderEntry` says "no".
    fn set_reboot_to_boot_loader_entry(
        &self,
        id: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        if self.can_entry == "no" || (self.clear_asks && id.is_empty() && !prompts(&header)) {
            return Err(zbus::fdo::Error::AccessDenied("not allowed".into()));
        }
        self.set.lock().unwrap().push(format!("entry {id:?}"));
        self.held.lock().unwrap().entry = id.into();
        Ok(())
    }

    fn set_reboot_to_firmware_setup(
        &self,
        on: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        if self.clear_asks && !on && !prompts(&header) {
            return Err(zbus::fdo::Error::AccessDenied("not allowed".into()));
        }
        self.set.lock().unwrap().push(format!("firmware {on}"));
        self.held.lock().unwrap().firmware = on;
        Ok(())
    }
}

/// A fake logind on a bus of its own: a connection to it, what it was asked to set, and what it
/// holds.
pub(crate) struct Fake {
    _bus: reprisebus::testbus::Bus,
    _server: zbus::Connection,
    pub client: zbus::Connection,
    pub set: Arc<Mutex<Vec<String>>>,
    pub held: Arc<Mutex<Held>>,
}

impl Fake {
    /// Set `entry` and `firmware` as another program would, past the call log.
    pub(crate) fn hold(&self, entry: &str, firmware: bool) {
        *self.held.lock().unwrap() = Held {
            entry: entry.into(),
            firmware,
            unreadable: false,
        };
    }

    /// logind fails to read either target.
    pub(crate) fn unreadable(&self) {
        self.held.lock().unwrap().unreadable = true;
    }
}

pub(crate) fn logind(
    can_entry: &'static str,
    can_firmware: &'static str,
    ids: &[&str],
) -> Option<Fake> {
    serve(can_entry, can_firmware, ids, false)
}

/// A fake logind that sets targets without a password but asks for one to clear them.
pub(crate) fn logind_asking_to_clear() -> Option<Fake> {
    serve("yes", "yes", &[], true)
}

fn serve(
    can_entry: &'static str,
    can_firmware: &'static str,
    ids: &[&str],
    clear_asks: bool,
) -> Option<Fake> {
    let bus = bus()?;
    let set = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::new(Mutex::new(Held::default()));
    let fake = FakeLogind {
        can_entry,
        can_firmware,
        ids: ids.iter().map(|s| s.to_string()).collect(),
        set: set.clone(),
        held: held.clone(),
        clear_asks,
    };
    let address = bus.1.clone();
    let (server, client) = zbus::block_on(async {
        let server = zbus::connection::Builder::address(address.as_str())
            .name(LOGIND)
            .serve_at(LOGIND_PATH, fake)
            .build()
            .await
            .expect("the fake logind");
        (server, connect(&address).await)
    });
    Some(Fake {
        _bus: bus,
        _server: server,
        client,
        set,
        held,
    })
}

fn entry(id: &str, title: &str) -> BootTarget {
    BootTarget {
        entry: Some(id.into()),
        title: title.into(),
        password: false,
    }
}

#[test]
fn an_entry_id_shows_as_words_without_its_extension() {
    assert_eq!(pretty("linux-cachyos.conf"), "Linux cachyos");
    assert_eq!(pretty("arch_fallback.conf"), "Arch fallback");
    assert_eq!(pretty("linux-6.12.efi"), "Linux 6.12");
    assert_eq!(pretty("auto-efi-shell"), "Efi shell");
    assert_eq!(pretty("memtest86+"), "Memtest86+");
}

#[test]
fn titles_come_from_the_entry_files_and_shared_ones_carry_the_id() {
    let root = std::env::temp_dir().join(format!("reprise-restartinto-{}", std::process::id()));
    let dir = root.join("loader/entries");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.conf"),
        "# a\ntitle   Arch Linux \nlinux /vmlinuz\n",
    )
    .unwrap();
    std::fs::write(dir.join("b.conf"), "title Arch Linux\n").unwrap();
    std::fs::write(dir.join("c.conf"), "linux /vmlinuz\n").unwrap();
    let ids: Vec<String> = [
        "a.conf",
        "b.conf",
        "c.conf",
        "auto-reboot-to-firmware-setup",
        "x.efi",
    ]
    .map(String::from)
    .to_vec();
    let got = entries(&ids, true, &[PathBuf::from("/nonexistent"), root.clone()]);
    let titles: Vec<&str> = got.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Arch Linux (a.conf)", "Arch Linux (b.conf)", "C", "X"],
        "firmware setup's own entry is left out"
    );
    assert!(got.iter().all(|t| t.password));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_title_is_read_from_a_regular_file_and_its_first_64_kib_only() {
    let root =
        std::env::temp_dir().join(format!("reprise-restartinto-read-{}", std::process::id()));
    let roots = std::slice::from_ref(&root);
    let dir = root.join("loader/entries");
    std::fs::create_dir_all(&dir).unwrap();
    let pipe =
        std::ffi::CString::new(dir.join("pipe.conf").as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `pipe` is a valid NUL-terminated path for the call's duration.
    assert_eq!(unsafe { libc::mkfifo(pipe.as_ptr(), 0o600) }, 0);
    assert_eq!(
        title_under(roots, "pipe.conf"),
        None,
        "no reader blocks on it"
    );
    let late = format!("{}\ntitle Late\n", "#".repeat(64 * 1024));
    std::fs::write(dir.join("late.conf"), late).unwrap();
    assert_eq!(title_under(roots, "late.conf"), None);
    std::fs::write(dir.join("early.conf"), "title Early\n").unwrap();
    assert_eq!(title_under(roots, "early.conf"), Some("Early".into()));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn firmware_setup_sits_under_a_separator_and_rows_map_past_it() {
    let targets = vec![
        entry("a.conf", "A"),
        entry("b.conf", "B"),
        BootTarget::firmware(false),
    ];
    let items = menu_items(&targets);
    assert_eq!(items.len(), 4);
    assert!(matches!(items[2], MenuItem::Separator));
    assert_eq!(target_at(&targets, 1), Some(targets[1].clone()));
    assert_eq!(target_at(&targets, 2), None, "the separator");
    assert_eq!(target_at(&targets, 3), Some(targets[2].clone()));
    let firmware_only = vec![BootTarget::firmware(false)];
    assert_eq!(
        menu_items(&firmware_only).len(),
        1,
        "no separator above the first row"
    );
    assert_eq!(target_at(&firmware_only, 0), Some(firmware_only[0].clone()));
}

#[test]
fn logind_offers_its_entries_then_firmware_setup() {
    let Some(f) = logind("yes", "challenge", &["linux.conf", "auto-poweroff"]) else {
        return;
    };
    let got = zbus::block_on(offer(&f.client)).unwrap();
    assert_eq!(
        got,
        vec![entry("linux.conf", "Linux"), BootTarget::firmware(true)]
    );
}

#[test]
fn an_unanswered_ask_offers_no_answer_and_the_panel_keeps_its_targets() {
    let Some(bus) = bus() else {
        return;
    };
    let client = zbus::block_on(connect(&bus.1));
    assert!(
        zbus::block_on(offer(&client)).is_err(),
        "no logind on the bus"
    );
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    answered(&mut h.state, vec![entry("a.conf", "A")]);
    let d = h.state.turn_off_dialog();
    h.state.open_dialog(d);
    assert!(top_offers(&h), "the system bus does not answer the tests");
    assert_eq!(h.state.boot_targets, vec![entry("a.conf", "A")]);
}

#[test]
fn logind_reports_the_targets_it_holds_whoever_set_them() {
    let Some(f) = logind("yes", "yes", &[]) else {
        return;
    };
    assert_eq!(zbus::block_on(aimed(&f.client)), Vec::new());
    f.hold("memtest86+.conf", true);
    assert_eq!(
        zbus::block_on(aimed(&f.client)),
        vec![
            Aimed::Set(entry("memtest86+.conf", "Memtest86+")),
            Aimed::Set(BootTarget::firmware(false))
        ]
    );
}

#[test]
fn a_target_logind_cannot_read_is_unknown_not_unset() {
    let Some(f) = logind("yes", "yes", &[]) else {
        return;
    };
    f.unreadable();
    assert_eq!(
        zbus::block_on(aimed(&f.client)),
        [Aimed::Unknown(Kind::Entry), Aimed::Unknown(Kind::Firmware)]
    );
}

#[test]
fn nothing_is_offered_where_logind_cannot_set_a_target() {
    let Some(f) = logind("na", "no", &["linux.conf"]) else {
        return;
    };
    assert_eq!(zbus::block_on(offer(&f.client)).unwrap(), Vec::new());
}

#[test]
fn aiming_sets_the_entry_or_firmware_setup_and_clears_it() {
    let Some(f) = logind("yes", "yes", &[]) else {
        return;
    };
    let linux = entry("linux.conf", "Linux");
    let firmware = BootTarget::firmware(false);
    zbus::block_on(async {
        aim(&f.client, &linux, false).await.unwrap();
        unaim(&f.client, Kind::Entry, false).await.unwrap();
        aim(&f.client, &firmware, true).await.unwrap();
        unaim(&f.client, Kind::Firmware, false).await.unwrap();
    });
    assert_eq!(
        *f.set.lock().unwrap(),
        [
            "entry \"linux.conf\"",
            "entry \"\"",
            "firmware true",
            "firmware false"
        ]
    );
}

fn top_offers(h: &crate::harness::Harness) -> bool {
    (h.state.dialogs.views.last()).is_some_and(|v| {
        (v.dialog.buttons.iter()).any(|b| b.action == crate::dialog::DialogAction::RestartInto)
    })
}

#[test]
fn the_turn_off_panel_offers_restart_into_only_with_somewhere_to_go() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let d = h.state.turn_off_dialog();
    h.state.open_dialog(d);
    assert!(!top_offers(&h), "logind offered nothing");
    answered(&mut h.state, vec![entry("a.conf", "A")]);
    assert!(top_offers(&h), "the open panel takes the answer");
    answered(&mut h.state, Vec::new());
    assert!(!top_offers(&h));
    let mut log_off = crate::dialog::Dialog::log_off(reprisekit::powerpanel::Leave::Disconnect);
    assert!(
        !log_off.offer_restart_into(true),
        "Log Off restarts nothing"
    );
    let mut greeter = crate::dialog::Dialog::greeter_turn_off();
    assert!(
        !greeter.offer_restart_into(true),
        "the login screen's restart is reprise-login's"
    );
}

#[test]
fn restart_into_lists_the_targets_and_a_pick_restarts_into_one() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let d = h.state.turn_off_dialog();
    h.state.open_dialog(d);
    answered(
        &mut h.state,
        vec![entry("a.conf", "A"), BootTarget::firmware(false)],
    );
    let top = &mut h.state.dialogs.views.last_mut().unwrap().dialog;
    let out = top.key(reprisekit::controls::form::Key::Access('i'));
    assert_eq!(out, crate::dialog::Outcome::RestartInto { keys: true });
    h.state.apply_dialog(out);
    let m = h.state.menus.first().expect("the targets' menu");
    assert!(matches!(&m.kind, MenuKind::RestartInto(t) if *t == h.state.boot_targets));
    assert_eq!(m.items.len(), 3, "A, a separator, Firmware Setup");
    let button = {
        let v = h.state.dialogs.views.last().unwrap();
        let i = (v.dialog.buttons.iter())
            .position(|b| b.action == crate::dialog::DialogAction::RestartInto)
            .unwrap();
        let r = v.layout.buttons[i];
        (v.x + r.x, v.y + r.bottom())
    };
    assert_eq!((m.x, m.y), button, "the menu drops from the button");
    h.state.config.power.reboot = "true".into();
    h.state.menu_activate(0, 2);
    assert!(h.state.menus.is_empty());
    assert!(!top_offers(&h), "the panel closed");
    let e = h.state.ending.as_ref().expect("the restart under way");
    assert_eq!(e.goal, reprisekit::controls::endsession::Goal::Restart);
}

#[test]
fn the_shut_down_options_open_restart_into_as_a_submenu() {
    let (items, rows) = crate::startmenu::shutdown_menu(
        false,
        reprisekit::powerpanel::Leave::SwitchUser { live: true },
        true,
    );
    let at = (rows.iter())
        .position(|r| *r == Some(crate::startmenu::StartRow::RestartInto))
        .expect("Restart into");
    assert_eq!(
        rows[at - 1],
        Some(crate::startmenu::StartRow::Restart),
        "under Restart"
    );
    assert!(matches!(items[at], MenuItem::Entry { submenu: true, .. }));
}
