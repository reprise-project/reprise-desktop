//! NVIDIA GPU use through NVML, which ships with the driver as `libnvidia-ml.so.1`. The library
//! is opened at run time; without it a card shows what sysfs and fdinfo give.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_ulonglong, c_void};

type Device = *mut c_void;
type Ret = c_int;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: c_ulonglong,
    free: c_ulonglong,
    used: c_ulonglong,
}

struct Fns {
    by_pci: unsafe extern "C" fn(*const c_char, *mut Device) -> Ret,
    utilization: unsafe extern "C" fn(Device, *mut Utilization) -> Ret,
    memory: unsafe extern "C" fn(Device, *mut Memory) -> Ret,
    temperature: unsafe extern "C" fn(Device, c_int, *mut c_uint) -> Ret,
    fan: unsafe extern "C" fn(Device, *mut c_uint) -> Ret,
    name: unsafe extern "C" fn(Device, *mut c_char, c_uint) -> Ret,
}

/// An initialised NVML, open until the program ends. Only the sampling thread holds it.
pub struct Nvml {
    fns: Fns,
}

// SAFETY: NVML is thread-safe, and the handle moves to the sampling thread once and stays there.
unsafe impl Send for Nvml {}

/// What NVML reports for one card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub name: Option<String>,
    pub busy: Option<f64>,
    pub vram: Option<(u64, u64)>,
    pub temp: Option<f64>,
    /// The fan's speed in percent of its maximum.
    pub fan: Option<u32>,
}

/// Symbol `name` of `lib` as a function pointer of type `F`.
///
/// # Safety
/// `lib` is a live handle, and `F` is a function pointer with the symbol's true signature.
unsafe fn sym<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    // SAFETY: lib is a live handle and name a valid C string.
    let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller names F as the symbol's type; a function pointer is pointer-sized.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

impl Nvml {
    /// Opens and initialises NVML; None without the library or a driver to talk to.
    pub fn open() -> Option<Nvml> {
        let path = c"libnvidia-ml.so.1";
        // SAFETY: the path is a valid C string; dlopen returns null on failure.
        let lib = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if lib.is_null() {
            return None;
        }
        let found = (|| {
            // SAFETY: each field's type is its function's signature in nvml.h.
            let (init, fns) = unsafe {
                let init: unsafe extern "C" fn() -> Ret = sym(lib, c"nvmlInit_v2")?;
                let fns = Fns {
                    by_pci: sym(lib, c"nvmlDeviceGetHandleByPciBusId_v2")?,
                    utilization: sym(lib, c"nvmlDeviceGetUtilizationRates")?,
                    memory: sym(lib, c"nvmlDeviceGetMemoryInfo")?,
                    temperature: sym(lib, c"nvmlDeviceGetTemperature")?,
                    fan: sym(lib, c"nvmlDeviceGetFanSpeed")?,
                    name: sym(lib, c"nvmlDeviceGetName")?,
                };
                (init, fns)
            };
            // SAFETY: nvmlInit_v2 takes nothing and returns a status.
            (unsafe { init() } == 0).then_some(fns)
        })();
        match found {
            Some(fns) => Some(Nvml { fns }),
            None => {
                // SAFETY: lib came from dlopen and nothing from it is kept.
                unsafe { libc::dlclose(lib) };
                None
            }
        }
    }

    /// The card at PCI address `pci` (`0000:01:00.0`), or None when NVML does not know it.
    pub fn card(&self, pci: &str) -> Option<Card> {
        let id = CString::new(pci).ok()?;
        let mut dev: Device = std::ptr::null_mut();
        // SAFETY: id is a valid C string and dev a valid out pointer.
        if unsafe { (self.fns.by_pci)(id.as_ptr(), &mut dev) } != 0 {
            return None;
        }
        let mut util = Utilization::default();
        let mut mem = Memory::default();
        let (mut temp, mut fan) = (0, 0);
        let mut name = [0 as c_char; 96];
        // SAFETY: dev is the handle NVML gave and every out pointer is valid for its type.
        let ok = unsafe {
            [
                (self.fns.utilization)(dev, &mut util) == 0,
                (self.fns.memory)(dev, &mut mem) == 0,
                (self.fns.temperature)(dev, 0, &mut temp) == 0,
                (self.fns.fan)(dev, &mut fan) == 0,
                (self.fns.name)(dev, name.as_mut_ptr(), name.len() as c_uint) == 0,
            ]
        };
        Some(Card {
            name: ok[4].then(|| {
                // SAFETY: on success NVML wrote a NUL-terminated name into the buffer.
                let n = unsafe { CStr::from_ptr(name.as_ptr()) };
                n.to_string_lossy().into_owned()
            }),
            busy: ok[0].then(|| f64::from(util.gpu) / 100.0),
            vram: ok[1].then_some((mem.used, mem.total)),
            temp: ok[2].then_some(f64::from(temp)),
            fan: ok[3].then_some(fan),
        })
    }
}
