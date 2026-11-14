use super::*;

/// A profile of class `class`, version `major`, holding `tags` in order.
fn profile(major: u8, class: &[u8; 4], tags: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut at = HEADER + 4 + tags.len() * 12;
    let mut table = Vec::new();
    let mut body = Vec::new();
    for (sig, data) in tags {
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(at as u32).to_be_bytes());
        table.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        at += data.len();
    }
    let mut b = vec![0; HEADER];
    b[0..4].copy_from_slice(&(at as u32).to_be_bytes());
    b[8] = major;
    b[12..16].copy_from_slice(class);
    b[16..20].copy_from_slice(b"RGB ");
    b[36..40].copy_from_slice(b"acsp");
    b.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    b.extend(table);
    b.extend(body);
    b
}

fn desc(s: &str) -> Vec<u8> {
    let mut b = b"desc\0\0\0\0".to_vec();
    b.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
    b.extend_from_slice(s.as_bytes());
    b.push(0);
    b.extend_from_slice(&[0; 79]);
    b
}

fn mluc(records: &[(&[u8; 4], &str)]) -> Vec<u8> {
    let mut b = b"mluc\0\0\0\0".to_vec();
    b.extend_from_slice(&(records.len() as u32).to_be_bytes());
    b.extend_from_slice(&12u32.to_be_bytes());
    let mut at = 16 + records.len() * 12;
    let mut strings = Vec::new();
    for (lang, s) in records {
        let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        b.extend_from_slice(*lang);
        b.extend_from_slice(&(units.len() as u32).to_be_bytes());
        b.extend_from_slice(&(at as u32).to_be_bytes());
        at += units.len();
        strings.extend(units);
    }
    b.extend(strings);
    b
}

fn table(channels: u16, width: u16, values: &[u32]) -> Vec<u8> {
    let entries = values.len() as u16 / channels;
    let mut b = b"vcgt\0\0\0\0".to_vec();
    b.extend_from_slice(&0u32.to_be_bytes());
    for v in [channels, entries, width] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    for &v in values {
        match width {
            1 => b.push(v as u8),
            _ => b.extend_from_slice(&(v as u16).to_be_bytes()),
        }
    }
    b
}

fn formula(f: [(f64, f64, f64); 3]) -> Vec<u8> {
    let mut b = b"vcgt\0\0\0\0".to_vec();
    b.extend_from_slice(&1u32.to_be_bytes());
    for (g, lo, hi) in f {
        for v in [g, lo, hi] {
            b.extend_from_slice(&((v * 65536.0).round() as i32).to_be_bytes());
        }
    }
    b
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_v2_profile_gives_its_description_and_its_three_channel_table() {
    let b = profile(
        2,
        b"mntr",
        &[
            (b"desc", desc("Studio monitor")),
            (b"vcgt", table(3, 2, &[0, 65535, 0, 32768, 1000, 60000])),
        ],
    );
    let p = parse(&b).unwrap();
    assert_eq!(p.description, "Studio monitor");
    assert!(p.display);
    let v = p.vcgt.unwrap();
    assert_eq!((v.value(0, 0.0), v.value(0, 1.0)), (0.0, 1.0));
    assert!(near(v.value(0, 0.25), 0.25), "linear between points");
    assert!(near(v.value(1, 1.0), 32768.0 / 65535.0));
    assert!(near(v.value(2, 0.0), 1000.0 / 65535.0));
}

#[test]
fn a_v4_profile_reads_its_us_english_name_and_a_one_channel_byte_table() {
    let names = mluc(&[(b"deDE", "Bildschirm"), (b"enUS", "Screen")]);
    let b = profile(
        4,
        b"mntr",
        &[(b"desc", names), (b"vcgt", table(1, 1, &[0, 51, 255]))],
    );
    let p = parse(&b).unwrap();
    assert_eq!(p.description, "Screen");
    let v = p.vcgt.unwrap();
    for c in 0..3 {
        assert!(near(v.value(c, 0.5), 0.2), "one table for all three");
        assert!(near(v.value(c, 0.75), 0.6));
    }
    let only = profile(4, b"mntr", &[(b"desc", mluc(&[(b"frFR", "Écran")]))]);
    assert_eq!(parse(&only).unwrap().description, "Écran", "else the first");
}

#[test]
fn a_formula_vcgt_is_min_plus_range_times_x_to_the_gamma() {
    let b = profile(
        2,
        b"mntr",
        &[(
            b"vcgt",
            formula([(1.0, 0.0, 1.0), (2.0, 0.0, 1.0), (1.0, 0.1, 0.9)]),
        )],
    );
    let v = parse(&b).unwrap().vcgt.unwrap();
    assert!(near(v.value(0, 0.5), 0.5));
    assert!(near(v.value(1, 0.5), 0.25));
    assert!(near(v.value(2, 0.0), 0.1));
    assert!(near(v.value(2, 1.0), 0.9));
    let wild = profile(
        2,
        b"mntr",
        &[(
            b"vcgt",
            formula([(0.0, 0.0, 1.0), (1.0, 0.0, 1.0), (1.0, 0.0, 1.0)]),
        )],
    );
    assert!(parse(&wild).is_err(), "a gamma of 0 is refused");
}

#[test]
fn a_profile_without_a_vcgt_has_no_curves_and_input_profiles_are_not_displays() {
    let b = profile(2, b"scnr", &[(b"desc", desc("Scanner"))]);
    let p = parse(&b).unwrap();
    assert!(!p.display);
    assert_eq!(p.vcgt, None);
}

#[test]
fn broken_profiles_are_refused() {
    let good = profile(2, b"mntr", &[(b"vcgt", table(3, 2, &[0; 6]))]);
    let mut b = good.clone();
    b[36] = b'x';
    assert!(parse(&b).is_err(), "no acsp");
    let mut b = good.clone();
    b[8] = 3;
    assert!(parse(&b).is_err(), "version 3");
    let mut b = good.clone();
    b[HEADER..HEADER + 4].copy_from_slice(&(MAX_TAGS + 1).to_be_bytes());
    assert!(parse(&b).is_err(), "too many tags");
    assert!(parse(&good[..good.len() - 1]).is_err(), "table cut short");
    assert!(parse(&good[..100]).is_err(), "header cut short");
    let odd = profile(2, b"mntr", &[(b"vcgt", table(2, 2, &[0; 4]))]);
    assert!(parse(&odd).is_err(), "two channels");
    let wide = profile(2, b"mntr", &[(b"vcgt", table(3, 4, &[0; 6]))]);
    assert!(parse(&wide).is_err(), "four-byte entries");
    let mut huge = table(3, 2, &[0; 6]);
    huge.resize(MAX_VCGT as usize + 1, 0);
    assert!(parse(&profile(2, b"mntr", &[(b"vcgt", huge)])).is_err());
}

#[test]
fn a_tag_past_the_profiles_end_is_refused() {
    let mut b = profile(2, b"mntr", &[(b"vcgt", table(3, 2, &[0; 6]))]);
    let entry = HEADER + 4;
    b[entry + 8..entry + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse(&b).is_err());
}

/// Every byte of a valid profile flipped at random, a few thousand times: an Err or a profile whose
/// curves stay in 0..=1, never a panic.
#[test]
fn random_damage_never_panics() {
    let good = profile(
        4,
        b"mntr",
        &[
            (b"desc", mluc(&[(b"enUS", "Screen")])),
            (
                b"vcgt",
                table(3, 2, &[0, 30000, 65535, 0, 100, 200, 9, 8, 7]),
            ),
        ],
    );
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..4000 {
        let mut b = good.clone();
        for _ in 0..1 + next() % 4 {
            let i = (next() as usize) % b.len();
            b[i] = next() as u8;
        }
        let cut = (next() as usize) % (b.len() + 1);
        let b = if next() % 4 == 0 { &b[..cut] } else { &b[..] };
        if let Ok(Profile { vcgt: Some(v), .. }) = parse(b) {
            for c in 0..3 {
                for x in [0.0, 0.3, 1.0] {
                    assert!((0.0..=1.0).contains(&v.value(c, x)));
                }
            }
        }
    }
}

#[test]
fn a_file_over_the_cap_is_refused_unread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.icc");
    let f = File::create(&path).unwrap();
    f.set_len(MAX_FILE + 1).unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn the_list_names_display_profiles_in_folders_and_skips_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("vendor/sub")).unwrap();
    let write = |name: &str, b: &[u8]| std::fs::write(d.join(name), b).unwrap();
    write("b.icc", &profile(2, b"mntr", &[(b"desc", desc("Bravo"))]));
    write("vendor/sub/a.ICM", &profile(4, b"mntr", &[]));
    write("scan.icc", &profile(2, b"scnr", &[(b"desc", desc("Scan"))]));
    write("junk.icc", b"not a profile");
    write("c.txt", &profile(2, b"mntr", &[(b"desc", desc("Text"))]));
    crate::testfs::mkfifo(&d.join("fifo.icc"));
    let dirs = vec![d.to_path_buf(), d.join("missing")];
    let extra = vec![d.join("b.icc")];
    let listed = crate::testfs::within(move || list(&dirs, &extra));
    assert_eq!(
        listed,
        [
            Listed {
                path: d.join("vendor/sub/a.ICM"),
                name: "a.ICM".into(),
            },
            Listed {
                path: d.join("b.icc"),
                name: "Bravo".into(),
            },
        ],
        "the file name stands in for a missing description; each path once"
    );
}
