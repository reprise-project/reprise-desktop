//! The device reset after a session return: every CRTC, connector and plane off, so a CRTC
//! another DRM master left lit on a connector one of ours claims cannot fail our commits. Its
//! blocking commit runs on the `commit-device` thread; main finishes the outputs on its `Done`.

use std::{
    collections::{HashMap, hash_map::Entry},
    time::{Duration, Instant},
};

use smithay::{
    backend::drm::DrmDeviceFd,
    reexports::drm::control::{
        AtomicCommitFlags, Device as _, RawResourceHandle, ResourceHandle, atomic::AtomicModeReq,
        connector, crtc, plane, property,
    },
};

/// An output left lit through the reset: a frame of ours took on its CRTC since the return, so
/// no other master changed it after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub crtc: crtc::Handle,
    pub connector: connector::Handle,
    pub primary: plane::Handle,
}

/// The device's routing as read before the reset.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// Each connector's CRTC.
    pub connectors: Vec<(connector::Handle, Option<crtc::Handle>)>,
    /// Each CRTC's ACTIVE.
    pub crtcs: Vec<(crtc::Handle, bool)>,
    /// Each plane's CRTC and whether it shows a framebuffer.
    pub planes: Vec<(plane::Handle, Option<crtc::Handle>, bool)>,
}

/// What the reset turns off, and the CRTCs it leaves lit.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub connectors: Vec<connector::Handle>,
    pub crtcs: Vec<crtc::Handle>,
    pub planes: Vec<plane::Handle>,
    pub kept: Vec<crtc::Handle>,
}

/// Everything off but the `keep` entries `now` confirms: the CRTC active, driving that
/// connector alone, its primary plane showing a framebuffer. Their other planes go off too.
pub fn plan(now: &Snapshot, keep: &[Keep]) -> Plan {
    let kept: Vec<Keep> = keep
        .iter()
        .copied()
        .filter(|k| {
            let active = now.crtcs.contains(&(k.crtc, true));
            let mut driven = now
                .connectors
                .iter()
                .filter(|(_, c)| *c == Some(k.crtc))
                .map(|(conn, _)| *conn);
            let alone = driven.next() == Some(k.connector) && driven.next().is_none();
            let shown = now.planes.contains(&(k.primary, Some(k.crtc), true));
            active && alone && shown
        })
        .collect();
    let on_kept = |c: &Option<crtc::Handle>| kept.iter().any(|k| Some(k.crtc) == *c);
    Plan {
        connectors: now
            .connectors
            .iter()
            .filter(|(_, c)| !on_kept(c))
            .map(|(conn, _)| *conn)
            .collect(),
        crtcs: now
            .crtcs
            .iter()
            .map(|(c, _)| *c)
            .filter(|c| !kept.iter().any(|k| k.crtc == *c))
            .collect(),
        planes: now
            .planes
            .iter()
            .map(|(p, ..)| *p)
            .filter(|p| !kept.iter().any(|k| k.primary == *p))
            .collect(),
        kept: kept.iter().map(|k| k.crtc).collect(),
    }
}

/// How long a return's reset waits for DRM master before it is sent anyway.
const MASTER_WAIT: Duration = Duration::from_secs(1);
const MASTER_POLL: Duration = Duration::from_millis(1);

/// Main's side of the reset each session return needs. Outputs render only once the latest
/// return's reset is done.
#[derive(Debug, Default)]
pub struct Return {
    returns: u64,
    since: Option<Instant>,
    due: bool,
    /// The return the reset on the thread was sent for.
    sent: Option<u64>,
}

impl Return {
    /// The session came back at `now`.
    pub fn activated(&mut self, now: Instant) {
        self.returns += 1;
        self.since = Some(now);
        self.due = true;
    }

    /// The latest return still waits for its reset.
    pub fn pending(&self) -> bool {
        self.due
    }

    /// A reset is due and none is on the thread.
    pub fn ready(&self) -> bool {
        self.due && self.sent.is_none()
    }

    pub fn sent(&mut self) {
        debug_assert!(self.sent.is_none(), "one device reset at a time");
        self.sent = Some(self.returns);
    }

    /// The reset's `Done`. True when it settles the latest return and the session is still
    /// `active`: main finishes the outputs now. False when the session went away or came back
    /// again meanwhile; another reset follows.
    pub fn done(&mut self, active: bool) -> bool {
        let settles = self.sent.take() == Some(self.returns) && active;
        if settles {
            self.due = false;
        }
        settles
    }

    /// logind reports the session active before it hands DRM master back, and a reset sent
    /// without master is refused (EACCES): the delay before checking `master` again, or `None`
    /// to go on.
    pub fn master_wait(&self, master: bool, now: Instant) -> Option<Duration> {
        let waited = self
            .since
            .map_or(Duration::MAX, |s| now.saturating_duration_since(s));
        (!master && waited < MASTER_WAIT).then_some(MASTER_POLL)
    }

    /// A commit that started at `at` went in after the latest return.
    pub fn after_return(&self, at: Instant) -> bool {
        self.since.is_some_and(|s| at >= s)
    }
}

/// The `commit-device` thread's side: the device fd.
pub struct Device {
    pub fd: DrmDeviceFd,
    pub atomic: bool,
}

impl super::commit::Committer for Device {
    type Payload = Vec<Keep>;
    type Output = Vec<crtc::Handle>;

    fn commit(
        &mut self,
        _: super::commit::Kind,
        keep: Vec<Keep>,
    ) -> Result<Vec<crtc::Handle>, String> {
        if self.atomic {
            reset_atomic(&self.fd, &keep)
        } else {
            reset_legacy(&self.fd).map(|()| Vec::new())
        }
    }
}

/// An object's properties by name: handle and value.
type Props = HashMap<String, (property::Handle, u64)>;

fn props<H: ResourceHandle>(
    fd: &DrmDeviceFd,
    h: H,
    names: &mut HashMap<property::Handle, String>,
) -> Result<Props, String> {
    let set = fd
        .get_properties(h)
        .map_err(|e| format!("properties: {e}"))?;
    let mut out = Props::new();
    for (prop, value) in set {
        let name = match names.entry(prop) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let info = fd
                    .get_property(prop)
                    .map_err(|e| format!("property: {e}"))?;
                e.insert(info.name().to_string_lossy().into_owned())
            }
        };
        out.insert(name.clone(), (prop, value));
    }
    Ok(out)
}

fn object<H: From<RawResourceHandle>>(value: u64) -> Option<H> {
    u32::try_from(value)
        .ok()
        .and_then(RawResourceHandle::new)
        .map(H::from)
}

fn handle_of(props: &Props, name: &str) -> Result<property::Handle, String> {
    props
        .get(name)
        .map(|(h, _)| *h)
        .ok_or_else(|| format!("no {name} property"))
}

/// One ALLOW_MODESET commit turning off what `plan` picks (as Smithay's `reset_state`, minus
/// what `keep` leaves lit). Returns the kept CRTCs.
fn reset_atomic(fd: &DrmDeviceFd, keep: &[Keep]) -> Result<Vec<crtc::Handle>, String> {
    let res = fd
        .resource_handles()
        .map_err(|e| format!("resources: {e}"))?;
    let planes = fd.plane_handles().map_err(|e| format!("planes: {e}"))?;
    let mut names = HashMap::new();
    let value = |p: &Props, name: &str| p.get(name).map_or(0, |(_, v)| *v);
    let mut now = Snapshot::default();
    let mut conns = HashMap::new();
    for &c in res.connectors() {
        let p = props(fd, c, &mut names)?;
        now.connectors.push((c, object(value(&p, "CRTC_ID"))));
        conns.insert(c, p);
    }
    let mut crtcs = HashMap::new();
    for &c in res.crtcs() {
        let p = props(fd, c, &mut names)?;
        now.crtcs.push((c, value(&p, "ACTIVE") != 0));
        crtcs.insert(c, p);
    }
    let mut planes_props = HashMap::new();
    for &pl in &planes {
        let p = props(fd, pl, &mut names)?;
        now.planes
            .push((pl, object(value(&p, "CRTC_ID")), value(&p, "FB_ID") != 0));
        planes_props.insert(pl, p);
    }
    let plan = plan(&now, keep);
    let mut req = AtomicModeReq::new();
    for c in &plan.connectors {
        req.add_property(
            *c,
            handle_of(&conns[c], "CRTC_ID")?,
            property::Value::CRTC(None),
        );
    }
    for pl in &plan.planes {
        let p = &planes_props[pl];
        req.add_property(*pl, handle_of(p, "CRTC_ID")?, property::Value::CRTC(None));
        req.add_property(
            *pl,
            handle_of(p, "FB_ID")?,
            property::Value::Framebuffer(None),
        );
    }
    for c in &plan.crtcs {
        let p = &crtcs[c];
        req.add_property(*c, handle_of(p, "ACTIVE")?, property::Value::Boolean(false));
        req.add_property(*c, handle_of(p, "MODE_ID")?, property::Value::Unknown(0));
    }
    fd.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)
        .map_err(|e| format!("disable all: {e}"))?;
    Ok(plan.kept)
}

/// Every CRTC off through the legacy API, as Smithay's legacy `reset_state` does.
fn reset_legacy(fd: &DrmDeviceFd) -> Result<(), String> {
    let res = fd
        .resource_handles()
        .map_err(|e| format!("resources: {e}"))?;
    for &c in res.crtcs() {
        fd.set_crtc(c, None, (0, 0), &[], None)
            .map_err(|e| format!("{c:?} off: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_sends_its_reset_only_once_the_device_is_master_again() {
        let mut r = Return::default();
        let t = Instant::now();
        r.activated(t);
        assert_eq!(
            r.master_wait(false, t),
            Some(MASTER_POLL),
            "waits for master"
        );
        assert_eq!(r.master_wait(false, t + MASTER_WAIT / 2), Some(MASTER_POLL));
        assert_eq!(r.master_wait(true, t), None, "master: the reset goes now");
        assert_eq!(
            r.master_wait(false, t + MASTER_WAIT),
            None,
            "after the wait the reset goes anyway and logs its failure"
        );
    }

    fn h<H: From<RawResourceHandle>>(id: u32) -> H {
        H::from(RawResourceHandle::new(id).unwrap())
    }

    /// CRTC 50 drives connector 10 with primary 70 and cursor 71 lit; CRTC 51 is off.
    fn one_lit() -> Snapshot {
        Snapshot {
            connectors: vec![(h(10), Some(h(50))), (h(11), None)],
            crtcs: vec![(h(50), true), (h(51), false)],
            planes: vec![
                (h(70), Some(h(50)), true),
                (h(71), Some(h(50)), true),
                (h(72), None, false),
            ],
        }
    }

    fn keep_50() -> Keep {
        Keep {
            crtc: h(50),
            connector: h(10),
            primary: h(70),
        }
    }

    #[test]
    fn with_nothing_kept_everything_goes_off() {
        let p = plan(&one_lit(), &[]);
        assert_eq!(p.connectors, vec![h(10), h(11)]);
        assert_eq!(p.crtcs, vec![h(50), h(51)]);
        assert_eq!(p.planes, vec![h(70), h(71), h(72)]);
        assert!(p.kept.is_empty());
    }

    #[test]
    fn a_crtc_our_modeset_lit_keeps_its_connector_and_primary_only() {
        let p = plan(&one_lit(), &[keep_50()]);
        assert_eq!(p.kept, vec![h(50)]);
        assert_eq!(p.connectors, vec![h(11)]);
        assert_eq!(p.crtcs, vec![h(51)]);
        assert_eq!(p.planes, vec![h(71), h(72)], "the cursor plane goes off");
    }

    #[test]
    fn a_kept_crtc_driving_another_connector_too_goes_off() {
        let mut now = one_lit();
        now.connectors[1].1 = Some(h(50));
        assert!(plan(&now, &[keep_50()]).kept.is_empty());
    }

    #[test]
    fn a_kept_crtc_whose_connector_moved_goes_off() {
        let mut now = one_lit();
        now.connectors = vec![(h(10), Some(h(51))), (h(11), Some(h(50)))];
        now.crtcs[1].1 = true;
        let p = plan(&now, &[keep_50()]);
        assert!(p.kept.is_empty());
        assert_eq!(p.crtcs, vec![h(50), h(51)]);
    }

    #[test]
    fn a_kept_crtc_that_is_off_or_shows_no_frame_goes_off() {
        let mut off = one_lit();
        off.crtcs[0].1 = false;
        assert!(plan(&off, &[keep_50()]).kept.is_empty());
        let mut blank = one_lit();
        blank.planes[0].2 = false;
        assert!(plan(&blank, &[keep_50()]).kept.is_empty());
        let mut moved = one_lit();
        moved.planes[0].1 = Some(h(51));
        assert!(plan(&moved, &[keep_50()]).kept.is_empty());
    }

    #[test]
    fn a_reset_settles_the_return_it_was_sent_for() {
        let mut r = Return::default();
        assert!(!r.pending() && !r.ready());
        r.activated(Instant::now());
        assert!(r.pending() && r.ready());
        r.sent();
        assert!(
            r.pending() && !r.ready(),
            "one reset on the thread at a time"
        );
        assert!(r.done(true));
        assert!(!r.pending() && !r.ready());
    }

    #[test]
    fn a_return_during_the_reset_sends_another() {
        let mut r = Return::default();
        r.activated(Instant::now());
        r.sent();
        r.activated(Instant::now());
        assert!(!r.ready(), "not while the first is out");
        assert!(!r.done(true), "the first reset predates the latest return");
        assert!(r.pending() && r.ready());
        r.sent();
        assert!(r.done(true));
        assert!(!r.pending());
    }

    #[test]
    fn a_reset_that_ends_with_the_session_away_runs_again_on_return() {
        let mut r = Return::default();
        r.activated(Instant::now());
        r.sent();
        assert!(!r.done(false));
        assert!(r.pending() && r.ready());
    }

    #[test]
    fn only_a_commit_after_the_latest_return_counts() {
        let mut r = Return::default();
        let before = Instant::now();
        assert!(!r.after_return(before), "no return yet");
        r.activated(Instant::now());
        assert!(!r.after_return(before));
        assert!(r.after_return(Instant::now()));
        let lit = Instant::now();
        r.activated(Instant::now());
        assert!(!r.after_return(lit), "a later return makes it stale");
    }
}
