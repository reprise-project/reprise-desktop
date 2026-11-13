//! `org.freedesktop.impl.portal.Email`: an app's message opened, unsent, in the default mail
//! program. With none, or one that fails to start, the call fails and a balloon says why.

use reprisefs::apps::{AppEntry, ExecRefused};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use zbus::{ObjectPath, Variant};

const MAILTO: &str = "x-scheme-handler/mailto";

pub struct Email {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Email")]
impl Email {
    async fn compose_email(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        options: HashMap<String, Variant<'_>>,
    ) -> zbus::fdo::Result<(u32, crate::Results)> {
        self.frontend.check(&hdr).await?;
        let mail = Mail::from_options(&options).map_err(|e| {
            log::info!("ComposeEmail for {app_id:?} refused: {e}");
            zbus::fdo::Error::InvalidArgs(e)
        })?;
        let token = activation_token(&options);
        let id = app_id.clone();
        match blocking::unblock(move || compose(&mail, token.as_deref())).await {
            Ok(client) => {
                log::info!("ComposeEmail for {app_id:?}: opened in {client}");
                Ok((crate::RESPONSE_OK, crate::Results::new()))
            }
            Err(failure) => {
                let said = failure.to_string();
                log::warn!("ComposeEmail for {app_id:?}: {said}");
                blocking::unblock(move || {
                    let who = crate::permit::who(&crate::remotedesktop::display_name(&id));
                    let (summary, body) = failure.balloon(&who);
                    if let Err(e) =
                        reprisebus::notify::notify("Reprise", "internet-mail", summary, &body)
                    {
                        log::warn!("ComposeEmail: no balloon: {e}");
                    }
                })
                .await;
                Err(zbus::fdo::Error::Failed(said))
            }
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Mail {
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body: String,
    attachments: Vec<PathBuf>,
}

impl Mail {
    /// Addresses must be plain `local@domain`, the subject one line, and each attachment a
    /// local file: the frontend's paths, or `file://` URIs.
    fn from_options(o: &HashMap<String, Variant<'_>>) -> Result<Mail, String> {
        let text = |k: &str| match o.get(k) {
            Some(Variant::Str(s)) => s.to_string(),
            _ => String::new(),
        };
        let list = |k: &str| -> Vec<String> {
            let Some(Variant::Array(a)) = o.get(k) else {
                return Vec::new();
            };
            (a.inner().iter())
                .filter_map(|v| match v {
                    Variant::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect()
        };
        let mut to = list("addresses");
        let one = text("address");
        if !one.is_empty() {
            to.insert(0, one);
        }
        let mail = Mail {
            to,
            cc: list("cc"),
            bcc: list("bcc"),
            subject: text("subject"),
            body: text("body"),
            attachments: (list("attachments").iter())
                .map(|a| attachment_path(a).ok_or_else(|| format!("attachment {a:?} is no file")))
                .collect::<Result<_, _>>()?,
        };
        let addresses = mail.to.iter().chain(&mail.cc).chain(&mail.bcc);
        if let Some(bad) = addresses.into_iter().find(|a| !is_address(a)) {
            return Err(format!("{bad:?} is no email address"));
        }
        if mail.subject.chars().any(char::is_control) {
            return Err("the subject is more than one line".into());
        }
        Ok(mail)
    }
}

/// A plain `local@domain` address: the local part in RFC 5322's atext and dots, the domain in
/// letters, digits and hyphens. Nothing a mail program reads as a header, an option or a URI.
fn is_address(a: &str) -> bool {
    let Some((local, domain)) = a.split_once('@') else {
        return false;
    };
    let atext = |c: char| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c);
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    !local.is_empty() && local.chars().all(atext) && domain.split('.').all(label)
}

fn attachment_path(a: &str) -> Option<PathBuf> {
    match a.starts_with("file:") {
        true => crate::wallpaper::file_path(a),
        false => Some(PathBuf::from(a)).filter(|p| p.is_absolute()),
    }
}

/// The caller's activation token when it is printable ASCII without spaces.
fn activation_token(o: &HashMap<String, Variant<'_>>) -> Option<String> {
    match o.get("activation_token") {
        Some(Variant::Str(s))
            if !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic()) =>
        {
            Some(s.to_string())
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Client {
    /// `-compose` with a field list.
    Thunderbird,
    /// A `mailto:` URI with `attach=` paths.
    Evolution,
    /// Through `xdg-email`, which knows the others.
    Other,
}

fn client_of(entry: &AppEntry) -> Client {
    let (program, _) = reprisefs::apps::exec_program_and_rest(&entry.exec_line);
    let program = Path::new(&program)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let id = entry.id.to_lowercase();
    let named = |n: &str| program.contains(n) || id.contains(n);
    if named("thunderbird") {
        Client::Thunderbird
    } else if named("evolution") {
        Client::Evolution
    } else {
        Client::Other
    }
}

#[derive(Debug)]
enum Failure {
    NoProgram,
    Refused(ExecRefused),
    Start { name: String, error: std::io::Error },
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::NoProgram => write!(f, "no mail program is set for {MAILTO}"),
            Failure::Refused(e) => write!(f, "{e}"),
            Failure::Start { name, error } => write!(f, "{name} did not start: {error}"),
        }
    }
}

impl Failure {
    /// The balloon's summary and body; `who` is the app that asked.
    fn balloon(&self, who: &str) -> (&'static str, String) {
        match self {
            Failure::NoProgram => (
                "No mail program",
                format!(
                    "{who} wanted to write an email. Choose a mail program on the Default \
                     Programs page of Reprise Settings."
                ),
            ),
            Failure::Refused(e) => ("The email did not open", e.to_string()),
            Failure::Start { name, error } => (
                "The email did not open",
                format!("Reprise could not start {name}: {error}."),
            ),
        }
    }
}

/// Open `mail` in the default mail program, in that program's app scope; its name on success.
fn compose(mail: &Mail, token: Option<&str>) -> Result<String, Failure> {
    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    let entry = reprisefs::props::default_app_entry(MAILTO, &apps).ok_or(Failure::NoProgram)?;
    let as_uri = || reprisefs::apps::open_args_argv(entry, "", vec![mailto(mail).into()]);
    let argv = match client_of(entry) {
        Client::Thunderbird => {
            let args = vec!["-compose".into(), compose_fields(mail).into()];
            reprisefs::apps::open_args_argv(entry, "", args)
        }
        Client::Evolution => as_uri(),
        Client::Other => Ok(xdg_email_argv(mail)),
    }
    .map_err(Failure::Refused)?;
    let started = match start(&argv, &entry.id, token) {
        // Without xdg-utils the program gets the mailto: URI itself.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && argv[0] == "xdg-email" => {
            start(&as_uri().map_err(Failure::Refused)?, &entry.id, token)
        }
        r => r,
    };
    started.map_err(|error| Failure::Start {
        name: entry.name.clone(),
        error,
    })?;
    Ok(entry.name.clone())
}

fn start(argv: &[OsString], app: &str, token: Option<&str>) -> std::io::Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(std::process::Stdio::null());
    if let Some(t) = token {
        c.env("XDG_ACTIVATION_TOKEN", t)
            .env("DESKTOP_STARTUP_ID", t);
    }
    reprisefs::scope::hold(&mut c, app)?;
    let mut child = c.spawn()?;
    // Reap it when it exits; the portal outlives it.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn encode(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn file_uri(p: &Path) -> String {
    format!("file://{}", reprisefs::trash::percent_encode(p))
}

/// Thunderbird's `-compose` list. It decodes an unquoted value, so each is percent-encoded and no
/// comma or quote ends one; the attachments' comma-separated `file://` URIs are encoded again.
fn compose_fields(m: &Mail) -> String {
    let uris: Vec<String> = m.attachments.iter().map(|p| file_uri(p)).collect();
    [
        ("to", m.to.join(",")),
        ("cc", m.cc.join(",")),
        ("bcc", m.bcc.join(",")),
        ("subject", m.subject.clone()),
        ("body", m.body.clone()),
        ("attachment", uris.join(",")),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| format!("{k}={}", encode(v.as_bytes())))
    .collect::<Vec<_>>()
    .join(",")
}

/// RFC 6068's `mailto:` with each attachment's path in an `attach=`.
fn mailto(m: &Mail) -> String {
    use std::os::unix::ffi::OsStrExt;
    let list = |l: &[String]| {
        (l.iter())
            .map(|a| encode(a.as_bytes()))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut query: Vec<String> = Vec::new();
    for (k, v) in [("cc", list(&m.cc)), ("bcc", list(&m.bcc))] {
        if !v.is_empty() {
            query.push(format!("{k}={v}"));
        }
    }
    for (k, v) in [("subject", &m.subject), ("body", &m.body)] {
        if !v.is_empty() {
            query.push(format!("{k}={}", encode(v.as_bytes())));
        }
    }
    for p in &m.attachments {
        query.push(format!("attach={}", encode(p.as_os_str().as_bytes())));
    }
    let mut uri = format!("mailto:{}", list(&m.to));
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(&query.join("&"));
    }
    uri
}

/// Every address goes behind `--to`, `--cc` or `--bcc`, which `xdg-email` encodes itself.
fn xdg_email_argv(m: &Mail) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec!["xdg-email".into(), "--utf8".into()];
    let mut pair = |flag: &str, v: OsString| {
        argv.push(flag.into());
        argv.push(v);
    };
    for (flag, list) in [("--to", &m.to), ("--cc", &m.cc), ("--bcc", &m.bcc)] {
        for a in list {
            pair(flag, a.into());
        }
    }
    for (flag, v) in [("--subject", &m.subject), ("--body", &m.body)] {
        if !v.is_empty() {
            pair(flag, v.into());
        }
    }
    for p in &m.attachments {
        pair("--attach", p.into());
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options<'a>(pairs: &[(&str, Variant<'a>)]) -> HashMap<String, Variant<'a>> {
        (pairs.iter())
            .map(|(k, v)| (k.to_string(), v.try_clone().unwrap()))
            .collect()
    }

    fn strings(v: &[&str]) -> Variant<'static> {
        Variant::from(v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn report() -> Mail {
        Mail {
            to: vec!["ann@example.org".into(), "bo@example.org".into()],
            cc: vec!["cy@example.org".into()],
            bcc: Vec::new(),
            subject: "Q3, \"final\"".into(),
            body: "Hi,\nit's attached.".into(),
            attachments: vec![PathBuf::from("/run/user/1000/doc/3f2a/q3, final.pdf")],
        }
    }

    #[test]
    fn the_options_become_a_message_with_the_single_address_first() {
        let o = options(&[
            ("address", Variant::from("ann@example.org")),
            ("addresses", strings(&["bo@example.org"])),
            ("cc", strings(&["cy@example.org"])),
            ("subject", Variant::from("Q3, \"final\"")),
            ("body", Variant::from("Hi,\nit's attached.")),
            (
                "attachments",
                strings(&["file:///run/user/1000/doc/3f2a/q3%2C%20final.pdf"]),
            ),
        ]);
        assert_eq!(Mail::from_options(&o), Ok(report()));
        let path = options(&[("attachments", strings(&["/home/u/a b.pdf"]))]);
        assert_eq!(
            Mail::from_options(&path).unwrap().attachments,
            [PathBuf::from("/home/u/a b.pdf")]
        );
    }

    #[test]
    fn an_address_a_program_could_read_as_more_is_refused() {
        for bad in [
            "",
            "ann",
            "@example.org",
            "ann@",
            "ann@-x.org",
            "ann@x..org",
            "mailto:ann@x.org",
            "ann@x.org?attach=/etc/passwd",
            "ann@x.org&attach=a",
            "Ann <ann@x.org>",
            "ann bo@x.org",
            "ann@x.org\nBcc: eve@x.org",
            "ann@x.org,eve@x.org",
        ] {
            assert!(!is_address(bad), "{bad:?}");
            let o = options(&[("cc", strings(&[bad]))]);
            assert!(Mail::from_options(&o).is_err(), "{bad:?}");
        }
        for good in ["ann@example.org", "a.b+tag@mail.example.co.uk", "-x@y.org"] {
            assert!(is_address(good), "{good:?}");
        }
    }

    #[test]
    fn a_subject_of_two_lines_or_a_relative_attachment_is_refused() {
        let two = options(&[("subject", Variant::from("Hi\nBcc: eve@x.org"))]);
        assert!(Mail::from_options(&two).is_err());
        for bad in ["a.pdf", "https://x.org/a.pdf", "file://host/a.pdf"] {
            let o = options(&[("attachments", strings(&[bad]))]);
            assert!(Mail::from_options(&o).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn thunderbird_gets_each_field_encoded_so_no_comma_or_quote_ends_it() {
        assert_eq!(
            compose_fields(&report()),
            "to=ann%40example.org%2Cbo%40example.org,cc=cy%40example.org,\
             subject=Q3%2C%20%22final%22,body=Hi%2C%0Ait%27s%20attached.,\
             attachment=file%3A%2F%2F%2Frun%2Fuser%2F1000%2Fdoc%2F3f2a%2Fq3%252C%2520final.pdf"
        );
        assert_eq!(compose_fields(&Mail::default()), "");
    }

    #[test]
    fn evolution_gets_a_mailto_with_attach_paths() {
        assert_eq!(
            mailto(&report()),
            "mailto:ann%40example.org,bo%40example.org?cc=cy%40example.org&\
             subject=Q3%2C%20%22final%22&body=Hi%2C%0Ait%27s%20attached.&\
             attach=%2Frun%2Fuser%2F1000%2Fdoc%2F3f2a%2Fq3%2C%20final.pdf"
        );
        assert_eq!(mailto(&Mail::default()), "mailto:");
    }

    #[test]
    fn xdg_email_gets_every_address_behind_a_flag() {
        let argv = xdg_email_argv(&Mail {
            to: vec!["-x@y.org".into()],
            ..report()
        });
        assert_eq!(
            argv,
            [
                "xdg-email",
                "--utf8",
                "--to",
                "-x@y.org",
                "--cc",
                "cy@example.org",
                "--subject",
                "Q3, \"final\"",
                "--body",
                "Hi,\nit's attached.",
                "--attach",
                "/run/user/1000/doc/3f2a/q3, final.pdf",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn the_mail_program_is_known_by_its_program_or_entry() {
        let entry = |id: &str, exec: &str| {
            let text = format!("[Desktop Entry]\nType=Application\nName=Mail\nExec={exec}\n");
            reprisefs::apps::parse_desktop(&text, id).unwrap()
        };
        let flatpak =
            "/usr/bin/flatpak run --command=thunderbird org.mozilla.Thunderbird @@u %u @@";
        assert_eq!(
            client_of(&entry("thunderbird.desktop", "thunderbird %u")),
            Client::Thunderbird
        );
        assert_eq!(
            client_of(&entry("org.mozilla.Thunderbird.desktop", flatpak)),
            Client::Thunderbird
        );
        assert_eq!(
            client_of(&entry("org.gnome.Evolution.desktop", "evolution %U")),
            Client::Evolution
        );
        assert_eq!(
            client_of(&entry("org.gnome.Geary.desktop", "geary %U")),
            Client::Other
        );
    }

    #[test]
    fn only_a_printable_token_is_passed_on() {
        let token = |s: &str| activation_token(&options(&[("activation_token", Variant::from(s))]));
        assert_eq!(token("abc_123"), Some("abc_123".into()));
        assert_eq!(token(""), None);
        assert_eq!(token("a b"), None);
        assert_eq!(token("a\n"), None);
    }
}
