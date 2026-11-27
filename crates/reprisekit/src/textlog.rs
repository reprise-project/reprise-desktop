//! The text each preview PNG drew, for `scripts/pseudo-locale`. With `REPRISE_TEXT_LOG=<dir>`
//! set, every line of text drawn and every text cut to an ellipsis collects on its thread, and
//! [`crate::raster::write_preview`] writes them to `<dir>/<program>-<pid>.tsv` under the PNG's
//! name; lines left when the thread ends go under the thread's name. Unset, nothing is kept.
//!
//! A row is `page \t kind \t text \t px`, `\`, tab and newline escaped in the text: kind `drawn`
//! with how far the line's advance reaches outside the rect it was laid in and how many ink
//! device pixels that rect cut beside the glyphs (`px` is `over,cut`), `ellipsized` with how
//! much wider than its room the whole text is, or `wrapped` with the lines a text broke into.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::{Mutex, OnceLock};

fn file() -> Option<&'static Mutex<std::fs::File>> {
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let dir = std::path::PathBuf::from(std::env::var_os("REPRISE_TEXT_LOG")?);
        let exe = std::env::current_exe().ok()?;
        let program = exe.file_stem()?.to_string_lossy().into_owned();
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{program}-{}.tsv", std::process::id()));
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Mutex::new(f))
    })
    .as_ref()
}

/// Whether drawn text is being kept; [`drawn`] and [`ellipsized`] are called only when it is.
pub(crate) fn on() -> bool {
    file().is_some()
}

struct Rows(BTreeSet<String>);

impl Drop for Rows {
    fn drop(&mut self) {
        let name = std::thread::current()
            .name()
            .unwrap_or("unnamed thread")
            .to_string();
        write(&name, &mut self.0);
    }
}

thread_local! {
    static ROWS: RefCell<Rows> = const { RefCell::new(Rows(BTreeSet::new())) };
}

fn keep(kind: &str, text: &str, px: String) {
    let text = text
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n");
    let _ = ROWS.try_with(|r| r.borrow_mut().0.insert(format!("{kind}\t{text}\t{px}")));
}

/// A line drawn reaching `over` logical px outside its rect, `cut` ink device px cut beside it.
pub(crate) fn drawn(text: &str, over: i32, cut: u32) {
    keep("drawn", text, format!("{over},{cut}"));
}

/// A text cut to an ellipsis, whole `over` logical px wider than its room.
pub(crate) fn ellipsized(text: &str, over: i32) {
    keep("ellipsized", text, over.to_string());
}

/// A text wrapped into `lines`, so a line drawn without its message's end is known as one.
pub(crate) fn wrapped(lines: &[String]) {
    keep("wrapped", &lines.join("\n"), String::new());
}

/// Writes the rows this thread kept since the last page under `page`.
pub(crate) fn page(page: &str) {
    if on() {
        let _ = ROWS.try_with(|r| write(page, &mut r.borrow_mut().0));
    }
}

fn write(page: &str, rows: &mut BTreeSet<String>) {
    let Some(file) = file().filter(|_| !rows.is_empty()) else {
        return;
    };
    let mut out = String::new();
    for row in std::mem::take(rows) {
        out += &format!("{page}\t{row}\n");
    }
    let _ = crate::mutex::lock(file).write_all(out.as_bytes());
}
