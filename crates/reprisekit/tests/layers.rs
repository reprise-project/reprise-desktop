//! reprisekit's prod code reaches only down its layers.
//!
//! Layers are the top-level modules of `src`. The test parses every prod
//! file (skipping `#[cfg(test)]` items), resolves `crate::`, `super::` and
//! `self::` paths (in `use` trees, types, expressions, patterns and macro
//! tokens) to their top-level module, and fails on a reference that points up.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use syn::buffer::{Cursor, TokenBuffer};
use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, Meta, UseTree};

/// The layers above core: (layer, its top-level modules, the layers below it).
/// A layer may reference itself and those below; modules missing here are core.
/// `lib` is items defined in lib.rs; `interfaces` builds each theme, so it sits
/// on top. Classic, Blue and Glass are siblings that may not reach each other.
/// `toolkit` sits with `look`: its GTK3 theme takes Classic's edges from Classic's painters.
#[rustfmt::skip]
const LAYERS: &[(&str, &[&str], &[&str])] = &[
    ("classic", &["classic"], &["core"]),
    ("blue", &["blue"], &["core"]),
    ("glass", &["glass"], &["core"]),
    ("look", &["look", "toolkit"], &["core", "classic"]),
    ("controls", &["controls"], &["core", "classic", "look"]),
    ("facade", &["lib", "interfaces"], &["core", "classic", "blue", "glass", "look", "controls"]),
];

/// Upward references let through: (file under `src`, resolved path). Empty today.
/// An entry needs a comment saying why it cannot move; the test fails once it no longer occurs.
#[rustfmt::skip]
const ALLOWED_UP: &[(&str, &str)] = &[];

#[test]
fn prod_code_references_only_its_own_layer_and_below() {
    let found: BTreeSet<(String, String)> = crate_refs()
        .into_iter()
        .filter(|(file, path)| is_upward(file, path))
        .collect();
    let allowed: BTreeSet<(String, String)> = ALLOWED_UP
        .iter()
        .map(|&(f, p)| (f.to_string(), p.to_string()))
        .collect();

    let mut msg = String::new();
    for (file, path) in found.difference(&allowed) {
        let (from, to) = (file_layer(file), path_layer(path));
        msg += &format!("  {file}: {path} ({from} -> {to})\n");
    }
    if !msg.is_empty() {
        msg.insert_str(0, "prod code references a higher layer:\n");
    }
    let stale: Vec<_> = allowed.difference(&found).collect();
    if !stale.is_empty() {
        msg += "ALLOWED_UP entries that no longer occur (delete them):\n";
        for (file, path) in stale {
            msg += &format!("  (\"{file}\", \"{path}\")\n");
        }
    }
    assert!(msg.is_empty(), "{msg}");
}

fn layer_of(top: &str) -> &'static str {
    LAYERS
        .iter()
        .find(|(_, mods, _)| mods.contains(&top))
        .map_or("core", |l| l.0)
}

fn below(layer: &str) -> &'static [&'static str] {
    LAYERS.iter().find(|l| l.0 == layer).map_or(&[], |l| l.2)
}

fn file_layer(file: &str) -> &'static str {
    let top = file.split('/').next().unwrap();
    layer_of(top.strip_suffix(".rs").unwrap_or(top))
}

fn path_layer(path: &str) -> &'static str {
    layer_of(path.split("::").next().unwrap())
}

fn is_upward(file: &str, path: &str) -> bool {
    let (from, to) = (file_layer(file), path_layer(path));
    from != to && !below(from).contains(&to)
}

fn crate_refs() -> BTreeSet<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = scan_all(&src);
    // A path to a top-level module named in lib.rs is not an item of lib.rs.
    let tops: BTreeSet<String> = files[&src.join("lib.rs")]
        .children
        .iter()
        .map(|c| module_of(&src, c).remove(0))
        .collect();
    let mut refs = BTreeSet::new();
    let mut todo = vec![src.join("lib.rs")];
    while let Some(file) = todo.pop() {
        let scan = files
            .remove(&file)
            .unwrap_or_else(|| panic!("no {}", file.display()));
        let rel = file.strip_prefix(&src).unwrap().to_str().unwrap();
        for path in scan.refs {
            let top = path.split("::").next().unwrap();
            let path = if tops.contains(top) {
                path
            } else {
                format!("lib::{path}")
            };
            refs.insert((rel.to_string(), path));
        }
        todo.extend(scan.children);
    }
    refs
}

fn scan_all(src: &Path) -> HashMap<PathBuf, Scan> {
    let mut paths = Vec::new();
    let mut dirs = vec![src.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                paths.push(path);
            }
        }
    }
    let next = AtomicUsize::new(0);
    let threads = thread::available_parallelism().map_or(4, |n| n.get());
    thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        out.push((path.clone(), scan(src, path)));
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    })
}

fn module_of(src: &Path, file: &Path) -> Vec<String> {
    let rel = file.strip_prefix(src).unwrap().with_extension("");
    let mut module: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_str().unwrap().to_string())
        .collect();
    if matches!(module.last().map(String::as_str), Some("mod" | "lib")) {
        module.pop();
    }
    module
}

struct Scan {
    refs: BTreeSet<String>,
    children: Vec<PathBuf>,
}

fn scan(src: &Path, file: &Path) -> Scan {
    let text = drop_number_rows(&fs::read_to_string(file).unwrap());
    let mut ast = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let module = module_of(src, file);
    // `mod x;` in lib.rs or a mod.rs loads a sibling; in a.rs it loads a/x.
    let dir = match file.file_stem().unwrap().to_str() {
        Some("lib" | "mod") => file.parent().unwrap().to_path_buf(),
        _ => file.with_extension(""),
    };
    let mut v = Refs {
        module,
        dir,
        scan: Scan {
            refs: BTreeSet::new(),
            children: Vec::new(),
        },
    };
    v.visit_file_mut(&mut ast);
    v.scan
}

/// Blanks lines holding only numbers and commas (the generated art tables), which name no path and are slow to parse.
fn drop_number_rows(text: &str) -> String {
    let number = |t: &str| {
        t.starts_with(|c: char| c.is_ascii_digit())
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    };
    text.lines()
        .map(|line| {
            let mut items = line
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .peekable();
            if items.peek().is_some() && items.all(number) {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| match &a.meta {
        Meta::List(l) => l.path.is_ident("cfg") && l.tokens.to_string() == "test",
        _ => false,
    })
}

struct Refs {
    module: Vec<String>,
    dir: PathBuf,
    scan: Scan,
}

impl Refs {
    fn add(&mut self, segs: &[String]) {
        let mut module = match segs.first().map(String::as_str) {
            Some("crate") => Vec::new(),
            Some("super" | "self") => self.module.clone(),
            _ => return,
        };
        let mut rest = &segs[usize::from(segs[0] == "crate")..];
        while let Some(head) = rest.first() {
            match head.as_str() {
                "super" => drop(module.pop()),
                "self" => {}
                _ => break,
            }
            rest = &rest[1..];
        }
        if rest.is_empty() && module.is_empty() {
            return;
        }
        module.extend(rest.iter().cloned());
        self.scan.refs.insert(module.join("::"));
    }

    fn use_tree(&mut self, prefix: &mut Vec<String>, tree: &UseTree) {
        match tree {
            UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.use_tree(prefix, &p.tree);
                prefix.pop();
            }
            UseTree::Name(n) if n.ident == "self" => self.add(prefix),
            UseTree::Name(n) => self.leaf(prefix, &n.ident),
            UseTree::Rename(r) if r.ident == "self" => self.add(prefix),
            UseTree::Rename(r) => self.leaf(prefix, &r.ident),
            UseTree::Glob(_) => self.add(prefix),
            UseTree::Group(g) => g.items.iter().for_each(|t| self.use_tree(prefix, t)),
        }
    }

    fn leaf(&mut self, prefix: &mut Vec<String>, ident: &syn::Ident) {
        prefix.push(ident.to_string());
        self.add(prefix);
        prefix.pop();
    }

    fn tokens(&mut self, mut c: Cursor<'_>) {
        while !c.eof() {
            if let Some((inner, _, _, next)) = c.any_group() {
                self.tokens(inner);
                c = next;
                continue;
            }
            if let Some((ident, next)) = c.ident() {
                let mut segs = vec![ident.to_string()];
                let mut c2 = next;
                while let Some((id, after)) = path_sep(c2).and_then(Cursor::ident) {
                    segs.push(id.to_string());
                    c2 = after;
                }
                if segs.len() > 1 {
                    self.add(&segs);
                }
                c = c2;
                continue;
            }
            c = c.token_tree().unwrap().1;
        }
    }
}

fn path_sep(c: Cursor<'_>) -> Option<Cursor<'_>> {
    let (a, c) = c.punct()?;
    let (b, c) = c.punct()?;
    (a.as_char() == ':' && b.as_char() == ':').then_some(c)
}

macro_rules! skip_tests {
    ($($visit:ident: $ty:ident),* $(,)?) => {$(
        fn $visit(&mut self, node: &mut syn::$ty) {
            if !is_test(&node.attrs) {
                visit_mut::$visit(self, node);
            }
        }
    )*};
}

impl VisitMut for Refs {
    skip_tests! {
        visit_item_const_mut: ItemConst,
        visit_item_enum_mut: ItemEnum,
        visit_item_fn_mut: ItemFn,
        visit_item_impl_mut: ItemImpl,
        visit_item_macro_mut: ItemMacro,
        visit_item_static_mut: ItemStatic,
        visit_item_struct_mut: ItemStruct,
        visit_item_trait_mut: ItemTrait,
        visit_item_type_mut: ItemType,
        visit_item_union_mut: ItemUnion,
        visit_impl_item_fn_mut: ImplItemFn,
        visit_impl_item_const_mut: ImplItemConst,
        visit_impl_item_type_mut: ImplItemType,
        visit_impl_item_macro_mut: ImplItemMacro,
        visit_trait_item_fn_mut: TraitItemFn,
        visit_trait_item_const_mut: TraitItemConst,
        visit_trait_item_type_mut: TraitItemType,
        visit_local_mut: Local,
        visit_stmt_macro_mut: StmtMacro,
        visit_field_mut: Field,
        visit_field_value_mut: FieldValue,
        visit_variant_mut: Variant,
        visit_arm_mut: Arm,
        visit_expr_assign_mut: ExprAssign,
        visit_expr_call_mut: ExprCall,
        visit_expr_macro_mut: ExprMacro,
        visit_expr_method_call_mut: ExprMethodCall,
    }

    fn visit_item_use_mut(&mut self, node: &mut syn::ItemUse) {
        if !is_test(&node.attrs) && node.leading_colon.is_none() {
            self.use_tree(&mut Vec::new(), &node.tree);
        }
    }

    fn visit_item_mod_mut(&mut self, node: &mut syn::ItemMod) {
        if is_test(&node.attrs) {
            return;
        }
        let dir = self.dir.join(node.ident.to_string());
        match &mut node.content {
            Some((_, items)) => {
                let outer = std::mem::replace(&mut self.dir, dir);
                self.module.push(node.ident.to_string());
                items.iter_mut().for_each(|i| self.visit_item_mut(i));
                self.module.pop();
                self.dir = outer;
            }
            None => {
                let file = dir.with_extension("rs");
                let file = if file.exists() {
                    file
                } else {
                    dir.join("mod.rs")
                };
                self.scan.children.push(file);
            }
        }
    }

    fn visit_path_mut(&mut self, node: &mut syn::Path) {
        if node.leading_colon.is_none() {
            let segs: Vec<String> = node.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.len() > 1 {
                self.add(&segs);
            }
        }
        visit_mut::visit_path_mut(self, node);
    }

    fn visit_visibility_mut(&mut self, _: &mut syn::Visibility) {}

    fn visit_macro_mut(&mut self, node: &mut syn::Macro) {
        self.visit_path_mut(&mut node.path);
        let buf = TokenBuffer::new2(node.tokens.clone());
        self.tokens(buf.begin());
    }
}
