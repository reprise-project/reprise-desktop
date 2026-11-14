//! A second X11 connection beside Smithay's window manager, on its own thread so a round trip never holds up the event loop. It moves the X input focus, reads `_NET_WM_ICON`, hears X11 programs' bells (XKB `BellNotify`, on a second thread), and counts X11 programs (X-Resource), saying so once none has been connected for [`IDLE_GRACE`], which stops the server. Xwayland's `-terminate` never fires, since this connection and the window manager's keep it up.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use reprisekit::icon::{IconSource, MAX_CLIENT_ICON, closer_to_target, premultiply};
use smithay::input::dnd::DndAction;
use smithay::reexports::calloop::channel::Sender;
use x11rb::{
    CURRENT_TIME, NONE,
    connection::{Connection, RequestConnection},
    protocol::{
        Event,
        res::{self, ConnectionExt as _},
        xkb::{self, ConnectionExt as _},
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, GrabMode,
            GrabStatus, InputFocus, PropMode, WindowClass,
        },
    },
    rust_connection::{DefaultStream, RustConnection},
    wrapper::ConnectionExt as _,
};

pub enum Req {
    /// Give `window` the X input focus (`input`) and offer `WM_TAKE_FOCUS` (`take`), per its ICCCM input model.
    Focus {
        window: u32,
        input: bool,
        take: bool,
    },
    Unfocus(u32),
    Icon(u32),
    Property(u32, u32),
    DragActions,
    /// Whether an X client holds the pointer grab as override-redirect `window` shows.
    Grabbed(u32),
    /// Disconnect the X client owning `window` if X-Resource says it is `pid`.
    KillOwner {
        window: u32,
        pid: u32,
    },
}

pub enum Back {
    Icon(u32, Option<IconSource>),
    Idle,
    DragActions(Vec<DndAction>),
    Grabbed(u32),
    /// An X11 program rang the bell from the window at the top of this X window's tree (a child
    /// of the root), or from none (a core `Bell`).
    Bell(Option<u32>),
}

pub const IDLE_GRACE: Duration = Duration::from_secs(10);

const COUNT_EVERY: Duration = Duration::from_secs(2);

/// Bells closer together than this after a passed-on one are dropped, so a program ringing in a
/// loop wakes the compositor five times a second at most.
const BELL_GAP: Duration = Duration::from_millis(200);

pub struct Helper {
    tx: mpsc::Sender<Req>,
}

impl Helper {
    pub fn spawn(display: u32, cookie: [u8; 16], back: Sender<Back>) -> std::io::Result<Helper> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("x11-helper".into())
            .spawn(move || {
                if let Err(e) = run(display, cookie, &rx, &back) {
                    log::warn!("X11 focus connection: {e}");
                }
            })?;
        Ok(Helper { tx })
    }

    pub fn send(&self, req: Req) {
        // A closed channel means the thread is gone; it logged why.
        let _ = self.tx.send(req);
    }
}

/// The compositor's environment has no `XAUTHORITY`, so the cookie is handed over here.
fn connect(display: u32, cookie: [u8; 16]) -> Result<RustConnection, Box<dyn std::error::Error>> {
    let socket = std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{display}"))?;
    let (stream, _) = DefaultStream::from_unix_stream(socket)?;
    Ok(RustConnection::connect_to_stream_with_auth_info(
        stream,
        0,
        super::auth::NAME.to_vec(),
        cookie.to_vec(),
    )?)
}

fn run(
    display: u32,
    cookie: [u8; 16],
    rx: &mpsc::Receiver<Req>,
    back: &Sender<Back>,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Arc::new(connect(display, cookie)?);
    let root = conn.setup().roots[0].root;
    listen_for_bells(&conn);
    {
        let (conn, back) = (conn.clone(), back.clone());
        std::thread::Builder::new()
            .name("x11-events".into())
            .spawn(move || read_events(&conn, root, &back))?;
    }
    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };
    let wm_protocols = atom(b"WM_PROTOCOLS")?;
    let wm_take_focus = atom(b"WM_TAKE_FOCUS")?;
    let net_wm_icon = atom(b"_NET_WM_ICON")?;
    let net_active_window = atom(b"_NET_ACTIVE_WINDOW")?;
    let xdnd_selection = atom(b"XdndSelection")?;
    let xdnd_action_list = atom(b"XdndActionList")?;
    let xdnd_actions = [
        (atom(b"XdndActionCopy")?, DndAction::Copy),
        (atom(b"XdndActionMove")?, DndAction::Move),
        (atom(b"XdndActionAsk")?, DndAction::Ask),
    ];
    // EWMH's record of the active window, which Smithay only clears on a focus change it sees as one to nothing.
    let set_active = |window: u32| {
        conn.change_property32(
            PropMode::REPLACE,
            root,
            net_active_window,
            AtomEnum::WINDOW,
            &[window],
        )
        .map(drop)
    };
    let mut focused = None;
    let wm_window = conn
        .get_property(
            false,
            root,
            atom(b"_NET_SUPPORTING_WM_CHECK")?,
            AtomEnum::WINDOW,
            0,
            1,
        )?
        .reply()?
        .value32()
        .and_then(|mut v| v.next());
    let can_count = conn
        .extension_information(res::X11_EXTENSION_NAME)?
        .is_some();
    if !can_count {
        log::warn!("Xwayland has no X-Resource; it keeps running once started");
    }
    // Never mapped, so a grab on it never activates: the status says only whether another
    // client holds one, which the server checks first.
    let probe = conn.generate_id()?;
    conn.create_window(
        0,
        probe,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )?;
    let mut idle = IdleClock::default();
    let mut counted = Instant::now();
    loop {
        let req = match rx.recv_timeout(COUNT_EVERY.saturating_sub(counted.elapsed())) {
            Ok(req) => Some(req),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if can_count && counted.elapsed() >= COUNT_EVERY {
            counted = Instant::now();
            let clients = conn.res_query_clients()?.reply()?.clients;
            let clients = clients.iter().map(|c| (c.resource_base, c.resource_mask));
            let n = programs(clients, conn.setup().resource_id_base, wm_window);
            if idle.note(n, counted) && back.send(Back::Idle).is_err() {
                return Ok(());
            }
        }
        let Some(req) = req else {
            continue;
        };
        match req {
            Req::Focus {
                window,
                input,
                take,
            } => {
                if input {
                    conn.set_input_focus(InputFocus::NONE, window, CURRENT_TIME)?;
                }
                if take {
                    let event = ClientMessageEvent::new(
                        32,
                        window,
                        wm_protocols,
                        [wm_take_focus, CURRENT_TIME, 0, 0, 0],
                    );
                    conn.send_event(false, window, EventMask::NO_EVENT, event)?;
                }
                set_active(window)?;
                conn.flush()?;
                focused = Some(window);
            }
            Req::Unfocus(window) => {
                if focused == Some(window) {
                    conn.set_input_focus(InputFocus::NONE, NONE, CURRENT_TIME)?;
                    set_active(NONE)?;
                    conn.flush()?;
                    focused = None;
                }
            }
            // The drag's source owns `XdndSelection`. A source that lists no actions, or is gone, answers with none.
            Req::DragActions => {
                let owner = conn.get_selection_owner(xdnd_selection)?.reply()?.owner;
                let listed = conn
                    .get_property(false, owner, xdnd_action_list, AtomEnum::ATOM, 0, 16)?
                    .reply()
                    .ok()
                    .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
                    .unwrap_or_default();
                let actions = xdnd_actions
                    .iter()
                    .filter(|(a, _)| listed.contains(a))
                    .map(|(_, action)| *action)
                    .collect();
                if back.send(Back::DragActions(actions)).is_err() {
                    return Ok(());
                }
            }
            Req::Grabbed(window) => {
                let status = conn
                    .grab_pointer(
                        false,
                        probe,
                        EventMask::NO_EVENT,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                        NONE,
                        NONE,
                        CURRENT_TIME,
                    )?
                    .reply()?
                    .status;
                if status == GrabStatus::SUCCESS {
                    conn.ungrab_pointer(CURRENT_TIME)?;
                    conn.flush()?;
                }
                if status == GrabStatus::ALREADY_GRABBED
                    && back.send(Back::Grabbed(window)).is_err()
                {
                    return Ok(());
                }
            }
            Req::KillOwner { window, pid } if !can_count => {
                log::info!("x11 window {window:#x}: no X-Resource to confirm pid {pid}; kept");
            }
            Req::KillOwner { window, pid } => {
                let spec = res::ClientIdSpec {
                    client: window,
                    mask: res::ClientIdMask::LOCAL_CLIENT_PID,
                };
                let owner = conn
                    .res_query_client_ids(&[spec])?
                    .reply()
                    .ok()
                    .and_then(|r| r.ids.iter().find_map(|id| id.value.first().copied()));
                if owner == Some(pid) {
                    log::info!("x11 window {window:#x}: disconnecting its client, pid {pid}");
                    conn.kill_client(window)?;
                    conn.flush()?;
                } else {
                    log::info!("x11 window {window:#x}: client is pid {owner:?}, not {pid}; kept");
                }
            }
            Req::Property(_, a) if a != net_wm_icon => {}
            Req::Icon(window) | Req::Property(window, _) => {
                let reply = conn
                    .get_property(
                        false,
                        window,
                        net_wm_icon,
                        AtomEnum::CARDINAL,
                        0,
                        u32::MAX / 4,
                    )?
                    .reply();
                // A window gone by now answers with an error: no icon.
                let icon = reply
                    .ok()
                    .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
                    .and_then(|data| best_icon(&data));
                if back.send(Back::Icon(window, icon)).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

/// Ask XKB for a `BellNotify` on every bell; without XKB, X11 bells go unheard.
fn listen_for_bells(conn: &RustConnection) {
    let selected = (|| -> Result<bool, Box<dyn std::error::Error>> {
        if conn
            .extension_information(xkb::X11_EXTENSION_NAME)?
            .is_none()
            || !conn.xkb_use_extension(1, 0)?.reply()?.supported
        {
            return Ok(false);
        }
        conn.xkb_select_events(
            xkb::ID::USE_CORE_KBD.into(),
            xkb::EventType::from(0u16),
            xkb::EventType::BELL_NOTIFY,
            xkb::MapPart::from(0u16),
            xkb::MapPart::from(0u16),
            &xkb::SelectEventsAux::new(),
        )?
        .check()?;
        Ok(true)
    })();
    match selected {
        Ok(true) => {}
        Ok(false) => log::warn!("Xwayland has no XKB; X11 programs' bells go unheard"),
        Err(e) => log::warn!("X11 bells: {e}"),
    }
}

/// The connection's events until it closes: each bell goes back as [`Back::Bell`], at most one
/// per [`BELL_GAP`]. Errors for requests nobody waits on (a focus on a window that just unmapped)
/// arrive here too and are dropped.
fn read_events(conn: &RustConnection, root: u32, back: &Sender<Back>) {
    let mut last = None;
    loop {
        let event = match conn.wait_for_event() {
            Ok(event) => event,
            Err(e) => {
                log::debug!("X11 events: {e}");
                return;
            }
        };
        let Event::XkbBellNotify(bell) = event else {
            continue;
        };
        if !rings(&mut last, Instant::now()) {
            continue;
        }
        let window = (bell.window != NONE).then(|| top_of(conn, root, bell.window));
        if back.send(Back::Bell(window)).is_err() {
            return;
        }
    }
}

/// Whether a bell at `now` is passed on, `last` holding the last one that was; records it if so.
fn rings(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|t| now.saturating_duration_since(t) < BELL_GAP) {
        return false;
    }
    *last = Some(now);
    true
}

/// The child of `root` that holds `window`: the window manager's frame around a toplevel, or
/// `window` itself when it is gone or already at the top.
fn top_of(conn: &RustConnection, root: u32, window: u32) -> u32 {
    let mut w = window;
    // The bound stops at a tree deeper than any program builds.
    for _ in 0..16 {
        match conn.query_tree(w).ok().and_then(|c| c.reply().ok()) {
            Some(tree) if tree.parent != root && tree.parent != NONE => w = tree.parent,
            _ => break,
        }
    }
    w
}

/// The X11 programs among the server's clients, as X-Resource lists them: all but the server (base 0), this connection (`own`) and the window manager's, which owns `wm_window`.
fn programs(clients: impl Iterator<Item = (u32, u32)>, own: u32, wm_window: Option<u32>) -> usize {
    clients
        .filter(|&(base, mask)| {
            base != 0 && base != own && wm_window.is_none_or(|w| w & !mask != base)
        })
        .count()
}

#[derive(Default)]
struct IdleClock {
    since: Option<Instant>,
    told: bool,
}

impl IdleClock {
    /// `n` programs are connected at `now`: whether to say the server is idle. Says it once.
    fn note(&mut self, n: usize, now: Instant) -> bool {
        if n > 0 {
            *self = IdleClock::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if self.told || now.duration_since(since) < IDLE_GRACE {
            return false;
        }
        self.told = true;
        true
    }
}

/// The `_NET_WM_ICON` image (width, height, ARGB pixels, repeated) whose width is closest to the preferred size, premultiplied. Images over `MAX_CLIENT_ICON` a side, empty ones and a truncated last one are skipped.
pub fn best_icon(data: &[u32]) -> Option<IconSource> {
    let max = MAX_CLIENT_ICON as u32;
    let mut best: Option<(u32, u32, &[u32])> = None;
    let mut rest = data;
    while let [w, h, tail @ ..] = rest {
        let n = (*w as usize).checked_mul(*h as usize)?;
        if n > tail.len() {
            break;
        }
        let (pixels, next) = tail.split_at(n);
        let usable = *w > 0 && *h > 0 && *w <= max && *h <= max;
        if usable && closer_to_target(*w as i32, best.map(|b| b.0 as i32)) {
            best = Some((*w, *h, pixels));
        }
        rest = next;
    }
    best.map(|(width, height, pixels)| IconSource {
        width,
        height,
        pixels: pixels.iter().map(|&p| premultiply(p)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, argb: u32) -> Vec<u32> {
        let mut v = vec![w, h];
        v.extend(std::iter::repeat_n(argb, (w * h) as usize));
        v
    }

    #[test]
    fn a_bell_inside_the_gap_of_a_passed_on_one_is_dropped() {
        let (mut last, t) = (None, Instant::now());
        assert!(rings(&mut last, t), "the first bell is passed on");
        assert!(!rings(&mut last, t + BELL_GAP / 2), "inside the gap");
        assert!(rings(&mut last, t + BELL_GAP), "the gap has passed");
        assert!(
            !rings(&mut last, t + BELL_GAP + BELL_GAP / 2),
            "the gap runs from the last bell passed on"
        );
    }

    #[test]
    fn the_server_its_own_and_the_window_managers_connections_are_not_programs() {
        let mask = 0x001f_ffff;
        let server = (0, mask);
        let own = (0x0040_0000, mask);
        let wm = (0x0020_0000, mask);
        let xterm = (0x0060_0000, mask);
        let wm_window = Some(0x0020_0001);
        let n = |c: &[(u32, u32)]| programs(c.iter().copied(), own.0, wm_window);
        assert_eq!(n(&[server, own, wm]), 0);
        assert_eq!(n(&[server, own, wm, xterm]), 1);
        assert_eq!(
            programs([server, own, wm].into_iter(), own.0, None),
            1,
            "with no check window, the window manager counts and the server stays up"
        );
    }

    #[test]
    fn the_server_is_idle_once_no_program_has_been_connected_for_the_grace() {
        let t = Instant::now();
        let s = |secs| t + Duration::from_secs(secs);
        let mut clock = IdleClock::default();
        assert!(!clock.note(1, t), "a program is connected");
        assert!(!clock.note(0, s(2)), "the last program just went");
        assert!(!clock.note(0, s(11)), "9 s without one");
        assert!(clock.note(0, s(12)), "10 s without one");
        assert!(!clock.note(0, s(14)), "said once");
        assert!(!clock.note(1, s(15)), "a program came back");
        assert!(!clock.note(0, s(16)));
        assert!(!clock.note(0, s(25)), "the grace starts over");
        assert!(clock.note(0, s(26)));
    }

    #[test]
    fn the_icon_closest_to_32_px_wins() {
        let mut data = image(16, 16, 0xff00_00ff);
        data.extend(image(32, 32, 0xff00_ff00));
        data.extend(image(48, 48, 0xffff_0000));
        let icon = best_icon(&data).expect("an icon");
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.pixels[0], 0xff00_ff00);
    }

    #[test]
    fn icon_pixels_are_premultiplied() {
        let icon = best_icon(&image(1, 1, 0x80ff_ffff)).expect("an icon");
        assert_eq!(icon.pixels, [premultiply(0x80ff_ffff)]);
        assert_ne!(icon.pixels[0], 0x80ff_ffff);
    }

    #[test]
    fn a_truncated_or_oversized_icon_is_skipped() {
        let mut data = image(16, 16, 0xff00_00ff);
        data.extend([MAX_CLIENT_ICON as u32 + 1, 1]);
        data.extend(std::iter::repeat_n(0, MAX_CLIENT_ICON + 1));
        data.extend([64, 64, 0, 0]);
        let icon = best_icon(&data).expect("the 16 px icon");
        assert_eq!(icon.width, 16);
        assert!(best_icon(&[]).is_none());
        assert!(best_icon(&[0, 0]).is_none());
        assert!(best_icon(&[u32::MAX, u32::MAX, 1]).is_none());
    }
}
