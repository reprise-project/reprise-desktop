//! Lock mode for the remote desktop server's trusted sessions: its `ConsoleLayout` becomes a
//! `reprise_lock_screen_v1`, and the session casts the screens the compositor made for it.

use crate::Pick;
use crate::inputlink::{Link, LockEvent, LockScreenHandle, Monitor};
use crate::inputproto::reprise_lock_screen_v1::Reason;
use crate::trusted::Trust;
use futures_lite::StreamExt;
use std::sync::{Arc, Weak};
use std::time::Duration;
use wayland_client::WEnum;

const SERVER: &str = crate::trusted::APP_ID;
const SERVER_PATH: &str = "/org/reprise/RemoteDesktop";
const SCREENS_WAIT: Duration = Duration::from_secs(2);

type Layouts = futures_lite::stream::Boxed<Vec<Monitor>>;

/// A trusted Start in lock mode: its lock-screen object and the screens made for it.
pub struct Locked {
    lock: LockScreenHandle,
    screens: Vec<String>,
    layouts: Option<Layouts>,
}

/// What a trusted Start casts: lock mode's screens, else every screen.
pub fn pick(locked: Option<&Locked>) -> Pick {
    match locked {
        Some(l) => Pick::Remote(l.screens.clone()),
        None => Pick::All,
    }
}

/// A Start whose server asked for lock mode that the compositor neither began nor refused.
#[derive(Debug, PartialEq, Eq)]
pub struct NotBegun;

/// Lock mode for a Start `trust`ed for `caller`; None is show mode: an empty layout, no lock
/// screens, or `unavailable` or `replaced`. Any other answer, or none in [`SCREENS_WAIT`], fails.
/// No layout read is show mode for the user's own server and a failure for the hand-over server,
/// whose session may have a locked console.
pub async fn begin(
    conn: &zbus::Connection,
    caller: Option<u32>,
    trust: Trust,
    link: Option<&Link>,
) -> Result<Option<Locked>, NotBegun> {
    let read = match caller {
        Some(caller) => console_layout(conn, caller).await,
        None => None,
    };
    let Some((layout, layouts)) = read else {
        return match trust {
            Trust::Own => Ok(None),
            Trust::HandOver => {
                log::warn!(
                    "lock mode: no layout from the hand-over server; the screen is not shared"
                );
                Err(NotBegun)
            }
        };
    };
    if layout.is_empty() {
        return Ok(None);
    }
    let Some(lock) = link.and_then(|l| l.lock_screen(&layout)) else {
        return Ok(None);
    };
    let events = lock.events();
    let first = futures_lite::future::or(async { events.recv().await.ok() }, async {
        async_io::Timer::after(SCREENS_WAIT).await;
        None
    });
    match first.await {
        Some(LockEvent::Screens(screens)) if !screens.is_empty() => {
            log::info!("lock mode: casting {screens:?}");
            Ok(Some(Locked {
                lock,
                screens,
                layouts: Some(layouts),
            }))
        }
        Some(LockEvent::Ended(WEnum::Value(why @ (Reason::Unavailable | Reason::Replaced)))) => {
            log::info!("lock mode: not begun ({why:?}); sharing every screen");
            Ok(None)
        }
        other => {
            log::warn!("lock mode: not begun ({other:?}); the screen is not shared");
            Err(NotBegun)
        }
    }
}

/// The server's `ConsoleLayout` and its later values, while the process `caller` owns
/// [`SERVER`]; subscribed before the read, so no change falls between.
async fn console_layout(conn: &zbus::Connection, caller: u32) -> Option<(Vec<Monitor>, Layouts)> {
    let owner = reprisebus::name_owner(conn, SERVER).await?;
    if crate::trusted::pid_of(conn, &owner).await != Some(caller) {
        log::warn!("{SERVER} is not owned by the session's caller");
        return None;
    }
    let read = async {
        let props = zbus::fdo::PropertiesProxy::builder(conn)
            .destination(owner)
            .path(SERVER_PATH)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let changes = props.receive_properties_changed().await?;
        let iface = zbus::names::InterfaceName::from_static_str_unchecked(SERVER);
        let layout = props.get(iface, "ConsoleLayout").await?;
        zbus::Result::Ok((layout, changes))
    };
    let (layout, changes) = (read.await)
        .inspect_err(|e| log::info!("{SERVER}: no ConsoleLayout: {e}"))
        .ok()?;
    let layout = Vec::<Monitor>::try_from(layout).ok()?;
    let layouts = changes.filter_map(|signal| {
        let args = signal.args().ok()?;
        if args.interface_name().as_str() != SERVER {
            return None;
        }
        let value = args.changed_properties().get("ConsoleLayout")?;
        Vec::<Monitor>::try_from(value.try_to_owned().ok()?).ok()
    });
    Some((layout, layouts.boxed()))
}

impl Locked {
    /// The handle the session holds, and the task that relays the server's layouts to it until
    /// lock mode ends for it, then closes session `key` if it still holds the handle.
    pub fn hold(
        self,
        conn: zbus::Connection,
        sessions: crate::Sessions,
        key: String,
    ) -> (
        Arc<LockScreenHandle>,
        impl Future<Output = ()> + Send + 'static,
    ) {
        let events = self.lock.events();
        let lock = Arc::new(self.lock);
        let weak = Arc::downgrade(&lock);
        let follow = async move {
            let why = relay(&weak, &events, &self.screens, self.layouts).await;
            close(&conn, &sessions, &key, &weak, why).await;
        };
        (lock, follow)
    }
}

/// Pass the server's layouts to `lock` until lock mode ends for it or its screens change; says
/// which.
async fn relay(
    lock: &Weak<LockScreenHandle>,
    events: &async_channel::Receiver<LockEvent>,
    screens: &[String],
    mut layouts: Option<Layouts>,
) -> String {
    enum Next {
        Lock(Option<LockEvent>),
        Layout(Option<Vec<Monitor>>),
    }
    loop {
        let lock_event = async { Next::Lock(events.recv().await.ok()) };
        let next = match layouts.as_mut() {
            Some(l) => {
                let layout = async { Next::Layout(l.next().await) };
                futures_lite::future::or(lock_event, layout).await
            }
            None => lock_event.await,
        };
        match next {
            Next::Layout(Some(layout)) if !layout.is_empty() => {
                if let Some(l) = lock.upgrade() {
                    l.set_layout(&layout);
                }
            }
            Next::Layout(Some(_)) => {}
            Next::Layout(None) => layouts = None,
            Next::Lock(Some(LockEvent::Screens(now))) if now == screens => {}
            Next::Lock(Some(LockEvent::Screens(now))) => {
                return format!("lock mode's screens are now {now:?}");
            }
            Next::Lock(Some(LockEvent::Ended(reason))) => {
                return format!("lock mode ended ({reason:?})");
            }
            Next::Lock(None) => return "the lock screen is gone".into(),
        }
    }
}

/// Close the session at `key` if it still holds `lock`; a Close from the frontend took it
/// out of the map first.
async fn close(
    conn: &zbus::Connection,
    sessions: &crate::Sessions,
    key: &str,
    lock: &Weak<LockScreenHandle>,
    why: String,
) {
    let gone = {
        let mut map = sessions.lock().unwrap();
        let holds = (map.get_mut(key).and_then(crate::SessionData::remote_mut))
            .and_then(|s| s.lock.as_ref())
            .is_some_and(|l| std::ptr::eq(Arc::as_ptr(l), lock.as_ptr()));
        holds.then(|| map.remove(key)).flatten()
    };
    if let Some(data) = gone {
        log::info!("Session {key}: {why}");
        crate::end_session(conn, key, Some(data)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputlink::tests::{fake_compositor, link_on, put_string, request, send};
    use crate::remotedesktop::RemoteSession;
    use crate::wayland::{OutputInfo, Target};
    use crate::{SessionData, SessionKind};
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::process::{Child, Command, Stdio};

    const FIRST: [Monitor; 2] = [(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 150)];
    const SECOND: [Monitor; 1] = [(0, 0, 1280, 800, 100)];
    const THIRD: [Monitor; 1] = [(0, 0, 1440, 900, 125)];
    const KEY: &str = "/org/freedesktop/portal/desktop/session/1_7/reprise_rdp_0";

    /// `reprise_input_v1` opcodes and `reprise_lock_screen_v1` ones.
    const CREATE_KEYS: u32 = 4;
    const LOCK_SCREEN: u32 = 8;
    const SET_LAYOUT: u32 = 0;
    const SCREEN: u32 = 0;
    const DONE: u32 = 1;
    const ENDED: u32 = 2;
    const CONSOLE_UNLOCKED: u32 = 1;
    const UNAVAILABLE: u32 = 2;
    const REPLACED: u32 = 3;

    struct Bus(Child, String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A private session bus.
    fn bus() -> Option<Bus> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some(Bus(child, line.trim().to_string()))
    }

    async fn connect(bus: &Bus) -> zbus::Connection {
        zbus::connection::Builder::address(bus.1.as_str())
            .build()
            .await
            .unwrap()
    }

    struct FakeServer(Vec<Monitor>);

    #[zbus::interface(name = "org.reprise.RemoteDesktop")]
    impl FakeServer {
        #[zbus(property)]
        fn console_layout(&self) -> Vec<Monitor> {
            self.0.clone()
        }
    }

    /// The remote desktop server, publishing `layout`, in this process.
    async fn server(bus: &Bus, layout: &[Monitor]) -> zbus::Connection {
        zbus::connection::Builder::address(bus.1.as_str())
            .name(SERVER)
            .serve_at(SERVER_PATH, FakeServer(layout.to_vec()))
            .build()
            .await
            .unwrap()
    }

    async fn publish(server: &zbus::Connection, layout: &[Monitor]) {
        let iface = (server.object_server())
            .interface::<_, FakeServer>(SERVER_PATH)
            .await
            .unwrap();
        iface.get_mut().await.0 = layout.to_vec();
        let emitter = iface.signal_emitter();
        iface
            .get()
            .await
            .console_layout_changed(emitter)
            .await
            .unwrap();
    }

    /// `set_layout`'s array for `layout`, as the compositor reads it.
    fn words(layout: &[Monitor]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for &(x, y, w, h, scale) in layout {
            for v in [x, y, w, h] {
                bytes.extend(v.to_ne_bytes());
            }
            bytes.extend(scale.to_ne_bytes());
        }
        bytes
    }

    fn array(body: &[u8]) -> Vec<u8> {
        let len = u32::from_ne_bytes(body[..4].try_into().unwrap()) as usize;
        body[4..4 + len].to_vec()
    }

    fn screens(sock: &mut UnixStream, lock: u32, names: &[&str]) {
        for name in names {
            let mut body = Vec::new();
            put_string(&mut body, name);
            send(sock, lock, SCREEN, &body);
        }
        send(sock, lock, DONE, &[]);
    }

    fn ended(sock: &mut UnixStream, lock: u32, reason: u32) {
        send(sock, lock, ENDED, &reason.to_ne_bytes());
    }

    /// A v9 compositor that hands each `set_layout` to `answer` with the lock-screen object it
    /// came on, until a `create_keys` marks the end or `answer` says to stop. Returns every
    /// layout it was sent.
    fn compositor(
        mut theirs: UnixStream,
        mut answer: impl FnMut(&mut UnixStream, u32, usize) -> bool + Send + 'static,
    ) -> std::thread::JoinHandle<Vec<Vec<u8>>> {
        std::thread::spawn(move || {
            let input = fake_compositor(&mut theirs, 2, 9);
            theirs
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut lock = None;
            let mut layouts = Vec::new();
            while let Ok((object, opcode, body)) = request(&mut theirs) {
                match (object, opcode) {
                    (o, LOCK_SCREEN) if o == input => {
                        lock = Some(u32::from_ne_bytes(body[..4].try_into().unwrap()));
                    }
                    (o, CREATE_KEYS) if o == input => break,
                    (o, SET_LAYOUT) if Some(o) == lock => {
                        layouts.push(array(&body));
                        if !answer(&mut theirs, o, layouts.len()) {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            layouts
        })
    }

    fn at(name: &str, x: i32, y: i32) -> OutputInfo {
        OutputInfo {
            name: name.into(),
            x,
            y,
            width: 1920,
            height: 1080,
            ..Default::default()
        }
    }

    /// Lock mode with `FIRST` on `link`, casting REMOTE-1.
    fn remote_one(link: &Link) -> Locked {
        Locked {
            lock: link.lock_screen(&FIRST).unwrap(),
            screens: vec!["REMOTE-1".into()],
            layouts: None,
        }
    }

    /// `locked` held by a RemoteDesktop session in `sessions`, and the task following it.
    fn held_in(
        sessions: &crate::Sessions,
        locked: Locked,
        conn: zbus::Connection,
    ) -> impl Future<Output = ()> + Send + 'static {
        let (lock, follow) = locked.hold(conn, sessions.clone(), KEY.into());
        let mut session = RemoteSession::default();
        session.lock = Some(lock);
        let data = SessionData::new(":1.7".into(), SessionKind::RemoteDesktop(session));
        sessions.lock().unwrap().insert(KEY.into(), data);
        follow
    }

    async fn within<T>(f: impl Future<Output = T>) -> T {
        futures_lite::future::or(f, async {
            async_io::Timer::after(Duration::from_secs(5)).await;
            panic!("not within 5 s")
        })
        .await
    }

    type Begun = Result<Pick, NotBegun>;

    /// `begin` for a server in this process publishing `layout`, asked by `caller`, on a
    /// compositor that answers each layout with `answer`. Returns the pick and the layouts the
    /// compositor was sent.
    fn begun(
        layout: &[Monitor],
        caller: u32,
        trust: Trust,
        answer: impl FnMut(&mut UnixStream, u32, usize) -> bool + Send + 'static,
    ) -> Option<(Begun, Vec<Vec<u8>>)> {
        let bus = bus()?;
        let (ours, theirs) = UnixStream::pair().unwrap();
        let fake = compositor(theirs, answer);
        let link = link_on(ours);
        let pick = zbus::block_on(async {
            let _server = server(&bus, layout).await;
            let conn = connect(&bus).await;
            let begun = begin(&conn, Some(caller), trust, Some(&link)).await;
            begun.map(|locked| pick(locked.as_ref()))
        });
        let _marker = link.create_keys();
        Some((pick, fake.join().unwrap()))
    }

    #[test]
    fn a_trusted_start_with_a_console_layout_casts_the_remote_screens_only() {
        let answer = |sock: &mut UnixStream, lock, _| {
            screens(sock, lock, &["REMOTE-1", "REMOTE-2"]);
            true
        };
        let Some((pick, layouts)) = begun(&FIRST, std::process::id(), Trust::Own, answer) else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(layouts, [words(&FIRST)], "the server's layout is asked for");
        let names = ["REMOTE-1".to_string(), "REMOTE-2".to_string()];
        let pick = pick.unwrap();
        assert_eq!(pick, Pick::Remote(names.to_vec()));
        let outputs = [
            at("eDP-1", 0, 0),
            at("REMOTE-2", 2560, 0),
            at("REMOTE-1", 0, 0),
        ];
        let chosen = crate::choose_from(pick, crate::SOURCE_MONITOR, &outputs, &[], None, |_| {
            panic!("a trusted start shows no dialog")
        })
        .unwrap()
        .unwrap();
        let cast: Vec<_> = chosen.targets.iter().map(|(t, _)| t.clone()).collect();
        assert_eq!(
            cast,
            names.map(Target::Output),
            "the client's screens in the compositor's order, not the console's"
        );
    }

    #[test]
    fn an_empty_console_layout_shares_every_screen_as_before() {
        let me = std::process::id();
        let never = |_: &mut UnixStream, _, _| panic!("no layout reaches the compositor");
        let Some((pick, layouts)) = begun(&[], me, Trust::Own, never) else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(pick, Ok(Pick::All));
        assert!(layouts.is_empty(), "no lock screen is made");

        let (pick, layouts) = begun(&FIRST, me + 1, Trust::Own, never).unwrap();
        assert_eq!(pick, Ok(Pick::All), "a layout another process publishes");
        assert!(layouts.is_empty(), "is not read");

        for (reason, name) in [(UNAVAILABLE, "unavailable"), (REPLACED, "replaced")] {
            let refused = move |sock: &mut UnixStream, lock, _| {
                ended(sock, lock, reason);
                true
            };
            let (pick, layouts) = begun(&FIRST, me, Trust::Own, refused).unwrap();
            assert_eq!(pick, Ok(Pick::All), "ended({name})");
            assert_eq!(layouts, [words(&FIRST)]);
        }
    }

    #[test]
    fn a_hand_over_server_without_the_name_shares_no_screen() {
        let me = std::process::id();
        let never = |_: &mut UnixStream, _, _| panic!("no layout reaches the compositor");
        let Some((pick, _)) = begun(&FIRST, me + 1, Trust::HandOver, never) else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(pick, Err(NotBegun), "another process owns the name");

        let bus = bus().unwrap();
        let (ours, theirs) = UnixStream::pair().unwrap();
        let fake = compositor(theirs, never);
        let link = link_on(ours);
        let unowned = zbus::block_on(async {
            let conn = connect(&bus).await;
            begin(&conn, Some(me), Trust::HandOver, Some(&link)).await
        });
        assert!(matches!(unowned, Err(NotBegun)), "nobody owns the name");
        let _marker = link.create_keys();
        fake.join().unwrap();

        let (pick, layouts) = begun(&[], me, Trust::HandOver, never).unwrap();
        assert_eq!(
            pick,
            Ok(Pick::All),
            "an owner publishing no layout asks for show mode"
        );
        assert!(layouts.is_empty());
    }

    #[test]
    fn lock_mode_the_compositor_neither_begins_nor_refuses_fails_the_start() {
        let me = std::process::id();
        let silent = |_: &mut UnixStream, _, _| true;
        let Some((pick, _)) = begun(&FIRST, me, Trust::Own, silent) else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        assert_eq!(pick, Err(NotBegun), "no screens within {SCREENS_WAIT:?}");

        let unlocked = |sock: &mut UnixStream, lock, _| {
            ended(sock, lock, CONSOLE_UNLOCKED);
            true
        };
        let (pick, _) = begun(&FIRST, me, Trust::Own, unlocked).unwrap();
        assert_eq!(pick, Err(NotBegun), "ended(console_unlocked) first");

        let none = |sock: &mut UnixStream, lock, _| {
            screens(sock, lock, &[]);
            true
        };
        let (pick, _) = begun(&FIRST, me, Trust::Own, none).unwrap();
        assert_eq!(pick, Err(NotBegun), "no screens");

        let lost = |_: &mut UnixStream, _, _| false;
        let (pick, _) = begun(&FIRST, me, Trust::Own, lost).unwrap();
        assert_eq!(pick, Err(NotBegun), "the compositor's connection lost");
    }

    #[test]
    fn a_layout_change_is_relayed_to_the_compositor() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (answered, answered_rx) = async_channel::unbounded();
        let fake = compositor(theirs, move |sock, lock, n| {
            if n < 3 {
                screens(sock, lock, &["REMOTE-1"]);
            } else {
                ended(sock, lock, REPLACED);
            }
            let _ = answered.send_blocking(n);
            true
        });
        let link = link_on(ours);
        let sessions = crate::Sessions::default();
        zbus::block_on(async {
            let server = server(&bus, &FIRST).await;
            let conn = connect(&bus).await;
            let locked = begin(&conn, Some(std::process::id()), Trust::Own, Some(&link))
                .await
                .unwrap()
                .expect("lock mode");
            let follow = zruntime::spawn(held_in(&sessions, locked, conn));
            assert_eq!(within(answered_rx.recv()).await, Ok(1));
            publish(&server, &SECOND).await;
            assert_eq!(
                within(answered_rx.recv()).await,
                Ok(2),
                "answered with the same screen, resized"
            );
            publish(&server, &THIRD).await;
            within(follow).await.unwrap();
        });
        let _marker = link.create_keys();
        assert_eq!(
            fake.join().unwrap(),
            [words(&FIRST), words(&SECOND), words(&THIRD)],
            "each published layout, the same screens kept between them"
        );
        assert!(sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn ended_closes_the_remote_desktop_session() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        for reason in [CONSOLE_UNLOCKED, REPLACED] {
            let (ours, theirs) = UnixStream::pair().unwrap();
            let fake = compositor(theirs, move |sock, lock, _| {
                ended(sock, lock, reason);
                true
            });
            let link = link_on(ours);
            let sessions = crate::Sessions::default();
            zbus::block_on(async {
                let follow = held_in(&sessions, remote_one(&link), connect(&bus).await);
                within(follow).await;
            });
            assert!(
                sessions.lock().unwrap().is_empty(),
                "ended({reason}) closes the session"
            );
            let _marker = link.create_keys();
            assert_eq!(fake.join().unwrap(), [words(&FIRST)]);
        }

        let (ours, theirs) = UnixStream::pair().unwrap();
        let fake = compositor(theirs, |_, _, _| false);
        let link = link_on(ours);
        let sessions = crate::Sessions::default();
        zbus::block_on(async {
            let follow = held_in(&sessions, remote_one(&link), connect(&bus).await);
            within(follow).await;
        });
        fake.join().unwrap();
        assert!(
            sessions.lock().unwrap().is_empty(),
            "a lost compositor closes the session"
        );
    }

    #[test]
    fn a_changed_set_of_screens_closes_the_session() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let (ours, theirs) = UnixStream::pair().unwrap();
        let fake = compositor(theirs, |sock, lock, _| {
            screens(sock, lock, &["REMOTE-1", "REMOTE-2"]);
            true
        });
        let link = link_on(ours);
        let sessions = crate::Sessions::default();
        zbus::block_on(async {
            let follow = held_in(&sessions, remote_one(&link), connect(&bus).await);
            within(follow).await;
        });
        assert!(
            sessions.lock().unwrap().is_empty(),
            "a screen more than the session casts closes it"
        );
        let _marker = link.create_keys();
        fake.join().unwrap();
    }
}
