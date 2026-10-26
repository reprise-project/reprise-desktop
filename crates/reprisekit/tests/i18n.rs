//! Every string id in the workspace is in its crate's English `.ftl`, and every id there is used:
//! scripts/i18n-check holds the logic.

use std::path::Path;
use std::process::Command;

#[test]
fn every_string_id_is_in_english_and_every_english_id_is_used() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/i18n-check"))
        .arg(&root)
        .output()
        .expect("python3 runs scripts/i18n-check");
    assert!(
        out.status.success(),
        "scripts/i18n-check:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
