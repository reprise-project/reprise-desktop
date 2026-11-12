use super::*;
use crate::compositor::Host;
use crate::config::{Action, Keybind};
use crate::harness::Harness;
use crate::keys::{KEY_Delete, KEY_r, MOD_ALT, MOD_CTRL};
use reprisekit::logon::Part;
use reprisekit::raster::Canvas;
use repriselogin::greeter::ErrorKind;
use zeroize::Zeroizing;

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
    let mut h = Harness::with_host(config, Host::Greeter);
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
const LEFTSHIFT: u32 = 42;
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
        outputs: Vec::new(),
        background: None,
        animation: true,
    };
    let mut config = crate::greeter::config_from(&greeting);
    // Glass's wallpaper draws on a thread into the cache; the screen is what these tests read.
    config.wallpaper.image = None;
    tune(&mut config);
    let mut h = Harness::with_host(config, Host::Greeter);
    crate::logonview::hold_clock();
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
        .view
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
        .view
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
fn a_login_that_cannot_run_shows_a_fixed_text_and_not_the_daemons() {
    for (kind, text) in [
        (ErrorKind::Failed, "no free VT for a session"),
        (ErrorKind::Refused, "another login is in progress"),
    ] {
        let (mut h, mut d) = logged_in();
        d.says(
            &mut h,
            GreeterReply::Error {
                kind,
                text: text.into(),
            },
        );
        assert_eq!(
            model(&h).message.as_deref(),
            Some(logon::text::failed().as_str()),
            "{kind:?}"
        );
    }
}

#[test]
fn a_later_greeting_brings_the_tiles_notes_up_to_date() {
    let mut anna = user("anna");
    anna.note = Some(Note::LoggedOn);
    let (mut h, mut d) = login_with(
        repriselogin::Interface::Blue,
        vec![anna, user("bob")],
        |_| {},
    );
    assert_eq!(model(&h).users[0].note, Some(UserNote::LoggedOn));
    let fresh = Greeting {
        interface: repriselogin::Interface::Blue,
        show_users: true,
        users: vec![user("anna"), user("bob")],
        last_user: None,
        keyboard: Default::default(),
        notice: None,
        outputs: Vec::new(),
        background: None,
        animation: true,
    };
    d.says(&mut h, GreeterReply::Greeting(fresh));
    assert_eq!(model(&h).users[0].note, None, "anna logged off");
}

#[test]
fn a_greeting_brought_by_switch_user_opens_on_the_tiles_with_the_locked_tile_noted() {
    let mut anna = user("anna");
    anna.note = Some(Note::Locked);
    let users = vec![anna, user("bob")];
    let glass = repriselogin::Interface::Glass;
    let (mut h, mut d) = login_with(glass, users.clone(), |_| {});
    assert_eq!(model(&h).screen, logon::Screen::Tile(0));
    let switched = Greeting {
        interface: glass,
        show_users: true,
        users,
        last_user: Some("anna".into()),
        keyboard: Default::default(),
        notice: Some(Notice::SwitchUser),
        outputs: Vec::new(),
        background: None,
        animation: true,
    };
    d.says(&mut h, GreeterReply::Greeting(switched.clone()));
    assert_eq!(model(&h).screen, logon::Screen::Tiles, "the running screen");
    assert_eq!(model(&h).users[0].note, Some(UserNote::Locked));
    setup(&mut h.state, switched);
    assert_eq!(
        model(&h).screen,
        logon::Screen::Tiles,
        "a new screen, not the last user's tile"
    );
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
    let parts = (h.state.greeter.login.as_ref().unwrap().view.layout.as_ref())
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
    assert!(
        !h.state.power.lid_closed,
        "the greeter keeps no session lid state"
    );
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
    // The delay outlasts the test, so only the ticks driven here repeat.
    let (mut h, _d) = login_with(repriselogin::Interface::Classic, Vec::new(), |c| {
        c.keyboard.repeat_delay = 60_000;
    });
    tap(&mut h, &[KEY_A, KEY_N, KEY_N, KEY_A]);
    assert_eq!(model(&h).name.text, "anna");
    h.key(BACKSPACE, true);
    assert_eq!(model(&h).name.text, "ann", "the press");
    for left in ["an", "a", ""] {
        h.state.field_repeat_tick();
        assert_eq!(model(&h).name.text, left);
    }
    h.key(BACKSPACE, false);
    assert!(
        h.state.key_input.field_repeat.is_none(),
        "the release ends it"
    );
}

#[test]
fn the_wheel_over_the_tiles_steps_through_them() {
    use smithay::backend::input::{Axis, AxisSource};
    let users = vec![user("anna"), user("bob"), user("carol")];
    let (mut h, _d) = login_with(repriselogin::Interface::Blue, users, |_| {});
    assert_eq!(model(&h).focus, Part::Tile(0));
    let r = (h.state.greeter.login.as_ref().unwrap().view.layout.as_ref())
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
    assert_eq!(g.view.covers.len(), 2);
    for c in &g.view.covers {
        assert_eq!(
            h.state.scene.parent(c.tree),
            Some(h.state.layers.background),
            "under any dialog"
        );
    }
    let screens: Vec<String> = (g.view.covers.iter())
        .filter(|c| c.screen.is_some())
        .map(|c| c.output.name())
        .collect();
    assert_eq!(screens, vec!["TEST-1".to_string()]);
    let first = (h.state.scene.outputs().iter())
        .find(|o| o.name() == "TEST-1")
        .cloned();
    assert_eq!(
        g.view.layout.as_ref().unwrap().output,
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
    for (i, u) in g.view.model.users.iter_mut().enumerate() {
        u.picture = Some(userpic::draw(userpic::ALL[i], userpic::SAVED_PX));
    }
    redraw(&mut hn.state);
    (hn, d)
}

/// The first output as the scene shows it: the cover's backdrop, then the screen over it.
fn capture(h: &Harness) -> Canvas {
    let g = h.state.greeter.login.as_ref().unwrap();
    let o = g.view.covers[0].output.clone();
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
            let [r, g, bl, _] = crate::logonview::backdrop(&h.state)
                .unwrap()
                .map(|v| (v * 255.0) as u8);
            c.fill_rect(c.bounds(), reprisekit::theme::Color::rgb(r, g, bl));
            c
        }
    };
    let (_, screen) =
        crate::logonview::screen(&mut g.view.model.clone(), &theme, &h.state.text, b, s);
    out.blit_canvas(0, 0, &screen);
    out
}

/// The screen of `h`, a greeter from [`shown_at`], on its password field with the change to it
/// played out.
fn on_password(h: &mut Harness, interface: repriselogin::Interface) {
    match interface {
        repriselogin::Interface::Classic => tap(h, &[KEY_A, KEY_N, KEY_N, KEY_A, ENTER]),
        repriselogin::Interface::Blue => tap(h, &[DOWN, ENTER]),
        repriselogin::Interface::Glass => {}
    }
    if view(h).play.is_some() {
        settle(h);
    }
    assert_eq!(model(h).focus, Part::Password, "{interface:?}");
}

#[test]
fn a_key_in_the_password_field_repaints_it_alone_and_leaves_the_screen_a_whole_redraw_draws() {
    for interface in [
        repriselogin::Interface::Classic,
        repriselogin::Interface::Blue,
        repriselogin::Interface::Glass,
    ] {
        for scale in [1, 2] {
            let (mut h, _d) = shown_at(interface, 1280 * scale, 800 * scale, scale as f64);
            h.output("TEST-2", 1024, 768);
            on_password(&mut h, interface);
            tap(&mut h, &[KEY_P]);
            for key in [
                KEY_W, KEY_4, BACKSPACE, KEY_2, BACKSPACE, BACKSPACE, BACKSPACE,
            ] {
                crate::backend::take_asks();
                crate::logonview::DRAW_PANICS.set(true);
                tap(&mut h, &[key]);
                crate::logonview::DRAW_PANICS.set(false);
                assert_eq!(
                    crate::backend::take_scene_outputs(),
                    ["TEST-1"],
                    "{interface:?} at {scale}: only the screen's output is asked"
                );
                assert!(
                    shown(&mut h).pixels == at_rest(&mut h).pixels,
                    "{interface:?} at {scale}: the screen after key {key} is a whole redraw's"
                );
            }
        }
    }
}

#[test]
fn caps_lock_and_a_message_repaint_their_notes_alone_and_leave_the_screen_a_whole_redraw_draws() {
    type Step = (&'static str, fn(&mut Harness));
    let steps: [Step; 6] = [
        ("caps on", |h| tap(h, &[CAPSLOCK])),
        ("message", |h| {
            view(h).model.info(logon::text::incorrect());
            update(&mut h.state);
        }),
        ("caps off", |h| tap(h, &[CAPSLOCK])),
        ("message cleared", |h| {
            view(h).model.message = None;
            update(&mut h.state);
        }),
        ("hint", |h| {
            view(h).model.hint = Some("Place a finger on the reader".into());
            update(&mut h.state);
        }),
        ("hint cleared", |h| {
            view(h).model.hint = None;
            update(&mut h.state);
        }),
    ];
    for interface in [
        repriselogin::Interface::Classic,
        repriselogin::Interface::Blue,
        repriselogin::Interface::Glass,
    ] {
        for scale in [1, 2] {
            let (mut h, _d) = shown_at(interface, 1280 * scale, 800 * scale, scale as f64);
            h.output("TEST-2", 1024, 768);
            on_password(&mut h, interface);
            tap(&mut h, &[KEY_P]);
            for (step, change) in steps {
                let before = shown(&mut h);
                crate::backend::take_asks();
                crate::logonview::DRAW_PANICS.set(true);
                change(&mut h);
                crate::logonview::DRAW_PANICS.set(false);
                // Glass shows Caps Lock in place of the message, so a message under it changes nothing.
                let changed = shown(&mut h).pixels != before.pixels;
                assert_eq!(
                    crate::backend::take_scene_outputs(),
                    if changed { &["TEST-1"][..] } else { &[] },
                    "{interface:?} at {scale}: {step} asks only the screen's output, if it changed"
                );
                assert!(
                    shown(&mut h).pixels == at_rest(&mut h).pixels,
                    "{interface:?} at {scale}: the screen after {step} is a whole redraw's"
                );
            }
        }
    }
}

/// Prints what a change costs each greeter at 1920x1080 and at 3840x2160 at scale 2, with a
/// second output beside it: a key typed into the password field, Shift tapped (the input path a
/// Caps Lock tap takes, drawing nothing), Caps Lock toggled, and a message set and cleared. Per
/// change: the time, the device px it changes and the outputs it asks for a frame; and the time
/// of a whole redraw. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn typing_cost() {
    fn cost(h: &mut Harness, mut change: impl FnMut(&mut Harness, u32)) -> String {
        const N: u32 = 40;
        let (mut spent, mut changed, mut asked) = (std::time::Duration::ZERO, 0, 0);
        for i in 0..N {
            let before = shown(h);
            crate::backend::take_asks();
            let t = std::time::Instant::now();
            change(h, i);
            spent += t.elapsed();
            asked += crate::backend::take_scene_outputs().len();
            let r = before.differing_rect(&shown(h));
            changed += r.width as u64 * r.height as u64;
        }
        format!(
            "{:?}, {} device px, {:.1} outputs",
            spent / N,
            changed / u64::from(N),
            asked as f64 / f64::from(N),
        )
    }
    for interface in [
        repriselogin::Interface::Classic,
        repriselogin::Interface::Blue,
        repriselogin::Interface::Glass,
    ] {
        for (w, hh, scale) in [(1920, 1080, 1.0), (3840, 2160, 2.0)] {
            let (mut h, _d) = shown_at(interface, w, hh, scale);
            h.output("TEST-2", 1024, 768);
            on_password(&mut h, interface);
            tap(&mut h, &[KEY_P, KEY_W]);
            let whole = cost(&mut h, |h, _| redraw(&mut h.state));
            let key = cost(&mut h, |h, i| match i % 2 {
                0 => logonview::type_text(&mut h.state, &HOST, "p"),
                _ => logonview::repeat_backspace(&mut h.state, &HOST),
            });
            let shift = cost(&mut h, |h, _| tap(h, &[LEFTSHIFT]));
            let caps = cost(&mut h, |h, _| tap(h, &[CAPSLOCK]));
            let message = cost(&mut h, |h, i| {
                match i % 2 {
                    0 => view(h)
                        .model
                        .info("The login service did not answer.".into()),
                    _ => view(h).model.message = None,
                }
                logonview::update(&mut h.state, &HOST);
            });
            eprintln!(
                "{interface:?} {w}x{hh} at scale {scale}:\n  whole redraw {whole}\n  key {key}\n  shift {shift}\n  \
                 caps lock {caps}\n  message {message}"
            );
        }
    }
}

/// Renders each greeter at rest and after a wrong password, at scales 1 and 2, to
/// `target/greeter-preview/` and prints a redraw's cost. Run with `--ignored --nocapture`.
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
        let o = g.view.covers[0].output.clone();
        let b = h.state.layout_box(Some(&o));
        let theme = h.state.config.theme.scaled(2);
        let mut m = g.view.model.clone();
        let paint = |m: &mut Logon| {
            let t = std::time::Instant::now();
            for _ in 0..N {
                crate::logonview::screen(m, &theme, &h.state.text, b, 2);
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

#[test]
fn the_screen_goes_on_the_greetings_primary_output_while_it_is_connected() {
    let primary = |name: &str| crate::config::OutputConfig {
        name: name.into(),
        scale: 1.0,
        mode: None,
        adaptive_sync: None,
        transform: crate::config::Transform::Normal,
        position: None,
        primary: true,
    };
    for (named, on) in [("TEST-2", "TEST-2"), ("HDMI-A-9", "TEST-1")] {
        let (mut h, _d) = login_with(repriselogin::Interface::Classic, Vec::new(), |c| {
            c.outputs = vec![primary(named)]
        });
        h.output("TEST-2", 800, 600);
        let g = h.state.greeter.login.as_ref().unwrap();
        let screens: Vec<String> = (g.view.covers.iter())
            .filter(|c| c.screen.is_some())
            .map(|c| c.output.name())
            .collect();
        assert_eq!(screens, [on], "{named} primary");
        let o = (h.state.scene.outputs().iter()).find(|o| o.name() == on);
        assert_eq!(
            g.view.layout.as_ref().unwrap().output,
            h.state.layout_box(o),
            "{named} primary"
        );
    }
}

#[test]
fn a_background_that_does_not_load_gives_back_the_interfaces_own_look() {
    let junk = crate::harness::scratch().join("not-a-picture.png");
    std::fs::write(&junk, b"not a picture").unwrap();
    for interface in [
        repriselogin::Interface::Classic,
        repriselogin::Interface::Glass,
    ] {
        let greeting = Greeting {
            interface,
            show_users: true,
            users: Vec::new(),
            last_user: None,
            keyboard: Default::default(),
            notice: None,
            outputs: Vec::new(),
            background: Some(repriselogin::machine::Background::Picture),
            animation: true,
        };
        let mut config = crate::greeter::config_from(&greeting);
        config.wallpaper.image = Some(junk.clone());
        let mut h = Harness::with_host(config, Host::Greeter);
        h.output("TEST-1", 1024, 768);
        setup(&mut h.state, greeting);
        h.state.apply_wallpaper_config();
        for _ in 0..100 {
            if !model(&h).over_picture {
                break;
            }
            h.run_for(20);
        }
        assert!(!model(&h).over_picture, "{interface:?}");
        let default = crate::greeter::default_wallpaper(crate::greeter::interface(interface));
        assert_eq!(h.state.config.wallpaper.image, default, "{interface:?}");
        let g = h.state.greeter.login.as_ref().unwrap();
        let rect = g.view.covers[0].rect;
        assert_eq!(
            h.state.scene.enabled(rect),
            interface == repriselogin::Interface::Classic,
            "Classic's desktop colour is back; Glass lies over its wallpaper"
        );
    }
}

/// A Glass greeter on its two users' tiles, at rest; `tune` adjusts the config.
fn glass_tiles(tune: impl FnOnce(&mut crate::config::Config)) -> (Harness, Daemon) {
    let users = vec![user("anna"), user("bob")];
    let (mut h, d) = login_with(repriselogin::Interface::Glass, users, tune);
    click_part(&mut h, Part::SwitchUser);
    if view(&mut h).play.is_some() {
        settle(&mut h);
    }
    assert_eq!(model(&h).screen, logon::Screen::Tiles);
    (h, d)
}

fn view(h: &mut Harness) -> &mut crate::logonview::LogonView {
    &mut h
        .state
        .greeter
        .login
        .as_mut()
        .expect("the login screen")
        .view
}

/// The asks made by the frame about to render on the screen's output, what plays on it begun
/// `ago` ms before when given.
fn next_frame(h: &mut Harness, ago: Option<u64>) -> Vec<String> {
    if let (Some(ms), Some(p)) = (ago, view(h).play.as_mut()) {
        p.since = Some(crate::logonview::now() - std::time::Duration::from_millis(ms));
    }
    let o = h.state.scene.outputs()[0].clone();
    crate::backend::take_asks();
    crate::backend::advance(&mut h.state, &o);
    let asks = crate::backend::take_asks();
    asks.into_iter().map(|(o, _)| o).collect()
}

/// Ms the change playing takes.
fn play_ms(h: &mut Harness) -> u64 {
    u64::from(view(h).play.as_ref().expect("a change plays").t.ms())
}

/// Let the change playing run to its end.
fn settle(h: &mut Harness) {
    let ms = play_ms(h);
    next_frame(h, Some(ms + 1));
}

/// The screen at rest as its model draws it whole.
fn at_rest(h: &mut Harness) -> Canvas {
    let o = h.state.scene.outputs()[0].clone();
    let b = h.state.layout_box(Some(&o));
    let s = h.state.draw_scale_at(Some(&o));
    let theme = h.state.config.theme.scaled(s);
    let mut m = model(h).clone();
    crate::logonview::screen(&mut m, &theme, &h.state.text, b, s).1
}

fn shown(h: &mut Harness) -> Canvas {
    view(h).shown.clone().expect("the screen was drawn")
}

#[test]
fn a_settled_transition_asks_no_more_frames() {
    let (mut h, _d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    let p = view(&mut h).play.as_ref().expect("the pick plays");
    assert_eq!(p.t.change, logon::Change::Pick(Part::Tile(0)));
    assert!(p.since.is_none(), "the clock waits for the first frame");
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    assert!(view(&mut h).play.as_ref().unwrap().since.is_some());
    assert_eq!(next_frame(&mut h, Some(100)), ["TEST-1"], "mid-pick asks");
    assert!(
        shown(&mut h) != at_rest(&mut h),
        "mid-pick is between the screens"
    );
    let ms = play_ms(&mut h);
    next_frame(&mut h, Some(ms + 1));
    assert!(view(&mut h).play.is_none(), "the pick ended");
    assert!(
        shown(&mut h) == at_rest(&mut h),
        "the end is the screen at rest"
    );
    assert_eq!(next_frame(&mut h, None), Vec::<String>::new());
}

#[test]
fn the_ring_turns_while_busy_and_asks_nothing_once_the_failure_settles() {
    let (mut h, mut d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    settle(&mut h);
    tap(&mut h, &[KEY_P, KEY_W, ENTER]);
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::CreateSession {
            user: "anna".into()
        }
    );
    assert!(model(&h).busy);
    assert_eq!(
        view(&mut h).play.as_ref().map(|p| p.t.change),
        Some(logon::Change::Busy)
    );
    settle(&mut h);
    assert_eq!(
        next_frame(&mut h, None),
        ["TEST-1"],
        "the ring asks for its next step"
    );
    assert_eq!(model(&h).spin, 0, "the ring starts from step 0");
    crate::logonview::pass(70);
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    assert!(
        model(&h).spin >= 3,
        "the ring turned, at step {}",
        model(&h).spin
    );
    assert!(
        shown(&mut h) == at_rest(&mut h),
        "the turned ring is the one drawn whole"
    );
    d.says(
        &mut h,
        GreeterReply::Error {
            kind: ErrorKind::Auth,
            text: "Authentication failure".into(),
        },
    );
    assert!(!model(&h).busy);
    assert_eq!(
        view(&mut h).play.as_ref().map(|p| p.t.change),
        Some(logon::Change::Fail)
    );
    settle(&mut h);
    assert_eq!(next_frame(&mut h, None), Vec::<String>::new());
    assert_eq!(model(&h).spin, 0);
}

#[test]
fn the_ring_draws_and_asks_for_frames_only_on_its_steps() {
    let (mut h, mut d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    settle(&mut h);
    tap(&mut h, &[KEY_P, KEY_W, ENTER]);
    let _ = d.heard();
    settle(&mut h);
    let ms = std::time::Duration::from_millis;
    let since = crate::logonview::now() - ms(25);
    view(&mut h).ring = Some(since);
    crate::backend::take_steps();
    next_frame(&mut h, None);
    assert_eq!(model(&h).spin, 1);
    assert_eq!(
        crate::backend::take_steps(),
        [since + ms(40)],
        "the next step's frame"
    );
    let drawn = shown(&mut h);
    next_frame(&mut h, None);
    assert_eq!(
        crate::backend::take_steps(),
        [since + ms(40)],
        "a frame between steps asks for the step again"
    );
    assert!(shown(&mut h) == drawn, "and draws nothing");
}

#[test]
fn keys_typed_during_a_pick_reach_the_password_box() {
    let (mut h, _d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    next_frame(&mut h, Some(30));
    tap(&mut h, &[KEY_A, KEY_A, KEY_A]);
    assert_eq!(model(&h).password.text, "aaa");
    assert!(view(&mut h).play.is_some(), "the pick still plays");
    settle(&mut h);
    let rest = at_rest(&mut h);
    assert!(shown(&mut h) == rest, "three bullets in the box at rest");
}

#[test]
fn a_click_mid_pick_lands_on_the_screen_it_goes_to() {
    let (mut h, _d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    next_frame(&mut h, Some(120));
    let p = view(&mut h).play.as_ref().unwrap();
    let (from, to) = (p.t.carried_at(0), p.t.carried_at(p.t.ms()));
    click_part(&mut h, Part::SwitchUser);
    assert_eq!(model(&h).screen, logon::Screen::Tiles);
    let p = view(&mut h).play.as_ref().expect("the way back plays");
    assert_eq!(p.t.change, logon::Change::Back(Part::Tile(0)));
    let start = p.t.carried_at(0);
    assert!(
        start != from && start != to,
        "the frame flies back from where it was: {start:?}, not {from:?} or {to:?}"
    );
}

#[test]
fn with_logon_animation_off_nothing_plays() {
    let (mut h, _d) = glass_tiles(|c| c.effects.logon_animation = false);
    click_part(&mut h, Part::Tile(0));
    assert_eq!(model(&h).screen, logon::Screen::Tile(0));
    assert!(view(&mut h).play.is_none());
    assert_eq!(next_frame(&mut h, None), Vec::<String>::new());
    assert!(shown(&mut h) == at_rest(&mut h));
}

#[test]
fn a_right_password_says_welcome_while_the_session_starts() {
    let (mut h, mut d) = logged_in();
    assert_eq!(model(&h).said(), Some(logon::text::verifying()));
    d.says(&mut h, GreeterReply::Success);
    assert_eq!(
        d.heard(),
        repriselogin::greeter::GreeterRequest::StartSession
    );
    assert_eq!(model(&h).said(), Some(logon::text::welcome()));
}

/// A Glass greeting for anna and bob, anna's tile noted `anna`.
fn glass_greeting(anna: Option<Note>, notice: Option<Notice>) -> Greeting {
    let mut users = vec![user("anna"), user("bob")];
    users[0].note = anna;
    Greeting {
        interface: repriselogin::Interface::Glass,
        show_users: true,
        users,
        last_user: Some("anna".into()),
        keyboard: Default::default(),
        notice,
        outputs: Vec::new(),
        background: None,
        animation: true,
    }
}

#[test]
fn a_greeter_started_by_switch_user_brings_its_tiles_in() {
    let (mut h, _d) = login_with(repriselogin::Interface::Glass, Vec::new(), |_| {});
    setup(
        &mut h.state,
        glass_greeting(Some(Note::Locked), Some(Notice::SwitchUser)),
    );
    assert_eq!(model(&h).screen, logon::Screen::Tiles);
    let p = view(&mut h).play.as_ref().expect("the tiles come in");
    assert_eq!(p.t.change, logon::Change::Enter);
    assert!(p.since.is_none(), "the clock waits for the first frame");
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    settle(&mut h);
    assert!(shown(&mut h) == at_rest(&mut h));
}

#[test]
fn a_greeting_that_changes_a_tiles_note_shows_it_in_place() {
    let (mut h, mut d) = glass_tiles(|_| {});
    click_part(&mut h, Part::Tile(0));
    settle(&mut h);
    tap(&mut h, &[KEY_A, KEY_A, KEY_A]);
    d.says(
        &mut h,
        GreeterReply::Greeting(glass_greeting(Some(Note::Locked), None)),
    );
    assert_eq!(model(&h).users[0].note, Some(UserNote::Locked));
    assert!(view(&mut h).play.is_none(), "nothing plays");
    assert_eq!(model(&h).password.text, "aaa", "the typed password stays");
    assert!(shown(&mut h) == at_rest(&mut h), "the note shows at once");
    assert_eq!(next_frame(&mut h, None), Vec::<String>::new());
}

#[test]
fn the_screen_enters_when_the_seat_comes_back_and_its_clock_waits_for_the_first_frame() {
    let (mut h, _d) = glass_tiles(|_| {});
    came_forward(&mut h.state);
    let p = view(&mut h).play.as_ref().expect("the screen enters");
    assert_eq!(p.t.change, logon::Change::Enter);
    assert!(p.since.is_none());
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    assert!(view(&mut h).play.as_ref().unwrap().since.is_some());
}

#[test]
fn a_fresh_greeters_tiles_enter_once_the_session_fade_begins_to_show_them() {
    let (mut h, _d) = login_with(repriselogin::Interface::Glass, Vec::new(), |_| {});
    h.state.session_fade = crate::sessionfade::SessionFade::new(600);
    setup(
        &mut h.state,
        glass_greeting(Some(Note::Locked), Some(Notice::SwitchUser)),
    );
    let o = h.state.scene.outputs()[0].clone();
    let ms = std::time::Duration::from_millis;
    let armed = std::time::Instant::now() - ms(2000);
    let since = |h: &mut Harness| view(h).play.as_ref().expect("the tiles come in").since;
    next_frame(&mut h, None);
    assert!(
        since(&mut h).is_none(),
        "the first frame is under the black"
    );
    let fade = h.state.session_fade.fade_mut();
    fade.frame(&o, true, armed);
    fade.modeset(&o, armed);
    fade.presented(&o, armed + ms(37));
    next_frame(&mut h, None);
    assert!(
        since(&mut h).is_none(),
        "the clock waits through the black after the modeset"
    );
    h.state
        .session_fade
        .fade_mut()
        .presented(&o, armed + ms(1038));
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    assert!(since(&mut h).is_some(), "the clock starts with the fade in");
    assert!(
        shown(&mut h) != at_rest(&mut h),
        "the tiles are still on their way in"
    );
}

#[test]
fn a_scale_change_mid_enter_carries_the_enter_on_from_where_it_was() {
    let (mut h, _d) = glass_tiles(|_| {});
    came_forward(&mut h.state);
    next_frame(&mut h, Some(0));
    let since = view(&mut h).play.as_ref().unwrap().since;
    let o = h.state.scene.outputs()[0].clone();
    o.change_current_state(
        None,
        None,
        Some(smithay::output::Scale::Fractional(2.0)),
        None,
    );
    h.state.update_layout();
    let p = view(&mut h).play.as_ref().expect("the Enter carries on");
    assert_eq!(p.t.change, logon::Change::Enter);
    assert_eq!(p.since, since, "it carries on from where it was");
    let l = view(&mut h).layout.as_ref().unwrap().output;
    assert_eq!((l.width, l.height), (512, 384), "laid out at scale 2");
    assert!(
        shown(&mut h) != at_rest(&mut h),
        "the tiles are still on their way in"
    );
    assert_eq!(next_frame(&mut h, None), ["TEST-1"]);
    settle(&mut h);
    assert!(shown(&mut h) == at_rest(&mut h), "it ends at rest");
}
