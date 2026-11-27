use super::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

fn places(cache: &Path) -> Places {
    Places {
        cache: Some(cache.to_path_buf()),
        bins: None,
        flatpak: None,
        flatpak_user: None,
        coredumpctl: None,
        // SAFETY: getuid cannot fail.
        uid: unsafe { libc::getuid() },
        home: Some(PathBuf::from("/nonexistent/n15e-home")),
        kept: Vec::new(),
    }
}

impl Places {
    fn count(&self, part: Part) -> Found {
        self.measure(part, &AtomicBool::new(false))
    }

    fn empty(&self, part: Part) -> io::Result<()> {
        self.clean(part, &Found::nothing(true))
    }
}

/// A cache folder with a file, a folder, a link out to `outside` and a thumbnail cache.
fn cache_in(root: &Path, outside: &Path) -> PathBuf {
    let cache = root.join("cache");
    fs::create_dir_all(cache.join("program/deep")).unwrap();
    fs::write(cache.join("program/deep/blob"), vec![1u8; 16 * 1024]).unwrap();
    fs::write(cache.join("loose"), vec![1u8; 8 * 1024]).unwrap();
    symlink(outside, cache.join("out")).unwrap();
    fs::create_dir_all(cache.join("thumbnails/normal")).unwrap();
    fs::write(cache.join("thumbnails/normal/a.png"), vec![1u8; 4096]).unwrap();
    cache
}

fn outside(root: &Path) -> PathBuf {
    let out = root.join("outside");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("keep"), b"keep").unwrap();
    out
}

#[test]
fn the_caches_go_but_the_thumbnails_and_whatever_a_link_points_at_stay() {
    let t = tempfile::tempdir().unwrap();
    let out = outside(t.path());
    let cache = cache_in(t.path(), &out);
    let p = places(&cache);
    let before = p.count(Part::Caches);
    assert!(before.removable);
    assert!(before.size >= 24 * 1024, "the file and the blob count");
    assert!(
        before.size < 24 * 1024 + p.count(Part::Thumbnails).size + 64 * 1024,
        "the thumbnails are a part of their own"
    );
    p.empty(Part::Caches).unwrap();
    let left: Vec<_> = fs::read_dir(&cache).unwrap().flatten().collect();
    assert_eq!(left.len(), 1, "only the thumbnail cache is left");
    assert!(cache.join("thumbnails/normal/a.png").exists());
    assert!(out.join("keep").exists(), "a link goes as a link");
    assert_eq!(p.count(Part::Caches).size, 0);
}

#[test]
fn the_thumbnails_go_and_their_folder_stays() {
    let t = tempfile::tempdir().unwrap();
    let out = outside(t.path());
    let cache = cache_in(t.path(), &out);
    let p = places(&cache);
    assert!(p.count(Part::Thumbnails).size >= 4096);
    p.empty(Part::Thumbnails).unwrap();
    assert!(cache.join("thumbnails").is_dir());
    assert_eq!(fs::read_dir(cache.join("thumbnails")).unwrap().count(), 0);
    assert!(cache.join("loose").exists(), "the rest of the cache stays");
    assert_eq!(p.count(Part::Thumbnails).size, 0);
}

#[test]
fn a_cache_folder_that_is_a_link_is_neither_counted_nor_emptied() {
    let t = tempfile::tempdir().unwrap();
    let out = outside(t.path());
    let real = cache_in(t.path(), &out);
    let link = t.path().join("linked-cache");
    symlink(&real, &link).unwrap();
    let p = places(&link);
    for part in [Part::Caches, Part::Thumbnails] {
        assert_eq!(p.count(part), Found::nothing(false), "{part:?}");
        assert!(p.empty(part).is_err(), "{part:?}");
    }
    assert!(real.join("loose").exists());
    assert!(real.join("thumbnails/normal/a.png").exists());
}

#[test]
fn a_thumbnail_cache_that_is_a_link_is_left_alone() {
    let t = tempfile::tempdir().unwrap();
    let out = outside(t.path());
    let cache = t.path().join("cache");
    fs::create_dir(&cache).unwrap();
    symlink(&out, cache.join("thumbnails")).unwrap();
    let p = places(&cache);
    assert_eq!(p.count(Part::Thumbnails), Found::nothing(false));
    assert!(p.empty(Part::Thumbnails).is_err());
    p.empty(Part::Caches).unwrap();
    assert!(out.join("keep").exists());
    assert!(fs::symlink_metadata(cache.join("thumbnails")).is_ok());
}

#[test]
fn a_missing_cache_folder_holds_nothing() {
    let t = tempfile::tempdir().unwrap();
    let p = places(&t.path().join("none"));
    assert_eq!(p.count(Part::Caches), Found::nothing(true));
    assert_eq!(p.count(Part::Thumbnails), Found::nothing(true));
}

#[test]
fn emptying_the_recycle_bin_takes_every_item() {
    let t = tempfile::tempdir().unwrap();
    let bin = t.path().join("Trash");
    fs::create_dir(&bin).unwrap();
    for name in ["a", "b"] {
        let f = t.path().join(name);
        fs::write(&f, vec![1u8; 8192]).unwrap();
        crate::trash::trash_in(&bin, &f).unwrap();
    }
    let p = Places {
        bins: Some(Bins::new(bin.clone(), Vec::new())),
        ..places(&t.path().join("cache"))
    };
    assert!(p.count(Part::RecycleBin).size >= 2 * 8192);
    p.empty(Part::RecycleBin).unwrap();
    assert_eq!(fs::read_dir(bin.join("files")).unwrap().count(), 0);
    assert_eq!(p.count(Part::RecycleBin).size, 0);
}

#[test]
fn crash_dumps_count_the_users_own_that_are_still_there() {
    let json = r#"[{"time":1,"pid":10,"uid":1000,"gid":1000,"sig":11,"corefile":"present","exe":"/usr/bin/a","size":4000},
        {"time":2,"pid":11,"uid":1000,"gid":1000,"sig":11,"corefile":"missing","exe":"/usr/bin/a","size":9000},
        {"time":3,"pid":12,"uid":0,"gid":0,"sig":6,"corefile":"present","exe":"/usr/bin/b","size":7000},
        {"time":4,"pid":13,"uid":1000,"gid":1000,"sig":6,"corefile":"present","exe":"/usr/bin/c","size":500}]"#;
    assert_eq!(dumps_size(json, 1000), 4500);
    assert_eq!(dumps_size("", 1000), 0, "no dumps prints nothing");
}

#[test]
fn crash_dumps_are_never_cleaned() {
    let t = tempfile::tempdir().unwrap();
    let p = places(t.path());
    assert!(!p.count(Part::Coredumps).removable);
    assert!(p.empty(Part::Coredumps).is_err());
}

const TABLE: &str = "
Info: these runtimes in installation 'user' are pinned and won't be removed; see flatpak-pin(1):
  runtime/org.example.Pinned/x86_64/1

        ID                                   Branch          Op
 1.     org.gnome.Platform                   47              r
 2.     org.freedesktop.Platform.GL.default  24.08extra      r

Proceed with these changes to the user installation? [Y/n]: n
";

/// [`TABLE`] by the time Yes is pressed: the GL runtime is used again and another went unused.
const TABLE_LATER: &str = "
 1.	   	org.gnome.Platform	47	r
 2.	   	org.example.Newly.Unused	1	r
";

#[test]
fn the_unused_runtimes_are_the_rows_flatpak_would_remove() {
    let rows = unused_rows(TABLE, "x86_64", true);
    let refs: Vec<String> = rows.iter().map(Runtime::full_ref).collect();
    assert_eq!(
        refs,
        [
            "runtime/org.gnome.Platform/x86_64/47",
            "runtime/org.freedesktop.Platform.GL.default/x86_64/24.08extra"
        ]
    );
    assert!(rows.iter().all(|r| r.system));
    let with_arch = " 1.     org.gnome.Platform   i386   47   r\n 2.   org.x.App  1  i\n";
    assert_eq!(
        unused_rows(with_arch, "x86_64", false),
        [Runtime {
            id: "org.gnome.Platform".into(),
            arch: "i386".into(),
            branch: "47".into(),
            system: false,
        }],
        "an install row is not a removal"
    );
    assert!(unused_rows("Nothing unused to uninstall\n", "x86_64", false).is_empty());
}

/// A `flatpak` that logs its arguments to `log`, lists [`TABLE`] for the user installation the
/// first time and [`TABLE_LATER`] after, and nothing for the system one.
fn fake_flatpak(dir: &Path, log: &Path) -> PathBuf {
    let p = dir.join("flatpak");
    let (first, later, seen) = (dir.join("first"), dir.join("later"), dir.join("seen"));
    fs::write(&first, TABLE).unwrap();
    fs::write(&later, TABLE_LATER).unwrap();
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$*\" in\n\
         '--default-arch') echo x86_64 ;;\n\
         'uninstall --unused --user') if [ -e '{seen}' ]; then cat '{later}'; \
         else touch '{seen}'; cat '{first}'; fi ;;\n\
         'uninstall --unused --system') echo 'Nothing unused to uninstall' ;;\n\
         'info --show-size --user --arch=x86_64 org.gnome.Platform 47') echo 3000 ;;\n\
         'info --show-size --user '*) echo 200 ;;\n\
         esac\n",
        log = log.display(),
        seen = seen.display(),
        first = first.display(),
        later = later.display(),
    );
    fs::write(&p, body).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn flatpak_lists_without_removing_and_removes_only_confirmed_runtimes_still_unused() {
    let t = tempfile::tempdir().unwrap();
    let log = t.path().join("log");
    let p = Places {
        flatpak: Some(fake_flatpak(t.path(), &log)),
        flatpak_user: Some(t.path().to_path_buf()),
        ..places(&t.path().join("cache"))
    };
    let found = p.count(Part::Flatpak);
    assert_eq!(found.size, 3200);
    let items: Vec<(&str, &str, bool)> = (found.items.iter())
        .map(|r| (r.id.as_str(), r.branch.as_str(), r.system))
        .collect();
    assert_eq!(
        items,
        [
            ("org.gnome.Platform", "47", false),
            ("org.freedesktop.Platform.GL.default", "24.08extra", false)
        ]
    );
    let listed = fs::read_to_string(&log).unwrap();
    assert!(
        listed.lines().all(|l| !l.contains("noninteractive")
            && !l.contains("-y")
            && !l.contains("assumeyes")),
        "measuring answers nothing: {listed}"
    );
    fs::remove_file(&log).unwrap();
    p.clean(Part::Flatpak, &found).unwrap();
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "--default-arch\nuninstall --unused --user\n\
         uninstall -y --no-related --user runtime/org.gnome.Platform/x86_64/47\n",
        "the system installation, with nothing confirmed, is not asked"
    );
}

#[test]
fn flatpak_lists_only_the_system_installation_while_the_user_has_none() {
    let t = tempfile::tempdir().unwrap();
    let log = t.path().join("log");
    let user = t.path().join("data/flatpak");
    let p = Places {
        flatpak: Some(fake_flatpak(t.path(), &log)),
        flatpak_user: Some(user.clone()),
        ..places(&t.path().join("cache"))
    };
    assert_eq!(p.count(Part::Flatpak), Found::nothing(true));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "--default-arch\nuninstall --unused --system\n"
    );
    assert!(!user.exists());
}

#[test]
fn a_cache_folder_that_is_home_or_above_it_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("u");
    fs::create_dir_all(home.join("Documents")).unwrap();
    fs::write(home.join("Documents/letter.txt"), b"keep").unwrap();
    for cache in [home.clone(), t.path().to_path_buf(), home.join("..")] {
        let p = Places {
            home: Some(home.clone()),
            ..places(&cache)
        };
        for part in [Part::Caches, Part::Thumbnails] {
            assert_eq!(p.count(part), Found::nothing(false), "{cache:?} {part:?}");
            assert!(p.empty(part).is_err(), "{cache:?} {part:?}");
        }
    }
    assert!(home.join("Documents/letter.txt").exists());
}

#[test]
fn a_cache_folder_holding_the_data_folder_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let cache = t.path().join("c");
    fs::create_dir_all(cache.join("data/app")).unwrap();
    fs::write(cache.join("data/app/state"), b"keep").unwrap();
    let p = Places {
        kept: vec![cache.join("data")],
        ..places(&cache)
    };
    assert_eq!(p.count(Part::Caches), Found::nothing(false));
    assert!(p.empty(Part::Caches).is_err());
    assert!(cache.join("data/app/state").exists());
}

#[test]
fn a_cache_folder_holding_where_a_user_folder_link_leads_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let cache = t.path().join("data/stuff");
    fs::create_dir_all(cache.join("docs")).unwrap();
    fs::write(cache.join("docs/letter.txt"), b"keep").unwrap();
    let home = t.path().join("u");
    fs::create_dir(&home).unwrap();
    symlink(cache.join("docs"), home.join("Documents")).unwrap();
    let p = Places {
        home: Some(home.clone()),
        kept: vec![home.join("Documents")],
        ..places(&cache)
    };
    assert_eq!(p.count(Part::Caches), Found::nothing(false));
    assert!(p.empty(Part::Caches).is_err());
    assert!(cache.join("docs/letter.txt").exists());
}

#[test]
fn without_a_home_the_cache_folder_is_refused() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("blob"), b"keep").unwrap();
    let p = Places {
        home: None,
        ..places(t.path())
    };
    for part in [Part::Caches, Part::Thumbnails] {
        assert_eq!(p.count(part), Found::nothing(false), "{part:?}");
    }
    assert!(p.empty(Part::Caches).is_err());
    assert!(t.path().join("blob").exists());
}

#[test]
fn only_the_default_cache_folder_may_be_a_drive_root() {
    let (cache, parent) = ((40, 7), (41, 2));
    assert!(
        !foreign_root(cache, parent, true, Some(cache)),
        "~/.cache as its own subvolume"
    );
    assert!(
        foreign_root(cache, parent, true, Some((40, 9))),
        "another drive root"
    );
    assert!(
        foreign_root(cache, parent, false, None),
        "a root found by its parent's device"
    );
    assert!(
        foreign_root((40, 2), (40, 2), false, None),
        "a folder that is its own parent"
    );
    assert!(!foreign_root(cache, (40, 3), false, None), "a plain folder");
}

#[test]
fn a_relative_cache_folder_is_refused() {
    let p = places(Path::new("rel/cache"));
    assert_eq!(p.count(Part::Caches), Found::nothing(false));
    assert!(p.empty(Part::Caches).is_err());
}

#[test]
fn a_file_with_two_links_counts_once() {
    let t = tempfile::tempdir().unwrap();
    let cache = t.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::write(cache.join("a"), vec![1u8; 64 * 1024]).unwrap();
    let one = p_size(&cache);
    fs::hard_link(cache.join("a"), cache.join("b")).unwrap();
    assert_eq!(p_size(&cache), one);
}

fn p_size(cache: &Path) -> u64 {
    places(cache).count(Part::Caches).size
}

#[test]
fn a_set_stop_ends_a_count_early() {
    let t = tempfile::tempdir().unwrap();
    let cache = t.path().join("cache");
    fs::create_dir_all(cache.join("deep")).unwrap();
    fs::write(cache.join("deep/a"), vec![1u8; 64 * 1024]).unwrap();
    let stopped = places(&cache).measure(Part::Caches, &AtomicBool::new(true));
    assert_eq!(stopped.size, 0);
}

#[test]
fn a_part_the_system_lacks_is_not_offered() {
    let t = tempfile::tempdir().unwrap();
    let p = Places {
        cache: None,
        ..places(t.path())
    };
    assert!(p.parts().is_empty());
    let all = Places {
        bins: Some(Bins::new(t.path().join("Trash"), Vec::new())),
        flatpak: Some("/bin/true".into()),
        coredumpctl: Some("/bin/true".into()),
        ..places(t.path())
    };
    assert_eq!(all.parts(), Part::ALL);
}

#[test]
fn a_read_only_folder_deep_in_the_thumbnails_keeps_only_itself() {
    let t = tempfile::tempdir().unwrap();
    let cache = t.path().join("cache");
    let large = cache.join("thumbnails/large");
    let locked = large.join("locked");
    fs::create_dir_all(&large).unwrap();
    for i in 0..20 {
        fs::write(large.join(format!("a{i}.png")), vec![1u8; 4096]).unwrap();
    }
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("held.png"), vec![1u8; 4096]).unwrap();
    for i in 0..20 {
        fs::write(large.join(format!("b{i}.png")), vec![1u8; 4096]).unwrap();
    }
    fs::create_dir_all(cache.join("thumbnails/normal/inner")).unwrap();
    fs::write(cache.join("thumbnails/normal/inner/c.png"), vec![1u8; 4096]).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let p = places(&cache);
    let got = p.empty(Part::Thumbnails);
    let left = p.count(Part::Thumbnails).size;
    let names: Vec<_> = fs::read_dir(&large)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: geteuid cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    assert_eq!(got.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        names,
        ["locked"],
        "every file next to the read-only folder goes"
    );
    assert!(locked.join("held.png").exists());
    assert!(
        !cache.join("thumbnails/normal").exists(),
        "the other folders go"
    );
    let blocks = |p: &Path| fs::symlink_metadata(p).unwrap().blocks() * 512;
    assert_eq!(
        left,
        blocks(&large) + blocks(&locked) + blocks(&locked.join("held.png")),
        "a count afterwards finds only what stayed"
    );
}
