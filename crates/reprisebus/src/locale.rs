//! The system locale through systemd-localed, and a user's language in AccountsService.

use zbus::Connection;

const LOCALED: &str = "org.freedesktop.locale1";
const LOCALED_PATH: &str = "/org/freedesktop/locale1";
/// The polkit action behind localed's `SetLocale`.
pub const SET_LOCALE: &str = "org.freedesktop.locale1.set-locale";

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const ACCOUNTS_PATH: &str = "/org/freedesktop/Accounts";
const USER_IFACE: &str = "org.freedesktop.Accounts.User";

/// The system locale's `LANG` and `LC_*` assignments.
pub async fn system(conn: &Connection) -> zbus::Result<Vec<(String, String)>> {
    let reply = conn
        .call_method(
            Some(LOCALED),
            LOCALED_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(LOCALED, "Locale"),
        )
        .await?;
    let v: zbus::OwnedVariant = reply.body().deserialize()?;
    let list = Vec::<String>::try_from(v)?;
    Ok(assignments(&list))
}

/// `KEY=value` strings as pairs; anything else is dropped.
pub fn assignments(list: &[String]) -> Vec<(String, String)> {
    list.iter()
        .filter_map(|s| s.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Replace the system locale with `pairs`. With `interactive`, polkit may ask for a password
/// first, and the call waits for the answer.
pub async fn set_system(
    conn: &Connection,
    pairs: &[(String, String)],
    interactive: bool,
) -> zbus::Result<()> {
    let list: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    conn.call_method(
        Some(LOCALED),
        LOCALED_PATH,
        Some(LOCALED),
        "SetLocale",
        &(list, interactive),
    )
    .await
    .map(|_| ())
}

/// [`set_system`] with interaction allowed, then the system locale as it reads after. Blocks on a
/// connection of its own with no method timeout, since a password prompt may stay up a while.
pub fn set_system_interactive(pairs: &[(String, String)]) -> zbus::Result<Vec<(String, String)>> {
    let conn = crate::connect(zbus::connection::Builder::system())?;
    crate::wait(async {
        set_system(&conn, pairs, true).await?;
        system(&conn).await
    })
}

/// Set the language AccountsService keeps for this process's user, which display managers start
/// the user's sessions in.
pub async fn set_own_language(conn: &Connection, language: &str) -> zbus::Result<()> {
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let path: zbus::OwnedObjectPath = conn
        .call_method(
            Some(ACCOUNTS),
            ACCOUNTS_PATH,
            Some(ACCOUNTS),
            "FindUserById",
            &(i64::from(uid),),
        )
        .await?
        .body()
        .deserialize()?;
    conn.call_method(
        Some(ACCOUNTS),
        path.as_str(),
        Some(USER_IFACE),
        "SetLanguage",
        &(language,),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localed_assignments_read_as_pairs() {
        let list = [
            "LANG=de_DE.UTF-8".to_string(),
            "junk".into(),
            "LC_TIME=C".into(),
        ];
        assert_eq!(
            assignments(&list),
            [
                ("LANG".to_string(), "de_DE.UTF-8".to_string()),
                ("LC_TIME".into(), "C".into())
            ]
        );
    }
}
