//! Event sounds a program plays (`[sounds] events`), kept to one sound per burst of an event by
//! [`repriseaudio::sounds::play_event`].

pub use repriseaudio::sounds::Event;
use repriseconf::config::SoundsConfig;

/// Play `event`'s sound unless `[sounds] events` is off.
pub fn event(event: Event) {
    #[cfg(test)]
    ASKED.with_borrow_mut(|a| a.push(event));
    play(&repriseconf::current().sounds, event);
}

/// Whether `cfg` lets `event` sound; plays it if so.
fn play(cfg: &SoundsConfig, event: Event) -> bool {
    if cfg.events {
        repriseaudio::sounds::play_event(event, cfg.theme.clone());
    }
    cfg.events
}

#[cfg(test)]
thread_local! {
    /// Each event this thread asked [`event`] for.
    pub(crate) static ASKED: std::cell::RefCell<Vec<Event>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_switch_off_silences_a_program_sound() {
        let on = SoundsConfig::default();
        let off = SoundsConfig {
            events: false,
            ..on.clone()
        };
        assert!(play(&on, Event::Warning), "events on by default");
        assert!(!play(&off, Event::Warning), "events = false is silent");
    }
}
