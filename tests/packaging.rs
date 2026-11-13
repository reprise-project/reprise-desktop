//! Both PKGBUILDs build reprise-apps, link every program crate to it (or build and install it on
//! its own) and install every desktop entry and polkit file, CI installs their dependencies, the
//! .deb's postinst applies every sysusers and tmpfiles file, its password stacks never reach a
//! fingerprint reader, the time zone polkit rule allows one action only, and the desktop entries
//! file only programs with a settings window under Settings.

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
        let uninstalled: Vec<_> = (apps.difference(&installed))
            .filter(|app| !built.contains(*app))
            .collect();
        assert!(
            uninstalled.is_empty(),
            "{path} links every program: {uninstalled:?}"
        );
        for app in apps.intersection(&built) {
            let binaries = [
                format!("target/release/{app}"),
                format!("target/$_profile/{app}"),
            ];
            assert!(
                (text.lines()).any(|l| l.contains("install -Dm755")
                    && binaries.iter().any(|b| l.contains(b.as_str()))),
                "{path} installs {app}, built on its own"
            );
        }
        let missing: Vec<_> = (entries.difference(&installed))
            .filter(|e| !text.contains(&format!("data/apps/{e}.desktop")))
            .collect();
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

#[test]
fn only_programs_with_a_settings_window_are_filed_under_settings() {
    let settings: BTreeSet<String> = desktop_entries()
        .into_iter()
        .filter(|e| {
            let text =
                std::fs::read_to_string(root().join(format!("data/apps/{e}.desktop"))).unwrap();
            text.lines()
                .find_map(|l| l.strip_prefix("Categories="))
                .is_some_and(|c| c.split(';').any(|c| c == "Settings"))
        })
        .collect();
    assert_eq!(
        settings,
        [
            "reprise-accounts",
            "reprise-bluetooth",
            "reprise-datetime",
            "reprise-network",
            "reprise-power",
            "reprise-printers",
            "reprise-settings",
            "reprise-sound",
            "reprise-updates"
        ]
        .map(String::from)
        .into(),
    );
}

/// The entries of the PKGBUILD array `key` (`depends`, `backup`), version bounds left off.
fn deps(pkgbuild: &str, key: &str) -> Vec<String> {
    let head = format!("\n{key}=(");
    let at = (pkgbuild.find(&head)).unwrap_or_else(|| panic!("the PKGBUILD sets {key}"));
    let rest = &pkgbuild[at + head.len()..];
    let body = &rest[..rest.find(')').unwrap_or_else(|| panic!("{key} closes"))];
    (body.split_whitespace())
        .map(|w| w.trim_matches('\'').split(['<', '>', '=']).next().unwrap())
        .map(|name| if name == "cargo" { "rust" } else { name }.to_string())
        .collect()
}

/// The lines of the job `job` in a workflow: from its heading to the next job's.
fn job<'a>(workflow: &'a str, job: &str) -> Vec<&'a str> {
    let heading = format!("  {job}:");
    (workflow.lines())
        .skip_while(|l| *l != heading)
        .enumerate()
        .take_while(|(i, l)| {
            *i == 0 || !(l.starts_with("  ") && !l.starts_with("   ") && l.ends_with(':'))
        })
        .map(|(_, l)| l)
        .collect()
}

/// The packages of the one `.github/pacman.sh` call among `lines`.
fn pacman_sh(lines: &[&str], what: &str) -> BTreeSet<String> {
    let calls: Vec<&str> = (lines.iter())
        .filter_map(|l| l.split_once(".github/pacman.sh ").map(|(_, pkgs)| pkgs))
        .collect();
    assert_eq!(calls.len(), 1, "{what} calls .github/pacman.sh once");
    calls[0].split_whitespace().map(str::to_string).collect()
}

#[test]
fn ci_installs_every_pkgbuild_dependency() {
    let read = |path: &str| std::fs::read_to_string(root().join(path)).expect("CI file reads");
    let image = read(".github/ci-image/packages");
    let nightly = read(".github/workflows/nightly.yml");
    let lists = [
        (
            "the CI image",
            image.split_whitespace().map(str::to_string).collect(),
        ),
        (
            "nightly.yml's build job",
            pacman_sh(&job(&nightly, "build"), "nightly.yml's build job"),
        ),
    ];
    for pkgbuild in PKGBUILDS {
        let text = std::fs::read_to_string(root().join(pkgbuild)).expect("PKGBUILD reads");
        for dep in deps(&text, "depends")
            .into_iter()
            .chain(deps(&text, "makedepends"))
        {
            for (what, installed) in &lists {
                assert!(
                    installed.contains(&dep),
                    "{what} installs {pkgbuild}'s {dep}"
                );
            }
        }
    }
}

#[test]
fn every_pam_file_is_backed_up_and_installed() {
    let pam: BTreeSet<String> = std::fs::read_dir(root().join("data/pam.d"))
        .expect("data/pam.d lists")
        .map(|e| {
            e.expect("data/pam.d entry")
                .file_name()
                .into_string()
                .unwrap()
        })
        .collect();
    assert!(pam.contains("reprise-login-greeter"), "{pam:?}");
    for path in PKGBUILDS {
        let text = std::fs::read_to_string(root().join(path)).expect("PKGBUILD reads");
        let backup = deps(&text, "backup");
        for name in &pam {
            let file = format!("etc/pam.d/{name}");
            assert!(backup.contains(&file), "{path} backs up {file}");
            let install = format!(r#"install -Dm644 data/pam.d/{name} "$pkgdir/{file}""#);
            assert!(
                text.lines().any(|l| l.trim() == install),
                "{path} installs data/pam.d/{name}"
            );
        }
    }
}

fn file_names(dir: &str) -> BTreeSet<String> {
    std::fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("{dir} lists: {e}"))
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect()
}

#[test]
fn both_pkgbuilds_install_every_polkit_file() {
    let mut files = Vec::new();
    for dir in ["actions", "rules.d"] {
        let names = file_names(&format!("data/polkit-1/{dir}"));
        files.extend(names.iter().map(|n| format!("polkit-1/{dir}/{n}")));
    }
    assert!(
        files.contains(&"polkit-1/rules.d/50-reprise-timezone.rules".to_owned()),
        "{files:?}"
    );
    for path in PKGBUILDS {
        let text = std::fs::read_to_string(root().join(path)).expect("PKGBUILD reads");
        for file in &files {
            let install = format!(r#"install -Dm644 data/{file} "$pkgdir/usr/share/{file}""#);
            assert!(
                text.lines().any(|l| l.trim() == install),
                "{path} installs data/{file} to /usr/share/{file}"
            );
        }
    }
}

/// Every double-quoted string in `js`.
fn quoted(js: &str) -> BTreeSet<&str> {
    js.split('"').skip(1).step_by(2).collect()
}

#[test]
fn the_time_zone_rule_allows_only_set_timezone_to_local_active_admins() {
    let rule =
        std::fs::read_to_string(root().join("data/polkit-1/rules.d/50-reprise-timezone.rules"))
            .unwrap();
    let code: String = (rule.lines())
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        quoted(&code),
        ["org.freedesktop.timedate1.set-timezone", "sudo", "wheel"].into(),
        "the rule names only the set-timezone action and the wheel and sudo groups"
    );
    assert_eq!(code.matches("action.id").count(), 1, "one action check");
    assert_eq!(
        code.matches("isInGroup(").count(),
        2,
        "group checks for wheel and sudo only"
    );
    assert!(
        code.contains("subject.local && subject.active"),
        "the subject must be local and active"
    );
    let results: BTreeSet<&str> = (code.split("polkit.Result.").skip(1))
        .map(|r| r.split(|c: char| !c.is_ascii_alphabetic()).next().unwrap())
        .collect();
    assert_eq!(results, ["YES"].into(), "the rule only ever answers YES");
}

#[test]
fn the_login_unit_sysusers_and_tmpfiles_are_installed() {
    let mut files = vec![(
        "data/systemd/reprise-login.service".to_owned(),
        "usr/lib/systemd/system/reprise-login.service".to_owned(),
    )];
    for (dir, to) in [
        ("data/sysusers.d", "usr/lib/sysusers.d"),
        ("data/tmpfiles.d", "usr/lib/tmpfiles.d"),
    ] {
        let names = file_names(dir);
        assert!(names.contains("reprise-login.conf"), "{dir}: {names:?}");
        files.extend(
            names
                .iter()
                .map(|n| (format!("{dir}/{n}"), format!("{to}/{n}"))),
        );
    }
    for path in PKGBUILDS {
        let text = std::fs::read_to_string(root().join(path)).expect("PKGBUILD reads");
        for (from, to) in &files {
            let install = format!(r#"install -Dm644 {from} "$pkgdir/{to}""#);
            assert!(
                text.lines().any(|l| l.trim() == install),
                "{path} installs {from} to /{to}"
            );
        }
    }
}

#[test]
fn the_deb_postinst_applies_every_sysusers_and_tmpfiles_file() {
    let postinst = std::fs::read_to_string(root().join("packaging/deb/postinst")).unwrap();
    for (dir, command) in [
        ("data/sysusers.d", "systemd-sysusers"),
        ("data/tmpfiles.d", "systemd-tmpfiles --create"),
    ] {
        for name in file_names(dir) {
            let line = format!("{command} {name}");
            assert!(
                postinst.lines().any(|l| l.trim() == line),
                "packaging/deb/postinst runs {line}"
            );
        }
    }
}

#[test]
fn deps_reads_a_wrapped_array() {
    let pkgbuild =
        "pkgname=x\ndepends=('seatd'\n         'libinput>=1.26')\nmakedepends=('cargo')\n";
    assert_eq!(deps(pkgbuild, "depends"), ["seatd", "libinput"]);
    assert_eq!(deps(pkgbuild, "makedepends"), ["rust"]);
}

#[test]
fn ci_pins_one_archive_date() {
    let dates: Vec<(&str, String)> = [
        ".github/ci-image/Dockerfile",
        ".github/workflows/nightly.yml",
        ".github/workflows/supply.yml",
    ]
    .into_iter()
    .flat_map(|path| {
        let text = std::fs::read_to_string(root().join(path)).expect("CI file reads");
        (text.lines())
            .filter_map(|l| {
                let l = l.trim();
                (l.strip_prefix("ARCH_ARCHIVE: ")).or_else(|| l.strip_prefix("ARG ARCH_ARCHIVE="))
            })
            .map(|date| (path, date.to_string()))
            .collect::<Vec<_>>()
    })
    .collect();
    for path in [
        ".github/ci-image/Dockerfile",
        ".github/workflows/supply.yml",
    ] {
        assert!(
            dates.iter().any(|(p, _)| *p == path),
            "{path} sets ARCH_ARCHIVE"
        );
    }
    let first = &dates[0].1;
    for (path, date) in &dates {
        assert_eq!(date, first, "{path} pins ARCH_ARCHIVE to {first}");
    }
}

/// The `auth` modules `service` runs once its includes are followed through `dir`, and the
/// included stacks `dir` doesn't hold.
fn auth_stack(dir: &Path, service: &str) -> (Vec<String>, Vec<String>) {
    let (mut modules, mut outside) = (Vec::new(), Vec::new());
    let text = std::fs::read_to_string(dir.join(service)).unwrap();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&kind, rest)) = words.split_first() else {
            continue;
        };
        if kind.trim_start_matches('-') != "auth" {
            continue;
        }
        if rest.first() == Some(&"include") {
            if dir.join(rest[1]).exists() {
                let (m, o) = auth_stack(dir, rest[1]);
                modules.extend(m);
                outside.extend(o);
            } else {
                outside.push(rest[1].to_owned());
            }
        } else if let Some(module) = rest.iter().find(|w| w.ends_with(".so")) {
            modules.push((*module).to_owned());
        }
    }
    (modules, outside)
}

#[test]
fn the_deb_password_stacks_check_the_password_with_pam_unix_and_no_reader() {
    let tree = std::env::temp_dir().join(format!("reprise-deb-pam-{}", std::process::id()));
    let pam = tree.join("etc/pam.d");
    std::fs::create_dir_all(&pam).unwrap();
    for name in file_names("data/pam.d") {
        std::fs::copy(root().join("data/pam.d").join(&name), pam.join(&name)).unwrap();
    }
    let status = std::process::Command::new(root().join("packaging/deb/pam.sh"))
        .arg(&tree)
        .status()
        .expect("pam.sh runs");
    assert!(status.success(), "pam.sh exits 0");
    for service in ["reprise", "reprise-login", "reprise-login-check"] {
        let (modules, outside) = auth_stack(&pam, service);
        assert!(
            modules.iter().any(|m| m == "pam_unix.so"),
            "{service}: {modules:?}"
        );
        assert!(
            !modules.iter().any(|m| m == "pam_fprintd.so"),
            "{service}: {modules:?}"
        );
        assert!(
            outside.is_empty(),
            "{service} includes no auth stack outside the package: {outside:?}"
        );
    }
    let (modules, _) = auth_stack(&pam, "reprise-fingerprint");
    assert!(modules.iter().any(|m| m == "pam_fprintd.so"), "{modules:?}");
    std::fs::remove_dir_all(&tree).unwrap();
}
