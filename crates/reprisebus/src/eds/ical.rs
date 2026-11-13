//! One local day's events in the iCalendar objects a calendar picked for it. A recurring event
//! comes as its master: its occurrence takes the master's wall time on the day, in its own zone.

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

/// One event on the day, its times in seconds since the epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub summary: String,
    /// Dated by day, not by time: from local midnight to local midnight.
    pub all_day: bool,
    pub start: i64,
    pub end: i64,
}

/// The local day `date` in `local`: its first second and the next day's.
pub fn day_bounds(date: Date, local: &TimeZone) -> Option<(i64, i64)> {
    let start = date.to_zoned(local.clone()).ok()?.timestamp().as_second();
    let end = (date.tomorrow().ok()?.to_zoned(local.clone()).ok()?)
        .timestamp()
        .as_second();
    Some((start, end))
}

/// The occurrences on local day `date` in `objects`, in their order.
pub fn occurrences(objects: &[String], date: Date, local: &TimeZone) -> Vec<Occurrence> {
    let Some(day) = day_bounds(date, local) else {
        return Vec::new();
    };
    let mut zones = Zones::new(local);
    let events: Vec<Event> = (objects.iter())
        .flat_map(|o| parse(o))
        .filter(|e| !e.cancelled)
        .collect();
    // A detached instance stands in for the occurrence its RECURRENCE-ID names.
    let mut moved: HashMap<&str, Vec<i64>> = HashMap::new();
    for e in &events {
        if let Some(rid) = e.recurrence_id.as_ref().and_then(|t| zones.instant(t)) {
            moved.entry(e.uid.as_str()).or_default().push(rid);
        }
    }
    let mut out = Vec::new();
    for e in &events {
        let Some(start) = e.start.as_ref() else {
            continue;
        };
        let all_day = start.value.is_date();
        let (first, last) = match (zones.instant(start), e.end.as_ref(), e.duration) {
            (Some(s), Some(end), _) => (s, zones.instant(end).unwrap_or(s)),
            (Some(s), None, Some(d)) => (s, s + d),
            (Some(s), None, None) if all_day => (s, zones.after_days(start, 1).unwrap_or(s)),
            (Some(s), None, None) => (s, s),
            (None, ..) => continue,
        };
        let length = (last - first).max(0);
        let at = if e.recurring && e.recurrence_id.is_none() {
            let skip: Vec<i64> = (e.exdates.iter())
                .filter_map(|t| zones.instant(t))
                .chain(moved.get(e.uid.as_str()).into_iter().flatten().copied())
                .collect();
            recurrence_on(&mut zones, start, length, day, &skip)
        } else {
            overlaps(first, length, day).then_some(first)
        };
        if let Some(s) = at {
            out.push(Occurrence {
                summary: e.summary.clone(),
                all_day,
                start: s,
                end: s + length,
            });
        }
    }
    out
}

fn overlaps(start: i64, length: i64, (from, to): (i64, i64)) -> bool {
    if length == 0 {
        (from..to).contains(&start)
    } else {
        start < to && start + length > from
    }
}

/// A recurring event's start on `day`: `start`'s wall time on the day's date, else the date before
/// or after, whichever overlaps the day, the one starting on it first; `skip` holds left-out starts.
fn recurrence_on(
    zones: &mut Zones,
    start: &Time,
    length: i64,
    day: (i64, i64),
    skip: &[i64],
) -> Option<i64> {
    let zone = zones.of(start);
    let here = Timestamp::from_second(day.0)
        .ok()?
        .to_zoned(zone.clone())
        .date();
    let dates = [Some(here), here.yesterday().ok(), here.tomorrow().ok()];
    let starts: Vec<i64> = (dates.into_iter().flatten())
        .filter_map(|d| {
            let s = match start.value {
                Value::Date(_) => d.to_zoned(zone.clone()).ok()?.timestamp().as_second(),
                Value::DateTime(dt) => (zone.to_ambiguous_zoned(d.to_datetime(dt.time())))
                    .compatible()
                    .ok()?
                    .timestamp()
                    .as_second(),
            };
            (overlaps(s, length, day) && !skip.contains(&s)).then_some(s)
        })
        .collect();
    let on_day = starts.iter().filter(|&&s| s >= day.0).min();
    on_day.or_else(|| starts.iter().max()).copied()
}

/// A DATE or DATE-TIME property: its value and the zone it is written in.
#[derive(Clone, Debug, PartialEq)]
struct Time {
    value: Value,
    zone: Zone,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Date(Date),
    DateTime(DateTime),
}

impl Value {
    fn is_date(self) -> bool {
        matches!(self, Value::Date(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Zone {
    Utc,
    /// A TZID parameter.
    Named(String),
    /// Floating: the same wall time everywhere, read in the local zone. Dates are floating too.
    Local,
}

/// Zones by TZID, each looked up once; an unknown one reads as local.
struct Zones {
    local: TimeZone,
    named: HashMap<String, TimeZone>,
}

impl Zones {
    fn new(local: &TimeZone) -> Zones {
        Zones {
            local: local.clone(),
            named: HashMap::new(),
        }
    }

    fn of(&mut self, t: &Time) -> TimeZone {
        match &t.zone {
            Zone::Utc => TimeZone::UTC,
            Zone::Local => self.local.clone(),
            Zone::Named(id) => {
                let local = &self.local;
                (self.named.entry(id.clone()))
                    .or_insert_with(|| zone_named(id).unwrap_or_else(|| local.clone()))
                    .clone()
            }
        }
    }

    fn instant(&mut self, t: &Time) -> Option<i64> {
        let zone = self.of(t);
        let z = match t.value {
            Value::Date(d) => d.to_zoned(zone).ok()?,
            Value::DateTime(dt) => zone.to_ambiguous_zoned(dt).compatible().ok()?,
        };
        Some(z.timestamp().as_second())
    }

    /// Local midnight `n` days after date `t`.
    fn after_days(&mut self, t: &Time, n: i32) -> Option<i64> {
        let Value::Date(d) = t.value else {
            return None;
        };
        let zone = self.of(t);
        let d = d.checked_add(jiff::Span::new().days(n)).ok()?;
        Some(d.to_zoned(zone).ok()?.timestamp().as_second())
    }
}

/// The zone a TZID names: an IANA name, or one with a vendor prefix such as
/// `/freeassociation.sourceforge.net/Europe/Budapest`, read from its last segments.
fn zone_named(id: &str) -> Option<TimeZone> {
    let id = id.trim_matches('"');
    std::iter::once(id)
        .chain(id.match_indices('/').map(|(i, _)| &id[i + 1..]))
        .filter(|s| !s.is_empty())
        .find_map(|s| TimeZone::get(s).ok())
}

#[derive(Debug, Default)]
struct Event {
    uid: String,
    summary: String,
    start: Option<Time>,
    end: Option<Time>,
    /// DURATION in seconds, when there is no DTEND.
    duration: Option<i64>,
    recurring: bool,
    recurrence_id: Option<Time>,
    exdates: Vec<Time>,
    cancelled: bool,
}

/// The VEVENTs in one iCalendar object, its lines unfolded; the alarms inside them are skipped.
fn parse(object: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut event: Option<Event> = None;
    let mut nested = 0usize;
    for line in unfold(object) {
        let Some((name, params, value)) = content_line(&line) else {
            continue;
        };
        match (name.as_str(), event.as_mut()) {
            ("BEGIN", None) if value.eq_ignore_ascii_case("VEVENT") => {
                event = Some(Event::default());
            }
            ("BEGIN", Some(_)) => nested += 1,
            ("END", Some(_)) if nested > 0 => nested -= 1,
            ("END", Some(_)) => out.extend(event.take()),
            (_, Some(e)) if nested == 0 => property(e, &name, &params, value),
            _ => {}
        }
    }
    out
}

fn property(e: &mut Event, name: &str, params: &[(String, String)], value: &str) {
    let at = || time(params, value);
    match name {
        "UID" => e.uid = value.to_string(),
        "SUMMARY" => e.summary = unescape(value),
        "DTSTART" => e.start = at(),
        "DTEND" => e.end = at(),
        "DURATION" => e.duration = duration(value),
        "RRULE" | "RDATE" => e.recurring = true,
        "RECURRENCE-ID" => e.recurrence_id = at(),
        "EXDATE" => (e.exdates).extend(value.split(',').filter_map(|v| time(params, v))),
        "STATUS" => e.cancelled = value.eq_ignore_ascii_case("CANCELLED"),
        _ => {}
    }
}

/// A content line's parameters: upper-case names, unquoted values.
type Params = Vec<(String, String)>;

/// The logical lines of `text`: a line starting with a space or a tab continues the one before.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ if raw.is_empty() => {}
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

/// A content line's upper-case name, its parameters and its value.
fn content_line(line: &str) -> Option<(String, Params, &str)> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.trim().to_ascii_uppercase(),
                v.trim_matches('"').to_string(),
            ))
        })
        .collect();
    Some((name, params, value))
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// A DATE (`20261113`) or DATE-TIME (`20261113T093000`, `...Z` for UTC) value.
fn time(params: &[(String, String)], value: &str) -> Option<Time> {
    let v = value.trim();
    let num = |r: std::ops::Range<usize>| v.get(r)?.parse::<i32>().ok();
    let date = Date::new(
        i16::try_from(num(0..4)?).ok()?,
        i8::try_from(num(4..6)?).ok()?,
        i8::try_from(num(6..8)?).ok()?,
    )
    .ok()?;
    if v.len() == 8 || param(params, "VALUE").is_some_and(|t| t.eq_ignore_ascii_case("DATE")) {
        return Some(Time {
            value: Value::Date(date),
            zone: Zone::Local,
        });
    }
    if v.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let dt = date.at(
        i8::try_from(num(9..11)?).ok()?,
        i8::try_from(num(11..13)?).ok()?,
        i8::try_from(num(13..15)?.min(59)).ok()?,
        0,
    );
    let zone = if v.ends_with(['Z', 'z']) {
        Zone::Utc
    } else if let Some(id) = param(params, "TZID") {
        Zone::Named(id.to_string())
    } else {
        Zone::Local
    };
    Some(Time {
        value: Value::DateTime(dt),
        zone,
    })
}

/// A DURATION (`PT1H30M`, `P1D`, `P2W`) in seconds; a negative one reads as none.
fn duration(value: &str) -> Option<i64> {
    let v = value.trim().strip_prefix('+').unwrap_or(value.trim());
    let mut rest = v.strip_prefix('P')?;
    let mut secs = 0i64;
    let mut in_time = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('T') {
            in_time = true;
            rest = r;
            continue;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        let n: i64 = rest[..digits].parse().ok()?;
        let unit = match (rest.as_bytes()[digits], in_time) {
            (b'W', false) => 7 * 86_400,
            (b'D', false) => 86_400,
            (b'H', true) => 3600,
            (b'M', true) => 60,
            (b'S', true) => 1,
            _ => return None,
        };
        secs = secs.checked_add(n.checked_mul(unit)?)?;
        rest = &rest[digits + 1..];
    }
    Some(secs)
}

/// A TEXT value with its escapes undone; a line break reads as a space.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budapest() -> TimeZone {
        TimeZone::get("Europe/Budapest").expect("the zoneinfo database")
    }

    fn event(lines: &str) -> String {
        format!("BEGIN:VEVENT\nUID:u\n{lines}END:VEVENT\n").replace('\n', "\r\n")
    }

    fn on(objects: &[String], date: Date) -> Vec<Occurrence> {
        occurrences(objects, date, &budapest())
    }

    /// Seconds of local wall time `h:m` on 2026-11-13 in Budapest (UTC+1).
    fn at(h: i64, m: i64) -> i64 {
        let midnight = day_bounds(Date::constant(2026, 11, 13), &budapest())
            .unwrap()
            .0;
        midnight + h * 3600 + m * 60
    }

    const DAY: Date = Date::constant(2026, 11, 13);

    #[test]
    fn a_timed_event_reads_in_its_own_zone_utc_or_local() {
        let objects = [
            event(
                "SUMMARY:Lunch\nDTSTART;TZID=Europe/Budapest:20261113T130000\nDTEND;TZID=Europe/Budapest:20261113T140000\n",
            ),
            event(
                "SUMMARY:Call\nDTSTART;TZID=America/New_York:20261113T100000\nDTEND;TZID=America/New_York:20261113T110000\n",
            ),
            event("SUMMARY:Review\nDTSTART:20261113T080000Z\nDURATION:PT30M\n"),
            event("SUMMARY:Floating\nDTSTART:20261113T070000\n"),
        ];
        let got = on(&objects, DAY);
        let times: Vec<_> = got
            .iter()
            .map(|o| (o.summary.as_str(), o.start, o.end))
            .collect();
        assert_eq!(
            times,
            [
                ("Lunch", at(13, 0), at(14, 0)),
                ("Call", at(16, 0), at(17, 0)),
                ("Review", at(9, 0), at(9, 30)),
                ("Floating", at(7, 0), at(7, 0)),
            ]
        );
        assert!(got.iter().all(|o| !o.all_day));
    }

    #[test]
    fn an_all_day_event_runs_midnight_to_midnight_and_spans_days() {
        let one =
            event("SUMMARY:Offsite\nDTSTART;VALUE=DATE:20261113\nDTEND;VALUE=DATE:20261114\n");
        let bare = event("SUMMARY:Bare\nDTSTART;VALUE=DATE:20261113\n");
        let trip = event("SUMMARY:Trip\nDTSTART;VALUE=DATE:20261112\nDTEND;VALUE=DATE:20261116\n");
        let got = on(&[one, bare, trip], DAY);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|o| o.all_day));
        assert_eq!((got[0].start, got[0].end), (at(0, 0), at(24, 0)));
        assert_eq!(
            (got[1].start, got[1].end),
            (at(0, 0), at(24, 0)),
            "one day without DTEND"
        );
        assert!(got[2].start < at(0, 0) && got[2].end > at(24, 0));
    }

    #[test]
    fn an_event_off_the_day_is_left_out() {
        let tomorrow = event("SUMMARY:Movie\nDTSTART:20261114T190000Z\nDTEND:20261114T210000Z\n");
        let ends_at_midnight = event("SUMMARY:Early\nDTSTART;VALUE=DATE:20261112\n");
        assert_eq!(on(&[tomorrow, ends_at_midnight], DAY), []);
    }

    #[test]
    fn a_recurring_event_takes_its_wall_time_on_the_day() {
        let daily = event(
            "SUMMARY:Standup\nDTSTART;TZID=Europe/Budapest:20260906T093000\nDTEND;TZID=Europe/Budapest:20260906T094500\nRRULE:FREQ=DAILY\n",
        );
        let got = on(&[daily], DAY);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].start, got[0].end),
            (at(9, 30), at(9, 45)),
            "summer time on DTSTART, winter now"
        );
        let birthday = event(
            "SUMMARY:Birthday\nDTSTART;VALUE=DATE:19801113\nDTEND;VALUE=DATE:19801114\nRRULE:FREQ=YEARLY\n",
        );
        let got = on(&[birthday], DAY);
        assert_eq!((got[0].start, got[0].all_day), (at(0, 0), true));
    }

    #[test]
    fn a_recurring_night_shows_the_one_starting_on_the_day() {
        let nights = event(
            "SUMMARY:Night\nDTSTART;TZID=Europe/Budapest:20261101T230000\nDTEND;TZID=Europe/Budapest:20261102T070000\nRRULE:FREQ=DAILY\n",
        );
        assert_eq!(on(std::slice::from_ref(&nights), DAY)[0].start, at(23, 0));
        let excluded = nights.replace(
            "RRULE",
            "EXDATE;TZID=Europe/Budapest:20261113T230000\r\nRRULE",
        );
        assert_eq!(
            on(&[excluded], DAY)[0].start,
            at(-1, 0),
            "with the day's own night left out, the one from the night before"
        );
    }

    #[test]
    fn a_detached_instance_replaces_its_occurrence() {
        let master =
            event("SUMMARY:Standup\nDTSTART:20261101T083000Z\nDURATION:PT15M\nRRULE:FREQ=DAILY\n");
        let moved = event(
            "SUMMARY:Standup (moved)\nRECURRENCE-ID:20261113T083000Z\nDTSTART:20261113T120000Z\nDURATION:PT15M\n",
        );
        let got = on(&[master, moved], DAY);
        let names: Vec<_> = got.iter().map(|o| (o.summary.as_str(), o.start)).collect();
        assert_eq!(names, [("Standup (moved)", at(13, 0))]);
    }

    #[test]
    fn folded_escaped_and_alarmed_lines_read_whole() {
        let object = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nSUMMARY:A long\r\n  summary\\, folded\\nonce\r\nDTSTART;TZID=\"/freeassociation.sourceforge.net/Europe/Budapest\":20261113T100000\r\nBEGIN:VALARM\r\nDURATION:PT5H\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nDURATION:PT1H\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let got = on(&[object.to_string()], DAY);
        assert_eq!(got[0].summary, "A long summary, folded once");
        assert_eq!((got[0].start, got[0].end), (at(10, 0), at(11, 0)));
    }

    #[test]
    fn a_cancelled_event_is_left_out() {
        let e = event("SUMMARY:Gone\nSTATUS:CANCELLED\nDTSTART:20261113T100000Z\n");
        assert_eq!(on(&[e], DAY), []);
    }

    #[test]
    fn durations_read_in_seconds() {
        assert_eq!(duration("PT1H30M"), Some(5400));
        assert_eq!(duration("P1DT2H"), Some(93_600));
        assert_eq!(duration("P2W"), Some(1_209_600));
        assert_eq!(duration("-PT5M"), None);
        assert_eq!(duration("P1H"), None);
    }
}
