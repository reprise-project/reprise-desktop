//! The strings every Reprise About box shows and the `/proc` and `/sys` reads behind them; parsing is split from IO so it can be tested on fixture text. The small box goes through [`fields`], reprise-about shows the system view from [`SystemFacts`].

use crate::heads::Head;
use reprisefs::sysinfo::{cpu_model, mem_total_kb};
use reprisekit::controls::aboutbox::{Fields, Group};
use reprisekit::raster::Canvas;

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
            .map(|d| format!("Video:  {}", escape_mnemonic(&d)))
            .unwrap_or_default();
        let free = reprisefs::places::statvfs_totals(std::path::Path::new("/"))
            .map(|(_, b)| format!("Free space on /:  {}", format_gb(b)))
            .unwrap_or_default();
        let memory = mem_total_kb(&read("/proc/meminfo"))
            .map(|kb| format!("{} KB", reprisefs::props::thousands(kb)))
            .unwrap_or_default();
        Machine {
            version: version_line(
                env!("CARGO_PKG_VERSION"),
                option_env!("REPRISE_BUILD"),
                &kernel,
            ),
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
            memory_label: "Physical memory available to Reprise:".into(),
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
            build: option_env!("REPRISE_BUILD").map(str::to_string),
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
    pub fn table(&self) -> Vec<(&'static str, Vec<String>)> {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let reprise = [
            Some(match &self.build {
                Some(b) => format!("Version {} (build {b})", self.version),
                None => format!("Version {}", self.version),
            }),
            some(&self.interface).map(|i| format!("{i} interface")),
            self.renderer.as_ref().map(|r| format!("{r} renderer")),
        ];
        let kernel = match (self.kernel.is_empty(), self.arch.is_empty()) {
            (false, false) => Some(format!("Linux {} ({})", self.kernel, self.arch)),
            (false, true) => Some(format!("Linux {}", self.kernel)),
            (true, false) => Some(self.arch.clone()),
            (true, true) => None,
        };
        let computer = [
            some(&self.hostname),
            some(&self.pretty).filter(|p| *p != self.hostname),
            self.model.clone(),
        ];
        let processor = [self.cpu.as_deref().map(cpu_name), self.cpu_line()];
        let memory = self.ram.map(|t| match self.ram_free {
            Some(f) => format!("{} ({} available)", format_gb(t), format_gb(f)),
            None => format_gb(t),
        });
        let disk = self
            .disk
            .map(|(t, f)| format!("{} ({} free)", format_gb(t), format_gb(f)));
        let user = match (self.user.as_str(), self.login.as_str()) {
            ("", l) => some(l),
            (u, l) if l.is_empty() || u == l => Some(u.to_string()),
            (u, l) => Some(format!("{u} ({l})")),
        };
        let session = [user, self.uptime.map(|s| format!("Up {}", uptime_words(s)))];
        let groups = [
            ("Reprise:", reprise.into_iter().flatten().collect()),
            (
                "Operating system:",
                [some(&self.os), kernel].into_iter().flatten().collect(),
            ),
            ("Computer:", computer.into_iter().flatten().collect()),
            ("Processor:", processor.into_iter().flatten().collect()),
            ("Graphics:", self.gpus.clone()),
            ("Memory:", memory.into_iter().collect()),
            ("Disk (/):", disk.into_iter().collect()),
            (
                if self.displays.len() > 1 {
                    "Displays:"
                } else {
                    "Display:"
                },
                self.displays.clone(),
            ),
            (
                "Session:",
                session.into_iter().flatten().collect::<Vec<_>>(),
            ),
        ];
        groups
            .into_iter()
            .filter(|(_, lines): &(_, Vec<String>)| !lines.is_empty())
            .collect()
    }

    fn cpu_line(&self) -> Option<String> {
        let plural = |n: usize, one: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {one}s")
            }
        };
        let mut parts = Vec::new();
        match self.cores {
            Some(c) if c != self.threads && self.threads > 0 => {
                parts.push(plural(c, "core"));
                parts.push(plural(self.threads, "thread"));
            }
            Some(c) => parts.push(plural(c, "core")),
            None if self.threads > 0 => parts.push(plural(self.threads, "thread")),
            None => {}
        }
        if let Some(m) = self.mhz {
            parts.push(format!("up to {:.2} GHz", m / 1000.0));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    pub fn groups(&self) -> Vec<Group> {
        self.table()
            .into_iter()
            .map(|(label, lines)| Group {
                label: label.into(),
                lines: lines.iter().map(|l| escape_mnemonic(l)).collect(),
            })
            .collect()
    }

    pub fn copy_text(&self) -> String {
        let table = self.table();
        let w = table.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 1;
        let mut out = String::new();
        for (label, lines) in &table {
            for (i, line) in lines.iter().enumerate() {
                let head = if i == 0 { *label } else { "" };
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
            let mut s = format!("{}x{}", m.width, m.height);
            if m.refresh_mhz > 0 {
                s += &format!(" at {} Hz", (m.refresh_mhz as f64 / 1000.0).round());
            }
            if (h.scale - 1.0).abs() > 0.001 && h.scale > 0.0 {
                s += &format!(", {}%", (h.scale * 100.0).round());
            }
            if named && !h.name.is_empty() {
                s += &format!(" ({})", h.name);
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
    let units = [(86_400, "day"), (3_600, "hour"), (60, "minute")];
    let parts: Vec<String> = units
        .iter()
        .scan(secs, |left, &(size, name)| {
            let n = *left / size;
            *left %= size;
            Some((n, name))
        })
        .skip_while(|(n, _)| *n == 0)
        .take(2)
        .filter(|(n, _)| *n > 0)
        .map(|(n, name)| format!("{n} {name}{}", if n == 1 { "" } else { "s" }))
        .collect();
    if parts.is_empty() {
        "less than a minute".into()
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

pub fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

/// Double every `&`, so a label (which reads a bare `&` as a mnemonic marker) draws it literally.
pub fn escape_mnemonic(s: &str) -> String {
    s.replace('&', "&&")
}

/// The version line. The parenthetical is a build identifier and, after a colon, a second element; with no git hash the whole parenthetical goes.
pub fn version_line(pkg: &str, build: Option<&str>, kernel: &str) -> String {
    match build {
        Some(b) => format!("Version {pkg} (Build {b} : {kernel})"),
        None => format!("Version {pkg}"),
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

    #[test]
    fn build_hash_is_the_checkout_head() {
        let head = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        if let Some(head) = head {
            assert_eq!(option_env!("REPRISE_BUILD"), Some(head.as_str()));
        }
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
            ("Memory:", vec!["16.0 GB (9.1 GB available)"]),
            ("Disk (/):", vec!["500.0 GB (120.0 GB free)"]),
            ("Display:", vec!["1920x1080 at 60 Hz"]),
            ("Session:", vec!["Jo Doe (jo)", "Up 3 hours, 12 minutes"]),
        ];
        let got: Vec<(&str, Vec<&str>)> = t
            .iter()
            .map(|(l, ls)| (*l, ls.iter().map(|s| s.as_str()).collect()))
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
            vec![("Reprise:", vec!["Version 0.1.0".to_string()])]
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
    fn escapes_ampersands_for_the_mnemonic_parser() {
        assert_eq!(
            escape_mnemonic("Intel(R) Core(TM) i5 & friends"),
            "Intel(R) Core(TM) i5 && friends"
        );
        assert_eq!(escape_mnemonic("&"), "&&");
        assert_eq!(escape_mnemonic("a&&b"), "a&&&&b");
        assert_eq!(escape_mnemonic(""), "");
        assert_eq!(escape_mnemonic("amdgpu"), "amdgpu");
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
