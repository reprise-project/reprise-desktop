//! The JPEG XL codec: `libreprise_encode_jxl.so`, the only object that links libjxl, loaded before
//! the helper confines itself. When it does not load, JPEG XL requests get
//! [`Reply::Unavailable`] and other formats are unaffected.

use std::ffi::{CStr, CString, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::{Jxl, Reply};

pub const LIBRARY: &str = "libreprise_encode_jxl.so";
/// Where the package installs [`LIBRARY`]; a build tree keeps it beside `reprise-encode`.
pub const INSTALLED: &str = "/usr/lib/reprise";
const SYMBOL: &CStr = c"reprise_encode_jxl_v1";

/// Takes the file, or the error text, once.
pub type Sink = unsafe extern "C" fn(ctx: *mut c_void, bytes: *const u8, len: usize);

/// `reprise_encode_jxl_v1` of [`LIBRARY`]: returns [`crate::REPLY_FILE`] after passing the file
/// to `sink`, [`crate::REPLY_MEMORY`] past `budget`, or [`crate::REPLY_CODEC`] after passing the
/// error text to `sink`.
pub type EncodeFn = unsafe extern "C" fn(
    rgb: *const u8,
    w: u32,
    h: u32,
    lossless: bool,
    quality: u8,
    effort: u8,
    budget: u64,
    threads: u32,
    sink: Sink,
    ctx: *mut c_void,
) -> u8;

pub struct JxlCodec(EncodeFn);

impl JxlCodec {
    /// The codec from beside the running executable, else from [`INSTALLED`]; the error names
    /// each path tried with the loader's message for it.
    pub fn load() -> Result<JxlCodec, String> {
        let mut tried = Vec::new();
        for path in candidates() {
            match open(&path) {
                Ok(f) => return Ok(JxlCodec(f)),
                Err(why) => tried.push(tried_line(&path, why)),
            }
        }
        Err(tried.join("; "))
    }

    /// # Safety
    /// `f` has [`EncodeFn`]'s signature and contract.
    pub unsafe fn from_fn(f: EncodeFn) -> JxlCodec {
        JxlCodec(f)
    }

    /// `rgb`, `w` by `h`, as a JPEG XL.
    pub fn encode(&self, rgb: &[u8], w: u32, h: u32, s: Jxl, budget: u64, threads: u32) -> Reply {
        unsafe extern "C" fn collect(ctx: *mut c_void, bytes: *const u8, len: usize) {
            // SAFETY: ctx is the Vec passed below, and the codec passes `len` live bytes.
            unsafe {
                let out = &mut *ctx.cast::<Vec<u8>>();
                out.extend_from_slice(std::slice::from_raw_parts(bytes, len));
            }
        }
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(Some(rgb.len()), (w as usize * h as usize).checked_mul(3));
        // SAFETY: the pointer has EncodeFn's contract; rgb holds w * h * 3 bytes and `out`
        // outlives the call.
        let code = unsafe {
            (self.0)(
                rgb.as_ptr(),
                w,
                h,
                s.lossless,
                s.quality,
                s.effort,
                budget,
                threads,
                collect,
                (&raw mut out).cast(),
            )
        };
        match code {
            crate::REPLY_FILE => Reply::File(out),
            crate::REPLY_MEMORY => Reply::Memory,
            crate::REPLY_CODEC => Reply::Codec(String::from_utf8_lossy(&out).into_owned()),
            other => Reply::Codec(format!("the JPEG XL codec returned {other}")),
        }
    }
}

fn candidates() -> Vec<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(LIBRARY)));
    beside
        .into_iter()
        .chain([Path::new(INSTALLED).join(LIBRARY)])
        .collect()
}

/// `path: why`, or `why` alone when the loader's message already starts with the path (a missing
/// file's does).
fn tried_line(path: &Path, why: String) -> String {
    let shown = path.display().to_string();
    if why.starts_with(&shown) {
        why
    } else {
        format!("{shown}: {why}")
    }
}

fn open(path: &Path) -> Result<EncodeFn, String> {
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: c is NUL-terminated; the library stays loaded for the life of the process.
    let lib = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if lib.is_null() {
        return Err(dlerror());
    }
    // SAFETY: lib is a live handle and SYMBOL NUL-terminated.
    let f = unsafe { libc::dlsym(lib, SYMBOL.as_ptr()) };
    if f.is_null() {
        return Err(dlerror());
    }
    // SAFETY: the library exports SYMBOL with EncodeFn's signature (pinned by a test against the
    // codec crate); data and function pointers share a size on Linux.
    Ok(unsafe { std::mem::transmute::<*mut c_void, EncodeFn>(f) })
}

fn dlerror() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated message valid until the next dl call.
    let e = unsafe { libc::dlerror() };
    if e.is_null() {
        return "unknown loader error".into();
    }
    // SAFETY: as above.
    unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_type_is_the_codecs_signature() {
        let _: EncodeFn = reprise_encode_jxl::reprise_encode_jxl_v1;
    }

    #[test]
    fn a_codec_that_does_not_load_names_each_path_and_why() {
        let dir = std::env::temp_dir().join(format!("reprise-encode-codec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(LIBRARY);
        std::fs::write(&fake, b"not an ELF").unwrap();
        let Err(why) = open(&fake) else {
            panic!("a text file is not a library")
        };
        assert!(why.contains(&*fake.to_string_lossy()), "{why}");
        let missing = dir.join("missing.so");
        let Err(why) = open(&missing) else {
            panic!("no file")
        };
        assert!(why.contains("No such file"), "{why}");
        let line = tried_line(&missing, why);
        assert_eq!(
            line.matches(&*missing.to_string_lossy()).count(),
            1,
            "the path appears once: {line}"
        );
        assert_eq!(
            tried_line(Path::new("/a.so"), "libjxl.so.0.13: cannot open".into()),
            "/a.so: libjxl.so.0.13: cannot open"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
