//! Network adapters: the interfaces the taskbar's network meter counts, and what kind each is.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ethernet,
    WiFi,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Ethernet => "Ethernet",
            Kind::WiFi => "Wi-Fi",
            Kind::Other => "Network",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub kind: Kind,
}

/// What kind interface `name` under `root` is: Wi-Fi with a wireless phy, Ethernet with
/// `ARPHRD_ETHER`, else something else (a tunnel, a modem).
pub fn kind(root: &Path, name: &str) -> Kind {
    let at = root.join("sys/class/net").join(name);
    if at.join("wireless").exists() || at.join("phy80211").exists() {
        return Kind::WiFi;
    }
    match super::disk::read_trim(&at.join("type")).as_deref() {
        Some("1") => Kind::Ethernet,
        _ => Kind::Other,
    }
}

/// The adapters under `root` that are up, as [`reprisefs::netspeed::read`] picks them.
pub fn scan(root: &Path) -> Vec<Adapter> {
    reprisefs::netspeed::read(root)
        .into_iter()
        .map(|(name, _)| Adapter {
            kind: kind(root, &name),
            name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adapter_is_wifi_with_a_phy_and_ethernet_by_its_type() {
        let root = tempfile::tempdir().unwrap();
        let net = root.path().join("sys/class/net");
        for (name, ty, wireless) in [
            ("wlan0", "1", true),
            ("enp4s0", "1", false),
            ("wg0", "65534", false),
        ] {
            let d = net.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("type"), format!("{ty}\n")).unwrap();
            if wireless {
                std::fs::create_dir(d.join("wireless")).unwrap();
            }
        }
        assert_eq!(kind(root.path(), "wlan0"), Kind::WiFi);
        assert_eq!(kind(root.path(), "enp4s0"), Kind::Ethernet);
        assert_eq!(kind(root.path(), "wg0"), Kind::Other);
    }
}
