//! Network throughput from the kernel's byte counters: which interfaces count, reading
//! `/sys/class/net/*/statistics`, the rate between two samples and how it is written.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub rx: u64,
    pub tx: u64,
}

/// Bytes per second each way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rate {
    pub down: u64,
    pub up: u64,
}

/// What decides whether an interface's traffic counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    /// It has a `device` link: a NIC, not a bridge, tunnel or veth.
    pub physical: bool,
    /// It is enslaved to a bridge or bond, whose own counters carry its traffic.
    pub port: bool,
    pub up: bool,
    pub default_route: bool,
}

/// Whether `i` counts: loopback and ports never; a virtual interface only while it carries a
/// default route; anything else while it is up.
pub fn counts(i: &Iface) -> bool {
    if i.name == "lo" || i.port {
        return false;
    }
    if i.physical { i.up } else { i.default_route }
}

/// The counters of every interface under `root` (`/` on a live system) that [`counts`].
pub fn read(root: &Path) -> Vec<(String, Counters)> {
    let net = root.join("sys/class/net");
    let routes = default_routes(root);
    let Ok(dir) = std::fs::read_dir(&net) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Counters)> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let at = net.join(&name);
            let operstate = std::fs::read_to_string(at.join("operstate")).unwrap_or_default();
            let iface = Iface {
                physical: at.join("device").exists(),
                port: at.join("master").exists(),
                up: matches!(operstate.trim(), "up" | "unknown"),
                default_route: routes.contains(&name),
                name,
            };
            if !counts(&iface) {
                return None;
            }
            Some((iface.name, stats(&at)?))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The counters of interface `name` under `root`, whether it [`counts`] or not.
pub fn read_iface(root: &Path, name: &str) -> Option<Counters> {
    stats(&root.join("sys/class/net").join(name))
}

fn stats(at: &Path) -> Option<Counters> {
    let stat = |f: &str| -> Option<u64> {
        std::fs::read_to_string(at.join("statistics").join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some(Counters {
        rx: stat("rx_bytes")?,
        tx: stat("tx_bytes")?,
    })
}

/// The interfaces holding a default route in the main IPv4 or IPv6 table.
fn default_routes(root: &Path) -> Vec<String> {
    let v4 = std::fs::read_to_string(root.join("proc/net/route")).unwrap_or_default();
    let v6 = std::fs::read_to_string(root.join("proc/net/ipv6_route")).unwrap_or_default();
    let mut out: Vec<String> = v4
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() > 7 && f[1] == "00000000" && f[7] == "00000000").then(|| f[0].to_string())
        })
        .chain(v6.lines().filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let any = "0".repeat(32);
            (f.len() == 10 && f[0] == any && f[1] == "00" && f[9] != "lo").then(|| f[9].to_string())
        }))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The last sample, from which the next one makes a [`Rate`].
#[derive(Debug, Default)]
pub struct Meter {
    last: Option<(Instant, HashMap<String, Counters>)>,
}

impl Meter {
    /// Take `now`'s counters; the rate since the previous sample, None on the first. An
    /// interface new since then adds nothing, and one whose counter went down (the interface was
    /// recreated, or a 32-bit counter wrapped) adds nothing that way until the next sample.
    pub fn sample(&mut self, now: Instant, counters: &[(String, Counters)]) -> Option<Rate> {
        let (secs, moved) = self.moved(now, counters)?;
        let (down, up) = (moved.iter()).fold((0, 0), |(d, u), (_, m)| (d + m.rx, u + m.tx));
        Some(Rate {
            down: per_sec(down, secs),
            up: per_sec(up, secs),
        })
    }

    /// [`Meter::sample`] for each interface on its own, in `counters`' order; one new since the
    /// previous sample is left out.
    pub fn sample_each(
        &mut self,
        now: Instant,
        counters: &[(String, Counters)],
    ) -> Option<Vec<(String, Rate)>> {
        let (secs, moved) = self.moved(now, counters)?;
        Some(
            moved
                .into_iter()
                .map(|(name, m)| {
                    let r = Rate {
                        down: per_sec(m.rx, secs),
                        up: per_sec(m.tx, secs),
                    };
                    (name, r)
                })
                .collect(),
        )
    }

    /// The seconds since the previous sample and the bytes each interface it knew moved since.
    fn moved(
        &mut self,
        now: Instant,
        counters: &[(String, Counters)],
    ) -> Option<(f64, Vec<(String, Counters)>)> {
        let next: HashMap<String, Counters> = counters.iter().cloned().collect();
        let (then, prev) = self.last.replace((now, next))?;
        let secs = now.saturating_duration_since(then).as_secs_f64();
        if secs <= 0.0 {
            return None;
        }
        let moved = (counters.iter())
            .filter_map(|(name, c)| {
                let p = prev.get(name)?;
                let m = Counters {
                    rx: c.rx.saturating_sub(p.rx),
                    tx: c.tx.saturating_sub(p.tx),
                };
                Some((name.clone(), m))
            })
            .collect();
        Some((secs, moved))
    }

    pub fn clear(&mut self) {
        self.last = None;
    }
}

fn per_sec(bytes: u64, secs: f64) -> u64 {
    (bytes as f64 / secs).round() as u64
}

/// `bps` in SI units: "0 B/s", "999 B/s", "2.4 KB/s", "120 KB/s", "1.0 MB/s", "12 GB/s". One
/// decimal below 10, none above, so a reading is at most three digits and the unit.
pub fn format(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    if bps < 1000 {
        return format!("{bps} B/s");
    }
    let mut v = bps as f64;
    let mut unit = 0;
    while unit + 1 < UNITS.len() && v >= 999.5 {
        v /= 1000.0;
        unit += 1;
    }
    if v < 9.95 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

/// The two readings with their arrows: "↓ 2.4 MB/s", "↑ 120 KB/s".
pub fn lines(r: Rate) -> [String; 2] {
    [
        format!("\u{2193} {}", format(r.down)),
        format!("\u{2191} {}", format(r.up)),
    ]
}

/// Both readings on one line: "↓ 2.4 MB/s ↑ 120 KB/s".
pub fn line(r: Rate) -> String {
    let [d, u] = lines(r);
    format!("{d} {u}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn c(name: &str, rx: u64, tx: u64) -> (String, Counters) {
        (name.into(), Counters { rx, tx })
    }

    #[test]
    fn the_first_sample_has_no_rate_and_the_second_divides_by_the_time_between() {
        let mut m = Meter::default();
        let t = Instant::now();
        assert_eq!(m.sample(t, &[c("eth0", 1000, 50)]), None);
        let r = m.sample(
            t + Duration::from_millis(500),
            &[c("eth0", 2_201_000, 60_050)],
        );
        assert_eq!(
            r,
            Some(Rate {
                down: 4_400_000,
                up: 120_000
            })
        );
    }

    #[test]
    fn interfaces_add_up_and_a_new_one_waits_a_sample() {
        let mut m = Meter::default();
        let t = Instant::now();
        m.sample(t, &[c("eth0", 0, 0)]);
        let r = m.sample(
            t + Duration::from_secs(1),
            &[c("eth0", 100, 10), c("wlan0", 5_000_000, 5_000_000)],
        );
        assert_eq!(r, Some(Rate { down: 100, up: 10 }));
        let r = m.sample(
            t + Duration::from_secs(2),
            &[c("eth0", 200, 20), c("wlan0", 5_000_300, 5_000_030)],
        );
        assert_eq!(r, Some(Rate { down: 400, up: 40 }));
    }

    #[test]
    fn each_interface_has_its_own_rate_and_a_new_one_waits_a_sample() {
        let mut m = Meter::default();
        let t = Instant::now();
        assert_eq!(m.sample_each(t, &[c("eth0", 0, 0)]), None);
        let r = m.sample_each(
            t + Duration::from_millis(500),
            &[c("eth0", 100, 10), c("wlan0", 7, 7)],
        );
        assert_eq!(r, Some(vec![("eth0".into(), Rate { down: 200, up: 20 })]));
    }

    #[test]
    fn a_counter_going_down_reads_as_zero_not_a_spike() {
        let mut m = Meter::default();
        let t = Instant::now();
        m.sample(t, &[c("eth0", 9_000_000, u64::from(u32::MAX) - 10)]);
        let reset = m.sample(t + Duration::from_secs(1), &[c("eth0", 500, 20)]);
        assert_eq!(reset, Some(Rate { down: 0, up: 0 }));
        let after = m.sample(t + Duration::from_secs(2), &[c("eth0", 1500, 40)]);
        assert_eq!(after, Some(Rate { down: 1000, up: 20 }));
    }

    #[test]
    fn a_cleared_meter_starts_over() {
        let mut m = Meter::default();
        let t = Instant::now();
        m.sample(t, &[c("eth0", 0, 0)]);
        m.clear();
        assert_eq!(
            m.sample(t + Duration::from_secs(1), &[c("eth0", 9, 9)]),
            None
        );
    }

    #[test]
    fn readings_scale_by_thousands_with_one_decimal_under_ten() {
        let cases = [
            (0, "0 B/s"),
            (999, "999 B/s"),
            (1000, "1.0 KB/s"),
            (2449, "2.4 KB/s"),
            (9949, "9.9 KB/s"),
            (9950, "10 KB/s"),
            (120_000, "120 KB/s"),
            (999_499, "999 KB/s"),
            (999_500, "1.0 MB/s"),
            (2_400_000, "2.4 MB/s"),
            (12_000_000_000, "12 GB/s"),
            (5_000_000_000_000, "5000 GB/s"),
        ];
        for (bps, want) in cases {
            assert_eq!(format(bps), want, "{bps}");
        }
    }

    #[test]
    fn the_line_puts_down_before_up() {
        let r = Rate {
            down: 2_400_000,
            up: 120_000,
        };
        assert_eq!(line(r), "\u{2193} 2.4 MB/s \u{2191} 120 KB/s");
    }

    #[test]
    fn loopback_ports_and_routeless_virtual_interfaces_do_not_count() {
        let i = |name: &str, physical, port, up, default_route| Iface {
            name: name.into(),
            physical,
            port,
            up,
            default_route,
        };
        assert!(!counts(&i("lo", false, false, true, true)));
        assert!(counts(&i("enp3s0", true, false, true, false)));
        assert!(!counts(&i("enp4s0", true, false, false, false)));
        assert!(
            !counts(&i("enp5s0", true, true, true, false)),
            "bridge port"
        );
        assert!(!counts(&i("docker0", false, false, true, false)));
        assert!(counts(&i("br0", false, false, true, true)));
        assert!(counts(&i("ppp0", false, false, true, true)));
    }

    #[test]
    fn read_walks_sysfs_and_the_route_tables() {
        let root = tempfile::tempdir().unwrap();
        let net = root.path().join("sys/class/net");
        let iface = |name: &str, device: bool, master: bool, state: &str, rx: u64, tx: u64| {
            let d = net.join(name);
            std::fs::create_dir_all(d.join("statistics")).unwrap();
            if device {
                std::fs::create_dir(d.join("device")).unwrap();
            }
            if master {
                std::fs::create_dir(d.join("master")).unwrap();
            }
            std::fs::write(d.join("operstate"), format!("{state}\n")).unwrap();
            std::fs::write(d.join("statistics/rx_bytes"), format!("{rx}\n")).unwrap();
            std::fs::write(d.join("statistics/tx_bytes"), format!("{tx}\n")).unwrap();
        };
        iface("lo", false, false, "unknown", 1, 1);
        iface("enp3s0", true, false, "up", 10, 20);
        iface("wlan0", true, false, "down", 30, 40);
        iface("virbr0", false, false, "up", 50, 60);
        iface("wg0", false, false, "unknown", 70, 80);
        iface("eth9", true, true, "up", 90, 100);
        let proc_net = root.path().join("proc/net");
        std::fs::create_dir_all(&proc_net).unwrap();
        std::fs::write(
            proc_net.join("route"),
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
             enp3s0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
             virbr0\t007AA8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n",
        )
        .unwrap();
        std::fs::write(
            proc_net.join("ipv6_route"),
            format!(
                "{z} 00 {z} 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wg0\n",
                z = "0".repeat(32)
            ),
        )
        .unwrap();
        let got = read(root.path());
        assert_eq!(got, vec![c("enp3s0", 10, 20), c("wg0", 70, 80)]);
        assert_eq!(
            read_iface(root.path(), "virbr0"),
            Some(Counters { rx: 50, tx: 60 })
        );
        assert_eq!(read_iface(root.path(), "eth7"), None);
    }
}
