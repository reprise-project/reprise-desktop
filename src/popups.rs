//! Reprise's own popups: what a press off one closes, what a motion off one
//! unhovers, and which are up while a finger or the pointer is routed.

use crate::{compositor::State, input::Hit, taskbar::TaskbarHit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    TrayFlyout,
    TaskPreview,
    SnapPicker,
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
    /// Every popup, in the order a press closes them.
    pub const ALL: [Popup; 13] = [
        Popup::TrayFlyout,
        Popup::TaskPreview,
        Popup::SnapPicker,
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
            Popup::TrayFlyout | Popup::TaskPreview | Popup::SnapPicker | Popup::Calendar => false,
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
            SceneOwner::NotifyCenter => Some(Popup::NotifyCenter),
            SceneOwner::ClipView => Some(Popup::ClipView),
            SceneOwner::EmojiView => Some(Popup::EmojiView),
            SceneOwner::Calendar => Some(Popup::Calendar),
            SceneOwner::Displays => Some(Popup::Displays),
            // Shown beside the pointer's work, never closed by a press
            // elsewhere: the balloons, the OSD, the debug panel.
            SceneOwner::Notification | SceneOwner::Osd | SceneOwner::Debug => None,
            SceneOwner::Window(_)
            | SceneOwner::Taskbar(_)
            | SceneOwner::Desktop
            | SceneOwner::Layer
            | SceneOwner::Dialog(_) => None,
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
            (SceneOwner::NotifyCenter, Some(Popup::NotifyCenter)),
            (SceneOwner::ClipView, Some(Popup::ClipView)),
            (SceneOwner::EmojiView, Some(Popup::EmojiView)),
            (SceneOwner::Calendar, Some(Popup::Calendar)),
            (SceneOwner::Displays, Some(Popup::Displays)),
            (SceneOwner::Notification, None),
            (SceneOwner::Osd, None),
            (SceneOwner::Debug, None),
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
                hit: reprisekit::controls::cliphistory::Hit::ScrollUp,
            },
            Hit::EmojiView {
                hit: reprisekit::controls::emojipicker::Hit::ScrollUp,
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
}
