use super::*;

/// Computer's view and its tree node read the drives scanned at the
/// last `/proc/mounts` change: neither stats the mounts again.
#[test]
fn computer_lists_the_cached_drives() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let stick = PathBuf::from("/run/media/u/STICK");
    ex.drives = vec![reprisefs::places::Drive {
        mount: stick.clone(),
        device: "/dev/sdz1".into(),
        fs: "vfat".into(),
        label: Some("STICK".into()),
        removable: true,
        optical: false,
        total: 1000,
        free: 500,
    }];

    ex.go(Place::Computer, &mut ctx);
    assert!(
        ex.items.iter().any(|it| it.path == stick),
        "Computer lists /run/media/u/STICK"
    );
    ex.fill_node(vec![0, 2], &mut ctx);
    let computer = ex.tree.node(&[0, 2]).expect("Computer's node");
    assert!(
        computer
            .children
            .iter()
            .any(|n| n.label == "STICK (/run/media/u/STICK)"),
        "the tree lists STICK (/run/media/u/STICK)"
    );
}

#[test]
fn the_desktop_holds_documents_home_computer_network_and_the_bin() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let kids: Vec<Place> = ex.tree.roots[0]
        .children
        .iter()
        .map(|n| ex.tree_places[n.id as usize].clone())
        .collect();
    assert_eq!(
        kids[..5],
        [
            Place::Documents,
            Place::Home,
            Place::Computer,
            Place::network(),
            Place::RecycleBin
        ]
    );
    assert!(
        kids[5..].iter().all(|p| matches!(p, Place::Folder(_))),
        "the rest are the Desktop folder's subfolders: {kids:?}"
    );
    let labels: Vec<&str> = ex.tree.roots[0].children[..5]
        .iter()
        .map(|n| n.label.as_str())
        .collect();
    let home = reprisefs::places::home_label();
    assert_eq!(
        labels,
        [
            "Documents",
            home.as_str(),
            "Computer",
            "Network",
            "Recycle Bin"
        ]
    );

    let drop: Vec<&Place> = ex
        .address_places
        .iter()
        .filter(|p| !matches!(p, Place::Folder(_)))
        .collect();
    assert_eq!(
        drop,
        [
            &Place::Desktop,
            &Place::Documents,
            &Place::Home,
            &Place::Computer,
            &Place::network(),
            &Place::RecycleBin
        ]
    );
}

#[test]
fn computer_sorts_within_its_groups() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let drive = |mount: &str, label: &str, removable: bool| reprisefs::places::Drive {
        mount: PathBuf::from(mount),
        device: "/dev/sdz1".into(),
        fs: "ext4".into(),
        label: Some(label.into()),
        removable,
        optical: false,
        total: 1000,
        free: 500,
    };
    ex.drives = vec![
        drive("/run/media/u/STICK", "AAA", true),
        drive("/", "AAB", false),
    ];
    ex.sv.sort = Some((0, true));

    ex.go(Place::Computer, &mut ctx);
    let first_of = |title: &str| {
        let g = ex.sv.groups.iter().find(|g| g.title == title);
        g.map(|g| ex.items[g.first].path.clone())
    };
    assert_eq!(
        first_of("Files Stored on This Computer"),
        std::env::var_os("HOME").map(PathBuf::from),
        "the documents folder heads Files Stored on This Computer"
    );
    assert_eq!(
        first_of("Hard Disk Drives"),
        Some(PathBuf::from("/")),
        "/ heads Hard Disk Drives"
    );
    assert_eq!(
        first_of("Devices with Removable Storage"),
        Some(PathBuf::from("/run/media/u/STICK")),
        "the stick heads Devices with Removable Storage"
    );
}

#[test]
fn computer_gains_and_loses_a_locked_stick_as_udisks_reports_it() {
    use reprisefs::udisks::{Disk, Snapshot, Volume};
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let stick = PathBuf::from("/dev/sdx");
    let listed = |ex: &Explorer| ex.items.iter().any(|it| it.path == stick);
    let plugged = Snapshot {
        disks: vec![Disk {
            path: "/drives/stick".into(),
            removable: true,
            ..Default::default()
        }],
        volumes: vec![Volume {
            path: "/block/sdx".into(),
            device: "/dev/sdx".into(),
            disk: Some("/drives/stick".into()),
            label: Some("VAULT".into()),
            fs_type: "crypto_LUKS".into(),
            encrypted: true,
            ..Default::default()
        }],
    };

    let reply = ex.volumes_changed(plugged.clone(), &mut ctx);
    assert!(reply.redraw);
    assert!(listed(&ex), "the locked stick is listed once plugged in");

    let reply = ex.volumes_changed(plugged, &mut ctx);
    assert_eq!(
        reply,
        Reply::none(),
        "an unchanged snapshot re-lists nothing"
    );

    ex.volumes_changed(Snapshot::default(), &mut ctx);
    assert!(!listed(&ex), "the stick's row goes once it is pulled out");
}

#[test]
fn a_udisks_snapshot_that_lands_after_the_drive_scan_marks_its_mounted_stick_removable() {
    use reprisefs::udisks::{Disk, Snapshot, Volume};
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.drives = vec![reprisefs::places::Drive {
        mount: "/run/media/u/STICK".into(),
        device: "/dev/sdz1".into(),
        fs: "vfat".into(),
        label: Some("STICK".into()),
        removable: false,
        optical: false,
        total: 1000,
        free: 500,
    }];
    let snap = Snapshot {
        disks: vec![Disk {
            path: "/drives/stick".into(),
            removable: true,
            ..Default::default()
        }],
        volumes: vec![Volume {
            path: "/block/sdz1".into(),
            device: "/dev/sdz1".into(),
            disk: Some("/drives/stick".into()),
            mounts: vec!["/run/media/u/STICK".into()],
            ..Default::default()
        }],
    };

    ex.volumes_changed(snap, &mut ctx);
    assert!(ex.drives[0].removable, "/dev/sdz1 is marked removable");
}

/// A UDisks2 that never answers GetManagedObjects: Computer still lists at once, since the
/// window reads UDisks2 only through `follow_volumes`' thread.
#[test]
fn computer_lists_while_udisks_does_not_answer() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    struct Silent;
    #[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
    impl Silent {
        async fn get_managed_objects(&self) -> u32 {
            std::future::pending().await
        }
    }

    let Ok(mut daemon) = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        reprisebus::testbus::skipped("no dbus-daemon");
        return;
    };
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().expect("stdout"))
        .read_line(&mut address)
        .expect("the bus address");
    let address = address.trim();
    let (_service, client) = zbus::block_on(async {
        let service = zbus::connection::Builder::address(address)
            .name("org.freedesktop.UDisks2")
            .serve_at("/org/freedesktop/UDisks2", Silent)
            .build()
            .await
            .expect("the service");
        let client = zbus::connection::Builder::address(address)
            .method_timeout(std::time::Duration::from_secs(2))
            .build()
            .await
            .expect("the client");
        (service, client)
    });
    let udisks = reprisefs::udisks::Udisks::on(client);
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let start = std::time::Instant::now();
    let mut ex = Explorer::with_settings(
        Place::Folder(dir.path().to_path_buf()),
        Settings::default(),
        Vec::new().into(),
        Some(udisks),
        &mut ctx,
    );
    ex.mounts_seen.clear();
    ex.go(Place::Computer, &mut ctx);
    let took = start.elapsed();
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(
        took < std::time::Duration::from_secs(1),
        "Computer lists within a second, took {took:?}"
    );
}

#[test]
fn computer_and_the_folders_tree_gain_and_lose_a_mounted_disk_image() {
    use reprisefs::udisks::{Image, Snapshot, Volume};
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let mount = PathBuf::from("/run/media/u/DEBIAN_13");
    let mounted = Snapshot {
        disks: vec![],
        volumes: vec![Volume {
            path: "/block/loop0".into(),
            device: "/dev/loop0".into(),
            label: Some("DEBIAN_13".into()),
            fs_type: "iso9660".into(),
            filesystem: true,
            mounts: vec![mount.clone()],
            image: Some(Image {
                file: "/home/u/debian.iso".into(),
                loop_path: "/block/loop0".into(),
            }),
            ..Default::default()
        }],
    };
    let listed = |ex: &Explorer| {
        ex.items
            .iter()
            .any(|it| it.path == mount && it.drive.is_some())
    };

    ex.volumes_changed(mounted, &mut ctx);
    assert!(listed(&ex), "the image is a drive row in Computer");
    assert!(
        ex.drive_mounts.contains(&mount),
        "and under Computer in the tree"
    );

    ex.volumes_changed(Snapshot::default(), &mut ctx);
    assert!(!listed(&ex), "ejected, its row goes");
    assert!(!ex.drive_mounts.contains(&mount));
}

/// The places under the Folders tree's Computer node, each checked to be the node the tree walks
/// to for its place: a child whose chain runs elsewhere can be clicked but never selected.
fn computer_nodes(ex: &Explorer) -> Vec<Place> {
    let at = ex.tree_path_of(&Place::Computer).expect("a Computer node");
    let node = ex.tree.node(&at).expect("the Computer node");
    (node.children.iter().enumerate())
        .map(|(i, n)| {
            let place = ex.tree_places[n.id as usize].clone();
            let mut path = at.clone();
            path.push(i);
            assert_eq!(
                ex.tree_path_of(&place),
                Some(path),
                "{place:?} under Computer is where the tree walks to it"
            );
            place
        })
        .collect()
}

#[test]
fn computer_in_the_folders_tree_holds_only_drives_across_an_image_mount_and_eject() {
    use reprisefs::udisks::{Image, Snapshot, Volume};
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let drives: Vec<Place> = (ex.drives.iter())
        .map(|d| Place::Folder(d.mount.clone()))
        .collect();
    assert_eq!(computer_nodes(&ex), drives);

    let mount = PathBuf::from("/run/media/u/TESTISO");
    let mounted = Snapshot {
        disks: vec![],
        volumes: vec![Volume {
            path: "/block/loop0".into(),
            device: "/dev/loop0".into(),
            label: Some("TESTISO".into()),
            fs_type: "iso9660".into(),
            filesystem: true,
            mounts: vec![mount.clone()],
            image: Some(Image {
                file: "/home/u/test.iso".into(),
                loop_path: "/block/loop0".into(),
            }),
            ..Default::default()
        }],
    };
    ex.volumes_changed(mounted, &mut ctx);
    let with_image: Vec<Place> = (drives.iter().cloned())
        .chain([Place::Folder(mount)])
        .collect();
    assert_eq!(computer_nodes(&ex), with_image);

    ex.volumes_changed(Snapshot::default(), &mut ctx);
    assert_eq!(computer_nodes(&ex), drives, "ejected, only the drives stay");
}

#[test]
fn computer_in_the_folders_tree_holds_only_drives_across_a_stick_mount_and_removal() {
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let before = ex.drives.clone();
    let stick = PathBuf::from("/run/media/u/STICK");
    let mut plugged = before.clone();
    plugged.push(reprisefs::places::Drive {
        mount: stick,
        device: "/dev/sdb1".into(),
        fs: "vfat".into(),
        label: Some("STICK".into()),
        removable: true,
        optical: false,
        total: 1 << 30,
        free: 1 << 29,
    });
    let places = |ds: &[reprisefs::places::Drive]| -> Vec<Place> {
        ds.iter().map(|d| Place::Folder(d.mount.clone())).collect()
    };

    ex.drives_read("plugged".into(), plugged.clone(), &mut ctx);
    assert_eq!(computer_nodes(&ex), places(&plugged));

    ex.drives_read("pulled".into(), before.clone(), &mut ctx);
    assert_eq!(
        computer_nodes(&ex),
        places(&before),
        "removed, only the drives stay"
    );
}

/// The drive scan stats every mount on a listing thread: on a mount that does not answer the
/// window keeps the drives it has, and takes the scan on the wake after it lands.
#[test]
fn a_drive_scan_that_answers_late_lands_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let scanned = reprisefs::places::drives();
    ex.drives.clear();
    ex.mounts_seen.clear();

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.refresh(&mut ctx);
    assert!(ex.drives.is_empty(), "the drives wait for the scan");
    assert!(ex.scanning.is_some());
    ex.refresh(&mut ctx);
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.scanning.is_none());
    assert!(
        !ex.drives.is_empty() || scanned.is_empty(),
        "the scan landed"
    );
    assert!(
        !ex.mounts_seen.is_empty(),
        "the mount table it scanned is kept"
    );
}

/// The opening drive scan runs on a listing thread too: a window opened while a mount does not
/// answer lists its folder and takes the drives on the wake after the scan lands.
#[test]
fn the_opening_drive_scan_that_answers_late_lands_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let scanned = reprisefs::places::drives();
    let start = Place::Folder(dir.path().to_path_buf());
    let mut ex = Explorer::unopened(
        start.clone(),
        Settings::default(),
        Vec::new().into(),
        None,
        &mut ctx,
    );

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.first_show(start, &mut ctx);
    assert!(ex.drives.is_empty(), "the drives wait for the scan");
    assert!(ex.scanning.is_some());
    wake_until(&mut ex, &mut ctx, |ex| ex.items.len() == 1);
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.scanning.is_none());
    let mounts =
        |d: &[reprisefs::places::Drive]| d.iter().map(|d| d.mount.clone()).collect::<Vec<_>>();
    assert_eq!(
        mounts(&ex.drives),
        mounts(&scanned),
        "free space moves between two scans"
    );
}

fn test_phone() -> reprisefs::portable::Device {
    reprisefs::portable::Device {
        name: "Test Phone".into(),
        kind: reprisefs::places::Gadget::Phone,
        uri: "mtp://Reprise_Test_Phone/".into(),
        mount: None,
    }
}

fn phone_mount(path: PathBuf) -> reprisefs::netshare::Snapshot {
    reprisefs::netshare::Snapshot {
        gio: true,
        shares: vec![reprisefs::netshare::Share {
            name: "mtp".into(),
            icon: "multimedia-player".into(),
            path,
            backend: "mtp".into(),
            host: "Reprise_Test_Phone".into(),
        }],
    }
}

#[test]
fn a_plugged_phone_lists_in_computer_and_its_mount_takes_its_name_and_eject() {
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let (tx, rx) = std::sync::mpsc::channel();
    ex.portable_changes = Some(rx);
    tx.send(vec![test_phone()]).unwrap();
    let row = |ex: &Explorer| ex.items.iter().position(|it| it.name == "Test Phone");
    wake_until(&mut ex, &mut ctx, |ex| row(ex).is_some());
    let i = row(&ex).unwrap();
    assert_eq!(ex.items[i].path, PathBuf::from("mtp://Reprise_Test_Phone/"));
    assert!(!ex.ejectable(i), "nothing to eject before it mounts");
    ex.sv.selection.set_only(Some(i));
    assert_eq!(ex.selected_unmounted_device(), Some(test_phone()));

    let at = PathBuf::from("/run/user/1000/gvfs/mtp:host=Reprise_Test_Phone");
    ex.shares_changed(phone_mount(at.clone()), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        ex.items.iter().any(|it| it.path == at)
    });
    let i = ex.items.iter().position(|it| it.path == at).unwrap();
    assert_eq!(ex.items[i].name, "Test Phone");
    assert_eq!(
        ex.items.iter().filter(|it| it.name == "Test Phone").count(),
        1
    );
    assert!(ex.ejectable(i), "a mounted phone ejects");
    assert!(
        !ex.share_selected(i),
        "its menu reads Eject, not Disconnect"
    );
    assert_eq!(Place::Folder(at.clone()).title(), "Test Phone");
    assert!(computer_nodes(&ex).contains(&Place::Folder(at)));
}

/// A phone's folder walked to before gvfs listed it as a share leaves no folder node behind under
/// its parent once it mounts, or once it goes.
#[test]
fn a_phones_folder_leaves_no_node_under_its_parent_after_a_mount_or_an_unmount() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let at = dir.path().join("mtp:host=Reprise_Test_Phone");
    std::fs::create_dir(&at).expect("the phone's folder");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(at.clone()), &mut ctx);
    // `tempdir` names start with a dot, so the tree hides the chain until hidden entries are shown.
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(Place::Folder(at.clone()), &mut ctx);
    let parent = Place::Folder(dir.path().to_path_buf());
    let kids = |ex: &Explorer| -> Vec<Option<Place>> {
        let Some(node) = ex.tree_path_of(&parent) else {
            return Vec::new();
        };
        let n = ex.tree.node(&node).map_or(0, |n| n.children.len());
        (0..n)
            .map(|i| {
                let mut child = node.clone();
                child.push(i);
                ex.place_of(&child)
            })
            .collect()
    };
    let phone = Some(Place::Folder(at.clone()));
    wake_until(&mut ex, &mut ctx, |ex| kids(ex).contains(&phone));

    ex.shares_changed(phone_mount(at.clone()), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| !kids(ex).contains(&phone));

    std::fs::remove_dir(&at).expect("the phone goes");
    let none = reprisefs::netshare::Snapshot {
        gio: true,
        shares: Vec::new(),
    };
    ex.shares_changed(none, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| !kids(ex).contains(&phone));
    assert_eq!(ex.place, Place::Computer);
}

#[test]
fn a_mount_landing_above_the_open_folder_moves_the_tree_under_computer() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let at = dir.path().join("mtp:host=Reprise_Test_Phone");
    let dcim = at.join("Internal shared storage/DCIM");
    std::fs::create_dir_all(&dcim).expect("the phone's folders");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dcim.clone()), &mut ctx);
    ex.shares_changed(phone_mount(at.clone()), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        ex.tree.selected.is_some() && ex.tree.selected == ex.tree_path_of(&ex.place)
    });
    let computer = ex.tree_path_of(&Place::Computer).unwrap();
    assert_eq!(
        ex.tree.selected.as_ref().map(|p| p.len()),
        Some(computer.len() + 3),
        "DCIM is selected two levels under the phone, Computer's child"
    );
}

#[test]
fn a_mount_landing_under_the_open_folder_moves_the_tree_under_computer() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let at = dir.path().join("mtp:host=Reprise_Test_Phone");
    std::fs::create_dir(&at).expect("the phone's folder");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(at.clone()), &mut ctx);
    assert_eq!(
        ex.place.chain_in(&ex.drive_mounts)[1],
        Place::Computer,
        "before the mount, the folder hangs off a drive"
    );
    ex.shares_changed(phone_mount(at.clone()), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        ex.tree.selected.is_some() && ex.tree.selected == ex.tree_path_of(&ex.place)
    });
    let computer = ex.tree_path_of(&Place::Computer).unwrap();
    assert_eq!(
        ex.tree.selected.as_ref().map(|p| p.len()),
        Some(computer.len() + 1),
        "the phone's folder is selected as Computer's child"
    );
}
