//! Which Flatpak app a process belongs to, from the `.flatpak-info` key
//! file Flatpak puts at the root of every sandbox.

/// The app id of the Flatpak sandbox process `pid` runs in, or None
/// outside one. A claim, not proof: any process in its own mount namespace
/// can put a `.flatpak-info` at its root, so never grant access on it.
pub fn app_of_pid(pid: u32) -> Option<String> {
    app_of_info(&std::fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info")).ok()?)
}

/// `name` in the `[Application]` group of a `.flatpak-info` or instance
/// `info` key file.
pub fn app_of_info(info: &str) -> Option<String> {
    let mut group = "";
    for line in info.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g;
        } else if group == "Application"
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_named_in_the_application_group() {
        let info =
            "[Application]\nname=org.example.Sync\nruntime=runtime/x\n\n[Instance]\nname=other\n";
        assert_eq!(app_of_info(info).as_deref(), Some("org.example.Sync"));
        assert_eq!(app_of_info("[Instance]\nname=x\n"), None);
    }

    #[test]
    fn a_process_outside_a_sandbox_has_no_app() {
        assert_eq!(app_of_pid(std::process::id()), None);
    }
}
