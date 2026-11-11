use super::item::{Pixmaps, ToolTip, decode_names, icon_ref, item_texts};
use super::menu::{LABEL_CHARS, MENU_ROWS, parse_layout};
use super::notifications::{
    CAPABILITIES, action_icons, as_sent_by, notification_icons, picture_file,
};
use super::watcher::{
    Admission, MAX_REGISTRATIONS, MAX_REGISTRATIONS_PER_SENDER, admit, resolve_registration,
};
use super::*;
use crate::tray::{
    DEFAULT_PROP, IconFiles, MenuNode, NOTIFY_SIZES, Notification, item_pixmaps, notify,
};
use repriseaudio::sounds::Sound;
use reprisebus::peer::AppIdentity;
use reprisekit::controls::tooltip::TIP_CHARS;
use reprisekit::icon::IconSource;
use std::collections::HashMap;
use std::os::fd::FromRawFd;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Receiver;
use zbus::Variant;
use zbus::as_variant;

fn child(id: i32, props: Vec<(&str, Variant<'static>)>) -> Variant<'static> {
    let map: HashMap<&str, Variant> = props.into_iter().collect();
    Variant::from((id, map, Vec::<Variant>::new()))
        .try_to_owned()
        .unwrap()
        .into()
}

/// A `GetLayout` reply's rows, parsed from a message carrying `children`.
fn parse(children: &[Variant<'static>]) -> Vec<MenuNode> {
    let props = HashMap::<&str, Variant>::new();
    let msg = zbus::Message::signal("/menu", "com.canonical.dbusmenu", "Reply")
        .build(&(1u32, (0i32, props, children)))
        .unwrap();
    parse_layout(&msg).expect("a well-formed reply")
}

#[test]
fn parse_layout_extracts_nodes() {
    let children = vec![
        child(
            7,
            vec![
                ("label", Variant::from("_Connect")),
                ("enabled", Variant::from(false)),
                ("toggle-type", Variant::from("checkmark")),
                ("toggle-state", Variant::from(1i32)),
            ],
        ),
        child(8, vec![("type", Variant::from("separator"))]),
        child(
            9,
            vec![
                ("label", Variant::from("More")),
                ("visible", Variant::from(false)),
                ("children-display", Variant::from("submenu")),
            ],
        ),
    ];
    let nodes = parse(&children);
    assert_eq!(nodes.len(), 3);
    assert_eq!(
        nodes[0],
        MenuNode {
            id: 7,
            label: "_Connect".into(),
            separator: false,
            enabled: false,
            visible: true,
            checked: Some(true),
            radio: false,
            has_children: false,
            default: false,
        }
    );
    assert!(nodes[1].separator);
    assert_eq!(nodes[1].id, 8);
    assert!(!nodes[2].visible);
    assert!(nodes[2].has_children);
    assert_eq!(nodes[2].checked, None);
    let radio = parse(&[child(
        10,
        vec![
            ("label", Variant::from("Balanced")),
            ("toggle-type", Variant::from("radio")),
            ("toggle-state", Variant::from(1i32)),
        ],
    )]);
    assert_eq!(
        (radio[0].checked, radio[0].radio),
        (Some(true), true),
        "a radio row is on and bulleted"
    );
}

#[test]
fn parse_layout_reads_variant_wrapped_properties() {
    let children = vec![child(
        3,
        vec![
            ("label", Variant::from("_Connect")),
            ("enabled", Variant::from(false)),
            ("toggle-type", Variant::from("checkmark")),
            ("toggle-state", Variant::from(0i32)),
            ("children-display", Variant::from("submenu")),
            (DEFAULT_PROP, Variant::from(true)),
        ],
    )];
    let Variant::Structure(s) = &children[0] else {
        panic!("fixture is not a structure");
    };
    let Some(Variant::Dict(props)) = s.fields().get(1) else {
        panic!("fixture has no property dictionary");
    };
    assert!(
        props.iter().all(|(_, v)| matches!(v, Variant::Variant(_))),
        "fixture is not in the a{{sv}} wire shape"
    );
    assert_eq!(
        parse(&children),
        vec![MenuNode {
            id: 3,
            label: "_Connect".into(),
            separator: false,
            enabled: false,
            visible: true,
            checked: Some(false),
            radio: false,
            has_children: true,
            default: true,
        }]
    );
}

#[test]
fn parse_layout_clips_labels_and_caps_rows() {
    let long = "x".repeat(LABEL_CHARS + 1000);
    let children: Vec<Variant<'static>> = (0..MENU_ROWS as i32 + 10)
        .map(|id| child(id, vec![("label", Variant::from(long.clone()))]))
        .collect();
    let nodes = parse(&children);
    assert_eq!(nodes.len(), MENU_ROWS);
    assert!(nodes.iter().all(|n| n.label.chars().count() == LABEL_CHARS));
}

#[test]
fn icon_ref_drops_empty_names_and_names_past_path_max() {
    let max = libc::PATH_MAX as usize;
    assert_eq!(icon_ref(None), None);
    assert_eq!(icon_ref(Some(String::new())), None);
    assert_eq!(icon_ref(Some("x".repeat(max + 1))), None);
    let longest = "x".repeat(max);
    assert_eq!(icon_ref(Some(longest.clone())), Some(longest));
}

#[test]
fn item_texts_clip_the_title_and_the_tooltip() {
    let long = "x".repeat(TIP_CHARS + 1000);
    let tip: ToolTip = ("", vec![], &long, "");
    let (title, tooltip) = item_texts(Some(long.clone()), Some(tip));
    assert_eq!(title.chars().count(), TIP_CHARS);
    assert_eq!(tooltip.chars().count(), TIP_CHARS);
    let blank: ToolTip = ("", vec![], " ", "");
    let (_, tooltip) = item_texts(Some(long), Some(blank));
    assert_eq!(tooltip.chars().count(), TIP_CHARS);
}

#[test]
fn item_texts_drop_bidi_and_control_characters() {
    let tip: ToolTip = ("", vec![], "Tip\u{202e}\u{0}", "");
    let (title, tooltip) = item_texts(Some("Ti\u{200b}tle\n".into()), Some(tip));
    assert_eq!((title.as_str(), tooltip.as_str()), ("Title", "Tip"));
}

#[test]
fn icon_pixmap_variant_decodes_borrowing_the_pixels() {
    use zbus::wire::{LE, serialized::Context, to_bytes};
    let list = vec![(2i32, 1i32, vec![7u8; 8])];
    let data = to_bytes(Context::new(LE, 0), &Variant::from(list)).unwrap();
    let (v, _): (as_variant::Deserialize<Pixmaps>, _) = data.deserialize().unwrap();
    assert_eq!(v.0, [(2, 1, &[7u8; 8][..])]);
}

fn blue_svg(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("reprise-notify-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("icon.svg");
    std::fs::write(
        &file,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#00f"/></svg>"##,
    )
    .unwrap();
    file.to_str().unwrap().to_string()
}

fn hicolor() -> Icons {
    Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[]))))
}

#[test]
fn a_picture_does_not_replace_the_app_icon() {
    let hints = notify::Hints {
        image: Some(notify::ImageData {
            width: 2,
            height: 2,
            stride: 6,
            alpha: false,
            bits: 8,
            channels: 3,
            data: &[0; 12],
        }),
        ..Default::default()
    };
    let (app, image) = zbus::block_on(notification_icons(
        &hicolor(),
        "",
        &hints,
        blue_svg("both"),
        None,
    ));
    assert_eq!(app.len(), NOTIFY_SIZES.len());
    assert_eq!(image.len(), NOTIFY_SIZES.len());
}

#[test]
fn image_path_is_the_picture_and_an_unresolved_app_icon_stays_empty() {
    let hints = notify::Hints {
        image_path: Some(format!("file://{}", blue_svg("path"))),
        ..Default::default()
    };
    let (app, image) = zbus::block_on(notification_icons(
        &hicolor(),
        "",
        &hints,
        String::new(),
        None,
    ));
    assert!(app.is_empty());
    assert_eq!(image.len(), NOTIFY_SIZES.len());
}

#[test]
fn a_jpeg_image_path_is_the_picture() {
    let dir = std::env::temp_dir().join(format!("reprise-notify-jpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cover.jpg");
    image::RgbImage::from_pixel(60, 40, image::Rgb([0xE0, 0x6A, 0x2E]))
        .save_with_format(&file, image::ImageFormat::Jpeg)
        .unwrap();
    let hints = notify::Hints {
        image_path: Some(format!("file://{}", file.display())),
        ..Default::default()
    };
    let (_, image) = zbus::block_on(notification_icons(
        &hicolor(),
        "",
        &hints,
        String::new(),
        None,
    ));
    assert_eq!(image.len(), NOTIFY_SIZES.len());
    let big = image.iter().max_by_key(|s| s.width).unwrap();
    let mid = big.pixels[(big.height / 2 * big.width + big.width / 2) as usize];
    let (r, g, b) = ((mid >> 16) & 0xFF, (mid >> 8) & 0xFF, mid & 0xFF);
    assert!(
        mid >> 24 == 0xFF && r.abs_diff(0xE0) < 8 && g.abs_diff(0x6A) < 8 && b.abs_diff(0x2E) < 8,
        "the cover's orange, opaque: {mid:#010x}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_desktop_entry_icon_stands_in_for_a_missing_app_icon() {
    let entry = reprisefs::apps::AppEntry {
        name: "Mail".into(),
        exec_line: "mail".into(),
        terminal: false,
        id: "mail.desktop".into(),
        icon: Some(blue_svg("entry")),
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: Vec::new(),
        shown: true,
    };
    let hints = notify::Hints::default();
    let (app, image) = zbus::block_on(notification_icons(
        &hicolor(),
        "",
        &hints,
        String::new(),
        Some(&entry),
    ));
    assert_eq!(app.len(), NOTIFY_SIZES.len());
    assert!(image.is_empty());
}

#[test]
fn an_item_name_is_decoded_here_at_the_drawn_sizes() {
    let dir = std::env::temp_dir().join(format!("reprise-tray-named-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("peer.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#00f"/></svg>"##,
    )
    .unwrap();
    let icons: Icons = Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[]))));
    let mut item = TrayItem {
        id: ":1.5/StatusNotifierItem".into(),
        key: "peer".into(),
        service: ":1.5".into(),
        path: "/StatusNotifierItem".into(),
        title: String::new(),
        tooltip: String::new(),
        icon_name: Some("peer".into()),
        icon_theme_path: Some(dir.clone()),
        pixmaps: Arc::from([]),
        named: Arc::from([]),
        menu_path: None,
        passive: false,
        pid: None,
        ..Default::default()
    };
    let drawn = [16, 24, 32, 48];
    let decode = |item: &mut TrayItem| {
        zbus::block_on(decode_names(&icons, IconFiles::Host, item, &drawn));
    };
    decode(&mut item);
    let sizes: Vec<(u32, u32)> = item.named.iter().map(|s| (s.width, s.height)).collect();
    assert_eq!(sizes, [(16, 16), (24, 24), (32, 32), (48, 48)]);
    let at24 = crate::tray::closest(&item.named, 24).unwrap();
    assert_eq!(at24.get(12, 12), 0xFF00_00FF);

    item.attention.name = Some("peer".into());
    item.overlay.name = Some("peer".into());
    decode(&mut item);
    assert_eq!(
        item.attention.named.len(),
        drawn.len(),
        "the attention name decodes too"
    );
    assert_eq!(item.overlay.named.len(), drawn.len(), "and the overlay's");

    item.pixmaps = item_pixmaps(&[(1, 1, &[255u8, 255, 0, 0][..])], &drawn).into();
    decode(&mut item);
    assert!(item.named.is_empty(), "pixmaps stand in for the name");
    item.pixmaps = Arc::from([]);
    item.icon_name = Some("absent".into());
    decode(&mut item);
    assert!(item.named.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A `side` px square pixmap entry of one ARGB colour, as `IconPixmap` sends it.
fn solid(side: i32, argb: u32) -> Vec<u8> {
    argb.to_be_bytes().repeat((side * side) as usize)
}

#[test]
fn a_needs_attention_item_shows_its_attention_icon_with_the_overlay_in_the_corner() {
    let drawn = [16];
    let (red, green, blue) = (
        solid(16, 0xFFFF_0000),
        solid(16, 0xFF00_FF00),
        solid(16, 0xFF00_00FF),
    );
    let set = |px: &[u8]| -> Arc<[IconSource]> { item_pixmaps(&[(16, 16, px)], &drawn).into() };
    let mut item = TrayItem {
        id: ":1.5/StatusNotifierItem".into(),
        pixmaps: set(&red),
        ..Default::default()
    };
    item.attention.pixmaps = set(&green);
    let at = |item: &TrayItem, x, y| crate::tray::icons::item_canvas(item, 16).unwrap().get(x, y);
    assert_eq!(at(&item, 2, 2), 0xFFFF_0000, "Active shows its own icon");
    item.needs_attention = true;
    assert_eq!(
        at(&item, 2, 2),
        0xFF00_FF00,
        "NeedsAttention shows the attention icon"
    );
    item.attention = Default::default();
    assert_eq!(at(&item, 2, 2), 0xFFFF_0000, "with none it keeps its own");

    item.overlay.pixmaps = set(&blue);
    assert_eq!(
        at(&item, 12, 12),
        0xFF00_00FF,
        "the overlay covers the bottom-right quarter"
    );
    assert_eq!(at(&item, 7, 7), 0xFFFF_0000, "and nothing else");
    assert_eq!(at(&item, 12, 2), 0xFFFF_0000);
}

#[test]
fn a_rows_own_children_are_skipped() {
    let nested = vec![child(9, vec![("label", Variant::from("Deeper"))])];
    let props = HashMap::from([("label", Variant::from("Top"))]);
    let row = Variant::from((4i32, props, nested));
    let nodes = parse(&[row]);
    assert_eq!(nodes.len(), 1);
    assert_eq!((nodes[0].id, nodes[0].label.as_str()), (4, "Top"));
}

#[test]
fn parse_layout_skips_malformed_children() {
    assert!(parse(&[Variant::from(42i32), Variant::from("row")]).is_empty());
}

/// The rows `get_layout` reads from a menu that answers `GetLayout` with `children`.
fn layout_served(children: Vec<Variant<'static>>) -> Option<Vec<MenuNode>> {
    zbus::block_on(async {
        let (ours, theirs) = peers(Duration::from_secs(10)).await;
        let mut calls = zbus::MessageStream::from(&theirs);
        let serve = async {
            while let Some(Ok(m)) = calls.next().await {
                let hdr = m.header();
                match hdr.member().map(|m| m.as_str()) {
                    Some("GetLayout") => {
                        let props = HashMap::<&str, Variant>::new();
                        let body = (1u32, (0i32, props, children.clone()));
                        let _ = theirs.reply(&hdr, &body).await;
                    }
                    _ => {
                        let _ = theirs.reply(&hdr, &false).await;
                    }
                }
            }
            std::future::pending().await
        };
        let item = TrayItem {
            service: ":1.1".into(),
            menu_path: Some("/menu".into()),
            ..TrayItem::default()
        };
        futures_lite::future::or(super::menu::get_layout(&ours, &item, 0), serve).await
    })
}

#[test]
fn a_menu_reply_over_the_body_cap_is_refused() {
    let small = |id: i32| {
        let props = HashMap::from([("label", Variant::from("Open"))]);
        Variant::from((id, props, Vec::<Variant>::new()))
    };
    assert_eq!(
        layout_served(vec![small(1)]).map(|rows| rows.len()),
        Some(1)
    );
    let icon = vec![0u8; 4 << 20];
    let props = HashMap::from([
        ("label", Variant::from("Open")),
        ("icon-data", Variant::from(icon)),
    ]);
    let big = Variant::from((2i32, props, Vec::<Variant>::new()));
    assert_eq!(layout_served(vec![small(1), big]), None);
}

/// The member and destination of each signal the loop sends for `cmd`, notification 5 having come
/// from `:1.1`.
fn notification_signals(cmd: TrayCommand) -> Vec<(String, Option<String>)> {
    let (tx, _events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    zbus::block_on(async {
        let (ours, theirs) = peers(Duration::from_secs(10)).await;
        let mut lp = test_loop(ours.clone(), tx.clone(), fd.as_raw_fd());
        let notes = Notifications {
            events: tx,
            eventfd: fd.as_raw_fd(),
            next_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
            senders: lp.senders.clone(),
            programs: Arc::default(),
            icons: hicolor(),
            identities: Arc::default(),
        };
        ours.object_server().at(NOTIFY_PATH, notes).await.unwrap();
        lock(&lp.senders)
            .0
            .insert(5, (":1.1".into(), "mail".into(), None));
        let mut signals = zbus::MessageStream::from(&theirs);
        lp.handle_command(cmd).await;
        let mut out = Vec::new();
        loop {
            let next = async { signals.next().await };
            let quiet = async {
                async_io::Timer::after(Duration::from_millis(300)).await;
                None
            };
            let Some(Ok(m)) = futures_lite::future::or(next, quiet).await else {
                break;
            };
            let hdr = m.header();
            if m.message_type() == zbus::message::Type::Signal {
                out.push((
                    hdr.member().map(|m| m.to_string()).unwrap_or_default(),
                    hdr.destination().map(|d| d.to_string()),
                ));
            }
        }
        out
    })
}

#[test]
fn a_notifications_signals_go_only_to_its_sender() {
    let to = |member: &str| (member.to_string(), Some(":1.1".to_string()));
    let click = TrayCommand::ActionInvoked {
        id: 5,
        key: "default".into(),
        token: Some("token".into()),
    };
    assert_eq!(
        notification_signals(click),
        [to("ActivationToken"), to("ActionInvoked")]
    );
    let reply = TrayCommand::Replied {
        id: 5,
        text: "hi".into(),
    };
    assert_eq!(notification_signals(reply), [to("NotificationReplied")]);
    let closed = TrayCommand::NotificationClosed { id: 5, reason: 2 };
    assert_eq!(notification_signals(closed), [to("NotificationClosed")]);
    let unknown = TrayCommand::ActionInvoked {
        id: 6,
        key: "default".into(),
        token: Some("token".into()),
    };
    assert_eq!(
        notification_signals(unknown),
        [],
        "nothing for a notification with no sender on record"
    );
}

fn test_loop(conn: Connection, events: Sender<TrayEvent>, eventfd: RawFd) -> Loop {
    Loop {
        conn,
        events: events.clone(),
        eventfd,
        items: HashMap::new(),
        fetching: HashMap::new(),
        next_fetch: 0,
        item_stops: HashMap::new(),
        changes: HashMap::new(),
        muted: HashSet::new(),
        muted_for: MUTED_FOR,
        published: None,
        idle_watcher: None,
        screensaver: screensaver::Shared::new(events, eventfd),
        streams: SelectAll::new(),
        appearance: Arc::new(Mutex::new(
            Appearance::of(&crate::config::Config::default()),
        )),
        icons: Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[])))),
        sizes: Arc::new(Mutex::new(vec![16])),
        senders: Arc::default(),
        responding: Arc::default(),
        identities: Arc::default(),
    }
}

async fn peers(timeout: Duration) -> (Connection, Connection) {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let (item_side, our_side) = futures_util::join!(
        zbus::connection::Builder::unix_stream(theirs)
            .server(zbus::Guid::generate())
            .p2p()
            .build(),
        zbus::connection::Builder::unix_stream(ours)
            .p2p()
            .method_timeout(timeout)
            .build(),
    );
    (our_side.unwrap(), item_side.unwrap())
}

/// Serve items on `conn` from `calls`, its message stream: a `Title` read on `/answering` or
/// `/changing` gets "B" and counts in `titles`, any other property read there an error, and
/// calls on any other path no reply at all. `/changing` sends `NewTitle` right after its first
/// `Title` read. The stream must exist before the calls arrive, or it never sees them.
async fn serve_items(conn: Connection, mut calls: zbus::MessageStream, titles: &AtomicUsize) {
    while let Some(Ok(msg)) = calls.next().await {
        let hdr = msg.header();
        let Some(path) = hdr.path().map(|p| p.as_str().to_string()) else {
            continue;
        };
        if path != "/answering" && path != "/changing" {
            continue;
        }
        let Ok((_, name)) = msg.body().deserialize::<(String, String)>() else {
            continue;
        };
        if name == "Title" {
            let first = titles.fetch_add(1, Ordering::Relaxed) == 0;
            let _ = conn.reply(&hdr, &Variant::from("B")).await;
            if first && path == "/changing" {
                let changed = zbus::message::Message::signal("/changing", ITEM_IFACE, "NewTitle")
                    .sender(":1.1")
                    .build(&())
                    .unwrap();
                let _ = conn.send(&changed).await;
            }
        } else {
            let err = "org.freedesktop.DBus.Error.UnknownProperty";
            let _ = conn.reply_error(&hdr, err, &name).await;
        }
    }
}

struct Harness {
    wakes: async_channel::Sender<Wake>,
    events: Receiver<TrayEvent>,
    titles: AtomicUsize,
    item: Connection,
}

fn drive<T>(script: impl AsyncFnOnce(&Harness) -> T) -> T {
    let (tx, events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    zbus::block_on(async {
        let (ours, item) = peers(Duration::from_secs(10)).await;
        let calls = zbus::MessageStream::from(&item);
        let mut lp = test_loop(ours, tx, fd.as_raw_fd());
        let (wakes, wake_rx) = async_channel::unbounded();
        lp.streams.push(wake_rx.boxed());
        let h = Harness {
            wakes,
            events,
            titles: AtomicUsize::new(0),
            item,
        };
        let script = async {
            let out = script(&h).await;
            let _ = h.wakes.send(Wake::Command(TrayCommand::Shutdown)).await;
            out
        };
        let work = async { futures_util::join!(script, lp.run()).0 };
        let serve = async {
            serve_items(h.item.clone(), calls, &h.titles).await;
            std::future::pending().await
        };
        futures_lite::future::or(work, serve).await
    })
}

async fn register(wakes: &async_channel::Sender<Wake>, path: &str) {
    let bus = ":1.1".to_string();
    let path = path.to_string();
    wakes.send(Wake::Registered { bus, path }).await.unwrap();
}

async fn next_event(events: &Receiver<TrayEvent>, within: Duration) -> Option<TrayEvent> {
    let tick = Duration::from_millis(10);
    for _ in 0..within.as_millis() / tick.as_millis() {
        if let Ok(ev) = events.try_recv() {
            return Some(ev);
        }
        async_io::Timer::after(tick).await;
    }
    None
}

/// An item that never answers holds up only its own read: one registered after it is added
/// within 2 s, while calls may take 10 s.
#[test]
fn a_silent_item_does_not_hold_up_the_next() {
    let added = drive(async |h| {
        register(&h.wakes, "/silent").await;
        register(&h.wakes, "/answering").await;
        match next_event(&h.events, Duration::from_secs(2)).await {
            Some(TrayEvent::Added(item)) => Some(item.id),
            _ => None,
        }
    });
    assert_eq!(added.as_deref(), Some(":1.1/answering"));
}

/// zbus stops reading the socket while any match queue (64 deep) is full, and `add_match`
/// waits on the reader's lock: a queue the loop has not drained must not stall the next watch.
#[test]
fn a_signal_flood_before_the_loop_drains_it_does_not_stall_the_connection() {
    let (tx, events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let added = zbus::block_on(async {
        let (ours, item) = peers(Duration::from_secs(10)).await;
        let calls = zbus::MessageStream::from(&item);
        let titles = AtomicUsize::new(0);
        let mut lp = test_loop(ours, tx, fd.as_raw_fd());
        lp.watch_item(":1.1", "/chatty").await;
        for _ in 0..200 {
            let changed = zbus::message::Message::signal("/chatty", ITEM_IFACE, "NewTitle")
                .sender(":1.1")
                .build(&())
                .unwrap();
            item.send(&changed).await.unwrap();
        }
        async_io::Timer::after(Duration::from_millis(200)).await;
        let (wakes, wake_rx) = async_channel::unbounded();
        lp.streams.push(wake_rx.boxed());
        register(&wakes, "/answering").await;
        let script = async {
            loop {
                match next_event(&events, Duration::from_secs(3)).await {
                    Some(TrayEvent::Added(item)) if item.path == "/answering" => {
                        return Some(item.id);
                    }
                    Some(_) => {}
                    None => return None,
                }
            }
        };
        let run = async {
            lp.run().await;
            std::future::pending().await
        };
        let serve = async {
            serve_items(item.clone(), calls, &titles).await;
            std::future::pending().await
        };
        futures_lite::future::or(script, futures_lite::future::or(run, serve)).await
    });
    assert_eq!(added.as_deref(), Some(":1.1/answering"));
}

/// A bus call holds zbus's write lock until its message is out, so a read the loop started
/// must run while the loop awaits a call of its own, not only when the loop polls its streams.
#[test]
fn an_item_read_reaches_the_item_while_the_loop_is_busy() {
    let (tx, _events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let titles = AtomicUsize::new(0);
    zbus::block_on(async {
        let (ours, item) = peers(Duration::from_secs(10)).await;
        let calls = zbus::MessageStream::from(&item);
        let mut lp = test_loop(ours, tx, fd.as_raw_fd());
        lp.fetch(":1.1".into(), "/answering".into(), Duration::ZERO, false);
        let busy = async {
            for _ in 0..200 {
                if titles.load(Ordering::Relaxed) > 0 {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(10)).await;
            }
        };
        let serve = async {
            serve_items(item.clone(), calls, &titles).await;
            std::future::pending().await
        };
        futures_lite::future::or(busy, serve).await;
    });
    assert_eq!(
        titles.load(Ordering::Relaxed),
        1,
        "the Title read reached the item"
    );
}

#[test]
fn a_burst_of_changes_reads_the_item_once() {
    let (reads, updates) = drive(async |h| {
        register(&h.wakes, "/answering").await;
        let added = next_event(&h.events, Duration::from_secs(2)).await;
        assert!(
            matches!(added, Some(TrayEvent::Added(_))),
            "the item is added"
        );
        for _ in 0..10 {
            let key = ":1.1/answering".to_string();
            h.wakes.send(Wake::ItemChanged(key)).await.unwrap();
        }
        async_io::Timer::after(Duration::from_millis(300)).await;
        let updates = std::iter::from_fn(|| h.events.try_recv().ok()).count();
        (h.titles.load(Ordering::Relaxed), updates)
    });
    assert_eq!(reads, 2, "one read adds the item and one follows the burst");
    assert_eq!(updates, 0, "an unchanged item sends no update");
}

#[test]
fn a_change_during_the_first_read_reads_the_item_again() {
    let reads = drive(async |h| {
        register(&h.wakes, "/changing").await;
        let added = next_event(&h.events, Duration::from_secs(2)).await;
        assert!(
            matches!(added, Some(TrayEvent::Added(_))),
            "the item is added"
        );
        async_io::Timer::after(Duration::from_millis(300)).await;
        h.titles.load(Ordering::Relaxed)
    });
    assert_eq!(
        reads, 2,
        "the NewTitle sent during the first read gets a second read"
    );
}

#[test]
fn new_icon_sizes_read_every_item_again() {
    let reads = drive(async |h| {
        register(&h.wakes, "/answering").await;
        let added = next_event(&h.events, Duration::from_secs(2)).await;
        assert!(
            matches!(added, Some(TrayEvent::Added(_))),
            "the item is added"
        );
        let sizes = TrayCommand::IconSizes(vec![16, 24]);
        h.wakes.send(Wake::Command(sizes)).await.unwrap();
        async_io::Timer::after(Duration::from_millis(300)).await;
        h.titles.load(Ordering::Relaxed)
    });
    assert_eq!(reads, 2, "a second Title read after the sizes changed");
}

#[test]
fn every_change_signal_reads_the_item_again() {
    let reads = drive(async |h| {
        register(&h.wakes, "/answering").await;
        let added = next_event(&h.events, Duration::from_secs(2)).await;
        assert!(
            matches!(added, Some(TrayEvent::Added(_))),
            "the item is added"
        );
        let mut reads = Vec::new();
        for signal in ["NewIcon", "NewTitle", "NewStatus", "NewToolTip", "NewMenu"] {
            let msg = zbus::message::Message::signal("/answering", ITEM_IFACE, signal)
                .sender(":1.1")
                .build(&())
                .unwrap();
            h.item.send(&msg).await.unwrap();
            async_io::Timer::after(Duration::from_millis(200)).await;
            reads.push((signal, h.titles.load(Ordering::Relaxed)));
        }
        reads
    });
    let signals = ["NewIcon", "NewTitle", "NewStatus", "NewToolTip", "NewMenu"];
    let want: Vec<(&str, usize)> = signals.into_iter().zip(2..).collect();
    assert_eq!(reads, want, "one more Title read after each signal");
}

#[test]
fn a_fresh_id_skips_the_compositors_own_and_open_ones() {
    let mut senders = Senders::default();
    let mut ids = [7, u32::MAX, 0, 7, 8].into_iter();
    let mut fresh = || ids.next().unwrap();
    assert_eq!(senders.claim(":1.7", None, "a", 0, false, &mut fresh), 7);
    assert_eq!(senders.claim(":1.8", None, "b", 0, false, &mut fresh), 8);
}

#[test]
fn only_its_sender_replaces_or_closes_a_balloon() {
    let mut senders = Senders::default();
    let mut next = 1;
    let mut fresh = || {
        next += 1;
        next - 1
    };
    let a = senders.claim(":1.7", Some(7), "mail", 0, false, &mut fresh);
    assert_eq!(a, 1, "a new balloon gets a fresh id");
    assert_eq!(
        senders.claim(":1.7", Some(7), "mail", a, false, &mut fresh),
        a,
        "its sender replaces it"
    );
    assert_eq!(
        senders.claim(":1.8", Some(8), "chat", a, true, &mut fresh),
        2,
        "another peer gets a fresh id"
    );
    assert_eq!(
        senders.claim(":1.8", Some(8), "", a, true, &mut fresh),
        3,
        "a peer with no app name gets a fresh id"
    );
    assert!(senders.owns(a, ":1.7"));
    assert!(!senders.owns(a, ":1.8"), "another peer may not close it");
    senders.0.remove(&a);
    assert_eq!(
        senders.claim(":1.7", Some(7), "mail", a, false, &mut fresh),
        4,
        "a closed id is not reused"
    );
}

#[test]
fn another_connection_under_the_same_app_name_gets_a_fresh_id() {
    let mut senders = Senders::default();
    let mut next = 1;
    let mut fresh = || {
        next += 1;
        next - 1
    };
    let a = senders.claim(":1.7", Some(7), "mail", 0, false, &mut fresh);
    assert_eq!(
        senders.other_sender(a, ":1.8", Some(8), "mail").as_deref(),
        Some(":1.7")
    );
    assert_eq!(
        senders.claim(":1.8", Some(8), "mail", a, false, &mut fresh),
        2
    );
    assert!(
        senders.owns(a, ":1.7"),
        "the first sender keeps its balloon"
    );
    assert_eq!(senders.other_sender(a, ":1.9", Some(7), "mail"), None);
    assert_eq!(
        senders.claim(":1.9", Some(7), "mail", a, false, &mut fresh),
        a,
        "another connection of its process replaces it"
    );
    assert!(
        senders.owns(a, ":1.9"),
        "and its signals go to the replacing connection"
    );
}

#[test]
fn a_new_process_under_the_same_app_name_replaces_a_balloon_whose_sender_left() {
    let mut senders = Senders::default();
    let mut next = 1;
    let mut fresh = || {
        next += 1;
        next - 1
    };
    let a = senders.claim(":1.7", Some(7), "notify-send", 0, false, &mut fresh);
    assert_eq!(
        senders.claim(":1.9", Some(9), "notify-send", a, true, &mut fresh),
        a
    );
    assert!(senders.owns(a, ":1.9"), "the new sender is on record");
    assert!(!senders.owns(a, ":1.7"));
}

#[test]
fn a_registration_takes_a_path_a_bus_or_both() {
    let own = |bus: &str, path: &str| Some((bus.to_string(), path.to_string()));
    let reg = |service| resolve_registration(service, Some(":1.5"));
    assert_eq!(reg("/org/x"), own(":1.5", "/org/x"));
    assert_eq!(reg("org.x.App"), own("org.x.App", "/StatusNotifierItem"));
    let both = ":1.42/org/ayatana/NotificationItem/x";
    assert_eq!(reg(both), own(":1.42", "/org/ayatana/NotificationItem/x"));
    assert_eq!(reg(""), None);
}

fn published(sender: &str, n: usize) -> Vec<Published> {
    (0..n)
        .map(|i| Published {
            key: format!("{sender}/item{i}"),
            sender: sender.to_string(),
        })
        .collect()
}

#[test]
fn admit_caps_one_sender_and_lets_another_register() {
    let mut items = published(":1.7", MAX_REGISTRATIONS_PER_SENDER);
    assert_eq!(admit(&items, ":1.7/extra", ":1.7"), Admission::SenderFull);
    assert_eq!(admit(&items, ":1.7/item0", ":1.7"), Admission::Known);
    assert_eq!(admit(&items, ":1.8/item0", ":1.8"), Admission::New);
    items.retain(|p| p.key != ":1.7/item3");
    assert_eq!(admit(&items, ":1.7/extra", ":1.7"), Admission::New);
}

#[test]
fn admit_caps_the_total_across_senders() {
    let items: Vec<Published> = (0..MAX_REGISTRATIONS / MAX_REGISTRATIONS_PER_SENDER)
        .flat_map(|i| published(&format!(":1.{i}"), MAX_REGISTRATIONS_PER_SENDER))
        .collect();
    assert_eq!(items.len(), MAX_REGISTRATIONS);
    assert_eq!(admit(&items, ":1.99/item0", ":1.99"), Admission::TotalFull);
}

#[test]
fn the_server_advertises_action_icons() {
    assert!(CAPABILITIES.contains(&"action-icons"));
}

#[test]
fn an_action_key_naming_no_icon_keeps_its_label() {
    let actions = [("reprise-no-such-icon".to_string(), "Stop".to_string())];
    assert!(action_icons(&hicolor(), &actions).is_empty());
}

#[test]
fn an_items_overlay_pixmap_is_read_with_its_other_properties() {
    let got = zbus::block_on(async {
        let (ours, item) = peers(Duration::from_secs(5)).await;
        let mut calls = zbus::MessageStream::from(&item);
        let serve = async {
            while let Some(Ok(msg)) = calls.next().await {
                let hdr = msg.header();
                let Ok((_, name)) = msg.body().deserialize::<(String, String)>() else {
                    continue;
                };
                let _ = match name.as_str() {
                    "Title" => item.reply(&hdr, &Variant::from("B")).await,
                    "OverlayIconPixmap" => {
                        let badge = vec![(16, 16, vec![255u8; 16 * 16 * 4])];
                        item.reply(&hdr, &Variant::from(badge)).await
                    }
                    _ => {
                        let err = "org.freedesktop.DBus.Error.UnknownProperty";
                        item.reply_error(&hdr, err, &name).await
                    }
                };
            }
            None
        };
        let icons: Icons = Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[]))));
        let sizes: Sizes = Arc::new(Mutex::new(vec![16]));
        let identities = Identities::default();
        let fetch = async {
            let read = fetch_item(&ours, &icons, &sizes, &identities, ":1.1", "/item").await;
            read.map(|r| r.0)
        };
        futures_lite::future::or(fetch, serve).await
    });
    let item = got.expect("the item is read");
    assert_eq!(item.overlay.pixmaps.len(), 1, "the badge is decoded");
    assert_eq!(item.overlay.pixmaps[0].width, 16);
}

#[test]
fn the_session_programs_keep_first_order_and_take_the_latest_name() {
    let mut p = Programs::default();
    p.saw("org.example.Mail", "Mail");
    p.saw("", "Nameless");
    p.saw("chat", "Chat");
    p.saw("org.example.Mail", "Mail 2");
    assert_eq!(
        p.0,
        [
            ("org.example.Mail".to_string(), "Mail 2".to_string()),
            ("chat".to_string(), "Chat".to_string()),
        ]
    );
}

#[test]
fn the_ping_round_runs_from_the_first_watcher_to_the_last_leaving() {
    let mut r = Responding::default();
    assert!(r.watch(":1.7"), "the first watcher starts it");
    assert!(!r.watch(":1.7"), "a second call is no new watcher");
    assert!(!r.watch(":1.8"));
    assert!(!r.left(":1.9"), "someone else leaving changes nothing");
    assert!(!r.left(":1.7"), "one watcher is left");
    assert!(r.left(":1.8"), "the last leaving stops it");
}

#[test]
fn a_session_deferring_to_another_takes_only_free_names_and_replaces_none() {
    use zbus::fdo::RequestNameFlags;
    let deferring = name_flags(true);
    assert!(deferring.contains(RequestNameFlags::DO_NOT_QUEUE));
    assert!(!deferring.contains(RequestNameFlags::REPLACE_EXISTING));
    assert!(!deferring.contains(RequestNameFlags::ALLOW_REPLACEMENT));
    assert_eq!(name_flags(false), RequestNameFlags::default());
}

/// A loop on a private bus hosting the screensaver as the compositor does, and its connection.
async fn screensaver_on(bus: &str) -> (Loop, Connection, OwnedFd) {
    let (tx, _events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let conn = zbus::connection::Builder::address(bus)
        .build()
        .await
        .unwrap();
    let lp = test_loop(conn.clone(), tx, fd.as_raw_fd());
    host_screensaver(&conn, &lp.screensaver, false)
        .await
        .unwrap();
    (lp, conn, fd)
}

/// Whether `conn` sees `org.freedesktop.ScreenSaver` owned by `want` within 5 s.
async fn screensaver_owned_by(conn: &Connection, want: &Connection) -> bool {
    let want = want.unique_name().unwrap().to_string();
    for _ in 0..100 {
        let owner = conn
            .call_method(
                Some(DBUS_IFACE),
                "/org/freedesktop/DBus",
                Some(DBUS_IFACE),
                "GetNameOwner",
                &(SCREENSAVER_NAME,),
            )
            .await
            .and_then(|m| m.body().deserialize::<String>());
        if owner.as_deref() == Ok(want.as_str()) {
            return true;
        }
        async_io::Timer::after(Duration::from_millis(50)).await;
    }
    false
}

#[test]
fn a_later_session_takes_the_screensaver_name_and_hands_it_back_when_it_leaves() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let (_first_lp, first, _first_fd) = screensaver_on(&bus.1).await;
        let (_second_lp, second, _second_fd) = screensaver_on(&bus.1).await;
        assert!(
            screensaver_owned_by(&first, &second).await,
            "the later session"
        );
        second.release_name(SCREENSAVER_NAME).await.unwrap();
        assert!(
            screensaver_owned_by(&second, &first).await,
            "the earlier session again"
        );
    });
}

#[test]
fn a_session_gets_the_screensaver_name_once_a_holder_allowing_no_replacement_leaves() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let holder = zbus::connection::Builder::address(bus.1.as_str())
            .build()
            .await
            .unwrap();
        own(&holder, SCREENSAVER_NAME, name_flags(true))
            .await
            .unwrap();
        let (_lp, ours, _fd) = screensaver_on(&bus.1).await;
        assert!(screensaver_owned_by(&ours, &holder).await);
        holder.release_name(SCREENSAVER_NAME).await.unwrap();
        assert!(screensaver_owned_by(&holder, &ours).await, "queued for it");
    });
}

#[test]
fn get_active_and_active_changed_follow_the_desktops_lock() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let (mut lp, ours, _fd) = screensaver_on(&bus.1).await;
        let other = zbus::connection::Builder::address(bus.1.as_str())
            .build()
            .await
            .unwrap();
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(SCREENSAVER_NAME)
            .member("ActiveChanged")
            .build()
            .unwrap();
        let mut changes = zbus::MessageStream::for_match_rule(rule, &other, None)
            .await
            .unwrap();
        lp.handle_command(TrayCommand::Locked(true)).await;
        let mut heard = Vec::new();
        for _ in SCREENSAVER_PATHS {
            let next = futures_lite::future::or(async { changes.next().await }, async {
                async_io::Timer::after(Duration::from_secs(5)).await;
                None
            });
            let Some(Ok(m)) = next.await else {
                break;
            };
            let path = m.header().path().map(|p| p.to_string());
            heard.push((path, m.body().deserialize::<bool>().unwrap()));
        }
        heard.sort();
        let mut want: Vec<_> = (SCREENSAVER_PATHS.iter())
            .map(|p| (Some(p.to_string()), true))
            .collect();
        want.sort();
        assert_eq!(heard, want, "ActiveChanged(true) on each path");
        for path in SCREENSAVER_PATHS {
            let reply = other
                .call_method(
                    Some(ours.unique_name().unwrap().as_str()),
                    path,
                    Some(SCREENSAVER_NAME),
                    "GetActive",
                    &(),
                )
                .await
                .unwrap();
            assert!(reply.body().deserialize::<bool>().unwrap(), "{path}");
        }
    });
}

#[test]
fn get_active_reads_locked_before_the_first_lock_report() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let (_lp, ours, _fd) = screensaver_on(&bus.1).await;
        let other = zbus::connection::Builder::address(bus.1.as_str())
            .build()
            .await
            .unwrap();
        let reply = other
            .call_method(
                Some(ours.unique_name().unwrap().as_str()),
                SCREENSAVER_PATHS[0],
                Some(SCREENSAVER_NAME),
                "GetActive",
                &(),
            )
            .await
            .unwrap();
        assert!(reply.body().deserialize::<bool>().unwrap());
    });
}

async fn owner_of(conn: &Connection, name: &str) -> Option<String> {
    let reply = conn
        .call_method(
            Some(DBUS_IFACE),
            "/org/freedesktop/DBus",
            Some(DBUS_IFACE),
            "GetNameOwner",
            &(name,),
        )
        .await
        .ok()?;
    reply.body().deserialize().ok()
}

#[test]
fn a_session_taking_its_names_back_takes_only_free_ones_and_a_later_session_replaces_them() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let connect = || zbus::connection::Builder::address(bus.1.as_str()).build();
        let ours = connect().await.unwrap();
        let newer = connect().await.unwrap();
        own(&newer, NOTIFY_NAME, name_flags(false)).await.unwrap();
        let (tx, _events) = std::sync::mpsc::channel();
        let mut lp = test_loop(ours.clone(), tx, -1);
        lp.retake_names().await;
        let us = ours.unique_name().map(|n| n.to_string());
        assert_eq!(
            owner_of(&ours, SETTINGS_NAME).await,
            us,
            "a name nobody held"
        );
        assert_eq!(owner_of(&ours, SCREENSAVER_NAME).await, us);
        assert_eq!(
            owner_of(&ours, NOTIFY_NAME).await,
            newer.unique_name().map(|n| n.to_string()),
            "a session started meanwhile keeps its own"
        );
        let later = connect().await.unwrap();
        own(&later, SETTINGS_NAME, name_flags(false)).await.unwrap();
        own(&later, SCREENSAVER_NAME, screensaver_flags(false))
            .await
            .unwrap();
        later.release_name(SCREENSAVER_NAME).await.unwrap();
        assert!(
            screensaver_owned_by(&ours, &ours).await,
            "queued for the screensaver name, it gets it back"
        );
    });
}

#[test]
fn a_session_that_joined_another_watcher_hosts_its_own_once_it_takes_the_name_back() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    zbus::block_on(async {
        let connect = || zbus::connection::Builder::address(bus.1.as_str()).build();
        let ours = connect().await.unwrap();
        let (registrations, registered) = async_channel::unbounded();
        let items: WatcherItems = Arc::default();
        let watcher = Watcher {
            items: items.clone(),
            registrations,
        };
        ours.object_server()
            .at(WATCHER_PATH, watcher)
            .await
            .unwrap();
        let (tx, _events) = std::sync::mpsc::channel();
        let mut lp = test_loop(ours.clone(), tx, -1);
        lp.idle_watcher = Some((items, registered));
        lp.retake_names().await;
        let app = connect().await.unwrap();
        app.call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some(WATCHER_IFACE),
            "RegisterStatusNotifierItem",
            &("/StatusNotifierItem",),
        )
        .await
        .unwrap();
        let app_name = app.unique_name().unwrap().to_string();
        match lp.streams.next().await {
            Some(Wake::Registered { bus, path }) => {
                assert_eq!(
                    (bus, path),
                    (app_name.clone(), "/StatusNotifierItem".into())
                )
            }
            _ => panic!("the registration reaches the loop"),
        }
        let listed: Vec<String> = Proxy::new(&app, WATCHER_NAME, WATCHER_PATH, WATCHER_IFACE)
            .await
            .unwrap()
            .get_property("RegisteredStatusNotifierItems")
            .await
            .unwrap();
        assert_eq!(listed, [format!("{app_name}/StatusNotifierItem")]);
    });
}

/// A `proc` folder in which this process shows Flatpak's `/.flatpak-info` for `app_id`.
fn flatpak_proc(name: &str, app_id: &str) -> std::path::PathBuf {
    let proc = crate::harness::scratch().join(format!("proc-{name}"));
    let dir = proc.join(std::process::id().to_string());
    std::fs::create_dir_all(dir.join("root")).unwrap();
    let info = format!("[Application]\nname={app_id}\n");
    std::fs::write(dir.join("root/.flatpak-info"), info).unwrap();
    std::fs::write(dir.join("stat"), "42 (app) S 1 42\n").unwrap();
    proc
}

/// [`flatpak_proc`] in which this process is also its sandbox's first process, as instance 7,
/// with a runtime folder holding Flatpak's record of it.
fn flatpak_sandbox(name: &str, app_id: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let proc = flatpak_proc(name, app_id);
    let dir = proc.join(std::process::id().to_string());
    let info = format!("[Application]\nname={app_id}\n\n[Instance]\ninstance-id=7\n");
    std::fs::write(dir.join("root/.flatpak-info"), info).unwrap();
    for ns in [proc.join("self/ns"), dir.join("ns")] {
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("mnt"), "").unwrap();
    }
    let ns = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(dir.join("ns/mnt")).unwrap());
    let runtime = crate::harness::scratch().join(format!("runtime-{name}"));
    std::fs::create_dir_all(runtime.join(".flatpak/7")).unwrap();
    let pid = std::process::id();
    let json = format!("{{\"child-pid\": {pid}, \"mnt-namespace\": {ns}}}");
    std::fs::write(runtime.join(".flatpak/7/bwrapinfo.json"), json).unwrap();
    (proc, runtime)
}

/// What a server reading its senders from `proc` (`/proc` when None) shows of a notification a
/// peer on `bus` sends as `app_name` with the `desktop-entry` hint `entry`.
fn notified(
    bus: &crate::testbus::Bus,
    proc: Option<&std::path::Path>,
    app_name: &str,
    entry: &str,
) -> Notification {
    let (tx, events) = std::sync::mpsc::channel();
    // SAFETY: eventfd takes no pointers; the result is checked below.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd failed");
    // SAFETY: `fd` was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let identities = proc.map_or_else(Identities::default, Identities::under);
    zbus::block_on(async {
        let connect = || zbus::connection::Builder::address(bus.1.as_str());
        let client = connect().build().await.unwrap();
        let notes = Notifications {
            events: tx,
            eventfd: fd.as_raw_fd(),
            next_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
            senders: Arc::default(),
            programs: Arc::default(),
            icons: hicolor(),
            identities: Arc::new(identities),
        };
        let serving = connect().serve_at(NOTIFY_PATH, notes);
        let server = serving.build().await.unwrap();
        let hints = HashMap::from([("desktop-entry", Variant::from(entry))]);
        let body = (
            app_name,
            0u32,
            "",
            "Hi",
            "",
            Vec::<&str>::new(),
            hints,
            -1i32,
        );
        let to = server.unique_name().unwrap().to_owned();
        let call = client.call_method(Some(to), NOTIFY_PATH, Some(NOTIFY_NAME), "Notify", &body);
        call.await.unwrap();
    });
    match events.recv_timeout(Duration::from_secs(5)) {
        Ok(TrayEvent::Notify(n)) => n,
        _ => panic!("no notification shown"),
    }
}

#[test]
fn a_sandboxed_senders_notification_goes_under_its_own_app() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let proc = flatpak_proc("notify", "org.example.Spoofer");
    let n = notified(&bus, Some(&proc), "Firefox", "firefox");
    assert_eq!(n.app, "org.example.Spoofer");
    assert_eq!(n.key, "org.example.Spoofer");
}

#[test]
fn a_host_senders_notification_keeps_the_app_it_names() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    // The notification portal forwards a sandboxed app's notification this way: as a host
    // program naming the app id it checked.
    let n = notified(&bus, None, "org.example.Chat", "org.example.Chat");
    assert_eq!(n.app, "org.example.Chat");
    assert_eq!(n.key, "org.example.Chat");
}

/// A tray item claiming to be another app's.
struct Spoofing;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Spoofing {
    #[zbus(property)]
    fn id(&self) -> String {
        "org.mozilla.firefox".into()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "Firefox".into()
    }
}

/// [`Spoofing`] served by a peer on `bus`, as read by a host reading its owner from `proc`
/// (`/proc` when None).
fn spoofing_item(bus: &crate::testbus::Bus, proc: Option<&std::path::Path>) -> TrayItem {
    let identities = proc.map_or_else(Identities::default, Identities::under);
    served_item(bus, identities, Spoofing)
}

/// `item` served by a peer on `bus`, as read by a host placing its owner through `identities`.
fn served_item(
    bus: &crate::testbus::Bus,
    identities: Identities,
    item: impl zbus::object_server::Interface,
) -> TrayItem {
    zbus::block_on(async {
        let connect = || zbus::connection::Builder::address(bus.1.as_str());
        // Served from the start: an object added once the connection runs may miss a call that
        // arrives first.
        let theirs = connect().serve_at("/item", item).build().await.unwrap();
        let ours = connect()
            .method_timeout(METHOD_TIMEOUT)
            .build()
            .await
            .unwrap();
        let icons: Icons = Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[]))));
        let sizes: Sizes = Arc::new(Mutex::new(vec![16]));
        let owner = theirs.unique_name().unwrap().to_string();
        let read = fetch_item(&ours, &icons, &sizes, &identities, &owner, "/item").await;
        read.map(|r| r.0)
    })
    .expect("the item is read")
}

#[test]
fn a_sandboxed_apps_tray_icon_is_kept_and_listed_under_its_own_app() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let proc = flatpak_proc("item", "org.example.Spoofer");
    let item = spoofing_item(&bus, Some(&proc));
    assert_eq!(item.key, "org.example.Spoofer");
    assert_eq!(item.title, "org.example.Spoofer");
    assert_eq!(item.tooltip, "org.example.Spoofer");
    let host = spoofing_item(&bus, None);
    assert_eq!(
        (host.key.as_str(), host.title.as_str()),
        ("org.mozilla.firefox", "Firefox")
    );
}

#[test]
fn a_sandboxed_or_unknown_sender_names_no_file_to_read() {
    let sound = |s: &str| Sound::File(s.into());
    let sent = |identity: &AppIdentity, image: &str, sound: Option<Sound>| {
        let mut hints = notify::Hints {
            image_path: Some(image.into()),
            sound,
            ..notify::Hints::default()
        };
        as_sent_by(identity, &mut hints, "App".into(), String::new());
        (hints.image_path, hints.sound)
    };
    let app = AppIdentity::Sandboxed {
        app_id: "org.example.App".into(),
    };
    for identity in [&app, &AppIdentity::Unknown] {
        for path in [
            "/home/u/Pictures/x.png",
            "file:///home/u/x.png",
            "x/../y.png",
        ] {
            let got = sent(identity, path, Some(sound("/home/u/a.oga")));
            assert_eq!(got, (None, None), "{identity:?} naming {path}");
        }
        let named = sent(identity, "mail-unread", Some(Sound::Name("bell".into())));
        assert_eq!(
            named,
            (Some("mail-unread".into()), Some(Sound::Name("bell".into())))
        );
    }
    let host = AppIdentity::Host { desktop_id: None };
    let got = sent(&host, "/home/u/x.png", Some(sound("/home/u/a.oga")));
    assert_eq!(
        got,
        (Some("/home/u/x.png".into()), Some(sound("/home/u/a.oga")))
    );
}

/// A tray item naming its icon by `icon` (a path or a name) under `theme_path`.
struct Naming {
    icon: String,
    theme_path: String,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Naming {
    #[zbus(property)]
    fn id(&self) -> String {
        "naming".into()
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon.clone()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }
}

/// An 8 px square SVG of one colour, `#rrggbb`.
fn write_svg(path: &std::path::Path, fill: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="{fill}"/></svg>"#
    );
    std::fs::write(path, svg).unwrap();
}

/// `icon` under `theme_path` as read from a peer placed through `identities`, at 16 px: the
/// colour at its centre, None when nothing decoded.
fn named_colour(
    bus: &crate::testbus::Bus,
    identities: Identities,
    icon: &str,
    theme_path: &str,
) -> (Option<u32>, TrayItem) {
    let naming = Naming {
        icon: icon.into(),
        theme_path: theme_path.into(),
    };
    let item = served_item(bus, identities, naming);
    let colour = crate::tray::closest(&item.named, 16).map(|c| c.get(8, 8));
    (colour, item)
}

const BLUE: u32 = 0xFF00_00FF;

#[test]
fn a_flatpak_items_icon_path_opens_inside_its_sandbox() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let (proc, runtime) = flatpak_sandbox("icon-root", "org.example.App");
    let ids = || Identities::under(&proc).with_runtime_dir(&runtime);
    let root = proc.join(std::process::id().to_string()).join("root");
    let icons = root.join("app/share/icons");
    write_svg(
        &icons.join("hicolor/16x16/apps/org.example.App.svg"),
        "#0000ff",
    );
    let path = "/app/share/icons/hicolor/16x16/apps/org.example.App.svg";
    let (colour, item) = named_colour(&bus, ids(), path, "");
    assert_eq!(colour, Some(BLUE), "{path} in the sandbox");
    assert_eq!(
        item.saved_icon, None,
        "Customize never keeps a sandbox's path"
    );
    let (colour, _) = named_colour(&bus, ids(), "org.example.App", "/app/share/icons");
    assert_eq!(
        colour,
        Some(BLUE),
        "a name under IconThemePath in the sandbox"
    );
    let on_host = icons.join("hicolor/16x16/apps/org.example.App.svg");
    let (colour, _) = named_colour(&bus, ids(), on_host.to_str().unwrap(), "");
    assert_eq!(colour, None, "a host path is not the sandbox's");
}

#[test]
fn an_unknown_or_snapped_items_host_path_is_dropped() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let host = crate::harness::scratch().join("icon-host");
    write_svg(&host.join("hicolor/16x16/apps/x.svg"), "#0000ff");
    let file = host.join("hicolor/16x16/apps/x.svg");
    let (file, dir) = (file.to_str().unwrap(), host.to_str().unwrap());
    let (colour, item) = named_colour(&bus, Identities::default(), file, "");
    assert_eq!(colour, Some(BLUE), "a host item's path opens");
    assert_eq!(item.saved_icon.as_deref(), Some(file));
    assert_eq!(
        named_colour(&bus, Identities::default(), "x", dir).0,
        Some(BLUE)
    );

    let unknown = crate::harness::scratch().join("proc-unknown");
    std::fs::create_dir_all(&unknown).unwrap();
    let snap = crate::harness::scratch().join("proc-snap");
    let at = snap.join(std::process::id().to_string());
    std::fs::create_dir_all(at.join("attr/apparmor")).unwrap();
    std::fs::write(
        at.join("attr/apparmor/current"),
        "snap.chat.chat (enforce)\n",
    )
    .unwrap();
    std::fs::write(at.join("stat"), "42 (chat) S 1 42\n").unwrap();
    for proc in [&unknown, &snap] {
        let (colour, item) = named_colour(&bus, Identities::under(proc), file, "");
        assert_eq!(colour, None, "{proc:?} naming {file}");
        assert_eq!(item.saved_icon, None);
        assert_eq!(
            named_colour(&bus, Identities::under(proc), "x", dir).0,
            None
        );
    }
}

fn decoders(threads: usize, queue: usize, per_app: usize, within: Duration) -> decoders::Decoders {
    decoders::Decoders::new(decoders::Limits {
        threads,
        queue,
        per_app,
        wait: within,
        within,
    })
}

#[test]
fn a_flood_of_icon_decodes_runs_at_most_two_at_once() {
    use std::sync::atomic::Ordering::SeqCst;
    let decoders = decoders(2, 64, 4, Duration::from_secs(30));
    let (live, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let runs = (0..32).map(|i| {
        let (live, most) = (live.clone(), most.clone());
        let decoders = &decoders;
        async move {
            let decode = move || {
                most.fetch_max(live.fetch_add(1, SeqCst) + 1, SeqCst);
                std::thread::sleep(Duration::from_millis(5));
                live.fetch_sub(1, SeqCst);
            };
            decoders.run(&format!("app{i}"), decode).await
        }
    });
    let done = zbus::block_on(futures_util::future::join_all(runs));
    assert!(
        done.iter().all(Option::is_some),
        "every queued decode finishes"
    );
    assert!(
        most.load(SeqCst) <= 2,
        "{} decodes ran at once",
        most.load(SeqCst)
    );
}

#[test]
fn hung_icon_reads_hold_their_threads_and_later_decodes_give_up() {
    use std::sync::atomic::Ordering::SeqCst;
    let decoders = decoders(2, 4, 64, Duration::from_millis(500));
    let started = Arc::new(AtomicUsize::new(0));
    let (release, gate) = async_channel::bounded::<()>(1);
    let runs = (0..22).map(|i| {
        let (started, gate) = (started.clone(), gate.clone());
        let decoders = &decoders;
        async move {
            let decode = move || {
                started.fetch_add(1, SeqCst);
                let _ = gate.recv_blocking();
            };
            decoders.run(&format!("app{i}"), decode).await
        }
    });
    let done = zbus::block_on(futures_util::future::join_all(runs));
    assert!(
        done.iter().all(Option::is_none),
        "no decode finishes while reads hang"
    );
    assert_eq!(
        started.load(SeqCst),
        2,
        "only the two threads started a read"
    );
    drop(release);
    assert_eq!(zbus::block_on(decoders.run("app", || 7)), Some(7));
    assert_eq!(started.load(SeqCst), 2, "decodes given up on never run");
}

#[test]
fn one_apps_flood_of_decodes_waits_behind_its_own_share_and_another_apps_runs() {
    use std::sync::atomic::Ordering::SeqCst;
    let decoders = decoders(2, 64, 4, Duration::from_millis(500));
    let ran = Arc::new(AtomicUsize::new(0));
    let flood = (0..40).map(|_| {
        let ran = ran.clone();
        decoders.run("flood", move || {
            ran.fetch_add(1, SeqCst);
            std::thread::sleep(Duration::from_millis(100));
        })
    });
    let other = async {
        async_io::Timer::after(Duration::from_millis(20)).await;
        decoders.run("other", || 7).await
    };
    let (flood, other) =
        zbus::block_on(async { futures_util::join!(futures_util::future::join_all(flood), other) });
    assert_eq!(other, Some(7), "another app's decode runs");
    assert_eq!(ran.load(SeqCst), 4, "the flooding app's share ran");
    assert_eq!(flood.iter().filter(|d| d.is_some()).count(), 4);
}

#[test]
fn a_decode_gets_its_whole_time_once_a_thread_takes_it() {
    let decoders = decoders(1, 8, 4, Duration::from_millis(300));
    let slow = || std::thread::sleep(Duration::from_millis(250));
    let (first, second) = zbus::block_on(async {
        futures_util::join!(decoders.run("a", slow), decoders.run("b", slow))
    });
    assert_eq!((first, second), (Some(()), Some(())));
}

#[test]
fn a_panicking_decode_leaves_its_thread_to_the_next() {
    let decoders = decoders(1, 8, 4, Duration::from_secs(5));
    let panicked = zbus::block_on(decoders.run("a", || -> u32 { panic!("a bad icon") }));
    assert_eq!(panicked, None);
    assert_eq!(zbus::block_on(decoders.run("a", || 7)), Some(7));
}

#[test]
fn a_burst_of_notification_pictures_waits_for_the_decoders_and_each_arrives() {
    use std::sync::atomic::Ordering::SeqCst;
    let dir = std::env::temp_dir().join(format!("reprise-notify-burst-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("avatar.png");
    image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0x40, 0x80, 0xFF]))
        .save(&file)
        .unwrap();
    let decoders = decoders(1, 64, 4, Duration::from_secs(5));
    let (release, gate) = async_channel::bounded::<()>(1);
    let held = decoders.run("tray", move || {
        let _ = gate.recv_blocking();
    });
    let done = Arc::new(AtomicUsize::new(0));
    let burst = (0..8).map(|i| {
        let (done, file, decoders) = (done.clone(), file.clone(), &decoders);
        async move {
            let image = picture_file(decoders, &format!(":1.{i}"), file).await;
            done.fetch_add(1, SeqCst);
            image
        }
    });
    let check = async {
        async_io::Timer::after(Duration::from_millis(200)).await;
        let early = done.load(SeqCst);
        drop(release);
        early
    };
    let (_, images, early) = zbus::block_on(async {
        futures_util::join!(held, futures_util::future::join_all(burst), check)
    });
    assert_eq!(early, 0, "no picture decodes while the one thread is held");
    for image in &images {
        assert_eq!(image.len(), NOTIFY_SIZES.len());
        let big = image.iter().max_by_key(|s| s.width).unwrap();
        let mid = big.pixels[(big.height / 2 * big.width + big.width / 2) as usize];
        assert_eq!(mid, 0xFF20_4080, "the picture's blue, opaque");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A tray item with an icon name, for reads that need only some answer.
struct Answering;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Answering {
    #[zbus(property)]
    fn icon_name(&self) -> String {
        "answering".into()
    }
}

/// How many match rules the bus daemon at `conn` holds for `name`.
async fn match_rules(conn: &Connection, name: &str) -> u32 {
    let reply = conn
        .call_method(
            Some(DBUS_IFACE),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Debug.Stats"),
            "GetConnectionStats",
            &(name,),
        )
        .await
        .expect("the bus daemon keeps connection stats");
    let body = reply.body();
    let stats: HashMap<String, Variant> = body.deserialize().unwrap();
    match stats["MatchRules"] {
        Variant::U32(n) => n,
        ref other => panic!("MatchRules is {other:?}"),
    }
}

/// Polls `f` every 20 ms for up to 5 s: whether it came true.
async fn becomes(mut f: impl AsyncFnMut() -> bool) -> bool {
    for _ in 0..250 {
        if f().await {
            return true;
        }
        async_io::Timer::after(Duration::from_millis(20)).await;
    }
    false
}

#[test]
fn an_item_flooding_change_signals_is_dropped_at_the_bus_until_it_is_heard_again() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let (muted, heard) = zbus::block_on(async {
        let connect = || zbus::connection::Builder::address(bus.1.as_str());
        let item = connect()
            .serve_at("/item", Answering)
            .build()
            .await
            .unwrap();
        let ours = connect().build().await.unwrap();
        let us = ours.unique_name().unwrap().to_string();
        let (tx, _events) = std::sync::mpsc::channel();
        let mut lp = test_loop(ours, tx, -1);
        lp.muted_for = Duration::from_millis(500);
        lp.track(item.unique_name().unwrap().to_string(), "/item".into())
            .await;
        let watching = match_rules(&item, &us).await;
        assert!(watching >= ITEM_SIGNALS.len() as u32);
        let script = async {
            for _ in 0..MAX_CHANGES * 3 {
                item.emit_signal(None::<()>, "/item", ITEM_IFACE, "NewIcon", &())
                    .await
                    .unwrap();
            }
            let fewer = watching - ITEM_SIGNALS.len() as u32;
            let muted = becomes(async || match_rules(&item, &us).await == fewer).await;
            let heard = becomes(async || match_rules(&item, &us).await == watching).await;
            (muted, heard)
        };
        let run = async {
            lp.run().await;
            std::future::pending().await
        };
        futures_lite::future::or(script, run).await
    });
    assert!(muted, "the flood's match rules are dropped");
    assert!(heard, "and come back once it is heard again");
}

#[test]
fn a_read_whose_names_werent_decoded_keeps_the_icons_shown_and_reads_once_more() {
    let (tx, events) = std::sync::mpsc::channel();
    let picture: Arc<[IconSource]> =
        Arc::from([IconSource::from(crate::raster::Canvas::new(16, 16))]);
    let (named, reads) = zbus::block_on(async {
        let (ours, _item) = peers(Duration::from_secs(1)).await;
        let mut lp = test_loop(ours, tx, -1);
        let key = ":1.1/item".to_string();
        let shown = TrayItem {
            id: key.clone(),
            icon_name: Some("chat".into()),
            named: picture.clone(),
            ..TrayItem::default()
        };
        lp.items.insert(key.clone(), shown.clone());
        let undecoded = TrayItem {
            named: Arc::from([]),
            ..shown
        };
        lp.fetch(
            ":1.1".into(),
            "/item".into(),
            Duration::from_secs(60),
            false,
        );
        let mut reads = 0;
        while let Some(id) = lp.fetching.get(&key).map(|f| f.id)
            && reads < 3
        {
            reads += 1;
            let item = Some(Box::new(undecoded.clone()));
            lp.fetched(key.clone(), id, item, false).await;
        }
        (lp.items[&key].named.clone(), reads)
    });
    assert_eq!(named, picture, "the icons shown are kept");
    assert_eq!(reads, 2, "one read, then one read again");
    assert!(
        events
            .try_iter()
            .all(|e| !matches!(e, TrayEvent::Updated(_))),
        "nothing changed for the compositor"
    );
}

#[test]
fn a_read_whose_new_name_wasnt_decoded_shows_no_icon_of_the_old_name() {
    let (tx, _events) = std::sync::mpsc::channel();
    let picture: Arc<[IconSource]> =
        Arc::from([IconSource::from(crate::raster::Canvas::new(16, 16))]);
    let (named, reads) = zbus::block_on(async {
        let (ours, _item) = peers(Duration::from_secs(1)).await;
        let mut lp = test_loop(ours, tx, -1);
        let key = ":1.1/item".to_string();
        let shown = TrayItem {
            id: key.clone(),
            icon_name: Some("online".into()),
            named: picture.clone(),
            ..TrayItem::default()
        };
        lp.items.insert(key.clone(), shown.clone());
        let away = TrayItem {
            icon_name: Some("away".into()),
            named: Arc::from([]),
            ..shown
        };
        lp.fetch(
            ":1.1".into(),
            "/item".into(),
            Duration::from_secs(60),
            false,
        );
        let mut reads = 0;
        while let Some(id) = lp.fetching.get(&key).map(|f| f.id)
            && reads < 3
        {
            reads += 1;
            lp.fetched(key.clone(), id, Some(Box::new(away.clone())), false)
                .await;
        }
        (lp.items[&key].named.clone(), reads)
    });
    assert!(named.is_empty(), "the old name's icon is not shown");
    assert_eq!(reads, 2, "one read, then one read again");
}

#[test]
fn a_removed_items_read_never_reaches_it() {
    let (tx, _events) = std::sync::mpsc::channel();
    let titles = AtomicUsize::new(0);
    zbus::block_on(async {
        let (ours, item) = peers(Duration::from_secs(10)).await;
        let calls = zbus::MessageStream::from(&item);
        let mut lp = test_loop(ours, tx, -1);
        lp.fetch(
            ":1.1".into(),
            "/answering".into(),
            Duration::from_millis(100),
            false,
        );
        lp.remove_item(":1.1/answering").await;
        let wait = async_io::Timer::after(Duration::from_millis(500));
        let serve = async {
            serve_items(item.clone(), calls, &titles).await;
            std::future::pending().await
        };
        futures_lite::future::or(
            async {
                wait.await;
            },
            serve,
        )
        .await;
    });
    assert_eq!(titles.load(Ordering::Relaxed), 0, "no Title read");
}

/// Clients' links to a bus daemon through this process. [`Relay::cut`] ends one as dbus-broker
/// drops a peer: the client reads the end, the daemon keeps its names until the client closes.
struct Relay {
    address: String,
    links: Links,
}

/// Each link's socket to its client, and whether the link is cut.
type Links = Arc<Mutex<Vec<(std::os::unix::net::UnixStream, Arc<AtomicBool>)>>>;

/// Copies `from` to `to` with the fds that come along (the daemon hands out pidfds), until
/// `from` ends or `to` fails; bytes read once `cut` is set are dropped.
fn forward(
    from: &std::os::unix::net::UnixStream,
    to: &std::os::unix::net::UnixStream,
    cut: &AtomicBool,
) {
    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    use std::io::{IoSlice, IoSliceMut};
    use std::mem::MaybeUninit;
    use std::os::fd::AsFd;
    let mut buf = [0u8; 4096];
    loop {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(16))];
        let mut came = RecvAncillaryBuffer::new(&mut space);
        let iov = &mut [IoSliceMut::new(&mut buf)];
        let n = match recvmsg(from, iov, &mut came, RecvFlags::CMSG_CLOEXEC) {
            Ok(r) if r.bytes > 0 => r.bytes,
            _ => return,
        };
        let fds: Vec<OwnedFd> = came
            .drain()
            .flat_map(|m| match m {
                RecvAncillaryMessage::ScmRights(fds) => fds.collect(),
                _ => Vec::new(),
            })
            .collect();
        if cut.load(Ordering::Relaxed) {
            continue;
        }
        let fds: Vec<_> = fds.iter().map(|f| f.as_fd()).collect();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(16))];
        let mut along = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() {
            along.push(SendAncillaryMessage::ScmRights(&fds));
        }
        let mut sent = 0;
        while sent < n {
            let iov = &[IoSlice::new(&buf[sent..n])];
            match sendmsg(to, iov, &mut along, SendFlags::NOSIGNAL) {
                Ok(k) => sent += k,
                Err(_) => return,
            }
            along.clear();
        }
    }
}

impl Relay {
    fn to(daemon: &str) -> Relay {
        use std::os::unix::net::{UnixListener, UnixStream};
        let daemon = daemon
            .strip_prefix("unix:path=")
            .and_then(|a| a.split(',').next())
            .expect("a dbus-daemon at a path")
            .to_string();
        static RELAYS: AtomicUsize = AtomicUsize::new(0);
        let n = RELAYS.fetch_add(1, Ordering::Relaxed);
        let path = crate::harness::scratch().join(format!("relay-{n}"));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let links: Links = Arc::default();
        let kept = links.clone();
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let upstream = UnixStream::connect(&daemon).unwrap();
                let cut = Arc::new(AtomicBool::new(false));
                lock(&kept).push((client.try_clone().unwrap(), cut.clone()));
                let (from, to) = (client.try_clone().unwrap(), upstream.try_clone().unwrap());
                std::thread::spawn(move || {
                    forward(&from, &to, &cut);
                    let _ = to.shutdown(std::net::Shutdown::Both);
                });
                std::thread::spawn(move || forward(&upstream, &client, &AtomicBool::new(false)));
            }
        });
        Relay {
            address: format!("unix:path={}", path.display()),
            links,
        }
    }

    fn cut(&self) {
        for (client, cut) in lock(&self.links).drain(..) {
            cut.store(true, Ordering::Relaxed);
            let _ = client.shutdown(std::net::Shutdown::Write);
        }
    }
}

/// A tray serving on `bus` through a [`Relay`], on a thread of its own.
struct Relayed {
    relay: Relay,
    commands: async_channel::Sender<TrayCommand>,
    events: Receiver<TrayEvent>,
    tray: JoinHandle<()>,
}

impl Relayed {
    fn on(bus: &crate::testbus::Bus) -> Relayed {
        let relay = Relay::to(&bus.1);
        let (events, heard) = std::sync::mpsc::channel();
        let (commands, rx) = async_channel::unbounded();
        let kept = Kept {
            events: events.clone(),
            eventfd: -1,
            commands: rx,
            appearance: Arc::new(Mutex::new(
                Appearance::of(&crate::config::Config::default()),
            )),
            icons: hicolor(),
            sizes: Arc::new(Mutex::new(vec![16])),
            pads: Arc::default(),
            senders: Arc::default(),
            programs: Arc::default(),
            responding: Arc::default(),
            identities: Arc::default(),
            screensaver: screensaver::Shared::new(events, -1),
            next_id: Arc::new(AtomicU32::new(1)),
            defers: false,
        };
        let address = relay.address.clone();
        let tray = std::thread::spawn(move || zbus::block_on(serve(&kept, Some(&address))));
        Relayed {
            relay,
            commands,
            events: heard,
            tray,
        }
    }

    /// Stops the tray; whether it was still serving.
    fn stop(self) -> bool {
        let serving = self.commands.send_blocking(TrayCommand::Shutdown).is_ok();
        self.tray.join().unwrap();
        serving
    }
}

async fn observer(bus: &crate::testbus::Bus) -> Connection {
    zbus::connection::Builder::address(bus.1.as_str())
        .build()
        .await
        .unwrap()
}

/// The owner of `name` once there is one other than `not`.
async fn new_owner(observer: &Connection, name: &str, not: Option<&str>) -> Option<String> {
    let mut owner = None;
    becomes(async || {
        owner = owner_of(observer, name).await;
        owner.is_some() && owner.as_deref() != not
    })
    .await;
    owner.filter(|o| Some(o.as_str()) != not)
}

#[test]
fn a_tray_dropped_by_the_bus_connects_again_and_takes_its_names_back() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let tray = Relayed::on(&bus);
    let (first, again) = zbus::block_on(async {
        let observer = observer(&bus).await;
        let first = new_owner(&observer, NOTIFY_NAME, None).await;
        tray.relay.cut();
        let again = new_owner(&observer, NOTIFY_NAME, first.as_deref()).await;
        (first, again)
    });
    tray.stop();
    assert!(first.is_some(), "the tray hosts notifications");
    assert!(
        again.is_some(),
        "a new connection hosts them after the cut, after {first:?}"
    );
}

#[test]
fn what_a_peer_left_behind_while_the_tray_was_off_the_bus_goes_once_it_is_back() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let tray = Relayed::on(&bus);
    let (holder, events) = zbus::block_on(async {
        let observer = observer(&bus).await;
        let first = new_owner(&observer, NOTIFY_NAME, None).await;
        let peer = self::observer(&bus).await;
        let holder = peer.unique_name().unwrap().to_string();
        let path = SCREENSAVER_PATHS[0];
        let inhibit = ("mpv", "video");
        (peer.call_method(
            Some(SCREENSAVER_NAME),
            path,
            Some(SCREENSAVER_NAME),
            "Inhibit",
            &inhibit,
        ))
        .await
        .unwrap();
        let note = (
            "mpv",
            0u32,
            "",
            "Hi",
            "",
            Vec::<&str>::new(),
            HashMap::<&str, Variant>::new(),
            -1i32,
        );
        (peer.call_method(
            Some(NOTIFY_NAME),
            NOTIFY_PATH,
            Some(NOTIFY_NAME),
            "Notify",
            &note,
        ))
        .await
        .unwrap();
        let tray_iface = Some("org.reprise.Tray");
        let (name, at) = (reprisebus::tray::NAME, reprisebus::tray::PATH);
        (peer.call_method(Some(name), at, tray_iface, "WatchResponding", &()))
            .await
            .unwrap();
        tray.relay.cut();
        peer.close().await.unwrap();
        new_owner(&observer, NOTIFY_NAME, first.as_deref()).await;
        let mut events = Vec::new();
        while let Some(ev) = next_event(&tray.events, Duration::from_secs(3)).await {
            events.push(ev);
        }
        (holder, events)
    });
    tray.stop();
    let saw = |f: &dyn Fn(&TrayEvent) -> bool| events.iter().any(f);
    assert!(
        saw(&|e| matches!(e, TrayEvent::ScreenSaverInhibited(false))),
        "its inhibit is dropped"
    );
    assert!(
        saw(&|e| matches!(e, TrayEvent::NotifySenderGone(n) if *n == holder)),
        "its notifications are told it left"
    );
    assert!(
        saw(&|e| matches!(e, TrayEvent::WatchResponding(false))),
        "the ping round stops with its last watcher"
    );
}

#[test]
fn a_tray_whose_first_try_back_on_the_bus_fails_tries_again() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let tray = Relayed::on(&bus);
    let back = zbus::block_on(async {
        let observer = observer(&bus).await;
        let first = new_owner(&observer, NOTIFY_NAME, None).await;
        let dbus = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
        let mut notify_owners = dbus
            .receive_name_owner_changed_with_args(&[(0, NOTIFY_NAME)])
            .await
            .unwrap();
        // Queued behind the tray, this connection holds the host name once the cut closes it.
        let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let flags = zbus::fdo::RequestNameFlags::empty();
        observer
            .request_name_with_flags(host.as_str(), flags)
            .await
            .unwrap();
        tray.relay.cut();
        let failed = async {
            while let Some(change) = notify_owners.next().await {
                let args = change.args().unwrap();
                let left = args.new_owner().is_none();
                let old = args.old_owner().as_ref().map(|o| o.to_string());
                if left && old.is_some() && old != first {
                    return true;
                }
            }
            false
        };
        let timeout = async {
            async_io::Timer::after(Duration::from_secs(10)).await;
            false
        };
        assert!(
            futures_lite::future::or(failed, timeout).await,
            "a try back fails on the host name"
        );
        observer.release_name(host.as_str()).await.unwrap();
        let mut back = None;
        for _ in 0..3 {
            back = back.or(new_owner(&observer, NOTIFY_NAME, None).await);
        }
        back
    });
    let serving = tray.stop();
    assert!(serving, "the tray still serves");
    assert!(back.is_some(), "and hosts notifications again");
}

#[test]
fn an_item_registered_once_comes_back_after_the_tray_connects_again() {
    let Some(bus) = crate::testbus::bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    let tray = Relayed::on(&bus);
    let added = zbus::block_on(async {
        let observer = observer(&bus).await;
        new_owner(&observer, WATCHER_NAME, None).await;
        let item = zbus::connection::Builder::address(bus.1.as_str())
            .serve_at("/item", Answering)
            .build()
            .await
            .unwrap();
        let register = ("/item",);
        (item.call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some(WATCHER_IFACE),
            "RegisterStatusNotifierItem",
            &register,
        ))
        .await
        .unwrap();
        let mut added = 0;
        let mut cut = false;
        while let Some(ev) = next_event(&tray.events, Duration::from_secs(5)).await {
            if matches!(ev, TrayEvent::Added(_)) {
                added += 1;
                if cut {
                    break;
                }
                tray.relay.cut();
                cut = true;
            }
        }
        added
    });
    tray.stop();
    assert_eq!(added, 2, "shown, then shown again on the new connection");
}
