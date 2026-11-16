use super::{collect_pages, interfaces, write_pages, write_text_size};
use reprisekit::pins::{self, Cut};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

/// Draws every page with its themes at 200% text and lists each line whose rect cuts its ink, by
/// page and text, against `tests/pins/text_clip_200.txt`: a line that newly clips fails, and so
/// does one listed that no longer does. `REPRISE_BLESS` rewrites the list.
#[test]
fn every_page_at_200_percent_text_cuts_no_line_beyond_the_listed_ones() {
    pins::fonts();
    let cut: Rc<RefCell<Vec<(String, Cut)>>> = Rc::default();
    let into = cut.clone();
    collect_pages(Some(Box::new(move |_, page| {
        let mut found = into.borrow_mut();
        found.extend(
            pins::take_clipped()
                .into_iter()
                .map(|c| (page.to_string(), c)),
        );
    })));
    pins::clipped_ink(|| {
        interfaces::set_text_size(200);
        write_pages(true, &[None]);
        interfaces::set_text_size(100);
        write_text_size(200);
    });
    collect_pages(None);
    let mut drawn: BTreeMap<String, (u32, String)> = BTreeMap::new();
    for (page, c) in cut.take() {
        let page = page.trim_start_matches("reprisekit-gallery-");
        let key = format!("{page} {:?}", c.text);
        let r = c.rect;
        let at = format!(
            "{} px in {}x{} at ({}, {}): {} px cut above or below, {} beside",
            c.px, r.width, r.height, r.x, r.y, c.rows, c.cols
        );
        drawn.entry(key).or_insert((0, at)).0 += 1;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pins/text_clip_200.txt");
    let mut text = String::new();
    for (key, (n, _)) in &drawn {
        writeln!(text, "{key} x{n}").unwrap();
    }
    if std::env::var_os("REPRISE_BLESS").is_some_and(|v| v != "0") {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text).unwrap();
        }
        return;
    }
    let listed = std::fs::read_to_string(&path).unwrap_or_default();
    let mut msg = String::new();
    for (key, (n, at)) in &drawn {
        if !listed.lines().any(|l| *l == format!("{key} x{n}")) {
            writeln!(msg, "  cut {n} times, not listed: {key}, {at}").unwrap();
        }
    }
    for l in listed.lines() {
        if !text.lines().any(|t| t == l) {
            writeln!(msg, "  listed, no longer cut so: {l}").unwrap();
        }
    }
    assert!(
        msg.is_empty(),
        "lines whose rect cuts their ink at 200% text differ from {}:\n{msg}\
         REPRISE_BLESS=1 under `scripts/pin-env` rewrites it",
        path.display()
    );
}
