//! A program's search results through `org.gnome.Shell.SearchProvider2`: the ids it finds for some
//! words, each result's name, description and icon, and opening a result or the program's own
//! search. Replies are decoded within [`crate::bounded`]'s caps.

use crate::bounded::{self, Dict};
use zbus::Connection;

const INTERFACE: &str = "org.gnome.Shell.SearchProvider2";

/// Where a provider answers on the session bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub bus_name: String,
    pub path: String,
}

/// One result as its row shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub id: String,
    pub name: String,
    /// One line; empty when the provider sends none.
    pub description: String,
    /// The icon as a GIcon's string form (`edit-copy`, `. GThemedIcon a b`, a path or `file://`
    /// URL); None for the program's own icon.
    pub gicon: Option<String>,
}

/// The words a provider is asked for: `query` split at white space.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_string).collect()
}

/// At most `max` results `addr` finds for `terms`, in its order; each of its two calls waits at
/// most the connection's method timeout.
pub async fn search(
    conn: &Connection,
    addr: &Address,
    terms: &[String],
    max: usize,
) -> Result<Vec<Meta>, bounded::Refused> {
    if terms.is_empty() || max == 0 {
        return Ok(Vec::new());
    }
    let reply = call(conn, addr, "GetInitialResultSet", &(terms,)).await?;
    let mut ids: Vec<String> = bounded::decode(&reply)?;
    ids.truncate(max);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let reply = call(conn, addr, "GetResultMetas", &(&ids,)).await?;
    let metas: Vec<Dict> = bounded::decode(&reply)?;
    Ok(in_order(&ids, &metas))
}

/// Opens result `id`, found for `terms`, in its program.
pub async fn activate(
    conn: &Connection,
    addr: &Address,
    id: &str,
    terms: &[String],
    timestamp: u32,
) -> zbus::Result<()> {
    call(conn, addr, "ActivateResult", &(id, terms, timestamp)).await?;
    Ok(())
}

/// Opens the program searching for `terms`.
pub async fn launch_search(
    conn: &Connection,
    addr: &Address,
    terms: &[String],
    timestamp: u32,
) -> zbus::Result<()> {
    call(conn, addr, "LaunchSearch", &(terms, timestamp)).await?;
    Ok(())
}

async fn call(
    conn: &Connection,
    addr: &Address,
    method: &str,
    body: &(impl zbus::export::serde::Serialize + zbus::DynamicType),
) -> zbus::Result<zbus::Message> {
    let (name, path) = (addr.bus_name.as_str(), addr.path.as_str());
    conn.call_method(Some(name), path, Some(INTERFACE), method, body)
        .await
}

/// The metas for `ids`, in `ids`' order; an id the provider sent no meta or no name for is left
/// out.
fn in_order(ids: &[String], metas: &[Dict]) -> Vec<Meta> {
    let line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: Vec<Meta> = Vec::new();
    for id in ids {
        let Some(m) = metas
            .iter()
            .find(|m| m.get("id").and_then(|v| v.str()) == Some(id))
        else {
            continue;
        };
        let Some(name) = m.get("name").and_then(|v| v.str()).map(line) else {
            continue;
        };
        if name.is_empty() || out.iter().any(|o| o.id == *id) {
            continue;
        }
        let description = m.get("description").and_then(|v| v.str()).map(line);
        out.push(Meta {
            id: id.clone(),
            name,
            description: description.unwrap_or_default(),
            gicon: (m.get("gicon").and_then(|v| v.str())).map(str::to_string),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use zbus::Variant as Value;

    #[test]
    fn the_terms_are_the_words_of_the_text() {
        assert_eq!(terms("  2 + 3 "), ["2", "+", "3"]);
        assert!(terms("   ").is_empty());
    }

    #[derive(Default)]
    struct Seen {
        initial: Vec<Vec<String>>,
        metas: Vec<Vec<String>>,
        activated: Vec<(String, Vec<String>)>,
        launched: Vec<Vec<String>>,
    }

    /// A provider finding every id in `ids`, with a meta for each but `no_meta`.
    struct Fake {
        seen: Arc<Mutex<Seen>>,
        ids: Vec<String>,
        hang: bool,
    }

    #[zbus::interface(name = "org.gnome.Shell.SearchProvider2")]
    impl Fake {
        async fn get_initial_result_set(&self, terms: Vec<String>) -> Vec<String> {
            self.seen.lock().unwrap().initial.push(terms);
            if self.hang {
                std::future::pending::<()>().await;
            }
            self.ids.clone()
        }

        fn get_subsearch_result_set(&self, _prev: Vec<String>, _terms: Vec<String>) -> Vec<String> {
            Vec::new()
        }

        fn get_result_metas(&self, ids: Vec<String>) -> Vec<HashMap<String, Value<'static>>> {
            self.seen.lock().unwrap().metas.push(ids.clone());
            (ids.iter().rev())
                .filter(|id| *id != "no-meta")
                .map(|id| {
                    let mut m = HashMap::from([
                        ("id".to_string(), Value::from(id.clone())),
                        ("name".to_string(), Value::from(format!("Result\n{id}"))),
                    ]);
                    if id == "a" {
                        m.insert("description".into(), Value::from("= 5"));
                        m.insert("gicon".into(), Value::from("accessories-calculator"));
                    }
                    m
                })
                .collect()
        }

        fn activate_result(&self, id: String, terms: Vec<String>, _timestamp: u32) {
            self.seen.lock().unwrap().activated.push((id, terms));
        }

        fn launch_search(&self, terms: Vec<String>, _timestamp: u32) {
            self.seen.lock().unwrap().launched.push(terms);
        }
    }

    const PATH: &str = "/org/example/SearchProvider";

    fn provider(ids: &[&str], hang: bool) -> (Connection, Connection, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fake = Fake {
            seen: seen.clone(),
            ids: ids.iter().map(|s| s.to_string()).collect(),
            hang,
        };
        let server = std::thread::spawn(move || {
            crate::connect(
                zbus::connection::Builder::unix_stream(a)
                    .server(zbus::Guid::generate())
                    .p2p()
                    .serve_at(PATH, fake),
            )
        });
        let client = crate::connect(
            zbus::connection::Builder::unix_stream(b)
                .p2p()
                .method_timeout(Duration::from_millis(500)),
        )
        .unwrap();
        (server.join().unwrap().unwrap(), client, seen)
    }

    fn addr() -> Address {
        Address {
            bus_name: "org.example.SearchProvider".into(),
            path: PATH.into(),
        }
    }

    #[test]
    fn a_search_reads_the_first_results_metas_in_the_providers_order() {
        let (_server, client, seen) = provider(&["a", "no-meta", "b", "c"], false);
        let words = terms("2 + 3");
        let got = crate::wait(search(&client, &addr(), &words, 3)).unwrap();
        assert_eq!(
            got,
            [
                Meta {
                    id: "a".into(),
                    name: "Result a".into(),
                    description: "= 5".into(),
                    gicon: Some("accessories-calculator".into()),
                },
                Meta {
                    id: "b".into(),
                    name: "Result b".into(),
                    description: String::new(),
                    gicon: None,
                },
            ],
            "metas back in the ids' order, one line each; an id with no meta left out"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.initial, [words]);
        assert_eq!(
            seen.metas,
            [vec!["a", "no-meta", "b"]],
            "only the first three"
        );
    }

    #[test]
    fn no_words_or_no_results_ask_for_no_metas() {
        let (_server, client, seen) = provider(&[], false);
        assert_eq!(crate::wait(search(&client, &addr(), &[], 3)).unwrap(), []);
        assert_eq!(
            crate::wait(search(&client, &addr(), &terms("x"), 3)).unwrap(),
            []
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.initial.len(), 1, "no words: no call");
        assert!(seen.metas.is_empty());
    }

    #[test]
    fn a_provider_that_never_answers_fails_at_the_calls_timeout() {
        let (_server, client, _) = provider(&["a"], true);
        let start = std::time::Instant::now();
        assert!(crate::wait(search(&client, &addr(), &terms("x"), 3)).is_err());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn opening_a_result_or_the_search_hands_the_provider_the_words() {
        let (_server, client, seen) = provider(&["a"], false);
        let words = terms("note");
        crate::wait(activate(&client, &addr(), "a", &words, 0)).unwrap();
        crate::wait(launch_search(&client, &addr(), &words, 0)).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.activated, [("a".to_string(), words.clone())]);
        assert_eq!(seen.launched, [words]);
    }
}
