//! What a connection learns of its client while it is set up: the monitors it sent.

use crate::layout;
use ironrdp_pdu::gcc::Monitor;
use ironrdp_server::{ConnectionHandler, ConnectionInfo, DesktopSize};
use std::sync::{Arc, Mutex};

/// The monitors a client sent in its Client Monitor Data: None until its connection is set
/// up, empty when it sent none.
#[derive(Clone, Default)]
pub struct ClientLayout(Arc<Mutex<Option<Vec<Monitor>>>>);

impl ClientLayout {
    /// The desktop the client's monitors span when they give Paired or Scaled.
    pub fn desktop(&self) -> Option<(u16, u16)> {
        let monitors = self.0.lock().unwrap();
        monitors.as_deref().and_then(layout::client_desktop)
    }
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
        *self.0.0.lock().unwrap() = Some(info.client_monitors.clone());
    }
}

/// A layout whose client connected with `monitors`.
#[cfg(test)]
pub fn connected(monitors: Vec<Monitor>) -> ClientLayout {
    let layout = ClientLayout::default();
    let mut info = ConnectionInfo::new(
        0x409,
        ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
        String::new(),
    );
    info.client_monitors = monitors;
    Handler(layout.clone()).on_connection_info(&info);
    layout
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
        assert_eq!(*layout.0.lock().unwrap(), Some(two_monitors()));
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
