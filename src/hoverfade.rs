//! Hover fades: caption buttons and Start light and go dark over `[effects] hover_in_ms` /
//! `hover_out_ms`; a timer ticks every `TICK_MS` while anything is fading.

use std::time::{Duration, Instant};

use reprisekit::frame::CaptionButton;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use reprisescene::TreeId;

use crate::compositor::State;
use crate::window::WindowId;

const TICK_MS: u64 = 16;

impl State {
    pub fn glow_to_hot(&mut self, w: WindowId) {
        let (fade_in, fade_out) = self.config.effects.hover_ms();
        let now = Instant::now();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        for (k, b) in CaptionButton::ALL.into_iter().enumerate() {
            let hot = win.hot == Some(b);
            win.glow[k].set(hot, now, fade_in, fade_out);
        }
        self.redraw_caption(w);
        self.watch_window(w);
    }

    fn watch_window(&mut self, w: WindowId) {
        let now = Instant::now();
        let settled = self
            .win(w)
            .is_none_or(|x| x.glow.iter().all(|g| g.settled(now)));
        if settled || self.fading_frames.contains(&w) {
            return;
        }
        self.arm_hover_timer();
        self.fading_frames.push(w);
    }

    pub fn watch_bar(&mut self, tree: TreeId) {
        let now = Instant::now();
        let fading = self.bar(tree).is_some_and(|b| !b.start_glow.settled(now));
        if !fading || self.fading_bars.contains(&tree) {
            return;
        }
        self.arm_hover_timer();
        self.fading_bars.push(tree);
    }

    fn arm_hover_timer(&mut self) {
        if self.hover_timer.is_some() {
            return;
        }
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(TICK_MS)),
            |_, _, state| state.hover_tick(),
        );
        match token {
            Ok(t) => self.hover_timer = Some(t),
            Err(e) => log::warn!("hover timer: {}", e.error),
        }
    }

    fn hover_tick(&mut self) -> TimeoutAction {
        let now = Instant::now();
        let frames = std::mem::take(&mut self.fading_frames);
        for &w in &frames {
            self.redraw_caption(w);
        }
        self.fading_frames = frames
            .into_iter()
            .filter(|&w| {
                self.win(w)
                    .is_some_and(|x| !x.glow.iter().all(|g| g.settled(now)))
            })
            .collect();
        let bars = std::mem::take(&mut self.fading_bars);
        self.fading_bars = bars
            .into_iter()
            .filter(|&t| self.start_fade_tick(t))
            .collect();
        if self.fading_frames.is_empty() && self.fading_bars.is_empty() {
            log::debug!("hover fades settled; the timer stops");
            self.hover_timer = None;
            TimeoutAction::Drop
        } else {
            TimeoutAction::ToDuration(Duration::from_millis(TICK_MS))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_caption_fade_asks_only_the_outputs_its_window_is_on() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        crate::backend::take_asks();
        h.state.glow_to_hot(t.id);
        let asked = crate::backend::take_asks();
        assert!(!asked.is_empty(), "the caption asks for a frame");
        assert!(
            asked.iter().all(|(o, _)| o == "TEST-1"),
            "only TEST-1, which shows the window: {asked:?}"
        );
    }
}
