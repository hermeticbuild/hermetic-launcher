// macOS backend: libc (libSystem) primitives, the C `main` entry point, and an
// execve-based launch. All FFI is confined to the `sys` module and wrapped in safe
// functions that form the platform seam consumed by the shared core.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::common::{cstr_len, print_number, Manifest};
use crate::run::Launch;
use crate::runfiles::Runfiles;

mod sys {
    extern "C" {
        pub fn exit(code: i32) -> !;
        pub fn write(fd: i32, buf: *const u8, count: usize) -> isize;
        pub fn open(path: *const u8, flags: i32, ...) -> i32;
        pub fn close(fd: i32) -> i32;
        pub fn access(path: *const u8, mode: i32) -> i32;
        pub fn execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32;
        pub fn mmap(
            addr: *mut core::ffi::c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut core::ffi::c_void;
        pub fn lseek(fd: i32, offset: i64, whence: i32) -> i64;
        // Launch path + working directory, for runfiles self-location.
        #[allow(non_snake_case)]
        pub fn _NSGetExecutablePath(buf: *mut u8, bufsize: *mut u32) -> i32;
        pub fn getcwd(buf: *mut u8, size: usize) -> *mut u8;
        // Thread-local errno is reached via __error() on macOS.
        pub fn __error() -> *mut i32;
        pub static mut environ: *const *const u8;
    }
}

const O_RDONLY: i32 = 0;
const STDOUT: i32 = 1;
const PROT_READ: i32 = 1;
const MAP_PRIVATE: i32 = 2;
const SEEK_END: i32 = 2;
const MAP_FAILED: *mut core::ffi::c_void = (-1isize) as *mut core::ffi::c_void;

// --- path semantics ---
pub const SEP: char = '/';
pub const NEWLINE: &[u8] = b"\n";

pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

pub fn to_native_path(s: &str) -> String {
    String::from(s)
}

// --- primitives ---
pub fn print(s: &[u8]) {
    unsafe {
        sys::write(STDOUT, s.as_ptr(), s.len());
    }
}

pub fn exit(code: i32) -> ! {
    unsafe { sys::exit(code) }
}

pub fn path_exists(path: &[u8]) -> bool {
    unsafe { sys::access(path.as_ptr(), 0) == 0 } // F_OK = 0
}

// Path used to launch this process, via _NSGetExecutablePath. This is the path
// as exec'd (symlinks NOT resolved); it is absolutized separately. None on
// failure.
fn launch_path() -> Option<Vec<u8>> {
    unsafe {
        // First query the required buffer size, then read the NUL-terminated path.
        let mut size: u32 = 0;
        sys::_NSGetExecutablePath(core::ptr::null_mut(), &mut size);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if sys::_NSGetExecutablePath(buf.as_mut_ptr(), &mut size) != 0 {
            return None;
        }
        buf.truncate(cstr_len(&buf));
        if buf.is_empty() {
            None
        } else {
            Some(buf)
        }
    }
}

// Current working directory via getcwd(3), used to absolutize a relative launch
// path. None on failure.
pub fn current_dir() -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 4096];
    if unsafe { sys::getcwd(buf.as_mut_ptr(), buf.len()) }.is_null() {
        return None;
    }
    buf.truncate(cstr_len(&buf));
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

// Environment variable lookup via the libc `environ` pointer.
pub fn get_env_var(name: &[u8]) -> Option<String> {
    unsafe {
        let mut env_ptr = sys::environ;
        while !(*env_ptr).is_null() {
            let entry_ptr = *env_ptr;
            let mut len = 0;
            while *entry_ptr.add(len) != 0 {
                len += 1;
                if len > 1048576 {
                    break;
                }
            }
            let entry = core::slice::from_raw_parts(entry_ptr, len);
            if let Some(eq_pos) = entry.iter().position(|&b| b == b'=') {
                if &entry[..eq_pos] == name {
                    return String::from_utf8(entry[eq_pos + 1..].to_vec()).ok();
                }
            }
            env_ptr = env_ptr.add(1);
        }
    }
    None
}

// Memory-map the manifest file read-only. Returns None on open/empty/mmap failure.
pub fn load_manifest(path: &[u8]) -> Option<Manifest> {
    unsafe {
        let fd = sys::open(path.as_ptr(), O_RDONLY);
        if fd < 0 {
            return None;
        }
        let size = sys::lseek(fd, 0, SEEK_END);
        if size <= 0 {
            sys::close(fd);
            return None;
        }
        let len = size as usize;
        let addr = sys::mmap(core::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, fd, 0);
        // The mapping keeps its own reference to the file; the fd can be closed.
        sys::close(fd);
        if addr == MAP_FAILED || addr.is_null() {
            return None;
        }
        // Leak the mapping: execve replaces the address space.
        Some(Manifest::from_mapping(addr as *const u8, len))
    }
}

// Build modified environment with runfiles variables, returning (data, pointers).
// The caller must keep `data` alive while the pointers are used.
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> (Vec<u8>, Vec<*const u8>) {
    let mut env_data = Vec::new();
    let mut env_ptrs = Vec::new();

    // Append "name=value\0" to env_data, recording its offset (fixed up to a pointer later).
    let add_env_var = |data: &mut Vec<u8>, ptrs: &mut Vec<*const u8>, name: &[u8], value: &str| {
        let start_pos = data.len();
        data.extend_from_slice(name);
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        data.push(0);
        ptrs.push(start_pos as *const u8);
    };

    if let Some(rf) = runfiles {
        if let Some(ref path) = rf.manifest_path {
            add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_MANIFEST_FILE", path);
        }
        if let Some(ref path) = rf.dir_path {
            add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_DIR", path);
            add_env_var(&mut env_data, &mut env_ptrs, b"JAVA_RUNFILES", path);
        }
    }

    // Copy existing environment, filtering out the runfiles vars we just set.
    unsafe {
        let mut env_ptr = sys::environ;
        while !(*env_ptr).is_null() {
            let entry_ptr = *env_ptr;
            let mut len = 0;
            while *entry_ptr.add(len) != 0 {
                len += 1;
                if len > 1048576 {
                    break;
                }
            }
            let entry = core::slice::from_raw_parts(entry_ptr, len);
            let should_skip = entry.starts_with(b"RUNFILES_MANIFEST_FILE=")
                || entry.starts_with(b"RUNFILES_DIR=")
                || entry.starts_with(b"JAVA_RUNFILES=");
            if !should_skip {
                let start_pos = env_data.len();
                env_data.extend_from_slice(entry);
                env_data.push(0);
                env_ptrs.push(start_pos as *const u8);
            }
            env_ptr = env_ptr.add(1);
        }
    }

    // Fix up offsets to real addresses now that env_data won't move.
    let base_ptr = env_data.as_ptr();
    for ptr in env_ptrs.iter_mut() {
        let offset = *ptr as usize;
        *ptr = unsafe { base_ptr.add(offset) };
    }
    env_ptrs.push(core::ptr::null());

    (env_data, env_ptrs)
}

// --- runtime args & launch ---
pub struct RuntimeArgs {
    argc: i32,
    argv: *const *const u8,
}

impl RuntimeArgs {
    /// Absolute path of the launching executable (`_NSGetExecutablePath`, made
    /// absolute), for runfiles self-location. Independent of argv[0].
    pub fn executable_path(&self) -> Option<Vec<u8>> {
        launch_path().map(crate::common::absolutize)
    }
}

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    unsafe {
        // Collect runtime args [1..] as NUL-terminated copies.
        let mut runtime: Vec<Vec<u8>> = Vec::new();
        if rt.argc > 1 {
            for i in 1..rt.argc as usize {
                let p = *rt.argv.add(i);
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                    if len > 1048576 {
                        print(b"ERROR: Runtime argument exceeds 1MB limit\n");
                        exit(1);
                    }
                }
                runtime.push(core::slice::from_raw_parts(p, len + 1).to_vec());
            }
        }

        // Build the argv pointer array: embedded resolved + runtime + NULL.
        let mut ptrs: Vec<*const u8> = Vec::with_capacity(launch.resolved.len() + runtime.len() + 1);
        for a in launch.resolved {
            ptrs.push(a.as_ptr());
        }
        for a in &runtime {
            ptrs.push(a.as_ptr());
        }
        ptrs.push(core::ptr::null());

        // The program to execute is the fully-resolved arg0; argv[0] may be overridden
        // with the runfiles-relative path (must read program before overwriting ptrs[0]).
        let program = launch.resolved[0].as_ptr();
        if let Some(override0) = launch.argv0_override {
            ptrs[0] = override0.as_ptr();
        }

        // Build the environment.
        let (_env_data, env_ptrs) = if launch.export_env {
            build_runfiles_environ(launch.runfiles)
        } else {
            let mut p = Vec::new();
            let mut e = sys::environ;
            while !(*e).is_null() {
                p.push(*e);
                e = e.add(1);
            }
            p.push(core::ptr::null());
            (Vec::new(), p)
        };

        let ret = sys::execve(program, ptrs.as_ptr(), env_ptrs.as_ptr());

        // execve only returns on failure; libc sets errno (reachable via __error()).
        let errno = *sys::__error();
        print(b"ERROR: execve failed with errno ");
        print_number(errno as usize);
        print(b" (return code ");
        print_number((-ret) as usize);
        print(b")\n");
        exit(1);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> ! {
    crate::run::main(RuntimeArgs { argc, argv })
}
