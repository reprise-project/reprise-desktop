//! `org.freedesktop.impl.portal.FileChooser`, run as our own child (`reprise-portal --file`).
//! The child answers an `Output`; paths cross the pipe as bytes so any file name survives.

use crate::choices::{self, Choice};
use crate::dialog;
use crate::filters::{self, Filter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::{ObjectPath, OwnedVariant, Variant};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open,
    Save,
    SaveFiles,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Input {
    pub parent: Option<String>,
    pub modal: bool,
    /// Always set; an `Option` only so `Input` has a `Default`.
    pub kind: Option<Kind>,
    pub folder_mode: bool,
    pub multiple: bool,
    pub title: String,
    pub accept_label: String,
    pub filters: Vec<Filter>,
    pub filter: usize,
    pub start: Vec<u8>,
    pub name: String,
    /// `name`'s real bytes, when it is not UTF-8.
    pub name_raw: Option<Vec<u8>>,
    /// SaveFiles' names, as bytes with their trailing NUL.
    pub files: Vec<Vec<u8>>,
    pub choices: Vec<Choice>,
}

/// What the dialog child answers. The default (nothing picked) is a
/// cancel, which is also what a killed or failed child reads as.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Output {
    pub paths: Vec<Vec<u8>>,
    pub filter: Option<usize>,
    pub choices: Vec<(String, String)>,
}

pub fn bytes_path(v: &[u8]) -> PathBuf {
    let v = v.strip_suffix(b"\0").unwrap_or(v);
    PathBuf::from(OsStr::from_bytes(v))
}

/// `path` as a `file://` URI: every byte outside RFC 3986's unreserved set
/// and `/` percent-encoded.
pub fn uri(path: &Path) -> String {
    let mut s = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The folders a call asks to start in, most specific first:
/// `current_file`'s folder, then `current_folder`.
fn asked_folders(o: &HashMap<String, Variant<'_>>) -> Vec<PathBuf> {
    let current_file_dir =
        opt_bytes(o, "current_file").and_then(|b| bytes_path(&b).parent().map(Path::to_path_buf));
    let current_folder = opt_bytes(o, "current_folder").map(|b| bytes_path(&b));
    current_file_dir.into_iter().chain(current_folder).collect()
}

/// The first of `asked`, Documents and home that is a folder. Blocking:
/// a caller's path may sit on a hung mount.
fn start_in(asked: Vec<PathBuf>) -> PathBuf {
    let mut candidates: Vec<Option<PathBuf>> = asked.into_iter().map(Some).collect();
    candidates.push(reprisefs::places::user_dir("DOCUMENTS", "Documents"));
    candidates.push(std::env::var_os("HOME").map(PathBuf::from));
    start_folder(&candidates)
}

pub fn start_folder(candidates: &[Option<PathBuf>]) -> PathBuf {
    candidates
        .iter()
        .flatten()
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// SaveFiles' answer: each of `files` in `dir`, a name taken on disk or
/// by an earlier name numbered to the next free one. None when a name is
/// empty, `.`, `..`, or has a `/` or a NUL: it would leave the folder the
/// user picked.
pub fn save_files_paths(dir: &Path, files: &[Vec<u8>]) -> Option<Vec<PathBuf>> {
    if !save_files_names_ok(files) {
        return None;
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for f in files {
        let name = f.strip_suffix(b"\0").unwrap_or(f);
        // Taken on disk (a dangling symlink too) or already handed out
        // for an earlier name of this call.
        let free = reprisefs::names::first_free(OsStr::from_bytes(name), |c| {
            let p = dir.join(c);
            std::fs::symlink_metadata(&p).is_ok() || out.contains(&p)
        });
        out.push(dir.join(free));
    }
    Some(out)
}

fn save_files_names_ok(files: &[Vec<u8>]) -> bool {
    files.iter().all(|f| {
        let name = f.strip_suffix(b"\0").unwrap_or(f);
        let ok = !(name.is_empty()
            || name == b"."
            || name == b".."
            || name.contains(&b'/')
            || name.contains(&0));
        if !ok {
            log::warn!(
                "SaveFiles: refused the name {:?}",
                String::from_utf8_lossy(name)
            );
        }
        ok
    })
}

fn reply(out: &Output, filters: &[Filter]) -> (u32, HashMap<String, OwnedVariant>) {
    let mut r = HashMap::new();
    if out.paths.is_empty() {
        return (crate::RESPONSE_CANCELLED, r);
    }
    let uris: Vec<String> = out
        .paths
        .iter()
        .map(|p| uri(Path::new(OsStr::from_bytes(p))))
        .collect();
    r.insert(
        "uris".into(),
        Variant::new(uris).try_to_owned().expect("strings"),
    );
    if let Some(f) = out.filter.and_then(|i| filters.get(i)) {
        r.insert("current_filter".into(), f.to_value());
    }
    if !out.choices.is_empty() {
        r.insert(
            "choices".into(),
            Variant::new(out.choices.clone())
                .try_to_owned()
                .expect("strings"),
        );
    }
    (crate::RESPONSE_OK, r)
}

pub struct FileChooser {
    pub frontend: crate::Frontend,
}

/// SaveFile's starting name: `current_file`'s file name, else
/// `current_name`; and the real bytes when the name is not UTF-8, so a
/// Save keeps them.
fn start_name(o: &HashMap<String, Variant<'_>>) -> (String, Option<Vec<u8>>) {
    if let Some(name) = opt_bytes(o, "current_file")
        .and_then(|b| bytes_path(&b).file_name().map(|n| n.to_os_string()))
    {
        let raw = name.to_str().is_none().then(|| name.as_bytes().to_vec());
        return (name.to_string_lossy().into_owned(), raw);
    }
    (opt_str(o, "current_name").unwrap_or_default(), None)
}

fn opt_str(o: &HashMap<String, Variant<'_>>, k: &str) -> Option<String> {
    match o.get(k) {
        Some(Variant::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn opt_bool(o: &HashMap<String, Variant<'_>>, k: &str) -> Option<bool> {
    match o.get(k) {
        Some(Variant::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn opt_bytes(o: &HashMap<String, Variant<'_>>, k: &str) -> Option<Vec<u8>> {
    let Some(Variant::Array(a)) = o.get(k) else {
        return None;
    };
    a.inner()
        .iter()
        .map(|v| match v {
            Variant::U8(b) => Some(*b),
            _ => None,
        })
        .collect()
}

fn base_input(
    kind: Kind,
    parent_window: &str,
    title: String,
    o: &HashMap<String, Variant<'_>>,
) -> Input {
    let folder_mode =
        kind == Kind::SaveFiles || (kind == Kind::Open && opt_bool(o, "directory") == Some(true));
    let default_label = match (kind, folder_mode) {
        (_, true) => "&Select Folder",
        (Kind::Open, _) => "&Open",
        _ => "&Save",
    };
    let (filters, filter) = filters::with_current(
        o.get("filters")
            .map(filters::parse_filters)
            .unwrap_or_default(),
        o.get("current_filter").and_then(filters::parse_filter),
    );
    Input {
        parent: dialog::parent_handle(parent_window),
        modal: opt_bool(o, "modal").unwrap_or(true),
        kind: Some(kind),
        folder_mode,
        multiple: kind == Kind::Open && !folder_mode && opt_bool(o, "multiple") == Some(true),
        title,
        accept_label: opt_str(o, "accept_label")
            .map_or(default_label.to_string(), |l| dialog::mnemonic(&l)),
        filters: if folder_mode { Vec::new() } else { filters },
        filter,
        start: Vec::new(),
        name: String::new(),
        name_raw: None,
        files: Vec::new(),
        choices: o
            .get("choices")
            .map(choices::parse_choices)
            .unwrap_or_default(),
    }
}

impl FileChooser {
    async fn run(
        &self,
        server: &zbus::ObjectServer,
        handle: &ObjectPath<'_>,
        mut input: Input,
        asked: Vec<PathBuf>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, handle, cancel.clone(), &self.frontend).await?;
        let filters = input.filters.clone();
        let out: Output = blocking::unblock(move || {
            input.start = start_in(asked).into_os_string().into_vec();
            dialog::run_toml(&["--file"], &input, &cancel)
                .and_then(|s| {
                    toml::from_str(&s)
                        .map_err(|e| log::warn!("file answer: {e}"))
                        .ok()
                })
                .unwrap_or_default()
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        Ok(reply(&out, &filters))
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[allow(clippy::too_many_arguments)]
    async fn open_file(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        log::info!("OpenFile for {app_id:?}");
        self.run(
            server,
            &handle,
            base_input(Kind::Open, &parent_window, title, &options),
            asked_folders(&options),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_file(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        log::info!("SaveFile for {app_id:?}");
        let mut input = base_input(Kind::Save, &parent_window, title, &options);
        (input.name, input.name_raw) = start_name(&options);
        self.run(server, &handle, input, asked_folders(&options))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_files(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        log::info!("SaveFiles for {app_id:?}");
        let mut input = base_input(Kind::SaveFiles, &parent_window, title, &options);
        let files: Vec<Vec<u8>> = match options.get("files") {
            Some(Variant::Array(a)) => a
                .inner()
                .iter()
                .filter_map(|v| match v {
                    Variant::Array(b) => b
                        .inner()
                        .iter()
                        .map(|x| match x {
                            Variant::U8(c) => Some(*c),
                            _ => None,
                        })
                        .collect(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if !save_files_names_ok(&files) {
            return Ok((crate::RESPONSE_OTHER, HashMap::new()));
        }
        input.files = files;
        self.run(server, &handle, input, asked_folders(&options))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn a_path_from_ay_loses_one_trailing_nul() {
        assert_eq!(bytes_path(b"/tmp/a\0"), PathBuf::from("/tmp/a"));
        assert_eq!(bytes_path(b"/tmp/a"), PathBuf::from("/tmp/a"));
        let latin1 = bytes_path(b"/tmp/caf\xe9\0");
        assert_eq!(latin1.as_os_str().as_bytes(), b"/tmp/caf\xe9");
    }

    #[test]
    fn uri_encodes_spaces_marks_and_percent() {
        assert_eq!(
            uri(Path::new("/home/u/My file #1 100%.txt")),
            "file:///home/u/My%20file%20%231%20100%25.txt"
        );
        assert_eq!(uri(Path::new("/a-b_c.d~e")), "file:///a-b_c.d~e");
    }

    #[test]
    fn uri_encodes_non_utf8_bytes() {
        let p = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9"));
        assert_eq!(uri(&p), "file:///tmp/caf%E9");
    }

    #[test]
    fn output_paths_round_trip_non_utf8() {
        let out = Output {
            paths: vec![b"/tmp/caf\xe9".to_vec()],
            filter: Some(1),
            choices: vec![],
        };
        let back: Output = toml::from_str(&toml::to_string(&out).unwrap()).unwrap();
        assert_eq!(back, out);
    }

    #[test]
    fn input_round_trips() {
        let input = Input {
            parent: Some("h".into()),
            modal: true,
            kind: Some(Kind::Save),
            title: "Save \"x\"\n".into(),
            accept_label: "_Save".into(),
            start: b"/tmp\0".to_vec(),
            name: "a.txt".into(),
            ..Default::default()
        };
        let back: Input = toml::from_str(&toml::to_string(&input).unwrap()).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn a_byte_array_option_is_read_off_the_wire() {
        use zbus::wire::{LE, serialized::Context, to_bytes};
        let mut o: HashMap<String, Variant<'_>> = HashMap::new();
        o.insert("current_folder".into(), Variant::new(b"/tmp\0".to_vec()));
        let ctxt = Context::new(LE, 0);
        let wire = to_bytes(ctxt, &o).unwrap();
        let (back, _): (HashMap<String, Variant<'_>>, _) = wire.deserialize().unwrap();
        assert_eq!(opt_bytes(&back, "current_folder"), Some(b"/tmp\0".to_vec()));
    }

    #[test]
    fn the_start_folder_is_the_first_that_exists() {
        let t = std::env::temp_dir();
        let missing = t.join("reprise-no-such-folder");
        assert_eq!(start_folder(&[Some(missing), None, Some(t.clone())]), t);
    }

    #[test]
    fn the_current_files_folder_is_asked_before_the_current_folder() {
        let mut o: HashMap<String, Variant<'_>> = HashMap::new();
        o.insert("current_folder".into(), Variant::new(b"/srv\0".to_vec()));
        o.insert(
            "current_file".into(),
            Variant::new(b"/tmp/a.txt\0".to_vec()),
        );
        assert_eq!(
            asked_folders(&o),
            [PathBuf::from("/tmp"), PathBuf::from("/srv")]
        );
        assert!(asked_folders(&HashMap::new()).is_empty());
    }

    #[test]
    fn save_files_numbers_taken_names() {
        let t = std::env::temp_dir().join(format!("reprise-savefiles-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        let got = save_files_paths(&t, &[b"a.txt\0".to_vec(), b"b.txt\0".to_vec()]).unwrap();
        assert_eq!(got, vec![t.join("a (2).txt"), t.join("b.txt")]);
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn save_files_numbers_repeated_names_apart() {
        let t = std::env::temp_dir().join(format!("reprise-savefiles-dup-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        let got = save_files_paths(
            &t,
            &[
                b"a.txt\0".to_vec(),
                b"a.txt\0".to_vec(),
                b"a (2).txt\0".to_vec(),
            ],
        )
        .unwrap();
        assert_eq!(
            got,
            vec![
                t.join("a (2).txt"),
                t.join("a (3).txt"),
                t.join("a (2) (2).txt")
            ]
        );
        let mut unique = got.clone();
        unique.dedup();
        assert_eq!(unique.len(), 3, "no two answers share a path");
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_start_name_that_is_not_utf8_keeps_its_bytes() {
        let mut o: HashMap<String, Variant<'_>> = HashMap::new();
        o.insert(
            "current_file".into(),
            Variant::new(b"/tmp/caf\xe9.pdf\0".to_vec()),
        );
        let (shown, raw) = start_name(&o);
        assert_eq!(shown, "caf\u{FFFD}.pdf");
        assert_eq!(raw, Some(b"caf\xe9.pdf".to_vec()));
        let mut o: HashMap<String, Variant<'_>> = HashMap::new();
        o.insert("current_name".into(), Variant::from("plain.txt"));
        assert_eq!(start_name(&o), ("plain.txt".to_string(), None));
    }

    #[test]
    fn save_files_refuses_path_names() {
        let t = std::env::temp_dir();
        for bad in [&b"../x"[..], b"a/b", b".", b"..", b"", b"a\0b"] {
            assert_eq!(
                save_files_paths(&t, &[bad.to_vec()]),
                None,
                "{:?} is refused",
                bad
            );
        }
    }

    #[test]
    fn replies_carry_the_bus_types() {
        let filters = vec![crate::filters::Filter {
            label: "T".into(),
            patterns: vec![crate::filters::Pattern::Glob("*.t".into())],
        }];
        let out = Output {
            paths: vec![b"/tmp/a b".to_vec()],
            filter: Some(0),
            choices: vec![("c".into(), "true".into())],
        };
        let (code, r) = reply(&out, &filters);
        assert_eq!(code, crate::RESPONSE_OK);
        assert_eq!(r["uris"].value_signature().to_string(), "as");
        assert_eq!(
            r["current_filter"].value_signature().to_string(),
            "(sa(us))"
        );
        assert_eq!(r["choices"].value_signature().to_string(), "a(ss)");
        let (code, r) = reply(&Output::default(), &filters);
        assert_eq!(code, crate::RESPONSE_CANCELLED);
        assert!(r.is_empty());
    }

    #[test]
    fn a_call_without_filters_gets_no_filter_back() {
        let input = base_input(Kind::Open, "", "Open".into(), &HashMap::new());
        assert!(input.filters.is_empty(), "no filter is made up");
        let out = Output {
            paths: vec![b"/tmp/a".to_vec()],
            filter: Some(0),
            choices: vec![],
        };
        let (code, r) = reply(&out, &input.filters);
        assert_eq!(code, crate::RESPONSE_OK);
        assert!(!r.contains_key("current_filter"));
    }
}
