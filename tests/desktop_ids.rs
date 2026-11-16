//! A name becomes a desktop file id, and loses its `.desktop`, only in
//! `reprisefs::apps::DesktopId`; app ids go through `AppId`.

use std::process::Command;

/// The one file that may take the suffix apart.
const OWNER: &str = "crates/reprisefs/src/apps/desktopid.rs";

#[test]
fn only_desktop_id_takes_the_desktop_suffix_apart() {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new("git")
        .args(["-C", root, "ls-files", "-z", "--", "*.rs"])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git ls-files succeeds");
    // Split so this file does not match itself.
    let suffix = concat!("(\".", "desktop\")");
    let banned = ["trim_end_matches", "strip_suffix", "ends_with"].map(|f| format!("{f}{suffix}"));
    let mut hits = Vec::new();
    for path in out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(path).expect("UTF-8 path");
        if path == OWNER {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(format!("{root}/{path}")) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if banned.iter().any(|b| line.contains(b.as_str())) {
                hits.push(format!("{path}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "every .desktop name goes through DesktopId::from_file_name or AppId::desktop_ids:\n{}",
        hits.join("\n")
    );
}
