//! The `screen-reader` action: starts Orca, or stops every copy this user runs. An OSD line says
//! which, and a sound marks it for a user who cannot see the line.

use std::time::Duration;

use reprisefs::screenreader::{self, PROGRAM};

use crate::compositor::State;
use crate::media::Report;

/// How long a switch waits for the reader it started or stopped to show in `/proc`.
const SETTLE: Duration = Duration::from_secs(2);

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
    /// Turn the screen reader on or off. The `/proc` walk and the spawn run off the loop, and the
    /// switch lasts until `/proc` shows it; a press meanwhile is dropped.
    pub fn toggle_screen_reader(&mut self) {
        if self.screen_reader_busy {
            return;
        }
        self.screen_reader_busy = true;
        let env = self.env.clone();
        let spawned = self.spawn_job(move || {
            let switched = match step(screenreader::running(), screenreader::installed()) {
                Step::Stop(pids) => {
                    screenreader::stop(&pids);
                    screenreader::wait_for(false, SETTLE);
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
                        Ok(Some(_)) => {
                            screenreader::wait_for(true, SETTLE);
                            Switched::On
                        }
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
        if !spawned {
            log::warn!("screen reader: no thread to switch it");
            self.screen_reader_busy = false;
        }
    }

    fn screen_reader_switched(&mut self, switched: Switched) {
        self.screen_reader_busy = false;
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
    fn a_press_while_a_switch_runs_is_dropped() {
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.state.fsjobs.refuse_threads = true;
        h.state.toggle_screen_reader();
        assert!(
            !h.state.screen_reader_busy,
            "a refused thread frees the key"
        );
        h.state.screen_reader_busy = true;
        h.state.toggle_screen_reader();
        assert!(h.state.screen_reader_busy, "no second switch was tried");
    }

    #[test]
    fn a_running_reader_is_stopped_else_one_starts_where_installed() {
        assert_eq!(step(vec![12, 40], true), Step::Stop(vec![12, 40]));
        assert_eq!(step(vec![12], false), Step::Stop(vec![12]));
        assert_eq!(step(Vec::new(), true), Step::Start);
        assert_eq!(step(Vec::new(), false), Step::Missing);
    }
}
