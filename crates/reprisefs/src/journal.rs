//! A unit's last journal lines as Scheduled Tasks' View Output and Task Manager's Open Log show
//! them: each entry as its time in the file lists' form and its message.

/// The `journalctl` options that write what [`lines`] reads: one JSON object an entry, holding its
/// message and time. Without `--all` the JSON output writes a field over 4096 bytes as null.
pub const JSON_ARGS: [&str; 3] = ["--all", "--output=json", "--output-fields=MESSAGE"];

/// The journal's entries, one JSON object a line, as `time message` lines: the time as
/// [`crate::listing::date_text`] writes one, without the `name[pid]:` the journal puts before a
/// message. An entry with no text message is left out.
pub fn lines(json: &str, is_12h: bool) -> String {
    use serde_json::Value;
    let line = |l: &str| {
        let e: Value = serde_json::from_str(l).ok()?;
        let message = match e.get("MESSAGE")? {
            Value::String(s) => s.clone(),
            // The journal writes a message that is not UTF-8 as its bytes.
            Value::Array(bytes) => {
                let bytes: Vec<u8> = bytes
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|b| b as u8)
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            }
            _ => return None,
        };
        let us: u64 = e.get("__REALTIME_TIMESTAMP")?.as_str()?.parse().ok()?;
        let at = crate::listing::date_text((us / 1_000_000) as i64, is_12h);
        Some(format!("{at} {}", message.trim_end()))
    };
    json.lines().filter_map(line).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_its_time_as_the_list_writes_it_and_its_message() {
        let at = 1_764_219_292_000_000u64;
        let when = crate::listing::date_text((at / 1_000_000) as i64, false);
        let json = [
            format!(r#"{{"__REALTIME_TIMESTAMP":"{at}","MESSAGE":"Starting hg test..."}}"#),
            format!(r#"{{"__REALTIME_TIMESTAMP":"{at}","MESSAGE":[104,105,255]}}"#),
            format!(r#"{{"__REALTIME_TIMESTAMP":"{at}","MESSAGE":null}}"#),
            format!(r#"{{"__REALTIME_TIMESTAMP":"{at}","MESSAGE":"sent 4 files\n"}}"#),
            "not json".to_string(),
        ]
        .join("\n");
        assert_eq!(
            lines(&json, false),
            format!("{when} Starting hg test...\n{when} hi\u{fffd}\n{when} sent 4 files")
        );
    }
}
