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
    /// A session a remote login started or reached that seat0 has not shown since.
    pub background: bool,
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
    /// Adds `entry`, replacing any entry with the same id, user or uid.
    pub fn insert(&mut self, entry: SessionEntry) {
        (self.entries).retain(|e| e.id != entry.id && e.user != entry.user && e.uid != entry.uid);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, id: &str) -> Option<SessionEntry> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(at))
    }

    pub fn by_id(&self, id: &str) -> Option<&SessionEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn by_uid(&self, uid: u32) -> Option<&SessionEntry> {
        self.entries.iter().find(|e| e.uid == uid)
    }

    pub fn by_user(&self, user: &str) -> Option<&SessionEntry> {
        self.entries.iter().find(|e| e.user == user)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SessionEntry> {
        self.entries.iter()
    }

    pub fn taken_vts(&self) -> BTreeSet<u16> {
        self.entries.iter().map(|e| e.vt).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn set_background(&mut self, id: &str, background: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.background = background;
        }
    }

    /// The login screen's note for each user, keyed by user name; `remote` says whether a
    /// uid's session has remote clients.
    pub fn notes(&self, remote: impl Fn(u32) -> bool) -> HashMap<String, Note> {
        (self.entries.iter())
            .map(|e| {
                let note = if remote(e.uid) {
                    Note::Remote
                } else {
                    Note::LoggedOn
                };
                (e.user.clone(), note)
            })
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
            background: false,
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
    fn notes_mark_every_user_with_a_session_logged_on_or_remote() {
        let notes = table().notes(|uid| uid == 1008);
        assert_eq!(
            notes,
            HashMap::from([
                ("anna".to_string(), Note::LoggedOn),
                ("bert".to_string(), Note::Remote),
            ])
        );
        assert!(Sessions::default().notes(|_| true).is_empty());
    }

    #[test]
    fn insert_keeps_one_entry_per_id_user_and_uid() {
        let mut s = table();
        s.insert(entry("9", "anna", 9));
        assert_eq!(s.by_user("anna").map(|e| e.id.as_str()), Some("9"));
        assert!(
            s.by_id("3").is_none(),
            "the same user replaces the old entry"
        );
        s.insert(SessionEntry {
            uid: 1009,
            ..entry("11", "Anna", 11)
        });
        assert_eq!(s.by_uid(1009).map(|e| e.id.as_str()), Some("11"));
        assert!(
            s.by_id("9").is_none(),
            "the same uid replaces the old entry"
        );
        s.insert(entry("5", "dora", 12));
        assert!(
            s.by_user("bert").is_none(),
            "the same id replaces the old entry"
        );
        assert_eq!(s.taken_vts(), BTreeSet::from([11, 12]));
    }

    #[test]
    fn taken_vts_lists_every_sessions_vt() {
        let mut s = table();
        assert_eq!(s.taken_vts(), BTreeSet::from([7, 8]));
        assert!(!s.is_empty());
        assert_eq!(s.len(), 2);
        s.remove("3");
        s.remove("5");
        assert!(s.taken_vts().is_empty());
        assert!(s.is_empty());
    }
}
