//! The PackageKit side: distribution updates on the system bus. Every request is a transaction (`CreateTransaction`, a method on the object, answers as signals until `Finished`). Refreshing and listing need no password; installing asks polkit, which reprise-polkit prompts for.

use std::collections::HashMap;

use futures_util::stream::StreamExt;
use reprisebus::Signal;
use zbus::Connection;
use zbus::{OwnedObjectPath, OwnedVariant};

use crate::model::{Kind, Origin, Restart, Update, split_package_id};

pub const NAME: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";
pub const IFACE: &str = "org.freedesktop.PackageKit";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `PK_FILTER_ENUM_NONE`'s bit: no filter.
const FILTER_NONE: u64 = 1 << 1;
/// `PK_FILTER_ENUM_INSTALLED`'s bit.
const FILTER_INSTALLED: u64 = 1 << 2;
/// `PK_TRANSACTION_FLAG_ENUM_ONLY_TRUSTED`'s bit: signed packages only.
const ONLY_TRUSTED: u64 = 1 << 1;
/// `PK_TRANSACTION_FLAG_ENUM_SIMULATE`'s bit: say what would change, change nothing.
const SIMULATE: u64 = 1 << 2;

const EXIT_SUCCESS: u32 = 1;
const EXIT_CANCELLED: u32 = 3;

/// `PK_ERROR_ENUM_NOT_AUTHORIZED`: polkit said no, for a dismissed prompt and
/// a refused password alike.
const ERROR_NOT_AUTHORIZED: u32 = 48;

const STATUS_REMOVE: u32 = 6;
const STATUS_DOWNLOAD: u32 = 8;
const STATUS_INSTALL: u32 = 9;
const STATUS_UPDATE: u32 = 10;

/// `Percentage` while PackageKit cannot tell.
const PERCENT_UNKNOWN: u32 = 101;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No PackageKit on this system: not an error, just nothing to ask.
    Unavailable,
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    Failed(String),
}

pub type Progress<'a> = &'a mut dyn FnMut(Option<f64>, Option<String>);

/// What a transaction says about itself besides its progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tx {
    /// Its object path, which [`cancel`] takes.
    Created(String),
    /// Whether it can be cancelled now (`AllowCancel`).
    Cancellable(bool),
}

pub type Watch<'a> = &'a mut dyn FnMut(Tx);

#[derive(Debug, Default)]
struct Said {
    packages: Vec<(u32, String, String)>,
    restart: Restart,
    /// The `size` of each `Details`, in bytes.
    sizes: Vec<u64>,
}

enum Request {
    Refresh,
    GetUpdates,
    Update(Vec<String>),
    /// The installed package that owns this file.
    SearchFile(String),
    GetDetails(Vec<String>),
    /// Remove these packages and any that need them.
    Remove {
        ids: Vec<String>,
        simulate: bool,
    },
    InstallFiles {
        files: Vec<String>,
        simulate: bool,
    },
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
        STATUS_REMOVE => Some(format!("Removing {name}")),
        _ => None,
    }
}

/// Whether a transaction in `status` works on its packages: its percentage then measures that
/// work, not the dependency resolve before it.
fn stepping(status: u32) -> bool {
    matches!(
        status,
        STATUS_REMOVE | STATUS_DOWNLOAD | STATUS_INSTALL | STATUS_UPDATE
    )
}

/// `SearchFiles`' arguments: installed packages only, and a list of paths.
fn search_files(file: &str) -> (u64, Vec<&str>) {
    (FILTER_INSTALLED, vec![file])
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
    if matches!(
        reprisebus::error::name(&e).as_deref(),
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        )
    ) {
        return Error::Unavailable;
    }
    Error::Failed(e.to_string())
}

async fn transaction(
    conn: &Connection,
    request: Request,
    interactive: bool,
    progress: Progress<'_>,
    watch: Option<Watch<'_>>,
) -> Result<Said, Error> {
    let tid: OwnedObjectPath = conn
        .call_method(Some(NAME), PATH, Some(IFACE), "CreateTransaction", &())
        .await
        .and_then(|m| m.body().deserialize())
        .map_err(failed)?;
    let mut watch = watch;
    if let Some(w) = watch.as_mut() {
        w(Tx::Created(tid.to_string()));
    }
    // Subscribed before the request goes out, so no signal of it is lost.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(NAME)
        .path(tid.as_str())
        .build()
        .map(Into::into)
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
        Request::SearchFile(file) => {
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "SearchFiles",
                &search_files(file),
            )
            .await
        }
        Request::GetDetails(ids) => {
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "GetDetails",
                &(ids,),
            )
            .await
        }
        Request::Remove { ids, simulate } => {
            let flags = if *simulate { SIMULATE } else { 0 };
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "RemovePackages",
                &(flags, ids, true, false),
            )
            .await
        }
        // Not ONLY_TRUSTED: a downloaded package carries no repository signature, so polkit
        // asks for an administrator's password for it.
        Request::InstallFiles { files, simulate } => {
            let flags = if *simulate { SIMULATE } else { 0 };
            conn.call_method(
                Some(NAME),
                tid.as_str(),
                Some(TX_IFACE),
                "InstallFiles",
                &(flags, files),
            )
            .await
        }
    };
    sent.map_err(failed)?;

    let mut said = Said::default();
    let mut error: Option<(u32, String)> = None;
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
            (TX_IFACE, "Details") => {
                if let Ok(d) = body.deserialize::<HashMap<String, OwnedVariant>>() {
                    let size = d.get("size").and_then(|v| u64::try_from(v).ok());
                    said.sizes.push(size.unwrap_or(0));
                }
            }
            (TX_IFACE, "ErrorCode") => {
                if let Ok(code) = body.deserialize::<(u32, String)>() {
                    error = Some(code);
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
                    EXIT_CANCELLED => Err(Error::Failed(cancelled(&request).into())),
                    _ => Err(refusal(error, reprisebus::polkit::dismissed)),
                };
            }
            (PROPERTIES, "PropertiesChanged") => {
                let Ok((_, changed, _)) =
                    body.deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                else {
                    continue;
                };
                if let Some(status) = changed.get("Status").and_then(|v| u32::try_from(v).ok())
                    && stepping(status)
                {
                    stepped = true;
                }
                if let Some(pct) = changed
                    .get("Percentage")
                    .and_then(|v| u32::try_from(v).ok())
                    && stepped
                    && pct < PERCENT_UNKNOWN
                {
                    progress(Some(f64::from(pct) / 100.0), None);
                }
                if let Some(can) = changed
                    .get("AllowCancel")
                    .and_then(|v| bool::try_from(v).ok())
                    && let Some(w) = watch.as_mut()
                {
                    w(Tx::Cancellable(can));
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
        match transaction(conn, Request::Refresh, false, &mut |_, _| {}, None).await {
            Ok(_) | Err(Error::Dismissed) => {}
            Err(Error::Unavailable) => return Err(Error::Unavailable),
            Err(Error::Failed(e)) => log::info!("refreshing the package lists: {e}"),
        }
    }
    let said = transaction(conn, Request::GetUpdates, false, &mut |_, _| {}, None).await?;
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

/// Why a transaction failed, from its last `ErrorCode`.
fn refusal(error: Option<(u32, String)>, dismissed: impl FnOnce() -> bool) -> Error {
    match error {
        Some((ERROR_NOT_AUTHORIZED, _)) if dismissed() => Error::Dismissed,
        Some((_, details)) => Error::Failed(details),
        None => Error::Failed("PackageKit did not finish the request.".into()),
    }
}

pub async fn install(
    conn: &Connection,
    ids: Vec<String>,
    progress: Progress<'_>,
) -> Result<Restart, Error> {
    let said = transaction(conn, Request::Update(ids), true, progress, None).await?;
    Ok(said.restart)
}

/// What a cancelled `request` says.
fn cancelled(request: &Request) -> &'static str {
    match request {
        Request::Remove { .. } => "The uninstall was cancelled.",
        Request::InstallFiles { .. } => "The install was cancelled.",
        _ => "The update was cancelled.",
    }
}

/// The installed package that owns `file`, by package id; None when no package does.
pub async fn owner(conn: &Connection, file: &str) -> Result<Option<String>, Error> {
    let said = transaction(
        conn,
        Request::SearchFile(file.into()),
        false,
        &mut |_, _| {},
        None,
    )
    .await?;
    Ok(said.packages.into_iter().next().map(|(_, id, _)| id))
}

/// The installed size of package `id`, in bytes, when PackageKit says.
pub async fn size(conn: &Connection, id: &str) -> Result<Option<u64>, Error> {
    let said = transaction(
        conn,
        Request::GetDetails(vec![id.into()]),
        false,
        &mut |_, _| {},
        None,
    )
    .await?;
    Ok(said.sizes.into_iter().next().filter(|&s| s > 0))
}

/// The package ids a change would touch, besides `own` (the package itself): the rows a simulation
/// lists as updating, installing, removing, obsoleting, reinstalling or downgrading.
fn others(said: Said, own: impl Fn(&str) -> bool) -> Vec<String> {
    said.packages
        .into_iter()
        .filter(|(info, _, _)| matches!(info & 0xffff, 11 | 12 | 13 | 15 | 19 | 20))
        .map(|(_, id, _)| id)
        .filter(|id| !own(id))
        .collect()
}

/// Remove package `id` and any package that needs it. With `simulate` nothing changes, and the
/// answer lists those other packages.
pub async fn remove(
    conn: &Connection,
    id: &str,
    simulate: bool,
    progress: Progress<'_>,
    watch: Watch<'_>,
) -> Result<Vec<String>, Error> {
    let request = Request::Remove {
        ids: vec![id.into()],
        simulate,
    };
    let said = transaction(conn, request, !simulate, progress, Some(watch)).await?;
    let name = split_package_id(id).0;
    Ok(others(said, |o| split_package_id(o).0 == name))
}

/// Install the package in `file` with what it needs. With `simulate` nothing changes, and the
/// answer lists the packages that would come along, without the one named `name`.
pub async fn install_file(
    conn: &Connection,
    file: &str,
    name: &str,
    simulate: bool,
    progress: Progress<'_>,
    watch: Watch<'_>,
) -> Result<Vec<String>, Error> {
    let request = Request::InstallFiles {
        files: vec![file.into()],
        simulate,
    };
    let said = transaction(conn, request, !simulate, progress, Some(watch)).await?;
    Ok(others(said, |o| split_package_id(o).0 == name))
}

/// The content types the backend installs files of (`MimeTypes`).
pub async fn file_types(conn: &Connection) -> Result<Vec<String>, Error> {
    let reply = conn
        .call_method(
            Some(NAME),
            PATH,
            Some(PROPERTIES),
            "Get",
            &(IFACE, "MimeTypes"),
        )
        .await
        .map_err(failed)?;
    let v: OwnedVariant = reply
        .body()
        .deserialize()
        .map_err(|e| Error::Failed(e.to_string()))?;
    Vec::<String>::try_from(v).map_err(|e| Error::Failed(e.to_string()))
}

/// Ask transaction `tid` ([`Tx::Created`]) to stop; it then finishes as cancelled.
pub async fn cancel(conn: &Connection, tid: &str) -> zbus::Result<()> {
    conn.call_method(Some(NAME), tid, Some(TX_IFACE), "Cancel", &())
        .await
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn not_authorized_is_a_dismissal_or_a_refusal() {
        let denied = || {
            Some((
                ERROR_NOT_AUTHORIZED,
                "Failed to obtain authentication.".into(),
            ))
        };
        assert_eq!(refusal(denied(), || true), Error::Dismissed);
        assert_eq!(
            refusal(denied(), || false),
            Error::Failed("Failed to obtain authentication.".into())
        );
        let other = refusal(Some((2, "No network.".into())), || {
            panic!("only NotAuthorized asks whether the prompt was dismissed")
        });
        assert_eq!(other, Error::Failed("No network.".into()));
    }

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
    fn search_files_sends_a_filter_and_a_list_of_paths() {
        use zbus::zvariant::DynamicType;
        assert_eq!(search_files("/x").signature().to_string(), "(tas)");
    }

    #[test]
    fn a_simulation_lists_the_other_packages_it_touches() {
        let said = Said {
            packages: vec![
                (13, "foot;1.28.0-2;x86_64;installed".into(), String::new()),
                (
                    13,
                    "foot-terminfo;1.28.0-2;x86_64;installed".into(),
                    String::new(),
                ),
                (1, "glibc;2.42-1;x86_64;installed".into(), String::new()),
            ],
            ..Said::default()
        };
        let own = |id: &str| split_package_id(id).0 == "foot";
        assert_eq!(
            others(said, own),
            ["foot-terminfo;1.28.0-2;x86_64;installed"]
        );
    }

    #[test]
    fn removing_and_the_work_after_the_resolve_show_their_steps() {
        assert_eq!(
            step(STATUS_REMOVE, "foot;1.28.0-2;x86_64;installed").as_deref(),
            Some("Removing foot")
        );
        assert!(stepping(STATUS_REMOVE) && stepping(STATUS_DOWNLOAD));
        assert!(!stepping(13), "the dependency resolve");
    }

    #[test]
    fn an_install_may_prompt_and_a_check_may_not() {
        assert!(hints(true).contains(&"interactive=true".to_string()));
        assert!(hints(false).contains(&"background=true".to_string()));
    }
}
