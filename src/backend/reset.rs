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
        AtomicCommitFlags, Device as _, ResourceHandle, atomic::AtomicModeReq, property,
    },
};

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
}

/// The `commit-device` thread's side: the device fd.
pub struct Device {
    pub fd: DrmDeviceFd,
    pub atomic: bool,
}

impl super::commit::Committer for Device {
    type Payload = ();
    type Output = ();

    fn commit(&mut self, _: super::commit::Kind, (): ()) -> Result<(), String> {
        if self.atomic {
            reset_atomic(&self.fd)
        } else {
            reset_legacy(&self.fd)
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

fn handle_of(props: &Props, name: &str) -> Result<property::Handle, String> {
    props
        .get(name)
        .map(|(h, _)| *h)
        .ok_or_else(|| format!("no {name} property"))
}

/// One ALLOW_MODESET commit turning every connector, plane and CRTC off, as Smithay's
/// `reset_state` does.
fn reset_atomic(fd: &DrmDeviceFd) -> Result<(), String> {
    let res = fd
        .resource_handles()
        .map_err(|e| format!("resources: {e}"))?;
    let planes = fd.plane_handles().map_err(|e| format!("planes: {e}"))?;
    let mut names = HashMap::new();
    let mut req = AtomicModeReq::new();
    for &c in res.connectors() {
        let p = props(fd, c, &mut names)?;
        req.add_property(c, handle_of(&p, "CRTC_ID")?, property::Value::CRTC(None));
    }
    for &pl in &planes {
        let p = props(fd, pl, &mut names)?;
        req.add_property(pl, handle_of(&p, "CRTC_ID")?, property::Value::CRTC(None));
        req.add_property(
            pl,
            handle_of(&p, "FB_ID")?,
            property::Value::Framebuffer(None),
        );
    }
    for &c in res.crtcs() {
        let p = props(fd, c, &mut names)?;
        req.add_property(c, handle_of(&p, "ACTIVE")?, property::Value::Boolean(false));
        req.add_property(c, handle_of(&p, "MODE_ID")?, property::Value::Unknown(0));
    }
    fd.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)
        .map_err(|e| format!("disable all: {e}"))
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
}
