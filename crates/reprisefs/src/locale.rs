use std::ffi::{CStr, CString};

use reprisekit::tr;
use std::sync::OnceLock;

/// Broken-down local time of `secs` (thread-safe `localtime_r`).
pub fn localtime(secs: i64) -> libc::tm {
    // SAFETY: `libc::tm` is plain data, and `localtime_r` writes only the
    // local `tm`.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = secs as libc::time_t;
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// The epoch of a local date-time, the inverse of `localtime`: -1 when
/// libc cannot represent it, which a caller treats as no date.
pub fn mktime(y: i32, mo: i32, d: i32, h: i32, mi: i32, s: i32) -> i64 {
    // SAFETY: `libc::tm` is plain data, and `mktime` reads and normalizes
    // only the local `tm`.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = y - 1900;
        tm.tm_mon = mo - 1;
        tm.tm_mday = d;
        tm.tm_hour = h;
        tm.tm_min = mi;
        tm.tm_sec = s;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm) as i64
    }
}

/// `tm` through strftime's `fmt` in the environment's LC_TIME locale (the
/// one `LC_ALL`, `LC_TIME` or `LANG` names), whatever the process's global
/// locale is, so every program names days and months in the same
/// language. The C locale's names when the environment's does not load.
pub fn strftime(tm: &libc::tm, fmt: &CStr) -> String {
    strftime_in(env_locale(), tm, fmt)
}

fn env_locale() -> Option<&'static TimeLocale> {
    static ENV: OnceLock<Option<TimeLocale>> = OnceLock::new();
    ENV.get_or_init(|| TimeLocale::load(c"").or_else(|| TimeLocale::load(c"C")))
        .as_ref()
}

/// The LC_TIME locale's own date ("11/14/2026" in en_US, "14.11.2026" in de_DE): its `%x`, in [`strftime`].
pub fn short_date(tm: &libc::tm) -> String {
    strftime(tm, SHORT_DATE)
}

/// The weekday and the date with the month named, in the order [`short_date`] writes day, month
/// and year ("Saturday, November 14, 2026" in en_US, "Samstag 14 November 2026" in de_DE).
pub fn long_date(tm: &libc::tm) -> String {
    long_date_in(env_locale(), tm)
}

/// [`long_date`]'s strftime pattern, which holds `%A` for the weekday.
pub fn long_date_format() -> String {
    static FORMAT: OnceLock<String> = OnceLock::new();
    FORMAT.get_or_init(|| long_format_in(env_locale())).clone()
}

const SHORT_DATE: &CStr = c"%x";

fn long_date_in(loc: Option<&TimeLocale>, tm: &libc::tm) -> String {
    let fmt = CString::new(long_format_in(loc)).unwrap_or_default();
    strftime_in(loc, tm, &fmt)
}

fn long_format_in(loc: Option<&TimeLocale>) -> String {
    // SAFETY: `loc` is a live locale object; `nl_langinfo_l` returns null or a NUL-terminated
    // string that stays valid while it lives, and it is copied out before `loc` can go.
    let d_fmt = unsafe {
        let p = match loc {
            Some(l) => libc::nl_langinfo_l(libc::D_FMT, l.0),
            None => std::ptr::null(),
        };
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    };
    long_format_for(d_fmt.as_deref().unwrap_or("%D"))
}

/// The long date's strftime pattern for a locale whose short date is `d_fmt` (its `D_FMT`).
/// glibc has no long date, so this is reprisefs's `long-date` for the order `d_fmt` puts day,
/// month and year in.
pub(crate) fn long_format_for(d_fmt: &str) -> String {
    long_pattern(date_order(d_fmt.as_bytes()))
}

fn long_pattern(order: &str) -> String {
    tr!("long-date", order = order)
}

/// "dmy" or "ymd" when the strftime format `fmt` writes day, month and year in that order, else
/// "mdy".
fn date_order(fmt: &[u8]) -> &'static str {
    let mut order = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        while i < fmt.len() && b"-_0^#EO".contains(&fmt[i]) {
            i += 1;
        }
        let part: &[u8] = match fmt.get(i) {
            Some(b'd' | b'e') => b"d",
            Some(b'm' | b'b' | b'B' | b'h') => b"m",
            Some(b'y' | b'Y') => b"y",
            Some(b'D') => b"mdy",
            Some(b'F') => b"ymd",
            _ => b"",
        };
        for &c in part {
            if !order.contains(&c) {
                order.push(c);
            }
        }
        i += 1;
    }
    match order.as_slice() {
        b"dmy" => "dmy",
        b"ymd" => "ymd",
        _ => "mdy",
    }
}

fn strftime_in(loc: Option<&TimeLocale>, tm: &libc::tm, fmt: &CStr) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for `buf.len()` bytes, `fmt` is
    // NUL-terminated, `tm` is a valid broken-down time and `loc` a live
    // locale object; strftime writes at most that many bytes and returns
    // 0 when the text does not fit.
    let n = unsafe {
        match loc {
            Some(l) => libc::strftime_l(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm, l.0),
            None => libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm),
        }
    };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

struct TimeLocale(libc::locale_t);

// SAFETY: a locale object is only read by the `_l` functions, which may
// use it from any thread at once; nothing changes it after `newlocale`.
unsafe impl Send for TimeLocale {}
// SAFETY: as for Send.
unsafe impl Sync for TimeLocale {}

impl TimeLocale {
    fn load(name: &CStr) -> Option<TimeLocale> {
        // SAFETY: `name` is NUL-terminated; a null base asks for a fresh
        // object.
        let loc =
            unsafe { libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut()) };
        (!loc.is_null()).then(|| TimeLocale(loc))
    }
}

impl Drop for TimeLocale {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `newlocale` and is freed once, here.
        unsafe { libc::freelocale(self.0) };
    }
}

/// Whether the environment's LC_TIME locale writes the time in 12-hour form.
/// Reads the locale named by `LC_ALL`, `LC_TIME` or `LANG` once, whatever the
/// process's global locale is.
pub fn is_12h() -> bool {
    static IS_12H: OnceLock<bool> = OnceLock::new();
    *IS_12H.get_or_init(|| uses_12h(c"").unwrap_or(false))
}

fn uses_12h(name: &CStr) -> Option<bool> {
    // SAFETY: `name` is NUL-terminated. `loc` is a fresh locale object this
    // function owns: `nl_langinfo_l` returns null or a NUL-terminated string
    // that stays valid until `freelocale`, which runs after the scan.
    unsafe {
        let loc = libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            return None;
        }
        let p = libc::nl_langinfo_l(libc::T_FMT, loc);
        let twelve = !p.is_null() && fmt_is_12h(CStr::from_ptr(p).to_bytes());
        libc::freelocale(loc);
        Some(twelve)
    }
}

/// Reread the system time zone. `localtime` keeps the zone it first
/// loaded; after this it follows /etc/localtime again if that changed.
pub fn refresh_zone() {
    // SAFETY: `tzset` takes no arguments and only updates libc's own zone
    // state.
    unsafe { tzset() };
}

unsafe extern "C" {
    fn tzset();
}

/// The first day of the week in the environment's LC_TIME locale, 0 for
/// Sunday through 6 for Saturday. Sunday when the locale does not load.
pub fn first_weekday() -> u32 {
    static FIRST: OnceLock<u32> = OnceLock::new();
    *FIRST.get_or_init(|| locale_first_weekday(c"").unwrap_or(0))
}

/// glibc's `_NL_TIME_WEEK_1STDAY` and `_NL_TIME_FIRST_WEEKDAY`: the date
/// (as a number, yyyymmdd) of a day that starts a week, and the first
/// weekday counted from it, 1-based.
const NL_TIME_WEEK_1STDAY: libc::nl_item = 0x20066;
const NL_TIME_FIRST_WEEKDAY: libc::nl_item = 0x20068;

fn locale_first_weekday(name: &CStr) -> Option<u32> {
    // SAFETY: `name` is NUL-terminated and `loc` is a fresh locale object
    // this function owns until `freelocale`. `_NL_TIME_WEEK_1STDAY` is a
    // number stored in the pointer itself, never dereferenced;
    // `_NL_TIME_FIRST_WEEKDAY` points at one byte.
    unsafe {
        let loc = libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            return None;
        }
        let week1 = libc::nl_langinfo_l(NL_TIME_WEEK_1STDAY, loc) as usize;
        let p = libc::nl_langinfo_l(NL_TIME_FIRST_WEEKDAY, loc);
        let first = if p.is_null() { 1 } else { *p as u8 };
        libc::freelocale(loc);
        Some(week_start(week1, first))
    }
}

/// The weekday (0 Sunday) that `first` (1-based) names, counted from the
/// week-1 day `week1` (yyyymmdd). glibc locales use 19971130, a Sunday;
/// 19971201 is the Monday some older ones use.
fn week_start(week1: usize, first: u8) -> u32 {
    let base = if week1 == 19971201 { 1 } else { 0 };
    (base + u32::from(first.max(1)) - 1) % 7
}

fn fmt_is_12h(fmt: &[u8]) -> bool {
    fmt.windows(2)
        .any(|w| w[0] == b'%' && matches!(w[1], b'I' | b'r' | b'p'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_locale_writes_24_hour_time() {
        assert_eq!(uses_12h(c"C"), Some(false));
    }

    #[test]
    fn en_us_writes_12_hour_time_without_setlocale() {
        if let Some(twelve) = uses_12h(c"en_US.UTF-8") {
            assert!(twelve, "en_US.UTF-8 T_FMT is 12-hour");
        }
    }

    /// The names come from the locale handed in, not the process's: a
    /// test process never calls setlocale, so its global locale is C.
    #[test]
    fn strftime_names_the_month_in_the_locale_it_is_given() {
        let tm = localtime(mktime(2026, 10, 4, 12, 0, 0));
        let c = TimeLocale::load(c"C").expect("the C locale");
        assert_eq!(strftime_in(Some(&c), &tm, c"%B %d, %Y"), "October 04, 2026");
        if let Some(hu) = TimeLocale::load(c"hu_HU.UTF-8") {
            assert_eq!(strftime_in(Some(&hu), &tm, c"%B"), "október");
            assert_eq!(strftime_in(None, &tm, c"%B"), "October");
        }
    }

    #[test]
    fn dates_take_the_order_of_the_locale_they_are_written_in() {
        let tm = localtime(mktime(2026, 11, 14, 12, 0, 0));
        let c = TimeLocale::load(c"C").expect("the C locale");
        assert_eq!(strftime_in(Some(&c), &tm, SHORT_DATE), "11/14/26");
        assert_eq!(long_date_in(Some(&c), &tm), "Saturday, November 14, 2026");
        if let Some(us) = TimeLocale::load(c"en_US.UTF-8") {
            assert_eq!(long_date_in(Some(&us), &tm), "Saturday, November 14, 2026");
        }
        if let Some(hu) = TimeLocale::load(c"hu_HU.UTF-8") {
            let short = strftime_in(Some(&hu), &tm, SHORT_DATE);
            assert!(short.starts_with("2026"), "the year first: {short}");
            assert_eq!(long_date_in(Some(&hu), &tm), format!("szombat, {short}"));
        }
        if let Some(de) = TimeLocale::load(c"de_DE.UTF-8") {
            assert_eq!(long_date_in(Some(&de), &tm), "Samstag 14 November 2026");
        }
    }

    #[test]
    fn a_short_dates_format_gives_the_order_of_day_month_and_year() {
        assert_eq!(date_order(b"%m/%d/%y"), "mdy", "C");
        assert_eq!(date_order(b"%D"), "mdy");
        assert_eq!(date_order(b"%d/%m/%y"), "dmy", "en_GB");
        assert_eq!(date_order(b"%d.%m.%Y"), "dmy", "de_DE");
        assert_eq!(date_order(b"%-d.%-m.%Y"), "dmy");
        assert_eq!(date_order(b"%e %b %Y"), "dmy");
        assert_eq!(date_order(b"%Y. %m. %d."), "ymd", "hu_HU");
        assert_eq!(date_order(b"%F"), "ymd");
        assert_eq!(date_order("%Y年%m月%d日".as_bytes()), "ymd", "ja_JP");
        assert_eq!(date_order(b"%Ey"), "mdy", "no day or month");
        assert_eq!(date_order(b"%d/%Y/%m"), "mdy", "another order");
    }

    #[test]
    fn the_long_date_names_the_month_in_the_short_dates_order() {
        assert_eq!(long_pattern("mdy"), "%A, %B %-d, %Y");
        assert_eq!(long_pattern("dmy"), "%A %-d %B %Y");
        assert_eq!(
            long_pattern("ymd"),
            "%A, %x",
            "a year-first locale keeps its short date"
        );
    }

    #[test]
    fn a_missing_locale_does_not_load() {
        assert_eq!(uses_12h(c"xx_NOPE.UTF-8"), None);
    }

    #[test]
    fn the_week_starts_on_the_locales_first_weekday() {
        assert_eq!(week_start(19971130, 1), 0, "Sunday");
        assert_eq!(week_start(19971130, 2), 1, "Monday");
        assert_eq!(week_start(19971201, 1), 1, "Monday-based week 1");
        assert_eq!(locale_first_weekday(c"C"), Some(0));
        if let Some(first) = locale_first_weekday(c"hu_HU.UTF-8") {
            assert_eq!(first, 1, "hu_HU starts on Monday");
        }
    }

    #[test]
    fn am_pm_and_12_hour_specifiers_mark_12_hour_formats() {
        assert!(fmt_is_12h(b"%r"));
        assert!(fmt_is_12h(b"%I:%M:%S %p"));
        assert!(!fmt_is_12h(b"%H:%M:%S"));
        assert!(!fmt_is_12h(b"%T"));
    }
}
