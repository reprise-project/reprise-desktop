//! What the About boxes say about the machine, read from `/etc`, `/proc` and
//! `/sys`; parsing is split from the reads so fixture text can test it.

pub const RENDERER_ENV: &str = "REPRISE_RENDERER";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OsRelease {
    pub name: String,
    pub version: Option<String>,
    pub logo: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub os: OsRelease,
    pub kernel: String,
    pub arch: String,
    pub user: String,
    pub cpu: Option<String>,
    pub mhz: Option<f64>,
    pub ram: Option<u64>,
    pub hostname: String,
    pub pretty: String,
}

pub fn read() -> Facts {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let os = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let cpuinfo = read("/proc/cpuinfo");
    let max_khz = read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq");
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let user = full_name(uid)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_default();
    Facts {
        os: os_release(&os),
        kernel: read("/proc/sys/kernel/osrelease").trim().to_string(),
        arch: std::env::consts::ARCH.to_string(),
        user,
        cpu: cpu_model(&cpuinfo),
        mhz: cpu_mhz(&max_khz, &cpuinfo),
        ram: mem_total_kb(&read("/proc/meminfo")).map(|kb| kb * 1024),
        hostname: read("/proc/sys/kernel/hostname").trim().to_string(),
        pretty: pretty_hostname(&read("/etc/machine-info")).unwrap_or_default(),
    }
}

fn env_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.trim().split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| {
            let v = v.trim();
            let quoted = v.len() >= 2
                && (v.starts_with('"') && v.ends_with('"')
                    || v.starts_with('\'') && v.ends_with('\''));
            let v = if quoted { &v[1..v.len() - 1] } else { v };
            v.replace("\\\"", "\"").replace("\\\\", "\\")
        })
        .filter(|v| !v.is_empty())
}

pub fn os_release(text: &str) -> OsRelease {
    OsRelease {
        name: env_value(text, "NAME")
            .or_else(|| env_value(text, "PRETTY_NAME"))
            .unwrap_or_else(|| "Linux".into()),
        version: env_value(text, "VERSION")
            .or_else(|| env_value(text, "VERSION_ID"))
            .or_else(|| env_value(text, "BUILD_ID")),
        logo: env_value(text, "LOGO"),
    }
}

pub fn pretty_hostname(machine_info: &str) -> Option<String> {
    env_value(machine_info, "PRETTY_HOSTNAME")
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn cpu_model(cpuinfo: &str) -> Option<String> {
    field(cpuinfo, "model name")
}

pub fn mem_total_kb(meminfo: &str) -> Option<u64> {
    field(meminfo, "MemTotal")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The CPU's top clock in MHz: cpufreq's `cpuinfo_max_freq`, else the first
/// `cpu MHz` in `/proc/cpuinfo` (all a VM has).
pub fn cpu_mhz(max_freq_khz: &str, cpuinfo: &str) -> Option<f64> {
    let khz = max_freq_khz.trim().parse::<f64>().ok().map(|k| k / 1000.0);
    khz.or_else(|| field(cpuinfo, "cpu MHz")?.parse().ok())
        .filter(|m| *m > 0.0)
}

pub fn pretty_name(os_release: &str) -> Option<String> {
    env_value(os_release, "PRETTY_NAME")
}

/// Cores and threads from `/proc/cpuinfo`; cores are `None` where it names no core ids (most ARM).
pub fn cpu_topology(cpuinfo: &str) -> (Option<usize>, usize) {
    let mut threads = 0;
    let mut cores = std::collections::BTreeSet::new();
    let mut package = String::new();
    for line in cpuinfo.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "processor" => threads += 1,
            "physical id" => package = v.trim().to_string(),
            "core id" => {
                cores.insert((package.clone(), v.trim().to_string()));
            }
            _ => {}
        }
    }
    ((!cores.is_empty()).then_some(cores.len()), threads)
}

pub fn mem_available_kb(meminfo: &str) -> Option<u64> {
    field(meminfo, "MemAvailable")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn uptime_secs(proc_uptime: &str) -> Option<u64> {
    let secs: f64 = proc_uptime.split_whitespace().next()?.parse().ok()?;
    (secs >= 0.0).then_some(secs as u64)
}

fn dmi_placeholder(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.is_empty()
        || [
            "to be filled by o.e.m.",
            "system product name",
            "system manufacturer",
            "system version",
            "default string",
            "not specified",
            "not applicable",
            "none",
            "o.e.m.",
            "oem",
            "unknown",
            "type1productconfigid",
            "0123456789",
        ]
        .contains(&l.as_str())
}

/// The computer's make and model from DMI. Lenovo keeps a part number in
/// `product_name` and the model in `product_version`. Firmware placeholders count as unread.
pub fn hardware_model(vendor: &str, name: &str, version: &str) -> Option<String> {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !dmi_placeholder(s));
    let vendor = clean(vendor);
    let lenovo = vendor
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("lenovo"));
    let model = match (lenovo, clean(version)) {
        (true, Some(v)) => Some(v),
        _ => clean(name),
    };
    let vendor = vendor.map(|v| vendor_short(&v)).filter(|v| !v.is_empty());
    match (vendor, model) {
        (Some(v), Some(m)) => {
            let first = v.split_whitespace().next().unwrap_or(&v).to_lowercase();
            if m.to_lowercase().starts_with(&first) {
                Some(m)
            } else {
                Some(format!("{v} {m}"))
            }
        }
        (v, m) => v.or(m),
    }
}

pub fn pci_id(uevent: &str) -> Option<(u16, u16)> {
    let id = uevent.lines().find_map(|l| l.strip_prefix("PCI_ID="))?;
    let (v, d) = id.trim().split_once(':')?;
    Some((
        u16::from_str_radix(v, 16).ok()?,
        u16::from_str_radix(d, 16).ok()?,
    ))
}

/// The marketing name of PCI device `vendor:device` in hwdata's `pci.ids`: the
/// bracketed short name where the entry has one, after the vendor's short name.
pub fn pci_name(pci_ids: &str, vendor: u16, device: u16) -> Option<String> {
    let hex = |s: &str| s.get(..4).and_then(|h| u16::from_str_radix(h, 16).ok());
    let mut vendor_name = None;
    for line in pci_ids.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(dev) = line.strip_prefix('\t') {
            if vendor_name.is_some() && !dev.starts_with('\t') && hex(dev) == Some(device) {
                let name = short_name(dev[4..].trim());
                let v = vendor_short(vendor_name.unwrap_or_default());
                return Some(if v.is_empty() || name.starts_with(&v) {
                    name
                } else {
                    format!("{v} {name}")
                });
            }
            continue;
        }
        if vendor_name.is_some() {
            return None;
        }
        if hex(line) == Some(vendor) {
            vendor_name = Some(line[4..].trim());
        }
    }
    None
}

fn short_name(name: &str) -> String {
    match (name.find('['), name.rfind(']')) {
        (Some(a), Some(b)) if a < b && !name[a + 1..b].trim().is_empty() => {
            name[a + 1..b].trim().to_string()
        }
        _ => name.to_string(),
    }
}

fn vendor_short(name: &str) -> String {
    if let (Some(a), Some(b)) = (name.find('['), name.rfind(']'))
        && a < b
    {
        return name[a + 1..b]
            .split('/')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let name = name.split(',').next().unwrap_or(name).trim();
    [" Corporation", " Corp.", " Inc.", " INC.", " Ltd.", " LTD."]
        .iter()
        .find_map(|s| name.strip_suffix(s))
        .unwrap_or(name)
        .trim()
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpu {
    pub driver: Option<String>,
    pub name: Option<String>,
}

pub fn gpus() -> Vec<Gpu> {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards: Vec<_> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card") && !n.contains('-'))
        })
        .collect();
    cards.sort();
    let uevents: Vec<String> = cards
        .iter()
        .filter_map(|p| std::fs::read_to_string(p.join("device/uevent")).ok())
        .collect();
    let mut pci_ids = None;
    uevents
        .iter()
        .map(|u| {
            let name = pci_id(u).and_then(|(v, d)| {
                let ids = pci_ids.get_or_insert_with(|| {
                    std::fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default()
                });
                pci_name(ids, v, d)
            });
            let driver = uevent_driver(u);
            Gpu { driver, name }
        })
        .filter(|g| g.driver.is_some() || g.name.is_some())
        .collect()
}

fn uevent_driver(uevent: &str) -> Option<String> {
    uevent
        .lines()
        .find_map(|l| l.strip_prefix("DRIVER="))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn read_hardware_model() -> Option<String> {
    let read =
        |f: &str| std::fs::read_to_string(format!("/sys/class/dmi/id/{f}")).unwrap_or_default();
    hardware_model(
        &read("sys_vendor"),
        &read("product_name"),
        &read("product_version"),
    )
}

/// The full name of `uid` through NSS, so homed, sssd or LDAP users missing from
/// /etc/passwd resolve too. None for an unknown uid.
pub fn full_name(uid: u32) -> Option<String> {
    use std::ffi::CStr;
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `libc::passwd` is a plain C struct; all zeroes is a
        // valid value.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: getpwuid_r writes only through the three pointers, each
        // to a live local; `buf` outlives the reads of `pwd` below, whose
        // strings point into it.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 64 * 1024 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return None;
        }
        let text = |p: *const libc::c_char| {
            if p.is_null() {
                return String::new();
            }
            // SAFETY: the call succeeded, so a non-null field is a
            // NUL-terminated string in `buf`, still in scope.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        };
        return display_name(&text(pwd.pw_name), &text(pwd.pw_gecos));
    }
}

pub fn display_name(login: &str, gecos: &str) -> Option<String> {
    let gecos = gecos.split(',').next().unwrap_or("").trim();
    Some(if gecos.is_empty() { login } else { gecos }.to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_os_release() {
        let arch = "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n\
                    BUILD_ID=rolling\nLOGO=archlinux-logo\n";
        assert_eq!(
            os_release(arch),
            OsRelease {
                name: "Arch Linux".into(),
                version: Some("rolling".into()),
                logo: Some("archlinux-logo".into()),
            }
        );
        let debian = "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian GNU/Linux\"\n\
                      VERSION_ID=\"12\"\nVERSION=\"12 (bookworm)\"\n";
        let d = os_release(debian);
        assert_eq!(d.name, "Debian GNU/Linux");
        assert_eq!(d.version.as_deref(), Some("12 (bookworm)"));
        assert_eq!(d.logo, None);
        assert_eq!(os_release("").name, "Linux");
    }

    #[test]
    fn reads_the_cpu_model() {
        let s = "processor\t: 0\nmodel name\t: Example CPU @ 2.00GHz\nstepping\t: 1\n";
        assert_eq!(cpu_model(s).as_deref(), Some("Example CPU @ 2.00GHz"));
        assert_eq!(cpu_model("processor\t: 0\n"), None);
    }

    #[test]
    fn reads_total_memory() {
        let s = "MemTotal:       16777216 kB\nMemFree:         1234 kB\n";
        assert_eq!(mem_total_kb(s), Some(16_777_216));
        assert_eq!(mem_total_kb("MemFree: 12 kB\n"), None);
    }

    #[test]
    fn the_top_clock_comes_from_cpufreq_then_cpuinfo() {
        let cpuinfo = "processor\t: 0\ncpu MHz\t\t: 2495.998\n";
        assert_eq!(cpu_mhz("4200000\n", cpuinfo), Some(4200.0));
        assert_eq!(cpu_mhz("", cpuinfo), Some(2495.998));
        assert_eq!(cpu_mhz("", "processor\t: 0\n"), None);
    }

    #[test]
    fn the_full_name_is_the_gecos_up_to_its_first_comma() {
        assert_eq!(
            display_name("jo", "Jo Doe,Room 1,,").as_deref(),
            Some("Jo Doe")
        );
        assert_eq!(display_name("sam", " ").as_deref(), Some("sam"));
        assert_eq!(display_name("", ""), None);
    }

    #[test]
    fn the_full_name_is_looked_up_through_nss() {
        // SAFETY: getuid cannot fail and touches no memory.
        let uid = unsafe { libc::getuid() };
        assert!(full_name(uid).is_some_and(|n| !n.is_empty()));
        assert!(full_name(0).is_some());
        assert_eq!(full_name(0x7fff_fff0), None);
    }

    #[test]
    fn the_pretty_name_is_os_releases_one_line() {
        let debian = "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian GNU/Linux\"\n";
        assert_eq!(
            pretty_name(debian).as_deref(),
            Some("Debian GNU/Linux 12 (bookworm)")
        );
        assert_eq!(pretty_name("NAME=Arch\n"), None);
    }

    #[test]
    fn counts_cores_and_threads() {
        let mut s = String::new();
        for (cpu, pkg, core) in [
            (0, 0, 0),
            (1, 0, 1),
            (2, 1, 0),
            (3, 1, 1),
            (4, 0, 0),
            (5, 0, 1),
            (6, 1, 0),
            (7, 1, 1),
        ] {
            s += &format!("processor\t: {cpu}\nphysical id\t: {pkg}\ncore id\t\t: {core}\n\n");
        }
        assert_eq!(cpu_topology(&s), (Some(4), 8));
        let arm = "processor\t: 0\nBogoMIPS\t: 48.00\n\nprocessor\t: 1\nBogoMIPS\t: 48.00\n";
        assert_eq!(cpu_topology(arm), (None, 2));
    }

    #[test]
    fn reads_available_memory_and_uptime() {
        let s = "MemTotal:       16777216 kB\nMemAvailable:    8388608 kB\n";
        assert_eq!(mem_available_kb(s), Some(8_388_608));
        assert_eq!(mem_available_kb("MemTotal: 1 kB\n"), None);
        assert_eq!(uptime_secs("11523.62 89021.10\n"), Some(11523));
        assert_eq!(uptime_secs(""), None);
    }

    #[test]
    fn the_hardware_model_is_vendor_and_product() {
        assert_eq!(
            hardware_model("QEMU\n", "Standard PC (Q35 + ICH9, 2009)\n", "pc-q35-9.0\n").as_deref(),
            Some("QEMU Standard PC (Q35 + ICH9, 2009)")
        );
        assert_eq!(
            hardware_model("LENOVO\n", "21A0005EGE\n", "ThinkPad P14s Gen 2a\n").as_deref(),
            Some("LENOVO ThinkPad P14s Gen 2a")
        );
        assert_eq!(
            hardware_model("Dell Inc.", "Dell Latitude 7420", "").as_deref(),
            Some("Dell Latitude 7420")
        );
        assert_eq!(
            hardware_model(
                "System manufacturer",
                "System Product Name",
                "System Version"
            ),
            None
        );
        assert_eq!(
            hardware_model("ASUS", "To be filled by O.E.M.", "").as_deref(),
            Some("ASUS")
        );
        assert_eq!(hardware_model("", "", ""), None);
    }

    #[test]
    fn reads_the_pci_id() {
        let u = "DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:1638\nPCI_SLOT_NAME=0000:07:00.0\n";
        assert_eq!(pci_id(u), Some((0x1002, 0x1638)));
        assert_eq!(pci_id("DRIVER=simple-framebuffer\n"), None);
    }

    #[test]
    fn names_a_pci_device_from_pci_ids() {
        let ids = "# comment\n\
                   1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
                   \t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\n\
                   \t\t17aa 509b  Radeon Vega 7\n\
                   \t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX]\n\
                   10de  NVIDIA Corporation\n\
                   \t2684  AD102 [GeForce RTX 4090]\n\
                   1af4  Red Hat, Inc.\n\
                   \t1050  Virtio 1.0 GPU\n\
                   8086  Intel Corporation\n\
                   \t1050  Some Intel device\n\
                   \t46a6  Alder Lake-P GT2 [Iris Xe Graphics]\n";
        assert_eq!(
            pci_name(ids, 0x1002, 0x1638).as_deref(),
            Some("AMD Radeon Vega Series / Radeon Vega Mobile Series")
        );
        assert_eq!(
            pci_name(ids, 0x10de, 0x2684).as_deref(),
            Some("NVIDIA GeForce RTX 4090")
        );
        assert_eq!(
            pci_name(ids, 0x1af4, 0x1050).as_deref(),
            Some("Red Hat Virtio 1.0 GPU")
        );
        assert_eq!(
            pci_name(ids, 0x8086, 0x46a6).as_deref(),
            Some("Intel Iris Xe Graphics")
        );
        assert_eq!(pci_name(ids, 0x1002, 0x9999), None, "not under AMD");
        assert_eq!(
            pci_name(ids, 0x17aa, 0x509b),
            None,
            "a subsystem is no device"
        );
        assert_eq!(pci_name("", 0x1002, 0x1638), None);
    }

    #[test]
    fn machine_info_holds_the_description() {
        let s = "PRETTY_HOSTNAME=\"Kitchen Computer\"\nICON_NAME=computer\n";
        assert_eq!(pretty_hostname(s).as_deref(), Some("Kitchen Computer"));
        assert_eq!(pretty_hostname(""), None);
    }
}
