//! `org.freedesktop.impl.portal.AppChooser`: the Open With chooser over
//! the caller's list of programs, run as our own child
//! (`reprise-portal --appchoose`, see `appchoose`). The child's stdin
//! stays open while it runs, so `UpdateChoices` refills a dialog that is
//! already up (`dialog::run_live`).

use crate::dialog;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Input {
    pub parent: Option<String>,
    pub modal: bool,
    pub file: String,
    pub content_type: Option<String>,
    /// Desktop-file ids, without `.desktop` or with it, as the caller sent them.
    pub choices: Vec<String>,
    pub last_choice: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Update {
    pub choices: Vec<String>,
}

/// What the chooser answers. A killed or failed child reads as the
/// default: nothing picked.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub choice: Option<String>,
}

pub struct AppChooser {
    live: Arc<Mutex<HashMap<String, Sender<String>>>>,
    frontend: crate::Frontend,
}

impl AppChooser {
    pub fn new(frontend: crate::Frontend) -> Self {
        AppChooser {
            live: Arc::default(),
            frontend,
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.AppChooser")]
impl AppChooser {
    #[allow(clippy::too_many_arguments)]
    async fn choose_application(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        choices: Vec<String>,
        options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr)?;
        if choices.is_empty() {
            log::warn!("AppChooser for {app_id:?}: no choices");
            return Ok((crate::RESPONSE_OTHER, HashMap::new()));
        }
        let input = Input::from_call(&parent_window, choices, &options);
        let token = match options.get("activation_token") {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let (tx, rx) = mpsc::channel();
        let key = handle.to_string();
        self.live.lock().unwrap().insert(key.clone(), tx);
        let out = blocking::unblock(move || ask(&input, rx, cancel)).await;
        self.live.lock().unwrap().remove(&key);
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        log::info!("AppChooser for {app_id:?}: {:?}", out.choice);
        Ok(reply(&out, token))
    }

    async fn update_choices(
        &self,
        handle: ObjectPath<'_>,
        choices: Vec<String>,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.frontend.check(&hdr)?;
        let Some(tx) = self.live.lock().unwrap().get(handle.as_str()).cloned() else {
            log::info!("UpdateChoices for {handle}: no chooser up");
            return Ok(());
        };
        match toml::to_string(&Update { choices }) {
            Ok(doc) => {
                let _ = tx.send(doc);
            }
            Err(e) => log::warn!("UpdateChoices: {e}"),
        }
        Ok(())
    }
}

impl Input {
    /// The chooser for one `ChooseApplication` call. `modal` defaults to
    /// true; the file line is `filename`, else the URI's name
    /// (`uri_name`), else empty.
    fn from_call(
        parent_window: &str,
        choices: Vec<String>,
        options: &HashMap<String, Value<'_>>,
    ) -> Input {
        let string = |k: &str| match options.get(k) {
            Some(Value::Str(s)) if !s.is_empty() => Some(s.to_string()),
            _ => None,
        };
        let file = string("filename")
            .or_else(|| string("uri").map(|u| uri_name(&u)))
            .unwrap_or_default();
        Input {
            parent: dialog::parent_handle(parent_window),
            modal: !matches!(options.get("modal"), Some(Value::Bool(false))),
            file,
            content_type: string("content_type"),
            choices,
            last_choice: string("last_choice"),
        }
    }
}

/// The file line for a URI: its path's last segment, unescaped, without
/// the query or fragment; empty when the path names nothing
/// ("file:///", "https://host/").
fn uri_name(uri: &str) -> String {
    let uri = uri.split(['?', '#']).next().unwrap_or_default();
    // Past "scheme://authority" only the path is left.
    let path = match uri.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]),
        None => uri.split_once(':').map_or(uri, |(_, p)| p),
    };
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    String::from_utf8_lossy(&reprisefs::trash::percent_decode(last)).into_owned()
}

fn ask(input: &Input, updates: mpsc::Receiver<String>, cancel: Arc<AtomicBool>) -> Output {
    let Ok(text) = toml::to_string(input).map_err(|e| log::warn!("appchoose input: {e}")) else {
        return Output::default();
    };
    dialog::run_live(&["--appchoose"], &text, updates, &cancel)
        .and_then(|out| {
            toml::from_str(&out)
                .map_err(|e| log::warn!("appchoose answer: {e}"))
                .ok()
        })
        .unwrap_or_default()
}

/// The D-Bus answer: 0 with `choice` (and the caller's activation token)
/// when a program was picked, 1 otherwise.
fn reply(out: &Output, token: Option<String>) -> (u32, HashMap<String, OwnedValue>) {
    let Some(choice) = &out.choice else {
        return (crate::RESPONSE_CANCELLED, HashMap::new());
    };
    let owned = |s: &str| Value::from(s).try_to_owned().expect("a plain string");
    let mut r = HashMap::new();
    r.insert("choice".to_string(), owned(choice));
    if let Some(t) = token {
        r.insert("activation_token".to_string(), owned(&t));
    }
    (crate::RESPONSE_OK, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_becomes_input() {
        let mut o: HashMap<String, Value<'_>> = HashMap::new();
        o.insert("last_choice".into(), Value::from("vim.desktop"));
        o.insert("content_type".into(), Value::from("text/plain"));
        o.insert("uri".into(), Value::from("file:///home/u/My%20Notes.txt"));
        o.insert("modal".into(), Value::Bool(false));
        let i = Input::from_call("wayland:h1", vec!["vim.desktop".into()], &o);
        assert_eq!(i.parent.as_deref(), Some("h1"));
        assert!(!i.modal);
        assert_eq!(i.file, "My Notes.txt", "the URI's last segment, unescaped");
        assert_eq!(i.content_type.as_deref(), Some("text/plain"));
        assert_eq!(i.last_choice.as_deref(), Some("vim.desktop"));
    }

    #[test]
    fn a_uri_names_its_last_segment_without_query_or_fragment() {
        assert_eq!(uri_name("file:///x/b.txt?x=1"), "b.txt");
        assert_eq!(uri_name("https://host/d/a%20b.pdf#page=2"), "a b.pdf");
        assert_eq!(uri_name("file:///x/dir/"), "dir", "a trailing slash");
        assert_eq!(
            uri_name("file:///x/what%3F.txt"),
            "what?.txt",
            "an escaped ?"
        );
        assert_eq!(uri_name("file:///"), "", "the root names nothing");
        assert_eq!(uri_name("https://host"), "", "nor does a bare host");
    }

    #[test]
    fn filename_wins_over_uri_and_modal_defaults_on() {
        let mut o: HashMap<String, Value<'_>> = HashMap::new();
        o.insert("filename".into(), Value::from("a.txt"));
        o.insert("uri".into(), Value::from("file:///x/b.txt"));
        let i = Input::from_call("", vec![], &o);
        assert_eq!(i.file, "a.txt");
        assert!(i.modal);
        assert_eq!(i.parent, None);
        assert_eq!(i.content_type, None);
    }

    #[test]
    fn input_and_update_survive_the_pipe() {
        let i = Input {
            parent: Some("h".into()),
            modal: true,
            file: "a b.txt".into(),
            content_type: Some("text/plain".into()),
            choices: vec!["x.desktop".into()],
            last_choice: None,
        };
        assert_eq!(
            toml::from_str::<Input>(&toml::to_string(&i).unwrap()).unwrap(),
            i
        );
        let u = Update {
            choices: vec!["y.desktop".into()],
        };
        assert_eq!(
            toml::from_str::<Update>(&toml::to_string(&u).unwrap()).unwrap(),
            u
        );
    }

    #[test]
    fn a_pick_answers_ok_with_the_choice_and_the_token() {
        let (code, r) = reply(
            &Output {
                choice: Some("vim.desktop".into()),
            },
            Some("tok".into()),
        );
        assert_eq!(code, crate::RESPONSE_OK);
        assert_eq!(
            r.get("choice")
                .map(|v| String::try_from(v.clone()).unwrap()),
            Some("vim.desktop".into())
        );
        assert_eq!(
            r.get("activation_token")
                .map(|v| String::try_from(v.clone()).unwrap()),
            Some("tok".into())
        );
    }

    #[test]
    fn a_dead_child_answers_cancelled() {
        let (code, r) = reply(&Output::default(), Some("tok".into()));
        assert_eq!(code, crate::RESPONSE_CANCELLED);
        assert!(r.is_empty(), "no choice and no token on cancel");
    }
}
