//! Tests serve D-Bus interfaces on a built connection only through `testbus::serve`, which waits
//! until the connection takes calls; `ObjectServer::at` drops the calls that come first.

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The test code of `file`, whose text is `text`: all of it in a tests folder or file, else
/// what follows its inline `#[cfg(test)] mod`.
fn test_code<'t>(file: &Path, text: &'t str) -> Option<&'t str> {
    let whole = file.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c == "tests" || c.ends_with("_tests") || c.ends_with("tests.rs")
    });
    if whole {
        return Some(text);
    }
    let mut at = 0;
    while let Some(found) = text[at..].find("#[cfg(test)]") {
        let start = at + found;
        let next = text[start + "#[cfg(test)]".len()..].trim_start();
        let item = (next.strip_prefix("pub"))
            .map_or(next, |r| r.trim_start_matches(|c| c != ' ').trim_start());
        if item.starts_with("mod ") && item.split(['\n', ';']).next().unwrap().contains('{') {
            return Some(&text[start..]);
        }
        at = start + 1;
    }
    None
}

/// The lines in `code` (counted from `first`) that call `at` on an object server, chained or
/// through a variable it was bound to.
fn at_calls(code: &str, first: usize) -> Vec<usize> {
    let mut compact = String::new();
    let mut lines = Vec::new();
    for (n, line) in code.lines().enumerate() {
        let line = line.split("//").next().unwrap();
        for c in line.chars().filter(|c| !c.is_whitespace()) {
            compact.push(c);
            lines.push(first + n);
        }
    }
    let mut hits = Vec::new();
    const SERVER: &str = "object_server()";
    let mut bound = Vec::new();
    for (i, _) in compact.match_indices(SERVER) {
        let rest = compact[i + SERVER.len()..].trim_start_matches(')');
        if rest.starts_with(".at(") {
            hits.push(lines[i]);
        }
        if rest.starts_with(';') {
            let before = &compact[..i];
            if let Some(eq) = before.rfind('=')
                && let Some(name) = before[..eq].rsplit_once("let").map(|(_, n)| n)
            {
                bound.push(format!("{name}.at("));
            }
        }
    }
    for name in bound {
        hits.extend(compact.match_indices(&name).map(|(i, _)| lines[i]));
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

#[test]
fn tests_serve_interfaces_only_through_testbus_serve() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    sources(&root.join("crates"), &mut files);
    sources(&root.join("src"), &mut files);
    let skipped = [
        root.join("crates/reprisebus/src/testbus.rs"),
        root.join(file!()),
    ];
    let mut found = Vec::new();
    for file in files.iter().filter(|f| !skipped.contains(f)) {
        let text = std::fs::read_to_string(file).unwrap();
        let Some(code) = test_code(file, &text) else {
            continue;
        };
        let first = text[..text.len() - code.len()].lines().count() + 1;
        for line in at_calls(code, first) {
            let file = file.strip_prefix(&root).unwrap_or(file);
            found.push(format!("{}:{line}", file.display()));
        }
    }
    assert!(
        found.is_empty(),
        "serve with reprisebus::testbus::serve, or Builder::serve_at: {found:#?}"
    );
    assert!(files.len() > 500, "the scan reads the workspace");
}

#[test]
fn the_scan_finds_chained_and_bound_calls_and_nothing_else() {
    let code = "\
        let tray = connect().await;\n\
        (tray.object_server().at(PATH, shell))\n\
        conn.object_server()\n    .at(PATH, x)\n\
        let server = bus.object_server();\n\
        server\n    .at(ROOT, y)\n\
        conn.object_server().interface::<_, X>(PATH)\n\
        // c.object_server().at(PATH, z)\n";
    assert_eq!(at_calls(code, 1), [2, 3, 6]);
}
