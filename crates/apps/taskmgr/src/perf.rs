//! The Performance page: a list of devices (the CPU, memory, each disk, network adapter and GPU,
//! and the sensors), each with a sparkline and its current figure, beside the picked device's
//! graphs and figures.

use std::collections::HashMap;

use repriseapp::detailslist::List;
use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput, Reply};
use reprisefs::netspeed;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{self, Item};
use reprisekit::controls::listview::{self, Column, Group};
use reprisekit::controls::perfgraph::{self, History};
use reprisekit::controls::tilelist::{self, TILE_H, Tile, TileList};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::keysym;

use crate::devices::{Source, hwmon};
use crate::sample::Snapshot;
use crate::{HISTORY, TaskMgr, grouped};

/// The device list's width at most, and at least while the window is narrow.
pub const LIST_W: i32 = 190;
const LIST_MIN_W: i32 = 140;
/// A row's sparkline, in the tile's icon slot.
const SPARK: (i32, i32) = (40, 32);
const PAD: i32 = 7;
/// A group box's insets round what it holds: the title band above, the edge elsewhere.
const GROUP_TOP: i32 = 16;
const GROUP_EDGE: i32 = 8;
const METER_GROUP_W: i32 = 84;
/// A figure group: three rows of label and value.
const FIGURES_H: i32 = 3 * LINE_H + 2 * 2 + GROUP_TOP + GROUP_EDGE;
/// Per-core graphs in a row before they wrap.
const CORES_PER_ROW: usize = 8;
/// The temperature a sensors sparkline tops out at.
const HOT: f64 = 100.0;
/// The least a rate graph's scale reaches, in bytes per second, so idle noise stays flat.
const DISK_FLOOR: f64 = 1_000_000.0;
const NET_FLOOR: f64 = 100_000.0;

/// A device on the list, as it is told apart from the others across samples.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Cpu,
    Memory,
    /// By kernel name.
    Disk(String),
    /// By interface name.
    Net(String),
    /// By card (`card1`).
    Gpu(String),
    Sensors,
}

/// What a device's pane holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    Cpu,
    Memory,
    Disk,
    Net,
    Gpu,
    Sensors,
}

impl Key {
    pub fn pane(&self) -> Pane {
        match self {
            Key::Cpu => Pane::Cpu,
            Key::Memory => Pane::Memory,
            Key::Disk(_) => Pane::Disk,
            Key::Net(_) => Pane::Net,
            Key::Gpu(_) => Pane::Gpu,
            Key::Sensors => Pane::Sensors,
        }
    }
}

/// A row of the device list.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub key: Key,
    pub title: String,
    pub figure: String,
}

/// A device's histories past the CPU's and memory's, which [`TaskMgr`] keeps for the status bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Hist {
    /// Active time as a fraction; read and write in bytes per second.
    Disk {
        active: History,
        read: History,
        write: History,
    },
    /// Bytes per second each way.
    Net { down: History, up: History },
    /// Busy share and the share of video memory in use.
    Gpu { busy: History, vram: History },
    /// The hottest temperature as a fraction of [`HOT`].
    Sensors { hottest: History },
}

pub struct Perf {
    pub list: TileList,
    pub devices: Vec<Device>,
    pub hist: HashMap<Key, Hist>,
    pub sensors: List,
}

impl Perf {
    pub fn new() -> Perf {
        let mut sensors = List::new(vec![
            Column::new("Sensor", 170),
            Column {
                right: true,
                ..Column::new("Value", 90)
            },
        ]);
        sensors.view.frame = true;
        let mut p = Perf {
            list: TileList::new(TILE_H),
            devices: Vec::new(),
            hist: HashMap::new(),
            sensors,
        };
        p.set_devices(devices(&Snapshot::default()));
        p
    }

    /// The device picked, the CPU before any is.
    pub fn selected(&self) -> &Key {
        (self.list.selected_row())
            .and_then(|i| self.devices.get(i))
            .map_or(&Key::Cpu, |d| &d.key)
    }

    fn set_devices(&mut self, devices: Vec<Device>) {
        let keep = self.selected().clone();
        self.list.entries = (0..devices.len()).map(tilelist::Entry::Row).collect();
        let at = devices.iter().position(|d| d.key == keep).unwrap_or(0);
        self.list.select_row(Some(at));
        self.devices = devices;
    }

    /// Takes in a snapshot: each device's histories, the rows, and the sensors list.
    pub fn take(&mut self, s: &Snapshot) {
        let d = &s.devices;
        let mut seen = Vec::new();
        for r in &d.disks {
            let key = Key::Disk(r.disk.name.clone());
            let h = self.hist.entry(key.clone()).or_insert_with(|| Hist::Disk {
                active: History::new(HISTORY),
                read: History::unbounded(HISTORY),
                write: History::unbounded(HISTORY),
            });
            if let (
                Hist::Disk {
                    active,
                    read,
                    write,
                },
                Some(x),
            ) = (h, r.rates)
            {
                active.push(x.active);
                read.push(x.read as f64);
                write.push(x.write as f64);
            }
            seen.push(key);
        }
        for n in &d.nets {
            let key = Key::Net(n.adapter.name.clone());
            let h = self.hist.entry(key.clone()).or_insert_with(|| Hist::Net {
                down: History::unbounded(HISTORY),
                up: History::unbounded(HISTORY),
            });
            if let (Hist::Net { down, up }, Some(r)) = (h, n.rate) {
                down.push(r.down as f64);
                up.push(r.up as f64);
            }
            seen.push(key);
        }
        for g in &d.gpus {
            let key = Key::Gpu(g.card.card.clone());
            let h = self.hist.entry(key.clone()).or_insert_with(|| Hist::Gpu {
                busy: History::new(HISTORY),
                vram: History::new(HISTORY),
            });
            if let Hist::Gpu { busy, vram } = h {
                if let Some(b) = g.busy {
                    busy.push(b);
                }
                if let Some((u, t)) = g.vram {
                    vram.push(u as f64 / t as f64);
                }
            }
            seen.push(key);
        }
        let h = (self.hist.entry(Key::Sensors)).or_insert_with(|| Hist::Sensors {
            hottest: History::new(HISTORY),
        });
        if let Hist::Sensors { hottest } = h {
            hottest.push(hottest_of(&d.sensors).unwrap_or(0.0) / HOT);
        }
        seen.push(Key::Sensors);
        self.hist.retain(|k, _| seen.contains(k));
        self.set_devices(devices(s));
        self.set_sensors(&d.sensors);
    }

    fn set_sensors(&mut self, chips: &[hwmon::Chip]) {
        let mut rows = Vec::new();
        let mut groups = Vec::new();
        for c in chips {
            groups.push(Group {
                title: c.name.clone(),
                first: rows.len(),
            });
            if c.asleep {
                rows.push(row(&["Not read while the device sleeps", ""]));
            }
            for r in &c.readings {
                rows.push(row(&[&r.label, &r.unit.text(r.value)]));
            }
        }
        let v = &mut self.sensors.view;
        v.top = v.top.min(rows.len().saturating_sub(1));
        v.rows = rows;
        v.groups = groups;
    }
}

fn row(cells: &[&str]) -> listview::Row {
    listview::Row {
        icon: None,
        cells: cells.iter().map(|c| c.to_string()).collect(),
    }
}

fn hottest_of(chips: &[hwmon::Chip]) -> Option<f64> {
    (chips.iter())
        .flat_map(|c| &c.readings)
        .filter(|r| r.unit == hwmon::Unit::Celsius)
        .map(|r| r.value)
        .reduce(f64::max)
}

fn pct(f: f64) -> String {
    format!("{}%", (f * 100.0).round() as u64)
}

fn celsius(v: f64) -> String {
    format!("{} \u{b0}C", v.round() as i64)
}

/// `b` bytes in binary units to one place: "512 MB", "1.5 GB", "1.8 TB".
pub fn bytes_text(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 || v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// The least of 1, 2 or 5 times a power of ten at or above `v` and `floor`: a graph's scale.
pub fn nice_scale(v: f64, floor: f64) -> f64 {
    let v = v.max(floor).max(f64::MIN_POSITIVE);
    let e = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * e)
        .find(|s| *s >= v)
        .unwrap_or(10.0 * e)
}

/// The device list for snapshot `s`, CPU and memory first and the sensors last.
pub fn devices(s: &Snapshot) -> Vec<Device> {
    let d = &s.devices;
    let mut out = Vec::new();
    let cpu = s.cpu.map_or_else(String::new, pct);
    out.push(Device {
        key: Key::Cpu,
        title: "CPU".into(),
        figure: match d.cpu_temp {
            Some(t) if !cpu.is_empty() => format!("{cpu}  {}", celsius(t)),
            _ => cpu,
        },
    });
    let m = &s.mem;
    out.push(Device {
        key: Key::Memory,
        title: "Memory".into(),
        figure: if m.total == 0 {
            String::new()
        } else {
            format!(
                "{}/{} ({})",
                bytes_text(m.used() * 1024),
                bytes_text(m.total * 1024),
                pct(m.used() as f64 / m.total as f64)
            )
        },
    });
    for (i, r) in d.disks.iter().enumerate() {
        out.push(Device {
            key: Key::Disk(r.disk.name.clone()),
            title: format!("Disk {i} ({})", r.disk.name),
            figure: r.rates.map_or_else(String::new, |x| pct(x.active)),
        });
    }
    for n in &d.nets {
        out.push(Device {
            key: Key::Net(n.adapter.name.clone()),
            title: format!("{} ({})", n.adapter.kind.label(), n.adapter.name),
            figure: n.rate.map_or_else(String::new, netspeed::line),
        });
    }
    for (i, g) in d.gpus.iter().enumerate() {
        let figure = match (g.asleep, g.busy) {
            (true, _) => "Asleep".into(),
            (false, None) => "No usage data".into(),
            (false, Some(b)) => match g.temp {
                Some(t) => format!("{}  {}", pct(b), celsius(t)),
                None => pct(b),
            },
        };
        out.push(Device {
            key: Key::Gpu(g.card.card.clone()),
            title: format!("GPU {i}"),
            figure,
        });
    }
    out.push(Device {
        key: Key::Sensors,
        title: "Sensors".into(),
        figure: hottest_of(&d.sensors).map_or_else(|| "None".into(), celsius),
    });
    out
}

/// A row of graphs: a meter's group id and title, then each graph's.
struct RowSpec {
    meter: Option<(&'static str, &'static str)>,
    graphs: &'static [(&'static str, &'static str)],
}

/// A figure group: its id and title, then each row's value id, label id and label.
type Figures = (
    &'static str,
    &'static str,
    [(&'static str, &'static str, &'static str); 3],
);

struct Spec {
    rows: &'static [RowSpec],
    figures: &'static [Figures],
    note: bool,
}

fn spec(p: Pane) -> &'static Spec {
    const TOTALS: Figures = (
        "totals",
        "Totals",
        [
            ("procs", "procs_label", "Processes"),
            ("threads", "threads_label", "Threads"),
            ("programs", "programs_label", "Programs"),
        ],
    );
    match p {
        Pane::Cpu => &Spec {
            rows: &[RowSpec {
                meter: Some(("cpu_meter", "CPU Usage")),
                graphs: &[("cpu_history", "CPU Usage History")],
            }],
            figures: &[
                TOTALS,
                (
                    "processor",
                    "Processor",
                    [
                        ("cores", "cores_label", "Cores"),
                        ("cpu_temp", "cpu_temp_label", "Temperature"),
                        ("cpu_fan", "cpu_fan_label", "Fan"),
                    ],
                ),
            ],
            note: false,
        },
        Pane::Memory => &Spec {
            rows: &[RowSpec {
                meter: Some(("mem_meter", "Memory")),
                graphs: &[("mem_history", "Memory Usage History")],
            }],
            figures: &[
                (
                    "physical",
                    "Physical Memory (MB)",
                    [
                        ("mem_total", "mem_total_label", "Total"),
                        ("mem_avail", "mem_avail_label", "Available"),
                        ("mem_cached", "mem_cached_label", "Cached"),
                    ],
                ),
                (
                    "swap",
                    "Swap (MB)",
                    [
                        ("swap_total", "swap_total_label", "Total"),
                        ("swap_used", "swap_used_label", "In use"),
                        ("swap_free", "swap_free_label", "Free"),
                    ],
                ),
            ],
            note: false,
        },
        Pane::Disk => &Spec {
            rows: &[
                RowSpec {
                    meter: None,
                    graphs: &[("disk_active_history", "Active Time")],
                },
                RowSpec {
                    meter: None,
                    graphs: &[
                        ("disk_read_history", "Read"),
                        ("disk_write_history", "Write"),
                    ],
                },
            ],
            figures: &[
                (
                    "activity",
                    "Activity",
                    [
                        ("disk_active", "disk_active_label", "Active time"),
                        ("disk_read", "disk_read_label", "Read speed"),
                        ("disk_write", "disk_write_label", "Write speed"),
                    ],
                ),
                (
                    "disk_totals",
                    "Since Startup",
                    [
                        ("disk_size", "disk_size_label", "Capacity"),
                        ("disk_read_total", "disk_read_total_label", "Read"),
                        ("disk_written", "disk_written_label", "Written"),
                    ],
                ),
            ],
            note: false,
        },
        Pane::Net => &Spec {
            rows: &[
                RowSpec {
                    meter: None,
                    graphs: &[("net_down_history", "Receive")],
                },
                RowSpec {
                    meter: None,
                    graphs: &[("net_up_history", "Send")],
                },
            ],
            figures: &[
                (
                    "throughput",
                    "Throughput",
                    [
                        ("net_down", "net_down_label", "Receive"),
                        ("net_up", "net_up_label", "Send"),
                        ("net_kind", "net_kind_label", "Type"),
                    ],
                ),
                (
                    "net_totals",
                    "Since Connected",
                    [
                        ("net_rx", "net_rx_label", "Received"),
                        ("net_tx", "net_tx_label", "Sent"),
                        ("net_name", "net_name_label", "Adapter"),
                    ],
                ),
            ],
            note: false,
        },
        Pane::Gpu => &Spec {
            rows: &[
                RowSpec {
                    meter: Some(("gpu_meter", "GPU Usage")),
                    graphs: &[("gpu_history", "GPU Usage History")],
                },
                RowSpec {
                    meter: Some(("vram_meter", "Memory")),
                    graphs: &[("vram_history", "Video Memory History")],
                },
            ],
            figures: &[
                (
                    "gpu",
                    "GPU",
                    [
                        ("gpu_busy", "gpu_busy_label", "Usage"),
                        ("gpu_temp", "gpu_temp_label", "Temperature"),
                        ("gpu_fan", "gpu_fan_label", "Fan"),
                    ],
                ),
                (
                    "vram",
                    "Video Memory (MB)",
                    [
                        ("vram_total", "vram_total_label", "Total"),
                        ("vram_used", "vram_used_label", "In use"),
                        ("vram_free", "vram_free_label", "Free"),
                    ],
                ),
            ],
            note: true,
        },
        Pane::Sensors => &Spec {
            rows: &[],
            figures: &[],
            note: false,
        },
    }
}

/// A group box and the rect inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boxed {
    pub group: Rect,
    pub inside: Rect,
}

fn boxed(group: Rect) -> Boxed {
    Boxed {
        group,
        inside: Rect::new(
            group.x + GROUP_EDGE,
            group.y + GROUP_TOP,
            (group.width - 2 * GROUP_EDGE).max(1),
            (group.height - GROUP_TOP - GROUP_EDGE).max(1),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    pub meter: Option<Boxed>,
    pub graphs: Vec<Boxed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub list: Rect,
    pub pane: Pane,
    pub heading: Rect,
    pub rows: Vec<GraphRow>,
    pub figures: Vec<Rect>,
    pub note: Option<Rect>,
    /// The sensors list, on the Sensors pane.
    pub sensors: Option<Rect>,
}

/// The page laid out in `inner` (the tab sheet's inside) for `pane`.
pub fn layout(inner: Rect, pane: Pane) -> Layout {
    let list_w = (inner.width * 2 / 5).clamp(LIST_MIN_W, LIST_W);
    let list = Rect::new(inner.x, inner.y, list_w.min(inner.width / 2), inner.height);
    let px = list.right() + PAD;
    let r = Rect::new(px, inner.y, (inner.right() - px).max(1), inner.height);
    let sp = spec(pane);
    let heading = Rect::new(r.x, r.y, r.width, LINE_H);
    let top = heading.bottom() + 4;
    let (figures, fig_y) = if sp.figures.is_empty() {
        (Vec::new(), r.bottom())
    } else {
        let n = sp.figures.len() as i32;
        let y = r.bottom() - FIGURES_H;
        let w = (r.width - (n - 1) * PAD) / n;
        let rects = (0..n)
            .map(|i| Rect::new(r.x + i * (w + PAD), y, w, FIGURES_H))
            .collect();
        (rects, y)
    };
    let note = sp
        .note
        .then(|| Rect::new(r.x, fig_y - PAD - LINE_H, r.width, LINE_H));
    let rows_bottom = note.map_or(fig_y, |n| n.y) - PAD;
    let n = sp.rows.len() as i32;
    let rows = (sp.rows.iter().enumerate())
        .map(|(i, rs)| {
            let row_h = ((rows_bottom - top - (n - 1) * PAD) / n.max(1)).max(2);
            let y = top + i as i32 * (row_h + PAD);
            let meter = rs
                .meter
                .map(|_| boxed(Rect::new(r.x, y, METER_GROUP_W, row_h)));
            let gx = meter.map_or(r.x, |m| m.group.right() + PAD);
            let k = rs.graphs.len() as i32;
            let gw = ((r.right() - gx - (k - 1) * PAD) / k.max(1)).max(1);
            let graphs = (0..k)
                .map(|j| boxed(Rect::new(gx + j * (gw + PAD), y, gw, row_h)))
                .collect();
            GraphRow { meter, graphs }
        })
        .collect();
    let sensors =
        (pane == Pane::Sensors).then(|| Rect::new(r.x, top, r.width, (r.bottom() - top).max(1)));
    Layout {
        list,
        pane,
        heading,
        rows,
        figures,
        note,
        sensors,
    }
}

/// The form items of `lay`'s pane: heading, group boxes, figure labels and values, note.
pub fn form_items(lay: &Layout) -> Vec<Item> {
    let sp = spec(lay.pane);
    let mut items = vec![Item::label("heading", lay.heading, "")];
    for (rs, row) in sp.rows.iter().zip(&lay.rows) {
        if let (Some((id, title)), Some(m)) = (rs.meter, row.meter) {
            items.push(Item::group(id, m.group, title));
        }
        for ((id, title), g) in rs.graphs.iter().zip(&row.graphs) {
            items.push(Item::group(id, g.group, title));
        }
    }
    for ((id, title, rows), r) in sp.figures.iter().zip(&lay.figures) {
        items.push(Item::group(id, *r, title));
        let w = r.width - 2 * GROUP_EDGE;
        let label_w = w * 3 / 5;
        for (i, (value, label_id, label)) in rows.iter().enumerate() {
            let y = r.y + GROUP_TOP + i as i32 * (LINE_H + 2);
            let x = r.x + GROUP_EDGE;
            items.push(Item::label_at(label_id, x, y, label_w, label));
            items.push(Item::value_at(value, x + label_w, y, w - label_w, ""));
        }
    }
    if let Some(n) = lay.note {
        items.push(Item::label("note", n, ""));
    }
    items
}

/// What a pane's row shows: its meter's fill and value, and each graph's lines.
pub struct Art {
    pub meter: Option<(f64, String)>,
    pub graphs: Vec<Vec<History>>,
}

/// The rate graph title with its scale: "Read (scale 10 MB/s)".
fn scaled_title(title: &str, scale: f64) -> String {
    format!("{title} (scale {})", netspeed::format(scale as u64))
}

/// The figures, titles and graphs of the device `t` has picked.
pub struct Shown {
    pub values: Vec<(&'static str, String)>,
    pub art: Vec<Art>,
}

fn mb(kb: u64) -> String {
    grouped(kb / 1024)
}

fn or_dash(v: Option<String>) -> String {
    v.unwrap_or_else(|| "-".into())
}

/// Memory in use as a meter writes it: MB under a gigabyte, else GB to one place.
fn mem_text(kb: u64) -> String {
    if kb < 1024 * 1024 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    }
}

pub fn shown(t: &TaskMgr) -> Shown {
    let s = &t.snap;
    let d = &s.devices;
    let key = t.perf.selected().clone();
    let hist = t.perf.hist.get(&key);
    let mut values: Vec<(&'static str, String)> = Vec::new();
    let mut art = Vec::new();
    match &key {
        Key::Cpu => {
            let cpu = t.cpu.latest().unwrap_or(0.0);
            values.extend([
                ("heading", d.cpu_name.clone()),
                ("procs", grouped(s.procs.len() as u64)),
                ("threads", grouped(s.threads())),
                ("programs", grouped(s.scopes.len() as u64)),
                ("cores", grouped(t.cores.len() as u64)),
                ("cpu_temp", or_dash(d.cpu_temp.map(celsius))),
                (
                    "cpu_fan",
                    or_dash(d.cpu_fan.map(|v| hwmon::Unit::Rpm.text(v))),
                ),
            ]);
            art.push(Art {
                meter: Some((cpu, format!("{} %", (cpu * 100.0).round()))),
                graphs: vec![vec![t.cpu.clone()]],
            });
        }
        Key::Memory => {
            let m = &s.mem;
            let used = if m.total == 0 {
                0.0
            } else {
                m.used() as f64 / m.total as f64
            };
            values.extend([
                (
                    "heading",
                    format!("{} installed", bytes_text(m.total * 1024)),
                ),
                ("mem_total", mb(m.total)),
                ("mem_avail", mb(m.available)),
                ("mem_cached", mb(m.cached)),
                ("swap_total", mb(m.swap_total)),
                ("swap_used", mb(m.swap_used())),
                ("swap_free", mb(m.swap_free)),
            ]);
            art.push(Art {
                meter: Some((used, mem_text(m.used()))),
                graphs: vec![vec![t.mem.clone(), t.swap.clone()]],
            });
        }
        Key::Disk(name) => {
            let r = d.disks.iter().find(|r| &r.disk.name == name);
            let rates = r.and_then(|r| r.rates);
            let heading = r.map_or_else(String::new, |r| match r.disk.model.as_str() {
                "" => r.disk.name.clone(),
                m => format!("{m} ({})", r.disk.name),
            });
            values.extend([
                ("heading", heading),
                ("disk_active", or_dash(rates.map(|x| pct(x.active)))),
                (
                    "disk_read",
                    or_dash(rates.map(|x| netspeed::format(x.read))),
                ),
                (
                    "disk_write",
                    or_dash(rates.map(|x| netspeed::format(x.write))),
                ),
                ("disk_size", or_dash(r.map(|r| bytes_text(r.disk.bytes)))),
                (
                    "disk_read_total",
                    or_dash(r.map(|r| bytes_text(r.read_total))),
                ),
                (
                    "disk_written",
                    or_dash(r.map(|r| bytes_text(r.written_total))),
                ),
            ]);
            if let Some(Hist::Disk {
                active,
                read,
                write,
            }) = hist
            {
                let (rs, ws) = (
                    nice_scale(read.peak(), DISK_FLOOR),
                    nice_scale(write.peak(), DISK_FLOOR),
                );
                values.push(("disk_read_history", scaled_title("Read", rs)));
                values.push(("disk_write_history", scaled_title("Write", ws)));
                art.push(Art {
                    meter: None,
                    graphs: vec![vec![active.clone()]],
                });
                art.push(Art {
                    meter: None,
                    graphs: vec![vec![read.scaled(rs)], vec![write.scaled(ws)]],
                });
            }
        }
        Key::Net(name) => {
            let n = d.nets.iter().find(|n| &n.adapter.name == name);
            let rate = n.and_then(|n| n.rate);
            values.extend([
                (
                    "heading",
                    n.map_or_else(String::new, |n| {
                        format!("{} adapter {}", n.adapter.kind.label(), n.adapter.name)
                    }),
                ),
                ("net_down", or_dash(rate.map(|r| netspeed::format(r.down)))),
                ("net_up", or_dash(rate.map(|r| netspeed::format(r.up)))),
                (
                    "net_kind",
                    or_dash(n.map(|n| n.adapter.kind.label().into())),
                ),
                ("net_rx", or_dash(n.map(|n| bytes_text(n.total.rx)))),
                ("net_tx", or_dash(n.map(|n| bytes_text(n.total.tx)))),
                ("net_name", or_dash(n.map(|n| n.adapter.name.clone()))),
            ]);
            if let Some(Hist::Net { down, up }) = hist {
                let (ds, us) = (
                    nice_scale(down.peak(), NET_FLOOR),
                    nice_scale(up.peak(), NET_FLOOR),
                );
                values.push(("net_down_history", scaled_title("Receive", ds)));
                values.push(("net_up_history", scaled_title("Send", us)));
                art.push(Art {
                    meter: None,
                    graphs: vec![vec![down.scaled(ds)]],
                });
                art.push(Art {
                    meter: None,
                    graphs: vec![vec![up.scaled(us)]],
                });
            }
        }
        Key::Gpu(card) => {
            let g = d.gpus.iter().find(|g| &g.card.card == card);
            let vram = g.and_then(|g| g.vram);
            let note = match g {
                Some(g) if g.asleep => "The GPU is asleep; it is not read, so it stays asleep.",
                Some(g) if g.source == Source::None => "No usage data.",
                Some(g) if g.source == Source::OwnProcesses && t.uid != 0 => {
                    "Usage counts only your own processes."
                }
                _ => "",
            };
            let busy = g.and_then(|g| g.busy);
            let mib = |b: u64| grouped(b >> 20);
            values.extend([
                (
                    "heading",
                    g.map_or_else(String::new, |g| {
                        format!("{} ({})", g.card.name, g.card.driver)
                    }),
                ),
                ("gpu_busy", or_dash(busy.map(pct))),
                ("gpu_temp", or_dash(g.and_then(|g| g.temp).map(celsius))),
                ("gpu_fan", or_dash(g.and_then(|g| g.fan.clone()))),
                ("vram_total", or_dash(vram.map(|(_, t)| mib(t)))),
                ("vram_used", or_dash(vram.map(|(u, _)| mib(u)))),
                (
                    "vram_free",
                    or_dash(vram.map(|(u, t)| mib(t.saturating_sub(u)))),
                ),
                ("note", note.into()),
            ]);
            if let Some(Hist::Gpu { busy: bh, vram: vh }) = hist {
                let b = busy.unwrap_or(0.0);
                let v = vram.map_or(0.0, |(u, t)| u as f64 / t as f64);
                art.push(Art {
                    meter: Some((
                        b,
                        busy.map_or_else(|| "-".into(), |b| format!("{} %", (b * 100.0).round())),
                    )),
                    graphs: vec![vec![bh.clone()]],
                });
                art.push(Art {
                    meter: Some((
                        v,
                        vram.map_or_else(|| "-".into(), |(u, _)| mem_text(u >> 10)),
                    )),
                    graphs: vec![vec![vh.clone()]],
                });
            }
        }
        Key::Sensors => {
            let n = d.sensors.len();
            let heading = match n {
                0 => "This computer reports no sensors.".to_string(),
                1 => "1 sensor chip".to_string(),
                n => format!("{n} sensor chips"),
            };
            values.push(("heading", heading));
        }
    }
    Shown { values, art }
}

/// Graph rects for `n` per-core graphs inside `r`, in rows of up to [`CORES_PER_ROW`].
pub fn core_rects(r: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let cols = n.min(CORES_PER_ROW) as i32;
    let rows = n.div_ceil(CORES_PER_ROW) as i32;
    let gap = 3;
    let w = ((r.width - gap * (cols - 1)) / cols).max(1);
    let h = ((r.height - gap * (rows - 1)) / rows).max(1);
    (0..n as i32)
        .map(|i| {
            Rect::new(
                r.x + (i % cols) * (w + gap),
                r.y + (i / cols) * (h + gap),
                w,
                h,
            )
        })
        .collect()
}

/// The rect the CPU history graph fills, where its menu opens.
pub fn cpu_graph(lay: &Layout) -> Option<Rect> {
    (lay.pane == Pane::Cpu)
        .then(|| lay.rows.first()?.graphs.first().map(|g| g.group))
        .flatten()
}

/// Device `i`'s sparkline: its main history, or a rate's lines on a shared scale.
fn spark(t: &TaskMgr, i: usize, p: &Paint, scale: i32) -> Canvas {
    let mut c = Canvas::with_scale(SPARK.0 as u32, SPARK.1 as u32, scale);
    let r = Rect::new(0, 0, SPARK.0, SPARK.1);
    let Some(d) = t.perf.devices.get(i) else {
        return c;
    };
    let lines: Vec<History> = match (&d.key, t.perf.hist.get(&d.key)) {
        (Key::Cpu, _) => vec![t.cpu.clone()],
        (Key::Memory, _) => vec![t.mem.clone()],
        (_, Some(Hist::Disk { active, .. })) => vec![active.clone()],
        (_, Some(Hist::Net { down, up })) => {
            let s = nice_scale(down.peak().max(up.peak()), NET_FLOOR);
            vec![down.scaled(s), up.scaled(s)]
        }
        (_, Some(Hist::Gpu { busy, .. })) => vec![busy.clone()],
        (_, Some(Hist::Sensors { hottest })) => vec![hottest.clone()],
        _ => vec![History::new(HISTORY)],
    };
    let refs: Vec<&History> = lines.iter().collect();
    perfgraph::draw(&mut c, p, r, &refs);
    c
}

pub fn draw(t: &TaskMgr, c: &mut Canvas, ctx: &Ctx, lay: &Layout, p: &Paint) {
    let px = ctx.theme.fonts.size_px;
    let cm = ctx.theme.controls();
    let tl = tilelist::layout(&t.perf.list, lay.list, &cm);
    let cs = &ctx.theme.colors;
    tilelist::draw(c, &ctx.text, p, &t.perf.list, &tl, px, |c, r, i, sel| {
        let Some(d) = t.perf.devices.get(i) else {
            return;
        };
        let art = spark(t, i, p, c.scale());
        let tile = Tile {
            icon: Some(&art),
            title: &d.title,
            tag: "",
            caption: &d.figure,
            greyed: false,
        };
        tilelist::draw_tile(c, &ctx.text, cs, r, &tile, sel, px, tl.text);
    });
    if let Some(r) = lay.sensors
        && !t.perf.sensors.view.rows.is_empty()
    {
        c.fill_rect(r, cs.window);
        t.perf.sensors.draw(c, r, ctx);
    }
    let shown = shown(t);
    let per_core = lay.pane == Pane::Cpu && t.per_core && !t.cores.is_empty();
    for (ri, (row, a)) in lay.rows.iter().zip(&shown.art).enumerate() {
        if let (Some(m), Some((f, v))) = (row.meter, &a.meter) {
            perfgraph::draw_meter(c, &ctx.text, p, m.inside, *f, v, px);
        }
        for (gi, (g, lines)) in row.graphs.iter().zip(&a.graphs).enumerate() {
            if per_core && (ri, gi) == (0, 0) {
                for (h, r) in t.cores.iter().zip(core_rects(g.inside, t.cores.len())) {
                    perfgraph::draw(c, p, r, &[h]);
                }
                continue;
            }
            let refs: Vec<&History> = lines.iter().collect();
            perfgraph::draw(c, p, g.inside, &refs);
        }
    }
}

pub fn access(
    t: &TaskMgr,
    tree: &mut Tree,
    ctx: &Ctx,
    lay: &Layout,
) -> reprisekit::access::accesskit::NodeId {
    let cm = ctx.theme.controls();
    let tl = tilelist::layout(&t.perf.list, lay.list, &cm);
    let focus = tilelist::access(
        &t.perf.list,
        &tl,
        |i| {
            (t.perf.devices.get(i))
                .map_or_else(Default::default, |d| (d.title.clone(), d.figure.clone()))
        },
        tree,
        ROOT,
        "devices",
    );
    let sp = spec(lay.pane);
    let shown = shown(t);
    for ((rs, row), a) in sp.rows.iter().zip(&lay.rows).zip(&shown.art) {
        let value = a
            .meter
            .as_ref()
            .map_or_else(String::new, |(_, v)| v.clone());
        if let (Some((_, title)), Some(m), Some((_, v))) = (rs.meter, row.meter, &a.meter) {
            perfgraph::access(tree, ROOT, m.inside, title, v);
        }
        for ((id, title), g) in rs.graphs.iter().zip(&row.graphs) {
            let title = (shown.values.iter())
                .find(|(i, _)| i == id)
                .map_or(*title, |(_, v)| v.as_str());
            perfgraph::access(tree, ROOT, g.inside, title, &value);
        }
    }
    if let Some(r) = lay.sensors
        && !t.perf.sensors.view.rows.is_empty()
    {
        let l = t.perf.sensors.layout(r, ctx);
        t.perf.sensors.view.access(&l, tree, ROOT, "sensors");
    }
    focus
}

impl TaskMgr {
    /// A press on the Performance page's list, sensors or CPU graph; None leaves it to the form.
    pub(crate) fn perf_press(
        &mut self,
        at: Click,
        button: u32,
        lay: &Layout,
        ctx: &Ctx,
    ) -> Option<Reply> {
        let (x, y) = (at.x, at.y);
        let cm = ctx.theme.controls();
        if lay.list.contains(x, y) {
            if button != BTN_LEFT {
                return Some(Reply::none());
            }
            return Some(match self.perf.list.press(lay.list, at, &cm) {
                tilelist::Press::None => Reply::none(),
                _ => Reply::redraw(),
            });
        }
        if let Some(r) = lay.sensors
            && r.contains(x, y)
            && !self.perf.sensors.view.rows.is_empty()
        {
            self.perf.sensors.press(at, button, r, ctx);
            return Some(Reply::redraw());
        }
        if button == BTN_RIGHT
            && let Some(g) = cpu_graph(lay)
            && g.contains(x, y)
        {
            return Some(self.open_history_menu(x, y, ctx, false));
        }
        None
    }

    pub(crate) fn perf_motion(&mut self, x: i32, y: i32, lay: &Layout, ctx: &Ctx) -> bool {
        let cm = ctx.theme.controls();
        let drag = self.perf.list.drag(lay.list, y, &cm);
        let bar = self.perf.list.hover(lay.list, x, y, &cm);
        let sensors = match lay.sensors {
            Some(r) => self.perf.sensors.motion(x, y, r, ctx),
            None => false,
        };
        drag || bar || sensors
    }

    /// A key on the Performance page: the arrows pick a device; the Menu key or Shift+F10 opens
    /// the CPU history graph's menu while its pane shows.
    pub(crate) fn perf_key(&mut self, k: &KeyInput, lay: &Layout, ctx: &Ctx) -> Option<Reply> {
        let menu = k.sym == keysym::MENU || (k.sym == keysym::F10 && k.mods.shift);
        if menu {
            let g = cpu_graph(lay)?;
            return Some(self.open_history_menu(g.x + 20, g.y + 20, ctx, true));
        }
        if k.mods.alt || k.mods.ctrl {
            return None;
        }
        let key = form::special_key(k.sym, k.mods.shift, false)?;
        let moved = self.perf.list.key(&key, lay.list, &ctx.theme.controls());
        moved.then(Reply::redraw)
    }

    pub(crate) fn perf_wheel(
        &mut self,
        rows: i32,
        x: i32,
        y: i32,
        lay: &Layout,
        ctx: &Ctx,
    ) -> bool {
        if lay.list.contains(x, y) {
            return self.perf.list.scroll(rows, lay.list, ctx.theme.text);
        }
        match lay.sensors {
            Some(r) if r.contains(x, y) => self.perf.sensors.wheel(rows, r, ctx),
            _ => false,
        }
    }

    pub(crate) fn perf_release(&mut self, ctx: &Ctx) -> bool {
        let list = self.perf.list.release();
        self.perf.sensors.release(ctx) || list
    }

    pub(crate) fn perf_leave(&mut self, ctx: &Ctx) -> bool {
        let list = self.perf.list.leave();
        self.perf.sensors.leave(ctx) || list
    }
}

#[cfg(test)]
#[path = "perf_tests.rs"]
mod tests;
