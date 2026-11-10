//! What another session-bus peer sends, decoded within bounds. A body over [`MAX_BODY`] is refused
//! before it is read, and values land in [`Value`] and [`Dict`], which keep numbers, strings and
//! string lists, cut each, and skip every other type unread. An `OwnedVariant` holds one `Variant`
//! per array element, so a 64 MiB `ay` decoded into one takes gigabytes.

use serde::de::{self, Deserialize, DeserializeOwned, Deserializer, IgnoredAny, MapAccess};
use serde::de::{SeqAccess, Visitor};
use std::collections::HashMap;
use zbus::message::Body;
use zbus::proxy::CacheProperties;
use zbus::{Connection, Message, Proxy, Signature, Type, Variant, as_variant};

/// The largest body decoded: a menu, a property reply or a signal of a program is far below it.
pub const MAX_BODY: usize = 4 << 20;
/// Bytes kept of a string, cut at a char boundary.
pub const MAX_STR: usize = 4096;
/// Strings kept of a string list.
pub const MAX_STRS: usize = 16;
/// Entries kept of a dictionary.
pub const MAX_ENTRIES: usize = 64;
/// A longer key is skipped with its value.
pub const MAX_KEY: usize = 255;

#[derive(Debug)]
pub enum Refused {
    /// The body's length in bytes, over the cap.
    TooLarge(usize),
    Bus(zbus::Error),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::TooLarge(n) => write!(f, "a body of {n} bytes, over {MAX_BODY}"),
            Refused::Bus(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Refused {}

impl From<zbus::Error> for Refused {
    fn from(e: zbus::Error) -> Self {
        Refused::Bus(e)
    }
}

/// `body`, unless it is over `cap` bytes.
pub fn within(body: Body, cap: usize) -> Result<Body, Refused> {
    match body.len() {
        n if n > cap => Err(Refused::TooLarge(n)),
        _ => Ok(body),
    }
}

/// `msg`'s body as `T`, when it is at most [`MAX_BODY`] bytes.
pub fn decode<T: DeserializeOwned + Type>(msg: &Message) -> Result<T, Refused> {
    Ok(within(msg.body(), MAX_BODY)?.deserialize::<T>()?)
}

/// A proxy that reads no property on its own; zbus's cache decodes a `GetAll` into `OwnedVariant`s.
pub async fn proxy(
    conn: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::<Proxy<'static>>::new(conn)
        .destination(destination.to_string())
        .path(path.to_string())
        .interface(interface.to_string())
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// The reply body of `Properties.Get` for `name` on `proxy`'s interface, for decoding in place.
pub async fn property_body(proxy: &Proxy<'_>, name: &str) -> Result<Body, Refused> {
    let reply = proxy
        .connection()
        .call_method(
            Some(proxy.destination()),
            proxy.path(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(proxy.interface(), name),
        )
        .await?;
    within(reply.body(), MAX_BODY)
}

/// Property `name` as `T`; a value of another type is refused before it is read.
pub async fn property<T: DeserializeOwned + Type>(
    proxy: &Proxy<'_>,
    name: &str,
) -> Result<T, Refused> {
    let body = property_body(proxy, name).await?;
    Ok(body.deserialize::<as_variant::Deserialize<T>>()?.0)
}

/// Every property of `proxy`'s interface, through `Properties.GetAll`.
pub async fn properties(proxy: &Proxy<'_>) -> Result<Dict, Refused> {
    let reply = proxy
        .connection()
        .call_method(
            Some(proxy.destination()),
            proxy.path(),
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &(proxy.interface(),),
        )
        .await?;
    Ok(within(reply.body(), MAX_BODY)?.deserialize::<Dict>()?)
}

/// `names` of `proxy`'s interface: one `GetAll`, else a `Get` each when that is over
/// [`MAX_BODY`], one still over it reading as [`Value::Other`] and one the peer lacks left out.
pub async fn properties_of(proxy: &Proxy<'_>, names: &[&str]) -> Result<Dict, Refused> {
    match properties(proxy).await {
        Err(Refused::TooLarge(_)) => {}
        all => return all,
    }
    let mut out = Vec::new();
    for &name in names {
        let value = match property_body(proxy, name).await {
            Ok(body) => body.deserialize::<Value>()?,
            Err(Refused::TooLarge(_)) => Value::Other,
            Err(Refused::Bus(zbus::Error::MethodError(..) | zbus::Error::FDO(_))) => continue,
            Err(e) => return Err(e),
        };
        out.push((name.to_string(), value));
    }
    Ok(Dict(out))
}

/// A variant's value: a number, a string, a string list or an `a{sv}`, each cut; any other type
/// is `Other`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    /// Every integer type; a `u64` past `i64::MAX` reads as `i64::MAX`.
    Int(i64),
    Float(f64),
    Str(String),
    Strs(Vec<String>),
    Dict(Dict),
    Other,
}

impl Value {
    pub fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn float(&self) -> Option<f64> {
        match self {
            Value::Float(n) => Some(*n),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn strs(&self) -> Option<&[String]> {
        match self {
            Value::Strs(s) => Some(s),
            _ => None,
        }
    }

    pub fn dict(&self) -> Option<&Dict> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
}

impl Type for Value {
    const SIGNATURE: &'static Signature = &Signature::Variant;
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

fn clip(s: &str) -> String {
    let mut end = s.len().min(MAX_STR);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a variant")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let sig: Signature = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        fn next<'de, T: Deserialize<'de>, A: SeqAccess<'de>>(seq: &mut A) -> Result<T, A::Error> {
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(1, &ValueVisitor))
        }
        let s = &mut seq;
        Ok(match sig {
            Signature::Bool => Value::Bool(next(s)?),
            Signature::U8 => Value::Int(next::<u8, _>(s)?.into()),
            Signature::I16 => Value::Int(next::<i16, _>(s)?.into()),
            Signature::U16 => Value::Int(next::<u16, _>(s)?.into()),
            Signature::I32 => Value::Int(next::<i32, _>(s)?.into()),
            Signature::U32 => Value::Int(next::<u32, _>(s)?.into()),
            Signature::I64 => Value::Int(next(s)?),
            Signature::U64 => Value::Int(i64::try_from(next::<u64, _>(s)?).unwrap_or(i64::MAX)),
            Signature::F64 => Value::Float(next(s)?),
            Signature::Str => Value::Str(clip(next::<&str, _>(s)?)),
            sig if sig == *Strs::SIGNATURE => Value::Strs(next::<Strs, _>(s)?.0),
            sig if sig == *Dict::SIGNATURE => Value::Dict(next(s)?),
            _ => {
                next::<IgnoredAny, _>(s)?;
                Value::Other
            }
        })
    }
}

/// An `as`, its first [`MAX_STRS`] strings kept.
struct Strs(Vec<String>);

impl Type for Strs {
    const SIGNATURE: &'static Signature = <Vec<&str>>::SIGNATURE;
}

impl<'de> Deserialize<'de> for Strs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strs;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string list")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strs, A::Error> {
                let mut out = Vec::new();
                while let Some(s) = seq.next_element::<&str>()? {
                    if out.len() < MAX_STRS {
                        out.push(clip(s));
                    }
                }
                Ok(Strs(out))
            }
        }
        d.deserialize_seq(V)
    }
}

/// An `a{sv}`: its first [`MAX_ENTRIES`] entries whose key is at most [`MAX_KEY`] bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict(Vec<(String, Value)>);

impl Dict {
    /// `key`'s value; of a key sent twice, the later.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl Type for Dict {
    const SIGNATURE: &'static Signature = <HashMap<&str, Variant<'_>>>::SIGNATURE;
}

impl<'de> Deserialize<'de> for Dict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Dict;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a dictionary of variants")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Dict, A::Error> {
                let mut out = Vec::new();
                while let Some(key) = map.next_key::<&str>()? {
                    if out.len() < MAX_ENTRIES && key.len() <= MAX_KEY {
                        out.push((key.to_owned(), map.next_value::<Value>()?));
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(Dict(out))
            }
        }
        d.deserialize_map(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal<B: serde::Serialize + zbus::DynamicType>(body: &B) -> Message {
        Message::signal("/a", "org.example.A", "Update")
            .build(body)
            .unwrap()
    }

    #[test]
    fn a_body_over_the_cap_is_refused_before_it_is_read() {
        let bytes = vec![0u8; MAX_BODY];
        let body = ("x", HashMap::from([("junk", Variant::from(&bytes[..]))]));
        let refused = decode::<(String, Dict)>(&signal(&body));
        assert!(
            matches!(refused, Err(Refused::TooLarge(n)) if n > MAX_BODY),
            "{refused:?}"
        );
    }

    #[test]
    fn a_dict_keeps_numbers_strings_lists_and_dicts_and_skips_the_rest() {
        let inner = HashMap::from([("xesam:title", Variant::from("Song"))]);
        let body = HashMap::from([
            ("b", Variant::from(true)),
            ("y", Variant::from(7u8)),
            ("i", Variant::from(-3i32)),
            ("t", Variant::from(u64::MAX)),
            ("d", Variant::from(0.5f64)),
            ("s", Variant::from("text")),
            ("as", Variant::from(vec!["a", "b"])),
            ("meta", Variant::from(inner)),
            ("ay", Variant::from(&[1u8, 2, 3][..])),
            (
                "o",
                Variant::from(zbus::ObjectPath::from_static_str_unchecked("/p")),
            ),
        ]);
        let d = decode::<Dict>(&signal(&body)).unwrap();
        assert_eq!(d.get("b").and_then(Value::bool), Some(true));
        assert_eq!(d.get("y").and_then(Value::int), Some(7));
        assert_eq!(d.get("i").and_then(Value::int), Some(-3));
        assert_eq!(d.get("t").and_then(Value::int), Some(i64::MAX));
        assert_eq!(d.get("d").and_then(Value::float), Some(0.5));
        assert_eq!(d.get("s").and_then(Value::str), Some("text"));
        assert_eq!(
            d.get("as").and_then(Value::strs),
            Some(&["a".to_string(), "b".to_string()][..])
        );
        let meta = d.get("meta").and_then(Value::dict).unwrap();
        assert_eq!(meta.get("xesam:title").and_then(Value::str), Some("Song"));
        assert_eq!(d.get("ay"), Some(&Value::Other));
        assert_eq!(d.get("o"), Some(&Value::Other));
    }

    #[test]
    fn strings_lists_and_dicts_are_cut_to_their_caps() {
        let long = format!("{}é", "x".repeat(MAX_STR - 1));
        let many: Vec<String> = (0..MAX_STRS + 5).map(|i| i.to_string()).collect();
        let body = HashMap::from([
            ("long", Variant::from(long.as_str())),
            ("many", Variant::from(many.clone())),
        ]);
        let d = decode::<Dict>(&signal(&body)).unwrap();
        assert_eq!(
            d.get("long").and_then(Value::str),
            Some("x".repeat(MAX_STR - 1).as_str()),
            "cut before the char that crosses the cap"
        );
        assert_eq!(d.get("many").and_then(Value::strs), Some(&many[..MAX_STRS]));

        let mut entries: HashMap<String, Variant> = (0..MAX_ENTRIES + 5)
            .map(|i| (format!("n{i}"), Variant::from(i as u32)))
            .collect();
        entries.insert("k".repeat(MAX_KEY + 1), Variant::from(true));
        let d = decode::<Dict>(&signal(&entries)).unwrap();
        assert_eq!(d.iter().count(), MAX_ENTRIES);
        assert!(
            d.iter().all(|(k, _)| k.starts_with('n')),
            "the long key is skipped"
        );
    }

    /// [`property`] `X` of a peer that answers every call with `value`.
    fn read_property<T: DeserializeOwned + Type>(value: Variant<'static>) -> Result<T, Refused> {
        use futures_util::StreamExt;
        zbus::block_on(async {
            let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
            let (server, client) = futures_util::join!(
                zbus::connection::Builder::unix_stream(a)
                    .server(zbus::Guid::generate())
                    .p2p()
                    .build(),
                zbus::connection::Builder::unix_stream(b).p2p().build(),
            );
            let (server, client) = (server.unwrap(), client.unwrap());
            let mut calls = zbus::MessageStream::from(&server);
            let serve = async {
                while let Some(Ok(m)) = calls.next().await {
                    let _ = server.reply(&m.header(), &value).await;
                }
                std::future::pending().await
            };
            let read = async {
                let p = proxy(&client, "org.example.P", "/p", "org.example.P").await?;
                property::<T>(&p, "X").await
            };
            futures_lite::future::or(read, serve).await
        })
    }

    #[test]
    fn an_oversized_getall_falls_back_to_each_property_and_drops_only_the_large_one() {
        use futures_util::StreamExt;
        let art = "x".repeat(MAX_BODY + 1);
        let value = |name: &str| match name {
            "PlaybackStatus" => Some(Variant::from("Playing")),
            "Metadata" => Some(Variant::from(HashMap::from([(
                "mpris:artUrl",
                art.as_str(),
            )]))),
            _ => None,
        };
        let got = zbus::block_on(async {
            let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
            let (server, client) = futures_util::join!(
                zbus::connection::Builder::unix_stream(a)
                    .server(zbus::Guid::generate())
                    .p2p()
                    .build(),
                zbus::connection::Builder::unix_stream(b).p2p().build(),
            );
            let (server, client) = (server.unwrap(), client.unwrap());
            let mut calls = zbus::MessageStream::from(&server);
            let serve = async {
                while let Some(Ok(m)) = calls.next().await {
                    let h = m.header();
                    if h.member().is_some_and(|n| n.as_str() == "GetAll") {
                        let all: HashMap<&str, Variant> = ["PlaybackStatus", "Metadata"]
                            .into_iter()
                            .filter_map(|n| Some((n, value(n)?)))
                            .collect();
                        let _ = server.reply(&h, &all).await;
                        continue;
                    }
                    let (_, name): (String, String) = m.body().deserialize().unwrap();
                    let _ = match value(&name) {
                        Some(v) => server.reply(&h, &v).await,
                        None => {
                            let e = "org.freedesktop.DBus.Error.InvalidArgs";
                            server.reply_error(&h, e, &"no such property").await
                        }
                    };
                }
                std::future::pending().await
            };
            let read = async {
                let p = proxy(&client, "org.example.P", "/p", "org.example.P").await?;
                properties_of(&p, &["PlaybackStatus", "Metadata", "CanPlay"]).await
            };
            futures_lite::future::or(read, serve).await
        });
        let d = got.unwrap();
        assert_eq!(
            d.get("PlaybackStatus").and_then(Value::str),
            Some("Playing")
        );
        assert_eq!(d.get("Metadata"), Some(&Value::Other), "too large to read");
        assert_eq!(d.get("CanPlay"), None, "a property the peer lacks");
    }

    #[test]
    fn a_property_is_read_only_at_its_own_type_and_within_the_cap() {
        let s: Result<String, _> = read_property(Variant::from("Playing"));
        assert_eq!(s.unwrap(), "Playing");
        let wrong: Result<String, _> = read_property(Variant::from(vec![7u8; 16]));
        assert!(matches!(wrong, Err(Refused::Bus(_))), "{wrong:?}");
        let large: Result<String, _> = read_property(Variant::from(vec![7u8; MAX_BODY]));
        assert!(matches!(large, Err(Refused::TooLarge(_))), "{large:?}");
    }
}
