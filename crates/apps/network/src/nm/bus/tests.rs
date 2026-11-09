use super::act::{act, hotspot_settings, import_error};
use super::call::{auth_failure, clean};
use super::wifi::{key_mgmt_for, profile_owner, rekey_mgmt, rekeyed, wifi_settings};
use super::*;
use zbus::Variant;

fn props(entries: Vec<(&str, Variant<'static>)>) -> Props {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
        .collect()
}

const DEV: &str = "/org/freedesktop/NetworkManager/Devices/2";

fn join() -> Cmd {
    Cmd::Connect {
        device: "/org/freedesktop/NetworkManager/Devices/10".into(),
        ap: "/".into(),
        ssid: "Net".into(),
        key: None,
        hidden: false,
    }
}

#[test]
fn common_network_manager_errors_read_as_sentences() {
    let nm = |name: &str, msg: &str| {
        let raw = format!("org.freedesktop.NetworkManager.{name}: {msg}");
        match clean(raw) {
            Failure::Text(t) => t,
            Failure::Dismissed => panic!("an error is never a dismissal"),
        }
    };
    let not_allowed = reprisebus::polkit::NOT_ALLOWED;
    let not_ready = "The network adapter is not ready. Make sure it is turned on and \
                     enabled, then try again.";
    assert_eq!(
        nm("PermissionDenied", "Not authorized to control networking."),
        not_allowed
    );
    assert_eq!(
        nm("Settings.PermissionDenied", "Insufficient privileges."),
        not_allowed
    );
    assert_eq!(
        nm("Failed", "Not authorized to share connections"),
        not_allowed
    );
    assert_eq!(
        nm(
            "ConnectionNotAvailable",
            "No suitable device found for this connection (device wlan0 not available \
             because device is not available)."
        ),
        not_ready
    );
    assert_eq!(
        nm("UnknownDevice", "No device found for the requested iface."),
        not_ready
    );
    assert_eq!(
        nm(
            "AgentManager.NoSecrets",
            "No agents were available for this request."
        ),
        "This connection needs a password, and none was given."
    );
    assert_eq!(
        nm(
            "ConnectionAlreadyActive",
            "This connection is already active."
        ),
        "ConnectionAlreadyActive: This connection is already active.",
        "an unmapped error keeps its text without the bus-name prefix"
    );
    assert_eq!(
        clean("org.freedesktop.zbus.Error: Not authorized to perform this operation"),
        Failure::Text(not_allowed.into()),
        "a refused property write reaches zbus as its own error name"
    );
    assert_eq!(
        clean("org.freedesktop.DBus.Error.AccessDenied: Rejected send message"),
        Failure::Text(not_allowed.into())
    );
}

#[test]
fn a_refused_stats_refresh_rate_has_no_balloon() {
    let stats = Cmd::Stats {
        device: "/org/freedesktop/NetworkManager/Devices/3".into(),
        on: true,
    };
    let refused = Err(clean(
        "org.freedesktop.zbus.Error: Not authorized to perform this operation",
    ));
    assert!(matches!(
        ran(&stats, refused),
        Wake::Ran { error: None, .. }
    ));
}

#[test]
fn a_dismissed_prompt_has_no_message_box_and_a_refusal_has_one() {
    let denied = || {
        reprisebus::error::reply(
            "org.freedesktop.NetworkManager.PermissionDenied",
            "Not authorized to control networking.",
        )
    };
    let dismissed = auth_failure(denied(), || true);
    assert_eq!(dismissed, Failure::Dismissed);
    assert!(matches!(
        ran(&join(), Err(dismissed)),
        Wake::Ran { error: None, .. }
    ));
    let refused = auth_failure(denied(), || false);
    assert_eq!(
        refused,
        Failure::Text(reprisebus::polkit::NOT_ALLOWED.into())
    );
    assert!(matches!(
        ran(&join(), Err(refused)),
        Wake::Ran { error: Some(_), .. }
    ));
}

#[test]
fn a_prompt_left_open_past_the_call_timeout_says_to_try_again() {
    let io = zbus::Error::from(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "timed out",
    ));
    let failure = auth_failure(io, || panic!("a timeout is no dismissal"));
    assert_eq!(failure, Failure::Text(reprisebus::polkit::TIMED_OUT.into()));
}

#[test]
fn a_status_sheet_gets_its_counters_while_a_join_waits_on_a_password() {
    use futures_util::FutureExt;
    use futures_util::future::{Either, select};
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    let (wake_tx, wake_rx) = async_channel::unbounded();
    cmd_tx.send_blocking(join()).unwrap();
    cmd_tx
        .send_blocking(Cmd::Stats {
            device: DEV.into(),
            on: true,
        })
        .unwrap();
    let q = queue(cmd_rx, wake_tx, |cmd| async move {
        if matches!(cmd, Cmd::Connect { .. }) {
            std::future::pending::<()>().await;
        }
        Ok(None)
    });
    match select(Box::pin(q), Box::pin(wake_rx.recv())).now_or_never() {
        Some(Either::Right((Ok(wake), _))) => assert_eq!(
            wake,
            Wake::Ran {
                device: Some(DEV.into()),
                error: None,
                reply: None,
            },
            "the Stats command answers while the join is pending"
        ),
        _ => panic!("the Stats command waited behind the pending join"),
    }
}

#[test]
fn quit_answers_after_a_queued_stats_command_has_run() {
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    let (wake_tx, wake_rx) = async_channel::unbounded();
    cmd_tx
        .send_blocking(Cmd::Stats {
            device: DEV.into(),
            on: false,
        })
        .unwrap();
    cmd_tx.send_blocking(Cmd::Quit).unwrap();
    drop(cmd_tx);
    zbus::block_on(queue(cmd_rx, wake_tx, |_| async { Ok(None) }));
    let wakes: Vec<Wake> = std::iter::from_fn(|| wake_rx.try_recv().ok()).collect();
    assert_eq!(
        wakes,
        vec![
            Wake::Ran {
                device: Some(DEV.into()),
                error: None,
                reply: None,
            },
            Wake::Quit
        ],
        "Stats off runs, then Quit answers"
    );
}

#[test]
fn only_a_bare_statistics_tick_skips_the_rebuild() {
    let dev = "/org/freedesktop/NetworkManager/Devices/2";
    let tick = props(vec![("RxBytes", Variant::from(10u64))]);
    assert_eq!(
        wake_for_change(dev, STATS_IFACE, &tick),
        Some(Wake::Stats {
            path: dev.into(),
            rx: Some(10),
            tx: None
        })
    );
    let rate = props(vec![
        ("RxBytes", Variant::from(10u64)),
        ("RefreshRateMs", Variant::from(2000u32)),
    ]);
    assert_eq!(wake_for_change(dev, STATS_IFACE, &rate), Some(Wake::Bus));
    assert_eq!(wake_for_change(dev, DEVICE_IFACE, &tick), Some(Wake::Bus));
    assert_eq!(
        wake_for_change(dev, STATS_IFACE, &props(vec![])),
        Some(Wake::Bus)
    );
}

#[test]
fn only_a_bare_signal_update_skips_the_rebuild() {
    let ap = "/org/freedesktop/NetworkManager/AccessPoint/7";
    let scan = props(vec![
        ("Strength", Variant::from(55u8)),
        ("LastSeen", Variant::from(1234i32)),
    ]);
    assert_eq!(
        wake_for_change(ap, AP_IFACE, &scan),
        Some(Wake::Strength {
            path: ap.into(),
            strength: Some(55)
        })
    );
    let seen = props(vec![("LastSeen", Variant::from(1234i32))]);
    assert_eq!(
        wake_for_change(ap, AP_IFACE, &seen),
        Some(Wake::Strength {
            path: ap.into(),
            strength: None
        })
    );
    let flags = props(vec![
        ("Strength", Variant::from(55u8)),
        ("RsnFlags", Variant::from(0x100u32)),
    ]);
    assert_eq!(wake_for_change(ap, AP_IFACE, &flags), Some(Wake::Bus));
}

#[test]
fn a_bare_scan_stamp_wakes_nothing() {
    let dev = "/org/freedesktop/NetworkManager/Devices/3";
    let stamp = props(vec![("LastScan", Variant::from(5000i64))]);
    assert_eq!(wake_for_change(dev, WIRELESS_IFACE, &stamp), None);
    let rate = props(vec![
        ("LastScan", Variant::from(5000i64)),
        ("Bitrate", Variant::from(54000u32)),
    ]);
    assert_eq!(wake_for_change(dev, WIRELESS_IFACE, &rate), Some(Wake::Bus));
}

#[test]
fn a_strength_update_patches_the_ap_and_keeps_the_list_in_order() {
    let ap = |path: &str, ssid: &str, strength: u8, active: bool| AccessPoint {
        path: path.into(),
        ssid: ssid.into(),
        strength,
        active,
        ..AccessPoint::default()
    };
    let mut snap = Snapshot {
        devices: vec![Device {
            path: "/d/1".into(),
            wifi: Some(Wifi {
                active_ssid: Some("Home".into()),
                strength: 70,
                aps: vec![
                    ap("/ap/1", "Home", 70, true),
                    ap("/ap/2", "Cafe", 40, false),
                ],
            }),
            ..Device::default()
        }],
        ..Snapshot::default()
    };
    assert!(patch_strength(&mut snap, "/ap/1", Some(20)));
    let wifi = snap.devices[0].wifi.as_ref().unwrap();
    assert_eq!(wifi.strength, 20, "the live network's strength follows");
    assert_eq!(
        wifi.aps
            .iter()
            .map(|a| (a.path.as_str(), a.strength))
            .collect::<Vec<_>>(),
        [("/ap/2", 40), ("/ap/1", 20)],
        "strongest first"
    );
    assert!(patch_strength(&mut snap, "/ap/2", None));
    assert!(
        !patch_strength(&mut snap, "/ap/9", Some(1)),
        "an unknown access point wants a read"
    );
}

#[test]
fn a_statistics_tick_patches_its_device_and_keeps_the_other_counter() {
    let mut snap = Snapshot {
        devices: vec![Device {
            path: "/d/1".into(),
            rx_bytes: 5,
            tx_bytes: 7,
            ..Device::default()
        }],
        ..Snapshot::default()
    };
    assert!(patch_stats(&mut snap, "/d/1", None, Some(9)));
    assert_eq!((snap.devices[0].rx_bytes, snap.devices[0].tx_bytes), (5, 9));
    assert!(
        !patch_stats(&mut snap, "/d/2", Some(1), None),
        "an unknown device wants a read"
    );
}

#[test]
fn a_new_profile_is_the_users_own_unless_they_may_modify_system_ones() {
    let login = || Some("alice".to_string());
    let with = |r: &str| HashMap::from([(MODIFY_SYSTEM.to_string(), r.to_string())]);
    assert_eq!(profile_owner(&with("yes"), login()), None);
    assert_eq!(
        profile_owner(&with("auth"), login()).as_deref(),
        Some("alice")
    );
    assert_eq!(
        profile_owner(&with("no"), login()).as_deref(),
        Some("alice")
    );
    assert_eq!(
        profile_owner(&HashMap::new(), login()).as_deref(),
        Some("alice"),
        "no answer from NetworkManager keeps the profile private"
    );
    assert_eq!(profile_owner(&with("auth"), None), None);
}

#[test]
fn an_owned_wifi_profile_carries_a_user_permission() {
    let key = psk("hunter22");
    let owned = wifi_settings(&"Home".into(), Some(&key), "wpa-psk", false, Some("alice"));
    assert_eq!(
        owned["connection"].get("permissions"),
        Some(&Variant::from(vec!["user:alice".to_string()])),
        "permissions is [\"user:alice\"]"
    );
    let shared = wifi_settings(&"Home".into(), None, "wpa-psk", false, None);
    assert!(
        !shared["connection"].contains_key("permissions"),
        "a system-wide profile has no permissions key"
    );
}

#[test]
fn a_wpa3_only_network_gets_an_sae_profile() {
    assert_eq!(key_mgmt_for(0, 0x400), "sae", "SAE alone is sae");
    assert_eq!(key_mgmt_for(0, 0x100 | 0x400), "wpa-psk", "a mixed network");
    assert_eq!(key_mgmt_for(0x100, 0x400), "wpa-psk", "WPA1 PSK beside SAE");
    assert_eq!(key_mgmt_for(0, 0x100), "wpa-psk");
    let key = psk("hunter22");
    let sae = wifi_settings(&"Home".into(), Some(&key), "sae", false, None);
    assert_eq!(
        sae["802-11-wireless-security"].get("key-mgmt"),
        Some(&Variant::from("sae")),
        "key-mgmt is sae"
    );
}

fn psk(s: &str) -> Key {
    Key::Psk(Psk::new(s))
}

fn campus(e: Enterprise) -> HashMap<String, HashMap<String, Variant<'static>>> {
    let key = Key::Enterprise(Box::new(e));
    wifi_settings(&"campus".into(), Some(&key), "wpa-psk", false, None)
        .into_iter()
        .map(|(g, entries)| {
            let entries = entries
                .into_iter()
                .map(|(k, v)| (k, v.try_to_owned().unwrap().into()))
                .collect();
            (g, entries)
        })
        .collect()
}

fn file(path: &str) -> Variant<'static> {
    Variant::from([b"file://", path.as_bytes(), b"\0"].concat())
}

#[test]
fn a_peap_profile_logs_in_with_mschapv2_inside_the_tunnel() {
    let s = campus(Enterprise {
        eap: Eap::Peap,
        anonymous_identity: "anonymous@uni.example".into(),
        ca: CaSource::File("/etc/ssl/certs/uni.pem".into()),
        inner: Inner::Mschapv2,
        identity: "alice@uni.example".into(),
        password: Psk::new("hunter22"),
        ..Enterprise::default()
    });
    assert_eq!(
        s["802-11-wireless-security"],
        HashMap::from([("key-mgmt".to_string(), Variant::from("wpa-eap"))]),
        "key-mgmt is wpa-eap and nothing else"
    );
    assert_eq!(
        s["802-1x"],
        HashMap::from([
            ("eap".to_string(), Variant::from(vec!["peap".to_string()])),
            ("identity".to_string(), Variant::from("alice@uni.example")),
            (
                "anonymous-identity".to_string(),
                Variant::from("anonymous@uni.example")
            ),
            ("ca-cert".to_string(), file("/etc/ssl/certs/uni.pem")),
            ("phase2-auth".to_string(), Variant::from("mschapv2")),
            ("password".to_string(), Variant::from("hunter22")),
            ("password-flags".to_string(), Variant::from(0u32)),
        ])
    );
}

#[test]
fn a_ttls_profile_with_no_ca_logs_in_with_pap() {
    let s = campus(Enterprise {
        eap: Eap::Ttls,
        inner: Inner::Pap,
        identity: "bob".into(),
        password: Psk::new("hunter22"),
        ..Enterprise::default()
    });
    assert_eq!(
        s["802-1x"],
        HashMap::from([
            ("eap".to_string(), Variant::from(vec!["ttls".to_string()])),
            ("identity".to_string(), Variant::from("bob")),
            ("phase2-auth".to_string(), Variant::from("pap")),
            ("password".to_string(), Variant::from("hunter22")),
            ("password-flags".to_string(), Variant::from(0u32)),
        ]),
        "no ca-cert and no empty anonymous-identity"
    );
    let gtc = campus(Enterprise {
        eap: Eap::Ttls,
        inner: Inner::Gtc,
        ..Enterprise::default()
    });
    assert_eq!(gtc["802-1x"]["phase2-autheap"], Variant::from("gtc"));
    assert!(
        !gtc["802-1x"].contains_key("phase2-auth"),
        "TTLS takes GTC as an inner EAP method only"
    );
}

#[test]
fn a_profile_names_the_server_domain_and_can_trust_the_system_cas() {
    let s = campus(Enterprise {
        eap: Eap::Peap,
        ca: CaSource::System,
        domain: "radius.uni.example".into(),
        identity: "alice".into(),
        password: Psk::new("hunter22"),
        ..Enterprise::default()
    });
    let x = &s["802-1x"];
    assert_eq!(x["system-ca-certs"], Variant::from(true));
    assert_eq!(
        x["domain-suffix-match"],
        Variant::from("radius.uni.example")
    );
    assert!(!x.contains_key("ca-cert"), "the system store needs no file");
    let file_ca = campus(Enterprise {
        ca: CaSource::File("/etc/ssl/certs/uni.pem".into()),
        domain: "uni.example".into(),
        ..Enterprise::default()
    });
    assert_eq!(
        file_ca["802-1x"]["domain-suffix-match"],
        Variant::from("uni.example")
    );
    assert!(!file_ca["802-1x"].contains_key("system-ca-certs"));
}

#[test]
fn a_tls_profile_carries_its_certificate_and_key() {
    let tls = Enterprise {
        eap: Eap::Tls,
        ca: CaSource::File("/home/alice/ca.pem".into()),
        identity: "alice".into(),
        password: Psk::new("unused"),
        client_cert: Some("/home/alice/alice.crt".into()),
        private_key: Some("/home/alice/alice.key".into()),
        private_key_password: Psk::new("swordfish"),
        ..Enterprise::default()
    };
    let s = campus(tls.clone());
    assert_eq!(
        s["802-1x"],
        HashMap::from([
            ("eap".to_string(), Variant::from(vec!["tls".to_string()])),
            ("identity".to_string(), Variant::from("alice")),
            ("ca-cert".to_string(), file("/home/alice/ca.pem")),
            ("client-cert".to_string(), file("/home/alice/alice.crt")),
            ("private-key".to_string(), file("/home/alice/alice.key")),
            (
                "private-key-password".to_string(),
                Variant::from("swordfish")
            ),
            (
                "private-key-password-flags".to_string(),
                Variant::from(0u32)
            ),
        ]),
        "no password or phase 2 for TLS"
    );
    let bare = campus(Enterprise {
        private_key_password: Psk::default(),
        ..tls
    });
    assert_eq!(
        bare["802-1x"]["private-key-password-flags"],
        Variant::from(4u32),
        "an unencrypted key needs no password"
    );
    assert!(!bare["802-1x"].contains_key("private-key-password"));
}

#[test]
fn an_enterprise_login_rewrites_a_saved_profile_as_wpa_eap() {
    let current = HashMap::from([
        (
            "connection".to_string(),
            props(vec![("uuid", Variant::from("u-1"))]),
        ),
        (
            "802-11-wireless-security".to_string(),
            props(vec![
                ("key-mgmt", Variant::from("wpa-psk")),
                ("psk-flags", Variant::from(1u32)),
            ]),
        ),
    ]);
    let key = Key::Enterprise(Box::new(Enterprise {
        identity: "alice".into(),
        password: Psk::new("hunter22"),
        ..Enterprise::default()
    }));
    let s = rekeyed(current.clone(), &key, "wpa-psk");
    assert_eq!(
        s["802-11-wireless-security"],
        HashMap::from([("key-mgmt".to_string(), Variant::from("wpa-eap"))])
    );
    assert_eq!(s["802-1x"]["identity"], Variant::from("alice"));
    assert_eq!(s["connection"]["uuid"], Variant::from("u-1"));
    let key = psk("hunter22");
    let s = rekeyed(current, &key, "sae");
    assert_eq!(
        s["802-11-wireless-security"],
        HashMap::from([
            ("key-mgmt".to_string(), Variant::from("wpa-psk")),
            ("psk".to_string(), Variant::from("hunter22")),
            ("psk-flags".to_string(), Variant::from(0u32)),
        ]),
        "a passphrase keeps the profile's own key-mgmt"
    );
}

#[test]
fn a_hidden_network_is_probed_for_by_name() {
    let key = psk("hunter22");
    let s = wifi_settings(&"Attic".into(), Some(&key), "wpa-psk", true, None);
    assert_eq!(s["802-11-wireless"]["hidden"], Variant::from(true));
    assert_eq!(
        s["802-11-wireless"]["ssid"],
        Variant::from(b"Attic".to_vec())
    );
    let sae = Key::Sae(Psk::new("hunter22"));
    let s = wifi_settings(&"Attic".into(), Some(&sae), "wpa-psk", true, None);
    assert_eq!(
        s["802-11-wireless-security"]["key-mgmt"],
        Variant::from("sae"),
        "a WPA3 key joins as sae though the hidden network has no access point to ask"
    );
    let open = wifi_settings(&"Attic".into(), None, "wpa-psk", true, None);
    assert_eq!(open["802-11-wireless"]["hidden"], Variant::from(true));
    assert!(
        !open.contains_key("802-11-wireless-security"),
        "no key, no security"
    );
    let seen = wifi_settings(&"Attic".into(), None, "wpa-psk", false, None);
    assert!(
        !seen["802-11-wireless"].contains_key("hidden"),
        "a network in the scan list is not marked hidden"
    );
}

#[test]
fn a_new_profile_carries_the_ssids_own_bytes() {
    let latin1 = Ssid(b"Caf\xe9".to_vec());
    let s = wifi_settings(&latin1, None, "wpa-psk", false, None);
    assert_eq!(
        s["802-11-wireless"].get("ssid"),
        Some(&Variant::from(b"Caf\xe9".to_vec())),
        "ssid is the bytes 43 61 66 e9"
    );
}

#[test]
fn a_hotspot_profile_is_an_access_point_sharing_its_address() {
    let s = hotspot_settings("Desk", "hunter222", false, Band::A, Some("alice"));
    assert_eq!(s["802-11-wireless"]["mode"], Variant::from("ap"));
    assert_eq!(s["802-11-wireless"]["band"], Variant::from("a"));
    assert_eq!(
        s["802-11-wireless"]["ssid"],
        Variant::from(b"Desk".to_vec())
    );
    assert_eq!(
        s["802-11-wireless-security"]["key-mgmt"],
        Variant::from("wpa-psk")
    );
    assert_eq!(s["ipv4"]["method"], Variant::from("shared"));
    assert_eq!(
        s["connection"]["permissions"],
        Variant::from(vec!["user:alice".to_string()])
    );
    let wpa3 = hotspot_settings("Desk", "hunter222", true, Band::Auto, None);
    assert_eq!(
        wpa3["802-11-wireless-security"]["key-mgmt"],
        Variant::from("sae")
    );
    assert!(
        !wpa3["802-11-wireless"].contains_key("band"),
        "Automatic leaves the band to the driver"
    );
}

#[test]
fn a_rewritten_hotspot_keeps_its_uuid_and_drops_the_old_band() {
    let current = HashMap::from([
        (
            "connection".to_string(),
            props(vec![
                ("id", Variant::from("Hotspot")),
                ("uuid", Variant::from("u-1")),
            ]),
        ),
        (
            "802-11-wireless".to_string(),
            props(vec![("band", Variant::from("a"))]),
        ),
    ]);
    let m = merge_groups(
        current,
        hotspot_settings("Desk", "hunter222", false, Band::Auto, None),
    );
    assert_eq!(m["connection"]["uuid"], Variant::from("u-1"));
    assert!(!m["802-11-wireless"].contains_key("band"));
}

#[test]
fn an_import_error_is_nmclis_own_sentence() {
    assert_eq!(
        import_error("Error: failed to find VPN plugin for openvpn.\n"),
        "failed to find VPN plugin for openvpn."
    );
    assert_eq!(import_error(""), "The file could not be imported.");
}

#[test]
fn a_rekey_keeps_the_profiles_key_mgmt() {
    assert_eq!(rekey_mgmt("sae", "wpa-psk"), "sae");
    assert_eq!(rekey_mgmt("wpa-psk", "sae"), "wpa-psk");
    assert_eq!(rekey_mgmt("", "sae"), "sae", "no security group yet");
    assert_eq!(rekey_mgmt("none", "wpa-psk"), "wpa-psk", "a WEP profile");
}

#[test]
fn quit_returns_at_once_with_no_thread() {
    let (cmds, rx) = async_channel::unbounded::<Cmd>();
    drop(rx);
    cmds.close();
    let (_done_tx, done) = channel();
    let handle = NmHandle { cmds, done };
    let start = std::time::Instant::now();
    handle.quit();
    assert!(start.elapsed() < QUIT_WAIT, "quit waited for the timeout");
}

type Calls = std::sync::Arc<std::sync::Mutex<Vec<(&'static str, bool)>>>;

fn interactive(hdr: &zbus::message::Header<'_>) -> bool {
    hdr.primary()
        .flags()
        .contains(zbus::message::Flags::ALLOW_INTERACTIVE_AUTH)
}

/// NetworkManager with one saved Wi-Fi profile, "Home", recording which calls may prompt.
struct FakeNm(Calls);

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl FakeNm {
    fn activate_connection(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _connection: zbus::OwnedObjectPath,
        _device: zbus::OwnedObjectPath,
        _ap: zbus::OwnedObjectPath,
    ) -> zbus::OwnedObjectPath {
        self.0
            .lock()
            .unwrap()
            .push(("ActivateConnection", interactive(&hdr)));
        zbus::OwnedObjectPath::try_from("/a/1").unwrap()
    }

    fn add_and_activate_connection(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _settings: HashMap<String, Props>,
        _device: zbus::OwnedObjectPath,
        _ap: zbus::OwnedObjectPath,
    ) -> (zbus::OwnedObjectPath, zbus::OwnedObjectPath) {
        self.0
            .lock()
            .unwrap()
            .push(("AddAndActivateConnection", interactive(&hdr)));
        let path = |p: &str| zbus::OwnedObjectPath::try_from(p).unwrap();
        (path("/c/2"), path("/a/2"))
    }
}

struct FakeSettings;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl FakeSettings {
    fn list_connections(&self) -> Vec<zbus::OwnedObjectPath> {
        vec![zbus::OwnedObjectPath::try_from(HOME).unwrap()]
    }
}

struct FakeProfile(Calls);

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl FakeProfile {
    fn get_settings(&self) -> HashMap<String, Props> {
        HashMap::from([(
            "802-11-wireless".to_string(),
            props(vec![("ssid", Variant::from(b"Home".to_vec()))]),
        )])
    }

    fn update(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        _settings: HashMap<String, Props>,
    ) {
        self.0.lock().unwrap().push(("Update", interactive(&hdr)));
    }
}

const HOME: &str = "/org/freedesktop/NetworkManager/Settings/1";

/// polkit refuses a call that needs a password unless it may ask for one.
#[test]
fn every_call_of_a_wifi_join_may_raise_the_password_prompt() {
    let calls = Calls::default();
    let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
    let recorded = calls.clone();
    zbus::block_on(async move {
        let (server, client) = futures_util::future::join(
            zbus::connection::Builder::unix_stream(a)
                .server(zbus::Guid::generate())
                .p2p()
                .serve_at(MANAGER_PATH, FakeNm(recorded.clone()))
                .serve_at(SETTINGS_PATH, FakeSettings)
                .serve_at(HOME, FakeProfile(recorded))
                .build(),
            zbus::connection::Builder::unix_stream(b).p2p().build(),
        )
        .await;
        let (_server, client) = (server.unwrap(), client.unwrap());
        let connect = |ssid: &str| Cmd::Connect {
            device: DEV.into(),
            ap: "/".into(),
            ssid: ssid.into(),
            key: Some(Key::Psk(Psk::new("new key"))),
            hidden: false,
        };
        act(&client, &connect("Home")).await.unwrap();
        act(&client, &connect("Cafe")).await.unwrap();
    });
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("Update", true),
            ("ActivateConnection", true),
            ("AddAndActivateConnection", true),
        ],
        "a rekeyed profile, then a new one"
    );
}
