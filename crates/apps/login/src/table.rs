use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use repriselogin::greeter::Note;

/// A user session started by reprise-login, or found in logind after a restart.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionEntry {
    /// The logind session id.
    pub id: String,
    pub user: String,
    pub uid: u32,
    pub vt: u16,
    /// `None` for a session found in logind after a daemon restart.
    pub started: Option<Instant>,
}

/// Every running user session, at most one per user.
#[derive(Debug, Default)]
pub struct Sessions {
    entries: Vec<SessionEntry>,
}

/// Where a login for a user goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Start,
    Resume { id: String, vt: u16 },
}

impl Sessions {
    /// Adds `entry`, replacing an entry with the same id.
    pub fn insert(&mut self, entry: SessionEntry) {
        self.remove(&entry.id);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, id: &str) -> Option<SessionEntry> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(at))
    }

    pub fn by_id(&self, id: &str) -> Option<&SessionEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn by_user(&self, user: &str) -> Option<&SessionEntry> {
        self.entries.iter().find(|e| e.user == user)
    }

    pub fn taken_vts(&self) -> BTreeSet<u16> {
        self.entries.iter().map(|e| e.vt).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The login screen's note for each user, keyed by user name.
    pub fn notes(&self) -> HashMap<String, Note> {
        (self.entries.iter())
            .map(|e| (e.user.clone(), Note::LoggedOn))
            .collect()
    }

    pub fn route(&self, user: &str) -> Route {
        match self.by_user(user) {
            Some(e) => Route::Resume {
                id: e.id.clone(),
                vt: e.vt,
            },
            None => Route::Start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, user: &str, vt: u16) -> SessionEntry {
        SessionEntry {
            id: id.into(),
            user: user.into(),
            uid: 1000 + vt as u32,
            vt,
            started: Some(Instant::now()),
        }
    }

    fn table() -> Sessions {
        let mut s = Sessions::default();
        s.insert(entry("3", "anna", 7));
        s.insert(entry("5", "bert", 8));
        s
    }

    #[test]
    fn route_resumes_a_users_session_and_starts_otherwise() {
        let mut s = table();
        assert_eq!(
            s.route("anna"),
            Route::Resume {
                id: "3".into(),
                vt: 7
            }
        );
        assert_eq!(s.route("carl"), Route::Start);
        assert_eq!(s.remove("3").map(|e| e.user), Some("anna".into()));
        assert_eq!(s.route("anna"), Route::Start);
        assert_eq!(s.by_id("5").map(|e| e.user.as_str()), Some("bert"));
        assert_eq!(s.by_user("bert").map(|e| e.id.as_str()), Some("5"));
        assert!(s.remove("3").is_none());
    }

    #[test]
    fn notes_mark_every_user_with_a_session_logged_on() {
        let notes = table().notes();
        assert_eq!(
            notes,
            HashMap::from([
                ("anna".to_string(), Note::LoggedOn),
                ("bert".to_string(), Note::LoggedOn),
            ])
        );
        assert!(Sessions::default().notes().is_empty());
    }

    #[test]
    fn taken_vts_lists_every_sessions_vt() {
        let mut s = table();
        assert_eq!(s.taken_vts(), BTreeSet::from([7, 8]));
        assert!(!s.is_empty());
        s.remove("3");
        s.remove("5");
        assert!(s.taken_vts().is_empty());
        assert!(s.is_empty());
    }
}
