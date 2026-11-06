//! What a connection learns of its client while it is set up: the monitors it sent and their
//! scale.

use crate::console;
use crate::layout;
use ironrdp_pdu::gcc::{ExtendedMonitorInfo, Monitor, MonitorFlags};
use ironrdp_server::{ConnectionHandler, ConnectionInfo, DesktopSize};
use std::sync::{Arc, Mutex};

/// The scale of a monitor whose client gave none, in percent.
const UNSCALED: u32 = 100;

/// What a client sent of its monitors: None until its connection is set up.
#[derive(Clone, Default)]
pub struct ClientLayout(Arc<Mutex<Option<Client>>>);

#[derive(Clone, Debug, Default, PartialEq)]
struct Client {
    /// Its Client Monitor Data, empty when it sent none.
    monitors: Vec<Monitor>,
    /// Its Client Monitor Extended Data, in `monitors` order, empty when it sent none.
    monitors_ex: Vec<ExtendedMonitorInfo>,
    /// The desktop scale factor of its Client Core Data, in percent.
    scale: Option<u32>,
}

impl ClientLayout {
    /// The desktop the client's monitors span when they give Paired or Scaled.
    pub fn desktop(&self) -> Option<(u16, u16)> {
        let client = self.0.lock().unwrap();
        client
            .as_ref()
            .and_then(|c| layout::client_desktop(&c.monitors))
    }

    pub fn monitors(&self) -> Vec<Monitor> {
        let client = self.0.lock().unwrap();
        client
            .as_ref()
            .map(|c| c.monitors.clone())
            .unwrap_or_default()
    }

    /// The scale of a monitor that gives none of its own: the client's desktop scale factor, else
    /// 100 percent.
    pub fn scale(&self) -> u32 {
        let client = self.0.lock().unwrap();
        client.as_ref().and_then(|c| c.scale).unwrap_or(UNSCALED)
    }

    /// The client's monitors as `ConsoleLayout` gives them, primary first, each at its own scale
    /// or [`Self::scale`]; one at `size` when it sent none or they cannot be used.
    pub fn console_layout(&self, size: DesktopSize) -> Vec<console::Monitor> {
        let scale = self.scale();
        let client = self.0.lock().unwrap();
        let (mut monitors, monitors_ex) = client.as_ref().map_or((&[][..], &[][..]), |c| {
            (&c.monitors[..], &c.monitors_ex[..])
        });
        if !monitors.is_empty()
            && let Err(why) = layout::usable(monitors).and(console::publishable(monitors))
        {
            log::warn!("client monitors not published: {why}");
            monitors = &[];
        }
        let paired = monitors_ex.len() == monitors.len();
        let mut out: Vec<(bool, console::Monitor)> = (monitors.iter().enumerate())
            .map(|(i, m)| {
                let own = paired.then(|| monitor_scale(&monitors_ex[i])).flatten();
                let (w, h) = (m.right - m.left + 1, m.bottom - m.top + 1);
                let primary = m.flags.contains(MonitorFlags::PRIMARY);
                (primary, (m.left, m.top, w, h, own.unwrap_or(scale)))
            })
            .collect();
        if out.is_empty() {
            let (w, h) = (i32::from(size.width), i32::from(size.height));
            return vec![(0, 0, w, h, scale)];
        }
        out.sort_by_key(|(primary, _)| !primary);
        out.into_iter().map(|(_, m)| m).collect()
    }
}

/// A monitor's own desktop scale factor, when it and its device scale factor are valid (MS-RDPBCGR
/// 2.2.1.3.9.1).
fn monitor_scale(ex: &ExtendedMonitorInfo) -> Option<u32> {
    let valid = (100..=500).contains(&ex.desktop_scale_factor)
        && matches!(ex.device_scale_factor, 100 | 140 | 180);
    valid.then_some(ex.desktop_scale_factor)
}

/// Offers a client the desktop its monitors span and keeps them in its [`ClientLayout`].
pub struct Handler(pub ClientLayout);

impl ConnectionHandler for Handler {
    fn choose_desktop_size(
        &mut self,
        client_monitors: &[Monitor],
        offered: DesktopSize,
    ) -> DesktopSize {
        layout::client_desktop(client_monitors)
            .map_or(offered, |(width, height)| DesktopSize { width, height })
    }

    fn on_connection_info(&mut self, info: &ConnectionInfo) {
        *self.0.0.lock().unwrap() = Some(Client {
            monitors: info.client_monitors.clone(),
            monitors_ex: info.client_monitors_ex.clone(),
            scale: info.desktop_scale_factor,
        });
    }
}

/// A layout whose client connected with `monitors`.
#[cfg(test)]
pub fn connected(monitors: Vec<Monitor>) -> ClientLayout {
    scaled(monitors, Vec::new(), None)
}

/// A layout whose client connected with `monitors`, their extended data and a desktop scale.
#[cfg(test)]
pub fn scaled(
    monitors: Vec<Monitor>,
    monitors_ex: Vec<ExtendedMonitorInfo>,
    scale: Option<u32>,
) -> ClientLayout {
    let layout = ClientLayout::default();
    let mut info = ConnectionInfo::new(
        0x409,
        ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
        String::new(),
    );
    info.client_monitors = monitors;
    info.client_monitors_ex = monitors_ex;
    info.desktop_scale_factor = scale;
    info.device_scale_factor = scale.map(|_| 100);
    Handler(layout.clone()).on_connection_info(&info);
    layout
}

/// Extended data giving a monitor `scale` percent.
#[cfg(test)]
pub fn at_scale(scale: u32) -> ExtendedMonitorInfo {
    ExtendedMonitorInfo {
        physical_width: 600,
        physical_height: 340,
        orientation: ironrdp_pdu::gcc::MonitorOrientation::Landscape,
        desktop_scale_factor: scale,
        device_scale_factor: 100,
    }
}

/// A primary 2560x1440 monitor with a 1920x1080 one to its right.
#[cfg(test)]
pub fn two_monitors() -> Vec<Monitor> {
    use ironrdp_pdu::gcc::MonitorFlags;
    vec![
        Monitor {
            left: 0,
            top: 0,
            right: 2559,
            bottom: 1439,
            flags: MonitorFlags::PRIMARY,
        },
        Monitor {
            left: 2560,
            top: 0,
            right: 4479,
            bottom: 1079,
            flags: MonitorFlags::empty(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_pdu::gcc::MonitorFlags;

    const OFFERED: DesktopSize = DesktopSize {
        width: 1920,
        height: 1200,
    };

    #[test]
    fn the_handler_keeps_the_client_monitors() {
        let layout = connected(two_monitors());
        assert_eq!(layout.monitors(), two_monitors());
    }

    #[test]
    fn each_monitor_is_published_at_its_own_scale_else_the_desktops_else_100() {
        let size = DesktopSize {
            width: 1280,
            height: 800,
        };
        let mut second_first = two_monitors();
        second_first.reverse();
        let own = scaled(second_first, vec![at_scale(125), at_scale(150)], Some(200));
        assert_eq!(
            own.console_layout(size),
            [(0, 0, 2560, 1440, 150), (2560, 0, 1920, 1080, 125)],
            "the primary first, each at its own scale"
        );
        let invalid = scaled(two_monitors(), vec![at_scale(50), at_scale(150)], Some(200));
        assert_eq!(
            invalid.console_layout(size),
            [(0, 0, 2560, 1440, 200), (2560, 0, 1920, 1080, 150)],
            "a scale out of range takes the desktop's"
        );
        let unpaired = scaled(two_monitors(), vec![at_scale(150)], None);
        assert_eq!(
            unpaired.console_layout(size),
            [(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)],
            "extended data for some monitors only is not used"
        );
        assert_eq!(
            scaled(vec![], vec![], Some(150)).console_layout(size),
            [(0, 0, 1280, 800, 150)],
            "a client that sent no monitors is one at its desktop size"
        );
    }

    #[test]
    fn out_of_range_client_monitors_publish_the_single_desktop() {
        let size = DesktopSize {
            width: 1280,
            height: 800,
        };
        let monitor = |left, right, bottom, flags| Monitor {
            left,
            top: 0,
            right,
            bottom,
            flags,
        };
        let primary = MonitorFlags::PRIMARY;
        for monitors in [
            vec![monitor(0, i32::MAX, 1079, primary)],
            vec![monitor(0, 99, 99, primary)],
            vec![
                monitor(0, 1919, 1079, primary),
                monitor(i32::MIN, -1, 1079, MonitorFlags::empty()),
            ],
            vec![monitor(10, 1929, 1079, primary)],
            vec![
                monitor(0, 1919, 1079, primary),
                monitor(1920, 2019, 1079, MonitorFlags::empty()),
            ],
            vec![
                monitor(0, 1919, 1079, primary),
                monitor(1920, 3839, 149, MonitorFlags::empty()),
            ],
        ] {
            assert_eq!(
                scaled(monitors.clone(), vec![], Some(150)).console_layout(size),
                [(0, 0, 1280, 800, 150)],
                "{monitors:?} give one monitor at the desktop size"
            );
        }
    }

    #[test]
    fn two_client_monitors_are_offered_the_desktop_they_span() {
        let mut handler = Handler(ClientLayout::default());
        let offered = handler.choose_desktop_size(&two_monitors(), OFFERED);
        assert_eq!(
            offered,
            DesktopSize {
                width: 4480,
                height: 1440
            }
        );
    }

    #[test]
    fn a_whole_client_keeps_the_offered_size() {
        let one = two_monitors()[..1].to_vec();
        let two_primaries = two_monitors()
            .into_iter()
            .map(|m| Monitor {
                flags: MonitorFlags::PRIMARY,
                ..m
            })
            .collect::<Vec<_>>();
        for monitors in [vec![], one, two_primaries] {
            let mut handler = Handler(ClientLayout::default());
            let offered = handler.choose_desktop_size(&monitors, OFFERED);
            assert_eq!(offered, OFFERED, "{monitors:?} keep 1920x1200");
        }
    }
}
