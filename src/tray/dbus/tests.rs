use super::item::{Pixmaps, ToolTip, decode_names, icon_ref, item_texts};
use super::menu::{LABEL_CHARS, MENU_ROWS, parse_layout};
use super::notifications::{CAPABILITIES, action_icons, notification_icons};
use super::watcher::{
    Admission, MAX_REGISTRATIONS, MAX_REGISTRATIONS_PER_SENDER, admit, resolve_registration,
};
use super::*;
use crate::tray::{DEFAULT_PROP, MenuNode, NOTIFY_SIZES, item_pixmaps, notify};
use reprisekit::controls::tooltip::TIP_CHARS;
use reprisekit::icon::IconSource;
use std::collections::HashMap;
use std::os::fd::FromRawFd;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Receiver;
use zbus::zvariant::{DeserializeValue, OwnedValue, Value};

fn child(id: i32, props: Vec<(&str, Value<'static>)>) -> OwnedValue {
    let map: HashMap<&str, Value> = props.into_iter().collect();
    Value::from((id, map, Vec::<Value>::new()))
        .try_to_owned()
        .unwrap()
}

#[test]
fn parse_layout_extracts_nodes() {
    let children = vec![
        child(
            7,
            vec![
                ("label", Value::from("_Connect")),
                ("enabled", Value::from(false)),
                ("toggle-type", Value::from("checkmark")),
                ("toggle-state", Value::from(1i32)),
            ],
        ),
        child(8, vec![("type", Value::from("separator"))]),
        child(
            9,
            vec![
                ("label", Value::from("More")),
                ("visible", Value::from(false)),
                ("children-display", Value::from("submenu")),
            ],
        ),
    ];
    let nodes = parse_layout(&children);
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
    let radio = parse_layout(&[child(
        10,
        vec![
            ("label", Value::from("Balanced")),
            ("toggle-type", Value::from("radio")),
            ("toggle-state", Value::from(1i32)),
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
            ("label", Value::from("_Connect")),
            ("enabled", Value::from(false)),
            ("toggle-type", Value::from("checkmark")),
            ("toggle-state", Value::from(0i32)),
            ("children-display", Value::from("submenu")),
            (DEFAULT_PROP, Value::from(true)),
        ],
    )];
    let Value::Structure(s) = &*children[0] else {
        panic!("fixture is not a structure");
    };
    let Some(Value::Dict(props)) = s.fields().get(1) else {
        panic!("fixture has no property dictionary");
    };
    assert!(
        props.iter().all(|(_, v)| matches!(v, Value::Value(_))),
        "fixture is not in the a{{sv}} wire shape"
    );
    assert_eq!(
        parse_layout(&children),
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
    let children: Vec<OwnedValue> = (0..MENU_ROWS as i32 + 10)
        .map(|id| child(id, vec![("label", Value::from(long.clone()))]))
        .collect();
    let nodes = parse_layout(&children);
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
fn icon_pixmap_variant_decodes_borrowing_the_pixels() {
    use zbus::zvariant::{LE, serialized::Context, to_bytes};
    let list = vec![(2i32, 1i32, vec![7u8; 8])];
    let data = to_bytes(Context::new_dbus(LE, 0), &Value::from(list)).unwrap();
    let (v, _): (DeserializeValue<Pixmaps>, _) = data.deserialize().unwrap();
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
    let (app, image) = notification_icons(&hicolor(), &hints, blue_svg("both"), None);
    assert_eq!(app.len(), NOTIFY_SIZES.len());
    assert_eq!(image.len(), NOTIFY_SIZES.len());
}

#[test]
fn image_path_is_the_picture_and_an_unresolved_app_icon_stays_empty() {
    let hints = notify::Hints {
        image_path: Some(format!("file://{}", blue_svg("path"))),
        ..Default::default()
    };
    let (app, image) = notification_icons(&hicolor(), &hints, String::new(), None);
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
    let (_, image) = notification_icons(&hicolor(), &hints, String::new(), None);
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
    let (app, image) = notification_icons(&hicolor(), &hints, String::new(), Some(&entry));
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
    assert!(decode_names(&icons, &mut item, &drawn), "the name decoded");
    let sizes: Vec<(u32, u32)> = item.named.iter().map(|s| (s.width, s.height)).collect();
    assert_eq!(sizes, [(16, 16), (24, 24), (32, 32), (48, 48)]);
    let at24 = crate::tray::closest(&item.named, 24).unwrap();
    assert_eq!(at24.get(12, 12), 0xFF00_00FF);
    assert!(!decode_names(&icons, &mut item, &drawn), "nothing changed");

    item.attention.name = Some("peer".into());
    item.overlay.name = Some("peer".into());
    assert!(decode_names(&icons, &mut item, &drawn));
    assert_eq!(
        item.attention.named.len(),
        drawn.len(),
        "the attention name decodes too"
    );
    assert_eq!(item.overlay.named.len(), drawn.len(), "and the overlay's");

    item.pixmaps = item_pixmaps(&[(1, 1, &[255u8, 255, 0, 0][..])], &drawn).into();
    decode_names(&icons, &mut item, &drawn);
    assert!(item.named.is_empty(), "pixmaps stand in for the name");
    item.pixmaps = Arc::from([]);
    item.icon_name = Some("absent".into());
    decode_names(&icons, &mut item, &drawn);
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
fn parse_layout_skips_malformed_children() {
    let junk = Value::from(42i32).try_to_owned().unwrap();
    assert!(parse_layout(&[junk]).is_empty());
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
        published: None,
        screensaver: screensaver::Shared::new(events, eventfd),
        streams: SelectAll::new(),
        appearance: Arc::new(Mutex::new(
            Appearance::of(&crate::config::Config::default()),
        )),
        icons: Arc::new(Mutex::new(IconCache::new(IconTheme::load("hicolor", &[])))),
        sizes: Arc::new(Mutex::new(vec![16])),
        senders: Arc::default(),
    }
}

async fn peers(timeout: Duration) -> (Connection, Connection) {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let (item_side, our_side) = futures_util::join!(
        zbus::connection::Builder::async_io_unix_stream(theirs)
            .server(zbus::Guid::generate())
            .unwrap()
            .p2p()
            .build(),
        zbus::connection::Builder::async_io_unix_stream(ours)
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
            let _ = conn.reply(&hdr, &Value::from("B")).await;
            if first && path == "/changing" {
                let changed = zbus::message::Message::signal("/changing", ITEM_IFACE, "NewTitle")
                    .unwrap()
                    .sender(":1.1")
                    .unwrap()
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
                .unwrap()
                .sender(":1.1")
                .unwrap()
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
        lp.fetch(":1.1".into(), "/answering".into(), Duration::ZERO);
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
                .unwrap()
                .sender(":1.1")
                .unwrap()
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
        senders.owns(a, ":1.7"),
        "and the first sender still owns it"
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
    assert!(senders.owns(a, ":1.7"), "the stored sender stays");
    assert!(!senders.owns(a, ":1.9"));
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
