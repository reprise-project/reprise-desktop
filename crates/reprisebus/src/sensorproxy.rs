//! iio-sensor-proxy's accelerometer on the system bus: whether the machine has one and which
//! edge of its built-in screen points up.

use std::collections::HashMap;

use zbus::{Connection, Message, OwnedMatchRule, OwnedVariant};

pub const NAME: &str = "net.hadess.SensorProxy";
pub const PATH: &str = "/net/hadess/SensorProxy";
const IFACE: &str = "net.hadess.SensorProxy";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// How the machine is held, as `AccelerometerOrientation` names it: which edge of the screen is
/// up, the screen's top in `Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Normal,
    BottomUp,
    LeftUp,
    RightUp,
}

impl Orientation {
    /// None for "undefined" (lying flat) and any name the proxy may add.
    pub fn parse(s: &str) -> Option<Orientation> {
        Some(match s {
            "normal" => Orientation::Normal,
            "bottom-up" => Orientation::BottomUp,
            "left-up" => Orientation::LeftUp,
            "right-up" => Orientation::RightUp,
            _ => return None,
        })
    }
}

/// What a `PropertiesChanged` from the proxy says about the accelerometer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub has_accelerometer: Option<bool>,
    /// Some(None) when it turned "undefined".
    pub orientation: Option<Option<Orientation>>,
}

/// The rule for the proxy's `PropertiesChanged`, for [`crate::signals`].
pub fn rules() -> Vec<OwnedMatchRule> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path(PATH)
        .build();
    rule.ok().map(Into::into).into_iter().collect()
}

/// The accelerometer part of a `PropertiesChanged` body; None for another interface's.
pub fn change(m: &Message) -> Option<Change> {
    let (iface, changed, _): (String, HashMap<String, OwnedVariant>, Vec<String>) =
        crate::bounded::decode(m).ok()?;
    change_of(&iface, &changed)
}

fn change_of(iface: &str, changed: &HashMap<String, OwnedVariant>) -> Option<Change> {
    if iface != IFACE {
        return None;
    }
    let has_accelerometer =
        (changed.get("HasAccelerometer")).and_then(|v| bool::try_from(v.try_clone().ok()?).ok());
    let orientation = (changed.get("AccelerometerOrientation"))
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .map(|s| Orientation::parse(&s));
    Some(Change {
        has_accelerometer,
        orientation,
    })
}

async fn property<T>(conn: &Connection, name: &str) -> zbus::Result<T>
where
    T: TryFrom<OwnedVariant>,
    T::Error: Into<zbus::Error>,
{
    let reply = conn
        .call_method(Some(NAME), PATH, Some(PROPERTIES), "Get", &(IFACE, name))
        .await?;
    let v: OwnedVariant = reply.body().deserialize()?;
    T::try_from(v).map_err(Into::into)
}

/// Whether the proxy runs (the bus starts it when installed) and reports an accelerometer.
pub async fn has_accelerometer(conn: &Connection) -> bool {
    property::<bool>(conn, "HasAccelerometer")
        .await
        .unwrap_or(false)
}

/// The orientation now; None while it is undefined. Read after [`claim`]: an unclaimed sensor
/// is not read.
pub async fn orientation(conn: &Connection) -> zbus::Result<Option<Orientation>> {
    let s: String = property(conn, "AccelerometerOrientation").await?;
    Ok(Orientation::parse(&s))
}

/// Start the sensor for `conn`; the proxy stops it once every claimant released it or left the
/// bus.
pub async fn claim(conn: &Connection) -> zbus::Result<()> {
    conn.call_method(Some(NAME), PATH, Some(IFACE), "ClaimAccelerometer", &())
        .await
        .map(|_| ())
}

pub async fn release(conn: &Connection) -> zbus::Result<()> {
    conn.call_method(Some(NAME), PATH, Some(IFACE), "ReleaseAccelerometer", &())
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn props(pairs: &[(&str, Variant<'_>)]) -> HashMap<String, OwnedVariant> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    #[test]
    fn orientations_parse_and_undefined_is_none() {
        assert_eq!(Orientation::parse("normal"), Some(Orientation::Normal));
        assert_eq!(Orientation::parse("bottom-up"), Some(Orientation::BottomUp));
        assert_eq!(Orientation::parse("left-up"), Some(Orientation::LeftUp));
        assert_eq!(Orientation::parse("right-up"), Some(Orientation::RightUp));
        assert_eq!(Orientation::parse("undefined"), None);
    }

    #[test]
    fn a_change_reads_the_accelerometer_keys_of_the_proxy_alone() {
        let c = change_of(
            IFACE,
            &props(&[
                ("AccelerometerOrientation", Variant::from("left-up")),
                ("LightLevel", Variant::from(3.0)),
            ]),
        );
        assert_eq!(
            c,
            Some(Change {
                has_accelerometer: None,
                orientation: Some(Some(Orientation::LeftUp)),
            })
        );
        let c = change_of(
            IFACE,
            &props(&[
                ("HasAccelerometer", Variant::from(false)),
                ("AccelerometerOrientation", Variant::from("undefined")),
            ]),
        );
        assert_eq!(
            c,
            Some(Change {
                has_accelerometer: Some(false),
                orientation: Some(None),
            })
        );
        assert_eq!(
            change_of("net.hadess.SensorProxy.Compass", &props(&[])),
            None
        );
    }

    #[test]
    fn a_value_of_another_type_reads_as_no_change() {
        let c = change_of(
            IFACE,
            &props(&[("AccelerometerOrientation", Variant::from(1u32))]),
        );
        assert_eq!(c, Some(Change::default()));
    }
}
