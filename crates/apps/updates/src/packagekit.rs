//! The PackageKit side: distribution updates on the system bus. Every request is a transaction (`CreateTransaction`, a method on the object, answers as signals until `Finished`). Refreshing and listing need no password; installing asks polkit, which reprise-polkit prompts for.

use std::collections::HashMap;

use futures_util::stream::StreamExt;
use reprisebus::Signal;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::model::{Kind, Origin, Restart, Update, split_package_id};

pub const NAME: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";
pub const IFACE: &str = "org.freedesktop.PackageKit";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `PK_FILTER_ENUM_NONE`'s bit: no filter.
const FILTER_NONE: u64 = 1 << 1;
/// `PK_TRANSACTION_FLAG_ENUM_ONLY_TRUSTED`'s bit: signed packages only.
const ONLY_TRUSTED: u64 = 1 << 1;

const EXIT_SUCCESS: u32 = 1;
const EXIT_CANCELLED: u32 = 3;

const STATUS_DOWNLOAD: u32 = 8;
const STATUS_INSTALL: u32 = 9;
const STATUS_UPDATE: u32 = 10;

/// `Percentage` while PackageKit cannot tell.
const PERCENT_UNKNOWN: u32 = 101;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No PackageKit on this system: not an error, just nothing to ask.
    Unavailable,
    Failed(String),
}

pub type Progress<'a> = &'a mut dyn FnMut(Option<f64>, Option<String>);

#[derive(Debug, Default)]
struct Said {
    packages: Vec<(u32, String, String)>,
    restart: Restart,
}

enum Request {
    Refresh,
    GetUpdates,
    Update(Vec<String>),
}

/// The update kind `Package`'s `info` gives. Since PackageKit 1.2.6 the low 16 bits are the info and the high ones a severity; None for a row that is no installable update.
fn kind(info: u32) -> Option<Kind> {
    Some(match info & 0xffff {
        3 => Kind::Low,
        4 => Kind::Enhancement,
        6 => Kind::Bugfix,
        7 | 26 => Kind::Important,
        8 => Kind::Security,
        0 | 2 | 5 => Kind::Normal,
        _ => return None,
    })
}

fn restart(kind: u32) -> Restart {
    match kind {
        3 | 5 => Restart::Session,
        4 | 6 => Restart::System,
        _ => Restart::None,
    }
}

fn step(status: u32, package_id: &str) -> Option<String> {
    let (name, _) = split_package_id(package_id);
    match status {
        STATUS_DOWNLOAD => Some(format!("Downloading {name}")),
        STATUS_INSTALL | STATUS_UPDATE => Some(format!("Installing {name}")),
        _ => None,
    }
}

fn hints(interactive: bool) -> Vec<String> {
    let mut out = vec![
        format!("interactive={interactive}"),
        format!("background={}", !interactive),
    ];
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
    if let Some(l) = locale {
        out.push(format!("locale={l}"));
    }
    out
}

fn failed(e: zbus::Error) -> Error {
    if let zbus::Error::MethodError(name, _, _) = &e
        && matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        )
    {
        return Error::Unavailable;
    }
    Error::Failed(e.to_string())
}

async fn transaction(
    conn: &Connection,
    request: Request,
    interactive: bool,
    progress: Progress<'_>,
) -> Result<Said, Error> {
    let tid: OwnedObjectPath = conn
        .call_method(Some(NAME), PATH, Some(IFACE), "CreateTransaction", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    // Subscribed before the request goes out, so no signal of it is lost.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(NAME)
        .and_then(|b| b.path(tid.as_str()))
        .map(|b| b.build().into())
        .map_err(|e| Error::Failed(e.to_string()))?;
    let mut signals = reprisebus::signals(conn, NAME, vec![rule]).await;
    // Old daemons lack some hints; none is worth failing over.
    if let Err(e) = conn
        .call_method(
            Some(NAME),
            tid.as_str(),
            Some(TX_IFACE),
            "SetHints",
            &(hints(interactive),),
        )
        .await
    {
        log::debug!("SetHints: {e}");
    }
    let sent = match &request {
        Request::Refresh => {
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "RefreshCache",
                &(false,),
            )
            .await
        }
        Request::GetUpdates => {
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "GetUpdates",
                &(FILTER_NONE,),
            )
            .await
        }
        Request::Update(ids) => {
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "UpdatePackages",
                &(ONLY_TRUSTED, ids),
            )
            .await
        }
    };
    sent.map_err(failed)?;

    let mut said = Said::default();
    let mut error: Option<String> = None;
    // The percentage runs 0 to 100 through the dependency resolve before it restarts for download and install; it is shown only once the packages' own steps begin, so the bar never goes back.
    let mut stepped = false;
    while let Some(sig) = signals.next().await {
        let m = match sig {
            Signal::OwnerChanged => {
                return Err(Error::Failed("PackageKit stopped.".into()));
            }
            Signal::Message(m) => m,
        };
        let hdr = m.header();
        let (Some(iface), Some(member)) = (hdr.interface(), hdr.member()) else {
            continue;
        };
        let body = m.body();
        match (iface.as_str(), member.as_str()) {
            (TX_IFACE, "Package") => {
                if let Ok(row) = body.deserialize::<(u32, String, String)>() {
                    said.packages.push(row);
                }
            }
            (TX_IFACE, "ErrorCode") => {
                if let Ok((_, details)) = body.deserialize::<(u32, String)>() {
                    error = Some(details);
                }
            }
            (TX_IFACE, "RequireRestart") => {
                if let Ok((kind, _)) = body.deserialize::<(u32, String)>() {
                    said.restart = said.restart.max(restart(kind));
                }
            }
            (TX_IFACE, "ItemProgress") => {
                if let Ok((id, status, _)) = body.deserialize::<(String, u32, u32)>()
                    && let Some(line) = step(status, &id)
                {
                    stepped = true;
                    progress(None, Some(line));
                }
            }
            (TX_IFACE, "Finished") => {
                let (exit, _) = body.deserialize::<(u32, u32)>().unwrap_or((0, 0));
                return match exit {
                    EXIT_SUCCESS => Ok(said),
                    EXIT_CANCELLED => Err(Error::Failed("The update was cancelled.".into())),
                    _ => Err(Error::Failed(error.unwrap_or_else(|| {
                        "PackageKit did not finish the request.".into()
                    }))),
                };
            }
            (PROPERTIES, "PropertiesChanged") => {
                let Ok((_, changed, _)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                else {
                    continue;
                };
                if let Some(pct) = changed
                    .get("Percentage")
                    .and_then(|v| u32::try_from(v).ok())
                    && stepped
                    && pct < PERCENT_UNKNOWN
                {
                    progress(Some(f64::from(pct) / 100.0), None);
                }
            }
            _ => {}
        }
    }
    Err(Error::Failed("PackageKit went quiet.".into()))
}

/// The package updates there are. With `refresh`, the lists are downloaded first; a refresh that fails (no network) leaves the old lists, still worth reading.
pub async fn updates(conn: &Connection, refresh: bool) -> Result<Vec<Update>, Error> {
    if refresh {
        match transaction(conn, Request::Refresh, false, &mut |_, _| {}).await {
            Ok(_) => {}
            Err(Error::Unavailable) => return Err(Error::Unavailable),
            Err(Error::Failed(e)) => log::info!("refreshing the package lists: {e}"),
        }
    }
    let said = transaction(conn, Request::GetUpdates, false, &mut |_, _| {}).await?;
    Ok(fold(said.packages))
}

fn fold(rows: Vec<(u32, String, String)>) -> Vec<Update> {
    rows.into_iter()
        .filter_map(|(info, id, summary)| {
            let kind = kind(info)?;
            let (name, version) = split_package_id(&id);
            Some(Update {
                name: name.to_string(),
                current: String::new(),
                version: version.to_string(),
                id,
                summary,
                kind,
                origin: Origin::Package,
            })
        })
        .collect()
}

pub async fn install(
    conn: &Connection,
    ids: Vec<String>,
    progress: Progress<'_>,
) -> Result<Restart, Error> {
    let said = transaction(conn, Request::Update(ids), true, progress).await?;
    Ok(said.restart)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_rows_become_updates_and_blocked_ones_are_left_out() {
        let rows = vec![
            (8, "openssl;3.3.2-1;x86_64;core".into(), "TLS".into()),
            // Info in the low bits, a severity above them.
            (
                (3 << 16) | 6,
                "bash;5.2-1;x86_64;core".into(),
                String::new(),
            ),
            (9, "kernel;6.11-1;x86_64;core".into(), String::new()),
        ];
        let u = fold(rows);
        assert_eq!(u.len(), 2, "the blocked row is dropped");
        assert_eq!(u[0].name, "openssl");
        assert_eq!(u[0].version, "3.3.2-1");
        assert_eq!(u[0].kind, Kind::Security);
        assert_eq!(u[0].id, "openssl;3.3.2-1;x86_64;core");
        assert_eq!(u[1].kind, Kind::Bugfix);
    }

    #[test]
    fn restarts_rank_by_what_they_ask() {
        assert_eq!(restart(1), Restart::None);
        assert_eq!(restart(2), Restart::None, "an application restart");
        assert_eq!(restart(3), Restart::Session);
        assert_eq!(restart(6), Restart::System);
        assert!(Restart::System > Restart::Session);
    }

    #[test]
    fn item_progress_names_the_package() {
        assert_eq!(
            step(STATUS_DOWNLOAD, "firefox;131;x86_64;extra").as_deref(),
            Some("Downloading firefox")
        );
        assert_eq!(
            step(STATUS_UPDATE, "firefox;131;x86_64;extra").as_deref(),
            Some("Installing firefox")
        );
        assert_eq!(step(2, "firefox;131;x86_64;extra"), None);
    }

    #[test]
    fn an_install_may_prompt_and_a_check_may_not() {
        assert!(hints(true).contains(&"interactive=true".to_string()));
        assert!(hints(false).contains(&"background=true".to_string()));
    }
}
