//! The first prompt each user's last conversation reported, so the next box opens on it and a
//! one-time code asked ahead of the password is typed where it is asked. Kept for the agent's
//! lifetime and in `$XDG_STATE_HOME/reprise/polkit.toml` (0600): prompt text only, never an
//! answer. Only a prompt other than the box's own Password field is stored; an unreadable or
//! broken file remembers nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::helper::Prompt;

pub fn path() -> PathBuf {
    repriseconf::statefile::path("polkit")
}

/// Without a path (the tests) nothing is read or written.
#[derive(Default)]
pub struct FirstPrompts {
    path: Option<PathBuf>,
    users: BTreeMap<String, Prompt>,
    writing: Option<std::thread::JoinHandle<()>>,
}

impl FirstPrompts {
    pub fn load(path: PathBuf) -> FirstPrompts {
        let users = repriseconf::statefile::load(&path);
        FirstPrompts {
            path: Some(path),
            users,
            writing: None,
        }
    }

    /// The prompt a box for `user` opens on.
    pub fn get(&self, user: &str) -> Prompt {
        self.users
            .get(user)
            .cloned()
            .unwrap_or_else(Prompt::password)
    }

    /// `user`'s conversation reported `prompt` first.
    pub fn learn(&mut self, user: &str, prompt: &Prompt) {
        if self.get(user) == *prompt {
            return;
        }
        if *prompt == Prompt::password() {
            self.users.remove(user);
        } else {
            self.users.insert(user.to_string(), prompt.clone());
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        let text = toml::to_string(&self.users).map_err(std::io::Error::other);
        let previous = self.writing.take();
        // Off the UI thread: the write syncs the file and its directory. Each write waits for
        // the one before, so the file ends on the newest memory.
        self.writing = std::thread::Builder::new()
            .name("polkit-first".into())
            .spawn(move || {
                if let Some(previous) = previous {
                    let _ = previous.join();
                }
                let written =
                    text.and_then(|text| reprisefs::ops::write_private(&path, text.as_bytes()));
                if let Err(e) = written {
                    log::warn!("{}: could not remember the prompt: {e}", path.display());
                }
            })
            .ok();
    }

    /// Block until the last write is done.
    #[cfg(test)]
    pub fn wait(&mut self) {
        if let Some(writing) = self.writing.take() {
            let _ = writing.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn code() -> Prompt {
        Prompt {
            text: "Verification code:".into(),
            echo: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "reprise-polkit-first-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_learned_prompt_is_read_back_per_user_from_a_private_file() {
        let dir = scratch("round");
        let path = dir.join("reprise").join("polkit.toml");
        let mut first = FirstPrompts::load(path.clone());
        assert_eq!(first.get("alice"), Prompt::password(), "nothing remembered");
        first.learn("alice", &code());
        let shown = Prompt {
            text: "Passcode:".into(),
            echo: true,
        };
        first.learn("bob", &shown);
        first.wait();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let again = FirstPrompts::load(path.clone());
        assert_eq!(again.get("alice"), code());
        assert_eq!(again.get("bob"), shown);
        assert_eq!(again.get("carol"), Prompt::password());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_prompt_is_replaced_and_the_password_forgets_it() {
        let dir = scratch("stale");
        let path = dir.join("polkit.toml");
        let mut first = FirstPrompts::load(path.clone());
        first.learn("alice", &code());
        first.learn("alice", &Prompt::password());
        assert_eq!(first.get("alice"), Prompt::password());
        first.wait();
        assert_eq!(
            FirstPrompts::load(path.clone()).get("alice"),
            Prompt::password()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_or_unreadable_file_remembers_nothing() {
        let dir = scratch("broken");
        let path = dir.join("polkit.toml");
        std::fs::write(&path, "[alice\ntext = 3\n\u{0}").unwrap();
        assert_eq!(
            FirstPrompts::load(path.clone()).get("alice"),
            Prompt::password()
        );
        std::fs::write(&path, "[alice]\ntext = 3\necho = \"no\"\n").unwrap();
        assert_eq!(
            FirstPrompts::load(path.clone()).get("alice"),
            Prompt::password()
        );
        std::fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();
        assert_eq!(
            FirstPrompts::load(path.clone()).get("alice"),
            Prompt::password()
        );
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir(&path).unwrap();
        let mut first = FirstPrompts::load(path.clone());
        assert_eq!(first.get("alice"), Prompt::password());
        first.learn("alice", &code());
        assert_eq!(first.get("alice"), code(), "kept for the agent's lifetime");
        let _ = std::fs::remove_dir_all(dir);
    }
}
