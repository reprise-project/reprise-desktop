//! Every `WinId::named` literal in the workspace hashes to its own id, so no program routes one
//! window's events to another.

use repriseapp::WinId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_named_window_in_the_workspace_has_its_own_id() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    sources(&root.join("crates"), &mut files);
    sources(&root.join("src"), &mut files);
    const CALL: &str = "WinId::named(\"";
    let mut ids: BTreeMap<WinId, String> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for (at, _) in text.match_indices(CALL) {
            let rest = &text[at + CALL.len()..];
            let name = &rest[..rest.find('"').unwrap()];
            if let Some(other) = ids.insert(WinId::named(name), name.to_string()) {
                assert_eq!(other, name, "{} shares its id", file.display());
            }
        }
    }
    assert!(
        ids.len() > 50,
        "the scan finds the programs' windows: {}",
        ids.len()
    );
}
