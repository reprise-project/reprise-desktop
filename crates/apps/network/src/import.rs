//! Import...: what a VPN file is and who reads it. A WireGuard `.conf` is
//! read here and opens in the WireGuard sheet for a look before it is
//! saved; anything else goes to NetworkManager's importer through the VPN
//! plugin it belongs to, when that plugin is installed.

use std::path::Path;

use crate::nm::vpn_kind_label;
use crate::wg;

const PLUGIN_DIRS: [&str; 2] = ["/usr/lib/NetworkManager/VPN", "/etc/NetworkManager/VPN"];

/// What to do with a picked file.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    WireGuard(Box<wg::Config>),
    Plugins(Vec<String>),
}

/// The installed plugins' short names (`openvpn`, `vpnc`, ...), from the
/// `name=` line of each `.name` file's `[VPN Connection]` group.
pub fn installed_plugins() -> Vec<String> {
    let mut out = Vec::new();
    for dir in PLUGIN_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            if f.extension().is_some_and(|e| e == "name")
                && let Ok(text) = std::fs::read_to_string(&f)
                && let Some(name) = plugin_name(&text)
                && !out.contains(&name)
            {
                out.push(name);
            }
        }
    }
    out
}

pub fn plugin_name(text: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[VPN Connection]";
        } else if in_group && let Some(v) = line.strip_prefix("name=") {
            let v = v.trim();
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
}

/// The plugin a file's extension names, if it names one: `.ovpn` is
/// OpenVPN, `.pcf` a Cisco profile for vpnc, and a `.conf` that is no
/// WireGuard file is taken for OpenVPN's.
fn wanted(ext: &str) -> Option<&'static str> {
    match ext {
        "ovpn" | "conf" => Some("openvpn"),
        "pcf" => Some("vpnc"),
        _ => None,
    }
}

/// The plan for `path`, whose text is `text` (None when it is not UTF-8),
/// with `installed` plugins. Err is the sentence to show.
pub fn plan(path: &Path, text: Option<&str>, installed: &[String]) -> Result<Plan, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "WireGuard".into());
    if let Some(text) = text
        && text
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case("[Interface]"))
    {
        return wg::parse_conf(text, &stem).map(|c| Plan::WireGuard(Box::new(c)));
    }
    match wanted(&ext) {
        Some(kind) if installed.iter().any(|p| p == kind) => Ok(Plan::Plugins(vec![kind.into()])),
        Some(kind) => Err(format!(
            "This is {} {} file, and NetworkManager's {} plugin is not installed. \
             Install networkmanager-{kind}, then import the file again.",
            if kind == "openvpn" { "an" } else { "a" },
            vpn_kind_label(kind),
            vpn_kind_label(kind),
        )),
        None if installed.is_empty() => Err(
            "No VPN plugin for NetworkManager is installed. Install the plugin for your VPN \
             (networkmanager-openvpn, for one), then import the file again."
                .into(),
        ),
        None => Ok(Plan::Plugins(installed.to_vec())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Vec<String> {
        vec!["openvpn".into(), "vpnc".into()]
    }

    #[test]
    fn a_wireguard_file_opens_in_the_sheet() {
        let text = "[Interface]\nPrivateKey = x\n";
        match plan(Path::new("/tmp/Home Office.conf"), Some(text), &[]) {
            Ok(Plan::WireGuard(c)) => {
                assert_eq!(c.name, "Home Office");
                assert_eq!(c.iface, "HomeOffice");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_extension_picks_the_plugin() {
        assert_eq!(
            plan(Path::new("/tmp/work.ovpn"), Some("client\n"), &installed()),
            Ok(Plan::Plugins(vec!["openvpn".into()]))
        );
        assert_eq!(
            plan(Path::new("/tmp/work.conf"), Some("client\n"), &installed()),
            Ok(Plan::Plugins(vec!["openvpn".into()])),
            "a .conf with no [Interface] is OpenVPN's"
        );
        assert_eq!(
            plan(Path::new("/tmp/cisco.PCF"), None, &installed()),
            Ok(Plan::Plugins(vec!["vpnc".into()]))
        );
        assert_eq!(
            plan(Path::new("/tmp/other.vpn"), None, &installed()),
            Ok(Plan::Plugins(installed())),
            "an unknown file tries every plugin"
        );
    }

    #[test]
    fn a_missing_plugin_says_what_to_install() {
        assert_eq!(
            plan(Path::new("/tmp/work.ovpn"), Some("client\n"), &[]),
            Err(
                "This is an OpenVPN file, and NetworkManager's OpenVPN plugin is not installed. \
                 Install networkmanager-openvpn, then import the file again."
                    .into()
            )
        );
        assert!(
            plan(Path::new("/tmp/x.vpn"), None, &[])
                .unwrap_err()
                .starts_with("No VPN plugin")
        );
    }

    #[test]
    fn a_name_file_gives_its_plugin_name() {
        let text = "[VPN Connection]\nname=openvpn\nservice=org.freedesktop.NetworkManager.openvpn\n\n[libnm]\nplugin=x\n";
        assert_eq!(plugin_name(text).as_deref(), Some("openvpn"));
        assert_eq!(plugin_name("[GNOME]\nname=x\n"), None);
    }
}
