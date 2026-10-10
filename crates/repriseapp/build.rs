//! Embeds the git short hash as REPRISE_BUILD for `about::fields`. Without git
//! or a checkout it emits nothing and the version line drops its "(Build ...)" part.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) else {
        return;
    };
    println!("cargo:rustc-env=REPRISE_BUILD={hash}");
    // HEAD's reflog logs every commit, reset and checkout, even while the branch ref is
    // packed; the branch ref and packed-refs catch moves from another worktree.
    // Only existing files are watched: cargo reruns every build for a missing one.
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]).unwrap_or_default();
    for name in ["HEAD", "logs/HEAD", branch.as_str(), "packed-refs"] {
        if name.is_empty() {
            continue;
        }
        if let Some(path) = git(&["rev-parse", "--git-path", name])
            && Path::new(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
