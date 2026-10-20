//! AccountsService over the system bus. Calls block until polkit is answered through the session agent; run them off the event loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::passwd;

const DEST: &str = "org.freedesktop.Accounts";
const PATH: &str = "/org/freedesktop/Accounts";
const IFACE: &str = "org.freedesktop.Accounts";
const USER_IFACE: &str = "org.freedesktop.Accounts.User";

const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The first and past-the-last uid of a person's account, as login.defs
/// hands them out.
const UID_MIN: u32 = 1000;
const UID_MAX: u32 = 60000;

const TYPE_STANDARD: i32 = 0;
const TYPE_ADMIN: i32 = 1;
/// AccountsService's `PasswordMode`: set at the next login.
const MODE_SET_AT_LOGIN: i32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct User {
    pub path: String,
    pub uid: u32,
    pub login: String,
    pub real_name: String,
    pub admin: bool,
    pub icon: String,
    pub home: String,
    pub you: bool,
}

impl User {
    pub fn shown_name(&self) -> &str {
        if self.real_name.trim().is_empty() {
            &self.login
        } else {
            &self.real_name
        }
    }

    pub fn kind(&self) -> &'static str {
        if self.admin {
            "Administrator"
        } else {
            "Standard user"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    RealName {
        path: String,
        name: String,
    },
    Admin {
        path: String,
        admin: bool,
    },
    /// A PNG at `file`, which the daemon copies in. For your own account
    /// an existing `~/.face` and `~/.face.icon` take the same picture.
    Picture {
        path: String,
        file: PathBuf,
        you: bool,
    },
    Password {
        path: String,
        password: String,
    },
    PasswordAtLogin {
        path: String,
    },
    OwnPassword {
        current: String,
        new: String,
    },
    Create {
        login: String,
        real_name: String,
        admin: bool,
        password: Option<String>,
    },
    Delete {
        uid: u32,
        remove_files: bool,
    },
}

async fn connect(timeout: Option<Duration>) -> zbus::Result<zbus::Connection> {
    let b = zbus::connection::Builder::system()?;
    match timeout {
        Some(t) => b.method_timeout(t).build().await,
        None => b.build().await,
    }
}

pub fn people(passwd: &str) -> Vec<String> {
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let shell = f.get(6).copied().unwrap_or("");
            let nologin = shell.ends_with("/nologin") || shell.ends_with("/false");
            ((UID_MIN..UID_MAX).contains(&uid) && !nologin).then(|| f[0].to_string())
        })
        .collect()
}

fn text(m: &HashMap<String, OwnedValue>, k: &str) -> String {
    m.get(k)
        .and_then(|v| v.downcast_ref::<&str>().ok())
        .unwrap_or_default()
        .to_string()
}

fn user_of(path: &str, m: &HashMap<String, OwnedValue>, me: u32) -> Option<User> {
    let system = m
        .get("SystemAccount")
        .and_then(|v| v.downcast_ref::<bool>().ok())
        .unwrap_or(false);
    if system {
        return None;
    }
    let uid = m.get("Uid").and_then(|v| v.downcast_ref::<u64>().ok())? as u32;
    let kind = m
        .get("AccountType")
        .and_then(|v| v.downcast_ref::<i32>().ok())
        .unwrap_or(TYPE_STANDARD);
    Some(User {
        path: path.into(),
        uid,
        login: text(m, "UserName"),
        real_name: text(m, "RealName"),
        admin: kind == TYPE_ADMIN,
        icon: text(m, "IconFile"),
        home: text(m, "HomeDirectory"),
        you: uid == me,
    })
}

pub fn sort(users: &mut [User]) {
    users.sort_by(|a, b| {
        b.you.cmp(&a.you).then_with(|| {
            a.shown_name()
                .to_lowercase()
                .cmp(&b.shown_name().to_lowercase())
        })
    });
}

/// Every person's account. AccountsService lists the ones it has seen;
/// the rest of passwd's people are looked up by name, which makes it
/// know them too.
pub fn read() -> zbus::Result<Vec<User>> {
    // SAFETY: getuid cannot fail and touches no memory.
    let me = unsafe { libc::getuid() };
    zbus::block_on(async {
        let conn = connect(Some(READ_TIMEOUT)).await?;
        let reply = conn
            .call_method(Some(DEST), PATH, Some(IFACE), "ListCachedUsers", &())
            .await?;
        let mut paths: Vec<OwnedObjectPath> = reply.body().deserialize()?;
        let mut extra: Vec<OwnedObjectPath> = Vec::new();
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        for login in people(&passwd) {
            match conn
                .call_method(
                    Some(DEST),
                    PATH,
                    Some(IFACE),
                    "FindUserByName",
                    &(login.as_str(),),
                )
                .await
            {
                Ok(r) => extra.push(r.body().deserialize()?),
                Err(e) => log::warn!("FindUserByName {login}: {e}"),
            }
        }
        if let Ok(r) = conn
            .call_method(
                Some(DEST),
                PATH,
                Some(IFACE),
                "FindUserById",
                &(i64::from(me),),
            )
            .await
        {
            extra.push(r.body().deserialize()?);
        }
        for p in extra {
            if !paths.contains(&p) {
                paths.push(p);
            }
        }
        let mut users = Vec::new();
        for p in paths {
            let reply = conn
                .call_method(
                    Some(DEST),
                    p.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "GetAll",
                    &(USER_IFACE,),
                )
                .await?;
            let m: HashMap<String, OwnedValue> = reply.body().deserialize()?;
            users.extend(user_of(p.as_str(), &m, me));
        }
        sort(&mut users);
        Ok(users)
    })
}

/// Why [`apply`] stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    Text(String),
}

impl From<String> for Failure {
    fn from(text: String) -> Failure {
        Failure::Text(text)
    }
}

pub fn apply(ops: &[Op]) -> Result<(), Failure> {
    removing_pictures(ops, |ops| {
        zbus::block_on(async {
            let conn = connect(None).await.map_err(|e| e.to_string())?;
            for op in ops {
                run(&conn, op).await?;
            }
            Ok(())
        })
    })
}

/// Runs `call`, then deletes the saved pictures its `Op::Picture`s name: the daemon has copied
/// them by the time `SetIconFile` returns, and a retried Apply saves a new one.
fn removing_pictures(
    ops: &[Op],
    call: impl FnOnce(&[Op]) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let done = call(ops);
    remove_pictures(ops);
    done
}

/// Deletes the saved pictures `ops` name, for ops that ran or were dropped.
pub fn remove_pictures(ops: &[Op]) {
    for op in ops {
        if let Op::Picture { file, .. } = op {
            let _ = std::fs::remove_file(file);
        }
    }
}

async fn run(conn: &zbus::Connection, op: &Op) -> Result<(), Failure> {
    match op {
        Op::RealName { path, name } => {
            user_call(conn, path, "SetRealName", &(name.as_str(),)).await
        }
        Op::Admin { path, admin } => {
            let t = if *admin { TYPE_ADMIN } else { TYPE_STANDARD };
            user_call(conn, path, "SetAccountType", &(t,)).await
        }
        Op::Picture { path, file, you } => {
            if *you {
                update_face(file);
            }
            let f = file.to_string_lossy();
            user_call(conn, path, "SetIconFile", &(f.as_ref(),)).await
        }
        Op::Password { path, password } => {
            let hash = passwd::crypt(password)?;
            user_call(conn, path, "SetPassword", &(hash.as_str(), "")).await
        }
        Op::PasswordAtLogin { path } => {
            user_call(conn, path, "SetPasswordMode", &(MODE_SET_AT_LOGIN,)).await
        }
        Op::OwnPassword { current, new } => {
            let (current, new) = (current.clone(), new.clone());
            // passwd talks over a terminal and waits on its own pace:
            // it runs on a thread of its own, off the bus's executor.
            let changed = std::thread::spawn(move || passwd::change_own(&current, &new))
                .join()
                .map_err(|_| "passwd stopped unexpectedly".to_string())?;
            Ok(changed?)
        }
        Op::Create {
            login,
            real_name,
            admin,
            password,
        } => {
            let t = if *admin { TYPE_ADMIN } else { TYPE_STANDARD };
            let path: OwnedObjectPath = interactive(
                conn,
                PATH,
                IFACE,
                "CreateUser",
                &(login.as_str(), real_name.as_str(), t),
            )
            .await?;
            let path = path.as_str().to_string();
            match password {
                Some(p) => {
                    let hash = passwd::crypt(p)?;
                    user_call(conn, &path, "SetPassword", &(hash.as_str(), "")).await
                }
                None => user_call(conn, &path, "SetPasswordMode", &(MODE_SET_AT_LOGIN,)).await,
            }
        }
        Op::Delete { uid, remove_files } => {
            interactive::<_, ()>(
                conn,
                PATH,
                IFACE,
                "DeleteUser",
                &(i64::from(*uid), *remove_files),
            )
            .await
        }
    }
}

async fn user_call<B>(
    conn: &zbus::Connection,
    path: &str,
    method: &str,
    body: &B,
) -> Result<(), Failure>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    interactive::<_, ()>(conn, path, USER_IFACE, method, body).await
}

/// Call `method` with the flag that lets the daemon's polkit check ask
/// for a password; without it, a change that needs one is refused
/// outright.
async fn interactive<B, R>(
    conn: &zbus::Connection,
    path: &str,
    iface: &'static str,
    method: &str,
    body: &B,
) -> Result<R, Failure>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(conn)
        .destination(DEST)
        .and_then(|b| b.path(path.to_string()))
        .and_then(|b| b.interface(iface))
        .map_err(|e| e.to_string())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .call_with_flags(
            method,
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            body,
        )
        .await
        .map_err(|e| describe(method, e))?
        .ok_or_else(|| Failure::Text(format!("{method} sent no reply")))
}

fn describe(method: &str, e: zbus::Error) -> Failure {
    match &e {
        zbus::Error::MethodError(name, msg, _) => {
            log::error!("{method}: {name}: {msg:?}");
            failure(name, msg.as_deref(), reprisebus::polkit::dismissed)
        }
        _ => {
            log::error!("{method}: {e}");
            Failure::Text(e.to_string())
        }
    }
}

/// AccountsService answers PermissionDenied for a dismissed prompt and a
/// refused password alike; `dismissed` tells them apart.
fn failure(name: &str, msg: Option<&str>, dismissed: impl FnOnce() -> bool) -> Failure {
    if !name.ends_with("PermissionDenied") {
        return Failure::Text(msg.unwrap_or(name).to_string());
    }
    if dismissed() {
        return Failure::Dismissed;
    }
    Failure::Text(reprisebus::polkit::NOT_ALLOWED.into())
}

/// Your `~/.face` and `~/.face.icon`, when you keep them, take the new
/// picture too: the Start panel reads `~/.face` first, SDDM's greeter
/// `~/.face.icon`.
fn update_face(file: &Path) {
    if let Some(home) = std::env::var_os("HOME") {
        update_faces_in(Path::new(&home), file);
    }
}

fn update_faces_in(home: &Path, file: &Path) {
    let face = home.join(".face");
    for name in [".face", ".face.icon"] {
        let dest = home.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&dest) else {
            continue;
        };
        // A `.face.icon` linked to `.face` already follows it; replacing the
        // link would only break the pairing.
        let linked = || match (std::fs::canonicalize(&dest), std::fs::canonicalize(&face)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if name == ".face.icon" && meta.file_type().is_symlink() && linked() {
            continue;
        }
        let tmp = home.join(format!("{name}.new"));
        let done = std::fs::copy(file, &tmp).and_then(|_| std::fs::rename(&tmp, &dest));
        if let Err(e) = done {
            log::warn!("~/{name}: {e}");
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn an_applied_picture_is_deleted_once_the_call_returns() {
        let file = std::env::temp_dir().join(format!(
            "reprise-accounts-applied-{}.png",
            std::process::id()
        ));
        std::fs::write(&file, "png").expect("write");
        let ops = [Op::Picture {
            path: "/org/freedesktop/Accounts/User1000".into(),
            file: file.clone(),
            you: false,
        }];
        let done = removing_pictures(&ops, |_| {
            assert!(file.exists(), "the file is there for the call");
            Err(Failure::Dismissed)
        });
        assert_eq!(done, Err(Failure::Dismissed));
        assert!(!file.exists(), "the saved picture is gone after the call");
    }

    #[test]
    fn a_dismissed_prompt_is_silent_and_a_refusal_is_told() {
        const DENIED: &str = "org.freedesktop.Accounts.Error.PermissionDenied";
        assert_eq!(
            failure(DENIED, Some("Not authorized"), || true),
            Failure::Dismissed
        );
        assert_eq!(
            failure(DENIED, Some("Not authorized"), || false),
            Failure::Text(reprisebus::polkit::NOT_ALLOWED.into())
        );
        let other = failure(
            "org.freedesktop.Accounts.Error.Failed",
            Some("running 'useradd' failed"),
            || panic!("only a PermissionDenied asks whether the prompt was dismissed"),
        );
        assert_eq!(other, Failure::Text("running 'useradd' failed".into()));
    }

    #[test]
    fn people_are_the_login_accounts_in_the_people_range() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      user:x:1000:1000:Jo Doe:/home/user:/bin/fish\n\
                      svc:x:1001:1001::/:/usr/bin/nologin\n\
                      old:x:1002:1002::/home/old:/bin/false\n\
                      nobody:x:65534:65534:Kernel Overflow User:/:/usr/bin/nologin\n\
                      sam:x:1003:1003::/home/sam:/bin/bash\n\
                      broken line\n";
        assert_eq!(people(passwd), ["user", "sam"]);
    }

    fn props(
        uid: u64,
        login: &str,
        name: &str,
        kind: i32,
        system: bool,
    ) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), OwnedValue::try_from(v).unwrap());
        };
        put("Uid", Value::from(uid));
        put("UserName", Value::from(login));
        put("RealName", Value::from(name));
        put("AccountType", Value::from(kind));
        put("SystemAccount", Value::from(system));
        put("IconFile", Value::from("/var/lib/AccountsService/icons/x"));
        put("HomeDirectory", Value::from("/home/x"));
        m
    }

    #[test]
    fn properties_become_a_user_and_system_accounts_drop_out() {
        let u = user_of("/u/1000", &props(1000, "jo", "Jo Doe", 1, false), 1000).unwrap();
        assert_eq!(
            (
                u.uid,
                u.login.as_str(),
                u.real_name.as_str(),
                u.admin,
                u.you
            ),
            (1000, "jo", "Jo Doe", true, true)
        );
        assert_eq!(u.kind(), "Administrator");
        assert_eq!(user_of("/u/5", &props(5, "svc", "", 0, true), 1000), None);
        let sam = user_of("/u/1001", &props(1001, "sam", " ", 0, false), 1000).unwrap();
        assert_eq!(
            (sam.shown_name(), sam.kind(), sam.you),
            ("sam", "Standard user", false)
        );
    }

    #[test]
    fn your_account_sorts_first_then_by_name() {
        let u = |login: &str, name: &str, you: bool| User {
            login: login.into(),
            real_name: name.into(),
            you,
            ..User::default()
        };
        let mut v = vec![
            u("zed", "", false),
            u("me", "Zoe", true),
            u("al", "Bea", false),
        ];
        sort(&mut v);
        let order: Vec<&str> = v.iter().map(|u| u.login.as_str()).collect();
        assert_eq!(order, ["me", "al", "zed"]);
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("accounts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_picture_replaces_only_the_face_files_you_keep() {
        let d = scratch("faces");
        let pic = d.join("pic.png");
        std::fs::write(&pic, b"new").unwrap();
        std::fs::write(d.join(".face.icon"), b"old").unwrap();
        update_faces_in(&d, &pic);
        assert_eq!(std::fs::read(d.join(".face.icon")).unwrap(), b"new");
        assert!(!d.join(".face").exists(), "a missing ~/.face stays missing");
        std::fs::write(d.join(".face"), b"old").unwrap();
        std::fs::remove_file(d.join(".face.icon")).unwrap();
        update_faces_in(&d, &pic);
        assert_eq!(std::fs::read(d.join(".face")).unwrap(), b"new");
        assert!(
            !d.join(".face.icon").exists(),
            "a missing ~/.face.icon stays missing"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_face_icon_linked_to_face_stays_a_link() {
        let d = scratch("link");
        let pic = d.join("pic.png");
        std::fs::write(&pic, b"new").unwrap();
        std::fs::write(d.join(".face"), b"old").unwrap();
        std::os::unix::fs::symlink(".face", d.join(".face.icon")).unwrap();
        update_faces_in(&d, &pic);
        let icon = d.join(".face.icon");
        assert!(
            std::fs::symlink_metadata(&icon)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&icon).unwrap(), b"new");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
