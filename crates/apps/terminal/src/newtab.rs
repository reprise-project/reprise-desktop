//! What the new-tab button's menu offers and what each row starts.

use std::ffi::OsString;

use reprisekit::controls::menu::MenuItem;

use crate::settings::Settings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Shell,
    Profile(usize),
    Ssh(String),
}

/// A tab's program and the label it wears until the program sets a title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// `None` runs the login shell.
    pub argv: Option<Vec<OsString>>,
    pub name: Option<String>,
}

/// The menu's rows (the shell, each profile, then the ssh hosts after a separator) and what each
/// row opens, `None` for the separator.
pub fn menu(s: &Settings, hosts: &[String]) -> (Vec<MenuItem>, Vec<Option<Choice>>) {
    let shell = match s.shell.is_empty() {
        true => "Login Shell".to_owned(),
        false => file_name(&s.shell),
    };
    let mut rows = vec![(MenuItem::entry(shell), Some(Choice::Shell))];
    rows.extend(
        s.profiles
            .iter()
            .enumerate()
            .map(|(i, p)| (MenuItem::entry(&p.name), Some(Choice::Profile(i)))),
    );
    if !hosts.is_empty() {
        rows.push((MenuItem::Separator, None));
    }
    rows.extend(hosts.iter().map(|h| {
        (
            MenuItem::entry(format!("ssh {h}")),
            Some(Choice::Ssh(h.clone())),
        )
    }));
    rows.into_iter().unzip()
}

pub fn launch(choice: &Choice, s: &Settings) -> Launch {
    match choice {
        Choice::Shell => Launch {
            argv: Some(&s.shell)
                .filter(|sh| !sh.is_empty())
                .map(|sh| vec![sh.into()]),
            name: None,
        },
        Choice::Profile(i) => match s.profiles.get(*i) {
            // `exec` leaves the program as the tab's own process, so closing it asks nothing.
            Some(p) => Launch {
                argv: Some(vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("exec {}", p.command).into(),
                ]),
                name: Some(p.name.clone()),
            },
            None => launch(&Choice::Shell, s),
        },
        Choice::Ssh(h) => Launch {
            argv: Some(vec!["ssh".into(), h.into()]),
            name: Some(format!("ssh {h}")),
        },
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Profile;

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Entry { label, .. } => label.clone(),
                MenuItem::Separator => "-".into(),
            })
            .collect()
    }

    fn settings() -> Settings {
        Settings {
            profiles: vec![Profile {
                name: "Top".into(),
                command: "htop -d 5".into(),
            }],
            ..Settings::default()
        }
    }

    #[test]
    fn menu_lists_shell_profiles_then_hosts() {
        let (items, choices) = menu(&settings(), &["box".into()]);
        assert_eq!(labels(&items), ["Login Shell", "Top", "-", "ssh box"]);
        assert_eq!(
            choices,
            [
                Some(Choice::Shell),
                Some(Choice::Profile(0)),
                None,
                Some(Choice::Ssh("box".into()))
            ]
        );
    }

    #[test]
    fn menu_without_hosts_has_no_separator() {
        let (items, _) = menu(&Settings::default(), &[]);
        assert_eq!(labels(&items), ["Login Shell"]);
    }

    #[test]
    fn a_set_shell_names_its_row_and_runs() {
        let s = Settings {
            shell: "/usr/bin/zsh".into(),
            ..Settings::default()
        };
        assert_eq!(labels(&menu(&s, &[]).0), ["zsh"]);
        assert_eq!(
            launch(&Choice::Shell, &s).argv,
            Some(vec!["/usr/bin/zsh".into()])
        );
        assert_eq!(
            launch(&Choice::Shell, &Settings::default()),
            Launch::default()
        );
    }

    #[test]
    fn a_profile_runs_through_sh_named_for_the_profile() {
        let l = launch(&Choice::Profile(0), &settings());
        assert_eq!(
            l.argv,
            Some(vec!["/bin/sh".into(), "-c".into(), "exec htop -d 5".into()])
        );
        assert_eq!(l.name.as_deref(), Some("Top"));
    }

    #[test]
    fn a_host_runs_ssh() {
        let l = launch(&Choice::Ssh("box".into()), &Settings::default());
        assert_eq!(l.argv, Some(vec!["ssh".into(), "box".into()]));
        assert_eq!(l.name.as_deref(), Some("ssh box"));
    }
}
