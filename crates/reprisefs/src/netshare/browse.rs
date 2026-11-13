//! The Network place's listings: gvfs's `network:///` and Avahi's SMB and SFTP servers, or an SMB
//! server's shares through `gio list`. Each call blocks for seconds, so it runs off the loop.

use std::collections::HashMap;
use std::time::Duration;

use super::{Address, GIO_WAIT, run_gio};

pub const ROOT: &str = "network:///";
/// How long Avahi gets to report what it has seen.
const AVAHI_WAIT: Duration = Duration::from_millis(1500);
/// The services the Network place lists, each with the scheme it is reached through.
const SERVICES: &[(&str, &str)] = &[("_smb._tcp", "smb"), ("_sftp-ssh._tcp", "sftp")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An SMB server or workgroup, which lists its shares.
    Server,
    /// Opens as a folder once mounted.
    Share,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub uri: String,
    pub kind: Kind,
}

impl Kind {
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Server => "network-server",
            Kind::Share => "folder-remote",
        }
    }

    fn of(uri: &str) -> Kind {
        match Address::parse(uri) {
            Some(a) if a.is_server() => Kind::Server,
            None if uri.starts_with("smb:///") => Kind::Server,
            _ => Kind::Share,
        }
    }
}

/// What `uri` holds: [`ROOT`]'s servers, or a mounted SMB server's shares. Sorted by name.
pub fn list(uri: &str) -> Result<Vec<Entry>, String> {
    let mut out = if uri == ROOT {
        let mut found = run_gio(
            &["list", "-l", "-d", "-a", "standard::target-uri", ROOT],
            GIO_WAIT,
        )
        .map(|o| parse_list(&o))
        .unwrap_or_default();
        for e in avahi() {
            if !found.iter().any(|f| f.uri == e.uri) {
                found.push(e);
            }
        }
        found
    } else {
        parse_list(&run_gio(
            &["list", "-l", "-d", "-a", "standard::target-uri", "--", uri],
            GIO_WAIT,
        )?)
    };
    out.sort_by_key(|e| e.name.to_lowercase());
    Ok(out)
}

/// `gio list -l -d -a standard::target-uri`'s lines: `name\tsize\t(type)\tattributes`.
fn parse_list(out: &str) -> Vec<Entry> {
    out.lines()
        .filter_map(|line| {
            let mut cols = line.splitn(4, '\t');
            let name = cols.next()?.to_string();
            let attrs = cols.nth(2)?;
            let at = attrs.find("standard::target-uri=")? + "standard::target-uri=".len();
            let uri = attrs[at..].split(" standard::").next()?.trim().to_string();
            (!uri.is_empty() && !name.is_empty()).then(|| Entry {
                kind: Kind::of(&uri),
                name,
                uri,
            })
        })
        .collect()
}

/// The SMB and SFTP servers Avahi has seen on IPv4, by their announced names. Empty without
/// Avahi.
fn avahi() -> Vec<Entry> {
    let Ok(conn) = reprisebus::system() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (service, scheme) in SERVICES {
        for (name, address, port) in reprisebus::wait(avahi_browse(&conn, service)) {
            let default = if *scheme == "sftp" { 22 } else { 445 };
            let port = if port != default {
                format!(":{port}")
            } else {
                String::new()
            };
            let uri = format!("{scheme}://{address}{port}/");
            if !out.iter().any(|e: &Entry| e.uri == uri) {
                out.push(Entry {
                    name,
                    kind: Kind::of(&uri),
                    uri,
                });
            }
        }
    }
    out
}

const AVAHI: &str = "org.freedesktop.Avahi";
const IPV4: i32 = 0;

/// Each `service` Avahi reports within [`AVAHI_WAIT`], resolved: name, IPv4 address, port.
async fn avahi_browse(conn: &zbus::Connection, service: &str) -> Vec<(String, String, u16)> {
    use futures_util::StreamExt;
    use zbus::OwnedObjectPath;

    let prepared = conn
        .call_method(
            Some(AVAHI),
            "/",
            Some("org.freedesktop.Avahi.Server2"),
            "ServiceBrowserPrepare",
            &(-1i32, IPV4, service, "", 0u32),
        )
        .await;
    let Ok(browser) = prepared.and_then(|m| m.body().deserialize::<OwnedObjectPath>()) else {
        return Vec::new();
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(AVAHI)
        .path(browser.as_str())
        .build();
    let Ok(rule) = rule else {
        return Vec::new();
    };
    let Ok(mut signals) = zbus::MessageStream::for_match_rule(rule, conn, None).await else {
        return Vec::new();
    };
    let iface = Some("org.freedesktop.Avahi.ServiceBrowser");
    if conn
        .call_method(Some(AVAHI), browser.as_str(), iface, "Start", &())
        .await
        .is_err()
    {
        return Vec::new();
    }
    // (interface, protocol, name, type, domain), each seen once per interface.
    let mut seen: HashMap<String, (i32, i32, String, String)> = HashMap::new();
    let deadline = async_io::Timer::after(AVAHI_WAIT);
    futures_util::pin_mut!(deadline);
    loop {
        let next = futures_util::future::select(signals.next(), &mut deadline).await;
        let msg = match next {
            futures_util::future::Either::Left((Some(Ok(m)), _)) => m,
            _ => break,
        };
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("ItemNew") => {
                if let Ok((i, p, name, ty, domain, _)) =
                    msg.body()
                        .deserialize::<(i32, i32, String, String, String, u32)>()
                {
                    seen.entry(name).or_insert((i, p, ty, domain));
                }
            }
            Some("AllForNow" | "Failure") => break,
            _ => {}
        }
    }
    let _ = conn
        .call_method(Some(AVAHI), browser.as_str(), iface, "Free", &())
        .await;
    let mut out = Vec::new();
    for (name, (i, p, ty, domain)) in seen {
        let resolved = conn
            .call_method(
                Some(AVAHI),
                "/",
                Some("org.freedesktop.Avahi.Server"),
                "ResolveService",
                &(
                    i,
                    p,
                    name.as_str(),
                    ty.as_str(),
                    domain.as_str(),
                    IPV4,
                    0u32,
                ),
            )
            .await;
        type Resolved = (
            i32,
            i32,
            String,
            String,
            String,
            String,
            i32,
            String,
            u16,
            Vec<Vec<u8>>,
            u32,
        );
        if let Ok(r) = resolved.and_then(|m| m.body().deserialize::<Resolved>()) {
            out.push((name, r.7, r.8));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gio_list_lines_give_each_entry_its_name_target_and_kind() {
        let out = "public\t0\t(mountable)\tstandard::name=public standard::target-uri=smb://nas/public\n\
                   my files\t0\t(mountable)\tstandard::target-uri=smb://nas/my%20files standard::name=my files\n\
                   Local Network\t0\t(shortcut)\tstandard::name=smb-root standard::target-uri=smb:///\n\
                   files on box\t0\t(shortcut)\tstandard::target-uri=sftp://box/\n\
                   no target\t0\t(directory)\tstandard::name=x\n";
        let got = parse_list(out);
        let want = [
            ("public", "smb://nas/public", Kind::Share),
            ("my files", "smb://nas/my%20files", Kind::Share),
            ("Local Network", "smb:///", Kind::Server),
            ("files on box", "sftp://box/", Kind::Share),
        ];
        assert_eq!(got.len(), want.len());
        for (e, (name, uri, kind)) in got.iter().zip(want) {
            assert_eq!((e.name.as_str(), e.uri.as_str(), e.kind), (name, uri, kind));
        }
    }
}
