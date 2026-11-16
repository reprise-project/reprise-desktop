//! Keep Awake: a cookie of the compositor's `org.freedesktop.ScreenSaver`, which holds off the
//! screen saver, the lock, screen off and idle standby until it is given back or this program
//! leaves the bus.

use zbus::Connection;

const NAME: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";

pub fn hold(conn: &Connection) -> zbus::Result<u32> {
    let body = ("Power Options", "Keep Awake");
    let call = conn.call_method(Some(NAME), PATH, Some(NAME), "Inhibit", &body);
    reprisebus::wait(call)?.body().deserialize()
}

pub fn release(conn: &Connection, cookie: u32) {
    let body = (cookie,);
    let call = conn.call_method(Some(NAME), PATH, Some(NAME), "UnInhibit", &body);
    if let Err(e) = reprisebus::wait(call) {
        log::warn!("Keep Awake: giving cookie {cookie} back failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A screen saver that hands out cookie 7 and keeps what comes back.
    struct Saver(Arc<Mutex<Vec<u32>>>);

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl Saver {
        fn inhibit(&self, _app: &str, _reason: &str) -> u32 {
            7
        }
        #[zbus(name = "UnInhibit")]
        fn un_inhibit(&self, cookie: u32) {
            self.0.lock().unwrap().push(cookie);
        }
    }

    #[test]
    fn keep_awake_holds_a_cookie_and_gives_it_back() {
        let Some(bus) = reprisebus::testbus::bus() else {
            return;
        };
        let connect = || {
            reprisebus::wait(zbus::connection::Builder::address(bus.1.as_str()).build())
                .expect("connect")
        };
        let back = Arc::new(Mutex::new(Vec::new()));
        let saver = connect();
        reprisebus::wait(saver.object_server().at(PATH, Saver(back.clone()))).unwrap();
        reprisebus::wait(saver.request_name(NAME)).unwrap();
        let me = connect();
        assert_eq!(hold(&me).ok(), Some(7));
        release(&me, 7);
        assert_eq!(*back.lock().unwrap(), [7]);
    }
}
