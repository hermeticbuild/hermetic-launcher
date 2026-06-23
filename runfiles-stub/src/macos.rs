// macOS backend: raw syscalls (no libc), a register-based entry point, and an
// execve-based launch. macOS still requires *linking* libSystem, but this backend
// never *calls* it: every primitive is a direct syscall (`svc #0x80` on arm64,
// `syscall` on x86_64) and the few compiler-intrinsic symbols are provided in-tree.
// With no dynamic calls there is no GOT and no load-time-written data, so the linked
// image carries no writable `__DATA` file page — the binary collapses to a single
// `__TEXT` page (mirroring the Linux backend; see linux.rs).

use alloc::string::String;
use alloc::vec::Vec;

use crate::common::{cstr_len, Manifest};
use crate::run::{Launch, ResolvedArg};
use crate::runfiles::Runfiles;

// Compiler intrinsics. Provided in-tree because we own the entry point and make no
// libc calls, so nothing else defines them; the optimizer/codegen still emits calls
// to memcpy/memset/memcmp for bulk moves and comparisons.
#[no_mangle]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    dst
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *s.add(i) = c as u8;
        i += 1;
    }
    s
}

// Optimized Darwin codegen emits bzero for zero-fills. Use volatile writes so
// LLVM cannot replace this implementation with a recursive call to bzero.
#[no_mangle]
pub unsafe extern "C" fn bzero(s: *mut u8, n: usize) {
    let mut i = 0;
    while i < n {
        core::ptr::write_volatile(s.add(i), 0);
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return a as i32 - b as i32;
        }
        i += 1;
    }
    0
}

// macOS BSD syscall numbers, shared by both arches (arm64 and x86_64 use the same
// unix-class numbers; only the class selector and trap instruction differ — see the
// `sc` modules). Stable across releases.
mod syscall_numbers {
    pub const SYS_EXIT: usize = 1;
    pub const SYS_WRITE: usize = 4;
    pub const SYS_OPEN: usize = 5;
    pub const SYS_CLOSE: usize = 6;
    pub const SYS_ACCESS: usize = 33;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_FCNTL: usize = 92;
    pub const SYS_MMAP: usize = 197;
    pub const SYS_LSEEK: usize = 199;
}

use syscall_numbers::*;

const O_RDONLY: i32 = 0;
const STDOUT: i32 = 1;

// mmap parameters for read-only file mapping.
const PROT_READ: usize = 1;
const MAP_PRIVATE: usize = 2;
const SEEK_END: i32 = 2;

// fcntl command: write the fd's full path into a caller buffer (>= MAXPATHLEN).
// Used to read the cwd, which has no public BSD syscall on macOS.
const F_GETPATH: i32 = 50;
const MAXPATHLEN: usize = 1024;

// --- path semantics ---
pub const SEP: char = '/';
pub const NEWLINE: &[u8] = b"\n";

pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

pub fn to_native_path(s: &str) -> String {
    String::from(s)
}

// --- syscall instruction layer ---
//
// One macro per arity, cfg-gated per arch so exactly one set compiles. macOS
// differs from Linux on both arches: errors are reported via the carry flag with a
// *positive* errno, so each macro's `b.cc/jnc + neg` tail converts that into Linux's
// convention (success unchanged, failure returns `-errno`) and the wrapper bodies
// and shared-core `< 0` / null checks stay identical to the Linux backend. The
// kernel does not preserve the argument registers across the trap, so every register
// the instruction touches is declared as written (`inout(...) => _`), not just read;
// declaring them as bare `in(...)` let the compiler assume a stale value survived the
// call — a layout-dependent miscompile that corrupted, e.g., the mmap length.
//
// arm64: number in `x16` (not `x8`), instruction `svc #0x80`. x86_64: number in
// `rax` OR'd with the BSD `SYSCALL_CLASS_UNIX` selector (0x2000000), instruction
// `syscall` (which additionally clobbers `rcx` and `r11`), args in
// rdi/rsi/rdx/r10/r8/r9. Inputs are widened to `i64` so a single register width
// satisfies `inout`'s same-type requirement regardless of the caller's arg types;
// the result is read back as `i64` and cast to the requested type.

#[cfg(target_arch = "aarch64")]
mod sc {
    macro_rules! syscall_noreturn {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!(
                "svc #0x80",
                in("x16") $nr as i64, in("x0") $a1 as i64, options(noreturn))
        };
    }
    macro_rules! syscall_void {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!(
                "svc #0x80",
                inout("x16") $nr as i64 => _, inout("x0") $a1 as i64 => _,
                options(nostack))
        };
    }
    macro_rules! syscall2 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "svc #0x80",
                "b.cc 3f",
                "neg x0, x0",
                "3:",
                inout("x16") $nr as i64 => _, inout("x0") $a1 as i64 => ret,
                inout("x1") $a2 as i64 => _,
                options(nostack));
            ret as $ty
        }};
    }
    macro_rules! syscall3 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "svc #0x80",
                "b.cc 3f",
                "neg x0, x0",
                "3:",
                inout("x16") $nr as i64 => _, inout("x0") $a1 as i64 => ret,
                inout("x1") $a2 as i64 => _, inout("x2") $a3 as i64 => _,
                options(nostack));
            ret as $ty
        }};
    }
    macro_rules! syscall6 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "svc #0x80",
                "b.cc 3f",
                "neg x0, x0",
                "3:",
                inout("x16") $nr as i64 => _, inout("x0") $a1 as i64 => ret,
                inout("x1") $a2 as i64 => _, inout("x2") $a3 as i64 => _,
                inout("x3") $a4 as i64 => _, inout("x4") $a5 as i64 => _,
                inout("x5") $a6 as i64 => _,
                options(nostack));
            ret as $ty
        }};
    }
    pub(super) use {syscall2, syscall3, syscall6, syscall_noreturn, syscall_void};
}

// macOS x86_64: the BSD class selector is OR'd into the number, the `syscall`
// instruction additionally clobbers rcx and r11, and the 4th argument goes in r10
// (not rcx). Carry-clear (`jnc`) marks success; failure is negated to `-errno`.
#[cfg(target_arch = "x86_64")]
mod sc {
    // BSD (unix) syscall class selector OR'd into the call number. Inlined as a plain
    // expression (not a sibling macro) so it resolves at each syscall macro's call
    // site, where a nested `macro_rules!` would be out of scope.
    macro_rules! syscall_noreturn {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!(
                "syscall",
                in("rax") (0x2000000i64 | ($nr as i64)), in("rdi") $a1 as i64,
                options(noreturn))
        };
    }
    macro_rules! syscall_void {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!(
                "syscall",
                inout("rax") (0x2000000i64 | ($nr as i64)) => _, inout("rdi") $a1 as i64 => _,
                lateout("rcx") _, lateout("r11") _, options(nostack))
        };
    }
    macro_rules! syscall2 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "syscall",
                "jnc 3f",
                "neg rax",
                "3:",
                inout("rax") (0x2000000i64 | ($nr as i64)) => ret, inout("rdi") $a1 as i64 => _,
                inout("rsi") $a2 as i64 => _,
                lateout("rcx") _, lateout("r11") _, options(nostack));
            ret as $ty
        }};
    }
    macro_rules! syscall3 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "syscall",
                "jnc 3f",
                "neg rax",
                "3:",
                inout("rax") (0x2000000i64 | ($nr as i64)) => ret, inout("rdi") $a1 as i64 => _,
                inout("rsi") $a2 as i64 => _, inout("rdx") $a3 as i64 => _,
                lateout("rcx") _, lateout("r11") _, options(nostack));
            ret as $ty
        }};
    }
    macro_rules! syscall6 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {{
            let ret: i64;
            core::arch::asm!(
                "syscall",
                "jnc 3f",
                "neg rax",
                "3:",
                inout("rax") (0x2000000i64 | ($nr as i64)) => ret, inout("rdi") $a1 as i64 => _,
                inout("rsi") $a2 as i64 => _, inout("rdx") $a3 as i64 => _,
                inout("r10") $a4 as i64 => _, inout("r8") $a5 as i64 => _,
                inout("r9") $a6 as i64 => _,
                lateout("rcx") _, lateout("r11") _, options(nostack));
            ret as $ty
        }};
    }
    pub(super) use {syscall2, syscall3, syscall6, syscall_noreturn, syscall_void};
}

use sc::*;

// --- syscall wrappers ---
pub fn exit(code: i32) -> ! {
    unsafe { syscall_noreturn!(SYS_EXIT, code) }
}

fn write(fd: i32, buf: &[u8]) -> isize {
    unsafe { syscall3!(isize; SYS_WRITE, fd, buf.as_ptr(), buf.len()) }
}

fn close(fd: i32) {
    unsafe { syscall_void!(SYS_CLOSE, fd) }
}

// open(path, O_RDONLY). macOS has a plain `open` syscall (no openat needed).
fn open(path: &[u8]) -> i32 {
    unsafe { syscall3!(i32; SYS_OPEN, path.as_ptr(), O_RDONLY, 0) }
}

// Seek to the end of the file and return its size (lseek with SEEK_END). Negative
// on error.
fn lseek_end(fd: i32) -> i64 {
    unsafe { syscall3!(i64; SYS_LSEEK, fd, 0i64, SEEK_END) }
}

// Memory-map `len` bytes of `fd` read-only (PROT_READ, MAP_PRIVATE, offset 0).
// Null on error: the carry-flag conversion makes failures return a small negative
// value, while valid user addresses are large and positive.
fn mmap_read(fd: i32, len: usize) -> *const u8 {
    let ret: isize =
        unsafe { syscall6!(isize; SYS_MMAP, 0usize, len, PROT_READ, MAP_PRIVATE, fd, 0usize) };
    if ret < 0 {
        core::ptr::null()
    } else {
        ret as *const u8
    }
}

// Check if a path exists using access() with F_OK (0).
pub fn path_exists(path: &[u8]) -> bool {
    unsafe { syscall2!(i32; SYS_ACCESS, path.as_ptr(), 0i32) == 0 }
}

pub fn utf8_path_exists(path: &str) -> bool {
    let mut terminated = Vec::from(path.as_bytes());
    terminated.push(0);
    path_exists(&terminated)
}

pub fn executable_relative(executable: &[u8], fallback: &str) -> Option<ResolvedArg> {
    crate::native_path::unix_executable_relative(executable, fallback.as_bytes())
        .map(ResolvedArg::Bytes)
}

pub fn resolved_arg_exists(arg: &ResolvedArg) -> bool {
    let ResolvedArg::Bytes(path) = arg;
    let mut terminated = path.clone();
    terminated.push(0);
    path_exists(&terminated)
}

fn fcntl(fd: i32, cmd: i32, arg: *mut u8) -> i32 {
    unsafe { syscall3!(i32; SYS_FCNTL, fd, cmd, arg) }
}

fn execve(filename: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32 {
    unsafe { syscall3!(i32; SYS_EXECVE, filename, argv, envp) }
}

// --- primitives ---
pub fn print(s: &[u8]) {
    write(STDOUT, s);
}

// Current working directory, used to absolutize a relative launch path. macOS has
// no public getcwd syscall, so open(".") and ask the kernel for its full path via
// fcntl(F_GETPATH). Best-effort: None on failure.
pub fn current_dir() -> Option<Vec<u8>> {
    let fd = open(b".\0");
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; MAXPATHLEN];
    let rc = fcntl(fd, F_GETPATH, buf.as_mut_ptr());
    close(fd);
    if rc != 0 {
        return None;
    }
    let len = cstr_len(&buf);
    if len == 0 {
        return None;
    }
    Some(buf[..len].to_vec())
}

// Memory-map the manifest file read-only. None on open/empty/mmap failure.
pub fn load_manifest(path: &[u8]) -> Option<Manifest> {
    let fd = open(path);
    if fd < 0 {
        return None;
    }
    let size = lseek_end(fd);
    if size <= 0 {
        close(fd);
        return None;
    }
    let len = size as usize;
    let ptr = mmap_read(fd, len);
    // The mapping keeps its own reference to the file; the fd can be closed.
    close(fd);
    if ptr.is_null() {
        return None;
    }
    // Leak the mapping: execve replaces the address space.
    Some(unsafe { Manifest::from_mapping(ptr, len) })
}

// --- environment ---
//
// The kernel hands us `envp` at entry, so there is no need for the libc `environ`
// global or /proc. The pointer array lives until execve replaces the address space.
static mut ENVP: *const *const u8 = core::ptr::null();

// Iterate the captured envp entries as byte slices.
unsafe fn each_env(mut f: impl FnMut(&[u8])) {
    let mut p = ENVP;
    if p.is_null() {
        return;
    }
    while !(*p).is_null() {
        let entry_ptr = *p;
        let mut len = 0;
        while *entry_ptr.add(len) != 0 {
            len += 1;
            if len > 1048576 {
                break;
            }
        }
        f(core::slice::from_raw_parts(entry_ptr, len));
        p = p.add(1);
    }
}

// Environment variable lookup over the captured envp array.
pub fn get_env_var(name: &[u8]) -> Option<String> {
    let mut found: Option<String> = None;
    unsafe {
        each_env(|entry| {
            if found.is_some() {
                return;
            }
            if let Some(eq_pos) = entry.iter().position(|&b| b == b'=') {
                if &entry[..eq_pos] == name {
                    found = String::from_utf8(entry[eq_pos + 1..].to_vec()).ok();
                }
            }
        });
    }
    found
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

    // Copy the existing environment without inherited runfiles variables.
    unsafe {
        each_env(|entry| {
            let should_skip = entry.starts_with(b"RUNFILES_MANIFEST_FILE=")
                || entry.starts_with(b"RUNFILES_DIR=")
                || entry.starts_with(b"JAVA_RUNFILES=");
            if !should_skip {
                let start_pos = env_data.len();
                env_data.extend_from_slice(entry);
                env_data.push(0);
                env_ptrs.push(start_pos as *const u8);
            }
        });
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

// Copy the captured envp into a fresh pointer array (terminated by NULL), for the
// no-export path.
unsafe fn clone_environ() -> Vec<*const u8> {
    let mut ptrs = Vec::new();
    let mut p = ENVP;
    if !p.is_null() {
        while !(*p).is_null() {
            ptrs.push(*p);
            p = p.add(1);
        }
    }
    ptrs.push(core::ptr::null());
    ptrs
}

// --- runtime args & launch ---
pub struct RuntimeArgs {
    argc: usize,
    argv: *const *const u8,
    // Pointer to the "executable_path=..." entry from the apple[] array, or null.
    apple0: *const u8,
}

impl RuntimeArgs {
    /// Absolute path of the launching executable, from the kernel-provided
    /// `apple[0]` ("executable_path=<path>") made absolute, for runfiles
    /// self-location. Independent of argv[0].
    pub fn executable_path(&self) -> Option<Vec<u8>> {
        if self.apple0.is_null() {
            return None;
        }
        // Read the NUL-terminated "executable_path=<path>" string.
        let mut len = 0;
        unsafe {
            while *self.apple0.add(len) != 0 {
                len += 1;
                if len > 1048576 {
                    return None;
                }
            }
        }
        let entry = unsafe { core::slice::from_raw_parts(self.apple0, len) };
        const PREFIX: &[u8] = b"executable_path=";
        if !entry.starts_with(PREFIX) {
            return None;
        }
        let path = &entry[PREFIX.len()..];
        if path.is_empty() {
            return None;
        }
        Some(crate::common::absolutize(path.to_vec()))
    }

    pub fn fallback_executable_path(&self) -> Option<Vec<u8>> {
        self.executable_path()
    }
}

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    unsafe {
        // Collect runtime args [1..] as NUL-terminated copies.
        let mut runtime: Vec<Vec<u8>> = Vec::new();
        if rt.argc > 1 {
            for i in 1..rt.argc {
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
            let ResolvedArg::Bytes(bytes) = a;
            ptrs.push(bytes.as_ptr());
        }
        for a in &runtime {
            ptrs.push(a.as_ptr());
        }
        ptrs.push(core::ptr::null());

        // The program to execute is the fully-resolved arg0; argv[0] may be overridden
        // with the runfiles-relative path (read program before overwriting ptrs[0]).
        let ResolvedArg::Bytes(program) = &launch.resolved[0];
        let program = program.as_ptr();
        if let Some(override0) = launch.argv0_override {
            ptrs[0] = override0.as_ptr();
        }

        // Build the environment.
        let (_env_data, env_ptrs) = if launch.export_runfiles_env {
            build_runfiles_environ(launch.child_runfiles)
        } else {
            (Vec::new(), clone_environ())
        };

        let ret = execve(program, ptrs.as_ptr(), env_ptrs.as_ptr());

        // execve only returns on failure.
        print(b"ERROR: execve failed with code ");
        let digit = if ret < 0 {
            print(b"-");
            (-ret) as u8 + b'0'
        } else {
            ret as u8 + b'0'
        };
        print(&[digit]);
        print(b"\n");
        exit(1);
    }
}

// Entry point. dyld invokes the LC_MAIN entry as a C `main`, passing
// argc/argv/envp/apple in the first four argument registers on both arm64 and
// x86_64. We capture envp and apple[0] into globals/args and hand off to the shared
// core. Returning a value here is fine — but the core never returns (it execve's or
// exit's).
#[no_mangle]
pub extern "C" fn main(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
    apple: *const *const u8,
) -> i32 {
    unsafe {
        ENVP = envp;
        let apple0 = if apple.is_null() {
            core::ptr::null()
        } else {
            *apple
        };
        crate::run::main(RuntimeArgs {
            argc: argc as usize,
            argv,
            apple0,
        })
    }
}
