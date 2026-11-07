use std::ops::RangeInclusive;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct PasswdEntry {
    pub name: String,
    pub uid: u32,
    pub gecos: String,
    pub home: PathBuf,
    pub shell: String,
}

/// `UID_MIN..=UID_MAX` from login.defs text, each `1000` and `60000` when unset.
pub fn uid_range(login_defs: &str) -> RangeInclusive<u32> {
    let (mut min, mut max) = (1000, 60000);
    for line in login_defs.lines() {
        let mut words = line.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(value) = value.parse() else { continue };
        match key {
            "UID_MIN" => min = value,
            "UID_MAX" => max = value,
            _ => {}
        }
    }
    min..=max
}

/// The accounts the login screen lists, sorted by name.
pub fn listed(
    entries: Vec<PasswdEntry>,
    shells: &[String],
    range: RangeInclusive<u32>,
) -> Vec<PasswdEntry> {
    let mut listed: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            range.contains(&e.uid)
                && shells.contains(&e.shell)
                && !e.shell.ends_with("nologin")
                && !e.shell.ends_with("false")
        })
        .collect();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    listed
}

/// The first comma field of the GECOS field, or the account name when that is empty.
pub fn real_name(e: &PasswdEntry) -> String {
    let full = e.gecos.split(',').next().unwrap_or("").trim();
    if full.is_empty() { &e.name } else { full }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, uid: u32, gecos: &str, shell: &str) -> PasswdEntry {
        PasswdEntry {
            name: name.into(),
            uid,
            gecos: gecos.into(),
            home: PathBuf::from("/home").join(name),
            shell: shell.into(),
        }
    }

    #[test]
    fn system_accounts_and_nologin_shells_are_not_listed() {
        let entries = vec![
            entry("root", 0, "", "/bin/bash"),
            entry("polkitd", 977, "", "/usr/bin/nologin"),
            entry("anna", 1000, "Anna", "/bin/bash"),
            entry("bert", 1001, "Bert", "/usr/bin/nologin"),
        ];
        let listed = listed(entries, &["/bin/bash".into()], 1000..=60000);
        let names: Vec<_> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["anna"]);
    }

    #[test]
    fn listed_accounts_are_sorted_by_name_and_need_a_login_shell() {
        let shells = ["/bin/bash".into(), "/bin/false".into()];
        let entries = vec![
            entry("zoe", 1002, "", "/bin/bash"),
            entry("anna", 1000, "", "/bin/bash"),
            entry("ftp", 1003, "", "/bin/false"),
            entry("carl", 1004, "", "/bin/zsh"),
        ];
        let names: Vec<_> = listed(entries, &shells, 1000..=60000)
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["anna", "zoe"]);
    }

    #[test]
    fn uid_range_reads_login_defs() {
        assert_eq!(uid_range("UID_MIN 2000\nUID_MAX 3000\n"), 2000..=3000);
        assert_eq!(
            uid_range("# UID_MIN 5\n\tUID_MIN\t\t1500\nGID_MIN 10\n"),
            1500..=60000
        );
        assert_eq!(uid_range(""), 1000..=60000);
    }

    #[test]
    fn real_name_takes_the_first_gecos_field() {
        assert_eq!(
            real_name(&entry("anna", 1000, "Anna Berg,Room 4,555-1234", "")),
            "Anna Berg"
        );
        assert_eq!(real_name(&entry("anna", 1000, "", "")), "anna");
        assert_eq!(real_name(&entry("anna", 1000, ",,,", "")), "anna");
    }
}
