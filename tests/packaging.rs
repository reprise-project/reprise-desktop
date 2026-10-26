//! Both PKGBUILDs build reprise-apps, link every program crate to it and install every desktop
//! entry.

use std::collections::BTreeSet;
use std::path::Path;

const PKGBUILDS: [&str; 2] = ["packaging/arch/PKGBUILD", "packaging/arch/local/PKGBUILD"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The `[package]` name of every program crate under `crates/apps`, the multi-call binary aside.
fn app_crates() -> BTreeSet<String> {
    std::fs::read_dir(root().join("crates/apps"))
        .expect("crates/apps lists")
        .map(|e| e.expect("crates/apps entry").path().join("Cargo.toml"))
        .filter(|p| p.exists())
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("Cargo.toml reads");
            let line = text
                .lines()
                .find(|l| l.starts_with("name = "))
                .unwrap_or_else(|| panic!("{} names its package", p.display()));
            line["name = ".len()..].trim_matches('"').to_string()
        })
        .filter(|name| name != "reprise-apps")
        .collect()
}

/// The stem of every `data/apps/*.desktop`.
fn desktop_entries() -> BTreeSet<String> {
    std::fs::read_dir(root().join("data/apps"))
        .expect("data/apps lists")
        .filter_map(|e| {
            let name = e.expect("data/apps entry").file_name().into_string().ok()?;
            name.strip_suffix(".desktop").map(str::to_string)
        })
        .collect()
}

/// The packages `cargo build` names with `-p`.
fn built(pkgbuild: &str) -> BTreeSet<String> {
    let mut words = pkgbuild.split_whitespace();
    let mut out = BTreeSet::new();
    while let Some(w) = words.next() {
        if w == "-p" {
            out.insert(words.next().expect("-p takes a package").to_string());
        }
    }
    out
}

/// The list of the `for app in ...; do` loop that links each program and installs
/// `data/apps/$app.desktop`.
fn installed_apps(pkgbuild: &str) -> BTreeSet<String> {
    let lines: Vec<&str> = pkgbuild.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains("data/apps/$app.desktop"))
        .expect("package() installs data/apps/$app.desktop");
    let head = lines[..at]
        .iter()
        .rev()
        .find_map(|l| l.trim().strip_prefix("for app in "))
        .expect("inside a for app in loop");
    head.trim_end_matches("; do")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[test]
fn both_pkgbuilds_link_every_program_and_install_every_desktop_entry() {
    let apps = app_crates();
    let entries = desktop_entries();
    assert!(apps.contains("reprise-notepad"), "{apps:?}");
    for path in PKGBUILDS {
        let text = std::fs::read_to_string(root().join(path)).expect("PKGBUILD reads");
        let built = built(&text);
        let installed = installed_apps(&text);
        for name in ["reprise", "reprise-portal", "reprise-apps"] {
            assert!(built.contains(name), "{path} builds {name}");
        }
        assert!(
            text.contains(r#"ln -s reprise-apps "$pkgdir/usr/bin/$app""#),
            "{path} links each program to reprise-apps"
        );
        let uninstalled: Vec<_> = apps.difference(&installed).collect();
        assert!(
            uninstalled.is_empty(),
            "{path} links every program: {uninstalled:?}"
        );
        let missing: Vec<_> = entries.difference(&installed).collect();
        assert!(
            missing.is_empty(),
            "{path} installs every desktop entry: {missing:?}"
        );
        let stray: Vec<_> = installed.difference(&entries).collect();
        assert!(
            stray.is_empty(),
            "{path} installs only entries that exist: {stray:?}"
        );
    }
}
