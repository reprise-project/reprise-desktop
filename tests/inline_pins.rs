//! An image hash belongs in a `tests/pins/*.txt` file that `REPRISE_BLESS=1` rewrites, so a
//! drawing change is blessed in one run. A source that hashes pixels with FNV-1a writes no other
//! 64-bit literal: a hash written in the test instead would have to be edited by hand.

use std::process::Command;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Each integer literal in `code` with its value, skipping those too large for a u64.
fn literals(code: &str) -> Vec<(&str, u64)> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        if !bytes[i].is_ascii_digit() || (i > 0 && (word(bytes[i - 1]) || bytes[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && word(bytes[i]) {
            i += 1;
        }
        let token = &code[start..i];
        let digits = token.replace('_', "");
        // A type suffix starts with `u` or `i`, neither a hex digit.
        let digits = digits.split(['u', 'i']).next().unwrap_or_default();
        let value = match digits.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16),
            None => digits.parse(),
        };
        if let Ok(v) = value {
            out.push((token, v));
        }
    }
    out
}

#[test]
fn no_source_that_hashes_pixels_writes_a_hash_inline() {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new("git")
        .args(["-C", root, "ls-files", "-z", "--", "*.rs"])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git ls-files succeeds");
    let mut hits = Vec::new();
    for path in out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(path).expect("UTF-8 path");
        let Ok(text) = std::fs::read_to_string(format!("{root}/{path}")) else {
            continue;
        };
        let hashes = text.lines().any(|l| {
            let l = l.split("//").next().unwrap_or_default();
            l.contains("fnv") || literals(l).iter().any(|&(_, v)| v == FNV_OFFSET)
        });
        if !hashes {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            for (token, v) in literals(code) {
                if v >= 1 << 52 && v != FNV_OFFSET {
                    hits.push(format!("{path}:{}: {token}", n + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "hashes go in tests/pins via reprisekit::pins::check_file, not inline:\n{}",
        hits.join("\n")
    );
}
