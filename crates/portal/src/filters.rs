//! A FileChooser call's file filters (`a(sa(us))`: a label and patterns, each a glob (0) or a MIME type (1)) and the Files of type list built from them.

pub use reprisefs::filechild::{Filter, Pattern};
use reprisefs::listing::Item;
use reprisefs::mimetree::MimeTree;
use zbus::{OwnedVariant, Variant};

const UNKNOWN: &str = "application/octet-stream";

/// Whether `item` is listed by `f`: a glob matches its name, its type is a kind of a pattern's
/// type in `tree` (a subclass, an alias, or under `major/*`), or it has an execute bit for
/// [`Pattern::Exec`].
pub fn keeps(f: &Filter, item: &Item, tree: &MimeTree) -> bool {
    let mime = item.mime.as_deref().unwrap_or(UNKNOWN);
    f.patterns.iter().any(|p| match p {
        Pattern::Glob(g) => reprisekit::glob::matches(g, &item.name),
        Pattern::Mime(m) => tree.is_a(mime, m),
        Pattern::Exec => item.exec,
    })
}

/// `f` as its `(sa(us))` value; a [`Pattern::Exec`], which the bus type cannot carry, is left out.
pub fn to_value(f: &Filter) -> OwnedVariant {
    let pats: Vec<(u32, String)> = (f.patterns.iter())
        .filter_map(|p| match p {
            Pattern::Glob(g) => Some((0, g.clone())),
            Pattern::Mime(m) => Some((1, m.clone())),
            Pattern::Exec => None,
        })
        .collect();
    Variant::new((f.label.clone(), pats))
        .try_to_owned()
        .expect("plain strings")
}

/// One filter from its `(sa(us))` value; None, logged, when it has no label or no usable pattern.
pub fn parse_filter(v: &Variant<'_>) -> Option<Filter> {
    let Variant::Structure(s) = v else {
        log::warn!("filter: not a structure ({})", v.value_signature());
        return None;
    };
    let (Some(Variant::Str(label)), Some(Variant::Array(pats))) =
        (s.fields().first(), s.fields().get(1))
    else {
        log::warn!("filter: not (sa(us))");
        return None;
    };
    let patterns: Vec<Pattern> = pats
        .inner()
        .iter()
        .filter_map(|p| match p {
            Variant::Structure(p) => match p.fields() {
                [Variant::U32(0), Variant::Str(g)] => Some(Pattern::Glob(g.to_string())),
                [Variant::U32(1), Variant::Str(m)] => Some(Pattern::Mime(m.to_string())),
                _ => {
                    log::warn!("filter {label}: dropped a pattern that is not (0|1, s)");
                    None
                }
            },
            _ => None,
        })
        .collect();
    if patterns.is_empty() {
        log::warn!("filter {label}: no usable pattern, dropped");
        return None;
    }
    Some(Filter {
        label: label.to_string(),
        patterns,
    })
}

pub fn parse_filters(v: &Variant<'_>) -> Vec<Filter> {
    let Variant::Array(a) = v else {
        log::warn!("filters: not an array ({})", v.value_signature());
        return Vec::new();
    };
    a.inner().iter().filter_map(parse_filter).collect()
}

/// The caller's filters and the row to start on: `current` picks its match, or joins the end when the list lacks it. With no filters the dialog shows its own All Files row, never reported back.
pub fn with_current(mut filters: Vec<Filter>, current: Option<Filter>) -> (Vec<Filter>, usize) {
    if let Some(c) = current {
        return match filters.iter().position(|f| *f == c) {
            Some(i) => (filters, i),
            None => {
                filters.push(c);
                let i = filters.len() - 1;
                (filters, i)
            }
        };
    }
    (filters, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn f(label: &str, pats: Vec<Pattern>) -> Filter {
        Filter {
            label: label.into(),
            patterns: pats,
        }
    }

    fn item(name: &str, mime: Option<&str>, exec: bool) -> Item {
        Item {
            name: name.into(),
            path: std::path::Path::new("/tmp").join(name),
            is_dir: false,
            is_link: false,
            exec,
            program: None,
            hidden: false,
            size: 0,
            modified: None,
            mime: mime.map(str::to_string),
            kind: String::new(),
            drive: None,
            mount_root: false,
            takes_writes: false,
            trashed: None,
            shortcut: None,
        }
    }

    fn kept(f: &Filter, name: &str, mime: Option<&str>, t: &MimeTree) -> bool {
        keeps(f, &item(name, mime, false), t)
    }

    #[test]
    fn filters_parse_from_the_bus_type() {
        let v = Variant::new(vec![
            (
                "Images".to_string(),
                vec![
                    (0u32, "*.png".to_string()),
                    (1u32, "image/jpeg".to_string()),
                ],
            ),
            ("Text".to_string(), vec![(0u32, "*.txt".to_string())]),
        ]);
        let fs = parse_filters(&v);
        assert_eq!(
            fs,
            vec![
                f(
                    "Images",
                    vec![
                        Pattern::Glob("*.png".into()),
                        Pattern::Mime("image/jpeg".into())
                    ]
                ),
                f("Text", vec![Pattern::Glob("*.txt".into())]),
            ]
        );
    }

    #[test]
    fn a_bad_pattern_kind_or_type_is_dropped() {
        let v = Variant::new(vec![(
            "Odd".to_string(),
            vec![(7u32, "x".to_string()), (0u32, "*.a".to_string())],
        )]);
        assert_eq!(
            parse_filters(&v),
            vec![f("Odd", vec![Pattern::Glob("*.a".into())])]
        );
        assert!(parse_filters(&Variant::new(3u32)).is_empty());
    }

    #[test]
    fn keeps_globs_exact_and_wildcard_mime_types() {
        let t = MimeTree::default();
        let img = f("Images", vec![Pattern::Mime("image/*".into())]);
        assert!(kept(&img, "a.png", Some("image/png"), &t));
        assert!(!kept(&img, "a.txt", Some("text/plain"), &t));
        let jpeg = f("JPEG", vec![Pattern::Mime("image/jpeg".into())]);
        assert!(!kept(&jpeg, "a.png", Some("image/png"), &t));
        let glob = f("C", vec![Pattern::Glob("*.[ch]".into())]);
        assert!(kept(&glob, "main.C", None, &t));
        let bin = f(
            "Binary",
            vec![Pattern::Mime("application/octet-stream".into())],
        );
        assert!(
            kept(&bin, "blob", None, &t),
            "an unknown type counts as octet-stream"
        );
        assert!(!kept(&jpeg, "blob", None, &t));
    }

    #[test]
    fn a_mime_filter_keeps_subclasses_and_aliases() {
        let t = MimeTree::parse(
            ["application/x-pdf application/pdf\n"],
            ["application/x-shellscript text/plain\nimage/svg+xml application/xml\n"],
        );
        let text = f("Text", vec![Pattern::Mime("text/plain".into())]);
        assert!(kept(&text, "run.sh", Some("application/x-shellscript"), &t));
        assert!(kept(&text, "a.c", Some("text/x-csrc"), &t));
        let xml = f("XML", vec![Pattern::Mime("application/xml".into())]);
        assert!(kept(&xml, "a.svg", Some("image/svg+xml"), &t));
        let pdf = f("PDF", vec![Pattern::Mime("application/x-pdf".into())]);
        assert!(kept(&pdf, "a.pdf", Some("application/pdf"), &t));
        assert!(!kept(&pdf, "a.svg", Some("image/svg+xml"), &t));
    }

    #[test]
    fn current_filter_missing_is_appended() {
        let a = f("A", vec![Pattern::Glob("*.a".into())]);
        let b = f("B", vec![Pattern::Glob("*.b".into())]);
        assert_eq!(
            with_current(vec![a.clone(), b.clone()], Some(b.clone())),
            (vec![a.clone(), b.clone()], 1)
        );
        let c = f("C", vec![Pattern::Glob("*.c".into())]);
        assert_eq!(
            with_current(vec![a.clone()], Some(c.clone())),
            (vec![a, c], 1)
        );
    }

    #[test]
    fn no_filters_stay_none() {
        assert_eq!(with_current(vec![], None), (vec![], 0));
    }

    #[test]
    fn a_filter_goes_back_on_the_bus_as_its_own_type() {
        let v = f(
            "Images",
            vec![
                Pattern::Glob("*.png".into()),
                Pattern::Mime("image/jpeg".into()),
            ],
        );
        assert_eq!(to_value(&v).value_signature().to_string(), "(sa(us))");
    }

    #[test]
    fn an_exec_pattern_keeps_files_with_an_execute_bit_whatever_their_name() {
        let t = MimeTree::default();
        let programs = f("Programs", vec![Pattern::Exec]);
        assert!(keeps(&programs, &item("tool", None, true), &t));
        assert!(keeps(
            &programs,
            &item("notes.txt", Some("text/plain"), true),
            &t
        ));
        assert!(!keeps(&programs, &item("tool", None, false), &t));
    }

    #[test]
    fn an_exec_pattern_stays_off_the_bus() {
        let v = to_value(&f(
            "Programs",
            vec![Pattern::Exec, Pattern::Glob("*.sh".into())],
        ));
        let (_, pats): (String, Vec<(u32, String)>) = v.try_into().unwrap();
        assert_eq!(pats, [(0, "*.sh".to_string())]);
    }
}
