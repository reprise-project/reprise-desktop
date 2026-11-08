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
fn the_desktop_holds_documents_home_computer_and_the_bin() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let kids: Vec<Place> = ex.tree.roots[0]
        .children
        .iter()
        .map(|n| ex.tree_places[n.id as usize].clone())
        .collect();
    assert_eq!(
        kids[..4],
        [
            Place::Documents,
            Place::Home,
            Place::Computer,
            Place::RecycleBin
        ]
    );
    assert!(
        kids[4..].iter().all(|p| matches!(p, Place::Folder(_))),
        "the rest are the Desktop folder's subfolders: {kids:?}"
    );
    let labels: Vec<&str> = ex.tree.roots[0].children[..4]
        .iter()
        .map(|n| n.label.as_str())
        .collect();
    let home = reprisefs::places::home_label();
    assert_eq!(
        labels,
        ["Documents", home.as_str(), "Computer", "Recycle Bin"]
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
