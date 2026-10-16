//! The `ExecStart` binaries of every unit Reprise's session target pulls in: the programs an
//! autostart entry marked `X-systemd-skip` may be left to.

use std::collections::HashSet;
use std::path::PathBuf;
use zbus::blocking::Proxy;

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
        programs.extend(u.programs);
        todo.extend(u.pulls.into_iter().filter(|p| !seen.contains(p)));
    }
    programs
}

/// [`programs_from`] the session's roots over the user manager's bus API, each program
/// canonicalised. Empty when the manager does not answer, so every entry starts as it would
/// without systemd.
pub fn session_programs() -> HashSet<PathBuf> {
    let conn = match crate::session() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("systemd user units: {e}");
            return HashSet::new();
        }
    };
    programs_from(&ROOTS, |name| {
        if name == BEFORE_SESSION {
            return None;
        }
        load(&conn, name).unwrap_or_else(|e| {
            log::info!("systemd unit {name}: {e}");
            None
        })
    })
    .into_iter()
    .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
    .collect()
}

const DEST: &str = "org.freedesktop.systemd1";

/// `ExecStart`'s D-Bus signature `a(sasbttttuii)`: the binary's path comes first.
type ExecCommand = (String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32);

fn load(conn: &zbus::blocking::Connection, name: &str) -> zbus::Result<Option<Unit>> {
    let path: zbus::zvariant::OwnedObjectPath = conn
        .call_method(
            Some(DEST),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "LoadUnit",
            &(name,),
        )?
        .body()
        .deserialize()?;
    let unit = Proxy::new(conn, DEST, &path, "org.freedesktop.systemd1.Unit")?;
    if unit.get_property::<String>("LoadState")? != "loaded" {
        return Ok(None);
    }
    let mut pulls = Vec::new();
    for p in PULLS {
        pulls.extend(unit.get_property::<Vec<String>>(p)?);
    }
    let programs = if name.ends_with(".service") {
        Proxy::new(conn, DEST, &path, "org.freedesktop.systemd1.Service")?
            .get_property::<Vec<ExecCommand>>("ExecStart")?
            .into_iter()
            .map(|c| PathBuf::from(c.0))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Some(Unit { pulls, programs }))
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
                },
            ),
            (
                "g.target",
                Unit {
                    pulls: vec!["agent.service".into(), "s.target".into()],
                    programs: vec![],
                },
            ),
            (
                "pre.target",
                Unit {
                    pulls: vec!["dirs.service".into()],
                    programs: vec![],
                },
            ),
            (
                "agent.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/agent".into()],
                },
            ),
            (
                "dirs.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/dirs-update".into()],
                },
            ),
            (
                "other.service",
                Unit {
                    pulls: vec![],
                    programs: vec!["/usr/bin/other".into()],
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
}
