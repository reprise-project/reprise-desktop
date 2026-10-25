//! The window a sound stream plays from, whose icon the mixer borrows (`org.reprise.Tray.WindowIcon`).

use reprisebus::tray::WindowIcon;

use crate::compositor::State;
use crate::window::Kind;

/// Deepest ancestry [`lineage`] follows.
const MAX_LINEAGE: usize = 32;

/// A process as `/proc/<pid>/status` gives it: its parent, and its pid in each pid namespace
/// from the host's inwards (`NSpid`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proc {
    pub ppid: i32,
    pub nspid: Vec<i32>,
}

/// One process of a candidate's ancestry: its host pid, and its innermost pid while it runs in a
/// nested pid namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub host: i32,
    pub inner: Option<i32>,
}

pub fn parse_status(status: &str) -> Option<Proc> {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
    };
    let ppid = field("PPid")?.trim().parse().ok()?;
    let nspid = field("NSpid")
        .map(|v| {
            v.split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    Some(Proc { ppid, nspid })
}

pub fn read_proc(pid: i32) -> Option<Proc> {
    parse_status(&std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?)
}

/// `pid` and its ancestors, nearest first, read with `read`. The walk ends before init and before
/// any of `session`, the compositor and the processes above it, so a stream whose only
/// ancestors are the session's names no window.
pub fn lineage(pid: i32, session: &[i32], read: impl Fn(i32) -> Option<Proc>) -> Vec<Step> {
    let mut chain = Vec::new();
    let mut levels = None;
    let mut next = Some(pid);
    while let Some(p) = next
        && p > 1
        && !session.contains(&p)
        && chain.len() < MAX_LINEAGE
    {
        let proc = read(p);
        let nspid = proc.as_ref().map_or(&[][..], |pr| &pr.nspid[..]);
        // Only the candidate's own namespace counts: an ancestor with fewer levels is outside it.
        let here = *levels.get_or_insert(nspid.len());
        let inner = (nspid.len() > 1 && nspid.len() == here)
            .then(|| nspid.last().copied())
            .flatten();
        chain.push(Step { host: p, inner });
        next = proc.map(|pr| pr.ppid);
    }
    chain
}

/// A window the lookup may pick: its process (the Wayland client's socket pid, an X11 window's
/// `_NET_WM_PID`), whether it is an X11 window, and its title.
pub struct Shown<'a> {
    pub pid: Option<i32>,
    pub x11: bool,
    pub title: &'a str,
}

/// The index in `windows` of the stream's window, first hit wins: one whose process is a step
/// of a lineage (candidates in order, nearest step first); an X11 window whose pid is a step's
/// innermost pid, as a sandboxed program's `_NET_WM_PID` reads; one titled `title`.
pub fn pick(lineages: &[Vec<Step>], title: &str, windows: &[Shown]) -> Option<usize> {
    let steps = || lineages.iter().flatten();
    steps()
        .find_map(|s| windows.iter().position(|w| w.pid == Some(s.host)))
        .or_else(|| {
            steps().find_map(|s| {
                let inner = s.inner?;
                windows.iter().position(|w| w.x11 && w.pid == Some(inner))
            })
        })
        .or_else(|| {
            (!title.is_empty())
                .then(|| windows.iter().position(|w| w.title == title))
                .flatten()
        })
}

/// The icon the taskbar shows for the window `lineages` and `title` pick among those with a
/// task button: the client's own pixels, else the app id's theme icon or entry icon.
pub fn window_icon(state: &mut State, lineages: &[Vec<Step>], title: &str) -> Option<WindowIcon> {
    let wins: Vec<_> = state
        .windows
        .iter()
        .filter(|w| w.mapped && !w.is_owned() && (w.minimized || !w.skips_taskbar()))
        .map(|w| (w.id, w.title(), matches!(w.kind, Kind::X11(_))))
        .collect();
    let shown: Vec<Shown> = wins
        .iter()
        .map(|(id, title, x11)| Shown {
            pid: state.client_pid(*id),
            x11: *x11,
            title,
        })
        .collect();
    let id = wins[pick(lineages, title, &shown)?].0;
    let win = state.win(id)?;
    if let Some(src) = &win.icon {
        return Some(WindowIcon::Pixels {
            width: src.width,
            height: src.height,
            pixels: src.pixels.clone(),
        });
    }
    let app_id = win.app_id();
    crate::taskbarview::app_icon_name(
        &mut state.icons,
        &state.app_index,
        &app_id,
        reprisekit::icon::ICON_SIZE,
    )
    .map(|n| WindowIcon::Named(n.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn procs(list: &[(i32, i32, &[i32])]) -> impl Fn(i32) -> Option<Proc> {
        let map: HashMap<i32, Proc> = list
            .iter()
            .map(|&(pid, ppid, nspid)| {
                (
                    pid,
                    Proc {
                        ppid,
                        nspid: nspid.to_vec(),
                    },
                )
            })
            .collect();
        move |p| map.get(&p).cloned()
    }

    fn host(steps: &[Step]) -> Vec<i32> {
        steps.iter().map(|s| s.host).collect()
    }

    fn win(pid: i32, x11: bool, title: &str) -> Shown<'_> {
        Shown {
            pid: Some(pid),
            x11,
            title,
        }
    }

    #[test]
    fn status_gives_the_parent_and_the_namespace_pids() {
        let status = "Name:\twine-preloader\nPid:\t190459\nPPid:\t190400\nNSpid:\t190459\t2\t327\n";
        assert_eq!(
            parse_status(status),
            Some(Proc {
                ppid: 190400,
                nspid: vec![190459, 2, 327]
            })
        );
        assert_eq!(parse_status("Name:\tx\n"), None, "no PPid");
    }

    #[test]
    fn a_child_process_finds_the_window_of_its_ancestor() {
        // 4312 the audio process, 4000 the browser, 900 the compositor, 800 the session above it.
        let read = procs(&[
            (4312, 4000, &[4312]),
            (4000, 900, &[4000]),
            (900, 800, &[900]),
        ]);
        let chain = lineage(4312, &[900, 800], &read);
        assert_eq!(host(&chain), [4312, 4000]);
        assert!(chain.iter().all(|s| s.inner.is_none()), "not sandboxed");
        let windows = [win(77, false, "Brave"), win(4000, false, "New Tab - Brave")];
        assert_eq!(
            pick(&[chain], "Brave", &windows),
            Some(1),
            "the pid beats the title"
        );
    }

    #[test]
    fn the_session_and_init_name_no_window() {
        let read = procs(&[(500, 1, &[500]), (900, 800, &[900])]);
        assert_eq!(host(&lineage(900, &[900, 800], &read)), Vec::<i32>::new());
        let chain = lineage(500, &[900, 800], &read);
        assert_eq!(host(&chain), [500], "init is never a step");
        assert_eq!(
            pick(
                &[lineage(900, &[900, 800], &read)],
                "",
                &[win(900, false, "")]
            ),
            None
        );
    }

    #[test]
    fn a_sandboxed_x11_window_matches_by_its_namespace_pid() {
        // bwrap (190400) sits in the outer namespace only; the game runs two levels in.
        let read = procs(&[
            (190459, 190420, &[190459, 3, 327]),
            (190420, 190400, &[190420, 2, 2]),
            (190400, 1, &[190400, 1]),
        ]);
        let chain = lineage(190459, &[], &read);
        let inner: Vec<Option<i32>> = chain.iter().map(|s| s.inner).collect();
        assert_eq!(inner, [Some(327), Some(2), None]);
        let windows = [
            win(327, false, "Other"),
            win(327, true, "World of Warcraft"),
        ];
        assert_eq!(
            pick(std::slice::from_ref(&chain), "", &windows),
            Some(1),
            "X11 windows only"
        );
        assert_eq!(
            pick(
                &[vec![Step {
                    host: 190459,
                    inner: None
                }]],
                "",
                &windows
            ),
            None,
            "a single-level NSpid is no sandbox"
        );
    }

    #[test]
    fn the_title_serves_when_no_process_matches() {
        let windows = [
            win(10, true, "World of Warcraft Launcher"),
            win(11, true, "World of Warcraft"),
        ];
        let chain = vec![Step {
            host: 99,
            inner: None,
        }];
        assert_eq!(
            pick(std::slice::from_ref(&chain), "World of Warcraft", &windows),
            Some(1)
        );
        assert_eq!(
            pick(&[chain], "", &[win(12, false, "")]),
            None,
            "an empty title matches nothing"
        );
    }
}
