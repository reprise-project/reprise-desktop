use super::*;
use crate::harness::Harness;
use crate::input::BTN_LEFT;
use reprisekit::interface::Interface;
use reprisekit::logon::Part;

/// Locked under `interface` on TEST-1, no fingerprint conversation, `alone` answering.
fn locked_in(interface: &str, alone: fn() -> bool) -> Harness {
    let config = format!("[appearance]\ninterface = \"{interface}\"\n");
    let mut h = Harness::with_config(crate::config::Config::parse(&config).unwrap());
    h.output("TEST-1", 1024, 768);
    h.state.lock.helpers.finger = |_| None;
    h.state.lock.helpers.alone = alone;
    lockview::lock(&mut h.state);
    h
}

fn locked(alone: fn() -> bool) -> Harness {
    locked_in("classic", alone)
}

fn part(h: &Harness, part: Part) -> Option<smithay::utils::Point<f64, smithay::utils::Logical>> {
    h.state.lock.console.as_ref()?.part(part)
}

fn press(h: &mut Harness, p: Part) {
    let at = part(h, p).unwrap_or_else(|| panic!("{p:?} shows"));
    lockview::button(&mut h.state, Side::Console, BTN_LEFT, at);
}

/// Turn the loop until the power menu is open (or 5 s pass), then its rows.
fn menu_rows(h: &mut Harness) -> Vec<LockPower> {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !menu_open(&h.state) {
        assert!(std::time::Instant::now() < end, "the power menu opens");
        h.run_for(5);
    }
    match &h.state.menus[0].kind {
        MenuKind::LockPower(rows) => rows.clone(),
        _ => unreachable!(),
    }
}

/// An empty scratch folder of its own for test `name`.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("reprise-lockpower-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Turn the loop until `done` holds (or 5 s pass).
fn until(h: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done(h) {
        assert!(std::time::Instant::now() < end, "{what}");
        h.run_for(5);
    }
}

fn message(h: &Harness) -> Option<String> {
    h.state.lock.console.as_ref()?.message()
}

fn ending(h: &Harness) -> bool {
    h.state.session_fade.fading_out() || h.state.ending.is_some()
}

#[test]
fn the_lock_offers_restart_and_turn_off_only_while_the_session_is_alone() {
    use LockPower::*;
    let mut h = locked(|| true);
    press(&mut h, Part::Power);
    assert_eq!(menu_rows(&mut h), [StandBy, Restart, TurnOff]);
    let mut h = locked(|| false);
    press(&mut h, Part::Power);
    assert_eq!(menu_rows(&mut h), [StandBy], "another session or autologin");
}

#[test]
fn a_turn_off_picked_after_someone_else_logged_on_ends_nothing() {
    let mut h = locked(|| true);
    press(&mut h, Part::Power);
    menu_rows(&mut h);
    h.state.lock.helpers.alone = || false;
    for p in [LockPower::Restart, LockPower::TurnOff] {
        pick(&mut h.state, p);
        until(&mut h, "the lock says why", |h| message(h).is_some());
        assert_eq!(message(&h), Some(logon::text::unlock_to_power()));
        assert!(!ending(&h), "{p:?} ends nothing");
    }
}

#[test]
fn a_turn_off_picked_while_alone_ends_the_session() {
    let mut h = locked(|| true);
    h.state.config.power.poweroff = "true".into();
    pick(&mut h.state, LockPower::TurnOff);
    until(&mut h, "the session ends", ending);
}

#[test]
fn the_glass_locks_power_button_opens_the_menu() {
    let mut h = locked_in("glass", || true);
    assert_eq!(h.state.config.interface, Interface::Glass);
    press(&mut h, Part::Power);
    assert_eq!(menu_rows(&mut h).len(), 3);
    assert!(!ending(&h), "nothing ends on one press");
}

#[test]
fn a_key_on_the_open_menu_picks_its_row() {
    const KEY_ENTER: u32 = 28;
    const KEY_DOWN: u32 = 108;
    let slept = scratch("key").join("slept");
    let mut h = locked(|| true);
    h.state.config.power.standby = format!("touch {}", slept.display());
    press(&mut h, Part::Power);
    menu_rows(&mut h);
    for code in [KEY_DOWN, KEY_ENTER] {
        h.key(code, true);
        h.key(code, false);
    }
    until(&mut h, "Stand By runs", |_| slept.exists());
    assert!(!menu_open(&h.state));
}

#[test]
fn lock_mode_closes_the_power_menu_and_refuses_its_rows() {
    let mut h = Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.state.lock.helpers.finger = |_| None;
    h.state.lock.helpers.alone = || true;
    lockview::lock(&mut h.state);
    press(&mut h, Part::Power);
    menu_rows(&mut h);
    let mut portal = h.connect_portal();
    let lock = portal.lock_screen();
    lock.set_layout(&[(0, 0, 1280, 720, 100)]);
    h.roundtrip(&mut portal);
    assert!(crate::console::remote_side(&h.state));
    assert!(!menu_open(&h.state), "the menu closes");
    assert_eq!(part(&h, Part::Power), None, "the button goes");
    pick(&mut h.state, LockPower::TurnOff);
    h.run_for(20);
    assert!(!ending(&h));
}

#[test]
fn a_locker_taking_over_closes_the_power_menu() {
    let mut h = locked(|| true);
    press(&mut h, Part::Power);
    menu_rows(&mut h);
    let mut c = h.connect();
    let (_lock, _surface) = h.map_lock_surface(&mut c);
    assert!(!menu_open(&h.state));
}

#[test]
fn a_remote_viewer_driving_the_seat_reaches_no_power() {
    let mut h = locked(|| true);
    let _peer = h.ei_remote_session();
    assert!(h.state.remote_controlled());
    lockview::redraw(&mut h.state);
    assert_eq!(part(&h, Part::Power), None);
    open(&mut h.state, Side::Console);
    h.run_for(20);
    assert!(!menu_open(&h.state));
    pick(&mut h.state, LockPower::TurnOff);
    h.run_for(20);
    assert!(!ending(&h));
}

#[test]
fn neither_lock_in_lock_mode_draws_a_power_button() {
    let power = |h: &Harness, side| {
        let l = h.state.lock.on(side).expect("locked");
        l.part(Part::Power).or(l.part(Part::PowerMenu))
    };
    let h = locked(|| true);
    assert!(power(&h, Side::Console).is_some(), "outside lock mode");
    let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
    lockview::lock_remote(&mut h.state);
    assert_eq!(power(&h, Side::Remote), None, "the remote side's lock");
    assert_eq!(power(&h, Side::Console), None, "the computer's own lock");
}

#[test]
fn the_power_menu_opens_over_the_lock_and_closes_with_it() {
    let mut h = locked(|| true);
    press(&mut h, Part::Power);
    menu_rows(&mut h);
    assert_eq!(h.state.menus.len(), 1);
    assert!(
        crate::console::tests::under_lock_layer(&h, h.state.menus[0].tree.into()),
        "above the covers"
    );
    lockview::unlock(&mut h.state);
    assert!(h.state.menus.is_empty());
}

#[test]
fn greeters_lock_screens_and_service_managers_hold_no_work() {
    for class in [
        "user",
        "user-early",
        "user-incomplete",
        "background",
        "background-light",
    ] {
        assert!(holds_work(class), "{class}");
    }
    for class in ["greeter", "lock-screen", "manager", "manager-early", "none"] {
        assert!(!holds_work(class), "{class}");
    }
}

#[test]
fn an_autologin_user_or_an_unreadable_login_toml_keeps_restart_off() {
    let dir = scratch("autologin");
    let path = dir.join("login.toml");
    assert!(!autologin(&path), "no login.toml");
    std::fs::write(&path, "[login]\nshow_users = true\n").unwrap();
    assert!(!autologin(&path));
    std::fs::write(&path, "[login]\nautologin_user = \"anna\"\n").unwrap();
    assert!(autologin(&path));
    std::fs::create_dir(dir.join("dir")).unwrap();
    assert!(autologin(&dir.join("dir")), "a read that fails");
}
