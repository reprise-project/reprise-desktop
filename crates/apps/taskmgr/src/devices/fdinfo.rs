//! GPU use from DRM fdinfo (the kernel's drm-usage-stats): each open DRM client's busy time per
//! engine, read from the fds of the processes we can see.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// An engine's busy counter: nanoseconds busy, or (xe) busy GPU cycles against the cycles that
/// passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Ns(u64),
    Cycles { busy: u64, total: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    pub name: String,
    pub busy: Busy,
    /// How many of this engine the device has: its busy time can reach capacity times the time
    /// that passed.
    pub capacity: u32,
}

/// One DRM client (one open of the device, shared by every fd duplicated from it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// The PCI address of the device: `0000:00:02.0`.
    pub pdev: String,
    pub id: u64,
    pub engines: Vec<Engine>,
}

/// A client in fdinfo text, None unless it is a DRM client with a device and an id.
pub fn parse(s: &str) -> Option<Client> {
    let mut pdev = None;
    let mut id = None;
    let mut ns: Vec<(String, u64)> = Vec::new();
    let mut cycles: HashMap<String, u64> = HashMap::new();
    let mut totals: HashMap<String, u64> = HashMap::new();
    let mut capacity: HashMap<String, u32> = HashMap::new();
    let mut drm = false;
    for line in s.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let number = || value.split_whitespace().next()?.parse::<u64>().ok();
        if key == "drm-driver" {
            drm = true;
        } else if key == "drm-pdev" {
            pdev = Some(value.to_string());
        } else if key == "drm-client-id" {
            id = number();
        } else if let Some(e) = key.strip_prefix("drm-engine-capacity-") {
            if let Some(n) = number() {
                capacity.insert(e.into(), n.max(1) as u32);
            }
        } else if let Some(e) = key.strip_prefix("drm-engine-") {
            if value.ends_with("ns")
                && let Some(n) = number()
            {
                ns.push((e.into(), n));
            }
        } else if let Some(e) = key.strip_prefix("drm-total-cycles-") {
            if let Some(n) = number() {
                totals.insert(e.into(), n);
            }
        } else if let Some(e) = key.strip_prefix("drm-cycles-")
            && let Some(n) = number()
        {
            cycles.insert(e.into(), n);
        }
    }
    if !drm {
        return None;
    }
    let cap = |e: &str| capacity.get(e).copied().unwrap_or(1);
    let mut engines: Vec<Engine> = ns
        .into_iter()
        .map(|(name, n)| Engine {
            capacity: cap(&name),
            busy: Busy::Ns(n),
            name,
        })
        .collect();
    let mut cyc: Vec<(String, u64)> = cycles.into_iter().collect();
    cyc.sort();
    for (name, busy) in cyc {
        if let Some(&total) = totals.get(&name) {
            engines.push(Engine {
                capacity: cap(&name),
                busy: Busy::Cycles { busy, total },
                name,
            });
        }
    }
    Some(Client {
        pdev: pdev?,
        id: id?,
        engines,
    })
}

/// The fds of process `pid` under `proc` that are open on a DRM device; empty when they cannot
/// be read (another user's process).
pub fn drm_fds(proc: &Path, pid: i32) -> Vec<u32> {
    let dir = proc.join(pid.to_string()).join("fd");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<u32> = rd
        .flatten()
        .filter_map(|e| {
            let fd: u32 = e.file_name().to_str()?.parse().ok()?;
            let to = std::fs::read_link(e.path()).ok()?;
            to.starts_with("/dev/dri/").then_some(fd)
        })
        .collect();
    out.sort_unstable();
    out
}

pub fn read_client(proc: &Path, pid: i32, fd: u32) -> Option<Client> {
    let s = std::fs::read_to_string(proc.join(format!("{pid}/fdinfo/{fd}"))).ok()?;
    parse(&s)
}

/// What one sample of every visible client came to: each device's busiest engine and each
/// process's, as fractions 0 to 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub devices: HashMap<String, f64>,
    pub procs: HashMap<i32, f64>,
}

/// An engine of a client: device, client id and engine name.
type EngineKey = (String, u64, String);

/// Turns successive readings of every client into [`Usage`].
#[derive(Debug, Default)]
pub struct Meter {
    last: Option<(Instant, HashMap<EngineKey, Busy>)>,
}

impl Meter {
    /// Takes the clients read at `now`, each with the process it was found in (a client open in
    /// two processes counts for the first); None on the first sample.
    pub fn sample(&mut self, now: Instant, clients: &[(i32, Client)]) -> Option<Usage> {
        let mut next = HashMap::new();
        let mut owner: Vec<(i32, &Client)> = Vec::new();
        for (pid, c) in clients {
            if owner.iter().any(|(_, o)| o.pdev == c.pdev && o.id == c.id) {
                continue;
            }
            owner.push((*pid, c));
            for e in &c.engines {
                next.insert((c.pdev.clone(), c.id, e.name.clone()), e.busy);
            }
        }
        let (then, prev) = self.last.replace((now, next))?;
        let wall_ns = now.saturating_duration_since(then).as_nanos() as f64;
        if wall_ns <= 0.0 {
            return None;
        }
        // Per (device, engine) and per (process, device, engine): busy over what could be busy.
        let mut dev: HashMap<(String, String), (f64, f64)> = HashMap::new();
        let mut per: HashMap<(i32, String, String), (f64, f64)> = HashMap::new();
        for (pid, c) in owner {
            for e in &c.engines {
                let Some(p) = prev.get(&(c.pdev.clone(), c.id, e.name.clone())) else {
                    continue;
                };
                let (busy, span) = match (*p, e.busy) {
                    (Busy::Ns(a), Busy::Ns(b)) => {
                        (b.saturating_sub(a) as f64, wall_ns * f64::from(e.capacity))
                    }
                    (Busy::Cycles { busy: a, total: ta }, Busy::Cycles { busy: b, total: tb }) => (
                        b.saturating_sub(a) as f64,
                        tb.saturating_sub(ta) as f64 * f64::from(e.capacity),
                    ),
                    _ => continue,
                };
                let d = dev
                    .entry((c.pdev.clone(), e.name.clone()))
                    .or_insert((0.0, 0.0));
                d.0 += busy;
                d.1 = d.1.max(span);
                let s = per
                    .entry((pid, c.pdev.clone(), e.name.clone()))
                    .or_insert((0.0, 0.0));
                s.0 += busy;
                s.1 = s.1.max(span);
            }
        }
        let frac = |(busy, span): (f64, f64)| {
            if span > 0.0 {
                (busy / span).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let mut usage = Usage::default();
        for ((pdev, _), v) in dev {
            let u = usage.devices.entry(pdev).or_insert(0.0);
            *u = u.max(frac(v));
        }
        for ((pid, _, _), v) in per {
            let u = usage.procs.entry(pid).or_insert(0.0);
            *u = u.max(frac(v));
        }
        Some(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const I915: &str = "pos:\t0\nflags:\t02104002\ndrm-driver:\ti915\ndrm-client-id:\t392\n\
        drm-pdev:\t0000:00:02.0\ndrm-total-system0:\t236156 KiB\n\
        drm-engine-render:\t1893538181184 ns\ndrm-engine-copy:\t0 ns\n\
        drm-engine-video:\t0 ns\ndrm-engine-capacity-video:\t2\n";

    #[test]
    fn an_i915_client_reads_its_engines_in_nanoseconds() {
        let c = parse(I915).unwrap();
        assert_eq!((c.pdev.as_str(), c.id), ("0000:00:02.0", 392));
        let names: Vec<&str> = c.engines.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["render", "copy", "video"],
            "capacity lines are not engines"
        );
        assert_eq!(c.engines[0].busy, Busy::Ns(1_893_538_181_184));
        assert_eq!(c.engines[2].capacity, 2);
    }

    #[test]
    fn an_xe_client_reads_cycles_against_total_cycles() {
        let s = "drm-driver:\txe\ndrm-client-id:\t7\ndrm-pdev:\t0000:00:02.0\n\
                 drm-cycles-rcs:\t28257900\ndrm-total-cycles-rcs:\t7655183225\n\
                 drm-cycles-bcs:\t0\n";
        let c = parse(s).unwrap();
        assert_eq!(
            c.engines,
            [Engine {
                name: "rcs".into(),
                busy: Busy::Cycles {
                    busy: 28_257_900,
                    total: 7_655_183_225
                },
                capacity: 1,
            }],
            "an engine without its total is left out"
        );
    }

    #[test]
    fn a_file_that_is_not_drm_is_no_client() {
        assert_eq!(parse("pos:\t0\nflags:\t02\nmnt_id:\t15\n"), None);
    }

    fn client(pdev: &str, id: u64, engines: &[(&str, Busy)]) -> Client {
        Client {
            pdev: pdev.into(),
            id,
            engines: (engines.iter())
                .map(|(n, b)| Engine {
                    name: (*n).into(),
                    busy: *b,
                    capacity: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn usage_is_the_busiest_engine_summed_over_clients() {
        let mut m = Meter::default();
        let t = Instant::now();
        let gpu = "0000:00:02.0";
        let at = |render_a: u64, render_b: u64, video_b: u64, dup: bool| {
            let mut v = vec![
                (10, client(gpu, 1, &[("render", Busy::Ns(render_a))])),
                (
                    11,
                    client(
                        gpu,
                        2,
                        &[("render", Busy::Ns(render_b)), ("video", Busy::Ns(video_b))],
                    ),
                ),
            ];
            if dup {
                v.push((12, client(gpu, 2, &[("render", Busy::Ns(render_b))])));
            }
            v
        };
        assert_eq!(m.sample(t, &at(0, 0, 0, false)), None);
        let u = m
            .sample(
                t + Duration::from_secs(1),
                &at(200_000_000, 300_000_000, 100_000_000, true),
            )
            .unwrap();
        assert!(
            (u.devices[gpu] - 0.5).abs() < 1e-9,
            "render: 0.2 + 0.3 of a second"
        );
        assert!((u.procs[&11] - 0.3).abs() < 1e-9);
        assert!((u.procs[&10] - 0.2).abs() < 1e-9);
        assert!(
            !u.procs.contains_key(&12),
            "a client seen twice counts once"
        );
    }

    #[test]
    fn cycles_divide_by_the_cycles_that_passed() {
        let mut m = Meter::default();
        let t = Instant::now();
        let c = |busy, total| vec![(5, client("p", 1, &[("rcs", Busy::Cycles { busy, total })]))];
        m.sample(t, &c(0, 1000));
        let u = m
            .sample(t + Duration::from_millis(10), &c(250, 2000))
            .unwrap();
        assert!((u.devices["p"] - 0.25).abs() < 1e-9);
    }
}
