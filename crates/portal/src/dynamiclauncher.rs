//! `org.freedesktop.impl.portal.DynamicLauncher`: an app installs a launcher or web app.
//! PrepareInstall asks through the Access prompt with the icon and an editable name;
//! xdg-desktop-portal writes the desktop file only after Install.

use crate::access;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::{ObjectPath, Variant};

const APPLICATION: u32 = 1;
const WEBAPP: u32 = 2;
/// RequestInstallToken's refusal; 0 allows.
const NOT_ALLOWED: u32 = 2;

pub struct DynamicLauncher {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.DynamicLauncher")]
impl DynamicLauncher {
    #[allow(clippy::too_many_arguments)]
    async fn prepare_install(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        name: String,
        icon_v: Variant<'_>,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, crate::Results)> {
        self.frontend.check(&hdr).await?;
        let ask = Ask::from_call(name, &icon_v, &options);
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let (id, shown) = (app_id.clone(), ask.clone());
        let out = blocking::unblock(move || {
            let who = crate::permit::who(&crate::remotedesktop::display_name(&id));
            access::ask(&input(&who, &parent_window, &shown), cancel)
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        let (code, results) = reply(&out, &ask, &icon_v);
        log::info!(
            "PrepareInstall for {app_id:?}: {}",
            if code == crate::RESPONSE_OK {
                "installing"
            } else {
                "cancelled"
            }
        );
        Ok((code, results))
    }

    async fn request_install_token(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        app_id: String,
        _options: HashMap<String, Variant<'_>>,
    ) -> zbus::fdo::Result<u32> {
        self.frontend.check(&hdr).await?;
        let id = app_id.clone();
        let allowed = blocking::unblock(move || {
            let entry = reprisefs::apps::find_file(&format!("{id}.desktop")).is_some();
            let flatpak = (reprisefs::flatpak::installations().iter())
                .any(|i| i.join("app").join(&id).exists());
            own_program(&id, entry, flatpak)
        })
        .await;
        log::info!(
            "RequestInstallToken for {app_id:?}: {}",
            if allowed { "allowed" } else { "refused" }
        );
        Ok(if allowed { 0 } else { NOT_ALLOWED })
    }

    #[zbus(property)]
    fn supported_launcher_types(&self) -> u32 {
        APPLICATION | WEBAPP
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// Whether `app_id` installs launchers without the prompt: a `reprise-` or `org.reprise.` id with
/// a desktop entry (`entry`) and no Flatpak app of that id (`flatpak`), which could claim it.
fn own_program(app_id: &str, entry: bool, flatpak: bool) -> bool {
    let named = app_id.starts_with("reprise-") || app_id.starts_with("org.reprise.");
    named && entry && !flatpak
}

/// What PrepareInstall asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ask {
    name: String,
    editable: bool,
    /// The icon file's bytes, from the serialized `GBytesIcon`.
    icon: Vec<u8>,
    webapp: bool,
    /// A web app's address.
    target: String,
    modal: bool,
}

impl Ask {
    fn from_call(
        name: String,
        icon_v: &Variant<'_>,
        options: &HashMap<String, Variant<'_>>,
    ) -> Ask {
        let flag = |k: &str, default: bool| match options.get(k) {
            Some(Variant::Bool(b)) => *b,
            _ => default,
        };
        let target = match options.get("target") {
            Some(Variant::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        Ask {
            name,
            editable: flag("editable_name", true),
            icon: icon_bytes(icon_v).unwrap_or_default(),
            webapp: matches!(options.get("launcher_type"), Some(Variant::U32(WEBAPP))),
            target,
            modal: flag("modal", true),
        }
    }
}

/// The bytes of a `GBytesIcon` as `g_icon_serialize` gives it: `('bytes', <ay>)`.
fn icon_bytes(icon_v: &Variant<'_>) -> Option<Vec<u8>> {
    let unboxed = |mut v: &Variant<'_>| {
        while let Variant::Variant(inner) = v {
            v = inner;
        }
        v.try_to_owned().ok()
    };
    let icon = unboxed(icon_v)?;
    let Variant::Structure(s) = &*icon else {
        return None;
    };
    let [Variant::Str(kind), data] = s.fields() else {
        return None;
    };
    (kind.as_str() == "bytes").then_some(())?;
    Vec::<u8>::try_from(Variant::from(unboxed(data)?)).ok()
}

/// The longest web address the prompt shows whole.
const ADDRESS_CHARS: usize = 200;

/// A web app's `target` as the prompt shows it: only an http(s) address with no spaces, control
/// or bidi characters, cut to [`ADDRESS_CHARS`], so the app cannot write lines of its own.
fn address(target: &str) -> Option<String> {
    let lower = target.get(..8).unwrap_or(target).to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return None;
    }
    let format = |c: char| {
        matches!(c, '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    if target
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || format(c))
    {
        return None;
    }
    if target.chars().count() <= ADDRESS_CHARS {
        return Some(target.to_string());
    }
    let mut cut: String = target.chars().take(ADDRESS_CHARS - 1).collect();
    cut.push('\u{2026}');
    Some(cut)
}

/// The Access prompt for `who` installing `ask`: the launcher's icon and name, a web app's address,
/// Install and Cancel.
fn input(who: &str, window: &str, ask: &Ask) -> access::Input {
    let (title, subtitle) = if ask.webapp {
        (
            "Install web app",
            format!("{who} wants to add a web app to the Start menu."),
        )
    } else {
        (
            "Add to Start menu",
            format!("{who} wants to add a launcher to the Start menu."),
        )
    };
    let mut body = Vec::new();
    if ask.webapp
        && let Some(address) = address(&ask.target)
    {
        body.push(format!("It opens {address}."));
    }
    body.push("You can uninstall it from its right-click menu in All Programs.".to_string());
    access::Input {
        parent: crate::dialog::parent_handle(window),
        modal: ask.modal,
        title: title.into(),
        subtitle,
        body: body.join("\n"),
        icon: Some("system-software-install".into()),
        grant_label: Some("_Install".into()),
        deny_label: Some("_Cancel".into()),
        choices: Vec::new(),
        person: None,
        launcher: Some(access::Launcher {
            name: ask.name.clone(),
            editable: ask.editable,
            icon: ask.icon.clone(),
        }),
    }
}

/// The portal's answer: on Install the name as edited (the app's when left blank or not
/// editable) and the icon as the app sent it.
fn reply(out: &access::Output, ask: &Ask, icon_v: &Variant<'_>) -> (u32, crate::Results) {
    let mut r = crate::Results::new();
    if !out.granted {
        return (crate::RESPONSE_CANCELLED, r);
    }
    let edited = (out.name.as_deref())
        .filter(|_| ask.editable)
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let name = edited.unwrap_or(&ask.name);
    let Ok(icon) = Variant::Variant(Box::new(icon_v.clone())).try_to_owned() else {
        return (crate::RESPONSE_OTHER, r);
    };
    let name = Variant::from(name)
        .try_to_owned()
        .expect("a string has no fd");
    r.insert("name".into(), name);
    r.insert("icon".into(), icon);
    (crate::RESPONSE_OK, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialized(bytes: &[u8]) -> Variant<'static> {
        Variant::from(("bytes", Variant::from(bytes.to_vec())))
    }

    fn options(pairs: Vec<(&str, Variant<'static>)>) -> HashMap<String, Variant<'static>> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn maps() -> Ask {
        Ask::from_call(
            "Maps".into(),
            &serialized(b"\x89PNG"),
            &options(vec![
                ("launcher_type", Variant::U32(WEBAPP)),
                ("target", Variant::from("https://maps.example.org/")),
            ]),
        )
    }

    fn text(r: &crate::Results, k: &str) -> String {
        String::try_from(r[k].try_clone().unwrap()).unwrap()
    }

    #[test]
    fn a_call_reads_the_icon_bytes_and_options() {
        let ask = maps();
        assert_eq!(ask.icon, b"\x89PNG");
        assert!(ask.webapp && ask.editable && ask.modal);
        assert_eq!(ask.target, "https://maps.example.org/");
        let fixed = Ask::from_call(
            "Tool".into(),
            &Variant::Variant(Box::new(serialized(b"x"))),
            &options(vec![
                ("editable_name", Variant::Bool(false)),
                ("modal", Variant::Bool(false)),
            ]),
        );
        assert_eq!(fixed.icon, b"x", "a boxed icon reads the same");
        assert!(!fixed.webapp && !fixed.editable && !fixed.modal);
        let themed = Variant::from(("themed", Variant::from(vec!["web"])));
        assert_eq!(icon_bytes(&themed), None, "only a bytes icon");
    }

    #[test]
    fn the_prompt_shows_the_launcher_and_a_web_apps_address() {
        let i = input("Web", "", &maps());
        assert_eq!(i.title, "Install web app");
        assert_eq!(i.subtitle, "Web wants to add a web app to the Start menu.");
        assert_eq!(
            i.body,
            "It opens https://maps.example.org/.\n\
             You can uninstall it from its right-click menu in All Programs."
        );
        assert_eq!(i.grant_label.as_deref(), Some("_Install"));
        assert_eq!(i.deny_label.as_deref(), Some("_Cancel"));
        assert!(i.choices.is_empty(), "no Remember");
        let l = i.launcher.unwrap();
        assert_eq!((l.name.as_str(), l.editable), ("Maps", true));
        assert_eq!(l.icon, b"\x89PNG");
        let app = Ask {
            webapp: false,
            ..maps()
        };
        let i = input("Store", "", &app);
        assert_eq!(
            i.subtitle,
            "Store wants to add a launcher to the Start menu."
        );
        assert!(!i.body.contains("opens"));
    }

    #[test]
    fn a_web_apps_address_shows_only_as_one_plain_url() {
        let body = |target: &str| {
            let ask = Ask {
                target: target.into(),
                ..maps()
            };
            input("Web", "", &ask).body
        };
        let hint = "You can uninstall it from its right-click menu in All Programs.";
        for forged in [
            "https://maps.example.org/.\nReprise checked this web app.",
            "https://maps.example.org/\u{202e}gro.elpmaxe",
            "https://maps example.org/",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert_eq!(body(forged), hint, "{forged:?} shows no address");
        }
        let long = format!("https://maps.example.org/{}", "a".repeat(300));
        let shown = address(&long).unwrap();
        assert_eq!(shown.chars().count(), ADDRESS_CHARS);
        assert!(shown.starts_with("https://maps.example.org/aaa") && shown.ends_with('\u{2026}'));
        assert_eq!(
            address("HTTP://Example.org/").as_deref(),
            Some("HTTP://Example.org/")
        );
    }

    #[test]
    fn install_answers_the_edited_name_and_the_same_icon() {
        let icon = serialized(b"\x89PNG");
        let out = |name: Option<&str>| access::Output {
            granted: true,
            choices: Vec::new(),
            name: name.map(String::from),
        };
        let (code, r) = reply(&out(Some("  My maps ")), &maps(), &icon);
        assert_eq!(code, crate::RESPONSE_OK);
        assert_eq!(text(&r, "name"), "My maps");
        let back = Variant::from(r["icon"].try_clone().unwrap());
        assert_eq!(
            back,
            Variant::Variant(Box::new(icon.clone())),
            "the icon unchanged"
        );
        assert_eq!(
            text(&reply(&out(Some(" ")), &maps(), &icon).1, "name"),
            "Maps"
        );
        let fixed = Ask {
            editable: false,
            ..maps()
        };
        assert_eq!(
            text(&reply(&out(Some("Other")), &fixed, &icon).1, "name"),
            "Maps"
        );
    }

    #[test]
    fn cancel_answers_nothing() {
        let (code, r) = reply(&access::Output::default(), &maps(), &serialized(b"x"));
        assert_eq!(code, crate::RESPONSE_CANCELLED);
        assert!(r.is_empty());
    }

    #[test]
    fn only_reprises_own_installed_programs_skip_the_prompt() {
        assert!(own_program("reprise-explorer", true, false));
        assert!(own_program("org.reprise.RemoteDesktop", true, false));
        assert!(
            !own_program("org.gnome.Epiphany", true, false),
            "another program"
        );
        assert!(
            !own_program("org.reprise.Fake", true, true),
            "a Flatpak of that id"
        );
        assert!(
            !own_program("reprise-gone", false, false),
            "no desktop entry"
        );
    }
}
