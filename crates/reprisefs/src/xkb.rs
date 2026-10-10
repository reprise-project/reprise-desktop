//! The keyboard layouts xkeyboard-config knows, read from its registry
//! (`rules/<rules>.xml`): each layout's code, short name, description and
//! variants. Also the `[keyboard] layout`/`variant` pair as (layout, variant) list.

use std::path::PathBuf;

/// The most layouts one keymap holds (xkb has four groups).
pub const MAX_LAYOUTS: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// The code `[keyboard] layout` names ("us").
    pub name: String,
    /// The indicator's text ("en"); may be empty.
    pub short: String,
    pub description: String,
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub short: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    pub short: String,
    pub description: String,
}

/// The registry file for `rules` ("" means evdev), under `$XKB_CONFIG_ROOT` else `/usr/share/X11/xkb`.
pub fn registry_path(rules: &str) -> PathBuf {
    let root = std::env::var_os("XKB_CONFIG_ROOT")
        .filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/X11/xkb"));
    let rules = if rules.is_empty() { "evdev" } else { rules };
    root.join("rules").join(format!("{rules}.xml"))
}

pub fn load(rules: &str) -> Vec<Layout> {
    let path = registry_path(rules);
    match std::fs::read_to_string(&path) {
        Ok(xml) => parse(&xml),
        Err(e) => {
            log::warn!("xkb registry {}: {e}", path.display());
            Vec::new()
        }
    }
}

pub fn parse(xml: &str) -> Vec<Layout> {
    let xml = strip_comments(xml);
    let Some(list) = blocks(&xml, "layoutList").into_iter().next() else {
        return Vec::new();
    };
    blocks(list, "layout")
        .into_iter()
        .filter_map(|block| {
            let own = blocks(block, "configItem").into_iter().next()?;
            let variants = blocks(block, "variant")
                .into_iter()
                .filter_map(|v| {
                    let item = blocks(v, "configItem").into_iter().next()?;
                    Some(Variant {
                        name: text(item, "name")?,
                        short: text(item, "shortDescription").unwrap_or_default(),
                        description: text(item, "description").unwrap_or_default(),
                    })
                })
                .collect();
            Some(Layout {
                name: text(own, "name")?,
                short: text(own, "shortDescription").unwrap_or_default(),
                description: text(own, "description").unwrap_or_default(),
                variants,
            })
        })
        .collect()
}

/// Indicator text and description of `layout` with `variant` ("" for none); the variant's own win, an unknown code shows as itself.
pub fn describe(reg: &[Layout], layout: &str, variant: &str) -> Described {
    let found = reg.iter().find(|l| l.name == layout);
    let var = found.and_then(|l| {
        (!variant.is_empty())
            .then(|| l.variants.iter().find(|v| v.name == variant))
            .flatten()
    });
    let short = var
        .map(|v| v.short.as_str())
        .filter(|s| !s.is_empty())
        .or(found.map(|l| l.short.as_str()).filter(|s| !s.is_empty()))
        .unwrap_or(layout);
    let description = match (var, found) {
        (Some(v), _) if !v.description.is_empty() => v.description.clone(),
        (_, Some(l)) if !l.description.is_empty() && variant.is_empty() => l.description.clone(),
        _ if variant.is_empty() => layout.to_string(),
        _ => format!("{layout} ({variant})"),
    };
    Described {
        short: short.to_uppercase(),
        description,
    }
}

/// `[keyboard] layout` and `variant` as pairs, lined up by position (missing variant: none). An empty layout is no pairs.
pub fn split(layout: &str, variant: &str) -> Vec<(String, String)> {
    if layout.trim().is_empty() {
        return Vec::new();
    }
    let mut variants = variant.split(',').map(str::trim);
    layout
        .split(',')
        .map(|l| {
            let v = variants.next().unwrap_or("");
            (l.trim().to_string(), v.to_string())
        })
        .collect()
}

/// The inverse of [`split`]; with no variant anywhere the variant string is empty, not a row of commas.
pub fn join(pairs: &[(String, String)]) -> (String, String) {
    let layout = pairs
        .iter()
        .map(|(l, _)| l.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let variant = if pairs.iter().all(|(_, v)| v.is_empty()) {
        String::new()
    } else {
        pairs
            .iter()
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    (layout, variant)
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The contents of every top-level `<tag>` element in `s`; the registry never nests a name in itself.
fn blocks<'a>(s: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        // `<layout` also starts `<layoutList`: only `>` or an attribute may follow the name.
        if !after.starts_with('>') && !after.starts_with(char::is_whitespace) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(&body[..end]);
        rest = &body[end + close.len()..];
    }
    out
}

fn text(s: &str, tag: &str) -> Option<String> {
    let body = blocks(s, tag).into_iter().next()?;
    Some(
        body.trim()
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<xkbConfigRegistry version="1.1">
  <modelList>
    <model><configItem><name>pc105</name><description>Generic 105-key PC</description></configItem></model>
  </modelList>
  <layoutList>
    <layout>
      <configItem>
        <name>us</name>
        <!-- Keyboard indicator for English layouts -->
        <shortDescription>en</shortDescription>
        <description>English (US)</description>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>intl</name>
            <description>English (US, intl., with dead keys)</description>
          </configItem>
        </variant>
        <variant>
          <configItem popularity="exotic">
            <name>chr</name>
            <shortDescription>chr</shortDescription>
            <description>Cherokee &lt;x&gt; &amp; y</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>hu</name>
        <shortDescription>hu</shortDescription>
        <description>Hungarian</description>
      </configItem>
    </layout>
  </layoutList>
  <optionList>
    <group><configItem><name>grp</name><description>Switching</description></configItem></group>
  </optionList>
</xkbConfigRegistry>
"#;

    #[test]
    fn the_layout_list_parses_with_its_variants() {
        let reg = parse(XML);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg[0].name, "us");
        assert_eq!(reg[0].short, "en");
        assert_eq!(reg[0].description, "English (US)");
        assert_eq!(reg[0].variants.len(), 2);
        assert_eq!(reg[0].variants[0].name, "intl");
        assert_eq!(
            reg[0].variants[0].short, "",
            "a variant's short name is its own"
        );
        assert_eq!(reg[0].variants[1].description, "Cherokee <x> & y");
        assert_eq!(reg[1].name, "hu");
        assert!(reg[1].variants.is_empty());
    }

    #[test]
    fn describe_takes_the_variant_first_and_falls_back_to_the_code() {
        let reg = parse(XML);
        let d = describe(&reg, "us", "");
        assert_eq!(
            (d.short.as_str(), d.description.as_str()),
            ("EN", "English (US)")
        );
        let d = describe(&reg, "us", "intl");
        assert_eq!(d.short, "EN", "no short name of its own: the layout's");
        assert_eq!(d.description, "English (US, intl., with dead keys)");
        assert_eq!(describe(&reg, "us", "chr").short, "CHR");
        let d = describe(&reg, "zz", "");
        assert_eq!((d.short.as_str(), d.description.as_str()), ("ZZ", "zz"));
        assert_eq!(describe(&reg, "hu", "nope").description, "hu (nope)");
    }

    #[test]
    fn layout_and_variant_split_by_position_and_join_back() {
        assert!(split("", "").is_empty());
        let pairs = split("us, hu", ",102_qwerty_dot_dead");
        assert_eq!(
            pairs,
            vec![
                ("us".to_string(), String::new()),
                ("hu".to_string(), "102_qwerty_dot_dead".to_string())
            ]
        );
        assert_eq!(
            join(&pairs),
            ("us,hu".to_string(), ",102_qwerty_dot_dead".to_string())
        );
        let plain = split("us,hu", "");
        assert_eq!(join(&plain), ("us,hu".to_string(), String::new()));
        assert_eq!(join(&[]), (String::new(), String::new()));
    }

    #[test]
    fn the_installed_registry_knows_us_and_hu() {
        let reg = load("");
        if reg.is_empty() {
            return;
        }
        assert_eq!(describe(&reg, "us", "").short, "EN");
        assert_eq!(describe(&reg, "hu", "").description, "Hungarian");
    }
}
