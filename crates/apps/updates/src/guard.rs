//! The packages an uninstall may not take: Reprise's own, and those the system marks essential
//! (pacman's HoldPkg and the `base` package with its members, dpkg's essential and required ones).

use std::collections::BTreeSet;
use std::path::Path;

/// The names Reprise's packages install as: packaging/arch's pkgname and provides, the .deb's
/// Package.
pub const OWN: [&str; 2] = ["reprise", "reprise-git"];

/// What pacman.conf ships as HoldPkg, assumed when it sets none.
const HOLD_DEFAULT: [&str; 2] = ["pacman", "glibc"];
const PACMAN_CONF: &str = "/etc/pacman.conf";
const PACMAN_LOCAL: &str = "/var/lib/pacman/local";
const BASE: &str = "base";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// The system marks it essential.
    System,
    /// One of [`OWN`].
    Reprise,
}

/// The packages in `set` (the target and every package that goes with it) a removal may not take,
/// given the names the system marks `essential`: sorted by why, then by name.
pub fn blocks(set: &[&str], essential: &BTreeSet<String>) -> Vec<(Why, String)> {
    let mut out: Vec<(Why, String)> = set
        .iter()
        .filter_map(|&n| {
            let why = if OWN.contains(&n) {
                Why::Reprise
            } else if essential.contains(n) {
                Why::System
            } else {
                return None;
            };
            Some((why, n.to_string()))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The sheet's note for non-empty `blocks`: "Uninstalling it would remove glibc (needed by the
/// system) and reprise (part of Reprise)."
pub fn refusal(blocks: &[(Why, String)]) -> String {
    const LISTED: usize = 6;
    let group = |why: Why, reason: &str| {
        let names: Vec<&str> = (blocks.iter())
            .filter(|(w, _)| *w == why)
            .map(|(_, n)| n.as_str())
            .collect();
        if names.is_empty() {
            return None;
        }
        let mut list = names[..names.len().min(LISTED)].join(", ");
        if names.len() > LISTED {
            list.push_str(&format!(" and {} more", names.len() - LISTED));
        }
        Some(format!("{list} ({reason})"))
    };
    let parts: Vec<String> = [
        group(Why::System, "needed by the system"),
        group(Why::Reprise, "part of Reprise"),
    ]
    .into_iter()
    .flatten()
    .collect();
    format!("Uninstalling it would remove {}.", parts.join(" and "))
}

/// The note for a dry run PackageKit refused because it would remove a package the system holds:
/// [`refusal`] when its `details` lead with the package's name ("glibc: could not remove HoldPkg"),
/// else the details.
pub fn held(details: &str) -> String {
    match details.split_once(": ") {
        Some((name, _)) if !name.is_empty() && !name.contains(char::is_whitespace) => {
            refusal(&[(Why::System, name.into())])
        }
        _ => details.into(),
    }
}

/// HoldPkg's names in pacman.conf `text`: every HoldPkg line under `[options]`, `#` starting a
/// comment, or pacman's shipped default when there is none.
pub fn hold_pkg(text: &str) -> Vec<String> {
    let mut section = "";
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim();
            continue;
        }
        if section != "options" {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "HoldPkg"
        {
            names.extend(value.split_whitespace().map(String::from));
        }
    }
    if names.is_empty() {
        names = HOLD_DEFAULT.map(String::from).to_vec();
    }
    names
}

/// A package's name, depends and groups from its pacman local db `desc` file.
#[derive(Debug, Default, PartialEq, Eq)]
struct Desc {
    name: String,
    depends: Vec<String>,
    groups: Vec<String>,
}

fn desc(text: &str) -> Desc {
    let mut d = Desc::default();
    let mut field = "";
    for line in text.lines() {
        if line.starts_with('%') && line.ends_with('%') {
            field = line;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match field {
            "%NAME%" => d.name = line.into(),
            "%DEPENDS%" => {
                let name = line.split(['<', '>', '=', ':']).next().unwrap_or(line);
                d.depends.push(name.trim().into());
            }
            "%GROUPS%" => d.groups.push(line.into()),
            _ => {}
        }
    }
    d
}

/// What pacman marks essential, from pacman.conf `conf` and the local db's `desc` files: HoldPkg,
/// the `base` package, what it depends on, and the members of the `base` group.
fn pacman_essential(conf: &str, descs: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = hold_pkg(conf).into_iter().collect();
    out.insert(BASE.into());
    for d in descs.into_iter().map(|t| desc(&t)) {
        if d.name == BASE {
            out.extend(d.depends);
        } else if d.groups.iter().any(|g| g == BASE) {
            out.insert(d.name);
        }
    }
    out
}

/// The dpkg format [`dpkg_essential`] reads.
const DPKG_FORMAT: &str = "${Package}\t${Essential}\t${Priority}\n";

/// The packages `dpkg-query -W -f DPKG_FORMAT` lists as `Essential: yes` or `Priority: required`.
fn dpkg_essential(out: &str) -> BTreeSet<String> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (name, essential, priority) = (f.next()?, f.next()?, f.next()?);
            (essential == "yes" || priority == "required").then(|| name.to_string())
        })
        .collect()
}

/// The packages among `set` this system marks essential: pacman's where its database is, else
/// dpkg's, else none.
pub fn essential(set: &[&str]) -> BTreeSet<String> {
    if Path::new(PACMAN_LOCAL).is_dir() {
        let conf = std::fs::read_to_string(PACMAN_CONF).unwrap_or_default();
        let descs = std::fs::read_dir(PACMAN_LOCAL)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("desc")).ok());
        return pacman_essential(&conf, descs);
    }
    let out = std::process::Command::new("dpkg-query")
        .args(["-W", "-f", DPKG_FORMAT])
        .args(set)
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) => dpkg_essential(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> BTreeSet<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_normal_removal_is_allowed() {
        let essential = names(&["glibc", "pacman", "base"]);
        assert!(blocks(&["htop"], &essential).is_empty());
        assert!(blocks(&["foot", "foot-terminfo"], &essential).is_empty());
    }

    #[test]
    fn an_essential_package_in_the_set_blocks_it() {
        let essential = names(&["glibc", "pacman", "base"]);
        let got = blocks(&["libfoo", "pacman", "base"], &essential);
        assert_eq!(
            got,
            [(Why::System, "base".into()), (Why::System, "pacman".into())]
        );
        assert_eq!(
            refusal(&got),
            "Uninstalling it would remove base, pacman (needed by the system)."
        );
    }

    #[test]
    fn reprise_s_own_package_blocks_it_even_as_the_target() {
        let got = blocks(&["reprise"], &BTreeSet::new());
        assert_eq!(got, [(Why::Reprise, "reprise".into())]);
        assert_eq!(
            refusal(&got),
            "Uninstalling it would remove reprise (part of Reprise)."
        );
        let both = blocks(&["reprise-git", "glibc", "x"], &names(&["glibc"]));
        assert_eq!(
            refusal(&both),
            "Uninstalling it would remove glibc (needed by the system) and reprise-git \
             (part of Reprise)."
        );
    }

    #[test]
    fn a_dry_run_packagekit_refused_names_the_held_package() {
        assert_eq!(
            held("guardtest-hold: could not remove HoldPkg"),
            "Uninstalling it would remove guardtest-hold (needed by the system)."
        );
        assert_eq!(
            held("Essential packages would be removed: bash"),
            "Essential packages would be removed: bash"
        );
    }

    #[test]
    fn a_long_list_is_cut_short() {
        let set = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let got = blocks(&set, &names(&set));
        assert_eq!(
            refusal(&got),
            "Uninstalling it would remove a, b, c, d, e, f and 2 more (needed by the system)."
        );
    }

    #[test]
    fn own_holds_every_name_the_packaging_installs_as() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../packaging");
        let mut found = BTreeSet::new();
        for pkgbuild in ["arch/PKGBUILD", "arch/local/PKGBUILD"] {
            let text = std::fs::read_to_string(root.join(pkgbuild)).expect("the PKGBUILD reads");
            for line in text.lines() {
                for key in ["pkgname=", "provides="] {
                    if let Some(v) = line.strip_prefix(key) {
                        let v = v.trim_matches(['(', ')']);
                        found.extend(
                            v.split_whitespace()
                                .map(|n| n.trim_matches('\'').to_string()),
                        );
                    }
                }
            }
        }
        let deb = std::fs::read_to_string(root.join("deb/build.sh")).expect("build.sh reads");
        found.extend(
            deb.lines()
                .filter_map(|l| l.strip_prefix("Package: "))
                .map(|n| n.trim().to_string()),
        );
        assert_eq!(found, names(&OWN));
    }

    #[test]
    fn hold_pkg_reads_every_line_of_options_and_skips_comments() {
        let conf = "\
# HoldPkg = commented
[options]
HoldPkg     = pacman glibc  # the defaults
#HoldPkg = nope
HoldPkg = linux
Architecture = auto

[core]
HoldPkg = not-options
Include = /etc/pacman.d/mirrorlist
";
        assert_eq!(hold_pkg(conf), ["pacman", "glibc", "linux"]);
        assert_eq!(
            hold_pkg("[options]\n#HoldPkg = pacman\n"),
            ["pacman", "glibc"]
        );
        assert_eq!(hold_pkg(""), ["pacman", "glibc"]);
    }

    #[test]
    fn pacman_marks_hold_pkg_base_its_depends_and_its_group_essential() {
        let base = "%NAME%\nbase\n\n%VERSION%\n3-3\n\n%DEPENDS%\nfilesystem\nglibc>=2.40\nbash\n\n\
                    %OPTDEPENDS%\nlinux: bare metal support\n";
        let member = "%NAME%\nold-tool\n\n%GROUPS%\nbase\n\n";
        let other = "%NAME%\nhtop\n\n%DEPENDS%\nncurses\n\n%GROUPS%\nutils\n";
        let got = pacman_essential(
            "[options]\nHoldPkg = pacman\n",
            [base, member, other].map(String::from),
        );
        assert_eq!(
            got,
            names(&["base", "bash", "filesystem", "glibc", "old-tool", "pacman"])
        );
    }

    #[test]
    fn dpkg_marks_essential_and_required_packages() {
        let out = "libc6\t\trequired\ncoreutils\tyes\trequired\ndpkg\tyes\trequired\n\
                   bash\tyes\trequired\nhtop\t\toptional\nmawk\t\trequired\ntzdata\t\timportant\n";
        assert_eq!(
            dpkg_essential(out),
            names(&["bash", "coreutils", "dpkg", "libc6", "mawk"])
        );
    }
}
