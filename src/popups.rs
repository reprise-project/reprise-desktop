//! Reprise's own popups: what a press off one closes, what a motion off one
//! unhovers, which go with their output, and which are up while input is routed.

use smithay::output::Output;

use crate::{compositor::State, input::Hit, taskbar::TaskbarHit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    TrayFlyout,
    TaskPreview,
    SnapPicker,
    Projector,
    NotifyCenter,
    Calendar,
    Displays,
    Menu,
    Launcher,
    StartPanel,
    ClipView,
    EmojiView,
    Switcher,
    Overview,
}

/// What a press's `Hit` alone does not say, read before the press closes anything.
#[derive(Clone, Copy, Debug, Default)]
pub struct PressAt {
    /// The press is on a field's edit menu.
    pub edit_menu: bool,
    /// The press is on the tray's displays cell.
    pub displays_cell: bool,
}

impl Popup {
    /// Every popup, in the order a press or a lost output closes them.
    pub const ALL: [Popup; 14] = [
        Popup::TrayFlyout,
        Popup::TaskPreview,
        Popup::SnapPicker,
        Popup::Projector,
        Popup::NotifyCenter,
        Popup::Calendar,
        Popup::Displays,
        Popup::Menu,
        Popup::Launcher,
        Popup::StartPanel,
        Popup::ClipView,
        Popup::EmojiView,
        Popup::Switcher,
        Popup::Overview,
    ];

    /// The popups up now.
    pub fn open(state: &State) -> impl Iterator<Item = Popup> + '_ {
        Self::ALL.into_iter().filter(|p| p.is_open(state))
    }

    pub fn is_open(self, state: &State) -> bool {
        match self {
            Popup::TrayFlyout => crate::trayflyout::open_on(state).is_some(),
            Popup::TaskPreview => state.task_preview.is_shown(),
            Popup::SnapPicker => state.snap_picker.is_shown(),
            Popup::Projector => state.projector.is_open(),
            Popup::NotifyCenter => crate::notifycenter::is_open(state),
            Popup::Calendar => crate::calendarview::is_open(state),
            Popup::Displays => crate::displaysview::is_open(state),
            Popup::Menu => !state.menus.is_empty(),
            Popup::Launcher => state.launcher.is_some(),
            Popup::StartPanel => state.start_panel.is_some(),
            Popup::ClipView => state.panels.clipview.is_some(),
            Popup::EmojiView => state.panels.emojiview.is_some(),
            Popup::Switcher => state.switcher.is_open(),
            Popup::Overview => state.overview.view.is_some(),
        }
    }

    /// The output the popup is kept on; None while it is down, and always for the overview, which
    /// covers every output.
    pub fn output(self, state: &State) -> Option<Output> {
        match self {
            Popup::TrayFlyout => crate::trayflyout::open_on(state),
            Popup::TaskPreview => crate::taskpreview::open_on(state),
            Popup::SnapPicker => crate::snappickerview::open_on(state),
            Popup::Projector => crate::projectorview::open_on(state),
            Popup::NotifyCenter => crate::notifycenter::open_on(state),
            Popup::Calendar => crate::calendarview::open_on(state),
            Popup::Displays => crate::displaysview::open_on(state),
            Popup::Menu => state.menus.first().map(|m| m.output.clone()),
            Popup::Launcher => crate::launcherview::open_on(state),
            Popup::StartPanel => crate::startpanelview::open_on(state),
            Popup::ClipView => crate::clipview::open_on(state),
            Popup::EmojiView => crate::emojiview::open_on(state),
            Popup::Switcher => state.switcher.open_on(),
            Popup::Overview => None,
        }
    }

    /// Close every popup kept on an output the scene no longer has, in `ALL`'s order: it would
    /// stay at stale coordinates, taking the keys.
    pub fn close_off_scene(state: &mut State) {
        for p in Self::ALL {
            if p.output(state)
                .is_some_and(|o| !state.scene.outputs().contains(&o))
            {
                p.close(state);
            }
        }
    }

    /// Whether the popup takes every press and motion while it is up, so the
    /// pointer reaches nothing around it.
    pub fn owns_pointer(self) -> bool {
        matches!(self, Popup::Switcher | Popup::Overview)
    }

    /// Whether `hit` lies on this popup.
    pub fn hit_is(self, hit: &Hit) -> bool {
        match self {
            Popup::TrayFlyout => matches!(hit, Hit::TrayFlyout { .. }),
            Popup::TaskPreview => matches!(hit, Hit::TaskPreview { .. }),
            Popup::SnapPicker => matches!(hit, Hit::SnapPicker { .. }),
            Popup::Projector => matches!(hit, Hit::Projector { .. }),
            Popup::NotifyCenter => matches!(hit, Hit::NotifyCenter),
            Popup::Calendar => matches!(hit, Hit::Calendar { .. }),
            Popup::Displays => matches!(hit, Hit::Displays { .. }),
            Popup::Menu => matches!(hit, Hit::Menu { .. }),
            Popup::Launcher => matches!(hit, Hit::Launcher { .. }),
            Popup::StartPanel => matches!(hit, Hit::StartPanel { .. }),
            Popup::ClipView => matches!(hit, Hit::ClipView { .. }),
            Popup::EmojiView => matches!(hit, Hit::EmojiView { .. }),
            Popup::Switcher => matches!(hit, Hit::Switcher { .. }),
            Popup::Overview => matches!(hit, Hit::Overview { .. }),
        }
    }

    /// Whether a press on `hit`, off the popup, leaves it open: a menu over
    /// it, or the cell that toggles it itself.
    pub fn keeps_on(self, hit: &Hit, at: PressAt) -> bool {
        let start = matches!(
            hit,
            Hit::Taskbar {
                hit: TaskbarHit::Start,
                ..
            }
        );
        let tray = matches!(
            hit,
            Hit::Taskbar {
                hit: TaskbarHit::Tray(_),
                ..
            }
        );
        let menu = matches!(hit, Hit::Menu { .. });
        match self {
            Popup::TrayFlyout
            | Popup::TaskPreview
            | Popup::SnapPicker
            | Popup::Projector
            | Popup::Calendar => false,
            // The bell toggles it (`tray::click`).
            Popup::NotifyCenter => menu || tray,
            // `builtin::click` toggles it.
            Popup::Displays => at.displays_cell,
            // Start toggles its menu; a tray cell decides itself (`tray::click`).
            Popup::Menu => start || tray,
            Popup::Launcher => menu,
            Popup::StartPanel => menu || start,
            Popup::ClipView | Popup::EmojiView => at.edit_menu,
            // `handle_button` and `access::click` give it the press first.
            Popup::Switcher | Popup::Overview => true,
        }
    }

    pub fn close(self, state: &mut State) {
        match self {
            Popup::TrayFlyout => crate::trayflyout::close(state),
            Popup::TaskPreview => crate::taskpreview::hide(state),
            Popup::SnapPicker => crate::snappickerview::hide(state),
            Popup::Projector => crate::projectorview::hide(state),
            Popup::NotifyCenter => crate::notifycenter::close(state),
            Popup::Calendar => crate::calendarview::close(state),
            Popup::Displays => crate::displaysview::close(state),
            Popup::Menu => state.close_menu(),
            Popup::Launcher => state.close_launcher(),
            Popup::StartPanel => state.close_start_panel(),
            Popup::ClipView => state.close_clipview(),
            Popup::EmojiView => state.close_emoji(),
            Popup::Switcher => state.switcher_cancel(),
            Popup::Overview => state.close_overview(),
        }
    }

    /// Whether a motion onto `hit`, off the popup, keeps its hover: the
    /// button that opened it, which the pointer crosses on the way over.
    pub fn hover_keeps_on(self, hit: &Hit) -> bool {
        match self {
            Popup::SnapPicker => matches!(hit, Hit::Frame { .. }),
            Popup::TaskPreview => matches!(hit, Hit::Taskbar { .. }),
            _ => false,
        }
    }

    /// The pointer left the popup: nothing on it stays lit. Called whether or
    /// not it is up, since some disarm a popup still waiting to show.
    pub fn unhover(self, state: &mut State) {
        match self {
            Popup::TrayFlyout => crate::trayflyout::hover(state, None),
            Popup::TaskPreview => crate::taskpreview::hide(state),
            Popup::SnapPicker => crate::snappickerview::leave(state),
            Popup::Projector => crate::projectorview::hover(state, None),
            Popup::Calendar => crate::calendarview::hover(state, None),
            Popup::Displays => crate::displaysview::hover(state, None),
            Popup::Menu => state.clear_menu_hover(),
            Popup::StartPanel => state.start_panel_pointer(None),
            Popup::Switcher => state.switcher_hover(None),
            Popup::Overview => state.overview_hover(None),
            Popup::NotifyCenter | Popup::Launcher | Popup::ClipView | Popup::EmojiView => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::SceneOwner;

    /// The popup a scene tree tagged `owner` belongs to; exhaustive so a new
    /// owner must be placed here.
    fn of(owner: SceneOwner) -> Option<Popup> {
        match owner {
            SceneOwner::Menu(_) => Some(Popup::Menu),
            SceneOwner::Launcher => Some(Popup::Launcher),
            SceneOwner::StartPanel => Some(Popup::StartPanel),
            SceneOwner::Switcher => Some(Popup::Switcher),
            SceneOwner::Overview => Some(Popup::Overview),
            SceneOwner::TrayFlyout => Some(Popup::TrayFlyout),
            SceneOwner::TaskPreview => Some(Popup::TaskPreview),
            SceneOwner::SnapPicker => Some(Popup::SnapPicker),
            SceneOwner::Projector => Some(Popup::Projector),
            SceneOwner::NotifyCenter => Some(Popup::NotifyCenter),
            SceneOwner::ClipView => Some(Popup::ClipView),
            SceneOwner::EmojiView => Some(Popup::EmojiView),
            SceneOwner::Calendar => Some(Popup::Calendar),
            SceneOwner::Displays => Some(Popup::Displays),
            // Shown beside the pointer's work, never closed by a press
            // elsewhere: the balloons, the OSD, the debug panel, the shortcut sheet.
            SceneOwner::Notification
            | SceneOwner::Osd
            | SceneOwner::Debug
            | SceneOwner::KeySheet => None,
            SceneOwner::Window(_)
            | SceneOwner::Taskbar(_)
            | SceneOwner::Desktop
            | SceneOwner::DeskWidget(_)
            | SceneOwner::Layer
            | SceneOwner::Dialog(_)
            | SceneOwner::MiniView(_) => None,
        }
    }

    #[test]
    fn every_panel_owner_is_one_popup_and_every_popup_has_an_owner() {
        let table = [
            (SceneOwner::Menu(0), Some(Popup::Menu)),
            (SceneOwner::Launcher, Some(Popup::Launcher)),
            (SceneOwner::StartPanel, Some(Popup::StartPanel)),
            (SceneOwner::Switcher, Some(Popup::Switcher)),
            (SceneOwner::Overview, Some(Popup::Overview)),
            (SceneOwner::TrayFlyout, Some(Popup::TrayFlyout)),
            (SceneOwner::TaskPreview, Some(Popup::TaskPreview)),
            (SceneOwner::SnapPicker, Some(Popup::SnapPicker)),
            (SceneOwner::Projector, Some(Popup::Projector)),
            (SceneOwner::NotifyCenter, Some(Popup::NotifyCenter)),
            (SceneOwner::ClipView, Some(Popup::ClipView)),
            (SceneOwner::EmojiView, Some(Popup::EmojiView)),
            (SceneOwner::Calendar, Some(Popup::Calendar)),
            (SceneOwner::Displays, Some(Popup::Displays)),
            (SceneOwner::Notification, None),
            (SceneOwner::Osd, None),
            (SceneOwner::Debug, None),
            (SceneOwner::KeySheet, None),
            (SceneOwner::Desktop, None),
            (SceneOwner::Layer, None),
            (SceneOwner::Dialog(0), None),
        ];
        for (owner, popup) in table {
            assert_eq!(of(owner), popup, "{owner:?}");
        }
        for p in Popup::ALL {
            let owners = table.iter().filter(|(_, q)| *q == Some(p)).count();
            assert_eq!(owners, 1, "{p:?} has one scene owner");
        }
    }

    #[test]
    fn each_popup_is_hit_on_itself_alone() {
        let hits = [
            Hit::TrayFlyout { cell: None },
            Hit::TaskPreview { cell: None },
            Hit::SnapPicker { zone: None },
            Hit::Projector { tile: None },
            Hit::NotifyCenter,
            Hit::Calendar {
                hit: reprisekit::controls::calendar::PopupHit::Panel,
            },
            Hit::Displays {
                hit: reprisekit::controls::displaypanel::Hit::Panel,
            },
            Hit::Menu {
                depth: 0,
                row: None,
            },
            Hit::Launcher {
                hit: reprisekit::controls::launcher::Hit::Panel,
            },
            Hit::StartPanel { spot: None },
            Hit::ClipView {
                hit: reprisekit::controls::cliphistory::Hit::Panel,
            },
            Hit::EmojiView {
                hit: reprisekit::controls::emojipicker::Hit::Panel,
            },
            Hit::Switcher { cell: None },
            Hit::Overview { cell: None },
        ];
        for (i, p) in Popup::ALL.into_iter().enumerate() {
            for (j, h) in hits.iter().enumerate() {
                assert_eq!(p.hit_is(h), i == j, "{p:?} on {h:?}");
            }
        }
    }

    /// Opens `p` on `two`, the output under the pointer, as its bind or its cell does; the task
    /// preview waits for its delay.
    fn open_on(h: &mut crate::harness::Harness, p: Popup, two: &Output) {
        use crate::input::BTN_LEFT;
        let bar = (h.state.bars.iter().find(|b| &b.output == two))
            .expect("TEST-2's bar")
            .tree;
        let r = h.state.layout_box(Some(two));
        let cell = crate::raster::Rect::new(r.x + r.width - 24, r.y + r.height - 28, 16, 28);
        let s = &mut h.state;
        match p {
            Popup::TrayFlyout => {
                let item = crate::tray::TrayItem {
                    id: "probe".into(),
                    key: "probe".into(),
                    ..Default::default()
                };
                s.tray.as_mut().expect("the tray").hidden.push(item);
                crate::trayflyout::chevron_press(s, bar, BTN_LEFT, None);
            }
            Popup::TaskPreview => crate::taskpreview::hover(s, bar, TaskbarHit::Task(0)),
            Popup::SnapPicker => crate::snappickerview::toggle(s),
            Popup::Projector => crate::projectorview::bind(s, crate::keys::MOD_LOGO),
            Popup::NotifyCenter => crate::notifycenter::toggle(s, two, cell),
            Popup::Calendar => crate::calendarview::clock_press(s, bar, (cell.x, cell.y), 0, false),
            Popup::Displays => crate::displaysview::toggle(s, two, cell),
            Popup::Menu => {
                let w = s.focused.expect("a focused window");
                s.open_window_menu_below(w, false);
            }
            Popup::Launcher => s.toggle_launcher(),
            Popup::StartPanel => s.toggle_start_panel(bar),
            Popup::ClipView => s.toggle_clipview(),
            Popup::EmojiView => s.toggle_emoji(),
            Popup::Switcher => s.open_switcher(crate::keys::MOD_ALT),
            Popup::Overview => s.open_overview(),
        }
        h.dispatch();
    }

    #[test]
    fn moving_the_taskbar_closes_every_popup() {
        for p in Popup::ALL {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            let two = h.output("TEST-2", 1024, 768);
            let r = h.state.layout_box(Some(&two));
            let _tray = crate::tray::headless(&mut h.state);
            let mut c = h.connect();
            for app in ["one", "two"] {
                let w = h.map_toplevel(&mut c, app, 300, 200);
                h.state.move_to(w.id, r.x + 100, r.y + 100);
            }
            h.pointer_to(f64::from(r.x + 700), f64::from(r.y + 400));
            open_on(&mut h, p, &two);
            assert!(
                p.is_open(&h.state) || p.output(&h.state).is_some(),
                "{p:?} opened"
            );
            let mut top = h.state.config.clone();
            top.taskbar.position = reprisekit::taskbar::Edge::Top;
            crate::configwatch::reload(&mut h.state, top);
            h.dispatch();
            assert!(!p.is_open(&h.state), "{p:?} closed with the bar's move");
            assert_eq!(p.output(&h.state), None, "{p:?} waits on no output");
        }
    }

    #[test]
    fn removing_an_output_closes_every_popup_on_it_and_keys_reach_the_window_again() {
        for p in Popup::ALL {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            let two = h.output("TEST-2", 1024, 768);
            let r = h.state.layout_box(Some(&two));
            let _tray = crate::tray::headless(&mut h.state);
            let mut c = h.connect();
            for app in ["one", "two"] {
                let w = h.map_toplevel(&mut c, app, 300, 200);
                h.state.move_to(w.id, r.x + 100, r.y + 100);
            }
            h.pointer_to(f64::from(r.x + 700), f64::from(r.y + 400));
            open_on(&mut h, p, &two);
            if p == Popup::Overview {
                assert!(p.is_open(&h.state), "the overview opened");
            } else {
                assert_eq!(
                    p.output(&h.state).as_ref(),
                    Some(&two),
                    "{p:?} is on TEST-2"
                );
            }
            h.state.scene.remove_output(&two);
            h.state.output_removed(&two);
            h.dispatch();
            assert!(!p.is_open(&h.state), "{p:?} closed with TEST-2");
            assert_eq!(p.output(&h.state), None, "{p:?} keeps no output");
            h.roundtrip(&mut c);
            c.side.keys.clear();
            h.key(30, true);
            h.key(30, false);
            h.roundtrip(&mut c);
            assert_eq!(
                c.side.keys,
                [(30, true), (30, false)],
                "after {p:?}: the window hears A"
            );
        }
    }
}
