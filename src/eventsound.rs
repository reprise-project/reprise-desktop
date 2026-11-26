//! Desktop event sounds (`[sounds] events`): the shell's moments play their sound theme ids through
//! `repriseaudio::sounds::play_event`, which keeps a burst of one event to one sound.

use repriseaudio::sounds::Event;
use smithay::{
    backend::session::Session,
    reexports::{
        calloop::{Interest, Mode, PostAction, generic::Generic},
        udev::{EventType, MonitorBuilder},
    },
};

use crate::compositor::State;

impl State {
    /// Play `event`'s sound unless `[sounds] events` is off or this seat is away.
    pub fn event_sound(&mut self, event: Event) {
        if !self.config.sounds.events {
            return;
        }
        if (self.backend.as_ref()).is_some_and(|b| !b.session.is_active()) {
            return;
        }
        #[cfg(test)]
        self.event_sounds.push(event);
        #[cfg(not(test))]
        repriseaudio::sounds::play_event(event, self.config.sounds.theme.clone());
    }
}

/// The sound of an error or warning box with icon `icon`.
pub fn of_box(icon: Option<&str>) -> Option<Event> {
    match icon? {
        "dialog-error" => Some(Event::Error),
        "dialog-warning" => Some(Event::Warning),
        _ => None,
    }
}

/// Watch udev for USB devices plugged in and out. Only changes sound: the devices there at start
/// add nothing.
pub fn watch_usb(state: &mut State) {
    let monitor = MonitorBuilder::new()
        .and_then(|m| m.match_subsystem_devtype("usb", "usb_device"))
        .and_then(|m| m.listen());
    let monitor = match monitor {
        Ok(m) => m,
        Err(e) => return log::warn!("event sounds: no udev monitor for USB devices: {e}"),
    };
    let added = state.handle.insert_source(
        Generic::new(monitor, Interest::READ, Mode::Level),
        |_, monitor, state| {
            for event in monitor.iter() {
                match event.event_type() {
                    EventType::Add => state.event_sound(Event::DeviceAdded),
                    EventType::Remove => state.event_sound(Event::DeviceRemoved),
                    _ => {}
                }
            }
            Ok(PostAction::Continue)
        },
    );
    if let Err(e) = added {
        log::warn!("event sounds: no USB device source: {}", e.error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    #[test]
    fn an_error_box_plays_the_error_sound_and_the_switch_silences_it() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state
            .open_dialog(crate::dialog::Dialog::error("Oops", "Broken.".into()));
        assert_eq!(h.state.event_sounds, [Event::Error]);
        h.state.close_all_dialogs();
        h.state.config.sounds.events = false;
        h.state
            .open_dialog(crate::dialog::Dialog::error("Oops", "Broken.".into()));
        assert_eq!(
            h.state.event_sounds,
            [Event::Error],
            "events = false is silent"
        );
    }

    #[test]
    fn only_error_and_warning_icons_have_a_box_sound() {
        assert_eq!(of_box(Some("dialog-error")), Some(Event::Error));
        assert_eq!(of_box(Some("dialog-warning")), Some(Event::Warning));
        assert_eq!(of_box(Some("dialog-question")), None);
        assert_eq!(of_box(None), None);
    }
}
