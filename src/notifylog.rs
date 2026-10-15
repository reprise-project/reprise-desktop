//! The notification history behind the tray's bell: non-transient notifications, newest first, and Do Not Disturb.
//! Pure bookkeeping: `notifyview` turns what these return into balloons and bus signals.

use crate::tray::Notification;
use crate::tray::notify::Urgency;

pub const MAX_ENTRIES: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub n: Notification,
    pub time: i64,
    pub open: bool,
    pub sender_gone: bool,
    /// Answered from its balloon's reply row.
    pub replied: bool,
}

impl Entry {
    /// Whether action `key` can still run: the entry is open, its sender
    /// is there, and it has that action.
    pub fn can_invoke(&self, key: &str) -> bool {
        self.open && !self.sender_gone && self.n.has_action(key)
    }
}

#[derive(Debug, Default)]
pub struct Log {
    pub entries: Vec<Entry>,
    pub unseen: usize,
    pub dnd: bool,
}

impl Log {
    pub fn shows_balloon(&self, n: &Notification) -> bool {
        !self.dnd || n.urgency == Urgency::Critical
    }

    /// Keep `n`, arrived at `time`, at the top: one with the same id is
    /// replaced (and stays open). `looking` is whether the history is on
    /// screen, where nothing is unseen. Returns the ids that fell off the
    /// end while still open, which expire.
    pub fn record(&mut self, n: &Notification, time: i64, looking: bool) -> Vec<u32> {
        self.entries.retain(|e| e.n.id != n.id);
        self.entries.insert(
            0,
            Entry {
                n: n.clone(),
                time,
                open: true,
                sender_gone: false,
                replied: false,
            },
        );
        if !looking {
            self.unseen = (self.unseen + 1).min(self.entries.len());
        }
        let dropped = self.entries.split_off(self.entries.len().min(MAX_ENTRIES));
        self.unseen = self.unseen.min(self.entries.len());
        dropped
            .into_iter()
            .filter(|e| e.open)
            .map(|e| e.n.id)
            .collect()
    }

    pub fn get(&self, id: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.n.id == id)
    }

    pub fn close(&mut self, id: u32) -> bool {
        match self.entries.iter_mut().find(|e| e.n.id == id) {
            Some(e) => std::mem::replace(&mut e.open, false),
            None => false,
        }
    }

    pub fn mark_replied(&mut self, id: u32) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.n.id == id) {
            e.replied = true;
        }
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let Some(i) = self.entries.iter().position(|e| e.n.id == id) else {
            return false;
        };
        let e = self.entries.remove(i);
        self.unseen = self.unseen.min(self.entries.len());
        e.open
    }

    /// Empty the history; returns the ids still open, which the user
    /// dismissed.
    pub fn clear(&mut self) -> Vec<u32> {
        self.unseen = 0;
        self.entries
            .drain(..)
            .filter(|e| e.open)
            .map(|e| e.n.id)
            .collect()
    }

    /// Take every entry from program `app` out (a group's Clear); returns
    /// the ids still open, which the user dismissed.
    pub fn clear_app(&mut self, app: &str) -> Vec<u32> {
        let (gone, kept) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition::<Vec<Entry>, _>(|e| e.n.app == app);
        self.entries = kept;
        self.unseen = self.unseen.min(self.entries.len());
        gone.into_iter()
            .filter(|e| e.open)
            .map(|e| e.n.id)
            .collect()
    }

    pub fn sender_gone(&mut self, sender: &str) {
        for e in self.entries.iter_mut().filter(|e| e.n.sender == sender) {
            e.sender_gone = true;
        }
    }

    /// Whether the tray shows the bell: there is history, or Do Not
    /// Disturb is on.
    pub fn bell_shown(&self) -> bool {
        !self.entries.is_empty() || self.dnd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32) -> Notification {
        Notification::new(id, format!("n{id}"), String::new())
    }

    #[test]
    fn newest_first_and_a_replace_moves_to_the_top() {
        let mut log = Log::default();
        log.record(&note(1), 10, false);
        log.record(&note(2), 20, false);
        let mut again = note(1);
        again.summary = "updated".into();
        log.record(&again, 30, false);
        let ids: Vec<u32> = log.entries.iter().map(|e| e.n.id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(log.entries[0].n.summary, "updated");
        assert_eq!(log.entries[0].time, 30);
    }

    #[test]
    fn past_fifty_the_oldest_falls_off_and_expires_if_open() {
        let mut log = Log::default();
        for id in 1..=MAX_ENTRIES as u32 {
            assert!(log.record(&note(id), 0, false).is_empty());
        }
        assert_eq!(log.record(&note(100), 0, false), vec![1]);
        log.close(2);
        assert_eq!(
            log.record(&note(101), 0, false),
            Vec::<u32>::new(),
            "a closed entry falls off without a signal"
        );
        assert_eq!(log.entries.len(), MAX_ENTRIES);
        assert_eq!(log.unseen, MAX_ENTRIES);
    }

    #[test]
    fn unseen_counts_only_while_nobody_looks() {
        let mut log = Log::default();
        log.record(&note(1), 0, false);
        log.record(&note(2), 0, true);
        assert_eq!(log.unseen, 1);
        log.unseen = 0;
        log.record(&note(3), 0, false);
        assert_eq!(log.unseen, 1);
        log.remove(3);
        assert_eq!(log.unseen, 1.min(log.entries.len()));
    }

    #[test]
    fn close_remove_and_clear_report_what_was_open() {
        let mut log = Log::default();
        for id in 1..=3 {
            log.record(&note(id), 0, false);
        }
        assert!(log.close(1));
        assert!(!log.close(1), "already closed");
        assert!(log.remove(2));
        assert!(!log.remove(2));
        assert_eq!(log.clear(), vec![3]);
        assert!(log.entries.is_empty());
        assert_eq!(log.unseen, 0);
    }

    #[test]
    fn a_groups_clear_takes_out_only_that_program() {
        let mut log = Log::default();
        for (id, app) in [(1, "Mail"), (2, "Chat"), (3, "Mail")] {
            let mut n = note(id);
            n.app = app.into();
            log.record(&n, 0, false);
        }
        log.close(1);
        assert_eq!(log.clear_app("Mail"), vec![3], "only the open one");
        let ids: Vec<u32> = log.entries.iter().map(|e| e.n.id).collect();
        assert_eq!(ids, [2]);
        assert_eq!(log.unseen, 1);
    }

    #[test]
    fn a_replied_entry_keeps_its_mark_once_closed_until_replaced() {
        let mut log = Log::default();
        log.record(&note(1), 0, false);
        log.record(&note(2), 0, false);
        log.mark_replied(1);
        log.close(1);
        let e = log.get(1).unwrap();
        assert!(e.replied && !e.open);
        assert!(!log.get(2).unwrap().replied);
        log.record(&note(1), 1, false);
        assert!(
            !log.get(1).unwrap().replied,
            "a new message under the same id"
        );
    }

    #[test]
    fn dnd_lets_only_critical_balloons_through() {
        let mut log = Log::default();
        let mut crit = note(1);
        crit.urgency = Urgency::Critical;
        assert!(log.shows_balloon(&note(2)));
        log.dnd = true;
        assert!(!log.shows_balloon(&note(2)));
        assert!(log.shows_balloon(&crit));
        assert!(log.bell_shown(), "the bell shows for DND alone");
    }

    #[test]
    fn actions_run_only_while_open_and_the_sender_is_there() {
        let mut log = Log::default();
        let mut n = note(1);
        n.sender = ":1.7".into();
        n.actions = vec![
            ("default".into(), String::new()),
            ("open".into(), "Open".into()),
        ];
        log.record(&n, 0, false);
        assert!(log.get(1).unwrap().can_invoke("open"));
        assert!(!log.get(1).unwrap().can_invoke("reply"));
        log.sender_gone(":1.8");
        assert!(log.get(1).unwrap().can_invoke("open"));
        log.sender_gone(":1.7");
        assert!(!log.get(1).unwrap().can_invoke("open"));
        let mut log = Log::default();
        log.record(&n, 0, false);
        log.close(1);
        assert!(!log.get(1).unwrap().can_invoke("default"));
    }
}
