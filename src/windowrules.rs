//! Window rules (`[[window_rules]]`) and remembered window geometry: what a new window asks of its
//! placement, applied by `window.rs` at map.

use serde::{Deserialize, Serialize};

pub use repriseconf::glob::glob;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Maximized,
    Minimized,
    Fullscreen,
}

impl WindowState {
    pub const ALL: [WindowState; 4] = [
        WindowState::Normal,
        WindowState::Maximized,
        WindowState::Minimized,
        WindowState::Fullscreen,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WindowState::Normal => "normal",
            WindowState::Maximized => "maximized",
            WindowState::Minimized => "minimized",
            WindowState::Fullscreen => "fullscreen",
        }
    }

    pub fn parse(s: &str) -> Option<WindowState> {
        WindowState::ALL.into_iter().find(|w| w.name() == s)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowRule {
    pub app_id: String,
    pub title: Option<String>,
    pub size: Option<(i32, i32)>,
    pub position: Option<(i32, i32)>,
    pub output: Option<String>,
    pub state: Option<WindowState>,
    /// 0-based; the file counts from 1.
    pub desktop: Option<usize>,
    pub always_on_top: Option<bool>,
    pub sticky: Option<bool>,
    /// Windows matching rules with the same name share one tab group.
    pub tab_group: Option<String>,
}

impl WindowRule {
    pub fn matches(&self, app_id: &str, title: &str) -> bool {
        glob(&self.app_id, app_id) && self.title.as_deref().is_none_or(|t| glob(t, title))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    At(i32, i32),
    Cascade,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub size: Option<(i32, i32)>,
    pub position: Option<Position>,
    pub output: Option<String>,
    pub state: Option<WindowState>,
    pub desktop: Option<usize>,
    pub always_on_top: Option<bool>,
    pub sticky: Option<bool>,
    pub tab_group: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub maximized: bool,
}

impl Remembered {
    pub fn want(&self, first: bool) -> Want {
        Want {
            size: Some((self.width, self.height)),
            position: Some(if first {
                Position::At(self.x, self.y)
            } else {
                Position::Cascade
            }),
            output: first.then(|| self.output.clone()),
            state: (first && self.maximized).then_some(WindowState::Maximized),
            ..Want::default()
        }
    }
}

pub fn want(rules: &[WindowRule], app_id: &str, title: &str, base: Want) -> Want {
    rules
        .iter()
        .filter(|r| r.matches(app_id, title))
        .fold(base, |mut w, r| {
            fn over<T: Clone>(slot: &mut Option<T>, v: &Option<T>) {
                if v.is_some() {
                    slot.clone_from(v);
                }
            }
            over(&mut w.size, &r.size);
            over(
                &mut w.position,
                &r.position.map(|(x, y)| Position::At(x, y)),
            );
            over(&mut w.output, &r.output);
            over(&mut w.state, &r.state);
            over(&mut w.desktop, &r.desktop);
            over(&mut w.always_on_top, &r.always_on_top);
            over(&mut w.sticky, &r.sticky);
            over(&mut w.tab_group, &r.tab_group);
            w
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app_id: &str) -> WindowRule {
        WindowRule {
            app_id: app_id.into(),
            ..WindowRule::default()
        }
    }

    #[test]
    fn a_rule_with_a_title_needs_both_to_match() {
        let r = WindowRule {
            title: Some("*Mail*".into()),
            ..rule("firefox")
        };
        assert!(r.matches("firefox", "Inbox - Mail - Firefox"));
        assert!(!r.matches("firefox", "News"));
        assert!(!r.matches("chromium", "Mail"));
        assert!(rule("firefox").matches("firefox", ""));
    }

    #[test]
    fn remembered_geometry_opens_the_first_window_there_and_cascades_the_rest() {
        let m = Remembered {
            output: "DP-1".into(),
            x: 100,
            y: 80,
            width: 640,
            height: 480,
            maximized: true,
        };
        assert_eq!(
            m.want(true),
            Want {
                size: Some((640, 480)),
                position: Some(Position::At(100, 80)),
                output: Some("DP-1".into()),
                state: Some(WindowState::Maximized),
                ..Want::default()
            }
        );
        assert_eq!(
            m.want(false),
            Want {
                size: Some((640, 480)),
                position: Some(Position::Cascade),
                ..Want::default()
            }
        );
    }

    #[test]
    fn rules_beat_memory_field_by_field_and_later_rules_win() {
        let base = Remembered {
            output: "DP-1".into(),
            x: 100,
            y: 80,
            width: 640,
            height: 480,
            maximized: true,
        }
        .want(true);
        let rules = [
            WindowRule {
                size: Some((800, 600)),
                desktop: Some(1),
                ..rule("foot")
            },
            WindowRule {
                state: Some(WindowState::Normal),
                always_on_top: Some(true),
                ..rule("f*")
            },
            WindowRule {
                size: Some((1, 1)),
                ..rule("kitty")
            },
            WindowRule {
                desktop: Some(2),
                title: Some("build".into()),
                tab_group: Some("shells".into()),
                ..rule("foot")
            },
        ];
        let w = want(&rules, "foot", "shell", base.clone());
        assert_eq!(
            w,
            Want {
                size: Some((800, 600)),
                position: Some(Position::At(100, 80)),
                output: Some("DP-1".into()),
                state: Some(WindowState::Normal),
                desktop: Some(1),
                always_on_top: Some(true),
                sticky: None,
                tab_group: None,
            }
        );
        let build = want(&rules, "foot", "build", base);
        assert_eq!(build.desktop, Some(2));
        assert_eq!(build.tab_group.as_deref(), Some("shells"));
        assert_eq!(want(&[], "foot", "", Want::default()), Want::default());
    }

    #[test]
    fn state_names_round_trip() {
        for s in WindowState::ALL {
            assert_eq!(WindowState::parse(s.name()), Some(s));
        }
        assert_eq!(WindowState::parse("Maximized"), None);
    }
}
