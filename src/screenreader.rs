//! The `screen-reader` action: starts Orca, or stops every copy this user runs. An OSD line says
//! which, and a sound marks it for a user who cannot see the line.

use reprisefs::screenreader::{self, PROGRAM};

use crate::compositor::State;
use crate::media::Report;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switched {
    On,
    Off,
    /// Orca is not installed, or would not start.
    Missing,
}

impl Switched {
    pub fn title(self) -> &'static str {
        match self {
            Switched::On => "Screen reader on",
            Switched::Off => "Screen reader off",
            Switched::Missing => "No screen reader installed",
        }
    }

    /// XDG sound theme names.
    fn sound(self) -> &'static str {
        match self {
            Switched::On => "device-added",
            Switched::Off => "device-removed",
            Switched::Missing => "dialog-error",
        }
    }
}

/// What the action does: stop the running readers `running`, else start one when `installed`.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Stop(Vec<i32>),
    Start,
    Missing,
}

fn step(running: Vec<i32>, installed: bool) -> Step {
    match (running.is_empty(), installed) {
        (false, _) => Step::Stop(running),
        (true, true) => Step::Start,
        (true, false) => Step::Missing,
    }
}

impl State {
    /// Turn the screen reader on or off. The `/proc` walk and the spawn run off the loop.
    pub fn toggle_screen_reader(&mut self) {
        let env = self.env.clone();
        self.spawn_job(move || {
            let switched = match step(screenreader::running(), screenreader::installed()) {
                Step::Stop(pids) => {
                    screenreader::stop(&pids);
                    Switched::Off
                }
                Step::Start => {
                    match crate::spawn::try_spawn_argv_in(
                        &[PROGRAM],
                        Some(PROGRAM),
                        None,
                        &env,
                        None,
                    ) {
                        Ok(Some(_)) => Switched::On,
                        Ok(None) => Switched::Missing,
                        Err(e) => {
                            log::warn!("screen reader: {e}");
                            Switched::Missing
                        }
                    }
                }
                Step::Missing => Switched::Missing,
            };
            log::info!("screen reader: {}", switched.title());
            Box::new(move |state: &mut State| state.screen_reader_switched(switched))
        });
    }

    fn screen_reader_switched(&mut self, switched: Switched) {
        let theme = self.config.sounds.theme.clone();
        repriseaudio::sounds::play(
            repriseaudio::sounds::Sound::Name(switched.sound().into()),
            theme,
        );
        self.show_osd(&Report::ScreenReader(switched));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_reader_is_stopped_else_one_starts_where_installed() {
        assert_eq!(step(vec![12, 40], true), Step::Stop(vec![12, 40]));
        assert_eq!(step(vec![12], false), Step::Stop(vec![12]));
        assert_eq!(step(Vec::new(), true), Step::Start);
        assert_eq!(step(Vec::new(), false), Step::Missing);
    }
}
