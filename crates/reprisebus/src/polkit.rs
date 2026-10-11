//! reprise-polkit's record of dismissed password prompts, on the session bus.
//! A service behind polkit answers the same error for a dismissed prompt and
//! a refused password; a program asks here which one it was.

pub const NAME: &str = "reprise.polkit";
pub const PATH: &str = "/reprise/polkit/Dismissals";
pub const INTERFACE: &str = "reprise.polkit.Dismissals";

/// Whether the last password prompt polkit opened for this process was
/// dismissed, forgetting it once asked. False when reprise-polkit does not
/// answer. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn dismissed() -> bool {
    let answer = crate::session().and_then(|conn| {
        conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Take", &())?
            .body()
            .deserialize::<bool>()
    });
    answer.unwrap_or_else(|e| {
        log::warn!("reprise-polkit does not say whether the prompt was dismissed: {e}");
        false
    })
}
