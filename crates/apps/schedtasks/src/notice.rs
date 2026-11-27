//! The notice of a failed run. A task New Task writes runs `reprise-schedtasks --failed NAME`
//! after each run (its `ExecStopPost=`), and systemd hands it how the run ended in
//! `$SERVICE_RESULT`, `$EXIT_CODE` and `$EXIT_STATUS`; a run that did not succeed raises a
//! notification.

use reprisekit::tr;

/// The notification's body for task `name`'s run that ended as `result` (`$SERVICE_RESULT`),
/// `code` and `status`; None for a run that succeeded. A run its conditions skip never starts,
/// so it is never told.
pub fn text(name: &str, result: &str, code: &str, status: &str) -> Option<String> {
    let what = match (result, code) {
        ("success" | "", _) => return None,
        ("exit-code", "exited") => tr!("failed-exit", name = name, status = status),
        ("exit-code", "") => tr!("failed-not-started", name = name),
        ("signal" | "core-dump", _) => tr!("failed-signal", name = name, signal = status),
        ("timeout", _) => tr!("failed-timeout", name = name),
        (result, _) => tr!("failed-other", name = name, result = result),
    };
    Some(format!("{what} {}", tr!("failed-output")))
}

/// Raises the notice for task `name` when the environment says its run failed.
pub fn tell(name: &str) {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let (result, code, status) = (var("SERVICE_RESULT"), var("EXIT_CODE"), var("EXIT_STATUS"));
    let Some(body) = text(name, &result, &code, &status) else {
        return;
    };
    let summary = tr!("failed-title");
    if let Err(e) = reprisebus::notify::notify(&tr!("name"), crate::ICON, &summary, &body) {
        log::warn!("the notice of {name}'s failed run was not shown: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_run_that_did_not_succeed_is_told() {
        let more = "View Output in Scheduled Tasks shows what it wrote.";
        for (result, code, status, said) in [
            (
                "exit-code",
                "exited",
                "2",
                "'Back up' exited with status 2.",
            ),
            (
                "signal",
                "killed",
                "KILL",
                "'Back up' was ended by signal KILL.",
            ),
            (
                "core-dump",
                "dumped",
                "SEGV",
                "'Back up' was ended by signal SEGV.",
            ),
            ("timeout", "killed", "TERM", "'Back up' ran out of time."),
            ("oom-kill", "killed", "KILL", "'Back up' failed: oom-kill."),
        ] {
            assert_eq!(
                text("Back up", result, code, status),
                Some(format!("{said} {more}")),
                "{result}"
            );
        }
        assert_eq!(text("Back up", "success", "exited", "0"), None);
        assert_eq!(
            text("Back up", "exit-code", "", ""),
            Some(format!(
                "'Back up' failed before the program started. {more}"
            )),
            "a step before the program failed, as an at-startup task's stamp"
        );
        assert_eq!(text("Back up", "", "", ""), None, "run by hand");
    }
}
