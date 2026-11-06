//! The portal's side of `reprise_lock_screen_v1` for tests.

use std::sync::{Arc, Mutex};

use wayland_client::{self as wc, Connection, QueueHandle};

use super::input_client::reprise_input_v1::RepriseInputV1;
use super::input_client::reprise_lock_screen_v1::{self, RepriseLockScreenV1};
use super::{ClientSide, TestClient};

/// What a lock-screen object heard, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    Screen(String),
    Ended(u32),
}

type Log = Arc<Mutex<Vec<Heard>>>;

pub struct LockScreen {
    pub object: RepriseLockScreenV1,
    heard: Log,
}

impl TestClient {
    /// The version `interface` is offered at, None when this client doesn't see it.
    pub fn global_version(&self, interface: &str) -> Option<u32> {
        (self.side.globals.iter())
            .find(|(_, i, _)| i == interface)
            .map(|&(_, _, v)| v)
    }

    /// Bind `reprise_input_v1` v9 and make a lock-screen object, as the portal does.
    pub fn lock_screen(&self) -> LockScreen {
        let input: RepriseInputV1 = self.bind("reprise_input_v1", 9);
        let heard = Log::default();
        let object = input.lock_screen(&self.qh(), heard.clone());
        LockScreen { object, heard }
    }
}

impl LockScreen {
    /// Send `monitors` as (x, y, width, height, scale percent), primary first.
    pub fn set_layout(&self, monitors: &[(i32, i32, i32, i32, u32)]) {
        let mut bytes = Vec::new();
        for &(x, y, w, h, scale) in monitors {
            for v in [x, y, w, h] {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
            bytes.extend_from_slice(&scale.to_ne_bytes());
        }
        self.object.set_layout(bytes);
    }

    pub fn heard(&self) -> Vec<Heard> {
        self.heard.lock().unwrap().clone()
    }

    /// The names of the `screen` events heard, then forgotten.
    pub fn take_screens(&self) -> Vec<String> {
        let mut heard = self.heard.lock().unwrap();
        let screens = (heard.iter())
            .filter_map(|e| match e {
                Heard::Screen(n) => Some(n.clone()),
                Heard::Ended(_) => None,
            })
            .collect();
        heard.retain(|e| !matches!(e, Heard::Screen(_)));
        screens
    }
}

impl wc::Dispatch<RepriseLockScreenV1, Log> for ClientSide {
    fn event(
        _: &mut Self,
        _: &RepriseLockScreenV1,
        ev: reprise_lock_screen_v1::Event,
        heard: &Log,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use reprise_lock_screen_v1::Event;
        heard.lock().unwrap().push(match ev {
            Event::Screen { name } => Heard::Screen(name),
            Event::Ended { reason } => Heard::Ended(match reason {
                wc::WEnum::Value(r) => r as u32,
                wc::WEnum::Unknown(v) => v,
            }),
        });
    }
}
