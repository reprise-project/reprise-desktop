//! The accounts the login screen lists, read through NSS and filtered by `SHELLS` and
//! `LOGIN_DEFS`; Settings offers the same accounts for automatic logon.

use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::ops::RangeInclusive;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub const SHELLS: &str = "/etc/shells";
pub const LOGIN_DEFS: &str = "/etc/login.defs";

#[derive(Clone, Debug, PartialEq)]
pub struct PasswdEntry {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
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

/// The login shells in `/etc/shells` text: every line but blank and `#` lines, kept exactly.
pub fn shells(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Passwd entries as the login screen reads them: an empty shell is `/bin/sh`, NIS `+`/`-`
/// lines go, and only the first entry of a name stays.
pub fn normalize(entries: Vec<PasswdEntry>) -> Vec<PasswdEntry> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|e| !e.name.is_empty() && !e.name.starts_with(['+', '-']))
        .filter(|e| seen.insert(e.name.clone()))
        .map(|mut e| {
            if e.shell.is_empty() {
                e.shell = "/bin/sh".into();
            }
            e
        })
        .collect()
}

/// What the listed accounts are picked from: passwd entries and the text of [`SHELLS`] and
/// [`LOGIN_DEFS`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub entries: Vec<PasswdEntry>,
    pub shells: String,
    pub login_defs: String,
}

impl Source {
    /// This machine's: the entries NSS lists and the two files, an unreadable one as empty.
    pub fn read() -> Source {
        let text = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        Source {
            entries: all(),
            shells: text(SHELLS),
            login_defs: text(LOGIN_DEFS),
        }
    }

    /// The accounts the login screen lists, sorted by name.
    pub fn listed(self) -> Vec<PasswdEntry> {
        listed(
            normalize(self.entries),
            &shells(&self.shells),
            uid_range(&self.login_defs),
        )
    }
}

/// A source with each kind of entry the filter drops or rewrites, for its callers' tests.
#[doc(hidden)]
pub fn sample() -> Source {
    let entry = |name: &str, uid: u32, shell: &str| PasswdEntry {
        name: name.into(),
        uid,
        gid: uid,
        gecos: String::new(),
        home: PathBuf::from("/home").join(name),
        shell: shell.into(),
    };
    Source {
        entries: vec![
            entry("zoe", 1001, "/bin/zsh"),
            entry("root", 0, "/bin/bash"),
            entry("anna", 1000, "/bin/bash"),
            entry("anna", 1006, "/bin/bash"),
            entry("svc", 1002, "/usr/bin/nologin"),
            entry("ftp", 1007, "/bin/false"),
            entry("old", 1003, ""),
            entry("far", 70000, "/bin/bash"),
            entry("+nis", 1004, "/bin/bash"),
            entry("", 1005, "/bin/bash"),
            entry("carl", 1008, "/bin/csh"),
        ],
        shells: "# login shells\n/bin/bash\n/bin/zsh\n/bin/sh\n/usr/bin/nologin\n/bin/false\n"
            .into(),
        login_defs: "UID_MIN 1000\nUID_MAX 60000\n".into(),
    }
}

/// Every passwd entry NSS lists. getpwent's position is process-wide: a process calls this
/// from one thread at a time.
pub fn all() -> Vec<PasswdEntry> {
    let mut entries = Vec::new();
    // SAFETY: each entry is copied out before the next getpwent call reuses its storage.
    unsafe {
        libc::setpwent();
        loop {
            let pwd = libc::getpwent();
            if pwd.is_null() {
                break;
            }
            entries.push(entry_of(&*pwd));
        }
        libc::endpwent();
    }
    entries
}

/// Copies a C passwd entry out.
///
/// # Safety
/// Each non-null field of `pwd` is a live NUL-terminated string.
unsafe fn entry_of(pwd: &libc::passwd) -> PasswdEntry {
    let field = |p: *const libc::c_char| match p.is_null() {
        true => &[][..],
        // SAFETY: the caller promises a non-null field is a NUL-terminated string.
        false => unsafe { CStr::from_ptr(p) }.to_bytes(),
    };
    let text = |p| String::from_utf8_lossy(field(p)).into_owned();
    PasswdEntry {
        name: text(pwd.pw_name),
        uid: pwd.pw_uid,
        gid: pwd.pw_gid,
        gecos: text(pwd.pw_gecos),
        home: PathBuf::from(OsStr::from_bytes(field(pwd.pw_dir))),
        shell: text(pwd.pw_shell),
    }
}

/// The passwd entry for `name` through NSS, or `None` when there is none.
pub fn lookup(name: &str) -> io::Result<Option<PasswdEntry>> {
    let Ok(cname) = CString::new(name) else {
        return Ok(None);
    };
    // SAFETY: getpwnam_r writes only through the pointers passed, which point at live locals.
    passwd(|pwd, buf, len, result| unsafe {
        libc::getpwnam_r(cname.as_ptr(), pwd, buf, len, result)
    })
}

/// The passwd entry for `uid` through NSS, or `None` when there is none.
pub fn by_uid(uid: u32) -> io::Result<Option<PasswdEntry>> {
    // SAFETY: getpwuid_r writes only through the pointers passed, which point at live locals.
    passwd(|pwd, buf, len, result| unsafe { libc::getpwuid_r(uid, pwd, buf, len, result) })
}

/// Runs a `getpw*_r` call, growing its buffer while it reports `ERANGE`.
fn passwd(
    call: impl Fn(*mut libc::passwd, *mut libc::c_char, usize, *mut *mut libc::passwd) -> i32,
) -> io::Result<Option<PasswdEntry>> {
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `libc::passwd` is a plain C struct; all zeroes is a valid value.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = call(&mut pwd, buf.as_mut_ptr().cast(), buf.len(), &mut result);
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        if result.is_null() {
            return Ok(None);
        }
        // SAFETY: the call filled `pwd` with strings in `buf`, which is still live.
        return Ok(Some(unsafe { entry_of(&pwd) }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, uid: u32, gecos: &str, shell: &str) -> PasswdEntry {
        PasswdEntry {
            name: name.into(),
            uid,
            gid: uid,
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
    fn shells_skip_blank_and_comment_lines_and_keep_the_rest_exactly() {
        let text = "# /etc/shells\n/bin/sh\n\n   \n/usr/bin/zsh \n#/bin/csh\n/bin/bash\n";
        assert_eq!(shells(text), ["/bin/sh", "/usr/bin/zsh ", "/bin/bash"]);
    }

    #[test]
    fn passwd_entries_get_sh_for_an_empty_shell_drop_nis_lines_and_repeats() {
        let entries = vec![
            entry("anna", 1000, "Anna", ""),
            entry("+", 0, "", ""),
            entry("+bert", 0, "", ""),
            entry("-carl", 0, "", ""),
            entry("anna", 1005, "Other", "/bin/zsh"),
            entry("dora", 1001, "", "/bin/bash"),
        ];
        let got = normalize(entries);
        let names: Vec<_> = got.iter().map(|e| (e.name.as_str(), e.uid)).collect();
        assert_eq!(names, [("anna", 1000), ("dora", 1001)]);
        assert_eq!(got[0].shell, "/bin/sh");
        assert_eq!(got[1].shell, "/bin/bash");
    }

    #[test]
    fn all_lists_root() {
        assert!(all().iter().any(|e| e.name == "root" && e.uid == 0));
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
    fn lookup_reads_root_and_misses_an_unknown_name() {
        let root = lookup("root").unwrap().expect("root has a passwd entry");
        assert_eq!((root.name.as_str(), root.uid, root.gid), ("root", 0, 0));
        assert_eq!(lookup("no such account").unwrap(), None);
    }

    #[test]
    fn by_uid_reads_the_account_of_a_uid() {
        assert_eq!(by_uid(0).unwrap().map(|e| e.name), Some("root".into()));
        assert_eq!(by_uid(4_000_000_123).unwrap(), None);
    }

    #[test]
    fn the_sample_lists_the_first_of_each_name_with_a_login_shell_in_the_range() {
        let listed: Vec<_> = sample()
            .listed()
            .into_iter()
            .map(|e| (e.name, e.uid, e.shell))
            .collect();
        assert_eq!(
            listed,
            [
                ("anna".into(), 1000, "/bin/bash".into()),
                ("old".into(), 1003, "/bin/sh".into()),
                ("zoe".into(), 1001, "/bin/zsh".into()),
            ]
        );
    }
}
