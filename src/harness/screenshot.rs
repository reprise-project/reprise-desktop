//! The portal's side of `reprise_area_pick_v1` and `reprise_active_window_v1` for tests.

use std::sync::{Arc, Mutex};

use wayland_client::{self as wc, Connection, QueueHandle};

use super::input_client::reprise_active_window_v1::{self, RepriseActiveWindowV1};
use super::input_client::reprise_area_pick_v1::{self, RepriseAreaPickV1};
use super::input_client::reprise_input_v1::RepriseInputV1;
use super::{ClientSide, TestClient};

/// What an area pick or an active-window query heard, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    Picked(i32, i32, i32, i32),
    Cancelled,
    Window(String),
}

pub type Log = Arc<Mutex<Vec<Heard>>>;

impl TestClient {
    /// Bind `reprise_input_v1` v13 and ask for an area for `app`, as the portal's Area target does.
    pub fn pick_area(&self, app: &str) -> (RepriseAreaPickV1, Log) {
        let input: RepriseInputV1 = self.bind("reprise_input_v1", 13);
        let heard = Log::default();
        let pick = input.pick_area(app.into(), &self.qh(), heard.clone());
        (pick, heard)
    }

    /// Bind `reprise_input_v1` v13 and ask for the focused window, as the portal's Active Window
    /// target does.
    pub fn active_window(&self) -> Log {
        let input: RepriseInputV1 = self.bind("reprise_input_v1", 13);
        let heard = Log::default();
        let _: RepriseActiveWindowV1 = input.active_window(&self.qh(), heard.clone());
        heard
    }
}

impl wc::Dispatch<RepriseAreaPickV1, Log> for ClientSide {
    fn event(
        _: &mut Self,
        _: &RepriseAreaPickV1,
        ev: reprise_area_pick_v1::Event,
        heard: &Log,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use reprise_area_pick_v1::Event;
        heard.lock().unwrap().push(match ev {
            Event::Picked {
                x,
                y,
                width,
                height,
            } => Heard::Picked(x, y, width, height),
            Event::Cancelled => Heard::Cancelled,
        });
    }
}

impl wc::Dispatch<RepriseActiveWindowV1, Log> for ClientSide {
    fn event(
        _: &mut Self,
        _: &RepriseActiveWindowV1,
        ev: reprise_active_window_v1::Event,
        heard: &Log,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let reprise_active_window_v1::Event::Window { identifier } = ev;
        heard.lock().unwrap().push(Heard::Window(identifier));
    }
}
