//! Lock mode: the run of viewers that locks this computer's screen, and the layout its owner (its
//! first viewer, or one that replaced the others) publishes as `ConsoleLayout` for the portal.

use crate::control::Status;
use ironrdp_displaycontrol::pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout};
use ironrdp_pdu::gcc;
use repriseconf::remote::{LaptopScreen, RemoteDesktop, SecondClient};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// A monitor as `ConsoleLayout` gives it: x, y, width, height and scale in percent.
pub type Monitor = (i32, i32, i32, i32, u32);

/// How long Display Control layouts must stop coming before the last one is published.
pub const SETTLE: Duration = Duration::from_millis(250);

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

    /// Whether a viewer joining now would start a lock-mode run, or take over the one going on
    /// once it logs in with `second_client = "replace"`, and so have its layouts published.
    pub fn would_own(&self) -> bool {
        let run = self.0.run.lock().unwrap();
        match run.viewers {
            0 => self.configured_lock(),
            _ => run.lock && self.0.config.borrow().second_client == SecondClient::Replace,
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

    /// Joins the run going on in place of its other viewers, taking its layout; None when no
    /// viewer is in a run.
    pub fn take_over(&self) -> Option<Joined> {
        let joined = {
            let mut run = self.0.run.lock().unwrap();
            if run.viewers == 0 {
                return None;
            }
            let id = run.next;
            run.next += 1;
            run.viewers += 1;
            Joined {
                console: self.clone(),
                id,
                lock: run.lock,
            }
        };
        self.take_owner(joined.id);
        Some(joined)
    }

    /// The published layout of a lock-mode run follows `viewer` from now on.
    pub fn take_owner(&self, viewer: u64) {
        let mut run = self.0.run.lock().unwrap();
        if run.lock {
            run.owner = Some(viewer);
        }
    }

    fn publish(&self, layout: Vec<Monitor>) {
        self.0.status.send_if_modified(|s| {
            let changed = s.console_layout != layout;
            s.console_layout = layout;
            changed
        });
    }

    /// True if `id` owns the run, so `layout` was published.
    fn publish_as(&self, id: u64, layout: Vec<Monitor>) -> bool {
        let run = self.0.run.lock().unwrap();
        let owner = run.owner == Some(id);
        if owner {
            self.publish(layout);
        }
        owner
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

    /// Publishes `monitors` if this viewer owns the run.
    pub fn take_layout(&self, monitors: Vec<Monitor>) {
        self.console.publish_as(self.id, monitors);
    }

    /// Publishes the last of each burst of `layouts` once none came for [`SETTLE`], while this
    /// viewer's layout is the published one, and passes each to `published`. Ends with the channel.
    pub fn follow(
        &self,
        mut layouts: mpsc::UnboundedReceiver<Vec<Monitor>>,
        published: impl Fn(&[Monitor]) + Send + 'static,
    ) {
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
                if console.publish_as(id, last.clone()) {
                    published(&last);
                }
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

/// The pixels a published monitor may have on a side, as the compositor takes them.
const SIDES: std::ops::RangeInclusive<i32> = 200..=8192;
/// The farthest a published monitor may start from the origin on either axis.
const MAX_COORD: i32 = 8192 * 16;

/// Why the compositor would refuse `client`'s monitors as a layout, if it would.
pub fn publishable(client: &[gcc::Monitor]) -> Result<(), &'static str> {
    let side = |min: i32, max: i32| i64::from(max) - i64::from(min) + 1;
    let fits = |m: &gcc::Monitor| {
        let sides = (side(m.left, m.right), side(m.top, m.bottom));
        let sides = (i32::try_from(sides.0), i32::try_from(sides.1));
        let start =
            (-MAX_COORD..=MAX_COORD).contains(&m.left) && (-MAX_COORD..=MAX_COORD).contains(&m.top);
        matches!(sides, (Ok(w), Ok(h)) if SIDES.contains(&w) && SIDES.contains(&h)) && start
    };
    match client.iter().all(fits) {
        true => Ok(()),
        false => Err("a monitor has a side outside 200 to 8192 or starts past 131072"),
    }
}

/// `layout` as Client Monitor Data gives it, the first monitor primary.
pub fn client_monitors(layout: &[Monitor]) -> Vec<gcc::Monitor> {
    (layout.iter().enumerate())
        .map(|(i, &(x, y, w, h, _))| gcc::Monitor {
            left: x,
            top: y,
            right: x.saturating_add(w - 1),
            bottom: y.saturating_add(h - 1),
            flags: match i {
                0 => gcc::MonitorFlags::PRIMARY,
                _ => gcc::MonitorFlags::empty(),
            },
        })
        .collect()
}

/// The area each offered monitor may have, as MS-RDPEDISP's MaxMonitorAreaFactorA and B, as
/// the server advertises them.
const MONITOR_AREA: (u32, u32) = (3840, 2400);

/// `layout`'s monitors, primary first, each at its own scale or `scale`; why not when the layout
/// breaks MS-RDPEDISP's rules or gives more than `offered` monitors.
pub fn from_display_control(
    layout: &DisplayControlMonitorLayout,
    offered: u32,
    scale: u32,
) -> Result<Vec<Monitor>, String> {
    let count = layout.monitors().len();
    if count > usize::try_from(offered).unwrap_or(usize::MAX) {
        return Err(format!("{count} monitors, {offered} offered"));
    }
    let capabilities = DisplayControlCapabilities::new(offered, MONITOR_AREA.0, MONITOR_AREA.1);
    let fitted = (capabilities.map_err(|e| e.to_string()))
        .and_then(|c| layout.fit_to_capabilities(&c).map_err(|e| e.to_string()))?;
    let mut monitors = Vec::new();
    for m in fitted.monitors() {
        let (x, y) = m.position().ok_or("the primary monitor is not at 0,0")?;
        let (w, h) = m.dimensions();
        let own = (m.desktop_scale_factor()).filter(|_| m.device_scale_factor().is_some());
        let monitor = (x, y, w.cast_signed(), h.cast_signed(), own.unwrap_or(scale));
        monitors.push((m.is_primary(), monitor));
    }
    monitors.sort_by_key(|(primary, _)| !primary);
    Ok(monitors.into_iter().map(|(_, m)| m).collect())
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

    /// A Display Control layout of `monitors` (primary, x, y, width, height) as a client may
    /// send it, unchecked.
    pub(crate) fn sent(monitors: &[(bool, i32, i32, u32, u32)]) -> DisplayControlMonitorLayout {
        let mut bytes = Vec::new();
        let mut put = |v: u32| bytes.extend_from_slice(&v.to_le_bytes());
        put(40);
        put(u32::try_from(monitors.len()).unwrap());
        for &(primary, x, y, w, h) in monitors {
            for v in [
                u32::from(primary),
                x.cast_unsigned(),
                y.cast_unsigned(),
                w,
                h,
            ] {
                put(v);
            }
            (0..5).for_each(|_| put(0));
        }
        ironrdp_core::decode(&bytes).unwrap()
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
        let passed = Arc::new(Mutex::new(Vec::new()));
        let to = passed.clone();
        first.follow(rx, move |l| to.lock().unwrap().push(l.to_vec()));
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
        assert_eq!(*passed.lock().unwrap(), [last], "and it alone is passed on");
    }

    #[tokio::test(start_paused = true)]
    async fn a_later_viewers_layout_is_ignored() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let later = console.join(SECOND.to_vec());
        assert!(later.lock(), "a later viewer runs in its run's mode");
        assert_eq!(published(&status), FIRST, "its monitors are not published");
        let (layouts, rx) = mpsc::unbounded_channel();
        later.follow(rx, |_| panic!("a later viewer's layout is not passed on"));
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

    #[tokio::test(start_paused = true)]
    async fn the_owner_leaving_mid_burst_publishes_nothing() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let later = console.join(SECOND.to_vec());
        let (layouts, rx) = mpsc::unbounded_channel();
        first.follow(rx, |_| {});
        layouts.send(SECOND.to_vec()).unwrap();
        drop(first);
        tokio::time::sleep(SETTLE * 2).await;
        assert_eq!(
            published(&status),
            FIRST,
            "the layout stays while a viewer does"
        );
        drop(later);

        let first = console.join(FIRST.to_vec());
        let (layouts, rx) = mpsc::unbounded_channel();
        first.follow(rx, |_| {});
        layouts.send(SECOND.to_vec()).unwrap();
        drop(first);
        tokio::time::sleep(SETTLE * 2).await;
        assert!(published(&status).is_empty(), "the ended run stays empty");
    }

    #[test]
    fn a_replacing_viewer_keeps_lock_mode_and_takes_the_layout() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let replacing = console.join(SECOND.to_vec());
        console.take_owner(replacing.id);
        drop(first);
        assert!(replacing.lock(), "the run stays in lock mode");
        assert_eq!(published(&status), FIRST, "the run goes on");
        assert!(console.publish_as(replacing.id, SECOND.to_vec()));
        assert_eq!(published(&status), SECOND);
        assert!(
            !console.publish_as(0, FIRST.to_vec()),
            "the replaced viewer no longer publishes"
        );
        assert_eq!(published(&status), SECOND);
    }

    #[test]
    fn a_take_over_joins_the_run_going_on_as_its_owner() {
        let (console, config, status) = configured(LaptopScreen::Lock, false);
        assert!(console.take_over().is_none(), "no run to take over");
        let first = console.join(FIRST.to_vec());
        assert!(
            !console.would_own(),
            "a joining viewer does not own the run"
        );
        config.send_modify(|c| c.second_client = SecondClient::Replace);
        assert!(console.would_own(), "a replacing one will");
        let replacing = console.take_over().expect("the run is going on");
        drop(first);
        assert!(replacing.lock());
        replacing.take_layout(SECOND.to_vec());
        assert_eq!(published(&status), SECOND, "its monitors are published");
        drop(replacing);
        assert!(published(&status).is_empty(), "the run ends with it");
    }

    #[tokio::test(start_paused = true)]
    async fn a_viewer_that_takes_the_run_over_after_following_publishes() {
        let (console, _config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        let later = console.join(SECOND.to_vec());
        let (layouts, rx) = mpsc::unbounded_channel();
        later.follow(rx, |_| {});
        console.take_owner(later.id);
        drop(first);
        let last = vec![(0, 0, 1440, 900, 100)];
        layouts.send(last.clone()).unwrap();
        tokio::time::sleep(SETTLE * 2).await;
        assert_eq!(published(&status), last);
    }

    #[test]
    fn a_laptop_screen_change_applies_from_the_next_first_viewer() {
        let (console, config, status) = configured(LaptopScreen::Lock, false);
        let first = console.join(FIRST.to_vec());
        config.send_modify(|c| c.laptop_screen = LaptopScreen::Show);
        let later = console.join(SECOND.to_vec());
        assert!(later.lock(), "the run keeps its mode");
        assert_eq!(published(&status), FIRST);
        drop((first, later));
        assert!(!console.would_own(), "with no run, as configured");
        assert!(!console.join(SECOND.to_vec()).lock());
        assert!(published(&status).is_empty());
    }

    #[test]
    fn a_published_layout_gives_client_monitors_with_the_first_primary() {
        use gcc::MonitorFlags;
        assert_eq!(
            client_monitors(&[(0, 0, 1280, 720, 100), (-1024, 0, 1024, 768, 150)]),
            [
                gcc::Monitor {
                    left: 0,
                    top: 0,
                    right: 1279,
                    bottom: 719,
                    flags: MonitorFlags::PRIMARY,
                },
                gcc::Monitor {
                    left: -1024,
                    top: 0,
                    right: -1,
                    bottom: 767,
                    flags: MonitorFlags::empty(),
                },
            ]
        );
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
            from_display_control(&dc, 16, 125).unwrap(),
            [(0, 0, 1920, 1080, 150), (-1280, 0, 1280, 1024, 125)]
        );
    }
}
