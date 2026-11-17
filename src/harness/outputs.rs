//! A client's side of wlr-output-management and `reprise_output_trial_v1` for tests, as
//! Settings' Display page and wlr-randr use them.

use std::sync::{Arc, Mutex};

use wayland_client::{self as wc, Connection, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use super::{ClientSide, TestClient};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all
)]
pub mod trial_client {
    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_protocols_wlr::output_management::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-output-trial-v1.xml");
    }
    use self::__interfaces::*;
    use wayland_client;
    use wayland_protocols_wlr::output_management::v1::client::*;
    wayland_scanner::generate_client_code!("protocols/reprise-output-trial-v1.xml");
}

use trial_client::reprise_output_trial_manager_v1::RepriseOutputTrialManagerV1;
use trial_client::reprise_output_trial_v1::{self, RepriseOutputTrialV1};

#[derive(Default)]
pub struct Seen {
    manager: Option<ZwlrOutputManagerV1>,
    serial: u32,
    heads: Vec<SeenHead>,
    modes: Vec<(ZwlrOutputModeV1, (i32, i32))>,
}

struct SeenHead {
    head: ZwlrOutputHeadV1,
    name: String,
    modes: Vec<ZwlrOutputModeV1>,
}

impl Seen {
    fn head_mut(&mut self, head: &ZwlrOutputHeadV1) -> Option<&mut SeenHead> {
        self.heads.iter_mut().find(|h| h.head == *head)
    }
}

/// A configuration's answer once heard: "succeeded", "failed" or "cancelled".
pub type Answer = Arc<Mutex<Option<&'static str>>>;

/// What a trial heard, oldest first: "started", then "kept", "reverted", "failed" or "busy".
pub type Heard = Arc<Mutex<Vec<&'static str>>>;

pub struct Trial {
    pub object: RepriseOutputTrialV1,
    pub heard: Heard,
}

impl Trial {
    pub fn heard(&self) -> Vec<&'static str> {
        self.heard.lock().unwrap().clone()
    }
}

impl TestClient {
    /// Bind `zwlr_output_manager_v1` v4; the heads it announces land in [`ClientSide::output_mgmt`].
    pub fn manage_outputs(&mut self) {
        let manager = self.bind("zwlr_output_manager_v1", 4);
        self.side.output_mgmt.manager = Some(manager);
    }

    /// Apply `name` in its `w`x`h` mode against the heads last heard, as `wlr-randr --output
    /// NAME --mode WxH` does.
    pub fn apply_mode(&self, name: &str, w: i32, h: i32) -> Answer {
        let (conf, answer) = self.mode_configuration(name, w, h);
        conf.apply();
        answer
    }

    /// [`TestClient::apply_mode`], the configuration covered by `trial` before its apply.
    pub fn apply_mode_covered(&self, trial: &Trial, name: &str, w: i32, h: i32) -> Answer {
        let (conf, answer) = self.mode_configuration(name, w, h);
        trial.object.cover(&conf);
        conf.apply();
        answer
    }

    /// A configuration setting `name` to its `w`x`h` mode, not yet applied.
    pub fn mode_configuration(
        &self,
        name: &str,
        w: i32,
        h: i32,
    ) -> (ZwlrOutputConfigurationV1, Answer) {
        let o = &self.side.output_mgmt;
        let manager = o.manager.as_ref().expect("manage_outputs first");
        let head = (o.heads.iter())
            .find(|x| x.name == name)
            .unwrap_or_else(|| panic!("{name} is announced"));
        let mode = (head.modes.iter())
            .find(|m| o.modes.iter().any(|(x, size)| x == *m && *size == (w, h)))
            .unwrap_or_else(|| panic!("{name} lists {w}x{h}"));
        let answer = Answer::default();
        let conf = manager.create_configuration(o.serial, &self.qh(), answer.clone());
        let ch: ZwlrOutputConfigurationHeadV1 = conf.enable_head(&head.head, &self.qh(), ());
        ch.set_mode(mode);
        (conf, answer)
    }

    /// Begin a trial of `seconds` through `reprise_output_trial_manager_v1`.
    pub fn begin_trial(&self, seconds: u32) -> Trial {
        let manager: RepriseOutputTrialManagerV1 = self.bind("reprise_output_trial_manager_v1", 1);
        let heard = Heard::default();
        let object = manager.begin(seconds, &self.qh(), heard.clone());
        Trial { object, heard }
    }
}

impl wc::Dispatch<ZwlrOutputManagerV1, ()> for ClientSide {
    fn event(
        st: &mut Self,
        _: &ZwlrOutputManagerV1,
        ev: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            zwlr_output_manager_v1::Event::Head { head } => st.output_mgmt.heads.push(SeenHead {
                head,
                name: String::new(),
                modes: Vec::new(),
            }),
            zwlr_output_manager_v1::Event::Done { serial } => st.output_mgmt.serial = serial,
            _ => {}
        }
    }

    wc::event_created_child!(ClientSide, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl wc::Dispatch<ZwlrOutputHeadV1, ()> for ClientSide {
    fn event(
        st: &mut Self,
        head: &ZwlrOutputHeadV1,
        ev: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            zwlr_output_head_v1::Event::Name { name } => {
                if let Some(h) = st.output_mgmt.head_mut(head) {
                    h.name = name;
                }
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                st.output_mgmt.modes.push((mode.clone(), (0, 0)));
                if let Some(h) = st.output_mgmt.head_mut(head) {
                    h.modes.push(mode);
                }
            }
            zwlr_output_head_v1::Event::Finished => {
                st.output_mgmt.heads.retain(|h| h.head != *head)
            }
            _ => {}
        }
    }

    wc::event_created_child!(ClientSide, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl wc::Dispatch<ZwlrOutputModeV1, ()> for ClientSide {
    fn event(
        st: &mut Self,
        mode: &ZwlrOutputModeV1,
        ev: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                if let Some((_, size)) = st.output_mgmt.modes.iter_mut().find(|(m, _)| m == mode) {
                    *size = (width, height);
                }
            }
            zwlr_output_mode_v1::Event::Finished => st.output_mgmt.modes.retain(|(m, _)| m != mode),
            _ => {}
        }
    }
}

impl wc::Dispatch<ZwlrOutputConfigurationV1, Answer> for ClientSide {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        ev: zwlr_output_configuration_v1::Event,
        answer: &Answer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        *answer.lock().unwrap() = Some(match ev {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            _ => "cancelled",
        });
    }
}

wc::delegate_noop!(ClientSide: ignore ZwlrOutputConfigurationHeadV1);
wc::delegate_noop!(ClientSide: ignore RepriseOutputTrialManagerV1);

impl wc::Dispatch<RepriseOutputTrialV1, Heard> for ClientSide {
    fn event(
        _: &mut Self,
        _: &RepriseOutputTrialV1,
        ev: reprise_output_trial_v1::Event,
        heard: &Heard,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use reprise_output_trial_v1::{Event, Reason};
        heard.lock().unwrap().push(match ev {
            Event::Started => "started",
            Event::Ended { reason } => match reason {
                wc::WEnum::Value(Reason::Kept) => "kept",
                wc::WEnum::Value(Reason::Reverted) => "reverted",
                wc::WEnum::Value(Reason::Failed) => "failed",
                wc::WEnum::Value(Reason::Busy) => "busy",
                _ => "unknown",
            },
        });
    }
}
