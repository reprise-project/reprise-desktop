//! Reprise's Remote Desktop server, `reprise-rdp`, from a program on the session bus: what it
//! reports, ending its connections, a new certificate, and turning its user unit on or off.

use std::collections::HashMap;
use zbus::{Connection, OwnedVariant};

pub const NAME: &str = "org.reprise.RemoteDesktop";
pub const PATH: &str = "/org/reprise/RemoteDesktop";
pub const INTERFACE: &str = NAME;
/// The server's user unit; enabled means remote connections are allowed.
pub const UNIT: &str = "app-org.reprise.RemoteDesktop.service";

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Where it listens (`0.0.0.0:3389`), or `error: ` and why it cannot.
    pub listening: String,
    pub clients: Vec<Client>,
    /// The video encoder it runs.
    pub encoder: String,
    /// The `[remote_desktop] encoder` values that work there besides `auto`.
    pub encoders: Vec<String>,
    /// SHA-256 of its certificate, hex pairs joined by `:`.
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub address: String,
    pub user: String,
    /// Unix seconds.
    pub since: u64,
}

/// The status a `GetAll` of [`INTERFACE`] describes; a missing or mistyped property reads empty.
pub fn from_props(props: &HashMap<String, OwnedVariant>) -> Status {
    let text = |k: &str| {
        props
            .get(k)
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default()
    };
    // `Clients` is `a(sst)`: address, user, since.
    let clients = props
        .get("Clients")
        .and_then(|v| Vec::<(String, String, u64)>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(address, user, since)| Client {
            address,
            user,
            since,
        })
        .collect();
    Status {
        listening: text("Listening"),
        clients,
        encoder: text("Encoder"),
        encoders: props
            .get("Encoders")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .unwrap_or_default(),
        fingerprint: text("Fingerprint"),
    }
}

/// The server's status on `conn`; None while nothing owns [`NAME`] (the server is off).
pub async fn read(conn: &Connection) -> Option<Status> {
    crate::name_owner(conn, NAME).await?;
    let reply = conn
        .call_method(
            Some(NAME),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &(INTERFACE,),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<HashMap<String, OwnedVariant>>()) {
        Ok(props) => Some(from_props(&props)),
        Err(e) => {
            log::info!("{NAME} does not report its status: {e}");
            None
        }
    }
}

/// [`read`] on the program's session bus connection, blocking.
pub fn status() -> Option<Status> {
    let conn = crate::session().ok()?;
    crate::wait(read(&conn))
}

/// End the session of the client at `address`, as [`Client::address`] gives it.
pub fn disconnect_client(address: &str) -> Result<(), String> {
    let conn = session()?;
    told(
        "DisconnectClient",
        crate::wait(conn.call_method(
            Some(NAME),
            PATH,
            Some(INTERFACE),
            "DisconnectClient",
            &(address,),
        )),
    )
}

/// Replace the server's certificate with a new one.
pub fn regenerate() -> Result<(), String> {
    let conn = session()?;
    told(
        "RegenerateCertificate",
        crate::wait(conn.call_method(
            Some(NAME),
            PATH,
            Some(INTERFACE),
            "RegenerateCertificate",
            &(),
        )),
    )
}

/// Whether [`UNIT`] is enabled; false when the user manager does not answer.
pub fn enabled() -> bool {
    let state = crate::session().and_then(|conn| {
        crate::wait(conn.call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            Some(MANAGER),
            "GetUnitFileState",
            &(UNIT,),
        ))?
        .body()
        .deserialize::<String>()
    });
    match state {
        Ok(s) => s == "enabled",
        Err(e) => {
            log::info!("{UNIT}: {e}");
            false
        }
    }
}

/// Enable and start [`UNIT`], or disable and stop it, as `systemctl --user enable --now` and
/// `disable --now` do.
pub fn set_enabled(on: bool) -> Result<(), String> {
    let conn = session()?;
    let units = [UNIT];
    let manager = Some(MANAGER);
    let files = if on {
        crate::wait(conn.call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            manager,
            "EnableUnitFiles",
            &(&units[..], false, false),
        ))
    } else {
        crate::wait(conn.call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            manager,
            "DisableUnitFiles",
            &(&units[..], false),
        ))
    };
    told("the unit file", files)?;
    told(
        "Reload",
        crate::wait(conn.call_method(Some(SYSTEMD), SYSTEMD_PATH, manager, "Reload", &())),
    )?;
    let job = if on { "StartUnit" } else { "StopUnit" };
    told(
        job,
        crate::wait(conn.call_method(
            Some(SYSTEMD),
            SYSTEMD_PATH,
            manager,
            job,
            &(UNIT, "replace"),
        )),
    )
}

fn session() -> Result<Connection, String> {
    crate::session().map_err(|e| e.to_string())
}

/// A call's reply as the user is told it: a method error's message, or the error itself.
fn told(what: &str, reply: zbus::Result<zbus::Message>) -> Result<(), String> {
    let e = match reply {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    log::warn!("remote desktop, {what}: {e}");
    Err(match e {
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        e => e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use zbus::Variant;

    fn owned(v: Variant<'_>) -> OwnedVariant {
        v.try_to_owned().unwrap()
    }

    #[test]
    fn clients_parse_from_the_bus_signature() {
        let wire = vec![(
            "192.168.1.30:51234".to_string(),
            "user".to_string(),
            1_762_000_000u64,
        )];
        let clients = owned(Variant::from(wire));
        assert_eq!(clients.value_signature(), "a(sst)");
        let props = HashMap::from([
            (
                "Listening".to_string(),
                owned(Variant::from("0.0.0.0:3389")),
            ),
            ("Clients".to_string(), clients),
            ("Encoder".to_string(), owned(Variant::from("VA-API H.264"))),
            ("Fingerprint".to_string(), owned(Variant::from("AB:CD"))),
        ]);
        assert_eq!(
            from_props(&props),
            Status {
                listening: "0.0.0.0:3389".into(),
                clients: vec![Client {
                    address: "192.168.1.30:51234".into(),
                    user: "user".into(),
                    since: 1_762_000_000,
                }],
                encoder: "VA-API H.264".into(),
                encoders: Vec::new(),
                fingerprint: "AB:CD".into(),
            }
        );
        assert_eq!(
            from_props(&HashMap::new()),
            Status::default(),
            "a missing property reads empty"
        );
    }

    #[test]
    fn encoders_parse_from_the_bus() {
        let wire = owned(Variant::from(vec!["vaapi", "software", "none"]));
        assert_eq!(wire.value_signature(), "as");
        let props = HashMap::from([("Encoders".to_string(), wire)]);
        assert_eq!(from_props(&props).encoders, ["vaapi", "software", "none"]);
    }

    struct Bus(Child, String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn bus() -> Option<Bus> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some(Bus(child, line.trim().to_string()))
    }

    struct FakeServer;

    #[zbus::interface(name = "org.reprise.RemoteDesktop")]
    impl FakeServer {
        #[zbus(property)]
        fn listening(&self) -> String {
            "0.0.0.0:3389".into()
        }
        #[zbus(property)]
        fn clients(&self) -> Vec<(String, String, u64)> {
            vec![("192.168.1.30:51234".into(), "user".into(), 1_762_000_000)]
        }
        #[zbus(property)]
        fn encoder(&self) -> String {
            "VA-API H.264".into()
        }
        #[zbus(property)]
        fn fingerprint(&self) -> String {
            "AB:CD".into()
        }
    }

    #[test]
    fn the_status_is_read_from_the_server_and_none_while_it_is_off() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async move {
            let connect = || zbus::connection::Builder::address(bus.1.as_str()).build();
            let conn = connect().await.unwrap();
            assert_eq!(read(&conn).await, None, "nothing owns the name");
            let server = connect().await.unwrap();
            server.object_server().at(PATH, FakeServer).await.unwrap();
            server.request_name(NAME).await.unwrap();
            let status = read(&conn).await.expect("the server answers");
            assert_eq!(status.listening, "0.0.0.0:3389");
            assert_eq!(status.clients.len(), 1);
            assert_eq!(status.clients[0].user, "user");
        });
    }
}
