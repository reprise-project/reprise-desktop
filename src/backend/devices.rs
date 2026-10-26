//! The GPUs the backend has open, keyed by primary node. One renders and scans out (the
//! primary); losing it ends the session, losing another drops only its heads.

use smithay::{backend::drm::DrmNode, reexports::drm::control::crtc};

/// A CRTC on one device: CRTC handles repeat across cards.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Head<N = DrmNode> {
    pub node: N,
    pub crtc: crtc::Handle,
}

/// A device key with the `dev_t` its udev events carry.
pub trait DevNum: Copy + Eq {
    fn dev_num(&self) -> u64;
}

impl DevNum for DrmNode {
    fn dev_num(&self) -> u64 {
        self.dev_id()
    }
}

/// The GPU that renders and scans out: `wanted` when the seat has it, else the seat's primary,
/// else its first.
pub fn pick_gpu<T: Copy + PartialEq>(
    wanted: Option<T>,
    primary: Option<T>,
    seat: &[T],
) -> Option<T> {
    wanted
        .filter(|w| seat.contains(w))
        .or(primary)
        .or_else(|| seat.first().copied())
}

/// The GPUs to open beside `primary`: every other GPU of the seat under `[display] all_gpus`,
/// else none.
pub fn secondaries<T: Copy + PartialEq>(all_gpus: bool, seat: &[T], primary: T) -> Vec<T> {
    let others = seat.iter().copied().filter(|n| *n != primary);
    others.filter(|_| all_gpus).collect()
}

/// The open devices; the primary is always among them.
pub struct DeviceSet<D, N = DrmNode> {
    primary: N,
    devices: Vec<(N, D)>,
}

/// What removing a device leaves to do.
#[derive(Debug, PartialEq)]
pub enum Removed<D, N> {
    /// The primary went: the session ends. It stays in the set.
    Primary,
    /// A secondary went, with the heads that were on it.
    Secondary(D, Vec<Head<N>>),
    Unknown,
}

impl<D, N: DevNum> DeviceSet<D, N> {
    pub fn new(primary: N, device: D) -> Self {
        DeviceSet {
            primary,
            devices: vec![(primary, device)],
        }
    }

    pub fn primary(&self) -> &D {
        self.get(self.primary)
            .expect("the primary stays in the set")
    }

    pub fn primary_mut(&mut self) -> &mut D {
        let node = self.primary;
        self.get_mut(node).expect("the primary stays in the set")
    }

    pub fn is_primary(&self, node: N) -> bool {
        node == self.primary
    }

    pub fn contains(&self, node: N) -> bool {
        self.devices.iter().any(|(n, _)| *n == node)
    }

    /// Add `device` as `node`; a known node keeps the device it has. False when nothing was
    /// added.
    pub fn add(&mut self, node: N, device: D) -> bool {
        if self.contains(node) {
            return false;
        }
        self.devices.push((node, device));
        true
    }

    /// Take `node` out of the set; `heads` are every head the backend holds.
    pub fn remove(&mut self, node: N, heads: impl IntoIterator<Item = Head<N>>) -> Removed<D, N> {
        if node == self.primary {
            return Removed::Primary;
        }
        let Some(i) = self.devices.iter().position(|(n, _)| *n == node) else {
            return Removed::Unknown;
        };
        let (_, device) = self.devices.remove(i);
        let gone = heads.into_iter().filter(|h| h.node == node).collect();
        Removed::Secondary(device, gone)
    }

    pub fn get(&self, node: N) -> Option<&D> {
        self.devices
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, d)| d)
    }

    pub fn get_mut(&mut self, node: N) -> Option<&mut D> {
        (self.devices.iter_mut())
            .find(|(n, _)| *n == node)
            .map(|(_, d)| d)
    }

    /// The device `head` is on.
    pub fn of(&self, head: Head<N>) -> Option<&D> {
        self.get(head.node)
    }

    /// The device whose udev events carry `devnum`.
    pub fn by_devnum(&self, devnum: u64) -> Option<N> {
        (self.devices.iter())
            .map(|(n, _)| *n)
            .find(|n| n.dev_num() == devnum)
    }

    pub fn iter(&self) -> impl Iterator<Item = (N, &D)> {
        self.devices.iter().map(|(n, d)| (*n, d))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (N, &mut D)> {
        self.devices.iter_mut().map(|(n, d)| (*n, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::drm::control::RawResourceHandle;

    impl DevNum for u64 {
        fn dev_num(&self) -> u64 {
            *self + 0xe200
        }
    }

    fn two() -> DeviceSet<&'static str, u64> {
        let mut set = DeviceSet::new(1u64, "a");
        assert!(set.add(2, "b"));
        set
    }

    fn nodes<D>(set: &DeviceSet<D, u64>) -> Vec<u64> {
        set.iter().map(|(n, _)| n).collect()
    }

    fn head(node: u64, crtc: u32) -> Head<u64> {
        Head {
            node,
            crtc: crtc::Handle::from(RawResourceHandle::new(crtc).unwrap()),
        }
    }

    #[test]
    fn only_all_gpus_opens_the_other_gpus() {
        assert_eq!(
            secondaries(false, &[1, 2, 3], 2),
            Vec::<u64>::new(),
            "the default opens no secondary"
        );
        assert_eq!(secondaries(true, &[1, 2, 3], 2), vec![1, 3]);
        assert_eq!(secondaries(true, &[2], 2), Vec::<u64>::new());
    }

    #[test]
    fn a_new_set_holds_only_its_primary() {
        let set = DeviceSet::new(1u64, "a");
        assert_eq!(*set.primary(), "a");
        assert_eq!(nodes(&set), vec![1]);
        assert!(set.is_primary(1) && !set.is_primary(2));
    }

    #[test]
    fn a_second_add_of_a_known_node_is_a_no_op() {
        let mut set = two();
        assert!(!set.add(2, "c"));
        assert!(!set.add(1, "c"), "nor does the primary change");
        assert_eq!(nodes(&set), vec![1, 2], "primary first");
        assert_eq!(set.get(2), Some(&"b"));
        assert_eq!(*set.primary(), "a");
    }

    #[test]
    fn removing_the_primary_keeps_it_and_ends_the_session() {
        let mut set = two();
        assert_eq!(set.remove(1, [head(1, 50)]), Removed::Primary);
        assert_eq!(*set.primary(), "a");
        assert_eq!(nodes(&set), vec![1, 2]);
    }

    #[test]
    fn removing_a_secondary_returns_its_heads_only() {
        let mut set = two();
        let heads = [head(1, 50), head(2, 50), head(2, 51)];
        assert_eq!(
            set.remove(2, heads),
            Removed::Secondary("b", vec![head(2, 50), head(2, 51)]),
            "the primary's CRTC 50 stays"
        );
        assert!(!set.contains(2));
        assert_eq!(set.remove(2, heads), Removed::Unknown);
    }

    #[test]
    fn a_head_finds_its_own_device_though_crtc_handles_repeat() {
        let set = two();
        assert_eq!(set.of(head(1, 50)), Some(&"a"));
        assert_eq!(set.of(head(2, 50)), Some(&"b"));
        assert_eq!(set.of(head(3, 50)), None);
        assert_ne!(head(1, 50), head(2, 50));
    }

    #[test]
    fn a_udev_devnum_finds_its_device() {
        let set = two();
        assert_eq!(set.by_devnum(0xe202), Some(2));
        assert_eq!(set.by_devnum(0xe201), Some(1));
        assert_eq!(set.by_devnum(0xe203), None);
    }
}
