use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

/// logind's answer to a manager `Can*` method: "yes", "no", "challenge" or "na"; None when the system bus or logind does not answer. Blocks for up to `TIMEOUT`.
pub fn can(method: &str) -> Option<String> {
    zbus::block_on(ask(method))
        .map_err(|e| log::warn!("logind {method} unanswered: {e}"))
        .ok()
}

async fn ask(method: &str) -> zbus::Result<String> {
    let conn = zbus::connection::Builder::system()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            method,
            &(),
        )
        .await?;
    reply.body().deserialize()
}
