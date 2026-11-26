//! OSC 8 hyperlinks (`ESC ] 8 ; params ; URI ST`, an empty URI ending the link). alacritty keeps
//! each cell's link; this module reads the escape for the PTY filter, caps the links a tab keeps,
//! and says what a click on one opens.
//!
//! A link counts as opened when the filter passes its escape. Past [`LINKS_MAX`] opens the filter
//! turns them into ends, so their text shows unlinked; from [`SWEEP_AT`] a pump runs [`sweep`],
//! which keeps the newest [`KEEP`] links and counts again. A sweep reaches only the shown screen:
//! the other one takes no output while hidden, so its links stay as many as when it was left.

use std::collections::HashSet;
use std::ffi::OsString;
use std::sync::atomic::{AtomicUsize, Ordering};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;

use crate::cwd;

/// The longest URI a link keeps, in bytes; a longer one leaves its text unlinked.
pub const URI_MAX: usize = 2083;
/// The most links a tab holds between sweeps.
pub const LINKS_MAX: usize = 16 << 10;
/// The links opened since the last sweep that make the next pump sweep.
pub const SWEEP_AT: usize = LINKS_MAX / 2;
/// The newest links a sweep leaves on the screen and in the scrollback.
pub const KEEP: usize = LINKS_MAX / 4;

/// What an OSC does to the link the next cells take, read from its payload (after `ESC ]`,
/// without the terminator) as vte 0.15 reads it.
#[derive(Debug, PartialEq, Eq)]
pub enum Osc8 {
    /// Not an OSC 8, or one vte ignores (no second `;`).
    Other,
    Open,
    End,
    /// An open whose URI is past [`URI_MAX`].
    Long,
}

pub fn osc8(payload: &[u8]) -> Osc8 {
    let Some(rest) = payload.strip_prefix(b"8;") else {
        return Osc8::Other;
    };
    let Some(semi) = rest.iter().position(|&b| b == b';') else {
        return Osc8::Other;
    };
    match rest.len() - semi - 1 {
        0 => Osc8::End,
        n if n > URI_MAX => Osc8::Long,
        _ => Osc8::Open,
    }
}

/// What xdg-open is handed for a click on `uri`: an http, https or mailto URI as it is, a
/// `file://` one on this machine (`host`) as its path. None for any other scheme or host.
pub fn target(uri: &str, host: &str) -> Option<OsString> {
    let (scheme, rest) = uri.split_once(':')?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" if rest.len() > 2 && rest.starts_with("//") => Some(uri.into()),
        "mailto" if !rest.is_empty() => Some(uri.into()),
        "file" => cwd::osc7_path(&format!("file:{rest}"), host).map(Into::into),
        _ => None,
    }
}

/// Unlink every cell of `term`'s screen and scrollback past its newest [`KEEP`] links, newest
/// cell first, and take the links left off from `opened`.
pub fn sweep<T>(term: &mut Term<T>, opened: &AtomicUsize) {
    let counted = opened.load(Ordering::Relaxed);
    let grid = term.grid_mut();
    let (top, bottom, cols) = (
        grid.topmost_line().0,
        grid.bottommost_line().0,
        grid.columns(),
    );
    let mut kept = HashSet::new();
    for line in (top..=bottom).rev() {
        let row = &mut grid[Line(line)];
        for col in (0..cols).rev() {
            let cell = &mut row[Column(col)];
            let Some(link) = cell.hyperlink() else {
                continue;
            };
            // Each open allocates its URI, so the address tells two opens of one URI apart.
            let key = link.uri().as_ptr() as usize;
            if !kept.contains(&key) && (kept.len() >= KEEP || !kept.insert(key)) {
                cell.set_hyperlink(None);
            }
        }
    }
    let kept = kept.len();
    // Opens the filter counted during the sweep stay counted.
    match counted.checked_sub(kept) {
        Some(gone) => opened.fetch_sub(gone, Ordering::Relaxed),
        None => opened.fetch_add(kept - counted, Ordering::Relaxed),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escapes::Filter;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Point;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::cell::Hyperlink;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use std::sync::Arc;

    /// A `Term` of `cols` x `rows` that parsed `bytes` through a [`Filter`] counting into `opened`.
    fn term_with(
        bytes: &[u8],
        cols: usize,
        rows: usize,
        opened: &Arc<AtomicUsize>,
    ) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: 100,
            ..Config::default()
        };
        let mut t = Term::new(config, &TermSize::new(cols, rows), VoidListener);
        let mut filter = Filter::default();
        filter
            .opened()
            .store(opened.load(Ordering::Relaxed), Ordering::Relaxed);
        let mut out = Vec::new();
        filter.feed(bytes, &mut out);
        Processor::<StdSyncHandler>::new().advance(&mut t, &out);
        opened.store(filter.opened().load(Ordering::Relaxed), Ordering::Relaxed);
        t
    }

    fn term(bytes: &[u8]) -> Term<VoidListener> {
        term_with(bytes, 20, 4, &Arc::default())
    }

    fn link<T>(t: &Term<T>, line: i32, col: usize) -> Option<Hyperlink> {
        t.grid()[Point::new(Line(line), Column(col))].hyperlink()
    }

    fn uri<T>(t: &Term<T>, line: i32, col: usize) -> Option<String> {
        link(t, line, col).map(|l| l.uri().to_owned())
    }

    #[test]
    fn an_osc_8_reads_as_vte_reads_it() {
        assert_eq!(osc8(b"8;;https://x"), Osc8::Open);
        assert_eq!(osc8(b"8;id=a:b=c;https://x;y"), Osc8::Open);
        assert_eq!(osc8(b"8;;"), Osc8::End);
        assert_eq!(osc8(b"8;id=a;"), Osc8::End);
        assert_eq!(osc8(b"8;https://x"), Osc8::Other, "no second ;");
        assert_eq!(osc8(b"0;title"), Osc8::Other);
        assert_eq!(osc8(b"80;;x"), Osc8::Other);
        let mut long = b"8;;https://x/".to_vec();
        long.resize(3 + URI_MAX, b'a');
        assert_eq!(osc8(&long), Osc8::Open, "a URI of URI_MAX bytes");
        long.push(b'a');
        assert_eq!(osc8(&long), Osc8::Long);
    }

    #[test]
    fn a_link_ends_with_either_terminator() {
        for (open, end) in [("\x1b\\", "\x1b\\"), ("\x07", "\x07")] {
            let t = term(format!("a\x1b]8;;https://x{open}link\x1b]8;;{end}b").as_bytes());
            assert_eq!(uri(&t, 0, 0), None, "the text before");
            for col in 1..5 {
                assert_eq!(uri(&t, 0, col).as_deref(), Some("https://x"));
            }
            assert_eq!(uri(&t, 0, 5), None, "the text after");
        }
    }

    #[test]
    fn an_id_groups_the_cells_of_one_link_and_a_new_open_replaces_the_last() {
        let t = term(
            b"\x1b]8;id=a;https://x\x1b\\one\x1b]8;;\x1b\\ \x1b]8;id=a;https://x\x1b\\two\
              \x1b]8;;https://y\x1b\\three\x1b]8;;https://y\x1b\\four",
        );
        assert_eq!(link(&t, 0, 0), link(&t, 0, 4), "one id, one URI: one link");
        assert_eq!(uri(&t, 0, 3), None, "the gap between its parts");
        assert_eq!(uri(&t, 0, 7).as_deref(), Some("https://y"), "no nesting");
        assert_ne!(
            link(&t, 0, 7),
            link(&t, 0, 12),
            "two opens without an id are two links"
        );
    }

    #[test]
    fn a_link_past_the_uri_cap_leaves_its_text_unlinked() {
        let mut bytes = b"\x1b]8;;https://x\x1b\\a\x1b]8;;https://y/".to_vec();
        bytes.resize(bytes.len() + URI_MAX, b'z');
        bytes.extend(b"\x1b\\b\x1b]8;;\x1b\\");
        let t = term(&bytes);
        assert_eq!(uri(&t, 0, 0).as_deref(), Some("https://x"));
        assert_eq!(uri(&t, 0, 1), None, "the long open ends the link before");
    }

    #[test]
    fn copying_takes_the_text_and_not_the_escape() {
        use alacritty_terminal::index::Side;
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut t = term(b"see \x1b]8;;https://x\x1b\\here\x1b]8;;\x1b\\ now");
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        sel.update(Point::new(Line(0), Column(11)), Side::Right);
        t.selection = Some(sel);
        assert_eq!(t.selection_to_string().as_deref(), Some("see here now"));
    }

    /// The line of `t`'s screen or history that starts with `c`.
    fn line_of<T>(t: &Term<T>, c: char) -> i32 {
        (t.topmost_line().0..=t.bottommost_line().0)
            .find(|&l| t.grid()[Point::new(Line(l), Column(0))].c == c)
            .expect("the line is kept")
    }

    #[test]
    fn a_link_keeps_its_cells_through_scrolling_and_a_reflow() {
        let mut bytes = b"\x1b]8;;https://x\x1b\\0123456789abcdefghijKLMN\x1b]8;;\x1b\\".to_vec();
        bytes.extend(b"\r\n".repeat(6));
        let mut t = term(&bytes);
        let first = line_of(&t, '0');
        assert!(first < 0, "scrolled into the history");
        assert_eq!(uri(&t, first, 19).as_deref(), Some("https://x"));
        assert_eq!(
            uri(&t, first + 1, 3).as_deref(),
            Some("https://x"),
            "its wrapped part"
        );
        assert_eq!(uri(&t, first + 1, 4), None);
        t.resize(TermSize::new(30, 4));
        let first = line_of(&t, '0');
        let linked: Vec<usize> = (0..30).filter(|&c| link(&t, first, c).is_some()).collect();
        assert_eq!(linked, (0..24).collect::<Vec<_>>(), "rejoined on one line");
        assert_eq!(t.grid()[Point::new(Line(first), Column(23))].c, 'N');
    }

    #[test]
    fn opens_past_the_cap_show_unlinked() {
        let opened = Arc::new(AtomicUsize::new(LINKS_MAX - 1));
        let t = term_with(
            b"\x1b]8;;https://x\x1b\\a\x1b]8;;https://y\x1b\\b\x1b]8;;\x1b\\",
            20,
            4,
            &opened,
        );
        assert_eq!(uri(&t, 0, 0).as_deref(), Some("https://x"));
        assert_eq!(uri(&t, 0, 1), None, "the open past the cap ends the link");
        assert_eq!(opened.load(Ordering::Relaxed), LINKS_MAX);
    }

    #[test]
    fn a_sweep_keeps_the_newest_links_and_counts_them() {
        let mut bytes = Vec::new();
        for i in 0..KEEP + 3 {
            bytes.extend(format!("\x1b]8;;https://x/{i}\x1b\\a\x1b]8;;\x1b\\").as_bytes());
        }
        bytes.extend(b"\x1b]8;id=k;https://same\x1b\\b\x1b]8;id=k;https://same\x1b\\c");
        let opened = Arc::new(AtomicUsize::new(0));
        let cols = 64;
        let mut t = term_with(&bytes, cols, 4, &opened);
        assert_eq!(opened.load(Ordering::Relaxed), KEEP + 5);
        sweep(&mut t, &opened);
        assert_eq!(opened.load(Ordering::Relaxed), KEEP);
        let cells: Vec<Option<String>> = (t.topmost_line().0..=t.bottommost_line().0)
            .flat_map(|l| (0..cols).map(move |c| (l, c)))
            .map(|(l, c)| uri(&t, l, c))
            .collect();
        let linked: Vec<&String> = cells.iter().flatten().collect();
        assert_eq!(linked.len(), KEEP, "one cell per kept link");
        assert_eq!(linked.last().map(|s| s.as_str()), Some("https://same"));
        assert_eq!(
            linked.first().map(|s| s.as_str()),
            Some("https://x/5"),
            "the oldest ones lost their link"
        );
    }

    #[test]
    fn a_click_opens_web_mail_and_local_file_links_only() {
        let host = "box";
        let open = |u: &str| target(u, host).map(|t| t.to_string_lossy().into_owned());
        assert_eq!(
            open("https://example.org/a").as_deref(),
            Some("https://example.org/a")
        );
        assert_eq!(
            open("HTTP://example.org").as_deref(),
            Some("HTTP://example.org")
        );
        assert_eq!(open("mailto:a@b.c").as_deref(), Some("mailto:a@b.c"));
        assert_eq!(open("file://box/tmp/a%20b").as_deref(), Some("/tmp/a b"));
        assert_eq!(open("file:///tmp/a").as_deref(), Some("/tmp/a"));
        assert_eq!(open("file://localhost/tmp").as_deref(), Some("/tmp"));
        assert_eq!(open("file://other/tmp/a"), None, "a file on another host");
        for refused in [
            "ssh://box",
            "ftp://x/y",
            "javascript:x",
            "https:",
            "mailto:",
            "x",
        ] {
            assert_eq!(open(refused), None, "{refused}");
        }
    }
}
