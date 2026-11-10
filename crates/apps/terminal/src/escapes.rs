//! The PTY's output on its way to vte 0.15's parser, whose states it follows as far as needed:
//! overlong OSCs and title pushes past [`DEPTH_MAX`] are dropped, OSC 7 is picked out.

/// The longest OSC payload passed on, in bytes; a longer one is dropped whole.
pub const OSC_MAX: usize = 64 << 10;
/// The longest OSC 52 passed on: a [`STORE_MAX`](crate::tab::STORE_MAX) store in base64 and its
/// selector. Past [`OSC_MAX`] the rest streams to the parser and no longer decodes.
pub const OSC52_MAX: usize = crate::tab::STORE_MAX.div_ceil(3) * 4 + 64;
/// The deepest the title stack grows (CSI 22 t pushes, CSI 23 t pops); a push past it is dropped.
pub const DEPTH_MAX: usize = 16;
/// The longest CSI held while it may still be a push past [`DEPTH_MAX`]; a longer one is dropped.
const CSI_HELD_MAX: usize = 64;
/// vte keeps at most this many CSI parameters; a CSI with more is not carried out.
const CSI_PARAMS: usize = 32;

const ESC: u8 = 0x1b;
/// What a dropped escape is replaced with: its ESC ends any string the parser is in (DCS, APC)
/// as the dropped one would have, and ST does nothing else.
const ST: &[u8] = b"\x1b\\";

#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq))]
enum State {
    #[default]
    Ground,
    /// After an ESC, which is held.
    Esc,
    /// An OSC, held whole until it ends.
    Osc,
    /// An OSC 52 past [`OSC_MAX`], passed on as it comes up to [`OSC52_MAX`] and its end; its
    /// payload bytes so far.
    Osc52(usize),
    /// An OSC past its cap, replaced by [`ST`] and dropped up to its end.
    OscDrop,
    Csi(Csi),
    /// A CSI held past [`CSI_HELD_MAX`], replaced by [`ST`] and dropped up to its end.
    CsiDrop,
}

/// What a CSI so far tells about whether vte carries it out as CSI 22 t or CSI 23 t.
#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct Csi {
    /// The first parameter, saturating like vte's.
    first: u16,
    /// Parameter separators (`;` and `:`) so far.
    seps: usize,
    /// An intermediate or private marker came, so vte does not read it as CSI t.
    marked: bool,
    /// Held while it may still be a push past [`DEPTH_MAX`].
    held: bool,
}

impl Csi {
    fn may_push(&self) -> bool {
        !self.marked
            && self.seps < CSI_PARAMS
            && self.first <= 22
            && (self.seps == 0 || self.first == 22)
    }

    /// Take a parameter, intermediate or private-marker byte (0x20 to 0x3f).
    fn param(&mut self, b: u8) {
        match b {
            b'0'..=b'9' if self.seps == 0 => {
                self.first = self
                    .first
                    .saturating_mul(10)
                    .saturating_add((b - b'0').into())
            }
            b':' | b';' => self.seps += 1,
            b' '..=b'/' | b'<'..=b'?' => self.marked = true,
            _ => {}
        }
    }

    /// The title stack change vte makes for final byte `b`: +1, -1 or 0.
    fn stack_change(&self, b: u8) -> i8 {
        if b != b't' || self.marked || self.seps >= CSI_PARAMS {
            return 0;
        }
        match self.first {
            22 => 1,
            23 => -1,
            _ => 0,
        }
    }
}

/// The filter one tab's PTY output passes through.
#[derive(Default)]
pub struct Filter {
    state: State,
    /// An escape held until it is known whole: an ESC, an OSC, or a CSI that may be a push.
    held: Vec<u8>,
    /// The title stack depth as vte keeps it, counted from the pushes and pops passed on.
    depth: usize,
    /// Takes every escape byte through [`Self::byte`], for tests that check the runs against it.
    #[cfg(test)]
    byte_path: bool,
}

impl Filter {
    /// Pass `input` on into `out`, which grows by at most `input.len()` and the bytes held
    /// before. Returns the URI of the last OSC 7 that `input` ends.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Option<String> {
        let mut found = None;
        let mut i = 0;
        while i < input.len() {
            match self.state {
                State::Ground => {
                    let run = self.pass(&input[i..]);
                    out.extend_from_slice(&input[i..i + run]);
                    i += run;
                }
                State::Osc if self.runs() => {
                    let room = (2 + OSC_MAX - self.held.len()).min(input.len() - i);
                    let run = input[i..i + room]
                        .iter()
                        .position(|&b| b < 0x20)
                        .unwrap_or(room);
                    self.held.extend_from_slice(&input[i..i + run]);
                    i += run;
                }
                _ => {}
            }
            if i == input.len() {
                break;
            }
            if let Some(uri) = self.byte(input[i], out) {
                found = Some(uri);
            }
            i += 1;
        }
        found
    }

    /// How much of `input`, from Ground, [`Self::byte`] passes on unchanged: text, whole CSIs and
    /// two-byte escapes while the stack has room. Counts their stack changes; stops at other ESCs.
    fn pass(&mut self, input: &[u8]) -> usize {
        let mut i = 0;
        loop {
            i += input[i..]
                .iter()
                .position(|&b| b == ESC)
                .unwrap_or(input.len() - i);
            if !self.runs() {
                return i;
            }
            match input.get(i + 1) {
                Some(b'[') if self.depth < DEPTH_MAX => {
                    let mut csi = Csi::default();
                    let Some(len) = input[i + 2..].iter().position(|&b| {
                        let param = matches!(b, 0x20..=0x3f);
                        if param {
                            csi.param(b);
                        }
                        !param
                    }) else {
                        return i;
                    };
                    let end = input[i + 2 + len];
                    if !matches!(end, 0x40..=0x7e) {
                        return i;
                    }
                    let change = csi.stack_change(end);
                    self.depth = self.depth.saturating_add_signed(change.into());
                    i += 3 + len;
                }
                Some(&b) if matches!(b, 0x20..=0x7e) && b != b'[' && b != b']' => {
                    if b == b'c' {
                        self.depth = 0;
                    }
                    i += 2;
                }
                _ => return i,
            }
        }
    }

    /// Whether [`Self::feed`] passes whole escapes and OSC payload runs at once.
    fn runs(&self) -> bool {
        #[cfg(test)]
        {
            !self.byte_path
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    fn byte(&mut self, b: u8, out: &mut Vec<u8>) -> Option<String> {
        match &mut self.state {
            State::Ground => self.escape(out),
            State::Esc => match b {
                ESC => out.push(ESC),
                b']' => {
                    self.held.push(b);
                    self.state = State::Osc;
                }
                b'[' => {
                    self.held.push(b);
                    let held = self.depth >= DEPTH_MAX;
                    if !held {
                        out.append(&mut self.held);
                    }
                    self.state = State::Csi(Csi {
                        held,
                        ..Csi::default()
                    });
                }
                // vte runs C0 controls without leaving its escape state, so they may go first.
                _ if runs_inside(b) => out.push(b),
                0x7f..=0xff => {}
                _ => {
                    if b == b'c' {
                        self.depth = 0;
                    }
                    out.append(&mut self.held);
                    out.push(b);
                    self.state = State::Ground;
                }
            },
            State::Osc => match b {
                0x07 | 0x18 | 0x1a | ESC => {
                    let uri = self.held[2..]
                        .strip_prefix(b"7;")
                        .map(|u| String::from_utf8_lossy(u).into_owned());
                    out.append(&mut self.held);
                    self.end_osc(b, out);
                    return uri;
                }
                _ if osc_skips(b) => {}
                _ if self.held.len() - 2 < OSC_MAX => self.held.push(b),
                _ if self.held[2..].starts_with(b"52;") => {
                    out.append(&mut self.held);
                    out.push(b);
                    self.state = State::Osc52(OSC_MAX + 1);
                }
                _ => {
                    self.held.clear();
                    out.extend_from_slice(ST);
                    self.state = State::OscDrop;
                }
            },
            State::Osc52(n) => match b {
                0x07 | 0x18 | 0x1a | ESC => self.end_osc(b, out),
                _ if osc_skips(b) => {}
                _ if *n < OSC52_MAX => {
                    *n += 1;
                    out.push(b);
                }
                _ if *n == OSC52_MAX => {
                    *n += 1;
                    out.push(0xff);
                }
                _ => {}
            },
            State::OscDrop => match b {
                0x07 => self.state = State::Ground,
                0x18 | 0x1a | ESC => self.end_osc(b, out),
                _ => {}
            },
            State::Csi(csi) => match b {
                ESC => {
                    self.held.clear();
                    self.escape(out);
                }
                0x18 | 0x1a => {
                    self.held.clear();
                    out.push(b);
                    self.state = State::Ground;
                }
                _ if runs_inside(b) => out.push(b),
                0x7f..=0xff => {}
                0x40..=0x7e => {
                    let change = csi.stack_change(b);
                    if change > 0 && self.depth >= DEPTH_MAX {
                        self.held.clear();
                        out.extend_from_slice(ST);
                    } else {
                        self.depth = self.depth.saturating_add_signed(change.into());
                        out.append(&mut self.held);
                        out.push(b);
                    }
                    self.state = State::Ground;
                }
                _ => {
                    csi.param(b);
                    if !csi.held {
                        out.push(b);
                    } else if !csi.may_push() {
                        csi.held = false;
                        out.append(&mut self.held);
                        out.push(b);
                    } else if self.held.len() < CSI_HELD_MAX {
                        self.held.push(b);
                    } else {
                        self.held.clear();
                        out.extend_from_slice(ST);
                        self.state = State::CsiDrop;
                    }
                }
            },
            State::CsiDrop => match b {
                ESC => self.escape(out),
                0x18 | 0x1a => {
                    out.push(b);
                    self.state = State::Ground;
                }
                _ if runs_inside(b) => out.push(b),
                0x40..=0x7e => self.state = State::Ground,
                _ => {}
            },
        }
        None
    }

    /// Hold an ESC.
    fn escape(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.held);
        self.held.push(ESC);
        self.state = State::Esc;
    }

    /// End an OSC with `b`: BEL is its terminator, CAN and SUB are also run, ESC starts the next
    /// escape.
    fn end_osc(&mut self, b: u8, out: &mut Vec<u8>) {
        match b {
            ESC => self.escape(out),
            _ => {
                out.push(b);
                self.state = State::Ground;
            }
        }
    }
}

/// A C0 control vte runs inside an escape or CSI without leaving it: all but CAN, SUB and ESC.
fn runs_inside(b: u8) -> bool {
    matches!(b, 0x00..=0x17 | 0x19 | 0x1c..=0x1f)
}

/// A C0 control vte skips inside an OSC.
fn osc_skips(b: u8) -> bool {
    runs_inside(b) && b != 0x07
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::{Event, EventListener};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;
    use std::sync::{Arc, Mutex};

    fn filtered(chunks: &[&[u8]]) -> Vec<u8> {
        let mut f = Filter::default();
        let mut out = Vec::new();
        for c in chunks {
            f.feed(c, &mut out);
        }
        out
    }

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<Event>>>);
    impl EventListener for Sink {
        fn send_event(&self, ev: Event) {
            self.0.lock().unwrap().push(ev);
        }
    }

    /// A `Term` that parsed `bytes` through a [`Filter`], and the events it sent.
    fn parsed(bytes: &[u8]) -> (Term<Sink>, Vec<Event>) {
        let sink = Sink::default();
        let mut term = Term::new(Config::default(), &TermSize::new(80, 24), sink.clone());
        let mut parser: Processor = Processor::new();
        let mut filter = Filter::default();
        for chunk in bytes.chunks(4096) {
            let mut out = Vec::new();
            filter.feed(chunk, &mut out);
            parser.advance(&mut term, &out);
        }
        let events = std::mem::take(&mut *sink.0.lock().unwrap());
        (term, events)
    }

    fn events(bytes: &[u8]) -> Vec<Event> {
        parsed(bytes).1
    }

    /// The text on the first line after `bytes`.
    fn first_line(bytes: &[u8]) -> String {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        let (term, _) = parsed(bytes);
        let row = &term.grid()[Line(0)];
        let text: String = (0..term.columns()).map(|c| row[Column(c)].c).collect();
        text.trim_end().to_owned()
    }

    fn titles(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Title(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plain_output_and_short_escapes_pass_unchanged_across_reads() {
        let all: &[u8] = "ls\r\n\x1b[1;31mred\x1b[0m \x1b]0;a title\x07\x1b]8;;http://x\x1b\\link\
             \x1b[22t\x1b[23t é€😀 \x1bP1$qm\x1b\\\x1b_apc\x1b\\\x1b(B\x1b[1\x18\x1b]0;x\x1a\
             \x1b]2;y\x1b[0m\x07"
            .as_bytes();
        let all: &[u8] = &[all, &b"\x9d0;c1\x9c\x7f\xff"[..]].concat();
        for cut in 0..all.len() {
            assert_eq!(filtered(&[&all[..cut], &all[cut..]]), all, "split at {cut}");
        }
    }

    #[test]
    fn an_osc_past_the_cap_is_dropped_whole() {
        let mut long = b"\x1b]0;".to_vec();
        long.resize(4 + OSC_MAX + 1, b'x');
        long.extend_from_slice(b"\x07after");
        assert_eq!(filtered(&[&long]), [ST, b"after"].concat());
        assert!(titles(&events(&long)).is_empty(), "no title is set");
    }

    #[test]
    fn an_osc_at_the_cap_passes() {
        let mut long = b"\x1b]2;".to_vec();
        long.resize(4 + OSC_MAX - 2, b'x');
        long.extend_from_slice(b"\x1b\\");
        assert_eq!(titles(&events(&long)), ["x".repeat(OSC_MAX - 2)]);
    }

    #[test]
    fn a_control_between_esc_and_bracket_still_opens_a_capped_osc() {
        let mut long = b"\x1b\x01]0;".to_vec();
        long.resize(5 + OSC_MAX + 1, b'x');
        long.push(0x07);
        assert_eq!(filtered(&[&long]), [&b"\x01"[..], ST].concat());
    }

    #[test]
    fn an_unended_osc_holds_no_more_than_the_cap() {
        let mut f = Filter::default();
        let mut out = Vec::new();
        f.feed(b"\x1b]0;", &mut out);
        for _ in 0..80 {
            f.feed(&[b'x'; 64 << 10], &mut out);
        }
        assert_eq!(out, ST);
        assert!(f.held.len() <= 2 + OSC_MAX, "held {}", f.held.len());
    }

    #[test]
    fn an_osc_52_past_its_cap_stores_nothing() {
        let mut long = b"\x1b]52;c;".to_vec();
        long.resize(7 + (OSC52_MAX + 1024) / 4 * 4, b'A');
        long.push(0x07);
        let ev = events(&long);
        assert!(
            !ev.iter().any(|e| matches!(e, Event::ClipboardStore(..))),
            "the store is dropped"
        );
    }

    #[test]
    fn an_osc_52_past_the_osc_cap_but_within_its_own_stores() {
        let text = "y".repeat(99_999);
        let seq = format!("\x1b]52;c;{}\x07", "eXl5".repeat(33_333));
        let ev = events(seq.as_bytes());
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::ClipboardStore(_, t) if *t == text)),
            "a 100 KB store arrives whole"
        );
    }

    #[test]
    fn a_capped_osc_ends_where_the_parser_ends_it() {
        for head in [&b"\x1b]0;"[..], b"\x1b]52;c;"] {
            for end in [&b"\x07"[..], b"\x18", b"\x1b\\"] {
                let mut seq = head.to_vec();
                seq.resize(head.len() + OSC52_MAX + 1024, b'A');
                seq.extend_from_slice(end);
                seq.extend_from_slice(b"after");
                let what = format!("{} ended by {}", head.escape_ascii(), end.escape_ascii());
                assert_eq!(first_line(&seq), "after", "{what}");
                assert!(
                    !events(&seq)
                        .iter()
                        .any(|e| matches!(e, Event::Title(..) | Event::ClipboardStore(..))),
                    "{what} sets nothing"
                );
            }
        }
    }

    #[test]
    fn a_dropped_escape_still_ends_the_string_before_it() {
        let mut long = b"\x1b]0;".to_vec();
        long.resize(4 + OSC_MAX + 1, b'x');
        long.push(0x07);
        let pushes = "\x1b[22t".repeat(DEPTH_MAX);
        for before in [&b"\x1bPq"[..], b"\x1b_apc", b"\x1b(", b"\x1b"] {
            let what = before.escape_ascii();
            let osc = [before, &long[..], &b"after"[..]].concat();
            assert_eq!(first_line(&osc), "after", "{what} then a long OSC");
            let push = [pushes.as_bytes(), before, &b"\x1b[22tafter"[..]].concat();
            assert_eq!(
                first_line(&push),
                "after",
                "{what} then a push past the cap"
            );
        }
    }

    #[test]
    fn title_pushes_past_the_depth_cap_are_dropped() {
        let mut seq = String::new();
        for i in 1..=40 {
            seq += &format!("\x1b]0;t{i}\x07\x1b[22t");
        }
        seq += &"\x1b[23t".repeat(DEPTH_MAX + 1);
        let shown = titles(&events(seq.as_bytes()));
        assert_eq!(
            shown.last().map(String::as_str),
            Some("t1"),
            "the stack held t1 to t16 and popped down to t1"
        );
    }

    #[test]
    fn a_pop_or_a_reset_makes_room_for_a_push() {
        let mut f = Filter::default();
        let mut out = Vec::new();
        let push: &[u8] = b"\x1b[22;0t";
        for _ in 0..DEPTH_MAX {
            f.feed(push, &mut out);
        }
        out.clear();
        f.feed(push, &mut out);
        assert_eq!(out, ST, "the stack is full");
        f.feed(b"\x1b[23t", &mut out);
        out.clear();
        f.feed(push, &mut out);
        assert_eq!(out, push, "a pop made room");
        f.feed(b"\x1bc", &mut out);
        out.clear();
        for _ in 0..DEPTH_MAX {
            f.feed(push, &mut out);
        }
        assert_eq!(
            out.len(),
            DEPTH_MAX * push.len(),
            "a reset emptied the stack"
        );
    }

    #[test]
    fn at_the_depth_cap_other_csis_still_pass() {
        let mut f = Filter::default();
        let mut out = Vec::new();
        for _ in 0..DEPTH_MAX {
            f.feed(b"\x1b[22t", &mut out);
        }
        out.clear();
        let others: &[u8] = b"\x1b[2J\x1b[?22t\x1b[220t\x1b[2;2H\x1b[0022";
        f.feed(others, &mut out);
        f.feed(b"t", &mut out);
        assert_eq!(
            out,
            [&others[..others.len() - 6], ST].concat(),
            "the zero-padded push is dropped"
        );
    }

    #[test]
    fn osc7_found_across_chunks_with_either_terminator() {
        let mut f = Filter::default();
        let mut out = Vec::new();
        assert_eq!(
            f.feed(b"hi \x1b]0;title\x07 \x1b]7;file://h", &mut out),
            None
        );
        assert_eq!(
            f.feed(b"/x\x1b", &mut out),
            Some("file://h/x".into()),
            "an ESC ends an OSC, as in vte"
        );
        assert_eq!(f.feed(b"\\ more", &mut out), None);
        assert_eq!(
            f.feed(b"\x1b]7;file:///a\x07\x1b]7;file:///b\x07", &mut out),
            Some("file:///b".into())
        );
        assert_eq!(
            f.feed(b"\x1b[31m\x1b]70;x\x07", &mut out),
            None,
            "OSC 70 is not OSC 7"
        );
    }

    /// xorshift64*, so a failing case replays from its printed seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Bytes built from the pieces the filter's states turn on, runs past the caps included.
    fn noise(rng: &mut Rng, pieces: usize) -> Vec<u8> {
        const TOKENS: &[&[u8]] = &[
            b"\x1b",
            b"[",
            b"]",
            b"\x1b[",
            b"\x1b]",
            b"\x1b[22t",
            b"\x1b[23t",
            b"\x1b[22;0t",
            b"\x1b[0022t",
            b"\x1b[?22t",
            b"\x1b[220t",
            b"\x1b[22 t",
            b"\x1b[22:1t",
            b"\x1bc",
            b"\x1b(B",
            b"\x1b(c",
            b"\x1bP",
            b"\x1b_",
            b"\x1b\\",
            b"\x1b]7;file:///a",
            b"\x1b]0;",
            b"\x1b]52;c;",
            b"52;",
            b"7;",
            b"0",
            b"1",
            b"2",
            b"22",
            b"23",
            b"9",
            b";",
            b":",
            b"t",
            b"m",
            b"H",
            b"c",
            b"@",
            b"~",
            b"\x07",
            b"\x18",
            b"\x1a",
            b"\x00",
            b"\x01",
            b"\x0a",
            b"\x1c",
            b"\x1f",
            b"\x7f",
            b"\x80",
            b"\x9b",
            b"\x9c",
            b"\x9d",
            b"\xff",
            b" ",
            b"<",
            b"?",
            b"!",
            b"x",
            b"A",
            b"\x1b[0m",
            b"\x1b[01;34m",
            b"\x1b[38;5;244m",
            b"\x1b[12;60H",
            b"\xc3\xa9",
            b"\xc3",
            b"\xa9",
        ];
        let mut out = Vec::new();
        for _ in 0..pieces {
            match rng.below(40) {
                0 => {
                    let n = [100, 4096, OSC_MAX - 4, 70_000][rng.below(4)] + rng.below(8);
                    out.resize(out.len() + n, b"xA2;"[rng.below(4)]);
                }
                1 => out.extend(std::iter::repeat_n(b"\x1b[22t", rng.below(20)).flatten()),
                2 => {
                    let n = [CSI_HELD_MAX, CSI_PARAMS][rng.below(2)] - 4 + rng.below(8);
                    out.extend_from_slice(b"\x1b[");
                    out.resize(out.len() + n, b"0;"[rng.below(2)]);
                }
                _ => out.extend_from_slice(TOKENS[rng.below(TOKENS.len())]),
            }
        }
        out
    }

    /// Feeds `input` in reads of the lengths `reads` gives to a filter taking runs and to one
    /// taking every escape byte by byte; returns where their output, OSC 7 or state first differs.
    fn diverges(input: &[u8], mut reads: impl Iterator<Item = usize>) -> Option<String> {
        let mut runs = Filter::default();
        let mut bytes = Filter {
            byte_path: true,
            ..Filter::default()
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut i = 0;
        while i < input.len() {
            let n = reads.next().unwrap_or(usize::MAX).min(input.len() - i);
            let read = &input[i..i + n];
            let (ua, ub) = (runs.feed(read, &mut a), bytes.feed(read, &mut b));
            if ua != ub {
                return Some(format!(
                    "OSC 7 after byte {i}+{n}: {ua:?}, byte path {ub:?}"
                ));
            }
            if a != b {
                return Some(format!("output after byte {i}+{n}"));
            }
            if (&runs.state, runs.depth) != (&bytes.state, bytes.depth) || runs.held != bytes.held {
                return Some(format!(
                    "after byte {i}+{n}: {:?} depth {} held {}, byte path {:?} depth {} held {}",
                    runs.state,
                    runs.depth,
                    runs.held.len(),
                    bytes.state,
                    bytes.depth,
                    bytes.held.len()
                ));
            }
            a.clear();
            b.clear();
            i += n;
        }
        None
    }

    #[test]
    fn runs_pass_on_what_the_byte_path_does_across_random_reads() {
        let mut seeds = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..3000 {
            let seed = seeds.next() | 1;
            let mut rng = Rng(seed);
            let pieces = 1 + rng.below(400);
            let input = noise(&mut rng, pieces);
            let reads = std::iter::from_fn(|| {
                let max = [1, 7, 64, 4096, 70_000][rng.below(5)];
                Some(1 + rng.below(max))
            });
            if let Some(at) = diverges(&input, reads) {
                panic!("case {case} seed {seed:#x}: {at}");
            }
        }
    }

    #[test]
    fn runs_pass_on_what_the_byte_path_does_with_a_read_cut_at_the_osc_cap() {
        let cap = 2 + OSC_MAX;
        for head in [&b"\x1b]0;"[..], b"\x1b]7;", b"\x1b]52;c;"] {
            for len in cap - 1..=cap + 1 {
                let mut input = head.to_vec();
                input.resize(len, b'x');
                input.extend_from_slice(b"\x07after");
                for cut in cap - 3..=cap + 3 {
                    if let Some(at) = diverges(&input, [cut].into_iter()) {
                        panic!("{} of {len} cut at {cut}: {at}", head.escape_ascii());
                    }
                }
            }
        }
    }
}
