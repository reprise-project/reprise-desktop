//! The Region & Language page's host: reads the locales, this account's locale.conf and the
//! system's locale off the loop when the page first opens; Apply writes locale.conf and tells
//! AccountsService, Apply to the System sets localed's locale at once.

use crate::regionpage::{self, System};
use crate::registry::{self, Page};
use crate::{SettingsApp, take};
use reprisebus::polkit::{self, Refusal};
use reprisefs::region::{self, Choice};
use std::path::{Path, PathBuf};

/// The page's locales and choices once read.
pub(crate) struct Region {
    /// (locale, name) sorted by name ([`regionpage::named`]).
    pub(crate) locales: Vec<(String, String)>,
    pub(crate) loaded: Choice,
    pub(crate) want: Choice,
    pub(crate) system: System,
    /// An Apply to the System waits for localed.
    pub(crate) setting: bool,
    /// localed read again: the page opened while the last read had no answer.
    pub(crate) reread: Option<std::sync::mpsc::Receiver<System>>,
}

impl Region {
    pub(crate) fn dirty(&self) -> bool {
        self.want != self.loaded
    }
}

/// What the page's first read found.
pub(crate) struct Reading {
    pub(crate) locales: Vec<(String, String)>,
    pub(crate) user: Choice,
    pub(crate) system: System,
}

/// How an Apply to the System ended: the system's locale after, or what to tell (None for a
/// dismissed prompt).
pub(crate) type Set = Result<Vec<(String, String)>, Option<String>>;

/// The account's choice: its locale.conf at `conf`, else the environment this program started in.
pub(crate) fn user_choice(conf: &Path, env: impl Fn(&str) -> Option<String>) -> Choice {
    let file = std::fs::read_to_string(conf).unwrap_or_default();
    let mut pairs = region::parse(&file);
    if !pairs.iter().any(|(k, _)| k == "LANG") {
        pairs.extend(
            region::VARS
                .iter()
                .filter(|k| !pairs.iter().any(|(set, _)| set == *k))
                .filter_map(|&k| Some((k.to_string(), env(k)?)))
                .collect::<Vec<_>>(),
        );
    }
    Choice::of(&pairs).unwrap_or(Choice {
        language: "C.UTF-8".into(),
        formats: None,
    })
}

/// `installed` with each locale's name, the account's and the system's locales added where
/// missing, named as not installed when they do not load, which is why their samples are blank.
pub(crate) fn with_names(installed: Vec<String>, also: &[&str]) -> Vec<(String, String)> {
    let mut named: Vec<(String, String)> = (installed.into_iter())
        .map(|c| {
            let name = region::name(&c);
            (c, name)
        })
        .collect();
    for c in also {
        if !named.iter().any(|(have, _)| have == c) {
            let name = match region::loads(c) {
                true => region::name(c),
                false => format!("{} - not installed", region::name(c)),
            };
            named.push((c.to_string(), name));
        }
    }
    regionpage::named(named)
}

fn read(conf: PathBuf) -> Reading {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let user = user_choice(&conf, env);
    let system = read_system();
    let mut also = vec![user.language.as_str()];
    also.extend(user.formats.as_deref());
    let sys_choice = match &system {
        System::Read { locale, .. } => Choice::of(locale),
        _ => None,
    };
    if let Some(c) = &sys_choice {
        also.push(&c.language);
        also.extend(c.formats.as_deref());
    }
    Reading {
        locales: with_names(region::installed(), &also),
        user,
        system,
    }
}

fn read_system() -> System {
    let read = reprisebus::system().and_then(|conn| {
        reprisebus::wait(async {
            let locale = reprisebus::locale::system(&conn).await?;
            let may_set = polkit::may_ask(&conn, reprisebus::locale::SET_LOCALE).await;
            Ok((locale, may_set))
        })
    });
    match read {
        Ok((locale, may_set)) => System::Read { locale, may_set },
        Err(e) => {
            log::warn!("localed: {e}");
            System::Unread
        }
    }
}

/// localed answers `AccessDenied` for a dismissed prompt and a refused password alike.
fn set_system(pairs: Vec<(String, String)>) -> Set {
    reprisebus::locale::set_system_interactive(&pairs).map_err(|e| {
        let denied = |n: &str| n == "org.freedesktop.DBus.Error.AccessDenied";
        match polkit::refusal(&e, denied, polkit::dismissed) {
            Some(Refusal::Dismissed) => None,
            Some(Refusal::TimedOut) => Some(polkit::TIMED_OUT.into()),
            Some(Refusal::Denied) => Some(polkit::NOT_ALLOWED.into()),
            None => {
                log::warn!("SetLocale: {e}");
                let why = reprisebus::error::message(&e).map_or(e.to_string(), String::from);
                Some(format!(
                    "The system's language could not be changed.\n\n{why}"
                ))
            }
        }
    })
}

fn set_language(language: String) {
    let set = reprisebus::system()
        .and_then(|c| reprisebus::wait(reprisebus::locale::set_own_language(&c, &language)));
    if let Err(e) = set {
        log::warn!("AccountsService keeps the old language: {e}");
    }
}

/// What localed takes to make `want` the system's `locale`. localed keeps a variable left out and
/// drops one equal to `LANG`, so formats that follow the language go as the language's locale.
pub(crate) fn localed_pairs(want: &Choice, locale: &[(String, String)]) -> Vec<(String, String)> {
    let every = Choice {
        formats: Some(want.formats_locale().to_string()),
        ..want.clone()
    };
    every.merge(locale)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl SettingsApp {
    /// Read the page's locales off the loop the first time it opens, and localed again at a
    /// later opening while it has not answered.
    pub(crate) fn ensure_region(&mut self) {
        let open = registry::open(&self.sheet) == Page::RegionLanguage;
        let was = std::mem::replace(&mut self.region_open, open);
        let opened = open && !was;
        if !open || self.region_read.is_some() {
            return;
        }
        let Some(r) = &self.region else {
            let conf = self.region_conf.clone();
            self.start_region_read(move || read(conf));
            return;
        };
        if opened && r.system == System::Unread && r.reread.is_none() {
            self.start_region_reread();
        }
    }

    /// The first read or an Apply to the System ended. True when the page changed.
    pub(crate) fn region_done(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.region_read {
            let done = match take(rx) {
                Ok(None) => None,
                Ok(Some(r)) => Some(r),
                Err(()) => {
                    log::warn!("the Region & Language read ended without a result");
                    Some(Reading {
                        locales: Vec::new(),
                        user: user_choice(&self.region_conf, |_| None),
                        system: System::Unread,
                    })
                }
            };
            if let Some(r) = done {
                self.region_read = None;
                self.region = Some(Region {
                    locales: r.locales,
                    loaded: r.user.clone(),
                    want: r.user,
                    system: r.system,
                    setting: false,
                    reread: None,
                });
                self.show_region(true);
                changed = true;
            }
        }
        if let Some(r) = &mut self.region
            && let Some(rx) = &r.reread
        {
            let read = match take(rx) {
                Ok(read) => read,
                Err(()) => Some(System::Unread),
            };
            if let Some(system) = read {
                r.reread = None;
                r.system = system;
                self.show_region(false);
                changed = true;
            }
        }
        if let Some(rx) = &self.region_set {
            let done = match take(rx) {
                Ok(None) => None,
                Ok(Some(r)) => Some(r),
                Err(()) => Some(Err(Some(
                    "The system's language could not be changed.".into(),
                ))),
            };
            if let Some(set) = done {
                self.region_set = None;
                if let Some(r) = &mut self.region {
                    r.setting = false;
                    if let (Ok(locale), System::Read { may_set, .. }) = (&set, &r.system) {
                        r.system = System::Read {
                            locale: locale.clone(),
                            may_set: *may_set,
                        };
                    }
                }
                if let Err(Some(why)) = set {
                    self.fail(&why);
                }
                self.show_region(false);
                changed = true;
            }
        }
        changed
    }

    /// Put the read state on the page; `choice` refills the combos and the sample too.
    pub(crate) fn show_region(&mut self, choice: bool) {
        let Some(r) = &self.region else {
            return;
        };
        let page = &mut self.sheet.pages[Page::RegionLanguage.index()];
        if choice {
            regionpage::show_choice(page, &r.locales, &r.want, now());
        }
        regionpage::show_system(page, &r.system, &r.locales, Some(&r.want), r.setting);
        self.sync_apply();
    }

    /// A combo moved: take the choice and show its sample.
    pub(crate) fn region_changed(&mut self) {
        let Some(r) = &mut self.region else {
            return;
        };
        let page = &self.sheet.pages[Page::RegionLanguage.index()];
        if let Some(c) = regionpage::read_choice(page, &r.locales) {
            r.want = c;
        }
        self.show_region(true);
    }

    /// Apply to the System: localed takes the page's choice, polkit asking for a password.
    pub(crate) fn region_to_system(&mut self) {
        let Some(r) = &mut self.region else {
            return;
        };
        let System::Read { locale, .. } = &r.system else {
            return;
        };
        if r.setting || Choice::of(locale).as_ref() == Some(&r.want) {
            return;
        }
        r.setting = true;
        let pairs = localed_pairs(&r.want, locale);
        self.start_region_set(move || set_system(pairs));
        self.show_region(false);
    }

    /// Write the account's locale.conf and tell AccountsService. False when the file could not be
    /// written; the page stays dirty.
    pub(crate) fn apply_region(&mut self) -> bool {
        let Some(r) = self.region.as_mut().filter(|r| r.dirty()) else {
            return true;
        };
        if let Err(e) = region::save(&self.region_conf, &r.want) {
            log::warn!("{}: {e}", self.region_conf.display());
            self.fail("The language could not be saved to locale.conf.");
            return false;
        }
        r.loaded = r.want.clone();
        let language = r.want.language.clone();
        self.start_set_language(move || set_language(language));
        true
    }

    pub(crate) fn revert_region(&mut self) {
        if let Some(r) = &mut self.region {
            r.want = r.loaded.clone();
        }
        self.show_region(true);
    }

    #[cfg(not(test))]
    fn start_region_read(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.region_read = Some(self.spawn(job));
    }

    #[cfg(not(test))]
    fn start_region_set(&mut self, job: impl FnOnce() -> Set + Send + 'static) {
        self.region_set = Some(self.spawn(job));
    }

    #[cfg(not(test))]
    fn start_region_reread(&mut self) {
        let rx = self.spawn(read_system);
        if let Some(r) = &mut self.region {
            r.reread = Some(rx);
        }
    }

    #[cfg(not(test))]
    fn start_set_language(&mut self, job: impl FnOnce() + Send + 'static) {
        std::thread::spawn(job);
    }

    /// Tests feed `region_read` and `region_set` themselves, so nothing reaches the system bus.
    #[cfg(test)]
    fn start_region_read(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}

    #[cfg(test)]
    fn start_region_set(&mut self, _: impl FnOnce() -> Set + Send + 'static) {}

    /// A read that gets no answer, so a test sees it start and feeds its own.
    #[cfg(test)]
    fn start_region_reread(&mut self) {
        if let Some(r) = &mut self.region {
            r.reread = Some(std::sync::mpsc::channel().1);
        }
    }

    #[cfg(test)]
    fn start_set_language(&mut self, _: impl FnOnce() + Send + 'static) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_choice_is_its_file_else_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("locale.conf");
        let env = |k: &str| match k {
            "LANG" => Some("en_US.UTF-8".to_string()),
            "LC_TIME" => Some("en_GB.utf8".to_string()),
            _ => None,
        };
        let from_env = user_choice(&conf, env);
        assert_eq!(
            from_env,
            Choice {
                language: "en_US.UTF-8".into(),
                formats: Some("en_GB.UTF-8".into()),
            }
        );
        std::fs::write(&conf, "LANG=de_DE.UTF-8\n").unwrap();
        assert_eq!(
            user_choice(&conf, env),
            Choice {
                language: "de_DE.UTF-8".into(),
                formats: None,
            },
            "the file's LANG stands for the whole locale"
        );
        std::fs::write(&conf, "LC_TIME=de_DE.UTF-8\n").unwrap();
        assert_eq!(
            user_choice(&conf, env).formats.as_deref(),
            Some("de_DE.UTF-8"),
            "a file without LANG keeps the inherited one"
        );
        assert_eq!(
            user_choice(&dir.path().join("none"), |_| None).language,
            "C.UTF-8"
        );
    }

    #[test]
    fn formats_that_follow_the_language_go_to_localed_as_its_locale() {
        let system = vec![
            ("LANG".to_string(), "en_US.UTF-8".to_string()),
            ("LC_TIME".into(), "hu_HU.UTF-8".into()),
            ("LC_NAME".into(), "hu_HU.UTF-8".into()),
        ];
        let de = Choice {
            language: "de_DE.UTF-8".into(),
            formats: None,
        };
        let sent = localed_pairs(&de, &system);
        let get = |k: &str| sent.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("LANG"), Some("de_DE.UTF-8"));
        for f in region::FORMATS {
            assert_eq!(get(f), Some("de_DE.UTF-8"), "{f}");
        }
        assert_eq!(get("LC_NAME"), Some("hu_HU.UTF-8"), "the rest stays");
    }

    #[test]
    fn a_locale_in_use_is_offered_even_when_not_listed() {
        let l = with_names(
            vec!["en_US.UTF-8".into()],
            &["xx_NOPE.UTF-8", "en_US.UTF-8"],
        );
        let codes: Vec<&str> = l.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes.len(), 2);
        let name = |code: &str| l.iter().find(|(c, _)| c == code).unwrap().1.clone();
        assert!(name("xx_NOPE.UTF-8").ends_with(" - not installed"));
        assert!(!name("en_US.UTF-8").contains("not installed"));
        let c = with_names(vec![], &["C.UTF-8"]);
        assert_eq!(c.len(), 1);
        assert!(
            !c[0].1.contains("not installed"),
            "the C library's own locale: {}",
            c[0].1
        );
    }
}
