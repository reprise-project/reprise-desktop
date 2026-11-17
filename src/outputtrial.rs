//! `reprise_output_trial_v1`: a display change on trial. A client begins a trial before its
//! wlr-output-management apply; the compositor notes every head as it runs then and puts the heads
//! back when the trial ends without `keep`: on `revert`, when its seconds run out, when the object
//! is destroyed, or when its client goes for any reason. One trial is held at a time; a `begin`
//! meanwhile ends `busy` and notes nothing.
//!
//! A client `cover`s its configuration with the trial before applying it. `outputmgmt::answer`, the
//! one place a configuration is applied, applies a covered one only while its trial is held,
//! checked in the same turn it applies; else it is cancelled. So a trial's configuration that
//! reaches the compositor after the trial ended (its client stalled, or it was sent again after a
//! cancel) changes nothing and is saved nowhere.
//!
//! While a trial is held or its put back waits, no layout is saved (`State::save_layout`), from any
//! path; applies from any client go through, and the noted heads stay what is put back. `keep`
//! saves the layout, and so does a put back that went through, so displays.toml ends on the kept or
//! the restored outputs; an `[[outputs]]` pin refused on trial and still refused at its end drops
//! the set's layout in place of that save, as it would have with no trial. A put back runs as an
//! apply does (`outputmgmt::resolve`, `apply_unsaved`), leaving out a head gone since or one that
//! no longer lists its noted mode (a changed monitor); adaptive sync goes back as the policy it
//! was, unless the head lost it. It waits while the session is off its seat or modes settle, then
//! runs before any other configuration; applies waiting behind it, or resolved before it ran, are
//! cancelled. A failed put back is tried once more once modes settle, then the trial ends `failed`.
//! An unplug does not end a trial.

use std::time::Duration;

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, New, Resource, backend::ClientId,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::compositor::State;
use crate::config::VrrPolicy;
use crate::outputmgmt::{HeadAsk, HeadState, ModeAsk};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_v1::ZwlrOutputConfigurationV1;
use proto::reprise_output_trial_manager_v1::{self, RepriseOutputTrialManagerV1};
use proto::reprise_output_trial_v1::{self, Reason, RepriseOutputTrialV1};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all
)]
pub mod proto {
    // The scanner macros open the XML with File::open, which leaves cargo no
    // dependency edge; this keeps an edit to the protocol rebuilding us.
    const _: &[u8] = include_bytes!("../protocols/reprise-output-trial-v1.xml");

    use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::__interfaces::*;
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/reprise-output-trial-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/reprise-output-trial-v1.xml");
}

const VERSION: u32 = 1;

pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, RepriseOutputTrialManagerV1, _>(VERSION, ManagerGlobal);
}

#[derive(Default)]
pub struct Trials {
    held: Option<Held>,
    waiting: Option<PutBack>,
    /// Put backs run so far: an apply resolved before the last one fails (`outputmgmt::answer`).
    pub put_backs: u32,
    /// Configurations not yet applied, each with the trial that covers it.
    covered: Vec<(ZwlrOutputConfigurationV1, RepriseOutputTrialV1)>,
}

struct Held {
    trial: RepriseOutputTrialV1,
    noted: Vec<Noted>,
    timer: Option<RegistrationToken>,
}

/// A head as a trial's begin found it.
struct Noted {
    head: HeadState,
    /// The adaptive sync policy output management set (`State::vrr_policy`); None: the config's.
    vrr: Option<VrrPolicy>,
}

struct PutBack {
    noted: Vec<Noted>,
    /// Told how it ended; None when the trial was destroyed.
    tell: Option<RepriseOutputTrialV1>,
    tries: u8,
}

impl Trials {
    /// Whether a trial is held or its put back waits: applies meanwhile save nothing.
    pub fn on_trial(&self) -> bool {
        self.held.is_some() || self.waiting.is_some()
    }

    pub fn waits(&self) -> bool {
        self.waiting.is_some()
    }

    /// Whether `trial` is the one held.
    pub fn holds(&self, trial: &RepriseOutputTrialV1) -> bool {
        self.held.as_ref().is_some_and(|h| h.trial == *trial)
    }

    /// The trial covering `conf`, which is applied or tested now or destroyed: no longer covered.
    pub fn take_cover(&mut self, conf: &ZwlrOutputConfigurationV1) -> Option<RepriseOutputTrialV1> {
        let i = self.covered.iter().position(|(c, _)| c == conf)?;
        Some(self.covered.swap_remove(i).1)
    }
}

/// `trial` covers `conf`; one applied, tested or covered already is a protocol error.
fn cover(state: &mut State, trial: &RepriseOutputTrialV1, conf: ZwlrOutputConfigurationV1) {
    let g = &mut state.outputs_mgmt;
    if g.trial.covered.iter().any(|(c, _)| *c == conf) || !g.unused(&conf) {
        trial.post_error(
            reprise_output_trial_v1::Error::InvalidConfiguration,
            "the configuration was applied, tested or covered already",
        );
        return;
    }
    g.trial.covered.push((conf, trial.clone()));
}

fn begin(state: &mut State, trial: RepriseOutputTrialV1, seconds: u32) {
    if state.outputs_mgmt.trial.on_trial() {
        trial.ended(Reason::Busy);
        return;
    }
    state.save_side_by_side();
    let noted = (state.outputs_mgmt.heads().iter())
        .map(|h| Noted {
            vrr: state.vrr_policy.get(&h.name).copied(),
            head: h.clone(),
        })
        .collect();
    let out = Timer::from_duration(held_for(seconds));
    let timer = state.handle.insert_source(out, |_, _, state| {
        if let Some(h) = state.outputs_mgmt.trial.held.as_mut() {
            h.timer = None;
        }
        log::info!("output trial: its time ran out, the outputs go back");
        end(state, false, true);
        TimeoutAction::Drop
    });
    let timer = match timer {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("output trial timer: {}", e.error);
            None
        }
    };
    trial.started();
    state.outputs_mgmt.trial.held = Some(Held {
        trial,
        noted,
        timer,
    });
}

/// A trial's bounds in seconds, so no client holds the one trial for hours.
const SHORTEST: u32 = 1;
const LONGEST: u32 = 60;

fn held_for(seconds: u32) -> Duration {
    Duration::from_secs(seconds.clamp(SHORTEST, LONGEST).into())
}

/// End the held trial: kept, or its heads put back; `tell` sends `ended`.
fn end(state: &mut State, keep: bool, tell: bool) {
    let Some(h) = state.outputs_mgmt.trial.held.take() else {
        return;
    };
    if let Some(t) = h.timer {
        state.handle.remove(t);
    }
    if keep {
        h.trial.ended(Reason::Kept);
        let refused = state.pin_refused();
        state
            .handle
            .insert_idle(move |state| state.save_layout_after_trial(refused));
        return;
    }
    let p = PutBack {
        noted: h.noted,
        tell: tell.then_some(h.trial),
        tries: 2,
    };
    put_back(state, p);
}

fn put_back(state: &mut State, mut p: PutBack) {
    if !crate::backend::session_active(state) || !crate::backend::modes_settle(state) {
        log::info!("output trial: the outputs go back once the display is ours and settled");
        state.outputs_mgmt.trial.waiting = Some(p);
        return;
    }
    state.outputs_mgmt.trial.put_backs = state.outputs_mgmt.trial.put_backs.wrapping_add(1);
    p.tries -= 1;
    // Before the put back: its apply clears the refused pins of the heads it sets.
    let refused = state.pin_refused();
    let ok = put_back_once(state, &p.noted);
    if !ok && p.tries > 0 {
        log::warn!("output trial: the outputs did not go back, trying once more");
        return put_back(state, p);
    }
    if ok {
        state
            .handle
            .insert_idle(move |state| state.save_layout_after_trial(refused));
    } else {
        log::warn!("output trial: the outputs could not be put back");
    }
    if let Some(t) = p.tell.filter(Resource::is_alive) {
        t.ended(if ok { Reason::Reverted } else { Reason::Failed });
    }
}

fn put_back_once(state: &mut State, noted: &[Noted]) -> bool {
    let heads = state.outputs_mgmt.heads();
    let asks = asks(noted, heads);
    if asks.is_empty() {
        return true;
    }
    let Some(mut reqs) = crate::outputmgmt::resolve(heads, &asks) else {
        return false;
    };
    let mut config_vrr = Vec::new();
    for (name, req) in &mut reqs {
        let (Some(n), Some(h)) = (
            noted.iter().find(|n| n.head.name == *name),
            heads.iter().find(|h| h.name == *name),
        ) else {
            continue;
        };
        match vrr_back(n.vrr, state.vrr_policy.get(name).copied(), h.vrr_capable) {
            VrrBack::Leave => {}
            VrrBack::Set(p) => req.adaptive_sync = Some(p),
            VrrBack::Config => config_vrr.push(name.clone()),
        }
    }
    if !config_vrr.is_empty() {
        for name in &config_vrr {
            state.vrr_policy.remove(name);
        }
        state.sync_vrr();
    }
    crate::outputmgmt::apply_unsaved(state, reqs)
}

#[derive(Debug, PartialEq)]
enum VrrBack {
    Leave,
    Set(VrrPolicy),
    /// Drop output management's policy: the config's decides again.
    Config,
}

/// What puts a head's adaptive sync policy back from `now` to `noted`. A head that lost adaptive
/// sync keeps its policy rather than refuse the rest of the put back.
fn vrr_back(noted: Option<VrrPolicy>, now: Option<VrrPolicy>, capable: bool) -> VrrBack {
    match noted {
        _ if noted == now => VrrBack::Leave,
        Some(VrrPolicy::On) if !capable => VrrBack::Leave,
        Some(p) => VrrBack::Set(p),
        None => VrrBack::Config,
    }
}

/// Run the put back that waited for the seat or for modes to settle. Whether one waited.
pub fn put_back_waiting(state: &mut State) -> bool {
    let Some(p) = state.outputs_mgmt.trial.waiting.take() else {
        return false;
    };
    put_back(state, p);
    true
}

/// What puts the `noted` heads still among `heads` back as they ran, all but adaptive sync
/// (`vrr_back`). A head that no longer lists its noted mode is another monitor: left out.
fn asks(noted: &[Noted], heads: &[HeadState]) -> Vec<(String, HeadAsk)> {
    (noted.iter().map(|n| &n.head))
        .filter_map(|n| {
            let h = heads.iter().find(|h| h.name == n.name)?;
            let mode = match n.current.and_then(|i| n.modes.get(i)) {
                _ if !n.enabled => ModeAsk::Keep,
                Some(_) if h.modes == n.modes => ModeAsk::Index(n.current?),
                Some(m) => {
                    let i =
                        crate::outputmgmt::custom_mode(&h.modes, m.width, m.height, m.refresh_mhz);
                    if i.is_none() {
                        log::info!("output trial: {} lists other modes now, it stays", n.name);
                    }
                    ModeAsk::Index(i?)
                }
                None => ModeAsk::Keep,
            };
            let ask = HeadAsk {
                enabled: n.enabled,
                mode,
                position: n.enabled.then_some(n.position),
                transform: n.transform,
                scale: n.scale,
                adaptive_sync: h.adaptive_sync,
            };
            Some((n.name.clone(), ask))
        })
        .collect()
}

fn held_is(state: &State, trial: &RepriseOutputTrialV1) -> bool {
    state.outputs_mgmt.trial.holds(trial)
}

pub struct ManagerGlobal;
pub struct ManagerData;
pub struct TrialData;

impl GlobalDispatch2<RepriseOutputTrialManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<RepriseOutputTrialManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "reprise_output_trial_manager_v1")
    }
}

impl Dispatch2<RepriseOutputTrialManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &RepriseOutputTrialManagerV1,
        request: reprise_output_trial_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            reprise_output_trial_manager_v1::Request::Begin { id, seconds } => {
                let trial = data_init.init(id, TrialData);
                begin(state, trial, seconds);
            }
            reprise_output_trial_manager_v1::Request::Destroy => {}
        }
    }
}

impl Dispatch2<RepriseOutputTrialV1, State> for TrialData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        trial: &RepriseOutputTrialV1,
        request: reprise_output_trial_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let reprise_output_trial_v1::Request::Cover { configuration } = request {
            return cover(state, trial, configuration);
        }
        if !held_is(state, trial) {
            return;
        }
        match request {
            reprise_output_trial_v1::Request::Keep => end(state, true, true),
            reprise_output_trial_v1::Request::Revert => end(state, false, true),
            _ => {}
        }
    }

    /// A destroy or the client's hang-up, which also destroys each of its objects.
    fn destroyed(&self, state: &mut State, _client: ClientId, trial: &RepriseOutputTrialV1) {
        if held_is(state, trial) {
            end(state, false, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use smithay::output::{Mode, Output};

    use crate::harness::{Harness, TestClient};

    /// Heads DP-1 and DP-2 at 1920x1080 that also list 1280x720 and take mode changes, and a
    /// client that has heard them.
    fn two_heads() -> (Harness, Output, Output, TestClient) {
        let mut h = Harness::new();
        let heads = ["DP-1", "DP-2"].map(|name| {
            let o = h.head(name, 1920, 1080);
            o.add_mode(Mode {
                size: (1280, 720).into(),
                refresh: 60_000,
            });
            o
        });
        h.state.test_modes = crate::backend::fake::Modes::Taken;
        crate::outputmgmt::publish(&mut h.state);
        let mut c = h.connect();
        c.manage_outputs();
        h.roundtrip(&mut c);
        let [a, b] = heads;
        (h, a, b, c)
    }

    fn width(o: &Output) -> i32 {
        o.current_mode().expect("a mode").size.w
    }

    fn answered(a: &crate::harness::outputs::Answer) -> Option<&'static str> {
        *a.lock().unwrap()
    }

    /// displays.toml once the saves queued so far have landed.
    fn saved(h: &mut Harness) -> String {
        h.dispatch();
        crate::state::flush();
        std::fs::read_to_string(&h.state.saved_layouts.path).unwrap_or_default()
    }

    /// The mode displays.toml saves for `name` among the connected set under `[[outputs]]` now.
    fn saved_mode(h: &mut Harness, name: &str) -> Option<(i32, i32)> {
        let text = saved(h);
        let heads = crate::backend::heads(&h.state);
        let outputs: Vec<&Output> = heads.iter().map(|x| &x.output).collect();
        let ids = crate::outputlayouts::monitor_ids(&outputs);
        let now = repriseconf::displays::outputs_fingerprint(&h.state.config.outputs);
        let layouts = repriseconf::displays::Layouts::parse(&text);
        let i = heads.iter().position(|x| x.output.name() == name)?;
        let (_, m) = (layouts.find(&ids, &now)?.monitors.iter()).find(|(id, _)| *id == ids[i])?;
        m.mode.map(|m| (m.width, m.height))
    }

    /// `c` begins a 15 s trial and applies DP-1 at 1280x720, covered by it.
    fn on_trial(h: &mut Harness, c: &mut TestClient, a: &Output) -> crate::harness::outputs::Trial {
        let t = c.begin_trial(15);
        h.roundtrip(c);
        assert_eq!(t.heard(), ["started"]);
        let applied = c.apply_mode_covered(&t, "DP-1", 1280, 720);
        h.roundtrip(c);
        assert_eq!(answered(&applied), Some("succeeded"));
        assert_eq!(width(a), 1280);
        t
    }

    #[test]
    fn a_kept_trial_stays_applied_and_is_saved() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        assert!(
            !saved(&mut h).contains("1280x720"),
            "nothing saved on trial"
        );
        t.object.keep();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "kept"]);
        assert_eq!(width(&a), 1280);
        assert!(
            saved(&mut h).contains("1280x720"),
            "the kept mode is saved as an apply's is"
        );
    }

    #[test]
    fn a_reverted_trial_puts_the_mode_back_and_saves_nothing() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!(width(&a), 1920);
        let saved = saved(&mut h);
        assert!(
            saved.contains("1920x1080") && !saved.contains("1280x720"),
            "the arrangement from before the trial is all that is remembered: {saved}"
        );
    }

    #[test]
    fn a_trial_whose_seconds_run_out_puts_the_mode_back() {
        let (mut h, a, _, mut c) = two_heads();
        let t = c.begin_trial(1);
        h.roundtrip(&mut c);
        c.apply_mode("DP-1", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(width(&a), 1280);
        h.run_for(500);
        assert_eq!(width(&a), 1280, "held for its second");
        h.until("the trial runs out", |_| width(&a) == 1920);
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
    }

    #[test]
    fn a_trial_destroyed_without_keep_puts_the_mode_back() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        t.object.destroy();
        h.roundtrip(&mut c);
        assert_eq!(width(&a), 1920);
        assert_eq!(t.heard(), ["started"], "no one is told");
    }

    #[test]
    fn a_client_that_goes_mid_trial_has_the_mode_put_back() {
        let (mut h, a, _, mut c) = two_heads();
        let _t = on_trial(&mut h, &mut c, &a);
        c.hang_up();
        h.dispatch();
        assert_eq!(width(&a), 1920, "as when it is killed");
    }

    #[test]
    fn a_second_trial_is_refused_busy_and_changes_nothing() {
        let (mut h, a, _, mut c) = two_heads();
        let first = on_trial(&mut h, &mut c, &a);
        let mut other = h.connect();
        let second = other.begin_trial(15);
        h.roundtrip(&mut other);
        assert_eq!(second.heard(), ["busy"]);
        second.object.revert();
        second.object.destroy();
        h.roundtrip(&mut other);
        assert_eq!(width(&a), 1280, "the held trial is left alone");
        let again = c.begin_trial(15);
        h.roundtrip(&mut c);
        assert_eq!(again.heard(), ["busy"], "from its own client too");
        first.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(first.heard(), ["started", "reverted"]);
        assert_eq!(width(&a), 1920);
    }

    #[test]
    fn an_unplug_keeps_the_trial_and_its_revert_puts_back_the_heads_left() {
        let (mut h, a, b, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        c.apply_mode("DP-2", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(width(&b), 1280);
        h.state.test_heads.retain(|o| *o != a);
        h.state.scene.remove_output(&a);
        h.state.output_removed(&a);
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started"], "the trial runs on");
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!(width(&b), 1920, "the head still there goes back");
    }

    #[test]
    fn a_revert_while_the_session_is_away_runs_when_it_is_back() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        crate::backend::pause_session(&mut h.state);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(width(&a), 1280, "nothing set while away");
        assert_eq!(t.heard(), ["started"]);
        let mut other = h.connect();
        other.manage_outputs();
        h.roundtrip(&mut other);
        let meanwhile = other.apply_mode("DP-2", 1280, 720);
        h.roundtrip(&mut other);
        assert_eq!(
            answered(&meanwhile),
            Some("cancelled"),
            "the put back goes first"
        );
        let next = other.begin_trial(15);
        h.roundtrip(&mut other);
        assert_eq!(next.heard(), ["busy"], "while the put back waits");
        crate::backend::activate_session(&mut h.state);
        h.roundtrip(&mut c);
        assert_eq!(width(&a), 1920);
        assert_eq!(t.heard(), ["started", "reverted"]);
    }

    #[test]
    fn a_put_back_the_heads_refuse_is_tried_again_then_fails() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        h.state.test_modes = crate::backend::fake::Modes::Refused;
        let before = h.state.outputs_mgmt.trial.put_backs;
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "failed"]);
        assert_eq!(width(&a), 1280);
        assert_eq!(
            h.state.outputs_mgmt.trial.put_backs - before,
            2,
            "tried twice"
        );
        let next = c.begin_trial(15);
        h.roundtrip(&mut c);
        assert_eq!(next.heard(), ["started"], "a failed trial holds nothing");
    }

    #[test]
    fn a_put_back_the_busy_display_cuts_short_is_finished_once_modes_settle() {
        let (mut h, a, b, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        c.apply_mode("DP-2", 1280, 720);
        h.roundtrip(&mut c);
        h.state.test_modes = crate::backend::fake::Modes::TakenOnce;
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(
            (width(&a), width(&b)),
            (1920, 1280),
            "DP-1's modeset holds DP-2's"
        );
        assert_eq!(t.heard(), ["started"], "the second try waits");
        crate::backend::fake::idle(&mut h.state);
        h.roundtrip(&mut c);
        assert_eq!((width(&a), width(&b)), (1920, 1920));
        assert_eq!(t.heard(), ["started", "reverted"]);
    }

    #[test]
    fn another_clients_apply_on_trial_is_taken_and_put_back_with_the_trial() {
        let (mut h, a, b, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        let mut randr = h.connect();
        randr.manage_outputs();
        h.roundtrip(&mut randr);
        let before = saved(&mut h);
        let applied = randr.apply_mode("DP-2", 1280, 720);
        h.roundtrip(&mut randr);
        assert_eq!(answered(&applied), Some("succeeded"));
        assert_eq!(width(&b), 1280);
        assert_eq!(saved(&mut h), before, "nothing saved on trial");
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!((width(&a), width(&b)), (1920, 1920));
    }

    #[test]
    fn a_save_from_another_path_on_trial_writes_nothing_and_the_revert_saves_the_old_layout() {
        let reload = |h: &mut Harness| {
            let old = h.state.config.clone();
            h.state.config.outputs.push(crate::config::OutputConfig {
                name: "DP-2".into(),
                scale: 1.0,
                mode: None,
                adaptive_sync: None,
                transform: Default::default(),
                position: None,
                primary: false,
                icc_profile: None,
            });
            h.state.apply_output_config(&old);
        };
        let switcher = |h: &mut Harness| {
            h.state
                .project(repriseconf::projection::Projection::Duplicate)
        };
        let primary = |h: &mut Harness| assert!(h.state.set_primary("DP-2"));
        let paths = [
            ("an [[outputs]] reload", reload as fn(&mut Harness)),
            ("the display switcher", switcher),
            ("set_primary", primary),
        ];
        for (path, run) in paths {
            let (mut h, a, _, mut c) = two_heads();
            let t = on_trial(&mut h, &mut c, &a);
            let before = saved(&mut h);
            run(&mut h);
            assert_eq!(saved(&mut h), before, "{path} writes nothing on trial");
            t.object.revert();
            h.roundtrip(&mut c);
            assert_eq!(t.heard(), ["started", "reverted"], "{path}");
            assert_eq!(
                saved_mode(&mut h, "DP-1"),
                Some((1920, 1080)),
                "{path}: the layout from before the trial is saved"
            );
        }
    }

    #[test]
    fn a_head_whose_modes_changed_on_trial_is_left_out_and_the_others_go_back() {
        let (mut h, a, b, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        c.apply_mode("DP-2", 1280, 720);
        h.roundtrip(&mut c);
        a.delete_mode(Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        });
        a.add_mode(Mode {
            size: (2560, 1440).into(),
            refresh: 60_000,
        });
        crate::outputmgmt::publish(&mut h.state);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!((width(&a), width(&b)), (1280, 1920));
    }

    #[test]
    fn a_put_back_takes_the_listed_mode_it_noted() {
        let mut h = Harness::new();
        let a = h.head("DP-1", 1920, 1080);
        for (w, ht, refresh) in [(1920, 1080, 60_001), (1280, 720, 60_000)] {
            a.add_mode(Mode {
                size: (w, ht).into(),
                refresh,
            });
        }
        h.state.test_modes = crate::backend::fake::Modes::Taken;
        crate::outputmgmt::publish(&mut h.state);
        let mut c = h.connect();
        c.manage_outputs();
        h.roundtrip(&mut c);
        let t = on_trial(&mut h, &mut c, &a);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(a.current_mode().map(|m| m.refresh), Some(60_000));
    }

    #[test]
    fn a_put_back_gives_back_the_adaptive_sync_policy_it_noted() {
        use crate::config::VrrPolicy::{Fullscreen, Off, On};
        // The policy output management set before the trial (None: the config's), and the one
        // set on trial (None: left alone). VRR runs at the begin and stops on trial each time.
        let cases = [
            (Some(Fullscreen), None),
            (None, Some(On)),
            (Some(On), Some(Off)),
        ];
        for (noted, set) in cases {
            let (mut h, a, _, mut c) = two_heads();
            h.state.test_vrr.capable = true;
            h.state
                .vrr_policy
                .extend(noted.map(|p| ("DP-1".to_string(), p)));
            h.state.test_vrr.on = vec!["DP-1".into()];
            crate::outputmgmt::publish(&mut h.state);
            let t = on_trial(&mut h, &mut c, &a);
            h.state.test_vrr.on.clear();
            h.state
                .vrr_policy
                .extend(set.map(|p| ("DP-1".to_string(), p)));
            crate::outputmgmt::publish(&mut h.state);
            t.object.revert();
            h.roundtrip(&mut c);
            assert_eq!(t.heard(), ["started", "reverted"], "{noted:?}");
            assert_eq!(
                h.state.vrr_policy.get("DP-1").copied(),
                noted,
                "{noted:?} {set:?}"
            );
            assert_eq!(width(&a), 1920);
        }
    }

    #[test]
    fn a_head_that_lost_adaptive_sync_on_trial_has_the_rest_put_back() {
        let (mut h, a, _, mut c) = two_heads();
        h.state.test_vrr.capable = true;
        h.state
            .vrr_policy
            .insert("DP-1".into(), crate::config::VrrPolicy::On);
        h.state.sync_vrr();
        let t = on_trial(&mut h, &mut c, &a);
        h.state
            .vrr_policy
            .insert("DP-1".into(), crate::config::VrrPolicy::Off);
        h.state.test_vrr.capable = false;
        h.state.sync_vrr();
        crate::outputmgmt::publish(&mut h.state);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!(width(&a), 1920);
    }

    #[test]
    fn a_covered_apply_after_its_trial_ended_is_cancelled_and_changes_nothing() {
        let (mut h, a, _, mut c) = two_heads();
        let t = c.begin_trial(1);
        h.roundtrip(&mut c);
        h.run_for(1200);
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        let before = saved(&mut h);
        let late = c.apply_mode_covered(&t, "DP-1", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(answered(&late), Some("cancelled"));
        assert_eq!(width(&a), 1920);
        assert_eq!(saved(&mut h), before, "nothing saved");
        let plain = c.apply_mode("DP-1", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(answered(&plain), Some("succeeded"), "an uncovered apply");
        assert_eq!(width(&a), 1280);
    }

    #[test]
    fn a_covered_apply_held_for_modes_is_cancelled_when_its_trial_runs_out_meanwhile() {
        let (mut h, a, _, mut c) = two_heads();
        let t = c.begin_trial(1);
        h.roundtrip(&mut c);
        h.state.test_modes = crate::backend::fake::Modes::Busy;
        let held = c.apply_mode_covered(&t, "DP-1", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(answered(&held), None, "held while modes do not settle");
        h.run_for(1200);
        crate::backend::fake::idle(&mut h.state);
        h.roundtrip(&mut c);
        assert_eq!(answered(&held), Some("cancelled"));
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!(width(&a), 1920);
        assert!(!saved(&mut h).contains("1280x720"), "nothing saved");
    }

    /// The interface and code of the protocol error `c` caused, once the compositor read it.
    fn error_of(
        h: &mut Harness,
        c: &mut TestClient,
        on: &impl wayland_client::Proxy,
    ) -> Option<(String, u32)> {
        c.queue.flush().expect("client flush");
        h.dispatch();
        if let Some(guard) = c.queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = c.queue.dispatch_pending(&mut c.side);
        let backend = on.backend().upgrade()?;
        let error = wayland_client::Connection::from_backend(backend).protocol_error()?;
        Some((error.object_interface, error.code))
    }

    #[test]
    fn covering_a_configuration_applied_or_covered_already_is_a_protocol_error() {
        // A configuration of another client cannot be named: object ids are per client.
        for applied in [true, false] {
            let (mut h, _, _, mut c) = two_heads();
            let t = c.begin_trial(15);
            h.roundtrip(&mut c);
            let (conf, _) = c.mode_configuration("DP-1", 1280, 720);
            if applied {
                conf.apply();
            } else {
                t.object.cover(&conf);
            }
            t.object.cover(&conf);
            assert_eq!(
                error_of(&mut h, &mut c, &t.object),
                Some(("reprise_output_trial_v1".to_string(), 0)),
                "applied first: {applied}"
            );
        }
    }

    #[test]
    fn a_pin_refused_on_trial_is_written_when_the_trial_ends_in_place_of_its_save() {
        let (mut h, a, _, mut c) = two_heads();
        let t = on_trial(&mut h, &mut c, &a);
        let before = saved(&mut h);
        h.state.test_modes = crate::backend::fake::Modes::Refused;
        let old = h.state.config.clone();
        h.state.config.outputs.push(crate::config::OutputConfig {
            name: "DP-2".into(),
            scale: 1.0,
            mode: Some(crate::config::OutputMode::parse("1280x720").unwrap()),
            adaptive_sync: None,
            transform: Default::default(),
            position: None,
            primary: false,
            icc_profile: None,
        });
        h.state.apply_output_config(&old);
        assert!(h.state.mode_refused("DP-2"));
        assert_eq!(saved(&mut h), before, "nothing written on trial");
        h.state.test_modes = crate::backend::fake::Modes::Taken;
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert_eq!(width(&a), 1920);
        assert_eq!(
            saved_mode(&mut h, "DP-1"),
            None,
            "no layout saved for the set, so the next start tries the pin"
        );
    }

    #[test]
    fn a_head_the_put_back_turns_off_gets_back_its_adaptive_sync_policy() {
        use crate::config::VrrPolicy::{Fullscreen, On};
        let (mut h, _, b, mut c) = two_heads();
        h.state.test_vrr.capable = true;
        h.state.vrr_policy.insert("DP-2".into(), Fullscreen);
        let off = crate::output::HeadRequest {
            enabled: false,
            mode: None,
            position: None,
            transform: smithay::utils::Transform::Normal,
            scale: 1.0,
            adaptive_sync: None,
        };
        assert!(h.state.apply_head(&b, &off));
        h.state.update_layout();
        crate::outputmgmt::publish(&mut h.state);
        h.roundtrip(&mut c);
        let t = c.begin_trial(15);
        h.roundtrip(&mut c);
        let on = c.apply_mode_covered(&t, "DP-2", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(answered(&on), Some("succeeded"));
        h.state.vrr_policy.insert("DP-2".into(), On);
        crate::outputmgmt::publish(&mut h.state);
        t.object.revert();
        h.roundtrip(&mut c);
        assert_eq!(t.heard(), ["started", "reverted"]);
        assert!(!h.state.scene.outputs().contains(&b), "off again");
        assert_eq!(h.state.vrr_policy.get("DP-2").copied(), Some(Fullscreen));
    }

    #[test]
    fn a_trial_is_held_for_one_to_sixty_seconds() {
        let held = [0, 1, 15, 60, 61, u32::MAX].map(|s| super::held_for(s).as_secs());
        assert_eq!(held, [1, 1, 15, 60, 60, 60]);
        let (mut h, a, _, mut c) = two_heads();
        let t = c.begin_trial(0);
        h.roundtrip(&mut c);
        c.apply_mode("DP-1", 1280, 720);
        h.roundtrip(&mut c);
        h.run_for(500);
        assert_eq!(t.heard(), ["started"], "a trial of 0 s is held for 1 s");
        h.until("the trial runs out", |_| width(&a) == 1920);
    }

    #[test]
    fn an_apply_with_no_trial_is_saved_and_stays() {
        let (mut h, a, _, mut c) = two_heads();
        let applied = c.apply_mode("DP-1", 1280, 720);
        h.roundtrip(&mut c);
        assert_eq!(answered(&applied), Some("succeeded"));
        assert!(saved(&mut h).contains("1280x720"));
        h.run_for(1200);
        assert_eq!(width(&a), 1280);
    }

    #[test]
    fn the_trial_global_is_offered_exactly_where_output_management_is() {
        let mut h = Harness::new();
        let mut clients = [
            h.connect(),
            h.connect_sandboxed(),
            h.connect_in_flatpak("org.example.App"),
        ];
        for c in &mut clients {
            h.roundtrip(c);
        }
        let offered = clients
            .iter()
            .map(|c| {
                (
                    c.offered("zwlr_output_manager_v1"),
                    c.offered("reprise_output_trial_manager_v1"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(offered, [(true, true), (false, false), (false, false)]);
    }
}
