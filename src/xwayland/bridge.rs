//! The clipboard bridge's rules: docs/security.md, "X11 programs". Each event is ruled from the
//! focus and the X11 owner at that moment, never from an earlier event.

use smithay::wayland::selection::SelectionTarget;

/// An X11 selection event the seat rules on. An X11 owner change rules nothing until the new
/// owner lists its targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The selection's X11 owner listed its targets.
    Targets,
    /// The selection's X11 owner let go of it.
    Cleared,
    /// An X11 client asks to read the seat's selection.
    Read,
}

/// The focused window, as the rules see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    None,
    X11,
    /// A Wayland window, with the app id of the Flatpak security context its client came in on.
    Wayland(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The X11 copy becomes the seat's, or the X11 client reads the seat's.
    Allow,
    /// The seat's selection stays as it is.
    Refuse,
    /// The seat's selection, which an X11 client holds, is emptied.
    Clear,
}

/// The rule for every X11 client: it takes and reads while an X11 window has the focus, and an
/// owner letting go empties an X11 client's copy on the seat whatever has the focus.
pub fn base_verdict(event: Event, focus: &Focus, x11_holds: bool) -> Verdict {
    match event {
        Event::Cleared if x11_holds => Verdict::Clear,
        Event::Targets | Event::Read if *focus == Focus::X11 => Verdict::Allow,
        _ => Verdict::Refuse,
    }
}

/// [`base_verdict`], but a clipboard take is allowed while a Wayland window of `owner_app`'s
/// Flatpak app is focused. `owner_app` reads the owner window creator's scope, only for that take.
pub fn verdict(
    event: Event,
    selection: SelectionTarget,
    focus: &Focus,
    x11_holds: bool,
    owner_app: impl FnOnce() -> Option<String>,
) -> Verdict {
    let base = base_verdict(event, focus, x11_holds);
    let Focus::Wayland(Some(app)) = focus else {
        return base;
    };
    let candidate = event == Event::Targets && selection == SelectionTarget::Clipboard;
    if base == Verdict::Refuse && candidate && owner_app().as_ref() == Some(app) {
        return Verdict::Allow;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::super::slot;
    use super::*;

    const PASS: &str = "org.example.Pass";
    const OTHER: &str = "org.example.Other";

    /// An X11 selection owner, as the X server and `/proc` would show it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Owner {
        Host,
        Sandboxed(&'static str),
        /// A `/.flatpak-info` naming [`PASS`] in a host scope.
        Claims,
        /// A pid no process holds.
        Gone,
        /// Xwm's selection window, or nobody: no pid. The root and Xwm's other windows read as
        /// this or as [`Owner::Host`].
        Unnamed,
    }

    /// The `(pid, proc)` the owner's window creator reads back as.
    struct Procs {
        host: std::path::PathBuf,
        pass: std::path::PathBuf,
        other: std::path::PathBuf,
        claims: std::path::PathBuf,
    }

    impl Procs {
        fn new() -> Procs {
            let scope = |app: &str| {
                let info = format!("[Application]\nname={app}\n");
                crate::sandbox::test_proc(&format!("app-flatpak-{app}-7.scope"), Some(&info))
            };
            let pass_info = format!("[Application]\nname={PASS}\n");
            Procs {
                host: crate::sandbox::test_proc("app-reprise-foot-1.scope", None),
                pass: scope(PASS),
                other: scope(OTHER),
                claims: crate::sandbox::test_proc("app-foot-2.scope", Some(&pass_info)),
            }
        }

        fn app(&self, owner: Owner) -> Option<String> {
            let me = std::process::id();
            let (pid, proc) = match owner {
                Owner::Host => (me, &self.host),
                Owner::Sandboxed(PASS) => (me, &self.pass),
                Owner::Sandboxed(_) => (me, &self.other),
                Owner::Claims => (me, &self.claims),
                Owner::Gone => (u32::MAX, &self.pass),
                Owner::Unnamed => return None,
            };
            crate::sandbox::x11_owner_app(pid, proc)
        }
    }

    /// xorshift64*, so a failing sequence replays from its seed.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n
        }

        fn pick<T: Clone>(&mut self, of: &[T]) -> T {
            of[self.below(of.len())].clone()
        }
    }

    #[test]
    fn a_clipboard_take_is_allowed_only_while_its_owners_app_is_focused() {
        let procs = Procs::new();
        let clipboard = SelectionTarget::Clipboard;
        let pass = Focus::Wayland(Some(PASS.into()));
        let take = |owner, selection, focus: &Focus| {
            verdict(Event::Targets, selection, focus, false, || procs.app(owner))
        };
        let pass_owner = Owner::Sandboxed(PASS);
        assert_eq!(take(pass_owner, clipboard, &pass), Verdict::Allow);
        for owner in [
            Owner::Sandboxed(OTHER),
            Owner::Claims,
            Owner::Host,
            Owner::Gone,
            Owner::Unnamed,
        ] {
            assert_eq!(take(owner, clipboard, &pass), Verdict::Refuse, "{owner:?}");
        }
        let primary = SelectionTarget::Primary;
        assert_eq!(take(pass_owner, primary, &pass), Verdict::Refuse);
        let other = Focus::Wayland(Some(OTHER.into()));
        assert_eq!(take(pass_owner, clipboard, &other), Verdict::Refuse);
        let read = verdict(Event::Read, clipboard, &pass, true, || {
            procs.app(pass_owner)
        });
        assert_eq!(read, Verdict::Refuse, "nothing is read through the app");
    }

    #[test]
    fn the_owners_app_is_read_only_for_a_clipboard_take_under_a_flatpak_windows_focus() {
        let unread = || -> Option<String> { panic!("the owner was read") };
        let pass = Focus::Wayland(Some(PASS.into()));
        let clipboard = SelectionTarget::Clipboard;
        for focus in [Focus::None, Focus::X11, Focus::Wayland(None)] {
            verdict(Event::Targets, clipboard, &focus, true, unread);
        }
        verdict(
            Event::Targets,
            SelectionTarget::Primary,
            &pass,
            false,
            unread,
        );
        verdict(Event::Cleared, clipboard, &pass, true, unread);
        verdict(Event::Read, clipboard, &pass, false, unread);
    }

    /// One step of a session on one selection. A targets notify, from any client, is ruled on
    /// the current owner.
    #[derive(Clone, Debug)]
    enum Step {
        Focus(Focus),
        Owner(Owner),
        WaylandCopy,
        Event(Event),
    }

    #[test]
    fn verdict_differs_from_base_verdict_only_at_the_owners_apps_clipboard_take() {
        let procs = Procs::new();
        let focuses = [
            Focus::None,
            Focus::X11,
            Focus::Wayland(None),
            Focus::Wayland(Some(PASS.into())),
            Focus::Wayland(Some(OTHER.into())),
        ];
        let owners = [
            Owner::Host,
            Owner::Sandboxed(PASS),
            Owner::Sandboxed(OTHER),
            Owner::Claims,
            Owner::Gone,
            Owner::Unnamed,
        ];
        let selections = [SelectionTarget::Clipboard, SelectionTarget::Primary];
        let events = [Event::Targets, Event::Cleared, Event::Read];
        let mut allowed = 0;
        for seed in 1..=200u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let mut focus = Focus::None;
            let mut owner = [Owner::Unnamed; 2];
            let mut x11_holds = [false; 2];
            for _ in 0..120 {
                let selection = rng.pick(&selections);
                let n = slot(selection);
                let step = match rng.below(6) {
                    0 => Step::Focus(rng.pick(&focuses)),
                    1 => Step::Owner(rng.pick(&owners)),
                    2 => Step::WaylandCopy,
                    _ => Step::Event(rng.pick(&events)),
                };
                let event = match step {
                    Step::Focus(f) => {
                        focus = f;
                        continue;
                    }
                    Step::Owner(o) => {
                        owner[n] = o;
                        continue;
                    }
                    Step::WaylandCopy => {
                        owner[n] = Owner::Unnamed;
                        x11_holds[n] = false;
                        continue;
                    }
                    Step::Event(event) => event,
                };
                let base = base_verdict(event, &focus, x11_holds[n]);
                let got = verdict(event, selection, &focus, x11_holds[n], || {
                    procs.app(owner[n])
                });
                let apps_take = event == Event::Targets
                    && selection == SelectionTarget::Clipboard
                    && matches!((&focus, owner[n]),
                        (Focus::Wayland(Some(app)), Owner::Sandboxed(o)) if app == o);
                let at = format!(
                    "seed {seed}: {event:?} {selection:?} {focus:?} {:?}",
                    owner[n]
                );
                if apps_take {
                    assert_eq!(got, Verdict::Allow, "{at}");
                    allowed += 1;
                } else {
                    assert_eq!(got, base, "{at}");
                }
                match (event, got) {
                    (Event::Targets, Verdict::Allow) => x11_holds[n] = true,
                    (_, Verdict::Clear) => x11_holds[n] = false,
                    _ => {}
                }
                if event == Event::Cleared {
                    owner[n] = Owner::Unnamed;
                }
            }
        }
        assert!(allowed > 0, "no sequence reached the app's own take");
    }
}
