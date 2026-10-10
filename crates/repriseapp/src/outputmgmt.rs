//! wlr-output-management on the wire, for a program that sets the outputs.

use crate::heads::{self, Configure, In, ObjId, Out};
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
}

pub(crate) fn bind(globals: &GlobalList, qh: &QueueHandle<Host>) -> Option<Outputs> {
    let manager = globals
        .bind::<ZwlrOutputManagerV1, _, _>(qh, 1..=4, ManagerData)
        .ok()?;
    let model = heads::Model::new(manager.version());
    Some(Outputs {
        manager,
        heads: HashMap::new(),
        modes: HashMap::new(),
        configs: HashMap::new(),
        model,
    })
}

fn id(p: &impl Proxy) -> ObjId {
    p.id().protocol_id()
}

impl Outputs {
    fn send(&mut self, c: Configure, qh: &QueueHandle<Host>) {
        let conf = self
            .manager
            .create_configuration(c.serial, qh, ConfigData(c.id));
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
                    if let Some(o) = self.outputs.as_mut() {
                        o.send(c, &qh);
                    }
                }
            }
        }
    }

    pub(crate) fn drain_head_requests(&mut self) {
        let asked = self.ctx.heads.take();
        if asked.is_empty() {
            return;
        }
        let outs: Vec<Out> = match self.outputs.as_mut() {
            Some(o) => asked
                .into_iter()
                .flat_map(|(serial, heads)| o.model.configure_all(serial, heads))
                .collect(),
            None => asked
                .into_iter()
                .map(|(serial, ..)| Out::Applied { serial, ok: false })
                .collect(),
        };
        self.loop_handle
            .insert_idle(move |host| host.heads_out(outs));
    }
}

struct ManagerData;
struct HeadData;
struct ModeData;
struct ConfigData(u32);
struct ConfigHeadData;

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
