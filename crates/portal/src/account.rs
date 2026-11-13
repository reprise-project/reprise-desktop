//! `org.freedesktop.impl.portal.Account`: your login, name and AccountsService picture, shared
//! after the Access prompt shows them with the app's reason. Every call asks; nothing is kept.

use crate::access;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::{ObjectPath, OwnedVariant, Variant};

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const ACCOUNTS_PATH: &str = "/org/freedesktop/Accounts";
const USER_IFACE: &str = "org.freedesktop.Accounts.User";

pub struct Account {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Account")]
impl Account {
    async fn get_user_information(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        window: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, crate::Results)> {
        self.frontend.check(&hdr).await?;
        let reason = match options.get("reason") {
            Some(Variant::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        let Some(me) = blocking::unblock(whoami).await else {
            log::warn!("GetUserInformation for {app_id:?}: no account for this user");
            return Ok((crate::RESPONSE_OTHER, crate::Results::new()));
        };
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let (id, shown) = (app_id.clone(), me.clone());
        let out = blocking::unblock(move || {
            let who = crate::permit::who(&crate::remotedesktop::display_name(&id));
            access::ask(&input(&who, &window, &reason, &shown), cancel)
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        log::info!(
            "GetUserInformation for {app_id:?}: {}",
            if out.granted { "shared" } else { "cancelled" }
        );
        Ok(reply(out.granted, &me))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Me {
    login: String,
    name: String,
    picture: Option<PathBuf>,
}

/// The Access prompt for `who`: what it asks for, the account as shared, the app's `reason`,
/// Share and Cancel.
fn input(who: &str, window: &str, reason: &str, me: &Me) -> access::Input {
    let what = match me.picture {
        Some(_) => "name and picture",
        None => "name",
    };
    access::Input {
        parent: crate::dialog::parent_handle(window),
        modal: true,
        title: "Your account".into(),
        subtitle: format!("{who} wants your {what}."),
        body: reason.into(),
        icon: Some("avatar-default".into()),
        grant_label: Some("_Share".into()),
        deny_label: Some("_Cancel".into()),
        choices: Vec::new(),
        person: Some(access::Person {
            name: me.name.clone(),
            login: me.login.clone(),
            picture: me
                .picture
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        }),
        launcher: None,
    }
}

/// The portal's answer: on Share the login as `id`, the name and the picture's `file://` URI
/// (which xdg-desktop-portal hands a sandboxed app through the document portal).
fn reply(granted: bool, me: &Me) -> (u32, crate::Results) {
    let mut r = crate::Results::new();
    if !granted {
        return (crate::RESPONSE_CANCELLED, r);
    }
    let image =
        (me.picture.as_ref()).map(|p| format!("file://{}", reprisefs::trash::percent_encode(p)));
    let fields = [
        ("id", Some(&me.login)),
        ("name", Some(&me.name)),
        ("image", image.as_ref()),
    ];
    for (k, v) in fields {
        if let Some(v) = v {
            r.insert(k.into(), owned(v));
        }
    }
    (crate::RESPONSE_OK, r)
}

fn owned(s: &str) -> OwnedVariant {
    Variant::from(s).try_to_owned().expect("a string has no fd")
}

/// This user's account: AccountsService's name and picture, else the passwd entry's name and no
/// picture. Blocking.
fn whoami() -> Option<Me> {
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    match accounts_service(uid) {
        Ok(props) => Some(me_of(&props)).filter(|m| !m.login.is_empty()),
        Err(e) => {
            log::info!("no AccountsService ({e}); the passwd entry names the account");
            None
        }
    }
    .or_else(|| {
        let (login, name) = reprisefs::sysinfo::account(uid)?;
        Some(Me {
            login,
            name,
            picture: None,
        })
    })
}

fn accounts_service(uid: u32) -> zbus::Result<HashMap<String, OwnedVariant>> {
    let conn = reprisebus::system()?;
    let path: zbus::OwnedObjectPath = reprisebus::wait(conn.call_method(
        Some(ACCOUNTS),
        ACCOUNTS_PATH,
        Some(ACCOUNTS),
        "FindUserById",
        &(i64::from(uid),),
    ))?
    .body()
    .deserialize()?;
    reprisebus::wait(conn.call_method(
        Some(ACCOUNTS),
        path.as_str(),
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &(USER_IFACE,),
    ))?
    .body()
    .deserialize()
}

/// The account in AccountsService's properties: the login when there is no real name, and the
/// picture only when its file is there.
fn me_of(props: &HashMap<String, OwnedVariant>) -> Me {
    let text = |k: &str| {
        (props.get(k))
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
    };
    let login = text("UserName");
    let name = Some(text("RealName"))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| login.clone());
    let picture = Some(PathBuf::from(text("IconFile")))
        .filter(|p| p.is_absolute() && std::fs::metadata(p).is_ok_and(|m| m.is_file()));
    Me {
        login,
        name,
        picture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jane(picture: Option<&str>) -> Me {
        Me {
            login: "jane".into(),
            name: "Jane Doe".into(),
            picture: picture.map(PathBuf::from),
        }
    }

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, OwnedVariant> {
        (pairs.iter())
            .map(|(k, v)| (k.to_string(), owned(v)))
            .collect()
    }

    fn text(r: &crate::Results, k: &str) -> Option<String> {
        r.get(k)
            .map(|v| String::try_from(v.try_clone().unwrap()).unwrap())
    }

    #[test]
    fn share_answers_the_login_name_and_picture_uri() {
        let (code, r) = reply(true, &jane(Some("/var/lib/AccountsService/icons/jane doe")));
        assert_eq!(code, crate::RESPONSE_OK);
        assert_eq!(text(&r, "id").as_deref(), Some("jane"));
        assert_eq!(text(&r, "name").as_deref(), Some("Jane Doe"));
        assert_eq!(
            text(&r, "image").as_deref(),
            Some("file:///var/lib/AccountsService/icons/jane%20doe")
        );
        let (_, bare) = reply(true, &jane(None));
        assert_eq!(text(&bare, "image"), None, "no picture, no image");
    }

    #[test]
    fn cancel_shares_nothing() {
        let (code, r) = reply(false, &jane(Some("/p")));
        assert_eq!(code, crate::RESPONSE_CANCELLED);
        assert!(r.is_empty());
    }

    #[test]
    fn the_prompt_names_what_is_shared_and_carries_the_reason() {
        let pic = jane(Some("/p"));
        let i = input("Recipes", "", "To sign your recipes.", &pic);
        assert_eq!(i.subtitle, "Recipes wants your name and picture.");
        assert_eq!(i.body, "To sign your recipes.");
        assert_eq!(i.grant_label.as_deref(), Some("_Share"));
        assert_eq!(i.deny_label.as_deref(), Some("_Cancel"));
        assert!(i.choices.is_empty(), "no Remember");
        let p = i.person.unwrap();
        assert_eq!((p.name.as_str(), p.login.as_str()), ("Jane Doe", "jane"));
        assert_eq!(p.picture.as_deref(), Some("/p"));
        let i = input("Recipes", "", "", &jane(None));
        assert_eq!(i.subtitle, "Recipes wants your name.");
    }

    #[test]
    fn accounts_service_properties_become_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("jane");
        std::fs::write(&icon, b"png").unwrap();
        let icon_s = icon.to_str().unwrap();
        let me = me_of(&props(&[
            ("UserName", "jane"),
            ("RealName", " Jane Doe "),
            ("IconFile", icon_s),
        ]));
        assert_eq!(me, jane(Some(icon_s)));
        let bare = me_of(&props(&[
            ("UserName", "jane"),
            ("RealName", ""),
            ("IconFile", "/nonexistent/jane"),
        ]));
        assert_eq!(bare.name, "jane", "no real name: the login");
        assert_eq!(bare.picture, None, "a missing file is no picture");
        let folder = me_of(&props(&[
            ("UserName", "jane"),
            ("IconFile", dir.path().to_str().unwrap()),
        ]));
        assert_eq!(folder.picture, None);
    }
}
