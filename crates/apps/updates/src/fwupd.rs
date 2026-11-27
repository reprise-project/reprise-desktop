//! fwupd on the system bus. Metadata and cabinets go to the daemon as file descriptors;
//! the files are fetched here with `curl`, as the daemon downloads nothing itself.

use reprisekit::tr;
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
use zbus::{Fd, OwnedVariant, Variant};

use crate::model::{Kind, Origin, Request, Restart, Update};

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
const STATUS_WAITING_FOR_USER: u32 = 14;

/// `FwupdFeatureFlags`: this client shows a request's generic and device-specific messages.
const FEATURE_REQUESTS: u64 = 1 << 4;
const FEATURE_REQUESTS_NON_GENERIC: u64 = 1 << 9;

/// `FwupdRequestKind::Post`: the request finishes the update after the install returns.
const REQUEST_POST: u32 = 1;
/// `FwupdRequestFlags::NonGenericMessage`: only the request's own message says what to do.
const REQUEST_NON_GENERIC_MESSAGE: u64 = 1 << 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    Failed(String),
}

pub type Progress<'a> = &'a mut dyn FnMut(Option<f64>, Option<String>);
/// A device's request while its firmware installs, and `None` once it carries on.
pub type Asks<'a> = &'a mut dyn FnMut(Option<Request>);

type Dict = HashMap<String, OwnedVariant>;

fn failed(e: zbus::Error) -> Error {
    if matches!(
        reprisebus::error::name(&e).as_deref(),
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        )
    ) {
        return Error::Unavailable;
    }
    Error::Failed(reprisebus::error::message(&e).map_or_else(|| e.to_string(), sentence))
}

/// fwupd answers `AuthFailed` for a dismissed prompt and a refused password
/// alike; `dismissed` tells them apart.
fn install_failed(e: zbus::Error, dismissed: impl FnOnce() -> bool) -> Error {
    let refused = |n: &str| n == "org.freedesktop.fwupd.AuthFailed";
    match reprisebus::polkit::refusal(&e, refused, dismissed) {
        Some(reprisebus::polkit::Refusal::Dismissed) => Error::Dismissed,
        Some(reprisebus::polkit::Refusal::TimedOut) => Error::Failed(unfinished()),
        _ => failed(e),
    }
}

/// How long an Install may take in all: a firmware write runs for many minutes,
/// and a device may ask to be replugged or restarted on the way.
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// What ended a wait in [`install`]'s loop.
enum Next {
    Done(zbus::Result<Option<()>>),
    Signal(Option<Signal>),
    /// The password prompt was up for [`reprisebus::polkit::PROMPT_TIMEOUT`].
    PromptOver,
}

/// An install's password prompt, over at [`reprisebus::polkit::PROMPT_TIMEOUT`] unless fwupd
/// shows the install past it first. Status is the daemon's: loading the cabinet, which comes
/// before the prompt, and another client's download set it too.
struct Prompt {
    timer: Option<async_io::Timer>,
    /// fwupd said it waits for authentication, which it sets just before its polkit check
    /// (deferred 250 ms, so a check that needs no prompt may never show it).
    asked: bool,
}

impl Prompt {
    fn new() -> Prompt {
        let timer = async_io::Timer::after(reprisebus::polkit::PROMPT_TIMEOUT);
        Prompt {
            timer: Some(timer),
            asked: false,
        }
    }

    /// fwupd's status became `s`: past the prompt once it leaves waiting for authentication, or,
    /// with that never seen, once it works at the device.
    fn status(&mut self, s: u32) {
        if s == STATUS_WAITING_FOR_AUTH {
            self.asked = true;
        } else if self.asked || at_device(s) {
            self.past();
        }
    }

    /// The install is past its prompt, or nothing more will say so.
    fn past(&mut self) {
        self.timer = None;
    }
}

fn at_device(s: u32) -> bool {
    matches!(
        s,
        STATUS_DEVICE_RESTART
            | STATUS_DEVICE_WRITE
            | STATUS_DEVICE_VERIFY
            | STATUS_SCHEDULING
            | STATUS_DEVICE_READ
            | STATUS_DEVICE_ERASE
            | STATUS_DEVICE_BUSY
            | STATUS_WAITING_FOR_USER
    )
}

fn unfinished() -> String {
    tr!("fwupd-unfinished")
}

fn sentence(s: &str) -> String {
    let mut s = s.trim().to_string();
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

fn nothing_newer(e: &zbus::Error) -> bool {
    matches!(
        reprisebus::error::name(e).as_deref(),
        Some(
            "org.freedesktop.fwupd.NothingToDo"
                | "org.freedesktop.fwupd.NotSupported"
                | "org.freedesktop.fwupd.NotFound"
        )
    )
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
    body: &(impl zbus::export::serde::Serialize + zbus::DynamicType),
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

/// Signals after which the firmware updates may differ: new metadata, a device plugged in or out.
pub fn change_rules() -> zbus::Result<Vec<zbus::OwnedMatchRule>> {
    ["Changed", "DeviceAdded", "DeviceRemoved"]
        .into_iter()
        .map(|member| {
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(NAME)
                .path(PATH)
                .interface(IFACE)
                .member(member)
                .build()
                .map(Into::into)
        })
        .collect()
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
            Ok(None) | Err(Error::Dismissed) => {}
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

/// Fetches `url` into `to`. A connection that stalls for a minute fails, so a dead server does
/// not hold the worker; a slow but moving download of a large capsule still completes.
fn download(url: &str, to: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "2",
            "--connect-timeout",
            "30",
            "--speed-limit",
            "1",
            "--speed-time",
            "60",
        ])
        .arg("--output")
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| tr!("fwupd-curl-failed", why = e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.trim().trim_start_matches("curl: ").trim();
        Err(sentence(&tr!(
            "fwupd-download-failed",
            url = url,
            why = why
        )))
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
    File::open(&path).map_err(|e| {
        tr!(
            "fwupd-open-failed",
            path = path.display().to_string(),
            why = e.to_string()
        )
    })
}

/// fwupd's polkit action for `UpdateMetadata`.
const REFRESH_REMOTE: &str = "org.freedesktop.fwupd.refresh-remote";

/// Hand the daemon fresh metadata for each due download remote (all with `force`, which is the
/// user's Check Now); fetch failures are logged and skipped. fwupd lets polkit prompt for any
/// call, so without `force` nothing is handed over when that would take a password.
pub async fn refresh(conn: &Connection, force: bool) -> Result<(), Error> {
    let remotes: Vec<Dict> = call(conn, "GetRemotes", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    let now = now();
    let fetch: Vec<(String, String)> = remotes
        .iter()
        .filter(|r| {
            let enabled = r
                .get("Enabled")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false);
            enabled
                && number(r, "Type") == u64::from(REMOTE_DOWNLOAD)
                && !text(r, "Uri").is_empty()
                && (force
                    || due(
                        number(r, "ModificationTime"),
                        number(r, "RefreshInterval"),
                        now,
                    ))
        })
        .map(|r| (text(r, "RemoteId"), text(r, "Uri")))
        .collect();
    if fetch.is_empty() {
        return Ok(());
    }
    if !force && !reprisebus::polkit::allowed_without_prompt(conn, REFRESH_REMOTE).await {
        log::info!("firmware metadata is not refreshed in the background: it needs a password");
        return Ok(());
    }
    for (id, uri) in fetch {
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
        STATUS_DECOMPRESSING => tr!("fwupd-step-decompress", name = name),
        STATUS_DEVICE_WRITE | STATUS_DEVICE_ERASE => tr!("fwupd-step-write", name = name),
        STATUS_DEVICE_VERIFY | STATUS_DEVICE_READ => tr!("fwupd-step-verify", name = name),
        STATUS_DEVICE_RESTART => tr!("fwupd-step-restart", name = name),
        STATUS_SCHEDULING => tr!("fwupd-step-schedule", name = name),
        STATUS_WAITING_FOR_AUTH => tr!("fwupd-step-auth"),
        STATUS_DEVICE_BUSY => tr!("fwupd-step-busy", name = name),
        STATUS_WAITING_FOR_USER => tr!("fwupd-step-user"),
        _ => return None,
    })
}

/// What a `DeviceRequest` asks of the user, said in Reprise's words unless only the device's own
/// message will do.
fn request(r: &Dict, name: &str) -> Request {
    let own = text(r, "UpdateMessage");
    let generic = match text(r, "AppstreamId").as_str() {
        "org.freedesktop.fwupd.request.remove-replug" | "org.freedesktop.fwupd.replug-install" => {
            Some(tr!("fwupd-request-replug", name = name))
        }
        "org.freedesktop.fwupd.request.remove-usb-cable" => {
            Some(tr!("fwupd-request-remove-cable", name = name))
        }
        "org.freedesktop.fwupd.request.insert-usb-cable" => {
            Some(tr!("fwupd-request-insert-cable", name = name))
        }
        "org.freedesktop.fwupd.replug-power" => {
            Some(tr!("fwupd-request-replug-power", name = name))
        }
        "org.freedesktop.fwupd.request.press-unlock" => {
            Some(tr!("fwupd-request-unlock", name = name))
        }
        "org.freedesktop.fwupd.request.do-not-power-off" => {
            Some(tr!("fwupd-request-keep-power", name = name))
        }
        "org.freedesktop.fwupd.restart-daemon" => {
            Some(tr!("fwupd-request-restart-daemon", name = name))
        }
        _ => None,
    };
    let message = if number(r, "Flags") & REQUEST_NON_GENERIC_MESSAGE != 0 && !own.is_empty() {
        sentence(&own)
    } else {
        generic
            .or_else(|| (!own.is_empty()).then(|| sentence(&own)))
            .unwrap_or_else(|| tr!("fwupd-request-waiting", name = name))
    };
    Request {
        message,
        now: number(r, "RequestKind") != u64::from(REQUEST_POST),
    }
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
    asks: Asks<'_>,
) -> Result<Restart, Error> {
    let name = device.name.as_str();
    let release = newest(conn, &device.id)
        .await?
        .ok_or_else(|| Error::Failed(tr!("fwupd-no-newer", name = name)))?;
    let remotes: Vec<Dict> = call(conn, "GetRemotes", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    let loc = location(&release, &remotes)
        .ok_or_else(|| Error::Failed(tr!("fwupd-no-source", name = name)))?;
    progress(None, Some(tr!("fwupd-downloading", name = name)));
    let scratch = Scratch::new("firmware.cab");
    let cab = fetch(&loc, &scratch).map_err(Error::Failed)?;

    let rule = |iface: &'static str, member: &'static str| {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .path(PATH)
            .interface(iface)
            .member(member)
            .build()
            .map(Into::into)
            .map_err(|e| Error::Failed(e.to_string()))
    };
    let rules = vec![
        rule(PROPERTIES, "PropertiesChanged")?,
        rule(IFACE, "DeviceRequest")?,
    ];
    let mut signals = reprisebus::signals(conn, NAME, rules).await;
    // Firmware whose metadata requires a client that shows requests installs only with these.
    if let Err(e) = call(
        conn,
        "SetFeatureFlags",
        &(FEATURE_REQUESTS | FEATURE_REQUESTS_NON_GENERIC,),
    )
    .await
    {
        log::info!("fwupd feature flags: {e}");
    }
    let proxy = zbus::proxy::Builder::<zbus::Proxy>::new(conn)
        .destination(NAME)
        .path(PATH)
        .interface(IFACE)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(failed)?;
    let options: HashMap<&str, Variant> = HashMap::new();
    // The daemon asks polkit, which may prompt through the session's
    // agent: the call allows it.
    let body = (device.id.as_str(), Fd::from(&cab), options);
    let call =
        proxy.call_with_flags::<_, _, ()>("Install", MethodFlags::ALLOW_INTERACTIVE_AUTH, &body);
    let call = reprisebus::polkit::asking(INSTALL_TIMEOUT, true, call);
    futures_util::pin_mut!(call);
    let mut listening = true;
    let mut waiting = false;
    let mut prompt = Prompt::new();
    let done = loop {
        let next = {
            let signal = async {
                match listening {
                    true => signals.next().await,
                    false => std::future::pending().await,
                }
            };
            let prompt_over = async {
                match prompt.timer.as_mut() {
                    Some(t) => drop(t.await),
                    None => std::future::pending().await,
                }
            };
            futures_util::pin_mut!(signal, prompt_over);
            match select(call.as_mut(), select(signal, prompt_over)).await {
                Either::Left((r, _)) => Next::Done(r),
                Either::Right((Either::Left((s, _)), _)) => Next::Signal(s),
                Either::Right((Either::Right(_), _)) => Next::PromptOver,
            }
        };
        match next {
            Next::Done(r) => break r,
            // Dropping the call withdraws its prompt.
            Next::PromptOver => {
                log::warn!(
                    "{name}: no install after {:?}",
                    reprisebus::polkit::PROMPT_TIMEOUT
                );
                return Err(Error::Failed(reprisekit::kit_tr!("polkit-timed-out")));
            }
            Next::Signal(None | Some(Signal::OwnerChanged)) => {
                listening = false;
                prompt.past();
            }
            Next::Signal(Some(Signal::Message(m)))
                if m.header().member().is_some_and(|n| n == "DeviceRequest") =>
            {
                let Ok(r) = m.body().deserialize::<Dict>() else {
                    continue;
                };
                prompt.past();
                let r = request(&r, name);
                log::info!("{name} asks: {}", r.message);
                waiting = r.now;
                asks(Some(r));
            }
            Next::Signal(Some(Signal::Message(m))) => {
                let Ok((_, changed, _)) = m.body().deserialize::<(String, Dict, Vec<String>)>()
                else {
                    continue;
                };
                let status = changed.get("Status").and_then(|v| u32::try_from(v).ok());
                if waiting && status.is_some_and(|s| s != STATUS_WAITING_FOR_USER) {
                    waiting = false;
                    asks(None);
                }
                let line = status.and_then(|s| status_line(s, name));
                let pct = changed
                    .get("Percentage")
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|&p| p > 0 && p <= 100)
                    .map(|p| f64::from(p) / 100.0);
                if let Some(s) = status {
                    prompt.status(s);
                }
                if line.is_some() || pct.is_some() {
                    progress(pct, line);
                }
            }
        }
    };
    done.map_err(|e| install_failed(e, reprisebus::polkit::dismissed))?;
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

    fn dict(pairs: &[(&str, Variant)]) -> Dict {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    OwnedVariant::try_from(v.try_clone().unwrap()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn auth_failed_is_a_dismissal_or_a_refusal() {
        let error = |name| reprisebus::error::reply(name, "Failed to obtain auth");
        let auth_failed = || error("org.freedesktop.fwupd.AuthFailed");
        assert_eq!(install_failed(auth_failed(), || true), Error::Dismissed);
        assert_eq!(
            install_failed(auth_failed(), || false),
            Error::Failed("Failed to obtain auth.".into())
        );
        let other = install_failed(error("org.freedesktop.fwupd.Internal"), || {
            panic!("only AuthFailed asks whether the prompt was dismissed")
        });
        assert_eq!(other, Error::Failed("Failed to obtain auth.".into()));
    }

    #[test]
    fn an_install_past_its_limit_says_fwupd_stopped_answering() {
        let late = zbus::Error::from(std::io::Error::from(std::io::ErrorKind::TimedOut));
        let failed = install_failed(late, || panic!("a timeout is no dismissal"));
        assert_eq!(failed, Error::Failed(unfinished()));
    }

    #[test]
    fn the_prompt_ends_on_leaving_waiting_for_auth_or_else_at_the_device() {
        let mut asked = Prompt::new();
        for s in [1, STATUS_DECOMPRESSING, STATUS_WAITING_FOR_AUTH] {
            asked.status(s);
            assert!(
                asked.timer.is_some(),
                "status {s} comes before or during the prompt"
            );
        }
        asked.status(STATUS_DECOMPRESSING);
        assert!(
            !asked.timer.is_some(),
            "any status after waiting for auth is past it"
        );
        let mut unasked = Prompt::new();
        unasked.status(STATUS_DECOMPRESSING);
        assert!(unasked.timer.is_some(), "loading the cabinet comes first");
        unasked.status(STATUS_DEVICE_WRITE);
        assert!(!unasked.timer.is_some());
        let mut waiting = Prompt::new();
        waiting.status(STATUS_WAITING_FOR_USER);
        assert!(!waiting.timer.is_some());
    }

    #[test]
    fn signals_that_stop_leave_only_the_install_bound() {
        let mut deaf = Prompt::new();
        deaf.status(STATUS_WAITING_FOR_AUTH);
        deaf.past();
        assert!(!deaf.timer.is_some(), "nothing more would end the prompt");
    }

    #[test]
    fn a_daemon_the_bus_cannot_reach_is_unavailable() {
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ] {
            let e = reprisebus::error::reply(name, "The name is not activatable");
            assert_eq!(failed(e), Error::Unavailable, "{name}");
        }
    }

    #[test]
    fn a_device_and_its_release_make_an_update() {
        let device = dict(&[
            ("DeviceId", Variant::from("08d460be")),
            ("Name", Variant::from("Integrated Webcam™")),
            ("Version", Variant::from("1.2.2")),
            ("Flags", Variant::from(UPDATABLE)),
        ]);
        let release = dict(&[
            ("Version", Variant::from("1.2.4")),
            ("Summary", Variant::from("Firmware for the webcam")),
            (
                "Description",
                Variant::from(
                    "<p>Fixes the picture going dark.</p><ul><li>Faster &amp; sharper</li></ul>",
                ),
            ),
            ("Urgency", Variant::from(3u32)),
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
        let release = dict(&[("Summary", Variant::from("Firmware for the webcam"))]);
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
            ("RemoteId", Variant::from("lvfs")),
            (
                "Locations",
                Variant::from(vec!["https://cdn.example.org/downloads/abc-webcam.cab"]),
            ),
        ]);
        let remotes = vec![dict(&[
            ("RemoteId", Variant::from("lvfs")),
            (
                "FirmwareBaseUri",
                Variant::from("https://mirror.example.org/fw/"),
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
    fn a_request_says_what_to_do_in_its_own_words_unless_only_the_device_can() {
        let replug = dict(&[
            (
                "AppstreamId",
                Variant::from("org.freedesktop.fwupd.request.remove-replug"),
            ),
            ("RequestKind", Variant::from(2u32)),
            ("Flags", Variant::from(1u64)),
            ("UpdateMessage", Variant::from("Please pull it out")),
        ]);
        assert_eq!(
            request(&replug, "Webcam"),
            Request {
                message: "Unplug Webcam and plug it back in to continue the update.".into(),
                now: true,
            }
        );

        let own = dict(&[
            (
                "AppstreamId",
                Variant::from("org.freedesktop.fwupd.request.remove-replug"),
            ),
            ("Flags", Variant::from(REQUEST_NON_GENERIC_MESSAGE)),
            ("UpdateMessage", Variant::from("Hold the pairing button")),
        ]);
        assert_eq!(request(&own, "Dock").message, "Hold the pairing button.");

        let unknown = dict(&[
            ("AppstreamId", Variant::from("com.example.dock.lever")),
            ("RequestKind", Variant::from(REQUEST_POST)),
        ]);
        let r = request(&unknown, "Dock");
        assert_eq!(r.message, "Dock is waiting for you to continue the update.");
        assert!(!r.now, "a post request finishes the update afterwards");
    }

    #[test]
    fn status_lines_name_the_device() {
        assert_eq!(
            status_line(STATUS_DEVICE_WRITE, "Webcam").as_deref(),
            Some("Writing the firmware to Webcam")
        );
        assert_eq!(status_line(1, "Webcam"), None, "idle says nothing");
    }

    mod bus {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        use zbus::{OwnedFd, OwnedVariant, Variant};

        use super::super::{Dict, NAME, refresh};

        use reprisebus::testbus::bus;

        use reprisebus::testbus::connect;

        struct Fwupd {
            uri: String,
            handed: Arc<AtomicUsize>,
        }

        #[zbus::interface(name = "org.freedesktop.fwupd")]
        impl Fwupd {
            fn get_remotes(&self) -> Vec<Dict> {
                let v = |v: Variant| OwnedVariant::try_from(v).unwrap();
                vec![Dict::from([
                    ("RemoteId".to_string(), v(Variant::from("lvfs"))),
                    ("Enabled".to_string(), v(Variant::from(true))),
                    ("Type".to_string(), v(Variant::from(1u32))),
                    ("Uri".to_string(), v(Variant::from(self.uri.as_str()))),
                    ("ModificationTime".to_string(), v(Variant::from(0u64))),
                    ("RefreshInterval".to_string(), v(Variant::from(86400u64))),
                ])]
            }

            fn update_metadata(&self, _id: &str, _data: OwnedFd, _sig: OwnedFd) {
                self.handed.fetch_add(1, Ordering::SeqCst);
            }
        }

        struct Authority(Arc<AtomicBool>);

        #[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
        impl Authority {
            fn check_authorization(
                &self,
                _subject: (String, std::collections::HashMap<String, OwnedVariant>),
                action: &str,
                _details: std::collections::HashMap<String, String>,
                flags: u32,
                _cancel: &str,
            ) -> (bool, bool, std::collections::HashMap<String, String>) {
                assert_eq!(action, "org.freedesktop.fwupd.refresh-remote");
                assert_eq!(flags, 0, "asked without a prompt");
                let yes = self.0.load(Ordering::SeqCst);
                (yes, !yes, Default::default())
            }
        }

        /// fwupd prompts whatever the call's flags, so the six-hourly refresh asks polkit first
        /// and leaves the metadata alone when it would take a password; Check Now hands it over.
        #[test]
        fn a_background_refresh_hands_over_no_metadata_that_needs_a_password() {
            let Some(bus) = bus() else {
                return;
            };
            let dir =
                std::env::temp_dir().join(format!("reprise-fwupd-refresh-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let metadata = dir.join("firmware.xml.zst");
            std::fs::write(&metadata, b"metadata").unwrap();
            std::fs::write(dir.join("firmware.xml.zst.jcat"), b"signature").unwrap();
            let handed = Arc::new(AtomicUsize::new(0));
            let allowed = Arc::new(AtomicBool::new(false));
            zbus::block_on(async {
                let services = connect(&bus.1).await;
                let fwupd = Fwupd {
                    uri: format!("file://{}", metadata.display()),
                    handed: handed.clone(),
                };
                reprisebus::testbus::serve(&services, "/", fwupd).await;
                services.request_name(NAME).await.unwrap();
                let authority = Authority(allowed.clone());
                reprisebus::testbus::serve(
                    &services,
                    "/org/freedesktop/PolicyKit1/Authority",
                    authority,
                )
                .await;
                services
                    .request_name("org.freedesktop.PolicyKit1")
                    .await
                    .unwrap();
                let conn = connect(&bus.1).await;

                refresh(&conn, false).await.unwrap();
                assert_eq!(
                    handed.load(Ordering::SeqCst),
                    0,
                    "a password would be asked for"
                );
                refresh(&conn, true).await.unwrap();
                assert_eq!(handed.load(Ordering::SeqCst), 1, "Check Now hands it over");
                allowed.store(true, Ordering::SeqCst);
                refresh(&conn, false).await.unwrap();
                assert_eq!(
                    handed.load(Ordering::SeqCst),
                    2,
                    "allowed without a password"
                );
            });
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
