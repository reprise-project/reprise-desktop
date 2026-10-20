//! The media keys' on-screen panel: one `reprisekit::controls::osd` box at
//! the bottom centre of the desktop icons' output, showing a report or one
//! line per display, replaced on every key and gone 1.5 s after the last. Its `layers.overlay` tree is tagged
//! `SceneOwner::Osd`: the glass tree, then the buffer, which takes the pointer
//! on the panel only (the shadow passes it through). A click removes the panel.
//! Stays hidden while locked; the key still acted.

use std::time::Duration;

use reprisekit::controls::osd::{self, Meter, Osd};
use reprisekit::look::panel;
use reprisescene::{BufferId, Input, TreeId};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Rectangle;

use crate::compositor::{SceneOwner, State};
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::media::{Report, Value};
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};

const HOLD: Duration = Duration::from_millis(1500);
const GAP: i32 = 24;

#[derive(Default)]
pub struct OsdView {
    panel: Option<Panel>,
    timer: Option<RegistrationToken>,
    /// The value whose display lines are up; None for a report or no panel.
    pub showing: Option<Value>,
}

struct Panel {
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
}

/// The panel for `r`; None for a report that shows no panel of its own.
fn describe(r: &Report) -> Option<(Osd, &'static str)> {
    let meter = |fraction: f64, enabled: bool| Some(Meter { fraction, enabled });
    Some(match r {
        Report::Volume { level, muted } => {
            let icon = if *muted {
                "audio-volume-muted"
            } else if *level < 0.34 {
                "audio-volume-low"
            } else if *level < 0.67 {
                "audio-volume-medium"
            } else {
                "audio-volume-high"
            };
            let title = if *muted { "Muted" } else { "Volume" };
            (
                Osd {
                    title: title.into(),
                    meter: meter(*level, !*muted),
                },
                icon,
            )
        }
        Report::Mic { muted } => (
            Osd {
                title: if *muted {
                    "Microphone muted"
                } else {
                    "Microphone on"
                }
                .into(),
                meter: None,
            },
            if *muted {
                "microphone-sensitivity-muted"
            } else {
                "microphone-sensitivity-high"
            },
        ),
        Report::Kbd { level } => (
            Osd {
                title: "Keyboard backlight".into(),
                meter: meter(*level, true),
            },
            "keyboard-brightness",
        ),
        Report::Player { name, verb } => {
            let verb = match *verb {
                "PlayPause" => "Play / Pause",
                other => other,
            };
            (
                Osd {
                    title: format!("{name}: {verb}"),
                    meter: None,
                },
                "multimedia-player",
            )
        }
        Report::Wifi { on } => (
            Osd {
                title: if *on { "Wireless on" } else { "Wireless off" }.into(),
                meter: None,
            },
            "network-wireless",
        ),
        Report::Display(_) => return None,
    })
}

/// Where a panel of `size` goes in `usable`: centred `GAP` above its bottom edge, top-left kept inside.
fn panel_origin(usable: Rect, size: (i32, i32)) -> (i32, i32) {
    let x = usable.x + (usable.width - size.0) / 2;
    let y = usable.bottom() - size.1 - GAP;
    (x.max(usable.x), y.max(usable.y))
}

impl State {
    /// Show (or replace) the panel for `r` and restart the hide timer.
    pub fn show_osd(&mut self, r: &Report) {
        let Some((osd, icon_name)) = describe(r) else {
            return;
        };
        let Some((theme, scale)) = self.osd_theme() else {
            return;
        };
        let icon = self
            .icons
            .get(icon_name, 32 * scale)
            .unwrap_or_else(|| reprisekit::icon::fallback(&theme, 32 * scale));
        let canvas = osd::draw(&theme, self.text.for_output(theme.scale), &osd, Some(&icon));
        if self.show_canvas(canvas, (osd::W, osd::H), true) {
            self.osd.showing = None;
        }
    }

    /// Show (or replace) one line per display for `value` and restart the hide timer.
    pub fn show_display_osd(&mut self, value: Value) {
        if self.draw_display_osd(value, true) {
            self.osd.showing = Some(value);
        }
    }

    /// Redraw the display lines in place, keeping the hide timer, while they show.
    pub fn update_display_osd(&mut self) {
        if let Some(value) = self.osd.showing {
            self.draw_display_osd(value, false);
        }
    }

    fn draw_display_osd(&mut self, value: Value, restart_timer: bool) -> bool {
        let Some((theme, _)) = self.osd_theme() else {
            return false;
        };
        let lines = self.displays.lines(value);
        let size = osd::lines_size(lines.lines.len());
        let canvas = osd::draw_lines(&theme, self.text.for_output(theme.scale), &lines);
        self.show_canvas(canvas, size, restart_timer)
    }

    /// The theme at the OSD output's scale, and that scale.
    fn osd_theme(&self) -> Option<(reprisekit::theme::Theme, i32)> {
        let out = self.first_output()?;
        let scale = self.draw_scale_at(Some(&out)).max(1);
        Some((self.config.theme.scaled(scale), scale))
    }

    /// Put `canvas` (a panel of `size` at scale 1) on screen in place of the last; false while locked, where the panel stays hidden.
    fn show_canvas(&mut self, canvas: Canvas, size: (i32, i32), restart_timer: bool) -> bool {
        if self.is_locked() {
            self.hide_osd();
            return false;
        }
        let Some(out) = self.first_output() else {
            return false;
        };
        self.drop_panel();
        let scale = self.draw_scale_at(Some(&out)).max(1);
        // The canvas reaches past the panel by the glass shadow, if any.
        let m = panel::margin(&self.config.theme);
        let at = panel_origin(self.usable_area(Some(&out)), size);
        let pane = match self.config.theme.glass() {
            Some(g) if crate::glass::wanted(&self.config) => Some(Pane {
                shapes: vec![PaneShape {
                    rect: Rect::new(0, 0, size.0, size.1),
                    corner: g.panel.corner,
                    square_bottom: false,
                    square_top: false,
                }],
                holes: Vec::new(),
            }),
            _ => None,
        };

        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.overlay);
        scene.set_data(tree, Some(SceneOwner::Osd));
        crate::glass::leave_out(scene, tree);
        scene.set_position(tree, at.0, at.1);
        let glass_tree = scene.tree_create(tree);
        let mut backdrop = Backdrop::new(&self.glass);
        backdrop.set(scene, &self.glass, glass_tree, pane);
        let buffer: BufferId = scene.buffer_create(tree);
        scene.set_position(buffer, -m.left, -m.top);
        scene.set_input(
            buffer,
            Input::Rect(Rectangle::new((m.left, m.top).into(), size.into())),
        );
        upload_scaled(scene, buffer, &canvas, scale);
        self.osd.panel = Some(Panel {
            tree,
            glass_tree,
            backdrop,
        });
        if restart_timer || self.osd.timer.is_none() {
            self.restart_osd_timer();
        }
        self.redraw_all();
        true
    }

    fn restart_osd_timer(&mut self) {
        if let Some(t) = self.osd.timer.take() {
            self.handle.remove(t);
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(HOLD), |_, _, state| {
                state.osd.timer = None;
                state.hide_osd();
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.osd.timer = Some(t),
            Err(e) => log::warn!("osd: no hide timer: {}", e.error),
        }
    }

    pub fn hide_osd(&mut self) {
        if let Some(t) = self.osd.timer.take() {
            self.handle.remove(t);
        }
        self.osd.showing = None;
        if self.drop_panel() {
            self.redraw_all();
        }
    }

    /// Take the panel off the scene, leaving the timer; whether there was one.
    fn drop_panel(&mut self) -> bool {
        let Some(mut p) = self.osd.panel.take() else {
            return false;
        };
        p.backdrop
            .set(&mut self.scene, &self.glass, p.glass_tree, None);
        self.scene.destroy(p.tree);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_muted_sink_greys_the_meter_and_says_so() {
        let (osd, icon) = describe(&Report::Volume {
            level: 0.4,
            muted: true,
        })
        .unwrap();
        assert_eq!(osd.title, "Muted");
        assert_eq!(icon, "audio-volume-muted");
        assert_eq!(
            osd.meter,
            Some(Meter {
                fraction: 0.4,
                enabled: false
            })
        );
    }

    #[test]
    fn the_volume_icon_follows_the_level() {
        let name = |level: f64| {
            describe(&Report::Volume {
                level,
                muted: false,
            })
            .unwrap()
            .1
        };
        assert_eq!(name(0.0), "audio-volume-low");
        assert_eq!(name(0.5), "audio-volume-medium");
        assert_eq!(name(1.0), "audio-volume-high");
    }

    #[test]
    fn a_player_report_reads_as_a_sentence_without_a_meter() {
        let (osd, icon) = describe(&Report::Player {
            name: "Rhythmbox".into(),
            verb: "PlayPause",
        })
        .unwrap();
        assert_eq!(osd.title, "Rhythmbox: Play / Pause");
        assert_eq!(osd.meter, None);
        assert_eq!(icon, "multimedia-player");
    }

    #[test]
    fn the_mic_says_which_way_it_went_and_shows_no_meter() {
        let (osd, icon) = describe(&Report::Mic { muted: true }).unwrap();
        assert_eq!(osd.title, "Microphone muted");
        assert_eq!(osd.meter, None);
        assert_eq!(icon, "microphone-sensitivity-muted");
        let (osd, icon) = describe(&Report::Mic { muted: false }).unwrap();
        assert_eq!(osd.title, "Microphone on");
        assert_eq!(osd.meter, None);
        assert_eq!(icon, "microphone-sensitivity-high");
    }

    #[test]
    fn the_keyboard_backlight_gets_a_live_meter() {
        let (osd, icon) = describe(&Report::Kbd { level: 0.5 }).unwrap();
        assert_eq!(osd.title, "Keyboard backlight");
        assert_eq!(
            osd.meter,
            Some(Meter {
                fraction: 0.5,
                enabled: true
            })
        );
        assert_eq!(icon, "keyboard-brightness");
    }

    #[test]
    fn the_radio_reads_on_or_off_without_a_meter() {
        let (osd, icon) = describe(&Report::Wifi { on: true }).unwrap();
        assert_eq!(osd.title, "Wireless on");
        assert_eq!(osd.meter, None);
        assert_eq!(icon, "network-wireless");
        let (osd, _) = describe(&Report::Wifi { on: false }).unwrap();
        assert_eq!(osd.title, "Wireless off");
    }

    #[test]
    fn the_panel_sits_centred_above_the_usable_areas_bottom() {
        let usable = Rect::new(0, 0, 1920, 1040);
        assert_eq!(
            panel_origin(usable, (osd::W, osd::H)),
            ((1920 - osd::W) / 2, 1040 - osd::H - GAP)
        );
        let (w, h) = osd::lines_size(3);
        assert_eq!(
            panel_origin(usable, (w, h)),
            ((1920 - w) / 2, 1040 - h - GAP)
        );
    }
}
