//! The Network page: airplane mode, which acts at once on the radios rather than through Apply,
//! and the session proxy, which Apply writes ([`crate::proxyhost`]).

use super::{Area, GX, X, edit, group, radio, updown};
use reprisefs::proxy::{Mode, Proxy, Server};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::updown::WIDTH as PORT_W;
use reprisekit::raster::Rect;

pub const AIRPLANE: &str = "airplane";
const AIRPLANE_NOTE: &str = "airplane_note";
const AIRPLANE_STATUS: &str = "airplane_status";

pub const NO_RADIO: &str = "This computer has no radio to turn off.";

pub const PROXY_NONE: &str = "proxy_none";
pub const PROXY_MANUAL: &str = "proxy_manual";
pub const PROXY_AUTO: &str = "proxy_auto";
const PROXY_GROUP: &str = "proxy";
/// Each protocol's host field and port up-down (0: none), in [`Protocol::ALL`] order.
pub const PROXY_HOSTS: [&str; 4] = ["proxy_http", "proxy_https", "proxy_ftp", "proxy_socks"];
pub const PROXY_PORTS: [&str; 4] = [
    "proxy_http_port",
    "proxy_https_port",
    "proxy_ftp_port",
    "proxy_socks_port",
];
const HOST_LABELS: [&str; 4] = [
    "proxy_http_label",
    "proxy_https_label",
    "proxy_ftp_label",
    "proxy_socks_label",
];
const PORT_LABELS: [&str; 4] = [
    "proxy_http_port_label",
    "proxy_https_port_label",
    "proxy_ftp_port_label",
    "proxy_socks_port_label",
];
pub const PROXY_IGNORE: &str = "proxy_ignore";
const PROXY_IGNORE_LABEL: &str = "proxy_ignore_label";
pub const PROXY_PAC: &str = "proxy_pac";
const PROXY_PAC_LABEL: &str = "proxy_pac_label";
const PROXY_NOTE: &str = "proxy_note";

/// Every control the proxy rows read, for the host to tell them from airplane mode.
pub fn is_proxy(id: &str) -> bool {
    [
        PROXY_NONE,
        PROXY_MANUAL,
        PROXY_AUTO,
        PROXY_IGNORE,
        PROXY_PAC,
    ]
    .contains(&id)
        || PROXY_HOSTS.contains(&id)
        || PROXY_PORTS.contains(&id)
}

const PROXY_Y: i32 = 108;
/// The rows under a radio start here, past its dot.
const INDENT: i32 = GX + 18;
const ROW_LABEL_W: i32 = 64;
const PORT_LABEL_W: i32 = 30;

/// The page before the radios are read: the check box greyed until [`show_airplane`], and no proxy
/// until [`show_proxy`].
pub fn network(a: Area) -> Form {
    let mut items = vec![
        Item::check(AIRPLANE, X, 44, "&Airplane mode"),
        Item::label_at(
            AIRPLANE_NOTE,
            X,
            64,
            a.w - 18,
            "Turns off Wi-Fi, mobile broadband and Bluetooth.",
        )
        .under_check(),
        Item::label_at(AIRPLANE_STATUS, X, 80, a.w - 18, "").under_check(),
    ];
    items.extend(proxy(a));
    let mut page = Form::new(items);
    page.set_enabled(AIRPLANE, false);
    show_proxy(&mut page, &Proxy::default());
    page
}

fn proxy(a: Area) -> Vec<Item> {
    let port_x = a.gright() - PORT_W;
    let port_label_x = port_x - 4 - PORT_LABEL_W;
    let host_x = INDENT + ROW_LABEL_W + 4;
    let host_w = port_label_x - 8 - host_x;
    let hosts = ["&HTTP:", "HTTP&S:", "&FTP:", "S&OCKS:"];
    let mut items = vec![
        group(a, PROXY_GROUP, PROXY_Y, 282, "Proxy"),
        radio(PROXY_NONE, PROXY_GROUP, GX, PROXY_Y + 22, "&No proxy"),
        radio(
            PROXY_MANUAL,
            PROXY_GROUP,
            GX,
            PROXY_Y + 44,
            "&Manual configuration",
        ),
    ];
    for i in 0..4 {
        let y = PROXY_Y + 72 + 26 * i as i32;
        items.extend([
            Item::label_at(HOST_LABELS[i], INDENT, y, ROW_LABEL_W, hosts[i]),
            edit(PROXY_HOSTS[i], host_x, y - 4, host_w),
            Item::label_at(PORT_LABELS[i], port_label_x, y, PORT_LABEL_W, "Port:"),
            updown(PROXY_PORTS[i], port_x, y - 4, 0, u16::MAX as i32),
        ]);
    }
    let ignore_y = PROXY_Y + 176;
    let pac_y = PROXY_Y + 254;
    items.extend([
        Item::label_at(
            PROXY_IGNORE_LABEL,
            INDENT,
            ignore_y,
            a.gright() - INDENT,
            "&Ignore hosts (separated by commas):",
        ),
        edit(PROXY_IGNORE, INDENT, ignore_y + 16, a.gright() - INDENT),
        radio(
            PROXY_AUTO,
            PROXY_GROUP,
            GX,
            ignore_y + 48,
            "Au&tomatic configuration",
        ),
        Item::label_at(PROXY_PAC_LABEL, INDENT, pac_y, ROW_LABEL_W, "Script &URL:"),
        edit(PROXY_PAC, host_x, pac_y - 4, a.gright() - host_x),
        Item::new(
            PROXY_NOTE,
            Rect::new(X, PROXY_Y + 292, a.w, 2 * LINE_H),
            Control::Lines(vec![
                "Programs started after Apply use the proxy; those".into(),
                "already running use it once restarted.".into(),
            ]),
        ),
    ]);
    items
}

/// Show `p` in the proxy rows, the rows of the modes not picked greyed.
pub fn show_proxy(page: &mut Form, p: &Proxy) {
    for (id, mode) in [
        (PROXY_NONE, Mode::None),
        (PROXY_MANUAL, Mode::Manual),
        (PROXY_AUTO, Mode::Auto),
    ] {
        page.set_check(id, p.mode == mode);
    }
    for (i, s) in p.servers.iter().enumerate() {
        page.show_edit_text(PROXY_HOSTS[i], &s.host);
        if let Some(Control::UpDown(u)) = page.item_mut(PROXY_PORTS[i]).map(|i| &mut i.control) {
            u.set(s.port.parse().unwrap_or(0));
        }
    }
    page.show_edit_text(PROXY_IGNORE, &p.ignore);
    page.show_edit_text(PROXY_PAC, &p.pac);
    follow_proxy(page, p.mode);
}

/// Grey the rows of the modes other than `mode`.
fn follow_proxy(page: &mut Form, mode: Mode) {
    let manual = mode == Mode::Manual;
    for id in HOST_LABELS
        .into_iter()
        .chain(PROXY_HOSTS)
        .chain(PORT_LABELS)
        .chain(PROXY_PORTS)
        .chain([PROXY_IGNORE_LABEL, PROXY_IGNORE])
    {
        page.set_enabled(id, manual);
    }
    for id in [PROXY_PAC_LABEL, PROXY_PAC] {
        page.set_enabled(id, mode == Mode::Auto);
    }
}

/// The proxy as the rows show it, the greying following the radio picked.
pub fn read_proxy(page: &mut Form) -> Proxy {
    let mode = if page.checked(PROXY_MANUAL) == Some(true) {
        Mode::Manual
    } else if page.checked(PROXY_AUTO) == Some(true) {
        Mode::Auto
    } else {
        Mode::None
    };
    follow_proxy(page, mode);
    let server = |i: usize| Server {
        host: page.edit_text(PROXY_HOSTS[i]).into(),
        port: match page.item(PROXY_PORTS[i]).map(|i| &i.control) {
            Some(Control::UpDown(u)) if u.value() > 0 => u.value().to_string(),
            _ => String::new(),
        },
    };
    Proxy {
        mode,
        servers: [0, 1, 2, 3].map(server),
        ignore: page.edit_text(PROXY_IGNORE).into(),
        pac: page.edit_text(PROXY_PAC).into(),
    }
}

/// Show airplane mode as `state` has it (None: no radio switch), the check box greyed while
/// `busy` carries out a change.
pub fn show_airplane(page: &mut Form, state: Option<bool>, busy: bool) {
    page.set_check(AIRPLANE, state == Some(true));
    page.set_enabled(AIRPLANE, state.is_some() && !busy);
    page.set_enabled(AIRPLANE_NOTE, state.is_some());
    page.set_text(AIRPLANE_STATUS, if state.is_none() { NO_RADIO } else { "" });
}
