use std::path::PathBuf;

use super::*;
use crate::devices::{self, DiskRates, DiskReading, GpuReading, NetReading, disk, gpu, net};
use crate::tests::{centre, click, ctx, fnv, key, loaded, open, pins, snapshot, themed_ctx};
use crate::{Page, pages};
use repriseapp::{App, Extended, MAIN};
use reprisefs::netspeed::{Counters, Rate};
use reprisekit::interface::Interface;

fn chip(name: &str, readings: &[(&str, hwmon::Unit, f64)]) -> hwmon::Chip {
    hwmon::Chip {
        name: name.into(),
        device: None,
        asleep: false,
        readings: (readings.iter())
            .map(|(label, unit, value)| hwmon::Reading {
                unit: *unit,
                label: (*label).into(),
                value: *value,
            })
            .collect(),
    }
}

fn card(card: &str, driver: &str, name: &str) -> gpu::Card {
    gpu::Card {
        card: card.into(),
        dev: PathBuf::from("/sys/devices/pci0000:00").join(card),
        pci: Some("0000:07:00.0".into()),
        driver: driver.into(),
        name: name.into(),
    }
}

/// A machine with one disk, one adapter, an AMD GPU, a virtio GPU and three sensor chips;
/// `k` moves its rates and loads from sample to sample.
fn readings(k: f64) -> devices::Readings {
    use hwmon::Unit::*;
    devices::Readings {
        cpu_name: "Test CPU 9000".into(),
        cpu_temp: Some(62.0),
        cpu_fan: None,
        disks: vec![DiskReading {
            disk: disk::Disk {
                name: "nvme0n1".into(),
                model: "Disk Model".into(),
                bytes: 1 << 40,
            },
            rates: Some(DiskRates {
                active: 0.5 * k,
                read: (2_097_152.0 * k) as u64,
                write: (300_000.0 * (1.0 - k)) as u64,
            }),
            read_total: 3 << 30,
            written_total: 5 << 30,
        }],
        nets: vec![NetReading {
            adapter: net::Adapter {
                name: "enp4s0".into(),
                kind: net::Kind::Ethernet,
            },
            rate: Some(Rate {
                down: (2000.0 * k) as u64,
                up: 10,
            }),
            total: Counters {
                rx: 3_000_000,
                tx: 10_000,
            },
        }],
        gpus: vec![
            GpuReading {
                card: card("card1", "amdgpu", "AMD/ATI Test GPU"),
                busy: Some(0.4 * k),
                source: devices::Source::Device,
                vram: Some((1 << 30, 4 << 30)),
                temp: Some(52.0),
                fan: None,
                asleep: false,
            },
            GpuReading {
                card: card("card2", "virtio_gpu", "Red Hat Virtio 1.0 GPU"),
                busy: None,
                source: devices::Source::None,
                vram: None,
                temp: None,
                fan: None,
                asleep: false,
            },
        ],
        sensors: vec![
            chip("k10temp", &[("Tctl", Celsius, 62.0)]),
            chip(
                "amdgpu",
                &[
                    ("edge", Celsius, 52.0),
                    ("vddgfx", Volts, 1.218),
                    ("PPT", Watts, 6.5),
                ],
            ),
            chip("thinkpad", &[("fan1", Rpm, 2950.0)]),
        ],
        proc_gpu: Default::default(),
    }
}

fn with_devices(k: f64) -> Snapshot {
    Snapshot {
        devices: readings(k),
        ..snapshot()
    }
}

fn perf_mgr(ctx: &mut Ctx) -> TaskMgr {
    let mut t = loaded(ctx);
    for i in 0..30 {
        let k = 0.5 + (f64::from(i) * 0.4).sin() * 0.5;
        let mut s = with_devices(k);
        s.cpu = Some(0.4 + (f64::from(i) * 0.3).sin() * 0.3);
        t.take(s, ctx);
    }
    open(&mut t, ctx, Page::Performance);
    t
}

fn text(t: &TaskMgr, id: &str) -> String {
    match t.form.item(id).map(|i| &i.control) {
        Some(form::Control::Label { text, .. }) => text.clone(),
        other => panic!("{id}: {other:?}"),
    }
}

fn pick(t: &mut TaskMgr, k: &Key) {
    let i = t.perf.devices.iter().position(|d| &d.key == k).unwrap();
    t.perf.list.select_row(Some(i));
}

#[test]
fn the_device_list_holds_each_device_with_its_figure() {
    let rows = devices(&with_devices(1.0));
    let got: Vec<(&str, &str)> = (rows.iter())
        .map(|d| (d.title.as_str(), d.figure.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("CPU", "25%  62 \u{b0}C"),
            ("Memory", "2.0 GB/8.0 GB (25%)"),
            ("Disk 0 (nvme0n1)", "50%"),
            ("Ethernet (enp4s0)", "\u{2193} 2.0 KB/s \u{2191} 10 B/s"),
            ("GPU 0", "40%  52 \u{b0}C"),
            ("GPU 1", "No usage data"),
            ("Sensors", "62 \u{b0}C"),
        ]
    );
    let bare = devices(&Snapshot::default());
    let keys: Vec<&Key> = bare.iter().map(|d| &d.key).collect();
    assert_eq!(keys, [&Key::Cpu, &Key::Memory, &Key::Sensors]);
    assert_eq!(bare[2].figure, "None");
}

#[test]
fn the_picked_device_stays_picked_across_samples_and_falls_back_to_the_cpu() {
    let mut ctx = ctx();
    let mut t = perf_mgr(&mut ctx);
    assert_eq!(t.perf.selected(), &Key::Cpu);
    let net = Key::Net("enp4s0".into());
    pick(&mut t, &net);
    t.take(with_devices(0.3), &mut ctx);
    assert_eq!(t.perf.selected(), &net);
    let mut gone = with_devices(0.3);
    gone.devices.nets.clear();
    t.take(gone, &mut ctx);
    assert_eq!(t.perf.selected(), &Key::Cpu);
    assert!(!t.perf.hist.contains_key(&net), "its history goes with it");
}

#[test]
fn a_click_on_a_row_picks_its_device_and_the_arrows_move_down_the_list() {
    let mut ctx = ctx();
    let mut t = perf_mgr(&mut ctx);
    let lay = pages::layout(&t, &ctx).perf.unwrap();
    let tl = tilelist::layout(&t.perf.list, lay.list, &ctx.theme.controls());
    t.take(with_devices(1.0), &mut ctx);
    click(&mut t, &mut ctx, centre(tl.slots[2].rect));
    assert_eq!(t.perf.selected(), &Key::Disk("nvme0n1".into()));
    pages::follow_form(&mut t, &ctx);
    assert_eq!(text(&t, "disk_active"), "50%");
    assert_eq!(text(&t, "disk_size"), "1.0 TB");
    let title = match t.form.item("disk_read_history").map(|i| &i.control) {
        Some(form::Control::Group(title)) => title.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        title, "Read (scale 5.0 MB/s)",
        "the scale over the history's peak"
    );
    t.event(MAIN, key(keysym::DOWN), &mut ctx);
    assert_eq!(t.perf.selected(), &Key::Net("enp4s0".into()));
    pages::follow_form(&mut t, &ctx);
    assert_eq!(text(&t, "net_kind"), "Ethernet");
}

#[test]
fn rate_graphs_scale_to_a_round_figure_over_their_peak() {
    assert_eq!(nice_scale(0.0, 1e6), 1e6);
    assert_eq!(nice_scale(1.3e6, 1e6), 2e6);
    assert_eq!(nice_scale(2e6, 1e6), 2e6);
    assert_eq!(nice_scale(5.1e7, 1e6), 1e8);
    assert_eq!(bytes_text(512 << 20), "512 MB");
    assert_eq!(bytes_text(3 << 29), "1.5 GB");
    assert_eq!(bytes_text(1 << 40), "1.0 TB");
}

#[test]
fn the_gpu_pane_says_where_its_usage_comes_from() {
    let mut ctx = ctx();
    let mut t = perf_mgr(&mut ctx);
    pick(&mut t, &Key::Gpu("card2".into()));
    pages::follow_form(&mut t, &ctx);
    assert_eq!(text(&t, "note"), "No usage data.");
    assert_eq!(text(&t, "heading"), "Red Hat Virtio 1.0 GPU (virtio_gpu)");
    assert_eq!(text(&t, "gpu_busy"), "-");
    let mut s = with_devices(1.0);
    s.devices.gpus[1].busy = Some(0.1);
    s.devices.gpus[1].source = devices::Source::OwnProcesses;
    t.take(s, &mut ctx);
    pages::follow_form(&mut t, &ctx);
    assert_eq!(text(&t, "note"), "Usage counts only your own processes.");
    pick(&mut t, &Key::Gpu("card1".into()));
    pages::follow_form(&mut t, &ctx);
    assert_eq!(text(&t, "note"), "");
    assert_eq!(text(&t, "vram_total"), "4,096");
}

#[test]
fn each_pane_fits_its_graphs_and_figures_inside_the_page_at_either_size() {
    let mut ctx = ctx();
    let t = perf_mgr(&mut ctx);
    let panes = [
        Pane::Cpu,
        Pane::Memory,
        Pane::Disk,
        Pane::Net,
        Pane::Gpu,
        Pane::Sensors,
    ];
    for size in [pages::CLIENT, pages::MIN] {
        ctx.size = size;
        let inner = pages::layout(&t, &ctx).inner;
        for pane in panes {
            let lay = layout(inner, pane);
            let mut rects = vec![lay.list, lay.heading];
            for r in &lay.rows {
                rects.extend(r.meter.map(|m| m.group));
                rects.extend(r.graphs.iter().map(|g| g.group));
            }
            rects.extend(&lay.figures);
            rects.extend(lay.note);
            rects.extend(lay.sensors);
            for (i, a) in rects.iter().enumerate() {
                let inside = inner.contains(a.x, a.y)
                    && a.right() <= inner.right()
                    && a.bottom() <= inner.bottom();
                assert!(inside, "{size:?} {pane:?}: {a:?} inside {inner:?}");
                for b in &rects[i + 1..] {
                    assert!(
                        a.intersect(b).is_empty(),
                        "{size:?} {pane:?}: {a:?} clear of {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_panes_are_pinned_in_every_interface_at_1_and_2() {
    reprisekit::pins::fonts();
    let keys = [
        Key::Cpu,
        Key::Memory,
        Key::Disk("nvme0n1".into()),
        Key::Net("enp4s0".into()),
        Key::Gpu("card1".into()),
        Key::Gpu("card2".into()),
        Key::Sensors,
    ];
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = perf_mgr(&mut ctx);
        for k in &keys {
            pick(&mut t, k);
            for d in [1, 2] {
                t.follow_scale(MAIN, d, &mut ctx);
                let (w, h) = ctx.size;
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                got.push((format!("{i:?} {k:?} @{d}"), fnv(&c)));
            }
        }
        t.per_core = true;
        pick(&mut t, &Key::Cpu);
        let (w, h) = ctx.size;
        let mut c = Canvas::with_scale(w as u32, h as u32, 1);
        t.draw(MAIN, &mut c, &ctx);
        got.push((format!("{i:?} per core @1"), fnv(&c)));
    }
    pins("perf", &got);
}

#[test]
#[ignore]
fn render_panes() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = perf_mgr(&mut ctx);
        for k in t
            .perf
            .devices
            .iter()
            .map(|d| d.key.clone())
            .collect::<Vec<_>>()
        {
            pick(&mut t, &k);
            let (w, h) = ctx.size;
            for d in [1, 2] {
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                let name = format!("{k:?}").replace(['(', ')', '"'], "");
                reprisekit::raster::write_preview(
                    &c,
                    &format!("taskmgr-perf-{i:?}-{name}@{d}.png"),
                );
            }
        }
    }
}
