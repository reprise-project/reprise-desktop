use std::time::Instant;

use reprisefs::props::thousands;
use reprisekit::art::neticon::{self, NetIcon};
use reprisekit::art::signalbars;
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::DialogFont;

use crate::nm;
use crate::tray::speed_text;

pub const CLIENT: (i32, i32) = (361, 419);
pub const TABS: [&str; 2] = ["General", "Support"];
pub const TAB_GENERAL: usize = 0;
pub const TAB_SUPPORT: usize = 1;

const GX: i32 = 21;
const GW: i32 = 320;
const GY: i32 = 47;
const ROW_X: i32 = GX + 16;
const VALUE_RIGHT: i32 = GX + GW - 15;
const LABEL_W: i32 = 110;
const VALUE_W: i32 = 150;
const VALUE_X: i32 = VALUE_RIGHT - VALUE_W;

const ROWS: [i32; 4] = [63, 82, 102, 121];
const SUP_ROWS: [i32; 4] = [64, 89, 113, 137];
const SUP_LABEL_X: i32 = 76;
const SUP_LABEL_W: i32 = 120;
const SUP_VALUE_X: i32 = 206;

const BTN_Y: i32 = 342;
const BTN_W: i32 = 83;

const PIC: (i32, i32) = (196, 212);
const DIVIDER: (i32, i32, i32, i32) = (211, 269, 2, 16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Disable,
    Repair,
    ViewWireless,
    Close,
}

/// The Status window's model: the sheet and the device it watches. Nothing here reads the bus; the
/// host feeds it [`nm::Device`] snapshots and acts on the [`Outcome`]s.
pub struct Status {
    pub sheet: PropSheet,
    pub device_path: String,
    /// The pages are cut once for the device the window opened on, so a refresh writes the Wi-Fi
    /// rows only if it built them.
    wifi: bool,
}

impl Status {
    pub fn new(d: &nm::Device) -> Status {
        let wifi = d.kind == nm::Kind::Wifi;
        let pages = vec![
            general_page(wifi).set_in(DialogFont::ShellDlg),
            support_page(d),
        ];
        let mut s = Status {
            sheet: propsheet::close_only(&TABS, pages, CLIENT),
            device_path: d.path.clone(),
            wifi,
        };
        for page in &mut s.sheet.pages {
            page.focus_id(propsheet::OK);
        }
        s.refresh(d);
        s
    }

    pub fn refresh(&mut self, d: &nm::Device) {
        let g = &mut self.sheet.pages[TAB_GENERAL];
        g.set_text("status", status_word(d));
        g.set_text("duration", &duration_text(d.connected_since));
        g.set_text("speed", &speed_text(d.speed_mbit));
        g.set_text("sent", &thousands(d.tx_bytes));
        g.set_text("received", &thousands(d.rx_bytes));
        if self.wifi {
            // A radio hosting a hotspot has no signal of its own: its row names the hotspot.
            match d.hotspot_summary() {
                Some(hotspot) => {
                    g.set_text("signal_l", "Hotspot:");
                    g.set_text("hotspot", &reprisekit::text::escape_mnemonic(&hotspot));
                    g.set_picture("signal", Canvas::new(0, 0));
                }
                None => {
                    let strength = d.wifi.as_ref().map(|w| w.strength).unwrap_or(0);
                    g.set_text("signal_l", "Signal Strength:");
                    g.set_text("hotspot", "");
                    g.set_picture("signal", bars(strength));
                }
            }
        }
        let (addr, mask, gateway) = match &d.ip4 {
            Some(ip) => (
                ip.addr.clone(),
                nm::subnet_mask(ip.prefix),
                ip.gateway.clone(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        let p = &mut self.sheet.pages[TAB_SUPPORT];
        p.set_picture("conn_icon", neticon::draw32(device_icon(d), false));
        p.set_text("address_type", address_type(&d.ip4));
        p.set_text("ip", &addr);
        p.set_text("mask", &mask);
        p.set_text("gateway", &gateway);
        p.set_lines("note", &note(d));
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::Ok | propsheet::Event::Cancel => Outcome::Close,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Pressed("disable")) => Outcome::Disable,
            propsheet::Event::Page(_, form::Event::Pressed("repair")) => Outcome::Repair,
            propsheet::Event::Page(_, form::Event::Pressed("view_wireless")) => {
                Outcome::ViewWireless
            }
            _ => Outcome::None,
        }
    }

    pub fn window_title(d: &nm::Device) -> String {
        format!("{} Status", d.shell_name)
    }
}

pub fn device_icon(d: &nm::Device) -> NetIcon {
    let live = d.managed && d.state == nm::State::Connected;
    let busy = d.managed && d.state == nm::State::Connecting;
    match d.kind {
        nm::Kind::Wired if live && d.carrier => NetIcon::Wired,
        nm::Kind::Wired if busy => NetIcon::Connecting,
        nm::Kind::Wired => NetIcon::WiredUnplugged,
        nm::Kind::Wifi if live => {
            NetIcon::Wireless(neticon::bars(d.wifi.as_ref().map_or(0, |w| w.strength)))
        }
        nm::Kind::Wifi if busy => NetIcon::Connecting,
        nm::Kind::Wifi => NetIcon::WirelessOff,
    }
}

pub fn duration_text(since: Option<Instant>) -> String {
    since
        .map(|t| elapsed_text(t.elapsed().as_secs()))
        .unwrap_or_default()
}

pub fn elapsed_text(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let hms = format!(
        "{:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    );
    match days {
        0 => hms,
        1 => format!("1 day {hms}"),
        d => format!("{d} days {hms}"),
    }
}

pub fn status_word(d: &nm::Device) -> &'static str {
    if !d.managed {
        return "Disabled";
    }
    if d.kind == nm::Kind::Wired && !d.carrier {
        return "Network cable unplugged";
    }
    match d.state {
        nm::State::Connected => "Connected",
        nm::State::Connecting => "Acquiring network address",
        nm::State::Failed => "Limited or no connectivity",
        _ => "Not connected",
    }
}

pub fn address_type(ip4: &Option<nm::Ip4>) -> &'static str {
    match ip4 {
        Some(ip) if ip.dhcp => "Assigned by DHCP",
        Some(_) => "Manually Configured",
        None => "",
    }
}

fn note(d: &nm::Device) -> Vec<String> {
    let text: &[&str] = if d.state == nm::State::Connected {
        &[
            "Reprise did not detect problems with this",
            "connection. If you cannot connect, click",
            "Repair.",
        ]
    } else {
        &[status_word(d), "Click Repair to try this connection again."]
    };
    text.iter().map(|s| (*s).to_string()).collect()
}

fn bars(strength: u8) -> Canvas {
    let mut c = Canvas::new(signalbars::WIDTH as u32, signalbars::HEIGHT as u32);
    signalbars::draw(&mut c, 0, 0, signalbars::five(strength), true);
    c
}

fn general_page(wifi: bool) -> Form {
    let row = |i: usize, id: &'static str, vid: &'static str, text: &str| {
        [
            Item::label_at(id, ROW_X, ROWS[i], LABEL_W, text),
            Item::value_at(vid, VALUE_X, ROWS[i], VALUE_W, ""),
        ]
    };
    let mut items = vec![Item::group(
        "conn_g",
        Rect::new(GX, GY, GW, 135),
        "Connection",
    )];
    items.extend(row(0, "status_l", "status", "Status:"));
    items.extend(row(1, "duration_l", "duration", "Duration:"));
    items.extend(row(2, "speed_l", "speed", "Speed:"));
    if wifi {
        items.push(Item::label_at(
            "signal_l",
            ROW_X,
            ROWS[3],
            LABEL_W,
            "Signal Strength:",
        ));
        items.push(Item::picture(
            "signal",
            Rect::new(
                VALUE_RIGHT - signalbars::WIDTH,
                ROWS[3] - 2,
                signalbars::WIDTH,
                signalbars::HEIGHT,
            ),
            Some(bars(0)),
        ));
        items.push(Item::value_at("hotspot", VALUE_X, ROWS[3], VALUE_W, ""));
    }
    items.extend([
        Item::group("activity_g", Rect::new(GX, 195, GW, 135), "Activity"),
        Item::value_at("sent_l", 93, 222, 60, "Sent"),
        Item::label_at("received_l", 279, 222, 62, "Received"),
        Item::picture(
            "activity_icon",
            Rect::new(PIC.0, PIC.1, 32, 32),
            Some(neticon::draw32(NetIcon::Wired, false)),
        ),
        Item::rule(
            "divider",
            Rect::new(DIVIDER.0, DIVIDER.1, DIVIDER.2, DIVIDER.3),
        ),
        Item::label_at("bytes_l", ROW_X, 269, 60, "Bytes:"),
        Item::value_at("sent", 103, 269, 80, ""),
        Item::value_at("received", 246, 269, 80, ""),
        Item::button_at("properties", GX, BTN_Y, 75, "&Properties...", false),
        Item::button_at("disable", 100, BTN_Y, BTN_W, "Disa&ble", true),
    ]);
    if wifi {
        items.push(Item::button_at(
            "view_wireless",
            187,
            BTN_Y,
            152,
            "&View Wireless Networks",
            true,
        ));
    }
    Form::new(items)
}

fn support_page(d: &nm::Device) -> Form {
    let row = |i: usize, id: &'static str, vid: &'static str, text: &str| {
        [
            Item::label_at(id, SUP_LABEL_X, SUP_ROWS[i], SUP_LABEL_W, text),
            Item::value_at(vid, SUP_VALUE_X, SUP_ROWS[i], VALUE_RIGHT - SUP_VALUE_X, ""),
        ]
    };
    let mut items = vec![
        Item::group("status_g", Rect::new(GX, GY, GW, 150), "Connection status"),
        Item::picture(
            "conn_icon",
            Rect::new(GX + 12, 64, 32, 32),
            Some(neticon::draw32(device_icon(d), false)),
        ),
    ];
    items.extend(row(0, "address_type_l", "address_type", "Address Type:"));
    items.extend(row(1, "ip_l", "ip", "IP Address:"));
    items.extend(row(2, "mask_l", "mask", "Subnet Mask:"));
    items.extend(row(3, "gateway_l", "gateway", "Default Gateway:"));
    items.extend([
        Item::button_at("details", 75, 162, BTN_W, "&Details...", false),
        Item::lines_at("note", 25, 214, 230, &["", "", ""]),
        Item::button_at("repair", 256, 214, BTN_W, "&Repair", true),
    ]);
    Form::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::form::Control;
    use reprisekit::raster::Rect;

    fn wired() -> nm::Device {
        nm::Device {
            path: "/org/freedesktop/NetworkManager/Devices/2".into(),
            iface: "enp3s0".into(),
            kind: nm::Kind::Wired,
            shell_name: "Local Area Connection 2".into(),
            product: "e1000e (enp3s0)".into(),
            state: nm::State::Connected,
            managed: true,
            carrier: true,
            speed_mbit: 100,
            connected_since: Some(Instant::now()),
            ip4: Some(nm::Ip4 {
                addr: "192.168.1.241".into(),
                prefix: 24,
                gateway: "192.168.1.1".into(),
                dns: vec!["192.168.1.1".into()],
                dhcp: true,
            }),
            rx_bytes: 1234,
            tx_bytes: 394_394,
            wifi: None,
            hotspot: None,
            hotspot_clients: None,
        }
    }

    fn wireless() -> nm::Device {
        nm::Device {
            kind: nm::Kind::Wifi,
            shell_name: "Wireless Network Connection".into(),
            wifi: Some(nm::Wifi {
                active_ssid: Some("Intrepid 5".into()),
                strength: 70,
                aps: vec![],
            }),
            ..wired()
        }
    }

    fn hotspot() -> nm::Device {
        nm::Device {
            hotspot: Some("Desk".into()),
            hotspot_clients: Some(2),
            ..wireless()
        }
    }

    fn text(page: &Form, id: &str) -> String {
        match &page.item(id).unwrap_or_else(|| panic!("no {id}")).control {
            Control::Label { text, .. } => text.clone(),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn note_lines(page: &Form) -> Vec<String> {
        match &page.item("note").unwrap().control {
            Control::Lines(l) => l.clone(),
            other => panic!("note is {other:?}"),
        }
    }

    fn picture_of(page: &Form, id: &str) -> Canvas {
        match &page.item(id).unwrap().control {
            Control::Picture { canvas, .. } => canvas.clone().unwrap(),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn rect(page: &Form, id: &str) -> Rect {
        page.item(id).unwrap_or_else(|| panic!("no {id}")).rect
    }

    /// The wired and wireless pages fit their text under every interface and text size.
    #[test]
    fn the_status_pages_fit_their_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            [
                ("wired", wired()),
                ("wireless", wireless()),
                ("hotspot", hotspot()),
            ]
            .into_iter()
            .flat_map(|(kind, d)| {
                let s = Status::new(&d).sheet;
                let tabs = s
                    .tabs
                    .labels
                    .into_iter()
                    .map(move |t| format!("{kind} {t}"));
                tabs.zip(s.pages)
            })
            .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn durations_format_as_clock_time_with_days() {
        assert_eq!(duration_text(None), "");
        assert_eq!(duration_text(Some(Instant::now())), "00:00:00");
        assert_eq!(elapsed_text(0), "00:00:00");
        assert_eq!(elapsed_text(45), "00:00:45");
        assert_eq!(elapsed_text(12 * 3600 + 34 * 60 + 56), "12:34:56");
        assert_eq!(
            elapsed_text(6 * 86400 + 23 * 3600 + 25 * 60 + 34),
            "6 days 23:25:34"
        );
        assert_eq!(elapsed_text(86400 + 61), "1 day 00:01:01");
    }

    #[test]
    fn status_words_per_state() {
        let mut d = wired();
        assert_eq!(status_word(&d), "Connected");
        d.state = nm::State::Connecting;
        assert_eq!(status_word(&d), "Acquiring network address");
        d.state = nm::State::Failed;
        assert_eq!(status_word(&d), "Limited or no connectivity");
        for s in [
            nm::State::Disconnected,
            nm::State::Unavailable,
            nm::State::Unmanaged,
        ] {
            d.state = s;
            assert_eq!(status_word(&d), "Not connected", "{s:?}");
        }
        d.carrier = false;
        d.state = nm::State::Unavailable;
        assert_eq!(status_word(&d), "Network cable unplugged");
        let mut w = wireless();
        w.carrier = false;
        w.state = nm::State::Disconnected;
        assert_eq!(status_word(&w), "Not connected");
        d.managed = false;
        assert_eq!(status_word(&d), "Disabled");
    }

    #[test]
    fn address_type_reads_dhcp() {
        let d = wired();
        assert_eq!(address_type(&d.ip4), "Assigned by DHCP");
        let manual = Some(nm::Ip4 {
            dhcp: false,
            ..d.ip4.clone().unwrap()
        });
        assert_eq!(address_type(&manual), "Manually Configured");
        assert_eq!(address_type(&None), "");
    }

    #[test]
    fn general_page_geometry() {
        let s = Status::new(&wired());
        let p = &s.sheet.pages[TAB_GENERAL];
        assert_eq!(rect(p, "conn_g"), Rect::new(21, 47, 320, 135));
        assert_eq!(rect(p, "activity_g"), Rect::new(21, 195, 320, 135));
        for (l, v) in [
            ("status_l", "status"),
            ("duration_l", "duration"),
            ("speed_l", "speed"),
        ] {
            assert!(rect(p, v).x > rect(p, l).right(), "{v} right of {l}");
            assert_eq!(rect(p, v).right(), 326, "{v} ends on x 326");
            assert_eq!(rect(p, v).y, rect(p, l).y, "{v} shares {l}'s row");
        }
        assert_eq!(rect(p, "properties"), Rect::new(21, 342, 75, 23));
        assert_eq!(rect(p, "disable"), Rect::new(100, 342, 83, 23));
        assert_eq!(rect(p, "activity_icon"), Rect::new(196, 212, 32, 32));
        assert_eq!(rect(p, "divider"), Rect::new(211, 269, 2, 16));
        assert_eq!(
            rect(p, "activity_icon").x + 16,
            rect(p, "divider").x + 1,
            "the icon stands over the divider"
        );
        assert_eq!(rect(p, "sent_l"), Rect::new(93, 222, 60, 13));
        assert_eq!(rect(p, "received_l"), Rect::new(279, 222, 62, 13));
        assert_eq!(rect(p, "bytes_l"), Rect::new(37, 269, 60, 13));
        assert_eq!(rect(p, "sent"), Rect::new(103, 269, 80, 13));
        assert_eq!(rect(p, "received"), Rect::new(246, 269, 80, 13));
        assert!(p.item("view_wireless").is_none(), "a wire has no chooser");
        assert!(p.item("signal").is_none(), "a wire has no bars");
        assert!(matches!(
            p.item("properties").unwrap().control,
            Control::Button { enabled: false, .. }
        ));
        let w = Status::new(&wireless());
        let p = &w.sheet.pages[TAB_GENERAL];
        assert_eq!(rect(p, "signal").right(), 326);
        assert!(rect(p, "signal").y > rect(p, "speed").y);
        assert_eq!(rect(p, "view_wireless"), Rect::new(187, 342, 152, 23));
    }

    #[test]
    fn support_page_geometry() {
        let s = Status::new(&wired());
        let p = &s.sheet.pages[TAB_SUPPORT];
        assert_eq!(rect(p, "status_g"), Rect::new(21, 47, 320, 150));
        assert_eq!(rect(p, "conn_icon"), Rect::new(33, 64, 32, 32));
        for (l, v) in [
            ("address_type_l", "address_type"),
            ("ip_l", "ip"),
            ("mask_l", "mask"),
            ("gateway_l", "gateway"),
        ] {
            assert_eq!(rect(p, l).x, 76);
            assert!(rect(p, l).x > rect(p, "conn_icon").right());
            assert_eq!(rect(p, v).right(), 326);
            assert_eq!(rect(p, v).y, rect(p, l).y);
        }
        assert_eq!(rect(p, "details"), Rect::new(75, 162, 83, 23));
        assert_eq!(rect(p, "repair"), Rect::new(256, 214, 83, 23));
        assert_eq!(rect(p, "note"), Rect::new(25, 214, 230, 39));
    }

    #[test]
    fn the_state_icon_follows_the_device() {
        assert_eq!(device_icon(&wired()), NetIcon::Wired);
        let mut d = wired();
        d.carrier = false;
        assert_eq!(device_icon(&d), NetIcon::WiredUnplugged);
        d.carrier = true;
        d.state = nm::State::Connecting;
        assert_eq!(device_icon(&d), NetIcon::Connecting);
        d.managed = false;
        assert_eq!(device_icon(&d), NetIcon::WiredUnplugged, "disabled");
        let mut w = wireless();
        assert_eq!(device_icon(&w), NetIcon::Wireless(3), "70 is three bars");
        w.state = nm::State::Disconnected;
        assert_eq!(device_icon(&w), NetIcon::WirelessOff);
        let mut s = Status::new(&wired());
        let before = picture_of(&s.sheet.pages[TAB_SUPPORT], "conn_icon");
        let mut off = wired();
        off.carrier = false;
        s.refresh(&off);
        let after = picture_of(&s.sheet.pages[TAB_SUPPORT], "conn_icon");
        assert_ne!(before, after, "the icon follows the state");
        assert_eq!(after, neticon::draw32(NetIcon::WiredUnplugged, false));
    }

    #[test]
    fn refresh_rewrites_labels() {
        let mut s = Status::new(&wired());
        let g = &s.sheet.pages[TAB_GENERAL];
        assert_eq!(text(g, "status"), "Connected");
        assert_eq!(text(g, "speed"), "100.0 Mbps");
        assert_eq!(text(g, "received"), "1,234");
        assert_eq!(text(g, "sent"), "394,394");
        let p = &s.sheet.pages[TAB_SUPPORT];
        assert_eq!(text(p, "address_type"), "Assigned by DHCP");
        assert_eq!(text(p, "ip"), "192.168.1.241");
        assert_eq!(text(p, "mask"), "255.255.255.0");
        assert_eq!(text(p, "gateway"), "192.168.1.1");
        assert_eq!(
            note_lines(p)[0],
            "Reprise did not detect problems with this"
        );

        let mut d = wired();
        d.rx_bytes = 7;
        d.state = nm::State::Disconnected;
        d.speed_mbit = 0;
        d.connected_since = None;
        d.ip4 = None;
        s.refresh(&d);
        let g = &s.sheet.pages[TAB_GENERAL];
        assert_eq!(text(g, "status"), "Not connected");
        assert_eq!(text(g, "duration"), "");
        assert_eq!(text(g, "speed"), "");
        assert_eq!(text(g, "received"), "7");
        let p = &s.sheet.pages[TAB_SUPPORT];
        assert_eq!(text(p, "address_type"), "");
        assert_eq!(text(p, "ip"), "");
        assert_eq!(text(p, "mask"), "");
        assert_eq!(text(p, "gateway"), "");
        assert_eq!(note_lines(p)[0], "Not connected");
    }

    #[test]
    fn a_hotspot_takes_the_signal_row() {
        let mut s = Status::new(&hotspot());
        let g = &s.sheet.pages[TAB_GENERAL];
        assert_eq!(text(g, "signal_l"), "Hotspot:");
        assert_eq!(text(g, "hotspot"), "Desk (2 devices)");
        assert_eq!(picture_of(g, "signal").width, 0, "no bars");
        assert_eq!(rect(g, "hotspot").y, rect(g, "signal_l").y);

        s.refresh(&wireless());
        let g = &s.sheet.pages[TAB_GENERAL];
        assert_eq!(text(g, "signal_l"), "Signal Strength:");
        assert_eq!(text(g, "hotspot"), "");
        assert_eq!(picture_of(g, "signal").width as i32, signalbars::WIDTH);
    }

    #[test]
    fn the_sheet_opens_with_the_focus_on_close() {
        let s = Status::new(&wired());
        for page in &s.sheet.pages {
            let focus = page.focus().map(|i| page.items[i].id);
            assert_eq!(focus, Some(propsheet::OK), "Enter closes the sheet");
        }
    }

    #[test]
    fn the_buttons_become_outcomes() {
        let mut s = Status::new(&wireless());
        let press = |s: &mut Status, id| {
            s.event(propsheet::Event::Page(
                s.sheet.tabs.open,
                form::Event::Pressed(id),
            ))
        };
        assert_eq!(press(&mut s, "disable"), Outcome::Disable);
        assert_eq!(press(&mut s, "view_wireless"), Outcome::ViewWireless);
        assert_eq!(press(&mut s, "repair"), Outcome::Repair);
        assert_eq!(press(&mut s, "properties"), Outcome::None);
        assert_eq!(s.event(propsheet::Event::Ok), Outcome::Close);
        assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
        assert_eq!(s.event(propsheet::Event::Redraw), Outcome::Redraw);
        assert_eq!(s.event(propsheet::Event::None), Outcome::None);
        assert_eq!(
            Status::window_title(&wired()),
            "Local Area Connection 2 Status"
        );
        assert_eq!(s.device_path, wired().path);
    }

    #[test]
    #[ignore]
    fn render_status() {
        let theme = reprisekit::theme::Theme {
            access_keys: true,
            ..reprisekit::theme::Theme::default()
        };
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut d = wired();
        d.connected_since = None;
        for (tab, name) in [
            (TAB_GENERAL, "status-general.png"),
            (TAB_SUPPORT, "status-support.png"),
        ] {
            let mut s = Status::new(&d);
            s.sheet.tabs.open = tab;
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(
                &mut c,
                &text,
                &theme.paint(),
                &s.sheet,
                false,
                theme.fonts.size_px,
            );
            reprisekit::raster::write_preview(&c, name);
        }
    }
}
