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
    /// The wheel's sub-notch remainder over the emoji picker or the
    /// clipboard history.
    pub wheel: i32,
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

    pub fn close_panels(&mut self) {
        self.close_emoji();
        self.close_clipview();
        self.close_region();
    }
}
