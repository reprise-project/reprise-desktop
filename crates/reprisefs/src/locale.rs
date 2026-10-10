use std::ffi::CStr;
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
    static ENV: OnceLock<Option<TimeLocale>> = OnceLock::new();
    let env = ENV.get_or_init(|| TimeLocale::load(c"").or_else(|| TimeLocale::load(c"C")));
    strftime_in(env.as_ref(), tm, fmt)
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
