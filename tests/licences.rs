//! Every crate is GPL-3.0-or-later, so no tracked source may cite a source
//! under a licence that cannot be relicensed that way.

use std::process::Command;

#[test]
fn no_source_cites_an_incompatible_licence() {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new("git")
        .args(["-C", root, "ls-files", "-z", "--", "*.rs"])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git ls-files succeeds");
    // A bare GPL-3.0 in parentheses leaves "or later" open, and the named
    // compositor is GPL-3.0 only. Split so this file does not match itself.
    let banned = [
        concat!("GPL-3.0", "-only"),
        concat!("GPL-2", ".0"),
        concat!("(GPL-3.0", ")"),
        concat!("cosmic", "-comp"),
    ];
    let mut hits = Vec::new();
    for path in out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(path).expect("UTF-8 path");
        let Ok(text) = std::fs::read_to_string(format!("{root}/{path}")) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if banned.iter().any(|b| line.contains(b)) {
                hits.push(format!("{path}:{}: {line}", n + 1));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "every source is GPL-3.0-or-later compatible:\n{}",
        hits.join("\n")
    );
}
