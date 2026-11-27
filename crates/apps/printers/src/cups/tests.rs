use super::*;

fn group(attrs: Vec<(&str, Vec<Value>)>) -> ipp::Group {
    ipp::Group {
        tag: PRINTER,
        attrs: attrs
            .into_iter()
            .map(|(n, v)| ipp::Attr {
                name: n.into(),
                values: v,
            })
            .collect(),
    }
}

#[test]
fn a_printer_reads_its_state_and_reasons() {
    let g = group(vec![
        ("printer-name", vec![Value::name("Laser")]),
        ("printer-state", vec![Value::Enum(5)]),
        ("printer-state-reasons", vec![Value::keyword("paused")]),
        ("printer-is-shared", vec![Value::Bool(true)]),
        ("queued-job-count", vec![Value::Int(2)]),
    ]);
    let p = Printer::from_group(&g);
    assert_eq!(p.name, "Laser");
    assert!(p.paused());
    assert_eq!(p.status_word(), "Paused");
    assert!(p.shared);
    assert_eq!(p.queued, 2);

    let g = group(vec![
        ("printer-state", vec![Value::Enum(5)]),
        (
            "printer-state-reasons",
            vec![
                Value::keyword("paused"),
                Value::keyword("media-empty-error"),
            ],
        ),
    ]);
    let p = Printer::from_group(&g);
    assert!(!p.paused(), "an error is not a pause");
    assert_eq!(p.status_word(), "Error: Media empty");

    let idle = Printer::from_group(&group(vec![(
        "printer-state-reasons",
        vec![Value::keyword("none")],
    )]));
    assert!(idle.reasons.is_empty());
    assert_eq!(idle.status_word(), "Ready");
}

#[test]
fn a_job_names_its_printer_from_its_uri() {
    let g = group(vec![
        ("job-id", vec![Value::Int(7)]),
        (
            "job-printer-uri",
            vec![Value::uri("ipp://localhost/printers/Front%20Desk")],
        ),
        ("job-state", vec![Value::Enum(4)]),
        ("job-name", vec![Value::name("report.pdf")]),
    ]);
    let j = Job::from_group(&g);
    assert_eq!(j.id, 7);
    assert_eq!(j.printer, "Front Desk");
    assert_eq!(j.state, JobState::Held);
    assert_eq!(j.status_word(), "Paused");
    assert_eq!(
        printer_uri("Front Desk"),
        "ipp://localhost/printers/Front%20Desk"
    );
}

#[test]
fn the_users_default_reads_and_writes_in_lpoptions() {
    assert_eq!(user_default(""), None);
    let text = "Dest Other sides=one-sided\nDefault Laser/draft media=a4\n";
    assert_eq!(user_default(text), Some("Laser".into()));
    let new = with_user_default(text, "Inkjet");
    assert_eq!(
        new,
        "Default Inkjet\nDest Other sides=one-sided\nDest Laser/draft media=a4\n"
    );
    assert_eq!(user_default(&new), Some("Inkjet".into()));
    assert_eq!(own_default_in(&new, text), Some("Inkjet".into()));
    assert_eq!(
        own_default_in("Dest Other\n", text),
        Some("Laser".into()),
        "the system's lpoptions names it when the user's does not"
    );
    assert_eq!(own_default_in("", ""), None);
}

#[test]
fn a_rename_carries_the_default_the_system_lpoptions_names() {
    let system = "Default Laser\nDest Laser media=a4\n";
    assert_eq!(
        renamed_default("", system, "Laser", "Office").as_deref(),
        Some("Default Office\n"),
        "the user's own default takes the new name"
    );
    assert_eq!(
        renamed_default("Dest Other sides=one-sided\n", system, "Laser", "Office").as_deref(),
        Some("Default Office\nDest Other sides=one-sided\n")
    );
    assert_eq!(
        renamed_default("Default Laser\n", "", "Laser", "Office").as_deref(),
        Some("Default Office\n")
    );
    assert_eq!(
        renamed_default("Default Inkjet\n", system, "Laser", "Office"),
        None,
        "the user's default is another printer"
    );
    assert_eq!(renamed_default("", system, "Inkjet", "Office"), None);
}

#[test]
fn devices_are_driverless_over_ipp() {
    assert!(driverless_uri("ipp://10.0.0.5/ipp/print"));
    assert!(driverless_uri(
        "dnssd://Office%20Jet._ipp._tcp.local/?uuid=1"
    ));
    assert!(!driverless_uri(
        "dnssd://Office%20Jet._pdl-datastream._tcp.local/"
    ));
    assert!(!driverless_uri("usb://HP/LaserJet?serial=1"));
    let d = Device {
        uri: "usb://HP/LaserJet".into(),
        info: "HP LaserJet".into(),
        class: "direct".into(),
        ..Default::default()
    };
    assert_eq!(d.label(), "HP LaserJet (connected to this computer)");
    assert_eq!(d.maker(), "", "no device ID and no make and model");
    let found = Device {
        uri: "ipps://Lab._ipps._tcp.local/".into(),
        info: "Lab (driverless)".into(),
        make_model: "Example Printer".into(),
        device_id: "MFG:Example;MDL:Printer;CMD:PDF;".into(),
        ..Default::default()
    };
    assert_eq!(found.label(), "Lab (network, no driver needed)");
    assert_eq!(found.maker(), "Example");
    let pdf = Device {
        make_model: "Virtual PDF Printer".into(),
        ..Default::default()
    };
    assert_eq!(pdf.maker(), "Virtual");
}

#[test]
fn choices_read_as_words() {
    assert_eq!(media_label("iso_a4_210x297mm"), "A4 (210 x 297 mm)");
    assert_eq!(media_label("na_letter_8.5x11in"), "Letter (8.5 x 11 in)");
    assert_eq!(media_label("na_index-4x6_4x6in"), "Index 4x6 (4 x 6 in)");
    assert_eq!(media_label("custom"), "custom");
    assert_eq!(media_label("iso_dl_110x220mm"), "DL (110 x 220 mm)");
    assert_eq!(sides_label("two-sided-long-edge"), "Long edge (standard)");
    assert_eq!(color_label("monochrome"), "Black and white");
    assert_eq!(quality_label("5"), "High");
}

#[test]
fn options_read_supported_and_default() {
    let g = group(vec![
        (
            "media-supported",
            vec![
                Value::keyword("iso_a4_210x297mm"),
                Value::keyword("na_letter_8.5x11in"),
            ],
        ),
        ("media-default", vec![Value::keyword("iso_a4_210x297mm")]),
        (
            "print-quality-supported",
            vec![Value::Enum(3), Value::Enum(4)],
        ),
        ("print-quality-default", vec![Value::Enum(4)]),
    ]);
    let o = Options::from_group(&g);
    assert_eq!(o.media.choices.len(), 2);
    assert_eq!(o.media.default.as_deref(), Some("iso_a4_210x297mm"));
    assert_eq!(o.quality.choices, ["3", "4"]);
    assert_eq!(o.quality.default.as_deref(), Some("4"));
    assert!(o.sides.choices.is_empty());
}

#[test]
fn messages_read_as_sentences() {
    assert_eq!(sentence("printer not found"), "Printer not found.");
    assert_eq!(sentence("Done!"), "Done!");
}

#[test]
fn a_printer_address_loses_its_user_and_password_in_logs_and_messages() {
    assert_eq!(
        without_userinfo("smb://alice:hunter2@srv/queue"),
        "smb://srv/queue"
    );
    assert_eq!(
        without_userinfo("Bad device-uri \"ipp://bob@[::1]:631/ipp\" and ipp://h/p@x"),
        "Bad device-uri \"ipp://[::1]:631/ipp\" and ipp://h/p@x"
    );
    assert_eq!(without_userinfo("usb://HP/LaserJet"), "usb://HP/LaserJet");
    assert!(
        !sentence("bad device-uri smb://alice:hunter2@srv/q").contains("hunter2"),
        "cupsd's and cups-pk-helper's messages"
    );
    assert!(
        !Error::from(http::Error::Failed("smb://alice:hunter2@srv/q".into()))
            .text()
            .contains("hunter2")
    );
    let change = Change {
        device_uri: Some("smb://alice:hunter2@srv/q".into()),
        ..Change::default()
    };
    let new = NewPrinter {
        name: "Laser".into(),
        device_uri: "ipp://alice:hunter2@srv/ipp".into(),
        driver: Driver::Everywhere,
        info: String::new(),
        location: String::new(),
    };
    for logged in [format!("{change:?}"), format!("{new:?}")] {
        assert!(
            !logged.contains("hunter2") && !logged.contains("alice"),
            "{logged}"
        );
        assert!(logged.contains("://srv/"), "{logged}");
    }
}

fn client(helper: &Arc<pkhelper::fake::Fake>) -> Client {
    Client {
        server: Server::Tcp("localhost:631".into()),
        user: "ann".into(),
        helper: helper.clone(),
    }
}

#[test]
fn admin_operations_go_to_the_helper_only_when_cupsd_says_no() {
    let delete = || vec![Call::PrinterDelete("Laser".into())];

    let h = Arc::new(pkhelper::fake::Fake::installed());
    assert_eq!(client(&h).admin(Ok(()), delete), Ok(()));
    assert!(h.calls().is_empty());

    assert_eq!(client(&h).admin(Err(Error::Denied), delete), Ok(()));
    assert_eq!(h.calls(), delete());

    let h = Arc::new(pkhelper::fake::Fake::installed());
    let refused = Err(Error::Refused("Printer not found.".into()));
    assert_eq!(client(&h).admin(refused.clone(), delete), refused);
    assert!(h.calls().is_empty());

    let h = Arc::new(pkhelper::fake::Fake::default());
    assert_eq!(
        client(&h).admin(Err(Error::Denied), delete),
        Err(Error::NeedsHelper)
    );
    assert!(h.calls().is_empty());
    assert!(Error::NeedsHelper.text().contains("Install cups-pk-helper"));

    let h = Arc::new(pkhelper::fake::Fake {
        answer: Some(Error::NotAuthorized),
        ..pkhelper::fake::Fake::installed()
    });
    assert_eq!(
        client(&h).admin(Err(Error::Denied), delete),
        Err(Error::NotAuthorized)
    );
    assert!(
        Error::NotAuthorized
            .text()
            .contains("administrator's password")
    );
}

#[test]
fn a_device_scan_without_the_prompt_never_asks_the_helper() {
    let h = Arc::new(pkhelper::fake::Fake {
        found: std::collections::BTreeMap::from([
            ("device-uri:0".into(), "usb://HP/LaserJet?serial=1".into()),
            ("device-info:0".into(), "HP LaserJet".into()),
        ]),
        ..pkhelper::fake::Fake::installed()
    });
    assert_eq!(
        devices_after(&client(&h), Err(Error::Denied), 6, false),
        Err(Error::Denied)
    );
    assert_eq!(*h.scans.lock().unwrap(), 0);

    let found = devices_after(&client(&h), Err(Error::Denied), 6, true).expect("devices");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].uri, "usb://HP/LaserJet?serial=1");
    assert_eq!(*h.scans.lock().unwrap(), 1);

    let none = Arc::new(pkhelper::fake::Fake::default());
    assert_eq!(
        devices_after(&client(&none), Err(Error::Denied), 6, true),
        Err(Error::NeedsHelper)
    );
}

#[test]
fn a_network_printer_cupsd_lists_too_shows_once_as_cupsds() {
    let cups = [
        Device {
            uri: "dnssd://Office%20Jet._ipp._tcp.local/?uuid=1".into(),
            info: "Office Jet".into(),
            make_model: "HP OfficeJet 5200".into(),
            class: "network".into(),
            device_id: "MFG:HP;MDL:OfficeJet 5200;".into(),
        },
        Device {
            uri: "ipps://Lab._ipps._tcp.local/".into(),
            info: "Example Laser (driverless)".into(),
            class: "network".into(),
            ..Default::default()
        },
        Device {
            uri: "usb://Brother/HL-1110?serial=9".into(),
            info: "Brother HL-1110".into(),
            class: "direct".into(),
            ..Default::default()
        },
    ];
    let network = |uri: &str, info: &str| Device {
        uri: uri.into(),
        info: info.into(),
        class: "network".into(),
        ..Default::default()
    };
    let avahi = [
        network("dnssd://Office%20Jet._ipp._tcp.local/?uuid=1", "Office Jet"),
        network("dnssd://Lab._ipp._tcp.local/", "Lab"),
        network("dnssd://Attic._ipp._tcp.local/", "Attic"),
    ];
    let merged = merge(&cups, &avahi);
    let uris: Vec<&str> = merged.iter().map(|d| d.uri.as_str()).collect();
    assert_eq!(
        uris,
        [
            "dnssd://Attic._ipp._tcp.local/",
            "ipps://Lab._ipps._tcp.local/",
            "dnssd://Office%20Jet._ipp._tcp.local/?uuid=1",
            "usb://Brother/HL-1110?serial=9",
        ]
    );
    assert_eq!(merged[2].make_model, "HP OfficeJet 5200");
    assert_eq!(merge(&[], &avahi).len(), 3);
}

#[test]
fn a_change_becomes_one_helper_call_per_field() {
    let change = Change {
        info: Some("Front desk".into()),
        location: Some("Hall".into()),
        device_uri: Some("socket://10.0.0.9".into()),
        shared: Some(true),
        defaults: vec![
            ("media-default".into(), Value::keyword("iso_a4_210x297mm")),
            ("print-quality-default".into(), Value::Enum(5)),
        ],
    };
    let n = || "Laser".to_string();
    let fields = [
        Call::PrinterSetInfo(n(), "Front desk".into()),
        Call::PrinterSetLocation(n(), "Hall".into()),
        Call::PrinterSetDevice(n(), "socket://10.0.0.9".into()),
        Call::PrinterSetShared(n(), true),
    ];
    let calls = change.calls("Laser", false, &[]);
    assert_eq!(calls[..4], fields);
    assert_eq!(
        calls[4..],
        [
            Call::PrinterAddOptionDefault(n(), "media".into(), vec!["iso_a4_210x297mm".into()]),
            Call::PrinterAddOptionDefault(n(), "print-quality".into(), vec!["5".into()]),
        ]
    );
    assert!(Change::default().calls("Laser", false, &[]).is_empty());
}

#[test]
fn on_a_ppd_queue_the_helper_sets_the_ppd_and_clears_media_and_sides() {
    let change = Change {
        defaults: vec![
            ("media-default".into(), Value::keyword("iso_a4_210x297mm")),
            ("sides-default".into(), Value::keyword("one-sided")),
            ("print-quality-default".into(), Value::Enum(5)),
        ],
        ..Change::default()
    };
    let n = || "Laser".to_string();
    let v = |s: &str| vec![s.to_string()];
    assert_eq!(
        change.calls("Laser", true, &[]),
        [
            Call::PrinterAddOption(n(), "media".into(), v("iso_a4_210x297mm")),
            Call::PrinterAddOptionDefault(n(), "media".into(), vec![]),
            Call::PrinterAddOption(n(), "sides".into(), v("one-sided")),
            Call::PrinterAddOptionDefault(n(), "sides".into(), vec![]),
            Call::PrinterAddOption(n(), "print-quality".into(), v("5")),
            Call::PrinterAddOptionDefault(n(), "print-quality".into(), v("5")),
        ]
    );
}

const PPD: &str = "*PPD-Adobe: \"4.3\"
*DefaultPageSize: Letter
*PageSize Letter/Letter: \"\"
*PageSize A4/A4: \"\"
*PaperDimension Letter/Letter: \"612 792\"
*PaperDimension A4/A4: \"595 842\"
*DefaultDuplex: None
*Duplex None/Off: \"\"
*Duplex DuplexNoTumble/Long Edge: \"\"
";

fn printer_attrs(req: &Request) -> Vec<(String, Value)> {
    req.groups
        .iter()
        .filter(|g| g.tag == PRINTER)
        .flat_map(|g| &g.attrs)
        .map(|a| (a.name.clone(), a.values[0].clone()))
        .collect()
}

#[test]
fn device_settings_on_a_ppd_queue_upload_the_ppd_with_its_defaults() {
    let change = Change {
        defaults: vec![
            ("media-default".into(), Value::keyword("iso_a4_210x297mm")),
            (
                "sides-default".into(),
                Value::keyword("two-sided-short-edge"),
            ),
            (
                "print-color-mode-default".into(),
                Value::keyword("monochrome"),
            ),
        ],
        ..Change::default()
    };
    let req = change
        .clone()
        .into_request("Laser", Some(PPD.as_bytes()), &[]);
    let ppd = String::from_utf8(req.data.clone()).expect("UTF-8");
    assert!(ppd.contains("*DefaultPageSize: A4\n"));
    assert!(ppd.contains("*DefaultDuplex: None\n"));
    assert_eq!(
        printer_attrs(&req),
        [
            ("media-default".into(), Value::Empty(ipp::DELETE_ATTR)),
            (
                "sides-default".into(),
                Value::keyword("two-sided-short-edge")
            ),
            (
                "print-color-mode-default".into(),
                Value::keyword("monochrome")
            ),
        ]
    );

    let req = change.clone().into_request("Laser", None, &[]);
    assert!(req.data.is_empty());
    assert_eq!(printer_attrs(&req), change.defaults);

    let same = Change {
        defaults: vec![("media-default".into(), Value::keyword("na_letter_8.5x11in"))],
        ..Change::default()
    };
    let req = same.into_request("Laser", Some(PPD.as_bytes()), &[]);
    assert!(req.data.is_empty(), "an unchanged PPD stays on the server");
    assert_eq!(
        printer_attrs(&req),
        [("media-default".into(), Value::Empty(ipp::DELETE_ATTR))]
    );
}

#[test]
fn a_default_reported_twice_reads_as_the_ppds_and_the_next_change_deletes_the_queue_option() {
    let g = group(vec![
        ("media-default", vec![Value::keyword("iso_a5_148x210mm")]),
        (
            "sides-default",
            vec![Value::keyword("two-sided-short-edge")],
        ),
        ("media-default", vec![Value::keyword("na_letter_8.5x11in")]),
        ("sides-default", vec![Value::keyword("one-sided")]),
    ]);
    let o = Options::from_group(&g);
    assert_eq!(o.media.default.as_deref(), Some("na_letter_8.5x11in"));
    assert_eq!(o.sides.default.as_deref(), Some("one-sided"));
    let stale = stale_in(&g);
    assert_eq!(stale, ["media-default", "sides-default"]);
    let once = group(vec![(
        "media-default",
        vec![Value::keyword("iso_a4_210x297mm")],
    )]);
    assert!(stale_in(&once).is_empty());

    let change = Change {
        defaults: vec![(
            "sides-default".into(),
            Value::keyword("two-sided-long-edge"),
        )],
        ..Change::default()
    };
    let req = change
        .clone()
        .into_request("Laser", Some(PPD.as_bytes()), &stale);
    assert_eq!(
        printer_attrs(&req),
        [
            ("media-default".into(), Value::Empty(ipp::DELETE_ATTR)),
            ("sides-default".into(), Value::Empty(ipp::DELETE_ATTR)),
        ]
    );
    let n = || "Laser".to_string();
    assert_eq!(
        change.calls("Laser", true, &stale),
        [
            Call::PrinterAddOptionDefault(n(), "media".into(), vec![]),
            Call::PrinterAddOption(n(), "sides".into(), vec!["two-sided-long-edge".into()]),
            Call::PrinterAddOptionDefault(n(), "sides".into(), vec![]),
        ]
    );
}

#[test]
fn a_helper_rename_is_refused_while_jobs_wait_or_print() {
    let job = |state| Job {
        printer: "Laser".into(),
        state,
        ..Job::default()
    };
    let with = |state| Snapshot {
        jobs: vec![job(state)],
        ..Snapshot::default()
    };
    for state in [JobState::Pending, JobState::Printing] {
        assert_eq!(
            helper_rename_refusal("Laser", &with(state)),
            Some(Error::Refused(
                "Laser can be renamed once no documents are waiting or printing on it.".into()
            ))
        );
        assert_eq!(helper_rename_refusal("Other", &with(state)), None);
    }
    assert_eq!(helper_rename_refusal("Laser", &with(JobState::Held)), None);
}

#[test]
fn a_rename_to_another_case_of_a_printer_name_is_refused() {
    let named = |name: &str| Printer {
        name: name.into(),
        ..Printer::default()
    };
    let s = Snapshot {
        printers: vec![named("laser"), named("Inkjet")],
        ..Snapshot::default()
    };
    assert_eq!(
        name_clash("laser", "Laser", &s).as_deref(),
        Some("Laser differs from laser only in case, which printer names ignore.")
    );
    assert_eq!(
        name_clash("Inkjet", "LASER", &s).as_deref(),
        Some("There is already a printer called laser.")
    );
    assert_eq!(name_clash("Inkjet", "Office", &s), None);
}

#[test]
fn a_rename_takes_the_address_typed_in_the_same_apply() {
    let p = Printer {
        name: "Share".into(),
        device_uri: "smb://server/q".into(),
        ..Printer::default()
    };
    let device_uri = |req: &Request| {
        printer_attrs(req)
            .into_iter()
            .find(|(k, _)| k == "device-uri")
            .map(|(_, v)| v)
    };
    let typed = DeviceUri("smb://u:p@server/q".into());
    assert_eq!(
        device_uri(&rename_request(
            "Office",
            &p,
            Some(&typed),
            None,
            &[],
            &ipp::Group::default()
        )),
        Some(Value::uri("smb://u:p@server/q"))
    );
    assert_eq!(
        device_uri(&rename_request(
            "Office",
            &p,
            None,
            None,
            &[],
            &ipp::Group::default()
        )),
        Some(Value::uri("smb://server/q"))
    );
    assert_eq!(
        helper_rename_calls("Share", "Office", Some(&typed), &[]),
        [
            Call::PrinterRename("Share".into(), "Office".into()),
            Call::PrinterSetDevice("Office".into(), "smb://u:p@server/q".into()),
        ]
    );
    assert_eq!(format!("{typed:?}"), "smb://server/q");
}

#[test]
fn a_rename_keeps_the_defaults_the_ppd_does_not_hold() {
    let o = Options {
        media: Setting {
            default: Some("iso_a4_210x297mm".into()),
            ..Setting::default()
        },
        sides: Setting {
            default: Some("one-sided".into()),
            ..Setting::default()
        },
        color: Setting {
            default: Some("monochrome".into()),
            ..Setting::default()
        },
        quality: Setting {
            default: Some("5".into()),
            ..Setting::default()
        },
    };
    let tail = [
        (
            "print-color-mode-default".into(),
            Value::keyword("monochrome"),
        ),
        ("print-quality-default".into(), Value::Enum(5)),
    ];
    assert_eq!(carried_defaults(&o, true), tail);
    let all = carried_defaults(&o, false);
    assert_eq!(
        all[..2],
        [
            ("media-default".into(), Value::keyword("iso_a4_210x297mm")),
            ("sides-default".into(), Value::keyword("one-sided")),
        ]
    );
    assert_eq!(all[2..], tail);
    assert!(carried_defaults(&Options::default(), false).is_empty());
}

#[test]
fn adding_through_the_helper_names_the_driver_then_enables() {
    let mut p = NewPrinter {
        name: "Lab".into(),
        device_uri: "ipp://10.0.0.5/ipp/print".into(),
        driver: Driver::Everywhere,
        info: "Lab printer".into(),
        location: "Room 2".into(),
    };
    let tail = [
        Call::PrinterSetEnabled("Lab".into(), true),
        Call::PrinterSetAcceptJobs("Lab".into(), true),
    ];
    let add = |ppd: &str| Call::PrinterAdd {
        name: "Lab".into(),
        uri: "ipp://10.0.0.5/ipp/print".into(),
        ppd: ppd.into(),
        info: "Lab printer".into(),
        location: "Room 2".into(),
    };
    assert_eq!(add_calls(&p)[0], add("everywhere"));
    assert_eq!(add_calls(&p)[1..], tail);
    p.driver = Driver::Ppd("drv:///sample.drv/generic.ppd".into());
    assert_eq!(add_calls(&p)[0], add("drv:///sample.drv/generic.ppd"));
    p.driver = Driver::File("/home/ann/laser.ppd".into());
    assert_eq!(
        add_calls(&p)[0],
        Call::PrinterAddWithPpdFile {
            name: "Lab".into(),
            uri: "ipp://10.0.0.5/ipp/print".into(),
            file: "/home/ann/laser.ppd".into(),
            info: "Lab printer".into(),
            location: "Room 2".into(),
        }
    );
    assert_eq!(add_calls(&p)[1..], tail);
}

#[test]
fn job_operations_map_to_the_helpers_job_calls() {
    assert_eq!(
        job_call(7, JobOp::Pause),
        Call::JobSetHoldUntil(7, "indefinite".into())
    );
    assert_eq!(
        job_call(7, JobOp::Resume),
        Call::JobSetHoldUntil(7, "no-hold".into())
    );
    assert_eq!(job_call(7, JobOp::Restart), Call::JobRestart(7));
    assert_eq!(job_call(7, JobOp::Cancel), Call::JobCancelPurge(7, false));
    assert_eq!(job_call(7, JobOp::Remove), Call::JobCancelPurge(7, true));
    let jobs = [(3, JobState::Pending), (9, JobState::Aborted)].map(|(id, state)| Job {
        id,
        state,
        ..Job::default()
    });
    assert_eq!(
        cancel_calls(&jobs),
        [
            Call::JobCancelPurge(3, false),
            Call::JobCancelPurge(9, true)
        ],
        "a finished job is purged, the only way it leaves"
    );
}

#[test]
fn each_job_state_has_its_status_word_and_restart_needs_kept_files() {
    let words = [
        (3, JobState::Pending, "Waiting"),
        (4, JobState::Held, "Paused"),
        (5, JobState::Printing, "Printing"),
        (6, JobState::Stopped, "Stopped"),
        (7, JobState::Canceled, "Deleting"),
        (8, JobState::Aborted, "Error"),
        (9, JobState::Completed, "Printed"),
    ];
    for (code, state, word) in words {
        let g = group(vec![
            ("job-state", vec![Value::Enum(code)]),
            ("job-preserved", vec![Value::Bool(true)]),
        ]);
        let j = Job::from_group(&g);
        assert_eq!((j.state, j.status_word()), (state, word.to_string()));
        assert!(j.preserved);
        let restartable = matches!(
            state,
            JobState::Stopped | JobState::Canceled | JobState::Aborted | JobState::Completed
        );
        assert_eq!(j.restartable(), restartable, "{state:?} with its files");
        let gone = Job {
            preserved: false,
            ..j
        };
        assert_eq!(
            gone.restartable(),
            state == JobState::Stopped,
            "{state:?} without its files"
        );
    }
}

#[test]
fn a_queue_lists_its_jobs_and_aborted_ones_by_number() {
    let job = |id, printer: &str, state| Job {
        id,
        printer: printer.into(),
        state,
        ..Job::default()
    };
    let s = Snapshot {
        jobs: vec![
            job(5, "Laser", JobState::Pending),
            job(6, "Inkjet", JobState::Held),
        ],
        aborted: vec![
            job(2, "Laser", JobState::Aborted),
            job(3, "Inkjet", JobState::Aborted),
        ],
        ..Snapshot::default()
    };
    let ids: Vec<i32> = s.queue_of("Laser").iter().map(|j| j.id).collect();
    assert_eq!(ids, [2, 5]);
    assert_eq!(s.documents_of("Laser").count(), 2);
    assert_eq!(s.jobs_of("Laser").count(), 1);
}

#[test]
fn a_rename_deletes_the_old_queue_when_making_the_new_one_the_default_fails() {
    let deleted = std::cell::Cell::new(false);
    let r = finish_rename("Laser", "Office", Some(Err(Error::NotAuthorized)), || {
        deleted.set(true);
        Ok(())
    });
    assert!(deleted.get());
    assert_eq!(
        r,
        Err(Error::Refused(format!(
            "Laser is now called Office, but Office could not be made the default printer. {}",
            Error::NotAuthorized.text()
        )))
    );

    assert_eq!(
        finish_rename("Laser", "Office", Some(Err(Error::Dismissed)), || Ok(())),
        Err(Error::Refused(
            "Laser is now called Office, but Office could not be made the default printer.".into()
        ))
    );
    assert_eq!(
        finish_rename("Laser", "Office", Some(Ok(())), || Ok(())),
        Ok(())
    );
}

/// A cupsd on a Unix socket: it records every IPP request in order, its operation in `status`,
/// and answers each with the reply `answer` builds (see [`ok`] and [`refused`]).
struct FakeCupsd {
    path: std::path::PathBuf,
    seen: Arc<std::sync::Mutex<Vec<Response>>>,
}

impl FakeCupsd {
    fn start(answer: impl Fn(&Response) -> Request + Send + 'static) -> FakeCupsd {
        use std::io::{Read, Write};
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("reprise-fake-cupsd-{}-{n}", std::process::id()));
        std::fs::remove_file(&path).ok();
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for mut s in listener.incoming().map_while(|s| s.ok()) {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let whole = |raw: &[u8]| {
                    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
                    let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map_or(0, |n| n.trim().parse().unwrap_or(0));
                    (raw.len() >= end + len).then(|| raw[end..end + len].to_vec())
                };
                let body = loop {
                    if let Some(body) = whole(&raw) {
                        break body;
                    }
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break Vec::new(),
                        Ok(n) => raw.extend_from_slice(&buf[..n]),
                    }
                };
                let req = ipp::parse(&body).expect("an IPP request");
                let reply = answer(&req).encode();
                log.lock().unwrap().push(req);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
                    reply.len()
                );
                s.write_all(head.as_bytes()).ok();
                s.write_all(&reply).ok();
            }
        });
        FakeCupsd { path, seen }
    }

    fn client(&self) -> Client {
        Client {
            server: Server::Socket(self.path.clone()),
            user: "ann".into(),
            helper: Arc::new(pkhelper::fake::Fake::default()),
        }
    }

    /// Each request as its operation's name and the printer or job it names, with the queue a
    /// Move-Job sends to.
    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().iter().map(request_line).collect()
    }

    fn sent(&self, op: u16) -> Response {
        let seen = self.seen.lock().unwrap();
        seen.iter().find(|r| r.status == op).cloned().expect("sent")
    }
}

impl Drop for FakeCupsd {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

fn ok() -> Request {
    Request::new(0)
}

/// A refusal with IPP status `status`, saying "Refused.".
fn refused(status: u16) -> Request {
    Request::new(status).op_attr("status-message", Value::text("Refused."))
}

fn request_line(r: &Response) -> String {
    let attr = |tag, name| {
        r.first(tag)
            .and_then(|g| g.str(name))
            .and_then(|u| u.rsplit('/').next())
            .unwrap_or("")
            .to_string()
    };
    match r.status {
        CUPS_ADD_MODIFY_PRINTER => format!("add {}", attr(OPERATION, "printer-uri")),
        CUPS_MOVE_JOB => format!(
            "move {} to {}",
            attr(OPERATION, "job-uri"),
            attr(JOB, "job-printer-uri")
        ),
        GET_JOB_ATTRIBUTES => format!("read {}", attr(OPERATION, "job-uri")),
        GET_PRINTER_ATTRIBUTES => format!("read {}", attr(OPERATION, "printer-uri")),
        PAUSE_PRINTER => format!("pause {}", attr(OPERATION, "printer-uri")),
        CUPS_SET_DEFAULT => format!("default {}", attr(OPERATION, "printer-uri")),
        CUPS_DELETE_PRINTER => format!("delete {}", attr(OPERATION, "printer-uri")),
        op => format!("0x{op:04x}"),
    }
}

fn laser_with_jobs(state: PrinterState, accepting: bool) -> Snapshot {
    let job = |id, state| Job {
        id,
        printer: "Laser".into(),
        state,
        ..Job::default()
    };
    Snapshot {
        running: true,
        printers: vec![Printer {
            name: "Laser".into(),
            device_uri: "file:/dev/null".into(),
            state,
            accepting,
            ..Printer::default()
        }],
        jobs: vec![
            job(5, JobState::Pending),
            job(6, JobState::Pending),
            job(7, JobState::Held),
        ],
        ..Snapshot::default()
    }
}

#[test]
fn a_renamed_queue_is_made_in_the_old_ones_state_before_its_jobs_move() {
    let cases = [
        (
            PrinterState::Stopped,
            false,
            5,
            "a paused queue not accepting",
        ),
        (PrinterState::Stopped, true, 5, "a paused queue"),
        (PrinterState::Idle, true, 3, "an idle queue"),
    ];
    for (state, accepting, made_state, case) in cases {
        let cupsd = FakeCupsd::start(|_| ok());
        let s = laser_with_jobs(state, accepting);
        assert_eq!(
            rename_by_peer(&cupsd.client(), "Laser", "Office", None, &s, None, &[]),
            Ok(()),
            "{case}"
        );
        assert_eq!(
            cupsd.requests(),
            [
                "read Laser",
                "add Office",
                "move 5 to Office",
                "move 6 to Office",
                "move 7 to Office",
                "delete Laser"
            ],
            "{case}: the jobs move once Office is made"
        );
        let add = cupsd.sent(CUPS_ADD_MODIFY_PRINTER);
        let made = add.first(PRINTER).expect("printer attributes");
        assert_eq!(made.int("printer-state"), Some(made_state), "{case}");
        assert_eq!(
            made.bool("printer-is-accepting-jobs"),
            Some(accepting),
            "{case}"
        );
    }
}

#[test]
fn a_renamed_queue_keeps_its_state_message_reasons_policies_users_banners_and_quotas() {
    let settings = [
        ("printer-state-message", vec![Value::text("Out of paper")]),
        (
            "printer-state-reasons",
            vec![
                Value::keyword("paused"),
                Value::keyword("media-empty-error"),
                Value::keyword("connecting-to-device"),
            ],
        ),
        ("printer-error-policy", vec![Value::name("stop-printer")]),
        ("printer-op-policy", vec![Value::name("authenticated")]),
        (
            "job-sheets-default",
            vec![Value::name("standard"), Value::name("none")],
        ),
        (
            "requesting-user-name-denied",
            vec![Value::name("bob"), Value::name("eve")],
        ),
        ("job-quota-period", vec![Value::Int(86400)]),
        ("job-k-limit", vec![Value::Int(1024)]),
        ("job-page-limit", vec![Value::Int(100)]),
    ];
    let reported = settings.clone();
    let cupsd = FakeCupsd::start(move |r| match request_line(r).as_str() {
        "read Laser" => reported
            .iter()
            .fold(ok(), |req, (k, v)| req.add(PRINTER, k, v.clone())),
        _ => ok(),
    });
    let s = laser_with_jobs(PrinterState::Stopped, true);
    let ppd = Some(b"*PPD-Adobe: \"4.3\"\n".to_vec());
    assert_eq!(
        rename_by_peer(&cupsd.client(), "Laser", "Office", None, &s, ppd, &[]),
        Ok(())
    );
    assert_eq!(
        cupsd.requests(),
        [
            "read Laser",
            "add Office",
            "add Office",
            "move 5 to Office",
            "move 6 to Office",
            "move 7 to Office",
            "delete Laser"
        ],
        "the reasons follow the PPD in a request of their own"
    );
    let seen = cupsd.seen.lock().unwrap();
    let made = seen[1].first(PRINTER).expect("printer attributes");
    for (k, v) in settings
        .iter()
        .filter(|(k, _)| *k != "printer-state-reasons")
    {
        assert_eq!(made.get(k), Some(v.as_slice()), "Office is made with {k}");
    }
    assert_eq!(made.get("printer-state-reasons"), None);
    let reasons = seen[2].first(PRINTER).expect("printer attributes");
    assert_eq!(
        reasons.attrs,
        [ipp::Attr {
            name: "printer-state-reasons".into(),
            values: vec![
                Value::keyword("paused"),
                Value::keyword("media-empty-error")
            ],
        }],
        "Office takes Laser's reasons but connecting-to-device"
    );
}

#[test]
fn a_move_refused_halfway_puts_the_moved_jobs_back_and_deletes_the_new_queue() {
    let still_waiting = Value::Enum(3);
    let cupsd = FakeCupsd::start(move |r| match request_line(r).as_str() {
        "move 6 to Office" => refused(0x0404),
        "read 6" => ok().add(JOB, "job-state", vec![still_waiting.clone()]),
        _ => ok(),
    });
    let s = laser_with_jobs(PrinterState::Stopped, true);
    assert_eq!(
        rename_by_peer(&cupsd.client(), "Laser", "Office", None, &s, None, &[]),
        Err(Error::Refused("Refused.".into()))
    );
    assert_eq!(
        cupsd.requests(),
        [
            "read Laser",
            "add Office",
            "move 5 to Office",
            "move 6 to Office",
            "read 6",
            "move 5 to Laser",
            "delete Office"
        ]
    );
}

#[test]
fn a_job_that_finished_or_went_before_its_move_is_left_and_the_rest_move() {
    type Answer = fn(&Response) -> Request;
    let cases: [(&str, Answer); 2] = [
        ("purged", |r| match request_line(r).as_str() {
            "move 6 to Office" | "read 6" => refused(0x0406),
            _ => ok(),
        }),
        ("completed", |r| match request_line(r).as_str() {
            "move 6 to Office" => refused(0x0404),
            "read 6" => ok().add(JOB, "job-state", vec![Value::Enum(9)]),
            _ => ok(),
        }),
    ];
    for (case, answer) in cases {
        let cupsd = FakeCupsd::start(answer);
        let s = laser_with_jobs(PrinterState::Idle, true);
        assert_eq!(
            rename_by_peer(&cupsd.client(), "Laser", "Office", None, &s, None, &[]),
            Ok(()),
            "{case}"
        );
        assert_eq!(
            cupsd.requests(),
            [
                "read Laser",
                "add Office",
                "move 5 to Office",
                "move 6 to Office",
                "read 6",
                "move 7 to Office",
                "delete Laser"
            ],
            "{case}"
        );
    }
}
