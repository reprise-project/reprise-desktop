use std::path::PathBuf;

fn service_lines(path: &str) -> Vec<String> {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../data")
        .join(path);
    let unit = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut inside = false;
    let mut lines = Vec::new();
    for line in unit.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Service]";
        } else if inside && !line.is_empty() && !line.starts_with('#') {
            lines.push(line.to_owned());
        }
    }
    lines
}

#[test]
fn the_login_unit_reloads_its_settings_on_sighup() {
    let lines = service_lines("systemd/reprise-login.service");
    assert!(
        lines
            .iter()
            .any(|l| l == "ExecReload=/bin/kill -HUP $MAINPID"),
        "[Service] has `ExecReload=/bin/kill -HUP $MAINPID`: {lines:?}"
    );
}
