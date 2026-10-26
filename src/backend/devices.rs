//! The GPUs the backend has open, keyed by primary node; the primary renders and scans out.

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

/// The open devices; the primary is always among them.
pub struct DeviceSet<D, N = DrmNode> {
    primary: N,
    devices: Vec<(N, D)>,
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
        set.devices.push((2, "b"));
        set
    }

    fn head(node: u64, crtc: u32) -> Head<u64> {
        Head {
            node,
            crtc: crtc::Handle::from(RawResourceHandle::new(crtc).unwrap()),
        }
    }

    #[test]
    fn a_new_set_holds_only_its_primary() {
        let set = DeviceSet::new(1u64, "a");
        assert_eq!(*set.primary(), "a");
        assert_eq!(set.iter().map(|(n, _)| n).collect::<Vec<_>>(), vec![1]);
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
