//! UI strings through Fluent. Each crate compiles its English `locale/en.ftl` in with
//! [`strings!`](crate::strings) and looks ids up with [`tr!`](crate::tr); other locales load from
//! `reprise/locale/<lang>/<domain>.ftl` under the XDG data dirs. docs/i18n.md has the workflow.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::types::FluentNumberOptions;
use fluent_bundle::{FluentArgs, FluentResource};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::FluentValue as Value;

type Bundle = FluentBundle<FluentResource>;

/// Declares the calling crate's `i18n::STRINGS`, the domain [`tr!`](crate::tr) reads; it goes
/// at the crate root. `domain` names the installed `<lang>/<domain>.ftl` files, `locale/en.ftl`
/// beside the crate's Cargo.toml is the compiled-in English. Under `cfg(test)` the crate's
/// strings are English whatever the locale.
#[macro_export]
macro_rules! strings {
    ($domain:literal) => {
        pub(crate) mod i18n {
            pub(crate) static STRINGS: $crate::i18n::Strings = $crate::i18n::Strings::new(
                $domain,
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locale/en.ftl")),
                cfg!(test),
            );
        }
    };
}

/// The calling crate's string `id`, with Fluent variables as `name = value` pairs:
/// `tr!("window-title", name = "notes.txt")`. The id is a literal so `scripts/i18n-check`
/// can list every one; it resolves through the crate's [`strings!`](crate::strings).
#[macro_export]
// `crate` is the caller's: each crate reads its own strings.
#[allow(clippy::crate_in_macro_def)]
macro_rules! tr {
    ($id:literal) => {
        crate::i18n::STRINGS.tr($id)
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {
        crate::i18n::STRINGS.tr_args(
            $id,
            &[$((stringify!($name), $crate::i18n::Value::from($value))),+],
        )
    };
}

/// reprisekit's own strings, which [`kit_tr!`](crate::kit_tr) reads from any crate.
pub static STRINGS: Strings =
    Strings::new("reprisekit", include_str!("../locale/en.ftl"), cfg!(test));

/// reprisekit's string `id` (the common buttons: `ok`, `cancel`, `yes`...) from any crate.
#[macro_export]
macro_rules! kit_tr {
    ($id:literal) => {
        $crate::i18n::STRINGS.tr($id)
    };
}

/// One crate's strings: the English text compiled in, and the [`Catalog`] built from it and the
/// user's locale on first use.
pub struct Strings {
    domain: &'static str,
    en: &'static str,
    english_only: bool,
    catalog: OnceLock<Catalog>,
}

impl Strings {
    pub const fn new(domain: &'static str, en: &'static str, english_only: bool) -> Strings {
        Strings {
            domain,
            en,
            english_only,
            catalog: OnceLock::new(),
        }
    }

    pub fn tr(&self, id: &str) -> String {
        self.catalog().tr_args(id, &[])
    }

    pub fn tr_args(&self, id: &str, args: &[(&str, Value)]) -> String {
        self.catalog().tr_args(id, args)
    }

    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| {
            if self.english_only {
                Catalog::english(self.en)
            } else {
                Catalog::load(self.domain, self.en, &requested(env), &locale_dirs())
            }
        })
    }
}

/// A chain of bundles, the most preferred language first and English last: an id missing from a
/// translation, or failing to format there, falls through to the next.
pub struct Catalog {
    bundles: Vec<Bundle>,
}

impl Catalog {
    pub fn english(en: &str) -> Catalog {
        Catalog::from_texts(&[(english(), en)])
    }

    /// `en` behind the translations of `domain` in `dirs` (earlier dirs win) for the languages
    /// `requested` negotiates to.
    pub fn load(
        domain: &str,
        en: &str,
        requested: &[LanguageIdentifier],
        dirs: &[PathBuf],
    ) -> Catalog {
        let file = format!("{domain}.ftl");
        let mut found: Vec<(LanguageIdentifier, PathBuf)> = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path().join(&file);
                let lang = entry.file_name().to_str().and_then(langid);
                if let Some(lang) = lang.filter(|_| path.is_file()) {
                    found.push((lang, path));
                }
            }
        }
        let mut available: Vec<LanguageIdentifier> = Vec::new();
        for (lang, _) in &found {
            if !available.contains(lang) {
                available.push(lang.clone());
            }
        }
        let default = english();
        let chosen = negotiate_languages(
            requested,
            &available,
            Some(&default),
            NegotiationStrategy::Filtering,
        );
        let mut texts = Vec::new();
        for lang in chosen.into_iter().filter(|l| **l != default) {
            for (_, path) in found.iter().filter(|(l, _)| l == lang) {
                match std::fs::read_to_string(path) {
                    Ok(text) => texts.push((lang.clone(), text)),
                    Err(e) => log::warn!("{}: {e}", path.display()),
                }
            }
        }
        let mut chain: Vec<(LanguageIdentifier, &str)> =
            texts.iter().map(|(l, t)| (l.clone(), t.as_str())).collect();
        chain.push((default, en));
        Catalog::from_texts(&chain)
    }

    /// A catalog of the given `(language, ftl text)` pairs, most preferred first; the last
    /// should be English and hold every id.
    pub fn from_texts(chain: &[(LanguageIdentifier, &str)]) -> Catalog {
        let bundles = chain
            .iter()
            .map(|(lang, text)| {
                let resource =
                    FluentResource::try_new(text.to_string()).unwrap_or_else(|(r, e)| {
                        log::warn!("{lang}: {} unparsed ftl entries: {e:?}", e.len());
                        r
                    });
                let mut bundle = Bundle::new_concurrent(vec![lang.clone()]);
                // FSI/PDI marks around placeables would reach the text renderer as glyphs.
                bundle.set_use_isolating(false);
                bundle.set_formatter(Some(number_formatter));
                if let Err(e) = bundle.add_builtins() {
                    log::warn!("{lang}: {e:?}");
                }
                if let Err(e) = bundle.add_resource(resource) {
                    log::warn!("{lang}: {e:?}");
                }
                bundle
            })
            .collect();
        Catalog { bundles }
    }

    /// `id` formatted with `args`; the id itself when no bundle has it.
    pub fn tr_args(&self, id: &str, args: &[(&str, Value)]) -> String {
        let fargs = (!args.is_empty()).then(|| {
            let mut a = FluentArgs::with_capacity(args.len());
            for (name, value) in args {
                a.set(*name, value.clone());
            }
            a
        });
        let last = self.bundles.len().saturating_sub(1);
        for (i, bundle) in self.bundles.iter().enumerate() {
            let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, fargs.as_ref(), &mut errors);
            if errors.is_empty() || i == last {
                if !errors.is_empty() {
                    log::warn!("string {id}: {errors:?}");
                }
                return text.into_owned();
            }
            log::warn!("string {id} in {}: {errors:?}", bundle.locales[0]);
        }
        log::warn!("no string {id}");
        id.to_string()
    }
}

/// The digit separators of numbers in UI text, from reprisekit's strings in the user's language.
pub struct Separators {
    pub group: String,
    pub decimal: String,
}

thread_local! {
    /// Set while the separators format, so a number in their own strings takes Fluent's plain
    /// form instead of re-entering [`separators`]'s `OnceLock`.
    static READING_SEPARATORS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn number_formatter<M>(v: &Value, _: &M) -> Option<String> {
    match v {
        Value::Number(_) if READING_SEPARATORS.get() => None,
        Value::Number(n) => Some(format_number(n.value, &n.options, separators())),
        _ => None,
    }
}

fn separators() -> &'static Separators {
    static SEPARATORS: OnceLock<Separators> = OnceLock::new();
    SEPARATORS.get_or_init(|| {
        separators_from(|| {
            [
                tr!("number-group-separator"),
                tr!("number-decimal-separator"),
            ]
        })
    })
}

fn separators_from(read: impl FnOnce() -> [String; 2]) -> Separators {
    struct Reading;
    impl Drop for Reading {
        fn drop(&mut self) {
            READING_SEPARATORS.set(false);
        }
    }
    READING_SEPARATORS.set(true);
    let _reading = Reading;
    let [group, decimal] = read();
    Separators { group, decimal }
}

/// `value` as Fluent's `NUMBER()` asks: fraction digits between the minimum and maximum
/// (0 and 3 by default, as in `Intl.NumberFormat`), digits grouped in threes unless
/// `useGrouping: "false"`.
pub fn format_number(value: f64, opts: &FluentNumberOptions, seps: &Separators) -> String {
    let min = opts.minimum_fraction_digits.unwrap_or(0);
    let max = opts.maximum_fraction_digits.unwrap_or(3).max(min);
    let fixed = format!("{:.*}", max, value.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let keep = frac.trim_end_matches('0').len().max(min);
    let mut out = String::new();
    if value < 0.0 && fixed.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if opts.use_grouping && i > 0 && (int.len() - i).is_multiple_of(3) {
            out += &seps.group;
        }
        out.push(c);
    }
    if keep > 0 {
        out += &seps.decimal;
        out += &frac[..keep];
    }
    out
}

fn english() -> LanguageIdentifier {
    LanguageIdentifier::from_bytes(b"en").expect("en is a language id")
}

/// A POSIX locale or a language tag (`de_DE.UTF-8@euro`, `pt-BR`) as a language id.
fn langid(s: &str) -> Option<LanguageIdentifier> {
    let base = s.split(['.', '@']).next()?.replace('_', "-");
    base.parse().ok().filter(|_| !base.is_empty())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The UI languages asked for, in order: `LANGUAGE`'s list, then the message locale
/// (`LC_ALL`, `LC_MESSAGES`, `LANG`). As in gettext, a `C` or `POSIX` locale means English and
/// silences `LANGUAGE`.
pub fn requested(env: impl Fn(&str) -> Option<String>) -> Vec<LanguageIdentifier> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|n| env(n));
    let Some(locale) = locale else {
        return Vec::new();
    };
    let base = locale.split(['.', '@']).next().unwrap_or("");
    if base == "C" || base == "POSIX" {
        return vec![english()];
    }
    let list = env("LANGUAGE").unwrap_or_default();
    list.split(':')
        .chain([locale.as_str()])
        .filter_map(langid)
        .collect()
}

/// `reprise/locale` under `$XDG_DATA_HOME`, then under each `$XDG_DATA_DIRS`.
pub fn locale_dirs() -> Vec<PathBuf> {
    let home = env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| Path::new(&h).join(".local/share")));
    let dirs = env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .map(|d| d.join("reprise/locale"))
        .collect()
}

#[cfg(test)]
mod tests;
