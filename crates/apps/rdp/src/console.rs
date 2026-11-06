//! Lock mode: the run of viewers that locks this computer's screen, and the layout its first
//! viewer publishes as `ConsoleLayout` for the portal to make the viewers' screens from.

use crate::control::Status;
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use repriseconf::remote::{LaptopScreen, RemoteDesktop};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// A monitor as `ConsoleLayout` gives it: x, y, width, height and scale in percent.
pub type Monitor = (i32, i32, i32, i32, u32);

/// How long Display Control layouts must stop coming before the last one is published.
pub const SETTLE: Duration = Duration::from_millis(250);

/// How the compositor's names for the screens it makes for lock mode begin.
const REMOTE_SCREEN: &str = "REMOTE-";

/// The viewers shown the session together, from the first one joining until the last one's
/// display goes. One for the server, shared by every connection.
#[derive(Clone)]
pub struct Console(Arc<Shared>);

struct Shared {
    /// Its `console_layout` is kept here.
    status: watch::Sender<Status>,
    config: watch::Receiver<RemoteDesktop>,
    /// Whether `[power] locker` names another locker, which turns lock mode off.
    locker_set: Box<dyn Fn() -> bool + Send + Sync>,
    run: Mutex<Run>,
}

#[derive(Default)]
struct Run {
    viewers: usize,
    lock: bool,
    /// The viewer whose layouts are published.
    owner: Option<u64>,
    next: u64,
}

impl Console {
    pub fn new(
        status: watch::Sender<Status>,
        config: watch::Receiver<RemoteDesktop>,
        locker_set: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Console {
        Console(Arc::new(Shared {
            status,
            config,
            locker_set: Box::new(locker_set),
            run: Mutex::default(),
        }))
    }

    /// Whether a viewer joining now runs in lock mode: as its run does, or as configured when
    /// none runs.
    pub fn lock_mode(&self) -> bool {
        let run = self.0.run.lock().unwrap();
        match run.viewers {
            0 => self.configured_lock(),
            _ => run.lock,
        }
    }

    fn configured_lock(&self) -> bool {
        self.0.config.borrow().laptop_screen == LaptopScreen::Lock && !(self.0.locker_set)()
    }

    /// Joins the run of viewers, starting one with `laptop_screen` as configured now when none
    /// runs. The first viewer of a lock-mode run publishes `monitors`.
    pub fn join(&self, monitors: Vec<Monitor>) -> Joined {
        let mut run = self.0.run.lock().unwrap();
        let id = run.next;
        run.next += 1;
        if run.viewers == 0 {
            run.lock = self.configured_lock();
            run.owner = run.lock.then_some(id);
            if run.lock {
                self.publish(monitors);
            }
        }
        run.viewers += 1;
        Joined {
            console: self.clone(),
            id,
            lock: run.lock,
        }
    }

    fn publish(&self, layout: Vec<Monitor>) {
        self.0.status.send_if_modified(|s| {
            let changed = s.console_layout != layout;
            s.console_layout = layout;
            changed
        });
    }

    fn publish_as(&self, id: u64, layout: Vec<Monitor>) {
        let run = self.0.run.lock().unwrap();
        if run.owner == Some(id) {
            self.publish(layout);
        }
    }
}

/// A viewer's place in its run, left when dropped.
pub struct Joined {
    console: Console,
    id: u64,
    lock: bool,
}

impl Joined {
    pub fn lock(&self) -> bool {
        self.lock
    }

    /// Publishes the last of each burst of `layouts` once none came for [`SETTLE`], while this
    /// viewer's layout is the published one. Ends with the channel.
    pub fn follow(&self, mut layouts: mpsc::UnboundedReceiver<Vec<Monitor>>) {
        if self.console.0.run.lock().unwrap().owner != Some(self.id) {
            return;
        }
        let (console, id) = (self.console.clone(), self.id);
        tokio::spawn(async move {
            while let Some(mut last) = layouts.recv().await {
                loop {
                    match tokio::time::timeout(SETTLE, layouts.recv()).await {
                        Ok(Some(newer)) => last = newer,
                        Ok(None) => return,
                        Err(_) => break,
                    }
                }
                console.publish_as(id, last);
            }
        });
    }
}

impl Drop for Joined {
    fn drop(&mut self) {
        let mut run = self.console.0.run.lock().unwrap();
        run.viewers = run.viewers.saturating_sub(1);
        if run.owner == Some(self.id) {
            run.owner = None;
        }
        if run.viewers == 0 {
            self.console.publish(Vec::new());
        }
    }
}

/// `layout`'s monitors, primary first, each at its own scale or `scale`.
pub fn from_display_control(layout: &DisplayControlMonitorLayout, scale: u32) -> Vec<Monitor> {
    let mut monitors: Vec<(bool, Monitor)> = (layout.monitors().iter())
        .filter_map(|m| {
            let (x, y) = m.position()?;
            let (w, h) = m.dimensions();
            let own = (m.desktop_scale_factor()).filter(|_| m.device_scale_factor().is_some());
            let size = (i32::try_from(w).ok()?, i32::try_from(h).ok()?);
            Some((m.is_primary(), (x, y, size.0, size.1, own.unwrap_or(scale))))
        })
        .collect();
    monitors.sort_by_key(|(primary, _)| !primary);
    monitors.into_iter().map(|(_, m)| m).collect()
}

/// Whether a grant shares screens, every one made for lock mode, named as in `mapping_ids`. In
/// show mode, or where the portal fell back to it, the physical screens are shared instead.
pub fn remote_screens<'a>(mapping_ids: impl IntoIterator<Item = Option<&'a str>>) -> bool {
    let mut any = false;
    for id in mapping_ids {
        if !id.is_some_and(|name| name.starts_with(REMOTE_SCREEN)) {
            return false;
        }
        any = true;
    }
    any
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ironrdp_displaycontrol::pdu::{DeviceScaleFactor, MonitorLayoutEntry};

    /// A console configured with `laptop_screen`, a locker set or not, and the status it
    /// publishes to.
    pub(crate) fn configured(
        laptop_screen: LaptopScreen,
        locker: bool,
    ) -> (
        Console,
        watch::Sender<RemoteDesktop>,
        watch::Receiver<Status>,
    ) {
        let (config, config_rx) = watch::channel(RemoteDesktop {
            laptop_screen,
            ..RemoteDesktop::default()
        });
        let (status, status_rx) = watch::channel(Status::default());
        (
            Console::new(status, config_rx, move || locker),
            config,
            status_rx,
        )
    }

    /// A Display Control layout of `monitors`, the first primary.
    pub(crate) fn layout(monitors: &[(i32, i32, u32, u32)]) -> DisplayControlMonitorLayout {
        let entries: Vec<MonitorLayoutEntry> = (monitors.iter().enumerate())
            .map(|(i, &(x, y, w, h))| {
                let entry = match i {
                    0 => MonitorLayoutEntry::new_primary(w, h),
                    _ => MonitorLayoutEntry::new_secondary(w, h),
                };
                entry.unwrap().with_position(x, y).unwrap()
            })
            .collect();
        DisplayControlMonitorLayout::new(&entries).unwrap()
    }

    const FIRST: [Monitor; 2] = [(0, 0, 2560, 1440, 100), (2560, 0, 1920, 1080, 100)];
    const SECOND: [Monitor; 1] = [(0, 0, 1280, 800, 100)];

    fn published(status: &watch::Receiver<Status>) -> Vec<Monitor> {
        status.borrow().console_layout.clone()
    }

    #[test]
    fn the_first_lock_mode_viewer_publishes_its_monitors() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        assert!(published(&status).is_empty(), "nothing before a viewer");
        let first = console.join(FIRST.to_vec());
        assert!(first.lock());
        assert_eq!(published(&status), FIRST);
        drop(first);
        assert!(published(&status).is_empty(), "nothing once the run ends");

        let (console, _config, status) = configured(LaptopScreen::Show, false);
        let shown = console.join(FIRST.to_vec());
        assert!(!shown.lock());
        assert!(published(&status).is_empty(), "nothing in show mode");
    }

    #[test]
    fn a_locker_set_runs_in_show_mode() {
        let (console, _config, status) = configured(LaptopScreen::Lock, true);
        assert!(!console.join(FIRST.to_vec()).lock());
        assert!(published(&status).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_display_control_layout_replaces_the_published_layout_after_the_debounce() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let (layouts, rx) = mpsc::unbounded_channel();
        first.follow(rx);
        layouts.send(SECOND.to_vec()).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let last = vec![(0, 0, 1440, 900, 100)];
        layouts.send(last.clone()).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            published(&status),
            FIRST,
            "a burst still coming is not published"
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            published(&status),
            last,
            "the burst's last layout, 250 ms after it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_later_viewers_layout_is_ignored() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let later = console.join(SECOND.to_vec());
        assert!(later.lock(), "a later viewer runs in its run's mode");
        assert_eq!(published(&status), FIRST, "its monitors are not published");
        let (layouts, rx) = mpsc::unbounded_channel();
        later.follow(rx);
        let _ = layouts.send(SECOND.to_vec());
        tokio::time::sleep(SETTLE * 2).await;
        assert_eq!(
            published(&status),
            FIRST,
            "nor are its Display Control layouts"
        );
        drop(first);
        assert_eq!(
            published(&status),
            FIRST,
            "the layout lasts until the last viewer goes"
        );
        drop(later);
        assert!(published(&status).is_empty());
    }

    #[test]
    fn a_laptop_screen_change_applies_from_the_next_first_viewer() {
        let (console, config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        config.send_modify(|c| c.laptop_screen = LaptopScreen::Show);
        assert!(console.lock_mode(), "the run keeps its mode");
        let later = console.join(SECOND.to_vec());
        assert!(later.lock());
        assert_eq!(published(&status), FIRST);
        drop((first, later));
        assert!(!console.lock_mode(), "with no run, as configured");
        assert!(!console.join(SECOND.to_vec()).lock());
        assert!(published(&status).is_empty());
    }

    #[test]
    fn a_display_control_layout_is_published_primary_first() {
        let mut dc = layout(&[(0, 0, 1920, 1080), (-1280, 0, 1280, 1024)]);
        let entries = [
            dc.monitors()[1].clone(),
            (dc.monitors()[0].clone())
                .with_device_scale_factor(DeviceScaleFactor::Scale100Percent)
                .with_desktop_scale_factor(150)
                .unwrap(),
        ];
        dc = DisplayControlMonitorLayout::new(&entries).unwrap();
        assert_eq!(
            from_display_control(&dc, 125),
            [(0, 0, 1920, 1080, 150), (-1280, 0, 1280, 1024, 125)]
        );
    }

    #[test]
    fn only_screens_made_for_lock_mode_are_a_lock_mode_grant() {
        assert!(remote_screens(
            [Some("REMOTE-1"), Some("REMOTE-2")].into_iter()
        ));
        assert!(!remote_screens(
            [Some("REMOTE-1"), Some("eDP-1")].into_iter()
        ));
        assert!(!remote_screens([Some("REMOTE-1"), None].into_iter()));
        assert!(!remote_screens(std::iter::empty()));
    }
}
