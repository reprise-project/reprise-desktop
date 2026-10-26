//! What a window's title, app id or icon change redraws, and how often
//! (`State::flush_titles`): a repeat of the same title costs a string
//! compare, a new title redraws the caption strip, and the taskbars redraw
//! only when the window's task button shows what changed.

use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::window::WindowId;

/// The least time between two title-driven redraws of one window; the last
/// title always shows, from a timer.
pub const MIN_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub title: bool,
    pub app_id: bool,
    pub icon: bool,
}

impl Changes {
    pub const ICON: Changes = Changes {
        title: false,
        app_id: false,
        icon: true,
    };

    pub fn any(self) -> bool {
        self.title || self.app_id || self.icon
    }

    pub fn merge(&mut self, other: Changes) {
        self.title |= other.title;
        self.app_id |= other.app_id;
        self.icon |= other.icon;
    }
}

/// The title and app id a window's last change carried; `note` tells a real
/// change from a repeat `set_title`.
#[derive(Debug, Default)]
pub struct Seen {
    title: String,
    app_id: String,
}

impl Seen {
    /// Keep the window's current strings; returns which differ from the last ones kept.
    pub fn note(&mut self, title: &str, app_id: &str) -> Changes {
        let changes = Changes {
            title: self.title != title,
            app_id: self.app_id != app_id,
            icon: false,
        };
        if changes.title {
            title.clone_into(&mut self.title);
        }
        if changes.app_id {
            app_id.clone_into(&mut self.app_id);
        }
        changes
    }
}

/// How the window's task button, as the bars last laid it out, shows it,
/// ordered by how much of the window it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskLabel {
    /// No button: off the taskbar, unmapped or on another desktop.
    Absent,
    /// An icon-only button, or a group's "Name (N)" (named by app id):
    /// the window's title is not drawn.
    Untitled,
    /// A button of its own, labelled with the title.
    Titled,
}

impl TaskLabel {
    /// The label of a window by its `button`: how many windows it holds and
    /// whether its bar's row is icons only; None: no button.
    fn of(button: Option<(usize, bool)>) -> Self {
        match button {
            None => Self::Absent,
            Some((1, false)) => Self::Titled,
            Some(_) => Self::Untitled,
        }
    }

    /// The label by the buttons every bar holding the window lays for it:
    /// Titled when any bar titles it.
    fn across(buttons: impl IntoIterator<Item = (usize, bool)>) -> Self {
        buttons
            .into_iter()
            .map(|b| Self::of(Some(b)))
            .max()
            .unwrap_or(Self::Absent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redraw {
    /// The frame's top strip: the caption's title and icon.
    pub caption: bool,
    /// Every output's bar.
    pub taskbars: bool,
}

/// What `c` redraws for a window whose button shows it as `label`. The
/// caption icon falls back on the app id, so any change redraws the
/// caption; an app id change regroups and re-pins the button.
pub fn plan(c: Changes, label: TaskLabel) -> Redraw {
    let taskbars = match label {
        TaskLabel::Absent => false,
        TaskLabel::Untitled => c.icon || c.app_id,
        TaskLabel::Titled => c.any(),
    };
    Redraw {
        caption: c.any(),
        taskbars,
    }
}

/// How long a window last redrawn for a title at `last` still waits at `now`; None draws it now.
pub fn wait(last: Option<Instant>, now: Instant) -> Option<Duration> {
    let since = now.saturating_duration_since(last?);
    MIN_INTERVAL.checked_sub(since).filter(|d| !d.is_zero())
}

/// A window's titles as the chrome last saw them, and the changes not drawn yet.
#[derive(Debug, Default)]
pub struct WindowTitles {
    seen: Seen,
    pending: Changes,
    drawn_at: Option<Instant>,
}

/// The flush queued for the pending changes: an idle pass, and the timer for windows `wait` held back.
#[derive(Default)]
pub struct Flush {
    idle: bool,
    timer: Option<RegistrationToken>,
}

impl State {
    /// `w`'s title or app id may have changed: queue what a real change
    /// redraws.
    pub fn note_title(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let (title, app_id) = (win.title(), win.app_id());
        let changes = win.titles.seen.note(&title, &app_id);
        self.toplevels.dirty = true;
        if changes.title {
            self.tab_retitled(w);
        }
        self.retitled(w, changes);
    }

    /// `w`'s title, app id or icon changed: redraw what shows them and
    /// update its foreign-toplevel handle on the next idle pass;
    /// `flush_titles` spaces the redraws.
    pub fn retitled(&mut self, w: WindowId, changes: Changes) {
        if !changes.any() {
            return;
        }
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.titles.pending.merge(changes);
        if !std::mem::replace(&mut self.titles.idle, true) {
            self.handle.insert_idle(|state| {
                state.titles.idle = false;
                state.flush_titles();
            });
        }
    }

    /// Redraw for the pending changes (`plan`): the caption strip, and the
    /// bars once if any window's button shows what changed. A window
    /// redrawn under `MIN_INTERVAL` ago waits for the timer.
    fn flush_titles(&mut self) {
        let now = Instant::now();
        let pending: Vec<WindowId> = (self.windows.iter())
            .filter(|w| w.titles.pending.any())
            .map(|w| w.id)
            .collect();
        let mut next: Option<Duration> = None;
        let mut bars = false;
        let mut drawn = false;
        for w in pending {
            let label = self.task_label(w);
            let Some(win) = self.win_mut(w) else {
                continue;
            };
            if let Some(d) = wait(win.titles.drawn_at, now) {
                next = Some(next.map_or(d, |n| n.min(d)));
                continue;
            }
            let changes = std::mem::take(&mut win.titles.pending);
            win.titles.drawn_at = Some(now);
            let mapped = win.mapped;
            drawn = true;
            if changes.title || changes.app_id {
                self.sync_toplevel(w);
            }
            let plan = plan(changes, label);
            if plan.caption && mapped {
                self.redraw_caption(w);
            }
            bars |= plan.taskbars;
        }
        if bars {
            self.redraw_taskbars();
        } else if drawn {
            // A task button's accessible name is the title, drawn or not. A caption asks its
            // window's outputs itself (`redraw_caption`).
            crate::access::touch(self);
        }
        if let Some(t) = self.titles.timer.take() {
            self.handle.remove(t);
        }
        if let Some(d) = next {
            let token = self
                .handle
                .insert_source(Timer::from_duration(d), |_, _, state| {
                    state.titles.timer = None;
                    state.flush_titles();
                    TimeoutAction::Drop
                });
            match token {
                Ok(t) => self.titles.timer = Some(t),
                Err(e) => log::warn!("title timer: {}", e.error),
            }
        }
    }

    fn task_label(&self, w: WindowId) -> TaskLabel {
        TaskLabel::across((self.bars.iter()).filter_map(|b| {
            let group = b.buttons.iter().find(|g| g.contains(&w))?;
            Some((group.len(), b.layout.icons_only))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: Changes = Changes {
        title: true,
        app_id: false,
        icon: false,
    };
    const APP_ID: Changes = Changes {
        title: false,
        app_id: true,
        icon: false,
    };

    #[test]
    fn a_button_of_its_own_is_titled_unless_its_row_is_icons_only() {
        assert_eq!(TaskLabel::of(None), TaskLabel::Absent);
        assert_eq!(TaskLabel::of(Some((1, false))), TaskLabel::Titled);
        assert_eq!(
            TaskLabel::of(Some((1, true))),
            TaskLabel::Untitled,
            "a combined row or a side column too narrow for a title draws none"
        );
        assert_eq!(
            TaskLabel::of(Some((3, false))),
            TaskLabel::Untitled,
            "a group is named by its app"
        );
    }

    #[test]
    fn a_window_is_titled_when_any_bar_titles_its_button() {
        assert_eq!(TaskLabel::across([]), TaskLabel::Absent);
        assert_eq!(
            TaskLabel::across([(1, true), (1, false)]),
            TaskLabel::Titled,
            "an icon-only column on one output, a labelled bar on another"
        );
        assert_eq!(
            TaskLabel::across([(1, false), (1, true)]),
            TaskLabel::Titled
        );
        assert_eq!(
            TaskLabel::across([(1, true), (2, false)]),
            TaskLabel::Untitled,
            "no bar draws its title"
        );
    }

    #[test]
    fn a_repeated_title_is_no_change() {
        let mut seen = Seen::default();
        assert_eq!(
            seen.note("same", "foot"),
            Changes {
                title: true,
                app_id: true,
                icon: false
            }
        );
        assert_eq!(seen.note("same", "foot"), Changes::default());
        assert!(!seen.note("same", "foot").any());
    }

    #[test]
    fn a_new_title_or_app_id_is_a_change() {
        let mut seen = Seen::default();
        seen.note("one", "foot");
        assert_eq!(seen.note("two", "foot"), TITLE);
        assert_eq!(seen.note("two", "kitty"), APP_ID);
        assert_eq!(
            seen.note("one", "foot"),
            Changes {
                title: true,
                app_id: true,
                icon: false
            }
        );
    }

    #[test]
    fn nothing_changed_redraws_nothing() {
        for label in [TaskLabel::Absent, TaskLabel::Untitled, TaskLabel::Titled] {
            assert_eq!(
                plan(Changes::default(), label),
                Redraw {
                    caption: false,
                    taskbars: false
                }
            );
        }
    }

    #[test]
    fn a_title_redraws_the_bars_only_where_the_button_shows_it() {
        let caption_only = Redraw {
            caption: true,
            taskbars: false,
        };
        assert_eq!(plan(TITLE, TaskLabel::Absent), caption_only);
        assert_eq!(plan(TITLE, TaskLabel::Untitled), caption_only);
        assert_eq!(
            plan(TITLE, TaskLabel::Titled),
            Redraw {
                caption: true,
                taskbars: true
            }
        );
    }

    #[test]
    fn an_icon_or_app_id_redraws_any_button() {
        let both = Redraw {
            caption: true,
            taskbars: true,
        };
        for c in [Changes::ICON, APP_ID] {
            assert_eq!(plan(c, TaskLabel::Untitled), both);
            assert_eq!(plan(c, TaskLabel::Titled), both);
            assert_eq!(
                plan(c, TaskLabel::Absent),
                Redraw {
                    caption: true,
                    taskbars: false
                }
            );
        }
    }

    #[test]
    fn redraws_are_spaced_by_the_interval() {
        let t0 = Instant::now();
        assert_eq!(wait(None, t0), None, "a first change draws at once");
        assert_eq!(wait(Some(t0), t0), Some(MIN_INTERVAL));
        let later = t0 + Duration::from_millis(30);
        assert_eq!(wait(Some(t0), later), Some(Duration::from_millis(70)));
        assert_eq!(wait(Some(t0), t0 + MIN_INTERVAL), None);
        assert_eq!(wait(Some(t0), t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn changes_merge() {
        let mut c = TITLE;
        c.merge(Changes::ICON);
        assert_eq!(
            c,
            Changes {
                title: true,
                app_id: false,
                icon: true
            }
        );
    }

    #[test]
    fn a_caption_retitled_asks_for_frames_only_on_its_windows_output() {
        let config = crate::config::Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "foot", 300, 200);
        let tree = h.state.win(t.id).unwrap().tree;
        let on: Vec<String> = h
            .state
            .scene
            .outputs_of(tree)
            .iter()
            .map(|o| o.name())
            .collect();
        assert_eq!(on, ["TEST-1"]);
        assert_eq!(
            h.state.task_label(t.id),
            TaskLabel::Untitled,
            "Glass bars draw no title"
        );
        h.run_for(MIN_INTERVAL.as_millis() as u64 + 20);
        crate::backend::take_asks();
        h.state.retitled(t.id, TITLE);
        h.dispatch();
        let asked = crate::backend::take_asks();
        assert!(!asked.is_empty(), "the caption asks for a frame");
        assert!(
            asked.iter().all(|(o, _)| o == "TEST-1"),
            "only TEST-1: {asked:?}"
        );
    }
}
