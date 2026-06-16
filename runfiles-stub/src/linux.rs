// Linux backend: raw syscalls (no libc), a custom `_start` entry, and an
// execve-based launch. The per-architecture syscall asm and the compiler-intrinsic
// symbols stay here; everything above the syscall layer is shared.

use alloc::vec;
use alloc::string::String;
use alloc::vec::Vec;

use crate::common::Manifest;
use crate::run::Launch;
use crate::runfiles::Runfiles;

// Compiler intrinsics and glibc-compat symbols. These are needed because we link
// without crt startup files (-nostartfiles) and provide our own _start.
#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *s.add(i) = c as u8;
        i += 1;
    }
    s
}

// glibc expects this symbol when linking without crt1.o/_start.
#[no_mangle]
pub static _IO_stdin_used: i32 = 0x20001;

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

#[no_mangle]
pub unsafe extern "C" fn bcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    memcmp(s1, s2, n)
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

// Syscall numbers - architecture specific
#[cfg(target_arch = "x86_64")]
mod syscall_numbers {
    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_LSEEK: usize = 8;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_ACCESS: usize = 21;
    pub const SYS_READLINKAT: usize = 267;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_EXIT: usize = 60;
    pub const AT_FDCWD: i32 = -100;
}

#[cfg(target_arch = "aarch64")]
mod syscall_numbers {
    pub const SYS_READ: usize = 63;
    pub const SYS_WRITE: usize = 64;
    pub const SYS_OPENAT: usize = 56;  // openat is used on aarch64
    pub const SYS_CLOSE: usize = 57;
    pub const SYS_LSEEK: usize = 62;
    pub const SYS_MMAP: usize = 222;
    pub const SYS_FACCESSAT: usize = 48;  // faccessat is used on aarch64
    pub const SYS_READLINKAT: usize = 78;
    pub const SYS_EXECVE: usize = 221;
    pub const SYS_EXIT: usize = 93;
    pub const AT_FDCWD: i32 = -100;  // Special fd for openat/faccessat to work like open/access
}

#[cfg(target_arch = "s390x")]
mod syscall_numbers {
    pub const SYS_EXIT: usize = 1;
    pub const SYS_READ: usize = 3;
    pub const SYS_WRITE: usize = 4;
    pub const SYS_OPEN: usize = 5;
    pub const SYS_CLOSE: usize = 6;
    pub const SYS_LSEEK: usize = 19;
    pub const SYS_EXECVE: usize = 11;
    pub const SYS_ACCESS: usize = 33;
    pub const SYS_MMAP: usize = 90;
}

use syscall_numbers::*;

const O_RDONLY: i32 = 0;
const STDOUT: i32 = 1;

// mmap parameters for read-only file mapping
const PROT_READ: usize = 1;
const MAP_PRIVATE: usize = 2;
const SEEK_END: i32 = 2;

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
// One macro per (arch, arity), cfg-gated so exactly one set is compiled. Each
// expansion is token-identical to the previous hand-written per-syscall asm (same
// instruction, registers, clobbers, operand types and return type), so the generated
// machine code is unchanged — this only removes the 3x per-arch source duplication.
// `_noreturn` is for exit; `_void` discards the return register (close); the rest
// return a value of the caller-specified type.

#[cfg(target_arch = "x86_64")]
mod sc {
    macro_rules! syscall_noreturn {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1, options(noreturn))
        };
    }
    macro_rules! syscall_void {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1,
                lateout("rax") _, lateout("rcx") _, lateout("r11") _)
        };
    }
    macro_rules! syscall2 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr) => {{
            let ret: $ty;
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1, in("rsi") $a2,
                lateout("rax") ret, lateout("rcx") _, lateout("r11") _);
            ret
        }};
    }
    macro_rules! syscall3 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
            let ret: $ty;
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1, in("rsi") $a2, in("rdx") $a3,
                lateout("rax") ret, lateout("rcx") _, lateout("r11") _);
            ret
        }};
    }
    macro_rules! syscall4 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr) => {{
            let ret: $ty;
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1, in("rsi") $a2, in("rdx") $a3,
                in("r10") $a4,
                lateout("rax") ret, lateout("rcx") _, lateout("r11") _);
            ret
        }};
    }
    macro_rules! syscall6 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {{
            let ret: $ty;
            core::arch::asm!("syscall", in("rax") $nr, in("rdi") $a1, in("rsi") $a2, in("rdx") $a3,
                in("r10") $a4, in("r8") $a5, in("r9") $a6,
                lateout("rax") ret, lateout("rcx") _, lateout("r11") _);
            ret
        }};
    }
    pub(super) use {syscall2, syscall3, syscall4, syscall6, syscall_noreturn, syscall_void};
}

#[cfg(target_arch = "aarch64")]
mod sc {
    macro_rules! syscall_noreturn {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("svc #0", in("x8") $nr, in("x0") $a1, options(noreturn))
        };
    }
    macro_rules! syscall_void {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("svc #0", in("x8") $nr, in("x0") $a1, lateout("x0") _)
        };
    }
    macro_rules! syscall3 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc #0", in("x8") $nr, in("x0") $a1, in("x1") $a2, in("x2") $a3,
                lateout("x0") ret);
            ret
        }};
    }
    macro_rules! syscall4 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc #0", in("x8") $nr, in("x0") $a1, in("x1") $a2, in("x2") $a3,
                in("x3") $a4, lateout("x0") ret);
            ret
        }};
    }
    macro_rules! syscall6 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc #0", in("x8") $nr, in("x0") $a1, in("x1") $a2, in("x2") $a3,
                in("x3") $a4, in("x4") $a5, in("x5") $a6, lateout("x0") ret);
            ret
        }};
    }
    pub(super) use {syscall3, syscall4, syscall6, syscall_noreturn, syscall_void};
}

#[cfg(target_arch = "s390x")]
mod sc {
    macro_rules! syscall_noreturn {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("svc 0", in("r1") $nr, in("r2") $a1, options(noreturn))
        };
    }
    macro_rules! syscall_void {
        ($nr:expr, $a1:expr) => {
            core::arch::asm!("svc 0", in("r1") $nr, in("r2") $a1, lateout("r2") _)
        };
    }
    macro_rules! syscall1 {
        ($ty:ty; $nr:expr, $a1:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc 0", in("r1") $nr, in("r2") $a1, lateout("r2") ret);
            ret
        }};
    }
    macro_rules! syscall2 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc 0", in("r1") $nr, in("r2") $a1, in("r3") $a2, lateout("r2") ret);
            ret
        }};
    }
    macro_rules! syscall3 {
        ($ty:ty; $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
            let ret: $ty;
            core::arch::asm!("svc 0", in("r1") $nr, in("r2") $a1, in("r3") $a2, in("r4") $a3,
                lateout("r2") ret);
            ret
        }};
    }
    pub(super) use {syscall1, syscall2, syscall3, syscall_noreturn, syscall_void};
}

#[allow(unused_imports)]
use sc::*;

// --- syscall wrappers (architecture-independent above the macro layer) ---
pub fn exit(code: i32) -> ! {
    unsafe { syscall_noreturn!(SYS_EXIT, code) }
}

fn write(fd: i32, buf: &[u8]) -> isize {
    unsafe { syscall3!(isize; SYS_WRITE, fd, buf.as_ptr(), buf.len()) }
}

fn read(fd: i32, buf: &mut [u8]) -> isize {
    unsafe { syscall3!(isize; SYS_READ, fd, buf.as_ptr(), buf.len()) }
}

fn close(fd: i32) {
    unsafe { syscall_void!(SYS_CLOSE, fd) }
}

fn open(path: &[u8]) -> i32 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { syscall3!(i32; SYS_OPEN, path.as_ptr(), O_RDONLY, 0) }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { syscall4!(i32; SYS_OPENAT, AT_FDCWD, path.as_ptr(), O_RDONLY, 0) }
    }
    #[cfg(target_arch = "s390x")]
    {
        unsafe { syscall3!(i64; SYS_OPEN, path.as_ptr(), O_RDONLY, 0i64) as i32 }
    }
}

// Seek to the end of the file and return its size in bytes (lseek with SEEK_END).
// Returns a negative value on error.
fn lseek_end(fd: i32) -> i64 {
    unsafe { syscall3!(i64; SYS_LSEEK, fd, 0i64, SEEK_END) }
}

// Memory-map `len` bytes of `fd` read-only (PROT_READ, MAP_PRIVATE, offset 0).
// Returns a null pointer on error. The kernel returns a negative errno in
// [-4095, -1] on failure; user-space addresses are positive on these arches.
fn mmap_read(fd: i32, len: usize) -> *const u8 {
    #[cfg(not(target_arch = "s390x"))]
    {
        let ret: isize =
            unsafe { syscall6!(isize; SYS_MMAP, 0usize, len, PROT_READ, MAP_PRIVATE, fd, 0usize) };
        if ret < 0 {
            core::ptr::null()
        } else {
            ret as *const u8
        }
    }
    #[cfg(target_arch = "s390x")]
    {
        // s390x uses the old mmap convention: r2 holds a pointer to an array of
        // 6 unsigned longs { addr, len, prot, flags, fd, offset }.
        let args: [usize; 6] = [0, len, PROT_READ, MAP_PRIVATE, fd as usize, 0];
        let ret: i64 = unsafe { syscall1!(i64; SYS_MMAP, args.as_ptr()) };
        if ret < 0 {
            core::ptr::null()
        } else {
            ret as usize as *const u8
        }
    }
}

// Check if a path exists using access()/faccessat() with F_OK (0).
pub fn path_exists(path: &[u8]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { syscall2!(i32; SYS_ACCESS, path.as_ptr(), 0i32) == 0 }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { syscall4!(i32; SYS_FACCESSAT, AT_FDCWD, path.as_ptr(), 0i32, 0i32) == 0 }
    }
    #[cfg(target_arch = "s390x")]
    {
        unsafe { syscall2!(i64; SYS_ACCESS, path.as_ptr(), 0i64) == 0 }
    }
}

/// Absolute path of the running executable, via `readlinkat(/proc/self/exe)`.
///
/// Used for runfiles discovery when `argv[0]` is relative (as under `bazel run`):
/// `<argv[0]>.runfiles` can't be located against the cwd, so resolve the real
/// executable path instead. Returns None on failure (callers fall back to
/// argv[0]). Not wired for s390x (no readlinkat number defined here yet).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn executable_path() -> Option<alloc::vec::Vec<u8>> {
    let mut buf = alloc::vec![0u8; 4096];
    // readlinkat(AT_FDCWD, "/proc/self/exe", buf, buf.len()); readlink does NOT
    // NUL-terminate, so the return value is the byte length.
    let path = b"/proc/self/exe\0";
    let n: i64 =
        unsafe { syscall4!(i64; SYS_READLINKAT, AT_FDCWD, path.as_ptr(), buf.as_mut_ptr(), buf.len()) };
    if n <= 0 || (n as usize) >= buf.len() {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn executable_path() -> Option<alloc::vec::Vec<u8>> {
    None
}

fn execve(filename: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32 {
    #[cfg(not(target_arch = "s390x"))]
    {
        unsafe { syscall3!(i32; SYS_EXECVE, filename, argv, envp) }
    }
    #[cfg(target_arch = "s390x")]
    {
        unsafe { syscall3!(i64; SYS_EXECVE, filename, argv, envp) as i32 }
    }
}

// --- primitives ---
pub fn print(s: &[u8]) {
    write(STDOUT, s);
}

// Read the whole of /proc/self/environ into a byte buffer (empty on failure).
// The buffer is a sequence of NUL-terminated "NAME=VALUE" entries.
fn slurp_environ() -> Vec<u8> {
    let mut environ_data = Vec::new();
    let fd = open(b"/proc/self/environ\0");
    if fd < 0 {
        return environ_data;
    }
    let mut chunk = [0u8; 8192];
    loop {
        let bytes_read = read(fd, &mut chunk);
        if bytes_read <= 0 {
            break;
        }
        environ_data.extend_from_slice(&chunk[..bytes_read as usize]);
    }
    close(fd);
    environ_data
}

// Environment variable lookup by reading /proc/self/environ.
pub fn get_env_var(name: &[u8]) -> Option<String> {
    let environ_data = slurp_environ();
    for entry in environ_data.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        if let Some(eq_pos) = entry.iter().position(|&b| b == b'=') {
            if &entry[..eq_pos] == name {
                return String::from_utf8(entry[eq_pos + 1..].to_vec()).ok();
            }
        }
    }
    None
}

// Load the manifest by memory-mapping it read-only. None on open/empty/mmap failure.
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

// Read /proc/self/environ into (data, pointers); pointers point into data.
fn read_environ() -> (Vec<u8>, Vec<*const u8>) {
    let environ_data = slurp_environ();
    if environ_data.is_empty() {
        return (Vec::new(), vec![core::ptr::null()]);
    }

    let mut env_ptrs = Vec::new();
    let mut pos = 0;
    let data_len = environ_data.len();
    while pos < data_len {
        if environ_data[pos] == 0 {
            pos += 1;
            continue;
        }
        env_ptrs.push(unsafe { environ_data.as_ptr().add(pos) });
        while pos < data_len && environ_data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    env_ptrs.push(core::ptr::null());
    (environ_data, env_ptrs)
}

// Build modified environment with runfiles variables; pointers point into data.
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> (Vec<u8>, Vec<*const u8>) {
    let (base_data, base_ptrs) = read_environ();

    let rf = match runfiles {
        Some(r) => r,
        None => return (base_data, base_ptrs),
    };

    let mut env_data = Vec::new();
    let mut env_ptrs = Vec::new();

    let add_env_var = |data: &mut Vec<u8>, ptrs: &mut Vec<*const u8>, name: &[u8], value: &str| {
        let start_pos = data.len();
        data.extend_from_slice(name);
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        data.push(0);
        ptrs.push(start_pos as *const u8);
    };

    if let Some(ref path) = rf.manifest_path {
        add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_MANIFEST_FILE", path);
    }
    if let Some(ref path) = rf.dir_path {
        add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_DIR", path);
        add_env_var(&mut env_data, &mut env_ptrs, b"JAVA_RUNFILES", path);
    }

    // Copy existing environment (skip runfiles vars we're setting).
    for env_entry in base_data.split(|&b| b == 0) {
        if env_entry.is_empty() {
            continue;
        }
        let is_runfiles_var = env_entry.starts_with(b"RUNFILES_MANIFEST_FILE=")
            || env_entry.starts_with(b"RUNFILES_DIR=")
            || env_entry.starts_with(b"JAVA_RUNFILES=");
        if !is_runfiles_var {
            let start_pos = env_data.len();
            env_data.extend_from_slice(env_entry);
            env_data.push(0);
            env_ptrs.push(start_pos as *const u8);
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
    argc: usize,
    argv: *const *const u8,
}

impl RuntimeArgs {
    /// argv[0] (the stub's own path) as bytes, for runfiles fallback discovery.
    pub fn program_path(&self) -> Option<&[u8]> {
        if self.argc == 0 {
            return None;
        }
        unsafe {
            let p = *self.argv;
            let mut len = 0;
            while *p.add(len) != 0 && len < 1048576 {
                len += 1;
            }
            if len > 0 {
                Some(core::slice::from_raw_parts(p, len))
            } else {
                None
            }
        }
    }
}

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    // Collect runtime args [1..] as NUL-terminated copies.
    let mut runtime: Vec<Vec<u8>> = Vec::new();
    if rt.argc > 1 {
        for i in 1..rt.argc {
            unsafe {
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
    // with the runfiles-relative path (read program before overwriting ptrs[0]).
    let program = launch.resolved[0].as_ptr();
    if let Some(override0) = launch.argv0_override {
        ptrs[0] = override0.as_ptr();
    }

    let (_env_data, env_ptrs) = if launch.export_env {
        build_runfiles_environ(launch.runfiles)
    } else {
        read_environ()
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

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mov rdi, rsp",                 // Pass stack pointer as first argument
    "call _start_rust",             // Call the actual start function
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mov x0, sp",                   // Pass stack pointer as first argument
    "b _start_rust",                // Jump to the actual start function
);

#[cfg(target_arch = "s390x")]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "lgr %r2, %r15",               // Pass stack pointer as first argument
    "aghi %r15, -160",             // Allocate mandatory register save area
    "brasl %r14, _start_rust",     // Call the actual start function
);

#[no_mangle]
pub extern "C" fn _start_rust(initial_sp: *const usize) -> ! {
    // Stack layout: [sp] = argc, [sp + 8] = argv[0], [sp + 16] = argv[1], ...
    let (argc, argv) = unsafe {
        let argc = *initial_sp;
        let argv = (initial_sp as usize + 8) as *const *const u8;
        (argc, argv)
    };
    crate::run::main(RuntimeArgs { argc, argv })
}
