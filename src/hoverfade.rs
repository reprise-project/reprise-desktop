//! Hover fades: caption buttons and Start light and go dark over `[effects] hover_in_ms` /
//! `hover_out_ms`, stepped on the frame clock (`animclock`) while anything is fading.

use std::time::Instant;

use reprisekit::frame::CaptionButton;

use reprisescene::TreeId;

use crate::animclock::Anim;
use crate::compositor::State;
use crate::window::WindowId;

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
        self.animate(Anim::Hover);
        self.fading_frames.push(w);
    }

    pub fn watch_bar(&mut self, tree: TreeId) {
        let now = Instant::now();
        let fading = self.bar(tree).is_some_and(|b| !b.start_glow.settled(now));
        if !fading || self.fading_bars.contains(&tree) {
            return;
        }
        self.animate(Anim::Hover);
        self.fading_bars.push(tree);
    }

    /// One frame of every fade; whether any still fades.
    pub(crate) fn hover_step(&mut self, now: Instant) -> bool {
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
            .filter(|&t| self.start_fade_step(t, now))
            .collect();
        !self.fading_frames.is_empty() || !self.fading_bars.is_empty()
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

    #[test]
    fn a_caption_fade_steps_on_its_outputs_frames_until_it_settles() {
        let mut config = crate::config::Config::default();
        config.effects.hover_fade = true;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.state.win_mut(t.id).unwrap().hot = Some(reprisekit::frame::CaptionButton::Close);
        let from = std::time::Instant::now();
        h.state.glow_to_hot(t.id);
        crate::animclock::assert_steps_until(
            &mut h.state,
            crate::animclock::Anim::Hover,
            &["TEST-1"],
            from,
            from + std::time::Duration::from_secs(5),
        );
    }
}
