use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::ops::RangeInclusive;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use repriselogin::greeter::UserEntry;

pub const SHELLS: &str = "/etc/shells";
pub const LOGIN_DEFS: &str = "/etc/login.defs";
/// Holds each account's picture as a file named after the account.
pub const PICTURES: &str = "/var/lib/AccountsService/icons";

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

/// The login screen's entries for `listed`, each with its picture when `pictures` holds one.
pub fn screen_entries(listed: Vec<PasswdEntry>, pictures: &Path) -> Vec<UserEntry> {
    listed
        .into_iter()
        .map(|e| {
            let picture =
                Some(pictures.join(&e.name)).filter(|p| !e.name.contains('/') && p.is_file());
            UserEntry {
                real_name: real_name(&e),
                name: e.name,
                picture,
                note: None,
            }
        })
        .collect()
}

/// The accounts the login screen lists, read now from NSS, `SHELLS`, `LOGIN_DEFS` and `PICTURES`.
pub fn read_listed() -> Vec<UserEntry> {
    let text = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let shells = shells(&text(SHELLS));
    let range = uid_range(&text(LOGIN_DEFS));
    screen_entries(
        listed(normalize(all()), &shells, range),
        Path::new(PICTURES),
    )
}

/// Every passwd entry NSS lists. getpwent's position is process-wide, so one thread reads it.
pub fn all() -> Vec<PasswdEntry> {
    let mut entries = Vec::new();
    // SAFETY: only the daemon's one thread walks the database, and each entry is copied out
    // before the next getpwent call reuses its storage.
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

/// The first comma field of the GECOS field, or the account name when that is empty.
pub fn real_name(e: &PasswdEntry) -> String {
    let full = e.gecos.split(',').next().unwrap_or("").trim();
    if full.is_empty() { &e.name } else { full }.to_string()
}

/// The passwd entry for `name` through NSS, or `None` when there is none.
pub fn lookup(name: &str) -> io::Result<Option<PasswdEntry>> {
    let Ok(cname) = CString::new(name) else {
        return Ok(None);
    };
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `libc::passwd` is a plain C struct; all zeroes is a valid value.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: getpwnam_r writes only through the pointers passed, which point at live locals.
        let rc = unsafe {
            libc::getpwnam_r(
                cname.as_ptr(),
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
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
        // SAFETY: getpwnam_r filled `pwd` with strings in `buf`, which is still live.
        return Ok(Some(unsafe { entry_of(&pwd) }));
    }
}

/// `user`'s uid and whether `group` is among its groups, the primary one included; `None`
/// when there is no such account. A group that does not exist has no members.
pub fn member_of(user: &str, group: &str) -> io::Result<Option<(u32, bool)>> {
    let Some(account) = lookup(user)? else {
        return Ok(None);
    };
    let Some(gid) = group_id(group)? else {
        return Ok(Some((account.uid, false)));
    };
    let groups = group_list(user, account.gid)?;
    Ok(Some((account.uid, groups.contains(&gid))))
}

/// The gid of group `name` through NSS.
fn group_id(name: &str) -> io::Result<Option<u32>> {
    let Ok(cname) = CString::new(name) else {
        return Ok(None);
    };
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `libc::group` is a plain C struct; all zeroes is a valid value.
        let mut grp: libc::group = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::group = std::ptr::null_mut();
        // SAFETY: getgrnam_r writes only through the pointers passed, which point at live locals.
        let rc = unsafe {
            libc::getgrnam_r(
                cname.as_ptr(),
                &mut grp,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        return Ok((!result.is_null()).then_some(grp.gr_gid));
    }
}

/// Every group of `user`, whose primary group is `gid`.
fn group_list(user: &str, gid: u32) -> io::Result<Vec<u32>> {
    let cuser = CString::new(user).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut groups: Vec<libc::gid_t> = vec![0; 64];
    loop {
        let mut n = libc::c_int::try_from(groups.len()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `groups` holds `n` entries, and getgrouplist writes at most `n` of them.
        let rc = unsafe { libc::getgrouplist(cuser.as_ptr(), gid, groups.as_mut_ptr(), &mut n) };
        let n = usize::try_from(n).unwrap_or(0);
        if rc >= 0 {
            groups.truncate(n);
            return Ok(groups);
        }
        if n <= groups.len() || n > 1 << 16 {
            return Err(io::Error::other("getgrouplist gave no room it needs"));
        }
        groups.resize(n, 0);
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
    fn a_picture_is_offered_only_when_it_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("anna"), "png").unwrap();
        std::fs::create_dir(dir.path().join("bert")).unwrap();
        let got = screen_entries(
            vec![
                entry("anna", 1000, "Anna Berg,,", "/bin/bash"),
                entry("bert", 1001, "", "/bin/bash"),
                entry("carl", 1002, "", "/bin/bash"),
            ],
            dir.path(),
        );
        let pictures: Vec<_> = got.iter().map(|u| u.picture.clone()).collect();
        assert_eq!(pictures, [Some(dir.path().join("anna")), None, None]);
        assert_eq!(got[0].real_name, "Anna Berg");
        assert_eq!(got[1].real_name, "bert");
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
    fn real_name_takes_the_first_gecos_field() {
        assert_eq!(
            real_name(&entry("anna", 1000, "Anna Berg,Room 4,555-1234", "")),
            "Anna Berg"
        );
        assert_eq!(real_name(&entry("anna", 1000, "", "")), "anna");
        assert_eq!(real_name(&entry("anna", 1000, ",,,", "")), "anna");
    }

    #[test]
    fn lookup_reads_root_and_misses_an_unknown_name() {
        let root = lookup("root").unwrap().expect("root has a passwd entry");
        assert_eq!((root.name.as_str(), root.uid, root.gid), ("root", 0, 0));
        assert_eq!(lookup("no such account").unwrap(), None);
    }

    #[test]
    fn member_of_reads_the_account_and_its_groups() {
        assert_eq!(member_of("root", "root").unwrap(), Some((0, true)));
        assert_eq!(
            member_of("root", "no-such-group-for-reprise").unwrap(),
            Some((0, false))
        );
        assert_eq!(member_of("no-such-user-for-reprise", "root").unwrap(), None);
    }
}
