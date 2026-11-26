//! A media player's track and its transport buttons, as the volume popup, the task preview and the
//! lock screen show them.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaGlyph {
    Previous,
    Play,
    Pause,
    Next,
}

/// The track a media player has on, and what it can do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    /// Empty when the player names none: the title stands alone.
    pub artist: String,
    /// Playing rather than paused: the middle button pauses.
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_play: bool,
    pub can_pause: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    Previous,
    PlayPause,
    Next,
}

impl Transport {
    pub const ALL: [Transport; 3] = [Transport::Previous, Transport::PlayPause, Transport::Next];
}

impl NowPlaying {
    /// Whether button `t` acts: Play / Pause by what the middle button
    /// would do now.
    pub fn can(&self, t: Transport) -> bool {
        match t {
            Transport::Previous => self.can_previous,
            Transport::Next => self.can_next,
            Transport::PlayPause if self.playing => self.can_pause,
            Transport::PlayPause => self.can_play,
        }
    }

    pub fn glyph(&self, t: Transport) -> (MediaGlyph, &'static str) {
        match t {
            Transport::Previous => (MediaGlyph::Previous, "Previous"),
            Transport::Next => (MediaGlyph::Next, "Next"),
            Transport::PlayPause if self.playing => (MediaGlyph::Pause, "Pause"),
            Transport::PlayPause => (MediaGlyph::Play, "Play"),
        }
    }
}
