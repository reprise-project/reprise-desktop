//! The devices and drivers cupsd offers the add wizard.

use super::*;

/// A device cupsd's backends found: a USB printer, a network printer
/// announced over DNS-SD.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub uri: String,
    pub info: String,
    pub make_model: String,
    pub class: String,
    pub device_id: String,
}

impl Device {
    /// A device from the `device-*` attributes `get` reads, by cupsd's names for them.
    fn from_attrs(get: impl Fn(&str) -> String) -> Device {
        Device {
            uri: get("device-uri"),
            info: get("device-info"),
            make_model: get("device-make-and-model"),
            class: get("device-class"),
            device_id: get("device-id"),
        }
    }

    /// The maker the device ID names, else the first word of the make
    /// and model: what the driver search starts on.
    pub fn maker(&self) -> String {
        let from_id = self.device_id.split(';').find_map(|f| {
            let (k, v) = f.split_once(':')?;
            matches!(k.trim(), "MFG" | "MANUFACTURER").then(|| v.trim().to_string())
        });
        from_id
            .filter(|m| !m.is_empty())
            .or_else(|| {
                self.make_model
                    .split_whitespace()
                    .next()
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }

    /// Whether the printer speaks IPP itself, so cupsd can set it up with
    /// no driver.
    pub fn driverless(&self) -> bool {
        driverless_uri(&self.uri)
    }

    pub fn label(&self) -> String {
        let name = if self.info.is_empty() {
            &self.make_model
        } else {
            &self.info
        };
        // The driverless backend names its finds "... (driverless)"
        // already.
        let name = name.trim_end_matches(" (driverless)");
        let how = if self.driverless() {
            "network, no driver needed"
        } else if self.class == "network" {
            "network"
        } else {
            "connected to this computer"
        };
        format!("{name} ({how})")
    }
}

/// An address cupsd can reach the printer at over IPP: ipp, ipps, or a
/// DNS-SD name of an IPP service.
pub fn driverless_uri(uri: &str) -> bool {
    uri.starts_with("ipp://")
        || uri.starts_with("ipps://")
        || uri.starts_with("dnssd://") && (uri.contains("._ipp.") || uri.contains("._ipps."))
}

const EXCLUDED_SCHEMES: [&str; 3] = ["cups-brf", "beh", "fax"];

/// The devices cupsd's backends find within `secs` seconds. The ones a
/// backend lists as a protocol without a printer behind it ("socket",
/// "ipp", "lpd") are left out; the wizard asks for an address instead.
/// CUPS-Get-Devices is an admin operation: when cupsd says no, cups-pk-helper
/// and its password prompt are asked only with `prompt`, else it is [`Error::Denied`].
pub fn devices(c: &Client, secs: i32, prompt: bool) -> Result<Vec<Device>> {
    let req = Request::new(CUPS_GET_DEVICES)
        .op_attr("timeout", Value::Int(secs))
        .add(
            OPERATION,
            "exclude-schemes",
            EXCLUDED_SCHEMES.iter().map(|s| Value::name(s)).collect(),
        );
    let peer = c.call("/admin/", req).map(|r| {
        r.groups(PRINTER)
            .map(|g| Device::from_attrs(|k| g.string(k)))
            .collect()
    });
    devices_after(c, peer, secs, prompt)
}

pub(super) fn devices_after(
    c: &Client,
    peer: Result<Vec<Device>>,
    secs: i32,
    prompt: bool,
) -> Result<Vec<Device>> {
    let found = match peer {
        Ok(found) => found,
        Err(Error::Denied) if !prompt => return Err(Error::Denied),
        Err(Error::Denied) if c.helper.installed() => {
            let found = c.helper.devices(secs, &EXCLUDED_SCHEMES)?;
            pkhelper::devices_from(&found)
                .iter()
                .map(|d| Device::from_attrs(|k| d.get(k).cloned().unwrap_or_default()))
                .collect()
        }
        Err(Error::Denied) => return Err(Error::NeedsHelper),
        Err(e) => return Err(e),
    };
    Ok(tidy(found))
}

fn tidy(found: Vec<Device>) -> Vec<Device> {
    let mut out: Vec<Device> = found
        .into_iter()
        .filter(|d| d.uri.contains(':') && d.uri.contains('/'))
        .collect();
    // A printer both announced over DNS-SD and found another way shows
    // once, as its driverless address when it has one.
    out.sort_by_key(|d| (!d.driverless(), d.info.to_lowercase()));
    out.dedup_by(|b, a| !a.info.is_empty() && a.info == b.info);
    out
}

/// Whether two devices are one printer. cupsd's backends name a printer by its service, its
/// make and model or both.
pub fn same(a: &Device, b: &Device) -> bool {
    let info = |d: &Device| d.info.trim_end_matches(" (driverless)").to_lowercase();
    a.uri == b.uri
        || !a.info.is_empty() && info(a) == info(b)
        || avahi::service_name(&a.uri).is_some_and(|n| Some(n) == avahi::service_name(&b.uri))
}

/// What cupsd found and what avahi-daemon sees, a printer both list shown once as cupsd's.
pub fn merge(cups: &[Device], network: &[Device]) -> Vec<Device> {
    let mut all = cups.to_vec();
    all.extend(
        network
            .iter()
            .filter(|d| !cups.iter().any(|c| same(c, d)))
            .cloned(),
    );
    tidy(all)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ppd {
    pub name: String,
    pub make: String,
    pub make_model: String,
}

pub fn ppds(c: &Client) -> Result<Vec<Ppd>> {
    let req =
        Request::new(CUPS_GET_PPDS).requested(&["ppd-name", "ppd-make", "ppd-make-and-model"]);
    let r = match c.call("/", req) {
        Ok(r) => r,
        Err(Error::Refused(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out: Vec<Ppd> = r
        .groups(PRINTER)
        .map(|g| Ppd {
            name: g.string("ppd-name"),
            make: g.string("ppd-make"),
            make_model: g.string("ppd-make-and-model"),
        })
        .filter(|p| !p.name.is_empty())
        .collect();
    out.sort_by_key(|p| p.make_model.to_lowercase());
    out.dedup_by(|b, a| a.make_model == b.make_model);
    Ok(out)
}
