use std::time::Instant;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::raster::Rect;

#[derive(Default)]
pub struct Panels {
    /// The emoji font, catalog and recent picks, kept across opens.
    pub emoji: crate::emojiview::Emoji,
    pub emojiview: Option<crate::emojiview::EmojiView>,
    pub history: crate::cliphistory::Recorder,
    pub clipview: Option<crate::clipview::ClipView>,
    pub region: Option<crate::regionview::RegionView>,
    pub colorpick: Option<crate::colorpickview::ColorPickView>,
    /// Super+. waits for the focused client's caret report (`State::toggle_emoji`).
    pub emoji_wait: Option<smithay::reexports::calloop::RegistrationToken>,
    /// The wheel's carry over the emoji picker or the clipboard history.
    pub wheel: reprisekit::pointer::WheelCarry,
}

impl Panels {
    /// The region overlay or the colour picker is up, taking the pointer and the keys.
    pub fn takes_pointer(&self) -> bool {
        self.region.is_some() || self.colorpick.is_some()
    }
}

impl State {
    pub fn panel_anchor(&self) -> Rect {
        self.text_caret().unwrap_or_else(|| {
            let p = self.pointer_location();
            Rect::new(p.x as i32, p.y as i32, 1, 1)
        })
    }

    /// A panel's field went away: a field under it (a dialog's) gets the
    /// blink back, else the blink stops.
    pub fn end_panel_caret(&mut self) {
        if self.editor().is_some() {
            self.restart_caret();
        } else {
            self.stop_caret();
        }
    }

    /// Tick a panel's held scrollbar arrow every [`REPEAT_EVERY`] while `step`, which steps the
    /// panel's model and redraws it, says the bar is still held.
    ///
    /// [`REPEAT_EVERY`]: reprisekit::controls::scrollbar::REPEAT_EVERY
    pub fn repeat_panel_bar(&mut self, step: fn(&mut State, Instant) -> bool) {
        let every = reprisekit::controls::scrollbar::REPEAT_EVERY;
        let added =
            self.handle.insert_source(
                Timer::from_duration(every),
                move |now, _, state| match step(state, now) {
                    true => TimeoutAction::ToDuration(every),
                    false => TimeoutAction::Drop,
                },
            );
        if let Err(e) = added {
            log::warn!("panel: no scroll timer: {}", e.error);
        }
    }

    pub fn close_panels(&mut self) {
        self.close_emoji();
        self.close_clipview();
        self.close_region();
        self.cancel_color_pick();
    }
}
