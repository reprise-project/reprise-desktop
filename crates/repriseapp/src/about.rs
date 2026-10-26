//! The strings every Reprise About box shows and the `/proc` and `/sys` reads behind them; parsing is split from IO so it can be tested on fixture text. The small box goes through [`fields`], reprise-about shows the system view from [`SystemFacts`].

use crate::heads::Head;
use reprisefs::sysinfo::{cpu_model, mem_total_kb};
use reprisekit::controls::aboutbox::{Fields, Group};
use reprisekit::raster::Canvas;
use reprisekit::text::escape_mnemonic;
use reprisekit::tr;

/// The build id a package compiles in (its PKGBUILD exports `REPRISE_BUILD`); `None` in a plain cargo build.
const BUILD: Option<&str> = option_env!("REPRISE_BUILD");

/// Every string an About box shows for `product` (the row under the banner) wearing `icon` (32x32; `None` leaves the slot empty).
pub fn fields(product: &str, icon: Option<Canvas>) -> Fields {
    Machine::read().fields(product, icon)
}

/// The machine's rows of an About box, read once. A fact that cannot be read is left empty, which blanks its row without moving the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub version: String,
    pub facts: [String; 3],
    pub memory: String,
}

impl Machine {
    /// Read `/proc`, `/sys` and the free space on `/`. Anything read is escaped: the label control parses `&` as a mnemonic marker.
    pub fn read() -> Self {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let kernel = escape_mnemonic(read("/proc/sys/kernel/osrelease").trim());
        let cpu = cpu_model(&read("/proc/cpuinfo"))
            .map(|c| escape_mnemonic(&c))
            .unwrap_or_default();
        let video = video_driver()
            .map(|d| tr!("about-video", driver = escape_mnemonic(&d)))
            .unwrap_or_default();
        let free = reprisefs::places::statvfs_totals(std::path::Path::new("/"))
            .map(|(_, b)| tr!("about-free-space", size = format_gb(b)))
            .unwrap_or_default();
        let memory = mem_total_kb(&read("/proc/meminfo"))
            .map(|kb| format_kb(kb * 1024))
            .unwrap_or_default();
        Machine {
            version: version_line(env!("CARGO_PKG_VERSION"), BUILD, &kernel),
            facts: [cpu, video, free],
            memory,
        }
    }

    pub fn fields(&self, product: &str, icon: Option<Canvas>) -> Fields {
        Fields {
            product: product.to_string(),
            version: self.version.clone(),
            copyright: String::new(),
            facts: self.facts.clone(),
            memory_label: tr!("about-memory-label"),
            memory_value: self.memory.clone(),
            icon,
        }
    }
}

/// What About Reprise's system view says about the machine, read once ([`SystemFacts::read`]) except the displays, which come from the compositor. An unreadable fact is `None` or empty and its line is left out. Strings are as read; [`SystemFacts::groups`] escapes them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemFacts {
    pub version: String,
    pub build: Option<String>,
    pub interface: String,
    pub renderer: Option<String>,
    pub os: String,
    pub logo: Option<String>,
    pub kernel: String,
    pub arch: String,
    pub hostname: String,
    pub pretty: String,
    pub model: Option<String>,
    pub cpu: Option<String>,
    pub cores: Option<usize>,
    pub threads: usize,
    pub mhz: Option<f64>,
    pub gpus: Vec<String>,
    pub ram: Option<u64>,
    pub ram_free: Option<u64>,
    pub disk: Option<(u64, u64)>,
    pub user: String,
    pub login: String,
    pub uptime: Option<u64>,
    pub displays: Vec<String>,
}

impl SystemFacts {
    /// Read os-release, `/proc`, `/sys`, the space on `/` and the renderer from the environment. The caller fills in the interface and displays: the program hears them later.
    pub fn read() -> SystemFacts {
        use reprisefs::sysinfo as si;
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let facts = si::read();
        let os_text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        let os = si::pretty_name(&os_text).unwrap_or_else(|| {
            let v = facts.os.version.as_deref().unwrap_or("");
            format!("{} {v}", facts.os.name).trim().to_string()
        });
        let cpuinfo = read("/proc/cpuinfo");
        let (cores, threads) = si::cpu_topology(&cpuinfo);
        let meminfo = read("/proc/meminfo");
        let login = std::env::var("USER").unwrap_or_default();
        SystemFacts {
            version: env!("CARGO_PKG_VERSION").into(),
            build: BUILD.map(str::to_string),
            interface: String::new(),
            renderer: std::env::var(si::RENDERER_ENV)
                .ok()
                .and_then(|k| renderer_name(&k)),
            os,
            logo: facts.os.logo.clone(),
            kernel: facts.kernel,
            arch: facts.arch,
            hostname: facts.hostname,
            pretty: facts.pretty,
            model: si::read_hardware_model(),
            cpu: facts.cpu,
            cores,
            threads,
            mhz: facts.mhz,
            gpus: si::gpus()
                .into_iter()
                .filter_map(|g| g.name.or(g.driver))
                .collect(),
            ram: facts.ram,
            ram_free: si::mem_available_kb(&meminfo).map(|kb| kb * 1024),
            disk: reprisefs::places::statvfs_totals(std::path::Path::new("/")),
            user: facts.user,
            login,
            uptime: si::uptime_secs(&read("/proc/uptime")),
            displays: Vec::new(),
        }
    }

    /// The table as label and lines per group, unescaped: what Copy puts on the clipboard and, escaped, what the view shows.
    pub fn table(&self) -> Vec<(String, Vec<String>)> {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let reprise = [
            Some(match &self.build {
                Some(b) => tr!(
                    "system-version-build",
                    version = self.version.as_str(),
                    build = b.as_str()
                ),
                None => tr!("system-version", version = self.version.as_str()),
            }),
            some(&self.interface).map(|i| tr!("system-interface", interface = i)),
            (self.renderer.as_ref()).map(|r| tr!("system-renderer", renderer = r.as_str())),
        ];
        let kernel = match (self.kernel.is_empty(), self.arch.is_empty()) {
            (false, false) => Some(tr!(
                "system-kernel-arch",
                kernel = self.kernel.as_str(),
                arch = self.arch.as_str()
            )),
            (false, true) => Some(tr!("system-kernel", kernel = self.kernel.as_str())),
            (true, false) => Some(self.arch.clone()),
            (true, true) => None,
        };
        let computer = [
            some(&self.hostname),
            some(&self.pretty).filter(|p| *p != self.hostname),
            self.model.clone(),
        ];
        let processor = [self.cpu.as_deref().map(cpu_name), self.cpu_line()];
        let memory = [
            self.ram.map(format_kb),
            self.ram
                .and(self.ram_free)
                .map(|f| tr!("system-memory-available", memory = format_kb(f))),
        ];
        let disk = self.disk.map(|(t, f)| {
            tr!(
                "system-disk-free",
                total = format_gb(t),
                free = format_gb(f)
            )
        });
        let user = match (self.user.as_str(), self.login.as_str()) {
            ("", l) => some(l),
            (u, l) if l.is_empty() || u == l => Some(u.to_string()),
            (u, l) => Some(tr!("system-user-login", user = u, login = l)),
        };
        let session = [
            user,
            (self.uptime).map(|s| tr!("system-uptime", duration = uptime_words(s))),
        ];
        let groups = [
            (
                tr!("system-reprise"),
                reprise.into_iter().flatten().collect(),
            ),
            (
                tr!("system-os"),
                [some(&self.os), kernel].into_iter().flatten().collect(),
            ),
            (
                tr!("system-computer"),
                computer.into_iter().flatten().collect(),
            ),
            (
                tr!("system-processor"),
                processor.into_iter().flatten().collect(),
            ),
            (tr!("system-graphics"), self.gpus.clone()),
            (tr!("system-memory"), memory.into_iter().flatten().collect()),
            (tr!("system-disk"), disk.into_iter().collect()),
            (
                tr!("system-displays", count = self.displays.len()),
                self.displays.clone(),
            ),
            (
                tr!("system-session"),
                session.into_iter().flatten().collect::<Vec<_>>(),
            ),
        ];
        groups
            .into_iter()
            .filter(|(_, lines): &(_, Vec<String>)| !lines.is_empty())
            .collect()
    }

    fn cpu_line(&self) -> Option<String> {
        let cores = |count: usize| tr!("cpu-cores", count = count);
        let threads = |count: usize| tr!("cpu-threads", count = count);
        let mut parts = Vec::new();
        match self.cores {
            Some(c) if c != self.threads && self.threads > 0 => {
                parts.push(cores(c));
                parts.push(threads(self.threads));
            }
            Some(c) => parts.push(cores(c)),
            None if self.threads > 0 => parts.push(threads(self.threads)),
            None => {}
        }
        if let Some(m) = self.mhz {
            parts.push(tr!("cpu-speed", ghz = m / 1000.0));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    pub fn groups(&self) -> Vec<Group> {
        self.table()
            .into_iter()
            .map(|(label, lines)| Group {
                label,
                lines: lines.iter().map(|l| escape_mnemonic(l)).collect(),
            })
            .collect()
    }

    pub fn copy_text(&self) -> String {
        let table = self.table();
        let w = table
            .iter()
            .map(|(l, _)| l.chars().count())
            .max()
            .unwrap_or(0)
            + 1;
        let mut out = String::new();
        for (label, lines) in &table {
            for (i, line) in lines.iter().enumerate() {
                let head = if i == 0 { label.as_str() } else { "" };
                out += &format!("{head:w$}{line}\n");
            }
        }
        out
    }
}

/// The name of the interface `theme` draws in. The theme follows a switch made while the program runs, which the startup environment does not.
pub fn interface_name(theme: &reprisekit::theme::Theme) -> &'static str {
    match theme.look.style() {
        None => "Classic",
        Some(s) if s.glass.is_some() => "Glass",
        Some(_) => "Blue",
    }
}

pub fn renderer_name(kind: &str) -> Option<String> {
    match kind {
        "" => None,
        "vulkan" => Some("Vulkan".into()),
        "gles2" => Some("OpenGL ES".into()),
        "software" => Some("Software".into()),
        other => Some(other.into()),
    }
}

pub fn display_lines(heads: &[Head]) -> Vec<String> {
    let running: Vec<_> = heads
        .iter()
        .filter(|h| h.enabled)
        .filter_map(|h| Some((h, h.current_mode()?)))
        .collect();
    let named = running.len() > 1;
    running
        .into_iter()
        .map(|(h, m)| {
            let mut s = tr!(
                "display-mode",
                width = m.width.to_string(),
                height = m.height.to_string()
            );
            if m.refresh_mhz > 0 {
                let hz = (m.refresh_mhz as f64 / 1000.0).round();
                s = tr!("display-refresh", mode = s, hz = hz);
            }
            if (h.scale - 1.0).abs() > 0.001 && h.scale > 0.0 {
                let percent = (h.scale * 100.0).round();
                s = tr!("display-scale", mode = s, percent = percent);
            }
            if named && !h.name.is_empty() {
                s = tr!("display-named", mode = s, name = h.name.as_str());
            }
            s
        })
        .collect()
}

pub fn cpu_name(model: &str) -> String {
    let mut s = model.to_string();
    for mark in ["(R)", "(r)", "(TM)", "(tm)"] {
        s = s.replace(mark, "");
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn uptime_words(secs: u64) -> String {
    let words = |unit: u64, count: u64| match unit {
        86_400 => tr!("uptime-days", count = count),
        3_600 => tr!("uptime-hours", count = count),
        _ => tr!("uptime-minutes", count = count),
    };
    let parts: Vec<String> = [86_400, 3_600, 60]
        .into_iter()
        .scan(secs, |left, size| {
            let n = *left / size;
            *left %= size;
            Some((size, n))
        })
        .skip_while(|(_, n)| *n == 0)
        .take(2)
        .filter(|(_, n)| *n > 0)
        .map(|(size, n)| words(size, n))
        .collect();
    if parts.is_empty() {
        tr!("uptime-under-a-minute")
    } else {
        parts.join(", ")
    }
}

pub fn drm_driver(uevent: &str) -> Option<String> {
    uevent
        .lines()
        .find_map(|l| l.strip_prefix("DRIVER="))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Memory in whole kilobytes with the UI language's digit grouping: "16,384,000 KB".
pub fn format_kb(bytes: u64) -> String {
    tr!("size-kb", kb = bytes / 1024)
}

pub fn format_gb(bytes: u64) -> String {
    tr!("size-gb", gb = bytes as f64 / 1_073_741_824.0)
}

/// The version line. The parenthetical is a build identifier and, after a colon, a second element; with no build id the whole parenthetical goes.
pub fn version_line(pkg: &str, build: Option<&str>, kernel: &str) -> String {
    match build {
        Some(b) => tr!(
            "about-version-build",
            version = pkg,
            build = b,
            kernel = kernel
        ),
        None => tr!("about-version", version = pkg),
    }
}

fn first_driver_in(uevents: &[&str]) -> Option<String> {
    uevents.iter().find_map(|uevent| drm_driver(uevent))
}

/// The driver bound to the parent device of the first card node that names one: on a PCI card the bus driver (`virtio-pci`, not `virtio_gpu`). Connector directories (`card1-DP-1`) are skipped, and the card is not always `card0`.
pub fn video_driver() -> Option<String> {
    let mut entries: Vec<_> = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card") && !n.contains('-'))
        })
        .collect();
    entries.sort();
    let uevents: Vec<String> = entries
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(p.join("device/uevent")).ok())
        .collect();
    let refs: Vec<&str> = uevents.iter().map(|s| &s[..]).collect();
    first_driver_in(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card's uevent carries DRIVER=; a connector's does not, so the caller skips files that return None.
    #[test]
    fn reads_the_drm_driver() {
        let card = "DRIVER=amdgpu\nPCI_ID=1002:0001\nPCI_SLOT_NAME=0000:03:00.0\n";
        assert_eq!(drm_driver(card).as_deref(), Some("amdgpu"));
        let connector = "DEVTYPE=drm_connector\n";
        assert_eq!(drm_driver(connector), None);
    }

    #[test]
    fn formats_free_space() {
        assert_eq!(format_gb(41_000_000_000), "38.2 GB");
        assert_eq!(format_gb(0), "0.0 GB");
        assert_eq!(format_kb(16 * 1_073_741_824), "16,777,216 KB");
        assert_eq!(format_kb(1023), "0 KB");
    }

    #[test]
    fn version_line_drops_the_build_when_there_is_no_hash() {
        assert_eq!(
            version_line("0.1.0", Some("a1b2c3d"), "6.9.0-generic"),
            "Version 0.1.0 (Build a1b2c3d : 6.9.0-generic)"
        );
        assert_eq!(
            version_line("0.1.0", None, "6.9.0-generic"),
            "Version 0.1.0"
        );
    }

    fn facts() -> SystemFacts {
        SystemFacts {
            version: "0.1.0".into(),
            build: Some("a1b2c3d".into()),
            interface: "Classic".into(),
            renderer: renderer_name("vulkan"),
            os: "Arch Linux".into(),
            logo: Some("archlinux-logo".into()),
            kernel: "6.9.0-1-cachyos".into(),
            arch: "x86_64".into(),
            hostname: "reprise-vm-1".into(),
            pretty: "Kitchen & Den".into(),
            model: Some("QEMU Standard PC (Q35 + ICH9, 2009)".into()),
            cpu: Some("Intel(R) Core(TM)  i5-8250U CPU @ 1.60GHz".into()),
            cores: Some(4),
            threads: 8,
            mhz: Some(3400.0),
            gpus: vec!["Red Hat Virtio 1.0 GPU".into()],
            ram: Some(16 * 1_073_741_824),
            ram_free: Some(9 * 1_073_741_824 + 107_374_182),
            disk: Some((500 * 1_073_741_824, 120 * 1_073_741_824)),
            user: "Jo Doe".into(),
            login: "jo".into(),
            uptime: Some(3 * 3600 + 12 * 60 + 5),
            displays: vec!["1920x1080 at 60 Hz".into()],
        }
    }

    #[test]
    fn the_table_reads_the_facts_in_groups() {
        let t = facts().table();
        let want: Vec<(&str, Vec<&str>)> = vec![
            (
                "Reprise:",
                vec![
                    "Version 0.1.0 (build a1b2c3d)",
                    "Classic interface",
                    "Vulkan renderer",
                ],
            ),
            (
                "Operating system:",
                vec!["Arch Linux", "Linux 6.9.0-1-cachyos (x86_64)"],
            ),
            (
                "Computer:",
                vec![
                    "reprise-vm-1",
                    "Kitchen & Den",
                    "QEMU Standard PC (Q35 + ICH9, 2009)",
                ],
            ),
            (
                "Processor:",
                vec![
                    "Intel Core i5-8250U CPU @ 1.60GHz",
                    "4 cores, 8 threads, up to 3.40 GHz",
                ],
            ),
            ("Graphics:", vec!["Red Hat Virtio 1.0 GPU"]),
            ("Memory:", vec!["16,777,216 KB", "9,542,041 KB available"]),
            ("Disk (/):", vec!["500.0 GB (120.0 GB free)"]),
            ("Display:", vec!["1920x1080 at 60 Hz"]),
            ("Session:", vec!["Jo Doe (jo)", "Up 3 hours, 12 minutes"]),
        ];
        let got: Vec<(&str, Vec<&str>)> = t
            .iter()
            .map(|(l, ls)| (l.as_str(), ls.iter().map(|s| s.as_str()).collect()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn an_unread_fact_leaves_no_line() {
        let bare = SystemFacts {
            version: "0.1.0".into(),
            ..SystemFacts::default()
        };
        assert_eq!(
            bare.table(),
            vec![("Reprise:".to_string(), vec!["Version 0.1.0".to_string()])]
        );
        let f = SystemFacts {
            pretty: "reprise-vm-1".into(),
            cores: Some(4),
            threads: 4,
            mhz: None,
            displays: vec!["a".into(), "b".into()],
            ..facts()
        };
        let t = f.table();
        let group = |l: &str| t.iter().find(|(g, _)| *g == l).map(|(_, v)| v.clone());
        assert_eq!(
            group("Computer:").expect("computer").len(),
            2,
            "a description equal to the host name is not repeated"
        );
        assert_eq!(
            group("Processor:").expect("processor")[1],
            "4 cores",
            "threads equal to cores are not repeated"
        );
        assert!(group("Displays:").is_some(), "two displays are plural");
    }

    #[test]
    fn the_view_escapes_and_the_copy_does_not() {
        let f = facts();
        let computer = &f.groups()[2];
        assert_eq!(computer.label, "Computer:");
        assert_eq!(computer.lines[1], "Kitchen && Den");
        let text = f.copy_text();
        assert!(text.starts_with("Reprise:          Version 0.1.0 (build a1b2c3d)\n"));
        assert!(text.contains("\n                  Kitchen & Den\n"));
        assert!(text.ends_with("                  Up 3 hours, 12 minutes\n"));
    }

    #[test]
    fn the_renderer_is_named_from_its_kind() {
        assert_eq!(renderer_name("gles2").as_deref(), Some("OpenGL ES"));
        assert_eq!(renderer_name("software").as_deref(), Some("Software"));
        assert_eq!(renderer_name(""), None, "an unset kind leaves no line");
    }

    #[test]
    fn the_interface_is_named_from_the_theme() {
        use reprisekit::interface::Interface;
        use reprisekit::interfaces::theme_for;
        assert_eq!(interface_name(&theme_for(Interface::Classic)), "Classic");
        assert_eq!(interface_name(&theme_for(Interface::Blue)), "Blue");
        assert_eq!(interface_name(&theme_for(Interface::Glass)), "Glass");
    }

    #[test]
    fn uptime_is_its_two_largest_units() {
        assert_eq!(uptime_words(30), "less than a minute");
        assert_eq!(uptime_words(60), "1 minute");
        assert_eq!(uptime_words(2 * 3600 + 5 * 60), "2 hours, 5 minutes");
        assert_eq!(uptime_words(86_400 + 59), "1 day");
        assert_eq!(
            uptime_words(3 * 86_400 + 4 * 3600 + 7 * 60),
            "3 days, 4 hours"
        );
    }

    #[test]
    fn a_display_line_per_running_output() {
        use crate::heads::HeadMode;
        let head = |name: &str, w, h, mhz, scale, enabled| Head {
            name: name.into(),
            modes: vec![HeadMode {
                width: w,
                height: h,
                refresh_mhz: mhz,
                preferred: true,
            }],
            current: Some(0),
            enabled,
            scale,
            ..Head::default()
        };
        let one = [
            head("eDP-1", 1920, 1200, 60_001, 1.25, true),
            head("DP-1", 2560, 1440, 165_000, 1.0, false),
        ];
        assert_eq!(display_lines(&one), vec!["1920x1200 at 60 Hz, 125%"]);
        let two = [
            head("eDP-1", 1920, 1200, 60_001, 1.0, true),
            head("DP-1", 2560, 1440, 0, 1.0, true),
        ];
        assert_eq!(
            display_lines(&two),
            vec!["1920x1200 at 60 Hz (eDP-1)", "2560x1440 (DP-1)"]
        );
    }

    #[test]
    fn finds_first_driver_skipping_unbound_cards() {
        let uevents = [
            "DEVTYPE=drm_minor\n",
            "DRIVER=amdgpu\nPCI_ID=1002:0001\nPCI_SLOT_NAME=0000:03:00.0\n",
        ];
        let refs: Vec<&str> = uevents.iter().map(|s| &s[..]).collect();
        assert_eq!(first_driver_in(&refs), Some("amdgpu".to_string()));
    }

    #[test]
    fn returns_none_when_no_uevent_names_a_driver() {
        let uevents = ["DEVTYPE=drm_minor\n"];
        let refs: Vec<&str> = uevents.iter().map(|s| &s[..]).collect();
        assert_eq!(first_driver_in(&refs), None);
    }

    #[test]
    fn returns_none_for_empty_uevent_slice() {
        assert_eq!(first_driver_in(&[]), None);
    }

    #[test]
    fn the_product_name_is_the_callers_and_the_facts_are_the_machines() {
        let machine = Machine::read();
        assert!(machine.version.starts_with("Version "));
        let ex = machine.fields("Explorer", None);
        assert_eq!(ex.product, "Explorer");
        assert_eq!(ex.memory_label, "Physical memory available to Reprise:");
        let own = machine.fields("Reprise", None);
        assert_eq!(own.product, "Reprise");
        for f in [&ex, &own] {
            assert_eq!(f.version, machine.version);
            assert_eq!(f.facts, machine.facts);
            assert_eq!(f.memory_value, machine.memory);
        }
    }
}
