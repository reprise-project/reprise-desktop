//! The `ExecStart` binaries of every unit Reprise's session target pulls in: the programs an
//! autostart entry marked `X-systemd-skip` may be left to.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use zbus::Proxy;

/// Reprise's session target (data/systemd). It binds `graphical-session.target`, so units wanted
/// by or part of that target run with the session.
pub const SESSION_TARGET: &str = "reprise-session.target";

/// Where the walk starts. `graphical-session.target` is named as well for a nested run, which
/// does not start [`SESSION_TARGET`].
const ROOTS: [&str; 2] = [SESSION_TARGET, "graphical-session.target"];

/// The dependencies that start a unit along with the one naming them.
const PULLS: [&str; 4] = ["Wants", "Requires", "BindsTo", "Upholds"];

/// Every service requires it, and it is up before any session starts: its sockets and timers
/// are not the session's, and walking them would double the D-Bus round trips.
const BEFORE_SESSION: &str = "basic.target";

/// What the walk needs of one loaded unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub pulls: Vec<String>,
    pub programs: Vec<PathBuf>,
    /// Its last start was skipped because a `Condition*` or `Assert*` failed: its programs did not run, the
    /// units it pulls were queued all the same.
    pub skipped: bool,
}

/// The programs of every unit `roots` pull in, transitively. `unit` describes a unit by name, or
/// gives `None` for one that is not loaded (missing or masked), which is skipped with what it pulls.
pub fn programs_from(
    roots: &[&str],
    mut unit: impl FnMut(&str) -> Option<Unit>,
) -> HashSet<PathBuf> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut todo: Vec<String> = roots.iter().map(|r| r.to_string()).collect();
    let mut programs = HashSet::new();
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(u) = unit(&name) else { continue };
        if !u.skipped {
            programs.extend(u.programs);
        }
        todo.extend(u.pulls.into_iter().filter(|p| !seen.contains(p)));
    }
    programs
}

/// [`programs_from`] the session's roots over the user manager's bus API, each program
/// canonicalised. Empty when the manager does not answer, so every entry starts as it would
/// without systemd. A service with a start job still queued is waited for, up to [`SETTLE`] for
/// the whole walk, so its conditions have been checked.
pub fn session_programs() -> HashSet<PathBuf> {
    let conn = match crate::session() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("systemd user units: {e}");
            return HashSet::new();
        }
    };
    let deadline = Instant::now() + SETTLE;
    programs_from(&ROOTS, |name| {
        if name == BEFORE_SESSION {
            return None;
        }
        load(&conn, name, deadline).unwrap_or_else(|e| {
            log::info!("systemd unit {name}: {e}");
            None
        })
    })
    .into_iter()
    .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
    .collect()
}

const DEST: &str = "org.freedesktop.systemd1";

/// How long one walk waits in all for start jobs queued at login.
const SETTLE: Duration = Duration::from_secs(3);

/// `ExecStart`'s D-Bus signature `a(sasbttttuii)`: the binary's path comes first.
type ExecCommand = (String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32);

fn load(conn: &zbus::Connection, name: &str, deadline: Instant) -> zbus::Result<Option<Unit>> {
    let path: zbus::OwnedObjectPath = crate::wait(conn.call_method(
        Some(DEST),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "LoadUnit",
        &(name,),
    ))?
    .body()
    .deserialize()?;
    // Uncached: `Job` and the condition result are polled while the start job runs.
    let unit: Proxy = crate::wait(
        zbus::proxy::Builder::new(conn)
            .destination(DEST)
            .path(&path)
            .interface("org.freedesktop.systemd1.Unit")
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build(),
    )?;
    if crate::wait(unit.get_property::<String>("LoadState"))? != "loaded" {
        return Ok(None);
    }
    let mut pulls = Vec::new();
    for p in PULLS {
        pulls.extend(crate::wait(unit.get_property::<Vec<String>>(p))?);
    }
    if !name.ends_with(".service") {
        return Ok(Some(Unit {
            pulls,
            ..Unit::default()
        }));
    }
    let service = crate::wait(Proxy::new(
        conn,
        DEST,
        &path,
        "org.freedesktop.systemd1.Service",
    ))?;
    let programs = crate::wait(service.get_property::<Vec<ExecCommand>>("ExecStart"))?
        .into_iter()
        .map(|c| PathBuf::from(c.0))
        .collect();
    while Instant::now() < deadline
        && crate::wait(unit.get_property::<(u32, zbus::OwnedObjectPath)>("Job"))?.0 != 0
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    let checked = |kind: &str| -> zbus::Result<(u64, bool)> {
        Ok((
            crate::wait(unit.get_property::<u64>(&format!("{kind}TimestampMonotonic")))?,
            crate::wait(unit.get_property::<bool>(&format!("{kind}Result")))?,
        ))
    };
    Ok(Some(Unit {
        pulls,
        programs,
        skipped: skipped(checked("Condition")?, checked("Assert")?),
    }))
}

/// Whether a failed check kept the unit's programs from running, from each check's
/// (`*TimestampMonotonic`, `*Result`): a timestamp of 0 was never checked, so the unit is about to run.
fn skipped(condition: (u64, bool), assert: (u64, bool)) -> bool {
    [condition, assert].iter().any(|&(at, ok)| at != 0 && !ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn programs_from_follows_pulls_and_skips_unloaded_units() {
        let units: HashMap<&str, Unit> = HashMap::from([
            (
                "s.target",
                Unit {
                    pulls: vec![
                        "g.target".into(),
                        "pre.target".into(),
                        "masked.service".into(),
                    ],
                    programs: vec![],
                    skipped: false,
                },
            ),
            (
                "g.target",
                Unit {
                    pulls: vec!["agent.service".into(), "s.target".into()],
                    programs: vec![],
                    skipped: false,
                },
            ),
            (
                "pre.target",
                Unit {
                    pulls: vec!["dirs.service".into()],
                    programs: vec![],
                    skipped: false,
                },
            ),
            (
                "agent.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/agent".into()],
                    skipped: false,
                },
            ),
            (
                "dirs.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/dirs-update".into()],
                    skipped: false,
                },
            ),
            (
                "other.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/other".into()],
                    skipped: false,
                },
            ),
        ]);
        let mut asked = Vec::new();
        let got = programs_from(&["s.target"], |n| {
            asked.push(n.to_string());
            units.get(n).cloned()
        });
        let want: HashSet<PathBuf> = ["/usr/bin/agent", "/usr/bin/dirs-update"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(got, want, "a unit nothing pulls in is not the session's");
        assert_eq!(
            asked.len(),
            6,
            "each unit is asked once despite the cycle: {asked:?}"
        );
    }

    #[test]
    fn a_failed_condition_or_assert_skips_the_unit_once_checked() {
        assert!(!skipped((0, false), (0, false)), "not checked yet");
        assert!(!skipped((5, true), (5, true)));
        assert!(skipped((5, false), (0, false)));
        assert!(skipped((5, true), (6, false)), "a failed assert");
        assert!(
            !skipped((5, true), (0, false)),
            "the assert not checked yet"
        );
    }

    #[test]
    fn a_unit_skipped_by_a_failed_condition_runs_no_program_but_still_pulls() {
        let units: HashMap<&str, Unit> = HashMap::from([
            (
                "s.target",
                Unit {
                    pulls: vec!["gated.service".into()],
                    ..Unit::default()
                },
            ),
            (
                "gated.service",
                Unit {
                    pulls: vec!["helper.service".into()],
                    programs: vec!["/usr/bin/gated".into()],
                    skipped: true,
                },
            ),
            (
                "helper.service",
                Unit {
                    programs: vec!["/usr/bin/helper".into()],
                    ..Unit::default()
                },
            ),
        ]);
        let got = programs_from(&["s.target"], |n| units.get(n).cloned());
        assert_eq!(got, HashSet::from([PathBuf::from("/usr/bin/helper")]));
    }
}
