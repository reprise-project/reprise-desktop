//! Every string id in the workspace is in its crate's English `.ftl`, every id there is used, and
//! a translation uses only its English message's variables: scripts/i18n-check holds the logic.

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

#[test]
fn a_translation_using_a_variable_its_english_message_lacks_fails_the_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/i18n-check"))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i18n-variables"))
        .output()
        .expect("python3 runs scripts/i18n-check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        "data/locale/de/demo.ftl: \"size\" uses $kib, which the English message does not\n\
         data/locale/de/demo.ftl: \"window-title\" uses $nme, which the English message does not\n"
    );
}
