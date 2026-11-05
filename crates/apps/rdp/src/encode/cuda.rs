//! GL textures copied on the GPU into CUDA frames (CUDA-OpenGL interop), through the CUDA
//! driver library loaded at run time.

use super::EncodeError;
use super::gl::Gl;
use super::hwmap::Ref;
use ffmpeg::{ffi, frame};
use std::ffi::{CStr, c_void};
use std::rc::Rc;
use std::sync::OnceLock;

type CuResult = i32;
type Ptr = *mut c_void;

/// `CU_GRAPHICS_REGISTER_FLAGS_READ_ONLY`.
const READ_ONLY: u32 = 1;
/// `CUmemorytype`.
const DEVICE: u32 = 2;
const ARRAY: u32 = 3;

/// `CUDA_MEMCPY2D`.
#[repr(C)]
#[derive(Default)]
struct Memcpy2D {
    src_x_in_bytes: usize,
    src_y: usize,
    src_memory_type: u32,
    src_host: usize,
    src_device: u64,
    src_array: usize,
    src_pitch: usize,
    dst_x_in_bytes: usize,
    dst_y: usize,
    dst_memory_type: u32,
    dst_host: usize,
    dst_device: u64,
    dst_array: usize,
    dst_pitch: usize,
    width_in_bytes: usize,
    height: usize,
}

/// libavutil's `AVCUDADeviceContext`.
#[repr(C)]
struct DeviceContext {
    context: Ptr,
    stream: Ptr,
    _internal: Ptr,
}

type ContextFn = unsafe extern "C" fn(Ptr) -> CuResult;
type PopFn = unsafe extern "C" fn(*mut Ptr) -> CuResult;
type RegisterFn = unsafe extern "C" fn(*mut Ptr, u32, u32, u32) -> CuResult;
type MapFn = unsafe extern "C" fn(u32, *mut Ptr, Ptr) -> CuResult;
type ArrayFn = unsafe extern "C" fn(*mut Ptr, Ptr, u32, u32) -> CuResult;
type CopyFn = unsafe extern "C" fn(*const Memcpy2D, Ptr) -> CuResult;
type NameFn = unsafe extern "C" fn(CuResult, *mut *const libc::c_char) -> CuResult;

struct Api {
    push: ContextFn,
    pop: PopFn,
    register: RegisterFn,
    unregister: ContextFn,
    map: MapFn,
    unmap: MapFn,
    array: ArrayFn,
    copy: CopyFn,
    error_name: NameFn,
}

fn api() -> Result<&'static Api, EncodeError> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load)
        .as_ref()
        .map_err(|e| EncodeError(e.clone()))
}

fn load() -> Result<Api, String> {
    // SAFETY: loading the system's CUDA driver library, kept loaded for the process; each
    // symbol is cast to its signature in cuda.h.
    unsafe {
        let lib = libc::dlopen(c"libcuda.so.1".as_ptr(), libc::RTLD_NOW);
        if lib.is_null() {
            return Err("no libcuda.so.1".into());
        }
        let sym = |name: &CStr| {
            let p = libc::dlsym(lib, name.as_ptr());
            match p.is_null() {
                true => Err(format!("libcuda has no {}", name.to_string_lossy())),
                false => Ok(p),
            }
        };
        Ok(Api {
            push: std::mem::transmute::<Ptr, ContextFn>(sym(c"cuCtxPushCurrent_v2")?),
            pop: std::mem::transmute::<Ptr, PopFn>(sym(c"cuCtxPopCurrent_v2")?),
            register: std::mem::transmute::<Ptr, RegisterFn>(sym(c"cuGraphicsGLRegisterImage")?),
            unregister: std::mem::transmute::<Ptr, ContextFn>(sym(
                c"cuGraphicsUnregisterResource",
            )?),
            map: std::mem::transmute::<Ptr, MapFn>(sym(c"cuGraphicsMapResources")?),
            unmap: std::mem::transmute::<Ptr, MapFn>(sym(c"cuGraphicsUnmapResources")?),
            array: std::mem::transmute::<Ptr, ArrayFn>(sym(
                c"cuGraphicsSubResourceGetMappedArray",
            )?),
            copy: std::mem::transmute::<Ptr, CopyFn>(sym(c"cuMemcpy2DAsync_v2")?),
            error_name: std::mem::transmute::<Ptr, NameFn>(sym(c"cuGetErrorName")?),
        })
    }
}

fn check(api: &Api, ret: CuResult, what: &str) -> Result<(), EncodeError> {
    if ret == 0 {
        return Ok(());
    }
    let mut name = std::ptr::null();
    // SAFETY: CUDA returns a static name or leaves the pointer null.
    let name = match unsafe { (api.error_name)(ret, &mut name) } == 0 && !name.is_null() {
        // SAFETY: a static NUL-terminated name.
        true => unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
        false => format!("error {ret}"),
    };
    Err(EncodeError(format!("{what}: {name}")))
}

/// GL textures of one context registered with a CUDA device's context.
pub struct Textures {
    api: &'static Api,
    gl: Rc<Gl>,
    context: Ptr,
    stream: Ptr,
    resources: Vec<Ptr>,
    _device: Ref,
}

/// The CUDA context current on this thread until dropped.
struct Pushed<'a>(&'a Api);

impl Drop for Pushed<'_> {
    fn drop(&mut self) {
        let mut popped = std::ptr::null_mut();
        // SAFETY: pops the context the guard pushed.
        unsafe { (self.0.pop)(&mut popped) };
    }
}

impl Textures {
    /// `textures` of `gl`, registered with CUDA device `cuda` for reading.
    pub fn register(
        gl: &Rc<Gl>,
        cuda: &Ref,
        textures: &[glow::Texture],
    ) -> Result<Textures, EncodeError> {
        let api = api()?;
        gl.current()?;
        // SAFETY: a live CUDA device context, whose hardware context is libavutil's
        // AVCUDADeviceContext.
        let (context, stream) = unsafe {
            let device = (*cuda.as_ptr()).data as *const ffi::AVHWDeviceContext;
            let hw = &*((*device).hwctx as *const DeviceContext);
            (hw.context, hw.stream)
        };
        let mut registered = Textures {
            api,
            gl: gl.clone(),
            context,
            stream,
            resources: Vec::new(),
            _device: cuda.try_clone()?,
        };
        let _pushed = registered.push()?;
        for texture in textures {
            let mut resource = std::ptr::null_mut();
            // SAFETY: a texture name of `gl`, current; the resource is unregistered
            // on drop.
            let ret = unsafe {
                (api.register)(&mut resource, texture.0.get(), glow::TEXTURE_2D, READ_ONLY)
            };
            check(api, ret, "CUDA texture registration")?;
            registered.resources.push(resource);
        }
        drop(_pushed);
        Ok(registered)
    }

    fn push(&self) -> Result<Pushed<'static>, EncodeError> {
        // SAFETY: the device's own context.
        let pushed = unsafe { (self.api.push)(self.context) };
        check(self.api, pushed, "CUDA context")?;
        Ok(Pushed(self.api))
    }

    /// Copies NV12 pictures of `size` into `frames[i]` from textures 2i (luma) and 2i + 1
    /// (chroma), four bytes to a texel, after the GL work issued so far.
    pub fn copy_nv12(
        &mut self,
        frames: &[&frame::Video],
        (w, h): (u32, u32),
    ) -> Result<(), EncodeError> {
        let api = self.api;
        let _pushed = self.push()?;
        let count = self.resources.len() as u32;
        // SAFETY: registered resources, mapped here and unmapped below.
        let mapped = unsafe { (api.map)(count, self.resources.as_mut_ptr(), self.stream) };
        check(api, mapped, "CUDA texture map")?;
        let planes = frames
            .iter()
            .enumerate()
            .flat_map(|(i, f)| [(2 * i, *f, 0, h), (2 * i + 1, *f, 1, h / 2)]);
        let copied = (|| {
            for (texture, frame, plane, rows) in planes {
                let resource = *self
                    .resources
                    .get(texture)
                    .ok_or_else(|| EncodeError("a view has no texture".into()))?;
                let mut array = std::ptr::null_mut();
                // SAFETY: a mapped resource's first layer and level.
                let ret = unsafe { (api.array)(&mut array, resource, 0, 0) };
                check(api, ret, "CUDA texture array")?;
                // SAFETY: a CUDA frame's plane pointer and pitch.
                let (device, pitch) = unsafe {
                    let f = frame.as_ptr();
                    ((*f).data[plane] as u64, (*f).linesize[plane] as usize)
                };
                let copy = Memcpy2D {
                    src_memory_type: ARRAY,
                    src_array: array as usize,
                    dst_memory_type: DEVICE,
                    dst_device: device,
                    dst_pitch: pitch,
                    width_in_bytes: w as usize,
                    height: rows as usize,
                    ..Default::default()
                };
                // SAFETY: a texture of at least `w` bytes by `rows` into an NV12 plane of `size`.
                check(api, unsafe { (api.copy)(&copy, self.stream) }, "CUDA copy")?;
            }
            Ok(())
        })();
        // SAFETY: the resources mapped above.
        let unmapped = unsafe { (api.unmap)(count, self.resources.as_mut_ptr(), self.stream) };
        copied?;
        check(api, unmapped, "CUDA texture unmap")
    }
}

impl Drop for Textures {
    fn drop(&mut self) {
        if self.gl.current().is_err() {
            return;
        }
        let Ok(_pushed) = self.push() else {
            return;
        };
        for &resource in &self.resources {
            // SAFETY: a resource registered by this value, unregistered once.
            unsafe { (self.api.unregister)(resource) };
        }
    }
}
