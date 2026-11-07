use super::*;
use crate::compositor::Host;
use crate::config::{Action, Keybind};
use crate::harness::Harness;
use crate::keys::{KEY_Delete, KEY_r, MOD_ALT, MOD_CTRL};
use repriselogin::greeter::ErrorKind;

const RIGHT: u32 = 0x111;
const LEFTMETA: u32 = 125;
const LEFTCTRL: u32 = 29;
const LEFTALT: u32 = 56;
const KEY_R: u32 = 19;
const ENTER: u32 = 28;
const DELETE: u32 = 111;

fn greeter() -> Harness {
    let mut config = crate::config::Config {
        base: repriseconf::config::Config {
            terminal: "true".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    config.keybinds.push(Keybind {
        mods: MOD_CTRL | MOD_ALT,
        keysym: KEY_Delete,
        action: Action::Run,
    });
    let mut h = Harness::with_config(config);
    h.state.host = Host::Greeter;
    h.output("TEST-1", 1024, 768);
    h
}

fn chord(h: &mut Harness, keys: &[u32]) {
    for &k in keys {
        h.key(k, true);
    }
    for &k in keys.iter().rev() {
        h.key(k, false);
    }
}

#[test]
fn keybinds_never_fire_in_the_greeter() {
    let mut h = greeter();
    chord(&mut h, &[LEFTMETA]);
    chord(&mut h, &[LEFTMETA, KEY_R]);
    chord(&mut h, &[LEFTMETA, ENTER]);
    chord(&mut h, &[LEFTCTRL, LEFTALT, DELETE]);
    assert!(h.state.menus.is_empty(), "no Start menu");
    assert!(h.state.start_panel.is_none(), "no Start panel");
    assert!(h.state.dialogs.views.is_empty(), "no Run dialog");
    assert!(h.state.launcher.is_none(), "no launcher");
    let keys = h
        .state
        .greeter
        .heard
        .iter()
        .filter(|e| matches!(e, Heard::Key(_)))
        .count();
    assert_eq!(keys, 8, "the login screen hears every press");
    assert!(
        h.state.greeter.heard.contains(&Heard::Key(vec![KEY_r])),
        "Super+R reaches the login screen as r"
    );
}

#[test]
fn a_right_press_goes_to_the_login_screen_not_the_desktop() {
    let mut h = greeter();
    h.pointer_to(500.0, 400.0);
    h.button(RIGHT, true);
    h.button(RIGHT, false);
    assert_eq!(h.state.greeter.heard, vec![Heard::Button(RIGHT)]);
    assert!(h.state.menus.is_empty(), "no desktop menu");
}

const KEY_A: u32 = 30;
const KEY_N: u32 = 49;
const KEY_P: u32 = 25;
const KEY_W: u32 = 17;
const KEY_4: u32 = 5;
const KEY_2: u32 = 3;
const BACKSPACE: u32 = 14;
const CAPSLOCK: u32 = 58;
const POWER: u32 = 116;
const DOWN: u32 = 108;

/// reprise-login's end of the greeter link.
struct Daemon(std::os::unix::net::UnixStream);

impl Daemon {
    fn heard(&mut self) -> repriselogin::greeter::GreeterRequest {
        repriselogin::frame::read_msg(&mut self.0).expect("a request from the greeter")
    }

    fn says(&mut self, h: &mut Harness, reply: GreeterReply) {
        repriselogin::frame::write_msg(&mut self.0, &reply).unwrap();
        h.run_for(30);
    }
}

fn user(name: &str) -> repriselogin::greeter::UserEntry {
    repriselogin::greeter::UserEntry {
        name: name.into(),
        real_name: String::new(),
        picture: None,
        note: None,
    }
}

/// A greeter in `interface` with `users`, linked to a fake reprise-login.
fn login_with(
    interface: repriselogin::Interface,
    users: Vec<repriselogin::greeter::UserEntry>,
    tune: impl FnOnce(&mut crate::config::Config),
) -> (Harness, Daemon) {
    let greeting = Greeting {
        interface,
        show_users: true,
        users,
        last_user: None,
        keyboard: Default::default(),
        notice: None,
    };
    let mut config = crate::greeter::config_from(&greeting);
    // Glass's wallpaper draws on a thread into the cache; the screen is what these tests read.
    config.wallpaper.image = None;
    tune(&mut config);
    let mut h = Harness::with_config(config);
    h.state.host = Host::Greeter;
    h.output("TEST-1", 1024, 768);
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    theirs
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let link = crate::greeterlink::GreeterLink::from_stream(ours);
    h.state.greeter_link = Some(link.tx().unwrap());
    link.register(&h.state.handle).unwrap();
    setup(&mut h.state, greeting);
    (h, Daemon(theirs))
}

fn login(interface: repriselogin::Interface) -> (Harness, Daemon) {
    login_with(interface, Vec::new(), |_| {})
}

fn model(h: &Harness) -> &Logon {
    &h.state
        .greeter
        .login
        .as_ref()
        .expect("the login screen")
        .model
}

fn tap(h: &mut Harness, keys: &[u32]) {
    for &k in keys {
        h.key(k, true);
        h.key(k, false);
    }
}

fn click_part(h: &mut Harness, part: Part) {
    let g = h.state.greeter.login.as_ref().unwrap();
    let r = g
        .layout
        .as_ref()
        .unwrap()
        .rect(part)
        .expect("the part is laid out");
    h.click((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
}

fn secret(text: &str) -> GreeterReply {
    GreeterReply::AuthMessage {
        kind: repriselogin::greeter::AuthKind::Secret,
        text: text.into(),
    }
}

fn answered(text: &str) -> repriselogin::greeter::GreeterRequest {
    repriselogin::greeter::GreeterRequest::PostAuthResponse {
        response: Some(Zeroizing::new(text.into())),
    }
}

/// Classic with "anna" and "pw" typed and sent, the password prompt answered.
fn logged_in() -> (Harness, Daemon) {
    let (mut h, mut d) = login(repriselogin::Interface::Classic);
    tap(
        &mut h,
        &[KEY_A, KEY_N, KEY_N, KEY_A, ENTER, KEY_P, KEY_W, ENTER],
    );
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::CreateSession {
            user: "anna".into()
        }
    );
    d.says(&mut h, secret("Password:"));
    assert_eq!(d.heard(), answered("pw"));
    (h, d)
}

#[test]
fn typing_a_name_and_password_and_enter_sends_create_session() {
    let (h, _d) = logged_in();
    assert!(model(&h).busy, "the screen says Verifying...");
    assert_eq!(
        model(&h).password.text,
        "",
        "the sent password leaves the field"
    );
}

#[test]
fn a_later_prompt_is_answered_from_the_field() {
    let (mut h, mut d) = logged_in();
    d.says(&mut h, secret("Token:"));
    assert_eq!(
        model(&h).prompt.as_ref().map(|p| p.text.as_str()),
        Some("Token:")
    );
    assert!(!model(&h).busy);
    tap(&mut h, &[KEY_4, KEY_2, ENTER]);
    assert_eq!(d.heard(), answered("42"));
}

#[test]
fn the_greeter_stops_after_start_session_succeeds() {
    let (mut h, mut d) = logged_in();
    d.says(&mut h, GreeterReply::Success);
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::StartSession
    );
    assert!(!h.state.greeter.login.as_ref().unwrap().started);
    d.says(&mut h, GreeterReply::Success);
    assert!(h.state.greeter.login.as_ref().unwrap().started);
    assert!(!h.state.greeter.lost, "the greeter exits 0");
}

#[test]
fn an_incorrect_password_says_so_and_empties_the_field() {
    let (mut h, mut d) = logged_in();
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Auth,
            text: "Authentication failure".into(),
        },
    );
    let m = model(&h);
    assert_eq!(
        m.message.as_deref(),
        Some(logon::text::incorrect().as_str())
    );
    assert!(m.password.text.is_empty());
    assert!(!m.busy);
    assert_eq!(m.name.text, "anna");
}

#[test]
fn a_lockout_note_stays_on_screen_after_the_failure() {
    let (mut h, mut d) = logged_in();
    let locked = "The account is locked due to 3 failed logins.";
    d.says(
        &mut h,
        GreeterReply::AuthMessage {
            kind: repriselogin::greeter::AuthKind::Error,
            text: locked.into(),
        },
    );
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::PostAuthResponse { response: None }
    );
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Auth,
            text: "Authentication failure".into(),
        },
    );
    assert_eq!(model(&h).message.as_deref(), Some(locked));
    assert!(!model(&h).busy);
}

#[test]
fn an_account_refused_after_its_password_shows_pams_text() {
    let (mut h, mut d) = logged_in();
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Account,
            text: "User account has expired".into(),
        },
    );
    assert_eq!(
        model(&h).message.as_deref(),
        Some("User account has expired")
    );
    assert!(model(&h).password.text.is_empty());
}

#[test]
fn replies_once_the_link_is_lost_change_nothing() {
    let (mut h, mut d) = logged_in();
    d.0.shutdown(std::net::Shutdown::Read).unwrap();
    let note = GreeterReply::AuthMessage {
        kind: repriselogin::greeter::AuthKind::Info,
        text: "Password expires in 3 days".into(),
    };
    let error = GreeterReply::Error {
        kind: ErrorKind::Auth,
        text: "Authentication failure".into(),
    };
    repriselogin::frame::write_msg(&mut d.0, &note).unwrap();
    repriselogin::frame::write_msg(&mut d.0, &error).unwrap();
    h.run_for(30);
    assert!(
        h.state.greeter.lost,
        "the note's acknowledgement could not be sent"
    );
    assert!(model(&h).busy, "still Verifying...");
    assert_eq!(
        model(&h).message.as_deref(),
        Some("Password expires in 3 days"),
        "the error after the loss is not shown"
    );
}

fn turned_off() -> (Harness, Daemon) {
    let (mut h, mut d) = login(repriselogin::Interface::Classic);
    click_part(&mut h, Part::Power);
    assert_eq!(h.state.dialogs.views.len(), 1, "the Turn Off dialog");
    tap(&mut h, &[ENTER]);
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::Power {
            action: PowerAction::PowerOff
        }
    );
    assert!(h.state.dialogs.views.is_empty());
    (h, d)
}

#[test]
fn turn_off_from_the_dialog_sends_power_off() {
    turned_off();
}

#[test]
fn a_refused_power_shows_why() {
    let (mut h, mut d) = turned_off();
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Refused,
            text: "other users are logged on".into(),
        },
    );
    assert_eq!(
        model(&h).message.as_deref(),
        Some(logon::text::power_refused().as_str())
    );
}

#[test]
fn a_failed_power_shows_a_fixed_message_and_not_loginds_text() {
    let (mut h, mut d) = turned_off();
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Failed,
            text: "org.freedesktop.DBus.Error.AccessDenied: Permission denied".into(),
        },
    );
    assert_eq!(
        model(&h).message.as_deref(),
        Some(logon::text::power_failed().as_str())
    );
}

#[test]
fn presses_beside_the_turn_off_dialog_do_nothing() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    click_part(&mut h, Part::Power);
    assert_eq!(h.state.dialogs.views.len(), 1, "the Turn Off dialog");
    let parts = (h.state.greeter.login.as_ref().unwrap().layout.as_ref())
        .unwrap()
        .parts
        .clone();
    let (x, y) = (parts.iter())
        .map(|(_, r)| ((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64))
        .find(|&at| {
            !matches!(
                h.state.scene_hit(at.into()),
                crate::input::Hit::Dialog { .. }
            )
        })
        .expect("a part beside the dialog");
    let before = format!("{:?}", model(&h));
    h.click(x, y);
    assert_eq!(
        format!("{:?}", model(&h)),
        before,
        "the screen is untouched"
    );
    assert_eq!(h.state.dialogs.views.len(), 1, "only the Turn Off dialog");
}

#[test]
fn a_swipe_neither_opens_the_overview_nor_steps_desktops() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    let desktop = h.state.desktops.current;
    h.swipe(3, &[(0.0, -60.0); 6]);
    assert!(h.state.overview.view.is_none(), "no overview");
    h.swipe(3, &[(-60.0, 0.0); 6]);
    assert_eq!(h.state.desktops.current, desktop, "the same desktop");
}

#[test]
fn the_lid_is_left_to_logind() {
    use smithay::backend::input::{Switch, SwitchState};
    let (mut h, _d) = login_with(repriselogin::Interface::Classic, Vec::new(), |c| {
        c.power.lid_close = crate::config::PowerAction::Ask;
    });
    crate::power::switch_toggled(&mut h.state, Some(Switch::Lid), SwitchState::On);
    assert!(!h.state.power.lid_closed, "the greeter keeps no lid state");
    assert!(h.state.dialogs.views.is_empty(), "no Turn Off dialog");
}

#[test]
fn the_power_key_opens_turn_off() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    tap(&mut h, &[POWER]);
    assert_eq!(h.state.dialogs.views.len(), 1, "the Turn Off dialog");
}

#[test]
fn glass_power_menu_restarts() {
    let (mut h, mut d) = login_with(repriselogin::Interface::Glass, vec![user("anna")], |_| {});
    click_part(&mut h, Part::PowerMenu);
    assert_eq!(h.state.menus.len(), 1, "the power menu");
    tap(&mut h, &[DOWN, ENTER]);
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::Power {
            action: PowerAction::Reboot
        }
    );
}

#[test]
fn caps_lock_shows_its_notice() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    assert!(!model(&h).caps_lock);
    tap(&mut h, &[CAPSLOCK]);
    assert!(model(&h).caps_lock);
}

#[test]
fn a_held_backspace_repeats() {
    let (mut h, _d) = login_with(repriselogin::Interface::Classic, Vec::new(), |c| {
        c.keyboard.repeat_delay = 20;
        c.keyboard.repeat_rate = 100;
    });
    tap(&mut h, &[KEY_A, KEY_N, KEY_N, KEY_A]);
    assert_eq!(model(&h).name.text, "anna");
    h.key(BACKSPACE, true);
    h.run_for(150);
    h.key(BACKSPACE, false);
    assert!(model(&h).name.text.is_empty(), "{:?}", model(&h).name.text);
}

#[test]
fn the_wheel_over_the_tiles_steps_through_them() {
    use smithay::backend::input::{Axis, AxisSource};
    let users = vec![user("anna"), user("bob"), user("carol")];
    let (mut h, _d) = login_with(repriselogin::Interface::Blue, users, |_| {});
    assert_eq!(model(&h).focus, Part::Tile(0));
    let r = (h.state.greeter.login.as_ref().unwrap().layout.as_ref())
        .and_then(|l| l.rect(Part::Tile(0)))
        .unwrap();
    h.pointer_to((r.x + 4) as f64, (r.y + 4) as f64);
    let down = AxisFrame::new(InputTime::now())
        .source(AxisSource::Wheel)
        .value(Axis::Vertical, 15.0)
        .v120(Axis::Vertical, 120);
    h.state.handle_axis(down);
    assert_eq!(model(&h).focus, Part::Tile(1));
}

#[test]
fn every_output_is_covered_and_only_the_first_has_the_screen() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    h.output("TEST-2", 800, 600);
    let g = h.state.greeter.login.as_ref().unwrap();
    assert_eq!(g.covers.len(), 2);
    for c in &g.covers {
        assert_eq!(
            h.state.scene.parent(c.tree),
            Some(h.state.layers.background),
            "under any dialog"
        );
    }
    let screens: Vec<String> = (g.covers.iter())
        .filter(|c| c.screen.is_some())
        .map(|c| c.output.name())
        .collect();
    assert_eq!(screens, vec!["TEST-1".to_string()]);
    let first = (h.state.scene.outputs().iter())
        .find(|o| o.name() == "TEST-1")
        .cloned();
    assert_eq!(
        g.layout.as_ref().unwrap().output,
        h.state.layout_box(first.as_ref())
    );
}

#[test]
fn the_login_screen_has_no_taskbar_and_the_whole_output_to_itself() {
    let (mut h, _d) = login(repriselogin::Interface::Classic);
    h.output("TEST-2", 800, 600);
    assert!(h.state.bars.is_empty(), "no taskbar on any output");
    for o in h.state.scene.outputs().to_vec() {
        assert_eq!(h.state.usable_area(Some(&o)), h.state.layout_box(Some(&o)));
    }
}

/// `interface`'s greeter on one `w` x `h` output at `scale`, three users with pictures.
fn shown_at(interface: repriselogin::Interface, w: i32, h: i32, scale: f64) -> (Harness, Daemon) {
    use reprisekit::art::userpic;
    let users = vec![user("anna"), user("bob"), user("carol")];
    let (mut hn, d) = login_with(interface, users, |_| {});
    let o = hn.state.scene.outputs()[0].clone();
    o.change_current_state(
        Some(smithay::output::Mode {
            size: (w, h).into(),
            refresh: 60_000,
        }),
        None,
        Some(smithay::output::Scale::Fractional(scale)),
        None,
    );
    hn.state.update_layout();
    let g = hn.state.greeter.login.as_mut().unwrap();
    for (i, u) in g.model.users.iter_mut().enumerate() {
        u.picture = Some(userpic::draw(userpic::ALL[i], userpic::SAVED_PX));
    }
    redraw(&mut hn.state);
    (hn, d)
}

/// The first output as the scene shows it: the cover's backdrop, then the screen over it.
fn capture(h: &Harness) -> Canvas {
    let g = h.state.greeter.login.as_ref().unwrap();
    let o = g.covers[0].output.clone();
    let b = h.state.layout_box(Some(&o));
    let s = h.state.draw_scale_at(Some(&o));
    let theme = h.state.config.theme.scaled(s);
    let (w, hh) = ((b.width * s) as u32, (b.height * s) as u32);
    let mut out = match h.state.config.interface {
        Interface::Glass => {
            let b = reprisekit::interfaces::default_wallpaper(Interface::Glass).unwrap();
            let b = reprisekit::interfaces::resolve_wallpaper(b, &h.state.config.glass);
            reprisekit::wallpapers::render(b, w, hh, reprisekit::wallpapers::SEED)
        }
        _ => {
            let mut c = Canvas::new(w, hh);
            let [r, g, bl, _] = cover_color(&h.state).unwrap().map(|v| (v * 255.0) as u8);
            c.fill_rect(c.bounds(), reprisekit::theme::Color::rgb(r, g, bl));
            c
        }
    };
    let (_, screen) = super::screen(&mut g.model.clone(), &theme, &h.state.text, b, s);
    out.blit_canvas(0, 0, &screen);
    out
}

/// Renders each interface's greeter, at rest and after a wrong password, at scales 1 and 2, to
/// `target/greeter-preview/`, and prints what one redraw costs. Run with
/// `cargo test render_greeter_preview -- --ignored --nocapture`.
#[test]
#[ignore]
fn render_greeter_preview() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/greeter-preview");
    std::fs::create_dir_all(&dir).unwrap();
    for (interface, name) in [
        (repriselogin::Interface::Classic, "classic"),
        (repriselogin::Interface::Blue, "blue"),
        (repriselogin::Interface::Glass, "glass"),
    ] {
        for scale in [1, 2] {
            let (mut h, mut d) = shown_at(interface, 1280 * scale, 800 * scale, scale as f64);
            if interface == repriselogin::Interface::Blue {
                tap(&mut h, &[DOWN, ENTER]);
            }
            if interface == repriselogin::Interface::Classic {
                tap(&mut h, &[KEY_A, KEY_N, KEY_N, KEY_A, ENTER]);
            }
            tap(&mut h, &[KEY_P, KEY_W]);
            let write = |h: &Harness, state: &str| {
                let path = dir.join(format!("{name}-{state}@{scale}.png"));
                std::fs::write(&path, reprisekit::raster::to_png(&capture(h))).unwrap();
                eprintln!("wrote {}", path.display());
            };
            write(&h, "typed");
            tap(&mut h, &[ENTER]);
            let _ = d.heard();
            d.says(
                &mut h,
                GreeterReply::Error {
                    kind: ErrorKind::Auth,
                    text: String::new(),
                },
            );
            write(&h, "incorrect");
        }
        let (mut h, _d) = shown_at(interface, 3840, 2160, 2.0);
        const N: u32 = 20;
        let t = std::time::Instant::now();
        for _ in 0..N {
            redraw(&mut h.state);
        }
        let full = t.elapsed() / N;
        let g = h.state.greeter.login.as_ref().unwrap();
        let o = g.covers[0].output.clone();
        let b = h.state.layout_box(Some(&o));
        let theme = h.state.config.theme.scaled(2);
        let mut m = g.model.clone();
        let paint = |m: &mut Logon| {
            let t = std::time::Instant::now();
            for _ in 0..N {
                super::screen(m, &theme, &h.state.text, b, 2);
            }
            t.elapsed() / N
        };
        let painting = paint(&mut m);
        for u in &mut m.users {
            u.picture = None;
        }
        eprintln!(
            "{name} 3840x2160 at scale 2: redraw {full:?}, of it painting {painting:?}, \
             {:?} without pictures",
            paint(&mut m)
        );
    }
}
