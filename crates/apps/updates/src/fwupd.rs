//! fwupd on the system bus. Metadata and cabinets go to the daemon as file descriptors;
//! the files are fetched here with `curl`, as the daemon downloads nothing itself.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::{Either, select};
use futures_util::stream::StreamExt;
use reprisebus::Signal;
use zbus::Connection;
use zbus::proxy::{CacheProperties, MethodFlags};
use zbus::zvariant::{Fd, OwnedValue, Value};

use crate::model::{Kind, Origin, Restart, Update};

pub const NAME: &str = "org.freedesktop.fwupd";
const PATH: &str = "/";
const IFACE: &str = "org.freedesktop.fwupd";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `FwupdDeviceFlags` bits.
const UPDATABLE: u64 = 1 << 1;
const NEEDS_REBOOT: u64 = 1 << 8;
const NEEDS_SHUTDOWN: u64 = 1 << 17;

/// `FwupdRemoteKind::Download`: a remote whose metadata is fetched.
const REMOTE_DOWNLOAD: u32 = 1;

/// `FwupdStatus` values worth a line of their own.
const STATUS_DECOMPRESSING: u32 = 3;
const STATUS_DEVICE_RESTART: u32 = 4;
const STATUS_DEVICE_WRITE: u32 = 5;
const STATUS_DEVICE_VERIFY: u32 = 6;
const STATUS_SCHEDULING: u32 = 7;
const STATUS_DEVICE_READ: u32 = 9;
const STATUS_DEVICE_ERASE: u32 = 10;
const STATUS_WAITING_FOR_AUTH: u32 = 11;
const STATUS_DEVICE_BUSY: u32 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Failed(String),
}

pub type Progress<'a> = &'a mut dyn FnMut(Option<f64>, Option<String>);

type Dict = HashMap<String, OwnedValue>;

fn failed(e: zbus::Error) -> Error {
    if let zbus::Error::MethodError(name, detail, _) = &e {
        if matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ) {
            return Error::Unavailable;
        }
        if let Some(d) = detail {
            return Error::Failed(sentence(d));
        }
    }
    Error::Failed(e.to_string())
}

fn sentence(s: &str) -> String {
    let mut s = s.trim().to_string();
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

fn nothing_newer(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::MethodError(name, _, _) if matches!(
        name.as_str(),
        "org.freedesktop.fwupd.NothingToDo"
            | "org.freedesktop.fwupd.NotSupported"
            | "org.freedesktop.fwupd.NotFound"
    ))
}

fn text(d: &Dict, key: &str) -> String {
    d.get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("")
        .to_string()
}

fn number(d: &Dict, key: &str) -> u64 {
    d.get(key)
        .and_then(|v| {
            u64::try_from(v)
                .ok()
                .or_else(|| u32::try_from(v).ok().map(u64::from))
        })
        .unwrap_or(0)
}

fn strings(d: &Dict, key: &str) -> Vec<String> {
    d.get(key)
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default()
}

async fn call(
    conn: &Connection,
    method: &str,
    body: &(impl zbus::export::serde::Serialize + zbus::zvariant::DynamicType),
) -> zbus::Result<zbus::Message> {
    conn.call_method(Some(NAME), PATH, Some(IFACE), method, body)
        .await
}

async fn devices(conn: &Connection) -> Result<Vec<Dict>, Error> {
    call(conn, "GetDevices", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)
}

async fn newest(conn: &Connection, device: &str) -> Result<Option<Dict>, Error> {
    match call(conn, "GetUpgrades", &(device,)).await {
        Ok(m) => {
            let releases: Vec<Dict> = m.body().deserialize().map_err(failed)?;
            Ok(releases.into_iter().next())
        }
        Err(e) if nothing_newer(&e) => Ok(None),
        Err(e) => Err(failed(e)),
    }
}

fn notes(release: &Dict) -> String {
    let markup = text(release, "Description");
    let mut out = String::new();
    let mut in_tag = false;
    for ch in markup.chars() {
        match ch {
            '<' => {
                in_tag = true;
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(if c.is_whitespace() { ' ' } else { c }),
            _ => {}
        }
    }
    let line = out
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    if line.is_empty() {
        text(release, "Summary")
    } else {
        line
    }
}

fn kind(urgency: u64) -> Kind {
    match urgency {
        1 => Kind::Low,
        3 | 4 => Kind::Important,
        _ => Kind::Normal,
    }
}

fn update(device: &Dict, release: &Dict) -> Update {
    Update {
        id: text(device, "DeviceId"),
        name: text(device, "Name"),
        current: text(device, "Version"),
        version: text(release, "Version"),
        summary: notes(release),
        kind: kind(number(release, "Urgency")),
        origin: Origin::Firmware,
    }
}

pub async fn updates(conn: &Connection) -> Result<Vec<Update>, Error> {
    let mut out = Vec::new();
    for d in devices(conn).await? {
        if number(&d, "Flags") & UPDATABLE == 0 {
            continue;
        }
        // One device the daemon cannot answer for leaves the rest listed.
        match newest(conn, &text(&d, "DeviceId")).await {
            Ok(Some(r)) => out.push(update(&d, &r)),
            Ok(None) => {}
            Err(Error::Unavailable) => return Err(Error::Unavailable),
            Err(Error::Failed(e)) => log::info!("firmware for {}: {e}", text(&d, "Name")),
        }
    }
    Ok(out)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether a remote's metadata is due: never fetched (the daemon says
/// `u64::MAX`), or older than its refresh interval.
fn due(modified: u64, interval: u64, now: u64) -> bool {
    modified == u64::MAX || now.saturating_sub(modified) >= interval.max(1)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Scratch(dir.join(format!("reprise-updates-{}-{name}", std::process::id())))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "2",
        ])
        .arg("--output")
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl could not be started: {e}."))?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.trim().trim_start_matches("curl: ").trim();
        Err(sentence(&format!("Could not download {url}: {why}")))
    }
}

fn fetch(location: &str, scratch: &Scratch) -> Result<File, String> {
    let path = match location.strip_prefix("file://") {
        Some(p) => PathBuf::from(p),
        None => {
            download(location, &scratch.0)?;
            scratch.0.clone()
        }
    };
    File::open(&path).map_err(|e| format!("Could not open {}: {e}.", path.display()))
}

/// Hand the daemon fresh metadata for each due download remote (all with `force`); fetch failures are logged and skipped.
pub async fn refresh(conn: &Connection, force: bool) -> Result<(), Error> {
    let remotes: Vec<Dict> = call(conn, "GetRemotes", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    let now = now();
    for r in remotes {
        let enabled = r
            .get("Enabled")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false);
        let (id, uri) = (text(&r, "RemoteId"), text(&r, "Uri"));
        if !enabled || number(&r, "Type") != u64::from(REMOTE_DOWNLOAD) || uri.is_empty() {
            continue;
        }
        if !force
            && !due(
                number(&r, "ModificationTime"),
                number(&r, "RefreshInterval"),
                now,
            )
        {
            continue;
        }
        let (data, sig) = (Scratch::new("metadata"), Scratch::new("metadata.jcat"));
        let fetched = download(&uri, &data.0)
            .and_then(|()| download(&format!("{uri}.jcat"), &sig.0))
            .and_then(|()| {
                let open = |p: &Path| File::open(p).map_err(|e| e.to_string());
                Ok((open(&data.0)?, open(&sig.0)?))
            });
        let (data, sig) = match fetched {
            Ok(f) => f,
            Err(e) => {
                log::info!("refreshing firmware metadata for {id}: {e}");
                continue;
            }
        };
        if let Err(e) = call(
            conn,
            "UpdateMetadata",
            &(id.as_str(), Fd::from(&data), Fd::from(&sig)),
        )
        .await
        {
            log::info!("firmware metadata for {id}: {e}");
        }
    }
    Ok(())
}

fn status_line(status: u32, name: &str) -> Option<String> {
    Some(match status {
        STATUS_DECOMPRESSING => format!("Preparing the firmware for {name}"),
        STATUS_DEVICE_WRITE | STATUS_DEVICE_ERASE => format!("Writing the firmware to {name}"),
        STATUS_DEVICE_VERIFY | STATUS_DEVICE_READ => format!("Verifying {name}"),
        STATUS_DEVICE_RESTART => format!("Restarting {name}"),
        STATUS_SCHEDULING => format!("Scheduling the update for {name}"),
        STATUS_WAITING_FOR_AUTH => "Waiting for authentication".into(),
        STATUS_DEVICE_BUSY => format!("Waiting for {name}"),
        _ => return None,
    })
}

fn location(release: &Dict, remotes: &[Dict]) -> Option<String> {
    let loc = strings(release, "Locations")
        .into_iter()
        .next()
        .or_else(|| {
            let uri = text(release, "Uri");
            (!uri.is_empty()).then_some(uri)
        })?;
    let remote = text(release, "RemoteId");
    let base = remotes
        .iter()
        .find(|r| text(r, "RemoteId") == remote)
        .map(|r| text(r, "FirmwareBaseUri"))
        .filter(|b| !b.is_empty());
    Some(match base {
        Some(b) if !loc.starts_with("file://") => {
            let file = loc.rsplit('/').next().unwrap_or(&loc);
            format!("{}/{file}", b.trim_end_matches('/'))
        }
        _ => loc,
    })
}

pub async fn install(
    conn: &Connection,
    device: &Update,
    progress: Progress<'_>,
) -> Result<Restart, Error> {
    let name = device.name.as_str();
    let release = newest(conn, &device.id)
        .await?
        .ok_or_else(|| Error::Failed(format!("{name} has no newer firmware.")))?;
    let remotes: Vec<Dict> = call(conn, "GetRemotes", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    let loc = location(&release, &remotes).ok_or_else(|| {
        Error::Failed(format!(
            "The firmware for {name} has nowhere to download from."
        ))
    })?;
    progress(None, Some(format!("Downloading the firmware for {name}")));
    let scratch = Scratch::new("firmware.cab");
    let cab = fetch(&loc, &scratch).map_err(Error::Failed)?;

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(NAME)
        .and_then(|b| b.path(PATH))
        .and_then(|b| b.interface(PROPERTIES))
        .and_then(|b| b.member("PropertiesChanged"))
        .map(|b| b.build().into())
        .map_err(|e| Error::Failed(e.to_string()))?;
    let mut signals = reprisebus::signals(conn, NAME, vec![rule]).await;
    let proxy = zbus::proxy::Builder::<zbus::Proxy>::new(conn)
        .destination(NAME)
        .and_then(|b| b.path(PATH))
        .and_then(|b| b.interface(IFACE))
        .map(|b| b.cache_properties(CacheProperties::No))
        .map_err(failed)?
        .build()
        .await
        .map_err(failed)?;
    let options: HashMap<&str, Value> = HashMap::new();
    // The daemon asks polkit, which may prompt through the session's
    // agent: the call allows it.
    let body = (device.id.as_str(), Fd::from(&cab), options);
    let call = proxy.call_with_flags::<_, _, ()>(
        "Install",
        MethodFlags::AllowInteractiveAuth.into(),
        &body,
    );
    futures_util::pin_mut!(call);
    let mut listening = true;
    let done = loop {
        if !listening {
            break call.as_mut().await;
        }
        match select(call.as_mut(), signals.next()).await {
            Either::Left((r, _)) => break r,
            Either::Right((None | Some(Signal::OwnerChanged), _)) => listening = false,
            Either::Right((Some(Signal::Message(m)), _)) => {
                let Ok((_, changed, _)) = m.body().deserialize::<(String, Dict, Vec<String>)>()
                else {
                    continue;
                };
                let line = changed
                    .get("Status")
                    .and_then(|v| u32::try_from(v).ok())
                    .and_then(|s| status_line(s, name));
                let pct = changed
                    .get("Percentage")
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|&p| p > 0 && p <= 100)
                    .map(|p| f64::from(p) / 100.0);
                if line.is_some() || pct.is_some() {
                    progress(pct, line);
                }
            }
        }
    };
    done.map_err(failed)?;
    let flags = devices(conn)
        .await?
        .iter()
        .find(|d| text(d, "DeviceId") == device.id)
        .map_or(0, |d| number(d, "Flags"));
    Ok(if flags & (NEEDS_REBOOT | NEEDS_SHUTDOWN) != 0 {
        Restart::System
    } else {
        Restart::None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, Value)]) -> Dict {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    OwnedValue::try_from(v.try_clone().unwrap()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn a_device_and_its_release_make_an_update() {
        let device = dict(&[
            ("DeviceId", Value::from("08d460be")),
            ("Name", Value::from("Integrated Webcam™")),
            ("Version", Value::from("1.2.2")),
            ("Flags", Value::from(UPDATABLE)),
        ]);
        let release = dict(&[
            ("Version", Value::from("1.2.4")),
            ("Summary", Value::from("Firmware for the webcam")),
            (
                "Description",
                Value::from(
                    "<p>Fixes the picture going dark.</p><ul><li>Faster &amp; sharper</li></ul>",
                ),
            ),
            ("Urgency", Value::from(3u32)),
        ]);
        let u = update(&device, &release);
        assert_eq!(u.id, "08d460be");
        assert_eq!(u.version_cell(), "1.2.2 → 1.2.4");
        assert_eq!(u.summary, "Fixes the picture going dark. Faster & sharper");
        assert_eq!(u.kind, Kind::Important);
        assert_eq!(u.origin, Origin::Firmware);
    }

    #[test]
    fn notes_fall_back_to_the_summary() {
        let release = dict(&[("Summary", Value::from("Firmware for the webcam"))]);
        assert_eq!(notes(&release), "Firmware for the webcam");
    }

    #[test]
    fn metadata_is_fetched_once_its_interval_has_passed() {
        assert!(due(u64::MAX, 86400, 1000), "never fetched");
        assert!(!due(1000, 86400, 2000));
        assert!(due(1000, 86400, 1000 + 86400));
    }

    #[test]
    fn a_remote_firmware_base_replaces_the_location_directory() {
        let release = dict(&[
            ("RemoteId", Value::from("lvfs")),
            (
                "Locations",
                Value::from(vec!["https://cdn.example.org/downloads/abc-webcam.cab"]),
            ),
        ]);
        let remotes = vec![dict(&[
            ("RemoteId", Value::from("lvfs")),
            (
                "FirmwareBaseUri",
                Value::from("https://mirror.example.org/fw/"),
            ),
        ])];
        assert_eq!(
            location(&release, &remotes).as_deref(),
            Some("https://mirror.example.org/fw/abc-webcam.cab")
        );
        assert_eq!(
            location(&release, &[]).as_deref(),
            Some("https://cdn.example.org/downloads/abc-webcam.cab")
        );
    }

    #[test]
    fn status_lines_name_the_device() {
        assert_eq!(
            status_line(STATUS_DEVICE_WRITE, "Webcam").as_deref(),
            Some("Writing the firmware to Webcam")
        );
        assert_eq!(status_line(1, "Webcam"), None, "idle says nothing");
    }
}
