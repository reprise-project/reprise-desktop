//! A day's events in evolution-data-server's calendars, read-only: the source registry lists the
//! calendars shown, the calendar factory opens each for the day's objects ([`ical`] times them).

pub mod ical;

use std::collections::HashMap;

use futures_util::future::join_all;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use zbus::{Connection, Message, OwnedObjectPath, Signature, Type, Variant};

pub use ical::Occurrence;

use crate::bounded::{self, Refused};

/// The registry's and the factory's bus names lack their version (`Sources5`, `Calendar8`).
const SOURCES: &str = "org.gnome.evolution.dataserver.Sources";
const CALENDARS: &str = "org.gnome.evolution.dataserver.Calendar";
const SOURCE_MANAGER: &str = "/org/gnome/evolution/dataserver/SourceManager";
const SOURCE: &str = "org.gnome.evolution.dataserver.Source";
const FACTORY_PATH: &str = "/org/gnome/evolution/dataserver/CalendarFactory";
const FACTORY: &str = "org.gnome.evolution.dataserver.CalendarFactory";
const CALENDAR: &str = "org.gnome.evolution.dataserver.Calendar";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// The local day `date`'s bounds in seconds since the epoch and its events, unsorted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day {
    pub start: i64,
    pub end: i64,
    pub events: Vec<Occurrence>,
}

struct Names {
    sources: String,
    calendars: String,
}

/// The newest registry and factory that run now or that the bus can start.
async fn names(conn: &Connection) -> Option<Names> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let mut all: Vec<String> = Vec::new();
    for listed in [dbus.list_names().await, dbus.list_activatable_names().await] {
        all.extend(listed.ok().into_iter().flatten().map(|n| n.to_string()));
    }
    Some(Names {
        sources: newest(&all, SOURCES)?,
        calendars: newest(&all, CALENDARS)?,
    })
}

/// Of `names`, the one that is `prefix` and the highest version number.
fn newest(names: &[String], prefix: &str) -> Option<String> {
    (names.iter())
        .filter_map(|n| Some((n.strip_prefix(prefix)?.parse::<u32>().ok()?, n)))
        .max_by_key(|(v, _)| *v)
        .map(|(_, n)| n.clone())
}

/// Whether evolution-data-server runs now or the bus can start it: its package is installed.
/// Starts nothing.
pub async fn installed(conn: &Connection) -> bool {
    names(conn).await.is_some()
}

/// The events on local day `date` (year, month, day) in every calendar shown: Ok(None) when
/// evolution-data-server is not installed. A calendar that fails to open or answer is skipped;
/// each call waits at most the connection's method timeout.
pub async fn day(conn: &Connection, date: (i32, u32, u32)) -> Result<Option<Day>, String> {
    let Some(names) = names(conn).await else {
        return Ok(None);
    };
    let local = TimeZone::system();
    let date = Date::new(
        i16::try_from(date.0).map_err(|e| e.to_string())?,
        i8::try_from(date.1).map_err(|e| e.to_string())?,
        i8::try_from(date.2).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let (start, end) = ical::day_bounds(date, &local).ok_or("the day has no bounds here")?;
    let shown = calendars(conn, &names.sources).await?;
    let query = occur_query(start, end, local.iana_name());
    let read = shown
        .iter()
        .map(|uid| objects(conn, &names.calendars, uid, &query));
    let mut objects = Vec::new();
    for (uid, got) in shown.iter().zip(join_all(read).await) {
        match got {
            Ok(o) => objects.extend(o),
            Err(e) => log::info!("calendar {uid}: {e}"),
        }
    }
    let events = ical::occurrences(&objects, date, &local);
    Ok(Some(Day { start, end, events }))
}

/// The objects with an occurrence in `start..end` (seconds), a floating time read in `zone`.
fn occur_query(start: i64, end: i64, zone: Option<&str>) -> String {
    let utc = |s: i64| {
        jiff::Timestamp::from_second(s)
            .map(|t| t.strftime("%Y%m%dT%H%M%SZ").to_string())
            .unwrap_or_default()
    };
    let zone = zone
        .filter(|z| {
            z.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
        })
        .map(|z| format!(" \"{z}\""))
        .unwrap_or_default();
    format!(
        "(occur-in-time-range? (make-time \"{}\") (make-time \"{}\"){zone})",
        utc(start),
        utc(end)
    )
}

/// The UIDs of the calendars to read: picked for showing, enabled along with every source above.
async fn calendars(conn: &Connection, registry: &str) -> Result<Vec<String>, String> {
    let reply = conn
        .call_method(
            Some(registry),
            SOURCE_MANAGER,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await
        .map_err(|e| e.to_string())?;
    let sources: SourceFiles = decode(&reply)?;
    Ok(shown_calendars(&sources.0))
}

fn decode<T: serde::de::DeserializeOwned + Type>(m: &Message) -> Result<T, String> {
    bounded::decode::<T>(m).map_err(|e: Refused| e.to_string())
}

/// The UIDs of `sources` (UID, key file) with a `[Calendar]` group that is selected, whose
/// `[Data Source]` and every parent's is enabled.
fn shown_calendars<'a>(sources: &'a [(String, String)]) -> Vec<String> {
    let files: HashMap<&'a str, KeyFile<'a>> = (sources.iter())
        .map(|(uid, data)| (uid.as_str(), KeyFile::parse(data)))
        .collect();
    let enabled = |mut uid: &'a str| {
        for _ in 0..files.len() + 1 {
            let Some(f) = files.get(uid) else {
                return true;
            };
            if f.get("Data Source", "Enabled") == Some("false") {
                return false;
            }
            match f.get("Data Source", "Parent") {
                Some(p) if !p.is_empty() => uid = p,
                _ => return true,
            }
        }
        false
    };
    let mut out: Vec<String> = (files.iter())
        .filter(|(_, f)| f.has("Calendar") && f.get("Calendar", "Selected") != Some("false"))
        .filter(|(uid, _)| enabled(uid))
        .map(|(uid, _)| uid.to_string())
        .collect();
    out.sort();
    out
}

/// The day's objects in calendar `uid`, opened through `factory` and closed again.
async fn objects(
    conn: &Connection,
    factory: &str,
    uid: &str,
    query: &str,
) -> Result<Vec<String>, String> {
    let opened = conn
        .call_method(
            Some(factory),
            FACTORY_PATH,
            Some(FACTORY),
            "OpenCalendar",
            &(uid,),
        )
        .await
        .map_err(|e| e.to_string())?;
    let (path, bus): (String, String) = decode(&opened)?;
    let path = OwnedObjectPath::try_from(path).map_err(|e| e.to_string())?;
    let call = |method: &'static str| {
        let (path, bus) = (path.clone(), bus.clone());
        async move {
            conn.call_method(Some(bus.as_str()), &path, Some(CALENDAR), method, &())
                .await
                .map_err(|e| e.to_string())
        }
    };
    let got = match call("Open").await {
        Ok(_) => conn
            .call_method(
                Some(bus.as_str()),
                &path,
                Some(CALENDAR),
                "GetObjectList",
                &(query,),
            )
            .await
            .map_err(|e| e.to_string())
            .and_then(|m| decode::<Vec<String>>(&m)),
        Err(e) => Err(e),
    };
    if let Err(e) = call("Close").await {
        log::debug!("calendar {uid}: close: {e}");
    }
    got
}

/// A source's key file: `[Group]` headers over `Key=value` lines; translated keys are skipped.
struct KeyFile<'a>(Vec<(&'a str, &'a str, &'a str)>);

impl<'a> KeyFile<'a> {
    fn parse(text: &'a str) -> KeyFile<'a> {
        let mut group = "";
        let mut out = Vec::new();
        for line in text.lines().map(str::trim) {
            if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                group = g;
            } else if let Some((k, v)) = line.split_once('=')
                && !line.starts_with('#')
                && !k.contains('[')
            {
                out.push((group, k.trim(), v.trim()));
            }
        }
        KeyFile(out)
    }

    fn has(&self, group: &str) -> bool {
        self.0.iter().any(|(g, ..)| *g == group)
    }

    fn get(&self, group: &str, key: &str) -> Option<&'a str> {
        (self.0.iter())
            .find(|(g, k, _)| *g == group && *k == key)
            .map(|(.., v)| *v)
    }
}

/// `GetManagedObjects`' reply kept to each source's UID and key file (`Source.UID`, `Source.Data`);
/// every other interface and property is skipped unread.
struct SourceFiles(Vec<(String, String)>);

impl Type for SourceFiles {
    const SIGNATURE: &'static Signature =
        <HashMap<OwnedObjectPath, HashMap<&str, HashMap<&str, Variant<'_>>>>>::SIGNATURE;
}

impl<'de> Deserialize<'de> for SourceFiles {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Objects;
        impl<'de> Visitor<'de> for Objects {
            type Value = SourceFiles;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("managed objects")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<SourceFiles, A::Error> {
                let mut out = Vec::new();
                while map.next_key::<IgnoredAny>()?.is_some() {
                    out.extend(map.next_value::<Interfaces>()?.0);
                }
                Ok(SourceFiles(out))
            }
        }
        d.deserialize_map(Objects)
    }
}

/// An object's interfaces, kept to the Source interface's UID and key file.
struct Interfaces(Option<(String, String)>);

impl<'de> Deserialize<'de> for Interfaces {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Interfaces;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("interfaces")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Interfaces, A::Error> {
                let mut out = None;
                while let Some(name) = map.next_key::<&str>()? {
                    if name == SOURCE {
                        let p = map.next_value::<Props>()?;
                        out = p.uid.zip(p.data);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(Interfaces(out))
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Default)]
struct Props {
    uid: Option<String>,
    data: Option<String>,
}

impl<'de> Deserialize<'de> for Props {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Props;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("properties")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Props, A::Error> {
                let mut out = Props::default();
                while let Some(name) = map.next_key::<&str>()? {
                    match name {
                        "UID" => out.uid = map.next_value::<Text>()?.0,
                        "Data" => out.data = map.next_value::<Text>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_map(V)
    }
}

/// A variant holding a string; any other type reads as None.
struct Text(Option<String>);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Text;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a variant")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Text, A::Error> {
                let sig: Signature = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let text = if sig == Signature::Str {
                    seq.next_element::<String>()?
                } else {
                    seq.next_element::<IgnoredAny>()?;
                    None
                };
                Ok(Text(text))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_versioned_names_are_used() {
        let names: Vec<String> = [
            "org.gnome.evolution.dataserver.Sources4",
            "org.gnome.evolution.dataserver.Sources5",
            "org.gnome.evolution.dataserver.Calendar8",
            "org.gnome.evolution.dataserver.CalendarFactory",
            "org.gnome.evolution.dataserver.AddressBook10",
        ]
        .map(String::from)
        .into();
        assert_eq!(
            newest(&names, SOURCES).as_deref(),
            Some("org.gnome.evolution.dataserver.Sources5")
        );
        assert_eq!(
            newest(&names, CALENDARS).as_deref(),
            Some("org.gnome.evolution.dataserver.Calendar8")
        );
        assert_eq!(newest(&names[..2], CALENDARS), None);
    }

    #[test]
    fn shown_calendars_are_selected_and_enabled_up_the_chain() {
        let s = |uid: &str, data: &str| (uid.to_string(), data.to_string());
        let sources = [
            s(
                "local-stub",
                "[Data Source]\nDisplayName=On This Computer\nEnabled=true\n",
            ),
            s(
                "system-calendar",
                "[Data Source]\nDisplayName=Personal\nDisplayName[de]=Persönlich\nParent=local-stub\n\n[Calendar]\nBackendName=local\nSelected=true\n",
            ),
            s(
                "birthdays",
                "[Data Source]\nParent=contacts-stub\n[Calendar]\nBackendName=contacts\n",
            ),
            s(
                "hidden",
                "[Data Source]\nParent=local-stub\n[Calendar]\nSelected=false\n",
            ),
            s(
                "off",
                "[Data Source]\nParent=off-stub\n[Calendar]\nSelected=true\n",
            ),
            s("off-stub", "[Data Source]\nEnabled=false\n"),
            s(
                "contacts",
                "[Data Source]\nParent=local-stub\n[Address Book]\nBackendName=local\n",
            ),
        ];
        assert_eq!(shown_calendars(&sources), ["birthdays", "system-calendar"]);
    }

    #[test]
    fn the_query_asks_for_the_day_in_utc_with_the_local_zone() {
        let q = occur_query(1_794_524_400, 1_794_610_800, Some("Europe/Budapest"));
        assert_eq!(
            q,
            "(occur-in-time-range? (make-time \"20261112T230000Z\") (make-time \"20261113T230000Z\") \"Europe/Budapest\")"
        );
        assert!(
            !occur_query(0, 1, Some("x\") (evil")).contains("evil"),
            "a zone name is never spliced in raw"
        );
    }

    #[test]
    fn managed_objects_keep_only_each_sources_uid_and_key_file() {
        fn props(uid: &'static str, data: &'static str) -> HashMap<&'static str, Variant<'static>> {
            HashMap::from([
                ("UID", Variant::from(uid)),
                ("Data", Variant::from(data)),
                ("ConnectionStatus", Variant::from(7u32)),
            ])
        }
        let path = |p: &str| OwnedObjectPath::try_from(p).unwrap();
        let other = "org.example.Other";
        let objects = HashMap::from([
            (
                path("/s/1"),
                HashMap::from([
                    (SOURCE, props("a", "[Calendar]\n")),
                    (other, props("x", "y")),
                ]),
            ),
            (path("/s/2"), HashMap::from([(other, props("b", "z"))])),
        ]);
        let msg = Message::method_call("/a", "GetManagedObjects")
            .build(&(objects,))
            .unwrap();
        let got: SourceFiles = decode(&msg).unwrap();
        assert_eq!(got.0, [("a".to_string(), "[Calendar]\n".to_string())]);
    }
}
