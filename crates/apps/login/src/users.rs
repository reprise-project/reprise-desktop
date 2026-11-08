use std::ffi::{CStr, CString};
use std::io;
use std::path::Path;

use repriselogin::greeter::UserEntry;
pub use repriselogin::users::{PasswdEntry, Source, by_uid, lookup};

/// Holds each account's picture as a file named after the account.
pub const PICTURES: &str = "/var/lib/AccountsService/icons";

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

/// The accounts the login screen lists, read now from NSS, the shells and login.defs, and
/// `PICTURES`.
pub fn read_listed() -> Vec<UserEntry> {
    screen_entries(Source::read().listed(), Path::new(PICTURES))
}

/// The first comma field of the GECOS field, or the account name when that is empty.
pub fn real_name(e: &PasswdEntry) -> String {
    let full = e.gecos.split(',').next().unwrap_or("").trim();
    if full.is_empty() { &e.name } else { full }.to_string()
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
    let cuser = CString::new(user).map_err(|_| io::ErrorKind::InvalidInput)?;
    let groups = group_list(&cuser, account.gid)?;
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
pub(crate) fn group_list(user: &CStr, gid: u32) -> io::Result<Vec<u32>> {
    let mut groups: Vec<libc::gid_t> = vec![0; 64];
    loop {
        let mut n = libc::c_int::try_from(groups.len()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `groups` holds `n` entries, and getgrouplist writes at most `n` of them.
        let rc = unsafe { libc::getgrouplist(user.as_ptr(), gid, groups.as_mut_ptr(), &mut n) };
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
    use std::path::PathBuf;

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
    fn real_name_takes_the_first_gecos_field() {
        assert_eq!(
            real_name(&entry("anna", 1000, "Anna Berg,Room 4,555-1234", "")),
            "Anna Berg"
        );
        assert_eq!(real_name(&entry("anna", 1000, "", "")), "anna");
        assert_eq!(real_name(&entry("anna", 1000, ",,,", "")), "anna");
    }

    #[test]
    fn the_login_screen_lists_the_shared_filters_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<_> = screen_entries(repriselogin::users::sample().listed(), dir.path())
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(names, ["anna", "old", "zoe"]);
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
