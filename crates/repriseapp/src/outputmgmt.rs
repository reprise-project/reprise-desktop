//! wlr-output-management and `reprise_output_trial_v1` on the wire, for a program that sets the
//! outputs.

use crate::heads::{self, Configure, In, ObjId, Out, TrialAsk, TrialEnd};
use crate::outputtrial::reprise_output_trial_manager_v1::{self, RepriseOutputTrialManagerV1};
use crate::outputtrial::reprise_output_trial_v1::{self, Reason, RepriseOutputTrialV1};
use crate::{Event, Host, MAIN};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use std::collections::HashMap;
use std::sync::Arc;
use wayland_client::backend::ObjectData;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_output;
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};

pub(crate) struct Outputs {
    manager: ZwlrOutputManagerV1,
    heads: HashMap<ObjId, ZwlrOutputHeadV1>,
    modes: HashMap<ObjId, ZwlrOutputModeV1>,
    configs: HashMap<u32, ZwlrOutputConfigurationV1>,
    model: heads::Model,
    /// None when the compositor offers no trials.
    trials: Option<RepriseOutputTrialManagerV1>,
    /// The trial last begun, until it ends.
    trial: Option<RepriseOutputTrialV1>,
}

pub(crate) fn bind(globals: &GlobalList, qh: &QueueHandle<Host>) -> Option<Outputs> {
    let manager = globals
        .bind::<ZwlrOutputManagerV1, _, _>(qh, 1..=4, ManagerData)
        .ok()?;
    let model = heads::Model::new(manager.version());
    let trials = globals
        .bind::<RepriseOutputTrialManagerV1, _, _>(qh, 1..=1, TrialManagerData)
        .ok();
    Some(Outputs {
        manager,
        heads: HashMap::new(),
        modes: HashMap::new(),
        configs: HashMap::new(),
        model,
        trials,
        trial: None,
    })
}

fn id(p: &impl Proxy) -> ObjId {
    p.id().protocol_id()
}

impl Outputs {
    /// Begin a trial, in place of one still running; false when the compositor offers none.
    fn begin_trial(&mut self, serial: u32, seconds: u32, qh: &QueueHandle<Host>) -> bool {
        let Some(m) = &self.trials else {
            return false;
        };
        if let Some(old) = self.trial.take() {
            old.destroy();
        }
        self.trial = Some(m.begin(seconds, qh, TrialData(serial)));
        true
    }

    /// Create, fill and send `c`; false, sending nothing, when it is to be covered and no trial
    /// is held to cover it.
    fn send(&mut self, c: Configure, qh: &QueueHandle<Host>) -> bool {
        let trial = match (c.cover, &self.trial) {
            (false, _) => None,
            (true, Some(t)) => Some(t),
            (true, None) => return false,
        };
        let conf = self
            .manager
            .create_configuration(c.serial, qh, ConfigData(c.id));
        if let Some(t) = trial {
            t.cover(&conf);
        }
        for (h, part) in &c.heads {
            let Some(head) = self.heads.get(h) else {
                continue;
            };
            let Some(part) = part else {
                conf.disable_head(head);
                continue;
            };
            let ch = conf.enable_head(head, qh, ConfigHeadData);
            if let Some(m) = part.mode.and_then(|m| self.modes.get(&m)) {
                ch.set_mode(m);
            }
            if let Some(t) = part
                .transform
                .and_then(|t| wl_output::Transform::try_from(t.wl()).ok())
            {
                ch.set_transform(t);
            }
            if let Some(s) = part.scale {
                ch.set_scale(s);
            }
            if let Some((x, y)) = part.position {
                ch.set_position(x, y);
            }
            if let Some(on) = part.adaptive_sync
                && ch.version() >= 4
            {
                ch.set_adaptive_sync(if on {
                    AdaptiveSyncState::Enabled
                } else {
                    AdaptiveSyncState::Disabled
                });
            }
        }
        if c.test {
            conf.test();
        } else {
            conf.apply();
        }
        self.configs.insert(c.id, conf);
        true
    }
}

impl Host {
    fn heads_in(&mut self, ev: In) {
        let Some(o) = self.outputs.as_mut() else {
            return;
        };
        let outs = o.model.event(ev);
        self.heads_out(outs);
    }

    fn heads_out(&mut self, outs: Vec<Out>) {
        for out in outs {
            match out {
                Out::Heads(h) => self.send(MAIN, Event::Heads(h)),
                Out::Applied { serial, ok } => self.send(MAIN, Event::HeadsApplied { serial, ok }),
                Out::Configure(c) => {
                    let qh = self.qh.clone();
                    let id = c.id;
                    if self.outputs.as_mut().is_some_and(|o| !o.send(c, &qh)) {
                        self.heads_in(In::Failed(id));
                    }
                }
                Out::BeginTrial { serial, seconds } => {
                    let qh = self.qh.clone();
                    let begun = (self.outputs.as_mut())
                        .is_some_and(|o| o.begin_trial(serial, seconds, &qh));
                    if !begun {
                        self.heads_in(In::TrialRefused);
                    }
                }
                Out::TrialEnded(end) => self.send(MAIN, Event::TrialEnded(end)),
            }
        }
    }

    /// Send what the program asked during the event: its keep or revert at once, its
    /// configurations through the model.
    pub(crate) fn drain_head_requests(&mut self) {
        let trial = self.ctx.heads.take_trial();
        let asked = self.ctx.heads.take();
        if asked.is_empty() && trial.is_empty() {
            return;
        }
        let outs: Vec<Out> = match self.outputs.as_mut() {
            Some(o) => {
                let mut on_trial = Vec::new();
                for ask in trial {
                    match (ask, &o.trial) {
                        (TrialAsk::Begin { serial, seconds }, _) => {
                            on_trial.push((serial, seconds))
                        }
                        (TrialAsk::Keep, Some(t)) => t.keep(),
                        (TrialAsk::Revert, Some(t)) => t.revert(),
                        (_, None) => {}
                    }
                }
                (asked.into_iter())
                    .flat_map(
                        |(serial, heads)| match on_trial.iter().find(|(s, _)| *s == serial) {
                            Some(&(_, seconds)) => {
                                o.model.configure_on_trial(serial, heads, seconds)
                            }
                            None => o.model.configure_all(serial, heads),
                        },
                    )
                    .collect()
            }
            None => asked
                .into_iter()
                .map(|(serial, ..)| Out::Applied { serial, ok: false })
                .collect(),
        };
        self.loop_handle
            .insert_idle(move |host| host.heads_out(outs));
    }

    /// An ending signal's [`Extended::emergency_outputs`](crate::Extended::emergency_outputs): its
    /// configurations go out now, and the compositor has read them before this returns. One that
    /// waits behind a configuration in flight is lost.
    pub(crate) fn send_last_outputs(&mut self, conn: &Connection) {
        let qh = self.qh.clone();
        let Some(o) = self.outputs.as_mut() else {
            return;
        };
        self.app.emergency_outputs(&mut self.ctx.heads);
        let mut sent = false;
        for (serial, heads) in self.ctx.heads.take() {
            for out in o.model.configure_all(serial, heads) {
                if let Out::Configure(c) = out {
                    o.send(c, &qh);
                    sent = true;
                }
            }
        }
        if sent {
            crate::hangup::read_through(conn);
        }
    }
}

struct ManagerData;
struct HeadData;
struct ModeData;
struct ConfigData(u32);
struct ConfigHeadData;
struct TrialManagerData;
/// The configuration serial the trial was begun for.
struct TrialData(u32);

impl Dispatch2<ZwlrOutputManagerV1, Host> for ManagerData {
    fn event(
        &self,
        host: &mut Host,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                let h = id(&head);
                if let Some(o) = host.outputs.as_mut() {
                    o.heads.insert(h, head);
                }
                host.heads_in(In::Head(h));
            }
            zwlr_output_manager_v1::Event::Done { serial } => host.heads_in(In::Done(serial)),
            // Gone: fail what was asked and tell the program there are
            // no heads.
            zwlr_output_manager_v1::Event::Finished => {
                if let Some(o) = host.outputs.take() {
                    o.configs.values().for_each(|c| c.destroy());
                    o.trial.iter().for_each(|t| t.destroy());
                    o.trials.iter().for_each(|m| m.destroy());
                    let mut model = o.model;
                    host.heads_out(model.finish());
                }
            }
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<Host>) -> Arc<dyn ObjectData> {
        match opcode {
            zwlr_output_manager_v1::EVT_HEAD_OPCODE => {
                qh.make_data::<ZwlrOutputHeadV1, _>(HeadData)
            }
            _ => panic!("zwlr_output_manager_v1 event {opcode} makes no object"),
        }
    }
}

impl Dispatch2<ZwlrOutputHeadV1, Host> for HeadData {
    fn event(
        &self,
        host: &mut Host,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwlr_output_head_v1::Event as E;
        let h = id(head);
        let ev = match event {
            E::Name { name } => In::Name(h, name),
            E::Description { description } => In::Description(h, description),
            E::Make { make } => In::Make(h, make),
            E::Model { model } => In::Model(h, model),
            E::SerialNumber { serial_number } => In::SerialNumber(h, serial_number),
            E::Mode { mode } => {
                let m = id(&mode);
                if let Some(o) = host.outputs.as_mut() {
                    o.modes.insert(m, mode);
                }
                In::Mode(h, m)
            }
            E::Enabled { enabled } => In::Enabled(h, enabled != 0),
            E::CurrentMode { mode } => In::CurrentMode(h, id(&mode)),
            E::Position { x, y } => In::Position(h, x, y),
            E::Transform { transform } => In::Transform(
                h,
                match transform {
                    WEnum::Value(t) => t.into(),
                    WEnum::Unknown(v) => v,
                },
            ),
            E::Scale { scale } => In::Scale(h, scale),
            E::AdaptiveSync { state } => {
                In::AdaptiveSync(h, matches!(state, WEnum::Value(AdaptiveSyncState::Enabled)))
            }
            E::Finished => {
                if let Some(o) = host.outputs.as_mut()
                    && let Some(p) = o.heads.remove(&h)
                    && p.version() >= 3
                {
                    p.release();
                }
                In::HeadFinished(h)
            }
            _ => return,
        };
        host.heads_in(ev);
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<Host>) -> Arc<dyn ObjectData> {
        match opcode {
            zwlr_output_head_v1::EVT_MODE_OPCODE => qh.make_data::<ZwlrOutputModeV1, _>(ModeData),
            _ => panic!("zwlr_output_head_v1 event {opcode} makes no object"),
        }
    }
}

impl Dispatch2<ZwlrOutputModeV1, Host> for ModeData {
    fn event(
        &self,
        host: &mut Host,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwlr_output_mode_v1::Event as E;
        let m = id(mode);
        let ev = match event {
            E::Size { width, height } => In::ModeSize(m, width, height),
            E::Refresh { refresh } => In::ModeRefresh(m, refresh),
            E::Preferred => In::ModePreferred(m),
            E::Finished => {
                if let Some(o) = host.outputs.as_mut()
                    && let Some(p) = o.modes.remove(&m)
                    && p.version() >= 3
                {
                    p.release();
                }
                In::ModeFinished(m)
            }
            _ => return,
        };
        host.heads_in(ev);
    }
}

impl Dispatch2<ZwlrOutputConfigurationV1, Host> for ConfigData {
    fn event(
        &self,
        host: &mut Host,
        conf: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwlr_output_configuration_v1::Event as E;
        let ev = match event {
            E::Succeeded => In::Succeeded(self.0),
            E::Failed => In::Failed(self.0),
            E::Cancelled => In::Cancelled(self.0),
            _ => return,
        };
        conf.destroy();
        if let Some(o) = host.outputs.as_mut() {
            o.configs.remove(&self.0);
        }
        host.heads_in(ev);
    }
}

impl Dispatch2<ZwlrOutputConfigurationHeadV1, Host> for ConfigHeadData {
    fn event(
        &self,
        _: &mut Host,
        _: &ZwlrOutputConfigurationHeadV1,
        _: zwlr_output_configuration_head_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

impl Dispatch2<RepriseOutputTrialManagerV1, Host> for TrialManagerData {
    fn event(
        &self,
        _: &mut Host,
        _: &RepriseOutputTrialManagerV1,
        _: reprise_output_trial_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
    }
}

impl Dispatch2<RepriseOutputTrialV1, Host> for TrialData {
    fn event(
        &self,
        host: &mut Host,
        trial: &RepriseOutputTrialV1,
        event: reprise_output_trial_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        let ev = match event {
            reprise_output_trial_v1::Event::Started => In::TrialStarted,
            reprise_output_trial_v1::Event::Ended { reason } => {
                trial.destroy();
                if let Some(o) = host.outputs.as_mut() {
                    o.trial.take_if(|t| t == trial);
                }
                let ended = |end| In::TrialEnded {
                    serial: self.0,
                    end,
                };
                match reason {
                    WEnum::Value(Reason::Busy) => In::TrialRefused,
                    WEnum::Value(Reason::Kept) => ended(TrialEnd::Kept),
                    WEnum::Value(Reason::Reverted) => ended(TrialEnd::Reverted),
                    _ => ended(TrialEnd::Failed),
                }
            }
        };
        host.heads_in(ev);
    }
}
