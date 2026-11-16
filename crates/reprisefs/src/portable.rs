//! Phones and cameras through gvfs's MTP and PTP volume monitors: the devices plugged in, and
//! mounting one with `gio mount` onto a FUSE folder, which [`netshare::follow`] then lists.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use zbus::OwnedVariant;
use zbus::zvariant::Type;

use crate::netshare::{self, Share};
use crate::places::Gadget;

/// The monitors gvfs-mtp and gvfs-gphoto2 install. A missing one answers with an error and
/// lists nothing.
const MONITORS: [&str; 2] = [
    "org.gtk.vfs.MTPVolumeMonitor",
    "org.gtk.vfs.GPhoto2VolumeMonitor",
];
const MONITOR_PATH: &str = "/org/gtk/Private/RemoteVolumeMonitor";
const MONITOR: &str = "org.gtk.Private.RemoteVolumeMonitor";
/// The URI schemes, and gvfs backends, of the devices listed here.
pub const SCHEMES: &[&str] = &["mtp", "gphoto2"];
/// A plug or unplug costs one read after this pause.
const SETTLE: Duration = Duration::from_millis(150);
/// A phone can be slow to open its first session, so a mount gets this long.
const MOUNT_WAIT: Duration = Duration::from_secs(60);

/// A phone or camera, plugged in or mounted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The volume monitor's name for it, else its mount's.
    pub name: String,
    pub kind: Gadget,
    /// The location `gio mount` takes: `mtp://SERIAL/`, `gphoto2://SERIAL/`.
    pub uri: String,
    /// Its FUSE folder while gvfs has it mounted.
    pub mount: Option<PathBuf>,
}

impl Device {
    /// Whether gvfs mounted `share` from this device: its mount spec's backend and host are the
    /// URI's scheme and host.
    fn is_mounted_as(&self, share: &Share) -> bool {
        split(&self.uri).is_some_and(|(scheme, host)| share.backend == scheme && share.host == host)
    }
}

/// A URI's scheme and its host, percent-decoded.
fn split(uri: &str) -> Option<(&str, String)> {
    let (scheme, rest) = uri.split_once("://")?;
    let host = rest.split('/').next().unwrap_or_default();
    let host = crate::trash::percent_decode(host);
    Some((scheme, String::from_utf8_lossy(&host).into_owned()))
}

/// Gives gvfs's mounts of phones and cameras among `shares` their devices' names and icons:
/// gvfs names every MTP mount `mtp`. One no device matches is named after its host.
pub fn dress(shares: &mut [Share], devices: &[Device]) {
    for s in shares.iter_mut().filter(|s| s.is_portable()) {
        match devices.iter().find(|d| d.is_mounted_as(s)) {
            Some(d) => {
                s.name = d.name.clone();
                s.icon = crate::places::DriveKind::Portable(d.kind)
                    .icon()
                    .to_string();
            }
            None => s.name = mount_name(s),
        }
    }
}

/// A portable mount's own name, else its host's words: `Google_Pixel_7_2A0F` as
/// `Google Pixel 7 2A0F`.
fn mount_name(s: &Share) -> String {
    match s.name.is_empty() || s.name == s.backend {
        true => s.host.replace('_', " "),
        false => s.name.clone(),
    }
}

/// `devices` with the FUSE folders of those gvfs mounted among `shares`, then the phones and
/// cameras mounted with no monitor listing them, by name.
pub fn with_mounts(devices: &[Device], shares: &[Share]) -> Vec<Device> {
    let mut out: Vec<Device> = devices
        .iter()
        .map(|d| Device {
            mount: (shares.iter())
                .find(|s| d.is_mounted_as(s))
                .map(|s| s.path.clone()),
            ..d.clone()
        })
        .collect();
    let mut alone: Vec<Device> = (shares.iter())
        .filter(|s| s.is_portable() && !devices.iter().any(|d| d.is_mounted_as(s)))
        .map(|s| Device {
            name: mount_name(s),
            kind: Gadget::from_icon(&s.icon),
            uri: format!("{}://{}/", s.backend, s.host),
            mount: Some(s.path.clone()),
        })
        .collect();
    alone.sort_by_key(|d| d.name.to_lowercase());
    out.extend(alone);
    out
}

/// Mounts the device at `uri` and gives its FUSE folder. Runs `gio` and waits on it.
pub fn mount(uri: &str) -> Result<PathBuf, String> {
    netshare::run_gio(&["mount", "--", uri], MOUNT_WAIT)?;
    netshare::local_path(uri)
        .ok_or_else(|| "The device is mounted, but gvfs gives it no folder.".to_string())
}

/// `(id, name, gicon, symbolic_gicon, uuid, activation_uri, can_mount, should_automount,
/// drive_id, mount_id, identifiers, sort_key, expansion)`.
type Volume = (
    String,
    String,
    String,
    String,
    String,
    String,
    bool,
    bool,
    String,
    String,
    HashMap<String, String>,
    String,
    HashMap<String, OwnedVariant>,
);

/// A drive's 17 fields, read only to reach the volumes after them.
#[allow(dead_code)]
#[derive(Deserialize, Type)]
struct Drive(
    String,
    String,
    String,
    String,
    bool,
    bool,
    bool,
    bool,
    bool,
    bool,
    bool,
    bool,
    u32,
    Vec<String>,
    HashMap<String, String>,
    String,
    HashMap<String, OwnedVariant>,
);

/// `(id, name, gicon, symbolic_gicon, uuid, root_uri, can_unmount, volume_id, x_content_types,
/// sort_key, expansion)`.
type Mount = (
    String,
    String,
    String,
    String,
    String,
    String,
    bool,
    String,
    Vec<String>,
    String,
    HashMap<String, OwnedVariant>,
);

/// The volumes a monitor lists that `gio mount` can take, ordered as the monitor sorts them.
fn devices(mut volumes: Vec<Volume>) -> Vec<Device> {
    volumes.sort_by(|a, b| a.11.cmp(&b.11).then_with(|| a.1.cmp(&b.1)));
    volumes
        .into_iter()
        .filter(|v| v.6 && !v.1.is_empty())
        .filter(|v| split(&v.5).is_some_and(|(scheme, _)| SCHEMES.contains(&scheme)))
        .map(|v| Device {
            kind: Gadget::from_icon(netshare::themed_icon(&v.2).unwrap_or_default()),
            name: v.1,
            uri: v.5,
            mount: None,
        })
        .collect()
}

/// The plugged-in devices on a thread of its own: `post` gets the whole list at once and again
/// after each burst of plugs and unplugs. Asking starts a monitor gvfs installed but has not
/// run yet.
pub fn follow(post: impl FnMut(Vec<Device>) + Send + 'static) {
    if let Err(e) = std::thread::Builder::new()
        .name("gvfs-devices".into())
        .spawn(move || zbus::block_on(run(post)))
    {
        log::warn!("could not start the gvfs devices thread: {e}");
    }
}

async fn run(mut post: impl FnMut(Vec<Device>)) {
    use futures_util::FutureExt;
    use futures_util::stream::{SelectAll, StreamExt};

    let conn = match reprisebus::session() {
        Ok(c) => c,
        Err(e) => {
            log::info!("no session bus ({e}); no phones or cameras to follow");
            post(Vec::new());
            return;
        }
    };
    let mut signals = SelectAll::new();
    for name in MONITORS {
        let mut rules = Vec::new();
        for member in ["VolumeAdded", "VolumeRemoved", "VolumeChanged"] {
            match zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(name)
                .interface(MONITOR)
                .member(member)
                .build()
            {
                Ok(r) => rules.push(r.into()),
                Err(e) => log::warn!("no match rule for {name} {member}: {e}"),
            }
        }
        signals.push(reprisebus::signals(&conn, name, rules).await);
    }
    loop {
        let mut all = Vec::new();
        for name in MONITORS {
            all.extend(read(&conn, name).await);
        }
        post(all);
        if signals.next().await.is_none() {
            log::warn!("the gvfs volume monitor streams all ended");
            return;
        }
        async_io::Timer::after(SETTLE).await;
        while let Some(Some(_)) = signals.next().now_or_never() {}
    }
}

async fn read(conn: &zbus::Connection, name: &str) -> Vec<Device> {
    let reply = conn
        .call_method(Some(name), MONITOR_PATH, Some(MONITOR), "List", &())
        .await;
    match reply.and_then(|m| {
        m.body()
            .deserialize::<(Vec<Drive>, Vec<Volume>, Vec<Mount>)>()
    }) {
        Ok((_, volumes, _)) => devices(volumes),
        Err(e) => {
            log::debug!("no volumes from {name} ({e})");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(name: &str, icon: &str, uri: &str, sort: &str) -> Volume {
        (
            "0x55d1".into(),
            name.into(),
            icon.into(),
            String::new(),
            String::new(),
            uri.into(),
            true,
            true,
            String::new(),
            String::new(),
            HashMap::new(),
            sort.into(),
            HashMap::new(),
        )
    }

    fn share(name: &str, backend: &str, host: &str, path: &str) -> Share {
        Share {
            name: name.into(),
            icon: "multimedia-player".into(),
            path: path.into(),
            backend: backend.into(),
            host: host.into(),
        }
    }

    #[test]
    fn a_monitors_volumes_list_as_devices_named_and_iconed_by_it() {
        let got = devices(vec![
            volume(
                "Pixel 7",
                ". GThemedIcon phone multimedia-player",
                "mtp://Google_Pixel_7_2A0F/",
                "b",
            ),
            volume(
                "Canon EOS 250D",
                "camera-photo",
                "gphoto2://%5Busb%3A001%2C005%5D/",
                "a",
            ),
            volume("Some Disk", "drive-harddisk", "file:///run/media/u/X", "c"),
        ]);
        assert_eq!(
            got,
            vec![
                Device {
                    name: "Canon EOS 250D".into(),
                    kind: Gadget::Camera,
                    uri: "gphoto2://%5Busb%3A001%2C005%5D/".into(),
                    mount: None,
                },
                Device {
                    name: "Pixel 7".into(),
                    kind: Gadget::Phone,
                    uri: "mtp://Google_Pixel_7_2A0F/".into(),
                    mount: None,
                },
            ]
        );
    }

    #[test]
    fn a_volume_that_cannot_mount_or_has_no_name_is_left_out() {
        let mut locked = volume("Pixel 7", "phone", "mtp://P/", "");
        locked.6 = false;
        assert!(devices(vec![locked, volume("", "phone", "mtp://Q/", "")]).is_empty());
    }

    #[test]
    fn a_mounted_device_gets_its_fuse_folder_from_the_mount_with_its_host() {
        let plugged = devices(vec![
            volume(
                "Pixel 7",
                "multimedia-player",
                "mtp://Google_Pixel_7_2A0F/",
                "a",
            ),
            volume(
                "EOS",
                "camera-photo",
                "gphoto2://%5Busb%3A001%2C005%5D/",
                "b",
            ),
        ]);
        let shares = [
            share(
                "Pixel 7",
                "mtp",
                "Google_Pixel_7_2A0F",
                "/run/user/1000/gvfs/mtp:host=Google_Pixel_7_2A0F",
            ),
            share(
                "public on nas",
                "smb-share",
                "nas",
                "/run/user/1000/gvfs/smb-share:server=nas,share=public",
            ),
        ];
        let got = with_mounts(&plugged, &shares);
        assert_eq!(
            got[0].mount.as_deref(),
            Some(std::path::Path::new(
                "/run/user/1000/gvfs/mtp:host=Google_Pixel_7_2A0F"
            ))
        );
        assert_eq!(got[0].kind, Gadget::Player);
        assert_eq!(got[1].mount, None, "the camera is not mounted");
        assert_eq!(got.len(), 2, "the SMB share is no device");
        let camera = share(
            "EOS",
            "gphoto2",
            "[usb:001,005]",
            "/run/user/1000/gvfs/gphoto2:host=%5Busb%3A001%2C005%5D",
        );
        assert!(with_mounts(&plugged, &[camera])[1].mount.is_some());
    }

    #[test]
    fn a_phones_mount_takes_its_devices_name_and_icon_or_its_hosts_words() {
        let plugged = devices(vec![volume(
            "Test Phone",
            "phone",
            "mtp://Reprise_Test_Test_Phone_RPT0001/",
            "",
        )]);
        let mut shares = vec![
            share(
                "mtp",
                "mtp",
                "Reprise_Test_Test_Phone_RPT0001",
                "/run/user/1000/gvfs/mtp:host=Reprise_Test_Test_Phone_RPT0001",
            ),
            share(
                "mtp",
                "mtp",
                "Other_Phone",
                "/run/user/1000/gvfs/mtp:host=Other_Phone",
            ),
            share(
                "public on nas",
                "smb-share",
                "nas",
                "/run/user/1000/gvfs/smb",
            ),
        ];
        dress(&mut shares, &plugged);
        assert_eq!(
            (shares[0].name.as_str(), shares[0].icon.as_str()),
            ("Test Phone", "phone")
        );
        assert_eq!(shares[1].name, "Other Phone");
        assert_eq!(shares[2].name, "public on nas");
        let once = shares.clone();
        dress(&mut shares, &[]);
        assert_eq!(
            shares, once,
            "a dressed mount keeps its name when the device list changes"
        );
    }

    #[test]
    fn a_mounted_phone_no_monitor_lists_still_shows() {
        let shares = [share(
            "Galaxy S24",
            "mtp",
            "SAMSUNG_SAMSUNG_Android_R5C",
            "/run/user/1000/gvfs/mtp:host=SAMSUNG_SAMSUNG_Android_R5C",
        )];
        let got = with_mounts(&[], &shares);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "Galaxy S24");
        assert_eq!(got[0].uri, "mtp://SAMSUNG_SAMSUNG_Android_R5C/");
        assert!(got[0].mount.is_some());
    }
}
