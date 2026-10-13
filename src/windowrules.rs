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
    /// A normal frame kept at another text size: `size` is left to the program.
    pub rescaled: Option<Rescaled>,
}

/// A normal frame kept at another text size than now. The window opens at its program's own
/// size, then takes that size held between the frame it was kept at and that frame scaled by
/// the ratio of the two text sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rescaled {
    pub to: (i32, i32),
    pub kept: (i32, i32),
}

impl Rescaled {
    /// The frame size of a window that opened at frame size `opening`.
    pub fn size(self, opening: (i32, i32)) -> (i32, i32) {
        let side = |to: i32, kept: i32, opening: i32| opening.clamp(to.min(kept), to.max(kept));
        (
            side(self.to.0, self.kept.0, opening.0),
            side(self.to.1, self.kept.1, opening.1),
        )
    }
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
    /// `[accessibility] text_size` when the frame was kept.
    #[serde(default = "full_size")]
    pub text_size: u32,
}

fn full_size() -> u32 {
    100
}

impl Remembered {
    /// Where the app's next window opens at `text_size`: a frame kept at another text size
    /// scales by the ratio of the two, so a window that grew or shrank with the text does so
    /// again, but no further than its program's own size ([`Rescaled`]). A maximized one
    /// restores to the scaled size.
    pub fn want(&self, first: bool, text_size: u32) -> Want {
        let kept = (self.width, self.height);
        let rescaled = (self.text_size != text_size).then(|| {
            let scale =
                |v: i32| (i64::from(v) * i64::from(text_size) / i64::from(self.text_size)) as i32;
            Rescaled {
                to: (scale(kept.0), scale(kept.1)),
                kept,
            }
        });
        let maximized = first && self.maximized;
        Want {
            size: match rescaled {
                Some(s) if maximized => Some(s.to),
                Some(_) => None,
                None => Some(kept),
            },
            rescaled: rescaled.filter(|_| !maximized),
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
            if r.size.is_some() {
                w.rescaled = None;
            }
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
            text_size: 100,
        };
        assert_eq!(
            m.want(true, 100),
            Want {
                size: Some((640, 480)),
                position: Some(Position::At(100, 80)),
                output: Some("DP-1".into()),
                state: Some(WindowState::Maximized),
                ..Want::default()
            }
        );
        assert_eq!(
            m.want(false, 100),
            Want {
                size: Some((640, 480)),
                position: Some(Position::Cascade),
                ..Want::default()
            }
        );
    }

    #[test]
    fn a_frame_kept_at_another_text_size_scales_toward_the_programs_size() {
        let m = Remembered {
            output: "DP-1".into(),
            x: 100,
            y: 80,
            width: 1088,
            height: 976,
            maximized: false,
            text_size: 200,
        };
        let opened = |pct: u32, opening: (i32, i32)| {
            let want = m.want(true, pct);
            want.size.or(want.rescaled.map(|s| s.size(opening)))
        };
        assert_eq!(opened(100, (500, 400)), Some((544, 488)));
        assert_eq!(opened(150, (500, 400)), Some((816, 732)));
        assert_eq!(opened(200, (500, 400)), Some((1088, 976)), "the same size");
        assert_eq!(
            opened(100, (766, 709)),
            Some((766, 709)),
            "no smaller than the program opens at"
        );
        assert_eq!(
            opened(100, (1200, 709)),
            Some((1088, 709)),
            "nor larger than it was kept"
        );
        assert_eq!(
            m.want(true, 100).size,
            None,
            "the program picks its size first"
        );
        let maximized = Remembered {
            maximized: true,
            ..m.clone()
        };
        assert_eq!(maximized.want(true, 100).size, Some((544, 488)));
        assert_eq!(maximized.want(true, 100).rescaled, None);
        let rule = WindowRule {
            size: Some((800, 600)),
            ..rule("app")
        };
        let ruled = want(&[rule], "app", "", m.want(true, 100));
        assert_eq!(
            (ruled.size, ruled.rescaled),
            (Some((800, 600)), None),
            "a rule's size wins"
        );
        let small = Remembered {
            text_size: 100,
            ..m.clone()
        };
        let grown = |opening: (i32, i32)| {
            let want = small.want(true, 200);
            want.size.or(want.rescaled.map(|s| s.size(opening)))
        };
        assert_eq!(
            grown((1520, 1100)),
            Some((1520, 1100)),
            "kept smaller: grows"
        );
        assert_eq!(
            grown((500, 400)),
            Some((1088, 976)),
            "no smaller than it was kept"
        );
        assert_eq!(
            grown((3000, 900)),
            Some((2176, 976)),
            "nor larger than twice that"
        );
        let old: Remembered =
            toml::from_str("output = \"DP-1\"\nx = 0\ny = 0\nwidth = 900\nheight = 700\n").unwrap();
        assert_eq!(old.text_size, 100, "a frame kept before the text size was");
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
            text_size: 100,
        }
        .want(true, 100);
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
                rescaled: None,
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
