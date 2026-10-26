//! Whether a toplevel takes the keyboard when it first maps: when it presents a trusted activation token, belongs to or is owned by the focused window's program, Reprise just started or clicked its program, is one of Reprise's own prompts, or nothing has focus. Any other window maps behind the focused one, so a background program cannot pop up mid-typing and read the keys.

use crate::mutex::lock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a launch lets its program's first window take focus. Matches
/// an activation token's lifetime (`activation::TOKEN_LIFETIME`).
pub const LAUNCH_GRACE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launch {
    pub pid: i32,
    pub at: Instant,
}

/// Every launch still within `LAUNCH_GRACE` that has not mapped a window.
/// A static rather than a `Server` field so `spawn` can record into it.
static LAUNCHES: Mutex<Vec<Launch>> = Mutex::new(Vec::new());

pub fn grant(pid: i32) {
    if pid <= 0 {
        return;
    }
    let mut v = lock(&LAUNCHES);
    let now = Instant::now();
    v.retain(|l| now.duration_since(l.at) < LAUNCH_GRACE);
    v.push(Launch { pid, at: now });
}

/// Drop the expired launches and take the one `lineage` or `session` names.
/// Every child Reprise starts leads its own session, so `session` still
/// names the launch once the process between has exited.
pub fn take_launch(
    launches: &mut Vec<Launch>,
    lineage: &[i32],
    session: Option<i32>,
    now: Instant,
) -> Option<Launch> {
    launches.retain(|l| now.duration_since(l.at) < LAUNCH_GRACE);
    let named = |l: &Launch| lineage.contains(&l.pid) || session == Some(l.pid);
    let i = launches.iter().position(named)?;
    Some(launches.remove(i))
}

pub fn take_recorded_launch(lineage: &[i32], session: Option<i32>) -> Option<Launch> {
    let mut v = lock(&LAUNCHES);
    take_launch(&mut v, lineage, session, Instant::now())
}

/// Hand a taken launch back, keeping its time: its window withdrew while
/// it still had the focus, so the program's next window takes the focus
/// in its place. Many X11 toolkits map a first window, then unmap it and
/// map the real one.
pub fn give_back(launch: Launch) {
    let mut v = lock(&LAUNCHES);
    if Instant::now().duration_since(launch.at) < LAUNCH_GRACE {
        v.push(launch);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Arrival<'a> {
    pub activated: bool,
    /// Its Wayland connection by address; `None` for an X11 window, whose
    /// connection is Xwayland's and shared by every X11 program.
    pub client: Option<usize>,
    pub lineage: &'a [i32],
    /// The session its process is in. A program that double-forks its
    /// children (a file manager opening a file) leaves them under init but
    /// in its own session.
    pub session: Option<i32>,
    pub launched: bool,
    /// It belongs to Reprise's portal or polkit agent: a prompt some other
    /// program asked for.
    pub prompt: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Holder {
    pub client: Option<usize>,
    pub pid: Option<i32>,
    /// The focused window owns the new one, up its owner chain: a dialog
    /// another program draws for it, as a portal's print dialog is made
    /// the child of its caller through xdg-foreign.
    pub owns: bool,
}

/// Whether a toplevel mapping while `focused` holds the keyboard takes it.
/// `None` means no window has focus. A program owns the focused window when
/// it shares its Wayland connection, or its process is the focused one's,
/// descends from it or sits in the session it leads (a dialog, a second
/// window, a program it started). A
/// window the focused one owns takes it too, whichever program draws it.
pub fn takes_focus(new: &Arrival, focused: Option<Holder>) -> bool {
    let Some(f) = focused else {
        return true;
    };
    let same_client = new.client.is_some() && new.client == f.client;
    let descends = f
        .pid
        .is_some_and(|p| new.lineage.contains(&p) || new.session == Some(p));
    new.activated || new.launched || new.prompt || same_client || descends || f.owns
}

/// The keyboard's holder: Reprise itself (`reprise`: a dialog or the
/// taskbar has it), owning no program's window, else `window`.
pub fn holder(reprise: bool, window: Option<Holder>) -> Option<Holder> {
    if reprise {
        Some(Holder::default())
    } else {
        window
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnMap {
    /// Under the lock: placed but not focused; `unlock` hands the keyboard
    /// back.
    Locked,
    Elsewhere,
    Focus,
    Behind,
    /// Behind the taskbar's keys: on top, its task button asking for
    /// attention, the keyboard left on the bar.
    Attention,
}

/// How a toplevel mapping on the current desktop (`here`) or not treats
/// the keyboard. `activated` is a trusted activation it presented before
/// the map. A map that may focus at all hands it to `weigh`
/// (`takes_focus`) and uses it up, so the window's next map has to earn
/// the focus again. One that may not while the taskbar has the keyboard
/// (`bar_keys`) asks for attention.
pub fn on_map(
    locked: bool,
    here: bool,
    bar_keys: bool,
    activated: &mut bool,
    weigh: impl FnOnce(bool) -> bool,
) -> OnMap {
    if locked {
        OnMap::Locked
    } else if !here {
        OnMap::Elsewhere
    } else if weigh(std::mem::take(activated)) {
        OnMap::Focus
    } else if bar_keys {
        OnMap::Attention
    } else {
        OnMap::Behind
    }
}

/// The parent pid and session id from the text of `/proc/<pid>/stat`.
/// `comm` sits in parentheses and may hold spaces or `)`, so the fields
/// are read after the last `)`.
fn parse_stat(stat: &str) -> Option<(i32, i32)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut f = rest.split_whitespace();
    let ppid = f.nth(1)?.parse().ok()?;
    let session = f.nth(1)?.parse().ok()?;
    Some((ppid, session))
}

/// Deepest ancestry `lineage` follows; a real chain is a handful deep.
const MAX_LINEAGE: usize = 32;

pub fn lineage(pid: i32) -> (Vec<i32>, Option<i32>) {
    let read = |p: i32| {
        std::fs::read_to_string(format!("/proc/{p}/stat"))
            .ok()
            .and_then(|s| parse_stat(&s))
    };
    let mut chain = vec![pid];
    let first = read(pid);
    let mut next = first.map(|(ppid, _)| ppid);
    while let Some(p) = next
        && p > 1
        && chain.len() < MAX_LINEAGE
    {
        chain.push(p);
        next = read(p).map(|(ppid, _)| ppid);
    }
    (chain, first.map(|(_, sid)| sid))
}

/// Whether the root of `lineage`, a server detached under init, runs the
/// same program as a process below `focused`: a terminal multiplexer whose
/// client sits in the focused terminal, so a program started in one of its
/// panes was started from that terminal. Scans `/proc`, so a host asks it
/// only once every cheaper check has failed.
pub fn serves(lineage: &[i32], focused: i32) -> bool {
    let exe = |p: i32| {
        let exe = std::fs::read_link(format!("/proc/{p}/exe")).ok()?;
        Some((exe, reprisefs::multicall::program_of(p)))
    };
    let Some(&root) = lineage.last().filter(|_| lineage.len() < MAX_LINEAGE) else {
        return false;
    };
    let Some(server) = exe(root) else {
        return false;
    };
    descendants(focused)
        .into_iter()
        .any(|p| p != root && exe(p).as_ref() == Some(&server))
}

/// Every process below `pid`, read from `/proc`.
fn descendants(pid: i32) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let parents: Vec<(i32, i32)> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|p| {
            let stat = std::fs::read_to_string(format!("/proc/{p}/stat")).ok()?;
            Some((p, parse_stat(&stat)?.0))
        })
        .collect();
    let mut found = vec![pid];
    let mut i = 0;
    while let Some(&p) = found.get(i) {
        found.extend(parents.iter().filter(|(_, pp)| *pp == p).map(|(c, _)| *c));
        i += 1;
    }
    found.remove(0);
    found
}

/// The installed programs whose windows are prompts other programs, or a
/// drive the user just plugged in, asked for: the portal's pickers, the
/// polkit agent's password box and the drives applet's passphrase box.
const PROMPTS: &[&str] = &["reprise-portal", "reprise-polkit", "reprise-drives"];

pub fn is_prompt(pid: i32) -> bool {
    PROMPTS
        .iter()
        .any(|p| crate::inputproto::runs_installed(pid, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDITOR: usize = 0x10;
    const OTHER: usize = 0x20;

    fn editor_focused() -> Option<Holder> {
        Some(Holder {
            client: Some(EDITOR),
            pid: Some(500),
            owns: false,
        })
    }

    const STRANGER: &[i32] = &[900, 800];

    #[test]
    fn a_background_window_maps_without_focus() {
        let new = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        assert!(!takes_focus(&new, editor_focused()), "expected no focus");
    }

    #[test]
    fn an_activated_window_takes_focus() {
        let new = Arrival {
            activated: true,
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        assert!(takes_focus(&new, editor_focused()), "expected focus");
    }

    #[test]
    fn a_second_window_of_the_focused_client_takes_focus() {
        let new = Arrival {
            client: Some(EDITOR),
            lineage: &[500, 1],
            ..Arrival::default()
        };
        assert!(takes_focus(&new, editor_focused()), "expected focus");
    }

    #[test]
    fn an_x11_window_of_the_focused_process_takes_focus() {
        let focused = Some(Holder {
            client: None,
            pid: Some(700),
            owns: false,
        });
        let dialog = Arrival {
            lineage: &[700],
            ..Arrival::default()
        };
        assert!(takes_focus(&dialog, focused), "expected focus");
        let stranger = Arrival {
            lineage: &[701],
            ..Arrival::default()
        };
        assert!(!takes_focus(&stranger, focused), "expected no focus");
    }

    #[test]
    fn x11_windows_do_not_match_on_the_shared_xwayland_client() {
        let focused = Some(Holder {
            client: None,
            pid: Some(700),
            owns: false,
        });
        let new = Arrival {
            client: None,
            lineage: &[701],
            ..Arrival::default()
        };
        assert!(!takes_focus(&new, focused), "expected no focus");
    }

    #[test]
    fn a_program_the_focused_one_started_takes_focus() {
        let new = Arrival {
            client: Some(OTHER),
            lineage: &[600, 500, 1],
            ..Arrival::default()
        };
        assert!(takes_focus(&new, editor_focused()), "expected focus");
    }

    #[test]
    fn a_program_in_the_session_the_focused_one_leads_takes_focus() {
        let orphan = Arrival {
            client: Some(OTHER),
            lineage: &[600],
            session: Some(500),
            ..Arrival::default()
        };
        assert!(takes_focus(&orphan, editor_focused()), "expected focus");
        let stranger = Arrival {
            session: Some(501),
            ..orphan
        };
        assert!(
            !takes_focus(&stranger, editor_focused()),
            "expected no focus"
        );
    }

    #[test]
    fn a_server_whose_client_runs_below_the_focused_process_serves_it() {
        // A private copy, so no other test's child runs the same program.
        let dir = std::env::temp_dir().join(format!("reprise-serves-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("server");
        std::fs::copy("/bin/sleep", &exe).unwrap();
        // Another test's fork may still hold the copy's write fd for a moment.
        let sleep = || loop {
            match std::process::Command::new(&exe).arg("30").spawn() {
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                r => break r.unwrap(),
            }
        };
        let (mut server, mut client) = (sleep(), sleep());
        let me = std::process::id() as i32;
        let lineage = [12345, server.id() as i32];
        assert!(
            serves(&lineage, me),
            "expected the client to tie the server to us"
        );
        client.kill().unwrap();
        client.wait().unwrap();
        assert!(!serves(&lineage, me), "expected no tie without a client");
        server.kill().unwrap();
        server.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launched_window_takes_focus() {
        let new = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            launched: true,
            ..Arrival::default()
        };
        assert!(takes_focus(&new, editor_focused()), "expected focus");
    }

    #[test]
    fn a_prompt_takes_focus() {
        let new = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            prompt: true,
            ..Arrival::default()
        };
        assert!(takes_focus(&new, editor_focused()), "expected focus");
    }

    #[test]
    fn a_dialog_the_focused_window_owns_takes_focus_from_another_program() {
        let dialog = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        let owner = editor_focused().map(|h| Holder { owns: true, ..h });
        assert!(takes_focus(&dialog, owner), "expected focus");
        let held = holder(true, owner);
        assert!(!takes_focus(&dialog, held), "expected no focus");
    }

    #[test]
    fn any_window_takes_focus_when_nothing_has_it() {
        let new = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        assert!(takes_focus(&new, None), "expected focus");
    }

    #[test]
    fn a_window_maps_behind_a_dialog_with_the_keyboard() {
        let held = holder(true, None);
        let stranger = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        assert!(!takes_focus(&stranger, held), "expected no focus");
        let launched = Arrival {
            launched: true,
            ..stranger
        };
        assert!(takes_focus(&launched, held), "expected focus");
        assert!(
            takes_focus(&stranger, holder(false, None)),
            "expected focus with no dialog"
        );
    }

    #[test]
    fn a_window_mapping_while_the_taskbar_has_the_keys_asks_for_attention() {
        let held = holder(true, None);
        let stranger = Arrival {
            client: Some(OTHER),
            lineage: STRANGER,
            ..Arrival::default()
        };
        let mut activated = false;
        let verdict = on_map(false, true, true, &mut activated, |_| {
            takes_focus(&stranger, held)
        });
        assert_eq!(verdict, OnMap::Attention);
        let launched = Arrival {
            launched: true,
            ..stranger
        };
        let verdict = on_map(false, true, true, &mut activated, |_| {
            takes_focus(&launched, held)
        });
        assert_eq!(
            verdict,
            OnMap::Focus,
            "a window the bar launched takes focus"
        );
    }

    #[test]
    fn a_pre_map_activation_focuses_one_map_only() {
        let weigh = |activated| {
            let new = Arrival {
                activated,
                client: Some(OTHER),
                lineage: STRANGER,
                ..Arrival::default()
            };
            takes_focus(&new, editor_focused())
        };
        let mut activated = true;
        assert_eq!(
            on_map(false, true, false, &mut activated, weigh),
            OnMap::Focus
        );
        assert!(!activated, "expected the activation used up");
        assert_eq!(
            on_map(false, true, false, &mut activated, weigh),
            OnMap::Behind
        );
    }

    #[test]
    fn a_window_mapping_under_the_lock_is_not_weighed() {
        let mut activated = true;
        let verdict = on_map(true, true, false, &mut activated, |_| panic!("weighed"));
        assert_eq!(verdict, OnMap::Locked);
        assert!(activated, "expected the activation kept");
        let verdict = on_map(false, false, false, &mut activated, |_| panic!("weighed"));
        assert_eq!(verdict, OnMap::Elsewhere);
    }

    #[test]
    fn a_launch_is_taken_by_its_first_window_only() {
        let t0 = Instant::now();
        let mut v = vec![Launch { pid: 42, at: t0 }];
        assert_eq!(
            take_launch(&mut v, &[43, 42], None, t0),
            Some(Launch { pid: 42, at: t0 }),
            "the child matches"
        );
        assert_eq!(
            take_launch(&mut v, &[44, 42], None, t0),
            None,
            "taken already"
        );
    }

    #[test]
    fn a_launch_handed_back_is_taken_by_the_next_window() {
        // A pid no other test grants: the recorded launches are shared.
        let pid = 900_001;
        grant(pid);
        let first = take_recorded_launch(&[pid], None).expect("the first window takes it");
        assert_eq!(take_recorded_launch(&[pid], None), None, "taken already");
        give_back(first);
        assert_eq!(
            take_recorded_launch(&[pid], None),
            Some(first),
            "the next window takes the launch back with its time"
        );
    }

    #[test]
    fn a_launch_matches_through_the_session_it_leads() {
        let t0 = Instant::now();
        let mut v = vec![Launch { pid: 42, at: t0 }];
        assert!(
            take_launch(&mut v, &[43, 1], Some(42), t0).is_some(),
            "expected a match"
        );
    }

    #[test]
    fn a_launch_expires_after_the_grace() {
        let t0 = Instant::now();
        let mut v = vec![Launch { pid: 42, at: t0 }];
        let late = t0 + LAUNCH_GRACE;
        assert_eq!(
            take_launch(&mut v, &[42], None, late),
            None,
            "expected expiry"
        );
        assert!(v.is_empty(), "the expired launch is dropped");
    }

    #[test]
    fn stat_fields_are_read_after_the_last_paren() {
        let stat = "1234 (a) b) (c) S 77 1234 88 34816 1234 4194560";
        assert_eq!(parse_stat(stat), Some((77, 88)));
    }

    #[test]
    fn lineage_of_this_test_reaches_its_parent() {
        let me = std::process::id() as i32;
        let (chain, session) = lineage(me);
        assert_eq!(chain.first(), Some(&me));
        // SAFETY: getppid and getsid(0) only read this process's ids.
        let (ppid, sid) = unsafe { (libc::getppid(), libc::getsid(0)) };
        if ppid > 1 {
            assert_eq!(chain.get(1), Some(&ppid), "the parent comes second");
        }
        assert_eq!(session, Some(sid));
    }
}
