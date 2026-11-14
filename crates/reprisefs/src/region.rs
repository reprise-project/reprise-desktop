//! Region & Language: the installed locales with their own names and formats, and the user's
//! `locale.conf` under `$XDG_CONFIG_HOME`, which the session takes at log on ([`session_env`]).

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

/// The locale variables a session may hold, `LANG` first.
pub const VARS: [&str; 15] = [
    "LANG",
    "LANGUAGE",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
    "LC_ALL",
];

/// The categories a formats locale sets: dates and times, numbers, money, measures, paper.
pub const FORMATS: [&str; 5] = [
    "LC_TIME",
    "LC_NUMERIC",
    "LC_MONETARY",
    "LC_MEASUREMENT",
    "LC_PAPER",
];

/// A display language and the locale its formats follow, None for the language's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub language: String,
    pub formats: Option<String>,
}

impl Choice {
    /// The choice `pairs` (locale.conf's or the environment's) hold: `LANG`, and the first
    /// format variable's locale where it differs. None without `LANG`.
    pub fn of(pairs: &[(String, String)]) -> Option<Choice> {
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| normalize(v))
        };
        let language = get("LANG")?;
        let formats = FORMATS
            .iter()
            .find_map(|k| get(k))
            .filter(|f| *f != language);
        Some(Choice { language, formats })
    }

    /// The locale dates, numbers and measures follow.
    pub fn formats_locale(&self) -> &str {
        self.formats.as_deref().unwrap_or(&self.language)
    }

    /// `pairs` with `LANG` and the format variables set as this choice says; the others kept.
    pub fn merge(&self, pairs: &[(String, String)]) -> Vec<(String, String)> {
        let mut out = vec![("LANG".to_string(), self.language.clone())];
        if let Some(f) = &self.formats {
            out.extend(FORMATS.iter().map(|k| (k.to_string(), f.clone())));
        }
        out.extend(
            pairs
                .iter()
                .filter(|(k, _)| k != "LANG" && !FORMATS.contains(&k.as_str()))
                .cloned(),
        );
        out
    }
}

/// A locale name with glibc's codeset spelling (`utf8`) written the usual way (`UTF-8`).
pub fn normalize(name: &str) -> String {
    let (base, modifier) = match name.split_once('@') {
        Some((b, m)) => (b, Some(m)),
        None => (name, None),
    };
    let base = match base.split_once('.') {
        Some((lang, cs)) if cs.eq_ignore_ascii_case("utf8") || cs.eq_ignore_ascii_case("utf-8") => {
            format!("{lang}.UTF-8")
        }
        _ => base.to_string(),
    };
    match modifier {
        Some(m) => format!("{base}@{m}"),
        None => base,
    }
}

/// The `LANG` and `LC_*` assignments of locale.conf `text`, quotes taken off.
pub fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().split_once('='))
        .filter(|(k, _)| VARS.contains(k))
        .map(|(k, v)| (k.to_string(), unquote(v.trim()).to_string()))
        .collect()
}

fn unquote(v: &str) -> &str {
    ['"', '\'']
        .iter()
        .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
        .unwrap_or(v)
}

/// locale.conf `text` rewritten for `choice`: `LANG` and the format variables first, then the
/// lines it had besides them, comments and other variables kept.
pub fn rewrite(text: &str, choice: &Choice) -> String {
    let mut out: String = choice
        .merge(&[])
        .iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect();
    for line in text.lines() {
        let set = line
            .trim()
            .split_once('=')
            .is_some_and(|(k, _)| k == "LANG" || FORMATS.contains(&k));
        if !set {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The user's locale.conf.
pub fn user_conf() -> PathBuf {
    crate::autostart::config_home().join("locale.conf")
}

/// Write `choice` into the locale.conf at `path`, keeping its other lines.
pub fn save(path: &Path, choice: &Choice) -> std::io::Result<()> {
    let old = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    crate::ops::write_atomic(path, rewrite(&old, choice).as_bytes())
}

/// How the session's environment takes the user's locale.conf `text`: each of [`VARS`] with
/// its value, or None to remove it. A file that sets nothing changes nothing; one without
/// `LANG` keeps the inherited `LANG`.
pub fn session_env(text: &str) -> Vec<(&'static str, Option<String>)> {
    let set = parse(text);
    if set.is_empty() {
        return Vec::new();
    }
    VARS.iter()
        .filter_map(|&k| match set.iter().rev().find(|(n, _)| n == k) {
            Some((_, v)) => Some((k, Some(v.clone()))),
            None if k == "LANG" => None,
            None => Some((k, None)),
        })
        .collect()
}

/// The UTF-8 locales `locale -a` lists, sorted, written as [`normalize`] does.
pub fn installed() -> Vec<String> {
    let out = std::process::Command::new("locale")
        .arg("-a")
        .env("LC_ALL", "C")
        .output();
    match out {
        Ok(o) if o.status.success() => utf8_locales(&String::from_utf8_lossy(&o.stdout)),
        Ok(o) => {
            log::warn!("locale -a exited with {}", o.status);
            Vec::new()
        }
        Err(e) => {
            log::warn!("locale -a: {e}");
            Vec::new()
        }
    }
}

/// The language-and-territory UTF-8 locales of a `locale -a` listing.
fn utf8_locales(listing: &str) -> Vec<String> {
    let mut v: Vec<String> = listing
        .lines()
        .map(str::trim)
        .filter(|l| l.contains('_'))
        .map(normalize)
        .filter(|l| l.split('@').next().is_some_and(|b| b.ends_with(".UTF-8")))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Whether the C library loads locale `code`: an installed one, or C.UTF-8 and POSIX, which it
/// carries itself.
pub fn loads(code: &str) -> bool {
    Loaded::new(libc::LC_ALL_MASK, code).is_some()
}

/// A loaded locale object for the `_l` functions.
struct Loaded(libc::locale_t);

impl Loaded {
    fn new(mask: libc::c_int, name: &str) -> Option<Loaded> {
        let name = CString::new(name).ok()?;
        // SAFETY: `name` is NUL-terminated; a null base asks for a fresh object.
        let loc = unsafe { libc::newlocale(mask, name.as_ptr(), std::ptr::null_mut()) };
        (!loc.is_null()).then(|| Loaded(loc))
    }

    /// `nl_langinfo_l`'s string for `item`, empty for none.
    fn info(&self, item: libc::nl_item) -> String {
        // SAFETY: `self.0` is a live locale object; the answer is null or a NUL-terminated
        // string that lives as long as it, copied out here.
        unsafe {
            let p = libc::nl_langinfo_l(item, self.0);
            if p.is_null() {
                return String::new();
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    /// An item glibc stores as a number in the answer's pointer itself.
    fn word(&self, item: libc::nl_item) -> u32 {
        // SAFETY: as in `info`; the pointer is read as a number, never dereferenced.
        unsafe { libc::nl_langinfo_l(item, self.0) as usize as u32 }
    }

    /// An item glibc stores as one byte the answer points at.
    fn byte(&self, item: libc::nl_item) -> u8 {
        // SAFETY: as in `info`; for these items the answer points at one byte.
        unsafe {
            let p = libc::nl_langinfo_l(item, self.0);
            if p.is_null() { 0 } else { *p as u8 }
        }
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `newlocale` and is freed once, here.
        unsafe { libc::freelocale(self.0) };
    }
}

/// glibc's `_NL_ADDRESS_COUNTRY_NAME`, `_NL_ADDRESS_LANG_NAME`, `_NL_PAPER_HEIGHT`,
/// `_NL_PAPER_WIDTH` and `_NL_MEASUREMENT_MEASUREMENT` (1 metric, 2 US customary).
const NL_COUNTRY_NAME: libc::nl_item = 0x90001;
const NL_LANG_NAME: libc::nl_item = 0x90008;
const NL_PAPER_HEIGHT: libc::nl_item = 0x70000;
const NL_PAPER_WIDTH: libc::nl_item = 0x70001;
const NL_MEASUREMENT: libc::nl_item = 0xb0000;

/// The locale's name in its own language, "Deutsch (Deutschland)"; the locale itself when it
/// does not load or names neither.
pub fn name(locale: &str) -> String {
    let Some(l) = Loaded::new(libc::LC_ADDRESS_MASK, locale) else {
        return locale.to_string();
    };
    let (lang, country) = (l.info(NL_LANG_NAME), l.info(NL_COUNTRY_NAME));
    match (lang.is_empty(), country.is_empty()) {
        (true, _) => locale.to_string(),
        (false, true) => lang,
        (false, false) => format!("{lang} ({country})"),
    }
}

/// How a formats locale writes a moment and an amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub date: String,
    pub time: String,
    pub number: String,
    pub currency: String,
    pub metric: bool,
    /// Paper width and height in mm.
    pub paper: (u32, u32),
}

/// The number and the amount [`sample`] writes.
const AMOUNT: f64 = 1_234_567.89;

/// The time `secs` and [`AMOUNT`] as `locale` writes them; None when it does not load.
pub fn sample(locale: &str, secs: i64) -> Option<Sample> {
    let mask = libc::LC_TIME_MASK
        | libc::LC_NUMERIC_MASK
        | libc::LC_MONETARY_MASK
        | libc::LC_MEASUREMENT_MASK
        | libc::LC_PAPER_MASK;
    let l = Loaded::new(mask, locale)?;
    let tm = crate::locale::localtime(secs);
    Some(Sample {
        date: strftime(&l, &tm, c"%A, %x"),
        time: strftime(&l, &tm, c"%X"),
        number: grouped(AMOUNT, &l.info(libc::THOUSEP), &l.info(libc::RADIXCHAR)),
        currency: money(&l, AMOUNT),
        metric: l.byte(NL_MEASUREMENT) != 2,
        paper: (l.word(NL_PAPER_WIDTH), l.word(NL_PAPER_HEIGHT)),
    })
}

fn strftime(l: &Loaded, tm: &libc::tm, fmt: &CStr) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for its length, `fmt` NUL-terminated, `tm` a valid broken-down
    // time and `l.0` a live locale object; strftime writes at most that many bytes.
    let n = unsafe { libc::strftime_l(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm, l.0) };
    String::from_utf8_lossy(&buf[..n]).trim().to_string()
}

unsafe extern "C" {
    fn strfmon_l(
        s: *mut libc::c_char,
        max: libc::size_t,
        loc: libc::locale_t,
        format: *const libc::c_char,
        ...
    ) -> libc::ssize_t;
}

fn money(l: &Loaded, amount: f64) -> String {
    let mut buf = [0u8; 128];
    // SAFETY: `buf` is writable for its length, the format NUL-terminated and takes the one
    // double passed, `l.0` a live locale object; strfmon_l writes at most that many bytes.
    let n = unsafe {
        strfmon_l(
            buf.as_mut_ptr().cast(),
            buf.len(),
            l.0,
            c"%n".as_ptr(),
            amount,
        )
    };
    match usize::try_from(n) {
        Ok(n) => String::from_utf8_lossy(&buf[..n]).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// `v` with two decimals, its whole part grouped in threes by `group`.
fn grouped(v: f64, group: &str, decimal: &str) -> String {
    let fixed = format!("{v:.2}");
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push_str(group);
        }
        out.push(c);
    }
    let decimal = if decimal.is_empty() { "." } else { decimal };
    format!("{out}{decimal}{frac}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn glibc_codeset_spelling_reads_as_utf_8() {
        assert_eq!(normalize("de_DE.utf8"), "de_DE.UTF-8");
        assert_eq!(normalize("ca_ES.utf8@valencia"), "ca_ES.UTF-8@valencia");
        assert_eq!(normalize("en_US.UTF-8"), "en_US.UTF-8");
        assert_eq!(normalize("C"), "C");
    }

    #[test]
    fn the_listing_keeps_utf_8_language_and_territory_locales() {
        let listing = "C\nC.utf8\nPOSIX\nde_DE\nde_DE.utf8\nen_US.utf8\nca_ES.utf8@valencia\n";
        assert_eq!(
            utf8_locales(listing),
            ["ca_ES.UTF-8@valencia", "de_DE.UTF-8", "en_US.UTF-8"]
        );
    }

    #[test]
    fn a_choice_is_lang_and_the_formats_that_differ() {
        let c = Choice::of(&pairs(&[
            ("LANG", "de_DE.utf8"),
            ("LC_TIME", "en_GB.UTF-8"),
        ]));
        assert_eq!(
            c,
            Some(Choice {
                language: "de_DE.UTF-8".into(),
                formats: Some("en_GB.UTF-8".into()),
            })
        );
        let same = Choice::of(&pairs(&[
            ("LANG", "de_DE.UTF-8"),
            ("LC_TIME", "de_DE.UTF-8"),
        ]));
        assert_eq!(same.unwrap().formats, None);
        assert_eq!(Choice::of(&pairs(&[("LC_TIME", "de_DE.UTF-8")])), None);
    }

    #[test]
    fn merging_sets_lang_and_every_format_and_keeps_the_rest() {
        let c = Choice {
            language: "de_DE.UTF-8".into(),
            formats: Some("en_GB.UTF-8".into()),
        };
        let had = pairs(&[
            ("LANG", "en_US.UTF-8"),
            ("LC_COLLATE", "C.UTF-8"),
            ("LC_TIME", "fr_FR.UTF-8"),
        ]);
        let merged = c.merge(&had);
        let keys: Vec<&str> = merged.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "LANG",
                "LC_TIME",
                "LC_NUMERIC",
                "LC_MONETARY",
                "LC_MEASUREMENT",
                "LC_PAPER",
                "LC_COLLATE"
            ]
        );
        assert!(merged[1..6].iter().all(|(_, v)| v == "en_GB.UTF-8"));
        let own = Choice { formats: None, ..c };
        assert_eq!(
            own.merge(&had),
            pairs(&[("LANG", "de_DE.UTF-8"), ("LC_COLLATE", "C.UTF-8")])
        );
    }

    #[test]
    fn a_rewrite_keeps_comments_and_other_variables() {
        let text = "# mine\nLANG=en_US.UTF-8\nLC_TIME=\"fr_FR.UTF-8\"\nLC_COLLATE=C.UTF-8\n";
        let c = Choice {
            language: "de_DE.UTF-8".into(),
            formats: None,
        };
        assert_eq!(
            rewrite(text, &c),
            "LANG=de_DE.UTF-8\n# mine\nLC_COLLATE=C.UTF-8\n"
        );
        assert_eq!(parse(&rewrite("", &c)), pairs(&[("LANG", "de_DE.UTF-8")]));
    }

    #[test]
    fn the_saved_file_reads_back_as_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg/locale.conf");
        let c = Choice {
            language: "hu_HU.UTF-8".into(),
            formats: Some("en_US.UTF-8".into()),
        };
        save(&path, &c).unwrap();
        let back = Choice::of(&parse(&std::fs::read_to_string(&path).unwrap()));
        assert_eq!(back, Some(c));
    }

    #[test]
    fn save_through_a_symlink_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/locale.conf");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "LANG=C.UTF-8\n").unwrap();
        let link = dir.path().join("locale.conf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let c = Choice {
            language: "hu_HU.UTF-8".into(),
            formats: None,
        };
        save(&link, &c).unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        let back = Choice::of(&parse(&std::fs::read_to_string(&real).unwrap()));
        assert_eq!(back, Some(c));
    }

    #[test]
    fn the_session_takes_the_files_variables_and_drops_the_others() {
        let env = session_env("LANG=de_DE.UTF-8\nLC_TIME=en_GB.UTF-8\n");
        let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        assert_eq!(get("LANG"), Some(Some("de_DE.UTF-8".into())));
        assert_eq!(get("LC_TIME"), Some(Some("en_GB.UTF-8".into())));
        assert_eq!(get("LANGUAGE"), Some(None), "an inherited list would win");
        assert_eq!(get("LC_ALL"), Some(None));
        assert_eq!(env.len(), VARS.len());

        let time_only = session_env("LC_TIME=en_GB.UTF-8\n");
        assert!(
            !time_only.iter().any(|(k, _)| *k == "LANG"),
            "the inherited LANG stays"
        );
        assert!(session_env("# nothing\n").is_empty());
    }

    #[test]
    fn numbers_group_in_threes() {
        assert_eq!(grouped(1_234_567.89, ",", "."), "1,234,567.89");
        assert_eq!(grouped(1_234_567.89, ".", ","), "1.234.567,89");
        assert_eq!(grouped(567.5, "", "."), "567.50");
    }

    #[test]
    fn a_locale_is_named_in_its_own_language() {
        assert_eq!(name("xx_NOPE.UTF-8"), "xx_NOPE.UTF-8");
        if Loaded::new(libc::LC_ADDRESS_MASK, "en_US.UTF-8").is_some() {
            assert_eq!(name("en_US.UTF-8"), "English (United States)");
        }
        if Loaded::new(libc::LC_ADDRESS_MASK, "hu_HU.UTF-8").is_some() {
            assert_eq!(name("hu_HU.UTF-8"), "magyar (Magyarország)");
        }
    }

    #[test]
    fn the_sample_follows_the_formats_locale() {
        let at = crate::locale::mktime(2026, 11, 13, 19, 54, 40);
        let c = sample("C", at).expect("the C locale");
        assert_eq!(c.number, "1234567.89");
        assert!(c.metric);
        if let Some(us) = sample("en_US.UTF-8", at) {
            assert_eq!(us.date, "Friday, 11/13/2026");
            assert_eq!(us.time, "07:54:40 PM");
            assert_eq!(us.number, "1,234,567.89");
            assert_eq!(us.currency, "$1,234,567.89");
            assert!(!us.metric);
            assert_eq!(us.paper, (216, 279));
        }
        if let Some(hu) = sample("hu_HU.UTF-8", at) {
            assert_eq!(hu.number, "1.234.567,89");
            assert_eq!(hu.time, "19:54:40");
            assert!(hu.metric);
            assert_eq!(hu.paper, (210, 297));
        }
        assert_eq!(sample("xx_NOPE.UTF-8", at), None);
    }
}
