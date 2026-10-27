//! Whether fprintd on the system bus has a finger enrolled for a user. The verification itself
//! runs in PAM (`pam_fprintd`), never here.

use zbus::OwnedObjectPath;
use zbus::{Connection, Proxy};

pub const NAME: &str = "net.reactivated.Fprint";
const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
const MANAGER: &str = "net.reactivated.Fprint.Manager";
const DEVICE: &str = "net.reactivated.Fprint.Device";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// [`enrolled`] on a system bus connection of its own, blocking, with each call cut off after 5 s
/// (false).
pub fn has_finger(user: &str) -> bool {
    zbus::block_on(async {
        let Ok(conn) = zbus::connection::Builder::system()
            .method_timeout(TIMEOUT)
            .build()
            .await
        else {
            return false;
        };
        enrolled(&conn, user).await
    })
}

/// Whether the default reader has a finger of `user`'s. False without fprintd or a reader, or when
/// the caller may not ask about `user`.
pub async fn enrolled(conn: &Connection, user: &str) -> bool {
    let Ok(manager) = Proxy::new(conn, NAME, MANAGER_PATH, MANAGER).await else {
        return false;
    };
    let path: OwnedObjectPath = match manager.call("GetDefaultDevice", &()).await {
        Ok(p) => p,
        Err(e) => {
            log::info!("fprint: no fingerprint reader ({e})");
            return false;
        }
    };
    let Ok(device) = zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NAME)
        .path(path)
        .interface(DEVICE)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
    else {
        return false;
    };
    match device
        .call::<_, _, Vec<String>>("ListEnrolledFingers", &(user,))
        .await
    {
        Ok(fingers) => !fingers.is_empty(),
        Err(e) => {
            log::info!("fprint: no enrolled finger for {user} ({e})");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

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

    async fn connect(address: &str) -> Connection {
        zbus::connection::Builder::address(address)
            .build()
            .await
            .expect("connect")
    }

    const DEVICE_PATH: &str = "/net/reactivated/Fprint/Device/0";

    struct Manager;

    #[zbus::interface(name = "net.reactivated.Fprint.Manager")]
    impl Manager {
        fn get_default_device(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(DEVICE_PATH).unwrap()
        }
    }

    struct FakeDevice(Vec<String>);

    #[zbus::interface(name = "net.reactivated.Fprint.Device")]
    impl FakeDevice {
        fn list_enrolled_fingers(&self, user: &str) -> Vec<String> {
            if user == "someone" {
                self.0.clone()
            } else {
                Vec::new()
            }
        }
    }

    fn ask(fingers: Option<&[&str]>, user: &str) -> Option<bool> {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return None;
        };
        Some(zbus::block_on(async move {
            let _fprintd = match fingers {
                Some(fingers) => {
                    let c = connect(&bus.1).await;
                    c.object_server().at(MANAGER_PATH, Manager).await.unwrap();
                    let device = FakeDevice(fingers.iter().map(|f| f.to_string()).collect());
                    c.object_server().at(DEVICE_PATH, device).await.unwrap();
                    c.request_name(NAME).await.unwrap();
                    Some(c)
                }
                None => None,
            };
            let conn = connect(&bus.1).await;
            enrolled(&conn, user).await
        }))
    }

    #[test]
    fn a_user_with_a_finger_has_one_enrolled() {
        let Some(yes) = ask(Some(&["right-index-finger"]), "someone") else {
            return;
        };
        assert!(yes);
        assert_eq!(ask(Some(&["right-index-finger"]), "other"), Some(false));
        assert_eq!(ask(Some(&[]), "someone"), Some(false));
    }

    #[test]
    fn no_fprintd_means_no_finger() {
        assert!(!ask(None, "someone").unwrap_or(false));
    }
}
