//! The Performance page's devices past the CPU and memory: disks, network adapters, GPUs and
//! sensors. [`Sampler`] looks for them every [`RESCAN`] and reads them on every sample.

pub mod disk;
pub mod fdinfo;
pub mod gpu;
pub mod hwmon;
pub mod net;
mod nvml;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reprisefs::netspeed::{self, Counters, Rate};

/// How often the devices are looked for again: a disk or adapter that comes or goes shows by
/// then.
pub const RESCAN: Duration = Duration::from_secs(10);

/// The sensor chips that measure the CPU package.
const CPU_CHIPS: [&str; 4] = ["k10temp", "coretemp", "zenpower", "cpu_thermal"];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DiskRates {
    /// The share of the time with I/O in flight, 0 to 1.
    pub active: f64,
    /// Bytes per second.
    pub read: u64,
    pub write: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskReading {
    pub disk: disk::Disk,
    /// None on the first sample.
    pub rates: Option<DiskRates>,
    /// Bytes since boot.
    pub read_total: u64,
    pub written_total: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetReading {
    pub adapter: net::Adapter,
    pub rate: Option<Rate>,
    /// Bytes since the interface came up.
    pub total: Counters,
}

/// Where a GPU's usage comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The driver's own counter for the whole device (amdgpu, NVML).
    Device,
    /// The busy time of the DRM clients of the processes we can read.
    OwnProcesses,
    #[default]
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuReading {
    pub card: gpu::Card,
    pub busy: Option<f64>,
    pub source: Source,
    /// Video memory in use and in all, in bytes.
    pub vram: Option<(u64, u64)>,
    pub temp: Option<f64>,
    /// The fan as its chip gives it: "1,200 RPM", or NVML's "40%".
    pub fan: Option<String>,
    /// Runtime suspended: nothing is read, so it stays asleep.
    pub asleep: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Readings {
    pub cpu_name: String,
    pub cpu_temp: Option<f64>,
    pub cpu_fan: Option<f64>,
    pub disks: Vec<DiskReading>,
    pub nets: Vec<NetReading>,
    pub gpus: Vec<GpuReading>,
    pub sensors: Vec<hwmon::Chip>,
    /// Each process's share of its busiest GPU engine, 0 to 1, when asked for.
    pub proc_gpu: HashMap<i32, f64>,
}

/// The CPU's name in `/proc/cpuinfo` text.
pub fn cpu_name(s: &str) -> Option<String> {
    s.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| matches!(k.trim(), "model name" | "Model" | "Hardware"))
        .map(|(_, v)| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty())
}

/// The sampler's device side, on the sampling thread.
pub struct Sampler {
    root: PathBuf,
    cpu_name: String,
    scanned: Option<Instant>,
    disks: Vec<disk::Disk>,
    nets: Vec<net::Adapter>,
    cards: Vec<gpu::Card>,
    chips: Vec<hwmon::ChipLayout>,
    names: gpu::Names,
    /// Opened at the first NVIDIA card; `Some(None)` once it failed.
    nvml: Option<Option<nvml::Nvml>>,
    disk_prev: Option<(Instant, HashMap<String, disk::Stat>)>,
    net_meter: netspeed::Meter,
    gpu_meter: fdinfo::Meter,
    /// Each process's DRM fds, by pid and start time.
    drm_fds: HashMap<(i32, u64), Vec<u32>>,
    /// Whether every process's GPU share is wanted (the Processes page's GPU column).
    pub proc_gpu: bool,
}

impl Sampler {
    /// Reads devices under `root`: `/` on a live system.
    pub fn new(root: &Path) -> Sampler {
        let cpu_name = std::fs::read_to_string(root.join("proc/cpuinfo"))
            .ok()
            .and_then(|s| cpu_name(&s))
            .unwrap_or_else(|| "CPU".into());
        Sampler {
            root: root.to_path_buf(),
            cpu_name,
            scanned: None,
            disks: Vec::new(),
            nets: Vec::new(),
            cards: Vec::new(),
            chips: Vec::new(),
            names: gpu::Names::default(),
            nvml: None,
            disk_prev: None,
            net_meter: netspeed::Meter::default(),
            gpu_meter: fdinfo::Meter::default(),
            drm_fds: HashMap::new(),
            proc_gpu: false,
        }
    }

    fn rescan(&mut self, now: Instant) -> bool {
        if self
            .scanned
            .is_some_and(|t| now.saturating_duration_since(t) < RESCAN)
        {
            return false;
        }
        self.scanned = Some(now);
        self.disks = disk::scan(&self.root);
        self.nets = net::scan(&self.root);
        self.cards = gpu::scan(&self.root, &mut self.names);
        self.chips = hwmon::scan(&self.root);
        if self.nvml.is_none() && self.cards.iter().any(|c| c.driver == "nvidia") {
            self.nvml = Some(nvml::Nvml::open());
        }
        true
    }

    /// Reads every device at `now`; `procs` are the processes running (pid, start time), whose
    /// DRM clients give the GPUs without a counter of their own.
    pub fn sample(&mut self, now: Instant, procs: &[(i32, u64)]) -> Readings {
        let rescanned = self.rescan(now);
        let sensors = hwmon::read(&self.chips);
        let cpu_chip = sensors
            .iter()
            .find(|c| CPU_CHIPS.contains(&c.name.as_str()));
        let mut r = Readings {
            cpu_name: self.cpu_name.clone(),
            cpu_temp: cpu_chip.and_then(|c| c.first(hwmon::Unit::Celsius)),
            cpu_fan: cpu_chip.and_then(|c| c.first(hwmon::Unit::Rpm)),
            disks: self.disks(now),
            nets: self.nets(now),
            ..Readings::default()
        };
        let mut gpus: Vec<GpuReading> = self.cards.iter().map(|c| self.gpu(c, &sensors)).collect();
        let fdinfo = self.proc_gpu || gpus.iter().any(|g| g.source == Source::None && !g.asleep);
        if fdinfo {
            let usage = self.clients(now, procs, rescanned);
            for g in &mut gpus {
                if g.source != Source::None || g.asleep {
                    continue;
                }
                if let Some(u) = (g.card.pci.as_ref()).and_then(|p| usage.devices.get(p)) {
                    g.busy = Some(*u);
                    g.source = Source::OwnProcesses;
                }
            }
            r.proc_gpu = usage.procs;
        } else {
            self.gpu_meter = fdinfo::Meter::default();
            self.drm_fds.clear();
        }
        r.gpus = gpus;
        r.sensors = sensors;
        r
    }

    fn disks(&mut self, now: Instant) -> Vec<DiskReading> {
        let stats = disk::read_stats(&self.root);
        let prev = self.disk_prev.replace((now, stats.clone()));
        (self.disks.iter())
            .filter_map(|d| {
                let s = *stats.get(&d.name)?;
                let rates = prev.as_ref().and_then(|(then, p)| {
                    let p = p.get(&d.name)?;
                    let secs = now.saturating_duration_since(*then).as_secs_f64();
                    (secs > 0.0).then(|| disk_rates(*p, s, secs))
                });
                Some(DiskReading {
                    disk: d.clone(),
                    rates,
                    read_total: s.read_sectors * disk::SECTOR,
                    written_total: s.written_sectors * disk::SECTOR,
                })
            })
            .collect()
    }

    fn nets(&mut self, now: Instant) -> Vec<NetReading> {
        let counters: Vec<(String, Counters)> = (self.nets.iter())
            .filter_map(|a| Some((a.name.clone(), netspeed::read_iface(&self.root, &a.name)?)))
            .collect();
        let rates = self
            .net_meter
            .sample_each(now, &counters)
            .unwrap_or_default();
        (self.nets.iter())
            .filter_map(|a| {
                let total = counters.iter().find(|(n, _)| *n == a.name)?.1;
                Some(NetReading {
                    adapter: a.clone(),
                    rate: rates.iter().find(|(n, _)| *n == a.name).map(|(_, r)| *r),
                    total,
                })
            })
            .collect()
    }

    fn gpu(&self, c: &gpu::Card, sensors: &[hwmon::Chip]) -> GpuReading {
        let mut g = GpuReading {
            card: c.clone(),
            busy: None,
            source: Source::None,
            vram: None,
            temp: None,
            fan: None,
            asleep: hwmon::suspended(&c.dev),
        };
        if g.asleep {
            return g;
        }
        if let Some(chip) = sensors
            .iter()
            .find(|s| s.device.as_deref() == Some(c.dev.as_path()))
        {
            g.temp = chip.first(hwmon::Unit::Celsius);
            g.fan = chip
                .first(hwmon::Unit::Rpm)
                .map(|v| hwmon::Unit::Rpm.text(v));
        }
        if c.driver == "amdgpu" {
            g.busy = gpu::amd_busy(&c.dev);
            g.vram = gpu::amd_vram(&c.dev);
        } else if let Some(Some(nv)) = &self.nvml
            && let Some(n) = c.pci.as_deref().and_then(|p| nv.card(p))
        {
            g.card.name = n.name.unwrap_or(g.card.name);
            g.busy = n.busy;
            g.vram = n.vram;
            g.temp = n.temp.or(g.temp);
            g.fan = n.fan.map(|f| format!("{f}%")).or(g.fan);
        }
        if g.busy.is_some() {
            g.source = Source::Device;
        }
        g
    }

    /// Every visible DRM client's usage since the last call. A process's DRM fds are looked for
    /// when it is first seen and again at each rescan.
    fn clients(&mut self, now: Instant, procs: &[(i32, u64)], rescanned: bool) -> fdinfo::Usage {
        let proc = self.root.join("proc");
        if rescanned {
            self.drm_fds.clear();
        }
        let running: std::collections::HashSet<&(i32, u64)> = procs.iter().collect();
        self.drm_fds.retain(|k, _| running.contains(k));
        let mut clients = Vec::new();
        for &(pid, start) in procs {
            let fds = self
                .drm_fds
                .entry((pid, start))
                .or_insert_with(|| fdinfo::drm_fds(&proc, pid));
            for &fd in fds.iter() {
                if let Some(c) = fdinfo::read_client(&proc, pid, fd) {
                    clients.push((pid, c));
                }
            }
        }
        self.gpu_meter.sample(now, &clients).unwrap_or_default()
    }
}

/// A disk's rates between two readings `secs` apart.
pub fn disk_rates(prev: disk::Stat, now: disk::Stat, secs: f64) -> DiskRates {
    let per =
        |a: u64, b: u64| (b.saturating_sub(a) as f64 * disk::SECTOR as f64 / secs).round() as u64;
    DiskRates {
        active: (now.busy_ms.saturating_sub(prev.busy_ms) as f64 / (secs * 1000.0)).clamp(0.0, 1.0),
        read: per(prev.read_sectors, now.read_sectors),
        write: per(prev.written_sectors, now.written_sectors),
    }
}

#[cfg(test)]
mod tests;
